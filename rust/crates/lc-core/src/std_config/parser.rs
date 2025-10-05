use crate::std_markup::Markup;
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
    let mut value = raw_value.trim();

    if let Some(unquoted) = strip_quotes(value) {
        value = unquoted;
    }

    let mut key_owned = key.to_string();
    let mut value_owned = value.to_string();
    unescape_comment_markers(&mut key_owned);
    unescape_comment_markers(&mut value_owned);
    Markup::strip_markup(&mut key_owned);
    Markup::strip_markup(&mut value_owned);

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
    let mut chars = line.char_indices().peekable();
    let mut in_quotes = false;
    while let Some((idx, ch)) = chars.next() {
        match ch {
            '"' => in_quotes = !in_quotes,
            '#' if !in_quotes => return (&line[..idx], Some(&line[idx + 1..])),
            '/' if !in_quotes => {
                if let Some((next_idx, '/')) = chars.peek() {
                    return (&line[..idx], Some(&line[*next_idx + 1..]));
                }
            }
            _ => {}
        }
    }
    (line, None)
}

fn split_key_value(line: &str) -> Option<(&str, &str)> {
    let mut chars = line.char_indices().peekable();
    let mut in_quotes = false;
    while let Some((idx, ch)) = chars.next() {
        match ch {
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
