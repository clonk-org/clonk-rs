use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, LineWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{anyhow, Context, Result};
use lc_graphics::{Color, PixelFormat, Surface};
use lc_gui::{DrawCommand, GuiEvent, KeyCode, Point as GuiPoint, Rect as GuiRect, Size as GuiSize};
use lc_launcher::{
    copy_support_artifacts, copy_support_bundle, ensure_support_bundle, load_launcher_preferences,
    load_shell_state, reveal_in_file_manager, save_launcher_preferences, timestamp_for_filename,
    timestamp_for_log, write_launcher_summary, LauncherLog, LauncherPreferences,
    LauncherShellState, ProviderAutomationRecord, ProviderAutomationSnapshot,
    ProviderAutomationState, ProviderDiagnostics, ProviderPathStatus, ProviderStatus,
    SupportArtifact, UpdateTelemetrySummary,
};
use lc_launcher_ui::{
    ActionFeedback, LauncherShellMessage, LauncherShellResponse, LauncherShellUi,
};
use lc_platform::AppPaths;
use pixels::{Pixels, SurfaceTexture};
use rfd::FileDialog;
use serde::Serialize;
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
    preferences: LauncherPreferences,
    providers: FirstPartyProviders,
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
        let preferences = match load_launcher_preferences(&paths) {
            Ok(preferences) => preferences,
            Err(err) => {
                logger
                    .log_line(&format!("failed to load launcher preferences: {err}"))
                    .context("failed to record launcher preferences load failure")?;
                LauncherPreferences::default()
            }
        };
        let providers = FirstPartyProviders::discover(&paths);

        let mut app = Self {
            paths,
            logger,
            ui,
            surface: Surface::new(width, height, PixelFormat::Rgba8888),
            preferences,
            providers,
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

    fn update_feedback(&mut self, feedback: Option<ActionFeedback>) -> Result<()> {
        self.ui
            .set_action_feedback(feedback)
            .map_err(|err| anyhow!(err))
    }

    fn set_feedback(&mut self, feedback: ActionFeedback) -> Result<()> {
        self.update_feedback(Some(feedback))
    }

    fn update_provider_diagnostics(&mut self) -> Result<ProviderDiagnostics> {
        let diagnostics = self.providers.diagnostics();
        self.ui
            .set_providers(diagnostics.clone())
            .map_err(|err| anyhow!(err))?;
        Ok(diagnostics)
    }

    fn feedback_from_reports(
        &self,
        message: String,
        stage: &ProviderStageReport,
        automation: &ProviderAutomationReport,
    ) -> ActionFeedback {
        let has_failure = !stage.failures().is_empty() || !automation.failures().is_empty();
        let has_success = !stage.successes().is_empty() || !automation.successes().is_empty();
        let has_skipped = !automation.skipped().is_empty();
        if has_failure {
            ActionFeedback::error(message)
        } else if has_success && !has_skipped {
            ActionFeedback::success(message)
        } else {
            ActionFeedback::info(message)
        }
    }

    fn persist_provider_snapshot(
        &self,
        state: &LauncherShellState,
        diagnostics: &ProviderDiagnostics,
    ) -> Result<ProviderAutomationSnapshot> {
        let telemetry = UpdateTelemetrySummary::from_serializable(
            &state.summary.update_telemetry,
            &state.logs_dir,
        );
        let snapshot = ProviderAutomationSnapshot::from_diagnostics(diagnostics, &state.logs_dir);
        write_launcher_summary(
            &self.paths,
            &self.logger,
            &state.launcher_log_path,
            &state.runtime_log_paths,
            &state.crash_report_paths,
            &telemetry,
            state.support_bundle_path.as_deref(),
            Some(snapshot.clone()),
        )
        .context("failed to persist provider automation snapshot")?;
        Ok(snapshot)
    }

    fn persist_preferences(&self) {
        if let Err(err) = save_launcher_preferences(&self.paths, &self.preferences) {
            let _ = self
                .logger
                .log_line(&format!("failed to persist launcher preferences: {err}"));
        }
    }

    fn remember_bundle_destination(&mut self, path: &Path) {
        self.preferences.set_bundle_destination(path);
        self.persist_preferences();
    }

    fn remember_upload_destination(&mut self, path: &Path) {
        self.preferences.set_upload_destination(path);
        self.persist_preferences();
    }

    fn configure_dialog_directory(
        &self,
        dialog: FileDialog,
        saved: Option<PathBuf>,
        fallback: Option<&Path>,
    ) -> FileDialog {
        if let Some(saved_path) = saved {
            return dialog.set_directory(saved_path);
        }
        if let Some(fallback) = fallback {
            return dialog.set_directory(fallback);
        }
        dialog.set_directory(self.paths.logs_dir())
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
            LauncherShellMessage::CopySupportBundle { bundle_path } => {
                self.logger
                    .log_line("copy support bundle action requested via launcher UI")
                    .context("failed to log copy request")?;

                if !bundle_path.exists() {
                    let message = format!(
                        "Support bundle {} is missing; regenerate it before copying.",
                        bundle_path.display()
                    );
                    self.logger
                        .log_line(&format!("support bundle copy aborted: {message}"))
                        .context("failed to log missing bundle")?;
                    self.set_feedback(ActionFeedback::error(message))?;
                    return Ok(());
                }

                let dialog = FileDialog::new().set_title("Choose where to copy the support bundle");
                let dialog = self.configure_dialog_directory(
                    dialog,
                    self.preferences.bundle_destination_path(),
                    bundle_path.parent(),
                );

                match dialog.pick_folder() {
                    Some(destination) => {
                        self.remember_bundle_destination(&destination);
                        match copy_support_bundle(&bundle_path, &destination) {
                            Ok(staged_path) => {
                                self.logger
                                    .log_line(&format!(
                                        "support bundle copied to {}",
                                        staged_path.display()
                                    ))
                                    .context("failed to log copy success")?;
                                let state_snapshot = self.ui.state().cloned();
                                let (stage_report, automation_report) =
                                    self.providers.stage_bundle(&self.logger, &bundle_path);
                                let diagnostics = self
                                    .update_provider_diagnostics()
                                    .context(
                                        "failed to refresh provider diagnostics after support bundle staging",
                                    )?;
                                if let Some(mut state) = state_snapshot {
                                    let snapshot = self
                                        .persist_provider_snapshot(&state, &diagnostics)
                                        .context(
                                        "failed to persist provider automation after bundle staging",
                                    )?;
                                    state.summary.provider_automation = snapshot;
                                    self.ui
                                        .set_state(Some(state))
                                        .map_err(|err| anyhow!(err))
                                        .context(
                                        "failed to refresh launcher summary after bundle staging",
                                    )?;
                                } else {
                                    let _ = self.logger.log_line(
                                        "provider automation could not be recorded because no launcher summary is loaded",
                                    );
                                }

                                let mut feedback_message =
                                    format!("Support bundle copied to {}", staged_path.display());
                                if self.providers.has_share_targets() {
                                    if let Some(summary) = stage_report.summary() {
                                        feedback_message.push_str("; ");
                                        feedback_message.push_str(&summary);
                                    }
                                    if let Some(summary) = automation_report.summary() {
                                        feedback_message.push_str("; ");
                                        feedback_message.push_str(&summary);
                                    }
                                } else {
                                    feedback_message
                                        .push_str("; no first-party share providers configured");
                                }

                                let feedback = self.feedback_from_reports(
                                    feedback_message,
                                    &stage_report,
                                    &automation_report,
                                );
                                self.set_feedback(feedback)?;
                            }
                            Err(err) => {
                                self.logger
                                    .log_line(&format!("support bundle copy failed: {err}"))
                                    .context("failed to log copy failure")?;
                                self.set_feedback(ActionFeedback::error(format!(
                                    "Failed to copy support bundle: {err}"
                                )))?;
                            }
                        }
                    }
                    None => {
                        self.logger
                            .log_line("support bundle copy cancelled by user")
                            .context("failed to log copy cancellation")?;
                        self.set_feedback(ActionFeedback::info(
                            "Copy cancelled. No files were staged.",
                        ))?;
                    }
                }
            }
            LauncherShellMessage::RevealPath { path, label } => {
                self.logger
                    .log_line(&format!("revealing {label} at {}", path.display()))?;
                reveal_in_file_manager(&path)
                    .with_context(|| format!("failed to reveal {}", path.display()))?;
            }
            LauncherShellMessage::UploadSupportArtifacts { artifacts } => {
                self.logger
                    .log_line("upload support artifacts requested via launcher UI")
                    .context("failed to log upload request")?;

                if artifacts.is_empty() {
                    self.set_feedback(ActionFeedback::info(
                        "No support artifacts are available yet. Launch the game to generate diagnostics.",
                    ))?;
                    return Ok(());
                }

                let dialog = FileDialog::new()
                    .set_title("Select where to stage support artifacts for upload");
                let dialog = self.configure_dialog_directory(
                    dialog,
                    self.preferences.upload_destination_path(),
                    Some(self.paths.logs_dir()),
                );

                match dialog.pick_folder() {
                    Some(destination) => {
                        self.remember_upload_destination(&destination);
                        match copy_support_artifacts(&artifacts, &destination) {
                            Ok(paths) => {
                                let staged_list = paths
                                    .iter()
                                    .map(|path| path.display().to_string())
                                    .collect::<Vec<_>>()
                                    .join(", ");
                                let mut summary = format!(
                                    "Staged {} artifact{} in {}",
                                    paths.len(),
                                    if paths.len() == 1 { "" } else { "s" },
                                    destination.display()
                                );
                                self.logger
                                    .log_line(&format!("{} ({})", summary, staged_list))
                                    .context("failed to log artifact staging success")?;

                                let state_snapshot = self.ui.state().cloned();
                                let (stage_report, automation_report) =
                                    self.providers.stage_artifacts(&self.logger, &artifacts);
                                let diagnostics = self.update_provider_diagnostics().context(
                                    "failed to refresh provider diagnostics after artifact staging",
                                )?;
                                if let Some(mut state) = state_snapshot {
                                    let snapshot = self
                                        .persist_provider_snapshot(&state, &diagnostics)
                                        .context(
                                            "failed to persist provider automation after artifact staging",
                                        )?;
                                    state.summary.provider_automation = snapshot;
                                    self.ui
                                        .set_state(Some(state))
                                        .map_err(|err| anyhow!(err))
                                        .context(
                                        "failed to refresh launcher summary after artifact staging",
                                    )?;
                                } else {
                                    let _ = self.logger.log_line(
                                        "provider automation could not be recorded because no launcher summary is loaded",
                                    );
                                }

                                if self.providers.has_upload_targets() {
                                    if let Some(extra) = stage_report.summary() {
                                        summary.push_str("; ");
                                        summary.push_str(&extra);
                                    }
                                    if let Some(extra) = automation_report.summary() {
                                        summary.push_str("; ");
                                        summary.push_str(&extra);
                                    }
                                } else {
                                    summary
                                        .push_str("; no first-party upload providers configured");
                                }

                                let feedback = self.feedback_from_reports(
                                    summary,
                                    &stage_report,
                                    &automation_report,
                                );
                                self.set_feedback(feedback)?;
                            }
                            Err(err) => {
                                self.logger
                                    .log_line(&format!("artifact staging failed: {err}"))
                                    .context("failed to log artifact staging failure")?;
                                self.set_feedback(ActionFeedback::error(format!(
                                    "Failed to stage artifacts: {err}"
                                )))?;
                            }
                        }
                    }
                    None => {
                        self.logger
                            .log_line("artifact upload staging cancelled by user")
                            .context("failed to log upload cancellation")?;
                        self.set_feedback(ActionFeedback::info(
                            "Upload cancelled. No files were copied.",
                        ))?;
                    }
                }
            }
        }
        Ok(())
    }

    fn refresh_state(&mut self) -> Result<()> {
        let state = load_shell_state(&self.paths).context("failed to load launcher state")?;
        match state {
            Some(state) => {
                self.providers
                    .hydrate_from_snapshot(&state.logs_dir, &state.summary.provider_automation);
                self.logger
                    .set_target(state.launcher_log_path.clone())
                    .context("failed to attach launcher log to runtime log file")?;
                self.ui
                    .set_state(Some(state))
                    .map_err(|err| anyhow!(err))
                    .context("failed to update launcher UI state")?;
            }
            None => {
                self.providers.reset_automation();
                self.ui
                    .set_state(None)
                    .map_err(|err| anyhow!(err))
                    .context("failed to reset launcher UI state")?;
            }
        }
        self.update_provider_diagnostics()
            .context("failed to refresh provider diagnostics")?;
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

const SHARE_PROVIDERS_ENV: &str = "LC_FIRST_PARTY_SHARE_DIRS";
const UPLOAD_PROVIDERS_ENV: &str = "LC_FIRST_PARTY_UPLOAD_DIRS";

#[derive(Default)]
struct ProviderStageReport {
    successes: Vec<ProviderStageSuccess>,
    failures: Vec<ProviderStageFailure>,
}

impl ProviderStageReport {
    fn summary(&self) -> Option<String> {
        if self.successes.is_empty() && self.failures.is_empty() {
            return None;
        }
        let mut parts = Vec::new();
        if !self.successes.is_empty() {
            let labels = self
                .successes
                .iter()
                .map(|success| success.display_label())
                .collect::<Vec<_>>()
                .join(", ");
            parts.push(format!("staged for {}", labels));
        }
        if !self.failures.is_empty() {
            let labels = self
                .failures
                .iter()
                .map(|failure| failure.display_label())
                .collect::<Vec<_>>()
                .join(", ");
            parts.push(format!("failed for {}", labels));
        }
        Some(parts.join("; "))
    }

    fn successes(&self) -> &[ProviderStageSuccess] {
        &self.successes
    }

    fn failures(&self) -> &[ProviderStageFailure] {
        &self.failures
    }
}

struct ProviderStageSuccess {
    provider: String,
    paths: Vec<PathBuf>,
}

impl ProviderStageSuccess {
    fn display_label(&self) -> String {
        let count = self.paths.len();
        let suffix = if count == 1 { "file" } else { "files" };
        format!("{} ({} {})", self.provider, count, suffix)
    }
}

struct ProviderStageFailure {
    provider: String,
    error: String,
}

impl ProviderStageFailure {
    fn display_label(&self) -> String {
        format!("{} ({})", self.provider, self.error)
    }
}

#[derive(Default)]
struct ProviderAutomationReport {
    successes: Vec<ProviderAutomationSuccess>,
    failures: Vec<ProviderAutomationFailure>,
    skipped: Vec<ProviderAutomationSkip>,
}

impl ProviderAutomationReport {
    fn summary(&self) -> Option<String> {
        if self.successes.is_empty() && self.failures.is_empty() && self.skipped.is_empty() {
            return None;
        }
        let mut parts = Vec::new();
        if !self.successes.is_empty() {
            let labels = self
                .successes
                .iter()
                .map(|success| success.display_label())
                .collect::<Vec<_>>()
                .join(", ");
            parts.push(format!("submission requests prepared for {}", labels));
        }
        if !self.failures.is_empty() {
            let labels = self
                .failures
                .iter()
                .map(|failure| failure.display_label())
                .collect::<Vec<_>>()
                .join(", ");
            parts.push(format!("submission requests failed for {}", labels));
        }
        if !self.skipped.is_empty() {
            let labels = self
                .skipped
                .iter()
                .map(|skip| skip.display_label())
                .collect::<Vec<_>>()
                .join(", ");
            parts.push(format!("submission requests skipped for {}", labels));
        }
        Some(parts.join("; "))
    }

    fn successes(&self) -> &[ProviderAutomationSuccess] {
        &self.successes
    }

    fn failures(&self) -> &[ProviderAutomationFailure] {
        &self.failures
    }

    fn skipped(&self) -> &[ProviderAutomationSkip] {
        &self.skipped
    }
}

struct ProviderAutomationSuccess {
    provider: String,
    detail: String,
}

impl ProviderAutomationSuccess {
    fn display_label(&self) -> String {
        format!("{} ({})", self.provider, self.detail)
    }
}

struct ProviderAutomationFailure {
    provider: String,
    error: String,
}

impl ProviderAutomationFailure {
    fn display_label(&self) -> String {
        format!("{} ({})", self.provider, self.error)
    }
}

struct ProviderAutomationSkip {
    provider: String,
    reason: String,
}

impl ProviderAutomationSkip {
    fn display_label(&self) -> String {
        format!("{} ({})", self.provider, self.reason)
    }
}

#[derive(Clone)]
struct ProviderTarget {
    name: String,
    path: PathBuf,
}

#[derive(Clone)]
struct ProviderTargetState {
    target: ProviderTarget,
    automation: ProviderAutomationState,
}

impl ProviderTargetState {
    fn new(target: ProviderTarget) -> Self {
        Self {
            target,
            automation: ProviderAutomationState::Idle,
        }
    }

    fn to_status(&self) -> ProviderStatus {
        ProviderStatus {
            name: self.target.name.clone(),
            path: self.target.path.clone(),
            path_status: compute_path_status(&self.target.path),
            automation: self.automation.clone(),
        }
    }
}

enum ProviderRole {
    Share,
    Upload,
}

impl ProviderRole {
    fn as_str(&self) -> &'static str {
        match self {
            ProviderRole::Share => "share",
            ProviderRole::Upload => "upload",
        }
    }

    fn file_prefix(&self) -> &'static str {
        match self {
            ProviderRole::Share => "share",
            ProviderRole::Upload => "upload",
        }
    }
}

struct FirstPartyProviders {
    share: Vec<ProviderTargetState>,
    upload: Vec<ProviderTargetState>,
}

impl FirstPartyProviders {
    fn discover(paths: &AppPaths) -> Self {
        let share = Self::parse_targets(
            SHARE_PROVIDERS_ENV,
            "Support Share Drop",
            paths.logs_dir().join("support-share"),
        )
        .into_iter()
        .map(ProviderTargetState::new)
        .collect();
        let upload = Self::parse_targets(
            UPLOAD_PROVIDERS_ENV,
            "Support Upload Drop",
            paths.logs_dir().join("support-upload"),
        )
        .into_iter()
        .map(ProviderTargetState::new)
        .collect();
        Self { share, upload }
    }

    fn parse_targets(
        env_var: &str,
        default_name: &str,
        default_path: PathBuf,
    ) -> Vec<ProviderTarget> {
        let base = default_path.parent().map(|dir| dir.to_path_buf());
        if let Some(raw) = env::var_os(env_var) {
            let mut targets = Vec::new();
            for (index, path_value) in env::split_paths(&raw).enumerate() {
                if path_value.as_os_str().is_empty() {
                    continue;
                }
                let mut path = path_value;
                if path.is_relative() {
                    if let Some(base) = &base {
                        path = base.join(&path);
                    }
                }
                let name = path
                    .file_name()
                    .and_then(|component| component.to_str())
                    .filter(|name| !name.is_empty())
                    .map(|name| name.to_string())
                    .unwrap_or_else(|| format!("{default_name} {}", index + 1));
                targets.push(ProviderTarget { name, path });
            }
            return targets;
        }
        vec![ProviderTarget {
            name: default_name.to_string(),
            path: default_path,
        }]
    }

    fn hydrate_from_snapshot(&mut self, logs_dir: &Path, snapshot: &ProviderAutomationSnapshot) {
        self.reset_automation();
        for record in &snapshot.share {
            let resolved = resolve_snapshot_path(logs_dir, &record.path);
            if let Some(state) = find_state_mut(&mut self.share, |state| {
                paths_equivalent(&state.target.path, &resolved)
            }) {
                Self::apply_snapshot_record(state, record, &resolved, true);
                continue;
            }
            if let Some(state) =
                find_state_mut(&mut self.share, |state| state.target.name == record.name)
            {
                Self::apply_snapshot_record(state, record, &resolved, false);
            }
        }
        for record in &snapshot.upload {
            let resolved = resolve_snapshot_path(logs_dir, &record.path);
            if let Some(state) = find_state_mut(&mut self.upload, |state| {
                paths_equivalent(&state.target.path, &resolved)
            }) {
                Self::apply_snapshot_record(state, record, &resolved, true);
                continue;
            }
            if let Some(state) =
                find_state_mut(&mut self.upload, |state| state.target.name == record.name)
            {
                Self::apply_snapshot_record(state, record, &resolved, false);
            }
        }
    }

    fn apply_snapshot_record(
        state: &mut ProviderTargetState,
        record: &ProviderAutomationRecord,
        resolved: &Path,
        matched_by_path: bool,
    ) {
        if should_mark_submission_as_stale(&record.automation) {
            if !matched_by_path {
                let reason = format!(
                    "staging directory changed from {} to {}; restage files to refresh submissions",
                    resolved.display(),
                    state.target.path.display()
                );
                state.automation = ProviderAutomationState::Stale { reason };
                return;
            }
            let current_status = compute_path_status(&state.target.path);
            if current_status != record.path_status {
                let reason = stale_reason_for_status(&state.target.path, current_status);
                state.automation = ProviderAutomationState::Stale { reason };
                return;
            }
        }
        state.automation = record.automation.clone();
    }

    fn reset_automation(&mut self) {
        for state in &mut self.share {
            state.automation = ProviderAutomationState::Idle;
        }
        for state in &mut self.upload {
            state.automation = ProviderAutomationState::Idle;
        }
    }

    fn stage_bundle(
        &mut self,
        logger: &dyn LauncherLog,
        bundle: &Path,
    ) -> (ProviderStageReport, ProviderAutomationReport) {
        let mut stage_report = ProviderStageReport::default();
        let mut automation_report = ProviderAutomationReport::default();
        for state in &mut self.share {
            let target = &state.target;
            match copy_support_bundle(bundle, &target.path) {
                Ok(path) => {
                    let _ = logger.log_line(&format!(
                        "first-party share staged for {} at {}",
                        target.name,
                        path.display()
                    ));
                    let staged_paths = vec![path];
                    match create_submission_request(
                        logger,
                        target,
                        &staged_paths,
                        ProviderRole::Share,
                    ) {
                        Ok(detail) => {
                            automation_report.successes.push(ProviderAutomationSuccess {
                                provider: target.name.clone(),
                                detail: detail.clone(),
                            });
                            state.automation = ProviderAutomationState::Submitted { detail };
                        }
                        Err(err) => {
                            let message = err.to_string();
                            let _ = logger.log_line(&format!(
                                "failed to prepare submission request for {}: {message}",
                                target.name
                            ));
                            automation_report.failures.push(ProviderAutomationFailure {
                                provider: target.name.clone(),
                                error: message.clone(),
                            });
                            state.automation = ProviderAutomationState::Failed { error: message };
                        }
                    }
                    stage_report.successes.push(ProviderStageSuccess {
                        provider: target.name.clone(),
                        paths: staged_paths,
                    });
                }
                Err(err) => {
                    let message = err.to_string();
                    let _ = logger.log_line(&format!(
                        "failed to stage support bundle for {}: {message}",
                        target.name
                    ));
                    stage_report.failures.push(ProviderStageFailure {
                        provider: target.name.clone(),
                        error: message.clone(),
                    });
                    let reason = format!("staging failed: {message}");
                    automation_report.skipped.push(ProviderAutomationSkip {
                        provider: target.name.clone(),
                        reason: reason.clone(),
                    });
                    state.automation = ProviderAutomationState::Skipped { reason };
                }
            }
        }
        (stage_report, automation_report)
    }

    fn stage_artifacts(
        &mut self,
        logger: &dyn LauncherLog,
        artifacts: &[SupportArtifact],
    ) -> (ProviderStageReport, ProviderAutomationReport) {
        let mut stage_report = ProviderStageReport::default();
        let mut automation_report = ProviderAutomationReport::default();
        for state in &mut self.upload {
            let target = &state.target;
            match copy_support_artifacts(artifacts, &target.path) {
                Ok(paths) => {
                    let staged = paths
                        .iter()
                        .map(|path| path.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ");
                    let _ = logger.log_line(&format!(
                        "first-party upload staged for {} at {}",
                        target.name, staged
                    ));
                    match create_submission_request(logger, target, &paths, ProviderRole::Upload) {
                        Ok(detail) => {
                            automation_report.successes.push(ProviderAutomationSuccess {
                                provider: target.name.clone(),
                                detail: detail.clone(),
                            });
                            state.automation = ProviderAutomationState::Submitted { detail };
                        }
                        Err(err) => {
                            let message = err.to_string();
                            let _ = logger.log_line(&format!(
                                "failed to prepare submission request for {}: {message}",
                                target.name
                            ));
                            automation_report.failures.push(ProviderAutomationFailure {
                                provider: target.name.clone(),
                                error: message.clone(),
                            });
                            state.automation = ProviderAutomationState::Failed { error: message };
                        }
                    }
                    stage_report.successes.push(ProviderStageSuccess {
                        provider: target.name.clone(),
                        paths,
                    });
                }
                Err(err) => {
                    let message = err.to_string();
                    let _ = logger.log_line(&format!(
                        "failed to stage support artifacts for {}: {message}",
                        target.name
                    ));
                    stage_report.failures.push(ProviderStageFailure {
                        provider: target.name.clone(),
                        error: message.clone(),
                    });
                    let reason = format!("staging failed: {message}");
                    automation_report.skipped.push(ProviderAutomationSkip {
                        provider: target.name.clone(),
                        reason: reason.clone(),
                    });
                    state.automation = ProviderAutomationState::Skipped { reason };
                }
            }
        }
        (stage_report, automation_report)
    }

    fn has_share_targets(&self) -> bool {
        !self.share.is_empty()
    }

    fn has_upload_targets(&self) -> bool {
        !self.upload.is_empty()
    }

    fn diagnostics(&self) -> ProviderDiagnostics {
        ProviderDiagnostics {
            share: self.share.iter().map(|state| state.to_status()).collect(),
            upload: self.upload.iter().map(|state| state.to_status()).collect(),
        }
    }
}

fn compute_path_status(path: &Path) -> ProviderPathStatus {
    match fs::metadata(path) {
        Ok(metadata) => {
            if metadata.is_dir() {
                ProviderPathStatus::Ready
            } else {
                ProviderPathStatus::NotDirectory
            }
        }
        Err(err) => match err.kind() {
            ErrorKind::NotFound => ProviderPathStatus::Missing,
            _ => ProviderPathStatus::Inaccessible(err.to_string()),
        },
    }
}

fn should_mark_submission_as_stale(state: &ProviderAutomationState) -> bool {
    matches!(state, ProviderAutomationState::Submitted { .. })
}

fn stale_reason_for_status(path: &Path, status: ProviderPathStatus) -> String {
    match status {
        ProviderPathStatus::Ready => format!(
            "staging directory {} was recreated; restage files to refresh submissions",
            path.display()
        ),
        ProviderPathStatus::Missing => format!(
            "staging directory {} is missing; restage files to refresh submissions",
            path.display()
        ),
        ProviderPathStatus::NotDirectory => format!(
            "staging directory {} is not a directory; restage files to refresh submissions",
            path.display()
        ),
        ProviderPathStatus::Inaccessible(err) => format!(
            "staging directory {} is inaccessible ({}); restage files to refresh submissions",
            path.display(),
            err
        ),
    }
}

fn resolve_snapshot_path(logs_dir: &Path, entry: &str) -> PathBuf {
    let candidate = PathBuf::from(entry);
    if candidate.is_absolute() {
        candidate
    } else {
        logs_dir.join(candidate)
    }
}

fn find_state_mut<'a, F>(
    states: &'a mut [ProviderTargetState],
    mut predicate: F,
) -> Option<&'a mut ProviderTargetState>
where
    F: FnMut(&ProviderTargetState) -> bool,
{
    for state in states {
        if predicate(state) {
            return Some(state);
        }
    }
    None
}

fn paths_equivalent(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

fn relative_display(base: &Path, path: &Path) -> String {
    match path.strip_prefix(base) {
        Ok(relative) => relative.display().to_string(),
        Err(_) => path.display().to_string(),
    }
}

fn create_submission_request(
    logger: &dyn LauncherLog,
    target: &ProviderTarget,
    staged_paths: &[PathBuf],
    role: ProviderRole,
) -> Result<String> {
    if staged_paths.is_empty() {
        return Err(anyhow!(
            "no staged files were provided for {} submission request",
            target.name
        ));
    }

    fs::create_dir_all(&target.path).with_context(|| {
        format!(
            "failed to prepare submission directory {}",
            target.path.display()
        )
    })?;

    let files = staged_paths
        .iter()
        .map(|path| relative_display(&target.path, path))
        .collect::<Vec<_>>();

    let request = SubmissionRequest {
        provider: &target.name,
        role: role.as_str(),
        generated_at: timestamp_for_log(),
        files,
    };

    let payload = serde_json::to_vec_pretty(&request)
        .context("failed to serialize submission request payload")?;
    let file_name = format!(
        "submission-request-{}-{}.json",
        role.file_prefix(),
        timestamp_for_filename()
    );
    let destination = target.path.join(&file_name);
    fs::write(&destination, payload).with_context(|| {
        format!(
            "failed to write submission request {}",
            destination.display()
        )
    })?;

    logger
        .log_line(&format!(
            "submission request prepared for {} at {}",
            target.name,
            destination.display()
        ))
        .context("failed to record submission request creation")?;

    Ok(file_name)
}

#[derive(Serialize)]
struct SubmissionRequest<'a> {
    provider: &'a str,
    role: &'a str,
    generated_at: String,
    files: Vec<String>,
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

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result as AnyResult;
    use lc_launcher::{
        ProviderAutomationSnapshot, ProviderAutomationState, ProviderDiagnostics,
        ProviderPathStatus, ProviderStatus, SupportArtifact,
    };
    use std::env;
    use std::ffi::OsString;
    use std::fs;
    use std::sync::{Mutex, OnceLock};
    use tempfile::TempDir;

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    struct DummyLog;
    impl LauncherLog for DummyLog {
        fn log_line(&self, _message: &str) -> AnyResult<()> {
            Ok(())
        }
    }

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: Option<&str>) -> Self {
            let previous = env::var_os(key);
            match value {
                Some(val) => env::set_var(key, val),
                None => env::remove_var(key),
            }
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(value) = self.previous.take() {
                env::set_var(self.key, value);
            } else {
                env::remove_var(self.key);
            }
        }
    }

    #[test]
    fn parse_targets_uses_default_when_env_unset() {
        let _env_guard = env_lock().lock().unwrap();
        let base = TempDir::new().unwrap();
        let default_path = base.path().join("support-share");
        let _guard = EnvVarGuard::set(SHARE_PROVIDERS_ENV, None);
        let targets = FirstPartyProviders::parse_targets(
            SHARE_PROVIDERS_ENV,
            "Support Share Drop",
            default_path.clone(),
        );
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].name, "Support Share Drop");
        assert_eq!(targets[0].path, default_path);
    }

    #[test]
    fn parse_targets_resolves_relative_env_paths() {
        let _env_guard = env_lock().lock().unwrap();
        let base = TempDir::new().unwrap();
        let default_path = base.path().join("support-share");
        let relative = format!("custom{}drop", std::path::MAIN_SEPARATOR);
        let _guard = EnvVarGuard::set(SHARE_PROVIDERS_ENV, Some(&relative));
        let targets = FirstPartyProviders::parse_targets(
            SHARE_PROVIDERS_ENV,
            "Support Share Drop",
            default_path.clone(),
        );
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].name, "drop");
        assert_eq!(targets[0].path, base.path().join("custom").join("drop"));
    }

    #[test]
    fn stage_bundle_reports_success() {
        let provider_dir = TempDir::new().unwrap();
        let source_dir = TempDir::new().unwrap();
        let bundle_path = source_dir.path().join("bundle.zip");
        fs::write(&bundle_path, b"bundle").unwrap();

        let mut providers = FirstPartyProviders {
            share: vec![ProviderTargetState::new(ProviderTarget {
                name: "Support Share".into(),
                path: provider_dir.path().to_path_buf(),
            })],
            upload: Vec::new(),
        };

        let logger = DummyLog;
        let (stage_report, automation_report) = providers.stage_bundle(&logger, &bundle_path);
        assert!(stage_report.failures().is_empty());
        assert_eq!(stage_report.successes().len(), 1);
        let staged_path = stage_report.successes()[0].paths[0].clone();
        assert!(staged_path.exists());
        assert_ne!(staged_path, bundle_path);
        let contents = fs::read(staged_path).unwrap();
        assert_eq!(contents, b"bundle");

        assert!(automation_report.failures().is_empty());
        assert!(automation_report.skipped().is_empty());
        assert_eq!(automation_report.successes().len(), 1);
        let detail = &automation_report.successes()[0].detail;
        assert!(detail.starts_with("submission-request-share-"));
        let request_path = provider_dir.path().join(detail);
        assert!(request_path.exists());
    }

    #[test]
    fn stage_artifacts_reports_success() {
        let provider_dir = TempDir::new().unwrap();
        let source_dir = TempDir::new().unwrap();
        let summary_path = source_dir.path().join("launcher-summary.json");
        fs::write(&summary_path, b"{}").unwrap();

        let artifact = SupportArtifact {
            path: summary_path.clone(),
            role: "summary",
        };

        let mut providers = FirstPartyProviders {
            share: Vec::new(),
            upload: vec![ProviderTargetState::new(ProviderTarget {
                name: "Support Upload".into(),
                path: provider_dir.path().to_path_buf(),
            })],
        };

        let logger = DummyLog;
        let artifacts = vec![artifact];
        let (stage_report, automation_report) = providers.stage_artifacts(&logger, &artifacts);
        assert!(stage_report.failures().is_empty());
        assert_eq!(stage_report.successes().len(), 1);
        let staged_paths = &stage_report.successes()[0].paths;
        assert_eq!(staged_paths.len(), artifacts.len());
        for path in staged_paths {
            assert!(path.exists());
        }

        assert!(automation_report.failures().is_empty());
        assert!(automation_report.skipped().is_empty());
        assert_eq!(automation_report.successes().len(), 1);
        let detail = &automation_report.successes()[0].detail;
        assert!(detail.starts_with("submission-request-upload-"));
        let request_path = provider_dir.path().join(detail);
        assert!(request_path.exists());
    }

    #[test]
    fn hydrate_marks_submitted_share_provider_stale_when_path_missing() {
        let logs_dir = TempDir::new().unwrap();
        let share_path = logs_dir.path().join("support-share");
        fs::create_dir_all(&share_path).unwrap();

        let mut providers = FirstPartyProviders {
            share: vec![ProviderTargetState::new(ProviderTarget {
                name: "Support Share Drop".into(),
                path: share_path.clone(),
            })],
            upload: Vec::new(),
        };

        let diagnostics = ProviderDiagnostics {
            share: vec![ProviderStatus {
                name: "Support Share Drop".into(),
                path: share_path.clone(),
                path_status: ProviderPathStatus::Ready,
                automation: ProviderAutomationState::Submitted {
                    detail: "submission-request-share-123.json".into(),
                },
            }],
            upload: Vec::new(),
        };

        let snapshot = ProviderAutomationSnapshot::from_diagnostics(&diagnostics, logs_dir.path());
        fs::remove_dir_all(&share_path).unwrap();

        providers.hydrate_from_snapshot(logs_dir.path(), &snapshot);

        match &providers.share[0].automation {
            ProviderAutomationState::Stale { reason } => {
                assert!(
                    reason.contains("missing"),
                    "expected missing path reason, got {reason}"
                );
            }
            state => panic!("expected stale automation state, got {state:?}"),
        }
    }

    #[test]
    fn hydrate_marks_submitted_share_provider_stale_when_path_changes() {
        let logs_dir = TempDir::new().unwrap();
        let original_path = logs_dir.path().join("support-share-old");
        let new_path = logs_dir.path().join("support-share-new");
        fs::create_dir_all(&original_path).unwrap();
        fs::create_dir_all(&new_path).unwrap();

        let mut providers = FirstPartyProviders {
            share: vec![ProviderTargetState::new(ProviderTarget {
                name: "Support Share Drop".into(),
                path: new_path.clone(),
            })],
            upload: Vec::new(),
        };

        let diagnostics = ProviderDiagnostics {
            share: vec![ProviderStatus {
                name: "Support Share Drop".into(),
                path: original_path.clone(),
                path_status: ProviderPathStatus::Ready,
                automation: ProviderAutomationState::Submitted {
                    detail: "submission-request-share-456.json".into(),
                },
            }],
            upload: Vec::new(),
        };

        let snapshot = ProviderAutomationSnapshot::from_diagnostics(&diagnostics, logs_dir.path());

        providers.hydrate_from_snapshot(logs_dir.path(), &snapshot);

        match &providers.share[0].automation {
            ProviderAutomationState::Stale { reason } => {
                assert!(
                    reason.contains(original_path.to_str().unwrap()),
                    "expected stale reason to mention original path, got {reason}"
                );
                assert!(
                    reason.contains(new_path.to_str().unwrap()),
                    "expected stale reason to mention new path, got {reason}"
                );
            }
            state => panic!("expected stale automation state, got {state:?}"),
        }
    }

    #[test]
    fn hydrate_from_snapshot_restores_automation_state() {
        let logs_dir = TempDir::new().unwrap();
        let share_path = logs_dir.path().join("support-share");
        let upload_path = logs_dir.path().join("support-upload");
        fs::create_dir_all(&share_path).unwrap();
        fs::create_dir_all(&upload_path).unwrap();

        let mut providers = FirstPartyProviders {
            share: vec![ProviderTargetState::new(ProviderTarget {
                name: "Support Share Drop".into(),
                path: share_path.clone(),
            })],
            upload: vec![ProviderTargetState::new(ProviderTarget {
                name: "Support Upload Drop".into(),
                path: upload_path.clone(),
            })],
        };

        let diagnostics = ProviderDiagnostics {
            share: vec![ProviderStatus {
                name: "Support Share Drop".into(),
                path: share_path.clone(),
                path_status: ProviderPathStatus::Ready,
                automation: ProviderAutomationState::Submitted {
                    detail: "submission-request-share-123.json".into(),
                },
            }],
            upload: vec![ProviderStatus {
                name: "Support Upload Drop".into(),
                path: upload_path.clone(),
                path_status: ProviderPathStatus::Ready,
                automation: ProviderAutomationState::Failed {
                    error: "network failure".into(),
                },
            }],
        };

        let snapshot = ProviderAutomationSnapshot::from_diagnostics(&diagnostics, logs_dir.path());
        providers.reset_automation();
        providers.hydrate_from_snapshot(logs_dir.path(), &snapshot);

        assert!(matches!(
            providers.share[0].automation,
            ProviderAutomationState::Submitted { .. }
        ));
        assert!(matches!(
            providers.upload[0].automation,
            ProviderAutomationState::Failed { .. }
        ));
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
