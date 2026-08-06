use std::env;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use thiserror::Error;

pub const PRODUCT_NAME: &str = "Clonk Rust";
/// `C4ENGINECAPTION` / `STD_PRODUCT` (C4Version.h:19,24): the caption
/// `C4FullScreen` gives its carrier window (C4FullScreen.cpp:474-480). This is
/// deliberately *not* `PRODUCT_NAME`, which names the port's user-data
/// directories and must keep identifying the Rust build.
pub const ENGINE_CAPTION: &str = "LegacyClonk";
/// `C4EDITORCAPTION` is "Clonk Editor"; the port's developer console is a
/// different surface, so it keeps its own caption built from the engine one.
pub const CONSOLE_CAPTION: &str = "LegacyClonk Console";
pub const PRODUCT_SLUG: &str = "clonk-rust";
pub const PRODUCT_COMPACT_NAME: &str = "ClonkRust";
// Compatibility-only names for profiles created before the product rename.
#[cfg(any(target_os = "macos", target_os = "windows"))]
const LEGACY_STORAGE_NAME: &str = "LegacyClonk";
#[cfg(all(unix, not(target_os = "macos")))]
const LEGACY_STORAGE_SLUG: &str = "legacyclonk";
const SAVE_DEMO_FOLDER_NAME: &str = "Records.c4f";
const SCREENSHOT_FOLDER_NAME: &str = "Screenshots";
#[cfg(target_os = "macos")]
const CONFIG_FILE_NAME: &str = "clonk-rust.config";
#[cfg(target_os = "macos")]
const LEGACY_CONFIG_FILE_NAME: &str = "legacyclonk.config";
#[cfg(target_os = "windows")]
const CONFIG_FILE_NAME: &str = "ClonkRust.cfg";
#[cfg(target_os = "windows")]
const LEGACY_CONFIG_FILE_NAME: &str = "LegacyClonk.cfg";
#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
const CONFIG_FILE_NAME: &str = "config";
#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
const LEGACY_CONFIG_FILE_NAME: &str = "config";

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PathsError {
    #[error("Clonk Rust install root could not be located (set LC_INSTALL_ROOT to override)")]
    InstallRootNotFound,
    #[error("Clonk Rust system group not found at {path} ({probe})")]
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
        let install_root = discover_unvalidated_install_root()?;
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

    /// The directory holding the shipped executables.
    ///
    /// A macOS install root *is* the bundle's `Contents/Resources`, so its
    /// executables sit in the sibling `Contents/MacOS` and no `bin` exists.
    pub fn binaries_dir(&self) -> PathBuf {
        binaries_dir_for(&self.install_root)
    }

    /// The enclosing `.app` directory when the install root is a macOS bundle.
    pub fn macos_bundle_root(&self) -> Option<PathBuf> {
        macos_bundle_root_for(&self.install_root)
    }

    pub fn content_dir(&self) -> Option<&Path> {
        self.content_dir.as_deref()
    }

    /// The directory C++ resolves an `ExePath`-relative *data* path against.
    ///
    /// `C4Config::AtExePath` (`C4Config.cpp:1344-1349`) and its inverse
    /// `ForceRelativePath` (`C4Config.cpp:1438-1459`) share one
    /// `General.ExePath`, and the `.c4f`/`.c4d` groups sit directly in it.
    /// A source checkout interposes `content/`, so that is the ExePath data
    /// layout here; a packaged layout has none and uses the install root.
    pub fn executable_data_root(&self) -> &Path {
        self.content_dir.as_deref().unwrap_or(&self.install_root)
    }

    /// The data roots probed for an `ExePath`-relative group path, outermost
    /// layout first.
    ///
    /// C++ has a single `ExePath`, so it needs no such list. This port splits
    /// it across roots that a source checkout and a packaged install spell
    /// differently, and a scenario `Origin` written under one of them has to
    /// keep resolving under the others.
    pub fn executable_data_roots(&self) -> Vec<PathBuf> {
        let mut roots = Vec::new();
        for root in [
            self.executable_data_root(),
            self.planet_dir(),
            self.install_root(),
            self.system_group_path(),
        ] {
            if !roots.iter().any(|seen: &PathBuf| seen == root) {
                roots.push(root.to_path_buf());
            }
        }
        roots
    }

    pub fn system_group_path(&self) -> &Path {
        &self.system_group
    }

    /// `C4Config::AtUserPath` (`C4Config.cpp:1351-1357`): resolve `filename`
    /// against `General.UserPath`, **re-reading and re-expanding it on every
    /// call**. C++ never caches this, so a `UserPath` or environment change
    /// made while the game runs moves later lookups.
    ///
    /// [`Self::user_data_dir`] is the cached counterpart, resolved once at
    /// discovery — use it for anything that must stay put for the session
    /// (the session log, the cache), and this for the paths C++ resolves
    /// through `AtUserPath`.
    pub fn at_user_path(&self, filename: &str) -> PathBuf {
        let root = discover_configured_user_data_dir(&self.config_file, &self.install_root)
            .unwrap_or_else(|| self.user_data_dir.clone());
        if filename.is_empty() {
            root
        } else {
            root.join(filename)
        }
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
        // C4Config.cpp:1326-1332 appends `ScreenshotFolder` to ExePath verbatim;
        // trimming it here would silently accept a value C++ keeps as written.
        self.install_root.join(configured)
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

/// macOS Gatekeeper runs a freshly downloaded, quarantined bundle from a
/// read-only `AppTranslocation` mount, so `current_exe` points at a copy whose
/// siblings are gone. C++ recovers the original location by resolving
/// `SecTranslocateIsTranslocatedURL`/`SecTranslocateCreateOriginalPathForURL`
/// out of Security.framework at run time (MacAppTranslocation.cpp:27-63) —
/// dynamically, so a system without those symbols simply reports "not
/// translocated" instead of failing to launch.
#[cfg(target_os = "macos")]
mod translocation {
    use std::ffi::{c_char, c_void, CString};
    use std::path::{Path, PathBuf};

    type CFTypeRef = *const c_void;
    type CFStringRef = *const c_void;
    type CFUrlRef = *const c_void;
    type CFIndex = isize;

    const RTLD_LAZY: i32 = 1;
    const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
    const K_CF_URL_POSIX_PATH_STYLE: CFIndex = 0;

    extern "C" {
        fn dlopen(filename: *const c_char, flag: i32) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
        fn dlclose(handle: *mut c_void) -> i32;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFStringCreateWithBytes(
            allocator: CFTypeRef,
            bytes: *const u8,
            num_bytes: CFIndex,
            encoding: u32,
            is_external_representation: u8,
        ) -> CFStringRef;
        fn CFURLCreateWithFileSystemPath(
            allocator: CFTypeRef,
            file_path: CFStringRef,
            path_style: CFIndex,
            is_directory: u8,
        ) -> CFUrlRef;
        fn CFURLCopyFileSystemPath(url: CFUrlRef, path_style: CFIndex) -> CFStringRef;
        fn CFStringGetLength(string: CFStringRef) -> CFIndex;
        fn CFStringGetCString(
            string: CFStringRef,
            buffer: *mut c_char,
            buffer_size: CFIndex,
            encoding: u32,
        ) -> u8;
        fn CFRelease(cf: CFTypeRef);
    }

    type IsTranslocatedFn = unsafe extern "C" fn(CFUrlRef, *mut bool, *mut CFTypeRef) -> u8;
    type OriginalPathFn = unsafe extern "C" fn(CFUrlRef, *mut CFTypeRef) -> CFUrlRef;

    /// Returns the bundle's original executable path when the running copy is
    /// translocated, and `None` in every other case — not translocated, the
    /// symbols are unavailable, or any step fails. `None` means "carry on with
    /// the path we already have", exactly like the C++ `std::optional`.
    pub(super) fn original_executable_path(executable: &Path) -> Option<PathBuf> {
        let path = executable.to_str()?;
        let library =
            CString::new("/System/Library/Frameworks/Security.framework/Security").ok()?;
        let is_translocated_symbol = CString::new("SecTranslocateIsTranslocatedURL").ok()?;
        let original_path_symbol = CString::new("SecTranslocateCreateOriginalPathForURL").ok()?;

        // SAFETY: every raw pointer below is either checked against null before
        // use or owned by this scope and released on the way out. The two
        // resolved symbols are called with exactly the signatures Security.framework
        // documents for them.
        unsafe {
            let handle = dlopen(library.as_ptr(), RTLD_LAZY);
            if handle.is_null() {
                return None;
            }
            let result = (|| {
                let is_translocated: IsTranslocatedFn = {
                    let symbol = dlsym(handle, is_translocated_symbol.as_ptr());
                    if symbol.is_null() {
                        return None;
                    }
                    std::mem::transmute(symbol)
                };
                let url = url_for_path(path)?;
                let mut translocated = false;
                let probed = is_translocated(url, &mut translocated, std::ptr::null_mut());
                if probed == 0 || !translocated {
                    CFRelease(url);
                    return None;
                }
                let original_path: OriginalPathFn = {
                    let symbol = dlsym(handle, original_path_symbol.as_ptr());
                    if symbol.is_null() {
                        CFRelease(url);
                        return None;
                    }
                    std::mem::transmute(symbol)
                };
                let original = original_path(url, std::ptr::null_mut());
                CFRelease(url);
                if original.is_null() {
                    return None;
                }
                let recovered = path_for_url(original);
                CFRelease(original);
                recovered.map(PathBuf::from)
            })();
            dlclose(handle);
            result
        }
    }

    /// SAFETY: the caller releases the returned URL.
    unsafe fn url_for_path(path: &str) -> Option<CFUrlRef> {
        let string = CFStringCreateWithBytes(
            std::ptr::null(),
            path.as_ptr(),
            path.len() as CFIndex,
            K_CF_STRING_ENCODING_UTF8,
            0,
        );
        if string.is_null() {
            return None;
        }
        let url =
            CFURLCreateWithFileSystemPath(std::ptr::null(), string, K_CF_URL_POSIX_PATH_STYLE, 0);
        CFRelease(string);
        (!url.is_null()).then_some(url)
    }

    /// C++ reads the recovered URL with `CFStringGetCStringPtr`, which returns
    /// null whenever the string is not already UTF-8 backed and throws there.
    /// `CFStringGetCString` copies instead, so an unusual encoding recovers the
    /// path rather than aborting startup.
    unsafe fn path_for_url(url: CFUrlRef) -> Option<String> {
        let string = CFURLCopyFileSystemPath(url, K_CF_URL_POSIX_PATH_STYLE);
        if string.is_null() {
            return None;
        }
        // Worst case UTF-8 is 3 bytes per UTF-16 unit, plus the terminator.
        let capacity = CFStringGetLength(string)
            .saturating_mul(3)
            .saturating_add(1);
        let mut buffer = vec![0 as c_char; capacity.max(1) as usize];
        let copied = CFStringGetCString(
            string,
            buffer.as_mut_ptr(),
            capacity.max(1),
            K_CF_STRING_ENCODING_UTF8,
        );
        CFRelease(string);
        if copied == 0 {
            return None;
        }
        let bytes = buffer
            .iter()
            .take_while(|byte| **byte != 0)
            .map(|byte| *byte as u8)
            .collect::<Vec<_>>();
        String::from_utf8(bytes).ok()
    }
}

/// The executable path install discovery should use: the original bundle
/// location when macOS translocated this copy, otherwise the path as given
/// (MacAppTranslocation.cpp:27-63; C4WinMain.cpp:233-238).
#[cfg(target_os = "macos")]
pub fn non_translocated_executable(executable: &Path) -> PathBuf {
    translocation::original_executable_path(executable).unwrap_or_else(|| executable.to_path_buf())
}

/// `main` chdirs before anything else runs (C4WinMain.cpp:233-238). Doing the
/// same keeps relative classic paths resolving against the bundle's parent even
/// when Finder launched the app with an unrelated working directory.
#[cfg(target_os = "macos")]
pub fn establish_macos_bundle_working_directory() {
    let Ok(executable) = env::current_exe() else {
        return;
    };
    let executable = non_translocated_executable(&executable);
    let Some(directory) = macos_bundle_working_directory(&executable) else {
        return;
    };
    if let Err(error) = env::set_current_dir(&directory) {
        // C++ ignores the chdir result; discovery below still walks ancestors.
        let _ = error;
    }
}

/// The directory C++ makes current before startup: `dirname` four times over
/// the (non-translocated) executable, i.e. the directory holding `X.app`
/// (C4WinMain.cpp:238). Returns `None` when the path has fewer components,
/// which is the case for a plain non-bundled binary.
#[cfg(target_os = "macos")]
pub fn macos_bundle_working_directory(executable: &Path) -> Option<PathBuf> {
    let mut directory = executable;
    for _ in 0..4 {
        directory = directory.parent()?;
    }
    Some(directory.to_path_buf())
}

/// Locates the installation without requiring any game-data file to exist.
///
/// Update recovery must run before [`AppPaths`] validates `planet/System.c4g`:
/// an interrupted directory swap can temporarily leave that exact path absent.
/// Environment overrides and packaged executable shapes therefore identify the
/// root on their own; the existing data-based ancestor search remains a
/// development-layout fallback.
pub fn discover_unvalidated_install_root() -> Result<PathBuf, PathsError> {
    if let Some(path) = env_path("LC_INSTALL_ROOT") {
        return Ok(path);
    }
    if let Some(path) = env_path("LC_APP_ROOT") {
        return Ok(path);
    }
    let executable = env::current_exe().ok();
    #[cfg(target_os = "macos")]
    let executable = executable.map(|executable| non_translocated_executable(&executable));
    let manifest = env_path("CARGO_MANIFEST_DIR");
    let current_dir = env::current_dir().ok();
    if let Some(root) = install_root_from_candidates(
        executable.as_deref(),
        manifest.as_deref(),
        current_dir.as_deref(),
    ) {
        return Ok(root);
    }
    Err(PathsError::InstallRootNotFound)
}

fn install_root_from_candidates(
    executable: Option<&Path>,
    manifest: Option<&Path>,
    current_dir: Option<&Path>,
) -> Option<PathBuf> {
    manifest
        .and_then(|path| find_root_starting_at(path.to_path_buf()))
        .or_else(|| executable.and_then(|path| find_root_starting_at(path.to_path_buf())))
        .or_else(|| current_dir.and_then(|path| find_root_starting_at(path.to_path_buf())))
        .or_else(|| executable.and_then(install_root_from_executable_shape))
}

/// Recognises the two shipped executable layouts without probing mutable data.
fn install_root_from_executable_shape(executable: &Path) -> Option<PathBuf> {
    let executable_dir = executable.parent()?;
    if executable_dir.file_name()? == "bin" {
        return executable_dir.parent().map(Path::to_path_buf);
    }
    if executable_dir.file_name()? != "MacOS" {
        return None;
    }
    let contents = executable_dir.parent()?;
    let bundle = contents.parent()?;
    (contents.file_name()? == "Contents" && bundle.extension()? == "app")
        .then(|| contents.join("Resources"))
}

/// The `Contents` directory when `install_root` is a bundle's
/// `Contents/Resources`.
///
/// Keyed on the path shape rather than the host, so the bundle layout stays
/// reachable from tests on every platform.
fn macos_bundle_contents(install_root: &Path) -> Option<&Path> {
    let contents = install_root.parent()?;
    (install_root.file_name()? == "Resources" && contents.file_name()? == "Contents")
        .then_some(contents)
}

fn binaries_dir_for(install_root: &Path) -> PathBuf {
    macos_bundle_contents(install_root)
        .map(|contents| contents.join("MacOS"))
        .unwrap_or_else(|| install_root.join("bin"))
}

fn macos_bundle_root_for(install_root: &Path) -> Option<PathBuf> {
    macos_bundle_contents(install_root)
        .and_then(Path::parent)
        .map(Path::to_path_buf)
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
            return prefer_existing_legacy_path(
                local_app_data.join(PRODUCT_NAME),
                local_app_data.join(LEGACY_STORAGE_NAME),
            );
        }
        if let Some(app_data) = env_path("APPDATA") {
            return prefer_existing_legacy_path(
                app_data.join(PRODUCT_NAME),
                app_data.join(LEGACY_STORAGE_NAME),
            );
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Some(home) = env_path("HOME") {
            let app_support = home.join("Library/Application Support");
            return prefer_existing_legacy_path(
                app_support.join(PRODUCT_NAME),
                app_support.join(LEGACY_STORAGE_NAME),
            );
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some(xdg) = env_path("XDG_DATA_HOME") {
            return prefer_existing_legacy_path(
                xdg.join(PRODUCT_SLUG),
                xdg.join(LEGACY_STORAGE_SLUG),
            );
        }
        if let Some(home) = env_path("HOME") {
            let local_share = home.join(".local/share");
            return prefer_existing_legacy_path(
                local_share.join(PRODUCT_SLUG),
                local_share.join(LEGACY_STORAGE_SLUG),
            );
        }
    }
    install_root.join("user-data")
}

fn prefer_existing_legacy_path(preferred: PathBuf, legacy: PathBuf) -> PathBuf {
    if !preferred.exists() && legacy.exists() {
        legacy
    } else {
        preferred
    }
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
    let mut projected = clonk_core::std_buf::StdStrBuf::new();
    projected.copy_bytes(&bytes);
    projected.ensure_unicode();
    let mut reader = Cursor::new(projected.as_bytes());
    let config = clonk_core::std_config::Config::from_reader(&mut reader).ok()?;
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
        .unwrap_or_else(|| default_config_file(user_data_dir))
}

fn default_config_file(user_data_dir: &Path) -> PathBuf {
    let config_dir = user_data_dir.join("Config");
    prefer_existing_legacy_path(
        config_dir.join(CONFIG_FILE_NAME),
        config_dir.join(LEGACY_CONFIG_FILE_NAME),
    )
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
    env::temp_dir().join(PRODUCT_SLUG)
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
    fn binaries_live_beside_a_plain_install_root() {
        let root = Path::new("/opt/clonk-rust");
        assert_eq!(binaries_dir_for(root), root.join("bin"));
        assert_eq!(macos_bundle_root_for(root), None);
    }

    #[test]
    fn binaries_live_in_the_bundle_macos_directory() {
        // A macOS install root *is* `Contents/Resources`, so the executables
        // are in the sibling `Contents/MacOS` and there is no `bin`.
        let root = Path::new("/Applications/Clonk Rust.app/Contents/Resources");
        assert_eq!(
            binaries_dir_for(root),
            Path::new("/Applications/Clonk Rust.app/Contents/MacOS")
        );
        assert_eq!(
            macos_bundle_root_for(root).as_deref(),
            Some(Path::new("/Applications/Clonk Rust.app"))
        );
    }

    #[test]
    fn unvalidated_discovery_derives_a_plain_root_from_its_bin_executable() {
        let executable = Path::new("/opt/clonk-rust/bin/clonk-game");

        assert_eq!(
            install_root_from_executable_shape(executable).as_deref(),
            Some(Path::new("/opt/clonk-rust"))
        );
    }

    #[test]
    fn validated_development_root_precedes_an_unrelated_bin_executable_shape() {
        let directory = TempDir::new().expect("candidate roots");
        let development = directory.path().join("workspace");
        fs::create_dir_all(development.join("planet/System.c4g"))
            .expect("development system group");
        let manifest = development.join("crates/clonk-platform");
        fs::create_dir_all(&manifest).expect("manifest directory");
        let unrelated = directory.path().join("usr/bin/clonk-app");

        assert_eq!(
            install_root_from_candidates(Some(&unrelated), Some(&manifest), None).as_deref(),
            Some(development.as_path())
        );
    }

    #[test]
    fn unvalidated_discovery_derives_bundle_resources_without_game_data() {
        let executable = Path::new("/Applications/Clonk Rust.app/Contents/MacOS/clonk-app");

        assert_eq!(
            install_root_from_executable_shape(executable).as_deref(),
            Some(Path::new("/Applications/Clonk Rust.app/Contents/Resources"))
        );
    }

    #[test]
    fn a_resources_directory_outside_a_bundle_is_not_treated_as_one() {
        let root = Path::new("/srv/game/Resources");
        assert_eq!(binaries_dir_for(root), root.join("bin"));
        assert_eq!(macos_bundle_root_for(root), None);
    }

    #[test]
    fn product_identity_is_clonk_rust() {
        assert_eq!(PRODUCT_NAME, "Clonk Rust");
        assert_eq!(PRODUCT_SLUG, "clonk-rust");
        assert_eq!(PRODUCT_COMPACT_NAME, "ClonkRust");
    }

    #[test]
    fn existing_legacy_profile_is_used_until_clonk_rust_profile_exists() {
        let profiles = TempDir::new().unwrap();
        let preferred = profiles.path().join("Clonk Rust");
        let legacy = profiles.path().join("LegacyClonk");
        fs::create_dir_all(&legacy).unwrap();

        assert_eq!(
            prefer_existing_legacy_path(preferred.clone(), legacy.clone()),
            legacy
        );

        fs::create_dir_all(&preferred).unwrap();
        assert_eq!(
            prefer_existing_legacy_path(preferred.clone(), legacy),
            preferred
        );
    }

    #[test]
    fn existing_legacy_config_is_used_until_clonk_rust_config_exists() {
        let profile = TempDir::new().unwrap();
        let config_dir = profile.path().join("Config");
        fs::create_dir_all(&config_dir).unwrap();
        let preferred = config_dir.join(CONFIG_FILE_NAME);
        let legacy = config_dir.join(LEGACY_CONFIG_FILE_NAME);
        fs::write(&legacy, b"[General]\n").unwrap();

        assert_eq!(default_config_file(profile.path()), legacy);

        fs::write(&preferred, b"[General]\n").unwrap();
        assert_eq!(default_config_file(profile.path()), preferred);
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
    fn unvalidated_discovery_accepts_an_explicit_root_without_the_system_group() {
        let install_dir = TempDir::new().unwrap();
        let _guard = EnvGuard::set(&[("LC_INSTALL_ROOT", Some(install_dir.path()))]);

        assert_eq!(
            discover_unvalidated_install_root().expect("discover install root before validation"),
            install_dir.path()
        );
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
                // ENOTDIR is errno 20 on unix; Windows reports its own code for
                // the same condition, so only the concreteness is portable.
                #[cfg(unix)]
                assert!(
                    probe.contains("os error 20"),
                    "probe must carry the concrete ENOTDIR io error, got {probe:?}"
                );
                #[cfg(not(unix))]
                assert!(
                    probe.contains("os error"),
                    "probe must carry a concrete io error, got {probe:?}"
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
    fn environment_config_file_precedes_explicit_candidate() {
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
    fn explicit_config_file_precedes_default_and_creates_its_parent() {
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
    // C4Config.cpp:1351-1357 — `AtUserPath` re-reads `General.UserPath` and
    // re-expands the environment on every call, so a change made while the game
    // runs moves later lookups. `user_data_dir()` stays where discovery put it.
    #[test]
    fn at_user_path_reexpands_live_user_path_and_environment() {
        let install_dir = TempDir::new().unwrap();
        touch_system_group(&install_dir);
        let home_dir = TempDir::new().unwrap();
        let config_dir = TempDir::new().unwrap();
        let config_file = config_dir.path().join("clonk.config");
        fs::write(&config_file, "[General]\nUserPath=\"$HOME/First\"\n").unwrap();
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", None),
            ("LC_CONFIG_FILE", Some(config_file.as_path())),
            ("LC_CACHE_DIR", None),
            ("LC_LOGS_DIR", None),
            ("HOME", Some(home_dir.path())),
        ]);

        let paths = AppPaths::discover_with_config_file(None).unwrap();
        let first = home_dir.path().join("First");
        assert_eq!(paths.user_data_dir(), first);
        // An empty filename yields the root itself, like `AtUserPath("")`
        // (C4Config.cpp:1337).
        assert_eq!(paths.at_user_path(""), first);
        assert_eq!(paths.at_user_path("Clonk.png"), first.join("Clonk.png"));

        // The config changes while the game runs.
        fs::write(&config_file, "[General]\nUserPath=\"$HOME/Second\"\n").unwrap();
        let second = home_dir.path().join("Second");
        assert_eq!(
            paths.at_user_path("Clonk.png"),
            second.join("Clonk.png"),
            "AtUserPath must re-read UserPath rather than cache it"
        );
        // The cached root is deliberately unmoved: the session log and cache
        // stay where discovery put them.
        assert_eq!(paths.user_data_dir(), first);

        // The environment is re-expanded too, not just the config text. The
        // guard above already holds the env lock and captured HOME, so it is
        // set directly here and restored on drop — a second EnvGuard would
        // deadlock on the same static mutex.
        let moved_home = TempDir::new().unwrap();
        std::env::set_var("HOME", moved_home.path());
        assert_eq!(
            paths.at_user_path("Clonk.png"),
            moved_home.path().join("Second").join("Clonk.png")
        );
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn selected_config_user_path_expands_without_relocating_config() {
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
        fs::write(&explicit_file, "[General]\nUserPath=\"$HOME/Wrong Data\"\n").unwrap();
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
    fn rust_user_data_override_precedes_config_user_path() {
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

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_bundle_resources_are_the_unvalidated_install_root() {
        // A packaged `.app` keeps its executable in `Contents/MacOS` and its
        // game data in the sibling `Contents/Resources`, so no ancestor of the
        // executable holds `planet/System.c4g`.
        let bundle = TempDir::new().unwrap();
        let contents = bundle.path().join("Clonk Rust.app/Contents");
        let executable = contents.join("MacOS/clonk-app");
        let resources = contents.join("Resources");
        fs::create_dir_all(executable.parent().unwrap()).unwrap();
        fs::write(&executable, b"runtime").unwrap();

        assert_eq!(
            install_root_from_executable_shape(&executable),
            Some(resources),
            "the bundle's Resources directory is the install root"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_plain_unix_executable_resolves_its_packaged_install_root() {
        let install = TempDir::new().unwrap();
        let executable = install.path().join("bin/clonk-app");
        fs::create_dir_all(executable.parent().unwrap()).unwrap();
        fs::write(&executable, b"runtime").unwrap();

        assert_eq!(
            install_root_from_executable_shape(&executable).as_deref(),
            Some(install.path())
        );
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

    /// macOS Gatekeeper runs a quarantined bundle from a read-only
    /// `AppTranslocation` mount. C++ recovers the original path through
    /// Security.framework and chdirs four parents up from the executable
    /// (MacAppTranslocation.cpp:27-63; C4WinMain.cpp:233-238).
    #[cfg(target_os = "macos")]
    #[test]
    fn macos_translocated_bundle_uses_original_root_and_cwd() {
        use std::path::Path;

        // `dirname` four times over `X.app/Contents/MacOS/exe` is the directory
        // that holds the bundle.
        assert_eq!(
            macos_bundle_working_directory(Path::new(
                "/Applications/Clonk.app/Contents/MacOS/clonk"
            )),
            Some(PathBuf::from("/Applications"))
        );
        assert_eq!(
            macos_bundle_working_directory(Path::new("/a/b/c/Clonk.app/Contents/MacOS/clonk")),
            Some(PathBuf::from("/a/b/c"))
        );
        // A plain binary has too few components, which C++'s repeated dirname
        // would bottom out on; report nothing rather than chdir to `/`.
        assert_eq!(
            macos_bundle_working_directory(Path::new("/usr/bin/clonk")),
            None
        );
        assert_eq!(macos_bundle_working_directory(Path::new("clonk")), None);

        // A bundle that is *not* translocated keeps its own path: the probe
        // returns None and the caller carries on unchanged. This is the branch
        // every ordinary launch takes, and it must never fabricate a path.
        let bundle = TempDir::new().expect("bundle root");
        let executable = bundle.path().join("Clonk.app/Contents/MacOS/clonk");
        fs::create_dir_all(executable.parent().expect("MacOS dir")).expect("bundle layout");
        fs::write(&executable, b"#!/bin/sh\n").expect("bundle executable");
        assert_eq!(non_translocated_executable(&executable), executable);
        // A path that does not exist at all is still returned verbatim.
        let missing = bundle.path().join("Gone.app/Contents/MacOS/clonk");
        assert_eq!(non_translocated_executable(&missing), missing);

        // Install discovery reads the bundle's Resources through whichever
        // executable path survived that recovery.
        let resources = bundle.path().join("Clonk.app/Contents/Resources");
        fs::create_dir_all(resources.join("planet/System.c4g")).expect("bundle resources");
        assert_eq!(
            install_root_from_executable_shape(&non_translocated_executable(&executable)),
            Some(resources)
        );
    }
}
