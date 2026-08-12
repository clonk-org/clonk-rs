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
use crate::GameApp;
use pixels::Pixels;
use std::sync::Arc;
use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

pub struct ToolboxWindowHost {
    pub window: Arc<Window>,
    pub pixels: Option<Pixels<'static>>,
    /// The last window-local pointer position. winit reports motion and button
    /// state separately, and the page's hit test needs both.
    pub(crate) last_pointer: (i32, i32),
    width: u32,
    height: u32,
    surface_rebuild: crate::main_audio::SurfaceRebuildState,
    visible: bool,
}

/// Create the toolbox window and its framebuffer.
///
/// `C4DevmodeDlg::AddPage` makes it resizable, a utility type hint, role
/// `"toolbox"`, transient for the console and centred on it
/// (`C4DevmodeDlg.cpp:63-68`, recorded in
/// [`crate::developer_toolbox::ToolboxChrome`]). winit carries only the first
/// of those five: it has no utility hint, no window role, and no
/// transient-for or centre-on-parent for a second top level. The window is
/// therefore an ordinary resizable one, positioned from the remembered
/// coordinates when there are any.
pub(crate) fn build_toolbox_window(
    target: &ActiveEventLoop,
    title: &str,
    chrome: crate::developer_toolbox::ToolboxChrome,
    position: Option<(i32, i32)>,
    width: u32,
    height: u32,
) -> anyhow::Result<ToolboxWindowHost> {
    use anyhow::Context;

    let mut attributes = Window::default_attributes()
        .with_title(title)
        .with_inner_size(winit::dpi::PhysicalSize::new(width, height))
        .with_resizable(chrome.resizable);
    if let Some((x, y)) = position {
        attributes = attributes.with_position(winit::dpi::PhysicalPosition::new(x, y));
    }
    let window = Arc::new(
        target
            .create_window(attributes)
            .context("failed to create the developer toolbox window")?,
    );
    let pixels = crate::main_audio::build_framebuffer(&window, window.inner_size())
        .context("failed to create the developer toolbox framebuffer")?;
    Ok(ToolboxWindowHost::new(window, pixels, width, height))
}

impl ToolboxWindowHost {
    pub fn new(window: Arc<Window>, pixels: Pixels<'static>, width: u32, height: u32) -> Self {
        Self {
            window,
            pixels: Some(pixels),
            last_pointer: (0, 0),
            width: width.max(1),
            height: height.max(1),
            surface_rebuild: crate::main_audio::SurfaceRebuildState::default(),
            visible: true,
        }
    }

    /// The extent the page is laid out and hit-tested in.
    pub(crate) fn surface_extent(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// The window's live position, which `SwitchPage` reads *before* hiding so
    /// the next show can restore it (`C4DevmodeDlg.cpp:91-115`).
    pub(crate) fn position(&self) -> Option<(i32, i32)> {
        self.window
            .outer_position()
            .ok()
            .map(|position| (position.x, position.y))
    }
}

impl DeveloperWindowHost for ToolboxWindowHost {
    fn resize(&mut self, width: u32, height: u32) {
        self.width = width.max(1);
        self.height = height.max(1);
        if let Some(pixels) = self.pixels.as_mut() {
            let _ = pixels.resize_surface(self.width, self.height);
            let _ = pixels.resize_buffer(self.width, self.height);
        }
    }

    fn request_redraw(&mut self) {
        self.window.request_redraw();
    }

    fn set_visible(&mut self, visible: bool) {
        self.window.set_visible(visible);
        self.visible = visible;
    }

    fn visible(&self) -> bool {
        self.visible
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
        let page = app.developer_toolbox.current_page();
        let Some(pixels) = self.pixels.as_mut() else {
            return Ok(());
        };
        match page {
            Some(page) => {
                let surface = app.render_developer_toolbox_page(page, self.width, self.height);
                let frame = pixels.frame_mut();
                let drawn = surface.pixels();
                if frame.len() == drawn.len() {
                    frame.copy_from_slice(drawn);
                } else {
                    // A resize the buffer has not caught up with; showing the
                    // previous frame beats tearing.
                    tracing::trace!("toolbox frame size does not match its buffer yet");
                }
            }
            None => pixels.frame_mut().fill(0),
        }
        match crate::main_audio::present_pixels_frame(pixels) {
            Ok(crate::main_audio::RetainedGpuPresentOutcome::Presented) => {
                self.surface_rebuild.note_presented();
                Ok(())
            }
            Ok(crate::main_audio::RetainedGpuPresentOutcome::Skipped) => Ok(()),
            Err(pixels::Error::SurfaceLost) => {
                // The pages survive a lost surface, so the window hides and
                // waits rather than being destroyed with them.
                let _ = self.surface_rebuild.note_loss();
                self.window.set_visible(false);
                self.visible = false;
                Err("the developer toolbox surface was lost".to_owned())
            }
            Err(error) => Err(error.to_string()),
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
