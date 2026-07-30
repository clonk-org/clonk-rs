//! A console viewport window's surface lifecycle.
//!
//! This is the port's `C4ViewportWindow`. On the reference build that class has
//! no platform body of its own — `CStdWindow::Init` just creates an SDL window
//! (`StdSDLWindow.cpp:52-66`) — and everything that makes it a *viewport*
//! window lives in `C4Viewport`: `UpdateOutputSize` converts the drawable to
//! the logical view extent (`C4Viewport.cpp:798`), `Execute` draws that one
//! viewport into it (`:1126-1155`), and `Close` routes the OS close through
//! `CloseViewport(cvp)` so exactly this viewport dies (`:775-778`).
//!
//! The identity is the C++ pointer. Two windows can follow the same player, so
//! the owner cannot address one of them — which is the whole reason
//! `CloseViewport(C4Viewport *)` exists beside its player-keyed sibling.

use crate::developer_windows::{DeveloperWindowHost, DeveloperWindowPresenter};
use crate::GameApp;
use pixels::Pixels;
use winit::window::Window;

pub struct ViewportWindowHost {
    pub window: Window,
    pub pixels: Pixels,
    /// The physical `C4Viewport` identity this window draws.
    pub identity: u64,
    /// `Application.GetScale()` — `Config.Graphics.Scale / 100`
    /// (`C4Application.h:119`), not a DPI factor.
    scale: f32,
    /// The logical view extent, `ceilf(drawable / scale)` (`C4Viewport.cpp:798`).
    buffer_width: u32,
    buffer_height: u32,
    visible: bool,
}

/// `ViewWdt = static_cast<int32_t>(ceilf(rect.Wdt / scale))`
/// (`C4Viewport.cpp:798`). A degenerate scale cannot divide, so the drawable is
/// taken as-is rather than producing a zero or infinite extent.
pub(crate) fn logical_view_extent(width: u32, height: u32, scale: f32) -> (u32, u32) {
    if !scale.is_finite() || scale <= 0.0 {
        return (width.max(1), height.max(1));
    }
    let logical = |extent: u32| ((extent as f32 / scale).ceil() as u32).max(1);
    (logical(width), logical(height))
}

/// Create one viewport window and its framebuffer.
///
/// `C4Viewport::Init(CStdWindow *pParent, ...)` passes the console shell as the
/// parent (`C4Viewport.cpp:1351`), but the reference `CStdWindow::Init` accepts
/// that argument and ignores it entirely (`StdSDLWindow.cpp:52-66`) — there is
/// no owner/child relationship to build. The window is resizable and high-DPI
/// aware there, so it is here.
pub(crate) fn build_viewport_window(
    target: &winit::event_loop::EventLoopWindowTarget<crate::NetworkEventWake>,
    title: &str,
    width: u32,
    height: u32,
    identity: u64,
    scale: f32,
) -> anyhow::Result<ViewportWindowHost> {
    use anyhow::Context;
    let window = winit::window::WindowBuilder::new()
        .with_title(title)
        .with_inner_size(winit::dpi::PhysicalSize::new(width, height))
        .with_resizable(true)
        .build(target)
        .context("failed to create a console viewport window")?;
    let pixels = crate::main_audio::build_framebuffer(&window, window.inner_size())
        .context("failed to create a console viewport framebuffer")?;
    Ok(ViewportWindowHost::new(window, pixels, identity, scale))
}

impl ViewportWindowHost {
    pub fn new(window: Window, mut pixels: Pixels, identity: u64, scale: f32) -> Self {
        let size = window.inner_size();
        let (buffer_width, buffer_height) = logical_view_extent(size.width, size.height, scale);
        let _ = pixels.resize_buffer(buffer_width, buffer_height);
        Self {
            window,
            pixels,
            identity,
            scale,
            buffer_width,
            buffer_height,
            visible: true,
        }
    }
}

impl DeveloperWindowHost for ViewportWindowHost {
    fn resize(&mut self, width: u32, height: u32) {
        let _ = self.pixels.resize_surface(width.max(1), height.max(1));
        let (buffer_width, buffer_height) = logical_view_extent(width, height, self.scale);
        let _ = self.pixels.resize_buffer(buffer_width, buffer_height);
        self.buffer_width = buffer_width;
        self.buffer_height = buffer_height;
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

impl DeveloperWindowPresenter<GameApp> for ViewportWindowHost {
    /// `C4Viewport::Execute` — draw this viewport, then `BlitOutput`, which
    /// page-flips immediately because the viewport has a window
    /// (`C4Viewport.cpp:1121-1124`).
    fn present(&mut self, app: &mut GameApp) -> Result<(), String> {
        match app.render_console_viewport(self.identity, self.buffer_width, self.buffer_height) {
            Some(surface) => {
                let frame = self.pixels.frame_mut();
                let drawn = surface.pixels();
                if frame.len() == drawn.len() {
                    frame.copy_from_slice(drawn);
                } else {
                    // A resize that has not reached the buffer yet. Skipping
                    // the copy shows the previous frame rather than tearing.
                    tracing::trace!(
                        identity = self.identity,
                        "viewport frame size does not match its buffer yet"
                    );
                }
            }
            // The viewport is gone. Its window goes blank rather than adopting
            // another viewport's view; the close pass destroys it next.
            None => self.pixels.frame_mut().fill(0),
        }
        self.pixels.render().map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // C4Viewport.cpp:798 — the logical view extent is a *ceiling* division of
    // the drawable by the application scale.
    #[test]
    fn viewport_logical_extent_is_a_ceiling_division_by_the_application_scale() {
        assert_eq!(logical_view_extent(400, 250, 1.0), (400, 250));
        assert_eq!(logical_view_extent(800, 500, 2.0), (400, 250));
        // Ceiling, not rounding or truncation: 401/2 = 200.5 -> 201.
        assert_eq!(logical_view_extent(401, 501, 2.0), (201, 251));
        // A fractional scale rounds up on both axes — but only when there is
        // a remainder: 404 / 1.01 is exactly 400.
        assert_eq!(logical_view_extent(405, 253, 1.01), (401, 251));
        assert_eq!(logical_view_extent(404, 253, 1.01), (400, 251));
        // Never zero — a zero-extent view would draw nothing forever.
        assert_eq!(logical_view_extent(0, 0, 1.0), (1, 1));
        // A degenerate scale takes the drawable rather than dividing by it.
        for scale in [0.0, -1.0, f32::NAN] {
            assert_eq!(logical_view_extent(320, 200, scale), (320, 200));
        }
    }
}
