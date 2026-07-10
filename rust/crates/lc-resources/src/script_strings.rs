use std::collections::HashMap;
use std::path::PathBuf;

use crate::{Group, GroupError};

const C4_MAX_NAME: usize = 30;

/// Preserves UTF-8 C4Script/StringTbl text and converts invalid legacy input
/// from the Windows-1252 system charset used by classic content.
pub fn decode_legacy_script_text(data: &[u8]) -> String {
    std::str::from_utf8(data)
        .map(str::to_owned)
        .unwrap_or_else(|_| {
            let (text, _, _) = encoding_rs::WINDOWS_1252.decode(data);
            text.into_owned()
        })
}

/// Applies the group's C4Script string table before parsing the source.
///
/// Candidate order and textual replacement mirror
/// `C4CFN_ScriptStringTbl`/`C4LangStringTable::ReplaceStrings`
/// (C4Components.h:56; C4LangStringTable.cpp:33-144).
pub fn localize_script_source<S: AsRef<str>>(
    group: &Group,
    source: &str,
    languages: &[S],
) -> Result<String, GroupError> {
    let (table, table_path) = load_script_string_table(group, languages)?;
    let entries = parse_string_table(&table);
    Ok(replace_localization_keys(source, &entries, &table_path))
}

/// Replaces localization keys inside double-quoted C4Script strings.
///
/// C++ replaces keys throughout the source before preparsing
/// (`C4ScriptHost::MakeScript`, C4ScriptHost.cpp:66-82). Rust's parser still
/// consumes the untranslated context annotations it discards, so definition
/// loading uses this runtime-visible subset until localized annotation text is
/// parsed with the same rules.
pub fn localize_quoted_script_strings<S: AsRef<str>>(
    group: &Group,
    source: &str,
    languages: &[S],
) -> Result<String, GroupError> {
    let (table, table_path) = load_script_string_table(group, languages)?;
    let entries = parse_string_table(&table);
    Ok(replace_quoted_localization_keys(
        source,
        &entries,
        &table_path,
    ))
}

fn load_script_string_table<S: AsRef<str>>(
    group: &Group,
    languages: &[S],
) -> Result<(String, PathBuf), GroupError> {
    let mut selected_name = None;
    let mut table = None;
    for candidate in std::iter::once("StringTbl.txt".to_string()).chain(
        languages
            .iter()
            .map(|language| format!("StringTbl{}.txt", language.as_ref())),
    ) {
        if !group.exists(&candidate) {
            continue;
        }
        table = Some(decode_legacy_script_text(&group.read_file(&candidate)?));
        selected_name = Some(candidate);
        break;
    }

    let table_path = group
        .root()
        .join(selected_name.as_deref().unwrap_or("StringTbl.txt"));
    Ok((table.unwrap_or_default(), table_path))
}

fn parse_string_table(table: &str) -> HashMap<&str, &str> {
    let mut entries = HashMap::new();
    for line in table.split(['\r', '\n']) {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        // C++ searches its entry vector from the front, so the first
        // duplicate wins.
        entries.entry(key).or_insert(value);
    }
    entries
}

fn replace_localization_keys(
    source: &str,
    entries: &HashMap<&str, &str>,
    table_path: &std::path::Path,
) -> String {
    let mut result = String::with_capacity(source.len());
    let mut copied_through = 0;
    let mut search_from = 0;

    while let Some(open_offset) = source[search_from..].find('$') {
        let open = search_from + open_offset;
        let key_start = open + 1;
        let Some(close_offset) = source[key_start..].find('$') else {
            break;
        };
        let close = key_start + close_offset;
        let key = &source[key_start..close];
        search_from = close + 1;

        let valid = key.len() <= C4_MAX_NAME
            && key.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'~' | b'+' | b'-')
            });
        if !valid {
            continue;
        }
        let Some(value) = entries.get(key) else {
            tracing::warn!(
                path = %table_path.display(),
                key,
                "string table entry not found"
            );
            continue;
        };

        result.push_str(&source[copied_through..open]);
        result.push_str(value);
        copied_through = close + 1;
    }

    if copied_through == 0 {
        return source.to_string();
    }
    result.push_str(&source[copied_through..]);
    result
}

fn replace_quoted_localization_keys(
    source: &str,
    entries: &HashMap<&str, &str>,
    table_path: &std::path::Path,
) -> String {
    let bytes = source.as_bytes();
    let mut result = String::with_capacity(source.len());
    let mut copied_through = 0;
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'/') {
            index += 2;
            while index < bytes.len() && !matches!(bytes[index], b'\n' | b'\r') {
                index += 1;
            }
            continue;
        }
        if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'*') {
            index += 2;
            while index + 1 < bytes.len()
                && !(bytes[index] == b'*' && bytes[index + 1] == b'/')
            {
                index += 1;
            }
            index = (index + 2).min(bytes.len());
            continue;
        }
        if bytes[index] != b'"' {
            index += 1;
            continue;
        }

        let start = index;
        index += 1;
        while index < bytes.len() {
            if bytes[index] == b'\\' {
                index = (index + 2).min(bytes.len());
            } else if bytes[index] == b'"' {
                index += 1;
                break;
            } else {
                index += 1;
            }
        }
        result.push_str(&source[copied_through..start]);
        result.push_str(&replace_localization_keys(
            &source[start..index],
            entries,
            table_path,
        ));
        copied_through = index;
    }

    result.push_str(&source[copied_through..]);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_script_decoder_preserves_valid_utf8() {
        // C4ComponentHost::LoadAppend carries the script bytes unchanged
        // (C4ComponentHost.cpp:155-213); Rust's source String must not
        // reinterpret already-valid UTF-8 while normalizing legacy input.
        assert_eq!(decode_legacy_script_text("Grüße".as_bytes()), "Grüße");
    }

    #[test]
    fn legacy_script_decoder_converts_windows_1252() {
        // Classic scripts use the same Windows system charset as the other
        // text components C4ComponentHost loads (C4ComponentHost.cpp:47-89).
        assert_eq!(
            decode_legacy_script_text(&[b'G', b'r', 0xfc, 0xdf, b'e']),
            "Grüße"
        );
    }
}
