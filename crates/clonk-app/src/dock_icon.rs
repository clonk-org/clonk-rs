//! The macOS Dock icon.
//!
//! A packaged `.app` gets its Dock tile from `CFBundleIconFile`, but an
//! unbundled binary — `cargo run`, or `Contents/MacOS/clonk-app` invoked
//! directly — has no bundle to read it from, and the window icon cannot stand
//! in: winit accepts one and discards it on macOS. Its platform impl is an
//! empty body explaining the refusal
//! (`winit-0.30.13/src/platform_impl/macos/window_delegate.rs:1541-1549`), so
//! `WindowAttributes::with_window_icon` is silently dropped. AppKit's
//! `-[NSApplication setApplicationIconImage:]` is the only route.
//!
//! This has no C++ counterpart to match: `C4FullScreen.cpp:196-211` sets
//! `WM_SETICON`, which is Windows-only, and the SDL build relies on the bundle.
//! It is a port-only addition, not a parity gap.

/// Whether the pending Dock tile is due on this event.
///
/// The image only sticks once the process *has* a Dock tile, and it does not
/// have one until AppKit raises it to a foreground application. winit defers
/// that to `applicationDidFinishLaunching`
/// (`winit-0.30.13/src/platform_impl/macos/app_state.rs:107-125`), which runs
/// inside `[NSApp run]`. An image handed over before then has no tile to land
/// on, and the tile the Dock subsequently creates is drawn from the bundle;
/// an unbundled binary has none, so the generic executable icon stands.
/// `Resumed` is emitted immediately after that transition
/// (`app_state.rs:327-333`) and is the first moment the icon survives.
pub(crate) fn should_attach_dock_tile<T: 'static>(
    event: &winit::event::Event<T>,
    attached: bool,
) -> bool {
    !attached && matches!(event, winit::event::Event::Resumed)
}

/// Attaches the product icon to the running application's Dock tile.
///
/// Must be called from inside the running event loop — see
/// [`should_attach_dock_tile`] for why, and for why nothing may reach AppKit
/// before winit has installed its own `NSApplication` subclass.
#[cfg(target_os = "macos")]
pub(crate) fn set_dock_icon() {
    macos::set_dock_icon();
}

/// Every other platform draws the icon the window itself carries, which
/// `startup_window_attributes` already attaches.
#[cfg(not(target_os = "macos"))]
pub(crate) fn set_dock_icon() {}

#[cfg(all(
    test,
    any(not(feature = "app-test-shard-mode"), feature = "app-test-shard-5",),
))]
mod tests {
    use super::*;
    use winit::event::Event;

    // A tile attached before `[NSApp run]` is discarded by the foreground
    // transition that creates the real one
    // (winit-0.30.13/src/platform_impl/macos/app_state.rs:107-125), leaving an
    // unbundled run with the generic executable icon.
    #[test]
    fn the_dock_tile_is_attached_on_the_first_resume_only() {
        assert!(
            should_attach_dock_tile(&Event::<()>::Resumed, false),
            "the first resume is the earliest moment the Dock keeps the image"
        );
        assert!(
            !should_attach_dock_tile(&Event::<()>::Resumed, true),
            "re-cutting a 512px tile on every resume is pure waste"
        );
        assert!(
            !should_attach_dock_tile(&Event::<()>::AboutToWait, false),
            "the tile is not re-attached on every pass through the loop"
        );
    }
}

#[cfg(target_os = "macos")]
mod macos {
    /// Side length of the Dock image.
    ///
    /// The mark is cut from a 170x176 region of the logo, so every larger size
    /// is an upscale. 512 covers a retina Dock tile and the Cmd-Tab switcher
    /// without asking Lanczos for more than three times the source.
    const DOCK_ICON_SIDE: u32 = 512;

    pub(super) fn set_dock_icon() {
        // `alloc` comes from this trait, not from the class.
        use objc2::AnyThread;

        // AppKit is not thread-safe and `sharedApplication` requires the main
        // thread; off it, there is nothing to decorate anyway.
        let Some(main_thread) = objc2::MainThreadMarker::new() else {
            return;
        };
        let Some(png) = dock_icon_png() else {
            return;
        };
        // `with_bytes` copies rather than handing AppKit the Vec's deallocator,
        // which would need the `block2` feature for a copy this small.
        let data = objc2_foundation::NSData::with_bytes(&png);
        let Some(image) =
            objc2_app_kit::NSImage::initWithData(objc2_app_kit::NSImage::alloc(), &data)
        else {
            // A tile AppKit cannot decode is not worth failing startup over;
            // the platform default stands, as it does for a null `HICON`.
            return;
        };

        // SAFETY: `image` is a live `NSImage` this call does not take ownership
        // of, and `main_thread` proves we are on the thread AppKit requires.
        unsafe {
            objc2_app_kit::NSApplication::sharedApplication(main_thread)
                .setApplicationIconImage(Some(&image));
        }
    }

    /// The encoded tile handed to AppKit.
    fn dock_icon_png() -> Option<Vec<u8>> {
        let square = clonk_icon::square_source(clonk_icon::LOGO_PNG)?;
        clonk_icon::png_bytes(&clonk_icon::resize_square(&square, DOCK_ICON_SIDE))
    }

    #[cfg(all(
        test,
        any(not(feature = "app-test-shard-mode"), feature = "app-test-shard-5",),
    ))]
    mod tests {
        use super::*;

        // The Dock tile is the whole point of this module, and an AppKit
        // `NSImage` that fails to decode falls back to the generic icon
        // silently. Pin the bytes instead, which needs no window server.
        #[test]
        fn the_dock_tile_is_a_decodable_square_at_the_requested_side() {
            let png = dock_icon_png().expect("the product icon encodes");

            let image = image::load_from_memory(&png)
                .expect("AppKit is handed something it can decode")
                .to_rgba8();
            assert_eq!(image.dimensions(), (DOCK_ICON_SIDE, DOCK_ICON_SIDE));
            assert!(
                image.pixels().any(|pixel| pixel.0[3] != 0),
                "a fully transparent tile would render as no icon at all"
            );
        }
    }
}
