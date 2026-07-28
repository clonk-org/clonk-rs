//! `main.rs` — the audio context, sound/music resolution and console plumbing.
//!
//! A contiguous slice moved verbatim from the crate root; it stays part of
//! the same binary crate, re-exported from `main.rs` so every path resolves.

use super::*;

pub(crate) fn retained_gpu_gamma_mode(
    config: clonk_frontend::AdvancedRendererConfig,
) -> GpuGammaMode {
    if config.disable_gamma {
        GpuGammaMode::Disabled
    } else if config.shader && config.use_shader_gamma {
        GpuGammaMode::Fragment
    } else {
        GpuGammaMode::Monitor
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RetainedGpuPresentRecovery {
    RebuildDevice,
    Retry,
    Fatal,
}

pub(crate) fn wgpu_device_loss_panic_detail(
    payload: &(dyn std::any::Any + Send),
) -> Option<String> {
    let detail = payload.downcast_ref::<String>().cloned().or_else(|| {
        payload
            .downcast_ref::<&'static str>()
            .map(|detail| (*detail).to_owned())
    })?;
    let normalized = detail.to_ascii_lowercase();
    (normalized.contains("parent device is lost")
        || normalized.contains("device was lost")
        || normalized.contains("device has been lost"))
    .then_some(detail)
}

pub(crate) fn retained_gpu_device_loss_error(detail: String) -> anyhow::Error {
    gpu_renderer::GpuRendererError::DeviceRecreationRequired {
        reason: gpu_renderer::RetainedGpuRecreateReason::DeviceLost,
        detail,
    }
    .into()
}

pub(crate) fn retained_gpu_present_recovery(error: &anyhow::Error) -> RetainedGpuPresentRecovery {
    if error.chain().any(|cause| {
        matches!(
            cause.downcast_ref::<gpu_renderer::GpuRendererError>(),
            Some(gpu_renderer::GpuRendererError::DeviceRecreationRequired { .. })
        )
    }) {
        return RetainedGpuPresentRecovery::RebuildDevice;
    }
    match error.downcast_ref::<pixels::Error>() {
        Some(pixels::Error::Surface(
            pixels::wgpu::SurfaceError::Lost | pixels::wgpu::SurfaceError::Outdated,
        )) => RetainedGpuPresentRecovery::RebuildDevice,
        Some(pixels::Error::Surface(pixels::wgpu::SurfaceError::Timeout)) => {
            RetainedGpuPresentRecovery::Retry
        }
        Some(pixels::Error::Surface(pixels::wgpu::SurfaceError::OutOfMemory)) | None => {
            RetainedGpuPresentRecovery::Fatal
        }
        Some(_) => RetainedGpuPresentRecovery::Fatal,
    }
}

/// Backend sets to try, in order, when creating the framebuffer device.
///
/// `Backends::PRIMARY` (VULKAN | METAL | DX12 | BROWSER_WEBGPU) comes first
/// because the GL backend probes for libEGL and logs a spurious "Unable to
/// open libEGL" on macOS before falling back to Metal. It contains no GL at
/// all, though, so a board whose only usable driver is GLES — the common
/// Raspberry Pi case — produced no adapter and aborted startup. Widening on
/// failure costs a desktop machine nothing and is the difference between
/// running and not running there.
///
/// An explicit `WGPU_BACKEND` is an operator instruction: honour it exactly
/// and never widen past it.
pub(crate) fn framebuffer_backend_attempts(
    requested: Option<pixels::wgpu::Backends>,
) -> Vec<pixels::wgpu::Backends> {
    requested.map_or_else(
        || vec![pixels::wgpu::Backends::PRIMARY, pixels::wgpu::Backends::all()],
        |backends| vec![backends],
    )
}

/// Create the framebuffer, widening the backend set rather than aborting.
///
/// Note what this cannot fix: wgpu-hal's GLES backend rejects any context
/// below GLES 3.0 (wgpu-hal-0.16.2 src/gles/adapter.rs:218-225), so VideoCore
/// IV boards (Pi 0-3) still produce no adapter on any backend. There is no CPU
/// presentation fallback either — `pixels` needs a wgpu device even to blit a
/// CPU buffer — so those boards fail here with the diagnostic below.
pub(crate) fn build_framebuffer(window: &Window, size: PhysicalSize<u32>) -> Result<Pixels> {
    let attempts = framebuffer_backend_attempts(pixels::wgpu::util::backend_bits_from_env());
    let mut last_error = None;
    for backends in attempts {
        let surface = SurfaceTexture::new(size.width, size.height, window);
        match PixelsBuilder::new(size.width, size.height, surface)
            .wgpu_backend(backends)
            // StdGLCtx::PageFlip calls SDL_GL_SwapWindow without ever selecting
            // a swap interval. Do not make drawable acquisition serialize the
            // independently scheduled simulation and graphics timers behind an
            // implicit FIFO-vsync wait that the C++ application does not request.
            .enable_vsync(false)
            .build()
        {
            Ok(pixels) => return Ok(pixels),
            Err(error) => {
                tracing::warn!(?backends, %error, "no usable GPU adapter for these backends");
                last_error = Some(error);
            }
        }
    }
    Err(last_error.map_or_else(
        || anyhow::anyhow!("no GPU backends were attempted"),
        anyhow::Error::from,
    ))
    .context(
        "failed to create pixel framebuffer: no GPU adapter on any backend \
         (GLES 2.0-only hardware such as Raspberry Pi 0-3 cannot be supported \
         by this renderer; it requires GLES 3.0 or a Vulkan/Metal/DX12 driver)",
    )
}

pub(crate) fn rebuild_retained_gpu_device(
    window: &Window,
    pixels: &mut Pixels,
    renderer: &mut gpu_renderer::RetainedGpuRenderer,
) -> Result<()> {
    let size = enforce_min_size(window.inner_size());
    let mut replacement =
        build_framebuffer(window, size).context("failed to rebuild retained GPU surface")?;
    replacement
        .resize_buffer(1, 1)
        .context("failed to restore retained GPU presentation buffer")?;
    renderer.recreate(
        replacement.device(),
        replacement.queue(),
        replacement.surface_texture_format(),
    );
    renderer
        .check_health()
        .context("replacement retained GPU device failed initialization")?;
    *pixels = replacement;
    Ok(())
}

pub(crate) fn present_retained_gpu_frame(
    app: &mut GameApp,
    pixels: &Pixels,
    presenter: &clonk_scaling::FramePresenter,
    renderer: &mut gpu_renderer::RetainedGpuRenderer,
) -> Result<()> {
    let geometry = presenter.presentation_geometry();
    let (physical_width, physical_height) = geometry.physical_size();
    let presentation = clonk_graphics::GpuPresentation {
        physical_extent: [physical_width, physical_height],
        scale: geometry.scale(),
        crop_top: geometry.crop_top(),
    };
    let frame = app.render_retained_gpu_frame(presentation)?;
    let layers = frame
        .layers
        .iter()
        .map(|layer| gpu_renderer::GpuSceneLayer::new(&layer.scene, layer.presentation))
        .collect::<Vec<_>>();
    let request_native_save_readback = !app.pending_native_save_thumbnails.is_empty();
    let request_current_readback =
        !app.pending_screenshots.is_empty() || !app.pending_gpu_thumbnail_paths.is_empty();
    let mut previous_native_readback = None;
    let mut readback = None;
    let submission = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        pixels.render_with(|encoder, surface_view, context| {
            if request_native_save_readback {
                previous_native_readback =
                    renderer.readback_last_presentation(&context.device, encoder)?;
            }
            readback = renderer.render_layers(
                &context.device,
                &context.queue,
                encoder,
                surface_view,
                &layers,
                request_current_readback
                    || (request_native_save_readback && previous_native_readback.is_none()),
            )?;
            Ok(())
        })
    }));
    match submission {
        Ok(result) => result.context("failed to submit retained GPU frame")?,
        Err(payload) => {
            if let Some(detail) = wgpu_device_loss_panic_detail(payload.as_ref()) {
                return Err(retained_gpu_device_loss_error(detail));
            }
            std::panic::resume_unwind(payload);
        }
    }

    let had_gpu_readback = previous_native_readback.is_some() || readback.is_some();
    let previous_native_frame = match previous_native_readback {
        Some(ticket) => {
            let read_result = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                ticket.read(pixels.device())
            })) {
                Ok(result) => result,
                Err(payload) => {
                    if let Some(detail) = wgpu_device_loss_panic_detail(payload.as_ref()) {
                        return Err(retained_gpu_device_loss_error(detail));
                    }
                    std::panic::resume_unwind(payload);
                }
            };
            match read_result {
                Ok(frame) => Some(frame),
                Err(error) => {
                    tracing::warn!(
                        saves = app.pending_native_save_thumbnails.len(),
                        ?error,
                        "failed to read previous retained GPU frame for native saves"
                    );
                    None
                }
            }
        }
        None => None,
    };
    let mut current_frame = match readback {
        Some(ticket) => {
            let read_result = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                ticket.read(pixels.device())
            })) {
                Ok(result) => result,
                Err(payload) => {
                    if let Some(detail) = wgpu_device_loss_panic_detail(payload.as_ref()) {
                        return Err(retained_gpu_device_loss_error(detail));
                    }
                    std::panic::resume_unwind(payload);
                }
            };
            match read_result {
                Ok(frame) => Some(frame),
                Err(error) => {
                    while let Some(path) = app.pending_gpu_thumbnail_paths.pop_front() {
                        tracing::warn!(
                            path = %path.display(),
                            ?error,
                            "failed to read retained GPU save thumbnail"
                        );
                    }
                    while !app.pending_screenshots.is_empty() {
                        let result = app.save_next_screenshot(
                            None,
                            physical_width,
                            physical_height,
                            presenter.scale(),
                        );
                        app.report_screenshot_result(result);
                    }
                    None
                }
            }
        }
        None => None,
    };
    if had_gpu_readback {
        renderer
            .check_health()
            .context("retained GPU device failed while completing readback")?;
    }

    if !app.pending_native_save_thumbnails.is_empty() {
        let title_png = previous_native_frame
            .as_ref()
            .or(current_frame.as_ref())
            .and_then(|frame| {
                match encode_presented_save_thumbnail(frame.extent[0], frame.extent[1], &frame.rgba)
                {
                    Ok(encoded) => Some(encoded),
                    Err(error) => {
                        tracing::warn!(
                            saves = app.pending_native_save_thumbnails.len(),
                            ?error,
                            "failed to encode retained GPU frame for native saves"
                        );
                        None
                    }
                }
            });
        app.finish_pending_native_save_thumbnails(title_png.as_deref());
    }

    if let Some(frame) = current_frame.as_mut() {
        if !app.pending_gpu_thumbnail_paths.is_empty() {
            match encode_presented_save_thumbnail(frame.extent[0], frame.extent[1], &frame.rgba) {
                Ok(encoded) => {
                    while let Some(path) = app.pending_gpu_thumbnail_paths.pop_front() {
                        let result = (|| -> Result<()> {
                            let mut file = File::create(&path).with_context(|| {
                                format!("failed to create thumbnail at {}", path.display())
                            })?;
                            file.write_all(&encoded)
                                .context("failed to write retained GPU save thumbnail")?;
                            file.flush()
                                .context("failed to flush retained GPU save thumbnail")
                        })();
                        if let Err(error) = result {
                            tracing::warn!(
                                path = %path.display(),
                                ?error,
                                "failed to persist retained GPU save thumbnail"
                            );
                        }
                    }
                }
                Err(error) => {
                    while let Some(path) = app.pending_gpu_thumbnail_paths.pop_front() {
                        tracing::warn!(
                            path = %path.display(),
                            ?error,
                            "failed to encode retained GPU save thumbnail"
                        );
                    }
                }
            }
        }
        while !app.pending_screenshots.is_empty() {
            let result = app.save_next_screenshot(
                Some(frame.rgba.as_mut_slice()),
                frame.extent[0],
                frame.extent[1],
                presenter.scale(),
            );
            app.report_screenshot_result(result);
        }
    }
    Ok(())
}

pub(crate) fn deliver_desktop_notifications<F>(app: &mut GameApp, mut show: F)
where
    F: FnMut(&DesktopNotification) -> Result<()>,
{
    while let Some(notification) = app.take_desktop_notification() {
        if let Err(error) = show(&notification) {
            tracing::warn!(
                %error,
                title = %notification.title,
                "failed to show desktop notification"
            );
        }
    }
}

pub(crate) fn apply_options_display_requests(
    window: &Window,
    app: &mut GameApp,
    presenter: &mut clonk_scaling::FramePresenter,
    display_options: &mut DisplayOptions,
    paths: Option<&AppPaths>,
) -> Result<()> {
    while let Some(request) = app.pending_options_display_requests.pop_front() {
        let recreate_options = app.mode == AppMode::Menu
            && app.startup_view == StartupView::Options
            && app.startup_options_dialog.is_some();
        match request {
            OptionsDisplayRequest::SetMode(mode) => {
                let mode = match mode {
                    clonk_frontend::startup_options_graphics::GraphicsDisplayMode::Fullscreen => {
                        DisplayMode::Fullscreen
                    }
                    clonk_frontend::startup_options_graphics::GraphicsDisplayMode::Window => {
                        DisplayMode::Window
                    }
                };
                match mode {
                    DisplayMode::Fullscreen if window.fullscreen().is_none() => {
                        window.set_fullscreen(Some(Fullscreen::Borderless(None)));
                    }
                    DisplayMode::Window if window.fullscreen().is_some() => {
                        window.set_fullscreen(None);
                    }
                    DisplayMode::Fullscreen | DisplayMode::Window => {}
                }
                display_options.record_mode(mode);
                app.set_display_mode(mode);
            }
            OptionsDisplayRequest::SetScale { percent, persist } => {
                anyhow::ensure!(percent > 0, "application scale must remain positive");
                presenter.set_scale(percent as f32 / 100.0);
                let point_filtering = app.graphics.point_filtering();
                app.configure_native_startup_fonts(presenter.scale(), point_filtering);
                let (logical_width, logical_height) = presenter.logical_size();
                app.resize(logical_width, logical_height)?;
                if persist {
                    let (physical_width, physical_height) = presenter.physical_size();
                    display_options.record_scale_percent(percent, physical_width, physical_height);
                }
            }
        }
        if !app.configuration_reset_requested {
            if let Some(paths) = paths {
                display_options.persist_if_dirty(paths);
            }
        }
        if recreate_options {
            app.begin_startup_dialog_fade(StartupDialog::Options);
        }
        window.request_redraw();
    }
    Ok(())
}

/// Drive C4Game's scheduler-owned one-second callback independently from the
/// fixed-step frame accumulator (StdAppUnix.cpp:286-291).
pub(crate) fn advance_game_clock_from_elapsed(
    app: &mut GameApp,
    accumulator: &mut Duration,
    elapsed: Duration,
) -> Result<bool, EngineError> {
    app.guard_classic_global_gui_bootstrap()?;
    *accumulator += elapsed;
    // Both C++ paths coalesce a missed second rather than replaying it: Win32
    // never queues WM_TIMER more than once (StdAppWin32.cpp:132) and the unix
    // path fires at most one callback per Execute. Replaying each missed second
    // serializes them all into a single event-loop pass, and `sec1_timer` is not
    // cheap -- it runs the network status reach-check, inactive-client
    // deactivation, lobby countdown/ready/telemetry refreshes and the host
    // league vote timeout. Any gap in pumping the loop (a suspend, a long load,
    // a modal) therefore froze the app for one such pass per second elapsed.
    //
    // Keep one pending second plus the sub-second phase and drop the backlog
    // beyond it. Normal operation never reaches this branch, so the timer's
    // phase is preserved and it cannot drift.
    if *accumulator >= GAME_SECOND_INTERVAL * 2 {
        *accumulator =
            GAME_SECOND_INTERVAL + Duration::from_nanos(u64::from(accumulator.subsec_nanos()));
    }
    let mut changed = false;
    while *accumulator >= GAME_SECOND_INTERVAL {
        changed |= app.sec1_timer()?;
        *accumulator -= GAME_SECOND_INTERVAL;
    }
    Ok(changed)
}

fn map_developer_console_key(key: VirtualKeyCode) -> Option<DeveloperConsoleKey> {
    Some(match key {
        VirtualKeyCode::Return | VirtualKeyCode::NumpadEnter => DeveloperConsoleKey::Enter,
        VirtualKeyCode::Escape => DeveloperConsoleKey::Escape,
        VirtualKeyCode::Back => DeveloperConsoleKey::Backspace,
        VirtualKeyCode::Delete => DeveloperConsoleKey::Delete,
        VirtualKeyCode::Left => DeveloperConsoleKey::Left,
        VirtualKeyCode::Right => DeveloperConsoleKey::Right,
        VirtualKeyCode::Home => DeveloperConsoleKey::Home,
        VirtualKeyCode::End => DeveloperConsoleKey::End,
        VirtualKeyCode::Up => DeveloperConsoleKey::Up,
        VirtualKeyCode::Down => DeveloperConsoleKey::Down,
        VirtualKeyCode::PageUp => DeveloperConsoleKey::PageUp,
        VirtualKeyCode::PageDown => DeveloperConsoleKey::PageDown,
        VirtualKeyCode::Tab => DeveloperConsoleKey::Tab,
        VirtualKeyCode::Pause => DeveloperConsoleKey::Pause,
        _ => return None,
    })
}

fn developer_console_menu_mnemonic(key: VirtualKeyCode) -> Option<char> {
    Some(match key {
        VirtualKeyCode::F => 'f',
        VirtualKeyCode::C => 'c',
        VirtualKeyCode::P => 'p',
        VirtualKeyCode::V => 'v',
        VirtualKeyCode::N => 'n',
        VirtualKeyCode::H => 'h',
        _ => return None,
    })
}

fn handle_developer_console_window_event(
    window: &Window,
    app: &mut GameApp,
    pixels: &mut Pixels,
    presenter: &mut clonk_scaling::FramePresenter,
    event: WindowEvent,
    control_flow: &mut ControlFlow,
) -> Result<()> {
    let message_dialog_active = !app.message_dialogs.is_empty();
    match event {
        WindowEvent::CloseRequested => {
            // A native modal C4Console::Message disables its parent window.
            if !message_dialog_active {
                app.request_exit();
            }
        }
        WindowEvent::Resized(size)
        | WindowEvent::ScaleFactorChanged {
            new_inner_size: &mut size,
            ..
        } => {
            let clamped = enforce_min_size(size);
            pixels
                .resize_surface(clamped.width, clamped.height)
                .context("failed to resize console pixel surface")?;
            pixels
                .resize_buffer(clamped.width, clamped.height)
                .context("failed to resize console pixel buffer")?;
            presenter.resize(clamped.width, clamped.height);
            let (width, height) = presenter.logical_size();
            app.resize(width, height)?;
            window.request_redraw();
        }
        WindowEvent::CursorMoved { position, .. } => {
            let (x, y) = presenter.position_to_gui(position.x, position.y);
            if message_dialog_active {
                app.handle_cursor_moved(PhysicalPosition::new(x, y))?;
            } else {
                let point = GuiPoint::new(x as f32, y as f32);
                app.developer_console_pointer = point;
                app.developer_console.handle_pointer_move(point);
                app.pointer_inside_window = true;
            }
            window.request_redraw();
        }
        WindowEvent::CursorEntered { .. } => {
            app.pointer_inside_window = true;
            window.request_redraw();
        }
        WindowEvent::CursorLeft { .. } => {
            if message_dialog_active {
                app.pointer_left()?;
            } else {
                app.pointer_inside_window = false;
            }
            window.request_redraw();
        }
        WindowEvent::MouseInput {
            state,
            button: MouseButton::Left,
            ..
        } => {
            if message_dialog_active {
                app.handle_mouse_button(state)?;
            } else {
                let surface = app.graphics.surface();
                let (width, height) = (surface.width(), surface.height());
                let point = app.developer_console_pointer;
                match state {
                    ElementState::Pressed => {
                        app.developer_console
                            .handle_pointer_down(point, width, height);
                    }
                    ElementState::Released => {
                        let actions = app
                            .developer_console
                            .handle_pointer_up(point, width, height);
                        app.dispatch_developer_console_actions(actions)?;
                    }
                }
            }
            window.request_redraw();
        }
        WindowEvent::MouseWheel { delta, .. } => {
            if message_dialog_active {
                app.handle_mouse_wheel(delta, presenter.scale())?;
            } else {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => (y * 3.0).round() as i32,
                    MouseScrollDelta::PixelDelta(position) => (position.y / 15.0).round() as i32,
                };
                if lines != 0 {
                    app.developer_console.scroll_log(lines);
                    window.request_redraw();
                }
            }
        }
        WindowEvent::ModifiersChanged(modifiers) => {
            if message_dialog_active {
                app.handle_modifiers_changed(modifiers)?;
            } else {
                app.keyboard_modifiers = modifiers;
            }
        }
        WindowEvent::KeyboardInput {
            input:
                KeyboardInput {
                    state,
                    virtual_keycode: Some(key),
                    ..
                },
            ..
        } => {
            if message_dialog_active {
                app.handle_key(key, state)?;
                window.request_redraw();
                return Ok(());
            }
            let pressed = state == ElementState::Pressed;
            let alt_only = app.keyboard_modifiers == ModifiersState::ALT
                || app.keyboard_modifiers == (ModifiersState::ALT | ModifiersState::SHIFT);
            if pressed
                && alt_only
                && developer_console_menu_mnemonic(key)
                    .is_some_and(|mnemonic| app.developer_console.handle_menu_mnemonic(mnemonic))
            {
                window.request_redraw();
            } else if let Some(key) = map_developer_console_key(key) {
                let actions = app.developer_console.handle_key(key, pressed);
                app.dispatch_developer_console_actions(actions)?;
                window.request_redraw();
            }
        }
        WindowEvent::ReceivedCharacter(character)
            if !app
                .keyboard_modifiers
                .intersects(ModifiersState::ALT | ModifiersState::CTRL | ModifiersState::LOGO) =>
        {
            if message_dialog_active {
                app.handle_text_input(character)?;
                window.request_redraw();
            } else if app.developer_console.handle_character(character) {
                window.request_redraw();
            }
        }
        WindowEvent::Focused(focused) => {
            app.window_active = focused;
            if message_dialog_active {
                if focused {
                    app.handle_focus_gained()?;
                } else {
                    app.handle_focus_lost()?;
                }
            } else if !focused {
                app.keyboard_modifiers = ModifiersState::empty();
            }
            window.request_redraw();
        }
        _ => {}
    }
    if app.take_exit_request() {
        control_flow.set_exit();
    }
    Ok(())
}

pub(crate) fn handle_window_event(
    window: &Window,
    app: &mut GameApp,
    pixels: &mut Pixels,
    presenter: &mut clonk_scaling::FramePresenter,
    display_options: &mut DisplayOptions,
    event: WindowEvent,
    control_flow: &mut ControlFlow,
) -> Result<()> {
    if app.console_mode {
        return handle_developer_console_window_event(
            window,
            app,
            pixels,
            presenter,
            event,
            control_flow,
        );
    }
    match event {
        WindowEvent::CloseRequested => app.handle_window_close_requested(),
        WindowEvent::Resized(size)
        | WindowEvent::ScaleFactorChanged {
            new_inner_size: &mut size,
            ..
        } => {
            app.reject_classic_global_gui_bootstrap()?;
            let clamped = enforce_min_size(size);
            pixels
                .resize_surface(clamped.width, clamped.height)
                .context("failed to resize pixel surface")?;
            pixels
                .resize_buffer(clamped.width, clamped.height)
                .context("failed to resize pixel buffer")?;
            presenter.resize(clamped.width, clamped.height);
            let (logical_width, logical_height) = presenter.logical_size();
            app.resize(logical_width, logical_height)?;
            if display_options.mode == DisplayMode::Window {
                display_options.record_actual_size(clamped.width, clamped.height);
            }
            display_options.record_maximized(window.is_maximized());
        }
        WindowEvent::CursorMoved { position, .. } => {
            let (x, y) = presenter.position_to_gui(position.x, position.y);
            app.handle_cursor_moved(PhysicalPosition::new(x, y))
                .context("failed to process cursor movement")?;
        }
        WindowEvent::CursorEntered { .. } => {
            app.pointer_inside_window = true;
            window.request_redraw();
        }
        WindowEvent::CursorLeft { .. } => {
            app.pointer_left()
                .context("failed to process cursor exit")?;
            window.request_redraw();
        }
        WindowEvent::Focused(false) => {
            app.window_active = false;
            app.handle_focus_lost()
                .context("failed to clear controls after focus loss")?;
            window.request_redraw();
        }
        WindowEvent::MouseInput { state, button, .. } => match button {
            MouseButton::Left => app
                .handle_mouse_button(state)
                .context("failed to process left mouse button")?,
            MouseButton::Right => app
                .handle_right_mouse_button(state)
                .context("failed to process right mouse button")?,
            MouseButton::Middle => app
                .handle_other_mouse_button(state)
                .context("failed to process middle mouse button")?,
            // LegacyClonk's SDL frontend recognizes only left, right and
            // middle buttons; extra platform buttons never reach CMouse.
            _ => {}
        },
        WindowEvent::MouseWheel { delta, .. } => {
            app.handle_mouse_wheel(delta, presenter.scale())
                .context("failed to process mouse wheel")?;
        }
        WindowEvent::ModifiersChanged(modifiers) => {
            app.handle_modifiers_changed(modifiers)
                .context("failed to process keyboard modifiers")?;
        }
        WindowEvent::KeyboardInput {
            input:
                KeyboardInput {
                    state,
                    virtual_keycode: Some(keycode),
                    ..
                },
            ..
        } => {
            if state == ElementState::Pressed
                && keycode == VirtualKeyCode::F11
                && !app.options_keyboard_control_capture_active()
            {
                app.startup_tooltip.note_non_pointer_input();
                app.note_classic_lobby_non_pointer_input();
                if app.mode == AppMode::Menu {
                    app.menu_frame_cache = None;
                }
                app.reject_classic_global_gui_bootstrap()?;
                toggle_fullscreen(window, display_options);
                app.set_display_mode(display_options.mode);
                return Ok(());
            }
            app.handle_key(keycode, state)
                .context("failed to process key input")?;
            if !app.pending_screenshots.is_empty() {
                window.request_redraw();
            }
        }
        WindowEvent::ReceivedCharacter(character) => {
            app.handle_text_input(character)
                .context("failed to process text input")?;
        }
        WindowEvent::Moved(position) => {
            if display_options.mode == DisplayMode::Window && !window.is_maximized() {
                display_options.record_position(position.x, position.y);
            }
            display_options.record_maximized(window.is_maximized());
        }
        WindowEvent::Touch(touch) => {
            let (x, y) = presenter.position_to_gui(touch.location.x, touch.location.y);
            let position = GuiPoint::new(x as f32, y as f32);
            app.handle_touch(touch.phase, position)
                .context("failed to process touch input")?;
        }
        WindowEvent::Focused(true) => {
            app.handle_focus_gained()
                .context("failed to restore controls after focus gain")?;
            window.request_redraw();
        }
        _ => {}
    }
    if !app.pending_native_save_thumbnails.is_empty() {
        window.request_redraw();
    }
    if app.take_exit_request() {
        control_flow.set_exit();
    }
    Ok(())
}

pub(crate) fn enforce_min_size(size: PhysicalSize<u32>) -> PhysicalSize<u32> {
    PhysicalSize::new(size.width.max(1), size.height.max(1))
}

pub(crate) fn classic_platform_cursor_visible(
    window_active: bool,
    pointer_inside_window: bool,
) -> bool {
    !(window_active && pointer_inside_window)
}

fn toggle_fullscreen(window: &Window, display_options: &mut DisplayOptions) {
    if window.fullscreen().is_some() {
        window.set_fullscreen(None);
        display_options.record_mode(DisplayMode::Window);
        display_options.record_maximized(window.is_maximized());
    } else {
        window.set_fullscreen(Some(Fullscreen::Borderless(None)));
        display_options.record_mode(DisplayMode::Fullscreen);
        display_options.record_maximized(false);
    }
}

#[derive(Debug)]
pub(crate) struct MusicControlState {
    pub(crate) generation: u64,
    configured_volume: f32,
    pub(crate) scenario_level: Option<u8>,
    pub(crate) most_recently_played: Option<Arc<MusicAssetIdentity>>,
}

impl MusicControlState {
    pub(crate) fn new(configured_volume: f32) -> Self {
        Self {
            generation: 0,
            configured_volume: configured_volume.clamp(0.0, 1.0),
            scenario_level: None,
            most_recently_played: None,
        }
    }

    pub(crate) fn advance_generation(&mut self) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        if self.generation == 0 {
            self.generation = 1;
        }
        self.generation
    }

    pub(crate) fn set_scenario_level(&mut self, level: Option<u8>) {
        self.scenario_level = level.map(|level| level.min(100));
    }

    pub(crate) fn set_configured_volume(&mut self, volume: f32) {
        self.configured_volume = volume.clamp(0.0, 1.0);
    }

    pub(crate) fn effective_volume(&self) -> f32 {
        self.scenario_level.map_or(self.configured_volume, |level| {
            self.configured_volume * f32::from(level) / 100.0
        })
    }

    pub(crate) fn start_volume(&self, generation: u64) -> Option<f32> {
        (self.generation == generation).then(|| self.effective_volume())
    }
}

pub(crate) fn lock_unpoisoned<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Clears only the pending-load generation owned by one worker. A stale
/// replaced worker must never clear the pending state of its successor.
pub(crate) struct PendingMusicLoadGuard(pub(crate) Arc<AtomicU64>, pub(crate) u64);

impl Drop for PendingMusicLoadGuard {
    fn drop(&mut self) {
        let _ = self
            .0
            .compare_exchange(self.1, 0, AtomicOrdering::AcqRel, AtomicOrdering::Acquire);
    }
}

#[cfg(test)]
pub(crate) struct ControlledMusicLoadRequest {
    generation: u64,
    pub(crate) looped: bool,
    pub(crate) identity: Option<Arc<MusicAssetIdentity>>,
}

#[cfg(test)]
pub(crate) struct ControlledMusicLoads {
    fixture: MusicHandle,
    pub(crate) requests: VecDeque<ControlledMusicLoadRequest>,
}

enum MusicStartKind {
    Default {
        catalog: MusicCatalog,
        playlist: Option<String>,
        looped: bool,
    },
    Asset {
        asset: MusicAsset,
        looped: bool,
    },
    Data {
        data: Vec<u8>,
        looped: bool,
    },
}

pub(crate) struct QueuedMusicStart {
    order: u64,
    kind: MusicStartKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CachedObjectAudibilityMix {
    pub(crate) object_position: Vector2,
    pub(crate) audibility: u8,
    pub(crate) pan: i32,
}

pub(crate) struct AudioContext {
    pub(crate) system: AudioSystem,
    pub(crate) options: AudioOptions,
    /// Music loads run on a worker thread (a MIDI track is a FULL
    /// FluidSynth render — it must never block scenario activation like it
    /// never blocks C++'s streamed SDL_mixer playback). The generation
    /// counter cancels a pending load when the music is stopped/replaced
    /// before the decode finishes; the slot carries the finished handle.
    pub(crate) music_control: Arc<std::sync::Mutex<MusicControlState>>,
    pub(crate) pending_music: Arc<std::sync::Mutex<Option<MusicHandle>>>,
    pub(crate) music_load_pending: Arc<AtomicU64>,
    pub(crate) queued_music_starts: VecDeque<QueuedMusicStart>,
    next_music_start_order: u64,
    #[cfg(test)]
    pub(crate) controlled_music_loads: Option<ControlledMusicLoads>,
    #[cfg(test)]
    pub(crate) music_fade_requests: Vec<u32>,
    /// Successfully decoded samples in native C4SoundSystem list order.
    playable_sounds: Vec<PlayableSound>,
    /// Mirror used by existing diagnostics to report the prepared sample
    /// handles. Playback resolves the ordered catalog above directly.
    pub(crate) loaded_sounds: HashMap<String, SoundHandle>,
    next_sound_sample_order: usize,
    pub(crate) active_channels: HashMap<SoundInstanceKey, ChannelInfo>,
    next_sound_instance_order: u64,
    /// Reduced special-object `SetAudibilityAt` calls from the last completed
    /// graphics pass. A skipped or failed pass deliberately leaves this map
    /// untouched.
    pub(crate) rendered_object_audibility: HashMap<ObjectId, CachedObjectAudibilityMix>,
    pub(crate) resolver: SoundResolver,
    pub(crate) music_resolver: MusicResolver,
    pub(crate) missing_sounds: HashSet<String>,
}

impl AudioContext {
    #[cfg(test)]
    pub(crate) fn try_new(options: AudioOptions) -> Result<Self, AudioError> {
        Self::try_new_with_paths(options, None)
    }

    pub(crate) fn try_new_with_paths(
        options: AudioOptions,
        paths: Option<&AppPaths>,
    ) -> Result<Self, AudioError> {
        let music_control = Arc::new(std::sync::Mutex::new(MusicControlState::new(
            options.music_volume,
        )));
        let resampling_mode = if options.prefer_linear_resampling {
            ResamplingMode::Linear
        } else {
            ResamplingMode::Default
        };
        #[cfg(test)]
        let audio_resources_enabled = options.sound_enabled
            || options.music_enabled
            || options.menu_music_enabled
            || options.menu_sound_enabled;
        #[cfg(test)]
        let system = if audio_resources_enabled {
            AudioSystem::new_null_with_resampling(options.max_channels, resampling_mode)
        } else {
            AudioSystem::new_deferred_null_with_resampling(options.max_channels, resampling_mode)
        };
        #[cfg(not(test))]
        let system = AudioSystem::new_with_resampling(options.max_channels, resampling_mode)?;
        #[cfg(test)]
        let (resolver, music_resolver) = if audio_resources_enabled {
            (
                SoundResolver::new(),
                MusicResolver::discover_for_paths(paths),
            )
        } else {
            // Silent app fixtures exercise audio state and fail-closed UI
            // routes, but never resolve install media. Avoid walking every
            // Sound/Music/Extra group for each nextest subprocess.
            (SoundResolver::empty(), MusicResolver::empty())
        };
        #[cfg(not(test))]
        let (resolver, music_resolver) = (
            SoundResolver::new(),
            MusicResolver::discover_for_paths(paths),
        );
        let mut context = Self {
            system,
            options,
            music_control,
            pending_music: Arc::new(std::sync::Mutex::new(None)),
            music_load_pending: Arc::new(AtomicU64::new(0)),
            queued_music_starts: VecDeque::new(),
            next_music_start_order: 1,
            #[cfg(test)]
            controlled_music_loads: None,
            #[cfg(test)]
            music_fade_requests: Vec::new(),
            playable_sounds: Vec::new(),
            loaded_sounds: HashMap::new(),
            next_sound_sample_order: 0,
            active_channels: HashMap::new(),
            next_sound_instance_order: 1,
            rendered_object_audibility: HashMap::new(),
            resolver,
            music_resolver,
            missing_sounds: HashSet::new(),
        };
        context.refresh_sound_catalog();
        Ok(context)
    }

    #[cfg(test)]
    pub(crate) fn control_music_loads_with(&mut self, fixture: MusicHandle) {
        assert_eq!(
            self.music_load_pending.load(AtomicOrdering::Acquire),
            0,
            "controlled music loading must be installed before a request starts"
        );
        assert!(
            self.queued_music_starts.is_empty(),
            "controlled music loading must be installed before a request queues"
        );
        self.controlled_music_loads = Some(ControlledMusicLoads {
            fixture,
            requests: VecDeque::new(),
        });
    }

    #[cfg(test)]
    pub(crate) fn complete_next_controlled_music_load(&mut self) -> Result<bool, AudioError> {
        self.finish_next_controlled_music_load(true)
    }

    #[cfg(test)]
    pub(crate) fn fail_next_controlled_music_load(&mut self) -> Result<bool, AudioError> {
        self.finish_next_controlled_music_load(false)
    }

    #[cfg(test)]
    fn finish_next_controlled_music_load(
        &mut self,
        start_successfully: bool,
    ) -> Result<bool, AudioError> {
        let (request, fixture) = {
            let controlled = self
                .controlled_music_loads
                .as_mut()
                .expect("controlled music loading is not installed");
            let request = controlled
                .requests
                .pop_front()
                .expect("controlled music load queue is empty");
            (request, controlled.fixture.clone())
        };
        let _pending_guard =
            PendingMusicLoadGuard(Arc::clone(&self.music_load_pending), request.generation);
        let result = (|| -> Result<bool, AudioError> {
            let mut control = lock_unpoisoned(&self.music_control);
            let Some(volume) = control.start_volume(request.generation) else {
                return Ok(false);
            };
            if !start_successfully {
                Ok(false)
            } else {
                // The request retains its production loop flag for assertions. Keep
                // the silent fixture looping until the test explicitly advances the
                // lifecycle so host wall-clock progress cannot end it first.
                self.system.play_music(&fixture, true)?;
                self.system.music_set_volume(volume);
                if let Some(identity) = request.identity {
                    control.most_recently_played = Some(identity);
                }
                *lock_unpoisoned(&self.pending_music) = Some(fixture);
                Ok(true)
            }
        })();
        drop(_pending_guard);
        self.pump_queued_music_starts();
        result
    }

    pub(crate) fn play_music(&mut self, data: &[u8], looped: bool) -> Result<(), AudioError> {
        self.push_music_start(MusicStartKind::Data {
            data: data.to_vec(),
            looped,
        });
        self.pump_queued_music_starts();
        Ok(())
    }

    fn start_music_asset_now(
        &mut self,
        data: Vec<u8>,
        looped: bool,
        identity: Option<Arc<MusicAssetIdentity>>,
    ) {
        // Initialize the pull decoder off-thread. This retains the compressed
        // bytes and parses bounded source state (including a MIDI event
        // schedule) without rendering the complete track to PCM, matching
        // C++'s SDL_mixer ownership model.
        let generation = lock_unpoisoned(&self.music_control).generation;
        let worker = self.system.worker_handle();
        let control = Arc::clone(&self.music_control);
        let slot = Arc::clone(&self.pending_music);
        self.music_load_pending
            .store(generation, AtomicOrdering::Release);
        #[cfg(test)]
        if let Some(controlled) = self.controlled_music_loads.as_mut() {
            controlled.requests.push_back(ControlledMusicLoadRequest {
                generation,
                looped,
                identity,
            });
            return;
        }
        let load_pending = Arc::clone(&self.music_load_pending);
        std::thread::spawn(move || {
            let _pending_guard = PendingMusicLoadGuard(load_pending, generation);
            let music = match worker.load_music_owned(data) {
                Ok(music) => music,
                Err(error) => {
                    tracing::warn!(%error, "music decode failed");
                    return;
                }
            };
            let mut control = lock_unpoisoned(&control);
            let Some(volume) = control.start_volume(generation) else {
                return;
            };
            if let Err(error) = worker.play_music(&music, looped) {
                tracing::warn!(%error, "music playback failed");
                return;
            }
            worker.music_set_volume(volume);
            if let Some(identity) = identity {
                // C4MusicSystem updates mostRecentlyPlayed only after the
                // replacement successfully starts. A decode/play failure or
                // superseded worker must leave the previous exclusion intact.
                control.most_recently_played = Some(identity);
            }
            *lock_unpoisoned(&slot) = Some(music);
        });
    }

    fn push_music_start(&mut self, kind: MusicStartKind) -> u64 {
        let order = self.next_music_start_order;
        self.next_music_start_order = self.next_music_start_order.wrapping_add(1);
        if self.next_music_start_order == 0 {
            self.next_music_start_order = 1;
        }
        self.queued_music_starts
            .push_back(QueuedMusicStart { order, kind });
        order
    }

    fn enqueue_catalog_music_start(&mut self, kind: MusicStartKind) -> anyhow::Result<()> {
        let order = self.push_music_start(kind);
        let mut own_error = None;
        for (failed_order, error) in self.try_pump_queued_music_starts() {
            if failed_order == order {
                own_error = Some(error);
            } else {
                tracing::warn!(%error, "deferred music start failed");
            }
        }
        own_error.map_or(Ok(()), Err)
    }

    pub(crate) fn pump_queued_music_starts(&mut self) {
        for (_, error) in self.try_pump_queued_music_starts() {
            tracing::warn!(%error, "deferred music start failed");
        }
    }

    fn try_pump_queued_music_starts(&mut self) -> Vec<(u64, anyhow::Error)> {
        let mut failures = Vec::new();
        while self.music_load_pending.load(AtomicOrdering::Acquire) == 0 {
            let Some(start) = self.queued_music_starts.pop_front() else {
                break;
            };
            if let Err(error) = self.start_queued_music_now(start.kind) {
                failures.push((start.order, error));
                continue;
            }
        }
        failures
    }

    fn start_queued_music_now(&mut self, kind: MusicStartKind) -> anyhow::Result<()> {
        let (data, looped, identity) = match kind {
            MusicStartKind::Default {
                catalog,
                playlist,
                looped,
            } => {
                let recent = lock_unpoisoned(&self.music_control)
                    .most_recently_played
                    .clone();
                let selected = {
                    let _guard = CLASSIC_SAFE_RANDOM_LOCK
                        .lock()
                        .map_err(|_| anyhow!("classic SafeRandom lock was poisoned"))?;
                    catalog
                        .select_enabled_with(playlist.as_deref(), recent.as_ref(), |range| {
                            debug_assert!(range > 0);
                            // SAFETY: C rand takes no arguments and C guarantees a
                            // non-negative result. The process-global lock above
                            // serializes this shared unsynced stream with the loader.
                            (unsafe { rand() } as usize) % range
                        })
                        .cloned()
                }
                .ok_or_else(|| anyhow!("queued default music has no enabled asset"))?;
                let identity = Arc::clone(&selected.identity);
                let data = self
                    .read_resolved_music_asset(&selected)
                    .context("failed to read default music asset")?;
                (data, looped, Some(identity))
            }
            MusicStartKind::Asset { asset, looped } => {
                let identity = Arc::clone(&asset.identity);
                let name = String::from_utf8_lossy(&asset.file_name_bytes).into_owned();
                let data = self
                    .read_resolved_music_asset(&asset)
                    .with_context(|| format!("failed to read named music asset `{name}`"))?;
                (data, looped, Some(identity))
            }
            MusicStartKind::Data { data, looped } => {
                self.stop_current_music();
                (data, looped, None)
            }
        };
        self.start_music_asset_now(data, looped, identity);
        Ok(())
    }

    fn read_resolved_music_asset(&mut self, asset: &MusicAsset) -> Result<Vec<u8>, GroupError> {
        // C4MusicSystem::Play returns before Stop when selection fails, but a
        // resolved Song hard-stops before C4Group_ReadFile can fail.
        self.stop_current_music();
        asset.load_audio()
    }

    pub(crate) fn stop_music(&mut self) {
        self.queued_music_starts.clear();
        self.stop_current_music();
    }

    fn stop_current_music(&mut self) {
        self.music_load_pending.store(0, AtomicOrdering::Release);
        let mut control = lock_unpoisoned(&self.music_control);
        control.advance_generation();
        self.system.halt_music();
        lock_unpoisoned(&self.pending_music).take();
    }

    pub(crate) fn fade_out_music(&mut self, duration_ms: u32) -> bool {
        #[cfg(test)]
        self.music_fade_requests.push(duration_ms);
        // C4MusicSystem::Stop(fadeoutMS) retains its current song state and is
        // a strict no-op when SDL_mixer has no active music. A Rust-only
        // pending decode represents the song that C++ would already have
        // started synchronously, though, so invalidate that generation before
        // it can begin stale frontend playback during scenario loading.
        self.queued_music_starts.clear();
        let pending = self.music_load_pending.load(AtomicOrdering::Acquire) != 0;
        let mut playing = self.system.music_is_playing();
        if !pending && !playing {
            return false;
        }
        if pending {
            self.music_load_pending.store(0, AtomicOrdering::Release);
            lock_unpoisoned(&self.music_control).advance_generation();
            playing = self.system.music_is_playing();
            if !playing {
                lock_unpoisoned(&self.pending_music).take();
            }
        }
        self.system.music_fade_out(duration_ms)
    }

    pub(crate) fn set_scenario_music_level(&mut self, level: Option<u8>) {
        let mut control = lock_unpoisoned(&self.music_control);
        control.set_scenario_level(level);
        self.system.music_set_volume(control.effective_volume());
    }

    pub(crate) fn set_music_volume_percent(&mut self, value: i32) {
        self.options.set_music_volume_percent(value);
        let mut control = lock_unpoisoned(&self.music_control);
        control.set_configured_volume(self.options.music_volume);
        self.system.music_set_volume(control.effective_volume());
    }

    pub(crate) fn set_sound_volume_percent(&mut self, value: i32) {
        // C4SoundSystem reads Config.Sound.SoundVolume when each new sound is
        // started. Existing instances retain their current channel volume;
        // the startup sheet's callback immediately starts a fresh test sound.
        self.options.set_sound_volume_percent(value);
    }

    #[cfg(test)]
    pub(crate) fn process_audio(
        &mut self,
        snapshot: &SimulationSnapshot,
        runtime_music_enabled: &mut bool,
    ) {
        let _ = self.process_audio_with_viewports(snapshot, &[], runtime_music_enabled);
        self.update_channels(snapshot, &[], true);
    }

    pub(crate) fn process_audio_with_viewports(
        &mut self,
        snapshot: &SimulationSnapshot,
        viewports: &[ActiveViewportProjection],
        runtime_music_enabled: &mut bool,
    ) -> Vec<SpeechPlaybackOutcome> {
        self.pump_queued_music_starts();
        let events = &snapshot.audio;
        if !events.is_empty() {
            return self.handle_events(events, snapshot, viewports, runtime_music_enabled);
        }
        Vec::new()
    }

    pub(crate) fn cache_rendered_object_audibility(
        &mut self,
        calls: &RenderedObjectAudibilityCalls,
        snapshot: &SimulationSnapshot,
        viewports: &[ActiveViewportProjection],
    ) {
        let rendered_object_audibility = reduce_rendered_object_audibility(
            calls,
            snapshot,
            viewports,
            &self.rendered_object_audibility,
        );
        self.rendered_object_audibility = rendered_object_audibility;
    }

    pub(crate) fn reset_sfx(&mut self) {
        self.remove_sound_instances_matching(|_| true);
    }

    pub(crate) fn reset_sound_system_generation(&mut self) {
        self.reset_sfx();
        self.playable_sounds.clear();
        self.loaded_sounds.clear();
        self.missing_sounds.clear();
        self.next_sound_sample_order = 0;
        self.next_sound_instance_order = 1;
        self.rendered_object_audibility.clear();
        self.resolver.reset_dynamic_catalog();
        self.refresh_sound_catalog();
    }

    pub(crate) fn reset_music_system_generation(&mut self, paths: Option<&AppPaths>) {
        // C4Application::PreInit replaces the C4MusicSystem object while the
        // process audio backend survives. Invalidate the shared generation
        // rather than replacing it so an older Rust decode worker can never
        // become current again after this boundary.
        self.stop_music();
        lock_unpoisoned(&self.music_control).most_recently_played = None;
        self.set_scenario_music_level(None);
        #[cfg(test)]
        let music_resolver = if self.options.sound_enabled
            || self.options.music_enabled
            || self.options.menu_music_enabled
            || self.options.menu_sound_enabled
        {
            MusicResolver::discover_for_paths(paths)
        } else {
            // Silent app fixtures deliberately avoid install-media discovery
            // at construction and must retain that test-only shortcut across
            // their modeled PreInit transitions.
            MusicResolver::empty()
        };
        #[cfg(not(test))]
        let music_resolver = MusicResolver::discover_for_paths(paths);
        self.music_resolver = music_resolver;
    }

    pub(crate) fn clear_object_sound_instances(&mut self) {
        self.remove_sound_instances_matching(|info| info.target.is_some());
    }

    fn remove_sound_instances_matching(
        &mut self,
        mut should_remove: impl FnMut(&ChannelInfo) -> bool,
    ) {
        let keys = self
            .active_channels
            .iter()
            .filter(|(_, info)| should_remove(info))
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        for key in keys {
            if let Some(info) = self.active_channels.remove(&key) {
                if let Some(channel) = info.channel {
                    self.system.halt_channel(channel);
                }
            }
        }
    }

    pub(crate) fn configure_scenario(&mut self, path: Option<&Path>) {
        self.configure_scenario_with_resources(path, None, None);
    }

    pub(crate) fn configure_scenario_with_resources(
        &mut self,
        path: Option<&Path>,
        definition_roots: Option<&[Group]>,
        sound_effect_groups: Option<&[Group]>,
    ) {
        let (sound_catalog_changed, candidates) = match sound_effect_groups {
            Some(sound_effect_groups) => {
                let changed = self
                    .resolver
                    .configure_scenario_with_sound_effect_groups(path, sound_effect_groups);
                (
                    changed,
                    sound_catalog_candidates_for_groups(sound_effect_groups, "load"),
                )
            }
            None => {
                let changed = self.resolver.configure_scenario(path);
                let candidates = if changed && path.is_some() {
                    self.resolver.scenario_load_candidates()
                } else {
                    Vec::new()
                };
                (changed, candidates)
            }
        };
        if sound_catalog_changed || !candidates.is_empty() {
            self.apply_sound_candidates(candidates);
        }
        let configured = match definition_roots {
            Some(definition_roots) => self
                .music_resolver
                .configure_scenario_with_definition_roots(path, definition_roots),
            None => self.music_resolver.configure_scenario(path),
        };
        match configured {
            Ok(true) => {
                // C4MusicSystem::ClearSongs clears mostRecentlyPlayed when a
                // local scenario catalog replaces the global song list.
                if self.music_resolver.scenario_has_local_sources {
                    self.stop_music();
                    lock_unpoisoned(&self.music_control).most_recently_played = None;
                }
                self.set_scenario_music_level(path.map(|_| 100));
            }
            Ok(false) => {}
            Err(error) => {
                tracing::warn!(
                    path = ?path,
                    %error,
                    "failed to configure scenario music catalog"
                );
            }
        }
    }

    pub(crate) fn play_default_music(&mut self, looped: bool) -> anyhow::Result<bool> {
        let catalog = self.music_resolver.active_catalog().clone();
        let playlist = self.music_resolver.playlist.clone();
        if catalog.first_enabled(playlist.as_deref()).is_none() {
            return Ok(false);
        }
        self.enqueue_catalog_music_start(MusicStartKind::Default {
            catalog,
            playlist,
            looped,
        })
        .context("failed to play default music")?;
        Ok(true)
    }

    pub(crate) fn prepare_frontend_music(&mut self) {
        self.configure_scenario(None);
        self.set_scenario_music_level(None);
        self.music_resolver
            .set_playlist(Some("Frontend.*".to_string()));
    }

    pub(crate) fn play_frontend_music(&mut self) -> anyhow::Result<bool> {
        self.prepare_frontend_music();
        self.play_default_music(false)
    }

    pub(crate) fn play_named_music(&mut self, name: &str, looped: bool) -> anyhow::Result<bool> {
        let Some(selected) = self.music_resolver.resolve(name) else {
            return Ok(false);
        };
        self.enqueue_catalog_music_start(MusicStartKind::Asset {
            asset: selected.clone(),
            looped,
        })
        .with_context(|| format!("failed to play named music asset `{name}`"))?;
        Ok(true)
    }

    pub(crate) fn register_definition_sounds(&mut self, definition_id: &str, group: &Group) {
        self.resolver
            .register_definition_group(definition_id, group);
        let label = format!("definition::{definition_id}");
        let candidates = sound_catalog_candidates_for_groups(std::slice::from_ref(group), &label);
        self.apply_sound_candidates(candidates);
    }

    pub(crate) fn available_sound_samples(&self) -> Vec<String> {
        let mut names = self
            .playable_sounds
            .iter()
            .map(|sound| sound.sample_name.clone())
            .collect::<Vec<_>>();
        names.sort_unstable();
        names
    }

    pub(crate) fn available_music_tracks(&self) -> Vec<String> {
        self.music_resolver.active_filenames()
    }

    pub(crate) fn set_music_playlist(&mut self, playlist: Option<String>) {
        self.music_resolver.set_playlist(playlist);
    }

    fn music_enabled(&self) -> bool {
        self.options.music_enabled
    }

    pub(crate) fn menu_music_enabled(&self) -> bool {
        self.options.menu_music_enabled
    }

    pub(crate) fn music_is_playing(&self) -> bool {
        self.music_load_pending.load(AtomicOrdering::Acquire) != 0
            || !self.queued_music_starts.is_empty()
            || self.system.music_is_playing()
    }

    pub(crate) fn play_gui_sound(
        &mut self,
        name: &str,
        game_running: bool,
        snapshot: &SimulationSnapshot,
    ) {
        // C4GUI::GUISound has an outer FESamples gate even while a game is
        // running. StartSoundEffect's instance then independently follows
        // the current RXSound/FESamples gate in Instance::Execute.
        if !self.options.menu_sound_enabled {
            return;
        }
        if let Err(error) = self.try_start_global_effect(name, game_running, snapshot) {
            tracing::error!(sound = %name, %error, "failed to play GUI sound");
        }
    }

    pub(crate) fn start_lobby_elevator(&mut self, snapshot: &SimulationSnapshot) {
        if let Err(error) = self.try_start_global_sound("Elevator", true, false, snapshot) {
            tracing::error!(%error, "failed to start lobby countdown loop");
        }
    }

    pub(crate) fn stop_lobby_elevator(&mut self) {
        self.stop_sound("Elevator", None);
    }

    pub(crate) fn handle_events(
        &mut self,
        events: &[AudioCommand],
        snapshot: &SimulationSnapshot,
        viewports: &[ActiveViewportProjection],
        runtime_music_enabled: &mut bool,
    ) -> Vec<SpeechPlaybackOutcome> {
        let mut speech_outcomes = Vec::new();
        for event in events {
            match event {
                AudioCommand::PlaySound {
                    name,
                    target,
                    volume,
                    looped,
                    multiple,
                    custom_falloff,
                } => {
                    if let Err(err) = self.start_sound(
                        name,
                        *target,
                        i32::from(*volume),
                        *looped,
                        *multiple,
                        *custom_falloff,
                        snapshot,
                        viewports,
                    ) {
                        tracing::error!(sound = %name, error = %err, "failed to play sound");
                    }
                }
                AudioCommand::PlaySpeech {
                    name,
                    target,
                    fallback,
                } => {
                    let result = self.try_start_sound(
                        name, *target, 100, false, true, None, snapshot, viewports,
                    );
                    if let Some(fallback) = fallback.clone() {
                        match result {
                            Ok(true) => {
                                speech_outcomes.push(SpeechPlaybackOutcome::Played(fallback));
                            }
                            Ok(false) => {
                                speech_outcomes.push(SpeechPlaybackOutcome::Rejected(fallback));
                            }
                            Err(err) => {
                                tracing::error!(
                                    sound = %name,
                                    error = %err,
                                    "failed to play message speech"
                                );
                                speech_outcomes.push(SpeechPlaybackOutcome::Rejected(fallback));
                            }
                        }
                    } else if let Err(err) = result {
                        tracing::error!(
                            sound = %name,
                            error = %err,
                            "failed to play message speech"
                        );
                    }
                }
                AudioCommand::DetachObjectSounds { target, position } => {
                    self.detach_object_sounds(*target, *position, snapshot, viewports);
                }
                AudioCommand::PlaySoundAt { name, position } => {
                    let (volume, pan) =
                        compute_positional_mix_values(*position, snapshot, viewports);
                    if let Err(err) = self.try_start_sound_with_mix(
                        name,
                        None,
                        i32::from(volume),
                        false,
                        true,
                        None,
                        Some((f32::from(volume) / 100.0, pan)),
                        snapshot,
                        viewports,
                    ) {
                        tracing::error!(sound = %name, error = %err, "failed to play positional sound");
                    }
                }
                AudioCommand::StopSound { name, target } => {
                    self.stop_sound(name, *target);
                }
                AudioCommand::SetSoundVolume {
                    name,
                    target,
                    volume,
                } => {
                    let updated =
                        self.update_sound_volume(name, *target, *volume, snapshot, viewports);
                    if !updated {
                        if let Err(err) = self.start_sound(
                            name, *target, *volume, true, false, None, snapshot, viewports,
                        ) {
                            tracing::error!(
                                sound = %name,
                                error = %err,
                                "failed to start SoundLevel fallback loop"
                            );
                        }
                    }
                }
                AudioCommand::PlayMusic { name, looped } => {
                    *runtime_music_enabled = true;
                    let result = if name.is_empty() {
                        self.play_default_music(*looped)
                    } else {
                        self.play_named_music(name, *looped)
                    };
                    match result {
                        Ok(true) => {}
                        Ok(false) => {
                            tracing::warn!(music = %name, "missing music asset; keeping current playback")
                        }
                        Err(error) => {
                            tracing::warn!(music = %name, %error, "failed to play script music")
                        }
                    }
                }
                AudioCommand::StopMusic => {
                    *runtime_music_enabled = false;
                    self.stop_music();
                }
                AudioCommand::SetMusicLevel { level } => {
                    self.set_scenario_music_level(Some(*level));
                }
                AudioCommand::SetMusicPlaylist { playlist, restart } => {
                    self.set_music_playlist(playlist.clone());
                    if !*restart || !*runtime_music_enabled {
                        continue;
                    }
                    let result = self.play_default_music(false);
                    match result {
                        Ok(true) => {}
                        Ok(false) => {
                            tracing::warn!(
                                "playlist has no matching music asset; keeping current playback"
                            )
                        }
                        Err(error) => {
                            tracing::warn!(%error, "failed to restart filtered music")
                        }
                    }
                }
            }
        }
        speech_outcomes
    }

    pub(crate) fn start_sound(
        &mut self,
        name: &str,
        target: Option<ObjectId>,
        volume: i32,
        looped: bool,
        multiple: bool,
        custom_falloff: Option<i32>,
        snapshot: &SimulationSnapshot,
        viewports: &[ActiveViewportProjection],
    ) -> Result<(), AudioError> {
        self.try_start_sound(
            name,
            target,
            volume,
            looped,
            multiple,
            custom_falloff,
            snapshot,
            viewports,
        )
        .map(|_| ())
    }

    pub(crate) fn try_start_sound(
        &mut self,
        name: &str,
        target: Option<ObjectId>,
        volume: i32,
        looped: bool,
        multiple: bool,
        custom_falloff: Option<i32>,
        snapshot: &SimulationSnapshot,
        viewports: &[ActiveViewportProjection],
    ) -> Result<bool, AudioError> {
        self.try_start_sound_with_mix(
            name,
            target,
            volume,
            looped,
            multiple,
            custom_falloff,
            None,
            snapshot,
            viewports,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn try_start_sound_with_mix(
        &mut self,
        name: &str,
        target: Option<ObjectId>,
        volume: i32,
        looped: bool,
        multiple: bool,
        custom_falloff: Option<i32>,
        initial_mix: Option<(f32, f32)>,
        snapshot: &SimulationSnapshot,
        viewports: &[ActiveViewportProjection],
    ) -> Result<bool, AudioError> {
        let sound_enabled = self.options.sound_enabled;
        self.try_start_sound_with_mix_enabled(
            name,
            target,
            volume,
            looped,
            multiple,
            custom_falloff,
            initial_mix,
            snapshot,
            viewports,
            sound_enabled,
        )
    }

    pub(crate) fn try_start_global_effect(
        &mut self,
        name: &str,
        game_running: bool,
        snapshot: &SimulationSnapshot,
    ) -> Result<bool, AudioError> {
        self.try_start_global_sound(name, false, game_running, snapshot)
    }

    fn try_start_global_sound(
        &mut self,
        name: &str,
        looped: bool,
        game_running: bool,
        snapshot: &SimulationSnapshot,
    ) -> Result<bool, AudioError> {
        let sound_enabled = self.sound_effects_enabled(game_running);
        self.try_start_sound_with_mix_enabled(
            name,
            None,
            100,
            looped,
            true,
            None,
            None,
            snapshot,
            &[],
            sound_enabled,
        )
    }

    fn sound_effects_enabled(&self, game_running: bool) -> bool {
        if game_running {
            self.options.sound_enabled
        } else {
            self.options.menu_sound_enabled
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn try_start_sound_with_mix_enabled(
        &mut self,
        name: &str,
        target: Option<ObjectId>,
        volume: i32,
        looped: bool,
        multiple: bool,
        custom_falloff: Option<i32>,
        initial_mix: Option<(f32, f32)>,
        snapshot: &SimulationSnapshot,
        viewports: &[ActiveViewportProjection],
        sound_enabled: bool,
    ) -> Result<bool, AudioError> {
        // FnSound checks IsSoundPlaying before StartSoundEffect unless the
        // caller explicitly requests multiple instances (C4Script.cpp:
        // 2317-2319). FindInst matches the prepared request against resolved
        // sample names, so do this before resolution (and its SafeRandom
        // draw), as well as before asking the fixed-size mixer for a slot.
        if !multiple && self.active_channel_key(name, target).is_some() {
            return Ok(false);
        }
        let Some(resolved) = self.ensure_sound_with_key(name)? else {
            return Ok(false);
        };
        // C4SoundSystem caps non-looping instances per resolved sample before
        // channel allocation (C4SoundSystem.cpp:337-338).
        const MAX_SOUND_INSTANCES: usize = 20;
        if !looped
            && self
                .active_channels
                .values()
                .filter(|info| info.sample_key == resolved.sample_key)
                .count()
                >= MAX_SOUND_INSTANCES
        {
            return Ok(false);
        }
        // NewInstance rejects another instance of the resolved sample within
        // NearSoundRadius, including global/global pairs, even when FnSound's
        // fMultiple flag bypassed its exact-object check
        // (C4SoundSystem.cpp:341-350).
        let already_playing_near = self.active_channels.values().any(|info| {
            info.sample_key == resolved.sample_key
                && sound_targets_are_near(info.target, target, snapshot)
        });
        if already_playing_near {
            return Ok(false);
        }
        let duration_ms = resolved.handle.duration_ms().unwrap_or(0);
        let instance_order = self.next_sound_instance_order;
        self.next_sound_instance_order = self
            .next_sound_instance_order
            .checked_add(1)
            .expect("sound instance insertion order overflowed");
        let mut key = SoundInstanceKey::new(name, target);
        if self.active_channels.contains_key(&key) {
            key.discriminator = instance_order;
        }
        let mut info = ChannelInfo {
            channel: None,
            handle: resolved.handle,
            duration_ms,
            sample_key: resolved.sample_key,
            sample_name: resolved.sample_name,
            sample_order: resolved.sample_order,
            instance_order,
            looped,
            target,
            volume,
            custom_falloff,
            started_at: Instant::now(),
            detached_mix: initial_mix,
        };
        let (mut mix_volume, pan) = compute_mix_values_with_rendered_audibility(
            &mut info,
            snapshot,
            viewports,
            Some(&self.rendered_object_audibility),
        );
        mix_volume *= self.options.sound_volume;
        if sound_enabled && mix_volume > 0.0 {
            let channel = self.system.play_sound(&info.handle, looped)?;
            self.system
                .channel_set_volume_and_pan(channel, mix_volume, pan);
            info.channel = Some(channel);
        }
        let replaced = self.active_channels.insert(key, info);
        assert!(replaced.is_none(), "sound instance key must be unique");
        Ok(true)
    }

    pub(crate) fn stop_sound(&mut self, name: &str, target: Option<ObjectId>) {
        let Some(key) = self.active_channel_key(name, target) else {
            return;
        };
        if let Some(info) = self.active_channels.remove(&key) {
            if let Some(channel) = info.channel {
                self.system.halt_channel(channel);
            }
        }
    }

    pub(crate) fn active_channel_key(
        &self,
        name: &str,
        target: Option<ObjectId>,
    ) -> Option<SoundInstanceKey> {
        let pattern = SoundSearchTerms::new(name).prepared_pattern();
        // C4SoundSystem::FindInst walks samples in catalog order, then each
        // sample's instances in insertion order. Detached one-shots have a
        // null object just like true global instances; neither is preferred.
        self.active_channels
            .iter()
            .filter(|(_, info)| {
                info.target == target && matches_sound_pattern(&pattern, &info.sample_name)
            })
            .min_by_key(|(_, info)| (info.sample_order, info.instance_order))
            .map(|(key, _)| key.clone())
    }

    pub(crate) fn detach_object_sounds(
        &mut self,
        target: ObjectId,
        position: Vector2,
        snapshot: &SimulationSnapshot,
        viewports: &[ActiveViewportProjection],
    ) {
        let detached_mix = compute_object_positional_mix(position, snapshot, viewports);
        let mut looping = Vec::new();
        let mut updates = Vec::new();
        for (key, info) in &mut self.active_channels {
            if info.target != Some(target) {
                continue;
            }
            if info.looped {
                looping.push(key.clone());
                continue;
            }
            info.target = None;
            info.detached_mix = Some(detached_mix);
            if let Some(channel) = info.channel {
                updates.push(channel);
            }
        }
        for channel in updates {
            self.system.channel_set_volume_and_pan(
                channel,
                detached_mix.0 * self.options.sound_volume,
                detached_mix.1,
            );
        }
        for key in looping {
            if let Some(info) = self.active_channels.remove(&key) {
                if let Some(channel) = info.channel {
                    self.system.halt_channel(channel);
                }
            }
        }
    }

    fn update_sound_volume(
        &mut self,
        name: &str,
        target: Option<ObjectId>,
        volume: i32,
        snapshot: &SimulationSnapshot,
        viewports: &[ActiveViewportProjection],
    ) -> bool {
        let Some(key) = self.active_channel_key(name, target) else {
            return false;
        };
        let rendered_object_audibility = &self.rendered_object_audibility;
        if let Some(info) = self.active_channels.get_mut(&key) {
            info.volume = volume;
            if info.target.is_none() {
                if let Some((_, pan)) = info.detached_mix {
                    info.detached_mix = Some(((volume as f32 / 100.0).max(0.0), pan));
                }
            }
            let Some(channel) = info.channel else {
                return true;
            };
            let (mut mix_volume, pan) = compute_mix_values_with_rendered_audibility(
                info,
                snapshot,
                viewports,
                Some(rendered_object_audibility),
            );
            mix_volume *= self.options.sound_volume;
            self.system
                .channel_set_volume_and_pan(channel, mix_volume, pan);
            return true;
        }
        false
    }

    /// Apply a completed draw's object mix without running a second
    /// finished/half-duration sweep. Native order matters here: an inaudible
    /// attached instance can release a scarce channel before a later newly
    /// audible one restores it in the same post-graphics pass.
    pub(crate) fn refresh_attached_channel_mix_after_render(
        &mut self,
        snapshot: &SimulationSnapshot,
        viewports: &[ActiveViewportProjection],
    ) {
        if !self.sound_effects_enabled(true) {
            return;
        }
        let rendered_object_audibility = &self.rendered_object_audibility;
        let mut ordered_channels = self
            .active_channels
            .iter()
            .filter_map(|(key, info)| {
                info.target.and_then(|target| snapshot.object(target))?;
                Some(((info.sample_order, info.instance_order), key.clone()))
            })
            .collect::<Vec<_>>();
        ordered_channels.sort_unstable_by_key(|(order, _)| *order);

        // The ordinary pre-render sound tick already swept everything that
        // was finished then. Remove only attached channels that completed
        // during the render, before any restoration can reuse their slot.
        let finished_during_render = ordered_channels
            .iter()
            .filter_map(|(_, key)| {
                let info = self.active_channels.get(key)?;
                info.channel
                    .is_some_and(|channel| !self.system.channel_is_playing(channel))
                    .then_some(key.clone())
            })
            .collect::<Vec<_>>();
        for key in finished_during_render {
            if let Some(info) = self.active_channels.remove(&key) {
                if let Some(channel) = info.channel {
                    self.system.halt_channel(channel);
                }
            }
        }

        let mut failed_restores = Vec::new();
        for (_, key) in ordered_channels {
            let Some(info) = self.active_channels.get_mut(&key) else {
                continue;
            };
            let (mut mix_volume, pan) = compute_mix_values_with_rendered_audibility(
                info,
                snapshot,
                viewports,
                Some(rendered_object_audibility),
            );
            mix_volume *= self.options.sound_volume;
            if mix_volume <= 0.0 {
                if let Some(channel) = info.channel.take() {
                    self.system.halt_channel(channel);
                }
                continue;
            }
            let channel = match info.channel {
                Some(channel) => channel,
                None => match self.system.play_sound(&info.handle, info.looped) {
                    Ok(channel) => {
                        info.channel = Some(channel);
                        channel
                    }
                    Err(error) => {
                        tracing::warn!(
                            %error,
                            sound = %info.sample_key,
                            "failed to restore attached sound after render"
                        );
                        failed_restores.push(key.clone());
                        continue;
                    }
                },
            };
            self.system
                .channel_set_volume_and_pan(channel, mix_volume, pan);
        }
        for key in failed_restores {
            if let Some(info) = self.active_channels.remove(&key) {
                if let Some(channel) = info.channel {
                    self.system.halt_channel(channel);
                }
            }
        }
    }

    pub(crate) fn update_channels(
        &mut self,
        snapshot: &SimulationSnapshot,
        viewports: &[ActiveViewportProjection],
        game_running: bool,
    ) {
        let now = Instant::now();
        let mut finished = Vec::new();
        let mut updates: Vec<(ChannelId, f32, f32)> = Vec::new();
        let rendered_object_audibility = &self.rendered_object_audibility;
        if !self.sound_effects_enabled(game_running) {
            for (key, info) in self.active_channels.iter_mut() {
                if info
                    .target
                    .is_some_and(|target| snapshot.object(target).is_none())
                {
                    if info.looped {
                        finished.push(key.clone());
                        continue;
                    }
                    info.target = None;
                }
                if let Some(channel) = info.channel {
                    if !self.system.channel_is_playing(channel) {
                        finished.push(key.clone());
                        continue;
                    }
                    self.system.halt_channel(channel);
                    info.channel = None;
                } else if info.non_looping_past_half_duration(now) {
                    finished.push(key.clone());
                }
            }
            for key in finished {
                if let Some(info) = self.active_channels.remove(&key) {
                    if let Some(channel) = info.channel {
                        self.system.halt_channel(channel);
                    }
                }
            }
            return;
        }
        // C4SoundSystem::Execute walks its sample list and then each sample's
        // instance list. Preserve that order for the whole enabled pass: an
        // earlier instance may release a scarce channel before a later one
        // restores.
        let mut ordered_channels = self
            .active_channels
            .iter()
            .map(|(key, info)| ((info.sample_order, info.instance_order), key.clone()))
            .collect::<Vec<_>>();
        ordered_channels.sort_unstable_by_key(|(order, _)| *order);
        for (_, key) in ordered_channels {
            let info = self
                .active_channels
                .get_mut(&key)
                .expect("ordered sound instance remains live during update");
            if info
                .target
                .is_some_and(|target| snapshot.object(target).is_none())
            {
                if info.looped {
                    if let Some(channel) = info.channel.take() {
                        self.system.halt_channel(channel);
                    }
                    finished.push(key.clone());
                    continue;
                }
                info.target = None;
            }
            match info.channel {
                Some(channel) if !self.system.channel_is_playing(channel) => {
                    finished.push(key.clone());
                    continue;
                }
                None if info.non_looping_past_half_duration(now) => {
                    finished.push(key.clone());
                    continue;
                }
                _ => {}
            }
            let (mut mix_volume, pan) = compute_mix_values_with_rendered_audibility(
                info,
                snapshot,
                viewports,
                Some(rendered_object_audibility),
            );
            mix_volume *= self.options.sound_volume;
            if mix_volume <= 0.0 {
                if let Some(channel) = info.channel.take() {
                    self.system.halt_channel(channel);
                }
                continue;
            }
            let channel = match info.channel {
                Some(channel) => channel,
                None => match self.system.play_sound(&info.handle, info.looped) {
                    Ok(channel) => {
                        info.channel = Some(channel);
                        channel
                    }
                    Err(error) => {
                        tracing::warn!(%error, sound = %info.sample_key, "failed to restore sound channel");
                        finished.push(key.clone());
                        continue;
                    }
                },
            };
            updates.push((channel, mix_volume, pan));
        }
        for (channel, volume, pan) in updates {
            self.system.channel_set_volume_and_pan(channel, volume, pan);
        }
        for key in finished {
            if let Some(info) = self.active_channels.remove(&key) {
                if let Some(channel) = info.channel {
                    self.system.halt_channel(channel);
                }
            }
        }
    }

    pub(crate) fn refresh_sound_catalog(&mut self) {
        let candidates = self.resolver.load_candidates();
        self.reset_sfx();
        self.playable_sounds.clear();
        self.loaded_sounds.clear();
        self.missing_sounds.clear();
        self.next_sound_sample_order = 0;
        self.apply_sound_candidates(candidates);
    }

    fn apply_sound_candidates(&mut self, candidates: Vec<SoundCatalogCandidate>) {
        for candidate in candidates {
            let missing_key = format!("asset::{}", candidate.cache_marker);
            let bytes = match candidate.load_audio() {
                Ok(bytes) => bytes,
                Err(error) => {
                    self.missing_sounds.insert(missing_key);
                    tracing::warn!(
                        sound = %candidate.file_name,
                        library = %candidate.description,
                        %error,
                        "failed to read sound candidate; keeping previous decoded sample"
                    );
                    continue;
                }
            };
            let handle = match self.system.load_sound(&bytes) {
                Ok(handle) => handle,
                Err(error) => {
                    self.missing_sounds.insert(missing_key);
                    tracing::warn!(
                        sound = %candidate.file_name,
                        library = %candidate.description,
                        %error,
                        "failed to decode sound candidate; keeping previous decoded sample"
                    );
                    continue;
                }
            };
            self.missing_sounds.remove(&missing_key);
            // LoadEffects appends the new Sample and only then erases the
            // previous case-insensitive filename match. A failed later load
            // therefore leaves the prior handle, list position and instances.
            if let Some(old_index) = self
                .playable_sounds
                .iter()
                .position(|sound| sound.sample_name == candidate.file_name)
            {
                self.remove_sound_instances_matching(|info| {
                    info.sample_name == candidate.file_name
                });
                self.playable_sounds.remove(old_index);
            }
            let sample_order = self.next_sound_sample_order;
            self.next_sound_sample_order = self
                .next_sound_sample_order
                .checked_add(1)
                .expect("sound sample insertion order overflowed");
            self.playable_sounds.push(PlayableSound {
                handle,
                sample_key: candidate.cache_key,
                sample_name: candidate.file_name,
                sample_order,
            });
        }

        self.loaded_sounds.clear();
        for sound in &self.playable_sounds {
            self.loaded_sounds
                .insert(sound.sample_key.clone(), sound.handle.clone());
        }
    }

    pub(crate) fn ensure_sound_with_key(
        &mut self,
        name: &str,
    ) -> Result<Option<LoadedSound>, AudioError> {
        let request_key = name.to_ascii_lowercase();
        let terms = SoundSearchTerms::new(name);
        let selected_index = if let Some(pattern) = terms.wildcard_pattern.as_deref() {
            let matches = self
                .playable_sounds
                .iter()
                .enumerate()
                .filter_map(|(index, sound)| {
                    matches_sound_pattern(pattern, &sound.sample_name).then_some(index)
                })
                .collect::<Vec<_>>();
            (!matches.is_empty()).then(|| matches[classic_safe_random(matches.len())])
        } else {
            terms.search_names.iter().find_map(|search_name| {
                self.playable_sounds
                    .iter()
                    .position(|sound| sound.sample_name == *search_name)
            })
        };
        if let Some(selected_index) = selected_index {
            let sound = &self.playable_sounds[selected_index];
            return Ok(Some(LoadedSound {
                handle: sound.handle.clone(),
                sample_key: sound.sample_key.clone(),
                sample_name: sound.sample_name.clone(),
                sample_order: sound.sample_order,
            }));
        }

        if self
            .missing_sounds
            .insert(format!("request::{request_key}"))
        {
            tracing::warn!(sound = %name, "missing sound asset; skipping playback");
        }
        Ok(None)
    }
}

/// Configure the app-owned sound resolver before any definition/scenario
/// callback can call the Message family, then hand the engine only the
/// client-local sample filenames needed for StartSoundEffect's success gate.
/// A missing AudioContext models a missing C++ Application.AudioSystem and
/// therefore exposes an empty catalog so speech falls back to text.
pub(crate) fn configure_scenario_sound_samples(
    audio: Option<&mut AudioContext>,
    scenario: &Scenario,
    path: &Path,
) -> Vec<String> {
    let Some(audio) = audio else {
        return Vec::new();
    };
    audio.set_music_playlist(None);
    audio.configure_scenario_with_resources(
        Some(path),
        Some(scenario.definition_root_groups()),
        Some(scenario.sound_effect_groups()),
    );
    audio.available_sound_samples()
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct SoundInstanceKey {
    name: String,
    target: Option<ObjectId>,
    /// The first live request keeps discriminator zero for stable lookup in
    /// diagnostics/tests. Concurrent identical requests receive their C++
    /// instance insertion order so no channel can be orphaned by replacement.
    discriminator: u64,
}

impl SoundInstanceKey {
    pub(crate) fn new(name: &str, target: Option<ObjectId>) -> Self {
        Self {
            name: name.to_ascii_lowercase(),
            target,
            discriminator: 0,
        }
    }
}

struct PlayableSound {
    handle: SoundHandle,
    sample_key: String,
    sample_name: String,
    sample_order: usize,
}

pub(crate) struct LoadedSound {
    pub(crate) handle: SoundHandle,
    pub(crate) sample_key: String,
    sample_name: String,
    pub(crate) sample_order: usize,
}

#[derive(Clone)]
pub(crate) struct ChannelInfo {
    pub(crate) channel: Option<ChannelId>,
    pub(crate) handle: SoundHandle,
    pub(crate) duration_ms: u32,
    pub(crate) sample_key: String,
    /// Lowercase basename stored by C4SoundSystem::Sample::name.
    pub(crate) sample_name: String,
    /// Effective sample-catalog order used by C++ FindInst.
    pub(crate) sample_order: usize,
    /// Stable order within one sample's instance list.
    pub(crate) instance_order: u64,
    pub(crate) looped: bool,
    pub(crate) target: Option<ObjectId>,
    pub(crate) volume: i32,
    pub(crate) custom_falloff: Option<i32>,
    pub(crate) started_at: Instant,
    /// C4SoundSystem::Instance::DetachObj replaces the scripted volume/pan
    /// with GetVolumeByPos at the object's final position. While attached,
    /// cache that raw positional pair as a fallback for legacy/missed detach
    /// events; once target is None it is the immutable detached mix.
    pub(crate) detached_mix: Option<(f32, f32)>,
}

impl ChannelInfo {
    fn playback_position_ms(&self, now: Instant) -> u32 {
        if self.duration_ms == 0 {
            return 0;
        }
        let elapsed_ms = now.saturating_duration_since(self.started_at).as_millis();
        u32::try_from(elapsed_ms % u128::from(self.duration_ms))
            .expect("position is below duration")
    }

    pub(crate) fn non_looping_past_half_duration(&self, now: Instant) -> bool {
        !self.looped
            && self.channel.is_none()
            && self.playback_position_ms(now) > self.duration_ms / 2
    }
}

fn sound_targets_are_near(
    existing: Option<ObjectId>,
    requested: Option<ObjectId>,
    snapshot: &SimulationSnapshot,
) -> bool {
    const NEAR_SOUND_RADIUS: i64 = 50;
    match (existing, requested) {
        (None, None) => true,
        (Some(existing), Some(requested)) if existing == requested => true,
        (Some(existing), Some(requested)) => snapshot
            .object(existing)
            .zip(snapshot.object(requested))
            .is_some_and(|(existing, requested)| {
                let dx = i64::from(existing.position.x) - i64::from(requested.position.x);
                let dy = i64::from(existing.position.y) - i64::from(requested.position.y);
                dx * dx + dy * dy <= NEAR_SOUND_RADIUS * NEAR_SOUND_RADIUS
            }),
        _ => false,
    }
}

pub(crate) struct SoundResolver {
    pub(crate) global: Vec<SoundLibrary>,
    pub(crate) scenario: Vec<SoundLibrary>,
    pub(crate) scenario_root: Option<PathBuf>,
    pub(crate) registered_definitions: HashSet<String>,
    pub(crate) definition_library_count: usize,
    pub(crate) base_sample_loads: Vec<String>,
    pub(crate) definition_sample_loads: Vec<String>,
    pub(crate) scenario_sample_loads: Vec<String>,
    pub(crate) sample_ranks: HashMap<String, usize>,
    pub(crate) sample_ranks_prebuilt: bool,
}

impl SoundResolver {
    pub(crate) fn empty() -> Self {
        Self {
            global: Vec::new(),
            scenario: Vec::new(),
            scenario_root: None,
            registered_definitions: HashSet::new(),
            definition_library_count: 0,
            base_sample_loads: Vec::new(),
            definition_sample_loads: Vec::new(),
            scenario_sample_loads: Vec::new(),
            sample_ranks: HashMap::new(),
            sample_ranks_prebuilt: false,
        }
    }

    fn new() -> Self {
        let (global, base_sample_loads) = discover_global_sound_libraries();
        let mut resolver = Self::empty();
        resolver.global = global;
        resolver.base_sample_loads = base_sample_loads;
        resolver.rebuild_sample_ranks();
        resolver
    }

    pub(crate) fn configure_scenario(&mut self, path: Option<&Path>) -> bool {
        let new_root = path.map(|p| p.to_path_buf());
        if self.scenario_root.as_deref() == new_root.as_deref() {
            return false;
        }
        let sound_effect_groups = path
            .map(collect_path_sound_effect_groups)
            .unwrap_or_default();
        self.install_scenario_sound_effect_groups(new_root, &sound_effect_groups);
        true
    }

    pub(crate) fn configure_scenario_with_sound_effect_groups(
        &mut self,
        path: Option<&Path>,
        sound_effect_groups: &[Group],
    ) -> bool {
        self.install_scenario_sound_effect_groups(path.map(Path::to_path_buf), sound_effect_groups);
        // A resource-aware call is a real activation/reload. C++ reconstructs
        // the sample bank even when the scenario path is unchanged.
        true
    }

    fn install_scenario_sound_effect_groups(
        &mut self,
        new_root: Option<PathBuf>,
        sound_effect_groups: &[Group],
    ) {
        self.clear_registered_definition_libraries();
        self.scenario = sound_effect_groups
            .iter()
            .enumerate()
            .rev()
            .filter_map(|(index, group)| {
                let label = format!("load::{index}::{}", group.root().display());
                collect_direct_sound_library(group, label)
            })
            .collect();
        self.scenario_root = new_root;
        self.definition_sample_loads.clear();
        self.scenario_sample_loads = sound_effect_groups
            .iter()
            .flat_map(direct_sound_sample_loads)
            .collect();
        self.sample_ranks_prebuilt =
            self.scenario_root.is_some() || !sound_effect_groups.is_empty();
        self.rebuild_sample_ranks();
    }

    fn clear_registered_definition_libraries(&mut self) {
        if self.definition_library_count != 0 {
            self.global.drain(0..self.definition_library_count);
            self.definition_library_count = 0;
        }
        self.registered_definitions.clear();
    }

    pub(crate) fn resolve_entry(&self, name: &str) -> Option<ResolvedSound<'_>> {
        self.resolve_entry_with_random(name, classic_safe_random)
    }

    pub(crate) fn resolve_entry_with_random(
        &self,
        name: &str,
        next_random: impl FnOnce(usize) -> usize,
    ) -> Option<ResolvedSound<'_>> {
        let terms = SoundSearchTerms::new(name);
        if let Some(pattern) = terms.wildcard_pattern.as_deref() {
            let mut matches = Vec::new();
            let mut seen_file_names = HashSet::new();
            for library in self.scenario.iter().chain(self.global.iter()) {
                for entry_index in library.wildcard_match_indices(pattern) {
                    if seen_file_names.insert(library.entries[entry_index].file_name.as_str()) {
                        matches.push((library, entry_index));
                    }
                }
            }
            if matches.is_empty() {
                return None;
            }
            matches.sort_by_key(|(library, entry_index)| {
                self.sample_order(&library.entries[*entry_index].file_name)
            });
            let selected = next_random(matches.len());
            let (library, entry_index) = matches.get(selected).copied()?;
            return Some(ResolvedSound {
                library,
                entry_index,
            });
        }

        for library in self.scenario.iter().chain(self.global.iter()) {
            if let Some(index) = library.find_exact_entry(&terms.search_names) {
                return Some(ResolvedSound {
                    library,
                    entry_index: index,
                });
            }
        }
        None
    }

    pub(crate) fn sample_names(&self) -> Vec<String> {
        let mut names = self
            .scenario
            .iter()
            .chain(&self.global)
            .flat_map(|library| library.entries.iter().map(|entry| entry.file_name.clone()))
            .collect::<Vec<_>>();
        names.sort_unstable();
        names.dedup();
        names
    }

    fn load_candidates(&self) -> Vec<SoundCatalogCandidate> {
        let mut candidates = Vec::new();
        // Resolver precedence is newest-first. Native LoadEffects order is
        // the reverse: global bank, definition groups in traversal order,
        // then scenario-local groups. Within one group it performs complete
        // WAV, OGG and MP3 scans in that order.
        for library in self.global.iter().rev().chain(self.scenario.iter().rev()) {
            candidates.extend(library.catalog_candidates());
        }
        candidates
    }

    fn scenario_load_candidates(&self) -> Vec<SoundCatalogCandidate> {
        self.scenario
            .iter()
            .rev()
            .flat_map(SoundLibrary::catalog_candidates)
            .collect()
    }

    pub(crate) fn sample_order(&self, file_name: &str) -> usize {
        *self
            .sample_ranks
            .get(file_name)
            .expect("every resolver-visible sound sample has a catalog rank")
    }

    pub(crate) fn rebuild_sample_ranks(&mut self) {
        self.sample_ranks.clear();
        for (serial, file_name) in self
            .base_sample_loads
            .iter()
            .chain(&self.definition_sample_loads)
            .chain(&self.scenario_sample_loads)
            .enumerate()
        {
            // C++ appends a successfully loaded replacement and then erases
            // the old sample. The final sample-vector order is therefore the
            // order of each filename's last load occurrence.
            self.sample_ranks.insert(file_name.clone(), serial);
        }
        let mut next_rank = self
            .sample_ranks
            .values()
            .copied()
            .max()
            .map_or(0, |rank| rank + 1);
        // Resolver-only fallback sources are not part of native C4DefList
        // traversal, but still need a deterministic total order.
        for library in self.scenario.iter().chain(self.global.iter()) {
            for entry in &library.entries {
                if let std::collections::hash_map::Entry::Vacant(slot) =
                    self.sample_ranks.entry(entry.file_name.clone())
                {
                    slot.insert(next_rank);
                    next_rank += 1;
                }
            }
        }
    }

    pub(crate) fn register_definition_group(&mut self, definition_id: &str, group: &Group) {
        let key = format!(
            "{}::{}",
            definition_id.to_ascii_lowercase(),
            group.root().to_string_lossy().to_ascii_lowercase()
        );
        if !self.registered_definitions.insert(key) {
            return;
        }
        let label = format!("definition::{}", definition_id);
        if let Some(library) = collect_direct_sound_library(group, label) {
            self.definition_library_count += 1;
            self.global.insert(0, library);
            if !self.sample_ranks_prebuilt {
                self.definition_sample_loads
                    .extend(direct_sound_sample_loads(group));
            }
            self.rebuild_sample_ranks();
        }
    }

    fn reset_dynamic_catalog(&mut self) {
        self.install_scenario_sound_effect_groups(None, &[]);
    }
}

pub(crate) struct SoundLibrary {
    label: String,
    cache_prefix: String,
    source: Arc<Group>,
    entries: Vec<SoundEntry>,
    by_file_name: HashMap<String, Vec<usize>>,
}

impl SoundLibrary {
    fn new(label: String, source: Arc<Group>) -> Self {
        let cache_prefix = source.root().to_string_lossy().to_ascii_lowercase();
        Self {
            label,
            cache_prefix,
            source,
            entries: Vec::new(),
            by_file_name: HashMap::new(),
        }
    }

    fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn add_entry(&mut self, relative_path: PathBuf) {
        let file_name = relative_path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| relative_path.to_string_lossy().to_string());
        let file_key = file_name.to_ascii_lowercase();
        let entry = SoundEntry {
            relative_path,
            file_name: file_key.clone(),
            extension_rank: extension_rank(
                Path::new(&file_name)
                    .extension()
                    .and_then(|ext| ext.to_str()),
            ),
        };
        let index = self.entries.len();
        self.entries.push(entry);
        self.by_file_name.entry(file_key).or_default().push(index);
    }

    fn find_exact_entry(&self, search_names: &[String]) -> Option<usize> {
        for file_name in search_names {
            if let Some(indices) = self.by_file_name.get(file_name) {
                return Some(self.pick_best_index(indices));
            }
        }
        None
    }

    fn wildcard_match_indices<'a>(&'a self, pattern: &'a str) -> impl Iterator<Item = usize> + 'a {
        self.entries
            .iter()
            .enumerate()
            .filter_map(move |(index, entry)| {
                matches_sound_pattern(pattern, &entry.file_name).then_some(index)
            })
    }

    fn pick_best_index(&self, indices: &[usize]) -> usize {
        let mut best = *indices.first().unwrap();
        let mut best_rank = self.entries[best].extension_rank;
        for &index in indices.iter().skip(1) {
            let rank = self.entries[index].extension_rank;
            if rank < best_rank || (rank == best_rank && index > best) {
                best = index;
                best_rank = rank;
            }
        }
        best
    }

    fn cache_key(&self, index: usize) -> String {
        format!(
            "{}::{}",
            self.cache_prefix,
            self.entries[index]
                .relative_path
                .to_string_lossy()
                .to_ascii_lowercase()
        )
    }

    fn cache_marker(&self, index: usize) -> String {
        self.cache_key(index)
    }

    fn describe_entry(&self, index: usize) -> String {
        format!(
            "{}::{}",
            self.label,
            self.entries[index].relative_path.display()
        )
    }

    fn read_bytes(&self, index: usize) -> Result<Vec<u8>, clonk_resources::GroupError> {
        self.source.read_file(&self.entries[index].relative_path)
    }

    fn catalog_candidate(&self, index: usize) -> SoundCatalogCandidate {
        SoundCatalogCandidate {
            cache_key: self.cache_key(index),
            cache_marker: self.cache_marker(index),
            file_name: self.entries[index].file_name.clone(),
            description: self.describe_entry(index),
            source: Arc::clone(&self.source),
            relative_path: self.entries[index].relative_path.clone(),
        }
    }

    fn catalog_candidates(&self) -> Vec<SoundCatalogCandidate> {
        let mut indices = (0..self.entries.len()).collect::<Vec<_>>();
        indices.sort_by_key(|index| (self.entries[*index].extension_rank, *index));
        indices
            .into_iter()
            .map(|index| self.catalog_candidate(index))
            .collect()
    }
}

struct SoundCatalogCandidate {
    cache_key: String,
    cache_marker: String,
    file_name: String,
    description: String,
    source: Arc<Group>,
    relative_path: PathBuf,
}

impl SoundCatalogCandidate {
    fn load_audio(&self) -> Result<Vec<u8>, clonk_resources::GroupError> {
        self.source.read_file(&self.relative_path)
    }
}

struct SoundEntry {
    relative_path: PathBuf,
    file_name: String,
    extension_rank: usize,
}

pub(crate) struct ResolvedSound<'a> {
    library: &'a SoundLibrary,
    entry_index: usize,
}

impl<'a> ResolvedSound<'a> {
    pub(crate) fn cache_key(&self) -> String {
        self.library.cache_key(self.entry_index)
    }

    fn cache_marker(&self) -> String {
        self.library.cache_marker(self.entry_index)
    }

    pub(crate) fn file_name(&self) -> &str {
        &self.library.entries[self.entry_index].file_name
    }

    fn describe(&self) -> String {
        self.library.describe_entry(self.entry_index)
    }

    pub(crate) fn load_audio(&self) -> Result<Vec<u8>, clonk_resources::GroupError> {
        self.library.read_bytes(self.entry_index)
    }
}

pub(crate) struct SoundSearchTerms {
    pub(crate) wildcard_pattern: Option<String>,
    pub(crate) search_names: Vec<String>,
}

impl SoundSearchTerms {
    pub(crate) fn new(name: &str) -> Self {
        let mut prepared = name.to_string();
        if !sound_name_has_extension(name) {
            prepared.push_str(".wav");
        }
        prepared = prepared.replace('*', "?");
        let has_wildcards = prepared.contains('*') || prepared.contains('?');
        let normalized_lower = prepared.to_ascii_lowercase();

        let wildcard_pattern = if has_wildcards {
            Some(normalized_lower.clone())
        } else {
            None
        };

        let mut search_names = Vec::new();
        if !has_wildcards {
            search_names.push(normalized_lower.clone());
        }

        Self {
            wildcard_pattern,
            search_names,
        }
    }

    fn prepared_pattern(&self) -> String {
        self.wildcard_pattern
            .clone()
            .or_else(|| self.search_names.first().cloned())
            .expect("a prepared sound request always has a name")
    }
}

fn sound_name_has_extension(name: &str) -> bool {
    let component_start = name
        .rfind(std::path::MAIN_SEPARATOR)
        .map_or(0, |index| index + 1);
    name[component_start..]
        .rfind('.')
        .is_some_and(|index| component_start + index + 1 < name.len())
}

fn discover_global_sound_libraries() -> (Vec<SoundLibrary>, Vec<String>) {
    match AppPaths::discover() {
        Ok(paths) => {
            discover_global_sound_libraries_at(paths.content_dir().unwrap_or(paths.install_root()))
        }
        Err(err) => {
            tracing::warn!(error = %err, "sound asset discovery skipped");
            (Vec::new(), Vec::new())
        }
    }
}

pub(crate) fn discover_global_sound_libraries_at(
    exe_data_root: &Path,
) -> (Vec<SoundLibrary>, Vec<String>) {
    // C4SoundSystem's constructor performs exactly one Config.AtExePath
    // lookup. Sound-like siblings, user data, and alternate extensions never
    // enter the native sample bank.
    let sound_path = exe_data_root.join("Sound.c4g");
    let Ok(group) = Group::open(&sound_path) else {
        return (Vec::new(), Vec::new());
    };
    let sample_loads = direct_sound_sample_loads(&group);
    let libraries = collect_direct_sound_library(&group, "Sound.c4g".to_string())
        .into_iter()
        .collect();
    (libraries, sample_loads)
}

pub(crate) fn direct_sound_sample_loads(group: &Group) -> Vec<String> {
    let Ok(entries) = group.entries() else {
        return Vec::new();
    };
    let mut names = Vec::new();
    // C4SoundSystem::LoadEffects performs three complete group scans in this
    // order, preserving each scan's group-entry order.
    for pattern in [
        b"*.wav".as_slice(),
        b"*.ogg".as_slice(),
        b"*.mp3".as_slice(),
    ] {
        names.extend(entries.iter().filter_map(|entry| {
            if entry.is_directory || !classic_wildcard_match(pattern, &entry.name_bytes) {
                return None;
            }
            entry
                .relative_path
                .file_name()
                .map(|name| name.to_string_lossy().to_ascii_lowercase())
        }));
    }
    names
}

#[cfg(test)]
pub(crate) fn collect_sound_libraries_for_path(path: &Path) -> Vec<SoundLibrary> {
    let group = match Group::open(path) {
        Ok(group) => group,
        Err(err) => {
            tracing::warn!(path = %path.display(), error = %err, "failed to open sound group");
            return Vec::new();
        }
    };
    let label = path
        .file_name()
        .and_then(|name| name.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string());
    collect_direct_sound_library(&group, label)
        .into_iter()
        .collect()
}

fn collect_direct_sound_library(group: &Group, label: String) -> Option<SoundLibrary> {
    let source = Arc::new(group.clone());
    let mut library = SoundLibrary::new(label, source);
    let entries = match group.entries() {
        Ok(entries) => entries,
        Err(error) => {
            tracing::warn!(
                path = %group.root().display(),
                %error,
                "failed to inspect direct sound entries"
            );
            return None;
        }
    };
    for pattern in [
        b"*.wav".as_slice(),
        b"*.ogg".as_slice(),
        b"*.mp3".as_slice(),
    ] {
        for entry in &entries {
            if !entry.is_directory && classic_wildcard_match(pattern, &entry.name_bytes) {
                library.add_entry(entry.relative_path.clone());
            }
        }
    }
    (!library.is_empty()).then_some(library)
}

fn sound_catalog_candidates_for_groups(
    groups: &[Group],
    label_prefix: &str,
) -> Vec<SoundCatalogCandidate> {
    groups
        .iter()
        .enumerate()
        .flat_map(|(index, group)| {
            let label = format!("{label_prefix}::{index}::{}", group.root().display());
            collect_direct_sound_library(group, label)
                .map(|library| library.catalog_candidates())
                .unwrap_or_default()
        })
        .collect()
}

fn collect_path_sound_effect_groups(scenario_path: &Path) -> Vec<Group> {
    let mut folder_paths = scenario_path
        .ancestors()
        .skip(1)
        .filter(|path| has_extension(path, "c4f"))
        .map(Path::to_path_buf)
        .collect::<Vec<_>>();
    folder_paths.reverse();

    let mut groups = Vec::new();
    for folder_path in folder_paths {
        let Ok(folder) = Group::open(&folder_path) else {
            continue;
        };
        let has_definition_child = folder.entries().is_ok_and(|entries| {
            entries
                .iter()
                .any(|entry| classic_wildcard_match(b"*.c4d", &entry.name_bytes))
        });
        if has_definition_child {
            collect_definition_tree_sound_groups(&folder, &mut groups);
        }
    }

    if let Ok(scenario) = Group::open(scenario_path) {
        collect_definition_tree_sound_groups(&scenario, &mut groups);
    }
    groups
}

pub(crate) fn collect_definition_tree_sound_groups(group: &Group, groups: &mut Vec<Group>) {
    groups.push(group.clone());
    let Ok(entries) = group.entries() else {
        return;
    };
    for entry in entries {
        if !classic_wildcard_match(b"*.c4d", &entry.name_bytes) {
            continue;
        }
        if let Ok(child) = group.open_child_entry_exact(&entry) {
            collect_definition_tree_sound_groups(&child, groups);
        }
    }
}

pub(crate) fn matches_sound_pattern(pattern: &str, candidate: &str) -> bool {
    clonk_core::std_file::wildcard_match(pattern, candidate)
}

fn extension_rank(ext: Option<&str>) -> usize {
    match ext.map(|value| value.to_ascii_lowercase()) {
        Some(ref ext) if ext == "wav" => 0,
        Some(ref ext) if ext == "ogg" => 1,
        Some(ref ext) if ext == "mp3" => 2,
        _ => 3,
    }
}

#[derive(Clone)]
pub(crate) struct MessageTextSpan {
    text: String,
    color: Color,
}

#[derive(Clone)]
pub(crate) struct MessageWordSegment {
    pub(crate) text: String,
    pub(crate) color: Color,
    pub(crate) width: f32,
}

#[derive(Clone)]
pub(crate) struct MessageLineLayout {
    pub(crate) segments: Vec<MessageWordSegment>,
    pub(crate) width: f32,
}

#[derive(Clone)]
pub(crate) enum MessageWordUnit {
    Segment(MessageWordSegment),
    ForcedBreak,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HorizontalAlignment {
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum VerticalAlignment {
    Top,
    Center,
    Bottom,
    Baseline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct GlobalMessageViewportGeometry {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) width: i32,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct GlobalPortraitPlacement {
    pub(crate) viewport: Rect,
    pub(crate) offset: Vector2,
    pub(crate) flags: u32,
}

fn message_extent_i32(extent: u32) -> i32 {
    i32::try_from(extent).unwrap_or(i32::MAX)
}

fn message_percent(value: i32, extent: i32) -> i32 {
    let scaled = i64::from(value) * i64::from(extent) / 100;
    scaled.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

/// Resolve the global-message fields against the owning viewport facet.
/// `C4Viewport::Execute` supplies `(DrawX, DrawY, ViewWdt, ViewHgt)` as `cgo`
/// (src/C4Viewport.cpp:1146-1149), and `C4GameMessage::Draw` applies relative
/// fields with integer arithmetic before adding `cgo.X/Y`
/// (src/C4GameMessage.cpp:109-111,136-137).
pub(crate) fn global_message_viewport_geometry(
    viewport: Rect,
    offset: Vector2,
    width: i32,
    flags: u32,
) -> GlobalMessageViewportGeometry {
    let viewport_width = message_extent_i32(viewport.width);
    let viewport_height = message_extent_i32(viewport.height);
    let x = if flags & FLAG_X_REL != 0 {
        message_percent(offset.x, viewport_width)
    } else {
        offset.x
    };
    let y = if flags & FLAG_Y_REL != 0 {
        message_percent(offset.y, viewport_height)
    } else {
        offset.y
    };
    let width = if flags & FLAG_WIDTH_REL != 0 {
        message_percent(width, viewport_width)
    } else {
        width
    };
    GlobalMessageViewportGeometry {
        x: viewport.x.saturating_add(x),
        y: viewport.y.saturating_add(y),
        width,
    }
}

/// Position a measured portrait frame from the C4GM positioning reference.
/// Left/Top mean the viewport origin; Right/Bottom and HCenter/VCenter first
/// add the respective viewport reference and then subtract the full/half
/// frame size (src/C4GameMessage.cpp:140-155). The separate integer halves
/// intentionally preserve C++'s odd-size truncation.
pub(crate) fn global_portrait_frame_rect(
    viewport: Rect,
    offset: Vector2,
    flags: u32,
    frame_size: (u32, u32),
) -> Rect {
    let geometry = global_message_viewport_geometry(viewport, offset, 0, flags);
    let viewport_width = message_extent_i32(viewport.width);
    let viewport_height = message_extent_i32(viewport.height);
    let frame_width = message_extent_i32(frame_size.0);
    let frame_height = message_extent_i32(frame_size.1);
    let mut x = geometry.x;
    let mut y = geometry.y;

    if flags & FLAG_RIGHT != 0 {
        x = x.saturating_add(viewport_width).saturating_sub(frame_width);
    } else if flags & FLAG_HCENTER != 0 {
        x = x
            .saturating_add(viewport_width / 2)
            .saturating_sub(frame_width / 2);
    }
    if flags & FLAG_BOTTOM != 0 {
        y = y
            .saturating_add(viewport_height)
            .saturating_sub(frame_height);
    } else if flags & FLAG_VCENTER != 0 {
        y = y
            .saturating_add(viewport_height / 2)
            .saturating_sub(frame_height / 2);
    }

    Rect::new(x, y, frame_size.0, frame_size.1)
}

/// Text alignment is the C4GM_A* family only. Frame positioning flags such as
/// C4GM_Left/Right are independent (src/C4GameMessage.cpp:101,140-168).
pub(crate) fn message_horizontal_alignment(flags: u32, has_frame: bool) -> HorizontalAlignment {
    if flags & FLAG_ALIGN_LEFT != 0 {
        HorizontalAlignment::Left
    } else if flags & FLAG_ALIGN_RIGHT != 0 {
        HorizontalAlignment::Right
    } else if flags & FLAG_ALIGN_CENTER != 0 {
        HorizontalAlignment::Center
    } else if has_frame {
        HorizontalAlignment::Left
    } else {
        HorizontalAlignment::Center
    }
}

pub(crate) fn parse_message_spans(line: &str, base_color: Color) -> Vec<MessageTextSpan> {
    let mut spans = Vec::new();
    let mut current = String::new();
    let mut color_stack = vec![base_color];
    let mut pos = 0usize;
    let line_len = line.len();

    while pos < line_len {
        let rest = &line[pos..];
        if rest.starts_with('<') {
            let mut handled = false;
            if let Some(close) = rest.find('>') {
                let raw_tag = &rest[1..close];
                if !raw_tag.is_empty() {
                    if raw_tag.starts_with('/') {
                        let name = raw_tag[1..].trim().to_ascii_lowercase();
                        if !current.is_empty() {
                            let text = std::mem::take(&mut current);
                            spans.push(MessageTextSpan {
                                text,
                                color: *color_stack.last().unwrap_or(&base_color),
                            });
                        }
                        match name.as_str() {
                            "c" => {
                                if color_stack.len() > 1 {
                                    color_stack.pop();
                                }
                                handled = true;
                            }
                            "i" => {
                                handled = true;
                            }
                            _ => {
                                // treat as literal
                            }
                        }
                    } else {
                        let mut parts = raw_tag.splitn(2, ' ');
                        let name = parts.next().unwrap_or("").trim();
                        let params = parts.next().map(str::trim);
                        let name_lower = name.to_ascii_lowercase();
                        match name_lower.as_str() {
                            "c" => {
                                if let Some(param) = params {
                                    if let Some(color) = parse_markup_color(param) {
                                        if !current.is_empty() {
                                            let text = std::mem::take(&mut current);
                                            spans.push(MessageTextSpan {
                                                text,
                                                color: *color_stack.last().unwrap_or(&base_color),
                                            });
                                        }
                                        color_stack.push(color);
                                        handled = true;
                                    }
                                }
                            }
                            "i" => {
                                if !current.is_empty() {
                                    let text = std::mem::take(&mut current);
                                    spans.push(MessageTextSpan {
                                        text,
                                        color: *color_stack.last().unwrap_or(&base_color),
                                    });
                                }
                                handled = true;
                            }
                            _ => {
                                // unknown tag: treat as literal
                            }
                        }
                    }
                }
                if handled {
                    pos += close + 1;
                    continue;
                }
            }
        }

        if rest.starts_with("{{") && rest.len() > 2 && !rest[2..].starts_with('{') {
            if let Some(end) = rest[2..].find("}}") {
                current.push(' ');
                pos += 2 + end + 2;
                continue;
            }
        }

        if let Some(ch) = rest.chars().next() {
            current.push(ch);
            pos += ch.len_utf8();
        } else {
            break;
        }
    }

    if !current.is_empty() {
        spans.push(MessageTextSpan {
            text: current,
            color: *color_stack.last().unwrap_or(&base_color),
        });
    }

    spans
}

fn parse_markup_color(param: &str) -> Option<Color> {
    let token = param.trim();
    if token.is_empty() || token.len() > 8 || !token.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return None;
    }
    let mut value = u32::from_str_radix(token, 16).ok()?;
    if token.len() <= 6 {
        value |= 0xff00_0000;
    }
    let inverted = (value & 0x00ff_ffff) | ((255 - ((value >> 24) & 0xff)) << 24);
    Some(Color::new(
        ((inverted >> 16) & 0xff) as u8,
        ((inverted >> 8) & 0xff) as u8,
        (inverted & 0xff) as u8,
        ((inverted >> 24) & 0xff) as u8,
    ))
}

pub(crate) fn split_span_into_segments(
    span: MessageTextSpan,
    font: &dyn TextFont,
    font_size: f32,
) -> Vec<MessageWordSegment> {
    if span.text.is_empty() {
        return Vec::new();
    }
    let mut segments = Vec::new();
    let mut current = String::new();
    for ch in span.text.chars() {
        current.push(ch);
        if ch.is_whitespace() {
            let width = font.measure_text(&current, font_size).width;
            segments.push(MessageWordSegment {
                text: std::mem::take(&mut current),
                color: span.color,
                width,
            });
        }
    }
    if !current.is_empty() {
        let width = font.measure_text(&current, font_size).width;
        segments.push(MessageWordSegment {
            text: current,
            color: span.color,
            width,
        });
    }
    segments
}

fn split_segment_to_fit(
    segment: MessageWordSegment,
    max_width: f32,
    font: &dyn TextFont,
    font_size: f32,
) -> Vec<MessageWordSegment> {
    if max_width <= 0.0 || segment.width <= max_width {
        return vec![segment];
    }

    let mut pieces = Vec::new();
    let mut current = String::new();

    for ch in segment.text.chars() {
        current.push(ch);
        let width = font.measure_text(&current, font_size).width;
        if width > max_width && current.len() > ch.len_utf8() {
            current.pop();
            let chunk = std::mem::take(&mut current);
            if !chunk.is_empty() {
                let chunk_width = font.measure_text(&chunk, font_size).width;
                pieces.push(MessageWordSegment {
                    text: chunk,
                    color: segment.color,
                    width: chunk_width,
                });
            }
            current.push(ch);
        }
    }

    if !current.is_empty() {
        let width = font.measure_text(&current, font_size).width;
        pieces.push(MessageWordSegment {
            text: current,
            color: segment.color,
            width,
        });
    }

    if pieces.is_empty() {
        pieces.push(segment);
    }

    pieces
}

pub(crate) fn wrap_word_units(
    units: Vec<MessageWordUnit>,
    max_width: Option<f32>,
    font: &dyn TextFont,
    font_size: f32,
) -> Vec<MessageLineLayout> {
    let mut lines = Vec::new();
    let mut current_segments: Vec<MessageWordSegment> = Vec::new();
    let mut current_width = 0.0f32;

    let push_line = |lines: &mut Vec<MessageLineLayout>,
                     segments: &mut Vec<MessageWordSegment>,
                     width: &mut f32| {
        lines.push(MessageLineLayout {
            width: *width,
            segments: std::mem::take(segments),
        });
        *width = 0.0;
    };

    let last_is_break = matches!(units.last(), Some(MessageWordUnit::ForcedBreak));

    for unit in units.into_iter() {
        match unit {
            MessageWordUnit::ForcedBreak => {
                push_line(&mut lines, &mut current_segments, &mut current_width);
            }
            MessageWordUnit::Segment(segment) => {
                if let Some(limit) = max_width {
                    let limit = if limit < 0.0 { 0.0 } else { limit };
                    let parts = split_segment_to_fit(segment, limit, font, font_size);
                    for piece in parts {
                        let piece_width = piece.width;
                        if limit > 0.0
                            && current_width + piece_width > limit
                            && !current_segments.is_empty()
                        {
                            push_line(&mut lines, &mut current_segments, &mut current_width);
                        }
                        if piece.text.trim().is_empty() && current_segments.is_empty() {
                            continue;
                        }
                        current_width += piece_width;
                        current_segments.push(piece);
                    }
                } else {
                    if segment.text.trim().is_empty() && current_segments.is_empty() {
                        continue;
                    }
                    current_width += segment.width;
                    current_segments.push(segment);
                }
            }
        }
    }

    if !current_segments.is_empty() || last_is_break {
        push_line(&mut lines, &mut current_segments, &mut current_width);
    }

    lines
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ScriptMenuPresentationKey {
    pub(crate) target: ObjectId,
    pub(crate) runtime_id: u64,
    pub(crate) symbol_id: String,
    pub(crate) caption: String,
    pub(crate) selection: i32,
    pub(crate) location: Option<Vector2>,
}

#[derive(Clone, Debug)]
pub(crate) struct ScriptMenuPresentationState {
    pub(crate) key: ScriptMenuPresentationKey,
    pub(crate) time_on_selection: u32,
    /// Initialized dialog origin. `None` retains the menu's native anchored
    /// alignment; free-aligned menus resolve their world anchor once.
    pub(crate) location: Option<(i32, i32)>,
    /// The stored location is still a world-derived C4MN_Align_Free anchor
    /// which must receive InitLocation's one-time viewport clamp.
    pub(crate) location_needs_initialization: bool,
    /// Native C4MN_Align_Free keeps its current bounds as the next reset
    /// candidate; ordinary anchored menus discard a dragged origin on reset.
    pub(crate) free_aligned: bool,
    /// Presentation-only C4GUI::ScrollWindow offset in logical pixels.
    pub(crate) scroll_y: i32,
    /// Selection for which ScrollRangeInView was most recently applied.
    pub(crate) scroll_selection: i32,
    /// C4Menu::AdjustPosition runs after a selection/location update, not on
    /// every draw (otherwise wheel scrolling would immediately be undone).
    pub(crate) selection_needs_adjustment: bool,
}

pub(crate) fn reset_script_menu_presentation_location(state: &mut ScriptMenuPresentationState) {
    if state.free_aligned {
        state.location_needs_initialization = state.location.is_some();
    } else {
        state.location = None;
        state.location_needs_initialization = false;
    }
    state.selection_needs_adjustment = true;
}

/// C4GUI's single retained `pDragElement` for a menu's wooden title label.
#[derive(Clone, Copy, Debug)]
pub(crate) enum MenuTitleDrag {
    Script {
        owner: i32,
        target: ObjectId,
        start_pointer: GuiPoint,
        start_location: (i32, i32),
    },
    Ingame {
        player: i32,
        start_pointer: GuiPoint,
        start_location: (i32, i32),
    },
}

#[derive(Clone)]
pub(crate) enum MessageDialogContinuation {
    None,
    /// `C4AbortGameDialog` owns one offline `Game.HaltCount` increment from
    /// `OnShown` until the first normal close path reaches `OnClosed`.
    AbortGame {
        halted_offline: bool,
    },
    /// Native `C4Console::Message` blocks before an optional second status
    /// dialog (the script-created-object warning precedes a save error).
    DeveloperConsoleNotice {
        follow_up: Option<String>,
    },
    StartupNetworkConnectProgress,
    StartupIrcConnectWarning {
        login: clonk_frontend::startup_netdlg::NetDlgChatLogin,
    },
    StartupIrcDisconnectConfirm,
    NetworkClientStartWait,
    BlockingResourceWait {
        scope: BlockingResourceScope,
        resource_id: i32,
    },
    NetworkRuntimeJoin {
        reference: clonk_network::NetworkGameReference,
    },
    NetworkServerRedirect {
        address: String,
    },
    ClassicLobbyStart {
        countdown_seconds: i32,
    },
    LobbyResourceOverwrite {
        resource_id: i32,
    },
    DeleteStartupPlayer {
        path: PathBuf,
    },
    DeleteStartupCrew {
        player_path: PathBuf,
        file_name: String,
    },
    DeleteScenario {
        path: PathBuf,
        next_identifier: Option<String>,
    },
    NetworkScenarioPlayerCountWarning {
        scenario: FrontendScenario,
    },
    LobbyReadyCheck {
        remaining_seconds: u32,
    },
    LiveMasterserverSignup,
    LeaguePlayerAuthWait,
    LeaguePlayerAuthWelcome,
    LeaguePlayerAuthError,
    LeaguePlayerAuthCancelled,
    LeagueEndRetry,
    LeagueEndRejected,
    LeagueSignupCancelled,
    LeagueVote {
        subject: LeagueVoteSubject,
    },
    LeagueSurrender,
    OptionsScaleTest {
        old_percent: i32,
        new_percent: i32,
        remaining_seconds: u32,
    },
    OptionsControlCapture(clonk_frontend::startup_options_controls::ControlCaptureTarget),
    OptionsAlternateServerNotice,
    OptionsResetConfiguration,
    OptionsAdvancedWarning,
}

pub(crate) const LEAGUE_END_MAX_ATTEMPTS: u8 = 10;

#[derive(Clone)]
pub(crate) struct PendingLeagueEnd {
    pub(crate) reference: clonk_network::HostGameReference,
    pub(crate) record: Option<clonk_network::LeagueEndRecord>,
    pub(crate) attempts: u8,
    pub(crate) last_failure: Option<String>,
    pub(crate) terminal_packet: Option<clonk_network::LeagueRoundResultsPacket>,
}

pub(crate) enum LeaguePlayerAuthContinuation {
    InitialClient {
        request: clonk_network::PlayerInfoUpdateRequest,
        index: usize,
        server_name: String,
    },
    StartupHost {
        mode: NetworkMode,
        manager: NetworkManager,
        selected_scenario: Option<(String, String)>,
        purpose: StartupNetworkPurpose,
        players: Vec<clonk_engine::ControlPlayerInfoEntry>,
        index: usize,
        server_name: String,
    },
    RuntimePlayer {
        request: clonk_network::PlayerInfoUpdateRequest,
        index: usize,
        server_name: String,
        host: bool,
        alternate_resource_id: i32,
        alternate_color: u32,
    },
}

pub(crate) enum PendingLeaguePlayerAuthStage {
    Waiting(network::PendingLeaguePlayerAuth),
    Decision,
}

pub(crate) struct PendingLeaguePlayerAuth {
    pub(crate) continuation: LeaguePlayerAuthContinuation,
    pub(crate) stage: PendingLeaguePlayerAuthStage,
    pub(crate) auth: clonk_network::LeagueAuthRequestHead,
    pub(crate) mode: clonk_frontend::league_signup::LeagueSignupMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LeaguePlayerAuthStatus {
    Pending,
    Completed(bool),
}

#[derive(Clone)]
pub(crate) struct PendingMessageDialog {
    pub(crate) running_stack_id: u64,
    pub(crate) state: clonk_frontend::message_dialog::MessageDialogState,
    pub(crate) continuation: MessageDialogContinuation,
}

#[derive(Clone, Debug)]
pub(crate) struct PendingDefinitionSelection {
    pub(crate) scenario: FrontendScenario,
    pub(crate) selector_mode: ScenarioSelectorMode,
    /// `Config.AtExePath(DefinitionPath)`, also used by the selector when the
    /// configured directory does not exist.
    pub(crate) root: PathBuf,
    /// C4Game applies DefinitionPath only when the configured directory
    /// exists. Keep that decision separate from the selector's display root.
    pub(crate) custom_definition_root: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub(crate) struct LobbyPlayerCandidate {
    pub(crate) source_path: PathBuf,
    pub(crate) wire_filename: String,
}

#[derive(Clone, Debug)]
pub(crate) struct PendingLobbyPlayerSelection {
    pub(crate) client_id: i32,
    /// C4PlayerSelDlg snapshots `Config.General.PlayerPath` at construction;
    /// F5 enumerates the same roots even if the config changes underneath it.
    pub(crate) config: Config,
    /// Physical selector result -> C++ `Config.AtExeRelativePath` wire name.
    pub(crate) candidates: BTreeMap<String, LobbyPlayerCandidate>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ScenarioSelectorMode {
    #[default]
    Local,
    NetworkHost,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum NetworkScenarioOpenDecision {
    Proceed,
    Error { message: String, caption: String },
    Warning { message: String, caption: String },
}

impl ScenarioSelectorMode {
    pub(crate) const fn game_option_context(self) -> GameOptionContext {
        match self {
            Self::Local => GameOptionContext::LocalSelector,
            Self::NetworkHost => GameOptionContext::NetworkHostSelector,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct PendingGameOptionInputDialog {
    pub(crate) purpose: PendingInputDialogPurpose,
    pub(crate) controller: InputDialogController,
}

pub(crate) struct PendingLeagueSignupDialog {
    pub(crate) controller: clonk_frontend::league_signup::LeagueSignupController,
    /// Existing credentials retained across the native login/register pair.
    /// Registration fills only NewAccount/NewPassword and still sends these
    /// old Account/Password fields.
    pub(crate) auth: clonk_network::LeagueAuthRequestHead,
    pub(crate) continuation: LeaguePlayerAuthContinuation,
}

pub(crate) struct PendingOptionsAdvancedDialog {
    pub(crate) controller: clonk_frontend::startup_options_advanced::AdvancedConfigController,
    pub(crate) return_sheet: clonk_frontend::startup_options_dlg::OptionsSheet,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PendingCrewInputAction {
    SetDeathMessage { index: usize },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PendingInputDialogPurpose {
    GameOption(GameOptionInputKind),
    RunningChat,
    NetworkJoinPassword,
    OptionsGraphicsScale,
    OptionsNetwork(clonk_frontend::startup_options_network::NetworkTextField),
    ScenarioMissionAccess,
    StartupCrew(PendingCrewInputAction),
}

pub(crate) struct StagedNetworkHostScenario {
    pub(crate) frontend: FrontendScenario,
    pub(crate) definition_load: ScenarioDefinitionLoad,
    pub(crate) effective_definition_modules: Vec<String>,
    pub(crate) definition_resources: Vec<clonk_network::HostInitialResourceSource>,
    pub(crate) definition_executable_path: String,
    pub(crate) definition_path: String,
    pub(crate) scenario: Scenario,
    /// The exact scenario loader selected before the host socket is opened.
    /// It is moved into `GameApp::loader_screen` while the worker starts and
    /// remains the transparent lobby's backdrop.
    pub(crate) loader_screen: Option<LoaderScreen>,
    pub(crate) loader_initial_tooltip_font: Arc<clonk_graphics::clonk_font::ClonkFont>,
    pub(crate) loader_initial_native_font_source: Option<ClassicNativeFontSource>,
    pub(crate) loader_refreshed_resources: LoaderResources,
    pub(crate) loader_refreshed_tooltip_font: Option<Arc<clonk_graphics::clonk_font::ClonkFont>>,
    pub(crate) loader_refreshed_native_font_source: Option<ClassicNativeFontSource>,
    /// Retained until the unported lobby Start handoff reaches the real
    /// GraphicsResource refresh. The visible lobby still uses startup GUI.
    pub(crate) pending_global_gui_failures: HashMap<&'static str, String>,
    /// Decoded scenario GUI sheet overrides applied at the same refresh.
    pub(crate) pending_gui_sheet_overrides: Vec<ClassicGuiSheetOverride>,
    /// Post-definition `GraphicsResource.Player`, used when a player file has
    /// no valid `BigIcon.png`.
    pub(crate) default_player_icon: ImageData,
    /// Post-definition `GraphicsResource.Crew`, used for savegame-association
    /// overlays on player icons.
    pub(crate) default_crew_icon: ImageData,
    /// Exact selector values accepted for this host round.
    pub(crate) options: GameOptionValues,
    /// Immutable C++-validated values needed by the bounded initial lobby.
    pub(crate) lobby: ClassicHostLobbyProjection,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ClassicHostLobbyProjection {
    pub(crate) local_name: String,
    pub(crate) nick: String,
    pub(crate) countdown_seconds: i32,
    pub(crate) max_players: i32,
    pub(crate) has_teams: bool,
    pub(crate) fair_crew: bool,
    pub(crate) fair_crew_forced: bool,
    pub(crate) fair_crew_strength: i32,
}

/// C4GameParameters' pre-game fair-crew resolution shared by local startup
/// and host staging: an embedded/forced value owns the flag; otherwise the
/// current user option does. A scenario strength of zero falls back to the
/// configured strength only for an unembedded enabled round.
pub(crate) fn resolve_scenario_fair_crew_parameters(
    metadata: &ScenarioLobbyMetadata,
    options: &GameOptionValues,
) -> (bool, i32) {
    let embedded = metadata.embedded_game_parameter_values();
    let parameters = embedded
        .as_ref()
        .unwrap_or_else(|| metadata.game_parameter_defaults());
    let use_fair_crew = if embedded.is_some() || parameters.fair_crew_forced() {
        parameters.use_fair_crew()
    } else {
        options.fair_crew
    };
    let mut fair_crew_strength = parameters.fair_crew_strength();
    if embedded.is_none() && use_fair_crew && fair_crew_strength == 0 {
        fair_crew_strength = options.fair_crew_strength;
    }
    (use_fair_crew, fair_crew_strength)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LobbyScenarioDescriptionUpdate {
    Loading(String),
    Complete(LobbyScenarioText),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct LobbyScenarioDescriptionState {
    pub(crate) text: LobbyScenarioText,
    pub(crate) finished: bool,
}

impl LobbyScenarioDescriptionState {
    pub(crate) fn apply(&mut self, update: Option<LobbyScenarioDescriptionUpdate>) -> bool {
        if self.finished {
            return false;
        }
        let Some(update) = update else {
            return false;
        };
        let (text, finished) = match update {
            LobbyScenarioDescriptionUpdate::Loading(text) => {
                (LobbyScenarioText::Message(text), false)
            }
            LobbyScenarioDescriptionUpdate::Complete(text) => (text, true),
        };
        let changed = self.text != text || self.finished != finished;
        self.text = text;
        self.finished = finished;
        changed
    }
}

pub(crate) struct ClassicHostLobbyState {
    pub(crate) controller: ClassicGameLobby,
    pub(crate) preload: LobbyPreloadState,
    pub(crate) pointer: Option<GuiPoint>,
    pub(crate) last_roster_click: Option<(LobbyRosterId, Instant)>,
    pub(crate) chat_history_index: i32,
    /// Retained Config.Network.NoRuntimeJoin inverse. Prepared hosts mirror
    /// this into admission so lobby exit can apply the selection atomically
    /// with its Go status request.
    pub(crate) runtime_join_allowed: bool,
    /// C4Network2ResDlg keeps receiving network updates while inactive, but
    /// does not reconcile its visible rows until the sheet is activated.
    pub(crate) resource_rows: BTreeMap<i32, LobbyResourceRow>,
    pub(crate) scenario_description: LobbyScenarioDescriptionState,
}

/// Process-local projection of `Game.CanPreload()` plus the construction-time
/// General.Preloading mode. Eligibility edges are shared by automatic and
/// manual paths; a failed launch stays retryable, while success is one-shot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LobbyPreloadState {
    pub(crate) automatic: bool,
    pub(crate) manual_button_present: bool,
    pub(crate) eligible: bool,
    pub(crate) spent: bool,
}

impl LobbyPreloadState {
    pub(crate) const fn new(automatic: bool) -> Self {
        Self {
            automatic,
            manual_button_present: !automatic,
            eligible: false,
            spent: false,
        }
    }

    pub(crate) fn synchronize(&mut self, resources_complete: bool, context_ready: bool) -> bool {
        let was_eligible = self.eligible;
        self.eligible = resources_complete && context_ready && !self.spent;
        self.automatic && !was_eligible && self.eligible
    }

    pub(crate) fn record_result(&mut self, succeeded: bool) {
        if succeeded {
            self.spent = true;
            self.eligible = false;
            self.manual_button_present = false;
        }
    }

    pub(crate) fn reset_for_context(&mut self) {
        *self = Self::new(self.automatic);
    }
}

pub(crate) const RESTART_RESTORE_PLAYER_TEAMS: i32 = 0x2;

/// Process-runtime `C4NetworkRestartInfos::Player` snapshot retained while a
/// round is restarted. Script-player restoration consumes the type/color too,
/// so keep the complete native payload even though PlayerListItem only reads
/// the recorded team.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RestartRestorePlayerInfo {
    pub(crate) player_type: u8,
    pub(crate) team: i32,
    pub(crate) color: u32,
}

/// `Game.RestartRestoreInfos` survives `C4Game::Clear` for Restart, but is
/// deliberately outside savegame/engine serialization.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RestartRestoreInfos {
    pub(crate) what: i32,
    pub(crate) players: BTreeMap<Vec<u8>, RestartRestorePlayerInfo>,
}

impl RestartRestoreInfos {
    pub(crate) fn capture_player_infos(&mut self, player_infos: &ControlPlayerInfoRegistry) {
        *self = Self::default();
        let (_, packets) = player_infos.retained_rows_snapshot();
        for (_, _, players) in packets {
            for player in players {
                if player.flags
                    & (clonk_engine::PLAYER_INFO_FLAG_REMOVED
                        | clonk_engine::PLAYER_INFO_FLAG_INVISIBLE)
                    != 0
                {
                    continue;
                }
                // std::map::emplace retains the first exact duplicate name.
                self.players
                    .entry(restart_restore_player_name(&player))
                    .or_insert(RestartRestorePlayerInfo {
                        player_type: player.player_type,
                        team: player.team,
                        color: player.color,
                    });
            }
        }
    }
}

fn restart_restore_player_name(player: &clonk_engine::ControlPlayerInfoEntry) -> Vec<u8> {
    if !player.league_account.as_bytes().is_empty() {
        player.league_account.as_bytes().to_vec()
    } else if !player.forced_name.as_bytes().is_empty() {
        player.forced_name.as_bytes().to_vec()
    } else {
        player.name.as_bytes().to_vec()
    }
}

pub(crate) fn restart_restore_lobby_name(player: &clonk_engine::ControlPlayerInfoEntry) -> Vec<u8> {
    if !player.league_account.as_bytes().is_empty() {
        if player.clan_tag.as_bytes().is_empty() {
            return player.league_account.as_bytes().to_vec();
        }
        let mut name = b"<c afafaf>".to_vec();
        name.extend_from_slice(player.clan_tag.as_bytes());
        name.extend_from_slice(b"</c> ");
        name.extend_from_slice(player.league_account.as_bytes());
        return name;
    }
    if !player.forced_name.as_bytes().is_empty() {
        player.forced_name.as_bytes().to_vec()
    } else {
        player.name.as_bytes().to_vec()
    }
}

pub(crate) struct NetworkStartWaitDialogState {
    pub(crate) controller: clonk_frontend::network_start_wait::NetworkStartWaitState,
    pub(crate) expected_status: clonk_network::NetworkStatus,
    pub(crate) visible: bool,
    pub(crate) pointer: Option<GuiPoint>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StartupNetworkPurpose {
    Join,
    StagedHost,
}

pub(crate) const STARTUP_RESTART_LOG_CAPACITY: usize = 100;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum StartupRestartPresentation {
    Fatal(String),
    Ringbuffer(Vec<String>),
    Empty,
}

/// Process-owned startup diagnostics captured before a failed game is
/// cleared. Fatal errors are deduplicated, while the ordinary log retains the
/// newest 100 entries in oldest-to-newest presentation order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct StartupRestartDiagnostics {
    quit_with_error: bool,
    fatal_errors: Vec<String>,
    ringbuffer_entries: VecDeque<String>,
}

impl StartupRestartDiagnostics {
    pub(crate) fn begin_game_init(&mut self) {
        self.quit_with_error = false;
        self.ringbuffer_entries.clear();
    }

    pub(crate) fn mark_quit_with_error(&mut self) {
        self.quit_with_error = true;
    }

    pub(crate) fn add_fatal_error(&mut self, message: impl Into<String>) {
        let message = message.into();
        if !message.is_empty() && !self.fatal_errors.contains(&message) {
            self.fatal_errors.push(message);
        }
    }

    pub(crate) fn add_log_entry(&mut self, message: impl Into<String>) {
        if self.ringbuffer_entries.len() == STARTUP_RESTART_LOG_CAPACITY {
            self.ringbuffer_entries.pop_front();
        }
        self.ringbuffer_entries.push_back(message.into());
    }

    pub(crate) fn take_presentation(&mut self) -> Option<StartupRestartPresentation> {
        if !self.quit_with_error && self.fatal_errors.is_empty() {
            return None;
        }
        self.quit_with_error = false;
        let fatal_errors = std::mem::take(&mut self.fatal_errors);
        if !fatal_errors.is_empty() {
            self.ringbuffer_entries.clear();
            return Some(StartupRestartPresentation::Fatal(fatal_errors.join("|")));
        }
        let entries = self.ringbuffer_entries.drain(..).collect::<Vec<_>>();
        if entries.is_empty() {
            Some(StartupRestartPresentation::Empty)
        } else {
            Some(StartupRestartPresentation::Ringbuffer(entries))
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum StartupDirectReferenceQueryState {
    Pending,
    Empty,
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StartupDirectReferenceQuery {
    pub(crate) id: u64,
    pub(crate) address: String,
    pub(crate) state: StartupDirectReferenceQueryState,
    /// `TT_RefReqWait` removes completed empty/error direct-query rows after
    /// `C4NetErrorRefTimeout`; an in-flight request retains no removal timer.
    pub(crate) expires_at: Option<Instant>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StartupDiscoveryReferenceQuery {
    pub(crate) id: u64,
    pub(crate) address: SocketAddr,
    pub(crate) state: StartupDirectReferenceQueryState,
    /// Game-discovery queries use the same completed-row timeout as manually
    /// entered direct queries.
    pub(crate) expires_at: Option<Instant>,
}

pub(crate) enum StartupNetworkJoinTarget {
    Reference(clonk_network::NetworkGameReference),
    DirectAddress(String),
    QueryError(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ScenarioDefinitionLoad {
    /// C4StartupScenSelDlg's unchecked branch seeds Objects.c4d; a non-local
    /// scenario preset may replace it during C4Game initialization.
    Seed {
        modules: Vec<String>,
        definition_root: Option<PathBuf>,
    },
    /// C4DefinitionSelDlg acceptance sets Game.FixedDefinitions and keeps the
    /// exact ordered vector returned by the selector.
    Fixed {
        modules: Vec<String>,
        definition_root: Option<PathBuf>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ScenselSearchContextCommand {
    Cut,
    Copy,
    Paste,
    Clear,
    SelectAll,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AppContextMenuCommand {
    StartupPlayer(PlrSelPlayerContextCommand),
    StartupCrew(PlrSelCrewContextCommand),
    AddStartupParticipant(String),
    RemoveStartupParticipant(usize),
    OptionsLanguage(String),
    OptionsFontFace(String),
    OptionsFontSize(i32),
    OptionsDisplayMode(clonk_frontend::startup_options_graphics::GraphicsDisplayMode),
    LobbyTeam {
        player_id: i32,
        team_id: i32,
    },
    LobbyControlRate(i32),
    LobbyRuntimeJoin(bool),
    RuntimeClientOption {
        option: LobbyOptionKind,
        value: i32,
    },
    LobbyTeamDistribution(i32),
    LobbyTeamColors(bool),
    LobbyRandomTeamCount(i32),
    LobbyPlayerTakeOver {
        savegame_player_id: i32,
        player_id: i32,
    },
    /// Deferred Take Over submenu request: children are computed when the
    /// submenu opens, mirroring `PlayerListItem::OnContextTakeOver`
    /// (src/C4PlayerInfoListBox.cpp:535-556) running at submenu-open time.
    LobbyPlayerTakeOverSubmenu {
        savegame_player_id: i32,
    },
    LobbyPlayerRemove {
        client_id: i32,
        player_id: i32,
    },
    LobbyPlayerNewColor {
        client_id: i32,
        player_id: i32,
    },
    LobbyClientToggleMute(i32),
    LobbyClientToggleActivate(i32),
    LobbyClientInfo(i32),
    LobbyKick(i32),
    LobbySheet(LobbySheet),
    NetworkJoinEdit(clonk_frontend::startup_netdlg::NetDlgEditContextCommand),
    LeagueSignupEdit {
        field: clonk_frontend::league_signup::LeagueSignupField,
        command: clonk_frontend::league_signup::LeagueSignupEditContextCommand,
    },
    LobbyChat(LobbyChatContextCommand),
    ScenarioSearch(ScenselSearchContextCommand),
    StartupCrewRename(clonk_frontend::startup_netdlg::NetDlgEditContextCommand),
    InputDialog(InputDialogContextCommand),
}

pub(crate) fn same_script_menu_presentation(
    state: &ScriptMenuPresentationState,
    target: ObjectId,
    menu: &clonk_engine::ObjectMenuState,
) -> bool {
    state.key.target == target
        && state.key.runtime_id == menu.runtime_id
        && state.key.symbol_id == menu.symbol_id
        && state.key.caption == menu.caption
        && state.key.location == menu.location
}

pub(crate) fn initial_control_clients(
    network: Option<&NetworkManager>,
    network_mode: Option<&NetworkMode>,
) -> ControlClientRegistry {
    let mut clients = ControlClientRegistry::default();
    if let Some(NetworkMode::Host(HostSettings {
        prepared: Some(prepared),
        ..
    })) = network_mode
    {
        clients.replace_snapshot([prepared.host_config().local_core.clone()]);
        return clients;
    }
    let client_id = network
        .and_then(|network| i32::try_from(network.local_client_id()).ok())
        .unwrap_or(0);
    let activated = network.is_none() || matches!(network_mode, Some(NetworkMode::Host(_)));
    let string_name = |name: &str| {
        let bytes = name
            .as_bytes()
            .iter()
            .copied()
            .take_while(|byte| *byte != 0)
            .collect();
        clonk_engine::LegacyCString::from_bytes(bytes).unwrap_or_default()
    };
    let name = match network_mode {
        Some(NetworkMode::Host(settings)) => string_name(&settings.player_name),
        Some(NetworkMode::Client(settings)) => string_name(&settings.player_name),
        None => string_name("Local"),
    };
    clients.replace_snapshot([clonk_engine::ClientCoreControlData {
        client_id,
        activated,
        observer: false,
        name,
        nick: clonk_engine::LegacyCString::default(),
        lobby_ready: false,
    }]);
    clients
}

pub(crate) fn readable_lobby_rgba(color: u32) -> [u8; 4] {
    // Lobby client/player names are GUI labels with readable-on-black enabled
    // (src/C4PlayerInfoListBox.cpp:72-87, 143, 648-685, 737-750, 824-825).
    clonk_frontend::game_lobby::make_color_readable_on_black(color)
}

/// Reduce the selected message/data route snapshot to the value displayed by
/// `C4PlayerInfoListBox::ClientListItem::UpdatePing`.
///
/// Native prefers a positive message-connection lag. It consults the data
/// connection only when the message connection is absent or reports `<= 0`;
/// this is deliberately not a minimum of both routes. `-1` removes the ping
/// label, while zero remains a visible `0 ms` value. Each route reports
/// `getLag()` (`lag_ms`), not the cached round trip: an unanswered ping shows
/// its growing wait once it exceeds the last measurement
/// (src/C4PlayerInfoListBox.cpp:894-905; src/C4Network2IO.cpp:1283-1295).
pub(crate) fn classic_lobby_client_ping_ms_by_id(
    connections: &[clonk_network::RuntimeNetworkConnection],
    local_client_id: ClientId,
) -> BTreeMap<ClientId, i32> {
    #[derive(Default)]
    struct ClientConnections {
        message: Option<i32>,
        data: Option<i32>,
    }

    let mut by_client = BTreeMap::<ClientId, ClientConnections>::new();
    for connection in connections {
        if connection.client_id == local_client_id {
            continue;
        }
        let client = by_client.entry(connection.client_id).or_default();
        match connection.usage.as_str() {
            "Data/Msg" => {
                client.message = Some(connection.lag_ms);
                client.data = Some(connection.lag_ms);
            }
            "Msg" => client.message = Some(connection.lag_ms),
            "Data" => client.data = Some(connection.lag_ms),
            _ => {}
        }
    }

    by_client
        .into_iter()
        .filter_map(|(client_id, connections)| {
            let mut ping_ms = connections.message.unwrap_or(-1);
            if ping_ms <= 0 {
                if let Some(data_ping_ms) = connections.data {
                    ping_ms = data_ping_ms;
                }
            }
            (ping_ms != -1).then_some((client_id, ping_ms))
        })
        .collect()
}

pub(crate) fn apply_classic_lobby_client_telemetry(
    rows: &mut [LobbyRosterRow],
    local_client_id: ClientId,
    telemetry: &clonk_network::RuntimeLobbyClientTelemetry,
) -> bool {
    let connected_clients = telemetry
        .connections
        .iter()
        .map(|connection| connection.client_id)
        .collect::<HashSet<_>>();
    let ping_by_client =
        classic_lobby_client_ping_ms_by_id(&telemetry.connections, local_client_id);
    let progress_by_client = telemetry
        .resource_progress
        .iter()
        .copied()
        .collect::<BTreeMap<_, _>>();
    let mut changed = false;

    for row in rows {
        let LobbyRosterRow::Client(client) = row else {
            continue;
        };
        let Ok(client_id) = ClientId::try_from(client.id) else {
            continue;
        };
        let local = client_id == local_client_id;
        let connected = !local && connected_clients.contains(&client_id);
        let ping_ms = connected
            .then(|| ping_by_client.get(&client_id).copied())
            .flatten();
        let resource_progress = connected
            .then(|| progress_by_client.get(&client_id).copied())
            .flatten();
        if client.connected != connected
            || client.ping_ms != ping_ms
            || client.resource_progress != resource_progress
        {
            client.connected = connected;
            client.ping_ms = ping_ms;
            client.resource_progress = resource_progress;
            changed = true;
        }
    }
    changed
}

pub(crate) fn classic_lobby_roster_projection(
    clients: &ControlClientRegistry,
    player_infos: &ControlPlayerInfoRegistry,
    teams: Option<&clonk_engine::InitialNetworkTeamMetadata>,
    local_client_id: i32,
    sheet: LobbySheet,
) -> (Vec<LobbyRosterRow>, i32) {
    let (_, packets) = player_infos.retained_rows_snapshot();
    let players_by_client = packets
        .into_iter()
        .map(|(client_id, _, players)| (client_id, players))
        .collect::<BTreeMap<_, _>>();
    let client_snapshot = clients.snapshot();
    let team_value = |client_id: i32, player: &clonk_engine::ControlPlayerInfoEntry| {
        let metadata = teams?;
        if !metadata.active {
            return None;
        }
        let current = metadata.teams.iter().find(|team| team.id == player.team);
        let hidden_random_team = matches!(
            metadata.team_distribution,
            clonk_engine::InitialNetworkTeamDistribution::RandomInvisible
        );
        let name = if hidden_random_team {
            "Random team".to_string()
        } else {
            current
                .map(|team| legacy_presentation_text(team.name.as_bytes()))
                .unwrap_or_default()
        };
        let local_is_host = local_client_id == 0;
        let distribution_selectable = match metadata.team_distribution {
            clonk_engine::InitialNetworkTeamDistribution::Free => {
                local_is_host || client_id == local_client_id
            }
            clonk_engine::InitialNetworkTeamDistribution::Host => local_is_host,
            clonk_engine::InitialNetworkTeamDistribution::None
            | clonk_engine::InitialNetworkTeamDistribution::Random
            | clonk_engine::InitialNetworkTeamDistribution::RandomInvisible => false,
        };
        let another_available = metadata.auto_generate_teams
            || metadata.teams.iter().any(|team| {
                team.id != player.team
                    && (team.max_players == 0
                        || i32::try_from(team.player_ids.len()).unwrap_or(i32::MAX)
                            < team.max_players)
            });
        Some(LobbyTeamValue {
            id: player.team,
            name,
            selectable: distribution_selectable
                && another_available
                && !player.is_joined()
                && player.savegame_player == 0,
        })
    };
    let visible_player = |player: &&clonk_engine::ControlPlayerInfoEntry| {
        player.flags
            & (clonk_engine::PLAYER_INFO_FLAG_REMOVED | clonk_engine::PLAYER_INFO_FLAG_INVISIBLE)
            == 0
    };
    let player_row = |client_id, player: &clonk_engine::ControlPlayerInfoEntry| {
        let name = if !player.forced_name.is_empty() {
            legacy_presentation_text(player.forced_name.as_bytes())
        } else {
            legacy_presentation_text(player.name.as_bytes())
        };
        let hide_random_color = teams.is_some_and(|metadata| {
            metadata.active
                && metadata.team_colors
                && matches!(
                    metadata.team_distribution,
                    clonk_engine::InitialNetworkTeamDistribution::RandomInvisible
                )
                && metadata.teams.iter().any(|team| team.id == player.team)
                && !player.is_joined()
                && player.savegame_player == 0
        });
        LobbyRosterRow::Player(LobbyPlayerRow {
            id: player.id,
            client_id,
            name,
            color: readable_lobby_rgba(if hide_random_color {
                player.original_color
            } else {
                player.color
            }),
            icon: LobbyRosterIcon::Standard(9),
            joined_player_overlay: None,
            team: team_value(client_id, player),
            league_score: (player.league_score != 0).then(|| player.league_score.to_string()),
            league_rank: u8::try_from(player.league_rank_symbol)
                .ok()
                .filter(|rank| (1..=9).contains(rank)),
        })
    };

    let script_players = players_by_client
        .iter()
        .flat_map(|(&client_id, players)| {
            players
                .iter()
                .filter(visible_player)
                .filter(|player| player.is_script_player())
                .map(move |player| player_row(client_id, player))
        })
        .collect::<Vec<_>>();
    let mut active_players = i32::try_from(script_players.len()).unwrap_or(i32::MAX);
    let mut rows = Vec::new();
    if !script_players.is_empty() || teams.is_some_and(|teams| teams.max_script_players > 0) {
        let can_add_player = teams.is_some_and(|teams| {
            let current = i32::try_from(script_players.len()).unwrap_or(i32::MAX);
            local_client_id == 0 && current < teams.max_script_players
        });
        rows.push(LobbyRosterRow::Header(LobbyHeaderRow {
            kind: LobbyRosterHeader::ScriptPlayers,
            label: "Script players".to_string(),
            icon: LobbyRosterIcon::Standard(21),
            can_add_player,
        }));
        rows.extend(script_players);
    }
    for core in &client_snapshot {
        let players = players_by_client.get(&core.client_id);
        let hide_random_colors = teams.is_some_and(|metadata| {
            metadata.active
                && metadata.team_colors
                && matches!(
                    metadata.team_distribution,
                    clonk_engine::InitialNetworkTeamDistribution::RandomInvisible
                )
        });
        let first_user_color = players
            .into_iter()
            .flat_map(|players| players.iter())
            .filter(visible_player)
            .find(|player| player.player_type == clonk_engine::PLAYER_INFO_TYPE_USER)
            .map(|player| {
                if hide_random_colors
                    && !player.is_joined()
                    && player.savegame_player == 0
                    && teams.is_some_and(|metadata| {
                        metadata.teams.iter().any(|team| team.id == player.team)
                    })
                {
                    player.original_color
                } else {
                    player.color
                }
            })
            .unwrap_or(0x00ff_ffff);
        rows.push(LobbyRosterRow::Client(LobbyClientRow {
            id: core.client_id,
            name: legacy_presentation_text(core.name.as_bytes()),
            nick: legacy_presentation_text(core.nick.as_bytes()),
            color: readable_lobby_rgba(first_user_color),
            status: if players.is_none() {
                LobbyClientStatus::Unknown
            } else if core.client_id == 0 {
                LobbyClientStatus::Host
            } else if core.activated {
                if core.lobby_ready {
                    LobbyClientStatus::Ready
                } else {
                    LobbyClientStatus::Client
                }
            } else {
                LobbyClientStatus::Observer
            },
            local: core.client_id == local_client_id,
            connected: core.client_id != local_client_id,
            resource_progress: None,
            ping_ms: None,
        }));
        if !core.observer {
            for player in players
                .into_iter()
                .flat_map(|players| players.iter())
                .filter(visible_player)
                .filter(|player| !player.is_script_player())
            {
                active_players = active_players.saturating_add(1);
                rows.push(player_row(core.client_id, player));
            }
        }
    }
    if sheet == LobbySheet::Teams {
        let Some(metadata) = teams.filter(|metadata| metadata.active) else {
            return (rows, active_players);
        };
        let mut team_rows = Vec::new();
        if matches!(
            metadata.team_distribution,
            clonk_engine::InitialNetworkTeamDistribution::RandomInvisible
        ) {
            let mut header_emitted = false;
            for core in &client_snapshot {
                if !core.activated {
                    continue;
                }
                let Some(players) = players_by_client.get(&core.client_id) else {
                    continue;
                };
                for player in players {
                    if player.flags & clonk_engine::PLAYER_INFO_FLAG_INVISIBLE != 0 {
                        continue;
                    }
                    if !header_emitted {
                        team_rows.push(LobbyRosterRow::Header(LobbyHeaderRow {
                            kind: LobbyRosterHeader::RandomTeam,
                            label: "Random team".to_string(),
                            icon: LobbyRosterIcon::Standard(19),
                            can_add_player: false,
                        }));
                        header_emitted = true;
                    }
                    team_rows.push(player_row(core.client_id, player));
                }
            }
        } else {
            let players_by_id = players_by_client
                .iter()
                .flat_map(|(&client_id, players)| {
                    players
                        .iter()
                        .map(move |player| (player.id, (client_id, player)))
                })
                .collect::<HashMap<_, _>>();
            for team in &metadata.teams {
                if metadata.auto_generate_teams && team.player_ids.is_empty() {
                    continue;
                }
                team_rows.push(LobbyRosterRow::Header(LobbyHeaderRow {
                    kind: LobbyRosterHeader::Team(team.id),
                    label: legacy_presentation_text(team.name.as_bytes()),
                    icon: LobbyRosterIcon::Standard(19),
                    can_add_player: false,
                }));
                for player_id in &team.player_ids {
                    let Some((client_id, player)) = players_by_id.get(player_id).copied() else {
                        continue;
                    };
                    if player.flags & clonk_engine::PLAYER_INFO_FLAG_INVISIBLE != 0
                        || !client_snapshot
                            .iter()
                            .any(|client| client.client_id == client_id && client.activated)
                    {
                        continue;
                    }
                    team_rows.push(player_row(client_id, player));
                }
            }
        }
        return (team_rows, active_players);
    }
    (rows, active_players)
}

pub(crate) fn discover_lobby_player_selector(
    paths: &AppPaths,
    config: &Config,
) -> io::Result<(
    PathBuf,
    Vec<clonk_frontend::definition_sel::DefinitionSelEntry>,
    BTreeMap<String, LobbyPlayerCandidate>,
)> {
    let configured_player_path = config
        .get_in(Some("General"), "PlayerPath")
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .unwrap_or_default();
    let root = if configured_player_path.is_absolute() {
        configured_player_path
    } else {
        paths.install_root().join(configured_player_path)
    };
    let players = discover_player_files_in(paths.install_root(), config)?;
    let mut candidates = BTreeMap::new();
    let entries = players
        .into_iter()
        .filter_map(|player| {
            let full_path = player.path.to_string_lossy().into_owned();
            // A non-UTF-8 collision cannot be represented by the selector's
            // string identity. Keep the first exact physical path rather
            // than silently replacing it with a different file.
            if candidates.contains_key(&full_path) {
                return None;
            }
            let label = Path::new(&player.file_name)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| player.file_name.clone());
            candidates.insert(
                full_path.clone(),
                LobbyPlayerCandidate {
                    source_path: player.path,
                    wire_filename: player.file_name,
                },
            );
            Some(clonk_frontend::definition_sel::DefinitionSelEntry::new(
                full_path, label,
            ))
        })
        .collect();
    Ok((root, entries, candidates))
}

fn classic_lobby_resource_cores<'a>(
    parameters: &'a clonk_network::JoinGameParametersEnvelope,
    dynamic: &'a clonk_engine::NetworkResourceCore,
) -> impl Iterator<Item = &'a clonk_engine::NetworkResourceCore> + 'a {
    let player_cores = parameters
        .player_infos
        .clients
        .iter()
        .flat_map(|client| &client.players)
        .filter(|player| {
            player.flags & clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE != 0
                && player.flags & clonk_engine::PLAYER_INFO_FLAG_REMOVED == 0
                && player.flags & clonk_engine::PLAYER_INFO_FLAG_IN_SCENARIO_FILE == 0
        })
        .filter_map(|player| player.resource.as_ref());
    std::iter::once(&parameters.scenario)
        .chain(parameters.game_resources.iter())
        .chain(std::iter::once(dynamic))
        .chain(player_cores)
        .filter(|core| core.id >= 0)
}

pub(crate) fn initial_classic_lobby_resource_rows(
    snapshot: Option<&clonk_network::HostJoinSnapshot>,
) -> BTreeMap<i32, LobbyResourceRow> {
    let Some(snapshot) = snapshot else {
        return BTreeMap::new();
    };
    classic_lobby_resource_cores(&snapshot.parameters, &snapshot.dynamic)
        .map(|core| {
            (
                core.id,
                LobbyResourceRow {
                    id: core.id,
                    filename: legacy_presentation_text(core.filename.as_bytes()),
                    present_percent: 100,
                    save_possible: false,
                },
            )
        })
        .collect()
}

pub(crate) fn joined_classic_lobby_resource_rows(
    join_data: &clonk_network::JoinDataEnvelope,
    present_percent: &BTreeMap<i32, u8>,
) -> BTreeMap<i32, LobbyResourceRow> {
    classic_lobby_resource_cores(&join_data.parameters, &join_data.dynamic)
        .map(|core| {
            (
                core.id,
                LobbyResourceRow {
                    id: core.id,
                    filename: legacy_presentation_text(core.filename.as_bytes()),
                    present_percent: present_percent.get(&core.id).copied().unwrap_or(0),
                    save_possible: false,
                },
            )
        })
        .collect()
}

pub(crate) fn path_has_raw_directory_prefix(path: &Path, directory: &Path) -> bool {
    let path = path_to_legacy_bytes(path);
    let directory = path_to_legacy_bytes(directory);
    path.starts_with(&directory)
}

pub(crate) fn lobby_resource_save_possible(
    local: bool,
    complete: bool,
    resource_type: u8,
    allow_player_save: bool,
    source: &Path,
    work_directory: &Path,
) -> bool {
    if local || !complete || !path_has_raw_directory_prefix(source, work_directory) {
        return false;
    }
    resource_type == clonk_network::HostResourceType::Scenario as u8
        || resource_type == clonk_network::HostResourceType::Definitions as u8
        || resource_type == clonk_network::HostResourceType::Player as u8 && allow_player_save
}

pub(crate) fn lobby_resource_save_target(
    exe_path: &Path,
    player_path: &Path,
    core: &clonk_engine::NetworkResourceCore,
) -> Option<(PathBuf, String)> {
    let raw_basename = core
        .filename
        .as_bytes()
        .rsplit(|byte| matches!(byte, b'/' | b'\\'))
        .next()
        .filter(|basename| !basename.is_empty())?;
    let basename = legacy_presentation_text(raw_basename);
    let mut target = exe_path.to_path_buf();
    if has_player_group_extension(raw_basename) && !player_path.as_os_str().is_empty() {
        let relative_player_path = player_path
            .to_string_lossy()
            .trim_start_matches(['/', '\\'])
            .to_string();
        target.push(relative_player_path);
    }
    #[cfg(unix)]
    target.push(path_from_group_name_bytes(raw_basename));
    #[cfg(not(unix))]
    target.push(&basename);
    Some((target, basename))
}

pub(crate) fn copy_lobby_resource_item(source: &Path, target: &Path) -> io::Result<()> {
    if let Ok(metadata) = fs::symlink_metadata(target) {
        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            fs::remove_dir_all(target)?;
        } else {
            fs::remove_file(target)?;
        }
    }
    clonk_core::std_file::copy_file(source, target, false).map(|_| ())
}

pub(crate) fn initial_network_control_clock(
    network_mode: Option<&NetworkMode>,
) -> Option<NetworkControlClock> {
    match network_mode {
        None => None,
        Some(NetworkMode::Client(_)) => None,
        Some(NetworkMode::Host(HostSettings {
            prepared: Some(prepared),
            ..
        })) => {
            let config = prepared.host_config();
            let start_tick = i32::try_from(config.start_tick).unwrap_or(i32::MAX);
            let control_rate = config
                .initial_join_snapshot
                .as_ref()
                .map(|snapshot| snapshot.parameters.control_rate)
                .unwrap_or(1);
            Some(NetworkControlClock::new(start_tick, control_rate))
        }
        // The transitional non-prepared host uses HostConfig::default(),
        // whose start tick is zero and whose synthetic parameters use rate 1.
        Some(NetworkMode::Host(_)) => Some(NetworkControlClock::new(0, 1)),
    }
}

pub(crate) fn network_material_load_plan<'a>(
    network_mode: Option<&NetworkMode>,
    material_groups: Option<&'a [Group]>,
) -> (Option<&'a [Group]>, bool) {
    let prepared_host = matches!(
        network_mode,
        Some(NetworkMode::Host(HostSettings {
            prepared: Some(_),
            ..
        }))
    );
    let authoritative_groups = match network_mode {
        Some(NetworkMode::Client(_)) => material_groups,
        Some(NetworkMode::Host(HostSettings {
            prepared: Some(_), ..
        })) => material_groups,
        Some(NetworkMode::Host(_)) | None => None,
    };
    let reuse_preloaded = !(prepared_host && authoritative_groups.is_some());
    (authoritative_groups, reuse_preloaded)
}

pub(crate) fn activated_definition_load(
    retained_modules: Option<Vec<String>>,
    effective_load: ScenarioDefinitionLoad,
) -> ScenarioDefinitionLoad {
    retained_modules.map_or(effective_load, |modules| ScenarioDefinitionLoad::Fixed {
        modules,
        definition_root: None,
    })
}

pub(crate) fn recording_definition_modules(
    scenario_data: &Scenario,
    retained_modules: Option<&[String]>,
) -> Vec<String> {
    retained_modules.map_or_else(
        || {
            scenario_data
                .definition_resource_paths()
                .iter()
                .map(|path| path_as_legacy_text(path))
                .collect()
        },
        <[String]>::to_vec,
    )
}

pub(crate) fn recording_description_definition_modules(
    scenario_data: &Scenario,
    retained_modules: Option<&[String]>,
) -> Vec<Vec<u8>> {
    retained_modules.map_or_else(
        || raw_definition_description_modules(scenario_data.definition_resource_paths()),
        |modules| {
            modules
                .iter()
                .map(|module| clonk_script::c4_string_bytes(module))
                .collect()
        },
    )
}

pub(crate) fn initial_host_join_snapshot(
    network_mode: Option<&NetworkMode>,
) -> Option<clonk_network::HostJoinSnapshot> {
    network_mode.and_then(|mode| match mode {
        NetworkMode::Host(HostSettings {
            prepared: Some(prepared),
            ..
        }) => prepared.host_config().initial_join_snapshot.clone(),
        NetworkMode::Host(_) | NetworkMode::Client(_) => None,
    })
}

pub(crate) fn initial_network_max_players(network_mode: Option<&NetworkMode>) -> usize {
    network_mode
        .and_then(|mode| match mode {
            NetworkMode::Host(HostSettings {
                prepared: Some(prepared),
                ..
            }) => Some(prepared.host_config().max_players),
            NetworkMode::Host(_) | NetworkMode::Client(_) => None,
        })
        .unwrap_or(DEFAULT_SCENARIO_MAX_PLAYERS)
}

pub(crate) fn initial_host_local_alternate_colors(
    network_mode: Option<&NetworkMode>,
) -> HashMap<i32, u32> {
    network_mode
        .and_then(|mode| match mode {
            NetworkMode::Host(HostSettings {
                prepared: Some(prepared),
                ..
            }) => Some(prepared.local_player_alternate_colors_by_resource().clone()),
            NetworkMode::Host(_) | NetworkMode::Client(_) => None,
        })
        .unwrap_or_default()
}

pub(crate) fn initial_host_local_player_info_ids(
    network_mode: Option<&NetworkMode>,
) -> HashSet<i32> {
    network_mode
        .and_then(|mode| match mode {
            NetworkMode::Host(HostSettings {
                prepared: Some(prepared),
                ..
            }) => Some(
                prepared
                    .initial_host_player_info_control()
                    .players
                    .iter()
                    .filter_map(|player| {
                        player.resource.as_ref().and_then(|resource| {
                            prepared
                                .local_player_alternate_colors_by_resource()
                                .contains_key(&resource.id)
                                .then_some(player.id)
                        })
                    })
                    .filter(|id| *id > 0)
                    .collect(),
            ),
            NetworkMode::Host(_) | NetworkMode::Client(_) => None,
        })
        .unwrap_or_default()
}

/// Native keeps `dwAlternateColor` only on the host process. Locally loaded
/// rows recover it by resource identity; a row that arrived through the wire
/// was compiled into a fresh `C4PlayerInfo`, whose known default is zero.
pub(crate) fn host_runtime_alternate_color(
    local_by_resource: &HashMap<i32, u32>,
    local_player_info_ids: &HashSet<i32>,
    player: &clonk_engine::ControlPlayerInfoEntry,
) -> Option<u32> {
    if !local_player_info_ids.contains(&player.id) {
        // Network compilation constructs the remote C4PlayerInfo with the
        // field's native zero default, even if its resource aliases a local
        // player's resource core.
        return Some(0);
    }
    player
        .resource
        .as_ref()
        .and_then(|resource| local_by_resource.get(&resource.id).copied())
}

pub(crate) fn host_restore_player_info_entries(
    snapshot: Option<&clonk_network::HostJoinSnapshot>,
) -> Vec<clonk_engine::ControlPlayerInfoEntry> {
    snapshot
        .into_iter()
        .flat_map(|snapshot| player_info_list_entries(&snapshot.parameters.restore_player_infos))
        .collect()
}

pub(crate) fn player_info_list_entries(
    snapshot: &clonk_network::PlayerInfoListSnapshot,
) -> impl Iterator<Item = clonk_engine::ControlPlayerInfoEntry> + '_ {
    snapshot
        .clients
        .iter()
        .flat_map(|client| client.players.iter().cloned())
}

/// C4Game::InitGame recomputes StartupPlayerCount from the synchronized
/// PlayerInfos only for frame zero. A runtime save/dynamic retains the scalar
/// serialized when the original game began.
pub(crate) fn startup_player_count_for_init(
    frame: i32,
    serialized: Option<i32>,
    frame_zero_player_count: Option<i32>,
) -> Option<i32> {
    if frame == 0 {
        frame_zero_player_count
    } else {
        serialized
    }
}

pub(crate) fn client_network_restore_player_infos(
    network_runtime_join: bool,
    scenario_group: &Group,
    packet_restore_infos: &clonk_network::PlayerInfoListSnapshot,
    languages: &[String],
    language_packs: &LanguagePacks,
) -> clonk_network::PlayerInfoListSnapshot {
    if network_runtime_join {
        prepared_host_bootstrap::load_runtime_join_restore_player_infos(
            scenario_group,
            languages,
            language_packs,
        )
    } else {
        packet_restore_infos.clone()
    }
}

/// `C4PlayerInfoList::RestoreSavegameInfos` merges the authoritative restore
/// rows before `RecreatePlayers` scans joined infos. Keep that transition
/// separate from ordinary `JoinPlayer` issuance: full legacy Game.txt player
/// reconstruction belongs to the later savegame-host staging boundary.
pub(crate) fn route_network_savegame_recreation(
    player_infos: &mut ControlPlayerInfoRegistry,
    restore_player_infos: &[clonk_engine::ControlPlayerInfoEntry],
) -> Vec<(i32, i32)> {
    if !restore_player_infos
        .iter()
        .any(|restore| restore.flags & clonk_engine::PLAYER_INFO_FLAG_REMOVED == 0)
    {
        return Vec::new();
    }
    let (_, packets) = player_infos.retained_rows_snapshot();
    let mut seen_associations = HashSet::new();
    for savegame_player in packets
        .iter()
        .flat_map(|(_, _, players)| players)
        .map(|player| player.savegame_player)
        .filter(|id| *id != 0 && seen_associations.insert(*id))
    {
        if let Some(restore_info) = restore_player_infos
            .iter()
            .find(|restore| restore.id == savegame_player)
        {
            player_infos.resume_savegame_player_from_info(restore_info);
        }
    }
    player_infos.recreation_players()
}

pub(crate) fn synchronized_parameters_are_league(
    parameters: &clonk_network::JoinGameParametersEnvelope,
) -> bool {
    // C4GameParameters::isLeague checks LeagueAddress, not the display-name
    // League field (src/C4GameParameters.h:126-173).
    !parameters.league_address.is_empty()
}

pub(crate) fn synchronized_league_name(
    parameters: &clonk_network::JoinGameParametersEnvelope,
) -> Vec<u8> {
    parameters.league.as_bytes().to_vec()
}

pub(crate) fn classic_lobby_player_can_choose_team(
    teams: &clonk_network::JoinTeamListSnapshot,
    player: &clonk_engine::ControlPlayerInfoEntry,
    has_joined_info: bool,
) -> bool {
    if teams.active == 0
        || player.is_joined()
        || has_joined_info
        || !matches!(teams.team_distribution, 0 | 1)
    {
        return false;
    }
    if teams.auto_generate_teams != 0 {
        return true;
    }
    let current_team = teams
        .teams
        .iter()
        .find(|team| team.player_ids.contains(&player.id))
        .map(|team| team.id);
    teams.teams.iter().any(|team| {
        Some(team.id) != current_team
            && (team.max_players == 0
                || i32::try_from(team.player_ids.len()).unwrap_or(i32::MAX) < team.max_players)
    })
}

pub(crate) fn seed_engine_player_info_parameters(
    engine: &mut Engine,
    league_name: &[u8],
    player_infos: &ControlPlayerInfoRegistry,
) {
    engine.set_league_name(league_name.to_vec());
    engine.replace_player_info_league_progress_data(player_infos.league_progress_data_snapshot());
    engine.replace_player_info_league_scores(player_infos.league_scores_snapshot());
}

pub(crate) fn game_over_host_reference(
    template: &clonk_network::HostGameReference,
    parameters: clonk_network::JoinGameParametersEnvelope,
    clients: &ControlClientRegistry,
    player_infos: &ControlPlayerInfoRegistry,
    teams: &[clonk_engine::TeamInfo],
    max_players: i32,
    snapshot: &SimulationSnapshot,
) -> Result<clonk_network::HostGameReference, clonk_network::HostGameReferenceError> {
    let winner_ids = snapshot
        .players
        .iter()
        .filter(|player| player.won)
        .map(|player| player.player_info_id)
        .collect::<HashSet<_>>();
    let parameters = live_host_reference_parameters(
        parameters,
        clients,
        player_infos,
        teams,
        max_players,
        Some(&winner_ids),
    );
    template.replacing_game_over(
        parameters,
        "Running",
        snapshot.game_time,
        i32::try_from(snapshot.frame).unwrap_or(i32::MAX),
        false,
        snapshot.round_results.league_performance,
        snapshot
            .round_results
            .players
            .iter()
            .map(|player| (player.player_info_id, player.league_performance)),
    )
}

pub(crate) fn running_host_reference(
    template: &clonk_network::HostGameReference,
    parameters: clonk_network::JoinGameParametersEnvelope,
    clients: &ControlClientRegistry,
    player_infos: &ControlPlayerInfoRegistry,
    teams: &[clonk_engine::TeamInfo],
    max_players: i32,
    state: &str,
    join_allowed: bool,
    snapshot: &SimulationSnapshot,
) -> Result<clonk_network::HostGameReference, clonk_network::HostGameReferenceError> {
    let parameters =
        live_host_reference_parameters(parameters, clients, player_infos, teams, max_players, None);
    template.replacing_runtime(
        parameters,
        state,
        snapshot.game_time,
        i32::try_from(snapshot.frame).unwrap_or(i32::MAX),
        join_allowed,
        snapshot.round_results.league_performance,
    )
}

pub(crate) fn live_host_reference_parameters(
    mut parameters: clonk_network::JoinGameParametersEnvelope,
    clients: &ControlClientRegistry,
    player_infos: &ControlPlayerInfoRegistry,
    teams: &[clonk_engine::TeamInfo],
    max_players: i32,
    winner_ids: Option<&HashSet<i32>>,
) -> clonk_network::JoinGameParametersEnvelope {
    parameters.max_players = max_players;
    parameters.clients = clonk_network::JoinClientRegistrySnapshot::new(clients.snapshot());
    let (last_player_id, retained_rows) = player_infos.retained_rows_snapshot();
    parameters.player_infos = clonk_network::PlayerInfoListSnapshot {
        last_player_id,
        clients: retained_rows
            .into_iter()
            .map(|(client_id, flags, mut players)| {
                for player in &mut players {
                    if winner_ids.is_some_and(|winner_ids| winner_ids.contains(&player.id)) {
                        player.flags |= clonk_engine::PLAYER_INFO_FLAG_WON;
                    }
                }
                clonk_network::ClientPlayerInfosSnapshot {
                    client_id,
                    flags,
                    players,
                }
            })
            .collect(),
    };
    project_live_team_memberships(&mut parameters.teams, teams);
    parameters
}

pub(crate) fn project_live_team_memberships(
    target: &mut clonk_network::JoinTeamListSnapshot,
    teams: &[clonk_engine::TeamInfo],
) {
    let mut matched = vec![false; teams.len()];
    for target_team in &mut target.teams {
        let Some((index, team)) = teams
            .iter()
            .enumerate()
            .find(|(index, team)| !matched[*index] && team.id == target_team.id)
        else {
            target_team.player_ids.clear();
            continue;
        };
        matched[index] = true;
        target_team.player_ids.clone_from(&team.player_ids);
    }
    for (index, team) in teams.iter().enumerate() {
        if matched[index] {
            continue;
        }
        let c_string = |value: &str, max_bytes: Option<usize>| {
            let mut bytes = clonk_script::c4_string_bytes(value);
            if let Some(nul) = bytes.iter().position(|byte| *byte == 0) {
                bytes.truncate(nul);
            }
            if let Some(max_bytes) = max_bytes {
                bytes.truncate(max_bytes);
            }
            clonk_engine::LegacyCString::from_bytes(bytes)
                .expect("truncating at the first NUL always yields a legacy C string")
        };
        target.teams.push(clonk_network::JoinTeamSnapshot {
            id: team.id,
            name: c_string(&team.name, Some(30)),
            player_start_index: team.player_start_index,
            player_ids: team.player_ids.clone(),
            color: team.color,
            icon_spec: c_string(team.icon_spec.as_deref().unwrap_or_default(), None),
            max_players: team.max_players,
        });
    }
    target.last_team_id = target
        .last_team_id
        .max(teams.iter().map(|team| team.id).fold(0, i32::max));
}

pub(crate) fn runtime_teams_from_initial_metadata(
    metadata: &clonk_engine::InitialNetworkTeamMetadata,
) -> Vec<clonk_engine::TeamInfo> {
    metadata
        .teams
        .iter()
        .map(|team| {
            clonk_engine::TeamInfo::new(
                team.id,
                clonk_script::c4_string_from_bytes(team.name.as_bytes()),
                team.color,
            )
            .with_player_ids(team.player_ids.clone())
            .with_player_start_index(team.player_start_index)
            .with_max_players(team.max_players)
            .with_icon_spec(clonk_script::c4_string_from_bytes(
                team.icon_spec.as_bytes(),
            ))
        })
        .collect()
}

pub(crate) fn runtime_teams_from_join_snapshot(
    snapshot: &clonk_network::JoinTeamListSnapshot,
) -> Vec<clonk_engine::TeamInfo> {
    snapshot
        .teams
        .iter()
        .map(|team| {
            clonk_engine::TeamInfo::new(
                team.id,
                clonk_script::c4_string_from_bytes(team.name.as_bytes()),
                team.color,
            )
            .with_player_ids(team.player_ids.clone())
            .with_player_start_index(team.player_start_index)
            .with_max_players(team.max_players)
            .with_icon_spec(clonk_script::c4_string_from_bytes(
                team.icon_spec.as_bytes(),
            ))
        })
        .collect()
}

pub(crate) fn initial_team_metadata_from_join_snapshot(
    snapshot: &clonk_network::JoinTeamListSnapshot,
) -> Option<clonk_engine::InitialNetworkTeamMetadata> {
    let team_distribution = match snapshot.team_distribution {
        0 => clonk_engine::InitialNetworkTeamDistribution::Free,
        1 => clonk_engine::InitialNetworkTeamDistribution::Host,
        2 => clonk_engine::InitialNetworkTeamDistribution::None,
        3 => clonk_engine::InitialNetworkTeamDistribution::Random,
        4 => clonk_engine::InitialNetworkTeamDistribution::RandomInvisible,
        _ => return None,
    };
    Some(clonk_engine::InitialNetworkTeamMetadata {
        active: snapshot.active != 0,
        custom: snapshot.custom != 0,
        allow_hostility_change: snapshot.allow_hostility_change != 0,
        allow_team_switch: snapshot.allow_team_switch != 0,
        auto_generate_teams: snapshot.auto_generate_teams != 0,
        last_team_id: snapshot.last_team_id,
        team_distribution,
        team_colors: snapshot.team_colors != 0,
        max_script_players: snapshot.max_script_players,
        script_player_names: snapshot.script_player_names.clone(),
        random_team_count: snapshot.random_team_count,
        teams: snapshot
            .teams
            .iter()
            .map(|team| clonk_engine::InitialNetworkTeam {
                id: team.id,
                name: team.name.clone(),
                player_start_index: team.player_start_index,
                player_ids: team.player_ids.clone(),
                color: team.color,
                icon_spec: team.icon_spec.clone(),
                max_players: team.max_players,
            })
            .collect(),
    })
}

pub(crate) fn initial_team_from_runtime(
    team: &clonk_engine::TeamInfo,
) -> clonk_engine::InitialNetworkTeam {
    let legacy = |value: &str, max_bytes: Option<usize>| {
        let mut bytes = clonk_script::c4_string_bytes(value);
        if let Some(nul) = bytes.iter().position(|byte| *byte == 0) {
            bytes.truncate(nul);
        }
        if let Some(max_bytes) = max_bytes {
            bytes.truncate(max_bytes);
        }
        clonk_engine::LegacyCString::from_bytes(bytes)
            .expect("truncating at the first NUL always yields a legacy C string")
    };
    clonk_engine::InitialNetworkTeam {
        id: team.id,
        name: legacy(&team.name, Some(30)),
        player_start_index: team.player_start_index,
        player_ids: team.player_ids.clone(),
        color: team.color,
        icon_spec: legacy(team.icon_spec.as_deref().unwrap_or_default(), None),
        max_players: team.max_players,
    }
}

pub(crate) fn initial_team_metadata_from_runtime(
    configuration: TeamConfiguration,
    teams: &[clonk_engine::TeamInfo],
) -> Option<clonk_engine::InitialNetworkTeamMetadata> {
    let team_distribution = match configuration.distribution {
        0 => clonk_engine::InitialNetworkTeamDistribution::Free,
        1 => clonk_engine::InitialNetworkTeamDistribution::Host,
        2 => clonk_engine::InitialNetworkTeamDistribution::None,
        3 => clonk_engine::InitialNetworkTeamDistribution::Random,
        4 => clonk_engine::InitialNetworkTeamDistribution::RandomInvisible,
        _ => return None,
    };
    Some(clonk_engine::InitialNetworkTeamMetadata {
        active: configuration.active,
        custom: configuration.custom,
        allow_hostility_change: configuration.allow_hostility_change,
        allow_team_switch: configuration.allow_team_switch,
        auto_generate_teams: configuration.auto_generate_teams,
        last_team_id: teams.iter().map(|team| team.id).fold(0, i32::max),
        team_distribution,
        team_colors: configuration.team_colors,
        max_script_players: 0,
        script_player_names: clonk_engine::LegacyCString::default(),
        random_team_count: 0,
        teams: teams.iter().map(initial_team_from_runtime).collect(),
    })
}

pub(crate) fn project_runtime_memberships_into_initial_metadata(
    metadata: &mut clonk_engine::InitialNetworkTeamMetadata,
    runtime_teams: &[clonk_engine::TeamInfo],
) {
    let mut matched = vec![false; runtime_teams.len()];
    for team in &mut metadata.teams {
        if let Some((index, runtime)) = runtime_teams
            .iter()
            .enumerate()
            .find(|(index, runtime)| !matched[*index] && runtime.id == team.id)
        {
            matched[index] = true;
            team.player_ids.clone_from(&runtime.player_ids);
        }
    }
    metadata.teams.extend(
        runtime_teams
            .iter()
            .enumerate()
            .filter(|(index, _)| !matched[*index])
            .map(|(_, team)| initial_team_from_runtime(team)),
    );
    metadata.last_team_id = metadata
        .last_team_id
        .max(runtime_teams.iter().map(|team| team.id).fold(0, i32::max));
}

pub(crate) fn ordered_control_player_team_memberships(
    player_infos: &ControlPlayerInfoRegistry,
) -> Vec<(i32, i32)> {
    let (_, rows) = player_infos.retained_rows_snapshot();
    let mut ids = rows
        .iter()
        .flat_map(|(_, _, players)| players)
        .map(|player| player.id)
        .filter(|id| *id > 0)
        .collect::<Vec<_>>();
    ids.sort_unstable();
    ids.dedup();
    ids.into_iter()
        .filter_map(|id| {
            player_infos.get(id).and_then(|player| {
                (player.flags & clonk_engine::PLAYER_INFO_FLAG_REMOVED == 0)
                    .then_some((id, player.team))
            })
        })
        .collect()
}

fn recheck_one_team_membership(
    team_id: i32,
    player_ids: &mut Vec<i32>,
    memberships: &[(i32, i32)],
) {
    player_ids.retain(|player_id| {
        memberships
            .iter()
            .any(|(id, assigned_team)| id == player_id && *assigned_team == team_id)
    });
    for (player_id, _) in memberships
        .iter()
        .filter(|(_, assigned_team)| *assigned_team == team_id)
    {
        if !player_ids.contains(player_id) {
            player_ids.push(*player_id);
        }
    }
}

pub(crate) fn recheck_runtime_team_memberships_from_infos(
    teams: &mut [clonk_engine::TeamInfo],
    memberships: &[(i32, i32)],
) {
    for team in teams {
        recheck_one_team_membership(team.id, &mut team.player_ids, memberships);
    }
}

pub(crate) fn recheck_join_team_memberships_from_infos(
    teams: &mut [clonk_network::JoinTeamSnapshot],
    memberships: &[(i32, i32)],
) {
    for team in teams {
        recheck_one_team_membership(team.id, &mut team.player_ids, memberships);
    }
}

pub(crate) fn synchronized_team_configuration(
    parameters: &clonk_network::JoinGameParametersEnvelope,
) -> TeamConfiguration {
    let teams = &parameters.teams;
    TeamConfiguration {
        custom: teams.custom != 0,
        active: teams.active != 0,
        allow_hostility_change: teams.allow_hostility_change != 0,
        distribution: i32::from(teams.team_distribution),
        allow_team_switch: teams.allow_team_switch != 0,
        auto_generate_teams: teams.auto_generate_teams != 0,
        team_colors: teams.team_colors != 0,
    }
}

pub(crate) fn synchronized_rule_goal_lists(
    parameters: &clonk_network::JoinGameParametersEnvelope,
) -> clonk_engine::GameParameterRuleGoalLists {
    let entries = |source: &[clonk_network::JoinDataIdListEntry]| {
        source
            .iter()
            .map(|entry| {
                clonk_engine::ScenarioIdListEntry::new(
                    String::from_utf8_lossy(entry.id.as_bytes()).into_owned(),
                    entry.count,
                )
            })
            .collect::<Vec<_>>()
    };
    clonk_engine::GameParameterRuleGoalLists::new(
        entries(&parameters.rules),
        entries(&parameters.goals),
    )
}

pub(crate) fn initial_network_is_league(network_mode: Option<&NetworkMode>) -> bool {
    network_mode.is_some_and(|mode| match mode {
        NetworkMode::Host(HostSettings {
            prepared: Some(prepared),
            ..
        }) => prepared
            .host_config()
            .initial_join_snapshot
            .as_ref()
            .is_some_and(|snapshot| synchronized_parameters_are_league(&snapshot.parameters)),
        NetworkMode::Host(_) | NetworkMode::Client(_) => false,
    })
}

pub(crate) fn retain_player_infos_with_cpp_swap_remove(
    players: &mut Vec<clonk_engine::ControlPlayerInfoEntry>,
    mut retain: impl FnMut(&mut clonk_engine::ControlPlayerInfoEntry) -> bool,
) {
    let mut index = 0;
    while index < players.len() {
        if retain(&mut players[index]) {
            index += 1;
        } else {
            players.swap_remove(index);
        }
    }
}

pub(crate) fn initial_network_league_name(network_mode: Option<&NetworkMode>) -> Vec<u8> {
    network_mode
        .and_then(|mode| match mode {
            NetworkMode::Host(HostSettings {
                prepared: Some(prepared),
                ..
            }) => prepared.host_config().initial_join_snapshot.as_ref(),
            NetworkMode::Host(_) | NetworkMode::Client(_) => None,
        })
        .map_or_else(Vec::new, |snapshot| {
            synchronized_league_name(&snapshot.parameters)
        })
}

pub(crate) fn initial_network_stream_address(network_mode: Option<&NetworkMode>) -> LegacyCString {
    network_mode
        .and_then(|mode| match mode {
            NetworkMode::Host(HostSettings {
                prepared: Some(prepared),
                ..
            }) => Some(prepared.stream_address().clone()),
            NetworkMode::Host(_) | NetworkMode::Client(_) => None,
        })
        .unwrap_or_default()
}

pub(crate) fn initial_network_team_assignment(
    network_mode: Option<&NetworkMode>,
    generated_team_name_template: &LegacyCString,
) -> Option<NetworkTeamAssignmentState> {
    network_mode.and_then(|mode| match mode {
        NetworkMode::Host(HostSettings {
            prepared: Some(prepared),
            ..
        }) => Some(
            NetworkTeamAssignmentState::from_prepared_host_with_team_name_template(
                prepared.runtime_team_metadata().clone(),
                generated_team_name_template.clone(),
            ),
        ),
        NetworkMode::Host(_) | NetworkMode::Client(_) => None,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum StartupPlayerPropertiesOrigin {
    MainMenuFirstPlayer,
    SelectionNew,
    SelectionEdit { path: PathBuf, was_activated: bool },
}

pub(crate) struct PendingStartupPlayerProperties {
    pub(crate) origin: StartupPlayerPropertiesOrigin,
    pub(crate) controller: clonk_frontend::startup_plrproperties::PlayerPropertiesController,
}

pub(crate) fn startup_player_big_icon(portrait: &ImageData, color_dw: u32) -> Option<ImageData> {
    let colored = clonk_frontend::hud::colorize_by_owner_software(
        portrait,
        Color::opaque(
            ((color_dw >> 16) & 0xff) as u8,
            ((color_dw >> 8) & 0xff) as u8,
            (color_dw & 0xff) as u8,
        ),
    );
    materialize_startup_player_image(&colored, 64)
}

pub(crate) fn resize_startup_player_image(image: &ImageData, maximum: u32) -> ImageData {
    let width = image.width();
    let height = image.height();
    if width == 0 || height == 0 {
        return ImageData::new(0, 0, Vec::new());
    }
    let largest = width.max(height);
    if largest <= maximum {
        return image.clone();
    }
    let resized_width = (u64::from(width) * u64::from(maximum) / u64::from(largest)) as u32;
    let resized_height = (u64::from(height) * u64::from(maximum) / u64::from(largest)) as u32;
    // C4FacetExSurface::CopyFromSfcMaxSize forwards a zero truncated extent
    // to C4Surface::Create, ignores the failure and leaves a blank 0×0 Face.
    if resized_width == 0 || resized_height == 0 {
        return ImageData::new(0, 0, Vec::new());
    }
    let mut pixels = vec![0; resized_width as usize * resized_height as usize * 4];
    for y in 0..resized_height {
        let source_y = (u64::from(y) * u64::from(height) / u64::from(resized_height)) as u32;
        for x in 0..resized_width {
            let source_x = (u64::from(x) * u64::from(width) / u64::from(resized_width)) as u32;
            let source = ((source_y * width + source_x) * 4) as usize;
            let target = ((y * resized_width + x) * 4) as usize;
            pixels[target..target + 4].copy_from_slice(&image.pixels()[source..source + 4]);
        }
    }
    ImageData::new(resized_width, resized_height, pixels)
}

pub(crate) fn materialize_startup_player_image(
    image: &ImageData,
    maximum: u32,
) -> Option<ImageData> {
    let image = resize_startup_player_image(image, maximum);
    (image.width() != 0 && image.height() != 0).then_some(image)
}

pub(crate) fn startup_player_image_from_rgba(
    width: u32,
    height: u32,
    mut pixels: Vec<u8>,
) -> ImageData {
    // C4Surface::ReadPNG canonicalizes fully transparent source texels before
    // CreateColorByOwner or CopyFromSfcMaxSize can inspect them.
    for pixel in pixels.chunks_exact_mut(4) {
        if pixel[3] == 0 {
            pixel[..3].fill(0);
        }
    }
    ImageData::new(width, height, pixels)
}

pub(crate) fn load_startup_portrait_image(path: &Path) -> std::result::Result<ImageData, String> {
    let rgba = image::open(path)
        .map_err(|error| error.to_string())?
        .into_rgba8();
    let (width, height) = rgba.dimensions();
    Ok(startup_player_image_from_rgba(
        width,
        height,
        rgba.into_raw(),
    ))
}

pub(crate) const DEFAULT_USER_PORTRAITS: [(&str, &str); 9] = [
    ("Portrait1.png", "Clonk.png"),
    ("PortraitBandit.png", "Bandit.png"),
    ("PortraitIndianChief.png", "IndianChief.png"),
    ("PortraitKing.png", "King.png"),
    ("PortraitKnight.png", "Knight.png"),
    ("PortraitMage.png", "Mage.png"),
    ("PortraitPiranha.png", "Piranha.png"),
    ("PortraitSheriff.png", "Sheriff.png"),
    ("PortraitWipf.png", "Wipf.png"),
];

pub(crate) fn extract_default_startup_portraits_once(paths: &AppPaths) {
    let config_path = paths.config_file();
    let mut config = match Config::load(&config_path) {
        Ok(config) => config,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Config::new(),
        Err(error) => {
            tracing::warn!(%error, path = %config_path.display(), "failed to read portrait extraction flag");
            return;
        }
    };
    if config
        .get_in(Some("General"), "UserPortraitsWritten")
        .is_some_and(parse_config_bool)
    {
        return;
    }

    if let Err(error) = fs::create_dir_all(paths.user_data_dir()) {
        tracing::warn!(%error, path = %paths.user_data_dir().display(), "failed to create portrait directory");
    } else {
        match main_graphics_group(paths) {
            Ok(graphics) => {
                for (source, destination) in DEFAULT_USER_PORTRAITS {
                    let result = graphics
                        .read_file(source)
                        .map_err(|error| error.to_string())
                        .and_then(|bytes| {
                            fs::write(paths.user_data_dir().join(destination), bytes)
                                .map_err(|error| error.to_string())
                        });
                    if let Err(error) = result {
                        tracing::warn!(%error, source, destination, "failed to extract bundled portrait");
                    }
                }
            }
            Err(error) => {
                tracing::warn!(%error, "failed to open bundled portraits");
            }
        }
    }

    config.set_in(Some("General"), "UserPortraitsWritten", "1");
    if let Some(parent) = config_path.parent() {
        if let Err(error) = fs::create_dir_all(parent) {
            tracing::warn!(%error, path = %parent.display(), "failed to create config directory for portrait flag");
            return;
        }
    }
    if let Err(error) =
        save_config_preserving_native_general_booleans(&config, &config_path, None, None)
    {
        tracing::warn!(%error, path = %config_path.display(), "failed to persist portrait extraction flag");
    }
}

pub(crate) fn startup_player_image_write(
    update: &clonk_frontend::startup_plrproperties::PlayerImageUpdate,
) -> PlayerImageWrite {
    match update {
        clonk_frontend::startup_plrproperties::PlayerImageUpdate::Keep => PlayerImageWrite::Keep,
        clonk_frontend::startup_plrproperties::PlayerImageUpdate::Replace(image) => {
            PlayerImageWrite::Replace(image.clone())
        }
        clonk_frontend::startup_plrproperties::PlayerImageUpdate::Clear => PlayerImageWrite::Clear,
    }
}

/// Player-owned `C4MainMenu` state keyed by `C4Player::Number`
/// (`C4Player.h:85`).
#[derive(Default)]
pub(crate) struct PlayerIngameMenus {
    pub(crate) by_player: BTreeMap<i32, IngameMenuState>,
}

impl PlayerIngameMenus {
    pub(crate) fn is_none(&self) -> bool {
        self.by_player.is_empty()
    }

    pub(crate) fn is_some(&self) -> bool {
        !self.is_none()
    }

    pub(crate) fn as_ref(&self) -> Option<&IngameMenuState> {
        self.by_player.values().next()
    }

    pub(crate) fn as_mut(&mut self) -> Option<&mut IngameMenuState> {
        self.by_player.values_mut().next()
    }

    pub(crate) fn contains(&self, player: i32) -> bool {
        self.by_player.contains_key(&player)
    }

    pub(crate) fn get(&self, player: i32) -> Option<&IngameMenuState> {
        self.by_player.get(&player)
    }

    pub(crate) fn get_mut(&mut self, player: i32) -> Option<&mut IngameMenuState> {
        self.by_player.get_mut(&player)
    }

    pub(crate) fn replace(&mut self, player: i32, menu: Option<IngameMenuState>) {
        match menu {
            Some(menu) => {
                self.by_player.insert(player, menu.for_player(player));
            }
            None => {
                self.by_player.remove(&player);
            }
        }
    }

    pub(crate) fn remove(&mut self, player: i32) -> Option<IngameMenuState> {
        self.by_player.remove(&player)
    }

    pub(crate) fn clear(&mut self) {
        self.by_player.clear();
    }

    pub(crate) fn values_mut(&mut self) -> impl Iterator<Item = &mut IngameMenuState> {
        self.by_player.values_mut()
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (i32, &IngameMenuState)> {
        self.by_player.iter().map(|(&player, menu)| (player, menu))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScreenshotKind {
    PresentedFrame,
    FullLandscape,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ScreenshotRequest {
    pub(crate) kind: ScreenshotKind,
    pub(crate) gamma: clonk_graphics::GammaRamp,
}

#[derive(Debug)]
pub(crate) struct PendingNativeSaveThumbnail {
    pub(crate) path: PathBuf,
    pub(crate) packed_group: Vec<u8>,
}

#[derive(Debug)]
pub(crate) struct ScreenshotSaveOutcome {
    pub(crate) kind: ScreenshotKind,
    pub(crate) path: PathBuf,
    pub(crate) result: Result<()>,
}

#[derive(Clone, Debug)]
pub(crate) struct ClassicMessageBoardState {
    pub(crate) mode: MessageBoardMode,
    pub(crate) line_count: i32,
    pub(crate) delay: i32,
    pub(crate) fader: i32,
    pub(crate) speed: i32,
    pub(crate) empty: bool,
    pub(crate) screen_fader: i32,
    pub(crate) back_scroll: i32,
    pub(crate) log_history: VecDeque<String>,
}

impl Default for ClassicMessageBoardState {
    fn default() -> Self {
        Self {
            mode: MessageBoardMode::SingleLine,
            line_count: 4,
            delay: -1,
            fader: 0,
            speed: 2,
            empty: true,
            screen_fader: 0,
            back_scroll: -1,
            log_history: VecDeque::new(),
        }
    }
}

impl ClassicMessageBoardState {
    pub(crate) fn initialize(&mut self, enabled: bool, line_height: i32) {
        *self = Self::default();
        self.line_count = i32::from(enabled);
        self.change_mode(
            if enabled {
                MessageBoardMode::SingleLine
            } else {
                MessageBoardMode::Hidden
            },
            line_height,
        );
    }

    /// `C4MessageBoard::ChangeMode`; the returned bool is the exact value
    /// assigned to the bool-typed `Config.Graphics.MsgBoard` field.
    pub(crate) fn change_mode(&mut self, mode: MessageBoardMode, line_height: i32) -> bool {
        let enabled = match mode {
            MessageBoardMode::SingleLine => {
                if self.mode == MessageBoardMode::Hidden {
                    self.back_scroll = -1;
                    self.empty = true;
                } else {
                    self.back_scroll = -1;
                    self.fader = -1;
                    self.empty = false;
                    self.speed = 2;
                    self.screen_fader = MESSAGE_BOARD_MAX_FADING_LINES * line_height;
                }
                true
            }
            MessageBoardMode::Continuous => {
                self.line_count = self.line_count.max(2);
                self.back_scroll = -1;
                self.fader = 0;
                true
            }
            MessageBoardMode::Hidden => false,
        };
        self.mode = mode;
        enabled
    }

    pub(crate) fn set_line_count(&mut self, line_count: i32, line_height: i32) -> bool {
        match line_count.clamp(0, 20) {
            0 => self.change_mode(MessageBoardMode::Hidden, line_height),
            1 => self.change_mode(MessageBoardMode::SingleLine, line_height),
            count => {
                self.line_count = count;
                self.change_mode(MessageBoardMode::Continuous, line_height)
            }
        }
    }

    /// `C4MessageBoard::Execute`, advanced once for each app graphics frame.
    fn execute(&mut self, line_height: i32, type_in: bool) -> bool {
        if self.mode == MessageBoardMode::Continuous {
            return false;
        }
        if self.mode == MessageBoardMode::Hidden && !type_in {
            self.screen_fader = 100;
            self.back_scroll = -1;
            return false;
        }

        if type_in {
            self.screen_fader = (self.screen_fader - 20).max(-100);
        }
        if self.back_scroll < 0 {
            self.empty = true;
            return !type_in;
        }
        if self.empty {
            self.fader = line_height;
            self.delay = -1;
            self.empty = false;
        }

        self.speed = (self.back_scroll / 5).max(1);
        if self.fader > 0 {
            self.fader = (self.fader - self.speed).max(0);
        }
        if self.fader < 0 {
            self.fader = (self.fader - self.speed).max(-line_height);
        }
        if self.fader == 0 {
            if self.delay == -1 {
                let index = (-self.back_scroll).min(-1);
                self.delay = self
                    .log_line(index)
                    .map(|line| clonk_script::c4_string_byte_len(line) as i32)
                    .unwrap_or(0);
            }
            if self.delay > 0 {
                self.delay = (self.delay - self.speed).max(0);
            }
            if self.delay == 0 {
                self.fader = (-self.speed).max(-line_height);
                self.delay = -1;
            }
        }
        self.screen_fader = (self.screen_fader - 20).max(-100);
        if self.fader == -line_height {
            self.back_scroll = (self.back_scroll - 1).max(-1);
            self.fader = 0;
        }
        false
    }

    fn log_line(&self, negative_index: i32) -> Option<&str> {
        let offset = usize::try_from(negative_index.checked_neg()?).ok()?;
        self.log_history
            .len()
            .checked_sub(offset)
            .and_then(|index| self.log_history.get(index))
            .map(String::as_str)
    }

    pub(crate) fn current_line(&self) -> Option<String> {
        self.log_line(-self.back_scroll - 1).map(str::to_string)
    }

    /// `AddLog` followed by the ordinary main-thread `LogNotify` call.
    pub(crate) fn enqueue(&mut self, line: String) {
        self.log_history.push_back(line);
        while self.log_history.len() > 1000
            || self
                .log_history
                .iter()
                .map(|line| clonk_script::c4_string_byte_len(line).saturating_add(1))
                .sum::<usize>()
                > 30_000
        {
            self.log_history.pop_front();
        }
        self.back_scroll = 0;
    }

    pub(crate) fn scroll(&mut self, older: bool) {
        self.delay = -1;
        self.fader = 0;
        self.empty = false;
        if older {
            self.back_scroll = self.back_scroll.saturating_add(1);
        } else if self.back_scroll > -1 {
            self.back_scroll -= 1;
        }
    }

    pub(crate) fn clear_log(&mut self) {
        self.log_history.clear();
    }

    pub(crate) fn overlay(&mut self, line_height: i32, type_in: bool) -> MessageBoardOverlay {
        if self.mode != MessageBoardMode::Hidden || type_in {
            self.screen_fader = self
                .screen_fader
                .min(MESSAGE_BOARD_MAX_FADING_LINES * line_height);
        }
        MessageBoardOverlay {
            mode: self.mode,
            line_count: self.line_count,
            log_lines: self.log_history.iter().cloned().collect(),
            back_scroll: self.back_scroll,
            fader: self.fader,
            screen_fader: self.screen_fader,
            type_in,
        }
    }

    pub(crate) fn advance_frame(&mut self, line_height: i32, type_in: bool) -> MessageBoardOverlay {
        let increment_screen_fader_after_draw = self.execute(line_height, type_in);
        let overlay = self.overlay(line_height, type_in);
        if increment_screen_fader_after_draw {
            self.screen_fader += 5;
        }
        overlay
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OptionsDisplayRequest {
    SetMode(clonk_frontend::startup_options_graphics::GraphicsDisplayMode),
    SetScale { percent: i32, persist: bool },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ActiveViewportEdgeScroll {
    pub(crate) viewport_index: usize,
    pub(crate) owner: i32,
    pub(crate) observer: bool,
    pub(crate) screen: GuiPoint,
    pub(crate) edge: ViewportEdgeScroll,
}

const FREE_VIEW_SCROLL_MOMENTUM_WINDOW: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct FreeViewScrollMomentum {
    velocity: Vector2,
    pub(crate) most_recent: Option<Instant>,
}

impl FreeViewScrollMomentum {
    pub(crate) fn apply(&mut self, requested: Vector2, now: Instant) -> Vector2 {
        let carries = self.most_recent.is_some_and(|most_recent| {
            now.checked_duration_since(most_recent)
                .is_some_and(|elapsed| elapsed < FREE_VIEW_SCROLL_MOMENTUM_WINDOW)
        });
        let mut applied = requested;
        if carries {
            applied += self.velocity;
        }
        self.velocity = applied;
        self.most_recent = Some(now);
        applied
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RetainedViewportMouse {
    pub(crate) viewport_index: usize,
    pub(crate) owner: i32,
    pub(crate) observer: bool,
    pub(crate) position: Vector2,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ConstructionMenuDrag {
    Candidate {
        owner: i32,
        menu_object_id: ObjectId,
        item_index: usize,
        definition_id: String,
        definition_c4id: i32,
        down: GuiPoint,
    },
    Active {
        owner: i32,
        definition_id: String,
        definition_c4id: i32,
        viewport_index: Option<usize>,
        pointer: Option<ViewportPointer>,
        site_valid: bool,
    },
}

impl ConstructionMenuDrag {
    pub(crate) fn is_active(&self) -> bool {
        matches!(self, Self::Active { .. })
    }
}

/// Runtime dialogs that inherit C4GUI::Dialog's default z=0. Screen keeps
/// equal-z dialogs in show/activation order: the last entry is drawn and hit
/// tested on top.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RuntimeDefaultDialog {
    Scoreboard,
    NetworkChart,
    ClientList,
    GameOver,
    ExternalIrc,
}
