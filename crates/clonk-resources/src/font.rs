use crate::{definition::parse_action_i32_prefix, Group, GroupError};
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum FontResourceError {
    #[error("font resource `{name}` not found")]
    NotFound { name: String },
    #[error(transparent)]
    Group(#[from] GroupError),
}

#[derive(Debug, Clone)]
pub struct FontResource {
    name: String,
    data: Arc<[u8]>,
}

/// One `[Font]` record from `Fonts.txt` (`C4FontDef`, C4Fonts.cpp:28-40).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontDefinition {
    pub name: String,
    pub size: i32,
    pub log_font: String,
    pub small_font: String,
    pub font: String,
    pub caption_font: String,
    pub title_font: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontRole {
    Log,
    MainSmall,
    Main,
    Caption,
    Title,
}

impl FontRole {
    pub fn derived_size(self, base_size: i32) -> i32 {
        match self {
            Self::Log => base_size.saturating_mul(12) / 14,
            Self::MainSmall => base_size.saturating_mul(13) / 14,
            Self::Main => base_size,
            Self::Caption => base_size.saturating_mul(16) / 14,
            Self::Title => base_size.saturating_mul(22) / 14,
        }
    }
}

impl FontDefinition {
    pub fn font_for(&self, role: FontRole) -> &str {
        match role {
            FontRole::Log => &self.log_font,
            FontRole::MainSmall => &self.small_font,
            FontRole::Main => &self.font,
            FontRole::Caption => &self.caption_font,
            FontRole::Title => &self.title_font,
        }
    }
}

#[derive(Debug, Clone)]
struct CatalogVectorFont {
    face: Vec<u8>,
    resource: FontResource,
}

const C4_MAX_NAME_BYTES: usize = 30;
const VECTOR_FONT_EXTENSIONS: [&[u8]; 6] = [b"fon", b"fnt", b"ttf", b"ttc", b"fot", b"otf"];

fn vector_font_face_bytes<'a>(filename: &'a [u8], extension: &[u8]) -> Option<&'a [u8]> {
    let suffix_length = extension.len().checked_add(1)?;
    let suffix_start = filename.len().checked_sub(suffix_length)?;
    let suffix = &filename[suffix_start..];
    (suffix.first() == Some(&b'.') && suffix[1..].eq_ignore_ascii_case(extension))
        .then_some(&filename[..suffix_start])
}

#[derive(Debug, Clone, Default)]
pub struct FontCatalog {
    definitions: Vec<FontDefinition>,
    vector_fonts: Vec<CatalogVectorFont>,
}

#[derive(Debug, Clone)]
pub enum ResolvedFontSpec {
    Vector {
        face: String,
        bytes: Option<Arc<[u8]>>,
        /// Face within a TrueType/OpenType collection. Standalone catalog
        /// fonts use zero; system-family lookup may select another face.
        face_index: u32,
        size: i32,
        weight: u32,
    },
    Bitmap {
        filename: String,
        indent: i32,
    },
}

impl Default for FontDefinition {
    fn default() -> Self {
        Self {
            name: String::new(),
            size: 1,
            log_font: String::new(),
            small_font: String::new(),
            font: String::new(),
            caption_font: String::new(),
            title_font: String::new(),
        }
    }
}

impl FontResource {
    pub fn new(name: impl Into<String>, bytes: Vec<u8>) -> Self {
        Self {
            name: name.into(),
            data: Arc::from(bytes.into_boxed_slice()),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn bytes(&self) -> &[u8] {
        &self.data
    }

    pub fn clone_bytes(&self) -> Arc<[u8]> {
        Arc::clone(&self.data)
    }
}

pub fn load_ttf(group: &Group, name: &str) -> Result<FontResource, FontResourceError> {
    let normalized = name.replace('\\', "/");
    let candidate = Path::new(&normalized);
    if !group.exists(candidate) {
        return Err(FontResourceError::NotFound {
            name: name.to_string(),
        });
    }
    let bytes = group.read_file(candidate)?;
    Ok(FontResource::new(name, bytes))
}

pub fn load_endeavour_font(group: &Group) -> Result<FontResource, FontResourceError> {
    load_ttf(group, "Endeavour.ttf")
}

/// Parses every active `[Font]` record in `Fonts.txt`. Commented records in
/// the shipped file remain inert, and duplicate fields use the first value
/// selected by `StdCompilerINIRead::Name`.
pub fn load_font_definitions(group: &Group) -> Result<Vec<FontDefinition>, FontResourceError> {
    if !group.exists("Fonts.txt") {
        return Ok(Vec::new());
    }
    let bytes = group.read_file("Fonts.txt")?;
    Ok(parse_font_definitions(&bytes))
}

/// Selects the closest exact-name definition. Equal-distance ties choose the
/// later-loaded record (`old_diff >= new_diff`, C4Fonts.cpp:187-200).
pub fn select_font_definition<'a>(
    definitions: &'a [FontDefinition],
    name: &str,
    size: i32,
) -> Option<&'a FontDefinition> {
    definitions
        .iter()
        .filter(|definition| definition.name == name)
        .fold(None, |best: Option<&FontDefinition>, candidate| {
            let candidate_diff = (i64::from(candidate.size) - i64::from(size)).unsigned_abs();
            match best {
                Some(current)
                    if (i64::from(current.size) - i64::from(size)).unsigned_abs()
                        < candidate_diff =>
                {
                    Some(current)
                }
                _ => Some(candidate),
            }
        })
}

impl FontCatalog {
    /// Appends vector faces and definitions from one group.
    /// Lookups scan vector faces newest-first, matching C4FontLoader's
    /// prepended `C4VectorFont` chain.
    pub fn load_group(&mut self, group: &Group) -> Result<(), FontResourceError> {
        if let Ok(entries) = group.entries() {
            for extension in VECTOR_FONT_EXTENSIONS {
                for entry in &entries {
                    if entry.relative_path.components().count() != 1 {
                        continue;
                    }
                    let Some(face) = vector_font_face_bytes(&entry.name_bytes, extension) else {
                        continue;
                    };
                    let Ok(bytes) = group.read_entry_bytes_exact(entry) else {
                        continue;
                    };
                    self.vector_fonts.push(CatalogVectorFont {
                        face: face.to_vec(),
                        resource: FontResource::new(
                            clonk_script::c4_string_from_bytes(&entry.name_bytes),
                            bytes,
                        ),
                    });
                }
            }
        }
        self.definitions.extend(load_font_definitions(group)?);
        Ok(())
    }

    pub fn definitions(&self) -> &[FontDefinition] {
        &self.definitions
    }

    pub fn resolve(
        &self,
        request: &str,
        base_size: i32,
        role: FontRole,
        shadow: bool,
    ) -> Option<ResolvedFontSpec> {
        self.resolve_candidates(request, base_size, role, shadow)
            .into_iter()
            .next()
    }

    /// Resolves every source candidate in native attempt order.
    /// Matching registered vector faces are newest-first, followed by one
    /// unresolved face/filename candidate for the host-system fallback.
    pub fn resolve_candidates(
        &self,
        request: &str,
        base_size: i32,
        role: FontRole,
        shadow: bool,
    ) -> Vec<ResolvedFontSpec> {
        let request_bytes = clonk_script::c4_string_bytes(request);
        let request_bytes = request_bytes
            .split(|byte| *byte == 0)
            .next()
            .unwrap_or_default();
        if request_bytes.is_empty() {
            return Vec::new();
        }
        let request = clonk_script::c4_string_from_bytes(request_bytes);
        let mapped = shadow
            .then(|| select_font_definition(&self.definitions, &request, base_size))
            .flatten()
            .map(|definition| definition.font_for(role))
            .unwrap_or(&request);
        if mapped.is_empty() {
            return Vec::new();
        }
        let mapped_bytes = clonk_script::c4_string_bytes(mapped);
        let mapped_bytes = mapped_bytes
            .split(|byte| *byte == 0)
            .next()
            .unwrap_or_default();
        if mapped_bytes.is_empty() {
            return Vec::new();
        }
        let face_bytes = font_spec_segment(mapped_bytes, 0).unwrap_or_default();
        let face = clonk_script::c4_string_from_bytes(face_bytes);
        let second = font_spec_segment(mapped_bytes, 1);
        let extension = font_spec_extension(face_bytes);
        if extension.eq_ignore_ascii_case(b"png") || extension.eq_ignore_ascii_case(b"bmp") {
            return vec![ResolvedFontSpec::Bitmap {
                filename: face,
                indent: second.and_then(parse_percent_i_prefix).unwrap_or(0),
            }];
        }
        let size = second
            .and_then(parse_percent_i_prefix)
            .unwrap_or_else(|| role.derived_size(base_size));
        let weight = font_spec_segment(mapped_bytes, 2)
            .and_then(parse_percent_i_prefix)
            .map(|value| value as u32)
            .unwrap_or(400);
        self.vector_fonts
            .iter()
            .rev()
            .filter(|font| font.face.as_slice() == face_bytes)
            .map(|font| ResolvedFontSpec::Vector {
                face: face.clone(),
                bytes: Some(font.resource.clone_bytes()),
                face_index: 0,
                size,
                weight,
            })
            .chain(std::iter::once_with(|| ResolvedFontSpec::Vector {
                face: face.clone(),
                bytes: None,
                face_index: 0,
                size,
                weight,
            }))
            .collect()
    }
}

fn font_spec_segment(value: &[u8], index: usize) -> Option<&[u8]> {
    let mut segment = value;
    for _ in 0..index {
        let separator = segment.iter().position(|byte| *byte == b',')?;
        segment = &segment[separator + 1..];
    }
    let end = segment
        .iter()
        .position(|byte| *byte == b',')
        .unwrap_or(segment.len())
        .min(C4_MAX_NAME_BYTES);
    Some(&segment[..end])
}

fn font_spec_extension(face: &[u8]) -> &[u8] {
    let separator = std::path::MAIN_SEPARATOR as u8;
    match face
        .iter()
        .rposition(|byte| matches!(*byte, b'.') || *byte == separator)
    {
        Some(position) if face[position] == b'.' => &face[position + 1..],
        _ => &face[face.len()..],
    }
}

fn ascii_digit_value(byte: u8) -> Option<u32> {
    match byte {
        b'0'..=b'9' => Some(u32::from(byte - b'0')),
        b'a'..=b'f' => Some(u32::from(byte - b'a') + 10),
        b'A'..=b'F' => Some(u32::from(byte - b'A') + 10),
        _ => None,
    }
}

fn parse_percent_i_prefix(value: &[u8]) -> Option<i32> {
    let mut index = 0;
    while value
        .get(index)
        .is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | b'\x0b' | b'\x0c'))
    {
        index += 1;
    }

    let negative = match value.get(index) {
        Some(b'-') => {
            index += 1;
            true
        }
        Some(b'+') => {
            index += 1;
            false
        }
        _ => false,
    };
    let radix = if value.get(index) == Some(&b'0')
        && value
            .get(index + 1)
            .is_some_and(|byte| byte.eq_ignore_ascii_case(&b'x'))
        && value
            .get(index + 2)
            .and_then(|byte| ascii_digit_value(*byte))
            .is_some()
    {
        index += 2;
        16_u32
    } else if value.get(index) == Some(&b'0') {
        8_u32
    } else {
        10_u32
    };

    let digit_start = index;
    let mut parsed = 0_u32;
    while let Some(digit) = value
        .get(index)
        .and_then(|byte| ascii_digit_value(*byte))
        .filter(|digit| *digit < radix)
    {
        parsed = parsed.wrapping_mul(radix).wrapping_add(digit);
        index += 1;
    }
    (index > digit_start).then(|| {
        if negative {
            0_u32.wrapping_sub(parsed) as i32
        } else {
            parsed as i32
        }
    })
}

fn parse_font_definitions(bytes: &[u8]) -> Vec<FontDefinition> {
    // LoadEntryString exposes a native C string. Keep legacy high bytes
    // lossless and hide any suffix after the first NUL from the INI reader.
    let bytes = bytes.split(|byte| *byte == 0).next().unwrap_or_default();
    let source = clonk_script::c4_string_from_bytes(bytes);
    let mut definitions = Vec::new();
    let mut current = None;
    let mut seen = HashSet::new();
    for raw_line in source.split(['\r', '\n']) {
        let line = raw_line.trim_start_matches([' ', '\t']);
        if line.is_empty()
            || line.starts_with('#')
            || line.starts_with(';')
            || line.starts_with("//")
        {
            continue;
        }
        if let Some(section) = stdcompiler_ini_section(line) {
            if let Some(definition) = current.take() {
                definitions.push(definition);
            }
            current = (section == "Font").then(FontDefinition::default);
            seen.clear();
            continue;
        }
        let Some(definition) = current.as_mut() else {
            continue;
        };
        let Some((key, value)) = stdcompiler_ini_value(line) else {
            continue;
        };
        if !seen.insert(key) {
            continue;
        }
        match key {
            "Name" => definition.name = value.to_string(),
            "Size" => {
                definition.size = parse_action_i32_prefix(&clonk_script::c4_string_bytes(value))
                    .map(|(size, _)| size)
                    .unwrap_or(1)
            }
            "LogFont" => definition.log_font = value.to_string(),
            "SmallFont" => definition.small_font = value.to_string(),
            "Font" => definition.font = value.to_string(),
            "CaptionFont" => definition.caption_font = value.to_string(),
            "TitleFont" => definition.title_font = value.to_string(),
            _ => {}
        }
    }
    if let Some(definition) = current {
        definitions.push(definition);
    }
    definitions
}

fn stdcompiler_ini_name_end(source: &str) -> usize {
    source
        .char_indices()
        .take_while(|(_, character)| {
            character.is_ascii_alphanumeric() || matches!(character, ' ' | '_')
        })
        .map(|(index, character)| index + character.len_utf8())
        .last()
        .unwrap_or(0)
}

fn stdcompiler_ini_section(line: &str) -> Option<&str> {
    let source = line.strip_prefix('[')?;
    source
        .as_bytes()
        .first()?
        .is_ascii_alphabetic()
        .then_some(())?;
    let name_end = stdcompiler_ini_name_end(source);
    let (name, rest) = source.split_at(name_end);
    rest.trim_start_matches([' ', '\t'])
        .starts_with(']')
        .then_some(name)
}

fn stdcompiler_ini_value(line: &str) -> Option<(&str, &str)> {
    line.as_bytes()
        .first()?
        .is_ascii_alphabetic()
        .then_some(())?;
    let name_end = stdcompiler_ini_name_end(line);
    let (name, rest) = line.split_at(name_end);
    let value = rest
        .trim_start_matches([' ', '\t'])
        .strip_prefix('=')?
        .trim_start_matches([' ', '\t']);
    Some((name, value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MutableGroup;
    use std::fs;
    fn tempdir() -> std::io::Result<tempfile::TempDir> {
        tempfile::Builder::new().prefix("lc-test-").tempdir()
    }

    #[test]
    fn font_defs_select_closest_name_size_and_later_tie() {
        let definitions = parse_font_definitions(
            br#"# [Font]
# Name=Commented
[Font]
Name=Endeavour
Size=12
LogFont=FirstLog
SmallFont=FirstSmall
Font=FirstMain
CaptionFont=FirstCaption
TitleFont=FirstTitle
[Font]
Name=Other
Size=14
Font=Other
[Font]
Name=Endeavour
Size=16
LogFont=LaterLog
SmallFont=LaterSmall
Font=LaterMain
CaptionFont=LaterCaption
TitleFont=LaterTitle
"#,
        );
        assert_eq!(definitions.len(), 3);
        let selected =
            select_font_definition(&definitions, "Endeavour", 14).expect("matching definition");
        assert_eq!(selected.log_font, "LaterLog");
        assert_eq!(selected.small_font, "LaterSmall");
        assert_eq!(selected.font, "LaterMain");
        assert_eq!(selected.caption_font, "LaterCaption");
        assert_eq!(selected.title_font, "LaterTitle");
        assert!(select_font_definition(&definitions, "endeavour", 14).is_none());
    }

    #[test]
    fn fully_commented_fonts_file_has_no_active_definitions() {
        assert!(parse_font_definitions(
            b"# [Font]\n#Name=Endeavour\n#Size=10\n#Font=FontEndeavour12.png,1\n"
        )
        .is_empty());
    }

    #[test]
    fn raw_explicit_size_applies_to_every_role_before_fontdef_matching() {
        let catalog = FontCatalog::default();
        for role in [
            FontRole::Log,
            FontRole::MainSmall,
            FontRole::Main,
            FontRole::Caption,
            FontRole::Title,
        ] {
            let ResolvedFontSpec::Vector { size, weight, .. } = catalog
                .resolve("SomeFace,20,700", 16, role, true)
                .expect("raw vector spec")
            else {
                panic!("expected vector spec")
            };
            assert_eq!(size, 20);
            assert_eq!(weight, 700);
        }
    }

    #[test]
    fn font_specs_apply_c4maxname_and_percent_i_numeric_grammar() {
        let catalog = FontCatalog::default();

        for (spec, expected_size, expected_weight) in [
            ("Face,19junk,700tail", 19, 700),
            ("Face,+0x10junk,0700tail", 16, 448),
            ("Face,-010junk,-0X2BCtail", -8, (-700_i32) as u32),
            ("Face,09ignored,+42ignored", 0, 42),
        ] {
            let Some(ResolvedFontSpec::Vector { size, weight, .. }) =
                catalog.resolve(spec, 14, FontRole::Main, true)
            else {
                panic!("expected vector spec for {spec}")
            };
            assert_eq!(size, expected_size, "size parsed from {spec}");
            assert_eq!(weight, expected_weight, "weight parsed from {spec}");
        }

        let bounded_parameters = format!("Face,{}42,{}70", " ".repeat(30), " ".repeat(30));
        let Some(ResolvedFontSpec::Vector { size, weight, .. }) =
            catalog.resolve(&bounded_parameters, 14, FontRole::Main, true)
        else {
            panic!("expected vector spec with bounded parameters")
        };
        assert_eq!(size, 14, "the size digit lies beyond its 30-byte field");
        assert_eq!(
            weight, 400,
            "the weight digit lies beyond its 30-byte field"
        );

        let utf8_boundary = clonk_script::c4_string_from_bytes(
            &[b'A'; 29]
                .into_iter()
                .chain([0xc3, 0xa9])
                .chain(*b",12,400")
                .collect::<Vec<_>>(),
        );
        let Some(ResolvedFontSpec::Vector { face, size, .. }) =
            catalog.resolve(&utf8_boundary, 14, FontRole::Main, true)
        else {
            panic!("expected UTF-8-boundary vector spec")
        };
        assert_eq!(
            clonk_script::c4_string_bytes(&face),
            [b'A'; 29].into_iter().chain([0xc3]).collect::<Vec<_>>()
        );
        assert_eq!(size, 12, "commas are found before fields are truncated");

        let high_byte_boundary = clonk_script::c4_string_from_bytes(
            &[b'B'; 29]
                .into_iter()
                .chain([0xe9, b'Z'])
                .chain(*b",13,500")
                .collect::<Vec<_>>(),
        );
        let Some(ResolvedFontSpec::Vector {
            face, size, weight, ..
        }) = catalog.resolve(&high_byte_boundary, 14, FontRole::Main, true)
        else {
            panic!("expected high-byte-boundary vector spec")
        };
        assert_eq!(
            clonk_script::c4_string_bytes(&face),
            [b'B'; 29].into_iter().chain([0xe9]).collect::<Vec<_>>()
        );
        assert_eq!((size, weight), (13, 500));

        let extension_beyond_boundary = format!("{}.png,7", "X".repeat(27));
        assert!(matches!(
            catalog.resolve(&extension_beyond_boundary, 14, FontRole::Main, true),
            Some(ResolvedFontSpec::Vector { .. })
        ));
        assert!(matches!(
            catalog.resolve("Glyph.png,010junk,ignored", 14, FontRole::Main, true),
            Some(ResolvedFontSpec::Bitmap { ref filename, indent: 8 })
                if filename == "Glyph.png"
        ));
        assert!(matches!(
            catalog.resolve(".PnG,010junk,ignored", 14, FontRole::Main, true),
            Some(ResolvedFontSpec::Bitmap { ref filename, indent: 8 })
                if filename == ".PnG"
        ));

        let nul_alias = clonk_script::c4_string_from_bytes(b"Alias\0ignored");
        let mapped_catalog = FontCatalog {
            definitions: vec![FontDefinition {
                name: "Alias".to_string(),
                size: 14,
                font: "Mapped,0x10,0700".to_string(),
                ..FontDefinition::default()
            }],
            vector_fonts: Vec::new(),
        };
        assert!(matches!(
            mapped_catalog.resolve(&nul_alias, 14, FontRole::Main, true),
            Some(ResolvedFontSpec::Vector { ref face, size: 16, weight: 448, .. })
                if face == "Mapped"
        ));
        let nul_parameters = clonk_script::c4_string_from_bytes(b"Face,12\0,700");
        assert!(matches!(
            catalog.resolve(&nul_parameters, 14, FontRole::Main, true),
            Some(ResolvedFontSpec::Vector {
                size: 12,
                weight: 400,
                ..
            })
        ));

        let long_registered_face = vec![b'R'; 31];
        let truncated_registered_face = long_registered_face[..C4_MAX_NAME_BYTES].to_vec();
        let registered_catalog = FontCatalog {
            definitions: Vec::new(),
            vector_fonts: vec![
                CatalogVectorFont {
                    face: truncated_registered_face.clone(),
                    resource: FontResource::new("truncated.ttf", b"truncated".to_vec()),
                },
                CatalogVectorFont {
                    face: long_registered_face.clone(),
                    resource: FontResource::new("untruncated.ttf", b"untruncated".to_vec()),
                },
            ],
        };
        let long_registered_spec = clonk_script::c4_string_from_bytes(
            &long_registered_face
                .into_iter()
                .chain(*b",12,400")
                .collect::<Vec<_>>(),
        );
        let candidates =
            registered_catalog.resolve_candidates(&long_registered_spec, 14, FontRole::Main, true);
        assert_eq!(
            candidates.len(),
            2,
            "one exact registered face plus fallback"
        );
        for candidate in &candidates {
            let ResolvedFontSpec::Vector { face, .. } = candidate else {
                panic!("expected registered vector candidate")
            };
            assert_eq!(
                clonk_script::c4_string_bytes(face),
                truncated_registered_face
            );
        }
        assert!(matches!(
            &candidates[0],
            ResolvedFontSpec::Vector { bytes: Some(bytes), .. }
                if bytes.as_ref() == b"truncated"
        ));
        assert!(matches!(
            &candidates[1],
            ResolvedFontSpec::Vector { bytes: None, .. }
        ));
    }

    #[test]
    fn catalog_resolves_configured_vector_face_and_cpp_derived_sizes() {
        let directory = tempdir().expect("font group");
        fs::write(directory.path().join("SomeFace.ttf"), b"font bytes").expect("vector font");
        fs::write(
            directory.path().join("Fonts.txt"),
            b"# [Font]\n# Name=SomeFace\n# Font=Ignored.png,1\n",
        )
        .expect("commented definitions");
        let group = Group::open(directory.path()).expect("open font group");
        let mut catalog = FontCatalog::default();
        catalog.load_group(&group).expect("load catalog");
        assert!(catalog.definitions().is_empty());

        for (role, expected_size) in [
            (FontRole::Log, 13),
            (FontRole::MainSmall, 14),
            (FontRole::Main, 16),
            (FontRole::Caption, 18),
            (FontRole::Title, 25),
        ] {
            let ResolvedFontSpec::Vector {
                face,
                bytes,
                face_index,
                size,
                weight,
            } = catalog
                .resolve("SomeFace", 16, role, true)
                .expect("configured face resolves")
            else {
                panic!("expected vector font")
            };
            assert_eq!(face, "SomeFace");
            assert_eq!(bytes.as_deref(), Some(b"font bytes".as_slice()));
            assert_eq!(face_index, 0);
            assert_eq!(size, expected_size);
            assert_eq!(weight, 400);
        }
    }

    #[test]
    fn font_catalog_preserves_native_byte_vector_filenames_in_directory_and_packed_groups() {
        const PACKED_U_FACE: &[u8] = b"Gr\xfcn";
        const PACKED_O_FACE: &[u8] = b"Gr\xf6n";
        const PACKED_U_TTF: &[u8] = b"Gr\xfcn.TTF";
        const PACKED_O_TTF: &[u8] = b"Gr\xf6n.ttf";

        assert_eq!(
            vector_font_face_bytes(b".ttf", b"ttf"),
            Some(b"".as_slice())
        );
        assert_eq!(
            vector_font_face_bytes(b"A.B.TtF", b"ttf"),
            Some(b"A.B".as_slice())
        );
        assert_eq!(vector_font_face_bytes(b"A.ttf.bak", b"ttf"), None);
        assert_eq!(vector_font_face_bytes(b"A.t\x80f", b"ttf"), None);

        assert_eq!(
            String::from_utf8_lossy(PACKED_U_FACE),
            String::from_utf8_lossy(PACKED_O_FACE),
            "the fixture must collide under lossy UTF-8"
        );
        assert_ne!(
            clonk_script::c4_string_from_bytes(PACKED_U_FACE),
            clonk_script::c4_string_from_bytes(PACKED_O_FACE),
            "the native byte projection must retain distinct faces"
        );

        let mut catalog = FontCatalog::default();
        #[cfg(unix)]
        {
            use std::ffi::OsStr;
            use std::os::unix::ffi::OsStrExt as _;

            // Linux can create the legacy single-byte names directly. macOS
            // rejects malformed UTF-8 at the filesystem boundary, so use two
            // non-ASCII UTF-8 names there; the packed half below still pins
            // the lossy-collision case on every platform.
            #[cfg(target_os = "linux")]
            const DIRECTORY_U_FACE: &[u8] = PACKED_U_FACE;
            #[cfg(target_os = "linux")]
            const DIRECTORY_O_FACE: &[u8] = PACKED_O_FACE;
            #[cfg(target_os = "linux")]
            const DIRECTORY_U_TTF: &[u8] = PACKED_U_TTF;
            #[cfg(target_os = "linux")]
            const DIRECTORY_O_TTF: &[u8] = PACKED_O_TTF;
            #[cfg(not(target_os = "linux"))]
            const DIRECTORY_U_FACE: &[u8] = b"Gr\xc3\xb8n";
            #[cfg(not(target_os = "linux"))]
            const DIRECTORY_O_FACE: &[u8] = b"Gr\xc3\x9fn";
            #[cfg(not(target_os = "linux"))]
            const DIRECTORY_U_TTF: &[u8] = b"Gr\xc3\xb8n.TTF";
            #[cfg(not(target_os = "linux"))]
            const DIRECTORY_O_TTF: &[u8] = b"Gr\xc3\x9fn.ttf";

            let directory = tempdir().expect("directory font group");
            fs::write(
                directory.path().join(OsStr::from_bytes(DIRECTORY_U_TTF)),
                b"directory-u",
            )
            .expect("write native-byte directory font");
            fs::write(
                directory.path().join(OsStr::from_bytes(DIRECTORY_O_TTF)),
                b"directory-o",
            )
            .expect("write second native-byte directory font");
            catalog
                .load_group(&Group::open(directory.path()).expect("open directory font group"))
                .expect("load directory font catalog");

            for (face, expected) in [
                (DIRECTORY_U_FACE, b"directory-u".as_slice()),
                (DIRECTORY_O_FACE, b"directory-o".as_slice()),
            ] {
                let request = clonk_script::c4_string_from_bytes(face);
                let ResolvedFontSpec::Vector {
                    face,
                    bytes: Some(bytes),
                    ..
                } = catalog
                    .resolve(&request, 14, FontRole::Main, true)
                    .expect("directory face resolves")
                else {
                    panic!("expected catalog vector font")
                };
                assert_eq!(face, request);
                assert_eq!(bytes.as_ref(), expected);
            }
        }

        let mut packed = MutableGroup::new("native-fonts.c4g");
        packed
            .add_file_bytes_with_metadata(
                b"Gr\xfcn.fon".to_vec(),
                b"packed-u-fon".to_vec(),
                1,
                false,
            )
            .expect("add older extension candidate");
        packed
            .add_file_bytes_with_metadata(PACKED_U_TTF.to_vec(), b"packed-u-ttf".to_vec(), 1, false)
            .expect("add newest extension candidate");
        packed
            .add_file_bytes_with_metadata(PACKED_O_TTF.to_vec(), b"packed-o-ttf".to_vec(), 1, false)
            .expect("add distinct packed candidate");
        packed
            .add_file_bytes_with_metadata(b"Same.ttf".to_vec(), b"same-ttf".to_vec(), 1, false)
            .expect("add earlier extension-priority candidate");
        packed
            .add_file_bytes_with_metadata(b"Same.otf".to_vec(), b"same-otf".to_vec(), 1, false)
            .expect("add later extension-priority candidate");
        let packed = Group::from_memory(
            std::path::PathBuf::from("native-fonts.c4g"),
            packed.pack().expect("pack native-byte font group"),
        )
        .expect("open packed font group");
        catalog
            .load_group(&packed)
            .expect("load packed font catalog");

        for (face_bytes, expected) in [
            (PACKED_U_FACE, b"packed-u-ttf".as_slice()),
            (PACKED_O_FACE, b"packed-o-ttf".as_slice()),
        ] {
            let request = clonk_script::c4_string_from_bytes(face_bytes);
            let ResolvedFontSpec::Vector {
                face,
                bytes: Some(bytes),
                ..
            } = catalog
                .resolve(&request, 14, FontRole::Main, true)
                .expect("packed face resolves")
            else {
                panic!("expected catalog vector font")
            };
            assert_eq!(face, request);
            assert_eq!(bytes.as_ref(), expected);
        }
        assert!(catalog
            .vector_fonts
            .iter()
            .any(|font| { clonk_script::c4_string_bytes(font.resource.name()) == PACKED_U_TTF }));
        let ResolvedFontSpec::Vector {
            bytes: Some(bytes), ..
        } = catalog
            .resolve("Same", 14, FontRole::Main, true)
            .expect("same-face extension priority resolves")
        else {
            panic!("expected catalog vector font")
        };
        assert_eq!(bytes.as_ref(), b"same-otf");
    }

    #[test]
    fn font_definition_bitmap_mapping_is_shadow_only() {
        let definitions = parse_font_definitions(
            b"[Font]\nName=Endeavour\nSize=14\nFont=FontEndeavour12.png,1\n",
        );
        let catalog = FontCatalog {
            definitions,
            vector_fonts: Vec::new(),
        };
        assert!(matches!(
            catalog.resolve("Endeavour", 14, FontRole::Main, true),
            Some(ResolvedFontSpec::Bitmap { ref filename, indent: 1 })
                if filename == "FontEndeavour12.png"
        ));
        assert!(matches!(
            catalog.resolve("Endeavour", 14, FontRole::Main, false),
            Some(ResolvedFontSpec::Vector { ref face, size: 14, weight: 400, .. })
                if face == "Endeavour"
        ));
    }

    #[test]
    fn stdcompiler_names_values_and_nul_suffix_keep_legacy_byte_semantics() {
        let definitions = parse_font_definitions(
            b"[Font]suffix-is-ignored\nName=Kept \nSize\t= 16\nName =Wrong\0\n[Font]\nName=Hidden\n",
        );
        assert_eq!(definitions.len(), 1);
        assert_eq!(definitions[0].name, "Kept ");
        assert_eq!(definitions[0].size, 16);
    }

    #[test]
    fn fonts_txt_size_uses_stdcompiler_numeric_prefix_and_error_semantics() {
        fn packed_fonts_group(name: &str, fonts: &[u8]) -> Group {
            let mut packed = MutableGroup::new(name);
            packed
                .add_file_with_metadata("Fonts.txt", fonts.to_vec(), 1, false)
                .expect("add Fonts.txt");
            Group::from_memory(
                std::path::PathBuf::from(name),
                packed.pack().expect("pack font group"),
            )
            .expect("open packed font group")
        }

        let prior = packed_fonts_group("font-size-prior.c4g", b"[Font]\nName=Prior\nSize=9\n");
        let later = packed_fonts_group(
            "font-size-later.c4g",
            br#"[Font]
Name=DecimalPrefix
Size=14junk
[Font]
Name=HexPrefix
Size=0X10tail
[Font]
Name=PositivePrefix
Size=+15tail
[Font]
Name=NegativeLeadingZeroDecimal
Size=-014tail
[Font]
Name=ConsumedZero
Size=0junk
[Font]
Name=SignedHexMarker
Size=-0X10
[Font]
Name=BareHexMarker
Size=0X
[Font]
Name=Defaulted
Size=not-a-number
[Font]
Name=AfterDefaulted
Size=17tail
[Font]
Name=Omitted
"#,
        );

        let mut catalog = FontCatalog::default();
        catalog.load_group(&prior).expect("load prior definitions");
        catalog
            .load_group(&later)
            .expect("defaulted Size keeps the later definitions");
        assert_eq!(
            catalog
                .definitions()
                .iter()
                .map(|definition| (definition.name.as_str(), definition.size))
                .collect::<Vec<_>>(),
            [
                ("Prior", 9),
                ("DecimalPrefix", 14),
                ("HexPrefix", 16),
                ("PositivePrefix", 15),
                ("NegativeLeadingZeroDecimal", -14),
                ("ConsumedZero", 0),
                ("SignedHexMarker", 0),
                ("BareHexMarker", 0),
                ("Defaulted", 1),
                ("AfterDefaulted", 17),
                ("Omitted", 1),
            ]
        );
    }
}
