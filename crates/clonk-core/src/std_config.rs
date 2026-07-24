mod parser;

use indexmap::IndexMap;
use parser::{parse_line, ParsedItem};
use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;

/// Decodes one complete C++ `RCT_Escaped` value into its native bytes.
///
/// `StdCompilerINIRead` skips horizontal whitespace, requires an opening
/// quote, consumes the C/C++ escape set bytewise, and stops at the first
/// unescaped closing quote or raw line ending. Numeric escapes deliberately
/// consume every following digit of their radix rather than using C's usual
/// length limits.
pub fn decode_cpp_escaped_string(value: &[u8], max_length: usize) -> Option<Vec<u8>> {
    decode_cpp_escaped_string_impl(value, max_length)
}

fn decode_cpp_escaped_string_impl(value: &[u8], max_length: usize) -> Option<Vec<u8>> {
    let start = value
        .iter()
        .position(|byte| !matches!(byte, b' ' | b'\t'))
        .unwrap_or(value.len());
    let value = &value[start..];
    if value.first() != Some(&b'"') {
        return None;
    }

    let mut output = Vec::with_capacity(value.len().min(max_length));
    let mut index = 1;
    while index < value.len() && output.len() < max_length {
        let byte = value[index];
        if matches!(byte, 0 | b'"' | b'\n' | b'\r') {
            break;
        }
        if byte != b'\\' {
            output.push(byte);
            index += 1;
            continue;
        }

        index += 1;
        if index >= value.len() {
            break;
        }
        let escaped = value[index];
        let decoded = match escaped {
            b'a' => {
                index += 1;
                b'\x07'
            }
            b'b' => {
                index += 1;
                b'\x08'
            }
            b'f' => {
                index += 1;
                b'\x0c'
            }
            b'n' => {
                index += 1;
                b'\n'
            }
            b'r' => {
                index += 1;
                b'\r'
            }
            b't' => {
                index += 1;
                b'\t'
            }
            b'v' => {
                index += 1;
                b'\x0b'
            }
            b'\'' | b'"' | b'\\' | b'?' => {
                index += 1;
                escaped
            }
            b'x' => {
                index += 1;
                if index >= value.len() || !value[index].is_ascii_hexdigit() {
                    b'x'
                } else {
                    let mut code = 0_i32;
                    while index < value.len() && value[index].is_ascii_hexdigit() {
                        code = code
                            .wrapping_mul(16)
                            .wrapping_add(cpp_hex_digit(value[index]));
                        index += 1;
                    }
                    code as u8
                }
            }
            b'0'..=b'7' => {
                let mut code = 0_i32;
                while index < value.len() && matches!(value[index], b'0'..=b'7') {
                    code = code
                        .wrapping_mul(8)
                        .wrapping_add(i32::from(value[index] - b'0'));
                    index += 1;
                }
                code as u8
            }
            _ => {
                index += 1;
                escaped
            }
        };
        if decoded == 0 {
            break;
        }
        output.push(decoded);
    }
    Some(output)
}

fn cpp_hex_digit(byte: u8) -> i32 {
    if byte.is_ascii_digit() {
        i32::from(byte - b'0')
    } else {
        // Preserve StdCompilerINIRead's lowercase subtraction even when
        // isxdigit accepted an uppercase byte.
        i32::from(byte) - i32::from(b'a') + 10
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ValueFormat {
    Automatic,
    CppEscaped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub section: Option<String>,
    pub key: String,
    pub value: String,
    pub comment: Option<String>,
    value_format: ValueFormat,
    escaped_bytes: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Copy)]
struct SectionMeta {
    commented: bool,
}

#[derive(Debug, Clone)]
pub struct Config {
    entries: IndexMap<(Option<String>, String), Entry>,
    section_meta: IndexMap<Option<String>, SectionMeta>,
    standalone_comments: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        let mut cfg = Self {
            entries: IndexMap::new(),
            section_meta: IndexMap::new(),
            standalone_comments: Vec::new(),
        };
        cfg.ensure_section(None, false);
        cfg
    }
}

impl Config {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn load<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let file = File::open(path)?;
        let mut reader = BufReader::new(file);
        Self::from_reader(&mut reader)
    }

    pub fn from_reader<R: BufRead>(reader: &mut R) -> io::Result<Self> {
        fn handle_item(
            config: &mut Config,
            current_section: &mut Option<String>,
            item: ParsedItem<'_>,
        ) {
            match item {
                ParsedItem::Section { name, commented } => {
                    let owned = name.into_owned();
                    config.ensure_section(Some(owned.clone()), commented);
                    *current_section = Some(owned);
                }
                ParsedItem::Entry {
                    key,
                    value,
                    escaped_bytes,
                    comment,
                } => {
                    let section_clone = current_section.clone();
                    config.ensure_section(section_clone.clone(), false);
                    let key_owned = key.into_owned();
                    let value_format = if escaped_bytes.is_some()
                        || is_cpp_escaped_config_field(section_clone.as_deref(), &key_owned)
                    {
                        ValueFormat::CppEscaped
                    } else {
                        ValueFormat::Automatic
                    };
                    let entry = Entry {
                        section: section_clone.clone(),
                        key: key_owned.clone(),
                        value: value.into_owned(),
                        comment,
                        value_format,
                        escaped_bytes,
                    };
                    config
                        .entries
                        .entry((section_clone, key_owned))
                        .or_insert(entry);
                }
            }
        }

        let mut config = Config::new();
        let mut buffer = String::new();
        let mut current_section: Option<String> = None;

        loop {
            buffer.clear();
            let bytes_read = reader.read_line(&mut buffer)?;
            if bytes_read == 0 {
                break;
            }
            if let Some(comment) = standalone_comment(&buffer) {
                config.standalone_comments.push(comment);
            } else if let Some(item) = parse_line(&buffer) {
                handle_item(&mut config, &mut current_section, item);
            }
        }
        Ok(config)
    }

    pub fn save<P: AsRef<Path>>(&self, path: P) -> io::Result<()> {
        let mut file = File::create(path)?;
        self.write_to(&mut file)
    }

    pub fn to_string(&self) -> io::Result<String> {
        let mut buffer = Vec::new();
        self.write_to(&mut buffer)?;
        String::from_utf8(buffer)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.utf8_error()))
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.get_in(None, key)
    }

    pub fn get_in(&self, section: Option<&str>, key: &str) -> Option<&str> {
        self.entries
            .values()
            .find(|entry| entry.section.as_deref() == section && entry.key == key)
            .map(|entry| entry.value.as_str())
    }

    pub fn get_bool(&self, key: &str) -> Option<bool> {
        self.get(key)
            .map(|value| matches!(value.to_ascii_lowercase().as_str(), "true" | "1" | "yes"))
    }

    pub fn get_i32(&self, key: &str) -> Option<i32> {
        self.get(key)?.parse().ok()
    }

    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.set_in(None, key, value);
    }

    pub fn set_in(
        &mut self,
        section: Option<&str>,
        key: impl Into<String>,
        value: impl Into<String>,
    ) {
        let key = key.into();
        let value = value.into();
        let section_owned = section.map(String::from);
        self.ensure_section(section_owned.clone(), false);
        let map_key = (section_owned.clone(), key.clone());
        if let Some(existing) = self.entries.get_mut(&map_key) {
            // Updating a live config field must not discard the comment that
            // was attached to its INI node. The advanced editor writes only
            // changed values through this path while preserving all other
            // source metadata and vendor extensions.
            if existing.value != value {
                existing.escaped_bytes = None;
            }
            existing.value = value;
            return;
        }
        let value_format = if is_cpp_escaped_config_field(section, &key) {
            ValueFormat::CppEscaped
        } else {
            ValueFormat::Automatic
        };
        let entry = Entry {
            section: section_owned.clone(),
            key: key.clone(),
            value,
            comment: None,
            value_format,
            escaped_bytes: None,
        };
        let insertion_index = self
            .entries
            .values()
            .rposition(|existing| existing.section == section_owned)
            .map(|index| index + 1)
            .unwrap_or(self.entries.len());
        self.entries.shift_insert(insertion_index, map_key, entry);
    }

    pub fn iter(&self) -> impl Iterator<Item = &Entry> {
        self.entries.values()
    }

    #[allow(dead_code)]
    pub(crate) fn entry_map(&self) -> &IndexMap<(Option<String>, String), Entry> {
        &self.entries
    }

    pub fn set_section_commented(&mut self, section: &str, commented: bool) {
        let section_owned = Some(section.to_string());
        self.ensure_section(section_owned.clone(), commented);
        if let Some(meta) = self.section_meta.get_mut(&section_owned) {
            meta.commented = commented;
        }
    }

    fn ensure_section(&mut self, section: Option<String>, commented: bool) {
        if !self.section_meta.contains_key(&section) {
            self.section_meta.insert(section, SectionMeta { commented });
        } else if commented {
            if let Some(meta) = self.section_meta.get_mut(&section) {
                meta.commented = commented;
            }
        }
    }

    fn write_to<W>(&self, writer: &mut W) -> io::Result<()>
    where
        W: Write,
    {
        for comment in &self.standalone_comments {
            write!(writer, "{comment}\r\n")?;
        }
        let mut current_section: Option<&Option<String>> = None;
        for entry in self.entries.values() {
            let section_ref = &entry.section;
            if current_section != Some(section_ref) {
                if current_section.is_some() {
                    writer.write_all(b"\r\n")?;
                }
                if let Some(section_name) = section_ref.as_ref() {
                    let commented = self
                        .section_meta
                        .get(section_ref)
                        .map(|meta| meta.commented)
                        .unwrap_or(false);
                    if commented {
                        write!(writer, "#[{}]\r\n", section_name)?;
                    } else {
                        write!(writer, "[{}]\r\n", section_name)?;
                    }
                }
                current_section = Some(section_ref);
            }
            let value = serialized_value(entry);
            if let Some(comment) = &entry.comment {
                // Keep entry comments inline, which is the form the parser
                // can associate with the same name node on the next load.
                // Emitting a standalone `#...` line silently detached it
                // during an Options -> advanced editor save/reload cycle.
                write!(writer, "{}={} #{}\r\n", entry.key, value, comment)?;
            } else {
                write!(writer, "{}={}\r\n", entry.key, value)?;
            }
        }
        Ok(())
    }
}

fn standalone_comment(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.starts_with("//")
        || trimmed
            .strip_prefix('#')
            .is_some_and(|rest| !rest.trim_start().starts_with('['))
    {
        Some(trimmed.to_string())
    } else {
        None
    }
}

fn serialized_value(entry: &Entry) -> String {
    match entry.value_format {
        ValueFormat::Automatic => quote_value(&entry.value),
        ValueFormat::CppEscaped => cpp_escaped_value(
            entry
                .escaped_bytes
                .as_deref()
                .unwrap_or(entry.value.as_bytes()),
        ),
    }
}

fn quote_value(value: &str) -> String {
    if !value.is_empty()
        && !value.contains(|ch: char| {
            ch.is_whitespace()
                || ch.is_control()
                || ch == '#'
                || ch == '"'
                || ch == '\\'
                || !ch.is_ascii()
        })
    {
        return value.to_string();
    }
    cpp_escaped_value(value.as_bytes())
}

fn cpp_escaped_value(value: &[u8]) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    let mut last_numeric_escape = false;
    for &byte in value {
        let escape_digit = last_numeric_escape && byte.is_ascii_digit();
        last_numeric_escape = false;
        if !escape_digit {
            let named_escape = match byte {
                b'\x07' => Some('a'),
                b'\x08' => Some('b'),
                b'\x0c' => Some('f'),
                b'\n' => Some('n'),
                b'\r' => Some('r'),
                b'\t' => Some('t'),
                b'\x0b' => Some('v'),
                b'"' => Some('"'),
                b'\\' => Some('\\'),
                _ => None,
            };
            if let Some(escaped) = named_escape {
                quoted.push('\\');
                quoted.push(escaped);
                continue;
            }
            if (b' '..=b'~').contains(&byte) {
                quoted.push(char::from(byte));
                continue;
            }
        }
        quoted.push('\\');
        push_unpadded_octal(&mut quoted, byte);
        last_numeric_escape = true;
    }
    quoted.push('"');
    quoted
}

fn push_unpadded_octal(output: &mut String, byte: u8) {
    let high = byte >> 6;
    let middle = (byte >> 3) & 7;
    let low = byte & 7;
    if high != 0 {
        output.push(char::from(b'0' + high));
    }
    if high != 0 || middle != 0 {
        output.push(char::from(b'0' + middle));
    }
    output.push(char::from(b'0' + low));
}

fn is_cpp_escaped_config_field(section: Option<&str>, key: &str) -> bool {
    match section {
        Some("General") => matches!(
            key,
            "Name"
                | "Language"
                | "LanguageEx"
                | "LanguageCharset"
                | "Definitions"
                | "Participants"
                | "LogPath"
                | "PlayerPath"
                | "DefinitionPath"
                | "UserPath"
                | "SaveGameFolder"
                | "SaveDemoFolder"
                | "MissionAccess"
                | "ScreenshotFolder"
                | "FontName"
        ),
        Some("Network") => matches!(
            key,
            "WorkPath"
                | "Comment"
                | "LocalName"
                | "Nick"
                | "ServerAddress"
                | "AlternateServerAddress"
                | "UpdateServerAddress"
                | "LastPassword"
                | "PuncherAddress"
                | "LeagueNick"
        ),
        Some("IRC") => matches!(key, "Server2" | "Nick" | "RealName" | "Channel"),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use tempfile::tempdir;

    fn cpp_value<'a>(config: &'a str, section: &str, key: &str) -> Option<&'a str> {
        let header = format!("[{section}]");
        let assignment = format!("{key}=");
        let mut in_section = false;
        for line in config.split(['\r', '\n']).filter(|line| !line.is_empty()) {
            if line.starts_with('[') {
                if in_section {
                    return None;
                }
                in_section = line == header;
            } else if in_section {
                if let Some(value) = line.strip_prefix(&assignment) {
                    return Some(value);
                }
            }
        }
        None
    }

    fn cpp_boolean(config: &str, section: &str, key: &str) -> Option<bool> {
        let value = cpp_value(config, section, key)?.as_bytes();
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

    fn cpp_escaped(config: &str, section: &str, key: &str) -> Option<Vec<u8>> {
        decode_cpp_escaped_string(cpp_value(config, section, key)?.as_bytes(), usize::MAX)
    }

    #[test]
    fn l024_parse_basic_config_preserves_markup_and_inline_hash() {
        let data = b"Name = <i>Player</i> # main user\nEnabled=true\n\n";
        let mut cursor = Cursor::new(&data[..]);
        let cfg = Config::from_reader(&mut cursor).unwrap();
        assert_eq!(cfg.get("Name"), Some("<i>Player</i> # main user"));
        assert_eq!(cfg.get_bool("Enabled"), Some(true));
        let entry = cfg
            .entries
            .values()
            .find(|entry| entry.key == "Name")
            .unwrap();
        assert_eq!(entry.comment, None);
        assert!(cfg.get("Unknown").is_none());
    }

    #[test]
    fn set_iter_and_save() {
        let mut cfg = Config::new();
        cfg.set("Key", "Some Value");
        cfg.set_in(Some("Section"), "Answer", "42");
        cfg.set_section_commented("Commented", true);
        let dir = tempdir().unwrap();
        let path = dir.path().join("out.cfg");
        cfg.save(&path).unwrap();
        let reloaded = Config::load(&path).unwrap();
        assert_eq!(reloaded.get("Key"), Some("Some Value"));
        assert_eq!(reloaded.get_in(Some("Section"), "Answer"), Some("42"));
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("[Section]"));
    }

    #[test]
    fn save_reload_preserves_escaped_quoted_values() {
        let data = b"[Vendor]\nTitle=\"Alice \\\"The #1\\\"\"\n";
        let mut cursor = Cursor::new(&data[..]);
        let cfg = Config::from_reader(&mut cursor).unwrap();
        assert_eq!(
            cfg.get_in(Some("Vendor"), "Title"),
            Some("Alice \"The #1\"")
        );

        let serialized = cfg.to_string().unwrap();
        assert!(serialized.contains("Title=\"Alice \\\"The #1\\\"\""));
        let mut cursor = Cursor::new(serialized.as_bytes());
        let reloaded = Config::from_reader(&mut cursor).unwrap();
        assert_eq!(
            reloaded.get_in(Some("Vendor"), "Title"),
            Some("Alice \"The #1\"")
        );
    }

    #[test]
    fn save_reload_preserves_vendor_markup_and_native_numeric_escapes() {
        let data = b"[Vendor]\nTemplate=\"<i>keep</i>\"\nValue=\"\\101\\x42\\33\"\n";
        let mut cursor = Cursor::new(&data[..]);
        let cfg = Config::from_reader(&mut cursor).unwrap();
        assert_eq!(
            cfg.get_in(Some("Vendor"), "Template"),
            Some("<i>keep</i>")
        );
        assert_eq!(cfg.get_in(Some("Vendor"), "Value"), Some("AB\u{1b}"));

        let serialized = cfg.to_string().unwrap();
        let mut cursor = Cursor::new(serialized.as_bytes());
        let reloaded = Config::from_reader(&mut cursor).unwrap();
        assert_eq!(
            reloaded.get_in(Some("Vendor"), "Template"),
            Some("<i>keep</i>")
        );
        assert_eq!(
            reloaded.get_in(Some("Vendor"), "Value"),
            Some("AB\u{1b}")
        );
    }

    #[test]
    fn server_url_survives_cpp_load_rust_save_and_reload() {
        let data = b"[Network]\nServerAddress=\"https://league.clonkspot.org\"\n";
        let mut cursor = Cursor::new(&data[..]);
        let cfg = Config::from_reader(&mut cursor).unwrap();
        assert_eq!(
            cfg.get_in(Some("Network"), "ServerAddress"),
            Some("https://league.clonkspot.org")
        );

        let dir = tempdir().unwrap();
        let path = dir.path().join("clonk-rust.config");
        cfg.save(&path).unwrap();
        let serialized = std::fs::read_to_string(&path).unwrap();
        assert!(serialized.contains("ServerAddress=\"https://league.clonkspot.org\"\r\n"));
        let reloaded = Config::load(&path).unwrap();

        assert_eq!(
            reloaded.get_in(Some("Network"), "ServerAddress"),
            Some("https://league.clonkspot.org")
        );
    }

    #[test]
    fn hash_prefixed_irc_channels_survive_save_and_reload() {
        let mut config = Config::new();
        config.set_in(Some("IRC"), "Channel", "#clonken,#legacyclonk");

        let serialized = config.to_string().expect("config serializes");
        assert!(serialized.contains("Channel=\"#clonken,#legacyclonk\""));
        let mut reader = Cursor::new(serialized.as_bytes());
        let reloaded = Config::from_reader(&mut reader).expect("config reloads");
        assert_eq!(
            reloaded.get_in(Some("IRC"), "Channel"),
            Some("#clonken,#legacyclonk")
        );
    }

    #[test]
    fn parse_sections() {
        let data = b"[Graphics]\nEngine=OpenGL\n\n#[Audio]\nEnabled = true\n";
        let mut cursor = Cursor::new(&data[..]);
        let cfg = Config::from_reader(&mut cursor).unwrap();
        assert_eq!(cfg.get_in(Some("Graphics"), "Engine"), Some("OpenGL"));
        assert_eq!(cfg.get_in(Some("Audio"), "Enabled"), Some("true"));
        assert!(cfg.get("Engine").is_none());

        let dir = tempdir().unwrap();
        let path = dir.path().join("sections.cfg");
        cfg.save(&path).unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("#[Audio]"));
    }

    #[test]
    fn duplicate_config_values_keep_first_cpp_name_node() {
        // C++ appends name nodes in source order and selects the first matching
        // child for a field compiled once (src/StdCompiler.cpp:517-531, 803-855).
        let data = b"[General]\nLanguage=DE\nLanguage=US\n";
        let mut cursor = Cursor::new(&data[..]);
        let cfg = Config::from_reader(&mut cursor).unwrap();

        assert_eq!(cfg.get_in(Some("General"), "Language"), Some("DE"));
    }

    #[test]
    fn setting_an_earlier_section_keeps_one_cpp_section_block() {
        // StdCompilerINIWrite emits one structural section node and all of
        // its named children together. A later mutation of General must not
        // serialize a second [General] after Network, because C++ reads only
        // the first matching section (src/StdCompiler.cpp:517-531,803-855).
        let mut cfg = Config::new();
        cfg.set_in(Some("General"), "LanguageEx", "US");
        cfg.set_in(Some("Network"), "LocalName", "Host");
        cfg.set_in(Some("General"), "Participants", "Exact.c4p");

        let serialized = cfg.to_string().expect("config serializes");
        assert_eq!(serialized.matches("[General]").count(), 1);
        assert!(
            serialized.find("Participants").expect("participants field")
                < serialized.find("[Network]").expect("network section")
        );

        let mut reader = Cursor::new(serialized.as_bytes());
        let reloaded = Config::from_reader(&mut reader).expect("config reloads");
        assert_eq!(
            reloaded.get_in(Some("General"), "Participants"),
            Some("Exact.c4p")
        );
    }

    #[test]
    fn updating_an_existing_value_replaces_its_full_inline_hash_value() {
        let data = b"[General]\nName=Old # keep this note\n";
        let mut reader = Cursor::new(&data[..]);
        let mut config = Config::from_reader(&mut reader).expect("parse config");

        config.set_in(Some("General"), "Name", "New");

        let entry = config
            .iter()
            .find(|entry| entry.section.as_deref() == Some("General") && entry.key == "Name")
            .expect("updated entry");
        assert_eq!(entry.value, "New");
        assert_eq!(entry.comment, None);
        let serialized = config.to_string().expect("serialize config");
        assert!(serialized.contains("Name=\"New\"\r\n"));
        assert!(!serialized.contains("keep this note"));
        let mut reloaded = Cursor::new(serialized.as_bytes());
        let reloaded = Config::from_reader(&mut reloaded).expect("reload config");
        assert_eq!(
            reloaded
                .iter()
                .find(|entry| entry.key == "Name")
                .and_then(|entry| entry.comment.as_deref()),
            None
        );
    }

    #[test]
    fn standalone_comments_survive_config_rewrites() {
        let data = b"# first note\n[General]\nName=Old\n// trailing note\n";
        let mut reader = Cursor::new(&data[..]);
        let mut config = Config::from_reader(&mut reader).expect("parse config");
        config.set_in(Some("General"), "Name", "New");

        let serialized = config.to_string().expect("serialize config");
        assert!(serialized.contains("# first note\r\n"));
        assert!(serialized.contains("// trailing note\r\n"));
        let mut reader = Cursor::new(serialized.as_bytes());
        let reloaded = Config::from_reader(&mut reader).expect("reload config");
        let rewritten = reloaded.to_string().expect("rewrite config");
        assert!(rewritten.contains("# first note\r\n"));
        assert!(rewritten.contains("// trailing note\r\n"));
    }

    #[test]
    fn l024_trailing_backslash_does_not_consume_following_line() {
        for line_ending in ["\n", "\r\n"] {
            let data = format!("[General]{line_ending}Path=C:\\{line_ending}Next=ok{line_ending}");
            let mut cursor = Cursor::new(data.as_bytes());
            let cfg = Config::from_reader(&mut cursor).unwrap();
            assert_eq!(cfg.get_in(Some("General"), "Path"), Some("C:\\"));
            assert_eq!(cfg.get_in(Some("General"), "Next"), Some("ok"));
        }

        let mut cursor = Cursor::new(b"[General]\nPath=C:\\".as_slice());
        let cfg = Config::from_reader(&mut cursor).unwrap();
        assert_eq!(cfg.get_in(Some("General"), "Path"), Some("C:\\"));
    }

    #[test]
    fn l022_cpp_dump_rust_save_cpp_read_preserves_types_and_bytes() {
        let cpp_dump = b"[General]\r\nEnabled=false\r\nOtherEnabled=true\r\nName=\"Alice\"\r\nLanguageEx=\"US\"\r\nSpecial=\"a b\\\"c\\\\d\"\r\nUtf8=\"M\\303\\274ller\"\r\nLegacy=\"\\374\"\r\nDigits=\"\\1\\61\\62\"\r\nQuotedNumber=\"1\"\r\n\r\n[Graphics]\r\nEngine=OpenGL\r\n";
        let mut reader = Cursor::new(&cpp_dump[..]);
        let config = Config::from_reader(&mut reader).expect("parse C++ config dump");

        let serialized = config.to_string().expect("serialize through Rust");
        assert_eq!(serialized.as_bytes(), cpp_dump);
        assert_eq!(cpp_boolean(&serialized, "General", "Enabled"), Some(false));
        assert_eq!(
            cpp_boolean(&serialized, "General", "OtherEnabled"),
            Some(true)
        );
        assert_eq!(
            cpp_escaped(&serialized, "General", "Name"),
            Some(b"Alice".to_vec())
        );
        assert_eq!(
            cpp_escaped(&serialized, "General", "LanguageEx"),
            Some(b"US".to_vec())
        );
        assert_eq!(
            cpp_escaped(&serialized, "General", "Special"),
            Some(b"a b\"c\\d".to_vec())
        );
        assert_eq!(
            cpp_escaped(&serialized, "General", "Utf8"),
            Some("Müller".as_bytes().to_vec())
        );
        assert_eq!(
            cpp_escaped(&serialized, "General", "Legacy"),
            Some(vec![0xfc])
        );
        assert_eq!(
            cpp_escaped(&serialized, "General", "Digits"),
            Some(vec![1, b'1', b'2'])
        );
        assert_eq!(
            cpp_escaped(&serialized, "General", "QuotedNumber"),
            Some(b"1".to_vec())
        );
        assert_eq!(cpp_value(&serialized, "Graphics", "Engine"), Some("OpenGL"));
    }

    #[test]
    fn l022_new_native_strings_are_quoted_and_scalars_remain_raw() {
        let mut config = Config::new();
        config.set_in(Some("General"), "GamepadEnabled", "false");
        config.set_in(Some("General"), "Name", "Müller \"Q\"\\path");
        config.set_in(Some("General"), "LanguageEx", "US");
        config.set_in(Some("Graphics"), "Engine", "OpenGL");

        let serialized = config.to_string().expect("serialize native fields");
        assert_eq!(
            serialized,
            "[General]\r\nGamepadEnabled=false\r\nName=\"M\\303\\274ller \\\"Q\\\"\\\\path\"\r\nLanguageEx=\"US\"\r\n\r\n[Graphics]\r\nEngine=OpenGL\r\n"
        );
        assert_eq!(
            cpp_boolean(&serialized, "General", "GamepadEnabled"),
            Some(false)
        );
        assert_eq!(
            cpp_escaped(&serialized, "General", "Name"),
            Some("Müller \"Q\"\\path".as_bytes().to_vec())
        );
        assert_eq!(
            cpp_escaped(&serialized, "General", "LanguageEx"),
            Some(b"US".to_vec())
        );
        assert_eq!(cpp_value(&serialized, "Graphics", "Engine"), Some("OpenGL"));
    }

    #[test]
    fn to_string_matches_saved_output() {
        let mut cfg = Config::new();
        cfg.set("Key", "Value");
        cfg.set_in(Some("Section"), "Answer", "42");
        cfg.set_section_commented("Commented", true);
        let dir = tempdir().unwrap();
        let path = dir.path().join("dump.cfg");
        cfg.save(&path).unwrap();
        let saved = std::fs::read_to_string(&path).unwrap();
        let dump = cfg.to_string().unwrap();
        assert_eq!(saved, dump);
    }
}
