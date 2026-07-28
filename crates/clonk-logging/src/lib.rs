use std::{
    fs::{self, File},
    io::{self, Write},
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
const DEFAULT_DEPENDENCY_FILTER: &str = "wgpu_core::device=warn";
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
    let diagnostics = io::stderr.and(Optional(file.map(Mutex::new)));
    let gui = Optional(capture).and(message_board_sink(game_log));
    tracing_subscriber::registry()
        .with(env_filter(default_level))
        .with(
            fmt::layer()
                .with_writer(diagnostics)
                .with_ansi(false)
                .with_target(false)
                .with_level(true),
        )
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

/// The level prefix `C4LogSystem`'s `LogLevelPrefixFormatterFlag` writes ahead
/// of every GUI line (`src/C4Log.cpp:44-76`).
fn level_prefix(level: Level) -> &'static str {
    match level {
        Level::ERROR => "ERROR: ",
        Level::WARN => "WARNING: ",
        Level::INFO | Level::DEBUG | Level::TRACE => "",
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
    if INITIALIZED.get().is_some() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "the tracing subscriber is already initialized",
        ));
    }

    let default_level = if verbose { "debug" } else { "info" };

    match open_session_log(log_path) {
        Ok(file) => {
            let init_result = install(default_level, Some(file), capture, game_log);
            let _ = INITIALIZED.set(());
            init_result.map_err(|err| {
                io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("failed to install the session-log subscriber: {err}"),
                )
            })
        }
        Err(err) => {
            INITIALIZED.get_or_init(|| {
                let _ = install(default_level, None, capture, game_log);
            });
            Err(err)
        }
    }
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
    INITIALIZED.get_or_init(|| {
        let _ = install(default_level, None, capture, game_log);
    });
}

fn open_session_log(log_path: &Path) -> io::Result<File> {
    if let Some(parent) = log_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    File::create(log_path)
}

fn env_filter(default_level: &'static str) -> EnvFilter {
    let lc_log = std::env::var("LC_LOG").ok();
    let rust_log = std::env::var("RUST_LOG").ok();
    explicit_filter_directive(lc_log.as_deref(), rust_log.as_deref())
        .map(EnvFilter::new)
        .unwrap_or_else(|| EnvFilter::new(format!("{default_level},{DEFAULT_DEPENDENCY_FILTER}")))
}

fn init_with_default_level(default_level: &'static str) {
    INITIALIZED.get_or_init(|| {
        let _ = fmt()
            .with_env_filter(env_filter(default_level))
            .with_target(false)
            .with_level(true)
            .try_init();
    });
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
