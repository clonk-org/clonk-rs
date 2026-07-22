use std::env;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use thiserror::Error;

const APP_NAME: &str = "LegacyClonk";
const SAVE_DEMO_FOLDER_NAME: &str = "Records.c4f";
const SCREENSHOT_FOLDER_NAME: &str = "Screenshots";
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
    #[error("LegacyClonk system group not found at {path} ({probe})")]
    SystemGroupMissing { path: PathBuf, probe: String },
}

#[derive(Debug, Clone)]
pub struct AppPaths {
    install_root: PathBuf,
    planet_dir: PathBuf,
    system_group: PathBuf,
    content_dir: Option<PathBuf>,
    user_data_dir: PathBuf,
    config_file: PathBuf,
    cache_dir: PathBuf,
    logs_dir: PathBuf,
    temp_dir: PathBuf,
    language_override: Option<String>,
}

impl AppPaths {
    pub fn discover() -> Result<Self, PathsError> {
        Self::discover_with_config_file(None)
    }

    /// Discovers application paths while accepting the command-line config
    /// candidate. The process-wide `LC_CONFIG_FILE` override intentionally
    /// takes precedence, matching `C4Config::Load`.
    pub fn discover_with_config_file(
        explicit_config_file: Option<&Path>,
    ) -> Result<Self, PathsError> {
        let install_root = discover_install_root()?;
        // Select the config from the platform/explicit bootstrap root first.
        // General.UserPath changes the user-data root, not the file from which
        // C4Config was loaded.
        let environment_user_data_dir = env_path("LC_USER_DATA_DIR");
        let bootstrap_user_data_dir = environment_user_data_dir
            .clone()
            .unwrap_or_else(|| discover_default_user_data_dir(&install_root));
        let config_file = discover_config_file(&bootstrap_user_data_dir, explicit_config_file);
        let user_data_dir = environment_user_data_dir.unwrap_or_else(|| {
            discover_configured_user_data_dir(&config_file, &install_root)
                .unwrap_or(bootstrap_user_data_dir)
        });
        let cache_dir = discover_cache_dir(&user_data_dir);
        let logs_dir = discover_logs_dir(&user_data_dir);
        let temp_dir = discover_temp_dir();
        let language_override = env_string("LC_LANGUAGE_OVERRIDE");
        build_paths(
            install_root,
            user_data_dir,
            config_file,
            cache_dir,
            logs_dir,
            temp_dir,
            language_override,
        )
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
        self.config_file.clone()
    }

    pub fn language_override(&self) -> Option<&str> {
        self.language_override.as_deref()
    }

    pub fn recordings_dir(&self) -> PathBuf {
        let configured = configured_general_value(&self.config_file, "SaveDemoFolder")
            .unwrap_or_else(|| SAVE_DEMO_FOLDER_NAME.to_string());
        let path = PathBuf::from(configured);
        if path.is_absolute() {
            path
        } else {
            self.install_root.join(path)
        }
    }

    pub fn screenshot_dir(&self) -> PathBuf {
        let configured = configured_general_value(&self.config_file, "ScreenshotFolder")
            .unwrap_or_else(|| SCREENSHOT_FOLDER_NAME.to_string());
        self.install_root.join(configured.trim())
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
        if let Some(parent) = self
            .config_file
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)?;
        }
        fs::create_dir_all(self.cache_dir())?;
        fs::create_dir_all(&self.logs_dir)?;
        Ok(())
    }
}

fn build_paths(
    install_root: PathBuf,
    user_data_dir: PathBuf,
    config_file: PathBuf,
    cache_dir: PathBuf,
    logs_dir: PathBuf,
    temp_dir: PathBuf,
    language_override: Option<String>,
) -> Result<AppPaths, PathsError> {
    let planet_dir = install_root.join("planet");
    let system_group = planet_dir.join("System.c4g");
    // Keep the concrete io::Error instead of an exists() collapse: a transient
    // EMFILE/EACCES/ENOTDIR here reads completely differently from ENOENT.
    if let Err(error) = fs::metadata(&system_group) {
        return Err(PathsError::SystemGroupMissing {
            path: system_group,
            probe: format!("{:?}: {error}", error.kind()),
        });
    }
    let content_dir = discover_content_dir(&install_root);
    Ok(AppPaths {
        install_root,
        planet_dir,
        system_group,
        content_dir,
        user_data_dir,
        config_file,
        cache_dir,
        logs_dir,
        temp_dir,
        language_override,
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

fn discover_default_user_data_dir(install_root: &Path) -> PathBuf {
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

fn discover_configured_user_data_dir(config_file: &Path, install_root: &Path) -> Option<PathBuf> {
    let configured = configured_general_value(config_file, "UserPath")?;
    if configured.is_empty() {
        return None;
    }
    let expanded = expand_user_path_environment(&configured);
    let path = PathBuf::from(expanded);
    Some(if path.is_absolute() {
        path
    } else {
        // Native startup makes ExePath the working directory before relative
        // config paths are evaluated.
        install_root.join(path)
    })
}

fn configured_general_value(config_file: &Path, key: &str) -> Option<String> {
    let bytes = fs::read(config_file).ok()?;
    if bytes.contains(&0) {
        return None;
    }
    let mut projected = lc_core::std_buf::StdStrBuf::new();
    projected.copy_bytes(&bytes);
    projected.ensure_unicode();
    let mut reader = Cursor::new(projected.as_bytes());
    let config = lc_core::std_config::Config::from_reader(&mut reader).ok()?;
    config
        .get_in(Some("General"), key)
        .or_else(|| config.get_in(None, key))
        .map(str::to_string)
}

#[cfg(not(target_os = "windows"))]
fn expand_user_path_environment(configured: &str) -> String {
    let Some(home) = env::var("HOME").ok().filter(|home| !home.is_empty()) else {
        return configured.to_string();
    };
    configured.replacen("$HOME", &home, 1)
}

#[cfg(target_os = "windows")]
fn expand_user_path_environment(configured: &str) -> String {
    let mut expanded = configured.to_string();
    let mut cursor = 0;
    while let Some(start_offset) = expanded[cursor..].find('%') {
        let start = cursor + start_offset;
        let Some(end_offset) = expanded[start + 1..].find('%') else {
            break;
        };
        let end = start + 1 + end_offset;
        let name = &expanded[start + 1..end];
        let Some(value) = env::var_os(name) else {
            cursor = end + 1;
            continue;
        };
        let value = value.to_string_lossy();
        expanded.replace_range(start..=end, &value);
        cursor = start + value.len();
    }
    expanded
}

fn discover_cache_dir(user_data_dir: &Path) -> PathBuf {
    if let Some(cache) = env_path("LC_CACHE_DIR") {
        return cache;
    }
    user_data_dir.join("Cache")
}

fn discover_config_file(user_data_dir: &Path, explicit_config_file: Option<&Path>) -> PathBuf {
    env_path("LC_CONFIG_FILE")
        .or_else(|| explicit_config_file.map(Path::to_path_buf))
        .unwrap_or_else(|| user_data_dir.join("Config").join(CONFIG_FILE_NAME))
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

fn env_string(key: &str) -> Option<String> {
    env::var(key).ok().filter(|value| !value.is_empty())
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
        match result {
            Err(PathsError::SystemGroupMissing { probe, .. }) => {
                assert!(
                    probe.contains("NotFound"),
                    "probe must carry the concrete io error, got {probe:?}"
                );
            }
            other => panic!("expected SystemGroupMissing, got {other:?}"),
        }
    }

    #[test]
    fn discover_reports_concrete_system_group_probe_error() {
        let install_dir = TempDir::new().unwrap();
        // A regular file where the planet directory is expected turns the
        // System.c4g stat into ENOTDIR rather than ENOENT; the error must
        // surface which one actually happened.
        fs::write(install_dir.path().join("planet"), b"not a directory").unwrap();
        let _guard = EnvGuard::set(&[("LC_INSTALL_ROOT", Some(install_dir.path()))]);
        let result = AppPaths::discover();
        match result {
            Err(PathsError::SystemGroupMissing { probe, .. }) => {
                assert!(
                    probe.contains("os error 20"),
                    "probe must carry the concrete ENOTDIR io error, got {probe:?}"
                );
            }
            other => panic!("expected SystemGroupMissing, got {other:?}"),
        }
    }

    #[test]
    fn config_file_is_nested_under_config_dir() {
        let install_dir = TempDir::new().unwrap();
        touch_system_group(&install_dir);
        let user_dir = TempDir::new().unwrap();
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.path())),
            ("LC_CONFIG_FILE", None),
            ("LC_LANGUAGE_OVERRIDE", None),
        ]);
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
            user_dir.join("Config").join(CONFIG_FILE_NAME),
            cache_dir.clone(),
            logs_dir.clone(),
            temp_dir.clone(),
            None,
        )
        .unwrap();
        assert_eq!(paths.install_root(), install_dir.path());
        assert_eq!(paths.user_data_dir(), user_dir);
        assert_eq!(paths.cache_dir(), cache_dir);
        assert_eq!(paths.logs_dir(), logs_dir);
        assert_eq!(paths.temp_dir(), temp_dir);
        assert!(paths.content_dir().is_none());
        assert_eq!(paths.language_override(), None);
    }

    #[test]
    fn discover_captures_language_override() {
        let install_dir = TempDir::new().unwrap();
        touch_system_group(&install_dir);
        let user_dir = TempDir::new().unwrap();
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.path())),
            ("LC_CONFIG_FILE", None),
            ("LC_LANGUAGE_OVERRIDE", Some(Path::new("DE,US"))),
        ]);

        let paths = AppPaths::discover().unwrap();
        env::set_var("LC_LANGUAGE_OVERRIDE", "FR");

        assert_eq!(paths.language_override(), Some("DE,US"));
    }

    #[test]
    fn l005_environment_config_file_precedes_explicit_candidate() {
        let install_dir = TempDir::new().unwrap();
        touch_system_group(&install_dir);
        let user_dir = TempDir::new().unwrap();
        let override_dir = TempDir::new().unwrap();
        let environment_file = override_dir.path().join("environment.config");
        let explicit_file = override_dir.path().join("explicit.config");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.path())),
            ("LC_CONFIG_FILE", Some(environment_file.as_path())),
        ]);

        let paths = AppPaths::discover_with_config_file(Some(&explicit_file)).unwrap();

        assert_eq!(paths.config_file(), environment_file);
    }

    #[test]
    fn l005_explicit_config_file_precedes_default_and_creates_its_parent() {
        let install_dir = TempDir::new().unwrap();
        touch_system_group(&install_dir);
        let user_dir = TempDir::new().unwrap();
        let explicit_file = user_dir.path().join("nested/custom.config");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.path())),
            ("LC_CONFIG_FILE", None),
        ]);

        let paths = AppPaths::discover_with_config_file(Some(&explicit_file)).unwrap();
        paths.ensure_user_dirs().unwrap();

        assert_eq!(paths.config_file(), explicit_file);
        assert!(explicit_file.parent().unwrap().is_dir());
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn l016_selected_config_user_path_expands_without_relocating_config() {
        let install_dir = TempDir::new().unwrap();
        touch_system_group(&install_dir);
        let home_dir = TempDir::new().unwrap();
        let config_dir = TempDir::new().unwrap();
        let environment_file = config_dir.path().join("environment.config");
        let explicit_file = config_dir.path().join("explicit.config");
        fs::write(
            &environment_file,
            "[General]\nUserPath=\"$HOME/Legacy Data\"\n",
        )
        .unwrap();
        fs::write(
            &explicit_file,
            "[General]\nUserPath=\"$HOME/Wrong Data\"\n",
        )
        .unwrap();
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", None),
            ("LC_CONFIG_FILE", Some(environment_file.as_path())),
            ("LC_CACHE_DIR", None),
            ("LC_LOGS_DIR", None),
            ("HOME", Some(home_dir.path())),
        ]);

        let paths = AppPaths::discover_with_config_file(Some(&explicit_file)).unwrap();

        assert_eq!(paths.config_file(), environment_file);
        assert_eq!(paths.user_data_dir(), home_dir.path().join("Legacy Data"));
        assert_eq!(paths.cache_dir(), home_dir.path().join("Legacy Data/Cache"));
        assert_eq!(paths.logs_dir(), home_dir.path().join("Legacy Data/Logs"));
        paths.ensure_user_dirs().unwrap();
        assert!(home_dir.path().join("Legacy Data/Config").is_dir());
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn l016_rust_user_data_override_precedes_config_user_path() {
        let install_dir = TempDir::new().unwrap();
        touch_system_group(&install_dir);
        let user_dir = TempDir::new().unwrap();
        let configured_dir = TempDir::new().unwrap();
        let config_dir = TempDir::new().unwrap();
        let config_file = config_dir.path().join("explicit.config");
        fs::write(
            &config_file,
            format!(
                "[General]\nUserPath=\"{}\"\n",
                configured_dir.path().display()
            ),
        )
        .unwrap();
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.path())),
            ("LC_CONFIG_FILE", None),
            ("HOME", None),
        ]);

        let paths = AppPaths::discover_with_config_file(Some(&config_file)).unwrap();

        assert_eq!(paths.config_file(), config_file);
        assert_eq!(paths.user_data_dir(), user_dir.path());
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
