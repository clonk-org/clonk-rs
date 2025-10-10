use std::fs::{self, File, OpenOptions};
use std::io::{LineWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{anyhow, Context, Result};
use lc_graphics::{Color, PixelFormat, Surface};
use lc_gui::{DrawCommand, GuiEvent, KeyCode, Point as GuiPoint, Rect as GuiRect, Size as GuiSize};
use lc_launcher::{
    ensure_support_bundle, load_shell_state, reveal_in_file_manager, timestamp_for_filename,
    timestamp_for_log, LauncherLog,
};
use lc_launcher_ui::{LauncherShellMessage, LauncherShellResponse, LauncherShellUi};
use lc_platform::AppPaths;
use pixels::{Pixels, SurfaceTexture};
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::{ElementState, Event, MouseButton, TouchPhase, VirtualKeyCode, WindowEvent};
use winit::event_loop::{ControlFlow, EventLoop};
use winit::window::{Window, WindowBuilder};

fn main() -> Result<()> {
    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title("LegacyClonk Launcher")
        .with_inner_size(LogicalSize::new(960.0, 640.0))
        .build(&event_loop)
        .context("failed to create launcher window")?;

    let size = window.inner_size();
    let (initial_width, initial_height) = enforce_min_size(size);
    let surface_texture = SurfaceTexture::new(initial_width, initial_height, &window);
    let mut pixels = Pixels::new(initial_width, initial_height, surface_texture)
        .context("failed to create pixel framebuffer")?;

    let mut app = LauncherApp::new(&window).context("failed to initialise launcher shell")?;
    let window_id = window.id();

    event_loop.run(move |event, _, control_flow| match event {
        Event::NewEvents(_) => control_flow.set_wait(),
        Event::WindowEvent {
            window_id: id,
            event,
        } if id == window_id => {
            if let Err(err) = app.handle_window_event(&mut pixels, event, control_flow) {
                eprintln!("launcher shell encountered an error: {err:?}");
                control_flow.set_exit();
            }
        }
        Event::MainEventsCleared => window.request_redraw(),
        Event::RedrawRequested(id) if id == window_id => {
            if let Err(err) = app.render(pixels.frame_mut()) {
                eprintln!("failed to render UI: {err:?}");
                control_flow.set_exit();
                return;
            }
            if let Err(err) = pixels.render() {
                eprintln!("failed to swap buffers: {err:?}");
                control_flow.set_exit();
            }
        }
        Event::LoopDestroyed => {}
        _ => {}
    })
}

fn enforce_min_size(size: PhysicalSize<u32>) -> (u32, u32) {
    let width = size.width.max(1);
    let height = size.height.max(1);
    (width, height)
}

struct LauncherApp {
    paths: AppPaths,
    logger: ShellLogger,
    ui: LauncherShellUi,
    surface: Surface,
    pointer_position: Option<GuiPoint>,
}

impl LauncherApp {
    fn new(window: &Window) -> Result<Self> {
        let size = window.inner_size();
        let (width, height) = enforce_min_size(size);
        let paths = AppPaths::discover().context("failed to discover LegacyClonk paths")?;
        paths
            .ensure_user_dirs()
            .context("failed to prepare launcher directories")?;

        let default_log = paths.logs_dir().join(format!(
            "lc-launcher-shell-{}.log",
            timestamp_for_filename()
        ));
        let logger =
            ShellLogger::new(default_log).context("failed to initialise launcher logging")?;
        logger
            .log_line("launcher shell initialised")
            .context("failed to record launcher startup")?;

        let ui = LauncherShellUi::new(None).map_err(|err| anyhow!(err))?;

        let mut app = Self {
            paths,
            logger,
            ui,
            surface: Surface::new(width, height, PixelFormat::Rgba8888),
            pointer_position: None,
        };
        app.refresh_state()
            .context("failed to load launcher state")?;
        Ok(app)
    }

    fn handle_window_event(
        &mut self,
        pixels: &mut Pixels,
        event: WindowEvent<'_>,
        control_flow: &mut ControlFlow,
    ) -> Result<()> {
        match event {
            WindowEvent::CloseRequested => {
                control_flow.set_exit();
            }
            WindowEvent::Resized(size) => {
                self.handle_resize(size, pixels)?;
            }
            WindowEvent::ScaleFactorChanged { new_inner_size, .. } => {
                let size = *new_inner_size;
                self.handle_resize(size, pixels)?;
            }
            WindowEvent::CursorMoved { position, .. } => {
                let logical = GuiPoint::new(position.x as f32, position.y as f32);
                self.pointer_position = Some(logical);
                self.dispatch_gui_event(GuiEvent::PointerMove { position: logical })?;
            }
            WindowEvent::CursorEntered { .. } => {}
            WindowEvent::CursorLeft { .. } => {
                self.pointer_position = None;
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if button == MouseButton::Left {
                    if let Some(position) = self.pointer_position {
                        let event = match state {
                            ElementState::Pressed => GuiEvent::PointerDown { position },
                            ElementState::Released => GuiEvent::PointerUp { position },
                        };
                        self.dispatch_gui_event(event)?;
                    }
                }
            }
            WindowEvent::KeyboardInput { input, .. } => {
                if let Some(key) = input.virtual_keycode.and_then(map_key_code) {
                    let event = match input.state {
                        ElementState::Pressed => GuiEvent::KeyDown { key },
                        ElementState::Released => GuiEvent::KeyUp { key },
                    };
                    self.dispatch_gui_event(event)?;
                }
            }
            WindowEvent::Touch(touch) => {
                let position = GuiPoint::new(touch.location.x as f32, touch.location.y as f32);
                match touch.phase {
                    TouchPhase::Started => {
                        self.pointer_position = Some(position);
                        self.dispatch_gui_event(GuiEvent::PointerDown { position })?;
                    }
                    TouchPhase::Moved => {
                        self.pointer_position = Some(position);
                        self.dispatch_gui_event(GuiEvent::PointerMove { position })?;
                    }
                    TouchPhase::Ended => {
                        self.pointer_position = None;
                        self.dispatch_gui_event(GuiEvent::PointerUp { position })?;
                    }
                    TouchPhase::Cancelled => {
                        self.pointer_position = None;
                    }
                }
            }
            WindowEvent::Focused(false) => {
                self.pointer_position = None;
            }
            WindowEvent::Focused(true) => {
                if let Err(err) = self.refresh_state() {
                    eprintln!("failed to refresh launcher state on focus gain: {err:?}");
                }
            }
            WindowEvent::DroppedFile(_)
            | WindowEvent::HoveredFile(_)
            | WindowEvent::HoveredFileCancelled => {}
            WindowEvent::ThemeChanged(_)
            | WindowEvent::ModifiersChanged(_)
            | WindowEvent::MouseWheel { .. }
            | WindowEvent::ReceivedCharacter(_)
            | WindowEvent::Ime(_)
            | WindowEvent::TouchpadPressure { .. }
            | WindowEvent::AxisMotion { .. }
            | WindowEvent::TouchpadMagnify { .. }
            | WindowEvent::TouchpadRotate { .. }
            | WindowEvent::SmartMagnify { .. }
            | WindowEvent::Occluded(_)
            | WindowEvent::Moved(_)
            | WindowEvent::Destroyed => {}
        }
        Ok(())
    }

    fn handle_resize(&mut self, size: PhysicalSize<u32>, pixels: &mut Pixels) -> Result<()> {
        if size.width == 0 || size.height == 0 {
            return Ok(());
        }
        pixels
            .resize_surface(size.width, size.height)
            .context("failed to resize pixel surface")?;
        pixels
            .resize_buffer(size.width, size.height)
            .context("failed to resize pixel buffer")?;
        self.surface = Surface::new(size.width, size.height, PixelFormat::Rgba8888);
        Ok(())
    }

    fn dispatch_gui_event(&mut self, event: GuiEvent) -> Result<()> {
        let response = self.ui.handle_event(event);
        self.process_response(response)
    }

    fn process_response(&mut self, response: LauncherShellResponse) -> Result<()> {
        for message in response.messages {
            if let Err(err) = self.handle_message(message) {
                eprintln!("launcher action failed: {err:?}");
                let _ = self
                    .logger
                    .log_line(&format!("launcher action failed: {err}"));
            }
        }
        Ok(())
    }

    fn handle_message(&mut self, message: LauncherShellMessage) -> Result<()> {
        match message {
            LauncherShellMessage::RegenerateSupportBundle => {
                let state = match self.ui.state().cloned() {
                    Some(state) => state,
                    None => {
                        self.logger
                            .log_line("support bundle requested before launcher summary exists")
                            .context("failed to log missing summary")?;
                        return Ok(());
                    }
                };
                self.logger
                    .log_line("support bundle regeneration requested via launcher UI")
                    .context("failed to record regeneration request")?;
                let result =
                    ensure_support_bundle(&self.paths, &self.logger, &state.launcher_log_path)
                        .context("support bundle regeneration failed")?;
                self.logger
                    .log_line(&format!(
                        "support bundle refreshed: {} ({} telemetry successes, {} failures)",
                        result.bundle_path.display(),
                        result.telemetry.successes().len(),
                        result.telemetry.failures().len()
                    ))
                    .context("failed to log regenerated bundle metadata")?;
                self.refresh_state()
                    .context("failed to refresh launcher state after bundle regeneration")?;
            }
            LauncherShellMessage::CopySupportBundle { .. } => {
                self.logger.log_line(
                    "copy support bundle action requested (pending native picker integration)",
                )?;
                println!(
                    "Copying support bundles requires a destination picker; this will be added in a later milestone."
                );
            }
            LauncherShellMessage::RevealPath { path, label } => {
                self.logger
                    .log_line(&format!("revealing {label} at {}", path.display()))?;
                reveal_in_file_manager(&path)
                    .with_context(|| format!("failed to reveal {}", path.display()))?;
            }
            LauncherShellMessage::UploadSupportArtifacts { artifacts } => {
                self.logger.log_line(
                    "upload support artifacts action requested (upload UI not yet wired)",
                )?;
                println!(
                    "Upload requested for {} artifacts; UI wiring will land alongside the transfer implementation.",
                    artifacts.len()
                );
            }
        }
        Ok(())
    }

    fn refresh_state(&mut self) -> Result<()> {
        let state = load_shell_state(&self.paths).context("failed to load launcher state")?;
        match state {
            Some(state) => {
                self.logger
                    .set_target(state.launcher_log_path.clone())
                    .context("failed to attach launcher log to runtime log file")?;
                self.ui
                    .set_state(Some(state))
                    .map_err(|err| anyhow!(err))
                    .context("failed to update launcher UI state")?;
            }
            None => {
                self.ui
                    .set_state(None)
                    .map_err(|err| anyhow!(err))
                    .context("failed to reset launcher UI state")?;
            }
        }
        Ok(())
    }

    fn render(&mut self, frame: &mut [u8]) -> Result<()> {
        let width = self.surface.width();
        let height = self.surface.height();

        self.surface.fill(Color::opaque(12, 16, 28));
        self.ui.layout(GuiSize::new(width as f32, height as f32));
        let commands = self.ui.render();
        render_commands(&mut self.surface, &commands);
        debug_assert_eq!(frame.len(), (width as usize) * (height as usize) * 4);
        frame.copy_from_slice(self.surface.pixels());
        Ok(())
    }
}

fn render_commands(surface: &mut Surface, commands: &[DrawCommand]) {
    for command in commands {
        match command {
            DrawCommand::Quad { rect, color } => fill_rect(surface, rect, *color),
            DrawCommand::Text { rect, text, color } => draw_text(surface, rect, text, *color),
        }
    }
}

fn fill_rect(surface: &mut Surface, rect: &GuiRect, color: Color) {
    let x0 = rect.origin.x.floor() as i32;
    let y0 = rect.origin.y.floor() as i32;
    let x1 = (rect.origin.x + rect.size.width).ceil() as i32;
    let y1 = (rect.origin.y + rect.size.height).ceil() as i32;

    let x0 = x0.clamp(0, surface.width() as i32);
    let y0 = y0.clamp(0, surface.height() as i32);
    let x1 = x1.clamp(0, surface.width() as i32);
    let y1 = y1.clamp(0, surface.height() as i32);

    for y in y0..y1 {
        for x in x0..x1 {
            let _ = surface.set_pixel(x as u32, y as u32, color);
        }
    }
}

fn draw_text(surface: &mut Surface, rect: &GuiRect, text: &str, color: Color) {
    let mut cursor_x = rect.origin.x;
    let baseline = rect.origin.y;
    let glyph_width = 6.0f32;
    let glyph_height = rect.size.height.clamp(6.0, 14.0);

    for ch in text.chars() {
        if cursor_x > surface.width() as f32 {
            break;
        }
        if ch == ' ' {
            cursor_x += glyph_width;
            continue;
        }
        let intensity = ((ch as u32).wrapping_mul(17) % 80) as u8;
        let glyph_color = Color::new(
            color.r.saturating_add(intensity / 2),
            color.g.saturating_add(intensity / 3),
            color.b.saturating_add(intensity / 4),
            255,
        );
        let glyph_rect = GuiRect::from_origin_size(
            GuiPoint::new(cursor_x, baseline),
            GuiSize::new(glyph_width - 1.0, glyph_height),
        );
        fill_rect(surface, &glyph_rect, glyph_color);
        cursor_x += glyph_width;
    }
}

fn map_key_code(code: VirtualKeyCode) -> Option<KeyCode> {
    match code {
        VirtualKeyCode::Return => Some(KeyCode::Enter),
        VirtualKeyCode::Escape => Some(KeyCode::Escape),
        VirtualKeyCode::Space => Some(KeyCode::Space),
        VirtualKeyCode::Tab => Some(KeyCode::Tab),
        VirtualKeyCode::Up => Some(KeyCode::Up),
        VirtualKeyCode::Down => Some(KeyCode::Down),
        VirtualKeyCode::Left => Some(KeyCode::Left),
        VirtualKeyCode::Right => Some(KeyCode::Right),
        _ => None,
    }
}

struct ShellLogger {
    inner: Mutex<LoggerInner>,
}

struct LoggerInner {
    writer: LineWriter<File>,
    path: PathBuf,
}

impl ShellLogger {
    fn new(path: PathBuf) -> Result<Self> {
        ensure_parent(&path)?;
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("failed to open launcher log {}", path.display()))?;
        Ok(Self {
            inner: Mutex::new(LoggerInner {
                writer: LineWriter::new(file),
                path,
            }),
        })
    }

    fn set_target(&self, path: PathBuf) -> Result<()> {
        {
            let guard = self
                .inner
                .lock()
                .map_err(|_| anyhow!("launcher log mutex poisoned"))?;
            if guard.path == path {
                return Ok(());
            }
        }

        ensure_parent(&path)?;
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("failed to open launcher log {}", path.display()))?;

        let mut guard = self
            .inner
            .lock()
            .map_err(|_| anyhow!("launcher log mutex poisoned"))?;
        guard.writer = LineWriter::new(file);
        guard.path = path.clone();
        drop(guard);

        self.log_line(&format!("launcher log redirected to {}", path.display()))?;
        Ok(())
    }
}

impl LauncherLog for ShellLogger {
    fn log_line(&self, message: &str) -> Result<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| anyhow!("launcher log mutex poisoned"))?;
        writeln!(guard.writer, "[{}] {}", timestamp_for_log(), message)
            .context("failed to write launcher log entry")?;
        guard.writer.flush().context("failed to flush launcher log")
    }
}

fn ensure_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    Ok(())
}
