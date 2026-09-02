use crate::{
    definition::{
        ini_section_name, ini_value, parse_action_i32_prefix, parse_action_u64_prefix, parse_bool,
        parse_int_array,
    },
    Group, GroupError,
};
use std::collections::HashMap;
use thiserror::Error;

/// Native byte capacity of `C4Material::Name` and `C4Texture::Name`.
pub const C4M_MAX_NAME_BYTES: usize = 15;

fn c4_name_byte_key(byte: u8) -> u8 {
    match byte {
        b'A'..=b'Z' => byte + (b'a' - b'A'),
        0xc4 | 0xe4 => 0xe4,
        0xd6 | 0xf6 => 0xf6,
        0xdc | 0xfc => 0xfc,
        _ => byte,
    }
}

/// Canonical key for C++ `SEqualNoCase` material/texture-name comparisons.
///
/// The private-use projection used by `clonk-script` lets the surrounding Rust
/// APIs remain strings without replacing arbitrary native bytes.
#[doc(hidden)]
pub fn c4_name_key(name: &str) -> String {
    let bytes = clonk_script::c4_string_bytes_cow(name);
    let folded = c4_name_key_bytes(&bytes).collect::<Vec<_>>();
    clonk_script::c4_string_from_bytes(&folded)
}

/// The folded, NUL-terminated byte stream [`c4_name_key`] projects back into a
/// string. Comparing the stream avoids the owned key the texmap lookups would
/// otherwise build for every candidate name on every frame.
fn c4_name_key_bytes(bytes: &[u8]) -> impl Iterator<Item = u8> + '_ {
    bytes
        .iter()
        .copied()
        .take_while(|byte| *byte != 0)
        .map(c4_name_byte_key)
}

/// C++ `SEqualNoCase` over the native bytes represented by two Rust strings.
#[doc(hidden)]
pub fn c4_names_equal(left: &str, right: &str) -> bool {
    // `c4_string_from_bytes` is injective, so equal keys and equal folded
    // byte streams are the same predicate.
    let left = clonk_script::c4_string_bytes_cow(left);
    let right = clonk_script::c4_string_bytes_cow(right);
    c4_name_key_bytes(&left).eq(c4_name_key_bytes(&right))
}

/// Copy one native C string into the reversible Rust projection.
#[doc(hidden)]
pub fn c4_c_string(value: &str) -> String {
    let bytes = clonk_script::c4_string_bytes(value);
    let visible = bytes.split(|byte| *byte == 0).next().unwrap_or_default();
    clonk_script::c4_string_from_bytes(visible)
}

/// Store one fixed `C4M_MaxName + 1` identity, stopping at a native NUL and
/// truncating by native bytes rather than UTF-8 scalar boundaries.
#[doc(hidden)]
pub fn truncate_c4m_name(name: &str) -> String {
    let visible = c4_c_string(name);
    let bytes = clonk_script::c4_string_bytes(&visible);
    clonk_script::c4_string_from_bytes(&bytes[..bytes.len().min(C4M_MAX_NAME_BYTES)])
}

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
        let source = source.split(|byte| *byte == 0).next().unwrap_or_default();
        let Some(header) = source
            .windows(Self::HEADER.len())
            .position(|window| window == Self::HEADER)
        else {
            return Err(MaterialEnumerationError::MissingHeader);
        };
        let mut remaining = &source[header + Self::HEADER.len()..];
        skip_enumeration_whitespace(&mut remaining);
        let mut names = Vec::new();
        while remaining
            .first()
            .is_some_and(|byte| enumeration_identifier(*byte))
        {
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
            let name = c4_c_string(name);
            bytes.extend_from_slice(&clonk_script::c4_string_bytes(&name));
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
        Self::parse_bytes(&clonk_script::c4_string_bytes(source))
    }

    /// Parse the native `C4MaterialCore` source without a UTF-8 round trip.
    pub fn parse_bytes(source: &[u8]) -> Result<Self, MaterialError> {
        let visible = source.split(|byte| *byte == 0).next().unwrap_or_default();
        let source = clonk_script::c4_string_from_bytes(visible);
        let parser = MaterialParser::new(&source);
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
            let visible = bytes.split(|byte| *byte == 0).next().unwrap_or_default();
            let text = clonk_script::c4_string_from_bytes(visible);
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
                    !merged
                        .iter()
                        .any(|existing| c4_names_equal(existing.name(), definition.name()))
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
            .get(&c4_name_key(name))
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
        let result = enumeration.names.iter().enumerate().try_for_each(
            |(requested_index, requested_name)| {
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
            },
        );

        // Keep name lookup coherent even on the fatal partial-sort path.
        self.by_name = first_name_indices(&self.materials);
        result
    }

    fn from_definitions(definitions: Vec<MaterialDefinition>) -> Result<Self, MaterialError> {
        let mut by_name = HashMap::with_capacity(definitions.len());
        for (index, material) in definitions.iter().enumerate() {
            let key = c4_name_key(&material.name);
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
        by_name.entry(c4_name_key(material.name())).or_insert(index);
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
        self.value(key).map(|value| parse_int_list(key, value))
    }

    pub fn bool_flag(&self, key: &str) -> Option<bool> {
        self.int(key).map(|value| value != 0)
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
        self.value(key).and_then(|value| {
            if normalize_key(key) == "execmask" {
                parse_u32_bits(value)
            } else {
                parse_i32(value)
            }
        })
    }

    pub fn bool_flag(&self, key: &str) -> Option<bool> {
        self.value(key).and_then(parse_bool)
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
        Ok(parse_native_material(self.source))
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

#[derive(Clone, Copy)]
struct NativeIniNode<'a> {
    name: &'a str,
    value: &'a str,
    parent: usize,
    indent: usize,
    section: bool,
}

fn parse_native_material(source: &str) -> MaterialDefinition {
    let nodes = native_ini_name_tree(source);
    // StdCompilerINIRead::Name selects the first exact child without checking
    // whether it is a section. A scalar `Material=...` can therefore shadow
    // a later real section, and an absent namespace simply leaves defaults.
    let properties = nodes
        .iter()
        .position(|node| node.parent == 0 && node.name == "Material")
        .map(|index| native_properties(&nodes, index, is_material_compiler_key))
        .unwrap_or_default();
    let name = properties
        .get("name")
        .and_then(|values| values.first())
        .cloned()
        .unwrap_or_default();

    let mut reactions = Vec::new();
    for (index, node) in nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| node.parent == 0 && node.name == "Reaction")
    {
        reactions.push(MaterialReactionDefinition {
            properties: native_properties(&nodes, index, is_reaction_compiler_key),
        });
        // Repeated named-container lookup advances only from a section. A
        // scalar Reaction node loses its value cursor while its first missing
        // nested field unwinds, then blocks later root Reaction sections.
        if !node.section {
            break;
        }
    }

    MaterialDefinition {
        name,
        properties,
        reactions,
    }
}

fn native_ini_name_tree(source: &str) -> Vec<NativeIniNode<'_>> {
    let source = source.split_once('\0').map_or(source, |(head, _)| head);
    let mut nodes = vec![NativeIniNode {
        name: "",
        value: "",
        parent: 0,
        indent: 0,
        section: true,
    }];
    let mut current = 0;

    for raw_line in source.split(['\r', '\n']) {
        let indent = raw_line
            .as_bytes()
            .iter()
            .take_while(|byte| matches!(byte, b' ' | b'\t'))
            .count();
        let line = &raw_line[indent..];
        let section = line.as_bytes().first() == Some(&b'[')
            && line.as_bytes().get(1).is_some_and(u8::is_ascii_alphabetic);
        let value = line.as_bytes().first().is_some_and(u8::is_ascii_alphabetic);
        if !section && !value {
            continue;
        }
        let node_indent = indent.saturating_add(usize::from(!section));
        // CreateNameTree unwinds before validating the closing `]` or `=`.
        // A malformed named line can therefore return later indented fields
        // to an outer section even though no node is created for that line.
        while current != 0 && nodes[current].indent >= node_indent {
            current = nodes[current].parent;
        }

        let (name, value) = if section {
            let Some(name) = ini_section_name(line) else {
                continue;
            };
            let value = line
                .find(']')
                .map(|closing| &line[closing + 1..])
                .unwrap_or_default();
            (name, value)
        } else {
            let Some((name, value)) = ini_value(line) else {
                continue;
            };
            (name, value)
        };

        let parent = current;
        nodes.push(NativeIniNode {
            name,
            value,
            parent,
            indent: node_indent,
            section,
        });
        if section {
            current = nodes.len() - 1;
        }
    }
    nodes
}

fn native_properties(
    nodes: &[NativeIniNode<'_>],
    parent: usize,
    accepted: fn(&str) -> bool,
) -> HashMap<String, Vec<String>> {
    let mut properties = HashMap::new();
    for node in nodes.iter().filter(|node| node.parent == parent) {
        if !accepted(node.name) {
            continue;
        }
        properties
            .entry(normalize_key(node.name))
            .or_insert_with(|| vec![native_compiled_string(node.name, node.value)]);
    }
    for (name, len, unsigned) in [
        ("Color", 9, true),
        ("ColorX", 9, true),
        ("Alpha", 6, true),
        ("PXSGfxRt", 6, false),
    ] {
        if !accepted(name) {
            continue;
        }
        let Some(values) = native_fixed_array(nodes, parent, name, len, unsigned) else {
            continue;
        };
        properties.insert(
            normalize_key(name),
            vec![values
                .iter()
                .map(i32::to_string)
                .collect::<Vec<_>>()
                .join(",")],
        );
    }
    properties
}

fn native_fixed_array(
    nodes: &[NativeIniNode<'_>],
    parent: usize,
    name: &str,
    len: usize,
    unsigned: bool,
) -> Option<Vec<i32>> {
    let mut node_index = nodes
        .iter()
        .position(|node| node.parent == parent && node.name == name)?;
    let mut bytes = clonk_script::c4_string_bytes(nodes[node_index].value);
    let mut cursor = 0;
    let mut values = Vec::with_capacity(len);

    while values.len() < len {
        let parsed = if unsigned {
            parse_action_u64_prefix(&bytes[cursor..]).map(|(value, consumed)| {
                cursor += consumed;
                value as u32 as i32
            })
        } else {
            parse_action_i32_prefix(&bytes[cursor..]).map(|(value, consumed)| {
                cursor += consumed;
                value
            })
        };
        values.push(parsed.unwrap_or(0));
        if values.len() == len {
            break;
        }

        if nodes[node_index].section {
            let Some(next_index) = nodes
                .iter()
                .enumerate()
                .skip(node_index + 1)
                .find(|(_, node)| node.parent == parent && node.name == name)
                .map(|(index, _)| index)
            else {
                break;
            };
            node_index = next_index;
            bytes = clonk_script::c4_string_bytes(nodes[node_index].value);
            cursor = 0;
        } else {
            while bytes
                .get(cursor)
                .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
            {
                cursor += 1;
            }
            if bytes.get(cursor) != Some(&b',') {
                break;
            }
            cursor += 1;
        }
    }
    values.resize(len, 0);
    Some(values)
}

fn native_compiled_string(name: &str, value: &str) -> String {
    match name {
        "Name" => truncate_c4m_name(value.trim_start_matches([' ', '\t'])),
        "TextureOverlay" | "PXSGfx" | "BlastShiftTo" | "InMatConvert" | "InMatConvertTo"
        | "AboveTempConvertTo" | "BelowTempConvertTo" => native_identifier(value, usize::MAX),
        "Blast2Object" | "Dig2Object" => {
            let identifier = native_identifier(value, 4);
            let bytes = clonk_script::c4_string_bytes(&identifier);
            if bytes.len() == 4 && bytes != b"NONE" && bytes != b"0000" {
                identifier
            } else {
                String::default()
            }
        }
        "Type" | "TargetSpec" | "ScriptFunc" | "ConvertMat" => native_escaped_string(value),
        _ => value.to_string(),
    }
}

fn native_identifier(value: &str, max_len: usize) -> String {
    let bytes = clonk_script::c4_string_bytes(value.trim_start_matches([' ', '\t']));
    let len = bytes
        .iter()
        .take(max_len)
        .take_while(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        .count();
    clonk_script::c4_string_from_bytes(&bytes[..len])
}

fn native_escaped_string(value: &str) -> String {
    let bytes = clonk_script::c4_string_bytes(value);
    if bytes.first() != Some(&b'"') {
        let start = bytes
            .iter()
            .take_while(|byte| matches!(byte, b' ' | b'\t'))
            .count();
        return clonk_script::c4_string_from_bytes(&bytes[start..]);
    }

    let mut cursor = 1;
    let mut output = Vec::new();
    while let Some(&byte) = bytes.get(cursor) {
        if matches!(byte, b'"' | b'\0' | b'\r' | b'\n') {
            break;
        }
        if byte != b'\\' {
            output.push(byte);
            cursor += 1;
            continue;
        }

        cursor += 1;
        let Some(&escaped) = bytes.get(cursor) else {
            break;
        };
        cursor += 1;
        match escaped {
            b'a' => output.push(b'\x07'),
            b'b' => output.push(b'\x08'),
            b'f' => output.push(b'\x0c'),
            b'n' => output.push(b'\n'),
            b'r' => output.push(b'\r'),
            b't' => output.push(b'\t'),
            b'v' => output.push(b'\x0b'),
            b'\'' => output.push(b'\''),
            b'"' => output.push(b'"'),
            b'\\' => output.push(b'\\'),
            b'?' => output.push(b'?'),
            b'x' => {
                if !bytes.get(cursor).is_some_and(u8::is_ascii_hexdigit) {
                    output.push(b'x');
                    continue;
                }
                let mut code = 0i32;
                while let Some(&digit) = bytes.get(cursor) {
                    if !digit.is_ascii_hexdigit() {
                        break;
                    }
                    let value = if digit.is_ascii_digit() {
                        i32::from(digit - b'0')
                    } else {
                        // Match the native implementation literally: it
                        // subtracts lowercase `a` even for uppercase hex.
                        i32::from(digit) - i32::from(b'a') + 10
                    };
                    code = code.wrapping_mul(16).wrapping_add(value);
                    cursor += 1;
                }
                output.push(code as u8);
            }
            first @ b'0'..=b'7' => {
                let mut code = i32::from(first - b'0');
                while let Some(&digit) = bytes.get(cursor) {
                    if !matches!(digit, b'0'..=b'7') {
                        break;
                    }
                    code = code.wrapping_mul(8).wrapping_add(i32::from(digit - b'0'));
                    cursor += 1;
                }
                output.push(code as u8);
            }
            b'\0' => break,
            other => output.push(other),
        }
    }
    if let Some(nul) = output.iter().position(|byte| *byte == 0) {
        output.truncate(nul);
    }
    clonk_script::c4_string_from_bytes(&output)
}

fn is_material_compiler_key(name: &str) -> bool {
    matches!(
        name,
        "Name"
            | "Color"
            | "ColorX"
            | "Alpha"
            | "ColorAnimation"
            | "Shape"
            | "Density"
            | "Friction"
            | "DigFree"
            | "BlastFree"
            | "Blast2Object"
            | "Dig2Object"
            | "Dig2ObjectRatio"
            | "Dig2ObjectRequest"
            | "Blast2ObjectRatio"
            | "Blast2PXSRatio"
            | "Instable"
            | "MaxAirSpeed"
            | "MaxSlide"
            | "WindDrift"
            | "Inflammable"
            | "Incindiary"
            | "Corrode"
            | "Corrosive"
            | "Extinguisher"
            | "Soil"
            | "Placement"
            | "TextureOverlay"
            | "OverlayType"
            | "PXSGfx"
            | "PXSGfxRt"
            | "PXSGfxSize"
            | "TempConvStrength"
            | "BlastShiftTo"
            | "InMatConvert"
            | "InMatConvertTo"
            | "InMatConvertDepth"
            | "AboveTempConvert"
            | "AboveTempConvertDir"
            | "AboveTempConvertTo"
            | "BelowTempConvert"
            | "BelowTempConvertDir"
            | "BelowTempConvertTo"
            | "MinHeightCount"
            | "SplashRate"
    )
}

fn is_reaction_compiler_key(name: &str) -> bool {
    matches!(
        name,
        "Type"
            | "TargetSpec"
            | "ScriptFunc"
            | "ExecMask"
            | "Reverse"
            | "InverseSpec"
            | "CheckSlide"
            | "Depth"
            | "ConvertMat"
            | "CorrosionRate"
    )
}

fn material_definition_from_record(
    record: MaterialRecord,
    index: usize,
) -> Result<MaterialDefinition, MaterialError> {
    let MaterialRecord {
        name_hint,
        mut properties,
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
    let name = truncate_c4m_name(&name);
    if let Some(first) = properties
        .get_mut("name")
        .and_then(|values| values.first_mut())
    {
        *first = name.clone();
    }
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
            b'/' if bytes[idx + 1] == b'/'
                && (idx == 0 || bytes[idx - 1].is_ascii_whitespace()) =>
            {
                return Some(idx);
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
    parse_action_i32_prefix(&clonk_script::c4_string_bytes(value)).map(|(value, _)| value)
}

fn parse_u32_bits(value: &str) -> Option<i32> {
    parse_action_u64_prefix(&clonk_script::c4_string_bytes(value))
        .map(|(value, _)| value as u32 as i32)
}

fn parse_int_list(key: &str, value: &str) -> Vec<i32> {
    let normalized = normalize_key(key);
    let unsigned = matches!(normalized.as_str(), "color" | "colorx" | "alpha");
    let mut result = if unsigned {
        parse_u32_array(value)
    } else {
        parse_int_array(value).collect()
    };
    let fixed_len = match normalized.as_str() {
        "color" | "colorx" => Some(9),
        "alpha" | "pxsgfxrt" => Some(6),
        _ => None,
    };
    if let Some(fixed_len) = fixed_len {
        result.resize(fixed_len, 0);
        result.truncate(fixed_len);
    }
    result
}

fn parse_u32_array(value: &str) -> Vec<i32> {
    let bytes = clonk_script::c4_string_bytes(value);
    let mut values = Vec::new();
    let mut cursor = 0;
    loop {
        if !values.is_empty() {
            while bytes
                .get(cursor)
                .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
            {
                cursor += 1;
            }
            if bytes.get(cursor) != Some(&b',') {
                break;
            }
            cursor += 1;
        }
        if let Some((parsed, consumed)) = parse_action_u64_prefix(&bytes[cursor..]) {
            values.push(parsed as u32 as i32);
            cursor += consumed;
        } else {
            values.push(0);
        }
    }
    values
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Raw native bytes projected into the reversible Rust spelling, so the
    /// name-folding matrix can exercise latin1 material names.
    fn native(bytes: &[u8]) -> String {
        clonk_script::c4_string_from_bytes(bytes)
    }

    /// The owned-key spelling `c4_names_equal` used before it compared the
    /// normalized byte streams directly (`SEqualNoCase`, C4Material.cpp).
    fn owned_key_names_equal(left: &str, right: &str) -> bool {
        c4_name_key(left) == c4_name_key(right)
    }

    #[test]
    fn name_equality_matches_the_owned_key_projection() {
        let cases = [
            ("Earth".to_owned(), "Earth".to_owned()),
            ("Earth".to_owned(), "earth".to_owned()),
            ("Earth".to_owned(), "EARTH".to_owned()),
            ("Earth".to_owned(), "Earth ".to_owned()),
            ("Earth".to_owned(), " Earth".to_owned()),
            ("Vehicle".to_owned(), "Vehicl".to_owned()),
            ("Vehicl".to_owned(), "Vehicle".to_owned()),
            (String::new(), String::new()),
            ("\0".to_owned(), String::new()),
            ("Earth\0extra".to_owned(), "Earth".to_owned()),
            ("Earth\0left".to_owned(), "earth\0right".to_owned()),
            ("\0Earth".to_owned(), "\0Water".to_owned()),
            (native(&[0xc4]), native(&[0xe4])),
            (native(&[0xd6]), native(&[0xf6])),
            (native(&[0xdc]), native(&[0xfc])),
            (native(&[0xc4]), "ä".to_owned()),
            ("Ä".to_owned(), "ä".to_owned()),
            (native(&[0xff]), native(&[0xff, 0xff])),
            (native(&[b'E', 0xe4, b'r']), native(&[b'e', 0xc4, b'R'])),
            (native(&[b'E', 0xe4, b'r']), native(&[b'e', 0xf6, b'R'])),
        ];
        for (left, right) in &cases {
            assert_eq!(
                c4_names_equal(left, right),
                owned_key_names_equal(left, right),
                "{left:?} vs {right:?}"
            );
            assert_eq!(
                c4_names_equal(right, left),
                owned_key_names_equal(right, left),
                "{right:?} vs {left:?}"
            );
        }
    }

    #[test]
    fn compares_material_names_without_owning_native_bytes() {
        // The texmap lookups run this per object per frame, so the ASCII
        // material names must not round-trip through owned byte storage.
        assert!(matches!(
            clonk_script::c4_string_bytes_cow("Vehicle"),
            std::borrow::Cow::Borrowed(bytes) if bytes == b"Vehicle"
        ));
        assert!(c4_names_equal("Vehicle", "vehicle"));
        assert!(!c4_names_equal("Vehicle", "Tunnel"));
    }

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
    fn folder_material_group_assigns_indices_in_packed_sort_order() {
        // C4MaterialMap::Load takes slots from C4Group entry order
        // (C4Material.cpp:263-299), so a folder-backed Material.c4g must
        // assign the slots its packed image would. ASHES/Acid is the pair
        // shipped content/Material.c4g has where stricmp and raw bytes
        // disagree.
        let files = [
            (
                "ASHES.c4m",
                b"[Material]\nName=Ashes\nDensity=2\n".as_slice(),
            ),
            ("Acid.c4m", b"[Material]\nName=Acid\nDensity=1\n".as_slice()),
        ];

        let load = |created: &[(&str, &[u8])]| {
            let parent = tempfile::Builder::new()
                .prefix("lc-mat-")
                .tempdir()
                .expect("temp material parent");
            let root = parent.path().join("Material.c4g");
            std::fs::create_dir(&root).expect("create material group");
            for (name, bytes) in created {
                std::fs::write(root.join(name), bytes).expect("write material");
            }
            let group = crate::Group::open(&root).expect("open material folder");
            let library = MaterialLibrary::from_group(&group).expect("load material folder");
            library
                .iter()
                .map(|material| material.name().to_owned())
                .collect::<Vec<_>>()
        };

        let expected = vec!["Acid".to_owned(), "Ashes".to_owned()];
        assert_eq!(load(&files), expected);
        assert_eq!(
            load(&files.iter().copied().rev().collect::<Vec<_>>()),
            expected
        );
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
            .add_file("A.C4M", b"[Material]\nName=Dup\nDensity=21\n".to_vec())
            .expect("add first duplicate");
        packed
            .add_file("B.c4m", b"[Material]\nName=dUp\nDensity=22\n".to_vec())
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
            library
                .get("DUP")
                .and_then(|material| material.int("density")),
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
            merged
                .get("dup")
                .and_then(|material| material.int("density")),
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
    fn material_parser_matches_stdcompiler_section_key_and_value_grammar() {
        let mut packed = crate::MutableGroup::new("Grammar.c4g");
        packed
            .add_file(
                "Grammar.c4m",
                br#"[Reaction] before material
Type=Poof
TargetSpec="Solid\x21"
ScriptFunc="Raw\x80"
Reverse=2
InverseSpec=1x
CheckSlide= true
Depth=-7tail
ExecMask=-1tail

[material]
Name=Wrong
Density=999

[Material] trailing section text
Name= Native // kept
Density=0x32junk
Density=99
density=77
Density =88
Shape =7
MaxAirSpeed	=12junk
soil=8
  [Ignored]
Bad!
   Soil=9
  [Friction]33
Friction	=0x10zz
Placement=077tail
WindDrift=-0x10
MaxSlide=0b11
DigFree=true
BlastFree=2tail
Color=1,,3;4,5
  [ColorX]4,5
  [ColorX]6
Alpha=-1,2
PXSGfxRt=1,2
Blast2Object=ABCDE
Dig2Object=NONE
TextureOverlay=Smooth junk

[Reaction Foo]
Type=Ignored

[Reaction] trailing section text
Type=Convert // note
Reverse=truejunk
InverseSpec=False
CheckSlide=0x
"#
                .to_vec(),
            )
            .expect("add grammar material");
        let group = Group::from_raw_memory(
            std::path::PathBuf::from("Grammar.c4g"),
            packed.pack_raw().expect("pack grammar material group"),
        )
        .expect("open grammar material group");

        let library = MaterialLibrary::from_group(&group).expect("load grammar material group");
        let material = library
            .get("Native // kept")
            .expect("exact Material section and unstripped Name load");
        assert_eq!(library.iter().count(), 1);
        assert_eq!(material.int("Density"), Some(50));
        assert_eq!(material.int("Friction"), Some(33));
        assert_eq!(material.int("Shape"), None);
        assert_eq!(material.int("MaxAirSpeed"), Some(12));
        assert_eq!(material.int("Placement"), Some(77));
        assert_eq!(material.int("WindDrift"), Some(0));
        assert_eq!(material.int("MaxSlide"), Some(0));
        assert_eq!(material.int("Soil"), Some(9));
        assert_eq!(material.bool_flag("DigFree"), None);
        assert_eq!(material.bool_flag("BlastFree"), Some(true));
        assert_eq!(
            material.int_list("Color"),
            Some(vec![1, 0, 3, 0, 0, 0, 0, 0, 0])
        );
        assert_eq!(
            material.int_list("ColorX"),
            Some(vec![4, 6, 0, 0, 0, 0, 0, 0, 0])
        );
        assert_eq!(material.int_list("Alpha"), Some(vec![-1, 2, 0, 0, 0, 0]));
        assert_eq!(material.int_list("PXSGfxRt"), Some(vec![1, 2, 0, 0, 0, 0]));
        assert_eq!(material.value("Blast2Object"), Some("ABCD"));
        assert_eq!(material.value("Dig2Object"), Some(""));
        assert_eq!(material.value("TextureOverlay"), Some("Smooth"));

        let reactions = material.reactions();
        assert_eq!(reactions.len(), 2);
        assert_eq!(reactions[0].value("Type"), Some("Poof"));
        assert_eq!(reactions[0].value("TargetSpec"), Some("Solid!"));
        assert_eq!(
            clonk_script::c4_string_bytes(
                reactions[0]
                    .value("ScriptFunc")
                    .expect("escaped ScriptFunc compiles")
            ),
            b"Raw\x80"
        );
        assert_eq!(reactions[0].bool_flag("Reverse"), None);
        assert_eq!(reactions[0].bool_flag("InverseSpec"), Some(true));
        assert_eq!(reactions[0].bool_flag("CheckSlide"), None);
        assert_eq!(reactions[0].int("Depth"), Some(-7));
        assert_eq!(reactions[0].int("ExecMask"), Some(-1));
        assert_eq!(reactions[1].value("Type"), Some("Convert // note"));
        assert_eq!(reactions[1].bool_flag("Reverse"), Some(true));
        assert_eq!(reactions[1].bool_flag("InverseSpec"), None);
        assert_eq!(reactions[1].bool_flag("CheckSlide"), Some(false));

        let shadowed = MaterialParser::new(
            "Material=shadow\nReaction=,,\n[Material]\nName=Later\nDensity=9\n\
             [Reaction]\nType=Poof\n",
        )
        .parse_first()
        .expect("scalar namespaces compile");
        assert_eq!(shadowed.name(), "");
        assert_eq!(shadowed.int("Density"), None);
        assert_eq!(shadowed.reactions().len(), 1);
        assert!(shadowed.reactions()[0].raw_properties().is_empty());

        let absent = MaterialParser::new("[material]\nName=Wrong\n[Material ]\nName=AlsoWrong\n")
            .parse_first()
            .expect("absent exact Material namespace compiles defaults");
        assert_eq!(absent.name(), "");
        assert!(absent.raw_properties().is_empty());

        let numeric_whitespace = MaterialParser::new(
            "[Material]\nName=Whitespace\nDensity=\u{000b}50tail\n\
             Friction=\u{000c}7tail\nMaxSlide=0x 1\nPXSGfxRt=1,\u{000b}2\n",
        )
        .parse_first()
        .expect("native numeric whitespace compiles");
        assert_eq!(numeric_whitespace.int("Density"), Some(50));
        assert_eq!(numeric_whitespace.int("Friction"), Some(7));
        assert_eq!(numeric_whitespace.int("MaxSlide"), Some(0));
        assert_eq!(
            numeric_whitespace.int_list("PXSGfxRt"),
            Some(vec![1, 2, 0, 0, 0, 0])
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
        let enumeration =
            MaterialEnumeration::parse(b"[Enumeration]\r\nC\r\n").expect("enumeration parses");

        library
            .sort_enumeration(&enumeration)
            .expect("enumeration sorts");

        assert_eq!(
            library
                .iter()
                .map(MaterialDefinition::name)
                .collect::<Vec<_>>(),
            vec!["C", "B", "A"]
        );
        let wrong_case =
            MaterialEnumeration::parse(b"[Enumeration] c").expect("lowercase enumeration parses");
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
