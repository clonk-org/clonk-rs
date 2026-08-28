//! Reusable pixel-faithful rendering primitives for the classic `C4GUI`
//! furniture (`src/C4Gui*.cpp`).

use crate::clonk_fonts::{expand_hotkey_markup, ClonkFontSet};
pub use crate::startup_main_menu::IntRect;
use crate::ImageData;
use clonk_graphics::clonk_font::{ClonkFont, TextAlign};
use clonk_graphics::{Color, GammaRamp, PixelFormat, Surface, SurfaceDrawTarget};
use clonk_gui::Rect as GuiRect;
use std::{cell::RefCell, collections::HashMap};

/// `C4GUI_StandardBGColor` (`C4Gui.h:80`). Box and line alpha in the C++
/// renderer is inverted: `0x00` is opaque and `0xff` transparent.
pub const STANDARD_BACKGROUND_COLOR: u32 = 0x5f00_0000;

/// Assets shared by a classic captioned dialog and its buttons.
///
/// `button_highlight` may be omitted when no control is highlighted. Callers
/// that bilinearly scale the original PNG should first clear RGB under fully
/// transparent pixels with [`blacken_transparent_pixels`], matching the
/// startup-dialog renderers.
#[derive(Clone, Copy)]
pub struct ClassicGuiSkin<'a> {
    caption: &'a ImageData,
    button: &'a ImageData,
    button_down: &'a ImageData,
    button_highlight: Option<&'a ImageData>,
}

/// The two independent visual flags used by `C4GUI::Button::DrawElement`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClassicButtonState {
    pub pressed: bool,
    pub highlighted: bool,
}

impl<'a> ClassicGuiSkin<'a> {
    pub const fn new(
        caption: &'a ImageData,
        button: &'a ImageData,
        button_down: &'a ImageData,
        button_highlight: Option<&'a ImageData>,
    ) -> Self {
        Self {
            caption,
            button,
            button_down,
            button_highlight,
        }
    }

    /// Draws the standard translucent dialog background and brown 3D frame
    /// (`C4GUI::Dialog::DrawElement`, `C4GuiDialogs.cpp:537-550`).
    pub fn draw_dialog(&self, surface: &mut Surface, rect: IntRect, gamma: Option<&GammaRamp>) {
        draw_engine_box(
            surface,
            rect.x,
            rect.y,
            rect.x + rect.w - 1,
            rect.y + rect.h - 1,
            STANDARD_BACKGROUND_COLOR,
            gamma,
        );
        draw_3d_frame(surface, rect, gamma);
    }

    /// Draws one `C4GUI::WoodenLabel`: `GUICaption.png` with its explicit
    /// 32px border, then clipped text centered vertically one pixel upward
    /// (`C4GuiLabels.cpp:168-214`, `C4Gui.cpp:1087-1088`).
    #[allow(clippy::too_many_arguments)]
    pub fn draw_caption(
        &self,
        surface: &mut Surface,
        rect: IntRect,
        text: &str,
        font: &ClonkFont,
        color: [u8; 4],
        align: TextAlign,
        gamma: Option<&GammaRamp>,
    ) {
        self.draw_caption_with_right_indent(surface, rect, text, font, color, align, 0, gamma);
    }

    /// Draws a wooden caption while reserving pixels at its right edge for
    /// controls such as `C4GUI::Dialog`'s close button.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_caption_with_right_indent(
        &self,
        surface: &mut Surface,
        rect: IntRect,
        text: &str,
        font: &ClonkFont,
        color: [u8; 4],
        align: TextAlign,
        right_indent: i32,
        gamma: Option<&GammaRamp>,
    ) {
        self.draw_caption_scrolled(
            surface,
            rect,
            text,
            font,
            color,
            align,
            right_indent,
            0,
            gamma,
        );
    }

    /// `WoodenLabel::DrawElement` with its current horizontal auto-scroll
    /// offset. The caller owns the frame-driven scroll timer/state.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw_caption_scrolled(
        &self,
        surface: &mut Surface,
        rect: IntRect,
        text: &str,
        font: &ClonkFont,
        color: [u8; 4],
        align: TextAlign,
        right_indent: i32,
        scroll_offset: i32,
        gamma: Option<&GammaRamp>,
    ) {
        draw_bar(surface, rect, self.caption, 32, gamma);
        let text_rect = rect.with_width((rect.w - right_indent.max(0)).max(1));
        let x = match align {
            TextAlign::Left => text_rect.x + 5,
            TextAlign::Center => text_rect.x + text_rect.w / 2,
            TextAlign::Right => text_rect.x + text_rect.w,
        } - scroll_offset;
        draw_clipped_text(
            surface,
            font,
            x,
            rect.y + (rect.h - font.line_height) / 2 - 1,
            text,
            color,
            align,
            gamma,
            inclusive_clip(text_rect),
        );
    }

    pub(crate) fn validate_message_dialog_assets(&self) -> anyhow::Result<()> {
        validate_bar_image("GUICaption.png", self.caption, 32)?;
        validate_bar_image("GUIButton.png", self.button, self.button.height())?;
        validate_bar_image(
            "GUIButtonDown.png",
            self.button_down,
            self.button_down.height(),
        )?;
        let highlight = self.button_highlight.ok_or_else(|| {
            anyhow::anyhow!("GUIButtonHighlight.png is required for classic message dialogs")
        })?;
        validate_nonempty_image("GUIButtonHighlight.png", highlight)
    }

    /// Draws one standard `C4GUI::Button`: normal/down three-slice plank,
    /// optional additive focus overlay, fitting GUI font, hotkey markup and
    /// the one-pixel pressed offset (`C4GuiButton.cpp:81-109`).
    pub fn draw_button(
        &self,
        surface: &mut Surface,
        rect: IntRect,
        label: &str,
        fonts: &ClonkFontSet,
        state: ClassicButtonState,
        gamma: Option<&GammaRamp>,
    ) {
        self.draw_button_with_highlight(
            surface,
            rect,
            label,
            fonts,
            state,
            self.button_highlight,
            gamma,
        );
    }

    /// Draws a standard button while overriding only the additive highlight
    /// facet. Modal resource bundles use this to share one validated,
    /// transparent-RGB-clean copy between text and icon buttons.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw_button_with_highlight(
        &self,
        surface: &mut Surface,
        rect: IntRect,
        label: &str,
        fonts: &ClonkFontSet,
        state: ClassicButtonState,
        button_highlight: Option<&ImageData>,
        gamma: Option<&GammaRamp>,
    ) {
        let plank = if state.pressed {
            self.button_down
        } else {
            self.button
        };
        draw_bar(surface, rect, plank, plank.height(), gamma);
        if state.highlighted {
            if let Some(highlight) = button_highlight {
                crate::draw_image_bilinear_additive(
                    surface,
                    &GuiRect::new(
                        (rect.x + 5) as f32,
                        (rect.y + 3) as f32,
                        (rect.w - 10) as f32,
                        (rect.h - 6) as f32,
                    ),
                    highlight,
                    gamma,
                );
            }
        }
        let font = fonts.button_font(rect.h);
        let (text, _) = expand_hotkey_markup(label);
        let offset = i32::from(state.pressed);
        font.draw_with_gamma(
            surface,
            (rect.x + rect.x + rect.w - 1) / 2 + offset,
            (rect.y + rect.y + rect.h - 1 - font.line_height) / 2 + offset,
            &text,
            [0xff, 0xff, 0x00, 0xff],
            TextAlign::Center,
            true,
            gamma,
        );
    }
}

fn validate_bar_image(name: &str, image: &ImageData, border: u32) -> anyhow::Result<()> {
    anyhow::ensure!(
        image.height() > 0 && border > 0 && image.width() >= border.saturating_mul(2),
        "{name} cannot form a classic three-slice bar: got {}x{} with {border}px borders",
        image.width(),
        image.height()
    );
    Ok(())
}

fn validate_nonempty_image(name: &str, image: &ImageData) -> anyhow::Result<()> {
    anyhow::ensure!(
        image.width() > 0 && image.height() > 0,
        "{name} must not be empty for classic message dialogs"
    );
    Ok(())
}

/// Draws a native-height horizontal three-slice bar with an explicit border
/// width. This is the exact branch of `C4GUI::Element::DrawBar`, including its
/// narrow-bar overflow behavior (`C4Gui.cpp:283-311`).
pub fn draw_bar(
    surface: &mut Surface,
    rect: IntRect,
    image: &ImageData,
    border: u32,
    gamma: Option<&GammaRamp>,
) {
    let h = image.height();
    if h == 0 || rect.w <= 0 || rect.h <= 0 || border == 0 || image.width() < 2 * border {
        return;
    }
    let mid_w = image.width().saturating_sub(2 * border);
    let bar_w = rect.w;
    if rect.h != h as i32 {
        let zoom = rect.h as f32 / h as f32;
        let begin_w = (zoom * border as f32) as i32;
        let middle_w = (zoom * mid_w as f32) as i32;
        let right_show = border / 3;
        draw_facet_stretch(
            surface,
            image,
            (0.0, 0.0, border as f32, h as f32),
            (rect.x as f32, rect.y as f32, begin_w as f32, rect.h as f32),
            gamma,
        );
        if middle_w > 0 {
            let mut ix = begin_w;
            while (ix as f32) < bar_w as f32 - zoom * right_show as f32 {
                let width = middle_w.min(bar_w - (zoom * right_show as f32) as i32 - ix);
                let source_width = (width as f32 / zoom) as i32;
                if width <= 0 || source_width <= 0 {
                    break;
                }
                draw_facet_stretch(
                    surface,
                    image,
                    (border as f32, 0.0, source_width as f32, h as f32),
                    (
                        (rect.x + ix) as f32,
                        rect.y as f32,
                        width as f32,
                        rect.h as f32,
                    ),
                    gamma,
                );
                ix += middle_w;
            }
        }
        let end_w = (zoom * border as f32) as i32;
        draw_facet_stretch(
            surface,
            image,
            (
                (image.width() - border) as f32,
                0.0,
                border as f32,
                h as f32,
            ),
            (
                (rect.x + bar_w - end_w) as f32,
                rect.y as f32,
                end_w as f32,
                rect.h as f32,
            ),
            gamma,
        );
        return;
    }
    let end_show = (border / 3) as i32;

    let begin_w = (border as i32).clamp(0, bar_w.max(0)) as u32;
    crate::draw_image_strip(surface, rect.x, rect.y, image, 0, 0, begin_w, h, gamma);

    if mid_w > 0 {
        let mut ix = border as i32;
        while ix < bar_w - end_show {
            let tile_w = (mid_w as i32).min(bar_w - end_show - ix).max(0) as u32;
            crate::draw_image_strip(
                surface,
                rect.x + ix,
                rect.y,
                image,
                border,
                0,
                tile_w,
                h,
                gamma,
            );
            ix += mid_w as i32;
        }
    }

    let end_w = (border as i32).clamp(0, bar_w.max(0)) as u32;
    let end_src_x = image.width() - border + (border - end_w);
    crate::draw_image_strip(
        surface,
        rect.x + bar_w - end_w as i32,
        rect.y,
        image,
        end_src_x,
        0,
        end_w,
        h,
        gamma,
    );
}

/// Stretch-blits an image subregion like `CStdDDraw::Blit`: one quad per C++
/// texture tile, GL_LINEAR sampling and fragment gamma before alpha blending
/// (`StdDDraw2.cpp:637-786`, `C4Surface.cpp:166-189,1102-1103`).
pub fn draw_facet_stretch(
    surface: &mut Surface,
    image: &ImageData,
    source: (f32, f32, f32, f32),
    destination: (f32, f32, f32, f32),
    gamma: Option<&GammaRamp>,
) {
    let (source_x, source_y, source_width, source_height) = source;
    let (target_x, target_y, target_width, target_height) = destination;
    if crate::draw_image_source_with_active_renderer_config(
        surface,
        &GuiRect::new(target_x, target_y, target_width, target_height),
        image,
        source,
        clonk_graphics::BlitSampling::Linear,
        gamma,
    ) {
        return;
    }
    if crate::capture_gpu_gui_image(
        surface,
        (target_x, target_y, target_width, target_height),
        image,
        crate::FloatSourceRect {
            x: source_x,
            y: source_y,
            width: source_width,
            height: source_height,
        },
        clonk_graphics::GpuSampler::Linear,
        crate::BilinearBlend::AlphaOver,
        None,
        gamma,
    ) {
        return;
    }
    if source_width <= 0.0 || source_height <= 0.0 || target_width <= 0.0 || target_height <= 0.0 {
        return;
    }
    let scale_x = target_width / source_width;
    let scale_y = target_height / source_height;
    let tile_size = cpp_texture_size(image.width(), image.height()) as i32;
    let image_tiles_x = (image.width() as i32 - 1) / tile_size + 1;
    let image_tiles_y = (image.height() as i32 - 1) / tile_size + 1;
    let first_tile_x = ((source_x / tile_size as f32) as i32).max(0);
    let first_tile_y = ((source_y / tile_size as f32) as i32).max(0);
    let last_tile_x = (((source_x + source_width - 1.0) as i32) / tile_size + 1).min(image_tiles_x);
    let last_tile_y =
        (((source_y + source_height - 1.0) as i32) / tile_size + 1).min(image_tiles_y);

    for tile_y in first_tile_y..last_tile_y {
        for tile_x in first_tile_x..last_tile_x {
            let (tile_origin_x, tile_origin_y) = (tile_x * tile_size, tile_y * tile_size);
            let source_left = (source_x - tile_origin_x as f32).max(0.0);
            let source_top = (source_y - tile_origin_y as f32).max(0.0);
            let source_right =
                (source_x + source_width - tile_origin_x as f32).min(tile_size as f32);
            let source_bottom =
                (source_y + source_height - tile_origin_y as f32).min(tile_size as f32);
            let target_left = (source_left + tile_origin_x as f32 - source_x) * scale_x + target_x;
            let target_top = (source_top + tile_origin_y as f32 - source_y) * scale_y + target_y;
            let target_right =
                (source_right + tile_origin_x as f32 - source_x) * scale_x + target_x;
            let target_bottom =
                (source_bottom + tile_origin_y as f32 - source_y) * scale_y + target_y;
            let first_pixel_x = (target_left - 0.5).ceil() as i32;
            let first_pixel_y = (target_top - 0.5).ceil() as i32;

            for pixel_y in first_pixel_y.max(0)..surface.height() as i32 {
                if pixel_y as f32 + 0.5 >= target_bottom {
                    break;
                }
                for pixel_x in first_pixel_x.max(0)..surface.width() as i32 {
                    if pixel_x as f32 + 0.5 >= target_right {
                        break;
                    }
                    let sample_x = source_x - tile_origin_x as f32
                        + (pixel_x as f32 + 0.5 - target_x) / scale_x
                        - 0.5;
                    let sample_y = source_y - tile_origin_y as f32
                        + (pixel_y as f32 + 0.5 - target_y) / scale_y
                        - 0.5;
                    let sample = bilinear_sample_tile(
                        image,
                        tile_origin_x,
                        tile_origin_y,
                        tile_size,
                        sample_x,
                        sample_y,
                    );
                    if sample[3] <= 0.0 {
                        continue;
                    }
                    if surface.is_gpu_scene_capture_active() {
                        // Fallback rasterization during capture must stay a
                        // painter-ordered retained fragment instead of
                        // blending against stale CPU backing.
                        let _ = surface.blend_fragment_over(
                            pixel_x as u32,
                            pixel_y as u32,
                            sample,
                            gamma,
                        );
                        continue;
                    }
                    let alpha = (sample[3] / 255.0).clamp(0.0, 1.0);
                    let destination = surface
                        .get_pixel(pixel_x as u32, pixel_y as u32)
                        .unwrap_or_default();
                    let blend = |source: f32, destination: u8| {
                        (encode_filtered_channel(gamma, source) * alpha
                            + f32::from(destination) * (1.0 - alpha))
                            .round()
                            .clamp(0.0, 255.0) as u8
                    };
                    let _ = surface.set_pixel(
                        pixel_x as u32,
                        pixel_y as u32,
                        Color::new(
                            blend(sample[0], destination.r),
                            blend(sample[1], destination.g),
                            blend(sample[2], destination.b),
                            alpha_over_alpha(alpha, destination.a),
                        ),
                    );
                }
            }
        }
    }
}

fn encode_filtered_channel(gamma: Option<&GammaRamp>, channel: f32) -> f32 {
    gamma
        .map(|ramp| f32::from(ramp.encode_float(channel)))
        .unwrap_or_else(|| channel.round().clamp(0.0, 255.0))
}

/// Alpha-channel counterpart of the live source-over framebuffer blend.
/// Keeping the destination alpha matters for transparent ordered layers:
/// their RGB is already premultiplied by the same opacity and must not turn
/// into an opaque replacement when the layer is presented later.
fn alpha_over_alpha(source_opacity: f32, destination_alpha: u8) -> u8 {
    (255.0 * source_opacity + f32::from(destination_alpha) * (1.0 - source_opacity))
        .round()
        .clamp(0.0, 255.0) as u8
}

/// Nearest-neighbour `C4Facet::DrawX` for an integer source and destination
/// rectangle. Retained targets receive one textured painter command instead
/// of a precomposited replacement copied from the CPU backing store.
pub fn draw_facet_nearest(
    surface: &mut Surface,
    image: &ImageData,
    source: clonk_graphics::Rect,
    destination: clonk_graphics::Rect,
    gamma: Option<&GammaRamp>,
) {
    if source.width == 0 || source.height == 0 || destination.width == 0 || destination.height == 0
    {
        return;
    }
    let target = GuiRect::new(
        destination.x as f32,
        destination.y as f32,
        destination.width as f32,
        destination.height as f32,
    );
    let source_float = (
        source.x as f32,
        source.y as f32,
        source.width as f32,
        source.height as f32,
    );
    if crate::draw_image_source_with_active_renderer_config(
        surface,
        &target,
        image,
        source_float,
        clonk_graphics::BlitSampling::Nearest,
        gamma,
    ) {
        return;
    }
    if crate::capture_gpu_gui_image(
        surface,
        (
            destination.x as f32,
            destination.y as f32,
            destination.width as f32,
            destination.height as f32,
        ),
        image,
        crate::FloatSourceRect {
            x: source.x as f32,
            y: source.y as f32,
            width: source.width as f32,
            height: source.height as f32,
        },
        clonk_graphics::GpuSampler::Nearest,
        crate::BilinearBlend::AlphaOver,
        None,
        gamma,
    ) {
        return;
    }

    let bounds = surface.bounds();
    let pixels = image.pixels();
    for dy in 0..destination.height {
        let target_y = destination.y + dy as i32;
        if target_y < bounds.y || target_y >= bounds.y + bounds.height as i32 {
            continue;
        }
        let source_y = source.y
            + ((u64::from(dy) * u64::from(source.height)) / u64::from(destination.height)) as i32;
        if source_y < 0 || source_y >= image.height() as i32 {
            continue;
        }
        for dx in 0..destination.width {
            let target_x = destination.x + dx as i32;
            if target_x < bounds.x || target_x >= bounds.x + bounds.width as i32 {
                continue;
            }
            let source_x = source.x
                + ((u64::from(dx) * u64::from(source.width)) / u64::from(destination.width)) as i32;
            if source_x < 0 || source_x >= image.width() as i32 {
                continue;
            }
            let offset = ((source_y as u32 * image.width() + source_x as u32) * 4) as usize;
            let Some(pixel) = pixels.get(offset..offset + 4) else {
                continue;
            };
            if pixel[3] == 0 {
                continue;
            }
            let color = Color::new(pixel[0], pixel[1], pixel[2], pixel[3]);
            let result = if let Some(gamma) = gamma {
                let destination = surface
                    .get_pixel(target_x as u32, target_y as u32)
                    .unwrap_or_default();
                let output = if color.a == 255 {
                    crate::gamma_encode_fragment(color, gamma)
                } else {
                    crate::gamma_blend_fragment_over(color, destination, gamma)
                };
                surface.set_pixel(target_x as u32, target_y as u32, output)
            } else if color.a == 255 {
                surface.set_pixel(target_x as u32, target_y as u32, color)
            } else {
                // Keep the established software/headless byte oracle. The
                // retained branch above submits the unblended source quad;
                // the legacy CPU path deliberately truncates its integer
                // source-over products through `Surface::blend_pixel`.
                surface.blend_pixel(target_x as u32, target_y as u32, color)
            };
            if result.is_err() {
                return;
            }
        }
    }
}

fn cpp_texture_size(width: u32, height: u32) -> u32 {
    let required = width.min(height).max(1);
    let mut size = 1u32;
    while size < required {
        size <<= 1;
    }
    size.min(4096)
}

fn bilinear_sample_tile(
    image: &ImageData,
    tile_x: i32,
    tile_y: i32,
    tile_size: i32,
    sample_x: f32,
    sample_y: f32,
) -> [f32; 4] {
    let texel = |relative_x: i32, relative_y: i32| -> [f32; 4] {
        let x = tile_x + relative_x.clamp(0, tile_size - 1);
        let y = tile_y + relative_y.clamp(0, tile_size - 1);
        if x < 0 || y < 0 || x >= image.width() as i32 || y >= image.height() as i32 {
            return [255.0, 255.0, 255.0, 0.0];
        }
        let index = ((y as u32 * image.width() + x as u32) * 4) as usize;
        image
            .pixels()
            .get(index..index + 4)
            .map(|pixel| {
                [
                    pixel[0] as f32,
                    pixel[1] as f32,
                    pixel[2] as f32,
                    pixel[3] as f32,
                ]
            })
            .unwrap_or([0.0; 4])
    };
    let (x0, y0) = (sample_x.floor() as i32, sample_y.floor() as i32);
    let (fraction_x, fraction_y) = (sample_x - x0 as f32, sample_y - y0 as f32);
    let (top_left, top_right) = (texel(x0, y0), texel(x0 + 1, y0));
    let (bottom_left, bottom_right) = (texel(x0, y0 + 1), texel(x0 + 1, y0 + 1));
    std::array::from_fn(|channel| {
        let top = top_left[channel] * (1.0 - fraction_x) + top_right[channel] * fraction_x;
        let bottom = bottom_left[channel] * (1.0 - fraction_x) + bottom_right[channel] * fraction_x;
        top * (1.0 - fraction_y) + bottom * fraction_y
    })
}

/// `CStdDDraw::DrawBoxDw`: inclusive coordinates and engine AARRGGBB color
/// with inverted alpha (`StdDDraw2.cpp:1401-1404`, `StdGL.cpp:846-891`).
#[allow(clippy::too_many_arguments)]
pub fn draw_engine_box(
    surface: &mut Surface,
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    color: u32,
    gamma: Option<&GammaRamp>,
) {
    if x2 < x1 || y2 < y1 {
        return;
    }
    let width = i64::from(x2) - i64::from(x1) + 1;
    let height = i64::from(y2) - i64::from(y1) + 1;
    if surface.is_gpu_scene_capture_active()
        || crate::active_advanced_renderer_config()
            .is_some_and(|config| config.blit_offset != 0 || config.no_box_fades)
    {
        crate::draw_color_rect(
            surface,
            clonk_graphics::Rect::new(
                x1,
                y1,
                width.clamp(0, i64::from(u32::MAX)) as u32,
                height.clamp(0, i64::from(u32::MAX)) as u32,
            ),
            Color::new(
                ((color >> 16) & 0xff) as u8,
                ((color >> 8) & 0xff) as u8,
                (color & 0xff) as u8,
                255 - ((color >> 24) & 0xff) as u8,
            ),
            gamma,
        );
        return;
    }

    // Preserve the byte-exact compatibility rasterizer for CPU/headless
    // callers. In particular, native GL's final RGBA8 store rounds these
    // products; the older generic `blend_colors` helper truncates them.
    let opacity = (255 - ((color >> 24) & 0xff)) as f32 / 255.0;
    if opacity <= 0.0 {
        return;
    }
    let encode = |channel: u32| -> f32 {
        let value = (channel & 0xff) as f32;
        gamma.map_or(value, |ramp| f32::from(ramp.encode_float(value)))
    };
    let (red, green, blue) = (encode(color >> 16), encode(color >> 8), encode(color));
    let blend = |source: f32, destination: u8| {
        (source * opacity + f32::from(destination) * (1.0 - opacity))
            .round()
            .clamp(0.0, 255.0) as u8
    };
    for y in y1.max(0)..=y2.min(surface.height() as i32 - 1) {
        for x in x1.max(0)..=x2.min(surface.width() as i32 - 1) {
            let destination = surface.get_pixel(x as u32, y as u32).unwrap_or_default();
            let _ = surface.set_pixel(
                x as u32,
                y as u32,
                Color::new(
                    blend(red, destination.r),
                    blend(green, destination.g),
                    blend(blue, destination.b),
                    (255.0 * opacity + f32::from(destination.a) * (1.0 - opacity))
                        .round()
                        .clamp(0.0, 255.0) as u8,
                ),
            );
        }
    }
}

/// `CStdDDraw::DrawFrameDw`: four directed GL lines around an inclusive
/// rectangle. The endpoint exclusion of each line covers every corner once.
#[allow(clippy::too_many_arguments)]
pub fn draw_engine_frame(
    surface: &mut Surface,
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    color: u32,
    gamma: Option<&GammaRamp>,
) {
    draw_engine_line(surface, x1, y1, x2, y1, color, gamma);
    draw_engine_line(surface, x2, y1, x2, y2, color, gamma);
    draw_engine_line(surface, x2, y2, x1, y2, color, gamma);
    draw_engine_line(surface, x1, y2, x1, y1, color, gamma);
}

/// `CStdDDraw::DrawFrame` (StdDDraw2.cpp:1173-1179): both horizontal lines
/// run `x1->x2` and both verticals `y1->y2` (render targets route through
/// `DrawLine`), so the four GL lines share `(x2,y2)` only as an excluded
/// endpoint — the bottom-right corner is never rasterized — while `(x1,y1)`
/// starts two lines and rasterizes twice. C++ capture evidence: Drachenfels
/// object-menu extra divider (1032,647)-(1208,662), 2026-07-21 — three
/// corners painted (68,1,1), bottom-right left at the (1,1,1) background.
#[allow(clippy::too_many_arguments)]
pub fn draw_engine_frame_hv(
    surface: &mut Surface,
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    color: u32,
    gamma: Option<&GammaRamp>,
) {
    draw_engine_line(surface, x1, y1, x2, y1, color, gamma);
    draw_engine_line(surface, x1, y2, x2, y2, color, gamma);
    draw_engine_line(surface, x1, y1, x1, y2, color, gamma);
    draw_engine_line(surface, x2, y1, x2, y2, color, gamma);
}

/// Default `C4GUI::Element::Draw3DFrame`, preserving C++ draw order and the
/// GL line diamond-exit endpoint rule (`C4Gui.cpp:264-279`).
pub fn draw_3d_frame(surface: &mut Surface, rect: IntRect, gamma: Option<&GammaRamp>) {
    const ALPHA: u32 = 0xaf << 24;
    const COLOR1: u32 = 0x0077_2200;
    const COLOR2: u32 = 0x0033_1100;
    const COLOR3: u32 = 0x00aa_4400;
    let (x0, y0) = (rect.x, rect.y);
    let (x1, y1) = (rect.x + rect.w - 1, rect.y + rect.h - 1);
    [
        (x0, y0, x1, y0, COLOR1),
        (x0, y0, x0, y1, COLOR1),
        (x0 + 1, y0 + 1, x1 - 1, y0 + 1, COLOR2),
        (x0 + 1, y0 + 1, x0 + 1, y1 - 1, COLOR2),
        (x0, y1, x1, y1, COLOR3),
        (x1, y0, x1, y1, COLOR3),
        (x0 + 1, y1 - 1, x1 - 1, y1 - 1, COLOR1),
        (x1 - 1, y0 + 1, x1 - 1, y1 - 1, COLOR1),
    ]
    .into_iter()
    .for_each(|(ax, ay, bx, by, line_color)| {
        draw_engine_line(surface, ax, ay, bx, by, line_color | ALPHA, gamma);
    });
}

/// Draws text through a scratch surface so output is clipped exactly to the
/// C++ primary clipper (`StdDDraw2.cpp:583-600`). `clip` uses exclusive width
/// and height.
#[allow(clippy::too_many_arguments)]
pub fn draw_clipped_text(
    surface: &mut Surface,
    font: &ClonkFont,
    x: i32,
    y: i32,
    text: &str,
    color: [u8; 4],
    align: TextAlign,
    gamma: Option<&GammaRamp>,
    clip: IntRect,
) {
    draw_clipped_text_with_markup(surface, font, x, y, text, color, align, gamma, clip, true);
}

pub(crate) fn with_surface_clip(
    surface: &mut Surface,
    clip: IntRect,
    draw: impl FnOnce(&mut Surface),
) {
    let previous = surface.clip();
    let mut left = i64::from(clip.x).max(0);
    let mut top = i64::from(clip.y).max(0);
    let mut right = (i64::from(clip.x) + i64::from(clip.w.max(0)))
        .min(i64::from(surface.width().min(i32::MAX as u32)));
    let mut bottom = (i64::from(clip.y) + i64::from(clip.h.max(0)))
        .min(i64::from(surface.height().min(i32::MAX as u32)));
    if let Some(existing) = previous {
        left = left.max(i64::from(existing.x));
        top = top.max(i64::from(existing.y));
        right = right.min(i64::from(existing.x) + i64::from(existing.width));
        bottom = bottom.min(i64::from(existing.y) + i64::from(existing.height));
    }
    if left < right && top < bottom {
        surface.set_clip(clonk_graphics::Rect::new(
            left as i32,
            top as i32,
            (right - left) as u32,
            (bottom - top) as u32,
        ));
        draw(surface);
    }
    match previous {
        Some(existing) => surface.set_clip(existing),
        None => surface.clear_clip(),
    }
}

/// [`draw_clipped_text`] with the caller-selected `C4GUI::Label::fMarkup`
/// mode. Text windows such as the license viewer deliberately draw literal
/// markup separators even though their log buffer uses markup-aware wrapping.
#[allow(clippy::too_many_arguments)]
pub fn draw_clipped_text_with_markup(
    surface: &mut Surface,
    font: &ClonkFont,
    x: i32,
    y: i32,
    text: &str,
    color: [u8; 4],
    align: TextAlign,
    gamma: Option<&GammaRamp>,
    clip: IntRect,
    markup: bool,
) {
    let cx0 = clip.x.max(0);
    let cy0 = clip.y.max(0);
    let cx1 = (clip.x + clip.w).min(surface.width() as i32);
    let cy1 = (clip.y + clip.h).min(surface.height() as i32);
    if cx0 >= cx1 || cy0 >= cy1 {
        return;
    }
    let (width, height) = ((cx1 - cx0) as u32, (cy1 - cy0) as u32);
    if surface.is_clonk_text_capture_active() || surface.is_gpu_scene_capture_active() {
        // A private scratch Surface cannot share the semantic capture sink.
        // During native presentation, draw against the original surface with
        // the equivalent primary clipper so the recorded command retains its
        // absolute coordinates and clip rectangle. Tagged fonts suppress the
        // logical glyph pixels; untagged fonts still rasterize through the
        // ordinary clipped Surface operations.
        let saved_clip = surface.clip();
        let requested_clip = clonk_graphics::Rect::new(cx0, cy0, width, height);
        let draw_clip = match saved_clip {
            Some(saved_clip) => {
                let Some(draw_clip) = requested_clip.intersection(saved_clip) else {
                    return;
                };
                draw_clip
            }
            None => requested_clip,
        };
        surface.set_clip(draw_clip);
        font.draw_with_gamma(surface, x, y, text, color, align, markup, gamma);
        match saved_clip {
            Some(clip) => surface.set_clip(clip),
            None => surface.clear_clip(),
        }
        return;
    }
    let mut scratch = Surface::new(width, height, PixelFormat::Rgba8888);
    for target_y in 0..height {
        for target_x in 0..width {
            let source = surface
                .get_pixel(cx0 as u32 + target_x, cy0 as u32 + target_y)
                .unwrap_or_default();
            let _ = scratch.set_pixel(target_x, target_y, source);
        }
    }
    font.draw_with_gamma(
        &mut scratch,
        x - cx0,
        y - cy0,
        text,
        color,
        align,
        markup,
        gamma,
    );
    for target_y in 0..height {
        for target_x in 0..width {
            if let Some(pixel) = scratch.get_pixel(target_x, target_y) {
                let _ = surface.set_pixel(cx0 as u32 + target_x, cy0 as u32 + target_y, pixel);
            }
        }
    }
}

/// Clears RGB values hidden by zero alpha before bilinear filtering, avoiding
/// colored fringes while preserving every visible source pixel.
pub fn blacken_transparent_pixels(image: &ImageData) -> ImageData {
    thread_local! {
        static BLACKENED_IMAGES: RefCell<HashMap<clonk_graphics::GpuTextureId, ImageData>> =
            RefCell::new(HashMap::new());
    }
    BLACKENED_IMAGES.with(|images| {
        if let Some(image) = images.borrow().get(&image.gpu_texture_id()).cloned() {
            return image;
        }
        let needs_fixup = image
            .pixels()
            .chunks_exact(4)
            .any(|pixel| pixel[3] == 0 && pixel[..3] != [0, 0, 0]);
        let blackened = if needs_fixup {
            let pixels = image
                .pixels()
                .chunks_exact(4)
                .flat_map(|pixel| {
                    if pixel[3] == 0 {
                        [0, 0, 0, 0]
                    } else {
                        [pixel[0], pixel[1], pixel[2], pixel[3]]
                    }
                })
                .collect();
            ImageData::new(image.width(), image.height(), pixels)
        } else {
            image.clone()
        };
        images
            .borrow_mut()
            .insert(image.gpu_texture_id(), blackened.clone());
        blackened
    })
}

/// `CStdGL::DrawLineDw`: center-shifted line geometry with a half-open final
/// endpoint and the native line-specific packed-alpha conversion.
#[allow(clippy::too_many_arguments)]
pub fn draw_engine_line(
    surface: &mut Surface,
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    color: u32,
    gamma: Option<&GammaRamp>,
) {
    if surface.is_gpu_scene_capture_active() {
        let rgba = [
            ((color >> 16) & 0xff) as f32 / 255.0,
            ((color >> 8) & 0xff) as f32 / 255.0,
            (color & 0xff) as f32 / 255.0,
            (255 - ((color >> 24) & 0xff)) as f32 / 255.0,
        ];
        let vertex = |x: i32, y: i32| clonk_graphics::GpuSolidVertex {
            // Unlike DrawQuadDw, CStdGL::DrawLineDw never applies the device
            // `blitOffset`; it only shifts to the logical pixel center.
            position: [x as f32 + 0.5, y as f32 + 0.5, 1.0],
            color: rgba,
            outer_modulation: clonk_graphics::GpuSolidOuterModulation::PackedC4,
        };
        surface.push_gpu_command(clonk_graphics::GpuCommand::Solid {
            // GL_LINES remains a line even when both endpoints coincide.  In
            // that case the diamond-exit rule excludes the final endpoint and
            // produces no fragment; lowering it to GL_POINTS would invent one.
            vertices: vec![vertex(x1, y1), vertex(x2, y2)],
            topology: clonk_graphics::GpuPrimitiveTopology::LineList,
            alpha_mode: clonk_graphics::GpuSolidAlphaMode::SourceOver,
            clip: surface.clip(),
            blend: clonk_graphics::GpuBlend::Normal,
            style: clonk_graphics::GpuSolidStyle::with_gamma(
                gamma.is_some_and(|gamma| !gamma.is_passthrough()),
            ),
        });
        return;
    }

    if x1 == x2 && y1 == y2 {
        return;
    }

    if y1 == y2 && x2 > x1 {
        draw_engine_line_box(surface, x1, y1, x2 - 1, y1, color, gamma);
    } else if y1 == y2 && x2 < x1 {
        draw_engine_line_box(surface, x2 + 1, y1, x1, y1, color, gamma);
    } else if x1 == x2 && y2 > y1 {
        draw_engine_line_box(surface, x1, y1, x1, y2 - 1, color, gamma);
    } else if x1 == x2 && y2 < y1 {
        draw_engine_line_box(surface, x1, y2 + 1, x1, y1, color, gamma);
    } else {
        draw_engine_line_box(surface, x1, y1, x2, y2, color, gamma);
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_engine_line_box(
    surface: &mut Surface,
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    color: u32,
    gamma: Option<&GammaRamp>,
) {
    draw_engine_box(surface, x1, y1, x2, y2, color, gamma);
}

const fn inclusive_clip(rect: IntRect) -> IntRect {
    IntRect::new(rect.x, rect.y, rect.w + 1, rect.h + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::startup_main_menu::draw_bar as previous_draw_button_bar;
    use crate::test_support::{endeavour_font_set, load_graphics_png};

    fn column_coded_image(width: u32, height: u32) -> ImageData {
        let pixels = (0..height)
            .flat_map(|_| {
                (0..width).flat_map(|x| [(10 * (x + 1)) as u8; 3].into_iter().chain([255u8]))
            })
            .collect();
        ImageData::new(width, height, pixels)
    }

    fn column_values(surface: &Surface, y: u32, width: u32) -> Vec<u8> {
        (0..width)
            .map(|x| surface.get_pixel(x, y).map(|color| color.r).unwrap_or(0))
            .collect()
    }

    #[test]
    fn facet_sampler_uses_native_transparent_white_texture_padding() {
        // C4TexRef initializes the complete power-of-two allocation to 0xff
        // before C4Surface uploads only the image rows
        // (src/C4Surface.cpp:190-205,955-991,1075-1113).
        let image = ImageData::new(4, 3, [10, 20, 30, 255].repeat(4 * 3));

        let sample = bilinear_sample_tile(&image, 0, 0, 4, 0.0, 3.0);

        assert_eq!(sample, [255.0, 255.0, 255.0, 0.0]);
    }

    // `DynBarFacet::SetHorizontal` + exact `Element::DrawBar`: begin, tiled
    // middle and right-aligned end (`C4Gui.cpp:92-99,283-311`).
    #[test]
    fn bar_tiles_middle_and_right_aligns_end() {
        let image = column_coded_image(8, 1);
        let mut surface = Surface::new(9, 1, PixelFormat::Rgba8888);
        draw_bar(&mut surface, IntRect::new(0, 0, 9, 1), &image, 2, None);
        assert_eq!(
            column_values(&surface, 0, 9),
            vec![10, 20, 30, 40, 50, 60, 30, 70, 80]
        );
    }

    // A bar narrower than its border crops the end from the left and lets it
    // overdraw the begin (`C4Gui.cpp:289-310`).
    #[test]
    fn narrow_bar_lets_end_overdraw_begin() {
        let image = column_coded_image(8, 1);
        let mut surface = Surface::new(1, 1, PixelFormat::Rgba8888);
        draw_bar(&mut surface, IntRect::new(0, 0, 1, 1), &image, 2, None);
        assert_eq!(column_values(&surface, 0, 1), vec![80]);
    }

    // GameOver's ComponentAligner yields 44px-tall buttons while GUIButton is
    // 32px high. DrawBar must take its zoomed branch and cover the last row
    // (`C4GameOverDlg.cpp:146-157,232-258`; `C4Gui.cpp:313-329`).
    #[test]
    fn zoomed_button_bar_covers_full_game_over_height() {
        let image = ImageData::new(96, 32, [0, 120, 0, 255].repeat(96 * 32));
        let background = Color::opaque(11, 22, 33);
        let mut surface = Surface::new(120, 50, PixelFormat::Rgba8888);
        surface.fill(background);
        draw_bar(&mut surface, IntRect::new(4, 3, 100, 44), &image, 32, None);

        assert_eq!(surface.get_pixel(50, 46), Some(Color::opaque(0, 120, 0)));
    }

    // `DrawBoxDw` covers inclusive corners and treats engine alpha as
    // transparency (`StdDDraw2.cpp:1401-1404`, `StdGL.cpp:846-891`).
    #[test]
    fn engine_box_blends_inverted_alpha_over_inclusive_rect() {
        let mut surface = Surface::new(3, 1, PixelFormat::Rgba8888);
        for x in 0..3 {
            let _ = surface.set_pixel(x, 0, Color::new(200, 100, 0, 255));
        }
        draw_engine_box(&mut surface, 0, 0, 1, 0, 0x7f00_0000, None);
        assert_eq!(column_values(&surface, 0, 3), vec![100, 100, 200]);
        assert_eq!(surface.get_pixel(0, 0).map(|color| color.g), Some(50));
    }

    #[test]
    fn engine_box_keeps_transparent_layer_premultiplied() {
        let mut surface = Surface::new(1, 1, PixelFormat::Rgba8888);
        draw_engine_box(&mut surface, 0, 0, 0, 0, 0x7f00_ff00, None);
        assert_eq!(surface.get_pixel(0, 0), Some(Color::new(0, 128, 0, 128)));
    }

    #[test]
    fn retained_engine_box_stays_alpha_over_after_prior_replace() {
        let mut surface = Surface::new(3, 2, PixelFormat::Rgba8888);
        surface.begin_gpu_scene_capture();
        surface.set_pixel(0, 0, Color::opaque(1, 2, 3)).unwrap();
        draw_engine_box(&mut surface, 0, 0, 1, 1, 0x7f20_4060, None);

        let scene = surface
            .take_gpu_scene_capture()
            .expect("capture remains active")
            .into_scene([3, 2], Color::transparent(), &GammaRamp::identity());
        assert_eq!(scene.commands.len(), 2);
        let clonk_graphics::GpuCommand::Solid { blend, .. } = &scene.commands[0] else {
            panic!("prior pixel did not remain a painter command");
        };
        assert_eq!(*blend, clonk_graphics::GpuBlend::Replace);
        let clonk_graphics::GpuCommand::Solid {
            blend, alpha_mode, ..
        } = &scene.commands[1]
        else {
            panic!("engine box did not remain a solid painter command");
        };
        assert_eq!(*blend, clonk_graphics::GpuBlend::Normal);
        assert_eq!(
            *alpha_mode,
            clonk_graphics::GpuSolidAlphaMode::SourceOver,
            "DrawBoxDw retains packed C4 alpha provenance"
        );
    }

    #[test]
    fn retained_engine_frame_keeps_draw_line_alpha_provenance() {
        let mut surface = Surface::new(8, 8, PixelFormat::Rgba8888);
        surface.begin_gpu_scene_capture();
        draw_engine_frame(&mut surface, 1, 1, 6, 6, 0x7f20_4060, None);

        let scene = surface
            .take_gpu_scene_capture()
            .expect("capture remains active")
            .into_scene([8, 8], Color::transparent(), &GammaRamp::identity());
        assert!(!scene.commands.is_empty());
        for command in &scene.commands {
            let clonk_graphics::GpuCommand::Solid {
                topology,
                alpha_mode,
                ..
            } = command
            else {
                panic!("engine frame did not remain solid painter commands");
            };
            assert_eq!(*topology, clonk_graphics::GpuPrimitiveTopology::LineList);
            assert_eq!(
                *alpha_mode,
                clonk_graphics::GpuSolidAlphaMode::SourceOver,
                "DrawFrameDw is composed from DrawLineDw segments"
            );
        }
    }

    // `CStdDDraw::DrawFrame` (StdDDraw2.cpp:1173-1179) on a render target:
    // two horizontals `x1->x2`, two verticals `y1->y2`. Their excluded GL
    // endpoints never rasterize the shared bottom-right corner while the
    // shared origin rasterizes twice. Capture oracle
    // (2026-07-21, Drachenfels divider (1032,647)-(1208,662)): bottom-right
    // (1208,662) stayed background while the other corners painted.
    #[test]
    fn hv_engine_frame_skips_shared_end_corner_and_double_covers_origin() {
        let background = Color::opaque(100, 100, 100);
        let mut cpu = Surface::new(8, 8, PixelFormat::Rgba8888);
        cpu.fill(background);
        // engine alpha 0x7f -> GL source alpha 128/255; one blend over 100
        // stores round(100*127/255) = 50, a second blend stores 25.
        draw_engine_frame_hv(&mut cpu, 1, 1, 6, 6, 0x7f00_0000, None);
        let once = Some(Color::new(50, 50, 50, 255));
        let twice = Some(Color::new(25, 25, 25, 255));
        assert_eq!(
            cpu.get_pixel(1, 1),
            twice,
            "both first lines start at the origin"
        );
        assert_eq!(cpu.get_pixel(6, 1), once);
        assert_eq!(cpu.get_pixel(1, 6), once);
        assert_eq!(
            cpu.get_pixel(6, 6),
            Some(background),
            "every line excludes the shared bottom-right endpoint"
        );
        for v in 2..6 {
            assert_eq!(cpu.get_pixel(v, 1), once);
            assert_eq!(cpu.get_pixel(v, 6), once);
            assert_eq!(cpu.get_pixel(1, v), once);
            assert_eq!(cpu.get_pixel(6, v), once);
        }

        let mut retained = Surface::new(8, 8, PixelFormat::Rgba8888);
        retained.begin_gpu_scene_capture();
        draw_engine_frame_hv(&mut retained, 1, 1, 6, 6, 0x7f00_0000, None);
        let scene = retained
            .take_gpu_scene_capture()
            .expect("capture remains active")
            .into_scene([8, 8], Color::transparent(), &GammaRamp::identity());
        // Consecutive compatible LineList commands may be coalesced; the
        // segment list in submission order is the invariant.
        let segments = scene
            .commands
            .iter()
            .flat_map(|command| {
                let clonk_graphics::GpuCommand::Solid {
                    vertices, topology, ..
                } = command
                else {
                    panic!("DrawFrame did not remain solid painter commands");
                };
                assert_eq!(*topology, clonk_graphics::GpuPrimitiveTopology::LineList);
                assert_eq!(vertices.len() % 2, 0);
                vertices
                    .chunks_exact(2)
                    .map(|pair| (pair[0].position, pair[1].position))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            segments,
            vec![
                ([1.5, 1.5, 1.0], [6.5, 1.5, 1.0]),
                ([1.5, 6.5, 1.0], [6.5, 6.5, 1.0]),
                ([1.5, 1.5, 1.0], [1.5, 6.5, 1.0]),
                ([6.5, 1.5, 1.0], [6.5, 6.5, 1.0]),
            ],
            "retained capture records the native DrawFrame segments in call order"
        );
    }

    #[test]
    fn zero_length_engine_line_stays_a_fragmentless_gl_line() {
        let background = Color::opaque(9, 8, 7);
        let mut cpu = Surface::new(4, 4, PixelFormat::Rgba8888);
        cpu.fill(background);
        draw_engine_line(&mut cpu, 2, 1, 2, 1, 0x0020_4060, None);
        assert_eq!(cpu.get_pixel(2, 1), Some(background));

        let mut retained = Surface::new(4, 4, PixelFormat::Rgba8888);
        retained.begin_gpu_scene_capture();
        draw_engine_line(&mut retained, 2, 1, 2, 1, 0x7f20_4060, None);
        let scene = retained
            .take_gpu_scene_capture()
            .expect("capture remains active")
            .into_scene([4, 4], Color::transparent(), &GammaRamp::identity());
        let [clonk_graphics::GpuCommand::Solid {
            vertices,
            topology,
            alpha_mode,
            ..
        }] = scene.commands.as_slice()
        else {
            panic!("zero-length DrawLineDw did not remain one line primitive");
        };
        assert_eq!(*topology, clonk_graphics::GpuPrimitiveTopology::LineList);
        assert_eq!(*alpha_mode, clonk_graphics::GpuSolidAlphaMode::SourceOver);
        assert_eq!(vertices.len(), 2);
        assert_eq!(vertices[0].position, vertices[1].position);
    }

    #[test]
    fn retained_engine_line_ignores_quad_only_blit_offset() {
        let _renderer = crate::activate_advanced_renderer_config(crate::AdvancedRendererConfig {
            blit_offset: 100,
            ..crate::AdvancedRendererConfig::DEFAULT
        });
        let mut surface = Surface::new(8, 6, PixelFormat::Rgba8888);
        surface.begin_gpu_scene_capture();
        draw_engine_line(&mut surface, 1, 2, 5, 2, 0x0020_4060, None);
        let scene = surface
            .take_gpu_scene_capture()
            .expect("capture remains active")
            .into_scene([8, 6], Color::transparent(), &GammaRamp::identity());
        let [clonk_graphics::GpuCommand::Solid { vertices, .. }] = scene.commands.as_slice() else {
            panic!("DrawLineDw did not remain one line command");
        };
        assert_eq!(vertices[0].position, [1.5, 2.5, 1.0]);
        assert_eq!(vertices[1].position, [5.5, 2.5, 1.0]);
    }

    #[test]
    fn retained_nearest_facet_is_one_ordered_textured_command() {
        let image = ImageData::new(2, 1, vec![10, 20, 30, 255, 40, 50, 60, 128]);
        let mut surface = Surface::new(5, 3, PixelFormat::Rgba8888);
        surface.begin_gpu_scene_capture();
        surface.set_pixel(0, 0, Color::opaque(1, 2, 3)).unwrap();
        draw_facet_nearest(
            &mut surface,
            &image,
            clonk_graphics::Rect::new(0, 0, 2, 1),
            clonk_graphics::Rect::new(1, 1, 4, 2),
            None,
        );

        let scene = surface
            .take_gpu_scene_capture()
            .expect("capture remains active")
            .into_scene([5, 3], Color::transparent(), &GammaRamp::identity());
        assert_eq!(scene.commands.len(), 2);
        // clonk-org/clonk-rs#271: an eligible non-object sprite is a compact
        // instance rather than a generic quad. Still one ordered textured
        // command in painter order — the record it carries is smaller.
        let clonk_graphics::GpuCommand::ObjectBatch {
            sprites,
            blend,
            gamma,
            ..
        } = &scene.commands[1]
        else {
            panic!("nearest facet did not remain a textured painter command");
        };
        assert_eq!(sprites.len(), 1);
        assert_eq!(*blend, clonk_graphics::GpuBlend::Normal);
        assert_eq!(sprites[0].sampler(), clonk_graphics::GpuSampler::Nearest);
        assert!(!*gamma);
    }

    #[test]
    fn nearest_facet_preserves_legacy_cpu_truncation() {
        let image = ImageData::new(1, 1, vec![1, 3, 5, 128]);
        let mut surface = Surface::new(1, 1, PixelFormat::Rgba8888);
        draw_facet_nearest(
            &mut surface,
            &image,
            clonk_graphics::Rect::new(0, 0, 1, 1),
            clonk_graphics::Rect::new(0, 0, 1, 1),
            None,
        );
        assert_eq!(surface.get_pixel(0, 0), Some(Color::new(0, 1, 2, 128)));
    }

    #[test]
    fn engine_frame_blends_each_corner_exactly_once() {
        let background = Color::opaque(200, 100, 0);
        let mut surface = Surface::new(5, 5, PixelFormat::Rgba8888);
        surface.fill(background);
        draw_engine_frame(&mut surface, 1, 1, 3, 3, 0x7f00_0000, None);

        let mut once = Surface::new(1, 1, PixelFormat::Rgba8888);
        once.fill(background);
        draw_engine_box(&mut once, 0, 0, 0, 0, 0x7f00_0000, None);
        let expected = once.get_pixel(0, 0);
        for (x, y) in [(1, 1), (3, 1), (3, 3), (1, 3)] {
            assert_eq!(surface.get_pixel(x, y), expected);
        }
        assert_eq!(surface.get_pixel(2, 2), Some(background));
    }

    // `DrawLineDw` omits its final pixel under GL's diamond-exit rule; the
    // ordered eight-line frame therefore has the C++ corner coverage
    // (`C4Gui.cpp:264-279`, `StdGL.cpp:893-934`).
    #[test]
    fn frame_preserves_line_endpoint_and_corner_coverage() {
        let mut surface = Surface::new(8, 8, PixelFormat::Rgba8888);
        surface.fill(Color::opaque(200, 200, 200));
        draw_3d_frame(&mut surface, IntRect::new(1, 1, 6, 6), None);

        assert_ne!(surface.get_pixel(1, 1), surface.get_pixel(6, 1));
        assert_ne!(surface.get_pixel(1, 6), surface.get_pixel(6, 6));
        assert_eq!(surface.get_pixel(3, 3), Some(Color::opaque(200, 200, 200)));
    }

    #[test]
    fn clipped_text_mode_can_draw_pipe_as_a_literal_character() {
        let fonts = endeavour_font_set();
        let font = &fonts.text;
        let height = (font.line_height * 2).max(1) as u32;
        let clip = IntRect::new(0, 0, 160, height as i32);
        let mut literal = Surface::new(160, height, PixelFormat::Rgba8888);
        draw_clipped_text_with_markup(
            &mut literal,
            font,
            0,
            0,
            "A|B",
            [255, 255, 255, 255],
            TextAlign::Left,
            None,
            clip,
            false,
        );
        let mut markup = Surface::new(160, height, PixelFormat::Rgba8888);
        draw_clipped_text_with_markup(
            &mut markup,
            font,
            0,
            0,
            "A|B",
            [255, 255, 255, 255],
            TextAlign::Left,
            None,
            clip,
            true,
        );

        let continuation_has_ink = |surface: &Surface| {
            (font.line_height.max(0) as u32..height).any(|y| {
                (0..surface.width())
                    .any(|x| surface.get_pixel(x, y).is_some_and(|pixel| pixel.a != 0))
            })
        };
        assert!(!continuation_has_ink(&literal));
        assert!(continuation_has_ink(&markup));
    }

    #[test]
    fn clipped_text_capture_keeps_absolute_coordinates_and_clipper() {
        let fonts = endeavour_font_set();
        let mut surface = Surface::new(80, 48, PixelFormat::Rgba8888);
        surface.begin_clonk_text_capture();
        draw_clipped_text_with_markup(
            &mut surface,
            &fonts.text,
            17,
            19,
            "capture",
            [255, 255, 255, 255],
            TextAlign::Left,
            None,
            IntRect::new(11, 13, 31, 23),
            true,
        );

        let commands = surface.take_clonk_text_capture();
        assert_eq!(commands.len(), 1);
        assert_eq!((commands[0].x, commands[0].y), (17, 19));
        assert_eq!(
            commands[0].clip,
            Some(clonk_graphics::Rect::new(11, 13, 31, 23))
        );
        assert!(surface.pixels().chunks_exact(4).all(|pixel| pixel[3] == 0));
    }

    // Structural equivalence guard for the extracted `Button::DrawElement`
    // body (`C4GuiButton.cpp:81-109`): compare against the prior net-dialog
    // composition for the most stateful pressed+focused case.
    #[test]
    fn button_matches_pre_extraction_composition() {
        let button = load_graphics_png("GUIButton.png");
        let button_down = load_graphics_png("GUIButtonDown.png");
        let raw_highlight = load_graphics_png("GUIButtonHighlight.png");
        let highlight = blacken_transparent_pixels(&raw_highlight);
        assert_eq!(
            highlight.gpu_texture_id(),
            blacken_transparent_pixels(&raw_highlight).gpu_texture_id()
        );
        let caption = load_graphics_png("GUICaption.png");
        let fonts = endeavour_font_set();
        let rect = IntRect::new(11, 9, 153, 32);
        let mut previous = Surface::new(176, 52, PixelFormat::Rgba8888);
        let mut extracted = Surface::new(176, 52, PixelFormat::Rgba8888);
        previous.fill(Color::opaque(32, 64, 96));
        extracted.fill(Color::opaque(32, 64, 96));

        previous_draw_button_bar(
            &mut previous,
            &GuiRect::new(rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32),
            &button_down,
            None,
        );
        crate::draw_image_bilinear_additive(
            &mut previous,
            &GuiRect::new(
                (rect.x + 5) as f32,
                (rect.y + 3) as f32,
                (rect.w - 10) as f32,
                (rect.h - 6) as f32,
            ),
            &highlight,
            None,
        );
        let font = fonts.button_font(rect.h);
        let (text, _) = expand_hotkey_markup("&Join game");
        font.draw_with_gamma(
            &mut previous,
            (rect.x + rect.x + rect.w - 1) / 2 + 1,
            (rect.y + rect.y + rect.h - 1 - font.line_height) / 2 + 1,
            &text,
            [0xff, 0xff, 0x00, 0xff],
            TextAlign::Center,
            true,
            None,
        );

        ClassicGuiSkin::new(&caption, &button, &button_down, Some(&highlight)).draw_button(
            &mut extracted,
            rect,
            "&Join game",
            &fonts,
            ClassicButtonState {
                pressed: true,
                highlighted: true,
            },
            None,
        );

        assert_eq!(extracted.pixels(), previous.pixels());
    }
}
