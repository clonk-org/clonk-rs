use std::{
    fs::{self, File},
    io,
    path::Path,
    sync::{Mutex, OnceLock},
};

use tracing_subscriber::fmt::writer::MakeWriterExt;
use tracing_subscriber::{fmt, EnvFilter};

static INITIALIZED: OnceLock<()> = OnceLock::new();

/// Initialise the global tracing subscriber used across the LegacyClonk binaries.
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
    if INITIALIZED.get().is_some() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "the tracing subscriber is already initialized",
        ));
    }

    let file = open_session_log(log_path);
    let default_level = if verbose { "debug" } else { "info" };

    match file {
        Ok(file) => {
            let writer = io::stderr.and(Mutex::new(file));
            let init_result = fmt()
                .with_env_filter(env_filter(default_level))
                .with_writer(writer)
                .with_ansi(false)
                .with_target(false)
                .with_level(true)
                .try_init();
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
                let _ = fmt()
                    .with_env_filter(env_filter(default_level))
                    .with_writer(io::stderr)
                    .with_target(false)
                    .with_level(true)
                    .try_init();
            });
            Err(err)
        }
    }
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
    let directive = select_filter_directive(lc_log.as_deref(), rust_log.as_deref(), default_level);
    EnvFilter::new(directive)
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
    lc_log
        .or(rust_log)
        .filter(|directive| EnvFilter::try_new(directive).is_ok())
        .unwrap_or(default_level)
}
