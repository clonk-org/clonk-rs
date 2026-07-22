use crate::log::LauncherLog;
use crate::paths::{relative_to_logs, resolve_logs_entry};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableTelemetrySummary {
    pub successes: Vec<String>,
    pub failures: Vec<SerializableTelemetryFailure>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableTelemetryFailure {
    pub log: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateTelemetryFailure {
    pub log_path: PathBuf,
    pub message: String,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct UpdateTelemetrySummary {
    success_sources: Vec<PathBuf>,
    failures: Vec<UpdateTelemetryFailure>,
}

impl UpdateTelemetrySummary {
    pub fn from_serializable(summary: &SerializableTelemetrySummary, logs_dir: &Path) -> Self {
        let mut telemetry = UpdateTelemetrySummary::default();
        for entry in &summary.successes {
            telemetry.record_success(resolve_logs_entry(logs_dir, entry));
        }
        for failure in &summary.failures {
            telemetry.record_failure(
                resolve_logs_entry(logs_dir, &failure.log),
                failure.message.clone(),
            );
        }
        telemetry
    }

    pub fn record_success(&mut self, path: PathBuf) {
        if !self
            .success_sources
            .iter()
            .any(|existing| existing == &path)
        {
            self.success_sources.push(path);
        }
    }

    pub fn record_failure(&mut self, path: PathBuf, message: String) {
        self.failures.push(UpdateTelemetryFailure {
            log_path: path,
            message,
        });
    }

    pub fn to_serializable(&self, base: &Path) -> SerializableTelemetrySummary {
        SerializableTelemetrySummary {
            successes: self
                .success_sources
                .iter()
                .map(|path| relative_to_logs(path, base))
                .collect(),
            failures: self
                .failures
                .iter()
                .map(|failure| SerializableTelemetryFailure {
                    log: relative_to_logs(&failure.log_path, base),
                    message: failure.message.clone(),
                })
                .collect(),
        }
    }

    pub fn successes(&self) -> &[PathBuf] {
        &self.success_sources
    }

    pub fn failures(&self) -> &[UpdateTelemetryFailure] {
        &self.failures
    }
}

pub fn digest_update_telemetry(
    log_paths: &[PathBuf],
    logger: &dyn LauncherLog,
) -> Result<UpdateTelemetrySummary> {
    if log_paths.is_empty() {
        logger
            .log_line("no runtime logs were captured; updater telemetry unavailable")
            .context("failed to log updater telemetry absence")?;
        return Ok(UpdateTelemetrySummary::default());
    }

    let mut summary = UpdateTelemetrySummary::default();

    for path in log_paths {
        let file = File::open(path)
            .with_context(|| format!("failed to open runtime log {}", path.display()))?;
        let reader = BufReader::new(file);
        for line in reader.lines() {
            let line = line.with_context(|| {
                format!("failed to read telemetry line from {}", path.display())
            })?;
            let trimmed = line.trim();
            let lower = trimmed.to_ascii_lowercase();
            if lower.contains("c4group returned status")
                || lower.contains("c4group killed with signal")
            {
                summary.record_failure(path.clone(), trimmed.to_string());
            } else if trimmed == "Done." {
                summary.record_success(path.clone());
            }
        }
    }

    for failure in &summary.failures {
        logger
            .log_line(&format!(
                "updater telemetry [{}]: {}",
                filename_or_display(&failure.log_path),
                failure.message
            ))
            .context("failed to log updater telemetry failure")?;
    }

    if !summary.success_sources.is_empty() {
        let sources = summary
            .success_sources
            .iter()
            .map(|path| filename_or_display(path))
            .collect::<Vec<_>>()
            .join(", ");
        logger
            .log_line(&format!("updater telemetry: success recorded in {sources}"))
            .context("failed to log updater telemetry success")?;
    }

    if summary.failures.is_empty() && summary.success_sources.is_empty() {
        logger
            .log_line("no updater telemetry found in captured runtime logs")
            .context("failed to log updater telemetry absence")?;
    }

    Ok(summary)
}

fn filename_or_display(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| path.display().to_string())
}
