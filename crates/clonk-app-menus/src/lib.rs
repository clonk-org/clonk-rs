//! In-game menu chrome extracted from the clonk-app monolith: the
//! `C4MainMenu` player menu, `C4Menu::ConvertCom` control mapping, the
//! game-over/evaluation dialog and their shared software image compositing.

pub mod clonk_fonts;
pub mod game_over;
pub mod ingame_menu;
pub mod menu_controls;
pub mod menu_images;
pub mod object_menu;
pub mod scrollbar;

pub use ingame_menu::substitute_resource_arguments;
pub use menu_images::{copy_menu_image_aspect, software_blit_menu_image};

#[cfg(test)]
fn tutorial_seven_gamma() -> clonk_graphics::GammaRamp {
    // The regression must track the shipped scenario rather than a synthetic
    // approximation (Tutorial07.c4s/Script.c:12).
    let script = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../content/Tutorial.c4f/Tutorial07.c4s/Script.c"
    ));
    assert!(script.contains("SetGamma(RGB(0,0,0),RGB(100,128,100),RGB(200,255,200))"));
    clonk_graphics::GammaRamp::from_control_points([0x000000, 0x648064, 0xc8ffc8])
}

#[cfg(test)]
fn tutorial_seven_gamma_color(color: clonk_graphics::Color) -> clonk_graphics::Color {
    use clonk_graphics::gamma::GammaChannel;

    let gamma = tutorial_seven_gamma();
    clonk_graphics::Color::new(
        gamma.encode_channel(GammaChannel::Red, color.r),
        gamma.encode_channel(GammaChannel::Green, color.g),
        gamma.encode_channel(GammaChannel::Blue, color.b),
        color.a,
    )
}
