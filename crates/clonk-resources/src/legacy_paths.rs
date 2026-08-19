#[cfg(not(windows))]
use std::ffi::OsString;
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

/// Reproduce LegacyClonk's `StdFile::RealPath` for a path that may not exist.
///
/// POSIX `realpath` rejects a missing leaf, while the C++ helper keeps trying
/// shorter existing prefixes and appends the unresolved suffix. Windows uses
/// `_fullpath`, which is lexical and does not require the leaf to exist.
pub fn real_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        let mut normalized = PathBuf::new();
        for component in absolute.components() {
            match component {
                std::path::Component::CurDir => {}
                std::path::Component::ParentDir => {
                    if matches!(
                        normalized.components().next_back(),
                        Some(std::path::Component::Normal(_))
                    ) {
                        normalized.pop();
                    }
                }
                component => normalized.push(component.as_os_str()),
            }
        }
        normalized
    }

    #[cfg(not(windows))]
    {
        let original = path.to_path_buf();
        let mut prefix = path.to_path_buf();
        let mut suffix = Vec::<OsString>::new();
        loop {
            if let Ok(mut resolved) = std::fs::canonicalize(&prefix) {
                for component in suffix.iter().rev() {
                    resolved.push(component);
                }
                return resolved;
            }
            let Some(component) = prefix.file_name().map(OsString::from) else {
                return original;
            };
            suffix.push(component);
            if !prefix.pop() {
                return original;
            }
        }
    }
}

/// Return the byte identity used by `StdFile::ItemIdentical`.
///
/// Windows compares the `_fullpath` results case-insensitively using the
/// native ANSI path APIs. The explicit CP_ACP folds mirror the legacy byte
/// comparison used by the resource path layer; POSIX remains byte-exact.
pub fn path_identity_bytes(path: &Path) -> Vec<u8> {
    let bytes = path_to_legacy_bytes(&real_path(path));
    #[cfg(windows)]
    {
        let mut bytes = bytes;
        for byte in &mut bytes {
            *byte = match *byte {
                b'a'..=b'z' => *byte - 32,
                0xe4 => 0xc4,
                0xf6 => 0xd6,
                0xfc => 0xdc,
                _ => *byte,
            };
        }
        bytes
    }
    #[cfg(not(windows))]
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn legacy_path_round_trips_the_platform_native_projection() {
        let bytes = if cfg!(unix) {
            vec![b'G', b'r', 0xfc, b'p', b'.', b'c', b'4', b'd']
        } else {
            b"Group.c4d".to_vec()
        };
        assert_eq!(path_to_legacy_bytes(&path_from_legacy_bytes(&bytes)), bytes);
    }

    #[cfg(not(windows))]
    #[test]
    fn real_path_appends_missing_suffix_after_canonical_existing_prefix() {
        let fixture = tempdir().expect("real-path fixture");
        let existing = fixture.path().join("existing");
        std::fs::create_dir(&existing).expect("existing directory");
        let requested = existing
            .join(".")
            .join("missing")
            .join("..")
            .join("Player.c4p");

        assert_eq!(
            real_path(&requested),
            existing.join("missing").join("..").join("Player.c4p")
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_real_path_identity_folds_native_case() {
        let left = Path::new(r"C:\Players\Shared.c4p");
        let right = Path::new(r"c:\players\shared.c4p");
        assert_eq!(path_identity_bytes(left), path_identity_bytes(right));
        assert_eq!(
            path_identity_bytes(Path::new(r"C:\Players\..\Players\Shared.c4p")),
            path_identity_bytes(right)
        );
    }
}
