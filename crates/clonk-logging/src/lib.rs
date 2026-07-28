use std::{
    fs::{self, File},
    io::{self, Write},
    path::Path,
    sync::{Arc, Mutex, OnceLock},
};

use tracing::Metadata;
use tracing_subscriber::fmt::writer::{MakeWriter, MakeWriterExt, OptionalWriter};
use tracing_subscriber::{fmt, EnvFilter};

static INITIALIZED: OnceLock<()> = OnceLock::new();
const DEFAULT_DEPENDENCY_FILTER: &str = "wgpu_core::device=warn";
/// Target of the C4Script `Log()`/`DebugLog()` stream. It is the Rust
/// counterpart of the C++ logger whose output `C4LogSystem::GuiSink` shows
/// in-game (`src/C4Log.cpp:226-240`).
pub const SCRIPT_LOG_TARGET: &str = "clonk-script";

/// Process-local copy of formatted log output consumed by the developer
/// console. The capture is intentionally independent from the bounded GUI
/// model: tracing may write from worker threads, while the window drains it
/// on the application thread.
#[derive(Clone, Debug, Default)]
pub struct ConsoleLogCapture {
    bytes: Arc<Mutex<Vec<u8>>>,
}

impl ConsoleLogCapture {
    /// Remove and return every byte written since the previous drain.
    pub fn take(&self) -> String {
        let mut bytes = self
            .bytes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let drained = std::mem::take(&mut *bytes);
        format_console_log(&String::from_utf8_lossy(&drained))
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

/// The GuiSink's message-board attachment: it receives the C4Script logger and
/// nothing else, so engine-internal Rust tracing — which has no C++ `Log()`
/// counterpart — never reaches `C4MessageBoard::AddLog`.
#[derive(Clone, Debug, Default)]
struct ScriptLogSink(Option<GameLogCapture>);

impl<'a> MakeWriter<'a> for ScriptLogSink {
    type Writer = OptionalWriter<ConsoleLogWriter>;

    fn make_writer(&'a self) -> Self::Writer {
        OptionalWriter::none()
    }

    fn make_writer_for(&'a self, meta: &Metadata<'_>) -> Self::Writer {
        self.0
            .as_ref()
            .filter(|_| meta.target() == SCRIPT_LOG_TARGET)
            .map(|capture| OptionalWriter::some(capture.make_writer()))
            .unwrap_or_else(OptionalWriter::none)
    }
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

fn format_console_log(raw: &str) -> String {
    let mut formatted = String::new();
    for line in raw.lines() {
        let (prefix, message) = [
            (" ERROR ", "ERROR: "),
            (" WARN ", "WARNING: "),
            (" INFO ", ""),
            (" DEBUG ", ""),
            (" TRACE ", ""),
        ]
        .into_iter()
        .find_map(|(marker, prefix)| {
            line.find(marker)
                .map(|position| (prefix, &line[position + marker.len()..]))
        })
        .unwrap_or(("", line));
        formatted.push_str(prefix);
        formatted.push_str(message);
        formatted.push('\n');
    }
    formatted
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

    let file = open_session_log(log_path);
    let default_level = if verbose { "debug" } else { "info" };
    let board = ScriptLogSink(game_log);

    match file {
        Ok(file) => {
            let init_result = if let Some(capture) = capture {
                fmt()
                    .with_env_filter(env_filter(default_level))
                    .with_writer(io::stderr.and(Mutex::new(file)).and(capture).and(board))
                    .with_ansi(false)
                    .with_target(false)
                    .with_level(true)
                    .try_init()
            } else {
                fmt()
                    .with_env_filter(env_filter(default_level))
                    .with_writer(io::stderr.and(Mutex::new(file)).and(board))
                    .with_ansi(false)
                    .with_target(false)
                    .with_level(true)
                    .try_init()
            };
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
                if let Some(capture) = capture {
                    let _ = fmt()
                        .with_env_filter(env_filter(default_level))
                        .with_writer(io::stderr.and(capture).and(board))
                        .with_ansi(false)
                        .with_target(false)
                        .with_level(true)
                        .try_init();
                } else {
                    let _ = fmt()
                        .with_env_filter(env_filter(default_level))
                        .with_writer(io::stderr.and(board))
                        .with_ansi(false)
                        .with_target(false)
                        .with_level(true)
                        .try_init();
                }
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
    let board = ScriptLogSink(game_log);
    let default_level = if verbose { "debug" } else { "info" };
    INITIALIZED.get_or_init(|| {
        if let Some(capture) = capture {
            let _ = fmt()
                .with_env_filter(env_filter(default_level))
                .with_writer(io::stderr.and(capture).and(board))
                .with_ansi(false)
                .with_target(false)
                .with_level(true)
                .try_init();
        } else {
            let _ = fmt()
                .with_env_filter(env_filter(default_level))
                .with_writer(io::stderr.and(board))
                .with_ansi(false)
                .with_target(false)
                .with_level(true)
                .try_init();
        }
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
