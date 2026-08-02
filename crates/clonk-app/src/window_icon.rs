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

/// Side length of the title-bar icon. This is what winit's `with_window_icon`
/// carries, and on Windows it lands in `ICON_SMALL`, whose base size is 16 —
/// a multiple of it keeps display scaling from having to interpolate.
pub(crate) const WINDOW_ICON_SIDE: u32 = 64;

/// Side length of the taskbar icon. `ICON_BIG` is a separate slot from
/// `ICON_SMALL`, and 256 is the ceiling winit documents for it.
pub(crate) const TASKBAR_ICON_SIDE: u32 = 256;

/// A decoded icon: `side * side` RGBA pixels.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WindowIconImage {
    pub(crate) rgba: Vec<u8>,
    pub(crate) side: u32,
}

/// Decodes the product logo into a square RGBA icon of the given side.
pub(crate) fn window_icon_image_at(side: u32) -> Option<WindowIconImage> {
    let square = clonk_icon::square_source(clonk_icon::LOGO_PNG)?;
    Some(WindowIconImage {
        rgba: clonk_icon::resize_square(&square, side).into_raw(),
        side,
    })
}

/// Decodes the product logo into the title-bar icon.
pub(crate) fn window_icon_image() -> Option<WindowIconImage> {
    window_icon_image_at(WINDOW_ICON_SIDE)
}

fn icon_at(side: u32) -> Option<winit::window::Icon> {
    let image = window_icon_image_at(side)?;
    winit::window::Icon::from_rgba(image.rgba, image.side, image.side).ok()
}

/// The icon both shells attach to their window. `None` leaves the platform
/// default in place, which is what C++ does when the resource fails to load
/// (`C4FullScreen.cpp:196-211` ignores a null `HICON`).
pub(crate) fn window_icon() -> Option<winit::window::Icon> {
    icon_at(WINDOW_ICON_SIDE)
}

/// The larger image Windows draws on the taskbar button. Nothing else has a
/// second slot to fill: winit only reads this on Windows.
#[cfg(windows)]
pub(crate) fn taskbar_icon() -> Option<winit::window::Icon> {
    icon_at(TASKBAR_ICON_SIDE)
}

#[cfg(all(
    test,
    any(not(feature = "app-test-shard-mode"), feature = "app-test-shard-5",),
))]
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

    // `with_window_icon` sets `ICON_SMALL` only
    // (winit-0.30.13/src/platform_impl/windows/window.rs:887-908), which is
    // the title-bar slot. The taskbar button reads `ICON_BIG`, so without a
    // second image Windows stretches the 64px one across a button drawn at up
    // to 256.
    #[test]
    fn the_windows_taskbar_gets_its_own_larger_image() {
        let small = window_icon_image_at(WINDOW_ICON_SIDE).expect("the product logo decodes");
        let big = window_icon_image_at(TASKBAR_ICON_SIDE).expect("the product logo decodes");

        assert_eq!(small.side, 64, "ICON_SMALL is a 16px slot's base multiple");
        assert_eq!(
            big.side, 256,
            "256 is winit's documented ceiling for ICON_BIG"
        );
        assert!(
            big.side > small.side,
            "the taskbar image must not be an upscale of the title-bar one"
        );
        assert_eq!(big.rgba.len(), (big.side * big.side * 4) as usize);
    }
}
