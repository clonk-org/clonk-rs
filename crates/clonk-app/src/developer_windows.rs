//! Registry of developer windows keyed by platform window id.
//!
//! The runner owns exactly one Window/Pixels/FramePresenter tuple, so console
//! viewports, the Tools/Property toolbox and the object-list utility window had
//! nowhere to live. C++ gives each viewport its own window and context
//! (`C4Viewport.cpp:775-834`), hosts Tools/Property as switchable pages of one
//! hidden-tab notebook (`C4DevmodeDlg.cpp:50-121`), and gives the object list a
//! separate utility window (`C4ObjectListDlg.cpp:726-787`).
//!
//! This registry is generic over [`DeveloperWindowHost`] so the routing can be
//! exercised without a real window; the app supplies the live implementation.

use std::collections::HashMap;

/// The console shell's fixed key. winit's `WindowId` is opaque and cannot be
/// known before the window exists, and the shell is a singleton for the
/// process, so it gets a reserved id rather than a derived one.
pub const SHELL_WINDOW: WindowId = WindowId(0);

/// A platform window id. The app maps its own window handles onto these.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct WindowId(pub u64);

/// Which toolbox page a notebook host currently shows
/// (`C4DevmodeDlg::SwitchPage`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolboxPage {
    Tools,
    Property,
}

/// What a record is for. The purpose decides its close semantics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostPurpose {
    /// The console shell. Closing a child must never destroy or exit it.
    Shell,
    /// A `C4ViewportWindow`, destroyed when closed.
    Viewport { viewport: u32 },
    /// The `C4DevmodeDlg` notebook, which hides rather than being destroyed so
    /// its pages survive (`C4DevmodeDlg.cpp:79-101`).
    Toolbox { page: ToolboxPage },
    /// The object-list utility window.
    ObjectList,
}

/// One window's surface lifecycle. Implementations own the real window, pixel
/// buffer and presenter; the registry only routes.
pub trait DeveloperWindowHost {
    fn resize(&mut self, width: u32, height: u32);
    /// Marks the host as needing a redraw before the next present.
    fn request_redraw(&mut self);
    fn set_visible(&mut self, visible: bool);
    fn visible(&self) -> bool;
}

/// Drawing a developer window needs the state it draws.
///
/// C++ gets this for free: `C4Viewport::Execute` reads the global `Game`, so a
/// viewport appears to present itself. The port passes that state explicitly,
/// and it is not the same for every purpose — the console shell renders from
/// `GameApp` through the retained GPU pipeline, while a mock needs nothing —
/// so the context is a parameter rather than baked into the host.
pub trait DeveloperWindowPresenter<Ctx>: DeveloperWindowHost {
    /// Presents the pending frame. An error is reported against this host only.
    fn present(&mut self, context: &mut Ctx) -> Result<(), String>;
}

struct Record<H> {
    host: H,
    purpose: HostPurpose,
}

/// The keyed registry.
pub struct DeveloperWindows<H> {
    records: HashMap<WindowId, Record<H>>,
}

impl<H> Default for DeveloperWindows<H> {
    fn default() -> Self {
        Self {
            records: HashMap::new(),
        }
    }
}

impl<H: DeveloperWindowHost> DeveloperWindows<H> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `host` under `id`, replacing any previous record for it.
    pub fn insert(&mut self, id: WindowId, purpose: HostPurpose, host: H) {
        self.records.insert(id, Record { host, purpose });
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub fn contains(&self, id: WindowId) -> bool {
        self.records.contains_key(&id)
    }

    pub fn purpose(&self, id: WindowId) -> Option<HostPurpose> {
        self.records.get(&id).map(|record| record.purpose)
    }

    pub fn host(&self, id: WindowId) -> Option<&H> {
        self.records.get(&id).map(|record| &record.host)
    }

    pub fn host_mut(&mut self, id: WindowId) -> Option<&mut H> {
        self.records.get_mut(&id).map(|record| &mut record.host)
    }

    /// The console shell's host. Present for the whole process once the runner
    /// has registered it.
    pub fn shell_mut(&mut self) -> Option<&mut H> {
        self.host_mut(SHELL_WINDOW)
    }

    /// Every registered key, in no particular order.
    pub fn keys(&self) -> impl Iterator<Item = WindowId> + '_ {
        self.records.keys().copied()
    }

    /// The key of the first record whose host satisfies `predicate`.
    ///
    /// The registry stays platform-agnostic: the runner resolves a winit
    /// `WindowId` by comparing the host's own window, which only the runner
    /// knows how to reach.
    pub fn find_key(&self, predicate: impl Fn(&H) -> bool) -> Option<WindowId> {
        self.records
            .iter()
            .find(|(_, record)| predicate(&record.host))
            .map(|(id, _)| *id)
    }

    /// Routes a resize to one record. Unknown ids are ignored, which is what an
    /// event for an already-closed window is.
    pub fn resize(&mut self, id: WindowId, width: u32, height: u32) -> bool {
        self.records
            .get_mut(&id)
            .map(|record| record.host.resize(width, height))
            .is_some()
    }

    /// Requests a redraw of one record.
    pub fn request_redraw(&mut self, id: WindowId) -> bool {
        self.records
            .get_mut(&id)
            .map(|record| record.host.request_redraw())
            .is_some()
    }

    /// The graphics deadline: every *visible* host is asked to redraw. Hidden
    /// toolboxes are skipped, as a hidden GTK window draws nothing.
    pub fn request_redraw_visible(&mut self) -> usize {
        self.records
            .values_mut()
            .filter(|record| record.host.visible())
            .map(|record| record.host.request_redraw())
            .count()
    }

    /// Presents every visible host, returning each failure against its own id.
    /// One host's failure never suppresses or is attributed to another.
    pub fn present_visible<Ctx>(&mut self, context: &mut Ctx) -> Vec<(WindowId, String)>
    where
        H: DeveloperWindowPresenter<Ctx>,
    {
        let mut failures: Vec<(WindowId, String)> = self
            .records
            .iter_mut()
            .filter(|(_, record)| record.host.visible())
            .filter_map(|(id, record)| record.host.present(context).err().map(|error| (*id, error)))
            .collect();
        // Deterministic order for callers that log or test them.
        failures.sort_by_key(|(id, _)| *id);
        failures
    }

    /// Hides a record without destroying it. This is the toolbox's close
    /// behaviour (`C4DevmodeDlg.cpp:79-101`).
    pub fn hide(&mut self, id: WindowId) -> bool {
        self.records
            .get_mut(&id)
            .map(|record| record.host.set_visible(false))
            .is_some()
    }

    /// Switches the toolbox's page identity. Returns false for a record that is
    /// not a toolbox.
    pub fn switch_page(&mut self, id: WindowId, page: ToolboxPage) -> bool {
        match self.records.get_mut(&id) {
            Some(record) => match &mut record.purpose {
                HostPurpose::Toolbox { page: current } => {
                    *current = page;
                    true
                }
                _ => false,
            },
            None => false,
        }
    }

    /// Closes a record. Viewports and the object list are destroyed; the
    /// toolbox is only hidden so its pages survive; the shell is never removed
    /// by this path — a child close must not take the console down with it.
    pub fn close(&mut self, id: WindowId) -> CloseOutcome {
        let Some(record) = self.records.get_mut(&id) else {
            return CloseOutcome::Unknown;
        };
        match record.purpose {
            HostPurpose::Shell => CloseOutcome::ShellRetained,
            HostPurpose::Toolbox { .. } => {
                record.host.set_visible(false);
                CloseOutcome::Hidden
            }
            HostPurpose::Viewport { .. } | HostPurpose::ObjectList => {
                self.records.remove(&id);
                CloseOutcome::Destroyed
            }
        }
    }
}

/// What [`DeveloperWindows::close`] did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CloseOutcome {
    /// The record was removed.
    Destroyed,
    /// The record survives, hidden.
    Hidden,
    /// The shell ignores a child-style close.
    ShellRetained,
    /// No such record.
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Default)]
    struct MockHost {
        size: (u32, u32),
        redraws: usize,
        presents: usize,
        visible: bool,
        fail_present: Option<String>,
    }

    impl MockHost {
        fn shown() -> Self {
            Self {
                visible: true,
                ..Self::default()
            }
        }
    }

    impl DeveloperWindowHost for MockHost {
        fn resize(&mut self, width: u32, height: u32) {
            self.size = (width, height);
        }

        fn request_redraw(&mut self) {
            self.redraws += 1;
        }

        fn set_visible(&mut self, visible: bool) {
            self.visible = visible;
        }

        fn visible(&self) -> bool {
            self.visible
        }
    }

    impl DeveloperWindowPresenter<()> for MockHost {
        fn present(&mut self, _context: &mut ()) -> Result<(), String> {
            self.presents += 1;
            self.fail_present.clone().map_or(Ok(()), Err)
        }
    }

    // The production key, so the routing is tested against the id the runner
    // actually registers the console shell under.
    const SHELL: WindowId = SHELL_WINDOW;
    const VIEWPORT: WindowId = WindowId(2);
    const TOOLBOX: WindowId = WindowId(3);

    fn registry() -> DeveloperWindows<MockHost> {
        let mut windows = DeveloperWindows::new();
        windows.insert(SHELL, HostPurpose::Shell, MockHost::shown());
        windows.insert(
            VIEWPORT,
            HostPurpose::Viewport { viewport: 0 },
            MockHost::shown(),
        );
        windows.insert(
            TOOLBOX,
            HostPurpose::Toolbox {
                page: ToolboxPage::Tools,
            },
            MockHost::shown(),
        );
        windows
    }

    // C4Viewport.cpp:775-834; C4DevmodeDlg.cpp:50-121; C4ObjectListDlg.cpp:726-787
    // — events address one record, viewports destroy, the toolbox hides and
    // switches page, and a child close never takes the shell down.
    /// The live console shell must satisfy the same contract as the mocks —
    /// including `present`, which needs `GameApp` and therefore could not be
    /// implemented at all before the presenter trait took a context. This is a
    /// compile-time check; constructing a real window needs an event loop.
    #[allow(dead_code)]
    fn shell_host_is_a_live_presenter(
        host: &mut crate::shell_window_host::ShellWindowHost,
        app: &mut crate::GameApp,
    ) {
        host.resize(640, 480);
        host.request_redraw();
        host.set_visible(host.visible());
        let _: Result<(), String> = host.present(app);
    }

    #[test]
    fn developer_window_host_routes_resize_redraw_hide_and_close_by_window_id() {
        let mut windows = registry();
        assert_eq!(windows.len(), 3);

        // The shell is addressable by its reserved key, which is how the runner
        // reaches its own surfaces now that they live in a record.
        assert!(windows.shell_mut().is_some());
        assert_eq!(windows.purpose(SHELL_WINDOW), Some(HostPurpose::Shell));

        // A resize reaches only its own record.
        assert!(windows.resize(VIEWPORT, 640, 480));
        assert_eq!(windows.host(VIEWPORT).expect("viewport").size, (640, 480));
        assert_eq!(windows.host(SHELL).expect("shell").size, (0, 0));
        assert_eq!(windows.host(TOOLBOX).expect("toolbox").size, (0, 0));
        // An event for an unknown window is ignored rather than misrouted.
        assert!(!windows.resize(WindowId(99), 1, 1));

        // A targeted redraw hits one host; the deadline hits every visible one.
        assert!(windows.request_redraw(TOOLBOX));
        assert_eq!(windows.host(TOOLBOX).expect("toolbox").redraws, 1);
        assert_eq!(windows.host(SHELL).expect("shell").redraws, 0);
        assert_eq!(windows.request_redraw_visible(), 3);
        assert_eq!(windows.host(SHELL).expect("shell").redraws, 1);

        // Hiding removes a host from the deadline without destroying it.
        assert!(windows.hide(TOOLBOX));
        assert!(!windows.host(TOOLBOX).expect("toolbox").visible());
        assert_eq!(windows.request_redraw_visible(), 2);
        assert!(windows.contains(TOOLBOX), "hiding must not destroy");

        // The toolbox switches page identity; other purposes refuse.
        assert!(windows.switch_page(TOOLBOX, ToolboxPage::Property));
        assert_eq!(
            windows.purpose(TOOLBOX),
            Some(HostPurpose::Toolbox {
                page: ToolboxPage::Property
            })
        );
        assert!(!windows.switch_page(VIEWPORT, ToolboxPage::Tools));
        assert!(!windows.switch_page(WindowId(99), ToolboxPage::Tools));

        // One host's presentation failure is reported against that host alone.
        windows.host_mut(VIEWPORT).expect("viewport").fail_present =
            Some("surface lost".to_owned());
        let failures = windows.present_visible(&mut ());
        assert_eq!(failures, vec![(VIEWPORT, "surface lost".to_owned())]);
        // The other visible host still presented.
        assert_eq!(windows.host(SHELL).expect("shell").presents, 1);
        // The hidden toolbox was not presented at all.
        assert_eq!(windows.host(TOOLBOX).expect("toolbox").presents, 0);

        // Closing a viewport destroys it; closing the toolbox only hides it;
        // closing the shell leaves the console standing.
        assert_eq!(windows.close(VIEWPORT), CloseOutcome::Destroyed);
        assert!(!windows.contains(VIEWPORT));
        assert_eq!(windows.close(TOOLBOX), CloseOutcome::Hidden);
        assert!(windows.contains(TOOLBOX));
        assert_eq!(windows.close(SHELL), CloseOutcome::ShellRetained);
        assert!(
            windows.contains(SHELL),
            "a child-style close must never destroy the console shell"
        );
        assert_eq!(windows.close(WindowId(99)), CloseOutcome::Unknown);
        assert_eq!(windows.len(), 2);
    }
}
