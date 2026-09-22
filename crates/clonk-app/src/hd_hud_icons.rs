//! Approved high-resolution replacements for standalone HUD images.

use clonk_frontend::HudGraphics;
use clonk_gui::ImageData;
use std::{collections::HashMap, sync::OnceLock};

const ICONS: &[(&str, &[u8], &[u8])] = &[
    (
        "Build.png",
        include_bytes!("../../../planet/Graphics.c4g/Build.png"),
        include_bytes!("../assets/hud-icons/build.png"),
    ),
    (
        "Captain.png",
        include_bytes!("../../../planet/Graphics.c4g/Captain.png"),
        include_bytes!("../assets/hud-icons/captain.png"),
    ),
    (
        "Construction.png",
        include_bytes!("../../../planet/Graphics.c4g/Construction.png"),
        include_bytes!("../assets/hud-icons/construction.png"),
    ),
    (
        "Energy.png",
        include_bytes!("../../../planet/Graphics.c4g/Energy.png"),
        include_bytes!("../assets/hud-icons/energy.png"),
    ),
    (
        "Exit.png",
        include_bytes!("../../../planet/Graphics.c4g/Exit.png"),
        include_bytes!("../assets/hud-icons/exit.png"),
    ),
    (
        "Magic.png",
        include_bytes!("../../../planet/Graphics.c4g/Magic.png"),
        include_bytes!("../assets/hud-icons/magic.png"),
    ),
    (
        "Player.png",
        include_bytes!("../../../planet/Graphics.c4g/Player.png"),
        include_bytes!("../assets/hud-icons/player.png"),
    ),
    (
        "Score.png",
        include_bytes!("../../../planet/Graphics.c4g/Score.png"),
        include_bytes!("../assets/hud-icons/score.png"),
    ),
    (
        "Wealth.png",
        include_bytes!("../../../planet/Graphics.c4g/Wealth.png"),
        include_bytes!("../assets/hud-icons/wealth.png"),
    ),
];

struct HudIcon {
    name: &'static str,
    original: ImageData,
    replacement: ImageData,
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

fn prepared_icons() -> anyhow::Result<&'static Vec<HudIcon>> {
    static ICONS_CACHE: OnceLock<Result<Vec<HudIcon>, String>> = OnceLock::new();
    ICONS_CACHE
        .get_or_init(|| {
            ICONS
                .iter()
                .map(|&(name, original, replacement)| {
                    Ok(HudIcon {
                        name,
                        original: decode(original)?,
                        replacement: decode(replacement)?,
                    })
                })
                .collect::<anyhow::Result<Vec<_>>>()
                .map_err(|error| error.to_string())
        })
        .as_ref()
        .map_err(|error| anyhow::anyhow!("failed to decode embedded HUD icons: {error}"))
}

fn install_one(image: &mut ImageData, icon: &HudIcon) {
    let source = [0, 0, icon.original.width(), icon.original.height()];
    if image.width() == icon.original.width()
        && image.height() == icon.original.height()
        && image.pixels() == icon.original.pixels()
        && image.region_replacement(source).is_none()
    {
        *image = image
            .clone()
            .with_region_replacement(source, icon.replacement.clone());
    }
}

pub(crate) fn install_hud(hud: &mut HudGraphics) -> anyhow::Result<()> {
    for icon in prepared_icons()? {
        let target = match icon.name {
            "Build.png" => hud.build.as_mut(),
            "Captain.png" => hud.captain.as_mut(),
            "Construction.png" => hud.construction.as_mut(),
            "Energy.png" => hud.energy.as_mut(),
            "Exit.png" => hud.exit.as_mut(),
            "Magic.png" => hud.magic.as_mut(),
            "Player.png" => hud.player.as_mut(),
            "Score.png" => hud.score.as_mut(),
            "Wealth.png" => hud.wealth.as_mut(),
            _ => None,
        };
        if let Some(target) = target {
            install_one(target, icon);
        }
    }
    Ok(())
}

pub(crate) fn install(
    hud: &mut HudGraphics,
    dialog_images: &mut HashMap<String, ImageData>,
) -> anyhow::Result<()> {
    install_hud(hud)?;
    if let (Some(player), Some(icon)) = (
        dialog_images.get_mut("Player.png"),
        prepared_icons()?
            .iter()
            .find(|icon| icon.name == "Player.png"),
    ) {
        install_one(player, icon);
    }
    Ok(())
}

pub(crate) fn is_installed(hud: &HudGraphics) -> bool {
    [
        (hud.build.as_ref(), [0, 0, 64, 64]),
        (hud.captain.as_ref(), [0, 0, 16, 16]),
        (hud.construction.as_ref(), [0, 0, 16, 16]),
        (hud.energy.as_ref(), [0, 0, 11, 17]),
        (hud.exit.as_ref(), [0, 0, 64, 64]),
        (hud.magic.as_ref(), [0, 0, 25, 35]),
        (hud.player.as_ref(), [0, 0, 48, 48]),
        (hud.score.as_ref(), [0, 0, 60, 30]),
        (hud.wealth.as_ref(), [0, 0, 60, 30]),
    ]
    .into_iter()
    .any(|(image, source)| {
        image
            .and_then(|image| image.region_replacement(source))
            .is_some_and(|art| art.width() == source[2] * 8 && art.height() == source[3] * 8)
    })
}

#[cfg(all(
    test,
    any(not(feature = "app-test-shard-mode"), feature = "app-test-shard-5")
))]
mod tests {
    use super::*;

    #[test]
    fn custom_build_image_does_not_disable_other_stock_hud_icons() {
        let captain = prepared_icons()
            .unwrap()
            .iter()
            .find(|icon| icon.name == "Captain.png")
            .unwrap();
        let mut hud = HudGraphics {
            build: Some(ImageData::new(64, 64, [7, 8, 9, 255].repeat(64 * 64))),
            captain: Some(captain.original.clone()),
            ..HudGraphics::default()
        };
        install_hud(&mut hud).unwrap();
        assert!(is_installed(&hud));
        assert!(hud
            .build
            .as_ref()
            .unwrap()
            .region_replacement([0, 0, 64, 64])
            .is_none());
    }
}
