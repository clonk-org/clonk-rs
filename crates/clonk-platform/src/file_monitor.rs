//! Developer live-reload directory watching.
//!
//! `C4FileMonitor`'s macOS backend — the one the reference build compiles — is
//! FSEvents with **latency 1.0 s and flags 0**
//! (`C4FileMonitor.cpp:287`). Flags 0 means `kFSEventStreamCreateFlagNone`,
//! *not* `kFileEvents`, so events are **directory-granular and recursive**: the
//! path handed to the callback is always a directory, never the file that
//! changed. Linux inotify behaves the same way — it pushes
//! `watchDescriptors[event->wd]`, ignoring `event->name` (`:80-126`). Only the
//! Windows backend reports a child file path.
//!
//! A one-second poll therefore reproduces the reference backend exactly, with
//! no new dependency. Two behaviours it must keep:
//!
//! - **A directory registered after the monitor starts is silently dropped.**
//!   `AddDirectory` on APPLE is `if (!started) paths.emplace_back(...)`
//!   (`:299-305`), and the Windows backend has the same effective behaviour
//!   because `StartMonitoring` already ran its `for_each`. Only Linux accepts
//!   late additions. The lifecycle that makes this correct is create during
//!   `InitGame`, register while definitions load, start in `InitGameFinal`
//!   (`C4Game.cpp:2413-2424`, `:2738`, `:4445`).
//! - **Dropped events are not recovered.** The callback skips
//!   `kFSEventStreamEventFlagUserDropped|KernelDropped` and does nothing else
//!   (`:256-273`) — no rescan, no resync. A port that adds recovery is
//!   stricter than C++.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// `C4FileMonitor`'s poll-based stand-in for the reference FSEvents backend.
#[derive(Debug, Default)]
pub struct DirectoryMonitor {
    watched: Vec<PathBuf>,
    seen: HashMap<PathBuf, Option<SystemTime>>,
    started: bool,
}

/// The FSEvents latency the reference build asks for (`C4FileMonitor.cpp:287`).
pub const MONITOR_LATENCY_SECONDS: f64 = 1.0;

impl DirectoryMonitor {
    pub fn new() -> Self {
        Self::default()
    }

    /// `C4FileMonitor::AddDirectory`. Registration after `start` is silently
    /// dropped, exactly as the reference backend drops it.
    pub fn add_directory(&mut self, path: impl Into<PathBuf>) -> bool {
        if self.started {
            return false;
        }
        let path = path.into();
        if self.watched.contains(&path) {
            return false;
        }
        self.watched.push(path);
        true
    }

    /// `C4FileMonitor::StartMonitoring` — takes the baseline every later poll
    /// is compared against, so a change that predates the start is not
    /// reported.
    pub fn start(&mut self) {
        self.started = true;
        self.seen = self
            .watched
            .iter()
            .map(|path| (path.clone(), directory_stamp(path)))
            .collect();
    }

    pub fn started(&self) -> bool {
        self.started
    }

    pub fn watched(&self) -> &[PathBuf] {
        &self.watched
    }

    /// The directories that changed since the last poll, in registration
    /// order. Always empty before `start`.
    pub fn poll(&mut self) -> Vec<PathBuf> {
        if !self.started {
            return Vec::new();
        }
        let mut changed = Vec::new();
        for path in &self.watched {
            let stamp = directory_stamp(path);
            let previous = self.seen.insert(path.clone(), stamp);
            // A directory that disappears reports once, then stays quiet: C++
            // gets one event for the removal and nothing after.
            if previous.is_some_and(|previous| previous != stamp) || previous.is_none() {
                if previous.is_none() && stamp.is_none() {
                    continue;
                }
                changed.push(path.clone());
            }
        }
        changed
    }
}

/// The newest modification time among a directory's immediate entries, and the
/// directory's own. `None` when it cannot be read at all.
///
/// Immediate entries only: FSEvents with flags 0 is recursive, but it still
/// reports the *watched* directory, and a definition group's own files are
/// what `C4Def::Load` re-reads.
fn directory_stamp(path: &Path) -> Option<SystemTime> {
    let own = std::fs::metadata(path).ok()?.modified().ok();
    let newest = std::fs::read_dir(path)
        .ok()?
        .filter_map(|entry| entry.ok()?.metadata().ok()?.modified().ok())
        .max();
    match (own, newest) {
        (Some(own), Some(newest)) => Some(own.max(newest)),
        (own, newest) => own.or(newest),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // C4FileMonitor.cpp:275-305 — registration closes at start, and the
    // reported path is the watched directory.
    #[test]
    fn file_monitor_reports_directories_and_refuses_late_registration() {
        let root = tempfile::tempdir().expect("temp root");
        let watched = root.path().join("Rock.c4d");
        std::fs::create_dir_all(&watched).expect("create group");
        std::fs::write(watched.join("Script.c"), b"func Foo() {}").expect("seed file");

        let mut monitor = DirectoryMonitor::new();
        assert!(monitor.add_directory(&watched));
        assert!(
            !monitor.add_directory(&watched),
            "a directory is registered once"
        );
        // Nothing is reported before StartMonitoring runs.
        assert!(monitor.poll().is_empty());

        monitor.start();
        assert!(
            monitor.poll().is_empty(),
            "the start takes the baseline, so an unchanged tree is quiet"
        );

        // A late registration is silently dropped, as the reference backend
        // drops it — the call reports the refusal, the monitor stays quiet.
        let late = root.path().join("Late.c4d");
        std::fs::create_dir_all(&late).expect("create late group");
        assert!(!monitor.add_directory(&late));
        assert_eq!(monitor.watched(), std::slice::from_ref(&watched));

        // A changed file reports its *directory*, never the file itself —
        // flags 0 is directory-granular.
        std::thread::sleep(std::time::Duration::from_millis(10));
        std::fs::write(watched.join("Script.c"), b"func Foo() { return 1; }")
            .expect("touch the script");
        filetime_bump(&watched.join("Script.c"));
        assert_eq!(monitor.poll(), vec![watched.clone()]);
        assert!(
            monitor.poll().is_empty(),
            "one change reports once, not on every later poll"
        );
    }

    /// Some filesystems have coarse mtime granularity; make the change visible
    /// without waiting a whole second in a test.
    fn filetime_bump(path: &Path) {
        let later = SystemTime::now() + std::time::Duration::from_secs(2);
        let _ = std::fs::File::open(path).map(|file| file.set_modified(later));
    }
}
