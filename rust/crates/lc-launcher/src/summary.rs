use crate::log::LauncherLog;
use crate::paths::{ensure_logs_dir, launcher_summary_path, relative_to_logs};
use crate::provider::{
    ProviderAutomationState, ProviderDiagnostics, ProviderPathStatus, ProviderStatus,
};
use crate::telemetry::{SerializableTelemetrySummary, UpdateTelemetrySummary};
use crate::time::timestamp_for_log;
use anyhow::{anyhow, Context, Result};
use lc_platform::AppPaths;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LauncherSummary {
    pub schema_version: u32,
    pub generated_at: String,
    pub launcher_log: String,
    pub runtime_logs: Vec<String>,
    pub crash_reports: Vec<String>,
    pub support_bundle: Option<String>,
    pub update_telemetry: SerializableTelemetrySummary,
    #[serde(default, skip_serializing_if = "ProviderAutomationSnapshot::is_empty")]
    pub provider_automation: ProviderAutomationSnapshot,
}

pub struct LauncherSummaryRecord {
    pub summary: LauncherSummary,
    pub path: PathBuf,
    pub logs_dir: PathBuf,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderAutomationSnapshot {
    pub share: Vec<ProviderAutomationRecord>,
    pub upload: Vec<ProviderAutomationRecord>,
}

impl ProviderAutomationSnapshot {
    pub fn from_diagnostics(diagnostics: &ProviderDiagnostics, logs_dir: &Path) -> Self {
        Self {
            share: diagnostics
                .share
                .iter()
                .map(|status| ProviderAutomationRecord::from_status(status, logs_dir))
                .collect(),
            upload: diagnostics
                .upload
                .iter()
                .map(|status| ProviderAutomationRecord::from_status(status, logs_dir))
                .collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.share.is_empty() && self.upload.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderAutomationRecord {
    pub name: String,
    pub path: String,
    pub path_status: ProviderPathStatus,
    pub automation: ProviderAutomationState,
}

impl ProviderAutomationRecord {
    fn from_status(status: &ProviderStatus, logs_dir: &Path) -> Self {
        Self {
            name: status.name.clone(),
            path: relative_to_logs(&status.path, logs_dir),
            path_status: status.path_status.clone(),
            automation: status.automation.clone(),
        }
    }
}

pub fn write_launcher_summary(
    paths: &AppPaths,
    logger: &dyn LauncherLog,
    launcher_log_path: &Path,
    runtime_logs: &[PathBuf],
    crash_reports: &[PathBuf],
    telemetry_summary: &UpdateTelemetrySummary,
    support_bundle: Option<&Path>,
    provider_snapshot: Option<ProviderAutomationSnapshot>,
) -> Result<PathBuf> {
    if !launcher_log_path.exists() {
        return Err(anyhow!(
            "launcher log {} does not exist",
            launcher_log_path.display()
        ));
    }

    let logs_dir = ensure_logs_dir(paths)?;
    let summary = LauncherSummary {
        schema_version: 1,
        generated_at: timestamp_for_log(),
        launcher_log: relative_to_logs(launcher_log_path, &logs_dir),
        runtime_logs: runtime_logs
            .iter()
            .map(|path| relative_to_logs(path, &logs_dir))
            .collect(),
        crash_reports: crash_reports
            .iter()
            .map(|path| relative_to_logs(path, &logs_dir))
            .collect(),
        support_bundle: support_bundle.map(|path| relative_to_logs(path, &logs_dir)),
        update_telemetry: telemetry_summary.to_serializable(&logs_dir),
        provider_automation: provider_snapshot.unwrap_or_default(),
    };

    let summary_path = launcher_summary_path(&logs_dir);
    let file = File::create(&summary_path).with_context(|| {
        format!(
            "failed to create launcher summary file {}",
            summary_path.display()
        )
    })?;

    serde_json::to_writer_pretty(file, &summary).context("failed to serialize launcher summary")?;
    logger
        .log_line(&format!(
            "wrote launcher summary to {}",
            summary_path.display()
        ))
        .context("failed to log launcher summary path")?;
    Ok(summary_path)
}

pub fn load_launcher_summary(paths: &AppPaths) -> Result<Option<LauncherSummaryRecord>> {
    let logs_dir = paths.logs_dir();
    let summary_path = launcher_summary_path(&logs_dir);
    if !summary_path.exists() {
        return Ok(None);
    }

    let file = File::open(&summary_path)
        .with_context(|| format!("failed to open launcher summary {}", summary_path.display()))?;
    let summary: LauncherSummary = serde_json::from_reader(file).with_context(|| {
        format!(
            "failed to parse launcher summary {}",
            summary_path.display()
        )
    })?;

    Ok(Some(LauncherSummaryRecord {
        summary,
        path: summary_path,
        logs_dir: logs_dir.to_path_buf(),
    }))
}
