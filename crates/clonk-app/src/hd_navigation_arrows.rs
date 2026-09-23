//! Approved high-resolution replacements for the two navigation-arrow sheets.

use clonk_frontend::HudGraphics;
use clonk_gui::ImageData;
use std::{collections::HashMap, sync::OnceLock};

const RED_ORIGINAL: &[u8] = include_bytes!("../../../planet/Graphics.c4g/Arrow.png");
const WOOD_ORIGINAL: &[u8] = include_bytes!("../../../planet/Graphics.c4g/GUIBigArrows.png");
const RED_ART: [&[u8]; 4] = [
    include_bytes!("../assets/navigation-arrows/red-1.png"),
    include_bytes!("../assets/navigation-arrows/red-2.png"),
    include_bytes!("../assets/navigation-arrows/red-3.png"),
    include_bytes!("../assets/navigation-arrows/red-4.png"),
];
const WOOD_ART: [&[u8]; 4] = [
    include_bytes!("../assets/navigation-arrows/wood-1.png"),
    include_bytes!("../assets/navigation-arrows/wood-2.png"),
    include_bytes!("../assets/navigation-arrows/wood-3.png"),
    include_bytes!("../assets/navigation-arrows/wood-4.png"),
];

struct ArrowSheet {
    original: ImageData,
    replacements: Vec<ImageData>,
    cell: [u32; 2],
}

struct NavigationArrows {
    red: ArrowSheet,
    wood: ArrowSheet,
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

fn prepare_sheet(original: &[u8], art: [&[u8]; 4], cell: [u32; 2]) -> anyhow::Result<ArrowSheet> {
    Ok(ArrowSheet {
        original: decode(original)?,
        replacements: art
            .iter()
            .map(|bytes| decode(bytes))
            .collect::<anyhow::Result<_>>()?,
        cell,
    })
}

fn prepared_icons() -> anyhow::Result<&'static NavigationArrows> {
    static ICONS: OnceLock<Result<NavigationArrows, String>> = OnceLock::new();
    ICONS
        .get_or_init(|| {
            (|| {
                Ok(NavigationArrows {
                    red: prepare_sheet(RED_ORIGINAL, RED_ART, [64, 64])?,
                    wood: prepare_sheet(WOOD_ORIGINAL, WOOD_ART, [19, 40])?,
                })
            })()
            .map_err(|error: anyhow::Error| error.to_string())
        })
        .as_ref()
        .map_err(|error| anyhow::anyhow!("failed to decode navigation arrows: {error}"))
}

fn stock_cell_matches(image: &ImageData, sheet: &ArrowSheet, phase: u32) -> bool {
    if image.width() != sheet.original.width() || image.height() != sheet.original.height() {
        return false;
    }
    let [width, height] = sheet.cell;
    (0..height).all(|row| {
        let start = ((row * image.width() + phase * width) * 4) as usize;
        let end = start + (width * 4) as usize;
        image.pixels()[start..end] == sheet.original.pixels()[start..end]
    })
}

fn install_sheet(image: &mut ImageData, sheet: &ArrowSheet) {
    let [width, height] = sheet.cell;
    for (phase, replacement) in sheet.replacements.iter().enumerate() {
        let source = [phase as u32 * width, 0, width, height];
        if stock_cell_matches(image, sheet, phase as u32)
            && image.region_replacement(source).is_none()
        {
            *image = image
                .clone()
                .with_region_replacement(source, replacement.clone());
        }
    }
}

pub(crate) fn install(
    hud: &mut HudGraphics,
    dialog_images: &mut HashMap<String, ImageData>,
) -> anyhow::Result<()> {
    let icons = prepared_icons()?;
    if let Some(image) = hud.arrow.as_mut() {
        install_sheet(image, &icons.red);
    }
    if let Some(image) = dialog_images.get_mut("GUIBigArrows.png") {
        install_sheet(image, &icons.wood);
    }
    Ok(())
}

pub(crate) fn install_hud(hud: &mut HudGraphics) -> anyhow::Result<()> {
    if let Some(image) = hud.arrow.as_mut() {
        install_sheet(image, &prepared_icons()?.red);
    }
    Ok(())
}

pub(crate) fn is_installed(hud: &HudGraphics) -> bool {
    hud.arrow
        .as_ref()
        .and_then(|image| image.region_replacement([0, 0, 64, 64]))
        .is_some()
}
