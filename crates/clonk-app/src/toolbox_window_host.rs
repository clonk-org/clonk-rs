//! The developer toolbox window's surface lifecycle.
//!
//! This is the port's `C4DevmodeDlg` — one shared utility window hosting the
//! Tools and Property pages as a tabless notebook (`C4DevmodeDlg.cpp:28-121`).
//! The *decisions* about it live in [`crate::developer_toolbox`]: lazy
//! creation, hide-rather-than-destroy, the remembered position and the
//! page-derived title. Only the pixels are here.
//!
//! It follows the console viewport window's shape rather than the shell's: a
//! plain `Pixels` surface blitted from a software `Surface`, with no retained
//! GPU renderer, because nothing the toolbox draws is a game scene. Unlike a
//! viewport it has no application scale to divide by — the pages are laid out
//! in window pixels — so buffer and surface stay the same extent.

use crate::developer_windows::{DeveloperWindowHost, DeveloperWindowPresenter, ToolboxPage};
use crate::software_window::{build_software_window, SoftwarePresent, SoftwareWindow};
use crate::GameApp;
use winit::event_loop::ActiveEventLoop;

/// The notebook's window. Everything about its surface is
/// [`SoftwareWindow`]'s; what is here is what makes it the *toolbox*.
pub struct ToolboxWindowHost {
    pub(crate) surface: SoftwareWindow,
}

/// Create the toolbox window and its framebuffer.
///
/// `C4DevmodeDlg::AddPage` makes it resizable, a utility type hint, role
/// `"toolbox"`, transient for the console and centred on it
/// (`C4DevmodeDlg.cpp:63-68`, recorded in
/// [`crate::developer_toolbox::ToolboxChrome`]). The X11-specific winit
/// extension applies the utility type when that backend is active. There is no
/// cross-platform equivalent for the arbitrary window role, transient-for
/// relation or centre-on-parent operation for a second top-level window. Those
/// three pieces are an accepted platform limitation here; the port deliberately
/// keeps an ordinary resizable window rather than substituting always-on-top or
/// a child window, which would change the window's lifetime and placement
/// semantics. It is positioned from the remembered coordinates when there are
/// any. `Window::focus_window` is still requested on every show and reopen;
/// winit documents that request as unsupported on Wayland, where the compositor
/// retains focus authority.
pub(crate) fn build_toolbox_window(
    target: &ActiveEventLoop,
    title: &str,
    chrome: crate::developer_toolbox::ToolboxChrome,
    position: Option<(i32, i32)>,
    width: u32,
    height: u32,
) -> anyhow::Result<ToolboxWindowHost> {
    Ok(ToolboxWindowHost {
        surface: build_software_window(
            target,
            title,
            chrome.resizable,
            chrome.utility,
            position,
            width,
            height,
            "developer toolbox",
        )?,
    })
}

impl ToolboxWindowHost {
    /// The extent the page is laid out and hit-tested in.
    pub(crate) fn surface_extent(&self) -> (u32, u32) {
        self.surface.surface_extent()
    }

    /// The window's live position, which `SwitchPage` reads *before* hiding so
    /// the next show can restore it (`C4DevmodeDlg.cpp:91-115`).
    pub(crate) fn position(&self) -> Option<(i32, i32)> {
        self.surface.position()
    }
}

impl DeveloperWindowHost for ToolboxWindowHost {
    fn resize(&mut self, width: u32, height: u32) {
        self.surface.resize(width, height);
    }

    fn request_redraw(&mut self) {
        self.surface.request_redraw();
    }

    fn focus_window(&mut self) {
        self.surface.focus_window();
    }

    fn set_visible(&mut self, visible: bool) {
        self.surface.set_visible(visible);
    }

    fn visible(&self) -> bool {
        self.surface.visible()
    }
}

impl DeveloperWindowPresenter<GameApp> for ToolboxWindowHost {
    /// Draw whichever page the notebook currently shows.
    ///
    /// A hidden window still holds its pages — that is the whole point of
    /// `SwitchPage(nullptr)` returning `TRUE` from `delete-event` — so a
    /// present with no current page blanks rather than tearing the record
    /// down.
    fn present(&mut self, app: &mut GameApp) -> Result<(), String> {
        let (width, height) = self.surface.surface_extent();
        let page = app
            .developer_toolbox
            .current_page()
            .map(|page| app.render_developer_toolbox_page(page, width, height));
        match self.surface.present_surface(page.as_ref())? {
            SoftwarePresent::Presented | SoftwarePresent::Skipped => Ok(()),
            // The pages survive a lost surface, so the window hides and waits
            // rather than being destroyed with them. The *model* has to learn
            // that too: it would otherwise still believe the toolbox visible,
            // answer the next `switch_page` with a bare retitle, and leave a
            // window nobody can bring back.
            SoftwarePresent::SurfaceLost => {
                let effect = app.developer_toolbox.close(None);
                app.developer_toolbox_effects.extend(effect);
                Err("the developer toolbox surface was lost".to_owned())
            }
        }
    }
}

/// Which page the toolbox shows for a cursor mode (`C4EditCursor::OpenPropTools`,
/// `C4EditCursor.cpp:361-374`).
///
/// Draw mode opens the Tools page; Edit and Play both open Property. There is
/// no fourth arm, and no mode opens neither.
pub(crate) fn prop_tools_page(mode: clonk_engine::developer_cursor::CursorMode) -> ToolboxPage {
    use clonk_engine::developer_cursor::CursorMode;

    match mode {
        CursorMode::Draw => ToolboxPage::Tools,
        CursorMode::Edit | CursorMode::Play => ToolboxPage::Property,
    }
}

#[cfg(all(
    test,
    any(not(feature = "app-test-shard-mode"), feature = "app-test-shard-5",),
))]
mod tests {
    use super::*;
    use clonk_engine::developer_cursor::CursorMode;

    // C4EditCursor.cpp:361-374 — the switch has two arms, and Play shares
    // Edit's rather than opening the tools.
    #[test]
    fn open_prop_tools_picks_the_page_by_cursor_mode() {
        assert_eq!(prop_tools_page(CursorMode::Draw), ToolboxPage::Tools);
        assert_eq!(prop_tools_page(CursorMode::Edit), ToolboxPage::Property);
        assert_eq!(prop_tools_page(CursorMode::Play), ToolboxPage::Property);
    }
}
