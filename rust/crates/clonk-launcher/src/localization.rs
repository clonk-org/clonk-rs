use anyhow::{anyhow, Context, Result};
use clonk_core::std_config::Config;
use clonk_platform::AppPaths;
use clonk_resources::{Group, GroupError};
use std::collections::{HashMap, HashSet};
use std::io;
use std::io::Cursor;
use std::sync::Arc;

const DEFAULT_LANGUAGE_CODE: &str = "US";

#[derive(Clone, Debug)]
pub struct Localization {
    active_language: String,
    fallback_language: String,
    translations: Arc<HashMap<String, String>>,
}

impl Localization {
    pub fn text<'a>(&'a self, key: &'a str) -> &'a str {
        self.translations
            .get(key)
            .map(|value| value.as_str())
            .unwrap_or(key)
    }

    pub fn format<'a, I>(&self, key: &str, replacements: I) -> String
    where
        I: IntoIterator<Item = (&'a str, &'a str)>,
    {
        let mut value = self.text(key).to_string();
        for (placeholder, replacement) in replacements {
            let token = format!("{{{placeholder}}}");
            value = value.replace(&token, replacement);
        }
        value
    }

    pub fn active_language(&self) -> &str {
        &self.active_language
    }

    pub fn fallback_language(&self) -> &str {
        &self.fallback_language
    }
}

pub fn load_localization(paths: &AppPaths) -> Result<Localization> {
    let system_group_path = paths.system_group_path();
    let system_group = Group::open(system_group_path).with_context(|| {
        format!(
            "failed to open system group at {}",
            system_group_path.display()
        )
    })?;

    let config = load_config(paths);
    let language_sequence = discover_language_codes(config.as_ref());

    let mut translations = HashMap::new();
    let mut fallback_language = None;
    let mut active_language = None;

    if let Some(config) = load_language_table(&system_group, DEFAULT_LANGUAGE_CODE)? {
        merge_translations(&mut translations, &config, true);
        fallback_language = Some(DEFAULT_LANGUAGE_CODE.to_string());
    }

    for code in language_sequence {
        if Some(code.as_str()) == fallback_language.as_deref() {
            active_language = Some(code);
            break;
        }
        match load_language_table(&system_group, &code)? {
            Some(config) => {
                merge_translations(&mut translations, &config, true);
                if fallback_language.is_none() {
                    fallback_language = Some(code.clone());
                }
                active_language = Some(code);
                break;
            }
            None => continue,
        }
    }

    if active_language.is_none() {
        active_language = fallback_language.clone();
    }

    let Some(fallback_language) = fallback_language.or_else(|| {
        active_language.clone().or_else(|| {
            if translations.is_empty() {
                None
            } else {
                Some(DEFAULT_LANGUAGE_CODE.to_string())
            }
        })
    }) else {
        return Err(anyhow!(
            "no language packs could be loaded from system group"
        ));
    };

    let active_language = active_language.unwrap_or_else(|| fallback_language.clone());

    if translations.is_empty() {
        return Err(anyhow!(
            "no language translations were populated for active language {active_language}"
        ));
    }

    Ok(Localization {
        active_language,
        fallback_language,
        translations: Arc::new(translations),
    })
}

fn load_config(paths: &AppPaths) -> Option<Config> {
    match Config::load(paths.config_file()) {
        Ok(config) => Some(config),
        Err(err) if err.kind() == io::ErrorKind::NotFound => None,
        Err(_) => None,
    }
}

fn discover_language_codes(config: Option<&Config>) -> Vec<String> {
    let mut codes = Vec::new();
    let mut seen = HashSet::new();

    if let Some(config) = config {
        if let Some(sequence) = config
            .get_in(Some("General"), "LanguageEx")
            .or_else(|| config.get("LanguageEx"))
        {
            append_language_codes(&mut codes, sequence);
        }

        if codes.is_empty() {
            if let Some(primary) = config
                .get_in(Some("General"), "Language")
                .or_else(|| config.get("Language"))
            {
                if let Some(code) = parse_language_code(primary) {
                    if seen.insert(code.clone()) {
                        codes.push(code);
                    }
                }
            }
        }
    }

    if codes.is_empty() {
        if let Some(code) = environment_language_code() {
            if seen.insert(code.clone()) {
                codes.push(code);
            }
        }
    }

    codes
}

fn append_language_codes(codes: &mut Vec<String>, sequence: &str) {
    for segment in sequence.split(',') {
        if let Some(code) = parse_language_code(segment) {
            codes.push(code);
        }
    }
}

fn parse_language_code(segment: &str) -> Option<String> {
    // C++ IsWhiteSpace intentionally recognizes only these four bytes
    // (C4Strings.cpp:48-55); notably, form feed remains part of the prefix.
    let trimmed = segment.trim_start_matches([' ', '\t', '\r', '\n']);
    let prefix = &trimmed.as_bytes()[..trimmed.len().min(2)];
    (!prefix.is_empty()).then(|| String::from_utf8_lossy(prefix).into_owned())
}

fn parse_environment_language_code(segment: &str) -> Option<String> {
    let mut code = String::new();
    for ch in segment.chars() {
        if ch.is_ascii_alphabetic() {
            code.push(ch.to_ascii_uppercase());
            if code.len() == 2 {
                break;
            }
        }
    }
    if code.len() == 2 {
        Some(code)
    } else {
        None
    }
}

fn environment_language_code() -> Option<String> {
    for key in ["LC_LANGUAGE", "LC_ALL", "LANG"] {
        if let Ok(value) = std::env::var(key) {
            if let Some(code) = parse_environment_language_code(&value) {
                return Some(code);
            }
        }
    }
    None
}

fn load_language_table(group: &Group, code: &str) -> Result<Option<Config>> {
    let filename = format!("Language{code}.txt");
    let data = match group.read_file(&filename) {
        Ok(bytes) => bytes,
        Err(GroupError::EntryNotFound(_)) => return Ok(None),
        Err(err) => return Err(err.into()),
    };

    let content = decode_latin1(&data);
    let mut cursor = Cursor::new(content);
    let mut config = Config::from_reader(&mut cursor).with_context(|| {
        format!("failed to parse language pack {filename} as configuration table")
    })?;
    normalize_language_table_newlines(&mut config);
    Ok(Some(config))
}

fn normalize_language_table_newlines(config: &mut Config) {
    let replacements: Vec<_> = config
        .iter()
        .filter(|entry| entry.value.contains("\\n"))
        .map(|entry| {
            (
                entry.section.clone(),
                entry.key.clone(),
                entry.value.replace("\\n", "\r\n"),
            )
        })
        .collect();
    for (section, key, value) in replacements {
        config.set_in(section.as_deref(), key, value);
    }
}

fn decode_latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&byte| byte as char).collect()
}

fn merge_translations(
    target: &mut HashMap<String, String>,
    config: &Config,
    override_existing: bool,
) {
    for entry in config.iter() {
        if override_existing || !target.contains_key(&entry.key) {
            target.insert(entry.key.clone(), entry.value.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_language_sequence_copies_two_char_prefixes_verbatim() {
        // C4ConfigGeneral::GetLanguageSequence uses SCopySegment with a
        // two-byte limit and leading-whitespace skipping; it neither filters
        // nor uppercases the copied prefix (src/C4Config.cpp:1492-1506;
        // src/C4Strings.cpp:185-203).
        let mut config = Config::new();
        config.set_in(Some("General"), "LanguageEx", " de - Deutsch, x, --invalid");

        assert_eq!(
            discover_language_codes(Some(&config)),
            vec!["de".to_string(), "x".to_string(), "--".to_string()]
        );
    }

    #[test]
    fn configured_language_sequence_preserves_duplicate_order() {
        // GetLanguageSequence appends every nonempty segment without a
        // uniqueness check (src/C4Config.cpp:1497-1505).
        let mut config = Config::new();
        config.set_in(Some("General"), "LanguageEx", "DE,US,DE,US");

        assert_eq!(
            discover_language_codes(Some(&config)),
            vec![
                "DE".to_string(),
                "US".to_string(),
                "DE".to_string(),
                "US".to_string(),
            ]
        );
    }

    #[test]
    fn configured_language_sequence_skips_only_cpp_whitespace() {
        // IsWhiteSpace/SAdvanceSpace skip space, tab, CR, and LF only
        // (src/C4Strings.cpp:48-55,334-339). Vertical tab and form feed are
        // copied as part of C++'s two-byte language prefix.
        let mut config = Config::new();
        config.set_in(
            Some("General"),
            "LanguageEx",
            "\u{000b}DE,\u{000c}US, \t\r\nFR",
        );

        assert_eq!(
            discover_language_codes(Some(&config)),
            vec!["\u{000b}D".to_string(), "\u{000c}U".to_string(), "FR".to_string()]
        );
    }

    #[test]
    fn language_table_converts_escaped_newlines_to_crlf() {
        // C4ResStrTable replaces each literal "\\n" in a recognized value
        // with CRLF while loading the selected table
        // (src/C4ResStrTable.cpp:25-50).
        let directory = tempfile::Builder::new()
            .prefix("lc-test-")
            .tempdir()
            .unwrap();
        std::fs::write(
            directory.path().join("LanguageZZ.txt"),
            b"IDS_LANG_NAME=First\\nSecond\n",
        )
        .unwrap();
        let group = Group::open(directory.path()).unwrap();

        let table = load_language_table(&group, "ZZ")
            .unwrap()
            .expect("language table exists");
        assert_eq!(table.get("IDS_LANG_NAME"), Some("First\r\nSecond"));
    }
}
