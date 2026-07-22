//! Byte-exact readers and writers for legacy native `C4Config` files,
//! shared by netplay's frozen player selection and the app's dozens of
//! local-config call sites (language, gamepad, graphics, toasts).

use std::io;

use clonk_engine::LegacyCString;

pub const CFG_MAX_STRING: usize = 1024;

/// One value written into a legacy native-byte configuration file.
///
/// Fixed C4Config string buffers use `RCT_Escaped`; simple scalar fields such
/// as boolean preferences are written as unquoted ASCII instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeConfigValue<'a> {
    /// Native C4 bytes, already converted from presentation text. The writer
    /// applies C++'s NUL termination and `CFG_MaxString` bound.
    CppEscapedString(&'a [u8]),
    /// An unquoted, single-line ASCII scalar such as `"0"` or `"1"`.
    /// Existing assignments are canonicalized to `Key=value` because C++'s
    /// Boolean reader does not skip whitespace after the equals sign.
    RawAscii(&'a str),
}

#[derive(Clone, Copy, Debug)]
struct NativeConfigLine {
    start: usize,
    content_end: usize,
    end: usize,
}

#[derive(Debug)]
struct EncodedNativeConfigUpdate {
    line: Vec<u8>,
    value: Vec<u8>,
    canonical_assignment: bool,
}

/// Reads one exact-case legacy config value without requiring the complete
/// file to be UTF-8. The returned bytes have the same escaped-string and
/// fixed-buffer decoding used by classic `C4Config`.
pub fn configured_native_value(config: &[u8], section: &str, key: &str) -> Option<LegacyCString> {
    configured_native_value_with_limit(config, section, key, CFG_MAX_STRING)
}

/// Reads one exact-case legacy config value into a dynamic `StdStrBuf` rather
/// than C4Config's common `CFG_MaxString` fixed buffer.
pub fn configured_native_dynamic_value(
    config: &[u8],
    section: &str,
    key: &str,
) -> Option<LegacyCString> {
    configured_native_value_with_limit(config, section, key, usize::MAX)
}

/// Reads one native Boolean with `StdCompilerINIRead::Boolean`'s exact value
/// grammar. In particular, the value starts immediately after `=` and the
/// textual forms are case-sensitive prefixes.
pub fn configured_native_boolean(config: &[u8], section: &str, key: &str) -> Option<bool> {
    let value = configured_native_scalar(config, section, key)?;
    if value.first() == Some(&b'1') && !value.get(1).is_some_and(u8::is_ascii_digit) {
        Some(true)
    } else if value.first() == Some(&b'0') && !value.get(1).is_some_and(u8::is_ascii_digit) {
        Some(false)
    } else if value.starts_with(b"true") {
        Some(true)
    } else if value.starts_with(b"false") {
        Some(false)
    } else {
        None
    }
}

/// Returns the undecoded bytes following `=` for the first exact key in the
/// first exact live section. Numeric and Boolean C4Config fields consume this
/// scalar grammar directly rather than accepting quoted string values.
pub fn configured_native_scalar<'a>(
    config: &'a [u8],
    section: &str,
    key: &str,
) -> Option<&'a [u8]> {
    let mut in_section = false;
    let mut selected_section = false;
    for raw_line in native_config_lines(config) {
        let line = &config[raw_line.start..raw_line.content_end];
        if let Some(name) = native_config_section_name(line) {
            if in_section {
                break;
            }
            let matches = name == section.as_bytes();
            in_section = matches && !selected_section;
            selected_section |= matches;
            continue;
        }
        if !in_section {
            continue;
        }
        let Some((name, value)) = native_config_assignment(line) else {
            continue;
        };
        if name == key.as_bytes() {
            return Some(value);
        }
    }
    None
}

fn configured_native_value_with_limit(
    config: &[u8],
    section: &str,
    key: &str,
    max_length: usize,
) -> Option<LegacyCString> {
    let mut in_section = false;
    let mut selected_section = false;
    for raw_line in native_config_lines(config) {
        let line = &config[raw_line.start..raw_line.content_end];
        let structural = trim_ascii(line);
        if let Some(name) = native_config_section_name(line) {
            if in_section {
                break;
            }
            let matches = name == section.as_bytes();
            in_section = matches && !selected_section;
            selected_section |= matches;
            continue;
        }
        if !in_section || structural.starts_with(b"#") || structural.starts_with(b";") {
            continue;
        }
        let Some(equals) = line.iter().position(|byte| *byte == b'=') else {
            continue;
        };
        if trim_ascii(&line[..equals]) == key.as_bytes() {
            return LegacyCString::from_bytes(decode_general_config_string(
                &line[equals + 1..],
                max_length,
            ));
        }
    }
    None
}

/// Updates selected values without requiring the complete legacy config to be
/// UTF-8. Only the first exact, live section and first exact key occurrence
/// are significant, matching `configured_native_value` and C++'s first
/// `NameNode` lookup. All unrelated source bytes are retained verbatim.
pub fn update_configured_native_values(
    config: &[u8],
    section: &str,
    updates: &[(&str, NativeConfigValue<'_>)],
) -> io::Result<Vec<u8>> {
    if updates.is_empty() {
        return Ok(config.to_vec());
    }
    validate_native_config_name(section, "section")?;

    let mut encoded_updates = Vec::with_capacity(updates.len());
    for (index, (key, value)) in updates.iter().enumerate() {
        validate_native_config_name(key, "key")?;
        if updates[..index].iter().any(|(previous, _)| previous == key) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("duplicate native config update key: {key}"),
            ));
        }
        let canonical_assignment = matches!(value, NativeConfigValue::RawAscii(_));
        let mut encoded_value = Vec::new();
        match value {
            NativeConfigValue::CppEscapedString(value) => {
                let value = value
                    .iter()
                    .copied()
                    .take_while(|byte| *byte != 0)
                    .take(CFG_MAX_STRING);
                write_cpp_escaped_config_string(&mut encoded_value, value);
            }
            NativeConfigValue::RawAscii(value) => {
                if !value.is_ascii()
                    || value
                        .as_bytes()
                        .iter()
                        .any(|byte| matches!(byte, b'\0' | b'\r' | b'\n'))
                {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("native config value for {key} is not single-line ASCII"),
                    ));
                }
                encoded_value.extend_from_slice(value.as_bytes());
            }
        }
        let mut line = Vec::with_capacity(key.len() + 1 + encoded_value.len());
        line.extend_from_slice(key.as_bytes());
        line.push(b'=');
        line.extend_from_slice(&encoded_value);
        encoded_updates.push(EncodedNativeConfigUpdate {
            line,
            value: encoded_value,
            canonical_assignment,
        });
    }

    let lines = native_config_lines(config);
    let mut section_line = None;
    let mut section_end = lines.len();
    for (index, line) in lines.iter().enumerate() {
        let Some(name) = native_config_section_name(&config[line.start..line.content_end]) else {
            continue;
        };
        if let Some(selected) = section_line {
            if index > selected {
                section_end = index;
                break;
            }
        } else if name == section.as_bytes() {
            section_line = Some(index);
        }
    }

    let line_ending = section_line
        .and_then(|index| native_config_line_ending(config, lines[index]))
        .or_else(|| {
            lines
                .iter()
                .find_map(|line| native_config_line_ending(config, *line))
        })
        .unwrap_or(b"\n");

    let Some(section_line) = section_line else {
        let mut output = config.to_vec();
        ensure_native_config_line_break(&mut output, line_ending);
        output.push(b'[');
        output.extend_from_slice(section.as_bytes());
        output.push(b']');
        output.extend_from_slice(line_ending);
        for update in &encoded_updates {
            output.extend_from_slice(&update.line);
            output.extend_from_slice(line_ending);
        }
        return Ok(output);
    };

    let mut replacements = vec![None; lines.len()];
    let mut found = vec![false; updates.len()];
    for line_index in section_line + 1..section_end {
        let content = &config[lines[line_index].start..lines[line_index].content_end];
        let structural = trim_ascii(content);
        if structural.starts_with(b"#") || structural.starts_with(b";") {
            continue;
        }
        let Some(equals) = content.iter().position(|byte| *byte == b'=') else {
            continue;
        };
        let key = trim_ascii(&content[..equals]);
        if let Some(update_index) =
            updates
                .iter()
                .enumerate()
                .find_map(|(index, (candidate, _))| {
                    (!found[index] && key == candidate.as_bytes()).then_some(index)
                })
        {
            found[update_index] = true;
            replacements[line_index] =
                Some((update_index, native_config_value_span(content, equals)));
        }
    }

    let mut output = Vec::with_capacity(config.len() + encoded_updates.len() * 32);
    for (index, line) in lines.iter().enumerate() {
        if index == section_end {
            append_missing_native_config_values(&mut output, &encoded_updates, &found, line_ending);
        }
        if let Some((update_index, value_span)) = &replacements[index] {
            let update = &encoded_updates[*update_index];
            if update.canonical_assignment {
                output.extend_from_slice(&update.line);
            } else {
                output.extend_from_slice(&config[line.start..line.start + value_span.start]);
                output.extend_from_slice(&update.value);
            }
            output.extend_from_slice(&config[line.start + value_span.end..line.end]);
        } else {
            output.extend_from_slice(&config[line.start..line.end]);
        }
    }
    if section_end == lines.len() {
        append_missing_native_config_values(&mut output, &encoded_updates, &found, line_ending);
    }
    Ok(output)
}

fn validate_native_config_name(name: &str, kind: &str) -> io::Result<()> {
    if !name.is_empty()
        && name.is_ascii()
        && name
            .as_bytes()
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b' ' | b'_'))
        && name.as_bytes()[0].is_ascii_alphabetic()
    {
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("invalid native config {kind}: {name}"),
    ))
}

fn native_config_lines(config: &[u8]) -> Vec<NativeConfigLine> {
    let mut lines = Vec::new();
    let mut start = 0;
    while start < config.len() {
        let content_end = config[start..]
            .iter()
            .position(|byte| matches!(byte, b'\r' | b'\n'))
            .map_or(config.len(), |offset| start + offset);
        let end = if content_end == config.len() {
            content_end
        } else if config[content_end] == b'\r' && config.get(content_end + 1) == Some(&b'\n') {
            content_end + 2
        } else {
            content_end + 1
        };
        lines.push(NativeConfigLine {
            start,
            content_end,
            end,
        });
        start = end;
    }
    lines
}

fn native_config_section_name(line: &[u8]) -> Option<&[u8]> {
    let start = line
        .iter()
        .position(|byte| !matches!(byte, b' ' | b'\t'))
        .unwrap_or(line.len());
    let structural = &line[start..];
    if structural.first() != Some(&b'[') || !structural.get(1).is_some_and(u8::is_ascii_alphabetic)
    {
        return None;
    }
    let mut cursor = 1;
    while structural
        .get(cursor)
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b' ' | b'_'))
    {
        cursor += 1;
    }
    let name_end = cursor;
    while matches!(structural.get(cursor), Some(b' ' | b'\t')) {
        cursor += 1;
    }
    (structural.get(cursor) == Some(&b']')).then_some(&structural[1..name_end])
}

fn native_config_assignment(line: &[u8]) -> Option<(&[u8], &[u8])> {
    let start = line
        .iter()
        .position(|byte| !matches!(byte, b' ' | b'\t'))
        .unwrap_or(line.len());
    if !line.get(start).is_some_and(u8::is_ascii_alphabetic) {
        return None;
    }
    let mut cursor = start + 1;
    while line
        .get(cursor)
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b' ' | b'_'))
    {
        cursor += 1;
    }
    let name_end = cursor;
    while matches!(line.get(cursor), Some(b' ' | b'\t')) {
        cursor += 1;
    }
    (line.get(cursor) == Some(&b'=')).then_some((&line[start..name_end], &line[cursor + 1..]))
}

fn native_config_value_span(line: &[u8], equals: usize) -> std::ops::Range<usize> {
    let mut start = equals + 1;
    while matches!(line.get(start), Some(b' ' | b'\t')) {
        start += 1;
    }
    if line.get(start) == Some(&b'"') {
        let mut cursor = start + 1;
        while cursor < line.len() {
            match line[cursor] {
                b'\\' if cursor + 1 < line.len() => cursor += 2,
                b'"' => return start..cursor + 1,
                _ => cursor += 1,
            }
        }
        return start..line.len();
    }

    let comment = (start..line.len())
        .find(|index| line[*index] == b'#' && line[*index - 1].is_ascii_whitespace());
    if comment == Some(start) {
        // Preserve the whitespace that separated an inline comment from an
        // empty value by inserting before it: `Key= #note` -> `Key="x" #note`.
        return equals + 1..equals + 1;
    }
    let mut end = comment.unwrap_or(line.len());
    while end > start && matches!(line[end - 1], b' ' | b'\t') {
        end -= 1;
    }
    start..end
}

fn native_config_line_ending(config: &[u8], line: NativeConfigLine) -> Option<&[u8]> {
    (line.content_end < line.end).then(|| &config[line.content_end..line.end])
}

fn ensure_native_config_line_break(output: &mut Vec<u8>, line_ending: &[u8]) {
    if !output.is_empty() && !output.ends_with(b"\n") && !output.ends_with(b"\r") {
        output.extend_from_slice(line_ending);
    }
}

fn append_missing_native_config_values(
    output: &mut Vec<u8>,
    encoded_updates: &[EncodedNativeConfigUpdate],
    found: &[bool],
    line_ending: &[u8],
) {
    if found.iter().all(|found| *found) {
        return;
    }
    ensure_native_config_line_break(output, line_ending);
    for (update, found) in encoded_updates.iter().zip(found) {
        if !found {
            output.extend_from_slice(&update.line);
            output.extend_from_slice(line_ending);
        }
    }
}

fn write_cpp_escaped_config_string(output: &mut Vec<u8>, value: impl IntoIterator<Item = u8>) {
    output.push(b'"');
    let mut last_numeric_escape = false;
    for byte in value {
        let escape_digit = last_numeric_escape && byte.is_ascii_digit();
        last_numeric_escape = false;
        if !escape_digit {
            let named_escape = match byte {
                b'\x07' => Some(b'a'),
                b'\x08' => Some(b'b'),
                b'\x0c' => Some(b'f'),
                b'\n' => Some(b'n'),
                b'\r' => Some(b'r'),
                b'\t' => Some(b't'),
                b'\x0b' => Some(b'v'),
                b'"' => Some(b'"'),
                b'\\' => Some(b'\\'),
                _ => None,
            };
            if let Some(escaped) = named_escape {
                output.extend_from_slice(&[b'\\', escaped]);
                continue;
            }
            if (b' '..=b'~').contains(&byte) {
                output.push(byte);
                continue;
            }
        }
        output.push(b'\\');
        write_unpadded_octal(output, byte);
        last_numeric_escape = true;
    }
    output.push(b'"');
}

fn write_unpadded_octal(output: &mut Vec<u8>, byte: u8) {
    let high = byte / 64;
    let middle = (byte / 8) % 8;
    if high != 0 {
        output.push(b'0' + high);
    }
    if high != 0 || middle != 0 {
        output.push(b'0' + middle);
    }
    output.push(b'0' + byte % 8);
}

pub fn decode_general_config_string(value: &[u8], max_length: usize) -> Vec<u8> {
    let value = trim_horizontal_start(value);
    clonk_core::std_config::decode_cpp_escaped_string(value, max_length)
        .unwrap_or_else(|| recover_unquoted_rust_config_value(value, max_length))
}

// Compatibility recovery, not C++ parity: the current Rust Config writer
// leaves whitespace-free values unquoted. C++ fixed-buffer RCT_Escaped fields
// require quotes, but rejecting these values would discard Participants that
// this port has already persisted.
fn recover_unquoted_rust_config_value(value: &[u8], max_length: usize) -> Vec<u8> {
    value
        .iter()
        .copied()
        .take_while(|byte| *byte != 0)
        .take(max_length)
        .collect()
}

pub fn trim_horizontal_start(value: &[u8]) -> &[u8] {
    let start = value
        .iter()
        .position(|byte| !matches!(byte, b' ' | b'\t'))
        .unwrap_or(value.len());
    &value[start..]
}

pub fn trim_ascii(value: &[u8]) -> &[u8] {
    let start = value
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(value.len());
    let end = value
        .iter()
        .rposition(|byte| !byte.is_ascii_whitespace())
        .map_or(start, |index| index + 1);
    &value[start..end]
}

#[cfg(test)]
mod tests {
    #[test]
    fn configured_native_value_reads_cp1252_and_escaped_values_without_utf8() {
        let config =
            b"[General]\nName=\"M\x81ker\"\n\n[Network]\nComment=\"Gr\xfc\\337e\"\nPortTCP=12345\n";
        assert_eq!(
            super::configured_native_value(config, "General", "Name")
                .expect("native maker")
                .as_bytes(),
            b"M\x81ker"
        );
        assert_eq!(
            super::configured_native_value(config, "Network", "Comment")
                .expect("native comment")
                .as_bytes(),
            b"Gr\xfc\xdfe"
        );
        assert_eq!(
            super::configured_native_value(config, "Network", "PortTCP")
                .expect("numeric value")
                .as_bytes(),
            b"12345"
        );

        let mut dynamic = b"[Network]\nLocalName=\"".to_vec();
        dynamic.extend(std::iter::repeat_n(b'{', super::CFG_MAX_STRING + 1));
        dynamic.extend_from_slice(b"Alice\"\n");
        assert!(
            super::configured_native_dynamic_value(&dynamic, "Network", "LocalName")
                .expect("dynamic native name")
                .as_bytes()
                .ends_with(b"Alice")
        );
    }

    #[test]
    fn l010_std_config_and_native_config_decoders_agree_on_cpp_escapes() {
        let config = br#"[Network]
Comment="M\303\274ller\\path\"quoted\""
"#;
        let native =
            super::configured_native_value(config, "Network", "Comment").expect("native comment");
        let mut reader = std::io::Cursor::new(config);
        let parsed =
            clonk_core::std_config::Config::from_reader(&mut reader).expect("parse shared config");
        let decoded = parsed
            .get_in(Some("Network"), "Comment")
            .expect("parsed comment");

        assert_eq!(decoded, "Müller\\path\"quoted\"");
        assert_eq!(decoded.as_bytes(), native.as_bytes());
    }

    #[test]
    fn native_config_update_preserves_unrelated_invalid_utf8_and_crlf() {
        let config = b"[General]\r\nName=\"M\x81ker\"\r\n[IRC]\r\nServer2=\"irc.example.test\"\r\nNick=\"Old\" #keep\r\nRealName= #real-name\r\nChannel=#old\r\n[Vendor]\r\nOpaque=\"\xfe\"\r\n";
        let updated = super::update_configured_native_values(
            config,
            "IRC",
            &[
                (
                    "Nick",
                    super::NativeConfigValue::CppEscapedString(b"NewNick"),
                ),
                (
                    "RealName",
                    super::NativeConfigValue::CppEscapedString(b"Gr\xfc1"),
                ),
                (
                    "Channel",
                    super::NativeConfigValue::CppEscapedString(b"#new"),
                ),
            ],
        )
        .expect("patch native IRC values");

        assert_eq!(
            updated,
            b"[General]\r\nName=\"M\x81ker\"\r\n[IRC]\r\nServer2=\"irc.example.test\"\r\nNick=\"NewNick\" #keep\r\nRealName=\"Gr\\374\\61\" #real-name\r\nChannel=\"#new\"\r\n[Vendor]\r\nOpaque=\"\xfe\"\r\n"
        );
        assert_eq!(
            super::configured_native_value(&updated, "IRC", "RealName")
                .expect("updated native real name")
                .as_bytes(),
            b"Gr\xfc1"
        );
        assert_eq!(
            super::configured_native_value(&updated, "IRC", "Server2")
                .expect("untouched IRC server")
                .as_bytes(),
            b"irc.example.test"
        );
    }

    #[test]
    fn native_config_update_uses_first_live_section_and_inserts_missing_keys() {
        let config = b"#[IRC]\nNick=\"Commented\"\n\x0b[IRC]\nNick=\"VerticalWhitespace\"\n[ IRC]\nNick=\"MalformedLeadingSpace\"\n[IRC] # first live section\nServer2=\"irc.first.test\"\n[Vendor!x]\nNick=\"First\"\nNick=\"Duplicate\"\n[IRC ]\nNick=\"PaddedSection\"\n[IRC]\nNick=\"Second\"\n";
        let updated = super::update_configured_native_values(
            config,
            "IRC",
            &[
                (
                    "Nick",
                    super::NativeConfigValue::CppEscapedString(b"Newest"),
                ),
                (
                    "RealName",
                    super::NativeConfigValue::CppEscapedString(b"Name"),
                ),
                (
                    "Channel",
                    super::NativeConfigValue::CppEscapedString(b"#channel"),
                ),
            ],
        )
        .expect("patch first live IRC section");

        assert_eq!(
            updated,
            b"#[IRC]\nNick=\"Commented\"\n\x0b[IRC]\nNick=\"VerticalWhitespace\"\n[ IRC]\nNick=\"MalformedLeadingSpace\"\n[IRC] # first live section\nServer2=\"irc.first.test\"\n[Vendor!x]\nNick=\"Newest\"\nNick=\"Duplicate\"\nRealName=\"Name\"\nChannel=\"#channel\"\n[IRC ]\nNick=\"PaddedSection\"\n[IRC]\nNick=\"Second\"\n"
        );
        assert_eq!(
            super::configured_native_value(&updated, "IRC", "Nick")
                .expect("updated first IRC nick")
                .as_bytes(),
            b"Newest"
        );
    }

    #[test]
    fn native_config_update_appends_sections_and_writes_raw_ascii() {
        let config = b"[General]\rName=Clonker";
        let updated = super::update_configured_native_values(
            config,
            "Startup",
            &[(
                "HideMsgIRCDangerous",
                super::NativeConfigValue::RawAscii("1"),
            )],
        )
        .expect("append startup preference");

        assert_eq!(
            updated,
            b"[General]\rName=Clonker\r[Startup]\rHideMsgIRCDangerous=1\r"
        );
        assert_eq!(
            super::configured_native_value(&updated, "Startup", "HideMsgIRCDangerous")
                .expect("appended bare-CR preference")
                .as_bytes(),
            b"1"
        );
    }

    #[test]
    fn native_config_update_canonicalizes_raw_ascii_for_cpp_boolean_reader() {
        let updated = super::update_configured_native_values(
            b"[General]\r\n  NoCrew = true # keep\r\nName=Clonker\r\n",
            "General",
            &[("NoCrew", super::NativeConfigValue::RawAscii("false"))],
        )
        .expect("update native Boolean");

        assert_eq!(
            updated,
            b"[General]\r\nNoCrew=false # keep\r\nName=Clonker\r\n"
        );
    }

    #[test]
    fn configured_native_boolean_matches_cpp_value_grammar() {
        for (assignment, expected) in [
            ("NoCrew=true", Some(true)),
            ("NoCrew=false", Some(false)),
            ("NoCrew=1", Some(true)),
            ("NoCrew=0", Some(false)),
            ("NoCrew=trueSuffix", Some(true)),
            ("NoCrew=falseSuffix", Some(false)),
            ("NoCrew=1x", Some(true)),
            ("NoCrew=10", None),
            ("NoCrew= true", None),
            ("NoCrew=\"true\"", None),
            ("NoCrew=TRUE", None),
            ("NoCrew=yes", None),
            ("NoCrew=on", None),
            ("NoCrew =true", None),
        ] {
            let config = format!("[General]\n{assignment}\n");
            assert_eq!(
                super::configured_native_boolean(config.as_bytes(), "General", "NoCrew"),
                expected,
                "assignment {assignment:?}"
            );
        }
    }

    #[test]
    fn native_config_update_matches_cpp_numeric_escape_disambiguation_and_bounds() {
        let updated = super::update_configured_native_values(
            b"[IRC]\n",
            "IRC",
            &[(
                "RealName",
                super::NativeConfigValue::CppEscapedString(b"\x8018A\0ignored"),
            )],
        )
        .expect("escape native numeric sequence");
        assert_eq!(updated, b"[IRC]\nRealName=\"\\200\\61\\70A\"\n");

        let oversized = vec![b'A'; super::CFG_MAX_STRING + 1];
        let bounded = super::update_configured_native_values(
            b"[IRC]\n",
            "IRC",
            &[(
                "RealName",
                super::NativeConfigValue::CppEscapedString(&oversized),
            )],
        )
        .expect("bound fixed C4Config string");
        assert_eq!(
            bounded.len(),
            b"[IRC]\nRealName=\"\"\n".len() + super::CFG_MAX_STRING
        );
        assert_eq!(
            super::configured_native_value(&bounded, "IRC", "RealName")
                .expect("bounded value")
                .as_bytes()
                .len(),
            super::CFG_MAX_STRING
        );
    }
}
