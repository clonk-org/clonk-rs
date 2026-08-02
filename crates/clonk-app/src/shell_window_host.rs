//! The console shell's [`DeveloperWindowHost`] implementation.
//!
//! C++ has no registry entry for its main window — `C4Application` owns
//! `FullScreen`/`Console` directly and viewports live in a separate list. The
//! port needs one, because a `WindowId` arriving from winit must resolve to
//! *some* record before it can be routed, and the shell is the only surface
//! that exists until the console opens its own windows
//! (`C4Viewport.cpp:775-834`, `C4DevmodeDlg.cpp:50-121`).
//!
//! Bundling the four pieces the runner used to hold as separate locals — the
//! window, its pixel surface, the frame presenter and the retained GPU renderer
//! built from that surface — is what makes them addressable by id. They are one
//! object's worth of state: the renderer is constructed from the surface's
//! device, queue and format, so it can never outlive or be swapped independently
//! of it.

use crate::developer_windows::{DeveloperWindowHost, DeveloperWindowPresenter};
use crate::{present_retained_gpu_frame, GameApp};
use clonk_app_render::gpu_renderer::RetainedGpuRenderer;
use pixels::Pixels;
use std::sync::Arc;
use winit::window::Window;

/// The console shell's window and everything bound to its surface.
pub struct ShellWindowHost {
    pub window: Arc<Window>,
    pub pixels: Pixels<'static>,
    pub presenter: clonk_scaling::FramePresenter,
    /// Built from `pixels`' device/queue/format, so it is part of this surface.
    pub renderer: RetainedGpuRenderer,
    visible: bool,
}

impl ShellWindowHost {
    pub fn new(
        window: Arc<Window>,
        pixels: Pixels<'static>,
        presenter: clonk_scaling::FramePresenter,
        renderer: RetainedGpuRenderer,
    ) -> Self {
        // Leave winit's IME disabled for the game shell. While preedit is
        // active winit suppresses KeyboardInput, which can lose releases and
        // leave gameplay controls stuck. The legacy shell had no IME opt-in.
        Self {
            window,
            pixels,
            presenter,
            renderer,
            visible: true,
        }
    }
}

impl DeveloperWindowHost for ShellWindowHost {
    fn resize(&mut self, width: u32, height: u32) {
        // Surface and buffer are resized together by the runner's own resize
        // path, which reports its errors; this is the registry-facing form.
        let _ = self.pixels.resize_surface(width, height);
        let _ = self.pixels.resize_buffer(width, height);
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

impl DeveloperWindowPresenter<GameApp> for ShellWindowHost {
    /// The shell draws from `GameApp` exactly as a C++ viewport draws from the
    /// global `Game`; the port just has to say so.
    fn present(&mut self, app: &mut GameApp) -> Result<(), String> {
        present_retained_gpu_frame(app, &self.pixels, &self.presenter, &mut self.renderer)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}
