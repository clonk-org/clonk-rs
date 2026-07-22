use std::borrow::Cow;

pub(crate) enum ParsedItem<'a> {
    Section {
        name: Cow<'a, str>,
        commented: bool,
    },
    Entry {
        key: Cow<'a, str>,
        value: Cow<'a, str>,
        escaped_bytes: Option<Vec<u8>>,
        comment: Option<String>,
    },
}

pub(crate) fn parse_line(line: &str) -> Option<ParsedItem<'_>> {
    let mut trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut commented_section = false;

    if let Some(after_hash) = trimmed.strip_prefix('#') {
        let rest = after_hash.trim_start();
        if rest.starts_with('[') {
            trimmed = rest;
            commented_section = true;
        } else {
            return None;
        }
    } else if trimmed.starts_with("//") {
        return None;
    }

    if trimmed.starts_with('[') {
        return parse_section(trimmed, commented_section).map(|name| ParsedItem::Section {
            name,
            commented: commented_section,
        });
    }

    let (raw_key, raw_value) = split_key_value(trimmed)?;
    let key = raw_key.trim();
    let value = raw_value.trim();

    let (value, escaped_bytes) =
        if let Some(decoded) = super::decode_cpp_escaped_string(value.as_bytes(), usize::MAX) {
            (
                Cow::Owned(decoded_config_bytes_to_string(&decoded)),
                Some(decoded),
            )
        } else {
            (Cow::Borrowed(value), None)
        };

    Some(ParsedItem::Entry {
        key: Cow::Borrowed(key),
        value,
        escaped_bytes,
        comment: None,
    })
}

fn parse_section(line: &str, commented: bool) -> Option<Cow<'_, str>> {
    if !line.starts_with('[') {
        return None;
    }
    let end = line.find(']')?;
    let name = line[1..end].trim();
    if name.is_empty() {
        return None;
    }
    if commented && name.starts_with('#') {
        return None;
    }
    Some(Cow::Owned(name.to_string()))
}

fn split_key_value(line: &str) -> Option<(&str, &str)> {
    let chars = line.char_indices().peekable();
    let mut in_quotes = false;
    let mut escaped = false;
    for (idx, ch) in chars {
        if in_quotes && escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if in_quotes => escaped = true,
            '"' => in_quotes = !in_quotes,
            '=' if !in_quotes => {
                let key = &line[..idx];
                let value = &line[idx + 1..];
                return Some((key, value));
            }
            _ => {}
        }
    }
    None
}

fn decoded_config_bytes_to_string(mut remaining: &[u8]) -> String {
    let mut decoded = String::with_capacity(remaining.len());
    while !remaining.is_empty() {
        match std::str::from_utf8(remaining) {
            Ok(valid) => {
                decoded.push_str(valid);
                break;
            }
            Err(error) => {
                let valid_up_to = error.valid_up_to();
                let valid = std::str::from_utf8(&remaining[..valid_up_to])
                    .expect("Utf8Error valid prefix is UTF-8");
                decoded.push_str(valid);
                let invalid = &remaining[valid_up_to..];
                let invalid_length = error.error_len().unwrap_or(invalid.len());
                decoded.extend(invalid[..invalid_length].iter().copied().map(char::from));
                remaining = &invalid[invalid_length..];
            }
        }
    }
    decoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_line() {
        match parse_line("Name = Player").unwrap() {
            ParsedItem::Entry { key, value, .. } => {
                assert_eq!(key, "Name");
                assert_eq!(value, "Player");
            }
            ParsedItem::Section { .. } => panic!("expected entry"),
        }
    }

    #[test]
    fn l024_inline_hash_is_value_data() {
        match parse_line("Name = A # B").unwrap() {
            ParsedItem::Entry {
                key,
                value,
                comment,
                ..
            } => {
                assert_eq!(key, "Name");
                assert_eq!(value, "A # B");
                assert_eq!(comment, None);
            }
            ParsedItem::Section { .. } => panic!("expected entry"),
        }
    }

    #[test]
    fn l024_unquoted_comment_marker_escapes_remain_verbatim() {
        match parse_line(r"Value=A\#B\//C").unwrap() {
            ParsedItem::Entry { value, .. } => assert_eq!(value, r"A\#B\//C"),
            ParsedItem::Section { .. } => panic!("expected entry"),
        }
    }

    #[test]
    fn parse_quoted_value() {
        match parse_line("Path = \"C:/Games\" ").unwrap() {
            ParsedItem::Entry { value, .. } => assert_eq!(value, "C:/Games"),
            ParsedItem::Section { .. } => panic!("expected entry"),
        }
    }

    #[test]
    fn l010_cpp_escaped_utf8_backslash_and_quote_decode_bytewise() {
        let line = r#"Comment="M\303\274ller\\path\"quoted\"" trailing data"#;
        match parse_line(line).unwrap() {
            ParsedItem::Entry { value, .. } => {
                assert_eq!(value, "Müller\\path\"quoted\"");
            }
            ParsedItem::Section { .. } => panic!("expected entry"),
        }
    }

    #[test]
    fn l010_quoted_latin1_fallback_does_not_reprocess_comment_escapes() {
        match parse_line("Value=\"\\374\\\\#\"").unwrap() {
            ParsedItem::Entry { value, .. } => assert_eq!(value, "ü\\#"),
            ParsedItem::Section { .. } => panic!("expected entry"),
        }
    }

    #[test]
    fn parse_unquoted_url_without_treating_scheme_as_comment() {
        match parse_line("ServerAddress = https://league.clonkspot.org/").unwrap() {
            ParsedItem::Entry { value, comment, .. } => {
                assert_eq!(value, "https://league.clonkspot.org/");
                assert_eq!(comment, None);
            }
            ParsedItem::Section { .. } => panic!("expected entry"),
        }
    }

    #[test]
    fn respect_equals_in_quotes() {
        match parse_line("Key=\"value=with=equals\"").unwrap() {
            ParsedItem::Entry { value, .. } => assert_eq!(value, "value=with=equals"),
            ParsedItem::Section { .. } => panic!("expected entry"),
        }
    }

    #[test]
    fn skip_commented_line() {
        assert!(parse_line("# comment").is_none());
        assert!(parse_line("// comment").is_none());
    }

    #[test]
    fn parse_section_line() {
        match parse_line("[Graphics]").unwrap() {
            ParsedItem::Section { name, commented } => {
                assert_eq!(name, "Graphics");
                assert!(!commented);
            }
            ParsedItem::Entry { .. } => panic!("expected section"),
        }
    }

    #[test]
    fn parse_commented_section_line() {
        match parse_line("#[Graphics]").unwrap() {
            ParsedItem::Section { name, commented } => {
                assert_eq!(name, "Graphics");
                assert!(commented);
            }
            ParsedItem::Entry { .. } => panic!("expected section"),
        }
    }
}
