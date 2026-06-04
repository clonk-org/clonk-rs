use std::collections::HashSet;
use std::env;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, LineWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::SystemTime;

#[cfg(unix)]
use std::os::unix::fs::symlink;

use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;
use lc_core::std_config::Config;
use lc_launcher::{
    append_support_bundle_report, build_support_bundle_report, create_support_bundle,
    digest_update_telemetry, regenerate_support_bundle, timestamp_for_filename, timestamp_for_log,
    write_launcher_summary, LauncherLog, ReportSearchTriageSummary, SupportBundleReport,
    UpdateTelemetrySummary,
};
use lc_platform::AppPaths;
use serde::Serialize;

const SKIP_PATCHER_VALIDATION_ENV: &str = "LC_GAME_SKIP_PATCHER_CHECK";
const FORCE_WINDOW_ENV: &str = "LC_GAME_FORCE_WINDOW";
const FORCE_FULLSCREEN_ENV: &str = "LC_GAME_FORCE_FULLSCREEN";
const DISABLE_HEADLESS_GUARD_ENV: &str = "LC_GAME_DISABLE_HEADLESS_GUARD";
const LEGACY_LOG_PREFIX: &str = "Clonk";
const LEGACY_LOG_SUFFIX: &str = ".log";
const CRASH_ARTIFACT_MARKER: &str = "-crash-";

#[derive(Debug, Parser)]
#[command(
    name = "lc-game",
    about = "LegacyClonk Rust launcher that runs the Rust runtime",
    version,
    author
)]
struct Cli {
    /// Override the detected LegacyClonk Rust runtime binary location
    #[arg(long = "binary", value_name = "PATH")]
    binary: Option<PathBuf>,

    /// Regenerate a support bundle using the latest launcher summary without starting the runtime
    #[arg(long = "support-bundle-only")]
    support_bundle_only: bool,

    /// Emit launcher diagnostics as JSON for automation consumers
    #[arg(long = "automation-report")]
    automation_report: bool,

    /// Arguments forwarded verbatim to the LegacyClonk runtime
    #[arg(trailing_var_arg = true)]
    forwarded: Vec<OsString>,
}

fn main() {
    lc_core::logging::init();

    if let Err(error) = run() {
        tracing::error!(error = ?error, "lc-game encountered an error");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let Cli {
        binary,
        support_bundle_only,
        automation_report,
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
        let report = build_support_bundle_report(&paths, Some(bundle.as_path()), &telemetry);
        emit_launcher_report(
            &paths,
            Some(bundle.as_path()),
            &telemetry,
            &report,
            automation_report,
        )?;
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
                "unable to locate Rust runtime binary under {}",
                paths.install_root().display()
            )
        })?;
    logger
        .log_line(&format!("resolved runtime binary {}", binary.display()))
        .context("failed to write binary resolution log")?;

    ensure_runtime_assets(&paths, &binary, &logger)
        .context("failed to prepare runtime assets for launch")?;

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
        None,
    );
    if let Err(err) = &summary_result {
        logger
            .log_line(&format!("failed to emit launcher summary: {err}"))
            .ok();
    }
    summary_result?;

    if let Some(bundle_path) = support_bundle.as_ref() {
        if let Err(err) = append_support_bundle_report(&paths, bundle_path, &telemetry_summary) {
            logger
                .log_line(&format!("failed to embed support bundle report: {err}"))
                .ok();
        }
    }

    let report = build_support_bundle_report(&paths, support_bundle.as_deref(), &telemetry_summary);
    emit_launcher_report(
        &paths,
        support_bundle.as_deref(),
        &telemetry_summary,
        &report,
        automation_report,
    )?;

    if status.success() {
        Ok(())
    } else {
        bail!("LegacyClonk exited {}", describe_exit_status(&status));
    }
}

fn ensure_runtime_assets(paths: &AppPaths, binary: &Path, logger: &LauncherLogger) -> Result<()> {
    let binary_dir = binary.parent().ok_or_else(|| {
        anyhow!(
            "resolved runtime binary {} does not have a parent directory",
            binary.display()
        )
    })?;

    let mut target_roots = vec![paths.install_root().to_path_buf()];
    if !target_roots.iter().any(|root| root == binary_dir) {
        target_roots.push(binary_dir.to_path_buf());
    }

    #[cfg(target_os = "macos")]
    {
        if binary_dir.file_name().and_then(|name| name.to_str()) == Some("MacOS") {
            if let Some(contents_dir) = binary_dir.parent() {
                if contents_dir.file_name().and_then(|name| name.to_str()) == Some("Contents") {
                    if let Some(app_dir) = contents_dir.parent() {
                        if app_dir
                            .extension()
                            .and_then(|ext| ext.to_str())
                            .is_some_and(|ext| ext.eq_ignore_ascii_case("app"))
                        {
                            if let Some(bundle_root) = app_dir.parent() {
                                if !target_roots.iter().any(|root| root == bundle_root) {
                                    target_roots.push(bundle_root.to_path_buf());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    for root in target_roots {
        ensure_runtime_asset(
            &paths.planet_dir().join("System.c4g"),
            &root.join("System.c4g"),
            "System.c4g",
            logger,
        )?;
        ensure_runtime_asset(
            &paths.planet_dir().join("Graphics.c4g"),
            &root.join("Graphics.c4g"),
            "Graphics.c4g",
            logger,
        )?;
    }

    Ok(())
}

fn ensure_runtime_asset(
    source: &Path,
    target: &Path,
    label: &str,
    logger: &LauncherLogger,
) -> Result<()> {
    if !source.exists() {
        bail!(
            "required runtime asset {label} missing at {}",
            source.display()
        );
    }

    if target.exists() {
        if same_file(source, target)? {
            return Ok(());
        }
        let target_meta = fs::symlink_metadata(target).with_context(|| {
            format!(
                "failed to inspect existing runtime asset {}",
                target.display()
            )
        })?;
        if target_meta.is_dir() {
            fs::remove_dir_all(target).with_context(|| {
                format!("failed to remove stale directory {}", target.display())
            })?;
        } else {
            fs::remove_file(target)
                .with_context(|| format!("failed to remove stale {}", target.display()))?;
        }
        logger
            .log_line(&format!("removed stale {label} at {}", target.display()))
            .context("failed to log stale asset removal")?;
    }

    match fs::hard_link(source, target) {
        Ok(_) => {
            logger
                .log_line(&format!("hard linked {label} into {}", target.display()))
                .context("failed to log hard link creation")?;
            return Ok(());
        }
        Err(link_err) => {
            #[cfg(unix)]
            {
                match symlink(source, target) {
                    Ok(_) => {
                        logger
                            .log_line(&format!(
                                "symlinked {label} into {} after hard link failed: {link_err}",
                                target.display()
                            ))
                            .context("failed to log symlink creation")?;
                        return Ok(());
                    }
                    Err(symlink_err) => {
                        logger
                            .log_line(&format!(
                                "failed to link {label}: hard link error {link_err}; \
                                 symlink error {symlink_err}; copying instead"
                            ))
                            .context("failed to log link fallback")?;
                    }
                }
            }
            #[cfg(not(unix))]
            {
                logger
                    .log_line(&format!(
                        "failed to hard link {label}: {link_err}; copying instead"
                    ))
                    .context("failed to log link fallback")?;
            }
        }
    }

    fs::copy(source, target).with_context(|| {
        format!(
            "failed to copy {label} from {} to {}",
            source.display(),
            target.display()
        )
    })?;
    logger
        .log_line(&format!("copied {label} into {}", target.display()))
        .context("failed to log asset copy")?;
    Ok(())
}

fn same_file(a: &Path, b: &Path) -> io::Result<bool> {
    let a_meta = fs::metadata(a)?;
    let b_meta = fs::metadata(b)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(a_meta.ino() == b_meta.ino() && a_meta.dev() == b_meta.dev())
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        return Ok(a_meta.file_index() == b_meta.file_index()
            && a_meta.volume_serial_number() == b_meta.volume_serial_number());
    }
    #[cfg(not(any(unix, windows)))]
    {
        return Ok(a_meta.len() == b_meta.len());
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
        "could not locate the Rust runtime binary under {} (set --binary or LC_GAME_BINARY)",
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

    let runtime_output = RuntimeOutputCollector::new();

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

    let stdout_thread = child.stdout.take().map(|stdout| {
        spawn_forwarding_thread(
            stdout,
            logger.clone(),
            Some(runtime_output.clone()),
            StreamKind::Stdout,
        )
    });
    let stderr_thread = child.stderr.take().map(|stderr| {
        spawn_forwarding_thread(
            stderr,
            logger.clone(),
            Some(runtime_output.clone()),
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

    runtime_output
        .persist(paths.install_root(), logger)
        .context("failed to persist captured runtime output")?;

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
    const RUST_RUNTIME_CANDIDATES: &[&str] = &[
        "lc-app",
        "lc-app.exe",
        "bin/lc-app",
        "bin/lc-app.exe",
        "lc-app.app/Contents/MacOS/lc-app",
        "bin/lc-app.app/Contents/MacOS/lc-app",
        "build/lc-app",
        "build/lc-app.exe",
        "build/Debug/lc-app",
        "build/Debug/lc-app.exe",
        "build/Release/lc-app",
        "build/Release/lc-app.exe",
        "build/lc-app.app/Contents/MacOS/lc-app",
        "build/Debug/lc-app.app/Contents/MacOS/lc-app",
        "build/Release/lc-app.app/Contents/MacOS/lc-app",
        "rust/target/debug/lc-app",
        "rust/target/debug/lc-app.exe",
        "rust/target/release/lc-app",
        "rust/target/release/lc-app.exe",
        "rust_port/target/debug/lc-app",
        "rust_port/target/debug/lc-app.exe",
        "rust_port/target/release/lc-app",
        "rust_port/target/release/lc-app.exe",
    ];

    let mut seen = HashSet::new();
    let mut candidates = Vec::new();

    if let Ok(current_exe) = env::current_exe() {
        for path in sibling_runtime_candidates(&current_exe) {
            if seen.insert(path.clone()) {
                candidates.push(path);
            }
        }
    }

    for relative in RUST_RUNTIME_CANDIDATES {
        let candidate = install_root.join(relative);
        if seen.insert(candidate.clone()) {
            candidates.push(candidate);
        }
    }

    candidates
}

fn sibling_runtime_candidates(exe: &Path) -> Vec<PathBuf> {
    let mut results = Vec::new();
    if let Some(dir) = exe.parent() {
        results.push(dir.join("lc-app"));
        results.push(dir.join("lc-app.exe"));
        results.push(
            dir.join("lc-app.app")
                .join("Contents")
                .join("MacOS")
                .join("lc-app"),
        );
    }
    if let Some(bundle_dir) = exe
        .parent()
        .and_then(|dir| dir.parent())
        .and_then(|contents| contents.parent())
    {
        // Handles lc-game inside a .app bundle by looking for a sibling lc-app bundle.
        results.push(
            bundle_dir
                .join("lc-app.app")
                .join("Contents")
                .join("MacOS")
                .join("lc-app"),
        );
    }
    results
}

fn prepare_config(paths: &AppPaths, logger: &LauncherLogger) -> Result<PathBuf> {
    if let Some(override_path) = config_override_path() {
        if override_path.is_dir() {
            bail!(
                "LC_CONFIG_FILE points to a directory: {}",
                override_path.display()
            );
        }
        if let Some(parent) = override_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let existed = override_path.exists();
        if !existed {
            File::create(&override_path).with_context(|| {
                format!(
                    "failed to create config override at {}",
                    override_path.display()
                )
            })?;
            logger
                .log_line(&format!(
                    "initialised LC_CONFIG_FILE override at {}",
                    override_path.display()
                ))
                .context("failed to log config override initialisation")?;
        }
        logger
            .log_line(&format!(
                "honouring LC_CONFIG_FILE override at {}",
                override_path.display()
            ))
            .context("failed to log config override usage")?;
        apply_headless_display_mode_override(&override_path, logger)
            .context("failed to apply headless display mode override")?;
        return Ok(override_path);
    }

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

    apply_headless_display_mode_override(&config_path, logger)
        .context("failed to apply headless display mode override")?;

    Ok(config_path)
}

fn config_override_path() -> Option<PathBuf> {
    env::var_os("LC_CONFIG_FILE")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
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

fn apply_headless_display_mode_override(config_path: &Path, logger: &LauncherLogger) -> Result<()> {
    let Some(reason) = headless_override_reason() else {
        return Ok(());
    };

    let mut config = match Config::load(config_path) {
        Ok(config) => config,
        Err(err) if err.kind() == io::ErrorKind::NotFound => Config::new(),
        Err(err) => {
            logger
                .log_line(&format!(
                    "headless display guard skipped because {} could not be loaded: {err}",
                    config_path.display()
                ))
                .ok();
            return Ok(());
        }
    };

    let current_value = config
        .get_in(Some("Graphics"), "DisplayMode")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let already_windowed = current_value
        .as_ref()
        .map(|value| {
            let lower = value.to_ascii_lowercase();
            lower == "1" || lower == "window"
        })
        .unwrap_or(false);

    if already_windowed {
        logger
            .log_line(&format!(
                "headless display guard active ({reason}); Graphics.DisplayMode already windowed"
            ))
            .ok();
        return Ok(());
    }

    let previous = current_value.as_deref().unwrap_or("unset");
    config.set_in(Some("Graphics"), "DisplayMode", "1");

    if let Err(err) = config.save(config_path) {
        logger
            .log_line(&format!(
                "headless display guard failed to persist override for {}: {err}",
                config_path.display()
            ))
            .ok();
        return Ok(());
    }

    logger
        .log_line(&format!(
            "headless display guard forced Graphics.DisplayMode=Window (was {previous}) because {reason}"
        ))
        .ok();

    Ok(())
}

fn headless_override_reason() -> Option<String> {
    if env_flag(DISABLE_HEADLESS_GUARD_ENV) || env_flag(FORCE_FULLSCREEN_ENV) {
        return None;
    }
    if env_flag(FORCE_WINDOW_ENV) {
        return Some(format!("{FORCE_WINDOW_ENV} is set"));
    }
    if env_flag("LC_HEADLESS") {
        return Some("LC_HEADLESS is set".to_string());
    }
    if env_value_equals("SDL_VIDEODRIVER", "dummy") {
        return Some("SDL_VIDEODRIVER=dummy".to_string());
    }
    #[cfg(target_os = "linux")]
    {
        if !linux_display_available() {
            return Some("no DISPLAY/WAYLAND/MIR environment variables detected".to_string());
        }
    }
    None
}

fn env_flag(key: &str) -> bool {
    env::var_os(key).filter(|value| !value.is_empty()).is_some()
}

fn env_value_equals(key: &str, expected: &str) -> bool {
    env::var(key)
        .map(|value| value.eq_ignore_ascii_case(expected))
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
fn linux_display_available() -> bool {
    env_flag("DISPLAY") || env_flag("WAYLAND_DISPLAY") || env_flag("MIR_SOCKET")
}

#[cfg(test)]
mod headless_tests {
    use super::*;
    use std::ffi::OsString;
    use std::sync::{Mutex, MutexGuard, OnceLock};
    use tempfile::TempDir;

    struct EnvGuard {
        _lock: MutexGuard<'static, ()>,
        saved: Vec<(String, Option<OsString>)>,
    }

    impl EnvGuard {
        fn set(vars: &[(&str, Option<&str>)]) -> Self {
            let lock = env_lock().lock().unwrap();
            let mut saved = Vec::with_capacity(vars.len());
            for (key, value) in vars {
                let original = env::var_os(key);
                saved.push(((*key).to_string(), original));
                match value {
                    Some(val) => env::set_var(key, val),
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

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn test_logger(dir: &TempDir) -> LauncherLogger {
        let log_path = dir.path().join("test.log");
        let file = File::create(&log_path).unwrap();
        LauncherLogger {
            inner: Arc::new(LauncherLoggerInner {
                writer: Mutex::new(LineWriter::new(file)),
                path: log_path,
            }),
        }
    }

    #[test]
    fn force_window_env_triggers_reason() {
        let _guard = EnvGuard::set(&[
            (FORCE_WINDOW_ENV, Some("1")),
            (FORCE_FULLSCREEN_ENV, None),
            (DISABLE_HEADLESS_GUARD_ENV, None),
        ]);
        let reason = headless_override_reason();
        assert!(reason
            .as_ref()
            .map(|r| r.contains(FORCE_WINDOW_ENV))
            .unwrap_or(false));
    }

    #[test]
    fn force_fullscreen_suppresses_reason() {
        let _guard = EnvGuard::set(&[
            (FORCE_FULLSCREEN_ENV, Some("1")),
            (FORCE_WINDOW_ENV, None),
            (DISABLE_HEADLESS_GUARD_ENV, None),
            ("LC_HEADLESS", Some("1")),
        ]);
        assert!(headless_override_reason().is_none());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_missing_display_triggers_guard() {
        let _guard = EnvGuard::set(&[
            ("DISPLAY", None),
            ("WAYLAND_DISPLAY", None),
            ("MIR_SOCKET", None),
            ("SDL_VIDEODRIVER", None),
            (FORCE_WINDOW_ENV, None),
            (FORCE_FULLSCREEN_ENV, None),
            (DISABLE_HEADLESS_GUARD_ENV, None),
        ]);
        let reason = headless_override_reason();
        assert!(reason
            .as_ref()
            .map(|r| r.contains("DISPLAY") || r.contains("environment"))
            .unwrap_or(false));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_display_variable_allows_fullscreen() {
        let _guard = EnvGuard::set(&[
            ("DISPLAY", Some(":0")),
            ("WAYLAND_DISPLAY", None),
            ("MIR_SOCKET", None),
            ("SDL_VIDEODRIVER", None),
            (FORCE_WINDOW_ENV, None),
            (FORCE_FULLSCREEN_ENV, None),
            (DISABLE_HEADLESS_GUARD_ENV, None),
            ("LC_HEADLESS", None),
        ]);
        assert!(
            headless_override_reason().is_none(),
            "presence of DISPLAY should prevent headless override"
        );
    }

    #[test]
    fn apply_override_updates_config_to_window() {
        let _guard = EnvGuard::set(&[
            (FORCE_WINDOW_ENV, Some("1")),
            (FORCE_FULLSCREEN_ENV, None),
            (DISABLE_HEADLESS_GUARD_ENV, None),
            ("LC_HEADLESS", None),
            ("SDL_VIDEODRIVER", None),
        ]);
        let temp = TempDir::new().unwrap();
        let config_path = temp.path().join("config.cfg");
        std::fs::write(&config_path, "[Graphics]\nDisplayMode=0\n").unwrap();
        let logger = test_logger(&temp);
        apply_headless_display_mode_override(&config_path, &logger).unwrap();
        let cfg = Config::load(&config_path).unwrap();
        assert_eq!(cfg.get_in(Some("Graphics"), "DisplayMode"), Some("1"));
    }

    #[test]
    fn already_windowed_config_remains_untouched() {
        let _guard = EnvGuard::set(&[(FORCE_WINDOW_ENV, Some("1"))]);
        let temp = TempDir::new().unwrap();
        let config_path = temp.path().join("config.cfg");
        std::fs::write(&config_path, "[Graphics]\nDisplayMode=1\n").unwrap();
        let logger = test_logger(&temp);
        apply_headless_display_mode_override(&config_path, &logger).unwrap();
        let contents = std::fs::read_to_string(&config_path).unwrap();
        assert!(contents.contains("DisplayMode=1"));
    }
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

#[derive(Clone)]
struct RuntimeOutputCollector {
    inner: Arc<RuntimeOutputCollectorInner>,
}

struct RuntimeOutputCollectorInner {
    lines: Mutex<Vec<RuntimeLine>>,
    order: AtomicUsize,
}

struct RuntimeLine {
    order: usize,
    kind: StreamKind,
    text: String,
}

impl RuntimeOutputCollector {
    fn new() -> Self {
        Self {
            inner: Arc::new(RuntimeOutputCollectorInner {
                lines: Mutex::new(Vec::new()),
                order: AtomicUsize::new(0),
            }),
        }
    }

    fn record(&self, kind: StreamKind, text: &str) -> Result<()> {
        let trimmed = text.trim_end_matches(['\n', '\r']);
        if trimmed.is_empty() {
            return Ok(());
        }
        let order = self.inner.order.fetch_add(1, Ordering::SeqCst);
        let mut guard = self
            .inner
            .lines
            .lock()
            .map_err(|_| anyhow!("runtime output buffer mutex poisoned"))?;
        guard.push(RuntimeLine {
            order,
            kind,
            text: trimmed.to_string(),
        });
        Ok(())
    }

    fn persist(&self, install_root: &Path, logger: &LauncherLogger) -> Result<Option<PathBuf>> {
        let mut guard = self
            .inner
            .lines
            .lock()
            .map_err(|_| anyhow!("runtime output buffer mutex poisoned"))?;

        if guard.is_empty() {
            return Ok(None);
        }

        guard.sort_by_key(|line| line.order);

        let filename = format!("Clonk-rust-{}.log", timestamp_for_filename());
        let path = install_root.join(&filename);
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .with_context(|| format!("failed to create runtime log {}", path.display()))?;
        let mut writer = LineWriter::new(file);

        for line in guard.iter() {
            writeln!(writer, "[{}] {}", line.kind.display_label(), line.text)?;
        }

        writer
            .flush()
            .with_context(|| format!("failed to flush runtime log {}", path.display()))?;

        logger
            .log_line(&format!(
                "captured runtime output in {} ({} lines)",
                path.display(),
                guard.len()
            ))
            .context("failed to log runtime output capture summary")?;

        guard.clear();

        Ok(Some(path))
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

impl StreamKind {
    fn display_label(self) -> &'static str {
        match self {
            StreamKind::Stdout => "STDOUT",
            StreamKind::Stderr => "STDERR",
        }
    }
}

fn spawn_forwarding_thread<R>(
    reader: R,
    logger: LauncherLogger,
    collector: Option<RuntimeOutputCollector>,
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
            if let Some(ref collector) = collector {
                collector
                    .record(kind, &text)
                    .context("failed to buffer runtime output")?;
            }
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
        .context("failed to log updater tool path")?;

    let summary = probe_update_tool(&patcher, install_root).with_context(|| {
        format!(
            "failed to execute {} while validating updater tool",
            patcher.display()
        )
    })?;

    logger
        .log_line(&format!("c4group responded: {summary}"))
        .context("failed to record updater probe output")
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

fn probe_update_tool(patcher: &Path, install_root: &Path) -> Result<String> {
    let output = Command::new(patcher)
        .current_dir(install_root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("failed to spawn {}", patcher.display()))?;

    if !output.status.success() {
        let mut message = format!(
            "c4group validation failed: {}",
            describe_exit_status(&output.status)
        );
        if let Some(snippet) = summarise_tool_output(&output.stdout, &output.stderr) {
            message.push_str(&format!(" ({snippet})"));
        }
        bail!(message);
    }

    let summary = summarise_tool_output(&output.stdout, &output.stderr)
        .unwrap_or_else(|| "c4group produced no output".to_string());
    Ok(summary)
}

fn summarise_tool_output(stdout: &[u8], stderr: &[u8]) -> Option<String> {
    for raw in [stdout, stderr] {
        if raw.is_empty() {
            continue;
        }
        let text = String::from_utf8_lossy(raw);
        if let Some(line) = text.lines().map(str::trim).find(|line| !line.is_empty()) {
            return Some(truncate_summary(line));
        }
    }
    None
}

fn truncate_summary(line: &str) -> String {
    const LIMIT: usize = 160;
    let mut truncated = String::with_capacity(LIMIT + 1);
    let mut chars = line.chars();
    for _ in 0..(LIMIT - 1) {
        if let Some(ch) = chars.next() {
            truncated.push(ch);
        } else {
            return line.to_string();
        }
    }
    if chars.next().is_none() {
        return line.to_string();
    }
    truncated.push('…');
    truncated
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
    fs::create_dir_all(logs_dir).with_context(|| {
        format!(
            "failed to ensure logs directory {} exists for crash artifacts",
            logs_dir.display()
        )
    })?;

    let mut sources = vec![
        paths.user_data_dir().to_path_buf(),
        paths.install_root().to_path_buf(),
    ];
    if !sources.iter().any(|dir| dir == logs_dir) {
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

            let dest_path = if path.starts_with(logs_dir) {
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

fn emit_launcher_report(
    paths: &AppPaths,
    support_bundle: Option<&Path>,
    telemetry_summary: &UpdateTelemetrySummary,
    report: &SupportBundleReport,
    automation_report: bool,
) -> Result<()> {
    if automation_report {
        emit_automation_report(paths, support_bundle, telemetry_summary, report)
    } else {
        print_launcher_report(report);
        Ok(())
    }
}

fn print_launcher_report(report: &SupportBundleReport) {
    let mut stdout = io::stdout();
    let _ = stdout.write_all(b"\n");
    tracing::info!("");
    for line in &report.lines {
        let _ = stdout.write_all(line.as_bytes());
        let _ = stdout.write_all(b"\n");
        tracing::info!("{}", line);
    }
}

fn emit_automation_report(
    paths: &AppPaths,
    support_bundle: Option<&Path>,
    telemetry_summary: &UpdateTelemetrySummary,
    report: &SupportBundleReport,
) -> Result<()> {
    let payload = build_automation_report(paths, support_bundle, telemetry_summary, report);
    let json =
        serde_json::to_string_pretty(&payload).context("failed to serialise automation report")?;
    let mut stdout = io::stdout();
    let _ = stdout.write_all(json.as_bytes());
    let _ = stdout.write_all(b"\n");
    tracing::info!("{}", json);
    Ok(())
}

fn build_automation_report(
    paths: &AppPaths,
    support_bundle: Option<&Path>,
    telemetry_summary: &UpdateTelemetrySummary,
    report: &SupportBundleReport,
) -> AutomationReport {
    AutomationReport {
        logs_dir: paths.logs_dir().display().to_string(),
        summary_path: paths
            .logs_dir()
            .join("launcher-summary.json")
            .display()
            .to_string(),
        support_bundle_path: support_bundle.map(|path| path.display().to_string()),
        telemetry: AutomationTelemetry::from(telemetry_summary),
        triage_summary: report.triage.clone(),
        report_lines: report.lines.clone(),
    }
}

#[derive(Debug, Serialize)]
struct AutomationReport {
    logs_dir: String,
    summary_path: String,
    support_bundle_path: Option<String>,
    telemetry: AutomationTelemetry,
    triage_summary: Option<ReportSearchTriageSummary>,
    report_lines: Vec<String>,
}

#[derive(Debug, Serialize)]
struct AutomationTelemetry {
    successes: Vec<String>,
    failures: Vec<AutomationTelemetryFailure>,
}

#[derive(Debug, Serialize)]
struct AutomationTelemetryFailure {
    log_path: String,
    message: String,
}

impl From<&UpdateTelemetrySummary> for AutomationTelemetry {
    fn from(summary: &UpdateTelemetrySummary) -> Self {
        let successes = summary
            .successes()
            .iter()
            .map(|path| path.display().to_string())
            .collect();
        let failures = summary
            .failures()
            .iter()
            .map(|failure| AutomationTelemetryFailure {
                log_path: failure.log_path.display().to_string(),
                message: failure.message.clone(),
            })
            .collect();
        Self {
            successes,
            failures,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lc_launcher::{
        ProviderAutomationRecord, ProviderAutomationSnapshot, ProviderAutomationState,
        ProviderBulkRetargetRecord, ProviderBulkRetargetSummary, ProviderPathStatus,
    };
    use serde_json::Value;
    use std::env;
    use std::fs::{self, File};
    use std::io::{LineWriter, Read};
    use std::path::Path;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, SystemTime};
    use tempfile::TempDir;
    use zip::ZipArchive;

    fn test_logger(dir: &TempDir) -> LauncherLogger {
        let log_path = dir.path().join("test.log");
        let file = File::create(&log_path).unwrap();
        LauncherLogger {
            inner: Arc::new(LauncherLoggerInner {
                writer: Mutex::new(LineWriter::new(file)),
                path: log_path,
            }),
        }
    }

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
    fn ensure_runtime_assets_populates_group_files() {
        let install_dir = TempDir::new().unwrap();
        let planet_dir = install_dir.path().join("planet");
        fs::create_dir_all(&planet_dir).unwrap();
        fs::write(planet_dir.join("System.c4g"), b"system payload").unwrap();
        fs::write(planet_dir.join("Graphics.c4g"), b"graphics payload").unwrap();

        let binary_path = install_dir
            .path()
            .join("build")
            .join("lc-app.app")
            .join("Contents")
            .join("MacOS")
            .join("lc-app");
        fs::create_dir_all(binary_path.parent().unwrap()).unwrap();
        fs::write(&binary_path, b"stub").unwrap();

        let user_dir = TempDir::new().unwrap();
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.path())),
        ]);

        let paths = AppPaths::discover().unwrap();
        paths.ensure_user_dirs().unwrap();
        let logger = LauncherLogger::new(&paths).unwrap();

        ensure_runtime_assets(&paths, &binary_path, &logger)
            .expect("runtime assets should be linked");

        let system_target = install_dir.path().join("System.c4g");
        let graphics_target = install_dir.path().join("Graphics.c4g");
        let binary_dir = binary_path.parent().unwrap();
        let binary_system = binary_dir.join("System.c4g");
        let binary_graphics = binary_dir.join("Graphics.c4g");
        let bundle_root = binary_dir
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.parent())
            .expect("bundle root should exist in test fixture");
        let bundle_system = bundle_root.join("System.c4g");
        let bundle_graphics = bundle_root.join("Graphics.c4g");
        assert!(system_target.exists(), "system group should exist");
        assert!(graphics_target.exists(), "graphics group should exist");
        assert!(
            binary_system.exists(),
            "system group should exist next to the binary"
        );
        assert!(
            binary_graphics.exists(),
            "graphics group should exist next to the binary"
        );
        assert!(
            bundle_system.exists(),
            "system group should exist at mac bundle root"
        );
        assert!(
            bundle_graphics.exists(),
            "graphics group should exist at mac bundle root"
        );
        assert_eq!(
            fs::read(&system_target).unwrap(),
            b"system payload",
            "system group contents should match source"
        );
        assert_eq!(
            fs::read(&graphics_target).unwrap(),
            b"graphics payload",
            "graphics group contents should match source"
        );
        assert_eq!(
            fs::read(&binary_system).unwrap(),
            b"system payload",
            "system group adjacent to binary should match source"
        );
        assert_eq!(
            fs::read(&binary_graphics).unwrap(),
            b"graphics payload",
            "graphics group adjacent to binary should match source"
        );
        assert_eq!(
            fs::read(&bundle_system).unwrap(),
            b"system payload",
            "system group at bundle root should match source"
        );
        assert_eq!(
            fs::read(&bundle_graphics).unwrap(),
            b"graphics payload",
            "graphics group at bundle root should match source"
        );
    }

    #[test]
    fn ensure_runtime_assets_replaces_stale_targets() {
        let install_dir = TempDir::new().unwrap();
        let planet_dir = install_dir.path().join("planet");
        fs::create_dir_all(&planet_dir).unwrap();
        fs::write(planet_dir.join("System.c4g"), b"system payload").unwrap();
        fs::write(planet_dir.join("Graphics.c4g"), b"graphics payload").unwrap();

        let binary_path = install_dir.path().join("lc-app.exe");
        fs::create_dir_all(binary_path.parent().unwrap()).unwrap();
        fs::write(&binary_path, b"stub").unwrap();

        let stale_system = install_dir.path().join("System.c4g");
        let stale_graphics = install_dir.path().join("Graphics.c4g");
        fs::write(&stale_system, b"old system").unwrap();
        fs::write(&stale_graphics, b"old graphics").unwrap();

        let user_dir = TempDir::new().unwrap();
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.path())),
        ]);

        let paths = AppPaths::discover().unwrap();
        paths.ensure_user_dirs().unwrap();
        let logger = LauncherLogger::new(&paths).unwrap();

        ensure_runtime_assets(&paths, &binary_path, &logger)
            .expect("runtime assets should be refreshed");

        assert_eq!(
            fs::read(&stale_system).unwrap(),
            b"system payload",
            "system target should match updated source"
        );
        assert_eq!(
            fs::read(&stale_graphics).unwrap(),
            b"graphics payload",
            "graphics target should match updated source"
        );

        let binary_system = binary_path.parent().unwrap().join("System.c4g");
        let binary_graphics = binary_path.parent().unwrap().join("Graphics.c4g");
        assert!(
            binary_system.exists(),
            "system group should be materialised alongside binary"
        );
        assert!(
            binary_graphics.exists(),
            "graphics group should be materialised alongside binary"
        );

        // A second run should succeed even when the targets already point at the source.
        assert!(ensure_runtime_assets(&paths, &binary_path, &logger).is_ok());
    }

    #[test]
    fn prepare_config_honours_override_path() {
        let install_dir = TempDir::new().unwrap();
        let planet_dir = install_dir.path().join("planet");
        fs::create_dir_all(&planet_dir).unwrap();
        fs::write(planet_dir.join("System.c4g"), b"system payload").unwrap();

        let user_dir = TempDir::new().unwrap();
        let override_dir = TempDir::new().unwrap();
        let override_path = override_dir.path().join("custom.cfg");
        let log_dir = TempDir::new().unwrap();

        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.path())),
            ("LC_CONFIG_FILE", Some(override_path.as_path())),
            ("LC_LEGACY_CONFIG_FILE", None),
            ("LC_GAME_DISABLE_HEADLESS_GUARD", Some(Path::new("1"))),
        ]);

        let paths = AppPaths::discover().unwrap();
        paths.ensure_user_dirs().unwrap();
        let logger = test_logger(&log_dir);

        let config_path =
            prepare_config(&paths, &logger).expect("config preparation should succeed");

        assert_eq!(config_path, override_path);
        assert!(config_path.exists(), "override config file should exist");
        assert!(
            fs::metadata(&config_path).unwrap().is_file(),
            "override config path should resolve to a file"
        );
    }

    #[test]
    fn prepare_config_migrates_from_legacy_env_path() {
        let install_dir = TempDir::new().unwrap();
        let planet_dir = install_dir.path().join("planet");
        fs::create_dir_all(&planet_dir).unwrap();
        fs::write(planet_dir.join("System.c4g"), b"system payload").unwrap();

        let user_dir = TempDir::new().unwrap();
        let legacy_dir = TempDir::new().unwrap();
        let legacy_path = legacy_dir.path().join("Legacy.cfg");
        fs::write(&legacy_path, b"[Game]\nFullscreen=1\n").unwrap();
        let log_dir = TempDir::new().unwrap();

        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.path())),
            ("LC_LEGACY_CONFIG_FILE", Some(legacy_path.as_path())),
            ("LC_CONFIG_FILE", None),
            ("LC_GAME_DISABLE_HEADLESS_GUARD", Some(Path::new("1"))),
        ]);

        let paths = AppPaths::discover().unwrap();
        paths.ensure_user_dirs().unwrap();
        let logger = test_logger(&log_dir);

        let config_path =
            prepare_config(&paths, &logger).expect("config preparation should succeed");

        assert_eq!(config_path, paths.config_file());
        assert!(
            config_path.exists(),
            "default config path should be materialised"
        );
        let migrated = fs::read(&config_path).expect("migrated config should be readable");
        let original = fs::read(&legacy_path).expect("legacy config should be readable");
        assert_eq!(
            migrated, original,
            "launcher should copy contents from LC_LEGACY_CONFIG_FILE"
        );
    }

    #[test]
    fn candidate_list_covers_macos_bundle() {
        let temp = TempDir::new().unwrap();
        let app_dir = temp
            .path()
            .join("build")
            .join("lc-app.app")
            .join("Contents")
            .join("MacOS");
        fs::create_dir_all(&app_dir).unwrap();
        let binary = app_dir.join("lc-app");
        fs::write(&binary, b"stub").unwrap();
        let resolved =
            resolve_runtime_binary(None, temp.path()).expect("should locate bundle binary");
        assert_eq!(resolved, binary);
    }

    #[test]
    fn respects_override_argument() {
        let temp = TempDir::new().unwrap();
        let override_bin = temp.path().join("custom").join("lc-app");
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

    #[cfg(unix)]
    #[test]
    fn validate_update_tool_runs_binary() {
        use std::os::unix::fs::PermissionsExt;

        let install_dir = TempDir::new().unwrap();
        let planet_dir = install_dir.path().join("planet");
        fs::create_dir_all(&planet_dir).unwrap();
        fs::write(planet_dir.join("System.c4g"), b"stub").unwrap();
        let patcher_path = install_dir.path().join("c4group");
        fs::write(
            &patcher_path,
            b"#!/bin/sh\necho \"LegacyClonk C4Group 9.0\"\nexit 0\n",
        )
        .unwrap();
        let mut perms = fs::metadata(&patcher_path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&patcher_path, perms).unwrap();

        let user_dir = TempDir::new().unwrap();
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.path())),
        ]);

        let paths = AppPaths::discover().unwrap();
        paths.ensure_user_dirs().unwrap();
        let logger = LauncherLogger::new(&paths).unwrap();

        validate_update_tool(&paths, &logger).unwrap();
        let log_contents = std::fs::read_to_string(logger.path()).unwrap();
        assert!(
            log_contents.contains("c4group responded: LegacyClonk C4Group 9.0"),
            "probe output missing in log:\n{log_contents}"
        );
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

    #[cfg(unix)]
    #[test]
    fn validate_update_tool_reports_probe_failure() {
        use std::os::unix::fs::PermissionsExt;

        let install_dir = TempDir::new().unwrap();
        let planet_dir = install_dir.path().join("planet");
        fs::create_dir_all(&planet_dir).unwrap();
        fs::write(planet_dir.join("System.c4g"), b"stub").unwrap();
        let patcher_path = install_dir.path().join("c4group");
        fs::write(
            &patcher_path,
            b"#!/bin/sh\necho \"probe failure\" >&2\nexit 3\n",
        )
        .unwrap();
        let mut perms = fs::metadata(&patcher_path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&patcher_path, perms).unwrap();

        let user_dir = TempDir::new().unwrap();
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.path())),
        ]);

        let paths = AppPaths::discover().unwrap();
        paths.ensure_user_dirs().unwrap();
        let logger = LauncherLogger::new(&paths).unwrap();

        let err = validate_update_tool(&paths, &logger).unwrap_err();
        let display = err.to_string();
        let root = err.chain().next_back().unwrap().to_string();
        assert!(
            display.contains("failed to execute"),
            "context missing command failure: {display}"
        );
        assert!(
            root.contains("exit code 3"),
            "unexpected root cause: {root}"
        );
        assert!(
            root.contains("probe failure"),
            "missing stderr snippet: {root}"
        );
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

        append_support_bundle_report(&paths, &bundle_path, &telemetry).unwrap();

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
        assert!(
            entries.contains(&"support-bundle-report.txt".to_string()),
            "bundle should include textual report, entries: {entries:?}"
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

        let mut report_contents = String::new();
        archive
            .by_name("support-bundle-report.txt")
            .unwrap()
            .read_to_string(&mut report_contents)
            .unwrap();
        assert!(
            report_contents.contains("Launcher summary written to"),
            "report should describe summary path: {report_contents}"
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
        assert!(
            entries.contains(&"support-bundle-report.txt".to_string()),
            "bundle should include textual report entries: {entries:?}"
        );

        let mut report = String::new();
        archive
            .by_name("support-bundle-report.txt")
            .unwrap()
            .read_to_string(&mut report)
            .unwrap();
        assert!(
            report.contains("Support bundle available"),
            "report should mention support bundle availability: {report}"
        );
    }

    #[test]
    fn provider_report_surfaces_history_cleared_without_records() {
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
            .log_line("provider report history cleared test")
            .unwrap();

        let bulk_summary = ProviderBulkRetargetSummary {
            history_cleared_at: Some("2024-06-05T18:30:00Z".into()),
            ..Default::default()
        };

        let telemetry = UpdateTelemetrySummary::default();

        write_launcher_summary(
            &paths,
            &logger,
            logger.path(),
            &[],
            &[],
            &telemetry,
            None,
            Some(ProviderAutomationSnapshot::default()),
            Some(bulk_summary),
            None,
        )
        .unwrap();

        let lines = build_support_bundle_report(&paths, None, &telemetry).lines;
        assert!(
            lines.iter().any(|line| line == "  Bulk retarget history:"),
            "expected bulk retarget history headline to be rendered: {lines:?}"
        );
        assert!(
            lines.iter().any(|line| line.contains(
                "Bulk retarget history was cleared at 2024-06-05T18:30:00Z. No retarget records remain while providers use default staging paths."
            )),
            "expected history cleared annotation to match UI wording: {lines:?}"
        );
    }

    #[test]
    fn provider_report_surfaces_history_cleared_alongside_records() {
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
        logger.log_line("provider report records test").unwrap();

        let mut bulk_summary = ProviderBulkRetargetSummary::default();
        bulk_summary.share.push(ProviderBulkRetargetRecord {
            base_path: "support-share".into(),
            retargeted_at: "2024-06-02T16:00:00Z".into(),
            total: 3,
            changed: 2,
        });
        bulk_summary.history_cleared_at = Some("2024-06-05T18:30:00Z".into());

        let mut automation = ProviderAutomationSnapshot::default();
        automation.share.push(ProviderAutomationRecord {
            name: "Support Share Drop".into(),
            path: "support-share".into(),
            path_status: ProviderPathStatus::Ready,
            automation: ProviderAutomationState::Idle,
            default_path: Some("support-share-default".into()),
            overrides: Vec::new(),
        });

        let telemetry = UpdateTelemetrySummary::default();

        write_launcher_summary(
            &paths,
            &logger,
            logger.path(),
            &[],
            &[],
            &telemetry,
            None,
            Some(automation),
            Some(bulk_summary),
            None,
        )
        .unwrap();

        let lines = build_support_bundle_report(&paths, None, &telemetry).lines;
        assert!(
            lines
                .iter()
                .any(|line| line
                    .contains("Bulk retarget history last cleared at 2024-06-05T18:30:00Z")),
            "expected history cleared annotation alongside records: {lines:?}"
        );
        assert!(
            lines.iter().any(|line| line.contains("support-share")),
            "expected share base path to appear in report: {lines:?}"
        );
    }

    #[test]
    fn automation_report_includes_triage_summary() {
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
        logger.log_line("automation triage test").unwrap();

        let telemetry = UpdateTelemetrySummary::default();
        let search_preferences = lc_launcher::ReportSearchPreferences {
            query: "support".into(),
            highlight: lc_launcher::ReportSearchHighlightPreference::Generic,
            active_line: None,
        };

        write_launcher_summary(
            &paths,
            &logger,
            logger.path(),
            &[],
            &[],
            &telemetry,
            None,
            None,
            None,
            Some(search_preferences),
        )
        .unwrap();

        let report = build_support_bundle_report(&paths, None, &telemetry);
        assert!(
            report.triage.is_some(),
            "triage summary should be attached to support bundle report"
        );

        let automation = build_automation_report(&paths, None, &telemetry, &report);
        let triage = automation
            .triage_summary
            .expect("automation report should include triage summary");
        assert_eq!(triage.query, "support");
        assert_eq!(triage.match_count, 2);
        assert!(
            automation
                .report_lines
                .iter()
                .any(|line| line == "Search (text): \"support\" — 2 match(es)."),
            "rendered report lines should include triage summary header: {:?}",
            automation.report_lines
        );
    }
}
