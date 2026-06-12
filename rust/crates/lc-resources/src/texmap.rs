//! `TexMap.txt` — the static-map material/texture table (C4TextureMap):
//! each map pixel's palette index (low 7 bits; bit 0x80 = IFT/underground)
//! selects a `Material-Texture` pair. Scenario-local Material.c4g groups
//! carry their own table with `OverloadMaterials`/`OverloadTextures`
//! directives controlling whether the global definitions also load.

/// One `index=Material-Texture` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TexMapEntry {
    pub material: String,
    pub texture: String,
}

#[derive(Debug, Clone, Default)]
pub struct TextureMap {
    /// Entries by texmap index (0..128). Index 0 is sky (never mapped).
    entries: Vec<Option<TexMapEntry>>,
    pub overload_materials: bool,
    pub overload_textures: bool,
}

/// The IFT (underground/background) bit on map pixel bytes
/// ("Index +128 for underground materials", TexMap.txt header).
pub const IFT_BIT: u8 = 0x80;

impl TextureMap {
    /// Parse a TexMap.txt source. Unknown or malformed lines are skipped
    /// (the C++ loader logs and continues); `#` starts a comment.
    pub fn parse(source: &str) -> Self {
        let mut map = Self {
            entries: vec![None; 128],
            overload_materials: false,
            overload_textures: false,
        };
        for raw_line in source.lines() {
            let line = raw_line.trim().trim_start_matches('\u{feff}').trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            // C++ matches the directives by PREFIX on non-entry lines
            // (C4Texture.cpp:220-221).
            if line.starts_with("OverloadMaterials") {
                map.overload_materials = true;
                continue;
            }
            if line.starts_with("OverloadTextures") {
                map.overload_textures = true;
                continue;
            }
            let Some((index_text, pair)) = line.split_once('=') else {
                continue;
            };
            let Ok(index) = index_text.trim().parse::<u8>() else {
                continue;
            };
            // AddEntry rejects index 0 (sky) and >= C4M_MaxTexIndex=127
            // (C4Texture.cpp:116-119; C4Constants.h:60).
            if index == 0 || index as usize >= 127 {
                continue;
            }
            let Some((material, texture)) = pair.trim().split_once('-') else {
                continue;
            };
            map.entries[index as usize] = Some(TexMapEntry {
                material: material.trim().to_string(),
                texture: texture.trim().to_string(),
            });
        }
        map
    }

    pub fn entry(&self, index: u8) -> Option<&TexMapEntry> {
        self.entries.get((index & !IFT_BIT) as usize)?.as_ref()
    }

    /// The material name a map pixel byte selects (IFT bit stripped);
    /// `None` for sky (index 0) and unmapped indices.
    pub fn material_for_pixel(&self, pixel: u8) -> Option<&str> {
        self.entry(pixel).map(|entry| entry.material.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_entries_comments_and_overload_directives() {
        let source = "# Static Map Material/Texture Table\n\
                      # Index +128 for underground materials\n\
                      \n\
                      OverloadMaterials\n\
                      OverloadTextures\n\
                      20=Water-Liquid\n\
                      25=Water-Smooth\n\
                      30=Earth-Smooth\n\
                      94=Granite-Smooth\n";
        let map = TextureMap::parse(source);
        assert!(map.overload_materials);
        assert!(map.overload_textures);
        assert_eq!(
            map.entry(20),
            Some(&TexMapEntry {
                material: "Water".into(),
                texture: "Liquid".into()
            })
        );
        assert_eq!(map.material_for_pixel(30), Some("Earth"));
        assert_eq!(map.material_for_pixel(0), None, "index 0 is sky");
        assert_eq!(map.material_for_pixel(99), None, "unmapped index");
    }

    #[test]
    fn ift_bit_strips_to_the_same_entry() {
        // "Index +128 for underground materials": pixel 25|0x80 is the
        // SAME Water entry, marked underground.
        let map = TextureMap::parse("25=Water-Smooth\n");
        assert_eq!(map.material_for_pixel(25 | IFT_BIT), Some("Water"));
    }

    #[test]
    fn malformed_lines_are_skipped() {
        let map = TextureMap::parse("abc=Nope-X\n300=Out-Range\n31=Bare\n32=Earth-Rough\n");
        assert_eq!(map.material_for_pixel(31), None);
        assert_eq!(map.material_for_pixel(32), Some("Earth"));
    }

    #[test]
    fn rejects_sky_and_reserved_indices_like_cpp() {
        // AddEntry fails for index 0 and >= C4M_MaxTexIndex=127
        // (C4Texture.cpp:116-119): byte 127 and 255 stay sky.
        let map = TextureMap::parse("0=Sky-Hack\n126=Earth-Rough\n127=Diff-Reserved\n");
        assert_eq!(map.material_for_pixel(0), None);
        assert_eq!(map.material_for_pixel(126), Some("Earth"));
        assert_eq!(map.material_for_pixel(127), None);
        assert_eq!(map.material_for_pixel(255), None, "255 = 127|IFT stays sky");
    }
}
