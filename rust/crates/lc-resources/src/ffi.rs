use crate::group::Group;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_uchar};
use std::ptr;

pub struct GroupHandle(Group);

#[repr(C)]
pub struct LcGroupEntry {
    pub path: *mut c_char,
    pub is_directory: bool,
    pub size: u64,
}

#[no_mangle]
pub extern "C" fn lc_group_open(path: *const c_char) -> *mut GroupHandle {
    if path.is_null() {
        return ptr::null_mut();
    }
    let c_path = unsafe { CStr::from_ptr(path) };
    match Group::open(c_path.to_string_lossy().as_ref()) {
        Ok(group) => Box::into_raw(Box::new(GroupHandle(group))),
        Err(_) => ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "C" fn lc_group_free(handle: *mut GroupHandle) {
    if handle.is_null() {
        return;
    }
    unsafe {
        drop(Box::from_raw(handle));
    }
}

#[no_mangle]
pub extern "C" fn lc_group_entries(
    handle: *mut GroupHandle,
    out_len: *mut usize,
) -> *mut LcGroupEntry {
    if handle.is_null() || out_len.is_null() {
        return ptr::null_mut();
    }

    let group = unsafe { &mut *handle };
    let entries = match group.0.entries() {
        Ok(entries) => entries,
        Err(_) => {
            unsafe {
                *out_len = 0;
            }
            return ptr::null_mut();
        }
    };

    let mut ffi_entries = Vec::with_capacity(entries.len());
    for entry in entries {
        let path_string = entry.relative_path.to_string_lossy().into_owned();
        let c_path = match CString::new(path_string) {
            Ok(cstr) => cstr.into_raw(),
            Err(_) => {
                unsafe {
                    *out_len = 0;
                }
                lc_group_entries_free_from_vec(&mut ffi_entries);
                return ptr::null_mut();
            }
        };
        ffi_entries.push(LcGroupEntry {
            path: c_path,
            is_directory: entry.is_directory,
            size: entry.size,
        });
    }

    let len = ffi_entries.len();
    let mut boxed = ffi_entries.into_boxed_slice();
    let ptr = boxed.as_mut_ptr();
    unsafe {
        *out_len = len;
    }
    std::mem::forget(boxed);
    ptr
}

fn lc_group_entries_free_from_vec(entries: &mut Vec<LcGroupEntry>) {
    for entry in entries.drain(..) {
        unsafe {
            if !entry.path.is_null() {
                drop(CString::from_raw(entry.path));
            }
        }
    }
}

#[no_mangle]
pub extern "C" fn lc_group_entries_free(entries: *mut LcGroupEntry, len: usize) {
    if entries.is_null() {
        return;
    }

    let slice_ptr = ptr::slice_from_raw_parts_mut(entries, len);
    unsafe {
        let slice = &mut *slice_ptr;
        for entry in slice.iter_mut() {
            if !entry.path.is_null() {
                drop(CString::from_raw(entry.path));
                entry.path = ptr::null_mut();
            }
        }
        drop(Box::from_raw(slice_ptr));
    }
}

#[no_mangle]
pub extern "C" fn lc_group_read_file(
    handle: *mut GroupHandle,
    path: *const c_char,
    out_len: *mut usize,
) -> *mut c_uchar {
    if handle.is_null() || path.is_null() || out_len.is_null() {
        return ptr::null_mut();
    }

    let group = unsafe { &mut *handle };
    let relative = unsafe { CStr::from_ptr(path) };
    match group.0.read_file(relative.to_string_lossy().as_ref()) {
        Ok(data) => {
            let mut boxed = data.into_boxed_slice();
            let len = boxed.len();
            let ptr = boxed.as_mut_ptr();
            unsafe {
                *out_len = len;
            }
            std::mem::forget(boxed);
            ptr
        }
        Err(_) => {
            unsafe {
                *out_len = 0;
            }
            ptr::null_mut()
        }
    }
}

#[no_mangle]
pub extern "C" fn lc_group_buffer_free(buffer: *mut c_uchar, len: usize) {
    if buffer.is_null() {
        return;
    }
    unsafe {
        drop(Box::from_raw(ptr::slice_from_raw_parts_mut(buffer, len)));
    }
}

#[no_mangle]
pub extern "C" fn lc_group_exists(handle: *mut GroupHandle, path: *const c_char) -> bool {
    if handle.is_null() || path.is_null() {
        return false;
    }
    let group = unsafe { &mut *handle };
    let relative = unsafe { CStr::from_ptr(path) };
    group
        .0
        .exists(std::path::Path::new(relative.to_string_lossy().as_ref()))
}

#[no_mangle]
pub extern "C" fn lc_group_maker(handle: *mut GroupHandle) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let group = unsafe { &mut *handle };
    match group.0.maker() {
        Some(maker) => match CString::new(maker) {
            Ok(cstring) => cstring.into_raw(),
            Err(_) => ptr::null_mut(),
        },
        None => ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "C" fn lc_group_root(handle: *mut GroupHandle) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let group = unsafe { &mut *handle };
    match CString::new(group.0.root().to_string_lossy().into_owned()) {
        Ok(cstring) => cstring.into_raw(),
        Err(_) => ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "C" fn lc_group_string_free(value: *mut c_char) {
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
    use std::ffi::{CStr, CString};
    use std::slice;
    use tempfile::tempdir;

    #[test]
    fn ffi_open_directory_and_list_entries() {
        let dir = tempdir().unwrap();
        let subdir = dir.path().join("sub");
        std::fs::create_dir(&subdir).unwrap();
        let file_path = subdir.join("file.txt");
        std::fs::write(&file_path, b"hello").unwrap();
        let root_file = dir.path().join("root.txt");
        std::fs::write(&root_file, b"world").unwrap();

        let c_path = CString::new(dir.path().to_string_lossy().into_owned()).unwrap();
        let handle = lc_group_open(c_path.as_ptr());
        assert!(!handle.is_null());

        let mut len = 0usize;
        let entries_ptr = lc_group_entries(handle, &mut len as *mut usize);
        assert!(!entries_ptr.is_null());
        assert_eq!(len, 2);

        let entries = unsafe { slice::from_raw_parts(entries_ptr, len) };
        let mut found_root = false;
        let mut found_sub = false;
        for entry in entries {
            if entry.path.is_null() {
                continue;
            }
            let path = unsafe { CStr::from_ptr(entry.path) }.to_string_lossy();
            if path == "root.txt" {
                found_root = true;
                assert!(!entry.is_directory);
                assert_eq!(entry.size, 5);
            } else if path == "sub" {
                found_sub = true;
                assert!(entry.is_directory);
            }
        }
        assert!(found_root);
        assert!(found_sub);
        lc_group_entries_free(entries_ptr, len);

        let relative = CString::new("sub/file.txt").unwrap();
        let mut data_len = 0usize;
        let data_ptr = lc_group_read_file(handle, relative.as_ptr(), &mut data_len as *mut usize);
        assert!(!data_ptr.is_null());
        assert_eq!(data_len, 5);
        let data = unsafe { slice::from_raw_parts(data_ptr, data_len) };
        assert_eq!(data, b"hello");
        lc_group_buffer_free(data_ptr, data_len);

        assert!(lc_group_exists(handle, relative.as_ptr()));
        assert!(lc_group_maker(handle).is_null());

        let root_ptr = lc_group_root(handle);
        assert!(!root_ptr.is_null());
        let root = unsafe { CStr::from_ptr(root_ptr) };
        assert_eq!(root.to_string_lossy(), dir.path().to_string_lossy());
        lc_group_string_free(root_ptr as *mut c_char);

        lc_group_free(handle);
    }

    #[test]
    fn ffi_open_missing_group_returns_null() {
        let missing = CString::new("/definitely/missing").unwrap();
        let handle = lc_group_open(missing.as_ptr());
        assert!(handle.is_null());
    }
}
