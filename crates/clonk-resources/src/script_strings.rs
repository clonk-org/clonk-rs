use std::collections::HashMap;
use std::path::PathBuf;

use crate::{ComponentGroups, Group, GroupError, ResourceLoadDiagnostic};

const C4_MAX_NAME: usize = 30;

/// Preserves UTF-8 C4Script/StringTbl text and converts invalid legacy input
/// from the Windows-1252 system charset used by classic content.
pub fn decode_legacy_script_text(data: &[u8]) -> String {
    std::str::from_utf8(data)
        .map(str::to_owned)
        .unwrap_or_else(|_| decode_legacy_system_text(data))
}

/// Decodes native legacy bytes for presentation without first interpreting a
/// coincidentally valid byte sequence as UTF-8. This is the reversible UI
/// boundary for byte-oriented configuration and wire protocols.
pub fn decode_legacy_system_text(data: &[u8]) -> String {
    let (text, _, _) = encoding_rs::WINDOWS_1252.decode(data);
    text.into_owned()
}

/// Encodes presentation/configuration text into the Windows-1252 system
/// charset used by classic native strings. Returns `None` when a scalar is
/// not representable instead of accepting replacement output.
pub fn encode_legacy_script_text(text: &str) -> Option<Vec<u8>> {
    const C4_RAW_BYTE_ESCAPE_BASE: u32 = 0xF0000;
    const C4_RAW_BYTE_ESCAPE_END: u32 = C4_RAW_BYTE_ESCAPE_BASE + u8::MAX as u32;

    let mut output = Vec::with_capacity(text.len());
    for character in text.chars() {
        let scalar = u32::from(character);
        if (C4_RAW_BYTE_ESCAPE_BASE..=C4_RAW_BYTE_ESCAPE_END).contains(&scalar) {
            output.push((scalar - C4_RAW_BYTE_ESCAPE_BASE) as u8);
            continue;
        }
        let mut utf8 = [0; 4];
        let (bytes, _, had_errors) =
            encoding_rs::WINDOWS_1252.encode(character.encode_utf8(&mut utf8));
        if had_errors {
            return None;
        }
        output.extend_from_slice(&bytes);
    }
    Some(output)
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
    localize_script_source_with_components(&ComponentGroups::local(group), source, languages)
}

/// Applies a script string table using the local group followed by the
/// language-pack groups at the same logical path.
pub fn localize_script_source_with_components<S: AsRef<str>>(
    components: &ComponentGroups,
    source: &str,
    languages: &[S],
) -> Result<String, GroupError> {
    localize_script_source_with_components_and_diagnostics(
        components,
        source,
        languages,
        ResourceLoadDiagnostic::emit,
    )
}

/// Applies a script string table while returning non-fatal missing-key
/// diagnostics to the caller instead of emitting them immediately.
pub fn localize_script_source_with_components_and_diagnostics<S: AsRef<str>>(
    components: &ComponentGroups,
    source: &str,
    languages: &[S],
    mut report_diagnostic: impl FnMut(ResourceLoadDiagnostic),
) -> Result<String, GroupError> {
    let (table, table_path) = load_script_string_table(components, languages)?;
    let entries = parse_string_table(&table);
    let source = clonk_script::c4_string_bytes(source);
    Ok(clonk_script::c4_string_from_bytes(
        &replace_localization_keys_with_diagnostics(
            &source,
            &entries,
            &table_path,
            &mut report_diagnostic,
        ),
    ))
}

fn load_script_string_table<S: AsRef<str>>(
    components: &ComponentGroups,
    languages: &[S],
) -> Result<(Vec<u8>, PathBuf), GroupError> {
    let mut table = None;
    let mut table_path = None;
    // C4ComponentHost copies at most two native bytes from each LanguageEx
    // segment, and its C string input stops at the first NUL.
    for candidate in
        std::iter::once("StringTbl.txt".to_string()).chain(languages.iter().map(|language| {
            let code = clonk_script::c4_string_bytes(language.as_ref());
            let visible = code
                .iter()
                .position(|byte| *byte == 0)
                .unwrap_or(code.len());
            let code = clonk_script::c4_string_from_bytes(&code[..visible.min(2)]);
            format!("StringTbl{code}.txt")
        }))
    {
        let Some(component) = components.read(&candidate)? else {
            continue;
        };
        // C4LangStringTable copies and scans this component as a native C
        // string; entries after the first NUL are not part of the table.
        let bytes = component
            .bytes
            .split(|byte| *byte == 0)
            .next()
            .unwrap_or_default()
            .to_vec();
        table = Some(bytes);
        table_path = Some(component.path);
        break;
    }

    Ok((
        table.unwrap_or_default(),
        table_path.unwrap_or_else(|| PathBuf::from("StringTbl.txt")),
    ))
}

fn parse_string_table(table: &[u8]) -> HashMap<&[u8], &[u8]> {
    let mut entries = HashMap::new();
    for line in table.split(|byte| matches!(*byte, b'\r' | b'\n')) {
        let Some(separator) = line.iter().position(|byte| *byte == b'=') else {
            continue;
        };
        let (key, value) = (&line[..separator], &line[separator + 1..]);
        // C++ searches its entry vector from the front, so the first
        // duplicate wins.
        entries.entry(key).or_insert(value);
    }
    entries
}

#[cfg(test)]
fn replace_localization_keys(
    source: &[u8],
    entries: &HashMap<&[u8], &[u8]>,
    table_path: &std::path::Path,
) -> Vec<u8> {
    replace_localization_keys_with_diagnostics(
        source,
        entries,
        table_path,
        &mut ResourceLoadDiagnostic::emit,
    )
}

fn replace_localization_keys_with_diagnostics(
    source: &[u8],
    entries: &HashMap<&[u8], &[u8]>,
    table_path: &std::path::Path,
    report_diagnostic: &mut impl FnMut(ResourceLoadDiagnostic),
) -> Vec<u8> {
    let mut result = Vec::with_capacity(source.len());
    let mut copied_through = 0;
    let mut search_from = 0;

    while let Some(open_offset) = source[search_from..].iter().position(|byte| *byte == b'$') {
        let open = search_from + open_offset;
        let key_start = open + 1;
        // C++ copies at most C4MaxName bytes, then checks the following byte.
        // On an overlong run, its next delimiter search resumes from here.
        let key_len = source[key_start..]
            .iter()
            .take(C4_MAX_NAME)
            .take_while(|byte| **byte != b'$')
            .count();
        let close = key_start + key_len;
        let Some(&terminator) = source.get(close) else {
            break;
        };
        search_from = close + 1;
        if terminator != b'$' {
            continue;
        }

        let key = &source[key_start..close];
        let valid = key
            .iter()
            .copied()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'~' | b'+' | b'-'));
        if !valid {
            continue;
        }
        let Some(value) = entries.get(key) else {
            report_diagnostic(ResourceLoadDiagnostic::ScriptStringTableEntryNotFound {
                path: table_path.to_path_buf(),
                key: String::from_utf8_lossy(key).into_owned(),
            });
            continue;
        };

        result.extend_from_slice(&source[copied_through..open]);
        result.extend_from_slice(value);
        copied_through = close + 1;
    }

    if copied_through == 0 {
        return source.to_vec();
    }
    result.extend_from_slice(&source[copied_through..]);
    result
}

pub(crate) fn emit_missing_script_string(table_path: &std::path::Path, key: &str) {
    tracing::warn!(path = %table_path.display(), %key, "string table entry not found");
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tempdir() -> std::io::Result<tempfile::TempDir> {
        tempfile::Builder::new().prefix("lc-test-").tempdir()
    }

    #[test]
    fn script_localization_preserves_native_source_and_replacement_bytes() {
        let directory = tempdir().expect("tempdir");
        std::fs::write(
            directory.path().join("StringTblUS.txt"),
            [b"Raw=".as_slice(), &[0xe9, 0xff], b"\n"].concat(),
        )
        .expect("write raw string table");
        let group = Group::open(directory.path()).expect("open group");
        let source = clonk_script::c4_string_from_bytes(&[
            b'"', 0xff, b' ', b'$', b'R', b'a', b'w', b'$', b'"',
        ]);

        let localized = localize_script_source(&group, &source, &["US"]).expect("source localizes");
        assert_eq!(
            clonk_script::c4_string_bytes(&localized),
            [b'"', 0xff, b' ', 0xe9, 0xff, b'"']
        );
    }

    #[test]
    fn localization_reports_missing_keys_in_source_order() {
        let directory = tempdir().expect("tempdir");
        let table_path = directory.path().join("StringTblUS.txt");
        std::fs::write(&table_path, b"Known=value\n").expect("write string table");
        let group = Group::open(directory.path()).expect("open group");
        let mut diagnostics = Vec::new();

        let localized = localize_script_source_with_components_and_diagnostics(
            &ComponentGroups::local(&group),
            "$First$/$Known$/$Second$",
            &["US"],
            |diagnostic| diagnostics.push(diagnostic),
        )
        .expect("source localizes");

        assert_eq!(localized, "$First$/value/$Second$");
        assert_eq!(
            diagnostics,
            [
                ResourceLoadDiagnostic::ScriptStringTableEntryNotFound {
                    path: table_path.clone(),
                    key: "First".to_string(),
                },
                ResourceLoadDiagnostic::ScriptStringTableEntryNotFound {
                    path: table_path,
                    key: "Second".to_string(),
                },
            ]
        );
    }

    #[test]
    fn overlong_localization_run_reuses_its_closing_dollar_as_next_opener() {
        let overlong = vec![b'A'; C4_MAX_NAME + 5];
        let source = [b"$".as_slice(), &overlong, b"$Key$"].concat();
        let expected = [b"$".as_slice(), &overlong, b"V"].concat();
        let entries = HashMap::from([(b"Key".as_slice(), b"V".as_slice())]);

        assert_eq!(
            replace_localization_keys(&source, &entries, std::path::Path::new("StringTbl.txt"),),
            expected
        );
    }

    #[test]
    fn localization_key_length_boundary_matches_c4maxname() {
        let key_at_limit = vec![b'A'; C4_MAX_NAME];
        let key_over_limit = vec![b'B'; C4_MAX_NAME + 1];
        let source = [
            b"$".as_slice(),
            &key_at_limit,
            b"$|$",
            &key_over_limit,
            b"$",
        ]
        .concat();
        let expected = [b"accepted|$".as_slice(), &key_over_limit, b"$"].concat();
        let entries = HashMap::from([
            (key_at_limit.as_slice(), b"accepted".as_slice()),
            (key_over_limit.as_slice(), b"rejected".as_slice()),
        ]);

        assert_eq!(
            replace_localization_keys(&source, &entries, std::path::Path::new("StringTbl.txt"),),
            expected
        );
    }

    #[test]
    fn script_string_table_stops_at_the_native_nul_terminator() {
        let directory = tempdir().expect("tempdir");
        std::fs::write(
            directory.path().join("StringTblUS.txt"),
            b"Before=kept\0After=hidden\n",
        )
        .expect("write NUL-terminated string table");
        let group = Group::open(directory.path()).expect("open group");

        let localized =
            localize_script_source(&group, "$Before$/$After$", &["US"]).expect("source localizes");
        assert_eq!(localized, "kept/$After$");
    }

    #[test]
    fn script_string_table_uses_two_native_language_bytes_before_nul() {
        let directory = tempdir().expect("tempdir");
        std::fs::write(directory.path().join("StringTblDE.txt"), b"Code=two-byte\n")
            .expect("write two-byte table");
        std::fs::write(
            directory.path().join("StringTblDE-extra.txt"),
            b"Code=untruncated\n",
        )
        .expect("write untruncated decoy");
        std::fs::write(directory.path().join("StringTblD.txt"), b"Code=pre-nul\n")
            .expect("write pre-NUL table");
        let group = Group::open(directory.path()).expect("open group");

        let truncated = localize_script_source(&group, "$Code$", &["DE-extra"])
            .expect("long language code localizes");
        assert_eq!(truncated, "two-byte");

        let nul_terminated = localize_script_source(&group, "$Code$", &["D\0E"])
            .expect("NUL-terminated language code localizes");
        assert_eq!(nul_terminated, "pre-nul");
    }

    #[test]
    fn empty_script_string_table_falls_through_to_localized_candidate() {
        let directory = tempdir().expect("tempdir");
        std::fs::write(directory.path().join("StringTbl.txt"), [])
            .expect("write empty default string table");
        std::fs::write(
            directory.path().join("StringTblUS.txt"),
            b"Greeting=localized\n",
        )
        .expect("write localized string table");
        let group = Group::open(directory.path()).expect("open group");

        let localized = localize_script_source(&group, "$Greeting$", &["US"])
            .expect("empty component is skipped");
        assert_eq!(localized, "localized");
    }

    #[test]
    fn nonempty_malformed_string_table_still_blocks_later_candidates() {
        let directory = tempdir().expect("tempdir");
        std::fs::write(
            directory.path().join("StringTbl.txt"),
            b"not a string-table entry\n",
        )
        .expect("write malformed default string table");
        std::fs::write(
            directory.path().join("StringTblUS.txt"),
            b"Greeting=localized\n",
        )
        .expect("write localized string table");
        let group = Group::open(directory.path()).expect("open group");

        let localized = localize_script_source(&group, "$Greeting$", &["US"])
            .expect("nonempty malformed component remains selected");
        assert_eq!(localized, "$Greeting$");
    }

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

    #[test]
    fn legacy_system_decoder_never_reinterprets_utf8_shaped_bytes() {
        let native = [0xc3, 0xa9];
        let presented = decode_legacy_system_text(&native);

        assert_eq!(presented, "\u{00c3}\u{00a9}");
        assert_eq!(encode_legacy_script_text(&presented), Some(native.to_vec()));
        assert_ne!(presented, "\u{00e9}");

        let every_native_byte = (u8::MIN..=u8::MAX).collect::<Vec<_>>();
        let presented = decode_legacy_system_text(&every_native_byte);
        assert_eq!(
            encode_legacy_script_text(&presented),
            Some(every_native_byte),
            "the presentation boundary must remain reversible for every native byte"
        );
    }

    #[test]
    fn legacy_text_encoder_uses_windows_1252_without_replacement() {
        assert_eq!(
            encode_legacy_script_text("Mäker €"),
            Some(vec![b'M', 0xe4, b'k', b'e', b'r', b' ', 0x80])
        );
        let native = clonk_script::c4_string_from_bytes(&[b'M', 0x81, b'k']);
        assert_eq!(
            encode_legacy_script_text(&native),
            Some(vec![b'M', 0x81, b'k'])
        );
        assert_eq!(encode_legacy_script_text("snowman ☃"), None);
    }
}
