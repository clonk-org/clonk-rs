//! The console scoreboard window's surface lifecycle.
//!
//! `C4ScoreboardDlg` is built with `fViewportDlg = false`
//! (`C4Scoreboard.cpp:292`), so `Dialog::Show` takes the console arm and gives
//! it a real child window of the console with its own rendering context
//! (`C4GuiDialogs.cpp:305-320,659-661`). `Dialog::Close` destroys that window
//! again (`:677`), so — like the object list and unlike the toolbox — closing
//! **destroys** rather than hides.
//!
//! The C++ window style is `WS_VISIBLE | WS_POPUP | WS_SYSMENU | WS_CAPTION |
//! WS_MINIMIZEBOX` (`C4GuiDialogs.cpp:56`): a titled popup with no
//! `WS_THICKFRAME`, so the player cannot resize it. Its size comes from
//! `C4ScoreboardDlg::Update` through `Dialog::UpdateSize` (`:445-473`)
//! instead, which is why the reconcile pass resizes it to follow live
//! `SetScoreboardData`.

use crate::developer_windows::{DeveloperWindowHost, DeveloperWindowPresenter};
use crate::software_window::{build_software_window, SoftwarePresent, SoftwareWindow};
use crate::GameApp;
use winit::event_loop::ActiveEventLoop;

/// `C4ScoreboardDlg::GetID()` (`C4Scoreboard.h:107`), which is what names the
/// dialog's remembered geometry entry.
pub(crate) const SCOREBOARD_DIALOG_ID: &str = "Scoreboard";

pub struct ScoreboardWindowHost {
    pub(crate) surface: SoftwareWindow,
}

/// Create the scoreboard window and its framebuffer at the dialog's own size.
pub(crate) fn build_scoreboard_window(
    target: &ActiveEventLoop,
    title: &str,
    width: u32,
    height: u32,
    position: Option<(i32, i32)>,
) -> anyhow::Result<ScoreboardWindowHost> {
    Ok(ScoreboardWindowHost {
        surface: build_software_window(
            target,
            title,
            // No `WS_THICKFRAME`: the dialog sizes itself.
            false,
            false,
            // `CStdWindow::RestorePosition` places the window from the stored
            // entry as it is created (`StdRegistry.cpp:300-327`).
            position,
            width,
            height,
            "console scoreboard",
        )?,
    })
}

impl ScoreboardWindowHost {
    pub(crate) fn surface_extent(&self) -> (u32, u32) {
        self.surface.surface_extent()
    }

    /// Where the window is now, for the entry stored when it closes.
    pub(crate) fn position(&self) -> Option<(i32, i32)> {
        self.surface.position()
    }

    /// Follow `Dialog::UpdateSize`, which resizes the console window whenever
    /// the dialog's own bounds change (`C4GuiDialogs.cpp:471`).
    pub(crate) fn set_chrome(&mut self, title: &str, width: u32, height: u32) {
        self.surface.window.set_title(title);
        if self.surface_extent() != (width, height) {
            let _ = self
                .surface
                .window
                .request_inner_size(winit::dpi::PhysicalSize::new(width, height));
        }
    }
}

impl DeveloperWindowHost for ScoreboardWindowHost {
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

impl DeveloperWindowPresenter<GameApp> for ScoreboardWindowHost {
    /// Draw the live board.
    ///
    /// `Dialog::Draw` clears the separate window and then draws the dialog
    /// into it (`C4GuiDialogs.cpp:479-489`). A board that cannot be laid out
    /// has no dialog either, so nothing is presented and the surface keeps its
    /// previous contents until the reconcile pass withdraws the window.
    fn present(&mut self, app: &mut GameApp) -> Result<(), String> {
        let (width, height) = self.surface.surface_extent();
        let surface = app.render_console_scoreboard(width, height);
        match self.surface.present_surface(surface.as_ref())? {
            SoftwarePresent::Presented | SoftwarePresent::Skipped => Ok(()),
            // The board is a view of state the console still owns, so a lost
            // surface closes the window exactly as its own close button would.
            SoftwarePresent::SurfaceLost => {
                Err("the console scoreboard surface was lost".to_owned())
            }
        }
    }
}
