pub(crate) struct DisplaySleepInhibitor {
    release: Option<Box<dyn FnOnce()>>,
}

impl DisplaySleepInhibitor {
    #[cfg(target_os = "macos")]
    pub(crate) fn acquire() -> Option<Self> {
        Some(Self::new(platform::acquire()))
    }

    #[cfg(not(target_os = "macos"))]
    pub(crate) fn acquire() -> Option<Self> {
        None
    }

    fn new(release: impl FnOnce() + 'static) -> Self {
        Self {
            release: Some(Box::new(release)),
        }
    }

    pub(crate) fn release(mut self) {
        self.release_now();
    }

    fn release_now(&mut self) {
        if let Some(release) = self.release.take() {
            release();
        }
    }
}

impl Drop for DisplaySleepInhibitor {
    fn drop(&mut self) {
        self.release_now();
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use objc2_foundation::{ns_string, NSActivityOptions, NSProcessInfo};

    pub(super) fn acquire() -> impl FnOnce() {
        let process_info = NSProcessInfo::processInfo();
        let activity = process_info.beginActivityWithOptions_reason(
            NSActivityOptions::IdleDisplaySleepDisabled,
            ns_string!("Clonk Rust game window is open"),
        );

        move || {
            // SAFETY: `activity` is the exact token returned by this
            // NSProcessInfo instance and is ended exactly once.
            unsafe { process_info.endActivity(&activity) };
        }
    }
}

#[cfg(all(
    test,
    any(not(feature = "app-test-shard-mode"), feature = "app-test-shard-5",),
))]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use super::DisplaySleepInhibitor;

    #[test]
    fn display_sleep_inhibitor_stays_active_until_event_loop_release() {
        let active = Rc::new(Cell::new(true));
        let releases = Rc::new(Cell::new(0));
        let active_on_release = Rc::clone(&active);
        let releases_on_release = Rc::clone(&releases);
        let inhibitor = DisplaySleepInhibitor::new(move || {
            active_on_release.set(false);
            releases_on_release.set(releases_on_release.get() + 1);
        });

        assert!(active.get());
        assert_eq!(releases.get(), 0);

        inhibitor.release();

        assert!(!active.get());
        assert_eq!(releases.get(), 1);
    }

    #[test]
    fn display_sleep_inhibitor_drop_releases_after_early_exit() {
        let active = Rc::new(Cell::new(true));
        let releases = Rc::new(Cell::new(0));
        let active_on_release = Rc::clone(&active);
        let releases_on_release = Rc::clone(&releases);

        {
            let _inhibitor = DisplaySleepInhibitor::new(move || {
                active_on_release.set(false);
                releases_on_release.set(releases_on_release.get() + 1);
            });
            assert!(active.get());
        }

        assert!(!active.get());
        assert_eq!(releases.get(), 1);
    }
}
