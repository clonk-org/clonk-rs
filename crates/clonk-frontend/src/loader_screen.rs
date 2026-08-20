//! Exact, IO-free frontend for `C4LoaderScreen` (`src/C4LoaderScreen.cpp`).
//!
//! Loader discovery and the weighted random choice deliberately stay in the
//! application layer.  The frontend receives the effective specification,
//! chosen filename and already-decoded background as typed state.  Likewise,
//! this module never substitutes a bitmap font or a generic background: bad
//! resources are constructor errors.
//!
//! The C++ loader does **not** have a dedicated copyright, version or footer
//! label.  During startup the fan-project string is an ordinary log-buffer
//! line; a version string is not drawn by `C4LoaderScreen` at all.

use crate::clonk_fonts::{NativeClonkFont, NativeClonkFontSet};
use crate::{ClonkFontSet, ImageData};
use anyhow::{ensure, Result};
use clonk_graphics::clonk_font::{ClonkFont, TextAlign};
use clonk_graphics::{
    stdgl_blit_sampling, BlitSampling, ClipperProjection, Color, GammaRamp, Rect, Surface,
    SurfaceDrawTarget,
};
use std::{cell::RefCell, collections::HashMap, sync::Arc};

/// `C4CFN_StartupBackgroundMain` (`src/C4Startup.h`).
pub const STARTUP_LOADER_SPECIFICATION: &str = "LoaderGoldmine1";
/// The fallback specification selected by `C4LoaderScreen::Init` for an empty
/// scenario `Loader` entry.
pub const DEFAULT_LOADER_SPECIFICATION: &str = "Loader*";

const H_INDENT: i32 = 20;
const V_INDENT: i32 = 20;
const LOG_BOX_HEIGHT: i32 = 84;
const LOG_BOX_MARGIN: i32 = 2;
const V_MARGIN: i32 = 5;
const PROGRESS_BAR_HEIGHT: i32 = 15;

const TITLE_COLOR: [u8; 4] = [0xdd, 0xdd, 0xdd, 0xdd];
const WHITE: [u8; 4] = [0xff, 0xff, 0xff, 0xff];
const PROGRESS_FRAME_COLOR: u32 = 0x4f00_0000;
const LOG_BOX_COLOR: u32 = 0x7f00_0000;
const FALLBACK_PROGRESS_COLOR: u32 = 0x4fff_0000;

/// Texture filtering selected by `CStdGL::PerformBlt` for loader blits
/// (`StdGL.cpp:527-532`).
pub type LoaderSampling = BlitSampling;

/// Application-scale and point-filtering inputs to C++'s blit decision.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LoaderRenderConfig {
    application_scale: f32,
    point_filtering: bool,
    aspect_fill: bool,
}

impl LoaderRenderConfig {
    pub fn new(application_scale: f32, point_filtering: bool) -> Result<Self> {
        ensure!(
            application_scale.is_finite() && application_scale > 0.0,
            "classic loader application scale must be finite and positive"
        );
        Ok(Self {
            application_scale,
            point_filtering,
            aspect_fill: false,
        })
    }

    pub const fn scale_one(point_filtering: bool) -> Self {
        Self {
            application_scale: 1.0,
            point_filtering,
            aspect_fill: false,
        }
    }

    /// Opt-in `Graphics.LoaderAspect`: cover-fit the fullscreen loader image
    /// instead of C++'s unconditional non-aspect stretch. Off keeps the blit
    /// bit-identical to `C4Facet::DrawFullScreen` (`C4Facet.cpp:182-192`).
    pub const fn with_aspect_fill(mut self, aspect_fill: bool) -> Self {
        self.aspect_fill = aspect_fill;
        self
    }

    pub const fn application_scale(self) -> f32 {
        self.application_scale
    }

    pub const fn point_filtering(self) -> bool {
        self.point_filtering
    }

    pub const fn aspect_fill(self) -> bool {
        self.aspect_fill
    }

    pub const fn uses_scaling_correction(self) -> bool {
        self.application_scale != 1.0
    }

    /// `exact_blit` is C++'s per-call `fExact`: no transform and identical
    /// source-facet/target dimensions after scaling correction.
    pub const fn sampling(self, exact_blit: bool) -> LoaderSampling {
        stdgl_blit_sampling(self.application_scale, self.point_filtering, exact_blit)
    }
}

impl Default for LoaderRenderConfig {
    fn default() -> Self {
        // Config.Graphics.PointFiltering defaults off, so non-exact blits use
        // GL_LINEAR even at application scale one.
        Self::scale_one(false)
    }
}

fn logical_extent_for(physical_extent: u32, scale: f32) -> u32 {
    ((physical_extent as f32) / scale).ceil() as u32
}

/// CStdGL::UpdateClipper uses `ceil(logical * application_scale)` for the
/// nominal viewport. The real framebuffer may be smaller because
/// C4Application first rounded its logical resolution up.
fn native_viewport_extent(logical_extent: u32, scale: f32) -> Option<u32> {
    let scaled = logical_extent as f32 * scale;
    (scaled.is_finite() && scaled > 0.0 && scaled <= u32::MAX as f32).then(|| scaled.ceil() as u32)
}

/// Why the loader was selected.  Both cases use exactly the same renderer;
/// the distinction controls app-owned discovery and title updates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LoaderContext {
    /// Pre-startup initialization (`C4Application::PreInit`).
    Startup,
    /// A local, hosted or joined scenario is being initialized.
    Scenario,
}

/// The result of C++'s loader search, supplied by the application rather than
/// recomputed by the frontend.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LoaderSelection {
    context: LoaderContext,
    effective_specification: String,
    selected_filename: String,
}

impl LoaderSelection {
    /// Records the fixed startup background selection.  The application still
    /// supplies the concrete decoded image and filename so missing resources
    /// remain explicit errors.
    pub fn startup(selected_filename: impl Into<String>) -> Result<Self> {
        Self::new(
            LoaderContext::Startup,
            STARTUP_LOADER_SPECIFICATION,
            selected_filename,
        )
    }

    /// Records a scenario loader choice.  Like C++, an empty specification is
    /// normalized to `Loader*`; the weighted random search itself remains app
    /// owned.
    pub fn scenario(
        specification: impl Into<String>,
        selected_filename: impl Into<String>,
    ) -> Result<Self> {
        Self::new(LoaderContext::Scenario, specification, selected_filename)
    }

    fn new(
        context: LoaderContext,
        specification: impl Into<String>,
        selected_filename: impl Into<String>,
    ) -> Result<Self> {
        let specification = specification.into();
        let effective_specification = if specification.is_empty() {
            DEFAULT_LOADER_SPECIFICATION.to_owned()
        } else {
            specification
        };
        let selected_filename = selected_filename.into();
        ensure!(
            !selected_filename.is_empty(),
            "classic loader selection has no filename"
        );
        let extension = selected_filename
            .rsplit_once('.')
            .map(|(_, extension)| extension)
            .unwrap_or_default();
        ensure!(
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "bmp"
            ),
            "classic loader selection '{}' is not PNG, JPEG, or BMP",
            selected_filename
        );
        Ok(Self {
            context,
            effective_specification,
            selected_filename,
        })
    }

    pub fn context(&self) -> LoaderContext {
        self.context
    }

    pub fn effective_specification(&self) -> &str {
        &self.effective_specification
    }

    pub fn selected_filename(&self) -> &str {
        &self.selected_filename
    }
}

/// Whether C4GUI resources exist while the loader is drawn.  A valid GUI must
/// provide its progress image; `None` is accepted by the type only so the
/// constructor can return the same explicit missing-asset error as other
/// resource loaders. When the GUI itself is unavailable, C++ uses a solid red
/// fallback instead (`C4LoaderScreen.cpp:145-152`).
#[derive(Clone, Debug, PartialEq)]
pub enum LoaderGuiProgress {
    GuiValid { progress_bar: Option<ImageData> },
    GuiUnavailable,
}

/// Resources shared by every loader choice.  C++ derives the progress facet
/// from whatever GUIProgress image was loaded: `(1, 0, width-2, height)`.
#[derive(Clone)]
pub struct LoaderResources {
    fonts: Arc<ClonkFontSet>,
    gui_progress: LoaderGuiProgress,
}

impl LoaderResources {
    /// Convenience constructor for a valid GUI with a present progress image.
    pub fn new(fonts: Arc<ClonkFontSet>, progress_bar: ImageData) -> Result<Self> {
        Self::from_gui_state(
            fonts,
            LoaderGuiProgress::GuiValid {
                progress_bar: Some(progress_bar),
            },
        )
    }

    pub fn gui_unavailable(fonts: Arc<ClonkFontSet>) -> Result<Self> {
        Self::from_gui_state(fonts, LoaderGuiProgress::GuiUnavailable)
    }

    pub fn from_gui_state(
        fonts: Arc<ClonkFontSet>,
        gui_progress: LoaderGuiProgress,
    ) -> Result<Self> {
        validate_font_set(&fonts)?;
        if let LoaderGuiProgress::GuiValid { progress_bar } = &gui_progress {
            let progress_bar = progress_bar.as_ref().ok_or_else(|| {
                anyhow::anyhow!("classic loader GUI is valid but GUIProgress is missing")
            })?;
            validate_rgba_image("GUIProgress", progress_bar)?;
            ensure!(
                progress_bar.width() >= 3,
                "classic loader GUIProgress width must be at least 3, got {}",
                progress_bar.width()
            );
        }
        Ok(Self {
            fonts,
            gui_progress,
        })
    }

    pub fn fonts(&self) -> &Arc<ClonkFontSet> {
        &self.fonts
    }

    pub fn gui_progress(&self) -> &LoaderGuiProgress {
        &self.gui_progress
    }

    pub fn progress_bar(&self) -> Option<&ImageData> {
        match &self.gui_progress {
            LoaderGuiProgress::GuiValid { progress_bar } => progress_bar.as_ref(),
            LoaderGuiProgress::GuiUnavailable => None,
        }
    }
}

/// Whether `C4LoaderScreen::Draw` received a log-buffer pointer.  `Visible`
/// draws the 84px region even when the buffer has no lines; `Hidden` omits it
/// exactly like a null `pLog` during the loader's initial draw.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum LoaderLog {
    #[default]
    Hidden,
    Visible(Vec<String>),
}

/// Mutable app-owned values used for one loader frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoaderState {
    title: String,
    progress: i32,
    log: LoaderLog,
    process: Option<i32>,
}

impl LoaderState {
    /// C++'s first draw after `Init`: title and 0% progress, but no log box.
    pub fn initial(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            progress: 0,
            log: LoaderLog::Hidden,
            process: None,
        }
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn progress(&self) -> i32 {
        self.progress
    }

    pub fn log(&self) -> &LoaderLog {
        &self.log
    }

    pub fn process(&self) -> Option<i32> {
        self.process
    }
}

/// IO-free updates suitable for boot progress callbacks and scenario loading
/// callbacks.  Values intentionally are not clamped: C++ renders the raw
/// integer percentage and computes the fill width from it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoaderUpdate {
    SetTitle(String),
    SetProgress(i32),
    ShowLog,
    HideLog,
    ReplaceLog(Vec<String>),
    AppendLog(String),
    /// `None` or `Some(0)` hides the suffix, matching `if (Process)`.
    SetProcess(Option<i32>),
}

/// Inclusive C++ pixel geometry represented as origin plus pixel count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoaderRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// All fixed-position geometry from `C4LoaderScreen::Draw`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoaderLayout {
    /// Right-aligned TitleFont anchor.
    pub title_anchor: (i32, i32),
    /// Inclusive `DrawBoxDw` progress-frame rectangle.
    pub progress_frame: LoaderRect,
    /// `GUIProgress` destination. A non-positive width means no facet draw.
    pub progress_fill: LoaderRect,
    /// Center-aligned regular-font anchor.
    pub progress_text_anchor: (i32, i32),
    /// Inclusive `DrawBoxDw` log rectangle.
    pub log_box: LoaderRect,
    /// First TinyFont line origin.
    pub log_text_origin: (i32, i32),
}

/// Computes C++'s integer layout without imposing a minimum resolution.
pub fn loader_layout(
    width: i32,
    height: i32,
    title_line_height: i32,
    regular_line_height: i32,
    progress: i32,
) -> LoaderLayout {
    let progress_y = height - V_INDENT - LOG_BOX_HEIGHT - V_MARGIN - PROGRESS_BAR_HEIGHT;
    let progress_width = width - H_INDENT * 2 - 2;
    let fill_width = ((i64::from(progress_width) * i64::from(progress)) / 100)
        .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
    LoaderLayout {
        title_anchor: (
            width - H_INDENT,
            height
                - V_INDENT
                - LOG_BOX_HEIGHT
                - V_MARGIN
                - PROGRESS_BAR_HEIGHT
                - V_MARGIN
                - title_line_height,
        ),
        progress_frame: LoaderRect {
            x: H_INDENT,
            y: progress_y,
            // DrawBoxDw receives x2 = width - 20, inclusively.
            width: width - H_INDENT * 2 + 1,
            // y2-y1+1: the nominal 15px bar is a 16px inclusive box.
            height: PROGRESS_BAR_HEIGHT + 1,
        },
        progress_fill: LoaderRect {
            x: H_INDENT + 1,
            y: progress_y + 1,
            width: fill_width,
            height: PROGRESS_BAR_HEIGHT - 2,
        },
        progress_text_anchor: (
            width / 2,
            height
                - V_INDENT
                - LOG_BOX_HEIGHT
                - V_MARGIN
                - regular_line_height / 2
                - PROGRESS_BAR_HEIGHT / 2,
        ),
        log_box: LoaderRect {
            x: H_INDENT,
            y: height - V_INDENT - LOG_BOX_HEIGHT,
            width: width - H_INDENT * 2 + 1,
            // The C++ inclusive endpoint makes the nominal 84px box 85px.
            height: LOG_BOX_HEIGHT + 1,
        },
        log_text_origin: (
            H_INDENT + LOG_BOX_MARGIN,
            height - V_INDENT - LOG_BOX_HEIGHT + LOG_BOX_MARGIN,
        ),
    }
}

/// Reusable classic loader controller and renderer.  It owns no file IO and
/// performs no loader selection or animation.
pub struct LoaderScreen {
    selection: LoaderSelection,
    background: ImageData,
    resources: LoaderResources,
    state: LoaderState,
}

impl LoaderScreen {
    pub fn new(
        selection: LoaderSelection,
        background: ImageData,
        resources: LoaderResources,
        state: LoaderState,
    ) -> Result<Self> {
        validate_rgba_image("loader background", &background)?;
        Ok(Self {
            selection,
            background,
            resources,
            state,
        })
    }

    pub fn selection(&self) -> &LoaderSelection {
        &self.selection
    }

    pub fn state(&self) -> &LoaderState {
        &self.state
    }

    pub fn resources(&self) -> &LoaderResources {
        &self.resources
    }

    /// Installs a loader already chosen and decoded by the application.
    pub fn replace_loader(
        &mut self,
        selection: LoaderSelection,
        background: ImageData,
    ) -> Result<()> {
        validate_rgba_image("loader background", &background)?;
        self.selection = selection;
        self.background = background;
        Ok(())
    }

    /// Refreshes fonts/GUI resources after C++'s mid-load `InitFonts` /
    /// resource reload without disturbing the chosen loader or live values.
    pub fn replace_resources(&mut self, resources: LoaderResources) {
        self.resources = resources;
    }

    pub fn update(&mut self, update: LoaderUpdate) {
        match update {
            LoaderUpdate::SetTitle(title) => self.state.title = title,
            LoaderUpdate::SetProgress(progress) => self.state.progress = progress,
            LoaderUpdate::ShowLog => {
                if matches!(self.state.log, LoaderLog::Hidden) {
                    self.state.log = LoaderLog::Visible(Vec::new());
                }
            }
            LoaderUpdate::HideLog => self.state.log = LoaderLog::Hidden,
            LoaderUpdate::ReplaceLog(lines) => self.state.log = LoaderLog::Visible(lines),
            LoaderUpdate::AppendLog(line) => match &mut self.state.log {
                LoaderLog::Hidden => self.state.log = LoaderLog::Visible(vec![line]),
                LoaderLog::Visible(lines) => lines.push(line),
            },
            LoaderUpdate::SetProcess(process) => {
                self.state.process = process.filter(|process| *process != 0);
            }
        }
    }

    /// Scale-one convenience renderer. Multi-scale callers must render
    /// [`Self::render_chrome`] in logical pixels, upscale it, then call
    /// [`Self::render_native_text`] so glyphs are never bilinearly enlarged.
    pub fn render(&self, surface: &mut Surface, gamma: Option<&GammaRamp>) -> Result<()> {
        self.validate_render_text()?;
        self.render_chrome(surface, LoaderRenderConfig::default(), gamma)?;
        self.render_logical_text(surface, gamma)
    }

    /// Scale-one renderer with the caller's exact PointFiltering setting.
    /// Multi-scale callers still use the documented chrome/native-text split.
    pub fn render_with_config(
        &self,
        surface: &mut Surface,
        config: LoaderRenderConfig,
        gamma: Option<&GammaRamp>,
    ) -> Result<()> {
        ensure!(
            config.application_scale() == 1.0,
            "classic loader logical text may only be rendered at application scale one"
        );
        self.validate_render_text()?;
        self.render_chrome(surface, config, gamma)?;
        self.render_logical_text(surface, gamma)
    }

    /// Draws only the selected loader image through the same fullscreen
    /// stretch, filtering, edge-clear, and gamma path used by the complete
    /// loader. No title, progress indicator, or log region is included.
    pub fn render_background(
        &self,
        surface: &mut Surface,
        config: LoaderRenderConfig,
        gamma: Option<&GammaRamp>,
    ) {
        draw_loader_background(surface, &self.background, gamma, config);
    }

    /// Draws only the filterable logical-pixel layers: background, progress
    /// frame/fill, and optional log box. No glyph is included in this pass.
    pub fn render_chrome(
        &self,
        surface: &mut Surface,
        config: LoaderRenderConfig,
        gamma: Option<&GammaRamp>,
    ) -> Result<()> {
        let width = i32::try_from(surface.width()).unwrap_or(i32::MAX);
        let height = i32::try_from(surface.height()).unwrap_or(i32::MAX);
        let fonts = &self.resources.fonts;
        let layout = loader_layout(
            width,
            height,
            fonts.title.line_height,
            fonts.text.line_height,
            self.state.progress,
        );

        // 1. fctBackground.DrawFullScreen(cgo): non-aspect stretch, unless the
        // opt-in Graphics.LoaderAspect cover-fit divergence is enabled.
        self.render_background(surface, config, gamma);

        // 3. Semi-transparent black progress frame, inclusive endpoints. The
        // title is deferred to the post-upscale native-font pass.
        draw_box_dw_rect(surface, layout.progress_frame, PROGRESS_FRAME_COLOR, gamma);

        // 4. GUIProgress's (1,0,width-2,height) facet, or the exact solid
        // fallback when C4GUI itself is unavailable.
        match &self.resources.gui_progress {
            LoaderGuiProgress::GuiValid { progress_bar } => {
                let progress_bar = progress_bar.as_ref().ok_or_else(|| {
                    anyhow::anyhow!("classic loader GUI is valid but GUIProgress is missing")
                })?;
                if layout.progress_fill.width > 0 && layout.progress_fill.height > 0 {
                    draw_progress_facet(surface, progress_bar, layout.progress_fill, gamma, config);
                }
            }
            LoaderGuiProgress::GuiUnavailable => draw_box_dw(
                surface,
                layout.progress_fill.x,
                layout.progress_fill.y,
                layout
                    .progress_fill
                    .x
                    .saturating_add(layout.progress_fill.width),
                layout
                    .progress_fill
                    .y
                    .saturating_add(PROGRESS_BAR_HEIGHT - 2),
                FALLBACK_PROGRESS_COLOR,
                gamma,
            ),
        }

        // 6. A null pLog omits the entire region; log glyphs are deferred.
        if matches!(self.state.log, LoaderLog::Visible(_)) {
            draw_box_dw_rect(surface, layout.log_box, LOG_BOX_COLOR, gamma);
        }
        Ok(())
    }

    /// Draws loader text directly into an already-upscaled, possibly clipped
    /// physical surface using scale-native CStdFont atlases.
    /// `logical_width/height` are explicit because each physical dimension
    /// must round up to the matching logical dimension when divided by the
    /// application scale.
    pub fn render_native_text(
        &self,
        surface: &mut Surface,
        fonts: &NativeClonkFontSet,
        logical_width: u32,
        logical_height: u32,
        gamma: Option<&GammaRamp>,
    ) -> Result<()> {
        self.render_native_text_to(surface, fonts, logical_width, logical_height, gamma)
    }

    pub fn render_native_text_to<T: SurfaceDrawTarget + ?Sized>(
        &self,
        surface: &mut T,
        fonts: &NativeClonkFontSet,
        logical_width: u32,
        logical_height: u32,
        gamma: Option<&GammaRamp>,
    ) -> Result<()> {
        self.validate_render_text()?;
        ensure!(
            logical_width > 0 && logical_height > 0,
            "classic loader logical dimensions must be positive"
        );
        ensure!(
            logical_width <= i32::MAX as u32 && logical_height <= i32::MAX as u32,
            "classic loader logical dimensions exceed C++ integer geometry"
        );
        let scale = fonts.scale();
        let expected_width = native_viewport_extent(logical_width, scale)
            .ok_or_else(|| anyhow::anyhow!("classic loader physical width overflow"))?;
        let expected_height = native_viewport_extent(logical_height, scale)
            .ok_or_else(|| anyhow::anyhow!("classic loader physical height overflow"))?;
        let physical_width_matches = logical_extent_for(surface.width(), scale) == logical_width;
        let physical_height_matches = logical_extent_for(surface.height(), scale) == logical_height;
        ensure!(
            surface.width() <= expected_width
                && surface.height() <= expected_height
                && physical_width_matches
                && physical_height_matches,
            "classic loader native text expected a physical surface within a {expected_width}x{expected_height} viewport whose dimensions round up to {logical_width}x{logical_height} logical pixels at scale {scale}, got {}x{}",
            surface.width(),
            surface.height()
        );
        let projection = ClipperProjection::new(
            scale,
            (logical_width, logical_height),
            surface.height(),
            Rect::new(0, 0, logical_width, logical_height),
        );

        let layout = loader_layout(
            logical_width as i32,
            logical_height as i32,
            fonts.title.logical_line_height(),
            fonts.text.logical_line_height(),
            self.state.progress,
        );
        fonts.title.draw_string_to_physical_surface_with_clipper_to(
            surface,
            layout.title_anchor.0,
            layout.title_anchor.1,
            &self.state.title,
            TITLE_COLOR,
            TextAlign::Right,
            true,
            projection,
            gamma,
        );
        let progress = self.state.progress;
        fonts.text.draw_string_to_physical_surface_with_clipper_to(
            surface,
            layout.progress_text_anchor.0,
            layout.progress_text_anchor.1,
            &format!("{progress}%"),
            WHITE,
            TextAlign::Center,
            true,
            projection,
            gamma,
        );

        let LoaderLog::Visible(lines) = &self.state.log else {
            return Ok(());
        };
        let line_height = fonts.mini.logical_line_height();
        ensure!(line_height > 0, "classic loader native FontTiny is invalid");
        let lines_visible = (LOG_BOX_HEIGHT - 2 * LOG_BOX_MARGIN) / line_height;
        let start = lines
            .len()
            .saturating_sub(usize::try_from(lines_visible).unwrap_or_default());
        let x = layout.log_text_origin.0;
        let mut y = layout.log_text_origin.1;
        let mut last_extent = None;
        for line in &lines[start..] {
            if line.is_empty() {
                continue;
            }
            let extent = fonts.mini.measure(line, true);
            fonts.mini.draw_to_physical_surface_with_clipper_to(
                surface,
                x,
                y,
                line,
                WHITE,
                TextAlign::Left,
                true,
                projection,
                gamma,
            );
            y += extent.1;
            last_extent = Some(extent);
        }
        draw_native_process_suffix(
            surface,
            &fonts.mini,
            self.state.process,
            x,
            y,
            last_extent,
            projection,
            gamma,
        )
    }

    fn render_logical_text(&self, surface: &mut Surface, gamma: Option<&GammaRamp>) -> Result<()> {
        let width = i32::try_from(surface.width()).unwrap_or(i32::MAX);
        let height = i32::try_from(surface.height()).unwrap_or(i32::MAX);
        let fonts = &self.resources.fonts;
        let layout = loader_layout(
            width,
            height,
            fonts.title.line_height,
            fonts.text.line_height,
            self.state.progress,
        );

        // 2. Scenario/startup title. C++ enables markup for StringOut.
        draw_string_out(
            &fonts.title,
            surface,
            layout.title_anchor.0,
            layout.title_anchor.1,
            &self.state.title,
            TITLE_COLOR,
            TextAlign::Right,
            true,
            gamma,
        );

        // 5. Raw integer percentage in FontRegular.
        let progress = self.state.progress;
        draw_string_out(
            &fonts.text,
            surface,
            layout.progress_text_anchor.0,
            layout.progress_text_anchor.1,
            &format!("{progress}%"),
            WHITE,
            TextAlign::Center,
            true,
            gamma,
        );

        // 7+. A null pLog omits all log text.
        let LoaderLog::Visible(lines) = &self.state.log else {
            return Ok(());
        };

        let line_height = fonts.mini.line_height;
        let lines_visible = (LOG_BOX_HEIGHT - 2 * LOG_BOX_MARGIN) / line_height;
        let start = lines
            .len()
            .saturating_sub(usize::try_from(lines_visible).unwrap_or_default());
        let mut x = layout.log_text_origin.0;
        let mut y = layout.log_text_origin.1;
        let mut last_extent = None;
        for line in &lines[start..] {
            if line.is_empty() {
                continue;
            }
            let extent = fonts.mini.measure(line, true);
            fonts
                .mini
                .draw_with_gamma(surface, x, y, line, WHITE, TextAlign::Left, true, gamma);
            y += extent.1;
            last_extent = Some(extent);
        }

        // Process is appended directly after the final displayed log line.
        if let Some(process) = self.state.process {
            let (last_width, last_height) = last_extent.ok_or_else(|| {
                anyhow::anyhow!(
                    "classic loader process suffix requires a visible non-empty log line"
                )
            })?;
            y -= last_height;
            x += last_width;
            fonts.mini.draw_with_gamma(
                surface,
                x,
                y,
                &format!("{process}%"),
                WHITE,
                TextAlign::Left,
                true,
                gamma,
            );
        }
        Ok(())
    }

    fn validate_render_text(&self) -> Result<()> {
        if let LoaderLog::Visible(lines) = &self.state.log {
            if self.state.process.is_some()
                && !visible_log_has_nonempty_line(lines, self.resources.fonts.mini.line_height)
            {
                anyhow::bail!(
                    "classic loader process suffix requires a visible non-empty log line"
                );
            }
        } else if self.state.process.is_some() {
            anyhow::bail!("classic loader process suffix requires a visible non-empty log line");
        }
        Ok(())
    }
}

/// Exact fallback used by `C4MessageBoard::Draw` and `C4GUI::Screen::Draw`
/// only while no loader screen exists.  Active loaders never fall back: their
/// missing background is a constructor error.
pub fn render_absent_loader_black(surface: &mut Surface, gamma: Option<&GammaRamp>) {
    let width = i32::try_from(surface.width()).unwrap_or(i32::MAX);
    let height = i32::try_from(surface.height()).unwrap_or(i32::MAX);
    draw_box_dw(surface, 0, 0, width, height, 0x0000_0000, gamma);
}

fn validate_rgba_image(name: &str, image: &ImageData) -> Result<()> {
    ensure!(
        image.width() > 0 && image.height() > 0,
        "classic loader {name} has zero dimensions"
    );
    let expected = usize::try_from(image.width())
        .ok()
        .and_then(|width| {
            usize::try_from(image.height())
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| anyhow::anyhow!("classic loader {name} dimensions overflow"))?;
    ensure!(
        image.pixels().len() == expected,
        "classic loader {name} RGBA length mismatch: expected {expected}, got {}",
        image.pixels().len()
    );
    Ok(())
}

fn validate_font_set(fonts: &ClonkFontSet) -> Result<()> {
    for (name, font) in [
        ("FontTitle", &fonts.title),
        ("FontRegular", &fonts.text),
        ("FontTiny", &fonts.mini),
    ] {
        ensure!(
            font.line_height > 0 && font.cell_height > 0,
            "classic loader {name} is not initialized"
        );
        ensure!(
            font.cell_height >= font.line_height,
            "classic loader {name} has invalid metrics"
        );
        for required in [' ', '0', '1', '9', '%'] {
            ensure!(
                font.glyph(required).is_some(),
                "classic loader {name} is missing glyph {required:?}"
            );
        }
    }
    Ok(())
}

fn visible_log_has_nonempty_line(lines: &[String], line_height: i32) -> bool {
    if line_height <= 0 {
        return false;
    }
    let visible = (LOG_BOX_HEIGHT - 2 * LOG_BOX_MARGIN) / line_height;
    let start = lines
        .len()
        .saturating_sub(usize::try_from(visible).unwrap_or_default());
    lines[start..].iter().any(|line| !line.is_empty())
}

#[allow(clippy::too_many_arguments)]
fn draw_native_process_suffix<T: SurfaceDrawTarget + ?Sized>(
    surface: &mut T,
    font: &NativeClonkFont,
    process: Option<i32>,
    mut x: i32,
    mut y: i32,
    last_extent: Option<(i32, i32)>,
    projection: ClipperProjection,
    gamma: Option<&GammaRamp>,
) -> Result<()> {
    let Some(process) = process else {
        return Ok(());
    };
    let (last_width, last_height) = last_extent.ok_or_else(|| {
        anyhow::anyhow!("classic loader process suffix requires a visible non-empty log line")
    })?;
    y -= last_height;
    x += last_width;
    font.draw_to_physical_surface_with_clipper_to(
        surface,
        x,
        y,
        &format!("{process}%"),
        WHITE,
        TextAlign::Left,
        true,
        projection,
        gamma,
    );
    Ok(())
}

/// `CStdDDraw::StringOut` calls `CStdFont::DrawText` once.  Its alignment
/// extent treats `|`/newline as virtual line breaks, but DrawText itself
/// ignores newline and draws `|` as an ordinary glyph on the same row.  The
/// shared Rust font API models `TextOut` and therefore splits both; preserve
/// the peculiar StringOut behavior here (relevant to scenario titles).
#[allow(clippy::too_many_arguments)]
fn draw_string_out(
    font: &ClonkFont,
    surface: &mut Surface,
    x: i32,
    y: i32,
    text: &str,
    color: [u8; 4],
    align: TextAlign,
    markup: bool,
    gamma: Option<&GammaRamp>,
) {
    if !text.contains('|') && !text.contains('\n') {
        font.draw_with_gamma(surface, x, y, text, color, align, markup, gamma);
        return;
    }

    // Pick a character absent from the input so replacing `|` cannot collide
    // with a real title character, then give it the exact pipe glyph cell.
    let sentinel = ('\u{E000}'..='\u{F8FF}')
        .find(|candidate| !text.contains(*candidate))
        .unwrap_or('\u{10FFFD}');
    let mut string_font = font.clone();
    if let Some(pipe) = font.glyph('|').cloned() {
        string_font.add_glyph(sentinel, pipe);
    }
    let transformed: String = text
        .chars()
        .filter_map(|character| match character {
            // DrawText ignores system characters (`c < ' '`).
            '\n' => None,
            // Prevent the Rust TextOut model from treating this as a break.
            '|' if markup => Some(sentinel),
            other => Some(other),
        })
        .collect();

    // Alignment still uses GetTextExtent on the original string, where the
    // virtual breaks count. Draw left-aligned from that already-adjusted x.
    let (extent, _) = font.measure(text, markup);
    let left = x.saturating_sub(match align {
        TextAlign::Left => 0,
        TextAlign::Center => extent / 2,
        TextAlign::Right => extent,
    });
    string_font.draw_with_gamma(
        surface,
        left,
        y,
        &transformed,
        color,
        TextAlign::Left,
        markup,
        gamma,
    );
}

fn draw_box_dw_rect(
    surface: &mut Surface,
    rect: LoaderRect,
    color: u32,
    gamma: Option<&GammaRamp>,
) {
    if rect.width <= 0 || rect.height <= 0 {
        return;
    }
    draw_box_dw(
        surface,
        rect.x,
        rect.y,
        rect.x.saturating_add(rect.width - 1),
        rect.y.saturating_add(rect.height - 1),
        color,
        gamma,
    );
}

/// `DrawBoxDw`: inclusive coordinates and engine-inverted alpha.
fn draw_box_dw(
    surface: &mut Surface,
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    color: u32,
    gamma: Option<&GammaRamp>,
) {
    if surface.width() == 0 || surface.height() == 0 || x2 < x1 || y2 < y1 {
        return;
    }
    if surface.is_gpu_scene_capture_active()
        || crate::active_advanced_renderer_config()
            .is_some_and(|config| config.blit_offset != 0 || config.no_box_fades)
    {
        crate::draw_color_rect(
            surface,
            Rect::new(
                x1,
                y1,
                x2.saturating_sub(x1).saturating_add(1) as u32,
                y2.saturating_sub(y1).saturating_add(1) as u32,
            ),
            Color::new(
                (color >> 16) as u8,
                (color >> 8) as u8,
                color as u8,
                255 - (color >> 24) as u8,
            ),
            gamma,
        );
        return;
    }
    let inverse_alpha = ((color >> 24) & 0xff) as f32 / 255.0;
    let opacity = 1.0 - inverse_alpha;
    if opacity <= 0.0 {
        return;
    }
    let rgb = [
        gamma_encode(gamma, ((color >> 16) & 0xff) as f32),
        gamma_encode(gamma, ((color >> 8) & 0xff) as f32),
        gamma_encode(gamma, (color & 0xff) as f32),
    ];
    let max_x = surface.width() as i32 - 1;
    let max_y = surface.height() as i32 - 1;
    for y in y1.max(0)..=y2.min(max_y) {
        for x in x1.max(0)..=x2.min(max_x) {
            let destination = surface.get_pixel(x as u32, y as u32).unwrap_or_default();
            let blend = |source: f32, destination: u8| {
                (source * opacity + f32::from(destination) * inverse_alpha)
                    .round()
                    .clamp(0.0, 255.0) as u8
            };
            let _ = surface.set_pixel(
                x as u32,
                y as u32,
                Color::new(
                    blend(rgb[0], destination.r),
                    blend(rgb[1], destination.g),
                    blend(rgb[2], destination.b),
                    255,
                ),
            );
        }
    }
}

fn gamma_encode(gamma: Option<&GammaRamp>, value: f32) -> f32 {
    gamma.map_or(value, |gamma| f32::from(gamma.encode_float(value)))
}

fn texture_size(width: u32, height: u32) -> i32 {
    let needed = width.min(height).max(1);
    let mut size = 2_u32;
    while size < needed {
        size = size.saturating_mul(2);
    }
    size.min(4096) as i32
}

fn last_texture_size(width: u32, height: u32, base: i32) -> i32 {
    let base = base as u32;
    if width.is_multiple_of(base) || height.is_multiple_of(base) {
        return base as i32;
    }
    let needed = (width % base).max(height % base).max(1);
    let mut size = 2_u32;
    while size < needed {
        size = size.saturating_mul(2);
    }
    size as i32
}

fn tile_dimensions(width: u32, height: u32, base: i32) -> (i32, i32) {
    (
        (width as i32 - 1) / base + 1,
        (height as i32 - 1) / base + 1,
    )
}

fn tile_size_at(width: u32, height: u32, base: i32, tile_x: i32, tile_y: i32) -> i32 {
    let (tiles_x, tiles_y) = tile_dimensions(width, height, base);
    if tile_x == tiles_x - 1 && tile_y == tiles_y - 1 {
        last_texture_size(width, height, base)
    } else {
        base
    }
}

/// Whether CStdDDraw's unchanged base-sized `chunkSize` produces any chunks
/// for this texture. C4Surface may allocate only the final bottom-right
/// C4TexRef smaller when both dimensions have a remainder. StdDDraw then
/// switches `iTexSize` to that smaller value but leaves `chunkSize` at the
/// base; `iTexSize / chunkSize == 0` omits the tile entirely
/// (`C4Surface.cpp:193-207`; `StdDDraw2.cpp:704,719-726`).
fn tile_has_cpp_blit_chunks(width: u32, height: u32, base: i32, tile_x: i32, tile_y: i32) -> bool {
    tile_size_at(width, height, base, tile_x, tile_y) >= base
}

fn surface_texel(
    image: &ImageData,
    tile_x: i32,
    tile_y: i32,
    tile_size: i32,
    x: i32,
    y: i32,
) -> [f32; 4] {
    let x = tile_x + x.clamp(0, tile_size - 1);
    let y = tile_y + y.clamp(0, tile_size - 1);
    if x < 0 || y < 0 || x >= image.width() as i32 || y >= image.height() as i32 {
        // Face.Load -> C4Surface::Create calls Default(), clearing the earlier
        // SetBackground flag. C4TexRef's untouched bytes remain 0xffffffff:
        // transparent white in the engine's inverted-alpha texture format.
        return [255.0, 255.0, 255.0, 0.0];
    }
    let index = ((y as u32 * image.width() + x as u32) * 4) as usize;
    image
        .pixels()
        .get(index..index + 4)
        .map(|pixel| {
            if pixel[3] == 0 {
                // C4Surface::SetPixDw forces fully transparent texels black.
                [0.0, 0.0, 0.0, 0.0]
            } else {
                [
                    f32::from(pixel[0]),
                    f32::from(pixel[1]),
                    f32::from(pixel[2]),
                    f32::from(pixel[3]),
                ]
            }
        })
        .unwrap_or([255.0, 255.0, 255.0, 0.0])
}

fn bilinear_tile_sample(
    image: &ImageData,
    tile_x: i32,
    tile_y: i32,
    tile_size: i32,
    u: f32,
    v: f32,
) -> [f32; 4] {
    let x0 = u.floor() as i32;
    let y0 = v.floor() as i32;
    let fx = u - x0 as f32;
    let fy = v - y0 as f32;
    let p00 = surface_texel(image, tile_x, tile_y, tile_size, x0, y0);
    let p10 = surface_texel(image, tile_x, tile_y, tile_size, x0 + 1, y0);
    let p01 = surface_texel(image, tile_x, tile_y, tile_size, x0, y0 + 1);
    let p11 = surface_texel(image, tile_x, tile_y, tile_size, x0 + 1, y0 + 1);
    std::array::from_fn(|channel| {
        let top = p00[channel] * (1.0 - fx) + p10[channel] * fx;
        let bottom = p01[channel] * (1.0 - fx) + p11[channel] * fx;
        top * (1.0 - fy) + bottom * fy
    })
}

fn nearest_tile_sample(
    image: &ImageData,
    tile_x: i32,
    tile_y: i32,
    tile_size: i32,
    u: f32,
    v: f32,
) -> [f32; 4] {
    surface_texel(
        image,
        tile_x,
        tile_y,
        tile_size,
        (u + 0.5).floor() as i32,
        (v + 0.5).floor() as i32,
    )
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct FloatRect {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

/// The shared retained sprite path has the same physical-tile sampling as
/// this compatibility rasterizer whenever C++ would submit every intersecting
/// texture tile. Loaded fully-transparent texels are first blackened, while
/// the sprite sampler independently preserves C4TexRef's untouched transparent
/// white padding. The historical undersized final-tile omission remains on
/// the CPU oracle.
fn retained_loader_facet_image(image: &ImageData, source: FloatRect) -> Option<ImageData> {
    if image.width() == 0
        || image.height() == 0
        || !source.x.is_finite()
        || !source.y.is_finite()
        || !source.width.is_finite()
        || !source.height.is_finite()
        || source.x < 0.0
        || source.y < 0.0
        || source.width <= 0.0
        || source.height <= 0.0
        || source.x + source.width > image.width() as f32
        || source.y + source.height > image.height() as f32
    {
        return None;
    }
    thread_local! {
        static COMPATIBLE_IMAGES: RefCell<HashMap<clonk_graphics::GpuTextureId, bool>> =
            RefCell::new(HashMap::new());
    }
    let compatible = COMPATIBLE_IMAGES.with(|images| {
        if let Some(&compatible) = images.borrow().get(&image.gpu_texture_id()) {
            return compatible;
        }
        let expected_len = usize::try_from(image.width())
            .ok()
            .and_then(|width| {
                usize::try_from(image.height())
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .and_then(|pixels| pixels.checked_mul(4));
        let base = texture_size(image.width(), image.height());
        let (tiles_x, tiles_y) = tile_dimensions(image.width(), image.height(), base);
        let compatible = expected_len == Some(image.pixels().len())
            && (0..tiles_y).all(|tile_y| {
                (0..tiles_x).all(|tile_x| {
                    tile_has_cpp_blit_chunks(image.width(), image.height(), base, tile_x, tile_y)
                })
            });
        images
            .borrow_mut()
            .insert(image.gpu_texture_id(), compatible);
        compatible
    });
    compatible.then(|| crate::classic_gui::blacken_transparent_pixels(image))
}

impl FloatRect {
    fn with_scaling_correction(mut self, enabled: bool) -> Self {
        if enabled {
            if self.width > 1.0 {
                self.x += 0.5;
                self.width -= 1.0;
            }
            if self.height > 1.0 {
                self.y += 0.5;
                self.height -= 1.0;
            }
        }
        self
    }
}

fn draw_surface_facet(
    surface: &mut Surface,
    image: &ImageData,
    source_rect: FloatRect,
    target: LoaderRect,
    gamma: Option<&GammaRamp>,
    sampling: LoaderSampling,
) {
    let retained_image = retained_loader_facet_image(image, source_rect);
    let configured_image = retained_image.as_ref().unwrap_or(image);
    if crate::draw_image_source_with_active_renderer_config(
        surface,
        &clonk_gui::Rect::new(
            target.x as f32,
            target.y as f32,
            target.width as f32,
            target.height as f32,
        ),
        configured_image,
        (
            source_rect.x,
            source_rect.y,
            source_rect.width,
            source_rect.height,
        ),
        sampling,
        gamma,
    ) {
        return;
    }
    if source_rect.width <= 0.0
        || source_rect.height <= 0.0
        || target.width <= 0
        || target.height <= 0
    {
        return;
    }
    if retained_image.as_ref().is_some_and(|retained_image| {
        crate::capture_gpu_gui_image(
            surface,
            (
                target.x as f32,
                target.y as f32,
                target.width as f32,
                target.height as f32,
            ),
            retained_image,
            crate::FloatSourceRect {
                x: source_rect.x,
                y: source_rect.y,
                width: source_rect.width,
                height: source_rect.height,
            },
            match sampling {
                LoaderSampling::Nearest => clonk_graphics::GpuSampler::Nearest,
                LoaderSampling::Linear => clonk_graphics::GpuSampler::Linear,
            },
            crate::BilinearBlend::AlphaOver,
            None,
            gamma,
        )
    }) {
        return;
    }
    let scale_x = target.width as f32 / source_rect.width;
    let scale_y = target.height as f32 / source_rect.height;
    let base_tile_size = texture_size(image.width(), image.height());
    let (tiles_x, tiles_y) = tile_dimensions(image.width(), image.height(), base_tile_size);
    let source_right = source_rect.x + source_rect.width;
    let source_bottom = source_rect.y + source_rect.height;

    for tile_y_index in 0..tiles_y {
        for tile_x_index in 0..tiles_x {
            if !tile_has_cpp_blit_chunks(
                image.width(),
                image.height(),
                base_tile_size,
                tile_x_index,
                tile_y_index,
            ) {
                continue;
            }
            // CStdDDraw computes the blit origin before replacing iTexSize
            // with a potentially smaller final C4TexRef size.
            let tile_x = tile_x_index * base_tile_size;
            let tile_y = tile_y_index * base_tile_size;
            let tile_size = tile_size_at(
                image.width(),
                image.height(),
                base_tile_size,
                tile_x_index,
                tile_y_index,
            );
            let facet_left = source_rect.x.max(tile_x as f32);
            let facet_top = source_rect.y.max(tile_y as f32);
            let facet_right = source_right.min((tile_x + tile_size) as f32);
            let facet_bottom = source_bottom.min((tile_y + tile_size) as f32);
            if facet_right <= facet_left || facet_bottom <= facet_top {
                continue;
            }

            let target_left = target.x as f32 + (facet_left - source_rect.x) * scale_x;
            let target_top = target.y as f32 + (facet_top - source_rect.y) * scale_y;
            let target_right = target.x as f32 + (facet_right - source_rect.x) * scale_x;
            let target_bottom = target.y as f32 + (facet_bottom - source_rect.y) * scale_y;
            let first_x = (target_left - 0.5).ceil() as i32;
            let first_y = (target_top - 0.5).ceil() as i32;
            for y in first_y.max(0)..surface.height() as i32 {
                if y as f32 + 0.5 >= target_bottom {
                    break;
                }
                for x in first_x.max(0)..surface.width() as i32 {
                    if x as f32 + 0.5 >= target_right {
                        break;
                    }
                    let source_x =
                        source_rect.x + (x as f32 + 0.5 - target.x as f32) / scale_x - 0.5;
                    let source_y =
                        source_rect.y + (y as f32 + 0.5 - target.y as f32) / scale_y - 0.5;
                    let u = source_x - tile_x as f32;
                    let v = source_y - tile_y as f32;
                    let source = match sampling {
                        LoaderSampling::Nearest => {
                            nearest_tile_sample(image, tile_x, tile_y, tile_size, u, v)
                        }
                        LoaderSampling::Linear => {
                            bilinear_tile_sample(image, tile_x, tile_y, tile_size, u, v)
                        }
                    };
                    blend_fragment(surface, x, y, source, gamma);
                }
            }
        }
    }
}

/// Full-facet `CStdDDraw::Blit`. `Face.Load` calls `C4Surface::Create`, whose
/// `Default` resets the pre-load `SetBackground` flag; unused C4TexRef bytes
/// therefore remain 0xffffffff (transparent white). `DrawFullScreen`
/// separately clears the target's final row/column before the stretched blit
/// when its size guard fires (`C4Surface.cpp:137-140,1108-1113`;
/// `C4Facet.cpp:130-141`).
/// Source facet for the fullscreen loader blit.
///
/// C++ always passes the whole facet and stretches it to the target with no
/// aspect handling (`C4Facet.cpp:191` calls `Draw(cgo, false)`), so a 4:3
/// loader on a 16:9 panel is squashed. The opt-in `Graphics.LoaderAspect`
/// divergence instead centre-crops the source to the target's aspect ratio
/// (cover). C++'s own `fAspect` mode (`C4Facet.cpp:124-133`) letterboxes, which
/// would replace distortion with black bars; cover keeps the screen filled and,
/// for the shipped 3840x2880 loaders on a 16:9 panel, reduces to an unscaled
/// one-to-one blit.
fn loader_background_source(
    source_width: f32,
    source_height: f32,
    target_width: f32,
    target_height: f32,
    aspect_fill: bool,
) -> FloatRect {
    let full = FloatRect {
        x: 0.0,
        y: 0.0,
        width: source_width,
        height: source_height,
    };
    if !aspect_fill
        || source_width <= 0.0
        || source_height <= 0.0
        || target_width <= 0.0
        || target_height <= 0.0
    {
        return full;
    }
    // w1 : h1 <=> w2 : h2  =>  w1 * h2 <=> w2 * h1, as in C4Facet::Draw.
    let source_relative = source_width * target_height;
    let target_relative = target_width * source_height;
    if source_relative > target_relative {
        // Source is relatively wider: keep full height, crop the sides.
        let width = (source_height * target_width / target_height).min(source_width);
        FloatRect {
            x: (source_width - width) * 0.5,
            width,
            ..full
        }
    } else if source_relative < target_relative {
        // Source is relatively taller: keep full width, crop top and bottom.
        let height = (source_width * target_height / target_width).min(source_height);
        FloatRect {
            y: (source_height - height) * 0.5,
            height,
            ..full
        }
    } else {
        full
    }
}

fn draw_loader_background(
    surface: &mut Surface,
    image: &ImageData,
    gamma: Option<&GammaRamp>,
    config: LoaderRenderConfig,
) {
    if surface.width() == 0 || surface.height() == 0 {
        return;
    }
    let target_width = surface.width() as i32;
    let target_height = surface.height() as i32;
    let source_width_i32 = image.width() as i32;
    // Preserve C4Facet::DrawFullScreen's historical height-vs-Wdt guard.
    if target_width > source_width_i32.saturating_add(2)
        || target_height > source_width_i32.saturating_add(2)
    {
        draw_box_dw(
            surface,
            0,
            target_height - 1,
            target_width.saturating_add(2),
            target_height.saturating_add(2),
            0x0000_0000,
            gamma,
        );
        draw_box_dw(
            surface,
            target_width - 1,
            0,
            target_width.saturating_add(2),
            target_height.saturating_add(2),
            0x0000_0000,
            gamma,
        );
    }
    let source = loader_background_source(
        image.width() as f32,
        image.height() as f32,
        target_width as f32,
        target_height as f32,
        config.aspect_fill(),
    )
    .with_scaling_correction(config.uses_scaling_correction());
    let target = LoaderRect {
        x: 0,
        y: 0,
        width: target_width,
        height: target_height,
    };
    let exact = source.width == target.width as f32 && source.height == target.height as f32;
    draw_surface_facet(
        surface,
        image,
        source,
        target,
        gamma,
        config.sampling(exact),
    );
}

fn draw_progress_facet(
    surface: &mut Surface,
    image: &ImageData,
    target: LoaderRect,
    gamma: Option<&GammaRamp>,
    config: LoaderRenderConfig,
) {
    // fctProgressBar.Set(surface, 1, 0, width-2, height).
    let source = FloatRect {
        x: 1.0,
        y: 0.0,
        width: (image.width() - 2) as f32,
        height: image.height() as f32,
    }
    .with_scaling_correction(config.uses_scaling_correction());
    let exact = source.width == target.width as f32 && source.height == target.height as f32;
    draw_surface_facet(
        surface,
        image,
        source,
        target,
        gamma,
        config.sampling(exact),
    );
}

fn blend_fragment(
    surface: &mut Surface,
    x: i32,
    y: i32,
    source: [f32; 4],
    gamma: Option<&GammaRamp>,
) {
    if source[3] <= 0.0 {
        return;
    }
    if surface.is_gpu_scene_capture_active() {
        // Fallback rasterization during capture must stay a painter-ordered
        // retained fragment instead of blending against stale CPU backing.
        let _ = surface.blend_fragment_over(x as u32, y as u32, source, gamma);
        return;
    }
    let alpha = (source[3] / 255.0).clamp(0.0, 1.0);
    let destination = surface.get_pixel(x as u32, y as u32).unwrap_or_default();
    let blend = |channel: usize, destination: u8| {
        (gamma_encode(gamma, source[channel]) * alpha + f32::from(destination) * (1.0 - alpha))
            .round()
            .clamp(0.0, 255.0) as u8
    };
    let _ = surface.set_pixel(
        x as u32,
        y as u32,
        Color::new(
            blend(0, destination.r),
            blend(1, destination.g),
            blend(2, destination.b),
            255,
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clonk_fonts::{build_font_set, build_native_font_set};
    use crate::test_support::{endeavour_font_set, load_graphics_png, repo_root, standard_gamma};
    use clonk_graphics::PixelFormat;

    fn real_image(name: &str) -> ImageData {
        let path = repo_root().join("planet/Graphics.c4g").join(name);
        let image = clonk_resources::open_image(&path)
            .unwrap_or_else(|error| panic!("decode {}: {error}", path.display()))
            .into_rgba8();
        let (width, height) = image.dimensions();
        ImageData::new(width, height, image.into_raw())
    }

    fn resources() -> LoaderResources {
        LoaderResources::new(endeavour_font_set(), load_graphics_png("GUIProgress.png"))
            .expect("real loader resources")
    }

    fn screen(context: LoaderContext, state: LoaderState) -> LoaderScreen {
        let selection = match context {
            LoaderContext::Startup => LoaderSelection::startup("LoaderGoldmine1.png"),
            LoaderContext::Scenario => LoaderSelection::scenario("LoaderSky*", "LoaderSky1.jpg"),
        }
        .expect("selection");
        let background = match context {
            LoaderContext::Startup => real_image("LoaderGoldmine1.png"),
            LoaderContext::Scenario => real_image("LoaderSky1.jpg"),
        };
        LoaderScreen::new(selection, background, resources(), state).expect("screen")
    }

    fn synthetic_screen(state: LoaderState, color: [u8; 4]) -> LoaderScreen {
        let selection = LoaderSelection::scenario("Loader*", "LoaderTest.png").unwrap();
        let background = ImageData::new(2, 2, color.repeat(4));
        LoaderScreen::new(selection, background, resources(), state).expect("synthetic screen")
    }

    fn fnv1a64(bytes: &[u8]) -> u64 {
        bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
        })
    }

    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    fn jpeg_decoder_uses_ssse3() -> bool {
        std::is_x86_feature_detected!("ssse3")
    }

    #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
    fn jpeg_decoder_uses_ssse3() -> bool {
        false
    }

    fn loader_sky_regression_hashes() -> (u64, u64) {
        if jpeg_decoder_uses_ssse3() {
            (5_382_921_512_495_582_144, 10_130_694_106_439_574_915)
        } else {
            (12_602_292_018_454_220_685, 2_045_718_955_757_962_211)
        }
    }

    fn endeavour_bytes() -> Vec<u8> {
        let path = repo_root().join("planet/System.c4g/Endeavour.ttf");
        std::fs::read(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
    }

    fn upscale_nearest(source: &Surface, scale: u32) -> Surface {
        let mut target = Surface::new(
            source.width() * scale,
            source.height() * scale,
            PixelFormat::Rgba8888,
        );
        for y in 0..target.height() {
            for x in 0..target.width() {
                let pixel = source.get_pixel(x / scale, y / scale).unwrap();
                let _ = target.set_pixel(x, y, pixel);
            }
        }
        target
    }

    #[test]
    fn constructor_records_effective_app_owned_loader_selection() {
        let startup = LoaderSelection::startup("LoaderGoldmine1.PNG").expect("startup");
        assert_eq!(startup.context(), LoaderContext::Startup);
        assert_eq!(
            startup.effective_specification(),
            STARTUP_LOADER_SPECIFICATION
        );
        assert_eq!(startup.selected_filename(), "LoaderGoldmine1.PNG");

        let scenario = LoaderSelection::scenario("", "LoaderSky1.jpeg").expect("scenario");
        assert_eq!(scenario.context(), LoaderContext::Scenario);
        assert_eq!(
            scenario.effective_specification(),
            DEFAULT_LOADER_SPECIFICATION
        );
        assert_eq!(scenario.selected_filename(), "LoaderSky1.jpeg");

        assert_eq!(
            LoaderSelection::scenario("Loader*", "")
                .unwrap_err()
                .to_string(),
            "classic loader selection has no filename"
        );
        assert_eq!(
            LoaderSelection::scenario("Loader*", "Loader.webp")
                .unwrap_err()
                .to_string(),
            "classic loader selection 'Loader.webp' is not PNG, JPEG, or BMP"
        );
    }

    #[test]
    fn layout_matches_every_inclusive_cpp_coordinate_at_1280x720() {
        let layout = loader_layout(1280, 720, 34, 22, 42);
        assert_eq!(layout.title_anchor, (1260, 557));
        assert_eq!(
            layout.progress_frame,
            LoaderRect {
                x: 20,
                y: 596,
                width: 1241,
                height: 16,
            }
        );
        assert_eq!(
            layout.progress_fill,
            LoaderRect {
                x: 21,
                y: 597,
                width: 519,
                height: 13,
            }
        );
        assert_eq!(layout.progress_text_anchor, (640, 593));
        assert_eq!(
            layout.log_box,
            LoaderRect {
                x: 20,
                y: 616,
                width: 1241,
                height: 85,
            }
        );
        assert_eq!(layout.log_text_origin, (22, 618));
    }

    #[test]
    fn updates_cover_boot_and_scenario_loading_state_without_io() {
        let mut loader = screen(LoaderContext::Scenario, LoaderState::initial("Loading..."));
        loader.update(LoaderUpdate::SetTitle("Gold Mine".into()));
        loader.update(LoaderUpdate::SetProgress(58));
        loader.update(LoaderUpdate::ShowLog);
        loader.update(LoaderUpdate::AppendLog("Loading definitions ".into()));
        loader.update(LoaderUpdate::SetProcess(Some(35)));
        assert_eq!(loader.state().title(), "Gold Mine");
        assert_eq!(loader.state().progress(), 58);
        assert_eq!(
            loader.state().log(),
            &LoaderLog::Visible(vec!["Loading definitions ".into()])
        );
        assert_eq!(loader.state().process(), Some(35));

        loader.update(LoaderUpdate::SetProcess(Some(0)));
        loader.update(LoaderUpdate::HideLog);
        assert_eq!(loader.state().process(), None);
        assert_eq!(loader.state().log(), &LoaderLog::Hidden);
    }

    #[test]
    fn resource_refresh_preserves_loader_selection_background_and_live_state() {
        let mut loader = synthetic_screen(LoaderState::initial("Loading..."), [7, 8, 9, 255]);
        loader.update(LoaderUpdate::SetProgress(73));
        loader.update(LoaderUpdate::ReplaceLog(vec!["Retained".into()]));
        let selection = loader.selection.clone();
        let state = loader.state.clone();
        let background = loader.background.clone();

        loader.replace_resources(
            LoaderResources::gui_unavailable(endeavour_font_set()).expect("fallback resources"),
        );
        assert_eq!(loader.selection, selection);
        assert_eq!(loader.state, state);
        assert_eq!(loader.background, background);
        assert_eq!(
            loader.resources.gui_progress(),
            &LoaderGuiProgress::GuiUnavailable
        );
    }

    #[test]
    fn sampling_config_matches_stdgl_at_integer_and_fractional_scales() {
        assert_eq!(
            LoaderRenderConfig::new(1.0, true).unwrap().sampling(false),
            LoaderSampling::Nearest
        );
        assert_eq!(
            LoaderRenderConfig::new(1.0, false).unwrap().sampling(false),
            LoaderSampling::Linear
        );
        assert_eq!(
            LoaderRenderConfig::new(2.0, true).unwrap().sampling(true),
            LoaderSampling::Linear
        );
        // fExact is computed per blit from source/target dimensions. At scale
        // one, even PointFiltering=false leaves an exact-size blit nearest.
        assert_eq!(
            LoaderRenderConfig::new(1.0, false).unwrap().sampling(true),
            LoaderSampling::Nearest
        );
        let fractional = LoaderRenderConfig::new(1.5, true).expect("fractional scale");
        assert_eq!(fractional.application_scale(), 1.5);
        assert!(fractional.uses_scaling_correction());
        assert_eq!(fractional.sampling(true), LoaderSampling::Linear);
        for scale in [0.0, -1.0, f32::INFINITY, f32::NAN] {
            assert!(LoaderRenderConfig::new(scale, false).is_err());
        }
    }

    #[test]
    fn retained_opaque_loader_facet_is_one_textured_command() {
        let image = ImageData::new(8, 8, [20, 40, 60, 255].repeat(64));
        let mut surface = Surface::new(320, 180, PixelFormat::Rgba8888);
        surface.begin_gpu_scene_capture();
        draw_surface_facet(
            &mut surface,
            &image,
            FloatRect {
                x: 0.0,
                y: 0.0,
                width: 8.0,
                height: 8.0,
            },
            LoaderRect {
                x: 0,
                y: 0,
                width: 320,
                height: 180,
            },
            None,
            LoaderSampling::Linear,
        );

        let scene = surface
            .take_gpu_scene_capture()
            .expect("capture remains active")
            .into_scene([320, 180], Color::transparent(), &GammaRamp::identity());
        assert_eq!(scene.commands.len(), 1);
        // clonk-org/clonk-rs#271: retained as a compact instance rather than a
        // generic quad; still one textured command.
        let clonk_graphics::GpuCommand::ObjectBatch { sprites, .. } = &scene.commands[0] else {
            panic!("opaque loader background was not retained as a texture command");
        };
        assert_eq!(sprites.len(), 1);
        assert_eq!(sprites[0].sampler(), clonk_graphics::GpuSampler::Linear);
    }

    #[test]
    fn retained_translucent_loader_facet_reuses_blackened_texture() {
        let mut pixels = [40, 80, 120, 180].repeat(64);
        pixels[..4].copy_from_slice(&[250, 10, 20, 0]);
        let image = ImageData::new(8, 8, pixels);
        let capture = || {
            let mut surface = Surface::new(160, 90, PixelFormat::Rgba8888);
            surface.begin_gpu_scene_capture();
            draw_surface_facet(
                &mut surface,
                &image,
                FloatRect {
                    x: 0.0,
                    y: 0.0,
                    width: 8.0,
                    height: 8.0,
                },
                LoaderRect {
                    x: 0,
                    y: 0,
                    width: 160,
                    height: 90,
                },
                None,
                LoaderSampling::Linear,
            );
            surface
                .take_gpu_scene_capture()
                .expect("capture remains active")
                .into_scene([160, 90], Color::transparent(), &GammaRamp::identity())
        };

        let first = capture();
        let second = capture();
        assert_eq!(first.commands.len(), 1);
        assert_eq!(first.textures.len(), 1);
        assert_eq!(first.textures[0].id, second.textures[0].id);
        assert_eq!(&first.textures[0].pixels[..4], &[0, 0, 0, 0]);
    }

    #[test]
    fn retained_loader_box_is_one_solid_command() {
        let mut surface = Surface::new(320, 180, PixelFormat::Rgba8888);
        surface.begin_gpu_scene_capture();
        draw_box_dw(&mut surface, 10, 12, 300, 160, 0x7f20_4060, None);

        let scene = surface
            .take_gpu_scene_capture()
            .expect("capture remains active")
            .into_scene([320, 180], Color::transparent(), &GammaRamp::identity());
        assert_eq!(scene.commands.len(), 1);
        assert!(matches!(
            &scene.commands[0],
            clonk_graphics::GpuCommand::Solid { .. }
        ));
    }

    #[test]
    fn high_dpi_source_correction_insets_background_and_progress_facets() {
        let mut background_pixels = Vec::new();
        for _y in 0..3 {
            for red in [0, 100, 200] {
                background_pixels.extend_from_slice(&[red, 0, 0, 255]);
            }
        }
        let background = ImageData::new(3, 3, background_pixels);
        let mut corrected_background = Surface::new(6, 6, PixelFormat::Rgba8888);
        let mut uncorrected_background = Surface::new(6, 6, PixelFormat::Rgba8888);
        draw_loader_background(
            &mut corrected_background,
            &background,
            None,
            LoaderRenderConfig::new(2.0, false).unwrap(),
        );
        draw_surface_facet(
            &mut uncorrected_background,
            &background,
            FloatRect {
                x: 0.0,
                y: 0.0,
                width: 3.0,
                height: 3.0,
            },
            LoaderRect {
                x: 0,
                y: 0,
                width: 6,
                height: 6,
            },
            None,
            LoaderSampling::Linear,
        );
        assert!(
            corrected_background.get_pixel(0, 2).unwrap().r
                > uncorrected_background.get_pixel(0, 2).unwrap().r
        );

        let mut progress_pixels = Vec::new();
        for _y in 0..3 {
            for red in [0, 50, 100, 150, 200] {
                progress_pixels.extend_from_slice(&[red, 0, 0, 255]);
            }
        }
        let progress = ImageData::new(5, 3, progress_pixels);
        let target = LoaderRect {
            x: 0,
            y: 0,
            width: 6,
            height: 6,
        };
        let mut corrected_progress = Surface::new(6, 6, PixelFormat::Rgba8888);
        let mut uncorrected_progress = Surface::new(6, 6, PixelFormat::Rgba8888);
        draw_progress_facet(
            &mut corrected_progress,
            &progress,
            target,
            None,
            LoaderRenderConfig::new(2.0, false).unwrap(),
        );
        draw_surface_facet(
            &mut uncorrected_progress,
            &progress,
            FloatRect {
                x: 1.0,
                y: 0.0,
                width: 3.0,
                height: 3.0,
            },
            target,
            None,
            LoaderSampling::Linear,
        );
        assert!(
            corrected_progress.get_pixel(0, 2).unwrap().r
                > uncorrected_progress.get_pixel(0, 2).unwrap().r
        );
    }

    #[test]
    fn variable_progress_facet_samples_across_all_c4_texture_tiles() {
        let mut pixels = Vec::new();
        for _y in 0..4 {
            for x in 0..10 {
                let color = if x < 4 {
                    [200, 0, 0, 255]
                } else if x < 8 {
                    [0, 200, 0, 255]
                } else {
                    [0, 0, 200, 255]
                };
                pixels.extend_from_slice(&color);
            }
        }
        let progress = ImageData::new(10, 4, pixels);
        assert_eq!(texture_size(10, 4), 4);
        let mut target = Surface::new(8, 4, PixelFormat::Rgba8888);
        draw_progress_facet(
            &mut target,
            &progress,
            LoaderRect {
                x: 0,
                y: 0,
                width: 8,
                height: 4,
            },
            None,
            LoaderRenderConfig::scale_one(false),
        );
        assert_eq!(target.get_pixel(0, 1).unwrap(), Color::opaque(200, 0, 0));
        assert_eq!(target.get_pixel(3, 1).unwrap(), Color::opaque(0, 200, 0));
        assert_eq!(target.get_pixel(7, 1).unwrap(), Color::opaque(0, 0, 200));
    }

    #[test]
    fn cpp_omits_only_a_genuinely_smaller_final_bottom_right_tile() {
        // No large pixel allocation is needed to pin the C4Surface/CStdDDraw
        // geometry bug. 5000x5000 creates a 2x2 grid at base 4096, with only
        // the final C4TexRef reduced to 1024. The unchanged 4096 chunk size
        // yields zero chunks for that tile.
        let base = texture_size(5000, 5000);
        assert_eq!(base, 4096);
        assert_eq!(tile_dimensions(5000, 5000, base), (2, 2));
        assert_eq!(tile_size_at(5000, 5000, base, 1, 1), 1024);
        assert!(tile_has_cpp_blit_chunks(5000, 5000, base, 0, 0));
        assert!(tile_has_cpp_blit_chunks(5000, 5000, base, 1, 0));
        assert!(tile_has_cpp_blit_chunks(5000, 5000, base, 0, 1));
        assert!(!tile_has_cpp_blit_chunks(5000, 5000, base, 1, 1));

        // An ordinary multi-tile 10x4 progress surface keeps a base-sized
        // final allocation because one dimension divides the base exactly.
        let ordinary_base = texture_size(10, 4);
        assert_eq!(ordinary_base, 4);
        assert_eq!(tile_dimensions(10, 4, ordinary_base), (3, 1));
        assert_eq!(tile_size_at(10, 4, ordinary_base, 2, 0), 4);
        assert!(tile_has_cpp_blit_chunks(10, 4, ordinary_base, 2, 0));

        // The smaller-allocation branch is likewise disabled when the large
        // surface's height divides the base texture size.
        assert!(tile_has_cpp_blit_chunks(5000, 4096, 4096, 1, 0));
    }

    #[test]
    fn malformed_resources_fail_instead_of_falling_back() {
        let variable_progress = ImageData::new(5, 7, [220, 10, 20, 255].repeat(5 * 7));
        let variable = LoaderResources::new(endeavour_font_set(), variable_progress)
            .expect("C++ accepts arbitrary valid progress facets");
        assert_eq!(
            variable
                .progress_bar()
                .map(|image| (image.width(), image.height())),
            Some((5, 7))
        );
        let mut variable_loader = synthetic_screen(LoaderState::initial(""), [255, 255, 255, 255]);
        variable_loader.update(LoaderUpdate::SetProgress(100));
        variable_loader.replace_resources(variable);
        let mut variable_frame = Surface::new(320, 240, PixelFormat::Rgba8888);
        variable_loader
            .render_chrome(
                &mut variable_frame,
                LoaderRenderConfig::scale_one(true),
                None,
            )
            .unwrap();
        let variable_pixel = variable_frame.get_pixel(30, 120).unwrap();
        assert!(variable_pixel.r > variable_pixel.g);

        let narrow_progress = ImageData::new(2, 7, vec![0; 2 * 7 * 4]);
        assert_eq!(
            LoaderResources::new(endeavour_font_set(), narrow_progress)
                .err()
                .expect("narrow progress")
                .to_string(),
            "classic loader GUIProgress width must be at least 3, got 2"
        );

        let short_progress = ImageData::new(32, 32, vec![0; 11]);
        assert_eq!(
            LoaderResources::new(endeavour_font_set(), short_progress)
                .err()
                .expect("short progress")
                .to_string(),
            "classic loader GUIProgress RGBA length mismatch: expected 4096, got 11"
        );

        assert_eq!(
            LoaderResources::from_gui_state(
                endeavour_font_set(),
                LoaderGuiProgress::GuiValid { progress_bar: None },
            )
            .err()
            .expect("missing GUIProgress")
            .to_string(),
            "classic loader GUI is valid but GUIProgress is missing"
        );

        let empty_fonts = Arc::new(ClonkFontSet {
            title: clonk_graphics::clonk_font::ClonkFont::new(34),
            caption: clonk_graphics::clonk_font::ClonkFont::new(25),
            text: clonk_graphics::clonk_font::ClonkFont::new(22),
            main_small: clonk_graphics::clonk_font::ClonkFont::new(20),
            mini: clonk_graphics::clonk_font::ClonkFont::new(18),
        });
        assert_eq!(
            LoaderResources::new(empty_fonts, load_graphics_png("GUIProgress.png"))
                .err()
                .expect("missing glyph")
                .to_string(),
            "classic loader FontTitle is missing glyph ' '"
        );

        let selection = LoaderSelection::scenario("Loader*", "LoaderSky1.jpg").unwrap();
        let malformed_background = ImageData::new(2, 2, vec![0; 7]);
        assert_eq!(
            LoaderScreen::new(
                selection,
                malformed_background,
                resources(),
                LoaderState::initial("Loading..."),
            )
            .err()
            .expect("short background")
            .to_string(),
            "classic loader loader background RGBA length mismatch: expected 16, got 7"
        );
    }

    #[test]
    fn absent_loader_fallback_is_black_and_never_generic_blue() {
        let mut surface = Surface::new(3, 2, PixelFormat::Rgba8888);
        for y in 0..2 {
            for x in 0..3 {
                let _ = surface.set_pixel(x, y, Color::opaque(16, 28, 52));
            }
        }
        render_absent_loader_black(&mut surface, None);
        assert!(surface
            .pixels()
            .chunks_exact(4)
            .all(|pixel| pixel == [0, 0, 0, 255]));
    }

    #[test]
    fn gui_unavailable_draws_the_inclusive_solid_red_fallback_even_at_zero() {
        let state = LoaderState::initial("");
        let gui = synthetic_screen(state.clone(), [255, 255, 255, 255]);
        let mut fallback = synthetic_screen(state, [255, 255, 255, 255]);
        fallback.replace_resources(
            LoaderResources::gui_unavailable(endeavour_font_set()).expect("fallback resources"),
        );
        let mut gui_frame = Surface::new(320, 240, PixelFormat::Rgba8888);
        let mut fallback_frame = Surface::new(320, 240, PixelFormat::Rgba8888);
        gui.render_chrome(&mut gui_frame, LoaderRenderConfig::default(), None)
            .unwrap();
        fallback
            .render_chrome(&mut fallback_frame, LoaderRenderConfig::default(), None)
            .unwrap();

        // At 0%, DrawX receives width zero and draws nothing. The no-GUI
        // DrawBoxDw path receives equal x endpoints and therefore paints one
        // red column, with an inclusive 14px vertical span.
        let gui_pixel = gui_frame.get_pixel(21, 117).unwrap();
        let fallback_pixel = fallback_frame.get_pixel(21, 117).unwrap();
        assert_eq!(gui_pixel.r, gui_pixel.g);
        assert!(fallback_pixel.r > fallback_pixel.g);
        assert!(fallback_frame.get_pixel(21, 130).unwrap().r > 100);
        assert_eq!(
            fallback_frame.get_pixel(22, 117),
            gui_frame.get_pixel(22, 117)
        );
    }

    #[test]
    fn fullscreen_edge_clear_and_background_padding_match_upscale_behavior() {
        let transparent = ImageData::new(1, 1, vec![0, 0, 0, 0]);
        let mut edge = Surface::new(4, 4, PixelFormat::Rgba8888);
        edge.pixels_mut()
            .chunks_exact_mut(4)
            .for_each(|pixel| pixel.copy_from_slice(&[20, 40, 80, 255]));
        draw_loader_background(
            &mut edge,
            &transparent,
            None,
            LoaderRenderConfig::scale_one(true),
        );
        assert_eq!(edge.get_pixel(0, 0), Some(Color::opaque(20, 40, 80)));
        assert_eq!(edge.get_pixel(3, 0), Some(Color::opaque(0, 0, 0)));
        assert_eq!(edge.get_pixel(0, 3), Some(Color::opaque(0, 0, 0)));

        // C4Surface::Create resets SetBackground, so unused texture bytes stay
        // transparent white. Over DrawFullScreen's black edge clear,
        // GL_LINEAR produces a translucent gray edge; nearest samples the last
        // real opaque-white texel.
        let white = ImageData::new(5, 7, vec![255; 5 * 7 * 4]);
        assert_eq!(texture_size(5, 7), 8);
        assert_eq!(
            surface_texel(&white, 0, 0, 8, 5, 0),
            [255.0, 255.0, 255.0, 0.0]
        );
        let mut linear = Surface::new(10, 14, PixelFormat::Rgba8888);
        let mut nearest = Surface::new(10, 14, PixelFormat::Rgba8888);
        draw_loader_background(
            &mut linear,
            &white,
            None,
            LoaderRenderConfig::scale_one(false),
        );
        draw_loader_background(
            &mut nearest,
            &white,
            None,
            LoaderRenderConfig::scale_one(true),
        );
        assert!(linear.get_pixel(9, 6).unwrap().r < 255);
        assert_eq!(nearest.get_pixel(9, 6).unwrap().r, 255);

        // Loaded fully transparent pixels are independently forced to black;
        // they are not the transparent-white untouched padding above.
        let transparent_red = ImageData::new(
            2,
            2,
            vec![
                255, 0, 0, 0, 255, 255, 255, 255, 255, 0, 0, 0, 255, 255, 255, 255,
            ],
        );
        assert_eq!(
            surface_texel(&transparent_red, 0, 0, 2, 0, 0),
            [0.0, 0.0, 0.0, 0.0]
        );
        let mut blackened = Surface::new(4, 2, PixelFormat::Rgba8888);
        draw_loader_background(
            &mut blackened,
            &transparent_red,
            None,
            LoaderRenderConfig::scale_one(false),
        );
        let transition = blackened.get_pixel(1, 0).unwrap();
        assert_eq!(transition.r, transition.g);
        assert_eq!(transition.g, transition.b);
    }

    /// `C4Facet::DrawFullScreen` guards its edge clear with
    /// `if (cgo.Wdt > Wdt + 2 || cgo.Hgt > Wdt + 2)` — both comparisons are
    /// against the *source width* `Wdt` (`C4Facet.cpp:185`). A target taller
    /// than the source but no wider than `Wdt + 2` therefore gets no clear at
    /// all. Pinning this keeps the mirror from "fixing" the C++ typo.
    #[test]
    fn fullscreen_edge_clear_guard_compares_height_against_source_width() {
        let prefill = [20, 40, 80, 255];
        // Fully transparent so nothing is blitted over the clear; only
        // DrawBoxDw can change a pixel.
        let transparent = ImageData::new(32, 2, vec![0; 32 * 2 * 4]);
        let mut wide_source = Surface::new(8, 8, PixelFormat::Rgba8888);
        wide_source
            .pixels_mut()
            .chunks_exact_mut(4)
            .for_each(|pixel| pixel.copy_from_slice(&prefill));
        draw_loader_background(
            &mut wide_source,
            &transparent,
            None,
            LoaderRenderConfig::scale_one(true),
        );
        // 8 > 32 + 2 is false for both the width and the height comparison,
        // even though the target is four times the source height.
        assert_eq!(wide_source.get_pixel(7, 7), Some(Color::opaque(20, 40, 80)));
        assert_eq!(wide_source.get_pixel(0, 7), Some(Color::opaque(20, 40, 80)));
        assert_eq!(wide_source.get_pixel(7, 0), Some(Color::opaque(20, 40, 80)));

        // A narrow source crosses the same `Wdt + 2` threshold and does clear.
        let narrow = ImageData::new(4, 40, vec![0; 4 * 40 * 4]);
        let mut narrow_source = Surface::new(8, 8, PixelFormat::Rgba8888);
        narrow_source
            .pixels_mut()
            .chunks_exact_mut(4)
            .for_each(|pixel| pixel.copy_from_slice(&prefill));
        draw_loader_background(
            &mut narrow_source,
            &narrow,
            None,
            LoaderRenderConfig::scale_one(true),
        );
        assert_eq!(narrow_source.get_pixel(7, 7), Some(Color::opaque(0, 0, 0)));
        assert_eq!(narrow_source.get_pixel(0, 7), Some(Color::opaque(0, 0, 0)));
        assert_eq!(
            narrow_source.get_pixel(0, 0),
            Some(Color::opaque(20, 40, 80))
        );
    }

    /// Divergence `Graphics.LoaderAspect`: cover-fit the loader instead of
    /// C++'s unconditional non-aspect stretch (`C4Facet.cpp:191`).
    #[test]
    fn loader_aspect_fill_centre_crops_the_source_instead_of_squashing_it() {
        let background = ImageData::new(
            2,
            4,
            vec![
                255, 0, 0, 255, 255, 0, 0, 255, // row 0: red
                0, 255, 0, 255, 0, 255, 0, 255, // row 1: green
                0, 0, 255, 255, 0, 0, 255, 255, // row 2: blue
                255, 255, 255, 255, 255, 255, 255, 255, // row 3: white
            ],
        );
        let config = LoaderRenderConfig::scale_one(false);
        let mut stretched = Surface::new(2, 2, PixelFormat::Rgba8888);
        draw_loader_background(&mut stretched, &background, None, config);

        let mut cropped = Surface::new(2, 2, PixelFormat::Rgba8888);
        draw_loader_background(
            &mut cropped,
            &background,
            None,
            config.with_aspect_fill(true),
        );
        // 2:4 source into a 2:2 target crops 1px off the top and bottom, which
        // makes the remaining 2x2 window an exact one-to-one blit.
        assert_eq!(cropped.get_pixel(0, 0), Some(Color::opaque(0, 255, 0)));
        assert_eq!(cropped.get_pixel(1, 0), Some(Color::opaque(0, 255, 0)));
        assert_eq!(cropped.get_pixel(0, 1), Some(Color::opaque(0, 0, 255)));
        assert_eq!(cropped.get_pixel(1, 1), Some(Color::opaque(0, 0, 255)));
        assert_ne!(stretched.pixels(), cropped.pixels());
        // Default config must stay bit-identical to the C++ stretch.
        let mut default_path = Surface::new(2, 2, PixelFormat::Rgba8888);
        draw_loader_background(
            &mut default_path,
            &background,
            None,
            config.with_aspect_fill(false),
        );
        assert_eq!(default_path.pixels(), stretched.pixels());
    }

    /// Cover-fit, not C++'s letterboxing `fAspect` mode (`C4Facet.cpp:124-133`):
    /// every target pixel must come from the image, with no black bars.
    #[test]
    fn loader_aspect_fill_covers_the_whole_target_without_letterbox_bars() {
        let background = ImageData::new(8, 2, [30, 90, 150, 255].repeat(16));
        let mut covered = Surface::new(2, 8, PixelFormat::Rgba8888);
        draw_loader_background(
            &mut covered,
            &background,
            None,
            LoaderRenderConfig::scale_one(false).with_aspect_fill(true),
        );
        for y in 0..8 {
            for x in 0..2 {
                assert_eq!(
                    covered.get_pixel(x, y),
                    Some(Color::opaque(30, 90, 150)),
                    "aspect fill left a bar at ({x}, {y})"
                );
            }
        }

        // A source that already matches the target aspect is untouched.
        let square = ImageData::new(4, 4, [10, 20, 30, 255].repeat(16));
        let config = LoaderRenderConfig::scale_one(false);
        let mut stretched = Surface::new(9, 9, PixelFormat::Rgba8888);
        let mut filled = Surface::new(9, 9, PixelFormat::Rgba8888);
        draw_loader_background(&mut stretched, &square, None, config);
        draw_loader_background(&mut filled, &square, None, config.with_aspect_fill(true));
        assert_eq!(filled.pixels(), stretched.pixels());
    }

    /// The shipped `LoaderWatercave1.png` / `LoaderGoldmine1.png` are
    /// 3840x2880. On a 16:9 panel C++ squashes 2880 rows into 2160; cover-fit
    /// crops 360 rows off each side instead, which happens to be an unscaled
    /// blit. A 4:3 panel keeps the (already correct) full-source stretch.
    #[test]
    fn loader_aspect_fill_maps_the_shipped_loaders_one_to_one_on_a_16_9_panel() {
        assert_eq!(
            loader_background_source(3840.0, 2880.0, 3840.0, 2160.0, true),
            FloatRect {
                x: 0.0,
                y: 360.0,
                width: 3840.0,
                height: 2160.0,
            }
        );
        assert_eq!(
            loader_background_source(3840.0, 2880.0, 3840.0, 2160.0, false),
            FloatRect {
                x: 0.0,
                y: 0.0,
                width: 3840.0,
                height: 2880.0,
            }
        );
        // 5:4 panel: relatively taller than 4:3, so the sides are cropped.
        assert_eq!(
            loader_background_source(3840.0, 2880.0, 1280.0, 1024.0, true),
            FloatRect {
                x: 120.0,
                y: 0.0,
                width: 3600.0,
                height: 2880.0,
            }
        );
        // Exactly 4:3 keeps the whole facet, with no rounding-driven crop.
        assert_eq!(
            loader_background_source(3840.0, 2880.0, 1024.0, 768.0, true),
            FloatRect {
                x: 0.0,
                y: 0.0,
                width: 3840.0,
                height: 2880.0,
            }
        );
        // Degenerate targets fall back to the full facet rather than dividing
        // by zero.
        for (target_width, target_height) in [(0.0, 8.0), (8.0, 0.0)] {
            assert_eq!(
                loader_background_source(4.0, 4.0, target_width, target_height, true),
                FloatRect {
                    x: 0.0,
                    y: 0.0,
                    width: 4.0,
                    height: 4.0,
                }
            );
        }
    }

    #[test]
    fn exact_size_blit_stays_nearest_when_point_filtering_is_off() {
        let pixels = vec![
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255,
        ];
        let background = ImageData::new(2, 2, pixels.clone());
        let selection = LoaderSelection::scenario("Loader*", "Exact.png").unwrap();
        let loader =
            LoaderScreen::new(selection, background, resources(), LoaderState::initial(""))
                .unwrap();
        let mut target = Surface::new(2, 2, PixelFormat::Rgba8888);
        loader
            .render_chrome(&mut target, LoaderRenderConfig::scale_one(false), None)
            .unwrap();
        assert_eq!(target.pixels(), pixels.as_slice());
    }

    #[test]
    fn public_background_pass_uses_exact_stretch_filter_and_gamma_without_chrome() {
        let background = ImageData::new(
            3,
            2,
            vec![
                240, 20, 40, 255, 20, 220, 60, 255, 30, 40, 200, 255, 80, 100, 120, 255, 180, 140,
                60, 255, 15, 35, 55, 255,
            ],
        );
        let selection = LoaderSelection::scenario("Loader*", "Background.png").unwrap();
        let mut state = LoaderState::initial("Title must not be drawn");
        state.progress = 67;
        state.log = LoaderLog::Visible(vec!["Log must not be drawn".into()]);
        let loader = LoaderScreen::new(selection, background, resources(), state).unwrap();
        let config = LoaderRenderConfig::new(2.0, false).unwrap();
        let gamma = GammaRamp::from_control_points([0x000000, 0x406080, 0xd0e0f0]);

        let mut expected = Surface::new(320, 240, PixelFormat::Rgba8888);
        draw_loader_background(&mut expected, &loader.background, Some(&gamma), config);
        let mut background_only = Surface::new(320, 240, PixelFormat::Rgba8888);
        loader.render_background(&mut background_only, config, Some(&gamma));
        assert_eq!(background_only.pixels(), expected.pixels());

        let mut without_gamma = Surface::new(320, 240, PixelFormat::Rgba8888);
        loader.render_background(&mut without_gamma, config, None);
        assert_ne!(background_only.pixels(), without_gamma.pixels());

        let mut chrome = Surface::new(320, 240, PixelFormat::Rgba8888);
        loader
            .render_chrome(&mut chrome, config, Some(&gamma))
            .unwrap();
        assert_ne!(background_only.pixels(), chrome.pixels());
    }

    #[test]
    fn initial_draw_omits_log_region_and_has_no_invented_footer() {
        let loader = screen(LoaderContext::Scenario, LoaderState::initial("Loading..."));
        let mut rendered = Surface::new(800, 600, PixelFormat::Rgba8888);
        loader.render(&mut rendered, None).expect("render");

        let mut background = Surface::new(800, 600, PixelFormat::Rgba8888);
        draw_loader_background(
            &mut background,
            &loader.background,
            None,
            LoaderRenderConfig::default(),
        );
        // Bottom-left is outside every real loader element when pLog is null.
        for point in [(10, 590), (200, 590), (790, 590)] {
            assert_eq!(
                rendered.get_pixel(point.0, point.1),
                background.get_pixel(point.0, point.1),
                "no copyright/version/footer is drawn at {point:?}"
            );
        }
    }

    #[test]
    fn visible_empty_log_draws_the_inclusive_84px_region() {
        let hidden = synthetic_screen(LoaderState::initial(""), [255, 255, 255, 255]);
        let mut visible = synthetic_screen(LoaderState::initial(""), [255, 255, 255, 255]);
        visible.update(LoaderUpdate::ShowLog);
        let mut hidden_frame = Surface::new(320, 240, PixelFormat::Rgba8888);
        let mut visible_frame = Surface::new(320, 240, PixelFormat::Rgba8888);
        hidden.render(&mut hidden_frame, None).unwrap();
        visible.render(&mut visible_frame, None).unwrap();

        // y=136..=220 at 240px high: 85 pixels due DrawBoxDw's inclusive end.
        assert_eq!(hidden_frame.get_pixel(20, 136).unwrap().r, 255);
        assert_eq!(visible_frame.get_pixel(20, 136).unwrap().r, 127);
        assert_eq!(visible_frame.get_pixel(20, 220).unwrap().r, 127);
        assert_eq!(visible_frame.get_pixel(20, 221).unwrap().r, 255);
    }

    #[test]
    fn log_buffer_draws_only_the_last_cpp_visible_line_count() {
        let mut all = LoaderState::initial("");
        all.log = LoaderLog::Visible(vec![
            "not visible".into(),
            "one".into(),
            "two".into(),
            "three".into(),
            "four".into(),
        ]);
        let mut tail = LoaderState::initial("");
        tail.log = LoaderLog::Visible(vec![
            "one".into(),
            "two".into(),
            "three".into(),
            "four".into(),
        ]);
        // Endeavour FontTiny has line-height 18: (84-4)/18 = four lines.
        let all = synthetic_screen(all, [90, 100, 110, 255]);
        let tail = synthetic_screen(tail, [90, 100, 110, 255]);
        let mut all_frame = Surface::new(480, 320, PixelFormat::Rgba8888);
        let mut tail_frame = Surface::new(480, 320, PixelFormat::Rgba8888);
        all.render(&mut all_frame, None).unwrap();
        tail.render(&mut tail_frame, None).unwrap();
        assert_eq!(all_frame.pixels(), tail_frame.pixels());
    }

    #[test]
    fn string_out_keeps_pipe_text_on_one_row_like_cpp_draw_text() {
        let fonts = endeavour_font_set();
        let mut surface = Surface::new(200, 100, PixelFormat::Rgba8888);
        draw_string_out(
            &fonts.title,
            &mut surface,
            10,
            5,
            "A|B",
            WHITE,
            TextAlign::Left,
            true,
            None,
        );
        // TextOut would put B at y + line_height; StringOut sends the whole
        // string to DrawText, where `|` is an ordinary glyph.
        let first_row_end = 5 + fonts.title.cell_height;
        assert!(surface
            .pixels()
            .chunks_exact(4)
            .enumerate()
            .filter(|(index, _)| index / surface.width() as usize >= first_row_end as usize)
            .all(|(_, pixel)| pixel[3] == 0));
    }

    #[test]
    fn loader_renders_italics_and_consumes_providerless_inline_images_like_cpp() {
        // Keep the tag open so right alignment cannot differ solely because
        // of C++'s trailing-close-tag h-space quirk.
        let italic = synthetic_screen(LoaderState::initial("<i>Loading"), [10, 20, 30, 255]);
        let plain = synthetic_screen(LoaderState::initial("Loading"), [10, 20, 30, 255]);
        let mut italic_frame = Surface::new(320, 240, PixelFormat::Rgba8888);
        let mut plain_frame = Surface::new(320, 240, PixelFormat::Rgba8888);
        italic
            .render(&mut italic_frame, None)
            .expect("valid italic loader title renders");
        plain
            .render(&mut plain_frame, None)
            .expect("plain title renders");
        assert_ne!(italic_frame.pixels(), plain_frame.pixels());

        // Invalid italic syntax is literal in C++ and remains renderable.
        let invalid = synthetic_screen(LoaderState::initial("<i bad>"), [10, 20, 30, 255]);
        invalid.render(&mut italic_frame, None).unwrap();

        let mut image_state = LoaderState::initial("Loading");
        image_state.log = LoaderLog::Visible(vec!["Loading {{CLNK}}".into()]);
        let image = synthetic_screen(image_state, [10, 20, 30, 255]);
        let mut omitted_state = LoaderState::initial("Loading");
        omitted_state.log = LoaderLog::Visible(vec!["Loading ".into()]);
        let omitted = synthetic_screen(omitted_state, [10, 20, 30, 255]);
        let mut image_frame = Surface::new(320, 240, PixelFormat::Rgba8888);
        let mut omitted_frame = Surface::new(320, 240, PixelFormat::Rgba8888);
        image
            .render(&mut image_frame, None)
            .expect("FontTiny consumes unresolved image markup");
        omitted.render(&mut omitted_frame, None).unwrap();
        assert_eq!(image_frame.pixels(), omitted_frame.pixels());

        let native = build_native_font_set(&endeavour_bytes(), 1.5)
            .expect("fractional scale-native loader fonts");
        let mut native_italic = Surface::new(480, 360, PixelFormat::Rgba8888);
        let mut native_plain = Surface::new(480, 360, PixelFormat::Rgba8888);
        italic
            .render_native_text(&mut native_italic, &native, 320, 240, None)
            .expect("scale-native italic loader title renders");
        plain
            .render_native_text(&mut native_plain, &native, 320, 240, None)
            .expect("scale-native plain loader title renders");
        assert_ne!(native_italic.pixels(), native_plain.pixels());

        let mut native_image = Surface::new(480, 360, PixelFormat::Rgba8888);
        let mut native_omitted = Surface::new(480, 360, PixelFormat::Rgba8888);
        image
            .render_native_text(&mut native_image, &native, 320, 240, None)
            .expect("scale-native FontTiny consumes unresolved image markup");
        omitted
            .render_native_text(&mut native_omitted, &native, 320, 240, None)
            .expect("scale-native omitted image control renders");
        assert_eq!(native_image.pixels(), native_omitted.pixels());

        let color = synthetic_screen(
            LoaderState::initial("<c ff0000>Loading</c>"),
            [10, 20, 30, 255],
        );
        color.render(&mut italic_frame, None).unwrap();
    }

    #[test]
    fn logical_chrome_defers_all_text_to_the_native_physical_pass() {
        let mut first_state = LoaderState::initial("First title");
        first_state.progress = 42;
        first_state.log = LoaderLog::Visible(vec!["First log".into()]);
        let mut second_state = first_state.clone();
        second_state.title = "Completely different title".into();
        second_state.log = LoaderLog::Visible(vec!["Different log text".into()]);
        let first = synthetic_screen(first_state, [40, 60, 80, 255]);
        let second = synthetic_screen(second_state, [40, 60, 80, 255]);
        let config = LoaderRenderConfig::new(2.0, true).unwrap();
        let mut first_chrome = Surface::new(320, 240, PixelFormat::Rgba8888);
        let mut second_chrome = Surface::new(320, 240, PixelFormat::Rgba8888);
        first
            .render_chrome(&mut first_chrome, config, None)
            .unwrap();
        second
            .render_chrome(&mut second_chrome, config, None)
            .unwrap();
        assert_eq!(first_chrome.pixels(), second_chrome.pixels());

        let native = build_native_font_set(&endeavour_bytes(), 2).expect("native fonts");
        let mut physical = upscale_nearest(&first_chrome, 2);
        let before = physical.pixels().to_vec();
        first
            .render_native_text(&mut physical, &native, 320, 240, None)
            .unwrap();
        assert_ne!(physical.pixels(), before.as_slice());

        let mut wrong_size = Surface::new(641, 480, PixelFormat::Rgba8888);
        assert_eq!(
            first
                .render_native_text(&mut wrong_size, &native, 320, 240, None)
                .unwrap_err()
                .to_string(),
            "classic loader native text expected a physical surface within a 640x480 viewport whose dimensions round up to 320x240 logical pixels at scale 2, got 641x480"
        );
        let mut too_short = Surface::new(638, 480, PixelFormat::Rgba8888);
        assert_eq!(
            first
                .render_native_text(&mut too_short, &native, 320, 240, None)
                .unwrap_err()
                .to_string(),
            "classic loader native text expected a physical surface within a 640x480 viewport whose dimensions round up to 320x240 logical pixels at scale 2, got 638x480"
        );
    }

    #[test]
    fn native_text_accepts_a_partially_clipped_scaled_viewport() {
        // C4Application::SetResolution rounds the GUI size up after dividing
        // by the application scale, while CStdGL::UpdateClipper lets the
        // scaled viewport extend past the physical framebuffer and relies on
        // OpenGL clipping (C4Application.cpp:536-538; StdGL.cpp:398-407).
        let loader = synthetic_screen(LoaderState::initial("Loading..."), [40, 60, 80, 255]);
        let native = build_native_font_set(&endeavour_bytes(), 3).expect("native fonts");
        let mut physical = Surface::new(960, 598, PixelFormat::Rgba8888);
        let before = physical.pixels().to_vec();

        loader
            .render_native_text(&mut physical, &native, 320, 200, None)
            .expect("the top two rows of the 3x viewport are clipped");

        assert_ne!(physical.pixels(), before.as_slice());
    }

    #[test]
    fn fractional_native_text_uses_ceil_viewport_and_logical_resolution_rules() {
        // 321 * 1.5 and 241 * 1.5 are fractional, so CStdGL installs a
        // 482x362 viewport. A 481x361 framebuffer still maps back to the
        // caller's 321x241 logical resolution and clips one top/right pixel.
        assert_eq!(native_viewport_extent(321, 1.5), Some(482));
        assert_eq!(native_viewport_extent(241, 1.5), Some(362));
        assert_eq!(logical_extent_for(481, 1.5), 321);
        assert_eq!(logical_extent_for(361, 1.5), 241);

        let loader = synthetic_screen(LoaderState::initial("Loading..."), [40, 60, 80, 255]);
        let native =
            build_native_font_set(&endeavour_bytes(), 1.5_f32).expect("fractional native fonts");
        let mut physical = Surface::new(481, 361, PixelFormat::Rgba8888);
        let before = physical.pixels().to_vec();
        loader
            .render_native_text(&mut physical, &native, 321, 241, None)
            .expect("fractional native loader text");
        assert_ne!(physical.pixels(), before.as_slice());

        let mut wrong_logical_width = Surface::new(480, 361, PixelFormat::Rgba8888);
        assert!(loader
            .render_native_text(&mut wrong_logical_width, &native, 321, 241, None)
            .is_err());
    }

    #[test]
    fn native_text_crops_scaled_viewport_overflow_from_the_top_and_right() {
        let loader = synthetic_screen(LoaderState::initial("Loading..."), [40, 60, 80, 255]);
        let native = build_native_font_set(&endeavour_bytes(), 3).expect("native fonts");
        let mut full = Surface::new(960, 600, PixelFormat::Rgba8888);
        for (index, pixel) in full.pixels_mut().chunks_exact_mut(4).enumerate() {
            let x = index % 960;
            let y = index / 960;
            pixel.copy_from_slice(&[x as u8, y as u8, (x ^ y) as u8, 255]);
        }
        let mut clipped = Surface::new(958, 598, PixelFormat::Rgba8888);
        for y in 0..598_usize {
            let source_start = ((y + 2) * 960) * 4;
            let target_start = y * 958 * 4;
            clipped.pixels_mut()[target_start..target_start + 958 * 4]
                .copy_from_slice(&full.pixels()[source_start..source_start + 958 * 4]);
        }

        loader
            .render_native_text(&mut full, &native, 320, 200, None)
            .expect("full viewport text");
        loader
            .render_native_text(&mut clipped, &native, 320, 200, None)
            .expect("clipped viewport text");

        for y in 0..598_usize {
            let source_start = ((y + 2) * 960) * 4;
            let target_start = y * 958 * 4;
            assert_eq!(
                &clipped.pixels()[target_start..target_start + 958 * 4],
                &full.pixels()[source_start..source_start + 958 * 4],
                "physical row {y} must be virtual row {}",
                y + 2
            );
        }
    }

    #[test]
    fn process_without_a_nonempty_visible_log_line_fails_explicitly() {
        let mut loader = screen(LoaderContext::Scenario, LoaderState::initial("Loading..."));
        loader.update(LoaderUpdate::ReplaceLog(vec![String::new()]));
        loader.update(LoaderUpdate::SetProcess(Some(1)));
        let mut surface = Surface::new(320, 240, PixelFormat::Rgba8888);
        assert_eq!(
            loader.render(&mut surface, None).unwrap_err().to_string(),
            "classic loader process suffix requires a visible non-empty log line"
        );
    }

    #[test]
    fn tiny_surface_clips_every_layer_without_panicking() {
        let mut loader = screen(LoaderContext::Startup, LoaderState::initial("Initialize"));
        loader.update(LoaderUpdate::SetProgress(150));
        loader.update(LoaderUpdate::ReplaceLog(vec!["line".into()]));
        loader.update(LoaderUpdate::SetProcess(Some(99)));
        let mut surface = Surface::new(7, 5, PixelFormat::Rgba8888);
        loader
            .render(&mut surface, Some(standard_gamma()))
            .expect("clipped render");
        assert_eq!(surface.pixels().len(), 7 * 5 * 4);
    }

    #[test]
    fn real_loader_jpeg_decode_hash_is_stable_for_decoder_backend() {
        let image = real_image("LoaderSky1.jpg");
        let (expected_jpeg, _) = loader_sky_regression_hashes();

        // This pins the Rust asset input, not a C++ pixel oracle. The pinned
        // C++ build selects WIC or system libjpeg (CMakeLists.txt:202-203,
        // 353-359,529-533), while image's jpeg-decoder selects its SSSE3 IDCT
        // and color conversion at runtime on capable x86 CPUs.
        assert_eq!(fnv1a64(image.pixels()), expected_jpeg);
    }

    #[test]
    fn real_graphics_and_endeavour_frame_hash_is_stable() {
        let mut state = LoaderState::initial("Sky Islands");
        state.progress = 37;
        state.log = LoaderLog::Visible(vec![
            "Clonk Rust is a fan project based on Clonk Rage.".into(),
            "Loading definitions ".into(),
        ]);
        state.process = Some(64);
        let loader = screen(LoaderContext::Scenario, state);
        let mut surface = Surface::new(640, 480, PixelFormat::Rgba8888);
        loader
            .render(&mut surface, Some(standard_gamma()))
            .expect("render real loader frame");
        let (_, expected_frame) = loader_sky_regression_hashes();

        // Snapshot over LoaderSky1.jpg, GUIProgress.png and Endeavour.ttf.
        // C4LoaderScreen::Draw supplies the layer order and geometry
        // (C4LoaderScreen.cpp:126-175), but this is a Rust raster regression,
        // not a C++ framebuffer oracle: C++'s decoder, FreeType and OpenGL
        // backends do not define one portable hash. The paired decode test
        // localizes the scalar/SSSE3 split before this renderer runs.
        // The title's 0xdd alpha uses C++ inverted-alpha blit addition rather
        // than multiplicative modulation, including on filtered edge texels.
        // Re-recorded 2026-07-24 when the trademark log line left the fixture;
        // the renderer is unchanged, only this fixture's line count.
        assert_eq!(fnv1a64(surface.pixels()), expected_frame);
    }

    #[test]
    fn invalid_ttf_never_falls_back_to_a_bitmap_font() {
        let error = build_font_set(b"not a font").err().expect("invalid TTF");
        assert!(error.to_string().contains("failed to load font face"));
    }
}
