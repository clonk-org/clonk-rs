//! Parser for `C4Game::DrawTextSpecImage` image specifications.
//!
//! The parser deliberately handles syntax only. Definition and portrait
//! availability remain the caller's responsibility, just as the C++ routine
//! parses a specification before consulting `Game.Defs`.

/// A parsed image specification accepted by `C4Game::DrawTextSpecImage`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextSpec<'a> {
    /// A definition picture. Bare IDs select phase zero.
    Definition { id: &'a str, phase: i32 },
    /// A named portrait. `color == None` means retain the caller's fallback
    /// color because no usable `%x` conversion was present.
    Portrait {
        definition_id: &'a str,
        portrait_name: &'a str,
        color: Option<u32>,
    },
    /// A built-in GUI or settlement icon.
    Icon(TextSpecIcon),
}

/// Built-in `Ico:*` specifications recognized by the C++ engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextSpecIcon {
    Locked,
    League,
    GameRunning,
    Lobby,
    RuntimeJoin,
    FairCrew,
    Settlement,
}

/// Parses the byte-oriented grammar from `C4Game::DrawTextSpecImage`
/// (`src/C4Game.cpp`) and `C4Portrait::EvaluatePortraitString`
/// (`src/C4DefGraphics.cpp`).
///
/// The C++ `SEqual2` icon checks are prefix checks, so trailing text after a
/// known `Ico:*` token is intentionally accepted. Portrait names are kept in
/// their original case; consumers must perform the C++ case-insensitive
/// graphics lookup.
pub fn parse_text_spec(spec: &str) -> Option<TextSpec<'_>> {
    if looks_like_id(spec) {
        return Some(TextSpec::Definition { id: spec, phase: 0 });
    }

    let bytes = spec.as_bytes();
    if bytes.len() > 5 && bytes.get(4) == Some(&b':') {
        if let (Some(id), Some(index_text)) = (spec.get(..4), spec.get(5..)) {
            if looks_like_id(id) {
                if let Some(phase) = parse_signed_decimal_prefix(index_text.as_bytes()) {
                    if phase >= 0 {
                        return Some(TextSpec::Definition { id, phase });
                    }
                }
            }
        }
    }

    if let Some(portrait) = spec.strip_prefix("Portrait:") {
        return parse_portrait_spec(portrait);
    }

    const ICONS: [(&str, TextSpecIcon); 7] = [
        ("Ico:Locked", TextSpecIcon::Locked),
        ("Ico:League", TextSpecIcon::League),
        ("Ico:GameRunning", TextSpecIcon::GameRunning),
        ("Ico:Lobby", TextSpecIcon::Lobby),
        ("Ico:RuntimeJoin", TextSpecIcon::RuntimeJoin),
        ("Ico:FairCrew", TextSpecIcon::FairCrew),
        ("Ico:Settlement", TextSpecIcon::Settlement),
    ];
    ICONS
        .into_iter()
        .find_map(|(prefix, icon)| spec.starts_with(prefix).then_some(TextSpec::Icon(icon)))
}

fn looks_like_id(id: &str) -> bool {
    id.len() == 4
        && id
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
}

fn parse_portrait_spec(spec: &str) -> Option<TextSpec<'_>> {
    let bytes = spec.as_bytes();
    if bytes.len() <= 6 || bytes.get(4..6) != Some(b"::") {
        return None;
    }
    let definition_id = spec.get(..4)?;
    let tail = spec.get(6..)?;
    let (color, portrait_name) = match tail.find("::") {
        Some(delimiter) => {
            let color_bytes = &tail.as_bytes()[..delimiter.min(6)];
            (
                parse_unsigned_hex_prefix(color_bytes),
                tail.get(delimiter + 2..)?,
            )
        }
        None => (None, tail),
    };
    if portrait_name.is_empty() {
        return None;
    }
    Some(TextSpec::Portrait {
        definition_id,
        portrait_name,
        color,
    })
}

/// The useful subset of `sscanf(text, "%d", &value)`: leading ASCII
/// whitespace, an optional sign, a nonempty decimal prefix and arbitrary
/// trailing text. Overflow is rejected; overflowing a C `int` is undefined
/// and no shipped specification relies on it.
fn parse_signed_decimal_prefix(bytes: &[u8]) -> Option<i32> {
    let mut index = bytes
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    let negative = match bytes.get(index) {
        Some(b'+') => {
            index += 1;
            false
        }
        Some(b'-') => {
            index += 1;
            true
        }
        _ => false,
    };
    let start = index;
    let mut magnitude = 0_i64;
    while let Some(byte @ b'0'..=b'9') = bytes.get(index) {
        magnitude = magnitude
            .checked_mul(10)?
            .checked_add(i64::from(*byte - b'0'))?;
        index += 1;
    }
    if index == start {
        return None;
    }
    let value = if negative { -magnitude } else { magnitude };
    i32::try_from(value).ok()
}

/// The useful subset of `sscanf(text, "%x", &value)`. The caller has
/// already applied C++'s six-byte temporary-buffer limit.
fn parse_unsigned_hex_prefix(bytes: &[u8]) -> Option<u32> {
    let mut index = bytes
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    let negative = match bytes.get(index) {
        Some(b'+') => {
            index += 1;
            false
        }
        Some(b'-') => {
            index += 1;
            true
        }
        _ => false,
    };
    if bytes.get(index) == Some(&b'0') && matches!(bytes.get(index + 1), Some(b'x' | b'X')) {
        index += 2;
    }
    let start = index;
    let mut value = 0_u32;
    while let Some(digit) = bytes.get(index).and_then(|byte| match byte {
        b'0'..=b'9' => Some(u32::from(*byte - b'0')),
        b'a'..=b'f' => Some(u32::from(*byte - b'a') + 10),
        b'A'..=b'F' => Some(u32::from(*byte - b'A') + 10),
        _ => None,
    }) {
        value = value.checked_mul(16)?.checked_add(digit)?;
        index += 1;
    }
    if index == start {
        return None;
    }
    Some(if negative {
        0_u32.wrapping_sub(value)
    } else {
        value
    })
}

#[cfg(test)]
mod tests {
    use super::{parse_text_spec, TextSpec, TextSpecIcon};

    #[test]
    fn definition_ids_match_cpp_case_and_underscore_rules() {
        assert_eq!(
            parse_text_spec("AB_D"),
            Some(TextSpec::Definition {
                id: "AB_D",
                phase: 0,
            })
        );
        assert_eq!(
            parse_text_spec("1234"),
            Some(TextSpec::Definition {
                id: "1234",
                phase: 0,
            })
        );
        for invalid in ["ab_D", "Ab_D", "ABC", "ABCDE", "AB-D", "ÄBCD"] {
            assert_eq!(parse_text_spec(invalid), None, "{invalid}");
        }
    }

    #[test]
    fn indexed_definitions_use_scanf_decimal_prefix_rules() {
        for (spec, phase) in [
            ("AB_D:12tail", 12),
            ("AB_D:  +12 rest", 12),
            ("AB_D:0x10", 0),
            ("AB_D:-0tail", 0),
        ] {
            assert_eq!(
                parse_text_spec(spec),
                Some(TextSpec::Definition { id: "AB_D", phase }),
                "{spec}"
            );
        }
        for invalid in [
            "ab_D:1",
            "AB_D:-1",
            "AB_D:",
            "AB_D:+tail",
            "AB_D:   tail",
            "AB-D:1",
        ] {
            assert_eq!(parse_text_spec(invalid), None, "{invalid}");
        }
    }

    #[test]
    fn portraits_keep_raw_ids_names_and_scanf_hex_fallback() {
        assert_eq!(
            parse_text_spec("Portrait:_TLK::1"),
            Some(TextSpec::Portrait {
                definition_id: "_TLK",
                portrait_name: "1",
                color: None,
            })
        );
        assert_eq!(
            parse_text_spec("Portrait:cowb::abcdef::captain1"),
            Some(TextSpec::Portrait {
                definition_id: "cowb",
                portrait_name: "captain1",
                color: Some(0xabcdef),
            })
        );
        assert_eq!(
            parse_text_spec("Portrait:COWB::abcdef12::1"),
            Some(TextSpec::Portrait {
                definition_id: "COWB",
                portrait_name: "1",
                color: Some(0xabcdef),
            })
        );
        assert_eq!(
            parse_text_spec("Portrait:COWB::12zz::1"),
            Some(TextSpec::Portrait {
                definition_id: "COWB",
                portrait_name: "1",
                color: Some(0x12),
            })
        );
        for spec in [
            "Portrait:COWB::1",
            "Portrait:COWB::::1",
            "Portrait:COWB::zz::1",
        ] {
            assert!(matches!(
                parse_text_spec(spec),
                Some(TextSpec::Portrait {
                    definition_id: "COWB",
                    portrait_name: "1",
                    color: None,
                })
            ));
        }
        for invalid in [
            " Portrait:COWB::1",
            "portrait:COWB::1",
            "Portrait:COWB:1",
            "Portrait:COWB::",
        ] {
            assert_eq!(parse_text_spec(invalid), None, "{invalid}");
        }
    }

    #[test]
    fn icons_are_case_sensitive_prefix_matches() {
        for (token, icon) in [
            ("Ico:Locked", TextSpecIcon::Locked),
            ("Ico:League", TextSpecIcon::League),
            ("Ico:GameRunning", TextSpecIcon::GameRunning),
            ("Ico:Lobby", TextSpecIcon::Lobby),
            ("Ico:RuntimeJoin", TextSpecIcon::RuntimeJoin),
            ("Ico:FairCrew", TextSpecIcon::FairCrew),
            ("Ico:Settlement", TextSpecIcon::Settlement),
        ] {
            assert_eq!(parse_text_spec(token), Some(TextSpec::Icon(icon)));
            assert_eq!(
                parse_text_spec(&format!("{token} trailing text")),
                Some(TextSpec::Icon(icon))
            );
        }
        assert_eq!(parse_text_spec("ico:League"), None);
        assert_eq!(parse_text_spec("Ico:Unknown"), None);
    }
}
