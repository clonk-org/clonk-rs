use crate::paths::AppPaths;
use std::ffi::CString;
use std::os::raw::c_char;
use std::ptr;

fn path_to_c_string(path: &std::path::Path) -> Option<*mut c_char> {
    let owned = path.as_os_str().to_string_lossy().into_owned();
    CString::new(owned).ok().map(CString::into_raw)
}

fn discover_and_convert<F>(mapper: F) -> *mut c_char
where
    F: Fn(&AppPaths) -> &std::path::Path,
{
    match AppPaths::discover()
        .ok()
        .and_then(|paths| path_to_c_string(mapper(&paths)))
    {
        Some(ptr) => ptr,
        None => ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "C" fn lc_platform_install_root() -> *mut c_char {
    discover_and_convert(|paths| paths.install_root())
}

#[no_mangle]
pub extern "C" fn lc_platform_planet_dir() -> *mut c_char {
    discover_and_convert(|paths| paths.planet_dir())
}

#[no_mangle]
pub extern "C" fn lc_platform_system_group_path() -> *mut c_char {
    discover_and_convert(|paths| paths.system_group_path())
}

#[no_mangle]
pub extern "C" fn lc_platform_user_data_dir() -> *mut c_char {
    discover_and_convert(|paths| paths.user_data_dir())
}

#[no_mangle]
pub extern "C" fn lc_platform_cache_dir() -> *mut c_char {
    discover_and_convert(|paths| paths.cache_dir())
}

#[no_mangle]
pub extern "C" fn lc_platform_logs_dir() -> *mut c_char {
    discover_and_convert(|paths| paths.logs_dir())
}

#[no_mangle]
pub extern "C" fn lc_platform_temp_dir() -> *mut c_char {
    discover_and_convert(|paths| paths.temp_dir())
}

#[no_mangle]
pub extern "C" fn lc_platform_config_dir() -> *mut c_char {
    match AppPaths::discover()
        .ok()
        .and_then(|paths| path_to_c_string(&paths.config_dir()))
    {
        Some(ptr) => ptr,
        None => ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "C" fn lc_platform_ensure_user_dirs() -> bool {
    match AppPaths::discover() {
        Ok(paths) => paths.ensure_user_dirs().is_ok(),
        Err(_) => false,
    }
}

#[no_mangle]
pub extern "C" fn lc_platform_string_free(value: *mut c_char) {
    if value.is_null() {
        return;
    }
    unsafe {
        drop(CString::from_raw(value));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;
    use std::path::Path;
    use std::sync::{Mutex, MutexGuard, OnceLock};
    use tempfile::TempDir;

    #[test]
    fn install_root_roundtrip() {
        let ptr = lc_platform_install_root();
        assert!(!ptr.is_null());
        let value = unsafe { CStr::from_ptr(ptr) }.to_string_lossy();
        assert!(!value.is_empty());
        lc_platform_string_free(ptr);
    }

    #[test]
    fn planet_dir_roundtrip() {
        let ptr = lc_platform_planet_dir();
        assert!(!ptr.is_null());
        let value = unsafe { CStr::from_ptr(ptr) }.to_string_lossy();
        assert!(value.ends_with("planet") || value.ends_with("planet/"));
        lc_platform_string_free(ptr);
    }

    struct EnvGuard {
        _lock: MutexGuard<'static, ()>,
        saved: Vec<(String, Option<std::ffi::OsString>)>,
    }

    impl EnvGuard {
        fn set(vars: &[(&str, Option<&Path>)]) -> Self {
            let lock = env_lock().lock().unwrap();
            let mut saved = Vec::with_capacity(vars.len());
            for (key, value) in vars {
                let original = std::env::var_os(key);
                saved.push((key.to_string(), original));
                match value {
                    Some(path) => std::env::set_var(key, path.as_os_str()),
                    None => std::env::remove_var(key),
                }
            }
            Self { _lock: lock, saved }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (key, value) in self.saved.drain(..) {
                match value {
                    Some(val) => std::env::set_var(&key, val),
                    None => std::env::remove_var(&key),
                }
            }
        }
    }

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn ensure_user_dirs_creates_expected_structure() {
        let install_dir = TempDir::new().unwrap();
        let planet_dir = install_dir.path().join("planet");
        std::fs::create_dir_all(&planet_dir).unwrap();
        std::fs::write(planet_dir.join("System.c4g"), b"stub").unwrap();

        let user_dir = TempDir::new().unwrap();
        std::fs::remove_dir_all(user_dir.path()).unwrap();

        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.path())),
        ]);

        assert!(lc_platform_ensure_user_dirs());

        assert!(user_dir.path().join("Config").exists());
        assert!(user_dir.path().join("Cache").exists());
        assert!(user_dir.path().join("Logs").exists());
        assert!(user_dir.path().exists());
    }
}
