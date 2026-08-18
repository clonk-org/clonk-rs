//! Installing a downloaded release over a live install.
//!
//! Everything here is written to one rule: **an interrupted or failed apply
//! must leave a working install**. A user who never updates loses nothing; a
//! user whose update fails halfway must be no worse off.
//!
//! # How a component is replaced
//!
//! Each component becomes one directory swap. For a destination `P` the new
//! tree is staged at the sibling `P.new-<nonce>` and the swap is
//! `rename(P -> backup)` followed by `rename(P.new-<nonce> -> P)`. Staging is a
//! sibling rather than something under `cache_dir`/`temp_dir` because `rename`
//! cannot cross filesystems: an install on another volume — `/opt`, a second
//! disk, a USB stick — would fail with `EXDEV` at the worst possible moment.
//!
//! Components are applied `content`, then `planet`, then `engine`, so the
//! binaries that would run a recovery are the last thing to change.
//!
//! # What is *not* replaced
//!
//! Only `content`, `planet` and the binaries directory (`bin`, or
//! `Contents/MacOS` inside a bundle) are ever swapped. The install tree is not
//! read-only: `Screenshots/`, `Records.c4f/`, the launcher's `Clonk-rust-*.log`
//! and the launcher-staged `System.c4g`/`Graphics.c4g` all live in it, and a
//! whole-tree replace would delete them. Inside a component, only a top-level
//! entry of the old `content/` tree that the new archive does not contain — a
//! pack the user dropped there, which `clonk-app` scans — is carried into the
//! staged tree before the swap. `planet/` and the binaries directory are exact
//! release snapshots; retaining an omitted entry there would create a hybrid
//! install. The launcher recreates its staged `System.c4g`/`Graphics.c4g`
//! copies from the installed `planet/` before starting the runtime.
//!
//! The engine component also ships `COPYING`, `README.md`, `credits.txt` and,
//! inside a bundle, `Contents/Info.plist`. Those are deliberately left alone:
//! they are not worth the risk of a file-level overwrite that no rename can
//! make atomic, and a stale copyright notice is not a failure a user can see.
//!
//! `Contents/Resources/ClonkRust.icns` is the one exception, because a stale
//! icon *is* a failure a user can see — an install updated in place otherwise
//! keeps its original icon for ever. It is replaced by a rename within its own
//! directory, which is atomic, with the old one moved aside so a rollback can
//! put it back. See `install_bundle_icon`.
//!
//! # macOS
//!
//! Replacing anything under `Contents/` breaks the bundle's code signature, and
//! a broken seal is reported by macOS as *"damaged and can't be opened"* —
//! strictly worse than a stale but working copy. So the bundle is re-signed
//! ad-hoc after the last swap, mirroring `xtask/src/main.rs`'s
//! `sign_macos_bundle`: the nested `clonk-game` and `c4group` executables first,
//! then the bundle, then `codesign --verify --deep --strict`. If verification
//! fails the whole update is rolled back.
//!
//! That has one consequence worth stating plainly, because it was **measured
//! rather than assumed**: `codesign --verify --strict` refuses a bundle holding
//! *any* file its seal does not cover, reporting `unsealed contents present in
//! the bundle root` for a file next to `Contents` and `a sealed resource is
//! missing or invalid` for one removed after signing. There is therefore
//! nowhere inside a `.app` to keep a journal or a backup across the signing
//! step, so for a bundle both live in its private namespace below the directory
//! *containing* the `.app` ([`InstallLayout::work_dir`]). A plain install keeps
//! them beside the destination, where the rename that produced them left them.

use crate::decide::PlannedComponent;
use crate::digest::{verify_file, DigestError};
use crate::extract::{extract_archive, ExtractError};
use crate::journal::{
    safe_child_name, InstallIdentity, Journal, JournalError, JournalStep, PreviousInstalledState,
    StepState, TransactionPhase,
};
use crate::recovery_registry;
use crate::state::{InstalledState, InstalledStateSnapshot, StateError};
use clonk_platform::AppPaths;
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::time::Duration;
use thiserror::Error;

/// Headroom demanded on top of the payload before a download starts.
///
/// An install that fills its own volume is a failure mode the user cannot fix
/// from inside the game, so the check is deliberately pessimistic.
pub const RESERVED_FREE_BYTES: u64 = 256 * 1024 * 1024;
/// Set by `clonk-game` after it has completed install-journal recovery, so the
/// child runtime does not try to upgrade the same shared install lease to an
/// exclusive recovery lock.
pub const UPDATE_RECOVERY_COMPLETE_ENV: &str = "LC_GAME_UPDATE_RECOVERY_COMPLETE";

/// How much a component archive is allowed to unpack to, as a multiple of its
/// own size.
///
/// The manifest records only the *archive* size, so the applier has to supply
/// the bound [`extract_archive`] enforces. Component payloads are largely
/// already-compressed group files, so eight times is generous for honest data
/// and still a hard ceiling on a decompression bomb.
const UNPACKED_BUDGET_FACTOR: u64 = 8;

/// A floor under [`UNPACKED_BUDGET_FACTOR`], so a small component is not
/// refused for expanding by more than its own tiny size.
const MINIMUM_UNPACKED_BUDGET: u64 = 64 * 1024 * 1024;

/// The groups `clonk-game`'s `ensure_runtime_asset` stages beside the
/// executables, purged after a `planet` swap so the next launch recreates them
/// from the new `planet/`.
pub const LAUNCHER_STAGED_GROUPS: [&str; 2] = ["System.c4g", "Graphics.c4g"];

/// The nested executables a bundle must sign before the bundle that seals them.
const NESTED_BUNDLE_EXECUTABLES: [&str; 2] = ["clonk-game", "c4group"];

/// The bundle icon, relative to the `.app`, as `xtask` writes it and
/// `Info.plist`'s `CFBundleIconFile` names it.
const BUNDLE_ICON: &str = "Contents/Resources/ClonkRust.icns";

/// A stable file whose host lock serializes apply and recovery processes.
///
/// The file itself is deliberately retained: unlinking a locked file lets a
/// second process create a new inode at the same name and lock that instead.
const UPDATE_LOCK_FILE_NAME: &str = ".clonk-update.lock";

/// `HKCU` subkey the Windows installer writes, and the value naming the
/// installed release (`scripts/windows-installer.nsi:54,76`).
#[cfg(windows)]
const UNINSTALL_KEY: &[u8] =
    b"Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\ClonkRust\0";
#[cfg(windows)]
const DISPLAY_VERSION_VALUE: &[u8] = b"DisplayVersion\0";

/// Where the shape of an install puts the things an update replaces.
///
/// A plain install is a directory holding `bin/`, `planet/` and `content/`. A
/// macOS install is an `.app` whose `Contents/MacOS` and `Contents/Resources`
/// hold the same things, and whose enclosing directory is the only writable
/// place a backup can wait without breaking the code signature.
///
/// The root is what a manifest's `install` destinations are relative to, which
/// on macOS is the bundle and *not* `AppPaths::install_root`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallLayout {
    root: PathBuf,
    bundle: bool,
    /// Where the path-independent transaction pointers live.
    ///
    /// `None` — every production layout — resolves the per-user directory from
    /// the environment when it is needed, so constructing a layout stays a pure
    /// function of the path it is given. Tests name their own.
    recovery_registry: Option<PathBuf>,
}

impl InstallLayout {
    pub fn plain(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            bundle: false,
            recovery_registry: None,
        }
    }

    /// `app_dir` is the `.app` itself, not its `Contents/Resources`.
    pub fn macos_bundle(app_dir: impl Into<PathBuf>) -> Self {
        Self {
            root: app_dir.into(),
            bundle: true,
            recovery_registry: None,
        }
    }

    /// Points this layout at an explicit registry directory instead of the
    /// per-user one.
    pub fn with_recovery_registry(mut self, registry: impl Into<PathBuf>) -> Self {
        self.recovery_registry = Some(registry.into());
        self
    }

    /// The registry this install records its transaction in, or `None` when the
    /// platform names no per-user directory to keep it in.
    fn recovery_registry(&self) -> Option<PathBuf> {
        self.recovery_registry
            .clone()
            .or_else(recovery_registry::default_dir)
    }

    /// The layout of a discovered installation.
    pub fn for_app_paths(paths: &AppPaths) -> Self {
        paths
            .macos_bundle_root()
            .map(Self::macos_bundle)
            .unwrap_or_else(|| Self::plain(paths.install_root()))
    }

    /// Infers the layout from a path, accepting either a bundle or the install
    /// root inside one.
    ///
    /// Both are accepted because callers hold different things: startup
    /// recovery has `AppPaths::install_root` (a bundle's `Contents/Resources`),
    /// while an applier invoked with an explicit path has the `.app`. Guessing
    /// wrong would look for a journal in a directory that never had one and
    /// silently report that nothing was interrupted.
    pub fn discover(path: &Path) -> Self {
        let inside_bundle = path.file_name() == Some(OsStr::new("Resources"))
            && path.parent().and_then(Path::file_name) == Some(OsStr::new("Contents"));
        let bundle_root = if inside_bundle {
            path.parent().and_then(Path::parent).map(Path::to_path_buf)
        } else if path.join("Contents").join("MacOS").is_dir() {
            Some(path.to_path_buf())
        } else {
            None
        };
        bundle_root
            .map(Self::macos_bundle)
            .unwrap_or_else(|| Self::plain(path))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn is_bundle(&self) -> bool {
        self.bundle
    }

    /// The game data root, relative to [`Self::root`].
    fn data_relative(&self) -> PathBuf {
        match self.bundle {
            true => PathBuf::from("Contents").join("Resources"),
            false => PathBuf::new(),
        }
    }

    /// The executables directory, relative to [`Self::root`].
    fn binaries_relative(&self) -> PathBuf {
        match self.bundle {
            true => PathBuf::from("Contents").join("MacOS"),
            false => PathBuf::from("bin"),
        }
    }

    pub fn data_dir(&self) -> PathBuf {
        self.root.join(self.data_relative())
    }

    pub fn binaries_dir(&self) -> PathBuf {
        self.root.join(self.binaries_relative())
    }

    /// Where the journal and the backups wait while an update is in flight.
    ///
    /// Beside the install for a plain tree. A bundle gets its own namespace
    /// under a hidden directory beside the `.app`, because nothing inside a
    /// `.app` can survive the re-sign that follows the last swap and sibling
    /// bundles must never share recovery state.
    pub fn work_dir(&self) -> PathBuf {
        match self.bundle {
            true => {
                let parent = self.root.parent().unwrap_or_else(|| Path::new("."));
                let name = self
                    .root
                    .file_name()
                    .unwrap_or_else(|| OsStr::new("bundle"));
                parent.join(".clonk-update").join(name)
            }
            false => self.root.clone(),
        }
    }

    fn scratch_dir(&self, nonce: &str) -> PathBuf {
        self.work_dir().join(format!("clonk-update-stage-{nonce}"))
    }

    fn quarantine_dir(&self, nonce: &str) -> PathBuf {
        self.work_dir().join(format!("clonk-update-backup-{nonce}"))
    }
}

/// One component, downloaded and waiting to be installed.
///
/// Serializable because the applier runs as a separate process
/// (`clonk-game --apply-update <plan.json>`) so that the binaries it replaces
/// are not the ones executing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StagedComponent {
    pub name: String,
    /// The downloaded archive, which may sit on any filesystem.
    pub archive: PathBuf,
    /// Re-verified immediately before extraction: the download was checked when
    /// it arrived, but the applier is a different process at a later time.
    pub sha256: String,
    /// Archive size from the manifest, which bounds what it may unpack to.
    pub size: u64,
    /// Where the component lands, relative to [`InstallLayout::root`].
    pub destination: PathBuf,
}

impl StagedComponent {
    pub fn from_planned(planned: &PlannedComponent, archive: PathBuf) -> Self {
        Self {
            name: planned.name.clone(),
            archive,
            sha256: planned.sha256.clone(),
            size: planned.size,
            destination: planned.destination.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplyPlan {
    pub version: String,
    pub components: Vec<StagedComponent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyOutcome {
    pub version: String,
    /// Component names in the order they were installed.
    pub applied: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeOutcome {
    /// No journal: nothing was interrupted, which is every ordinary launch.
    NothingToDo,
    RolledForward {
        version: String,
    },
    RolledBack {
        version: String,
    },
}

#[derive(Debug, Error)]
pub enum PlatformError {
    #[error("{operation} is not available on this platform")]
    Unsupported { operation: &'static str },
    #[error("{operation} failed for {path}: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("codesign {arguments} failed for {target}: {status}")]
    Codesign {
        arguments: String,
        target: PathBuf,
        status: String,
    },
    #[error("process {pid} was still running after {seconds}s")]
    WaitTimeout { pid: u32, seconds: u64 },
}

/// The host operations an apply needs, behind a trait so the whole path —
/// including the macOS re-sign — runs in a `TempDir` on any machine.
pub trait PlatformOps {
    /// Bytes an unprivileged writer may still use on the filesystem holding
    /// `path`.
    fn available_space(&self, path: &Path) -> Result<u64, PlatformError>;

    /// Blocks until `pid` exits, or the timeout elapses.
    ///
    /// The applier must outlive the processes writing into the tree it is about
    /// to rename — on Windows and Linux the launcher outlives the runtime.
    fn wait_for_process(&self, pid: u32, timeout: Duration) -> Result<(), PlatformError>;

    fn codesign(&self, arguments: &[&str], target: &Path) -> Result<(), PlatformError>;

    /// Records the installed release where the platform's software inventory
    /// reads it — the Windows uninstall entry's `DisplayVersion`. A no-op
    /// elsewhere.
    fn set_installed_version(&self, version: &str) -> Result<(), PlatformError>;
}

/// The real host.
#[derive(Debug, Default, Clone, Copy)]
pub struct RealPlatform;

impl PlatformOps for RealPlatform {
    fn available_space(&self, path: &Path) -> Result<u64, PlatformError> {
        available_space(path)
    }

    fn wait_for_process(&self, pid: u32, timeout: Duration) -> Result<(), PlatformError> {
        wait_for_process(pid, timeout)
    }

    #[cfg(target_os = "macos")]
    fn codesign(&self, arguments: &[&str], target: &Path) -> Result<(), PlatformError> {
        // The absolute path is base-OS and needs no Xcode installation.
        let status = std::process::Command::new("/usr/bin/codesign")
            .args(arguments)
            .arg(target)
            .status()
            .map_err(|source| PlatformError::Io {
                operation: "codesign",
                path: target.to_path_buf(),
                source,
            })?;
        status
            .success()
            .then_some(())
            .ok_or_else(|| PlatformError::Codesign {
                arguments: arguments.join(" "),
                target: target.to_path_buf(),
                status: format!("{status}"),
            })
    }

    #[cfg(not(target_os = "macos"))]
    fn codesign(&self, _arguments: &[&str], _target: &Path) -> Result<(), PlatformError> {
        Err(PlatformError::Unsupported {
            operation: "codesign",
        })
    }

    fn set_installed_version(&self, version: &str) -> Result<(), PlatformError> {
        set_installed_version(version)
    }
}

#[cfg(unix)]
#[allow(clippy::unnecessary_cast)] // `f_bavail`/`f_frsize` widths vary by libc.
fn available_space(path: &Path) -> Result<u64, PlatformError> {
    use std::os::unix::ffi::OsStrExt;

    let io = |source| PlatformError::Io {
        operation: "statvfs",
        path: path.to_path_buf(),
        source,
    };
    let text = std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io(std::io::Error::from(std::io::ErrorKind::InvalidInput)))?;
    let mut stats: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `text` is a NUL-terminated path valid for the call, and `stats`
    // is a live, correctly typed out parameter.
    let status = unsafe { libc::statvfs(text.as_ptr(), &mut stats) };
    if status != 0 {
        return Err(io(std::io::Error::last_os_error()));
    }
    // `f_bavail` deliberately, not `f_bfree`: the reserved blocks are not ours.
    Ok((stats.f_bavail as u64).saturating_mul(stats.f_frsize as u64))
}

#[cfg(windows)]
fn available_space(path: &Path) -> Result<u64, PlatformError> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut available: u64 = 0;
    // SAFETY: `wide` is a NUL-terminated wide path valid for the call, and the
    // two totals we do not need are passed as null, which the API permits.
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    (ok != 0)
        .then_some(available)
        .ok_or_else(|| PlatformError::Io {
            operation: "GetDiskFreeSpaceExW",
            path: path.to_path_buf(),
            source: std::io::Error::last_os_error(),
        })
}

#[cfg(not(any(unix, windows)))]
fn available_space(path: &Path) -> Result<u64, PlatformError> {
    let _ = path;
    Err(PlatformError::Unsupported {
        operation: "free space",
    })
}

/// How often a unix wait re-checks whether the process is gone. Windows blocks
/// on a handle instead and needs no poll.
#[cfg(unix)]
const PROCESS_POLL: Duration = Duration::from_millis(50);

#[cfg(any(test, windows))]
const WINDOWS_ERROR_INVALID_PARAMETER: u32 = 87;
#[cfg(any(test, windows))]
const WINDOWS_WAIT_OBJECT_0: u32 = 0;
#[cfg(any(test, windows))]
const WINDOWS_WAIT_TIMEOUT: u32 = 0x102;
#[cfg(any(test, windows))]
const WINDOWS_WAIT_FAILED: u32 = u32::MAX;

#[cfg(any(test, windows))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WindowsOpenError {
    ProcessGone,
    Failure(u32),
}

#[cfg(any(test, windows))]
fn classify_windows_open_error(code: u32) -> WindowsOpenError {
    match code {
        WINDOWS_ERROR_INVALID_PARAMETER => WindowsOpenError::ProcessGone,
        _ => WindowsOpenError::Failure(code),
    }
}

#[cfg(any(test, windows))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WindowsWaitResult {
    Exited,
    TimedOut,
    Failure(u32),
    Unexpected(u32),
}

#[cfg(any(test, windows))]
fn classify_windows_wait(status: u32, last_error: u32) -> WindowsWaitResult {
    match status {
        WINDOWS_WAIT_OBJECT_0 => WindowsWaitResult::Exited,
        WINDOWS_WAIT_TIMEOUT => WindowsWaitResult::TimedOut,
        WINDOWS_WAIT_FAILED => WindowsWaitResult::Failure(last_error),
        other => WindowsWaitResult::Unexpected(other),
    }
}

#[cfg(unix)]
fn wait_for_process(pid: u32, timeout: Duration) -> Result<(), PlatformError> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        // SAFETY: signal 0 performs the permission and existence checks without
        // delivering anything.
        let alive = unsafe { libc::kill(pid as libc::pid_t, 0) } == 0
            || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM);
        if !alive {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            return Err(PlatformError::WaitTimeout {
                pid,
                seconds: timeout.as_secs(),
            });
        }
        std::thread::sleep(PROCESS_POLL);
    }
}

#[cfg(windows)]
fn wait_for_process(pid: u32, timeout: Duration) -> Result<(), PlatformError> {
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError};
    // `SYNCHRONIZE` is a standard access right; windows-sys happens to declare
    // it beside the file rights, and both aliases are plain `u32`.
    use windows_sys::Win32::Storage::FileSystem::SYNCHRONIZE;
    use windows_sys::Win32::System::Threading::{OpenProcess, WaitForSingleObject};

    // SAFETY: a plain process-id lookup.
    let handle = unsafe { OpenProcess(SYNCHRONIZE, 0, pid) };
    if handle.is_null() {
        let code = unsafe { GetLastError() };
        return match classify_windows_open_error(code) {
            WindowsOpenError::ProcessGone => Ok(()),
            WindowsOpenError::Failure(code) => Err(PlatformError::Io {
                operation: "OpenProcess",
                path: PathBuf::from(format!("process {pid}")),
                source: std::io::Error::from_raw_os_error(code as i32),
            }),
        };
    }
    let milliseconds = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX);
    // SAFETY: `handle` is a live handle this function owns until it is closed.
    let waited = unsafe { WaitForSingleObject(handle, milliseconds) };
    let wait_error = unsafe { GetLastError() };
    // SAFETY: closing the handle opened above, exactly once.
    unsafe { CloseHandle(handle) };
    match classify_windows_wait(waited, wait_error) {
        WindowsWaitResult::Exited => Ok(()),
        WindowsWaitResult::TimedOut => Err(PlatformError::WaitTimeout {
            pid,
            seconds: timeout.as_secs(),
        }),
        WindowsWaitResult::Failure(code) => Err(PlatformError::Io {
            operation: "WaitForSingleObject",
            path: PathBuf::from(format!("process {pid}")),
            source: std::io::Error::from_raw_os_error(code as i32),
        }),
        WindowsWaitResult::Unexpected(status) => Err(PlatformError::Io {
            operation: "WaitForSingleObject",
            path: PathBuf::from(format!("process {pid}")),
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("unexpected wait status {status:#x}"),
            ),
        }),
    }
}

#[cfg(not(any(unix, windows)))]
fn wait_for_process(pid: u32, timeout: Duration) -> Result<(), PlatformError> {
    let _ = (pid, timeout);
    Err(PlatformError::Unsupported {
        operation: "waiting for a process",
    })
}

#[cfg(windows)]
fn set_installed_version(version: &str) -> Result<(), PlatformError> {
    use std::ffi::CString;
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExA, RegSetValueExA, HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_SZ,
    };

    let failed = |operation: &'static str, status: u32| PlatformError::Io {
        operation,
        path: PathBuf::from("HKCU\\...\\Uninstall\\ClonkRust"),
        source: std::io::Error::from_raw_os_error(status as i32),
    };

    let mut key: HKEY = std::ptr::null_mut();
    // SAFETY: a NUL-terminated subkey and a live out parameter.
    let status = unsafe {
        RegOpenKeyExA(
            HKEY_CURRENT_USER,
            UNINSTALL_KEY.as_ptr(),
            0,
            KEY_SET_VALUE,
            &mut key,
        )
    };
    // A portable (zip) install has no uninstall entry, and inventing one would
    // advertise an uninstaller that does not exist.
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(());
    }
    if status != ERROR_SUCCESS {
        return Err(failed("RegOpenKeyExA", status));
    }

    let text = CString::new(version).map_err(|_| {
        failed(
            "RegSetValueExA",
            windows_sys::Win32::Foundation::ERROR_INVALID_DATA,
        )
    })?;
    let bytes = text.as_bytes_with_nul();
    // SAFETY: `key` is open for writing, and `bytes` is a NUL-terminated buffer
    // of exactly the length passed alongside it.
    let status = unsafe {
        RegSetValueExA(
            key,
            DISPLAY_VERSION_VALUE.as_ptr(),
            0,
            REG_SZ,
            bytes.as_ptr(),
            bytes.len() as u32,
        )
    };
    // SAFETY: closing the handle opened above, exactly once.
    unsafe { RegCloseKey(key) };
    (status == ERROR_SUCCESS)
        .then_some(())
        .ok_or_else(|| failed("RegSetValueExA", status))
}

#[cfg(not(windows))]
fn set_installed_version(version: &str) -> Result<(), PlatformError> {
    let _ = version;
    Ok(())
}

/// A recorded host operation, so tests can assert on order rather than effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlatformCall {
    Codesign {
        arguments: Vec<String>,
        target: PathBuf,
    },
    WaitForProcess {
        pid: u32,
    },
    SetInstalledVersion {
        version: String,
    },
}

/// A [`PlatformOps`] that touches nothing, for exercising the whole apply path
/// — the macOS re-sign included — inside a temporary directory on any host.
///
/// Not `#[cfg(test)]`: `clonk-game` and `clonk-app` need it to test their own
/// wiring, and a test double that only exists inside this crate would be
/// reimplemented badly in each of them.
#[derive(Debug)]
pub struct FakePlatform {
    available_space: u64,
    failing_codesign_argument: Option<String>,
    failing_codesign_once: bool,
    codesign_has_failed: std::sync::atomic::AtomicBool,
    calls: std::sync::Mutex<Vec<PlatformCall>>,
}

impl Default for FakePlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl FakePlatform {
    pub fn new() -> Self {
        Self {
            available_space: u64::MAX,
            failing_codesign_argument: None,
            failing_codesign_once: false,
            codesign_has_failed: std::sync::atomic::AtomicBool::new(false),
            calls: std::sync::Mutex::new(Vec::new()),
        }
    }

    pub fn with_available_space(mut self, bytes: u64) -> Self {
        self.available_space = bytes;
        self
    }

    /// Makes every `codesign` invocation carrying `argument` fail, which is how
    /// "the bundle does not verify after the swap" is reproduced without a
    /// signing identity or even a macOS host.
    pub fn failing_codesign(mut self, argument: &str) -> Self {
        self.failing_codesign_argument = Some(argument.to_string());
        self
    }

    /// Fails only the first `codesign` invocation carrying `argument`.
    pub fn failing_codesign_once(mut self, argument: &str) -> Self {
        self.failing_codesign_argument = Some(argument.to_string());
        self.failing_codesign_once = true;
        self
    }

    pub fn calls(&self) -> Vec<PlatformCall> {
        self.calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn record(&self, call: PlatformCall) {
        self.calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(call);
    }
}

impl PlatformOps for FakePlatform {
    fn available_space(&self, _path: &Path) -> Result<u64, PlatformError> {
        Ok(self.available_space)
    }

    fn wait_for_process(&self, pid: u32, _timeout: Duration) -> Result<(), PlatformError> {
        self.record(PlatformCall::WaitForProcess { pid });
        Ok(())
    }

    fn codesign(&self, arguments: &[&str], target: &Path) -> Result<(), PlatformError> {
        self.record(PlatformCall::Codesign {
            arguments: arguments
                .iter()
                .map(|argument| argument.to_string())
                .collect(),
            target: target.to_path_buf(),
        });
        let matches_failure = self
            .failing_codesign_argument
            .as_deref()
            .is_some_and(|failing| arguments.contains(&failing));
        let already_failed = matches_failure
            && self
                .codesign_has_failed
                .swap(true, std::sync::atomic::Ordering::Relaxed);
        let failing = matches_failure && (!self.failing_codesign_once || !already_failed);
        match failing {
            true => Err(PlatformError::Codesign {
                arguments: arguments.join(" "),
                target: target.to_path_buf(),
                status: "exit status: 1".to_string(),
            }),
            false => Ok(()),
        }
    }

    fn set_installed_version(&self, version: &str) -> Result<(), PlatformError> {
        self.record(PlatformCall::SetInstalledVersion {
            version: version.to_string(),
        });
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum ApplyError {
    #[error("update component {component:?} is not one this build installs")]
    UnknownComponent { component: String },
    #[error("update journal repeats component {component:?}")]
    DuplicateComponent { component: String },
    #[error("update journal belongs to {recorded}, not the requested installation at {requested}")]
    InstallRootMismatch {
        recorded: PathBuf,
        requested: PathBuf,
    },
    #[error("macOS update journal does not record whether the old bundle icon existed")]
    MissingBundleRecoveryState,
    #[error("legacy macOS update journal cannot prove the original bundle icon state")]
    UnsafeLegacyBundleJournal,
    #[error("legacy update journal lacks the digest required for safe recovery")]
    UnsafeLegacyJournalDigest,
    #[error("rollback state for component {component:?} is inconsistent with its journal")]
    InconsistentRollbackState { component: String },
    #[error(
        "component {component} asks to be installed at {destination}, \
         but this install puts it at {expected}"
    )]
    DestinationOutOfScope {
        component: String,
        destination: PathBuf,
        expected: PathBuf,
    },
    #[error("the {component} archive does not contain {path}")]
    MissingPayload { component: String, path: PathBuf },
    #[error("{path} already exists; another update may be in progress")]
    StagingOccupied { path: PathBuf },
    #[error("another update process already holds {path}")]
    UpdateInProgress { path: PathBuf },
    #[error("the staged tree for component {component} is gone, so the update cannot be finished")]
    StagingLost { component: String },
    #[error("{path} holds an entry whose name is not text and cannot be carried across an update")]
    UnrepresentableEntry { path: PathBuf },
    #[error("{path} has no name, so nothing can be staged beside it")]
    UnnamedPath { path: PathBuf },
    #[error("{operation} {path} failed: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "installing this update needs {needed} bytes on {path}, but only {available} are free"
    )]
    NotEnoughSpace {
        path: PathBuf,
        needed: u64,
        available: u64,
    },
    #[error("{cause}; the install could not be rolled back either: {failure}")]
    RollbackFailed { cause: String, failure: String },
    #[error(transparent)]
    Digest(#[from] DigestError),
    #[error(transparent)]
    Extract(#[from] ExtractError),
    #[error(transparent)]
    Journal(#[from] JournalError),
    #[error(transparent)]
    State(#[from] StateError),
    #[error(transparent)]
    Platform(#[from] PlatformError),
}

struct UpdateLock {
    _file: File,
}

impl UpdateLock {
    fn path_in(directory: &Path) -> PathBuf {
        directory.join(UPDATE_LOCK_FILE_NAME)
    }

    fn acquire(directory: &Path) -> Result<Self, ApplyError> {
        let path = Self::path_in(directory);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(io("opening update lock", &path))?;
        match file.try_lock() {
            Ok(()) => Ok(Self { _file: file }),
            Err(TryLockError::WouldBlock) => Err(ApplyError::UpdateInProgress { path }),
            Err(TryLockError::Error(source)) => Err(io("locking", &path)(source)),
        }
    }
}

/// Shared lifetime claim held by every process using an installed tree.
///
/// The updater takes an exclusive lock on the same file before its first live
/// mutation, so a second launcher or a directly started runtime cannot be
/// missed by a finite PID hand-off list. A read-only installation receives a
/// no-op guard: the updater cannot create its journal or mutate that tree
/// either.
pub struct InstallUseGuard {
    _file: Option<File>,
}

pub fn acquire_install_use(layout: &InstallLayout) -> Result<InstallUseGuard, ApplyError> {
    let work = layout.work_dir();
    if let Err(error) = ensure_dir(&work) {
        if matches!(
            &error,
            ApplyError::Io { source, .. }
                if matches!(
                    source.kind(),
                    std::io::ErrorKind::PermissionDenied
                        | std::io::ErrorKind::ReadOnlyFilesystem
                )
        ) {
            return Ok(InstallUseGuard { _file: None });
        }
        return Err(error);
    }
    let path = UpdateLock::path_in(&work);
    let file = match OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
    {
        Ok(file) => file,
        Err(source)
            if matches!(
                source.kind(),
                std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::ReadOnlyFilesystem
            ) =>
        {
            return Ok(InstallUseGuard { _file: None });
        }
        Err(source) => return Err(io("opening install-use lock", &path)(source)),
    };
    match file.try_lock_shared() {
        Ok(()) => Ok(InstallUseGuard { _file: Some(file) }),
        Err(TryLockError::WouldBlock) => Err(ApplyError::UpdateInProgress { path }),
        Err(TryLockError::Error(source)) => Err(io("locking install for use", &path)(source)),
    }
}

/// Bytes that must be free before a download starts.
///
/// Twice the archive total plus headroom: the download and its unpacked form
/// coexist, and component payloads are already-compressed groups that barely
/// grow. Deliberately a pre-*download* check — refusing after 299 MB has been
/// fetched is a worse experience than refusing before.
pub fn required_free_space<I: IntoIterator<Item = u64>>(archive_sizes: I) -> u64 {
    archive_sizes
        .into_iter()
        .fold(0u64, |total, size| {
            total.saturating_add(size.saturating_mul(2))
        })
        .saturating_add(RESERVED_FREE_BYTES)
}

/// Refuses an update that would not fit, before a byte of it is fetched.
pub fn ensure_free_space<I: IntoIterator<Item = u64>>(
    ops: &dyn PlatformOps,
    path: &Path,
    archive_sizes: I,
) -> Result<(), ApplyError> {
    let needed = required_free_space(archive_sizes);
    let available = ops.available_space(path)?;
    (available >= needed)
        .then_some(())
        .ok_or_else(|| ApplyError::NotEnoughSpace {
            path: path.to_path_buf(),
            needed,
            available,
        })
}

/// Apply order: data first, the binaries that would run a recovery last.
fn apply_rank(component: &str) -> Option<u8> {
    match component {
        "content" => Some(0),
        "planet" => Some(1),
        "engine" => Some(2),
        _ => None,
    }
}

/// One resolved directory swap.
#[derive(Debug, Clone)]
struct Replacement {
    component: String,
    archive: PathBuf,
    sha256: String,
    size: u64,
    /// Relative to the layout root.
    destination: PathBuf,
    /// The subtree of the extracted archive that becomes the destination.
    source: PathBuf,
}

/// Turns a planned component into the one subtree it may replace, refusing
/// anything outside `content`, `planet` and the binaries directory.
///
/// The manifest is publisher-supplied text, so the destination it names is
/// *checked against* what this install's shape implies rather than trusted:
/// that is what keeps an update from replacing the install root wholesale and
/// taking `Screenshots/` with it.
fn resolve(layout: &InstallLayout, component: &StagedComponent) -> Result<Replacement, ApplyError> {
    let out_of_scope = |expected: PathBuf| ApplyError::DestinationOutOfScope {
        component: component.name.clone(),
        destination: component.destination.clone(),
        expected,
    };
    let (destination, source) = match component.name.as_str() {
        name @ ("content" | "planet") => {
            let expected = layout.data_relative().join(name);
            (component.destination == expected)
                .then(|| (expected.clone(), PathBuf::new()))
                .ok_or_else(|| out_of_scope(expected))?
        }
        "engine" => {
            // The engine archive already carries `bin/…` or `Contents/…`, so
            // the manifest installs it at the root — but only its binaries
            // directory is actually swapped.
            if !component.destination.as_os_str().is_empty() {
                return Err(out_of_scope(PathBuf::new()));
            }
            let binaries = layout.binaries_relative();
            (binaries.clone(), binaries)
        }
        other => {
            return Err(ApplyError::UnknownComponent {
                component: other.to_string(),
            })
        }
    };
    Ok(Replacement {
        component: component.name.clone(),
        archive: component.archive.clone(),
        sha256: component.sha256.clone(),
        size: component.size,
        destination,
        source,
    })
}

fn validate_journal_layout(layout: &InstallLayout, journal: &Journal) -> Result<(), ApplyError> {
    if journal.schema == crate::journal::JOURNAL_SCHEMA {
        let requested = canonical_install_root(layout)?;
        // The pathname is only a proxy for the install. When both sides expose
        // a filesystem identity it answers the question directly, and a rename
        // stops looking like a different install.
        let identities_agree = matches!(
            (journal.install_identity, InstallIdentity::of(&requested)),
            (Some(recorded), Some(current)) if recorded == current
        );
        if !identities_agree && journal.install_root != requested {
            return Err(ApplyError::InstallRootMismatch {
                recorded: journal.install_root.clone(),
                requested,
            });
        }
    }
    if layout.is_bundle()
        && journal.schema == crate::journal::JOURNAL_SCHEMA
        && journal.previous_bundle_icon_present.is_none()
    {
        return Err(ApplyError::MissingBundleRecoveryState);
    }
    let mut seen = std::collections::BTreeSet::new();
    journal.steps.iter().try_for_each(|step| {
        if !seen.insert(step.component.as_str()) {
            return Err(ApplyError::DuplicateComponent {
                component: step.component.clone(),
            });
        }
        let expected = match step.component.as_str() {
            name @ ("content" | "planet") => layout.data_relative().join(name),
            "engine" => layout.binaries_relative(),
            other => {
                return Err(ApplyError::UnknownComponent {
                    component: other.to_string(),
                });
            }
        };
        let destination = PathBuf::from(&step.destination);
        (destination == expected)
            .then_some(())
            .ok_or_else(|| ApplyError::DestinationOutOfScope {
                component: step.component.clone(),
                destination,
                expected,
            })
    })
}

fn upgrade_legacy_journal(
    layout: &InstallLayout,
    journal: &mut Journal,
    work: &Path,
) -> Result<(), ApplyError> {
    if journal.schema == crate::journal::JOURNAL_SCHEMA {
        return Ok(());
    }
    if layout.is_bundle() {
        return Err(ApplyError::UnsafeLegacyBundleJournal);
    }
    let install_root = canonical_install_root(layout)?;
    if journal.schema == 2 {
        journal.schema = crate::journal::JOURNAL_SCHEMA;
        journal.install_root = install_root;
        journal.save(work)?;
        return Ok(());
    }
    if journal.steps.iter().any(|step| step.sha256.is_empty()) {
        return Err(ApplyError::UnsafeLegacyJournalDigest);
    }

    let previous = InstalledState::load_snapshot(&layout.data_dir())?;
    journal.schema = crate::journal::JOURNAL_SCHEMA;
    journal.install_root = install_root;
    journal.previous_installed_state = Some(
        previous
            .raw()
            .map(|bytes| PreviousInstalledState::Present(bytes.to_vec()))
            .unwrap_or(PreviousInstalledState::Absent),
    );
    let nonce = journal.nonce.clone();
    for step in &mut journal.steps {
        let destination = step.destination_in(layout.root())?;
        let locations = locate(layout, &nonce, &step.component, &destination)?;
        step.destination_existed =
            Some(present(&locations.destination) || present(&locations.backup));
        step.rollback_complete = false;
    }
    journal.save(work)?;
    Ok(())
}

/// Where one step's three trees live.
#[derive(Debug, Clone)]
struct Locations {
    destination: PathBuf,
    staged: PathBuf,
    backup: PathBuf,
}

fn locate(
    layout: &InstallLayout,
    nonce: &str,
    component: &str,
    destination: &Path,
) -> Result<Locations, ApplyError> {
    let staged = suffixed(destination, &format!(".new-{nonce}"))?;
    let backup = match layout.is_bundle() {
        true => layout.quarantine_dir(nonce).join(component),
        false => suffixed(destination, &format!(".old-{nonce}"))?,
    };
    Ok(Locations {
        destination: destination.to_path_buf(),
        staged,
        backup,
    })
}

/// `…/content` plus `.new-<nonce>` — a sibling, so the rename that consumes it
/// stays inside one directory and therefore one filesystem.
fn suffixed(path: &Path, suffix: &str) -> Result<PathBuf, ApplyError> {
    let name = path.file_name().ok_or_else(|| ApplyError::UnnamedPath {
        path: path.to_path_buf(),
    })?;
    let mut suffixed = name.to_os_string();
    suffixed.push(suffix);
    Ok(path.with_file_name(suffixed))
}

fn unpacked_budget(archive_size: u64) -> u64 {
    archive_size
        .saturating_mul(UNPACKED_BUDGET_FACTOR)
        .max(MINIMUM_UNPACKED_BUDGET)
}

/// Whether the path itself exists, without following a symlink to decide.
fn present(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

fn io<'a>(operation: &'static str, path: &'a Path) -> impl Fn(std::io::Error) -> ApplyError + 'a {
    move |source| ApplyError::Io {
        operation,
        path: path.to_path_buf(),
        source,
    }
}

fn canonical_install_root(layout: &InstallLayout) -> Result<PathBuf, ApplyError> {
    std::fs::canonicalize(layout.root()).map_err(io("resolving install root", layout.root()))
}

fn rename(from: &Path, to: &Path) -> Result<(), ApplyError> {
    std::fs::rename(from, to).map_err(|source| ApplyError::Io {
        operation: "renaming",
        path: from.to_path_buf(),
        source,
    })?;
    sync_rename_parents_with(from, to, sync_apply_directory)
}

fn sync_rename_parents_with<F>(from: &Path, to: &Path, mut sync: F) -> Result<(), ApplyError>
where
    F: FnMut(&Path) -> Result<(), ApplyError>,
{
    if to.parent() != from.parent() {
        if let Some(parent) = to.parent() {
            sync(parent)?;
        }
    }
    if let Some(parent) = from.parent() {
        sync(parent)?;
    }
    Ok(())
}

#[cfg(unix)]
fn sync_apply_directory(path: &Path) -> Result<(), ApplyError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(io("syncing", path))
}

#[cfg(not(unix))]
fn sync_apply_directory(_path: &Path) -> Result<(), ApplyError> {
    Ok(())
}

fn ensure_dir(path: &Path) -> Result<(), ApplyError> {
    ensure_dir_with_sync(path, sync_apply_directory)
}

fn ensure_dir_with_sync<F>(path: &Path, mut sync_parent: F) -> Result<(), ApplyError>
where
    F: FnMut(&Path) -> Result<(), ApplyError>,
{
    let mut missing = Vec::new();
    let mut current = path;
    while !present(current) {
        missing.push(current.to_path_buf());
        let Some(parent) = current.parent() else {
            break;
        };
        current = parent;
    }

    std::fs::create_dir_all(path).map_err(io("creating", path))?;
    // Each new directory entry becomes durable before anything is moved under
    // it. Outermost first means a later parent sync is itself named durably.
    for created in missing.iter().rev() {
        if let Some(parent) = created.parent() {
            sync_parent(parent)?;
        }
    }
    Ok(())
}

/// Removes a file, directory or symlink, treating absence as success.
fn remove_any(path: &Path) -> Result<(), ApplyError> {
    match std::fs::symlink_metadata(path) {
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(source) => Err(io("inspecting", path)(source)),
        // A symlink is removed as a link, never followed: `symlink_metadata`
        // reports the link itself, so a link to a directory lands here.
        Ok(metadata) if metadata.is_dir() => {
            std::fs::remove_dir_all(path).map_err(io("removing", path))
        }
        Ok(_) => std::fs::remove_file(path).map_err(io("removing", path)),
    }?;
    if let Some(parent) = path.parent() {
        sync_apply_directory(parent)?;
    }
    Ok(())
}

/// Top-level names the staged archive brings, i.e. what this release owns.
fn staged_top_level_names(locations: &Locations) -> Result<Vec<String>, ApplyError> {
    if !std::fs::symlink_metadata(&locations.staged).is_ok_and(|meta| meta.is_dir()) {
        return Ok(Vec::new());
    }
    let mut names = Vec::new();
    for entry in std::fs::read_dir(&locations.staged).map_err(io("listing", &locations.staged))? {
        let entry = entry.map_err(io("listing", &locations.staged))?;
        let name = entry.file_name();
        let Some(name) = name.to_str().filter(|name| safe_child_name(name)) else {
            continue;
        };
        names.push(name.to_string());
    }
    names.sort();
    Ok(names)
}

/// Top-level names of the old tree the new archive does not contain and the
/// previous release did not own — that is, the user's own packs.
///
/// Users legitimately drop packs into `content/`, which `clonk-app` scans.
/// `planet` and the binaries directory are complete release-owned snapshots:
/// carrying anything there would produce a hybrid installation. The launcher
/// recreates its `System.c4g` and `Graphics.c4g` copies from `planet` before it
/// starts the runtime, so those copies do not need to survive an engine swap.
///
/// `previously_owned` is what the installed release recorded owning here. A
/// name in it that the new archive omits is an official pack that release
/// dropped, and it goes; every other omitted name is the user's and stays.
/// Deleting a user-installed scenario or definition is unrecoverable, so an
/// empty `previously_owned` — a state file written before ownership was
/// recorded — keeps everything, exactly as before.
fn carry_over_names(
    component: &str,
    locations: &Locations,
    previously_owned: &[String],
) -> Result<Vec<String>, ApplyError> {
    if component != "content" {
        return Ok(Vec::new());
    }
    if !std::fs::symlink_metadata(&locations.destination).is_ok_and(|meta| meta.is_dir()) {
        return Ok(Vec::new());
    }
    let mut names = Vec::new();
    for entry in
        std::fs::read_dir(&locations.destination).map_err(io("listing", &locations.destination))?
    {
        let entry = entry.map_err(io("listing", &locations.destination))?;
        let name = entry.file_name();
        // The journal is JSON, so a name it cannot hold is a name a rollback
        // could not put back. Refusing the update is recoverable; carrying the
        // entry without being able to restore it is not.
        let name = name
            .to_str()
            .filter(|name| safe_child_name(name))
            .ok_or_else(|| ApplyError::UnrepresentableEntry {
                path: locations.destination.clone(),
            })?
            .to_string();
        if !present(&locations.staged.join(&name)) && !previously_owned.contains(&name) {
            names.push(name);
        }
    }
    names.sort();
    Ok(names)
}

/// Unpacks every component beside its destination, touching nothing live.
///
/// All of it happens before the first swap, so a corrupt or oversized archive
/// fails while the install is still entirely the old one.
fn stage_components(
    layout: &InstallLayout,
    steps: &[Replacement],
    nonce: &str,
) -> Result<(), ApplyError> {
    let scratch = layout.scratch_dir(nonce);
    for step in steps {
        verify_file(&step.archive, &step.sha256)?;
        let unpacked = scratch.join(&step.component);
        extract_archive(&step.archive, &unpacked, unpacked_budget(step.size))?;

        let source = unpacked.join(&step.source);
        if !source.is_dir() {
            return Err(ApplyError::MissingPayload {
                component: step.component.clone(),
                path: step.source.clone(),
            });
        }
        let locations = locate(
            layout,
            nonce,
            &step.component,
            &layout.root().join(&step.destination),
        )?;
        if present(&locations.staged) {
            return Err(ApplyError::StagingOccupied {
                path: locations.staged,
            });
        }
        rename(&source, &locations.staged)?;
    }
    Ok(())
}

/// Runs the two renames per component, journalling on either side of each.
fn swap_components(
    layout: &InstallLayout,
    steps: &[Replacement],
    journal: &mut Journal,
    work: &Path,
    previous: &crate::state::InstalledState,
    owned: &mut Vec<(String, Vec<String>)>,
) -> Result<(), ApplyError> {
    for (index, step) in steps.iter().enumerate() {
        let locations = locate(
            layout,
            &journal.nonce,
            &step.component,
            &layout.root().join(&step.destination),
        )?;

        // Recorded *before* the moves: a rollback has to know which entries
        // were the user's without being able to re-derive it afterwards.
        let carried = carry_over_names(
            &step.component,
            &locations,
            previous.owned_names(&step.component),
        )?;
        // The staged tree is what this release owns. Recorded here because it
        // is the only point where it exists and has not yet been renamed into
        // place, and the next apply needs it to tell a retired official pack
        // from a user-added one.
        owned.push((step.component.clone(), staged_top_level_names(&locations)?));
        journal.steps[index].carried.clone_from(&carried);
        journal.save(work)?;
        for name in &carried {
            rename(
                &locations.destination.join(name),
                &locations.staged.join(name),
            )?;
        }

        if present(&locations.destination) {
            if let Some(parent) = locations.backup.parent() {
                ensure_dir(parent)?;
            }
            rename(&locations.destination, &locations.backup)?;
        }
        journal.steps[index].state = StepState::BackupMoved;
        journal.save(work)?;

        rename(&locations.staged, &locations.destination)?;
        journal.steps[index].state = StepState::Completed;
        journal.save(work)?;
    }
    Ok(())
}

/// Deletes the launcher's staged copies of `System.c4g` and `Graphics.c4g`.
///
/// `clonk-game`'s `ensure_runtime_asset` links or copies them out of `planet/`
/// at launch, so a copy made from the *old* planet outlives the swap; deleting
/// them is what makes the next launch recreate them. `planet/System.c4g` itself
/// is a different path and is never touched.
fn purge_launcher_staged_groups(layout: &InstallLayout) -> Result<Vec<PathBuf>, ApplyError> {
    let mut purged = Vec::new();
    for directory in [layout.data_dir(), layout.binaries_dir()] {
        for name in LAUNCHER_STAGED_GROUPS {
            let path = directory.join(name);
            if present(&path) {
                remove_any(&path)?;
                purged.push(path);
            }
        }
    }
    Ok(purged)
}

/// Where the displaced icon waits, so a rollback can put it back. Outside the
/// `.app`, like every other transient, so it cannot break the seal.
fn displaced_bundle_icon(layout: &InstallLayout, nonce: &str) -> PathBuf {
    layout.quarantine_dir(nonce).join("ClonkRust.icns")
}

/// Installs the bundle icon the engine component ships.
///
/// Only `Contents/MacOS` is swapped, so the icon the archive also carries was
/// extracted and then thrown away, and an install updated in place kept its
/// original icon for ever. It is the one file in `Contents` worth a file-level
/// overwrite: unlike a stale copyright notice, a stale icon is the thing the
/// user looks at.
///
/// A component that ships no icon leaves the installed one alone — an iconless
/// bundle would be worse than a stale one. The old icon is moved aside rather
/// than overwritten so [`roll_back`] can restore it; the rename that replaces it
/// is within one directory, so no window exists where neither is in place.
fn install_bundle_icon(layout: &InstallLayout, journal: &Journal) -> Result<(), ApplyError> {
    if !journal.steps.iter().any(|step| step.component == "engine") {
        return Ok(());
    }
    let staged = layout
        .scratch_dir(&journal.nonce)
        .join("engine")
        .join(BUNDLE_ICON);
    if !present(&staged) {
        return Ok(());
    }

    let installed = layout.root().join(BUNDLE_ICON);
    if present(&installed) {
        let displaced = displaced_bundle_icon(layout, &journal.nonce);
        if let Some(parent) = displaced.parent() {
            ensure_dir(parent)?;
        }
        rename(&installed, &displaced)?;
    } else if let Some(parent) = installed.parent() {
        ensure_dir(parent)?;
    }
    rename(&staged, &installed)
}

/// Re-seals a bundle, in the order `xtask`'s `sign_macos_bundle` uses.
fn resign_bundle(layout: &InstallLayout, ops: &dyn PlatformOps) -> Result<(), ApplyError> {
    // Nested code first: the bundle's own signature seals it, so signing the
    // bundle before its nested executables would seal the old ones.
    for executable in NESTED_BUNDLE_EXECUTABLES {
        ops.codesign(
            &["--force", "--sign", "-"],
            &layout.binaries_dir().join(executable),
        )?;
    }
    ops.codesign(&["--force", "--sign", "-"], layout.root())?;
    // Proving it validates is the point: an unopenable "damaged" bundle is
    // worse than a stale one, so a failure here rolls the update back.
    ops.codesign(&["--verify", "--deep", "--strict"], layout.root())?;
    Ok(())
}

/// Fallible finalization that must happen before installed state is recorded.
fn prepare_commit(layout: &InstallLayout, journal: &Journal) -> Result<(), ApplyError> {
    if journal.steps.iter().any(|step| step.component == "planet") {
        purge_launcher_staged_groups(layout)?;
    }
    if layout.is_bundle() {
        // Before state is recorded and the bundle is signed, so the new seal
        // covers both changes.
        install_bundle_icon(layout, journal)?;
    }
    Ok(())
}

/// Everything after installed state has become durable.
fn finish_commit(
    layout: &InstallLayout,
    journal: &Journal,
    ops: &dyn PlatformOps,
) -> Result<(), ApplyError> {
    if layout.is_bundle() {
        // Nothing transient is inside the bundle at this point: the staged
        // trees were consumed by their swaps, and the scratch and the backups
        // live in the work directory outside the `.app` precisely so the seal
        // can be computed over a finished tree and stay valid once they go.
        resign_bundle(layout, ops)?;
    }
    // Only now, because until the signature verifies the backups are the only
    // way back — and never fatally, because at this point the update has
    // succeeded and a directory that will not delete is no reason to undo it.
    discard_transients_quietly(layout, journal);
    Ok(())
}

fn restore_installed_state(
    layout: &InstallLayout,
    previous: &InstalledStateSnapshot,
) -> Result<(), ApplyError> {
    previous
        .restore(&layout.data_dir())
        .map_err(ApplyError::from)
}

fn restore_journalled_installed_state(
    layout: &InstallLayout,
    journal: &Journal,
) -> Result<(), ApplyError> {
    match journal.previous_installed_state.as_ref() {
        Some(PreviousInstalledState::Absent) => {
            InstalledState::restore_bytes(&layout.data_dir(), None)?
        }
        Some(PreviousInstalledState::Present(bytes)) => {
            InstalledState::restore_bytes(&layout.data_dir(), Some(bytes))?
        }
        // Schema 1 never included InstalledState in the apply transaction, so
        // there is nothing for its rollback to restore.
        None => {}
    }
    Ok(())
}

/// Removes the staging scratch, the staged trees and the backups.
fn discard_transients(layout: &InstallLayout, journal: &Journal) -> Result<(), ApplyError> {
    for step in &journal.steps {
        let destination = step.destination_in(layout.root())?;
        let locations = locate(layout, &journal.nonce, &step.component, &destination)?;
        remove_any(&locations.staged)?;
        remove_any(&locations.backup)?;
    }
    remove_any(&layout.scratch_dir(&journal.nonce))?;
    remove_any(&layout.quarantine_dir(&journal.nonce))
}

/// [`discard_transients`] where the install is already in its final state, so a
/// leftover directory is untidy rather than a reason to fail.
fn discard_transients_quietly(layout: &InstallLayout, journal: &Journal) {
    if let Err(error) = discard_transients(layout, journal) {
        tracing::warn!(%error, "could not remove the update's temporary trees");
    }
}

/// Puts every step back the way it was, newest first.
///
/// Safe to run against any journal state because it is written in terms of what
/// is on disk: a rename either happened or it did not, and each branch asks.
fn roll_back(layout: &InstallLayout, journal: &mut Journal, work: &Path) -> Result<(), ApplyError> {
    if journal.phase != TransactionPhase::RollingBack {
        journal.phase = TransactionPhase::RollingBack;
        journal.save(work)?;
    }

    for index in (0..journal.steps.len()).rev() {
        let step = journal.steps[index].clone();
        let destination = step.destination_in(layout.root())?;
        let locations = locate(layout, &journal.nonce, &step.component, &destination)?;
        let original_existed =
            step.destination_existed
                .ok_or_else(|| ApplyError::InconsistentRollbackState {
                    component: step.component.clone(),
                })?;

        if step.rollback_complete {
            if present(&locations.backup) {
                if !original_existed {
                    return Err(ApplyError::InconsistentRollbackState {
                        component: step.component,
                    });
                }
                // The durable bit got ahead of the filesystem (or was
                // corrupted). Clear it durably, then run the ordinary
                // idempotent restoration path instead of deleting the backup.
                journal.steps[index].rollback_complete = false;
                journal.save(work)?;
            } else if present(&locations.destination) != original_existed {
                return Err(ApplyError::InconsistentRollbackState {
                    component: step.component,
                });
            } else {
                continue;
            }
        }

        if original_existed && !present(&locations.backup) && !present(&locations.destination) {
            return Err(ApplyError::InconsistentRollbackState {
                component: step.component,
            });
        }

        if present(&locations.backup) {
            // The backup's presence is the durable fact that the first rename
            // ran. The journal may still say `Pending` if power failed between
            // that rename and its following save.
            if present(&locations.destination) {
                remove_any(&locations.staged)?;
                rename(&locations.destination, &locations.staged)?;
            }
            rename(&locations.backup, &locations.destination)?;
        } else if !original_existed
            && step.state != StepState::Pending
            && present(&locations.destination)
            && !present(&locations.staged)
        {
            // A destination that did not exist before the update has no
            // backup. Moving the installed new tree aside restores absence;
            // on a retry, the staged tree proves this move already happened.
            rename(&locations.destination, &locations.staged)?;
        } else if original_existed
            && step.state != StepState::Pending
            && present(&locations.destination)
            && !present(&locations.staged)
        {
            return Err(ApplyError::InconsistentRollbackState {
                component: step.component,
            });
        }

        // The carried entries were moved out of the old tree; put them back.
        for name in &step.carried {
            let from = locations.staged.join(name);
            let to = locations.destination.join(name);
            if present(&from) && !present(&to) {
                if let Some(parent) = to.parent() {
                    ensure_dir(parent)?;
                }
                rename(&from, &to)?;
            }
        }
        journal.steps[index].rollback_complete = true;
        journal.save(work)?;
    }

    // The icon is a file rather than one of the swapped trees, so it needs its
    // own restore: `install_bundle_icon` runs before the bundle is re-signed and
    // therefore has already replaced it by the time a failing seal lands here.
    let displaced = displaced_bundle_icon(layout, &journal.nonce);
    if present(&displaced) {
        let installed = layout.root().join(BUNDLE_ICON);
        remove_any(&installed)?;
        rename(&displaced, &installed)?;
    } else if journal.previous_bundle_icon_present == Some(false) {
        remove_any(&layout.root().join(BUNDLE_ICON))?;
    }
    Ok(())
}

/// Installs a downloaded release.
///
/// The plan's archives must already be on disk; nothing here touches the
/// network. Every failure path rolls the install back to what it was.
///
/// Free space is *not* re-checked here — [`ensure_free_space`] runs before the
/// download, which is the only point at which refusing costs the user nothing.
/// Nor does this wait for the processes holding the install open; the applier
/// runs as its own process and calls [`PlatformOps::wait_for_process`] first.
pub fn apply_update(
    layout: &InstallLayout,
    plan: &ApplyPlan,
    ops: &dyn PlatformOps,
) -> Result<ApplyOutcome, ApplyError> {
    apply_update_with_stager(layout, plan, ops, stage_components)
}

fn apply_update_with_stager<F>(
    layout: &InstallLayout,
    plan: &ApplyPlan,
    ops: &dyn PlatformOps,
    stager: F,
) -> Result<ApplyOutcome, ApplyError>
where
    F: FnOnce(&InstallLayout, &[Replacement], &str) -> Result<(), ApplyError>,
{
    // Resolved first and in full: a manifest naming a component or a
    // destination this build will not install must be refused while the tree is
    // still untouched.
    let mut steps = plan
        .components
        .iter()
        .map(|component| resolve(layout, component))
        .collect::<Result<Vec<_>, _>>()?;
    steps.sort_by_key(|step| apply_rank(&step.component));

    let outcome = ApplyOutcome {
        version: plan.version.clone(),
        applied: steps.iter().map(|step| step.component.clone()).collect(),
    };
    if steps.is_empty() {
        return Ok(outcome);
    }

    let work = layout.work_dir();
    ensure_dir(&work)?;
    let _lock = UpdateLock::acquire(&work)?;
    if Journal::load(&work)?.is_some() {
        return Err(ApplyError::UpdateInProgress {
            path: Journal::path_in(&work),
        });
    }

    // Read before touching the live tree. Besides preserving components this
    // plan does not replace, this makes an unreadable state file a harmless
    // refusal rather than a failure discovered halfway through the swap.
    let previous_state = InstalledState::load_snapshot(&layout.data_dir())?;
    let mut installed_state = previous_state.state.clone().unwrap_or_default();
    steps.iter().for_each(|step| {
        installed_state.record(&step.component, &plan.version, &step.sha256);
    });

    let nonce = new_nonce();
    let mut journal = Journal::new(
        &plan.version,
        &nonce,
        canonical_install_root(layout)?,
        steps
            .iter()
            .map(|step| {
                let mut journal_step = JournalStep::new(
                    &step.component,
                    &step.sha256,
                    &journal_destination(&step.destination),
                );
                journal_step.destination_existed =
                    Some(present(&layout.root().join(&step.destination)));
                journal_step
            })
            .collect(),
    );
    journal.previous_installed_state = Some(
        previous_state
            .raw()
            .map(|bytes| PreviousInstalledState::Present(bytes.to_vec()))
            .unwrap_or(PreviousInstalledState::Absent),
    );
    journal.previous_bundle_icon_present = layout
        .is_bundle()
        .then(|| present(&layout.root().join(BUNDLE_ICON)));

    journal.save(&work)?;
    // Durable before the first tree is touched, and before the bundle can be
    // moved out from under the sidecar this names.
    remember_bundle_transaction(layout);
    if let Err(cause) = stager(layout, &steps, &nonce) {
        // The pending journal was durable before staging began. Remove it only
        // after every partial tree is gone; otherwise startup recovery retains
        // the exact nonce it needs to finish that cleanup.
        let cleaned = discard_transients(layout, &journal)
            .and_then(|()| finish_transaction(layout, &work).map_err(ApplyError::from));
        return Err(match cleaned {
            Ok(()) => cause,
            Err(failure) => ApplyError::RollbackFailed {
                cause: cause.to_string(),
                failure: failure.to_string(),
            },
        });
    }

    let mut state_write_attempted = false;
    let mut signing_attempted = false;
    let previous_for_ownership = previous_state.state.clone().unwrap_or_default();
    let applied = (|| {
        let mut owned = Vec::new();
        swap_components(
            layout,
            &steps,
            &mut journal,
            &work,
            &previous_for_ownership,
            &mut owned,
        )?;
        for (component, names) in owned {
            installed_state.set_owned_names(&component, names);
        }
        prepare_commit(layout, &journal)?;
        state_write_attempted = true;
        installed_state.save_preserving_unknown(&layout.data_dir(), previous_state.raw())?;
        signing_attempted = layout.is_bundle();
        finish_commit(layout, &journal, ops)
    })();
    if let Err(cause) = applied {
        return Err(match roll_back(layout, &mut journal, &work) {
            Ok(()) => {
                let restored: Result<(), ApplyError> = (|| {
                    if state_write_attempted {
                        restore_installed_state(layout, &previous_state)?;
                    }
                    // Rollback parks the rejected new trees at `.new-*` paths
                    // beside their restored destinations. They must be gone
                    // before codesign seals the old bundle again.
                    discard_transients(layout, &journal)?;
                    // `codesign --force` mutates signatures in place. Restoring
                    // the old trees and exact state bytes is not enough to make
                    // their old seal valid again, especially for a content-only
                    // update whose executable directory had no swap backup.
                    if signing_attempted {
                        resign_bundle(layout, ops)?;
                    }
                    finish_transaction(layout, &work)?;
                    Ok(())
                })();
                match restored {
                    Ok(()) => cause,
                    Err(failure) => ApplyError::RollbackFailed {
                        cause: cause.to_string(),
                        failure: failure.to_string(),
                    },
                }
            }
            Err(failure) => ApplyError::RollbackFailed {
                cause: cause.to_string(),
                failure: failure.to_string(),
            },
        });
    }

    finish_transaction(layout, &work)?;
    // Best effort by design: a software-inventory entry that lags by one
    // release is not worth undoing an update the user asked for.
    if let Err(error) = ops.set_installed_version(&plan.version) {
        tracing::warn!(%error, "could not record the installed version with the platform");
    }
    Ok(outcome)
}

/// The `/`-separated form the journal records.
fn journal_destination(relative: &Path) -> String {
    relative
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .collect::<Vec<_>>()
        .join("/")
}

fn new_nonce() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    format!("{:x}-{:x}", std::process::id(), nanos)
}

/// Finishes or undoes an apply that a crash interrupted.
///
/// `install_root` may be either a macOS `.app` or the install root inside it;
/// [`InstallLayout::discover`] accepts both, because looking in the wrong place
/// would report that nothing was interrupted.
pub fn resume_interrupted_update(install_root: &Path) -> Result<ResumeOutcome, ApplyError> {
    resume_interrupted_update_with(&InstallLayout::discover(install_root), &RealPlatform)
}

/// [`resume_interrupted_update`] against an explicit layout and host.
///
/// A schema-2 journal records the recovery direction before rollback touches a
/// tree, so every retry continues that rollback. A legacy journal is first
/// upgraded with the missing recovery state; one without component digests, or
/// one for a macOS bundle, is rejected before recovery mutates the install.
/// Where this install's in-flight transaction actually lives.
///
/// Normally its own [`InstallLayout::work_dir`]. A bundle keeps its sidecar
/// beside the `.app` under the bundle's *name*, so renaming an interrupted
/// bundle leaves the sidecar under the old one; the rename is invisible to the
/// name-derived path but not to the recorded identity, so the sibling namespace
/// is searched for the journal this install owns.
///
/// A bundle moved to a different parent directory is out of reach of this:
/// its sidecar stays in the namespace beside where it used to be, and nothing
/// short of a filesystem-wide search or a separate registry would find it.
fn transaction_dir_for(layout: &InstallLayout) -> Result<PathBuf, ApplyError> {
    let own = layout.work_dir();
    if !layout.is_bundle() || Journal::load(&own)?.is_some() {
        return Ok(own);
    }
    let Ok(root) = canonical_install_root(layout) else {
        return Ok(own);
    };
    let Some(identity) = InstallIdentity::of(&root) else {
        return Ok(own);
    };
    let found = sibling_namespace_transaction(&own, identity)
        .or_else(|| registered_transaction(layout, identity));
    let Some(found) = found else {
        return Ok(own);
    };
    // Move it into this bundle's own namespace rather than working out of the
    // old one: every staged, backup and quarantine path is derived from
    // `work_dir()`, so a transaction read from somewhere else would look for
    // its backups under the new name and find nothing.
    if own.exists() {
        std::fs::remove_dir(&own).map_err(io("clearing the transaction directory", &own))?;
    } else if let Some(namespace) = own.parent() {
        // A bundle moved to a directory that has never hosted an update has no
        // namespace of its own yet, and a rename cannot create one.
        ensure_dir(namespace)?;
    }
    rename(&found, &own)?;
    // The transaction now lives where this install derives it, and the entry
    // that led here names a directory that no longer exists.
    remember_bundle_transaction(layout);
    Ok(own)
}

/// The transaction beside where this bundle sits, which is where a *renamed*
/// bundle's own sidecar was left.
fn sibling_namespace_transaction(own: &Path, identity: InstallIdentity) -> Option<PathBuf> {
    let entries = std::fs::read_dir(own.parent()?).ok()?;
    let mut adopted: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|candidate| candidate != own)
        .filter(|candidate| {
            matches!(
                Journal::load(candidate),
                Ok(Some(journal)) if journal.install_identity == Some(identity)
            )
        })
        .collect();
    // Deterministic when more than one names this install, which only a
    // hand-edited namespace can produce.
    adopted.sort();
    adopted.into_iter().next()
}

/// The transaction this install recorded before it moved, which is the only way
/// back to a sidecar left in a directory the bundle no longer sits in.
///
/// The pointer alone never grants adoption: the journal it names is read and
/// checked against this install's identity exactly as a sibling candidate is,
/// so a stale entry — or one aimed by hand — reaches nothing.
fn registered_transaction(layout: &InstallLayout, identity: InstallIdentity) -> Option<PathBuf> {
    let registry = layout.recovery_registry()?;
    let recorded = recovery_registry::locate(&registry, identity)?;
    match Journal::load(&recorded) {
        Ok(Some(journal)) if journal.install_identity == Some(identity) => Some(recorded),
        // The transaction it named is finished or gone; an entry that leads
        // nowhere is only a slower way of finding nothing next time.
        _ => {
            recovery_registry::forget(&registry, identity);
            None
        }
    }
}

/// Records where this bundle's transaction is waiting, so recovery can find it
/// again after a move that leaves the sidecar behind.
///
/// Best effort by design: the registry only widens what recovery reaches, so a
/// registry that cannot be written costs the moved-bundle case and nothing
/// else, and must never fail an update that is otherwise fine.
fn remember_bundle_transaction(layout: &InstallLayout) {
    if !layout.is_bundle() {
        return;
    }
    let Ok(root) = canonical_install_root(layout) else {
        return;
    };
    let (Some(identity), Some(registry)) = (InstallIdentity::of(&root), layout.recovery_registry())
    else {
        return;
    };
    if let Err(source) = recovery_registry::record(&registry, identity, &layout.work_dir(), &root) {
        tracing::warn!(
            ?source,
            registry = %registry.display(),
            "could not record where this install's update transaction is waiting"
        );
    }
}

/// Declares the transaction over: removes the journal, then the entry pointing
/// at it.
///
/// In that order, because the journal is the authority. An entry outliving its
/// journal is read as absent and dropped on the next lookup; a journal outliving
/// its entry is still found wherever this install derives it.
fn finish_transaction(layout: &InstallLayout, work: &Path) -> Result<(), JournalError> {
    Journal::remove(work)?;
    forget_bundle_transaction(layout);
    Ok(())
}

/// Drops this bundle's registry entry, which finishing a transaction means.
fn forget_bundle_transaction(layout: &InstallLayout) {
    if !layout.is_bundle() {
        return;
    }
    let Ok(root) = canonical_install_root(layout) else {
        return;
    };
    let (Some(identity), Some(registry)) = (InstallIdentity::of(&root), layout.recovery_registry())
    else {
        return;
    };
    recovery_registry::forget(&registry, identity);
}

pub fn resume_interrupted_update_with(
    layout: &InstallLayout,
    ops: &dyn PlatformOps,
) -> Result<ResumeOutcome, ApplyError> {
    let work = transaction_dir_for(layout)?;
    let journal_present = Journal::load(&work)?.is_some();
    if let Err(error) = ensure_dir(&work) {
        if !journal_present
            && matches!(
                &error,
                ApplyError::Io { source, .. }
                    if matches!(
                        source.kind(),
                        std::io::ErrorKind::PermissionDenied
                            | std::io::ErrorKind::ReadOnlyFilesystem
                    )
            )
        {
            return Ok(ResumeOutcome::NothingToDo);
        }
        return Err(error);
    }
    // Opening/creating the lock is the atomic operation: an existence check
    // followed by an early return has a race with an applier creating it. A
    // pristine read-only install can still take the ordinary no-journal path.
    let _lock = match UpdateLock::acquire(&work) {
        Ok(lock) => lock,
        Err(ApplyError::Io { source, .. })
            if !journal_present
                && matches!(
                    source.kind(),
                    std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::ReadOnlyFilesystem
                ) =>
        {
            return Ok(ResumeOutcome::NothingToDo);
        }
        Err(error) => return Err(error),
    };
    let Some(mut journal) = Journal::load(&work)? else {
        return Ok(ResumeOutcome::NothingToDo);
    };
    // A journal recorded against a different install belongs to whatever was
    // at this path before. Leaving it untouched is the whole point: adopting
    // it would restore a stranger's backup over this install.
    if let (Some(recorded), Some(current)) = (
        journal.install_identity,
        InstallIdentity::of(&canonical_install_root(layout)?),
    ) {
        if recorded != current {
            return Ok(ResumeOutcome::NothingToDo);
        }
    }
    validate_journal_layout(layout, &journal)?;
    upgrade_legacy_journal(layout, &mut journal, &work)?;
    validate_journal_layout(layout, &journal)?;
    let version = journal.version.clone();

    if journal.phase == TransactionPhase::RollingBack || !journal.any_step_completed() {
        roll_back(layout, &mut journal, &work)?;
        restore_journalled_installed_state(layout, &journal)?;
        discard_transients(layout, &journal)?;
        if layout.is_bundle() {
            resign_bundle(layout, ops)?;
        }
        finish_transaction(layout, &work)?;
        return Ok(ResumeOutcome::RolledBack { version });
    }

    let previous_state = InstalledState::load_snapshot(&layout.data_dir())?;
    let mut installed_state = previous_state.state.clone().unwrap_or_default();
    journal.steps.iter().for_each(|step| {
        installed_state.record(&step.component, &journal.version, &step.sha256);
    });
    roll_forward(layout, &mut journal, &work)?;
    prepare_commit(layout, &journal)?;
    installed_state.save_preserving_unknown(&layout.data_dir(), previous_state.raw())?;
    finish_commit(layout, &journal, ops)?;
    finish_transaction(layout, &work)?;
    Ok(ResumeOutcome::RolledForward { version })
}

/// Drives every step to `Completed`, journalling as it goes.
fn roll_forward(
    layout: &InstallLayout,
    journal: &mut Journal,
    work: &Path,
) -> Result<(), ApplyError> {
    let nonce = journal.nonce.clone();
    for index in 0..journal.steps.len() {
        let step = journal.steps[index].clone();
        let destination = step.destination_in(layout.root())?;
        let locations = locate(layout, &nonce, &step.component, &destination)?;
        let lost = || ApplyError::StagingLost {
            component: step.component.clone(),
        };

        match step.state {
            StepState::Completed => continue,
            StepState::BackupMoved => {
                if !present(&locations.destination) {
                    // Only the staged tree contains the new component. The
                    // backup is the rollback source and must never be accepted
                    // as a successful forward install.
                    present(&locations.staged).then_some(()).ok_or_else(lost)?;
                    rename(&locations.staged, &locations.destination)?;
                }
            }
            StepState::Pending => {
                if !present(&locations.staged) {
                    return Err(lost());
                }
                // Idempotent: only entries still in the old tree move.
                for name in &step.carried {
                    let from = locations.destination.join(name);
                    let to = locations.staged.join(name);
                    if present(&from) && !present(&to) {
                        rename(&from, &to)?;
                    }
                }
                if present(&locations.destination) {
                    if let Some(parent) = locations.backup.parent() {
                        ensure_dir(parent)?;
                    }
                    rename(&locations.destination, &locations.backup)?;
                }
                journal.steps[index].state = StepState::BackupMoved;
                journal.save(work)?;
                rename(&locations.staged, &locations.destination)?;
            }
        }
        journal.steps[index].state = StepState::Completed;
        journal.save(work)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};
    use tempfile::TempDir;
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    fn write_file(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent");
        }
        std::fs::write(path, contents).expect("write file");
    }

    fn read_file(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap_or_else(|error| panic!("read {path:?}: {error}"))
    }

    /// Every file under `root`, as `relative path -> contents`.
    fn snapshot(root: &Path) -> Vec<(String, String)> {
        fn walk(root: &Path, at: &Path, into: &mut Vec<(String, String)>) {
            let Ok(listing) = std::fs::read_dir(at) else {
                return;
            };
            for entry in listing.flatten() {
                let path = entry.path();
                let relative = path
                    .strip_prefix(root)
                    .map(|relative| relative.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_default();
                if relative == UPDATE_LOCK_FILE_NAME {
                    continue;
                }
                if path.is_dir() {
                    into.push((format!("{relative}/"), String::new()));
                    walk(root, &path, into);
                } else {
                    into.push((relative, read_file(&path)));
                }
            }
        }
        let mut entries = Vec::new();
        walk(root, root, &mut entries);
        entries.sort();
        entries
    }

    fn archive(directory: &Path, name: &str, entries: &[(&str, &str)]) -> PathBuf {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        for (entry, contents) in entries {
            writer
                .start_file(*entry, SimpleFileOptions::default())
                .expect("start file");
            writer
                .write_all(contents.as_bytes())
                .expect("write archive entry");
        }
        let bytes = writer.finish().expect("finish archive").into_inner();
        let path = directory.join(name);
        std::fs::write(&path, bytes).expect("write archive");
        path
    }

    fn component(name: &str, archive: &Path, destination: &str) -> StagedComponent {
        StagedComponent {
            name: name.to_string(),
            archive: archive.to_path_buf(),
            sha256: crate::digest::sha256_file(archive).expect("digest"),
            size: std::fs::metadata(archive).expect("metadata").len(),
            destination: destination
                .split('/')
                .filter(|segment| !segment.is_empty())
                .collect(),
        }
    }

    /// A plain install carrying exactly the things a whole-tree replace would
    /// destroy: a user's own pack, screenshots, recordings, a launcher log and
    /// the launcher-staged groups.
    struct Install {
        _directory: TempDir,
        layout: InstallLayout,
        downloads: PathBuf,
    }

    impl Install {
        fn root(&self) -> &Path {
            self.layout.root()
        }

        fn canonical_root(&self) -> PathBuf {
            std::fs::canonicalize(self.root()).expect("canonical install root")
        }

        fn content_archive(&self) -> PathBuf {
            archive(
                &self.downloads,
                "content.zip",
                &[("Worlds.c4f/Info.txt", "new world")],
            )
        }

        fn planet_archive(&self) -> PathBuf {
            archive(
                &self.downloads,
                "planet.zip",
                &[("System.c4g/Rank.txt", "new rank")],
            )
        }

        fn engine_archive(&self) -> PathBuf {
            let prefix = match self.layout.is_bundle() {
                true => "Contents/MacOS",
                false => "bin",
            };
            archive(
                &self.downloads,
                "engine.zip",
                &[
                    (&format!("{prefix}/clonk-app"), "new app"),
                    (&format!("{prefix}/clonk-game"), "new launcher"),
                    (&format!("{prefix}/c4group"), "new c4group"),
                    ("COPYING", "licence"),
                    (BUNDLE_ICON, "new icon"),
                ],
            )
        }
    }

    fn install_with(bundle: bool) -> Install {
        let directory = TempDir::new().expect("directory");
        let layout = match bundle {
            true => InstallLayout::macos_bundle(directory.path().join("Clonk Rust.app")),
            false => InstallLayout::plain(directory.path().join("install")),
        };
        let root = layout.root().to_path_buf();
        let data = layout.data_dir();
        let binaries = layout.binaries_dir();

        write_file(&data.join("content/Worlds.c4f/Info.txt"), "old world");
        write_file(&data.join("content/MyPack.c4f/Scenario.txt"), "user pack");
        write_file(&data.join("planet/System.c4g/Rank.txt"), "old rank");
        write_file(&binaries.join("clonk-app"), "old app");
        write_file(&binaries.join("clonk-game"), "old launcher");
        write_file(&binaries.join("c4group"), "old c4group");
        // Launcher-staged copies, in both places `ensure_runtime_assets` puts
        // them.
        write_file(&data.join("System.c4g/Rank.txt"), "staged rank");
        write_file(&data.join("Graphics.c4g/Font.png"), "staged font");
        write_file(&binaries.join("System.c4g/Rank.txt"), "staged rank");
        // User data the install tree also holds.
        write_file(&data.join("Screenshots/shot.png"), "screenshot");
        write_file(&data.join("Records.c4f/run.c4v"), "recording");
        write_file(&data.join("Clonk-rust-2026-07-28.log"), "launcher log");
        if bundle {
            write_file(&root.join(BUNDLE_ICON), "old icon");
        }

        let downloads = directory.path().join("downloads");
        std::fs::create_dir_all(&downloads).expect("create downloads");
        let _ = root;
        Install {
            _directory: directory,
            layout,
            downloads,
        }
    }

    fn install() -> Install {
        install_with(false)
    }

    fn plan(version: &str, components: Vec<StagedComponent>) -> ApplyPlan {
        ApplyPlan {
            version: version.to_string(),
            components,
        }
    }

    #[test]
    fn applying_content_replaces_the_tree_and_leaves_user_data_alone() {
        // The install tree is not read-only: screenshots, recordings and
        // launcher logs live in it, and replacing the whole thing would be the
        // simplest possible implementation and would delete all of them.
        let install = install();
        let plan = plan(
            "0.4.0",
            vec![component("content", &install.content_archive(), "content")],
        );

        let outcome =
            apply_update(&install.layout, &plan, &FakePlatform::new()).expect("apply the update");
        assert_eq!(outcome.applied, ["content"]);

        let data = install.layout.data_dir();
        assert_eq!(
            read_file(&data.join("content/Worlds.c4f/Info.txt")),
            "new world"
        );
        assert_eq!(read_file(&data.join("Screenshots/shot.png")), "screenshot");
        assert_eq!(read_file(&data.join("Records.c4f/run.c4v")), "recording");
        assert_eq!(
            read_file(&data.join("Clonk-rust-2026-07-28.log")),
            "launcher log"
        );
        // Untouched components stay exactly as they were.
        assert_eq!(
            read_file(&data.join("planet/System.c4g/Rank.txt")),
            "old rank"
        );
        assert_eq!(
            read_file(&install.layout.binaries_dir().join("clonk-app")),
            "old app"
        );
    }

    /// Ownership is what separates the two kinds of omitted entry. Before it
    /// was recorded the applier had to keep both, so a pack a later release
    /// retired stayed on disk as hybrid content — and it could not simply
    /// delete omitted entries instead, because that deletes user-installed
    /// scenarios and definitions, which is unrecoverable.
    #[test]
    fn a_release_that_drops_an_official_pack_removes_it_and_keeps_the_user_s() {
        let install = install();

        // First release owns two packs.
        let first = archive(
            &install.downloads,
            "content-first.zip",
            &[
                ("Worlds.c4f/Info.txt", "official world"),
                ("Retired.c4f/Info.txt", "official pack this release ships"),
            ],
        );
        apply_update(
            &install.layout,
            &plan("0.4.0", vec![component("content", &first, "content")]),
            &FakePlatform::new(),
        )
        .expect("apply the first release");

        let data = install.layout.data_dir();
        // The player installs their own pack beside them.
        std::fs::create_dir_all(data.join("content/MyPack.c4f")).expect("user pack directory");
        std::fs::write(data.join("content/MyPack.c4f/Info.txt"), "user pack")
            .expect("user pack contents");

        let recorded = InstalledState::load(&data)
            .expect("load installed state")
            .expect("first apply records state");
        assert_eq!(
            recorded.owned_names("content"),
            ["Retired.c4f".to_string(), "Worlds.c4f".to_string()],
            "the first release records both of its packs and not the user's"
        );

        // Second release drops `Retired.c4f`.
        let second = archive(
            &install.downloads,
            "content-second.zip",
            &[("Worlds.c4f/Info.txt", "official world, updated")],
        );
        apply_update(
            &install.layout,
            &plan("0.5.0", vec![component("content", &second, "content")]),
            &FakePlatform::new(),
        )
        .expect("apply the second release");

        assert_eq!(
            read_file(&data.join("content/Worlds.c4f/Info.txt")),
            "official world, updated"
        );
        assert!(
            !data.join("content/Retired.c4f").exists(),
            "the pack this release dropped is gone"
        );
        assert_eq!(
            read_file(&data.join("content/MyPack.c4f/Info.txt")),
            "user pack",
            "the user's own pack survives"
        );

        let after = InstalledState::load(&data)
            .expect("load installed state")
            .expect("second apply records state");
        assert_eq!(
            after.owned_names("content"),
            ["Worlds.c4f".to_string()],
            "ownership now reflects what this release ships"
        );
    }

    #[test]
    fn a_successful_apply_records_the_component_release_and_digest() {
        let install = install();
        let archive = install.content_archive();
        let component = component("content", &archive, "content");
        let digest = component.sha256.clone();
        let plan = plan("0.4.0", vec![component]);

        apply_update(&install.layout, &plan, &FakePlatform::new()).expect("apply the update");

        let state = InstalledState::load(&install.layout.data_dir())
            .expect("load installed state")
            .expect("successful apply records installed state");
        let content = state.component("content").expect("content is recorded");
        assert_eq!(content.version, "0.4.0");
        assert_eq!(content.sha256, digest);
        // The apply also records what the archive owned, so the next one can
        // tell a pack this release drops from a pack the user added.
        assert!(
            content.owned_names.contains(&"Worlds.c4f".to_string()),
            "recorded ownership covers the archive's packs, got {:?}",
            content.owned_names
        );
    }

    #[test]
    fn a_successful_apply_preserves_unknown_installed_state_fields() {
        let install = install();
        let previous = format!(
            "{{\n  \"future_field\": true,\n  \"components\": {{\n    \"content\": {{ \"sha256\": \"{}\", \"version\": \"0.3.0\", \"future_component_field\": 7 }}\n  }},\n  \"schema\": 1\n}}\n",
            "cc".repeat(32)
        );
        std::fs::write(
            InstalledState::path_in(&install.layout.data_dir()),
            previous,
        )
        .expect("write previous state");
        let plan = plan(
            "0.4.0",
            vec![component("content", &install.content_archive(), "content")],
        );

        apply_update(&install.layout, &plan, &FakePlatform::new()).expect("apply");

        let written: serde_json::Value = serde_json::from_slice(
            &std::fs::read(InstalledState::path_in(&install.layout.data_dir()))
                .expect("read state"),
        )
        .expect("parse state");
        assert_eq!(written["future_field"], true);
        assert_eq!(
            written["components"]["content"]["future_component_field"],
            7
        );
        assert_eq!(written["components"]["content"]["version"], "0.4.0");
    }

    #[test]
    fn apply_carries_over_a_user_added_pack_the_new_archive_does_not_contain() {
        // `clonk-app` scans `content/` for packs, so users drop their own in
        // there. A release archive cannot contain them, and installing one must
        // not take them away.
        let install = install();
        let plan = plan(
            "0.4.0",
            vec![component("content", &install.content_archive(), "content")],
        );

        apply_update(&install.layout, &plan, &FakePlatform::new()).expect("apply the update");

        let content = install.layout.data_dir().join("content");
        assert_eq!(
            read_file(&content.join("MyPack.c4f/Scenario.txt")),
            "user pack"
        );
        assert_eq!(read_file(&content.join("Worlds.c4f/Info.txt")), "new world");
    }

    #[test]
    fn applying_planet_purges_the_launcher_staged_groups() {
        // `ensure_runtime_asset` copies `planet/System.c4g` beside the
        // executables on any platform that cannot link it, so a copy of the
        // *old* planet outlives the swap. Deleting them is what makes the next
        // launch stage the new ones.
        let install = install();
        let plan = plan(
            "0.4.0",
            vec![component("planet", &install.planet_archive(), "planet")],
        );

        apply_update(&install.layout, &plan, &FakePlatform::new()).expect("apply the update");

        let data = install.layout.data_dir();
        assert_eq!(
            read_file(&data.join("planet/System.c4g/Rank.txt")),
            "new rank"
        );
        assert!(!data.join("System.c4g").exists());
        assert!(!data.join("Graphics.c4g").exists());
        assert!(!install.layout.binaries_dir().join("System.c4g").exists());
    }

    #[test]
    fn applying_planet_drops_a_group_the_new_snapshot_omits() {
        let install = install();
        let planet = install.layout.data_dir().join("planet");
        write_file(&planet.join("Obsolete.c4g/DefCore.txt"), "old group");
        let plan = plan(
            "0.4.0",
            vec![component("planet", &install.planet_archive(), "planet")],
        );

        apply_update(&install.layout, &plan, &FakePlatform::new()).expect("apply the update");

        assert!(
            !planet.join("Obsolete.c4g").exists(),
            "planet is a complete release snapshot, not an overlay"
        );
        assert_eq!(read_file(&planet.join("System.c4g/Rank.txt")), "new rank");
    }

    #[test]
    fn the_engine_component_replaces_only_the_binaries_directory() {
        // The engine archive installs at the root, but only `bin` is swapped:
        // renaming the root would take the whole install with it.
        let install = install();
        let plan = plan(
            "0.4.0",
            vec![component("engine", &install.engine_archive(), "")],
        );

        apply_update(&install.layout, &plan, &FakePlatform::new()).expect("apply the update");

        let binaries = install.layout.binaries_dir();
        assert_eq!(read_file(&binaries.join("clonk-app")), "new app");
        assert_eq!(read_file(&binaries.join("clonk-game")), "new launcher");
        assert_eq!(read_file(&binaries.join("c4group")), "new c4group");
        assert!(
            !binaries.join("System.c4g").exists(),
            "the launcher recreates its staged group from planet"
        );
        // Everything outside `bin` is untouched, including the top-level
        // documents the engine component also ships.
        let data = install.layout.data_dir();
        assert_eq!(
            read_file(&data.join("content/Worlds.c4f/Info.txt")),
            "old world"
        );
        assert_eq!(read_file(&data.join("Screenshots/shot.png")), "screenshot");
        assert!(!data.join("COPYING").exists());
    }

    #[test]
    fn applying_engine_drops_binaries_and_staged_groups_the_snapshot_omits() {
        let install = install();
        let binaries = install.layout.binaries_dir();
        write_file(&binaries.join("obsolete-helper"), "old helper");
        write_file(&binaries.join("Graphics.c4g/Font.png"), "staged font");
        let plan = plan(
            "0.4.0",
            vec![component("engine", &install.engine_archive(), "")],
        );

        apply_update(&install.layout, &plan, &FakePlatform::new()).expect("apply the update");

        assert!(
            !binaries.join("obsolete-helper").exists(),
            "the binaries directory is a complete release snapshot"
        );
        for staged_group in LAUNCHER_STAGED_GROUPS {
            assert!(
                !binaries.join(staged_group).exists(),
                "the installed launcher regenerates {staged_group} from planet"
            );
        }
    }

    #[test]
    fn components_are_applied_data_first_and_the_engine_last() {
        // An interrupted apply must leave the *old* binaries able to recover,
        // never a new binary beside stale data — so the order is fixed here
        // rather than taken from the manifest.
        let install = install();
        let plan = plan(
            "0.4.0",
            vec![
                component("engine", &install.engine_archive(), ""),
                component("planet", &install.planet_archive(), "planet"),
                component("content", &install.content_archive(), "content"),
            ],
        );

        let outcome =
            apply_update(&install.layout, &plan, &FakePlatform::new()).expect("apply the update");
        assert_eq!(outcome.applied, ["content", "planet", "engine"]);
    }

    #[test]
    fn a_finished_apply_leaves_no_journal_staging_or_backup_behind() {
        let install = install();
        let plan = plan(
            "0.4.0",
            vec![
                component("content", &install.content_archive(), "content"),
                component("planet", &install.planet_archive(), "planet"),
                component("engine", &install.engine_archive(), ""),
            ],
        );

        apply_update(&install.layout, &plan, &FakePlatform::new()).expect("apply the update");

        let leftovers: Vec<_> = std::fs::read_dir(install.root())
            .expect("list the install root")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .filter(|name| {
                name.starts_with("clonk-update") || name.contains(".new-") || name.contains(".old-")
            })
            .collect();
        assert!(leftovers.is_empty(), "left behind: {leftovers:?}");
        assert_eq!(
            Journal::load(&install.layout.work_dir()).expect("load"),
            None
        );
    }

    #[test]
    fn a_second_applier_is_refused_while_the_install_is_locked() {
        let install = install();
        let _first = UpdateLock::acquire(&install.layout.work_dir()).expect("take first lock");
        let before = snapshot(install.root());
        let plan = plan(
            "0.4.0",
            vec![component("content", &install.content_archive(), "content")],
        );

        let error = apply_update(&install.layout, &plan, &FakePlatform::new())
            .expect_err("a concurrent applier must be refused");

        assert!(
            matches!(error, ApplyError::UpdateInProgress { .. }),
            "{error}"
        );
        assert_eq!(snapshot(install.root()), before);
    }

    #[test]
    fn a_live_install_lease_blocks_component_mutation() {
        let install = install();
        let lease = acquire_install_use(&install.layout).expect("hold live install lease");
        let before = snapshot(install.root());
        let plan = plan(
            "0.4.0",
            vec![component("content", &install.content_archive(), "content")],
        );

        let error = apply_update(&install.layout, &plan, &FakePlatform::new())
            .expect_err("a running instance must exclude the updater");

        assert!(matches!(error, ApplyError::UpdateInProgress { .. }));
        assert_eq!(snapshot(install.root()), before);
        drop(lease);
        apply_update(&install.layout, &plan, &FakePlatform::new())
            .expect("the update proceeds after the live instance exits");
    }

    #[test]
    fn a_new_apply_never_overwrites_an_interrupted_update_journal() {
        let install = install();
        let existing = Journal::new("0.3.0", "interrupted", install.canonical_root(), Vec::new());
        existing
            .save(&install.layout.work_dir())
            .expect("save interrupted journal");
        drop(UpdateLock::acquire(&install.layout.work_dir()).expect("seed lock file"));
        let before = snapshot(install.root());
        let plan = plan(
            "0.4.0",
            vec![component("content", &install.content_archive(), "content")],
        );

        let error = apply_update(&install.layout, &plan, &FakePlatform::new())
            .expect_err("the interrupted update must be recovered first");

        assert!(
            matches!(error, ApplyError::UpdateInProgress { .. }),
            "{error}"
        );
        assert_eq!(snapshot(install.root()), before);
        assert_eq!(
            Journal::load(&install.layout.work_dir()).expect("load journal"),
            Some(existing)
        );
    }

    #[test]
    fn an_unknown_component_is_refused_before_anything_moves() {
        // A manifest is publisher text. A component this build does not know
        // where to put has no safe default, and guessing is how an updater
        // deletes a directory it was never meant to touch.
        let install = install();
        let before = snapshot(install.root());
        let plan = plan(
            "0.4.0",
            vec![component(
                "savegames",
                &install.content_archive(),
                "content",
            )],
        );

        assert!(matches!(
            apply_update(&install.layout, &plan, &FakePlatform::new()),
            Err(ApplyError::UnknownComponent { .. })
        ));
        assert_eq!(snapshot(install.root()), before);
    }

    #[test]
    fn a_component_installing_outside_its_scope_is_refused() {
        // The destination is checked against what this install's shape implies
        // rather than merely being confined to the root: `content` installing
        // over `bin`, or the engine over the root, would each be a containment
        // pass and a catastrophe.
        let install = install();
        let before = snapshot(install.root());
        for (name, destination) in [
            ("content", "bin"),
            ("content", "Screenshots"),
            ("planet", ""),
            ("engine", "bin"),
        ] {
            let archive = install.content_archive();
            let plan = plan("0.4.0", vec![component(name, &archive, destination)]);
            assert!(
                matches!(
                    apply_update(&install.layout, &plan, &FakePlatform::new()),
                    Err(ApplyError::DestinationOutOfScope { .. })
                ),
                "{name} installing at {destination:?} should be refused"
            );
        }
        assert_eq!(snapshot(install.root()), before);
    }

    #[test]
    fn an_archive_whose_digest_no_longer_matches_is_refused_before_the_swap() {
        // The download was verified when it arrived, but the applier is a
        // different process at a later time and the file has been sitting on
        // disk in between.
        let install = install();
        let before = snapshot(install.root());
        let mut component = component("content", &install.content_archive(), "content");
        component.sha256 = "aa".repeat(32);

        assert!(matches!(
            apply_update(
                &install.layout,
                &plan("0.4.0", vec![component]),
                &FakePlatform::new()
            ),
            Err(ApplyError::Digest(_))
        ));
        assert_eq!(snapshot(install.root()), before);
    }

    #[test]
    fn an_engine_archive_without_a_binaries_directory_is_refused_before_the_swap() {
        let install = install();
        let before = snapshot(install.root());
        let archive = archive(&install.downloads, "engine.zip", &[("COPYING", "licence")]);

        assert!(matches!(
            apply_update(
                &install.layout,
                &plan("0.4.0", vec![component("engine", &archive, "")]),
                &FakePlatform::new()
            ),
            Err(ApplyError::MissingPayload { .. })
        ));
        assert_eq!(snapshot(install.root()), before);
    }

    #[test]
    fn the_pending_journal_is_durable_before_staging_and_cleared_after_failure() {
        let install = install_with(true);
        let plan = plan(
            "0.4.0",
            vec![component(
                "content",
                &install.content_archive(),
                "Contents/Resources/content",
            )],
        );
        let staged_path = std::cell::RefCell::new(None);

        let error = apply_update_with_stager(
            &install.layout,
            &plan,
            &FakePlatform::new(),
            |layout, steps, nonce| {
                assert!(
                    Journal::load(&layout.work_dir())
                        .expect("load journal while staging")
                        .is_some(),
                    "a crash while staging needs a durable cleanup record"
                );
                let destination = layout.root().join(&steps[0].destination);
                let locations = locate(layout, nonce, &steps[0].component, &destination)?;
                write_file(&locations.staged.join("partial"), "partial extraction");
                staged_path.replace(Some(locations.staged));
                Err(ApplyError::StagingLost {
                    component: steps[0].component.clone(),
                })
            },
        )
        .expect_err("the injected staging failure must propagate");

        assert!(matches!(error, ApplyError::StagingLost { .. }), "{error}");
        assert_eq!(
            Journal::load(&install.layout.work_dir()).expect("load after cleanup"),
            None
        );
        assert!(!staged_path.into_inner().expect("staged path").exists());
    }

    #[test]
    fn staging_is_a_sibling_of_the_destination_not_a_temporary_directory() {
        // `rename` cannot cross filesystems, so staging under `cache_dir` or
        // the system temp directory would fail with EXDEV on any install that
        // lives on its own volume — after the download had already happened.
        let layout = InstallLayout::plain(Path::new("/opt/clonk"));
        let locations =
            locate(&layout, "n0nce", "content", Path::new("/opt/clonk/content")).expect("locate");
        assert_eq!(locations.staged, Path::new("/opt/clonk/content.new-n0nce"));
        assert_eq!(locations.staged.parent(), locations.destination.parent());
        assert_eq!(locations.backup, Path::new("/opt/clonk/content.old-n0nce"));
        assert_eq!(locations.backup.parent(), locations.destination.parent());
    }

    #[test]
    fn newly_created_update_directories_sync_each_parent_outermost_first() {
        let directory = TempDir::new().expect("directory");
        let target = directory.path().join("quarantine/engine");
        let mut synced = Vec::new();

        ensure_dir_with_sync(&target, |parent| {
            synced.push(parent.to_path_buf());
            Ok(())
        })
        .expect("create durable directories");

        assert_eq!(
            synced,
            [
                directory.path().to_path_buf(),
                directory.path().join("quarantine")
            ]
        );
    }

    #[test]
    fn cross_directory_rename_syncs_destination_before_source() {
        let mut synced = Vec::new();

        sync_rename_parents_with(
            Path::new("/installed/bin"),
            Path::new("/recovery/bin"),
            |parent| {
                synced.push(parent.to_path_buf());
                Ok(())
            },
        )
        .expect("sync renamed entry");

        assert_eq!(
            synced,
            [PathBuf::from("/recovery"), PathBuf::from("/installed")]
        );
    }

    #[test]
    fn a_bundle_keeps_its_journal_and_backups_outside_the_app() {
        // Measured, not assumed: `codesign --verify --strict` reports
        // "unsealed contents present in the bundle root" for a file beside
        // `Contents`, and "a sealed resource is missing or invalid" for one
        // deleted after signing. Nothing transient can live inside a `.app`
        // across the re-sign that has to follow the last swap.
        let layout = InstallLayout::macos_bundle(Path::new("/Applications/Clonk Rust.app"));
        assert_eq!(
            layout.work_dir(),
            Path::new("/Applications/.clonk-update/Clonk Rust.app")
        );
        assert_eq!(
            layout.data_dir(),
            Path::new("/Applications/Clonk Rust.app/Contents/Resources")
        );
        assert_eq!(
            layout.binaries_dir(),
            Path::new("/Applications/Clonk Rust.app/Contents/MacOS")
        );

        let locations = locate(
            &layout,
            "n0nce",
            "content",
            &layout.data_dir().join("content"),
        )
        .expect("locate");
        assert!(
            !locations.backup.starts_with(layout.root()),
            "a backup inside the bundle would fail --verify --strict: {:?}",
            locations.backup
        );
        assert_eq!(
            locations.backup,
            Path::new(
                "/Applications/.clonk-update/Clonk Rust.app/clonk-update-backup-n0nce/content"
            )
        );
    }

    #[test]
    fn sibling_bundles_have_disjoint_transaction_namespaces() {
        let first = InstallLayout::macos_bundle(Path::new("/Applications/First.app"));
        let second = InstallLayout::macos_bundle(Path::new("/Applications/Second.app"));

        assert_ne!(first.work_dir(), second.work_dir());
        assert!(first.work_dir().starts_with(Path::new("/Applications")));
        assert!(second.work_dir().starts_with(Path::new("/Applications")));
        assert!(!first.work_dir().starts_with(first.root()));
        assert!(!second.work_dir().starts_with(second.root()));
    }

    #[test]
    fn a_bundle_layout_is_found_from_either_the_app_or_the_install_root() {
        let directory = TempDir::new().expect("directory");
        let app = directory.path().join("Clonk Rust.app");
        std::fs::create_dir_all(app.join("Contents/MacOS")).expect("create bundle");
        std::fs::create_dir_all(app.join("Contents/Resources")).expect("create resources");

        assert_eq!(
            InstallLayout::discover(&app),
            InstallLayout::macos_bundle(&app)
        );
        assert_eq!(
            InstallLayout::discover(&app.join("Contents/Resources")),
            InstallLayout::macos_bundle(&app)
        );
        let plain = directory.path().join("install");
        std::fs::create_dir_all(plain.join("bin")).expect("create install");
        assert_eq!(
            InstallLayout::discover(&plain),
            InstallLayout::plain(&plain)
        );
    }

    // The engine component has always carried the bundle icon and always thrown
    // it away: only `Contents/MacOS` is swapped, so an install updated in place
    // kept its original icon for ever. A stale icon, unlike a stale copyright
    // notice, is a failure the user looks straight at.
    #[test]
    fn a_bundle_apply_installs_the_new_icon() {
        let install = install_with(true);
        let plan = plan(
            "0.4.0",
            vec![component("engine", &install.engine_archive(), "")],
        );

        apply_update(&install.layout, &plan, &FakePlatform::new()).expect("apply the update");

        assert_eq!(read_file(&install.root().join(BUNDLE_ICON)), "new icon");
        // The rest of `Contents` still stays alone: the icon is an exception
        // made for one file, not a general widening of what a swap touches.
        assert!(!install.layout.data_dir().join("COPYING").exists());
    }

    // A component that ships no icon must leave the installed one in place
    // rather than deleting it — an iconless bundle is worse than a stale icon.
    #[test]
    fn a_bundle_apply_without_an_icon_keeps_the_installed_one() {
        let install = install_with(true);
        let archive_without_icon = archive(
            &install.downloads,
            "engine-no-icon.zip",
            &[
                ("Contents/MacOS/clonk-app", "new app"),
                ("Contents/MacOS/clonk-game", "new launcher"),
                ("Contents/MacOS/c4group", "new c4group"),
            ],
        );
        let plan = plan(
            "0.4.0",
            vec![component("engine", &archive_without_icon, "")],
        );

        apply_update(&install.layout, &plan, &FakePlatform::new()).expect("apply the update");

        assert_eq!(read_file(&install.root().join(BUNDLE_ICON)), "old icon");
    }

    // The icon has to go in before the bundle is re-signed, or the seal would
    // not cover it — so it is already installed when a failing signature rolls
    // the update back, and the rollback has to put the old one back too.
    #[test]
    fn a_rolled_back_bundle_apply_restores_the_old_icon() {
        let install = install_with(true);
        let plan = plan(
            "0.4.0",
            vec![component("engine", &install.engine_archive(), "")],
        );

        let error = apply_update(
            &install.layout,
            &plan,
            &FakePlatform::new().failing_codesign("--verify"),
        )
        .expect_err("a bundle whose signature does not verify must not be kept");

        assert_eq!(read_file(&install.root().join(BUNDLE_ICON)), "old icon");
        assert_eq!(
            read_file(&install.layout.binaries_dir().join("clonk-app")),
            "old app",
            "{error}"
        );
        assert_eq!(
            InstalledState::load(&install.layout.data_dir()).expect("load installed state"),
            None,
            "a failed first update must remove the state it wrote before signing"
        );
    }

    #[test]
    fn a_bundle_rollback_restores_the_absence_of_an_icon() {
        let install = install_with(true);
        std::fs::remove_file(install.root().join(BUNDLE_ICON)).expect("remove old icon");
        let plan = plan(
            "0.4.0",
            vec![component("engine", &install.engine_archive(), "")],
        );

        apply_update(
            &install.layout,
            &plan,
            &FakePlatform::new().failing_codesign_once("--verify"),
        )
        .expect_err("the first verification must fail");

        assert!(!install.root().join(BUNDLE_ICON).exists());
    }

    #[test]
    fn a_rolled_back_bundle_apply_restores_the_previous_installed_state() {
        let install = install_with(true);
        let mut previous = InstalledState::default();
        previous.record("content", "0.3.0", &"cc".repeat(32));
        previous
            .save(&install.layout.data_dir())
            .expect("save previous state");
        let plan = plan(
            "0.4.0",
            vec![component("engine", &install.engine_archive(), "")],
        );

        apply_update(
            &install.layout,
            &plan,
            &FakePlatform::new().failing_codesign("--verify"),
        )
        .expect_err("a bundle whose signature does not verify must not be kept");

        assert_eq!(
            InstalledState::load(&install.layout.data_dir()).expect("load installed state"),
            Some(previous)
        );
    }

    #[test]
    fn a_bundle_rollback_restores_the_previous_state_bytes_exactly() {
        let install = install_with(true);
        let previous = format!(
            "{{\n  \"future_field\": true,\n  \"components\": {{\n    \"content\": {{ \"sha256\": \"{}\", \"version\": \"0.3.0\" }}\n  }},\n  \"schema\": 1\n}}\n",
            "cc".repeat(32)
        );
        let path = InstalledState::path_in(&install.layout.data_dir());
        std::fs::write(&path, previous.as_bytes()).expect("write previous state");
        let plan = plan(
            "0.4.0",
            vec![component("engine", &install.engine_archive(), "")],
        );

        apply_update(
            &install.layout,
            &plan,
            &FakePlatform::new().failing_codesign("--verify"),
        )
        .expect_err("a bundle whose signature does not verify must not be kept");

        assert_eq!(
            std::fs::read(&path).expect("read restored state"),
            previous.as_bytes()
        );
    }

    #[test]
    fn a_bundle_apply_signs_the_nested_executables_before_the_bundle_and_verifies() {
        // The bundle's own signature seals its nested code, so signing the
        // bundle first would seal executables that are about to be replaced.
        let install = install_with(true);
        let platform = FakePlatform::new();
        let plan = plan(
            "0.4.0",
            vec![
                component(
                    "content",
                    &install.content_archive(),
                    "Contents/Resources/content",
                ),
                component("engine", &install.engine_archive(), ""),
            ],
        );

        apply_update(&install.layout, &plan, &platform).expect("apply the update");

        let root = install.root().to_path_buf();
        let codesigns: Vec<_> = platform
            .calls()
            .into_iter()
            .filter_map(|call| match call {
                PlatformCall::Codesign { arguments, target } => Some((arguments, target)),
                _ => None,
            })
            .collect();
        assert_eq!(
            codesigns,
            vec![
                (
                    vec!["--force".to_string(), "--sign".to_string(), "-".to_string()],
                    root.join("Contents/MacOS/clonk-game")
                ),
                (
                    vec!["--force".to_string(), "--sign".to_string(), "-".to_string()],
                    root.join("Contents/MacOS/c4group")
                ),
                (
                    vec!["--force".to_string(), "--sign".to_string(), "-".to_string()],
                    root.clone()
                ),
                (
                    vec![
                        "--verify".to_string(),
                        "--deep".to_string(),
                        "--strict".to_string()
                    ],
                    root
                ),
            ]
        );
    }

    /// Reports what was inside the bundle at the moment it was signed, which is
    /// the only moment the answer matters.
    struct Watcher {
        root: PathBuf,
        seen: std::sync::Mutex<Vec<String>>,
        states_seen: std::sync::Mutex<Vec<Option<InstalledState>>>,
    }

    impl PlatformOps for Watcher {
        fn available_space(&self, _path: &Path) -> Result<u64, PlatformError> {
            Ok(u64::MAX)
        }

        fn wait_for_process(&self, _pid: u32, _timeout: Duration) -> Result<(), PlatformError> {
            Ok(())
        }

        fn codesign(&self, _arguments: &[&str], _target: &Path) -> Result<(), PlatformError> {
            let transient = snapshot(&self.root)
                .into_iter()
                .map(|(path, _)| path)
                .filter(|path| {
                    path.contains("clonk-update")
                        || path.contains(".new-")
                        || path.contains(".old-")
                });
            self.seen
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .extend(transient);
            self.states_seen
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(
                    InstalledState::load(&self.root.join("Contents/Resources"))
                        .expect("load state while signing"),
                );
            Ok(())
        }

        fn set_installed_version(&self, _version: &str) -> Result<(), PlatformError> {
            Ok(())
        }
    }

    struct RollbackWatcher {
        watcher: Watcher,
        fail_verify_once: std::sync::atomic::AtomicBool,
    }

    impl PlatformOps for RollbackWatcher {
        fn available_space(&self, path: &Path) -> Result<u64, PlatformError> {
            self.watcher.available_space(path)
        }

        fn wait_for_process(&self, pid: u32, timeout: Duration) -> Result<(), PlatformError> {
            self.watcher.wait_for_process(pid, timeout)
        }

        fn codesign(&self, arguments: &[&str], target: &Path) -> Result<(), PlatformError> {
            self.watcher.codesign(arguments, target)?;
            let fail = arguments.contains(&"--verify")
                && self
                    .fail_verify_once
                    .swap(false, std::sync::atomic::Ordering::Relaxed);
            if fail {
                return Err(PlatformError::Codesign {
                    arguments: arguments.join(" "),
                    target: target.to_path_buf(),
                    status: "exit status: 1".to_string(),
                });
            }
            Ok(())
        }

        fn set_installed_version(&self, version: &str) -> Result<(), PlatformError> {
            self.watcher.set_installed_version(version)
        }
    }

    #[test]
    fn nothing_transient_is_inside_a_bundle_at_the_moment_it_is_signed() {
        // Measured on macOS: a staging or backup directory still inside the
        // `.app` is either sealed in — and then breaks the seal the moment it
        // is deleted, "a sealed resource is missing or invalid" — or, if it
        // arrives afterwards, fails outright with "unsealed contents present in
        // the bundle root". Asserting on the finished tree would not catch
        // either; the state *during* signing is what decides.
        let install = install_with(true);
        let watcher = Watcher {
            root: install.root().to_path_buf(),
            seen: std::sync::Mutex::new(Vec::new()),
            states_seen: std::sync::Mutex::new(Vec::new()),
        };
        let plan = plan(
            "0.4.0",
            vec![
                component(
                    "content",
                    &install.content_archive(),
                    "Contents/Resources/content",
                ),
                component("engine", &install.engine_archive(), ""),
            ],
        );

        apply_update(&install.layout, &plan, &watcher).expect("apply the update");

        let seen = watcher
            .seen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        assert!(seen.is_empty(), "inside the bundle while signing: {seen:?}");
        let states_seen = watcher
            .states_seen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        assert!(
            !states_seen.is_empty()
                && states_seen.iter().all(|state| {
                    state.as_ref().is_some_and(|state| {
                        state.component("content").is_some() && state.component("engine").is_some()
                    })
                }),
            "installed state was not complete before signing: {states_seen:?}"
        );
    }

    #[test]
    fn bundle_rollback_discards_transients_before_resigning() {
        let install = install_with(true);
        let watcher = RollbackWatcher {
            watcher: Watcher {
                root: install.root().to_path_buf(),
                seen: std::sync::Mutex::new(Vec::new()),
                states_seen: std::sync::Mutex::new(Vec::new()),
            },
            fail_verify_once: std::sync::atomic::AtomicBool::new(true),
        };
        let plan = plan(
            "0.4.0",
            vec![component(
                "content",
                &install.content_archive(),
                "Contents/Resources/content",
            )],
        );

        apply_update(&install.layout, &plan, &watcher)
            .expect_err("the first signature verification triggers rollback");

        let seen = watcher
            .watcher
            .seen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        assert!(seen.is_empty(), "inside the bundle while signing: {seen:?}");
    }

    #[test]
    fn a_failing_codesign_verify_rolls_every_component_back() {
        // A bundle that will not verify is reported by macOS as "damaged and
        // can't be opened" — strictly worse than a stale but working copy.
        let install = install_with(true);
        let plan = plan(
            "0.4.0",
            vec![
                component(
                    "content",
                    &install.content_archive(),
                    "Contents/Resources/content",
                ),
                component(
                    "planet",
                    &install.planet_archive(),
                    "Contents/Resources/planet",
                ),
                component("engine", &install.engine_archive(), ""),
            ],
        );

        let error = apply_update(
            &install.layout,
            &plan,
            &FakePlatform::new().failing_codesign("--verify"),
        )
        .expect_err("a bundle that does not verify must not be installed");
        assert!(
            matches!(error, ApplyError::RollbackFailed { .. }),
            "{error}"
        );

        assert_eq!(
            read_file(
                &install
                    .layout
                    .data_dir()
                    .join("content/Worlds.c4f/Info.txt")
            ),
            "old world"
        );
        assert_eq!(
            read_file(&install.layout.data_dir().join("planet/System.c4g/Rank.txt")),
            "old rank"
        );
        assert_eq!(
            read_file(&install.layout.binaries_dir().join("clonk-app")),
            "old app"
        );
        let journal = Journal::load(&install.layout.work_dir())
            .expect("load")
            .expect("a failed restored signature must remain retryable");
        assert_eq!(journal.phase, TransactionPhase::RollingBack);
        assert!(journal.steps.iter().all(|step| step.rollback_complete));
    }

    #[test]
    fn a_bundle_rollback_resigns_and_verifies_the_restored_install() {
        let install = install_with(true);
        let platform = FakePlatform::new().failing_codesign_once("--verify");
        let plan = plan(
            "0.4.0",
            vec![component("engine", &install.engine_archive(), "")],
        );

        let error = apply_update(&install.layout, &plan, &platform)
            .expect_err("the first bundle verification must fail");
        assert!(matches!(error, ApplyError::Platform(_)), "{error}");

        let codesigns: Vec<_> = platform
            .calls()
            .into_iter()
            .filter(|call| matches!(call, PlatformCall::Codesign { .. }))
            .collect();
        assert_eq!(
            codesigns.len(),
            8,
            "the restored bundle needs the complete sign-and-verify sequence"
        );
        assert!(matches!(
            codesigns.last(),
            Some(PlatformCall::Codesign { arguments, .. })
                if arguments == &["--verify", "--deep", "--strict"]
        ));
    }

    /// Hand-builds the on-disk state a crash would leave, because a test cannot
    /// pull the power out from under a real apply.
    fn interrupt(install: &Install, nonce: &str, states: [StepState; 2]) -> Journal {
        let data = install.layout.data_dir();
        let steps = ["content", "planet"];
        let journal = Journal::new(
            "0.4.0",
            nonce,
            install.canonical_root(),
            steps
                .iter()
                .zip(states)
                .map(|(component, state)| JournalStep {
                    component: (*component).to_string(),
                    sha256: match *component {
                        "content" => "aa".repeat(32),
                        "planet" => "bb".repeat(32),
                        _ => String::new(),
                    },
                    destination: (*component).to_string(),
                    carried: match *component {
                        "content" => vec!["MyPack.c4f".to_string()],
                        _ => Vec::new(),
                    },
                    state,
                    destination_existed: Some(true),
                    rollback_complete: false,
                })
                .collect(),
        );

        for (component, state) in steps.iter().zip(states) {
            let destination = data.join(component);
            let staged = destination.with_file_name(format!("{component}.new-{nonce}"));
            let backup = destination.with_file_name(format!("{component}.old-{nonce}"));
            write_file(&staged.join("Fresh.txt"), &format!("new {component}"));
            match state {
                StepState::Pending => {}
                StepState::BackupMoved => {
                    std::fs::rename(&destination, &backup).expect("move the backup aside");
                }
                StepState::Completed => {
                    if *component == "content" {
                        // The carried pack rode into the staged tree before the
                        // swap, exactly as `swap_components` moves it.
                        std::fs::rename(destination.join("MyPack.c4f"), staged.join("MyPack.c4f"))
                            .expect("carry the user pack");
                    }
                    std::fs::rename(&destination, &backup).expect("move the backup aside");
                    std::fs::rename(&staged, &destination).expect("swap the staged tree in");
                }
            }
        }
        journal
            .save(&install.layout.work_dir())
            .expect("save the journal");
        journal
    }

    #[test]
    fn an_update_interrupted_before_any_step_completed_rolls_back() {
        // Nothing the rest of the release depends on is installed yet, so the
        // safe end state is the install the user started the day with.
        let install = install();
        let before = snapshot(install.root());
        interrupt(
            &install,
            "crash1",
            [StepState::BackupMoved, StepState::Pending],
        );

        let outcome =
            resume_interrupted_update_with(&install.layout, &FakePlatform::new()).expect("resume");
        assert_eq!(
            outcome,
            ResumeOutcome::RolledBack {
                version: "0.4.0".to_string()
            }
        );
        assert_eq!(snapshot(install.root()), before);
    }

    #[test]
    fn rollback_restores_a_backup_moved_before_its_state_was_journalled() {
        let install = install();
        drop(UpdateLock::acquire(&install.layout.work_dir()).expect("seed lock file"));
        let before = snapshot(install.root());
        let nonce = "before-backup-save";
        interrupt(&install, nonce, [StepState::Pending, StepState::Pending]);
        let destination = install.layout.data_dir().join("content");
        let backup = destination.with_file_name(format!("content.old-{nonce}"));
        std::fs::rename(&destination, &backup).expect("crash after moving backup");

        let outcome =
            resume_interrupted_update_with(&install.layout, &FakePlatform::new()).expect("resume");

        assert_eq!(
            outcome,
            ResumeOutcome::RolledBack {
                version: "0.4.0".to_string()
            }
        );
        assert_eq!(snapshot(install.root()), before);
    }

    #[test]
    fn rollback_can_resume_after_backups_have_already_been_restored() {
        let install = install();
        let before = snapshot(install.root());
        let mut journal = interrupt(
            &install,
            "rollback-retry",
            [StepState::Completed, StepState::BackupMoved],
        );

        roll_back(&install.layout, &mut journal, &install.layout.work_dir())
            .expect("first rollback attempt");
        roll_back(&install.layout, &mut journal, &install.layout.work_dir())
            .expect("retry rollback after a crash");
        discard_transients(&install.layout, &journal).expect("discard rollback transients");
        Journal::remove(&install.layout.work_dir()).expect("remove journal");

        assert_eq!(snapshot(install.root()), before);
    }

    #[test]
    fn rollback_complete_never_discards_a_backup_that_still_needs_restoring() {
        let install = install();
        let mut journal = interrupt(
            &install,
            "false-rollback-complete",
            [StepState::BackupMoved, StepState::Pending],
        );
        journal.phase = TransactionPhase::RollingBack;
        journal.steps[0].rollback_complete = true;
        journal
            .save(&install.layout.work_dir())
            .expect("save inconsistent crash state");

        resume_interrupted_update_with(&install.layout, &FakePlatform::new()).expect("reconcile");

        assert_eq!(
            read_file(
                &install
                    .layout
                    .data_dir()
                    .join("content/Worlds.c4f/Info.txt")
            ),
            "old world"
        );
    }

    #[test]
    fn rollback_complete_rejects_a_missing_restored_destination() {
        let install = install();
        std::fs::remove_dir_all(install.layout.data_dir().join("content"))
            .expect("remove allegedly restored destination");
        let mut step = JournalStep::new("content", &"aa".repeat(32), "content");
        step.state = StepState::Completed;
        step.rollback_complete = true;
        let mut journal = Journal::new(
            "0.4.0",
            "missing-restored",
            install.canonical_root(),
            vec![step],
        );
        journal.phase = TransactionPhase::RollingBack;
        journal
            .save(&install.layout.work_dir())
            .expect("save inconsistent journal");

        let error = resume_interrupted_update_with(&install.layout, &FakePlatform::new())
            .expect_err("missing old bytes must fail closed");

        assert!(
            matches!(error, ApplyError::InconsistentRollbackState { ref component } if component == "content"),
            "{error}"
        );
        assert!(Journal::path_in(&install.layout.work_dir()).exists());
    }

    #[test]
    fn an_update_interrupted_after_a_step_completed_rolls_forward() {
        // `content` is already the new release; undoing it would leave data one
        // release behind the rest, which is a combination no build was tested
        // as.
        let install = install();
        interrupt(
            &install,
            "crash2",
            [StepState::Completed, StepState::BackupMoved],
        );

        let outcome =
            resume_interrupted_update_with(&install.layout, &FakePlatform::new()).expect("resume");
        assert_eq!(
            outcome,
            ResumeOutcome::RolledForward {
                version: "0.4.0".to_string()
            }
        );

        let data = install.layout.data_dir();
        assert_eq!(read_file(&data.join("content/Fresh.txt")), "new content");
        assert_eq!(read_file(&data.join("planet/Fresh.txt")), "new planet");
        // The carried pack survived the interruption too.
        assert_eq!(
            read_file(&data.join("content/MyPack.c4f/Scenario.txt")),
            "user pack"
        );
        // A resumed `planet` swap still purges the launcher's staged copies.
        assert!(!data.join("System.c4g").exists());
        assert_eq!(read_file(&data.join("Screenshots/shot.png")), "screenshot");
        assert_eq!(
            Journal::load(&install.layout.work_dir()).expect("load"),
            None
        );
    }

    #[test]
    fn a_resumed_apply_records_each_component_release_and_digest() {
        let install = install();
        interrupt(
            &install,
            "crash-state",
            [StepState::Completed, StepState::BackupMoved],
        );

        resume_interrupted_update_with(&install.layout, &FakePlatform::new()).expect("resume");

        let state = InstalledState::load(&install.layout.data_dir())
            .expect("load installed state")
            .expect("resumed apply records installed state");
        assert_eq!(
            state.component("content").expect("content").version,
            "0.4.0"
        );
        assert_eq!(
            state.component("content").expect("content").sha256,
            "aa".repeat(32)
        );
        assert_eq!(state.component("planet").expect("planet").version, "0.4.0");
        assert_eq!(
            state.component("planet").expect("planet").sha256,
            "bb".repeat(32)
        );
    }

    #[test]
    fn a_pending_step_is_completed_from_its_staged_tree_when_rolling_forward() {
        let install = install();
        interrupt(
            &install,
            "crash3",
            [StepState::Completed, StepState::Pending],
        );

        resume_interrupted_update_with(&install.layout, &FakePlatform::new()).expect("resume");
        assert_eq!(
            read_file(&install.layout.data_dir().join("planet/Fresh.txt")),
            "new planet"
        );
    }

    #[test]
    fn roll_forward_never_mistakes_the_old_backup_for_new_staged_data() {
        let install = install();
        let nonce = "missing-new-tree";
        interrupt(
            &install,
            nonce,
            [StepState::Completed, StepState::BackupMoved],
        );
        let destination = install.layout.data_dir().join("planet");
        let staged = destination.with_file_name(format!("planet.new-{nonce}"));
        std::fs::remove_dir_all(&staged).expect("lose staged tree");

        let error = resume_interrupted_update_with(&install.layout, &FakePlatform::new())
            .expect_err("old bytes cannot satisfy a new component step");

        assert!(
            matches!(error, ApplyError::StagingLost { ref component } if component == "planet"),
            "{error}"
        );
        assert!(!destination.exists());
        assert!(
            destination
                .with_file_name(format!("planet.old-{nonce}"))
                .exists(),
            "the old tree remains available for recovery"
        );
    }

    #[test]
    fn resuming_twice_changes_nothing() {
        // Recovery runs at every launch, and the launch after a recovery must
        // be an ordinary one.
        let install = install();
        interrupt(
            &install,
            "crash4",
            [StepState::Completed, StepState::BackupMoved],
        );

        resume_interrupted_update_with(&install.layout, &FakePlatform::new()).expect("first");
        let after_first = snapshot(install.root());
        assert_eq!(
            resume_interrupted_update_with(&install.layout, &FakePlatform::new()).expect("second"),
            ResumeOutcome::NothingToDo
        );
        assert_eq!(snapshot(install.root()), after_first);
    }

    #[test]
    fn resuming_a_rolled_back_update_twice_changes_nothing() {
        let install = install();
        interrupt(&install, "crash5", [StepState::Pending, StepState::Pending]);

        resume_interrupted_update_with(&install.layout, &FakePlatform::new()).expect("first");
        let after_first = snapshot(install.root());
        assert_eq!(
            resume_interrupted_update_with(&install.layout, &FakePlatform::new()).expect("second"),
            ResumeOutcome::NothingToDo
        );
        assert_eq!(snapshot(install.root()), after_first);
    }

    #[test]
    fn an_install_that_was_never_interrupted_has_nothing_to_resume() {
        let install = install();
        assert_eq!(
            resume_interrupted_update_with(&install.layout, &FakePlatform::new()).expect("resume"),
            ResumeOutcome::NothingToDo
        );
    }

    #[test]
    fn recovery_is_refused_while_an_applier_holds_the_install_lock() {
        let install = install();
        let journal = Journal::new(
            "0.4.0",
            "locked-recovery",
            install.canonical_root(),
            Vec::new(),
        );
        journal
            .save(&install.layout.work_dir())
            .expect("save journal");
        let _applier = UpdateLock::acquire(&install.layout.work_dir()).expect("hold lock");

        let error = resume_interrupted_update_with(&install.layout, &FakePlatform::new())
            .expect_err("recovery must not race an applier");

        assert!(
            matches!(error, ApplyError::UpdateInProgress { .. }),
            "{error}"
        );
        assert_eq!(
            Journal::load(&install.layout.work_dir()).expect("load journal"),
            Some(journal)
        );
    }

    #[test]
    fn recovery_detects_an_applier_before_its_first_journal_save() {
        let install = install();
        let _applier = UpdateLock::acquire(&install.layout.work_dir()).expect("hold lock");

        let error = resume_interrupted_update_with(&install.layout, &FakePlatform::new())
            .expect_err("startup must not enter an install while staging begins");

        assert!(
            matches!(error, ApplyError::UpdateInProgress { .. }),
            "{error}"
        );
    }

    #[test]
    fn rollback_recovery_restores_the_exact_previous_installed_state() {
        let install = install();
        let previous = format!(
            "{{\n  \"future_field\": true,\n  \"components\": {{\n    \"content\": {{ \"sha256\": \"{}\", \"version\": \"0.3.0\" }}\n  }},\n  \"schema\": 1\n}}\n",
            "cc".repeat(32)
        )
        .into_bytes();
        let mut current = InstalledState::default();
        current.record("content", "0.4.0", &"dd".repeat(32));
        current
            .save(&install.layout.data_dir())
            .expect("save failed update state");
        let mut journal = Journal::new(
            "0.4.0",
            "state-rollback",
            install.canonical_root(),
            Vec::new(),
        );
        journal.phase = TransactionPhase::RollingBack;
        journal.previous_installed_state = Some(PreviousInstalledState::Present(previous.clone()));
        journal
            .save(&install.layout.work_dir())
            .expect("save rollback journal");

        resume_interrupted_update_with(&install.layout, &FakePlatform::new()).expect("resume");

        assert_eq!(
            std::fs::read(InstalledState::path_in(&install.layout.data_dir()))
                .expect("read restored state"),
            previous
        );
    }

    #[test]
    fn a_resume_refuses_a_journal_it_cannot_trust() {
        // The journal drives renames and deletions, so a corrupt one is an
        // error rather than an invitation to guess.
        let install = install();
        std::fs::write(
            Journal::path_in(&install.layout.work_dir()),
            b"{ not json at all",
        )
        .expect("write");
        assert!(matches!(
            resume_interrupted_update_with(&install.layout, &FakePlatform::new()),
            Err(ApplyError::Journal(_))
        ));
    }

    #[test]
    fn recovery_refuses_a_component_mapped_to_an_unrelated_install_directory() {
        let install = install();
        let mut step = JournalStep::new("content", &"aa".repeat(32), "Screenshots");
        step.state = StepState::Completed;
        Journal::new(
            "0.4.0",
            "wrong-destination",
            install.canonical_root(),
            vec![step],
        )
        .save(&install.layout.work_dir())
        .expect("save journal");
        let before = snapshot(install.root());

        let error = resume_interrupted_update_with(&install.layout, &FakePlatform::new())
            .expect_err("component destinations are fixed by the install layout");

        assert!(
            matches!(error, ApplyError::DestinationOutOfScope { ref component, .. } if component == "content"),
            "{error}"
        );
        assert_eq!(snapshot(install.root()), before);
    }

    #[test]
    fn recovery_refuses_duplicate_component_steps() {
        let install = install();
        let step = JournalStep::new("content", &"aa".repeat(32), "content");
        Journal::new(
            "0.4.0",
            "duplicate",
            install.canonical_root(),
            vec![step.clone(), step],
        )
        .save(&install.layout.work_dir())
        .expect("save journal");

        let error = resume_interrupted_update_with(&install.layout, &FakePlatform::new())
            .expect_err("duplicate steps share staging and backup paths");

        assert!(
            matches!(error, ApplyError::DuplicateComponent { ref component } if component == "content"),
            "{error}"
        );
    }

    #[test]
    fn bundle_recovery_requires_the_original_icon_presence() {
        let install = install_with(true);
        let step = JournalStep::new("content", &"aa".repeat(32), "Contents/Resources/content");
        ensure_dir(&install.layout.work_dir()).expect("create bundle update work directory");
        Journal::new(
            "0.4.0",
            "missing-icon-state",
            install.canonical_root(),
            vec![step],
        )
        .save(&install.layout.work_dir())
        .expect("save journal");

        let error = resume_interrupted_update_with(&install.layout, &FakePlatform::new())
            .expect_err("bundle rollback cannot guess whether a new icon must be removed");

        assert!(
            matches!(error, ApplyError::MissingBundleRecoveryState),
            "{error}"
        );
    }

    #[test]
    fn an_unrelated_install_at_the_same_path_refuses_the_stale_recovery_state() {
        // The pathname says "same location", which is not the same question as
        // "same install". A bundle removed and replaced by an unrelated one
        // leaves the sidecar behind, and the replacement must not consume it.
        let directory = TempDir::new().expect("bundle parent");
        let path = directory.path().join("Clonk.app");
        let stranger = InstallLayout::macos_bundle(path.clone());
        let nonce = "interrupted-by-a-previous-install";
        write_file(&stranger.root().join("Contents/Info.plist"), "metadata");
        let planet = stranger.data_dir().join("planet/System.c4g/Rank.txt");
        write_file(&planet, "the replacement install");
        let backup = stranger.quarantine_dir(nonce).join("planet");
        write_file(
            &backup.join("System.c4g/Rank.txt"),
            "the interrupted install",
        );

        let mut step = JournalStep::new("planet", &"aa".repeat(32), "Contents/Resources/planet");
        step.state = StepState::BackupMoved;
        let mut journal = Journal::new(
            "0.7.0",
            nonce,
            canonical_install_root(&stranger).expect("canonical bundle"),
            vec![step],
        );
        journal.previous_bundle_icon_present = Some(false);
        // Same canonical path, a different install: exactly what removing the
        // old bundle and installing a new one at that path leaves behind.
        journal.install_identity = Some(InstallIdentity::Inode { volume: 0, file: 0 });
        journal
            .save(&stranger.work_dir())
            .expect("save the stale journal");

        let recovery = resume_interrupted_update_with(&stranger, &FakePlatform::new())
            .expect("a stranger declines the state rather than failing");

        assert_eq!(recovery, ResumeOutcome::NothingToDo);
        assert_eq!(
            std::fs::read_to_string(&planet).expect("read the untouched replacement"),
            "the replacement install"
        );
    }

    #[test]
    fn a_renamed_bundle_still_recovers_its_own_transaction() {
        // `install_root` no longer matches after a rename, but the identity
        // does, and the sidecar is found by identity in the parent namespace.
        let directory = TempDir::new().expect("bundle parent");
        let original = InstallLayout::macos_bundle(directory.path().join("Clonk.app"));
        write_file(&original.root().join("Contents/Info.plist"), "metadata");
        // The data directory has to exist for the restore to land in it; the
        // interrupted `planet` is the one thing missing from it.
        write_file(&original.data_dir().join("Graphics.c4g/Keep.txt"), "kept");
        let nonce = "renamed-mid-update";
        let backup = original.quarantine_dir(nonce).join("planet");
        write_file(
            &backup.join("System.c4g/Rank.txt"),
            "the interrupted install",
        );

        let mut step = JournalStep::new("planet", &"aa".repeat(32), "Contents/Resources/planet");
        step.state = StepState::BackupMoved;
        step.destination_existed = Some(true);
        let mut journal = Journal::new(
            "0.7.0",
            nonce,
            canonical_install_root(&original).expect("canonical bundle"),
            vec![step],
        );
        journal.previous_bundle_icon_present = Some(false);
        journal
            .save(&original.work_dir())
            .expect("save the journal for the original name");

        let renamed_path = directory.path().join("Clonk Renamed.app");
        std::fs::rename(original.root(), &renamed_path).expect("rename the bundle");
        let renamed = InstallLayout::macos_bundle(renamed_path);

        let recovery = resume_interrupted_update_with(&renamed, &FakePlatform::new())
            .expect("the renamed bundle owns this transaction");

        assert_ne!(
            recovery,
            ResumeOutcome::NothingToDo,
            "the sidecar has to be found under the bundle's old name"
        );
        assert_eq!(
            std::fs::read_to_string(renamed.data_dir().join("planet/System.c4g/Rank.txt"))
                .expect("the backup was restored into the renamed bundle"),
            "the interrupted install"
        );
    }

    #[test]
    fn a_sibling_bundle_never_recovers_another_bundles_transaction() {
        let directory = TempDir::new().expect("bundle parent");
        let first = InstallLayout::macos_bundle(directory.path().join("First.app"));
        let second = InstallLayout::macos_bundle(directory.path().join("Second.app"));
        let nonce = "first-bundle-interrupted";
        write_file(&first.root().join("Contents/Info.plist"), "first metadata");
        let first_backup = first.quarantine_dir(nonce).join("planet");
        write_file(&first_backup.join("System.c4g/Rank.txt"), "first bundle");
        let second_planet = second.data_dir().join("planet/System.c4g/Rank.txt");
        write_file(&second_planet, "second bundle");
        let mut step = JournalStep::new("planet", &"aa".repeat(32), "Contents/Resources/planet");
        step.state = StepState::BackupMoved;
        let mut journal = Journal::new(
            "0.7.0",
            nonce,
            canonical_install_root(&first).expect("canonical first bundle"),
            vec![step],
        );
        journal.previous_bundle_icon_present = Some(false);
        journal
            .save(&first.work_dir())
            .expect("save first bundle journal");

        let recovery = resume_interrupted_update_with(&second, &FakePlatform::new())
            .expect("a sibling has its own empty recovery namespace");

        assert_eq!(recovery, ResumeOutcome::NothingToDo);
        assert_eq!(
            std::fs::read_to_string(&second_planet).expect("read untouched second bundle"),
            "second bundle"
        );
        assert_eq!(
            std::fs::read_to_string(first_backup.join("System.c4g/Rank.txt"))
                .expect("read retained first bundle backup"),
            "first bundle"
        );
    }

    #[test]
    fn a_bundle_moved_to_another_directory_still_recovers_its_own_transaction() {
        // The sibling-namespace search only reaches a rename: it looks beside
        // where the bundle is *now*. Moving one to a different parent leaves
        // the sidecar in the old namespace entirely, so the transaction is
        // found through the registry the apply recorded, which is keyed by the
        // install's filesystem identity and derived from no install path.
        let directory = TempDir::new().expect("volume");
        let registry = directory.path().join("registry");
        let old_parent = directory.path().join("Downloads");
        let new_parent = directory.path().join("Applications");
        std::fs::create_dir_all(&old_parent).expect("old parent");
        std::fs::create_dir_all(&new_parent).expect("new parent");

        let original = InstallLayout::macos_bundle(old_parent.join("Clonk.app"))
            .with_recovery_registry(&registry);
        write_file(&original.root().join("Contents/Info.plist"), "metadata");
        write_file(&original.data_dir().join("Graphics.c4g/Keep.txt"), "kept");
        let nonce = "moved-mid-update";
        write_file(
            &original
                .quarantine_dir(nonce)
                .join("planet/System.c4g/Rank.txt"),
            "the interrupted install",
        );

        let mut step = JournalStep::new("planet", &"aa".repeat(32), "Contents/Resources/planet");
        step.state = StepState::BackupMoved;
        step.destination_existed = Some(true);
        let mut journal = Journal::new(
            "0.7.0",
            nonce,
            canonical_install_root(&original).expect("canonical bundle"),
            vec![step],
        );
        journal.previous_bundle_icon_present = Some(false);
        journal
            .save(&original.work_dir())
            .expect("save the journal beside the original parent");
        remember_bundle_transaction(&original);

        let moved_path = new_parent.join("Clonk.app");
        std::fs::rename(original.root(), &moved_path).expect("move the bundle");
        let moved = InstallLayout::macos_bundle(moved_path).with_recovery_registry(&registry);

        let recovery = resume_interrupted_update_with(&moved, &FakePlatform::new())
            .expect("the moved bundle owns this transaction");

        assert_ne!(
            recovery,
            ResumeOutcome::NothingToDo,
            "the sidecar has to be found through the registry"
        );
        assert_eq!(
            std::fs::read_to_string(moved.data_dir().join("planet/System.c4g/Rank.txt"))
                .expect("the backup was restored into the moved bundle"),
            "the interrupted install"
        );
        assert_eq!(
            std::fs::read_dir(&registry)
                .expect("read the registry")
                .count(),
            0,
            "a finished transaction leaves nothing for a later install to chase"
        );
    }

    #[test]
    fn one_bundle_never_reaches_another_install_s_registry_entry() {
        // Keying the registry by identity is what keeps it from becoming the
        // pathname test it replaces: a second install reads its own identity,
        // which names no entry, so the first one's sidecar stays untouched even
        // though both consult the same registry.
        let directory = TempDir::new().expect("volume");
        let registry = directory.path().join("registry");
        let interrupted_parent = directory.path().join("Downloads");
        let other_parent = directory.path().join("Applications");
        std::fs::create_dir_all(&interrupted_parent).expect("interrupted parent");
        std::fs::create_dir_all(&other_parent).expect("other parent");

        let interrupted = InstallLayout::macos_bundle(interrupted_parent.join("Clonk.app"))
            .with_recovery_registry(&registry);
        write_file(&interrupted.root().join("Contents/Info.plist"), "metadata");
        let nonce = "interrupted-elsewhere";
        let backup = interrupted.quarantine_dir(nonce).join("planet");
        write_file(
            &backup.join("System.c4g/Rank.txt"),
            "the interrupted install",
        );
        let mut step = JournalStep::new("planet", &"aa".repeat(32), "Contents/Resources/planet");
        step.state = StepState::BackupMoved;
        let mut journal = Journal::new(
            "0.7.0",
            nonce,
            canonical_install_root(&interrupted).expect("canonical interrupted bundle"),
            vec![step],
        );
        journal.previous_bundle_icon_present = Some(false);
        journal
            .save(&interrupted.work_dir())
            .expect("save the interrupted journal");
        remember_bundle_transaction(&interrupted);

        let other = InstallLayout::macos_bundle(other_parent.join("Clonk.app"))
            .with_recovery_registry(&registry);
        write_file(&other.root().join("Contents/Info.plist"), "other metadata");
        let other_planet = other.data_dir().join("planet/System.c4g/Rank.txt");
        write_file(&other_planet, "the other install");

        let recovery = resume_interrupted_update_with(&other, &FakePlatform::new())
            .expect("a second install has nothing of its own to recover");

        assert_eq!(recovery, ResumeOutcome::NothingToDo);
        assert_eq!(
            std::fs::read_to_string(&other_planet).expect("read the untouched second install"),
            "the other install"
        );
        assert!(
            Journal::load(&interrupted.work_dir())
                .expect("load the interrupted journal")
                .is_some(),
            "the interrupted install keeps its transaction"
        );
        assert_eq!(
            std::fs::read_to_string(backup.join("System.c4g/Rank.txt"))
                .expect("read the retained backup"),
            "the interrupted install"
        );
    }

    #[test]
    fn an_entry_naming_a_finished_transaction_is_dropped_rather_than_followed() {
        // A registry entry is a hint, not an authority. One left behind by a
        // transaction that has since finished names a directory holding no
        // journal, and following it would be inventing an interrupted update.
        let directory = TempDir::new().expect("volume");
        let registry = directory.path().join("registry");
        let parent = directory.path().join("Applications");
        std::fs::create_dir_all(&parent).expect("bundle parent");
        let bundle =
            InstallLayout::macos_bundle(parent.join("Clonk.app")).with_recovery_registry(&registry);
        write_file(&bundle.root().join("Contents/Info.plist"), "metadata");
        let planet = bundle.data_dir().join("planet/System.c4g/Rank.txt");
        write_file(&planet, "the installed release");
        ensure_dir(&bundle.work_dir()).expect("an empty transaction directory");
        remember_bundle_transaction(&bundle);

        let recovery = resume_interrupted_update_with(&bundle, &FakePlatform::new())
            .expect("an entry without a journal is not an interrupted update");

        assert_eq!(recovery, ResumeOutcome::NothingToDo);
        assert_eq!(
            std::fs::read_to_string(&planet).expect("read the untouched install"),
            "the installed release"
        );
        assert_eq!(
            std::fs::read_dir(&registry)
                .expect("read the registry")
                .count(),
            0,
            "an entry that leads nowhere is dropped rather than re-read next launch"
        );
    }

    #[test]
    fn legacy_plain_journals_are_upgraded_before_recovery_mutates_the_install() {
        let install = install();
        let mut step = JournalStep::new("content", &"aa".repeat(32), "content");
        step.destination_existed = None;
        let mut journal = Journal::new(
            "0.3.0",
            "legacy-upgrade",
            install.canonical_root(),
            vec![step],
        );
        journal.schema = 1;
        journal.previous_installed_state = None;
        journal
            .save(&install.layout.work_dir())
            .expect("save legacy journal");

        upgrade_legacy_journal(&install.layout, &mut journal, &install.layout.work_dir())
            .expect("upgrade journal");

        assert_eq!(journal.schema, crate::journal::JOURNAL_SCHEMA);
        assert_eq!(journal.steps[0].destination_existed, Some(true));
        assert!(journal.previous_installed_state.is_some());
        assert_eq!(
            Journal::load(&install.layout.work_dir())
                .expect("load upgraded")
                .expect("journal")
                .schema,
            crate::journal::JOURNAL_SCHEMA
        );
    }

    #[test]
    fn legacy_journals_without_digests_fail_closed_before_recovery() {
        let install = install();
        let mut step = JournalStep::new("content", "", "content");
        step.destination_existed = None;
        let mut journal = Journal::new(
            "0.3.0",
            "legacy-without-digest",
            install.canonical_root(),
            vec![step],
        );
        journal.schema = 1;
        journal.previous_installed_state = None;
        journal
            .save(&install.layout.work_dir())
            .expect("save legacy journal");
        let before = snapshot(install.root());

        let error = resume_interrupted_update_with(&install.layout, &FakePlatform::new())
            .expect_err("a legacy journal without digests is unsafe to upgrade");

        assert!(matches!(error, ApplyError::UnsafeLegacyJournalDigest));
        assert_eq!(snapshot(install.root()), before);
    }

    #[test]
    fn free_space_is_demanded_before_a_download_begins() {
        // 299 MB fetched onto a full volume fails at the worst moment; the
        // check costs one syscall and runs first.
        let full = FakePlatform::new().with_available_space(1024);
        let error = ensure_free_space(&full, Path::new("/install"), [100u64, 200])
            .expect_err("a full volume must refuse the update");
        assert!(
            matches!(error, ApplyError::NotEnoughSpace { .. }),
            "{error}"
        );

        let roomy = FakePlatform::new();
        ensure_free_space(&roomy, Path::new("/install"), [100u64, 200]).expect("room to install");
    }

    #[test]
    fn the_space_demanded_covers_the_download_and_its_unpacked_form() {
        assert_eq!(
            required_free_space([100u64, 200]),
            600 + RESERVED_FREE_BYTES
        );
        assert_eq!(required_free_space([]), RESERVED_FREE_BYTES);
        // Sizes are manifest text: an absurd one must saturate, never wrap.
        assert_eq!(required_free_space([u64::MAX, u64::MAX]), u64::MAX);
    }

    #[test]
    fn a_real_install_reports_some_free_space() {
        // Thin, but it is the only assertion that exercises the actual syscall
        // rather than the double every other test uses.
        let directory = TempDir::new().expect("directory");
        let available = RealPlatform
            .available_space(directory.path())
            .expect("free space");
        assert!(available > 0, "a writable temporary directory has room");
    }

    #[test]
    fn windows_only_treats_an_invalid_pid_as_an_already_exited_process() {
        assert_eq!(
            classify_windows_open_error(WINDOWS_ERROR_INVALID_PARAMETER),
            WindowsOpenError::ProcessGone
        );
        assert_eq!(
            classify_windows_open_error(5),
            WindowsOpenError::Failure(5),
            "access denied must not let the updater rename a live process's tree"
        );
    }

    #[test]
    fn windows_wait_failure_is_not_reported_as_a_timeout() {
        assert_eq!(
            classify_windows_wait(WINDOWS_WAIT_OBJECT_0, 0),
            WindowsWaitResult::Exited
        );
        assert_eq!(
            classify_windows_wait(WINDOWS_WAIT_TIMEOUT, 0),
            WindowsWaitResult::TimedOut
        );
        assert_eq!(
            classify_windows_wait(WINDOWS_WAIT_FAILED, 6),
            WindowsWaitResult::Failure(6)
        );
    }

    #[test]
    fn a_planned_component_becomes_a_staged_one_once_its_archive_is_on_disk() {
        let planned = PlannedComponent {
            name: "content".to_string(),
            archive: "content-abc.zip".to_string(),
            sha256: "cc".repeat(32),
            size: 4096,
            source: None,
            destination: PathBuf::from("content"),
        };
        let staged =
            StagedComponent::from_planned(&planned, PathBuf::from("/cache/content-abc.zip"));
        assert_eq!(staged.name, "content");
        assert_eq!(staged.archive, Path::new("/cache/content-abc.zip"));
        assert_eq!(staged.sha256, planned.sha256);
        assert_eq!(staged.size, 4096);
        assert_eq!(staged.destination, Path::new("content"));
    }

    #[test]
    fn an_unpacked_budget_bounds_a_component_without_starving_a_small_one() {
        assert_eq!(unpacked_budget(0), MINIMUM_UNPACKED_BUDGET);
        assert_eq!(
            unpacked_budget(300 * 1024 * 1024),
            300 * 1024 * 1024 * UNPACKED_BUDGET_FACTOR
        );
        assert_eq!(unpacked_budget(u64::MAX), u64::MAX);
    }
}
