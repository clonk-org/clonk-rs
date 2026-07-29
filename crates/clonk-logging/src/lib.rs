use std::{
    collections::VecDeque,
    fs::{self, File},
    io::{self, IsTerminal, Write},
    path::Path,
    sync::{Arc, Mutex, OnceLock},
};

use tracing::{Event, Level, Metadata, Subscriber};
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::writer::{MakeWriter, MakeWriterExt, OptionalWriter};
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::{SubscriberInitExt, TryInitError};
use tracing_subscriber::{fmt, EnvFilter};

static INITIALIZED: OnceLock<()> = OnceLock::new();
/// Graphics, windowing and event-loop crates log per-frame detail below `warn`
/// that would bury our own output. This is applied ahead of any user directive
/// so a more specific one — `LC_LOG=info,wgpu_hal=debug` — still wins.
const DEFAULT_DEPENDENCY_FILTER: &str =
    "wgpu=warn,wgpu_core=warn,wgpu_hal=warn,naga=warn,winit=warn,calloop=warn,mio=warn";
/// Target of the panic hook. Deliberately not the script target: a Rust panic
/// is not content output and has no business on the in-game message board.
const PANIC_LOG_TARGET: &str = "panic";
/// Target of the C4Script `Log()`/`DebugLog()` stream. It is the Rust
/// counterpart of the C++ logger whose output `C4LogSystem::GuiSink` shows
/// in-game (`src/C4Log.cpp:226-240`). Re-exported from `clonk-core` so the
/// engine crates that emit and this crate that routes share one definition.
pub use clonk_core::log_target::{SCRIPT_DEBUG_LOG_TARGET, SCRIPT_LOG_TARGET};

/// Process-local copy of formatted log output consumed by the developer
/// console. The capture is intentionally independent from the bounded GUI
/// model: tracing may write from worker threads, while the window drains it
/// on the application thread.
#[derive(Clone, Debug, Default)]
pub struct ConsoleLogCapture {
    bytes: Arc<Mutex<Vec<u8>>>,
}

impl ConsoleLogCapture {
    /// Remove and return every byte written since the previous drain. The
    /// GuiSink formatter has already projected each line, so this is a plain
    /// drain — nothing re-reads the text.
    pub fn take(&self) -> String {
        let mut bytes = self
            .bytes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let drained = std::mem::take(&mut *bytes);
        String::from_utf8_lossy(&drained).into_owned()
    }
}

/// Message-board destination of `C4LogSystem::GuiSink`
/// (`src/C4Log.cpp:226-240`): every line the C4Script logger emits reaches
/// `C4MessageBoard::AddLog`, which drops empty messages
/// (`src/C4MessageBoard.cpp:327-347`). The application drains this on its own
/// thread, mirroring the sink's `ExecuteInMainThread` marshalling.
#[derive(Clone, Debug, Default)]
pub struct GameLogCapture {
    inner: ConsoleLogCapture,
}

impl GameLogCapture {
    /// Remove and return every line logged since the previous drain, formatted
    /// with the GUI sink's `%*%v` pattern (`src/C4Log.cpp:44-83,187-204`).
    pub fn take(&self) -> Vec<String> {
        self.inner
            .take()
            .lines()
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect()
    }
}

impl<'a> MakeWriter<'a> for GameLogCapture {
    type Writer = ConsoleLogWriter;

    fn make_writer(&'a self) -> Self::Writer {
        self.inner.make_writer()
    }
}

/// A destination that may be absent. Wrapping each optional sink lets one
/// writer chain serve every binary instead of one builder per combination.
#[derive(Clone, Debug, Default)]
struct Optional<M>(Option<M>);

impl<'a, M> MakeWriter<'a> for Optional<M>
where
    M: MakeWriter<'a>,
{
    type Writer = OptionalWriter<M::Writer>;

    fn make_writer(&'a self) -> Self::Writer {
        self.0
            .as_ref()
            .map(|make| OptionalWriter::some(make.make_writer()))
            .unwrap_or_else(OptionalWriter::none)
    }

    fn make_writer_for(&'a self, meta: &Metadata<'_>) -> Self::Writer {
        self.0
            .as_ref()
            .map(|make| OptionalWriter::some(make.make_writer_for(meta)))
            .unwrap_or_else(OptionalWriter::none)
    }
}

/// The GuiSink's message-board attachment: it receives the C4Script logger and
/// nothing else, so engine-internal Rust tracing — which has no C++ `Log()`
/// counterpart — never reaches `C4MessageBoard::AddLog`
/// (`src/C4Log.cpp:226-240`).
/// The loading screen's attachment: it takes the same events the message
/// board does, so the loader shows what C++ shows, and retains them only while
/// the loading screen is up.
fn loader_log_sink() -> impl for<'a> MakeWriter<'a> + Send + Sync + 'static {
    LoaderLogCapture.with_filter(|meta: &Metadata<'_>| {
        loader_log_is_active() && debug_log_reaches_gui(meta.target())
    })
}

fn message_board_sink(
    game_log: Option<GameLogCapture>,
) -> impl for<'a> MakeWriter<'a> + Send + Sync + 'static {
    Optional(game_log).with_filter(|meta: &Metadata<'_>| debug_log_reaches_gui(meta.target()))
}

/// Whether an event on `target` belongs in a GUI sink right now.
///
/// `Log()` output always does. `DebugLog()` output does only while the round has
/// debug mode enabled, so a verbose or `RUST_LOG`-driven session cannot leak
/// debug-only lines into the message board in rounds that disabled it. The file
/// sink is deliberately not consulted here — it keeps these diagnostics
/// unconditionally, which is the point of routing them separately.
pub fn debug_log_reaches_gui(target: &str) -> bool {
    match target {
        SCRIPT_LOG_TARGET => true,
        SCRIPT_DEBUG_LOG_TARGET => clonk_core::log_target::debug_mode_presentation(),
        _ => false,
    }
}

/// Install the process-wide subscriber. Every event fans out to stderr and the
/// session log; the developer console and the message board attach only when
/// the application opened them.
fn install(
    default_level: &'static str,
    file: Option<File>,
    capture: Option<ConsoleLogCapture>,
    game_log: Option<GameLogCapture>,
) -> Result<(), TryInitError> {
    let gui = Optional(capture)
        .and(message_board_sink(game_log))
        .and(loader_log_sink());
    let (filter, rejected_directives) = env_filter(default_level);
    let installed = tracing_subscriber::registry()
        .with(filter)
        .with(
            fmt::layer()
                // The registry admits `DebugLog` unconditionally for the file
                // sink's sake; stderr still honours the requested verbosity.
                .with_writer(io::stderr.with_filter(move |meta: &Metadata<'_>| {
                    meta.target() != SCRIPT_DEBUG_LOG_TARGET
                        || debug_log_reaches_stderr(default_level)
                }))
                // Colour helps a developer reading a terminal and corrupts
                // every other consumer, so it follows the stream itself.
                .with_ansi(io::stderr().is_terminal())
                .with_target(true)
                .with_level(true),
        )
        .with(file.map(|file| {
            fmt::layer()
                .with_writer(Mutex::new(file))
                .with_ansi(false)
                .with_target(true)
                .with_level(true)
        }))
        .with(fmt::layer().with_writer(gui).event_format(GuiSinkFormat))
        .try_init();
    // Only reportable once a subscriber exists; before this point every event
    // is discarded without a trace.
    if installed.is_ok() && !rejected_directives.is_empty() {
        tracing::warn!(
            directives = %rejected_directives.join(","),
            "ignored unparseable log filter directives"
        );
    }
    installed
}

pub struct ConsoleLogWriter {
    bytes: Arc<Mutex<Vec<u8>>>,
    pending: Vec<u8>,
}

impl Write for ConsoleLogWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.pending.extend_from_slice(buf);
        Ok(buf.len())
    }

    /// Commit the buffered event to the shared capture. Buffering until flush
    /// keeps a single event's bytes contiguous when tracing writes it from a
    /// worker thread while the application drains on its own.
    fn flush(&mut self) -> io::Result<()> {
        let pending = std::mem::take(&mut self.pending);
        self.bytes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .extend_from_slice(&pending);
        Ok(())
    }
}

impl Drop for ConsoleLogWriter {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}

impl<'a> MakeWriter<'a> for ConsoleLogCapture {
    type Writer = ConsoleLogWriter;

    fn make_writer(&'a self) -> Self::Writer {
        ConsoleLogWriter {
            bytes: Arc::clone(&self.bytes),
            pending: Vec::new(),
        }
    }
}

/// Bounded ordered buffer behind the graphical loading screen's log box.
///
/// `C4MessageBoard::Init` hands the loader a startup log buffer that
/// `C4LoaderScreen::Draw` renders above the progress bar
/// (`src/C4MessageBoard.cpp:223-251`; `src/C4LoaderScreen.cpp:126-177`), and
/// `C4LogSystem`'s GUI sink feeds it every log event from any thread
/// (`src/C4Log.cpp:208-243`). Both the tracing sink and the loader's own phase
/// milestones append here, so one mutex orders worker-thread log events against
/// main-thread progress updates instead of one source replacing the other.
static LOADER_LOG: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());

/// Whether the loading screen is up. The sink is always attached — activation
/// decides whether it retains anything, so no subscriber is rebuilt per round.
static LOADER_LOG_ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Lines retained for the loader's log box before the oldest is dropped.
pub const LOADER_LOG_CAPACITY: usize = 1_000;

/// Starts capturing into the loader log, discarding anything a previous round
/// left behind.
pub fn activate_loader_log() {
    loader_log().clear();
    LOADER_LOG_ACTIVE.store(true, std::sync::atomic::Ordering::SeqCst);
}

/// Stops capturing and releases the buffer when the loading screen closes.
pub fn deactivate_loader_log() {
    LOADER_LOG_ACTIVE.store(false, std::sync::atomic::Ordering::SeqCst);
    loader_log().clear();
}

/// Whether the loading screen is currently capturing.
pub fn loader_log_is_active() -> bool {
    LOADER_LOG_ACTIVE.load(std::sync::atomic::Ordering::SeqCst)
}

fn loader_log() -> std::sync::MutexGuard<'static, VecDeque<String>> {
    LOADER_LOG
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Appends one already-formatted line, dropping the oldest past capacity.
/// Ignored while the loader is not up.
pub fn push_loader_log_line(line: &str) {
    if !loader_log_is_active() || line.is_empty() {
        return;
    }
    let mut log = loader_log();
    if log.len() == LOADER_LOG_CAPACITY {
        log.pop_front();
    }
    log.push_back(line.to_owned());
}

/// The retained lines, oldest first — the order `C4LoaderScreen` draws.
pub fn loader_log_snapshot() -> Vec<String> {
    loader_log().iter().cloned().collect()
}

/// Sink that feeds tracing events into the loader log in `GuiSinkFormat` form.
#[derive(Clone, Debug, Default)]
struct LoaderLogCapture;

/// Buffers one event so a worker thread's line stays contiguous, then splits it
/// into the loader's line-oriented buffer on flush.
pub struct LoaderLogWriter {
    pending: Vec<u8>,
}

impl Write for LoaderLogWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.pending.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        let pending = std::mem::take(&mut self.pending);
        String::from_utf8_lossy(&pending)
            .lines()
            .for_each(push_loader_log_line);
        Ok(())
    }
}

impl Drop for LoaderLogWriter {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}

impl<'a> MakeWriter<'a> for LoaderLogCapture {
    type Writer = LoaderLogWriter;

    fn make_writer(&'a self) -> Self::Writer {
        LoaderLogWriter {
            pending: Vec::new(),
        }
    }
}

/// The severity marker shown ahead of a GUI line. These sinks have no room for
/// a level column, so the marker is the only thing separating a diagnostic from
/// the content text around it; every level except `INFO` therefore carries one.
fn level_prefix(level: Level) -> &'static str {
    match level {
        Level::ERROR => "ERROR: ",
        Level::WARN => "WARNING: ",
        Level::INFO => "",
        Level::DEBUG => "DEBUG: ",
        Level::TRACE => "TRACE: ",
    }
}

/// `C4LogSystem::GuiSink` sets the pattern `"%*%v"` (`src/C4Log.cpp:185-200`):
/// the level prefix followed by the message payload alone. There is no
/// timestamp, no level token, and no span context in a line C++ shows in-game,
/// so the prefix is projected from the record's own level here rather than
/// recovered by scanning already-formatted text — message bodies are content
/// strings and may themselves contain the word `ERROR`.
struct GuiSinkFormat;

impl<S, N> FormatEvent<S, N> for GuiSinkFormat
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> std::fmt::Result {
        writer.write_str(level_prefix(*event.metadata().level()))?;
        ctx.field_format().format_fields(writer.by_ref(), event)?;
        writeln!(writer)
    }
}

/// Initialise the global tracing subscriber used across the Clonk Rust binaries.
///
/// Accepts filter directives from the `LC_LOG` environment variable, falling back to `RUST_LOG`
/// and ultimately to `info` level logging when no directives are provided.
pub fn init() {
    init_with_default_level("info");
}

/// Initialise tracing, defaulting to `debug` when `verbose` is true.
///
/// Explicit `LC_LOG` and `RUST_LOG` directives keep the same precedence as [`init`].
pub fn init_verbose(verbose: bool) {
    init_with_default_level(if verbose { "debug" } else { "info" });
}

/// Initialise tracing for a game session, writing the same events to stderr and `log_path`.
///
/// The session log is truncated at startup. If its parent directory or file cannot be opened,
/// stderr logging is still installed and the I/O error is returned to the caller. Explicit
/// `LC_LOG` and `RUST_LOG` directives keep the same precedence as [`init`]. Calling this after a
/// subscriber has already been initialized returns [`io::ErrorKind::AlreadyExists`].
pub fn init_verbose_with_file(verbose: bool, log_path: &Path) -> io::Result<()> {
    init_verbose_with_file_and_capture(verbose, log_path, None, None)
}

/// Raw descriptor of the active session log, for signal-handler use only.
/// `-1` means "no log yet", matching C++'s `GetLogFD` sentinel.
#[cfg(unix)]
static CRASH_LOG_DESCRIPTOR: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);

/// Duplicates the session log's descriptor so the crash handler keeps a stable
/// one even if the tracing writer is later dropped.
#[cfg(unix)]
fn publish_crash_log_descriptor(file: &File) {
    use std::os::fd::AsRawFd;
    extern "C" {
        fn dup(fildes: i32) -> i32;
    }
    // SAFETY: `file` is a live, open descriptor for the duration of this call.
    let duplicated = unsafe { dup(file.as_raw_fd()) };
    if duplicated >= 0 {
        CRASH_LOG_DESCRIPTOR.store(duplicated, std::sync::atomic::Ordering::Release);
    }
}

/// The active session log's descriptor, or `-1` when there is none. Only a
/// signal handler should use this; everything else goes through `tracing`.
#[cfg(unix)]
pub fn crash_log_descriptor() -> i32 {
    CRASH_LOG_DESCRIPTOR.load(std::sync::atomic::Ordering::Acquire)
}

/// Initialise session logging, mirror the formatted stream into the
/// developer-console log pane when one is open, and always feed the C4Script
/// log stream to the in-game message board.
pub fn init_verbose_with_file_and_capture(
    verbose: bool,
    log_path: &Path,
    capture: Option<ConsoleLogCapture>,
    game_log: Option<GameLogCapture>,
) -> io::Result<()> {
    claim_initialization()?;
    let default_level = if verbose { "debug" } else { "info" };

    match open_session_log(log_path) {
        Ok(file) => {
            // The session log lives behind a buffered tracing writer, which a
            // signal handler must not touch. Publish the raw descriptor so the
            // crash banner can `write(2)` to it the way `GetLogFD` does
            // (C4WinMain.cpp:199-209).
            #[cfg(unix)]
            publish_crash_log_descriptor(&file);
            install(default_level, Some(file), capture, game_log).map_err(|err| {
                io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("failed to install the session-log subscriber: {err}"),
                )
            })
        }
        Err(err) => {
            let _ = install(default_level, None, capture, game_log);
            Err(err)
        }
    }
}

/// Record the build and platform a session ran on, as its first log line.
///
/// A log without this is not self-diagnosing: the two version numbers diverge
/// deliberately, and almost every triage question starts with which build on
/// which platform. Emitting it once, immediately after install, means every
/// attached log answers that before the first real event.
pub fn log_startup_banner(port_version: &str, engine_version: &str) {
    tracing::info!(
        port_version,
        engine_version,
        os = std::env::consts::OS,
        arch = std::env::consts::ARCH,
        debug_assertions = cfg!(debug_assertions),
        "starting clonk"
    );
}

/// Route panics through the log before the process dies.
///
/// The default hook writes straight to stderr, which bypasses the session log
/// and, in a windowed build, goes nowhere at all — so the one event that ends
/// the session is missing from the file a user attaches to a bug report. The
/// backtrace is force-captured because end users never set `RUST_BACKTRACE`.
/// The previous hook still runs, so the usual stderr message is unaffected.
///
/// Call this after initializing logging; the hook applies to every thread.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| info.payload().downcast_ref::<String>().map(String::as_str))
            .unwrap_or("<non-string panic payload>");
        tracing::error!(
            target: PANIC_LOG_TARGET,
            thread = std::thread::current().name().unwrap_or("<unnamed>"),
            location = %info.location().map_or_else(
                || "<unknown>".to_string(),
                std::panic::Location::to_string,
            ),
            payload,
            backtrace = %std::backtrace::Backtrace::force_capture(),
            "the process panicked"
        );
        previous(info);
    }));
}

/// Claim the one-time initialization slot, refusing when any subscriber — ours
/// or a foreign one, as a test harness or embedder installs — is already in
/// place. Claiming before opening the session log keeps a doomed install from
/// rotating and truncating the file first.
fn claim_initialization() -> io::Result<()> {
    let already_installed = tracing::dispatcher::has_been_set();
    INITIALIZED
        .set(())
        .ok()
        .filter(|()| !already_installed)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::AlreadyExists,
                "the tracing subscriber is already initialized",
            )
        })
}

/// Initialise stderr logging, mirror it into the developer console when one is
/// open, and keep feeding the in-game message board when no file-backed
/// application paths are available.
pub fn init_verbose_with_capture(
    verbose: bool,
    capture: Option<ConsoleLogCapture>,
    game_log: Option<GameLogCapture>,
) {
    let default_level = if verbose { "debug" } else { "info" };
    if claim_initialization().is_ok() {
        let _ = install(default_level, None, capture, game_log);
    }
}

/// Open this session's log, first setting the previous one aside. A bug report
/// is filed after a relaunch, so the run worth reading is usually the one that
/// just ended; renaming keeps exactly one generation of it. A missing previous
/// log is the ordinary first-run case, not a failure.
fn open_session_log(log_path: &Path) -> io::Result<File> {
    if let Some(parent) = log_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    let previous = log_path.with_extension("previous.log");
    fs::rename(log_path, previous).or_else(|err| {
        (err.kind() == io::ErrorKind::NotFound)
            .then_some(())
            .ok_or(err)
    })?;
    File::create(log_path)
}

/// The `[Logging]` directive published by [`set_logging_config_directive`],
/// applied when neither `LC_LOG` nor `RUST_LOG` is set.
static LOGGING_CONFIG_DIRECTIVE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();

/// Publishes the directive built from the shared config's `[Logging]` section.
/// Must be called before logging is initialized; later calls are ignored, so a
/// second shell cannot silently retune an installed subscriber.
pub fn set_logging_config_directive(directive: Option<String>) {
    let _ = LOGGING_CONFIG_DIRECTIVE.set(directive);
}

fn configured_logging_directive() -> Option<String> {
    LOGGING_CONFIG_DIRECTIVE.get().cloned().flatten()
}

fn env_filter(default_level: &str) -> (EnvFilter, Vec<String>) {
    let lc_log = std::env::var("LC_LOG").ok();
    let rust_log = std::env::var("RUST_LOG").ok();
    // `[Logging]` sits below the explicit environment filters and above the
    // bare `--verbose` default, so a shared config tunes verbosity without
    // overriding an operator's `LC_LOG`.
    let configured = configured_logging_directive();
    let (requested, rejected) = resolve_filter_directive(
        lc_log.as_deref(),
        rust_log.as_deref().or(configured.as_deref()),
        default_level,
    );
    // `DebugLog` diagnostics always reach the registry so the session log can
    // keep them; the stderr layer re-applies the operator's verbosity below,
    // and the GUI sink applies the round's debug mode.
    (
        EnvFilter::new(format!(
            "{DEFAULT_DEPENDENCY_FILTER},{requested},{SCRIPT_DEBUG_LOG_TARGET}=debug"
        )),
        rejected,
    )
}

/// Whether `default_level` is verbose enough that `DebugLog` output belongs on
/// stderr. The session log keeps it either way.
pub fn debug_log_reaches_stderr(default_level: &str) -> bool {
    matches!(default_level, "debug" | "trace")
}

fn init_with_default_level(default_level: &'static str) {
    if claim_initialization().is_ok() {
        let _ = install(default_level, None, None, None);
    }
}

/// `C4ConfigLogging`'s components, in `CompileFunc` order
/// (C4Config.cpp:703-714). Each is an INI section under `[Logging]` holding a
/// `LogLevel` key, and maps onto the tracing target the port's equivalent
/// subsystem emits under.
///
/// Logging is judged on best practice rather than C4Log parity, so this is a
/// name mapping for the *shared configuration file*, not a claim that the two
/// log streams match line for line.
pub const LOGGING_COMPONENTS: &[(&str, &str)] = &[
    ("AudioSystem", "clonk_audio"),
    ("AulExec", clonk_core::log_target::SCRIPT_LOG_TARGET),
    (
        "AulProfiler",
        clonk_core::log_target::SCRIPT_PROFILER_TARGET,
    ),
    ("DDraw", "clonk_graphics"),
    ("GameControl", "clonk_engine"),
    ("Network", "clonk_network"),
    ("Network2IO", "clonk_network::session"),
    ("Network2HTTPClient", "clonk_network::league"),
    ("Network2UPnP", "clonk_network::upnp"),
    ("Playback", "clonk_engine::record"),
    ("PNGFile", "clonk_resources"),
];

/// `spdlog` level names as `C4ConfigLogging` writes them, mapped onto the
/// tracing levels `EnvFilter` accepts. `off` and spdlog's `critical` have
/// direct equivalents; anything else is refused so a typo cannot silently
/// change verbosity.
pub fn tracing_level_for_spdlog_name(name: &str) -> Option<&'static str> {
    Some(match name.trim().to_ascii_lowercase().as_str() {
        "trace" => "trace",
        "debug" => "debug",
        "info" => "info",
        "warn" | "warning" => "warn",
        "err" | "error" => "error",
        // spdlog's most severe level has no tracing counterpart above error.
        "critical" => "error",
        "off" => "off",
        _ => return None,
    })
}

/// Builds an `EnvFilter` directive from `[Logging]`: the global stdout level
/// followed by one `target=level` directive per configured component. Returns
/// `None` when the section configures nothing, so the caller keeps its existing
/// default rather than pinning a level the user did not ask for.
pub fn logging_config_directive(
    stdout_level: Option<&str>,
    component_levels: &[(&str, &str)],
) -> Option<String> {
    let mut directives = Vec::new();
    if let Some(level) = stdout_level.and_then(tracing_level_for_spdlog_name) {
        directives.push(level.to_string());
    }
    for (component, level) in component_levels {
        let Some(target) = LOGGING_COMPONENTS
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(component))
            .map(|(_, target)| *target)
        else {
            continue;
        };
        if let Some(level) = tracing_level_for_spdlog_name(level) {
            directives.push(format!("{target}={level}"));
        }
    }
    (!directives.is_empty()).then(|| directives.join(","))
}

/// Resolve `LC_LOG`/`RUST_LOG` into the directive to install, along with any
/// comma-separated parts that had to be dropped.
///
/// A directive is kept part by part rather than all-or-nothing: rejecting the
/// whole string over one typo moves the level towards the default in whichever
/// direction that happens to lie, so asking to go quieter can make the log
/// louder. An empty or whitespace-only value counts as unset, which is how
/// shell wrappers export a variable they did not set.
#[doc(hidden)]
pub fn resolve_filter_directive(
    lc_log: Option<&str>,
    rust_log: Option<&str>,
    default_level: &str,
) -> (String, Vec<String>) {
    let (accepted, rejected): (Vec<&str>, Vec<&str>) = lc_log
        .or(rust_log)
        .map(str::trim)
        .filter(|directive| !directive.is_empty())
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .partition(|part| EnvFilter::try_new(*part).is_ok());
    let directive = if accepted.is_empty() {
        default_level.to_string()
    } else {
        accepted.join(",")
    };
    (
        directive,
        rejected.into_iter().map(str::to_string).collect(),
    )
}
