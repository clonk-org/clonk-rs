use std::path::{Path, PathBuf};

use clonk_resources::{Group, GroupError};

/// Opens a logical C4Group path, including children nested in packed parents.
///
/// Packed children have no filesystem metadata of their own. Reopen the
/// nearest physical ancestor and traverse the remaining components through
/// C4Group's case-insensitive, `?`-aware child lookup instead.
pub(crate) fn open_group_path(path: &Path) -> Result<Group, GroupError> {
    if path.exists() {
        return Group::open(path);
    }

    let mut ancestor = path.to_path_buf();
    let mut children = Vec::new();
    while !ancestor.exists() {
        let Some(name) = ancestor.file_name().map(|name| name.to_os_string()) else {
            return Err(GroupError::Missing(path.to_path_buf()));
        };
        children.push(PathBuf::from(name));
        if !ancestor.pop() {
            return Err(GroupError::Missing(path.to_path_buf()));
        }
    }

    let mut group = Group::open(&ancestor)?;
    for child in children.iter().rev() {
        group = group.open_child(child)?;
    }
    Ok(group)
}

/// Reproduces the filename retained by C4Group after opening `lookup_name`.
/// The caller's lexical prefix stays intact, while child-group wildcard/case
/// matches and Win32's final-component correction use the physical spelling.
pub fn opened_group_name(path: &Path, lookup_name: &[u8], executable_root: &Path) -> Vec<u8> {
    let separator = std::path::MAIN_SEPARATOR as u8;
    let mut opened_name = lookup_name
        .iter()
        .map(|byte| if *byte == b'\\' { separator } else { *byte })
        .collect::<Vec<_>>();

    // A physical group retains the spelling passed to C4Group::Open (apart
    // from its native backslash conversion). If that spelling does not name a
    // physical item, Open walks back to the deepest real mother group and its
    // child lookup retains the actual selected entry names. Preserve the
    // caller's lexical prefix (including `./`) while replacing only that
    // child suffix.
    let lookup_path = path_from_wire_bytes(&opened_name);
    let mut lookup_physical = if legacy_path_is_absolute(&opened_name) {
        lookup_path
    } else {
        executable_root.join(lookup_path)
    };
    let mut selected_child_count = 0;
    while !lookup_physical.exists() {
        if !lookup_physical.pop() {
            break;
        }
        selected_child_count += 1;
    }
    if selected_child_count != 0 {
        for _ in 0..selected_child_count {
            truncate_last_wire_component(&mut opened_name);
        }
        let mut selected_components = path
            .components()
            .rev()
            .filter_map(|component| match component {
                std::path::Component::Normal(name) => Some(path_wire_bytes(Path::new(name))),
                _ => None,
            })
            .take(selected_child_count)
            .collect::<Vec<_>>();
        selected_components.reverse();
        for component in selected_components {
            if !opened_name.is_empty()
                && !opened_name
                    .last()
                    .is_some_and(|byte| matches!(byte, b'/' | b'\\'))
            {
                opened_name.push(separator);
            }
            opened_name.extend(component);
        }
    }

    #[cfg(windows)]
    if let Some(actual_basename) = original_basename_bytes(path) {
        // MakeOriginalFilename uses _findfirst to replace the final component
        // with its on-disk spelling on Windows; parent components stay as
        // supplied to Open.
        let basename_start = opened_name
            .iter()
            .rposition(|byte| matches!(byte, b'/' | b'\\'))
            .map_or(0, |index| index + 1);
        opened_name.truncate(basename_start);
        opened_name.extend_from_slice(&actual_basename);
    }

    opened_name
}

pub(crate) fn executable_relative_wire_name(
    mut wire_name: Vec<u8>,
    executable_root: &Path,
) -> Vec<u8> {
    let executable_root = path_wire_bytes(executable_root);
    let prefix_matches = wire_name.len() >= executable_root.len()
        && wire_name[..executable_root.len()]
            .iter()
            .zip(&executable_root)
            .all(|(left, right)| {
                if cfg!(windows) {
                    legacy_byte_capital(*left) == legacy_byte_capital(*right)
                } else {
                    left == right
                }
            });
    if prefix_matches {
        wire_name.drain(..executable_root.len());
        let separator = std::path::MAIN_SEPARATOR as u8;
        if !executable_root.ends_with(&[separator]) && wire_name.first() == Some(&separator) {
            wire_name.remove(0);
        }
    }
    wire_name
}

pub(crate) fn executable_relative_group_name(path: &Path, executable_root: &Path) -> Vec<u8> {
    let lookup_name = path_wire_bytes(path);
    let opened_name = opened_group_name(path, &lookup_name, executable_root);
    executable_relative_wire_name(opened_name, executable_root)
}

pub(crate) fn opened_physical_group_name(path: &Path, executable_root: &Path) -> Vec<u8> {
    let lookup_name = path_wire_bytes(path);
    opened_group_name(path, &lookup_name, executable_root)
}

fn truncate_last_wire_component(path: &mut Vec<u8>) {
    while path.last().is_some_and(|byte| matches!(byte, b'/' | b'\\')) && path.len() > 1 {
        path.pop();
    }
    match path.iter().rposition(|byte| matches!(byte, b'/' | b'\\')) {
        Some(0) => path.truncate(1),
        Some(separator) => path.truncate(separator),
        None => path.clear(),
    }
}

fn legacy_path_is_absolute(path: &[u8]) -> bool {
    if cfg!(windows) {
        path.first()
            .is_some_and(|byte| matches!(byte, b'/' | b'\\'))
            || path.get(1) == Some(&b':')
    } else {
        path.first() == Some(&b'/')
    }
}

#[cfg(windows)]
fn original_basename_bytes(path: &Path) -> Option<Vec<u8>> {
    let requested = path_wire_bytes(Path::new(path.file_name()?));
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::read_dir(parent).ok()?.find_map(|entry| {
        let entry = entry.ok()?;
        let name = path_wire_bytes(Path::new(&entry.file_name()));
        wildcard_match(&requested, &name).then_some(name)
    })
}

#[cfg(windows)]
fn wildcard_match(pattern: &[u8], value: &[u8]) -> bool {
    let (mut pattern_index, mut value_index) = (0usize, 0usize);
    let (mut backtrack_pattern, mut backtrack_value) = (None, None);
    while pattern_index < pattern.len() || backtrack_pattern.is_some() {
        if pattern.get(pattern_index) == Some(&b'*') {
            pattern_index += 1;
            backtrack_pattern = Some(pattern_index);
            backtrack_value = Some(value_index);
        } else if pattern.get(pattern_index) == Some(&b'?')
            || pattern
                .get(pattern_index)
                .zip(value.get(value_index))
                .is_some_and(|(left, right)| {
                    legacy_byte_capital(*left) == legacy_byte_capital(*right)
                })
        {
            pattern_index += 1;
            value_index += 1;
        } else if let (Some(saved_pattern), Some(saved_value)) =
            (backtrack_pattern, backtrack_value)
        {
            pattern_index = saved_pattern;
            value_index = saved_value + 1;
            backtrack_value = Some(value_index);
        } else {
            return false;
        }
        if value_index > value.len() {
            return false;
        }
    }
    pattern_index == pattern.len() && value_index == value.len()
}

fn legacy_byte_capital(byte: u8) -> u8 {
    match byte {
        b'a'..=b'z' => byte - 32,
        0xe4 => 0xc4,
        0xf6 => 0xd6,
        0xfc => 0xdc,
        _ => byte,
    }
}

pub fn path_wire_bytes(path: &Path) -> Vec<u8> {
    clonk_resources::path_to_legacy_bytes(path)
}

pub fn path_from_wire_bytes(bytes: &[u8]) -> PathBuf {
    clonk_resources::path_from_legacy_bytes(bytes)
}
