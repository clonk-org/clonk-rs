//! `scenario` — moved verbatim from the parent module.
//!
//! Structural only: same crate, same items, same bodies.

use super::*;

#[derive(Debug, Clone, PartialEq)]
pub(in crate::scenario) enum SerializedC4Value {
    Value(clonk_script::Value),
    /// Untyped legacy C4V_Any word. Values in the old enumerated-pointer
    /// range are denumerated when the referenced object exists; otherwise
    /// C4Value::GuessType leaves these serialized words as integers.
    Any(i32),
    ObjectNumber(i32),
    StringTableIndex(i32),
    Array(Vec<SerializedC4Value>),
    Map {
        entries: Vec<(SerializedC4Value, SerializedC4Value)>,
        // Compile-time removals can only leave cleared (nil) mapped slots.
        // C4ValueHash::DenumeratePointers does not traverse emptyValues.
        empty_value_count: usize,
    },
}

pub(in crate::scenario) struct SerializedC4ValueResolution<'a> {
    pub(in crate::scenario) object_numbers: &'a HashSet<u64>,
    pub(in crate::scenario) string_registrations: &'a clonk_script::StringRegistrations,
}

impl SerializedC4Value {
    /// Mirror C4Value::DenumeratePointer and serialized-string lookup
    /// (C4Value.cpp:686-713,783-798). Serialized identities become live VM
    /// values only after the accepted object-number and string tables exist.
    pub(in crate::scenario) fn resolve(
        self,
        resolution: &SerializedC4ValueResolution<'_>,
    ) -> clonk_script::Value {
        self.resolve_strings(resolution.string_registrations)
            .denumerate_objects(resolution.object_numbers)
    }

    /// C4Value::CompileFunc resolves every serialized string while compiling
    /// the complete container. Pointer denumeration is a later pass, so a
    /// missing object entry cannot prevent a sibling from claiming the same
    /// loaded C4String identity.
    fn resolve_strings(
        self,
        string_registrations: &clonk_script::StringRegistrations,
    ) -> SerializedC4Value {
        match self {
            Self::StringTableIndex(index) => Self::Value(
                clonk_script::resolve_c4_string(string_registrations, index)
                    .map(clonk_script::Value::String)
                    .unwrap_or(clonk_script::Value::Nil),
            ),
            Self::Array(values) => Self::Array(
                values
                    .into_iter()
                    .map(|value| value.resolve_strings(string_registrations))
                    .collect(),
            ),
            Self::Map {
                entries,
                empty_value_count,
            } => {
                let mut compiled_entries =
                    Vec::<(SerializedC4Value, SerializedC4Value)>::with_capacity(entries.len());
                let mut compiled_empty_value_count = empty_value_count;
                for (key, value) in entries {
                    let key = key.resolve_strings(string_registrations);
                    let value = value.resolve_strings(string_registrations);
                    if let Some(index) = compiled_entries
                        .iter()
                        .position(|(existing, _)| existing == &key)
                    {
                        if value.is_compiled_nil() && !compiled_entries[index].1.is_compiled_nil() {
                            compiled_entries.remove(index);
                            compiled_empty_value_count += 1;
                        } else {
                            compiled_entries[index].1 = value;
                        }
                    } else {
                        // CompileFunc's `map[key] = value` consumes a recycled
                        // mapped slot only for a genuinely new key. Compile-
                        // time removals leave nil slots, so assigning nil to
                        // one takes C4Value::Set's unchanged-value return.
                        compiled_empty_value_count = compiled_empty_value_count.saturating_sub(1);
                        compiled_entries.push((key, value));
                    }
                }
                Self::Map {
                    entries: compiled_entries,
                    empty_value_count: compiled_empty_value_count,
                }
            }
            value => value,
        }
    }

    fn denumerate_objects(self, object_numbers: &HashSet<u64>) -> clonk_script::Value {
        use clonk_script::Value;
        match self {
            Self::Value(value) => value,
            Self::Any(number) => {
                if (1_000_000_000..=1_001_000_000).contains(&number) {
                    let object_number = number - 1_000_000_000;
                    if let Ok(object_number) = u64::try_from(object_number) {
                        if object_numbers.contains(&object_number) {
                            return Value::Object(object_number);
                        }
                    }
                }
                serialized_any_fallback(number)
            }
            Self::ObjectNumber(number) => {
                // Old pointer-enumeration saves add C4EnumPointer1. For an
                // explicitly C4V_C4ObjectEnum value C++ subtracts it from any
                // value at or above the lower bound, then searches active and
                // inactive object lists (C4Value.cpp:693-703).
                let number = if number >= 1_000_000_000 {
                    number - 1_000_000_000
                } else {
                    number
                };
                u64::try_from(number)
                    .ok()
                    .filter(|number| object_numbers.contains(number))
                    .map(Value::Object)
                    .unwrap_or(Value::Nil)
            }
            Self::Array(values) => Value::Array(
                values
                    .into_iter()
                    .map(|value| value.denumerate_objects(object_numbers))
                    .collect(),
            ),
            Self::Map {
                entries,
                empty_value_count,
            } => {
                // Denumerate every key and value before mutating the visible
                // hash. C4ValueHash::DenumeratePointers iterates already-
                // compiled C4Values; a key that clears retains its mapped
                // slot in emptyValues, while a value that clears contributes
                // the now-nil slot.
                let entries = entries
                    .into_iter()
                    .map(|(key, value)| {
                        let missing_key = key.is_missing_direct_object(object_numbers);
                        let missing_value = value.is_missing_direct_object(object_numbers);
                        (
                            missing_key || missing_value,
                            key.denumerate_objects(object_numbers),
                            value.denumerate_objects(object_numbers),
                        )
                    })
                    .collect::<Vec<_>>();
                let mut values = clonk_script::ValueMap::with_capacity(entries.len());
                let mut removed_values = Vec::new();
                for (removed, key, value) in entries {
                    if removed {
                        removed_values.push(value);
                        continue;
                    }
                    values.insert_key(key, value);
                }
                // Every surviving slot was allocated during CompileFunc,
                // before DenumeratePointers can populate emptyValues. Queue
                // removed slots only now so ordinary loaded entries cannot
                // accidentally reuse them. push_front/pop_front makes the
                // last removed slot the first one reused, matching Vec::pop.
                for _ in 0..empty_value_count {
                    values.recycle_value_slot(Value::Nil);
                }
                for value in removed_values {
                    values.recycle_value_slot(value);
                }
                Value::Proplist(values)
            }
            Self::StringTableIndex(_) => {
                unreachable!("serialized strings resolve before object denumeration")
            }
        }
    }

    fn is_missing_direct_object(&self, object_numbers: &HashSet<u64>) -> bool {
        let Self::ObjectNumber(number) = self else {
            return false;
        };
        let number = if *number >= 1_000_000_000 {
            *number - 1_000_000_000
        } else {
            *number
        };
        u64::try_from(number)
            .ok()
            .is_none_or(|number| !object_numbers.contains(&number))
    }

    fn is_compiled_nil(&self) -> bool {
        match self {
            Self::Value(clonk_script::Value::Nil) | Self::Any(0) | Self::ObjectNumber(0) => true,
            Self::Value(clonk_script::Value::C4Id(value)) => clonk_script::c4_id_raw(value) == 0,
            _ => false,
        }
    }
}

fn serialized_any_fallback(number: i32) -> clonk_script::Value {
    if number == 0 {
        return clonk_script::Value::Nil;
    }
    // GuessType checks packed literal IDs before falling back to int. The
    // numeric 1..9999 spelling is deliberately excluded by its >=10000 gate
    // (C4Value.cpp:299-330; C4Id.cpp:55-67).
    let raw = number as u32;
    if raw >= 10_000
        && raw
            .to_le_bytes()
            .iter()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || *byte == b'_')
    {
        clonk_script::Value::C4Id(clonk_script::c4_id_from_raw(raw as usize))
    } else {
        clonk_script::Value::Int(number)
    }
}

/// Objects.txt `Locals=` is C4ValueList::CompileFunc
/// (C4ValueList.cpp:102-136). Current saves write
/// `<size>;<typed-value>,...`; trailing default values may be omitted. The
/// pre-size legacy form stores its first raw integer in slot zero and always
/// restores the ten C4MaxVariable slots.
pub(in crate::scenario) fn parse_local_slots(
    value: &str,
    line: usize,
) -> Result<Vec<SerializedC4Value>, ScenarioError> {
    const C4_MAX_VARIABLE: usize = 10;
    const C4_VALUE_LIST_MAX_SIZE: usize = 1_000_000;

    let parse_error = |detail: String| {
        ScenarioError::LegacyObjectsParse(format!("Objects.txt line {}: {}", line, detail))
    };
    let trimmed = value.trim();
    let (size, values): (usize, Vec<SerializedC4Value>) = if let Some((size_text, values_text)) =
        trimmed.split_once(';')
    {
        let size = parse_i32(size_text.trim())
            .map_err(|error| parse_error(format!("invalid Locals size `{size_text}` ({error})")))?
            .try_into()
            .map_err(|_| parse_error(format!("invalid negative Locals size `{size_text}`")))?;
        if size > C4_VALUE_LIST_MAX_SIZE {
            return Err(parse_error(format!(
                "Locals size {size} exceeds C4ValueList::MaxSize"
            )));
        }
        let values = split_outside_brackets(values_text)
            .into_iter()
            .take(size)
            .map(str::trim)
            .map(|encoded| parse_serialized_c4value(encoded, line))
            .collect::<Result<Vec<_>, _>>()?;
        (size, values)
    } else {
        let mut encoded = split_outside_brackets(trimmed).into_iter();
        let first = encoded.next().unwrap_or_default().trim();
        let first = parse_i32(first).map_err(|error| {
            parse_error(format!("invalid legacy Locals value `{first}` ({error})"))
        })?;
        let mut values = vec![SerializedC4Value::Any(first)];
        values.extend(
            encoded
                .take(C4_MAX_VARIABLE - 1)
                .map(str::trim)
                .map(|entry| parse_serialized_c4value(entry, line))
                .collect::<Result<Vec<_>, _>>()?,
        );
        (C4_MAX_VARIABLE, values)
    };

    let mut values = values;
    values.truncate(size);
    values.resize_with(size, || SerializedC4Value::Value(clonk_script::Value::Nil));
    Ok(values)
}

pub(in crate::scenario) fn parse_local_named(
    value: &str,
    line: usize,
) -> Result<Vec<(String, SerializedC4Value)>, ScenarioError> {
    let trimmed = value.trim();
    let (count_text, rest) = trimmed
        .split_once(';')
        .map_or((trimmed, None), |(count, rest)| (count, Some(rest)));
    let count = parse_std_i32(count_text).unwrap_or(0);
    if count == 0 {
        // C4ValueMapData returns immediately for a zero/defaulted count and
        // never consumes a trailing payload.
        return Ok(Vec::new());
    }
    let count = usize::try_from(count).map_err(|_| {
        ScenarioError::LegacyObjectsParse(format!(
            "Objects.txt line {}: invalid negative LocalNamed count `{}`",
            line, count_text
        ))
    })?;
    let rest = rest.ok_or_else(|| {
        ScenarioError::LegacyObjectsParse(format!(
            "Objects.txt line {}: LocalNamed count {} is missing `;`",
            line, count
        ))
    })?;
    let mut parts = split_outside_brackets(rest).into_iter();
    let mut entries = Vec::new();
    for index in 0..count {
        let part = parts.next().ok_or_else(|| {
            ScenarioError::LegacyObjectsParse(format!(
                "Objects.txt line {}: LocalNamed declares {} entries but contains {}",
                line, count, index
            ))
        })?;
        let part = part.trim();
        if part.is_empty() {
            return Err(ScenarioError::LegacyObjectsParse(format!(
                "Objects.txt line {}: LocalNamed entry {} is empty",
                line, index
            )));
        }
        let Some((name, encoded)) = part.split_once('=') else {
            return Err(ScenarioError::LegacyObjectsParse(format!(
                "Objects.txt line {}: LocalNamed entry `{}` missing `=`",
                line, part
            )));
        };
        entries.push((
            name.trim().to_string(),
            parse_serialized_c4value(encoded.trim(), line)?,
        ));
    }
    // StdCompiler reads exactly iValueCnt entries. Any remaining bytes in
    // the named value are ignored, so trailing entries must not leak into
    // the live name map.
    Ok(entries)
}

/// Split on commas outside `[...]` (array payloads carry their own commas).
fn split_outside_brackets(text: &str) -> Vec<&str> {
    split_outside_delimiter(text, ',')
}

pub(in crate::scenario) fn split_outside_delimiter(text: &str, delimiter: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut square_depth = 0usize;
    let mut round_depth = 0usize;
    let mut start = 0;
    for (index, ch) in text.char_indices() {
        match ch {
            '[' => square_depth += 1,
            ']' => square_depth = square_depth.saturating_sub(1),
            '(' => round_depth += 1,
            ')' => round_depth = round_depth.saturating_sub(1),
            ch if ch == delimiter && square_depth == 0 && round_depth == 0 => {
                parts.push(&text[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    parts.push(&text[start..]);
    parts
}

/// `splitn`, but separators nested in the C4Value/transform bracket pairs do
/// not count. The unsplit tail is needed for C4Command::Text (RCT_All), which
/// may itself contain commas.
pub(in crate::scenario) fn split_outside_delimiter_limit(
    text: &str,
    delimiter: char,
    limit: usize,
) -> Vec<&str> {
    if limit <= 1 {
        return vec![text];
    }
    let mut parts = Vec::with_capacity(limit);
    let mut square_depth = 0usize;
    let mut round_depth = 0usize;
    let mut start = 0usize;
    for (index, ch) in text.char_indices() {
        match ch {
            '[' => square_depth += 1,
            ']' => square_depth = square_depth.saturating_sub(1),
            '(' => round_depth += 1,
            ')' => round_depth = round_depth.saturating_sub(1),
            ch if ch == delimiter
                && square_depth == 0
                && round_depth == 0
                && parts.len() + 1 < limit =>
            {
                parts.push(&text[start..index]);
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(&text[start..]);
    parts
}

/// One serialized C4Value (C4Value::CompileFunc, C4Value.cpp:717-800 +
/// GetC4VID :368-394): `A`=any (zero reads nil; old pointer-range words
/// denumerate before remaining nonzero values guess int), `i`=int, `b`=bool,
/// `o`/`O`=enumerated object number (0 = no object),
/// `I`=C4ID stored as its signed 32-bit payload, `a[size;elems]`=array with
/// trailing nils omitted on write, and `S` indexes the scenario Strings.txt.
/// `m[count;key=value;...]` retains arbitrary typed keys in insertion order.
pub(in crate::scenario) fn parse_serialized_c4value(
    encoded: &str,
    line: usize,
) -> Result<SerializedC4Value, ScenarioError> {
    use clonk_script::Value;
    let parse_error = |detail: String| {
        ScenarioError::LegacyObjectsParse(format!("Objects.txt line {}: {}", line, detail))
    };
    let mut chars = encoded.chars();
    let Some(type_char) = chars.next() else {
        return Ok(SerializedC4Value::Value(Value::Nil));
    };
    let payload = &encoded[type_char.len_utf8()..];
    let int_payload = || {
        parse_i32(payload.trim())
            .map_err(|err| parse_error(format!("invalid C4Value payload `{}` ({})", encoded, err)))
    };
    match type_char {
        'A' => Ok(SerializedC4Value::Any(int_payload()?)),
        'i' => Ok(SerializedC4Value::Value(Value::Int(int_payload()?))),
        'b' => Ok(SerializedC4Value::Value(Value::from_c4_bool_raw(
            int_payload()?,
        ))),
        'o' | 'O' => Ok(SerializedC4Value::ObjectNumber(int_payload()?)),
        'I' => Ok(SerializedC4Value::Value(Value::C4Id(
            clonk_script::c4_id_from_raw(int_payload()? as isize as usize),
        ))),
        'a' => {
            let inner = payload
                .strip_prefix('[')
                .and_then(|rest| rest.strip_suffix(']'))
                .ok_or_else(|| {
                    parse_error(format!(
                        "invalid C4Value array `{}` (expected a[...])",
                        encoded
                    ))
                })?;
            let (size_text, elements_text) = inner.split_once(';').unwrap_or((inner, ""));
            let size = parse_i32(size_text.trim()).map_err(|err| {
                parse_error(format!("invalid array size in `{}` ({})", encoded, err))
            })?;
            if !(0..=1_000_000).contains(&size) {
                return Err(parse_error(format!(
                    "array size {} in `{}` exceeds C4ValueList::MaxSize",
                    size, encoded
                )));
            }
            let size = size as usize;
            let mut elements: Vec<SerializedC4Value> = split_outside_brackets(elements_text)
                .into_iter()
                .take(size)
                .map(str::trim)
                .map(|element| parse_serialized_c4value(element, line))
                .collect::<Result<_, _>>()?;
            // Trailing nils are omitted on write; restore the full size.
            if elements.len() < size {
                elements.resize_with(size, || SerializedC4Value::Value(Value::Nil));
            }
            Ok(SerializedC4Value::Array(elements))
        }
        'S' => Ok(SerializedC4Value::StringTableIndex(int_payload()?)),
        'm' => {
            let inner = payload
                .strip_prefix('[')
                .and_then(|rest| rest.strip_suffix(']'))
                .ok_or_else(|| {
                    parse_error(format!(
                        "invalid C4Value map `{}` (expected m[...])",
                        encoded
                    ))
                })?;
            let (count_text, entries_text) = inner.split_once(';').unwrap_or((inner, ""));
            let count = parse_i32(count_text.trim()).map_err(|err| {
                parse_error(format!("invalid map size in `{}` ({})", encoded, err))
            })?;
            if count < 0 {
                return Err(parse_error(format!("negative map size in `{}`", encoded)));
            }
            let count = count as usize;
            let mut serialized_entries = split_outside_delimiter(entries_text, ';').into_iter();
            let mut entries = Vec::new();
            for index in 0..count {
                let entry = serialized_entries.next().ok_or_else(|| {
                    parse_error(format!(
                        "map `{}` declares {} entries but contains {}",
                        encoded, count, index
                    ))
                })?;
                let entry = entry.trim();
                let equals = entry
                    .char_indices()
                    .scan((0usize, 0usize), |depth, (index, ch)| {
                        match ch {
                            '[' => depth.0 += 1,
                            ']' => depth.0 = depth.0.saturating_sub(1),
                            '(' => depth.1 += 1,
                            ')' => depth.1 = depth.1.saturating_sub(1),
                            '=' if depth.0 == 0 && depth.1 == 0 => return Some(Some(index)),
                            _ => {}
                        }
                        Some(None)
                    })
                    .flatten()
                    .next()
                    .ok_or_else(|| parse_error(format!("map entry `{entry}` missing `=`")))?;
                let key = parse_serialized_c4value(entry[..equals].trim(), line)?;
                let value = parse_serialized_c4value(entry[equals + 1..].trim(), line)?;
                entries.push((key, value));
            }
            Ok(SerializedC4Value::Map {
                entries,
                empty_value_count: 0,
            })
        }
        // Character only consumes an alphabetic byte. A raw number therefore
        // falls back to C4V_Any without consuming its first digit; an unknown
        // alphabetic type byte is consumed and GetC4VFromID also returns Any.
        // C4V_pC4Value is the one nonserializable exception.
        'V' => Err(parse_error(format!(
            "nonserializable C4Value reference in `{}`",
            encoded
        ))),
        other if other.is_ascii_alphabetic() => Ok(SerializedC4Value::Any(int_payload()?)),
        _ => Ok(SerializedC4Value::Any(parse_i32(encoded.trim()).map_err(
            |err| parse_error(format!("invalid C4Value payload `{}` ({})", encoded, err)),
        )?)),
    }
}

/// Comma-separated int array (StdCompiler mkArrayAdapt serialization,
/// e.g. `VertexX=2,-14,14`).
pub(in crate::scenario) fn parse_i32_list(
    value: &str,
    line: usize,
    key: &str,
) -> Result<Vec<i32>, ScenarioError> {
    value
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            parse_i32(entry).map_err(|err| {
                ScenarioError::LegacyObjectsParse(format!(
                    "Objects.txt line {}: invalid {} entry `{}` ({})",
                    line, key, entry, err
                ))
            })
        })
        .collect()
}

/// C4IDList::CompileFunc with values (C4IDList.cpp:240-259): semicolon-
/// separated four-character IDs, each optionally followed by `=count`.
pub(in crate::scenario) fn parse_legacy_object_components(
    value: &str,
    line: usize,
) -> Result<Vec<(DefinitionId, i32)>, ScenarioError> {
    value
        .split(';')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            let (id, count) = entry.split_once('=').unwrap_or((entry, "0"));
            let id = id.trim();
            let valid_id = id.len() == 4
                && id != "NONE"
                && id
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_');
            if !valid_id {
                return Err(ScenarioError::LegacyObjectsParse(format!(
                    "Objects.txt line {}: invalid Component id `{}`",
                    line, id
                )));
            }
            let count = parse_i32(count.trim()).map_err(|err| {
                ScenarioError::LegacyObjectsParse(format!(
                    "Objects.txt line {}: invalid Component count `{}` ({})",
                    line,
                    count.trim(),
                    err
                ))
            })?;
            Ok((DefinitionId::from(id), count))
        })
        .collect()
}

/// A serialized C4Fixed (Fixed.h:247-266): lowercase `f` means the int32
/// contains float bits converted through `FLOAT_TO_FIXED`; any other
/// alphabetic format byte leaves the following int32 raw. A missing format
/// byte is the old raw representation. GoldRush's hanging stalactites carry
/// `YDir=f1067030938` = 1.2 px/frame.
pub(in crate::scenario) fn parse_c4fixed(value: &str) -> Result<crate::math::C4Fixed, String> {
    let trimmed = value.trim();
    // StdCompilerINIRead::Character consumes any alphabetic format byte.
    // Only lowercase `f` requests the legacy float-bit conversion; every
    // other letter (including `F`) leaves the following int32 word raw.
    let (format, rest) = match trimmed.as_bytes().first().copied() {
        Some(format) if format.is_ascii_alphabetic() => (Some(format), &trimmed[1..]),
        _ => (None, trimmed),
    };
    let raw = parse_std_i32(rest).ok_or_else(|| "invalid int32 value".to_string())?;
    if format == Some(b'f') {
        Ok(crate::math::ftofix(f32::from_bits(raw as u32)))
    } else {
        Ok(crate::math::C4Fixed::from_raw(raw))
    }
}

pub(crate) fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "y" | "on" => Some(true),
        "false" | "0" | "no" | "n" | "off" => Some(false),
        _ => None,
    }
}

pub(in crate::scenario) fn owner_index_from_section(section: &str) -> Option<i32> {
    let suffix = section.trim_start_matches("player");
    if suffix.is_empty() {
        return Some(0);
    }
    let index = suffix.parse::<i32>().ok()?;
    let owner = index - 1;
    if owner < 0 {
        None
    } else {
        Some(owner)
    }
}

pub(in crate::scenario) fn is_missing_group_error(error: &GroupError) -> bool {
    matches!(
        error,
        GroupError::Missing(_) | GroupError::NotDirectory(_) | GroupError::EntryNotFound(_)
    ) || matches!(
        error,
        GroupError::Io(io_error) if io_error.kind() == io::ErrorKind::NotFound
    )
}

pub(in crate::scenario) fn resolve_one_definition_group(
    scenario: &Group,
    resolver: &dyn LegacyDefinitionResolver,
    spec: &str,
) -> Result<Group, ScenarioError> {
    let normalized = spec.replace('\\', "/");
    if normalized.is_empty() {
        return Err(ScenarioError::LegacyDefinitionNotFound {
            path: spec.to_string(),
        });
    }

    let normalized_path = legacy_definition_path(&normalized);
    if normalized_path.is_absolute() {
        return match open_group_path(&normalized_path) {
            Ok(group) => Ok(group),
            Err(error) if is_missing_group_error(&error) => {
                Err(ScenarioError::LegacyDefinitionNotFound { path: normalized })
            }
            Err(error) => Err(ScenarioError::Resources(error)),
        };
    }

    // C4GameResList opens one filename once. The resolver owns search-path
    // priority (global/external roots before scenario fallback); retain only
    // its first result so intentional external `.c4f/...` paths are valid.
    resolver
        .resolve_definition_groups(scenario, &normalized)?
        .into_iter()
        .next()
        .ok_or_else(|| ScenarioError::LegacyDefinitionNotFound { path: normalized })
}

fn legacy_definition_path(value: &str) -> PathBuf {
    clonk_resources::path_from_legacy_bytes(&clonk_script::c4_string_bytes(value))
}

fn open_group_relative_case_insensitive(
    mut group: Group,
    relative: &Path,
) -> Result<Group, GroupError> {
    for component in relative.components() {
        let Component::Normal(name) = component else {
            if component == Component::CurDir {
                continue;
            }
            return Err(GroupError::EntryNotFound(relative.to_path_buf()));
        };
        let name_bytes = legacy_group_path_component_bytes(name);
        let entry = group
            .entries()?
            .into_iter()
            .find(|entry| entry.name_bytes.eq_ignore_ascii_case(&name_bytes))
            .ok_or_else(|| GroupError::EntryNotFound(relative.to_path_buf()))?;
        group = group.open_child_entry_exact(&entry)?;
    }
    Ok(group)
}

fn legacy_group_path_component_bytes(name: &OsStr) -> Vec<u8> {
    clonk_resources::path_to_legacy_bytes(Path::new(name))
}

/// Opens physical groups and virtual paths nested inside packed groups. A
/// packed child group's `root()` is a stable full-name label rather than a
/// host-filesystem path; walking from the deepest physical prefix makes that
/// retained label usable as a fixed definition resource on restart.
pub(in crate::scenario) fn open_group_path(path: &Path) -> Result<Group, GroupError> {
    let direct_error = match Group::open(path) {
        Ok(group) => return Ok(group),
        Err(error) if is_missing_group_error(&error) => error,
        Err(error) => return Err(error),
    };

    for physical_prefix in path.ancestors().skip(1) {
        if physical_prefix.as_os_str().is_empty() || !physical_prefix.exists() {
            continue;
        }
        let group = Group::open(physical_prefix)?;
        let relative = path
            .strip_prefix(physical_prefix)
            .map_err(|_| GroupError::EntryNotFound(path.to_path_buf()))?;
        return open_group_relative_case_insensitive(group, relative);
    }
    Err(direct_error)
}

/// Opens `spec` strictly below `root`, one immediate group component at a
/// time. C4Group entry lookup is ASCII-case-insensitive even while crossing
/// packed child groups; host `Path::join` alone cannot model either property.
pub(in crate::scenario) fn resolve_rooted_definition_group(
    root: &Path,
    spec: &str,
) -> Result<Group, ScenarioError> {
    let normalized = spec.replace('\\', "/");
    let normalized_path = legacy_definition_path(&normalized);
    let candidate = root.join(&normalized_path);
    let not_found = || ScenarioError::LegacyDefinitionNotFound {
        path: candidate.display().to_string(),
    };
    let group = match open_group_path(root) {
        Ok(group) => group,
        Err(error) if is_missing_group_error(&error) => return Err(not_found()),
        Err(error) => return Err(ScenarioError::Resources(error)),
    };

    match open_group_relative_case_insensitive(group, &normalized_path) {
        Ok(group) => Ok(group),
        Err(error) if is_missing_group_error(&error) => Err(not_found()),
        Err(error) => Err(ScenarioError::Resources(error)),
    }
}

/// Applies C4Game's exact `DefinitionPath + module` operation. Unlike
/// `Path::join`, this neither inserts a separator nor lets an absolute module
/// replace the configured prefix.
pub(in crate::scenario) fn resolve_prefixed_definition_group(
    prefix: &Path,
    spelling: &str,
) -> Result<Group, ScenarioError> {
    let mut candidate = legacy_path_bytes(prefix);
    candidate.extend(clonk_script::c4_string_bytes(spelling));
    for byte in &mut candidate {
        if *byte == b'\\' {
            *byte = std::path::MAIN_SEPARATOR as u8;
        }
    }
    let candidate = legacy_path_from_bytes(candidate);
    match open_group_path(&candidate) {
        Ok(group) => Ok(group),
        Err(error) if is_missing_group_error(&error) => {
            Err(ScenarioError::LegacyDefinitionNotFound {
                path: candidate.display().to_string(),
            })
        }
        Err(error) => Err(ScenarioError::Resources(error)),
    }
}

fn legacy_path_bytes(path: &Path) -> Vec<u8> {
    clonk_resources::path_to_legacy_bytes(path)
}

fn legacy_path_from_bytes(bytes: Vec<u8>) -> PathBuf {
    clonk_resources::path_from_legacy_bytes(&bytes)
}

pub(in crate::scenario) fn folder_local_definition_groups(
    scenario: &Group,
) -> Result<Vec<Group>, ScenarioError> {
    let mut folder_paths = scenario
        .root()
        .ancestors()
        .skip(1)
        .filter(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("c4f"))
        })
        .map(Path::to_path_buf)
        .collect::<Vec<_>>();
    folder_paths.reverse();

    let mut groups = Vec::new();
    for path in folder_paths {
        let group = match open_group_path(&path) {
            Ok(group) => group,
            // C4Game::FoldersWithLocalsDefs skips path prefixes it cannot
            // open rather than turning them into definition resources.
            Err(error) if is_missing_group_error(&error) => continue,
            Err(error) => return Err(ScenarioError::Resources(error)),
        };
        let has_immediate_definition = group
            .entries()?
            .into_iter()
            .any(|entry| legacy_group_wildcard_match(b"*.c4d", &entry.name_bytes));
        if has_immediate_definition {
            groups.push(group);
        }
    }
    Ok(groups)
}

#[cfg(test)]
mod tests {
    use super::{parse_serialized_c4value, SerializedC4Value};
    use clonk_script::Value;

    fn parse(encoded: &str) -> SerializedC4Value {
        parse_serialized_c4value(encoded, 1).expect("the encoding parses")
    }

    /// The savegame `C4Value` tag table, which had no test despite deciding how
    /// every stored local, array and map comes back (clonk-org/clonk-rs#523).
    ///
    /// The pair worth pinning is the asymmetry the module comment describes:
    /// C++'s reader consumes a type *character* only when it is alphabetic, so
    /// an unknown letter is swallowed and its payload parsed, while a bare
    /// number keeps **all** of its digits and falls back to `C4V_Any`. An
    /// implementation that always consumed the first byte would silently turn
    /// `42` into `2`, which no round-trip of well-formed data would ever
    /// reveal — every value it writes carries a tag.
    #[test]
    fn serialized_c4value_tags_follow_the_cpp_consume_rules() {
        // The ordinary tags.
        assert!(matches!(
            parse("i42"),
            SerializedC4Value::Value(Value::Int(42))
        ));
        assert!(matches!(parse("o7"), SerializedC4Value::ObjectNumber(7)));
        assert!(matches!(parse("O7"), SerializedC4Value::ObjectNumber(7)));
        assert!(matches!(
            parse("S3"),
            SerializedC4Value::StringTableIndex(3)
        ));

        // An empty encoding is nil rather than an error.
        assert!(matches!(parse(""), SerializedC4Value::Value(Value::Nil)));

        // An unknown *alphabetic* tag is consumed, so the payload is what
        // follows it.
        assert!(matches!(parse("z42"), SerializedC4Value::Any(42)));

        // A bare number has no tag to consume, so the whole text is the
        // payload — the digit must not be eaten.
        assert!(matches!(parse("42"), SerializedC4Value::Any(42)));
        assert!(matches!(parse("-7"), SerializedC4Value::Any(-7)));

        // `C4V_pC4Value` is the one type that cannot be serialized.
        assert!(parse_serialized_c4value("V1", 1).is_err());
    }

    /// Arrays restore the trailing nils that the writer omits, and refuse a
    /// declared size past `C4ValueList::MaxSize`.
    ///
    /// The restore matters because the stored element count and the declared
    /// size legitimately disagree: a writer that drops trailing nils produces
    /// `a[4;i1,i2]`, and reading that as a two-element array would shift every
    /// later index.
    #[test]
    fn serialized_arrays_restore_omitted_trailing_nils_and_bound_their_size() {
        let SerializedC4Value::Array(elements) = parse("a[4;i1,i2]") else {
            panic!("expected an array");
        };
        assert_eq!(elements.len(), 4, "the declared size wins over the count");
        assert!(matches!(elements[3], SerializedC4Value::Value(Value::Nil)));

        // More elements than declared are truncated rather than growing it.
        let SerializedC4Value::Array(elements) = parse("a[1;i1,i2,i3]") else {
            panic!("expected an array");
        };
        assert_eq!(elements.len(), 1);

        // An empty array keeps its size.
        let SerializedC4Value::Array(elements) = parse("a[0;]") else {
            panic!("expected an array");
        };
        assert!(elements.is_empty());

        let error = parse_serialized_c4value("a[1000001;]", 1)
            .expect_err("a size past C4ValueList::MaxSize is refused");
        assert!(
            error.to_string().contains("MaxSize"),
            "the refusal should name the native bound: {error}"
        );
        assert!(parse_serialized_c4value("a[-1;]", 1).is_err());
    }
}
