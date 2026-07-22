use crate::bundle::regenerate_support_bundle;
use crate::log::LauncherLog;
use crate::paths::resolve_logs_entry;
use crate::report::render_support_bundle_report;
use crate::summary::{load_launcher_summary, LauncherSummary};
use crate::telemetry::UpdateTelemetrySummary;
use anyhow::{anyhow, Context, Result};
use clonk_platform::AppPaths;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct LauncherShellState {
    pub summary_path: PathBuf,
    pub logs_dir: PathBuf,
    pub summary: LauncherSummary,
    pub launcher_log_path: PathBuf,
    pub runtime_log_paths: Vec<PathBuf>,
    pub crash_report_paths: Vec<PathBuf>,
    pub support_bundle_path: Option<PathBuf>,
    pub telemetry_success_logs: Vec<PathBuf>,
    pub telemetry_failures: Vec<LauncherTelemetryFailure>,
    pub support_bundle_report: Vec<String>,
}

pub struct LauncherShellEnsureResult {
    pub bundle_path: PathBuf,
    pub telemetry: UpdateTelemetrySummary,
}

#[derive(Clone, Debug)]
pub struct SupportArtifact {
    pub path: PathBuf,
    pub role: &'static str,
}

#[derive(Clone, Debug)]
pub struct LauncherTelemetryFailure {
    pub log_path: PathBuf,
    pub message: String,
}

pub fn load_shell_state(paths: &AppPaths) -> Result<Option<LauncherShellState>> {
    let record = match load_launcher_summary(paths)? {
        Some(record) => record,
        None => return Ok(None),
    };

    let launcher_log_path = resolve_logs_entry(&record.logs_dir, &record.summary.launcher_log);
    let runtime_log_paths = record
        .summary
        .runtime_logs
        .iter()
        .map(|entry| resolve_logs_entry(&record.logs_dir, entry))
        .collect();
    let crash_report_paths = record
        .summary
        .crash_reports
        .iter()
        .map(|entry| resolve_logs_entry(&record.logs_dir, entry))
        .collect();
    let support_bundle_path = record
        .summary
        .support_bundle
        .as_ref()
        .map(|entry| resolve_logs_entry(&record.logs_dir, entry));
    let telemetry_summary = UpdateTelemetrySummary::from_serializable(
        &record.summary.update_telemetry,
        &record.logs_dir,
    );
    let support_bundle_report =
        render_support_bundle_report(paths, support_bundle_path.as_deref(), &telemetry_summary);
    let telemetry_success_logs = telemetry_summary.successes().to_vec();
    let telemetry_failures = telemetry_summary
        .failures()
        .iter()
        .map(|entry| LauncherTelemetryFailure {
            log_path: entry.log_path.clone(),
            message: entry.message.clone(),
        })
        .collect();

    Ok(Some(LauncherShellState {
        summary_path: record.path,
        logs_dir: record.logs_dir,
        summary: record.summary,
        launcher_log_path,
        runtime_log_paths,
        crash_report_paths,
        support_bundle_path,
        telemetry_success_logs,
        telemetry_failures,
        support_bundle_report,
    }))
}

pub fn ensure_support_bundle(
    paths: &AppPaths,
    logger: &dyn LauncherLog,
    launcher_log_path: &Path,
) -> Result<LauncherShellEnsureResult> {
    let (bundle_path, telemetry) = regenerate_support_bundle(paths, logger, launcher_log_path)?;
    Ok(LauncherShellEnsureResult {
        bundle_path,
        telemetry,
    })
}

pub fn copy_support_bundle(bundle_path: &Path, destination_dir: &Path) -> Result<PathBuf> {
    copy_with_unique_name(bundle_path, destination_dir)
}

pub fn copy_support_artifacts(
    artifacts: &[SupportArtifact],
    destination_dir: &Path,
) -> Result<Vec<PathBuf>> {
    if artifacts.is_empty() {
        return Err(anyhow!("no support artifacts were provided"));
    }
    fs::create_dir_all(destination_dir).with_context(|| {
        format!(
            "failed to create destination directory {}",
            destination_dir.display()
        )
    })?;

    let mut copied = Vec::with_capacity(artifacts.len());
    for artifact in artifacts {
        let path = copy_with_unique_name(&artifact.path, destination_dir)
            .with_context(|| format!("failed to stage {}", artifact.path.display()))?;
        copied.push(path);
    }
    Ok(copied)
}

fn copy_with_unique_name(source: &Path, destination_dir: &Path) -> Result<PathBuf> {
    if !source.exists() {
        return Err(anyhow!("artifact {} does not exist", source.display()));
    }
    fs::create_dir_all(destination_dir).with_context(|| {
        format!(
            "failed to create destination directory {}",
            destination_dir.display()
        )
    })?;

    let file_name = source
        .file_name()
        .ok_or_else(|| anyhow!("artifact lacks filename component"))?;
    let mut candidate = destination_dir.join(file_name);
    if candidate.exists() {
        let stem = source
            .file_stem()
            .and_then(OsStr::to_str)
            .unwrap_or("support-artifact");
        let extension = source
            .extension()
            .and_then(OsStr::to_str)
            .unwrap_or_default();
        let mut counter = 1usize;
        loop {
            let new_name = if extension.is_empty() {
                format!("{stem}-copy{counter}")
            } else {
                format!("{stem}-copy{counter}.{extension}")
            };
            candidate = destination_dir.join(new_name);
            if !candidate.exists() {
                break;
            }
            counter += 1;
        }
    }

    fs::copy(source, &candidate)
        .with_context(|| format!("failed to copy artifact to {}", candidate.display()))?;
    Ok(candidate)
}

pub fn reveal_in_file_manager(path: &Path) -> Result<()> {
    if !path.exists() {
        return Err(anyhow!("path {} does not exist", path.display()));
    }
    open::that_detached(path).with_context(|| {
        format!(
            "failed to open {} in the platform file manager",
            path.display()
        )
    })
}

pub fn support_artifacts(state: &LauncherShellState) -> Vec<SupportArtifact> {
    let mut artifacts = Vec::new();
    if let Some(bundle) = &state.support_bundle_path {
        artifacts.push(SupportArtifact {
            path: bundle.clone(),
            role: "support bundle",
        });
    }
    artifacts.push(SupportArtifact {
        path: state.summary_path.clone(),
        role: "launcher summary",
    });
    artifacts
}
