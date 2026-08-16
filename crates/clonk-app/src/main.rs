// Explorer must not open a console window behind the game; `main` reattaches
// stdio when the runtime is started from a terminal instead.
#![cfg_attr(windows, windows_subsystem = "windows")]
#![allow(dead_code)]
#![allow(
    clippy::explicit_counter_loop,
    clippy::large_enum_variant,
    clippy::manual_clamp,
    clippy::manual_strip,
    clippy::match_like_matches_macro,
    clippy::too_many_arguments
)]

// A simulation frame makes many thousands of short-lived allocations: host
// world contexts, per-call object materialization and script values are all
// built and dropped inside one tick. The system allocator is the bottleneck in
// that regime on macOS; measured -29% mean and -35% p99 tick time on MeltMe.
// The win is allocator-relative, so platforms whose default allocator already
// handles small-object churn well (glibc's tcache) may see much less.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod advanced_config;
mod classic_record_stream;
mod component_editor_window_host;
mod console_toolbox_window;
mod console_viewport_windows;
mod console_window_position;
mod control_options;
mod deferred_config;
mod desktop_notification;
mod developer_component_editor;
mod developer_console_save;
mod developer_host;
mod developer_object_list_view;
mod developer_toolbox;
mod developer_toolbox_view;
mod developer_tools_page;
mod developer_windows;
mod display_backend;
mod display_sleep_inhibitor;
mod dock_icon;
use clonk_app_render::draw_commands;
mod game_message;
mod gamepad;
mod gpu_instance;
mod headed_surface_smoke;
use clonk_app_menus::ingame_menu;
use clonk_app_netplay::host_game_resource_sources;
use clonk_app_render::gpu_renderer;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
mod input;
mod local_control;
use clonk_app_netplay::network;
mod network_team_assignment;
use clonk_app_menus::object_menu;
mod object_list_window_host;
mod offline_savegame;
mod offline_startup;
mod output_folders;
mod ready_check_notification;
use clonk_app_netplay::prepared_host_bootstrap;
use clonk_app_netplay::resource_path_identity;
mod runtime_join_save;
mod settings;
mod shell_window_host;
mod software_window;
mod startup_player_files;
mod system_fonts;
mod toolbox_window_host;
mod update_check;
mod update_download;
mod viewport_window_host;
mod voice_chat;
mod window_icon;

// Step 6a of the decomposition campaign (rust/REFACTOR_PLAN.md): per-area
// extension files of the `impl GameApp` block. Each file holds
// `use super::*;` plus its own `impl GameApp { ... }` with methods moved
// verbatim from the root block below.
#[path = "game_app/chat.rs"]
mod game_app_chat;
#[path = "game_app/config.rs"]
mod game_app_config;
#[path = "game_app/console_record.rs"]
mod game_app_console_record;
#[path = "game_app/input.rs"]
mod game_app_input;
#[path = "game_app/lobby.rs"]
mod game_app_lobby;
#[path = "game_app/menu.rs"]
mod game_app_menu;
#[path = "game_app/network.rs"]
mod game_app_network;
#[path = "game_app/player.rs"]
mod game_app_player;
#[path = "game_app/render.rs"]
mod game_app_render;
#[path = "game_app/saves.rs"]
mod game_app_saves;
#[path = "game_app/scenario.rs"]
mod game_app_scenario;
#[path = "game_app/scensel.rs"]
mod game_app_scensel;
#[path = "game_app/sound.rs"]
mod game_app_sound;
#[path = "game_app/startup.rs"]
mod game_app_startup;
#[path = "game_app/update.rs"]
mod game_app_update;
#[path = "game_app/voice.rs"]
mod game_app_voice;

#[path = "main_parts/app_state.rs"]
mod main_app_state;
#[path = "main_parts/assets.rs"]
mod main_assets;
#[path = "main_parts/audio.rs"]
mod main_audio;
#[path = "main_parts/gpu_profile.rs"]
mod main_gpu_profile;
#[path = "main_parts/render_io.rs"]
mod main_render_io;
#[path = "main_parts/resources.rs"]
mod main_resources;

pub(crate) use main_app_state::*;
pub(crate) use main_assets::*;
pub(crate) use main_audio::*;
pub(crate) use main_gpu_profile::*;
pub(crate) use main_render_io::*;
pub(crate) use main_resources::*;

use std::cmp::Ordering;
use std::collections::{hash_map::DefaultHasher, BTreeMap, HashMap, HashSet, VecDeque};
use std::convert::TryFrom;
use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File};
use std::hash::{Hash, Hasher};
use std::io::{self, BufRead, Write};
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::ops::ControlFlow as OpsControlFlow;
use std::path::{Component, Path, PathBuf};
use std::ptr::NonNull;
use std::sync::{
    atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering as AtomicOrdering},
    mpsc::{self, Receiver, RecvTimeoutError, TryRecvError},
    Arc, Mutex, OnceLock,
};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use clonk_app_core::pictures::{
    apply_definition_owner_color, definition_menu_picture, inventory_object_picture_layers,
    resolve_portrait_text_spec, resolve_script_font_image, ScriptTextSpecResources,
};
use clonk_app_core::{
    AppMode, ClassicGameLobbyBoundary, ClassicGameLobbyChild, ClassicGuiBootstrapIssue,
    ClassicStartupBootstrapIssue,
};
use clonk_app_menus::game_over::{
    resolve_league_evaluation_icon, EvaluationGoal, EvaluationPlayer, EvaluationViewModel,
    GameOverAction, GameOverActivationKey, GameOverClassicResources, GameOverEntry, GameOverFocus,
    GameOverOutcome, GameOverSound, GameOverState, NextMissionButton,
};
use clonk_app_menus::ingame_menu::{
    DisplayFlags, DisplayToggle, GoalRuleEntry, HostDisconnectClientEntry, HostilityEntry,
    IngameMenuGraphics, IngameMenuLabels, IngameMenuPointerTarget, IngameMenuState,
    MainMenuConditions, MenuAction, MenuOutcome, NewPlayerEntry, ObserverPlayerEntry,
    ObserverTarget, OptionFlags, SaveSlotState, TeamSelectionEntry, UpperBoardMode,
};
use clonk_app_menus::menu_controls::{map_async_cursor_menu_control_event, map_menu_control_event};
use clonk_app_menus::object_menu::{
    engine_script_menu_inline_image_specs, engine_script_menu_layout_with_free_anchor,
    engine_script_menu_layout_with_presentation,
    engine_script_menu_pointer_target_with_free_anchor,
    engine_script_menu_pointer_target_with_presentation, engine_script_menu_presentation_geometry,
    engine_script_menu_presentation_geometry_with_free_anchor,
    render_engine_script_menu_with_gamma, resolve_engine_script_menu_footer,
    validate_menu_decoration_for_area, EngineScriptMenuLayout, EngineScriptMenuPointerTarget,
    EngineScriptMenuPresentationGeometry, MenuMode as AppObjectMenuMode, ObjectMenuAction,
    ObjectMenuCommand, ObjectMenuSelection, ObjectMenuState,
};
use clonk_app_netplay::control_message::{mentions_nick, ControlMessageState};
use clonk_app_netplay::network::{
    ClientSettings, HostSettings, LeagueEndAttempt, LeagueEndFailurePhase,
    LeagueRecordStreamStatus, NetworkControl, NetworkControlClock, NetworkEvent, NetworkEventWake,
    NetworkEventWakeCallback, NetworkManager, NetworkMode, NetworkStartError,
    NetworkStartupCancellation,
};
use clonk_app_netplay::network_host_preparation::NetworkHostPreparation;
use clonk_app_netplay::prepared_host_bootstrap::{
    PreparedHostBootstrap, PreparedHostPlayerIdentity, PreparedHostPlayerSource,
    ProcessInitialHostTeamAssignmentOracle, CLASSIC_SAFE_RANDOM_LOCK,
};
use clonk_app_netplay::{
    compose_client_network_scenario, load_configured_mission_access,
    load_snapshotted_client_players, publish_initial_configured_client_players,
    resolve_client_game_resources, resolve_client_scenario_resources,
    snapshot_configured_client_player_selection, ClientScenarioResources, ClientStartBarrier,
    ClientStartResourceRole, ConfiguredClientPlayerSelection, PendingClientStartResource,
    ResolvedClientStartResource, SelectedClientPlayer,
};
use clonk_audio::{AudioError, AudioSystem, ChannelId, MusicHandle, ResamplingMode, SoundHandle};
use clonk_core::{std_config::Config, std_markup::Markup};
use clonk_engine::command::CommandId;
use clonk_engine::player_file::PlayerFile;
use clonk_engine::scenario::{LegacyDefinitionResolver, ScenarioLoaderHead, ScenarioLobbyMetadata};
use clonk_engine::text_spec::{parse_text_spec, TextSpec};
use clonk_engine::ControlKeyName;
use clonk_engine::{
    ActionSpec, ActionState, AudioCommand, CommandKind, ControlButton, ControlClientRegistry,
    ControlCommand, ControlEvent, ControlPlayerInfoRegistry, Definition, Engine, EngineError,
    EngineState, EnvironmentSettings, JoinPlayerConfig, Landscape, LegacyCString, MaterialSet,
    MenuCommandKind, MenuCommandSelection, MenuRequestKind, MessageControlData, MessageKind,
    MissionAccessStore, MouseDragCarryableCursor, MouseDragSource, MouseWorldCursor,
    MovementProfile, ObjectId, ObjectSnapshot, ObjectUpdate, PlayerCommandControlData,
    PlayerConfig, PlayerSelectControlData, RgbColor, Scenario, ScenarioError,
    ScoreboardPresentationRequest, ScriptControlPolicy, ShowCommandsRequestStore,
    SimulationSnapshot, SkyConfig, SpawnConfig, SpeechPlaybackOutcome, SyncCheckPacket,
    TeamConfiguration, Vector2, FLAG_ALIGN_CENTER, FLAG_ALIGN_LEFT, FLAG_ALIGN_RIGHT, FLAG_BOTTOM,
    FLAG_HCENTER, FLAG_LEFT, FLAG_NO_BREAK, FLAG_RIGHT, FLAG_TOP, FLAG_VCENTER, FLAG_WIDTH_REL,
    FLAG_X_REL, FLAG_Y_REL, MESSAGE_TYPE_ALERT, MESSAGE_TYPE_ME, MESSAGE_TYPE_NORMAL,
    MESSAGE_TYPE_PRIVATE, MESSAGE_TYPE_SAY, MESSAGE_TYPE_SOUND, MESSAGE_TYPE_SYSTEM,
    MESSAGE_TYPE_TEAM, OWNER_NONE, PLAYER_VIEW_MODE_SCROLLING, PLAYER_VIEW_MODE_TARGET,
};
use clonk_frontend::clonk_fonts::expand_hotkey_markup;
use clonk_frontend::context_menu::{
    ClassicContextMenu, ClassicTooltipTracker, ContextMenuDirection, ContextMenuEntry,
    ContextMenuEvent, ContextMenuIcon, ContextMenuOutcome, ContextMenuPointerButton,
    ContextMenuSound,
};
use clonk_frontend::developer_console::{
    ConsoleClientRow, ConsoleEditMode, ConsolePathRequest, ConsolePlayerRow, ConsoleSaveKind,
    ConsoleStrings, ConsoleViewModel, DeveloperConsole, DeveloperConsoleAction,
    DeveloperConsoleKey,
};
use clonk_frontend::game_lobby::{
    core_lobby_option_rows, core_runtime_option_rows, team_lobby_option_rows,
    GameLobby as ClassicGameLobby, LobbyAction as ClassicLobbyAction, LobbyChatClipboardShortcut,
    LobbyChatContextCommand, LobbyChatEditKey, LobbyChatEditView, LobbyChatKeyModifiers,
    LobbyChatRequest, LobbyClientRow, LobbyClientStatus, LobbyControl, LobbyGameOptionInput,
    LobbyHeaderRow, LobbyJoinedPlayerOverlay, LobbyLabels, LobbyLayout, LobbyLogLine,
    LobbyOptionKind, LobbyOptionLabels, LobbyOptionRow, LobbyPlayerRow, LobbyResourceRow,
    LobbyResources, LobbyRole, LobbyRosterHeader, LobbyRosterIcon, LobbyRosterId,
    LobbyRosterLayout, LobbyRosterRow, LobbyScenarioText, LobbySheet, LobbySound,
    LobbyTeamOptionState, LobbyTeamValue,
};
use clonk_frontend::game_option_buttons::{
    FairCrewConstraint, GameOptionAction, GameOptionButton, GameOptionButtons, GameOptionContext,
    GameOptionGamepadDirection, GameOptionInputDialogRequest, GameOptionInputDialogResult,
    GameOptionInputKind, GameOptionSound, GameOptionValues,
};
use clonk_frontend::hud::MESSAGE_BOARD_MAX_FADING_LINES;
use clonk_frontend::input_dialog::{
    InputDialogAction, InputDialogClipboardShortcut, InputDialogContextCommand,
    InputDialogContextLabels, InputDialogControl, InputDialogController, InputDialogEditKey,
    InputDialogIcon, InputDialogKeyModifiers, InputDialogPlacement, InputDialogSound,
};
use clonk_frontend::loader_screen::{
    LoaderGuiProgress, LoaderRenderConfig, LoaderResources, LoaderScreen, LoaderSelection,
    LoaderState, LoaderUpdate, STARTUP_LOADER_SPECIFICATION,
};
use clonk_frontend::rename_edit::{
    RenameEdit, RenameEditAction, RenameEditCursorOperation, RenameEditResolution, RenameEditResult,
};
use clonk_frontend::startup_plrsel::{
    PlrSelControl, PlrSelCrewContextCommand, PlrSelPlayerContextCommand,
};
use clonk_frontend::{
    default_owner_color, viewport_edge_scroll, viewport_edge_scroll_at, ActiveViewportProjection,
    ColorByOwnerMask, CrewNameOverlay, CrewOverlay, CursorAtlas, CursorTiers,
    DefinitionDebugGeometry, DefinitionSprite, GamePalette, GraphicsOverlay, GraphicsSystem,
    GuiPoint, HudGraphics, ImageData, InputDispatcher, InventoryOverlay, KeyCode, MainMenuAction,
    MainMenuItem, MaterialRenderInfo, MaterialTextureSurface, MessageBoardMode,
    MessageBoardOverlay, MouseCursorPhase, ParticleFacet, ParticleRenderDefinition, PlayerOverlay,
    RenderedAudibilityCall, RenderedObjectAudibilityCalls, ScenarioEntry, ScenarioKind,
    SkyRenderState, SpeakingOverlay, StartupMainMenu, StartupMenu, StartupMenuAction,
    StartupTooltip, ViewportEdgeScroll, ViewportInput, ViewportPointer,
};
use clonk_graphics::clonk_font::{
    font_image_lookup_tag, inline_image_token, FontImageProvider, FontImageRef,
};
use clonk_graphics::{
    BitmapFont, Color, GpuGammaMode, GpuPresentation, GpuScene, GpuSceneRecorder, PixelFormat,
    Point as SurfacePoint, Rect, RgbaSurfaceViewMut, Surface, TextFont, Transform, TrueTypeFont,
};
use clonk_gui::{ButtonTextures, Rect as GuiRect};
use clonk_network::{
    ClientId, ClientPingSample, ControlRecordPlayback, ControlRecordWriter, LeagueEndRecord,
    NetworkStats, ParticipantKind, PlayerControlSample, ProtocolRateSample, Tick,
};
use clonk_platform::{discover_unvalidated_install_root, AppPaths, PathsError};
use clonk_resources::{
    load_endeavour_font, scenario as resource_scenario, DefCore as ResourceDefCore,
    DefinitionError as ResourceDefinitionError, FontCatalog, FontRole, GraphicsError,
    GraphicsImage, GraphicsResource, Group, GroupEntry, GroupError, LanguagePacks, MutableGroup,
    MutableGroupChildMut, MutableGroupEntryKind, ParticleDefinition as ResourceParticleDefinition,
    ResolvedFontSpec, ResourceDefinition as ResourceDefinitionData,
};
use clonk_surface::WindowSurface;
use control_options::format_key_label;
use desktop_notification::{DesktopNotification, DesktopNotifier};
use display_sleep_inhibitor::DisplaySleepInhibitor;
use gamepad::{
    GamepadActionType, GamepadEvent, GamepadManager, GamepadSlot, GuiButtonClass,
    LegacyGamepadAxis, LegacyGamepadButton, SourcedGamepadEvent,
};
use input::{ControlBindingId, GamepadBindings, KeyboardBindings};
use local_control::{KeyboardRoutingOutcome, LocalControlInit, LocalControlRegistry};
use network_team_assignment::{NetworkTeamAssignmentState, NetworkTeamControlError};
use offline_savegame::{prepare_offline_savegame_startup, OfflineSavegameStartup};
use offline_startup::{
    offline_player_paths_identical, offline_player_real_path, OfflineStartupPlayers,
};
use png::{BitDepth, ColorType, Encoder};
use serde::{
    de::{self, Unexpected, Visitor},
    ser::Serializer,
    Deserialize, Serialize,
};
use settings::{AudioOptions, DisplayMode, DisplayOptions};
use sha1::{Digest, Sha1};
use startup_player_files::{
    crew_file_name_for_title, delete_crew_file, delete_player_file, discover_crew_files,
    discover_player_files, discover_player_files_in, load_local_player_big_icon,
    load_network_player_big_icon, load_packed_network_player_big_icon,
    load_player_big_icon_from_group, persist_activations, player_group_filename, rename_crew,
    save_player_properties, set_crew_death_message, set_crew_participation,
    PlayerActivationRefusal, PlayerImageWrite, PlayerPropertiesSaveError, SavedStartupPlayer,
    StartupCrewFile, StartupCrewMutationError, StartupPlayerFile,
};
use strsim::damerau_levenshtein;
use time::{macros::format_description, OffsetDateTime};
use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{
    DeviceEvent, DeviceId, ElementState, Event, MouseButton, MouseScrollDelta, StartCause,
    TouchPhase, WindowEvent,
};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode as VirtualKeyCode, ModifiersState};
use winit::window::{Fullscreen, UserAttentionType, Window, WindowId};

type RuntimeEventHandler = Box<dyn FnMut(Event<NetworkEventWake>, &ActiveEventLoop)>;
type RuntimeInitializer = Box<dyn FnOnce(&ActiveEventLoop) -> Result<RuntimeEventHandler>>;

/// Bridges the legacy single-event callback onto winit's lifecycle API.
///
/// Window and surface creation deliberately happen in the first `resumed`
/// callback. Besides being winit's portable lifecycle contract, that keeps the
/// macOS fullscreen and Dock setup after AppKit has finished launching.
struct RuntimeApplication {
    initializer: Option<RuntimeInitializer>,
    handler: Option<RuntimeEventHandler>,
    startup_error: Option<anyhow::Error>,
}

impl RuntimeApplication {
    fn new(initializer: RuntimeInitializer) -> Self {
        Self {
            initializer: Some(initializer),
            handler: None,
            startup_error: None,
        }
    }

    fn initialize(&mut self, event_loop: &ActiveEventLoop) {
        let Some(initializer) = self.initializer.take() else {
            return;
        };
        match initializer(event_loop) {
            Ok(handler) => self.handler = Some(handler),
            Err(error) => {
                self.startup_error = Some(error);
                event_loop.exit();
            }
        }
    }

    fn dispatch(&mut self, event: Event<NetworkEventWake>, event_loop: &ActiveEventLoop) {
        if let Some(handler) = self.handler.as_mut() {
            handler(event, event_loop);
        }
    }

    fn finish(self) -> Result<()> {
        self.startup_error.map_or(Ok(()), Err)
    }
}

impl ApplicationHandler<NetworkEventWake> for RuntimeApplication {
    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
        self.dispatch(Event::NewEvents(cause), event_loop);
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.initialize(event_loop);
        self.dispatch(Event::Resumed, event_loop);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: NetworkEventWake) {
        self.dispatch(Event::UserEvent(event), event_loop);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        self.dispatch(Event::WindowEvent { window_id, event }, event_loop);
    }

    fn device_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        device_id: DeviceId,
        event: DeviceEvent,
    ) {
        self.dispatch(Event::DeviceEvent { device_id, event }, event_loop);
    }

    fn suspended(&mut self, event_loop: &ActiveEventLoop) {
        self.dispatch(Event::Suspended, event_loop);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.dispatch(Event::AboutToWait, event_loop);
    }

    fn exiting(&mut self, event_loop: &ActiveEventLoop) {
        self.dispatch(Event::LoopExiting, event_loop);
    }

    fn memory_warning(&mut self, event_loop: &ActiveEventLoop) {
        self.dispatch(Event::MemoryWarning, event_loop);
    }
}

fn main() -> Result<()> {
    let result = run();
    // `C4WinMain` reports a failure raised before the window exists through a
    // native dialog, in addition to the diagnostic it already wrote, and still
    // returns C4XRV_Failure (C4WinMain.cpp:97-117,274-289). Whether a dialog is
    // actually shown depends on the platform sink, so a target without one —
    // like C++'s Unix build without WITH_DEVELOPER_MODE — stays stderr-only.
    if let Err(error) = &result {
        // First, and unconditionally: the runtime prints a returned error to
        // stderr alone, which a windowed build has nowhere to show. winit's
        // Wayland loop reaches here after a failed `Connection::flush` without
        // logging anything itself
        // (`winit-0.30.13/src/platform_impl/linux/wayland/event_loop/mod.rs:284-287`),
        // so a lost compositor connection used to end the session with an
        // empty-looking log (clonk-org/clonk-rs#40).
        clonk_logging::log_fatal_error(&format!("{error:#}"));
        if !clonk_platform::startup_dialog::window_was_created() {
            let mut sink = native_startup_dialog_sink();
            clonk_platform::startup_dialog::report_startup_failure(
                &mut sink,
                clonk_platform::startup_dialog::headless_run_was_requested(),
                &format!("{error:#}"),
            );
        }
    }
    result
}

/// The dialog backend for this target. Only Windows has one, matching C++,
/// where the Unix path shows a dialog solely in developer builds.
#[cfg(windows)]
fn native_startup_dialog_sink() -> clonk_platform::startup_dialog::NativeStartupDialog {
    clonk_platform::startup_dialog::NativeStartupDialog
}

#[cfg(not(windows))]
fn native_startup_dialog_sink() -> clonk_platform::startup_dialog::NoStartupDialog {
    clonk_platform::startup_dialog::NoStartupDialog
}

/// The active monitor's refresh period in whole milliseconds (120 Hz -> 8 ms),
/// when winit can report it. Kept to a thin adapter because a `Window` cannot
/// be constructed in a unit test; the policy it feeds is tested separately in
/// `effective_max_refresh_delay_ms`.
fn display_refresh_period_ms(window: &Window) -> Option<u64> {
    window
        .current_monitor()
        .and_then(|monitor| monitor.refresh_rate_millihertz())
        .filter(|millihertz| *millihertz > 0)
        .map(|millihertz| (1_000_000 / u64::from(millihertz)).max(1))
}

fn recover_interrupted_update_before_path_discovery() -> Result<clonk_update::ResumeOutcome> {
    recover_interrupted_update_before_path_discovery_with(&clonk_update::RealPlatform)
}

fn recover_interrupted_update_before_path_discovery_with(
    platform: &dyn clonk_update::PlatformOps,
) -> Result<clonk_update::ResumeOutcome> {
    if std::env::var_os(clonk_update::UPDATE_RECOVERY_COMPLETE_ENV).as_deref()
        == Some(std::ffi::OsStr::new("1"))
    {
        return Ok(clonk_update::ResumeOutcome::NothingToDo);
    }
    let install_root = match discover_unvalidated_install_root() {
        Ok(install_root) => install_root,
        Err(PathsError::InstallRootNotFound) => {
            return Ok(clonk_update::ResumeOutcome::NothingToDo)
        }
        Err(error) => return Err(error.into()),
    };
    clonk_update::resume_interrupted_update_with(
        &clonk_update::InstallLayout::discover(&install_root),
        platform,
    )
    .context("failed to recover interrupted component update")
}

fn update_recovery_message(outcome: &clonk_update::ResumeOutcome) -> String {
    match outcome {
        clonk_update::ResumeOutcome::NothingToDo => {
            "no interrupted component update to recover".to_string()
        }
        clonk_update::ResumeOutcome::RolledForward { version } => {
            format!("completed interrupted component update to {version}")
        }
        clonk_update::ResumeOutcome::RolledBack { version } => {
            format!("rolled back interrupted component update to {version}")
        }
    }
}

fn run() -> Result<()> {
    // C++ recovers a translocated bundle path and chdirs to the directory
    // holding the .app before anything else (C4WinMain.cpp:233-238;
    // MacAppTranslocation.cpp:27-63). It must precede path discovery.
    #[cfg(target_os = "macos")]
    clonk_platform::establish_macos_bundle_working_directory();
    // Root is refused before the debug facilities and any initialization
    // (C4WinMain.cpp:251-255), so this precedes the handlers installed below.
    #[cfg(unix)]
    if let Some(refusal) = clonk_platform::privileges::root_startup_refusal_for_current_process(
        std::env::args().next().as_deref(),
    ) {
        println!("{refusal}");
        std::process::exit(clonk_platform::privileges::STARTUP_FAILURE_EXIT_CODE);
    }
    // `C4WinMain` installs the fatal-signal handlers before application
    // initialization (C4WinMain.cpp:256-265). The session log does not exist
    // yet, so the banner starts stderr-only and gains the log descriptor below.
    #[cfg(unix)]
    clonk_platform::crash::install(-1);
    // `C4WinMain.cpp:68-70` installs the unhandled-exception filter before
    // application initialization; it reads the user path and log descriptor
    // lazily, so both are published below once they exist.
    #[cfg(windows)]
    clonk_platform::crash_win32::install();
    // The classic GUI-build console policy runs before normal initialization:
    // debug builds always allocate, release builds only for `/allocconsole`,
    // and a failure aborts startup (C4WinMain.cpp:72-93).
    #[cfg(windows)]
    {
        let arguments: Vec<String> = std::env::args().collect();
        if clonk_platform::alloc_console::console_is_required(
            cfg!(debug_assertions),
            &arguments[..],
        ) {
            clonk_platform::alloc_console::allocate_console()?;
        }
    }
    // Must precede any output: the GUI subsystem starts with stdio detached.
    clonk_platform::attach_parent_console();
    let cli = Cli::parse();
    if let Some(report_path) = cli.headed_surface_smoke.as_deref() {
        headed_surface_smoke::prepare(report_path)?;
    }
    // The startup-failure reporter in `main` is installed before this point
    // and has no access to the parsed command line. A dedicated server must
    // never wait on a modal acknowledgement, so latch the choice as soon as it
    // is known.
    if cli.headless {
        clonk_platform::startup_dialog::note_headless_run();
    }
    let update_notice_detail = update_download::take_update_notice_detail();
    let classic = parse_classic_command_line(&cli.classic_arguments);
    install_classic_language_override(&classic);
    let console_log_capture = classic
        .console
        .then(clonk_logging::ConsoleLogCapture::default);
    // C4LogSystem installs its GuiSink for every session: the running message
    // board draws the C4Script log stream (src/C4Log.cpp:226-240).
    let game_log_capture = clonk_logging::GameLogCapture::default();

    let explicit_config = classic
        .config_file
        .as_deref()
        .or(cli.config_file.as_deref());
    let update_recovery = recover_interrupted_update_before_path_discovery()?;
    let app_paths = discover_validated_startup_paths(explicit_config)?;
    let _install_use = app_paths
        .as_ref()
        .map(|paths| {
            clonk_update::acquire_install_use(&clonk_update::InstallLayout::for_app_paths(paths))
        })
        .transpose()
        .context("the installation is being updated by another process")?;
    // The crash filter writes its dump under `Config.General.UserPath`
    // (C4CrashHandlerWin32.cpp:374-375).
    #[cfg(windows)]
    if let Some(paths) = app_paths.as_ref() {
        clonk_platform::crash_win32::set_user_path(&paths.user_data_dir().to_string_lossy());
    }
    // The definition loader reads this the way C++ reads the global `Config`
    // (C4Config.cpp:453; C4Def.cpp:555,1051).
    clonk_engine::scenario::verbose_loading::set_verbose_object_loading(
        load_verbose_object_loading(app_paths.as_deref()),
    );
    // `[Logging]` must reach the subscriber before it is installed below.
    clonk_logging::set_logging_config_directive(load_logging_config_directive(
        app_paths.as_deref(),
    ));
    // `C4Application::DoInit` sizes the global asynchronous pool from
    // `General.ThreadPoolThreadCount` on every non-Windows target
    // (C4Application.cpp:152-159). Must precede any worker thread.
    #[cfg(not(windows))]
    clonk_app_netplay::network::set_network_runtime_worker_threads(load_thread_pool_thread_count(
        app_paths.as_deref(),
    ));
    if let Some(paths) = app_paths.as_ref() {
        let log_path = paths.logs_dir().join("Clonk.log");
        match clonk_logging::init_verbose_with_file_and_capture(
            classic.verbose,
            &log_path,
            console_log_capture.clone(),
            Some(game_log_capture.clone()),
        ) {
            Ok(()) => {
                // The crash banner now has a log to write to, like `GetLogFD`
                // once the session log exists (C4WinMain.cpp:199-209).
                #[cfg(unix)]
                clonk_platform::crash::set_log_descriptor(clonk_logging::crash_log_descriptor());
                #[cfg(windows)]
                clonk_platform::crash_win32::set_log_descriptor(
                    clonk_logging::crash_log_descriptor(),
                );
                tracing::info!(
                    path = %log_path.display(),
                    "engine session log initialized"
                );
            }
            Err(err) => tracing::warn!(
                error = %err,
                path = %log_path.display(),
                "failed to initialize engine session log; continuing with stderr logging"
            ),
        }
    } else {
        clonk_logging::init_verbose_with_capture(
            classic.verbose,
            console_log_capture.clone(),
            Some(game_log_capture.clone()),
        );
    }
    clonk_logging::install_panic_hook();
    clonk_logging::log_startup_banner(
        clonk_core::version::PORT_VERSION,
        clonk_core::version::ENGINE_VERSION_COMPACT,
    );
    tracing::info!("{}", update_recovery_message(&update_recovery));
    if let Some(paths) = app_paths.as_ref() {
        if let Err(err) = paths.ensure_user_dirs() {
            tracing::warn!(
                error = %err,
                path = %paths.user_data_dir().display(),
                "failed to ensure user data directories"
            );
        }
        match repair_rust_truncated_masterserver_urls(&paths.config_file()) {
            Ok(true) => tracing::info!(
                path = %paths.config_file().display(),
                "repaired masterserver URLs truncated by the old Rust config parser"
            ),
            Ok(false) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => tracing::warn!(
                error = %err,
                path = %paths.config_file().display(),
                "failed to repair Rust-truncated masterserver URLs"
            ),
        }
    }
    // Handle test-load mode: load scenario and exit without starting UI
    if let Some(test_path) = &cli.test_load {
        return test_scenario_load(test_path, app_paths.as_ref());
    }

    let runtime = RuntimeConfig {
        player_owner: cli.player_owner,
        player_name: cli.player_name.clone(),
        network: resolve_network_mode(&cli, &classic, app_paths.as_deref())?,
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

    // Ahead of `validate_classic_loader_graphics_config` deliberately: the
    // loader screen it checks is a rendered artifact, and a dedicated server
    // never builds one — `C4Application::PreInit` only calls
    // `InitLoaderScreen` for a startup-dialog run (C4Application.cpp:239).
    if cli.headless {
        return run_headless_server(&cli, &classic, app_paths.as_ref(), runtime);
    }
    let audio_options = AudioOptions::load(app_paths.as_deref());
    if let Some(paths) = app_paths.as_deref() {
        validate_classic_loader_graphics_config(paths).map_err(|error| {
            anyhow::Error::new(report_classic_parity_boundary(
                ClassicParityBoundary::LoaderScreen {
                    context: "startup loading",
                    detail: error.to_string(),
                },
            ))
        })?;
    }
    let mut display_options = DisplayOptions::load(app_paths.as_deref());
    let mut event_loop_builder = EventLoop::<NetworkEventWake>::with_user_event();
    display_backend::apply_display_backend(
        &mut event_loop_builder,
        display_backend::select_display_backend(
            cli.display_server,
            &display_backend::DisplayServerEnvironment::from_env(),
            display_backend::steam_input_pad_present(),
        ),
    );
    let event_loop = event_loop_builder
        .build()
        .context("failed to create application event loop")?;
    // winit's macOS proxy is Send but not Sync. The network sender can be
    // cloned across worker tasks, so serialize proxy access behind a mutex.
    let network_event_proxy = Arc::new(Mutex::new(event_loop.create_proxy()));
    let benchmark_exit_code = Arc::new(AtomicI32::new(0));
    let event_handler_exit_code = Arc::clone(&benchmark_exit_code);
    let initializer: RuntimeInitializer = Box::new(move |event_target| {
        // The active event loop is the first portable point at which monitors can
        // be queried and a native window plus render surface can be created.
        if let Some(scale_factor) = event_target
            .primary_monitor()
            .or_else(|| event_target.available_monitors().next())
            .map(|monitor| monitor.scale_factor())
        {
            if display_options.apply_first_run_display_scale(scale_factor) {
                tracing::info!(
                    scale_factor,
                    scale_percent = display_options.scale_percent(),
                    "seeded the first-run application scale from the display density"
                );
            }
        }
        let (initial_width, initial_height) = if classic.console {
            // C4Console's GTK shell uses a 320x320 native-pixel default and never
            // inherits the fullscreen game window configuration.
            display_options.mode = DisplayMode::Window;
            display_options.maximized = false;
            // `C4Console::RestorePosition` applies the console's own `Console/Main`
            // slot right after the window is created (C4Console.cpp:296-305). It
            // carries a position only — the 320x320 default size stands, and the
            // game window's geometry is neither read nor written.
            display_options.position =
                load_console_window_position(app_paths.as_deref()).and_then(|placement| {
                    use crate::console_window_position::ConsoleWindowPlacement;
                    if matches!(
                        placement,
                        ConsoleWindowPlacement::Maximized | ConsoleWindowPlacement::Minimized
                    ) {
                        // C++ shows the window zoomed or iconic rather than moving
                        // it (StdRegistry.cpp:310-313); the port has no console
                        // equivalent, so it falls back to platform placement.
                        tracing::debug!("ignoring a non-positional console window placement");
                    }
                    placement.position()
                });
            (320, 320)
        } else {
            display_options
                .checked_loader_actual_size()
                .map_err(|detail| {
                    anyhow::Error::new(report_classic_parity_boundary(
                        ClassicParityBoundary::LoaderScreen {
                            context: "startup loading",
                            detail,
                        },
                    ))
                })?
        };
        let desktop_notifier = match DesktopNotifier::initialize() {
            Ok(notifier) => notifier,
            Err(error) => {
                tracing::warn!(%error, "failed to initialize desktop notification system");
                None
            }
        };
        // The stored resolution is in output pixels (ResX*Scale), like the C++
        // window setup (C4Application.cpp:183).
        let window = Arc::new(
            event_target
                .create_window(startup_window_attributes(
                    &display_options,
                    PhysicalSize::new(initial_width, initial_height),
                ))
                .context("failed to create application window")?,
        );
        // Past this point a failure is no longer a startup failure, so it is
        // reported by the running application rather than a native dialog.
        clonk_platform::startup_dialog::note_window_created();
        // `C4Application::DoInit` registers the file classes in the graphical
        // Windows build only, best-effort — C++ notes it "will only work if we have
        // administrator rights" and ignores the result (C4Application.cpp:219-223).
        #[cfg(windows)]
        if !classic.console {
            let registered = std::env::current_exe()
                .ok()
                .map(|module| {
                    clonk_platform::file_classes::register_file_classes(&module.to_string_lossy())
                })
                .unwrap_or(false);
            if !registered {
                tracing::debug!("could not register the Clonk file classes");
            }
        }
        if classic.console {
            window.set_title(native_window_title(true));
        }
        let mut display_sleep_inhibitor = DisplaySleepInhibitor::acquire();
        if display_options.maximized && matches!(display_options.mode, DisplayMode::Window) {
            window.set_maximized(true);
        }

        let size = enforce_min_size(window.inner_size());
        let pixels = build_framebuffer(&window, size)?;
        let mut retained_gpu_renderer = gpu_renderer::RetainedGpuRenderer::new(
            pixels.device(),
            pixels.queue(),
            pixels.surface_texture_format(),
        );
        let renderer_config = load_native_config_bytes(app_paths.as_deref());
        retained_gpu_renderer.set_mipmaps(configured_mipmaps(&renderer_config));
        retained_gpu_renderer.set_smooth_landscape(configured_smooth_landscape(&renderer_config));
        retained_gpu_renderer.set_shader_landscape(configured_shader_landscape(&renderer_config));
        retained_gpu_renderer.set_landscape_detail(configured_landscape_detail(&renderer_config));

        // The app lays out and renders at the GUI resolution; the presenter
        // scales the finished frame to the window like the C++ engine scales
        // its GUI output (C4Gui.cpp:461).
        let presenter = clonk_scaling::FramePresenter::new(
            if classic.console {
                1.0
            } else {
                display_options.scale
            },
            size.width,
            size.height,
        );
        let (logical_width, logical_height) = presenter.logical_size();

        let mut app = GameApp::new(
            logical_width,
            logical_height,
            audio_options,
            app_paths.as_deref(),
            runtime,
        )
        .context("failed to initialise app state")?;
        let update_failure_prefix =
            app.runtime_resource_text("IDS_MSG_UPDATEFAILED", "Update failed.");
        if let Some(message) = update_download::update_notice_message(
            &update_failure_prefix,
            update_notice_detail.as_deref(),
        ) {
            let caption = app.update_check_caption();
            app.show_update_notice(message, caption)?;
        }
        app.console_mode = classic.console;
        app.console_log_capture = console_log_capture;
        app.game_log_capture = Some(game_log_capture);
        if classic.console {
            arm_configured_engine_debug_mode(&mut app.engine, app_paths.as_deref(), true);
        }
        app.window_active = initial_window_active();
        // C++ only has an `ITaskbarList3` once `CStdWindow` owns a handle, so the
        // real sink replaces the no-op one here rather than in `GameApp::new`.
        // Everything but Windows keeps the no-op, matching the SDL and X11
        // `CStdWindow` implementations, which are no-ops there too.
        //
        // The handle is extracted unconditionally so this compiles and is checked
        // on every host: `RawWindowHandle::Win32` exists on all platforms, and only
        // the sink construction is target-gated. `clonk-app` cannot be
        // cross-checked for Windows (stacker's C build needs an MSVC toolchain), so
        // keeping the untestable part to two lines is deliberate.
        let taskbar_window = window
            .window_handle()
            .ok()
            .and_then(|handle| match handle.as_raw() {
                RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
                _ => None,
            });
        #[cfg(windows)]
        if let Some(handle) = taskbar_window {
            // SAFETY: winit initialised COM on this thread and owns the window for
            // the rest of the process.
            if let Some(sink) =
                unsafe { clonk_platform::taskbar_progress::Win32TaskbarProgress::new(handle) }
            {
                app.taskbar_progress.replace_sink(Box::new(sink));
            }
        }
        #[cfg(not(windows))]
        let _ = taskbar_window;

        // Retained for the event loop's inactive-draw gate (C4Config.cpp:481).
        let render_inactive_mask = load_render_inactive_mask(app.app_paths.as_ref());
        // `GameApp::new` resolves the refresh ceiling before any window exists, so
        // the panel period can only be substituted here (opt-in; see
        // `configured_smooth_presentation`).
        app.display_refresh_period_ms = display_refresh_period_ms(&window);
        app.startup_refresh_delay_ms = effective_max_refresh_delay_ms(
            &load_native_config_bytes(app.app_paths.as_ref()),
            app.display_refresh_period_ms,
        );
        app.set_display_mode(display_options.mode);
        app.graphics
            .set_runtime_sprite_filtering(presenter.scale(), display_options.point_filtering);
        app.configure_native_startup_fonts(presenter.scale(), display_options.point_filtering);
        app.apply_classic_command_line(&classic)?;
        app.auto_start_sandbox = cli.sandbox;
        app.launch_classic_command_line_join()
            .context("failed to start command-line network join")?;
        app.launch_classic_command_line_scenario()
            .context("failed to start command-line scenario")?;
        app.install_network_event_waker(Arc::new(move |wake| {
            if let Ok(proxy) = network_event_proxy.lock() {
                let _ = proxy.send_event(wake);
            }
        }));
        let mut console_commands = classic
            .console
            .then(spawn_console_stdin_reader)
            .transpose()?;

        let mut deferred_fullscreen_retry_at = None;
        let mut previous_instant = Instant::now();
        let mut accumulator = Duration::ZERO;
        let mut game_clock_accumulator = Duration::ZERO;
        let mut frame_schedule = frame_schedule_for_mode(
            app.mode,
            app.engine.game_tick_delay_ms(),
            app.engine.game_tick_delay_revision(),
            app.refresh_ceilings(),
        );
        let mut next_graphics_deadline = previous_instant + frame_schedule.refresh_interval;
        let mut automatic_frame_skip = AutomaticFrameSkip::default();
        let mut render_floor = RenderFloor::default();
        let mut presentation_detail = PresentationDetailGovernor::default();
        let mut presentation_benchmark = presentation_benchmark_from_env();
        let mut presentation_benchmark_runtime_readiness =
            PresentationBenchmarkRuntimeReadiness::default();
        let presentation_benchmark_asserts_native_tick =
            presentation_benchmark_asserts_native_tick();
        let presentation_benchmark_keeps_running = presentation_benchmark_keeps_running();

        // The shell is a registry record like every other developer window, so a
        // WindowId arriving from winit resolves to a purpose before it is
        // routed. It is the only record until the console opens its own.
        let mut developer_windows: developer_windows::DeveloperWindows<
            developer_host::DeveloperHost,
        > = developer_windows::DeveloperWindows::new();
        developer_windows.insert(
            developer_windows::SHELL_WINDOW,
            developer_windows::HostPurpose::Shell,
            developer_host::DeveloperHost::Shell(shell_window_host::ShellWindowHost::new(
                window,
                pixels,
                presenter,
                retained_gpu_renderer,
            )),
        );
        // The next viewport window's key. `SHELL_WINDOW` is 0, so console windows
        // start above it; the value is a registry key, not a viewport identity.
        let mut next_developer_window_key = 1u64;
        let mut headed_surface_smoke = cli
            .headed_surface_smoke
            .clone()
            .map(|report_path| {
                headed_surface_smoke::HeadedSurfaceSmoke::start(
                    report_path,
                    event_target,
                    &mut developer_windows,
                    &mut next_developer_window_key,
                )
            })
            .transpose()?;
        // Set when the shell takes a graphics pass; consumed on the next event
        // loop entry, before the shell record is borrowed.
        let mut viewport_redraw_pending = false;
        // The component editor's own modifier state. Each developer window
        // sees only its own `ModifiersChanged`, so a shared field would hold
        // whatever the last *other* window left there.
        let mut component_editor_modifiers = winit::keyboard::ModifiersState::empty();

        let mut dock_tile_attached = false;
        Ok(Box::new(move |event, event_target| {
            if let Some(smoke) = headed_surface_smoke.as_mut() {
                let (outcome, consumed) = match &event {
                    Event::AboutToWait => (
                        smoke.about_to_wait(event_target, &mut developer_windows),
                        true,
                    ),
                    Event::WindowEvent {
                        window_id,
                        event: WindowEvent::RedrawRequested,
                    } => (
                        smoke.redraw(*window_id, event_target, &mut developer_windows),
                        true,
                    ),
                    Event::LoopExiting => (Ok(()), false),
                    _ => (Ok(()), false),
                };
                if let Err(error) = outcome {
                    tracing::error!(%error, "headed surface smoke failed");
                    smoke.fail(error);
                    event_handler_exit_code.store(1, AtomicOrdering::Relaxed);
                    event_target.exit();
                }
                if consumed {
                    return;
                }
            }
            // Before the window borrow below, because the Dock tile belongs to the
            // application rather than to any one window.
            if dock_icon::should_attach_dock_tile(&event, dock_tile_attached) {
                dock_icon::set_dock_icon();
                dock_tile_attached = true;
            }
            // `C4GraphicsSystem` opens and closes a viewport's window inside
            // Create/CloseViewport (`C4GraphicsSystem.cpp:229-240,205-224`). winit
            // can only create a window from the event loop's target, so the same
            // decisions are taken here instead, before the shell record is
            // borrowed for the rest of the pass.
            if app.console_mode && matches!(event, Event::AboutToWait) {
                let scale = developer_windows
                    .shell_mut()
                    .and_then(developer_host::DeveloperHost::as_shell_mut)
                    .map_or(1.0, |shell| shell.presenter.scale());
                console_viewport_windows::reconcile_console_viewport_windows(
                    &mut app,
                    &mut developer_windows,
                    &mut next_developer_window_key,
                    scale,
                    event_target,
                );
                // The `C4DevmodeDlg` notebook needs the same treatment for the
                // same reason: `AddPage`/`SwitchPage` act on their window
                // inside the call that decided to, and winit cannot.
                console_toolbox_window::reconcile_developer_toolbox_window(
                    &mut app,
                    &mut developer_windows,
                    &mut next_developer_window_key,
                    event_target,
                );
                console_toolbox_window::reconcile_developer_object_list_window(
                    &mut app,
                    &mut developer_windows,
                    &mut next_developer_window_key,
                    event_target,
                );
                console_toolbox_window::reconcile_developer_component_editor_window(
                    &mut app,
                    &mut developer_windows,
                    &mut next_developer_window_key,
                    event_target,
                );
            }
            // Every viewport window redraws with the shell and only with it, the
            // way `C4GraphicsSystem::Execute` runs `cvp->Execute()` for each
            // viewport inside one graphics pass (`:167-169`). Redrawing them per
            // event-loop pass instead would ignore the frame schedule, the
            // automatic frame skip and the repaint floor, and spin.
            if std::mem::take(&mut viewport_redraw_pending) {
                // The toolbox rides the same pass: its pages read live engine
                // state — the landscape mode, the selection — so a page left
                // to repaint only on its own events would show a stale one.
                // Hidden records are skipped, as a hidden GTK window draws
                // nothing.
                developer_windows.request_redraw_visible();
            }
            // An event naming a viewport window is that window's alone. Resolving
            // it before the shell destructure keeps the shell arms — all of which
            // already guard on `window.id()` — exactly as they were.
            if let Some(os_window) = console_viewport_windows::event_window_id(&event) {
                if let Some(key) = developer_windows
                    .find_key(|host| host.window().id() == os_window)
                    .filter(|key| *key != developer_windows::SHELL_WINDOW)
                {
                    // The toolbox is a child window too, but its events are
                    // nothing like a viewport's — it has no edit cursor, no
                    // projection and no player lock.
                    if console_toolbox_window::toolbox_window_key(&developer_windows) == Some(key) {
                        console_toolbox_window::handle_developer_toolbox_event(
                            key,
                            &event,
                            &mut app,
                            &mut developer_windows,
                        );
                        return;
                    }
                    if console_toolbox_window::object_list_window_key(&developer_windows)
                        == Some(key)
                    {
                        console_toolbox_window::handle_developer_object_list_event(
                            key,
                            &event,
                            &mut app,
                            &mut developer_windows,
                        );
                        return;
                    }
                    if console_toolbox_window::component_editor_window_key(&developer_windows)
                        == Some(key)
                    {
                        console_toolbox_window::handle_developer_component_editor_event(
                            key,
                            &event,
                            &mut app,
                            &mut developer_windows,
                            &mut component_editor_modifiers,
                        );
                        return;
                    }
                    console_viewport_windows::handle_console_viewport_event(
                        key,
                        &event,
                        &mut app,
                        &mut developer_windows,
                    );
                    return;
                }
            }
            // Read before the match, which moves out of `event`, and acted on
            // after the shell borrow below has ended.
            let loop_is_exiting = matches!(event, Event::LoopExiting);
            let shell_window_host::ShellWindowHost {
                window,
                pixels: pixels_slot,
                presenter,
                renderer: retained_gpu_renderer,
                surface_rebuild,
                ..
            } = developer_windows
                .shell_mut()
                .expect("the console shell record lives for the whole process")
                .as_shell_mut()
                .expect("the reserved shell key holds the shell host");
            match event {
                Event::Resumed => {
                    if reconcile_deferred_fullscreen(window, display_options.mode) {
                        deferred_fullscreen_retry_at =
                            Some(Instant::now() + DEFERRED_FULLSCREEN_RETRY_DELAY);
                    }
                }
                Event::WindowEvent { window_id, event }
                    if window_id == window.id()
                        && !matches!(event, WindowEvent::RedrawRequested) =>
                {
                    let Some(pixels) = pixels_slot.as_mut() else {
                        return;
                    };
                    if let Err(err) = handle_window_event(
                        window,
                        &mut app,
                        pixels,
                        presenter,
                        &mut display_options,
                        event,
                        event_target,
                    ) {
                        tracing::error!(error = ?err, "window event handling failed");
                        event_target.exit();
                    }
                }
                Event::UserEvent(wake) => app.note_network_event_wake(wake),
                Event::AboutToWait => {
                    // Network managers may be replaced by asynchronous menu/lobby
                    // transitions. Carry the process event-loop wake handle onto
                    // the currently live manager before this pass can block.
                    app.refresh_network_event_waker();
                    let mut close_console_commands = false;
                    if let Some(commands) = console_commands.as_ref() {
                        loop {
                            match commands.try_recv() {
                                Ok(ConsoleInputEvent::Command(command)) => {
                                    if let Err(error) = app.process_console_command(&command) {
                                        tracing::error!(%error, command, "console command failed");
                                    }
                                }
                                Ok(ConsoleInputEvent::Eof) => {
                                    // The native developer window remains usable
                                    // when its optional terminal input closes.
                                    close_console_commands = true;
                                    break;
                                }
                                Ok(ConsoleInputEvent::Error(error)) => {
                                    tracing::warn!(%error, "console stdin reader stopped");
                                    close_console_commands = true;
                                    break;
                                }
                                Err(TryRecvError::Empty) => break,
                                Err(TryRecvError::Disconnected) => {
                                    close_console_commands = true;
                                    break;
                                }
                            }
                        }
                    }
                    if close_console_commands {
                        console_commands = None;
                    }
                    app.console_edit_cursor_tick();
                    app.poll_developer_file_monitor();
                    app.drain_console_log_capture();
                    app.drain_game_log_capture();
                    // Ahead of every path that can leave this arm early, so a
                    // password the last frame's script earned is already on disk
                    // whichever way the process ends.
                    app.persist_mission_access_if_changed();
                    if app.sync_developer_console_view() {
                        window.set_title(&app.developer_console.view_model().caption);
                        window.request_redraw();
                    }
                    if let Err(err) = app.process_gamepad_events() {
                        tracing::error!(error = ?err, "gamepad input failed");
                        event_target.exit();
                        return;
                    }
                    if let Err(err) = apply_options_display_requests(
                        window,
                        &mut app,
                        presenter,
                        &mut display_options,
                        app_paths.as_deref(),
                    ) {
                        tracing::error!(error = ?err, "options display change failed");
                        event_target.exit();
                        return;
                    }
                    // A post-launch native fullscreen transition can still fail
                    // during another macOS Space animation. winit restores its
                    // state to windowed on failure, so retry the configured mode.
                    if deferred_fullscreen_retry_at
                        .is_some_and(|retry_at| Instant::now() >= retry_at)
                    {
                        deferred_fullscreen_retry_at =
                            reconcile_deferred_fullscreen(window, display_options.mode)
                                .then(|| Instant::now() + DEFERRED_FULLSCREEN_RETRY_DELAY);
                    }
                    if app.take_exit_request() {
                        event_target.exit();
                        return;
                    }
                    let now = Instant::now();
                    let frame_time = now.saturating_duration_since(previous_instant);
                    previous_instant = now;
                    if let Err(err) = advance_game_clock_from_elapsed(
                        &mut app,
                        &mut game_clock_accumulator,
                        frame_time,
                    ) {
                        tracing::error!(error = ?err, "one-second timer failed");
                        event_target.exit();
                        return;
                    }
                    let previous_frame_schedule = frame_schedule;
                    // SetGameTickDelay installs a new timer when C++ enters or
                    // leaves the running game. Do not carry a fractional tick
                    // from the old cadence across that boundary
                    // (C4Application.cpp:510-531; C4Game.cpp:443).
                    accumulate_frame_time_for_mode(
                        app.mode,
                        app.engine.game_tick_delay_ms(),
                        app.engine.game_tick_delay_revision(),
                        app.refresh_ceilings(),
                        &mut frame_schedule,
                        &mut accumulator,
                        frame_time,
                    );
                    if app.mode == AppMode::Running && app.full_speed {
                        // C4Application::NextTick(false) drives one unpaced game
                        // iteration. Do not let wall-clock debt create a second
                        // source of fast-forward ticks.
                        accumulator = Duration::ZERO;
                    }

                    let burst_budget = simulation_burst_budget_before(
                        render_floor.simulation_burst_budget(frame_schedule.simulation_interval),
                        now,
                        next_graphics_deadline,
                    );
                    // The runtime/input benchmark follows the scheduler even
                    // when a covered window has no drawable surface. Poll
                    // before probes and simulation so its window is exactly
                    // half-open: [started, deadline).
                    let benchmark_now = Instant::now();
                    let benchmark_runtime_ready =
                        presentation_benchmark_runtime_readiness.ready(app.mode);
                    if let Some(mut report) =
                        presentation_benchmark.as_mut().and_then(|benchmark| {
                            benchmark.poll_with_runtime_stippel_census(
                                benchmark_runtime_ready,
                                benchmark_now,
                                app.engine.frame(),
                                || runtime_stippel_object_count(&app.snapshot),
                            )
                        })
                    {
                        let profile_line = pixels_slot
                            .as_ref()
                            .context("presentation benchmark ended without a drawable surface")
                            .and_then(|pixels| {
                                finish_retained_gpu_profile_artifact(
                                    &mut report,
                                    pixels,
                                    retained_gpu_renderer,
                                    app.graphics.advanced_renderer_config(),
                                    presenter.presentation_geometry(),
                                )
                            });
                        let profile_line = match profile_line {
                            Ok(profile_line) => profile_line,
                            Err(error) => {
                                eprintln!(
                                    "LC_APP_PRESENTATION_BENCHMARK result=fail error={error:#}"
                                );
                                event_handler_exit_code.store(2, AtomicOrdering::Relaxed);
                                event_target.exit();
                                return;
                            }
                        };
                        println!("{profile_line}");
                        finish_app_presentation_benchmark(
                            event_target,
                            &event_handler_exit_code,
                            &app,
                            report,
                            presentation_benchmark_asserts_native_tick,
                            presentation_benchmark_keeps_running,
                        );
                        if !presentation_benchmark_keeps_running {
                            return;
                        }
                    }
                    if let Some((started, deadline)) = presentation_benchmark
                        .as_ref()
                        .and_then(PresentationBenchmark::measurement_window)
                    {
                        let probe_now = Instant::now();
                        if probe_now < deadline {
                            app.submit_due_input_latency_benchmark_pair(started, probe_now);
                        }
                    }
                    let simulation_pass = match advance_simulation_pass_within(
                        &mut app,
                        &mut frame_schedule,
                        &mut accumulator,
                        burst_budget,
                    ) {
                        Ok(outcome) => outcome,
                        Err(err) => {
                            tracing::error!(error = ?err, "tick failed");
                            event_target.exit();
                            return;
                        }
                    };
                    presentation_benchmark_runtime_readiness
                        .observe(app.mode, simulation_pass.executed_frames);
                    if simulation_pass.skipped_render_frames > 0 {
                        tracing::trace!(
                            frames = simulation_pass.executed_frames,
                            skipped_renders = simulation_pass.skipped_render_frames,
                            "simulation pacing skipped intermediate renders"
                        );
                    }

                    if app.take_user_attention_request() {
                        window.request_user_attention(Some(UserAttentionType::Informational));
                    }
                    deliver_desktop_notifications(&mut app, |notification| {
                        desktop_notifier
                            .as_ref()
                            .map_or(Ok(()), |notifier| notifier.show(notification))
                    });
                    let graphics_now = Instant::now();
                    if frame_schedule != previous_frame_schedule {
                        // SetGameTickDelay replaces the application timer. Anchor
                        // the first graphics opportunity to the new interval so
                        // an old partial period cannot leak across game modes.
                        next_graphics_deadline = graphics_now + frame_schedule.refresh_interval;
                    }

                    // Window/input/network events may wake Winit early. Native
                    // graphics still run only on the application timer, so keep
                    // an absolute deadline instead of treating every wake as a
                    // decoupled graphics opportunity.
                    // The repaint floor outranks every skip decision: `/fast N`,
                    // the network catch-up divisor and a long burst can each
                    // suppress graphics indefinitely, and a frozen window is worse
                    // than a late one.
                    let repaint_overdue = render_floor.must_present(graphics_now);
                    let graphics_due = simulation_pass.immediate_network_retry
                        || repaint_overdue
                        || graphics_now >= next_graphics_deadline;
                    if graphics_due {
                        next_graphics_deadline = if simulation_pass.immediate_network_retry {
                            graphics_now + frame_schedule.refresh_interval
                        } else {
                            advance_graphics_deadline(
                                next_graphics_deadline,
                                graphics_now,
                                frame_schedule.refresh_interval,
                            )
                        };
                        if simulation_pass.skip_redraw && !repaint_overdue {
                            // Native's manual/network DoSkipFrame takes this same
                            // graphics opportunity and clears the shared latch.
                            automatic_frame_skip.consume_suppressed_graphics_pass();
                        } else {
                            if repaint_overdue {
                                // An overdue repaint must also outrank the
                                // automatic latch, or a slow graphics pass keeps
                                // re-arming the very skip the floor exists to stop.
                                automatic_frame_skip.consume_suppressed_graphics_pass();
                                // `apply_render_floor` counts frames since the last
                                // redraw and has already run for this pass. Drawing
                                // on the wall-clock floor instead would otherwise
                                // leave its counter climbing against a screen that
                                // did update, firing it a second time for nothing.
                                app.frames_since_redraw = 0;
                            }
                            window.request_redraw();
                            viewport_redraw_pending = true;
                        }
                    }
                    if app.mode != AppMode::Running {
                        automatic_frame_skip.consume_suppressed_graphics_pass();
                    }

                    if app.mode == AppMode::Running && app.full_speed {
                        event_target.set_control_flow(ControlFlow::Poll);
                    } else {
                        let simulation_deadline = now
                            + frame_schedule
                                .simulation_interval
                                .saturating_sub(accumulator);
                        event_target.set_control_flow(ControlFlow::WaitUntil(
                            next_graphics_deadline.min(simulation_deadline),
                        ));
                    }
                }
                // `C4GraphicsSystem::StartDrawing` refuses to draw while the
                // application is inactive unless `Graphics.RenderInactive` carries
                // the active shell's bit (C4GraphicsSystem.cpp:96-106) — which the
                // shipped default does for both shells — and never draws a window
                // the display server has hidden. Placed ahead of the frame-skip
                // arm so a suppressed frame costs nothing.
                Event::WindowEvent {
                    window_id,
                    event: WindowEvent::RedrawRequested,
                } if window_id == window.id()
                    && !render_inactive_allows_drawing(
                        render_inactive_mask,
                        app.window_active,
                        app.console_mode,
                        render_floor.has_presented(),
                        app.window_occluded,
                    ) =>
                {
                    // The opportunity was still consumed. Leaving the repaint floor
                    // armed would make every later event-loop pass take one, which
                    // both spins and banks graphics-deadline debt.
                    render_floor.note_refused_presentation(Instant::now());
                }
                Event::WindowEvent {
                    window_id,
                    event: WindowEvent::RedrawRequested,
                } if window_id == window.id()
                    && automatic_frame_skip.begin_graphics_pass(
                        app.mode == AppMode::Running && app.auto_frame_skip,
                    ) =>
                {
                    tracing::trace!("automatic frame skip consumed one graphics pass");
                    app.presentation_stats.record_automatic_graphics_skip();
                    if let Some(benchmark) = presentation_benchmark.as_mut() {
                        benchmark.record_automatic_graphics_skip();
                    }
                }
                Event::WindowEvent {
                    window_id,
                    event: WindowEvent::RedrawRequested,
                } if window_id == window.id() => {
                    let graphics_started = Instant::now();
                    app.graphics.set_presentation_scale(presenter.scale());
                    if matches!(
                        app.mode,
                        AppMode::Menu | AppMode::Loading | AppMode::Running
                    ) && should_attempt_retained_gpu_presentation(
                        retained_gpu_renderer.requires_cpu_presentation(),
                    ) {
                        app.retained_gpu_presentation_active = true;
                        let Some(pixels) = pixels_slot.as_mut() else {
                            return;
                        };
                        if pixels.buffer_extent().0 != 1 || pixels.buffer_extent().1 != 1 {
                            if let Err(error) = pixels.resize_buffer(1, 1) {
                                tracing::error!(%error, "failed to enter retained GPU presentation");
                                event_target.exit();
                                return;
                            }
                        }
                        let fallback_to_cpu = match present_retained_gpu_frame_profiled(
                            &mut app,
                            pixels,
                            presenter,
                            retained_gpu_renderer,
                        ) {
                            Ok(RetainedGpuProfiledOutcome::Presented(profile)) => {
                                surface_rebuild.note_presented();
                                if app.mode == AppMode::Running && !app.console_mode {
                                    app.finish_rendered_object_audibility_pass();
                                }
                                let graphics_duration = graphics_started.elapsed();
                                automatic_frame_skip.finish_graphics_pass(
                                    app.auto_frame_skip,
                                    graphics_duration,
                                    frame_schedule.simulation_interval,
                                );
                                render_floor
                                    .record_presentation(graphics_started, graphics_duration);
                                app.presentation_stats
                                    .record_presentation(graphics_duration);
                                presentation_detail.record_graphics_pass(
                                    app.mode == AppMode::Running && app.auto_frame_skip,
                                    graphics_duration,
                                    frame_schedule.simulation_interval,
                                );
                                app.presentation_detail = presentation_detail.detail();
                                if let Some(benchmark) = presentation_benchmark.as_mut() {
                                    let completed_at = Instant::now();
                                    benchmark.record_successful_retained_gpu_presentation(
                                        completed_at,
                                        graphics_duration,
                                        true,
                                        profile,
                                    );
                                }
                                let timestamp_frames = retained_gpu_renderer
                                    .take_completed_timestamp_frames(pixels.device());
                                if let Some(benchmark) = presentation_benchmark.as_mut() {
                                    benchmark.record_gpu_timestamp_frames(timestamp_frames);
                                }
                                false
                            }
                            Ok(RetainedGpuProfiledOutcome::Skipped) => {
                                // Pixels could not acquire a visible surface frame.
                                // The attempt consumed this graphics opportunity;
                                // retry on the normal refresh schedule without
                                // treating it as a presentation or spinning while
                                // the surface remains occluded.
                                render_floor.note_refused_presentation(Instant::now());
                                false
                            }
                            Err(error) => match retained_gpu_present_recovery(&error) {
                                RetainedGpuPresentRecovery::RebuildDevice => {
                                    let rebuild_schedule = surface_rebuild.note_loss();
                                    tracing::warn!(
                                        ?error,
                                        "retained GPU device requires recreation"
                                    );
                                    match rebuild_retained_gpu_device(
                                        window,
                                        pixels_slot,
                                        retained_gpu_renderer,
                                    ) {
                                        Ok(()) => {
                                            render_floor.note_refused_presentation(Instant::now());
                                            if rebuild_schedule == SurfaceRebuildSchedule::Immediate
                                            {
                                                window.request_redraw();
                                            }
                                        }
                                        Err(rebuild_error) => {
                                            tracing::error!(
                                                ?rebuild_error,
                                                "retained GPU recovery failed"
                                            );
                                            event_target.exit();
                                        }
                                    }
                                    false
                                }
                                RetainedGpuPresentRecovery::CpuFallback => {
                                    // `render_layers` validated the composition
                                    // extent against this device before returning
                                    // this source/shader limit error. The ordinary
                                    // CPU presenter below therefore has a valid
                                    // physical target and is the exact reference
                                    // fallback for this device.
                                    tracing::warn!(
                                        ?error,
                                        "retained GPU texture exceeds device limit; using CPU presentation"
                                    );
                                    true
                                }
                                RetainedGpuPresentRecovery::Fatal => {
                                    tracing::error!(?error, "retained GPU render failed");
                                    event_target.exit();
                                    false
                                }
                            },
                        };
                        if !fallback_to_cpu {
                            window.set_cursor_visible(app.platform_cursor_visible());
                            return;
                        }
                    }
                    let Some(pixels) = pixels_slot.as_mut() else {
                        return;
                    };
                    app.retained_gpu_presentation_active = false;
                    let (physical_width, physical_height) = presenter.physical_size();
                    let max_texture_dimension_2d =
                        pixels.device().limits().max_texture_dimension_2d;
                    if physical_width > max_texture_dimension_2d
                        || physical_height > max_texture_dimension_2d
                    {
                        tracing::error!(
                            physical_width,
                            physical_height,
                            max_texture_dimension_2d,
                            "CPU presentation extent exceeds the device texture limit"
                        );
                        event_target.exit();
                        return;
                    }
                    if pixels.buffer_extent().0 != physical_width
                        || pixels.buffer_extent().1 != physical_height
                    {
                        if let Err(error) = pixels.resize_buffer(physical_width, physical_height) {
                            tracing::error!(%error, "failed to restore CPU presentation buffer");
                            event_target.exit();
                            return;
                        }
                    }
                    let ordered_native_text =
                        !app.console_mode && app.can_present_ordered_native_text(presenter.scale());
                    let defer_native_main_text = !ordered_native_text
                        && app.can_defer_native_main_menu_text(presenter.scale());
                    let defer_native_loader_text =
                        !ordered_native_text && app.can_defer_native_loader_text(presenter.scale());
                    let defer_native_game_messages = !ordered_native_text
                        && app.can_defer_native_game_messages(presenter.scale());
                    let presentation_monitor_gamma = if ordered_native_text {
                        None
                    } else {
                        match app.mode {
                            AppMode::Menu | AppMode::Loading => app.startup_monitor_gamma(),
                            AppMode::Running => app.graphics.monitor_gamma_enabled().then(|| {
                                app.graphics
                                    .active_gamma_ramp(&app.snapshot.environment.gamma)
                            }),
                        }
                    };
                    let native_game_message_gamma = if defer_native_game_messages {
                        let active = app
                            .graphics
                            .active_gamma_ramp(&app.snapshot.environment.gamma);
                        Some(if app.graphics.fragment_gamma_enabled() {
                            active
                        } else {
                            clonk_graphics::GammaRamp::identity()
                        })
                    } else {
                        None
                    };
                    let refreshed = match presenter.present(pixels.frame_mut(), |frame| {
                        if ordered_native_text {
                            app.render_ordered_native_base(frame)
                        } else {
                            app.render_for_presentation_with_monitor_defer(
                                frame,
                                defer_native_main_text,
                                defer_native_loader_text,
                                defer_native_game_messages,
                                true,
                            )
                        }
                    }) {
                        Ok(refreshed) => refreshed,
                        Err(err) => {
                            tracing::error!(error = ?err, "render failed");
                            event_target.exit();
                            return;
                        }
                    };
                    if refreshed && ordered_native_text {
                        let mut composer = presenter.ordered_composer(pixels.frame_mut());
                        if let Err(err) = app.replay_pending_native_presentation(&mut composer) {
                            tracing::error!(error = ?err, "ordered native text render failed");
                            event_target.exit();
                            return;
                        }
                    } else if refreshed && defer_native_loader_text {
                        let (width, height) = presenter.physical_size();
                        if let Err(err) =
                            app.render_native_loader_text(pixels.frame_mut(), width, height)
                        {
                            tracing::error!(error = ?err, "native loader text render failed");
                            event_target.exit();
                            return;
                        }
                    } else if refreshed && defer_native_main_text {
                        let (width, height) = presenter.physical_size();
                        if let Err(err) =
                            app.render_native_main_menu_text(pixels.frame_mut(), width, height)
                        {
                            tracing::error!(error = ?err, "native main-menu text render failed");
                            event_target.exit();
                            return;
                        }
                    } else if refreshed && defer_native_game_messages {
                        let geometry = presenter.presentation_geometry();
                        let Some(gamma) = native_game_message_gamma.as_ref() else {
                            tracing::error!("deferred game-message gamma was not captured");
                            event_target.exit();
                            return;
                        };
                        if let Err(err) =
                            app.render_native_game_messages(pixels.frame_mut(), geometry, gamma)
                        {
                            tracing::error!(error = ?err, "native game-message render failed");
                            event_target.exit();
                            return;
                        }
                    }
                    if refreshed {
                        if let Some(gamma) = presentation_monitor_gamma.as_ref() {
                            gamma.apply_to_rgba_bytes(pixels.frame_mut());
                        }
                    }
                    match present_pixels_frame(pixels) {
                        Ok(RetainedGpuPresentOutcome::Presented) => {
                            surface_rebuild.note_presented();
                            while !app.pending_screenshots.is_empty() {
                                let (width, height) = presenter.physical_size();
                                let result = app.save_next_screenshot(
                                    Some(pixels.frame_mut()),
                                    width,
                                    height,
                                    presenter.scale(),
                                );
                                app.report_screenshot_result(result);
                            }
                            // Console presentation returns before render_running
                            // and leaves the prior world call list untouched.
                            if refreshed && app.mode == AppMode::Running && !app.console_mode {
                                app.finish_rendered_object_audibility_pass();
                            }
                            let graphics_duration = graphics_started.elapsed();
                            automatic_frame_skip.finish_graphics_pass(
                                app.mode == AppMode::Running && app.auto_frame_skip,
                                graphics_duration,
                                frame_schedule.simulation_interval,
                            );
                            render_floor.record_presentation(graphics_started, graphics_duration);
                            app.presentation_stats
                                .record_presentation(graphics_duration);
                            presentation_detail.record_graphics_pass(
                                app.mode == AppMode::Running && app.auto_frame_skip,
                                graphics_duration,
                                frame_schedule.simulation_interval,
                            );
                            app.presentation_detail = presentation_detail.detail();
                            if let Some(benchmark) = presentation_benchmark.as_mut() {
                                let completed_at = Instant::now();
                                benchmark.record_successful_presentation(
                                    completed_at,
                                    graphics_duration,
                                    refreshed,
                                    PresentationPath::Cpu,
                                );
                            }
                        }
                        Ok(RetainedGpuPresentOutcome::Skipped) => {
                            // Do not advance presentation-dependent governors or
                            // spin while Pixels reports an occluded surface. The
                            // normal refresh deadline will schedule the retry.
                            render_floor.note_refused_presentation(Instant::now());
                        }
                        Err(err) => {
                            let error = anyhow::Error::new(err)
                                .context("failed to submit CPU presentation frame");
                            if retained_gpu_present_recovery(&error)
                                == RetainedGpuPresentRecovery::RebuildDevice
                            {
                                let rebuild_schedule = surface_rebuild.note_loss();
                                tracing::warn!(
                                    ?error,
                                    "CPU presentation surface requires recreation"
                                );
                                match rebuild_retained_gpu_device(
                                    window,
                                    pixels_slot,
                                    retained_gpu_renderer,
                                ) {
                                    Ok(()) => {
                                        render_floor.note_refused_presentation(Instant::now());
                                        if rebuild_schedule == SurfaceRebuildSchedule::Immediate {
                                            window.request_redraw();
                                        }
                                    }
                                    Err(rebuild_error) => {
                                        tracing::error!(
                                            ?rebuild_error,
                                            "CPU presentation recovery failed"
                                        );
                                        event_target.exit();
                                    }
                                }
                            } else {
                                tracing::error!(?error, "present failed");
                                event_target.exit();
                            }
                        }
                    }
                }
                Event::LoopExiting => {
                    if app.console_mode {
                        app.finish_console_shutdown();
                    }
                    // `~C4Application` spawns the editor only after subsystem
                    // cleanup (C4Application.cpp:58-74).
                    if let Some(editor) = app.pending_editor_launch.take() {
                        if let Err(error) = std::process::Command::new(&editor).spawn() {
                            tracing::warn!(
                                %error,
                                path = %editor.display(),
                                "failed to launch the classic editor"
                            );
                        }
                    }
                    if let Some(inhibitor) = display_sleep_inhibitor.take() {
                        inhibitor.release();
                    }
                    if !app.configuration_reset_requested {
                        if let Some(paths) = app_paths.as_ref() {
                            if let Err(error) = persist_dirty_gamepad_axis_calibration(
                                paths.as_ref(),
                                &mut app.gamepad_bindings,
                            ) {
                                tracing::warn!(
                                    %error,
                                    path = %paths.config_file().display(),
                                    "failed to persist gamepad axis calibration"
                                );
                            }
                        }
                    }
                }
                _ => {}
            }
            window.set_ime_allowed(app.platform_ime_allowed());
            // SDL hides the platform pointer throughout the game client area;
            // C4MouseControl/C4GUI draw the selected themed cell themselves.
            window.set_cursor_visible(app.platform_cursor_visible());
            if let Some(paths) = app_paths.as_ref().filter(|_| {
                event_target.exiting() && !app.configuration_reset_requested && !app.console_mode
            }) {
                display_options.persist_if_dirty(paths.as_ref());
            }
            // `C4Console::StorePosition` on window destruction (C4Console.cpp:154-159)
            // writes the console's own slot and nothing else, so this is deliberately
            // separate from the game window's `persist_if_dirty` above.
            // `C4Application::Clear` writes the accumulated config once on a clean
            // quit; an aborted run discards it (C4Application.cpp:351-367).
            // Mission access deliberately does not wait for this — see
            // `persist_mission_access_if_changed`.
            if event_target.exiting() && !app.configuration_reset_requested {
                if let Some(paths) = app_paths.as_ref() {
                    for (section, entries) in app.deferred_config.take_by_section() {
                        let updates: Vec<(&str, clonk_app_netplay::NativeConfigValue<'_>)> =
                            entries
                                .iter()
                                .map(|(key, value)| {
                                    (
                                        key.as_str(),
                                        clonk_app_netplay::NativeConfigValue::RawAscii(
                                            value.as_str(),
                                        ),
                                    )
                                })
                                .collect();
                        if let Err(error) =
                            persist_native_config_values(paths.as_ref(), &section, &updates)
                        {
                            tracing::warn!(%error, section, "could not save deferred config values");
                        }
                    }
                }
            }
            let console_shutdown =
                event_target.exiting() && app.console_mode && !app.configuration_reset_requested;
            if let (true, Some(paths), Some((x, y))) = (
                console_shutdown,
                app_paths.as_ref(),
                display_options.position,
            ) {
                if let Err(error) = store_console_window_position(paths.as_ref(), x, y) {
                    tracing::warn!(%error, "could not store the console window position");
                }
            }
            // Last, because everything above still draws from or writes about
            // these windows. `run_app` consumes the event loop
            // (`winit-0.30.13/src/event_loop.rs:264`) and winit vouches for the
            // platform display only while the loop is alive (`:489`), offering
            // `OwnedDisplayHandle` for surfaces meant to outlive it — which
            // these are not. Left to the closure's own drop, every surface in
            // the process would be destroyed after the loop that owned the
            // display it was created from, which is where the console-quit
            // fault landed (clonk-org/clonk-rs#54).
            //
            // Only the windows. Releasing the whole handler here would be
            // simpler and is wrong: on macOS this event is dispatched from
            // inside `applicationWillTerminate:`
            // (`winit-0.30.13/src/platform_impl/macos/app_state.rs:166-172`),
            // an AppKit callback that never returns to `run_app`, so the rest
            // of the graph — `NetworkManager::drop`'s unbounded
            // `blocking_send` + `join`, the lobby preload worker's
            // cancellation-free join — would run nested inside the OS's own
            // quit, where a slow worker hangs termination and a panicking drop
            // unwinds across an `extern "C"` boundary and aborts.
            if loop_is_exiting {
                let destroyed = developer_windows.release_all();
                tracing::debug!(
                    windows = destroyed.len(),
                    "released the developer windows before the event loop returned"
                );
                if let Some(smoke) = headed_surface_smoke.as_mut() {
                    if let Err(error) = smoke.finish(&destroyed, developer_windows.is_empty()) {
                        tracing::error!(%error, "headed surface smoke report failed");
                        event_handler_exit_code.store(1, AtomicOrdering::Relaxed);
                    }
                }
                // After every other teardown line, so a log ending here ended
                // on purpose. Nothing marked a shutdown before, which left
                // "the log stops and the process is gone" reading identically
                // whether the player quit or the process was destroyed
                // (clonk-org/clonk-rs#40). It follows the config and console
                // persistence above deliberately: those can still `warn!`, and
                // a marker printed ahead of them would call a session clean
                // that then died saving its own config. This is the last point
                // the app controls on both platforms — on macOS `run_app`
                // never returns past here.
                clonk_logging::log_shutdown_banner(
                    app.exit_reason.unwrap_or("the event loop ended"),
                );
            }
        }))
    });
    let mut application = RuntimeApplication::new(initializer);
    event_loop
        .run_app(&mut application)
        .context("application event loop failed")?;
    application.finish()?;
    match benchmark_exit_code.load(AtomicOrdering::Relaxed) {
        0 => Ok(()),
        code => std::process::exit(code),
    }
}

impl GameApp {
    fn new(
        width: u32,
        height: u32,
        audio_options: AudioOptions,
        paths: Option<&AppPaths>,
        runtime: RuntimeConfig,
    ) -> Result<Self> {
        Self::new_with_frontend_scenarios(width, height, audio_options, paths, runtime, None)
    }

    fn new_with_frontend_scenarios(
        width: u32,
        height: u32,
        audio_options: AudioOptions,
        paths: Option<&AppPaths>,
        runtime: RuntimeConfig,
        frontend_scenarios: Option<Vec<FrontendScenario>>,
    ) -> Result<Self> {
        // Capture native Boolean grammar before participant validation or any
        // other UTF-8 convenience writer can rewrite the config projection.
        // Both values are process-local in C++ and fixed before startup UI.
        let native_config = load_native_config_bytes(paths);
        let advanced_renderer_config = load_advanced_renderer_config(&native_config);
        let high_dpi_cursor = configured_high_dpi_cursor(&native_config);
        let sky_dither = configured_sky_dither(&native_config);
        let loader_gamma = load_classic_loader_gamma_from_native(&native_config);
        let gamepads_enabled = configured_gamepads_enabled(&native_config);
        let allow_scripting_in_replays = configured_allow_scripting_in_replays(&native_config);
        let process_group_maker = configured_process_group_maker(&native_config);
        let save_description_language_table = load_runtime_language_bytes_table(paths).ok();
        let save_description_language = materialized_save_description_language(&native_config);
        // A real installation must establish C4GUI's process-global bundle
        // before any controller, discovery worker, renderer, or app-owned UI
        // state is constructed. Asset-less test apps install their explicit
        // fixture immediately after construction instead.
        let assets = Arc::new(FrontendAssets::load(paths));
        if paths.is_some() {
            assets
                .require_classic_global_gui_bootstrap_resources(&HashMap::new())
                .map_err(report_classic_parity_boundary)?;
        }
        if let Some(paths) = paths {
            if let Err(error) = validate_startup_participant_config(paths) {
                // C++ configuration strings are legacy byte buffers. Until
                // the general Config model is byte-preserving, never rewrite
                // a file merely because the UTF-8 convenience parser rejects it.
                if error.kind() != io::ErrorKind::InvalidData {
                    return Err(error).with_context(|| {
                        format!(
                            "failed to validate startup participants in {}",
                            paths.config_file().display()
                        )
                    });
                }
            }
        }
        let voice_enabled = audio_options.voice_enabled;
        let network_mode = runtime.network.clone();
        let network = match network_mode.clone() {
            Some(mode) => Some(NetworkManager::for_mode_with_voice_enabled(
                mode,
                runtime.player_owner,
                voice_enabled,
            )?),
            None => None,
        };
        let player_name = runtime.player_name.clone();
        let selected_player_file = load_selected_player_file(paths);
        let startup_player_files = match paths.map(discover_player_files).transpose() {
            Ok(players) => players.unwrap_or_default(),
            Err(error) => {
                tracing::warn!(%error, "failed to discover startup player files");
                Vec::new()
            }
        };
        let startup_player_models = startup_player_files
            .iter()
            .map(|entry| entry.render_model.clone())
            .collect();
        let network_lobby = match (&network_mode, &network) {
            (Some(mode), Some(manager)) => Some(
                NetworkLobbyState::new(
                    manager.local_client_id(),
                    player_name.clone(),
                    matches!(mode, NetworkMode::Host(_)),
                )
                .with_preloading(
                    load_options_program_state(paths, None).preloading,
                    LobbyLabels::default(),
                ),
            ),
            _ => None,
        };
        let control_clients = initial_control_clients(network.as_ref(), network_mode.as_ref());
        let network_control_running = network.is_none();
        let network_control_clock = initial_network_control_clock(network_mode.as_ref());
        let network_client_next_control_ticks =
            network_control_clock.map_or_else(HashMap::new, |clock| {
                control_clients
                    .activated_client_ids()
                    .into_iter()
                    .map(|client_id| (client_id, clock.current_tick()))
                    .collect()
            });
        let host_join_snapshot = initial_host_join_snapshot(network_mode.as_ref());
        let network_max_players = initial_network_max_players(network_mode.as_ref());
        let network_is_league = initial_network_is_league(network_mode.as_ref());
        let network_league_name = initial_network_league_name(network_mode.as_ref());
        let network_stream_address = initial_network_stream_address(network_mode.as_ref());
        let host_local_alternate_colors_by_resource =
            initial_host_local_alternate_colors(network_mode.as_ref());
        let host_local_player_info_ids = initial_host_local_player_info_ids(network_mode.as_ref());
        let control_player_infos = ControlPlayerInfoRegistry::default();
        // Scenario discovery only walks directories and reads scenario
        // groups; start it only after the process-global resource gate.
        let scenario_discovery = frontend_scenarios.is_none().then(|| {
            let paths = paths.cloned();
            std::thread::spawn(move || match paths {
                Some(paths) => load_frontend_scenarios_from_paths(&paths),
                None => load_frontend_scenarios(),
            })
        });
        let (loader_screen, loader_error) = match paths {
            Some(paths) => match build_startup_loader(paths, assets.as_ref()) {
                Ok(setup) => (Some(setup.screen), None),
                Err(error) => {
                    tracing::error!(%error, "classic startup loader initialization failed");
                    (None, Some(error.to_string()))
                }
            },
            None => (
                None,
                Some("application paths are unavailable for classic loader discovery".to_string()),
            ),
        };
        let (system_scripts, standard_names) = paths
            .and_then(|paths| {
                let group = Group::open(paths.system_group_path()).ok()?;
                let scripts = match load_classic_global_system_scripts(paths, &group) {
                    Ok(scripts) => scripts,
                    Err(error) => {
                        tracing::warn!(%error, "failed to localize System.c4g scripts");
                        Vec::new()
                    }
                };
                let names = group
                    .read_file("Names.txt")
                    .ok()
                    .map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
                Some((scripts, names))
            })
            .unwrap_or_default();
        if system_scripts.is_empty() {
            tracing::warn!("no System.c4g scripts found; global script functions unavailable");
        }

        // Pathless test/sandbox apps have no install material library. Keep
        // their loader boundary state intact without spawning a worker whose
        // only possible result is immediately `None`.
        let boot_loading = paths.map(|paths| {
            let (tx, rx) = std::sync::mpsc::channel();
            let paths = paths.clone();
            std::thread::spawn(move || {
                let material_library = load_install_material_library(Some(&paths));
                let _ = tx.send(BootLoadingEvent::Finished(material_library));
            });
            BootLoadingState::new(rx)
        });

        let base_sprites = assets.base_sprite_map().clone();
        let sprite_cache = Arc::new(base_sprites.clone());

        // Engine starts with default materials; will be updated when boot loading finishes
        let graphics_smoke_level = load_graphics_smoke_level(paths);
        let display_flags = load_display_flags(paths);
        let mission_access = paths
            .and_then(|paths| match load_configured_mission_access(paths) {
                Ok(access) => Some(MissionAccessStore::new(access)),
                Err(error) => {
                    tracing::warn!(%error, "failed to load General.MissionAccess; using empty list");
                    None
                }
            })
            .unwrap_or_default();
        let show_folder_maps = load_show_folder_maps(paths);
        let bindings = KeyboardBindings::load(paths);
        let gamepad_bindings = GamepadBindings::load(paths);
        let gamepads = if cfg!(test) || !gamepads_enabled {
            // Global disable mirrors native's null C4GamePadControl. Synthetic
            // app tests also inject normalized events directly; starting the
            // macOS gilrs backend for every nextest process would otherwise leave
            // hundreds of unused threads contending during the workspace gate.
            GamepadManager::disabled()
        } else {
            GamepadManager::new(gamepad_bindings.axis_calibrations())
        };
        let show_commands_requests = ShowCommandsRequestStore::default();
        let mut engine = Engine::new();
        engine.set_mission_access_store(mission_access.clone());
        engine.set_show_commands_request_store(show_commands_requests.clone());
        engine.set_control_key_names(configured_control_key_names(&bindings));
        engine.set_smoke_level(graphics_smoke_level);
        engine.set_fire_particles(display_flags.fire_particles);
        engine.set_control_host(!matches!(
            network_mode.as_ref(),
            Some(NetworkMode::Client(_))
        ));
        engine.set_local_players([runtime.player_owner]);
        engine.set_max_players(i32::try_from(network_max_players).unwrap_or(i32::MAX));
        seed_engine_player_info_parameters(
            &mut engine,
            &network_league_name,
            &control_player_infos,
        );
        if let Some(snapshot) = host_join_snapshot.as_ref() {
            engine.set_use_fair_crew(snapshot.parameters.use_fair_crew);
            engine.set_fair_crew_strength(snapshot.parameters.fair_crew_strength);
            engine.set_fair_crew_forced(snapshot.parameters.fair_crew_forced);
            engine.set_allow_debug(snapshot.parameters.allow_debug);
            engine.set_team_distribution(i32::from(snapshot.parameters.teams.team_distribution));
            engine.set_team_colors(snapshot.parameters.teams.team_colors != 0);
        }
        let snapshot = engine.snapshot();

        let scenarios = match frontend_scenarios {
            Some(scenarios) => scenarios,
            None => scenario_discovery
                .expect("scenario discovery starts without preloaded scenarios")
                .join()
                .map_err(|_| anyhow!("scenario discovery thread panicked"))?,
        };
        let button_textures = assets.button_textures();
        let menu_entries = build_menu_entries(&scenarios, false);
        let mut menu = StartupMenu::new(menu_entries, assets.font_arc(), button_textures.clone())
            .map_err(|err| anyhow!("failed to create startup menu: {err}"))?;
        menu.resize(width as f32, height as f32);
        let mut main_menu = StartupMainMenu::new(assets.font_arc(), button_textures.clone());
        main_menu.set_highlight_texture(assets.button_highlight.clone());
        main_menu.set_clonk_fonts(assets.clonk_fonts.clone());
        main_menu.set_gamma_ramp(
            (!advanced_renderer_config.disable_gamma
                && advanced_renderer_config.shader
                && advanced_renderer_config.use_shader_gamma)
                .then(|| {
                    Arc::new(
                        loader_gamma
                            .clone()
                            .unwrap_or_else(clonk_graphics::GammaRamp::standard),
                    )
                }),
        );
        main_menu.resize(width as f32, height as f32);
        let participants_label = load_participants_label(paths);
        let main_menu_state = MainMenuState::new(main_menu, participants_label);

        let scenario_catalog = build_scenario_catalog(&scenarios);
        let menu_state = MenuState::new(menu, scenarios);
        let scenario_game_options = GameOptionButtons::new(
            GameOptionContext::LocalSelector,
            load_scenario_game_option_values(paths),
        );
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
        graphics.set_advanced_renderer_config(advanced_renderer_config);
        graphics.set_cursor_tiers(if high_dpi_cursor {
            CursorTiers::HighDpi
        } else {
            CursorTiers::Classic
        });
        graphics.set_sky_dither(sky_dither);
        graphics.set_fine_fog_of_war(configured_fine_fog_of_war(&native_config));
        graphics.set_hd_exact_blits(configured_hd_exact_blits(&native_config));
        graphics.set_shader_landscape(configured_shader_landscape(&native_config));
        graphics.set_clonk_fonts(assets.clonk_fonts.clone());
        graphics.set_game_palette(assets.game_palette());
        graphics.set_liquid_animation(assets.liquid_animation());
        graphics.surface_mut().fill(Color::opaque(16, 28, 52));
        let control_messages = ControlMessageState::new(
            load_sound_command_cooldown(paths),
            audio_options.mute_sound_command,
        );
        let audio = match AudioContext::try_new_with_paths(audio_options, paths) {
            Ok(ctx) => Some(ctx),
            Err(err) => {
                tracing::warn!(error = %err, "audio initialisation failed");
                None
            }
        };
        let runtime_language_table = load_runtime_language_table(paths);
        let generated_team_name_template = runtime_language_table
            .as_ref()
            .map(generated_team_name_template)
            .unwrap_or_else(|_| {
                LegacyCString::from_bytes(b"Team %d".to_vec())
                    .expect("the shipped team-name resource contains no NUL")
            });
        let network_team_assignment =
            initial_network_team_assignment(network_mode.as_ref(), &generated_team_name_template);
        let (needed_material_need, needed_material_none) = runtime_language_table
            .as_ref()
            .map(needed_material_resource_strings)
            .unwrap_or_else(|_| {
                (
                    "%s|needs".to_owned(),
                    "%s needs|no more material.".to_owned(),
                )
            });
        let object_no_dig = runtime_language_table
            .as_ref()
            .map(object_no_dig_resource_string)
            .unwrap_or_else(|_| "%s cannot dig.".to_owned());
        let definition_overload_template = runtime_language_table
            .as_ref()
            .map(definition_overload_resource_string)
            .unwrap_or_else(|_| {
                clonk_engine::scenario::verbose_loading::DEFAULT_DEFINITION_OVERLOAD_TEMPLATE
                    .to_owned()
            });
        let construction_check_feedback = runtime_language_table
            .as_ref()
            .map(construction_check_resource_strings)
            .unwrap_or_else(|_| default_construction_check_feedback());
        let default_rank_names = runtime_language_table
            .as_ref()
            .ok()
            .map(default_rank_resource_names);
        let loaded_default_rank_names = default_rank_names.clone();
        let (startup_tooltip_resources, runtime_language_charset) =
            match runtime_language_table.as_ref() {
                Ok(table) => (table.entries.clone(), table.charset),
                Err(_) => {
                    let table = embedded_runtime_language_table();
                    (table.entries.clone(), table.charset)
                }
            };
        engine.set_needed_material_resource_strings(
            needed_material_need.clone(),
            needed_material_none.clone(),
        );
        clonk_engine::scenario::verbose_loading::set_definition_overload_template(
            definition_overload_template,
        );
        engine.set_object_no_dig_resource_string(object_no_dig.clone());
        {
            let [undefined, no_construction, no_room, no_level, no_other] =
                construction_check_feedback.clone();
            engine.set_construction_check_resource_strings(
                undefined,
                no_construction,
                no_room,
                no_level,
                no_other,
            );
        }
        if let Some(names) = default_rank_names.as_ref() {
            engine.set_default_rank_names(names.clone());
        }
        // Left empty so the lazy accessor can build it once the live
        // KeyConfig is loadable; a language failure still surfaces there.
        let runtime_help_text_cache = OnceLock::new();
        if let Err(error) = &runtime_language_table {
            let _ = runtime_help_text_cache.set(Err(format!("{error:#}")));
        }
        let runtime_flash_resources_cache = OnceLock::new();
        let _ = runtime_flash_resources_cache.set(match &runtime_language_table {
            Ok(table) => Ok(build_runtime_flash_resources(table)),
            Err(error) => Err(format!("{error:#}")),
        });

        let mut app = Self {
            engine,
            graphics,
            taskbar_progress: clonk_platform::taskbar_progress::LoaderTaskbarProgress::new(
                Box::new(clonk_platform::taskbar_progress::NoTaskbarProgress),
            ),
            deferred_config: crate::deferred_config::DeferredConfig::default(),
            sky: None,
            material_texture_images: Arc::new(HashMap::new()),
            material_render_info: Arc::new(HashMap::new()),
            system_scripts,
            standard_names,
            needed_material_need,
            needed_material_none,
            object_no_dig,
            construction_check_feedback,
            default_rank_names,
            loaded_default_rank_names,
            startup_tooltip_resources,
            runtime_language_charset,
            persisted_mission_access: mission_access.snapshot(),
            mission_access,
            process_group_maker,
            save_description_language_table,
            save_description_language,
            show_folder_maps,
            show_commands_requests,
            allow_scripting_in_replays,
            input: InputDispatcher::new(),
            bindings,
            gamepad_bindings,
            local_controls: LocalControlRegistry::default(),
            pressed_engine_keys: HashSet::new(),
            engine_key_repeated: false,
            key_event_suppresses_text: false,
            scoreboard_tab_raw_pressed: false,
            keyboard_modifiers: ModifiersState::empty(),
            pending_screenshots: VecDeque::new(),
            retained_gpu_presentation_active: false,
            retained_gpu_ordered_capture_active: false,
            retained_native_capture_surface: None,
            pending_gpu_thumbnail_paths: VecDeque::new(),
            pending_native_save_thumbnails: VecDeque::new(),
            pending_options_display_requests: VecDeque::new(),
            gamepads_enabled,
            gamepad_input_enabled: gamepads_enabled,
            gamepads,
            #[cfg(test)]
            gamepad_poll_count: 0,
            gamepad_gui_control: load_gamepad_gui_control(paths),
            snapshot,
            focus_id: None,
            focus_snapshot: None,
            frame_text: String::new(),
            status_text: String::new(),
            startup_restart_diagnostics: StartupRestartDiagnostics::default(),
            energy_fraction: 0.0,
            scenario_label,
            fallback_ground: DEFAULT_GROUND_HEIGHT,
            menu_state,
            main_menu_state,
            startup_tooltip: ClassicTooltipTracker::new(),
            startup_network_dialog: None,
            startup_irc_client: None,
            pending_editor_launch: None,
            startup_irc_server: String::new(),
            external_irc_dialog_visible: false,
            external_irc_dialog: None,
            startup_irc_initial_connect_pending: false,
            irc_dialog_last_click: None,
            external_irc_pointer_capture: false,
            startup_game_search: None,
            #[cfg(test)]
            startup_game_search_test_events: VecDeque::new(),
            startup_network_last_refresh: None,
            startup_masterserver_next_query_at: None,
            startup_masterserver_request_timeout_at: None,
            startup_network_refresh_waiting_for_clear: false,
            startup_network_ignore_redirect: false,
            startup_game_references: Vec::new(),
            startup_discovery_reference_queries: Vec::new(),
            startup_direct_reference_queries: Vec::new(),
            next_startup_direct_reference_query_id: 0,
            network_game_advertiser: None,
            advertised_game_reference: None,
            startup_player_dialog: None,
            startup_player_properties_dialog: None,
            startup_user_portraits_written: false,
            startup_last_portrait_folder_index: None,
            startup_player_files,
            startup_player_models,
            startup_crew_files: Vec::new(),
            startup_crew_models: Vec::new(),
            startup_crew_player_index: None,
            startup_crew_rename: None,
            startup_options_dialog: None,
            startup_options_advanced_dialog: None,
            startup_about_dialog: None,
            startup_view: StartupView::MainMenu,
            startup_dialog_fade: None,
            last_startup_dialog: StartupDialog::MainMenu,
            startup_scenario_back_dialog: None,
            startup_view_flags: StartupViewFlags {
                fair_crew: load_fair_crew_flag(paths),
                record: load_recording_flag(paths),
            },
            scenario_selector_mode: ScenarioSelectorMode::Local,
            scenario_game_options,
            object_menu: None,
            ingame_menu: PlayerIngameMenus::default(),
            ingame_menu_gfx: None,
            runtime_player_big_icons: HashMap::new(),
            runtime_player_big_icon_misses: HashSet::new(),
            script_menu_presentations: BTreeMap::new(),
            menu_viewport_rects: BTreeMap::new(),
            display_flags,
            white_lobby_chat: load_white_lobby_chat(paths),
            show_log_timestamps: load_show_log_timestamps(paths),
            graphics_smoke_level,
            mouse_control: true,
            mouse_control_allowed: true,
            mode: AppMode::Loading,
            scenario_catalog,
            scenario_selector_discovery: None,
            scenario_entry_enabled: HashMap::new(),
            active_scenario: None,
            active_definition_load: None,
            active_description_definition_modules: Vec::new(),
            active_game_graphics: None,
            audio,
            voice_chat: crate::voice_chat::VoiceChatState::default(),
            #[cfg(test)]
            ui_sound_log: Vec::new(),
            #[cfg(test)]
            league_surrender_pre_abort_results: None,
            runtime_music_enabled: false,
            resume_frontend_music_after_fade: false,
            frontend_music_attempted_for_entry: false,
            assets: assets.clone(),
            active_global_gui_failures: HashMap::new(),
            native_startup_fonts: None,
            pending_native_presentation: None,
            loader_screen,
            loader_error,
            loader_render_config: Some(LoaderRenderConfig::scale_one(
                DisplayOptions::load(paths).point_filtering,
            )),
            loader_render_error: None,
            loader_gamma,
            app_paths: paths.cloned(),
            classic_command_line: ClassicCommandLine::default(),
            classic_record_stream_activation_pending: false,
            initial_definition_seed: Some(classic_command_line_definition_modules(
                &load_native_config_bytes(paths),
                &[],
            )),
            console_mode: false,
            headless: false,
            developer_console: DeveloperConsole::new(),
            developer_console_edit_mode: ConsoleEditMode::Play,
            developer_selection: Default::default(),
            developer_tools: Default::default(),
            console_viewport_projections: Default::default(),
            edit_cursor_drop_target: None,
            edit_cursor_tick_frame: None,
            file_monitor: None,
            edit_cursor_hold: false,
            edit_cursor_last_world: None,
            edit_cursor_drag_frame: None,
            console_viewport_context_menu: None,
            console_viewport_context_menu_grab: None,
            developer_toolbox: Default::default(),
            developer_toolbox_effects: Vec::new(),
            developer_object_list_open: false,
            developer_component_editor: None,
            developer_component_hosts: Vec::new(),
            developer_console_editing_enabled: true,
            developer_console_pointer: GuiPoint::new(0.0, 0.0),
            console_log_capture: None,
            game_log_capture: None,
            script_created_objects: false,
            sandbox_crew_definition_paths: None,
            configured_client_player_selection: None,
            material_library: None,
            pending_lobby_internet_signup: None,
            pending_league_player_auth: None,
            network_event_waker: None,
            network,
            network_mode,
            league_auth_session: None,
            network_lobby,
            classic_host_lobby: None,
            lobby_preload_task: None,
            lobby_preload_artifact: None,
            network_start_wait: None,
            host_lobby_countdown: None,
            network_lobby_min_players: None,
            pending_local_lobby_countdown_echoes: VecDeque::new(),
            lobby_ready_check_cooldown: load_lobby_ready_check_cooldown(paths),
            ready_check_toasts_enabled: load_ready_check_toasts_enabled(paths),
            pending_desktop_notifications: VecDeque::new(),
            control_messages,
            league_votes: LeagueVoteState::default(),
            startup_network_connection: None,
            classic_direct_reference_query: None,
            pending_network_join: None,
            staged_network_host_scenario: None,
            sync_checks: SyncCheckState::new(),
            network_ticks: NetworkTickGate::default(),
            waiting_network_control: None,
            network_stall_since: None,
            frames_since_redraw: 0,
            network_control_retry_pending: false,
            network_sync: NetworkSyncGate::default(),
            offline_control_input: Vec::new(),
            offline_halt_count: 0,
            network_control_running,
            runtime_network_status_barrier: None,
            host_reference_paused: false,
            runtime_network_control_mode: None,
            runtime_network_committed_control_mode: None,
            runtime_network_committed_status: None,
            runtime_network_join_allowed: None,
            network_rejoin_after_elimination_allowed: None,
            network_control_clock,
            network_max_players,
            network_is_league,
            network_league_name,
            network_stream_address,
            frames_per_second: 0,
            frames_since_second: 0,
            presentation_stats: PresentationStats::default(),
            input_latency_benchmark: input_latency_benchmark_from_env(),
            full_speed: false,
            frame_skip: 1,
            auto_frame_skip: configured_auto_frame_skip(&native_config),
            presentation_detail: PresentationDetail::default(),
            max_refresh_delay_ms: configured_max_refresh_delay_ms(&native_config),
            startup_refresh_delay_ms: configured_max_refresh_delay_ms(&native_config),
            display_refresh_period_ms: None,
            network_stats: None,
            network_stats_clients: HashSet::new(),
            network_stats_players: HashSet::new(),
            control_clients,
            network_client_next_control_ticks,
            network_client_activity: NetworkClientActivity::default(),
            control_player_infos,
            local_player_profile_paths: HashMap::new(),
            restart_restore_infos: RestartRestoreInfos::default(),
            abort_restart_pending: false,
            restart_restore_roster_items: HashSet::new(),
            pending_host_rejoin: None,
            host_local_alternate_colors_by_resource,
            host_local_player_info_ids,
            deferred_network_savegame_recreation: Vec::new(),
            network_savegame_recreation_progress: None,
            generated_team_name_template,
            network_team_assignment,
            admission_resources: AdmissionResourceStore::default(),
            blocking_resource_wait: None,
            aborted_player_resource_joins: HashSet::new(),
            host_join_snapshot,
            pending_runtime_dynamic_request: None,
            pending_network_join_data: None,
            initial_lobby_status_ack_pending: false,
            client_start_barrier: ClientStartBarrier::default(),
            pending_client_start_status: None,
            client_combined_scenario_path: None,
            client_combined_preload_file: ClientCombinedPreloadFile::default(),
            network_material_resource_groups: None,
            executing_ready_tick: None,
            recording_enabled: runtime.record_enabled && paths.is_some(),
            recordings_dir: paths.map(AppPaths::recordings_dir),
            live_save_seed: None,
            recording_template: None,
            recording: None,
            runtime_record_requested: false,
            control_playback: None,
            local_owner: runtime.player_owner,
            player_name: player_name.clone(),
            selected_player_file,
            last_save_path: None,
            object_sprites: base_sprites,
            sprite_cache: Arc::clone(&sprite_cache),
            loading_state: None,
            boot_loading,
            auto_start_sandbox: false,
            auto_start_classic_command_line_scenario: false,
            incoming_update: None,
            update_check_requested: false,
            update_check: None,
            update_download: None,
            automatic_update_check_allowed: !cfg!(test),
            ingame_gui_pointer: None,
            ingame_pointer: None,
            ingame_mouse_help: false,
            ingame_mouse_init_centered: false,
            ingame_viewport_mouse: None,
            ingame_edge_scroll: None,
            free_view_scroll_momentum: FreeViewScrollMomentum::default(),
            ingame_mouse_caption: IngameMouseCaptionState::default(),
            window_mouse_position: None,
            pointer_inside_window: false,
            running_gui_mouse_owned: false,
            // C4MouseControl::Default starts with fMouseOwned set even while
            // the control itself is inactive outside a running game.
            running_world_mouse_owned: true,
            running_pointer_position: None,
            primary_pointer_left_down: false,
            last_application_left_press: None,
            ingame_menu_close_pointer_capture: None,
            script_menu_close_pointer_capture: None,
            menu_title_drag: None,
            ingame_mouse_help_caption: None,
            mouse_state: None,
            ingame_right_mouse_state: None,
            construction_menu_drag: None,
            ingame_dragged_objects: Vec::new(),
            ingame_last_left_down: None,
            ingame_ignore_left_up: false,
            window_active: true,
            window_occluded: false,
            exit_requested: false,
            exit_reason: None,
            configuration_reset_requested: false,
            game_over_dialog: None,
            game_over_handled: false,
            pending_league_end: None,
            runtime_help_visible: false,
            runtime_help_text_cache,
            runtime_key_config_cache: OnceLock::new(),
            runtime_flash_resources_cache,
            runtime_flash_message: None,
            film_view_player: None,
            physical_viewports: Vec::new(),
            next_physical_viewport_identity: 1,
            physical_viewports_authoritative: false,
            runtime_client_list: None,
            runtime_client_list_consumed_keys: HashSet::new(),
            runtime_client_list_above_game_over: false,
            runtime_default_dialog_order: Vec::new(),
            scoreboard_dialog: None,
            scoreboard_initial_reconcile_pending: false,
            scoreboard_close_pointer_capture: false,
            scoreboard_runtime: ScoreboardDialogRuntime::default(),
            running_dialog_stack: Vec::new(),
            running_active_dialog: None,
            next_running_message_stack_id: 1,
            message_dialogs: Vec::new(),
            league_signup_dialog: None,
            cancelled_league_signup_continuation: None,
            message_dialog_active_index: None,
            message_dialog_pointer_capture_index: None,
            definition_selector: None,
            pending_definition_selection: None,
            pending_lobby_player_selection: None,
            game_option_input_dialog: None,
            context_menu: None,
            context_menu_lobby_team_player: None,
            context_menu_lobby_option: None,
            context_menu_lobby_kick_client: None,
            context_menu_lobby_player: None,
            context_menu_pointer_dismissed_lobby_team_player: None,
            context_menu_pointer_dismissed_lobby_option: None,
            context_menu_pointer_capture: None,
            message_dialog_consumed_keys: HashSet::new(),
            league_signup_consumed_keys: HashSet::new(),
            league_signup_pointer_capture: false,
            league_signup_pointer_position: None,
            definition_selector_consumed_keys: HashSet::new(),
            netdlg_edit_consumed_keys: HashSet::new(),
            definition_selector_pointer_capture: false,
            game_option_input_consumed_keys: HashSet::new(),
            game_option_input_pointer_capture: None,
            game_option_input_pointer_position: None,
            game_option_input_last_click: None,
            game_option_consumed_keys: HashSet::new(),
            game_option_pointer_capture: false,
            menu_backdrop_cache: StartupBackdropCache::default(),
            scensel_last_click: None,
            scensel_rename_pointer_focus: None,
            scensel_search_last_click: None,
            definition_selector_last_click: None,
            plrsel_last_click: None,
            netdlg_last_click: None,
            netdlg_join_edit_last_click: None,
            message_board: ClassicMessageBoardState::default(),
            network_chart_dialog: None,
            network_chart_consumed_keys: HashSet::new(),
            network_chart_pointer_capture: false,
            network_chart_elevated: false,
            running_chat: None,
            chat_paste_consumed_keys: HashSet::new(),
            lobby_chat_drag_anchor: None,
            message_input_history: VecDeque::new(),
            show_startup_hint: false,
            debug_hud: std::env::var("LC_APP_HUD_DEBUG")
                .map(|v| v == "1")
                .unwrap_or(false),
        };
        if let Some(existing) = existing_quick_save_path() {
            app.last_save_path = Some(existing);
        }
        app.sync_scenario_game_option_bounds();
        if matches!(app.network_mode.as_ref(), Some(NetworkMode::Client(_))) {
            app.freeze_configured_client_players_for_game()
                .context("failed to snapshot configured client players")?;
        }
        // Don't show menu yet; we're in Loading mode for boot loading
        // show_main_menu() and ensure_menu_music() will be called when boot loading finishes
        Ok(app)
    }

    fn apply_classic_command_line(&mut self, classic: &ClassicCommandLine) -> Result<()> {
        self.classic_command_line = classic.clone();
        self.classic_record_stream_activation_pending = false;
        self.initial_definition_seed = Some(classic_command_line_definition_modules(
            &load_native_config_bytes(self.app_paths.as_ref()),
            &classic.definition_files,
        ));
        if !classic.player_files.is_empty() {
            self.configured_client_player_selection = self
                .app_paths
                .as_ref()
                .map(|paths| snapshot_effective_client_player_selection(paths, classic))
                .transpose()?;
        }
        self.apply_classic_game_option_overrides();
        self.incoming_update = classic.incoming_update.clone();
        self.update_check_requested = classic.update_requested;

        if let Some(screen) = classic.startup_screen.as_deref() {
            self.apply_classic_startup_screen(screen);
        }
        Ok(())
    }

    fn classic_command_line_definition_load(&self) -> ScenarioDefinitionLoad {
        let modules = self.initial_definition_seed.clone().unwrap_or_default();
        let definition_root =
            self.app_paths
                .as_ref()
                .and_then(|paths| match startup_definition_paths(paths) {
                    Ok(paths) => paths.active_custom_root,
                    Err(error) => {
                        tracing::error!(
                            %error,
                            "failed to read General.DefinitionPath for command-line scenario"
                        );
                        None
                    }
                });
        ScenarioDefinitionLoad::Seed {
            modules,
            definition_root,
        }
    }

    fn native_font_cache_for_source(
        &self,
        source: Option<&ClassicNativeFontSource>,
    ) -> Option<Arc<clonk_frontend::clonk_fonts::NativeClonkFontSet>> {
        let scale = self.loader_render_config?.application_scale();
        if scale <= 0.0 || !scale.is_finite() {
            return None;
        }
        let source = source?;
        match clonk_frontend::clonk_fonts::build_native_font_set_face(
            &source.bytes,
            source.face_index,
            scale,
        ) {
            Ok(fonts) => Some(Arc::new(fonts)),
            Err(error) => {
                tracing::warn!(%error, scale, "failed to build scale-native active fonts");
                None
            }
        }
    }

    fn can_defer_native_game_messages(&self, scale: f32) -> bool {
        self.mode == AppMode::Running
            && scale > 0.0
            && scale.is_finite()
            // The physical commit point is after the filtered logical frame.
            // Keep C4Viewport/C4GUI z-order authoritative whenever a layer
            // that C++ draws after game messages is visible.
            && self.ingame_selection_frame().is_none()
            && !self.runtime_help_visible
            && !self.runtime_halt_active()
            && self
                .runtime_flash_message
                .as_ref()
                .is_none_or(|message| message.remaining_draws == 0)
            && self.runtime_client_list.is_none()
            && self.network_chart_dialog.is_none()
            && self.scoreboard_dialog.is_none()
            && self.game_over_dialog.is_none()
            && self.game_option_input_dialog.is_none()
            && self.message_dialogs.is_empty()
            && self.context_menu.is_none()
            && self
                .native_startup_fonts
                .as_ref()
                .is_some_and(|fonts| fonts.scale() == scale)
    }

    fn can_present_ordered_native_text(&self, scale: f32) -> bool {
        let ordered_loading_overlay = self.mode == AppMode::Loading
            && (!self.message_dialogs.is_empty()
                || self
                    .network_start_wait
                    .as_ref()
                    .is_some_and(|wait| wait.visible))
            && self
                .loader_render_config
                .is_some_and(|config| config.application_scale() == scale);
        (matches!(self.mode, AppMode::Menu | AppMode::Running) || ordered_loading_overlay)
            && scale > 0.0
            && scale.is_finite()
            && self
                .native_startup_fonts
                .as_ref()
                .is_some_and(|fonts| (fonts.scale() - scale).abs() < f32::EPSILON)
    }

    fn begin_native_text_capture(&mut self, clear_to_transparent: bool) {
        let surface = self.graphics.surface_mut();
        surface.clear_clip();
        if clear_to_transparent && !self.retained_gpu_ordered_capture_active {
            surface.fill(Color::transparent());
        }
        surface.begin_clonk_text_capture();
        if self.retained_gpu_ordered_capture_active {
            debug_assert!(!surface.is_gpu_scene_capture_active());
            surface.begin_gpu_scene_capture();
        }
    }

    fn finish_native_base_batch(&mut self, frame: &mut [u8], plan: &mut NativePresentationPlan) {
        let surface = self.graphics.surface_mut();
        let text = surface.take_clonk_text_capture();
        let gpu_recorder = surface.take_gpu_scene_capture();
        if gpu_recorder.is_none() {
            if surface.pixels().len() == frame.len() {
                frame.copy_from_slice(surface.pixels());
            } else {
                copy_surface(surface.pixels(), surface.width(), surface.height(), frame);
            }
        }
        plan.batches.push(NativePresentationBatch {
            logical_layer: None,
            clip: None,
            native_loader_text: false,
            text,
            fonts: None,
            gpu_recorder,
        });
    }

    fn finish_native_overlay_batch(&mut self, plan: &mut NativePresentationPlan) {
        Self::capture_native_overlay_batch(self.graphics.surface_mut(), plan, None);
    }

    fn capture_native_overlay_batch(
        surface: &mut Surface,
        plan: &mut NativePresentationPlan,
        isolated_clip: Option<Rect>,
    ) {
        let text = surface.take_clonk_text_capture();
        let gpu_recorder = surface.take_gpu_scene_capture();
        // Additive passes can contribute RGB while preserving a transparent
        // destination alpha. Retain those bytes for ordered replay too.
        let has_raster = gpu_recorder.is_none() && surface.pixels().iter().any(|byte| *byte != 0);
        let has_gpu_commands = gpu_recorder
            .as_ref()
            .is_some_and(|recorder| !recorder.is_empty());
        if has_raster || has_gpu_commands || !text.is_empty() {
            let clip = isolated_clip.filter(|clip| {
                has_raster
                    && !text.is_empty()
                    && text.iter().all(|command| command.clip == Some(*clip))
            });
            plan.batches.push(NativePresentationBatch {
                logical_layer: has_raster.then(|| surface.pixels().to_vec()),
                clip,
                native_loader_text: false,
                text,
                fonts: None,
                gpu_recorder,
            });
        }
    }

    fn commit_pending_native_base(&mut self, frame: &mut [u8]) {
        let mut plan = self.pending_native_presentation.take().unwrap_or_default();
        self.finish_native_base_batch(frame, &mut plan);
        self.pending_native_presentation = Some(plan);
    }

    fn commit_pending_native_overlay(&mut self) {
        let mut plan = self.pending_native_presentation.take().unwrap_or_default();
        self.finish_native_overlay_batch(&mut plan);
        self.pending_native_presentation = Some(plan);
    }

    fn next_native_overlay_parts(
        graphics: &mut GraphicsSystem,
        pending_native_presentation: &mut Option<NativePresentationPlan>,
        retained_gpu_capture: bool,
    ) {
        Self::next_native_overlay_parts_with_clip(
            graphics,
            pending_native_presentation,
            None,
            retained_gpu_capture,
        );
    }

    fn next_native_overlay_parts_with_clip(
        graphics: &mut GraphicsSystem,
        pending_native_presentation: &mut Option<NativePresentationPlan>,
        isolated_clip: Option<Rect>,
        retained_gpu_capture: bool,
    ) {
        let mut plan = pending_native_presentation.take().unwrap_or_default();
        {
            let surface = graphics.surface_mut();
            Self::capture_native_overlay_batch(surface, &mut plan, isolated_clip);
        }
        *pending_native_presentation = Some(plan);
        let surface = graphics.surface_mut();
        surface.clear_clip();
        if !retained_gpu_capture {
            surface.fill(Color::transparent());
        }
        surface.begin_clonk_text_capture();
        if retained_gpu_capture {
            debug_assert!(!surface.is_gpu_scene_capture_active());
            surface.begin_gpu_scene_capture();
        }
    }

    fn next_pending_native_overlay(&mut self) {
        Self::next_native_overlay_parts(
            &mut self.graphics,
            &mut self.pending_native_presentation,
            self.retained_gpu_ordered_capture_active,
        );
    }

    fn next_pending_native_overlay_with_clip(&mut self, isolated_clip: Rect) {
        Self::next_native_overlay_parts_with_clip(
            &mut self.graphics,
            &mut self.pending_native_presentation,
            Some(isolated_clip),
            self.retained_gpu_ordered_capture_active,
        );
    }

    fn current_game_palette(&self) -> Arc<GamePalette> {
        self.active_game_graphics
            .as_ref()
            .map(|resources| Arc::clone(&resources.palette))
            .unwrap_or_else(|| self.assets.game_palette())
    }

    fn current_liquid_animation(&self) -> Option<ImageData> {
        self.active_game_graphics
            .as_ref()
            .and_then(|resources| resources.liquid_animation.as_deref().cloned())
            .or_else(|| self.assets.liquid_animation())
    }

    fn script_text_spec_resources(&self) -> ScriptTextSpecResources<'_> {
        script_text_spec_resources_from_assets_and_hud(
            self.assets.as_ref(),
            self.current_hud_graphics_ref(),
        )
    }

    fn script_menu_item_icons(
        &self,
        menu: &clonk_engine::ObjectMenuState,
    ) -> Vec<Option<ImageData>> {
        let item_definition_color = if !menu.user_menu
            && matches!(
                menu.title_symbol,
                clonk_engine::ObjectMenuSymbol::Buy { .. }
            ) {
            object_menu_buying_player_color(&self.snapshot, menu.command_object)
        } else {
            0
        };
        let text_spec_resources = self.script_text_spec_resources();
        let hud_graphics = self.current_hud_graphics();
        let allowed_blit_modes = self.graphics.advanced_renderer_config().allowed_blit_modes;
        // A Context ObjectRank facet is sized by the menu's resolved
        // ItemHeight, which only the layout knows (C4Script.cpp:1721).
        let context_item_height = (menu.style == 1).then(|| {
            // Without the classic set the row falls back to the
            // C4MN_SymbolSize floor, which is what the layout uses too.
            let line_height = self
                .assets
                .clonk_fonts
                .as_deref()
                .map_or(0, |fonts| fonts.text.line_height);
            clonk_app_menus::object_menu::classic_context_item_height(line_height)
        });
        menu.items
            .iter()
            .map(|item| {
                clonk_app_core::pictures::object_menu_item_picture_with_context_height(
                    &self.engine,
                    &self.snapshot,
                    item,
                    item_definition_color,
                    &hud_graphics,
                    menu.style,
                    text_spec_resources,
                    allowed_blit_modes,
                    context_item_height,
                )
            })
            .collect()
    }

    fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        self.reject_classic_global_gui_bootstrap()?;
        let restart_same_dialog_fade = self
            .startup_dialog_fade
            .as_ref()
            .and_then(|fade| (fade.outgoing == Some(fade.incoming)).then_some(fade.incoming));
        self.startup_dialog_fade = None;
        self.close_context_menu_silently();
        // Native resize tears down/repositions dialog elements and therefore
        // clears CMouse's owned hover element. A retained screen coordinate
        // must not acquire whichever control moves underneath it.
        self.startup_tooltip.pointer_left();
        self.release_message_dialog_pointer_elements();
        self.context_menu_pointer_capture = None;
        if let Some(dialog) = self.league_signup_dialog.as_mut() {
            dialog.controller.cancel_interaction();
            dialog.controller.reset_location();
        }
        self.league_signup_consumed_keys.clear();
        self.league_signup_pointer_capture = false;
        self.league_signup_pointer_position = None;
        if let Some(dialog) = self.game_option_input_dialog.as_mut() {
            dialog.controller.cancel_interaction();
        }
        self.scenario_game_options.cancel_interaction();
        self.game_option_input_consumed_keys.clear();
        self.game_option_consumed_keys.clear();
        self.game_option_input_pointer_capture = None;
        self.game_option_input_pointer_position = None;
        self.game_option_input_last_click = None;
        self.game_option_pointer_capture = false;
        self.running_pointer_position = None;
        self.ingame_menu_close_pointer_capture = None;
        self.script_menu_close_pointer_capture = None;
        self.scoreboard_pointer_left();
        self.cancel_network_chart_pointer_capture();
        self.menu_title_drag = None;
        for menu in self.ingame_menu.by_player.values_mut() {
            menu.reset_location();
        }
        for state in self.script_menu_presentations.values_mut() {
            reset_script_menu_presentation_location(state);
        }
        let cursor_atlas = self.current_cursor_atlas();
        let hud_graphics = self.current_hud_graphics();
        let game_palette = self.current_game_palette();
        let liquid_animation = self.current_liquid_animation();
        let mut graphics = GraphicsSystem::new(
            width,
            height,
            self.fallback_ground,
            &self.scenario_label,
            self.assets.font_arc(),
            Arc::clone(&self.sprite_cache),
            cursor_atlas,
            hud_graphics,
        );
        graphics.inherit_liquid_animation_cycle(&self.graphics);
        graphics.inherit_pending_observer_scroll(&self.graphics);
        graphics.inherit_debug_draw_state(&self.graphics);
        graphics.inherit_runtime_sprite_filtering(&self.graphics);
        graphics.inherit_advanced_renderer_config(&self.graphics);
        graphics.inherit_cursor_tiers(&self.graphics);
        graphics.set_particle_sprites(Arc::new(particle_sprite_map(&self.engine)));
        graphics.set_clonk_fonts(self.assets.clonk_fonts.clone());
        graphics.set_game_palette(game_palette);
        graphics.set_liquid_animation(liquid_animation);
        graphics.surface_mut().fill(Color::opaque(16, 28, 52));
        self.graphics = graphics;
        self.sync_scenario_game_option_bounds();
        self.graphics.set_sky(self.sky.clone());
        self.graphics
            .set_material_texture_surfaces(Arc::clone(&self.material_texture_images));
        self.graphics
            .set_material_render_info(Arc::clone(&self.material_render_info));

        // Startup models are constructed before boot relinquishes Loading.
        // macOS may apply the real fullscreen size during that interval, so
        // keep their retained geometry in sync in every mode. Otherwise the
        // first Main frame (including buttons and its two footer labels) uses
        // the smaller pre-fullscreen coordinates even though GraphicsSystem
        // already owns the final surface.
        let width_f = width as f32;
        let height_f = height as f32;
        self.menu_state.menu().resize(width_f, height_f);
        self.menu_state.set_pointer_position(None);
        self.main_menu_state.resize(width_f, height_f);
        self.main_menu_state.set_pointer_position(None);

        if self.mode == AppMode::Menu {
            if let Some(dialog) = self.startup_network_dialog.as_mut() {
                dialog.resize(width as i32, height as i32);
                dialog.pointer_left();
            }
            if let Some(dialog) = self.startup_player_dialog.as_mut() {
                if let (Some(fonts), Some(book)) = (
                    self.assets.clonk_fonts.as_deref(),
                    self.assets.plrsel_book_fonts.as_deref(),
                ) {
                    dialog.resize_with_fonts(width as i32, height as i32, fonts, book);
                } else {
                    dialog.resize(width as i32, height as i32);
                }
                dialog.pointer_left();
            }
            if let Some(pending) = self.startup_player_properties_dialog.as_mut() {
                pending.controller.resize(width as i32, height as i32);
                pending.controller.pointer_left();
            }
            if let (Some(dialog), Some(fonts), Some(book)) = (
                self.startup_options_dialog.as_mut(),
                self.assets.clonk_fonts.as_deref(),
                self.assets.options_book_fonts.as_deref(),
            ) {
                dialog.resize(width as i32, height as i32, fonts, book);
                dialog.pointer_left();
            }
            if let Some(pending) = self.startup_options_advanced_dialog.as_mut() {
                pending.controller.resize(width as i32, height as i32);
                pending.controller.cancel_interaction();
            }
            if let (Some(dialog), Some(fonts)) = (
                self.startup_about_dialog.as_mut(),
                self.assets.clonk_fonts.as_deref(),
            ) {
                dialog.resize(width as i32, height as i32, fonts);
                dialog.pointer_left();
            }
            if let Some(lobby) = self.network_lobby.as_mut() {
                lobby.update_layout(width_f, height_f);
                lobby.pointer_left();
            }
            self.cancel_classic_lobby_interaction();
            if let Some(controller) = self.definition_selector.as_mut() {
                controller.cancel_interaction();
            }
        }
        if let Some(dialog) = self.runtime_client_list.as_mut() {
            dialog.pointer_left();
        }
        if self.external_irc_dialog_visible {
            if let Some(dialog) = self.external_irc_dialog.as_mut() {
                dialog.resize(width as i32, height as i32);
                dialog.set_chat_bounds_override(Some(
                    clonk_frontend::startup_netdlg::NetDlgController::standalone_chat_bounds(
                        width as i32,
                        height as i32,
                    ),
                ));
                dialog.pointer_left();
            }
        }
        if let Some(dialog) = restart_same_dialog_fade {
            self.begin_startup_dialog_fade(dialog);
        }
        Ok(())
    }

    fn apply_material_library(&mut self) {
        self.engine
            .set_mission_access_store(self.mission_access.clone());
        self.engine
            .set_show_commands_request_store(self.show_commands_requests.clone());
        self.engine
            .set_control_key_names(configured_control_key_names(&self.bindings));
        self.engine.set_control_host(!matches!(
            self.network_mode.as_ref(),
            Some(NetworkMode::Client(_))
        ));
        self.engine.set_needed_material_resource_strings(
            self.needed_material_need.clone(),
            self.needed_material_none.clone(),
        );
        self.engine
            .set_object_no_dig_resource_string(self.object_no_dig.clone());
        // `FnSetNextMission` substitutes these for omitted arguments
        // (C4Script.cpp:6250,6258).
        self.engine.set_next_mission_defaults(
            self.runtime_resource_text("IDS_BTN_NEXTSCENARIO", "&Next scenario"),
            self.runtime_resource_text("IDS_DESC_NEXTSCENARIO", "Continue with the next scenario."),
        );
        {
            let [undefined, no_construction, no_room, no_level, no_other] =
                self.construction_check_feedback.clone();
            self.engine.set_construction_check_resource_strings(
                undefined,
                no_construction,
                no_room,
                no_level,
                no_other,
            );
        }
        if let Some(names) = self.default_rank_names.as_ref() {
            self.engine.set_default_rank_names(names.clone());
        }
        if let Some(materials) = self.material_library.as_ref() {
            self.engine.set_materials((**materials).clone());
        } else {
            self.engine.set_materials(MaterialSet::default());
        }
    }

    fn apply_material_library_to(&self, engine: &mut Engine) {
        engine.set_mission_access_store(self.mission_access.clone());
        engine.set_show_commands_request_store(self.show_commands_requests.clone());
        engine.set_control_key_names(configured_control_key_names(&self.bindings));
        engine.set_control_host(!matches!(
            self.network_mode.as_ref(),
            Some(NetworkMode::Client(_))
        ));
        engine.set_needed_material_resource_strings(
            self.needed_material_need.clone(),
            self.needed_material_none.clone(),
        );
        engine.set_object_no_dig_resource_string(self.object_no_dig.clone());
        {
            let [undefined, no_construction, no_room, no_level, no_other] =
                self.construction_check_feedback.clone();
            engine.set_construction_check_resource_strings(
                undefined,
                no_construction,
                no_room,
                no_level,
                no_other,
            );
        }
        if let Some(names) = self.default_rank_names.as_ref() {
            engine.set_default_rank_names(names.clone());
        }
        if let Some(materials) = self.material_library.as_ref() {
            engine.set_materials((**materials).clone());
        } else {
            engine.set_materials(MaterialSet::default());
        }
        engine.set_standard_names(self.standard_names.clone());
        self.install_global_scripts_to(engine);
    }

    /// Installs the System.c4g global scripts (the C++ `Game.ScriptEngine`
    /// scripts) into a fresh game engine.
    fn install_global_scripts_to(&self, engine: &mut Engine) {
        if self.system_scripts.is_empty() {
            return;
        }
        let loaded = engine.install_global_scripts(&self.system_scripts);
        tracing::debug!(scripts = loaded, "installed System.c4g global scripts");
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
        let particle_sprites = particle_sprite_map(&self.engine);
        let mut rotateable_definitions = HashSet::new();
        let mut debug_geometry = HashMap::new();
        for definition_id in self.engine.definition_ids() {
            if self.engine.definition_rotateable(definition_id) != 0 {
                rotateable_definitions.insert(definition_id.to_string());
            }
            let actions = self
                .engine
                .definition_action_graphics(definition_id)
                .unwrap_or_default();

            let default_key = sprite_map_key(definition_id, None);
            let graphics_scale = self.engine.definition_graphics_scale(definition_id);
            // A missing parsed DefCore shape is native's zero C4Shape, while
            // `DefinitionSprite::shape == None` is reserved for bootstrap
            // sprites whose only available geometry is their bitmap.
            let shape_facet = Some(
                self.engine
                    .definition_shape_rect(definition_id)
                    .unwrap_or_default(),
            );
            let fire_top = self.engine.definition_fire_top(definition_id);
            let rotateable = self.engine.definition_rotateable(definition_id);
            let line = self.engine.definition_line(definition_id);
            let stretch_growth = self.engine.definition_stretch_growth(definition_id);
            let top_face = self.engine.definition_top_face(definition_id);
            // C4GraphicsOverlay::UpdateFacet takes pSourceGfx->pDef->PictureRect
            // verbatim and unscaled (src/C4DefGraphics.cpp:660-664); the
            // definition Scale is applied to the source crop by C4Facet::DrawT
            // (src/C4Facet.cpp:74-79).
            let picture = self
                .engine
                .definition_picture(definition_id)
                .map(|picture| {
                    clonk_engine::DefinitionRect::new(
                        picture.x,
                        picture.y,
                        picture.width,
                        picture.height,
                    )
                });
            debug_geometry.insert(
                definition_id.to_string(),
                DefinitionDebugGeometry {
                    name: self
                        .engine
                        .definition_name(definition_id)
                        .map(str::to_string),
                    entrance: self.engine.definition_entrance_rect(definition_id),
                    collection: self.engine.definition_collection_rect(definition_id),
                    solid_mask: self.engine.definition_solid_mask(definition_id),
                },
            );
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
                        graphics_scale,
                        shape: shape_facet,
                        fire_top,
                        rotateable,
                        line,
                        stretch_growth,
                        top_face,
                        picture,
                        image: ImageData::from_arc(width, height, pixels),
                        actions: actions.clone(),
                        color_mask: mask,
                    },
                );
            } else if let Some(image) = self.engine.definition_picture_image(definition_id) {
                let width = image.width();
                let height = image.height();
                let mask = image
                    .color_mask()
                    .map(|mask| ColorByOwnerMask::new(width, height, mask));
                sprites.insert(
                    default_key.clone(),
                    DefinitionSprite {
                        graphics_scale,
                        shape: shape_facet,
                        fire_top,
                        rotateable,
                        line,
                        stretch_growth,
                        top_face,
                        picture,
                        image: ImageData::from_arc(width, height, image.into_pixels()),
                        actions: actions.clone(),
                        color_mask: mask,
                    },
                );
            } else if let Some(existing) = sprites.get_mut(&default_key) {
                existing.graphics_scale = graphics_scale;
                existing.actions = actions.clone();
                existing.shape = shape_facet;
                existing.fire_top = fire_top;
                existing.rotateable = rotateable;
                existing.line = line;
                existing.stretch_growth = stretch_growth;
                existing.top_face = top_face;
                existing.picture = picture;
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
                            graphics_scale,
                            shape: shape_facet,
                            fire_top,
                            rotateable,
                            line,
                            stretch_growth,
                            top_face,
                            picture,
                            image: ImageData::from_arc(width, height, pixels),
                            actions: actions.clone(),
                            color_mask: mask,
                        },
                    );
                }
            }
        }
        self.graphics
            .set_rotateable_definitions(rotateable_definitions);
        self.graphics.set_definition_debug_geometry(debug_geometry);
        self.graphics
            .set_particle_sprites(Arc::new(particle_sprites));
        if sprites != self.object_sprites {
            self.object_sprites = sprites;
            self.update_sprite_cache();
        }
    }

    fn copy_search_edit_selection(&mut self, cut: bool) -> bool {
        let result = transfer_edit_selection(&mut self.menu_state.search_edit, cut, |selected| {
            arboard::Clipboard::new()
                .and_then(|mut clipboard| clipboard.set_text(selected.to_string()))
        });
        match result {
            Ok(transferred) => cut && transferred,
            Err(err) => {
                tracing::warn!(error = %err, "failed to copy scenario search text");
                false
            }
        }
    }

    fn paste_search_edit_clipboard(&mut self) -> Result<(), EngineError> {
        let text = match arboard::Clipboard::new().and_then(|mut clipboard| clipboard.get_text()) {
            Ok(text) => text,
            Err(err) => {
                tracing::warn!(error = %err, "failed to paste scenario search text");
                return Ok(());
            }
        };
        self.paste_scenario_search_text(&text)
    }

    fn paste_scenario_search_text(&mut self, text: &str) -> Result<(), EngineError> {
        let before = self.menu_state.search_text().to_string();
        let _submitted = apply_scensel_search_paste(&mut self.menu_state.search_edit, text);
        if self.menu_state.search_text() != before {
            self.submit_scenario_search()?;
        }
        Ok(())
    }

    fn handle_modifiers_changed(&mut self, modifiers: ModifiersState) -> Result<(), EngineError> {
        self.guard_classic_global_gui_bootstrap()?;
        self.keyboard_modifiers = modifiers;
        Ok(())
    }

    fn runtime_halt_active(&self) -> bool {
        if self.network.is_some() {
            // The synchronized status barrier owns network HaltCount. Control
            // stops at the same reached/committed boundary that native tests
            // before Control.Execute and the simulation ticks.
            !self.network_control_running
        } else {
            self.offline_halt_count != 0
        }
    }

    fn set_runtime_pause(&mut self, paused: bool) {
        let role = self.runtime_network_role();
        let currently_paused = match role {
            RuntimeNetworkRole::Offline => self.offline_halt_count != 0,
            RuntimeNetworkRole::Host | RuntimeNetworkRole::Client => {
                self.runtime_network_is_paused()
            }
            RuntimeNetworkRole::Ambiguous => return,
        };
        if paused == currently_paused {
            return;
        }
        match role {
            RuntimeNetworkRole::Offline => {
                // C++ HaltCount is an integer for nested engine holds, but
                // Game::Pause and Game::Unpause assign true/false.
                self.offline_halt_count = i32::from(paused);
            }
            RuntimeNetworkRole::Host | RuntimeNetworkRole::Client
                if self.network_is_league && !self.game_over_handled =>
            {
                let _ = self.submit_own_league_vote(
                    LeagueVoteSubject {
                        vote_type: clonk_engine::VOTE_TYPE_PAUSE,
                        data: i32::from(paused),
                    },
                    true,
                );
            }
            RuntimeNetworkRole::Host => self.request_host_runtime_pause(paused),
            RuntimeNetworkRole::Client => {
                // Native non-host Pause/Unpause is a consumed no-op.
            }
            RuntimeNetworkRole::Ambiguous => unreachable!("handled above"),
        }
    }

    fn toggle_runtime_pause(&mut self) {
        // C4Game::TogglePause refuses while the evaluation dialog owns its
        // temporary halt. This guard applies to the console Pause key too.
        if self.game_over_dialog.is_some() {
            return;
        }
        let paused = match self.runtime_network_role() {
            RuntimeNetworkRole::Offline => self.offline_halt_count != 0,
            RuntimeNetworkRole::Host | RuntimeNetworkRole::Client => {
                self.runtime_network_is_paused()
            }
            RuntimeNetworkRole::Ambiguous => return,
        };
        self.set_runtime_pause(!paused);
    }

    fn apply_engine_pause_game_requests(&mut self) {
        let requests = self.engine.take_pause_game_requests();
        if !matches!(self.mode, AppMode::Running) || self.game_over_dialog.is_some() {
            // The evaluation dialog owns native's temporary game pause.
            // PauseGame(true) is guarded by TogglePause, while PauseGame()
            // sees the existing hold and is a no-op. Rust freezes evaluation
            // at the app boundary, so consume both without creating a hold
            // that would survive Continue.
            return;
        }
        for request in requests {
            match request {
                clonk_engine::PauseGameRequest::Halt => self.set_runtime_pause(true),
                clonk_engine::PauseGameRequest::Toggle => self.toggle_runtime_pause(),
            }
        }
    }

    fn runtime_help_columns(&self) -> Result<&RuntimeHelpColumns> {
        self.runtime_help_text_cache
            .get_or_init(|| {
                // `GetKeyboardInputName` reads the live registration, so the
                // displayed chord follows a KeyConfig override
                // (C4GraphicsSystem.cpp:692-724).
                build_runtime_help_columns_with_keys(
                    &self.startup_tooltip_resources,
                    self.runtime_key_config().ok(),
                )
                .map_err(|error| format!("{error:#}"))
            })
            .as_ref()
            .map_err(|detail| anyhow!(detail.clone()))
    }

    fn runtime_help_resources(&self) -> Result<&RuntimeHelpColumns> {
        anyhow::ensure!(
            self.graphics.hud_graphics().upper_board.is_some(),
            "runtime F1 help requires the classic UpperBoard resource for viewport geometry"
        );
        let mode = frontend_upper_board_mode(self.display_flags.upper_board);
        let viewport_area = self
            .graphics
            .preferred_dialog_rect_for_upper_board_mode(None, mode);
        let expected_top = clonk_frontend::hud::upper_board_reserved_height(mode);
        anyhow::ensure!(
            viewport_area.y == expected_top
                && viewport_area.height < self.graphics.surface().height(),
            "runtime F1 help cannot establish the {:?}-mode {}px viewport origin and message-board bounds on this surface",
            self.display_flags.upper_board,
            expected_top
        );
        self.runtime_key_config()?;
        self.runtime_help_columns()
    }

    fn runtime_flash_resources(&self) -> Result<&RuntimeFlashResources> {
        self.runtime_flash_resources_cache
            .get_or_init(|| {
                Ok(build_runtime_flash_resources(&RuntimeLanguageTable {
                    charset: self.runtime_language_charset,
                    entries: self.startup_tooltip_resources.clone(),
                }))
            })
            .as_ref()
            .map_err(|detail| anyhow!(detail.clone()))
    }

    fn runtime_flash_y(&self) -> i32 {
        let upper_board_height = match self.display_flags.upper_board {
            UpperBoardMode::Hide | UpperBoardMode::Mini => 0,
            UpperBoardMode::Full => clonk_frontend::hud::UPPER_BOARD_HEIGHT,
            UpperBoardMode::Small => clonk_frontend::hud::UPPER_BOARD_HEIGHT / 2,
        };
        10 + upper_board_height
            + if self.runtime_flash_viewport_count() > 1 {
                64
            } else {
                0
            }
    }

    fn prepare_runtime_flash_message(
        &self,
        text: &str,
        charset: RuntimeHelpCharset,
    ) -> Result<Option<RuntimeFlashMessage>> {
        let bytes = runtime_flash_stored_bytes(text, charset)?;
        if bytes.is_empty() {
            return Ok(None);
        }
        let text = decode_runtime_flash_bytes(&bytes, charset)?;
        let remaining_draws = u16::try_from(bytes.len() * 2)
            .context("classic timed flash duration exceeds its 1024-draw bound")?;
        Ok(Some(RuntimeFlashMessage {
            text,
            remaining_draws,
            y: self.runtime_flash_y(),
        }))
    }

    fn set_runtime_flash_message(&mut self, text: &str, charset: RuntimeHelpCharset) -> Result<()> {
        let message = self.prepare_runtime_flash_message(text, charset)?;
        self.runtime_flash_message = message;
        Ok(())
    }

    fn apply_control_presend_change(
        &mut self,
        change: network::ControlPreSendChange,
    ) -> Result<(), EngineError> {
        self.set_network_pacing_flash(&format!(
            "PreSend: {}  - TargetFPS: {}",
            change.control_presend, change.target_fps
        ))
    }

    fn preflight_visible_runtime_flash(&self) -> Result<Option<RuntimeFlashMessage>> {
        let Some(message) = self
            .runtime_flash_message
            .as_ref()
            .filter(|message| message.remaining_draws != 0)
        else {
            return Ok(None);
        };
        Ok(Some(message.clone()))
    }

    fn preflight_visible_runtime_help(&self) -> Result<Option<RuntimeHelpColumns>> {
        if !self.runtime_help_visible {
            return Ok(None);
        }
        self.runtime_help_resources()
            .cloned()
            .map(Some)
            .map_err(|error| {
                tracing::error!(%error, "classic runtime F1 help preflight failed");
                anyhow::Error::new(report_classic_parity_boundary(
                    ClassicParityBoundary::RuntimeHelpResources {
                        detail: error.to_string(),
                    },
                ))
            })
    }

    fn scoreboard_request(&self) -> ScoreboardPresentationRequest {
        let scoreboard = &self.snapshot.hud.scoreboard;
        ScoreboardPresentationRequest {
            rows: scoreboard.row_count(),
            columns: scoreboard.column_count(),
            show_count: scoreboard.show_count(),
            layout_revision: self.engine.scoreboard_layout_revision(),
            title_widget_present: scoreboard
                .cell(0, 0)
                .and_then(clonk_engine::ScoreboardCell::text)
                .is_some_and(|title| !title.is_empty()),
            scoreboard: scoreboard.clone(),
        }
    }

    fn scoreboard_boundary_for_request(
        trigger: ClassicScoreboardTrigger,
        request: &ScoreboardPresentationRequest,
    ) -> ClassicParityBoundary {
        ClassicParityBoundary::Scoreboard {
            trigger,
            rows: request.rows,
            columns: request.columns,
            show_count: request.show_count,
        }
    }

    fn scoreboard_boundary(&self, trigger: ClassicScoreboardTrigger) -> ClassicParityBoundary {
        Self::scoreboard_boundary_for_request(trigger, &self.scoreboard_request())
    }

    fn scoreboard_presentation_error(
        &self,
        trigger: ClassicScoreboardTrigger,
        error: anyhow::Error,
    ) -> anyhow::Error {
        tracing::error!(%error, ?trigger, "classic scoreboard presentation failed");
        anyhow::Error::new(report_classic_parity_boundary(
            self.scoreboard_boundary(trigger),
        ))
    }

    /// Reproduce the synchronous C4ScoreboardDlg constructor Update from the
    /// request-time matrix. SetCell may already have invalidated it, but C++
    /// retains these bounds for input until Draw performs the lazy Update.
    fn materialize_scoreboard_presentation(&mut self) -> Result<()> {
        if self.scoreboard_runtime.presentation.is_some() {
            return Ok(());
        }
        let Some(request) = self.scoreboard_dialog.clone() else {
            return Ok(());
        };
        let font_images = resolve_scoreboard_font_images(
            &self.engine,
            &request.scoreboard,
            self.script_text_spec_resources(),
        )?;
        let assets = Arc::clone(&self.assets);
        let resources = assets.scoreboard_resources(&font_images)?;
        let live_preferred = scoreboard_preferred_rect(
            self.graphics
                .preferred_dialog_rect(self.mouse_control.then_some(self.local_owner)),
        );
        let preferred = self.scoreboard_runtime.preferred.unwrap_or(live_preferred);
        self.scoreboard_runtime.presentation = Some(
            clonk_frontend::scoreboard::ScoreboardPresentationState::new(
                preferred,
                &request.scoreboard,
                &resources,
            )?,
        );
        self.scoreboard_runtime.layout_revision = request.layout_revision;
        Ok(())
    }

    /// Validate the live draw matrix, then apply C4ScoreboardDlg::Draw's lazy
    /// Update exactly once for the current SetCell generation. A preferred-
    /// viewport change alone never moves the retained dialog.
    fn preflight_visible_scoreboard_unchecked(
        &mut self,
    ) -> Result<Option<HashMap<String, ImageData>>> {
        if self.scoreboard_dialog.is_none() {
            return Ok(None);
        }
        self.materialize_scoreboard_presentation()?;
        let font_images = resolve_scoreboard_font_images(
            &self.engine,
            &self.snapshot.hud.scoreboard,
            self.script_text_spec_resources(),
        )?;
        let assets = Arc::clone(&self.assets);
        let resources = assets.scoreboard_resources(&font_images)?;
        let scoreboard = self.snapshot.hud.scoreboard.clone();
        let layout_revision = self.engine.scoreboard_layout_revision();
        if self.scoreboard_runtime.layout_revision != layout_revision {
            let preferred = scoreboard_preferred_rect(
                self.graphics
                    .preferred_dialog_rect(self.mouse_control.then_some(self.local_owner)),
            );
            self.scoreboard_runtime.preferred = Some(preferred);
            self.scoreboard_runtime
                .presentation
                .as_mut()
                .expect("constructor presentation was materialized")
                .update(preferred, &scoreboard, &resources)?;
            self.scoreboard_runtime.layout_revision = layout_revision;
        }
        if self
            .scoreboard_runtime
            .presentation
            .as_ref()
            .is_none_or(|presentation| presentation.layout().close_button.is_none())
        {
            self.scoreboard_close_pointer_capture = false;
            self.scoreboard_runtime.close_hovered = false;
        }
        Ok(Some(font_images))
    }

    /// Resolve every live `FontRegular` custom image and validate layout
    /// before the viewport renderer may touch the output surface. The owned
    /// image map survives until the later GUI draw pass.
    fn preflight_visible_scoreboard(&mut self) -> Result<Option<HashMap<String, ImageData>>> {
        let trigger = ClassicScoreboardTrigger::ScriptVisibility;
        let result = self.preflight_visible_scoreboard_unchecked();
        result.map_err(|error| self.scoreboard_presentation_error(trigger, error))
    }

    fn scoreboard_tooltip_target_cached(&self, point: GuiPoint) -> Option<StartupTooltip> {
        match self.scoreboard_pointer_target_cached(point)? {
            ScoreboardPointerTarget::Close => Some(StartupTooltip::resource("IDS_MNU_CLOSE")),
            ScoreboardPointerTarget::Title => self
                .scoreboard_runtime
                .presentation
                .as_ref()
                .and_then(|presentation| presentation.title())
                .map(StartupTooltip::text),
            ScoreboardPointerTarget::Dialog => None,
        }
    }

    fn scoreboard_is_above_all_messages(&self) -> bool {
        let scoreboard = self
            .running_dialog_stack
            .iter()
            .rposition(|entry| *entry == RunningDialogStackEntry::Scoreboard);
        let top_message = self
            .running_dialog_stack
            .iter()
            .rposition(|entry| matches!(entry, RunningDialogStackEntry::Message(_)));
        matches!((scoreboard, top_message), (Some(scoreboard), Some(message)) if scoreboard > message)
    }

    fn running_message_index(&self, stack_id: u64) -> Option<usize> {
        self.message_dialogs
            .iter()
            .position(|dialog| dialog.running_stack_id == stack_id)
    }

    fn running_shared_entry_is_in_tail(&self, entry: RunningDialogStackEntry) -> bool {
        let split = self
            .running_dialog_stack
            .iter()
            .position(|candidate| candidate.z_order() > 0)
            .unwrap_or(self.running_dialog_stack.len());
        self.running_dialog_stack
            .iter()
            .rposition(|candidate| *candidate == entry)
            .is_some_and(|position| position >= split)
    }

    fn scoreboard_opening_blocked_by_game_over(&self) -> bool {
        self.game_over_dialog.is_some() || (self.snapshot.game_over && !self.game_over_handled)
    }

    /// Arms runtime request capture only after scenario initialization or save
    /// restoration has produced its final snapshot. The explicit game-start
    /// DoDlgShow(0,false) reconciliation is deferred to the first input/tick/
    /// render so a visible dialog is validated against the final live matrix
    /// and exact resources before any output pixels.
    fn arm_initial_scoreboard_reconcile(&mut self) {
        self.engine.begin_scoreboard_presentation_capture();
        self.snapshot.hud.scoreboard_presentations.clear();
        self.close_scoreboard_dialog();
        self.scoreboard_initial_reconcile_pending = true;
    }

    fn reconcile_initial_scoreboard(&mut self) {
        if !matches!(self.mode, AppMode::Running) || !self.scoreboard_initial_reconcile_pending {
            return;
        }
        self.scoreboard_initial_reconcile_pending = false;
        let request = self.scoreboard_request();
        if request.should_be_shown() && !self.scoreboard_opening_blocked_by_game_over() {
            self.open_scoreboard_dialog(request);
        } else {
            self.close_scoreboard_dialog();
        }
    }

    fn apply_scoreboard_presentation_requests(&mut self) {
        let requests = std::mem::take(&mut self.snapshot.hud.scoreboard_presentations);
        for request in requests {
            if request.should_be_shown() {
                if self.scoreboard_dialog.is_none()
                    && !self.scoreboard_opening_blocked_by_game_over()
                {
                    self.open_scoreboard_dialog(request);
                }
            } else if self.scoreboard_dialog.is_some() {
                self.close_scoreboard_dialog();
            }
        }
    }

    /// C4 registers ScoreboardToggle at PRIO_Base. Active dialog focus and a
    /// configured local-player control therefore get first refusal, while a
    /// context menu's unrecognized Tab falls through to the lower priorities
    /// (C4KeyboardInput.h:343-353; C4GuiMenu.cpp:302-325).
    fn scoreboard_tab_has_higher_priority_route(&self) -> bool {
        self.context_menu.is_none()
            && ((self.runtime_gui_has_keyboard_focus() && !self.network_chart_elevated)
                || self.runtime_top_default_dialog_is_exclusive())
    }

    fn handle_focus_lost(&mut self) -> Result<(), EngineError> {
        self.voice_chat.stop_capture();
        self.guard_classic_global_gui_bootstrap()?;
        self.primary_pointer_left_down = false;
        self.ingame_menu_close_pointer_capture = None;
        self.script_menu_close_pointer_capture = None;
        self.menu_title_drag = None;
        // No native backend clears player controls on focus loss: Win32
        // deactivation only minimizes a fullscreen window
        // (C4FullScreen.cpp:139-145), X11 FocusOut/Unmap only clears
        // `Application.Active` (:310-315), and the SDL branch does not handle
        // focus at all (:432-447). Synchronized `ClearPressed` belongs to the
        // explicit modal flows (C4PlayerList.cpp:588-595). Only the physical
        // repeat state is forgotten below, so the first press after refocus is
        // not discarded as a repeat.
        self.close_context_menu_silently();
        self.context_menu_pointer_capture = None;
        for dialog in &mut self.message_dialogs {
            dialog.state.cancel_interaction();
        }
        self.message_dialog_pointer_capture_index = None;
        if let Some(controller) = self.definition_selector.as_mut() {
            controller.cancel_interaction();
        }
        if let Some(dialog) = self.game_option_input_dialog.as_mut() {
            dialog.controller.cancel_interaction();
        }
        if let Some(dialog) = self.league_signup_dialog.as_mut() {
            dialog.controller.cancel_interaction();
        }
        self.scenario_game_options.cancel_interaction();
        self.cancel_classic_lobby_interaction();
        self.message_dialog_consumed_keys.clear();
        self.network_chart_consumed_keys.clear();
        self.runtime_client_list_consumed_keys.clear();
        self.definition_selector_consumed_keys.clear();
        self.definition_selector_pointer_capture = false;
        self.league_signup_consumed_keys.clear();
        self.league_signup_pointer_capture = false;
        self.league_signup_pointer_position = None;
        self.game_option_input_consumed_keys.clear();
        self.game_option_input_pointer_capture = None;
        self.game_option_input_pointer_position = None;
        self.game_option_consumed_keys.clear();
        self.game_option_pointer_capture = false;
        self.chat_paste_consumed_keys.clear();
        self.pressed_engine_keys.clear();
        self.scoreboard_tab_raw_pressed = false;
        self.keyboard_modifiers = ModifiersState::empty();
        self.running_pointer_position = None;
        if let Some(rename) = self.startup_crew_rename.as_mut() {
            rename.edit.cancel_pointer_selection();
            rename.last_click = None;
            rename.ignore_pointer_up = false;
        }
        self.pointer_left_unchecked();
        self.mouse_state = None;
        self.ingame_right_mouse_state = None;
        self.construction_menu_drag = None;
        self.ingame_dragged_objects.clear();
        self.ingame_last_left_down = None;
        self.ingame_ignore_left_up = false;
        Ok(())
    }

    /// The refresh ceiling for each application timer. Only the startup one is
    /// ever subdivided below the oracle default; see `RefreshCeilings`.
    pub(crate) fn refresh_ceilings(&self) -> RefreshCeilings {
        RefreshCeilings {
            running_ms: self.max_refresh_delay_ms,
            startup_ms: self.startup_refresh_delay_ms,
        }
    }

    fn handle_focus_gained(&mut self) -> Result<(), EngineError> {
        self.window_active = true;
        let Some(point) = self.window_mouse_position else {
            return Ok(());
        };
        // Focus loss clears C4MouseControl/C4GUI hover ownership but retains
        // the client-area position. Re-route it so a stationary pointer is
        // themed again as soon as the OS cursor is hidden on reactivation.
        self.handle_cursor_moved(PhysicalPosition::new(
            f64::from(point.x),
            f64::from(point.y),
        ))
    }

    fn dispatch_control_event(&mut self, event: ControlEvent) -> Result<(), EngineError> {
        self.dispatch_control_event_for_local_player(self.local_owner, event)
    }

    fn dispatch_control_event_for_owner(
        &mut self,
        owner: i32,
        event: ControlEvent,
    ) -> Result<(), EngineError> {
        if self.ingame_menu_belongs_to(owner) || owner == self.local_owner {
            if let ControlEvent::Command { command, kind } = event {
                if self.handle_menu_command_failsafe(owner, command, kind)? {
                    return Ok(());
                }
            }
            if self.ingame_menu_belongs_to(owner)
                || (owner == self.local_owner && self.object_menu.is_some())
            {
                return Ok(());
            }
        }
        if let Some(packet) = (NetworkControl::Player { owner, event }).into_packet() {
            self.record_control_batch(std::slice::from_ref(&packet));
        }
        if let Err(err) = self.input.handle_event(&mut self.engine, owner, event) {
            let status = control_script_error_to_status(err)?;
            tracing::error!(status, "control script error (non-fatal like C++)");
            self.status_text = status;
        }
        Ok(())
    }

    fn local_control_submission_tick(&self) -> Tick {
        let after_executing = self.executing_ready_tick.map(|tick| tick.saturating_add(1));
        let next_unsent = self
            .network_control_clock
            .and_then(|clock| Tick::try_from(clock.next_unsent_tick()).ok());
        after_executing
            .into_iter()
            .chain(next_unsent)
            .max()
            .unwrap_or_else(|| u32::try_from(self.engine.frame()).unwrap_or(u32::MAX))
    }

    fn clear_local_control(&mut self, owner: i32) -> Result<(), EngineError> {
        if let Some(network) = self.network.as_ref() {
            let tick = self.local_control_submission_tick();
            network.submit_local_control(owner, ControlEvent::ClearPressed, tick);
            return Ok(());
        }
        if let Some(packet) = (NetworkControl::Player {
            owner,
            event: ControlEvent::ClearPressed,
        })
        .into_packet()
        {
            self.record_control_batch(std::slice::from_ref(&packet));
        }
        let _ = self
            .input
            .handle_event(&mut self.engine, owner, ControlEvent::ClearPressed)?;
        Ok(())
    }

    fn clear_local_controls(&mut self) -> Result<(), EngineError> {
        let mut owners = self.local_controls.owners().collect::<Vec<_>>();
        if owners.is_empty() && self.engine.player(self.local_owner).is_some() {
            owners.push(self.local_owner);
        }
        for owner in owners {
            self.clear_local_control(owner)?;
        }
        Ok(())
    }

    fn hostility_opponent_is_user(&self, opponent: i32) -> bool {
        let Some(opponent) = self.engine.player(opponent) else {
            return false;
        };
        self.control_player_infos
            .get(opponent.player_info_id())
            .is_none_or(|info| info.player_type == clonk_engine::PLAYER_INFO_TYPE_USER)
    }

    /// `C4Game::Abort(false)`: league rounds vote first; all remaining cases
    /// enter the hard QuitGame path. Restart is represented by the retained
    /// `Application.NextMission` analogue consumed from `return_to_menu`.
    fn route_abort_confirmation(&mut self) -> Result<(), EngineError> {
        if self.network.is_some() && self.network_is_league && !self.engine.is_game_over() {
            if self.engine.is_control_host() {
                let _ = self.submit_own_league_vote(
                    LeagueVoteSubject {
                        vote_type: clonk_engine::VOTE_TYPE_CANCEL,
                        data: 0,
                    },
                    true,
                );
                return Ok(());
            }
            let local_client_id = self
                .network
                .as_ref()
                .and_then(|network| i32::try_from(network.local_client_id()).ok());
            let has_local_player = self
                .local_controls
                .owners()
                .any(|owner| self.engine.player(owner).is_some());
            if let Some(local_client_id) = local_client_id.filter(|_| has_local_player) {
                let _ = self.submit_own_league_vote(
                    LeagueVoteSubject {
                        vote_type: clonk_engine::VOTE_TYPE_KICK,
                        data: local_client_id,
                    },
                    true,
                );
                return Ok(());
            }
        }
        self.hard_abort_running_game()
    }

    fn hard_abort_running_game(&mut self) -> Result<(), EngineError> {
        let local_client_id = self
            .network
            .as_ref()
            .and_then(|network| i32::try_from(network.local_client_id()).ok())
            .unwrap_or(-1);
        let _ = self
            .engine
            .abort_players_without_callbacks(local_client_id)?;
        self.snapshot = self.engine.snapshot();
        if !self.abort_restart_pending && self.network.is_some() {
            self.change_network_control_to_local(local_client_id);
        }
        self.return_to_menu();
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

    /// Unique live definitions with the given category bit, in object-list
    /// order (`Game.Objects.ObjectsInt().GetListID`, C4MainMenu.cpp:394).
    fn goal_rule_entries(&self, category_bit: i32) -> Vec<GoalRuleEntry> {
        let mut seen = HashSet::new();
        let mut entries = Vec::new();
        for object in &self.snapshot.objects {
            if !object.status.is_active() {
                continue;
            }
            let id = object.definition_id.as_str();
            if !seen.insert(id.to_string()) {
                continue;
            }
            let category = self.engine.definition_category(id).unwrap_or(0);
            if category & category_bit == 0 {
                continue;
            }
            entries.push(GoalRuleEntry {
                definition_id: id.to_string(),
                name: self.engine.definition_name(id).unwrap_or(id).to_string(),
                description: self.engine.definition_description(id).map(str::to_string),
                fulfilled: false,
            });
        }
        entries
    }

    fn submit_game_goal_rule_activation(
        &mut self,
        player: i32,
        unresolved: ClassicIngameMenuChild,
    ) -> Result<(), EngineError> {
        let definition_id = match &unresolved {
            ClassicIngameMenuChild::GoalInfo(id) | ClassicIngameMenuChild::RuleInfo(id) => id,
            _ => unreachable!("goal/rule activation requires its corresponding typed boundary"),
        };
        let Some(object) = self
            .engine
            .first_active_object_for_definition(definition_id)
            .and_then(|object| i32::try_from(object.as_u64()).ok())
        else {
            return Err(classic_ingame_menu_child_error(unresolved));
        };
        if self.network.is_some() {
            let tick = self.local_control_submission_tick();
            if let Some(Err(error)) = self
                .network
                .as_ref()
                .map(|network| network.submit_activate_game_goal_rule(tick, player, object))
            {
                tracing::warn!(player, object, %error, "failed to queue goal/rule activation");
            }
        } else {
            let by_client = self
                .engine
                .player(player)
                .map(|player| player.at_client().get())
                .unwrap_or(-1);
            let control = clonk_engine::ActivateGameGoalRuleControlData {
                object,
                player,
                by_client,
            };
            self.record_control_batch(std::slice::from_ref(
                &clonk_engine::ControlPacket::ActivateGameGoalRule(control),
            ));
            self.engine
                .execute_activate_game_goal_rule_control(&control)?;
        }
        Ok(())
    }

    /// Loads the definition pictures for goal/rule menu symbols
    /// (`pDef->Draw(fctSymbol)`, C4MainMenu.cpp:367,397).
    fn cache_definition_icons(&mut self, entries: &[GoalRuleEntry]) -> Result<(), EngineError> {
        let icons =
            entries
                .iter()
                .map(|entry| {
                    let image = self
                        .engine
                        .try_definition_picture_image(&entry.definition_id)
                        .map_err(|error| {
                            classic_parity_engine_error(report_classic_parity_boundary(
                                ClassicParityBoundary::IngameMenuDefinitionIcon {
                                    definition_id: entry.definition_id.clone(),
                                    detail: error.to_string(),
                                },
                            ))
                        })?;
                    Ok(image
                        .map(|image| (entry.definition_id.clone(), definition_menu_picture(image))))
                })
                .collect::<std::result::Result<Vec<_>, EngineError>>()?;
        let gfx = self.ensure_ingame_menu_gfx();
        for (id, image) in icons.into_iter().flatten() {
            gfx.definition_icons.insert(id, image);
        }
        Ok(())
    }

    fn prepare_runtime_resource_flash(
        &self,
        message: impl Fn(&RuntimeFlashResources) -> String,
    ) -> Option<RuntimeFlashMessage> {
        let fallback = RuntimeFlashResources::undefined();
        let resources = match self.runtime_flash_resources() {
            Ok(resources) => resources,
            Err(error) => {
                tracing::warn!(%error, "debug flash resources unavailable; using C++ undefined labels");
                &fallback
            }
        };
        let message_text = message(resources);
        match self.prepare_runtime_flash_message(&message_text, resources.charset) {
            Ok(flash) => flash,
            Err(error) => {
                tracing::warn!(%error, "debug flash text is not renderable; using C++ undefined labels");
                self.prepare_runtime_flash_message(&message(&fallback), fallback.charset)
                    .ok()
                    .flatten()
            }
        }
    }

    fn prepare_runtime_speed_flash(
        &self,
        frame_skip: i32,
    ) -> Result<Option<RuntimeFlashMessage>, EngineError> {
        let (charset, message_text) = self
            .runtime_flash_resources()
            .map(|resources| (resources.charset, resources.speed(frame_skip)))
            .map_err(|error| {
                classic_parity_engine_error(report_classic_parity_boundary(
                    ClassicParityBoundary::RuntimeFlashResources {
                        detail: error.to_string(),
                    },
                ))
            })?;
        self.prepare_runtime_flash_message(&message_text, charset)
            .map_err(|error| {
                classic_parity_engine_error(report_classic_parity_boundary(
                    ClassicParityBoundary::RuntimeFlashResources {
                        detail: error.to_string(),
                    },
                ))
            })
    }

    /// C4Game::SpeedUp/SlowDown use a narrower interactive 1..=50 clamp
    /// than `/fast`, and only the key callbacks produce IDS_MSG_SPEED.
    fn step_runtime_speed(&mut self, speed_up: bool) -> Result<(), EngineError> {
        let frame_skip = if speed_up {
            self.frame_skip.saturating_add(1).clamp(1, 50)
        } else {
            self.frame_skip.saturating_sub(1).clamp(1, 50)
        };
        let flash_message = self.prepare_runtime_speed_flash(frame_skip)?;
        self.frame_skip = frame_skip;
        if speed_up {
            self.full_speed = true;
        } else if frame_skip == 1 {
            self.full_speed = false;
        }
        self.runtime_flash_message = flash_message;
        Ok(())
    }

    /// Restart the running round (C4AbortGameDialog's Restart button:
    /// `Application.SetNextMission` + `Game.Abort`, C4GameDialogs.cpp:116-120).
    fn retain_restart_restore_mask_for_restart(&mut self) {
        self.restart_restore_infos.what = self.engine.restart_restore_info_mask();
    }

    fn load_saved_game_from_path(&mut self, path: &Path) -> Result<()> {
        let contents = fs::read_to_string(path)
            .with_context(|| format!("failed to read save from {}", path.display()))?;
        let mut save: SavedGameFile = serde_json::from_str(&contents)
            .with_context(|| format!("failed to parse save data from {}", path.display()))?;
        save.source_title_png = fs::read(path.with_extension("png")).ok();
        let save = migrate_save_file(save)?;
        self.apply_loaded_game(save)?;
        self.last_save_path = Some(path.to_path_buf());
        Ok(())
    }

    fn complete_prepared_network_go(
        &mut self,
        network_savegame: bool,
    ) -> Result<bool, EngineError> {
        if !self.finalize_network_loaded_scenario(network_savegame)? {
            return Ok(false);
        }
        self.advance_scenario_loader(100, "Scenario activation complete");
        self.loading_state = None;
        self.pending_client_start_status = None;
        self.running_gui_mouse_owned = false;
        self.running_world_mouse_owned = true;
        self.mode = AppMode::Running;
        Ok(true)
    }

    fn finish_network_go_activation_tail(&mut self) {
        if let Some(network) = self.network.as_ref() {
            network.reset_client_performance();
        }
        self.network_control_running = true;
        self.refresh_network_client_next_control_ticks();
        self.publish_running_host_reference();
    }

    fn try_finish_deferred_prepared_network_go(&mut self) -> Result<(), EngineError> {
        let ready = self.mode == AppMode::Loading
            && self
                .runtime_network_committed_status
                .is_some_and(|status| status.state == clonk_network::NETWORK_STATE_GO)
            && self.loading_state.as_ref().is_some_and(|loading| {
                loading.finished
                    && loading
                        .prepared_go
                        .as_ref()
                        .is_some_and(|prepared| prepared.local_reached)
            });
        if !ready {
            return Ok(());
        }
        let network_savegame = self
            .loading_state
            .as_ref()
            .and_then(|loading| loading.prepared_go.as_ref())
            .is_some_and(|prepared| prepared.save_game);
        if self.complete_prepared_network_go(network_savegame)? {
            self.finish_network_go_activation_tail();
        }
        Ok(())
    }

    fn handle_status_committed(
        &mut self,
        status: clonk_network::NetworkStatus,
    ) -> Result<(), EngineError> {
        let runtime_commit = self.runtime_network_status_barrier;
        if runtime_commit.is_some_and(|pending| {
            !pending.local_reached || !same_runtime_network_status_barrier(pending.status, status)
        }) {
            tracing::warn!(
                state = status.state,
                target_tick = status.target_tick,
                "ignoring runtime network commit before local arrival"
            );
            return Ok(());
        }
        let client_commit = if runtime_commit.is_some() {
            true
        } else if matches!(self.network_mode, Some(NetworkMode::Client(_))) {
            self.client_start_barrier.status_committed(status).is_some()
        } else {
            true
        };
        if !client_commit {
            tracing::warn!(
                state = status.state,
                target_tick = status.target_tick,
                "ignoring network commit before the client reached its matching barrier"
            );
            return Ok(());
        }
        self.network_control_running = false;
        let prepared_go = self
            .loading_state
            .as_ref()
            .and_then(|loading| loading.prepared_go.as_ref())
            .map(|pending| (pending.status, pending.local_reached, pending.save_game));
        if let Some((expected, local_reached, _)) = prepared_go {
            let matching_barrier = if matches!(self.network_mode, Some(NetworkMode::Client(_))) {
                expected.state == status.state && expected.target_tick == status.target_tick
            } else {
                expected == status
            };
            if !matching_barrier || !local_reached {
                tracing::warn!(
                    expected_state = expected.state,
                    expected_control_mode = expected.control_mode,
                    expected_target_tick = expected.target_tick,
                    state = status.state,
                    control_mode = status.control_mode,
                    target_tick = status.target_tick,
                    "ignoring commit before the prepared game reached its exact Go barrier"
                );
                return Ok(());
            }
        }
        let Ok(target_tick) = Tick::try_from(status.target_tick) else {
            tracing::warn!(
                target_tick = status.target_tick,
                "ignoring negative committed status tick"
            );
            return Ok(());
        };
        if status.state == clonk_network::NETWORK_STATE_GO {
            // Native DoLobby destroys pLobby before returning into scenario
            // initialization. Clear a joined-client adapter at the same
            // boundary so no hidden lobby state can consume synchronized
            // runtime traffic (src/C4Network2.cpp:493-515).
            let closed_joined_lobby = self.network_lobby.take().is_some();
            if closed_joined_lobby && self.mode != AppMode::Running {
                self.close_context_menu_silently();
            }
        }
        self.runtime_network_control_mode = Some(status.control_mode);
        self.runtime_network_committed_status = Some(status);
        if status.state == clonk_network::NETWORK_STATE_GO {
            self.runtime_network_committed_control_mode = Some(status.control_mode);
        }
        if let Some(clock) = self.network_control_clock.as_mut() {
            clock.set_target_tick(None);
        }
        // Clear the completed barrier before synchronized controls execute.
        // A VoteEnd control may synchronously open the follow-up Go barrier;
        // that newer transition must own the final running/reference state.
        if runtime_commit.is_some() {
            self.runtime_network_status_barrier = None;
        }
        if status.state == clonk_network::NETWORK_STATE_GO
            && matches!(self.network_mode, Some(NetworkMode::Host(_)))
        {
            // Host CheckStatusAck assigns fStatusAck before its local
            // ExecSyncControl, so Network::isRunning already reports true.
            // Clients execute PID_ExecSyncCtrl before receiving the status
            // acknowledgement and remain paused until the tail below.
            self.network_control_running = true;
        }
        let sync_tick = runtime_commit
            .and_then(|pending| pending.actual_control_tick)
            .and_then(|tick| Tick::try_from(tick).ok())
            .unwrap_or(target_tick);
        let sync_controls = self.network_sync.take_exact(sync_tick);
        if !sync_controls.is_empty() {
            self.apply_synchronized_controls(sync_tick, sync_controls)?;
        }
        if runtime_commit.is_some()
            && (self.mode != AppMode::Running
                || self.network.is_none()
                || self.runtime_network_status_barrier.is_some())
        {
            return Ok(());
        }
        if status.state == clonk_network::NETWORK_STATE_GO {
            self.host_reference_paused = false;
            if matches!(self.network_mode.as_ref(), Some(NetworkMode::Host(_))) {
                for client_id in self.control_clients.activated_client_ids() {
                    self.issue_unjoined_joins_for_client(client_id);
                }
            }
            if prepared_go.is_some() {
                let network_savegame =
                    prepared_go.is_some_and(|(_, _, network_savegame)| network_savegame);
                if self.complete_prepared_network_go(network_savegame)? {
                    self.finish_network_go_activation_tail();
                }
            } else {
                self.finish_network_go_activation_tail();
            }
        } else if status.state == clonk_network::NETWORK_STATE_PAUSE {
            self.host_reference_paused = true;
            self.publish_running_host_reference();
        }
        Ok(())
    }

    fn append_network_client_join_log(&mut self, name: &[u8]) {
        // C4ControlClientJoin logs only after the host-authored core was
        // accepted, before refreshing the lobby client list
        // (src/C4Control.cpp:552-565). C4Log then routes this ordinary log to
        // MainDlg::OnLog (src/C4Log.cpp:227-239;
        // src/C4GameLobby.cpp:738-753).
        let name = legacy_presentation_text(name);
        let template = self.runtime_resource_text("IDS_NET_CLIENT_JOIN", "Client %s connected.");
        let message = format_resource_string(template, &[&name]);
        self.append_control_message_log(message, CONTROL_LOG_COLOR, None);
    }

    fn append_control_message_log(&mut self, line: String, color: u32, lobby_sender: Option<i32>) {
        tracing::info!(message = %line, "network message");
        let line = self.timestamp_log_line(line);
        let color = lobby_sender
            .map(|client_id| {
                if self.control_message_has_lobby() && !self.control_clients.contains(client_id) {
                    self.network
                        .as_ref()
                        .and_then(|network| i32::try_from(network.local_client_id()).ok())
                        .unwrap_or(0)
                } else {
                    client_id
                }
            })
            .map(|client_id| self.control_message_lobby_chat_color(client_id))
            .unwrap_or(color);
        let rgba = clonk_frontend::game_lobby::make_color_readable_on_black(color);
        if let Some(lobby) = self.classic_host_lobby.as_mut() {
            lobby.controller.push_log(LobbyLogLine {
                text: line,
                color: rgba,
            });
            return;
        }
        if self.startup_view == StartupView::NetworkLobby {
            if let Some(lobby) = self.network_lobby.as_mut() {
                lobby.push_log(LobbyLogLine {
                    text: line,
                    color: rgba,
                });
                return;
            }
        }
        self.enqueue_control_message_board_line(line);
    }

    fn timestamp_log_line(&self, line: String) -> String {
        if self.show_log_timestamps {
            format!(
                "{} {line}",
                clonk_core::chrono_util::current_timestamp(true)
            )
        } else {
            line
        }
    }

    /// Native ProcessCommand entries that mutate only this process. Command
    /// names are case-sensitive; `/msgboard` is available only in-game.
    fn process_control_message_local_command(&mut self, text: &str) -> bool {
        let raw = clonk_script::c4_string_bytes(text);
        let Some(command) = raw.strip_prefix(b"/") else {
            return false;
        };
        let (name, parameter) = command
            .iter()
            .position(|byte| *byte == b' ')
            .map_or((command, &[][..]), |space| {
                (&command[..space], &command[space + 1..])
            });
        if name == b"msgboard" {
            if !matches!(self.mode, AppMode::Running) {
                return false;
            }
            let parameter = native_bytes_as_legacy_text(parameter);
            self.set_message_board_line_count(legacy_atoi_i32(&parameter).clamp(0, 20));
            return true;
        }
        let muted = match name {
            b"mute" => true,
            b"unmute" => false,
            _ => return false,
        };
        if let Some(client_id) = self
            .control_clients
            .snapshot()
            .into_iter()
            .find(|client| client.name.as_bytes() == parameter)
            .map(|client| client.client_id)
        {
            self.control_messages.set_muted(client_id, muted);
        }
        true
    }

    fn append_running_command_help(&mut self) {
        let header = self.runtime_resource_text(
            "IDS_TEXT_COMMANDSAVAILABLEDURINGGA",
            "Commands available during game:",
        );
        self.append_control_message_log(header, CONTROL_LOG_COLOR, None);
        for (syntax, key, fallback) in [
            (
                "/private [player] [message]",
                "IDS_MSG_SENDAPRIVATEMESSAGETOTHES",
                "Send a private message to the specified player.",
            ),
            (
                "/team [message]",
                "IDS_MSG_SENDAPRIVATEMESSAGETOYOUR",
                "Send a private message to your team.",
            ),
            (
                "/me [action]",
                "IDS_TEXT_PERFORMANACTIONINYOURNAME",
                "Perform an action in your name.",
            ),
            (
                "/sound [sound]",
                "IDS_TEXT_PLAYASOUNDFROMTHEGLOBALSO",
                "Play a sound from the global sound group.",
            ),
            (
                "/mute [client]",
                "IDS_TEXT_MUTESOUNDCOMMANDSBYTHESPE",
                "Mute /sound commands by specified client.",
            ),
            (
                "/unmute [client]",
                "IDS_TEXT_UNMUTESOUNDCOMMANDSBYTHESP",
                "Unmute /sound commands by the specified client.",
            ),
            (
                "/kick [client]",
                "IDS_TEXT_KICKTHESPECIFIEDCLIENT",
                "Kick the specified client.",
            ),
            (
                "/observer [client]",
                "IDS_TEXT_SETTHESPECIFIEDCLIENTTOOB",
                "Set the specified client to observer mode.",
            ),
            (
                "/fast [x]",
                "IDS_TEXT_SETTOFASTMODESKIPPINGXFRA",
                "Set to fast mode, skipping x frames.",
            ),
            (
                "/slow",
                "IDS_TEXT_SETTONORMALSPEEDMODE",
                "Set to normal speed mode.",
            ),
            (
                "/chart",
                "IDS_TEXT_DISPLAYNETWORKSTATISTICS",
                "Display network statistics.",
            ),
            (
                "/nodebug",
                "IDS_TEXT_PREVENTDEBUGMODEINTHISROU",
                "Prevent debug mode in this round.",
            ),
            (
                "/set comment [comment]",
                "IDS_TEXT_SETANEWNETWORKCOMMENT",
                "Set a new network comment.",
            ),
            (
                "/set password [password]",
                "IDS_TEXT_SETANEWNETWORKPASSWORD",
                "Set a new network password.",
            ),
            (
                "/set faircrew [on/off]",
                "IDS_TEXT_ENABLEORDISABLEFAIRCREW",
                "Enable or disable fair crew.",
            ),
            (
                "/set maxplayer [4]",
                "IDS_TEXT_SETANEWMAXIMUMNUMBEROFPLA",
                "Set a new maximum number of players for this round.",
            ),
            (
                "/script [script]",
                "IDS_TEXT_EXECUTEASCRIPTCOMMAND",
                "Execute a script command.",
            ),
            (
                "/clear",
                "IDS_MSG_CLEARTHEMESSAGEBOARD",
                "Clear the message board.",
            ),
        ] {
            let description = self.runtime_resource_text(key, fallback);
            self.append_control_message_log(
                format!("{syntax} - {description}"),
                CONTROL_LOG_COLOR,
                None,
            );
        }
    }

    fn append_unknown_running_command(&mut self, text: &str) {
        let raw = clonk_script::c4_string_bytes(text);
        let name = raw
            .get(1..)
            .unwrap_or_default()
            .split(|byte| *byte == b' ')
            .next()
            .unwrap_or_default();
        let name = legacy_presentation_text(&name[..name.len().min(30)]);
        let template = self.runtime_resource_text(
            "IDS_ERR_UNKNOWNCMD",
            "Unknown command: \"%s\" - type /help to get a list of valid commands",
        );
        let message = format_resource_string(template, &[&name]);
        if self.control_message_has_lobby() {
            self.append_lobby_command_error(message);
        } else {
            self.append_control_message_log(message, CONTROL_LOG_COLOR, None);
        }
    }

    fn submit_or_execute_running_control_set(
        &mut self,
        value_type: i32,
        data: i32,
    ) -> Result<(), EngineError> {
        let set = clonk_network::LegacyControlSet {
            value_type,
            data,
            by_client: 0,
        };
        if let Some(network) = self.network.as_ref() {
            let tick = self.local_control_submission_tick();
            let sync = self.running_control_prefers_sync();
            if let Err(error) = network.submit_decided_control_set(tick, set, sync) {
                tracing::error!(%error, value_type, data, "failed to submit chat control set");
            }
            return Ok(());
        }
        self.apply_ready_controls(
            self.local_control_submission_tick(),
            vec![NetworkControl::Set(set)],
        )
    }

    fn submit_or_execute_running_script(
        &mut self,
        script: clonk_engine::ScriptControlData,
    ) -> Result<(), EngineError> {
        let tick = self.local_control_submission_tick();
        if let Some(network) = self.network.as_ref() {
            let sync = self.running_control_prefers_sync();
            if let Err(error) = network.submit_decided_script_control(tick, script, sync) {
                tracing::error!(%error, "failed to submit chat script control");
            }
            return Ok(());
        }
        self.apply_ready_controls(tick, vec![NetworkControl::Script(script)])
    }

    fn submit_or_execute_editor_selection_script(
        &mut self,
        control: clonk_engine::EmMoveObjectControlData,
    ) -> Result<(), EngineError> {
        let tick = self.local_control_submission_tick();
        if let Some(network) = self.network.as_ref() {
            let sync = self.running_control_prefers_sync();
            if let Err(error) = network.submit_decided_em_move_object_control(tick, control, sync) {
                tracing::error!(%error, "failed to submit editor selection script");
            }
            return Ok(());
        }
        self.apply_ready_controls(tick, vec![NetworkControl::EmMoveObject(control)])
    }

    /// The landscape drawing tools' half of the same seam
    /// (`C4EditCursor::ApplyToolBrush`, `C4EditCursor.cpp:551-556`).
    ///
    /// Drawing is a *control* for the same reason moving an object is: the
    /// landscape is synchronized state, so every peer has to apply the same
    /// edit at the same tick.
    fn submit_or_execute_editor_draw_tool(
        &mut self,
        control: clonk_engine::EmDrawToolControlData,
    ) -> Result<(), EngineError> {
        let tick = self.local_control_submission_tick();
        if let Some(network) = self.network.as_ref() {
            let sync = self.running_control_prefers_sync();
            if let Err(error) = network.submit_decided_em_draw_tool_control(tick, control, sync) {
                tracing::error!(%error, "failed to submit an editor draw tool");
            }
            return Ok(());
        }
        self.apply_ready_controls(tick, vec![NetworkControl::EmDrawTool(control)])
    }

    /// `C4Game::DropDef`'s half of the same seam (`C4Game.cpp:1667`).
    ///
    /// Placing an object is a *control* for the same reason moving one is: it
    /// creates synchronized state, so every peer has to create it at the same
    /// tick and in the same order.
    fn submit_or_execute_editor_drop_definition(
        &mut self,
        control: clonk_engine::EmDropDefControlData,
    ) -> Result<(), EngineError> {
        let tick = self.local_control_submission_tick();
        if let Some(network) = self.network.as_ref() {
            let sync = self.running_control_prefers_sync();
            if let Err(error) = network.submit_decided_em_drop_def_control(tick, control, sync) {
                tracing::error!(%error, "failed to submit an editor definition drop");
            }
            return Ok(());
        }
        self.apply_ready_controls(tick, vec![NetworkControl::EmDropDef(control)])
    }

    /// `C4EditCursor::In`, called by the separate property-dialog script
    /// entry. Selection ownership stays with the edit cursor; this snapshots
    /// its live C++-ordered object numbers into one `EMMO_Script` packet.
    fn submit_editor_selection_script(
        &mut self,
        text: &str,
        selected_object_numbers: &[i32],
    ) -> Result<(), EngineError> {
        let Some(script) =
            clonk_engine::LegacyCString::from_bytes(clonk_script::c4_string_bytes(text))
        else {
            tracing::warn!("editor selection script contained an embedded NUL");
            return Ok(());
        };
        self.submit_or_execute_editor_selection_script(clonk_engine::EmMoveObjectControlData {
            action: clonk_engine::EMMO_SCRIPT,
            objects: selected_object_numbers.to_vec(),
            strictness: self.running_console_script_strictness(),
            script,
            by_client: if self.control_playback.is_some() {
                -1
            } else {
                0
            },
            ..Default::default()
        })
    }

    fn submit_or_execute_running_custom_command(
        &mut self,
        command: clonk_engine::CustomCommandControlData,
    ) -> Result<(), EngineError> {
        let tick = self.local_control_submission_tick();
        if let Some(network) = self.network.as_ref() {
            let sync = self.running_control_prefers_sync();
            if let Err(error) = network.submit_custom_command(tick, command, sync) {
                tracing::error!(%error, "failed to submit custom chat command");
            }
            return Ok(());
        }
        self.apply_ready_controls(tick, vec![NetworkControl::CustomCommand(command)])
    }

    fn request_control_message_attention(&mut self) -> bool {
        if self.window_active {
            return false;
        }
        self.control_messages.request_user_attention();
        true
    }

    fn execute_message_control(&mut self, control: MessageControlData) -> MessageControlOutcome {
        self.execute_message_control_with_sound_at(control, Instant::now(), |app, name| {
            app.play_control_message_sound(name)
        })
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
        self.play_global_sound_effect("SyncError");
        if let Some(local_client_id) = self
            .network
            .as_ref()
            .and_then(|network| i32::try_from(network.local_client_id()).ok())
        {
            // C4ControlSyncCheck::Execute clears C4Network2 on a live network
            // mismatch; C4Network2::Clear then invokes ChangeToLocal rather
            // than sending a graceful removal or aborting the round
            // (C4Control.cpp:469-519; C4Network2.cpp:746-789).
            self.report_league_disconnect(
                local_client_id,
                clonk_network::LeagueDisconnectReason::Desync,
            );
            self.engine.evaluate_network_round_results(
                clonk_engine::RoundResultsNetworkResult::NetworkError,
                Some(b"Network: Synchronization loss!".to_vec()),
            );
            self.snapshot.round_results = self.engine.snapshot().round_results;
            self.change_network_control_to_local(local_client_id);
        }
        self.status_text = "Network desync detected; disconnected from host".to_string();
    }

    fn ingame_moving_drag_active(&self) -> bool {
        self.mouse_state.is_some_and(|state| {
            state.motion.region_drag_started || state.motion.world_drag_started
        }) || self.ingame_right_mouse_state.is_some_and(|state| {
            state.motion.region_drag_started || state.motion.world_drag_started
        })
    }

    fn ingame_selection_drag_active(&self) -> bool {
        self.mouse_state
            .is_some_and(|state| state.motion.moved && state.motion.selection_frame)
            || self
                .ingame_right_mouse_state
                .is_some_and(|state| state.motion.moved && state.motion.selection_frame)
    }

    fn remove_local_control_assignment(&mut self, owner: i32) {
        let previous_mouse_owner = self.local_controls.mouse_owner();
        self.local_controls.remove(owner);
        let mouse_owner = self.local_controls.mouse_owner();
        if mouse_owner != previous_mouse_owner {
            self.reset_ingame_mouse_control();
        }
        self.mouse_control = mouse_owner.is_some();
    }

    fn ingame_construction_drag_active(&self) -> bool {
        self.construction_menu_drag
            .as_ref()
            .is_some_and(ConstructionMenuDrag::is_active)
    }

    fn ingame_captured_drag_active(&self) -> bool {
        self.ingame_moving_drag_active() || self.ingame_construction_drag_active()
    }

    fn running_classic_gui_is_active(&self, external_menu_shown: bool) -> bool {
        external_menu_shown
            || !self.running_dialog_stack.is_empty()
            || !self.runtime_default_dialog_order_snapshot().is_empty()
            || self.context_menu.is_some()
            || self.definition_selector.is_some()
            || self.game_option_input_dialog.is_some()
            || self.league_signup_dialog.is_some()
            || !self.message_dialogs.is_empty()
            || self.game_over_dialog.is_some()
            || self.network_chart_dialog.is_some()
            || self.runtime_client_list.is_some()
            || self.external_irc_dialog_visible
            || self.startup_options_advanced_dialog.is_some()
            || self.startup_player_properties_dialog.is_some()
            || self
                .network_start_wait
                .as_ref()
                .is_some_and(|wait| wait.visible)
            || self.object_menu.is_some()
    }

    /// Apply one C4MouseControl::UpdateScrolling step. Pointer movement calls
    /// this immediately; the successful simulation path calls it once more
    /// after every engine tick while the retained border state remains live.
    /// The return value reports an engine mutation that needs a fresh app
    /// snapshot; observer cameras are presentation-owned and return false.
    fn apply_ingame_edge_scroll(&mut self) -> Result<bool, EngineError> {
        if self.ingame_edge_scroll.is_none() {
            return Ok(false);
        }
        let Some((scroll, viewport)) = self.reevaluate_ingame_edge_scroll()? else {
            self.ingame_edge_scroll = None;
            return Ok(false);
        };
        self.ingame_edge_scroll = Some(scroll);
        self.perform_ingame_edge_scroll(scroll, viewport)
    }

    /// Native executes a synthetic Move on every Tick5 even when the previous
    /// move was interior or suppressed by a viewport region. Reevaluate the
    /// retained VpX/VpY so disappearing regions and resized layouts take
    /// effect without a new platform motion event.
    fn refresh_ingame_edge_scroll_tick5(&mut self) -> Result<bool, EngineError> {
        let Some((scroll, viewport)) = self.reevaluate_ingame_edge_scroll()? else {
            self.ingame_edge_scroll = None;
            return Ok(false);
        };
        self.ingame_edge_scroll = Some(scroll);
        self.perform_ingame_edge_scroll(scroll, viewport)
    }

    fn reevaluate_ingame_edge_scroll(
        &mut self,
    ) -> Result<Option<(ActiveViewportEdgeScroll, ActiveViewportProjection)>, EngineError> {
        let Some(retained) = self.ingame_viewport_mouse else {
            return Ok(None);
        };
        let Some(gui_point) = self.ingame_gui_pointer else {
            return Ok(None);
        };
        let routing_still_active = if retained.observer {
            self.local_controls.mouse_owner().is_none()
        } else {
            self.local_controls.mouse_owner() == Some(retained.owner)
        };
        if self.mode != AppMode::Running
            || !self.window_active
            || !routing_still_active
            || !self.message_dialogs.is_empty()
            || self.startup_player_properties_dialog.is_some()
            || self.definition_selector.is_some()
            || self.context_menu.is_some()
            || self.game_option_input_dialog.is_some()
            || self.game_over_dialog.is_some()
            || self.network_chart_pointer_capture
        {
            return Ok(None);
        }
        let construction_drag_active = self.ingame_construction_drag_active();
        if !construction_drag_active && self.handle_runtime_client_list_pointer_move(gui_point) {
            return Ok(None);
        }
        if !construction_drag_active && self.network_chart_contains_point(gui_point) {
            return Ok(None);
        }

        let viewport = self
            .graphics
            .active_viewport_projections()
            .into_iter()
            .find(|viewport| {
                viewport.index == retained.viewport_index
                    && if retained.observer {
                        viewport.is_no_owner_viewport
                    } else {
                        viewport.owner == retained.owner
                    }
            });
        let Some(viewport) = viewport else {
            return Ok(None);
        };
        let width = i32::try_from(viewport.rect.width).unwrap_or(i32::MAX);
        let height = i32::try_from(viewport.rect.height).unwrap_or(i32::MAX);
        let screen = GuiPoint::new(
            viewport.rect.x.saturating_add(retained.position.x) as f32,
            viewport.rect.y.saturating_add(retained.position.y) as f32,
        );
        // C4MouseControl::Execute repeats Move(VpX, VpY), which refreshes the
        // world-space pointer and drag motion from the camera's current
        // position before applying the next direct scroll step.
        let Some(pointer) = self
            .graphics
            .viewport_output_point_for_index(viewport.index, screen)
        else {
            return Ok(None);
        };
        let fog_blocked = self.ingame_pointer_fog_blocked(pointer);
        if let Some(state) = self.mouse_state.as_mut() {
            state.update_with_fog(pointer, fog_blocked);
        }
        if let Some(state) = self.ingame_right_mouse_state.as_mut() {
            state.update_with_fog(pointer, fog_blocked);
        }
        self.ingame_pointer = Some(pointer);
        self.update_ingame_drag_selection_kinds();
        self.refresh_ingame_mouse_help_region_caption(pointer);
        let Some(edge) =
            viewport_edge_scroll_at(retained.position.x, retained.position.y, width, height)
        else {
            return Ok(None);
        };
        let gui_target = !construction_drag_active
            && (self.scoreboard_close_pointer_capture
                || self.scoreboard_pointer_target(gui_point)?.is_some()
                || self.ingame_menu_pointer_target(gui_point).is_some()
                || self
                    .script_menu_pointer_target_for_owner(viewport.owner, gui_point)?
                    .is_some());
        let target_region = gui_target
            || self
                .ingame_viewport_region(viewport.owner, screen)
                .is_some();
        if target_region {
            return Ok(None);
        }

        Ok(Some((
            ActiveViewportEdgeScroll {
                viewport_index: viewport.index,
                owner: viewport.owner,
                observer: retained.observer,
                screen,
                edge,
            },
            viewport,
        )))
    }

    fn perform_ingame_edge_scroll(
        &mut self,
        scroll: ActiveViewportEdgeScroll,
        viewport: ActiveViewportProjection,
    ) -> Result<bool, EngineError> {
        if scroll.observer {
            for delta in scroll.edge.steps() {
                if !self
                    .graphics
                    .scroll_observer_viewport(scroll.viewport_index, delta)
                {
                    self.ingame_edge_scroll = None;
                    break;
                }
            }
            return Ok(false);
        }

        for delta in scroll.edge.steps() {
            self.engine.scroll_player_view(
                scroll.owner,
                delta,
                viewport.logical_width,
                viewport.logical_height,
                // C4Application::isFullScreen distinguishes the game UI from
                // console mode, not an OS fullscreen window. This app has no
                // console mode, so every running viewport gets the 40px margin.
                true,
            )?;
        }
        Ok(true)
    }

    fn cancel_ingame_selection_for_region(&mut self, cancel_left: bool, cancel_right: bool) {
        if cancel_left || cancel_right {
            self.ingame_dragged_objects.clear();
        }
        if cancel_left {
            if let Some(state) = self.mouse_state.as_mut() {
                state.motion.selection_frame = false;
                state.motion.selection_kind = IngameDragSelectionKind::Unknown;
                state.motion.selection_cancelled_by_region = true;
            }
        }
        if cancel_right {
            if let Some(state) = self.ingame_right_mouse_state.as_mut() {
                state.motion.selection_frame = false;
                state.motion.selection_kind = IngameDragSelectionKind::Unknown;
                state.motion.selection_cancelled_by_region = true;
            }
        }
    }

    fn ingame_drag_selection(
        &self,
        motion: IngameMouseState,
    ) -> Option<(IngameDragSelectionKind, Vec<ObjectId>)> {
        if !motion.moved || !motion.selection_frame || self.ingame_pointer_fog_blocked(motion.last)
        {
            return None;
        }
        let first = ingame_pointer_world_pixel(motion.start);
        let second = ingame_pointer_world_pixel(motion.selection_last);
        match motion.selection_kind {
            IngameDragSelectionKind::Crew => Some((
                IngameDragSelectionKind::Crew,
                self.engine
                    .mouse_drag_crew_in_rect(self.local_owner, first, second),
            )),
            IngameDragSelectionKind::Objects => Some((
                IngameDragSelectionKind::Objects,
                self.engine.mouse_drag_carryables_in_rect(first, second),
            )),
            IngameDragSelectionKind::Unknown => {
                let crew = self
                    .engine
                    .mouse_drag_crew_in_rect(self.local_owner, first, second);
                if !crew.is_empty() {
                    return Some((IngameDragSelectionKind::Crew, crew));
                }
                let objects = self.engine.mouse_drag_carryables_in_rect(first, second);
                if !objects.is_empty() {
                    Some((IngameDragSelectionKind::Objects, objects))
                } else {
                    Some((IngameDragSelectionKind::Unknown, Vec::new()))
                }
            }
        }
    }

    fn ingame_fog_allows_target(&self, pointer: ViewportPointer, target: ObjectId) -> bool {
        !self.ingame_pointer_fog_blocked(pointer)
            || self
                .snapshot
                .object(target)
                .is_some_and(|object| object.category & C4D_IGNORE_FOW != 0)
    }

    fn update_ingame_drag_selection_kinds(&mut self) {
        let left_selection = self
            .mouse_state
            .and_then(|state| self.ingame_drag_selection(state.motion));
        let right_selection = self
            .ingame_right_mouse_state
            .and_then(|state| self.ingame_drag_selection(state.motion));
        if let Some((kind, selection)) = left_selection {
            if let Some(state) = self.mouse_state.as_mut() {
                state.motion.selection_kind = kind;
            }
            self.ingame_dragged_objects = selection;
        }
        if let Some((kind, selection)) = right_selection {
            if let Some(state) = self.ingame_right_mouse_state.as_mut() {
                state.motion.selection_kind = kind;
            }
            self.ingame_dragged_objects = selection;
        }
    }

    /// The target copied into C4MouseControl::DownRegion for one grouped
    /// cursor-inventory cell. These regions sit above the world pick layer
    /// and retain the group's first object as their Target
    /// (C4Viewport.cpp:911-917; C4ObjectList.cpp:343-372).
    fn ingame_inventory_region_hit(&self, owner: i32, point: GuiPoint) -> Option<(ObjectId, Rect)> {
        let pointer = self.graphics.viewport_output_point_at(point)?;
        if pointer.owner != owner {
            return None;
        }
        let viewport = self.graphics.viewport_rect(pointer.owner)?;
        let cursor = self
            .snapshot
            .players
            .iter()
            .find(|player| player.id == pointer.owner)
            .and_then(|player| player.view_cursor.or(player.cursor))?;
        let cursor_definition = &self.snapshot.object(cursor)?.definition_id;
        if self.engine.definition_hide_hud_elements(cursor_definition)
            & clonk_engine::HIDE_HUD_ELEMENT_INVENTORY
            != 0
        {
            return None;
        }
        let inventory = collect_crew_inventory(
            &self.engine,
            &self.snapshot,
            cursor,
            self.graphics.advanced_renderer_config(),
        );
        let section =
            clonk_frontend::hud::inventory_region_index(viewport, point, inventory.len())?;
        let region =
            clonk_frontend::hud::inventory_region_rect(viewport, section, inventory.len())?;
        inventory.get(section).map(|item| (item.object_id, region))
    }

    fn ingame_inventory_region_target(&self, owner: i32, point: GuiPoint) -> Option<ObjectId> {
        self.ingame_inventory_region_hit(owner, point)
            .map(|(target, _)| target)
    }

    fn ingame_command_region_hit(&self, owner: i32, point: GuiPoint) -> Option<(u8, String, Rect)> {
        if !self.display_flags.show_commands
            || self.object_menu.is_some()
            || self.engine.cursor_object_menu(owner).is_some()
        {
            return None;
        }
        let pointer = self.graphics.viewport_output_point_at(point)?;
        if pointer.owner != owner {
            return None;
        }
        let cursor = self
            .snapshot
            .players
            .iter()
            .find(|player| player.id == pointer.owner)
            .and_then(|player| player.cursor)?;
        let viewport = self.graphics.viewport_rect(pointer.owner)?;
        let context = AppCommandContext {
            engine: &self.engine,
            bindings: &self.bindings,
            snapshot: &self.snapshot,
            resources: &self.startup_tooltip_resources,
        };
        let commands = draw_commands::build_cursor_commands(&self.snapshot, cursor, &context);
        let (index, rect) = clonk_frontend::hud::command_region_hit(viewport, point, &commands)?;
        commands
            .get(index)
            .map(|command| (command.com, command.caption.clone(), rect))
    }

    fn ingame_command_region_at(&self, owner: i32, point: GuiPoint) -> Option<u8> {
        self.ingame_command_region_hit(owner, point)
            .map(|(command, _, _)| command)
    }

    fn ingame_object_caption_name(&self, object: ObjectId) -> Option<String> {
        let snapshot = self.snapshot.object(object)?;
        let name = snapshot
            .custom_name
            .as_deref()
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .or_else(|| {
                self.engine
                    .crew_object_info(object)
                    .map(|info| info.name.clone())
            })
            .or_else(|| {
                self.engine
                    .definition_name(&snapshot.definition_id)
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| snapshot.definition_id.clone());
        Some(c4_presentation_text(&name))
    }

    fn active_ingame_moving_drag(&self) -> Option<(MouseDragSource, Vec<ObjectId>)> {
        let state = self
            .mouse_state
            .as_ref()
            .filter(|state| state.motion.region_drag_started || state.motion.world_drag_started)
            .or_else(|| {
                self.ingame_right_mouse_state.as_ref().filter(|state| {
                    state.motion.region_drag_started || state.motion.world_drag_started
                })
            })?;
        let target = state.down_target?;
        let mut selected =
            if state.motion.region_drag_started || self.ingame_dragged_objects.contains(&target) {
                self.ingame_dragged_objects.clone()
            } else {
                vec![target]
            };
        selected.retain(|object| {
            self.snapshot
                .object(*object)
                .is_some_and(|object| object.status != clonk_engine::ObjectStatus::Deleted)
        });
        let first = self.snapshot.object(*selected.first()?)?;
        let source = if first.ocf & clonk_engine::ocf::CARRYABLE != 0 {
            MouseDragSource::Carryable
        } else {
            MouseDragSource::Vehicle
        };
        Some((source, selected))
    }

    fn ingame_moving_drag_caption(
        &mut self,
        pointer: ViewportPointer,
    ) -> Option<(IngameMouseCursorKind, Option<String>)> {
        let (source, selected) = self.active_ingame_moving_drag()?;
        if self.ingame_pointer_fog_blocked(pointer) {
            return Some((IngameMouseCursorKind::Nothing, None));
        }
        let target = self
            .keyboard_modifiers
            .control_key()
            .then(|| {
                self.graphics.object_at_point_with_ocf(
                    &self.snapshot,
                    pointer.owner,
                    pointer.screen,
                    clonk_engine::ocf::CONTAINER,
                )
            })
            .flatten();
        let Some(target) = target else {
            let kind = match source {
                MouseDragSource::Carryable => match self
                    .engine
                    .mouse_drag_carryable_cursor(pointer.owner, ingame_pointer_world_pixel(pointer))
                {
                    Some(MouseDragCarryableCursor::Drop) => IngameMouseCursorKind::Drop,
                    Some(MouseDragCarryableCursor::Throw {
                        direction: -1,
                        landing,
                    }) => IngameMouseCursorKind::ThrowLeft(landing),
                    Some(MouseDragCarryableCursor::Throw { landing, .. }) => {
                        IngameMouseCursorKind::ThrowRight(landing)
                    }
                    _ => IngameMouseCursorKind::Carryable,
                },
                MouseDragSource::Vehicle => IngameMouseCursorKind::Vehicle,
            };
            return Some((kind, None));
        };
        let target_name = self.ingame_object_caption_name(target)?;
        let selected_name = if selected.len() > 1 {
            let noun = match source {
                MouseDragSource::Carryable => self
                    .startup_tooltip_resources
                    .get("IDS_CON_ITEMS")
                    .map(String::as_str)
                    .unwrap_or("items"),
                MouseDragSource::Vehicle => self
                    .startup_tooltip_resources
                    .get("IDS_CON_VEHICLES")
                    .map(String::as_str)
                    .unwrap_or("Vehicles"),
            };
            format!("{} {noun}", selected.len())
        } else {
            self.ingame_object_caption_name(selected[0])?
        };
        let (kind, key, fallback) = match source {
            MouseDragSource::Carryable => {
                (IngameMouseCursorKind::Put, "IDS_CON_PUT", "Drop %s in %s")
            }
            MouseDragSource::Vehicle => (
                IngameMouseCursorKind::VehiclePut,
                "IDS_CON_VEHICLEPUT",
                "Push %s into %s.",
            ),
        };
        Some((
            kind,
            Some(self.localized_ingame_mouse_caption(
                key,
                fallback,
                &[selected_name.as_str(), target_name.as_str()],
                false,
            )),
        ))
    }

    fn advance_ingame_time_on_target(&mut self, kind: IngameMouseCursorKind) -> bool {
        let state = &mut self.ingame_mouse_caption;
        if state.cursor == kind {
            state.time_on_target = state.time_on_target.saturating_add(1);
            state.time_on_target >= INGAME_MOUSE_CAPTION_DELAY && state.keep_caption == 0
        } else {
            state.cursor = kind;
            state.time_on_target = 0;
            false
        }
    }

    fn update_scoreboard_title_drag(&mut self, point: GuiPoint) -> bool {
        let Some(drag) = self.scoreboard_runtime.title_drag else {
            return false;
        };
        let Some(presentation) = self.scoreboard_runtime.presentation.as_mut() else {
            self.scoreboard_runtime.title_drag = None;
            return false;
        };
        let target_x = drag
            .origin
            .0
            .saturating_add((point.x - drag.pointer.x).round() as i32);
        let target_y = drag
            .origin
            .1
            .saturating_add((point.y - drag.pointer.y).round() as i32);
        let bounds = presentation.layout().bounds;
        presentation.layout_mut().translate(
            target_x.saturating_sub(bounds.x),
            target_y.saturating_sub(bounds.y),
        );
        true
    }

    fn finish_ingame_moved_drag(
        &mut self,
        drag: IngameButtonMouseState,
        expand_region_group: bool,
    ) -> Result<bool, EngineError> {
        let owner = drag.motion.start.owner;
        // Both ButtonUp paths refresh TargetRegion before evaluating either
        // Selecting or Moving. A region cursor has no object-command case.
        if self
            .ingame_viewport_region(drag.motion.start.owner, drag.motion.last.screen)
            .is_some()
        {
            self.ingame_dragged_objects.clear();
            return Ok(true);
        }
        if drag.motion.selection_frame {
            // C4MouseControl locks an unknown landscape frame to the first
            // type found. Crew frames queue CID_PlrSelect and clear; object
            // frames retain their local Selection for a subsequent moving
            // drag (C4MouseControl.cpp:795-817,909-968,1158-1169).
            let selected = self.ingame_selection_candidates(drag.motion);
            match drag.motion.selection_kind {
                IngameDragSelectionKind::Crew => {
                    self.ingame_dragged_objects.clear();
                    self.submit_or_execute_player_select(PlayerSelectControlData {
                        player: owner,
                        objects: selected
                            .into_iter()
                            .map(|object| object.as_u64() as i32)
                            .collect(),
                        by_client: -1,
                    })?;
                    self.snapshot = self.engine.snapshot();
                    self.refresh_object_menu();
                    self.refresh_focus();
                }
                IngameDragSelectionKind::Objects => {
                    self.ingame_dragged_objects = selected;
                }
                IngameDragSelectionKind::Unknown => {
                    self.ingame_dragged_objects.clear();
                }
            }
            return Ok(true);
        }
        let Some(target) = drag.down_target else {
            return Ok(false);
        };
        let source = if drag.down_region {
            self.engine.mouse_region_drag_source(target)
        } else {
            self.engine.mouse_world_drag_source(
                owner,
                target,
                ingame_pointer_world_pixel(drag.motion.start),
            )
        };
        let region_selection = drag.down_region.then(|| {
            self.engine
                .mouse_region_drag_objects(target, expand_region_group)
        });
        match source {
            Some(MouseDragSource::Carryable) => {
                self.finish_ingame_carryable_drag(drag, target, region_selection)?;
                Ok(true)
            }
            Some(MouseDragSource::Vehicle) => {
                self.finish_ingame_vehicle_drag(drag, target, region_selection)?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    fn finish_ingame_noop_drag(
        &mut self,
        motion: IngameMouseState,
        selected: usize,
    ) -> Result<(), EngineError> {
        self.ingame_dragged_objects.clear();
        if motion.last.owner != self.local_owner {
            return Ok(());
        }
        let position = ingame_pointer_world_pixel(motion.last);
        let shift_append = self.keyboard_modifiers.shift_key();
        let mut add_mode = 1;
        for _ in 0..selected {
            self.submit_or_execute_player_command(PlayerCommandControlData {
                player: self.local_owner,
                command: 0,
                x: position.x,
                y: position.y,
                target: 0,
                target2: 0,
                data: 0,
                add_mode: add_mode | if shift_append { 4 } else { 0 },
                by_client: -1,
            })?;
            add_mode = 4;
        }
        self.snapshot = self.engine.snapshot();
        self.refresh_object_menu();
        self.refresh_focus();
        Ok(())
    }

    fn finish_ingame_region_drag(
        &mut self,
        motion: IngameMouseState,
        selected: Vec<ObjectId>,
        cursor: Option<IngameRegionDragCursor>,
    ) -> Result<(), EngineError> {
        if cursor.is_none() {
            return self.finish_ingame_noop_drag(motion, selected.len());
        }
        self.ingame_dragged_objects.clear();
        if motion.last.owner != self.local_owner {
            return Ok(());
        }
        let position = ingame_pointer_world_pixel(motion.last);
        let shift_append = self.keyboard_modifiers.shift_key();
        let mut add_mode = 1;
        for object in selected {
            let (command, x, y, target, target2) = match cursor {
                Some(IngameRegionDragCursor::Drop) => (
                    CommandId::Drop as i32,
                    position.x,
                    position.y,
                    object.as_u64() as i32,
                    0,
                ),
                Some(IngameRegionDragCursor::Throw) => (
                    CommandId::Throw as i32,
                    position.x,
                    position.y,
                    object.as_u64() as i32,
                    0,
                ),
                Some(IngameRegionDragCursor::Put(container)) => (
                    CommandId::Put as i32,
                    0,
                    0,
                    self.engine
                        .object_snapshot(container)
                        .filter(|target| target.status != clonk_engine::ObjectStatus::Deleted)
                        .map_or(0, |_| container.as_u64() as i32),
                    object.as_u64() as i32,
                ),
                Some(IngameRegionDragCursor::Vehicle) => (
                    CommandId::PushTo as i32,
                    position.x,
                    position.y,
                    object.as_u64() as i32,
                    0,
                ),
                Some(IngameRegionDragCursor::VehiclePut(container)) => (
                    CommandId::PushTo as i32,
                    position.x,
                    position.y,
                    object.as_u64() as i32,
                    self.engine
                        .object_snapshot(container)
                        .filter(|target| target.status != clonk_engine::ObjectStatus::Deleted)
                        .map_or(0, |_| container.as_u64() as i32),
                ),
                None => unreachable!("no-command drag handled above"),
            };
            self.submit_or_execute_player_command(PlayerCommandControlData {
                player: self.local_owner,
                command,
                x,
                y,
                target,
                target2,
                data: 0,
                add_mode: add_mode | if shift_append { 4 } else { 0 },
                by_client: -1,
            })?;
            add_mode = 4;
        }
        self.snapshot = self.engine.snapshot();
        self.refresh_object_menu();
        self.refresh_focus();
        Ok(())
    }

    fn finish_ingame_carryable_drag(
        &mut self,
        drag: IngameButtonMouseState,
        down_target: ObjectId,
        region_selection: Option<Vec<ObjectId>>,
    ) -> Result<(), EngineError> {
        let selected = if let Some(selected) = region_selection {
            self.ingame_dragged_objects.clear();
            selected
        } else if self.ingame_dragged_objects.contains(&down_target) {
            std::mem::take(&mut self.ingame_dragged_objects)
        } else {
            self.ingame_dragged_objects.clear();
            vec![down_target]
        };
        if drag.motion.last.owner != self.local_owner {
            return Ok(());
        }
        if self.ingame_pointer_fog_blocked(drag.motion.last) {
            return self.finish_ingame_noop_drag(drag.motion, selected.len());
        }
        let position = ingame_pointer_world_pixel(drag.motion.last);
        let put_target = self
            .keyboard_modifiers
            .control_key()
            .then(|| {
                self.graphics.object_at_point_with_ocf(
                    &self.snapshot,
                    self.local_owner,
                    drag.motion.last.screen,
                    clonk_engine::ocf::CONTAINER,
                )
            })
            .flatten();
        let command = if put_target.is_none() {
            self.engine
                .mouse_drag_carryable_command(self.local_owner, position)
        } else {
            None
        };
        if put_target.is_none() && command.is_none() {
            return Ok(());
        }
        self.show_startup_hint = false;
        let mut add_mode = 1;
        let shift_append = self.keyboard_modifiers.shift_key();
        for object in selected {
            let (command, x, y, target, target2) = if let Some(container) = put_target {
                (
                    CommandId::Put,
                    0,
                    0,
                    container.as_u64() as i32,
                    object.as_u64() as i32,
                )
            } else if let Some(command) = command {
                (command, position.x, position.y, object.as_u64() as i32, 0)
            } else {
                continue;
            };
            self.submit_or_execute_player_command(PlayerCommandControlData {
                player: self.local_owner,
                command: command as i32,
                x,
                y,
                target,
                target2,
                data: 0,
                add_mode: add_mode | if shift_append { 4 } else { 0 },
                by_client: -1,
            })?;
            add_mode = 4;
        }
        self.snapshot = self.engine.snapshot();
        self.refresh_object_menu();
        self.refresh_focus();
        Ok(())
    }

    fn finish_ingame_vehicle_drag(
        &mut self,
        drag: IngameButtonMouseState,
        down_target: ObjectId,
        region_selection: Option<Vec<ObjectId>>,
    ) -> Result<(), EngineError> {
        self.ingame_dragged_objects.clear();
        let selected = region_selection.unwrap_or_else(|| vec![down_target]);
        if drag.motion.last.owner != self.local_owner {
            return Ok(());
        }
        if self.ingame_pointer_fog_blocked(drag.motion.last) {
            return self.finish_ingame_noop_drag(drag.motion, selected.len());
        }
        let position = ingame_pointer_world_pixel(drag.motion.last);
        let put_target = self
            .keyboard_modifiers
            .control_key()
            .then(|| {
                self.graphics.object_at_point_with_ocf(
                    &self.snapshot,
                    self.local_owner,
                    drag.motion.last.screen,
                    clonk_engine::ocf::CONTAINER,
                )
            })
            .flatten();
        self.show_startup_hint = false;
        let mut add_mode = 1;
        let shift_append = self.keyboard_modifiers.shift_key();
        for vehicle in selected {
            self.submit_or_execute_player_command(PlayerCommandControlData {
                player: self.local_owner,
                command: CommandId::PushTo as i32,
                x: position.x,
                y: position.y,
                target: vehicle.as_u64() as i32,
                target2: put_target.map_or(0, |target| target.as_u64() as i32),
                data: 0,
                add_mode: add_mode | if shift_append { 4 } else { 0 },
                by_client: -1,
            })?;
            add_mode = 4;
        }
        self.snapshot = self.engine.snapshot();
        self.refresh_object_menu();
        self.refresh_focus();
        Ok(())
    }

    fn dispatch_ingame_region_control(
        &mut self,
        owner: i32,
        region: IngameViewportRegion,
        release: bool,
    ) -> Result<(), EngineError> {
        if let IngameViewportRegion::ViewportButton(button) = region {
            // These controls are C4MouseControl-local. In particular, raw
            // COM_PlayerMenu would bypass the local menu handler and could
            // leak into the synchronized queue in a network game.
            if release {
                return Ok(());
            }
            return match button {
                clonk_frontend::hud::ViewportButton::Help => {
                    self.ingame_mouse_help = true;
                    Ok(())
                }
                clonk_frontend::hud::ViewportButton::PlayerMenu => {
                    self.activate_ingame_main_menu_for_player(owner)
                }
                clonk_frontend::hud::ViewportButton::Chat => self.show_external_irc_dialog(),
            };
        }
        let (command, data) = region.control();
        self.dispatch_control_event_for_local_player(
            owner,
            ControlEvent::RawPlayerControl {
                command: if release {
                    command + clonk_engine::COM_RELEASE_OFFSET
                } else {
                    command
                },
                data,
            },
        )
    }

    fn cancel_underlying_interaction(&mut self) {
        self.pointer_left_unchecked();
        if self.game_over_dialog.is_some() {
            return;
        }
        if let Some(dialog) = self.league_signup_dialog.as_mut() {
            dialog.controller.cancel_interaction();
            self.league_signup_pointer_capture = false;
            return;
        }
        if let Some(dialog) = self.game_option_input_dialog.as_mut() {
            dialog.controller.cancel_interaction();
            return;
        }
        if let Some(controller) = self.definition_selector.as_mut() {
            controller.cancel_interaction();
            return;
        }
        if matches!(self.mode, AppMode::Menu) {
            match self.startup_view {
                StartupView::NetworkGame => {
                    if let Some(dialog) = self.startup_network_dialog.as_mut() {
                        dialog.cancel_interaction();
                    }
                }
                StartupView::PlayerSelection => {
                    if let Some(dialog) = self.startup_player_dialog.as_mut() {
                        dialog.cancel_interaction();
                    }
                }
                StartupView::ScenarioBrowser => {
                    self.scenario_game_options.cancel_interaction();
                    self.menu_state.menu().cancel_interaction();
                }
                StartupView::NetworkLobby => {
                    if !self.cancel_classic_lobby_interaction() {
                        self.menu_state.menu().cancel_interaction();
                    }
                }
                StartupView::MainMenu | StartupView::Options | StartupView::About => {}
            }
        }
    }

    /// C4StartupScenSelDlg::DoBack(true) (cpp:1705-1725): backtrace the
    /// folder stack first; from the root, return to the main screen.
    fn configure_current_folder_map(&mut self) {
        // SetFolderData/UpdateList replace Book/Map child elements without
        // replacing the outer dialog. Mirror native element destruction by
        // dropping hover ownership before resolving the rebuilt hierarchy.
        self.startup_tooltip.pointer_left();
        let width = self.graphics.surface().width() as i32;
        let height = self.graphics.surface().height() as i32;
        let languages = self
            .app_paths
            .as_ref()
            .and_then(|paths| classic_runtime_language_sequence(paths).ok())
            .filter(|languages| !languages.is_empty())
            .unwrap_or_else(|| vec!["US".to_string()]);
        self.menu_state.configure_current_folder_map(
            self.show_folder_maps,
            width,
            height,
            &self.mission_access,
            &languages,
        );
    }

    fn reload_application_language_resources(&mut self) -> Result<String> {
        let bytes_table = load_runtime_language_bytes_table(self.app_paths.as_ref())?;
        let table = load_runtime_language_table(self.app_paths.as_ref())?;
        self.save_description_language_table = Some(bytes_table);
        self.save_description_language = materialized_save_description_language(
            &load_native_config_bytes(self.app_paths.as_ref()),
        );
        let generated_team_name_template = generated_team_name_template(&table);
        self.generated_team_name_template = generated_team_name_template.clone();
        if let Some(assignment) = self.network_team_assignment.as_mut() {
            assignment.set_generated_team_name_template(generated_team_name_template);
        }
        let charset = table
            .entries
            .get("IDS_LANG_CHARSET")
            .cloned()
            .unwrap_or_else(|| "[Undefined: IDS_LANG_CHARSET]".to_string());
        let (needed_material_need, needed_material_none) = needed_material_resource_strings(&table);
        let object_no_dig = object_no_dig_resource_string(&table);
        let definition_overload_template = definition_overload_resource_string(&table);
        self.needed_material_need = needed_material_need.clone();
        self.needed_material_none = needed_material_none.clone();
        self.object_no_dig = object_no_dig.clone();
        self.loaded_default_rank_names = Some(default_rank_resource_names(&table));
        self.startup_tooltip_resources = table.entries.clone();
        self.runtime_language_charset = table.charset;
        self.engine
            .set_needed_material_resource_strings(needed_material_need, needed_material_none);
        clonk_engine::scenario::verbose_loading::set_definition_overload_template(
            definition_overload_template,
        );
        self.engine.set_object_no_dig_resource_string(object_no_dig);
        let construction_check_feedback = construction_check_resource_strings(&table);
        self.construction_check_feedback = construction_check_feedback.clone();
        {
            let [undefined, no_construction, no_room, no_level, no_other] =
                construction_check_feedback;
            self.engine.set_construction_check_resource_strings(
                undefined,
                no_construction,
                no_room,
                no_level,
                no_other,
            );
        }

        // Dropping the cache is enough; the accessor rebuilds it against the
        // new table and the live KeyConfig.
        self.runtime_help_text_cache = OnceLock::new();
        self.runtime_flash_resources_cache = OnceLock::new();
        let _ = self
            .runtime_flash_resources_cache
            .set(Ok(build_runtime_flash_resources(&table)));
        Ok(charset)
    }

    fn refresh_participants_label(&mut self) {
        // An unflushed `General.Participants` wins over the file, so a
        // concurrent writer cannot change what the label shows — C++ reads its
        // in-memory Config (C4StartupMainDlg.cpp:174-200).
        let pending = self
            .deferred_config
            .get("General", "Participants")
            .map(str::to_owned);
        let label = participants_label_with_pending(self.app_paths.as_ref(), pending.as_deref());
        self.main_menu_state.update_participants_label(label);
    }

    /// Ask the event loop to unwind, recording which exit ran so the shutdown
    /// banner can name it. Several of these quit with no dialog and no other
    /// log line, so `reason` is all a bug report has to distinguish a
    /// deliberate quit from the process being destroyed (clonk-org/clonk-rs#40).
    fn request_exit(&mut self, reason: &'static str) {
        self.exit_requested = true;
        self.exit_reason = Some(reason);
        // `CStdApp::Quit` only latches a flag too (src/StdAppUnix.cpp:256-259),
        // but the unwind behind it always reaches `C4Application::Clear` →
        // `Game.Clear()` → `Network.Clear()` → `LeagueEnd(); DeinitLeague();`
        // (src/C4Application.cpp:304-332; src/C4Game.cpp:581;
        // src/C4Network2.cpp:746-763), so a native host never leaves its game
        // registered behind it. The port has no such unwind — nothing drops
        // `GameApp` — and `Event::LoopExiting` cannot stand in for one, because
        // on macOS it is dispatched from inside `applicationWillTerminate:`.
        // The latch is the last point that still runs on an ordinary loop turn,
        // where blocking on the worker is safe.
        //
        // A quit against a dead league server does block: `finish_league_runtime`
        // retries a failed send up to ten times, each bounded by the 20-second
        // `C4HTTPQueryTimeout` (`clonk_network::LEAGUE_HTTP_TIMEOUT`). Native
        // pays the same worst case at the same point — `LeagueEnd`'s
        // `MAX_RETRIES = 10` loop `continue`s on a failed reply once
        // `C4Game::Clear` has deleted `pGUI` (src/C4Network2.cpp:2513-2579;
        // src/C4Game.cpp:576-581) — and every other teardown here already
        // carries it, so this adds no new class of stall.
        self.clear_live_network_session();
    }

    fn take_exit_request(&mut self) -> bool {
        if self.exit_requested {
            self.exit_requested = false;
            true
        } else {
            false
        }
    }

    fn take_user_attention_request(&mut self) -> bool {
        self.control_messages.take_user_attention_request()
    }

    fn take_desktop_notification(&mut self) -> Option<DesktopNotification> {
        self.pending_desktop_notifications.pop_front()
    }

    /// Shared `pVP->Init(target, true)` path for observer selection previews
    /// and Enter. The durable `fIsNoOwnerViewport` identity is retained while
    /// its temporary displayed player changes.
    fn apply_observer_target(&mut self, target: ObserverTarget) -> bool {
        let player = match target {
            ObserverTarget::Free => OWNER_NONE,
            ObserverTarget::Player(player) if self.engine.player(player).is_some() => player,
            ObserverTarget::Player(_) => return false,
        };
        let Some(index) = self.observer_viewport_index() else {
            return false;
        };
        self.set_physical_view_target(index, player)
    }

    /// Temporary `C4Viewport::Init`: mutate one physical viewport in place
    /// while retaining its stable camera identity and ownerless bit.
    fn set_physical_view_target(&mut self, index: usize, player: i32) -> bool {
        let preserved_zoom = self.physical_viewports.get(index).and_then(|viewport| {
            viewport
                .uses_live_player_presentation
                .then(|| self.engine.player(viewport.camera_identity_owner))
                .flatten()
                .map(|source| {
                    source
                        .viewports()
                        .first()
                        .map_or(viewport.preserved_zoom, |viewport| viewport.zoom)
                })
        });
        let Some(viewport) = self.physical_viewports.get_mut(index) else {
            return false;
        };
        if let Some(zoom) = preserved_zoom {
            viewport.preserved_zoom = zoom;
        }
        viewport.displayed_player = player;
        viewport.uses_live_player_presentation = false;
        if index == 0 {
            self.film_view_player = Some(player);
        }
        self.physical_viewports_authoritative = true;
        if player != OWNER_NONE {
            self.runtime_flash_message = None;
        }
        self.update_film_viewport_availability();
        true
    }

    fn fail_pending_runtime_dynamic_request(&mut self, detail: String) {
        let Some(pending) = self.pending_runtime_dynamic_request.take() else {
            return;
        };
        tracing::error!(
            error = %detail,
            clients = ?pending.client_ids,
            "failed to create synchronized runtime JoinData"
        );
        let reason = self.runtime_resource_text(
            "IDS_ERR_ERRORWHILECREATINGJOINDAT",
            "Error while creating join data",
        );
        let reason =
            LegacyCString::from_bytes(clonk_script::c4_string_bytes(&reason)).unwrap_or_default();
        let expected = pending.client_ids.len();
        match self
            .network
            .as_ref()
            .ok_or_else(|| anyhow!("network manager is unavailable"))
            .and_then(|network| network.fail_pending_join_data(reason))
        {
            Ok(removed) if removed < expected => tracing::warn!(
                removed,
                expected,
                clients = ?pending.client_ids,
                "fewer pending JoinData clients were removed than requested"
            ),
            Ok(removed) => tracing::debug!(
                removed,
                clients = ?pending.client_ids,
                "removed clients waiting for failed runtime JoinData"
            ),
            Err(error) => tracing::error!(
                %error,
                clients = ?pending.client_ids,
                "failed to evict clients waiting for runtime JoinData"
            ),
        }
    }

    /// Execute the six C4ControlSet value types in packet order. Native
    /// `HostControl` means exactly author 0; DisableDebug is the deliberate
    /// exception and may be sent by any client.
    fn execute_control_set(&mut self, set: clonk_network::LegacyControlSet) {
        if set.by_client != 0 && set.value_type != 1 {
            return;
        }
        if set.value_type == 2 && self.network_is_league {
            self.append_control_message_log(
                "/set maxplayer disabled in league!".to_string(),
                CONTROL_LOG_COLOR,
                None,
            );
            self.play_ui_sound("Error");
            return;
        }

        let mut host_snapshot_changed = false;
        let runtime_network_role = self.runtime_network_role();
        let executes_control_host_team_logic =
            matches!(runtime_network_role, RuntimeNetworkRole::Host)
                || (matches!(runtime_network_role, RuntimeNetworkRole::Offline)
                    && self.engine.is_control_host());
        match set.value_type {
            // C4CVT_ControlRate
            0 => {
                let control_rate = self.network_control_clock.as_mut().map_or_else(
                    || {
                        self.engine
                            .control_rate()
                            .saturating_add(set.data)
                            .clamp(1, 20)
                    },
                    |clock| clock.adjust_control_rate(set.data),
                );
                self.engine.set_control_rate(control_rate);
                if let Some(join_data) = self.pending_network_join_data.as_mut() {
                    join_data.parameters.control_rate = control_rate;
                }
                if let Some(snapshot) = self.host_join_snapshot.as_mut() {
                    snapshot.parameters.control_rate = control_rate;
                    host_snapshot_changed = true;
                }
                if matches!(runtime_network_role, RuntimeNetworkRole::Host) {
                    self.persist_game_option_value(
                        "Network",
                        "ControlRate",
                        control_rate.to_string(),
                    );
                }
                self.refresh_classic_lobby_options(false);
            }
            // C4CVT_DisableDebug has no HostControl gate.
            1 => {
                let debug_was_enabled = self.engine.debug_mode();
                self.engine.disable_debug();
                if debug_was_enabled {
                    self.graphics
                        .set_debug_draw_flags(clonk_frontend::DebugDrawFlags::default());
                }
                if let Some(prepared) = self
                    .loading_state
                    .as_mut()
                    .and_then(|loading| loading.prepared_go.as_mut())
                {
                    prepared.allow_debug = false;
                }
                if let Some(join_data) = self.pending_network_join_data.as_mut() {
                    join_data.parameters.allow_debug = false;
                }
                if let Some(snapshot) = self.host_join_snapshot.as_mut() {
                    snapshot.parameters.allow_debug = false;
                    host_snapshot_changed = true;
                }
            }
            // C4CVT_MaxPlayer
            2 => {
                self.network_max_players = usize::try_from(set.data).unwrap_or(0);
                self.engine.set_max_players(set.data);
                if let Some(join_data) = self.pending_network_join_data.as_mut() {
                    join_data.parameters.max_players = set.data;
                }
                if let Some(snapshot) = self.host_join_snapshot.as_mut() {
                    snapshot.parameters.max_players = set.data;
                    host_snapshot_changed = true;
                }
            }
            // C4CVT_TeamDistribution
            3 => {
                let host_team_update = if executes_control_host_team_logic {
                    let has_or_will_have_lobby = self.has_or_will_have_network_lobby();
                    let Some(team_assignment) = self.network_team_assignment.as_mut() else {
                        let detail = "prepared host team state is unavailable for TeamDistribution";
                        tracing::error!(detail, "cannot execute exact TeamDistribution control");
                        self.status_text = detail.to_string();
                        return;
                    };
                    let updates = match team_assignment.set_distribution(
                        &mut self.control_player_infos,
                        set.data,
                        has_or_will_have_lobby,
                    ) {
                        Ok(updates) => updates,
                        Err(NetworkTeamControlError::InvalidDistribution(_)) => return,
                        Err(error) => {
                            tracing::error!(%error, "cannot execute exact TeamDistribution control");
                            self.status_text = format!(
                                "TeamDistribution is unavailable without native team generation: {error}"
                            );
                            return;
                        }
                    };
                    Some((team_assignment.teams().clone(), updates))
                } else {
                    None
                };
                if !self.engine.set_team_distribution(set.data) {
                    return;
                }
                if let Some(prepared) = self
                    .loading_state
                    .as_mut()
                    .and_then(|loading| loading.prepared_go.as_mut())
                {
                    prepared.team_configuration.distribution = set.data;
                }
                if let Some(join_data) = self.pending_network_join_data.as_mut() {
                    join_data.parameters.teams.team_distribution = set.data as u8;
                }
                if let Some((metadata, updates)) = host_team_update {
                    let runtime_teams = runtime_teams_from_initial_metadata(&metadata);
                    let team_snapshot = clonk_network::join_team_list_snapshot(metadata);
                    self.engine.set_teams(runtime_teams.clone());
                    if let Some(prepared) = self
                        .loading_state
                        .as_mut()
                        .and_then(|loading| loading.prepared_go.as_mut())
                    {
                        prepared.team_registry = runtime_teams;
                    }
                    if let Some(join_data) = self.pending_network_join_data.as_mut() {
                        join_data.parameters.teams = team_snapshot.clone();
                    }
                    if let Some(snapshot) = self.host_join_snapshot.as_mut() {
                        snapshot.parameters.teams = team_snapshot;
                        host_snapshot_changed = true;
                    }
                    host_snapshot_changed |= self.refresh_current_host_player_infos();
                    if let Some(network) = self.network.as_ref() {
                        for update in updates {
                            if let Err(error) = network.broadcast_player_info(update) {
                                tracing::error!(%error, "failed to broadcast TeamDistribution PlayerInfo update");
                            }
                        }
                    }
                } else if let Some(snapshot) = self.host_join_snapshot.as_mut() {
                    snapshot.parameters.teams.team_distribution = set.data as u8;
                    host_snapshot_changed = true;
                }
            }
            // C4CVT_TeamColors
            4 => {
                let enabled = set.data != 0;
                let restore_players = self
                    .host_join_snapshot
                    .as_ref()
                    .map(|snapshot| {
                        snapshot
                            .parameters
                            .restore_player_infos
                            .clients
                            .iter()
                            .flat_map(|client| client.players.iter().cloned())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let host_team_update = if executes_control_host_team_logic {
                    let alternate_colors = &self.host_local_alternate_colors_by_resource;
                    let local_player_info_ids = &self.host_local_player_info_ids;
                    let Some(team_assignment) = self.network_team_assignment.as_mut() else {
                        let detail = "prepared host team state is unavailable for TeamColors";
                        tracing::error!(detail, "cannot execute exact TeamColors control");
                        self.status_text = detail.to_string();
                        return;
                    };
                    if team_assignment.teams().team_colors == enabled {
                        return;
                    }
                    let updates = match team_assignment.set_team_colors_with_alternate_colors(
                        &mut self.control_player_infos,
                        enabled,
                        &restore_players,
                        |player| {
                            host_runtime_alternate_color(
                                alternate_colors,
                                local_player_info_ids,
                                player,
                            )
                        },
                    ) {
                        Ok(updates) => updates,
                        Err(error) => {
                            tracing::error!(%error, "cannot execute exact TeamColors control");
                            self.status_text = format!(
                                "TeamColors needs unavailable native color-conflict state: {error}"
                            );
                            return;
                        }
                    };
                    Some((team_assignment.teams().clone(), updates))
                } else {
                    None
                };
                self.engine.set_team_colors(enabled);
                if let Some(prepared) = self
                    .loading_state
                    .as_mut()
                    .and_then(|loading| loading.prepared_go.as_mut())
                {
                    prepared.team_configuration.team_colors = enabled;
                }
                if let Some(join_data) = self.pending_network_join_data.as_mut() {
                    join_data.parameters.teams.team_colors = u8::from(enabled);
                }
                if let Some((metadata, updates)) = host_team_update {
                    let runtime_teams = runtime_teams_from_initial_metadata(&metadata);
                    let team_snapshot = clonk_network::join_team_list_snapshot(metadata);
                    self.engine.set_teams(runtime_teams.clone());
                    if let Some(prepared) = self
                        .loading_state
                        .as_mut()
                        .and_then(|loading| loading.prepared_go.as_mut())
                    {
                        prepared.team_registry = runtime_teams;
                    }
                    if let Some(join_data) = self.pending_network_join_data.as_mut() {
                        join_data.parameters.teams = team_snapshot.clone();
                    }
                    if let Some(snapshot) = self.host_join_snapshot.as_mut() {
                        snapshot.parameters.teams = team_snapshot;
                        host_snapshot_changed = true;
                    }
                    host_snapshot_changed |= self.refresh_current_host_player_infos();
                    if let Some(network) = self.network.as_ref() {
                        for update in updates {
                            if let Err(error) = network.broadcast_player_info(update) {
                                tracing::error!(%error, "failed to broadcast TeamColors PlayerInfo update");
                            }
                        }
                    }
                } else if let Some(snapshot) = self.host_join_snapshot.as_mut() {
                    snapshot.parameters.teams.team_colors = u8::from(enabled);
                    host_snapshot_changed = true;
                }
            }
            // C4CVT_FairCrew. Native clears every definition's cached
            // projection for every accepted control, even if the parameters
            // are unchanged.
            5 => {
                let prepared_forced = self
                    .loading_state
                    .as_ref()
                    .and_then(|loading| loading.prepared_go.as_ref())
                    .is_some_and(|prepared| prepared.fair_crew_forced);
                let pending_forced = self
                    .pending_network_join_data
                    .as_ref()
                    .is_some_and(|join_data| join_data.parameters.fair_crew_forced);
                let host_forced = self
                    .host_join_snapshot
                    .as_ref()
                    .is_some_and(|snapshot| snapshot.parameters.fair_crew_forced);
                if self.engine.fair_crew_forced()
                    || prepared_forced
                    || pending_forced
                    || host_forced
                {
                    return;
                }
                let (use_fair_crew, fair_crew_strength) = if set.data < 0 {
                    (false, 0)
                } else {
                    (true, set.data)
                };
                self.engine.set_use_fair_crew(use_fair_crew);
                self.engine.set_fair_crew_strength(fair_crew_strength);
                if self.mode == AppMode::Running {
                    self.engine.clear_fair_crew_physicals();
                }
                if let Some(prepared) = self
                    .loading_state
                    .as_mut()
                    .and_then(|loading| loading.prepared_go.as_mut())
                {
                    prepared.use_fair_crew = use_fair_crew;
                    prepared.fair_crew_strength = fair_crew_strength;
                }
                if let Some(join_data) = self.pending_network_join_data.as_mut() {
                    join_data.parameters.use_fair_crew = use_fair_crew;
                    join_data.parameters.fair_crew_strength = fair_crew_strength;
                }
                if let Some(snapshot) = self.host_join_snapshot.as_mut() {
                    snapshot.parameters.use_fair_crew = use_fair_crew;
                    snapshot.parameters.fair_crew_strength = fair_crew_strength;
                    host_snapshot_changed = true;
                }
                if self.scenario_game_options.context().is_lobby() {
                    self.scenario_game_options.set_lobby_fair_crew_state(
                        use_fair_crew,
                        fair_crew_strength,
                        false,
                    );
                }
            }
            // C4CVT_None and unknown raw values are release-build no-ops.
            _ => {}
        }

        if host_snapshot_changed {
            self.publish_updated_host_join_snapshot();
        }
        if matches!(set.value_type, 3 | 4) {
            self.sync_classic_lobby_roster();
            self.refresh_classic_lobby_options(false);
        }
        if set.value_type == 2 {
            self.append_control_message_log(
                format!("MaxPlayer = {}", set.data),
                CONTROL_LOG_COLOR,
                None,
            );
            self.sync_classic_lobby_roster();
        }
    }

    fn apply_ready_controls(
        &mut self,
        tick: Tick,
        controls: Vec<NetworkControl>,
    ) -> Result<(), EngineError> {
        self.apply_ready_controls_from_queue(tick, controls, true)
    }

    fn apply_synchronized_controls(
        &mut self,
        tick: Tick,
        controls: Vec<NetworkControl>,
    ) -> Result<(), EngineError> {
        self.apply_ready_controls_from_queue(tick, controls, false)
    }

    fn apply_ready_controls_from_queue(
        &mut self,
        tick: Tick,
        controls: Vec<NetworkControl>,
        queued_runtime_record_request: bool,
    ) -> Result<(), EngineError> {
        let packets = controls
            .iter()
            .cloned()
            .filter_map(NetworkControl::into_packet)
            .collect::<Vec<_>>();
        let runtime_record_waiting = self.runtime_record_requested && self.recording.is_none();
        let mut batch_recorded = self.recording.is_some();
        if batch_recorded {
            self.record_control_batch(&packets);
        }
        debug_assert!(self.executing_ready_tick.is_none());
        self.executing_ready_tick = Some(tick);
        let stop_if_running_mode_exits = matches!(self.mode, AppMode::Running);
        let require_live_network = self.network.is_some();
        let replaying = self.control_playback.is_some();
        let allow_scripting_in_replays = replaying && self.allow_scripting_in_replays;
        let console_active = self.console_mode;
        let mut result = Ok(());
        for control in controls {
            result = match control {
                NetworkControl::PlayerInfo(info) => {
                    let client_id = info.client_id;
                    let local_origin = self
                        .network
                        .as_ref()
                        .and_then(|network| i32::try_from(network.local_client_id()).ok())
                        == Some(info.by_client);
                    let had_client_packet =
                        self.control_player_infos.client_packet(client_id).is_some();
                    let send_clean_follow_up =
                        matches!(self.runtime_network_role(), RuntimeNetworkRole::Host)
                            && info.flags & clonk_engine::CLIENT_PLAYER_INFO_FLAG_UPDATED != 0
                            && (info.flags & clonk_engine::CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS
                                == 0
                                || !had_client_packet);
                    self.admission_resources
                        .register_player_info_resources(&info.players);
                    self.generate_incoming_player_info_teams(&info.players);
                    self.control_player_infos.apply(info);
                    let rebalance_updates = self.recheck_team_memberships_from_player_infos();
                    if local_origin {
                        let mut updated_clients = rebalance_updates
                            .iter()
                            .map(|update| update.client_id)
                            .collect::<HashSet<_>>();
                        if send_clean_follow_up {
                            updated_clients.insert(client_id);
                        }
                        let updates = self.control_player_infos.client_packets(&updated_clients);
                        if let Some(network) = self.network.as_ref() {
                            for update in updates {
                                if let Err(error) = network.broadcast_player_info(update) {
                                    tracing::error!(%error, "failed to broadcast updated PlayerInfo");
                                }
                            }
                        }
                    }
                    seed_engine_player_info_parameters(
                        &mut self.engine,
                        &self.network_league_name,
                        &self.control_player_infos,
                    );
                    if matches!(self.runtime_network_role(), RuntimeNetworkRole::Offline)
                        && self.engine.is_control_host()
                    {
                        // C4ControlPlayerInfo::Execute rechecks teams before
                        // LocalJoinUnjoinedPlayersInQueue. Offline Rust folds
                        // that queued follow-up inline, like the existing
                        // CreateScriptPlayer admission path above.
                        let local_client_id = self.offline_local_client_id();
                        let joins = self
                            .control_player_infos
                            .issue_unjoined_local_players(local_client_id, |info| {
                                (!info.filename.is_empty()).then(|| info.filename.clone())
                            });
                        for join in joins {
                            if self.recording.is_some() {
                                let mut recorded_join = join.clone();
                                let recorded = if join.filename.is_empty() {
                                    Some(recorded_join)
                                } else {
                                    let path =
                                        PathBuf::from(join.filename.to_string_lossy().into_owned());
                                    match packed_group_bytes(
                                        &path,
                                        self.process_group_maker.as_bytes(),
                                    ) {
                                        Ok(player_data) => {
                                            recorded_join.source =
                                                clonk_engine::JoinPlayerSource::Embedded(
                                                    player_data,
                                                );
                                            Some(recorded_join)
                                        }
                                        Err(error) => {
                                            tracing::warn!(
                                                info_id = join.info_id,
                                                path = %path.display(),
                                                %error,
                                                "failed to embed offline runtime player in recording"
                                            );
                                            None
                                        }
                                    }
                                };
                                if let Some(recorded) = recorded {
                                    self.record_control_batch(std::slice::from_ref(
                                        &clonk_engine::ControlPacket::JoinPlayer(recorded),
                                    ));
                                }
                            }
                            self.apply_join_player_control(join)?;
                        }
                    }
                    self.publish_current_host_player_infos();
                    self.sync_classic_lobby_roster();
                    Ok(())
                }
                NetworkControl::JoinPlayer(join) => {
                    let aborted_key = match &join.source {
                        clonk_engine::JoinPlayerSource::Resource(core) => {
                            Some((core.id, join.info_id))
                        }
                        clonk_engine::JoinPlayerSource::Embedded(_) => None,
                    };
                    if aborted_key
                        .is_some_and(|key| self.aborted_player_resource_joins.remove(&key))
                    {
                        Ok(())
                    } else {
                        self.apply_join_player_control(join)
                    }
                }
                NetworkControl::RemovePlayer(control) => {
                    self.execute_remove_player_control(control)
                }
                NetworkControl::InitScenarioPlayer(control) => {
                    self.execute_init_scenario_player_control(control.player, control.team)
                }
                NetworkControl::SurrenderPlayer(control) => {
                    self.engine.execute_surrender_player_control(control);
                    Ok(())
                }
                NetworkControl::EmMoveObject(control) => self
                    .engine
                    .execute_em_move_object_control(
                        &control,
                        if replaying {
                            ScriptControlPolicy::replay(allow_scripting_in_replays)
                        } else {
                            ScriptControlPolicy::live(console_active)
                        },
                    )
                    .map(|_| ()),
                NetworkControl::EmDrawTool(control) => {
                    self.engine.execute_em_draw_tool_control(&control);
                    Ok(())
                }
                NetworkControl::EmDropDef(control) => self
                    .engine
                    .execute_em_drop_def_control(&control)
                    .map(|_| ()),
                NetworkControl::ActivateGameGoalMenu(control) => self
                    .engine
                    .execute_activate_game_goal_menu_control(&control)
                    .and_then(|_| self.apply_game_goal_menu_requests()),
                NetworkControl::ToggleHostility(control) => self
                    .engine
                    .execute_toggle_hostility_control(&control)
                    .map(|_| ()),
                NetworkControl::ActivateGameGoalRule(control) => self
                    .engine
                    .execute_activate_game_goal_rule_control(&control)
                    .map(|_| ()),
                NetworkControl::SetPlayerTeam(control) => self
                    .engine
                    .execute_set_player_team_control(&control)
                    .map(|_| ()),
                NetworkControl::EliminatePlayer(control) => self
                    .engine
                    .execute_eliminate_player_control(&control)
                    .map(|_| ()),
                NetworkControl::Vote(vote) => self.execute_league_vote(vote),
                NetworkControl::VoteEnd(result) => {
                    self.execute_league_vote_end(result);
                    Ok(())
                }
                NetworkControl::PlayerControl(data) => {
                    let outcome =
                        self.execute_player_control_failsafe(data.player, data.command, data.data);
                    if outcome.is_ok() && runtime_player_has_live_crew(&self.snapshot, data.player)
                    {
                        if let (Some(tick), Some(benchmark)) = (
                            self.executing_ready_tick,
                            self.input_latency_benchmark.as_mut(),
                        ) {
                            benchmark.record_execution(tick, &data, Instant::now());
                        }
                    }
                    outcome
                }
                NetworkControl::PlayerCommand(data) => self.execute_player_command_failsafe(data),
                NetworkControl::PlayerSelect(data) => {
                    self.engine.execute_player_select(&data).map(|_| ())
                }
                NetworkControl::Script(data) => self
                    .engine
                    .execute_script_control(
                        &data,
                        if replaying {
                            ScriptControlPolicy::replay(allow_scripting_in_replays)
                        } else {
                            ScriptControlPolicy::live(console_active)
                        },
                    )
                    .map(|_| ()),
                NetworkControl::MessageBoardAnswer(data) => self
                    .engine
                    .execute_message_board_answer_control(&data)
                    .map(|_| ()),
                NetworkControl::Message(message) => {
                    self.execute_message_control(message);
                    Ok(())
                }
                NetworkControl::CustomCommand(data) => {
                    let game_running = matches!(self.mode, AppMode::Running);
                    self.engine
                        .execute_custom_command_control(&data, game_running)
                        .map(|_| ())
                }
                NetworkControl::Player { owner, event } => {
                    self.dispatch_control_event_for_owner(owner, event)
                }
                NetworkControl::Synchronize(control) => (|| {
                    if runtime_record_waiting && self.runtime_record_requested {
                        // C4Game::Synchronize calls OnGameSynchronizing before
                        // mutating synchronized state. Earlier packets in this
                        // complete control have already executed, so capture
                        // the non-initial record here rather than at batch
                        // admission. StartRecord then records the entire
                        // executing C4Control, including those earlier rows.
                        self.runtime_record_requested = false;
                        let started = self
                            .prepare_runtime_recording_at_synchronize()
                            .and_then(|()| self.start_recording(true));
                        match started {
                            Ok(true) if !batch_recorded => {
                                self.record_control_batch(&packets);
                                batch_recorded = true;
                            }
                            Ok(_) => {}
                            Err(error) => {
                                tracing::warn!(%error, "failed to start runtime control recording");
                            }
                        }
                    }
                    self.engine
                        .execute_synchronize_control_before_network(control.save_player_files)?;
                    if control.save_player_files && !replaying {
                        // C4Game owns the state checkpoint; C4Player owns the
                        // physical group write. Its aggregate failure is not
                        // propagated by C4Game::Synchronize.
                        let _ = self.persist_synchronized_local_player_files();
                    }
                    if self.pending_runtime_dynamic_request.is_some() {
                        self.on_runtime_join_synchronized(tick);
                    }
                    self.engine
                        .execute_synchronize_control_after_network(control.sync_clearance)?;
                    Ok(())
                })(),
                NetworkControl::SyncCheck(packet) => {
                    self.handle_sync_check(packet);
                    Ok(())
                }
                NetworkControl::Set(set) => {
                    self.execute_control_set(set);
                    Ok(())
                }
                NetworkControl::DebugRecord(_) => {
                    // Preserve and record the native packet; execution is a no-op.
                    Ok(())
                }
                NetworkControl::ClientUpdate(update) => {
                    let host_authored = update.by_client == 0;
                    let activates_client = host_authored
                        && update.update_type == clonk_engine::CLIENT_UPDATE_ACTIVATE
                        && update.data != 0
                        && self.control_clients.contains(update.client_id)
                        && !self.control_clients.is_activated(update.client_id);
                    let removes_players = host_authored
                        && update.update_type == clonk_engine::CLIENT_UPDATE_SET_OBSERVER
                        && self.control_clients.contains(update.client_id)
                        && !self.control_clients.is_observer(update.client_id);
                    self.control_clients.apply_update(&update);
                    if activates_client && self.control_clients.is_activated(update.client_id) {
                        self.network_client_activity.mark_activated(
                            update.client_id,
                            i32::try_from(self.engine.frame()).unwrap_or(i32::MAX),
                        );
                    }
                    if removes_players && self.control_clients.is_observer(update.client_id) {
                        self.remove_runtime_players_at_client(update.client_id, true);
                        self.refresh_current_host_player_infos();
                    }
                    if matches!(self.network_mode.as_ref(), Some(NetworkMode::Client(_))) {
                        if let Some(Err(error)) = self
                            .network
                            .as_ref()
                            .map(|network| network.notify_client_update_executed(update))
                        {
                            tracing::error!(%error, "failed to report executed client update");
                        }
                    }
                    if host_authored {
                        self.publish_updated_host_join_snapshot();
                    }
                    self.sync_classic_lobby_roster();
                    Ok(())
                }
                NetworkControl::ClientJoin(join) => {
                    if self.control_clients.apply_join(&join) {
                        self.append_network_client_join_log(join.core.name.as_bytes());
                        self.network_client_activity
                            .reset_client(join.core.client_id);
                        self.publish_updated_host_join_snapshot();
                    }
                    self.sync_classic_lobby_roster();
                    Ok(())
                }
                NetworkControl::ClientRemove(remove) => {
                    if remove.by_client == 0 {
                        if let Ok(client_id) = ClientId::try_from(remove.client_id) {
                            self.forget_pending_runtime_join_client(client_id);
                        }
                    }
                    let local_client_id = self
                        .network
                        .as_ref()
                        .and_then(|network| i32::try_from(network.local_client_id()).ok());
                    if remove.by_client == 0
                        && remove.client_id != 0
                        && local_client_id == Some(remove.client_id)
                    {
                        self.change_network_control_to_local(remove.client_id);
                        Ok(())
                    } else {
                        let removes_client = remove.by_client == 0
                            && remove.client_id != 0
                            && self.control_clients.contains(remove.client_id);
                        if removes_client
                            || remove.by_client == 0 && remove.client_id != 0 && replaying
                        {
                            self.remove_runtime_players_at_client(remove.client_id, true);
                        }
                        if self.control_clients.apply_remove(&remove) {
                            if let Some(wait) = self.network_start_wait.as_mut() {
                                wait.controller.remove_client(remove.client_id);
                            }
                            self.remove_classic_lobby_resources_at_client(remove.client_id);
                            self.network_client_activity.remove_client(remove.client_id);
                            self.control_messages.remove_client(remove.client_id);
                            let had_player_info = self
                                .control_player_infos
                                .client_packet(remove.client_id)
                                .is_some();
                            self.control_player_infos.on_client_part(remove.client_id);
                            self.finish_control_client_part(had_player_info);
                        }
                        self.sync_classic_lobby_roster();
                        Ok(())
                    }
                }
            };
            // `C4Player::Eliminate` queues its synchronized client update at
            // the point of the state transition, even if a later control in
            // this batch reports an error.
            self.flush_pending_client_updates();
            // Process-local SetPreSend effects happen at this packet's exact
            // position. In particular, apply them before a later ClientRemove
            // tears down the network clock and local-name registry. Drain even
            // when this packet reports an error: native has already performed
            // the mutation before the later failure.
            let pacing_result = self.apply_engine_network_target_fps_requests();
            if result.is_ok() {
                result = pacing_result;
            }
            if result.is_err()
                || (stop_if_running_mode_exits && !matches!(self.mode, AppMode::Running))
                || (require_live_network && self.network.is_none())
            {
                break;
            }
        }
        // A synchronized script executes inside native Control.Execute and
        // changes pause state before ControlTick advances, while the current
        // game frame still runs to completion. Apply those app-owned requests
        // now; the caller advances its cadence clock after this method.
        self.apply_engine_pause_game_requests();
        // `FnReloadParticle` answered the script synchronously from pre-seeded
        // state; the reload itself happens here, once the call has returned.
        // `FnReloadParticle`/`FnReloadDef` answered the script synchronously
        // from pre-seeded state; the reloads themselves happen here, once the
        // call has returned. C++ reloads inside the call, so this defers the
        // *work* by one pass — the script's answer is unaffected.
        let reloaded = self.engine.apply_particle_reload_requests()
            + self.engine.apply_definition_reload_requests();
        if reloaded > 0 {
            tracing::debug!(reloaded, "applied script-driven reloads");
        }
        let goal_menu_result = self.apply_game_goal_menu_requests();
        if result.is_ok() {
            result = goal_menu_result;
        }
        // A script/control callback may have called
        // EliminatePlayer(plr, true). While the current ready marker is live,
        // local_control_submission_tick selects tick + 1, matching Game.Input.
        let follow_up_result = self.flush_pending_remove_player_controls(false);
        if result.is_ok() {
            result = follow_up_result;
        }
        // Controls generated reentrantly above used tick + 1. Clear the
        // marker before propagating errors or returning after session state
        // changes so later local input cannot inherit a stale target tick.
        self.executing_ready_tick = None;
        if !queued_runtime_record_request && self.network.is_some() {
            // C4GameControlNetwork::ExecQueuedSyncCtrl refreshes its private
            // activated-client copy after every synchronized batch.
            self.refresh_network_client_next_control_ticks();
        }
        result
    }

    fn install_network_event_waker(&mut self, callback: NetworkEventWakeCallback) {
        self.network_event_waker = Some(callback);
        self.refresh_network_event_waker();
    }

    fn refresh_network_event_waker(&self) {
        if let (Some(network), Some(callback)) =
            (self.network.as_ref(), self.network_event_waker.as_ref())
        {
            network.install_event_waker(callback.clone());
        }
    }

    fn note_network_event_wake(&mut self, wake: NetworkEventWake) {
        self.network_control_retry_pending |= matches!(
            (self.waiting_network_control, wake),
            (
                Some(NetworkControlWait::ReadyTick(expected)),
                NetworkEventWake::ReadyTick(actual)
            ) if expected == actual
        ) || matches!(
            (self.waiting_network_control, wake),
            (
                Some(NetworkControlWait::PlayerResource { resource_id: expected }),
                NetworkEventWake::ResourceComplete(actual)
                    | NetworkEventWake::ResourceLoadFailed(actual)
            ) if expected == actual
        );
    }

    fn take_network_control_retry(&mut self) -> bool {
        std::mem::take(&mut self.network_control_retry_pending)
    }

    fn update(&mut self) -> Result<(), EngineError> {
        let result = self.update_before_sound_instance_step();
        // C4Application runs SoundSystem::Execute after every application
        // pass, including game-over, halt and network-control early returns.
        self.update_sound_instances_for_current_mode();
        result
    }

    /// Consume C4Game's scheduler-owned one-second callback. Headless callers
    /// stay deterministic because only the window loop drives this method;
    /// tests and other hosts may pulse it explicitly.
    fn sec1_timer(&mut self) -> Result<bool, EngineError> {
        self.guard_classic_global_gui_bootstrap()?;
        let status_reached = self.check_runtime_network_status_reached();
        // C4Network2 is also a one-second timer proc and calls Execute from
        // that independent scheduler callback (`src/C4Network2.cpp:674-677`).
        self.deactivate_inactive_network_clients();
        let lobby_countdown_changed = self.tick_network_lobby_countdown();
        let ready_check_changed = self.tick_lobby_ready_check_prompt();
        let scale_test_changed = self.tick_options_scale_test_prompt();
        let lobby_options_changed = self.refresh_classic_lobby_options(false);
        let lobby_scenario_changed = self.refresh_lobby_scenario_description();
        let lobby_client_telemetry_changed = self.refresh_classic_lobby_client_telemetry();
        let now = i64::try_from(current_unix_timestamp()).unwrap_or(i64::MAX);
        let vote_timeout_changed = self.tick_host_league_vote_timeout_at(now);
        let before = self.engine.game_time();
        self.engine.sec1_timer();
        let after = self.engine.game_time();
        self.frames_per_second = std::mem::take(&mut self.frames_since_second);
        self.presentation_stats.sample_second();
        self.record_network_stats_second();
        let client_list_changed = self.refresh_runtime_client_list_on_sec1();
        if after != before {
            self.snapshot.game_time = after;
        }
        if matches!(self.mode, AppMode::Running) && !self.snapshot.game_over {
            // Native refreshes the reference server throughout play rather
            // than releasing its listener at the lobby boundary.
            self.publish_running_host_reference();
        }
        self.tick_league_update_at(now);
        self.tick_league_record_stream_at(now);
        let game_over_network_result_changed = self.refresh_game_over_network_result();
        Ok(status_reached == RuntimeStatusReachOutcome::Reported
            || lobby_countdown_changed
            || ready_check_changed
            || scale_test_changed
            || lobby_options_changed
            || lobby_scenario_changed
            || lobby_client_telemetry_changed
            || vote_timeout_changed
            || client_list_changed
            || game_over_network_result_changed
            || after != before)
    }

    fn finish_rendered_object_audibility_pass(&mut self) {
        let viewports = self.graphics.active_viewport_projections();
        let calls = self.graphics.rendered_object_audibility_calls();
        if let Some(audio) = self.audio.as_mut() {
            audio.cache_rendered_object_audibility(calls, &self.snapshot, &viewports);
            audio.refresh_attached_channel_mix_after_render(&self.snapshot, &viewports);
        }
    }

    fn handle_game_over_action(&mut self, action: GameOverAction) -> Result<(), EngineError> {
        // Drain any DoDlgShow issued while the exclusive evaluation dialog was
        // open before Continue can dismiss it. Such a request was suppressed
        // at call time and must not create a scoreboard afterwards.
        self.sync_scoreboard_presentation();
        match action {
            GameOverAction::Continue => {
                let resumes_changed_to_local_control = self.network.is_none()
                    && !self.network_control_running
                    && self.offline_halt_count != 0;
                self.dismiss_game_over_dialog();
                // C4GameOverDlg::OnClosed invokes Game.Unpause only after an
                // accepted Continue close. This requests synchronized GS_Go
                // for a host, remains a client no-op, and clears the direct
                // offline halt (src/C4GameOverDlg.cpp:360-381;
                // src/C4Game.cpp:1071-1084).
                self.set_runtime_pause(false);
                if resumes_changed_to_local_control {
                    // Fatal network cleanup transfers the dialog's stopped
                    // control into an offline halt. Continue releases both
                    // halves of that ChangeToLocal handoff.
                    self.network_control_running = true;
                }
            }
            GameOverAction::End => {
                self.return_to_menu();
            }
            GameOverAction::Restart => {
                self.restart_current_scenario()?;
            }
            GameOverAction::NextMission => {
                let path = self.engine.next_mission().path.clone();
                let definition_load = self.active_definition_load.clone();
                // C4GameOverDlg preserves restart infos only for Restart;
                // actual Next Mission clears them as soon as it closes.
                self.restart_restore_infos = RestartRestoreInfos::default();
                let Some(scenario) = resolve_next_mission_scenario(&self.scenario_catalog, &path)
                else {
                    self.status_text = format!("Next scenario is unavailable: {path}");
                    return Ok(());
                };
                self.return_to_menu_for_relaunch();
                match definition_load {
                    Some(definition_load) => {
                        self.start_scenario_with_definition_load(scenario, definition_load)?;
                    }
                    None => self.start_scenario(scenario)?,
                }
            }
        }
        Ok(())
    }

    fn handle_game_over(&mut self) -> Result<(), EngineError> {
        self.guard_classic_global_gui_bootstrap()?;
        // The evaluation dialog's graphics are only needed to draw one. A
        // console engine never builds it (C4Game.cpp:3679-3690), and a
        // `USE_CONSOLE` build has no `C4FontLoader` to render it with
        // (C4Game.h:132-135), so a server must not fail its round end on
        // resources it will not use.
        if !self.headless {
            self.assets
                .require_classic_game_over_resources_with_hud(self.current_hud_graphics_ref())
                .map_err(report_classic_parity_boundary)
                .map_err(classic_parity_engine_error)?;
        }
        // DoGameOver calls C4Player::EvaluateLeague for every survivor,
        // which sets PIF_Won on the linked live C4PlayerInfo before the
        // reference or evaluation UI consumes it.
        let winner_info_ids = self
            .snapshot
            .players
            .iter()
            .filter(|player| player.won && player.player_info_id != 0)
            .map(|player| player.player_info_id)
            .collect::<Vec<_>>();
        for player_info_id in winner_info_ids {
            self.control_player_infos.mark_winner(player_info_id);
        }
        let league_record = self.finish_recording();
        self.game_over_handled = true;
        self.publish_game_over_host_reference();
        if self.network_is_league && league_record.is_none() {
            // A league game is admitted only after its forced recorder has
            // materialized. Never downgrade a failed close into a recordless
            // End request, which the league server cannot validate.
            tracing::error!("refusing to finish league game without its required record");
            return self.finish_game_over_after_league();
        } else if self.network_is_league {
            if let (Some(_), Some(reference)) = (
                self.network.as_ref(),
                self.advertised_game_reference.clone(),
            ) {
                self.pending_league_end = Some(PendingLeagueEnd {
                    reference,
                    record: league_record,
                    attempts: 0,
                    last_failure: None,
                    terminal_packet: None,
                });
                return self.run_pending_league_end_attempt();
            }
        }
        self.finish_game_over_after_league()
    }

    fn apply_pending_loading_resource_refresh(&mut self) -> Result<(), EngineError> {
        let Some(failures) = self
            .loading_state
            .as_ref()
            .filter(|state| state.refresh_requested)
            .and_then(|state| state.refreshed_global_gui_failures.as_ref())
            .cloned()
        else {
            return Ok(());
        };
        self.assets
            .require_classic_global_gui_bootstrap_resources(&failures)
            .map_err(report_classic_parity_boundary)
            .map_err(classic_parity_engine_error)?;

        let Some(state) = self.loading_state.as_mut() else {
            return Ok(());
        };
        let resources = state.refreshed_resources.take();
        let tooltip_font = state.refreshed_tooltip_font.take();
        let native_font_source = state.refreshed_native_font_source.take();
        let failures = state
            .refreshed_global_gui_failures
            .take()
            .unwrap_or_default();
        let sheet_overrides = state.refreshed_gui_sheet_overrides.take();
        state.refresh_requested = false;
        if let Some(resources) = resources {
            let fonts = resources.fonts().clone();
            self.install_active_classic_fonts(fonts, tooltip_font, native_font_source);
            if let Some(loader) = self.loader_screen.as_mut() {
                loader.replace_resources(resources);
            }
        }
        if let Some(sheet_overrides) = sheet_overrides {
            self.install_active_gui_sheet_overrides(&sheet_overrides);
        }
        self.active_global_gui_failures = failures;
        Ok(())
    }

    fn try_reach_loaded_network_go_barrier(&mut self) -> Result<(), EngineError> {
        let ready_to_reach = self.mode == AppMode::Loading
            && self.loading_state.as_ref().is_some_and(|loading| {
                loading.finished
                    && loading
                        .prepared_go
                        .as_ref()
                        .is_some_and(|prepared| !prepared.local_reached)
            });
        if !ready_to_reach {
            return Ok(());
        }

        // Network::FinalInit reaches and waits for the GO barrier before
        // InitPlayers begins recreating players and retrieving their files
        // (src/C4Network2.cpp:558-616; src/C4Game.cpp:459-483,2805-2850).
        let client_control_tick = if matches!(self.network_mode, Some(NetworkMode::Client(_))) {
            self.network_control_clock
                .map(NetworkControlClock::current_tick)
        } else {
            None
        };
        let reached = if matches!(self.network_mode, Some(NetworkMode::Client(_))) {
            match client_control_tick {
                Some(current_control_tick) => {
                    let current_frame = i32::try_from(self.engine.frame()).unwrap_or(i32::MAX);
                    match self
                        .client_start_barrier
                        .local_initialized_at(current_control_tick)
                    {
                        Some(_) => self.network.as_mut().map(|network| {
                            network.acknowledge_requested_status_at_frame(
                                current_control_tick,
                                current_frame,
                            )
                        }),
                        None => None,
                    }
                }
                None => None,
            }
        } else {
            self.network
                .as_ref()
                .map(NetworkManager::status_reached_current)
        };
        match reached {
            Some(Ok(())) => {
                if let Some(pending) = self
                    .loading_state
                    .as_mut()
                    .and_then(|loading| loading.prepared_go.as_mut())
                {
                    pending.local_reached = true;
                    if let Some(current_control_tick) = client_control_tick {
                        pending.status.target_tick = current_control_tick;
                    }
                }
                self.show_reached_network_start_wait()?;
            }
            Some(Err(error)) => {
                self.status_text = format!("Unable to reach network Go barrier: {error}");
            }
            None => {
                self.status_text = "Network Go barrier is unavailable".to_string();
            }
        }
        Ok(())
    }

    fn poll_loading(&mut self) -> Result<(), EngineError> {
        self.apply_pending_loading_resource_refresh()?;
        let mut completion: Option<(FrontendScenario, Result<Scenario, String>, bool)> = None;
        while let Some(event) = self
            .loading_state
            .as_ref()
            .filter(|state| !state.finished)
            .map(|state| state.receiver.try_recv())
        {
            match event {
                Ok(ScenarioLoadingEvent::LoaderFrame { progress, log }) => {
                    self.apply_scenario_loader_frame(progress, log);
                }
                Ok(ScenarioLoadingEvent::RefreshResources) => {
                    if let Some(state) = self.loading_state.as_mut() {
                        state.refresh_requested = true;
                    }
                    self.apply_pending_loading_resource_refresh()?;
                }
                Ok(ScenarioLoadingEvent::AcceptedRandomSeed(random_seed)) => {
                    let state = self
                        .loading_state
                        .as_mut()
                        .expect("loading state exists while draining its receiver");
                    state.offline_random_seed = Some(random_seed);
                }
                Ok(ScenarioLoadingEvent::Finished(result)) => {
                    let state = self
                        .loading_state
                        .as_mut()
                        .expect("loading state exists while draining its receiver");
                    state.finished = true;
                    // The loading screen is done; release its log buffer the
                    // way C4MessageBoard drops the startup buffer once the
                    // round is up (src/C4MessageBoard.cpp:223-251).
                    clonk_logging::deactivate_loader_log();
                    // Entering startup removes the taskbar indicator
                    // (C4Application.cpp:422-426).
                    self.taskbar_progress.enter_startup();
                    completion =
                        Some((state.scenario.clone(), result, state.prepared_go.is_some()));
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    let state = self
                        .loading_state
                        .as_mut()
                        .expect("loading state exists while draining its receiver");
                    state.finished = true;
                    completion = Some((
                        state.scenario.clone(),
                        Err("Scenario loading interrupted".to_string()),
                        state.prepared_go.is_some(),
                    ));
                    break;
                }
            }
        }

        if let Some((scenario, result, prepared_go)) = completion {
            match result {
                Ok(data) => {
                    if let Some(pending) = self
                        .loading_state
                        .as_mut()
                        .and_then(|loading| loading.prepared_go.as_mut())
                    {
                        let (save_game, network_runtime_join) =
                            data.lobby_metadata().map_or((false, false), |metadata| {
                                (
                                    metadata.head().is_save_game(),
                                    metadata.head().allows_network_runtime_join(),
                                )
                            });
                        pending.save_game = save_game;
                        pending.network_runtime_join = network_runtime_join;
                    }
                    let activation = self.activate_loaded_scenario(scenario.clone(), &data);
                    if activation.is_ok() {
                        self.classic_record_stream_activation_pending = false;
                    }
                    if let Err(error) = activation {
                        let ScenarioActivationError::Recoverable(message) = error;
                        tracing::error!(scenario = %scenario.title, error = %message, "failed to start scenario");
                        self.finish_scenario_loading_failure(message, prepared_go)?;
                    } else if prepared_go {
                        self.mode = AppMode::Loading;
                    } else {
                        self.advance_scenario_loader(100, "Scenario activation complete");
                        self.loading_state = None;
                    }
                }
                Err(message) => {
                    tracing::error!(scenario = %scenario.title, error = %message, "failed to load scenario");
                    self.finish_scenario_loading_failure(message, prepared_go)?;
                }
            }
        }

        self.try_reach_loaded_network_go_barrier()?;
        self.try_finish_deferred_prepared_network_go()?;
        Ok(())
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

    fn about_tooltip_target_at(&self, point: GuiPoint) -> Option<StartupTooltip> {
        let dialog = self.startup_about_dialog.as_ref()?;
        let fonts = self.assets.clonk_fonts.as_deref()?;
        let surface = self.graphics.surface();
        let layout = clonk_frontend::startup_about_dlg::about_layout(
            surface.width() as i32,
            surface.height() as i32,
        );
        // About passes LoadResStr directly rather than LoadResStrNoAmp. The
        // Label expands the hotkey marker for sizing, while SetToolTip keeps
        // the raw ampersand text.
        let title = self.startup_tooltip_resource_string("IDS_DLG_ABOUT");
        let (display_title, _) = clonk_frontend::expand_hotkey_markup(&title);
        clonk_frontend::centered_label_tooltip_at(
            point,
            layout.title_anchor,
            fonts.title.measure(&display_title, true),
            StartupTooltip::text(title),
        )
        .or_else(|| dialog.tooltip_at(point))
    }

    fn reject_classic_global_gui_bootstrap(&self) -> Result<()> {
        self.assets
            .require_classic_global_gui_bootstrap_resources(self.effective_global_gui_failures())
            .map_err(report_classic_parity_boundary)
            .map_err(anyhow::Error::new)
    }

    fn gui_overlay_boundary(
        overlay: &'static str,
        detail: impl Into<String>,
    ) -> ClassicParityBoundary {
        ClassicParityBoundary::GuiOverlayResources {
            overlay,
            detail: detail.into(),
        }
    }

    fn preflight_visible_gui_overlay_resources(&self) -> Result<()> {
        let check = |result: Result<()>, overlay| {
            result.map_err(|error| {
                anyhow::Error::new(report_classic_parity_boundary(Self::gui_overlay_boundary(
                    overlay,
                    error.to_string(),
                )))
            })
        };

        if self.definition_selector.is_some() {
            let mode = self
                .definition_selector
                .as_ref()
                .map(|selector| selector.mode())
                .unwrap_or(clonk_frontend::definition_sel::FileSelMode::Definitions);
            check(
                self.assets
                    .definition_sel_resources()
                    .context("exact C4DefinitionSelDlg resource set is absent")
                    .and_then(|resources| resources.validate_for_mode(mode)),
                if mode == clonk_frontend::definition_sel::FileSelMode::Player {
                    "C4PlayerSelDlg"
                } else {
                    "C4DefinitionSelDlg"
                },
            )?;
        }
        if self.game_option_input_dialog.is_some() {
            check(
                self.assets.input_dialog_resources().map(|_| ()),
                "C4GUI::InputDialog",
            )?;
        }
        if self.league_signup_dialog.is_some() {
            check(
                self.assets
                    .league_signup_resources()
                    .and_then(|resources| resources.validate()),
                "C4LeagueSignupDialog",
            )?;
        }
        if self.context_menu.is_some() {
            check(
                self.assets.context_menu_resources().map(|_| ()),
                "C4GUI context menu",
            )?;
        }
        if !self.message_dialogs.is_empty() {
            check(
                self.assets
                    .message_dialog_resources()
                    .context("exact C4GUI::MessageDialog resource set is absent")
                    .and_then(|resources| resources.validate()),
                "C4GUI::MessageDialog",
            )?;
        }
        if let Some(dialog) = self.runtime_client_list.as_ref() {
            if dialog.is_static_info_only() {
                check(
                    self.assets
                        .static_info_dialog_resources()
                        .context("exact C4GUI::InfoDialog resource set is absent")
                        .and_then(|resources| resources.validate()),
                    "C4GUI::InfoDialog",
                )?;
            } else {
                check(
                    self.assets
                        .runtime_client_list_resources()
                        .context("exact C4Network2ClientListDlg resource set is absent")
                        .and_then(|resources| resources.validate()),
                    "C4Network2ClientListDlg",
                )?;
            }
        }
        if self.network_chart_dialog.is_some() {
            check(
                self.assets
                    .network_chart_resources()
                    .context("exact C4ChartDialog resource set is absent")
                    .and_then(|resources| resources.validate()),
                "C4ChartDialog",
            )?;
        }
        if self.external_irc_dialog_visible {
            check(
                self.assets
                    .netdlg_assets()
                    .context("exact C4ChatDlg graphics are unavailable")
                    .and_then(|_| {
                        self.assets
                            .clonk_fonts
                            .as_ref()
                            .context("C4ChatDlg fonts are unavailable")
                            .map(|_| ())
                    }),
                "C4ChatDlg",
            )?;
        }
        if self.mode == AppMode::Menu
            && matches!(
                self.startup_view,
                StartupView::ScenarioBrowser | StartupView::NetworkLobby
            )
        {
            if self.startup_view == StartupView::NetworkLobby && self.classic_host_lobby.is_some() {
                check(
                    self.assets.game_lobby_resources().map(|_| ()),
                    "C4GameLobby",
                )?;
            }
            check(
                self.assets.game_option_resources().map(|_| ()),
                "scenario/lobby game-option strip",
            )?;
        }
        Ok(())
    }

    fn guard_classic_global_gui_bootstrap(&self) -> Result<(), EngineError> {
        self.assets
            .require_classic_global_gui_bootstrap_resources(self.effective_global_gui_failures())
            .map_err(report_classic_parity_boundary)
            .map_err(classic_parity_engine_error)
    }

    fn effective_global_gui_failures(&self) -> &HashMap<&'static str, String> {
        self.loading_state
            .as_ref()
            .filter(|state| state.refresh_requested)
            .and_then(|state| state.refreshed_global_gui_failures.as_ref())
            .unwrap_or(&self.active_global_gui_failures)
    }

    fn guard_gui_overlay_result(
        overlay: &'static str,
        result: Result<()>,
    ) -> Result<(), EngineError> {
        result.map_err(|error| Self::gui_overlay_engine_error(overlay, error))
    }

    fn gui_overlay_engine_error(overlay: &'static str, error: impl fmt::Display) -> EngineError {
        classic_parity_engine_error(report_classic_parity_boundary(Self::gui_overlay_boundary(
            overlay,
            error.to_string(),
        )))
    }

    fn ingame_selection_candidates(&self, motion: IngameMouseState) -> Vec<ObjectId> {
        if motion.selection_kind == IngameDragSelectionKind::Unknown {
            return Vec::new();
        }
        self.ingame_dragged_objects
            .iter()
            .copied()
            .filter(|object| {
                self.engine
                    .object_snapshot(*object)
                    .is_some_and(|object| object.status != clonk_engine::ObjectStatus::Deleted)
            })
            .collect()
    }

    fn apply_show_commands_enable_request(&mut self) {
        if self.show_commands_requests.take_enable_request() {
            self.display_flags.show_commands = true;
        }
    }

    /// `Game.Time` from the last deterministic engine snapshot.
    fn game_time_seconds(&self) -> u64 {
        self.snapshot.game_time.max(0) as u64
    }

    fn install_active_classic_fonts(
        &mut self,
        fonts: Arc<clonk_frontend::ClonkFontSet>,
        tooltip: Option<Arc<clonk_graphics::clonk_font::ClonkFont>>,
        native_source: Option<ClassicNativeFontSource>,
    ) {
        let native_fonts = self.native_font_cache_for_source(native_source.as_ref());
        let assets = Arc::make_mut(&mut self.assets);
        assets.clonk_fonts = Some(fonts.clone());
        if let Some(tooltip) = tooltip {
            assets.global_tooltip_font = Some(tooltip);
        }
        self.graphics.set_clonk_fonts(Some(fonts.clone()));
        self.main_menu_state.menu.set_clonk_fonts(Some(fonts));
        self.native_startup_fonts = native_fonts;
    }

    /// Rebinds the process-global GUI sheets to the active scenario's
    /// winners (`C4GraphicsResource::Init` → `C4GUI::Resource::Load` over
    /// the registered set). Cached menu graphics captured the previous
    /// sheets and are rebuilt lazily from the rebound ones.
    fn install_active_gui_sheet_overrides(&mut self, overrides: &[ClassicGuiSheetOverride]) {
        if overrides.is_empty() && self.assets.active_gui_sheet_sources.is_empty() {
            return;
        }
        if Arc::make_mut(&mut self.assets).apply_active_gui_sheet_overrides(overrides) {
            self.ingame_menu_gfx = None;
        }
    }

    fn runtime_resource_string(&self, key: &str) -> String {
        self.startup_tooltip_resources
            .get(key)
            .cloned()
            .unwrap_or_else(|| format!("[Undefined: {key}]"))
    }

    fn runtime_resource_bytes(&self, key: &str) -> Vec<u8> {
        self.runtime_resource_bytes_with_fallback(key, &format!("[Undefined: {key}]"))
    }

    fn runtime_resource_bytes_with_fallback(&self, key: &str, fallback: &str) -> Vec<u8> {
        let Some(value) = self.startup_tooltip_resources.get(key) else {
            return fallback.as_bytes().to_vec();
        };
        match self.runtime_language_charset {
            RuntimeHelpCharset::Windows1252 => value
                .chars()
                .map(runtime_cp1252_byte)
                .collect::<Result<Vec<_>>>()
                .expect("decoded Windows-1252 resources re-encode losslessly"),
            RuntimeHelpCharset::Utf8 => value.as_bytes().to_vec(),
        }
    }

    fn runtime_resource_text(&self, key: &str, fallback: &str) -> String {
        runtime_resource_text_from_table(&self.startup_tooltip_resources, key, fallback)
    }

    fn quick_load(&mut self) -> Result<()> {
        self.reject_classic_global_gui_bootstrap()?;
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

    fn loaded_game_classic_font_bundle(
        &self,
        frontend: &FrontendScenario,
        definition_load: Option<&ScenarioDefinitionLoad>,
    ) -> Result<ClassicFontBundle> {
        let paths = self
            .app_paths
            .as_ref()
            .context("application paths are unavailable for saved-game font resolution")?;
        let (head, catalog, graphics_registrations) =
            loaded_game_gui_registrations(frontend, definition_load, paths)?;
        resolve_classic_font_bundle(paths, Some(head.font()), &catalog, &graphics_registrations)
    }

    fn apply_loaded_game(&mut self, save: SavedGameFile) -> Result<()> {
        self.reject_classic_global_gui_bootstrap()?;
        let scenario_info = save.scenario.clone();
        let frontend = scenario_info.to_frontend();
        let saved_definition_load = save.definition_load.clone();

        let (loaded_resolution, loaded_fonts, loaded_game_graphics) = if scenario_info.sandbox {
            (ClassicGuiSheetResolution::default(), None, None)
        } else {
            (
                self.loaded_game_global_gui_resolution(&frontend, saved_definition_load.as_ref())?,
                Some(
                    self.loaded_game_classic_font_bundle(
                        &frontend,
                        saved_definition_load.as_ref(),
                    )?,
                ),
                Some(
                    self.loaded_game_graphics_resources(&frontend, saved_definition_load.as_ref())?,
                ),
            )
        };
        self.assets
            .require_classic_global_gui_bootstrap_resources(&loaded_resolution.failures)
            .map_err(report_classic_parity_boundary)
            .map_err(anyhow::Error::new)?;
        self.install_active_gui_sheet_overrides(&loaded_resolution.overrides);
        self.active_global_gui_failures = loaded_resolution.failures;
        if let Some(bundle) = loaded_fonts {
            self.install_active_classic_fonts(
                bundle.fonts,
                Some(bundle.tooltip),
                bundle.native_source,
            );
        }

        // C4Player runtime objects are recreated from their linked
        // C4PlayerInfo entries. Keep process-local input assignments keyed by
        // that stable identity rather than by the old round's player number.
        let previous_local_owner = self.local_owner;
        let previous_primary_info_id = self
            .engine
            .player(previous_local_owner)
            .map(|player| player.player_info_id())
            .filter(|info_id| *info_id > 0);
        let previous_local_controls_by_owner = self
            .local_controls
            .assignments()
            .map(|assignment| (assignment.owner, assignment))
            .collect::<HashMap<_, _>>();
        let previous_local_controls_by_info = self
            .local_controls
            .assignments()
            .filter_map(|assignment| {
                self.engine
                    .player(assignment.owner)
                    .map(|player| player.player_info_id())
                    .filter(|info_id| *info_id > 0)
                    .map(|info_id| (info_id, assignment))
            })
            .collect::<HashMap<_, _>>();
        let previous_player_preferences_by_owner = self
            .engine
            .players()
            .map(|player| {
                let (pref_control, pref_mouse) = player.control_preferences();
                let (pref_control_style, pref_auto_context_menu) =
                    player.control_style_preferences();
                (
                    player.id(),
                    (
                        pref_control,
                        pref_mouse,
                        pref_control_style,
                        pref_auto_context_menu,
                    ),
                )
            })
            .collect::<HashMap<_, _>>();
        let previous_player_preferences_by_info = self
            .engine
            .players()
            .filter(|player| player.player_info_id() > 0)
            .map(|player| {
                (
                    player.player_info_id(),
                    previous_player_preferences_by_owner[&player.id()],
                )
            })
            .collect::<HashMap<_, _>>();
        let parameter_bootstrap = (
            self.engine.fair_crew_forced(),
            self.engine.allow_debug(),
            self.engine.control_rate(),
        );
        let saved_allow_debug = save.engine_state.allow_debug;
        if let Some(league_name) = save.engine_state.league_name.as_ref() {
            self.network_league_name = league_name.clone();
        }

        self.finish_recording();
        self.live_save_seed = None;
        self.recording_template = None;
        self.control_playback = None;
        if let Some(audio) = self.audio.as_mut() {
            // Loading a save starts a fresh native round. Rebuild the
            // SoundSystem generation before installing that round's effects.
            audio.reset_sound_system_generation();
        }
        self.engine = Engine::new();
        self.film_view_player = None;
        self.clear_physical_viewport_states();
        self.physical_viewports_authoritative = false;
        self.engine.set_smoke_level(self.graphics_smoke_level);
        self.engine
            .set_fire_particles(self.display_flags.fire_particles);
        self.engine.set_local_players([self.local_owner]);
        self.engine.set_network_game(self.network.is_some());
        self.engine.set_network_control_mode(self.network.is_some());
        self.engine.set_league_game(self.network_is_league);
        seed_engine_player_info_parameters(
            &mut self.engine,
            &self.network_league_name,
            &self.control_player_infos,
        );
        self.engine
            .set_max_players(i32::try_from(self.network_max_players).unwrap_or(i32::MAX));
        self.engine.set_recording_active(self.recording_enabled);
        self.engine.set_fair_crew_forced(parameter_bootstrap.0);
        self.engine
            .set_allow_debug(saved_allow_debug.unwrap_or(parameter_bootstrap.1));
        self.engine.set_control_rate(parameter_bootstrap.2);
        self.apply_material_library();
        self.input = InputDispatcher::new();
        self.pressed_engine_keys.clear();
        self.scoreboard_tab_raw_pressed = false;
        self.ingame_gui_pointer = None;
        self.ingame_pointer = None;
        self.ingame_mouse_init_centered = false;
        self.ingame_viewport_mouse = None;
        self.ingame_edge_scroll = None;
        self.ingame_mouse_caption = IngameMouseCaptionState::default();
        self.mouse_state = None;
        self.ingame_right_mouse_state = None;
        self.construction_menu_drag = None;
        self.ingame_dragged_objects.clear();
        self.mouse_control_allowed = true;
        self.mouse_control = true;
        self.active_definition_load = None;
        self.active_description_definition_modules.clear();
        let mut recording_scenario_data = None;

        if scenario_info.sandbox {
            let catalog_paths = self.app_paths.clone();
            let crew_paths = self.sandbox_crew_definition_paths.clone();
            let definition_load = match (catalog_paths.as_ref(), crew_paths.as_ref()) {
                (Some(paths), _) => SandboxDefinitionLoad::InstallCatalog(paths),
                (None, Some(paths)) => SandboxDefinitionLoad::InstallCrew(paths),
                (None, None) => SandboxDefinitionLoad::None,
            };
            arm_configured_engine_debug_mode(
                &mut self.engine,
                self.app_paths.as_ref(),
                self.console_mode,
            );
            configure_sandbox_engine(&mut self.engine, definition_load, self.audio.as_mut())
                .context("failed to prepare sandbox engine for saved game")?;
        } else {
            let path = frontend.path.as_ref().ok_or_else(|| {
                anyhow::anyhow!(
                    "saved scenario `{}` does not include a playable path",
                    scenario_info.title
                )
            })?;
            let resolver_paths = cached_app_paths().ok();
            let languages = startup_language_sequence(resolver_paths.as_deref());
            let resolver = InstallDefinitionResolver::new(resolver_paths);
            let scenario_data = match saved_definition_load.as_ref() {
                Some(definition_load) => {
                    load_scenario_with_definition_load(path, &resolver, &languages, definition_load)
                }
                None => Scenario::load_from_path_with_languages(path, &resolver, &languages),
            }
            .with_context(|| {
                format!(
                    "failed to reload scenario `{}` from {}",
                    scenario_info.title,
                    path.display()
                )
            })?;
            self.mouse_control_allowed = !scenario_data.disables_mouse();
            self.mouse_control = self.mouse_control_allowed;
            if self.network.is_none() {
                if let Some(metadata) = scenario_data.lobby_metadata() {
                    let embedded = metadata.embedded_game_parameter_values();
                    let parameters = embedded
                        .as_ref()
                        .unwrap_or_else(|| metadata.game_parameter_defaults());
                    self.engine
                        .set_fair_crew_forced(parameters.fair_crew_forced());
                    self.engine.set_allow_debug(
                        saved_allow_debug.unwrap_or_else(|| parameters.allow_debug()),
                    );
                    self.engine.set_control_rate(parameters.control_rate());
                }
            }
            arm_configured_engine_debug_mode(
                &mut self.engine,
                self.app_paths.as_ref(),
                self.console_mode,
            );
            let sound_samples =
                configure_scenario_sound_samples(self.audio.as_mut(), &scenario_data, path);
            let music_tracks = self
                .audio
                .as_ref()
                .map(AudioContext::available_music_tracks)
                .unwrap_or_default();
            self.engine.configure_sound_samples(sound_samples);
            self.engine.configure_music_tracks(music_tracks);
            // Restoring a save reloads definitions and world resources, but
            // C++ skips Script.Initialize for savegames. The captured engine
            // state below supplies the already-initialized world.
            let apply_result = match save.engine_state.team_configuration {
                Some(configuration) => scenario_data
                    .apply_before_players_for_restore_with_team_configuration(
                        &mut self.engine,
                        configuration,
                    ),
                None => scenario_data.apply_before_players_for_restore(&mut self.engine),
            };
            apply_result.with_context(|| {
                format!(
                    "failed to apply scenario `{}` from {}",
                    scenario_info.title,
                    path.display()
                )
            })?;
            self.active_description_definition_modules =
                raw_definition_description_modules(scenario_data.definition_resource_paths());
            self.active_definition_load = Some(ScenarioDefinitionLoad::Fixed {
                modules: scenario_data
                    .definition_resource_paths()
                    .iter()
                    .map(|path| path_as_legacy_text(path))
                    .collect(),
                definition_root: None,
            });
            recording_scenario_data = Some(scenario_data);
        }

        self.rebuild_definition_sprites();

        if let Some(source_string_table) = save.source_string_table.as_deref() {
            self.engine
                .adopt_loaded_c4_string_table(source_string_table);
        }
        self.engine
            .restore_state(&save.engine_state)
            .context("failed to restore saved engine state")?;
        // DebugMode is process-local and deliberately absent from EngineState.
        // C4Game::Init reapplies AlwaysDebug after the restored AllowDebug
        // parameter is known.
        arm_configured_engine_debug_mode(
            &mut self.engine,
            self.app_paths.as_ref(),
            self.console_mode,
        );
        // InitControl starts fInitial before InitPlayers mutates current
        // takeover rows through SetSavegameResume.
        let loaded_record_prepare_result = recording_scenario_data.as_ref().map(|scenario_data| {
            let initial_source = self
                .recording_enabled
                .then_some(InitialRecordingSource::Loaded {
                    music_enabled: save.runtime_music_enabled.unwrap_or(false),
                    source_save_player_infos: save.source_save_player_infos.as_deref(),
                    source_title_png: save.source_title_png.as_deref(),
                });
            self.prepare_recording_for(&frontend, scenario_data, initial_source, None, None)
        });
        if let Some(clock) = self.network_control_clock.as_mut() {
            clock.set_control_rate(self.engine.control_rate());
        }
        // Savegame runtime players live in C++'s RestorePlayerInfos until
        // current takeover entries absorb their joined ID/flags/team via
        // SetSavegameResume. Do this before RecreatePlayers filters for the
        // JOINED bit: a nonempty current roster may consist entirely of
        // unjoined takeover entries at this point.
        for saved_player in &save.engine_state.players {
            self.control_player_infos.resume_joined_savegame_player(
                saved_player.player_info_id,
                saved_player.team.unwrap_or(0),
                saved_player.no_elimination_check,
            );
        }
        let networked = self.network.is_some();
        let authoritative_player_infos = self.control_player_infos.player_count() != 0;
        if authoritative_player_infos {
            // Savegame takeover keeps the freshly authenticated C4PlayerInfo
            // league fields rather than copying stale saved values.
            seed_engine_player_info_parameters(
                &mut self.engine,
                &self.network_league_name,
                &self.control_player_infos,
            );
        }
        let recreation_players = self
            .control_player_infos
            .recreation_players()
            .into_iter()
            .filter(|(client_id, _)| !networked || self.control_clients.contains(*client_id))
            .collect::<Vec<_>>();
        let recreation_info_ids = recreation_players
            .iter()
            .map(|(_, info_id)| *info_id)
            .collect::<HashSet<_>>();
        let retained_player_numbers = self
            .engine
            .players()
            .filter_map(|player| {
                (!authoritative_player_infos
                    || recreation_info_ids.contains(&player.player_info_id()))
                .then_some(player.id())
            })
            .collect::<Vec<_>>();
        // RecreatePlayers skips unjoined/removed infos and whole missing
        // network-client packets. It does not RemovePlayer their saved
        // objects; the following ValidateOwners phase only orphans the three
        // runtime player-number references.
        self.engine.retain_restored_players(retained_player_numbers);
        // C++ compiles runtime data first, then C4Player::Init overwrites the
        // current client/name and reruns InitControl for every recreated
        // player. Control is always recalculated; MouseControl is only set
        // true, so a compiled true value survives a failed current gate.
        let mut restored_players = self
            .engine
            .players()
            .map(|player| {
                let (preferred_control_set, prefers_mouse) = player.control_preferences();
                let (pref_control_style, pref_auto_context_menu) =
                    player.control_style_preferences();
                (
                    player.id(),
                    player.player_info_id(),
                    player.is_script_player(),
                    player.no_elimination_check(),
                    player.at_client(),
                    player.at_client_name().to_string(),
                    player.name().to_string(),
                    player.mouse_control(),
                    preferred_control_set,
                    prefers_mouse,
                    pref_control_style,
                    pref_auto_context_menu,
                )
            })
            .collect::<Vec<_>>();
        let recreation_order = recreation_players
            .into_iter()
            .enumerate()
            .map(|(index, (_, info_id))| (info_id, index))
            .collect::<HashMap<_, _>>();
        restored_players.sort_by_key(|(number, player_info_id, ..)| {
            (
                recreation_order
                    .get(player_info_id)
                    .copied()
                    .unwrap_or(usize::MAX),
                *number,
            )
        });
        let local_client_id = self
            .network
            .as_ref()
            .and_then(|network| i32::try_from(network.local_client_id()).ok())
            .unwrap_or(0);
        let mut rebound_local_controls = LocalControlRegistry::default();
        let mut local_players = Vec::new();
        let mut restored_primary_owner = None;

        for (
            number,
            player_info_id,
            saved_script_player,
            saved_no_elimination_check,
            saved_at_client,
            saved_at_client_name,
            saved_player_name,
            saved_mouse_control,
            saved_preferred_control_set,
            saved_prefers_mouse,
            saved_pref_control_style,
            saved_pref_auto_context_menu,
        ) in restored_players
        {
            let linked_client_id = self.control_player_infos.client_id_for_info(player_info_id);
            let current_info = self.control_player_infos.get(player_info_id);
            let script_player = current_info
                .map(|info| info.is_script_player())
                .unwrap_or(saved_script_player);
            let player_name = current_info
                .and_then(|info| {
                    [&info.league_account, &info.forced_name, &info.name]
                        .into_iter()
                        .find(|name| !name.is_empty())
                })
                .map(|name| clonk_script::c4_string_from_bytes(name.as_bytes()))
                .unwrap_or(saved_player_name);
            let no_elimination_check = current_info
                .map(clonk_engine::ControlPlayerInfoEntry::no_elimination_check)
                .unwrap_or(saved_no_elimination_check);
            let previous_control = previous_local_controls_by_info
                .get(&player_info_id)
                .copied()
                .or_else(|| {
                    (player_info_id == 0)
                        .then(|| previous_local_controls_by_owner.get(&number).copied())
                        .flatten()
                });
            let (preferred_control_set, prefers_mouse, pref_control_style, pref_auto_context_menu) =
                current_info
                    .and_then(|_| {
                        previous_player_preferences_by_info
                            .get(&player_info_id)
                            .copied()
                            .or_else(|| {
                                (player_info_id == 0)
                                    .then(|| {
                                        previous_player_preferences_by_owner.get(&number).copied()
                                    })
                                    .flatten()
                            })
                    })
                    .unwrap_or((
                        saved_preferred_control_set,
                        saved_prefers_mouse,
                        saved_pref_control_style,
                        saved_pref_auto_context_menu,
                    ));
            let locally_controlled = !script_player
                && match linked_client_id {
                    Some(client_id) if networked => client_id == local_client_id,
                    Some(client_id) => client_id == 0,
                    _ => {
                        previous_control.is_some()
                            || (!networked
                                && (saved_at_client == clonk_engine::PlayerAtClient::HOST
                                    || (player_info_id == 0 && number == previous_local_owner)))
                    }
                };

            let at_client_id = linked_client_id.unwrap_or_else(|| {
                if locally_controlled && !networked {
                    0
                } else {
                    saved_at_client.get()
                }
            });
            let at_client_name = self
                .control_clients
                .state(at_client_id)
                .map(|client| {
                    if !networked && client.name.is_empty() {
                        "Local".to_string()
                    } else {
                        clonk_script::c4_string_from_bytes(client.name.as_bytes())
                    }
                })
                .unwrap_or_else(|| {
                    if locally_controlled && !networked {
                        "Local".to_string()
                    } else {
                        saved_at_client_name
                    }
                });
            let control_init = LocalControlInit {
                owner: number,
                preferred_set: preferred_control_set,
                prefers_mouse,
                gamepads_enabled: self.gamepads_enabled,
                replay: false,
                disable_mouse: !self.mouse_control_allowed,
            };
            let control = if locally_controlled {
                let control = rebound_local_controls
                    .initialize_after_restore(control_init, saved_mouse_control != 0);
                local_players.push(number);
                if previous_primary_info_id == Some(player_info_id)
                    || (previous_primary_info_id.is_none() && number == previous_local_owner)
                {
                    restored_primary_owner = Some(number);
                }
                control
            } else {
                rebound_local_controls.resolve(control_init)
            };

            self.engine
                .reinitialize_player_after_restore(
                    number,
                    clonk_engine::PlayerAtClient::new(at_client_id),
                    at_client_name,
                    player_name,
                    control.runtime_control(),
                    script_player,
                    no_elimination_check,
                    pref_control_style,
                    pref_auto_context_menu,
                )
                .with_context(|| format!("failed to reinitialize restored player {number}"))?;
        }
        rebound_local_controls.finalize_restored_mouse_owner(
            self.engine
                .players()
                .map(|player| (player.id(), player.status())),
        );
        self.local_controls = rebound_local_controls;
        if let Some(owner) = restored_primary_owner.or_else(|| local_players.first().copied()) {
            self.local_owner = owner;
        }
        self.engine.set_local_players(local_players);
        self.engine
            .finalize_restored_players(false)
            .context("failed to run restored player FinalInit")?;
        let restored_music_enabled = self
            .engine
            .reconcile_music_after_restore(save.runtime_music_enabled.unwrap_or(false));

        // Commit presentation resources only after every fallible restore
        // step. A rejected save must not leak its HUD/cursor/palette into the
        // still-visible previous game.
        let offline_player_infos = self
            .network
            .is_none()
            .then(|| std::mem::take(&mut self.control_player_infos));
        self.active_game_graphics = loaded_game_graphics;
        self.ingame_menu_gfx = None;
        self.configure_running_state(scenario_info.label.clone(), scenario_info.fallback_ground);
        if let Some(player_infos) = offline_player_infos {
            self.control_player_infos = player_infos;
        }
        // PlayScenarioMusic one-way enables Game.IsMusicEnabled when RXMusic
        // is on; a configured-off client does not erase a restored true.
        self.runtime_music_enabled |= restored_music_enabled;
        self.active_scenario = Some(frontend.clone());
        if let Some(audio) = self.audio.as_mut() {
            // CompileRuntimeData temporarily applied Game.PlayList, but
            // PlayScenarioMusic always installs its physical DEFAULT filter
            // without changing the saved Game.PlayList string.
            audio.set_music_playlist(None);
        }
        if scenario_info.sandbox {
            self.play_sandbox_audio();
        } else if let Some(path) = frontend.path.as_ref() {
            self.play_scenario_audio(path);
        }
        // C4Game::InitGameFinal starts scenario music before applying the
        // restored Game.iMusicLevel. Scenario configuration installs its
        // default first, so the saved level must win afterward.
        let restored_music_level = self.engine.music_level();
        if let Some(audio) = self.audio.as_mut() {
            audio.set_scenario_music_level(Some(restored_music_level));
        }

        self.mouse_control = self.local_controls.mouse_owner().is_some();
        if let Some(max_players) = self.engine.max_players() {
            self.network_max_players = usize::try_from(max_players).unwrap_or(0);
        }

        self.snapshot = self.engine.snapshot();
        // RecreatePlayers/InitGameFinal still runs with Game.IsRunning false
        // even when quick-load began from a running app.
        self.initialize_physical_viewports(false);
        self.arm_initial_scoreboard_reconcile();
        self.focus_id = save.focus_id;
        if self
            .focus_id
            .and_then(|id| self.snapshot.object(id))
            .is_none()
        {
            self.focus_id = None;
        }
        self.refresh_focus();

        if let Some(prepare_result) = loaded_record_prepare_result {
            if let Err(error) = prepare_result {
                tracing::warn!(%error, "failed to prepare loaded-game recording");
            } else if let Err(error) = self.start_recording(false) {
                tracing::warn!(%error, "failed to start loaded-game recording");
            }
        }
        self.scenario_catalog
            .insert(frontend.identifier.clone(), frontend.clone());

        self.status_text = format!("Loaded {}", scenario_info.title);
        Ok(())
    }

    fn configure_running_state(&mut self, label: String, fallback_ground: i32) {
        self.ingame_mouse_help = false;
        self.ingame_mouse_help_caption = None;
        self.ingame_mouse_caption = IngameMouseCaptionState::default();
        self.mouse_state = None;
        self.ingame_right_mouse_state = None;
        self.construction_menu_drag = None;
        self.ingame_dragged_objects.clear();
        self.ingame_last_left_down = None;
        self.ingame_ignore_left_up = false;
        self.frames_per_second = 0;
        self.frames_since_second = 0;
        self.presentation_stats = PresentationStats::default();
        self.full_speed = false;
        self.frame_skip = 1;
        self.network_stats = Some(NetworkStats::new());
        self.network_stats_clients.clear();
        self.network_stats_players.clear();
        self.scenario_label = label;
        self.fallback_ground = fallback_ground;
        let width = self.graphics.surface().width();
        let height = self.graphics.surface().height();
        let cursor_atlas = self.current_cursor_atlas();
        let hud_graphics = self.current_hud_graphics();
        let game_palette = self.current_game_palette();
        let liquid_animation = self.current_liquid_animation();
        let mut graphics = GraphicsSystem::new(
            width,
            height,
            self.fallback_ground,
            &self.scenario_label,
            self.assets.font_arc(),
            Arc::clone(&self.sprite_cache),
            cursor_atlas,
            hud_graphics,
        );
        graphics.inherit_liquid_animation_cycle(&self.graphics);
        graphics.inherit_runtime_sprite_filtering(&self.graphics);
        graphics.inherit_advanced_renderer_config(&self.graphics);
        graphics.inherit_cursor_tiers(&self.graphics);
        // Particle definitions live in Game.Particles independently of the
        // viewport (oracle-src-pinned src/C4Particles.cpp:118-189). Rebind
        // their draw resources when entering the running presentation just
        // like a viewport recreation.
        graphics.set_particle_sprites(Arc::new(particle_sprite_map(&self.engine)));
        self.graphics = graphics;
        self.graphics
            .set_clonk_fonts(self.assets.clonk_fonts.clone());
        self.graphics.set_game_palette(game_palette);
        self.graphics.set_liquid_animation(liquid_animation);
        self.graphics.surface_mut().fill(Color::opaque(12, 24, 40));
        self.graphics.set_sky(self.sky.clone());
        self.graphics
            .set_material_texture_surfaces(Arc::clone(&self.material_texture_images));
        self.graphics
            .set_material_render_info(Arc::clone(&self.material_render_info));
        self.frame_text.clear();
        self.status_text.clear();
        self.energy_fraction = 0.0;
        self.runtime_music_enabled = self
            .audio
            .as_ref()
            .is_some_and(|audio| audio.options.music_enabled);
        self.sync_checks.clear();
        self.network_ticks.clear();
        self.network_sync.clear();
        self.offline_control_input.clear();
        self.offline_halt_count = 0;
        self.network_control_running = self.network.is_none();
        self.runtime_network_status_barrier = None;
        if self.network.is_none() {
            self.control_clients = initial_control_clients(None, None);
            self.control_player_infos = ControlPlayerInfoRegistry::default();
            self.clear_blocking_resource_wait();
            self.admission_resources.clear();
            self.host_local_alternate_colors_by_resource.clear();
            self.host_local_player_info_ids.clear();
        }
        self.menu_state.set_pointer_position(None);
        self.object_menu = None;
        self.ingame_menu.clear();
        self.script_menu_presentations.clear();
        self.game_over_handled = false;
        self.pending_league_end = None;
        self.clear_pending_league_player_auth();
        self.runtime_help_visible = false;
        self.runtime_flash_message = None;
        self.runtime_client_list = None;
        self.running_dialog_stack.clear();
        self.running_active_dialog = None;
        self.runtime_client_list_consumed_keys.clear();
        self.runtime_key_config_cache = OnceLock::new();
        let _ = self.runtime_key_config_cache.set(
            load_runtime_global_key_config(self.app_paths.as_ref())
                .map_err(|error| format!("{error:#}")),
        );
        self.scoreboard_dialog = None;
        self.scoreboard_initial_reconcile_pending = false;
        self.scoreboard_close_pointer_capture = false;
        self.scoreboard_runtime = ScoreboardDialogRuntime::default();
        self.network_chart_dialog = None;
        self.network_chart_consumed_keys.clear();
        self.network_chart_pointer_capture = false;
        self.reset_runtime_default_dialog_order();
        self.running_gui_mouse_owned = false;
        self.running_world_mouse_owned = true;
        self.mode = AppMode::Running;
        self.reconcile_network_stats_series();
        // Startup hint + join log line for the HUD. Game.Time is owned by the
        // engine and pulsed by the event loop's one-second accumulator.
        // ShowStartup is set on player init (C4Player.cpp:1735).
        self.show_startup_hint = true;
        // C4MessageBoard::Init reloads the bool-typed MsgBoard setting for
        // every game. A runtime multi-line count therefore collapses back to
        // ordinary one-line mode on the next initialization.
        self.running_chat = None;
        self.game_option_input_dialog = None;
        self.league_signup_dialog = None;
        self.cancelled_league_signup_continuation = None;
        self.league_signup_consumed_keys.clear();
        self.league_signup_pointer_capture = false;
        self.league_signup_pointer_position = None;
        let line_height = self.graphics.message_board_line_height();
        self.message_board.initialize(
            load_message_board_enabled(self.app_paths.as_ref()),
            line_height,
        );
        // C4PlayerList::JoinNew logs IDS_PRC_JOINPLR "Player join: %s"
        // (C4PlayerList.cpp:281, LanguageUS.txt:1222).
        let join_line = self
            .engine
            .snapshot()
            .players
            .iter()
            .find(|state| state.id == self.local_owner)
            .map(|state| player_join_board_line(&state.name));
        if let Some(line) = join_line {
            let line = self.timestamp_log_line(line);
            self.enqueue_control_message_board_line(line);
        }
    }

    fn apply_focus_selection(&mut self) {
        // A join already selected the hi-rank cursor (AdjustCursorCommand,
        // C4Player.cpp:1235-1258): adopt it rather than stacking a second
        // crew selection on top.
        if let Some(cursor) = self
            .snapshot
            .crew_selection
            .get(&self.local_owner)
            .and_then(|selection| selection.cursor)
            .filter(|cursor| {
                self.snapshot
                    .object(*cursor)
                    .map(|object| object.status.is_active())
                    .unwrap_or(false)
            })
        {
            self.focus_id = Some(cursor);
            self.focus_snapshot = None;
            return;
        }
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

#[cfg(test)]
fn compute_mix_values_for_with_rendered_audibility(
    volume: u8,
    target_id: Option<ObjectId>,
    custom_falloff: Option<i32>,
    snapshot: &SimulationSnapshot,
    viewports: &[ActiveViewportProjection],
    rendered_object_audibility: &HashMap<ObjectId, CachedObjectAudibilityMix>,
) -> (f32, f32) {
    let base_volume = (f32::from(volume) / 100.0).clamp(0.0, 1.0);
    let Some(target_id) = target_id else {
        return (base_volume, 0.0);
    };
    let Some(target) = snapshot.object(target_id) else {
        return (base_volume, 0.0);
    };
    let origin_mix = compute_positional_mix_values(target.position, snapshot, viewports);
    let (audibility, pan) =
        cached_attached_object_mix_values(target, rendered_object_audibility).unwrap_or(origin_mix);
    (
        base_volume * adjusted_audibility(audibility, custom_falloff),
        pan,
    )
}

/// `C4Object::GetAudibility` (C4Object.cpp:5622-5628): the drawn Audible /
/// AudiblePan pair is authoritative until the next completed graphics pass
/// runs `ResetAudibility` (C4GraphicsSystem.cpp:158-159). The object moving
/// away from where it was drawn never invalidates it, so a sound started
/// after the object has already moved this frame still mixes at the drawn
/// audibility rather than falling back to a live listener distance.
fn cached_attached_object_mix_values(
    target: &ObjectSnapshot,
    rendered_object_audibility: &HashMap<ObjectId, CachedObjectAudibilityMix>,
) -> Option<(u8, f32)> {
    rendered_object_audibility
        .get(&target.id)
        .map(|cached| (cached.audibility, cached.pan as f32 / 100.0))
}

fn reduce_rendered_object_audibility(
    calls: &RenderedObjectAudibilityCalls,
    snapshot: &SimulationSnapshot,
    viewports: &[ActiveViewportProjection],
    previous: &HashMap<ObjectId, CachedObjectAudibilityMix>,
) -> HashMap<ObjectId, CachedObjectAudibilityMix> {
    // Inactive MODE_Object targets are not members of Game.Objects, so the
    // frame-start ResetAudibility loop never clears them. Preserve their
    // cache (and seed parallax pan from it) until movement invalidates it.
    let mut reduced = previous
        .iter()
        .filter_map(|(&object_id, &cached)| {
            snapshot
                .object(object_id)
                .is_some_and(|target| {
                    target.status == clonk_engine::ObjectStatus::Inactive
                        && target.position == cached.object_position
                })
                .then_some((object_id, cached))
        })
        .collect::<HashMap<_, _>>();
    reduced.reserve(calls.len());
    for (&object_id, calls) in calls {
        let Some(target) = snapshot.object(object_id) else {
            continue;
        };
        let initial_pan = (target.status == clonk_engine::ObjectStatus::Inactive)
            .then(|| reduced.get(&object_id).map(|cached| cached.pan))
            .flatten()
            .unwrap_or(0);
        let mut mix: Option<(u8, i32)> = None;
        for call in calls {
            let pan = mix.map_or(initial_pan, |(_, pan)| pan);
            mix = Some(match *call {
                RenderedAudibilityCall::World { point } => {
                    compute_raw_positional_mix_values(point, snapshot, viewports)
                }
                RenderedAudibilityCall::Parallax {
                    point,
                    rendered_center,
                } => (
                    positional_audibility(point, rendered_center),
                    pan.wrapping_add(point.x.wrapping_sub(rendered_center.x) / 5)
                        .clamp(-100, 100),
                ),
            });
        }
        if let Some((audibility, pan)) = mix {
            reduced.insert(
                object_id,
                CachedObjectAudibilityMix {
                    object_position: target.position,
                    audibility,
                    pan,
                },
            );
        }
    }
    reduced
}

fn adjusted_audibility(audibility: u8, custom_falloff: Option<i32>) -> f32 {
    const AUDIBILITY_RADIUS: i32 = 700;

    let mut audibility = i32::from(audibility);
    if let Some(falloff_distance) = custom_falloff.filter(|distance| *distance != 0) {
        audibility = 100 + (audibility - 100) * AUDIBILITY_RADIUS / falloff_distance;
    }
    audibility.clamp(0, 100) as f32 / 100.0
}

/// `C4GraphicsSystem::GetAudibility`. `StartSoundEffectAt` evaluates this
/// once; object-bound instances evaluate it again on every sound-system tick.
fn compute_positional_mix_values(
    source: Vector2,
    snapshot: &SimulationSnapshot,
    viewports: &[ActiveViewportProjection],
) -> (u8, f32) {
    let (volume, pan) = compute_raw_positional_mix_values(source, snapshot, viewports);
    (volume, pan as f32 / 100.0)
}

fn compute_raw_positional_mix_values(
    source: Vector2,
    snapshot: &SimulationSnapshot,
    viewports: &[ActiveViewportProjection],
) -> (u8, i32) {
    let mut volume = 0u8;
    let mut pan = 0i32;

    for viewport in viewports {
        let center = Vector2::new(
            viewport.target_x.wrapping_add(viewport.logical_width / 2),
            viewport.target_y.wrapping_add(viewport.logical_height / 2),
        );
        let listener = snapshot
            .players
            .iter()
            .find(|player| player.id == viewport.owner)
            .and_then(|player| {
                player
                    .view_cursor
                    .and_then(|object| snapshot.object(object))
                    .or_else(|| {
                        player
                            .view_target
                            .and_then(|object| snapshot.object(object))
                    })
                    .or_else(|| player.cursor.and_then(|object| snapshot.object(object)))
            })
            .map(|object| object.position)
            .unwrap_or(center);
        volume = volume.max(positional_audibility(source, listener));
        pan = pan.wrapping_add((source.x.wrapping_sub(center.x)) / 5);
    }

    (volume, pan.clamp(-100, 100))
}

fn positional_audibility(source: Vector2, listener: Vector2) -> u8 {
    const AUDIBILITY_RADIUS: i32 = 700;
    let distance = c4_audio_distance(source, listener);
    (100 - 100 * distance / AUDIBILITY_RADIUS).clamp(0, 100) as u8
}

/// `Distance` (C4Math.cpp): integer square root with the post-sqrt
/// correction used by GetAudibility's integer arithmetic.
fn c4_audio_distance(first: Vector2, second: Vector2) -> i32 {
    let dx = i64::from(first.x) - i64::from(second.x);
    let dy = i64::from(first.y) - i64::from(second.y);
    let squared = dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy));
    if squared < 0 {
        return -1;
    }
    let mut distance = (squared as f64).sqrt() as i64;
    if distance.wrapping_mul(distance) < squared {
        distance += 1;
    }
    if distance.wrapping_mul(distance) > squared {
        distance -= 1;
    }
    distance as i32
}

fn walker_script() -> &'static str {
    r#"
global func Initialize(state, random) { return 0; }
global func Step(state, frame, random) { return 0; }
"#
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
