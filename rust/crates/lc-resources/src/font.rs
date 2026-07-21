use crate::{Group, GroupError};
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
    /// Appends vector faces and definitions from one registered group.
    /// Lookups scan vector faces newest-first, matching C4FontLoader's
    /// prepended `C4VectorFont` chain.
    pub fn load_group(&mut self, group: &Group) -> Result<(), FontResourceError> {
        let entries = group.entries()?;
        for extension in VECTOR_FONT_EXTENSIONS {
            for entry in &entries {
                if entry.relative_path.components().count() != 1 {
                    continue;
                }
                let Some(face) = vector_font_face_bytes(&entry.name_bytes, extension) else {
                    continue;
                };
                let bytes = group.read_entry_bytes_exact(entry)?;
                self.vector_fonts.push(CatalogVectorFont {
                    face: face.to_vec(),
                    resource: FontResource::new(
                        lc_script::c4_string_from_bytes(&entry.name_bytes),
                        bytes,
                    ),
                });
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
        if request.is_empty() {
            return None;
        }
        let mapped = shadow
            .then(|| select_font_definition(&self.definitions, request, base_size))
            .flatten()
            .map(|definition| definition.font_for(role))
            .unwrap_or(request);
        if mapped.is_empty() {
            return None;
        }
        let mut segments = mapped.split(',');
        let face = segments.next().unwrap_or_default();
        let second = segments.next();
        let extension = Path::new(face)
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        if extension.eq_ignore_ascii_case("png") || extension.eq_ignore_ascii_case("bmp") {
            return Some(ResolvedFontSpec::Bitmap {
                filename: face.to_string(),
                indent: second.and_then(parse_i32_prefix).unwrap_or(0),
            });
        }
        let size = second
            .and_then(parse_i32_prefix)
            .unwrap_or_else(|| role.derived_size(base_size));
        let weight = segments
            .next()
            .and_then(parse_i32_prefix)
            .map(|value| value as u32)
            .unwrap_or(400);
        let face_bytes = lc_script::c4_string_bytes(face);
        let bytes = self
            .vector_fonts
            .iter()
            .rev()
            .find(|font| font.face.as_slice() == face_bytes.as_slice())
            .map(|font| font.resource.clone_bytes());
        Some(ResolvedFontSpec::Vector {
            face: face.to_string(),
            bytes,
            face_index: 0,
            size,
            weight,
        })
    }
}

fn parse_i32_prefix(value: &str) -> Option<i32> {
    let value = value.trim_start();
    let end = value
        .char_indices()
        .take_while(|(index, character)| {
            character.is_ascii_digit() || (*index == 0 && matches!(character, '+' | '-'))
        })
        .map(|(index, character)| index + character.len_utf8())
        .last()?;
    value[..end].parse().ok()
}

fn parse_font_definitions(bytes: &[u8]) -> Vec<FontDefinition> {
    // LoadEntryString exposes a native C string. Keep legacy high bytes
    // lossless and hide any suffix after the first NUL from the INI reader.
    let bytes = bytes.split(|byte| *byte == 0).next().unwrap_or_default();
    let source = lc_script::c4_string_from_bytes(bytes);
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
            "Size" => definition.size = value.parse().unwrap_or(1),
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
            lc_script::c4_string_from_bytes(PACKED_U_FACE),
            lc_script::c4_string_from_bytes(PACKED_O_FACE),
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
                let request = lc_script::c4_string_from_bytes(face);
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
            let request = lc_script::c4_string_from_bytes(face_bytes);
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
            .any(|font| { lc_script::c4_string_bytes(font.resource.name()) == PACKED_U_TTF }));
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
}
