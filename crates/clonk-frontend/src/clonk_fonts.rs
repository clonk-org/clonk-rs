//! The GUI font set, mirroring `C4GraphicsResource::InitFonts` /
//! `C4GUI::Resource` (C4GraphicsResource.cpp:144-169, C4GuiResource.h:48-57).
//!
//! All sizes derive from the base font size 14 (`Config.General.RXFontSize`,
//! C4Config.cpp:391) via C4Fonts.cpp:280-288: Log 12, MainSmall 13, Main 14,
//! Caption 16, Title 22.

use anyhow::{Context, Result};
use clonk_graphics::clonk_font::{
    compose_glyph_cell, font_image_lookup_tag, inline_image_token, line_height_for,
    markup_blit_color, scaled_font_image_width, skip_markup_tag, CapturedClonkText,
    CapturedFontImage, ClonkFont, ClonkFontRole, FontImageProvider, FontImageRef, GlyphCell,
    TextAlign,
};
use clonk_graphics::{
    ClipperProjection, Color, GammaRamp, GpuBlend, GpuCommand, GpuOuterModulation, GpuSampler,
    GpuVertex, Surface, SurfaceDrawTarget,
};
use clonk_gui::{ImageData, Rect as GuiRect};
use freetype::face::LoadFlag;
use freetype::{Library, Matrix, Vector};
use std::collections::{BTreeSet, HashMap};
use std::sync::Mutex;

/// The five GUI fonts the startup menus draw with.
pub struct ClonkFontSet {
    /// C4FT_Title (22px) — `C4GUI::Resource::TitleFont`.
    pub title: ClonkFont,
    /// C4FT_Caption (16px) — `CaptionFont`.
    pub caption: ClonkFont,
    /// C4FT_Main (14px) — `TextFont`.
    pub text: ClonkFont,
    /// C4FT_MainSmall (13px) — used by the startup book fonts.
    pub main_small: ClonkFont,
    /// C4FT_Log (12px) — `MiniFont`.
    pub mini: ClonkFont,
}

/// One CStdFont rasterized at the application's output scale.
///
/// C++ keeps the glyph atlas in physical pixels while all public metrics and
/// draw coordinates remain in GUI units (`StdFont.cpp:319-352,571-638,841-842,
/// 938`). The Rust renderer uses this type only for the physical overlay pass;
/// the ordinary [`ClonkFontSet`] remains the scale-1 logical renderer.
pub struct NativeClonkFont {
    raster: ClonkFont,
    application_scale: f32,
    effective_scale: f32,
    logical_height: u32,
    raster_height: u32,
    logical_h_space: i32,
    /// `Graphics.SnapTextToPixels`: this atlas was rasterized at the rounded
    /// physical height, so a whole-pixel origin gives a 1:1 texel blit.
    snap_to_pixels: bool,
    /// Stable per-glyph retained resources. C++ keeps these cells resident in
    /// a font atlas; the Rust backend gives each immutable cell a stable GPU
    /// identity so replay emits one quad per glyph without CPU resampling.
    retained_glyph_images: Mutex<HashMap<char, NativeGlyphImages>>,
}

#[derive(Clone)]
struct NativeGlyphImages {
    cpu: ImageData,
    /// One transparent texel around the cell preserves the C++/software
    /// bilinear footprint without leaking an adjacent glyph.
    retained: ImageData,
}

#[derive(Clone, Copy)]
struct NativeDrawProjection {
    scale_x: f64,
    scale_y: f64,
    offset_x: f64,
    offset_y: f64,
}

impl NativeDrawProjection {
    fn application(scale: f32, offset: (i32, i32)) -> Self {
        Self {
            scale_x: f64::from(scale),
            scale_y: f64::from(scale),
            offset_x: f64::from(offset.0),
            offset_y: f64::from(offset.1),
        }
    }

    fn clipper(projection: ClipperProjection) -> Self {
        let (scale_x, scale_y) = projection.scale();
        let (offset_x, offset_y) = projection.logical_to_physical(0.0, 0.0);
        Self {
            scale_x,
            scale_y,
            offset_x,
            offset_y,
        }
    }

    fn project(self, x: i32, y: i32) -> (f64, f64) {
        (
            f64::from(x) * self.scale_x + self.offset_x,
            f64::from(y) * self.scale_y + self.offset_y,
        )
    }

    fn requires_resampling(self, effective_scale: f32) -> bool {
        let effective_scale = f64::from(effective_scale);
        let differs = |left: f64, right: f64| (left - right).abs() > 1.0e-6;
        differs(self.scale_x, effective_scale)
            || differs(self.scale_y, effective_scale)
            || differs(self.scale_x, self.scale_x.round())
            || differs(self.scale_y, self.scale_y.round())
            || differs(self.offset_x, self.offset_x.round())
            || differs(self.offset_y, self.offset_y.round())
    }
}

impl NativeClonkFont {
    /// True when `Graphics.SnapTextToPixels` is on *and* this atlas really was
    /// rasterized for the projection about to draw it, i.e. `raster_height` is
    /// the rounded projected height on both axes. A mismatched projection (a
    /// stale font set, or an anisotropic clipper far from the build scale)
    /// still takes the C++-exact resampling path.
    fn snaps_to_physical_pixels(&self, projection: NativeDrawProjection) -> bool {
        let logical = f64::from(self.logical_height);
        let raster = f64::from(self.raster_height);
        let rounds_to_raster =
            |projection_scale: f64| (projection_scale * logical - raster).abs() <= 0.5 + 1.0e-6;
        self.snap_to_pixels
            && logical > 0.0
            && rounds_to_raster(projection.scale_x)
            && rounds_to_raster(projection.scale_y)
    }

    /// Exact rational denominator used by CStdFont's scale-native GUI metrics.
    ///
    /// A glyph facet of width `W` occupies `W * logical_height` of these units;
    /// dividing by `raster_height` is exactly `W / effective_scale` without
    /// losing precision to an early floating-point conversion.
    pub(crate) fn message_width_units_per_gui_pixel(&self) -> i32 {
        i32::try_from(self.raster_height).unwrap_or(i32::MAX)
    }

    /// One `BreakMessage` character advance in exact raster-height units.
    /// C++ accumulates `facet.Wdt / scale + iHSpace`, where the shadowed
    /// font's `iHSpace` remains -1 GUI pixel (`StdFont.cpp:640-760`). Keeping
    /// the numerator avoids losing the fractional width before the wrap test.
    pub(crate) fn message_character_advance_units(&self, character: char) -> i32 {
        if character < ' ' {
            return 0;
        }
        let raster_width = self
            .raster
            .rendered_glyph(character)
            .map_or(0, |glyph| glyph.width);
        let logical_height = i32::try_from(self.logical_height).unwrap_or(i32::MAX);
        let raster_height = self.message_width_units_per_gui_pixel();
        raster_width
            .saturating_mul(logical_height)
            .saturating_add(self.logical_h_space.saturating_mul(raster_height))
    }

    pub(crate) fn message_image_advance_units(&self, image: FontImageRef<'_>) -> i32 {
        scaled_font_image_width(self.raster.cell_height, image)
            .saturating_mul(i32::try_from(self.logical_height).unwrap_or(i32::MAX))
    }

    /// Configured application scale passed to `CStdFont::Init`.
    pub fn application_scale(&self) -> f32 {
        self.application_scale
    }

    /// Scale C++ stores after truncating the requested raster height:
    /// `floor(logical_height * application_scale) / logical_height`.
    pub fn effective_scale(&self) -> f32 {
        self.effective_scale
    }

    /// Logical FreeType height requested by the C4 font role.
    pub fn logical_height(&self) -> u32 {
        self.logical_height
    }

    /// FreeType pixel height after C++'s positive float-to-integer truncation.
    pub fn raster_height(&self) -> u32 {
        self.raster_height
    }

    /// CStdFont's internal `iLineHgt`, in physical atlas pixels.
    pub fn raster_line_height(&self) -> i32 {
        self.raster.line_height
    }

    /// CStdFont's internal `iGfxLineHgt`, in physical atlas pixels.
    pub fn raster_cell_height(&self) -> i32 {
        self.raster.cell_height
    }

    /// `CStdFont::GetLineHeight`: internal height divided by effective scale.
    pub fn logical_line_height(&self) -> i32 {
        (self.raster.line_height as f32 / self.effective_scale) as i32
    }

    pub fn glyph(&self, ch: char) -> Option<&GlyphCell> {
        self.raster.glyph(ch)
    }

    fn retained_glyph_images(&self, character: char) -> Option<NativeGlyphImages> {
        // Every unmapped scalar resolves to the same FreeType glyph-zero cell.
        // Cache that immutable resource once instead of allocating one GPU id
        // per arbitrary Unicode scalar encountered in chat or scenario text.
        let cache_key = if self.raster.glyph(character).is_some() {
            character
        } else {
            '\0'
        };
        if let Some(images) = self
            .retained_glyph_images
            .lock()
            .expect("native glyph cache poisoned")
            .get(&cache_key)
            .cloned()
        {
            return Some(images);
        }

        let glyph = self.raster.rendered_glyph(character)?;
        let width = u32::try_from(glyph.width.max(0)).ok()?;
        let height = u32::try_from(self.raster.cell_height.max(0)).ok()?;
        if width == 0 || height == 0 {
            return None;
        }
        let pixel_count = usize::try_from(width)
            .ok()?
            .checked_mul(usize::try_from(height).ok()?)?;
        let mut rgba = Vec::with_capacity(pixel_count.checked_mul(4)?);
        for index in 0..pixel_count {
            let pixel = glyph.pixels.get(index).copied().unwrap_or_default();
            rgba.extend_from_slice(&[pixel.r, pixel.g, pixel.b, pixel.a]);
        }
        let cpu = ImageData::new(width, height, rgba);

        let retained_width = width.checked_add(2)?;
        let retained_height = height.checked_add(2)?;
        let retained_len = usize::try_from(retained_width)
            .ok()?
            .checked_mul(usize::try_from(retained_height).ok()?)?
            .checked_mul(4)?;
        let mut retained_rgba = vec![0_u8; retained_len];
        for row in 0..height {
            let source_start = usize::try_from(row)
                .ok()?
                .checked_mul(usize::try_from(width).ok()?)?
                .checked_mul(4)?;
            let source_end =
                source_start.checked_add(usize::try_from(width).ok()?.checked_mul(4)?)?;
            let destination_start = usize::try_from(row.checked_add(1)?)
                .ok()?
                .checked_mul(usize::try_from(retained_width).ok()?)?
                .checked_add(1)?
                .checked_mul(4)?;
            let destination_end = destination_start.checked_add(source_end - source_start)?;
            retained_rgba
                .get_mut(destination_start..destination_end)?
                .copy_from_slice(cpu.pixels().get(source_start..source_end)?);
        }
        let images = NativeGlyphImages {
            cpu,
            retained: ImageData::new(retained_width, retained_height, retained_rgba),
        };
        let mut cache = self
            .retained_glyph_images
            .lock()
            .expect("native glyph cache poisoned");
        Some(
            cache
                .entry(cache_key)
                .or_insert_with(|| images.clone())
                .clone(),
        )
    }

    /// `CStdFont::GetTextExtent` in GUI units. Each physical glyph width is
    /// divided by this font's effective scale while `iHSpace` remains one
    /// logical pixel, including when height truncation makes the two scales
    /// differ.
    pub fn measure(&self, text: &str, markup: bool) -> (i32, i32) {
        self.measure_impl(text, markup, None)
    }

    pub fn measure_with_images(
        &self,
        text: &str,
        markup: bool,
        images: &dyn FontImageProvider,
    ) -> (i32, i32) {
        self.measure_impl(text, markup, Some(images))
    }

    fn measure_impl(
        &self,
        text: &str,
        markup: bool,
        images: Option<&dyn FontImageProvider>,
    ) -> (i32, i32) {
        let line_step_height =
            (self.raster.line_height as f32 / self.effective_scale).ceil() as i32;
        let mut rest = text;
        let mut row_width = 0.0_f32;
        let mut width = 0.0_f32;
        let mut height = line_step_height;
        loop {
            if markup {
                while let Some(advance) = skip_markup_tag(rest) {
                    rest = &rest[advance..];
                }
            }
            if rest.is_empty() {
                break;
            }
            if markup {
                if let Some((tag, advance)) = inline_image_token(rest) {
                    let image_width = images
                        .and_then(|provider| provider.font_image(font_image_lookup_tag(tag)))
                        .map_or(0, |image| {
                            scaled_font_image_width(self.raster.cell_height, image)
                        });
                    row_width += image_width as f32 / self.effective_scale;
                    rest = &rest[advance..];
                    if !rest.is_empty() {
                        row_width += self.logical_h_space as f32;
                    }
                    width = width.max(row_width);
                    continue;
                }
            }
            let mut characters = rest.chars();
            let Some(character) = characters.next() else {
                break;
            };
            rest = characters.as_str();
            if character == '\n' || (markup && character == '|') {
                row_width = 0.0;
                height = height.saturating_add(line_step_height);
                continue;
            }
            if character < ' ' {
                continue;
            }
            row_width += self
                .raster
                .rendered_glyph(character)
                .map_or(0, |glyph| glyph.width) as f32
                / self.effective_scale;
            if !rest.is_empty() {
                row_width += self.logical_h_space as f32;
            }
            width = width.max(row_width);
        }
        (width as i32, height)
    }

    /// Draw a native-resolution glyph run onto a physical surface while
    /// accepting C++ GUI-unit coordinates.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_to_physical_surface(
        &self,
        surface: &mut Surface,
        x: i32,
        y: i32,
        text: &str,
        color: [u8; 4],
        align: TextAlign,
        markup: bool,
        gamma: Option<&GammaRamp>,
    ) {
        self.draw_to_physical_surface_with_offset(
            surface,
            x,
            y,
            text,
            color,
            align,
            markup,
            (0, 0),
            gamma,
        );
    }

    /// Physical-surface draw with the framebuffer offset of C++'s GL
    /// viewport. A negative Y offset represents rows clipped from the top.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_to_physical_surface_with_offset(
        &self,
        surface: &mut Surface,
        x: i32,
        y: i32,
        text: &str,
        color: [u8; 4],
        align: TextAlign,
        markup: bool,
        physical_offset: (i32, i32),
        gamma: Option<&GammaRamp>,
    ) {
        self.draw_to_physical_surface_with_projection_impl(
            surface,
            x,
            y,
            text,
            color,
            align,
            markup,
            NativeDrawProjection::application(self.application_scale, physical_offset),
            gamma,
            None,
            1.0,
        );
    }

    /// [`Self::draw_to_physical_surface_with_offset`] with custom images.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_to_physical_surface_with_offset_and_images(
        &self,
        surface: &mut Surface,
        x: i32,
        y: i32,
        text: &str,
        color: [u8; 4],
        align: TextAlign,
        markup: bool,
        physical_offset: (i32, i32),
        gamma: Option<&GammaRamp>,
        images: &dyn FontImageProvider,
    ) {
        self.draw_to_physical_surface_with_projection_impl(
            surface,
            x,
            y,
            text,
            color,
            align,
            markup,
            NativeDrawProjection::application(self.application_scale, physical_offset),
            gamma,
            Some(images),
            1.0,
        );
    }

    /// Draw through the exact rounded viewport and orthographic projection of
    /// CStdGL's active primary clipper (`StdGL.cpp:402-407`).
    #[allow(clippy::too_many_arguments)]
    pub fn draw_to_physical_surface_with_clipper(
        &self,
        surface: &mut Surface,
        x: i32,
        y: i32,
        text: &str,
        color: [u8; 4],
        align: TextAlign,
        markup: bool,
        projection: ClipperProjection,
        gamma: Option<&GammaRamp>,
    ) {
        self.draw_to_physical_surface_with_clipper_to(
            surface, x, y, text, color, align, markup, projection, gamma,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw_to_physical_surface_with_clipper_to<T: SurfaceDrawTarget + ?Sized>(
        &self,
        surface: &mut T,
        x: i32,
        y: i32,
        text: &str,
        color: [u8; 4],
        align: TextAlign,
        markup: bool,
        projection: ClipperProjection,
        gamma: Option<&GammaRamp>,
    ) {
        self.draw_to_physical_surface_with_projection_impl(
            surface,
            x,
            y,
            text,
            color,
            align,
            markup,
            NativeDrawProjection::clipper(projection),
            gamma,
            None,
            1.0,
        );
    }

    /// [`Self::draw_to_physical_surface_with_clipper`] with custom images.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_to_physical_surface_with_clipper_and_images(
        &self,
        surface: &mut Surface,
        x: i32,
        y: i32,
        text: &str,
        color: [u8; 4],
        align: TextAlign,
        markup: bool,
        projection: ClipperProjection,
        gamma: Option<&GammaRamp>,
        images: &dyn FontImageProvider,
    ) {
        self.draw_to_physical_surface_with_clipper_and_images_to(
            surface, x, y, text, color, align, markup, projection, gamma, images,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw_to_physical_surface_with_clipper_and_images_to<T: SurfaceDrawTarget + ?Sized>(
        &self,
        surface: &mut T,
        x: i32,
        y: i32,
        text: &str,
        color: [u8; 4],
        align: TextAlign,
        markup: bool,
        projection: ClipperProjection,
        gamma: Option<&GammaRamp>,
        images: &dyn FontImageProvider,
    ) {
        self.draw_to_physical_surface_with_projection_impl(
            surface,
            x,
            y,
            text,
            color,
            align,
            markup,
            NativeDrawProjection::clipper(projection),
            gamma,
            Some(images),
            1.0,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_to_physical_surface_with_projection_impl<T: SurfaceDrawTarget + ?Sized>(
        &self,
        surface: &mut T,
        x: i32,
        y: i32,
        text: &str,
        color: [u8; 4],
        align: TextAlign,
        markup: bool,
        projection: NativeDrawProjection,
        gamma: Option<&GammaRamp>,
        images: Option<&dyn FontImageProvider>,
        zoom: f32,
    ) {
        // `fZoom` scales the alignment offset and the per-line advance as well
        // as the glyphs (`src/StdFont.cpp:831,838`; `src/StdDDraw2.cpp:1039`).
        let line_height = (zoom * self.logical_line_height() as f32) as i32;
        let origins = text
            .split(|character: char| character == '\n' || (markup && character == '|'))
            .enumerate()
            .map(|(line_index, line)| {
                let logical_width = self.measure_impl(line, markup, images).0;
                let offset = match align {
                    TextAlign::Left => 0,
                    TextAlign::Center => logical_width / 2,
                    TextAlign::Right => logical_width,
                };
                let logical_left = x.saturating_sub((zoom * offset as f32) as i32);
                let line_index = i32::try_from(line_index).unwrap_or(i32::MAX);
                let logical_y = y.saturating_add(line_index.saturating_mul(line_height));
                projection.project(logical_left, logical_y)
            })
            .collect::<Vec<_>>();
        let snaps = self.snaps_to_physical_pixels(projection);
        if surface.is_gpu_command_capture_active()
            || (!snaps && projection.requires_resampling(self.effective_scale))
        {
            self.draw_fractional_lines_at_origins(
                surface, &origins, text, color, markup, gamma, images, projection, zoom,
            );
            return;
        }
        let origins = origins
            .into_iter()
            .map(|(x, y)| (physical_integer(x), physical_integer(y)))
            .collect::<Vec<_>>();
        if let Some(images) = images {
            self.raster.draw_lines_at_origins_with_gamma_and_images_to(
                surface, &origins, text, color, markup, gamma, images,
            );
        } else {
            self.raster.draw_zoomed_lines_at_origins_with_gamma_to(
                surface, &origins, text, color, markup, gamma, zoom,
            );
        }
    }

    /// `CStdDDraw::StringOut` variant. Alignment uses `GetTextExtent` (where
    /// newline and markup-enabled `|` are virtual breaks), but the one
    /// `CStdFont::DrawText` call ignores newline and draws `|` on the current
    /// row. This differs deliberately from [`Self::draw_to_physical_surface`]
    /// and is used by `C4LoaderScreen` title/progress strings.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_string_to_physical_surface(
        &self,
        surface: &mut Surface,
        x: i32,
        y: i32,
        text: &str,
        color: [u8; 4],
        align: TextAlign,
        markup: bool,
        gamma: Option<&GammaRamp>,
    ) {
        self.draw_string_to_physical_surface_with_offset(
            surface,
            x,
            y,
            text,
            color,
            align,
            markup,
            (0, 0),
            gamma,
        );
    }

    /// [`Self::draw_string_to_physical_surface`] with the framebuffer offset
    /// of an oversized C++ GL viewport.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_string_to_physical_surface_with_offset(
        &self,
        surface: &mut Surface,
        x: i32,
        y: i32,
        text: &str,
        color: [u8; 4],
        align: TextAlign,
        markup: bool,
        physical_offset: (i32, i32),
        gamma: Option<&GammaRamp>,
    ) {
        self.draw_string_to_physical_surface_with_projection_impl(
            surface,
            x,
            y,
            text,
            color,
            align,
            markup,
            NativeDrawProjection::application(self.application_scale, physical_offset),
            gamma,
        );
    }

    /// StringOut through CStdGL's rounded clipper viewport and orthographic
    /// projection. This is required at fractional application scales, where
    /// the full viewport extent may not equal `logical * scale` exactly.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_string_to_physical_surface_with_clipper(
        &self,
        surface: &mut Surface,
        x: i32,
        y: i32,
        text: &str,
        color: [u8; 4],
        align: TextAlign,
        markup: bool,
        projection: ClipperProjection,
        gamma: Option<&GammaRamp>,
    ) {
        self.draw_string_to_physical_surface_with_clipper_to(
            surface, x, y, text, color, align, markup, projection, gamma,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw_string_to_physical_surface_with_clipper_to<T: SurfaceDrawTarget + ?Sized>(
        &self,
        surface: &mut T,
        x: i32,
        y: i32,
        text: &str,
        color: [u8; 4],
        align: TextAlign,
        markup: bool,
        projection: ClipperProjection,
        gamma: Option<&GammaRamp>,
    ) {
        self.draw_string_to_physical_surface_with_projection_impl(
            surface,
            x,
            y,
            text,
            color,
            align,
            markup,
            NativeDrawProjection::clipper(projection),
            gamma,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_string_to_physical_surface_with_projection_impl<T: SurfaceDrawTarget + ?Sized>(
        &self,
        surface: &mut T,
        x: i32,
        y: i32,
        text: &str,
        color: [u8; 4],
        align: TextAlign,
        markup: bool,
        projection: NativeDrawProjection,
        gamma: Option<&GammaRamp>,
    ) {
        let (logical_width, _) = self.measure(text, markup);
        let logical_left = x.saturating_sub(match align {
            TextAlign::Left => 0,
            TextAlign::Center => logical_width / 2,
            TextAlign::Right => logical_width,
        });
        let physical_origin = projection.project(logical_left, y);
        if surface.is_gpu_command_capture_active()
            || (!self.snaps_to_physical_pixels(projection)
                && projection.requires_resampling(self.effective_scale))
        {
            // StringOut invokes one CStdFont::DrawText call. Newlines are
            // ignored by DrawText and markup-enabled pipes remain ordinary
            // glyphs; only GetTextExtent treats them as virtual line breaks.
            let transformed = text
                .chars()
                .filter(|character| *character != '\n')
                .collect::<String>();
            self.draw_fractional_line(
                surface,
                physical_origin.0,
                physical_origin.1,
                &transformed,
                color,
                markup,
                gamma,
                None,
                &mut Vec::new(),
                projection,
                1.0,
            );
            return;
        }
        if !text.contains('\n') && (!markup || !text.contains('|')) {
            self.raster.draw_with_gamma_to(
                surface,
                physical_integer(physical_origin.0),
                physical_integer(physical_origin.1),
                text,
                color,
                TextAlign::Left,
                markup,
                gamma,
            );
            return;
        }

        let sentinel = ('\u{E000}'..='\u{F8FF}')
            .find(|candidate| !text.contains(*candidate))
            .unwrap_or('\u{10FFFD}');
        let mut raster = self.raster.clone();
        if let Some(pipe) = self.raster.glyph('|').cloned() {
            raster.add_glyph(sentinel, pipe);
        }
        let transformed: String = text
            .chars()
            .filter_map(|character| match character {
                '\n' => None,
                '|' if markup => Some(sentinel),
                other => Some(other),
            })
            .collect();
        raster.draw_with_gamma_to(
            surface,
            physical_integer(physical_origin.0),
            physical_integer(physical_origin.1),
            &transformed,
            color,
            TextAlign::Left,
            markup,
            gamma,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_fractional_lines_at_origins<T: SurfaceDrawTarget + ?Sized>(
        &self,
        surface: &mut T,
        origins: &[(f64, f64)],
        text: &str,
        color: [u8; 4],
        markup: bool,
        gamma: Option<&GammaRamp>,
        images: Option<&dyn FontImageProvider>,
        projection: NativeDrawProjection,
        zoom: f32,
    ) {
        let mut stack = Vec::new();
        for ((x, y), line) in origins
            .iter()
            .copied()
            .zip(text.split(|character: char| character == '\n' || (markup && character == '|')))
        {
            self.draw_fractional_line(
                surface, x, y, line, color, markup, gamma, images, &mut stack, projection, zoom,
            );
        }
    }

    /// Draw one CStdFont line after the application transform. C++ divides
    /// each source facet by the font's truncated effective scale and then the
    /// global graphics transform multiplies it by Application.GetScale().
    /// Those factors cancel at integer/exact scales, but not for e.g.
    /// FontMainSmall's `floor(13 * 1.5) / 13` effective scale.
    #[allow(clippy::too_many_arguments)]
    fn draw_fractional_line<T: SurfaceDrawTarget + ?Sized>(
        &self,
        surface: &mut T,
        x: f64,
        y: f64,
        line: &str,
        color: [u8; 4],
        markup: bool,
        gamma: Option<&GammaRamp>,
        images: Option<&dyn FontImageProvider>,
        stack: &mut Vec<NativeMarkupTag>,
        projection: NativeDrawProjection,
        zoom: f32,
    ) {
        // `Graphics.SnapTextToPixels` draws the atlas at its rasterized size
        // from a whole-pixel pen instead of rescaling every facet by
        // `scale / effective_scale` (StdFont.cpp:685,701,841).
        let snaps = self.snaps_to_physical_pixels(projection);
        let (quad_scale_x, quad_scale_y) = if snaps {
            (f64::from(zoom), f64::from(zoom))
        } else {
            (
                f64::from(zoom) * projection.scale_x / f64::from(self.effective_scale),
                f64::from(zoom) * projection.scale_y / f64::from(self.effective_scale),
            )
        };
        let physical_spacing = if snaps {
            (f64::from(self.logical_h_space) * projection.scale_x).round()
        } else {
            f64::from(self.logical_h_space) * projection.scale_x
        };
        let (x, y) = if snaps {
            (x.round(), y.round())
        } else {
            (x, y)
        };
        let mut pen_x = x;
        let pen_y = y;
        let mut rest = line;
        let mut transform_active = !stack.is_empty();

        while let Some(character) = rest.chars().next() {
            let after = &rest[character.len_utf8()..];
            if character < ' ' {
                rest = after;
                continue;
            }
            if markup && character == '<' {
                if let Some(advance) = read_native_markup_tag(rest, stack) {
                    transform_active = true;
                    rest = &rest[advance..];
                    continue;
                }
            }
            if markup && character == '{' {
                if let Some((tag, advance)) = inline_image_token(rest) {
                    rest = &rest[advance..];
                    let Some(image) =
                        images.and_then(|provider| provider.font_image(font_image_lookup_tag(tag)))
                    else {
                        continue;
                    };
                    if image.height == 0 {
                        continue;
                    }
                    let raw_height = self.raster.cell_height.max(0);
                    let raw_width =
                        f64::from(image.width) * f64::from(raw_height) / f64::from(image.height);
                    if raw_width > 0.0 && raw_height > 0 {
                        blit_scaled_native_image(
                            surface,
                            image,
                            pen_x,
                            pen_y,
                            raw_width * quad_scale_x,
                            f64::from(raw_height) * quad_scale_y,
                            native_image_modulation_rgb(stack, color, transform_active),
                            color[3],
                            gamma,
                            native_physical_shear(stack, projection),
                        );
                    }
                    pen_x += raw_width * quad_scale_x + physical_spacing;
                    continue;
                }
            }

            rest = after;
            let cell = self.raster.rendered_glyph(character);
            let raw_width = cell.map_or(0, |glyph| glyph.width).max(0);
            let raw_height = self.raster.cell_height.max(0);
            if let Some(images) = (raw_width > 0 && raw_height > 0)
                .then(|| self.retained_glyph_images(character))
                .flatten()
            {
                blit_scaled_native_glyph(
                    surface,
                    &images,
                    self.raster.texture_size(),
                    pen_x,
                    pen_y,
                    f64::from(raw_width) * quad_scale_x,
                    f64::from(raw_height) * quad_scale_y,
                    native_modulation_rgb(stack, color),
                    color[3],
                    gamma,
                    native_physical_shear(stack, projection),
                );
            }
            pen_x += f64::from(raw_width) * quad_scale_x + physical_spacing;
        }
    }
}

fn physical_integer(value: f64) -> i32 {
    value
        .round()
        .clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NativeMarkupTag {
    Italic,
    TextColor(u32),
}

impl NativeMarkupTag {
    const fn name(self) -> &'static str {
        match self {
            Self::Italic => "i",
            Self::TextColor(_) => "c",
        }
    }
}

/// Draw-mode counterpart of `CMarkup::Read`. The scale-native renderer needs
/// the live color stack because it submits each resized glyph/image quad
/// separately instead of delegating one unscaled line to `ClonkFont`.
fn read_native_markup_tag(text: &str, stack: &mut Vec<NativeMarkupTag>) -> Option<usize> {
    let inner = text.strip_prefix('<')?;
    let close = inner.find('>')?;
    let full = &inner[..close];
    let mut tag_len = full.len().min(49);
    while !full.is_char_boundary(tag_len) {
        tag_len -= 1;
    }
    let tag = &full[..tag_len];
    let mut advance = (tag_len + 2).min(text.len());
    while !text.is_char_boundary(advance) {
        advance += 1;
    }
    let (name, parameters) = match tag.find(' ') {
        Some(index) => (&tag[..index], Some(&tag[index + 1..])),
        None => (tag, None),
    };
    let valid = if let Some(closing) = name.strip_prefix('/') {
        parameters.is_none()
            && match stack.last() {
                Some(tag) if tag.name() == closing => {
                    stack.pop();
                    true
                }
                _ => false,
            }
    } else if name == "i" {
        parameters.is_none() && {
            stack.push(NativeMarkupTag::Italic);
            true
        }
    } else if name == "c" {
        parameters
            .filter(|parameters| parameters.len() <= 8)
            .and_then(parse_native_color_tag)
            .map(|color| stack.push(NativeMarkupTag::TextColor(color)))
            .is_some()
    } else {
        false
    };
    valid.then_some(advance)
}

fn parse_native_color_tag(parameters: &str) -> Option<u32> {
    let len = parameters.len();
    parameters
        .bytes()
        .enumerate()
        .try_fold(0_u32, |color, (index, byte)| {
            let digit = match byte {
                b'0'..=b'9' => byte - b'0',
                b'a'..=b'f' => byte - b'a' + 10,
                _ => return None,
            };
            Some(color | (u32::from(digit) << ((len - index - 1) * 4)))
        })
        .map(|color| {
            let color = if len <= 6 { color | 0xff00_0000 } else { color };
            (color & 0x00ff_ffff) | ((255 - (color >> 24)) << 24)
        })
}

fn native_modulation_rgb(stack: &[NativeMarkupTag], color: [u8; 4]) -> [u8; 3] {
    let base = (u32::from(255 - color[3]) << 24)
        | (u32::from(color[0]) << 16)
        | (u32::from(color[1]) << 8)
        | u32::from(color[2]);
    stack
        .iter()
        .rev()
        .find_map(|tag| match tag {
            NativeMarkupTag::TextColor(color) => Some(*color),
            NativeMarkupTag::Italic => None,
        })
        .filter(|tag_color| *tag_color != base)
        .map(|tag_color| {
            markup_blit_color([
                (tag_color >> 16) as u8,
                (tag_color >> 8) as u8,
                tag_color as u8,
            ])
        })
        .unwrap_or([color[0], color[1], color[2]])
}

fn native_physical_shear(stack: &[NativeMarkupTag], projection: NativeDrawProjection) -> f64 {
    let logical_shear = stack
        .iter()
        .filter(|tag| matches!(tag, NativeMarkupTag::Italic))
        .fold(0.0_f32, |shear, _| shear - 0.3);
    if projection.scale_y == 0.0 {
        0.0
    } else {
        f64::from(logical_shear) * projection.scale_x / projection.scale_y
    }
}

fn native_image_modulation_rgb(
    stack: &[NativeMarkupTag],
    color: [u8; 4],
    transform_active: bool,
) -> [u8; 3] {
    if stack
        .iter()
        .any(|tag| matches!(tag, NativeMarkupTag::TextColor(_)))
    {
        native_modulation_rgb(stack, color)
    } else if transform_active {
        [color[0], color[1], color[2]]
    } else {
        [255, 255, 255]
    }
}

#[allow(clippy::too_many_arguments)]
fn blit_scaled_native_glyph<T: SurfaceDrawTarget + ?Sized>(
    surface: &mut T,
    images: &NativeGlyphImages,
    physical_texture_size: i32,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    modulation: [u8; 3],
    color_alpha: u8,
    gamma: Option<&GammaRamp>,
    shear: f64,
) {
    draw_scaled_native_image(
        surface,
        &images.cpu,
        Some((&images.retained, [1.0, 1.0])),
        x,
        y,
        width,
        height,
        gamma,
        shear,
        modulation,
        color_alpha,
        Some(physical_texture_size),
    );
}

#[allow(clippy::too_many_arguments)]
fn blit_scaled_native_image<T: SurfaceDrawTarget + ?Sized>(
    surface: &mut T,
    image: FontImageRef<'_>,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    modulation: [u8; 3],
    color_alpha: u8,
    gamma: Option<&GammaRamp>,
    shear: f64,
) {
    let pixels = image.rgba.chunks_exact(4).flatten().copied().collect();
    let image = retained_font_image(image.width, image.height, pixels);
    draw_scaled_native_image(
        surface,
        &image,
        None,
        x,
        y,
        width,
        height,
        gamma,
        shear,
        modulation,
        color_alpha,
        None,
    );
}

/// Return one stable retained texture identity for immutable font/source RGBA.
///
/// Several classic text paths must first build an isolated CPU source before
/// applying spatial ClrModMap modulation. Canonicalizing that source by exact
/// dimensions and bytes keeps repeated frames on one renderer-cache entry
/// without changing the software oracle's pixels.
pub(crate) fn retained_font_image(width: u32, height: u32, pixels: Vec<u8>) -> ImageData {
    // ImageData globally interns immutable dimensions + exact RGBA with a
    // collision check and bounded retention. Keep this helper as the font
    // boundary without maintaining a second unbounded cache.
    ImageData::new(width, height, pixels)
}

/// Add the transparent one-texel font-atlas gutter used by retained glyph
/// cells. The gutter prevents linear filtering from clamping to the glyph's
/// last opaque texel when its logical atlas tile is larger than the cell.
pub(crate) fn retained_padded_font_image(image: &ImageData) -> Option<ImageData> {
    let width = image.width();
    let height = image.height();
    let retained_width = width.checked_add(2)?;
    let retained_height = height.checked_add(2)?;
    let retained_len = usize::try_from(retained_width)
        .ok()?
        .checked_mul(usize::try_from(retained_height).ok()?)?
        .checked_mul(4)?;
    let mut pixels = vec![0_u8; retained_len];
    for row in 0..height {
        let source_start = usize::try_from(row)
            .ok()?
            .checked_mul(usize::try_from(width).ok()?)?
            .checked_mul(4)?;
        let source_end = source_start.checked_add(usize::try_from(width).ok()?.checked_mul(4)?)?;
        let destination_start = usize::try_from(row.checked_add(1)?)
            .ok()?
            .checked_mul(usize::try_from(retained_width).ok()?)?
            .checked_add(1)?
            .checked_mul(4)?;
        let destination_end = destination_start.checked_add(source_end - source_start)?;
        pixels
            .get_mut(destination_start..destination_end)?
            .copy_from_slice(image.pixels().get(source_start..source_end)?);
    }
    Some(retained_font_image(retained_width, retained_height, pixels))
}

/// Retained counterpart of the CStdFont facet draw used by presentation code
/// that has already resolved markup. GPU targets receive one stable textured
/// quad; ordinary targets continue through the exact software sampler.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_retained_font_image<T: SurfaceDrawTarget + ?Sized>(
    surface: &mut T,
    image: &ImageData,
    retained: Option<(&ImageData, [f32; 2])>,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    gamma: Option<&GammaRamp>,
    shear: f32,
    modulation: [u8; 3],
    color_alpha: u8,
    physical_texture_size: Option<i32>,
) {
    draw_scaled_native_image(
        surface,
        image,
        retained,
        f64::from(x),
        f64::from(y),
        f64::from(width),
        f64::from(height),
        gamma,
        f64::from(shear),
        modulation,
        color_alpha,
        physical_texture_size,
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_scaled_native_image<T: SurfaceDrawTarget + ?Sized>(
    surface: &mut T,
    image: &ImageData,
    retained: Option<(&ImageData, [f32; 2])>,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    gamma: Option<&GammaRamp>,
    shear: f64,
    modulation: [u8; 3],
    color_alpha: u8,
    physical_texture_size: Option<i32>,
) {
    let rect = GuiRect::new(x as f32, y as f32, width as f32, height as f32);
    if capture_scaled_native_image(
        surface,
        image,
        retained,
        &rect,
        gamma,
        shear as f32,
        modulation,
        color_alpha,
        physical_texture_size,
    ) {
        return;
    }
    if let Some(physical_texture_size) = physical_texture_size {
        crate::draw_image_bilinear_sheared_target_on_texture(
            surface,
            &rect,
            image,
            gamma,
            shear as f32,
            modulation,
            color_alpha,
            physical_texture_size,
        );
    } else {
        crate::draw_image_bilinear_sheared_target(
            surface,
            &rect,
            image,
            gamma,
            shear as f32,
            modulation,
            color_alpha,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn capture_scaled_native_image<T: SurfaceDrawTarget + ?Sized>(
    surface: &mut T,
    image: &ImageData,
    retained: Option<(&ImageData, [f32; 2])>,
    rect: &GuiRect,
    gamma: Option<&GammaRamp>,
    shear: f32,
    modulation: [u8; 3],
    color_alpha: u8,
    physical_texture_size: Option<i32>,
) -> bool {
    if !surface.is_gpu_command_capture_active()
        || image.width() == 0
        || image.height() == 0
        || rect.size.width <= 0.0
        || rect.size.height <= 0.0
        || ![
            rect.origin.x,
            rect.origin.y,
            rect.size.width,
            rect.size.height,
            shear,
        ]
        .into_iter()
        .all(f32::is_finite)
    {
        return false;
    }
    let expected_len = usize::try_from(image.width())
        .ok()
        .and_then(|width| {
            usize::try_from(image.height())
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .and_then(|pixels| pixels.checked_mul(4));
    if expected_len != Some(image.pixels().len()) {
        return false;
    }

    let renderer = crate::active_advanced_renderer_config().unwrap_or_default();
    let destination_offset = renderer.destination_offset();
    let texture_indent = renderer.texture_indent();
    let physical_texture_size = physical_texture_size
        .unwrap_or_else(|| crate::cpp_tex_size(image.width(), image.height()) as i32)
        .max(1) as f32;
    let texture_denominator = physical_texture_size + texture_indent * 2.0;
    if !texture_denominator.is_finite() || texture_denominator == 0.0 {
        return false;
    }
    let texture_scale = physical_texture_size / texture_denominator;
    let (texture, inset) = retained.unwrap_or((image, [0.0, 0.0]));
    if texture.width() == 0 || texture.height() == 0 {
        return false;
    }

    let tx = rect.origin.x + destination_offset;
    let ty = rect.origin.y + destination_offset;
    let center_y = rect.origin.y + rect.size.height / 2.0;
    let top_shift = shear * (ty - center_y);
    let bottom_shift = shear * (ty + rect.size.height - center_y);
    let left = tx;
    let right = tx + rect.size.width;
    let top = ty;
    let bottom = ty + rect.size.height;
    let positions = [
        [left + top_shift, top, 1.0],
        [right + top_shift, top, 1.0],
        [left + bottom_shift, bottom, 1.0],
        [right + bottom_shift, bottom, 1.0],
    ];

    let source_left = inset[0] + texture_indent;
    let source_top = inset[1] + texture_indent;
    let source_right = source_left + image.width() as f32 * texture_scale;
    let source_bottom = source_top + image.height() as f32 * texture_scale;
    let texture_width = texture.width() as f32;
    let texture_height = texture.height() as f32;
    let uv = [
        [source_left / texture_width, source_top / texture_height],
        [source_right / texture_width, source_top / texture_height],
        [source_left / texture_width, source_bottom / texture_height],
        [source_right / texture_width, source_bottom / texture_height],
    ];
    if !uv.iter().flatten().all(|value| value.is_finite()) {
        return false;
    }

    let transparency = if !renderer.shader && renderer.no_alpha_add {
        0
    } else {
        255 - color_alpha
    };
    let packed_modulation = [
        f32::from(modulation[0]) / 255.0,
        f32::from(modulation[1]) / 255.0,
        f32::from(modulation[2]) / 255.0,
        f32::from(transparency) / 255.0,
    ];
    let vertices = std::array::from_fn(|index| {
        GpuVertex::new(positions[index], uv[index], packed_modulation)
            .with_outer_modulation(GpuOuterModulation::Combine)
            .with_sample_tile(0.0, 0.0, physical_texture_size)
    });
    let resource = texture.gpu_texture_resource();
    if !resource.is_valid() {
        return false;
    }
    surface.capture_gpu_textured_command(
        resource,
        GpuCommand::Quad {
            texture: texture.gpu_texture_id(),
            owner_mask: None,
            vertices,
            clip: surface.clip(),
            blend: GpuBlend::Normal,
            base_mod2: false,
            owner_mod2: false,
            sampler: GpuSampler::Linear,
            gamma: gamma.is_some_and(|gamma| !gamma.is_passthrough()),
        },
    )
}

/// The five GUI fonts rasterized at the application's physical output scale.
pub struct NativeClonkFontSet {
    pub title: NativeClonkFont,
    pub caption: NativeClonkFont,
    pub text: NativeClonkFont,
    pub main_small: NativeClonkFont,
    pub mini: NativeClonkFont,
    book_title: NativeClonkFont,
    book_caption: NativeClonkFont,
    book_text: NativeClonkFont,
    book_small: NativeClonkFont,
    scale: f32,
}

impl NativeClonkFontSet {
    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// C4GUI::Button chooses the largest logical font fitting height - 2
    /// (`C4GuiButton.cpp:100-108`).
    pub fn button_font(&self, button_height: i32) -> &NativeClonkFont {
        let text_height = button_height - 2;
        if self.title.logical_line_height() <= text_height {
            &self.title
        } else if self.caption.logical_line_height() <= text_height {
            &self.caption
        } else {
            &self.text
        }
    }

    fn font_for_role(&self, role: ClonkFontRole) -> &NativeClonkFont {
        match role {
            ClonkFontRole::GuiTitle => &self.title,
            ClonkFontRole::GuiCaption => &self.caption,
            ClonkFontRole::GuiText => &self.text,
            ClonkFontRole::GuiMainSmall => &self.main_small,
            ClonkFontRole::GuiMini => &self.mini,
            ClonkFontRole::GuiTooltip | ClonkFontRole::BookText => &self.book_text,
            ClonkFontRole::BookTitle => &self.book_title,
            ClonkFontRole::BookCaption => &self.book_caption,
            ClonkFontRole::BookSmall => &self.book_small,
        }
    }

    /// Replay one ordered logical text batch with scale-native atlases.
    /// Each captured logical clipper installs the same independently rounded
    /// viewport and clip-relative glyph projection as CStdGL.
    pub fn draw_captured_text(
        &self,
        surface: &mut Surface,
        commands: &[CapturedClonkText],
        logical_target_size: (u32, u32),
    ) {
        self.draw_captured_text_to(surface, commands, logical_target_size);
    }

    pub fn draw_captured_text_to<T: SurfaceDrawTarget + ?Sized>(
        &self,
        surface: &mut T,
        commands: &[CapturedClonkText],
        logical_target_size: (u32, u32),
    ) {
        let saved_clip = surface.clip();
        for command in commands {
            let logical_clip = command.clip.unwrap_or_else(|| {
                clonk_graphics::Rect::new(0, 0, logical_target_size.0, logical_target_size.1)
            });
            let projection = ClipperProjection::new(
                self.scale,
                logical_target_size,
                surface.height(),
                logical_clip,
            );
            if command.clip.is_some() {
                surface.set_clip(projection.physical_clip());
            } else {
                surface.clear_clip();
            }
            let font = self.font_for_role(command.role);
            if command.images.is_empty() {
                font.draw_to_physical_surface_with_projection_impl(
                    surface,
                    command.x,
                    command.y,
                    &command.text,
                    command.color,
                    command.align,
                    command.markup,
                    NativeDrawProjection::clipper(projection),
                    command.gamma.as_ref(),
                    None,
                    command.zoom,
                );
            } else {
                let images = CapturedImageProvider(&command.images);
                font.draw_to_physical_surface_with_clipper_and_images_to(
                    surface,
                    command.x,
                    command.y,
                    &command.text,
                    command.color,
                    command.align,
                    command.markup,
                    projection,
                    command.gamma.as_ref(),
                    &images,
                );
            }
        }
        match saved_clip {
            Some(clip) => surface.set_clip(clip),
            None => surface.clear_clip(),
        }
    }
}

struct CapturedImageProvider<'a>(&'a [CapturedFontImage]);

impl FontImageProvider for CapturedImageProvider<'_> {
    fn font_image(&self, tag: &str) -> Option<FontImageRef<'_>> {
        self.0
            .iter()
            .find(|image| image.tag == tag)
            .map(|image| FontImageRef {
                width: image.width,
                height: image.height,
                rgba: &image.rgba,
            })
    }
}

impl ClonkFontSet {
    /// Picks the caption font for a button of the given height: the largest
    /// of Title/Caption/Text whose line height fits `height - 2`
    /// (Button::DrawElement, C4GuiButton.cpp:100-108).
    pub fn button_font(&self, button_height: i32) -> &ClonkFont {
        let text_height = button_height - 2;
        if self.title.line_height <= text_height {
            &self.title
        } else if self.caption.line_height <= text_height {
            &self.caption
        } else {
            &self.text
        }
    }
}

/// Expands a `&x` hotkey marker into the C++ markup highlight
/// `<c ffffff7f>x</c>` and returns the expanded label plus the (uppercased)
/// hotkey character (C4GUI::ExpandHotkeyMarkup, C4Gui.cpp:39-69).
pub fn expand_hotkey_markup(label: &str) -> (String, Option<char>) {
    label
        .find('&')
        .and_then(|pos| {
            let hotkey = label[pos + 1..].chars().next()?;
            let expanded = format!(
                "{}<c ffffff7f>{}</c>{}",
                &label[..pos],
                hotkey,
                &label[pos + 1 + hotkey.len_utf8()..]
            );
            Some((expanded, Some(hotkey.to_ascii_uppercase())))
        })
        .unwrap_or_else(|| (label.to_string(), None))
}

/// Windows-1252 specials in 0x80..=0x9F; the rest of 0x80..=0xFF maps to the
/// same Unicode scalar. Mirrors the C++ iconv conversion of the legacy
/// charset (StdFont.cpp:386-401, default charset per C4Config).
pub(crate) fn cp1252_to_char(byte: u8) -> Option<char> {
    match byte {
        0x80 => Some('\u{20AC}'),
        0x82 => Some('\u{201A}'),
        0x83 => Some('\u{0192}'),
        0x84 => Some('\u{201E}'),
        0x85 => Some('\u{2026}'),
        0x86 => Some('\u{2020}'),
        0x87 => Some('\u{2021}'),
        0x88 => Some('\u{02C6}'),
        0x89 => Some('\u{2030}'),
        0x8A => Some('\u{0160}'),
        0x8B => Some('\u{2039}'),
        0x8C => Some('\u{0152}'),
        0x8E => Some('\u{017D}'),
        0x91 => Some('\u{2018}'),
        0x92 => Some('\u{2019}'),
        0x93 => Some('\u{201C}'),
        0x94 => Some('\u{201D}'),
        0x95 => Some('\u{2022}'),
        0x96 => Some('\u{2013}'),
        0x97 => Some('\u{2014}'),
        0x98 => Some('\u{02DC}'),
        0x99 => Some('\u{2122}'),
        0x9A => Some('\u{0161}'),
        0x9B => Some('\u{203A}'),
        0x9C => Some('\u{0153}'),
        0x9E => Some('\u{017E}'),
        0x9F => Some('\u{0178}'),
        0x81 | 0x8D | 0x8F | 0x90 | 0x9D => None,
        b if b >= 0x80 => Some(b as char),
        b => Some(b as char),
    }
}

/// Characters materialized into the Rust cell map.
///
/// C++ pre-renders the active single-byte charset. In UTF-8 mode it instead
/// renders Unicode characters lazily through `GetUnicodeCharacterFacet`
/// (`StdFont.cpp:307-315,386-430`). Rust cells are independent allocations,
/// so eagerly materializing the face's bounded charmap is equivalent while
/// retaining the complete CP1252 map used by the default language.
/// Whole-pixel advance from FreeType's 26.6 fixed-point vector.
///
/// `FT_Pos` is `c_long`, which is 32-bit on Windows and 64-bit everywhere else,
/// so the narrowing is a real conversion on one target and a no-op on the
/// other. Written once here because a per-site cast reads as redundant on
/// whichever platform its author happened to build for.
#[allow(clippy::unnecessary_cast)]
pub(crate) fn advance_pixels(advance: Vector) -> i32 {
    (advance.x >> 6) as i32
}

fn classic_font_characters(face: &freetype::Face) -> BTreeSet<char> {
    let mut characters = (0x20_u16..=0xff)
        .filter_map(|byte| cp1252_to_char(byte as u8))
        .collect::<BTreeSet<_>>();
    characters.extend(face.chars().filter_map(|(charcode, _)| {
        u32::try_from(charcode)
            .ok()
            .and_then(char::from_u32)
            .filter(|character| *character >= ' ')
    }));
    characters
}

/// Convert the glyph currently loaded in `face` into a scale-one shadowed
/// CStdFont cell.
fn loaded_glyph_cell(
    face: &freetype::Face,
    cell_height: usize,
    ascent_px: i64,
    shadow: bool,
) -> Option<GlyphCell> {
    let slot = face.glyph();
    let bitmap = slot.bitmap();
    if bitmap.rows() > 0 && bitmap.pixel_mode().ok() != Some(freetype::bitmap::PixelMode::Gray) {
        return None; // StdFont.cpp:211-216
    }

    let (cov_w, cov_h) = (bitmap.width() as usize, bitmap.rows() as usize);
    let pitch = bitmap.pitch();
    let buffer = bitmap.buffer();
    // Repack honoring the pitch (rows may be padded).
    let cov: Vec<u8> = (0..cov_h)
        .flat_map(|y| {
            let start = (y as i32 * pitch) as usize;
            buffer[start..start + cov_w].iter().copied()
        })
        .collect();

    // width = max(advance, bearing+width) + shadow (StdFont.cpp:218).
    let advance_px = advance_pixels(slot.advance());
    let bearing = slot.bitmap_left().max(0);
    let shadow_size = i32::from(shadow);
    let cell_w = (advance_px.max(bearing + cov_w as i32) + shadow_size).max(1) as usize;
    let at_x = bearing as usize;
    let at_y = (ascent_px - i64::from(slot.bitmap_top())).max(0) as usize;
    let pixels = if shadow {
        compose_glyph_cell(&cov, cov_w, cov_h, cell_w, cell_height, at_x, at_y)
    } else {
        let mut pixels = vec![Color::transparent(); cell_w.saturating_mul(cell_height)];
        for y in 0..cov_h {
            for x in 0..cov_w {
                let (target_x, target_y) = (at_x + x, at_y + y);
                if target_x < cell_w && target_y < cell_height {
                    pixels[target_y * cell_w + target_x] =
                        Color::new(255, 255, 255, cov[y * cov_w + x]);
                }
            }
        }
        pixels
    };
    Some(GlyphCell {
        width: cell_w as i32,
        pixels,
    })
}

fn vector_font_texture_size(raster_height: u32) -> u32 {
    if raster_height > 40 {
        512
    } else {
        128
    }
}

fn surface_texture_size(width: u32, height: u32) -> u32 {
    width
        .min(height)
        .max(2)
        .checked_next_power_of_two()
        .unwrap_or(4096)
        .min(4096)
}

/// Rasterizes one ClonkFont at `px_height` from `face`.
fn build_font(
    face: &freetype::Face,
    px_height: u32,
    weight: u32,
    shadow: bool,
) -> Result<ClonkFont> {
    let boldness = i64::from(weight) - 400;
    let mut matrix = Matrix {
        xx: ((1_i64 << 16) + boldness * (1_i64 << 16) / 400) as _,
        xy: 0,
        yx: 0,
        yy: 1 << 16,
    };
    let mut delta = Vector { x: 0, y: 0 };
    face.set_transform(&mut matrix, &mut delta);
    face.set_pixel_sizes(px_height, px_height)
        .context("FT_Set_Pixel_Sizes failed")?;

    let raw = face.raw();
    let units_per_em = i32::from(raw.units_per_EM);
    let (ascender, descender) = (i32::from(raw.ascender), i32::from(raw.descender));
    let line_height = line_height_for(ascender, descender, units_per_em, px_height);
    // iGfxLineHgt includes one extra row only for a scale-one shadow.
    let cell_height = (line_height + i32::from(shadow)) as usize;
    // Baseline offset inside the cell (StdFont.cpp:221).
    let ascent_px = i64::from(px_height) * i64::from(ascender) / i64::from(units_per_em);

    let mut font = ClonkFont::new(line_height);
    font.set_texture_size(vector_font_texture_size(px_height));
    font.cell_height = cell_height as i32;
    font.h_space = if shadow { -1 } else { 0 };
    for ch in classic_font_characters(face) {
        if face
            .load_char(ch as usize, LoadFlag::RENDER | LoadFlag::NO_HINTING)
            .is_err()
        {
            // C++ skips characters the font cannot render (StdFont.cpp:203-208).
            continue;
        }
        if let Some(cell) = loaded_glyph_cell(face, cell_height, ascent_px, shadow) {
            font.add_glyph(ch, cell);
        }
    }
    // FT_Load_Char maps an absent UTF-8 scalar to glyph index zero before
    // loading it. Reuse that one `.notdef` cell instead of retaining a live
    // mutable FreeType face solely for C++'s on-demand cache behavior.
    let missing_glyph = face
        .load_glyph(0, LoadFlag::RENDER | LoadFlag::NO_HINTING)
        .ok()
        .and_then(|_| loaded_glyph_cell(face, cell_height, ascent_px, shadow));
    if let Some(cell) = missing_glyph {
        font.set_missing_glyph(cell);
    }
    Ok(font)
}

/// CStdFont::AddRenderedChar's glyph-cell compositor for an application
/// scale greater than one (`StdFont.cpp:184,218-258`). Unlike the scale-1
/// helper in clonk-graphics, the shadow sample is offset by `round(scale)`
/// physical pixels.
#[allow(clippy::too_many_arguments)]
fn compose_scaled_glyph_cell(
    cov: &[u8],
    cov_w: usize,
    cov_h: usize,
    cell_w: usize,
    cell_h: usize,
    at_x: usize,
    at_y: usize,
    shadow_size: usize,
) -> Vec<Color> {
    let Some(len) = cell_w.checked_mul(cell_h) else {
        return Vec::new();
    };
    let mut cell = vec![Color::transparent(); len];
    let coverage = |x: usize, y: usize| -> u32 {
        (x < cov_w && y < cov_h)
            .then(|| {
                y.checked_mul(cov_w)
                    .and_then(|row| row.checked_add(x))
                    .and_then(|index| cov.get(index))
            })
            .flatten()
            .map_or(0, |&value| u32::from(value))
    };
    for y in 0..cov_h.saturating_add(shadow_size) {
        for x in 0..cov_w.saturating_add(shadow_size) {
            let alpha_inverted = if x < cov_w && y < cov_h {
                255 - coverage(x, y)
            } else {
                255
            };
            let (base_grey, shadow_alpha_inverted) = if shadow_size > 0
                && x >= shadow_size
                && y >= shadow_size
            {
                let lower = shadow_size - 1;
                let upper = shadow_size + 1;
                let shadow = [
                    (x < cov_w && y < cov_h).then(|| coverage(x - lower, y - lower)),
                    (x > shadow_size && y < cov_h).then(|| coverage(x - upper, y - lower)),
                    (x > lower && y < cov_h).then(|| coverage(x - shadow_size, y - lower)),
                    (x < cov_w && y > shadow_size).then(|| coverage(x - lower, y - upper)),
                    (x > shadow_size && y > shadow_size).then(|| coverage(x - upper, y - upper)),
                    (x > lower && y > shadow_size).then(|| coverage(x - shadow_size, y - upper)),
                    (x < cov_w && y > lower).then(|| coverage(x - lower, y - shadow_size)),
                    (x > shadow_size && y > lower).then(|| coverage(x - upper, y - shadow_size)),
                    (x > lower && y > lower)
                        .then(|| coverage(x - shadow_size, y - shadow_size) * 8),
                ]
                .into_iter()
                .flatten()
                .sum::<u32>();
                ((255 - alpha_inverted) / 2, 255 - shadow / 16)
            } else {
                (0, 255)
            };
            let (r, g, b, out_alpha_inverted) = if shadow_alpha_inverted == 255 {
                (255, 255, 255, alpha_inverted)
            } else {
                let source_alpha = 255 - alpha_inverted;
                let mix = |destination: u32| {
                    ((255 * source_alpha + destination * alpha_inverted) >> 8).min(255)
                };
                (
                    mix(base_grey),
                    mix(base_grey),
                    mix(base_grey),
                    shadow_alpha_inverted.saturating_sub(source_alpha),
                )
            };
            if let (Some(target_x), Some(target_y)) = (at_x.checked_add(x), at_y.checked_add(y)) {
                if target_x < cell_w && target_y < cell_h {
                    cell[target_y * cell_w + target_x] =
                        Color::new(r as u8, g as u8, b as u8, (255 - out_alpha_inverted) as u8);
                }
            }
        }
    }
    cell
}

/// Convert the glyph currently loaded in `face` into a native-resolution
/// shadowed CStdFont cell.
fn loaded_native_glyph_cell(
    face: &freetype::Face,
    cell_height: usize,
    ascent_px: i64,
    shadow_size: u32,
) -> Option<GlyphCell> {
    let slot = face.glyph();
    let bitmap = slot.bitmap();
    if bitmap.rows() > 0 && bitmap.pixel_mode().ok() != Some(freetype::bitmap::PixelMode::Gray) {
        return None;
    }
    let (cov_w, cov_h) = (bitmap.width() as usize, bitmap.rows() as usize);
    let pitch = bitmap.pitch();
    let buffer = bitmap.buffer();
    let cov: Vec<u8> = (0..cov_h)
        .flat_map(|y| {
            let start = (y as i32 * pitch) as usize;
            buffer[start..start + cov_w].iter().copied()
        })
        .collect();
    let advance_px = advance_pixels(slot.advance());
    let bearing = slot.bitmap_left().max(0);
    let cell_width = (advance_px.max(bearing + cov_w as i32) + shadow_size as i32).max(1) as usize;
    let at_x = bearing as usize;
    let at_y = (ascent_px - i64::from(slot.bitmap_top())).max(0) as usize;
    let pixels = compose_scaled_glyph_cell(
        &cov,
        cov_w,
        cov_h,
        cell_width,
        cell_height,
        at_x,
        at_y,
        shadow_size as usize,
    );
    Some(GlyphCell {
        width: cell_width as i32,
        pixels,
    })
}

fn build_native_font(
    face: &freetype::Face,
    logical_height: u32,
    application_scale: f32,
    shadow: bool,
    snap_to_pixels: bool,
) -> Result<NativeClonkFont> {
    let scaled_height = logical_height as f32 * application_scale;
    anyhow::ensure!(
        scaled_height.is_finite() && scaled_height <= i32::MAX as f32,
        "scaled font height overflow"
    );
    // C++ truncates (`StdFont.cpp:325`). Whole-pixel snapping instead asks
    // FreeType for the nearest physical height so the atlas can be blitted
    // 1:1 at fractional application scales.
    let raster_height = if snap_to_pixels {
        scaled_height.round() as u32
    } else {
        scaled_height as u32
    };
    anyhow::ensure!(raster_height > 0, "scaled font height truncates to zero");
    let effective_scale = raster_height as f32 / logical_height as f32;
    let shadow_size = if shadow {
        application_scale.round() as u32
    } else {
        0
    };
    face.set_pixel_sizes(raster_height, raster_height)
        .context("FT_Set_Pixel_Sizes failed")?;

    let raw = face.raw();
    let units_per_em = i32::from(raw.units_per_EM);
    let (ascender, descender) = (i32::from(raw.ascender), i32::from(raw.descender));
    let line_height = line_height_for(ascender, descender, units_per_em, raster_height);
    // C++ deliberately adds one atlas row for a shadowed font even when
    // shadowSize is 3; shadowless book/tooltip fonts add none.
    let cell_height = (line_height + i32::from(shadow)) as usize;
    let ascent_px = i64::from(raster_height) * i64::from(ascender) / i64::from(units_per_em);

    let mut font = ClonkFont::new(line_height);
    font.set_texture_size(vector_font_texture_size(raster_height));
    // iHSpace remains -1 GUI unit in C++. The existing physical-run blitter
    // accepts integer pen positions, so retain exact integer-scale behavior
    // and use the nearest physical spacing at fractional scales. Logical
    // measurement above uses the exact unscaled -1 independently.
    font.cell_height = cell_height as i32;
    font.h_space = if shadow {
        -(application_scale.round() as i32)
    } else {
        0
    };
    for ch in classic_font_characters(face) {
        if face
            .load_char(ch as usize, LoadFlag::RENDER | LoadFlag::NO_HINTING)
            .is_err()
        {
            continue;
        }
        if let Some(cell) = loaded_native_glyph_cell(face, cell_height, ascent_px, shadow_size) {
            font.add_glyph(ch, cell);
        }
    }
    let missing_glyph = face
        .load_glyph(0, LoadFlag::RENDER | LoadFlag::NO_HINTING)
        .ok()
        .and_then(|_| loaded_native_glyph_cell(face, cell_height, ascent_px, shadow_size));
    if let Some(cell) = missing_glyph {
        font.set_missing_glyph(cell);
    }
    Ok(NativeClonkFont {
        raster: font,
        application_scale,
        effective_scale,
        logical_height,
        raster_height,
        logical_h_space: if shadow { -1 } else { 0 },
        snap_to_pixels,
        retained_glyph_images: Mutex::new(HashMap::new()),
    })
}

/// Builds one vector CStdFont with the requested FreeType height, weight and
/// shadow mode (`C4FontLoader::InitFont`, C4Fonts.cpp:158-173).
pub fn build_vector_font(
    ttf_bytes: &[u8],
    px_height: u32,
    weight: u32,
    shadow: bool,
) -> Result<ClonkFont> {
    build_vector_font_face(ttf_bytes, 0, px_height, weight, shadow)
}

/// Builds one vector CStdFont from a selected face in a standalone font or
/// TrueType/OpenType collection.
pub fn build_vector_font_face(
    ttf_bytes: &[u8],
    face_index: u32,
    px_height: u32,
    weight: u32,
    shadow: bool,
) -> Result<ClonkFont> {
    let library = Library::init().context("FreeType init failed")?;
    let face_index = isize::try_from(face_index).context("font face index exceeds FreeType")?;
    let face = library
        .new_memory_face(ttf_bytes.to_vec(), face_index)
        .context("failed to load font face")?;
    build_font(&face, px_height, weight, shadow)
}

/// Builds the five GUI fonts from a TTF at a configurable RX base size.
pub fn build_font_set_at_size(ttf_bytes: &[u8], base_size: u32) -> Result<ClonkFontSet> {
    let library = Library::init().context("FreeType init failed")?;
    let face = library
        .new_memory_face(ttf_bytes.to_vec(), 0)
        .context("failed to load font face")?;
    Ok(ClonkFontSet {
        title: build_font(&face, base_size.saturating_mul(22) / 14, 400, true)?
            .with_role(ClonkFontRole::GuiTitle),
        caption: build_font(&face, base_size.saturating_mul(16) / 14, 400, true)?
            .with_role(ClonkFontRole::GuiCaption),
        text: build_font(&face, base_size, 400, true)?.with_role(ClonkFontRole::GuiText),
        main_small: build_font(&face, base_size.saturating_mul(13) / 14, 400, true)?
            .with_role(ClonkFontRole::GuiMainSmall),
        mini: build_font(&face, base_size.saturating_mul(12) / 14, 400, true)?
            .with_role(ClonkFontRole::GuiMini),
    })
}

/// Builds the default Endeavour-14 GUI set.
pub fn build_font_set(ttf_bytes: &[u8]) -> Result<ClonkFontSet> {
    build_font_set_at_size(ttf_bytes, 14)
}

/// Builds `C4GraphicsResource::FontTooltip`: the Main-14 RX face initialized
/// independently with `fDoShadow = false` (`C4GraphicsResource.cpp:165`).
/// It is deliberately not borrowed from the startup book-font bundle because
/// the process-global GUI resource owns a separate `CStdFont` instance.
pub fn build_tooltip_font(ttf_bytes: &[u8]) -> Result<ClonkFont> {
    build_vector_font(ttf_bytes, 14, 400, false)
        .map(|font| font.with_role(ClonkFontRole::GuiTooltip))
}

/// Builds a prerendered byte-slot font from a decoded PNG/BMP surface.
pub fn build_prerendered_font(
    width: u32,
    height: u32,
    rgba: &[u8],
    indent: i32,
) -> Result<ClonkFont> {
    anyhow::ensure!(width > 0 && height > 0, "bitmap font surface is empty");
    let expected_len = usize::try_from(width)
        .ok()
        .and_then(|width| {
            usize::try_from(height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .and_then(|pixels| pixels.checked_mul(4))
        .context("bitmap font dimensions overflow")?;
    anyhow::ensure!(
        rgba.len() == expected_len,
        "bitmap font RGBA plane has the wrong size"
    );
    let pixel = |x: u32, y: u32| {
        let index = (y as usize * width as usize + x as usize) * 4;
        [
            rgba[index],
            rgba[index + 1],
            rgba[index + 2],
            rgba[index + 3],
        ]
    };
    let delimiter = |color: [u8; 4]| {
        // C++ compares `ClrDw2W` values. Its surface alpha is inverted, so
        // an opaque decoded RGBA pixel contributes a zero alpha nibble and
        // transparent lookalikes do not count as delimiter colors.
        let rgba4 = (color[0] >> 4, color[1] >> 4, color[2] >> 4, color[3] >> 4);
        match rgba4 {
            (0xf, 0x0, 0x0, 0xf) | (0xf, 0xf, 0x0, 0xf) | (0xf, 0x0, 0xf, 0xf) => Some(false),
            (0x0, 0xf, 0x0, 0xf) => Some(true),
            _ => None,
        }
    };
    let mut gfx_line_height = 1;
    while gfx_line_height < height && delimiter(pixel(0, gfx_line_height)).is_none() {
        gfx_line_height += 1;
    }
    let mut font = ClonkFont::new(gfx_line_height as i32 - indent);
    font.set_texture_size(surface_texture_size(width, height));
    font.cell_height = gfx_line_height as i32;
    font.h_space = -indent;

    let (mut x, mut y) = (0_u32, 0_u32);
    for byte in b' '..=u8::MAX {
        let start = x;
        let mut line_break = false;
        while x < width {
            if let Some(is_line_break) = delimiter(pixel(x, y)) {
                line_break = is_line_break;
                break;
            }
            x += 1;
        }
        let glyph_width = x - start;
        let mut pixels = Vec::with_capacity(glyph_width as usize * gfx_line_height as usize);
        for cell_y in 0..gfx_line_height {
            for cell_x in 0..glyph_width {
                let [r, g, b, a] = pixel(start + cell_x, y + cell_y);
                pixels.push(Color::new(r, g, b, a));
            }
        }
        if let Some(character) = cp1252_to_char(byte) {
            font.add_glyph(
                character,
                GlyphCell {
                    width: glyph_width as i32,
                    pixels,
                },
            );
        }
        x += 1;
        if x >= width || line_break {
            y = y.saturating_add(gfx_line_height).saturating_add(1);
            x = 0;
            if y.saturating_add(gfx_line_height) > height {
                break;
            }
        }
    }
    Ok(font)
}

/// The per-role FreeType heights `C4FontLoader::InitFont` derives from
/// `Config.General.RXFontSize` (`C4Fonts.cpp:279-287`). Only `Main` uses the
/// configured size unchanged; the remaining roles are integer ratios of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeFontSizes {
    pub title: u32,
    pub caption: u32,
    pub text: u32,
    pub main_small: u32,
    pub mini: u32,
}

impl NativeFontSizes {
    /// `RXFontSize = 14`, the shipped default recipe.
    pub const CLASSIC: Self = Self {
        title: 22,
        caption: 16,
        text: 14,
        main_small: 13,
        mini: 12,
    };

    /// `C4Fonts.cpp:281-286` — each role is `iSize * numerator / 14` in C++
    /// integer arithmetic.
    pub fn for_base_size(base_size: u32) -> Self {
        Self {
            title: base_size * 22 / 14,
            caption: base_size * 16 / 14,
            text: base_size,
            main_small: base_size * 13 / 14,
            mini: base_size * 12 / 14,
        }
    }
}

/// Everything about a scale-native font set that is independent of the
/// application scale: which face of the file to use, the per-role heights, and
/// the opt-in whole-pixel snapping divergence.
#[derive(Clone, Copy, Debug)]
pub struct NativeFontRecipe {
    pub sizes: NativeFontSizes,
    /// Face within a TrueType/OpenType collection.
    pub face_index: u32,
    /// `Graphics.SnapTextToPixels`: rasterize at `round(logical * scale)` and
    /// blit glyphs at whole physical pixels. C++ truncates the scaled height
    /// (`StdFont.cpp:325`) and resamples the atlas through the application
    /// transform, so this is a deliberate presentation divergence and stays
    /// off unless the player asks for it.
    pub snap_to_pixels: bool,
}

impl NativeFontRecipe {
    pub fn new(sizes: NativeFontSizes) -> Self {
        Self {
            sizes,
            face_index: 0,
            snap_to_pixels: false,
        }
    }

    pub fn with_face_index(self, face_index: u32) -> Self {
        Self { face_index, ..self }
    }

    pub fn with_snap_to_pixels(self, snap_to_pixels: bool) -> Self {
        Self {
            snap_to_pixels,
            ..self
        }
    }
}

/// Build the five GUI fonts the way C++ does at `Graphics.Scale`: native
/// physical raster data with logical GUI metrics. C++ truncates each scaled
/// FreeType height independently, then stores that font's effective scale as
/// `raster_height / logical_height` (`C4Fonts.cpp:158-173`;
/// `StdFont.cpp:319-352,436-439,571-638,938`).
pub fn build_native_font_set(
    ttf_bytes: &[u8],
    scale: impl Into<f64>,
) -> Result<NativeClonkFontSet> {
    build_native_font_set_face(ttf_bytes, 0, scale)
}

/// Builds the scale-native GUI font set from a selected face in a standalone
/// font or TrueType/OpenType collection.
pub fn build_native_font_set_face(
    ttf_bytes: &[u8],
    face_index: u32,
    scale: impl Into<f64>,
) -> Result<NativeClonkFontSet> {
    build_native_font_set_recipe(
        ttf_bytes,
        NativeFontRecipe::new(NativeFontSizes::CLASSIC).with_face_index(face_index),
        scale,
    )
}

/// Builds the scale-native GUI font set for an arbitrary `RXFontSize` recipe.
pub fn build_native_font_set_recipe(
    ttf_bytes: &[u8],
    recipe: NativeFontRecipe,
    scale: impl Into<f64>,
) -> Result<NativeClonkFontSet> {
    let scale = scale.into();
    anyhow::ensure!(
        scale.is_finite() && scale > 0.0,
        "font scale must be finite and positive"
    );
    let scale = scale as f32;
    anyhow::ensure!(scale.is_finite(), "font scale exceeds f32 geometry");
    let sizes = recipe.sizes;
    anyhow::ensure!(
        [
            sizes.title,
            sizes.caption,
            sizes.text,
            sizes.main_small,
            sizes.mini
        ]
        .iter()
        .all(|size| *size > 0),
        "native font recipe requests a zero FreeType height"
    );
    let library = Library::init().context("FreeType init failed")?;
    let face_index =
        isize::try_from(recipe.face_index).context("font face index exceeds FreeType")?;
    let face = library
        .new_memory_face(ttf_bytes.to_vec(), face_index)
        .context("failed to load font face")?;
    let snap = recipe.snap_to_pixels;
    Ok(NativeClonkFontSet {
        title: build_native_font(&face, sizes.title, scale, true, snap)?,
        caption: build_native_font(&face, sizes.caption, scale, true, snap)?,
        text: build_native_font(&face, sizes.text, scale, true, snap)?,
        main_small: build_native_font(&face, sizes.main_small, scale, true, snap)?,
        mini: build_native_font(&face, sizes.mini, scale, true, snap)?,
        book_title: build_native_font(&face, sizes.title, scale, false, snap)?,
        book_caption: build_native_font(&face, sizes.caption, scale, false, snap)?,
        book_text: build_native_font(&face, sizes.text, scale, false, snap)?,
        book_small: build_native_font(&face, sizes.main_small, scale, false, snap)?,
        scale,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn endeavour_bytes() -> Vec<u8> {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../planet/System.c4g/Endeavour.ttf");
        std::fs::read(path).expect("read Endeavour.ttf")
    }

    fn duplicate_font_collection(font: &[u8]) -> Vec<u8> {
        const TTC_HEADER_SIZE: u32 = 20;
        let table_count = usize::from(u16::from_be_bytes([font[4], font[5]]));
        let mut embedded = font.to_vec();
        for table in 0..table_count {
            let offset_position = 12 + table * 16 + 8;
            let offset = u32::from_be_bytes(
                embedded[offset_position..offset_position + 4]
                    .try_into()
                    .expect("font table offset"),
            );
            embedded[offset_position..offset_position + 4]
                .copy_from_slice(&offset.saturating_add(TTC_HEADER_SIZE).to_be_bytes());
        }

        let mut collection = Vec::with_capacity(TTC_HEADER_SIZE as usize + embedded.len());
        collection.extend_from_slice(b"ttcf");
        collection.extend_from_slice(&0x0001_0000_u32.to_be_bytes());
        collection.extend_from_slice(&2_u32.to_be_bytes());
        collection.extend_from_slice(&TTC_HEADER_SIZE.to_be_bytes());
        collection.extend_from_slice(&TTC_HEADER_SIZE.to_be_bytes());
        collection.extend_from_slice(&embedded);
        collection
    }

    #[test]
    fn nonzero_collection_face_builds_logical_and_scale_native_fonts() {
        let collection = duplicate_font_collection(&endeavour_bytes());
        let logical = build_vector_font_face(&collection, 1, 14, 400, true)
            .expect("build second collection face");
        assert!(logical.glyph('A').is_some());

        let native = build_native_font_set_face(&collection, 1, 2.0)
            .expect("build second collection face at native scale");
        assert_eq!(native.scale(), 2.0);
        assert!(native.text.glyph('A').is_some());
        assert!(build_vector_font_face(&collection, 2, 14, 400, true).is_err());
    }

    #[test]
    fn scale_three_fonts_use_native_raster_shadow_and_logical_metrics() {
        // C4FontLoader passes Application.GetScale() to CStdFont::Init
        // (C4Fonts.cpp:158-173). CStdFont rasterizes at height*scale, uses
        // round(scale) for the shadow, and divides metrics back to GUI units
        // (StdFont.cpp:184,319-352,571-638,938).
        let fonts = build_native_font_set(&endeavour_bytes(), 3).expect("build 3x fonts");

        assert_eq!(fonts.scale(), 3.0);
        assert_eq!(fonts.title.raster_line_height(), 103);
        assert_eq!(fonts.title.raster_cell_height(), 104);
        assert_eq!(fonts.title.logical_line_height(), 34);
        assert_eq!(fonts.caption.logical_line_height(), 25);
        assert_eq!(fonts.text.logical_line_height(), 22);
        assert_eq!(fonts.main_small.logical_line_height(), 20);
        assert_eq!(fonts.mini.logical_line_height(), 18);
        assert!(
            fonts.text.glyph('\u{0100}').is_some(),
            "native FontRegular must include Endeavour's Unicode charmap"
        );
        assert!(
            fonts.text.measure("\u{1f642}", false).0 > 0,
            "native FontRegular must measure an unmapped scalar as glyph zero"
        );

        let base = build_font_set(&endeavour_bytes()).expect("build 1x fonts");
        assert!(
            fonts.title.glyph('A').expect("native A").width
                > base.title.glyph('A').expect("base A").width * 2,
            "the 3x font must contain a newly rasterized glyph, not a scaled 1x cell"
        );

        let cell = compose_scaled_glyph_cell(&[255], 1, 1, 5, 5, 0, 0, 3);
        assert_eq!(cell[6].a, 0, "3x shadow is not a 1px shadow");
        assert_eq!(
            cell[3 * 5 + 3],
            clonk_graphics::Color::new(0, 0, 0, 127),
            "round(scale)=3 places the C++ shadow three physical pixels away"
        );
    }

    #[test]
    fn configured_font_size_recipe_rasterizes_every_native_role() {
        // C4FontLoader::InitFont derives one FreeType height per role from
        // Config.General.RXFontSize (C4Fonts.cpp:279-287); the 22/16/14/13/12
        // recipe is only the RXFontSize=14 case. CStdFont::Init then scales
        // each of those heights independently (StdFont.cpp:319-352).
        assert_eq!(
            NativeFontSizes::for_base_size(14),
            NativeFontSizes::CLASSIC,
            "the classic recipe is the size-14 derivation"
        );
        let sizes = NativeFontSizes::for_base_size(16);
        assert_eq!(
            sizes,
            NativeFontSizes {
                title: 25,
                caption: 18,
                text: 16,
                main_small: 14,
                mini: 13,
            }
        );

        let fonts =
            build_native_font_set_recipe(&endeavour_bytes(), NativeFontRecipe::new(sizes), 2.0_f32)
                .expect("build the size-16 recipe at 2x");
        assert_eq!(fonts.scale(), 2.0);
        assert_eq!(fonts.title.logical_height(), 25);
        assert_eq!(fonts.title.raster_height(), 50);
        assert_eq!(fonts.caption.logical_height(), 18);
        assert_eq!(fonts.text.logical_height(), 16);
        assert_eq!(fonts.text.raster_height(), 32);
        assert_eq!(fonts.main_small.logical_height(), 14);
        assert_eq!(fonts.mini.logical_height(), 13);
        assert!(fonts.text.glyph('A').is_some());
    }

    #[test]
    fn fractional_native_fonts_truncate_each_raster_height_and_keep_effective_metrics() {
        let fonts =
            build_native_font_set(&endeavour_bytes(), 1.5_f32).expect("build scale-1.5 fonts");

        assert_eq!(fonts.scale(), 1.5);
        assert_eq!(fonts.text.raster_height(), 21);
        assert_eq!(fonts.text.effective_scale(), 1.5);
        assert_eq!(fonts.main_small.logical_height(), 13);
        assert_eq!(fonts.main_small.raster_height(), 19);
        assert_eq!(fonts.main_small.application_scale(), 1.5);
        assert_eq!(fonts.main_small.effective_scale(), 19.0 / 13.0);
        assert_ne!(
            fonts.main_small.effective_scale(),
            fonts.main_small.application_scale(),
            "13 * 1.5 truncates before CStdFont stores its effective scale"
        );

        let main_small = &fonts.main_small;
        let glyph_width = main_small.glyph('A').expect("native A").width as f32;
        assert_eq!(
            main_small.measure("A", false),
            (
                (glyph_width / main_small.effective_scale()) as i32,
                (main_small.raster_line_height() as f32 / main_small.effective_scale()).ceil()
                    as i32,
            )
        );
        assert_eq!(main_small.logical_line_height(), 19);
        assert_eq!(main_small.measure("A\nA", false).1, 40);
    }

    #[test]
    fn snapped_native_fonts_round_each_raster_height_and_blit_whole_pixels() {
        // Opt-in `Graphics.SnapTextToPixels`. C++ truncates the scaled
        // FreeType height (StdFont.cpp:325) and then rescales every facet by
        // `scale / effective_scale` (StdFont.cpp:685,701,841), so a
        // fractional Graphics.Scale resamples all text. Snapping asks
        // FreeType for the nearest physical height instead and blits the
        // atlas at whole physical pixels: a deliberate divergence.
        let snapped = build_native_font_set_recipe(
            &endeavour_bytes(),
            NativeFontRecipe::new(NativeFontSizes::CLASSIC).with_snap_to_pixels(true),
            1.5_f32,
        )
        .expect("snapped 1.5x fonts");
        assert_eq!(snapped.main_small.raster_height(), 20, "round(13 * 1.5)");
        assert_eq!(snapped.text.raster_height(), 21);
        let truncated =
            build_native_font_set(&endeavour_bytes(), 1.5_f32).expect("C++-exact 1.5x fonts");
        assert_eq!(truncated.main_small.raster_height(), 19, "trunc(13 * 1.5)");

        // A striped atlas cell: a 1:1 blit reproduces the stripes exactly,
        // while any resample mixes neighbouring columns into partial alpha.
        let striped_font = |snap_to_pixels: bool| {
            let raster_height = if snap_to_pixels { 20 } else { 19 };
            let mut raster = ClonkFont::new(raster_height);
            raster.cell_height = raster_height;
            raster.h_space = -2;
            raster.add_glyph(
                'A',
                GlyphCell {
                    width: raster_height,
                    pixels: (0..raster_height * raster_height)
                        .map(|index| {
                            if index % raster_height % 2 == 0 {
                                Color::opaque(255, 255, 255)
                            } else {
                                Color::new(255, 255, 255, 0)
                            }
                        })
                        .collect(),
                },
            );
            NativeClonkFont {
                raster,
                application_scale: 1.5,
                effective_scale: raster_height as f32 / 13.0,
                logical_height: 13,
                raster_height: raster_height as u32,
                logical_h_space: -1,
                snap_to_pixels,
                retained_glyph_images: Mutex::new(HashMap::new()),
            }
        };
        let covered = |font: &NativeClonkFont| {
            let mut surface = Surface::new(48, 48, clonk_graphics::PixelFormat::Rgba8888);
            font.draw_to_physical_surface(
                &mut surface,
                1,
                1,
                "A",
                [255, 255, 255, 255],
                TextAlign::Left,
                false,
                None,
            );
            (0..surface.height())
                .flat_map(|y| (0..surface.width()).map(move |x| (x, y)))
                .filter_map(|(x, y)| {
                    surface
                        .get_pixel(x, y)
                        .filter(|pixel| pixel.a != 0)
                        .map(|pixel| (x, y, pixel.a))
                })
                .collect::<Vec<_>>()
        };

        let resampled = covered(&striped_font(false));
        assert!(
            resampled.iter().any(|(_, _, alpha)| *alpha != 255),
            "the C++-exact 19px atlas is resampled to 19.5 physical pixels"
        );

        let snapped = covered(&striped_font(true));
        assert!(
            snapped.iter().all(|(_, _, alpha)| *alpha == 255),
            "a snapped atlas blits one texel per physical pixel"
        );
        assert_eq!(
            snapped.len(),
            10 * 20,
            "every second column of the 20px cell stays fully transparent"
        );
        let left = snapped.iter().map(|(x, _, _)| *x).min();
        assert_eq!(left, Some(2), "the line origin snaps to round(1 * 1.5)");
    }

    #[test]
    fn fractional_main_small_resamples_glyph_and_image_quads_to_application_scale() {
        struct SolidImage([u8; 4]);

        impl FontImageProvider for SolidImage {
            fn font_image(&self, tag: &str) -> Option<FontImageRef<'_>> {
                (tag == "icon").then_some(FontImageRef {
                    width: 1,
                    height: 1,
                    rgba: &self.0,
                })
            }
        }

        // MainSmall at 1.5x truncates its 13px FreeType request to a 19px
        // atlas. C++ draws each 19px facet at 19 / (19/13) logical pixels,
        // then applies the 1.5x application transform: 19.5 physical pixels.
        let mut raster = ClonkFont::new(19);
        raster.cell_height = 19;
        raster.h_space = -2;
        raster.add_glyph(
            'A',
            GlyphCell {
                width: 19,
                pixels: vec![Color::opaque(255, 255, 255); 19 * 19],
            },
        );
        let font = NativeClonkFont {
            raster,
            application_scale: 1.5,
            effective_scale: 19.0 / 13.0,
            logical_height: 13,
            raster_height: 19,
            logical_h_space: -1,
            snap_to_pixels: false,
            retained_glyph_images: Mutex::new(HashMap::new()),
        };
        let mut surface = Surface::new(64, 40, clonk_graphics::PixelFormat::Rgba8888);
        font.draw_to_physical_surface_with_offset_and_images(
            &mut surface,
            2,
            2,
            "A{{icon}}A",
            [255, 255, 255, 255],
            TextAlign::Left,
            true,
            (0, 0),
            None,
            &SolidImage([0, 255, 0, 255]),
        );

        let occupied_x = (0..surface.width())
            .filter(|x| {
                (0..surface.height())
                    .any(|y| surface.get_pixel(*x, y).is_some_and(|pixel| pixel.a != 0))
            })
            .collect::<Vec<_>>();
        let quad_scale = font.application_scale / font.effective_scale;
        let expected_right = 3.0 + 3.0 * 19.0 * quad_scale - 2.0 * 1.5;
        let expected_end = (expected_right - 0.5).ceil() as u32;
        assert_eq!(occupied_x.first(), Some(&3));
        assert_eq!(occupied_x.last(), Some(&(expected_end - 1)));
        assert_eq!(occupied_x.len(), (expected_end - 3) as usize);
        assert!(
            occupied_x.len() > 53,
            "the old raw-atlas blit covered only 53 physical columns"
        );
        assert!(surface
            .pixels()
            .chunks_exact(4)
            .any(|pixel| pixel[1] > pixel[0] && pixel[1] > pixel[2]));
    }

    #[test]
    fn fractional_native_italics_shear_a_mixed_text_and_image_row_together() {
        // The neighbouring subcase draws rows that are *either* glyphs or an
        // inline image. A row carrying both is the case where a shear applied
        // to one and not the other — or applied from a different origin —
        // would show, and it is the shape Info menus actually use: prose with
        // an icon in the middle of it (C4Menu.cpp:613-849; StdFont.cpp:817-929).
        struct SolidImage(Vec<u8>);

        impl FontImageProvider for SolidImage {
            fn font_image(&self, tag: &str) -> Option<FontImageRef<'_>> {
                (tag == "icon").then_some(FontImageRef {
                    width: 19,
                    height: 19,
                    rgba: &self.0,
                })
            }
        }

        fn row_span(surface: &Surface, y: u32) -> Option<(u32, u32)> {
            let changed = (0..surface.width())
                .filter(|x| surface.get_pixel(*x, y).is_some_and(|pixel| pixel.a != 0))
                .collect::<Vec<_>>();
            changed.first().copied().zip(changed.last().copied())
        }

        let mut raster = ClonkFont::new(19);
        raster.cell_height = 19;
        raster.h_space = -2;
        raster.add_glyph(
            'A',
            GlyphCell {
                width: 19,
                pixels: vec![Color::opaque(255, 255, 255); 19 * 19],
            },
        );
        let font = NativeClonkFont {
            raster,
            application_scale: 1.5,
            effective_scale: 19.0 / 13.0,
            logical_height: 13,
            raster_height: 19,
            logical_h_space: -1,
            snap_to_pixels: false,
            retained_glyph_images: Mutex::new(HashMap::new()),
        };
        let images = SolidImage(vec![255; 19 * 19 * 4]);
        let projection =
            ClipperProjection::new(1.5, (127, 39), 59, clonk_graphics::Rect::new(0, 0, 127, 39));
        let (_, scale_y) = projection.scale();

        // Italic must not move the row's metrics, mixed content included.
        // The tag is left open, as the neighbouring subcase does, so this
        // isolates italic metrics from the native trailing-close-tag h-space
        // quirk rather than re-measuring that.
        assert_eq!(
            font.measure_with_images("A{{icon}}A", true, &images),
            font.measure_with_images("<i>A{{icon}}A", true, &images),
            "shear is a draw-time transform, not a metric"
        );

        let mut plain = Surface::new(191, 59, clonk_graphics::PixelFormat::Rgba8888);
        let mut italic = Surface::new(191, 59, clonk_graphics::PixelFormat::Rgba8888);
        for (surface, text) in [(&mut plain, "A{{icon}}A"), (&mut italic, "<i>A{{icon}}A")] {
            font.draw_to_physical_surface_with_clipper_and_images(
                surface,
                20,
                2,
                text,
                [255, 255, 255, 255],
                TextAlign::Left,
                true,
                projection,
                None,
                &images,
            );
        }

        let (_, physical_y) = projection.logical_to_physical(20.0, 2.0);
        let physical_height = 19.0 / f64::from(font.effective_scale()) * scale_y;
        let top_y = (physical_y - 0.5).ceil() as u32;
        let bottom_y = (physical_y + physical_height - 0.5).ceil() as u32 - 1;
        let plain_top = row_span(&plain, top_y).expect("plain top row");
        let plain_bottom = row_span(&plain, bottom_y).expect("plain bottom row");
        let italic_top = row_span(&italic, top_y).expect("italic top row");
        let italic_bottom = row_span(&italic, bottom_y).expect("italic bottom row");

        // The whole row leans as one: its top edge moves right and its bottom
        // edge moves left, exactly as the glyph-only and image-only rows do.
        assert!(
            italic_top.0 > plain_top.0,
            "the sheared row's top starts further right: {italic_top:?} vs {plain_top:?}"
        );
        assert!(
            italic_bottom.0 < plain_bottom.0,
            "and its bottom starts further left: {italic_bottom:?} vs {plain_bottom:?}"
        );

        // The lean is uniform across the row rather than applied to one kind of
        // content: both edges shift by the same amount.
        assert_eq!(
            italic_top.0 - plain_top.0,
            plain_bottom.0 - italic_bottom.0,
            "glyphs and the inline image shear from the same origin"
        );
    }

    #[test]
    fn fractional_native_italics_shear_glyphs_and_images_without_changing_metrics() {
        struct SolidImage(Vec<u8>);

        impl FontImageProvider for SolidImage {
            fn font_image(&self, tag: &str) -> Option<FontImageRef<'_>> {
                (tag == "icon").then_some(FontImageRef {
                    width: 19,
                    height: 19,
                    rgba: &self.0,
                })
            }
        }

        fn row_span(surface: &Surface, y: u32) -> Option<(u32, u32)> {
            let changed = (0..surface.width())
                .filter(|x| surface.get_pixel(*x, y).is_some_and(|pixel| pixel.a != 0))
                .collect::<Vec<_>>();
            changed.first().copied().zip(changed.last().copied())
        }

        let mut raster = ClonkFont::new(19);
        raster.cell_height = 19;
        raster.h_space = -2;
        raster.add_glyph(
            'A',
            GlyphCell {
                width: 19,
                pixels: vec![Color::opaque(255, 255, 255); 19 * 19],
            },
        );
        let font = NativeClonkFont {
            raster,
            application_scale: 1.5,
            effective_scale: 19.0 / 13.0,
            logical_height: 13,
            raster_height: 19,
            logical_h_space: -1,
            snap_to_pixels: false,
            retained_glyph_images: Mutex::new(HashMap::new()),
        };
        let images = SolidImage(vec![255; 19 * 19 * 4]);
        let projection =
            ClipperProjection::new(1.5, (63, 39), 59, clonk_graphics::Rect::new(0, 0, 63, 39));
        let (scale_x, scale_y) = projection.scale();
        assert_ne!(
            scale_x, scale_y,
            "rounded clipper projection is anisotropic"
        );
        let mut plain = Surface::new(95, 59, clonk_graphics::PixelFormat::Rgba8888);
        let mut italic = Surface::new(95, 59, clonk_graphics::PixelFormat::Rgba8888);
        let mut nested = Surface::new(95, 59, clonk_graphics::PixelFormat::Rgba8888);
        let mut image = Surface::new(95, 59, clonk_graphics::PixelFormat::Rgba8888);
        for (surface, text) in [
            (&mut plain, "A"),
            (&mut italic, "<i>A</i>"),
            (&mut nested, "<i><i>A</i></i>"),
            (&mut image, "<i>{{icon}}</i>"),
        ] {
            font.draw_to_physical_surface_with_clipper_and_images(
                surface,
                20,
                2,
                text,
                [255, 255, 255, 255],
                TextAlign::Left,
                true,
                projection,
                None,
                &images,
            );
        }

        // Keep the tag open so this isolates italic metrics from the native
        // trailing-close-tag h-space quirk.
        assert_eq!(font.measure("A", true), font.measure("<i>A", true));
        assert_eq!(
            font.measure_with_images("{{icon}}", true, &images),
            font.measure_with_images("<i>{{icon}}", true, &images),
        );
        let (_, physical_y) = projection.logical_to_physical(20.0, 2.0);
        let physical_height = 19.0 / f64::from(font.effective_scale()) * scale_y;
        let top_y = (physical_y - 0.5).ceil() as u32;
        let bottom_y = (physical_y + physical_height - 0.5).ceil() as u32 - 1;
        let plain_top = row_span(&plain, top_y).expect("plain top row");
        let plain_bottom = row_span(&plain, bottom_y).expect("plain bottom row");
        let italic_top = row_span(&italic, top_y).expect("italic top row");
        let italic_bottom = row_span(&italic, bottom_y).expect("italic bottom row");
        assert!(italic_top.0 > plain_top.0);
        assert!(italic_bottom.0 < plain_bottom.0);
        assert!(row_span(&nested, top_y).expect("nested top row").0 > italic_top.0);
        assert_eq!(row_span(&image, top_y), Some(italic_top));
        assert_eq!(row_span(&image, bottom_y), Some(italic_bottom));

        let triple = [
            NativeMarkupTag::Italic,
            NativeMarkupTag::Italic,
            NativeMarkupTag::Italic,
        ];
        assert_eq!(
            native_physical_shear(&triple, NativeDrawProjection::clipper(projection)),
            f64::from(((0.0_f32 - 0.3) - 0.3) - 0.3) * scale_x / scale_y,
        );

        let mut clipped = Surface::new(95, 59, clonk_graphics::PixelFormat::Rgba8888);
        clipped.set_clip(clonk_graphics::Rect::new(
            italic_top.0 as i32 + 1,
            top_y as i32,
            3,
            bottom_y - top_y + 1,
        ));
        font.draw_to_physical_surface_with_clipper_and_images(
            &mut clipped,
            20,
            2,
            "<i>A</i>",
            [255, 255, 255, 255],
            TextAlign::Left,
            true,
            projection,
            None,
            &images,
        );
        assert_ne!(
            italic.get_pixel(italic_top.0, top_y),
            Some(Color::transparent())
        );
        assert_eq!(
            clipped.get_pixel(italic_top.0, top_y),
            Some(Color::transparent())
        );
        assert_ne!(
            clipped.get_pixel(italic_top.0 + 1, top_y),
            Some(Color::transparent())
        );
    }

    #[test]
    fn fractional_native_italic_modulation_happens_after_linear_filtering() {
        let rgba = (0..20)
            .flat_map(|_| [[1_u8, 0, 0, 255], [2_u8, 0, 0, 255]])
            .flatten()
            .collect::<Vec<_>>();
        let image = FontImageRef {
            width: 2,
            height: 20,
            rgba: &rgba,
        };
        let mut surface = Surface::new(10, 10, clonk_graphics::PixelFormat::Rgba8888);

        blit_scaled_native_image(
            &mut surface,
            image,
            4.0,
            0.0,
            1.0,
            10.0,
            [126, 0, 0],
            255,
            None,
            -0.3,
        );

        assert_eq!(surface.get_pixel(4, 4), Some(Color::opaque(1, 0, 0)));

        let translucent = [255_u8, 0, 0, 200].repeat(20);
        let translucent = FontImageRef {
            width: 1,
            height: 20,
            rgba: &translucent,
        };
        let mut alpha_surface = Surface::new(10, 10, clonk_graphics::PixelFormat::Rgba8888);
        blit_scaled_native_image(
            &mut alpha_surface,
            translucent,
            4.0,
            0.0,
            1.0,
            10.0,
            [255, 255, 255],
            128,
            None,
            -0.3,
        );
        assert_eq!(
            alpha_surface.get_pixel(4, 4),
            Some(Color::new(73, 0, 0, 21))
        );
    }

    fn solid_fractional_native_font() -> NativeClonkFont {
        let mut raster = ClonkFont::new(1);
        raster.cell_height = 1;
        raster.h_space = 0;
        raster.add_glyph(
            'A',
            GlyphCell {
                width: 1,
                pixels: vec![Color::opaque(255, 255, 255)],
            },
        );
        NativeClonkFont {
            raster,
            application_scale: 1.5,
            effective_scale: 1.5,
            logical_height: 2,
            raster_height: 3,
            logical_h_space: 0,
            snap_to_pixels: false,
            retained_glyph_images: Mutex::new(HashMap::new()),
        }
    }

    fn solid_multi_glyph_native_font(glyphs: &str) -> NativeClonkFont {
        let mut raster = ClonkFont::new(1);
        raster.cell_height = 1;
        raster.h_space = 0;
        for (index, character) in glyphs.chars().enumerate() {
            // Distinct pixels per glyph, so nothing can coincidentally share a
            // retained identity by having identical contents.
            let shade = 40 + (index as u8) * 20;
            raster.add_glyph(
                character,
                GlyphCell {
                    width: 1,
                    pixels: vec![Color::opaque(shade, shade, shade)],
                },
            );
        }
        NativeClonkFont {
            raster,
            application_scale: 1.5,
            effective_scale: 1.5,
            logical_height: 2,
            raster_height: 3,
            logical_h_space: 0,
            snap_to_pixels: false,
            retained_glyph_images: Mutex::new(HashMap::new()),
        }
    }

    /// The baseline clonk-org/clonk-rs#286 gates its atlas work on: what a
    /// string of *distinct* glyphs costs today.
    ///
    /// `native_gpu_text_is_one_stable_textured_quad_per_glyph` covers the
    /// repeated-character case, where one retained identity is reused. Ordinary
    /// text is the opposite: every distinct character owns its own
    /// `NativeGlyphImages`, so an eight-character string binds eight textures
    /// and cannot form a single compatible run. That per-character binding is
    /// the cost an atlas would remove — criterion 2 there is one binding per
    /// page rather than one per character — so it is pinned here as the number
    /// to beat rather than left to be re-derived.
    #[test]
    fn distinct_native_glyphs_bind_one_texture_each() {
        let glyphs = "ABCDEFGH";
        let font = solid_multi_glyph_native_font(glyphs);
        let mut surface = Surface::new(64, 8, clonk_graphics::PixelFormat::Rgba8888);
        surface.begin_gpu_scene_capture();
        font.draw_to_physical_surface_with_offset(
            &mut surface,
            1,
            1,
            glyphs,
            [255, 255, 255, 255],
            TextAlign::Left,
            false,
            (0, 0),
            None,
        );
        let scene = surface
            .take_gpu_scene_capture()
            .expect("capture remains active")
            .into_scene([64, 8], Color::transparent(), &GammaRamp::identity());

        let textures = scene
            .commands
            .iter()
            .map(|command| match command {
                GpuCommand::Quad { texture, .. } => *texture,
                other => panic!("native glyph was CPU-rasterized instead of retained: {other:?}"),
            })
            .collect::<Vec<_>>();

        assert_eq!(
            textures.len(),
            glyphs.chars().count(),
            "one quad per glyph today"
        );
        let distinct: std::collections::HashSet<_> = textures.iter().collect();
        assert_eq!(
            distinct.len(),
            glyphs.chars().count(),
            "and one texture identity per distinct character, so no two adjacent \
             glyphs can share a binding"
        );
    }

    /// What a realistic label costs, which is the number
    /// clonk-org/clonk-rs#286's threshold has to be set against.
    ///
    /// `distinct_native_glyphs_bind_one_texture_each` shows the rule on eight
    /// characters; the question the atlas decision turns on is how the two
    /// counts diverge on real text, because repeated characters already share
    /// a retained identity and only *distinct* ones cost a binding.
    ///
    /// A label of this shape spends one command per glyph drawn but only one
    /// binding per distinct character — so an atlas would collapse the second
    /// number to one page, not the first. That ratio, not the glyph count, is
    /// what an atlas is worth.
    #[test]
    fn a_realistic_label_costs_one_binding_per_distinct_character() {
        // Ordinary UI text: far more glyphs drawn than distinct characters.
        // Kept within ten distinct characters because `solid_multi_glyph_native_font`
        // shades each glyph `40 + index * 20`, which leaves the `u8` past that.
        let label = "Round 3 - Round 4";
        let distinct_characters: std::collections::BTreeSet<char> = label.chars().collect();
        let alphabet: String = distinct_characters.iter().collect();
        let font = solid_multi_glyph_native_font(&alphabet);

        let mut surface = Surface::new(256, 8, clonk_graphics::PixelFormat::Rgba8888);
        surface.begin_gpu_scene_capture();
        font.draw_to_physical_surface_with_offset(
            &mut surface,
            1,
            1,
            label,
            [255, 255, 255, 255],
            TextAlign::Left,
            false,
            (0, 0),
            None,
        );
        let scene = surface
            .take_gpu_scene_capture()
            .expect("capture remains active")
            .into_scene([256, 8], Color::transparent(), &GammaRamp::identity());

        let textures = scene
            .commands
            .iter()
            .map(|command| match command {
                GpuCommand::Quad { texture, .. } => *texture,
                other => panic!("native glyph was CPU-rasterized instead of retained: {other:?}"),
            })
            .collect::<Vec<_>>();
        let distinct: std::collections::HashSet<_> = textures.iter().collect();

        assert_eq!(
            textures.len(),
            label.chars().count(),
            "one command per glyph drawn"
        );
        assert_eq!(
            distinct.len(),
            distinct_characters.len(),
            "one binding per distinct character, however often it repeats"
        );
        // The gap is the point: this label draws far more glyphs than it binds
        // textures, and an atlas removes bindings rather than draws.
        assert!(
            distinct.len() < textures.len(),
            "a realistic label must repeat characters, or it is not realistic",
        );
    }

    #[test]
    fn native_gpu_text_is_one_stable_textured_quad_per_glyph() {
        let font = solid_fractional_native_font();
        let capture = || {
            let mut surface = Surface::new(32, 8, clonk_graphics::PixelFormat::Rgba8888);
            surface.begin_gpu_scene_capture();
            font.draw_to_physical_surface_with_offset(
                &mut surface,
                1,
                1,
                "AAA",
                [255, 255, 255, 255],
                TextAlign::Left,
                false,
                (0, 0),
                None,
            );
            surface
                .take_gpu_scene_capture()
                .expect("capture remains active")
                .into_scene([32, 8], Color::transparent(), &GammaRamp::identity())
        };

        let first = capture();
        assert_eq!(first.commands.len(), 3);
        let textures = first
            .commands
            .iter()
            .map(|command| match command {
                GpuCommand::Quad { texture, .. } => *texture,
                other => panic!("native glyph was CPU-rasterized instead of retained: {other:?}"),
            })
            .collect::<Vec<_>>();
        assert!(textures.windows(2).all(|pair| pair[0] == pair[1]));

        let second = capture();
        let GpuCommand::Quad {
            texture: second_texture,
            ..
        } = second.commands[0]
        else {
            panic!("second native glyph capture was not textured");
        };
        assert_eq!(
            textures[0], second_texture,
            "glyph texture identity churned"
        );
    }

    #[test]
    fn captured_fractional_glyph_anchor_is_relative_to_the_rounded_clipper() {
        let fonts = NativeClonkFontSet {
            title: solid_fractional_native_font(),
            caption: solid_fractional_native_font(),
            text: solid_fractional_native_font(),
            main_small: solid_fractional_native_font(),
            mini: solid_fractional_native_font(),
            book_title: solid_fractional_native_font(),
            book_caption: solid_fractional_native_font(),
            book_text: solid_fractional_native_font(),
            book_small: solid_fractional_native_font(),
            scale: 1.5,
        };
        let command = CapturedClonkText {
            role: ClonkFontRole::GuiText,
            x: 1,
            y: 1,
            text: "A".to_string(),
            color: [255, 255, 255, 255],
            align: TextAlign::Left,
            markup: false,
            clip: Some(clonk_graphics::Rect::new(1, 1, 2, 2)),
            gamma: None,
            images: Vec::new(),
            zoom: 1.0,
        };
        let mut surface = Surface::new(6, 6, clonk_graphics::PixelFormat::Rgba8888);

        fonts.draw_captured_text(&mut surface, std::slice::from_ref(&command), (4, 4));

        assert_eq!(
            surface.get_pixel(1, 2),
            Some(Color::opaque(255, 255, 255)),
            "logical clip-left x=1 projects to physical x=1, not round(1*1.5)=2",
        );
        assert_eq!(surface.get_pixel(2, 2), Some(Color::transparent()));

        let mut replay_command = command;
        replay_command.color = [96, 180, 240, 127];
        replay_command.gamma = Some(GammaRamp::from_control_points([
            0x000000, 0x304080, 0xffffff,
        ]));
        let initial = (0..6 * 6)
            .flat_map(|index| {
                let value = (index * 7) as u8;
                [value, value.wrapping_add(19), value.wrapping_add(37), 255]
            })
            .collect::<Vec<_>>();
        let mut owned =
            Surface::from_bytes(6, 6, clonk_graphics::PixelFormat::Rgba8888, initial.clone())
                .expect("valid owned framebuffer");
        let mut borrowed = initial;
        fonts.draw_captured_text(&mut owned, std::slice::from_ref(&replay_command), (4, 4));
        {
            let mut view = clonk_graphics::RgbaSurfaceViewMut::new(6, 6, &mut borrowed)
                .expect("valid borrowed framebuffer");
            fonts.draw_captured_text_to(&mut view, std::slice::from_ref(&replay_command), (4, 4));
        }
        assert_eq!(borrowed, owned.pixels());
    }

    #[test]
    fn scale_one_native_font_preserves_the_existing_raster_and_metrics() {
        let native = build_native_font_set(&endeavour_bytes(), 1.0_f32)
            .expect("build scale-one native fonts");
        let logical = build_font_set(&endeavour_bytes()).expect("build logical fonts");

        // (1303 - (-308)) * size / 1024 (StdFont.cpp:351).
        assert_eq!(logical.title.line_height, 34);
        assert_eq!(logical.caption.line_height, 25);
        assert_eq!(logical.text.line_height, 22);
        assert_eq!(logical.main_small.line_height, 20);
        assert_eq!(logical.mini.line_height, 18);
        assert_eq!(logical.title.cell_height, 35);
        assert_eq!(logical.title.h_space, -1);
        assert_eq!(native.text.raster_height(), 14);
        assert_eq!(native.text.effective_scale(), 1.0);
        assert_eq!(native.text.raster_line_height(), logical.text.line_height);
        assert_eq!(native.text.raster_cell_height(), logical.text.cell_height);
        assert_eq!(native.text.glyph('A'), logical.text.glyph('A'));
        assert_eq!(
            native.text.measure("A A", false),
            logical.text.measure("A A", false)
        );
    }

    #[test]
    fn tooltip_is_an_independent_shadowless_main_14_font() {
        let bytes = endeavour_bytes();
        let regular = build_font_set(&bytes).expect("build GUI font set");
        let tooltip = build_tooltip_font(&bytes).expect("build tooltip font");

        let title_cell = regular.title.glyph('A').expect("title A glyph");
        assert!(title_cell.width > 5, "A should be wider than 5px");
        // The glyph core bakes to 254 (BltAlpha >>8 quirk) or 255 (pure src),
        // with full alpha; verify some near-white fully-opaque pixel exists.
        assert!(
            title_cell
                .pixels
                .iter()
                .any(|pixel| pixel.r >= 254 && pixel.g >= 254 && pixel.b >= 254 && pixel.a == 255),
            "expected an opaque white core pixel in 'A'"
        );
        assert!(
            title_cell
                .pixels
                .iter()
                .any(|pixel| pixel.r == 0 && pixel.a > 0 && pixel.a < 255),
            "expected a translucent black shadow pixel in 'A'"
        );

        assert_eq!(tooltip.line_height, regular.text.line_height);
        assert_eq!(tooltip.h_space, 0);
        assert_eq!(tooltip.cell_height, tooltip.line_height);
        let cell = tooltip.glyph('A').expect("tooltip A glyph");
        assert!(
            cell.pixels
                .iter()
                .all(|pixel| pixel.a == 0 || (pixel.r == 255 && pixel.g == 255 && pixel.b == 255)),
            "FontTooltip must not contain the regular font's black shadow pixels"
        );
        assert!(
            regular
                .text
                .glyph('A')
                .expect("regular A glyph")
                .pixels
                .iter()
                .any(|pixel| pixel.a > 0 && pixel.r == 0 && pixel.g == 0 && pixel.b == 0),
            "control glyph should retain a baked black shadow"
        );
    }

    #[test]
    fn native_font_scale_must_be_finite_and_positive() {
        for scale in [0.0_f32, -1.0, f32::INFINITY, f32::NAN] {
            assert!(build_native_font_set(&endeavour_bytes(), scale).is_err());
        }
    }

    #[test]
    fn vector_font_covers_unicode_charmap_and_missing_glyph() {
        let fonts = build_font_set(&endeavour_bytes()).expect("build GUI fonts");

        assert!(
            fonts.text.glyph('\u{0100}').is_some(),
            "U+0100 is present in Endeavour but outside Windows-1252"
        );
        assert!(fonts.text.measure("\u{0100}", false).0 > 0);

        assert!(
            fonts.text.glyph('\u{1f642}').is_none(),
            "an unmapped scalar must remain distinguishable from direct coverage"
        );
        assert!(
            fonts.text.measure("\u{1f642}", false).0 > 0,
            "FT_Load_Char resolves an unmapped scalar through glyph index zero"
        );
    }

    #[test]
    fn base_size_sixteen_uses_cpp_integer_derived_sizes() {
        let fonts = build_font_set_at_size(&endeavour_bytes(), 16).expect("build size-16 fonts");
        assert_eq!(fonts.mini.line_height, 20); // 13px
        assert_eq!(fonts.main_small.line_height, 22); // 14px
        assert_eq!(fonts.text.line_height, 25); // 16px
        assert_eq!(fonts.caption.line_height, 28); // 18px
        assert_eq!(fonts.title.line_height, 39); // 25px
    }

    #[test]
    fn weight_seven_hundred_applies_the_freetype_width_transform() {
        let bytes = endeavour_bytes();
        let regular = build_vector_font(&bytes, 14, 400, true).expect("regular font");
        let bold = build_vector_font(&bytes, 14, 700, true).expect("weighted font");
        assert!(bold.measure("MMMM", false).0 > regular.measure("MMMM", false).0);
    }

    #[test]
    fn prerendered_font_scans_delimiters_rows_and_indent() {
        // Two 2x2 cells separated by red, then green to end the row. The
        // delimiter at x=0,y=2 establishes iGfxLineHgt=2.
        let (width, height) = (6, 5);
        let mut rgba = vec![0_u8; width * height * 4];
        let mut put = |x: usize, y: usize, color: [u8; 4]| {
            let index = (y * width + x) * 4;
            rgba[index..index + 4].copy_from_slice(&color);
        };
        for y in 0..2 {
            for x in [0, 1, 3, 4] {
                put(x, y, [255, 255, 255, 255]);
            }
            put(2, y, [255, 0, 0, 255]);
            put(5, y, [0, 255, 0, 255]);
        }
        put(0, 2, [255, 0, 0, 255]);
        let font =
            build_prerendered_font(width as u32, height as u32, &rgba, 1).expect("bitmap font");
        assert_eq!(font.line_height, 1);
        assert_eq!(font.cell_height, 2);
        assert_eq!(font.h_space, -1);
        assert_eq!(font.glyph(' ').expect("space").width, 2);
        assert_eq!(font.glyph('!').expect("bang").width, 2);

        // ClrDw2W includes the (inverted) alpha nibble. A transparent red
        // pixel therefore belongs to glyph data rather than ending a cell.
        let mut transparent_delimiter = rgba;
        transparent_delimiter[(2 * width) * 4 + 3] = 0;
        let font = build_prerendered_font(width as u32, height as u32, &transparent_delimiter, 1)
            .expect("transparent red is not a delimiter");
        assert_eq!(font.cell_height, height as i32);
    }

    #[test]
    fn expands_hotkey_marker_to_color_markup() {
        assert_eq!(
            expand_hotkey_markup("&Start Game"),
            ("<c ffffff7f>S</c>tart Game".to_string(), Some('S'))
        );
        assert_eq!(
            expand_hotkey_markup("E&xit"),
            ("E<c ffffff7f>x</c>it".to_string(), Some('X'))
        );
        assert_eq!(
            expand_hotkey_markup("No Marker"),
            ("No Marker".to_string(), None)
        );
    }

    #[test]
    fn button_font_prefers_largest_fitting_font() {
        let set = ClonkFontSet {
            title: ClonkFont::new(34),
            caption: ClonkFont::new(25),
            text: ClonkFont::new(22),
            main_small: ClonkFont::new(20),
            mini: ClonkFont::new(18),
        };
        // 40px button: title (34) fits 38.
        assert_eq!(set.button_font(40).line_height, 34);
        // 32px button: title doesn't fit 30, caption (25) does.
        assert_eq!(set.button_font(32).line_height, 25);
        // tiny button: falls back to text font.
        assert_eq!(set.button_font(20).line_height, 22);
    }

    /// The same five Info-row forms at **scale 1** (clonk-org/clonk-rs#563).
    ///
    /// The neighbouring fractional subcases run an anisotropic projection,
    /// where `native_physical_shear` divides by `scale_y` and multiplies by
    /// `scale_x`. At scale 1 that correction is the identity, so this is the
    /// case that pins the *raw* `mat[1] -= 0.3` per tag
    /// (`StdMarkup.cpp:24-28`) with nothing to hide a wrong constant or a
    /// dropped scale term: one tag leans by 0.3, two lean by exactly twice
    /// that, and an inline image leans with the glyphs beside it.
    #[test]
    fn native_italics_shear_cumulatively_at_scale_one() {
        struct SolidImage(Vec<u8>);

        impl FontImageProvider for SolidImage {
            fn font_image(&self, tag: &str) -> Option<FontImageRef<'_>> {
                (tag == "icon").then_some(FontImageRef {
                    width: 13,
                    height: 13,
                    rgba: &self.0,
                })
            }
        }

        fn row_start(surface: &Surface, y: u32) -> Option<u32> {
            (0..surface.width())
                .find(|x| surface.get_pixel(*x, y).is_some_and(|pixel| pixel.a != 0))
        }

        let mut raster = ClonkFont::new(13);
        raster.cell_height = 13;
        raster.h_space = -1;
        raster.add_glyph(
            'A',
            GlyphCell {
                width: 13,
                pixels: vec![Color::opaque(255, 255, 255); 13 * 13],
            },
        );
        let font = NativeClonkFont {
            raster,
            application_scale: 1.0,
            effective_scale: 1.0,
            logical_height: 13,
            raster_height: 13,
            logical_h_space: -1,
            snap_to_pixels: false,
            retained_glyph_images: Mutex::new(HashMap::new()),
        };
        let images = SolidImage(vec![255; 13 * 13 * 4]);
        let projection =
            ClipperProjection::new(1.0, (64, 40), 64, clonk_graphics::Rect::new(0, 0, 64, 40));
        let (scale_x, scale_y) = projection.scale();
        assert_eq!(
            (scale_x, scale_y),
            (1.0, 1.0),
            "this subcase is the isotropic one"
        );

        // One tag is 0.3; the correction term is the identity here, so the
        // physical shear must be the bare constant.
        assert_eq!(
            native_physical_shear(
                &[NativeMarkupTag::Italic],
                NativeDrawProjection::clipper(projection)
            ),
            f64::from(-0.3_f32),
        );
        assert_eq!(
            native_physical_shear(
                &[NativeMarkupTag::Italic, NativeMarkupTag::Italic],
                NativeDrawProjection::clipper(projection)
            ),
            f64::from(-0.3_f32 - 0.3),
            "a nested tag leans exactly twice as far, not once and not thrice"
        );

        let mut plain = Surface::new(64, 40, clonk_graphics::PixelFormat::Rgba8888);
        let mut italic = Surface::new(64, 40, clonk_graphics::PixelFormat::Rgba8888);
        let mut nested = Surface::new(64, 40, clonk_graphics::PixelFormat::Rgba8888);
        let mut image = Surface::new(64, 40, clonk_graphics::PixelFormat::Rgba8888);
        for (surface, text) in [
            (&mut plain, "A"),
            (&mut italic, "<i>A</i>"),
            (&mut nested, "<i><i>A</i></i>"),
            (&mut image, "<i>{{icon}}</i>"),
        ] {
            font.draw_to_physical_surface_with_clipper_and_images(
                surface,
                10,
                2,
                text,
                [255, 255, 255, 255],
                TextAlign::Left,
                true,
                projection,
                None,
                &images,
            );
        }

        // Markup never changes advance widths, so measuring is unaffected.
        // The tag is left open to isolate this from the trailing-close-tag
        // h-space quirk the fractional subcase documents.
        assert_eq!(font.measure("A", true), font.measure("<i>A", true));
        assert_eq!(font.measure("A", true), font.measure("<i><i>A", true));

        // The clipper offsets the row within the surface, so the glyph band is
        // taken from the plain render rather than assumed.
        let rows: Vec<u32> = (0..40)
            .filter(|y| row_start(&plain, *y).is_some())
            .collect();
        let (top, bottom) = (
            *rows.first().expect("the plain row drew something"),
            *rows.last().expect("the plain row drew something"),
        );
        assert!(bottom > top, "the glyph spans more than one row");
        let plain_top = row_start(&plain, top).expect("plain top row");
        let italic_top = row_start(&italic, top).expect("italic top row");
        let nested_top = row_start(&nested, top).expect("nested top row");
        assert!(
            italic_top > plain_top,
            "italic leans the top of the glyph to the right"
        );
        assert!(
            nested_top > italic_top,
            "a nested tag leans it further still"
        );
        assert!(
            row_start(&italic, bottom).expect("italic bottom row")
                < row_start(&plain, bottom).expect("plain bottom row"),
            "and leans the bottom the other way, so the row shears rather than shifting"
        );
        assert_eq!(
            row_start(&image, top),
            Some(italic_top),
            "an inline image shears with the glyphs it sits among"
        );

        // A row carrying glyphs *and* an image is the form Info menus actually
        // use — prose with an icon in it — and the case where a shear applied
        // to one but not the other would show. Both ends of the row must lean,
        // and by the same amount as the glyph-only row.
        let mut mixed = Surface::new(64, 40, clonk_graphics::PixelFormat::Rgba8888);
        font.draw_to_physical_surface_with_clipper_and_images(
            &mut mixed,
            10,
            2,
            "<i>A{{icon}}</i>",
            [255, 255, 255, 255],
            TextAlign::Left,
            true,
            projection,
            None,
            &images,
        );
        assert_eq!(
            row_start(&mixed, top),
            Some(italic_top),
            "the mixed row starts where the glyph-only italic row starts"
        );
        let mixed_lean = i64::from(row_start(&mixed, top).expect("mixed top"))
            - i64::from(row_start(&mixed, bottom).expect("mixed bottom"));
        let plain_lean =
            i64::from(italic_top) - i64::from(row_start(&italic, bottom).expect("italic bottom"));
        assert_eq!(
            mixed_lean, plain_lean,
            "text and image in one row lean together, not by different amounts"
        );
    }
}
