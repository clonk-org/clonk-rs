use std::{
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
pub use clonk_core::log_target::SCRIPT_LOG_TARGET;

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
fn message_board_sink(
    game_log: Option<GameLogCapture>,
) -> impl for<'a> MakeWriter<'a> + Send + Sync + 'static {
    Optional(game_log).with_filter(|meta: &Metadata<'_>| meta.target() == SCRIPT_LOG_TARGET)
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
    let gui = Optional(capture).and(message_board_sink(game_log));
    tracing_subscriber::registry()
        .with(env_filter(default_level))
        .with(
            fmt::layer()
                .with_writer(io::stderr)
                // Colour helps a developer reading a terminal and corrupts
                // every other consumer, so it follows the stream itself.
                .with_ansi(io::stderr().is_terminal())
                .with_target(false)
                .with_level(true),
        )
        .with(file.map(|file| {
            fmt::layer()
                .with_writer(Mutex::new(file))
                .with_ansi(false)
                .with_target(false)
                .with_level(true)
        }))
        .with(fmt::layer().with_writer(gui).event_format(GuiSinkFormat))
        .try_init()
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
        Ok(file) => install(default_level, Some(file), capture, game_log).map_err(|err| {
            io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("failed to install the session-log subscriber: {err}"),
            )
        }),
        Err(err) => {
            let _ = install(default_level, None, capture, game_log);
            Err(err)
        }
    }
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

fn env_filter(default_level: &'static str) -> EnvFilter {
    let lc_log = std::env::var("LC_LOG").ok();
    let rust_log = std::env::var("RUST_LOG").ok();
    let requested =
        explicit_filter_directive(lc_log.as_deref(), rust_log.as_deref()).unwrap_or(default_level);
    EnvFilter::new(format!("{DEFAULT_DEPENDENCY_FILTER},{requested}"))
}

fn init_with_default_level(default_level: &'static str) {
    if claim_initialization().is_ok() {
        let _ = install(default_level, None, None, None);
    }
}

#[doc(hidden)]
pub fn select_filter_directive<'a>(
    lc_log: Option<&'a str>,
    rust_log: Option<&'a str>,
    default_level: &'a str,
) -> &'a str {
    explicit_filter_directive(lc_log, rust_log).unwrap_or(default_level)
}

fn explicit_filter_directive<'a>(
    lc_log: Option<&'a str>,
    rust_log: Option<&'a str>,
) -> Option<&'a str> {
    lc_log
        .or(rust_log)
        .filter(|directive| EnvFilter::try_new(directive).is_ok())
}
