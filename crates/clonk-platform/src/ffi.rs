//! The C ABI the pinned oracle's `USE_RUST_PLATFORM_PATHS` bridge links
//! against (clonk-org/clonk-rs#1267).
//!
//! `parity/bridge/lc_platform_ffi.h` is the pinned header verbatim. Every
//! getter is a fresh `AppPaths::discover()` — the bridge holds no handle, so
//! there is nothing to invalidate and nothing to free but the strings, which
//! [`lc_platform_string_free`] owns. A discovery failure is a null return, not
//! an error the caller reports.
//!
//! Off by default behind the `ffi` feature: this is a differential-testing
//! surface, not part of the shipped library.
//!
//! Every entry point is an `extern "C"` boundary, so pointer validity,
//! lifetime and aliasing are the C++ caller's contract — which is what
//! `clippy::not_unsafe_ptr_arg_deref` is asking about. Marking these `unsafe
//! fn` would change neither the ABI nor the symbols, but it would diverge from
//! the pinned reference for no safety gained on the only caller there is, so
//! the lint is silenced module-wide and the contract is documented above.
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::ffi::CString;
use std::os::raw::c_char;
use std::path::Path;
use std::ptr;

use crate::paths::AppPaths;

fn owned_c_path(path: &Path) -> *mut c_char {
    CString::new(path.as_os_str().to_string_lossy().into_owned())
        .map_or(ptr::null_mut(), CString::into_raw)
}

/// Rediscovers the roots and hands back one of them.
///
/// Discovery per call rather than a cached handle is the pinned shape: the
/// bridge exposes no lifetime, so a caller can never hold a stale root across
/// a config change.
fn discovered(select: impl Fn(&AppPaths) -> &Path) -> *mut c_char {
    AppPaths::discover()
        .ok()
        .map_or(ptr::null_mut(), |paths| owned_c_path(select(&paths)))
}

#[no_mangle]
pub extern "C" fn lc_platform_install_root() -> *mut c_char {
    discovered(AppPaths::install_root)
}

#[no_mangle]
pub extern "C" fn lc_platform_planet_dir() -> *mut c_char {
    discovered(AppPaths::planet_dir)
}

#[no_mangle]
pub extern "C" fn lc_platform_system_group_path() -> *mut c_char {
    discovered(AppPaths::system_group_path)
}

#[no_mangle]
pub extern "C" fn lc_platform_user_data_dir() -> *mut c_char {
    discovered(AppPaths::user_data_dir)
}

#[no_mangle]
pub extern "C" fn lc_platform_cache_dir() -> *mut c_char {
    discovered(AppPaths::cache_dir)
}

#[no_mangle]
pub extern "C" fn lc_platform_logs_dir() -> *mut c_char {
    discovered(AppPaths::logs_dir)
}

#[no_mangle]
pub extern "C" fn lc_platform_temp_dir() -> *mut c_char {
    discovered(AppPaths::temp_dir)
}

/// `config_dir` returns an owned `PathBuf` rather than a borrow, so it cannot
/// go through [`discovered`].
#[no_mangle]
pub extern "C" fn lc_platform_config_dir() -> *mut c_char {
    AppPaths::discover()
        .ok()
        .map_or(ptr::null_mut(), |paths| owned_c_path(&paths.config_dir()))
}

#[no_mangle]
pub extern "C" fn lc_platform_ensure_user_dirs() -> bool {
    AppPaths::discover().is_ok_and(|paths| paths.ensure_user_dirs().is_ok())
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
    use crate::paths::tests::EnvGuard;
    use std::ffi::CStr;
    use tempfile::TempDir;

    fn take(value: *mut c_char) -> Option<String> {
        (!value.is_null()).then(|| {
            let owned = unsafe { CStr::from_ptr(value) }
                .to_string_lossy()
                .into_owned();
            lc_platform_string_free(value);
            owned
        })
    }

    /// A discoverable install root, which needs its `planet/System.c4g`.
    fn staged_install() -> TempDir {
        let install = TempDir::new().expect("install dir");
        let planet = install.path().join("planet");
        std::fs::create_dir_all(&planet).expect("planet dir");
        std::fs::write(planet.join("System.c4g"), b"stub").expect("system group");
        install
    }

    /// The bridge is a view onto the same roots the process uses, not a second
    /// opinion: every getter must agree with `AppPaths` for one environment.
    ///
    /// `EnvGuard` holds the crate-wide env lock, so this cannot race the
    /// `paths` tests that move `LC_INSTALL_ROOT` out from under it — without
    /// that lock this passes alone and fails in the full crate run.
    #[test]
    fn every_getter_reports_the_roots_the_environment_selects() {
        let install = staged_install();
        let user = TempDir::new().expect("user dir");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install.path())),
            ("LC_USER_DATA_DIR", Some(user.path())),
        ]);

        let paths = AppPaths::discover().expect("the staged roots discover");
        let expect = |actual: Option<String>, expected: &Path, what: &str| {
            assert_eq!(
                actual.as_deref(),
                Some(expected.as_os_str().to_string_lossy().as_ref()),
                "{what}"
            );
        };
        expect(
            take(lc_platform_install_root()),
            paths.install_root(),
            "install root",
        );
        expect(
            take(lc_platform_planet_dir()),
            paths.planet_dir(),
            "planet dir",
        );
        expect(
            take(lc_platform_system_group_path()),
            paths.system_group_path(),
            "system group",
        );
        expect(
            take(lc_platform_user_data_dir()),
            paths.user_data_dir(),
            "user data",
        );
        expect(take(lc_platform_cache_dir()), paths.cache_dir(), "cache");
        expect(take(lc_platform_logs_dir()), paths.logs_dir(), "logs");
        expect(take(lc_platform_temp_dir()), paths.temp_dir(), "temp");
        expect(
            take(lc_platform_config_dir()),
            &paths.config_dir(),
            "config dir",
        );
    }

    /// The pinned bridge's own coverage: the one call with a side effect.
    #[test]
    fn ensure_user_dirs_creates_the_expected_structure() {
        let install = staged_install();
        let user = TempDir::new().expect("user dir");
        let user_path = user.path().to_path_buf();
        // Remove it so the call has to create the tree rather than find it.
        std::fs::remove_dir_all(&user_path).expect("clear user dir");

        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install.path())),
            ("LC_USER_DATA_DIR", Some(user_path.as_path())),
        ]);

        assert!(lc_platform_ensure_user_dirs());
        assert!(user_path.join("Config").exists(), "Config");
        assert!(user_path.join("Cache").exists(), "Cache");
        assert!(user_path.join("Logs").exists(), "Logs");
    }

    #[test]
    fn a_freed_string_is_a_no_op_on_null() {
        // The bridge frees unconditionally after a failed getter, so this has
        // to be safe rather than merely unused.
        lc_platform_string_free(ptr::null_mut());
    }
}
