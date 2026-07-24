// Explorer must not open a console window behind the launcher; `main`
// reattaches stdio when it is started from a terminal instead.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod legacy_registry;

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
use clonk_core::std_config::Config;
use clonk_launcher::{
    append_support_bundle_report, build_support_bundle_report, create_support_bundle,
    digest_update_telemetry, regenerate_support_bundle, timestamp_for_filename, timestamp_for_log,
    write_launcher_summary, LauncherLog, ReportSearchTriageSummary, SupportBundleReport,
    UpdateTelemetrySummary,
};
use clonk_platform::AppPaths;
use legacy_registry::{
    read_legacy_windows_registry, serialize_legacy_registry_config, LegacyRegistryConfig,
};
use serde::Serialize;

const SKIP_PATCHER_VALIDATION_ENV: &str = "LC_GAME_SKIP_PATCHER_CHECK";
const FORCE_WINDOW_ENV: &str = "LC_GAME_FORCE_WINDOW";
const FORCE_FULLSCREEN_ENV: &str = "LC_GAME_FORCE_FULLSCREEN";
const DISABLE_HEADLESS_GUARD_ENV: &str = "LC_GAME_DISABLE_HEADLESS_GUARD";
const LEGACY_LOG_PREFIX: &str = "Clonk";
const LEGACY_LOG_SUFFIX: &str = ".log";
const CRASH_ARTIFACT_MARKER: &str = "-crash-";
const OFFICIAL_LEAGUE_SERVER: &str = "https://league.clonkspot.org";
const OFFICIAL_UPDATE_SERVER: &str = "https://update.clonkspot.org/lc/update";
const OFFICIAL_PUNCHER_SERVER: &str = "netpuncher.openclonk.org:11115";
const CLASSIC_CONFIG_VERSION: u32 = 362;
const CLASSIC_CONFIG_VERSION_VALUE: &str = "362";
const CLASSIC_UNVERSIONED_CONFIG_VERSION: u32 = 347;

#[derive(Debug, Parser)]
#[command(
    name = "clonk-game",
    about = "Clonk Rust launcher that runs the Rust runtime",
    version,
    author
)]
struct Cli {
    /// Override the detected Clonk Rust runtime binary location
    #[arg(long = "binary", value_name = "PATH")]
    binary: Option<PathBuf>,

    /// Regenerate a support bundle using the latest launcher summary without starting the runtime
    #[arg(long = "support-bundle-only")]
    support_bundle_only: bool,

    /// Emit launcher diagnostics as JSON for automation consumers
    #[arg(long = "automation-report")]
    automation_report: bool,

    /// Arguments forwarded verbatim to the Clonk Rust runtime
    #[arg(trailing_var_arg = true)]
    forwarded: Vec<OsString>,
}

fn main() {
    // Must precede any output: the GUI subsystem starts with stdio detached.
    clonk_platform::attach_parent_console();
    clonk_logging::init();

    if let Err(error) = run() {
        tracing::error!(error = ?error, "clonk-game encountered an error");
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
    // `prepare_config` may have copied a legacy config that was not present
    // during bootstrap discovery. Re-read the selected file before deriving
    // runtime paths so its General.UserPath applies on this first launch.
    let paths = rediscover_paths_after_config(&config_path)
        .context("failed to apply configured application paths")?;

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
        bail!("Clonk Rust exited {}", describe_exit_status(&status));
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
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let a_meta = fs::metadata(a)?;
        let b_meta = fs::metadata(b)?;
        Ok(a_meta.ino() == b_meta.ino() && a_meta.dev() == b_meta.dev())
    }
    #[cfg(not(unix))]
    {
        // Windows file identity (`MetadataExt::file_index` and
        // `volume_serial_number`) is still unstable, so identity comes from the
        // fully resolved path. `canonicalize` resolves symlinks and junctions
        // and errors on a missing path, matching the Unix arm's guarantees.
        Ok(fs::canonicalize(a)? == fs::canonicalize(b)?)
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
        "clonk-app",
        "clonk-app.exe",
        "bin/clonk-app",
        "bin/clonk-app.exe",
        "clonk-app.app/Contents/MacOS/clonk-app",
        "bin/clonk-app.app/Contents/MacOS/clonk-app",
        "build/clonk-app",
        "build/clonk-app.exe",
        "build/Debug/clonk-app",
        "build/Debug/clonk-app.exe",
        "build/Release/clonk-app",
        "build/Release/clonk-app.exe",
        "build/clonk-app.app/Contents/MacOS/clonk-app",
        "build/Debug/clonk-app.app/Contents/MacOS/clonk-app",
        "build/Release/clonk-app.app/Contents/MacOS/clonk-app",
        "rust/target/debug/clonk-app",
        "rust/target/debug/clonk-app.exe",
        "rust/target/release/clonk-app",
        "rust/target/release/clonk-app.exe",
        "rust_port/target/debug/clonk-app",
        "rust_port/target/debug/clonk-app.exe",
        "rust_port/target/release/clonk-app",
        "rust_port/target/release/clonk-app.exe",
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
        results.push(dir.join("clonk-app"));
        results.push(dir.join("clonk-app.exe"));
        results.push(
            dir.join("clonk-app.app")
                .join("Contents")
                .join("MacOS")
                .join("clonk-app"),
        );
    }
    if let Some(bundle_dir) = exe
        .parent()
        .and_then(|dir| dir.parent())
        .and_then(|contents| contents.parent())
    {
        // Handles clonk-game inside a .app bundle by looking for a sibling clonk-app bundle.
        results.push(
            bundle_dir
                .join("clonk-app.app")
                .join("Contents")
                .join("MacOS")
                .join("clonk-app"),
        );
    }
    results
}

fn prepare_config(paths: &AppPaths, logger: &LauncherLogger) -> Result<PathBuf> {
    prepare_config_with_registry_reader(paths, logger, read_legacy_windows_registry)
}

fn prepare_config_with_registry_reader<F>(
    paths: &AppPaths,
    logger: &LauncherLogger,
    read_registry: F,
) -> Result<PathBuf>
where
    F: FnOnce() -> Result<Option<LegacyRegistryConfig>>,
{
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
        adapt_config_to_current_version(&override_path, logger)
            .context("failed to adapt classic configuration")?;
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
        adapt_config_to_current_version(&config_path, logger)
            .context("failed to adapt classic configuration")?;
        return Ok(config_path);
    }

    let mut migrated = false;
    let mut migrated_from_registry = false;
    if let Some(candidate) = explicit_legacy_config_file().filter(|candidate| candidate.exists()) {
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
    }

    if !migrated {
        if let Some(registry) =
            read_registry().context("failed to read classic Windows registry config")?
        {
            if let Some(config) = serialize_legacy_registry_config(&registry)
                .context("failed to convert classic Windows registry config")?
            {
                migrate_registry_config_atomically(&config_path, &config, logger).with_context(
                    || {
                        format!(
                            "failed to migrate Windows registry config to {}",
                            config_path.display()
                        )
                    },
                )?;
                logger
                    .log_line("migrated config from HKCU\\Software\\LegacyClonk Team\\LegacyClonk")
                    .context("failed to log Windows registry config migration")?;
                migrated = true;
                migrated_from_registry = true;
            }
        }
    }

    for candidate in heuristic_legacy_config_candidates() {
        if migrated {
            break;
        }
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

    if !migrated_from_registry {
        adapt_config_to_current_version(&config_path, logger)
            .context("failed to adapt classic configuration")?;
        apply_headless_display_mode_override(&config_path, logger)
            .context("failed to apply headless display mode override")?;
    }

    Ok(config_path)
}

fn migrate_registry_config_atomically(
    config_path: &Path,
    config: &[u8],
    logger: &LauncherLogger,
) -> Result<()> {
    let file_name = config_path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "config path has no filename"))?
        .to_string_lossy();
    let temporary_path = config_path.with_file_name(format!(
        ".{file_name}.registry-import-{}.tmp",
        std::process::id()
    ));

    let result = (|| {
        fs::write(&temporary_path, config)
            .with_context(|| format!("failed to stage {}", config_path.display()))?;
        adapt_config_to_current_version(&temporary_path, logger)
            .context("failed to adapt staged registry config")?;
        apply_headless_display_mode_override_with_io_policy(&temporary_path, logger, true)
            .context("failed to apply headless override to staged registry config")?;
        fs::rename(&temporary_path, config_path)
            .with_context(|| format!("failed to publish {}", config_path.display()))?;
        Ok(())
    })();
    if result.is_err() {
        // A failed first-run import must not leave a destination that would
        // suppress the next migration attempt.
        let _ = fs::remove_file(&temporary_path);
    }
    result
}

fn rediscover_paths_after_config(config_path: &Path) -> Result<AppPaths> {
    let paths = AppPaths::discover_with_config_file(Some(config_path))
        .context("failed to rediscover paths from the prepared config")?;
    paths
        .ensure_user_dirs()
        .context("failed to prepare configured user directories")?;
    Ok(paths)
}

fn adapt_config_to_current_version(config_path: &Path, logger: &LauncherLogger) -> Result<()> {
    let original = fs::read(config_path)
        .with_context(|| format!("failed to read {}", config_path.display()))?;
    let source_version = classic_config_exact_assignment_value(&original, "General", "Version")
        .and_then(parse_cpp_config_u32)
        .unwrap_or(CLASSIC_UNVERSIONED_CONFIG_VERSION);
    let mut config = original.clone();
    let mut repaired = Vec::new();
    for key in ["ServerAddress", "AlternateServerAddress"] {
        let was_truncated = classic_config_string_value(&config, "Network", key)
            .is_some_and(|value| matches!(value.as_slice(), b"http:" | b"https:"));
        if was_truncated {
            apply_classic_config_update(
                &mut config,
                "Network",
                key,
                ClassicConfigWriteValue::Escaped(OFFICIAL_LEAGUE_SERVER),
                key,
                &mut repaired,
            );
        }
    }

    let mut adapted = Vec::new();
    apply_classic_config_update(
        &mut config,
        "General",
        "Version",
        ClassicConfigWriteValue::Raw(CLASSIC_CONFIG_VERSION_VALUE),
        "General.Version",
        &mut adapted,
    );

    #[cfg(target_os = "macos")]
    if source_version == 349 {
        apply_classic_config_update(
            &mut config,
            "General",
            "Preloading",
            ClassicConfigWriteValue::Raw("false"),
            "General.Preloading",
            &mut adapted,
        );
    }

    if source_version == 347 {
        apply_classic_config_update(
            &mut config,
            "Sound",
            "MaxChannels",
            ClassicConfigWriteValue::Raw("1024"),
            "Sound.MaxChannels",
            &mut adapted,
        );
    }
    if matches!(source_version, 346 | 347) {
        apply_classic_config_update(
            &mut config,
            "Sound",
            "Music",
            ClassicConfigWriteValue::Raw("true"),
            "Sound.Music",
            &mut adapted,
        );
    }

    if source_version <= 359 {
        migrate_classic_config_string(
            &mut config,
            "Network",
            "ServerAddress",
            "league.clonkspot.org:80",
            OFFICIAL_LEAGUE_SERVER,
            &mut adapted,
        );
        migrate_classic_config_string(
            &mut config,
            "Network",
            "AlternateServerAddress",
            "league.clonkspot.org:80",
            OFFICIAL_LEAGUE_SERVER,
            &mut adapted,
        );
        migrate_classic_config_string(
            &mut config,
            "Network",
            "UpdateServerAddress",
            "update.clonkspot.org/lc/update",
            OFFICIAL_UPDATE_SERVER,
            &mut adapted,
        );
        migrate_classic_config_string(
            &mut config,
            "Network",
            "PuncherAddress",
            "clonk.de:11115",
            OFFICIAL_PUNCHER_SERVER,
            &mut adapted,
        );
        apply_classic_config_update(
            &mut config,
            "Graphics",
            "Shader",
            ClassicConfigWriteValue::Raw("true"),
            "Graphics.Shader",
            &mut adapted,
        );
        apply_classic_config_update(
            &mut config,
            "Graphics",
            "DisableGamma",
            ClassicConfigWriteValue::Raw("false"),
            "Graphics.DisableGamma",
            &mut adapted,
        );
    }

    if config == original {
        return Ok(());
    }

    fs::write(config_path, config)
        .with_context(|| format!("failed to save {}", config_path.display()))?;
    if !repaired.is_empty() {
        logger
            .log_line(&format!(
                "repaired Rust-truncated network URL field(s): {}",
                repaired.join(", ")
            ))
            .context("failed to log masterserver configuration repair")?;
    }
    if !adapted.is_empty() {
        logger
            .log_line(&format!(
                "adapted classic configuration from version {source_version} to {CLASSIC_CONFIG_VERSION}: {}",
                adapted.join(", ")
            ))
            .context("failed to log classic configuration adaptation")?;
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum ClassicConfigWriteValue<'a> {
    Raw(&'a str),
    Escaped(&'a str),
}

#[derive(Clone, Copy)]
struct ClassicConfigLine {
    start: usize,
    content_end: usize,
    end: usize,
}

fn apply_classic_config_update(
    config: &mut Vec<u8>,
    section: &str,
    key: &str,
    value: ClassicConfigWriteValue<'_>,
    label: &'static str,
    changed: &mut Vec<&'static str>,
) {
    let updated = update_classic_config_value(config, section, key, value);
    if updated != *config {
        *config = updated;
        changed.push(label);
    }
}

fn migrate_classic_config_string(
    config: &mut Vec<u8>,
    section: &str,
    key: &str,
    old_value: &str,
    new_value: &str,
    adapted: &mut Vec<&'static str>,
) {
    if !classic_config_string_value(config, section, key)
        .is_some_and(|value| value == old_value.as_bytes())
    {
        return;
    }
    let label = match key {
        "ServerAddress" => "Network.ServerAddress",
        "AlternateServerAddress" => "Network.AlternateServerAddress",
        "UpdateServerAddress" => "Network.UpdateServerAddress",
        "PuncherAddress" => "Network.PuncherAddress",
        _ => unreachable!("unexpected classic network migration key: {key}"),
    };
    apply_classic_config_update(
        config,
        section,
        key,
        ClassicConfigWriteValue::Escaped(new_value),
        label,
        adapted,
    );
}

fn update_classic_config_value(
    config: &[u8],
    section: &str,
    key: &str,
    value: ClassicConfigWriteValue<'_>,
) -> Vec<u8> {
    let lines = classic_config_lines(config);
    let mut section_line = None;
    let mut section_end = lines.len();
    for (index, line) in lines.iter().enumerate() {
        let Some(name) = classic_config_section_name(&config[line.start..line.content_end]) else {
            continue;
        };
        if let Some(selected) = section_line {
            if index > selected {
                section_end = index;
                break;
            }
        } else if name == section.as_bytes() {
            section_line = Some(index);
        }
    }

    let line_ending = section_line
        .and_then(|index| classic_config_line_ending(config, lines[index]))
        .or_else(|| {
            lines
                .iter()
                .find_map(|line| classic_config_line_ending(config, *line))
        })
        .unwrap_or(b"\n");
    let assignment = encode_classic_config_assignment(key, value);

    if let Some(section_line) = section_line {
        for line in &lines[section_line + 1..section_end] {
            let content = &config[line.start..line.content_end];
            if classic_config_assignment_key(content) == Some(key.as_bytes()) {
                let indent_end = content
                    .iter()
                    .position(|byte| !matches!(byte, b' ' | b'\t'))
                    .unwrap_or(content.len());
                let suffix = classic_config_assignment_suffix(content);
                let mut output = Vec::with_capacity(config.len() + assignment.len());
                output.extend_from_slice(&config[..line.start]);
                output.extend_from_slice(&content[..indent_end]);
                output.extend_from_slice(&assignment);
                output.extend_from_slice(suffix);
                output.extend_from_slice(&config[line.content_end..]);
                return output;
            }
        }

        let insert_at = lines
            .get(section_end)
            .map_or(config.len(), |line| line.start);
        let mut output = config[..insert_at].to_vec();
        ensure_classic_config_line_break(&mut output, line_ending);
        output.extend_from_slice(&assignment);
        output.extend_from_slice(line_ending);
        output.extend_from_slice(&config[insert_at..]);
        return output;
    }

    let mut output = config.to_vec();
    ensure_classic_config_line_break(&mut output, line_ending);
    output.push(b'[');
    output.extend_from_slice(section.as_bytes());
    output.push(b']');
    output.extend_from_slice(line_ending);
    output.extend_from_slice(&assignment);
    output.extend_from_slice(line_ending);
    output
}

fn encode_classic_config_assignment(key: &str, value: ClassicConfigWriteValue<'_>) -> Vec<u8> {
    let mut output = Vec::with_capacity(key.len() + 2 + 64);
    output.extend_from_slice(key.as_bytes());
    output.push(b'=');
    match value {
        ClassicConfigWriteValue::Raw(value) => output.extend_from_slice(value.as_bytes()),
        ClassicConfigWriteValue::Escaped(value) => {
            output.push(b'"');
            for byte in value.bytes() {
                if matches!(byte, b'"' | b'\\') {
                    output.push(b'\\');
                }
                output.push(byte);
            }
            output.push(b'"');
        }
    }
    output
}

fn classic_config_assignment_value<'a>(
    config: &'a [u8],
    section: &str,
    key: &str,
) -> Option<&'a [u8]> {
    let mut in_section = false;
    let mut selected_section = false;
    for line in classic_config_lines(config) {
        let content = &config[line.start..line.content_end];
        if let Some(name) = classic_config_section_name(content) {
            if in_section {
                break;
            }
            let matches = name == section.as_bytes();
            in_section = matches && !selected_section;
            selected_section |= matches;
            continue;
        }
        if !in_section || classic_config_assignment_key(content) != Some(key.as_bytes()) {
            continue;
        }
        let equals = content.iter().position(|byte| *byte == b'=')?;
        return Some(&content[equals + 1..]);
    }
    None
}

fn classic_config_exact_assignment_value<'a>(
    config: &'a [u8],
    section: &str,
    key: &str,
) -> Option<&'a [u8]> {
    let mut in_section = false;
    let mut selected_section = false;
    for line in classic_config_lines(config) {
        let content = &config[line.start..line.content_end];
        if let Some(name) = classic_config_section_name(content) {
            if in_section {
                break;
            }
            let matches = name == section.as_bytes();
            in_section = matches && !selected_section;
            selected_section |= matches;
            continue;
        }
        if !in_section || classic_config_exact_assignment_key(content) != Some(key.as_bytes()) {
            continue;
        }
        let equals = content.iter().position(|byte| *byte == b'=')?;
        return Some(&content[equals + 1..]);
    }
    None
}

fn classic_config_string_value(config: &[u8], section: &str, key: &str) -> Option<Vec<u8>> {
    let value = trim_classic_config_start(classic_config_assignment_value(config, section, key)?);
    if value.first() == Some(&b'"') {
        return decode_classic_config_string(value);
    }

    let comment = value
        .iter()
        .enumerate()
        .find_map(|(index, byte)| {
            (*byte == b'#' && index > 0 && value[index - 1].is_ascii_whitespace()).then_some(index)
        })
        .unwrap_or(value.len());
    Some(trim_classic_config_end(&value[..comment]).to_vec())
}

fn decode_classic_config_string(value: &[u8]) -> Option<Vec<u8>> {
    let mut output = Vec::new();
    let mut cursor = 1;
    while cursor < value.len() {
        match value[cursor] {
            b'"' => return Some(output),
            b'\\' => {
                cursor += 1;
                let escaped = *value.get(cursor)?;
                match escaped {
                    b'a' | b'b' | b'f' | b'n' | b'r' | b't' | b'v' => {
                        output.push(match escaped {
                            b'a' => b'\x07',
                            b'b' => b'\x08',
                            b'f' => b'\x0c',
                            b'n' => b'\n',
                            b'r' => b'\r',
                            b't' => b'\t',
                            _ => b'\x0b',
                        });
                        cursor += 1;
                    }
                    b'x' => {
                        cursor += 1;
                        let start = cursor;
                        let mut decoded = 0u8;
                        while let Some(digit) = value
                            .get(cursor)
                            .and_then(|byte| (*byte as char).to_digit(16).map(|digit| digit as u8))
                        {
                            decoded = decoded.wrapping_mul(16).wrapping_add(digit);
                            cursor += 1;
                        }
                        if cursor == start {
                            output.push(b'x');
                        } else {
                            output.push(decoded);
                        }
                    }
                    b'0'..=b'7' => {
                        let mut decoded = 0u8;
                        while let Some(byte @ b'0'..=b'7') = value.get(cursor).copied() {
                            decoded = decoded.wrapping_mul(8).wrapping_add(byte - b'0');
                            cursor += 1;
                        }
                        output.push(decoded);
                    }
                    other => {
                        output.push(other);
                        cursor += 1;
                    }
                }
            }
            byte => {
                output.push(byte);
                cursor += 1;
            }
        }
    }
    None
}

fn parse_cpp_config_u32(value: &[u8]) -> Option<u32> {
    let value = trim_classic_config_start(value);
    let base = if value.starts_with(b"0x") || value.starts_with(b"0X") {
        16u64
    } else {
        10u64
    };
    let mut cursor = 0;
    let negative = match value.first() {
        Some(b'+') => {
            cursor = 1;
            false
        }
        Some(b'-') => {
            cursor = 1;
            true
        }
        _ => false,
    };
    if base == 16
        && value
            .get(cursor..cursor + 2)
            .is_some_and(|prefix| prefix == b"0x" || prefix == b"0X")
    {
        cursor += 2;
    }

    let start = cursor;
    let mut parsed = 0u64;
    while let Some(digit) = value.get(cursor).and_then(|byte| {
        (*byte as char)
            .to_digit(base as u32)
            .map(|digit| digit as u64)
    }) {
        parsed = parsed.saturating_mul(base).saturating_add(digit);
        cursor += 1;
    }
    if cursor == start {
        return None;
    }
    if negative {
        parsed = 0u64.wrapping_sub(parsed);
    }
    Some(parsed as u32)
}

fn classic_config_lines(config: &[u8]) -> Vec<ClassicConfigLine> {
    let mut lines = Vec::new();
    let mut start = 0;
    while start < config.len() {
        let content_end = config[start..]
            .iter()
            .position(|byte| matches!(byte, b'\r' | b'\n'))
            .map_or(config.len(), |offset| start + offset);
        let end = if content_end == config.len() {
            content_end
        } else if config[content_end] == b'\r' && config.get(content_end + 1) == Some(&b'\n') {
            content_end + 2
        } else {
            content_end + 1
        };
        lines.push(ClassicConfigLine {
            start,
            content_end,
            end,
        });
        start = end;
    }
    lines
}

fn classic_config_section_name(line: &[u8]) -> Option<&[u8]> {
    let line = trim_classic_config_indent(line);
    let mut cursor = 1;
    if line.first() != Some(&b'[') || !line.get(cursor).is_some_and(u8::is_ascii_alphabetic) {
        return None;
    }
    while line
        .get(cursor)
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b' ' | b'_'))
    {
        cursor += 1;
    }
    let name_end = cursor;
    while matches!(line.get(cursor), Some(b' ' | b'\t')) {
        cursor += 1;
    }
    (line.get(cursor) == Some(&b']')).then_some(&line[1..name_end])
}

fn classic_config_assignment_key(line: &[u8]) -> Option<&[u8]> {
    let line = trim_classic_config_start(line);
    if matches!(line.first(), None | Some(b'#' | b';')) {
        return None;
    }
    let equals = line.iter().position(|byte| *byte == b'=')?;
    let key = trim_classic_config(&line[..equals]);
    (!key.is_empty() && key[0].is_ascii_alphabetic()).then_some(key)
}

fn classic_config_exact_assignment_key(line: &[u8]) -> Option<&[u8]> {
    let line = trim_classic_config_indent(line);
    if !line.first().is_some_and(u8::is_ascii_alphabetic) {
        return None;
    }
    let mut cursor = 1;
    while line
        .get(cursor)
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b' ' | b'_'))
    {
        cursor += 1;
    }
    let name_end = cursor;
    while matches!(line.get(cursor), Some(b' ' | b'\t')) {
        cursor += 1;
    }
    (line.get(cursor) == Some(&b'=')).then_some(&line[..name_end])
}

fn classic_config_assignment_suffix(line: &[u8]) -> &[u8] {
    let Some(equals) = line.iter().position(|byte| *byte == b'=') else {
        return &line[line.len()..];
    };
    let value = &line[equals + 1..];
    let value_start = value
        .iter()
        .position(|byte| !matches!(byte, b' ' | b'\t'))
        .unwrap_or(value.len());
    if value.get(value_start) == Some(&b'"') {
        let mut cursor = value_start + 1;
        while cursor < value.len() {
            match value[cursor] {
                b'\\' if cursor + 1 < value.len() => cursor += 2,
                b'"' => return &value[cursor + 1..],
                _ => cursor += 1,
            }
        }
        return &value[value.len()..];
    }

    if let Some(comment) = (value_start..value.len()).find(|index| {
        *index > 0 && value[*index] == b'#' && value[*index - 1].is_ascii_whitespace()
    }) {
        let mut suffix_start = comment;
        while suffix_start > value_start && matches!(value[suffix_start - 1], b' ' | b'\t') {
            suffix_start -= 1;
        }
        return &value[suffix_start..];
    }
    &value[value.len()..]
}

fn classic_config_line_ending(config: &[u8], line: ClassicConfigLine) -> Option<&[u8]> {
    (line.content_end < line.end).then(|| &config[line.content_end..line.end])
}

fn ensure_classic_config_line_break(output: &mut Vec<u8>, line_ending: &[u8]) {
    if !output.is_empty() && !output.ends_with(b"\n") && !output.ends_with(b"\r") {
        output.extend_from_slice(line_ending);
    }
}

fn trim_classic_config(value: &[u8]) -> &[u8] {
    trim_classic_config_end(trim_classic_config_start(value))
}

fn trim_classic_config_indent(mut value: &[u8]) -> &[u8] {
    while matches!(value.first(), Some(b' ' | b'\t')) {
        value = &value[1..];
    }
    value
}

fn trim_classic_config_start(mut value: &[u8]) -> &[u8] {
    while value.first().is_some_and(u8::is_ascii_whitespace) {
        value = &value[1..];
    }
    value
}

fn trim_classic_config_end(mut value: &[u8]) -> &[u8] {
    while value.last().is_some_and(u8::is_ascii_whitespace) {
        value = &value[..value.len() - 1];
    }
    value
}

fn config_override_path() -> Option<PathBuf> {
    env::var_os("LC_CONFIG_FILE")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn explicit_legacy_config_file() -> Option<PathBuf> {
    env::var_os("LC_LEGACY_CONFIG_FILE")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn heuristic_legacy_config_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

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
    apply_headless_display_mode_override_with_io_policy(config_path, logger, false)
}

fn apply_headless_display_mode_override_with_io_policy(
    config_path: &Path,
    logger: &LauncherLogger,
    fail_on_io_error: bool,
) -> Result<()> {
    let Some(reason) = headless_override_reason() else {
        return Ok(());
    };

    let config = match Config::load(config_path) {
        Ok(config) => config,
        Err(err) if err.kind() == io::ErrorKind::NotFound => Config::new(),
        Err(err) => {
            if fail_on_io_error {
                return Err(err)
                    .with_context(|| format!("failed to load {}", config_path.display()));
            }
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
    let native_config = match fs::read(config_path) {
        Ok(config) => config,
        Err(err) if err.kind() == io::ErrorKind::NotFound && !fail_on_io_error => Vec::new(),
        Err(err) => {
            if fail_on_io_error {
                return Err(err)
                    .with_context(|| format!("failed to read {}", config_path.display()));
            }
            logger
                .log_line(&format!(
                    "headless display guard failed to read {}: {err}",
                    config_path.display()
                ))
                .ok();
            return Ok(());
        }
    };
    let native_config = update_classic_config_value(
        &native_config,
        "Graphics",
        "DisplayMode",
        ClassicConfigWriteValue::Raw("1"),
    );

    if let Err(err) = fs::write(config_path, native_config) {
        if fail_on_io_error {
            return Err(err)
                .with_context(|| format!("failed to persist {}", config_path.display()));
        }
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
    fn l014_headless_override_preserves_native_adaptation() {
        let _guard = EnvGuard::set(&[
            (FORCE_WINDOW_ENV, Some("1")),
            (FORCE_FULLSCREEN_ENV, None),
            (DISABLE_HEADLESS_GUARD_ENV, None),
            ("LC_HEADLESS", None),
            ("SDL_VIDEODRIVER", None),
        ]);
        let temp = TempDir::new().unwrap();
        let config_path = temp.path().join("config.cfg");
        std::fs::write(
            &config_path,
            "[General]\nVersion=347 # keep version note\n[Graphics]\nDisplayMode=0\n",
        )
        .unwrap();
        let logger = test_logger(&temp);
        adapt_config_to_current_version(&config_path, &logger).unwrap();
        apply_headless_display_mode_override(&config_path, &logger).unwrap();
        let cfg = Config::load(&config_path).unwrap();
        assert_eq!(
            cfg.get_in(Some("General"), "Version"),
            Some("362 # keep version note")
        );
        assert_eq!(cfg.get_in(Some("Graphics"), "DisplayMode"), Some("1"));
        let native = std::fs::read_to_string(&config_path).unwrap();
        assert!(native.contains("Version=362 # keep version note\n"));
        assert!(native.contains("DisplayMode=1\n"));
        assert!(!native.contains("Version ="));
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

        let log_path = logs_dir.join(format!("clonk-game-{}.log", timestamp_for_filename()));
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
    let Some(patcher) = locate_update_tool(install_root) else {
        logger
            .log_line(
                "optional c4group updater not found; continuing without legacy update support",
            )
            .context("failed to log optional updater availability")?;
        return Ok(());
    };

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

fn locate_update_tool(install_root: &Path) -> Option<PathBuf> {
    candidate_patcher_paths(install_root)
        .into_iter()
        .find(|candidate| candidate.exists())
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
                    .unwrap_or("ClonkRust-crash.dmp");
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
    use crate::legacy_registry::{LegacyRegistryData, LegacyRegistryKey, LegacyRegistryValue};
    use clonk_launcher::{
        ProviderAutomationRecord, ProviderAutomationSnapshot, ProviderAutomationState,
        ProviderBulkRetargetRecord, ProviderBulkRetargetSummary, ProviderPathStatus,
    };
    use serde_json::Value;
    use std::cell::Cell;
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
            .join("clonk-app.app")
            .join("Contents")
            .join("MacOS")
            .join("clonk-app");
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

        let binary_path = install_dir.path().join("clonk-app.exe");
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
    fn l005_prepare_config_and_app_paths_use_the_same_override_file() {
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

        let config_path = prepare_config_with_registry_reader(&paths, &logger, || {
            panic!("LC_CONFIG_FILE must bypass legacy registry migration")
        })
        .expect("config preparation should succeed");

        assert_eq!(paths.config_file(), override_path);
        assert_eq!(config_path, override_path);
        assert!(config_path.exists(), "override config file should exist");
        assert!(
            fs::metadata(&config_path).unwrap().is_file(),
            "override config path should resolve to a file"
        );
    }

    #[test]
    fn l025_default_config_imports_windows_registry_once() {
        fn value(name: &str, data: LegacyRegistryData) -> LegacyRegistryValue {
            LegacyRegistryValue {
                name: name.to_string(),
                data,
            }
        }

        let install_dir = TempDir::new().unwrap();
        fs::create_dir_all(install_dir.path().join("planet")).unwrap();
        fs::write(install_dir.path().join("planet/System.c4g"), b"system").unwrap();
        let user_dir = TempDir::new().unwrap();
        let legacy_home = TempDir::new().unwrap();
        let log_dir = TempDir::new().unwrap();
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.path())),
            ("LC_CONFIG_FILE", None),
            ("LC_LEGACY_CONFIG_FILE", None),
            ("HOME", Some(legacy_home.path())),
            ("APPDATA", Some(legacy_home.path())),
            ("LC_GAME_DISABLE_HEADLESS_GUARD", Some(Path::new("1"))),
        ]);

        let heuristic_path = heuristic_legacy_config_candidates()
            .into_iter()
            .next()
            .expect("the host platform should expose a legacy config candidate");
        fs::create_dir_all(heuristic_path.parent().unwrap()).unwrap();
        fs::write(heuristic_path, b"[General]\nVersion=362\nName=Heuristic\n").unwrap();

        let paths = AppPaths::discover().unwrap();
        paths.ensure_user_dirs().unwrap();
        let logger = test_logger(&log_dir);
        let registry = LegacyRegistryConfig {
            keys: vec![
                LegacyRegistryKey {
                    path: vec!["General".to_string()],
                    values: vec![
                        value(
                            "Version",
                            LegacyRegistryData::Dword(362_u32.to_le_bytes().to_vec()),
                        ),
                        value("Name", LegacyRegistryData::String(b"M\xfcller\0".to_vec())),
                        value(
                            "GamepadEnabled",
                            LegacyRegistryData::Dword(0_u32.to_le_bytes().to_vec()),
                        ),
                    ],
                },
                LegacyRegistryKey {
                    path: vec!["Graphics".to_string()],
                    values: vec![
                        value(
                            "ResolutionX",
                            LegacyRegistryData::Dword(1920_u32.to_le_bytes().to_vec()),
                        ),
                        value(
                            "DisplayMode",
                            LegacyRegistryData::String(b"Window\0".to_vec()),
                        ),
                    ],
                },
                LegacyRegistryKey {
                    path: vec!["Gamepad0".to_string()],
                    values: vec![value(
                        "Button1",
                        LegacyRegistryData::Dword(u32::MAX.to_le_bytes().to_vec()),
                    )],
                },
                LegacyRegistryKey {
                    path: vec!["Network".to_string()],
                    values: vec![value(
                        "LastUpdateTime",
                        LegacyRegistryData::Qword(0x1122_3344_5566_7788_u64.to_le_bytes().to_vec()),
                    )],
                },
                LegacyRegistryKey {
                    path: vec!["Logging".to_string(), "C4AudioSystem".to_string()],
                    values: vec![value(
                        "LogLevel",
                        LegacyRegistryData::String(b"debug\0".to_vec()),
                    )],
                },
                LegacyRegistryKey {
                    path: vec!["Console".to_string()],
                    values: vec![value(
                        "Unrelated",
                        LegacyRegistryData::String(b"ignored\0".to_vec()),
                    )],
                },
            ],
        };
        let reads = Cell::new(0);

        let config_path = prepare_config_with_registry_reader(&paths, &logger, || {
            reads.set(reads.get() + 1);
            Ok(Some(registry))
        })
        .expect("first run should import the registry snapshot");

        assert_eq!(reads.get(), 1);
        let imported_bytes = fs::read(&config_path).unwrap();
        let imported = Config::load(&config_path).unwrap();
        assert_eq!(imported.get_in(Some("General"), "Version"), Some("362"));
        assert_eq!(imported.get_in(Some("General"), "Name"), Some("Müller"));
        assert_eq!(
            imported.get_in(Some("General"), "GamepadEnabled"),
            Some("false")
        );
        assert_eq!(
            imported.get_in(Some("Graphics"), "ResolutionX"),
            Some("1920")
        );
        assert_eq!(
            imported.get_in(Some("Graphics"), "DisplayMode"),
            Some("Window")
        );
        assert_eq!(imported.get_in(Some("Gamepad0"), "Button1"), Some("-1"));
        assert_eq!(
            imported.get_in(Some("Network"), "LastUpdateTime"),
            Some("1234605616436508552")
        );
        assert!(imported_bytes
            .windows(b"  [C4AudioSystem]\r\n  LogLevel=debug\r\n".len())
            .any(|window| window == b"  [C4AudioSystem]\r\n  LogLevel=debug\r\n"));
        assert!(!imported_bytes
            .windows(b"[Console]".len())
            .any(|window| window == b"[Console]"));

        let second_path = prepare_config_with_registry_reader(&paths, &logger, || {
            panic!("an existing destination must not read the legacy registry")
        })
        .expect("later runs should keep the imported destination");
        assert_eq!(second_path, config_path);
        assert_eq!(fs::read(second_path).unwrap(), imported_bytes);
    }

    #[test]
    fn prepare_config_repairs_urls_truncated_by_the_old_rust_parser() {
        // Before the std-config URL fix, saving an unquoted C++ URL treated
        // `//` as a comment and persisted only `https:`. Repair that specific
        // Rust-port corruption before handing configuration to the now
        // C++-faithful network client.
        let install_dir = TempDir::new().unwrap();
        let planet_dir = install_dir.path().join("planet");
        fs::create_dir_all(&planet_dir).unwrap();
        fs::write(planet_dir.join("System.c4g"), b"system payload").unwrap();

        let user_dir = TempDir::new().unwrap();
        let log_dir = TempDir::new().unwrap();
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.path())),
            ("LC_CONFIG_FILE", None),
            ("LC_LEGACY_CONFIG_FILE", None),
            ("LC_GAME_DISABLE_HEADLESS_GUARD", Some(Path::new("1"))),
        ]);

        let paths = AppPaths::discover().unwrap();
        paths.ensure_user_dirs().unwrap();
        fs::write(
            paths.config_file(),
            "[Network]\nServerAddress = https:\nAlternateServerAddress = https:\n",
        )
        .unwrap();
        let logger = test_logger(&log_dir);

        prepare_config(&paths, &logger).unwrap();

        let config = Config::load(paths.config_file()).unwrap();
        assert_eq!(
            config.get_in(Some("Network"), "ServerAddress"),
            Some("https://league.clonkspot.org")
        );
        assert_eq!(
            config.get_in(Some("Network"), "AlternateServerAddress"),
            Some("https://league.clonkspot.org")
        );
    }

    #[test]
    fn l014_prepare_config_applies_classic_version_347_migrations() {
        let install_dir = TempDir::new().unwrap();
        let planet_dir = install_dir.path().join("planet");
        fs::create_dir_all(&planet_dir).unwrap();
        fs::write(planet_dir.join("System.c4g"), b"system payload").unwrap();

        let user_dir = TempDir::new().unwrap();
        let legacy_dir = TempDir::new().unwrap();
        let legacy_path = legacy_dir.path().join("Legacy.cfg");
        let legacy = b"# preserve this comment\r\n[General]\r\nVersion=347\r\nName=Sentinel\r\n[Network]\r\nServerAddress=league.clonkspot.org:80\r\nAlternateServerAddress=league.clonkspot.org:80\r\nUpdateServerAddress=update.clonkspot.org/lc/update\r\nPuncherAddress=clonk.de:11115\r\n[Graphics]\r\n  Shader=false # keep shader note\r\nDisableGamma=true\r\n[Sound]\r\nMaxChannels=7\r\nMusic=false\r\n";
        fs::write(&legacy_path, legacy).unwrap();
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

        let config_path = prepare_config_with_registry_reader(&paths, &logger, || {
            panic!("LC_LEGACY_CONFIG_FILE must bypass legacy registry migration")
        })
        .expect("config preparation should succeed");

        assert_eq!(config_path, paths.config_file());
        assert!(
            config_path.exists(),
            "default config path should be materialised"
        );
        let config = Config::load(&config_path).expect("migrated config should be readable");
        assert_eq!(config.get_in(Some("General"), "Version"), Some("362"));
        assert_eq!(config.get_in(Some("General"), "Name"), Some("Sentinel"));
        assert_eq!(
            config.get_in(Some("Network"), "ServerAddress"),
            Some(OFFICIAL_LEAGUE_SERVER)
        );
        assert_eq!(
            config.get_in(Some("Network"), "AlternateServerAddress"),
            Some(OFFICIAL_LEAGUE_SERVER)
        );
        assert_eq!(
            config.get_in(Some("Network"), "UpdateServerAddress"),
            Some(OFFICIAL_UPDATE_SERVER)
        );
        assert_eq!(
            config.get_in(Some("Network"), "PuncherAddress"),
            Some(OFFICIAL_PUNCHER_SERVER)
        );
        assert_eq!(
            config.get_in(Some("Graphics"), "Shader"),
            Some("true # keep shader note")
        );
        assert_eq!(
            config.get_in(Some("Graphics"), "DisableGamma"),
            Some("false")
        );
        assert_eq!(config.get_in(Some("Sound"), "MaxChannels"), Some("1024"));
        assert_eq!(config.get_in(Some("Sound"), "Music"), Some("true"));
        assert_eq!(fs::read(&legacy_path).unwrap(), legacy);

        let first_adaptation = fs::read(&config_path).unwrap();
        let first_adaptation_text = String::from_utf8(first_adaptation.clone()).unwrap();
        assert!(first_adaptation_text.contains("Version=362\r\n"));
        assert!(!first_adaptation_text.contains("Version ="));
        assert!(
            first_adaptation_text.contains("ServerAddress=\"https://league.clonkspot.org\"\r\n")
        );
        assert!(first_adaptation_text.contains("  Shader=true # keep shader note\r\n"));
        assert!(first_adaptation_text.contains("DisableGamma=false\r\n"));
        assert!(first_adaptation_text.contains("Name=Sentinel\r\n"));
        assert!(first_adaptation_text.starts_with("# preserve this comment\r\n"));
        prepare_config(&paths, &logger).expect("current config should remain valid");
        assert_eq!(
            fs::read(&config_path).unwrap(),
            first_adaptation,
            "Version=362 should make adaptation byte-stable"
        );
    }

    #[test]
    fn l014_config_migrations_keep_cpp_exact_version_gates() {
        let temp = TempDir::new().unwrap();
        let config_path = temp.path().join("config.cfg");
        let logger = test_logger(&temp);

        fs::write(
            &config_path,
            "[General]\nVersion=0x168junk\n[Network]\nServerAddress=league.clonkspot.org:80\nPuncherAddress=clonk.de:11115\n[Graphics]\nShader=false\nDisableGamma=true\n[Sound]\nMaxChannels=7\nMusic=false\n",
        )
        .unwrap();
        adapt_config_to_current_version(&config_path, &logger).unwrap();
        let config = Config::load(&config_path).unwrap();
        assert_eq!(config.get_in(Some("General"), "Version"), Some("362"));
        assert_eq!(
            config.get_in(Some("Network"), "ServerAddress"),
            Some("league.clonkspot.org:80")
        );
        assert_eq!(
            config.get_in(Some("Network"), "PuncherAddress"),
            Some("clonk.de:11115")
        );
        assert_eq!(config.get_in(Some("Graphics"), "Shader"), Some("false"));
        assert_eq!(
            config.get_in(Some("Graphics"), "DisableGamma"),
            Some("true")
        );
        assert_eq!(config.get_in(Some("Sound"), "MaxChannels"), Some("7"));
        assert_eq!(config.get_in(Some("Sound"), "Music"), Some("false"));

        fs::write(
            &config_path,
            "[General]\nVersion =360\n[Sound]\nMaxChannels=7\nMusic=false\n",
        )
        .unwrap();
        adapt_config_to_current_version(&config_path, &logger).unwrap();
        let config = Config::load(&config_path).unwrap();
        assert_eq!(config.get_in(Some("General"), "Version"), Some("362"));
        assert_eq!(config.get_in(Some("Sound"), "MaxChannels"), Some("1024"));
        assert_eq!(config.get_in(Some("Sound"), "Music"), Some("true"));
        assert!(fs::read_to_string(&config_path)
            .unwrap()
            .contains("Version=362\n"));

        fs::write(
            &config_path,
            "[General]\nVersion=346\n[Sound]\nMaxChannels=7\nMusic=false\n",
        )
        .unwrap();
        adapt_config_to_current_version(&config_path, &logger).unwrap();
        let config = Config::load(&config_path).unwrap();
        assert_eq!(config.get_in(Some("Sound"), "MaxChannels"), Some("7"));
        assert_eq!(config.get_in(Some("Sound"), "Music"), Some("true"));

        fs::write(
            &config_path,
            "[General]\nVersion=999\n[Graphics]\nShader=false\nDisableGamma=true\n",
        )
        .unwrap();
        adapt_config_to_current_version(&config_path, &logger).unwrap();
        let config = Config::load(&config_path).unwrap();
        assert_eq!(config.get_in(Some("General"), "Version"), Some("362"));
        assert_eq!(config.get_in(Some("Graphics"), "Shader"), Some("false"));
        assert_eq!(
            config.get_in(Some("Graphics"), "DisableGamma"),
            Some("true")
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn l014_macos_version_349_disables_preloading() {
        let temp = TempDir::new().unwrap();
        let config_path = temp.path().join("config.cfg");
        fs::write(&config_path, "[General]\nVersion=349\nPreloading=true\n").unwrap();
        let logger = test_logger(&temp);

        adapt_config_to_current_version(&config_path, &logger).unwrap();

        let config = Config::load(&config_path).unwrap();
        assert_eq!(config.get_in(Some("General"), "Version"), Some("362"));
        assert_eq!(config.get_in(Some("General"), "Preloading"), Some("false"));
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn l016_migrated_config_user_path_applies_before_runtime_launch() {
        let install_dir = TempDir::new().unwrap();
        let planet_dir = install_dir.path().join("planet");
        fs::create_dir_all(&planet_dir).unwrap();
        fs::write(planet_dir.join("System.c4g"), b"system payload").unwrap();

        let home_dir = TempDir::new().unwrap();
        let legacy_dir = TempDir::new().unwrap();
        let legacy_path = legacy_dir.path().join("Legacy.cfg");
        fs::write(
            &legacy_path,
            b"[General]\nUserPath=\"$HOME/Relocated Data\"\n",
        )
        .unwrap();
        let configured_user = home_dir.path().join("Relocated Data");
        let log_dir = TempDir::new().unwrap();

        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", None),
            ("LC_CONFIG_FILE", None),
            ("LC_LEGACY_CONFIG_FILE", Some(legacy_path.as_path())),
            ("LC_CACHE_DIR", None),
            ("LC_LOGS_DIR", None),
            ("XDG_DATA_HOME", None),
            ("HOME", Some(home_dir.path())),
            ("LC_GAME_DISABLE_HEADLESS_GUARD", Some(Path::new("1"))),
        ]);

        let bootstrap_paths = AppPaths::discover().unwrap();
        assert_ne!(bootstrap_paths.user_data_dir(), configured_user);
        bootstrap_paths.ensure_user_dirs().unwrap();
        let logger = test_logger(&log_dir);
        let config_path = prepare_config(&bootstrap_paths, &logger).unwrap();

        let runtime_paths = rediscover_paths_after_config(&config_path).unwrap();

        assert_eq!(runtime_paths.config_file(), config_path);
        assert_eq!(runtime_paths.user_data_dir(), configured_user);
        assert_eq!(runtime_paths.cache_dir(), configured_user.join("Cache"));
        assert_eq!(runtime_paths.logs_dir(), configured_user.join("Logs"));
        assert!(configured_user.join("Config").is_dir());
    }

    #[test]
    fn candidate_list_covers_macos_bundle() {
        let temp = TempDir::new().unwrap();
        let app_dir = temp
            .path()
            .join("build")
            .join("clonk-app.app")
            .join("Contents")
            .join("MacOS");
        fs::create_dir_all(&app_dir).unwrap();
        let binary = app_dir.join("clonk-app");
        fs::write(&binary, b"stub").unwrap();
        let resolved =
            resolve_runtime_binary(None, temp.path()).expect("should locate bundle binary");
        assert_eq!(resolved, binary);
    }

    #[test]
    fn packaged_launcher_resolves_sibling_runtime_from_bin_directory() {
        let install_dir = TempDir::new().unwrap();
        let binary = install_dir.path().join("bin").join("clonk-app");
        fs::create_dir_all(binary.parent().unwrap()).unwrap();
        fs::write(&binary, b"stub").unwrap();

        let resolved =
            resolve_runtime_binary(None, install_dir.path()).expect("locate packaged runtime");

        assert_eq!(resolved, binary);
    }

    #[test]
    fn respects_override_argument() {
        let temp = TempDir::new().unwrap();
        let override_bin = temp.path().join("custom").join("clonk-app");
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
    fn validate_update_tool_allows_missing_optional_binary() {
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

        validate_update_tool(&paths, &logger).unwrap();
        let log_contents = std::fs::read_to_string(logger.path()).unwrap();
        assert!(
            log_contents.contains(
                "optional c4group updater not found; continuing without legacy update support"
            ),
            "optional updater notice missing in log:\n{log_contents}"
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
            .join("ClonkRust-crash-2024-01-01-00-00-00.dmp");
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

        let summary =
            digest_update_telemetry(std::slice::from_ref(&telemetry_log), &logger).unwrap();
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
        let crash_log = paths.logs_dir().join("ClonkRust-crash-2024.dmp");
        fs::write(&crash_log, "crash dump payload").unwrap();

        let mut telemetry = UpdateTelemetrySummary::default();
        telemetry.record_success(runtime_log.clone());
        telemetry.record_failure(runtime_log.clone(), "c4group returned status 0".into());

        let bundle_path = create_support_bundle(
            &paths,
            &logger,
            logger.path(),
            std::slice::from_ref(&runtime_log),
            std::slice::from_ref(&crash_log),
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
                .any(|name| name.starts_with("crash/01_ClonkRust-crash-2024.dmp")),
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
        let crash_log = paths.logs_dir().join("ClonkRust-crash-summary.dmp");
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
            std::slice::from_ref(&runtime_log),
            std::slice::from_ref(&crash_log),
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
                .starts_with("clonk-game"),
            "summary should include launcher log path"
        );
        assert_eq!(
            value["runtime_logs"][0].as_str(),
            Some("Clonk-summary.log"),
            "summary should include runtime log relative path"
        );
        assert_eq!(
            value["crash_reports"][0].as_str(),
            Some("ClonkRust-crash-summary.dmp"),
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
        let crash_log = paths.logs_dir().join("ClonkRust-crash-regenerate.dmp");
        fs::write(&crash_log, "crash dump payload").unwrap();

        let mut telemetry = UpdateTelemetrySummary::default();
        telemetry.record_success(runtime_log.clone());
        telemetry.record_failure(runtime_log.clone(), "c4group returned status 1".into());

        write_launcher_summary(
            &paths,
            &logger,
            logger.path(),
            std::slice::from_ref(&runtime_log),
            std::slice::from_ref(&crash_log),
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
        let search_preferences = clonk_launcher::ReportSearchPreferences {
            query: "support".into(),
            highlight: clonk_launcher::ReportSearchHighlightPreference::Generic,
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
