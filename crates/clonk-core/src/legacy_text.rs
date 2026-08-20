use std::borrow::Cow;

const WINDOWS_1252_EXTRA: [char; 32] = [
    '€', '?', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '?', 'Ž', '?', '?', '‘', '’',
    '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '?', 'ž', 'Ÿ',
];

/// Preserves UTF-8 and projects invalid native text from Windows-1252.
pub fn ensure_utf8(bytes: &[u8]) -> Cow<'_, str> {
    std::str::from_utf8(bytes).map_or_else(
        |_| {
            Cow::Owned(
                bytes
                    .iter()
                    .map(|byte| match *byte {
                        0x00..=0x7f | 0xa0..=0xff => char::from(*byte),
                        byte => WINDOWS_1252_EXTRA[usize::from(byte - 0x80)],
                    })
                    .collect(),
            )
        },
        Cow::Borrowed,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_utf8_is_preserved_without_allocation() {
        let text = "Grüße";

        assert!(matches!(
            ensure_utf8(text.as_bytes()),
            Cow::Borrowed(borrowed) if borrowed == text
        ));
    }

    #[test]
    fn invalid_utf8_is_projected_like_std_str_buf() {
        // StdStrBuf::EnsureUnicode converts the complete buffer as Windows-1252,
        // including its question-mark mappings (src/StdBuf.cpp:227-302).
        assert_eq!(ensure_utf8(&[b'A', 0x80, 0x81, 0xa0]), "A€?\u{a0}");
    }
}
