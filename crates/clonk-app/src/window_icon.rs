//! The product window icon.
//!
//! `C4FullScreen.cpp:196-211` and `C4Console.cpp:297-310` load the same icon
//! resource into both the large and small window-class icon slots, so the game
//! window and the developer console show one identical icon.
//!
//! C++ takes that icon from `src/res/lc.ico`, which carries LegacyClonk's
//! branding. This port ships as a separate product and derives its icon from
//! `planet/Graphics.c4g/Logo.png` instead. `clonk-icon` owns that derivation so
//! the window chrome, the macOS bundle `.icns` and the Windows executable
//! resource are cut from one image — see that crate for the rationale.

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
pub(crate) fn window_icon_image() -> Option<WindowIconImage> {
    let square = clonk_icon::square_source(clonk_icon::LOGO_PNG)?;
    Some(WindowIconImage {
        rgba: clonk_icon::resize_square(&square, WINDOW_ICON_SIDE).into_raw(),
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
