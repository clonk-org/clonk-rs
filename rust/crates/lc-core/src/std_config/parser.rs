use std::borrow::Cow;

pub(crate) enum ParsedItem<'a> {
    Section {
        name: Cow<'a, str>,
        commented: bool,
    },
    Entry {
        key: Cow<'a, str>,
        value: Cow<'a, str>,
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

    let (content, comment) = split_comment(trimmed);
    let (raw_key, raw_value) = split_key_value(content)?;
    let key = raw_key.trim();
    let value = raw_value.trim();

    let mut key_owned = key.to_string();
    let mut value_owned = strip_quotes(value)
        .map(decode_escaped_value)
        .unwrap_or_else(|| value.to_string());
    unescape_comment_markers(&mut key_owned);
    unescape_comment_markers(&mut value_owned);

    let comment = comment.and_then(|c| {
        let trimmed = c.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });

    Some(ParsedItem::Entry {
        key: Cow::Owned(key_owned),
        value: Cow::Owned(value_owned),
        comment,
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

fn split_comment(line: &str) -> (&str, Option<&str>) {
    let chars = line.char_indices();
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
            '#' if !in_quotes && comment_marker_is_separated(line, idx) => {
                return (&line[..idx], Some(&line[idx + 1..]));
            }
            // C++ treats `//` only as a whole-line comment. Inside a value it
            // is ordinary data (notably in unquoted `https://` URLs).
            '/' if !in_quotes => {}
            _ => {}
        }
    }
    (line, None)
}

fn comment_marker_is_separated(line: &str, index: usize) -> bool {
    index == 0
        || line[..index]
            .chars()
            .next_back()
            .is_some_and(char::is_whitespace)
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

fn strip_quotes(value: &str) -> Option<&str> {
    if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
        Some(&value[1..value.len() - 1])
    } else {
        None
    }
}

fn decode_escaped_value(value: &str) -> String {
    let mut decoded = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(character) = chars.next() {
        if character != '\\' {
            decoded.push(character);
            continue;
        }
        let Some(escaped) = chars.next() else {
            decoded.push('\\');
            break;
        };
        let decoded_character = match escaped {
            'a' => '\u{7}',
            'b' => '\u{8}',
            'f' => '\u{c}',
            'n' => '\n',
            'r' => '\r',
            't' => '\t',
            'v' => '\u{b}',
            '\'' => '\'',
            '"' => '"',
            '\\' => '\\',
            '?' => '?',
            'x' => {
                let mut number = 0_u32;
                let mut found = false;
                while let Some(digit) = chars.peek().and_then(|digit| digit.to_digit(16)) {
                    found = true;
                    number = number.wrapping_mul(16).wrapping_add(digit);
                    chars.next();
                }
                if found {
                    char::from_u32(number & 0xff).unwrap_or('\u{fffd}')
                } else {
                    'x'
                }
            }
            first @ '0'..='7' => {
                let mut number = first.to_digit(8).expect("matched octal digit");
                while let Some(digit) = chars.peek().and_then(|digit| digit.to_digit(8)) {
                    number = number.wrapping_mul(8).wrapping_add(digit);
                    chars.next();
                }
                char::from_u32(number & 0xff).unwrap_or('\u{fffd}')
            }
            other => other,
        };
        decoded.push(decoded_character);
    }
    decoded
}

fn unescape_comment_markers(s: &mut String) {
    if s.contains("\\#") {
        *s = s.replace("\\#", "#");
    }
    if s.contains("\\//") {
        *s = s.replace("\\//", "//");
    }
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
    fn parse_with_comment() {
        match parse_line("Name = Player # comment").unwrap() {
            ParsedItem::Entry {
                key,
                value,
                comment,
            } => {
                assert_eq!(key, "Name");
                assert_eq!(value, "Player");
                assert_eq!(comment.as_deref(), Some("comment"));
            }
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
