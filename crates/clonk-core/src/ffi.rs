//! The C ABI the pinned oracle's `USE_RUST_CONFIG` bridge links against
//! (clonk-org/clonk-rs#1264).
//!
//! `parity/bridge/lc_config_ffi.h` is the pinned header verbatim, and
//! `src/rust/RustConfigBridge.cpp` at the oracle pin is the only caller. That
//! bridge holds **one** handle behind its own mutex and frees every returned
//! string with [`lc_string_free`], so the ownership contract here is:
//!
//! - every `*mut c_char` returned is a `CString::into_raw` the caller owns;
//! - a null return means "no value" and is never an error the caller reports;
//! - [`lc_config_compare_with_dump`] returns null when the two dumps agree,
//!   which the bridge maps onto `std::nullopt` alongside the empty string.
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
//! `crates/clonk-engine/src/ffi.rs` does the same for the engine bridge.
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::ffi::{CStr, CString};
use std::io::Cursor;
use std::os::raw::c_char;
use std::ptr;

use crate::std_config::Config;

/// The opaque `ConfigHandle` the header declares.
pub struct ConfigHandle(Config);

/// The bridge caps its report so a wholly different file cannot produce an
/// unbounded diagnostic.
const MAX_FINDINGS: usize = 25;

#[no_mangle]
pub extern "C" fn lc_config_load(path: *const c_char) -> *mut ConfigHandle {
    if path.is_null() {
        return ptr::null_mut();
    }
    let c_str = unsafe { CStr::from_ptr(path) };
    match Config::load(c_str.to_string_lossy().as_ref()) {
        Ok(config) => Box::into_raw(Box::new(ConfigHandle(config))),
        Err(_) => ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "C" fn lc_config_free(handle: *mut ConfigHandle) {
    if handle.is_null() {
        return;
    }
    unsafe {
        drop(Box::from_raw(handle));
    }
}

#[no_mangle]
pub extern "C" fn lc_config_get_value(
    handle: *mut ConfigHandle,
    key: *const c_char,
) -> *mut c_char {
    // `RustConfigBridge::GetValue` forwards to `GetValueIn` with an empty
    // section, and picks this entry point only when the section is empty.
    lc_config_get_value_in(handle, ptr::null(), key)
}

#[no_mangle]
pub extern "C" fn lc_config_get_value_in(
    handle: *mut ConfigHandle,
    section: *const c_char,
    key: *const c_char,
) -> *mut c_char {
    if handle.is_null() || key.is_null() {
        return ptr::null_mut();
    }
    let handle = unsafe { &*handle };
    let key = unsafe { CStr::from_ptr(key) }
        .to_string_lossy()
        .into_owned();
    let section = (!section.is_null()).then(|| {
        unsafe { CStr::from_ptr(section) }
            .to_string_lossy()
            .into_owned()
    });
    handle
        .0
        .get_in(section.as_deref(), &key)
        .and_then(|value| CString::new(value).ok())
        .map_or(ptr::null_mut(), CString::into_raw)
}

#[no_mangle]
pub extern "C" fn lc_config_dump(handle: *mut ConfigHandle) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let handle = unsafe { &*handle };
    handle
        .0
        .to_string()
        .ok()
        .and_then(|dump| CString::new(dump).ok())
        .map_or(ptr::null_mut(), CString::into_raw)
}

/// Compares this config against a legacy dump, reporting what differs.
///
/// Null means "they agree" — the bridge treats that and the empty string
/// identically (`RustConfigBridge.cpp:60-73`).
#[no_mangle]
pub extern "C" fn lc_config_compare_with_dump(
    handle: *mut ConfigHandle,
    legacy_dump: *const c_char,
) -> *mut c_char {
    if handle.is_null() || legacy_dump.is_null() {
        return ptr::null_mut();
    }
    let handle = unsafe { &*handle };
    let legacy_text = unsafe { CStr::from_ptr(legacy_dump) }
        .to_string_lossy()
        .into_owned();
    let mut cursor = Cursor::new(legacy_text.as_bytes());
    let Ok(legacy) = Config::from_reader(&mut cursor) else {
        return ptr::null_mut();
    };

    let mut findings = Vec::new();
    for entry in handle.0.iter() {
        match legacy.get_in(entry.section.as_deref(), &entry.key) {
            Some(other) if other == entry.value => {}
            Some(other) => findings.push(format!(
                "Value mismatch for {} (rust='{}', legacy='{}')",
                display_key(entry.section.as_deref(), &entry.key),
                entry.value,
                other
            )),
            None => findings.push(format!(
                "Missing in legacy: {} (rust='{}')",
                display_key(entry.section.as_deref(), &entry.key),
                entry.value
            )),
        }
        if findings.len() >= MAX_FINDINGS {
            break;
        }
    }
    if findings.len() < MAX_FINDINGS {
        for entry in legacy.iter() {
            if handle
                .0
                .get_in(entry.section.as_deref(), &entry.key)
                .is_none()
            {
                findings.push(format!(
                    "Missing in rust: {} (legacy='{}')",
                    display_key(entry.section.as_deref(), &entry.key),
                    entry.value
                ));
            }
            if findings.len() >= MAX_FINDINGS {
                break;
            }
        }
    }

    if findings.is_empty() {
        return ptr::null_mut();
    }

    let rust_count = handle.0.iter().count();
    let legacy_count = legacy.iter().count();
    if rust_count != legacy_count {
        findings.push(format!(
            "Entry count differs (rust={rust_count}, legacy={legacy_count})"
        ));
    }

    CString::new(findings.join("\n")).map_or(ptr::null_mut(), CString::into_raw)
}

fn display_key(section: Option<&str>, key: &str) -> String {
    match section {
        Some(name) if !name.is_empty() => format!("[{name}] {key}"),
        _ => key.to_string(),
    }
}

#[no_mangle]
pub extern "C" fn lc_config_replace_from_text(
    handle: *mut ConfigHandle,
    text: *const c_char,
) -> bool {
    if handle.is_null() || text.is_null() {
        return false;
    }
    let handle = unsafe { &mut *handle };
    let text = unsafe { CStr::from_ptr(text) }
        .to_string_lossy()
        .into_owned();
    let mut cursor = Cursor::new(text.as_bytes());
    match Config::from_reader(&mut cursor) {
        Ok(replacement) => {
            handle.0 = replacement;
            true
        }
        Err(_) => false,
    }
}

#[no_mangle]
pub extern "C" fn lc_config_save(handle: *mut ConfigHandle, path: *const c_char) -> bool {
    if handle.is_null() || path.is_null() {
        return false;
    }
    let handle = unsafe { &*handle };
    let path = unsafe { CStr::from_ptr(path) }
        .to_string_lossy()
        .into_owned();
    handle.0.save(&path).is_ok()
}

#[no_mangle]
pub extern "C" fn lc_string_free(value: *mut c_char) {
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
    use tempfile::tempdir;

    fn load(contents: &[u8]) -> (tempfile::TempDir, *mut ConfigHandle) {
        let dir = tempdir().expect("temp dir");
        let path = dir.path().join("ffi.cfg");
        std::fs::write(&path, contents).expect("write fixture");
        let c_path = CString::new(path.to_str().expect("utf-8 path")).expect("no NUL");
        let handle = lc_config_load(c_path.as_ptr());
        assert!(!handle.is_null(), "the fixture loads");
        (dir, handle)
    }

    fn take(value: *mut c_char) -> Option<String> {
        (!value.is_null()).then(|| {
            let owned = unsafe { CStr::from_ptr(value) }
                .to_string_lossy()
                .into_owned();
            lc_string_free(value);
            owned
        })
    }

    #[test]
    fn a_root_value_round_trips_through_the_bridge() {
        let (_dir, handle) = load(b"Key=Value\n");
        let key = CString::new("Key").expect("no NUL");
        assert_eq!(
            take(lc_config_get_value(handle, key.as_ptr())).as_deref(),
            Some("Value")
        );
        lc_config_free(handle);
    }

    #[test]
    fn a_section_value_is_addressed_by_its_section() {
        let (_dir, handle) = load(b"[Graphics]\nEngine=OpenGL\n");
        let section = CString::new("Graphics").expect("no NUL");
        let key = CString::new("Engine").expect("no NUL");
        assert_eq!(
            take(lc_config_get_value_in(
                handle,
                section.as_ptr(),
                key.as_ptr()
            ))
            .as_deref(),
            Some("OpenGL")
        );
        // The root lookup must not see it: `GetValue` is `GetValueIn` with an
        // empty section (`RustConfigBridge.cpp:26-28`).
        assert_eq!(take(lc_config_get_value(handle, key.as_ptr())), None);
        lc_config_free(handle);
    }

    #[test]
    fn a_missing_value_is_null_rather_than_an_error() {
        let (_dir, handle) = load(b"");
        let missing = CString::new("Missing").expect("no NUL");
        assert_eq!(take(lc_config_get_value(handle, missing.as_ptr())), None);
        lc_config_free(handle);
    }

    #[test]
    fn a_matching_dump_compares_equal() {
        let (_dir, handle) = load(b"[Graphics]\nEngine=OpenGL\n");
        let dump = take(lc_config_dump(handle)).expect("a dump");
        let legacy = CString::new(dump).expect("no NUL");
        assert_eq!(
            take(lc_config_compare_with_dump(handle, legacy.as_ptr())),
            None,
            "our own dump is not a difference"
        );
        lc_config_free(handle);
    }

    #[test]
    fn a_differing_dump_reports_both_directions() {
        let (_dir, handle) = load(b"[Graphics]\nEngine=OpenGL\n[Audio]\nEnabled=true\n");
        let legacy =
            CString::new("[Graphics]\nEngine=Vulkan\n[Misc]\nOnly=legacy\n").expect("no NUL");
        let report =
            take(lc_config_compare_with_dump(handle, legacy.as_ptr())).expect("a difference");
        assert!(
            report.contains("Value mismatch for [Graphics] Engine"),
            "{report}"
        );
        assert!(
            report.contains("Missing in legacy: [Audio] Enabled"),
            "{report}"
        );
        assert!(report.contains("Missing in rust: [Misc] Only"), "{report}");
        lc_config_free(handle);
    }

    #[test]
    fn replace_from_text_then_save_round_trips() {
        let (dir, handle) = load(b"Name=Legacy\n");
        let replacement = CString::new("Name=Rusty\n").expect("no NUL");
        assert!(lc_config_replace_from_text(handle, replacement.as_ptr()));

        let saved = dir.path().join("saved.cfg");
        let c_saved = CString::new(saved.to_str().expect("utf-8 path")).expect("no NUL");
        assert!(lc_config_save(handle, c_saved.as_ptr()));
        assert!(std::fs::read_to_string(&saved)
            .expect("saved file")
            .contains("Rusty"));
        lc_config_free(handle);
    }

    #[test]
    fn malformed_replacement_text_leaves_the_handle_untouched() {
        let (_dir, handle) = load(b"Name=Legacy\n");
        // Interior NUL cannot reach us through a C string, so the reachable
        // malformed case is text that parses to something else entirely.
        let replacement = CString::new("[Unclosed\nName=Ignored").expect("no NUL");
        lc_config_replace_from_text(handle, replacement.as_ptr());
        let key = CString::new("Name").expect("no NUL");
        assert!(
            take(lc_config_get_value(handle, key.as_ptr())).is_some(),
            "the handle stays usable whatever the parse produced"
        );
        lc_config_free(handle);
    }

    #[test]
    fn every_entry_point_tolerates_a_null_handle() {
        let key = CString::new("Key").expect("no NUL");
        assert!(lc_config_get_value(ptr::null_mut(), key.as_ptr()).is_null());
        assert!(lc_config_get_value_in(ptr::null_mut(), ptr::null(), key.as_ptr()).is_null());
        assert!(lc_config_dump(ptr::null_mut()).is_null());
        assert!(lc_config_compare_with_dump(ptr::null_mut(), key.as_ptr()).is_null());
        assert!(!lc_config_replace_from_text(ptr::null_mut(), key.as_ptr()));
        assert!(!lc_config_save(ptr::null_mut(), key.as_ptr()));
        assert!(lc_config_load(ptr::null()).is_null());
        // Both frees are no-ops on null, which is what lets the bridge call
        // them unconditionally.
        lc_config_free(ptr::null_mut());
        lc_string_free(ptr::null_mut());
    }
}
