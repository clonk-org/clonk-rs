use std::env;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, LineWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use arboard::Clipboard;
use clonk_graphics::{BitmapFont, Color, PixelFormat, Point, Surface, TextFont, TrueTypeFont};
use clonk_gui::{
    DrawCommand, GuiEvent, ImageData, KeyCode, Point as GuiPoint, Rect as GuiRect, Size as GuiSize,
};
use clonk_launcher::{
    copy_support_artifacts, copy_support_bundle, ensure_support_bundle, load_launcher_preferences,
    load_localization, load_shell_state, render_support_bundle_report, reveal_in_file_manager,
    save_launcher_preferences, support_artifacts, timestamp_for_filename, timestamp_for_log,
    write_launcher_summary, LauncherLog, LauncherPreferences, LauncherShellState,
    ProviderAutomationRecord, ProviderAutomationSnapshot, ProviderAutomationState,
    ProviderBulkRetargetRecord, ProviderBulkRetargetSummary, ProviderDiagnostics,
    ProviderOverrideSource, ProviderOverrideSourceRecord, ProviderPathProvenance,
    ProviderPathStatus, ProviderStatus, ReportSearchHighlightPreference, ReportSearchPreferences,
    SupportArtifact, UpdateTelemetrySummary,
};
use clonk_launcher_ui::{
    ActionFeedback, LauncherShellMessage, LauncherShellResponse, LauncherShellUi, ProviderKind,
    ReportSearchHighlight, ReportSearchPreset, ReportSearchState,
};
use clonk_platform::AppPaths;
use pixels::{Pixels, SurfaceTexture};
use rfd::FileDialog;
use serde::Serialize;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::{ElementState, Ime, KeyEvent, MouseButton, TouchPhase, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Window, WindowId};

fn main() -> Result<()> {
    clonk_logging::init();
    clonk_logging::install_panic_hook();

    let event_loop = EventLoop::new().context("failed to create launcher event loop")?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut shell = LauncherShell::default();
    event_loop
        .run_app(&mut shell)
        .context("launcher event loop failed")?;
    shell.finish()
}

#[derive(Default)]
struct LauncherShell {
    runtime: Option<LauncherRuntime>,
    fatal_error: Option<anyhow::Error>,
}

impl LauncherShell {
    fn initialize(event_loop: &ActiveEventLoop) -> Result<LauncherRuntime> {
        let attributes = Window::default_attributes()
            .with_title("Clonk Rust Launcher")
            // The launcher is a product window like any other; without this it
            // carried whatever default the platform hands an iconless window.
            .with_window_icon(window_icon())
            .with_inner_size(LogicalSize::new(960.0, 640.0));
        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .context("failed to create launcher window")?,
        );

        let size = window.inner_size();
        let (initial_width, initial_height) = enforce_min_size(size);
        let surface_texture =
            SurfaceTexture::new(initial_width, initial_height, Arc::clone(&window));
        let pixels: Pixels<'static> = Pixels::new(initial_width, initial_height, surface_texture)
            .context("failed to create pixel framebuffer")?;
        let app = LauncherApp::new(&window).context("failed to initialise launcher shell")?;

        let mut runtime = LauncherRuntime {
            window_focused: window.has_focus(),
            window,
            pixels: Some(pixels),
            app,
            ime_allowed: false,
            surface_rebuild: SurfaceRebuildState::default(),
            surface_retry_at: None,
        };
        runtime.sync_report_search_ime();
        Ok(runtime)
    }

    fn exit_after_initialization_error(
        &mut self,
        event_loop: &ActiveEventLoop,
        error: anyhow::Error,
    ) {
        tracing::error!(error = ?error, "failed to initialize launcher shell");
        self.fatal_error.get_or_insert(error);
        event_loop.exit();
    }

    fn finish(self) -> Result<()> {
        self.fatal_error.map_or(Ok(()), Err)
    }
}

impl ApplicationHandler for LauncherShell {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.runtime.is_some() || self.fatal_error.is_some() {
            return;
        }
        match Self::initialize(event_loop) {
            Ok(runtime) => self.runtime = Some(runtime),
            Err(error) => self.exit_after_initialization_error(event_loop, error),
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(runtime) = self
            .runtime
            .as_mut()
            .filter(|runtime| runtime.window.id() == window_id)
        else {
            return;
        };
        if let Err(error) = runtime.handle_window_event(event_loop, event) {
            tracing::error!(error = ?error, "launcher shell encountered an error");
            self.fatal_error.get_or_insert(error);
            event_loop.exit();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(runtime) = self.runtime.as_mut() {
            match launcher_redraw_action(Instant::now(), runtime.surface_retry_at) {
                LauncherRedrawAction::Request => {
                    runtime.surface_retry_at = None;
                    event_loop.set_control_flow(ControlFlow::Wait);
                    runtime.window.request_redraw();
                }
                LauncherRedrawAction::WaitUntil(retry_at) => {
                    event_loop.set_control_flow(ControlFlow::WaitUntil(retry_at));
                }
            }
        }
    }
}

struct LauncherRuntime {
    window: Arc<Window>,
    pixels: Option<Pixels<'static>>,
    app: LauncherApp,
    window_focused: bool,
    ime_allowed: bool,
    surface_rebuild: SurfaceRebuildState,
    surface_retry_at: Option<Instant>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LauncherPresentRecovery {
    RebuildFramebuffer,
    Report,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LauncherPresentOutcome {
    Presented,
    Skipped,
}

const fn launcher_present_outcome(render_callback_invoked: bool) -> LauncherPresentOutcome {
    if render_callback_invoked {
        LauncherPresentOutcome::Presented
    } else {
        LauncherPresentOutcome::Skipped
    }
}

fn present_launcher_frame(
    pixels: &Pixels<'_>,
) -> std::result::Result<LauncherPresentOutcome, pixels::Error> {
    let mut render_callback_invoked = false;
    pixels.render_with(|encoder, surface_view, context| {
        render_callback_invoked = true;
        context.scaling_renderer.render(encoder, surface_view);
        Ok(())
    })?;
    Ok(launcher_present_outcome(render_callback_invoked))
}

fn launcher_present_recovery(error: &pixels::Error) -> LauncherPresentRecovery {
    if matches!(error, pixels::Error::SurfaceLost) {
        LauncherPresentRecovery::RebuildFramebuffer
    } else {
        LauncherPresentRecovery::Report
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SurfaceRebuildSchedule {
    Immediate,
    Cadenced,
}

#[derive(Default)]
struct SurfaceRebuildState {
    prompt_spent: bool,
}

impl SurfaceRebuildState {
    fn note_loss(&mut self) -> SurfaceRebuildSchedule {
        if self.prompt_spent {
            SurfaceRebuildSchedule::Cadenced
        } else {
            self.prompt_spent = true;
            SurfaceRebuildSchedule::Immediate
        }
    }

    fn note_presented(&mut self) {
        self.prompt_spent = false;
    }
}

const LOST_SURFACE_RETRY_DELAY: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LauncherRedrawAction {
    Request,
    WaitUntil(Instant),
}

fn launcher_redraw_action(now: Instant, surface_retry_at: Option<Instant>) -> LauncherRedrawAction {
    surface_retry_at.filter(|retry_at| *retry_at > now).map_or(
        LauncherRedrawAction::Request,
        LauncherRedrawAction::WaitUntil,
    )
}

fn replace_after_drop<T, E>(
    current: &mut Option<T>,
    build_replacement: impl FnOnce() -> std::result::Result<T, E>,
) -> std::result::Result<(), E> {
    drop(current.take());
    *current = Some(build_replacement()?);
    Ok(())
}

impl LauncherRuntime {
    fn handle_window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        event: WindowEvent,
    ) -> Result<()> {
        if event == WindowEvent::RedrawRequested {
            let pixels = self
                .pixels
                .as_mut()
                .context("launcher framebuffer is unavailable")?;
            self.app
                .render(pixels.frame_mut())
                .context("failed to render launcher UI")?;
            return match present_launcher_frame(pixels) {
                Ok(LauncherPresentOutcome::Presented) => {
                    self.surface_rebuild.note_presented();
                    self.surface_retry_at = None;
                    Ok(())
                }
                Ok(LauncherPresentOutcome::Skipped) => {
                    self.surface_retry_at = Some(Instant::now() + LOST_SURFACE_RETRY_DELAY);
                    Ok(())
                }
                Err(error)
                    if launcher_present_recovery(&error)
                        == LauncherPresentRecovery::RebuildFramebuffer =>
                {
                    let schedule = self.surface_rebuild.note_loss();
                    tracing::warn!("launcher surface was lost; rebuilding its framebuffer");
                    self.rebuild_framebuffer()?;
                    match schedule {
                        SurfaceRebuildSchedule::Immediate => self.window.request_redraw(),
                        SurfaceRebuildSchedule::Cadenced => {
                            self.surface_retry_at = Some(Instant::now() + LOST_SURFACE_RETRY_DELAY);
                        }
                    }
                    Ok(())
                }
                Err(error) => Err(error).context("failed to swap launcher buffers"),
            };
        }
        if let WindowEvent::Focused(focused) = &event {
            self.window_focused = *focused;
        }
        let result = self.pixels.as_mut().map_or_else(
            || Err(anyhow!("launcher framebuffer is unavailable")),
            |pixels| self.app.handle_window_event(pixels, event, event_loop),
        );
        self.sync_report_search_ime();
        result
    }

    fn rebuild_framebuffer(&mut self) -> Result<()> {
        let prior_frame = self
            .pixels
            .as_ref()
            .context("launcher framebuffer is unavailable")?
            .frame()
            .to_vec();
        let (surface_width, surface_height) = enforce_min_size(self.window.inner_size());
        let buffer_width = self.app.surface.width().max(1);
        let buffer_height = self.app.surface.height().max(1);
        replace_after_drop(&mut self.pixels, || {
            let surface_texture =
                SurfaceTexture::new(surface_width, surface_height, Arc::clone(&self.window));
            let mut replacement = Pixels::new(buffer_width, buffer_height, surface_texture)
                .context("failed to rebuild launcher framebuffer")?;
            if replacement.frame().len() == prior_frame.len() {
                replacement.frame_mut().copy_from_slice(&prior_frame);
            }
            Ok(replacement)
        })
    }

    fn sync_report_search_ime(&mut self) {
        let allowed =
            should_enable_report_search_ime(self.window_focused, self.app.report_search.editing());
        if allowed != self.ime_allowed {
            self.window.set_ime_allowed(allowed);
            self.ime_allowed = allowed;
        }
    }
}

/// Side length of the launcher's window icon, matching the game window's
/// title-bar slot (`clonk-app`'s `window_icon`).
const WINDOW_ICON_SIDE: u32 = 64;

/// The product icon, or `None` to leave the platform default in place.
fn window_icon() -> Option<winit::window::Icon> {
    let square = clonk_icon::square_source(clonk_icon::LOGO_PNG)?;
    let icon = clonk_icon::resize_square(&square, WINDOW_ICON_SIDE);
    winit::window::Icon::from_rgba(icon.into_raw(), WINDOW_ICON_SIDE, WINDOW_ICON_SIDE).ok()
}

fn enforce_min_size(size: PhysicalSize<u32>) -> (u32, u32) {
    let width = size.width.max(1);
    let height = size.height.max(1);
    (width, height)
}

fn should_enable_report_search_ime(window_focused: bool, report_search_editing: bool) -> bool {
    window_focused && report_search_editing
}

/// Accept composed text while excluding shortcut keystrokes. Alt is retained
/// because Option and AltGr composition legitimately carry it; AltGr also
/// carries Control. IME commits bypass this keyboard-only policy.
fn report_search_key_text_allowed(modifiers: ModifiersState) -> bool {
    !modifiers.contains(ModifiersState::SUPER)
        && (!modifiers.contains(ModifiersState::CONTROL) || modifiers.contains(ModifiersState::ALT))
}

/// A scale factor that can safely divide layout geometry.
///
/// winit only ever reports a positive, finite factor, but a hostile compositor
/// or a headless stub must not be able to collapse the layout box to zero.
fn normalize_scale(scale_factor: f32) -> f32 {
    if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    }
}

/// The logical layout box for a framebuffer of `physical_width` x
/// `physical_height` device pixels.
///
/// The pixel buffer stays at the window's physical extent so the rasteriser
/// paints at native resolution, but layout and hit-testing run in logical
/// units; otherwise a 2x display lays the whole UI out at half its size.
fn logical_extent(physical_width: u32, physical_height: u32, scale_factor: f32) -> (f32, f32) {
    let scale = normalize_scale(scale_factor);
    (
        (physical_width as f32 / scale).max(1.0),
        (physical_height as f32 / scale).max(1.0),
    )
}

/// Maps a winit physical pointer position into the logical space the GUI was
/// laid out in, so hit-testing agrees with what `scaled_rect` painted.
fn physical_to_logical_point(x: f64, y: f64, scale_factor: f32) -> GuiPoint {
    let scale = normalize_scale(scale_factor);
    GuiPoint::new(x as f32 / scale, y as f32 / scale)
}

/// The inverse mapping: logical layout geometry to the physical pixels the CPU
/// rasteriser writes.
fn scaled_rect(rect: &GuiRect, scale_factor: f32) -> GuiRect {
    let scale = normalize_scale(scale_factor);
    GuiRect::new(
        rect.origin.x * scale,
        rect.origin.y * scale,
        rect.size.width * scale,
        rect.size.height * scale,
    )
}

/// The launcher's text face.
///
/// `planet/System.c4g` ships as an unpacked directory in both the repo and the
/// packaged install (`xtask` copies the tracked tree verbatim), so the shell
/// can read the shipped Endeavour face directly and render real anti-aliased
/// vector text. The `font8x8` bitmap face remains the fallback for installs
/// whose system group is missing or unreadable.
fn load_ui_font(paths: &AppPaths) -> Arc<dyn TextFont> {
    let path = paths.system_group_path().join("Endeavour.ttf");
    fs::read(&path)
        .map_err(|err| err.to_string())
        .and_then(|bytes| TrueTypeFont::from_bytes(Arc::from(bytes)).map_err(|err| err.to_string()))
        .map(|font| Arc::new(font) as Arc<dyn TextFont>)
        .unwrap_or_else(|err| {
            tracing::warn!(
                path = %path.display(),
                error = %err,
                "falling back to the bitmap launcher font"
            );
            Arc::new(BitmapFont::new())
        })
}

struct ReportSearchController {
    query: String,
    normalized_query: String,
    highlight: ReportSearchHighlight,
    matches: Vec<usize>,
    active_index: Option<usize>,
    editing: bool,
}

impl Default for ReportSearchController {
    fn default() -> Self {
        Self {
            query: String::new(),
            normalized_query: String::new(),
            highlight: ReportSearchHighlight::Generic,
            matches: Vec::new(),
            active_index: None,
            editing: false,
        }
    }
}

fn highlight_to_preference(highlight: ReportSearchHighlight) -> ReportSearchHighlightPreference {
    match highlight {
        ReportSearchHighlight::Generic => ReportSearchHighlightPreference::Generic,
        ReportSearchHighlight::Error => ReportSearchHighlightPreference::Error,
        ReportSearchHighlight::Warning => ReportSearchHighlightPreference::Warning,
    }
}

fn preference_to_highlight(preference: ReportSearchHighlightPreference) -> ReportSearchHighlight {
    match preference {
        ReportSearchHighlightPreference::Generic => ReportSearchHighlight::Generic,
        ReportSearchHighlightPreference::Error => ReportSearchHighlight::Error,
        ReportSearchHighlightPreference::Warning => ReportSearchHighlight::Warning,
    }
}

impl ReportSearchController {
    fn is_active(&self) -> bool {
        !self.query.is_empty()
    }

    fn editing(&self) -> bool {
        self.editing
    }

    fn set_editing(&mut self, editing: bool) {
        self.editing = editing;
    }

    fn clear(&mut self) {
        self.query.clear();
        self.normalized_query.clear();
        self.highlight = ReportSearchHighlight::Generic;
        self.matches.clear();
        self.active_index = None;
        self.editing = false;
    }

    fn apply_preset(&mut self, preset: ReportSearchPreset, lines: &[String]) {
        let (query, highlight) = match preset {
            ReportSearchPreset::Errors => ("error", ReportSearchHighlight::Error),
            ReportSearchPreset::Warnings => ("warning", ReportSearchHighlight::Warning),
        };
        self.query = query.into();
        self.normalized_query = self.query.to_lowercase();
        self.highlight = highlight;
        self.recompute_after_query_change(lines);
        self.editing = false;
    }

    fn append_char(&mut self, ch: char, lines: &[String]) {
        self.query.push(ch);
        self.normalized_query = self.query.to_lowercase();
        self.highlight = ReportSearchHighlight::Generic;
        self.recompute_after_query_change(lines);
    }

    fn backspace(&mut self, lines: &[String]) {
        if self.query.pop().is_some() {
            self.normalized_query = self.query.to_lowercase();
        } else {
            self.normalized_query.clear();
        }
        self.highlight = ReportSearchHighlight::Generic;
        self.recompute_after_query_change(lines);
    }

    fn refresh_for_lines(&mut self, lines: &[String]) {
        if !self.editing && !self.is_active() {
            self.matches.clear();
            self.active_index = None;
            return;
        }
        self.recompute_matches(lines, RecomputeMode::Preserve);
    }

    fn recompute_after_query_change(&mut self, lines: &[String]) {
        if !self.has_search_term() {
            self.matches.clear();
            self.active_index = None;
            return;
        }
        self.recompute_matches(lines, RecomputeMode::Reset);
    }

    fn next(&mut self) -> Option<usize> {
        if self.matches.is_empty() {
            self.active_index = None;
            return None;
        }
        let next_index = match self.active_index {
            Some(index) => (index + 1) % self.matches.len(),
            None => 0,
        };
        self.active_index = Some(next_index);
        self.matches.get(next_index).copied()
    }

    fn previous(&mut self) -> Option<usize> {
        if self.matches.is_empty() {
            self.active_index = None;
            return None;
        }
        let previous_index = match self.active_index {
            Some(0) | None => self.matches.len().saturating_sub(1),
            Some(index) => index - 1,
        };
        self.active_index = Some(previous_index);
        self.matches.get(previous_index).copied()
    }

    fn active_line(&self) -> Option<usize> {
        self.active_index
            .and_then(|index| self.matches.get(index).copied())
    }

    fn match_count(&self) -> usize {
        self.matches.len()
    }

    fn has_matches(&self) -> bool {
        !self.matches.is_empty()
    }

    fn persisted_preferences(&self) -> Option<ReportSearchPreferences> {
        if !self.has_search_term() {
            return None;
        }
        Some(ReportSearchPreferences {
            query: self.query.clone(),
            highlight: highlight_to_preference(self.highlight),
            active_line: self.active_line(),
        })
    }

    fn restore_from_preferences(
        &mut self,
        preferences: &ReportSearchPreferences,
        lines: &[String],
    ) {
        self.query = preferences.query.clone();
        self.normalized_query = self.query.to_lowercase();
        self.highlight = preference_to_highlight(preferences.highlight);
        self.editing = false;
        self.recompute_matches(lines, RecomputeMode::Reset);
        if let Some(line) = preferences.active_line {
            if let Ok(index) = self.matches.binary_search(&line) {
                self.active_index = Some(index);
            }
        }
    }

    fn export_annotations(&self, lines: &[String]) -> Option<Vec<String>> {
        if !self.has_search_term() {
            return None;
        }

        let mut annotations = Vec::new();
        annotations.push(String::new());
        let label = self.highlight.english_label();
        let query_display = if self.highlight == ReportSearchHighlight::Generic {
            if self.query.is_empty() {
                "<none>".into()
            } else {
                format!("\"{}\"", self.query)
            }
        } else {
            format!("\"{}\"", self.query)
        };

        if self.matches.is_empty() {
            annotations.push(format!(
                "Search ({label}): {query_display} — no matches found."
            ));
            return Some(annotations);
        }

        annotations.push(format!(
            "Search ({label}): {query_display} — {} match(es).",
            self.matches.len()
        ));
        for (index, line_index) in self.matches.iter().enumerate() {
            let line_number = *line_index + 1;
            let prefix = if Some(index) == self.active_index {
                "  *"
            } else {
                "   "
            };
            let line_text = lines
                .get(*line_index)
                .map(|line| line.as_str())
                .unwrap_or("<line unavailable>");
            annotations.push(format!("{prefix} Line {line_number}: {line_text}"));
        }

        Some(annotations)
    }

    fn ui_state(&self) -> Option<ReportSearchState> {
        if !self.editing && !self.is_active() {
            None
        } else {
            Some(ReportSearchState {
                query: self.query.clone(),
                matches: self.matches.clone(),
                active_index: self.active_index,
                highlight: self.highlight,
                editing: self.editing,
            })
        }
    }

    fn query(&self) -> &str {
        &self.query
    }

    fn has_search_term(&self) -> bool {
        match self.highlight {
            ReportSearchHighlight::Generic => !self.normalized_query.is_empty(),
            ReportSearchHighlight::Error | ReportSearchHighlight::Warning => true,
        }
    }

    fn recompute_matches(&mut self, lines: &[String], mode: RecomputeMode) {
        let previous_line = match mode {
            RecomputeMode::Reset => None,
            RecomputeMode::Preserve => self.active_line(),
        };

        self.matches.clear();
        if !self.has_search_term() {
            self.active_index = None;
            return;
        }

        for (index, line) in lines.iter().enumerate() {
            if self.matches_line(line) {
                self.matches.push(index);
            }
        }

        if self.matches.is_empty() {
            self.active_index = None;
        } else if let Some(previous_line) = previous_line {
            self.active_index = self.matches.binary_search(&previous_line).ok().or(Some(0));
        } else {
            self.active_index = Some(0);
        }
    }

    fn matches_line(&self, line: &str) -> bool {
        if !self.has_search_term() {
            return false;
        }
        let lower = line.to_lowercase();
        match self.highlight {
            ReportSearchHighlight::Generic => lower.contains(&self.normalized_query),
            ReportSearchHighlight::Error => {
                lower.contains("error") || lower.contains("failed") || lower.contains("fatal")
            }
            ReportSearchHighlight::Warning => lower.contains("warning") || lower.contains("warn"),
        }
    }
}

enum RecomputeMode {
    Reset,
    Preserve,
}

struct LauncherApp {
    paths: AppPaths,
    logger: ShellLogger,
    ui: LauncherShellUi,
    surface: Surface,
    /// Device pixels per logical pixel, from `Window::scale_factor`.
    scale_factor: f32,
    preferences: LauncherPreferences,
    providers: FirstPartyProviders,
    /// Pointer position in *logical* GUI coordinates.
    pointer_position: Option<GuiPoint>,
    keyboard_modifiers: ModifiersState,
    report_search: ReportSearchController,
}

impl LauncherApp {
    fn new(window: &Window) -> Result<Self> {
        let size = window.inner_size();
        let (width, height) = enforce_min_size(size);
        let paths = AppPaths::discover().context("failed to discover Clonk Rust paths")?;
        paths
            .ensure_user_dirs()
            .context("failed to prepare launcher directories")?;

        let default_log = paths.logs_dir().join(format!(
            "clonk-launcher-shell-{}.log",
            timestamp_for_filename()
        ));
        let logger =
            ShellLogger::new(default_log).context("failed to initialise launcher logging")?;
        logger
            .log_line("launcher shell initialised")
            .context("failed to record launcher startup")?;

        let ui = LauncherShellUi::with_font(
            None,
            load_localization(&paths).context("failed to load launcher localization")?,
            load_ui_font(&paths),
        )
        .map_err(|err| anyhow!(err))?;
        let preferences = match load_launcher_preferences(&paths) {
            Ok(preferences) => preferences,
            Err(err) => {
                logger
                    .log_line(&format!("failed to load launcher preferences: {err}"))
                    .context("failed to record launcher preferences load failure")?;
                LauncherPreferences::default()
            }
        };
        let providers = FirstPartyProviders::discover(&paths, &preferences);

        let mut app = Self {
            paths,
            logger,
            ui,
            surface: Surface::new(width, height, PixelFormat::Rgba8888),
            scale_factor: window.scale_factor() as f32,
            preferences,
            providers,
            pointer_position: None,
            keyboard_modifiers: ModifiersState::empty(),
            report_search: ReportSearchController::default(),
        };
        app.refresh_state()
            .context("failed to load launcher state")?;
        app.restore_report_search_from_preferences()
            .context("failed to restore report search context")?;
        Ok(app)
    }

    fn handle_window_event(
        &mut self,
        pixels: &mut Pixels,
        event: WindowEvent,
        event_loop: &ActiveEventLoop,
    ) -> Result<()> {
        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                self.handle_resize(size, pixels)?;
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.scale_factor = scale_factor as f32;
            }
            WindowEvent::CursorMoved { position, .. } => {
                let logical = physical_to_logical_point(position.x, position.y, self.scale_factor);
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
            WindowEvent::KeyboardInput { event, .. } => {
                if self.handle_keyboard_input(&event)? {
                    return Ok(());
                }
                if let Some(key) = map_key_code(&event.logical_key) {
                    let gui_event = match event.state {
                        ElementState::Pressed => GuiEvent::KeyDown { key },
                        ElementState::Released => GuiEvent::KeyUp { key },
                    };
                    self.dispatch_gui_event(gui_event)?;
                }
                if event.state == ElementState::Pressed
                    && report_search_key_text_allowed(self.keyboard_modifiers)
                {
                    for ch in event.text.as_deref().into_iter().flat_map(str::chars) {
                        self.handle_received_character(ch)?;
                    }
                }
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                self.keyboard_modifiers = modifiers.state();
            }
            WindowEvent::Ime(Ime::Commit(text)) => {
                for ch in text.chars() {
                    self.handle_received_character(ch)?;
                }
            }
            WindowEvent::Touch(touch) => {
                let position = physical_to_logical_point(
                    touch.location.x,
                    touch.location.y,
                    self.scale_factor,
                );
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
                self.keyboard_modifiers = ModifiersState::empty();
            }
            WindowEvent::Focused(true) => {
                if let Err(err) = self.refresh_state() {
                    tracing::warn!(
                        error = ?err,
                        "failed to refresh launcher state on focus gain"
                    );
                }
            }
            WindowEvent::DroppedFile(_)
            | WindowEvent::HoveredFile(_)
            | WindowEvent::HoveredFileCancelled => {}
            _ => {}
        }
        Ok(())
    }

    fn report_has_any_lines(&self) -> bool {
        matches!(self.ui.state(), Some(state) if !state.support_bundle_report.is_empty())
    }

    fn sync_report_search_state(&mut self) -> Result<()> {
        let state = self.report_search.ui_state();
        self.ui
            .set_report_search(state)
            .map_err(|err| anyhow!(err))
            .context("failed to update report search UI")?;
        self.update_report_search_preferences();
        Ok(())
    }

    fn update_report_search_preferences(&mut self) {
        let new_preference = self.report_search.persisted_preferences();
        let existing = self.preferences.report_search().cloned();
        if existing != new_preference {
            self.preferences.set_report_search(new_preference.clone());
            self.persist_preferences();
            if let Err(err) = self.persist_report_search_summary(new_preference) {
                let _ = self.logger.log_line(&format!(
                    "failed to persist report search state into launcher summary: {err}"
                ));
            }
        }
    }

    fn ensure_active_match_visible(&mut self) -> Result<()> {
        if let Some(line) = self.report_search.active_line() {
            self.ui
                .ensure_report_line_visible(line)
                .map_err(|err| anyhow!(err))
                .context("failed to align report preview with search result")?;
        }
        Ok(())
    }

    fn append_report_search_character(&mut self, ch: char) -> Result<()> {
        if !self.report_has_any_lines() {
            return Ok(());
        }
        {
            let Some(state) = self.ui.state() else {
                return Ok(());
            };
            self.report_search
                .append_char(ch, &state.support_bundle_report);
        }
        self.sync_report_search_state()?;
        self.ensure_active_match_visible()
    }

    fn backspace_report_search(&mut self) -> Result<()> {
        if !self.report_search.editing() {
            return Ok(());
        }
        {
            let Some(state) = self.ui.state() else {
                return Ok(());
            };
            self.report_search.backspace(&state.support_bundle_report);
        }
        self.sync_report_search_state()?;
        self.ensure_active_match_visible()
    }

    fn finish_report_search_editing(&mut self) -> Result<()> {
        if !self.report_search.editing() {
            return Ok(());
        }
        self.report_search.set_editing(false);
        self.sync_report_search_state()
    }

    fn focus_report_search(&mut self) -> Result<()> {
        if !self.report_has_any_lines() {
            self.set_feedback(ActionFeedback::info(
                "Launch `clonk-game` once to generate diagnostics before searching the support bundle report.",
            ))?;
            return Ok(());
        }
        self.logger
            .log_line("report search focus requested via diagnostics UI")
            .context("failed to log report search focus request")?;
        self.report_search.set_editing(true);
        self.sync_report_search_state()?;
        self.set_feedback(ActionFeedback::info(
            "Search activated. Type to filter the report, press Enter to accept, Esc to clear.",
        ))?;
        Ok(())
    }

    fn clear_report_search(&mut self) -> Result<()> {
        if !self.report_search.editing() && !self.report_search.is_active() {
            return Ok(());
        }
        self.logger
            .log_line("report search cleared via diagnostics UI")
            .context("failed to log report search clear action")?;
        self.report_search.clear();
        self.sync_report_search_state()?;
        self.set_feedback(ActionFeedback::info("Search cleared."))?;
        Ok(())
    }

    fn apply_report_search_preset(&mut self, preset: ReportSearchPreset) -> Result<()> {
        if !self.report_has_any_lines() {
            self.set_feedback(ActionFeedback::info(
                "Launch `clonk-game` once to generate diagnostics before searching the support bundle report.",
            ))?;
            return Ok(());
        }
        self.logger
            .log_line(&format!(
                "report search preset {:?} requested via diagnostics UI",
                preset
            ))
            .context("failed to log report search preset request")?;
        let match_count = {
            let Some(state) = self.ui.state() else {
                return Ok(());
            };
            self.report_search
                .apply_preset(preset, &state.support_bundle_report);
            self.report_search.match_count()
        };
        self.sync_report_search_state()?;
        self.ensure_active_match_visible()?;
        if match_count == 0 {
            let label = match preset {
                ReportSearchPreset::Errors => "errors or failures",
                ReportSearchPreset::Warnings => "warnings",
            };
            self.set_feedback(ActionFeedback::info(format!(
                "No {label} were detected in the report."
            )))?;
        } else {
            let descriptor = match preset {
                ReportSearchPreset::Errors => "error",
                ReportSearchPreset::Warnings => "warning",
            };
            self.set_feedback(ActionFeedback::info(format!(
                "Found {match_count} {descriptor} match{} in the report.",
                if match_count == 1 { "" } else { "es" }
            )))?;
        }
        Ok(())
    }

    fn next_report_search_match(&mut self) -> Result<()> {
        if !self.report_search.has_matches() {
            if self.report_search.is_active() {
                let term = self.report_search.query();
                if term.is_empty() {
                    self.set_feedback(ActionFeedback::info(
                        "No matches are available for the current search.",
                    ))?;
                } else {
                    self.set_feedback(ActionFeedback::info(format!(
                        "No matches found for \"{term}\"."
                    )))?;
                }
            }
            return Ok(());
        }
        self.logger
            .log_line("report search next match requested via diagnostics UI")
            .context("failed to log next search match request")?;
        self.report_search.next();
        self.sync_report_search_state()?;
        self.ensure_active_match_visible()
    }

    fn previous_report_search_match(&mut self) -> Result<()> {
        if !self.report_search.has_matches() {
            if self.report_search.is_active() {
                let term = self.report_search.query();
                if term.is_empty() {
                    self.set_feedback(ActionFeedback::info(
                        "No matches are available for the current search.",
                    ))?;
                } else {
                    self.set_feedback(ActionFeedback::info(format!(
                        "No matches found for \"{term}\"."
                    )))?;
                }
            }
            return Ok(());
        }
        self.logger
            .log_line("report search previous match requested via diagnostics UI")
            .context("failed to log previous search match request")?;
        self.report_search.previous();
        self.sync_report_search_state()?;
        self.ensure_active_match_visible()
    }

    fn refresh_report_search_for_state(&mut self) -> Result<()> {
        if !self.report_search.editing() && !self.report_search.is_active() {
            self.report_search.matches.clear();
            self.report_search.active_index = None;
            return self.sync_report_search_state();
        }
        if let Some(state) = self.ui.state() {
            self.report_search
                .refresh_for_lines(&state.support_bundle_report);
            self.sync_report_search_state()?;
            self.ensure_active_match_visible()
        } else {
            self.report_search.clear();
            self.sync_report_search_state()
        }
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

    fn handle_keyboard_input(&mut self, input: &KeyEvent) -> Result<bool> {
        if input.state != ElementState::Pressed {
            return Ok(false);
        }
        match &input.logical_key {
            Key::Named(NamedKey::Escape) => {
                if self.report_search.editing() || self.report_search.is_active() {
                    self.clear_report_search()?;
                    return Ok(true);
                }
            }
            Key::Named(NamedKey::Enter) => {
                if self.report_search.editing() {
                    self.finish_report_search_editing()?;
                    return Ok(true);
                }
            }
            Key::Named(NamedKey::Backspace) => {
                if self.report_search.editing() {
                    self.backspace_report_search()?;
                    return Ok(true);
                }
            }
            Key::Named(NamedKey::ArrowUp) => {
                if self.report_search.has_matches() {
                    self.previous_report_search_match()?;
                    return Ok(true);
                }
            }
            Key::Named(NamedKey::ArrowDown) if self.report_search.has_matches() => {
                self.next_report_search_match()?;
                return Ok(true);
            }
            _ => {}
        }
        Ok(false)
    }

    fn handle_received_character(&mut self, ch: char) -> Result<bool> {
        if !self.report_search.editing() {
            return Ok(false);
        }
        if ch == '\u{8}' || ch == '\u{7f}' {
            // Backspace/delete are handled via keyboard events.
            return Ok(true);
        }
        if ch == '\r' || ch == '\n' {
            self.finish_report_search_editing()?;
            return Ok(true);
        }
        if ch.is_control() {
            return Ok(true);
        }
        self.append_report_search_character(ch)?;
        Ok(true)
    }

    fn process_response(&mut self, response: LauncherShellResponse) -> Result<()> {
        for message in response.messages {
            if let Err(err) = self.handle_message(message) {
                tracing::error!(error = ?err, "launcher action failed");
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

    fn refresh_report_preview(&self, state: &mut LauncherShellState) {
        let telemetry = UpdateTelemetrySummary::from_serializable(
            &state.summary.update_telemetry,
            &state.logs_dir,
        );
        state.support_bundle_report = render_support_bundle_report(
            &self.paths,
            state.support_bundle_path.as_deref(),
            &telemetry,
        );
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
        bulk_retarget: Option<ProviderBulkRetargetSummary>,
    ) -> Result<(
        ProviderAutomationSnapshot,
        Option<ProviderBulkRetargetSummary>,
    )> {
        let telemetry = UpdateTelemetrySummary::from_serializable(
            &state.summary.update_telemetry,
            &state.logs_dir,
        );
        let snapshot = ProviderAutomationSnapshot::from_diagnostics(diagnostics, &state.logs_dir);
        let bulk_summary = prune_bulk_retarget_history(
            bulk_retarget.or_else(|| state.summary.provider_bulk_retarget.clone()),
            diagnostics,
        );
        write_launcher_summary(
            &self.paths,
            &self.logger,
            &state.launcher_log_path,
            &state.runtime_log_paths,
            &state.crash_report_paths,
            &telemetry,
            state.support_bundle_path.as_deref(),
            Some(snapshot.clone()),
            bulk_summary.clone(),
            self.preferences.report_search().cloned(),
        )
        .context("failed to persist provider automation snapshot")?;
        if let Some(summary) = bulk_summary.as_ref() {
            if let Some(cleared_at) = summary.history_cleared_at.as_ref() {
                let previous_cleared = state
                    .summary
                    .provider_bulk_retarget
                    .as_ref()
                    .and_then(|previous| previous.history_cleared_at.as_ref());
                if previous_cleared != Some(cleared_at) {
                    let _ = self
                        .logger
                        .log_line(&format!("bulk retarget history cleared at {cleared_at}"));
                }
            }
        }
        Ok((snapshot, bulk_summary))
    }

    fn persist_preferences(&self) {
        if let Err(err) = save_launcher_preferences(&self.paths, &self.preferences) {
            let _ = self
                .logger
                .log_line(&format!("failed to persist launcher preferences: {err}"));
        }
    }

    fn persist_report_search_summary(
        &self,
        preference: Option<ReportSearchPreferences>,
    ) -> Result<()> {
        let Some(state) = self.ui.state().cloned() else {
            return Ok(());
        };
        let telemetry = UpdateTelemetrySummary::from_serializable(
            &state.summary.update_telemetry,
            &state.logs_dir,
        );
        write_launcher_summary(
            &self.paths,
            &self.logger,
            &state.launcher_log_path,
            &state.runtime_log_paths,
            &state.crash_report_paths,
            &telemetry,
            state.support_bundle_path.as_deref(),
            Some(state.summary.provider_automation.clone()),
            state.summary.provider_bulk_retarget.clone(),
            preference,
        )
        .context("failed to update launcher summary with report search state")?;
        Ok(())
    }

    fn restore_report_search_from_preferences(&mut self) -> Result<()> {
        let Some(preference) = self.preferences.report_search().cloned() else {
            return Ok(());
        };
        let lines = self
            .ui
            .state()
            .map(|state| state.support_bundle_report.clone())
            .unwrap_or_default();
        self.report_search
            .restore_from_preferences(&preference, &lines);
        self.sync_report_search_state()?;
        self.ensure_active_match_visible()
    }

    fn remember_bundle_destination(&mut self, path: &Path) {
        self.preferences.set_bundle_destination(path);
        self.persist_preferences();
    }

    fn remember_upload_destination(&mut self, path: &Path) {
        self.preferences.set_upload_destination(path);
        self.persist_preferences();
    }

    fn remember_provider_override(&mut self, role: ProviderRole, name: &str, path: &Path) {
        self.preferences
            .set_provider_override(role.as_str(), name, path);
        self.persist_preferences();
    }

    fn forget_provider_override(&mut self, role: ProviderRole, name: &str) {
        self.preferences
            .clear_provider_override(role.as_str(), name);
        self.persist_preferences();
    }

    fn forget_all_first_party_overrides(&mut self) {
        self.preferences
            .clear_provider_overrides_for_role(ProviderRole::Share.as_str());
        self.preferences
            .clear_provider_overrides_for_role(ProviderRole::Upload.as_str());
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
            LauncherShellMessage::FocusReportSearch => {
                self.focus_report_search()?;
            }
            LauncherShellMessage::ClearReportSearch => {
                self.clear_report_search()?;
            }
            LauncherShellMessage::NextReportSearchMatch => {
                self.next_report_search_match()?;
            }
            LauncherShellMessage::PreviousReportSearchMatch => {
                self.previous_report_search_match()?;
            }
            LauncherShellMessage::SetReportSearchPreset { preset } => {
                self.apply_report_search_preset(preset)?;
            }
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
                                let (stage_report, automation_report) = self
                                    .providers
                                    .stage_bundle(&self.logger, &bundle_path, None);
                                let diagnostics = self
                                    .update_provider_diagnostics()
                                    .context(
                                        "failed to refresh provider diagnostics after support bundle staging",
                                    )?;
                                if let Some(mut state) = state_snapshot {
                                    let (snapshot, bulk_summary) = self
                                        .persist_provider_snapshot(&state, &diagnostics, None)
                                        .context(
                                        "failed to persist provider automation after bundle staging",
                                    )?;
                                    state.summary.provider_automation = snapshot;
                                    state.summary.provider_bulk_retarget = bulk_summary;
                                    self.refresh_report_preview(&mut state);
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
            LauncherShellMessage::CopyReportPreview => {
                self.handle_copy_report_preview()?;
            }
            LauncherShellMessage::ExportReportPreview => {
                self.handle_export_report_preview()?;
            }
            LauncherShellMessage::ScrollReportPreview { delta } => {
                self.handle_scroll_report_preview(delta)?;
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
                                let (stage_report, automation_report) = self
                                    .providers
                                    .stage_artifacts(&self.logger, &artifacts, None);
                                let diagnostics = self.update_provider_diagnostics().context(
                                    "failed to refresh provider diagnostics after artifact staging",
                                )?;
                                if let Some(mut state) = state_snapshot {
                                    let (snapshot, bulk_summary) = self
                                        .persist_provider_snapshot(&state, &diagnostics, None)
                                        .context(
                                            "failed to persist provider automation after artifact staging",
                                        )?;
                                    state.summary.provider_automation = snapshot;
                                    state.summary.provider_bulk_retarget = bulk_summary;
                                    self.refresh_report_preview(&mut state);
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
            LauncherShellMessage::RestageProvider { role, index } => {
                self.handle_restage_provider(role, index)?;
            }
            LauncherShellMessage::RetargetProvider { role, index } => {
                self.handle_retarget_provider(role, index)?;
            }
            LauncherShellMessage::ClearProviderOverride { role, index } => {
                self.handle_clear_provider_override(role, index)?;
            }
            LauncherShellMessage::RetargetAllProviders => {
                self.handle_retarget_all_providers()?;
            }
            LauncherShellMessage::RestoreAllProviderDefaults => {
                self.handle_restore_all_provider_defaults()?;
            }
        }
        Ok(())
    }

    fn handle_copy_report_preview(&mut self) -> Result<()> {
        self.logger
            .log_line("report preview copy requested via launcher UI")
            .context("failed to log report preview copy request")?;

        let (report_text, line_count) = match self.report_preview_text() {
            Some(value) => value,
            None => {
                self.set_feedback(ActionFeedback::info(
                    "No report preview is available yet. Launch the game to generate diagnostics.",
                ))?;
                return Ok(());
            }
        };

        match Clipboard::new() {
            Ok(mut clipboard) => match clipboard.set_text(report_text) {
                Ok(()) => {
                    self.logger
                        .log_line(&format!(
                            "report preview copied to clipboard ({} lines)",
                            line_count
                        ))
                        .context("failed to log report preview copy success")?;
                    self.set_feedback(ActionFeedback::success(
                        "Report preview copied to the clipboard.",
                    ))?;
                }
                Err(err) => {
                    self.logger
                        .log_line(&format!("failed to copy report preview: {err}"))
                        .context("failed to log report preview copy failure")?;
                    self.set_feedback(ActionFeedback::error(format!(
                        "Failed to copy the report preview: {err}"
                    )))?;
                }
            },
            Err(err) => {
                self.logger
                    .log_line(&format!("failed to access clipboard: {err}"))
                    .context("failed to log clipboard initialisation failure")?;
                self.set_feedback(ActionFeedback::error(format!(
                    "Failed to access the clipboard: {err}"
                )))?;
            }
        }
        Ok(())
    }

    fn handle_export_report_preview(&mut self) -> Result<()> {
        self.logger
            .log_line("report preview export requested via launcher UI")
            .context("failed to log report preview export request")?;

        let (report_text, line_count) = match self.report_preview_text() {
            Some(value) => value,
            None => {
                self.set_feedback(ActionFeedback::info(
                    "No report preview is available yet. Launch the game to generate diagnostics.",
                ))?;
                return Ok(());
            }
        };

        let mut dialog = FileDialog::new()
            .set_title("Save support bundle report")
            .add_filter("Text files", &["txt"])
            .set_file_name(format!(
                "support-bundle-report-{}.txt",
                timestamp_for_filename()
            ));
        dialog = self.configure_dialog_directory(dialog, None, Some(self.paths.logs_dir()));

        match dialog.save_file() {
            Some(path) => {
                let mut file_text = report_text;
                if !file_text.ends_with('\n') {
                    file_text.push('\n');
                }
                match fs::write(&path, file_text) {
                    Ok(()) => {
                        self.logger
                            .log_line(&format!(
                                "report preview exported to {} ({} lines)",
                                path.display(),
                                line_count
                            ))
                            .context("failed to log report preview export success")?;
                        self.set_feedback(ActionFeedback::success(format!(
                            "Report preview saved to {}",
                            path.display()
                        )))?;
                    }
                    Err(err) => {
                        self.logger
                            .log_line(&format!(
                                "failed to write report preview export to {}: {err}",
                                path.display()
                            ))
                            .context("failed to log report preview export failure")?;
                        self.set_feedback(ActionFeedback::error(format!(
                            "Failed to save the report preview: {err}"
                        )))?;
                    }
                }
            }
            None => {
                self.logger
                    .log_line("report preview export cancelled by user")
                    .context("failed to log report preview export cancellation")?;
                self.set_feedback(ActionFeedback::info(
                    "Export cancelled. No files were written.",
                ))?;
            }
        }

        Ok(())
    }

    fn handle_scroll_report_preview(&mut self, delta: isize) -> Result<()> {
        self.ui
            .scroll_report_preview(delta)
            .map_err(|err| anyhow!(err))
            .context("failed to update report preview scroll state")
    }

    fn report_preview_text(&self) -> Option<(String, usize)> {
        let state = self.ui.state()?;
        if state.support_bundle_report.is_empty() {
            return None;
        }
        let mut export_lines = state.support_bundle_report.clone();
        if let Some(annotations) = self
            .report_search
            .export_annotations(&state.support_bundle_report)
        {
            export_lines.extend(annotations);
        }
        let line_count = export_lines.len();
        Some((export_lines.join("\n"), line_count))
    }

    fn handle_restage_provider(&mut self, kind: ProviderKind, index: usize) -> Result<()> {
        let role: ProviderRole = kind.into();
        let provider_label = kind.label();
        let provider_name = match self.providers.provider_name(role, index) {
            Some(name) => name.to_string(),
            None => {
                let message = format!(
                    "The {} you selected is no longer available. Refresh diagnostics and try again.",
                    provider_label
                );
                self.set_feedback(ActionFeedback::error(message))?;
                return Ok(());
            }
        };

        let state_snapshot = self.ui.state().cloned();
        let state_ref = match state_snapshot.as_ref() {
            Some(state) => state,
            None => {
                self.set_feedback(ActionFeedback::info(
                    "Launch `clonk-game` once to generate diagnostics before restaging providers.",
                ))?;
                return Ok(());
            }
        };

        self.logger
            .log_line(&format!(
                "restaging {} {} requested via diagnostics UI",
                provider_label, provider_name
            ))
            .context("failed to log provider restage request")?;

        let (stage_report, automation_report) = match kind {
            ProviderKind::Share => {
                let bundle_path = match &state_ref.support_bundle_path {
                    Some(path) => path.clone(),
                    None => {
                        let message = format!(
                            "Support bundle is missing; regenerate it before restaging {} {}.",
                            provider_label, provider_name
                        );
                        self.logger
                            .log_line(&message)
                            .context("failed to log missing support bundle for restage")?;
                        self.set_feedback(ActionFeedback::error(message))?;
                        return Ok(());
                    }
                };
                if !bundle_path.exists() {
                    let message = format!(
                        "Support bundle {} is missing; regenerate it before restaging {} {}.",
                        bundle_path.display(),
                        provider_label,
                        provider_name
                    );
                    self.logger
                        .log_line(&message)
                        .context("failed to log missing support bundle for restage")?;
                    self.set_feedback(ActionFeedback::error(message))?;
                    return Ok(());
                }
                self.providers
                    .stage_bundle(&self.logger, &bundle_path, Some(&[index]))
            }
            ProviderKind::Upload => {
                let artifacts = support_artifacts(state_ref);
                if artifacts.is_empty() {
                    let message = format!(
                        "No support artifacts are available to restage {} {}. Launch the game first.",
                        provider_label, provider_name
                    );
                    self.set_feedback(ActionFeedback::info(message))?;
                    return Ok(());
                }
                self.providers
                    .stage_artifacts(&self.logger, &artifacts, Some(&[index]))
            }
        };

        let diagnostics = self
            .update_provider_diagnostics()
            .context("failed to refresh provider diagnostics after provider restage")?;
        if let Some(mut state) = state_snapshot {
            let (snapshot, bulk_summary) = self
                .persist_provider_snapshot(&state, &diagnostics, None)
                .context("failed to persist provider automation after provider restage")?;
            state.summary.provider_automation = snapshot;
            state.summary.provider_bulk_retarget = bulk_summary;
            self.refresh_report_preview(&mut state);
            self.ui
                .set_state(Some(state))
                .map_err(|err| anyhow!(err))
                .context("failed to refresh launcher summary after provider restage")?;
        }

        let message = format!("Restaged {} {}", provider_label, provider_name);
        let feedback = self.feedback_from_reports(message, &stage_report, &automation_report);
        self.set_feedback(feedback)?;
        Ok(())
    }

    fn handle_retarget_provider(&mut self, kind: ProviderKind, index: usize) -> Result<()> {
        let role: ProviderRole = kind.into();
        let provider_label = kind.label();
        let provider_name = match self.providers.provider_name(role, index) {
            Some(name) => name.to_string(),
            None => {
                let message = format!(
                    "The {} you selected is no longer available. Refresh diagnostics and try again.",
                    provider_label
                );
                self.set_feedback(ActionFeedback::error(message))?;
                return Ok(());
            }
        };

        let current_path = self
            .providers
            .provider_path(role, index)
            .map(|path| path.to_path_buf());
        let saved_override = self
            .preferences
            .provider_override_path(role.as_str(), &provider_name);

        let mut dialog =
            FileDialog::new().set_title(format!("Select directory for {}", provider_name));
        dialog = self.configure_dialog_directory(
            dialog,
            saved_override,
            current_path.as_deref().or(Some(self.paths.logs_dir())),
        );

        match dialog.pick_folder() {
            Some(destination) => {
                self.logger
                    .log_line(&format!(
                        "retargeting {} {} to {} via diagnostics UI",
                        provider_label,
                        provider_name,
                        destination.display()
                    ))
                    .context("failed to log provider retarget request")?;
                self.providers
                    .retarget_provider(role, index, destination.clone())
                    .context("failed to retarget provider")?;
                self.remember_provider_override(role, &provider_name, &destination);

                let diagnostics = self
                    .update_provider_diagnostics()
                    .context("failed to refresh provider diagnostics after provider retarget")?;
                if let Some(mut state) = self.ui.state().cloned() {
                    let (snapshot, bulk_summary) = self
                        .persist_provider_snapshot(&state, &diagnostics, None)
                        .context("failed to persist provider automation after provider retarget")?;
                    state.summary.provider_automation = snapshot;
                    state.summary.provider_bulk_retarget = bulk_summary;
                    self.refresh_report_preview(&mut state);
                    self.ui
                        .set_state(Some(state))
                        .map_err(|err| anyhow!(err))
                        .context("failed to refresh launcher summary after provider retarget")?;
                }

                let message = format!(
                    "Retargeted {} {} to {}",
                    provider_label,
                    provider_name,
                    destination.display()
                );
                self.set_feedback(ActionFeedback::success(message))?;
            }
            None => {
                self.logger
                    .log_line(&format!(
                        "retarget {} {} cancelled by user",
                        provider_label, provider_name
                    ))
                    .context("failed to log provider retarget cancellation")?;
                self.set_feedback(ActionFeedback::info(
                    "Retarget cancelled. No changes were made.",
                ))?;
            }
        }

        Ok(())
    }

    fn handle_clear_provider_override(&mut self, kind: ProviderKind, index: usize) -> Result<()> {
        let role: ProviderRole = kind.into();
        let provider_label = kind.label();
        let provider_name = match self.providers.provider_name(role, index) {
            Some(name) => name.to_string(),
            None => {
                let message = format!(
                    "The {} you selected is no longer available. Refresh diagnostics and try again.",
                    provider_label
                );
                self.set_feedback(ActionFeedback::error(message))?;
                return Ok(());
            }
        };

        let default_path = match self.providers.provider_default_path(role, index) {
            Some(path) => path.to_path_buf(),
            None => {
                self.set_feedback(ActionFeedback::error(format!(
                    "Unable to determine the default path for {} {}.",
                    provider_label, provider_name
                )))?;
                return Ok(());
            }
        };

        let had_preference_override = self
            .preferences
            .provider_override_path(role.as_str(), &provider_name)
            .is_some();
        let previous_path = self
            .providers
            .provider_path(role, index)
            .map(|path| path.to_path_buf());
        let was_default_path = previous_path
            .as_ref()
            .map(|path| paths_equivalent(path, &default_path))
            .unwrap_or(false);

        if !had_preference_override && was_default_path {
            self.set_feedback(ActionFeedback::info(format!(
                "{} {} already uses the default path.",
                provider_label, provider_name
            )))?;
            return Ok(());
        }

        self.logger
            .log_line(&format!(
                "clearing overrides for {} {} via diagnostics UI",
                provider_label, provider_name
            ))
            .context("failed to log provider override clearing request")?;

        let path_changed = self
            .providers
            .restore_default(role, index)
            .context("failed to restore provider default path")?;

        self.forget_provider_override(role, &provider_name);

        let diagnostics = self
            .update_provider_diagnostics()
            .context("failed to refresh provider diagnostics after clearing override")?;
        if let Some(mut state) = self.ui.state().cloned() {
            let (snapshot, bulk_summary) = self
                .persist_provider_snapshot(&state, &diagnostics, None)
                .context("failed to persist provider automation after clearing override")?;
            state.summary.provider_automation = snapshot;
            state.summary.provider_bulk_retarget = bulk_summary;
            self.refresh_report_preview(&mut state);
            self.ui
                .set_state(Some(state))
                .map_err(|err| anyhow!(err))
                .context("failed to refresh launcher summary after clearing override")?;
        }

        let message = if path_changed {
            format!(
                "Restored {} {} to default path {}",
                provider_label,
                provider_name,
                default_path.display()
            )
        } else {
            format!(
                "Cleared saved overrides for {} {} (already using default path).",
                provider_label, provider_name
            )
        };
        self.set_feedback(ActionFeedback::success(message))?;

        Ok(())
    }

    fn handle_retarget_all_providers(&mut self) -> Result<()> {
        if !self.providers.has_share_targets() && !self.providers.has_upload_targets() {
            self.set_feedback(ActionFeedback::info(
                "No first-party providers are configured. Configure LC_FIRST_PARTY_* variables to enable automated submissions.",
            ))?;
            return Ok(());
        }

        self.logger
            .log_line("bulk retarget requested via diagnostics UI")
            .context("failed to log bulk provider retarget request")?;

        let share_base = if self.providers.has_share_targets() {
            match self.pick_bulk_retarget_directory(ProviderRole::Share) {
                Some(path) => Some(path),
                None => {
                    self.logger
                        .log_line("bulk retarget cancelled while selecting share target directory")
                        .context("failed to log bulk retarget cancellation")?;
                    self.set_feedback(ActionFeedback::info(
                        "Retarget cancelled. No changes were made.",
                    ))?;
                    return Ok(());
                }
            }
        } else {
            None
        };

        let upload_base = if self.providers.has_upload_targets() {
            match self.pick_bulk_retarget_directory(ProviderRole::Upload) {
                Some(path) => Some(path),
                None => {
                    self.logger
                        .log_line("bulk retarget cancelled while selecting upload target directory")
                        .context("failed to log bulk retarget cancellation")?;
                    self.set_feedback(ActionFeedback::info(
                        "Retarget cancelled. No changes were made.",
                    ))?;
                    return Ok(());
                }
            }
        } else {
            None
        };

        let mut selections = Vec::new();

        if let Some(base) = share_base {
            self.logger
                .log_line(&format!(
                    "bulk retargeting share targets under {}",
                    base.display()
                ))
                .context("failed to log bulk share retarget target")?;
            let outcomes = self
                .providers
                .retarget_all_to_base(ProviderRole::Share, &base);
            for outcome in &outcomes {
                self.remember_provider_override(
                    ProviderRole::Share,
                    &outcome.name,
                    &outcome.new_path,
                );
            }
            selections.push(RoleRetargetOutcome {
                role: ProviderRole::Share,
                base,
                outcomes,
            });
        }

        if let Some(base) = upload_base {
            self.logger
                .log_line(&format!(
                    "bulk retargeting upload targets under {}",
                    base.display()
                ))
                .context("failed to log bulk upload retarget target")?;
            let outcomes = self
                .providers
                .retarget_all_to_base(ProviderRole::Upload, &base);
            for outcome in &outcomes {
                self.remember_provider_override(
                    ProviderRole::Upload,
                    &outcome.name,
                    &outcome.new_path,
                );
            }
            selections.push(RoleRetargetOutcome {
                role: ProviderRole::Upload,
                base,
                outcomes,
            });
        }

        let diagnostics = self
            .update_provider_diagnostics()
            .context("failed to refresh provider diagnostics after bulk retarget")?;
        if let Some(mut state) = self.ui.state().cloned() {
            let bulk_summary = bulk_retarget_summary(&selections, &state.logs_dir);
            let (snapshot, persisted_bulk) = self
                .persist_provider_snapshot(&state, &diagnostics, bulk_summary.clone())
                .context("failed to persist provider automation after bulk retarget")?;
            state.summary.provider_automation = snapshot;
            state.summary.provider_bulk_retarget = persisted_bulk;
            self.refresh_report_preview(&mut state);
            self.ui
                .set_state(Some(state))
                .map_err(|err| anyhow!(err))
                .context("failed to refresh launcher summary after bulk retarget")?;
        }

        for selection in &selections {
            for outcome in &selection.outcomes {
                if outcome.changed {
                    self.logger
                        .log_line(&format!(
                            "retargeted {} {} from {} to {} via bulk retarget",
                            selection.role.label(),
                            outcome.name,
                            outcome.previous_path.display(),
                            outcome.new_path.display()
                        ))
                        .context("failed to log bulk provider retarget outcome")?;
                } else {
                    self.logger
                        .log_line(&format!(
                            "{} {} already used {}; no change during bulk retarget",
                            selection.role.label(),
                            outcome.name,
                            outcome.new_path.display()
                        ))
                        .context("failed to log no-op bulk provider retarget outcome")?;
                }
            }
        }

        let feedback_message = bulk_retarget_feedback(&selections);
        let changed = selections
            .iter()
            .flat_map(|selection| selection.outcomes.iter())
            .any(|outcome| outcome.changed);
        let feedback = if changed {
            ActionFeedback::success(feedback_message)
        } else {
            ActionFeedback::info(feedback_message)
        };
        self.set_feedback(feedback)?;

        Ok(())
    }

    fn handle_restore_all_provider_defaults(&mut self) -> Result<()> {
        if !self.providers.has_share_targets() && !self.providers.has_upload_targets() {
            self.set_feedback(ActionFeedback::info(
                "No first-party providers are configured. Configure LC_FIRST_PARTY_* variables to enable automated submissions.",
            ))?;
            return Ok(());
        }

        self.logger
            .log_line("restoring all first-party providers to default paths via diagnostics UI")
            .context("failed to log bulk provider restore request")?;

        let outcome = self
            .providers
            .restore_all_defaults()
            .context("failed to restore default paths for first-party providers")?;

        self.forget_all_first_party_overrides();

        let diagnostics = self
            .update_provider_diagnostics()
            .context("failed to refresh provider diagnostics after restoring defaults")?;
        if let Some(mut state) = self.ui.state().cloned() {
            let (snapshot, bulk_summary) = self
                .persist_provider_snapshot(&state, &diagnostics, None)
                .context("failed to persist provider automation after restoring defaults")?;
            state.summary.provider_automation = snapshot;
            state.summary.provider_bulk_retarget = bulk_summary;
            self.refresh_report_preview(&mut state);
            self.ui
                .set_state(Some(state))
                .map_err(|err| anyhow!(err))
                .context("failed to refresh launcher summary after restoring defaults")?;
        }

        let log_summary = format!(
            "bulk provider restore results: share {}/{} reset, upload {}/{} reset",
            outcome.share_restored,
            outcome.share_total,
            outcome.upload_restored,
            outcome.upload_total
        );
        self.logger
            .log_line(&log_summary)
            .context("failed to log bulk provider restore results")?;

        let feedback_message = outcome.feedback_message();
        let feedback = if outcome.restored() > 0 {
            ActionFeedback::success(feedback_message)
        } else {
            ActionFeedback::info(feedback_message)
        };
        self.set_feedback(feedback)?;

        Ok(())
    }

    fn pick_bulk_retarget_directory(&self, role: ProviderRole) -> Option<PathBuf> {
        let title = match role {
            ProviderRole::Share => "Select base directory for share targets",
            ProviderRole::Upload => "Select base directory for upload targets",
        };
        let mut dialog = FileDialog::new().set_title(title);
        if let Some(dir) = self.bulk_retarget_start_directory(role) {
            dialog = dialog.set_directory(dir);
        } else {
            dialog = dialog.set_directory(self.paths.logs_dir());
        }
        dialog.pick_folder()
    }

    fn bulk_retarget_start_directory(&self, role: ProviderRole) -> Option<PathBuf> {
        if let Some(name) = self.providers.provider_name(role, 0) {
            if let Some(path) = self.preferences.provider_override_path(role.as_str(), name) {
                if let Some(parent) = path.parent() {
                    return Some(parent.to_path_buf());
                }
            }
        }
        self.providers.bulk_dialog_hint(role)
    }

    fn refresh_state(&mut self) -> Result<()> {
        let state = load_shell_state(&self.paths).context("failed to load launcher state")?;
        match state {
            Some(mut state) => {
                self.providers
                    .hydrate_from_snapshot(&state.logs_dir, &state.summary.provider_automation);
                self.logger
                    .set_target(state.launcher_log_path.clone())
                    .context("failed to attach launcher log to runtime log file")?;
                self.refresh_report_preview(&mut state);
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
        self.refresh_report_search_for_state()
            .context("failed to refresh report search state")?;
        self.update_provider_diagnostics()
            .context("failed to refresh provider diagnostics")?;
        Ok(())
    }

    fn render(&mut self, frame: &mut [u8]) -> Result<()> {
        let width = self.surface.width();
        let height = self.surface.height();

        self.surface.fill(Color::opaque(12, 16, 28));
        let (logical_width, logical_height) = logical_extent(width, height, self.scale_factor);
        self.ui.layout(GuiSize::new(logical_width, logical_height));
        let commands = self.ui.render();
        // The face layout measured with is the face the rasteriser must use.
        let font = self.ui.font();
        render_commands(
            &mut self.surface,
            &commands,
            self.scale_factor,
            font.as_ref(),
        );
        debug_assert_eq!(frame.len(), (width as usize) * (height as usize) * 4);
        frame.copy_from_slice(self.surface.pixels());
        Ok(())
    }
}

fn prune_bulk_retarget_history(
    summary: Option<ProviderBulkRetargetSummary>,
    diagnostics: &ProviderDiagnostics,
) -> Option<ProviderBulkRetargetSummary> {
    let mut summary = summary?;
    let mut share_cleared = false;
    if role_uses_default_paths(&diagnostics.share) {
        share_cleared = !summary.share.is_empty();
        summary.share.clear();
    }
    let mut upload_cleared = false;
    if role_uses_default_paths(&diagnostics.upload) {
        upload_cleared = !summary.upload.is_empty();
        summary.upload.clear();
    }

    if summary.share.is_empty() && summary.upload.is_empty() {
        if (share_cleared || upload_cleared) && summary.history_cleared_at.is_none() {
            summary.history_cleared_at = Some(timestamp_for_log());
        }
        if summary.history_cleared_at.is_some() {
            Some(summary)
        } else {
            None
        }
    } else {
        summary.history_cleared_at = None;
        Some(summary)
    }
}

fn role_uses_default_paths(statuses: &[ProviderStatus]) -> bool {
    if statuses.is_empty() {
        return true;
    }
    statuses
        .iter()
        .all(|status| status.path == status.path_provenance.default_path())
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
    provenance: ProviderPathProvenance,
    automation: ProviderAutomationState,
}

impl ProviderTargetState {
    fn new(target: ProviderTarget) -> Self {
        let provenance = ProviderPathProvenance::new(target.path.clone());
        Self {
            target,
            provenance,
            automation: ProviderAutomationState::Idle,
        }
    }

    fn to_status(&self) -> ProviderStatus {
        ProviderStatus {
            name: self.target.name.clone(),
            path: self.target.path.clone(),
            path_status: compute_path_status(&self.target.path),
            automation: self.automation.clone(),
            path_provenance: self.provenance.clone(),
        }
    }

    fn apply_override(&mut self, path: PathBuf, source: ProviderOverrideSource) {
        if self.target.path == path
            && self
                .provenance
                .overrides()
                .last()
                .map(|override_entry| {
                    override_entry.path() == path.as_path() && override_entry.source() == &source
                })
                .unwrap_or(false)
        {
            return;
        }
        self.target.path = path.clone();
        self.provenance.apply_override(path, source);
        self.automation = ProviderAutomationState::Idle;
    }

    fn sync_provenance_from_record(&mut self, logs_dir: &Path, record: &ProviderAutomationRecord) {
        let mut provenance = match &record.default_path {
            Some(entry) => {
                let default_path = resolve_snapshot_path(logs_dir, entry);
                ProviderPathProvenance::new(default_path)
            }
            None => ProviderPathProvenance::new(self.provenance.default_path().to_path_buf()),
        };

        for override_record in &record.overrides {
            let path = resolve_snapshot_path(logs_dir, &override_record.path);
            let source = match &override_record.source {
                ProviderOverrideSourceRecord::Preference => ProviderOverrideSource::Preference,
                ProviderOverrideSourceRecord::Retargeted { applied_at } => {
                    ProviderOverrideSource::Retargeted {
                        applied_at: applied_at.clone(),
                    }
                }
            };
            provenance.apply_override(path, source);
        }

        let current_target = &self.target.path;
        let last_recorded_path = provenance
            .overrides()
            .last()
            .map(|override_entry| override_entry.path())
            .unwrap_or_else(|| provenance.default_path());

        if !paths_equivalent(last_recorded_path, current_target) {
            provenance.apply_override(current_target.clone(), ProviderOverrideSource::Preference);
        }

        self.provenance = provenance;
    }
}

#[derive(Clone, Copy)]
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

    fn plural_label(&self) -> &'static str {
        match self {
            ProviderRole::Share => "share targets",
            ProviderRole::Upload => "upload targets",
        }
    }

    fn label(&self) -> &'static str {
        match self {
            ProviderRole::Share => "share target",
            ProviderRole::Upload => "upload target",
        }
    }
}

impl From<ProviderKind> for ProviderRole {
    fn from(kind: ProviderKind) -> Self {
        match kind {
            ProviderKind::Share => ProviderRole::Share,
            ProviderKind::Upload => ProviderRole::Upload,
        }
    }
}

#[derive(Default, Debug)]
struct RestoreAllDefaultsResult {
    share_total: usize,
    share_restored: usize,
    upload_total: usize,
    upload_restored: usize,
}

impl RestoreAllDefaultsResult {
    fn restored(&self) -> usize {
        self.share_restored + self.upload_restored
    }

    fn total(&self) -> usize {
        self.share_total + self.upload_total
    }

    fn unchanged(&self) -> usize {
        self.total().saturating_sub(self.restored())
    }

    fn feedback_message(&self) -> String {
        let mut sentences = Vec::new();
        if let Some(sentence) = self.role_sentence(ProviderRole::Share) {
            sentences.push(sentence);
        }
        if let Some(sentence) = self.role_sentence(ProviderRole::Upload) {
            sentences.push(sentence);
        }

        if self.total() > 0 {
            if self.restored() == 0 {
                sentences.push("Cleared saved overrides for all first-party providers.".into());
            } else if self.unchanged() > 0 {
                sentences.push("Cleared saved overrides for the remaining targets.".into());
            } else {
                sentences.push("Saved overrides are now reset.".into());
            }
        }

        sentences.join(" ")
    }

    fn role_sentence(&self, role: ProviderRole) -> Option<String> {
        let (restored, total) = match role {
            ProviderRole::Share => (self.share_restored, self.share_total),
            ProviderRole::Upload => (self.upload_restored, self.upload_total),
        };
        if total == 0 {
            return None;
        }
        let label = role.plural_label();
        if restored == total {
            Some(format!("Restored all {label} to default paths."))
        } else if restored == 0 {
            Some(format!(
                "{} already use default paths.",
                capitalize_first(label)
            ))
        } else {
            let remaining = total - restored;
            Some(format!(
                "Restored {restored} of {total} {label} to default paths; {remaining} already used defaults."
            ))
        }
    }
}

struct ProviderRetargetOutcome {
    name: String,
    previous_path: PathBuf,
    new_path: PathBuf,
    changed: bool,
    applied_at: String,
}

struct RoleRetargetOutcome {
    role: ProviderRole,
    base: PathBuf,
    outcomes: Vec<ProviderRetargetOutcome>,
}

fn capitalize_first(input: &str) -> String {
    let mut chars = input.chars();
    match chars.next() {
        Some(first) => {
            let mut result = String::with_capacity(input.len());
            result.push(first.to_ascii_uppercase());
            result.push_str(chars.as_str());
            result
        }
        None => String::new(),
    }
}

fn sanitize_directory_component(name: &str) -> OsString {
    let sanitized: String = name
        .chars()
        .map(|ch| match ch {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            ch if ch.is_control() => '_',
            other => other,
        })
        .collect();
    let trimmed = sanitized.trim_matches(|ch: char| ch == ' ' || ch == '.');
    if trimmed.is_empty() {
        OsString::from("provider")
    } else {
        OsString::from(trimmed)
    }
}

fn bulk_retarget_feedback(selections: &[RoleRetargetOutcome]) -> String {
    if selections.is_empty() {
        return "No first-party providers were retargeted.".into();
    }

    let mut sentences = Vec::new();
    for selection in selections {
        let total = selection.outcomes.len();
        if total == 0 {
            continue;
        }
        let changed = selection
            .outcomes
            .iter()
            .filter(|outcome| outcome.changed)
            .count();
        let label = selection.role.plural_label();
        let sentence = if changed == 0 {
            format!(
                "{} already use directories under {}.",
                capitalize_first(label),
                selection.base.display()
            )
        } else if changed == total {
            format!(
                "Retargeted all {} under {}.",
                label,
                selection.base.display()
            )
        } else {
            let unchanged = total - changed;
            format!(
                "Retargeted {changed} of {total} {label} under {}; {unchanged} already pointed there.",
                selection.base.display()
            )
        };
        sentences.push(sentence);
    }

    if sentences.is_empty() {
        "No first-party providers were retargeted.".into()
    } else {
        sentences.join(" ")
    }
}

fn bulk_retarget_summary(
    selections: &[RoleRetargetOutcome],
    logs_dir: &Path,
) -> Option<ProviderBulkRetargetSummary> {
    let mut summary = ProviderBulkRetargetSummary::default();
    for selection in selections {
        if selection.outcomes.is_empty() {
            continue;
        }
        let first_applied = match selection.outcomes.first() {
            Some(outcome) => outcome.applied_at.clone(),
            None => continue,
        };
        let retargeted_at = selection
            .outcomes
            .iter()
            .map(|outcome| outcome.applied_at.as_str())
            .max()
            .map(|value| value.to_string())
            .unwrap_or(first_applied);
        let changed = selection
            .outcomes
            .iter()
            .filter(|outcome| outcome.changed)
            .count();
        let record = ProviderBulkRetargetRecord {
            base_path: relative_to_logs(&selection.base, logs_dir),
            retargeted_at,
            total: selection.outcomes.len(),
            changed,
        };
        match selection.role {
            ProviderRole::Share => summary.share.push(record),
            ProviderRole::Upload => summary.upload.push(record),
        }
    }
    if summary.is_empty() {
        None
    } else {
        Some(summary)
    }
}

fn relative_to_logs(path: &Path, logs_dir: &Path) -> String {
    path.strip_prefix(logs_dir)
        .map(|relative| relative.display().to_string())
        .unwrap_or_else(|_| path.display().to_string())
}

struct FirstPartyProviders {
    share: Vec<ProviderTargetState>,
    upload: Vec<ProviderTargetState>,
}

impl FirstPartyProviders {
    fn discover(paths: &AppPaths, preferences: &LauncherPreferences) -> Self {
        let mut share = Self::parse_targets(
            SHARE_PROVIDERS_ENV,
            "Support Share Drop",
            paths.logs_dir().join("support-share"),
        )
        .into_iter()
        .map(ProviderTargetState::new)
        .collect::<Vec<_>>();
        Self::apply_overrides(&mut share, ProviderRole::Share, preferences);
        let mut upload = Self::parse_targets(
            UPLOAD_PROVIDERS_ENV,
            "Support Upload Drop",
            paths.logs_dir().join("support-upload"),
        )
        .into_iter()
        .map(ProviderTargetState::new)
        .collect::<Vec<_>>();
        Self::apply_overrides(&mut upload, ProviderRole::Upload, preferences);
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

    fn apply_overrides(
        states: &mut [ProviderTargetState],
        role: ProviderRole,
        preferences: &LauncherPreferences,
    ) {
        for state in states {
            if let Some(path) =
                preferences.provider_override_path(role.as_str(), &state.target.name)
            {
                state.apply_override(path, ProviderOverrideSource::Preference);
            }
        }
    }

    fn hydrate_from_snapshot(&mut self, logs_dir: &Path, snapshot: &ProviderAutomationSnapshot) {
        self.reset_automation();
        for record in &snapshot.share {
            let resolved = resolve_snapshot_path(logs_dir, &record.path);
            if let Some(state) = find_state_mut(&mut self.share, |state| {
                paths_equivalent(&state.target.path, &resolved)
            }) {
                Self::apply_snapshot_record(state, logs_dir, record, &resolved, true);
                continue;
            }
            if let Some(state) =
                find_state_mut(&mut self.share, |state| state.target.name == record.name)
            {
                Self::apply_snapshot_record(state, logs_dir, record, &resolved, false);
            }
        }
        for record in &snapshot.upload {
            let resolved = resolve_snapshot_path(logs_dir, &record.path);
            if let Some(state) = find_state_mut(&mut self.upload, |state| {
                paths_equivalent(&state.target.path, &resolved)
            }) {
                Self::apply_snapshot_record(state, logs_dir, record, &resolved, true);
                continue;
            }
            if let Some(state) =
                find_state_mut(&mut self.upload, |state| state.target.name == record.name)
            {
                Self::apply_snapshot_record(state, logs_dir, record, &resolved, false);
            }
        }
    }

    fn apply_snapshot_record(
        state: &mut ProviderTargetState,
        logs_dir: &Path,
        record: &ProviderAutomationRecord,
        resolved: &Path,
        matched_by_path: bool,
    ) {
        state.sync_provenance_from_record(logs_dir, record);

        if matched_by_path && !paths_equivalent(&state.target.path, resolved) {
            state.target.path = resolved.to_path_buf();
        }

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
        filter: Option<&[usize]>,
    ) -> (ProviderStageReport, ProviderAutomationReport) {
        let mut stage_report = ProviderStageReport::default();
        let mut automation_report = ProviderAutomationReport::default();
        for (index, state) in self.share.iter_mut().enumerate() {
            if let Some(indices) = filter {
                if !indices.contains(&index) {
                    continue;
                }
            }
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
        filter: Option<&[usize]>,
    ) -> (ProviderStageReport, ProviderAutomationReport) {
        let mut stage_report = ProviderStageReport::default();
        let mut automation_report = ProviderAutomationReport::default();
        for (index, state) in self.upload.iter_mut().enumerate() {
            if let Some(indices) = filter {
                if !indices.contains(&index) {
                    continue;
                }
            }
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

    fn provider_name(&self, role: ProviderRole, index: usize) -> Option<&str> {
        self.states(role)
            .get(index)
            .map(|state| state.target.name.as_str())
    }

    fn provider_path(&self, role: ProviderRole, index: usize) -> Option<&Path> {
        self.states(role)
            .get(index)
            .map(|state| state.target.path.as_path())
    }

    fn provider_default_path(&self, role: ProviderRole, index: usize) -> Option<&Path> {
        self.states(role)
            .get(index)
            .map(|state| state.provenance.default_path())
    }

    fn retarget_all_to_base(
        &mut self,
        role: ProviderRole,
        base: &Path,
    ) -> Vec<ProviderRetargetOutcome> {
        let states = self.states_mut(role);
        let mut outcomes = Vec::with_capacity(states.len());
        for state in states {
            let previous = state.target.path.clone();
            let leaf = Self::directory_leaf(state);
            let destination = base.join(&leaf);
            let changed = previous != destination && !paths_equivalent(&previous, &destination);
            let applied_at = timestamp_for_log();
            state.apply_override(
                destination.clone(),
                ProviderOverrideSource::Retargeted {
                    applied_at: applied_at.clone(),
                },
            );
            outcomes.push(ProviderRetargetOutcome {
                name: state.target.name.clone(),
                previous_path: previous,
                new_path: destination,
                changed,
                applied_at,
            });
        }
        outcomes
    }

    fn bulk_dialog_hint(&self, role: ProviderRole) -> Option<PathBuf> {
        self.states(role)
            .first()
            .and_then(|state| state.target.path.parent())
            .map(|parent| parent.to_path_buf())
    }

    fn retarget_provider(&mut self, role: ProviderRole, index: usize, path: PathBuf) -> Result<()> {
        let states = self.states_mut(role);
        let state = states
            .get_mut(index)
            .ok_or_else(|| anyhow!("invalid {} provider index {}", role.as_str(), index))?;
        let applied_at = timestamp_for_log();
        state.apply_override(path, ProviderOverrideSource::Retargeted { applied_at });
        Ok(())
    }

    fn restore_default(&mut self, role: ProviderRole, index: usize) -> Result<bool> {
        let states = self.states_mut(role);
        let state = states
            .get_mut(index)
            .ok_or_else(|| anyhow!("invalid {} provider index {}", role.as_str(), index))?;
        state.provenance.remove_preference_overrides();
        let default_path = state.provenance.default_path().to_path_buf();
        let previous_path = state.target.path.clone();
        let path_changed = !paths_equivalent(&previous_path, &default_path);
        let applied_at = timestamp_for_log();
        state.apply_override(
            default_path,
            ProviderOverrideSource::Retargeted { applied_at },
        );
        Ok(path_changed)
    }

    fn restore_all_defaults(&mut self) -> Result<RestoreAllDefaultsResult> {
        let share_total = self.share.len();
        let upload_total = self.upload.len();

        let mut result = RestoreAllDefaultsResult {
            share_total,
            upload_total,
            ..RestoreAllDefaultsResult::default()
        };

        for index in 0..share_total {
            if self
                .restore_default(ProviderRole::Share, index)
                .context("failed to restore share provider default path")?
            {
                result.share_restored += 1;
            }
        }

        for index in 0..upload_total {
            if self
                .restore_default(ProviderRole::Upload, index)
                .context("failed to restore upload provider default path")?
            {
                result.upload_restored += 1;
            }
        }

        Ok(result)
    }

    fn states(&self, role: ProviderRole) -> &[ProviderTargetState] {
        match role {
            ProviderRole::Share => &self.share,
            ProviderRole::Upload => &self.upload,
        }
    }

    fn states_mut(&mut self, role: ProviderRole) -> &mut [ProviderTargetState] {
        match role {
            ProviderRole::Share => &mut self.share,
            ProviderRole::Upload => &mut self.upload,
        }
    }

    fn directory_leaf(state: &ProviderTargetState) -> OsString {
        if let Some(component) = state.target.path.file_name() {
            if !component.is_empty() {
                return component.to_os_string();
            }
        }
        sanitize_directory_component(&state.target.name)
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

fn find_state_mut<F>(
    states: &mut [ProviderTargetState],
    mut predicate: F,
) -> Option<&mut ProviderTargetState>
where
    F: FnMut(&ProviderTargetState) -> bool,
{
    states.iter_mut().find(|state| predicate(state))
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

/// Rasterises logical draw commands into the physical framebuffer.
///
/// Every command carries logical geometry; `scale_factor` is the drawing
/// transform that turns it into device pixels. Text is rasterised at the
/// scaled size rather than magnified afterwards, so glyphs stay crisp on a
/// high-DPI panel.
fn render_commands(
    surface: &mut Surface,
    commands: &[DrawCommand],
    scale_factor: f32,
    font: &dyn TextFont,
) {
    let scale = normalize_scale(scale_factor);
    for command in commands {
        match command {
            DrawCommand::Quad { rect, color } => {
                fill_rect(surface, &scaled_rect(rect, scale), *color)
            }
            DrawCommand::Text {
                rect,
                text,
                color,
                font_size,
                padding,
            } => draw_text(
                surface,
                &scaled_rect(rect, scale),
                text,
                *color,
                font_size * scale,
                padding * scale,
                font,
            ),
            DrawCommand::Image { rect, image } => {
                draw_image(surface, &scaled_rect(rect, scale), image)
            }
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

fn draw_image(surface: &mut Surface, rect: &GuiRect, image: &ImageData) {
    if rect.size.width <= 0.0 || rect.size.height <= 0.0 {
        return;
    }

    let dest_width = rect.size.width.max(1.0).round() as u32;
    let dest_height = rect.size.height.max(1.0).round() as u32;
    if dest_width == 0 || dest_height == 0 || image.width() == 0 || image.height() == 0 {
        return;
    }

    let dest_x = rect.origin.x.round() as i32;
    let dest_y = rect.origin.y.round() as i32;

    if dest_width == image.width() && dest_height == image.height() {
        if let Ok(src_surface) = Surface::from_bytes(
            image.width(),
            image.height(),
            PixelFormat::Rgba8888,
            image.pixels().to_vec(),
        ) {
            let _ = surface.blit(&src_surface, Point::new(dest_x, dest_y));
        }
        return;
    }

    let bounds = surface.bounds();
    let src_width = image.width();
    let src_height = image.height();
    let pixels = image.pixels();

    for dy in 0..dest_height {
        let target_y = dest_y + dy as i32;
        if target_y < bounds.y || target_y >= bounds.y + bounds.height as i32 {
            continue;
        }

        let src_y = ((dy as f32 / dest_height as f32) * src_height as f32)
            .floor()
            .clamp(0.0, (src_height - 1) as f32) as u32;

        for dx in 0..dest_width {
            let target_x = dest_x + dx as i32;
            if target_x < bounds.x || target_x >= bounds.x + bounds.width as i32 {
                continue;
            }

            let src_x = ((dx as f32 / dest_width as f32) * src_width as f32)
                .floor()
                .clamp(0.0, (src_width - 1) as f32) as u32;
            let idx = ((src_y * src_width + src_x) * 4) as usize;
            if idx + 3 >= pixels.len() {
                continue;
            }

            let color = Color::new(
                pixels[idx],
                pixels[idx + 1],
                pixels[idx + 2],
                pixels[idx + 3],
            );

            if color.a == 0 {
                continue;
            }

            let blended = if color.a == 255 {
                color
            } else {
                let background = surface
                    .get_pixel(target_x as u32, target_y as u32)
                    .unwrap_or_default();
                blend_colors(color, background)
            };

            let _ = surface.set_pixel(target_x as u32, target_y as u32, blended);
        }
    }
}

fn blend_colors(foreground: Color, background: Color) -> Color {
    if foreground.a == 0 {
        return background;
    }
    if foreground.a == 255 {
        return foreground;
    }

    let alpha = foreground.a as u16;
    let inv_alpha = 255u16 - alpha;
    let blend_channel =
        |fg: u8, bg: u8| -> u8 { ((fg as u16 * alpha + bg as u16 * inv_alpha) / 255) as u8 };
    let blended_alpha = alpha + (background.a as u16 * inv_alpha) / 255;

    Color::new(
        blend_channel(foreground.r, background.r),
        blend_channel(foreground.g, background.g),
        blend_channel(foreground.b, background.b),
        blended_alpha.min(255) as u8,
    )
}

fn draw_text(
    surface: &mut Surface,
    rect: &GuiRect,
    text: &str,
    color: Color,
    font_size: f32,
    padding: f32,
    font: &dyn TextFont,
) {
    let origin_x = rect.origin.x + padding;
    let origin_y = rect.origin.y + padding;
    font.draw_text(surface, origin_x, origin_y, text, font_size.max(1.0), color);
}

fn map_key_code(code: &Key) -> Option<KeyCode> {
    match code {
        Key::Named(NamedKey::Enter) => Some(KeyCode::Enter),
        Key::Named(NamedKey::Escape) => Some(KeyCode::Escape),
        Key::Named(NamedKey::Space) => Some(KeyCode::Space),
        Key::Named(NamedKey::Tab) => Some(KeyCode::Tab),
        Key::Named(NamedKey::ArrowUp) => Some(KeyCode::Up),
        Key::Named(NamedKey::ArrowDown) => Some(KeyCode::Down),
        Key::Named(NamedKey::ArrowLeft) => Some(KeyCode::Left),
        Key::Named(NamedKey::ArrowRight) => Some(KeyCode::Right),
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

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result as AnyResult;
    use clonk_launcher::{
        LauncherPreferences, ProviderAutomationSnapshot, ProviderAutomationState,
        ProviderBulkRetargetRecord, ProviderBulkRetargetSummary, ProviderDiagnostics,
        ProviderOverrideSource, ProviderPathStatus, ProviderStatus, SupportArtifact,
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

    // The launcher window used to be built with no icon at all, on every
    // platform, so it showed whatever default the window manager hands out.
    #[test]
    fn the_launcher_window_carries_the_product_icon() {
        assert!(
            window_icon().is_some(),
            "winit rejected the launcher's product icon"
        );
    }

    #[test]
    fn named_winit_keys_keep_the_launcher_gui_mapping() {
        let mappings = [
            (NamedKey::Enter, KeyCode::Enter),
            (NamedKey::Escape, KeyCode::Escape),
            (NamedKey::Space, KeyCode::Space),
            (NamedKey::Tab, KeyCode::Tab),
            (NamedKey::ArrowUp, KeyCode::Up),
            (NamedKey::ArrowDown, KeyCode::Down),
            (NamedKey::ArrowLeft, KeyCode::Left),
            (NamedKey::ArrowRight, KeyCode::Right),
        ];
        for (winit_key, gui_key) in mappings {
            assert_eq!(map_key_code(&Key::Named(winit_key)), Some(gui_key));
        }
        assert_eq!(map_key_code(&Key::Character("x".into())), None);
    }

    #[test]
    fn ime_is_allowed_only_for_focused_report_search_editing() {
        assert!(!should_enable_report_search_ime(false, false));
        assert!(!should_enable_report_search_ime(false, true));
        assert!(!should_enable_report_search_ime(true, false));
        assert!(should_enable_report_search_ime(true, true));
    }

    #[test]
    fn report_search_key_text_accepts_composition_but_rejects_shortcuts() {
        for (modifiers, expected) in [
            (ModifiersState::empty(), true),
            (ModifiersState::SHIFT, true),
            (ModifiersState::ALT, true),
            (ModifiersState::ALT | ModifiersState::SHIFT, true),
            (ModifiersState::CONTROL, false),
            (ModifiersState::CONTROL | ModifiersState::SHIFT, false),
            (ModifiersState::CONTROL | ModifiersState::ALT, true),
            (
                ModifiersState::CONTROL | ModifiersState::ALT | ModifiersState::SHIFT,
                true,
            ),
            (ModifiersState::SUPER, false),
            (ModifiersState::SUPER | ModifiersState::ALT, false),
            (
                ModifiersState::SUPER | ModifiersState::CONTROL | ModifiersState::ALT,
                false,
            ),
        ] {
            assert_eq!(report_search_key_text_allowed(modifiers), expected);
        }
    }

    #[test]
    fn finish_returns_a_retained_runtime_error() {
        let shell = LauncherShell {
            fatal_error: Some(anyhow!("failed to swap launcher buffers")),
            ..LauncherShell::default()
        };

        let error = shell
            .finish()
            .expect_err("the runtime error must survive event-loop shutdown");
        assert_eq!(error.to_string(), "failed to swap launcher buffers");
    }

    #[test]
    fn only_a_lost_launcher_surface_rebuilds_its_framebuffer() {
        assert_eq!(
            launcher_present_recovery(&pixels::Error::SurfaceLost),
            LauncherPresentRecovery::RebuildFramebuffer
        );
        assert_eq!(
            launcher_present_recovery(&pixels::Error::Validation),
            LauncherPresentRecovery::Report
        );
    }

    #[test]
    fn launcher_builds_a_replacement_only_after_dropping_the_previous_surface() {
        struct DropProbe<'a>(&'a std::cell::Cell<bool>);

        impl Drop for DropProbe<'_> {
            fn drop(&mut self) {
                self.0.set(true);
            }
        }

        let previous_dropped = std::cell::Cell::new(false);
        let replacement_dropped = std::cell::Cell::new(false);
        let mut surface = Some(DropProbe(&previous_dropped));
        replace_after_drop(&mut surface, || {
            assert!(previous_dropped.get());
            Ok::<_, std::convert::Infallible>(DropProbe(&replacement_dropped))
        })
        .expect("replace the launcher surface");

        assert!(surface.is_some());
        assert!(!replacement_dropped.get());

        let failed_dropped = std::cell::Cell::new(false);
        let mut failed_surface = Some(DropProbe(&failed_dropped));
        let result = replace_after_drop(&mut failed_surface, || {
            Err::<DropProbe<'_>, _>("replacement failed")
        });
        assert_eq!(result, Err("replacement failed"));
        assert!(failed_dropped.get());
        assert!(failed_surface.is_none());
    }

    #[test]
    fn launcher_requires_a_presented_frame_before_another_surface_rebuild() {
        let mut recovery = SurfaceRebuildState::default();
        assert_eq!(recovery.note_loss(), SurfaceRebuildSchedule::Immediate);
        assert_eq!(
            recovery.note_loss(),
            SurfaceRebuildSchedule::Cadenced,
            "a skipped frame must not replenish the prompt redraw"
        );
        recovery.note_presented();
        assert_eq!(recovery.note_loss(), SurfaceRebuildSchedule::Immediate);
    }

    #[test]
    fn launcher_distinguishes_a_skipped_surface_acquisition_from_presentation() {
        assert_eq!(
            launcher_present_outcome(false),
            LauncherPresentOutcome::Skipped
        );
        assert_eq!(
            launcher_present_outcome(true),
            LauncherPresentOutcome::Presented
        );
    }

    #[test]
    fn launcher_waits_before_a_cadenced_surface_retry() {
        let now = Instant::now();
        let retry_at = now + LOST_SURFACE_RETRY_DELAY;
        assert_eq!(
            launcher_redraw_action(now, Some(retry_at)),
            LauncherRedrawAction::WaitUntil(retry_at)
        );
        assert_eq!(
            launcher_redraw_action(retry_at, Some(retry_at)),
            LauncherRedrawAction::Request
        );
        assert_eq!(
            launcher_redraw_action(now, None),
            LauncherRedrawAction::Request
        );
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
        let (stage_report, automation_report) = providers.stage_bundle(&logger, &bundle_path, None);
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
        let (stage_report, automation_report) =
            providers.stage_artifacts(&logger, &artifacts, None);
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
    fn retarget_provider_updates_path_and_resets_state() {
        let initial_dir = TempDir::new().unwrap();
        let new_dir = TempDir::new().unwrap();

        let mut providers = FirstPartyProviders {
            share: vec![ProviderTargetState::new(ProviderTarget {
                name: "Support Share Drop".into(),
                path: initial_dir.path().to_path_buf(),
            })],
            upload: Vec::new(),
        };
        providers.share[0].automation = ProviderAutomationState::Submitted {
            detail: "submission-request-share-1.json".into(),
        };

        providers
            .retarget_provider(ProviderRole::Share, 0, new_dir.path().to_path_buf())
            .expect("retarget succeeds");

        assert_eq!(providers.share[0].target.path, new_dir.path());
        assert_eq!(providers.share[0].automation, ProviderAutomationState::Idle);
    }

    #[test]
    fn retarget_all_to_base_updates_all_providers() {
        let initial_root = TempDir::new().unwrap();
        let share_initial = initial_root.path().join("share-initial");
        let upload_initial = initial_root.path().join("upload-initial");

        let mut providers = FirstPartyProviders {
            share: vec![ProviderTargetState::new(ProviderTarget {
                name: "Support Share Drop".into(),
                path: share_initial.clone(),
            })],
            upload: vec![ProviderTargetState::new(ProviderTarget {
                name: "Support Upload Drop".into(),
                path: upload_initial.clone(),
            })],
        };
        providers.share[0].automation = ProviderAutomationState::Submitted {
            detail: "submission-request-share-1.json".into(),
        };
        providers.upload[0].automation = ProviderAutomationState::Failed {
            error: "network".into(),
        };

        let destination_root = TempDir::new().unwrap();
        let share_base = destination_root.path().join("new-share-base");
        let upload_base = destination_root.path().join("new-upload-base");

        let share_outcomes = providers.retarget_all_to_base(ProviderRole::Share, &share_base);
        assert_eq!(share_outcomes.len(), 1);
        let share_outcome = &share_outcomes[0];
        assert!(share_outcome.changed);
        assert_eq!(share_outcome.previous_path, share_initial);
        assert_eq!(share_outcome.new_path, share_base.join("share-initial"));
        assert_eq!(providers.share[0].target.path, share_outcome.new_path);
        assert!(matches!(
            providers.share[0].automation,
            ProviderAutomationState::Idle
        ));

        let upload_outcomes = providers.retarget_all_to_base(ProviderRole::Upload, &upload_base);
        assert_eq!(upload_outcomes.len(), 1);
        let upload_outcome = &upload_outcomes[0];
        assert!(upload_outcome.changed);
        assert_eq!(upload_outcome.previous_path, upload_initial);
        assert_eq!(upload_outcome.new_path, upload_base.join("upload-initial"));
        assert_eq!(providers.upload[0].target.path, upload_outcome.new_path);
        assert!(matches!(
            providers.upload[0].automation,
            ProviderAutomationState::Idle
        ));
    }

    #[test]
    fn restore_default_resets_path_and_prunes_preference_overrides() {
        let default_dir = TempDir::new().unwrap();
        let override_dir = TempDir::new().unwrap();

        let mut providers = FirstPartyProviders {
            share: vec![ProviderTargetState::new(ProviderTarget {
                name: "Support Share Drop".into(),
                path: default_dir.path().to_path_buf(),
            })],
            upload: Vec::new(),
        };
        providers.share[0].apply_override(
            override_dir.path().to_path_buf(),
            ProviderOverrideSource::Preference,
        );

        let changed = providers
            .restore_default(ProviderRole::Share, 0)
            .expect("restore default succeeds");
        assert!(changed, "expected restore_default to report path change");

        let state = &providers.share[0];
        assert!(
            paths_equivalent(&state.target.path, default_dir.path()),
            "expected provider path to revert to default"
        );
        assert!(
            !state.provenance.has_preference_override(),
            "preference overrides should be removed"
        );
        let last_override = state
            .provenance
            .overrides()
            .last()
            .expect("expected default override entry");
        assert!(
            paths_equivalent(last_override.path(), default_dir.path()),
            "expected default override path"
        );
        assert!(
            matches!(
                last_override.source(),
                ProviderOverrideSource::Retargeted { .. }
            ),
            "expected default override to be recorded as retargeted"
        );
    }

    #[test]
    fn restore_default_reports_no_change_when_already_default() {
        let default_dir = TempDir::new().unwrap();

        let mut providers = FirstPartyProviders {
            share: vec![ProviderTargetState::new(ProviderTarget {
                name: "Support Share Drop".into(),
                path: default_dir.path().to_path_buf(),
            })],
            upload: Vec::new(),
        };
        providers.share[0].apply_override(
            default_dir.path().to_path_buf(),
            ProviderOverrideSource::Preference,
        );

        let changed = providers
            .restore_default(ProviderRole::Share, 0)
            .expect("restore default succeeds");
        assert!(
            !changed,
            "expected restore_default to report no path change when already default"
        );
        assert!(
            !providers.share[0].provenance.has_preference_override(),
            "preference overrides should be cleared even when no path change occurs"
        );
    }

    #[test]
    fn restore_all_defaults_resets_all_providers() {
        let share_default = TempDir::new().unwrap();
        let share_override = TempDir::new().unwrap();
        let upload_default = TempDir::new().unwrap();
        let upload_override = TempDir::new().unwrap();

        let mut providers = FirstPartyProviders {
            share: vec![ProviderTargetState::new(ProviderTarget {
                name: "Support Share Drop".into(),
                path: share_default.path().to_path_buf(),
            })],
            upload: vec![ProviderTargetState::new(ProviderTarget {
                name: "Support Upload Drop".into(),
                path: upload_default.path().to_path_buf(),
            })],
        };
        providers.share[0].apply_override(
            share_override.path().to_path_buf(),
            ProviderOverrideSource::Preference,
        );
        providers.upload[0].apply_override(
            upload_override.path().to_path_buf(),
            ProviderOverrideSource::Preference,
        );

        let outcome = providers
            .restore_all_defaults()
            .expect("bulk default restoration");

        assert_eq!(outcome.share_total, 1);
        assert_eq!(outcome.upload_total, 1);
        assert_eq!(outcome.share_restored, 1);
        assert_eq!(outcome.upload_restored, 1);

        let share_state = &providers.share[0];
        assert!(
            paths_equivalent(&share_state.target.path, share_default.path()),
            "share provider should reset to default path"
        );
        assert!(
            !share_state.provenance.has_preference_override(),
            "share provider should drop preference overrides"
        );

        let upload_state = &providers.upload[0];
        assert!(
            paths_equivalent(&upload_state.target.path, upload_default.path()),
            "upload provider should reset to default path"
        );
        assert!(
            !upload_state.provenance.has_preference_override(),
            "upload provider should drop preference overrides"
        );
    }

    #[test]
    fn restore_all_defaults_reports_no_changes_when_paths_already_default() {
        let share_default = TempDir::new().unwrap();
        let upload_default = TempDir::new().unwrap();

        let mut providers = FirstPartyProviders {
            share: vec![ProviderTargetState::new(ProviderTarget {
                name: "Support Share Drop".into(),
                path: share_default.path().to_path_buf(),
            })],
            upload: vec![ProviderTargetState::new(ProviderTarget {
                name: "Support Upload Drop".into(),
                path: upload_default.path().to_path_buf(),
            })],
        };
        // Record preference overrides that point at defaults to ensure they are pruned.
        providers.share[0].apply_override(
            share_default.path().to_path_buf(),
            ProviderOverrideSource::Preference,
        );
        providers.upload[0].apply_override(
            upload_default.path().to_path_buf(),
            ProviderOverrideSource::Preference,
        );

        let outcome = providers
            .restore_all_defaults()
            .expect("bulk default restoration");

        assert_eq!(outcome.share_total, 1);
        assert_eq!(outcome.upload_total, 1);
        assert_eq!(outcome.share_restored, 0);
        assert_eq!(outcome.upload_restored, 0);

        assert!(
            !providers.share[0].provenance.has_preference_override(),
            "share preference overrides should be cleared"
        );
        assert!(
            !providers.upload[0].provenance.has_preference_override(),
            "upload preference overrides should be cleared"
        );
    }

    #[test]
    fn prune_bulk_retarget_history_drops_records_when_role_defaults_restored() {
        let share_default = TempDir::new().unwrap();
        let upload_default = TempDir::new().unwrap();
        let upload_override = TempDir::new().unwrap();

        let share_state = ProviderTargetState::new(ProviderTarget {
            name: "Support Share Drop".into(),
            path: share_default.path().to_path_buf(),
        });
        let mut upload_state = ProviderTargetState::new(ProviderTarget {
            name: "Support Upload Drop".into(),
            path: upload_default.path().to_path_buf(),
        });
        upload_state.apply_override(
            upload_override.path().to_path_buf(),
            ProviderOverrideSource::Retargeted {
                applied_at: "2024-06-02T12:00:00Z".into(),
            },
        );

        let diagnostics = ProviderDiagnostics {
            share: vec![share_state.to_status()],
            upload: vec![upload_state.to_status()],
        };

        let mut summary = ProviderBulkRetargetSummary::default();
        summary.share.push(ProviderBulkRetargetRecord {
            base_path: "support-share".into(),
            retargeted_at: "2024-05-01T12:00:00Z".into(),
            total: 2,
            changed: 2,
        });
        summary.upload.push(ProviderBulkRetargetRecord {
            base_path: "support-upload".into(),
            retargeted_at: "2024-05-03T09:30:00Z".into(),
            total: 1,
            changed: 1,
        });

        let pruned =
            prune_bulk_retarget_history(Some(summary), &diagnostics).expect("upload should remain");
        assert!(
            pruned.share.is_empty(),
            "share history should be cleared once defaults are restored"
        );
        assert_eq!(
            pruned.upload.len(),
            1,
            "upload history should be preserved when overrides remain"
        );
        assert!(
            pruned.history_cleared_at.is_none(),
            "history cleared marker should not be recorded while records remain"
        );
    }

    #[test]
    fn prune_bulk_retarget_history_clears_summary_when_all_defaults_restored() {
        let share_default = TempDir::new().unwrap();
        let upload_default = TempDir::new().unwrap();

        let share_state = ProviderTargetState::new(ProviderTarget {
            name: "Support Share Drop".into(),
            path: share_default.path().to_path_buf(),
        });
        let upload_state = ProviderTargetState::new(ProviderTarget {
            name: "Support Upload Drop".into(),
            path: upload_default.path().to_path_buf(),
        });

        let diagnostics = ProviderDiagnostics {
            share: vec![share_state.to_status()],
            upload: vec![upload_state.to_status()],
        };

        let mut summary = ProviderBulkRetargetSummary::default();
        summary.share.push(ProviderBulkRetargetRecord {
            base_path: "support-share".into(),
            retargeted_at: "2024-05-01T12:00:00Z".into(),
            total: 2,
            changed: 2,
        });
        summary.upload.push(ProviderBulkRetargetRecord {
            base_path: "support-upload".into(),
            retargeted_at: "2024-05-03T09:30:00Z".into(),
            total: 1,
            changed: 1,
        });

        let pruned = prune_bulk_retarget_history(Some(summary), &diagnostics)
            .expect("summary should persist");
        assert!(
            pruned.share.is_empty() && pruned.upload.is_empty(),
            "all bulk retarget records should be cleared once defaults are restored"
        );
        assert!(
            pruned.history_cleared_at.is_some(),
            "history cleared marker should be recorded when defaults are restored"
        );
    }

    #[test]
    fn apply_overrides_updates_provider_paths() {
        let override_dir = TempDir::new().unwrap();
        let mut prefs = LauncherPreferences::default();
        prefs.set_provider_override("share", "Support Share Drop", override_dir.path());

        let mut states = vec![ProviderTargetState::new(ProviderTarget {
            name: "Support Share Drop".into(),
            path: PathBuf::from("/tmp/original"),
        })];

        FirstPartyProviders::apply_overrides(&mut states, ProviderRole::Share, &prefs);

        assert_eq!(states[0].target.path, override_dir.path());
        let override_entry = states[0]
            .provenance
            .overrides()
            .last()
            .expect("expected override to be recorded");
        assert!(matches!(
            override_entry.source(),
            ProviderOverrideSource::Preference
        ));
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
                path_provenance: ProviderPathProvenance::new(share_path.clone()),
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
                path_provenance: ProviderPathProvenance::new(original_path.clone()),
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
                path_provenance: ProviderPathProvenance::new(share_path.clone()),
            }],
            upload: vec![ProviderStatus {
                name: "Support Upload Drop".into(),
                path: upload_path.clone(),
                path_status: ProviderPathStatus::Ready,
                automation: ProviderAutomationState::Failed {
                    error: "network failure".into(),
                },
                path_provenance: ProviderPathProvenance::new(upload_path.clone()),
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

    #[test]
    fn hydrate_from_snapshot_restores_override_history() {
        let logs_dir = TempDir::new().unwrap();
        let default_path = logs_dir.path().join("support-share");
        let override_path = PathBuf::from("/tmp/custom-support-share");
        fs::create_dir_all(&default_path).unwrap();

        let mut providers = FirstPartyProviders {
            share: vec![ProviderTargetState::new(ProviderTarget {
                name: "Support Share Drop".into(),
                path: override_path.clone(),
            })],
            upload: Vec::new(),
        };

        let mut provenance = ProviderPathProvenance::new(default_path.clone());
        provenance.apply_override(
            override_path.clone(),
            ProviderOverrideSource::Retargeted {
                applied_at: "2024-05-01T12:34:56Z".into(),
            },
        );

        let diagnostics = ProviderDiagnostics {
            share: vec![ProviderStatus {
                name: "Support Share Drop".into(),
                path: override_path.clone(),
                path_status: ProviderPathStatus::Ready,
                automation: ProviderAutomationState::Submitted {
                    detail: "submission-request-share-override.json".into(),
                },
                path_provenance: provenance,
            }],
            upload: Vec::new(),
        };

        let snapshot = ProviderAutomationSnapshot::from_diagnostics(&diagnostics, logs_dir.path());
        providers.reset_automation();
        providers.hydrate_from_snapshot(logs_dir.path(), &snapshot);

        let overrides = providers.share[0].provenance.overrides();
        assert_eq!(overrides.len(), 1, "expected restored override entry");
        assert!(paths_equivalent(overrides[0].path(), &override_path));
        assert!(matches!(
            overrides[0].source(),
            ProviderOverrideSource::Retargeted { .. }
        ));
    }

    fn rect_contains(rect: &GuiRect, point: GuiPoint) -> bool {
        point.x >= rect.origin.x
            && point.y >= rect.origin.y
            && point.x <= rect.origin.x + rect.size.width
            && point.y <= rect.origin.y + rect.size.height
    }

    #[test]
    fn logical_extent_divides_physical_pixels_by_the_scale_factor() {
        // A 960x640 logical window on a 2x panel reports 1920x1280 physical
        // pixels from `Window::inner_size`; layout must still see 960x640.
        assert_eq!(logical_extent(1920, 1280, 2.0), (960.0, 640.0));
        assert_eq!(logical_extent(960, 640, 1.0), (960.0, 640.0));
        assert_eq!(logical_extent(1440, 960, 1.5), (960.0, 640.0));
        // A degenerate scale factor must never collapse the layout box.
        assert_eq!(logical_extent(960, 640, 0.0), (960.0, 640.0));
    }

    #[test]
    fn scaled_rect_maps_logical_geometry_onto_physical_pixels() {
        let logical = GuiRect::new(10.0, 20.0, 100.0, 30.0);
        let physical = scaled_rect(&logical, 2.0);
        assert_eq!(physical.origin.x, 20.0);
        assert_eq!(physical.origin.y, 40.0);
        assert_eq!(physical.size.width, 200.0);
        assert_eq!(physical.size.height, 60.0);
    }

    #[test]
    fn pointer_positions_land_inside_the_widget_painted_at_two_times_scale() {
        let scale = 2.0f32;
        let (logical_width, logical_height) = logical_extent(1920, 1280, scale);
        let paths = AppPaths::discover().expect("app paths");
        let localization = load_localization(&paths).expect("localization");
        let mut ui = LauncherShellUi::new(None, localization).expect("launcher ui");
        ui.layout(GuiSize::new(logical_width, logical_height));

        let button = ui.regenerate_button().expect("regenerate button");
        let logical_rect = ui.widget_rect(button).expect("regenerate rect");
        let painted = scaled_rect(&logical_rect, scale);

        // The cursor arrives in physical pixels at the centre of what was
        // actually painted; mapping it back must land on the same widget.
        let centre_x = f64::from(painted.origin.x + painted.size.width / 2.0);
        let centre_y = f64::from(painted.origin.y + painted.size.height / 2.0);
        let mapped = physical_to_logical_point(centre_x, centre_y, scale);
        assert!(
            rect_contains(&logical_rect, mapped),
            "cursor {mapped:?} fell outside {logical_rect:?}"
        );

        // And the painted geometry must stay inside the physical framebuffer.
        assert!(painted.origin.x + painted.size.width <= 1920.0);
        assert!(painted.origin.y + painted.size.height <= 1280.0);
    }

    #[test]
    fn ui_font_uses_proportional_advances_when_endeavour_is_available() {
        let paths = AppPaths::discover().expect("app paths");
        let font = load_ui_font(&paths);
        // The 8x8 bitmap fallback gives every glyph the same advance; a real
        // vector face makes "iiii" narrower than "WWWW".
        let narrow = font.measure_text("iiii", 16.0).width;
        let wide = font.measure_text("WWWW", 16.0).width;
        assert!(
            narrow < wide,
            "expected proportional advances, got narrow={narrow} wide={wide}"
        );
    }
}
