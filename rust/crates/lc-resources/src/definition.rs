use crate::{GraphicsImage, Group, GroupError};
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

use thiserror::Error;

/// Files required to construct an engine definition from a classic C4 definition folder.
#[derive(Debug, Clone)]
pub struct Definition {
    pub core: DefCore,
    pub script: DefinitionScript,
    pub action_map: Option<ActionMap>,
    pub picture_image: Option<GraphicsImage>,
    pub graphics_image: Option<GraphicsImage>,
}

impl Definition {
    pub fn load(group: &Group) -> Result<Self, DefinitionError> {
        let core = DefCore::load(group)?;

        let script = load_scripts(group)?;

        let action_map = match group.read_file("ActMap.txt") {
            Ok(bytes) => Some(parse_act_map(&bytes)?),
            Err(GroupError::EntryNotFound(_)) => None,
            Err(GroupError::Io(ref err)) if err.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(DefinitionError::Resources(error)),
        };

        let picture_image = load_definition_picture(group, &core);
        let graphics_image = load_definition_graphics(group);

        Ok(Self {
            core,
            script,
            action_map,
            picture_image,
            graphics_image,
        })
    }
}

/// Parsed metadata from `DefCore.txt`.
#[derive(Debug, Clone)]
pub struct DefCore {
    pub id: String,
    pub name: Option<String>,
    pub category: i32,
    pub crew_member: bool,
    pub value: i32,
    pub mass: i32,
    pub picture: Option<PictureRect>,
}

impl DefCore {
    pub fn load(group: &Group) -> Result<Self, DefinitionError> {
        let bytes = group.read_file("DefCore.txt").map_err(|err| match err {
            GroupError::EntryNotFound(_) => DefinitionError::DefCoreMissing,
            other => DefinitionError::Resources(other),
        })?;
        parse_def_core(&bytes)
    }
}

/// Combined script sources originating from a definition group.
#[derive(Debug, Clone)]
pub struct DefinitionScript {
    files: Vec<DefinitionScriptFile>,
    combined: String,
}

impl DefinitionScript {
    pub fn files(&self) -> &[DefinitionScriptFile] {
        &self.files
    }

    pub fn combined(&self) -> &str {
        &self.combined
    }
}

/// Individual script source file.
#[derive(Debug, Clone)]
pub struct DefinitionScriptFile {
    pub path: PathBuf,
    pub contents: String,
}

/// Rectangle metadata parsed from `Picture=` in `DefCore.txt`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PictureRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// Representation of `ActMap.txt`.
#[derive(Debug, Clone)]
pub struct ActionMap {
    pub default_action: Option<String>,
    pub actions: HashMap<String, ActionDefinition>,
}

/// Action metadata used to construct runtime action specifications.
#[derive(Debug, Clone, Default)]
pub struct ActionDefinition {
    pub procedure: Option<String>,
    pub length: Option<u32>,
    pub next_action: Option<String>,
    pub delay: Option<u32>,
    pub step: Option<u32>,
    pub phase_call: Option<String>,
    pub start_call: Option<String>,
    pub end_call: Option<String>,
    pub abort_call: Option<String>,
    pub no_other_action: bool,
    pub dig_free: Option<i32>,
}

#[derive(Debug, Error)]
pub enum DefinitionError {
    #[error("definition core `DefCore.txt` missing")]
    DefCoreMissing,
    #[error("definition core is missing required field `{0}`")]
    MissingDefCoreField(&'static str),
    #[error("definition core references unknown category flag `{0}`")]
    UnknownCategoryFlag(String),
    #[error("definition core category value `{0}` is not valid")]
    InvalidCategoryValue(String),
    #[error("definition core could not be parsed: {0}")]
    DefCoreParse(String),
    #[error("definition is missing script sources")]
    ScriptMissing,
    #[error("definition script `{path}` is not valid UTF-8")]
    ScriptEncoding {
        path: PathBuf,
        #[source]
        source: std::string::FromUtf8Error,
    },
    #[error("ActMap.txt could not be parsed: {0}")]
    ActMapParse(String),
    #[error(transparent)]
    Resources(#[from] GroupError),
}

fn parse_def_core(bytes: &[u8]) -> Result<DefCore, DefinitionError> {
    let text = String::from_utf8_lossy(bytes);
    let mut current_section: Option<String> = None;

    let mut id: Option<String> = None;
    let mut name: Option<String> = None;
    let mut category: i32 = 0;
    let mut category_set = false;
    let mut crew_member = false;
    let mut object_value: i32 = 0;
    let mut object_mass: i32 = 0;
    let mut picture: Option<PictureRect> = None;

    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty()
            || line.starts_with(';')
            || line.starts_with('#')
            || line.starts_with("//")
        {
            continue;
        }

        if line.starts_with('[') && line.ends_with(']') {
            current_section = Some(line[1..line.len() - 1].trim().to_ascii_lowercase());
            continue;
        }

        let Some((raw_key, raw_value)) = line.split_once('=') else {
            continue;
        };
        let key = raw_key.trim();
        let value = raw_value.trim();

        let section = current_section.as_deref().unwrap_or_else(|| "defcore");

        if section != "defcore" {
            continue;
        }

        match key.to_ascii_lowercase().as_str() {
            "id" => {
                if !value.is_empty() {
                    id = Some(value.to_string());
                }
            }
            "name" => {
                if !value.is_empty() {
                    name = Some(value.to_string());
                }
            }
            "value" => {
                object_value = parse_i32(value).unwrap_or(0);
            }
            "mass" => {
                object_mass = parse_i32(value).unwrap_or(0).max(0);
            }
            "category" => {
                category = parse_category(value)?;
                category_set = true;
            }
            "crewmember" => {
                crew_member = parse_bool(value);
            }
            "picture" => {
                if let Some(rect) = parse_rect(value) {
                    picture = Some(rect);
                }
            }
            _ => {}
        }
    }

    let id = id.ok_or(DefinitionError::MissingDefCoreField("id"))?;
    if !category_set {
        // Preserve compatibility with the C++ engine where unspecified category defaults to 0.
        category = 0;
    }

    Ok(DefCore {
        id,
        name,
        category,
        crew_member,
        value: object_value,
        mass: object_mass,
        picture,
    })
}

fn load_scripts(group: &Group) -> Result<DefinitionScript, DefinitionError> {
    let mut files: Vec<DefinitionScriptFile> = Vec::new();
    collect_script_files(group, Path::new(""), &mut files)?;
    if files.is_empty() {
        return Err(DefinitionError::ScriptMissing);
    }

    files.sort_by(|a, b| a.path.cmp(&b.path));

    let mut combined = String::new();
    for file in &files {
        if !combined.is_empty() {
            combined.push('\n');
        }
        combined.push_str("//#file ");
        combined.push_str(&file.path.to_string_lossy());
        combined.push('\n');
        combined.push_str(&file.contents);
        if !combined.ends_with('\n') {
            combined.push('\n');
        }
    }

    Ok(DefinitionScript { files, combined })
}

fn collect_script_files(
    group: &Group,
    prefix: &Path,
    files: &mut Vec<DefinitionScriptFile>,
) -> Result<(), DefinitionError> {
    let entries = group.entries().map_err(DefinitionError::Resources)?;
    for entry in entries {
        let mut relative_path = PathBuf::from(prefix);
        relative_path.push(&entry.relative_path);
        if entry.is_directory {
            let child = group
                .open_child(&entry.relative_path)
                .map_err(DefinitionError::Resources)?;
            if child.exists("DefCore.txt") {
                continue;
            }
            collect_script_files(&child, &relative_path, files)?;
            continue;
        }
        if !is_script_file(&entry.relative_path) {
            continue;
        }
        let data = group
            .read_file(&entry.relative_path)
            .map_err(DefinitionError::Resources)?;
        let contents = String::from_utf8(data).map_err(|err| DefinitionError::ScriptEncoding {
            path: relative_path.clone(),
            source: err,
        })?;
        files.push(DefinitionScriptFile {
            path: relative_path,
            contents,
        });
    }
    Ok(())
}

fn is_script_file(path: &Path) -> bool {
    let Some(extension) = path.extension() else {
        return false;
    };
    if extension.eq_ignore_ascii_case("c") {
        return true;
    }
    false
}

fn parse_act_map(bytes: &[u8]) -> Result<ActionMap, DefinitionError> {
    let text = String::from_utf8_lossy(bytes);
    let mut default_action: Option<String> = None;
    let mut actions: HashMap<String, ActionDefinition> = HashMap::new();
    let mut current_name: Option<String> = None;
    let mut current_definition = ActionDefinition::default();

    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty()
            || line.starts_with(';')
            || line.starts_with('#')
            || line.starts_with("//")
        {
            continue;
        }

        if line.starts_with('[') && line.ends_with(']') {
            if let Some(name) = current_name.take() {
                actions.insert(name, current_definition);
            }
            current_definition = ActionDefinition::default();
            continue;
        }

        let Some((raw_key, raw_value)) = line.split_once('=') else {
            continue;
        };
        let key = raw_key.trim();
        let value = raw_value.trim();

        if key.eq_ignore_ascii_case("Name") {
            if !value.is_empty() {
                if let Some(name) = current_name.replace(value.to_string()) {
                    actions.insert(name, current_definition);
                    current_definition = ActionDefinition::default();
                }
            }
            continue;
        }

        if key.eq_ignore_ascii_case("Default") {
            if !value.is_empty() {
                default_action = Some(value.to_string());
            }
            continue;
        }

        match key.to_ascii_lowercase().as_str() {
            "procedure" => {
                if !value.is_empty() {
                    current_definition.procedure = Some(value.to_string());
                }
            }
            "length" => {
                current_definition.length = parse_u32(value);
            }
            "nextaction" => {
                if !value.is_empty() {
                    current_definition.next_action = Some(value.to_string());
                }
            }
            "delay" => {
                current_definition.delay = parse_u32(value);
            }
            "step" => {
                current_definition.step = parse_u32(value);
            }
            "phasecall" => {
                if !value.is_empty() {
                    current_definition.phase_call = Some(value.to_string());
                }
            }
            "startcall" => {
                if !value.is_empty() {
                    current_definition.start_call = Some(value.to_string());
                }
            }
            "endcall" => {
                if !value.is_empty() {
                    current_definition.end_call = Some(value.to_string());
                }
            }
            "abortcall" => {
                if !value.is_empty() {
                    current_definition.abort_call = Some(value.to_string());
                }
            }
            "nootheraction" => {
                current_definition.no_other_action = parse_bool(value);
            }
            "digfree" => {
                current_definition.dig_free = parse_i32(value);
            }
            _ => {}
        }
    }

    if let Some(name) = current_name {
        actions.insert(name, current_definition);
    }

    if actions.is_empty() && default_action.is_some() {
        return Err(DefinitionError::ActMapParse(
            "ActMap.txt declared a default action but no actions".into(),
        ));
    }

    Ok(ActionMap {
        default_action,
        actions,
    })
}

fn load_definition_picture(group: &Group, core: &DefCore) -> Option<GraphicsImage> {
    let path = find_picture_entry(group).ok().flatten()?;
    let data = group.read_file(&path).ok()?;
    let image = image::load_from_memory(&data).ok()?.into_rgba8();
    let (width, height) = image.dimensions();
    if width == 0 || height == 0 {
        return None;
    }

    let (crop_x, crop_y, crop_w, crop_h) = match core.picture {
        Some(rect) => normalize_crop(rect, width, height).unwrap_or((0, 0, width, height)),
        None => (0, 0, width, height),
    };

    let pixels = extract_rgba_region(&image, crop_x, crop_y, crop_w, crop_h);
    Some(GraphicsImage::new(crop_w, crop_h, pixels))
}

fn find_picture_entry(group: &Group) -> Result<Option<PathBuf>, GroupError> {
    const PRIORITY_FILES: [&str; 4] = ["Graphics32.png", "Graphics.png", "Picture.png", "Icon.png"];
    for candidate in PRIORITY_FILES {
        if group.exists(candidate) {
            return Ok(Some(PathBuf::from(candidate)));
        }
    }

    const PRIORITY_GROUPS: [&str; 3] = ["Graphics.ocg", "Graphics.c4d", "Graphics.c4g"];
    for candidate in PRIORITY_GROUPS {
        if let Ok(child) = group.open_child(candidate) {
            if let Some(found) = find_picture_entry(&child)? {
                let mut combined = PathBuf::from(candidate);
                combined.push(found);
                return Ok(Some(combined));
            }
        }
    }

    find_picture_entry_recursive(group, PathBuf::new())
}

fn find_picture_entry_recursive(
    group: &Group,
    base: PathBuf,
) -> Result<Option<PathBuf>, GroupError> {
    for entry in group.entries()? {
        let mut combined = base.clone();
        combined.push(&entry.relative_path);
        if entry.is_directory {
            let child = group.open_child(&entry.relative_path)?;
            if let Some(found) = find_picture_entry_recursive(&child, combined.clone())? {
                return Ok(Some(found));
            }
        } else if is_image_path(&entry.relative_path) {
            return Ok(Some(combined));
        }
    }
    Ok(None)
}

fn load_definition_graphics(group: &Group) -> Option<GraphicsImage> {
    let path = find_graphics_entry(group).ok().flatten()?;
    let data = group.read_file(&path).ok()?;
    let image = image::load_from_memory(&data).ok()?.into_rgba8();
    let (width, height) = image.dimensions();
    if width == 0 || height == 0 {
        return None;
    }
    Some(GraphicsImage::new(width, height, image.into_raw()))
}

fn find_graphics_entry(group: &Group) -> Result<Option<PathBuf>, GroupError> {
    const PRIORITY_FILES: [&str; 4] = [
        "Graphics32.png",
        "Graphics64.png",
        "Graphics.png",
        "Graphics.bmp",
    ];
    for candidate in PRIORITY_FILES {
        if group.exists(candidate) {
            return Ok(Some(PathBuf::from(candidate)));
        }
    }

    const PRIORITY_GROUPS: [&str; 3] = ["Graphics.ocg", "Graphics.c4d", "Graphics.c4g"];
    for candidate in PRIORITY_GROUPS {
        if let Ok(child) = group.open_child(candidate) {
            if let Some(found) =
                find_graphics_entry_recursive(&child, PathBuf::from(candidate), true)?
            {
                return Ok(Some(found));
            }
        }
    }

    find_graphics_entry_recursive(group, PathBuf::new(), false)
}

fn find_graphics_entry_recursive(
    group: &Group,
    base: PathBuf,
    in_graphics_dir: bool,
) -> Result<Option<PathBuf>, GroupError> {
    for entry in group.entries()? {
        let mut combined = base.clone();
        combined.push(&entry.relative_path);
        let name_lower = entry
            .relative_path
            .file_name()
            .map(|name| name.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        let next_in_graphics_dir =
            in_graphics_dir || name_lower.contains("graphics") || name_lower.starts_with("gfx");
        if entry.is_directory {
            let child = group.open_child(&entry.relative_path)?;
            if let Some(found) =
                find_graphics_entry_recursive(&child, combined.clone(), next_in_graphics_dir)?
            {
                return Ok(Some(found));
            }
        } else if next_in_graphics_dir && is_image_path(&entry.relative_path) {
            return Ok(Some(combined));
        }
    }
    Ok(None)
}

fn is_image_path(path: &Path) -> bool {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some(ext) => matches!(
            ext.to_ascii_lowercase().as_str(),
            "png" | "bmp" | "jpg" | "jpeg" | "tga"
        ),
        None => false,
    }
}

fn normalize_crop(
    rect: PictureRect,
    image_width: u32,
    image_height: u32,
) -> Option<(u32, u32, u32, u32)> {
    let width = rect.width.max(0) as u32;
    let height = rect.height.max(0) as u32;
    if width == 0 || height == 0 {
        return None;
    }
    let mut x = rect.x.max(0) as u32;
    let mut y = rect.y.max(0) as u32;
    if x >= image_width || y >= image_height {
        return None;
    }
    if x + width > image_width {
        x = x.min(image_width.saturating_sub(1));
    }
    if y + height > image_height {
        y = y.min(image_height.saturating_sub(1));
    }
    let crop_width = width.min(image_width - x);
    let crop_height = height.min(image_height - y);
    Some((x, y, crop_width.max(1), crop_height.max(1)))
}

fn extract_rgba_region(
    image: &image::RgbaImage,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
) -> Vec<u8> {
    let stride = image.width();
    let mut output = Vec::with_capacity((width * height * 4) as usize);
    for row in y..(y + height) {
        let row_start = ((row * stride) + x) as usize * 4;
        let row_end = row_start + (width as usize * 4);
        output.extend_from_slice(&image.as_raw()[row_start..row_end]);
    }
    output
}

fn parse_category(value: &str) -> Result<i32, DefinitionError> {
    let mut result: i32 = 0;
    if value.is_empty() {
        return Ok(result);
    }

    for token in value.split(|c: char| c == '|' || c == '+' || c == ',' || c.is_whitespace()) {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        if let Some(flag) = category_flag(token) {
            result |= flag;
            continue;
        }
        if token.starts_with("C4D_") {
            return Err(DefinitionError::UnknownCategoryFlag(token.to_string()));
        }
        let parsed = parse_i32(token)
            .ok_or_else(|| DefinitionError::InvalidCategoryValue(token.to_string()))?;
        result |= parsed;
    }

    Ok(result)
}

fn category_flag(token: &str) -> Option<i32> {
    let normalized = token.trim();
    for (name, value) in CATEGORY_FLAGS {
        if normalized.eq_ignore_ascii_case(name) {
            return Some(*value);
        }
    }
    None
}

fn parse_bool(value: &str) -> bool {
    let lower = value.trim().to_ascii_lowercase();
    matches!(lower.as_str(), "1" | "true" | "yes" | "on")
}

fn parse_u32(value: &str) -> Option<u32> {
    parse_i64(value).and_then(|num| if num < 0 { None } else { Some(num as u32) })
}

fn parse_rect(value: &str) -> Option<PictureRect> {
    let mut parts = value
        .split(|c: char| c == ',' || c == ';')
        .map(|part| part.trim())
        .filter(|part| !part.is_empty());
    let x = parse_i32(parts.next()?)?;
    let y = parse_i32(parts.next()?)?;
    let width = parse_i32(parts.next()?)?;
    let height = parse_i32(parts.next()?)?;
    Some(PictureRect {
        x,
        y,
        width,
        height,
    })
}

fn parse_i32(value: &str) -> Option<i32> {
    parse_i64(value).and_then(|num| num.try_into().ok())
}

fn parse_i64(value: &str) -> Option<i64> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(rest) = trimmed.strip_prefix("0x") {
        i64::from_str_radix(rest, 16).ok()
    } else if let Some(rest) = trimmed.strip_prefix("$") {
        i64::from_str_radix(rest, 16).ok()
    } else if let Some(rest) = trimmed.strip_prefix("0b") {
        i64::from_str_radix(rest, 2).ok()
    } else {
        trimmed.parse().ok()
    }
}

const CATEGORY_FLAGS: &[(&str, i32)] = &[
    ("C4D_None", 0),
    ("C4D_All", !0),
    ("C4D_StaticBack", 1 << 0),
    ("C4D_Structure", 1 << 1),
    ("C4D_Vehicle", 1 << 2),
    ("C4D_Living", 1 << 3),
    ("C4D_Object", 1 << 4),
    (
        "C4D_SortLimit",
        (1 << 0) | (1 << 1) | (1 << 2) | (1 << 3) | (1 << 4),
    ),
    ("C4D_Goal", 1 << 5),
    ("C4D_Environment", 1 << 6),
    ("C4D_SelectBuilding", 1 << 7),
    ("C4D_SelectVehicle", 1 << 8),
    ("C4D_SelectMaterial", 1 << 9),
    ("C4D_SelectKnowledge", 1 << 10),
    ("C4D_SelectHomebase", 1 << 11),
    ("C4D_SelectAnimal", 1 << 12),
    ("C4D_SelectNest", 1 << 13),
    ("C4D_SelectInEarth", 1 << 14),
    ("C4D_SelectVegetation", 1 << 15),
    ("C4D_TradeLiving", 1 << 16),
    ("C4D_Magic", 1 << 17),
    ("C4D_CrewMember", 1 << 18),
    ("C4D_Rule", 1 << 19),
    ("C4D_Background", 1 << 20),
    ("C4D_Parallax", 1 << 21),
    ("C4D_MouseSelect", 1 << 22),
    ("C4D_Foreground", 1 << 23),
    ("C4D_MouseIgnore", 1 << 24),
    ("C4D_IgnoreFoW", 1 << 25),
    ("C4D_BackgroundOrForeground", (1 << 20) | (1 << 23)),
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};
    use tempfile::tempdir;

    #[test]
    fn parse_def_core_basic_fields() {
        let data = br#"
            [DefCore]
            id=CLNK
            Name=Clonk
            Category=C4D_Living|C4D_Object
            CrewMember=1
        "#;
        let parsed = parse_def_core(data).expect("defcore parsed");
        assert_eq!(parsed.id, "CLNK");
        assert_eq!(parsed.name.as_deref(), Some("Clonk"));
        assert_eq!(parsed.category, (1 << 3) | (1 << 4));
        assert!(parsed.crew_member);
    }

    #[test]
    fn load_definition_with_scripts_and_actions() {
        let temp = tempdir().unwrap();
        let def_dir = temp.path().join("Example.ocd");
        fs::create_dir(&def_dir).unwrap();
        fs::write(
            def_dir.join("DefCore.txt"),
            br#"[DefCore]
id=EXMP
Name=Example
Category=C4D_Object
CrewMember=0
"#,
        )
        .unwrap();
        fs::write(def_dir.join("Script.c"), b"func Initialize() {}\n").unwrap();
        fs::write(
            def_dir.join("ActMap.txt"),
            br#"
[Action]
Name=Idle
Procedure=Walk
Length=20
NextAction=Idle
StartCall=OnIdleStart
EndCall=OnIdleEnd
"#,
        )
        .unwrap();

        let group = Group::open(&def_dir).unwrap();
        let def = Definition::load(&group).expect("definition load succeeds");
        assert_eq!(def.core.id, "EXMP");
        assert_eq!(def.core.name.as_deref(), Some("Example"));
        assert_eq!(def.core.category, 1 << 4);
        assert!(!def.core.crew_member);
        assert_eq!(def.script.files.len(), 1);
        assert!(def.script.combined.contains("Initialize"));
        let action_map = def.action_map.expect("action map present");
        assert!(action_map.default_action.is_none());
        let idle = action_map.actions.get("Idle").expect("idle action present");
        assert_eq!(idle.procedure.as_deref(), Some("Walk"));
        assert_eq!(idle.length, Some(20));
        assert_eq!(idle.next_action.as_deref(), Some("Idle"));
        assert_eq!(idle.start_call.as_deref(), Some("OnIdleStart"));
        assert_eq!(idle.end_call.as_deref(), Some("OnIdleEnd"));
    }

    #[test]
    fn parse_act_map_records_dig_free() {
        let data = br#"
[Action]
Name=Dig
Procedure=Dig
DigFree=24
"#;
        let map = parse_act_map(data).expect("act map parsed");
        let dig = map.actions.get("Dig").expect("dig action present");
        assert_eq!(dig.dig_free, Some(24));
    }

    #[test]
    fn parse_def_core_value_mass_picture() {
        let data = br#"
            [DefCore]
            id=VALU
            Name=Valuable
            Value=75
            Mass=12
            Picture=1,2,32,24
        "#;
        let parsed = parse_def_core(data).expect("defcore parsed");
        assert_eq!(parsed.value, 75);
        assert_eq!(parsed.mass, 12);
        assert_eq!(
            parsed.picture,
            Some(PictureRect {
                x: 1,
                y: 2,
                width: 32,
                height: 24
            })
        );
    }

    #[test]
    fn load_definition_collects_scripts_from_nested_groups() {
        let temp = tempdir().unwrap();
        let def_dir = temp.path().join("Nested.ocd");
        fs::create_dir(&def_dir).unwrap();
        fs::write(
            def_dir.join("DefCore.txt"),
            br#"[DefCore]
id=NNNN
Name=Nested
Category=C4D_Object
"#,
        )
        .unwrap();
        let script_dir = def_dir.join("Script.c4d");
        fs::create_dir(&script_dir).unwrap();
        fs::write(script_dir.join("Main.c"), b"func Main() {}\n").unwrap();
        let helpers_dir = script_dir.join("Helpers");
        fs::create_dir(&helpers_dir).unwrap();
        fs::write(helpers_dir.join("Util.c"), b"func Util() {}\n").unwrap();

        let group = Group::open(&def_dir).unwrap();
        let definition = Definition::load(&group).expect("definition load succeeds");
        assert!(definition
            .script
            .files()
            .iter()
            .any(|file| file.path == Path::new("Script.c4d").join("Main.c")));
        assert!(definition
            .script
            .files()
            .iter()
            .any(|file| file.path == Path::new("Script.c4d").join("Helpers").join("Util.c")));
    }

    #[test]
    fn load_definition_ignores_nested_definitions() {
        let temp = tempdir().unwrap();
        let def_dir = temp.path().join("Parent.ocd");
        fs::create_dir(&def_dir).unwrap();
        fs::write(
            def_dir.join("DefCore.txt"),
            br#"[DefCore]
id=PARA
Name=Parent
Category=C4D_Object
"#,
        )
        .unwrap();
        fs::write(def_dir.join("Script.c"), b"func Parent() {}\n").unwrap();
        let nested = def_dir.join("Child.ocd");
        fs::create_dir(&nested).unwrap();
        fs::write(
            nested.join("DefCore.txt"),
            br#"[DefCore]
id=CHLD
Name=Child
Category=C4D_Object
"#,
        )
        .unwrap();
        fs::write(nested.join("Script.c"), b"func Child() {}\n").unwrap();

        let group = Group::open(&def_dir).unwrap();
        let definition = Definition::load(&group).expect("definition load succeeds");
        assert_eq!(definition.script.files.len(), 1);
        assert_eq!(definition.script.files[0].path, PathBuf::from("Script.c"));
    }
}
