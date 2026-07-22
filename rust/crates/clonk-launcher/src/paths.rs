use anyhow::{Context, Result};
use clonk_platform::AppPaths;
use std::fs;
use std::path::{Path, PathBuf};

pub fn ensure_logs_dir(paths: &AppPaths) -> Result<PathBuf> {
    let logs_dir = paths.logs_dir();
    fs::create_dir_all(logs_dir).with_context(|| {
        format!(
            "failed to ensure logs directory {} exists",
            logs_dir.display()
        )
    })?;
    Ok(logs_dir.to_path_buf())
}

pub fn relative_to_logs(path: &Path, logs_dir: &Path) -> String {
    path.strip_prefix(logs_dir)
        .map(|relative| relative.display().to_string())
        .unwrap_or_else(|_| path.display().to_string())
}

pub fn resolve_logs_entry(logs_dir: &Path, entry: &str) -> PathBuf {
    logs_dir.join(entry)
}

pub fn launcher_summary_path(logs_dir: &Path) -> PathBuf {
    logs_dir.join("launcher-summary.json")
}
