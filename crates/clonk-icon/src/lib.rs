//! The product icon, derived from one source for every consumer.
//!
//! C++ takes its icons from `src/res/lc.ico`, which carries LegacyClonk's
//! branding. This port ships as a separate product and derives its own icon
//! from `planet/Graphics.c4g/Logo.png` instead, so one image backs the window
//! chrome, the macOS bundle `.icns`, and the Windows executable resource.
//!
//! This crate exists because that derivation has four consumers — the running
//! window, the macOS Dock, the release packaging tool, and the Windows
//! resource build scripts — and it must produce the same pixels for all of
//! them. It is deliberately a leaf: `image` is its only dependency, so a build
//! script can depend on it without dragging in the engine.

#[cfg(feature = "build-script")]
pub mod build;

/// The image every icon is cut from. Embedded so consumers that have no data
/// root — build scripts, and the game before its paths are resolved — still
/// produce an icon.
pub const LOGO_PNG: &[u8] = include_bytes!("../../../planet/Graphics.c4g/Logo.png");

/// The leading stone "C" of the wordmark, as `(x, y, width, height)` in
/// [`LOGO_PNG`] pixels.
///
/// The wordmark is 2.2:1, so squaring the whole of it letterboxes the mark into
/// a strip that is illegible below about 48px — which is most of the places an
/// icon is drawn. Its first glyph is a near-square stone "C" that reads at every
/// size, and cutting the icon from it keeps one product identity in one source
/// image. The glyphs overlap, so the right edge is the narrowest column between
/// the "C" and the "L" rather than a transparent gap.
const APP_MARK: (u32, u32, u32, u32) = (134, 30, 170, 176);

/// Cuts the icon's mark out of the logo and composites it onto a transparent
/// square.
///
/// Padding with a background colour is not an option: the icon has to sit on
/// whatever the Dock, the taskbar or a file manager puts behind it.
pub fn square_source(png: &[u8]) -> Option<image::RgbaImage> {
    let logo = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .ok()?
        .to_rgba8();
    let mark = app_mark(&logo);
    let side = mark.width().max(mark.height());
    if side == 0 {
        return None;
    }
    let mut square = image::RgbaImage::from_pixel(side, side, image::Rgba([0, 0, 0, 0]));
    image::imageops::overlay(
        &mut square,
        &mark,
        i64::from((side - mark.width()) / 2),
        i64::from((side - mark.height()) / 2),
    );
    Some(square)
}

/// Crops [`APP_MARK`] out of the logo.
///
/// A source that does not contain the rectangle is used whole: the rectangle
/// describes one specific image, and a caller handing over another one wants
/// that image scaled, not an empty icon.
fn app_mark(logo: &image::RgbaImage) -> image::RgbaImage {
    let (x, y, width, height) = APP_MARK;
    if logo.width() < x + width || logo.height() < y + height {
        return logo.clone();
    }
    image::imageops::crop_imm(logo, x, y, width, height).to_image()
}

/// Scales a square icon source to `side` pixels.
///
/// Split from [`square_source`] because the packaging tool asks for ten sizes
/// from one decode, and decoding the logo per size is the expensive half.
pub fn resize_square(square: &image::RgbaImage, side: u32) -> image::RgbaImage {
    // `image` filters each channel independently, so scaling straight alpha
    // averages the transparent margin's RGB — black, in the source PNG — into
    // every antialiased edge and leaves a dark fringe at the small sizes.
    let mut scaled = image::imageops::resize(
        &premultiplied(square),
        side,
        side,
        image::imageops::FilterType::Lanczos3,
    );
    straighten(&mut scaled);
    scaled
}

/// One `ICON` entry of the Windows executable's resource table.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WindowsIconResource {
    /// The `engine_resource.h` symbol, kept so the table can be read against the
    /// C++ script it mirrors.
    pub symbol: &'static str,
    /// The numeric resource id. Windows orders icon groups by id, and a
    /// `DefaultIcon` ordinal indexes that order.
    pub id: u16,
    /// The `.ico` under `res/windows`, or `None` for the application icon, which
    /// is generated from the logo rather than shipped as a file.
    pub file: Option<&'static str>,
}

/// The engine's icon resources, in `src/res/engine.rc:33-46` order.
///
/// The order is load-bearing: `clonk-platform`'s `file_classes` writes
/// `DefaultIcon` values of the form `<exe>,<ordinal>`, and Windows reads a
/// positive ordinal as an index into the executable's icon groups sorted by
/// resource id. Reordering this table silently repoints every file association
/// at the wrong picture.
///
/// Slot 0 is the application icon, and it is the one deliberate divergence:
/// C++ puts `lc.ico` there, which carries LegacyClonk's branding, while this
/// port generates its own mark from the logo. The thirteen file-class icons are
/// the engine's own, recovered from `src/res` at the pinned snapshot — there is
/// no Clonk Rust artwork for a scenario, a group or a definition.
pub const WINDOWS_ICON_RESOURCES: [WindowsIconResource; 14] = [
    WindowsIconResource {
        symbol: "IDI_00_C4X",
        id: 4000,
        file: None,
    },
    WindowsIconResource {
        symbol: "IDI_01_C4S",
        id: 4001,
        file: Some("c4s.ico"),
    },
    WindowsIconResource {
        symbol: "IDI_02_C4G",
        id: 4002,
        file: Some("c4g.ico"),
    },
    WindowsIconResource {
        symbol: "IDI_03_C4F",
        id: 4003,
        file: Some("c4f.ico"),
    },
    WindowsIconResource {
        symbol: "IDI_04_C4P",
        id: 4004,
        file: Some("c4p.ico"),
    },
    WindowsIconResource {
        symbol: "IDI_05_C4X",
        id: 4005,
        file: Some("c4x.ico"),
    },
    WindowsIconResource {
        symbol: "IDI_06_C4D",
        id: 4006,
        file: Some("c4d.ico"),
    },
    WindowsIconResource {
        symbol: "IDI_07_C4I",
        id: 4007,
        file: Some("c4i.ico"),
    },
    WindowsIconResource {
        symbol: "IDI_08_C4M",
        id: 4008,
        file: Some("c4m.ico"),
    },
    WindowsIconResource {
        symbol: "IDI_09_C4B",
        id: 4009,
        file: Some("c4b.ico"),
    },
    WindowsIconResource {
        symbol: "IDI_10_C4V",
        id: 4010,
        file: Some("c4v.ico"),
    },
    WindowsIconResource {
        symbol: "IDI_11_C4L",
        id: 4011,
        file: Some("c4l.ico"),
    },
    WindowsIconResource {
        symbol: "IDI_12_C4K",
        id: 4012,
        file: Some("c4k.ico"),
    },
    WindowsIconResource {
        symbol: "IDI_13_C4U",
        id: 4013,
        file: Some("c4u.ico"),
    },
];

/// The sizes the Windows executable resource carries, ascending.
///
/// Explorer, the taskbar, Alt-Tab and the jumbo view all ask for different
/// sizes and upscale whatever is nearest, so an `.ico` with one entry looks
/// wrong in most of them. 256 is the largest an `ICONDIRENTRY` can address.
pub const WINDOWS_ICON_SIDES: [u32; 6] = [16, 32, 48, 64, 128, 256];

/// Encodes the product icon as a Windows `.ico` carrying every size in
/// [`WINDOWS_ICON_SIDES`].
pub fn app_ico_bytes() -> Option<Vec<u8>> {
    let square = square_source(LOGO_PNG)?;
    let frames = WINDOWS_ICON_SIDES
        .iter()
        .map(|&side| {
            let icon = resize_square(&square, side);
            image::codecs::ico::IcoFrame::as_png(
                icon.as_raw(),
                icon.width(),
                icon.height(),
                image::ColorType::Rgba8.into(),
            )
            .ok()
        })
        .collect::<Option<Vec<_>>>()?;

    let mut bytes = Vec::new();
    image::codecs::ico::IcoEncoder::new(&mut bytes)
        .encode_images(&frames)
        .ok()?;
    Some(bytes)
}

/// Encodes an icon as a PNG.
///
/// AppKit's `NSImage` and the Windows `.ico` container both take an encoded
/// image rather than a raw buffer, and PNG is the only format both accept.
pub fn png_bytes(icon: &image::RgbaImage) -> Option<Vec<u8>> {
    use image::ImageEncoder;

    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(
            icon.as_raw(),
            icon.width(),
            icon.height(),
            image::ColorType::Rgba8.into(),
        )
        .ok()?;
    Some(bytes)
}

/// Scales each colour channel by its own alpha, so a filter that averages
/// channels independently cannot pull colour out of transparent pixels.
fn premultiplied(icon: &image::RgbaImage) -> image::RgbaImage {
    let mut premultiplied = icon.clone();
    premultiplied.pixels_mut().for_each(|pixel| {
        let alpha = u32::from(pixel.0[3]);
        pixel.0[..3]
            .iter_mut()
            .for_each(|channel| *channel = ((u32::from(*channel) * alpha + 127) / 255) as u8);
    });
    premultiplied
}

/// Undoes [`premultiplied`]. Lanczos undershoots around a hard edge, so a
/// channel can land above its own alpha; clamping keeps the result a valid
/// straight-alpha image, which is what every icon container stores.
fn straighten(icon: &mut image::RgbaImage) {
    icon.pixels_mut().for_each(|pixel| {
        let alpha = u32::from(pixel.0[3]);
        if alpha == 0 {
            pixel.0 = [0, 0, 0, 0];
            return;
        }
        pixel.0[..3].iter_mut().for_each(|channel| {
            *channel = ((u32::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_of(image: &image::RgbaImage) -> Vec<u8> {
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image.clone())
            .write_to(&mut bytes, image::ImageFormat::Png)
            .expect("an in-memory PNG encode cannot fail");
        bytes.into_inner()
    }

    fn jpeg_of(image: &image::RgbImage) -> Vec<u8> {
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image.clone())
            .write_to(&mut bytes, image::ImageFormat::Jpeg)
            .expect("an in-memory JPEG encode cannot fail");
        bytes.into_inner()
    }

    /// The darkest colour channel over every pixel the viewer can actually see.
    fn darkest_visible_channel(icon: &image::RgbaImage) -> u8 {
        icon.pixels()
            .filter(|pixel| pixel.0[3] >= 8)
            .filter_map(|pixel| pixel.0[..3].iter().copied().min())
            .min()
            .expect("the scaled icon has visible pixels")
    }

    /// The share of a tile the viewer sees as solid, which is what makes an icon
    /// read as a mark rather than as a smudge.
    fn solid_coverage_percent(icon: &image::RgbaImage) -> u32 {
        let solid = icon.pixels().filter(|pixel| pixel.0[3] >= 128).count();
        (100 * solid as u32) / (icon.width() * icon.height())
    }

    // The smallest tile is the honest test of an app icon: a Finder list row and
    // a crowded taskbar both draw it at 16px. A wordmark squared up covers 18%
    // of that tile and reads as a beige smear; a real mark fills it.
    #[test]
    fn the_app_icon_fills_its_smallest_tile() {
        let square = square_source(LOGO_PNG).expect("the embedded product logo decodes");

        let coverage = solid_coverage_percent(&resize_square(&square, 16));

        assert!(
            coverage >= 50,
            "the 16px icon is only {coverage}% solid, which reads as no icon at all"
        );
    }

    // `DefaultIcon` values are written as `<exe>,<ordinal>`, and Windows reads a
    // positive ordinal as an index into the exe's icon groups sorted by resource
    // id. So the table's ORDER is the contract, and it has to be engine.rc's.
    #[test]
    fn the_resource_table_mirrors_the_engine_resource_script() {
        // `src/res/engine.rc:33-46` at the pinned snapshot, in file order.
        let engine_rc = [
            "IDI_00_C4X",
            "IDI_01_C4S",
            "IDI_02_C4G",
            "IDI_03_C4F",
            "IDI_04_C4P",
            "IDI_05_C4X",
            "IDI_06_C4D",
            "IDI_07_C4I",
            "IDI_08_C4M",
            "IDI_09_C4B",
            "IDI_10_C4V",
            "IDI_11_C4L",
            "IDI_12_C4K",
            "IDI_13_C4U",
        ];

        let symbols: Vec<&str> = WINDOWS_ICON_RESOURCES
            .iter()
            .map(|resource| resource.symbol)
            .collect();

        assert_eq!(symbols, engine_rc.to_vec());
        // `src/res/engine_resource.h:61-74` numbers them 4000 upwards with no
        // gaps, which is what makes the ordinals contiguous.
        assert!(
            WINDOWS_ICON_RESOURCES
                .iter()
                .enumerate()
                .all(|(ordinal, resource)| resource.id == 4000 + ordinal as u16),
            "the resource ids are no longer 4000.. in table order"
        );
    }

    // The engine registers `<exe>,1` for `.c4s`, `<exe>,6` for `.c4d` and so on
    // (`clonk-platform`'s `file_classes`), and the ordinals it picks skip 5 and
    // 12. Those slots still have to exist or every later ordinal shifts down.
    #[test]
    fn the_ordinals_the_file_classes_register_all_exist() {
        // `C4FileClasses.cpp:47-58` icon indices, plus the protocol's `,1`.
        let registered = [1, 2, 3, 4, 6, 7, 8, 9, 10, 11, 13];

        assert!(
            registered
                .iter()
                .all(|ordinal| *ordinal < WINDOWS_ICON_RESOURCES.len()),
            "a registered DefaultIcon ordinal has no resource behind it"
        );
        assert!(
            !registered.contains(&5) && !registered.contains(&12),
            "C++ deliberately skips ordinals 5 and 12"
        );
    }

    // Explorer picks the nearest embedded size, so an `.ico` that carries only
    // one size is upscaled in half the places it is drawn.
    #[test]
    fn the_windows_icon_carries_every_size_explorer_asks_for() {
        let ico = app_ico_bytes().expect("the product icon encodes as an .ico");

        // ICONDIR: reserved 0, type 1 (icon), then the entry count.
        assert_eq!(&ico[..4], &[0, 0, 1, 0], "not an ICONDIR");
        let entries = u16::from_le_bytes([ico[4], ico[5]]);
        assert_eq!(usize::from(entries), WINDOWS_ICON_SIDES.len());

        // Every entry must declare its side and point at bytes inside the file.
        // `0` means 256 in an ICONDIRENTRY, which is the only way to spell it.
        let declared: Vec<u32> = (0..usize::from(entries))
            .map(|index| {
                let entry = &ico[6 + index * 16..6 + (index + 1) * 16];
                let length =
                    u32::from_le_bytes([entry[8], entry[9], entry[10], entry[11]]) as usize;
                let offset =
                    u32::from_le_bytes([entry[12], entry[13], entry[14], entry[15]]) as usize;
                assert!(
                    offset + length <= ico.len(),
                    "entry {index} points past the end of the file"
                );
                if entry[0] == 0 {
                    256
                } else {
                    u32::from(entry[0])
                }
            })
            .collect();
        assert_eq!(declared, WINDOWS_ICON_SIDES.to_vec());
    }

    // AppKit and the icon containers take an encoded image, not a raw buffer.
    #[test]
    fn an_icon_encodes_to_a_png_that_decodes_back_unchanged() {
        let square = square_source(LOGO_PNG).expect("the embedded product logo decodes");
        let icon = resize_square(&square, 32);

        let bytes = png_bytes(&icon).expect("a 32px icon encodes");

        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "not a PNG");
        let decoded = image::load_from_memory(&bytes)
            .expect("the encoded icon decodes")
            .to_rgba8();
        assert_eq!(decoded, icon, "the encode lost pixels");
    }

    // `APP_MARK` is tied to one specific image. If the logo is ever redrawn or
    // repositioned, the mark stops touching its own crop edges — clipped on one
    // side, floating on another — and that has to fail here rather than ship.
    #[test]
    fn the_crop_rectangle_is_tight_around_the_mark() {
        let logo = image::load_from_memory(LOGO_PNG)
            .expect("the embedded product logo decodes")
            .to_rgba8();
        let mark = app_mark(&logo);
        assert_ne!(
            mark.dimensions(),
            logo.dimensions(),
            "the crop rectangle no longer fits inside the logo"
        );

        let (width, height) = mark.dimensions();
        let inked = |x: u32, y: u32| mark.get_pixel(x, y).0[3] >= 8;
        assert!(
            (0..width).any(|x| inked(x, 0)),
            "the mark does not reach the top of its crop"
        );
        assert!(
            (0..width).any(|x| inked(x, height - 1)),
            "the mark does not reach the bottom of its crop"
        );
        assert!(
            (0..height).any(|y| inked(0, y)),
            "the mark does not reach the left of its crop"
        );
        assert!(
            (0..height).any(|y| inked(width - 1, y)),
            "the mark does not reach the right of its crop"
        );
    }

    // A hard edge between opaque white and transparent black. Filtering straight
    // alpha ignores the alpha channel, so the margin's RGB — black, as it is in
    // the source PNG — is averaged into the antialiased edge and shows up as a
    // dark fringe at exactly the sizes a Dock or taskbar draws.
    #[test]
    fn downscaling_does_not_bleed_the_transparent_margin_into_the_edge() {
        let mut source = image::RgbaImage::from_pixel(64, 64, image::Rgba([0, 0, 0, 0]));
        (0..64).for_each(|y| {
            (0..32).for_each(|x| source.put_pixel(x, y, image::Rgba([255, 255, 255, 255])));
        });

        let darkest = darkest_visible_channel(&resize_square(&source, 16));

        assert!(
            darkest >= 250,
            "the transparent margin darkened a visible edge pixel to {darkest}"
        );
    }

    #[test]
    fn wide_source_is_centred_on_a_transparent_square() {
        // 4x2 opaque red: the square is 4x4 and the source lands on rows 1..3.
        let source = image::RgbaImage::from_pixel(4, 2, image::Rgba([255, 0, 0, 255]));
        let square = square_source(&png_of(&source)).expect("a 4x2 RGBA PNG decodes");

        assert_eq!(square.dimensions(), (4, 4));
        assert_eq!(
            square.get_pixel(0, 0).0,
            [0, 0, 0, 0],
            "row 0 stays padding"
        );
        assert_eq!(
            square.get_pixel(0, 3).0,
            [0, 0, 0, 0],
            "row 3 stays padding"
        );
        assert_eq!(square.get_pixel(0, 1).0, [255, 0, 0, 255]);
        assert_eq!(square.get_pixel(3, 2).0, [255, 0, 0, 255]);
    }

    #[test]
    fn the_embedded_logo_yields_a_square_of_the_requested_side() {
        let square = square_source(LOGO_PNG).expect("the embedded product logo decodes");
        assert_eq!(
            square.width(),
            square.height(),
            "the source handed to every consumer must be square"
        );

        let scaled = resize_square(&square, 64);
        assert_eq!(scaled.dimensions(), (64, 64));
        // A fully transparent icon would silently render as nothing.
        assert!(
            scaled.pixels().any(|pixel| pixel.0[3] != 0),
            "the scaled icon is entirely transparent"
        );
    }

    #[test]
    fn a_source_that_is_not_an_image_yields_no_icon() {
        assert!(square_source(b"not a png").is_none());
    }

    #[test]
    fn a_non_png_image_yields_no_icon() {
        let source = image::RgbImage::from_pixel(4, 2, image::Rgb([255, 0, 0]));

        assert!(square_source(&jpeg_of(&source)).is_none());
    }
}
