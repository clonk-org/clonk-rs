//! Exact C4GameMessage drawing primitives.
//!
//! C++ draws the message list once inside every viewport, after that
//! viewport's menu and before its mouse overlay (`C4Viewport::DrawOverlay`).
//! This module owns only the message pixels; the app retains viewport
//! visibility/FoW classification and resource resolution.

use crate::object_menu::draw_menu_decoration;
use lc_engine::{
    MessageKind, MessageSnapshot, ObjectMenuFrameDecoration, FLAG_ALIGN_CENTER, FLAG_ALIGN_LEFT,
    FLAG_ALIGN_RIGHT, FLAG_BOTTOM, FLAG_HCENTER, FLAG_NO_BREAK, FLAG_RIGHT, FLAG_VCENTER,
    FLAG_WIDTH_REL, FLAG_X_REL, FLAG_Y_REL,
};
use lc_frontend::clonk_fonts::NativeClonkFont;
use lc_frontend::ImageData;
use lc_graphics::clonk_font::{ClonkFont, FontImageProvider, TextAlign};
use lc_graphics::{ClipperProjection, GammaRamp, Rect, Surface};
use lc_gui::Rect as GuiRect;

const DRAW_MESSAGE_OFFSET: i32 = -35;
const PORTRAIT_WIDTH: i32 = 64;
const PORTRAIT_INDENT: i32 = 10;

enum MessageFontMetrics<'font> {
    Logical(&'font ClonkFont),
    Native(&'font NativeClonkFont),
}

impl MessageFontMetrics<'_> {
    fn measure(&self, text: &str, images: &dyn FontImageProvider) -> (i32, i32) {
        match self {
            Self::Logical(font) => font.measure_with_images(text, true, images),
            Self::Native(font) => font.measure_with_images(text, true, images),
        }
    }

    fn break_message(&self, text: &str, width: i32, images: &dyn FontImageProvider) -> String {
        match self {
            Self::Logical(font) => {
                lc_frontend::message_dialog::break_message_with_images(font, text, width, images)
            }
            Self::Native(font) => lc_frontend::message_dialog::break_native_message_with_images(
                font, text, width, images,
            ),
        }
    }

    fn break_line_height(&self) -> i32 {
        match self {
            Self::Logical(font) => font.line_height,
            Self::Native(font) => font.logical_line_height(),
        }
    }
}

struct GlobalMessageLayout {
    text: String,
    color: [u8; 4],
    alignment: TextAlign,
    frame: Option<Rect>,
    portrait: Option<GuiRect>,
    text_x: i32,
    text_y: i32,
}

pub(crate) fn is_supported(message: &MessageSnapshot) -> bool {
    matches!(
        message.kind,
        MessageKind::Global | MessageKind::GlobalPlayer
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_global_message(
    surface: &mut Surface,
    font: &ClonkFont,
    viewport: Rect,
    message: &MessageSnapshot,
    decoration: Option<&ObjectMenuFrameDecoration>,
    decoration_image: Option<&ImageData>,
    portrait: Option<&ImageData>,
    images: &dyn FontImageProvider,
    gamma: Option<&GammaRamp>,
) -> Result<(), &'static str> {
    let layout = layout_global_message(
        MessageFontMetrics::Logical(font),
        viewport,
        message,
        decoration,
        images,
    )?;
    draw_logical_layout(
        surface,
        font,
        viewport,
        &layout,
        decoration,
        decoration_image,
        portrait,
        true,
        images,
        gamma,
    );
    Ok(())
}

/// Commit one complete scale-native message to the already-presented physical
/// framebuffer. Keeping frame, portrait and glyphs in this one pass preserves
/// C++ insertion order when messages overlap. Coordinates remain GUI logical;
/// each viewport uses C++'s independently rounded GL clipper projection.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_global_message_native(
    surface: &mut Surface,
    native_font: &NativeClonkFont,
    scale: f32,
    logical_target_size: (u32, u32),
    viewport: Rect,
    message: &MessageSnapshot,
    decoration: Option<&ObjectMenuFrameDecoration>,
    decoration_image: Option<&ImageData>,
    portrait: Option<&ImageData>,
    images: &dyn FontImageProvider,
    gamma: Option<&GammaRamp>,
) -> Result<(), &'static str> {
    let layout = layout_global_message(
        MessageFontMetrics::Native(native_font),
        viewport,
        message,
        decoration,
        images,
    )?;
    let previous_clip = surface.clip();
    let projection = ClipperProjection::new(scale, logical_target_size, surface.height(), viewport);
    set_nested_clip(surface, previous_clip, projection.physical_clip());
    if let (Some(frame), Some(decoration)) = (layout.frame, decoration) {
        draw_native_decoration(
            surface,
            frame,
            projection,
            decoration,
            decoration_image,
            gamma,
        );
    }
    if let (Some(rect), Some(portrait)) = (layout.portrait.as_ref(), portrait) {
        let rect = projected_gui_rect(rect, projection);
        lc_frontend::draw_image_bilinear(surface, &rect, portrait, gamma);
    }
    native_font.draw_to_physical_surface_with_clipper_and_images(
        surface,
        layout.text_x,
        layout.text_y,
        &layout.text,
        layout.color,
        layout.alignment,
        true,
        projection,
        gamma,
        images,
    );
    restore_clip(surface, previous_clip);
    Ok(())
}

fn layout_global_message(
    metrics: MessageFontMetrics<'_>,
    viewport: Rect,
    message: &MessageSnapshot,
    decoration: Option<&ObjectMenuFrameDecoration>,
    images: &dyn FontImageProvider,
) -> Result<GlobalMessageLayout, &'static str> {
    if !matches!(
        message.kind,
        MessageKind::Global | MessageKind::GlobalPlayer
    ) {
        return Err("target C4GameMessage requires the target renderer");
    }
    let text = message_text(message);

    let viewport_width = extent_i32(viewport.width);
    let viewport_height = extent_i32(viewport.height);
    let mut width = message.width.unwrap_or(0);
    let x = if message.flags & FLAG_X_REL != 0 {
        percent(message.offset.x, viewport_width)
    } else {
        message.offset.x
    };
    let y = if message.flags & FLAG_Y_REL != 0 {
        percent(message.offset.y, viewport_height)
    } else {
        message.offset.y
    };
    if message.flags & FLAG_WIDTH_REL != 0 {
        width = percent(width, viewport_width);
    }

    let portrait_requested = message
        .portrait
        .as_deref()
        .is_some_and(|spec| !spec.is_empty());
    let (draw_text, text_width, text_height) = if message.flags & FLAG_NO_BREAK != 0 {
        let extent = metrics.measure(&text, images);
        (text, extent.0, extent.1)
    } else {
        if portrait_requested {
            if width == 0 {
                width = bound_by(viewport_width / 2, 50, 500.min(viewport_width - 10));
            }
            width = width.min(metrics.measure(&text, images).0.saturating_add(10));
        } else if width == 0 {
            width = bound_by(viewport_width - 50, 50, 500);
        } else {
            width = bound_by(width, 10, viewport_width - 10);
        }
        let broken = metrics.break_message(&text, width, images);
        let line_count = i32::try_from(broken.split(['\n', '|']).count()).unwrap_or(i32::MAX);
        let height = metrics.break_line_height().saturating_mul(line_count);
        (broken, width, height)
    };

    let color = [
        ((message.color >> 16) & 0xff) as u8,
        ((message.color >> 8) & 0xff) as u8,
        (message.color & 0xff) as u8,
        ((message.color >> 24) & 0xff) as u8,
    ];
    let alignment = text_alignment(message.flags, portrait_requested);
    let mut draw_x = viewport.x.saturating_add(x);
    let mut draw_y = viewport.y.saturating_add(y);
    let mut frame = None;
    let mut portrait = None;
    if portrait_requested {
        if message.flags & FLAG_BOTTOM != 0 {
            draw_y = draw_y.saturating_add(viewport_height);
        } else if message.flags & FLAG_VCENTER != 0 {
            draw_y = draw_y.saturating_add(viewport_height / 2);
        }
        if message.flags & FLAG_RIGHT != 0 {
            draw_x = draw_x.saturating_add(viewport_width);
        } else if message.flags & FLAG_HCENTER != 0 {
            draw_x = draw_x.saturating_add(viewport_width / 2);
        }

        if let Some(decoration) = decoration {
            let frame_width = text_width
                .saturating_add(PORTRAIT_WIDTH)
                .saturating_add(PORTRAIT_INDENT)
                .saturating_add(decoration.border_left)
                .saturating_add(decoration.border_right);
            let frame_height = text_height
                .max(PORTRAIT_WIDTH)
                .saturating_add(decoration.border_top)
                .saturating_add(decoration.border_bottom);
            if message.flags & FLAG_BOTTOM != 0 {
                draw_y = draw_y.saturating_sub(frame_height);
            } else if message.flags & FLAG_VCENTER != 0 {
                draw_y = draw_y.saturating_sub(frame_height / 2);
            }
            if message.flags & FLAG_RIGHT != 0 {
                draw_x = draw_x.saturating_sub(frame_width);
            } else if message.flags & FLAG_HCENTER != 0 {
                draw_x = draw_x.saturating_sub(frame_width / 2);
            }
            if frame_width > 0 && frame_height > 0 {
                frame = Some(Rect::new(
                    draw_x,
                    draw_y,
                    frame_width as u32,
                    frame_height as u32,
                ));
            }
            draw_x = draw_x.saturating_add(decoration.border_left);
            draw_y = draw_y.saturating_add(decoration.border_top);
        } else {
            draw_y = draw_y.saturating_sub(text_height);
        }

        portrait = Some(GuiRect::new(
            draw_x as f32,
            draw_y as f32,
            PORTRAIT_WIDTH as f32,
            PORTRAIT_WIDTH as f32,
        ));
        draw_x = draw_x
            .saturating_add(PORTRAIT_WIDTH)
            .saturating_add(PORTRAIT_INDENT);
    } else {
        draw_x = draw_x.saturating_add(viewport_width / 2);
        draw_y = draw_y
            .saturating_add(2 * viewport_height / 3)
            .saturating_add(50);
        if message.flags & FLAG_BOTTOM == 0 {
            draw_y = draw_y.saturating_add(DRAW_MESSAGE_OFFSET);
        }
    }

    Ok(GlobalMessageLayout {
        text: draw_text,
        color,
        alignment,
        frame,
        portrait,
        text_x: draw_x,
        text_y: draw_y,
    })
}

#[allow(clippy::too_many_arguments)]
fn draw_logical_layout(
    surface: &mut Surface,
    font: &ClonkFont,
    viewport: Rect,
    layout: &GlobalMessageLayout,
    decoration: Option<&ObjectMenuFrameDecoration>,
    decoration_image: Option<&ImageData>,
    portrait: Option<&ImageData>,
    draw_text: bool,
    images: &dyn FontImageProvider,
    gamma: Option<&GammaRamp>,
) {
    let previous_clip = surface.clip();
    set_nested_clip(surface, previous_clip, viewport);
    if let (Some(frame), Some(decoration)) = (layout.frame, decoration) {
        draw_menu_decoration(surface, frame, decoration, decoration_image, gamma);
    }
    if let (Some(rect), Some(portrait)) = (layout.portrait.as_ref(), portrait) {
        // C4Facet::Draw uses the ordinary non-exact textured blit. With the
        // default PointFiltering=false this is GL_LINEAR, including 150x150
        // tutorial portraits reduced to the fixed 64x64 message square.
        lc_frontend::draw_image_bilinear(surface, rect, portrait, gamma);
    }
    if draw_text {
        font.draw_with_gamma_and_images(
            surface,
            layout.text_x,
            layout.text_y,
            &layout.text,
            layout.color,
            layout.alignment,
            true,
            gamma,
            images,
        );
    }
    restore_clip(surface, previous_clip);
}

fn set_nested_clip(surface: &mut Surface, previous: Option<Rect>, requested: Rect) {
    let clip = previous
        .and_then(|active| active.intersection(requested))
        .or_else(|| previous.is_none().then_some(requested))
        .unwrap_or_else(|| Rect::new(0, 0, 0, 0));
    surface.set_clip(clip);
}

fn restore_clip(surface: &mut Surface, previous: Option<Rect>) {
    if let Some(clip) = previous {
        surface.set_clip(clip);
    } else {
        surface.clear_clip();
    }
}

fn projected_rect(rect: Rect, projection: ClipperProjection) -> Rect {
    let clamp = |value: f64| value.clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
    let (left, top) = projection.logical_to_physical(f64::from(rect.x), f64::from(rect.y));
    let (right, bottom) = projection.logical_to_physical(
        (i64::from(rect.x) + i64::from(rect.width)) as f64,
        (i64::from(rect.y) + i64::from(rect.height)) as f64,
    );
    let left = clamp(left.floor());
    let top = clamp(top.floor());
    let right = clamp(right.ceil());
    let bottom = clamp(bottom.ceil());
    Rect::new(
        left,
        top,
        right.saturating_sub(left).max(0) as u32,
        bottom.saturating_sub(top).max(0) as u32,
    )
}

fn projected_gui_rect(rect: &GuiRect, projection: ClipperProjection) -> GuiRect {
    let (left, top) =
        projection.logical_to_physical(f64::from(rect.origin.x), f64::from(rect.origin.y));
    let (right, bottom) = projection.logical_to_physical(
        f64::from(rect.origin.x + rect.size.width),
        f64::from(rect.origin.y + rect.size.height),
    );
    GuiRect::new(
        left as f32,
        top as f32,
        (right - left) as f32,
        (bottom - top) as f32,
    )
}

#[allow(clippy::too_many_arguments)]
fn draw_native_decoration(
    surface: &mut Surface,
    bounds: Rect,
    projection: ClipperProjection,
    decoration: &ObjectMenuFrameDecoration,
    image: Option<&ImageData>,
    gamma: Option<&GammaRamp>,
) {
    let physical_bounds = projected_rect(bounds, projection);
    if physical_bounds.width == 0 || physical_bounds.height == 0 {
        return;
    }
    lc_frontend::classic_gui::draw_engine_box(
        surface,
        physical_bounds.x,
        physical_bounds.y,
        physical_bounds
            .x
            .saturating_add(physical_bounds.width as i32)
            .saturating_sub(1),
        physical_bounds
            .y
            .saturating_add(physical_bounds.height as i32)
            .saturating_sub(1),
        decoration.background_color,
        gamma,
    );
    let Some(image) = image else {
        return;
    };
    let width = extent_i32(bounds.width);
    let height = extent_i32(bounds.height);
    let mut draw_facet = |facet: &lc_engine::DefinitionActionFacet,
                          x: i32,
                          y: i32,
                          draw_width: i32,
                          draw_height: i32| {
        if draw_width <= 0 || draw_height <= 0 {
            return;
        }
        let (target_x, target_y) = projection.logical_to_physical(f64::from(x), f64::from(y));
        let (target_right, target_bottom) = projection.logical_to_physical(
            f64::from(x.saturating_add(draw_width)),
            f64::from(y.saturating_add(draw_height)),
        );
        lc_frontend::classic_gui::draw_facet_stretch(
            surface,
            image,
            (
                facet.x as f32,
                facet.y as f32,
                draw_width as f32,
                draw_height as f32,
            ),
            (
                target_x as f32,
                target_y as f32,
                (target_right - target_x) as f32,
                (target_bottom - target_y) as f32,
            ),
            gamma,
        );
    };

    if let Some(facet) = decoration.top.as_ref().filter(|facet| facet.width > 0) {
        let mut x = decoration.border_left;
        while x < width - decoration.border_right {
            let draw_width = facet.width.min(width - decoration.border_right - x);
            draw_facet(
                facet,
                bounds.x.saturating_add(x),
                bounds.y.saturating_add(facet.target_y),
                draw_width,
                facet.height,
            );
            x = x.saturating_add(facet.width);
        }
    }
    if let Some(facet) = decoration.left.as_ref().filter(|facet| facet.height > 0) {
        let mut y = decoration.border_top;
        while y < height - decoration.border_bottom {
            let draw_height = facet.height.min(height - decoration.border_bottom - y);
            draw_facet(
                facet,
                bounds.x.saturating_add(facet.target_x),
                bounds.y.saturating_add(y),
                facet.width,
                draw_height,
            );
            y = y.saturating_add(facet.height);
        }
    }
    if let Some(facet) = decoration.right.as_ref().filter(|facet| facet.height > 0) {
        let mut y = decoration.border_top;
        while y < height - decoration.border_bottom {
            let draw_height = facet.height.min(height - decoration.border_bottom - y);
            draw_facet(
                facet,
                bounds
                    .x
                    .saturating_add(width - decoration.border_right)
                    .saturating_add(facet.target_x),
                bounds.y.saturating_add(y),
                facet.width,
                draw_height,
            );
            y = y.saturating_add(facet.height);
        }
    }
    if let Some(facet) = decoration.bottom.as_ref().filter(|facet| facet.width > 0) {
        let mut x = decoration.border_left;
        while x < width - decoration.border_right {
            let draw_width = facet.width.min(width - decoration.border_right - x);
            draw_facet(
                facet,
                bounds.x.saturating_add(x),
                bounds
                    .y
                    .saturating_add(height - decoration.border_bottom)
                    .saturating_add(facet.target_y),
                draw_width,
                facet.height,
            );
            x = x.saturating_add(facet.width);
        }
    }
    for (facet, x, y) in [
        (decoration.top_left.as_ref(), bounds.x, bounds.y),
        (
            decoration.top_right.as_ref(),
            bounds.x.saturating_add(width - decoration.border_right),
            bounds.y,
        ),
        (
            decoration.bottom_left.as_ref(),
            bounds.x,
            bounds.y.saturating_add(height - decoration.border_bottom),
        ),
        (
            decoration.bottom_right.as_ref(),
            bounds.x.saturating_add(width - decoration.border_right),
            bounds.y.saturating_add(height - decoration.border_bottom),
        ),
    ] {
        if let Some(facet) = facet {
            draw_facet(
                facet,
                x.saturating_add(facet.target_x),
                y.saturating_add(facet.target_y),
                facet.width,
                facet.height,
            );
        }
    }
}

fn message_text(message: &MessageSnapshot) -> String {
    message.lines.join("|")
}

fn text_alignment(flags: u32, portrait: bool) -> TextAlign {
    if flags & FLAG_ALIGN_LEFT != 0 {
        TextAlign::Left
    } else if flags & FLAG_ALIGN_CENTER != 0 {
        TextAlign::Center
    } else if flags & FLAG_ALIGN_RIGHT != 0 {
        TextAlign::Right
    } else if portrait {
        TextAlign::Left
    } else {
        TextAlign::Center
    }
}

fn extent_i32(value: u32) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

fn percent(value: i32, extent: i32) -> i32 {
    (i64::from(value) * i64::from(extent) / 100).clamp(i64::from(i32::MIN), i64::from(i32::MAX))
        as i32
}

fn bound_by(value: i32, left: i32, right: i32) -> i32 {
    if value < left {
        left
    } else if value > right {
        right
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lc_engine::{Vector2, FLAG_LEFT, FLAG_TOP};

    #[derive(Default)]
    struct TestFontImages {
        rgba: Vec<u8>,
    }

    impl FontImageProvider for TestFontImages {
        fn font_image(&self, tag: &str) -> Option<lc_graphics::clonk_font::FontImageRef<'_>> {
            (tag == "Ico:Test" && !self.rgba.is_empty()).then_some(
                lc_graphics::clonk_font::FontImageRef {
                    width: 2,
                    height: 1,
                    rgba: &self.rgba,
                },
            )
        }
    }

    #[test]
    fn scale_three_tutorial01_layout_uses_native_cpp_metrics() {
        // C4GameMessage::Draw asks scale-native FontRegular for the extent and
        // BreakMessage height before it sizes the DECO frame
        // (src/C4GameMessage.cpp:99-170; src/StdFont.cpp:571-760).
        let font_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../planet/System.c4g/Endeavour.ttf");
        let bytes = std::fs::read(font_path).expect("read Endeavour.ttf");
        let fonts = lc_frontend::clonk_fonts::build_native_font_set(&bytes, 3)
            .expect("build scale-three FontRegular");
        assert_eq!(
            fonts.text.measure("Welcome to the world of Clonk.", true),
            (195, 22)
        );
        let decoration = ObjectMenuFrameDecoration {
            source_definition: "DECO".to_string(),
            background_color: 0x8032_3232,
            border_top: 0,
            border_left: 0,
            border_right: 0,
            border_bottom: 0,
            top: None,
            top_right: None,
            right: None,
            bottom_right: None,
            bottom: None,
            bottom_left: None,
            left: None,
            top_left: None,
        };
        let message = MessageSnapshot {
            id: 1,
            kind: MessageKind::GlobalPlayer,
            lines: vec!["Welcome to the world of Clonk.".to_string()],
            target: None,
            player: Some(1),
            offset: Vector2::new(50, 50),
            color: 0xffff_ffff,
            flags: FLAG_TOP | FLAG_LEFT | FLAG_WIDTH_REL | FLAG_X_REL | (1 << 8), // C4GM_DropSpeech
            width: Some(30),
            decoration: Some("DECO".to_string()),
            frame_decoration: Some(decoration.clone()),
            portrait: Some("Portrait:SCLK::0000ff::1".to_string()),
        };
        let images = TestFontImages::default();

        let layout = layout_global_message(
            MessageFontMetrics::Native(&fonts.text),
            Rect::new(0, 0, 320, 200),
            &message,
            Some(&decoration),
            &images,
        )
        .expect("layout Tutorial01 message");

        assert_eq!(layout.frame, Some(Rect::new(160, 50, 170, 66)));
        assert_eq!((layout.text_x, layout.text_y), (234, 50));
        assert_eq!(
            layout.text, "Welcome to\nthe world of\nClonk.",
            "96 GUI pixels wrap the native text to three 22px lines",
        );

        let reported_layout = layout_global_message(
            MessageFontMetrics::Native(&fonts.text),
            Rect::new(216, 56, 720, 560),
            &message,
            Some(&decoration),
            &images,
        )
        .expect("layout Tutorial01 at the reported 1152x644 logical surface");
        assert_eq!(reported_layout.frame, Some(Rect::new(576, 106, 279, 64)));
        assert_eq!((reported_layout.text_x, reported_layout.text_y), (650, 106));
        assert_eq!(reported_layout.text, "Welcome to the world of Clonk.");
    }

    #[test]
    fn tagged_global_message_is_supported_and_draws_inline_image() {
        let message = MessageSnapshot {
            id: 7,
            kind: MessageKind::Global,
            lines: vec!["{{Ico:Test}}".to_string()],
            target: None,
            player: None,
            offset: Vector2::new(0, 0),
            color: 0xffff_ffff,
            flags: FLAG_NO_BREAK | FLAG_ALIGN_LEFT,
            width: None,
            decoration: None,
            frame_decoration: None,
            portrait: None,
        };
        assert!(is_supported(&message));

        let images = TestFontImages {
            rgba: vec![255, 0, 0, 255, 255, 0, 0, 255],
        };
        let font = ClonkFont::new(3);
        let mut surface = Surface::new(64, 64, lc_graphics::PixelFormat::Rgba8888);
        draw_global_message(
            &mut surface,
            &font,
            Rect::new(0, 0, 64, 64),
            &message,
            None,
            None,
            None,
            &images,
            None,
        )
        .expect("draw tagged global message");

        let pixel = surface.get_pixel(32, 57).expect("drawn image pixel");
        assert!(pixel.r > 0 && pixel.a > 0, "inline image was not drawn");
    }
}
