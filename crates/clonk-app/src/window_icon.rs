//! The product window icon.
//!
//! `C4FullScreen.cpp:196-211` and `C4Console.cpp:297-310` load the same icon
//! resource into both the large and small window-class icon slots, so the game
//! window and the developer console show one identical icon.
//!
//! C++ takes that icon from `src/res/lc.ico`, which carries LegacyClonk's
//! branding. This port ships as a separate product, and the release tooling
//! already derives its bundle icon from `planet/Graphics.c4g/Logo.png`
//! (`xtask/src/main.rs:31-32`), so the window icon is taken from the same
//! source rather than the engine's. That keeps one product identity across the
//! bundle icon and the window chrome.

/// The source the release bundle icon also uses. Embedded so the window still
/// has an icon when the data root is missing or unreadable.
const LOGO_PNG: &[u8] = include_bytes!("../../../planet/Graphics.c4g/Logo.png");

/// Side length of the decoded icon. Windows asks for a large and a small
/// variant and downsamples the rest itself; winit takes one RGBA image and
/// lets each platform pick.
pub(crate) const WINDOW_ICON_SIDE: u32 = 64;

/// A decoded icon: `side * side` RGBA pixels.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WindowIconImage {
    pub(crate) rgba: Vec<u8>,
    pub(crate) side: u32,
}

/// Decodes the product logo into a square RGBA icon.
///
/// The logo is not square, so it is centred on a transparent square before
/// scaling — the same fit the bundle icon uses (`xtask/src/main.rs:1936-1943`) —
/// which preserves its aspect ratio instead of stretching it.
pub(crate) fn window_icon_image() -> Option<WindowIconImage> {
    let logo = image::load_from_memory(LOGO_PNG).ok()?.to_rgba8();
    let side = logo.width().max(logo.height());
    if side == 0 {
        return None;
    }
    let mut square = image::RgbaImage::from_pixel(side, side, image::Rgba([0, 0, 0, 0]));
    image::imageops::overlay(
        &mut square,
        &logo,
        i64::from((side - logo.width()) / 2),
        i64::from((side - logo.height()) / 2),
    );
    let scaled = image::imageops::resize(
        &square,
        WINDOW_ICON_SIDE,
        WINDOW_ICON_SIDE,
        image::imageops::FilterType::Lanczos3,
    );
    Some(WindowIconImage {
        rgba: scaled.into_raw(),
        side: WINDOW_ICON_SIDE,
    })
}

/// The icon both shells attach to their window. `None` leaves the platform
/// default in place, which is what C++ does when the resource fails to load
/// (`C4FullScreen.cpp:196-211` ignores a null `HICON`).
pub(crate) fn window_icon() -> Option<winit::window::Icon> {
    let image = window_icon_image()?;
    winit::window::Icon::from_rgba(image.rgba, image.side, image.side).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    // C4FullScreen.cpp:196-211; C4Console.cpp:297-310 — one icon serves both
    // shells, and both large and small slots take the same image.
    #[test]
    fn classic_window_icon_decodes_and_is_attached_to_both_shells() {
        let image = window_icon_image().expect("the embedded product logo decodes");
        assert_eq!(image.side, WINDOW_ICON_SIDE);
        assert_eq!(
            image.rgba.len(),
            (WINDOW_ICON_SIDE * WINDOW_ICON_SIDE * 4) as usize,
            "winit requires exactly side * side RGBA bytes"
        );
        // A fully transparent icon would silently render as nothing.
        assert!(
            image.rgba.chunks_exact(4).any(|pixel| pixel[3] != 0),
            "the decoded icon is entirely transparent"
        );

        // winit accepts it, which is what both shells attach.
        assert!(window_icon().is_some(), "winit rejected the decoded icon");

        // Both shells take the same image, as C++ assigns one resource to the
        // fullscreen and console window classes alike.
        assert_eq!(window_icon_image(), Some(image));
    }
}
