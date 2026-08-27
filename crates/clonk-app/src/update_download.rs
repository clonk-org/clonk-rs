//! Downloading and staging an accepted update off the main thread.
//!
//! The manifest check decides which components are needed. This module keeps
//! those planned components intact through URL resolution, streams each one to
//! a private cache directory, verifies its size and digest, and writes the
//! [`clonk_update::ApplyPlan`] consumed by the out-of-process launcher.

use anyhow::{bail, Context, Result};
use clonk_platform::AppPaths;
#[cfg(not(test))]
use clonk_update::RealPlatform;
use clonk_update::{
    ensure_free_space, verify_file, ApplyPlan, InstallLayout, PlannedComponent, PlatformOps,
    StagedComponent,
};
#[cfg(not(test))]
use clonk_update_net::HttpTransport;
use clonk_update_net::{
    component_archive_url, published_archive_url, UpdateTransport, DEFAULT_UPDATE_BASE_URL,
};
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;

/// The launcher process exports its PID under this name before starting the
/// runtime. The detached applier waits for both processes before replacing
/// their binaries.
pub(crate) const LAUNCHER_PID_ENV: &str = "LC_GAME_LAUNCHER_PID";
/// A detached applier puts a relaunch failure detail here. The restarted app
/// consumes it once so a successful relaunch stays silent.
pub(crate) const UPDATE_NOTICE_ENV: &str = "LC_GAME_UPDATE_NOTICE";
const UPDATE_OWNER_FILE_PREFIX: &str = ".owner-";

fn record_update_owner(directory: &Path, pid: u32) -> Result<()> {
    if pid == 0 {
        bail!("an update staging owner must have a nonzero process ID");
    }
    let path = directory.join(format!("{UPDATE_OWNER_FILE_PREFIX}{pid}"));
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(file) => file
            .sync_all()
            .with_context(|| format!("failed to persist update owner {}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let metadata = std::fs::symlink_metadata(&path)
                .with_context(|| format!("failed to inspect update owner {}", path.display()))?;
            if metadata.file_type().is_file() {
                Ok(())
            } else {
                bail!("update owner {} is not a regular file", path.display())
            }
        }
        Err(error) => {
            Err(error).with_context(|| format!("failed to record update owner {}", path.display()))
        }
    }
}

#[derive(Debug)]
pub(crate) enum UpdateDownloadEvent {
    Progress { downloaded: u64, total: u64 },
    Prepared { update: PreparedUpdate },
    Failed { detail: String },
}

/// A verified update remains owned by the event queue until the app has
/// successfully handed its plan to the detached helper.
#[derive(Debug)]
pub(crate) struct PreparedUpdate {
    plan_path: PathBuf,
    cleanup_on_drop: bool,
}

impl PreparedUpdate {
    fn new(plan_path: PathBuf) -> Self {
        Self {
            plan_path,
            cleanup_on_drop: true,
        }
    }

    pub(crate) fn plan_path(&self) -> &Path {
        &self.plan_path
    }

    pub(crate) fn hand_off(mut self) {
        self.cleanup_on_drop = false;
    }
}

impl Drop for PreparedUpdate {
    fn drop(&mut self) {
        if self.cleanup_on_drop {
            discard_prepared_update(&self.plan_path);
        }
    }
}

pub(crate) struct PendingUpdateDownload {
    pub(crate) receiver: Receiver<UpdateDownloadEvent>,
    cancelled: Arc<AtomicBool>,
}

impl PendingUpdateDownload {
    pub(crate) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    /// A transfer whose worker has already produced `events`, so a test can
    /// drive the real terminal path in `poll_update_download` instead of a
    /// stand-in for it.
    #[cfg(test)]
    pub(crate) fn with_events(events: Vec<UpdateDownloadEvent>) -> Self {
        let (sender, receiver) = std::sync::mpsc::channel();
        for event in events {
            sender.send(event).expect("the receiver is still alive");
        }
        Self {
            receiver,
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl Drop for PendingUpdateDownload {
    fn drop(&mut self) {
        self.cancel();
    }
}

pub(crate) fn total_component_size(components: &[PlannedComponent]) -> u64 {
    components.iter().fold(0u64, |total, component| {
        total.saturating_add(component.size)
    })
}

fn absolute_archive_path(path: &Path) -> std::io::Result<PathBuf> {
    std::path::absolute(path)
}

fn launcher_pid(value: Option<&str>, runtime_pid: u32) -> Option<u32> {
    value
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|pid| *pid != 0 && *pid != runtime_pid)
}

pub(crate) fn take_update_notice_detail() -> Option<OsString> {
    let detail = std::env::var_os(UPDATE_NOTICE_ENV);
    std::env::remove_var(UPDATE_NOTICE_ENV);
    detail
}

pub(crate) fn update_notice_message(prefix: &str, detail: Option<&OsStr>) -> Option<String> {
    detail.map(|detail| {
        let separator = match prefix.chars().last() {
            Some('.' | '!' | '?') => " ",
            _ => ": ",
        };
        format!("{prefix}{separator}{}", detail.to_string_lossy())
    })
}

fn clear_standard_handle_inheritance_with(
    handles: [isize; 3],
    mut clear: impl FnMut(isize) -> Result<()>,
) -> Result<()> {
    for handle in handles
        .into_iter()
        .filter(|handle| *handle != 0 && *handle != -1)
    {
        clear(handle).with_context(|| {
            format!("failed to clear HANDLE_FLAG_INHERIT on standard handle {handle}")
        })?;
    }
    Ok(())
}

#[cfg(not(windows))]
fn clear_standard_handle_inheritance() -> Result<()> {
    Ok(())
}

#[cfg(windows)]
fn clear_standard_handle_inheritance() -> Result<()> {
    use windows::Win32::Foundation::{
        SetHandleInformation, SetLastError, HANDLE, HANDLE_FLAGS, HANDLE_FLAG_INHERIT, WIN32_ERROR,
    };
    use windows::Win32::System::Console::{
        GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
    };

    let mut handles = [0isize; 3];
    for (index, standard_handle) in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE]
        .into_iter()
        .enumerate()
    {
        // SAFETY: both functions have no pointer arguments. Clearing last error
        // lets a NULL "no standard handle" result remain distinguishable from
        // INVALID_HANDLE_VALUE and other unexpected failures.
        unsafe { SetLastError(WIN32_ERROR(0)) };
        handles[index] = match unsafe { GetStdHandle(standard_handle) } {
            Ok(handle) => handle.0 as isize,
            Err(error) if error.code().is_ok() => 0,
            Err(error) => {
                return Err(anyhow::Error::from(error))
                    .context("failed to read a standard handle before launching the updater")
            }
        };
    }

    clear_standard_handle_inheritance_with(handles, |handle| {
        // SAFETY: invalid sentinel values were filtered above and the handle is
        // used only to clear its inheritance metadata.
        unsafe {
            SetHandleInformation(
                HANDLE(handle as *mut core::ffi::c_void),
                HANDLE_FLAG_INHERIT.0,
                HANDLE_FLAGS(0),
            )
        }
        .map_err(anyhow::Error::from)
    })
}

fn archive_url(
    manifest_base_url: &str,
    version: &str,
    component: &PlannedComponent,
) -> Result<String> {
    component.source.as_ref().map_or_else(
        || {
            // The stock manifest is fetched through `latest`, but unchanged
            // archives live on the tagged release that first published them.
            // A configured mirror instead owns its source-less archives next
            // to the manifest it successfully served.
            let official = component_archive_url(version, &component.archive)?;
            if manifest_base_url.trim_end_matches('/')
                == DEFAULT_UPDATE_BASE_URL.trim_end_matches('/')
            {
                Ok(official)
            } else {
                Ok(format!(
                    "{}/{}",
                    manifest_base_url.trim_end_matches('/'),
                    component.archive
                ))
            }
        },
        |source| published_archive_url(source, &component.archive).map_err(Into::into),
    )
}

fn write_plan(directory: &Path, plan: &ApplyPlan) -> Result<PathBuf> {
    let path = directory.join("plan.json");
    let bytes = serde_json::to_vec_pretty(plan).context("failed to serialize the update plan")?;
    let mut file =
        File::create(&path).with_context(|| format!("failed to create {}", path.display()))?;
    file.write_all(&bytes)
        .and_then(|()| file.write_all(b"\n"))
        .and_then(|()| file.sync_all())
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(path)
}

/// Downloads and verifies one complete release into an owned cache directory.
///
/// Kept synchronous so tests can run it against an in-memory transport. The
/// production caller runs it on a plain worker thread.
fn prepare_update(
    transport: &dyn UpdateTransport,
    platform: &dyn PlatformOps,
    manifest_base_url: &str,
    version: &str,
    components: &[PlannedComponent],
    cache_root: &Path,
    layout: &InstallLayout,
    cancelled: &AtomicBool,
    mut report_progress: impl FnMut(u64, u64),
) -> Result<PathBuf> {
    ensure_free_space(
        platform,
        &layout.work_dir(),
        components.iter().map(|component| component.size),
    )
    .context("the update does not fit beside this installation")?;
    tempfile::Builder::new()
        .prefix(".clonk-update-write-check-")
        .tempfile_in(layout.work_dir())
        .with_context(|| {
            format!(
                "the installation at {} is not writable",
                layout.root().display()
            )
        })?;

    std::fs::create_dir_all(cache_root)
        .with_context(|| format!("failed to create {}", cache_root.display()))?;
    ensure_free_space(
        platform,
        cache_root,
        components.iter().map(|component| component.size),
    )
    .context("the update does not fit in the download cache")?;
    let directory = tempfile::Builder::new()
        .prefix("pending-")
        .tempdir_in(cache_root)
        .with_context(|| format!("failed to stage an update under {}", cache_root.display()))?;
    record_update_owner(directory.path(), std::process::id())?;

    let total = total_component_size(components);
    report_progress(0, total);
    let mut completed = 0u64;
    let mut staged = Vec::with_capacity(components.len());
    for (index, component) in components.iter().enumerate() {
        if cancelled.load(Ordering::Acquire) {
            bail!("the update download was cancelled");
        }
        let url = archive_url(manifest_base_url, version, component)?;
        let archive = absolute_archive_path(
            &directory
                .path()
                .join(format!("{index}-{}", component.archive)),
        )
        .context("failed to resolve a staged update archive path")?;
        let mut size_violation = None;
        let download = transport.download(&url, &archive, &mut |downloaded, declared| {
            if declared != component.size {
                size_violation = Some(format!(
                    "the server declared {declared} bytes for {}, but the manifest promised {}",
                    component.name, component.size
                ));
                return false;
            }
            if downloaded > component.size {
                size_violation = Some(format!(
                    "the server reported {downloaded} downloaded bytes for {}, but the manifest promised {}",
                    component.name, component.size
                ));
                return false;
            }
            report_progress(
                completed.saturating_add(downloaded.min(component.size)),
                total,
            );
            !cancelled.load(Ordering::Acquire)
        });
        if let Some(detail) = size_violation {
            bail!(detail);
        }
        let downloaded =
            download.with_context(|| format!("failed to download {}", component.name))?;
        if cancelled.load(Ordering::Acquire) {
            bail!("the update download was cancelled");
        }
        if downloaded != component.size {
            bail!(
                "downloaded {} bytes for {}, but the manifest promised {}",
                downloaded,
                component.name,
                component.size
            );
        }
        verify_file(&archive, &component.sha256)
            .with_context(|| format!("downloaded {} failed verification", component.name))?;
        completed = completed.saturating_add(component.size);
        report_progress(completed, total);
        staged.push(StagedComponent::from_planned(component, archive));
    }

    let plan = ApplyPlan {
        version: version.to_string(),
        components: staged,
    };
    let plan_path = write_plan(directory.path(), &plan)?;
    let kept = directory.keep();
    Ok(kept.join(
        plan_path
            .file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new("plan.json")),
    ))
}

#[cfg(not(test))]
pub(crate) fn spawn_update_download(
    manifest_base_url: String,
    version: String,
    components: Vec<PlannedComponent>,
    paths: AppPaths,
) -> PendingUpdateDownload {
    let (sender, receiver) = mpsc::channel();
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = Arc::clone(&cancelled);
    std::thread::spawn(move || {
        let result = HttpTransport::new()
            .map_err(anyhow::Error::from)
            .and_then(|transport| {
                let layout = InstallLayout::for_app_paths(&paths);
                let cache_root = paths.cache_dir().join("Updates");
                prepare_update(
                    &transport,
                    &RealPlatform,
                    &manifest_base_url,
                    &version,
                    &components,
                    &cache_root,
                    &layout,
                    &worker_cancelled,
                    |downloaded, total| {
                        let _ = sender.send(UpdateDownloadEvent::Progress { downloaded, total });
                    },
                )
            });
        let terminal = match result {
            Ok(plan_path) => UpdateDownloadEvent::Prepared {
                update: PreparedUpdate::new(plan_path),
            },
            Err(error) => UpdateDownloadEvent::Failed {
                detail: format!("{error:#}"),
            },
        };
        let _ = sender.send(terminal);
    });
    PendingUpdateDownload {
        receiver,
        cancelled,
    }
}

/// No app test reaches the network. The parked sender makes an accepted update
/// look like a slow transfer until a test explicitly supplies a transport to
/// [`prepare_update`].
#[cfg(test)]
pub(crate) fn spawn_update_download(
    _manifest_base_url: String,
    _version: String,
    _components: Vec<PlannedComponent>,
    _paths: AppPaths,
) -> PendingUpdateDownload {
    use std::sync::{Mutex, OnceLock};

    static PARKED: OnceLock<Mutex<Vec<mpsc::Sender<UpdateDownloadEvent>>>> = OnceLock::new();
    let (sender, receiver) = mpsc::channel();
    PARKED
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(sender);
    PendingUpdateDownload {
        receiver,
        cancelled: Arc::new(AtomicBool::new(false)),
    }
}

#[derive(Debug)]
struct UpdateProcessPaths {
    install_root: PathBuf,
    user_data_dir: PathBuf,
    cache_dir: PathBuf,
    logs_dir: PathBuf,
    temp_dir: PathBuf,
    config_dir: PathBuf,
    config_file: PathBuf,
}

impl UpdateProcessPaths {
    fn for_app_paths(paths: &AppPaths, install_root: PathBuf) -> Self {
        Self {
            install_root,
            user_data_dir: paths.user_data_dir().to_path_buf(),
            cache_dir: paths.cache_dir().to_path_buf(),
            logs_dir: paths.logs_dir().to_path_buf(),
            temp_dir: paths.temp_dir().to_path_buf(),
            config_dir: paths.config_dir(),
            config_file: paths.config_file(),
        }
    }

    fn apply_to(&self, command: &mut std::process::Command) {
        command
            .env("LC_INSTALL_ROOT", &self.install_root)
            .env("LC_APP_ROOT", &self.install_root)
            .env("LC_USER_DATA_DIR", &self.user_data_dir)
            .env("LC_CACHE_DIR", &self.cache_dir)
            .env("LC_LOGS_DIR", &self.logs_dir)
            .env("LC_TEMP_DIR", &self.temp_dir)
            .env("LC_CONFIG_DIR", &self.config_dir)
            .env("LC_CONFIG_FILE", &self.config_file);
    }
}

pub(crate) fn launch_update_applier(paths: &AppPaths, plan_path: &Path) -> Result<()> {
    let plan_path = std::fs::canonicalize(plan_path)
        .with_context(|| format!("failed to resolve update plan {}", plan_path.display()))?;
    let install_root = std::fs::canonicalize(paths.install_root()).with_context(|| {
        format!(
            "failed to resolve installation root {}",
            paths.install_root().display()
        )
    })?;
    let process_paths = UpdateProcessPaths::for_app_paths(paths, install_root);
    let directory = plan_path
        .parent()
        .context("the prepared update plan has no parent directory")?;
    let executable_name = if cfg!(windows) {
        "clonk-game.exe"
    } else {
        "clonk-game"
    };
    let installed = std::fs::canonicalize(paths.binaries_dir().join(executable_name))
        .with_context(|| format!("the installed update helper {executable_name} is missing"))?;
    let helper = directory.join(executable_name);
    std::fs::copy(&installed, &helper).with_context(|| {
        format!(
            "failed to copy the update helper from {} to {}",
            installed.display(),
            helper.display()
        )
    })?;

    let launcher_pid_value = std::env::var(LAUNCHER_PID_ENV).ok();
    let launcher_pid = launcher_pid(launcher_pid_value.as_deref(), std::process::id());
    let mut command = update_applier_command(
        &helper,
        &plan_path,
        &process_paths,
        std::process::id(),
        launcher_pid,
    );
    clear_standard_handle_inheritance()?;
    let child = command
        .spawn()
        .with_context(|| format!("failed to start the update helper at {}", helper.display()))?;
    if let Err(error) = record_update_owner(directory, child.id()) {
        // The helper repeats this claim before doing any work. A failure here
        // must not make `PreparedUpdate::drop` remove staging under that live
        // process.
        tracing::warn!(%error, "the spawned update helper could not be recorded by its parent");
    }
    Ok(())
}

fn update_applier_command(
    helper: &Path,
    plan_path: &Path,
    paths: &UpdateProcessPaths,
    runtime_pid: u32,
    launcher_pid: Option<u32>,
) -> std::process::Command {
    let mut command = std::process::Command::new(helper);
    command
        .arg("--apply-update")
        .arg(plan_path)
        .arg("--install-root")
        .arg(&paths.install_root)
        .arg("--wait-pid")
        .arg(runtime_pid.to_string())
        .arg("--relaunch")
        .current_dir(
            plan_path
                .parent()
                .and_then(Path::parent)
                .unwrap_or_else(|| Path::new(".")),
        )
        .env_remove(LAUNCHER_PID_ENV)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    paths.apply_to(&mut command);
    if let Some(pid) = launcher_pid.filter(|pid| *pid != 0 && *pid != runtime_pid) {
        command.arg("--wait-pid").arg(pid.to_string());
    }
    command
}

pub(crate) fn discard_prepared_update(plan_path: &Path) {
    if let Some(directory) = plan_path.parent() {
        if let Err(error) = std::fs::remove_dir_all(directory) {
            tracing::warn!(%error, path = %directory.display(), "failed to discard a prepared update");
        }
    }
}

#[cfg(all(
    test,
    any(not(feature = "app-test-shard-mode"), feature = "app-test-shard-5",),
))]
mod tests {
    use super::*;
    use clonk_update::{sha256_reader, ArchiveSource, FakePlatform};
    use clonk_update_net::TransportError;
    use std::io::Cursor;
    use std::sync::Mutex;
    use tempfile::TempDir;

    struct FixtureTransport {
        bytes: Vec<u8>,
        urls: Mutex<Vec<String>>,
    }

    impl UpdateTransport for FixtureTransport {
        fn fetch_manifest(&self, _url: &str) -> Result<Vec<u8>, TransportError> {
            unreachable!("preparing an accepted update never fetches another manifest")
        }

        fn download(
            &self,
            url: &str,
            into: &Path,
            progress: &mut dyn FnMut(u64, u64) -> bool,
        ) -> Result<u64, TransportError> {
            self.urls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(url.to_string());
            std::fs::write(into, &self.bytes).map_err(|source| TransportError::Io {
                path: into.to_path_buf(),
                source,
            })?;
            let size = self.bytes.len() as u64;
            progress(size, size)
                .then_some(size)
                .ok_or(TransportError::Cancelled)
        }
    }

    #[test]
    fn a_source_less_component_uses_the_manifest_mirror_that_succeeded() {
        use crate::update_check::test_support::{manifest_for, FakeTransport, OFFERED_VERSION};

        let manifest_base_url = "https://mirror.example/releases/v0.7.0";
        let outcome = crate::update_check::check_for_updates(
            &FakeTransport::serving(&manifest_for(
                OFFERED_VERSION,
                clonk_core::version::ENGINE_VERSION,
            )),
            manifest_base_url,
            None,
            "aarch64-apple-darwin",
        );
        let crate::update_check::UpdateCheckOutcome::Available {
            manifest_base_url: retained_base,
            version,
            components,
        } = outcome
        else {
            panic!("mirror manifest must offer its source-less component: {outcome:?}");
        };

        assert_eq!(
            archive_url(&retained_base, &version, &components[0]).expect("safe mirror archive URL"),
            format!("{manifest_base_url}/content-{OFFERED_VERSION}.zip")
        );
    }

    #[test]
    fn the_official_manifest_keeps_source_less_components_on_the_tagged_release() {
        let component = PlannedComponent {
            name: "engine".to_string(),
            archive: "update-engine.zip".to_string(),
            sha256: "00".repeat(32),
            size: 1,
            destination: PathBuf::new(),
            source: None,
        };

        assert_eq!(
            archive_url(DEFAULT_UPDATE_BASE_URL, "0.7.0", &component)
                .expect("official archive URL"),
            "https://github.com/clonk-org/clonk-rs/releases/download/v0.7.0/update-engine.zip"
        );
    }

    #[test]
    fn a_mirror_does_not_relax_archive_name_validation() {
        let component = PlannedComponent {
            name: "engine".to_string(),
            archive: "../outside.zip".to_string(),
            sha256: "00".repeat(32),
            size: 1,
            destination: PathBuf::new(),
            source: None,
        };

        assert!(archive_url("https://mirror.example/releases", "0.7.0", &component).is_err());
    }

    #[test]
    fn accepted_components_keep_their_release_source_and_become_a_verified_plan() {
        let root = TempDir::new().expect("install");
        let cache = TempDir::new().expect("update cache");
        let layout = InstallLayout::plain(root.path());
        let bytes = b"component archive".to_vec();
        let sha256 = sha256_reader(Cursor::new(&bytes)).expect("digest");
        let transport = FixtureTransport {
            bytes: bytes.clone(),
            urls: Mutex::new(Vec::new()),
        };
        let components = vec![PlannedComponent {
            name: "content".to_string(),
            archive: "content.zip".to_string(),
            sha256: sha256.clone(),
            size: bytes.len() as u64,
            destination: PathBuf::from("content"),
            source: Some(ArchiveSource {
                repo: "clonk-org/clonk-rs-content".to_string(),
                tag: "content-deadbeef".to_string(),
            }),
        }];
        let mut progress = Vec::new();

        let plan_path = prepare_update(
            &transport,
            &FakePlatform::new(),
            "https://mirror.example/releases/v0.7.0",
            "0.7.0",
            &components,
            cache.path(),
            &layout,
            &AtomicBool::new(false),
            |downloaded, total| progress.push((downloaded, total)),
        )
        .expect("prepare update");

        assert_eq!(
            transport
                .urls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .as_slice(),
            [
                "https://github.com/clonk-org/clonk-rs-content/releases/download/\
              content-deadbeef/content.zip"
            ]
        );
        let plan: ApplyPlan =
            serde_json::from_slice(&std::fs::read(&plan_path).expect("read prepared plan"))
                .expect("parse prepared plan");
        assert_eq!(plan.version, "0.7.0");
        assert_eq!(plan.components.len(), 1);
        assert_eq!(plan.components[0].sha256, sha256);
        assert_eq!(
            std::fs::read(&plan.components[0].archive).expect("staged archive"),
            bytes
        );
        assert_eq!(progress.first(), Some(&(0, 17)));
        assert_eq!(progress.last(), Some(&(17, 17)));
        assert!(
            plan_path
                .parent()
                .expect("pending directory")
                .join(format!(".owner-{}", std::process::id()))
                .is_file(),
            "the active app must own staging until helper handoff"
        );

        discard_prepared_update(&plan_path);
    }

    #[test]
    fn cache_volume_space_is_checked_before_any_component_download() {
        struct SplitVolume {
            cache: PathBuf,
        }

        impl PlatformOps for SplitVolume {
            fn available_space(&self, path: &Path) -> Result<u64, clonk_update::PlatformError> {
                Ok(if path.starts_with(&self.cache) {
                    0
                } else {
                    u64::MAX
                })
            }

            fn wait_for_process(
                &self,
                _pid: u32,
                _timeout: std::time::Duration,
            ) -> Result<(), clonk_update::PlatformError> {
                Ok(())
            }

            fn codesign(
                &self,
                _arguments: &[&str],
                _target: &Path,
            ) -> Result<(), clonk_update::PlatformError> {
                Ok(())
            }

            fn set_installed_version(
                &self,
                _version: &str,
            ) -> Result<(), clonk_update::PlatformError> {
                Ok(())
            }
        }

        let root = TempDir::new().expect("install");
        let cache = TempDir::new().expect("update cache");
        let bytes = b"component archive".to_vec();
        let transport = FixtureTransport {
            bytes: bytes.clone(),
            urls: Mutex::new(Vec::new()),
        };
        let component = PlannedComponent {
            name: "content".to_string(),
            archive: "content.zip".to_string(),
            sha256: sha256_reader(Cursor::new(&bytes)).expect("digest"),
            size: bytes.len() as u64,
            destination: PathBuf::from("content"),
            source: None,
        };

        let error = prepare_update(
            &transport,
            &SplitVolume {
                cache: cache.path().to_path_buf(),
            },
            DEFAULT_UPDATE_BASE_URL,
            "0.7.0",
            &[component],
            cache.path(),
            &InstallLayout::plain(root.path()),
            &AtomicBool::new(false),
            |_, _| {},
        )
        .expect_err("a full cache volume must fail before download");

        assert!(error.to_string().contains("does not fit"), "{error:#}");
        assert!(transport
            .urls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty());
    }

    #[test]
    fn cancelling_before_a_component_leaves_no_staged_update() {
        let root = TempDir::new().expect("install");
        let cache = TempDir::new().expect("update cache");
        let layout = InstallLayout::plain(root.path());
        let bytes = b"component archive".to_vec();
        let transport = FixtureTransport {
            urls: Mutex::new(Vec::new()),
            bytes: bytes.clone(),
        };
        let components = vec![PlannedComponent {
            name: "engine".to_string(),
            archive: "update-engine.zip".to_string(),
            sha256: sha256_reader(Cursor::new(&bytes)).expect("digest"),
            size: bytes.len() as u64,
            destination: PathBuf::new(),
            source: None,
        }];
        let cancelled = AtomicBool::new(true);

        let error = prepare_update(
            &transport,
            &FakePlatform::new(),
            DEFAULT_UPDATE_BASE_URL,
            "0.7.0",
            &components,
            cache.path(),
            &layout,
            &cancelled,
            |_, _| {},
        )
        .expect_err("cancelled update");

        assert!(error.to_string().contains("cancelled"), "{error:#}");
        assert_eq!(
            std::fs::read_dir(cache.path()).expect("read cache").count(),
            0,
            "a cancelled update must not strand its private staging directory"
        );
    }

    #[test]
    fn a_component_with_the_wrong_digest_never_becomes_an_apply_plan() {
        let root = TempDir::new().expect("install");
        let cache = TempDir::new().expect("update cache");
        let layout = InstallLayout::plain(root.path());
        let bytes = b"tampered component".to_vec();
        let transport = FixtureTransport {
            urls: Mutex::new(Vec::new()),
            bytes: bytes.clone(),
        };
        let components = vec![PlannedComponent {
            name: "planet".to_string(),
            archive: "update-planet.zip".to_string(),
            sha256: "00".repeat(32),
            size: bytes.len() as u64,
            destination: PathBuf::from("planet"),
            source: None,
        }];

        let error = prepare_update(
            &transport,
            &FakePlatform::new(),
            DEFAULT_UPDATE_BASE_URL,
            "0.7.0",
            &components,
            cache.path(),
            &layout,
            &AtomicBool::new(false),
            |_, _| {},
        )
        .expect_err("digest mismatch");

        assert!(
            error.to_string().contains("failed verification"),
            "{error:#}"
        );
        assert_eq!(
            std::fs::read_dir(cache.path()).expect("read cache").count(),
            0,
            "unverified bytes must be removed with their private staging directory"
        );
    }

    #[test]
    fn a_declared_component_size_mismatch_stops_at_the_first_chunk() {
        struct WrongDeclaredSize {
            continued: AtomicBool,
        }

        impl UpdateTransport for WrongDeclaredSize {
            fn fetch_manifest(&self, _url: &str) -> Result<Vec<u8>, TransportError> {
                unreachable!("preparing an accepted update never fetches another manifest")
            }

            fn download(
                &self,
                _url: &str,
                into: &Path,
                progress: &mut dyn FnMut(u64, u64) -> bool,
            ) -> Result<u64, TransportError> {
                std::fs::write(into, b"x").map_err(|source| TransportError::Io {
                    path: into.to_path_buf(),
                    source,
                })?;
                if progress(1, 18) {
                    self.continued.store(true, Ordering::Relaxed);
                }
                Ok(17)
            }
        }

        let root = TempDir::new().expect("install");
        let cache = TempDir::new().expect("update cache");
        let bytes = b"component archive";
        let component = PlannedComponent {
            name: "engine".to_string(),
            archive: "update-engine.zip".to_string(),
            sha256: sha256_reader(Cursor::new(bytes)).expect("digest"),
            size: bytes.len() as u64,
            destination: PathBuf::new(),
            source: None,
        };
        let transport = WrongDeclaredSize {
            continued: AtomicBool::new(false),
        };

        let error = prepare_update(
            &transport,
            &FakePlatform::new(),
            DEFAULT_UPDATE_BASE_URL,
            "0.7.0",
            &[component],
            cache.path(),
            &InstallLayout::plain(root.path()),
            &AtomicBool::new(false),
            |_, _| {},
        )
        .expect_err("a response whose declaration differs from the manifest");

        assert!(
            !transport.continued.load(Ordering::Relaxed),
            "the first chunk must cancel a response with the wrong declared size"
        );
        assert!(error.to_string().contains("declared 18"), "{error:#}");
    }

    #[test]
    fn a_component_streaming_past_the_manifest_size_stops_at_that_chunk() {
        struct OversizedBody {
            continued: AtomicBool,
        }

        impl UpdateTransport for OversizedBody {
            fn fetch_manifest(&self, _url: &str) -> Result<Vec<u8>, TransportError> {
                unreachable!("preparing an accepted update never fetches another manifest")
            }

            fn download(
                &self,
                _url: &str,
                into: &Path,
                progress: &mut dyn FnMut(u64, u64) -> bool,
            ) -> Result<u64, TransportError> {
                std::fs::write(into, b"x").map_err(|source| TransportError::Io {
                    path: into.to_path_buf(),
                    source,
                })?;
                if progress(18, 17) {
                    self.continued.store(true, Ordering::Relaxed);
                }
                Ok(18)
            }
        }

        let root = TempDir::new().expect("install");
        let cache = TempDir::new().expect("update cache");
        let bytes = b"component archive";
        let component = PlannedComponent {
            name: "engine".to_string(),
            archive: "update-engine.zip".to_string(),
            sha256: sha256_reader(Cursor::new(bytes)).expect("digest"),
            size: bytes.len() as u64,
            destination: PathBuf::new(),
            source: None,
        };
        let transport = OversizedBody {
            continued: AtomicBool::new(false),
        };

        let error = prepare_update(
            &transport,
            &FakePlatform::new(),
            DEFAULT_UPDATE_BASE_URL,
            "0.7.0",
            &[component],
            cache.path(),
            &InstallLayout::plain(root.path()),
            &AtomicBool::new(false),
            |_, _| {},
        )
        .expect_err("a response that streams beyond the manifest size");

        assert!(
            !transport.continued.load(Ordering::Relaxed),
            "the first oversized chunk must cancel the response"
        );
        assert!(error.to_string().contains("reported 18"), "{error:#}");
    }

    #[test]
    fn dropping_a_queued_prepared_event_discards_its_staging_directory() {
        let cache = TempDir::new().expect("update cache");
        let pending = cache.path().join("pending-finished");
        std::fs::create_dir(&pending).expect("pending directory");
        let plan_path = pending.join("plan.json");
        std::fs::write(&plan_path, b"{}").expect("prepared plan");
        let (sender, receiver) = mpsc::channel();

        sender
            .send(UpdateDownloadEvent::Prepared {
                update: PreparedUpdate::new(plan_path),
            })
            .expect("queue prepared update");
        drop(receiver);

        assert!(
            !pending.exists(),
            "cancelling after preparation must not strand the verified download"
        );
    }

    #[test]
    fn dropping_a_pending_download_cancels_its_worker() {
        let (_sender, receiver) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));

        drop(PendingUpdateDownload {
            receiver,
            cancelled: Arc::clone(&cancelled),
        });

        assert!(cancelled.load(Ordering::Acquire));
    }

    #[test]
    fn updater_helpers_reject_pid_zero_and_saturate_release_size() {
        assert_eq!(launcher_pid(Some("0"), 41), None);
        assert_eq!(launcher_pid(Some("41"), 41), None);
        assert_eq!(launcher_pid(Some("42"), 41), Some(42));
        assert_eq!(launcher_pid(Some("not-a-pid"), 41), None);

        let components = [
            PlannedComponent {
                name: "engine".to_string(),
                archive: "engine.zip".to_string(),
                sha256: "00".repeat(32),
                size: u64::MAX,
                destination: PathBuf::new(),
                source: None,
            },
            PlannedComponent {
                name: "content".to_string(),
                archive: "content.zip".to_string(),
                sha256: "00".repeat(32),
                size: 1,
                destination: PathBuf::from("content"),
                source: None,
            },
        ];
        assert_eq!(total_component_size(&components), u64::MAX);
    }

    #[test]
    fn staged_archive_paths_are_made_absolute() {
        let archive = absolute_archive_path(Path::new("pending/engine.zip"))
            .expect("resolve a relative staging path");

        assert!(archive.is_absolute());
        assert!(archive.ends_with("pending/engine.zip"));
    }

    #[test]
    fn standard_handle_cleanup_skips_invalid_handles_and_surfaces_failures() {
        let mut cleared = Vec::new();
        clear_standard_handle_inheritance_with([0, -1, 11], |handle| {
            cleared.push(handle);
            Ok(())
        })
        .expect("clear the one valid handle");
        assert_eq!(cleared, [11]);

        let mut attempted = Vec::new();
        let error = clear_standard_handle_inheritance_with([11, 12, 13], |handle| {
            attempted.push(handle);
            if handle == 12 {
                bail!("access denied");
            }
            Ok(())
        })
        .expect_err("a Win32 handle failure must abort helper launch");
        assert_eq!(attempted, [11, 12]);
        assert!(format!("{error:#}").contains("access denied"));
    }

    #[test]
    fn relaunched_update_notice_is_present_only_for_a_failure_detail() {
        assert_eq!(update_notice_message("Update failed.", None), None);
        assert_eq!(
            update_notice_message(
                "Update failed.",
                Some(std::ffi::OsStr::new("could not replace clonk-game")),
            ),
            Some("Update failed. could not replace clonk-game".to_string())
        );
    }

    #[test]
    fn the_detached_helper_receives_the_plan_install_and_both_live_processes() {
        let directory = TempDir::new().expect("update directory");
        let pending = directory.path().join("pending-accepted");
        let helper = pending.join("clonk-game");
        let plan = pending.join("plan.json");
        let install = directory.path().join("install");
        let selected = UpdateProcessPaths {
            install_root: install.clone(),
            user_data_dir: directory.path().join("profile"),
            cache_dir: directory.path().join("cache"),
            logs_dir: directory.path().join("logs"),
            temp_dir: directory.path().join("temp"),
            config_dir: directory.path().join("profile/Config"),
            config_file: directory.path().join("profile/Config/config"),
        };

        let command = update_applier_command(&helper, &plan, &selected, 41, Some(42));

        assert_eq!(command.get_program(), helper);
        assert_eq!(command.get_current_dir(), Some(directory.path()));
        assert_eq!(
            command
                .get_envs()
                .find(|(name, _)| *name == std::ffi::OsStr::new(LAUNCHER_PID_ENV)),
            Some((std::ffi::OsStr::new(LAUNCHER_PID_ENV), None))
        );
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [
                "--apply-update",
                plan.to_str().expect("plan path"),
                "--install-root",
                install.to_str().expect("install path"),
                "--wait-pid",
                "41",
                "--relaunch",
                "--wait-pid",
                "42",
            ]
        );

        let command = update_applier_command(&helper, &plan, &selected, 41, Some(0));
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [
                "--apply-update",
                plan.to_str().expect("plan path"),
                "--install-root",
                install.to_str().expect("install path"),
                "--wait-pid",
                "41",
                "--relaunch",
            ]
        );
    }

    #[test]
    fn the_detached_helper_retains_the_selected_profile_paths() {
        let directory = TempDir::new().expect("paths");
        let selected = UpdateProcessPaths {
            install_root: directory.path().join("install"),
            user_data_dir: directory.path().join("profile"),
            cache_dir: directory.path().join("cache"),
            logs_dir: directory.path().join("logs"),
            temp_dir: directory.path().join("temp"),
            config_dir: directory.path().join("profile/Config"),
            config_file: directory.path().join("selected.config"),
        };
        let pending = directory.path().join("Updates/pending-selected");
        let helper = pending.join("clonk-game");
        let plan = pending.join("plan.json");

        let command = update_applier_command(&helper, &plan, &selected, 41, None);
        let environment = command
            .get_envs()
            .collect::<std::collections::BTreeMap<_, _>>();

        for (name, value) in [
            ("LC_INSTALL_ROOT", selected.install_root.as_os_str()),
            ("LC_APP_ROOT", selected.install_root.as_os_str()),
            ("LC_USER_DATA_DIR", selected.user_data_dir.as_os_str()),
            ("LC_CACHE_DIR", selected.cache_dir.as_os_str()),
            ("LC_LOGS_DIR", selected.logs_dir.as_os_str()),
            ("LC_TEMP_DIR", selected.temp_dir.as_os_str()),
            ("LC_CONFIG_DIR", selected.config_dir.as_os_str()),
            ("LC_CONFIG_FILE", selected.config_file.as_os_str()),
        ] {
            assert_eq!(
                environment.get(std::ffi::OsStr::new(name)),
                Some(&Some(value)),
                "missing {name}"
            );
        }
    }
}
