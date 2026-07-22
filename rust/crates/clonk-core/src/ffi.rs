use crate::std_config::Config;
use std::ffi::{CStr, CString};
use std::io::Cursor;
use std::os::raw::c_char;
use std::ptr;

pub struct ConfigHandle(Config);

#[no_mangle]
pub extern "C" fn lc_config_load(path: *const c_char) -> *mut ConfigHandle {
    if path.is_null() {
        return ptr::null_mut();
    }
    let c_str = unsafe { CStr::from_ptr(path) };
    match Config::load(c_str.to_string_lossy().as_ref()) {
        Ok(cfg) => Box::into_raw(Box::new(ConfigHandle(cfg))),
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
    let cfg = unsafe { &mut *handle };
    let key_str = unsafe { CStr::from_ptr(key) };
    let section_string = if section.is_null() {
        None
    } else {
        Some(
            unsafe { CStr::from_ptr(section) }
                .to_string_lossy()
                .to_string(),
        )
    };
    match cfg.0.get_in(
        section_string.as_deref(),
        key_str.to_string_lossy().as_ref(),
    ) {
        Some(value) => match CString::new(value) {
            Ok(c_string) => c_string.into_raw(),
            Err(_) => ptr::null_mut(),
        },
        None => ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "C" fn lc_config_dump(handle: *mut ConfigHandle) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let cfg = unsafe { &mut *handle };
    match cfg.0.to_string() {
        Ok(dump) => match CString::new(dump) {
            Ok(c_string) => c_string.into_raw(),
            Err(_) => ptr::null_mut(),
        },
        Err(_) => ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "C" fn lc_config_compare_with_dump(
    handle: *mut ConfigHandle,
    legacy_dump: *const c_char,
) -> *mut c_char {
    if handle.is_null() || legacy_dump.is_null() {
        return ptr::null_mut();
    }

    let cfg = unsafe { &mut *handle };
    let legacy_str = unsafe { CStr::from_ptr(legacy_dump) }
        .to_string_lossy()
        .into_owned();
    let mut cursor = Cursor::new(legacy_str.as_bytes());
    let legacy_config = match Config::from_reader(&mut cursor) {
        Ok(config) => config,
        Err(_) => return ptr::null_mut(),
    };

    const MAX_FINDINGS: usize = 25;
    let mut findings = Vec::new();

    let legacy_entries = legacy_config.entry_map();
    for ((section, key), entry) in cfg.0.entry_map().iter() {
        match legacy_entries.get(&(section.clone(), key.clone())) {
            Some(other) => {
                if entry.value != other.value {
                    findings.push(format!(
                        "Value mismatch for {} (rust='{}', legacy='{}')",
                        display_key(section.as_ref(), key),
                        entry.value,
                        other.value
                    ));
                }
            }
            None => findings.push(format!(
                "Missing in legacy: {} (rust='{}')",
                display_key(section.as_ref(), key),
                entry.value
            )),
        }
        if findings.len() >= MAX_FINDINGS {
            break;
        }
    }

    if findings.len() < MAX_FINDINGS {
        for ((section, key), entry) in legacy_entries.iter() {
            if cfg
                .0
                .entry_map()
                .get(&(section.clone(), key.clone()))
                .is_none()
            {
                findings.push(format!(
                    "Missing in rust: {} (legacy='{}')",
                    display_key(section.as_ref(), key),
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

    if cfg.0.entry_map().len() != legacy_entries.len() {
        findings.push(format!(
            "Entry count differs (rust={}, legacy={})",
            cfg.0.entry_map().len(),
            legacy_entries.len()
        ));
    }

    if let Ok(summary) = CString::new(findings.join("\n")) {
        summary.into_raw()
    } else {
        ptr::null_mut()
    }
}

fn display_key(section: Option<&String>, key: &str) -> String {
    match section {
        Some(name) if !name.is_empty() => format!("[{}] {}", name, key),
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
    let cfg = unsafe { &mut *handle };
    let text_str = unsafe { CStr::from_ptr(text) }
        .to_string_lossy()
        .into_owned();
    let mut cursor = Cursor::new(text_str.as_bytes());
    match Config::from_reader(&mut cursor) {
        Ok(new_cfg) => {
            cfg.0 = new_cfg;
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
    let cfg = unsafe { &mut *handle };
    let path_str = unsafe { CStr::from_ptr(path) }
        .to_string_lossy()
        .into_owned();
    cfg.0.save(&path_str).is_ok()
}

#[no_mangle]
pub extern "C" fn lc_string_free(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    unsafe {
        drop(CString::from_raw(s));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn ffi_load_and_get() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("ffi.cfg");
        std::fs::write(&path, b"Key=Value\n").unwrap();
        let c_path = CString::new(path.to_str().unwrap()).unwrap();
        let handle = lc_config_load(c_path.as_ptr());
        assert!(!handle.is_null());
        let key = CString::new("Key").unwrap();
        let value_ptr = lc_config_get_value(handle, key.as_ptr());
        assert!(!value_ptr.is_null());
        let value = unsafe { CStr::from_ptr(value_ptr) }
            .to_str()
            .unwrap()
            .to_string();
        assert_eq!(value, "Value");
        lc_string_free(value_ptr);
        lc_config_free(handle);
    }

    #[test]
    fn ffi_handles_missing_values() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("ffi_missing.cfg");
        std::fs::write(&path, b"").unwrap();
        let c_path = CString::new(path.to_str().unwrap()).unwrap();
        let handle = lc_config_load(c_path.as_ptr());
        assert!(!handle.is_null());
        let missing = CString::new("Missing").unwrap();
        let value_ptr = lc_config_get_value(handle, missing.as_ptr());
        assert!(value_ptr.is_null());
        lc_config_free(handle);
    }

    #[test]
    fn ffi_get_with_section() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("ffi_section.cfg");
        std::fs::write(&path, b"[Graphics]\nEngine=OpenGL\n").unwrap();
        let c_path = CString::new(path.to_str().unwrap()).unwrap();
        let handle = lc_config_load(c_path.as_ptr());
        assert!(!handle.is_null());
        let section = CString::new("Graphics").unwrap();
        let key = CString::new("Engine").unwrap();
        let value_ptr = lc_config_get_value_in(handle, section.as_ptr(), key.as_ptr());
        assert!(!value_ptr.is_null());
        let value = unsafe { CStr::from_ptr(value_ptr) }.to_str().unwrap();
        assert_eq!(value, "OpenGL");
        lc_string_free(value_ptr);
        lc_config_free(handle);
    }

    #[test]
    fn ffi_compare_with_dump_reports_difference() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("ffi_compare.cfg");
        std::fs::write(&path, b"[Graphics]\nEngine=OpenGL\n[Audio]\nEnabled=true\n").unwrap();
        let c_path = CString::new(path.to_str().unwrap()).unwrap();
        let handle = lc_config_load(c_path.as_ptr());
        assert!(!handle.is_null());

        let legacy_dump = "[Graphics]\nEngine=Vulkan\n";
        let dump_c = CString::new(legacy_dump).unwrap();
        let diff_ptr = lc_config_compare_with_dump(handle, dump_c.as_ptr());
        assert!(!diff_ptr.is_null());
        let diff = unsafe { CStr::from_ptr(diff_ptr) }.to_str().unwrap();
        assert!(diff.contains("Value mismatch"));
        lc_string_free(diff_ptr as *mut c_char);
        lc_config_free(handle);
    }

    #[test]
    fn ffi_replace_and_save_roundtrip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("replace.cfg");
        std::fs::write(&path, b"Name=Legacy\n").unwrap();
        let c_path = CString::new(path.to_str().unwrap()).unwrap();
        let handle = lc_config_load(c_path.as_ptr());
        assert!(!handle.is_null());

        let new_text = CString::new("Name=Rusty\n").unwrap();
        assert!(lc_config_replace_from_text(handle, new_text.as_ptr()));

        let save_path = dir.path().join("saved.cfg");
        let save_c = CString::new(save_path.to_str().unwrap()).unwrap();
        assert!(lc_config_save(handle, save_c.as_ptr()));
        let saved = std::fs::read_to_string(&save_path).unwrap();
        assert!(saved.contains("Rusty"));

        lc_config_free(handle);
    }
}
