//! Reusable pixel-faithful rendering primitives for the classic `C4GUI`
//! furniture (`src/C4Gui*.cpp`).

use crate::clonk_fonts::{expand_hotkey_markup, ClonkFontSet};
pub use crate::startup_main_menu::IntRect;
use crate::ImageData;
use lc_graphics::clonk_font::{ClonkFont, TextAlign};
use lc_graphics::{Color, GammaRamp, PixelFormat, Surface};
use lc_gui::Rect as GuiRect;

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
        draw_bar(surface, rect, self.caption, 32, gamma);
        let x = match align {
            TextAlign::Left => rect.x + 5,
            TextAlign::Center => rect.x + rect.w / 2,
            TextAlign::Right => rect.x + rect.w,
        };
        draw_clipped_text(
            surface,
            font,
            x,
            rect.y + (rect.h - font.line_height) / 2 - 1,
            text,
            color,
            align,
            gamma,
            inclusive_clip(rect),
        );
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
        let plank = if state.pressed {
            self.button_down
        } else {
            self.button
        };
        draw_bar(surface, rect, plank, plank.height(), gamma);
        if state.highlighted {
            if let Some(highlight) = self.button_highlight {
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
fn draw_facet_stretch(
    surface: &mut Surface,
    image: &ImageData,
    source: (f32, f32, f32, f32),
    destination: (f32, f32, f32, f32),
    gamma: Option<&GammaRamp>,
) {
    let (source_x, source_y, source_width, source_height) = source;
    let (target_x, target_y, target_width, target_height) = destination;
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
                            255,
                        ),
                    );
                }
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
            return [0.0; 4];
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

fn encode_filtered_channel(gamma: Option<&GammaRamp>, channel: f32) -> f32 {
    gamma
        .map(|ramp| f32::from(ramp.encode_float(channel)))
        .unwrap_or_else(|| channel.round().clamp(0.0, 255.0))
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
                    255,
                ),
            );
        }
    }
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
    let cx0 = clip.x.max(0);
    let cy0 = clip.y.max(0);
    let cx1 = (clip.x + clip.w).min(surface.width() as i32);
    let cy1 = (clip.y + clip.h).min(surface.height() as i32);
    if cx0 >= cx1 || cy0 >= cy1 {
        return;
    }
    let (width, height) = ((cx1 - cx0) as u32, (cy1 - cy0) as u32);
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
        true,
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
}

fn draw_engine_line(
    surface: &mut Surface,
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    color: u32,
    gamma: Option<&GammaRamp>,
) {
    if y1 == y2 && x2 > x1 {
        draw_engine_box(surface, x1, y1, x2 - 1, y2, color, gamma);
    } else if x1 == x2 && y2 > y1 {
        draw_engine_box(surface, x1, y1, x2, y2 - 1, color, gamma);
    } else {
        draw_engine_box(surface, x1, y1, x2, y2, color, gamma);
    }
}

const fn inclusive_clip(rect: IntRect) -> IntRect {
    IntRect {
        x: rect.x,
        y: rect.y,
        w: rect.w + 1,
        h: rect.h + 1,
    }
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

    // `DynBarFacet::SetHorizontal` + exact `Element::DrawBar`: begin, tiled
    // middle and right-aligned end (`C4Gui.cpp:92-99,283-311`).
    #[test]
    fn bar_tiles_middle_and_right_aligns_end() {
        let image = column_coded_image(8, 1);
        let mut surface = Surface::new(9, 1, PixelFormat::Rgba8888);
        draw_bar(
            &mut surface,
            IntRect {
                x: 0,
                y: 0,
                w: 9,
                h: 1,
            },
            &image,
            2,
            None,
        );
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
        draw_bar(
            &mut surface,
            IntRect {
                x: 0,
                y: 0,
                w: 1,
                h: 1,
            },
            &image,
            2,
            None,
        );
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
        draw_bar(
            &mut surface,
            IntRect {
                x: 4,
                y: 3,
                w: 100,
                h: 44,
            },
            &image,
            32,
            None,
        );

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

    // `DrawLineDw` omits its final pixel under GL's diamond-exit rule; the
    // ordered eight-line frame therefore has the C++ corner coverage
    // (`C4Gui.cpp:264-279`, `StdGL.cpp:893-934`).
    #[test]
    fn frame_preserves_line_endpoint_and_corner_coverage() {
        let mut surface = Surface::new(8, 8, PixelFormat::Rgba8888);
        surface.fill(Color::opaque(200, 200, 200));
        draw_3d_frame(
            &mut surface,
            IntRect {
                x: 1,
                y: 1,
                w: 6,
                h: 6,
            },
            None,
        );

        assert_ne!(surface.get_pixel(1, 1), surface.get_pixel(6, 1));
        assert_ne!(surface.get_pixel(1, 6), surface.get_pixel(6, 6));
        assert_eq!(surface.get_pixel(3, 3), Some(Color::opaque(200, 200, 200)));
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
        let caption = load_graphics_png("GUICaption.png");
        let fonts = endeavour_font_set();
        let rect = IntRect {
            x: 11,
            y: 9,
            w: 153,
            h: 32,
        };
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
