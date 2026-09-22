//! Approved high-resolution replacements for cells of the in-game menu sheets.

use clonk_frontend::HudGraphics;
use clonk_gui::ImageData;
use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
};

const ICONS: &[(&str, [u32; 4], &[u8])] = &[
    (
        "Menu.png",
        [0, 0, 35, 35],
        include_bytes!("../assets/menu-icons/menu-save.png"),
    ),
    (
        "Menu.png",
        [140, 0, 35, 35],
        include_bytes!("../assets/menu-icons/menu-goals.png"),
    ),
    (
        "Menu.png",
        [175, 0, 35, 35],
        include_bytes!("../assets/menu-icons/menu-rules.png"),
    ),
    (
        "Menu.png",
        [245, 0, 35, 35],
        include_bytes!("../assets/menu-icons/menu-hostility.png"),
    ),
    (
        "Menu.png",
        [280, 0, 35, 35],
        include_bytes!("../assets/menu-icons/menu-display.png"),
    ),
    (
        "Options.png",
        [0, 0, 35, 35],
        include_bytes!("../assets/menu-icons/options-settings.png"),
    ),
    (
        "Options.png",
        [35, 0, 35, 35],
        include_bytes!("../assets/menu-icons/options-music.png"),
    ),
    (
        "Options.png",
        [70, 0, 35, 35],
        include_bytes!("../assets/menu-icons/options-music-checked.png"),
    ),
    (
        "Options.png",
        [175, 0, 35, 35],
        include_bytes!("../assets/menu-icons/options-fps.png"),
    ),
    (
        "Options.png",
        [210, 0, 35, 35],
        include_bytes!("../assets/menu-icons/options-fps-checked.png"),
    ),
    (
        "Options.png",
        [595, 0, 35, 35],
        include_bytes!("../assets/menu-icons/options-audio.png"),
    ),
    (
        "Options.png",
        [630, 0, 35, 35],
        include_bytes!("../assets/menu-icons/options-audio-checked.png"),
    ),
];

struct IconSheet {
    name: &'static str,
    original: ImageData,
    icons: Vec<([u32; 4], ImageData)>,
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

fn prepared_sheets() -> anyhow::Result<&'static Vec<IconSheet>> {
    static SHEETS: OnceLock<Result<Vec<IconSheet>, String>> = OnceLock::new();
    SHEETS
        .get_or_init(|| {
            [
                (
                    "Menu.png",
                    include_bytes!("../../../planet/Graphics.c4g/Menu.png").as_slice(),
                ),
                (
                    "Options.png",
                    include_bytes!("../../../planet/Graphics.c4g/Options.png").as_slice(),
                ),
            ]
            .into_iter()
            .map(|(name, bytes)| {
                Ok(IconSheet {
                    name,
                    original: decode(bytes)?,
                    icons: ICONS
                        .iter()
                        .filter(|(sheet, _, _)| *sheet == name)
                        .map(|(_, source, bytes)| Ok((*source, decode(bytes)?)))
                        .collect::<anyhow::Result<Vec<_>>>()?,
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()
            .map_err(|error| error.to_string())
        })
        .as_ref()
        .map_err(|error| anyhow::anyhow!("failed to decode embedded menu icons: {error}"))
}

fn stock_cell_matches(
    image: &ImageData,
    original: &ImageData,
    [x, y, width, height]: [u32; 4],
) -> bool {
    if image.width() != original.width() || image.height() != original.height() {
        return false;
    }
    (y..y + height).all(|row| {
        let start = ((row * image.width() + x) * 4) as usize;
        let end = start + (width * 4) as usize;
        image.pixels()[start..end] == original.pixels()[start..end]
    })
}

fn install_sheet(image: &mut ImageData, sheet: &IconSheet) {
    for (source, replacement) in &sheet.icons {
        if stock_cell_matches(image, &sheet.original, *source)
            && image.region_replacement(*source).is_none()
        {
            *image = image
                .clone()
                .with_region_replacement(*source, replacement.clone());
        }
    }
}

pub(crate) fn install(
    hud: &mut HudGraphics,
    dialog_images: &mut HashMap<String, ImageData>,
) -> anyhow::Result<()> {
    for sheet in prepared_sheets()? {
        if sheet.name == "Menu.png" {
            if let Some(image) = hud.menu.as_mut() {
                install_sheet(image, sheet);
            }
        }
        if let Some(image) = dialog_images.get_mut(sheet.name) {
            install_sheet(image, sheet);
        }
    }
    Ok(())
}

pub(crate) fn install_game(
    hud: &mut HudGraphics,
    options: &mut Option<Arc<ImageData>>,
) -> anyhow::Result<()> {
    for sheet in prepared_sheets()? {
        match sheet.name {
            "Menu.png" => {
                if let Some(image) = hud.menu.as_mut() {
                    install_sheet(image, sheet);
                }
            }
            "Options.png" => {
                if let Some(image) = options.as_mut() {
                    install_sheet(Arc::make_mut(image), sheet);
                }
            }
            _ => {}
        }
    }
    Ok(())
}

pub(crate) fn is_installed(hud: &HudGraphics) -> bool {
    hud.menu
        .as_ref()
        .and_then(|image| image.region_replacement([0, 0, 35, 35]))
        .is_some()
}
