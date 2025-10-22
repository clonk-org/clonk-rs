use crate::{Group, GroupError};
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MaterialError {
    #[error("material resource error: {0}")]
    Resources(#[from] GroupError),
    #[error("material data is not valid UTF-8")]
    Encoding,
    #[error("material entry missing required name (index {index})")]
    MissingName { index: usize },
    #[error("duplicate material `{0}`")]
    DuplicateName(String),
    #[error("no material definitions found in resource")]
    NotFound,
}

#[derive(Debug, Clone)]
pub struct MaterialLibrary {
    materials: Vec<MaterialDefinition>,
    by_name: HashMap<String, usize>,
}

impl MaterialLibrary {
    pub fn parse(source: &str) -> Result<Self, MaterialError> {
        let parser = MaterialParser::new(source);
        let parsed = parser.parse()?;
        Self::from_definitions(parsed)
    }

    pub fn from_group(group: &Group) -> Result<Self, MaterialError> {
        let mut collected = Vec::new();
        let mut visited = HashSet::new();
        collect_materials_recursive(group, &mut collected, &mut visited)?;
        if collected.is_empty() {
            return Err(MaterialError::NotFound);
        }
        Self::from_definitions(collected)
    }

    pub fn iter(&self) -> impl Iterator<Item = &MaterialDefinition> {
        self.materials.iter()
    }

    pub fn get(&self, name: &str) -> Option<&MaterialDefinition> {
        self.by_name
            .get(&normalize_key(name))
            .and_then(|&index| self.materials.get(index))
    }

    pub fn is_empty(&self) -> bool {
        self.materials.is_empty()
    }

    fn from_definitions(definitions: Vec<MaterialDefinition>) -> Result<Self, MaterialError> {
        let mut by_name = HashMap::with_capacity(definitions.len());
        for (index, material) in definitions.iter().enumerate() {
            let key = normalize_key(&material.name);
            if by_name.insert(key.clone(), index).is_some() {
                return Err(MaterialError::DuplicateName(material.name.clone()));
            }
        }
        Ok(Self {
            materials: definitions,
            by_name,
        })
    }
}

#[derive(Debug, Clone)]
pub struct MaterialDefinition {
    name: String,
    properties: HashMap<String, Vec<String>>,
}

impl MaterialDefinition {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn values(&self, key: &str) -> Option<&[String]> {
        self.properties
            .get(&normalize_key(key))
            .map(|values| values.as_slice())
    }

    pub fn value(&self, key: &str) -> Option<&str> {
        self.values(key)
            .and_then(|values| values.first().map(|s| s.as_str()))
    }

    pub fn int(&self, key: &str) -> Option<i32> {
        self.value(key).and_then(parse_i32)
    }

    pub fn int_list(&self, key: &str) -> Option<Vec<i32>> {
        self.value(key).and_then(parse_int_list)
    }

    pub fn bool_flag(&self, key: &str) -> Option<bool> {
        self.value(key).and_then(parse_bool_flag)
    }

    pub fn strings(&self, key: &str) -> &[String] {
        self.properties
            .get(&normalize_key(key))
            .map(|values| values.as_slice())
            .unwrap_or(&[])
    }

    pub fn raw_properties(&self) -> &HashMap<String, Vec<String>> {
        &self.properties
    }
}

struct MaterialRecord {
    name_hint: Option<String>,
    properties: HashMap<String, Vec<String>>,
}

struct MaterialParser<'a> {
    source: &'a str,
    records: Vec<MaterialRecord>,
    current: Option<MaterialRecord>,
}

impl<'a> MaterialParser<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source,
            records: Vec::new(),
            current: None,
        }
    }

    fn parse(mut self) -> Result<Vec<MaterialDefinition>, MaterialError> {
        for raw_line in self.source.lines() {
            let mut line = raw_line.trim();
            if line.is_empty() {
                continue;
            }
            line = strip_comments(line);
            if line.is_empty() {
                continue;
            }

            if let Some(section) = parse_section_header(line) {
                self.finish_current()?;
                self.current = Some(MaterialRecord {
                    name_hint: section,
                    properties: HashMap::new(),
                });
                continue;
            }

            if let Some((key, value)) = parse_key_value(line) {
                let entry = self.current.get_or_insert_with(|| MaterialRecord {
                    name_hint: None,
                    properties: HashMap::new(),
                });
                let normalized_key = normalize_key(key);
                entry
                    .properties
                    .entry(normalized_key)
                    .or_insert_with(Vec::new)
                    .push(value.trim().to_string());
            }
        }
        self.finish_current()?;

        let mut definitions = Vec::with_capacity(self.records.len());
        for (index, record) in self.records.into_iter().enumerate() {
            let mut name = record.name_hint.clone();
            if let Some(values) = record.properties.get("name") {
                if let Some(first) = values.first() {
                    if !first.trim().is_empty() {
                        name = Some(first.trim().to_string());
                    }
                }
            }
            let Some(name) = name else {
                return Err(MaterialError::MissingName { index });
            };
            definitions.push(MaterialDefinition {
                name,
                properties: record.properties,
            });
        }
        Ok(definitions)
    }

    fn finish_current(&mut self) -> Result<(), MaterialError> {
        if let Some(record) = self.current.take() {
            self.records.push(record);
        }
        Ok(())
    }
}

fn collect_materials_recursive(
    group: &Group,
    out: &mut Vec<MaterialDefinition>,
    visited: &mut HashSet<PathBuf>,
) -> Result<(), MaterialError> {
    for entry in group.entries()? {
        if entry.is_directory {
            let child = group.open_child(&entry.relative_path)?;
            collect_materials_recursive(&child, out, visited)?;
            continue;
        }
        if is_group_container(&entry.relative_path) {
            let child = group.open_child(&entry.relative_path)?;
            collect_materials_recursive(&child, out, visited)?;
            continue;
        }
        if !is_material_candidate(&entry.relative_path) {
            continue;
        }
        let normalized = normalize_components(&entry.relative_path);
        if !visited.insert(normalized.clone()) {
            continue;
        }
        let bytes = group.read_file(&entry.relative_path)?;
        let text = match String::from_utf8(bytes) {
            Ok(text) => text,
            Err(err) => String::from_utf8_lossy(&err.into_bytes()).into_owned(),
        };
        let parser = MaterialParser::new(&text);
        let definitions = parser.parse()?;
        out.extend(definitions);
    }
    Ok(())
}

fn is_material_candidate(path: &Path) -> bool {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some(ext) if ext.eq_ignore_ascii_case("c4m") => return true,
        Some(ext) if ext.eq_ignore_ascii_case("txt") => {}
        _ => return false,
    }
    match path.file_stem().and_then(|stem| stem.to_str()) {
        Some(stem) if stem.eq_ignore_ascii_case("material") => true,
        Some(_) => false,
        None => false,
    }
}

fn parse_section_header(line: &str) -> Option<Option<String>> {
    if !line.starts_with('[') || !line.ends_with(']') {
        return None;
    }
    let inner = &line[1..line.len() - 1];
    let mut parts = inner.split_whitespace();
    let section = parts.next()?.trim();
    if !section.eq_ignore_ascii_case("material") {
        return None;
    }
    let rest: Vec<&str> = parts.collect();
    if rest.is_empty() {
        Some(None)
    } else {
        Some(Some(rest.join(" ")))
    }
}

fn parse_key_value(line: &str) -> Option<(&str, &str)> {
    let mut split = line.splitn(2, '=');
    let key = split.next()?.trim();
    let value = split.next()?.trim();
    if key.is_empty() {
        return None;
    }
    Some((key, value))
}

fn strip_comments(line: &str) -> &str {
    let mut bytes = line.as_bytes();
    if let Some(index) = find_comment_start(bytes) {
        bytes = &bytes[..index];
    }
    std::str::from_utf8(bytes).unwrap_or("").trim()
}

fn find_comment_start(bytes: &[u8]) -> Option<usize> {
    let mut idx = 0;
    while idx + 1 < bytes.len() {
        match bytes[idx] {
            b';' => {
                if idx == 0 {
                    return Some(idx);
                }
                let prefix = &bytes[..idx];
                if prefix.iter().all(|b| b.is_ascii_whitespace()) {
                    return Some(idx);
                }
            }
            b'/' if bytes[idx + 1] == b'/' => {
                if idx == 0 || bytes[idx - 1].is_ascii_whitespace() {
                    return Some(idx);
                }
            }
            _ => {}
        }
        idx += 1;
    }
    None
}

fn normalize_key(key: &str) -> String {
    key.trim().to_ascii_lowercase()
}

fn parse_i32(value: &str) -> Option<i32> {
    let trimmed = value.trim();
    if trimmed.starts_with("0x") || trimmed.starts_with("0X") {
        i32::from_str_radix(
            trimmed.trim_start_matches("0x").trim_start_matches("0X"),
            16,
        )
        .ok()
    } else {
        trimmed.parse::<i32>().ok()
    }
}

fn parse_int_list(value: &str) -> Option<Vec<i32>> {
    let mut result = Vec::new();
    for segment in value.split(|c| c == ',' || c == ';') {
        let trimmed = segment.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Some(parsed) = parse_i32(trimmed) else {
            return None;
        };
        result.push(parsed);
    }
    Some(result)
}

fn parse_bool_flag(value: &str) -> Option<bool> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.eq_ignore_ascii_case("true") {
        return Some(true);
    }
    if trimmed.eq_ignore_ascii_case("false") {
        return Some(false);
    }
    parse_i32(trimmed).map(|num| num != 0)
}

fn normalize_components(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            other => result.push(other),
        }
    }
    result
}

fn is_group_container(path: &Path) -> bool {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some(ext)
            if matches!(
                ext.to_ascii_lowercase().as_str(),
                "c4g" | "c4d" | "ocg" | "c4f" | "c4p"
            ) =>
        {
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_materials() -> &'static str {
        r#"
        // Example material definitions
        [Material Earth]
        Name = Earth
        Color = 120, 80, 40, 90, 60, 30, 60, 40, 20
        Alpha = 255, 255, 200, 180, 120, 80
        Density = 100
        Friction = 50
        DigFree = 1
        BlastFree = 0
        Inflammable = 0
        SplashRate = 5

        [Material Water]
        Name = Water
        Color = 30, 60, 180, 30, 60, 140, 20, 40, 120
        Alpha = 200, 180, 160, 120, 90, 60
        Density = 50
        Friction = 5
        DigFree = 0
        BlastFree = 0
        Inflammable = 0
        SplashRate = 15
        Reaction = Insert
        "#
    }

    #[test]
    fn parse_sample_materials() {
        let library = MaterialLibrary::parse(sample_materials()).expect("parse");
        assert_eq!(library.materials.len(), 2);
        let earth = library.get("earth").expect("earth");
        assert_eq!(earth.name(), "Earth");
        assert_eq!(
            earth.int_list("color").expect("color list"),
            vec![120, 80, 40, 90, 60, 30, 60, 40, 20]
        );
        assert_eq!(earth.int("density"), Some(100));
        assert_eq!(earth.bool_flag("digfree"), Some(true));

        let water = library.get("Water").expect("water");
        assert_eq!(water.int("friction"), Some(5));
        assert_eq!(water.strings("reaction"), &["Insert"]);
    }

    #[test]
    fn parse_without_section_name_uses_name_property() {
        let source = r#"
        [Material]
        Name=Rock
        Density=120
        "#;
        let library = MaterialLibrary::parse(source).expect("parse");
        let rock = library.get("rock").expect("rock");
        assert_eq!(rock.int("density"), Some(120));
    }

    #[test]
    fn parse_hex_values() {
        let source = r#"
        [Material]
        Name=HexMat
        Density=0x10
        "#;
        let library = MaterialLibrary::parse(source).expect("parse");
        let mat = library.get("hexmat").expect("hex");
        assert_eq!(mat.int("density"), Some(16));
    }
}
