use clonk_engine::LegacyCString;

/// Applies C++'s byte-exact `VAL_NameNoEmpty` transformation.
pub fn validate_name_no_empty(value: LegacyCString) -> LegacyCString {
    validate_name(value, false)
}

/// Applies C++'s byte-exact `VAL_NameAllowEmpty` transformation.
pub fn validate_name_allow_empty(value: LegacyCString) -> LegacyCString {
    validate_name(value, true)
}

/// `C4InVal::VAL_NameNoEmpty` / `VAL_NameAllowEmpty`
/// (`src/C4InputValidation.cpp:39-55,97-118`).
fn validate_name(value: LegacyCString, allow_empty: bool) -> LegacyCString {
    let mut bytes = if value.is_empty() && !allow_empty {
        b"empty".to_vec()
    } else {
        value.as_bytes().to_vec()
    };
    bytes.retain(|byte| *byte != b'{');
    bytes = strip_markup(&bytes);

    let first = bytes
        .iter()
        .position(|byte| !is_cpp_space(*byte))
        .unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|byte| !is_cpp_space(*byte))
        .map_or(first, |index| index + 1);
    bytes = bytes[first..end].to_vec();
    if bytes.is_empty() && !allow_empty {
        bytes.extend_from_slice(b"Unknown");
    }
    bytes.truncate(30);
    LegacyCString::from_bytes(bytes).expect("validated bytes came from a NUL-free LegacyCString")
}

fn is_cpp_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r')
}

/// Reachable `CMarkup::StripMarkup` behavior after validation removed every
/// opening brace (`src/StdMarkup.cpp:131-164`).
fn strip_markup(bytes: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(bytes.len());
    let mut offset = 0;
    while offset < bytes.len() {
        while let Some(length) = markup_tag_length(&bytes[offset..]) {
            offset += length;
        }
        if offset >= bytes.len() {
            break;
        }
        if bytes[offset..].starts_with(b"}}") {
            offset += 2;
            continue;
        }
        output.push(bytes[offset]);
        offset += 1;
    }
    output
}

fn markup_tag_length(bytes: &[u8]) -> Option<usize> {
    if bytes.first() != Some(&b'<') {
        return None;
    }
    let close = bytes.get(1..)?.iter().position(|byte| *byte == b'>')? + 1;
    let tag_len = (close - 1).min(49);
    let tag = bytes.get(1..1 + tag_len)?;
    let space = tag.iter().position(|byte| *byte == b' ');
    let valid = if tag.first() == Some(&b'/') {
        space.is_none()
    } else if tag == b"i" {
        true
    } else if tag.starts_with(b"c ") {
        tag[2..].len() <= 8
    } else {
        false
    };
    valid.then_some(tag_len + 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_empty_and_allow_empty_follow_cpp_name_validation() {
        assert_eq!(
            validate_name_no_empty(LegacyCString::default()).as_bytes(),
            b"empty"
        );
        assert!(validate_name_allow_empty(LegacyCString::default()).is_empty());
        let dirty = LegacyCString::from_bytes(b" {<i>Alice</i>{ ".to_vec()).unwrap();
        assert_eq!(validate_name_no_empty(dirty).as_bytes(), b"Alice");
        let vertical_tab = LegacyCString::from_bytes(b"\x0bAlice\x0b".to_vec()).unwrap();
        assert_eq!(
            validate_name_no_empty(vertical_tab).as_bytes(),
            b"Alice"
        );
    }

    #[test]
    fn overlong_closing_markup_uses_cpp_truncated_tag_cursor() {
        // SCopyEnclosed truncates the parsed tag to 49 bytes and CMarkup::Read
        // advances by that truncated length, not to the actual '>'
        // (src/C4Strings.cpp:425-432; src/StdMarkup.cpp:36-105).
        let mut value = b"</".to_vec();
        value.extend(std::iter::repeat_n(b'x', 49));
        value.extend_from_slice(b">Alice");
        let value = LegacyCString::from_bytes(value).unwrap();

        assert_eq!(validate_name_no_empty(value).as_bytes(), b">Alice");
    }
}
