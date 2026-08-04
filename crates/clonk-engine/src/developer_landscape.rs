//! Read-only landscape-tool model for the developer console.
//!
//! `C4ToolsDlg::InitMaterialCtrls` (`C4ToolsDlg.cpp:482-508`) lists `Sky`
//! followed by the material map in order; `UpdateTextures` (:510-560) lists the
//! *invalid* material/texture pairs first and the valid ones above them, and
//! treats every texture as valid in Exact mode. `AssertValidTexture`
//! (:965-983) corrects an invalid selection in Static mode only.
//! `C4EditCursor::ApplyToolPicker` (`C4EditCursor.cpp:698-731`) samples the
//! retained map through `MapZoom` in Static mode and the live landscape in
//! Exact mode.
//!
//! This module is the read side only — dialog state, rendering, shortcuts and
//! `EMDrawTool` emission stay out — they belong to the Tools/draw dialog
//! (`C4ToolsDlg`), tracked in `PORT_STATUS.md`.

/// `C4TLS_MatSky` (`C4ToolsDlg.h:43`): the sky pseudo-material heading the
/// material list, and the picker's answer for an empty pixel.
pub const TOOL_SKY_MATERIAL: &str = "Sky";

/// `IFT` (`C4Landscape.h:31`): the pixel's in-front-of-tunnel bit.
pub const IFT_BIT: u8 = 0x80;

/// A texture as the native combo box would list it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolTextureEntry {
    pub name: String,
    /// Whether `TextureMap::GetIndex(material, texture, false)` resolves. Exact
    /// mode reports every texture valid (`C4ToolsDlg.cpp:546`).
    pub valid: bool,
}

/// The texture-map slots the tool reads. Mirrors the runtime tex-map's parallel
/// slot arrays without borrowing its private representation.
#[derive(Clone, Copy, Debug)]
pub struct ToolTexMapView<'a> {
    /// Per-slot material name, indexed by tex-map index.
    pub material_names: &'a [Option<String>],
    /// Per-slot texture name, indexed by tex-map index.
    pub texture_names: &'a [Option<String>],
    /// Every texture the scenario loaded, in load order.
    pub texture_inventory: &'a [String],
}

impl ToolTexMapView<'_> {
    /// `C4TextureMap::GetIndex(material, texture, false)` — the slot carrying
    /// this pair, without creating one. Slot 0 is sky and never matches.
    pub fn index_of(&self, material: &str, texture: &str) -> Option<u8> {
        (1..self.material_names.len().min(self.texture_names.len()))
            .find(|&slot| {
                self.material_names[slot].as_deref() == Some(material)
                    && self.texture_names[slot].as_deref() == Some(texture)
            })
            .and_then(|slot| u8::try_from(slot).ok())
    }

    /// `C4TextureMap::GetEntry(index)` — the pair a tex-map index names.
    pub fn entry(&self, index: u8) -> Option<(&str, &str)> {
        let slot = usize::from(index);
        let material = self.material_names.get(slot)?.as_deref()?;
        let texture = self.texture_names.get(slot)?.as_deref()?;
        Some((material, texture))
    }

    fn pair_is_valid(&self, material: &str, texture: &str) -> bool {
        self.index_of(material, texture).is_some()
    }
}

/// The material list: `Sky`, then the material map in its own order
/// (`C4ToolsDlg.cpp:486-489`).
pub fn tool_material_catalog(material_map_names: &[String]) -> Vec<String> {
    std::iter::once(TOOL_SKY_MATERIAL.to_owned())
        .chain(material_map_names.iter().cloned())
        .collect()
}

/// The texture list for `material`, in native combo-box order: the invalid
/// entries occupy the bottom and the valid ones sit above them
/// (`C4ToolsDlg.cpp:517-548`). Exact mode contributes no invalid section,
/// because every texture is selectable there (:519,:546).
pub fn tool_texture_catalog(
    texmap: ToolTexMapView<'_>,
    material: &str,
    landscape_mode: i32,
) -> Vec<ToolTextureEntry> {
    let exact = landscape_mode == crate::landscape::LANDSCAPE_MODE_EXACT;
    let entry = |name: &String, valid: bool| ToolTextureEntry {
        name: name.clone(),
        valid,
    };
    let invalid = texmap
        .texture_inventory
        .iter()
        .filter(|texture| !exact && !texmap.pair_is_valid(material, texture))
        .map(|texture| entry(texture, false));
    let valid = texmap
        .texture_inventory
        .iter()
        .filter(|texture| exact || texmap.pair_is_valid(material, texture))
        .map(|texture| entry(texture, true));
    invalid.chain(valid).collect()
}

/// `C4ToolsDlg::AssertValidTexture` (`C4ToolsDlg.cpp:965-983`): in Static mode
/// only, an invalid material/texture pair falls back to the first texture that
/// does pair with the material. `None` means the selection stands.
pub fn corrected_tool_texture(
    texmap: ToolTexMapView<'_>,
    material: &str,
    texture: &str,
    landscape_mode: i32,
) -> Option<String> {
    // Static map mode only, and sky is exempt (:967-970).
    if landscape_mode != crate::landscape::LANDSCAPE_MODE_STATIC || material == TOOL_SKY_MATERIAL {
        return None;
    }
    if texmap.pair_is_valid(material, texture) {
        return None;
    }
    texmap
        .texture_inventory
        .iter()
        .find(|candidate| texmap.pair_is_valid(material, candidate))
        .cloned()
}

/// What the tool picker selected (`C4EditCursor::ApplyToolPicker`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ToolPick {
    /// An empty pixel, or a material the map does not resolve: C++ selects the
    /// sky pseudo-material and leaves texture and IFT alone
    /// (`C4EditCursor.cpp:715-717,727-729`).
    Sky,
    /// Static mode resolves a full pair plus the IFT bit (:706-714).
    MaterialTexture {
        material: String,
        texture: String,
        ift: bool,
    },
    /// Exact mode has only a material and the live IFT bit (:722-726).
    Material { material: String, ift: bool },
}

/// The map coordinate a landscape pixel samples in Static mode:
/// `GetMapIndex(X / MapZoom, Y / MapZoom)` (`C4EditCursor.cpp:706`). A
/// non-positive zoom cannot divide, so nothing is sampled.
pub fn static_pick_map_coordinates(x: i32, y: i32, map_zoom: i32) -> Option<(i32, i32)> {
    (map_zoom > 0).then(|| (x / map_zoom, y / map_zoom))
}

/// Static mode: decode a retained map byte (`C4EditCursor.cpp:706-717`). A zero
/// byte is sky; otherwise the low bits index the tex map and the high bit is
/// IFT. A byte whose slot has no entry also yields sky, since C++ leaves the
/// selection untouched only when `GetEntry` succeeds.
pub fn static_tool_pick(texmap: ToolTexMapView<'_>, map_byte: u8) -> ToolPick {
    if map_byte == 0 {
        return ToolPick::Sky;
    }
    // `byIndex & (IFT - 1)` and `byIndex & ~(IFT - 1)` (:708,:712).
    texmap
        .entry(map_byte & (IFT_BIT - 1))
        .map(|(material, texture)| ToolPick::MaterialTexture {
            material: material.to_owned(),
            texture: texture.to_owned(),
            ift: map_byte & IFT_BIT != 0,
        })
        .unwrap_or(ToolPick::Sky)
}

/// Exact mode: the live material and IFT bit (`C4EditCursor.cpp:721-729`). An
/// invalid material — `MatValid` failing, which the port models as no material
/// at that pixel — resolves to sky.
pub fn exact_tool_pick(material: Option<&str>, ift: bool) -> ToolPick {
    material.map_or(ToolPick::Sky, |material| ToolPick::Material {
        material: material.to_owned(),
        ift,
    })
}

/// Everything the native material/texture controls need to populate and enable
/// themselves (`C4ToolsDlg.cpp:482-508,796-940`). Owned, so the console can
/// hold it without borrowing the engine.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeveloperLandscapeToolState {
    /// `Game.Landscape.Mode` — one of the `LANDSCAPE_MODE_*` constants.
    pub mode: i32,
    /// `Game.Landscape.MapZoom`, absent when the landscape keeps no raster.
    pub map_zoom: Option<i32>,
    /// Whether a retained map exists, which is what gates the Static controls.
    pub has_map: bool,
    /// The material map in its own order, without the `Sky` entry.
    pub material_map_names: Vec<String>,
    /// Per-slot tex-map material names.
    pub material_names: Vec<Option<String>>,
    /// Per-slot tex-map texture names.
    pub texture_names: Vec<Option<String>>,
    /// Every loaded texture, in load order.
    pub texture_inventory: Vec<String>,
}

impl DeveloperLandscapeToolState {
    /// Borrowed view for the catalog and picker helpers.
    pub fn texmap(&self) -> ToolTexMapView<'_> {
        ToolTexMapView {
            material_names: &self.material_names,
            texture_names: &self.texture_names,
            texture_inventory: &self.texture_inventory,
        }
    }

    /// The material combo's contents (`C4ToolsDlg.cpp:486-489`).
    pub fn material_catalog(&self) -> Vec<String> {
        tool_material_catalog(&self.material_map_names)
    }

    /// The texture combo's contents for `material` (`C4ToolsDlg.cpp:517-548`).
    pub fn texture_catalog(&self, material: &str) -> Vec<ToolTextureEntry> {
        tool_texture_catalog(self.texmap(), material, self.mode)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::landscape::{LANDSCAPE_MODE_DYNAMIC, LANDSCAPE_MODE_EXACT, LANDSCAPE_MODE_STATIC};

    /// Slots 1..3 of a tex map: Earth-Smooth, Earth-Rough, Water-Smooth.
    fn fixture() -> (Vec<Option<String>>, Vec<Option<String>>, Vec<String>) {
        let material_names = vec![
            None,
            Some("Earth".to_owned()),
            Some("Earth".to_owned()),
            Some("Water".to_owned()),
        ];
        let texture_names = vec![
            None,
            Some("Smooth".to_owned()),
            Some("Rough".to_owned()),
            Some("Smooth".to_owned()),
        ];
        // Load order, including a texture that pairs with nothing.
        let inventory = vec![
            "Smooth".to_owned(),
            "Rough".to_owned(),
            "Unpaired".to_owned(),
        ];
        (material_names, texture_names, inventory)
    }

    // C4ToolsDlg.cpp:486-489,517-548,965-983.
    #[test]
    fn developer_landscape_tool_catalog_partitions_valid_pairs() {
        let (materials, textures, inventory) = fixture();
        let texmap = ToolTexMapView {
            material_names: &materials,
            texture_names: &textures,
            texture_inventory: &inventory,
        };

        // Sky heads the material list, then the map in its own order (:486-489).
        assert_eq!(
            tool_material_catalog(&["Earth".to_owned(), "Water".to_owned()]),
            vec!["Sky".to_owned(), "Earth".to_owned(), "Water".to_owned()]
        );

        // Static: invalid at the bottom, valid above (:517-548). Earth pairs
        // with Smooth and Rough but not Unpaired.
        assert_eq!(
            tool_texture_catalog(texmap, "Earth", LANDSCAPE_MODE_STATIC),
            vec![
                ToolTextureEntry {
                    name: "Unpaired".to_owned(),
                    valid: false
                },
                ToolTextureEntry {
                    name: "Smooth".to_owned(),
                    valid: true
                },
                ToolTextureEntry {
                    name: "Rough".to_owned(),
                    valid: true
                },
            ]
        );
        // Water pairs with Smooth only, so two entries fall to the bottom.
        assert_eq!(
            tool_texture_catalog(texmap, "Water", LANDSCAPE_MODE_STATIC)
                .iter()
                .map(|entry| (entry.name.as_str(), entry.valid))
                .collect::<Vec<_>>(),
            vec![("Rough", false), ("Unpaired", false), ("Smooth", true)]
        );
        // Exact treats every texture as selectable, so nothing is partitioned
        // out and the inventory keeps its load order (:519,:546).
        assert_eq!(
            tool_texture_catalog(texmap, "Water", LANDSCAPE_MODE_EXACT),
            inventory
                .iter()
                .map(|name| ToolTextureEntry {
                    name: name.clone(),
                    valid: true
                })
                .collect::<Vec<_>>()
        );

        // The index/entry round trip C++ reads pairs through.
        assert_eq!(texmap.index_of("Earth", "Rough"), Some(2));
        assert_eq!(texmap.index_of("Water", "Rough"), None);
        assert_eq!(texmap.entry(3), Some(("Water", "Smooth")));
        assert_eq!(texmap.entry(9), None);

        // AssertValidTexture: Static only, sky exempt, first valid wins.
        assert_eq!(
            corrected_tool_texture(texmap, "Water", "Rough", LANDSCAPE_MODE_STATIC).as_deref(),
            Some("Smooth")
        );
        assert_eq!(
            corrected_tool_texture(texmap, "Earth", "Rough", LANDSCAPE_MODE_STATIC),
            None,
            "a valid pair is left alone"
        );
        assert_eq!(
            corrected_tool_texture(texmap, "Water", "Rough", LANDSCAPE_MODE_EXACT),
            None,
            "correction is Static-mode only (:967-968)"
        );
        assert_eq!(
            corrected_tool_texture(texmap, "Water", "Rough", LANDSCAPE_MODE_DYNAMIC),
            None
        );
        assert_eq!(
            corrected_tool_texture(texmap, "Sky", "Rough", LANDSCAPE_MODE_STATIC),
            None,
            "sky is exempt (:969-970)"
        );
    }

    // C4EditCursor.cpp:698-731.
    #[test]
    fn developer_landscape_picker_reads_static_mapzoom_and_exact_ift() {
        let (materials, textures, inventory) = fixture();
        let texmap = ToolTexMapView {
            material_names: &materials,
            texture_names: &textures,
            texture_inventory: &inventory,
        };

        // Static samples the map at X/MapZoom, Y/MapZoom (:706).
        assert_eq!(static_pick_map_coordinates(45, 82, 10), Some((4, 8)));
        assert_eq!(static_pick_map_coordinates(9, 9, 10), Some((0, 0)));
        assert_eq!(static_pick_map_coordinates(45, 82, 0), None);

        // A zero byte is sky; the selection keeps its texture and IFT (:715-716).
        assert_eq!(static_tool_pick(texmap, 0), ToolPick::Sky);

        // The low bits index the tex map, the high bit is IFT (:708-712).
        assert_eq!(
            static_tool_pick(texmap, 2),
            ToolPick::MaterialTexture {
                material: "Earth".to_owned(),
                texture: "Rough".to_owned(),
                ift: false,
            }
        );
        assert_eq!(
            static_tool_pick(texmap, 2 | IFT_BIT),
            ToolPick::MaterialTexture {
                material: "Earth".to_owned(),
                texture: "Rough".to_owned(),
                ift: true,
            }
        );
        // A byte naming an empty slot resolves to sky rather than a half pair.
        assert_eq!(static_tool_pick(texmap, 9), ToolPick::Sky);
        assert_eq!(static_tool_pick(texmap, 9 | IFT_BIT), ToolPick::Sky);

        // Exact carries a material and the live IFT bit, with no texture (:722-726).
        assert_eq!(
            exact_tool_pick(Some("Earth"), true),
            ToolPick::Material {
                material: "Earth".to_owned(),
                ift: true,
            }
        );
        assert_eq!(
            exact_tool_pick(Some("Earth"), false),
            ToolPick::Material {
                material: "Earth".to_owned(),
                ift: false,
            }
        );
        // An invalid material resolves to sky (:727-728).
        assert_eq!(exact_tool_pick(None, true), ToolPick::Sky);
    }
}
