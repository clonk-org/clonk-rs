use crate::{Group, GroupError};
use std::collections::HashMap;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MaterialError {
    #[error("material resource error: {0}")]
    Resources(#[from] GroupError),
    #[error("material data is not valid UTF-8")]
    Encoding,
    #[error("reaction section encountered before any material definition")]
    ReactionBeforeMaterial,
    #[error("material entry missing required name (index {index})")]
    MissingName { index: usize },
    #[error("duplicate material `{0}`")]
    DuplicateName(String),
    #[error("no material definitions found in resource")]
    NotFound,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum MaterialEnumerationError {
    #[error("material enumeration is missing the exact `[Enumeration]` header")]
    MissingHeader,
    #[error("material enumeration references unavailable material `{0}`")]
    MissingMaterial(String),
}

/// A savegame `MatMap.txt` material-index ledger
/// (`C4MaterialMap::LoadEnumeration`, C4Material.cpp:510-558).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterialEnumeration {
    names: Vec<String>,
}

impl MaterialEnumeration {
    const HEADER: &'static [u8] = b"[Enumeration]";
    const MAX_NAME_BYTES: usize = 15;

    pub fn parse(source: &[u8]) -> Result<Self, MaterialEnumerationError> {
        // LoadEntryString exposes a C string to SSearch/SCopyIdentifier;
        // bytes after the first NUL are not visible to the native parser.
        let source = source
            .split(|byte| *byte == 0)
            .next()
            .unwrap_or_default();
        let Some(header) = source
            .windows(Self::HEADER.len())
            .position(|window| window == Self::HEADER)
        else {
            return Err(MaterialEnumerationError::MissingHeader);
        };
        let mut remaining = &source[header + Self::HEADER.len()..];
        skip_enumeration_whitespace(&mut remaining);
        let mut names = Vec::new();
        while remaining.first().is_some_and(|byte| enumeration_identifier(*byte)) {
            // SCopyIdentifier caps each token at C4M_MaxName. A longer raw
            // identifier therefore continues as another token, because the
            // unconsumed suffix still begins with an identifier byte.
            let length = remaining
                .iter()
                .take_while(|byte| enumeration_identifier(**byte))
                .take(Self::MAX_NAME_BYTES)
                .count();
            names.push(
                String::from_utf8(remaining[..length].to_vec())
                    .expect("material enumeration identifiers are ASCII"),
            );
            remaining = &remaining[length..];
            skip_enumeration_whitespace(&mut remaining);
        }
        Ok(Self { names })
    }

    pub fn from_names<'a>(names: impl IntoIterator<Item = &'a str>) -> Self {
        Self {
            names: names.into_iter().map(str::to_owned).collect(),
        }
    }

    pub fn names(&self) -> &[String] {
        &self.names
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// `C4MaterialMap::SaveEnumeration`: exact header/name CRLF framing and
    /// the trailing `EndOfFile` byte (`"\x020"` is ASCII space).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Self::HEADER.to_vec();
        bytes.extend_from_slice(b"\r\n");
        for name in &self.names {
            bytes.extend_from_slice(name.as_bytes());
            bytes.extend_from_slice(b"\r\n");
        }
        bytes.push(b' ');
        bytes
    }
}

fn enumeration_identifier(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'~' | b'+' | b'-')
}

fn skip_enumeration_whitespace(source: &mut &[u8]) {
    let count = source
        .iter()
        .take_while(|byte| matches!(**byte, b' ' | b'\t' | b'\r' | b'\n'))
        .count();
    *source = &source[count..];
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
        for entry in group.entries()? {
            if !is_material_file_name(&entry.name_bytes) {
                continue;
            }
            let bytes = group.read_entry_bytes_exact(&entry)?;
            let text = match String::from_utf8(bytes) {
                Ok(text) => text,
                Err(error) => String::from_utf8_lossy(&error.into_bytes()).into_owned(),
            };
            collected.push(MaterialParser::new(&text).parse_first()?);
        }
        if collected.is_empty() {
            return Err(MaterialError::NotFound);
        }
        Ok(Self::from_definitions_allow_duplicates(collected))
    }

    /// C4MaterialMap::Load overload semantics (C4Material.cpp:263-299):
    /// each load PREPENDS the materials whose names are new; earlier
    /// loads win name collisions. `loads` in LOAD order (scenario-local
    /// first, global after) yields [later-uniques…, …, earliest…] — the
    /// C++ final map order (dynamic texmap slots depend on it).
    pub fn from_overloaded_loads(loads: &[&MaterialLibrary]) -> Result<Self, MaterialError> {
        let mut merged: Vec<MaterialDefinition> = Vec::new();
        for load in loads {
            let fresh: Vec<MaterialDefinition> = load
                .iter()
                .filter(|definition| {
                    !merged.iter().any(|existing| {
                        existing
                            .name()
                            .eq_ignore_ascii_case(definition.name())
                    })
                })
                .cloned()
                .collect();
            merged.splice(0..0, fresh);
        }
        Ok(Self::from_definitions_allow_duplicates(merged))
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

    /// `C4MaterialMap::SortEnumeration`: for every requested index, search
    /// only the still-unordered suffix and swap the exact-case match into
    /// place. This is intentionally not a stable rank sort.
    pub fn sort_enumeration(
        &mut self,
        enumeration: &MaterialEnumeration,
    ) -> Result<(), MaterialEnumerationError> {
        let result = enumeration
            .names
            .iter()
            .enumerate()
            .try_for_each(|(requested_index, requested_name)| {
                let found_index = self
                    .materials
                    .get(requested_index..)
                    .and_then(|suffix| {
                        suffix
                            .iter()
                            .position(|material| material.name() == requested_name)
                    })
                    .map(|offset| requested_index + offset)
                    .ok_or_else(|| {
                        MaterialEnumerationError::MissingMaterial(requested_name.clone())
                    })?;
                self.materials.swap(requested_index, found_index);
                Ok(())
            });

        // Keep name lookup coherent even on the fatal partial-sort path.
        self.by_name = first_name_indices(&self.materials);
        result
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

    fn from_definitions_allow_duplicates(definitions: Vec<MaterialDefinition>) -> Self {
        let by_name = first_name_indices(&definitions);
        Self {
            materials: definitions,
            by_name,
        }
    }
}

fn first_name_indices(definitions: &[MaterialDefinition]) -> HashMap<String, usize> {
    let mut by_name = HashMap::with_capacity(definitions.len());
    for (index, material) in definitions.iter().enumerate() {
        by_name.entry(normalize_key(material.name())).or_insert(index);
    }
    by_name
}

fn is_material_file_name(name: &[u8]) -> bool {
    name.len() >= 4 && name[name.len() - 4..].eq_ignore_ascii_case(b".c4m")
}

#[derive(Debug, Clone)]
pub struct MaterialDefinition {
    name: String,
    properties: HashMap<String, Vec<String>>,
    reactions: Vec<MaterialReactionDefinition>,
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

    pub fn reactions(&self) -> &[MaterialReactionDefinition] {
        &self.reactions
    }
}

#[derive(Debug, Clone)]
pub struct MaterialReactionDefinition {
    properties: HashMap<String, Vec<String>>,
}

impl MaterialReactionDefinition {
    pub fn value(&self, key: &str) -> Option<&str> {
        self.properties
            .get(&normalize_key(key))
            .and_then(|values| values.first().map(|s| s.as_str()))
    }

    pub fn int(&self, key: &str) -> Option<i32> {
        self.value(key).and_then(parse_i32)
    }

    pub fn bool_flag(&self, key: &str) -> Option<bool> {
        self.value(key).and_then(parse_bool_flag)
    }

    pub fn raw_properties(&self) -> &HashMap<String, Vec<String>> {
        &self.properties
    }
}

struct MaterialRecord {
    name_hint: Option<String>,
    properties: HashMap<String, Vec<String>>,
    reactions: Vec<ReactionRecord>,
}

#[derive(Debug, Default, Clone)]
struct ReactionRecord {
    properties: HashMap<String, Vec<String>>,
}

struct MaterialParser<'a> {
    source: &'a str,
    records: Vec<MaterialRecord>,
    current: Option<MaterialRecord>,
    current_reaction: Option<ReactionRecord>,
}

impl<'a> MaterialParser<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source,
            records: Vec::new(),
            current: None,
            current_reaction: None,
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
                match section {
                    SectionHeader::Material(name_hint) => {
                        self.finish_current()?;
                        self.current = Some(MaterialRecord {
                            name_hint,
                            properties: HashMap::new(),
                            reactions: Vec::new(),
                        });
                    }
                    SectionHeader::Reaction => {
                        if self.current.is_none() {
                            return Err(MaterialError::ReactionBeforeMaterial);
                        }
                        self.finish_current_reaction()?;
                        self.current_reaction = Some(ReactionRecord::default());
                    }
                }
                continue;
            }

            if let Some((key, value)) = parse_key_value(line) {
                let target = if let Some(reaction) = self.current_reaction.as_mut() {
                    &mut reaction.properties
                } else {
                    let entry = self.current.get_or_insert_with(|| MaterialRecord {
                        name_hint: None,
                        properties: HashMap::new(),
                        reactions: Vec::new(),
                    });
                    &mut entry.properties
                };
                let normalized_key = normalize_key(key);
                target
                    .entry(normalized_key)
                    .or_insert_with(Vec::new)
                    .push(value.trim().to_string());
            }
        }
        self.finish_current_reaction()?;
        self.finish_current()?;

        let mut definitions = Vec::with_capacity(self.records.len());
        for (index, record) in self.records.into_iter().enumerate() {
            definitions.push(material_definition_from_record(record, index)?);
        }
        Ok(definitions)
    }

    /// C4MaterialCore compiles one `Material` namespace per `.c4m` file.
    /// Later material namespaces are ignored, while every root `Reaction`
    /// namespace still belongs to that single core.
    fn parse_first(self) -> Result<MaterialDefinition, MaterialError> {
        let mut first_material = None;
        let mut in_first_material = false;
        let mut reactions = Vec::new();
        let mut current_reaction = None;

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
                if let Some(reaction) = current_reaction.take() {
                    reactions.push(reaction);
                }
                match section {
                    SectionHeader::Material(name_hint) => {
                        if first_material.is_none() {
                            first_material = Some(MaterialRecord {
                                name_hint,
                                properties: HashMap::new(),
                                reactions: Vec::new(),
                            });
                            in_first_material = true;
                        } else {
                            in_first_material = false;
                        }
                    }
                    SectionHeader::Reaction => {
                        current_reaction = Some(ReactionRecord::default());
                        in_first_material = false;
                    }
                }
                continue;
            }

            if let Some((key, value)) = parse_key_value(line) {
                let target = if let Some(reaction) = current_reaction.as_mut() {
                    Some(&mut reaction.properties)
                } else if in_first_material {
                    first_material
                        .as_mut()
                        .map(|material| &mut material.properties)
                } else {
                    None
                };
                if let Some(target) = target {
                    target
                        .entry(normalize_key(key))
                        .or_insert_with(Vec::new)
                        .push(value.trim().to_string());
                }
            }
        }
        if let Some(reaction) = current_reaction {
            reactions.push(reaction);
        }
        let mut material = first_material.ok_or(MaterialError::NotFound)?;
        material.reactions = reactions;
        material_definition_from_record(material, 0)
    }

    fn finish_current(&mut self) -> Result<(), MaterialError> {
        self.finish_current_reaction()?;
        if let Some(record) = self.current.take() {
            self.records.push(record);
        }
        Ok(())
    }

    fn finish_current_reaction(&mut self) -> Result<(), MaterialError> {
        if let Some(reaction) = self.current_reaction.take() {
            if let Some(current) = self.current.as_mut() {
                current.reactions.push(reaction);
            } else {
                return Err(MaterialError::ReactionBeforeMaterial);
            }
        }
        Ok(())
    }
}

fn material_definition_from_record(
    record: MaterialRecord,
    index: usize,
) -> Result<MaterialDefinition, MaterialError> {
    let MaterialRecord {
        name_hint,
        properties,
        reactions,
    } = record;
    let mut name = name_hint;
    if let Some(first) = properties.get("name").and_then(|values| values.first()) {
        if !first.trim().is_empty() {
            name = Some(first.trim().to_string());
        }
    }
    let Some(name) = name else {
        return Err(MaterialError::MissingName { index });
    };
    Ok(MaterialDefinition {
        name,
        properties,
        reactions: reactions
            .into_iter()
            .map(|reaction| MaterialReactionDefinition {
                properties: reaction.properties,
            })
            .collect(),
    })
}

enum SectionHeader {
    Material(Option<String>),
    Reaction,
}

fn parse_section_header(line: &str) -> Option<SectionHeader> {
    if !line.starts_with('[') || !line.ends_with(']') {
        return None;
    }
    let inner = &line[1..line.len() - 1];
    let mut parts = inner.split_whitespace();
    let section = parts.next()?.trim();
    if section.eq_ignore_ascii_case("material") {
        let rest: Vec<&str> = parts.collect();
        if rest.is_empty() {
            Some(SectionHeader::Material(None))
        } else {
            Some(SectionHeader::Material(Some(rest.join(" "))))
        }
    } else if section.eq_ignore_ascii_case("reaction") {
        Some(SectionHeader::Reaction)
    } else {
        None
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
    for segment in value.split([',', ';']) {
        let trimmed = segment.trim();
        if trimmed.is_empty() {
            continue;
        }
        let parsed = parse_i32(trimmed)?;
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
    fn group_loads_only_top_level_c4m_first_material_and_keeps_duplicates() {
        let mut child = crate::MutableGroup::new("Child.c4g");
        child
            .add_file(
                "Hidden.c4m",
                b"[Material]\nName=Hidden\nDensity=99\n".to_vec(),
            )
            .expect("add nested material");

        // Use a nonstandard logical filename so pack_raw preserves this
        // explicit physical entry order instead of applying C4FLS_Material.
        let mut packed = crate::MutableGroup::new("Unsorted.bin");
        packed
            .add_file(
                "Z.c4m",
                b"[Material]\nName=First\nDensity=11\n\
                  [Material]\nName=Ignored\nDensity=99\n\
                  [Reaction]\nType=Poof\n"
                    .to_vec(),
            )
            .expect("add multi-material file");
        packed
            .add_child("Child.c4g", child)
            .expect("add nested group");
        packed
            .add_file(
                "Material.txt",
                b"[Material]\nName=TextOnly\nDensity=98\n".to_vec(),
            )
            .expect("add ignored Material.txt");
        packed
            .add_file(
                "A.C4M",
                b"[Material]\nName=Dup\nDensity=21\n".to_vec(),
            )
            .expect("add first duplicate");
        packed
            .add_file(
                "B.c4m",
                b"[Material]\nName=dUp\nDensity=22\n".to_vec(),
            )
            .expect("add second duplicate");
        let group = Group::from_raw_memory(
            std::path::PathBuf::from("Material.c4g"),
            packed.pack_raw().expect("pack material group"),
        )
        .expect("open material group");

        let library = MaterialLibrary::from_group(&group).expect("load material group");
        assert_eq!(
            library
                .iter()
                .map(MaterialDefinition::name)
                .collect::<Vec<_>>(),
            vec!["First", "Dup", "dUp"]
        );
        let first = library.get("first").expect("first material wins");
        assert_eq!(first.int("density"), Some(11));
        assert_eq!(first.reactions().len(), 1);
        assert_eq!(first.reactions()[0].value("type"), Some("Poof"));
        assert_eq!(
            library.get("DUP").and_then(|material| material.int("density")),
            Some(21),
            "case-insensitive lookup resolves the lower duplicate index"
        );

        let global = MaterialLibrary::parse("[Material]\nName=Global\nDensity=30\n")
            .expect("global material parses");
        let merged = MaterialLibrary::from_overloaded_loads(&[&library, &global])
            .expect("duplicate-bearing overload chain merges");
        assert_eq!(
            merged
                .iter()
                .map(MaterialDefinition::name)
                .collect::<Vec<_>>(),
            vec!["Global", "First", "Dup", "dUp"]
        );
        assert_eq!(
            merged.get("dup").and_then(|material| material.int("density")),
            Some(21)
        );

        let mut enumerated = library.clone();
        enumerated
            .sort_enumeration(&MaterialEnumeration::from_names(["dUp"]))
            .expect("duplicate enumeration sorts");
        assert_eq!(
            enumerated
                .get("dup")
                .and_then(|material| material.int("density")),
            Some(22),
            "lookup follows the lower duplicate after enumeration swaps"
        );
    }

    #[test]
    fn aggregate_parse_keeps_duplicate_name_validation() {
        assert!(matches!(
            MaterialLibrary::parse(
                "[Material]\nName=Same\n\n[Material]\nName=sAmE\n"
            ),
            Err(MaterialError::DuplicateName(name)) if name == "sAmE"
        ));
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

    #[test]
    fn parse_material_with_reaction_sections() {
        let source = r#"
        [Material]
        Name=Snow
        Density=50

        [Reaction]
        Type=Poof
        TargetSpec=Incindiary
        Reverse=1
        "#;
        let library = MaterialLibrary::parse(source).expect("parse");
        let snow = library.get("snow").expect("snow");
        assert_eq!(snow.reactions().len(), 1);
        let reaction = &snow.reactions()[0];
        assert_eq!(reaction.value("type"), Some("Poof"));
        assert_eq!(reaction.value("targetspec"), Some("Incindiary"));
        assert_eq!(reaction.bool_flag("reverse"), Some(true));
    }

    #[test]
    fn enumeration_uses_suffix_swaps_and_exact_case_names() {
        let mut library = MaterialLibrary::parse(
            "[Material A]\nName=A\n\n[Material B]\nName=B\n\n[Material C]\nName=C\n",
        )
        .expect("materials parse");
        let enumeration = MaterialEnumeration::parse(b"[Enumeration]\r\nC\r\n")
            .expect("enumeration parses");

        library
            .sort_enumeration(&enumeration)
            .expect("enumeration sorts");

        assert_eq!(
            library.iter().map(MaterialDefinition::name).collect::<Vec<_>>(),
            vec!["C", "B", "A"]
        );
        let wrong_case = MaterialEnumeration::parse(b"[Enumeration] c")
            .expect("lowercase enumeration parses");
        assert_eq!(
            library.sort_enumeration(&wrong_case),
            Err(MaterialEnumerationError::MissingMaterial("c".to_string()))
        );
    }

    #[test]
    fn enumeration_parser_cannot_see_a_header_after_nul() {
        assert_eq!(
            MaterialEnumeration::parse(b"\0[Enumeration]\r\nA\r\n"),
            Err(MaterialEnumerationError::MissingHeader)
        );
    }
}
