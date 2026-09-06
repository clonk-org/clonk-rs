//! The C ABI the pinned oracle's `USE_RUST_GROUP` bridge links against
//! (clonk-org/clonk-rs#1265).
//!
//! `parity/bridge/lc_group_ffi.h` is the pinned header verbatim. The ownership
//! contract has three distinct kinds of allocation, each with its own free —
//! getting them crossed is a heap corruption the C++ side cannot diagnose:
//!
//! - [`lc_group_open`] hands back a handle freed by [`lc_group_free`];
//! - [`lc_group_entries`] hands back an *array* freed by
//!   [`lc_group_entries_free`], which also owns each row's `path` string;
//! - [`lc_group_read_file`] hands back a byte buffer freed by
//!   [`lc_group_buffer_free`], and both it and the entries free need the same
//!   `len` they were given, because a boxed slice is deallocated by layout;
//! - [`lc_group_maker`] and [`lc_group_root`] hand back strings freed by
//!   [`lc_group_string_free`].
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

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_uchar};
use std::path::Path;
use std::ptr;

use crate::group::Group;

/// The opaque `GroupHandle` the header declares.
pub struct GroupHandle(Group);

/// One row of [`lc_group_entries`], matching the header's `LcGroupEntry`.
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
    let path = unsafe { CStr::from_ptr(path) };
    match Group::open(path.to_string_lossy().as_ref()) {
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

/// Lists the group's entries, writing the count through `out_len`.
///
/// `out_len` is set on every path including failure, so the caller never reads
/// a stale length beside a null pointer.
#[no_mangle]
pub extern "C" fn lc_group_entries(
    handle: *mut GroupHandle,
    out_len: *mut usize,
) -> *mut LcGroupEntry {
    if handle.is_null() || out_len.is_null() {
        return ptr::null_mut();
    }
    let handle = unsafe { &*handle };
    let Ok(entries) = handle.0.entries() else {
        unsafe { *out_len = 0 };
        return ptr::null_mut();
    };

    let mut rows: Vec<LcGroupEntry> = Vec::with_capacity(entries.len());
    for entry in entries {
        // A path containing an interior NUL cannot cross a C string boundary.
        // Abandon the whole array rather than hand back a short one the caller
        // would read as complete.
        let Ok(path) = CString::new(entry.relative_path.to_string_lossy().into_owned()) else {
            free_rows(&mut rows);
            unsafe { *out_len = 0 };
            return ptr::null_mut();
        };
        rows.push(LcGroupEntry {
            path: path.into_raw(),
            is_directory: entry.is_directory,
            size: entry.size,
        });
    }

    let len = rows.len();
    unsafe { *out_len = len };
    Box::into_raw(rows.into_boxed_slice()).cast::<LcGroupEntry>()
}

/// Frees the owned strings of rows that never reached the caller.
fn free_rows(rows: &mut Vec<LcGroupEntry>) {
    for row in rows.drain(..) {
        if !row.path.is_null() {
            unsafe { drop(CString::from_raw(row.path)) };
        }
    }
}

#[no_mangle]
pub extern "C" fn lc_group_entries_free(entries: *mut LcGroupEntry, len: usize) {
    if entries.is_null() {
        return;
    }
    let slice = ptr::slice_from_raw_parts_mut(entries, len);
    unsafe {
        for row in (*slice).iter_mut() {
            if !row.path.is_null() {
                drop(CString::from_raw(row.path));
                row.path = ptr::null_mut();
            }
        }
        drop(Box::from_raw(slice));
    }
}

#[no_mangle]
pub extern "C" fn lc_group_read_file(
    handle: *mut GroupHandle,
    relative_path: *const c_char,
    out_len: *mut usize,
) -> *mut c_uchar {
    if handle.is_null() || relative_path.is_null() || out_len.is_null() {
        return ptr::null_mut();
    }
    let handle = unsafe { &*handle };
    let relative = unsafe { CStr::from_ptr(relative_path) };
    match handle.0.read_file(relative.to_string_lossy().as_ref()) {
        Ok(data) => {
            let len = data.len();
            unsafe { *out_len = len };
            Box::into_raw(data.into_boxed_slice()).cast::<c_uchar>()
        }
        Err(_) => {
            unsafe { *out_len = 0 };
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
pub extern "C" fn lc_group_exists(handle: *mut GroupHandle, relative_path: *const c_char) -> bool {
    if handle.is_null() || relative_path.is_null() {
        return false;
    }
    let handle = unsafe { &*handle };
    let relative = unsafe { CStr::from_ptr(relative_path) };
    handle
        .0
        .exists(Path::new(relative.to_string_lossy().as_ref()))
}

#[no_mangle]
pub extern "C" fn lc_group_maker(handle: *mut GroupHandle) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let handle = unsafe { &*handle };
    handle
        .0
        .maker()
        .and_then(|maker| CString::new(maker).ok())
        .map_or(ptr::null_mut(), CString::into_raw)
}

#[no_mangle]
pub extern "C" fn lc_group_root(handle: *mut GroupHandle) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let handle = unsafe { &*handle };
    CString::new(handle.0.root().to_string_lossy().into_owned())
        .map_or(ptr::null_mut(), CString::into_raw)
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

    /// `Group::open` refuses a dot-prefixed directory as an ignored group
    /// entry, and `tempfile`'s default prefix is `.tmp`. The crate's own tests
    /// use a named prefix for the same reason.
    fn tempdir() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix("lc-ffi-test-")
            .tempdir()
            .expect("temp dir")
    }

    fn open(dir: &Path) -> *mut GroupHandle {
        let c_path = CString::new(dir.to_str().expect("utf-8 path")).expect("no NUL");
        let handle = lc_group_open(c_path.as_ptr());
        assert!(!handle.is_null(), "the directory opens as a group");
        handle
    }

    fn take_string(value: *mut c_char) -> Option<String> {
        (!value.is_null()).then(|| {
            let owned = unsafe { CStr::from_ptr(value) }
                .to_string_lossy()
                .into_owned();
            lc_group_string_free(value);
            owned
        })
    }

    #[test]
    fn a_directory_lists_its_entries_and_frees_them() {
        let dir = tempdir();
        std::fs::write(dir.path().join("Alpha.txt"), b"alpha").expect("write");
        std::fs::create_dir(dir.path().join("Nested")).expect("mkdir");
        let handle = open(dir.path());

        let mut len = usize::MAX;
        let rows = lc_group_entries(handle, &mut len);
        assert!(!rows.is_null(), "the listing succeeds");
        assert_eq!(len, 2, "one file and one directory");

        let listed: Vec<(String, bool)> = unsafe { std::slice::from_raw_parts(rows, len) }
            .iter()
            .map(|row| {
                (
                    unsafe { CStr::from_ptr(row.path) }
                        .to_string_lossy()
                        .into_owned(),
                    row.is_directory,
                )
            })
            .collect();
        assert!(
            listed.contains(&("Alpha.txt".to_string(), false)),
            "{listed:?}"
        );
        assert!(listed.contains(&("Nested".to_string(), true)), "{listed:?}");

        lc_group_entries_free(rows, len);
        lc_group_free(handle);
    }

    #[test]
    fn a_files_bytes_round_trip_through_the_buffer_contract() {
        let dir = tempdir();
        std::fs::write(dir.path().join("Data.bin"), b"\x00\x01\x02payload").expect("write");
        let handle = open(dir.path());

        let name = CString::new("Data.bin").expect("no NUL");
        let mut len = usize::MAX;
        let buffer = lc_group_read_file(handle, name.as_ptr(), &mut len);
        assert!(!buffer.is_null(), "the file reads");
        assert_eq!(
            unsafe { std::slice::from_raw_parts(buffer, len) },
            b"\x00\x01\x02payload",
            "interior NULs survive: this is a byte buffer, not a C string"
        );
        lc_group_buffer_free(buffer, len);
        lc_group_free(handle);
    }

    #[test]
    fn a_missing_file_is_null_with_a_zeroed_length() {
        let dir = tempdir();
        let handle = open(dir.path());

        let name = CString::new("Absent.bin").expect("no NUL");
        let mut len = usize::MAX;
        assert!(lc_group_read_file(handle, name.as_ptr(), &mut len).is_null());
        assert_eq!(len, 0, "the caller never reads a stale length");
        assert!(!lc_group_exists(handle, name.as_ptr()));
        lc_group_free(handle);
    }

    #[test]
    fn exists_answers_for_a_present_entry() {
        let dir = tempdir();
        std::fs::write(dir.path().join("Here.txt"), b"x").expect("write");
        let handle = open(dir.path());
        let name = CString::new("Here.txt").expect("no NUL");
        assert!(lc_group_exists(handle, name.as_ptr()));
        lc_group_free(handle);
    }

    #[test]
    fn the_root_is_reported_and_a_directory_has_no_maker() {
        let dir = tempdir();
        let handle = open(dir.path());
        let root = take_string(lc_group_root(handle)).expect("a root");
        assert!(root.contains(
            dir.path()
                .file_name()
                .expect("name")
                .to_string_lossy()
                .as_ref()
        ));
        // `Maker` is a packed C4Group header field, so an unpacked directory
        // has none and the bridge must see null rather than an empty string.
        assert_eq!(take_string(lc_group_maker(handle)), None);
        lc_group_free(handle);
    }

    #[test]
    fn every_entry_point_tolerates_a_null_handle() {
        let name = CString::new("Any").expect("no NUL");
        let mut len = usize::MAX;
        assert!(lc_group_entries(ptr::null_mut(), &mut len).is_null());
        assert!(lc_group_read_file(ptr::null_mut(), name.as_ptr(), &mut len).is_null());
        assert!(!lc_group_exists(ptr::null_mut(), name.as_ptr()));
        assert!(lc_group_maker(ptr::null_mut()).is_null());
        assert!(lc_group_root(ptr::null_mut()).is_null());
        assert!(lc_group_open(ptr::null()).is_null());
        // A null `out_len` is refused rather than written through.
        let handle = ptr::null_mut();
        assert!(lc_group_entries(handle, ptr::null_mut()).is_null());
        // Every free is a no-op on null, which is what lets the bridge call
        // them unconditionally on a failed path.
        lc_group_free(ptr::null_mut());
        lc_group_entries_free(ptr::null_mut(), 0);
        lc_group_buffer_free(ptr::null_mut(), 0);
        lc_group_string_free(ptr::null_mut());
    }
}
