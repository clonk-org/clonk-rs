use crate::bundle::regenerate_support_bundle;
use crate::log::LauncherLog;
use crate::paths::resolve_logs_entry;
use crate::summary::{load_launcher_summary, LauncherSummary};
use crate::telemetry::UpdateTelemetrySummary;
use anyhow::{anyhow, Context, Result};
use lc_platform::AppPaths;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

pub struct LauncherShellState {
    pub summary_path: PathBuf,
    pub logs_dir: PathBuf,
    pub summary: LauncherSummary,
    pub launcher_log_path: PathBuf,
    pub runtime_log_paths: Vec<PathBuf>,
    pub crash_report_paths: Vec<PathBuf>,
    pub support_bundle_path: Option<PathBuf>,
}

pub struct LauncherShellEnsureResult {
    pub bundle_path: PathBuf,
    pub telemetry: UpdateTelemetrySummary,
}

pub struct SupportArtifact {
    pub path: PathBuf,
    pub role: &'static str,
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

    Ok(Some(LauncherShellState {
        summary_path: record.path,
        logs_dir: record.logs_dir,
        summary: record.summary,
        launcher_log_path,
        runtime_log_paths,
        crash_report_paths,
        support_bundle_path,
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
    if !bundle_path.exists() {
        return Err(anyhow!(
            "support bundle {} does not exist",
            bundle_path.display()
        ));
    }
    fs::create_dir_all(destination_dir).with_context(|| {
        format!(
            "failed to create destination directory {}",
            destination_dir.display()
        )
    })?;

    let file_name = bundle_path
        .file_name()
        .ok_or_else(|| anyhow!("support bundle lacks filename component"))?;
    let mut candidate = destination_dir.join(file_name);
    if candidate.exists() {
        let stem = bundle_path
            .file_stem()
            .and_then(OsStr::to_str)
            .unwrap_or("support-bundle");
        let extension = bundle_path
            .extension()
            .and_then(OsStr::to_str)
            .unwrap_or("zip");
        let mut counter = 1usize;
        loop {
            let new_name = format!("{stem}-copy{counter}.{extension}");
            candidate = destination_dir.join(new_name);
            if !candidate.exists() {
                break;
            }
            counter += 1;
        }
    }

    fs::copy(bundle_path, &candidate)
        .with_context(|| format!("failed to copy support bundle to {}", candidate.display()))?;
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
