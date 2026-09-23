//! Approved high-resolution replacements for the seven Hand.png gestures.

use clonk_frontend::HudGraphics;
use clonk_gui::ImageData;
use std::sync::OnceLock;

const ORIGINAL: &[u8] = include_bytes!("../../../planet/Graphics.c4g/Hand.png");
const ART: [&[u8]; 7] = [
    include_bytes!("../assets/hand-icons/hand-1.png"),
    include_bytes!("../assets/hand-icons/hand-2.png"),
    include_bytes!("../assets/hand-icons/hand-3.png"),
    include_bytes!("../assets/hand-icons/hand-4.png"),
    include_bytes!("../assets/hand-icons/hand-5.png"),
    include_bytes!("../assets/hand-icons/hand-6.png"),
    include_bytes!("../assets/hand-icons/hand-7.png"),
];

struct HandIcons {
    original: ImageData,
    replacements: Vec<ImageData>,
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

fn prepared_icons() -> anyhow::Result<&'static HandIcons> {
    static ICONS: OnceLock<Result<HandIcons, String>> = OnceLock::new();
    ICONS
        .get_or_init(|| {
            (|| {
                Ok(HandIcons {
                    original: decode(ORIGINAL)?,
                    replacements: ART
                        .iter()
                        .map(|bytes| decode(bytes))
                        .collect::<anyhow::Result<_>>()?,
                })
            })()
            .map_err(|error: anyhow::Error| error.to_string())
        })
        .as_ref()
        .map_err(|error| anyhow::anyhow!("failed to decode hand icons: {error}"))
}

fn stock_cell_matches(image: &ImageData, original: &ImageData, x: u32) -> bool {
    if image.width() != original.width() || image.height() != original.height() {
        return false;
    }
    (0..64).all(|row| {
        let start = ((row * image.width() + x) * 4) as usize;
        let end = start + 64 * 4;
        image.pixels()[start..end] == original.pixels()[start..end]
    })
}

fn install_sheet(image: &mut ImageData, icons: &HandIcons) {
    for (phase, replacement) in icons.replacements.iter().enumerate() {
        let source = [phase as u32 * 64, 0, 64, 64];
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
    if let Some(image) = hud.hand.as_mut() {
        install_sheet(image, prepared_icons()?);
    }
    Ok(())
}

pub(crate) fn is_installed(hud: &HudGraphics) -> bool {
    hud.hand
        .as_ref()
        .and_then(|image| image.region_replacement([0, 0, 64, 64]))
        .is_some()
}
