//! The one host type the runner's window registry holds.
//!
//! `DeveloperWindows` is generic over its host so mocks can drive it, but the
//! process needs a single concrete type covering every window it can open. C++
//! does not: `C4Application` owns `Console` directly and viewports live in
//! their own list (`C4GraphicsSystem::Viewports`), each a `C4Viewport` with its
//! own `pWindow`. The port routes them all by `WindowId`, so they must share a
//! type — and they cannot share a *shape*, because the shell owns a retained
//! GPU renderer built from its surface and a viewport window does not.

use crate::developer_windows::{DeveloperWindowHost, DeveloperWindowPresenter};
use crate::shell_window_host::ShellWindowHost;
use crate::toolbox_window_host::ToolboxWindowHost;
use crate::viewport_window_host::ViewportWindowHost;
use crate::GameApp;

pub enum DeveloperHost {
    Shell(ShellWindowHost),
    Viewport(ViewportWindowHost),
    /// The `C4DevmodeDlg` notebook. There is one for the process, and it
    /// outlives every close — only shutdown removes its record.
    Toolbox(ToolboxWindowHost),
}

impl DeveloperHost {
    /// The shell's concrete state. The runner destructures it every pass, so
    /// this is infallible by construction at that call site — the shell record
    /// is inserted before the event loop starts and is never replaced.
    pub fn as_shell_mut(&mut self) -> Option<&mut ShellWindowHost> {
        match self {
            Self::Shell(shell) => Some(shell),
            Self::Viewport(_) | Self::Toolbox(_) => None,
        }
    }

    /// The toolbox's concrete state, for the page it draws and the position it
    /// is asked to remember.
    pub fn as_toolbox_mut(&mut self) -> Option<&mut ToolboxWindowHost> {
        match self {
            Self::Toolbox(toolbox) => Some(toolbox),
            Self::Shell(_) | Self::Viewport(_) => None,
        }
    }

    /// The physical `C4Viewport` identity a viewport window draws, if it is one.
    pub fn viewport_identity(&self) -> Option<u64> {
        match self {
            Self::Viewport(viewport) => Some(viewport.identity),
            Self::Shell(_) | Self::Toolbox(_) => None,
        }
    }

    /// This host's OS window, whichever kind it is.
    pub fn window(&self) -> &winit::window::Window {
        match self {
            Self::Shell(shell) => &shell.window,
            Self::Viewport(viewport) => &viewport.window,
            Self::Toolbox(toolbox) => &toolbox.surface.window,
        }
    }
}

impl DeveloperWindowHost for DeveloperHost {
    fn resize(&mut self, width: u32, height: u32) {
        match self {
            Self::Shell(shell) => shell.resize(width, height),
            Self::Viewport(viewport) => viewport.resize(width, height),
            Self::Toolbox(toolbox) => toolbox.resize(width, height),
        }
    }

    fn request_redraw(&mut self) {
        match self {
            Self::Shell(shell) => shell.request_redraw(),
            Self::Viewport(viewport) => viewport.request_redraw(),
            Self::Toolbox(toolbox) => toolbox.request_redraw(),
        }
    }

    fn set_visible(&mut self, visible: bool) {
        match self {
            Self::Shell(shell) => shell.set_visible(visible),
            Self::Viewport(viewport) => viewport.set_visible(visible),
            Self::Toolbox(toolbox) => toolbox.set_visible(visible),
        }
    }

    fn visible(&self) -> bool {
        match self {
            Self::Shell(shell) => shell.visible(),
            Self::Viewport(viewport) => viewport.visible(),
            Self::Toolbox(toolbox) => toolbox.visible(),
        }
    }
}

impl DeveloperWindowPresenter<GameApp> for DeveloperHost {
    fn present(&mut self, app: &mut GameApp) -> Result<(), String> {
        match self {
            Self::Shell(shell) => shell.present(app),
            Self::Viewport(viewport) => viewport.present(app),
            Self::Toolbox(toolbox) => toolbox.present(app),
        }
    }
}
