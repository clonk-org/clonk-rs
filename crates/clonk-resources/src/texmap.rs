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
    loaded_entry_count: usize,
    pub overload_materials: bool,
    pub overload_textures: bool,
}

/// The IFT (underground/background) bit on map pixel bytes
/// ("Index +128 for underground materials", TexMap.txt header).
pub const IFT_BIT: u8 = 0x80;

fn parse_decimal_prefix(bytes: &[u8]) -> Option<i32> {
    let mut cursor = 0;
    while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
        cursor += 1;
    }
    let negative = match bytes.get(cursor) {
        Some(b'-') => {
            cursor += 1;
            true
        }
        Some(b'+') => {
            cursor += 1;
            false
        }
        _ => false,
    };
    let start = cursor;
    let mut value = 0i64;
    while let Some(digit) = bytes.get(cursor).and_then(|byte| byte.checked_sub(b'0')) {
        if digit > 9 {
            break;
        }
        value = value.checked_mul(10)?.checked_add(i64::from(digit))?;
        cursor += 1;
    }
    if cursor == start {
        return None;
    }
    let value = if negative {
        value.checked_neg()?
    } else {
        value
    };
    i32::try_from(value).ok()
}

impl TextureMap {
    /// Parse the first material group's TexMap.txt through
    /// `C4TextureMap::LoadMap`. Unknown or malformed lines are skipped; only
    /// a `#` in column zero suppresses entry parsing.
    pub fn parse(source: &str) -> Self {
        Self::parse_bytes(&clonk_script::c4_string_bytes(source))
    }

    /// Parse the raw `LoadEntry` buffer without replacing native bytes.
    pub fn parse_bytes(source: &[u8]) -> Self {
        let mut map = Self {
            entries: vec![None; 128],
            loaded_entry_count: 0,
            overload_materials: false,
            overload_textures: false,
        };
        let source = source.split(|byte| *byte == 0).next().unwrap_or_default();
        for raw_line in source.split(|byte| *byte == b'\n') {
            // SCopySegment writes at most 100 bytes into szLine.
            let raw_line = &raw_line[..raw_line.len().min(100)];
            // LoadMap decides between entries and flags before removing CR:
            // entries require exactly one '=' and a non-comment first byte.
            if raw_line.first() == Some(&b'#')
                || raw_line.iter().filter(|byte| **byte == b'=').count() != 1
            {
                if raw_line.starts_with(b"OverloadMaterials") {
                    map.overload_materials = true;
                }
                if raw_line.starts_with(b"OverloadTextures") {
                    map.overload_textures = true;
                }
                continue;
            }
            // SReplaceChar(line, '\r', '\0') makes the first CR terminate all
            // subsequent C-string operations.
            let line = raw_line
                .split(|byte| *byte == b'\r')
                .next()
                .unwrap_or_default();
            let Some(equals) = line.iter().position(|byte| *byte == b'=') else {
                continue;
            };
            let (index_text, pair) = (&line[..equals], &line[equals + 1..]);
            let Some(index) = parse_decimal_prefix(index_text) else {
                continue;
            };
            // AddEntry rejects index 0 (sky) and >= C4M_MaxTexIndex=127
            // (C4Texture.cpp:116-119; C4Constants.h:60).
            if !(1..127).contains(&index) {
                continue;
            }
            let Some(hyphen) = pair.iter().position(|byte| *byte == b'-') else {
                continue;
            };
            map.entries[index as usize] = Some(TexMapEntry {
                material: clonk_script::c4_string_from_bytes(&pair[..hyphen]),
                texture: clonk_script::c4_string_from_bytes(&pair[hyphen + 1..]),
            });
            map.loaded_entry_count += 1;
        }
        map
    }

    /// Read continuation flags from a later material group through
    /// `C4TextureMap::LoadFlags`. Unlike [`Self::parse`], this is a raw
    /// prefix scan and deliberately accepts suffixes such as `=1`.
    pub fn parse_flags(source: &str) -> Self {
        Self::parse_flags_bytes(&clonk_script::c4_string_bytes(source))
    }

    /// Read continuation flags from native `LoadEntryString` bytes.
    pub fn parse_flags_bytes(source: &[u8]) -> Self {
        let mut map = Self {
            entries: vec![None; 128],
            loaded_entry_count: 0,
            overload_materials: false,
            overload_textures: false,
        };
        let source = source.split(|byte| *byte == 0).next().unwrap_or_default();
        for raw_line in source.split(|byte| *byte == b'\n') {
            let line = raw_line
                .iter()
                .position(|byte| *byte != b'\r' && *byte != b'\n')
                .map_or(&[][..], |start| &raw_line[start..]);
            if line.starts_with(b"OverloadMaterials") {
                map.overload_materials = true;
            }
            if line.starts_with(b"OverloadTextures") {
                map.overload_textures = true;
            }
        }
        map
    }

    pub fn entry(&self, index: u8) -> Option<&TexMapEntry> {
        self.entries.get((index & !IFT_BIT) as usize)?.as_ref()
    }

    /// Number returned by C4TextureMap::LoadMap before Init removes invalid
    /// mappings or CrossMapMaterials allocates dynamic ones.
    pub fn loaded_entry_count(&self) -> usize {
        self.loaded_entry_count
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

    #[test]
    fn loaded_entry_count_includes_replaced_slots() {
        // Pinned C++ oracle: src/C4Texture.cpp:117-137,198-227 increments
        // LoadMap's result after every successful AddEntry, including a
        // later source line that replaces an occupied numeric slot.
        let map = TextureMap::parse("20=Water-Liquid\n20=Earth-Rough\n");

        assert_eq!(map.loaded_entry_count(), 2);
        assert_eq!(map.material_for_pixel(20), Some("Earth"));
    }

    #[test]
    fn load_map_requires_one_equals_before_checking_flag_prefixes() {
        let map = TextureMap::parse("20=Water-Sm=ooth\nOverloadMaterials=1\nOverloadTextures==1\n");
        assert!(map.entry(20).is_none());
        assert!(!map.overload_materials);
        assert!(
            map.overload_textures,
            "two equals select LoadMap's prefix-based flag branch"
        );
    }

    #[test]
    fn load_map_preserves_mapping_whitespace_and_does_not_strip_bom() {
        let map =
            TextureMap::parse("\u{feff}29=Earth-Rough\n30= Earth-Smooth\n31=Earth - Smooth \n");
        assert!(
            map.entry(29).is_none(),
            "a BOM makes strtol return index zero"
        );
        assert_eq!(
            map.entry(30),
            Some(&TexMapEntry {
                material: " Earth".into(),
                texture: "Smooth".into(),
            })
        );
        assert_eq!(
            map.entry(31),
            Some(&TexMapEntry {
                material: "Earth ".into(),
                texture: " Smooth ".into(),
            })
        );
    }

    #[test]
    fn load_map_index_uses_the_strtol_decimal_prefix() {
        let map = TextureMap::parse(" 30 junk=Earth-Smooth\n+31=Rock-Rough\n0x20=Gold-Rough\n");
        assert_eq!(map.material_for_pixel(30), Some("Earth"));
        assert_eq!(map.material_for_pixel(31), Some("Rock"));
        assert!(map.entry(32).is_none());
    }

    #[test]
    fn texmap_load_map_truncates_each_line_to_100_bytes_like_cpp() {
        let mut source = Vec::new();

        // The equals sign is the 101st byte and therefore invisible.
        source.extend_from_slice(&[b' '; 98]);
        source.extend_from_slice(b"20=Earth-Smooth\n");

        // The first 100 bytes form a complete entry; its physical-line suffix
        // neither extends the texture name nor becomes another segment.
        source.extend_from_slice(b"21=Earth-");
        source.extend(vec![b'R'; 91]);
        source.extend_from_slice(b"ignored suffix\n");

        // The second equals is likewise invisible, so this remains the
        // one-equals entry branch rather than completing an overload flag.
        source.extend_from_slice(b"OverloadTextures=");
        source.extend(vec![b' '; 83]);
        source.extend_from_slice(b"=\n");

        // Conversely, an equals beyond the cap cannot suppress a flag that
        // has no equals in its visible prefix.
        source.extend_from_slice(b"OverloadMaterials");
        source.extend(vec![b' '; 83]);
        source.extend_from_slice(b"=1\n");

        // The cap resets at the real LF, not after each 100-byte chunk.
        source.extend_from_slice(b"22=Water-Liquid\n");

        let map = TextureMap::parse_bytes(&source);
        assert!(map.entry(20).is_none());
        assert_eq!(
            map.entry(21),
            Some(&TexMapEntry {
                material: "Earth".into(),
                texture: "R".repeat(91),
            })
        );
        assert!(map.overload_materials);
        assert!(!map.overload_textures);
        assert_eq!(map.material_for_pixel(22), Some("Water"));
    }

    #[test]
    fn later_group_load_flags_keeps_raw_prefix_semantics() {
        let map = TextureMap::parse_flags(
            "OverloadMaterials=1\nOverloadTextures suffix\n30=Earth-Smooth\n",
        );
        assert!(map.overload_materials);
        assert!(map.overload_textures);
        assert!(
            map.entry(30).is_none(),
            "LoadFlags never parses table entries"
        );
    }

    #[test]
    fn shipped_global_texmap_retains_all_well_formed_entries() {
        let map = TextureMap::parse(include_str!("../../../content/Material.c4g/TEXMAP.TXT"));
        assert_eq!(map.entries.iter().flatten().count(), 48);
        assert_eq!(
            map.entry(10),
            Some(&TexMapEntry {
                material: "Tunnel".into(),
                texture: "Smooth".into(),
            })
        );
        assert_eq!(
            map.entry(81),
            Some(&TexMapEntry {
                material: "FlySand".into(),
                texture: "Smooth3".into(),
            })
        );
    }
}
