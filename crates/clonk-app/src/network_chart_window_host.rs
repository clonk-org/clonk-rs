//! The console network-chart window's surface lifecycle.
//!
//! `C4ChartDialog` reaches `Dialog::Show` without `fViewportDlg`, so the
//! console arm gives it a real child window of the console with its own
//! rendering context (`C4GuiDialogs.cpp:305-320,659-661`). `Dialog::Close`
//! destroys that window again (`:677`), so — like the scoreboard and the
//! object list — closing **destroys** rather than hides.
//!
//! The C++ window style is `WS_VISIBLE | WS_POPUP | WS_SYSMENU | WS_CAPTION |
//! WS_MINIMIZEBOX` (`C4GuiDialogs.cpp:56`): a titled popup with no
//! `WS_THICKFRAME`. The chart's bounds are the fixed
//! `NETWORK_CHART_DIALOG_WIDTH`/`_HEIGHT`, so unlike the scoreboard it never
//! grows — `Dialog::UpdateSize` (`:445-473`) has nothing to report — and the
//! reconcile pass only has to follow the caption.

use crate::developer_windows::{DeveloperWindowHost, DeveloperWindowPresenter};
use crate::software_window::{build_software_window, SoftwarePresent, SoftwareWindow};
use crate::GameApp;
use winit::event_loop::ActiveEventLoop;

/// `C4ChartDialog::GetID()` (`C4Network2Dialogs.h:285`), which is what names
/// the dialog's remembered geometry entry.
pub(crate) const NETWORK_CHART_DIALOG_ID: &str = "ChartDialog";

pub struct NetworkChartWindowHost {
    pub(crate) surface: SoftwareWindow,
}

/// Create the chart window and its framebuffer at the dialog's own size.
pub(crate) fn build_network_chart_window(
    target: &ActiveEventLoop,
    title: &str,
    width: u32,
    height: u32,
    position: Option<(i32, i32)>,
) -> anyhow::Result<NetworkChartWindowHost> {
    Ok(NetworkChartWindowHost {
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
            "console network chart",
        )?,
    })
}

impl NetworkChartWindowHost {
    pub(crate) fn surface_extent(&self) -> (u32, u32) {
        self.surface.surface_extent()
    }

    /// Where the window is now, for the entry stored when it closes.
    pub(crate) fn position(&self) -> Option<(i32, i32)> {
        self.surface.position()
    }

    /// Follow the dialog's caption. The extent is fixed, so a resize request
    /// only ever restores a size the window manager changed behind us.
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

impl DeveloperWindowHost for NetworkChartWindowHost {
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

impl DeveloperWindowPresenter<GameApp> for NetworkChartWindowHost {
    /// Draw the live chart.
    ///
    /// The graphs are resampled on every pass, which is what `C4ChartDialog`
    /// does too: its `Update` refreshes the sheets from the live graph owner
    /// rather than retaining a copy.
    fn present(&mut self, app: &mut GameApp) -> Result<(), String> {
        let (width, height) = self.surface.surface_extent();
        let surface = app.render_console_network_chart(width, height);
        match self.surface.present_surface(surface.as_ref())? {
            SoftwarePresent::Presented | SoftwarePresent::Skipped => Ok(()),
            // The chart is a view of state the console still owns, so a lost
            // surface closes the window exactly as its own close button would.
            SoftwarePresent::SurfaceLost => {
                Err("the console network chart surface was lost".to_owned())
            }
        }
    }
}
