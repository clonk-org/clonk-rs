use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use thiserror::Error;

const APP_NAME: &str = "LegacyClonk";
#[cfg(target_os = "macos")]
const CONFIG_FILE_NAME: &str = "legacyclonk.config";
#[cfg(target_os = "windows")]
const CONFIG_FILE_NAME: &str = "LegacyClonk.cfg";
#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
const CONFIG_FILE_NAME: &str = "config";

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PathsError {
    #[error("LegacyClonk install root could not be located (set LC_INSTALL_ROOT to override)")]
    InstallRootNotFound,
    #[error("LegacyClonk system group not found at {path}")]
    SystemGroupMissing { path: PathBuf },
}

#[derive(Debug, Clone)]
pub struct AppPaths {
    install_root: PathBuf,
    planet_dir: PathBuf,
    system_group: PathBuf,
    content_dir: Option<PathBuf>,
    user_data_dir: PathBuf,
    cache_dir: PathBuf,
    logs_dir: PathBuf,
    temp_dir: PathBuf,
}

impl AppPaths {
    pub fn discover() -> Result<Self, PathsError> {
        let install_root = discover_install_root()?;
        let user_data_dir = discover_user_data_dir(&install_root);
        let cache_dir = discover_cache_dir(&user_data_dir);
        let logs_dir = discover_logs_dir(&user_data_dir);
        let temp_dir = discover_temp_dir();
        build_paths(install_root, user_data_dir, cache_dir, logs_dir, temp_dir)
    }

    pub fn install_root(&self) -> &Path {
        &self.install_root
    }

    pub fn planet_dir(&self) -> &Path {
        &self.planet_dir
    }

    pub fn content_dir(&self) -> Option<&Path> {
        self.content_dir.as_deref()
    }

    pub fn system_group_path(&self) -> &Path {
        &self.system_group
    }

    pub fn user_data_dir(&self) -> &Path {
        &self.user_data_dir
    }

    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    pub fn logs_dir(&self) -> &Path {
        &self.logs_dir
    }

    pub fn temp_dir(&self) -> &Path {
        &self.temp_dir
    }

    pub fn config_dir(&self) -> PathBuf {
        self.user_data_dir.join("Config")
    }

    pub fn config_file(&self) -> PathBuf {
        self.config_dir().join(CONFIG_FILE_NAME)
    }

    pub fn recordings_dir(&self) -> PathBuf {
        self.user_data_dir.join("Recordings")
    }

    pub fn playlists_dir(&self) -> PathBuf {
        self.user_data_dir.join("Playlists")
    }

    pub fn scenario_dir(&self) -> PathBuf {
        self.user_data_dir.join("Scenarios")
    }

    pub fn ensure_user_dirs(&self) -> std::io::Result<()> {
        fs::create_dir_all(&self.user_data_dir)?;
        fs::create_dir_all(self.config_dir())?;
        fs::create_dir_all(self.cache_dir())?;
        fs::create_dir_all(&self.logs_dir)?;
        Ok(())
    }
}

fn build_paths(
    install_root: PathBuf,
    user_data_dir: PathBuf,
    cache_dir: PathBuf,
    logs_dir: PathBuf,
    temp_dir: PathBuf,
) -> Result<AppPaths, PathsError> {
    let planet_dir = install_root.join("planet");
    let system_group = planet_dir.join("System.c4g");
    if !system_group.exists() {
        return Err(PathsError::SystemGroupMissing { path: system_group });
    }
    let content_dir = discover_content_dir(&install_root);
    Ok(AppPaths {
        install_root,
        planet_dir,
        system_group,
        content_dir,
        user_data_dir,
        cache_dir,
        logs_dir,
        temp_dir,
    })
}

fn discover_install_root() -> Result<PathBuf, PathsError> {
    if let Some(path) = env_path("LC_INSTALL_ROOT") {
        return Ok(path);
    }
    if let Some(path) = env_path("LC_APP_ROOT") {
        return Ok(path);
    }
    if let Some(path) = env_path("CARGO_MANIFEST_DIR") {
        if let Some(root) = find_root_starting_at(path) {
            return Ok(root);
        }
    }
    if let Ok(exe) = env::current_exe() {
        if let Some(root) = find_root_starting_at(exe) {
            return Ok(root);
        }
    }
    if let Ok(current_dir) = env::current_dir() {
        if let Some(root) = find_root_starting_at(current_dir) {
            return Ok(root);
        }
    }
    Err(PathsError::InstallRootNotFound)
}

fn find_root_starting_at(start: PathBuf) -> Option<PathBuf> {
    for ancestor in start.ancestors() {
        let candidate = ancestor.join("planet/System.c4g");
        if candidate.exists() {
            return Some(ancestor.to_path_buf());
        }
    }
    None
}

fn discover_user_data_dir(install_root: &Path) -> PathBuf {
    if let Some(override_dir) = env_path("LC_USER_DATA_DIR") {
        return override_dir;
    }
    #[cfg(target_os = "windows")]
    {
        if let Some(local_app_data) = env_path("LOCALAPPDATA") {
            return local_app_data.join(APP_NAME);
        }
        if let Some(app_data) = env_path("APPDATA") {
            return app_data.join(APP_NAME);
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Some(home) = env_path("HOME") {
            return home.join("Library/Application Support").join(APP_NAME);
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some(xdg) = env_path("XDG_DATA_HOME") {
            return xdg.join("legacyclonk");
        }
        if let Some(home) = env_path("HOME") {
            return home.join(".local/share/legacyclonk");
        }
    }
    install_root.join("user-data")
}

fn discover_cache_dir(user_data_dir: &Path) -> PathBuf {
    if let Some(cache) = env_path("LC_CACHE_DIR") {
        return cache;
    }
    user_data_dir.join("Cache")
}

fn discover_logs_dir(user_data_dir: &Path) -> PathBuf {
    if let Some(logs) = env_path("LC_LOGS_DIR") {
        return logs;
    }
    user_data_dir.join("Logs")
}

fn discover_temp_dir() -> PathBuf {
    if let Some(temp) = env_path("LC_TEMP_DIR") {
        return temp;
    }
    env::temp_dir().join(APP_NAME)
}

fn discover_content_dir(install_root: &Path) -> Option<PathBuf> {
    if let Some(dir) = env_path("LC_CONTENT_DIR") {
        if dir.exists() {
            return Some(dir);
        }
    }
    for name in ["content", "Content", "lc-content", "LCContent"] {
        let candidate = install_root.join(name);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

fn env_path(key: &str) -> Option<PathBuf> {
    env::var_os(key)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::fs;
    use std::io::Write;
    use std::path::Path;
    use std::sync::{Mutex, MutexGuard, OnceLock};
    use tempfile::TempDir;

    pub struct EnvGuard {
        _lock: MutexGuard<'static, ()>,
        saved: Vec<(String, Option<OsString>)>,
    }

    impl EnvGuard {
        pub fn set(vars: &[(&str, Option<&Path>)]) -> Self {
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

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn touch_system_group(dir: &TempDir) {
        let planet = dir.path().join("planet");
        fs::create_dir_all(&planet).unwrap();
        let path = planet.join("System.c4g");
        let mut file = fs::File::create(path).unwrap();
        writeln!(file, "stub").unwrap();
    }

    #[test]
    fn discover_uses_env_overrides() {
        let install_dir = TempDir::new().unwrap();
        touch_system_group(&install_dir);
        let user_dir = TempDir::new().unwrap();
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.path())),
        ]);
        let paths = AppPaths::discover().unwrap();
        assert_eq!(paths.install_root(), install_dir.path());
        assert_eq!(
            paths.system_group_path(),
            install_dir.path().join("planet/System.c4g")
        );
        assert_eq!(paths.user_data_dir(), user_dir.path());
    }

    #[test]
    fn discover_reports_missing_system_group() {
        let install_dir = TempDir::new().unwrap();
        let _guard = EnvGuard::set(&[("LC_INSTALL_ROOT", Some(install_dir.path()))]);
        let result = AppPaths::discover();
        assert!(matches!(result, Err(PathsError::SystemGroupMissing { .. })));
    }

    #[test]
    fn config_file_is_nested_under_config_dir() {
        let paths = AppPaths::discover().unwrap();
        let config_file = paths.config_file();
        let config_dir = paths.config_dir();
        assert!(
            config_file.starts_with(&config_dir),
            "config file {} should live under {}",
            config_file.display(),
            config_dir.display()
        );
        assert_eq!(
            config_file.file_name().and_then(|name| name.to_str()),
            Some(CONFIG_FILE_NAME)
        );
    }

    #[test]
    fn build_paths_derives_standard_directories() {
        let install_dir = TempDir::new().unwrap();
        touch_system_group(&install_dir);
        let user_dir = install_dir.path().join("player");
        let cache_dir = install_dir.path().join("cache");
        let logs_dir = install_dir.path().join("logs");
        let temp_dir = install_dir.path().join("tmp");
        let paths = super::build_paths(
            install_dir.path().to_path_buf(),
            user_dir.clone(),
            cache_dir.clone(),
            logs_dir.clone(),
            temp_dir.clone(),
        )
        .unwrap();
        assert_eq!(paths.install_root(), install_dir.path());
        assert_eq!(paths.user_data_dir(), user_dir);
        assert_eq!(paths.cache_dir(), cache_dir);
        assert_eq!(paths.logs_dir(), logs_dir);
        assert_eq!(paths.temp_dir(), temp_dir);
        assert!(paths.content_dir().is_none());
    }

    #[test]
    fn discover_detects_content_dir() {
        let install_dir = TempDir::new().unwrap();
        touch_system_group(&install_dir);
        let content_dir = install_dir.path().join("content");
        fs::create_dir_all(&content_dir).unwrap();
        let _guard = EnvGuard::set(&[("LC_INSTALL_ROOT", Some(install_dir.path()))]);
        let paths = AppPaths::discover().unwrap();
        assert_eq!(paths.content_dir(), Some(content_dir.as_path()));
    }
}
