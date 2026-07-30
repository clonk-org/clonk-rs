//! The developer toolbox window: one window, one tabless notebook.
//!
//! `C4DevmodeDlg` (`C4DevmodeDlg.cpp:28-121`) is a single shared utility window
//! that hosts the Tools and Property pages. Its behaviour is entirely
//! programmatic — the notebook is created with
//! `gtk_notebook_set_show_tabs(FALSE)`, so a page is never chosen by clicking a
//! tab; the console switches it.
//!
//! Four behaviours are easy to lose in a port, and this module owns all four:
//!
//! - **Closing hides, it never destroys.** The `delete-event` handler calls
//!   `SwitchPage(nullptr)` and returns `TRUE`, which suppresses GTK's own
//!   destroy (`:36-42`). The pages — and everything they hold — survive.
//! - **The window position is remembered across hides.** `SwitchPage` records
//!   `x,y` while the window is still visible and restores them on the next show
//!   (`:91-115`). They are `static`, so the position persists for the whole
//!   process, not just one open/close cycle.
//! - **The window title follows the current page**, taken from that page's
//!   notebook tab label even though the tabs are invisible (`:106`).
//! - **The window is destroyed only when its last page is removed**
//!   (`RemovePage`, `:79-88`) — not when it is closed.
//!
//! Chrome (utility type hint, `"toolbox"` role, transient-for the console,
//! centred on parent) is recorded in [`ToolboxChrome`] so the platform layer
//! applies it rather than inventing its own.

use crate::developer_windows::ToolboxPage;

/// The window hints `C4DevmodeDlg::AddPage` sets on creation
/// (`C4DevmodeDlg.cpp:63-68`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ToolboxChrome {
    pub(crate) resizable: bool,
    /// `GDK_WINDOW_TYPE_HINT_UTILITY` — a tool window, not a normal top level.
    pub(crate) utility: bool,
    /// `gtk_window_set_role(..., "toolbox")`.
    pub(crate) role: &'static str,
    /// `gtk_window_set_transient_for(..., parent)`.
    pub(crate) transient_for_console: bool,
    /// `GTK_WIN_POS_CENTER_ON_PARENT`, used only until a position is
    /// remembered.
    pub(crate) center_on_parent: bool,
}

impl Default for ToolboxChrome {
    fn default() -> Self {
        Self {
            resizable: true,
            utility: true,
            role: "toolbox",
            transient_for_console: true,
            center_on_parent: true,
        }
    }
}

/// What the platform layer should do after a notebook operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ToolboxEffect {
    /// Create the window with this chrome. Emitted for the first page only.
    Create(ToolboxChrome),
    /// Show it, restoring a remembered position when there is one.
    Show {
        page: ToolboxPage,
        title: String,
        position: Option<(i32, i32)>,
    },
    /// Hide it, keeping every page.
    Hide,
    /// Retitle without changing visibility.
    Title(String),
    /// The last page was removed, so the window itself goes.
    Destroy,
}

/// The shared toolbox notebook.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct DeveloperToolbox {
    /// Pages in insertion order, as `gtk_notebook_append_page` builds them.
    pages: Vec<ToolboxPage>,
    current: Option<ToolboxPage>,
    visible: bool,
    created: bool,
    /// The `static` x/y `SwitchPage` remembers. `None` is C++'s `-1, -1`.
    remembered_position: Option<(i32, i32)>,
}

impl DeveloperToolbox {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn pages(&self) -> &[ToolboxPage] {
        &self.pages
    }

    pub(crate) fn current_page(&self) -> Option<ToolboxPage> {
        self.current
    }

    pub(crate) fn visible(&self) -> bool {
        self.visible
    }

    pub(crate) fn remembered_position(&self) -> Option<(i32, i32)> {
        self.remembered_position
    }

    /// The window title for a page — its notebook tab label
    /// (`gtk_notebook_get_tab_label_text`, `C4DevmodeDlg.cpp:106`).
    pub(crate) fn page_title(page: ToolboxPage) -> String {
        match page {
            ToolboxPage::Tools => "Tools".to_owned(),
            ToolboxPage::Property => "Properties".to_owned(),
        }
    }

    /// `C4DevmodeDlg::AddPage` (`:53-77`). The window is created lazily, by the
    /// first page; adding a page does not show it.
    pub(crate) fn add_page(&mut self, page: ToolboxPage) -> Option<ToolboxEffect> {
        if self.pages.contains(&page) {
            return None;
        }
        self.pages.push(page);
        (!std::mem::replace(&mut self.created, true))
            .then(|| ToolboxEffect::Create(ToolboxChrome::default()))
    }

    /// `C4DevmodeDlg::RemovePage` (`:79-88`). The window is destroyed only when
    /// the notebook runs out of pages.
    pub(crate) fn remove_page(&mut self, page: ToolboxPage) -> Option<ToolboxEffect> {
        let index = self.pages.iter().position(|held| *held == page)?;
        self.pages.remove(index);
        if self.current == Some(page) {
            self.current = None;
        }
        if !self.pages.is_empty() {
            return None;
        }
        self.created = false;
        self.visible = false;
        Some(ToolboxEffect::Destroy)
    }

    /// `C4DevmodeDlg::SwitchPage(widget)` (`:90-121`).
    ///
    /// `position` is the window's live position, read *before* anything moves —
    /// C++ calls `gtk_window_get_position` while it is still visible. Passing
    /// `None` (or a hidden window) leaves the remembered position alone.
    pub(crate) fn switch_page(
        &mut self,
        page: ToolboxPage,
        position: Option<(i32, i32)>,
    ) -> Option<ToolboxEffect> {
        if !self.pages.contains(&page) {
            return None;
        }
        self.remember_position(position);
        self.current = Some(page);
        let title = Self::page_title(page);
        if self.visible {
            // Already up: only the page and title change.
            return Some(ToolboxEffect::Title(title));
        }
        self.visible = true;
        Some(ToolboxEffect::Show {
            page,
            title,
            position: self.remembered_position,
        })
    }

    /// `C4DevmodeDlg::SwitchPage(nullptr)` — what the close button does
    /// (`:36-42`, `:116-120`). The pages survive; only the window hides.
    pub(crate) fn close(&mut self, position: Option<(i32, i32)>) -> Option<ToolboxEffect> {
        self.remember_position(position);
        if !self.visible {
            return None;
        }
        self.visible = false;
        Some(ToolboxEffect::Hide)
    }

    /// The position is captured only while the window is actually up, which is
    /// what stops a hidden window's stale coordinates overwriting a good one.
    fn remember_position(&mut self, position: Option<(i32, i32)>) {
        if self.visible {
            if let Some(position) = position {
                self.remembered_position = Some(position);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // C4DevmodeDlg.cpp:28-121 — lazy creation, hide-not-destroy, the remembered
    // position, the page-derived title, and destroy-on-last-page.
    #[test]
    fn developer_toolbox_hides_on_close_and_remembers_its_position() {
        let mut toolbox = DeveloperToolbox::new();
        assert!(!toolbox.visible());
        assert_eq!(toolbox.current_page(), None);

        // The first page creates the window; the second does not, and neither
        // shows it.
        assert_eq!(
            toolbox.add_page(ToolboxPage::Tools),
            Some(ToolboxEffect::Create(ToolboxChrome {
                resizable: true,
                utility: true,
                role: "toolbox",
                transient_for_console: true,
                center_on_parent: true,
            }))
        );
        assert_eq!(toolbox.add_page(ToolboxPage::Property), None);
        assert_eq!(
            toolbox.add_page(ToolboxPage::Tools),
            None,
            "a page is appended once"
        );
        assert!(!toolbox.visible(), "adding a page does not show the window");
        assert_eq!(
            toolbox.pages(),
            &[ToolboxPage::Tools, ToolboxPage::Property]
        );

        // Switching to a page shows the window, titled from that page's
        // (invisible) tab label. No position is remembered yet, so the platform
        // falls back to centre-on-parent.
        assert_eq!(
            toolbox.switch_page(ToolboxPage::Tools, None),
            Some(ToolboxEffect::Show {
                page: ToolboxPage::Tools,
                title: "Tools".to_owned(),
                position: None,
            })
        );
        assert!(toolbox.visible());

        // Switching while already up only changes page and title.
        assert_eq!(
            toolbox.switch_page(ToolboxPage::Property, Some((120, 80))),
            Some(ToolboxEffect::Title("Properties".to_owned()))
        );
        assert_eq!(toolbox.current_page(), Some(ToolboxPage::Property));
        assert_eq!(toolbox.remembered_position(), Some((120, 80)));

        // Closing hides and keeps every page — the `delete-event` handler
        // returns TRUE precisely so GTK does not destroy the window.
        assert_eq!(toolbox.close(Some((140, 90))), Some(ToolboxEffect::Hide));
        assert!(!toolbox.visible());
        assert_eq!(
            toolbox.pages(),
            &[ToolboxPage::Tools, ToolboxPage::Property],
            "hiding must not drop the notebook pages"
        );
        assert_eq!(toolbox.remembered_position(), Some((140, 90)));

        // A second close is a no-op, and cannot overwrite the remembered
        // position with a hidden window's coordinates.
        assert_eq!(toolbox.close(Some((0, 0))), None);
        assert_eq!(toolbox.remembered_position(), Some((140, 90)));

        // Re-showing restores that position rather than re-centring.
        assert_eq!(
            toolbox.switch_page(ToolboxPage::Tools, None),
            Some(ToolboxEffect::Show {
                page: ToolboxPage::Tools,
                title: "Tools".to_owned(),
                position: Some((140, 90)),
            })
        );

        // Removing one of two pages keeps the window; removing the last
        // destroys it.
        assert_eq!(toolbox.remove_page(ToolboxPage::Property), None);
        assert_eq!(
            toolbox.remove_page(ToolboxPage::Property),
            None,
            "removing an absent page does nothing"
        );
        assert_eq!(
            toolbox.remove_page(ToolboxPage::Tools),
            Some(ToolboxEffect::Destroy)
        );
        assert!(toolbox.pages().is_empty());
        assert!(!toolbox.visible());
        assert_eq!(toolbox.current_page(), None);

        // Switching to a page the notebook does not hold is refused rather than
        // creating one implicitly.
        assert_eq!(toolbox.switch_page(ToolboxPage::Tools, None), None);
    }
}
