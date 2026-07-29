//! Loader progress mirrored to the platform taskbar.
//!
//! `C4Game::SetInitProgress` forwards each *increasing* initialization
//! percentage to the application window (`C4Game.cpp:4094-4106`), and
//! `CStdWindow::SetProgress` publishes it (`StdWindow.cpp:183-196`):
//!
//! ```c
//! if (progress == 100) taskBarList->SetProgressState(hWindow, TBPF_NOPROGRESS);
//! else { taskBarList->SetProgressState(hWindow, TBPF_INDETERMINATE);
//!        taskBarList->SetProgressValue(hWindow, progress, 100); }
//! ```
//!
//! Entering startup calls `SetProgress(100)`, which clears the indicator
//! (`C4Application.cpp:422-426`). The SDL and X11 backends are deliberate
//! no-ops, so the sink is injected rather than assumed.

/// One call to the platform taskbar, as `CStdWindow::SetProgress` issues it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskbarProgressUpdate {
    /// `TBPF_NOPROGRESS` — the indicator is removed (`StdWindow.cpp:189`).
    Clear,
    /// `TBPF_INDETERMINATE` followed by `SetProgressValue(value, 100)`
    /// (`StdWindow.cpp:193-194`). C++ sets both, so both are modelled here.
    Value(u32),
}

/// The platform backend. Implementations must treat failure as non-fatal:
/// C++ simply skips the call when `taskBarList` is null (`StdWindow.cpp:185`).
pub trait TaskbarProgressSink {
    fn apply(&mut self, update: TaskbarProgressUpdate);
}

/// The no-op backend used wherever the platform has no taskbar progress, which
/// is what the SDL and X11 `CStdWindow` implementations do.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoTaskbarProgress;

impl TaskbarProgressSink for NoTaskbarProgress {
    fn apply(&mut self, _update: TaskbarProgressUpdate) {}
}

/// Mirrors loader progress onto `sink`, applying C++'s monotone gate.
#[derive(Clone, Copy, Debug)]
pub struct LoaderTaskbarProgress<S> {
    sink: S,
    /// `C4Game::LastInitProgress` (`C4Game.cpp:4094-4096`).
    last_progress: u32,
}

impl<S: TaskbarProgressSink> LoaderTaskbarProgress<S> {
    pub fn new(sink: S) -> Self {
        Self {
            sink,
            last_progress: 0,
        }
    }

    /// `C4Game::SetInitProgress` — only a strictly increasing percentage is
    /// forwarded (`C4Game.cpp:4094-4106`).
    pub fn report(&mut self, progress: u32) {
        if progress <= self.last_progress {
            return;
        }
        self.last_progress = progress;
        self.sink.apply(update_for(progress));
    }

    /// `C4Application`'s entry into startup, which clears the indicator by
    /// reporting a complete load (`C4Application.cpp:422-426`).
    pub fn enter_startup(&mut self) {
        self.last_progress = 0;
        self.sink.apply(TaskbarProgressUpdate::Clear);
    }

    pub fn sink(&self) -> &S {
        &self.sink
    }
}

/// `CStdWindow::SetProgress`'s branch (`StdWindow.cpp:186-195`).
fn update_for(progress: u32) -> TaskbarProgressUpdate {
    if progress == 100 {
        TaskbarProgressUpdate::Clear
    } else {
        TaskbarProgressUpdate::Value(progress)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Default)]
    struct RecordingSink(Vec<TaskbarProgressUpdate>);

    impl TaskbarProgressSink for RecordingSink {
        fn apply(&mut self, update: TaskbarProgressUpdate) {
            self.0.push(update);
        }
    }

    // C4Game.cpp:4094-4106; StdWindow.cpp:183-196; C4Application.cpp:422-426.
    #[test]
    fn windows_loader_progress_updates_and_clears_taskbar_state() {
        let mut progress = LoaderTaskbarProgress::new(RecordingSink::default());

        progress.report(10);
        progress.report(40);
        // Not increasing: C++'s `InitProgress > LastInitProgress` gate drops
        // both a repeat and a regression, so neither reaches the taskbar.
        progress.report(40);
        progress.report(25);
        progress.report(99);
        assert_eq!(
            progress.sink().0,
            vec![
                TaskbarProgressUpdate::Value(10),
                TaskbarProgressUpdate::Value(40),
                TaskbarProgressUpdate::Value(99),
            ]
        );

        // A complete load clears the indicator rather than showing 100%
        // (StdWindow.cpp:188-190).
        progress.report(100);
        assert_eq!(
            progress.sink().0.last(),
            Some(&TaskbarProgressUpdate::Clear)
        );

        // Entering startup clears it again and re-arms the monotone gate, so
        // the next round's early percentages are not swallowed.
        let mut progress = LoaderTaskbarProgress::new(RecordingSink::default());
        progress.report(80);
        progress.enter_startup();
        progress.report(5);
        assert_eq!(
            progress.sink().0,
            vec![
                TaskbarProgressUpdate::Value(80),
                TaskbarProgressUpdate::Clear,
                TaskbarProgressUpdate::Value(5),
            ]
        );
    }

    // The platforms C++ leaves unimplemented must stay silent but still gate.
    #[test]
    fn no_taskbar_backend_accepts_every_update() {
        let mut progress = LoaderTaskbarProgress::new(NoTaskbarProgress);
        progress.report(50);
        progress.report(100);
        progress.enter_startup();
    }
}
