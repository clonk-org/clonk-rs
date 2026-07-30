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
    let logo = image::load_from_memory(png).ok()?.to_rgba8();
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
            .write_to(&mut bytes, image::ImageOutputFormat::Png)
            .expect("an in-memory PNG encode cannot fail");
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
}
