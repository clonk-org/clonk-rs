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
use clonk_platform::{discover_unvalidated_install_root, AppPaths};
use clonk_update::{
    acquire_install_use, apply_update, resume_interrupted_update, ApplyOutcome, ApplyPlan,
    InstallLayout, PlatformOps, RealPlatform, ResumeOutcome,
};
use legacy_registry::{
    read_legacy_windows_registry, serialize_legacy_registry_config, LegacyRegistryConfig,
};
use serde::{Deserialize, Serialize};

const SKIP_PATCHER_VALIDATION_ENV: &str = "LC_GAME_SKIP_PATCHER_CHECK";
const LAUNCHER_PID_ENV: &str = "LC_GAME_LAUNCHER_PID";
const UPDATE_NOTICE_ENV: &str = "LC_GAME_UPDATE_NOTICE";
const UPDATE_RESULT_FILE_NAME: &str = "update-result.json";
const UPDATE_OWNER_FILE_PREFIX: &str = ".owner-";
const UPDATE_RESULT_SCHEMA: u32 = 1;
const UPDATE_PROCESS_WAIT_SECONDS: u64 = 120;
const UPDATE_CLEANUP_ATTEMPTS: usize = 100;
const FORCE_WINDOW_ENV: &str = "LC_GAME_FORCE_WINDOW";
const FORCE_FULLSCREEN_ENV: &str = "LC_GAME_FORCE_FULLSCREEN";
const DISABLE_HEADLESS_GUARD_ENV: &str = "LC_GAME_DISABLE_HEADLESS_GUARD";
const LEGACY_LOG_PREFIX: &str = "Clonk";
const LEGACY_LOG_SUFFIX: &str = ".log";
const CRASH_ARTIFACT_MARKER: &str = "-crash-";
const OFFICIAL_LEAGUE_SERVER: &str = "https://league.clonkspot.org";
const OFFICIAL_UPDATE_SERVER: &str = "https://update.clonkspot.org/lc/update";
const OFFICIAL_PUNCHER_SERVER: &str = "netpuncher.openclonk.org:11115";
const CLASSIC_CONFIG_VERSION: u32 = clonk_core::version::ENGINE_VERSION[4] as u32;
const CLASSIC_CONFIG_VERSION_VALUE: &str = "362";
const CLASSIC_UNVERSIONED_CONFIG_VERSION: u32 = 347;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct UpdateResultDocument {
    schema: u32,
    #[serde(flatten)]
    status: UpdateResultStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
enum UpdateResultStatus {
    Applied {
        version: String,
        components: Vec<String>,
    },
    Failed {
        message: String,
    },
}

#[derive(Debug, Parser)]
#[command(
    name = "clonk-game",
    about = "Clonk Rust launcher that runs the Rust runtime",
    version,
    author
)]
struct Cli {
    /// Apply a downloaded update plan instead of launching the runtime
    #[arg(
        long = "apply-update",
        value_name = "PLAN",
        requires_all = ["install_root", "relaunch"],
        conflicts_with_all = [
            "finish_update",
            "binary",
            "support_bundle_only",
            "automation_report",
            "forwarded"
        ]
    )]
    apply_update: Option<PathBuf>,

    /// Installation the update plan replaces
    #[arg(long = "install-root", value_name = "PATH", requires = "apply_update")]
    install_root: Option<PathBuf>,

    /// Process that must exit before the update may replace installed files
    #[arg(
        long = "wait-pid",
        value_name = "PID",
        requires = "apply_update",
        value_parser = clap::value_parser!(u32).range(1..)
    )]
    wait_pids: Vec<u32>,

    /// Start the updated launcher after applying the plan
    #[arg(long, requires = "apply_update")]
    relaunch: bool,

    /// Complete cleanup after a detached update helper exits
    #[arg(
        long = "finish-update",
        value_name = "PENDING_DIR",
        hide = true,
        requires_all = ["update_result", "update_helper_pid"],
        conflicts_with_all = [
            "apply_update",
            "install_root",
            "wait_pids",
            "relaunch",
            "binary",
            "support_bundle_only",
            "automation_report",
            "forwarded"
        ]
    )]
    finish_update: Option<PathBuf>,

    /// Typed result written by the detached update helper
    #[arg(long, value_name = "RESULT", hide = true, requires = "finish_update")]
    update_result: Option<PathBuf>,

    /// Detached helper that must exit before its staging directory is removed
    #[arg(
        long,
        value_name = "PID",
        hide = true,
        requires = "finish_update",
        value_parser = clap::value_parser!(u32).range(1..)
    )]
    update_helper_pid: Option<u32>,

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
    clonk_logging::install_panic_hook();

    if let Err(error) = run() {
        tracing::error!(error = ?error, "clonk-game encountered an error");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    run_cli(Cli::parse())
}

fn run_cli(cli: Cli) -> Result<()> {
    let Cli {
        apply_update,
        install_root,
        wait_pids,
        relaunch,
        finish_update,
        update_result,
        update_helper_pid,
        binary,
        support_bundle_only,
        automation_report,
        forwarded,
    } = cli;

    if let Some(plan_path) = apply_update {
        let install_root = install_root
            .ok_or_else(|| anyhow!("--apply-update requires an explicit --install-root"))?;
        apply_update_plan(
            &plan_path,
            &install_root,
            &wait_pids,
            relaunch,
            &RealPlatform,
        )?;
        return Ok(());
    }

    let update_recovery = recover_interrupted_update_before_path_discovery()
        .context("failed to recover interrupted component update")?;
    let paths = AppPaths::discover().context("failed to discover application paths")?;
    let _install_use = acquire_install_use(&InstallLayout::for_app_paths(&paths))
        .context("the installation is being updated by another process")?;
    paths
        .ensure_user_dirs()
        .context("failed to prepare user directories")?;

    let update_notice = match finish_update {
        Some(pending_dir) => {
            let result_path =
                update_result.ok_or_else(|| anyhow!("--finish-update requires --update-result"))?;
            let helper_pid = update_helper_pid
                .ok_or_else(|| anyhow!("--finish-update requires --update-helper-pid"))?;
            finish_update_mode(
                &paths,
                &pending_dir,
                &result_path,
                helper_pid,
                &RealPlatform,
            )?
        }
        None => recover_abandoned_pending_updates(&paths, &RealPlatform),
    };

    let logger = LauncherLogger::new(&paths).context("failed to initialise launcher logging")?;
    logger
        .log_line("launcher initialised")
        .context("failed to write initial log entry")?;
    log_update_recovery(&update_recovery, &logger)?;

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
    let status = launch_runtime(
        &binary,
        &paths,
        &config_path,
        &forwarded,
        update_notice.as_deref(),
        &logger,
    )?;
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

    #[cfg(target_os = "macos")]
    let target_roots = macos_bundle_external_runtime_root(binary_dir)
        .map(|root| vec![root])
        .unwrap_or_else(|| ordinary_runtime_asset_roots(paths.install_root(), binary_dir));
    #[cfg(not(target_os = "macos"))]
    let target_roots = ordinary_runtime_asset_roots(paths.install_root(), binary_dir);

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

fn ordinary_runtime_asset_roots(install_root: &Path, binary_dir: &Path) -> Vec<PathBuf> {
    let mut roots = vec![install_root.to_path_buf()];
    if roots.iter().all(|root| root != binary_dir) {
        roots.push(binary_dir.to_path_buf());
    }
    roots
}

#[cfg(target_os = "macos")]
fn macos_bundle_external_runtime_root(binary_dir: &Path) -> Option<PathBuf> {
    let contents = (binary_dir.file_name()? == "MacOS")
        .then(|| binary_dir.parent())
        .flatten()?;
    let bundle = (contents.file_name()? == "Contents")
        .then(|| contents.parent())
        .flatten()?;
    (bundle.extension()?.eq_ignore_ascii_case("app"))
        .then(|| bundle.parent().map(Path::to_path_buf))
        .flatten()
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

    copy_runtime_asset(source, target).with_context(|| {
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

/// Copies a runtime asset that may be a C4Group *directory* such as
/// `planet/System.c4g`.
///
/// `fs::copy` fails on a directory, and Windows reaches this fallback for every
/// group: it cannot hard link a directory and the symlink recovery above is
/// `#[cfg(unix)]`.
fn copy_runtime_asset(source: &Path, target: &Path) -> io::Result<()> {
    if !source.is_dir() {
        return fs::copy(source, target).map(|_| ());
    }

    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        copy_runtime_asset(&entry.path(), &target.join(entry.file_name()))?;
    }
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
    update_notice: Option<&str>,
    logger: &LauncherLogger,
) -> Result<ExitStatus> {
    let mut command = runtime_command(binary, paths, config_path, forwarded, update_notice);

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

fn runtime_command(
    binary: &Path,
    paths: &AppPaths,
    config_path: &Path,
    forwarded: &[OsString],
    update_notice: Option<&str>,
) -> Command {
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
    command.env(LAUNCHER_PID_ENV, std::process::id().to_string());
    command.env(clonk_update::UPDATE_RECOVERY_COMPLETE_ENV, "1");
    match update_notice {
        Some(notice) => command.env(UPDATE_NOTICE_ENV, notice),
        None => command.env_remove(UPDATE_NOTICE_ENV),
    };

    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    command
}

fn recover_interrupted_update_before_path_discovery() -> Result<ResumeOutcome> {
    let install_root = discover_unvalidated_install_root()
        .context("failed to locate the installation for update recovery")?;
    resume_interrupted_update(&install_root).map_err(Into::into)
}

fn log_update_recovery(outcome: &ResumeOutcome, logger: &LauncherLogger) -> Result<()> {
    let message = match outcome {
        ResumeOutcome::NothingToDo => "no interrupted component update to recover".to_string(),
        ResumeOutcome::RolledForward { version } => {
            format!("completed interrupted component update to {version}")
        }
        ResumeOutcome::RolledBack { version } => {
            format!("rolled back interrupted component update to {version}")
        }
    };
    logger
        .log_line(&message)
        .context("failed to log component update recovery")
}

fn apply_update_plan(
    plan_path: &Path,
    install_root: &Path,
    wait_pids: &[u32],
    relaunch: bool,
    platform: &dyn PlatformOps,
) -> Result<ApplyOutcome> {
    apply_update_plan_with_relauncher(
        plan_path,
        install_root,
        wait_pids,
        relaunch,
        platform,
        std::process::id(),
        relaunch_updated_game,
    )
}

fn apply_update_plan_with_relauncher<F>(
    plan_path: &Path,
    install_root: &Path,
    wait_pids: &[u32],
    relaunch: bool,
    platform: &dyn PlatformOps,
    helper_pid: u32,
    relauncher: F,
) -> Result<ApplyOutcome>
where
    F: FnOnce(&InstallLayout, &Path, &Path, u32) -> Result<()>,
{
    let pending_dir = plan_path.parent().ok_or_else(|| {
        anyhow!(
            "update plan {} has no parent directory",
            plan_path.display()
        )
    })?;
    let result_path = pending_dir.join(UPDATE_RESULT_FILE_NAME);
    let layout = InstallLayout::discover(install_root);
    record_update_owner(pending_dir, helper_pid)
        .context("failed to record the active update helper")?;
    let mut first_wait_error = None;
    for pid in wait_pids {
        if let Err(error) = platform.wait_for_process(
            *pid,
            std::time::Duration::from_secs(UPDATE_PROCESS_WAIT_SECONDS),
        ) {
            first_wait_error.get_or_insert_with(|| {
                anyhow::Error::new(error).context(format!("failed while waiting for process {pid}"))
            });
        }
    }
    if let Some(error) = first_wait_error {
        let result = UpdateResultDocument {
            schema: UPDATE_RESULT_SCHEMA,
            status: UpdateResultStatus::Failed {
                message: format!("{error:#}"),
            },
        };
        if let Err(write_error) = write_update_result(&result_path, &result) {
            tracing::warn!(error = ?write_error, "could not write failed update result");
        }
        return Err(error);
    }

    let operation = (|| {
        let plan_bytes = fs::read(plan_path)
            .with_context(|| format!("failed to read update plan {}", plan_path.display()))?;
        let plan: ApplyPlan = serde_json::from_slice(&plan_bytes)
            .with_context(|| format!("failed to parse update plan {}", plan_path.display()))?;
        apply_update(&layout, &plan, platform)
            .with_context(|| format!("failed to apply update plan {}", plan_path.display()))
    })();
    let result = match &operation {
        Ok(outcome) => UpdateResultDocument {
            schema: UPDATE_RESULT_SCHEMA,
            status: UpdateResultStatus::Applied {
                version: outcome.version.clone(),
                components: outcome.applied.clone(),
            },
        },
        Err(error) => UpdateResultDocument {
            schema: UPDATE_RESULT_SCHEMA,
            status: UpdateResultStatus::Failed {
                message: format!("{error:#}"),
            },
        },
    };
    let write_result = write_update_result(&result_path, &result);
    let relaunch_result = match relaunch {
        true => relauncher(&layout, pending_dir, &result_path, helper_pid)
            .context("failed to start the updated launcher"),
        false => Ok(()),
    };

    match operation {
        Err(error) => {
            if let Err(write_error) = write_result {
                tracing::warn!(error = ?write_error, "could not write failed update result");
            }
            if let Err(relaunch_error) = relaunch_result {
                tracing::warn!(error = ?relaunch_error, "could not relaunch after failed update");
            }
            Err(error)
        }
        Ok(outcome) => {
            write_result?;
            relaunch_result?;
            Ok(outcome)
        }
    }
}

fn write_update_result(path: &Path, result: &UpdateResultDocument) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(result).context("failed to serialize update result")?;
    let mut file = File::create(path)
        .with_context(|| format!("failed to create update result {}", path.display()))?;
    file.write_all(&bytes)
        .and_then(|()| file.write_all(b"\n"))
        .and_then(|()| file.sync_all())
        .with_context(|| format!("failed to write update result {}", path.display()))
}

fn record_update_owner(directory: &Path, pid: u32) -> Result<()> {
    if pid == 0 {
        bail!("an update staging owner must have a nonzero process ID");
    }
    let path = directory.join(format!("{UPDATE_OWNER_FILE_PREFIX}{pid}"));
    match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(file) => file
            .sync_all()
            .with_context(|| format!("failed to persist update owner {}", path.display())),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let metadata = fs::symlink_metadata(&path)
                .with_context(|| format!("failed to inspect update owner {}", path.display()))?;
            if metadata.file_type().is_file() {
                Ok(())
            } else {
                bail!("update owner {} is not a regular file", path.display())
            }
        }
        Err(error) => {
            Err(error).with_context(|| format!("failed to record update owner {}", path.display()))
        }
    }
}

fn finish_update_mode(
    paths: &AppPaths,
    pending_dir: &Path,
    result_path: &Path,
    helper_pid: u32,
    platform: &dyn PlatformOps,
) -> Result<Option<String>> {
    finish_update_mode_with_cleanup(
        paths,
        pending_dir,
        result_path,
        helper_pid,
        platform,
        remove_pending_update_with_retry,
    )
}

fn finish_update_mode_with_cleanup<F>(
    paths: &AppPaths,
    pending_dir: &Path,
    result_path: &Path,
    helper_pid: u32,
    platform: &dyn PlatformOps,
    cleanup: F,
) -> Result<Option<String>>
where
    F: FnOnce(&Path) -> Result<()>,
{
    let (pending_dir, result_path) =
        validate_pending_update_paths(paths, pending_dir, result_path)?;
    record_update_owner(&pending_dir, std::process::id())?;
    platform
        .wait_for_process(
            helper_pid,
            std::time::Duration::from_secs(UPDATE_PROCESS_WAIT_SECONDS),
        )
        .with_context(|| format!("failed while waiting for update helper {helper_pid}"))?;
    let result = read_update_result(&result_path);
    let mut notice = match result {
        Ok(UpdateResultDocument {
            status: UpdateResultStatus::Applied { .. },
            ..
        }) => None,
        Ok(UpdateResultDocument {
            status: UpdateResultStatus::Failed { message },
            ..
        }) => Some(message),
        Err(error) => Some(format!("{error:#}")),
    };
    if let Err(error) = cleanup(&pending_dir) {
        let cleanup = format!("Could not remove temporary update files: {error:#}");
        if let Some(detail) = notice.as_mut() {
            detail.push_str("\n\n");
            detail.push_str(&cleanup);
        } else {
            tracing::warn!(%error, path = %pending_dir.display(), "could not clean successful update staging");
        }
    }
    Ok(notice)
}

fn validate_pending_update_paths(
    paths: &AppPaths,
    pending_dir: &Path,
    result_path: &Path,
) -> Result<(PathBuf, PathBuf)> {
    let updates = fs::canonicalize(paths.cache_dir().join("Updates")).with_context(|| {
        format!(
            "failed to resolve update cache {}",
            paths.cache_dir().join("Updates").display()
        )
    })?;
    let pending = fs::canonicalize(pending_dir)
        .with_context(|| format!("failed to resolve pending update {}", pending_dir.display()))?;
    let valid_pending_name = pending
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("pending-") && name.len() > "pending-".len());
    if !valid_pending_name {
        bail!("pending update directory has an invalid name");
    }
    if pending.parent() != Some(updates.as_path()) {
        bail!(
            "pending update {} is not a direct child of {}",
            pending.display(),
            updates.display()
        );
    }

    let result_parent = result_path
        .parent()
        .ok_or_else(|| anyhow!("update result {} has no parent", result_path.display()))?;
    let result_parent = fs::canonicalize(result_parent).with_context(|| {
        format!(
            "failed to resolve update result parent {}",
            result_parent.display()
        )
    })?;
    if result_parent != pending || result_path.file_name() != Some(UPDATE_RESULT_FILE_NAME.as_ref())
    {
        bail!(
            "update result {} does not belong to pending update {}",
            result_path.display(),
            pending.display()
        );
    }
    if fs::symlink_metadata(result_path)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
    {
        bail!(
            "update result {} must not be a symlink",
            result_path.display()
        );
    }
    Ok((pending.clone(), pending.join(UPDATE_RESULT_FILE_NAME)))
}

fn read_update_result(path: &Path) -> Result<UpdateResultDocument> {
    let bytes = fs::read(path)
        .with_context(|| format!("failed to read update result {}", path.display()))?;
    let result: UpdateResultDocument = serde_json::from_slice(&bytes)
        .with_context(|| format!("failed to parse update result {}", path.display()))?;
    if result.schema != UPDATE_RESULT_SCHEMA {
        bail!(
            "update result {} uses schema {}; expected {UPDATE_RESULT_SCHEMA}",
            path.display(),
            result.schema
        );
    }
    Ok(result)
}

fn recover_abandoned_pending_updates(
    paths: &AppPaths,
    platform: &dyn PlatformOps,
) -> Option<String> {
    let updates = paths.cache_dir().join("Updates");
    let entries = match fs::read_dir(&updates) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return None,
        Err(error) => {
            tracing::warn!(%error, path = %updates.display(), "could not inspect update staging");
            return None;
        }
    };
    let mut candidates = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .collect::<Vec<_>>();
    candidates.sort();
    let mut notices = Vec::new();
    for pending in candidates {
        let valid_name = pending
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("pending-") && name.len() > "pending-".len());
        let is_plain_directory = fs::symlink_metadata(&pending)
            .map(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
            .unwrap_or(false);
        if !valid_name || !is_plain_directory || !pending_update_owners_are_gone(&pending, platform)
        {
            continue;
        }

        let result_path = pending.join(UPDATE_RESULT_FILE_NAME);
        let result_is_symlink = fs::symlink_metadata(&result_path)
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(false);
        let mut notice = if result_is_symlink {
            Some("An earlier update left an unsafe result marker.".to_string())
        } else {
            match read_update_result(&result_path) {
                Ok(UpdateResultDocument {
                    status: UpdateResultStatus::Applied { .. },
                    ..
                }) => None,
                Ok(UpdateResultDocument {
                    status: UpdateResultStatus::Failed { message },
                    ..
                }) => Some(message),
                Err(_) if !result_path.exists() => Some(
                    "An earlier update was interrupted before it could report a result."
                        .to_string(),
                ),
                Err(error) => Some(format!("{error:#}")),
            }
        };
        if let Err(error) = remove_pending_update_with_retry(&pending) {
            let cleanup = format!("Could not remove temporary update files: {error:#}");
            if let Some(detail) = notice.as_mut() {
                detail.push_str("\n\n");
                detail.push_str(&cleanup);
            } else {
                tracing::warn!(%error, path = %pending.display(), "could not reclaim successful update staging");
            }
        }
        if let Some(notice) = notice {
            notices.push(notice);
        }
    }
    (!notices.is_empty()).then(|| notices.join("\n\n"))
}

fn pending_update_owners_are_gone(pending: &Path, platform: &dyn PlatformOps) -> bool {
    let entries = match fs::read_dir(pending) {
        Ok(entries) => entries,
        Err(error) => {
            tracing::warn!(%error, path = %pending.display(), "could not inspect update owners");
            return false;
        }
    };
    let mut saw_owner = false;
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                tracing::warn!(%error, path = %pending.display(), "could not inspect an update owner");
                return false;
            }
        };
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Some(pid_text) = name.strip_prefix(UPDATE_OWNER_FILE_PREFIX) else {
            continue;
        };
        saw_owner = true;
        let marker_is_file = entry
            .file_type()
            .map(|kind| kind.is_file() && !kind.is_symlink())
            .unwrap_or(false);
        let Ok(pid) = pid_text.parse::<u32>() else {
            return false;
        };
        if !marker_is_file || pid == 0 || pid == std::process::id() {
            return false;
        }
        match platform.wait_for_process(pid, std::time::Duration::ZERO) {
            Ok(()) => {}
            Err(clonk_update::PlatformError::WaitTimeout { .. }) => return false,
            Err(error) => {
                tracing::warn!(%error, pid, "could not prove an update owner exited");
                return false;
            }
        }
    }
    saw_owner
}

fn remove_pending_update_with_retry(path: &Path) -> Result<()> {
    let mut last_error = None;
    for attempt in 0..UPDATE_CLEANUP_ATTEMPTS {
        match fs::remove_dir_all(path) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => last_error = Some(error),
        }
        if attempt + 1 < UPDATE_CLEANUP_ATTEMPTS {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
    Err(last_error
        .map(anyhow::Error::from)
        .unwrap_or_else(|| anyhow!("failed to remove pending update {}", path.display())))
    .with_context(|| format!("failed to remove pending update {}", path.display()))
}

fn updated_launcher_command(
    layout: &InstallLayout,
    pending_dir: &Path,
    result_path: &Path,
    helper_pid: u32,
) -> Command {
    let mut command = Command::new(
        layout
            .binaries_dir()
            .join(format!("clonk-game{}", env::consts::EXE_SUFFIX)),
    );
    command
        .arg("--finish-update")
        .arg(pending_dir)
        .arg("--update-result")
        .arg(result_path)
        .arg("--update-helper-pid")
        .arg(helper_pid.to_string());
    command.current_dir(
        pending_dir
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| layout.work_dir()),
    );
    command.env_remove(LAUNCHER_PID_ENV);
    command.env_remove(UPDATE_NOTICE_ENV);
    command.env_remove(clonk_update::UPDATE_RECOVERY_COMPLETE_ENV);
    command.stdin(Stdio::null());
    command.stdout(Stdio::null());
    command.stderr(Stdio::null());
    command
}

fn relaunch_updated_game(
    layout: &InstallLayout,
    pending_dir: &Path,
    result_path: &Path,
    helper_pid: u32,
) -> Result<()> {
    let mut command = updated_launcher_command(layout, pending_dir, result_path, helper_pid);
    let program = command.get_program().to_os_string();
    let child = command
        .spawn()
        .with_context(|| format!("failed to start {}", PathBuf::from(program).display()))?;
    record_update_owner(pending_dir, child.id())?;
    Ok(())
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
    let Some(patcher) = locate_update_tool(paths) else {
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

fn locate_update_tool(paths: &AppPaths) -> Option<PathBuf> {
    candidate_patcher_paths(paths)
        .into_iter()
        .find(|candidate| candidate.exists())
}

fn candidate_patcher_paths(paths: &AppPaths) -> Vec<PathBuf> {
    const DEVELOPMENT_CANDIDATES: &[&str] = &[
        "c4group",
        "c4group.exe",
        "build/c4group",
        "build/c4group.exe",
        "build/Debug/c4group",
        "build/Debug/c4group.exe",
        "build/Release/c4group",
        "build/Release/c4group.exe",
    ];
    let binaries_dir = paths.binaries_dir();
    ["c4group", "c4group.exe"]
        .iter()
        .map(|name| binaries_dir.join(name))
        .chain(
            DEVELOPMENT_CANDIDATES
                .iter()
                .map(|relative| paths.install_root().join(relative)),
        )
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
    use clonk_update::{
        sha256_file, FakePlatform, InstalledState, Journal, JournalStep, PlatformCall,
        PlatformError, StagedComponent, StepState,
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
    use zip::write::FileOptions;
    use zip::{ZipArchive, ZipWriter};

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

    struct FailingWaitPlatform {
        waited: Mutex<Vec<u32>>,
    }

    impl FailingWaitPlatform {
        fn new() -> Self {
            Self {
                waited: Mutex::new(Vec::new()),
            }
        }
    }

    impl PlatformOps for FailingWaitPlatform {
        fn available_space(&self, _path: &Path) -> Result<u64, PlatformError> {
            Ok(u64::MAX)
        }

        fn wait_for_process(&self, pid: u32, _timeout: Duration) -> Result<(), PlatformError> {
            self.waited
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(pid);
            Err(PlatformError::WaitTimeout { pid, seconds: 120 })
        }

        fn codesign(&self, _arguments: &[&str], _target: &Path) -> Result<(), PlatformError> {
            Ok(())
        }

        fn set_installed_version(&self, _version: &str) -> Result<(), PlatformError> {
            Ok(())
        }
    }

    struct LivenessPlatform {
        alive: Vec<u32>,
        waited: Mutex<Vec<u32>>,
    }

    impl LivenessPlatform {
        fn new(alive: Vec<u32>) -> Self {
            Self {
                alive,
                waited: Mutex::new(Vec::new()),
            }
        }
    }

    impl PlatformOps for LivenessPlatform {
        fn available_space(&self, _path: &Path) -> Result<u64, PlatformError> {
            Ok(u64::MAX)
        }

        fn wait_for_process(&self, pid: u32, _timeout: Duration) -> Result<(), PlatformError> {
            self.waited
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(pid);
            if self.alive.contains(&pid) {
                Err(PlatformError::WaitTimeout { pid, seconds: 0 })
            } else {
                Ok(())
            }
        }

        fn codesign(&self, _arguments: &[&str], _target: &Path) -> Result<(), PlatformError> {
            Ok(())
        }

        fn set_installed_version(&self, _version: &str) -> Result<(), PlatformError> {
            Ok(())
        }
    }

    #[test]
    fn update_apply_cli_requires_an_explicit_plan_root_and_accepts_each_wait_pid() {
        let cli = Cli::try_parse_from([
            "clonk-game",
            "--apply-update",
            "pending-plan.json",
            "--install-root",
            "/opt/clonk",
            "--wait-pid",
            "41",
            "--wait-pid",
            "42",
            "--relaunch",
        ])
        .expect("parse update apply mode");

        assert_eq!(cli.apply_update, Some(PathBuf::from("pending-plan.json")));
        assert_eq!(cli.install_root, Some(PathBuf::from("/opt/clonk")));
        assert_eq!(cli.wait_pids, [41, 42]);
        assert!(cli.relaunch);
        assert!(
            Cli::try_parse_from(["clonk-game", "--apply-update", "pending-plan.json"]).is_err(),
            "apply mode must not infer an install root from its temporary executable"
        );
        assert!(
            Cli::try_parse_from(["clonk-game", "--install-root", "/opt/clonk"]).is_err(),
            "an install root is meaningful only in apply mode"
        );
    }

    #[test]
    fn finish_update_cli_carries_the_staging_result_and_helper_process() {
        let cli = Cli::try_parse_from([
            "clonk-game",
            "--finish-update",
            "/cache/Updates/pending-abc",
            "--update-result",
            "/cache/Updates/pending-abc/update-result.json",
            "--update-helper-pid",
            "73",
        ])
        .expect("parse hidden finish mode");

        assert_eq!(
            cli.finish_update,
            Some(PathBuf::from("/cache/Updates/pending-abc"))
        );
        assert_eq!(
            cli.update_result,
            Some(PathBuf::from(
                "/cache/Updates/pending-abc/update-result.json"
            ))
        );
        assert_eq!(cli.update_helper_pid, Some(73));
    }

    #[test]
    fn finish_update_waits_reports_failure_and_removes_the_whole_pending_directory() {
        let install = TempDir::new().expect("install root");
        fs::create_dir_all(install.path().join("planet")).expect("planet directory");
        fs::write(install.path().join("planet/System.c4g"), b"stub").expect("system group");
        let user = TempDir::new().expect("user data");
        let cache = TempDir::new().expect("cache");
        let updates = cache.path().join("Updates");
        let pending = updates.join("pending-failed-apply");
        fs::create_dir_all(&pending).expect("pending directory");
        fs::write(pending.join("plan.json"), b"plan").expect("plan");
        fs::write(pending.join("0-content.zip"), b"archive").expect("archive");
        fs::write(pending.join("clonk-game"), b"helper").expect("helper");
        let result_path = pending.join(UPDATE_RESULT_FILE_NAME);
        write_update_result(
            &result_path,
            &UpdateResultDocument {
                schema: UPDATE_RESULT_SCHEMA,
                status: UpdateResultStatus::Failed {
                    message: "digest mismatch".to_string(),
                },
            },
        )
        .expect("write result");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install.path())),
            ("LC_USER_DATA_DIR", Some(user.path())),
            ("LC_CACHE_DIR", Some(cache.path())),
        ]);
        let platform = FakePlatform::new();
        let paths = AppPaths::discover().expect("discover paths");

        let notice = finish_update_mode(&paths, &pending, &result_path, 73, &platform)
            .expect("finish failed update");

        assert_eq!(platform.calls(), [PlatformCall::WaitForProcess { pid: 73 }]);
        assert_eq!(notice.as_deref(), Some("digest mismatch"));
        assert!(!pending.exists(), "finisher must remove every staged file");
    }

    #[test]
    fn finish_update_rejects_a_pending_directory_outside_the_discovered_cache() {
        let install = TempDir::new().expect("install root");
        fs::create_dir_all(install.path().join("planet")).expect("planet directory");
        fs::write(install.path().join("planet/System.c4g"), b"stub").expect("system group");
        let user = TempDir::new().expect("user data");
        let cache = TempDir::new().expect("cache");
        fs::create_dir(cache.path().join("Updates")).expect("updates directory");
        let outside = TempDir::new().expect("outside directory");
        let pending = outside.path().join("pending-escape");
        fs::create_dir(&pending).expect("pending directory");
        let result_path = pending.join(UPDATE_RESULT_FILE_NAME);
        write_update_result(
            &result_path,
            &UpdateResultDocument {
                schema: UPDATE_RESULT_SCHEMA,
                status: UpdateResultStatus::Applied {
                    version: "0.7.0".to_string(),
                    components: Vec::new(),
                },
            },
        )
        .expect("result");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install.path())),
            ("LC_USER_DATA_DIR", Some(user.path())),
            ("LC_CACHE_DIR", Some(cache.path())),
        ]);
        let platform = FakePlatform::new();
        let paths = AppPaths::discover().expect("discover paths");

        let error = finish_update_mode(&paths, &pending, &result_path, 73, &platform)
            .expect_err("outside pending directory must be rejected");

        assert!(error.to_string().contains("not a direct child"));
        assert!(
            platform.calls().is_empty(),
            "untrusted paths must be rejected before process interaction"
        );
        assert!(pending.exists(), "rejected paths must never be removed");
    }

    #[test]
    fn successful_update_does_not_become_a_failure_notice_when_cleanup_exhausts_retries() {
        let install = TempDir::new().expect("install root");
        fs::create_dir_all(install.path().join("planet")).expect("planet directory");
        fs::write(install.path().join("planet/System.c4g"), b"stub").expect("system group");
        let user = TempDir::new().expect("user data");
        let cache = TempDir::new().expect("cache");
        let pending = cache.path().join("Updates/pending-cleanup-busy");
        fs::create_dir_all(&pending).expect("pending directory");
        let result_path = pending.join(UPDATE_RESULT_FILE_NAME);
        write_update_result(
            &result_path,
            &UpdateResultDocument {
                schema: UPDATE_RESULT_SCHEMA,
                status: UpdateResultStatus::Applied {
                    version: "0.7.0".to_string(),
                    components: vec!["engine".to_string()],
                },
            },
        )
        .expect("result");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install.path())),
            ("LC_USER_DATA_DIR", Some(user.path())),
            ("LC_CACHE_DIR", Some(cache.path())),
        ]);
        let paths = AppPaths::discover().expect("discover paths");

        let notice = finish_update_mode_with_cleanup(
            &paths,
            &pending,
            &result_path,
            73,
            &FakePlatform::new(),
            |_| bail!("directory is still busy"),
        )
        .expect("cleanup exhaustion is nonfatal");

        assert_eq!(notice, None);
        assert!(pending.exists());
    }

    #[test]
    fn startup_recovery_never_removes_staging_owned_by_a_live_process() {
        let install = TempDir::new().expect("install root");
        fs::create_dir_all(install.path().join("planet")).expect("planet directory");
        fs::write(install.path().join("planet/System.c4g"), b"stub").expect("system group");
        let user = TempDir::new().expect("user data");
        let cache = TempDir::new().expect("cache");
        let pending = cache.path().join("Updates/pending-active-download");
        fs::create_dir_all(&pending).expect("pending directory");
        fs::write(pending.join(".owner-41"), b"").expect("owner marker");
        fs::write(pending.join("plan.json"), b"active").expect("active plan");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install.path())),
            ("LC_USER_DATA_DIR", Some(user.path())),
            ("LC_CACHE_DIR", Some(cache.path())),
        ]);
        let paths = AppPaths::discover().expect("discover paths");
        let platform = LivenessPlatform::new(vec![41]);

        let notice = recover_abandoned_pending_updates(&paths, &platform);

        assert_eq!(notice, None);
        assert!(pending.exists(), "live staging must never be reclaimed");
        assert_eq!(
            *platform
                .waited
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            [41]
        );
    }

    #[test]
    fn startup_recovery_reports_and_reclaims_staging_after_its_owner_crashes() {
        let install = TempDir::new().expect("install root");
        fs::create_dir_all(install.path().join("planet")).expect("planet directory");
        fs::write(install.path().join("planet/System.c4g"), b"stub").expect("system group");
        let user = TempDir::new().expect("user data");
        let cache = TempDir::new().expect("cache");
        let pending = cache.path().join("Updates/pending-crashed-download");
        fs::create_dir_all(&pending).expect("pending directory");
        fs::write(pending.join(".owner-41"), b"").expect("owner marker");
        fs::write(pending.join("0-engine.zip"), b"partial").expect("partial archive");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install.path())),
            ("LC_USER_DATA_DIR", Some(user.path())),
            ("LC_CACHE_DIR", Some(cache.path())),
        ]);
        let paths = AppPaths::discover().expect("discover paths");
        let platform = LivenessPlatform::new(Vec::new());

        let notice = recover_abandoned_pending_updates(&paths, &platform)
            .expect("interrupted update notice");

        assert!(notice.contains("interrupted before it could report"));
        assert!(!pending.exists(), "dead staging must be reclaimed");
    }

    #[test]
    fn wait_failure_records_a_terminal_result_without_relaunching() {
        let directory = TempDir::new().expect("update directory");
        let pending = directory.path().join("pending-wait-failure");
        fs::create_dir(&pending).expect("pending directory");
        let missing_plan = pending.join("plan.json");
        let install_root = directory.path().join("install");
        fs::create_dir(&install_root).expect("install root");
        let platform = FailingWaitPlatform::new();
        let relaunched = Cell::new(false);

        let error = apply_update_plan_with_relauncher(
            &missing_plan,
            &install_root,
            &[41, 42],
            true,
            &platform,
            73,
            |_, _, _, _| {
                relaunched.set(true);
                Ok(())
            },
        )
        .expect_err("wait failure must stop the helper");

        assert!(error.to_string().contains("waiting for process 41"));
        assert_eq!(
            *platform
                .waited
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            [41, 42]
        );
        assert!(pending.exists(), "staging must remain for diagnosis/retry");
        let result = read_update_result(&pending.join(UPDATE_RESULT_FILE_NAME))
            .expect("wait failure result");
        assert!(matches!(
            result.status,
            UpdateResultStatus::Failed { message }
                if message.contains("waiting for process 41")
        ));
        assert!(
            !relaunched.get(),
            "a live old process makes relaunch unsafe"
        );
    }

    #[test]
    fn helper_refuses_to_wait_or_apply_without_a_regular_owner_marker() {
        let directory = TempDir::new().expect("update directory");
        let pending = directory.path().join("pending-owner-blocked");
        fs::create_dir(&pending).expect("pending directory");
        fs::create_dir(pending.join(".owner-73")).expect("block owner marker");
        let plan_path = pending.join("plan.json");
        fs::write(&plan_path, br#"{"version":"0.7.0","components":[]}"#).expect("write plan");
        let install_root = directory.path().join("install");
        fs::create_dir(&install_root).expect("install root");
        let platform = FakePlatform::new();

        let error = apply_update_plan_with_relauncher(
            &plan_path,
            &install_root,
            &[41],
            true,
            &platform,
            73,
            |_, _, _, _| Ok(()),
        )
        .expect_err("an invalid ownership marker must fail closed");

        assert!(error
            .to_string()
            .contains("record the active update helper"));
        assert!(platform.calls().is_empty(), "waiting must not begin");
        assert!(!pending.join(UPDATE_RESULT_FILE_NAME).exists());
    }

    #[test]
    fn malformed_plan_records_failure_and_relaunches_without_cleanup() {
        let directory = TempDir::new().expect("update directory");
        let pending = directory.path().join("pending-malformed");
        fs::create_dir(&pending).expect("pending directory");
        let plan_path = pending.join("plan.json");
        fs::write(&plan_path, b"{ not json").expect("malformed plan");
        let archive = pending.join("0-content.zip");
        fs::write(&archive, b"staged archive").expect("staged archive");
        let install_root = directory.path().join("install");
        fs::create_dir(&install_root).expect("install root");
        let relaunched = Cell::new(false);

        let error = apply_update_plan_with_relauncher(
            &plan_path,
            &install_root,
            &[],
            true,
            &FakePlatform::new(),
            73,
            |layout, finish_pending, result_path, helper_pid| {
                assert_eq!(layout, &InstallLayout::plain(&install_root));
                assert_eq!(finish_pending, pending);
                assert_eq!(result_path, pending.join("update-result.json"));
                assert_eq!(helper_pid, 73);
                let result: Value = serde_json::from_slice(
                    &fs::read(result_path).expect("read typed update result"),
                )
                .expect("parse typed update result");
                assert_eq!(result["schema"], 1);
                assert_eq!(result["status"], "failed");
                assert!(result["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("parse update plan")));
                relaunched.set(true);
                Ok(())
            },
        )
        .expect_err("malformed plan remains the helper's result");

        assert!(error.to_string().contains("parse update plan"));
        assert!(relaunched.get(), "parse failure must relaunch the game");
        assert!(pending.exists());
        assert!(plan_path.exists());
        assert!(archive.exists());
    }

    #[test]
    fn apply_failure_relaunches_and_remains_primary_when_relaunch_also_fails() {
        let directory = TempDir::new().expect("update directory");
        let pending = directory.path().join("pending-apply-failure");
        fs::create_dir(&pending).expect("pending directory");
        let archive = pending.join("0-content.zip");
        fs::write(&archive, b"not the promised archive").expect("archive");
        let plan_path = pending.join("plan.json");
        let plan = ApplyPlan {
            version: "0.7.0".to_string(),
            components: vec![StagedComponent {
                name: "content".to_string(),
                archive: archive.clone(),
                sha256: "00".repeat(32),
                size: fs::metadata(&archive).expect("archive metadata").len(),
                destination: PathBuf::from("content"),
            }],
        };
        fs::write(
            &plan_path,
            serde_json::to_vec(&plan).expect("serialize plan"),
        )
        .expect("plan");
        let install_root = directory.path().join("install");
        fs::create_dir_all(install_root.join("content")).expect("install content");
        let relaunched = Cell::new(false);

        let error = apply_update_plan_with_relauncher(
            &plan_path,
            &install_root,
            &[],
            true,
            &FakePlatform::new(),
            73,
            |_, _, result_path, _| {
                relaunched.set(true);
                assert!(result_path.exists(), "failure result must precede relaunch");
                bail!("synthetic relaunch failure")
            },
        )
        .expect_err("apply must fail");

        assert!(relaunched.get());
        assert!(error.to_string().contains("failed to apply update plan"));
        assert!(!error.to_string().contains("synthetic relaunch failure"));
        assert!(pending.exists(), "finisher owns failed staging cleanup");
    }

    #[test]
    fn update_apply_cli_rejects_process_id_zero() {
        let parsed = Cli::try_parse_from([
            "clonk-game",
            "--apply-update",
            "pending-plan.json",
            "--install-root",
            "/opt/clonk",
            "--wait-pid",
            "0",
        ]);

        assert!(
            parsed.is_err(),
            "PID zero addresses the current process group"
        );
    }

    #[test]
    fn update_apply_cli_rejects_normal_launcher_options() {
        let apply = [
            "clonk-game",
            "--apply-update",
            "pending-plan.json",
            "--install-root",
            "/opt/clonk",
            "--relaunch",
        ];
        for incompatible in [
            vec!["--binary", "alternate-clonk-app"],
            vec!["--support-bundle-only"],
            vec!["--automation-report"],
            vec!["Scenario.c4s"],
        ] {
            let parsed = Cli::try_parse_from(apply.into_iter().chain(incompatible.iter().copied()));
            assert!(
                parsed.is_err(),
                "apply mode accepted normal launcher arguments {incompatible:?}"
            );
        }
    }

    #[test]
    fn runtime_command_names_the_launcher_process_for_update_handoff() {
        let install_dir = TempDir::new().expect("install root");
        let planet_dir = install_dir.path().join("planet");
        fs::create_dir_all(&planet_dir).expect("create planet directory");
        fs::write(planet_dir.join("System.c4g"), b"stub").expect("write system group");
        let user_dir = TempDir::new().expect("user data");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.path())),
        ]);
        let paths = AppPaths::discover().expect("discover paths");

        let command = runtime_command(
            Path::new("clonk-app"),
            &paths,
            &paths.config_file(),
            &[],
            Some("digest mismatch"),
        );
        let launcher_pid = command
            .get_envs()
            .find_map(|(name, value)| {
                (name == LAUNCHER_PID_ENV)
                    .then(|| value.map(std::ffi::OsStr::to_os_string))
                    .flatten()
            })
            .expect("launcher PID environment variable");

        assert_eq!(launcher_pid, OsString::from(std::process::id().to_string()));
        assert_eq!(
            command
                .get_envs()
                .find_map(|(name, value)| (name == UPDATE_NOTICE_ENV).then_some(value).flatten()),
            Some(std::ffi::OsStr::new("digest mismatch"))
        );
        let no_notice = runtime_command(
            Path::new("clonk-app"),
            &paths,
            &paths.config_file(),
            &[],
            None,
        );
        assert_eq!(
            no_notice
                .get_envs()
                .find_map(|(name, value)| (name == UPDATE_NOTICE_ENV).then_some(value)),
            Some(None),
            "ordinary launches must clear a stale update notice"
        );
    }

    #[test]
    fn update_apply_mode_waits_for_every_process_and_records_a_successful_plan() {
        let directory = TempDir::new().expect("update directory");
        let install_root = directory.path().join("install");
        fs::create_dir(&install_root).expect("create install root");
        let plan_path = directory.path().join("pending-plan.json");
        fs::write(&plan_path, br#"{"version":"0.7.0","components":[]}"#).expect("write apply plan");
        let platform = FakePlatform::new();

        let outcome = apply_update_plan(&plan_path, &install_root, &[41, 42], false, &platform)
            .expect("apply empty plan");

        assert_eq!(outcome.version, "0.7.0");
        assert!(outcome.applied.is_empty());
        assert_eq!(
            platform.calls(),
            [
                PlatformCall::WaitForProcess { pid: 41 },
                PlatformCall::WaitForProcess { pid: 42 },
            ]
        );
        assert!(plan_path.exists(), "the finisher owns staging cleanup");
        let result: UpdateResultDocument = serde_json::from_slice(
            &fs::read(directory.path().join(UPDATE_RESULT_FILE_NAME)).expect("read result"),
        )
        .expect("parse result");
        assert_eq!(
            result.status,
            UpdateResultStatus::Applied {
                version: "0.7.0".to_string(),
                components: Vec::new(),
            }
        );
    }

    #[test]
    fn update_apply_mode_installs_records_and_leaves_cleanup_for_the_finisher() {
        let directory = TempDir::new().expect("update directory");
        let install_root = directory.path().join("install");
        fs::create_dir_all(install_root.join("content")).expect("create installed content");
        fs::write(install_root.join("content/old.txt"), b"old").expect("write old content");

        let archive_path = directory.path().join("content.zip");
        let archive_file = File::create(&archive_path).expect("create archive");
        let mut archive = ZipWriter::new(archive_file);
        archive
            .start_file("new.txt", FileOptions::default())
            .expect("start archive entry");
        archive.write_all(b"new").expect("write archive entry");
        archive.finish().expect("finish archive");

        let digest = sha256_file(&archive_path).expect("hash archive");
        let plan = ApplyPlan {
            version: "0.7.0".to_string(),
            components: vec![StagedComponent {
                name: "content".to_string(),
                archive: archive_path.clone(),
                sha256: digest.clone(),
                size: fs::metadata(&archive_path).expect("archive metadata").len(),
                destination: PathBuf::from("content"),
            }],
        };
        let plan_path = directory.path().join("pending-plan.json");
        fs::write(
            &plan_path,
            serde_json::to_vec(&plan).expect("serialize plan"),
        )
        .expect("write plan");

        let outcome =
            apply_update_plan(&plan_path, &install_root, &[], false, &FakePlatform::new())
                .expect("apply content plan");

        assert_eq!(outcome.applied, ["content"]);
        assert_eq!(
            fs::read(install_root.join("content/new.txt")).expect("read installed content"),
            b"new"
        );
        let state = InstalledState::load(&install_root)
            .expect("load installed state")
            .expect("successful apply records installed state");
        let content = state.component("content").expect("content state");
        assert_eq!(content.version, "0.7.0");
        assert_eq!(content.sha256, digest);
        assert!(archive_path.exists(), "the finisher owns archive cleanup");
        assert!(plan_path.exists(), "the finisher owns plan cleanup");
    }

    #[test]
    fn update_apply_mode_dispatches_before_normal_install_discovery() {
        let directory = TempDir::new().expect("update directory");
        let install_root = directory.path().join("otherwise-undiscoverable-install");
        fs::create_dir(&install_root).expect("create install root");
        let plan_path = directory.path().join("pending-plan.json");
        fs::write(&plan_path, b"{ not json").expect("write malformed plan");
        let cli = Cli::try_parse_from([
            OsString::from("clonk-game"),
            OsString::from("--apply-update"),
            plan_path.clone().into_os_string(),
            OsString::from("--install-root"),
            install_root.into_os_string(),
            OsString::from("--relaunch"),
        ])
        .expect("parse apply command");

        let error = run_cli(cli).expect_err("malformed plan must fail after early dispatch");

        assert!(error.to_string().contains("parse update plan"));
        assert!(plan_path.exists(), "finish mode owns staging cleanup");
        assert!(directory.path().join(UPDATE_RESULT_FILE_NAME).exists());
    }

    #[test]
    fn updated_launcher_command_targets_the_applied_install_layout() {
        let directory = TempDir::new().expect("install directory");
        let pending = directory.path().join("pending-finish");
        let result = pending.join(UPDATE_RESULT_FILE_NAME);
        let plain = InstallLayout::plain(directory.path().join("plain"));
        let plain_command = updated_launcher_command(&plain, &pending, &result, 73);
        assert_eq!(
            plain_command.get_program(),
            plain
                .binaries_dir()
                .join(format!("clonk-game{}", env::consts::EXE_SUFFIX))
        );
        assert_eq!(plain_command.get_current_dir(), pending.parent());
        assert_eq!(
            plain_command.get_args().collect::<Vec<_>>(),
            [
                OsString::from("--finish-update"),
                pending.as_os_str().to_os_string(),
                OsString::from("--update-result"),
                result.as_os_str().to_os_string(),
                OsString::from("--update-helper-pid"),
                OsString::from("73"),
            ]
        );
        assert_eq!(
            plain_command
                .get_envs()
                .find_map(|(name, value)| (name == LAUNCHER_PID_ENV).then_some(value)),
            Some(None),
            "the finisher must not inherit the old launcher's PID"
        );
        assert_eq!(
            plain_command
                .get_envs()
                .find_map(|(name, value)| (name == UPDATE_NOTICE_ENV).then_some(value)),
            Some(None),
            "the finisher must not inherit an earlier update notice"
        );

        let bundle = InstallLayout::macos_bundle(directory.path().join("Clonk Rust.app"));
        let bundle_command = updated_launcher_command(&bundle, &pending, &result, 73);
        assert_eq!(
            bundle_command.get_program(),
            bundle
                .binaries_dir()
                .join(format!("clonk-game{}", env::consts::EXE_SUFFIX))
        );
        assert_eq!(
            bundle_command.get_args().collect::<Vec<_>>(),
            [
                OsString::from("--finish-update"),
                pending.as_os_str().to_os_string(),
                OsString::from("--update-result"),
                result.as_os_str().to_os_string(),
                OsString::from("--update-helper-pid"),
                OsString::from("73"),
            ]
        );
    }

    #[test]
    fn requested_relaunch_happens_after_the_result_is_written_without_cleanup() {
        let directory = TempDir::new().expect("update directory");
        let install_root = directory.path().join("install");
        fs::create_dir(&install_root).expect("create install root");
        let plan_path = directory.path().join("pending-plan.json");
        fs::write(&plan_path, br#"{"version":"0.7.0","components":[]}"#).expect("write plan");
        let relaunched = Cell::new(false);

        apply_update_plan_with_relauncher(
            &plan_path,
            &install_root,
            &[],
            true,
            &FakePlatform::new(),
            73,
            |layout, pending, result_path, helper_pid| {
                assert_eq!(layout, &InstallLayout::plain(&install_root));
                assert_eq!(pending, directory.path());
                assert_eq!(result_path, directory.path().join(UPDATE_RESULT_FILE_NAME));
                assert_eq!(helper_pid, 73);
                assert!(result_path.exists(), "result must precede relaunch");
                assert!(plan_path.exists(), "the helper must not clean itself up");
                relaunched.set(true);
                Ok(())
            },
        )
        .expect("apply and relaunch");

        assert!(relaunched.get());
    }

    #[test]
    fn launcher_recovers_a_missing_planet_before_validated_path_discovery() {
        let install_dir = TempDir::new().expect("install root");
        let user_dir = TempDir::new().expect("user data");
        let nonce = "missing-planet";
        let backup = install_dir.path().join(format!("planet.old-{nonce}"));
        fs::create_dir_all(&backup).expect("create backed-up planet");
        fs::write(backup.join("System.c4g"), b"stub").expect("write backed-up system group");
        let mut step = JournalStep::new("planet", &"aa".repeat(32), "planet");
        step.state = StepState::BackupMoved;
        Journal::new(
            "0.7.0",
            nonce,
            fs::canonicalize(install_dir.path()).expect("canonical install root"),
            vec![step],
        )
        .save(install_dir.path())
        .expect("save interrupted update journal");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.path())),
        ]);

        assert!(matches!(
            AppPaths::discover(),
            Err(clonk_platform::PathsError::SystemGroupMissing { .. })
        ));
        let outcome = recover_interrupted_update_before_path_discovery()
            .expect("recover before validating the system group");
        let paths = AppPaths::discover().expect("discover recovered paths");

        assert_eq!(
            outcome,
            ResumeOutcome::RolledBack {
                version: "0.7.0".to_string()
            }
        );
        assert_eq!(
            fs::read(paths.system_group_path()).expect("read restored system group"),
            b"stub"
        );
    }

    #[test]
    fn normal_startup_logs_that_no_interrupted_update_needed_recovery() {
        let install_dir = TempDir::new().expect("install root");
        let planet_dir = install_dir.path().join("planet");
        fs::create_dir_all(&planet_dir).expect("create planet directory");
        fs::write(planet_dir.join("System.c4g"), b"stub").expect("write system group");
        let user_dir = TempDir::new().expect("user data");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.path())),
        ]);
        let paths = AppPaths::discover().expect("discover paths");
        paths.ensure_user_dirs().expect("prepare user directories");
        let logger = LauncherLogger::new(&paths).expect("launcher logger");

        let outcome = recover_interrupted_update_before_path_discovery().expect("check recovery");
        log_update_recovery(&outcome, &logger).expect("log update recovery");

        let log = fs::read_to_string(logger.path()).expect("read launcher log");
        assert!(
            log.contains("no interrupted component update to recover"),
            "missing recovery result in launcher log: {log}"
        );
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
            .join("bin")
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
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn shipped_bundle_stages_runtime_groups_only_beside_the_signed_app() {
        let directory = TempDir::new().expect("bundle parent");
        let bundle = directory.path().join("Clonk Rust.app");
        let resources = bundle.join("Contents/Resources");
        let binaries = bundle.join("Contents/MacOS");
        fs::create_dir_all(resources.join("planet")).expect("planet directory");
        fs::create_dir_all(&binaries).expect("binary directory");
        fs::write(resources.join("planet/System.c4g"), b"system").expect("system group");
        fs::write(resources.join("planet/Graphics.c4g"), b"graphics").expect("graphics group");
        let binary = binaries.join("clonk-app");
        fs::write(&binary, b"runtime").expect("runtime");
        let user = TempDir::new().expect("user data");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(resources.as_path())),
            ("LC_USER_DATA_DIR", Some(user.path())),
        ]);
        let paths = AppPaths::discover().expect("bundle paths");
        paths.ensure_user_dirs().expect("user directories");
        let logger = LauncherLogger::new(&paths).expect("logger");

        ensure_runtime_assets(&paths, &binary, &logger).expect("stage runtime groups");

        assert!(directory.path().join("System.c4g").exists());
        assert!(directory.path().join("Graphics.c4g").exists());
        assert!(!resources.join("System.c4g").exists());
        assert!(!resources.join("Graphics.c4g").exists());
        assert!(!binaries.join("System.c4g").exists());
        assert!(!binaries.join("Graphics.c4g").exists());
    }

    #[test]
    fn ensure_runtime_assets_replaces_stale_targets() {
        let install_dir = TempDir::new().unwrap();
        let planet_dir = install_dir.path().join("planet");
        fs::create_dir_all(&planet_dir).unwrap();
        // Shipped groups are directories, so a stale target is removed through
        // the `remove_dir_all` arm rather than `remove_file`.
        fs::create_dir_all(planet_dir.join("System.c4g")).unwrap();
        fs::write(
            planet_dir.join("System.c4g").join("C4.c"),
            b"system payload",
        )
        .unwrap();
        fs::create_dir_all(planet_dir.join("Graphics.c4g")).unwrap();
        fs::write(
            planet_dir.join("Graphics.c4g").join("Logo.png"),
            b"graphics payload",
        )
        .unwrap();

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
            fs::read(stale_system.join("C4.c")).unwrap(),
            b"system payload",
            "system target should match updated source"
        );
        assert_eq!(
            fs::read(stale_graphics.join("Logo.png")).unwrap(),
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
    fn l004_runtime_asset_copy_materialises_a_group_directory() {
        // `planet/System.c4g` is a C4Group *directory*, not a file. Windows can
        // neither hard link a directory nor reach the `#[cfg(unix)]` symlink
        // arm, so the terminal copy fallback is the only path it has — and
        // `fs::copy` fails on a directory.
        let source_dir = TempDir::new().unwrap();
        let source = source_dir.path().join("System.c4g");
        fs::create_dir_all(source.join("nested")).unwrap();
        fs::write(source.join("C4.c"), b"root script").unwrap();
        fs::write(source.join("nested").join("Extra.c"), b"nested script").unwrap();

        let target_dir = TempDir::new().unwrap();
        let target = target_dir.path().join("System.c4g");

        copy_runtime_asset(&source, &target).expect("group directory should be copied");

        assert_eq!(
            fs::read(target.join("C4.c")).unwrap(),
            b"root script",
            "top level group entry should be copied"
        );
        assert_eq!(
            fs::read(target.join("nested").join("Extra.c")).unwrap(),
            b"nested script",
            "nested group entry should be copied"
        );
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
    fn packaged_update_tool_precedes_development_fallbacks() {
        // xtask/src/main.rs:1741-1763 stages every shipped executable under
        // `bin`; a source-tree fallback must not shadow the installed tool.
        let install_dir = TempDir::new().unwrap();
        let planet_dir = install_dir.path().join("planet");
        fs::create_dir_all(&planet_dir).unwrap();
        fs::write(planet_dir.join("System.c4g"), b"stub").unwrap();
        let packaged = install_dir.path().join("bin/c4group");
        fs::create_dir_all(packaged.parent().unwrap()).unwrap();
        fs::write(&packaged, b"packaged").unwrap();
        fs::write(install_dir.path().join("c4group"), b"development").unwrap();

        let user_dir = TempDir::new().unwrap();
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.path())),
        ]);
        let paths = AppPaths::discover().unwrap();

        assert_eq!(locate_update_tool(&paths), Some(packaged));
    }

    #[test]
    fn bundled_update_tool_resolves_from_contents_macos() {
        // xtask/src/main.rs:1812-1824 moves the flat `bin` payload beside the
        // bundle executable while AppPaths treats `Contents/Resources` as the
        // install root.
        let install_dir = TempDir::new().unwrap();
        let contents = install_dir.path().join("Clonk Rust.app/Contents");
        let resources = contents.join("Resources");
        fs::create_dir_all(resources.join("planet")).unwrap();
        fs::write(resources.join("planet/System.c4g"), b"stub").unwrap();
        let bundled = contents.join("MacOS/c4group");
        fs::create_dir_all(bundled.parent().unwrap()).unwrap();
        fs::write(&bundled, b"bundled").unwrap();
        fs::write(resources.join("c4group"), b"development").unwrap();

        let user_dir = TempDir::new().unwrap();
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(resources.as_path())),
            ("LC_USER_DATA_DIR", Some(user_dir.path())),
        ]);
        let paths = AppPaths::discover().unwrap();

        assert_eq!(paths.binaries_dir(), contents.join("MacOS"));
        assert_eq!(locate_update_tool(&paths), Some(bundled));
    }

    #[test]
    fn development_update_tool_fallback_remains_available() {
        let install_dir = TempDir::new().unwrap();
        let planet_dir = install_dir.path().join("planet");
        fs::create_dir_all(&planet_dir).unwrap();
        fs::write(planet_dir.join("System.c4g"), b"stub").unwrap();
        let fallback = install_dir.path().join("build/Release/c4group");
        fs::create_dir_all(fallback.parent().unwrap()).unwrap();
        fs::write(&fallback, b"development").unwrap();

        let user_dir = TempDir::new().unwrap();
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.path())),
        ]);
        let paths = AppPaths::discover().unwrap();

        assert_eq!(locate_update_tool(&paths), Some(fallback));
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

        // Backdated deliberately: `collect_runtime_logs` skips anything whose
        // mtime precedes `started_at`, and a filesystem timestamp can round
        // below a `now()` captured moments earlier — coarsely so on Windows.
        let start = SystemTime::now() - Duration::from_secs(1);
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
