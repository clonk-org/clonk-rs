//! Approved high-resolution replacements addressed in the original UI sheets.

use clonk_gui::ImageData;
use std::{collections::HashMap, sync::OnceLock};

struct IconSheet {
    name: &'static str,
    original: ImageData,
    replacement: ImageData,
}

pub(crate) const ICONS: &[(&str, [u32; 4], &[u8])] = &[
    (
        "GUIIcons2.png",
        [64, 256, 64, 64],
        include_bytes!("../assets/ui-icons/voice-chat.png"),
    ),
    (
        "GUIIcons2.png",
        [0, 256, 64, 64],
        include_bytes!("../assets/ui-icons/game-list.png"),
    ),
    (
        "GUIIcons.png",
        [160, 40, 40, 40],
        include_bytes!("../assets/ui-icons/folder.png"),
    ),
    (
        "GUIIcons2.png",
        [128, 192, 64, 64],
        include_bytes!("../assets/ui-icons/parent-folder.png"),
    ),
    (
        "GUIIcons.png",
        [40, 80, 40, 40],
        include_bytes!("../assets/ui-icons/save.png"),
    ),
    (
        "GUIIcons.png",
        [40, 0, 40, 40],
        include_bytes!("../assets/ui-icons/notify.png"),
    ),
    (
        "GUIIcons.png",
        [200, 40, 40, 40],
        include_bytes!("../assets/ui-icons/error.png"),
    ),
    (
        "GUIIcons.png",
        [0, 120, 40, 40],
        include_bytes!("../assets/ui-icons/confirm.png"),
    ),
    (
        "GUIIcons.png",
        [160, 200, 40, 40],
        include_bytes!("../assets/ui-icons/close.png"),
    ),
    (
        "GUIIcons2.png",
        [0, 0, 64, 64],
        include_bytes!("../assets/ui-icons/record-off.png"),
    ),
    (
        "GUIIcons2.png",
        [64, 0, 64, 64],
        include_bytes!("../assets/ui-icons/record-on.png"),
    ),
    (
        "GUIIcons2.png",
        [192, 128, 64, 64],
        include_bytes!("../assets/ui-icons/locked.png"),
    ),
    (
        "GUIIcons2.png",
        [0, 192, 64, 64],
        include_bytes!("../assets/ui-icons/unlocked.png"),
    ),
    (
        "GUICheckbox.png",
        [0, 0, 32, 32],
        include_bytes!("../assets/ui-icons/checkbox-off.png"),
    ),
    (
        "GUICheckbox.png",
        [32, 0, 32, 32],
        include_bytes!("../assets/ui-icons/checkbox-on.png"),
    ),
    (
        "GUICheckbox.png",
        [64, 0, 32, 32],
        include_bytes!("../assets/ui-icons/checkbox-off.png"),
    ),
    (
        "GUICheckbox.png",
        [96, 0, 32, 32],
        include_bytes!("../assets/ui-icons/checkbox-disabled.png"),
    ),
];

fn decode(bytes: &[u8]) -> anyhow::Result<ImageData> {
    let mut rgba =
        clonk_resources::load_image_from_memory_with_format(bytes, image::ImageFormat::Png)?
            .into_rgba8();
    // GraphicsResource applies the same transparent-RGB normalization when
    // loading the source sheet from disk.
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
                    "GUIIcons.png",
                    include_bytes!("../../../planet/Graphics.c4g/GUIIcons.png").as_slice(),
                ),
                (
                    "GUIIcons2.png",
                    include_bytes!("../../../planet/Graphics.c4g/GUIIcons2.png").as_slice(),
                ),
                (
                    "GUICheckbox.png",
                    include_bytes!("../../../planet/Graphics.c4g/GUICheckbox.png").as_slice(),
                ),
            ]
            .into_iter()
            .map(|(name, bytes)| {
                let original = decode(bytes)?;
                let mut replacement = original.clone();
                for (_, rect, bytes) in ICONS.iter().filter(|(sheet, _, _)| *sheet == name) {
                    replacement = replacement.with_region_replacement(*rect, decode(bytes)?);
                }
                Ok(IconSheet {
                    name,
                    original,
                    replacement,
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()
            .map_err(|error| error.to_string())
        })
        .as_ref()
        .map_err(|error| anyhow::anyhow!("failed to decode embedded UI icons: {error}"))
}

pub(crate) fn install(images: &mut HashMap<String, ImageData>) -> anyhow::Result<()> {
    for sheet in prepared_sheets()? {
        // Custom graphics packs own their artwork. Only the shipped source
        // sheets receive these replacements; all coordinates remain unchanged.
        if let Some(image) = images.get_mut(sheet.name).filter(|image| {
            image.width() == sheet.original.width()
                && image.height() == sheet.original.height()
                && image.pixels() == sheet.original.pixels()
        }) {
            *image = sheet.replacement.clone();
        }
    }
    Ok(())
}

#[cfg(all(
    test,
    any(not(feature = "app-test-shard-mode"), feature = "app-test-shard-5")
))]
mod tests {
    use super::*;

    #[test]
    fn custom_sheets_keep_their_artwork_and_repeated_installation_reuses_textures() {
        let prepared = prepared_sheets().unwrap();
        let mut images: HashMap<_, _> = prepared
            .iter()
            .map(|sheet| (sheet.name.to_string(), sheet.original.clone()))
            .collect();
        let custom = ImageData::new(240, 360, [10, 20, 30, 255].repeat(240 * 360));
        images.insert("GUIIcons.png".to_string(), custom.clone());
        install(&mut images).unwrap();
        assert_eq!(images["GUIIcons.png"], custom);
        let first = images["GUIIcons2.png"].gpu_texture_id();
        install(&mut images).unwrap();
        assert_eq!(images["GUIIcons2.png"].gpu_texture_id(), first);
        assert!(images["GUIIcons2.png"]
            .region_replacement([128, 0, 64, 64])
            .is_none());
    }

    /// Bounds of a cell's opaque, non-black pixels, relative to the cell:
    /// the box itself, without the pure black drop shadow the classic box
    /// casts.
    fn box_bounds(image: &ImageData, [left, top, width, height]: [u32; 4]) -> [u32; 4] {
        (top..top + height)
            .flat_map(|y| (left..left + width).map(move |x| (x, y)))
            .filter(|&(x, y)| {
                let offset = ((y * image.width() + x) * 4) as usize;
                let pixel = &image.pixels()[offset..offset + 4];
                pixel[3] >= 128 && pixel[..3].iter().any(|&channel| channel >= 8)
            })
            .fold([u32::MAX, u32::MAX, 0, 0], |[l, t, r, b], (x, y)| {
                [
                    l.min(x - left),
                    t.min(y - top),
                    r.max(x - left + 1),
                    b.max(y - top + 1),
                ]
            })
    }

    #[test]
    fn checkbox_art_keeps_the_classic_box_bounds_at_its_resolution() {
        // The classic checkmark overhangs its box on every side. The
        // replacement checkmark spans the classic one's extent, so it sits
        // on the box the same way only if the box keeps the classic bounds.
        let icons = prepared_sheets().unwrap();
        let sheet = icons
            .iter()
            .find(|sheet| sheet.name == "GUICheckbox.png")
            .unwrap();
        let classic = box_bounds(&sheet.original, [0, 0, 32, 32]);
        let replacement = sheet
            .replacement
            .region_replacement([0, 0, 32, 32])
            .unwrap();
        let scale = replacement.width() / 32;
        let bounds = box_bounds(
            replacement,
            [0, 0, replacement.width(), replacement.height()],
        );
        assert!(
            classic
                .iter()
                .zip(bounds)
                .all(|(&classic, bound)| (classic * scale).abs_diff(bound) <= scale / 2),
            "the replacement box {bounds:?} must keep the classic box {classic:?} at {scale}x"
        );
    }

    #[test]
    fn checkbox_states_share_identical_uncovered_base_pixels() {
        let icons = prepared_sheets().unwrap();
        let sheet = &icons
            .iter()
            .find(|sheet| sheet.name == "GUICheckbox.png")
            .unwrap()
            .replacement;
        let unchecked = sheet.region_replacement([0, 0, 32, 32]).unwrap();
        for rect in [[32, 0, 32, 32], [96, 0, 32, 32]] {
            let checked = sheet.region_replacement(rect).unwrap();
            // The upper-left inset and bevel are clear of both checkmarks.
            for y in 45..100 {
                for x in 35..100 {
                    let offset = (y * 256 + x) * 4;
                    assert_eq!(
                        &checked.pixels()[offset..offset + 4],
                        &unchecked.pixels()[offset..offset + 4]
                    );
                }
            }
        }
    }
}
