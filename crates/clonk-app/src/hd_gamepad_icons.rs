//! Approved high-resolution replacements for the four Gamepad.png phases.

use clonk_frontend::HudGraphics;
use clonk_gui::ImageData;
use std::{collections::HashMap, sync::OnceLock};

const ORIGINAL: &[u8] = include_bytes!("../../../planet/Graphics.c4g/Gamepad.png");
const ART: [&[u8]; 4] = [
    include_bytes!("../assets/gamepad-icons/gamepad-1.png"),
    include_bytes!("../assets/gamepad-icons/gamepad-2.png"),
    include_bytes!("../assets/gamepad-icons/gamepad-3.png"),
    include_bytes!("../assets/gamepad-icons/gamepad-4.png"),
];

struct GamepadIcons {
    original: ImageData,
    replacements: [ImageData; 4],
}

fn decode(bytes: &[u8]) -> anyhow::Result<ImageData> {
    let mut rgba =
        clonk_resources::load_image_from_memory_with_format(bytes, image::ImageFormat::Png)?
            .into_rgba8();
    for pixel in rgba.pixels_mut().filter(|pixel| pixel[3] == 0) {
        pixel.0 = [0; 4];
    }
    Ok(ImageData::new(rgba.width(), rgba.height(), rgba.into_raw()))
}

fn prepared_icons() -> anyhow::Result<&'static GamepadIcons> {
    static ICONS: OnceLock<Result<GamepadIcons, String>> = OnceLock::new();
    ICONS
        .get_or_init(|| {
            (|| {
                Ok(GamepadIcons {
                    original: decode(ORIGINAL)?,
                    replacements: [
                        decode(ART[0])?,
                        decode(ART[1])?,
                        decode(ART[2])?,
                        decode(ART[3])?,
                    ],
                })
            })()
            .map_err(|error: anyhow::Error| error.to_string())
        })
        .as_ref()
        .map_err(|error| anyhow::anyhow!("failed to decode gamepad icons: {error}"))
}

fn stock_cell_matches(image: &ImageData, original: &ImageData, x: u32) -> bool {
    if image.width() != original.width() || image.height() != original.height() {
        return false;
    }
    (0..36).all(|row| {
        let start = ((row * image.width() + x) * 4) as usize;
        let end = start + 80 * 4;
        image.pixels()[start..end] == original.pixels()[start..end]
    })
}

fn install_sheet(image: &mut ImageData, icons: &GamepadIcons) {
    for (phase, replacement) in icons.replacements.iter().enumerate() {
        let source = [phase as u32 * 80, 0, 80, 36];
        if stock_cell_matches(image, &icons.original, source[0])
            && image.region_replacement(source).is_none()
        {
            *image = image
                .clone()
                .with_region_replacement(source, replacement.clone());
        }
    }
}

pub(crate) fn install_hud(hud: &mut HudGraphics) -> anyhow::Result<()> {
    if let Some(image) = hud.gamepad.as_mut() {
        install_sheet(image, prepared_icons()?);
    }
    Ok(())
}

pub(crate) fn install(
    hud: &mut HudGraphics,
    dialog_images: &mut HashMap<String, ImageData>,
) -> anyhow::Result<()> {
    let icons = prepared_icons()?;
    if let Some(image) = hud.gamepad.as_mut() {
        install_sheet(image, icons);
    }
    if let Some(image) = dialog_images.get_mut("Gamepad.png") {
        install_sheet(image, icons);
    }
    Ok(())
}

pub(crate) fn is_installed(hud: &HudGraphics) -> bool {
    hud.gamepad
        .as_ref()
        .and_then(|image| image.region_replacement([0, 0, 80, 36]))
        .is_some()
}
