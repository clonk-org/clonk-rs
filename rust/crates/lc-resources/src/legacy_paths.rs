use std::path::{Path, PathBuf};

/// Projects a classic native `char *` path into the host operating system's
/// path type. Unix retains arbitrary bytes; Windows uses the process ANSI code
/// page exactly like the C++ engine's `A` filesystem APIs.
#[cfg(unix)]
pub fn path_from_legacy_bytes(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStringExt as _;

    std::ffi::OsString::from_vec(bytes.to_vec()).into()
}

#[cfg(windows)]
pub fn path_from_legacy_bytes(bytes: &[u8]) -> PathBuf {
    use std::os::windows::ffi::OsStringExt as _;
    use std::ptr::null_mut;
    use windows_sys::Win32::Globalization::{MultiByteToWideChar, CP_ACP};

    if bytes.is_empty() {
        return PathBuf::new();
    }
    let byte_count = i32::try_from(bytes.len()).expect("legacy path exceeds the Win32 API limit");
    let wide_count =
        unsafe { MultiByteToWideChar(CP_ACP, 0, bytes.as_ptr(), byte_count, null_mut(), 0) };
    assert!(
        wide_count > 0,
        "MultiByteToWideChar(CP_ACP) failed for a legacy path: {}",
        std::io::Error::last_os_error()
    );
    let mut wide = vec![0_u16; wide_count as usize];
    let written = unsafe {
        MultiByteToWideChar(
            CP_ACP,
            0,
            bytes.as_ptr(),
            byte_count,
            wide.as_mut_ptr(),
            wide_count,
        )
    };
    assert_eq!(
        written, wide_count,
        "MultiByteToWideChar(CP_ACP) changed size for a legacy path"
    );
    std::ffi::OsString::from_wide(&wide).into()
}

#[cfg(all(not(unix), not(windows)))]
pub fn path_from_legacy_bytes(bytes: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
}

/// Projects an operating-system path back through the C++ engine's native
/// `char *` boundary.
#[cfg(unix)]
pub fn path_to_legacy_bytes(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt as _;

    path.as_os_str().as_bytes().to_vec()
}

#[cfg(windows)]
pub fn path_to_legacy_bytes(path: &Path) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt as _;
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Globalization::{WideCharToMultiByte, CP_ACP};

    let wide = path.as_os_str().encode_wide().collect::<Vec<_>>();
    if wide.is_empty() {
        return Vec::new();
    }
    let wide_count = i32::try_from(wide.len()).expect("path exceeds the Win32 API limit");
    let byte_count = unsafe {
        WideCharToMultiByte(
            CP_ACP,
            0,
            wide.as_ptr(),
            wide_count,
            null_mut(),
            0,
            null(),
            null_mut(),
        )
    };
    assert!(
        byte_count > 0,
        "WideCharToMultiByte(CP_ACP) failed for a path: {}",
        std::io::Error::last_os_error()
    );
    let mut bytes = vec![0_u8; byte_count as usize];
    let written = unsafe {
        WideCharToMultiByte(
            CP_ACP,
            0,
            wide.as_ptr(),
            wide_count,
            bytes.as_mut_ptr(),
            byte_count,
            null(),
            null_mut(),
        )
    };
    assert_eq!(
        written, byte_count,
        "WideCharToMultiByte(CP_ACP) changed size for a path"
    );
    bytes
}

#[cfg(all(not(unix), not(windows)))]
pub fn path_to_legacy_bytes(path: &Path) -> Vec<u8> {
    path.to_string_lossy().as_bytes().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_path_round_trips_the_platform_native_projection() {
        let bytes = if cfg!(unix) {
            vec![b'G', b'r', 0xfc, b'p', b'.', b'c', b'4', b'd']
        } else {
            b"Group.c4d".to_vec()
        };
        assert_eq!(path_to_legacy_bytes(&path_from_legacy_bytes(&bytes)), bytes);
    }
}
