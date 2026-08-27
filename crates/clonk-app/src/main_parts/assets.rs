//! `main.rs` — frontend asset loading, key names and the integration-test entry points.
//!
//! A contiguous slice moved verbatim from the crate root; it stays part of
//! the same binary crate, re-exported from `main.rs` so every path resolves.

use super::*;
use crate::settings::CompatProfile;
use clonk_frontend::clonk_fonts::NativeFontSizes;

const PLAYER_OWNER: i32 = 1;
pub(crate) const STARTUP_FRAME_INTERVAL: Duration = Duration::from_millis(16);
pub(crate) const INGAME_FRAME_INTERVAL: Duration = Duration::from_millis(28);
// C4Config defaults Graphics.MaxRefreshDelay to 30 ms. With the native 28 ms
// game timer, C4Application therefore schedules one graphics opportunity per
// simulation interval (src/C4Config.cpp:481-485; C4Application.cpp:510-520).
pub(crate) const DEFAULT_MAX_REFRESH_DELAY_MS: u64 = 30;
pub(crate) const MAX_ACCUMULATED_TIME: Duration = Duration::from_millis(250); // clamp backlog to avoid runaway catch-up
/// Share of the wall clock a catch-up burst leaves to drawing. C++ reserves
/// nothing here — `Game.DoSkipFrame` only thins whole graphics opportunities,
/// so a machine slow enough to spend an entire application pass inside
/// `advance_simulation_pass` never repaints at all. Spring's game controller
/// keeps a comparable slice for drawing while it fast-forwards; this is a
/// deliberate presentation-only divergence for slow hardware.
///
/// `C4Application::Execute` runs at most one `Game.Execute()` per pass and
/// draws in the same pass (LegacyClonk 7d43b47 src/C4Application.cpp:463-476,
/// 451-478), so C++ hands drawing a slot every pass and an overloaded machine
/// runs the *game* slow instead. Here one pass can drain the whole clamped
/// 250 ms backlog (`MAX_ACCUMULATED_TIME`) without returning to the event
/// loop, and the ported `AutoFrameSkip` cannot help — it is a one-shot latch
/// consumed at a graphics opportunity that never arrives. Arithmetic from
/// these constants (no Pi was in the loop): at 35 ms per simulation frame and
/// a 10 ms graphics pass a pass used to drain 9 frames / ~315 ms with no
/// repaint (~3 Hz, arbitrarily worse under `/fast N`), while a budget of
/// `max(28 ms, 5.67 x 10 ms) = 57 ms` runs ~2 frames and yields, putting
/// repaints near 14 Hz for about 15 % of the CPU.
///
/// Determinism is unaffected and must stay that way: the budget is checked
/// only *after* a frame executed, unspent backlog stays in the accumulator,
/// so the same simulation frames run in the same order, just spread over more
/// application passes. Nothing here is visible to script or to the control
/// stream.
pub(crate) const RENDER_RESERVE_PERCENT: u32 = 15;
/// Hard repaint floor (~2 Hz). `/fast N`, the network catch-up divisor and a
/// long catch-up burst can each suppress every graphics opportunity for an
/// unbounded stretch; this is the only guarantee the window still updates.
pub(crate) const MAX_TIME_BETWEEN_RENDERS: Duration = Duration::from_millis(500);
pub(crate) const PRESENTATION_BENCHMARK_ENV: &str = "LC_APP_PRESENTATION_BENCHMARK_SECONDS";
pub(crate) const PRESENTATION_BENCHMARK_PLAYER_TEAMS_ENV: &str =
    "LC_APP_PRESENTATION_BENCHMARK_PLAYER_TEAMS";
pub(crate) const INPUT_LATENCY_BENCHMARK_INTERVAL_ENV: &str =
    "LC_APP_PRESENTATION_BENCHMARK_INPUT_INTERVAL_MS";
pub(crate) const PRESENTATION_BENCHMARK_ASSERT_NATIVE_TICK_ENV: &str =
    "LC_APP_PRESENTATION_BENCHMARK_ASSERT_NATIVE_TICK";
pub(crate) const PRESENTATION_BENCHMARK_KEEP_RUNNING_ENV: &str =
    "LC_APP_PRESENTATION_BENCHMARK_KEEP_RUNNING";
pub(crate) const PRESENTATION_BENCHMARK_WARMUP: Duration = Duration::from_secs(2);
pub(crate) const SAVE_THUMBNAIL_WIDTH: u32 = 200;
pub(crate) const SAVE_THUMBNAIL_HEIGHT: u32 = 150;
/// The same size the retained GPU renderer reduces a presentation to when a
/// frame is read back only to become a save thumbnail.
pub(crate) const SAVE_THUMBNAIL_EXTENT: [u32; 2] = [SAVE_THUMBNAIL_WIDTH, SAVE_THUMBNAIL_HEIGHT];
pub(crate) static LOBBY_PRELOAD_SERIAL: AtomicU64 = AtomicU64::new(0);
pub(crate) const NETWORK_CONTROL_OVERFLOW_LIMIT: u32 = 3;
pub(crate) const NETWORK_RENDER_SKIP_BEHIND: u32 = 25;

/// Simulation frames that may pass without drawing anything while catching up.
///
/// C++ thins rendering during catch-up by `(behind + 15) / 20`, which at a large
/// backlog means drawing one frame in twenty or worse — and the port's pass
/// coalescing can stack several such passes, so a client recovering from a long
/// stall can go a noticeable time with a completely static picture. That is
/// indistinguishable from a hang to the person watching it.
///
/// Spring solves the same problem with an explicit floor, pinning draw to 2 Hz
/// while fast-forwarding rather than letting the sim have everything. 18 frames
/// is that 2 Hz at the 28 ms in-game tick. Counted in frames rather than wall
/// time so the behaviour is deterministic and testable.
pub(crate) const NETWORK_RENDER_FLOOR_FRAMES: u32 = 18;
pub(crate) const GAME_SECOND_INTERVAL: Duration = Duration::from_secs(1);
pub(crate) const STARTUP_NETWORK_MIN_REFRESH_INTERVAL: Duration = Duration::from_secs(1);
pub(crate) const STARTUP_NETWORK_QUERY_ERROR_LIFETIME: Duration = Duration::from_secs(10);
/// `C4NetDeactivationDelay` is measured in simulation frames, despite the
/// native header's historical "ticks" comment (`src/C4Network2.h:57-60`).
const NETWORK_CLIENT_DEACTIVATION_DELAY: i32 = 500;
pub(crate) const GAME_MUSIC_FADE_OUT_MS: u32 = 2_000;
pub(crate) const FALLBACK_SCENARIO_TITLE: &str = "Rust Sandbox";
pub(crate) const DEFAULT_GROUND_HEIGHT: i32 = 360;
pub(crate) const DEFAULT_SCENARIO_MAX_PLAYERS: usize = 12;
pub(crate) const CLASSIC_ENGINE_BUILD: i32 = 362;
pub(crate) const BACK_ENTRY_IDENTIFIER: &str = "__lc_menu_back";
pub(crate) const OFFICIAL_LEAGUE_SERVER: &str = "https://league.clonkspot.org";

pub(crate) const BACK_ENTRY_TITLE: &str = "← Back";
pub(crate) const SAVE_DIR_NAME: &str = "Savegames";
pub(crate) const DEFAULT_CLASSIC_SAVE_GAME_FOLDER: &str = "Savegames.c4f";
pub(crate) const QUICK_SAVE_FILE: &str = "quicksave.lcsave";
pub(crate) const SAVE_FILE_VERSION: SaveFileVersion = SaveFileVersion::new(1, 0, 0);
pub(crate) const MOUSE_DRAG_THRESHOLD: f32 = 6.0;
/// C4Menu::DoDragging starts a menu-element drag at `>= C4MC_DragSensitivity`,
/// unlike world-origin mouse drags, which use `> C4MC_DragSensitivity`.
pub(crate) const MENU_DRAG_THRESHOLD: f32 = 5.0;
/// The SDL/X11 application paths synthesize LeftDouble when the second press
/// arrives less than 400 ms after the first (C4FullScreen.cpp:327-350;
/// C4Viewport.cpp:657-676).
pub(crate) const CPP_DOUBLE_CLICK_INTERVAL: Duration = Duration::from_millis(400);

pub(crate) fn classic_press_is_double_click(
    last_click: &mut Option<Instant>,
    now: Instant,
) -> bool {
    if last_click
        .is_some_and(|last| now.saturating_duration_since(last) < CPP_DOUBLE_CLICK_INTERVAL)
    {
        *last_click = None;
        true
    } else {
        *last_click = Some(now);
        false
    }
}

pub(crate) static APP_PATH_CACHE: Mutex<Option<std::result::Result<Arc<AppPaths>, PathsError>>> =
    Mutex::new(None);

pub(crate) fn sprite_map_key(definition_id: &str, graphics_name: Option<&str>) -> String {
    match graphics_name {
        Some(name) if !name.is_empty() => {
            format!("{}::{}", definition_id, name.to_ascii_lowercase())
        }
        _ => definition_id.to_string(),
    }
}

pub(crate) fn particle_sprite_map(engine: &Engine) -> HashMap<String, ParticleRenderDefinition> {
    engine
        .particle_render_catalog()
        .iter()
        .filter_map(|definition| {
            let graphics = definition.graphics.as_ref()?;
            Some((
                definition.core.name.clone(),
                ParticleRenderDefinition {
                    image: ImageData::from_arc(
                        graphics.image.width(),
                        graphics.image.height(),
                        graphics.image.clone_pixels(),
                    ),
                    facet: ParticleFacet::new(
                        graphics.facet.x,
                        graphics.facet.y,
                        graphics.facet.width,
                        graphics.facet.height,
                    ),
                    length: definition.length,
                    aspect: definition.aspect,
                    core: definition.core.clone(),
                    draw_proc: definition.draw_proc,
                },
            ))
        })
        .collect()
}

#[derive(Debug, Parser)]
#[command(name = "clonk-app", about = "Clonk Rust runtime", version)]
pub(crate) struct Cli {
    #[arg(
        long = "config",
        value_name = "PATH",
        help = "Use PATH for all configuration reads and writes"
    )]
    pub(crate) config_file: Option<std::path::PathBuf>,

    #[arg(
        long = "test-load",
        value_name = "PATH",
        help = "Test scenario loading without starting the UI"
    )]
    pub(crate) test_load: Option<std::path::PathBuf>,

    #[arg(
        long = "integration-test",
        value_name = "PATH",
        help = "Run full scenario integration test (load, apply, start, run frames)"
    )]
    pub(crate) integration_test: Option<std::path::PathBuf>,

    #[arg(
        long = "test-frames",
        value_name = "N",
        default_value_t = 60,
        help = "Number of frames to run during integration test"
    )]
    pub(crate) test_frames: u32,

    #[arg(long = "host", value_name = "ADDR", conflicts_with = "join")]
    host: Option<String>,

    #[arg(long = "join", value_name = "ADDR")]
    join: Option<String>,

    #[arg(long = "player-owner", value_name = "OWNER", default_value_t = PLAYER_OWNER)]
    pub(crate) player_owner: i32,

    #[arg(long = "player-name", value_name = "NAME", default_value = "Player")]
    pub(crate) player_name: String,

    #[arg(
        long = "sandbox",
        help = "Boot straight into the built-in sandbox scenario (skips the menu); useful for capturing the in-game scene"
    )]
    pub(crate) sandbox: bool,

    #[arg(
        long = "display-server",
        value_name = "BACKEND",
        value_enum,
        default_value = "auto",
        help = "Linux and BSD: which display server the window uses. `auto` keeps winit's Wayland-first order unless Steam Input is running on a Wayland session, whose XTEST controller input only ever reaches X11 clients."
    )]
    pub(crate) display_server: crate::display_backend::DisplayServerPreference,

    #[arg(
        long = "headless",
        help = "Run as a dedicated server: no window, no render device and no sound, driven by the stdin console. Equivalent to [Graphics] Engine=3 (NoGfx), which is honoured on its own."
    )]
    pub(crate) headless: bool,

    #[arg(
        long = "headed-surface-smoke",
        value_name = "REPORT.json",
        conflicts_with_all = [
            "headless",
            "test_load",
            "integration_test",
            "host",
            "join",
            "sandbox",
            "dump_frame",
            "dump_menu_frame",
            "classic_arguments"
        ],
        hide = true
    )]
    pub(crate) headed_surface_smoke: Option<std::path::PathBuf>,

    #[arg(
        long = "software-present-smoke",
        value_name = "REPORT.json",
        conflicts_with_all = [
            "headless",
            "test_load",
            "integration_test",
            "host",
            "join",
            "sandbox",
            "dump_frame",
            "dump_menu_frame",
            "headed_surface_smoke",
            "classic_arguments"
        ],
        hide = true
    )]
    pub(crate) software_present_smoke: Option<std::path::PathBuf>,

    #[arg(
        long = "dump-frame",
        value_name = "PATH",
        help = "Headless: boot the sandbox, advance --test-frames frames, render one in-game frame to a PNG at PATH, and exit (no window). For visual rendering-parity checks."
    )]
    pub(crate) dump_frame: Option<std::path::PathBuf>,

    #[arg(
        long = "dump-menu-frame",
        value_name = "PATH",
        help = "Headless: boot to the startup main menu, render one frame to a PNG at PATH, and exit (no window). For menu rendering-parity checks."
    )]
    pub(crate) dump_menu_frame: Option<std::path::PathBuf>,

    #[arg(
        long = "menu-view",
        value_name = "VIEW",
        default_value = "main",
        help = "Startup view for --dump-menu-frame: main, scenarios, net, plrsel, options, or about."
    )]
    pub(crate) menu_view: String,

    /// Compatibility arguments accepted by C4Application::DoInit and
    /// C4Game::ParseCommandLine. This is an ordinary variadic positional (not
    /// a trailing var-arg), so modern `--...` switches may still follow a
    /// classic file, URL, or slash argument and retain clap's validation.
    #[arg(value_name = "CLASSIC_ARG", num_args = 0..)]
    pub(crate) classic_arguments: Vec<OsString>,
}

/// Explicit developer-only opt-in for the Rust FRAME/POS/VEL HUD.
pub(crate) const LC_APP_HUD_DEBUG_ENV: &str = "LC_APP_HUD_DEBUG";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DebugHudLaunch {
    Interactive,
    ParityCapture,
    Compatibility,
}

/// Classify launch surfaces before constructing app state.
///
/// Compatibility arguments and every non-interactive test/capture route are
/// fail-closed for the developer HUD. They must retain the classic overlays
/// and produce parity evidence without Rust-only presentation text.
pub(crate) fn debug_hud_launch(cli: &Cli) -> DebugHudLaunch {
    if !cli.classic_arguments.is_empty() {
        DebugHudLaunch::Compatibility
    } else if cli.test_load.is_some()
        || cli.integration_test.is_some()
        || cli.dump_frame.is_some()
        || cli.dump_menu_frame.is_some()
        || cli.headless
        || cli.headed_surface_smoke.is_some()
        || cli.software_present_smoke.is_some()
    {
        DebugHudLaunch::ParityCapture
    } else {
        DebugHudLaunch::Interactive
    }
}

/// Return whether the explicit HUD opt-in is valid for this launch.
///
/// `LC_APP_HUD_DEBUG=1` is intentionally the whole contract: it is accepted
/// only in a debug build, for an interactive launch, and when no forensic
/// capture environment is active. All other values and contexts are off.
pub(crate) fn debug_hud_enabled(
    requested: Option<&str>,
    developer_build: bool,
    launch: DebugHudLaunch,
    capture_environment: bool,
) -> bool {
    developer_build
        && requested == Some("1")
        && launch == DebugHudLaunch::Interactive
        && !capture_environment
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ClassicCommandLine {
    pub(crate) scenario: Option<PathBuf>,
    pub(crate) player_files: Vec<PathBuf>,
    pub(crate) definition_files: Vec<PathBuf>,
    pub(crate) incoming_update: Option<PathBuf>,
    pub(crate) record_stream: Option<PathBuf>,
    pub(crate) direct_join: Option<String>,
    pub(crate) network_active: Option<bool>,
    pub(crate) master_server_signup: Option<bool>,
    pub(crate) league_server_signup: Option<bool>,
    pub(crate) lobby_timeout: Option<Option<u32>>,
    pub(crate) observe: bool,
    pub(crate) runtime_join: Option<bool>,
    pub(crate) update_requested: bool,
    /// `/compatprofile:<token>`: the operating mode for **this run only**.
    ///
    /// Kept apart from the persisted `General.CompatProfile` key on purpose —
    /// a launch override is a property of the run and is never written back,
    /// so launching once in compatibility mode does not change what the player
    /// finds in their configuration afterwards.
    pub(crate) compat_profile: Option<CompatProfile>,
    pub(crate) fair_crew: Option<bool>,
    pub(crate) record_dump: Option<String>,
    pub(crate) startup_screen: Option<String>,
    pub(crate) tcp_port: Option<u16>,
    pub(crate) udp_port: Option<u16>,
    pub(crate) password: Option<String>,
    pub(crate) comment: Option<String>,
    pub(crate) console: bool,
    pub(crate) config_file: Option<PathBuf>,
    pub(crate) verbose: bool,
    pub(crate) language: Option<String>,
}

fn classic_argument_value<'a>(argument: &'a str, prefix: &str) -> Option<&'a str> {
    let candidate = argument.get(..prefix.len())?;
    candidate
        .eq_ignore_ascii_case(prefix)
        .then(|| &argument[prefix.len()..])
}

/// `atoi` semantics for `/client:N`: the longest leading decimal prefix, zero
/// when there is none. C++ feeds the result straight into its port formula
/// (C4Game.cpp:3298-3301), so a non-numeric index behaves like index 0.
fn classic_atoi(value: &str) -> i64 {
    let value = value.trim_start();
    let (sign, digits) = match value.strip_prefix('-') {
        Some(rest) => (-1, rest),
        None => (1, value.strip_prefix('+').unwrap_or(value)),
    };
    let magnitude: i64 =
        digits
            .bytes()
            .take_while(u8::is_ascii_digit)
            .fold(0_i64, |accumulated, byte| {
                accumulated
                    .saturating_mul(10)
                    .saturating_add(i64::from(byte - b'0'))
            });
    sign * magnitude
}

/// `#ifndef NDEBUG` in `C4Game::ParseCommandLine` (C4Game.cpp:3288-3304). The
/// two shortcuts stand up a local lobby on predictable ports; release builds
/// never see them, matching the C++ gate.
pub(crate) const DEBUG_CLASSIC_SHORTCUTS: bool = cfg!(debug_assertions);

/// `/client:N` port pair: TCP `11112 + 2 * (N + 1)`, UDP `11113 + 2 * (N + 1)`
/// (C4Game.cpp:3300-3301).
pub(crate) fn classic_debug_client_ports(index: &str) -> (u16, u16) {
    let offset = classic_atoi(index).saturating_add(1).saturating_mul(2);
    (
        i64::from(11_112_u16)
            .saturating_add(offset)
            .clamp(0, i64::from(u16::MAX)) as u16,
        i64::from(11_113_u16)
            .saturating_add(offset)
            .clamp(0, i64::from(u16::MAX)) as u16,
    )
}

fn classic_port(value: &str) -> u16 {
    value
        .trim()
        .parse::<i64>()
        .unwrap_or(0)
        .clamp(0, i64::from(u16::MAX)) as u16
}

fn classic_path_extension(path: &Path) -> Option<&str> {
    let path = path.to_str()?;
    path.rsplit(std::path::MAIN_SEPARATOR)
        .next()
        .and_then(|basename| basename.rsplit_once('.').map(|(_, extension)| extension))
}

pub(crate) fn parse_classic_command_line(arguments: &[OsString]) -> ClassicCommandLine {
    let mut parsed = ClassicCommandLine::default();

    for argument in arguments {
        let path = Path::new(argument);
        let extension = if path.to_str().is_some() {
            classic_path_extension(path)
        } else {
            path.extension().and_then(|extension| extension.to_str())
        };
        if extension.is_some_and(|extension| extension.eq_ignore_ascii_case("c4s")) {
            parsed.scenario = Some(path.to_path_buf());
            continue;
        }
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("Scenario.txt"))
        {
            parsed.scenario = Some(
                path.parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                    .unwrap_or_else(|| Path::new("."))
                    .to_path_buf(),
            );
            continue;
        }
        if extension.is_some_and(|extension| extension.eq_ignore_ascii_case("c4p")) {
            parsed.player_files.push(path.to_path_buf());
            continue;
        }
        if extension.is_some_and(|extension| extension.eq_ignore_ascii_case("c4d")) {
            parsed.definition_files.push(path.to_path_buf());
            continue;
        }
        if extension.is_some_and(|extension| extension.eq_ignore_ascii_case("c4u")) {
            parsed.incoming_update = Some(path.to_path_buf());
            continue;
        }
        if extension.is_some_and(|extension| extension.eq_ignore_ascii_case("c4r")) {
            parsed.record_stream = Some(path.to_path_buf());
        }

        let Some(argument) = argument.to_str() else {
            // Native command switches are byte strings, but a non-Unicode OS
            // value can still be an unknown positional argument. Match the
            // classic parser's permissive fallback instead of rejecting it.
            continue;
        };
        if argument.eq_ignore_ascii_case("/network") {
            parsed.network_active = Some(true);
        } else if argument.eq_ignore_ascii_case("/nonetwork") {
            parsed.network_active = Some(false);
        } else if argument.eq_ignore_ascii_case("/signup") {
            parsed.network_active = Some(true);
            parsed.master_server_signup = Some(true);
        } else if argument.eq_ignore_ascii_case("/nosignup") {
            parsed.master_server_signup = Some(false);
            parsed.league_server_signup = Some(false);
        } else if argument.eq_ignore_ascii_case("/league") {
            parsed.network_active = Some(true);
            parsed.master_server_signup = Some(true);
            parsed.league_server_signup = Some(true);
        } else if argument.eq_ignore_ascii_case("/noleague") {
            parsed.league_server_signup = Some(false);
        } else if argument.eq_ignore_ascii_case("/lobby") {
            parsed.network_active = Some(true);
            parsed.lobby_timeout = Some(None);
        } else if let Some(value) = classic_argument_value(argument, "/lobby:") {
            let timeout = value.trim().parse::<i64>().unwrap_or(0).max(0);
            parsed.network_active = Some(true);
            parsed.lobby_timeout = Some(Some(timeout.min(i64::from(u32::MAX)) as u32));
        } else if argument.eq_ignore_ascii_case("/observe") {
            parsed.network_active = Some(true);
            parsed.observe = true;
        } else if argument.eq_ignore_ascii_case("/runtimejoin") {
            parsed.runtime_join = Some(true);
        } else if argument.eq_ignore_ascii_case("/noruntimejoin") {
            parsed.runtime_join = Some(false);
        } else if argument.eq_ignore_ascii_case("/update") {
            parsed.update_requested = true;
        } else if let Some(value) = classic_argument_value(argument, "/compatprofile:") {
            // An unrecognised token leaves the override unset rather than
            // guessing, so a typo cannot enrol the run in a promise the port
            // would then have to keep.
            parsed.compat_profile = CompatProfile::parse(value);
        } else if argument.eq_ignore_ascii_case("/faircrew")
            || argument.eq_ignore_ascii_case("/ncrw")
        {
            parsed.fair_crew = Some(true);
        } else if argument.eq_ignore_ascii_case("/trainedcrew")
            || argument.eq_ignore_ascii_case("/ucrw")
        {
            parsed.fair_crew = Some(false);
        } else if let Some(value) = classic_argument_value(argument, "/join:") {
            parsed.direct_join = Some(value.to_string());
            parsed.network_active = Some(true);
        } else if let Some(value) = classic_argument_value(argument, "clonk:") {
            let target = value.trim_matches('/');
            if target.eq_ignore_ascii_case("update") {
                parsed.direct_join = None;
                parsed.update_requested = true;
            } else {
                parsed.direct_join = Some(target.to_string());
                parsed.network_active = Some(true);
            }
        } else if DEBUG_CLASSIC_SHORTCUTS && argument.eq_ignore_ascii_case("/host") {
            // C4Game.cpp:3288-3296: network + lobby on the fixed pair, with
            // both signup modes off.
            parsed.network_active = Some(true);
            parsed.lobby_timeout = Some(None);
            parsed.tcp_port = Some(11_112);
            parsed.udp_port = Some(11_113);
            parsed.master_server_signup = Some(false);
            parsed.league_server_signup = Some(false);
        } else if let Some(value) =
            classic_argument_value(argument, "/client:").filter(|_| DEBUG_CLASSIC_SHORTCUTS)
        {
            // C4Game.cpp:3297-3303: join localhost with a lobby on the
            // index-derived pair. Signup state is deliberately untouched.
            let (tcp, udp) = classic_debug_client_ports(value);
            parsed.network_active = Some(true);
            parsed.direct_join = Some("localhost".to_string());
            parsed.lobby_timeout = Some(None);
            parsed.tcp_port = Some(tcp);
            parsed.udp_port = Some(udp);
        } else if let Some(value) = classic_argument_value(argument, "/tcpport:") {
            parsed.tcp_port = Some(classic_port(value));
        } else if let Some(value) = classic_argument_value(argument, "/udpport:") {
            parsed.udp_port = Some(classic_port(value));
        } else if let Some(value) = classic_argument_value(argument, "/pass:") {
            parsed.password = Some(value.to_string());
        } else if let Some(value) = classic_argument_value(argument, "/comment:") {
            parsed.comment = Some(value.to_string());
        } else if let Some(value) = classic_argument_value(argument, "/recdump:") {
            parsed.record_dump = Some(value.to_string());
        } else if let Some(value) = classic_argument_value(argument, "/stream:") {
            parsed.record_stream = Some(PathBuf::from(value));
        } else if let Some(value) = classic_argument_value(argument, "/startup:") {
            parsed.startup_screen = Some(value.to_string());
        } else if argument.eq_ignore_ascii_case("/console") {
            parsed.console = true;
        } else if let Some(value) = classic_argument_value(argument, "/config:") {
            parsed.config_file = Some(PathBuf::from(value));
        } else if argument.eq_ignore_ascii_case("/verbose") {
            parsed.verbose = true;
        } else if parsed.language.is_none() {
            if let Some(value) = classic_argument_value(argument, "/Language:") {
                parsed.language = Some(value.to_string());
            }
        }
    }

    parsed
}

pub(crate) fn write_classic_record_dump(
    chunks: &[clonk_network::ControlRecordChunk],
    destination: &Path,
) -> Result<()> {
    // GetExtension scans back to the last platform directory separator and
    // accepts a leading dot as the extension separator. `Path::extension`
    // deliberately treats a basename such as `.txt` as extensionless.
    let output = if classic_path_extension(destination)
        .is_some_and(|extension| extension.eq_ignore_ascii_case("txt"))
    {
        clonk_network::encode_control_record_text(chunks)
            .context("failed to create classic text record dump")?
    } else {
        clonk_network::encode_control_record_binary(chunks)
            .context("failed to create classic binary record dump")?
    };
    fs::write(destination, output).with_context(|| {
        format!(
            "failed to write classic record dump {}",
            destination.display()
        )
    })
}

/// Split the parameter tail accepted by `C4Application::OnCommand("/open …")`.
/// `SGetParameter` treats a leading double quote as a spaced argument and
/// otherwise skips empty space-delimited fields. It does not implement shell
/// escaping, so keep this deliberately smaller than a shell-word parser.
pub(crate) fn parse_classic_console_parameters(command_line: &str) -> Vec<OsString> {
    let mut remaining = command_line;
    let mut parameters = Vec::new();
    while !remaining.is_empty() {
        if let Some(quoted) = remaining.strip_prefix('"') {
            let end = quoted.find('"').unwrap_or(quoted.len());
            let parameter = &quoted[..end];
            if !parameter.is_empty() {
                parameters.push(OsString::from(parameter));
            }
            let closing_quote = usize::from(end < quoted.len());
            remaining = &quoted[end + closing_quote..];
            continue;
        }

        let space = remaining.find(' ');
        let quote = remaining.find('"');
        let end = match (space, quote) {
            (Some(space), Some(quote)) if quote < space => quote,
            (Some(space), _) => space,
            (None, _) => remaining.len(),
        };
        if end > 0 {
            parameters.push(OsString::from(&remaining[..end]));
            remaining = &remaining[end..];
        } else {
            // SGetParameter advances over empty fields one byte at a time.
            remaining = &remaining[1..];
        }
    }
    parameters
}

#[derive(Debug)]
pub(crate) enum ConsoleInputEvent {
    Command(String),
    Eof,
    Error(io::Error),
}

pub(crate) fn forward_console_input<R: BufRead>(
    mut input: R,
    sender: mpsc::Sender<ConsoleInputEvent>,
) -> io::Result<()> {
    let mut line = Vec::new();
    loop {
        line.clear();
        if input.read_until(b'\n', &mut line)? == 0 {
            let _ = sender.send(ConsoleInputEvent::Eof);
            return Ok(());
        }
        let terminated =
            line.last() == Some(&b'\n') || (cfg!(windows) && line.last() == Some(&b'\r'));
        if !terminated {
            // Native buffers bytes until Return/LF. EOF does not dispatch a
            // final unterminated fragment.
            let _ = sender.send(ConsoleInputEvent::Eof);
            return Ok(());
        }
        // StdAppUnix/Win32 append printable input bytes and dispatch only a
        // non-empty line. Filtering ASCII controls also normalizes CRLF while
        // retaining spaces exactly for OnCommand's matching rules.
        let command = line
            .iter()
            .copied()
            .filter(|byte| !byte.is_ascii_control())
            .collect::<Vec<_>>();
        let command = native_bytes_as_legacy_text(&command);
        if !command.is_empty() && sender.send(ConsoleInputEvent::Command(command)).is_err() {
            return Ok(());
        }
    }
}

pub(crate) fn spawn_console_stdin_reader() -> Result<Receiver<ConsoleInputEvent>> {
    let (sender, receiver) = mpsc::channel();
    thread::Builder::new()
        .name("lc-console-stdin".to_string())
        .spawn(move || {
            let stdin = io::stdin();
            let error_sender = sender.clone();
            if let Err(error) = forward_console_input(stdin.lock(), sender) {
                tracing::warn!(%error, "console stdin reader stopped");
                let _ = error_sender.send(ConsoleInputEvent::Error(error));
            }
        })
        .context("failed to start console stdin reader")?;
    Ok(receiver)
}

/// Boots the dedicated server: no window, no render device, no sound.
///
/// `CStdNoGfx` owns neither a window nor a context — `Init` only records the
/// application and every draw entry point returns success without touching one
/// (StdNoGfx.cpp:20-41) — so a NoGfx engine reaches the game loop having
/// created nothing a display server or GPU is needed for. The process is then
/// driven entirely by the stdin console C++ compiles in for exactly this build
/// (StdAppUnix.cpp:413-449, StdAppWin32.cpp:77-79 ->
/// `C4Application::OnCommand`, C4Application.cpp:586).
pub(crate) fn run_headless_server(
    cli: &Cli,
    classic: &ClassicCommandLine,
    app_paths: Option<&Arc<AppPaths>>,
    runtime: RuntimeConfig,
) -> Result<()> {
    // `CStdNoGfx::CreateDirectDraw` reports the configured resolution like any
    // other renderer, so the logical surface keeps its ordinary size. Nothing
    // presents it.
    let (logical_width, logical_height) = DisplayOptions::default().actual_size();
    let mut app = GameApp::new_with_debug_hud(
        logical_width,
        logical_height,
        AudioOptions::silenced(),
        app_paths.map(|paths| &**paths),
        runtime,
        false,
    )
    .context("failed to initialise headless server state")?;
    app.headless = true;
    app.set_display_mode(DisplayMode::Window);
    app.apply_classic_command_line(classic)?;
    app.auto_start_sandbox = cli.sandbox;
    app.launch_classic_command_line_join()
        .context("failed to start command-line network join")?;
    app.launch_classic_command_line_scenario()
        .context("failed to start command-line scenario")?;
    tracing::info!("running as a headless dedicated server; type /quit on stdin to stop");
    run_console_event_loop(app, spawn_console_stdin_reader()?)
}

pub(crate) fn run_console_event_loop(
    mut app: GameApp,
    commands: Receiver<ConsoleInputEvent>,
) -> Result<()> {
    enum ConsoleLoopEvent {
        Input(ConsoleInputEvent),
        Network(NetworkEventWake),
    }

    let (loop_sender, loop_receiver) = mpsc::channel();
    let input_sender = loop_sender.clone();
    thread::Builder::new()
        .name("lc-console-input-events".to_string())
        .spawn(move || {
            while let Ok(event) = commands.recv() {
                if input_sender.send(ConsoleLoopEvent::Input(event)).is_err() {
                    break;
                }
            }
        })
        .context("failed to start console input event bridge")?;
    app.install_network_event_waker(Arc::new(move |wake| {
        let _ = loop_sender.send(ConsoleLoopEvent::Network(wake));
    }));

    let mut previous_instant = Instant::now();
    let mut accumulator = Duration::ZERO;
    let mut game_clock_accumulator = Duration::ZERO;
    let mut frame_schedule = frame_schedule_for_mode(
        app.mode,
        app.engine.game_tick_delay_ms(),
        app.engine.game_tick_delay_revision(),
        app.refresh_ceilings(),
    );

    let outcome = (|| -> Result<()> {
        loop {
            app.refresh_network_event_waker();
            if app.take_exit_request() {
                return Ok(());
            }

            let now = Instant::now();
            let frame_time = now.saturating_duration_since(previous_instant);
            previous_instant = now;
            advance_game_clock_from_elapsed(&mut app, &mut game_clock_accumulator, frame_time)?;
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
                accumulator = Duration::ZERO;
            }
            advance_simulation_pass(&mut app, &mut frame_schedule, &mut accumulator)?;

            if app.take_exit_request() {
                return Ok(());
            }

            let wait_duration = if app.mode == AppMode::Running && app.full_speed {
                Duration::ZERO
            } else {
                frame_schedule.refresh_interval.min(
                    frame_schedule
                        .simulation_interval
                        .saturating_sub(accumulator),
                )
            };
            match loop_receiver.recv_timeout(wait_duration) {
                Ok(ConsoleLoopEvent::Input(ConsoleInputEvent::Command(command))) => {
                    if let Err(error) = app.process_console_command(&command) {
                        tracing::error!(%error, command, "console command failed");
                    }
                }
                Ok(ConsoleLoopEvent::Input(ConsoleInputEvent::Eof)) => return Ok(()),
                Ok(ConsoleLoopEvent::Input(ConsoleInputEvent::Error(error))) => {
                    return Err(error.into());
                }
                Ok(ConsoleLoopEvent::Network(wake)) => app.note_network_event_wake(wake),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(anyhow!("console stdin reader disconnected unexpectedly"));
                }
            }
        }
    })();
    app.finish_console_shutdown();
    outcome
}

pub(crate) fn install_classic_language_override(classic: &ClassicCommandLine) {
    // This runs before logging, path discovery, or worker creation. Capturing
    // the override in AppPaths also makes fresh discovery inside startup
    // workers observe the same process-local C4Application switch.
    if let Some(language) = classic.language.as_deref() {
        std::env::set_var("LC_LANGUAGE_OVERRIDE", language);
    }
}

/// The eager, all-or-nothing `C4StartupGraphics::Init` image sequence
/// (`C4Startup.cpp:38-89`). Keep this in exact oracle order: startup is
/// initialized before any dialog is selected, so every root owns the whole
/// bundle rather than a per-screen subset.
const CLASSIC_STARTUP_BOOTSTRAP_IMAGES: [&str; 16] = [
    "StartupScenSelBG.png",
    "StartupPlrSelBG.png",
    "StartupPlrPropBG.png",
    "StartupNetworkBG.png",
    "LoaderWatercave1.png",
    "StartupBigButton.png",
    "StartupBigButtonDown.png",
    "StartupBookScroll.png",
    "StartupContext.png",
    "StartupScenSelIcons.png",
    "StartupScenSelTitleOv.png",
    "StartupPlrCtrlType.png",
    "StartupDlgPaper.png",
    "StartupOptionIcons.png",
    "StartupTabClip.png",
    "StartupNetGetRef.png",
];

/// The process-global `C4GraphicsResource::InitFonts` sequence consumed by
/// `C4GUI::Resource` (`C4GraphicsResource.cpp:144-165`). The tooltip face is
/// the same main-size RX font, but is initialized without a shadow.
pub(crate) const CLASSIC_GLOBAL_GUI_FONTS: [&str; 5] = [
    "FontRegular",
    "FontTitle",
    "FontCaption",
    "FontTiny",
    "FontTooltip",
];

/// Exact `C4GUI::Resource::Load` order (`C4Gui.cpp:1086-1110`). C++ resolves
/// these as extensionless stems through the active graphics group set.
pub(crate) const CLASSIC_GLOBAL_GUI_SHEETS: [(&str, &str); 13] = [
    ("GUICaption", "GUICaption.png"),
    ("GUIButton", "GUIButton.png"),
    ("GUIButtonDown", "GUIButtonDown.png"),
    ("GUIButtonHighlight", "GUIButtonHighlight.png"),
    ("GUIIcons", "GUIIcons.png"),
    ("GUIIcons2", "GUIIcons2.png"),
    ("GUIScroll", "GUIScroll.png"),
    ("GUIContext", "GUIContext.png"),
    ("GUISubmenu", "GUISubmenu.png"),
    ("GUICheckbox", "GUICheckbox.png"),
    ("GUIBigArrows", "GUIBigArrows.png"),
    ("GUIProgress", "GUIProgress.png"),
    ("GUISpinBoxArrow", "GUISpinBoxArrow.png"),
];

/// Global GUI/resource images used by currently ported startup descendants,
/// in addition to [`CLASSIC_STARTUP_BOOTSTRAP_IMAGES`]
/// (`C4Gui.cpp:1087-1112`).
const SUPPLEMENTAL_STARTUP_DIALOG_IMAGES: &[&str] = &[
    "GUIButton.png",
    "GUIButtonDown.png",
    "GUIButtonHighlight.png",
    "GUICaption.png",
    "GUICheckbox.png",
    "GUIIcons.png",
    "GUIIcons2.png",
    "GUIContext.png",
    "GUISubmenu.png",
    "GUIScroll.png",
    "GUIProgress.png",
    "GUIBigArrows.png",
    "GUISpinBoxArrow.png",
    "Player.png",
    // In-game menu sheets (C4GraphicsResource.cpp:199-227).
    "Menu.png",
    "Options.png",
    "Control.png",
    // C4StartupPlrPropertiesDlg's colour preview and gamepad control image
    // (C4GraphicsResource.cpp:209,229).
    "Flag.png",
    "Gamepad.png",
    // C4StartupPlrPropertiesDlg's default portrait pool.
    "Portrait1.png",
    "Portrait2.png",
    "Portrait3.png",
    "Portrait4.png",
    "Portrait5.png",
];

pub(crate) struct RuntimeConfig {
    pub(crate) player_owner: i32,
    pub(crate) player_name: String,
    pub(crate) network: Option<NetworkMode>,
    pub(crate) record_enabled: bool,
}

pub(crate) const SYNC_CHECK_RATE: u32 = if cfg!(debug_assertions) { 1 } else { 100 };
pub(crate) const SYNC_CHECK_HISTORY: i32 = 50;

/// `C4D_Goal` / `C4D_Rule` DefCore category bits
/// (clonk-engine script_constants.rs:18,22; C4Def.h).
pub(crate) const C4D_GOAL: i32 = 1 << 5;
pub(crate) const C4D_RULE: i32 = 1 << 19;
pub(crate) const C4D_PARALLAX: i32 = 1 << 21;
pub(crate) const C4D_IGNORE_FOW: i32 = 1 << 25;

pub(crate) fn validate_client_network_scenario(scenario: &Scenario) -> Result<(), String> {
    scenario
        .network_game()
        .then_some(())
        .ok_or_else(|| "retrieved scenario is not marked as a network game".to_string())
}

pub(crate) fn read_optional_initial_network_game_source(
    group: &Group,
) -> std::result::Result<Option<Vec<u8>>, GroupError> {
    match group.read_file("Game.txt") {
        Ok(source) => Ok((!source.is_empty()).then_some(source)),
        Err(GroupError::EntryNotFound(_) | GroupError::Missing(_)) => Ok(None),
        Err(GroupError::Io(error)) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

pub(crate) enum ScenarioLoadingEvent {
    /// Monotonic, C4Game-numbered coarse progress paired with an
    /// oldest-to-newest snapshot of Rust's phase-status history. The worker
    /// stops below 100; only successful main-thread activation may publish the
    /// terminal frame.
    LoaderFrame {
        progress: i32,
        log: Option<Vec<String>>,
    },
    RefreshResources,
    /// A fresh authoritative load rejected one or more malformed generated
    /// landscapes and selected this replacement Parameters.RandomSeed.
    AcceptedRandomSeed(u64),
    Finished(Result<Scenario, String>),
}

pub(crate) const SCENARIO_LOADING_LOG_CAPACITY: usize = 1_000;

pub(crate) struct ScenarioLoadingReporter {
    sender: mpsc::Sender<ScenarioLoadingEvent>,
    last_progress: i32,
}

impl ScenarioLoadingReporter {
    /// Opens the loader's log buffer, the way `C4MessageBoard::Init` hands the
    /// loader a startup buffer before the round loads
    /// (`src/C4MessageBoard.cpp:223-251`).
    pub(crate) fn new(sender: mpsc::Sender<ScenarioLoadingEvent>) -> Self {
        clonk_logging::activate_loader_log();
        Self {
            sender,
            last_progress: 0,
        }
    }

    /// Records a phase milestone. The line goes into the same buffer the GUI
    /// log sink appends to, so a worker thread's log event between two
    /// milestones keeps its position instead of either source replacing the
    /// other (`src/C4Log.cpp:208-243`).
    pub(crate) fn report(&mut self, progress: i32, line: &'static str) {
        self.last_progress = self.last_progress.max(progress.clamp(0, 99));
        clonk_logging::push_loader_log_line(line);
        let _ = self.sender.send(ScenarioLoadingEvent::LoaderFrame {
            progress: self.last_progress,
            log: Some(clonk_logging::loader_log_snapshot()),
        });
    }

    pub(crate) fn send(&self, event: ScenarioLoadingEvent) {
        let _ = self.sender.send(event);
    }
}

pub(crate) struct PreparedGoLoadingState {
    pub(crate) status: clonk_network::NetworkStatus,
    pub(crate) local_reached: bool,
    pub(crate) save_game: bool,
    /// `C4S.Head.NetworkRuntimeJoin`: selects the exclusive runtime branch of
    /// `C4Game::InitPlayers`, even when SavePlayerInfos contains no players.
    pub(crate) network_runtime_join: bool,
    pub(crate) restore_player_infos: Vec<clonk_engine::ControlPlayerInfoEntry>,
    /// Joined SavePlayerInfos rows in client-packet/player order. Runtime
    /// recreation consumes this only after the combined scenario is loaded.
    pub(crate) runtime_join_players: Vec<clonk_engine::RuntimeJoinPlayerSource>,
    /// Narrow client-only inputs retained until the loaded Scenario core
    /// selects the ordinary or NetworkRuntimeJoin restore branch.
    pub(crate) pending_client_runtime_join: Option<PendingClientRuntimeJoinLoading>,
    pub(crate) initial_game_data: Option<clonk_engine::InitialNetworkGameData>,
    pub(crate) random_seed: u64,
    pub(crate) use_fair_crew: bool,
    pub(crate) fair_crew_strength: i32,
    pub(crate) fair_crew_forced: bool,
    pub(crate) allow_debug: bool,
    pub(crate) auto_frame_skip: bool,
    /// The exact `C4GameParameters` lists consumed by InitRules/InitGoals.
    /// Unlike a host's mutable lobby snapshot, this narrow copy survives a
    /// client's pending JoinData packet being consumed before activation.
    pub(crate) synchronized_rule_goal_lists: clonk_engine::GameParameterRuleGoalLists,
    pub(crate) team_configuration: TeamConfiguration,
    pub(crate) team_registry: Vec<clonk_engine::TeamInfo>,
    /// Separately retained `Game.DefinitionFilenames`. Final C4GameRes types
    /// select the groups InitDefs opens but do not rewrite this save identity.
    pub(crate) definition_modules: Option<Vec<String>>,
}

#[derive(Clone)]
pub(crate) struct PendingClientRuntimeJoinLoading {
    pub(crate) local_client_id: i32,
    pub(crate) packet_restore_player_infos: clonk_network::PlayerInfoListSnapshot,
}

#[derive(Debug)]
pub(crate) enum ScenarioActivationError {
    Recoverable(String),
}

impl fmt::Display for ScenarioActivationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Recoverable(message) => f.write_str(message),
        }
    }
}

impl From<String> for ScenarioActivationError {
    fn from(message: String) -> Self {
        Self::Recoverable(message)
    }
}

pub(crate) struct ScenarioLoadingState {
    pub(crate) scenario: FrontendScenario,
    pub(crate) refreshed_resources: Option<LoaderResources>,
    pub(crate) refreshed_tooltip_font: Option<Arc<clonk_graphics::clonk_font::ClonkFont>>,
    pub(crate) refreshed_native_font_source: Option<ClassicNativeFontSource>,
    pub(crate) refreshed_global_gui_failures: Option<HashMap<&'static str, String>>,
    pub(crate) refreshed_gui_sheet_overrides: Option<Vec<ClassicGuiSheetOverride>>,
    pub(crate) refresh_requested: bool,
    pub(crate) receiver: Receiver<ScenarioLoadingEvent>,
    pub(crate) finished: bool,
    pub(crate) last_progress: i32,
    pub(crate) log: Vec<String>,
    pub(crate) prepared_go: Option<PreparedGoLoadingState>,
    pub(crate) offline_startup_players: Option<OfflineStartupPlayers>,
    pub(crate) offline_savegame: Option<OfflineSavegameStartup>,
    /// Fresh local-round Parameters.RandomSeed, frozen before the async
    /// loader creates the dynamic landscape and reused for Engine creation.
    pub(crate) offline_random_seed: Option<u64>,
}

impl ScenarioLoadingState {
    pub(crate) fn new(
        scenario: FrontendScenario,
        refreshed_resources: LoaderResources,
        refreshed_global_gui_failures: HashMap<&'static str, String>,
        refreshed_gui_sheet_overrides: Vec<ClassicGuiSheetOverride>,
        receiver: Receiver<ScenarioLoadingEvent>,
    ) -> Self {
        Self {
            refreshed_resources: Some(refreshed_resources),
            refreshed_tooltip_font: None,
            refreshed_native_font_source: None,
            refreshed_global_gui_failures: Some(refreshed_global_gui_failures),
            refreshed_gui_sheet_overrides: Some(refreshed_gui_sheet_overrides),
            refresh_requested: false,
            scenario,
            receiver,
            finished: false,
            last_progress: 0,
            log: Vec::new(),
            prepared_go: None,
            offline_startup_players: None,
            offline_savegame: None,
            offline_random_seed: None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_network_receiver(
        scenario: FrontendScenario,
        receiver: Receiver<ScenarioLoadingEvent>,
        status: clonk_network::NetworkStatus,
        restore_player_infos: Vec<clonk_engine::ControlPlayerInfoEntry>,
        initial_game_data: Option<clonk_engine::InitialNetworkGameData>,
        random_seed: u64,
        use_fair_crew: bool,
        fair_crew_strength: i32,
        fair_crew_forced: bool,
        allow_debug: bool,
        auto_frame_skip: bool,
        synchronized_rule_goal_lists: clonk_engine::GameParameterRuleGoalLists,
        team_configuration: TeamConfiguration,
        team_registry: Vec<clonk_engine::TeamInfo>,
    ) -> Self {
        Self {
            scenario,
            refreshed_resources: None,
            refreshed_tooltip_font: None,
            refreshed_native_font_source: None,
            refreshed_global_gui_failures: None,
            refreshed_gui_sheet_overrides: None,
            refresh_requested: false,
            receiver,
            finished: false,
            last_progress: 0,
            log: Vec::new(),
            prepared_go: Some(PreparedGoLoadingState {
                status,
                local_reached: false,
                save_game: false,
                network_runtime_join: false,
                restore_player_infos,
                runtime_join_players: Vec::new(),
                pending_client_runtime_join: None,
                initial_game_data,
                random_seed,
                use_fair_crew,
                fair_crew_strength,
                fair_crew_forced,
                allow_debug,
                auto_frame_skip,
                synchronized_rule_goal_lists,
                team_configuration,
                team_registry,
                definition_modules: None,
            }),
            offline_startup_players: None,
            offline_savegame: None,
            offline_random_seed: None,
        }
    }

    pub(crate) fn accept_loader_frame(
        &mut self,
        progress: i32,
        log: Option<Vec<String>>,
    ) -> (i32, Option<Vec<String>>) {
        self.last_progress = self.last_progress.max(progress.clamp(0, 100));
        let replace_log = log.is_some();
        if let Some(mut lines) = log {
            if lines.len() > SCENARIO_LOADING_LOG_CAPACITY {
                lines.drain(..lines.len() - SCENARIO_LOADING_LOG_CAPACITY);
            }
            self.log = lines;
        }
        (self.last_progress, replace_log.then(|| self.log.clone()))
    }
}

pub(crate) enum BootLoadingEvent {
    Finished(Option<Arc<MaterialSet>>),
}

pub(crate) enum ScenarioSelectorDiscoveryEvent {
    Progress(u8),
    Finished(Vec<FrontendScenario>),
}

pub(crate) struct ScenarioSelectorDiscoveryState {
    pub(crate) receiver: Receiver<ScenarioSelectorDiscoveryEvent>,
    pub(crate) cancel: Arc<AtomicBool>,
    pub(crate) progress_percent: u8,
    pub(crate) selected_identifier: Option<String>,
    pub(crate) select_first_when_missing: bool,
    pub(crate) apply_live_search: bool,
    pub(crate) retained_title: Option<(String, String)>,
}

impl Drop for ScenarioSelectorDiscoveryState {
    fn drop(&mut self) {
        self.cancel.store(true, AtomicOrdering::Relaxed);
    }
}

pub(crate) struct BootLoadingState {
    pub(crate) receiver: Receiver<BootLoadingEvent>,
}

impl BootLoadingState {
    pub(crate) fn new(receiver: Receiver<BootLoadingEvent>) -> Self {
        Self { receiver }
    }
}

pub(crate) struct ClassicLoaderSetup {
    pub(crate) screen: LoaderScreen,
    pub(crate) initial_tooltip_font: Option<Arc<clonk_graphics::clonk_font::ClonkFont>>,
    pub(crate) initial_native_font_source: Option<ClassicNativeFontSource>,
    pub(crate) refreshed_resources: LoaderResources,
    pub(crate) refreshed_tooltip_font: Option<Arc<clonk_graphics::clonk_font::ClonkFont>>,
    pub(crate) refreshed_native_font_source: Option<ClassicNativeFontSource>,
    pub(crate) refreshed_global_gui_failures: HashMap<&'static str, String>,
    pub(crate) refreshed_gui_sheet_overrides: Vec<ClassicGuiSheetOverride>,
    pub(crate) refreshed_player_icon: Option<ImageData>,
    pub(crate) refreshed_crew_icon: Option<ImageData>,
    pub(crate) scenario_title: Option<String>,
}

pub(crate) fn retain_selected_scenario_title(frontend: &mut FrontendScenario, title: Option<&str>) {
    if let Some(title) = title {
        frontend.title.clear();
        frontend.title.push_str(title);
    }
}

#[derive(Clone)]
pub(crate) struct LoaderGroupRegistration {
    pub(crate) priority: i32,
    pub(crate) registration_order: usize,
    pub(crate) group: Group,
}

pub(crate) struct SelectedLoaderSource {
    pub(crate) group: Group,
    pub(crate) entry: GroupEntry,
}

impl SelectedLoaderSource {
    pub(crate) fn filename_bytes(&self) -> &[u8] {
        &self.entry.name_bytes
    }

    fn presentation_filename(&self) -> String {
        legacy_presentation_text(self.filename_bytes())
    }

    fn extension_bytes(&self) -> &[u8] {
        let Some(dot) = self.filename_bytes().iter().rposition(|byte| *byte == b'.') else {
            return b"";
        };
        &self.filename_bytes()[dot + 1..]
    }

    fn read_bytes(&self) -> std::result::Result<Vec<u8>, GroupError> {
        self.group.read_entry_bytes_exact(&self.entry)
    }
}

struct ResolvedGraphicsImage {
    image: ImageData,
}

pub(crate) struct SelectedGraphicsImageSource {
    pub(crate) source: SelectedLoaderSource,
    pub(crate) from_registration: bool,
}

#[derive(Clone)]
pub(crate) struct GameGraphicsResources {
    pub(crate) cursor_atlas: Arc<CursorAtlas>,
    pub(crate) hud_graphics: Arc<HudGraphics>,
    pub(crate) options: Option<Arc<ImageData>>,
    pub(crate) palette: Arc<GamePalette>,
    pub(crate) liquid_animation: Option<Arc<ImageData>>,
}

/// Immutable activation work produced by the lobby Preload action and
/// consumed only when the same scenario/definition vector enters the game.
pub(crate) struct LobbyPreloadArtifact {
    pub(crate) scenario_path: PathBuf,
    pub(crate) definition_paths: Vec<String>,
    pub(crate) game_graphics: GameGraphicsResources,
    pub(crate) material_texture_images: Arc<HashMap<String, MaterialTextureSurface>>,
    pub(crate) material_render_info: Arc<HashMap<String, MaterialRenderInfo>>,
    pub(crate) catalog_host: Option<CatalogHostLobbyPreloadArtifact>,
    pub(crate) client: Option<ClientLobbyPreloadArtifact>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CatalogHostLobbyPreloadKey {
    pub(crate) identifier: String,
    pub(crate) scenario_path: PathBuf,
    pub(crate) definition_load: ScenarioDefinitionLoad,
    pub(crate) languages: Vec<String>,
}

pub(crate) struct CatalogHostLobbyPreloadArtifact {
    pub(crate) key: CatalogHostLobbyPreloadKey,
    pub(crate) scenario: Option<Scenario>,
}

impl CatalogHostLobbyPreloadArtifact {
    pub(crate) fn take_matching_scenario(
        &mut self,
        key: &CatalogHostLobbyPreloadKey,
    ) -> Option<Scenario> {
        if &self.key == key {
            self.scenario.take()
        } else {
            None
        }
    }
}

pub(crate) struct ClientLobbyPreloadArtifact {
    pub(crate) client_id: i32,
    pub(crate) dynamic_resource_id: i32,
    pub(crate) random_seed: u64,
    pub(crate) scenario: Option<Scenario>,
    pub(crate) material_groups: Vec<Group>,
    pub(crate) staging_path: Option<PathBuf>,
}

impl Drop for ClientLobbyPreloadArtifact {
    fn drop(&mut self) {
        if let Some(path) = self.staging_path.take() {
            let _ = fs::remove_file(path);
        }
    }
}

#[derive(Default)]
pub(crate) struct ClientCombinedPreloadFile(Option<PathBuf>);

impl ClientCombinedPreloadFile {
    pub(crate) fn replace(&mut self, path: PathBuf) {
        self.clear();
        self.0 = Some(path);
    }

    pub(crate) fn clear(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = fs::remove_file(path);
        }
    }

    pub(crate) fn is_owned(&self) -> bool {
        self.0.is_some()
    }
}

impl Drop for ClientCombinedPreloadFile {
    fn drop(&mut self) {
        self.clear();
    }
}

#[derive(Clone)]
pub(crate) struct LobbyPreloadGraphicsContext {
    pub(crate) app_paths: Option<AppPaths>,
    pub(crate) fallback: GameGraphicsResources,
    pub(crate) liquid_animation_enabled: bool,
}

pub(crate) struct LobbyPreloadJob {
    pub(crate) graphics: LobbyPreloadGraphicsContext,
    pub(crate) source: LobbyPreloadJobSource,
}

pub(crate) enum LobbyPreloadJobSource {
    Host {
        frontend: FrontendScenario,
        scenario_path: PathBuf,
        definition_paths: Vec<String>,
    },
    CatalogHost {
        frontend: FrontendScenario,
        key: CatalogHostLobbyPreloadKey,
    },
    Client {
        join_data: clonk_network::JoinDataEnvelope,
        scenario_resources: Option<ClientScenarioResources>,
        game_resources: Vec<ResolvedClientStartResource>,
        resource_directory: PathBuf,
        maker: LegacyCString,
        scenario_path: PathBuf,
        staging_path: Option<PathBuf>,
    },
}

pub(crate) struct LobbyPreloadTask {
    pub(crate) state: LobbyPreloadTaskState,
    pub(crate) start_host_when_ready: bool,
    pub(crate) worker: LobbyPreloadWorker,
}

pub(crate) struct LobbyPreloadWorker(Option<thread::JoinHandle<()>>);

impl LobbyPreloadWorker {
    pub(crate) fn new(worker: thread::JoinHandle<()>) -> Self {
        Self(Some(worker))
    }

    pub(crate) fn join(&mut self) {
        if let Some(worker) = self.0.take() {
            if worker.join().is_err() {
                tracing::error!("lobby preload worker panicked");
            }
        }
    }
}

impl Drop for LobbyPreloadWorker {
    fn drop(&mut self) {
        self.join();
    }
}

pub(crate) enum LobbyPreloadTaskState {
    Loading(Receiver<std::result::Result<LobbyPreloadArtifact, String>>),
    RemovingClientResource {
        artifact: LobbyPreloadArtifact,
        receiver: Receiver<std::result::Result<(), String>>,
    },
}

extern "C" {
    pub(crate) fn rand() -> std::os::raw::c_int;
}

pub(crate) fn classic_safe_random_unlocked(range: usize) -> usize {
    if range == 0 {
        return 0;
    }
    // SAFETY: C rand takes no arguments and C guarantees a non-negative
    // result. Callers serialize access with CLASSIC_SAFE_RANDOM_LOCK.
    (unsafe { rand() } as usize) % range
}

pub(crate) fn classic_safe_random(range: usize) -> usize {
    let _guard = lock_unpoisoned(&CLASSIC_SAFE_RANDOM_LOCK);
    classic_safe_random_unlocked(range)
}

fn classic_names_equal_case_insensitive(first: &[u8], second: &[u8]) -> bool {
    let capital = |byte| match byte {
        b'a'..=b'z' => byte - (b'a' - b'A'),
        0xe4 => 0xc4,
        0xf6 => 0xd6,
        0xfc => 0xdc,
        _ => byte,
    };
    first.len() == second.len()
        && first
            .iter()
            .zip(second)
            .all(|(&first, &second)| capital(first) == capital(second))
}

pub(crate) fn classic_script_player_name(
    configured_names: &LegacyCString,
    active_names: &[&[u8]],
    next_random: &mut impl FnMut(usize) -> usize,
) -> LegacyCString {
    if configured_names.is_empty() {
        return clonk_network::validate_name_no_empty(
            LegacyCString::from_bytes(b"Computer".to_vec())
                .expect("the shipped script-player fallback contains no NUL"),
        );
    }

    let names = configured_names
        .as_bytes()
        .split(|byte| *byte == b'|')
        .collect::<Vec<_>>();
    let selected = names
        .iter()
        .copied()
        .find(|candidate| {
            active_names
                .iter()
                .all(|active| !classic_names_equal_case_insensitive(active, candidate))
        })
        .unwrap_or_else(|| names[next_random(names.len())]);
    clonk_network::validate_name_no_empty(
        LegacyCString::from_bytes(selected.to_vec())
            .expect("configured script-player names contain no interior NUL"),
    )
}

pub(crate) fn classic_script_player_color(next_random: &mut impl FnMut(usize) -> usize) -> u32 {
    let mut channel = || next_random(302).min(256) as u8;
    let red = channel();
    let green = channel();
    let blue = channel();
    // StdColors.h's RGB helper stores red in the low byte.
    u32::from(red) | (u32::from(green) << 8) | (u32::from(blue) << 16)
}

fn select_loader_with_safe_random(
    groups: &[Group],
    graphics: &Group,
    specification: &str,
) -> Result<SelectedLoaderSource> {
    let _guard = CLASSIC_SAFE_RANDOM_LOCK
        .lock()
        .map_err(|_| anyhow!("classic SafeRandom lock was poisoned"))?;
    select_loader_source(
        groups,
        graphics,
        specification,
        classic_safe_random_unlocked,
    )
}

pub(crate) fn select_loader_source(
    groups: &[Group],
    graphics: &Group,
    specification: &str,
    mut next_mod: impl FnMut(usize) -> usize,
) -> Result<SelectedLoaderSource> {
    let patterns = loader_patterns(specification)?;
    let mut count = 0usize;
    let mut chosen = None;

    // C4LoaderScreen's GroupSet pass uses png, jpeg, jpg, doubles all of
    // those reservoir slots, then visits bmp.
    for group in groups {
        seek_loader_candidates(group, &patterns.png, &mut count, &mut chosen, &mut next_mod)?;
        seek_loader_candidates(
            group,
            &patterns.jpeg,
            &mut count,
            &mut chosen,
            &mut next_mod,
        )?;
        seek_loader_candidates(group, &patterns.jpg, &mut count, &mut chosen, &mut next_mod)?;
        count = count
            .checked_mul(2)
            .context("classic loader reservoir count overflow")?;
        anyhow::ensure!(
            count <= i32::MAX as usize,
            "classic loader reservoir exceeds C++ int range"
        );
        seek_loader_candidates(group, &patterns.bmp, &mut count, &mut chosen, &mut next_mod)?;
    }
    if count > 0 {
        return chosen.context("classic loader reservoir selected no local candidate");
    }

    // Main Graphics.c4g differs in the jpeg/jpg order.
    seek_loader_candidates(
        graphics,
        &patterns.png,
        &mut count,
        &mut chosen,
        &mut next_mod,
    )?;
    seek_loader_candidates(
        graphics,
        &patterns.jpg,
        &mut count,
        &mut chosen,
        &mut next_mod,
    )?;
    seek_loader_candidates(
        graphics,
        &patterns.jpeg,
        &mut count,
        &mut chosen,
        &mut next_mod,
    )?;
    count = count
        .checked_mul(2)
        .context("classic loader reservoir count overflow")?;
    anyhow::ensure!(
        count <= i32::MAX as usize,
        "classic loader reservoir exceeds C++ int range"
    );
    seek_loader_candidates(
        graphics,
        &patterns.bmp,
        &mut count,
        &mut chosen,
        &mut next_mod,
    )?;

    if count == 0 {
        // The final fallback intentionally excludes bmp.
        for pattern in ["Loader*.png", "Loader*.jpg", "Loader*.jpeg"] {
            seek_loader_candidates(graphics, pattern, &mut count, &mut chosen, &mut next_mod)?;
        }
    }

    // Exhaustion is an ordinary fatal in C++, not an unimplemented path: Init
    // logs this reason itself and returns false, and the caller then logs
    // IDS_PRC_ERRLOADER (src/C4LoaderScreen.cpp:83-86). The pattern list is
    // printed png/bmp/jpg/jpeg, which is none of the search orders above.
    chosen.with_context(|| {
        format!(
            "No loaders found for loader specification: {}/{}/{}/{}",
            patterns.png, patterns.bmp, patterns.jpg, patterns.jpeg
        )
    })
}

pub(crate) struct LoaderPatterns {
    pub(crate) png: String,
    bmp: String,
    jpg: String,
    jpeg: String,
}

pub(crate) fn loader_patterns(specification: &str) -> Result<LoaderPatterns> {
    let specification = if specification.is_empty() {
        "Loader*"
    } else {
        specification
    };
    let specification_bytes = clonk_script::c4_string_bytes(specification);
    anyhow::ensure!(
        specification_bytes.len() <= 128,
        "classic loader specification exceeds C++'s 128-byte buffer"
    );
    anyhow::ensure!(
        !specification_bytes.contains(&0),
        "classic loader specification contains an embedded NUL"
    );
    let with_default_extension = |extension: &str| {
        let directory_separator = if cfg!(windows) { '\\' } else { '/' };
        let filename = specification
            .rsplit_once(directory_separator)
            .map_or(specification, |(_, filename)| filename);
        if filename
            .rsplit_once('.')
            .is_some_and(|(_, extension)| !extension.is_empty())
        {
            specification.to_string()
        } else {
            format!("{specification}.{extension}")
        }
    };
    Ok(LoaderPatterns {
        png: with_default_extension("png"),
        bmp: with_default_extension("bmp"),
        jpg: with_default_extension("jpg"),
        jpeg: with_default_extension("jpeg"),
    })
}

fn seek_loader_candidates(
    group: &Group,
    wildcard: &str,
    count: &mut usize,
    chosen: &mut Option<SelectedLoaderSource>,
    next_mod: &mut impl FnMut(usize) -> usize,
) -> Result<()> {
    let wildcard = clonk_script::c4_string_bytes(wildcard);
    for entry in group.entries()? {
        if !classic_wildcard_match(&wildcard, &entry.name_bytes) {
            continue;
        }
        *count = count
            .checked_add(1)
            .context("classic loader reservoir count overflow")?;
        anyhow::ensure!(
            *count <= i32::MAX as usize,
            "classic loader reservoir exceeds C++ int range"
        );
        let draw = next_mod(*count);
        anyhow::ensure!(
            draw < *count,
            "classic loader RNG returned {draw} outside 0..{}",
            *count
        );
        if draw == 0 {
            *chosen = Some(SelectedLoaderSource {
                group: group.clone(),
                entry,
            });
        }
    }
    Ok(())
}

pub(crate) fn classic_wildcard_match(wildcard: &[u8], value: &[u8]) -> bool {
    let wildcard = if wildcard == b"*.*" { b"*" } else { wildcard };
    classic_raw_wildcard_match(wildcard, value)
}

/// `WildcardMatch` as used by lobby player-name commands. Unlike the loader
/// wrapper above, native command matching does not treat `*.*` as `*`.
pub(crate) fn classic_raw_wildcard_match(wildcard: &[u8], value: &[u8]) -> bool {
    let (mut wildcard_index, mut value_index) = (0usize, 0usize);
    let (mut backtrack_wildcard, mut backtrack_value) = (None, None);
    while wildcard_index < wildcard.len() || backtrack_wildcard.is_some() {
        if wildcard.get(wildcard_index) == Some(&b'*') {
            wildcard_index += 1;
            backtrack_wildcard = Some(wildcard_index);
            backtrack_value = Some(value_index);
        } else if value_index >= value.len() {
            break;
        } else if wildcard.get(wildcard_index) == Some(&b'?')
            || wildcard
                .get(wildcard_index)
                .is_some_and(|byte| byte.eq_ignore_ascii_case(&value[value_index]))
        {
            wildcard_index += 1;
            value_index += 1;
        } else if let (Some(saved_wildcard), Some(saved_value)) =
            (backtrack_wildcard, backtrack_value)
        {
            wildcard_index = saved_wildcard;
            value_index = saved_value.saturating_add(1);
            backtrack_value = Some(value_index);
        } else {
            return false;
        }
    }
    wildcard_index == wildcard.len() && value_index == value.len()
}

pub(crate) fn loader_group_has_content(group: &Group) -> Result<bool> {
    Ok(loader_entries_have_content(&group.entries()?))
}

pub(crate) fn loader_entries_have_content(entries: &[GroupEntry]) -> bool {
    entries.iter().any(|entry| {
        ["Loader*.bmp", "Loader*.png", "Loader*.jpg", "Loader*.jpeg"]
            .iter()
            .any(|wildcard| classic_wildcard_match(wildcard.as_bytes(), &entry.name_bytes))
    })
}

fn loader_parent_paths(path: &Path) -> Vec<PathBuf> {
    let mut parents = Vec::new();
    let mut current = if has_extension(path, "c4f") {
        Some(path)
    } else {
        path.parent()
    };
    while let Some(parent) = current.filter(|candidate| has_extension(candidate, "c4f")) {
        parents.push(parent.to_path_buf());
        current = parent.parent();
    }
    parents.reverse();
    parents
}

fn absolute_loader_path(path: &Path, exe_data_root: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        exe_data_root.join(path)
    }
}

pub(crate) fn classic_language_packs(paths: &AppPaths) -> LanguagePacks {
    let mut logical_roots = Vec::new();
    if let Some(content) = paths.content_dir() {
        logical_roots.push(content.to_path_buf());
    }
    logical_roots.extend([
        paths.planet_dir().to_path_buf(),
        paths.install_root().to_path_buf(),
    ]);
    // C4Language::Init opens one process-global `Language.c4g`. The Rust
    // install layout maps that classic global-data namespace to `planet`;
    // similarly named containers under content/install roots must not be
    // concatenated into an invented precedence chain.
    LanguagePacks::discover(&[paths.planet_dir().join("Language.c4g")], &logical_roots)
}

pub(crate) fn load_lobby_scenario_description(
    path: &Path,
    languages: &[String],
    language_packs: &LanguagePacks,
) -> std::result::Result<Option<String>, GroupError> {
    let group = Group::open(path)?;
    let components = language_packs.component_groups(&group, None, None);
    for candidate in languages.iter().map(|code| format!("Desc{code}.rtf")) {
        let Some(component) = components.read(candidate).ok().flatten() else {
            continue;
        };
        let visible = component
            .bytes
            .split(|byte| *byte == 0)
            .next()
            .unwrap_or_default();
        let description = clonk_resources::rtf::rtf_to_plain_text(visible);
        return Ok((!description.is_empty()).then_some(description));
    }
    Ok(None)
}

/// C4Extra::InitGroup opens exactly one path, `Config.AtExePath(C4CFN_Extra)`,
/// and returns false when `ItemExists` says it is absent (C4Extra.cpp:40-49).
/// The Rust install layout maps that classic global-data namespace to
/// `planet`, as `classic_language_packs` already does for `Language.c4g`, so
/// same-named groups under the content or install roots are unrelated files:
/// C++ never sees them, and they must neither be adopted nor make the mapped
/// group ambiguous. An unreadable mapped group stays optional too — the
/// callers mirror `Open` failing by carrying on without an extra root.
pub(crate) fn mapped_classic_extra_group_path(paths: &AppPaths) -> Result<Option<PathBuf>> {
    let mapped = paths.planet_dir().join("Extra.c4g");
    match fs::symlink_metadata(&mapped) {
        Ok(_) => Ok(Some(mapped)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| {
            format!(
                "classic loader cannot inspect global data path {}",
                mapped.display()
            )
        }),
    }
}

/// `RealPath` used by C++ `ItemIdentical` does not require the leaf to exist.
/// On POSIX it canonicalizes the longest existing prefix and appends the
/// untouched suffix; this is also how logical children below packed groups
/// remain comparable even though they are not filesystem directories.
fn cpp_loader_real_path(logical: &Path) -> Result<PathBuf> {
    anyhow::ensure!(
        logical.is_absolute(),
        "classic loader ItemIdentical path is not absolute: {}",
        logical.display()
    );
    let logical_text = logical.to_str().with_context(|| {
        format!(
            "classic loader cannot represent ItemIdentical for non-UTF-8 logical path {}",
            logical.display()
        )
    })?;
    anyhow::ensure!(
        !logical_text.contains('\0'),
        "classic loader cannot represent ItemIdentical for a logical path containing NUL"
    );

    #[cfg(windows)]
    {
        // `_fullpath` is lexical and does not require the target to exist.
        let mut normalized = PathBuf::new();
        for component in logical.components() {
            match component {
                Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                    normalized.push(component.as_os_str());
                }
                Component::CurDir => {}
                Component::ParentDir => {
                    if normalized.file_name().is_some() {
                        normalized.pop();
                    }
                }
            }
        }
        Ok(normalized)
    }

    #[cfg(not(windows))]
    {
        let mut prefix = logical.to_path_buf();
        let mut suffix = Vec::new();
        loop {
            if let Ok(mut resolved) = fs::canonicalize(&prefix) {
                for component in suffix.iter().rev() {
                    resolved.push(component);
                }
                return Ok(resolved);
            }
            let Some(component) = prefix.file_name().map(|name| name.to_os_string()) else {
                // C++ falls back to the original logical spelling when no
                // prefix can be resolved.
                return Ok(logical.to_path_buf());
            };
            suffix.push(component);
            if !prefix.pop() {
                return Ok(logical.to_path_buf());
            }
        }
    }
}

pub(crate) fn cpp_loader_items_identical(left: &Path, right: &Path) -> Result<bool> {
    let left = cpp_loader_real_path(left)?;
    let right = cpp_loader_real_path(right)?;
    if cfg!(windows) {
        let left = left.to_str().context(
            "classic loader cannot represent the normalized left ItemIdentical path as UTF-8",
        )?;
        let right = right.to_str().context(
            "classic loader cannot represent the normalized right ItemIdentical path as UTF-8",
        )?;
        Ok(left.eq_ignore_ascii_case(right))
    } else {
        Ok(left == right)
    }
}

/// The data root a relative `Origin` is spelled against.
///
/// C++ needs no such choice: `C4GroupSet.cpp:297` opens the `Origin` unprefixed
/// against the working directory `C4Config.cpp:1320-1321` forces to the single
/// `ExePath`. This port splits that root, and a savegame keeps whichever
/// spelling it was written with (`C4GameSave.cpp:96` keeps an assigned Origin),
/// so the outermost parent it names decides which root it belongs to.
fn loader_origin_data_root(origin: &Path, paths: &AppPaths) -> PathBuf {
    if origin.is_absolute() {
        return paths.executable_data_root().to_path_buf();
    }
    paths
        .executable_data_roots()
        .into_iter()
        .find(|root| {
            loader_parent_paths(&root.join(origin))
                .first()
                .is_some_and(|outer| outer.exists())
        })
        .unwrap_or_else(|| paths.executable_data_root().to_path_buf())
}

pub(crate) fn resolve_loader_origin(
    raw_origin: &str,
    scenario_path: &Path,
    paths: &AppPaths,
) -> Result<Option<PathBuf>> {
    let normalized = raw_origin.replace('\\', "/");
    let origin = PathBuf::from(normalized);
    let exe_data_root = loader_origin_data_root(&origin, paths);
    let candidate = absolute_loader_path(&origin, &exe_data_root);
    if loader_parent_paths(&candidate).is_empty() {
        // This includes the validated explicit empty value (`empty`):
        // RegisterParentFolders has no contiguous .c4f parent and is a no-op.
        return Ok(None);
    }
    let scenario = absolute_loader_path(scenario_path, &exe_data_root);
    let identical = cpp_loader_items_identical(&candidate, &scenario)?;
    Ok((!identical).then_some(candidate))
}

pub(crate) fn register_loader_origin_parents(
    origin_path: &Path,
    registrations: &mut Vec<LoaderGroupRegistration>,
    registration_order: &mut usize,
) {
    let parents = loader_parent_paths(origin_path);
    let Some((outer_path, inner_paths)) = parents.split_first() else {
        return;
    };
    // `C4Game::OpenScenario` discards this result (`C4Game.cpp:177-178`), and
    // `RegisterParentFolders` registers each parent as it opens it
    // (`C4GroupSet.cpp:310`) before reporting the first it cannot and
    // returning null (`C4GroupSet.cpp:291-301`). An Origin naming a parent the
    // install no longer holds therefore costs that parent and the groups below
    // it, never the scenario.
    let Some(mut group) = open_group_path_for_folder_map(outer_path)
        .inspect_err(|error| {
            tracing::warn!(
                %error,
                parent = %outer_path.display(),
                "classic loader cannot open the outer Origin parent; registering none like C++"
            );
        })
        .ok()
    else {
        return;
    };
    registrations.push(LoaderGroupRegistration {
        priority: 100,
        registration_order: *registration_order,
        group: group.clone(),
    });
    *registration_order = registration_order.saturating_add(1);

    for (inner_index, inner_path) in inner_paths.iter().enumerate() {
        let child = inner_path.file_name().and_then(|name| {
            open_child_flexible(&group, Path::new(name))
                .inspect_err(|error| {
                    tracing::warn!(
                        %error,
                        child = %inner_path.display(),
                        "classic loader cannot inspect an Origin parent child"
                    );
                })
                .ok()
                .flatten()
        });
        let Some(child) = child else {
            tracing::warn!(
                opened = %group.root().display(),
                child = %inner_path.display(),
                "classic loader stopped at a missing Origin parent; keeping the parents already registered like C++"
            );
            return;
        };
        group = child;
        registrations.push(LoaderGroupRegistration {
            priority: 100 + i32::try_from(inner_index + 1).unwrap_or(i32::MAX),
            registration_order: *registration_order,
            group: group.clone(),
        });
        *registration_order = registration_order.saturating_add(1);
    }
}

fn effective_loader_definition_modules(
    head: &ScenarioLoaderHead,
    definition_load: &ScenarioDefinitionLoad,
) -> Result<Vec<String>> {
    // `DefinitionFilenamesFromSaveGame` replaces the whole vector — preset,
    // DefinitionPath expansion and folder-local scan alike — before the
    // resource list and the graphics/material load ever read it
    // (C4Game.cpp:180-227).
    if let Some(modules) = head.savegame_definition_override().effective_modules() {
        return Ok(modules);
    }
    Ok(match definition_load {
        ScenarioDefinitionLoad::Fixed { modules, .. } => modules.clone(),
        ScenarioDefinitionLoad::Seed { modules, .. }
            if head.local_only() || head.configured_definition_modules().is_empty() =>
        {
            modules.clone()
        }
        ScenarioDefinitionLoad::Seed { .. } => {
            head.configured_definition_module_spellings().to_vec()
        }
    })
}

pub(crate) fn extra_definition_filename(module: &str) -> Option<&str> {
    module
        .rsplit(|character| character == '/' || (cfg!(windows) && character == '\\'))
        .next()
        .filter(|name| !name.is_empty())
}

pub(crate) fn extra_definition_group_names(
    head: &ScenarioLoaderHead,
    definition_load: &ScenarioDefinitionLoad,
    scenario_path: &Path,
) -> Result<Vec<String>> {
    let modules = effective_loader_definition_modules(head, definition_load)?;
    let module_names = modules
        .iter()
        .filter_map(|module| extra_definition_filename(module).map(str::to_string))
        .collect::<Vec<_>>();
    let definition_root = match definition_load {
        ScenarioDefinitionLoad::Seed {
            definition_root, ..
        }
        | ScenarioDefinitionLoad::Fixed {
            definition_root, ..
        } => definition_root,
    };
    let mut names = Vec::new();
    // OpenScenario prepends a rooted copy of the selected vector whenever
    // DefinitionPath is active, then retains the original entries.
    if definition_root.is_some() {
        names.extend(module_names.iter().cloned());
    }
    names.extend(module_names);

    // FoldersWithLocalsDefs appends each outer-to-inner c4f ancestor that
    // directly contains at least one definition child.
    for parent_path in loader_parent_paths(scenario_path) {
        let parent = open_group_path_for_folder_map(&parent_path)?;
        let has_definitions = parent.entries()?.into_iter().any(|entry| {
            entry.relative_path.components().count() == 1
                && entry
                    .relative_path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("c4d"))
        });
        if has_definitions {
            if let Some(name) = parent_path.file_name().and_then(|name| name.to_str()) {
                names.push(name.to_string());
            }
        }
    }
    Ok(names)
}

fn register_classic_extra_groups(
    paths: &AppPaths,
    definition_names: &[String],
    registrations: &mut Vec<LoaderGroupRegistration>,
    registration_order: &mut usize,
) -> Result<()> {
    let Some(extra_path) = mapped_classic_extra_group_path(paths)? else {
        return Ok(());
    };
    let extra = match Group::open(&extra_path) {
        Ok(extra) => extra,
        Err(error) => {
            tracing::warn!(
                path = %extra_path.display(),
                %error,
                "failed to open optional global Extra.c4g"
            );
            return Ok(());
        }
    };
    registrations.push(LoaderGroupRegistration {
        priority: 2,
        registration_order: *registration_order,
        group: extra.clone(),
    });
    *registration_order = registration_order.saturating_add(1);

    for name in definition_names {
        let group = match open_child_flexible(&extra, Path::new(name)) {
            Ok(Some(group)) => group,
            Ok(None) => continue,
            Err(error) => {
                tracing::warn!(
                    extra = %extra_path.display(),
                    definition = name,
                    %error,
                    "failed to open activated Extra.c4g definition group"
                );
                continue;
            }
        };
        tracing::info!(
            extra = %extra_path.display(),
            definition = name,
            "loading activated Extra.c4g definition group"
        );
        registrations.push(LoaderGroupRegistration {
            priority: 3,
            registration_order: *registration_order,
            group,
        });
        *registration_order = registration_order.saturating_add(1);
    }
    Ok(())
}

pub(crate) fn classic_loader_registrations(
    scenario: &FrontendScenario,
    scenario_group: &Group,
    head: &ScenarioLoaderHead,
    definition_load: &ScenarioDefinitionLoad,
    paths: &AppPaths,
) -> Result<Vec<LoaderGroupRegistration>> {
    let scenario_path = scenario
        .path
        .as_deref()
        .context("classic scenario loader has no scenario path")?;
    let mut registrations = Vec::new();
    let mut registration_order = 0usize;

    for (depth, parent_path) in loader_parent_paths(scenario_path).into_iter().enumerate() {
        registrations.push(LoaderGroupRegistration {
            priority: 100 + i32::try_from(depth).unwrap_or(i32::MAX),
            registration_order,
            group: open_group_path_for_folder_map(&parent_path)?,
        });
        registration_order += 1;
    }
    registrations.push(LoaderGroupRegistration {
        priority: 200,
        registration_order,
        group: scenario_group.clone(),
    });
    registration_order += 1;

    // OpenScenario registers Origin parents after the actual scenario, so
    // they precede actual-path parents at equal priorities.
    if let Some(origin) = head.origin() {
        if let Some(origin_path) = resolve_loader_origin(origin, scenario_path, paths)? {
            register_loader_origin_parents(
                &origin_path,
                &mut registrations,
                &mut registration_order,
            );
        }
    }

    let extra_names = extra_definition_group_names(head, definition_load, scenario_path)?;
    register_classic_extra_groups(
        paths,
        &extra_names,
        &mut registrations,
        &mut registration_order,
    )?;
    Ok(registrations)
}

pub(crate) fn highest_loader_tier(registrations: &[LoaderGroupRegistration]) -> Result<Vec<Group>> {
    let mut eligible = Vec::new();
    for registration in registrations {
        if loader_group_has_content(&registration.group)? {
            eligible.push(registration);
        }
    }
    let Some(priority) = eligible.iter().map(|entry| entry.priority).max() else {
        return Ok(Vec::new());
    };
    eligible.retain(|entry| entry.priority == priority);
    eligible.sort_by_key(|entry| std::cmp::Reverse(entry.registration_order));
    Ok(eligible
        .into_iter()
        .map(|entry| entry.group.clone())
        .collect())
}

pub(crate) fn decode_selected_loader(source: &SelectedLoaderSource) -> Result<ImageData> {
    // Force direct-entry semantics first. A child group whose name happens
    // to match Loader*.png participates in C4Group::FindEntry, but its image
    // load fails rather than recursively finding a nested image.
    let bytes = source.read_bytes().with_context(|| {
        format!(
            "selected classic loader `{}` in {} is not a readable file",
            source.presentation_filename(),
            source.group.root().display()
        )
    })?;
    // C4Surface selects its decoder from the entry name rather than sniffing
    // the payload. Preserve that behavior so a renamed image fails instead of
    // silently loading through a different codec.
    let extension = source.extension_bytes();
    let format = if extension.eq_ignore_ascii_case(b"png") {
        image::ImageFormat::Png
    } else if extension.eq_ignore_ascii_case(b"jpg") || extension.eq_ignore_ascii_case(b"jpeg") {
        image::ImageFormat::Jpeg
    } else {
        image::ImageFormat::Bmp
    };
    let image =
        clonk_resources::load_image_from_memory_with_format(&bytes, format).with_context(|| {
            format!(
                "failed to decode exact classic image entry `{}` from {}",
                source.presentation_filename(),
                source.group.root().display()
            )
        })?;
    let rgba = image.into_rgba8();
    let (width, height) = rgba.dimensions();
    let image = GraphicsImage::new(width, height, rgba.into_raw());
    let (width, height, pixels) = image.into_parts();
    Ok(ImageData::from_arc(width, height, pixels))
}

#[derive(Clone)]
pub(crate) struct ClassicFontBundle {
    pub(crate) fonts: Arc<clonk_frontend::ClonkFontSet>,
    pub(crate) tooltip: Arc<clonk_graphics::clonk_font::ClonkFont>,
    /// One vector face that exactly matches the fixed role recipe supported by
    /// `build_native_font_set`. Arbitrary FontDefs and bitmap mappings remain
    /// logical-only until the native renderer accepts per-role recipes.
    pub(crate) native_source: Option<ClassicNativeFontSource>,
}

pub(crate) struct ClassicStartupFontBundle {
    pub(crate) book: Arc<clonk_frontend::startup_scensel::BookFontSet>,
    pub(crate) options: Arc<clonk_frontend::startup_options_dlg::BookFonts>,
    pub(crate) player_selection: Arc<clonk_frontend::startup_plrsel::BookFontSet>,
}

#[derive(Clone)]
pub(crate) struct ClassicNativeFontSource {
    pub(crate) bytes: Arc<[u8]>,
    pub(crate) face_index: u32,
    /// Per-role FreeType heights the classic loader resolved for this face.
    pub(crate) sizes: NativeFontSizes,
    /// `Graphics.SnapTextToPixels`, resolved once with the rest of the bundle.
    pub(crate) snap_to_pixels: bool,
}

fn classic_font_request(paths: &AppPaths, scenario_font: Option<&str>) -> Result<(String, i32)> {
    let config = load_classic_loader_config(paths)?;
    let configured_name = config
        .as_ref()
        .map(|config| classic_loader_bounded_config_value(config, "FontName"))
        .transpose()?
        .flatten()
        .unwrap_or("Endeavour");
    let configured_size = config
        .as_ref()
        .and_then(|config| classic_loader_config_value(config, "FontSize"))
        .and_then(|size| size.trim().parse::<i32>().ok())
        .unwrap_or(14);
    let effective_name = scenario_font
        .filter(|font| !font.is_empty())
        .unwrap_or(configured_name);
    Ok((effective_name.to_string(), configured_size))
}

fn load_classic_font_catalog(
    paths: &AppPaths,
    registrations: &[LoaderGroupRegistration],
) -> Result<FontCatalog> {
    let system = Group::open(paths.system_group_path()).with_context(|| {
        format!(
            "cannot open classic system font group {}",
            paths.system_group_path().display()
        )
    })?;
    let mut catalog = FontCatalog::default();
    catalog
        .load_group(&system)
        .context("failed to load classic system font resources")?;
    for registration in registrations {
        // C4GroupSet::RegisterGroup ignores LoadDefs failures. Vector faces
        // loaded before an unreadable optional Fonts.txt remain registered.
        let _ = catalog.load_group(&registration.group);
    }
    Ok(catalog)
}

fn build_classic_font_spec(
    spec: ResolvedFontSpec,
    registrations: &[LoaderGroupRegistration],
    graphics: &Group,
    shadow: bool,
) -> Result<clonk_graphics::clonk_font::ClonkFont> {
    match spec {
        ResolvedFontSpec::Vector {
            face,
            bytes,
            face_index,
            size,
            weight,
        } => {
            let bytes =
                bytes.with_context(|| format!("classic font face `{face}` was not resolved"))?;
            let size = u32::try_from(size)
                .ok()
                .filter(|size| *size > 0)
                .with_context(|| format!("classic font `{face}` has invalid size {size}"))?;
            clonk_frontend::clonk_fonts::build_vector_font_face(
                &bytes, face_index, size, weight, shadow,
            )
            .with_context(|| format!("failed to initialize classic vector font `{face}`"))
        }
        ResolvedFontSpec::Bitmap { filename, indent } => {
            let image = load_classic_bitmap_font_image(&filename, registrations, graphics)?;
            clonk_frontend::clonk_fonts::build_prerendered_font(
                image.width(),
                image.height(),
                image.pixels(),
                indent,
            )
            .with_context(|| format!("failed to initialize classic bitmap font `{filename}`"))
        }
    }
}

fn resolve_classic_font_spec(
    spec: ResolvedFontSpec,
    system_fonts: &dyn system_fonts::SystemFontProvider,
) -> Result<ResolvedFontSpec> {
    let ResolvedFontSpec::Vector {
        face,
        bytes,
        face_index,
        size,
        weight,
    } = spec
    else {
        return Ok(spec);
    };
    if bytes.is_some() {
        return Ok(ResolvedFontSpec::Vector {
            face,
            bytes,
            face_index,
            size,
            weight,
        });
    }

    let (bytes, face_index) = match fs::read(&face) {
        Ok(bytes) => (Arc::from(bytes.into_boxed_slice()), 0),
        Err(_) => {
            let resolved = system_fonts
                .resolve(&face, weight)
                .with_context(|| format!("classic font face `{face}` is unavailable"))?;
            (resolved.bytes, resolved.face_index)
        }
    };
    Ok(ResolvedFontSpec::Vector {
        face,
        bytes: Some(bytes),
        face_index,
        size,
        weight,
    })
}

fn build_classic_font_from_catalog(
    catalog: &FontCatalog,
    request: &str,
    base_size: i32,
    role: FontRole,
    apply_definition: bool,
    system_fonts: &dyn system_fonts::SystemFontProvider,
    registrations: &[LoaderGroupRegistration],
    graphics: &Group,
    shadow: bool,
) -> Result<(ResolvedFontSpec, clonk_graphics::clonk_font::ClonkFont)> {
    let candidates = catalog.resolve_candidates(request, base_size, role, apply_definition);
    if candidates.is_empty() {
        return Err(anyhow!("classic font `{request}` has no {role:?} mapping"));
    }

    let mut last_error = None;
    for candidate in candidates {
        let resolved = match resolve_classic_font_spec(candidate, system_fonts) {
            Ok(resolved) => resolved,
            Err(error) => {
                last_error = Some(error);
                continue;
            }
        };
        match build_classic_font_spec(resolved.clone(), registrations, graphics, shadow) {
            Ok(font) => return Ok((resolved, font)),
            Err(error) => last_error = Some(error),
        }
    }

    Err(last_error
        .unwrap_or_else(|| anyhow!("classic font `{request}` has no usable {role:?} face")))
}

fn matching_native_font_source(
    title: &ResolvedFontSpec,
    caption: &ResolvedFontSpec,
    text: &ResolvedFontSpec,
    main_small: Option<&ResolvedFontSpec>,
    mini: &ResolvedFontSpec,
    tooltip: &ResolvedFontSpec,
) -> Option<ClassicNativeFontSource> {
    // The scale-native builder rasterizes every role itself, so it needs one
    // shared regular-weight vector face — not one particular size recipe.
    // `C4FontLoader::InitFont` derives each role's FreeType height from
    // `Config.General.RXFontSize` (C4Fonts.cpp:279-287), and a FontDef may
    // override any of them, so carry the resolved sizes instead of asserting
    // the RXFontSize=14 literals.
    let vector_source = |spec: &ResolvedFontSpec| match spec {
        ResolvedFontSpec::Vector {
            bytes: Some(bytes),
            face_index,
            size,
            weight,
            ..
        } if *size > 0 && *weight == 400 => u32::try_from(*size)
            .ok()
            .map(|size| (bytes.clone(), *face_index, size)),
        _ => None,
    };
    let sources = [
        vector_source(title)?,
        vector_source(caption)?,
        vector_source(text)?,
        vector_source(main_small?)?,
        vector_source(mini)?,
        // FontTooltip has no native slot of its own: `font_for_role` draws it
        // from the shadowless Main atlas, so it must stay the Main size.
        vector_source(tooltip)?,
    ];
    let (bytes, face_index, text_size) = sources[2].clone();
    let sizes = NativeFontSizes {
        title: sources[0].2,
        caption: sources[1].2,
        text: text_size,
        main_small: sources[3].2,
        mini: sources[4].2,
    };
    (sources
        .iter()
        .all(|candidate| candidate.0.as_ref() == bytes.as_ref() && candidate.1 == face_index)
        && sources[5].2 == text_size)
        .then_some(ClassicNativeFontSource {
            bytes,
            face_index,
            sizes,
            snap_to_pixels: false,
        })
}

#[cfg(all(
    test,
    any(not(feature = "app-test-shard-mode"), feature = "app-test-shard-5",),
))]
mod native_font_recipe_tests {
    use super::*;

    fn vector_spec(size: i32, bytes: &Arc<[u8]>, face_index: u32, weight: u32) -> ResolvedFontSpec {
        ResolvedFontSpec::Vector {
            face: "Endeavour".to_string(),
            bytes: Some(bytes.clone()),
            face_index,
            size,
            weight,
        }
    }

    #[test]
    fn snap_text_to_pixels_defaults_to_the_remaster_master_switch() {
        assert!(!configured_snap_text_to_pixels(b""));
        assert!(!configured_snap_text_to_pixels(
            b"[Graphics]\nSnapTextToPixels=0\n"
        ));
        assert!(configured_snap_text_to_pixels(
            b"[Graphics]\nSnapTextToPixels=1\n"
        ));
        assert!(configured_snap_text_to_pixels(b"[Graphics]\nRemaster=1\n"));
        assert!(!configured_snap_text_to_pixels(
            b"[Graphics]\nRemaster=1\nSnapTextToPixels=0\n"
        ));
    }

    #[test]
    fn native_font_source_carries_any_configured_size_recipe() {
        // `C4FontLoader::InitFont` derives one FreeType height per role from
        // `Config.General.RXFontSize` (C4Fonts.cpp:279-287). A scale-native
        // atlas only needs every role to share one vector face at the regular
        // weight; the 22/16/14/13/12 recipe is just RXFontSize=14.
        let bytes: Arc<[u8]> = Arc::from(vec![7_u8; 8].into_boxed_slice());
        let spec = |size| vector_spec(size, &bytes, 0, 400);

        let classic = matching_native_font_source(
            &spec(22),
            &spec(16),
            &spec(14),
            Some(&spec(13)),
            &spec(12),
            &spec(14),
        )
        .expect("the size-14 recipe keeps its native source");
        assert_eq!(classic.sizes, NativeFontSizes::CLASSIC);

        let sixteen = matching_native_font_source(
            &spec(25),
            &spec(18),
            &spec(16),
            Some(&spec(14)),
            &spec(13),
            &spec(16),
        )
        .expect("a size-16 recipe keeps its native source");
        assert_eq!(sixteen.sizes, NativeFontSizes::for_base_size(16));
        assert_eq!(sixteen.bytes.as_ref(), bytes.as_ref());
        assert_eq!(sixteen.face_index, 0);

        assert!(
            matching_native_font_source(
                &spec(25),
                &spec(18),
                &spec(16),
                Some(&spec(14)),
                &spec(13),
                &spec(14),
            )
            .is_none(),
            "FontTooltip is drawn from the shadowless Main atlas, so it must \
             keep the Main size"
        );
        let other: Arc<[u8]> = Arc::from(vec![9_u8; 8].into_boxed_slice());
        assert!(
            matching_native_font_source(
                &vector_spec(25, &other, 0, 400),
                &spec(18),
                &spec(16),
                Some(&spec(14)),
                &spec(13),
                &spec(16),
            )
            .is_none(),
            "a per-role face mixture has no single native source"
        );
        assert!(
            matching_native_font_source(
                &vector_spec(25, &bytes, 0, 700),
                &spec(18),
                &spec(16),
                Some(&spec(14)),
                &spec(13),
                &spec(16),
            )
            .is_none(),
            "the native builder only rasterizes the regular weight"
        );
        assert!(
            matching_native_font_source(
                &spec(25),
                &spec(18),
                &spec(16),
                None,
                &spec(13),
                &spec(16),
            )
            .is_none(),
            "a missing MainSmall alias has no native recipe"
        );
    }
}

pub(crate) fn resolve_classic_font_bundle(
    paths: &AppPaths,
    scenario_font: Option<&str>,
    catalog_registrations: &[LoaderGroupRegistration],
    graphics_registrations: &[LoaderGroupRegistration],
) -> Result<ClassicFontBundle> {
    let (request, base_size) = classic_font_request(paths, scenario_font)?;
    resolve_classic_font_bundle_for_request(
        paths,
        &request,
        base_size,
        catalog_registrations,
        graphics_registrations,
    )
}

/// Shipped English for `IDS_ERR_INITFONTS`
/// (`planet/System.c4g/LanguageUS.txt:612`).
const FONT_INIT_FAILURE_TEXT: &str = "Error initializing fonts";

/// C4FontLoader::InitFont rejects an empty font name before it consults
/// FontDefs at all, logging IDS_ERR_INITFONTS and returning false
/// (`src/C4Fonts.cpp:176-183`). "Endeavour" is the default for a *missing*
/// `FontName` key (`src/C4Config.cpp:390`), not for a configured empty one, so
/// an empty request must fail here rather than reach the catalog and report a
/// missing mapping.
fn reject_empty_classic_font_request(request: &str) -> Result<()> {
    anyhow::ensure!(!request.is_empty(), "{FONT_INIT_FAILURE_TEXT}");
    Ok(())
}

fn resolve_classic_font_bundle_for_request(
    paths: &AppPaths,
    request: &str,
    base_size: i32,
    catalog_registrations: &[LoaderGroupRegistration],
    graphics_registrations: &[LoaderGroupRegistration],
) -> Result<ClassicFontBundle> {
    resolve_classic_font_bundle_for_request_with_system_fonts(
        paths,
        request,
        base_size,
        catalog_registrations,
        graphics_registrations,
        system_fonts::installed_system_fonts(),
    )
}

pub(crate) fn resolve_classic_font_bundle_for_request_with_system_fonts(
    paths: &AppPaths,
    request: &str,
    base_size: i32,
    catalog_registrations: &[LoaderGroupRegistration],
    graphics_registrations: &[LoaderGroupRegistration],
    system_fonts: &dyn system_fonts::SystemFontProvider,
) -> Result<ClassicFontBundle> {
    reject_empty_classic_font_request(request)?;
    let catalog = load_classic_font_catalog(paths, catalog_registrations)?;
    let graphics = main_graphics_group(paths)?;
    let build = |role, apply_definition, shadow| {
        build_classic_font_from_catalog(
            &catalog,
            request,
            base_size,
            role,
            apply_definition,
            system_fonts,
            graphics_registrations,
            &graphics,
            shadow,
        )
    };
    use clonk_graphics::clonk_font::ClonkFontRole;
    let (text_spec, text) = build(FontRole::Main, true, true)?;
    let text = text.with_role(ClonkFontRole::GuiText);
    let (title_spec, title) = build(FontRole::Title, true, true)?;
    let (caption_spec, caption) = build(FontRole::Caption, true, true)?;
    let (mini_spec, mini) = build(FontRole::Log, true, true)?;
    let (tooltip_spec, tooltip) = build(FontRole::Main, false, false)?;
    // C4GraphicsResource never requests C4FT_MainSmall. Populate the Rust
    // compatibility slot at the derived raw size when possible, but never
    // make a valid logical FontDef alias depend on a same-named vector face.
    let (main_small_spec, main_small) = match build(FontRole::MainSmall, false, true) {
        Ok((spec, font)) => (Some(spec), font),
        Err(_) => (None, text.clone()),
    };
    let main_small = main_small.with_role(ClonkFontRole::GuiMainSmall);
    let snap_to_pixels =
        configured_snap_text_to_pixels(&fs::read(paths.config_file()).unwrap_or_default());
    let native_source = matching_native_font_source(
        &title_spec,
        &caption_spec,
        &text_spec,
        main_small_spec.as_ref(),
        &mini_spec,
        &tooltip_spec,
    )
    .map(|source| ClassicNativeFontSource {
        snap_to_pixels,
        ..source
    });
    let fonts = clonk_frontend::ClonkFontSet {
        title: title.with_role(ClonkFontRole::GuiTitle),
        caption: caption.with_role(ClonkFontRole::GuiCaption),
        text,
        main_small,
        mini: mini.with_role(ClonkFontRole::GuiMini),
    };
    let tooltip = tooltip.with_role(ClonkFontRole::GuiTooltip);
    Ok(ClassicFontBundle {
        fonts: Arc::new(fonts),
        tooltip: Arc::new(tooltip),
        native_source,
    })
}

fn resolve_classic_startup_font_bundle_for_request(
    paths: &AppPaths,
    request: &str,
    base_size: i32,
    catalog_registrations: &[LoaderGroupRegistration],
    graphics_registrations: &[LoaderGroupRegistration],
) -> Result<ClassicStartupFontBundle> {
    resolve_classic_startup_font_bundle_for_request_with_system_fonts(
        paths,
        request,
        base_size,
        catalog_registrations,
        graphics_registrations,
        system_fonts::installed_system_fonts(),
    )
}

pub(crate) fn resolve_classic_startup_font_bundle_for_request_with_system_fonts(
    paths: &AppPaths,
    request: &str,
    base_size: i32,
    catalog_registrations: &[LoaderGroupRegistration],
    graphics_registrations: &[LoaderGroupRegistration],
    system_fonts: &dyn system_fonts::SystemFontProvider,
) -> Result<ClassicStartupFontBundle> {
    use clonk_graphics::clonk_font::ClonkFontRole;

    reject_empty_classic_font_request(request)?;
    let catalog = load_classic_font_catalog(paths, catalog_registrations)?;
    let graphics = main_graphics_group(paths)?;
    let build = |role| {
        build_classic_font_from_catalog(
            &catalog,
            request,
            base_size,
            role,
            true,
            system_fonts,
            graphics_registrations,
            &graphics,
            false,
        )
        .map(|(_, font)| font)
    };

    let title = build(FontRole::Title)?.with_role(ClonkFontRole::BookTitle);
    let caption = build(FontRole::Caption)?.with_role(ClonkFontRole::BookCaption);
    let text = build(FontRole::Main)?.with_role(ClonkFontRole::BookText);
    let small = build(FontRole::MainSmall)?.with_role(ClonkFontRole::BookSmall);

    Ok(ClassicStartupFontBundle {
        book: Arc::new(clonk_frontend::startup_scensel::BookFontSet {
            title: title.clone(),
            caption: caption.clone(),
            text: text.clone(),
            small: small.clone(),
        }),
        // All four tiers, because `GetBlackFontByHeight` picks between them
        // (C4Startup.cpp:125-143).
        options: Arc::new(clonk_frontend::startup_options_dlg::BookFonts {
            book: text.clone(),
            book_small: small,
            book_caption: caption.clone(),
            book_title: title,
        }),
        player_selection: Arc::new(clonk_frontend::startup_plrsel::BookFontSet { caption, text }),
    })
}

fn validate_classic_loader_font(
    paths: &AppPaths,
    scenario_font: Option<&str>,
    registrations: &[LoaderGroupRegistration],
) -> Result<()> {
    resolve_classic_font_bundle(paths, scenario_font, registrations, registrations).map(drop)
}

pub(crate) fn load_named_graphics_image(
    name: &str,
    registrations: &[LoaderGroupRegistration],
    graphics: &Group,
) -> Result<ImageData> {
    Ok(resolve_named_graphics_image(name, registrations, graphics)?.image)
}

fn resolve_named_graphics_image(
    name: &str,
    registrations: &[LoaderGroupRegistration],
    graphics: &Group,
) -> Result<ResolvedGraphicsImage> {
    let selected = select_named_graphics_image_source(name, registrations, graphics)?;
    let image = decode_selected_loader(&selected.source)?;
    Ok(ResolvedGraphicsImage { image })
}

/// Higher-resolution stems for a named graphics resource, most detailed
/// first.
///
/// Graphics.c4g has no per-sheet scale metadata — DefCore `Scale=` covers
/// object definitions only — so this filename suffix is the opt-in channel a
/// remastered pack uses to ship, say, `Control@2x.png` beside the 1x art.
/// `FindSuitableFile` (`C4Group.cpp:1178-1206`) knows nothing about it, so
/// with no such file present resolution is byte-identical to the oracle's,
/// and the drawing code then recognises the sheet's scale from its exact
/// integer multiple of the 1x dimensions (`clonk_frontend::hud::GuiArtScale`).
pub(crate) const REMASTERED_GRAPHICS_STEM_SUFFIXES: [&str; 3] = ["@4x", "@3x", "@2x"];

pub(crate) fn select_named_graphics_image_source(
    name: &str,
    registrations: &[LoaderGroupRegistration],
    graphics: &Group,
) -> Result<SelectedGraphicsImageSource> {
    REMASTERED_GRAPHICS_STEM_SUFFIXES
        .into_iter()
        .find_map(|suffix| {
            select_oracle_graphics_image_source(&format!("{name}{suffix}"), registrations, graphics)
                .ok()
        })
        .map_or_else(
            || select_oracle_graphics_image_source(name, registrations, graphics),
            Ok,
        )
}

/// `C4GraphicsResource`'s own `FindSuitableFile` selection, verbatim.
fn select_oracle_graphics_image_source(
    name: &str,
    registrations: &[LoaderGroupRegistration],
    graphics: &Group,
) -> Result<SelectedGraphicsImageSource> {
    struct GraphicsCandidate<'a> {
        registration: &'a LoaderGroupRegistration,
        from_registration: bool,
    }

    let base = LoaderGroupRegistration {
        priority: 0,
        registration_order: 0,
        group: graphics.clone(),
    };
    let mut candidates = registrations
        .iter()
        .map(|registration| GraphicsCandidate {
            registration,
            from_registration: true,
        })
        .collect::<Vec<_>>();
    candidates.push(GraphicsCandidate {
        registration: &base,
        from_registration: false,
    });
    let mut selected: Option<(Group, GroupEntry, bool)> = None;
    for extension in ["bmp", "jpeg", "jpg", "png"] {
        let filename = format!("{name}.{extension}");
        candidates.sort_by(|left, right| {
            right
                .registration
                .priority
                .cmp(&left.registration.priority)
                // RegisterMainGroups iterates Game.GroupSet's later-first
                // order, then Files.RegisterGroup prepends each call. The
                // second reversal makes earlier Game registrations win.
                .then_with(|| {
                    left.registration
                        .registration_order
                        .cmp(&right.registration.registration_order)
                })
        });
        let mut candidate = None;
        for source in &candidates {
            if let Some(entry) = find_classic_named_entry(&source.registration.group, &filename)? {
                candidate = Some((
                    source.registration.priority,
                    source.registration.group.clone(),
                    entry,
                    source.from_registration,
                ));
                break;
            }
        }
        if let Some(candidate) = candidate {
            // FindSuitableFile never assigns its local `iPrio` after a hit.
            // Consequently every later extension replaces an earlier one,
            // even when it comes from a lower-priority group.
            selected = Some((candidate.1, candidate.2, candidate.3));
        }
    }
    let (group, entry, from_registration) =
        selected.with_context(|| format!("classic graphics resource `{name}` is unavailable"))?;
    Ok(SelectedGraphicsImageSource {
        source: SelectedLoaderSource { group, entry },
        from_registration,
    })
}

fn select_exact_graphics_source(
    filename: &str,
    registrations: &[LoaderGroupRegistration],
    graphics: &Group,
) -> Result<SelectedLoaderSource> {
    let base = LoaderGroupRegistration {
        priority: 0,
        registration_order: 0,
        group: graphics.clone(),
    };
    let mut candidates = registrations.iter().collect::<Vec<_>>();
    candidates.push(&base);
    candidates.sort_by(|left, right| {
        right
            .priority
            .cmp(&left.priority)
            .then_with(|| left.registration_order.cmp(&right.registration_order))
    });
    for candidate in candidates {
        if let Some(entry) = find_classic_named_entry(&candidate.group, filename)? {
            return Ok(SelectedLoaderSource {
                group: candidate.group.clone(),
                entry,
            });
        }
    }
    anyhow::bail!("classic graphics resource `{filename}` is unavailable")
}

fn decode_game_graphics_image(
    source: &SelectedLoaderSource,
    palette: &GamePalette,
) -> Result<ImageData> {
    let is_bmp = source.extension_bytes().eq_ignore_ascii_case(b"bmp");
    if is_bmp {
        let bytes = source.read_bytes().with_context(|| {
            format!(
                "failed to read game graphic `{}` from {}",
                source.presentation_filename(),
                source.group.root().display()
            )
        })?;
        let bit_depth = bytes
            .get(28..30)
            .map(|value| u16::from_le_bytes([value[0], value[1]]));
        if bit_depth == Some(8) {
            let bitmap =
                clonk_resources::bitmap::IndexedBitmap::decode(&bytes).with_context(|| {
                    format!(
                        "failed to decode indexed game graphic `{}` from {}",
                        source.presentation_filename(),
                        source.group.root().display()
                    )
                })?;
            let mut pixels = Vec::with_capacity(bitmap.indices.len() * 4);
            for index in bitmap.indices {
                let color = palette.color(index);
                pixels.extend_from_slice(&[color.r, color.g, color.b, color.a]);
            }
            return Ok(ImageData::new(bitmap.width, bitmap.height, pixels));
        }
    }
    decode_selected_loader(source)
}

fn load_game_graphics_image(
    stem: &str,
    registrations: &[LoaderGroupRegistration],
    graphics: &Group,
    palette: &GamePalette,
) -> Result<ImageData> {
    let selected = select_named_graphics_image_source(stem, registrations, graphics)?;
    decode_game_graphics_image(&selected.source, palette)
}

fn load_game_palette(
    registrations: &[LoaderGroupRegistration],
    graphics: &Group,
) -> Result<GamePalette> {
    let palette_source = select_exact_graphics_source("C4.pal", registrations, graphics)?;
    let palette_bytes = palette_source.read_bytes().with_context(|| {
        format!(
            "failed to read game palette `{}` from {}",
            palette_source.presentation_filename(),
            palette_source.group.root().display()
        )
    })?;
    GamePalette::from_c4_pal(&palette_bytes).with_context(|| {
        format!(
            "game palette `{}` in {} is shorter than {} bytes",
            palette_source.presentation_filename(),
            palette_source.group.root().display(),
            GamePalette::BYTE_LEN
        )
    })
}

pub(crate) fn resolve_game_graphics_resources(
    registrations: &[LoaderGroupRegistration],
    graphics: &Group,
    cached_cursor_atlas: Option<Arc<CursorAtlas>>,
    liquid_animation_enabled: bool,
) -> Result<GameGraphicsResources> {
    let palette = load_game_palette(registrations, graphics)?;
    let load = |stem: &str| {
        load_game_graphics_image(stem, registrations, graphics, &palette)
            .with_context(|| format!("failed to load game graphics resource `{stem}`"))
    };
    let hud_graphics = HudGraphics {
        player: Some(load("Player")?),
        flag: Some(load("Flag")?),
        crew: Some(load("Crew")?),
        score: Some(load("Score")?),
        wealth: Some(load("Wealth")?),
        rank: Some(load("Rank")?),
        captain: Some(load("Captain")?),
        fire: Some(load("Fire")?),
        menu: Some(load("Menu")?),
        upper_board: Some(load("UpperBoard")?),
        logo: Some(load("Logo")?),
        construction: Some(load("Construction")?),
        energy: Some(load("Energy")?),
        magic: Some(load("Magic")?),
        arrow: Some(load("Arrow")?),
        exit: Some(load("Exit")?),
        hand: Some(load("Hand")?),
        build: Some(load("Build")?),
        energy_bars: Some(load("EnergyBars")?),
        select_mark: Some(load("SelectMark")?),
        control: Some(load("Control")?),
        gamepad: Some(load("Gamepad")?),
        background: Some(load("Background")?),
    };
    let options = Some(Arc::new(load("Options")?));
    // C4GraphicsResource::Init validates the selected Liquid surface as part
    // of its mandatory resource chain. ColorAnimation/Shader only control
    // whether the already-valid surface is installed for landscape drawing.
    let liquid_animation_image = load("Liquid")?;
    let liquid_animation = liquid_animation_enabled.then(|| Arc::new(liquid_animation_image));

    // PreInit fills fctCursors[0..7] from the sized files. Clear deliberately
    // keeps those cached facets. A valid game-local Cursor.* suppresses the
    // sized reload, then ReloadResolutionDependentFiles replaces that legacy
    // sheet with the already-cached selected size before any frame can use it.
    let cursor_atlas = match load_game_graphics_image("Cursor", registrations, graphics, &palette) {
        Ok(_) => cached_cursor_atlas.as_deref().cloned().context(
            "legacy Cursor.* suppressed sized cursor initialization before a cache existed",
        )?,
        Err(_) => {
            let cursor_stems = [
                "CursorXXXXXLarge",
                "CursorXXXXLarge",
                "CursorXXXLarge",
                "CursorXXLarge",
                "CursorXLarge",
                "CursorLarge",
                "CursorMedium",
                "CursorSmall",
            ];
            CursorAtlas::new(
                cursor_stems
                    .into_iter()
                    .map(|stem| load(stem).map(Some))
                    .collect::<Result<Vec<_>>>()?,
            )
        }
    };

    Ok(GameGraphicsResources {
        cursor_atlas: Arc::new(cursor_atlas),
        hud_graphics: Arc::new(hud_graphics),
        options,
        palette: Arc::new(palette),
        liquid_animation,
    })
}

pub(crate) fn load_game_graphics_resources(
    paths: Option<&AppPaths>,
    fallback: GameGraphicsResources,
    liquid_animation_enabled: bool,
    frontend: &FrontendScenario,
    definition_load: Option<&ScenarioDefinitionLoad>,
) -> Result<GameGraphicsResources> {
    let Some(paths) = paths else {
        return Ok(fallback);
    };
    let path = frontend
        .path
        .as_deref()
        .context("loaded scenario has no path for game graphics resolution")?;
    let scenario_group = open_group_path_for_folder_map(path)
        .with_context(|| format!("failed to open loaded scenario at {}", path.display()))?;
    // Graphics registration only consumes Origin, Definitions and Extra;
    // it must not resolve or validate unrelated presentation title data.
    let head = ScenarioLoaderHead::load_from_group_for_resource_registration(&scenario_group)
        .map_err(anyhow::Error::from)?;
    let fallback_definition_load;
    let definition_load = match definition_load {
        Some(definition_load) => definition_load,
        None => {
            fallback_definition_load = ScenarioDefinitionLoad::Seed {
                modules: Vec::new(),
                definition_root: None,
            };
            &fallback_definition_load
        }
    };
    let mut registrations =
        classic_loader_registrations(frontend, &scenario_group, &head, definition_load, paths)?;
    let first_definition_order = registrations
        .iter()
        .map(|registration| registration.registration_order)
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    registrations.extend(definition_graphics_source_registrations(
        &head,
        &scenario_group,
        definition_load,
        paths,
        first_definition_order,
    )?);
    let graphics_registrations = loader_graphics_registrations(&registrations)?;
    if graphics_registrations.is_empty() {
        return Ok(fallback);
    }
    let graphics = main_graphics_group(paths)?;
    resolve_game_graphics_resources(
        &graphics_registrations,
        &graphics,
        Some(Arc::clone(&fallback.cursor_atlas)),
        liquid_animation_enabled,
    )
}

fn load_classic_bitmap_font_image(
    filename: &str,
    registrations: &[LoaderGroupRegistration],
    graphics: &Group,
) -> Result<ImageData> {
    let mut candidates = loader_graphics_registrations(registrations)?;
    candidates.push(LoaderGroupRegistration {
        priority: 0,
        registration_order: 0,
        group: graphics.clone(),
    });
    candidates.sort_by(|left, right| {
        right
            .priority
            .cmp(&left.priority)
            .then_with(|| left.registration_order.cmp(&right.registration_order))
    });
    for candidate in candidates {
        if let Some(entry) = find_classic_named_entry(&candidate.group, filename)? {
            return decode_selected_loader(&SelectedLoaderSource {
                group: candidate.group,
                entry,
            })
            .with_context(|| format!("failed to decode classic bitmap font `{filename}`"));
        }
    }
    anyhow::bail!("classic bitmap font `{filename}` is unavailable")
}

pub(crate) fn find_classic_named_entry(
    group: &Group,
    filename: &str,
) -> Result<Option<GroupEntry>> {
    let filename = clonk_script::c4_string_bytes(filename);
    Ok(find_classic_named_entry_from_entries(
        group.entries()?,
        &filename,
    ))
}

pub(crate) fn find_classic_named_entry_from_entries(
    entries: Vec<GroupEntry>,
    filename: &[u8],
) -> Option<GroupEntry> {
    entries
        .into_iter()
        .find(|entry| classic_wildcard_match(filename, &entry.name_bytes))
}

pub(crate) fn loader_graphics_registrations(
    registrations: &[LoaderGroupRegistration],
) -> Result<Vec<LoaderGroupRegistration>> {
    let mut graphics = Vec::new();
    for registration in registrations {
        match open_child_flexible(&registration.group, Path::new("Graphics.c4g")) {
            Ok(Some(group)) => graphics.push(LoaderGroupRegistration {
                priority: registration.priority,
                registration_order: registration.registration_order,
                group,
            }),
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(
                    root = %registration.group.root().display(),
                    %error,
                    "failed to open optional registered Graphics.c4g"
                );
            }
        }
    }
    Ok(graphics)
}

/// One decoded active-scenario GUI sheet override selected from the
/// registered `Graphics.c4g` groups, ready to rebind the process-global
/// `C4GUI::Resource` sheet (`C4GUI::Resource::Load`, C4Gui.cpp:1085-1112).
/// `source` is the winning group/file identity standing in for the C++
/// group id, so a rebind reloads only when the winning group changes
/// (`C4GraphicsResource::LoadFile`, C4GraphicsResource.cpp:418-470).
#[derive(Clone)]
pub(crate) struct ClassicGuiSheetOverride {
    pub(crate) stem: &'static str,
    pub(crate) canonical_name: &'static str,
    pub(crate) source: String,
    pub(crate) image: ImageData,
}

/// `C4GUI::Resource::Load` over the refreshed group set: decoded sheet
/// overrides to apply, plus per-resource failures that must keep failing
/// typed before any pixels (C++ LoadFile logs and Init returns false).
#[derive(Default)]
pub(crate) struct ClassicGuiSheetResolution {
    pub(crate) overrides: Vec<ClassicGuiSheetOverride>,
    pub(crate) failures: HashMap<&'static str, String>,
}

pub(crate) fn resolve_classic_global_gui_sheet_overrides(
    registrations: &[LoaderGroupRegistration],
    graphics: &Group,
) -> ClassicGuiSheetResolution {
    let graphics_registrations = match loader_graphics_registrations(registrations) {
        Ok(registrations) => registrations,
        Err(error) => {
            let detail = format!("cannot inspect active graphics groups: {error}");
            return ClassicGuiSheetResolution {
                overrides: Vec::new(),
                failures: CLASSIC_GLOBAL_GUI_SHEETS
                    .into_iter()
                    .map(|(stem, _)| (stem, detail.clone()))
                    .collect(),
            };
        }
    };
    let mut resolution = ClassicGuiSheetResolution::default();
    for (stem, canonical_name) in CLASSIC_GLOBAL_GUI_SHEETS {
        match select_named_graphics_image_source(stem, &graphics_registrations, graphics) {
            Ok(selected) if selected.from_registration => {
                let source = format!(
                    "{}:{}",
                    selected.source.group.root().display(),
                    selected.source.presentation_filename()
                );
                match decode_selected_loader(&selected.source) {
                    Ok(image) => resolution.overrides.push(ClassicGuiSheetOverride {
                        stem,
                        canonical_name,
                        source,
                        image,
                    }),
                    Err(error) => {
                        resolution
                            .failures
                            .insert(stem, format!("{source}: {error}"));
                    }
                }
            }
            Ok(_) => {}
            Err(error) => {
                // A missing base resource is already represented by the
                // accepted initial bundle. Inspection errors here belong to
                // the refreshed group set and must not fall back silently.
                let detail = error.to_string();
                if detail != format!("classic graphics resource `{stem}` is unavailable") {
                    resolution.failures.insert(stem, detail);
                }
            }
        }
    }
    resolution
}

fn open_loader_group_with_prefix(prefix: &Path, specification: &str) -> Result<Group> {
    let candidate = concatenate_legacy_path(prefix, &clonk_script::c4_string_bytes(specification));
    open_group_path_for_folder_map(&candidate).map_err(|error| {
        let context = format!(
            "definition group `{specification}` is unavailable at {}",
            candidate.display()
        );
        match error {
            error
                if matches!(
                    &error,
                    GroupError::Missing(_)
                        | GroupError::NotDirectory(_)
                        | GroupError::EntryNotFound(_)
                ) || matches!(&error, GroupError::Io(error) if error.kind() == io::ErrorKind::NotFound) =>
            {
                anyhow::Error::new(ScenarioError::LegacyDefinitionNotFound {
                    path: candidate.display().to_string(),
                })
                .context(context)
            }
            error => anyhow::Error::new(error).context(context),
        }
    })
}

pub(crate) fn definition_graphics_source_registrations(
    head: &ScenarioLoaderHead,
    scenario_group: &Group,
    definition_load: &ScenarioDefinitionLoad,
    paths: &AppPaths,
    first_registration_order: usize,
) -> Result<Vec<LoaderGroupRegistration>> {
    let modules = effective_loader_definition_modules(head, definition_load)?;
    let definition_root = match definition_load {
        ScenarioDefinitionLoad::Seed {
            definition_root, ..
        }
        | ScenarioDefinitionLoad::Fixed {
            definition_root, ..
        } => definition_root.as_deref(),
    };
    let mut groups = Vec::new();
    if let Some(root) = definition_root {
        for module in &modules {
            groups.push(open_loader_group_with_prefix(root, module)?);
        }
    }
    let resolver = InstallDefinitionResolver::new(Some(Arc::new(paths.clone())));
    for module in &modules {
        groups.extend(
            resolver
                .resolve_definition_groups(scenario_group, module)
                .map_err(anyhow::Error::from)?,
        );
    }
    Ok(groups
        .into_iter()
        .enumerate()
        .map(|(index, group)| LoaderGroupRegistration {
            priority: 1,
            registration_order: first_registration_order.saturating_add(index),
            group,
        })
        .collect())
}

fn validate_loader_graphics_font_sources(registrations: &[LoaderGroupRegistration]) -> Result<()> {
    // Graphics.c4g children are registered with C4GSCnt_Graphics only.
    // Fonts.txt/vector files present there are ignored by C++, while bitmap
    // files remain eligible when an active FontDef names them exactly.
    loader_graphics_registrations(registrations).map(drop)
}

/// The registered group set of a joining network client at
/// `C4GraphicsResource::Init` time. `Extra.Init` runs before the join with
/// the pre-join DefinitionFilenames (C4Game.cpp:368-381), so only the Extra
/// root is registered and no per-definition Extra children exist; the
/// combined scenario and its Origin parents follow through OpenScenario
/// (C4Game.cpp:161-178), and the synchronized definition resources register
/// last at C4GSPrio_Definitions during InitGame (C4Game.cpp:2432-2441).
/// Returns (font-catalog, graphics) registrations: definition roots carry
/// C4GSCnt_DefinitionRoot content, which includes Graphics but not FontDefs
/// (C4GroupSet.h:37-51), so they extend only the graphics set.
pub(crate) fn client_network_gui_registrations(
    scenario: &FrontendScenario,
    scenario_group: &Group,
    head: &ScenarioLoaderHead,
    definition_groups: &[Group],
    paths: &AppPaths,
) -> Result<(Vec<LoaderGroupRegistration>, Vec<LoaderGroupRegistration>)> {
    let catalog = classic_loader_registrations(
        scenario,
        scenario_group,
        head,
        &ScenarioDefinitionLoad::Fixed {
            modules: Vec::new(),
            definition_root: None,
        },
        paths,
    )?;
    let first_definition_order = catalog
        .iter()
        .map(|registration| registration.registration_order)
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    let mut graphics_registrations = catalog.clone();
    graphics_registrations.extend(definition_groups.iter().enumerate().map(|(index, group)| {
        LoaderGroupRegistration {
            priority: 1,
            registration_order: first_definition_order.saturating_add(index),
            group: group.clone(),
        }
    }));
    Ok((catalog, graphics_registrations))
}

/// One re-callable `C4GraphicsResource::Init` pass over an active network
/// group set (C4GraphicsResource.cpp:278-292): the GUI sheet winners plus
/// the font bundle, with font failures latched typed per global GUI font
/// exactly like `loaded_game_global_gui_resolution`.
pub(crate) struct ActiveNetworkGuiResolution {
    pub(crate) overrides: Vec<ClassicGuiSheetOverride>,
    pub(crate) failures: HashMap<&'static str, String>,
    pub(crate) font_bundle: Option<ClassicFontBundle>,
}

pub(crate) fn resolve_active_network_gui_resolution(
    paths: &AppPaths,
    scenario_font: Option<&str>,
    catalog_registrations: &[LoaderGroupRegistration],
    graphics_registrations: &[LoaderGroupRegistration],
) -> Result<ActiveNetworkGuiResolution> {
    let graphics = main_graphics_group(paths)?;
    let mut font_failure =
        validate_classic_loader_font(paths, scenario_font, catalog_registrations)
            .and_then(|()| validate_loader_graphics_font_sources(catalog_registrations))
            .err()
            .map(|error| error.to_string());
    let font_bundle = match resolve_classic_font_bundle(
        paths,
        scenario_font,
        catalog_registrations,
        graphics_registrations,
    )
    .and_then(|bundle| {
        validate_loader_graphics_font_sources(graphics_registrations)?;
        Ok(bundle)
    }) {
        Ok(bundle) => Some(bundle),
        Err(error) => {
            font_failure = Some(error.to_string());
            None
        }
    };
    let mut resolution =
        resolve_classic_global_gui_sheet_overrides(graphics_registrations, &graphics);
    if let Some(detail) = font_failure {
        for name in CLASSIC_GLOBAL_GUI_FONTS {
            resolution.failures.insert(name, detail.clone());
        }
    }
    Ok(ActiveNetworkGuiResolution {
        overrides: resolution.overrides,
        failures: resolution.failures,
        font_bundle,
    })
}

/// The staged loading-refresh payload for a network client GO: the same
/// resolve→pending→apply flow local loading uses, so
/// `apply_pending_loading_resource_refresh` runs the identical typed
/// failure gate before any pixels.
pub(crate) struct ClientNetworkLoadingRefresh {
    pub(crate) resources: Option<LoaderResources>,
    pub(crate) tooltip_font: Option<Arc<clonk_graphics::clonk_font::ClonkFont>>,
    pub(crate) native_font_source: Option<ClassicNativeFontSource>,
    pub(crate) failures: HashMap<&'static str, String>,
    pub(crate) overrides: Vec<ClassicGuiSheetOverride>,
}

pub(crate) fn resolve_client_network_loading_refresh(
    assets: &FrontendAssets,
    paths: &AppPaths,
    scenario: &FrontendScenario,
    scenario_group: &Group,
    definition_groups: &[Group],
) -> Result<ClientNetworkLoadingRefresh> {
    let head = load_classic_scenario_loader_head(scenario_group, paths)?;
    let (catalog, graphics_registrations) = client_network_gui_registrations(
        scenario,
        scenario_group,
        &head,
        definition_groups,
        paths,
    )?;
    let resolution = resolve_active_network_gui_resolution(
        paths,
        Some(head.font()),
        &catalog,
        &graphics_registrations,
    )?;
    let resources = match resolution.font_bundle.as_ref() {
        Some(bundle) => {
            // The client's loader was initialized before the join from the
            // startup groups (C4Game.cpp:370-381); without an active winner
            // the progress bar keeps that startup sheet, mirroring
            // build_scenario_loader's refreshed arm.
            let refreshed_gui_progress = match resolution
                .overrides
                .iter()
                .find(|sheet| sheet.stem == "GUIProgress")
            {
                Some(sheet) => LoaderGuiProgress::GuiValid {
                    progress_bar: sheet.image.clone(),
                },
                None => {
                    let graphics = main_graphics_group(paths)?;
                    let startup_registrations = startup_loader_registrations(paths)?;
                    classic_loader_resources(assets, &startup_registrations, &graphics)?
                        .gui_progress()
                        .clone()
                }
            };
            Some(LoaderResources::from_gui_state(
                bundle.fonts.clone(),
                refreshed_gui_progress,
            )?)
        }
        None => None,
    };
    let native_font_source = resolution
        .font_bundle
        .as_ref()
        .and_then(|bundle| bundle.native_source.clone());
    let tooltip_font = resolution.font_bundle.map(|bundle| bundle.tooltip);
    Ok(ClientNetworkLoadingRefresh {
        resources,
        tooltip_font,
        native_font_source,
        failures: resolution.failures,
        overrides: resolution.overrides,
    })
}

/// The registered group set of a local/host round rebuilt from its retained
/// activation inputs: OpenScenario's parents/scenario/origin/Extra chain
/// plus the effective definition roots at C4GSPrio_Definitions
/// (C4Game.cpp:124-213,2432-2441). Returns the loader head plus
/// (font-catalog, graphics) registrations.
pub(crate) fn loaded_game_gui_registrations(
    frontend: &FrontendScenario,
    definition_load: Option<&ScenarioDefinitionLoad>,
    paths: &AppPaths,
) -> Result<(
    ScenarioLoaderHead,
    Vec<LoaderGroupRegistration>,
    Vec<LoaderGroupRegistration>,
)> {
    let path = frontend
        .path
        .as_deref()
        .context("active scenario has no path for GUI resolution")?;
    let scenario_group = open_group_path_for_folder_map(path)
        .with_context(|| format!("failed to open active scenario at {}", path.display()))?;
    let head = load_classic_scenario_loader_head(&scenario_group, paths)?;
    let fallback_definition_load;
    let definition_load = match definition_load {
        Some(definition_load) => definition_load,
        None => {
            fallback_definition_load = ScenarioDefinitionLoad::Seed {
                modules: Vec::new(),
                definition_root: None,
            };
            &fallback_definition_load
        }
    };
    let catalog =
        classic_loader_registrations(frontend, &scenario_group, &head, definition_load, paths)?;
    let first_definition_order = catalog
        .iter()
        .map(|registration| registration.registration_order)
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    let mut graphics_registrations = catalog.clone();
    graphics_registrations.extend(definition_graphics_source_registrations(
        &head,
        &scenario_group,
        definition_load,
        paths,
        first_definition_order,
    )?);
    Ok((head, catalog, graphics_registrations))
}

pub(crate) fn main_graphics_group(paths: &AppPaths) -> Result<Group> {
    let path = paths.planet_dir().join("Graphics.c4g");
    // C4LoaderScreen::Init reports a graphics group it cannot open as
    // IDS_PRC_NOGFXFILE, whose two arguments are C4CFN_Graphics -- the group's
    // own name, not the resolved path -- and the C4Group error
    // (src/C4LoaderScreen.cpp:61-66). The path stays out of the message for
    // the same reason it does natively: it is the install layout, not the
    // failure.
    Group::open(&path)
        .map_err(|error| anyhow::anyhow!("Error at graphics file Graphics.c4g: {error}"))
}

fn classic_loader_language_prefix(
    segment: &str,
    skip_whitespace: bool,
    source: &str,
) -> Result<String> {
    let segment = if skip_whitespace {
        segment.trim_start_matches([' ', '\t', '\r', '\n'])
    } else {
        segment
    };
    let end = segment.len().min(2);
    anyhow::ensure!(
        segment.is_char_boundary(end),
        "classic loader cannot represent {source}'s two-byte language segment without splitting UTF-8"
    );
    let code = &segment[..end];
    anyhow::ensure!(
        !code.contains('\0'),
        "classic loader cannot represent {source} language segment containing NUL"
    );
    Ok(code.to_string())
}

pub(crate) fn classic_loader_system_language() -> Result<&'static str> {
    Ok(if input::is_german_system() {
        "DE"
    } else {
        "US"
    })
}

pub(crate) fn load_classic_loader_config(paths: &AppPaths) -> Result<Option<Config>> {
    let bytes = match fs::read(paths.config_file()) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            if let Some(language) = paths.language_override() {
                let mut config = Config::new();
                config.set_in(Some("General"), "LanguageEx", language);
                return Ok(Some(config));
            }
            return Ok(None);
        }
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "classic loader cannot read configuration {}",
                    paths.config_file().display()
                )
            });
        }
    };
    anyhow::ensure!(
        !bytes.contains(&0),
        "classic loader configuration {} contains an embedded NUL",
        paths.config_file().display()
    );
    // C4Config retains the source file's legacy bytes, while the INI reader
    // projects them to Unicode for field parsing. Keep that conversion local
    // to the loader view so merely reading configuration never rewrites it.
    let projected = clonk_core::legacy_text::ensure_utf8(&bytes);
    let mut reader = io::Cursor::new(projected.as_bytes());
    let mut config = Config::from_reader(&mut reader).with_context(|| {
        format!(
            "classic loader cannot parse configuration {}",
            paths.config_file().display()
        )
    })?;
    if let Some(language) = paths.language_override() {
        config.set_in(Some("General"), "LanguageEx", language);
    }
    Ok(Some(config))
}

pub(crate) fn classic_loader_config_value<'a>(config: &'a Config, key: &str) -> Option<&'a str> {
    config
        .get_in(Some("General"), key)
        .or_else(|| config.get(key))
}

pub(crate) fn classic_loader_bounded_config_value<'a>(
    config: &'a Config,
    key: &str,
) -> Result<Option<&'a str>> {
    const CFG_MAX_STRING: usize = 1024;
    let value = classic_loader_config_value(config, key);
    if let Some(value) = value {
        anyhow::ensure!(
            value.len() <= CFG_MAX_STRING,
            "classic loader config string `{key}` exceeds the C++ {CFG_MAX_STRING}-byte capacity"
        );
        anyhow::ensure!(
            !value.contains('\0'),
            "classic loader config string `{key}` contains an embedded NUL"
        );
    }
    Ok(value)
}

pub(crate) fn classic_loader_language_sequence(paths: &AppPaths) -> Result<Vec<String>> {
    classic_configured_language_sequence(paths, false)
}

/// `C4Language::LoadLanguage` parses the same configured `LanguageEx`
/// sequence as component loading, except that its `SCopySegment` call skips
/// leading C++ whitespace on every segment. Keep that distinction local to
/// the process resource table; `C4ComponentHost` deliberately keeps the raw
/// two-byte prefixes.
pub(crate) fn classic_runtime_language_sequence(paths: &AppPaths) -> Result<Vec<String>> {
    classic_configured_language_sequence(paths, true)
}

pub(crate) fn load_classic_global_system_scripts(
    paths: &AppPaths,
    system: &Group,
) -> Result<Vec<(String, String)>> {
    // C4Config has already materialized LanguageEx before InitScriptEngine.
    // Preserve the app's best-effort fallback if an unusual platform locale
    // still cannot be projected into the strict two-byte sequence.
    let config = load_classic_loader_config(paths)?;
    let has_explicit_language = config.as_ref().is_some_and(|config| {
        ["LanguageEx", "Language"].into_iter().any(|key| {
            classic_loader_config_value(config, key).is_some_and(|value| !value.is_empty())
        })
    });
    let languages = match classic_configured_language_sequence_from_config(config.as_ref(), false) {
        Ok(languages) => languages,
        Err(_) if !has_explicit_language => startup_language_sequence(Some(paths)),
        Err(error) => return Err(error),
    };
    let language_packs = classic_language_packs(paths);
    let components = language_packs.component_groups(system, None, None);
    clonk_engine::scenario::load_system_scripts_with_components(system, &components, &languages)
        .map_err(anyhow::Error::from)
}

fn classic_configured_language_sequence(
    paths: &AppPaths,
    language_ex_skip_whitespace: bool,
) -> Result<Vec<String>> {
    let config = load_classic_loader_config(paths)?;
    classic_configured_language_sequence_from_config(config.as_ref(), language_ex_skip_whitespace)
}

fn classic_configured_language_sequence_from_config(
    config: Option<&Config>,
    language_ex_skip_whitespace: bool,
) -> Result<Vec<String>> {
    let language_ex = config
        .map(|config| classic_loader_bounded_config_value(config, "LanguageEx"))
        .transpose()?
        .flatten();
    if let Some(language_ex) = language_ex.filter(|value| !value.is_empty()) {
        return language_ex
            .split(',')
            .map(|segment| {
                classic_loader_language_prefix(segment, language_ex_skip_whitespace, "LanguageEx")
            })
            .collect();
    }

    let configured_primary = config
        .map(|config| classic_loader_bounded_config_value(config, "Language"))
        .transpose()?
        .flatten()
        .filter(|value| !value.is_empty());
    let primary = match configured_primary {
        Some(primary) => primary,
        None => classic_loader_system_language()?,
    };
    let mut codes = Vec::new();
    for segment in primary.split(',') {
        let code = classic_loader_language_prefix(segment, true, "Language")?;
        if !code.is_empty() {
            codes.push(code);
        }
    }
    // An unusual non-empty Language value can condense to an empty
    // LanguageEx. C4ComponentHost still performs one empty segment pass.
    if codes.is_empty() {
        codes.push(String::new());
    }
    Ok(codes)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RuntimeHelpCharset {
    Windows1252,
    Utf8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RuntimeLanguageTable {
    pub(crate) charset: RuntimeHelpCharset,
    pub(crate) entries: HashMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RuntimeLanguageBytesTable {
    pub(crate) entries: HashMap<String, Vec<u8>>,
}

pub(crate) fn generated_team_name_template(table: &RuntimeLanguageTable) -> LegacyCString {
    table
        .entries
        .get("IDS_MSG_TEAM")
        .and_then(|value| {
            let bytes = match table.charset {
                RuntimeHelpCharset::Windows1252 => value
                    .chars()
                    .map(runtime_cp1252_byte)
                    .collect::<Result<Vec<_>>>()
                    .expect("a decoded Windows-1252 resource value re-encodes losslessly"),
                RuntimeHelpCharset::Utf8 => value.as_bytes().to_vec(),
            };
            LegacyCString::from_bytes(bytes)
        })
        .unwrap_or_else(|| {
            LegacyCString::from_bytes(b"Team %d".to_vec())
                .expect("the shipped team-name resource contains no NUL")
        })
}

pub(crate) fn format_two_legacy_string_arguments(
    template: &[u8],
    first: &[u8],
    second: &[u8],
) -> Option<Vec<u8>> {
    let first_marker = template.windows(2).position(|window| window == b"%s")?;
    let after_first = first_marker + 2;
    let second_marker = template[after_first..]
        .windows(2)
        .position(|window| window == b"%s")?
        + after_first;
    let mut formatted = Vec::with_capacity(template.len() - 4 + first.len() + second.len());
    formatted.extend_from_slice(&template[..first_marker]);
    formatted.extend_from_slice(first);
    formatted.extend_from_slice(&template[after_first..second_marker]);
    formatted.extend_from_slice(second);
    formatted.extend_from_slice(&template[second_marker + 2..]);
    Some(formatted)
}

fn runtime_help_raw_table_value<'a>(bytes: &'a [u8], wanted: &[u8]) -> Option<&'a [u8]> {
    let mut remaining = bytes;
    while let Some(equals) = remaining.iter().position(|byte| *byte == b'=') {
        let key = &remaining[..equals];
        remaining = &remaining[equals + 1..];
        let value_end = remaining
            .iter()
            .position(|byte| matches!(*byte, b'\r' | b'\n'))
            .and_then(|line_end| {
                remaining[line_end..]
                    .iter()
                    .position(|byte| !matches!(*byte, b'\r' | b'\n'))
                    .map(|offset| line_end + offset)
            })
            .unwrap_or(remaining.len());
        let value_with_line_end = &remaining[..value_end];
        remaining = &remaining[value_end..];
        let value_end = value_with_line_end
            .iter()
            .rposition(|byte| !matches!(*byte, b'\r' | b'\n'))
            .map_or(0, |index| index + 1);
        if key == wanted {
            return Some(&value_with_line_end[..value_end]);
        }
    }
    None
}

fn runtime_help_table_charset(bytes: &[u8], source: &str) -> Result<RuntimeHelpCharset> {
    let raw = runtime_help_raw_table_value(bytes, b"IDS_LANG_CHARSET").unwrap_or_default();
    let charset = std::str::from_utf8(raw)
        .with_context(|| format!("classic language table {source} has a non-ASCII charset name"))?;
    if charset == "UTF-8" {
        return Ok(RuntimeHelpCharset::Utf8);
    }
    anyhow::ensure!(
        !charset.eq_ignore_ascii_case("UTF-8"),
        "runtime F1 help cannot reproduce non-canonical classic charset spelling {charset}"
    );
    let unsupported = [
        "SHIFTJIS",
        "HANGUL",
        "JOHAB",
        "CHINESEBIG5",
        "GREEK",
        "TURKISH",
        "VIETNAMESE",
        "HEBREW",
        "ARABIC",
        "BALTIC",
        "RUSSIAN",
        "THAI",
        "EASTEUROPE",
    ];
    anyhow::ensure!(
        !unsupported
            .iter()
            .any(|known| charset.eq_ignore_ascii_case(known)),
        "runtime F1 help cannot decode configured classic charset {charset}"
    );
    // C4Config::GetCharsetCodeName maps empty and unknown names to CP1252.
    Ok(RuntimeHelpCharset::Windows1252)
}

fn runtime_help_cp1252_char(byte: u8) -> Result<char> {
    let character = match byte {
        0x80 => '\u{20ac}',
        0x82 => '\u{201a}',
        0x83 => '\u{0192}',
        0x84 => '\u{201e}',
        0x85 => '\u{2026}',
        0x86 => '\u{2020}',
        0x87 => '\u{2021}',
        0x88 => '\u{02c6}',
        0x89 => '\u{2030}',
        0x8a => '\u{0160}',
        0x8b => '\u{2039}',
        0x8c => '\u{0152}',
        0x8e => '\u{017d}',
        0x91 => '\u{2018}',
        0x92 => '\u{2019}',
        0x93 => '\u{201c}',
        0x94 => '\u{201d}',
        0x95 => '\u{2022}',
        0x96 => '\u{2013}',
        0x97 => '\u{2014}',
        0x98 => '\u{02dc}',
        0x99 => '\u{2122}',
        0x9a => '\u{0161}',
        0x9b => '\u{203a}',
        0x9c => '\u{0153}',
        0x9e => '\u{017e}',
        0x9f => '\u{0178}',
        0x81 | 0x8d | 0x8f | 0x90 | 0x9d => {
            anyhow::bail!("undefined Windows-1252 byte 0x{byte:02x}")
        }
        other => char::from(other),
    };
    Ok(character)
}

pub(crate) fn runtime_cp1252_byte(character: char) -> Result<u8> {
    let byte = match character {
        '\u{0000}'..='\u{007f}' => character as u8,
        '\u{00a0}'..='\u{00ff}' => character as u8,
        '\u{20ac}' => 0x80,
        '\u{201a}' => 0x82,
        '\u{0192}' => 0x83,
        '\u{201e}' => 0x84,
        '\u{2026}' => 0x85,
        '\u{2020}' => 0x86,
        '\u{2021}' => 0x87,
        '\u{02c6}' => 0x88,
        '\u{2030}' => 0x89,
        '\u{0160}' => 0x8a,
        '\u{2039}' => 0x8b,
        '\u{0152}' => 0x8c,
        '\u{017d}' => 0x8e,
        '\u{2018}' => 0x91,
        '\u{2019}' => 0x92,
        '\u{201c}' => 0x93,
        '\u{201d}' => 0x94,
        '\u{2022}' => 0x95,
        '\u{2013}' => 0x96,
        '\u{2014}' => 0x97,
        '\u{02dc}' => 0x98,
        '\u{2122}' => 0x99,
        '\u{0161}' => 0x9a,
        '\u{203a}' => 0x9b,
        '\u{0153}' => 0x9c,
        '\u{017e}' => 0x9e,
        '\u{0178}' => 0x9f,
        _ => anyhow::bail!(
            "classic Windows-1252 text cannot encode scalar U+{:04X}",
            character as u32
        ),
    };
    Ok(byte)
}

pub(crate) fn runtime_flash_stored_bytes(
    text: &str,
    charset: RuntimeHelpCharset,
) -> Result<Vec<u8>> {
    const C4_MAX_TITLE_BYTES: usize = 512;
    let mut encoded = match charset {
        RuntimeHelpCharset::Windows1252 => text
            .chars()
            .map(runtime_cp1252_byte)
            .collect::<Result<Vec<_>>>()?,
        RuntimeHelpCharset::Utf8 => text.as_bytes().to_vec(),
    };
    if let Some(nul) = encoded.iter().position(|byte| *byte == 0) {
        encoded.truncate(nul);
    }
    encoded.truncate(C4_MAX_TITLE_BYTES);
    if charset == RuntimeHelpCharset::Utf8 {
        std::str::from_utf8(&encoded).context(
            "classic timed flash message's 512-byte SCopy truncation splits a UTF-8 scalar",
        )?;
    }
    Ok(encoded)
}

pub(crate) fn decode_runtime_flash_bytes(
    bytes: &[u8],
    charset: RuntimeHelpCharset,
) -> Result<String> {
    match charset {
        RuntimeHelpCharset::Windows1252 => bytes
            .iter()
            .map(|byte| runtime_help_cp1252_char(*byte))
            .collect::<Result<String>>(),
        RuntimeHelpCharset::Utf8 => Ok(std::str::from_utf8(bytes)?.to_string()),
    }
}

fn decode_runtime_help_language_table(
    bytes: &[u8],
    source: &str,
    charset: RuntimeHelpCharset,
) -> Result<String> {
    match charset {
        RuntimeHelpCharset::Windows1252 => bytes
            .iter()
            .map(|byte| runtime_help_cp1252_char(*byte))
            .collect::<Result<String>>()
            .with_context(|| format!("decoding classic language table {source} as Windows-1252")),
        RuntimeHelpCharset::Utf8 => std::str::from_utf8(bytes)
            .map(str::to_string)
            .with_context(|| format!("decoding classic language table {source} as UTF-8")),
    }
}

pub(crate) fn parse_runtime_help_language_table(
    bytes: &[u8],
    source: &str,
) -> Result<HashMap<String, String>> {
    Ok(parse_runtime_language_table(bytes, source)?.entries)
}

pub(crate) fn parse_runtime_language_table(
    bytes: &[u8],
    source: &str,
) -> Result<RuntimeLanguageTable> {
    let charset = runtime_help_table_charset(bytes, source)?;
    let entries = parse_runtime_help_language_table_with_charset(bytes, source, charset)?;
    Ok(RuntimeLanguageTable { charset, entries })
}

fn parse_runtime_help_language_table_with_charset(
    bytes: &[u8],
    source: &str,
    charset: RuntimeHelpCharset,
) -> Result<HashMap<String, String>> {
    anyhow::ensure!(
        !bytes.contains(&0),
        "classic language table {source} contains an embedded NUL"
    );
    let text = decode_runtime_help_language_table(bytes, source, charset)?;
    let mut table = HashMap::new();
    let mut remaining = text.as_str();
    while let Some(equals) = remaining.find('=') {
        let key = &remaining[..equals];
        remaining = &remaining[equals + 1..];
        let value_end = remaining
            .find(['\r', '\n'])
            .and_then(|line_end| {
                remaining[line_end..]
                    .find(|character| character != '\r' && character != '\n')
                    .map(|offset| line_end + offset)
            })
            .unwrap_or(remaining.len());
        let value_with_line_end = &remaining[..value_end];
        remaining = &remaining[value_end..];
        let value = value_with_line_end.trim_end_matches(['\r', '\n']);
        table
            .entry(key.to_string())
            .or_insert_with(|| value.replace("\\n", "\r\n"));
    }
    Ok(table)
}

/// The rest of SDL's physical scancode-name space, spelled exactly as
/// `SDL_GetScancodeName` returns it. `String2KeyCode` accepts every non-UNKNOWN
/// result (C4KeyboardInput.cpp:315-330), so a migrated `KeyConfig.txt` may name
/// any of these; the port resolves the ones this event backend can deliver.
/// Names SDL knows but winit cannot report stay unmapped and disable the
/// binding, which is the same dead-binding outcome as `SDL_SCANCODE_UNKNOWN`.
const EXTENDED_SDL_SCANCODE_NAMES: &[(&str, VirtualKeyCode)] = &[
    // Modifiers (SDL_SCANCODE_LCTRL..RGUI).
    ("Left Ctrl", VirtualKeyCode::ControlLeft),
    ("Right Ctrl", VirtualKeyCode::ControlRight),
    ("Left Shift", VirtualKeyCode::ShiftLeft),
    ("Right Shift", VirtualKeyCode::ShiftRight),
    ("Left Alt", VirtualKeyCode::AltLeft),
    ("Right Alt", VirtualKeyCode::AltRight),
    ("Left GUI", VirtualKeyCode::SuperLeft),
    ("Right GUI", VirtualKeyCode::SuperRight),
    // Media and volume keys.
    ("Mute", VirtualKeyCode::AudioVolumeMute),
    ("AudioMute", VirtualKeyCode::AudioVolumeMute),
    ("VolumeUp", VirtualKeyCode::AudioVolumeUp),
    ("VolumeDown", VirtualKeyCode::AudioVolumeDown),
    ("AudioNext", VirtualKeyCode::MediaTrackNext),
    ("AudioPrev", VirtualKeyCode::MediaTrackPrevious),
    ("AudioStop", VirtualKeyCode::MediaStop),
    ("AudioPlay", VirtualKeyCode::MediaPlayPause),
    ("MediaSelect", VirtualKeyCode::MediaSelect),
    // Application launch keys.
    ("Mail", VirtualKeyCode::LaunchMail),
    ("Computer", VirtualKeyCode::LaunchApp1),
    ("Calculator", VirtualKeyCode::LaunchApp2),
    ("Sleep", VirtualKeyCode::Sleep),
    ("Power", VirtualKeyCode::Power),
    ("Stop", VirtualKeyCode::Abort),
    // Application-control (browser) keys.
    ("AC Search", VirtualKeyCode::BrowserSearch),
    ("AC Home", VirtualKeyCode::BrowserHome),
    ("AC Back", VirtualKeyCode::BrowserBack),
    ("AC Forward", VirtualKeyCode::BrowserForward),
    ("AC Stop", VirtualKeyCode::BrowserStop),
    ("AC Refresh", VirtualKeyCode::BrowserRefresh),
    ("AC Bookmarks", VirtualKeyCode::BrowserFavorites),
    // Editing keys SDL names separately from the Ctrl chords.
    ("Cut", VirtualKeyCode::Cut),
    ("Copy", VirtualKeyCode::Copy),
    ("Paste", VirtualKeyCode::Paste),
    // International and non-US keys. SDL names the extra ISO key next to the
    // left Shift `NonUSBackslash`; winit reports it as OEM102. The JIS keys
    // carry SDL's positional `International*`/`Lang*` names.
    ("NonUSBackslash", VirtualKeyCode::IntlBackslash),
    ("International1", VirtualKeyCode::IntlRo),
    ("International2", VirtualKeyCode::KanaMode),
    ("International3", VirtualKeyCode::IntlYen),
    ("International4", VirtualKeyCode::Convert),
    ("International5", VirtualKeyCode::NonConvert),
    ("Lang1", VirtualKeyCode::Lang1),
    ("Lang2", VirtualKeyCode::Lang2),
    // Keypad names outside the arithmetic set handled above.
    ("Keypad 00", VirtualKeyCode::Numpad0),
    ("Keypad Equals", VirtualKeyCode::NumpadEqual),
];

fn extended_sdl_scancode_name(name: &str) -> Option<VirtualKeyCode> {
    EXTENDED_SDL_SCANCODE_NAMES
        .iter()
        .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
        .map(|(_, key)| *key)
}

fn runtime_key_name(name: &str) -> Option<VirtualKeyCode> {
    let name = name.trim();
    if name.len() == 1 {
        return match name.as_bytes()[0].to_ascii_uppercase() {
            b'0' => Some(VirtualKeyCode::Digit0),
            b'1' => Some(VirtualKeyCode::Digit1),
            b'2' => Some(VirtualKeyCode::Digit2),
            b'3' => Some(VirtualKeyCode::Digit3),
            b'4' => Some(VirtualKeyCode::Digit4),
            b'5' => Some(VirtualKeyCode::Digit5),
            b'6' => Some(VirtualKeyCode::Digit6),
            b'7' => Some(VirtualKeyCode::Digit7),
            b'8' => Some(VirtualKeyCode::Digit8),
            b'9' => Some(VirtualKeyCode::Digit9),
            b'A' => Some(VirtualKeyCode::KeyA),
            b'B' => Some(VirtualKeyCode::KeyB),
            b'C' => Some(VirtualKeyCode::KeyC),
            b'D' => Some(VirtualKeyCode::KeyD),
            b'E' => Some(VirtualKeyCode::KeyE),
            b'F' => Some(VirtualKeyCode::KeyF),
            b'G' => Some(VirtualKeyCode::KeyG),
            b'H' => Some(VirtualKeyCode::KeyH),
            b'I' => Some(VirtualKeyCode::KeyI),
            b'J' => Some(VirtualKeyCode::KeyJ),
            b'K' => Some(VirtualKeyCode::KeyK),
            b'L' => Some(VirtualKeyCode::KeyL),
            b'M' => Some(VirtualKeyCode::KeyM),
            b'N' => Some(VirtualKeyCode::KeyN),
            b'O' => Some(VirtualKeyCode::KeyO),
            b'P' => Some(VirtualKeyCode::KeyP),
            b'Q' => Some(VirtualKeyCode::KeyQ),
            b'R' => Some(VirtualKeyCode::KeyR),
            b'S' => Some(VirtualKeyCode::KeyS),
            b'T' => Some(VirtualKeyCode::KeyT),
            b'U' => Some(VirtualKeyCode::KeyU),
            b'V' => Some(VirtualKeyCode::KeyV),
            b'W' => Some(VirtualKeyCode::KeyW),
            b'X' => Some(VirtualKeyCode::KeyX),
            b'Y' => Some(VirtualKeyCode::KeyY),
            b'Z' => Some(VirtualKeyCode::KeyZ),
            b'-' => Some(VirtualKeyCode::Minus),
            b'=' => Some(VirtualKeyCode::Equal),
            b'[' => Some(VirtualKeyCode::BracketLeft),
            b']' => Some(VirtualKeyCode::BracketRight),
            b'\\' => Some(VirtualKeyCode::Backslash),
            b';' => Some(VirtualKeyCode::Semicolon),
            b'\'' => Some(VirtualKeyCode::Quote),
            b'`' => Some(VirtualKeyCode::Backquote),
            b',' => Some(VirtualKeyCode::Comma),
            b'.' => Some(VirtualKeyCode::Period),
            b'/' => Some(VirtualKeyCode::Slash),
            _ => None,
        };
    }

    let directional_key = if name.eq_ignore_ascii_case("Left") {
        Some(VirtualKeyCode::ArrowLeft)
    } else if name.eq_ignore_ascii_case("Right") {
        Some(VirtualKeyCode::ArrowRight)
    } else if name.eq_ignore_ascii_case("Up") {
        Some(VirtualKeyCode::ArrowUp)
    } else if name.eq_ignore_ascii_case("Down") {
        Some(VirtualKeyCode::ArrowDown)
    } else {
        None
    };
    if directional_key.is_some() {
        return directional_key;
    }

    let lower_name = name.to_ascii_lowercase();
    let named_key = if name.eq_ignore_ascii_case("Return") {
        Some(VirtualKeyCode::Enter)
    } else if name.eq_ignore_ascii_case("Escape") {
        Some(VirtualKeyCode::Escape)
    } else if name.eq_ignore_ascii_case("Backspace") {
        Some(VirtualKeyCode::Backspace)
    } else if name.eq_ignore_ascii_case("Tab") {
        Some(VirtualKeyCode::Tab)
    } else if name.eq_ignore_ascii_case("Space") {
        Some(VirtualKeyCode::Space)
    } else if name.eq_ignore_ascii_case("Pause") {
        Some(VirtualKeyCode::Pause)
    } else if name.eq_ignore_ascii_case("Insert") {
        Some(VirtualKeyCode::Insert)
    } else if name.eq_ignore_ascii_case("Home") {
        Some(VirtualKeyCode::Home)
    } else if name.eq_ignore_ascii_case("PageUp") {
        Some(VirtualKeyCode::PageUp)
    } else if name.eq_ignore_ascii_case("Delete") {
        Some(VirtualKeyCode::Delete)
    } else if name.eq_ignore_ascii_case("End") {
        Some(VirtualKeyCode::End)
    } else if name.eq_ignore_ascii_case("PageDown") {
        Some(VirtualKeyCode::PageDown)
    } else if name.eq_ignore_ascii_case("Minus") {
        Some(VirtualKeyCode::Minus)
    } else if name.eq_ignore_ascii_case("Equals") {
        Some(VirtualKeyCode::Equal)
    } else if name.eq_ignore_ascii_case("Left Bracket") {
        Some(VirtualKeyCode::BracketLeft)
    } else if name.eq_ignore_ascii_case("Right Bracket") {
        Some(VirtualKeyCode::BracketRight)
    } else if name.eq_ignore_ascii_case("Backslash") {
        Some(VirtualKeyCode::Backslash)
    } else if name.eq_ignore_ascii_case("Semicolon") {
        Some(VirtualKeyCode::Semicolon)
    } else if name.eq_ignore_ascii_case("Apostrophe") {
        Some(VirtualKeyCode::Quote)
    } else if name.eq_ignore_ascii_case("Grave") {
        Some(VirtualKeyCode::Backquote)
    } else if name.eq_ignore_ascii_case("Comma") {
        Some(VirtualKeyCode::Comma)
    } else if name.eq_ignore_ascii_case("Period") {
        Some(VirtualKeyCode::Period)
    } else if name.eq_ignore_ascii_case("Slash") {
        Some(VirtualKeyCode::Slash)
    } else if name.eq_ignore_ascii_case("CapsLock") {
        Some(VirtualKeyCode::CapsLock)
    } else if name.eq_ignore_ascii_case("PrintScreen") {
        Some(VirtualKeyCode::PrintScreen)
    } else if name.eq_ignore_ascii_case("ScrollLock") {
        Some(VirtualKeyCode::ScrollLock)
    } else if name.eq_ignore_ascii_case("NumLockClear") {
        Some(VirtualKeyCode::NumLock)
    } else if name.eq_ignore_ascii_case("Application") {
        Some(VirtualKeyCode::ContextMenu)
    } else if name.eq_ignore_ascii_case("Keypad /")
        || name.eq_ignore_ascii_case("Keypad Divide")
        || name.eq_ignore_ascii_case("KP_Divide")
    {
        Some(VirtualKeyCode::NumpadDivide)
    } else if name.eq_ignore_ascii_case("Keypad *")
        || name.eq_ignore_ascii_case("Keypad Multiply")
        || name.eq_ignore_ascii_case("KP_Multiply")
    {
        Some(VirtualKeyCode::NumpadMultiply)
    } else if name.eq_ignore_ascii_case("Keypad -")
        || name.eq_ignore_ascii_case("Keypad Minus")
        || name.eq_ignore_ascii_case("KP_Subtract")
    {
        Some(VirtualKeyCode::NumpadSubtract)
    } else if name.eq_ignore_ascii_case("Keypad +")
        || name.eq_ignore_ascii_case("Keypad Plus")
        || name.eq_ignore_ascii_case("KP_Add")
    {
        Some(VirtualKeyCode::NumpadAdd)
    } else if name.eq_ignore_ascii_case("Keypad Enter") || name.eq_ignore_ascii_case("KP_Enter") {
        Some(VirtualKeyCode::NumpadEnter)
    } else if name.eq_ignore_ascii_case("Keypad .")
        || name.eq_ignore_ascii_case("Keypad Period")
        || name.eq_ignore_ascii_case("KP_Decimal")
    {
        Some(VirtualKeyCode::NumpadDecimal)
    } else if name.eq_ignore_ascii_case("Keypad =")
        || name.eq_ignore_ascii_case("Keypad Equals")
        || name.eq_ignore_ascii_case("KP_Equal")
    {
        Some(VirtualKeyCode::NumpadEqual)
    } else if name.eq_ignore_ascii_case("Keypad ,")
        || name.eq_ignore_ascii_case("Keypad Comma")
        || name.eq_ignore_ascii_case("KP_Separator")
    {
        Some(VirtualKeyCode::NumpadComma)
    } else if let Some(number) = lower_name
        .strip_prefix("keypad ")
        .or_else(|| lower_name.strip_prefix("kp_"))
        .and_then(|number| number.parse::<u8>().ok())
    {
        match number {
            0 => Some(VirtualKeyCode::Numpad0),
            1 => Some(VirtualKeyCode::Numpad1),
            2 => Some(VirtualKeyCode::Numpad2),
            3 => Some(VirtualKeyCode::Numpad3),
            4 => Some(VirtualKeyCode::Numpad4),
            5 => Some(VirtualKeyCode::Numpad5),
            6 => Some(VirtualKeyCode::Numpad6),
            7 => Some(VirtualKeyCode::Numpad7),
            8 => Some(VirtualKeyCode::Numpad8),
            9 => Some(VirtualKeyCode::Numpad9),
            _ => None,
        }
    } else {
        None
    };
    if named_key.is_some() {
        return named_key;
    }
    if let Some(key) = extended_sdl_scancode_name(name) {
        return Some(key);
    }

    if let Some(number) = lower_name
        .strip_prefix('f')
        .filter(|number| !number.starts_with('0'))
        .and_then(|number| number.parse::<u8>().ok())
    {
        return match number {
            1 => Some(VirtualKeyCode::F1),
            2 => Some(VirtualKeyCode::F2),
            3 => Some(VirtualKeyCode::F3),
            4 => Some(VirtualKeyCode::F4),
            5 => Some(VirtualKeyCode::F5),
            6 => Some(VirtualKeyCode::F6),
            7 => Some(VirtualKeyCode::F7),
            8 => Some(VirtualKeyCode::F8),
            9 => Some(VirtualKeyCode::F9),
            10 => Some(VirtualKeyCode::F10),
            11 => Some(VirtualKeyCode::F11),
            12 => Some(VirtualKeyCode::F12),
            13 => Some(VirtualKeyCode::F13),
            14 => Some(VirtualKeyCode::F14),
            15 => Some(VirtualKeyCode::F15),
            16 => Some(VirtualKeyCode::F16),
            17 => Some(VirtualKeyCode::F17),
            18 => Some(VirtualKeyCode::F18),
            19 => Some(VirtualKeyCode::F19),
            20 => Some(VirtualKeyCode::F20),
            21 => Some(VirtualKeyCode::F21),
            22 => Some(VirtualKeyCode::F22),
            23 => Some(VirtualKeyCode::F23),
            24 => Some(VirtualKeyCode::F24),
            _ => None,
        };
    }

    // Names outside this backend-independent subset are intentionally left
    // to the full global key-registry port. C++ delegates them to mutually
    // incompatible Win32/X11/SDL name parsers; guessing an alias here could
    // activate a physical key that native treats as KEY_Default.
    None
}

fn runtime_raw_physical_key(raw: u32) -> RuntimePhysicalKey {
    if raw & 0x00ff_0000 == 0x0042_0000 {
        return RuntimePhysicalKey::Gamepad {
            slot: ((raw >> 8) & 0xff) as u8,
            button: (raw & 0xff) as u8,
        };
    }
    input::decode_platform_key_code(raw as i32)
        .map(RuntimePhysicalKey::Keyboard)
        .unwrap_or(RuntimePhysicalKey::Raw(raw))
}

fn parse_runtime_physical_key(raw: &str) -> RuntimePhysicalKey {
    if raw.eq_ignore_ascii_case("None") {
        return RuntimePhysicalKey::Disabled;
    }
    if let Some(hex) = raw.strip_prefix("\\x") {
        let digits = hex
            .bytes()
            .take_while(u8::is_ascii_hexdigit)
            .collect::<Vec<_>>();
        if !digits.is_empty() {
            if let Ok(digits) = std::str::from_utf8(&digits) {
                if let Ok(value) = u32::from_str_radix(digits, 16) {
                    return runtime_raw_physical_key(value);
                }
            }
        }
    }
    // Preserve the source oracle's `sscanf("Joy%dLeft") == 1` behavior:
    // after the integer assignment, a suffix mismatch still reports one
    // conversion, so every canonical `Joy` + integer spelling becomes Left.
    if let Some(rest) = raw.strip_prefix("Joy") {
        let number = rest
            .strip_prefix('-')
            .map(|rest| (true, rest))
            .unwrap_or((false, rest));
        let digits = number
            .1
            .bytes()
            .take_while(u8::is_ascii_digit)
            .collect::<Vec<_>>();
        if !digits.is_empty() {
            if let Ok(digits) = std::str::from_utf8(&digits) {
                if let Ok(mut gamepad) = digits.parse::<i32>() {
                    if number.0 {
                        gamepad = -gamepad;
                    }
                    return RuntimePhysicalKey::Gamepad {
                        slot: gamepad.wrapping_sub(1) as u8,
                        button: 1,
                    };
                }
            }
        }
    }
    if let Some(key) = runtime_key_name(raw) {
        return RuntimePhysicalKey::Keyboard(key);
    }
    let identifier_len = raw
        .bytes()
        .take_while(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'_' | b'-'))
        .count();
    runtime_key_name(&raw[..identifier_len])
        .map(RuntimePhysicalKey::Keyboard)
        // SDL_GetScancodeFromName returns SDL_SCANCODE_UNKNOWN/KEY_Default
        // for an unknown spelling. A nonempty custom vector containing zero
        // disables the named registration rather than falling back.
        .unwrap_or(RuntimePhysicalKey::Disabled)
}

fn parse_runtime_key_chord(raw: &str) -> Result<Option<RuntimeKeyChord>> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    let mut sections = raw.split('+').map(str::trim).peekable();
    let mut modifiers = ModifiersState::empty();
    let mut key = None;
    while let Some(section) = sections.next() {
        if sections.peek().is_some() {
            let modifier = if section.eq_ignore_ascii_case("Alt") {
                ModifiersState::ALT
            } else if section.eq_ignore_ascii_case("Ctrl") {
                ModifiersState::CONTROL
            } else if section.eq_ignore_ascii_case("Shift") {
                ModifiersState::SHIFT
            } else {
                anyhow::bail!("undefined key shift state `{section}`");
            };
            modifiers |= modifier;
        } else {
            key = Some(parse_runtime_physical_key(section));
        }
    }
    Ok(key.map(|physical| RuntimeKeyChord {
        physical,
        modifiers,
    }))
}

const RUNTIME_REGISTERED_GLOBAL_KEYS: &str = "MusicToggle SoundToggle Screenshot ScreenshotEx \
    ToggleChat ToggleShowHelp NetClientListDlgToggle MsgBoardScrollUp MsgBoardScrollDown \
    DbgModeToggle DbgShowVtxToggle DbgShowActionToggle DbgShowSolidMaskToggle GameSpeedUp \
    GameSlowDown FullscreenMenuLeft FullscreenMenuRight FullscreenMenuUp FullscreenMenuDown \
    FullscreenMenuOK FullscreenMenuCancel FullscreenMenuOpen FilmNextPlayer ChatOpen \
    ChatOpen2Allies ChatOpen2Say FreeViewScrollLeft FreeViewScrollRight FreeViewScrollUp \
    FreeViewScrollDown ScoreboardToggle GameAbort FullscreenPauseToggle ConsolePauseToggle \
    EditCursorModeToggle ToolsDlgGradeUp ToolsDlgGradeDown ToolsDlgPopMaterial ToolsDlgPopTextures \
    ToolsDlgIFTToggle ToolsDlgToolToggle EditCursorDelete ChartToggle NetObsNextPlayer \
    CtrlRateDown CtrlRateUp NetAllowJoinToggle NetStatsToggle StatsToggle";

fn runtime_player_key_slot(name: &str) -> Option<(usize, ControlBindingId)> {
    let rest = name.strip_prefix("Kbd")?;
    let (set, control) = rest.split_once("Key")?;
    let set_number = set.parse::<usize>().ok()?;
    let control_number = control.parse::<usize>().ok()?;
    if set != set_number.to_string() || control != control_number.to_string() {
        return None;
    }
    let set = set_number.checked_sub(1)?;
    let control = control_number.checked_sub(1)?;
    (set < KeyboardBindings::SET_COUNT)
        .then(|| ControlBindingId::ALL.get(control).copied())
        .flatten()
        .map(|control| (set, control))
}

fn runtime_registered_key_name(name: &str) -> bool {
    // `StatsToggle` is a port-only diagnostic binding, default-unbound and
    // gated behind opt-in `Graphics.ShowStats`. It exists because C++ draws a
    // single frame rate (src/C4UpperBoard.cpp:81-86) and that number is
    // `C4Game::FPS`, a count of executed *game* frames (C4Game.cpp:1915-1916,
    // sampled by `C4Game::Sec1Timer`, C4Game.cpp:1758-1762); C++ presents once
    // per tick so there it is also the render rate, while here smooth
    // presentation, the detail governor, automatic graphics skips and the
    // inactive gate all move the present rate independently — measured 35.7
    // simulation FPS held steady across a 9.03 -> 0.93 collapse in presented
    // frames, invisible to the ported counter. It deliberately registers after
    // every C++ action and yields the chord to all of them, so no shipped
    // binding changes meaning; keep it last.
    RUNTIME_REGISTERED_GLOBAL_KEYS
        .split_ascii_whitespace()
        .any(|registered| registered == name)
        || runtime_player_key_slot(name).is_some()
        || (1..=4)
            .any(|gamepad| (1..=12).any(|control| name == format!("Joy{gamepad}Btn{control}")))
}

pub(crate) fn parse_runtime_key_config(bytes: &[u8]) -> Result<RuntimeKeyConfig> {
    let text = String::from_utf8_lossy(bytes);
    let mut found_keys = false;
    let mut in_keys = false;
    let mut raw_overrides = BTreeMap::new();
    for raw_line in text.lines() {
        let line = raw_line.trim_start_matches('\u{feff}').trim_start();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        let structural = line
            .split_once(';')
            .map_or(line, |(head, _)| head)
            .trim_end();
        if structural.starts_with('[') && structural.ends_with(']') {
            if in_keys {
                break;
            }
            if &structural[1..structural.len() - 1] == "Keys" && !found_keys {
                found_keys = true;
                in_keys = true;
            }
            continue;
        }
        if !in_keys {
            continue;
        }
        let Some((name, value)) = line.split_once('=') else {
            tracing::warn!(line, "ignoring malformed custom key configuration line");
            continue;
        };
        let name = name.trim();
        raw_overrides
            .entry(name.to_string())
            .or_insert_with(|| value.trim().to_string());
    }

    let mut config = RuntimeKeyConfig::default();
    for (name, value) in raw_overrides {
        if !runtime_registered_key_name(&name) {
            tracing::warn!(key_name = name, "unexpected custom key configuration value");
            continue;
        }
        let mut codes = Vec::new();
        let mut corrupt = None;
        for raw_code in value.split(',') {
            match parse_runtime_key_chord(raw_code) {
                Ok(Some(code)) => codes.push(code),
                Ok(None) => break,
                Err(error) => {
                    corrupt = Some(error);
                    break;
                }
            }
        }
        // An empty custom vector falls back to the registration defaults.
        if !codes.is_empty() {
            if name == "NetObsNextPlayer" {
                config.net_observer_next_player = codes.clone();
            }
            if name == "ChartToggle" {
                config.chart_toggle = codes.clone();
            }
            config.overrides.insert(name.clone(), codes);
        }
        if let Some(error) = corrupt {
            tracing::warn!(key_name = name, %error, "failed to compile custom key configuration");
            break;
        }
    }
    Ok(config)
}

pub(crate) fn load_runtime_global_key_config(paths: Option<&AppPaths>) -> Result<RuntimeKeyConfig> {
    let Some(paths) = paths else {
        return Ok(RuntimeKeyConfig::default());
    };
    let extra_path = match mapped_classic_extra_group_path(paths) {
        Ok(Some(path)) => path,
        Ok(None) => return Ok(RuntimeKeyConfig::default()),
        Err(error) => {
            tracing::warn!(%error, "could not locate classic Extra.c4g custom key configuration");
            return Ok(RuntimeKeyConfig::default());
        }
    };
    let extra = match Group::open(&extra_path) {
        Ok(extra) => extra,
        Err(error) => {
            tracing::warn!(path = %extra_path.display(), %error, "could not open classic Extra.c4g custom key configuration");
            return Ok(RuntimeKeyConfig::default());
        }
    };
    let entries = match extra.entries() {
        Ok(entries) => entries,
        Err(error) => {
            tracing::warn!(path = %extra_path.display(), %error, "could not enumerate classic Extra.c4g custom key configuration");
            return Ok(RuntimeKeyConfig::default());
        }
    };
    let key_config = entries.into_iter().find(|entry| {
        entry.relative_path.components().count() == 1
            && entry
                .relative_path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case("KeyConfig.txt"))
    });
    let Some(key_config) = key_config else {
        return Ok(RuntimeKeyConfig::default());
    };
    let bytes = match extra.read_file(&key_config.relative_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::warn!(path = %extra_path.display(), %error, "could not read classic Extra.c4g custom key configuration");
            return Ok(RuntimeKeyConfig::default());
        }
    };
    parse_runtime_key_config(&bytes)
}

pub(crate) fn guard_runtime_global_key_config(paths: Option<&AppPaths>) -> Result<()> {
    load_runtime_global_key_config(paths).map(|_| ())
}

fn read_runtime_help_language_file(group: &Group, filename: &str) -> Option<Vec<u8>> {
    // C4Language::LoadStringTable treats every LoadEntryString failure as a
    // miss, closes this group, and continues with the next registered pack.
    // Group::read_file already performs C4Group's native-order,
    // ASCII-case-insensitive lookup for folder groups.
    let bytes = group.read_file(filename).ok()?;
    (!bytes.is_empty()).then_some(bytes)
}

pub(crate) fn load_runtime_language_table(
    paths: Option<&AppPaths>,
) -> Result<RuntimeLanguageTable> {
    const EMBEDDED_US: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../planet/System.c4g/LanguageUS.txt"
    ));

    let Some(paths) = paths else {
        // Asset-complete in-memory test/sandbox apps have no AppPaths. The
        // bytes are the same shipped System.c4g table used by production.
        return parse_runtime_language_table(EMBEDDED_US, "embedded LanguageUS.txt");
    };
    #[cfg(test)]
    {
        let counts = RUNTIME_LANGUAGE_TABLE_LOAD_COUNTS.get_or_init(|| Mutex::new(HashMap::new()));
        *counts
            .lock()
            .expect("runtime language load counter mutex")
            .entry(paths.system_group_path().to_path_buf())
            .or_default() += 1;
    }
    let system = Group::open(paths.system_group_path()).ok();
    let language_packs = classic_language_packs(paths);
    let system_groups = language_packs.system_groups_with_optional_local(system.as_ref());
    for code in classic_runtime_language_sequence(paths)? {
        let filename = format!("Language{code}.txt");
        for group in system_groups.groups() {
            if let Some(bytes) = read_runtime_help_language_file(group, &filename) {
                return parse_runtime_language_table(&bytes, &filename);
            }
        }
    }

    for group in system_groups.groups() {
        if let Some(bytes) = read_runtime_help_language_file(group, "LanguageUS.txt") {
            return parse_runtime_language_table(&bytes, "LanguageUS.txt");
        }
    }
    anyhow::bail!(
        "loading the classic US language fallback for F1 help: LanguageUS.txt is unavailable"
    )
}

#[cfg(test)]
static RUNTIME_LANGUAGE_TABLE_LOAD_COUNTS: OnceLock<Mutex<HashMap<PathBuf, usize>>> =
    OnceLock::new();

#[cfg(test)]
pub(crate) fn runtime_language_table_load_count(system_group_path: &Path) -> usize {
    RUNTIME_LANGUAGE_TABLE_LOAD_COUNTS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .expect("runtime language load counter mutex")
        .get(system_group_path)
        .copied()
        .unwrap_or_default()
}

pub(crate) fn embedded_runtime_language_table() -> &'static RuntimeLanguageTable {
    static TABLE: OnceLock<RuntimeLanguageTable> = OnceLock::new();
    TABLE.get_or_init(|| {
        load_runtime_language_table(None)
            .expect("the embedded LanguageUS.txt startup resource is valid")
    })
}

pub(crate) fn parse_runtime_language_bytes_table(
    bytes: &[u8],
    source: &str,
) -> Result<RuntimeLanguageBytesTable> {
    anyhow::ensure!(
        !bytes.contains(&0),
        "classic language table {source} contains an embedded NUL"
    );
    let mut entries = HashMap::new();
    let mut remaining = bytes;
    while let Some(equals) = remaining.iter().position(|byte| *byte == b'=') {
        let key = &remaining[..equals];
        remaining = &remaining[equals + 1..];
        let value_end = remaining
            .iter()
            .position(|byte| matches!(*byte, b'\r' | b'\n'))
            .and_then(|line_end| {
                remaining[line_end..]
                    .iter()
                    .position(|byte| !matches!(*byte, b'\r' | b'\n'))
                    .map(|offset| line_end + offset)
            })
            .unwrap_or(remaining.len());
        let value_with_line_end = &remaining[..value_end];
        remaining = &remaining[value_end..];
        let value_end = value_with_line_end
            .iter()
            .rposition(|byte| !matches!(*byte, b'\r' | b'\n'))
            .map_or(0, |index| index + 1);
        let Ok(key) = std::str::from_utf8(key) else {
            continue;
        };
        entries.entry(key.to_string()).or_insert_with(|| {
            let value = &value_with_line_end[..value_end];
            let mut decoded = Vec::with_capacity(value.len());
            let mut cursor = 0;
            while cursor < value.len() {
                if value.get(cursor..cursor + 2) == Some(b"\\n") {
                    decoded.extend_from_slice(b"\r\n");
                    cursor += 2;
                } else {
                    decoded.push(value[cursor]);
                    cursor += 1;
                }
            }
            decoded
        });
    }
    Ok(RuntimeLanguageBytesTable { entries })
}

pub(crate) fn load_runtime_language_bytes_table(
    paths: Option<&AppPaths>,
) -> Result<RuntimeLanguageBytesTable> {
    const EMBEDDED_US: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../planet/System.c4g/LanguageUS.txt"
    ));

    let Some(paths) = paths else {
        return parse_runtime_language_bytes_table(EMBEDDED_US, "embedded LanguageUS.txt");
    };
    let system = Group::open(paths.system_group_path()).ok();
    let language_packs = classic_language_packs(paths);
    let system_groups = language_packs.system_groups_with_optional_local(system.as_ref());
    for code in classic_runtime_language_sequence(paths)? {
        let filename = format!("Language{code}.txt");
        for group in system_groups.groups() {
            if let Some(bytes) = read_runtime_help_language_file(group, &filename) {
                return parse_runtime_language_bytes_table(&bytes, &filename);
            }
        }
    }
    for group in system_groups.groups() {
        if let Some(bytes) = read_runtime_help_language_file(group, "LanguageUS.txt") {
            return parse_runtime_language_bytes_table(&bytes, "LanguageUS.txt");
        }
    }
    anyhow::bail!("Language string table not loaded.")
}

pub(crate) fn runtime_resource_text_from_table(
    resources: &HashMap<String, String>,
    key: &str,
    fallback: &str,
) -> String {
    resources
        .get(key)
        .cloned()
        .unwrap_or_else(|| fallback.to_string())
}

/// Reads the port-owned enhanced-search wording out of an active language
/// table, keeping the shipped English for any key the table lacks
/// (clonk-org/clonk-rs#1175).
pub(crate) fn enhanced_search_resources(
    resources: &HashMap<String, String>,
) -> EnhancedSearchResources {
    let defaults = EnhancedSearchResources::default();
    EnhancedSearchResources {
        results: runtime_resource_text_from_table(
            resources,
            "IDS_MSG_SEARCHRESULTS",
            &defaults.results,
        ),
        no_matches: runtime_resource_text_from_table(
            resources,
            "IDS_MSG_SEARCHNOMATCHES",
            &defaults.no_matches,
        ),
        no_result: runtime_resource_text_from_table(
            resources,
            "IDS_MSG_SEARCHNORESULT",
            &defaults.no_result,
        ),
        clear_hint: runtime_resource_text_from_table(
            resources,
            "IDS_MSG_SEARCHCLEARHINT",
            &defaults.clear_hint,
        ),
        scenario: runtime_resource_text_from_table(
            resources,
            "IDS_MSG_SEARCHSCENARIO",
            &defaults.scenario,
        ),
        scenarios: runtime_resource_text_from_table(
            resources,
            "IDS_MSG_SEARCHSCENARIOS",
            &defaults.scenarios,
        ),
    }
}

pub(crate) fn format_resource_string(mut template: String, arguments: &[&str]) -> String {
    for argument in arguments {
        let placeholder = [
            template.find("%s"),
            template.find("%d"),
            template.find("%i"),
        ]
        .into_iter()
        .flatten()
        .min();
        let Some(placeholder) = placeholder else {
            break;
        };
        template.replace_range(placeholder..placeholder + 2, argument);
    }
    template
}

/// Substitute template placeholders without rescanning inserted arguments.
/// Goal names/descriptions are arbitrary definition text, so a literal `%s`
/// inside either argument must not consume the next template placeholder.
pub(crate) fn format_resource_string_with_opaque_arguments(
    template: String,
    arguments: &[&str],
) -> String {
    let mut output = String::with_capacity(template.len());
    let mut remainder = template.as_str();
    for argument in arguments {
        let placeholder = [
            remainder.find("%s"),
            remainder.find("%d"),
            remainder.find("%i"),
        ]
        .into_iter()
        .flatten()
        .min();
        let Some(placeholder) = placeholder else {
            break;
        };
        output.push_str(&remainder[..placeholder]);
        output.push_str(argument);
        remainder = &remainder[placeholder + 2..];
    }
    output.push_str(remainder);
    output
}

pub(crate) fn network_game_version_string(game: &str, version: [i32; 4], build: i32) -> String {
    format!(
        "{game} {}.{}.{}.{} [{build}]",
        version[0], version[1], version[2], version[3]
    )
}

pub(crate) fn open_external_http_url(url: &str) -> io::Result<()> {
    let scheme = url
        .split_once(':')
        .map(|(scheme, _)| scheme)
        .unwrap_or_default();
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "only HTTP and HTTPS hyperlinks can be opened",
        ));
    }

    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = std::process::Command::new("rundll32");
        command.arg("url.dll,FileProtocolHandler");
        command
    };
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = std::process::Command::new("xdg-open");
    #[cfg(not(any(target_os = "windows", target_os = "macos", unix)))]
    return Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "opening external hyperlinks is unsupported on this platform",
    ));

    command.arg(url).spawn().map(drop)
}

pub(crate) fn needed_material_resource_strings(table: &RuntimeLanguageTable) -> (String, String) {
    let get = |key: &str| {
        table
            .entries
            .get(key)
            .cloned()
            .unwrap_or_else(|| format!("[Undefined: {key}]"))
    };
    (get("IDS_CON_BUILDMATNEED"), get("IDS_CON_BUILDMATNONE"))
}

pub(crate) fn object_no_dig_resource_string(table: &RuntimeLanguageTable) -> String {
    table
        .entries
        .get("IDS_OBJ_NODIG")
        .cloned()
        .unwrap_or_else(|| "[Undefined: IDS_OBJ_NODIG]".to_string())
}

pub(crate) fn definition_overload_resource_string(table: &RuntimeLanguageTable) -> String {
    table
        .entries
        .get("IDS_PRC_DEFOVERLOAD")
        .cloned()
        .unwrap_or_else(|| "[Undefined: IDS_PRC_DEFOVERLOAD]".to_string())
}

/// `LoadResStr` bundle behind ConstructionCheck's red failure feedback:
/// IDS_OBJ_UNDEF, IDS_OBJ_NOCON, IDS_OBJ_NOROOM, IDS_OBJ_NOLEVEL,
/// IDS_OBJ_NOOTHER (C4Landscape.cpp:2131-2163).
pub(crate) fn construction_check_resource_strings(table: &RuntimeLanguageTable) -> [String; 5] {
    let get = |key: &str| {
        table
            .entries
            .get(key)
            .cloned()
            .unwrap_or_else(|| format!("[Undefined: {key}]"))
    };
    [
        get("IDS_OBJ_UNDEF"),
        get("IDS_OBJ_NOCON"),
        get("IDS_OBJ_NOROOM"),
        get("IDS_OBJ_NOLEVEL"),
        get("IDS_OBJ_NOOTHER"),
    ]
}

pub(crate) fn default_construction_check_feedback() -> [String; 5] {
    [
        "Structure %s undefined.".to_owned(),
        "%s cannot|be built.".to_owned(),
        "Not enough room!".to_owned(),
        "No level ground!".to_owned(),
        "%s is in the way.".to_owned(),
    ]
}

pub(crate) fn default_rank_resource_names(table: &RuntimeLanguageTable) -> Vec<String> {
    table
        .entries
        .get("IDS_GAME_DEFRANKS")
        .cloned()
        .unwrap_or_else(|| "[Undefined: IDS_GAME_DEFRANKS]".to_string())
        .split('|')
        .map(str::to_owned)
        .collect()
}

fn runtime_help_default_key_name(name: &str, index: usize) -> &'static str {
    match (name, index) {
        ("ToggleShowHelp", 0) => "F1",
        ("MusicToggle", 0) => "F3",
        ("SoundToggle", 0) => "Ctrl+F3",
        ("NetClientListDlgToggle", 0) => "F4",
        ("ChatOpen", 0) => "Return",
        ("ChatOpen", 1) => "F2",
        ("MsgBoardScrollUp", 0) => "Shift+Up",
        ("MsgBoardScrollDown", 0) => "Shift+Down",
        ("ToggleChat", 0) => "Alt+C",
        ("ScoreboardToggle", 0) => "Tab",
        // C4Game::InitKeyboard registers the observer-menu opener on K_SPACE
        // (C4Game.cpp:3428); C4FullScreen::ViewportCheck renders its name.
        ("FullscreenMenuOpen", 0) => "Space",
        ("Screenshot", 0) => "F9",
        ("ScreenshotEx", 0) => "Ctrl+F9",
        ("GameSpeedUp", 0) => {
            if cfg!(target_os = "windows") {
                "Shift+Add"
            } else if cfg!(target_os = "linux") {
                "Shift+KP_Add"
            } else {
                "Shift+Keypad +"
            }
        }
        // The oracle asks for GameSpeedDown, but registration is named
        // GameSlowDown. GetKeyCodeNameByKeyName therefore returns empty.
        ("GameSpeedDown", 0) => "",
        ("DbgModeToggle", 0) => "Ctrl+F5",
        ("DbgShowVtxToggle", 0) => "Ctrl+F6",
        ("DbgShowActionToggle", 0) => "Ctrl+F7",
        ("DbgShowSolidMaskToggle", 0) => "Ctrl+F8",
        _ => "",
    }
}

fn validate_runtime_help_line_buffers(left: &str, right: &str) -> Result<()> {
    const CPP_LINE_BUFFER_BYTES: usize = 2500;
    for (column, text) in [("left", left), ("right", right)] {
        for line in text.split(['\n', '|']) {
            anyhow::ensure!(
                line.len() <= CPP_LINE_BUFFER_BYTES,
                "classic runtime-help {column} line exceeds the C++ {CPP_LINE_BUFFER_BYTES}-byte TextOut buffer"
            );
        }
    }
    Ok(())
}

/// `C4KeyboardInput::GetKeyCodeNameByKeyName` renders the registered chord's
/// current code, so a `KeyConfig` override changes the displayed name
/// (C4GraphicsSystem.cpp:692-724). Modifiers precede the key, joined by `+`,
/// exactly like `C4KeyCodeEx::ToString`.
pub(crate) fn runtime_help_key_name(
    config: Option<&RuntimeKeyConfig>,
    name: &str,
    index: usize,
) -> String {
    let Some(chord) = config
        .and_then(|config| config.override_for(name))
        .and_then(|chords| chords.get(index))
    else {
        return runtime_help_default_key_name(name, index).to_string();
    };
    let RuntimePhysicalKey::Keyboard(key) = chord.physical else {
        // A gamepad override has no keyboard name, which is the empty string
        // `GetKeyCodeNameByKeyName` yields for an unresolvable code.
        return String::new();
    };
    let mut label = String::new();
    for (active, modifier) in [
        (chord.modifiers.shift_key(), "Shift"),
        (chord.modifiers.control_key(), "Ctrl"),
        (chord.modifiers.alt_key(), "Alt"),
    ] {
        if active {
            label.push_str(modifier);
            label.push('+');
        }
    }
    label.push_str(&crate::control_options::format_key_label(key));
    label
}

pub(crate) fn build_runtime_help_columns(
    table: &HashMap<String, String>,
) -> Result<RuntimeHelpColumns> {
    build_runtime_help_columns_with_keys(table, None)
}

pub(crate) fn build_runtime_help_columns_with_keys(
    table: &HashMap<String, String>,
    keys: Option<&RuntimeKeyConfig>,
) -> Result<RuntimeHelpColumns> {
    let text = |key: &str| {
        table
            .get(key)
            .cloned()
            .unwrap_or_else(|| format!("[Undefined: {key}]"))
    };
    let key = |name: &str, index: usize| runtime_help_key_name(keys, name, index);
    let left = format!(
        "[{}]\n\n<c ffff00>{}</c> - {}\n<c ffff00>{}</c> - {}\n<c ffff00>{}</c> - {}\n<c ffff00>{}</c> - {}\n\n<c ffff00>{}/{}</c> - {}\n<c ffff00>{}</c> - {}\n<c ffff00>{}</c> - {}\n\n<c ffff00>{}</c> - {}\n\n<c ffff00>{}</c> - {}\n\n<c ffff00>{}</c> - {}\n<c ffff00>{}</c> - {}\n",
        text("IDS_CTL_GAMEFUNCTIONS"),
        key("ToggleShowHelp", 0),
        text("IDS_CON_HELP"),
        key("MusicToggle", 0),
        text("IDS_CTL_MUSIC"),
        key("SoundToggle", 0),
        text("IDS_CTL_SOUND"),
        key("NetClientListDlgToggle", 0),
        text("IDS_DLG_NETWORK"),
        key("ChatOpen", 1),
        key("ChatOpen", 0),
        text("IDS_CTL_SENDMESSAGE"),
        key("MsgBoardScrollUp", 0),
        text("IDS_CTL_MESSAGEBOARDBACK"),
        key("MsgBoardScrollDown", 0),
        text("IDS_CTL_MESSAGEBOARDFORWARD"),
        key("ToggleChat", 0),
        text("IDS_CTL_IRCCHAT"),
        key("ScoreboardToggle", 0),
        text("IDS_CTL_SCOREBOARD"),
        key("Screenshot", 0),
        text("IDS_CTL_SCREENSHOT"),
        key("ScreenshotEx", 0),
        text("IDS_CTL_SCREENSHOTEX"),
    );
    let right = format!(
        "\n\n<c ffff00>{}</c> - {}\n<c ffff00>{}</c> - {}\n\n\n[Debug]\n\n<c ffff00>{}</c> - {}\n<c ffff00>{}</c> - Entrance+Vertices\n<c ffff00>{}</c> - Actions/Commands/Pathfinder\n<c ffff00>{}</c> - SolidMasks\n",
        key("GameSpeedUp", 0),
        text("IDS_CTL_GAMESPEEDUP"),
        key("GameSpeedDown", 0),
        text("IDS_CTL_GAMESPEEDDOWN"),
        key("DbgModeToggle", 0),
        text("IDS_CTL_DEBUGMODE"),
        key("DbgShowVtxToggle", 0),
        key("DbgShowActionToggle", 0),
        key("DbgShowSolidMaskToggle", 0),
    );
    validate_runtime_help_line_buffers(&left, &right)?;
    Ok(RuntimeHelpColumns { left, right })
}

pub(crate) fn build_runtime_flash_resources(table: &RuntimeLanguageTable) -> RuntimeFlashResources {
    let text = |key: &str| {
        table
            .entries
            .get(key)
            .cloned()
            .unwrap_or_else(|| format!("[Undefined: {key}]"))
    };
    RuntimeFlashResources {
        charset: table.charset,
        music: text("IDS_CTL_MUSIC"),
        speed: text("IDS_MSG_SPEED"),
        debug_mode: text("IDS_CTL_DEBUGMODE"),
        debug_mode_not_allowed: text("IDS_MSG_DEBUGMODENOTALLOWED"),
        no_debug_mode: text("IDS_MSG_NODEBUGMODE"),
        on: text("IDS_CTL_ON"),
        off: text("IDS_CTL_OFF"),
        observer_menu: text("IDS_MSG_PRESSORPUSHANYGAMEPADBUTT"),
    }
}

pub(crate) fn load_classic_scenario_loader_head(
    scenario_group: &Group,
    paths: &AppPaths,
) -> Result<ScenarioLoaderHead> {
    let languages = classic_loader_language_sequence(paths)?;
    let language_packs = classic_language_packs(paths);
    ScenarioLoaderHead::load_from_group_with_languages_and_packs(
        scenario_group,
        &languages,
        &language_packs,
    )
    .map_err(anyhow::Error::from)
}

pub(crate) fn parse_classic_loader_bool(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" => Some(true),
        "0" | "false" => Some(false),
        _ => None,
    }
}

pub(crate) fn parse_classic_loader_i32(raw: &str) -> Option<i32> {
    raw.trim().parse::<i32>().ok()
}

pub(crate) fn validate_classic_loader_graphics_config(paths: &AppPaths) -> Result<()> {
    let Some(config) = load_classic_loader_config(paths)? else {
        return Ok(());
    };
    for key in ["PointFiltering", "DisableGamma"] {
        if let Some(raw) = config.get_in(Some("Graphics"), key) {
            anyhow::ensure!(
                parse_classic_loader_bool(raw).is_some(),
                "classic loader graphics boolean `{key}={raw}` is invalid; expected 1, 0, true, or false"
            );
        }
    }
    if let Some(raw) = config.get_in(Some("Graphics"), "Scale") {
        anyhow::ensure!(
            parse_classic_loader_i32(raw).is_some(),
            "classic loader graphics scale `Scale={raw}` is not a valid decimal i32"
        );
    }
    Ok(())
}

pub(crate) fn load_classic_loader_gamma_from_native(
    config: &[u8],
) -> Option<clonk_graphics::GammaRamp> {
    if load_advanced_renderer_config(config).disable_gamma {
        return None;
    }
    Some(clonk_graphics::GammaRamp::from_control_points([
        startup_config_integer(config, "Graphics", "Gamma1", 0x000000) as u32,
        startup_config_integer(config, "Graphics", "Gamma2", 0x808080) as u32,
        startup_config_integer(config, "Graphics", "Gamma3", 0xffffff) as u32,
    ]))
}

pub(crate) fn load_classic_loader_gamma(
    paths: Option<&AppPaths>,
) -> Option<clonk_graphics::GammaRamp> {
    load_classic_loader_gamma_from_native(&load_native_config_bytes(paths))
}

pub(crate) fn startup_loader_registrations(
    paths: &AppPaths,
) -> Result<Vec<LoaderGroupRegistration>> {
    match mapped_classic_extra_group_path(paths)? {
        Some(extra_path) => match Group::open(&extra_path) {
            Ok(group) => Ok(vec![LoaderGroupRegistration {
                priority: 2,
                registration_order: 0,
                group,
            }]),
            Err(error) => {
                tracing::warn!(
                    path = %extra_path.display(),
                    %error,
                    "failed to open optional startup Extra.c4g"
                );
                Ok(Vec::new())
            }
        },
        None => Ok(Vec::new()),
    }
}

fn classic_loader_resources(
    assets: &FrontendAssets,
    registrations: &[LoaderGroupRegistration],
    graphics: &Group,
) -> Result<LoaderResources> {
    let fonts = assets
        .clonk_fonts
        .clone()
        .context("CStdFont-faithful loader fonts are unavailable")?;
    let graphics_registrations = loader_graphics_registrations(registrations)?;
    let progress = load_named_graphics_image("GUIProgress", &graphics_registrations, graphics)?;
    LoaderResources::new(fonts, progress)
}

/// `C4LoaderScreen::Init` answers a missing `C4CFN_StartupBackgroundMain` by
/// seeking the requested extensions in the main graphics group and then the
/// general `Loader*` wildcard, so a classic pack without `LoaderGoldmine1.png`
/// still has a startup background (src/C4LoaderScreen.cpp:57-87). The search
/// runs over the already-open group; the GroupSet pass belongs to the loader
/// screen itself (`build_startup_loader`).
fn startup_menu_background_wildcard(graphics: &GraphicsResource) -> Option<ImageData> {
    let selected =
        select_loader_with_safe_random(&[], graphics.group(), STARTUP_LOADER_SPECIFICATION).ok()?;
    decode_selected_loader(&selected).ok()
}

pub(crate) fn build_startup_loader(
    paths: &AppPaths,
    assets: &FrontendAssets,
) -> Result<ClassicLoaderSetup> {
    validate_classic_loader_graphics_config(paths)?;
    // C4LoaderScreen::Init opens Graphics.c4g only when the GroupSet pass found
    // no loader (src/C4LoaderScreen.cpp:59-67); this opens it unconditionally
    // because the loader's GUI resources are read from it below. That is not a
    // reachable divergence: natively those same resources come from
    // Game.GraphicsResource, which has already opened the group and failed the
    // game before Init runs, so neither engine reaches a loader screen with an
    // unopenable Graphics.c4g.
    let graphics = main_graphics_group(paths)?;
    let registrations = startup_loader_registrations(paths)?;
    validate_classic_loader_font(paths, None, &registrations)?;
    validate_loader_graphics_font_sources(&registrations)?;
    let tier = highest_loader_tier(&registrations)?;
    let selected = select_loader_with_safe_random(&tier, &graphics, STARTUP_LOADER_SPECIFICATION)?;
    let selected_filename = selected.presentation_filename();
    let selection = LoaderSelection::startup(selected_filename)?;
    let background = decode_selected_loader(&selected)?;
    let resources = classic_loader_resources(assets, &registrations, &graphics)?;
    let screen = LoaderScreen::new(
        selection,
        background,
        resources.clone(),
        LoaderState::initial("Loading..."),
    )?;
    Ok(ClassicLoaderSetup {
        screen,
        initial_tooltip_font: None,
        initial_native_font_source: assets.startup_native_font_source.clone(),
        refreshed_resources: resources,
        refreshed_tooltip_font: None,
        refreshed_native_font_source: assets.startup_native_font_source.clone(),
        refreshed_global_gui_failures: HashMap::new(),
        refreshed_gui_sheet_overrides: Vec::new(),
        refreshed_player_icon: None,
        refreshed_crew_icon: None,
        scenario_title: None,
    })
}

pub(crate) fn build_scenario_loader(
    scenario: &FrontendScenario,
    definition_load: &ScenarioDefinitionLoad,
    paths: &AppPaths,
    assets: &FrontendAssets,
) -> Result<ClassicLoaderSetup> {
    validate_classic_loader_graphics_config(paths)?;
    let path = scenario
        .path
        .as_deref()
        .context("classic scenario loader has no scenario path")?;
    let scenario_group = open_group_path_for_folder_map(path)
        .with_context(|| format!("failed to open scenario group at {}", path.display()))?;
    let head = load_classic_scenario_loader_head(&scenario_group, paths)?;
    let graphics = main_graphics_group(paths)?;
    let registrations =
        classic_loader_registrations(scenario, &scenario_group, &head, definition_load, paths)?;
    validate_loader_graphics_font_sources(&registrations)?;
    let tier = highest_loader_tier(&registrations)?;
    let selected =
        select_loader_with_safe_random(&tier, &graphics, head.loader().configured_specification())?;
    let selected_filename = selected.presentation_filename();
    let selection =
        LoaderSelection::scenario(head.loader().configured_specification(), selected_filename)?;
    let background = decode_selected_loader(&selected)?;

    // C4LoaderScreen::Init reinitializes the process-global fonts after the
    // scenario/parent/origin groups have been registered, but before any
    // definition roots exist. GUIProgress remains the already-live startup
    // sheet until the later full GraphicsResource::Init.
    let startup_registrations = startup_loader_registrations(paths)?;
    let startup_resources = classic_loader_resources(assets, &startup_registrations, &graphics)?;
    let initial_font_bundle =
        resolve_classic_font_bundle(paths, Some(head.font()), &registrations, &registrations)?;
    let initial_resources = LoaderResources::from_gui_state(
        initial_font_bundle.fonts.clone(),
        startup_resources.gui_progress().clone(),
    )?;
    let initial_tooltip_font = Some(initial_font_bundle.tooltip.clone());
    let initial_native_font_source = initial_font_bundle.native_source.clone();
    let first_definition_order = registrations
        .iter()
        .map(|registration| registration.registration_order)
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    let mut refreshed_registrations = registrations.clone();
    refreshed_registrations.extend(definition_graphics_source_registrations(
        &head,
        &scenario_group,
        definition_load,
        paths,
        first_definition_order,
    )?);
    let mut refreshed_gui_resolution =
        resolve_classic_global_gui_sheet_overrides(&refreshed_registrations, &graphics);
    let refreshed_font_bundle = resolve_classic_font_bundle(
        paths,
        Some(head.font()),
        &registrations,
        &refreshed_registrations,
    );
    let refreshed_font_bundle = match refreshed_font_bundle.and_then(|bundle| {
        validate_loader_graphics_font_sources(&refreshed_registrations)?;
        Ok(bundle)
    }) {
        Ok(bundle) => Some(bundle),
        Err(error) => {
            let detail = error.to_string();
            for name in CLASSIC_GLOBAL_GUI_FONTS {
                refreshed_gui_resolution
                    .failures
                    .insert(name, detail.clone());
            }
            None
        }
    };
    // The complete GUI resource reload occurs only after the worker's
    // RefreshResources event. Definition Graphics groups may replace bitmap
    // font images then, but cannot make a definition-only image available to
    // the pre-definition loader initialization above. The loader progress
    // bar is C4GUI::GetRes()->fctProgressBar (C4LoaderScreen.cpp:147), so
    // the refresh rebinds it to the winning active GUIProgress sheet.
    let refreshed_gui_progress = refreshed_gui_resolution
        .overrides
        .iter()
        .find(|sheet| sheet.stem == "GUIProgress")
        .map(|sheet| LoaderGuiProgress::GuiValid {
            progress_bar: sheet.image.clone(),
        })
        .unwrap_or_else(|| initial_resources.gui_progress().clone());
    let refreshed_resources = match refreshed_font_bundle.as_ref() {
        Some(bundle) => {
            LoaderResources::from_gui_state(bundle.fonts.clone(), refreshed_gui_progress)?
        }
        None => initial_resources.clone(),
    };
    let refreshed_native_font_source = refreshed_font_bundle
        .as_ref()
        .and_then(|bundle| bundle.native_source.clone());
    let refreshed_tooltip_font = refreshed_font_bundle.map(|bundle| bundle.tooltip);
    let (refreshed_player_icon, refreshed_crew_icon) =
        match (|| -> Result<(ImageData, ImageData)> {
            let registrations = loader_graphics_registrations(&refreshed_registrations)?;
            let palette = load_game_palette(&registrations, &graphics)?;
            let player = load_game_graphics_image("Player", &registrations, &graphics, &palette)
                .context("failed to load the active Player game graphic")?;
            let crew = load_game_graphics_image("Crew", &registrations, &graphics, &palette)
                .context("failed to load the active Crew game graphic")?;
            Ok((player, crew))
        })() {
            Ok((player, crew)) => (Some(player), Some(crew)),
            // The ordinary loader screen does not consume gameplay icons.
            // Network-host staging below requires both before opening a
            // socket, so this optional probe cannot hide a live lobby gap.
            Err(_) => (None, None),
        };
    let screen = LoaderScreen::new(
        selection,
        background,
        initial_resources,
        LoaderState::initial(head.scenario_title()),
    )?;
    Ok(ClassicLoaderSetup {
        screen,
        initial_tooltip_font,
        initial_native_font_source,
        refreshed_resources,
        refreshed_tooltip_font,
        refreshed_native_font_source,
        refreshed_global_gui_failures: refreshed_gui_resolution.failures,
        refreshed_gui_sheet_overrides: refreshed_gui_resolution.overrides,
        refreshed_player_icon,
        refreshed_crew_icon,
        scenario_title: Some(head.scenario_title().to_string()),
    })
}

pub(crate) struct SyncCheckState {
    pub(crate) local: HashMap<i32, SyncCheckPacket>,
    pub(crate) remote: HashMap<i32, SyncCheckPacket>,
}

impl SyncCheckState {
    pub(crate) fn new() -> Self {
        Self {
            local: HashMap::new(),
            remote: HashMap::new(),
        }
    }

    pub(crate) fn clear(&mut self) {
        self.local.clear();
        self.remote.clear();
    }

    pub(crate) fn record_local(
        &mut self,
        check: SyncCheckPacket,
    ) -> Option<(SyncCheckPacket, SyncCheckPacket)> {
        let frame = check.frame;
        let remote = self.remote.remove(&frame);
        self.local.insert(frame, check.clone());
        remote.map(|remote_check| (check, remote_check))
    }

    pub(crate) fn record_remote(
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

    pub(crate) fn prune_before(&mut self, threshold: i32) {
        self.local.retain(|&frame, _| frame >= threshold);
        self.remote.retain(|&frame, _| frame >= threshold);
    }
}

/// Host-local `C4Network2Client::iLastActivity` state. Native deliberately
/// keeps this outside the synchronized client core and does not serialize it.
#[derive(Debug, Default)]
pub(crate) struct NetworkClientActivity {
    pub(crate) last_frame: BTreeMap<i32, i32>,
}

impl NetworkClientActivity {
    pub(crate) fn replace_clients(&mut self, client_ids: impl IntoIterator<Item = i32>) {
        self.last_frame = client_ids
            .into_iter()
            .map(|client_id| (client_id, 0))
            .collect();
    }

    pub(crate) fn reset_client(&mut self, client_id: i32) {
        self.last_frame.insert(client_id, 0);
    }

    pub(crate) fn remove_client(&mut self, client_id: i32) {
        self.last_frame.remove(&client_id);
    }

    pub(crate) fn mark_activated(&mut self, client_id: i32, current_frame: i32) {
        self.last_frame.insert(client_id, current_frame);
    }

    pub(crate) fn clear(&mut self) {
        self.last_frame.clear();
    }

    pub(crate) fn deactivation_candidates(
        &mut self,
        activated_client_ids: impl IntoIterator<Item = i32>,
        player_client_ids: impl IntoIterator<Item = i32>,
        local_client_id: i32,
        current_frame: i32,
    ) -> Vec<i32> {
        let clients_with_players = player_client_ids.into_iter().collect::<HashSet<_>>();
        let mut candidates = Vec::new();
        for client_id in activated_client_ids {
            let last_frame = self.last_frame.entry(client_id).or_insert(0);
            if clients_with_players.contains(&client_id) {
                *last_frame = current_frame;
            }
            if client_id != local_client_id
                && i64::from(*last_frame) + i64::from(NETWORK_CLIENT_DEACTIVATION_DELAY)
                    < i64::from(current_frame)
            {
                candidates.push(client_id);
            }
        }
        candidates
    }
}

#[derive(Debug, Default)]
pub(crate) struct NetworkTickGate {
    pub(crate) ready: BTreeMap<Tick, Vec<NetworkControl>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NetworkControlWait {
    ReadyTick(Tick),
    PlayerResource { resource_id: i32 },
}

#[derive(Debug, Default)]
pub(crate) struct NetworkSyncGate {
    pub(crate) scheduled: BTreeMap<Tick, Vec<Vec<NetworkControl>>>,
}

#[derive(Debug)]
pub(crate) struct PendingRuntimeDynamicRequest {
    pub(crate) client_ids: HashSet<ClientId>,
    pub(crate) requested_control_tick: Tick,
    pub(crate) synchronize_queued: bool,
    pub(crate) synchronized_control_tick: Option<Tick>,
    pub(crate) save_generation: Option<u64>,
}

impl PendingRuntimeDynamicRequest {
    pub(crate) fn new(client_id: ClientId, requested_control_tick: Tick) -> Self {
        Self {
            client_ids: HashSet::from([client_id]),
            requested_control_tick,
            synchronize_queued: false,
            synchronized_control_tick: None,
            save_generation: None,
        }
    }

    pub(crate) fn include(&mut self, client_id: ClientId, requested_control_tick: Tick) {
        self.client_ids.insert(client_id);
        self.requested_control_tick = self.requested_control_tick.max(requested_control_tick);
        if self
            .synchronized_control_tick
            .is_some_and(|tick| tick < self.requested_control_tick)
        {
            self.synchronized_control_tick = None;
            self.save_generation = None;
        }
    }

    pub(crate) fn needs_synchronize(&self) -> bool {
        !self.synchronize_queued && self.synchronized_control_tick.is_none()
    }
}

pub(crate) fn published_runtime_dynamic_covers_request(
    dynamic: &clonk_engine::NetworkResourceCore,
    dynamic_tick: i32,
    requested_control_tick: Tick,
) -> bool {
    dynamic.resource_type != clonk_engine::NETWORK_RESOURCE_TYPE_NULL
        && dynamic_tick >= i32::try_from(requested_control_tick).unwrap_or(i32::MAX)
}

impl NetworkSyncGate {
    pub(crate) fn queue(&mut self, expected_tick: Tick, tick: Tick, controls: Vec<NetworkControl>) {
        self.scheduled
            .retain(|queued_tick, _| *queued_tick >= expected_tick);
        if tick < expected_tick {
            return;
        }
        self.scheduled.entry(tick).or_default().push(controls);
    }

    pub(crate) fn take_exact(&mut self, expected_tick: Tick) -> Vec<NetworkControl> {
        self.scheduled
            .retain(|queued_tick, _| *queued_tick >= expected_tick);
        self.scheduled
            .remove(&expected_tick)
            .unwrap_or_default()
            .into_iter()
            .flatten()
            .collect()
    }

    pub(crate) fn clear(&mut self) {
        self.scheduled.clear();
    }
}

impl NetworkTickGate {
    pub(crate) fn queue(&mut self, expected_tick: Tick, tick: Tick, controls: Vec<NetworkControl>) {
        self.ready
            .retain(|queued_tick, _| *queued_tick >= expected_tick);
        if tick < expected_tick {
            return;
        }
        self.ready.entry(tick).or_insert(controls);
    }

    pub(crate) fn take_exact_if_ready<F>(
        &mut self,
        expected_tick: Tick,
        pre_execute: F,
    ) -> Option<Vec<NetworkControl>>
    where
        F: FnOnce(&[NetworkControl]) -> bool,
    {
        self.ready
            .retain(|queued_tick, _| *queued_tick >= expected_tick);
        let controls = self.ready.get(&expected_tick)?;
        if !pre_execute(controls) {
            return None;
        }
        self.ready.remove(&expected_tick)
    }

    pub(crate) fn exact_is_ready_if<F>(&self, expected_tick: Tick, pre_execute: F) -> bool
    where
        F: FnOnce(&[NetworkControl]) -> bool,
    {
        self.ready
            .get(&expected_tick)
            .is_some_and(|controls| pre_execute(controls.as_slice()))
    }

    /// Number of consecutive ready control ticks beginning at `expected_tick`.
    /// C++ advances `iControlReady` only across a gap-free prefix, so a future
    /// out-of-order packet must not trigger catch-up by itself.
    pub(crate) fn contiguous_ready_behind_if<F>(&self, expected_tick: Tick, mut is_ready: F) -> u32
    where
        F: FnMut(&[NetworkControl]) -> bool,
    {
        let mut next_tick = expected_tick;
        let mut behind = 0_u32;
        for (&tick, controls) in self.ready.range(expected_tick..) {
            if tick != next_tick {
                break;
            }
            if !is_ready(controls) {
                break;
            }
            behind = behind.saturating_add(1);
            let Some(next) = next_tick.checked_add(1) else {
                break;
            };
            next_tick = next;
        }
        behind
    }

    pub(crate) fn contiguous_ready_behind(&self, expected_tick: Tick) -> u32 {
        self.contiguous_ready_behind_if(expected_tick, |_| true)
    }

    pub(crate) fn clear(&mut self) {
        self.ready.clear();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AdmissionResourceUnavailable {
    Unloadable,
    TransferFailed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AdmissionResourceState {
    Loading {
        removed: bool,
    },
    Complete {
        path: PathBuf,
        removed: bool,
        local: bool,
    },
    Unavailable(AdmissionResourceUnavailable),
}

pub(crate) const BLOCKING_RESOURCE_STALL_TIMEOUT: Duration = Duration::from_millis(100_000);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BlockingResourceScope {
    ClientStart,
    PlayerJoin,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PendingAdmissionResource {
    pub(crate) core: clonk_engine::NetworkResourceCore,
    pub(crate) info_id: i32,
    pub(crate) player_name: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct BlockingResourceWait {
    pub(crate) scope: BlockingResourceScope,
    pub(crate) resource_id: i32,
    pub(crate) player_info_id: Option<i32>,
    pub(crate) display_name: String,
    last_percent: i16,
    deadline: Instant,
}

impl BlockingResourceWait {
    pub(crate) fn new_at(
        scope: BlockingResourceScope,
        resource_id: i32,
        player_info_id: Option<i32>,
        display_name: String,
        present_percent: u8,
        now: Instant,
    ) -> Self {
        let mut wait = Self {
            scope,
            resource_id,
            player_info_id,
            display_name,
            last_percent: -1,
            deadline: now + BLOCKING_RESOURCE_STALL_TIMEOUT,
        };
        let _ = wait.observe_at(present_percent, now);
        wait
    }

    /// Returns true only after an unchanged percentage has outlived the
    /// native stall deadline. Any numeric change resets the full timeout.
    pub(crate) fn observe_at(&mut self, present_percent: u8, now: Instant) -> bool {
        let present_percent = present_percent.min(100);
        if self.last_percent != i16::from(present_percent) {
            self.last_percent = i16::from(present_percent);
            self.deadline = now + BLOCKING_RESOURCE_STALL_TIMEOUT;
            false
        } else {
            now > self.deadline
        }
    }

    pub(crate) fn present_percent(&self) -> u8 {
        u8::try_from(self.last_percent).unwrap_or_default()
    }
}

#[derive(Debug, Default)]
pub(crate) struct AdmissionResourceStore {
    pub(crate) resources: BTreeMap<i32, AdmissionResourceState>,
    pub(crate) resource_cores: BTreeMap<i32, clonk_engine::NetworkResourceCore>,
    pub(crate) present_percent: BTreeMap<i32, u8>,
}

impl AdmissionResourceStore {
    pub(crate) fn register_lobby_resource(&mut self, core: &clonk_engine::NetworkResourceCore) {
        self.resource_cores.insert(core.id, core.clone());
        self.present_percent.entry(core.id).or_insert(0);
        self.resources
            .entry(core.id)
            .or_insert(AdmissionResourceState::Loading { removed: false });
    }

    pub(crate) fn register_join_data_resources(
        &mut self,
        join_data: &clonk_network::JoinDataEnvelope,
    ) {
        self.register_lobby_resource(&join_data.parameters.scenario);
        for core in &join_data.parameters.game_resources {
            self.register_lobby_resource(core);
        }
        self.register_lobby_resource(&join_data.dynamic);
        for client in &join_data.parameters.player_infos.clients {
            self.register_player_info_resources(&client.players);
        }
    }

    /// Replaces one finished round's catalog with the freshly synchronized
    /// JoinData catalog. Player files are session identities and may be reused
    /// when their complete cores still match; every round-owned resource is
    /// rearmed even when its numeric ID was reused.
    pub(crate) fn reconcile_join_data_resources(
        &mut self,
        join_data: &clonk_network::JoinDataEnvelope,
    ) {
        self.reconcile_round_resources(
            &join_data.parameters.scenario,
            &join_data.parameters.game_resources,
            &join_data.dynamic,
            &join_data.parameters.player_infos,
        );
    }

    pub(crate) fn reconcile_host_join_snapshot(
        &mut self,
        snapshot: &clonk_network::HostJoinSnapshot,
    ) {
        self.reconcile_round_resources(
            &snapshot.parameters.scenario,
            &snapshot.parameters.game_resources,
            &snapshot.dynamic,
            &snapshot.parameters.player_infos,
        );
    }

    fn reconcile_round_resources(
        &mut self,
        scenario: &clonk_engine::NetworkResourceCore,
        game_resources: &[clonk_engine::NetworkResourceCore],
        dynamic: &clonk_engine::NetworkResourceCore,
        player_infos: &clonk_network::PlayerInfoListSnapshot,
    ) {
        let mut next_cores = BTreeMap::new();
        for core in std::iter::once(scenario)
            .chain(game_resources)
            .chain(std::iter::once(dynamic))
        {
            next_cores.insert(core.id, core.clone());
        }
        let mut reusable_player_ids = HashSet::new();
        for player in player_infos
            .clients
            .iter()
            .flat_map(|client| &client.players)
            .filter(|player| {
                player.flags & clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE != 0
                    && player.flags & clonk_engine::PLAYER_INFO_FLAG_REMOVED == 0
                    && player.flags & clonk_engine::PLAYER_INFO_FLAG_IN_SCENARIO_FILE == 0
            })
        {
            let Some(core) = player.resource.as_ref() else {
                continue;
            };
            next_cores.insert(core.id, core.clone());
            if core.resource_type == clonk_network::HostResourceType::Player as u8 {
                reusable_player_ids.insert(core.id);
            }
        }

        let previous_cores = std::mem::take(&mut self.resource_cores);
        let mut previous_resources = std::mem::take(&mut self.resources);
        let mut next_resources = BTreeMap::new();
        let mut next_present_percent = BTreeMap::new();
        for (&resource_id, core) in &next_cores {
            let retained_player = reusable_player_ids.contains(&resource_id)
                && previous_cores.get(&resource_id) == Some(core);
            let retained_complete = retained_player
                .then(|| previous_resources.remove(&resource_id))
                .flatten()
                .and_then(|state| match state {
                    AdmissionResourceState::Complete { path, local, .. } => {
                        Some(AdmissionResourceState::Complete {
                            path,
                            removed: false,
                            local,
                        })
                    }
                    AdmissionResourceState::Loading { .. }
                    | AdmissionResourceState::Unavailable(_) => None,
                });
            if let Some(state) = retained_complete {
                next_resources.insert(resource_id, state);
                next_present_percent.insert(resource_id, 100);
            } else {
                next_resources.insert(
                    resource_id,
                    AdmissionResourceState::Loading { removed: false },
                );
                next_present_percent.insert(resource_id, 0);
            }
        }
        self.resource_cores = next_cores;
        self.resources = next_resources;
        self.present_percent = next_present_percent;
    }

    pub(crate) fn register_player_info_resources(
        &mut self,
        players: &[clonk_engine::ControlPlayerInfoEntry],
    ) {
        for player in players {
            if player.flags & clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE == 0
                || player.flags & clonk_engine::PLAYER_INFO_FLAG_REMOVED != 0
                || player.flags & clonk_engine::PLAYER_INFO_FLAG_IN_SCENARIO_FILE != 0
            {
                continue;
            }
            if let Some(core) = &player.resource {
                self.ensure_by_core(core);
            }
        }
    }

    pub(crate) fn lobby_ready_available(&self) -> bool {
        self.resource_cores.iter().all(|(resource_id, core)| {
            core.resource_type == clonk_network::HostResourceType::Player as u8
                || !matches!(
                    self.resources.get(resource_id),
                    Some(AdmissionResourceState::Loading { removed: false })
                )
        })
    }

    pub(crate) fn ensure_by_core(
        &mut self,
        core: &clonk_engine::NetworkResourceCore,
    ) -> &AdmissionResourceState {
        self.resource_cores
            .entry(core.id)
            .or_insert_with(|| core.clone());
        self.resources.entry(core.id).or_insert_with(|| {
            if core.loadable {
                AdmissionResourceState::Loading { removed: false }
            } else {
                AdmissionResourceState::Unavailable(AdmissionResourceUnavailable::Unloadable)
            }
        })
    }

    pub(crate) fn status(&self, resource_id: i32) -> Option<&AdmissionResourceState> {
        self.resources.get(&resource_id)
    }

    pub(crate) fn complete_path(&self, resource_id: i32) -> Option<&Path> {
        match self.resources.get(&resource_id) {
            Some(AdmissionResourceState::Complete { path, .. }) => Some(path),
            _ => None,
        }
    }

    /// Resolve the newest official resource in a C4Network2Res derivation
    /// chain rooted at a PlayerInfo resource.
    pub(crate) fn derivation_target(&self, root_resource_id: i32) -> Option<i32> {
        if !matches!(
            self.resources.get(&root_resource_id),
            Some(AdmissionResourceState::Complete { .. })
        ) {
            return None;
        }
        let mut current_resource_id = root_resource_id;
        let mut visited = HashSet::new();
        loop {
            if !visited.insert(current_resource_id) {
                return None;
            }
            let Some(next_resource_id) = self
                .resource_cores
                .values()
                .filter(|core| core.derived_id == current_resource_id)
                .map(|core| core.id)
                .max()
            else {
                return Some(current_resource_id);
            };
            current_resource_id = next_resource_id;
        }
    }

    pub(crate) fn register_finished_derivation(
        &mut self,
        core: &clonk_engine::NetworkResourceCore,
        mutable_path: PathBuf,
        ownership: clonk_network::ResourceFileOwnership,
    ) {
        self.register_lobby_resource(core);
        self.mark_complete_with_locality(
            core.id,
            mutable_path,
            ownership == clonk_network::ResourceFileOwnership::Persistent,
        );
    }

    pub(crate) fn mark_complete(&mut self, resource_id: i32, path: PathBuf) {
        self.mark_complete_with_locality(resource_id, path, true);
    }

    pub(crate) fn mark_complete_with_locality(
        &mut self,
        resource_id: i32,
        path: PathBuf,
        local: bool,
    ) {
        self.present_percent.insert(resource_id, 100);
        self.resources.insert(
            resource_id,
            AdmissionResourceState::Complete {
                path,
                removed: false,
                local,
            },
        );
    }

    pub(crate) fn mark_progress(&mut self, resource_id: i32, present_percent: u8) {
        if self.resources.contains_key(&resource_id) {
            self.present_percent
                .insert(resource_id, present_percent.min(100));
        }
    }

    pub(crate) fn mark_failed(&mut self, resource_id: i32) {
        self.present_percent.remove(&resource_id);
        self.resources.insert(
            resource_id,
            AdmissionResourceState::Unavailable(AdmissionResourceUnavailable::TransferFailed),
        );
    }

    pub(crate) fn clear(&mut self) {
        self.resources.clear();
        self.resource_cores.clear();
        self.present_percent.clear();
    }
}

pub(crate) fn preflight_admission_resources(
    resources: &mut AdmissionResourceStore,
    clients: &ControlClientRegistry,
    controls: &[NetworkControl],
    aborted_player_joins: &HashSet<(i32, i32)>,
) -> bool {
    pending_admission_resource(resources, clients, controls, aborted_player_joins).is_none()
}

pub(crate) fn pending_admission_resource(
    resources: &mut AdmissionResourceStore,
    clients: &ControlClientRegistry,
    controls: &[NetworkControl],
    aborted_player_joins: &HashSet<(i32, i32)>,
) -> Option<PendingAdmissionResource> {
    let mut pending = None;
    for control in controls {
        let waiting = match control {
            NetworkControl::JoinPlayer(clonk_engine::JoinPlayerControlData {
                at_client,
                info_id,
                source: clonk_engine::JoinPlayerSource::Resource(core),
                ..
            }) if clients.contains(*at_client) => (matches!(
                resources.ensure_by_core(core),
                AdmissionResourceState::Loading { .. }
            ) && !aborted_player_joins
                .contains(&(core.id, *info_id)))
            .then(|| {
                let player_name = controls
                    .iter()
                    .filter_map(|candidate| match candidate {
                        NetworkControl::PlayerInfo(info) => Some(&info.players),
                        _ => None,
                    })
                    .flatten()
                    .find(|player| player.id == *info_id)
                    .map(|player| legacy_presentation_text(player.name.as_bytes()))
                    .filter(|name| !name.is_empty());
                PendingAdmissionResource {
                    core: core.clone(),
                    info_id: *info_id,
                    player_name,
                }
            }),
            _ => None,
        };
        if pending.is_none() {
            pending = waiting;
        }
    }
    pending
}

#[derive(Clone)]
pub(crate) struct FrontendAssets {
    font: Arc<dyn TextFont>,
    pub(crate) menu_background: Option<ImageData>,
    scenario_browser_background: Option<ImageData>,
    options_background: Option<ImageData>,
    about_background: Option<ImageData>,
    /// CStdFont-faithful GUI fonts for pixel-parity startup text.
    pub(crate) clonk_fonts: Option<Arc<clonk_frontend::ClonkFontSet>>,
    pub(crate) startup_clonk_fonts: Option<Arc<clonk_frontend::ClonkFontSet>>,
    pub(crate) startup_native_font_source: Option<ClassicNativeFontSource>,
    /// Independent process-global shadowless Main-14 `FontTooltip`.
    pub(crate) global_tooltip_font: Option<Arc<clonk_graphics::clonk_font::ClonkFont>>,
    pub(crate) startup_global_tooltip_font: Option<Arc<clonk_graphics::clonk_font::ClonkFont>>,
    pub(crate) logo: Option<ImageData>,
    pub(crate) button_textures: Option<ButtonTextures>,
    /// GUIButtonHighlight.png — additive focus/hover overlay for GUI buttons
    /// (C4GraphicsResource.cpp:1089-1093, C4GuiButton.cpp:94-98).
    pub(crate) button_highlight: Option<ImageData>,
    /// Consumer-side transparent-RGB normalization retained for bilinear
    /// classic in-game dialogs and raw/test-injected `ImageData`.
    pub(crate) game_over_button_highlight: Option<ImageData>,
    /// Graphics.c4g images used by the startup dialog parity renderers,
    /// keyed by file name (the eager bootstrap plus supplemental GUI images).
    /// While a scenario runs, `C4GUI::Resource::Load` sheet entries may be
    /// rebound to active-scenario overrides; the pristine startup entries are
    /// retained in `startup_gui_sheet_images` until teardown restores them.
    pub(crate) startup_dialog_images: HashMap<String, ImageData>,
    /// Winning override source per rebound GUI sheet stem — the C++ group-id
    /// reload cache (idSfcCaption/idSourceGroup, C4Gui.cpp:1085-1130): a
    /// repeated refresh reloads a sheet only when its winning source changes.
    pub(crate) active_gui_sheet_sources: HashMap<&'static str, String>,
    /// Pristine startup images for every rebound canonical sheet name.
    /// Teardown (C++ `Resource::Clear` + `CloseFiles`, restoring the
    /// global-only group set) swaps these back in.
    pub(crate) startup_gui_sheet_images: HashMap<String, ImageData>,
    /// Non-lookup failures for eager startup images. A missing map entry with
    /// no recorded failure is a true absence; decode/read failures remain a
    /// typed malformed issue instead of being collapsed into "missing".
    pub(crate) startup_bootstrap_image_failures: HashMap<String, String>,
    /// Exact initial base/Extra C4GUI source failures, retained separately
    /// from the canonical renderer map so a lower-priority PNG cannot mask a
    /// malformed winning bmp/jpeg/jpg/group source.
    global_gui_font_failures: HashMap<&'static str, String>,
    global_gui_sheet_failures: HashMap<&'static str, String>,
    /// Shadowless startup "book" fonts (C4StartupGraphics::InitFonts).
    pub(crate) book_fonts: Option<Arc<clonk_frontend::startup_scensel::BookFontSet>>,
    /// Book + book-small shadowless fonts for the options paper sheet.
    pub(crate) options_book_fonts: Option<Arc<clonk_frontend::startup_options_dlg::BookFonts>>,
    /// The player-selection dialog's own shadowless book fonts.
    pub(crate) plrsel_book_fonts: Option<Arc<clonk_frontend::startup_plrsel::BookFontSet>>,
    base_sprites: HashMap<String, DefinitionSprite>,
    pub(crate) cursor_atlas: Arc<CursorAtlas>,
    pub(crate) hud_graphics: Arc<HudGraphics>,
    /// True for repository/install-backed assets, whose C4GraphicsResource
    /// initialization must reject missing mandatory HUD images. Asset-less
    /// state-only fixtures intentionally exercise non-presentation behavior.
    pub(crate) classic_hud_resources_required: bool,
    game_palette: Arc<GamePalette>,
    /// Selected Liquid graphic, retained only when both legacy graphics
    /// switches required for the landscape shader are enabled. The resource
    /// is validated during every install-backed graphics initialization even
    /// when this remains `None`.
    liquid_animation: Option<ImageData>,
    /// Mandatory selected-Liquid failure swallowed by the presentation asset
    /// collector and surfaced at the shared classic bootstrap boundary.
    pub(crate) liquid_animation_issue: Option<ClassicGuiBootstrapIssue>,
    liquid_animation_enabled: bool,
}

impl FrontendAssets {
    pub(crate) fn load(paths: Option<&AppPaths>) -> Self {
        let classic_hud_resources_required = paths.is_some();
        let liquid_animation_enabled = load_graphics_color_animation(paths);
        let font = Self::load_font(paths);
        let classic_fonts = Self::load_classic_fonts(paths);
        let clonk_fonts = classic_fonts.as_ref().map(|bundle| bundle.fonts.clone());
        let startup_clonk_fonts = clonk_fonts.clone();
        let startup_native_font_source = classic_fonts
            .as_ref()
            .and_then(|bundle| bundle.native_source.clone());
        let global_tooltip_font = classic_fonts.as_ref().map(|bundle| bundle.tooltip.clone());
        let startup_global_tooltip_font = global_tooltip_font.clone();
        let (book_fonts, options_book_fonts, plrsel_book_fonts) =
            match Self::load_classic_startup_fonts(paths) {
                Some(bundle) => (
                    Some(bundle.book),
                    Some(bundle.options),
                    Some(bundle.player_selection),
                ),
                None => (None, None, None),
            };
        let mut startup_dialog_images = HashMap::new();
        let mut startup_bootstrap_image_failures = HashMap::new();
        let mut global_gui_font_failures = HashMap::new();
        let mut global_gui_sheet_failures = HashMap::new();
        let mut menu_background = None;
        let mut scenario_browser_background = None;
        let mut options_background = None;
        let mut about_background = None;
        let mut logo = None;
        let mut button_textures = None;
        let mut sprites = HashMap::new();
        let mut cursor_atlas = CursorAtlas::empty();
        let mut hud_graphics = HudGraphics::default();
        let mut game_palette = GamePalette::default();
        let mut liquid_animation = None;
        let mut liquid_animation_issue = None;

        if let Some(paths) = paths {
            let graphics_path = paths.planet_dir().join("Graphics.c4g");
            match GraphicsResource::open(&graphics_path) {
                Ok(graphics) => {
                    Self::prewarm_startup_images(&graphics);
                    menu_background = graphics
                        .load_image("LoaderGoldmine1.png")
                        .ok()
                        .map(Self::image_to_data)
                        .or_else(|| startup_menu_background_wildcard(&graphics));
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
                    if let Ok(sprite) = graphics.load_image("Crew.png") {
                        let image = Self::image_to_data(sprite);
                        sprites.insert(
                            "Walker".to_string(),
                            DefinitionSprite {
                                graphics_scale: 1.0,
                                shape: None,
                                fire_top: 0,
                                rotateable: 0,
                                line: 0,
                                stretch_growth: false,
                                top_face: None,
                                picture: None,
                                image,
                                actions: HashMap::new(),
                                color_mask: None,
                            },
                        );
                    }
                    for name in CLASSIC_STARTUP_BOOTSTRAP_IMAGES
                        .into_iter()
                        .chain(SUPPLEMENTAL_STARTUP_DIALOG_IMAGES.iter().copied())
                    {
                        match graphics.load_image(name) {
                            Ok(image) => {
                                startup_dialog_images
                                    .insert((*name).to_string(), Self::image_to_data(image));
                            }
                            Err(err) => {
                                if CLASSIC_STARTUP_BOOTSTRAP_IMAGES.contains(&name)
                                    && !matches!(&err, GraphicsError::EntryNotFound { .. })
                                {
                                    startup_bootstrap_image_failures
                                        .insert(name.to_string(), err.to_string());
                                }
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

            let source_setup = (|| -> Result<_> {
                let graphics = main_graphics_group(paths)?;
                let registrations = startup_loader_registrations(paths)?;
                let graphics_registrations = loader_graphics_registrations(&registrations)?;
                Ok((graphics, registrations, graphics_registrations))
            })();
            match source_setup {
                Ok((graphics, registrations, graphics_registrations)) => {
                    if let Err(error) = validate_classic_loader_font(paths, None, &registrations)
                        .and_then(|()| validate_loader_graphics_font_sources(&registrations))
                    {
                        let detail = error.to_string();
                        for name in CLASSIC_GLOBAL_GUI_FONTS {
                            global_gui_font_failures.insert(name, detail.clone());
                        }
                    }
                    for (stem, canonical_name) in CLASSIC_GLOBAL_GUI_SHEETS {
                        match resolve_named_graphics_image(stem, &graphics_registrations, &graphics)
                        {
                            Ok(resolved) => {
                                startup_dialog_images
                                    .insert(canonical_name.to_string(), resolved.image);
                            }
                            Err(error) => {
                                let detail = error.to_string();
                                if detail
                                    != format!("classic graphics resource `{stem}` is unavailable")
                                {
                                    global_gui_sheet_failures.insert(stem, detail);
                                }
                                startup_dialog_images.remove(canonical_name);
                            }
                        }
                    }
                    match resolve_game_graphics_resources(
                        &graphics_registrations,
                        &graphics,
                        None,
                        liquid_animation_enabled,
                    ) {
                        Ok(resources) => {
                            liquid_animation_issue = None;
                            cursor_atlas = resources.cursor_atlas.as_ref().clone();
                            hud_graphics = resources.hud_graphics.as_ref().clone();
                            game_palette = resources.palette.as_ref().clone();
                            liquid_animation = resources.liquid_animation.as_deref().cloned();
                            if let Some(options) = resources.options {
                                startup_dialog_images
                                    .insert("Options.png".to_string(), options.as_ref().clone());
                            }
                        }
                        Err(error) => {
                            liquid_animation_issue = Self::liquid_animation_issue(&error);
                            tracing::warn!(
                                %error,
                                "failed to resolve startup game graphics bundle"
                            );
                        }
                    }
                }
                Err(error) => {
                    let detail = error.to_string();
                    for name in CLASSIC_GLOBAL_GUI_FONTS {
                        global_gui_font_failures.insert(name, detail.clone());
                    }
                    for (stem, _) in CLASSIC_GLOBAL_GUI_SHEETS {
                        global_gui_sheet_failures.insert(stem, detail.clone());
                    }
                }
            }
        }

        // The fixed-PNG prepass only primes caches. Any accepted base/Extra
        // stem selection above is authoritative for every derived consumer.
        let button_highlight = startup_dialog_images.get("GUIButtonHighlight.png").cloned();
        let game_over_button_highlight = startup_dialog_images
            .get("GUIButtonHighlight.png")
            .map(clonk_frontend::classic_gui::blacken_transparent_pixels);

        Self {
            font,
            clonk_fonts,
            startup_clonk_fonts,
            startup_native_font_source,
            global_tooltip_font,
            startup_global_tooltip_font,
            menu_background,
            scenario_browser_background,
            options_background,
            about_background,
            logo,
            button_textures,
            button_highlight,
            game_over_button_highlight,
            startup_dialog_images,
            active_gui_sheet_sources: HashMap::new(),
            startup_gui_sheet_images: HashMap::new(),
            startup_bootstrap_image_failures,
            global_gui_font_failures,
            global_gui_sheet_failures,
            book_fonts,
            options_book_fonts,
            plrsel_book_fonts,
            base_sprites: sprites,
            cursor_atlas: Arc::new(cursor_atlas),
            hud_graphics: Arc::new(hud_graphics),
            classic_hud_resources_required,
            game_palette: Arc::new(game_palette),
            liquid_animation,
            liquid_animation_issue,
            liquid_animation_enabled,
        }
    }

    /// Decodes the startup images into the resource's cache from a worker
    /// pool, so the sequential loads in `load` become cache hits. PNG
    /// decoding dominates boot time and every image is independent.
    fn prewarm_startup_images(graphics: &GraphicsResource) {
        let names: Vec<&str> = [
            "LoaderGoldmine1.png",
            "Logo.png",
            // Cursor atlas (`load_cursor_atlas`).
            "CursorXXXXXLarge.png",
            "CursorXXXXLarge.png",
            "CursorXXXLarge.png",
            "CursorXXLarge.png",
            "CursorXLarge.png",
            "CursorLarge.png",
            "CursorMedium.png",
            "CursorSmall.png",
            // HUD graphics (`load_hud_graphics`).
            "Player.png",
            "Flag.png",
            "Crew.png",
            "Score.png",
            "Wealth.png",
            "Rank.png",
            "Captain.png",
            "Fire.png",
            "Menu.png",
            "UpperBoard.png",
            "Construction.png",
            "Energy.png",
            "Magic.png",
            "Arrow.png",
            "Exit.png",
            "Hand.png",
            "Build.png",
            "EnergyBars.png",
            "SelectMark.png",
            "Control.png",
            "Gamepad.png",
            "Background.png",
        ]
        .into_iter()
        .chain(CLASSIC_STARTUP_BOOTSTRAP_IMAGES)
        .chain(SUPPLEMENTAL_STARTUP_DIALOG_IMAGES.iter().copied())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
        // nextest already parallelizes individual cases as processes. Letting
        // every test process fan out to every host core turns `-j 8` into more
        // than a hundred simultaneous PNG/font workers and makes the suite
        // slower and timing-sensitive. Production startup keeps the full-core
        // prewarm; tests retain parallel decode with a bounded inner pool.
        let workers = if cfg!(test) {
            2
        } else {
            std::thread::available_parallelism()
                .map(std::num::NonZeroUsize::get)
                .unwrap_or(4)
        }
        .min(names.len())
        .max(1);
        let next = std::sync::atomic::AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..workers {
                scope.spawn(|| {
                    loop {
                        let index = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        // Failures surface (with logging) on the cache-miss
                        // retry in `load`.
                        match names.get(index) {
                            Some(name) => {
                                let _ = graphics.load_image(name);
                            }
                            None => break,
                        }
                    }
                });
            }
        });
    }

    /// Resolves the configured startup RX face, Fonts.txt mappings and
    /// vector/bitmap resources into the live C4GUI font bundle.
    fn load_classic_fonts(paths: Option<&AppPaths>) -> Option<ClassicFontBundle> {
        let paths = paths?;
        let registrations = match startup_loader_registrations(paths) {
            Ok(registrations) => registrations,
            Err(error) => {
                tracing::warn!(%error, "failed to inspect startup font groups");
                return None;
            }
        };
        match resolve_classic_font_bundle(paths, None, &registrations, &registrations) {
            Ok(bundle) => Some(bundle),
            Err(error) => {
                tracing::warn!(%error, "failed to resolve configured classic fonts");
                None
            }
        }
    }

    /// Resolves the configured startup RX face for every shadowless book
    /// font set (C4Startup.cpp:92-116).
    fn load_classic_startup_fonts(paths: Option<&AppPaths>) -> Option<ClassicStartupFontBundle> {
        let paths = paths?;
        let registrations = match startup_loader_registrations(paths) {
            Ok(registrations) => registrations,
            Err(error) => {
                tracing::warn!(%error, "failed to inspect startup book-font groups");
                return None;
            }
        };
        let (request, base_size) = match classic_font_request(paths, None) {
            Ok(request) => request,
            Err(error) => {
                tracing::warn!(%error, "failed to read configured startup book font");
                return None;
            }
        };
        match resolve_classic_startup_font_bundle_for_request(
            paths,
            &request,
            base_size,
            &registrations,
            &registrations,
        ) {
            Ok(bundle) => Some(bundle),
            Err(error) => {
                tracing::warn!(%error, "failed to resolve configured startup book fonts");
                None
            }
        }
    }

    pub(crate) fn options_dlg_assets(
        &self,
    ) -> Option<clonk_frontend::startup_options_dlg::OptionsDlgAssets> {
        Some(clonk_frontend::startup_options_dlg::OptionsDlgAssets {
            background: self.menu_background()?,
            paper: self.dialog_image("StartupDlgPaper.png")?,
            tab_clip: self.dialog_image("StartupTabClip.png")?,
            option_icons: self.dialog_image("StartupOptionIcons.png")?,
            book_scroll: self.dialog_image("StartupBookScroll.png")?,
            context_arrow: self.dialog_image("StartupContext.png")?,
            checkbox: self.dialog_image("GUICheckbox.png")?,
            // The control facets degrade to text buttons when absent, so these
            // are optional rather than failing the whole dialog.
            control: self.dialog_image("Control.png"),
            gamepad: self.dialog_image("Gamepad.png"),
            button_highlight: self.dialog_image("GUIButtonHighlight.png")?,
            button: self.dialog_image("GUIButton.png")?,
        })
    }

    pub(crate) fn options_advanced_assets(
        &self,
    ) -> Option<clonk_frontend::startup_options_advanced::AdvancedConfigAssets> {
        Some(
            clonk_frontend::startup_options_advanced::AdvancedConfigAssets {
                caption: self.dialog_image("GUICaption.png")?,
                button: self.dialog_image("GUIButton.png")?,
                button_down: self.dialog_image("GUIButtonDown.png")?,
                button_highlight: self.dialog_image("GUIButtonHighlight.png")?,
                checkbox: self.dialog_image("GUICheckbox.png")?,
            },
        )
    }

    pub(crate) fn plrsel_assets(&self) -> Option<clonk_frontend::startup_plrsel::PlrSelAssets> {
        Some(clonk_frontend::startup_plrsel::PlrSelAssets {
            background: self.dialog_image("StartupPlrSelBG.png")?,
            checkbox: self.dialog_image("GUICheckbox.png")?,
            button: self.dialog_image("GUIButton.png")?,
            button_down: self.dialog_image("GUIButtonDown.png")?,
            button_highlight: self.dialog_image("GUIButtonHighlight.png")?,
            book_scroll: self.dialog_image("StartupBookScroll.png")?,
            player: self.dialog_image("Player.png")?,
        })
    }

    /// The nested portrait selector blits thumbnails, so it needs the same
    /// filtering inputs every other textured blit takes (`StdGL.cpp:527`).
    pub(crate) fn plrprop_assets(
        &self,
        point_filtering: bool,
        application_scale: f32,
    ) -> Option<clonk_frontend::startup_plrproperties::PlayerPropertiesAssets> {
        Some(
            clonk_frontend::startup_plrproperties::PlayerPropertiesAssets {
                point_filtering,
                application_scale,
                background: self.dialog_image("StartupPlrPropBG.png")?,
                big_arrows: self.dialog_image("GUIBigArrows.png")?,
                book_scroll: self.dialog_image("StartupBookScroll.png")?,
                icons: self.dialog_image("GUIIcons.png")?,
                button_highlight: self.dialog_image("GUIButtonHighlight.png")?,
                flag: self.dialog_image("Flag.png")?,
                control: self.dialog_image("Control.png")?,
                gamepad: self.dialog_image("Gamepad.png"),
                control_types: self.dialog_image("StartupPlrCtrlType.png"),
                caption: self.dialog_image("GUICaption.png")?,
                button: self.dialog_image("GUIButton.png")?,
                button_down: self.dialog_image("GUIButtonDown.png")?,
                context: self.dialog_image("GUIContext.png")?,
                checkbox: self.dialog_image("GUICheckbox.png")?,
                scroll: self.dialog_image("GUIScroll.png")?,
            },
        )
    }

    pub(crate) fn dialog_image(&self, name: &str) -> Option<ImageData> {
        self.startup_dialog_images.get(name).cloned()
    }

    /// `C4GUI::Resource::Load` over the active group set
    /// (C4Gui.cpp:1085-1112): rebinds every sheet whose winning source
    /// changed, keeps unchanged winners loaded (the group-id cache of
    /// C4GraphicsResource.cpp:418-470), and restores the pristine startup
    /// sheet when the global group wins again. Returns whether any sheet
    /// was rebound.
    pub(crate) fn apply_active_gui_sheet_overrides(
        &mut self,
        overrides: &[ClassicGuiSheetOverride],
    ) -> bool {
        let by_stem: HashMap<&'static str, &ClassicGuiSheetOverride> =
            overrides.iter().map(|sheet| (sheet.stem, sheet)).collect();
        let mut changed = false;
        for (stem, canonical_name) in CLASSIC_GLOBAL_GUI_SHEETS {
            match by_stem.get(stem) {
                Some(sheet) => {
                    if self.active_gui_sheet_sources.get(stem) == Some(&sheet.source) {
                        continue;
                    }
                    if !self.startup_gui_sheet_images.contains_key(canonical_name) {
                        if let Some(pristine) = self.startup_dialog_images.get(canonical_name) {
                            self.startup_gui_sheet_images
                                .insert(canonical_name.to_string(), pristine.clone());
                        }
                    }
                    self.startup_dialog_images
                        .insert(canonical_name.to_string(), sheet.image.clone());
                    self.active_gui_sheet_sources
                        .insert(stem, sheet.source.clone());
                    changed = true;
                }
                None => {
                    if self.active_gui_sheet_sources.remove(stem).is_some() {
                        match self.startup_gui_sheet_images.remove(canonical_name) {
                            Some(pristine) => {
                                self.startup_dialog_images
                                    .insert(canonical_name.to_string(), pristine);
                            }
                            None => {
                                self.startup_dialog_images.remove(canonical_name);
                            }
                        }
                        changed = true;
                    }
                }
            }
        }
        if changed {
            self.refresh_derived_gui_sheet_state();
        }
        changed
    }

    /// Derived consumers of the rebindable sheets; C++ recomputes the bars
    /// and facet cuts inside every `Resource::Load`.
    fn refresh_derived_gui_sheet_state(&mut self) {
        self.button_highlight = self
            .startup_dialog_images
            .get("GUIButtonHighlight.png")
            .cloned();
        self.game_over_button_highlight = self
            .startup_dialog_images
            .get("GUIButtonHighlight.png")
            .map(clonk_frontend::classic_gui::blacken_transparent_pixels);
    }

    pub(crate) fn loader_resources(&self) -> Result<LoaderResources> {
        let fonts = self
            .clonk_fonts
            .clone()
            .context("CStdFont-faithful loader fonts are unavailable")?;
        let progress = self
            .dialog_image("GUIProgress.png")
            .context("GUIProgress.png is unavailable")?;
        LoaderResources::new(fonts, progress)
    }

    pub(crate) fn context_menu_resources(
        &self,
    ) -> Result<clonk_frontend::context_menu::ContextMenuResources> {
        let fonts = self
            .clonk_fonts
            .as_deref()
            .context("CStdFont-faithful GUI fonts are unavailable")?;
        let tooltip_font = self
            .global_tooltip_font
            .as_deref()
            .context("process-global shadowless tooltip font is unavailable")?;
        let icons = self
            .startup_dialog_images
            .get("GUIIcons.png")
            .context("GUIIcons.png is unavailable")?;
        let submenu_arrow = self
            .startup_dialog_images
            .get("GUISubmenu.png")
            .context("GUISubmenu.png is unavailable")?;
        clonk_frontend::context_menu::ContextMenuResources::new(
            &fonts.text,
            tooltip_font,
            icons,
            submenu_arrow,
        )
    }

    pub(crate) fn game_over_classic_resources<'a>(
        &'a self,
        hud: &'a HudGraphics,
    ) -> Option<GameOverClassicResources<'a>> {
        let caption = self.startup_dialog_images.get("GUICaption.png")?;
        let button = self.startup_dialog_images.get("GUIButton.png")?;
        let button_down = self.startup_dialog_images.get("GUIButtonDown.png")?;
        let button_highlight = self.game_over_button_highlight.as_ref()?;
        let fonts = self.clonk_fonts.as_deref()?;
        Some(
            GameOverClassicResources::new(
                clonk_frontend::classic_gui::ClassicGuiSkin::new(
                    caption,
                    button,
                    button_down,
                    Some(button_highlight),
                ),
                fonts,
                Some(button_highlight),
                self.startup_dialog_images.get("GUIIcons.png"),
                hud.player
                    .as_ref()
                    .or_else(|| self.startup_dialog_images.get("Player.png")),
                hud.score.as_ref(),
                self.startup_dialog_images.get("GUIScroll.png"),
                hud.crew.as_ref(),
            )
            .with_gui_icons_extended(self.startup_dialog_images.get("GUIIcons2.png")),
        )
    }

    pub(crate) fn scoreboard_resources<'a>(
        &'a self,
        font_images: &'a HashMap<String, ImageData>,
    ) -> Result<clonk_frontend::scoreboard::ScoreboardResources<'a>> {
        let caption = self
            .startup_dialog_images
            .get("GUICaption.png")
            .context("GUICaption.png is unavailable")?;
        let icons = self
            .startup_dialog_images
            .get("GUIIcons.png")
            .context("GUIIcons.png is unavailable")?;
        let fonts = self
            .clonk_fonts
            .as_deref()
            .context("CStdFont-faithful GUI fonts are unavailable")?;
        let button_highlight = self
            .game_over_button_highlight
            .as_ref()
            .context("GUIButtonHighlight.png is unavailable")?;
        Ok(
            clonk_frontend::scoreboard::ScoreboardResources::new(caption, icons, fonts)?
                .with_button_highlight(button_highlight)
                .with_font_images(font_images),
        )
    }

    pub(crate) fn message_dialog_resources(
        &self,
    ) -> Option<clonk_frontend::message_dialog::MessageDialogResources<'_>> {
        let caption = self.startup_dialog_images.get("GUICaption.png")?;
        let button = self.startup_dialog_images.get("GUIButton.png")?;
        let button_down = self.startup_dialog_images.get("GUIButtonDown.png")?;
        let button_highlight = self.game_over_button_highlight.as_ref()?;
        Some(clonk_frontend::message_dialog::MessageDialogResources {
            skin: clonk_frontend::classic_gui::ClassicGuiSkin::new(
                caption,
                button,
                button_down,
                Some(button_highlight),
            ),
            fonts: self.clonk_fonts.as_deref()?,
            tooltip_font: self.global_tooltip_font.as_deref()?,
            icons: self.startup_dialog_images.get("GUIIcons.png")?,
            icons_extended: self.startup_dialog_images.get("GUIIcons2.png")?,
            button_highlight,
            checkbox: self.startup_dialog_images.get("GUICheckbox.png")?,
            progress: self.startup_dialog_images.get("GUIProgress.png")?,
        })
    }

    pub(crate) fn league_signup_resources(
        &self,
    ) -> Result<clonk_frontend::league_signup::LeagueSignupResources<'_>> {
        let image = |name| {
            self.startup_dialog_images
                .get(name)
                .with_context(|| format!("{name} is unavailable"))
        };
        let highlight = self
            .game_over_button_highlight
            .as_ref()
            .context("clean classic button highlight is unavailable")?;
        Ok(clonk_frontend::league_signup::LeagueSignupResources {
            skin: clonk_frontend::classic_gui::ClassicGuiSkin::new(
                image("GUICaption.png")?,
                image("GUIButton.png")?,
                image("GUIButtonDown.png")?,
                Some(highlight),
            ),
            fonts: self
                .clonk_fonts
                .as_deref()
                .context("CStdFont-faithful GUI fonts are unavailable")?,
            icons: image("GUIIcons.png")?,
            icons_extended: image("GUIIcons2.png")?,
            checkbox: image("GUICheckbox.png")?,
            button_highlight: highlight,
        })
    }

    pub(crate) fn network_start_wait_resources(
        &self,
    ) -> Result<clonk_frontend::network_start_wait::NetworkStartWaitResources<'_>> {
        let caption = self
            .startup_dialog_images
            .get("GUICaption.png")
            .context("GUICaption.png is unavailable")?;
        let button = self
            .startup_dialog_images
            .get("GUIButton.png")
            .context("GUIButton.png is unavailable")?;
        let button_down = self
            .startup_dialog_images
            .get("GUIButtonDown.png")
            .context("GUIButtonDown.png is unavailable")?;
        let button_highlight = self
            .game_over_button_highlight
            .as_ref()
            .context("GUIButtonHighlight.png is unavailable")?;
        let fonts = self
            .clonk_fonts
            .as_deref()
            .context("CStdFont-faithful GUI fonts are unavailable")?;
        let icons = self
            .startup_dialog_images
            .get("GUIIcons.png")
            .context("GUIIcons.png is unavailable")?;
        clonk_frontend::network_start_wait::NetworkStartWaitResources::new(
            clonk_frontend::classic_gui::ClassicGuiSkin::new(
                caption,
                button,
                button_down,
                Some(button_highlight),
            ),
            fonts,
            icons,
            button_highlight,
        )
    }

    pub(crate) fn runtime_client_list_resources(
        &self,
    ) -> Option<clonk_frontend::runtime_client_list::RuntimeClientListResources<'_>> {
        let caption = self.startup_dialog_images.get("GUICaption.png")?;
        let button = self.startup_dialog_images.get("GUIButton.png")?;
        let button_down = self.startup_dialog_images.get("GUIButtonDown.png")?;
        let button_highlight = self.game_over_button_highlight.as_ref()?;
        Some(
            clonk_frontend::runtime_client_list::RuntimeClientListResources {
                skin: clonk_frontend::classic_gui::ClassicGuiSkin::new(
                    caption,
                    button,
                    button_down,
                    Some(button_highlight),
                ),
                fonts: self.clonk_fonts.as_deref()?,
                tooltip_font: self.global_tooltip_font.as_deref()?,
                icons: self.startup_dialog_images.get("GUIIcons.png")?,
                button_highlight,
                context: self.startup_dialog_images.get("GUIContext.png")?,
                scroll: self.startup_dialog_images.get("GUIScroll.png")?,
            },
        )
    }

    pub(crate) fn static_info_dialog_resources(
        &self,
    ) -> Option<clonk_frontend::runtime_client_list::StaticInfoDialogResources<'_>> {
        let caption = self.startup_dialog_images.get("GUICaption.png")?;
        let button = self.startup_dialog_images.get("GUIButton.png")?;
        let button_down = self.startup_dialog_images.get("GUIButtonDown.png")?;
        let button_highlight = self.game_over_button_highlight.as_ref()?;
        Some(
            clonk_frontend::runtime_client_list::StaticInfoDialogResources {
                skin: clonk_frontend::classic_gui::ClassicGuiSkin::new(
                    caption,
                    button,
                    button_down,
                    Some(button_highlight),
                ),
                fonts: self.clonk_fonts.as_deref()?,
                icons: self.startup_dialog_images.get("GUIIcons.png")?,
                button_highlight,
                scroll: self.startup_dialog_images.get("GUIScroll.png")?,
            },
        )
    }

    pub(crate) fn network_chart_resources(
        &self,
    ) -> Option<clonk_frontend::network_chart::NetworkChartResources<'_>> {
        let caption = self.startup_dialog_images.get("GUICaption.png")?;
        let button = self.startup_dialog_images.get("GUIButton.png")?;
        let button_down = self.startup_dialog_images.get("GUIButtonDown.png")?;
        let button_highlight = self.game_over_button_highlight.as_ref()?;
        Some(clonk_frontend::network_chart::NetworkChartResources {
            skin: clonk_frontend::classic_gui::ClassicGuiSkin::new(
                caption,
                button,
                button_down,
                Some(button_highlight),
            ),
            fonts: self.clonk_fonts.as_deref()?,
            icons: self.startup_dialog_images.get("GUIIcons.png")?,
        })
    }

    pub(crate) fn definition_sel_resources(
        &self,
    ) -> Option<clonk_frontend::definition_sel::DefinitionSelResources<'_>> {
        let caption = self.startup_dialog_images.get("GUICaption.png")?;
        let button = self.startup_dialog_images.get("GUIButton.png")?;
        let button_down = self.startup_dialog_images.get("GUIButtonDown.png")?;
        let button_highlight = self.startup_dialog_images.get("GUIButtonHighlight.png")?;
        Some(clonk_frontend::definition_sel::DefinitionSelResources {
            skin: clonk_frontend::classic_gui::ClassicGuiSkin::new(
                caption,
                button,
                button_down,
                Some(button_highlight),
            ),
            fonts: self.clonk_fonts.as_deref()?,
            icons: self.startup_dialog_images.get("GUIIcons.png")?,
            checkbox: self.startup_dialog_images.get("GUICheckbox.png")?,
            scroll: self.startup_dialog_images.get("GUIScroll.png")?,
            button_highlight,
        })
    }

    pub(crate) fn game_option_resources(
        &self,
    ) -> Result<clonk_frontend::game_option_buttons::GameOptionButtonResources<'_>> {
        let icons = self
            .startup_dialog_images
            .get("GUIIcons2.png")
            .context("GUIIcons2.png is unavailable")?;
        let highlight = self
            .startup_dialog_images
            .get("GUIButtonHighlight.png")
            .context("GUIButtonHighlight.png is unavailable")?;
        let tooltip_font = self
            .global_tooltip_font
            .as_deref()
            .context("process-global shadowless tooltip font is unavailable")?;
        clonk_frontend::game_option_buttons::GameOptionButtonResources::new(
            icons,
            highlight,
            tooltip_font,
        )
    }

    pub(crate) fn game_lobby_resources(&self) -> Result<LobbyResources<'_>> {
        let image = |name| {
            self.startup_dialog_images
                .get(name)
                .with_context(|| format!("{name} is unavailable"))
        };
        LobbyResources::new(
            self.clonk_fonts
                .as_deref()
                .context("CStdFont-faithful lobby fonts are unavailable")?,
            self.global_tooltip_font
                .as_deref()
                .context("CStdFont-faithful lobby tooltip font is unavailable")?,
            image("GUICaption.png")?,
            image("GUIButton.png")?,
            image("GUIButtonDown.png")?,
            image("GUIIcons.png")?,
            image("GUIIcons2.png")?,
            image("GUIButtonHighlight.png")?,
            image("GUICheckbox.png")?,
            image("GUIScroll.png")?,
            image("GUIContext.png")?,
        )
    }

    pub(crate) fn input_dialog_resources(
        &self,
    ) -> Result<clonk_frontend::input_dialog::InputDialogResources<'_>> {
        let caption = self
            .startup_dialog_images
            .get("GUICaption.png")
            .context("GUICaption.png is unavailable")?;
        let button = self
            .startup_dialog_images
            .get("GUIButton.png")
            .context("GUIButton.png is unavailable")?;
        let button_down = self
            .startup_dialog_images
            .get("GUIButtonDown.png")
            .context("GUIButtonDown.png is unavailable")?;
        let highlight = self
            .game_over_button_highlight
            .as_ref()
            .context("clean classic button highlight is unavailable")?;
        let fonts = self
            .clonk_fonts
            .as_deref()
            .context("CStdFont-faithful GUI fonts are unavailable")?;
        let tooltip_font = self
            .global_tooltip_font
            .as_deref()
            .context("process-global shadowless tooltip font is unavailable")?;
        let icons = self
            .startup_dialog_images
            .get("GUIIcons.png")
            .context("GUIIcons.png is unavailable")?;
        let icons_extended = self
            .startup_dialog_images
            .get("GUIIcons2.png")
            .context("GUIIcons2.png is unavailable")?;
        clonk_frontend::input_dialog::InputDialogResources::new(
            clonk_frontend::classic_gui::ClassicGuiSkin::new(
                caption,
                button,
                button_down,
                Some(highlight),
            ),
            fonts,
            tooltip_font,
            icons,
            icons_extended,
            highlight,
        )
    }

    pub(crate) fn about_dlg_assets(
        &self,
    ) -> Option<clonk_frontend::startup_about_dlg::AboutDlgAssets> {
        Some(clonk_frontend::startup_about_dlg::AboutDlgAssets {
            background: self.dialog_image("LoaderWatercave1.png")?,
            caption: self.dialog_image("GUICaption.png")?,
            button: self.dialog_image("GUIButton.png")?,
            button_down: self.dialog_image("GUIButtonDown.png")?,
            button_highlight: self.dialog_image("GUIButtonHighlight.png")?,
            scroll: self.dialog_image("GUIScroll.png")?,
        })
    }

    pub(crate) fn scensel_assets(&self) -> Option<clonk_frontend::startup_scensel::ScenSelAssets> {
        Some(clonk_frontend::startup_scensel::ScenSelAssets {
            background: self.dialog_image("StartupScenSelBG.png")?,
            book_scroll: self.dialog_image("StartupBookScroll.png")?,
            scen_icons: self.dialog_image("StartupScenSelIcons.png")?,
            caption_bar: self.dialog_image("GUICaption.png")?,
            button: self.dialog_image("GUIButton.png")?,
            checkbox: self.dialog_image("GUICheckbox.png")?,
            button_highlight: self.dialog_image("GUIButtonHighlight.png")?,
            icons_ex: self.dialog_image("GUIIcons2.png")?,
            title_overlay: self.dialog_image("StartupScenSelTitleOv.png")?,
        })
    }

    pub(crate) fn netdlg_assets(&self) -> Option<clonk_frontend::startup_netdlg::NetDlgAssets> {
        Some(clonk_frontend::startup_netdlg::NetDlgAssets {
            background: self.dialog_image("StartupNetworkBG.png")?,
            net_get_ref: self.dialog_image("StartupNetGetRef.png")?,
            scen_icons: self.dialog_image("StartupScenSelIcons.png")?,
            gui_caption: self.dialog_image("GUICaption.png")?,
            gui_button: self.dialog_image("GUIButton.png")?,
            gui_button_down: self.dialog_image("GUIButtonDown.png")?,
            gui_button_highlight: self.dialog_image("GUIButtonHighlight.png")?,
            gui_scroll: self.dialog_image("GUIScroll.png")?,
            gui_icons: self.dialog_image("GUIIcons.png")?,
            gui_icons_ex: self.dialog_image("GUIIcons2.png")?,
        })
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

    pub(crate) fn font_arc(&self) -> Arc<dyn TextFont> {
        self.font.clone()
    }

    pub(crate) fn menu_background(&self) -> Option<ImageData> {
        self.menu_background.clone()
    }

    pub(crate) fn scenario_browser_background(&self) -> Option<ImageData> {
        self.scenario_browser_background.clone()
    }

    pub(crate) fn options_background(&self) -> Option<ImageData> {
        self.options_background.clone()
    }

    pub(crate) fn about_background(&self) -> Option<ImageData> {
        self.about_background.clone()
    }

    pub(crate) fn logo(&self) -> Option<ImageData> {
        self.logo.clone()
    }

    pub(crate) fn button_textures(&self) -> Option<ButtonTextures> {
        self.button_textures.clone()
    }

    pub(crate) fn require_classic_global_gui_bootstrap_resources(
        &self,
        active_failures: &HashMap<&'static str, String>,
    ) -> std::result::Result<(), ClassicParityBoundary> {
        let issues = self.classic_global_gui_bootstrap_issues(active_failures);
        if issues.is_empty() {
            Ok(())
        } else {
            Err(ClassicParityBoundary::GlobalGuiBootstrapResources { issues })
        }
    }

    fn classic_global_gui_bootstrap_issues(
        &self,
        active_failures: &HashMap<&'static str, String>,
    ) -> Vec<ClassicGuiBootstrapIssue> {
        let fonts = self.clonk_fonts.as_deref();
        let font_states = [
            ("FontRegular", fonts.map(|fonts| &fonts.text), true),
            ("FontTitle", fonts.map(|fonts| &fonts.title), true),
            ("FontCaption", fonts.map(|fonts| &fonts.caption), true),
            ("FontTiny", fonts.map(|fonts| &fonts.mini), true),
            ("FontTooltip", self.global_tooltip_font.as_deref(), false),
        ];
        let mut issues = Vec::new();
        for (name, font, shadowed) in font_states {
            if let Some(actual) = active_failures.get(name) {
                issues.push(ClassicGuiBootstrapIssue::malformed(
                    name,
                    "the exact active RX font source",
                    actual.clone(),
                ));
                continue;
            }
            if let Some(actual) = self.global_gui_font_failures.get(name) {
                issues.push(ClassicGuiBootstrapIssue::malformed(
                    name,
                    "the exact active RX font source",
                    actual.clone(),
                ));
                continue;
            }
            let Some(font) = font else {
                issues.push(ClassicGuiBootstrapIssue::missing(name));
                continue;
            };
            let expected_h_space = if shadowed { -1 } else { 0 };
            let atlas_ready = font.glyph('A').is_some_and(|glyph| {
                glyph.width > 0
                    && !glyph.pixels.is_empty()
                    && glyph.pixels.iter().any(|pixel| pixel.a != 0)
            });
            if font.line_height <= 0
                || font.cell_height <= 0
                || font.h_space != expected_h_space
                || !atlas_ready
            {
                issues.push(ClassicGuiBootstrapIssue::malformed(
                    name,
                    if shadowed {
                        "an initialized shadowed RX font"
                    } else {
                        "an initialized shadowless Main-14 RX font"
                    },
                    format!(
                        "line_height={}, cell_height={}, h_space={}, A_glyph_ready={atlas_ready}",
                        font.line_height, font.cell_height, font.h_space
                    ),
                ));
            }
        }

        for (stem, canonical_name) in CLASSIC_GLOBAL_GUI_SHEETS {
            if let Some(actual) = active_failures.get(stem) {
                issues.push(ClassicGuiBootstrapIssue::malformed(
                    stem,
                    "a readable selected bmp/jpeg/jpg/png RGBA surface",
                    actual.clone(),
                ));
                continue;
            }
            if let Some(actual) = self.global_gui_sheet_failures.get(stem) {
                issues.push(ClassicGuiBootstrapIssue::malformed(
                    stem,
                    "a readable selected bmp/jpeg/jpg/png RGBA surface",
                    actual.clone(),
                ));
                continue;
            }
            let Some(image) = self.startup_dialog_images.get(canonical_name) else {
                issues.push(ClassicGuiBootstrapIssue::missing(stem));
                continue;
            };
            let expected_len = (image.width() as usize)
                .checked_mul(image.height() as usize)
                .and_then(|pixels| pixels.checked_mul(4));
            if image.width() == 0
                || image.height() == 0
                || expected_len != Some(image.pixels().len())
            {
                issues.push(ClassicGuiBootstrapIssue::malformed(
                    stem,
                    "a non-empty decoded RGBA surface",
                    format!(
                        "{}x{} with {} bytes",
                        image.width(),
                        image.height(),
                        image.pixels().len()
                    ),
                ));
            }
        }
        if self.classic_hud_resources_required && !self.cursor_atlas.is_complete() {
            issues.push(ClassicGuiBootstrapIssue::missing(
                "CursorSmall..CursorXXXXXLarge",
            ));
        }
        if let Some(issue) = self.liquid_animation_issue.as_ref() {
            issues.push(issue.clone());
        }
        issues
    }

    pub(crate) fn require_classic_startup_bootstrap_resources(
        &self,
    ) -> std::result::Result<(), ClassicParityBoundary> {
        let issues = self.classic_startup_bootstrap_issues();
        if issues.is_empty() {
            Ok(())
        } else {
            Err(ClassicParityBoundary::StartupBootstrapResources { issues })
        }
    }

    pub(crate) fn classic_startup_bootstrap_issues(&self) -> Vec<ClassicStartupBootstrapIssue> {
        let mut issues = Vec::new();
        for name in CLASSIC_STARTUP_BOOTSTRAP_IMAGES {
            match self.startup_dialog_images.get(name) {
                None => match self.startup_bootstrap_image_failures.get(name) {
                    Some(actual) => issues.push(ClassicStartupBootstrapIssue::malformed(
                        name,
                        "a non-empty decoded RGBA surface",
                        actual.clone(),
                    )),
                    None => issues.push(ClassicStartupBootstrapIssue::missing(name)),
                },
                Some(image) => {
                    let expected_len = (image.width() as usize)
                        .checked_mul(image.height() as usize)
                        .and_then(|pixels| pixels.checked_mul(4));
                    if image.width() == 0
                        || image.height() == 0
                        || expected_len != Some(image.pixels().len())
                    {
                        issues.push(ClassicStartupBootstrapIssue::malformed(
                            name,
                            "a non-empty decoded RGBA surface",
                            format!(
                                "{}x{} with {} bytes",
                                image.width(),
                                image.height(),
                                image.pixels().len()
                            ),
                        ));
                    }
                }
            }
        }

        Self::push_startup_font_issue(
            &mut issues,
            "BookFontCapt",
            &[
                self.book_fonts.as_deref().map(|fonts| &fonts.caption),
                self.plrsel_book_fonts
                    .as_deref()
                    .map(|fonts| &fonts.caption),
            ],
        );
        Self::push_startup_font_issue(
            &mut issues,
            "BookFont",
            &[
                self.book_fonts.as_deref().map(|fonts| &fonts.text),
                self.options_book_fonts.as_deref().map(|fonts| &fonts.book),
                self.plrsel_book_fonts.as_deref().map(|fonts| &fonts.text),
            ],
        );
        Self::push_startup_font_issue(
            &mut issues,
            "BookFontTitle",
            &[self.book_fonts.as_deref().map(|fonts| &fonts.title)],
        );
        Self::push_startup_font_issue(
            &mut issues,
            "BookSmallFont",
            &[self
                .options_book_fonts
                .as_deref()
                .map(|fonts| &fonts.book_small)],
        );

        issues
    }

    fn push_startup_font_issue(
        issues: &mut Vec<ClassicStartupBootstrapIssue>,
        resource: &'static str,
        fonts: &[Option<&clonk_graphics::clonk_font::ClonkFont>],
    ) {
        if fonts.iter().any(Option::is_none) {
            issues.push(ClassicStartupBootstrapIssue::missing(resource));
            return;
        }
        if let Some(font) = fonts
            .iter()
            .copied()
            .flatten()
            .find(|font| font.line_height <= 0 || font.cell_height <= 0 || font.h_space != 0)
        {
            issues.push(ClassicStartupBootstrapIssue::malformed(
                resource,
                "an initialized shadowless RX font",
                format!(
                    "line_height={}, cell_height={}, h_space={}",
                    font.line_height, font.cell_height, font.h_space
                ),
            ));
        }
    }

    pub(crate) fn require_classic_startup_main_resources(
        &self,
    ) -> std::result::Result<(), ClassicParityBoundary> {
        let mut missing = Vec::new();
        if self.menu_background.is_none() {
            missing.push("LoaderGoldmine1.png");
        }
        if self.logo.is_none() {
            missing.push("Logo.png");
        }
        if self.button_textures.is_none() {
            missing.push("StartupBigButton.png/StartupBigButtonDown.png");
        }
        if self.button_highlight.is_none() {
            missing.push("GUIButtonHighlight.png");
        }
        if self.clonk_fonts.is_none() {
            missing.push("CStdFont/Endeavour.ttf");
        }
        if missing.is_empty() {
            Ok(())
        } else {
            Err(ClassicParityBoundary::StartupMainResources { missing })
        }
    }

    pub(crate) fn require_classic_ingame_menu_resources(
        &self,
    ) -> std::result::Result<(), ClassicParityBoundary> {
        let mut missing = Vec::new();
        if self.clonk_fonts.is_none() {
            missing.push("CStdFont/Endeavour.ttf");
        }
        for name in [
            "Menu.png",
            "Options.png",
            "Control.png",
            "GUIIcons.png",
            "Player.png",
            "GUICaption.png",
        ] {
            if !self.startup_dialog_images.contains_key(name) {
                missing.push(name);
            }
        }
        if self.hud_graphics.crew.is_none() {
            missing.push("Crew.png");
        }
        if self.hud_graphics.captain.is_none() {
            missing.push("Captain.png");
        }
        if missing.is_empty() {
            Ok(())
        } else {
            Err(ClassicParityBoundary::IngameMenuResources { missing })
        }
    }

    pub(crate) fn require_classic_game_over_resources(
        &self,
    ) -> std::result::Result<(), ClassicParityBoundary> {
        self.require_classic_game_over_resources_with_hud(self.hud_graphics.as_ref())
    }

    pub(crate) fn require_classic_game_over_resources_with_hud(
        &self,
        hud: &HudGraphics,
    ) -> std::result::Result<(), ClassicParityBoundary> {
        self.require_classic_game_over_resources_with_hud_and_evaluation(hud, None)
    }

    pub(crate) fn require_classic_game_over_resources_with_hud_and_evaluation(
        &self,
        hud: &HudGraphics,
        evaluation: Option<&EvaluationViewModel>,
    ) -> std::result::Result<(), ClassicParityBoundary> {
        let mut missing = Vec::new();
        if self.clonk_fonts.is_none() {
            missing.push("CStdFont/Endeavour.ttf".to_string());
        }
        for name in ["GUICaption.png", "GUIButton.png", "GUIButtonDown.png"] {
            if !self.startup_dialog_images.contains_key(name) {
                missing.push(name.to_string());
            }
        }
        if self.game_over_button_highlight.is_none() {
            missing.push("GUIButtonHighlight.png".to_string());
        }
        // GUIIcons2/GUIScroll are normally rejected by the earlier
        // process-global C4GUI gate. Keep them in this recursive inventory as
        // well so direct presentation validation remains complete.
        for name in ["GUIIcons.png", "GUIIcons2.png", "GUIScroll.png"] {
            if !self.startup_dialog_images.contains_key(name) {
                missing.push(name.to_string());
            }
        }
        if self
            .startup_dialog_images
            .get("GUIIcons2.png")
            .is_some_and(|image| resolve_league_evaluation_icon(image).is_none())
        {
            missing.push("GUIIcons2.png (Ico:League)".to_string());
        }
        if hud.player.is_none() && !self.startup_dialog_images.contains_key("Player.png") {
            missing.push("Player.png".to_string());
        }
        if hud.crew.is_none() {
            missing.push("Crew.png".to_string());
        }
        if hud.score.is_none() {
            missing.push("Score.png".to_string());
        }
        let malformed = |image: &ImageData| {
            image.width() == 0
                || image.height() == 0
                || u64::from(image.width())
                    .checked_mul(u64::from(image.height()))
                    .and_then(|pixels| pixels.checked_mul(4))
                    != Some(image.pixels().len() as u64)
        };
        if let Some(evaluation) = evaluation {
            for goal in evaluation.goals() {
                if goal.picture.as_ref().is_some_and(malformed) {
                    missing.push(format!("goal definition picture `{}`", goal.definition_id));
                }
            }
            for player in evaluation.players() {
                if player.big_icon.as_ref().is_some_and(malformed) {
                    missing.push(format!("player {} BigIcon", player.player_info_id));
                }
            }
        }
        if missing.is_empty() {
            Ok(())
        } else {
            Err(ClassicParityBoundary::GameOverResources { missing })
        }
    }

    pub(crate) fn require_classic_hud_resources_with_hud(
        &self,
        hud: &HudGraphics,
    ) -> std::result::Result<(), ClassicParityBoundary> {
        if !self.classic_hud_resources_required {
            return Ok(());
        }

        // `C4GraphicsResource::Init` returns false on the first game/HUD file
        // it cannot load, so every entry below is mandatory
        // (C4GraphicsResource.cpp:200-231). The order mirrors that sequence, so
        // the reported set reads like the native load order. `Options`, the
        // cursor atlas and `Liquid` have their own gates.
        let mandatory: [(&str, bool); 23] = [
            ("Control.png", hud.control.is_none()),
            ("Fire.png", hud.fire.is_none()),
            ("Background.png", hud.background.is_none()),
            ("Flag.png", hud.flag.is_none()),
            ("Crew.png", hud.crew.is_none()),
            ("Score.png", hud.score.is_none()),
            ("Wealth.png", hud.wealth.is_none()),
            ("Player.png", hud.player.is_none()),
            ("Rank.png", hud.rank.is_none()),
            ("Captain.png", hud.captain.is_none()),
            ("SelectMark.png", hud.select_mark.is_none()),
            ("Menu.png", hud.menu.is_none()),
            ("Logo.png", hud.logo.is_none()),
            ("Construction.png", hud.construction.is_none()),
            ("Energy.png", hud.energy.is_none()),
            ("Magic.png", hud.magic.is_none()),
            ("UpperBoard.png", hud.upper_board.is_none()),
            ("Arrow.png", hud.arrow.is_none()),
            ("Exit.png", hud.exit.is_none()),
            ("Hand.png", hud.hand.is_none()),
            ("Gamepad.png", hud.gamepad.is_none()),
            ("Build.png", hud.build.is_none()),
            ("EnergyBars.png", hud.energy_bars.is_none()),
        ];
        let missing: Vec<&str> = mandatory
            .into_iter()
            .filter_map(|(name, absent)| absent.then_some(name))
            .collect();
        if missing.is_empty() {
            Ok(())
        } else {
            Err(ClassicParityBoundary::HudResources { missing })
        }
    }

    pub(crate) fn cursor_atlas(&self) -> Arc<CursorAtlas> {
        Arc::clone(&self.cursor_atlas)
    }

    pub(crate) fn hud_graphics(&self) -> Arc<HudGraphics> {
        Arc::clone(&self.hud_graphics)
    }

    pub(crate) fn game_palette(&self) -> Arc<GamePalette> {
        Arc::clone(&self.game_palette)
    }

    pub(crate) fn liquid_animation(&self) -> Option<ImageData> {
        self.liquid_animation.clone()
    }

    pub(crate) fn liquid_animation_enabled(&self) -> bool {
        self.liquid_animation_enabled
    }

    pub(crate) fn base_sprite_map(&self) -> &HashMap<String, DefinitionSprite> {
        &self.base_sprites
    }

    pub(crate) fn load_hud_graphics(graphics: &GraphicsResource) -> HudGraphics {
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
            control: load("Control.png"),
            gamepad: load("Gamepad.png"),
            background: load("Background.png"),
        };

        if !missing.is_empty() {
            tracing::warn!(
                files = ?missing,
                "failed to load one or more HUD graphics from Graphics.c4g"
            );
        }

        hud
    }

    pub(crate) fn liquid_animation_issue(
        error: &anyhow::Error,
    ) -> Option<ClassicGuiBootstrapIssue> {
        if error.to_string() != "failed to load game graphics resource `Liquid`" {
            return None;
        }
        let detail = format!("{error:#}");
        if detail.contains("classic graphics resource `Liquid` is unavailable") {
            Some(ClassicGuiBootstrapIssue::missing("Liquid"))
        } else {
            Some(ClassicGuiBootstrapIssue::malformed(
                "Liquid",
                "a readable selected bmp/jpeg/jpg/png surface",
                detail,
            ))
        }
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

pub(crate) fn snapshot_effective_client_player_selection(
    paths: &AppPaths,
    classic: &ClassicCommandLine,
) -> Result<ConfiguredClientPlayerSelection> {
    let mut selection = snapshot_configured_client_player_selection(paths)?;
    if !classic.player_files.is_empty() {
        selection.replace_participant_modules(&classic.player_files);
    }
    Ok(selection)
}

pub(crate) fn client_settings_for_paths(
    server_addr: SocketAddr,
    player_name: String,
    paths: Option<&AppPaths>,
) -> Result<ClientSettings> {
    let mut settings = ClientSettings::new(server_addr, player_name);
    let query = load_reference_query_settings(paths);
    settings.league_transport = clonk_network::LeagueHttpTransportConfig {
        language_charset: query.language_charset,
        language_sequence: query.language_sequence,
        http_backend: query.http_backend,
    };
    settings.league_auth = load_league_auth_settings(paths);
    if let Some(paths) = paths {
        let (client_name, client_nick) = load_classic_network_identity(paths)?;
        settings.client_name = client_name;
        settings.client_nick = client_nick;
        if let Ok(selection) = snapshot_configured_client_player_selection(paths) {
            settings.group_maker = selection.group_maker().clone();
        }
        let ports = load_network_ports(Some(paths));
        settings.mesh_tcp_bind_address =
            (ports.tcp != 0).then_some(SocketAddr::from(([0_u16; 8], ports.tcp)));
        settings.mesh_udp_bind_address =
            (ports.udp != 0).then_some(SocketAddr::from(([0_u16; 8], ports.udp)));
        settings.resource_directory = paths
            .cache_dir()
            .join(network_work_directory_name(Some(paths)));
        settings.local_system_path = Some(paths.system_group_path().to_path_buf());
        settings.local_resource_roots = vec![
            paths.install_root().to_path_buf(),
            paths.planet_dir().to_path_buf(),
        ];
        if let Some(content) = paths.content_dir() {
            settings.local_resource_roots.push(content.to_path_buf());
        }
        // `SearchLocal` bounds its candidate walk by the configured depth
        // (C4Network2Res.cpp:460-490).
        settings.max_resource_search_recursion = load_max_resource_search_recursion(Some(paths));
    }
    Ok(settings)
}

fn apply_classic_client_settings(
    settings: &mut ClientSettings,
    classic: &ClassicCommandLine,
) -> Result<()> {
    settings.observer = classic.observe;
    if let Some(password) = classic.password.as_deref() {
        let password = clonk_resources::encode_legacy_script_text(password).ok_or_else(|| {
            anyhow!("classic network password is not representable as Windows-1252")
        })?;
        settings.password = LegacyCString::from_bytes(password)
            .ok_or_else(|| anyhow!("classic network password contains an interior NUL"))?;
    }
    if let Some(port) = classic.tcp_port {
        settings.mesh_tcp_bind_address =
            (port != 0).then_some(SocketAddr::from(([0_u16; 8], port)));
    }
    if let Some(port) = classic.udp_port {
        settings.mesh_udp_bind_address =
            (port != 0).then_some(SocketAddr::from(([0_u16; 8], port)));
    }
    Ok(())
}

pub(crate) fn classic_client_settings_for_reference(
    reference: &clonk_network::NetworkGameReference,
    player_name: String,
    paths: Option<&AppPaths>,
    group_maker: Option<LegacyCString>,
    classic: &ClassicCommandLine,
) -> Result<ClientSettings> {
    let mut settings = client_settings_for_paths(reference.source_address, player_name, paths)?
        .with_compatibility_build(reference.build)
        .with_join_route_plan(reference.join_route_plan_for_local_host())
        .with_netpuncher(
            reference.netpuncher_address.clone(),
            clonk_network::NetpuncherGameIds {
                ipv4: reference.netpuncher_ipv4,
                ipv6: reference.netpuncher_ipv6,
            },
        );
    if let Some(group_maker) = group_maker {
        settings.group_maker = group_maker;
    }
    apply_classic_client_settings(&mut settings, classic)?;
    Ok(settings)
}

pub(crate) fn startup_network_connect_targets(settings: &ClientSettings) -> String {
    settings
        .logical_server_addresses
        .iter()
        .filter(|address| !address.is_ip_null() && settings.join_protocol_enabled(address))
        .map(|address| {
            let protocol = match address.protocol {
                clonk_network::NetworkProtocol::Tcp => "TCP".to_string(),
                clonk_network::NetworkProtocol::Udp => "UDP".to_string(),
                _ => "INVALID".to_string(),
            };
            format!("{protocol}:{}", address.endpoint)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

pub(crate) fn resolve_network_mode(
    cli: &Cli,
    classic: &ClassicCommandLine,
    paths: Option<&AppPaths>,
) -> Result<Option<NetworkMode>> {
    if let Some(ref host_addr) = cli.host {
        let bind_addr = parse_socket_addr(host_addr, "host")?;
        return Ok(Some(NetworkMode::Host(HostSettings {
            bind_addr,
            player_name: cli.player_name.clone(),
            prepared: None,
        })));
    }
    if let Some(ref join_addr) = cli.join {
        let server_addr = parse_socket_addr(join_addr, "join")?;
        let mut settings = client_settings_for_paths(server_addr, cli.player_name.clone(), paths)?;
        apply_classic_client_settings(&mut settings, classic)?;
        return Ok(Some(NetworkMode::Client(settings)));
    }
    Ok(None)
}

fn parse_socket_addr(input: &str, kind: &str) -> Result<SocketAddr> {
    input
        .parse::<SocketAddr>()
        .with_context(|| format!("invalid {kind} address `{input}`"))
}

pub(crate) fn resolve_join_socket(input: &str, default_port: u16) -> Result<SocketAddr> {
    let input = input.trim();
    if input.is_empty() {
        return Err(anyhow!("address is empty"));
    }
    if let Ok(address) = input.parse::<SocketAddr>() {
        return Ok(address);
    }
    if let Ok(ip) = input.parse::<IpAddr>() {
        return Ok(SocketAddr::new(ip, default_port));
    }
    let address = if input.rsplit_once(':').is_some_and(|(_, port)| {
        !port.is_empty() && port.chars().all(|character| character.is_ascii_digit())
    }) {
        input.to_string()
    } else {
        format!("{input}:{default_port}")
    };
    address
        .to_socket_addrs()
        .with_context(|| format!("could not resolve `{input}`"))?
        .next()
        .ok_or_else(|| anyhow!("`{input}` resolved to no socket addresses"))
}

pub(crate) fn classic_direct_reference_endpoint(
    input: &str,
    paths: Option<&AppPaths>,
) -> Result<clonk_network::ReferenceEndpoint> {
    clonk_network::direct_reference_endpoint(input, load_network_reference_port(paths))
        .map_err(anyhow::Error::msg)
}

pub(crate) fn query_first_classic_reference(
    endpoint: clonk_network::ReferenceEndpoint,
    config: &clonk_network::ReferenceQueryConfig,
) -> std::result::Result<clonk_network::NetworkGameReference, NetworkStartError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| {
            NetworkStartError::Other(format!("unable to start reference query: {error}"))
        })?;
    runtime
        .block_on(clonk_network::fetch_reference_endpoint_with_config(
            endpoint,
            clonk_network::REFERENCE_QUERY_TIMEOUT,
            config,
        ))
        .map_err(|error| NetworkStartError::Other(format!("reference query failed: {error}")))?
        .into_iter()
        .next()
        .ok_or_else(|| {
            NetworkStartError::Other("reference query returned no network games".to_string())
        })
}

pub(crate) fn test_scenario_load(
    path: &std::path::Path,
    app_paths: Option<&Arc<AppPaths>>,
) -> Result<()> {
    use std::time::Instant;

    println!("Testing scenario load from: {}", path.display());
    println!(
        "Using InstallDefinitionResolver with app paths: {}",
        if app_paths.is_some() { "yes" } else { "no" }
    );

    let resolver = InstallDefinitionResolver::new(app_paths.cloned());
    let languages = startup_language_sequence(app_paths.map(|paths| paths.as_ref()));
    let start = Instant::now();

    match Scenario::load_from_path_with_languages(path, &resolver, &languages) {
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
pub(crate) fn run_sandbox_dump(
    dump_path: &std::path::Path,
    test_frames: u32,
    app_paths: Option<&Arc<AppPaths>>,
    runtime: RuntimeConfig,
) -> Result<()> {
    use std::thread;
    use std::time::Duration;

    let mut app = GameApp::new_with_debug_hud(
        1280,
        720,
        AudioOptions::default(),
        app_paths.map(|v| &**v),
        runtime,
        false,
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
    app.discard_terminal_loader_frame_for_headless_render();

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

/// The Options tab named by an `options:<sheet>` menu view.
fn options_sheet_by_name(name: &str) -> Option<clonk_frontend::startup_options_dlg::OptionsSheet> {
    use clonk_frontend::startup_options_dlg::OptionsSheet;
    match name.to_ascii_lowercase().as_str() {
        "program" => Some(OptionsSheet::Program),
        "graphics" => Some(OptionsSheet::Graphics),
        // `audio` is the caption the sheet now carries
        // (clonk-org/clonk-rs#452); `sound` stays accepted because it is the
        // C++ name and what existing invocations pass.
        "audio" | "sound" => Some(OptionsSheet::Sound),
        "keyboard" => Some(OptionsSheet::Keyboard),
        "gamepad" => Some(OptionsSheet::Gamepad),
        "network" => Some(OptionsSheet::Network),
        _ => None,
    }
}

/// Headless: boot to the startup main menu (`AppMode::Menu`), render one frame to
/// the renderer's CPU surface, and write it as a PNG. Counterpart of
/// `run_sandbox_dump` for startup-menu rendering-parity checks against the C++
/// engine's F9 screenshots.
pub(crate) fn run_menu_dump(
    dump_path: &std::path::Path,
    menu_view: &str,
    app_paths: Option<&Arc<AppPaths>>,
    runtime: RuntimeConfig,
) -> Result<()> {
    use std::thread;
    use std::time::Duration;

    let mut app = GameApp::new_with_debug_hud(
        1280,
        720,
        AudioOptions::default(),
        app_paths.map(|v| &**v),
        runtime,
        false,
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
    // the UI uses, so per-view state objects exist. "scenarios:<Folder>[/..]"
    // additionally descends into the named folder(s) of the book.
    let (view_name, folder_path) = menu_view
        .split_once(':')
        .map(|(view, path)| (view, Some(path)))
        .unwrap_or((menu_view, None));
    // "plrprops[:<color>/<portrait>]" opens the first-start new-player form over
    // the main menu with pinned draws, so captures are reproducible.
    if view_name == "plrprops" {
        let (color, portrait) = folder_path
            .and_then(|spec| spec.split_once('/'))
            .and_then(|(color, portrait)| {
                Some((
                    color.parse::<usize>().ok()?,
                    portrait.parse::<usize>().ok()?,
                ))
            })
            .unwrap_or((0, 0));
        let controller = app.new_startup_player_properties_controller(color, portrait);
        app.startup.player_properties_dialog = Some(PendingStartupPlayerProperties {
            origin: StartupPlayerPropertiesOrigin::MainMenuFirstPlayer,
            controller,
        });
        return finish_menu_dump(&mut app, dump_path);
    }
    let item = match view_name {
        "main" => None,
        "scenarios" => Some(MainMenuItem::LocalGame),
        "options" => Some(MainMenuItem::Options),
        "about" => Some(MainMenuItem::About),
        "plrsel" => Some(MainMenuItem::PlayerSelection),
        "net" => Some(MainMenuItem::NetworkGame),
        other => anyhow::bail!(
            "unknown --menu-view `{other}` (main|scenarios|options|about|plrsel|net|plrprops)"
        ),
    };
    if let Some(item) = item {
        app.handle_main_menu_activation(item)
            .map_err(|err| anyhow::anyhow!("activating menu view `{menu_view}`: {err}"))?;
    }
    // "options:<sheet>" opens the named tab, the way clicking its tab clip does,
    // so a capture can target one sheet instead of the default Program page.
    if view_name == "options" {
        if let Some(name) = folder_path {
            let sheet = options_sheet_by_name(name)
                .ok_or_else(|| anyhow::anyhow!("unknown Options sheet `{name}`"))?;
            app.startup
                .options_dialog
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("Options dialog is not open"))?
                .restore_sheet(sheet);
        }
        return finish_menu_dump(&mut app, dump_path);
    }
    if let Some(path) = folder_path {
        for segment in path.split('/').filter(|segment| !segment.is_empty()) {
            let identifier = app
                .menu_state
                .current_entries()
                .iter()
                .find(|entry| entry.title.eq_ignore_ascii_case(segment))
                .map(|entry| entry.identifier.clone())
                .ok_or_else(|| anyhow::anyhow!("no folder titled `{segment}` in the book"))?;
            app.menu_state
                .require_supported_activation(&identifier)
                .map_err(report_classic_parity_boundary)
                .map_err(anyhow::Error::new)?;
            app.enter_scenario_folder(&identifier);
            let actions = app.menu_state.select_default_entry();
            app.process_menu_actions(actions)
                .map_err(anyhow::Error::new)?;
        }
        app.scenario_label = app.menu_state.label_path();
    }

    finish_menu_dump(&mut app, dump_path)
}

/// Renders one settled startup frame and writes it to `dump_path`.
fn finish_menu_dump(app: &mut GameApp, dump_path: &std::path::Path) -> Result<()> {
    let (w, h) = {
        let s = app.graphics.surface();
        (s.width(), s.height())
    };
    let mut frame = vec![0u8; (w as usize) * (h as usize) * 4];
    while app.startup_dialog_fade_active() {
        app.render(&mut frame)
            .context("failed to settle startup dialog fade for menu dump")?;
    }
    app.render(&mut frame)
        .context("failed to render menu frame")?;
    let png = encode_surface_to_png(app.graphics.surface())
        .context("failed to encode menu frame to PNG")?;
    std::fs::write(dump_path, &png)
        .with_context(|| format!("failed to write {}", dump_path.display()))?;
    println!("wrote {} ({}x{}, startup menu)", dump_path.display(), w, h);
    Ok(())
}

pub(crate) fn run_integration_test(
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
    let mut app = GameApp::new_with_debug_hud(
        640, // width
        480, // height
        AudioOptions::default(),
        app_paths.map(|v| &**v),
        runtime,
        false,
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
        mission_access: None,
        path: Some(scenario_path.to_path_buf()),
        source_paths: Vec::new(),
        root_label: None,
        preview: None,
        title_picture: None,
        children: Vec::new(),
        folder_index: None,
        icon_index: None,
        difficulty: None,
        author: None,
        version: None,
        local_only: None,
        allow_user_change: None,
        definition_modules: Vec::new(),
    };

    println!("Starting scenario: {}", scenario.title);

    // Start scenario (begins async loading)
    app.start_scenario(scenario)
        .context("failed to start scenario")?;

    // Wait for running state (reuses test helper pattern). Real packed
    // scenarios take several seconds to load (350 defs from Objects.c4d),
    // so allow up to ~30s.
    let mut waited_frames = 0;
    for _ in 0..15_000 {
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
        anyhow::bail!("Scenario did not enter Running mode after 15000 update cycles");
    }

    // Run test frames, optionally with scripted control input:
    // LC_APP_TEST_INPUT="5:Right+,20:Right-,25:Up+" presses (+) and
    // releases (-) the named control at the given test frame, driving the
    // same dispatch path as live key input.
    let scripted_input = std::env::var("LC_APP_TEST_INPUT")
        .ok()
        .map(|spec| parse_test_input_spec(&spec))
        .transpose()
        .context("parse LC_APP_TEST_INPUT")?
        .unwrap_or_default();
    println!("Running {} test frames...", test_frames);
    let mut last_action: Option<String> = None;
    for frame in 0..test_frames {
        for (when, event) in &scripted_input {
            if *when == frame {
                println!("  test input at frame {frame}: {event:?}");
                let owner = app.players.local_owner;
                app.dispatch_control_event_for_owner(owner, *event)
                    .map_err(|error| anyhow::anyhow!("scripted input failed: {error}"))?;
            }
        }
        app.update()
            .with_context(|| format!("failed to update app at frame {}", frame))?;
        // Scripted-input forensics: log the cursor crew's action
        // transitions so control-chain effects are observable per frame.
        if !scripted_input.is_empty() {
            if let Some(snapshot) = app
                .engine
                .crew_cursor(app.players.local_owner)
                .and_then(|cursor| app.engine.object_snapshot(cursor))
            {
                if last_action.as_deref() != Some(snapshot.action.name.as_str()) {
                    println!(
                        "  frame {frame}: cursor action -> {} pos=({}, {}) comdir={:?}",
                        snapshot.action.name,
                        snapshot.position.x,
                        snapshot.position.y,
                        snapshot.command_direction
                    );
                    last_action = Some(snapshot.action.name.clone());
                }
            }
        }
    }

    // Forensics for scripted-input runs: where did the cursor crew end up?
    if let Some(cursor) = app.engine.crew_cursor(app.players.local_owner) {
        if let Some(snapshot) = app.engine.object_snapshot(cursor) {
            println!(
                "  cursor crew: def={} action={} pos=({}, {}) comdir={:?}",
                snapshot.definition_id,
                snapshot.action.name,
                snapshot.position.x,
                snapshot.position.y,
                snapshot.command_direction
            );
        }
    }

    let elapsed = start.elapsed();
    println!(
        "\n✓ Integration test PASSED in {:.2}s",
        elapsed.as_secs_f32()
    );
    println!("  Scenario started successfully");
    println!("  Ran {} frames without errors", test_frames);

    // Optionally open the in-game player menu before the frame dump so its
    // rendering can be captured headlessly. Values other than "player" jump
    // into the named submenu; LC_APP_OPEN_MENU_FRAMES controls how long the
    // menu idles (e.g. past the 90-frame tooltip delay, C4Menu.cpp:37).
    if let Ok(page) = std::env::var("LC_APP_OPEN_MENU") {
        app.open_ingame_menu()
            .context("opening the player menu for headless capture")?;
        let submenu = match page.as_str() {
            "options" => Some(MenuAction::ActivateOptions),
            "display" => Some(MenuAction::ActivateDisplay),
            "savegame" => Some(MenuAction::ActivateSavegame),
            "goals" => Some(MenuAction::ActivateGoals),
            "rules" => Some(MenuAction::ActivateRules),
            "hostility" => Some(MenuAction::ActivateHostility),
            "surrender" => Some(MenuAction::ActivateSurrender),
            "abort" => Some(MenuAction::Abort),
            _ => None,
        };
        if let Some(action) = submenu {
            app.apply_ingame_menu_action(action)
                .context("simulated submenu activation")?;
        }
        let idle_frames = std::env::var("LC_APP_OPEN_MENU_FRAMES")
            .ok()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(5);
        for frame in 0..idle_frames {
            app.update()
                .with_context(|| format!("failed to update app with menu open, frame {frame}"))?;
        }
    }

    // Optional visual check: render the final frame to a PNG
    // (rendering-parity forensics vs the C++ engine's F9 shots).
    if let Ok(dump) = std::env::var("LC_APP_DUMP_FRAME") {
        app.discard_terminal_loader_frame_for_headless_render();
        let (w, h) = {
            let s = app.graphics.surface();
            (s.width(), s.height())
        };
        let mut frame = vec![0u8; (w as usize) * (h as usize) * 4];
        app.render(&mut frame).context("render integration frame")?;
        let png =
            encode_surface_to_png(app.graphics.surface()).context("encode integration frame")?;
        std::fs::write(&dump, &png).with_context(|| format!("write {dump}"))?;
        println!("  wrote {dump} ({w}x{h})");
    }

    // Forensics: `LC_APP_DUMP_OBJECTS=1` lists every object with its def,
    // position, category, container and whether a sprite was registered —
    // the render-parity companion to LC_APP_DUMP_FRAME.
    if std::env::var("LC_APP_DUMP_OBJECTS").is_ok() {
        let snapshot = app.engine.snapshot();
        for object in &snapshot.objects {
            let has_sprite = app
                .object_sprites
                .contains_key(&sprite_map_key(&object.definition_id, None));
            let overlays = object
                .graphics_overlays
                .iter()
                .map(|overlay| {
                    format!(
                        "{}:{:?}{}",
                        overlay.id,
                        overlay.mode,
                        overlay
                            .action
                            .as_deref()
                            .map(|action| format!("({action})"))
                            .unwrap_or_default()
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            println!(
                "  obj id={} def={} pos=({}, {}) cat={:#x} sprite={} contained={:?} overlays=[{}] action={}",
                object.id.as_u64(),
                object.definition_id,
                object.position.x,
                object.position.y,
                object.category,
                has_sprite,
                object.container.map(|id| id.as_u64()),
                overlays,
                object.action.name,
            );
        }
    }

    Ok(())
}

/// `LC_APP_TEST_INPUT` grammar: comma-separated `frame:Name+` (press) /
/// `frame:Name-` (release) entries with Name in Left/Right/Up/Down/
/// Throw/Dig/Special/Special2.
fn parse_test_input_spec(spec: &str) -> Result<Vec<(u32, ControlEvent)>> {
    spec.split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            let (frame, action) = entry
                .split_once(':')
                .ok_or_else(|| anyhow::anyhow!("expected frame:Action in `{entry}`"))?;
            let frame: u32 = frame.trim().parse()?;
            let action = action.trim();
            let (name, press) = match action.as_bytes().last() {
                Some(b'+') => (&action[..action.len() - 1], true),
                Some(b'-') => (&action[..action.len() - 1], false),
                _ => (action, true),
            };
            let event = match name {
                "Left" | "Right" | "Up" | "Down" => {
                    let button = match name {
                        "Left" => clonk_engine::ControlButton::Left,
                        "Right" => clonk_engine::ControlButton::Right,
                        "Up" => clonk_engine::ControlButton::Up,
                        _ => clonk_engine::ControlButton::Down,
                    };
                    if press {
                        ControlEvent::Press(button)
                    } else {
                        ControlEvent::Release(button)
                    }
                }
                "Throw" | "Dig" | "Special" | "Special2" => {
                    let command = match name {
                        "Throw" => ControlCommand::Throw,
                        "Dig" => ControlCommand::Dig,
                        "Special" => ControlCommand::Special,
                        _ => ControlCommand::Special2,
                    };
                    ControlEvent::Command {
                        command,
                        kind: if press {
                            CommandKind::Press
                        } else {
                            CommandKind::Release
                        },
                    }
                }
                other => anyhow::bail!("unknown control `{other}` in `{entry}`"),
            };
            Ok((frame, event))
        })
        .collect()
}

const CLASSIC_CONFIG_RESET_SAFETY: i32 = 42;
const CLASSIC_DEFAULT_RESOLUTION_X: i32 = 800;
pub(crate) const CUSTOM_CONFIG_CORRUPTED_ERROR: &str =
    "Warning: Custom configuration corrupted - program abort!";

fn load_startup_integrity_config(config_path: &Path) -> Result<Option<Vec<u8>>> {
    let bytes = match fs::read(config_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "failed to read startup configuration {}",
                    config_path.display()
                )
            });
        }
    };
    Ok(Some(bytes))
}

fn narrow_startup_strtol_value(value: i128) -> i32 {
    // C4's INI reader parses DWord values through native `long` and then
    // assigns them to int32_t. `long` is 32-bit on Windows and 32-bit targets,
    // but 64-bit on the Unix targets we support.
    #[cfg(all(not(windows), target_pointer_width = "64"))]
    {
        value.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64 as i32
    }
    #[cfg(any(windows, target_pointer_width = "32"))]
    {
        value.clamp(i128::from(i32::MIN), i128::from(i32::MAX)) as i32
    }
}

pub(crate) fn parse_startup_config_integer(value: &[u8]) -> Option<i32> {
    let start = value
        .iter()
        .position(|byte| !matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
        .unwrap_or(value.len());
    let value = &value[start..];
    let radix = if value.first() == Some(&b'0')
        && value
            .get(1)
            .is_some_and(|byte| byte.eq_ignore_ascii_case(&b'x'))
    {
        16_i128
    } else {
        10_i128
    };
    let (negative, unsigned) = match value.first() {
        Some(b'-') => (true, &value[1..]),
        Some(b'+') => (false, &value[1..]),
        _ => (false, value),
    };
    let digits = if radix == 16 {
        unsigned.get(2..).unwrap_or_default()
    } else {
        unsigned
    };
    let mut parsed = 0_i128;
    // strtol(base 16) consumes the leading zero even when `x` is not followed
    // by a hex digit, so a bare `0x` is the numeric value zero.
    let mut consumed = radix == 16;
    for byte in digits.iter().copied() {
        let digit = match byte {
            b'0'..=b'9' => i128::from(byte - b'0'),
            b'a'..=b'f' if radix == 16 => i128::from(byte - b'a') + 10,
            b'A'..=b'F' if radix == 16 => i128::from(byte - b'A') + 10,
            _ => break,
        };
        if digit >= radix {
            break;
        }
        parsed = parsed.saturating_mul(radix).saturating_add(digit);
        consumed = true;
    }
    consumed.then(|| narrow_startup_strtol_value(if negative { -parsed } else { parsed }))
}

fn startup_config_integer(config: &[u8], section: &str, key: &str, default: i32) -> i32 {
    clonk_app_netplay::configured_native_scalar(config, section, key)
        .and_then(parse_startup_config_integer)
        .unwrap_or(default)
}

fn narrow_startup_strtoul_value(value: u128, negative: bool) -> u32 {
    // C++'s DWord INI overload parses through native `unsigned long` before
    // narrowing to uint32_t. Keep Unix-64's wider parse range and Windows /
    // 32-bit saturation boundary, including unsigned negation.
    #[cfg(all(not(windows), target_pointer_width = "64"))]
    {
        if value > u128::from(u64::MAX) {
            return u64::MAX as u32;
        }
        let value = value as u64;
        (if negative {
            value.wrapping_neg()
        } else {
            value
        }) as u32
    }
    #[cfg(any(windows, target_pointer_width = "32"))]
    {
        if value > u128::from(u32::MAX) {
            return u32::MAX;
        }
        let value = value as u32;
        if negative {
            value.wrapping_neg()
        } else {
            value
        }
    }
}

pub(crate) fn parse_startup_config_unsigned(value: &[u8]) -> Option<u32> {
    let start = value
        .iter()
        .position(|byte| !matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
        .unwrap_or(value.len());
    let value = &value[start..];
    // C4's parser selects the radix before strtoul consumes an optional sign.
    // Consequently `-0x10` is decimal zero (the leading `0` is consumed).
    let radix = if value.first() == Some(&b'0')
        && value
            .get(1)
            .is_some_and(|byte| byte.eq_ignore_ascii_case(&b'x'))
    {
        16_u128
    } else {
        10_u128
    };
    let (negative, unsigned) = match value.first() {
        Some(b'-') => (true, &value[1..]),
        Some(b'+') => (false, &value[1..]),
        _ => (false, value),
    };
    let digits = if radix == 16 {
        unsigned.get(2..).unwrap_or_default()
    } else {
        unsigned
    };
    let mut parsed = 0_u128;
    let mut consumed = radix == 16;
    for byte in digits.iter().copied() {
        let digit = match byte {
            b'0'..=b'9' => u128::from(byte - b'0'),
            b'a'..=b'f' if radix == 16 => u128::from(byte - b'a') + 10,
            b'A'..=b'F' if radix == 16 => u128::from(byte - b'A') + 10,
            _ => break,
        };
        if digit >= radix {
            break;
        }
        parsed = parsed.saturating_mul(radix).saturating_add(digit);
        consumed = true;
    }
    consumed.then(|| narrow_startup_strtoul_value(parsed, negative))
}

fn startup_config_unsigned(config: &[u8], section: &str, key: &str, default: u32) -> u32 {
    clonk_app_netplay::configured_native_scalar(config, section, key)
        .and_then(parse_startup_config_unsigned)
        .unwrap_or(default)
}

pub(crate) fn load_advanced_renderer_config(
    config: &[u8],
) -> clonk_frontend::AdvancedRendererConfig {
    let defaults = clonk_frontend::AdvancedRendererConfig::DEFAULT;
    let boolean = |key: &str, default: bool| {
        clonk_app_netplay::configured_native_boolean(config, "Graphics", key).unwrap_or(default)
    };
    clonk_frontend::AdvancedRendererConfig {
        no_alpha_add: boolean("NoAlphaAdd", defaults.no_alpha_add),
        no_box_fades: boolean("NoBoxFades", defaults.no_box_fades),
        tex_indent: startup_config_integer(config, "Graphics", "TexIndent", defaults.tex_indent),
        blit_offset: startup_config_integer(config, "Graphics", "BlitOffset", defaults.blit_offset),
        allowed_blit_modes: startup_config_unsigned(
            config,
            "Graphics",
            "AllowedBlitModes",
            defaults.allowed_blit_modes,
        ),
        shader: boolean("Shader", defaults.shader),
        use_shader_gamma: boolean("UseShaderGamma", defaults.use_shader_gamma),
        disable_gamma: boolean("DisableGamma", defaults.disable_gamma),
    }
}

/// `Graphics.Remaster`: the master switch for every presentation-only
/// divergence from the oracle. Each individual key still wins where the player
/// set it explicitly, so this only supplies their default.
pub(crate) fn configured_remaster(config: &[u8]) -> bool {
    clonk_app_netplay::configured_native_boolean(config, "Graphics", "Remaster").unwrap_or(false)
}

fn configured_remaster_feature(config: &[u8], key: &str) -> bool {
    clonk_app_netplay::configured_native_boolean(config, "Graphics", key)
        .unwrap_or_else(|| configured_remaster(config))
}

/// `Graphics.HighDpiCursor`: opt in to the physical-width cursor ladder. This
/// has no C++ counterpart — the oracle only grows the pointer with
/// `Graphics.Scale` — so it is off unless the remaster is on.
pub(crate) fn configured_high_dpi_cursor(config: &[u8]) -> bool {
    configured_remaster_feature(config, "HighDpiCursor")
}

/// Legacy `Graphics.Mipmaps` preference retained for config compatibility.
/// Art shaders currently pin LOD zero, so the renderer does not allocate or
/// upload unreachable lower texture levels.
pub(crate) fn configured_mipmaps(config: &[u8]) -> bool {
    configured_remaster_feature(config, "Mipmaps")
}

/// `Graphics.SmoothLandscape`: opt in to alpha-weighted magnification of the
/// landscape. C++ blits its landscape surface with GL_NEAREST.
pub(crate) fn configured_smooth_landscape(config: &[u8]) -> bool {
    configured_remaster_feature(config, "SmoothLandscape")
}

/// `Graphics.FineFogOfWar`: subdivide the fog-of-war modulation grid so the
/// visibility boundary stops showing 64-world-pixel facets.
pub(crate) fn configured_fine_fog_of_war(config: &[u8]) -> bool {
    configured_remaster_feature(config, "FineFogOfWar")
}

/// `Graphics.HDExactBlits`: treat a source that matches its destination in
/// PHYSICAL pixels as exact, so higher-resolution definition art lands one
/// authored texel per device pixel instead of a corrected linear resample.
pub(crate) fn configured_hd_exact_blits(config: &[u8]) -> bool {
    configured_remaster_feature(config, "HDExactBlits")
}

/// `Graphics.ShaderLandscape`: compose the landscape in the fragment shader
/// instead of on the CPU. The CPU composer walks integer landscape coordinates,
/// so one pattern texel per landscape pixel is its ceiling and higher-resolution
/// material art only stretches the tiling period. The shader evaluates the same
/// arithmetic per fragment, which is what `Graphics.LandscapeDetail` needs.
///
/// Deliberately has no advanced-settings row: `default_config` materializes
/// every editor row, and `configured_remaster_feature` only consults
/// `Graphics.Remaster` while a key is ABSENT, so a row would silently stop the
/// master switch from reaching this after a config repair. `SnapTextToPixels`
/// and `SmoothPresentation` are the same hand-edit-only shape.
pub(crate) fn configured_shader_landscape(config: &[u8]) -> bool {
    configured_remaster_feature(config, "ShaderLandscape")
}

/// `Graphics.LandscapeDetail`: landscape supersampling factor for the shader
/// composer. 1 reproduces the CPU composer byte for byte, so the default is
/// C++-exact; N evaluates the material pattern at 1/N of a landscape pixel, so
/// N-times-larger art keeps its world-space tiling period rather than stretching
/// it across N times as much world.
///
/// Clamped here rather than only in the editor, because a hand-edited config
/// reaches this reader directly and the composer rejects 0 outright.
pub(crate) fn configured_landscape_detail(config: &[u8]) -> u32 {
    u32::try_from(startup_config_integer(
        config,
        "Graphics",
        "LandscapeDetail",
        1,
    ))
    .unwrap_or(1)
    .clamp(1, clonk_app_render::gpu_renderer::MAX_LANDSCAPE_DETAIL)
}

/// `Graphics.LoaderAspect`: cover-fit the fullscreen loader image instead of
/// C++'s unconditional non-aspect stretch.
pub(crate) fn configured_loader_aspect(config: &[u8]) -> bool {
    configured_remaster_feature(config, "LoaderAspect")
}

/// `Graphics.SnapTextToPixels`: opt in to rasterizing the scale-native GUI
/// fonts at `round(logical * Graphics.Scale)` and blitting their glyph cells
/// at whole physical pixels. C++ truncates the scaled FreeType height
/// (`StdFont.cpp:325`) and then rescales every facet by
/// `scale / effective_scale` (`StdFont.cpp:685,701,841`), so at a fractional
/// scale like 150% the oracle resamples all text. This changes glyph bytes and
/// therefore defaults off.
pub(crate) fn configured_snap_text_to_pixels(config: &[u8]) -> bool {
    configured_remaster_feature(config, "SnapTextToPixels")
}

/// `Graphics.SkyDither`: opt in to sub-LSB dithering of the sky gradient.
/// C++ emits the fade as one plain interpolated quad into an 8-bit target, so
/// visible bands equal the channel delta spread over viewport height: the
/// shipped default fade `RGB(28,64,152)→RGB(192,196,252)` spans 100 blue
/// steps, a band every ~22 rows at 2160p, and it worsens as panels grow. The
/// divergence adds interleaved-gradient noise on a triangular PDF spanning one
/// step before the framebuffer quantizes; the mean is unchanged, so the result
/// is closer to the exact ramp than the banded output. It is presentation-only
/// and is set only on the sky path and only on a quad whose corner colours
/// actually differ, and it defaults off so the default path stays
/// byte-identical to the oracle.
pub(crate) fn configured_sky_dither(config: &[u8]) -> bool {
    configured_remaster_feature(config, "SkyDither")
}

/// `Graphics.SmoothPresentation`: opt in to presenting at the display's own
/// refresh period instead of C++'s 30 ms `Graphics.MaxRefreshDelay` ceiling.
///
/// C++ defaults `Graphics.MaxRefreshDelay` to 30 (src/C4Config.cpp:485)
/// against a 28 ms game tick, so `C4Application` leaves that tick undivided
/// (src/C4Application.cpp:510-531) and presents once per tick; the startup
/// timer is a flat 16 ms. That is invisible for world content — the simulation
/// really does advance only every 28 ms — but the mouse pointer is drawn *into*
/// the frame (`draw_classic_gui_cursor`) with the platform cursor hidden, so
/// the refresh period is also the pointer's update period: measured 35.7 Hz in
/// game and 62.9 Hz in the startup menu against a 120 Hz panel whose GPU pass
/// costs 0.83 ms and whose event loop is idle 96 % of the time.
///
/// This substitutes the panel period for the ceiling of the **startup timer**
/// only, and changes nothing else: the divisor is C++'s own
/// (`refresh_interval_for_tick`) and the 16 ms logic tick is untouched, so menu
/// animation ages identically. The **game timer keeps the oracle ceiling
/// unconditionally** (`RefreshCeilings`) and must continue to: that is what
/// keeps all four C++-mirrored per-render behaviours — the C4Viewport camera
/// smoother, `C4MessageBoard::Execute` plus the screen fader, flash-message
/// `remaining_draws` and the object-audibility cache — from ever seeing a
/// changed cadence. Subdividing the game timer was measured and rejected: on
/// an M4 Max at Scale=300 fullscreen a 7 ms ceiling moved presentation 35.66 ->
/// 36.30 FPS while the average graphics pass grew 10.49 -> 18.17 ms and
/// automatic frame skips went 2 -> 98, because in game the pass cost and
/// swapchain back-pressure bind long before the timer does. The default
/// therefore stays at the oracle's 30 permanently; an unlogged
/// `DEFAULT_MAX_REFRESH_DELAY_MS = 16` divergence was landed and correctly
/// reverted for exactly that reason, and the faster cadence is reachable only
/// through this key or `Graphics.Remaster`.
pub(crate) fn configured_smooth_presentation(config: &[u8]) -> bool {
    configured_remaster_feature(config, "SmoothPresentation")
}

/// The refresh ceiling actually in force. An explicit `Graphics.MaxRefreshDelay`
/// is always honoured; otherwise smooth presentation substitutes the display
/// period, clamped so it can never be slower than the native default.
pub(crate) fn effective_max_refresh_delay_ms(
    config: &[u8],
    display_refresh_period_ms: Option<u64>,
) -> u64 {
    if crate::native_config_text(config, "Graphics", "MaxRefreshDelay").is_some() {
        return configured_max_refresh_delay_ms(config);
    }
    if !configured_smooth_presentation(config) {
        return DEFAULT_MAX_REFRESH_DELAY_MS;
    }
    // A panel that reports nothing still gets the 60+ FPS presentation the
    // startup timer already assumes; it just does not get 120.
    display_refresh_period_ms
        .unwrap_or(STARTUP_FRAME_INTERVAL.as_millis() as u64)
        .clamp(1, DEFAULT_MAX_REFRESH_DELAY_MS)
}

pub(crate) fn configured_max_refresh_delay_ms(config: &[u8]) -> u64 {
    u64::try_from(startup_config_integer(
        config,
        "Graphics",
        "MaxRefreshDelay",
        DEFAULT_MAX_REFRESH_DELAY_MS as i32,
    ))
    .ok()
    .filter(|delay| *delay > 0)
    .unwrap_or(DEFAULT_MAX_REFRESH_DELAY_MS)
}

pub(crate) fn startup_config_is_corrupted(config: &[u8]) -> bool {
    startup_config_integer(
        config,
        "General",
        "ConfigResetSafety",
        CLASSIC_CONFIG_RESET_SAFETY,
    ) != CLASSIC_CONFIG_RESET_SAFETY
        || startup_config_integer(
            config,
            "Graphics",
            "ResolutionX",
            CLASSIC_DEFAULT_RESOLUTION_X,
        ) == 0
}

fn canonicalize_startup_integrity_scalars(config: &[u8]) -> Result<Option<Vec<u8>>> {
    let mut canonical = config.to_vec();
    for (section, key, default) in [
        ("General", "ConfigResetSafety", CLASSIC_CONFIG_RESET_SAFETY),
        ("Graphics", "ResolutionX", CLASSIC_DEFAULT_RESOLUTION_X),
    ] {
        if clonk_app_netplay::configured_native_scalar(&canonical, section, key).is_none() {
            continue;
        }
        let value = startup_config_integer(&canonical, section, key, default).to_string();
        canonical = clonk_app_netplay::update_configured_native_values(
            &canonical,
            section,
            &[(key, clonk_app_netplay::NativeConfigValue::RawAscii(&value))],
        )
        .with_context(|| format!("failed to canonicalize startup configuration {section}.{key}"))?;
    }
    Ok((canonical != config).then_some(canonical))
}

fn publish_startup_config_in_place(serialized: &[u8], config_path: &Path) -> Result<()> {
    let mut config_file = File::create(config_path).with_context(|| {
        format!(
            "failed to open startup configuration {} for in-place save",
            config_path.display()
        )
    })?;
    config_file.write_all(serialized).with_context(|| {
        format!(
            "failed to save startup configuration {} in place",
            config_path.display()
        )
    })?;
    config_file.sync_all().with_context(|| {
        format!(
            "failed to flush startup configuration {} saved in place",
            config_path.display()
        )
    })
}

fn publish_startup_config_atomically(serialized: &[u8], config_path: &Path) -> Result<()> {
    let file_name = config_path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "config path has no filename"))?
        .to_string_lossy();
    let parent = config_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut staged = match tempfile::Builder::new()
        .prefix(&format!(".{file_name}.startup-config-"))
        .tempfile_in(parent)
    {
        Ok(staged) => staged,
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
            // C4Config::Save opens the existing file directly. Preserve that
            // path when the file is writable but its parent forbids creating
            // a sibling staging file.
            return publish_startup_config_in_place(serialized, config_path);
        }
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "failed to stage startup configuration {}",
                    config_path.display()
                )
            });
        }
    };
    staged.write_all(serialized).with_context(|| {
        format!(
            "failed to stage startup configuration {}",
            config_path.display()
        )
    })?;
    staged.as_file().sync_all().with_context(|| {
        format!(
            "failed to flush startup configuration {}",
            config_path.display()
        )
    })?;
    match staged.persist(config_path) {
        Ok(_) => Ok(()),
        Err(error) if error.error.kind() == io::ErrorKind::PermissionDenied => {
            drop(error.file);
            publish_startup_config_in_place(serialized, config_path)
        }
        Err(error) => Err(error.error).with_context(|| {
            format!(
                "failed to publish startup configuration {}",
                config_path.display()
            )
        }),
    }
}

pub(crate) fn validate_or_repair_startup_config(
    config_path: &Path,
    command_line_config: bool,
) -> Result<bool> {
    let Some(config) = load_startup_integrity_config(config_path)? else {
        return Ok(false);
    };
    if !startup_config_is_corrupted(&config) {
        if let Some(canonical) = canonicalize_startup_integrity_scalars(&config)? {
            publish_startup_config_atomically(&canonical, config_path)?;
        }
        return Ok(false);
    }
    if command_line_config {
        return Err(anyhow!(CUSTOM_CONFIG_CORRUPTED_ERROR));
    }

    tracing::warn!("Configuration corrupted - restoring default!");
    let mut defaults = advanced_config::default_config();
    // The native recovery immediately reloads the saved defaults. Reflect the
    // post-load version adaptation in this file-backed runtime: build 347
    // enables shaders and is then stamped to the current engine build.
    defaults.set_in(Some("General"), "Version", CLASSIC_ENGINE_BUILD.to_string());
    defaults.set_in(Some("Graphics"), "Shader", "1");
    let serialized = defaults
        .to_string()
        .context("failed to serialize restored default configuration")?;
    publish_startup_config_atomically(serialized.as_bytes(), config_path)?;
    let repaired = load_startup_integrity_config(config_path)?.with_context(|| {
        format!(
            "restored default configuration {} disappeared before reload",
            config_path.display()
        )
    })?;
    anyhow::ensure!(
        !startup_config_is_corrupted(&repaired),
        "restored default configuration {} is still corrupt",
        config_path.display()
    );
    Ok(true)
}

pub(crate) fn discover_validated_startup_paths(
    explicit_config: Option<&Path>,
) -> Result<Option<Arc<AppPaths>>> {
    let command_line_config = explicit_config.is_some_and(|path| !path.as_os_str().is_empty());
    let mut app_paths = match cached_app_paths_with_config_file(explicit_config) {
        Ok(paths) => Some(paths),
        Err(_) => {
            // C4Application checks a custom configuration before opening the
            // System group. Preserve that ordering even when broader AppPaths
            // discovery cannot complete yet; LC_CONFIG_FILE still wins the
            // selected file while only a command-line path selects abort.
            let selected_config = std::env::var_os("LC_CONFIG_FILE")
                .filter(|path| !path.is_empty())
                .map(PathBuf::from)
                .or_else(|| explicit_config.map(Path::to_path_buf));
            if let Some(config_path) = selected_config {
                validate_or_repair_startup_config(&config_path, command_line_config)?;
            }
            return Ok(None);
        }
    };
    let repaired_config = app_paths
        .as_ref()
        .map(|paths| validate_or_repair_startup_config(&paths.config_file(), command_line_config))
        .transpose()?
        .unwrap_or(false);
    if repaired_config {
        // AppPaths may already have projected a poisoned General.UserPath.
        // C++ reloads the freshly defaulted config, so rediscover every path
        // from that repaired document before any consumer observes it.
        reset_cached_app_paths();
        app_paths = cached_app_paths_with_config_file(explicit_config).ok();
    }
    Ok(app_paths)
}

/// `C4FullScreen::Init` titles the carrier window with `STD_PRODUCT`
/// (C4FullScreen.cpp:474-480); the developer console is a separate surface with
/// its own caption. Factored so the selection is testable without a window.
pub(crate) fn native_window_title(console: bool) -> &'static str {
    if console {
        clonk_platform::CONSOLE_CAPTION
    } else {
        clonk_platform::ENGINE_CAPTION
    }
}

pub(crate) fn startup_window_attributes(
    display_options: &DisplayOptions,
    initial_size: PhysicalSize<u32>,
) -> winit::window::WindowAttributes {
    let mut attributes = Window::default_attributes()
        .with_title(native_window_title(false))
        // AccessKit refuses to adapt a window that has already been shown, so
        // the carrier opens hidden and `run` reveals it as soon as the
        // accessibility bridge is attached (clonk-org/clonk-rs#392).
        .with_visible(false)
        // Both shells share this builder, matching C++ assigning one icon
        // resource to the fullscreen and console window classes alike
        // (C4FullScreen.cpp:196-211; C4Console.cpp:297-310). Inert on macOS,
        // where winit discards the attribute and the Dock tile comes from the
        // bundle or from `dock_icon`.
        .with_window_icon(crate::window_icon::window_icon())
        .with_inner_size(initial_size);
    // `with_window_icon` fills `ICON_SMALL` only, so the taskbar button would
    // otherwise stretch the title-bar image
    // (winit-0.30.13/src/platform_impl/windows/window.rs:887-908).
    #[cfg(windows)]
    {
        use winit::platform::windows::WindowAttributesExtWindows;
        attributes = attributes.with_taskbar_icon(crate::window_icon::taskbar_icon());
    }
    if matches!(display_options.mode, DisplayMode::Window) && !display_options.maximized {
        if let Some((x, y)) = display_options.position {
            attributes = attributes.with_position(PhysicalPosition::new(x, y));
        }
    }
    if matches!(display_options.mode, DisplayMode::Fullscreen)
        && !defer_startup_fullscreen_until_resumed(display_options.mode)
    {
        attributes = attributes.with_fullscreen(Some(Fullscreen::Borderless(None)));
    }
    attributes
}

pub(crate) fn defer_startup_fullscreen_until_resumed(mode: DisplayMode) -> bool {
    // AppKit only honors the native fullscreen transition after the
    // application has entered its resumed foreground state. Create the first
    // macOS window without fullscreen attributes, then reconcile it from the
    // resumed event to match C++/SDL's desktop fullscreen.
    cfg!(target_os = "macos") && matches!(mode, DisplayMode::Fullscreen)
}

pub(crate) fn should_reconcile_deferred_fullscreen(mode: DisplayMode, is_fullscreen: bool) -> bool {
    defer_startup_fullscreen_until_resumed(mode) && !is_fullscreen
}

pub(crate) const DEFERRED_FULLSCREEN_RETRY_DELAY: Duration = Duration::from_secs(2);

/// `Config.Graphics.Monitor` — "monitor index to play on" (`C4Config.h:162`),
/// stored with the `D3DADAPTER_DEFAULT` default of zero
/// (`C4Config.cpp:483`). The oracle's SDL/GL build never reads the value back,
/// so the row is inert there; honouring it is a presentation-only divergence
/// that cannot reach simulation state. Zero — the default, and every negative
/// value — keeps the pre-existing "whichever monitor the window opened on"
/// behaviour, which is what the default adapter means.
pub(crate) fn configured_startup_monitor(config: &[u8]) -> Option<usize> {
    usize::try_from(startup_config_integer(config, "Graphics", "Monitor", 0))
        .ok()
        .filter(|index| *index > 0)
}

/// The configured monitor out of the ones this session can enumerate.
///
/// An index past the end — a laptop undocked since the key was written, an
/// empty list on a headless session — falls back to `None` so the window still
/// opens borderless on its current monitor rather than failing.
pub(crate) fn select_startup_monitor<M>(
    monitors: impl IntoIterator<Item = M>,
    index: Option<usize>,
) -> Option<M> {
    index.and_then(|index| monitors.into_iter().nth(index))
}

fn configured_startup_monitor_handle(window: &Window) -> Option<winit::monitor::MonitorHandle> {
    select_startup_monitor(
        window.available_monitors(),
        configured_startup_monitor(&load_native_config_bytes(
            cached_app_paths().ok().as_deref(),
        )),
    )
}

pub(crate) fn reconcile_deferred_fullscreen(window: &Window, mode: DisplayMode) -> bool {
    // `startup_window_attributes` runs before a window exists, so this is
    // the first point at which the monitor list is knowable at all.
    let monitor = configured_startup_monitor_handle(window);
    if should_reconcile_deferred_fullscreen(mode, window.fullscreen().is_some()) {
        window.set_fullscreen(Some(Fullscreen::Borderless(monitor)));
        return true;
    }
    // Platforms that went fullscreen straight from the builder landed on
    // whichever monitor the window was placed on. Move them across once.
    if matches!(mode, DisplayMode::Fullscreen)
        && monitor.is_some()
        && window.current_monitor() != monitor
    {
        window.set_fullscreen(Some(Fullscreen::Borderless(monitor)));
    }
    false
}

#[cfg(all(
    test,
    any(not(feature = "app-test-shard-mode"), feature = "app-test-shard-5",),
))]
mod remastered_graphics_stem_tests {
    use super::*;
    use tempfile::tempdir;

    fn selected_name(group: &Group, stem: &str) -> String {
        select_named_graphics_image_source(stem, &[], group)
            .expect("stem resolves")
            .source
            .presentation_filename()
    }

    #[test]
    fn remastered_stem_suffix_wins_over_the_oracle_filename() {
        let install = tempdir().expect("graphics group root");
        let root = install.path().join("Graphics.c4g");
        fs::create_dir_all(&root).expect("graphics group directory");
        for name in ["GUIButton.png", "Control.png", "Control.bmp"] {
            fs::write(root.join(name), b"").expect("seed sheet");
        }
        let group = Group::open(&root).expect("open graphics group");

        // Nothing remastered present: FindSuitableFile's own selection,
        // including its "every later extension replaces an earlier one" rule.
        assert_eq!(selected_name(&group, "GUIButton"), "GUIButton.png");
        assert_eq!(selected_name(&group, "Control"), "Control.png");

        fs::write(root.join("GUIButton@2x.png"), b"").expect("seed 2x sheet");
        let group = Group::open(&root).expect("reopen graphics group");
        assert_eq!(selected_name(&group, "GUIButton"), "GUIButton@2x.png");
        // Untouched stems keep resolving exactly as before.
        assert_eq!(selected_name(&group, "Control"), "Control.png");

        // The most detailed variant wins, whatever order the files appear in.
        fs::write(root.join("GUIButton@4x.png"), b"").expect("seed 4x sheet");
        let group = Group::open(&root).expect("reopen graphics group");
        assert_eq!(selected_name(&group, "GUIButton"), "GUIButton@4x.png");

        // A missing stem still reports the oracle stem, not a suffixed probe.
        let error = select_named_graphics_image_source("Absent", &[], &group)
            .err()
            .map(|error| error.to_string());
        assert_eq!(
            error.as_deref(),
            Some("classic graphics resource `Absent` is unavailable")
        );
    }
}

#[cfg(all(
    test,
    any(not(feature = "app-test-shard-mode"), feature = "app-test-shard-5",),
))]
mod startup_monitor_tests {
    use super::*;

    #[test]
    fn configured_monitor_row_selects_the_nth_enumerated_monitor() {
        // Graphics.Monitor is `int32_t Monitor; // monitor index to play on`
        // defaulted to D3DADAPTER_DEFAULT (C4Config.h:162, C4Config.cpp:483),
        // so zero and below mean "the default adapter" — the pre-existing
        // borderless-current-monitor behaviour.
        let monitors = ["primary", "4k", "tv"];
        for (config, expected) in [
            (&b""[..], None),
            (b"[Graphics]\nMonitor=0\n", None),
            (b"[Graphics]\nMonitor=-1\n", None),
            (b"[Graphics]\nMonitor=1\n", Some("4k")),
            (b"[Graphics]\nMonitor=2\n", Some("tv")),
            // Out of range: the display the window already opened on wins
            // rather than the window failing to go fullscreen.
            (b"[Graphics]\nMonitor=3\n", None),
            (b"[Graphics]\nMonitor=9999\n", None),
        ] {
            assert_eq!(
                select_startup_monitor(monitors, configured_startup_monitor(config)),
                expected,
                "config {}",
                String::from_utf8_lossy(config)
            );
        }
        // An empty monitor list must not panic or select anything.
        assert_eq!(
            select_startup_monitor(
                std::iter::empty::<&str>(),
                configured_startup_monitor(b"[Graphics]\nMonitor=1\n")
            ),
            None
        );
    }
}
