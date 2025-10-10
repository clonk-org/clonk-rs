use std::collections::HashSet;
use std::env;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, LineWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::SystemTime;

use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;
use lc_launcher::{
    create_support_bundle, digest_update_telemetry, load_launcher_summary,
    regenerate_support_bundle, timestamp_for_filename, timestamp_for_log, write_launcher_summary,
    LauncherLog, LauncherSummaryRecord, ProviderAutomationRecord, ProviderAutomationState,
    ProviderBulkRetargetRecord, ProviderOverrideSourceRecord, ProviderPathStatus,
    UpdateTelemetrySummary,
};
use lc_platform::AppPaths;

const SKIP_PATCHER_VALIDATION_ENV: &str = "LC_GAME_SKIP_PATCHER_CHECK";
const LEGACY_LOG_PREFIX: &str = "Clonk";
const LEGACY_LOG_SUFFIX: &str = ".log";
const CRASH_ARTIFACT_MARKER: &str = "-crash-";

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

    /// Regenerate a support bundle using the latest launcher summary without starting the runtime
    #[arg(long = "support-bundle-only")]
    support_bundle_only: bool,

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
    let Cli {
        binary,
        support_bundle_only,
        forwarded,
    } = Cli::parse();

    let paths = AppPaths::discover().context("failed to discover application paths")?;
    paths
        .ensure_user_dirs()
        .context("failed to prepare user directories")?;

    let logger = LauncherLogger::new(&paths).context("failed to initialise launcher logging")?;
    logger
        .log_line("launcher initialised")
        .context("failed to write initial log entry")?;

    if support_bundle_only {
        if binary.is_some() {
            bail!("--support-bundle-only cannot be combined with --binary");
        }
        if !forwarded.is_empty() {
            bail!("--support-bundle-only cannot be combined with forwarded runtime arguments");
        }

        let (bundle, telemetry) = regenerate_support_bundle(&paths, &logger, logger.path())
            .context("failed to regenerate support bundle")?;
        print_launcher_report(&paths, Some(bundle.as_path()), &telemetry);
        return Ok(());
    }

    let config_path =
        prepare_config(&paths, &logger).context("failed to prepare configuration file")?;
    logger
        .log_line(&format!("using config file at {}", config_path.display()))
        .context("failed to log config path")?;

    validate_update_tool(&paths, &logger)
        .context("failed to validate updater tool availability")?;

    let binary =
        resolve_runtime_binary(binary.as_deref(), paths.install_root()).with_context(|| {
            format!(
                "unable to locate LegacyClonk binary under {}",
                paths.install_root().display()
            )
        })?;
    logger
        .log_line(&format!("resolved runtime binary {}", binary.display()))
        .context("failed to write binary resolution log")?;

    let runtime_start = SystemTime::now();
    let status = launch_runtime(&binary, &paths, &config_path, &forwarded, &logger)?;
    let log_collection_result = collect_runtime_logs(&paths, runtime_start, &logger);
    let crash_report_result = collect_crash_reports(&paths, runtime_start, &logger);

    if let Err(err) = &log_collection_result {
        logger
            .log_line(&format!("failed to collect runtime logs: {err}"))
            .ok();
    }
    if let Err(err) = &crash_report_result {
        logger
            .log_line(&format!("failed to collect crash artifacts: {err}"))
            .ok();
    }

    let copied_logs = log_collection_result?;
    let crash_reports = crash_report_result?;

    let telemetry_result = digest_update_telemetry(&copied_logs, &logger);
    if let Err(err) = &telemetry_result {
        logger
            .log_line(&format!("failed to extract updater telemetry: {err}"))
            .ok();
    }
    let telemetry_summary = telemetry_result?;

    let bundle_result = create_support_bundle(
        &paths,
        &logger,
        logger.path(),
        &copied_logs,
        &crash_reports,
        &telemetry_summary,
    );
    if let Err(err) = &bundle_result {
        logger
            .log_line(&format!("failed to create support bundle: {err}"))
            .ok();
    }
    let support_bundle = bundle_result?;

    let summary_result = write_launcher_summary(
        &paths,
        &logger,
        logger.path(),
        &copied_logs,
        &crash_reports,
        &telemetry_summary,
        support_bundle.as_deref(),
        None,
        None,
    );
    if let Err(err) = &summary_result {
        logger
            .log_line(&format!("failed to emit launcher summary: {err}"))
            .ok();
    }
    summary_result?;

    print_launcher_report(&paths, support_bundle.as_deref(), &telemetry_summary);

    if status.success() {
        Ok(())
    } else {
        bail!("LegacyClonk exited {}", describe_exit_status(&status));
    }
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
) -> Result<ExitStatus> {
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
        .log_line(&format!(
            "launching {} (forwarding {} args)",
            binary.display(),
            forwarded.len()
        ))
        .context("failed to record launch message")?;

    let mut child = command
        .spawn()
        .with_context(|| format!("failed to launch {}", binary.display()))?;

    let stdout_thread = child
        .stdout
        .take()
        .map(|stdout| spawn_forwarding_thread(stdout, logger.clone(), StreamKind::Stdout));
    let stderr_thread = child
        .stderr
        .take()
        .map(|stderr| spawn_forwarding_thread(stderr, logger.clone(), StreamKind::Stderr));

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

    let exit_summary = describe_exit_status(&status);
    logger
        .log_line(&format!(
            "runtime exited with {} (raw status {})",
            exit_summary, status
        ))
        .context("failed to log runtime status")?;

    Ok(status)
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
                .log_line(&format!("migrated config from {}", candidate.display()))
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
            candidates.push(PathBuf::from(home).join("Library/Preferences/legacyclonk.config"));
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
                    OpenOptions::new().create(true).append(true).open(&log_path)
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
            .log_line(&format!(
                "log file ready at {}",
                logger.inner.path.display()
            ))
            .context("failed to record log file path")?;
        Ok(logger)
    }

    fn log_line(&self, message: &str) -> Result<()> {
        let mut guard = self
            .inner
            .writer
            .lock()
            .map_err(|_| anyhow!("launcher log mutex poisoned"))?;
        writeln!(guard, "[{}] {}", timestamp_for_log(), message)?;
        guard.flush()?;
        Ok(())
    }

    fn log_stream(&self, kind: StreamKind, line: &str) -> Result<()> {
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if trimmed.is_empty() {
            return Ok(());
        }
        self.log_line(&format!("{}: {}", kind.label(), trimmed))
    }

    fn path(&self) -> &Path {
        &self.inner.path
    }
}
impl LauncherLog for LauncherLogger {
    fn log_line(&self, message: &str) -> Result<()> {
        LauncherLogger::log_line(self, message)
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

fn validate_update_tool(paths: &AppPaths, logger: &LauncherLogger) -> Result<()> {
    if env::var_os(SKIP_PATCHER_VALIDATION_ENV).is_some() {
        logger
            .log_line(&format!(
                "skipping patcher validation because {SKIP_PATCHER_VALIDATION_ENV} is set"
            ))
            .context("failed to log patcher validation skip")?;
        return Ok(());
    }

    let install_root = paths.install_root();
    let patcher = locate_update_tool(install_root).with_context(|| {
        format!(
            "c4group update tool not found under {}",
            install_root.display()
        )
    })?;

    logger
        .log_line(&format!("located updater tool at {}", patcher.display()))
        .context("failed to log updater tool path")
}

fn locate_update_tool(install_root: &Path) -> Result<PathBuf> {
    for candidate in candidate_patcher_paths(install_root) {
        if candidate.exists() {
            return Ok(candidate);
        }
    }
    bail!(
        "no c4group updater binary found (expected one of: {})",
        candidate_patcher_paths(install_root)
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
}

fn candidate_patcher_paths(install_root: &Path) -> Vec<PathBuf> {
    const CANDIDATES: &[&str] = &[
        "c4group",
        "c4group.exe",
        "build/c4group",
        "build/c4group.exe",
        "build/Debug/c4group",
        "build/Debug/c4group.exe",
        "build/Release/c4group",
        "build/Release/c4group.exe",
    ];
    CANDIDATES
        .iter()
        .map(|relative| install_root.join(relative))
        .collect()
}

fn collect_runtime_logs(
    paths: &AppPaths,
    started_at: SystemTime,
    logger: &LauncherLogger,
) -> Result<Vec<PathBuf>> {
    let install_root = paths.install_root();
    let logs_dir = paths.logs_dir();

    if install_root == logs_dir {
        logger
            .log_line(
                "skipping runtime log sync because install root and logs directory are identical",
            )
            .context("failed to record runtime log sync skip")?;
        return Ok(Vec::new());
    }

    fs::create_dir_all(logs_dir).with_context(|| {
        format!(
            "failed to ensure logs directory {} exists",
            logs_dir.display()
        )
    })?;

    let copy_stamp = timestamp_for_filename();
    let mut copied = 0usize;
    let mut copies = Vec::new();

    for entry in fs::read_dir(install_root).with_context(|| {
        format!(
            "failed to enumerate install root {} while syncing logs",
            install_root.display()
        )
    })? {
        let entry = entry?;
        let path = entry.path();
        if !is_legacy_log(&path) {
            continue;
        }
        let metadata = entry
            .metadata()
            .with_context(|| format!("failed to stat legacy log {}", path.display()))?;
        if let Ok(modified) = metadata.modified() {
            if modified < started_at {
                continue;
            }
        }

        copied += 1;
        let stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or(LEGACY_LOG_PREFIX);
        let dest_name = format!("{}-{}-{}.log", stem, copy_stamp, copied);
        let dest_path = logs_dir.join(dest_name);
        fs::copy(&path, &dest_path).with_context(|| {
            format!(
                "failed to copy runtime log {} to {}",
                path.display(),
                dest_path.display()
            )
        })?;
        copies.push(dest_path.clone());
        logger
            .log_line(&format!(
                "copied runtime log {} -> {}",
                path.display(),
                dest_path.display()
            ))
            .context("failed to record runtime log copy")?;
    }

    if copied == 0 {
        logger
            .log_line("no runtime logs updated during this session")
            .context("failed to record empty runtime log state")?;
    } else {
        logger
            .log_line(&format!(
                "copied {copied} runtime log(s) into {}",
                logs_dir.display()
            ))
            .context("failed to summarise runtime log copy")?;
    }

    Ok(copies)
}

fn is_legacy_log(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    match path.file_name().and_then(|name| name.to_str()) {
        Some(name) => {
            let lower = name.to_ascii_lowercase();
            lower.starts_with(&LEGACY_LOG_PREFIX.to_ascii_lowercase())
                && lower.ends_with(&LEGACY_LOG_SUFFIX.to_ascii_lowercase())
        }
        None => false,
    }
}

fn is_crash_artifact(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    match path.file_name().and_then(|name| name.to_str()) {
        Some(name) => name.to_ascii_lowercase().contains(CRASH_ARTIFACT_MARKER),
        None => false,
    }
}

fn describe_exit_status(status: &ExitStatus) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(code) = status.code() {
            return if status.success() {
                format!("success (code {code})")
            } else {
                format!("exit code {code}")
            };
        }
        if let Some(signal) = status.signal() {
            let mut description = format!("signal {signal}");
            if status.core_dumped() {
                description.push_str(" (core dumped)");
            }
            return description;
        }
    }
    if let Some(code) = status.code() {
        return if status.success() {
            format!("success (code {code})")
        } else {
            format!("exit code {code}")
        };
    }
    format!("{status}")
}

fn collect_crash_reports(
    paths: &AppPaths,
    started_at: SystemTime,
    logger: &LauncherLogger,
) -> Result<Vec<PathBuf>> {
    let logs_dir = paths.logs_dir();
    fs::create_dir_all(&logs_dir).with_context(|| {
        format!(
            "failed to ensure logs directory {} exists for crash artifacts",
            logs_dir.display()
        )
    })?;

    let mut sources = vec![
        paths.user_data_dir().to_path_buf(),
        paths.install_root().to_path_buf(),
    ];
    if !sources.iter().any(|dir| dir == &logs_dir) {
        sources.push(logs_dir.to_path_buf());
    }

    let mut captured = Vec::new();
    let mut processed = HashSet::new();
    let mut copies = 0usize;
    let stamp = timestamp_for_filename();

    for source in sources {
        if !source.exists() || !source.is_dir() {
            continue;
        }
        for entry in fs::read_dir(&source).with_context(|| {
            format!(
                "failed to enumerate crash artifacts under {}",
                source.display()
            )
        })? {
            let entry = entry?;
            let path = entry.path();
            if !is_crash_artifact(&path) {
                continue;
            }
            if !processed.insert(path.clone()) {
                continue;
            }
            let include = match entry.metadata() {
                Ok(metadata) => match metadata.modified() {
                    Ok(modified) => modified >= started_at,
                    Err(_) => true,
                },
                Err(_) => true,
            };
            if !include {
                continue;
            }

            let dest_path = if path.starts_with(&logs_dir) {
                logger
                    .log_line(&format!(
                        "detected crash artifact already in logs dir: {}",
                        path.display()
                    ))
                    .context("failed to record existing crash artifact")?;
                path.clone()
            } else {
                copies += 1;
                let suffix = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("LegacyClonk-crash.dmp");
                let dest_name = format!("crash-{}-{:02}-{}", stamp, copies, suffix);
                let dest_path = logs_dir.join(dest_name);
                fs::copy(&path, &dest_path).with_context(|| {
                    format!(
                        "failed to copy crash artifact {} to {}",
                        path.display(),
                        dest_path.display()
                    )
                })?;
                logger
                    .log_line(&format!(
                        "copied crash artifact {} -> {}",
                        path.display(),
                        dest_path.display()
                    ))
                    .context("failed to record crash artifact copy")?;
                dest_path
            };
            captured.push(dest_path);
        }
    }

    if captured.is_empty() {
        logger
            .log_line("no crash artifacts generated during this session")
            .context("failed to record crash artifact summary")?;
    } else {
        logger
            .log_line(&format!(
                "captured {} crash artifact(s) in {}",
                captured.len(),
                logs_dir.display()
            ))
            .context("failed to summarise crash artifact capture")?;
    }

    Ok(captured)
}

fn filename_or_display(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| path.display().to_string())
}

fn print_launcher_report(
    paths: &AppPaths,
    support_bundle: Option<&Path>,
    telemetry_summary: &UpdateTelemetrySummary,
) {
    let summary_path = paths.logs_dir().join("launcher-summary.json");
    println!();
    println!("Launcher summary written to {}", summary_path.display());
    match support_bundle {
        Some(path) => println!("Support bundle available at {}", path.display()),
        None => println!("Support bundle was not created; check launcher logs for details."),
    }

    if !telemetry_summary.failures().is_empty() {
        println!("Updater issues detected:");
        for failure in telemetry_summary.failures() {
            println!("  {} -> {}", failure.log_path.display(), failure.message);
        }
    } else if !telemetry_summary.successes().is_empty() {
        let successes = telemetry_summary
            .successes()
            .iter()
            .map(|path| filename_or_display(path))
            .collect::<Vec<_>>()
            .join(", ");
        println!("Updater telemetry success recorded in: {successes}");
    } else {
        println!("Updater telemetry: no signals captured in the collected runtime logs.");
    }

    emit_provider_snapshot(paths);

    match support_bundle {
        Some(_) => println!(
            "Share the support bundle when filing bugs to include launcher, runtime, and telemetry logs."
        ),
        None => println!(
            "Share launcher-summary.json when filing bugs so support can collect the right logs."
        ),
    }
}

fn emit_provider_snapshot(paths: &AppPaths) {
    match load_launcher_summary(paths) {
        Ok(Some(record)) => {
            println!();
            print_provider_sections(&record);
        }
        Ok(None) => {
            println!();
            println!("First-party providers: launcher summary not available yet.");
        }
        Err(err) => {
            println!();
            println!("First-party providers: failed to load launcher summary ({err}).");
        }
    }
}

fn print_provider_sections(record: &LauncherSummaryRecord) {
    let snapshot = &record.summary.provider_automation;
    if snapshot.share.is_empty() && snapshot.upload.is_empty() {
        println!("First-party providers: no automation targets recorded.");
        return;
    }

    println!(
        "First-party providers (logs dir: {}):",
        record.logs_dir.display()
    );

    if !snapshot.share.is_empty() {
        print_provider_category("Share targets", &snapshot.share, &record.logs_dir);
    }
    if !snapshot.upload.is_empty() {
        print_provider_category("Upload targets", &snapshot.upload, &record.logs_dir);
    }
    print_bulk_retarget_sections(record);
}

fn print_provider_category(label: &str, providers: &[ProviderAutomationRecord], logs_dir: &Path) {
    println!("  {label}:");
    for provider in providers {
        print_provider_entry("    ", provider, logs_dir);
    }
}

fn print_provider_entry(indent: &str, provider: &ProviderAutomationRecord, logs_dir: &Path) {
    let current_path = resolve_summary_entry(logs_dir, &provider.path);
    println!(
        "{indent}- {} ({})",
        provider.name,
        describe_provider_path_status(&provider.path_status)
    );
    println!("{indent}  Current path: {}", current_path.display());
    println!(
        "{indent}  Automation: {}",
        describe_provider_automation(&provider.automation)
    );

    let default_entry = provider.default_path.as_deref().unwrap_or(&provider.path);
    let default_path = resolve_summary_entry(logs_dir, default_entry);
    println!("{indent}  Default path: {}", default_path.display());

    if provider.overrides.is_empty() {
        println!("{indent}  Overrides: none recorded.");
    } else {
        println!("{indent}  Overrides:");
        for override_entry in &provider.overrides {
            let path = resolve_summary_entry(logs_dir, &override_entry.path);
            let source = describe_override_source(&override_entry.source);
            println!("{indent}    - {} -> {}", source, path.display());
        }
    }
}

fn print_bulk_retarget_sections(record: &LauncherSummaryRecord) {
    let Some(summary) = record.summary.provider_bulk_retarget.as_ref() else {
        return;
    };
    let has_records = !summary.share.is_empty() || !summary.upload.is_empty();
    if !has_records && summary.history_cleared_at.is_none() {
        return;
    }

    println!("  Bulk retarget history:");
    if has_records {
        print_bulk_retarget_category("Share targets", &summary.share, &record.logs_dir);
        print_bulk_retarget_category("Upload targets", &summary.upload, &record.logs_dir);
    }
    if let Some(cleared_at) = &summary.history_cleared_at {
        if has_records {
            println!("    History last cleared at {cleared_at}.");
        } else {
            println!(
                "    History cleared at {cleared_at}; all providers currently use default staging paths."
            );
        }
    }
}

fn print_bulk_retarget_category(
    label: &str,
    records: &[ProviderBulkRetargetRecord],
    logs_dir: &Path,
) {
    if records.is_empty() {
        return;
    }
    println!("    {label}:");
    for record in records {
        let base_path = resolve_summary_entry(logs_dir, &record.base_path);
        println!(
            "      - {} (last retargeted at {}, changed {} of {} targets)",
            base_path.display(),
            record.retargeted_at,
            record.changed,
            record.total
        );
    }
}

fn resolve_summary_entry(logs_dir: &Path, entry: &str) -> PathBuf {
    let candidate = Path::new(entry);
    if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        logs_dir.join(candidate)
    }
}

fn describe_provider_path_status(status: &ProviderPathStatus) -> String {
    match status {
        ProviderPathStatus::Ready => "ready".into(),
        ProviderPathStatus::Missing => "missing".into(),
        ProviderPathStatus::NotDirectory => "not a directory".into(),
        ProviderPathStatus::Inaccessible(err) => format!("inaccessible ({err})"),
    }
}

fn describe_provider_automation(state: &ProviderAutomationState) -> String {
    match state {
        ProviderAutomationState::Idle => "Idle".into(),
        ProviderAutomationState::Submitted { detail } => format!("Submitted ({detail})"),
        ProviderAutomationState::Stale { reason } => format!("Stale ({reason})"),
        ProviderAutomationState::Skipped { reason } => format!("Skipped ({reason})"),
        ProviderAutomationState::Failed { error } => format!("Failed ({error})"),
    }
}

fn describe_override_source(source: &ProviderOverrideSourceRecord) -> String {
    match source {
        ProviderOverrideSourceRecord::Preference => "Launcher preference".into(),
        ProviderOverrideSourceRecord::Retargeted { applied_at } => {
            format!("Retargeted at {applied_at}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::env;
    use std::fs;
    use std::io::Read;
    use std::path::Path;
    use std::time::{Duration, SystemTime};
    use tempfile::TempDir;
    use zip::ZipArchive;

    struct EnvGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        saved: Vec<(String, Option<std::ffi::OsString>)>,
    }

    impl EnvGuard {
        fn set(vars: &[(&str, Option<&Path>)]) -> Self {
            let lock = env_lock().lock().unwrap();
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

    fn env_lock() -> &'static std::sync::Mutex<()> {
        static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(()))
    }

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

    #[test]
    fn validate_update_tool_finds_primary_binary() {
        let install_dir = TempDir::new().unwrap();
        let planet_dir = install_dir.path().join("planet");
        fs::create_dir_all(&planet_dir).unwrap();
        fs::write(planet_dir.join("System.c4g"), b"stub").unwrap();
        fs::write(install_dir.path().join("c4group"), b"stub").unwrap();

        let user_dir = TempDir::new().unwrap();
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.path())),
        ]);

        let paths = AppPaths::discover().unwrap();
        paths.ensure_user_dirs().unwrap();
        let logger = LauncherLogger::new(&paths).unwrap();

        assert!(validate_update_tool(&paths, &logger).is_ok());
    }

    #[test]
    fn validate_update_tool_reports_missing_binary() {
        let install_dir = TempDir::new().unwrap();
        let planet_dir = install_dir.path().join("planet");
        fs::create_dir_all(&planet_dir).unwrap();
        fs::write(planet_dir.join("System.c4g"), b"stub").unwrap();

        let user_dir = TempDir::new().unwrap();
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.path())),
        ]);

        let paths = AppPaths::discover().unwrap();
        paths.ensure_user_dirs().unwrap();
        let logger = LauncherLogger::new(&paths).unwrap();

        let err = validate_update_tool(&paths, &logger).unwrap_err();
        assert!(
            err.to_string().contains("c4group"),
            "expected error about missing c4group, got {err}"
        );
    }

    #[test]
    fn validate_update_tool_respects_skip_env() {
        let install_dir = TempDir::new().unwrap();
        let planet_dir = install_dir.path().join("planet");
        fs::create_dir_all(&planet_dir).unwrap();
        fs::write(planet_dir.join("System.c4g"), b"stub").unwrap();

        let user_dir = TempDir::new().unwrap();
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.path())),
            (SKIP_PATCHER_VALIDATION_ENV, Some(Path::new("1"))),
        ]);

        let paths = AppPaths::discover().unwrap();
        paths.ensure_user_dirs().unwrap();
        let logger = LauncherLogger::new(&paths).unwrap();

        assert!(validate_update_tool(&paths, &logger).is_ok());
    }

    #[test]
    fn collect_runtime_logs_copies_recent_files() {
        let install_dir = TempDir::new().unwrap();
        let planet_dir = install_dir.path().join("planet");
        fs::create_dir_all(&planet_dir).unwrap();
        fs::write(planet_dir.join("System.c4g"), b"stub").unwrap();

        let user_dir = TempDir::new().unwrap();
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.path())),
        ]);

        let paths = AppPaths::discover().unwrap();
        paths.ensure_user_dirs().unwrap();
        let logger = LauncherLogger::new(&paths).unwrap();

        let start = SystemTime::now();
        fs::write(
            install_dir.path().join("Clonk.log"),
            b"legacy runtime log contents",
        )
        .unwrap();

        let copies = collect_runtime_logs(&paths, start, &logger).unwrap();

        assert_eq!(
            copies.len(),
            1,
            "expected exactly one copied log in {}",
            paths.logs_dir().display()
        );
        assert!(
            copies[0].starts_with(paths.logs_dir()),
            "expected copied log to live in {}",
            paths.logs_dir().display()
        );
        assert!(
            copies[0].exists(),
            "copied log {} should exist",
            copies[0].display()
        );
    }

    #[test]
    fn collect_crash_reports_copies_recent_artifacts() {
        let install_dir = TempDir::new().unwrap();
        let planet_dir = install_dir.path().join("planet");
        fs::create_dir_all(&planet_dir).unwrap();
        fs::write(planet_dir.join("System.c4g"), b"stub").unwrap();

        let user_dir = TempDir::new().unwrap();
        let crash_file = user_dir
            .path()
            .join("LegacyClonk-crash-2024-01-01-00-00-00.dmp");
        fs::write(&crash_file, b"crash").unwrap();

        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.path())),
        ]);

        let paths = AppPaths::discover().unwrap();
        paths.ensure_user_dirs().unwrap();
        let logger = LauncherLogger::new(&paths).unwrap();

        let start = SystemTime::now() - Duration::from_secs(60);
        let artifacts = collect_crash_reports(&paths, start, &logger).unwrap();

        assert!(
            artifacts
                .iter()
                .any(|path| path.starts_with(paths.logs_dir())),
            "expected crash artifact to be copied into logs dir {}",
            paths.logs_dir().display()
        );
        for artifact in artifacts {
            assert!(
                artifact.exists(),
                "expected crash artifact {} to exist",
                artifact.display()
            );
        }
    }

    #[test]
    fn digest_update_telemetry_detects_failure_lines() {
        let install_dir = TempDir::new().unwrap();
        let planet_dir = install_dir.path().join("planet");
        fs::create_dir_all(&planet_dir).unwrap();
        fs::write(planet_dir.join("System.c4g"), b"stub").unwrap();

        let user_dir = TempDir::new().unwrap();
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.path())),
        ]);

        let paths = AppPaths::discover().unwrap();
        paths.ensure_user_dirs().unwrap();
        let logger = LauncherLogger::new(&paths).unwrap();

        let telemetry_log = paths.logs_dir().join("Clonk-session.log");
        fs::write(
            &telemetry_log,
            "c4group returned status 2\nDone.\nRandom other line\n",
        )
        .unwrap();

        let summary = digest_update_telemetry(&[telemetry_log.clone()], &logger).unwrap();
        let failures = summary.failures();
        assert_eq!(failures.len(), 1, "expected one failure event");
        let successes = summary.successes();
        assert_eq!(successes.len(), 1, "expected one success source");
        assert_eq!(
            failures[0].log_path, telemetry_log,
            "failure should reference telemetry log"
        );
        assert_eq!(
            failures[0].message, "c4group returned status 2",
            "failure message should match telemetry line"
        );
        assert_eq!(
            successes[0], telemetry_log,
            "success should reference telemetry log"
        );

        let launcher_log = fs::read_to_string(logger.path()).unwrap();
        assert!(
            launcher_log
                .contains("updater telemetry [Clonk-session.log]: c4group returned status 2"),
            "launcher log should record failure telemetry, contents:\n{launcher_log}"
        );
        assert!(
            launcher_log.contains("updater telemetry: success recorded in Clonk-session.log"),
            "launcher log should record success telemetry, contents:\n{launcher_log}"
        );
    }

    #[test]
    fn create_support_bundle_packages_artifacts() {
        let install_dir = TempDir::new().unwrap();
        let planet_dir = install_dir.path().join("planet");
        fs::create_dir_all(&planet_dir).unwrap();
        fs::write(planet_dir.join("System.c4g"), b"stub").unwrap();

        let user_dir = TempDir::new().unwrap();
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.path())),
        ]);

        let paths = AppPaths::discover().unwrap();
        paths.ensure_user_dirs().unwrap();
        let logger = LauncherLogger::new(&paths).unwrap();
        logger.log_line("bundle integration test start").unwrap();

        let runtime_log = paths.logs_dir().join("Clonk-session.log");
        fs::write(&runtime_log, "runtime log payload").unwrap();
        let crash_log = paths.logs_dir().join("LegacyClonk-crash-2024.dmp");
        fs::write(&crash_log, "crash dump payload").unwrap();

        let mut telemetry = UpdateTelemetrySummary::default();
        telemetry.record_success(runtime_log.clone());
        telemetry.record_failure(runtime_log.clone(), "c4group returned status 0".into());

        let bundle_path = create_support_bundle(
            &paths,
            &logger,
            logger.path(),
            &[runtime_log.clone()],
            &[crash_log.clone()],
            &telemetry,
        )
        .unwrap()
        .expect("bundle path should be returned");

        assert!(
            bundle_path.exists(),
            "support bundle {} should exist",
            bundle_path.display()
        );

        let file = fs::File::open(&bundle_path).unwrap();
        let mut archive = ZipArchive::new(file).unwrap();
        let mut entries = Vec::new();
        for idx in 0..archive.len() {
            let entry = archive.by_index(idx).unwrap();
            entries.push(entry.name().to_string());
        }
        assert!(
            entries.iter().any(|name| name.starts_with("launcher/")),
            "bundle should include launcher log, entries: {entries:?}"
        );
        assert!(
            entries
                .iter()
                .any(|name| name.starts_with("runtime/01_Clonk-session.log")),
            "bundle should include runtime log, entries: {entries:?}"
        );
        assert!(
            entries
                .iter()
                .any(|name| name.starts_with("crash/01_LegacyClonk-crash-2024.dmp")),
            "bundle should include crash artifact, entries: {entries:?}"
        );
        assert!(
            entries.contains(&"telemetry-summary.json".to_string()),
            "bundle should include telemetry summary, entries: {entries:?}"
        );

        let mut telemetry_json = String::new();
        archive
            .by_name("telemetry-summary.json")
            .unwrap()
            .read_to_string(&mut telemetry_json)
            .unwrap();
        let value: Value = serde_json::from_str(&telemetry_json).unwrap();
        assert_eq!(
            value["successes"][0].as_str(),
            Some("Clonk-session.log"),
            "telemetry summary should list runtime log success"
        );
    }

    #[test]
    fn write_launcher_summary_emits_machine_readable_output() {
        let install_dir = TempDir::new().unwrap();
        let planet_dir = install_dir.path().join("planet");
        fs::create_dir_all(&planet_dir).unwrap();
        fs::write(planet_dir.join("System.c4g"), b"stub").unwrap();

        let user_dir = TempDir::new().unwrap();
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.path())),
        ]);

        let paths = AppPaths::discover().unwrap();
        paths.ensure_user_dirs().unwrap();
        let logger = LauncherLogger::new(&paths).unwrap();
        logger.log_line("summary integration test start").unwrap();

        let runtime_log = paths.logs_dir().join("Clonk-summary.log");
        fs::write(&runtime_log, "summary runtime log").unwrap();
        let crash_log = paths.logs_dir().join("LegacyClonk-crash-summary.dmp");
        fs::write(&crash_log, "summary crash dump").unwrap();
        let bundle_path = paths.logs_dir().join("support-bundle-test.zip");
        fs::write(&bundle_path, b"bundle").unwrap();

        let mut telemetry = UpdateTelemetrySummary::default();
        telemetry.record_success(runtime_log.clone());
        telemetry.record_failure(runtime_log.clone(), "c4group returned status 1".into());

        write_launcher_summary(
            &paths,
            &logger,
            logger.path(),
            &[runtime_log.clone()],
            &[crash_log.clone()],
            &telemetry,
            Some(&bundle_path),
            None,
            None,
        )
        .unwrap();

        let summary_path = paths.logs_dir().join("launcher-summary.json");
        assert!(
            summary_path.exists(),
            "summary file {} should exist",
            summary_path.display()
        );

        let contents = fs::read_to_string(&summary_path).unwrap();
        let value: Value = serde_json::from_str(&contents).unwrap();

        assert_eq!(
            value["schema_version"].as_u64(),
            Some(1),
            "summary should include schema version"
        );
        assert!(
            value["launcher_log"]
                .as_str()
                .unwrap()
                .starts_with("lc-game"),
            "summary should include launcher log path"
        );
        assert_eq!(
            value["runtime_logs"][0].as_str(),
            Some("Clonk-summary.log"),
            "summary should include runtime log relative path"
        );
        assert_eq!(
            value["crash_reports"][0].as_str(),
            Some("LegacyClonk-crash-summary.dmp"),
            "summary should include crash artifact relative path"
        );
        assert_eq!(
            value["support_bundle"].as_str(),
            Some("support-bundle-test.zip"),
            "summary should reference support bundle"
        );
        assert!(
            value["update_telemetry"]["successes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|entry| entry.as_str() == Some("Clonk-summary.log")),
            "summary should record telemetry success: {value}"
        );
        assert!(
            value["update_telemetry"]["failures"]
                .as_array()
                .unwrap()
                .iter()
                .any(|entry| entry["message"].as_str() == Some("c4group returned status 1")),
            "summary should record telemetry failure: {value}"
        );
    }

    #[test]
    fn regenerate_support_bundle_requires_existing_summary() {
        let install_dir = TempDir::new().unwrap();
        let planet_dir = install_dir.path().join("planet");
        fs::create_dir_all(&planet_dir).unwrap();
        fs::write(planet_dir.join("System.c4g"), b"stub").unwrap();

        let user_dir = TempDir::new().unwrap();
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.path())),
        ]);

        let paths = AppPaths::discover().unwrap();
        paths.ensure_user_dirs().unwrap();
        let logger = LauncherLogger::new(&paths).unwrap();

        let result = regenerate_support_bundle(&paths, &logger, logger.path());
        assert!(
            result.is_err(),
            "regenerating a support bundle without a summary should fail"
        );
    }

    #[test]
    fn regenerate_support_bundle_uses_launcher_summary() {
        let install_dir = TempDir::new().unwrap();
        let planet_dir = install_dir.path().join("planet");
        fs::create_dir_all(&planet_dir).unwrap();
        fs::write(planet_dir.join("System.c4g"), b"stub").unwrap();

        let user_dir = TempDir::new().unwrap();
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.path())),
        ]);

        let paths = AppPaths::discover().unwrap();
        paths.ensure_user_dirs().unwrap();
        let logger = LauncherLogger::new(&paths).unwrap();
        logger
            .log_line("regeneration integration test start")
            .unwrap();

        let runtime_log = paths.logs_dir().join("Clonk-regenerate.log");
        fs::write(&runtime_log, "Done.\n").unwrap();
        let crash_log = paths.logs_dir().join("LegacyClonk-crash-regenerate.dmp");
        fs::write(&crash_log, "crash dump payload").unwrap();

        let mut telemetry = UpdateTelemetrySummary::default();
        telemetry.record_success(runtime_log.clone());
        telemetry.record_failure(runtime_log.clone(), "c4group returned status 1".into());

        write_launcher_summary(
            &paths,
            &logger,
            logger.path(),
            &[runtime_log.clone()],
            &[crash_log.clone()],
            &telemetry,
            None,
            None,
            None,
        )
        .unwrap();

        let (bundle_path, regenerated) =
            regenerate_support_bundle(&paths, &logger, logger.path()).unwrap();
        assert!(
            bundle_path.exists(),
            "regenerated bundle {} should exist",
            bundle_path.display()
        );
        assert!(
            bundle_path.starts_with(paths.logs_dir()),
            "bundle {} should be created inside logs dir {}",
            bundle_path.display(),
            paths.logs_dir().display()
        );
        assert_eq!(
            regenerated.successes().len(),
            1,
            "telemetry should report one success"
        );
        assert_eq!(
            regenerated.failures().len(),
            1,
            "telemetry should report one failure"
        );

        let summary_path = paths.logs_dir().join("launcher-summary.json");
        let summary_text = fs::read_to_string(&summary_path).unwrap();
        let document: Value = serde_json::from_str(&summary_text).unwrap();
        let recorded_bundle = document["support_bundle"]
            .as_str()
            .expect("support bundle entry should exist");
        let expected_relative = bundle_path
            .strip_prefix(paths.logs_dir())
            .map(|relative| relative.display().to_string())
            .unwrap_or_else(|_| bundle_path.display().to_string());
        assert_eq!(
            recorded_bundle, expected_relative,
            "summary should point at regenerated bundle"
        );

        let file = fs::File::open(&bundle_path).unwrap();
        let mut archive = ZipArchive::new(file).unwrap();
        let mut entries = Vec::new();
        for idx in 0..archive.len() {
            let entry = archive.by_index(idx).unwrap();
            entries.push(entry.name().to_string());
        }
        assert!(
            entries.iter().any(|name| name.starts_with("launcher/")),
            "bundle should include launcher log entries: {entries:?}"
        );
        assert!(
            entries.iter().any(|name| name.starts_with("runtime/")),
            "bundle should include runtime logs entries: {entries:?}"
        );
        assert!(
            entries.iter().any(|name| name.starts_with("crash/")),
            "bundle should include crash artifacts entries: {entries:?}"
        );
        assert!(
            entries.iter().any(|name| name == "telemetry-summary.json"),
            "bundle should include telemetry summary entries: {entries:?}"
        );
    }
}
