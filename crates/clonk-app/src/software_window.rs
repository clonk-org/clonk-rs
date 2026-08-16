//! A developer window that is nothing but a software surface.
//!
//! The toolbox and the object list are the same object twice over: an
//! ordinary winit window, a `Pixels` framebuffer at the window's own extent,
//! and a `clonk_graphics::Surface` blitted into it once per redraw. Neither
//! draws a game scene, so neither carries a `RetainedGpuRenderer` the way the
//! console shell does, and neither divides by an application scale the way a
//! viewport does — their contents are laid out in window pixels.
//!
//! C++ needs no equivalent: `C4DevmodeDlg` and `C4ObjectListDlg` are GTK
//! widget trees, and the toolkit owns their pixels. What the port shares
//! between them is exactly this surface lifecycle, so it lives here rather
//! than being written twice.

use crate::developer_windows::DeveloperWindowHost;
use clonk_surface::WindowSurface;
use std::sync::Arc;
use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

/// One software-drawn developer window.
pub struct SoftwareWindow {
    pub window: Arc<Window>,
    pub pixels: Option<WindowSurface>,
    /// The last window-local pointer position. winit reports motion and button
    /// state in separate events, and every hit test needs both.
    pub(crate) last_pointer: (i32, i32),
    width: u32,
    height: u32,
    surface_rebuild: crate::main_audio::SurfaceRebuildState,
    visible: bool,
}

/// What a present did, so the caller can decide what a lost surface costs it —
/// the toolbox keeps its pages and hides, the object list is destroyed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SoftwarePresent {
    Presented,
    Skipped,
    /// `clonk_surface::SurfaceError::SurfaceLost`. The window has already been hidden.
    SurfaceLost,
}

/// Create a window and its framebuffer.
///
/// `position` places it where a previous session left it; without one the
/// platform chooses, which is the closest thing winit has to GTK's
/// centre-on-parent. `utility` requests the X11 utility window type when that
/// backend is active; other backends keep their ordinary top-level type.
pub(crate) fn build_software_window(
    target: &ActiveEventLoop,
    title: &str,
    resizable: bool,
    utility: bool,
    position: Option<(i32, i32)>,
    width: u32,
    height: u32,
    context: &'static str,
) -> anyhow::Result<SoftwareWindow> {
    use anyhow::Context;

    let mut attributes = Window::default_attributes()
        .with_title(title)
        .with_inner_size(winit::dpi::PhysicalSize::new(width, height))
        .with_resizable(resizable);
    if let Some((x, y)) = position {
        attributes = attributes.with_position(winit::dpi::PhysicalPosition::new(x, y));
    }
    #[cfg(target_os = "linux")]
    {
        use winit::platform::x11::{ActiveEventLoopExtX11, WindowAttributesExtX11, WindowType};

        if utility && target.is_x11() {
            attributes = attributes.with_x11_window_type(vec![WindowType::Utility]);
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = utility;
    let window = Arc::new(
        target
            .create_window(attributes)
            .with_context(|| format!("failed to create the {context} window"))?,
    );
    let pixels = crate::main_audio::build_framebuffer(&window, window.inner_size())
        .with_context(|| format!("failed to create the {context} framebuffer"))?;
    Ok(SoftwareWindow::new(window, pixels, width, height))
}

impl SoftwareWindow {
    pub fn new(window: Arc<Window>, pixels: WindowSurface, width: u32, height: u32) -> Self {
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

    /// The extent the contents are laid out and hit-tested in.
    pub(crate) fn surface_extent(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// The window's live position, which a hide reads *before* moving so the
    /// next show can restore it.
    pub(crate) fn position(&self) -> Option<(i32, i32)> {
        self.window
            .outer_position()
            .ok()
            .map(|position| (position.x, position.y))
    }

    /// Blit a finished surface and present it.
    ///
    /// `None` blanks the window, which is what a record with nothing to draw
    /// gets rather than a stale frame from whatever drew last.
    pub(crate) fn present_surface(
        &mut self,
        surface: Option<&clonk_graphics::Surface>,
    ) -> Result<SoftwarePresent, String> {
        let Some(pixels) = self.pixels.as_mut() else {
            return Ok(SoftwarePresent::Skipped);
        };
        match surface {
            Some(surface) => {
                let frame = pixels.frame_mut();
                let drawn = surface.pixels();
                if frame.len() == drawn.len() {
                    frame.copy_from_slice(drawn);
                } else {
                    // A resize the buffer has not caught up with; showing the
                    // previous frame beats tearing.
                    tracing::trace!("a developer window's frame does not match its buffer yet");
                }
            }
            None => pixels.frame_mut().fill(0),
        }
        match crate::main_audio::present_pixels_frame(pixels) {
            Ok(crate::main_audio::RetainedGpuPresentOutcome::Presented) => {
                self.surface_rebuild.note_presented();
                Ok(SoftwarePresent::Presented)
            }
            Ok(crate::main_audio::RetainedGpuPresentOutcome::Skipped) => {
                Ok(SoftwarePresent::Skipped)
            }
            Err(clonk_surface::SurfaceError::SurfaceLost) => {
                let _ = self.surface_rebuild.note_loss();
                self.set_visible(false);
                Ok(SoftwarePresent::SurfaceLost)
            }
            Err(error) => Err(error.to_string()),
        }
    }
}

impl DeveloperWindowHost for SoftwareWindow {
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

    fn focus_window(&mut self) {
        self.window.focus_window();
    }

    fn set_visible(&mut self, visible: bool) {
        self.window.set_visible(visible);
        self.visible = visible;
    }

    fn visible(&self) -> bool {
        self.visible
    }
}
