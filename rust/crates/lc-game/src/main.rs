use std::env;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, LineWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;
use lc_platform::AppPaths;

#[derive(Debug, Parser)]
#[command(
    name = "lc-game",
    about = "LegacyClonk Rust launcher that forwards to the C++ runtime",
    version,
    author
)]
struct Cli {
    /// Override the detected LegacyClonk binary location
    #[arg(long = "binary", value_name = "PATH")]
    binary: Option<PathBuf>,

    /// Arguments forwarded verbatim to the LegacyClonk runtime
    #[arg(trailing_var_arg = true)]
    forwarded: Vec<OsString>,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("lc-game: {error:?}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();

    let paths = AppPaths::discover().context("failed to discover application paths")?;
    paths
        .ensure_user_dirs()
        .context("failed to prepare user directories")?;

    let logger = LauncherLogger::new(&paths).context("failed to initialise launcher logging")?;
    logger
        .log_line("launcher initialised")
        .context("failed to write initial log entry")?;

    let config_path =
        prepare_config(&paths, &logger).context("failed to prepare configuration file")?;
    logger
        .log_line(format!(
            "using config file at {}",
            config_path.display()
        ))
        .context("failed to log config path")?;

    let binary =
        resolve_runtime_binary(cli.binary.as_deref(), paths.install_root()).with_context(|| {
            format!(
                "unable to locate LegacyClonk binary under {}",
                paths.install_root().display()
            )
        })?;
    logger
        .log_line(format!(
            "resolved runtime binary {}",
            binary.display()
        ))
        .context("failed to write binary resolution log")?;

    launch_runtime(&binary, &paths, &config_path, &cli.forwarded, &logger)
}

fn resolve_runtime_binary(override_path: Option<&Path>, install_root: &Path) -> Result<PathBuf> {
    if let Some(path) = override_path {
        if path.exists() {
            return Ok(path.to_path_buf());
        }
        bail!("binary override at {} does not exist", path.display());
    }

    if let Some(env_path) = std::env::var_os("LC_GAME_BINARY") {
        let candidate = PathBuf::from(&env_path);
        if candidate.exists() {
            return Ok(candidate);
        }
        bail!(
            "LC_GAME_BINARY points to {} but the file was not found",
            PathBuf::from(env_path).display()
        );
    }

    for candidate in candidate_binaries(install_root) {
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    bail!(
        "could not locate the LegacyClonk runtime under {} (set --binary or LC_GAME_BINARY)",
        install_root.display()
    );
}

fn launch_runtime(
    binary: &Path,
    paths: &AppPaths,
    config_path: &Path,
    forwarded: &[OsString],
    logger: &LauncherLogger,
) -> Result<()> {
    let mut command = Command::new(binary);
    command.current_dir(paths.install_root());
    command.args(forwarded);

    command.env("LC_INSTALL_ROOT", paths.install_root());
    command.env("LC_APP_ROOT", paths.install_root());
    command.env("LC_USER_DATA_DIR", paths.user_data_dir());
    command.env("LC_CACHE_DIR", paths.cache_dir());
    command.env("LC_LOGS_DIR", paths.logs_dir());
    command.env("LC_TEMP_DIR", paths.temp_dir());
    command.env("LC_CONFIG_DIR", paths.config_dir());
    command.env("LC_CONFIG_FILE", config_path);

    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    logger
        .log_line(format!(
            "launching {} (forwarding {} args)",
            binary.display(),
            forwarded.len()
        ))
        .context("failed to record launch message")?;

    let mut child = command
        .spawn()
        .with_context(|| format!("failed to launch {}", binary.display()))?;

    let stdout_thread = child.stdout.take().map(|stdout| {
        spawn_forwarding_thread(
            stdout,
            logger.clone(),
            StreamKind::Stdout,
        )
    });
    let stderr_thread = child.stderr.take().map(|stderr| {
        spawn_forwarding_thread(
            stderr,
            logger.clone(),
            StreamKind::Stderr,
        )
    });

    let status = child
        .wait()
        .with_context(|| format!("failed to wait for {}", binary.display()))?;

    if let Some(handle) = stdout_thread {
        handle
            .join()
            .map_err(|err| anyhow!("stdout forwarding thread panicked: {:?}", err))??
    }
    if let Some(handle) = stderr_thread {
        handle
            .join()
            .map_err(|err| anyhow!("stderr forwarding thread panicked: {:?}", err))??
    }

    logger
        .log_line(format!("runtime exited with status {}", status))
        .context("failed to log runtime status")?;

    if status.success() {
        Ok(())
    } else {
        bail!("LegacyClonk exited with status {status}");
    }
}

fn candidate_binaries(install_root: &Path) -> Vec<PathBuf> {
    const CANDIDATES: &[&str] = &[
        "clonk",
        "clonk.exe",
        "build/clonk",
        "build/clonk.exe",
        "build/Debug/clonk",
        "build/Debug/clonk.exe",
        "build/Release/clonk",
        "build/Release/clonk.exe",
        "build/clonk.app/Contents/MacOS/clonk",
        "build/Debug/clonk.app/Contents/MacOS/clonk",
        "build/Release/clonk.app/Contents/MacOS/clonk",
    ];
    CANDIDATES
        .iter()
        .map(|rel| install_root.join(rel))
        .collect()
}

fn prepare_config(paths: &AppPaths, logger: &LauncherLogger) -> Result<PathBuf> {
    let config_path = paths.config_file();
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    if config_path.exists() {
        return Ok(config_path);
    }

    let mut migrated = false;
    for candidate in legacy_config_candidates() {
        if candidate.exists() {
            fs::copy(&candidate, &config_path).with_context(|| {
                format!(
                    "failed to migrate config from {} to {}",
                    candidate.display(),
                    config_path.display()
                )
            })?;
            logger
                .log_line(format!(
                    "migrated config from {}",
                    candidate.display()
                ))
                .context("failed to log config migration")?;
            migrated = true;
            break;
        }
    }

    if !migrated {
        File::create(&config_path)
            .with_context(|| format!("failed to create {}", config_path.display()))?;
        logger
            .log_line("created new empty config file")
            .context("failed to log config creation")?;
    }

    Ok(config_path)
}

fn legacy_config_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Some(explicit) = env::var_os("LC_LEGACY_CONFIG_FILE") {
        candidates.push(PathBuf::from(explicit));
    }

    #[cfg(target_os = "macos")]
    {
        if let Some(home) = env::var_os("HOME") {
            candidates.push(
                PathBuf::from(home).join("Library/Preferences/legacyclonk.config"),
            );
        }
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some(home) = env::var_os("HOME") {
            candidates.push(PathBuf::from(home).join(".legacyclonk/config"));
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(app_data) = env::var_os("APPDATA") {
            let base = PathBuf::from(app_data).join("LegacyClonk");
            candidates.push(base.join("LegacyClonk.cfg"));
            candidates.push(base.join("config"));
        }
    }

    candidates
}

#[derive(Clone)]
struct LauncherLogger {
    inner: Arc<LauncherLoggerInner>,
}

struct LauncherLoggerInner {
    writer: Mutex<LineWriter<File>>,
    path: PathBuf,
}

impl LauncherLogger {
    fn new(paths: &AppPaths) -> Result<Self> {
        let logs_dir = paths.logs_dir();
        fs::create_dir_all(logs_dir)
            .with_context(|| format!("failed to create {}", logs_dir.display()))?;

        let log_path = logs_dir.join(format!("lc-game-{}.log", timestamp_for_filename()));
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&log_path)
            .or_else(|err| {
                if err.kind() == io::ErrorKind::AlreadyExists {
                    OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&log_path)
                } else {
                    Err(err)
                }
            })
            .with_context(|| format!("failed to open log file {}", log_path.display()))?;

        let logger = Self {
            inner: Arc::new(LauncherLoggerInner {
                writer: Mutex::new(LineWriter::new(file)),
                path: log_path,
            }),
        };
        logger
            .log_line("launcher log started")
            .context("failed to write launcher header")?;
        logger
            .log_line(format!(
                "log file ready at {}",
                logger.inner.path.display()
            ))
            .context("failed to record log file path")?;
        Ok(logger)
    }

    fn log_line(&self, message: impl AsRef<str>) -> Result<()> {
        let mut guard = self
            .inner
            .writer
            .lock()
            .map_err(|_| anyhow!("launcher log mutex poisoned"))?;
        writeln!(guard, "[{}] {}", timestamp_for_log(), message.as_ref())?;
        guard.flush()?;
        Ok(())
    }

    fn log_stream(&self, kind: StreamKind, line: &str) -> Result<()> {
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if trimmed.is_empty() {
            return Ok(());
        }
        self.log_line(format!("{}: {}", kind.label(), trimmed))
    }

}

#[derive(Clone, Copy)]
enum StreamKind {
    Stdout,
    Stderr,
}

impl StreamKind {
    fn label(self) -> &'static str {
        match self {
            StreamKind::Stdout => "stdout",
            StreamKind::Stderr => "stderr",
        }
    }

    fn print(self, text: &str) {
        match self {
            StreamKind::Stdout => {
                print!("{}", text);
                let _ = io::stdout().flush();
            }
            StreamKind::Stderr => {
                eprint!("{}", text);
                let _ = io::stderr().flush();
            }
        }
    }
}

fn spawn_forwarding_thread<R>(
    reader: R,
    logger: LauncherLogger,
    kind: StreamKind,
) -> thread::JoinHandle<Result<()>>
where
    R: io::Read + Send + 'static,
{
    thread::spawn(move || {
        let mut buf_reader = BufReader::new(reader);
        let mut buffer = Vec::new();
        loop {
            buffer.clear();
            let bytes = buf_reader.read_until(b'\n', &mut buffer)?;
            if bytes == 0 {
                break;
            }
            let text = String::from_utf8_lossy(&buffer);
            kind.print(&text);
            logger
                .log_stream(kind, &text)
                .context("failed to log runtime output")?;
        }
        Ok(())
    })
}

fn timestamp_for_filename() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

fn timestamp_for_log() -> String {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => format!(
            "{}.{:03}",
            duration.as_secs(),
            duration.subsec_millis()
        ),
        Err(_) => "0.000".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn candidate_list_covers_macos_bundle() {
        let temp = TempDir::new().unwrap();
        let app_dir = temp
            .path()
            .join("build")
            .join("clonk.app")
            .join("Contents")
            .join("MacOS");
        fs::create_dir_all(&app_dir).unwrap();
        let binary = app_dir.join("clonk");
        fs::write(&binary, b"stub").unwrap();
        let resolved =
            resolve_runtime_binary(None, temp.path()).expect("should locate bundle binary");
        assert_eq!(resolved, binary);
    }

    #[test]
    fn respects_override_argument() {
        let temp = TempDir::new().unwrap();
        let override_bin = temp.path().join("custom").join("clonk");
        fs::create_dir_all(override_bin.parent().unwrap()).unwrap();
        fs::write(&override_bin, b"stub").unwrap();
        let result = resolve_runtime_binary(Some(&override_bin), temp.path()).unwrap();
        assert_eq!(result, override_bin);
    }

    #[test]
    fn rejects_missing_override() {
        let temp = TempDir::new().unwrap();
        let override_bin = temp.path().join("missing");
        let err = resolve_runtime_binary(Some(&override_bin), temp.path()).unwrap_err();
        assert!(
            err.to_string().contains("does not exist"),
            "unexpected message: {err}"
        );
    }
}
