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
//! whole-tree replace would delete them. For the same reason, any top-level
//! entry of the *old* component that the new archive does not contain — a pack
//! the user dropped into `content/`, which `clonk-app` scans — is carried into
//! the staged tree before the swap.
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
//! step, so for a bundle both live in the directory *containing* the `.app`
//! ([`InstallLayout::work_dir`]). A plain install keeps them beside the
//! destination, where the rename that produced them left them.

use crate::decide::PlannedComponent;
use crate::digest::{verify_file, DigestError};
use crate::extract::{extract_archive, ExtractError};
use crate::journal::{Journal, JournalError, JournalStep, StepState};
use clonk_platform::AppPaths;
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;
use thiserror::Error;

/// Headroom demanded on top of the payload before a download starts.
///
/// An install that fills its own volume is a failure mode the user cannot fix
/// from inside the game, so the check is deliberately pessimistic.
pub const RESERVED_FREE_BYTES: u64 = 256 * 1024 * 1024;

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
}

impl InstallLayout {
    pub fn plain(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            bundle: false,
        }
    }

    /// `app_dir` is the `.app` itself, not its `Contents/Resources`.
    pub fn macos_bundle(app_dir: impl Into<PathBuf>) -> Self {
        Self {
            root: app_dir.into(),
            bundle: true,
        }
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
    /// Beside the install for a plain tree. For a bundle it is the directory
    /// containing the `.app`, because nothing inside a `.app` can survive the
    /// re-sign that has to follow the last swap — see the module documentation.
    pub fn work_dir(&self) -> PathBuf {
        match self.bundle {
            true => self
                .root
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| self.root.clone()),
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
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
    // `SYNCHRONIZE` is a standard access right; windows-sys happens to declare
    // it beside the file rights, and both aliases are plain `u32`.
    use windows_sys::Win32::Storage::FileSystem::SYNCHRONIZE;
    use windows_sys::Win32::System::Threading::{OpenProcess, WaitForSingleObject};

    // SAFETY: a plain process-id lookup; a null handle means the process is
    // already gone, which is exactly what the caller is waiting for.
    let handle = unsafe { OpenProcess(SYNCHRONIZE, 0, pid) };
    if handle == 0 {
        return Ok(());
    }
    let milliseconds = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX);
    // SAFETY: `handle` is a live handle this function owns until it is closed.
    let waited = unsafe { WaitForSingleObject(handle, milliseconds) };
    // SAFETY: closing the handle opened above, exactly once.
    unsafe { CloseHandle(handle) };
    (waited == WAIT_OBJECT_0)
        .then_some(())
        .ok_or(PlatformError::WaitTimeout {
            pid,
            seconds: timeout.as_secs(),
        })
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
        RegCloseKey, RegOpenKeyExA, RegSetValueExA, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_SZ,
    };

    let failed = |operation: &'static str, status: u32| PlatformError::Io {
        operation,
        path: PathBuf::from("HKCU\\...\\Uninstall\\ClonkRust"),
        source: std::io::Error::from_raw_os_error(status as i32),
    };

    let mut key = 0;
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
        let failing = self
            .failing_codesign_argument
            .as_deref()
            .is_some_and(|failing| arguments.contains(&failing));
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
    Platform(#[from] PlatformError),
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

fn rename(from: &Path, to: &Path) -> Result<(), ApplyError> {
    std::fs::rename(from, to).map_err(|source| ApplyError::Io {
        operation: "renaming",
        path: from.to_path_buf(),
        source,
    })
}

fn ensure_dir(path: &Path) -> Result<(), ApplyError> {
    std::fs::create_dir_all(path).map_err(io("creating", path))
}

/// Removes a file, directory or symlink, treating absence as success.
fn remove_any(path: &Path) -> Result<(), ApplyError> {
    match std::fs::symlink_metadata(path) {
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(io("inspecting", path)(source)),
        // A symlink is removed as a link, never followed: `symlink_metadata`
        // reports the link itself, so a link to a directory lands here.
        Ok(metadata) if metadata.is_dir() => {
            std::fs::remove_dir_all(path).map_err(io("removing", path))
        }
        Ok(_) => std::fs::remove_file(path).map_err(io("removing", path)),
    }
}

/// Top-level names of the old tree that the new archive does not contain.
///
/// Users legitimately drop packs into `content/`, which `clonk-app` scans, and
/// the launcher stages `System.c4g` beside the executables. Neither is in a
/// release archive, and neither may be deleted by installing one.
fn carry_over_names(locations: &Locations) -> Result<Vec<String>, ApplyError> {
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
            .ok_or_else(|| ApplyError::UnrepresentableEntry {
                path: locations.destination.clone(),
            })?
            .to_string();
        if !present(&locations.staged.join(&name)) {
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
        let carried = carry_over_names(&locations)?;
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

/// Everything that follows the last swap and can still fail.
fn commit(
    layout: &InstallLayout,
    journal: &Journal,
    ops: &dyn PlatformOps,
) -> Result<(), ApplyError> {
    if journal.steps.iter().any(|step| step.component == "planet") {
        purge_launcher_staged_groups(layout)?;
    }
    if layout.is_bundle() {
        // Before signing, so the new seal covers the new icon.
        install_bundle_icon(layout, journal)?;
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
fn roll_back(layout: &InstallLayout, journal: &Journal) -> Result<(), ApplyError> {
    for step in journal.steps.iter().rev() {
        let destination = step.destination_in(layout.root())?;
        let locations = locate(layout, &journal.nonce, &step.component, &destination)?;

        if step.state != StepState::Pending {
            // Past `Pending` the destination, if it exists at all, holds the
            // *new* tree: the backup rename already ran.
            if present(&locations.destination) {
                remove_any(&locations.staged)?;
                rename(&locations.destination, &locations.staged)?;
            }
            if present(&locations.backup) {
                rename(&locations.backup, &locations.destination)?;
            }
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
    }

    // The icon is a file rather than one of the swapped trees, so it needs its
    // own restore: `install_bundle_icon` runs before the bundle is re-signed and
    // therefore has already replaced it by the time a failing seal lands here.
    let displaced = displaced_bundle_icon(layout, &journal.nonce);
    if present(&displaced) {
        let installed = layout.root().join(BUNDLE_ICON);
        remove_any(&installed)?;
        rename(&displaced, &installed)?;
    }
    // Every destination is restored by this point, so a stubborn temporary is
    // untidy rather than a failed rollback.
    discard_transients_quietly(layout, journal);
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

    let nonce = new_nonce();
    let work = layout.work_dir();
    ensure_dir(&work)?;

    let mut journal = Journal::new(
        &plan.version,
        &nonce,
        steps
            .iter()
            .map(|step| JournalStep::new(&step.component, &journal_destination(&step.destination)))
            .collect(),
    );

    if let Err(error) = stage_components(layout, &steps, &nonce) {
        // Nothing live has moved yet, so cleaning up is just deleting our own
        // scratch — there is no install state to restore.
        let _ = discard_transients(layout, &journal);
        return Err(error);
    }

    journal.save(&work)?;
    let applied = swap_components(layout, &steps, &mut journal, &work)
        .and_then(|()| commit(layout, &journal, ops));
    if let Err(cause) = applied {
        return Err(match roll_back(layout, &journal) {
            Ok(()) => {
                let _ = Journal::remove(&work);
                cause
            }
            Err(failure) => ApplyError::RollbackFailed {
                cause: cause.to_string(),
                failure: failure.to_string(),
            },
        });
    }

    Journal::remove(&work)?;
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
/// The direction is decided by one rule: **roll forward once any step has
/// completed, roll back otherwise**. A completed step means the install already
/// holds new data that the rest of the release expects, so undoing it would
/// produce a mixture no build was ever tested as.
pub fn resume_interrupted_update_with(
    layout: &InstallLayout,
    ops: &dyn PlatformOps,
) -> Result<ResumeOutcome, ApplyError> {
    let work = layout.work_dir();
    let Some(mut journal) = Journal::load(&work)? else {
        return Ok(ResumeOutcome::NothingToDo);
    };
    let version = journal.version.clone();

    if !journal.any_step_completed() {
        roll_back(layout, &journal)?;
        Journal::remove(&work)?;
        return Ok(ResumeOutcome::RolledBack { version });
    }

    roll_forward(layout, &mut journal, &work)?;
    commit(layout, &journal, ops)?;
    Journal::remove(&work)?;
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
                    // Exactly one of the two exists — that is what makes the
                    // window between the renames survivable.
                    let source = [&locations.staged, &locations.backup]
                        .into_iter()
                        .find(|candidate| present(candidate))
                        .ok_or_else(lost)?
                        .clone();
                    rename(&source, &locations.destination)?;
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
    use zip::write::FileOptions;
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
                .start_file(*entry, FileOptions::default())
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
        // The launcher's staged group in `bin` is not in the archive, so it is
        // carried across rather than deleted.
        assert_eq!(
            read_file(&binaries.join("System.c4g/Rank.txt")),
            "staged rank"
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
    fn a_bundle_keeps_its_journal_and_backups_outside_the_app() {
        // Measured, not assumed: `codesign --verify --strict` reports
        // "unsealed contents present in the bundle root" for a file beside
        // `Contents`, and "a sealed resource is missing or invalid" for one
        // deleted after signing. Nothing transient can live inside a `.app`
        // across the re-sign that has to follow the last swap.
        let layout = InstallLayout::macos_bundle(Path::new("/Applications/Clonk Rust.app"));
        assert_eq!(layout.work_dir(), Path::new("/Applications"));
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
            Path::new("/Applications/clonk-update-backup-n0nce/content")
        );
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
            Ok(())
        }

        fn set_installed_version(&self, _version: &str) -> Result<(), PlatformError> {
            Ok(())
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
    }

    #[test]
    fn a_failing_codesign_verify_rolls_every_component_back() {
        // A bundle that will not verify is reported by macOS as "damaged and
        // can't be opened" — strictly worse than a stale but working copy.
        let install = install_with(true);
        let before = snapshot(install.root());
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
        assert!(matches!(error, ApplyError::Platform(_)), "{error}");

        // The launcher-staged groups are purged before signing and are
        // recreated on the next launch, so they are the one expected
        // difference.
        let expected: Vec<_> = before
            .into_iter()
            .filter(|(path, _)| {
                !path.contains("Resources/System.c4g")
                    && !path.contains("Resources/Graphics.c4g")
                    && !path.contains("MacOS/System.c4g")
            })
            .collect();
        assert_eq!(snapshot(install.root()), expected);
        assert_eq!(
            Journal::load(&install.layout.work_dir()).expect("load"),
            None
        );
    }

    /// Hand-builds the on-disk state a crash would leave, because a test cannot
    /// pull the power out from under a real apply.
    fn interrupt(install: &Install, nonce: &str, states: [StepState; 2]) -> Journal {
        let data = install.layout.data_dir();
        let steps = ["content", "planet"];
        let journal = Journal::new(
            "0.4.0",
            nonce,
            steps
                .iter()
                .zip(states)
                .map(|(component, state)| JournalStep {
                    component: (*component).to_string(),
                    destination: (*component).to_string(),
                    carried: match *component {
                        "content" => vec!["MyPack.c4f".to_string()],
                        _ => Vec::new(),
                    },
                    state,
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
