use crate::paths::ensure_logs_dir;
use anyhow::{Context, Result};
use clonk_platform::AppPaths;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File};
use std::path::{Path, PathBuf};

const PREFERENCES_FILE: &str = "launcher-shell-preferences.json";

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct LauncherPreferences {
    #[serde(default)]
    pub last_bundle_destination: Option<String>,
    #[serde(default)]
    pub last_upload_destination: Option<String>,
    #[serde(default)]
    provider_overrides: HashMap<String, String>,
    #[serde(default)]
    report_search: Option<ReportSearchPreferences>,
}

impl LauncherPreferences {
    pub fn bundle_destination_path(&self) -> Option<PathBuf> {
        self.last_bundle_destination.as_ref().map(PathBuf::from)
    }

    pub fn upload_destination_path(&self) -> Option<PathBuf> {
        self.last_upload_destination.as_ref().map(PathBuf::from)
    }

    pub fn set_bundle_destination(&mut self, path: &Path) {
        self.last_bundle_destination = Some(path.display().to_string());
    }

    pub fn set_upload_destination(&mut self, path: &Path) {
        self.last_upload_destination = Some(path.display().to_string());
    }

    pub fn provider_override_path(&self, role: &str, name: &str) -> Option<PathBuf> {
        self.provider_overrides
            .get(&override_key(role, name))
            .map(PathBuf::from)
    }

    pub fn set_provider_override(&mut self, role: &str, name: &str, path: &Path) {
        self.provider_overrides
            .insert(override_key(role, name), path.display().to_string());
    }

    pub fn clear_provider_override(&mut self, role: &str, name: &str) {
        self.provider_overrides.remove(&override_key(role, name));
    }

    pub fn clear_provider_overrides_for_role(&mut self, role: &str) {
        let prefix = format!("{role}:");
        self.provider_overrides
            .retain(|key, _| !key.starts_with(&prefix));
    }

    pub fn clear_all_provider_overrides(&mut self) {
        self.provider_overrides.clear();
    }

    pub fn report_search(&self) -> Option<&ReportSearchPreferences> {
        self.report_search.as_ref()
    }

    pub fn set_report_search(&mut self, search: Option<ReportSearchPreferences>) {
        self.report_search = search;
    }
}

fn override_key(role: &str, name: &str) -> String {
    format!("{role}:{name}")
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReportSearchPreferences {
    pub query: String,
    pub highlight: ReportSearchHighlightPreference,
    #[serde(default)]
    pub active_line: Option<usize>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReportSearchHighlightPreference {
    Generic,
    Error,
    Warning,
}

pub fn load_launcher_preferences(paths: &AppPaths) -> Result<LauncherPreferences> {
    let logs_dir = ensure_logs_dir(paths)?;
    let path = logs_dir.join(PREFERENCES_FILE);
    if !path.exists() {
        return Ok(LauncherPreferences::default());
    }

    let file = File::open(&path).with_context(|| {
        format!(
            "failed to open launcher preferences file {}",
            path.display()
        )
    })?;
    let prefs: LauncherPreferences = serde_json::from_reader(file).with_context(|| {
        format!(
            "failed to parse launcher preferences file {}",
            path.display()
        )
    })?;
    Ok(prefs)
}

pub fn save_launcher_preferences(
    paths: &AppPaths,
    preferences: &LauncherPreferences,
) -> Result<PathBuf> {
    let logs_dir = ensure_logs_dir(paths)?;
    let path = logs_dir.join(PREFERENCES_FILE);
    let payload = serde_json::to_vec_pretty(preferences)
        .context("failed to serialize launcher preferences")?;
    fs::write(&path, payload).with_context(|| {
        format!(
            "failed to write launcher preferences file {}",
            path.display()
        )
    })?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;

    struct EnvGuard {
        saved: Vec<(String, Option<std::ffi::OsString>)>,
    }

    impl EnvGuard {
        fn set(vars: &[(&str, Option<&Path>)]) -> Self {
            let mut saved = Vec::with_capacity(vars.len());
            for (key, value) in vars {
                saved.push((key.to_string(), std::env::var_os(key)));
                match value {
                    Some(path) => std::env::set_var(key, path.as_os_str()),
                    None => std::env::remove_var(key),
                }
            }
            Self { saved }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (key, value) in self.saved.drain(..) {
                match value {
                    Some(v) => std::env::set_var(key, v),
                    None => std::env::remove_var(key),
                }
            }
        }
    }

    fn prepare_install_root(dir: &Path) {
        let planet_dir = dir.join("planet");
        fs::create_dir_all(&planet_dir).unwrap();
        fs::write(planet_dir.join("System.c4g"), b"stub").unwrap();
    }

    #[test]
    fn roundtrip_launcher_preferences() {
        let install_root = TempDir::new().unwrap();
        prepare_install_root(install_root.path());
        let user_dir = TempDir::new().unwrap();
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_root.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.path())),
        ]);

        let paths = AppPaths::discover().expect("paths");
        paths.ensure_user_dirs().expect("user dirs");

        let initial = load_launcher_preferences(&paths).expect("load default");
        assert_eq!(initial, LauncherPreferences::default());

        let mut updated = LauncherPreferences {
            last_bundle_destination: Some("/tmp/support-bundle".into()),
            last_upload_destination: Some("/tmp/upload-target".into()),
            ..Default::default()
        };
        updated.set_provider_override("share", "Support Share Drop", Path::new("/tmp/share"));
        updated.set_provider_override("upload", "Support Upload Drop", Path::new("/tmp/upload"));
        updated.set_report_search(Some(ReportSearchPreferences {
            query: "error".into(),
            highlight: ReportSearchHighlightPreference::Error,
            active_line: Some(42),
        }));

        let written = save_launcher_preferences(&paths, &updated).expect("write prefs");
        assert!(written.ends_with(PREFERENCES_FILE));

        let reloaded = load_launcher_preferences(&paths).expect("reload prefs");
        assert_eq!(reloaded, updated);
        assert_eq!(
            reloaded
                .bundle_destination_path()
                .as_deref()
                .map(|path| path.to_path_buf()),
            Some(PathBuf::from("/tmp/support-bundle"))
        );
        assert_eq!(
            reloaded
                .upload_destination_path()
                .as_deref()
                .map(|path| path.to_path_buf()),
            Some(PathBuf::from("/tmp/upload-target"))
        );
        assert_eq!(
            reloaded.provider_override_path("share", "Support Share Drop"),
            Some(PathBuf::from("/tmp/share"))
        );
        assert_eq!(
            reloaded.provider_override_path("upload", "Support Upload Drop"),
            Some(PathBuf::from("/tmp/upload"))
        );
        assert_eq!(
            reloaded.report_search().cloned(),
            updated.report_search().cloned()
        );
    }
}

#[test]
fn clearing_provider_overrides_removes_expected_entries() {
    let mut prefs = LauncherPreferences::default();
    prefs.set_provider_override("share", "Share Drop", Path::new("/tmp/share"));
    prefs.set_provider_override("upload", "Upload Drop", Path::new("/tmp/upload"));
    prefs.set_provider_override("custom", "Other", Path::new("/tmp/other"));

    prefs.clear_provider_overrides_for_role("share");
    assert!(
        prefs
            .provider_override_path("share", "Share Drop")
            .is_none(),
        "share override should be removed by role-specific clearing"
    );
    assert!(
        prefs
            .provider_override_path("upload", "Upload Drop")
            .is_some(),
        "non-matching overrides should be preserved"
    );
    assert!(
        prefs.provider_override_path("custom", "Other").is_some(),
        "overrides for other roles should survive role-specific clearing"
    );

    prefs.clear_all_provider_overrides();
    assert!(
        prefs
            .provider_override_path("upload", "Upload Drop")
            .is_none(),
        "clearing all overrides should remove the remaining upload override"
    );
    assert!(
        prefs.provider_override_path("custom", "Other").is_none(),
        "clearing all overrides should remove custom entries"
    );
}
