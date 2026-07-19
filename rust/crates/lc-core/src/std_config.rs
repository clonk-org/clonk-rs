mod parser;

use indexmap::IndexMap;
use parser::{parse_line, ParsedItem};
use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub section: Option<String>,
    pub key: String,
    pub value: String,
    pub comment: Option<String>,
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
                    comment,
                } => {
                    let section_clone = current_section.clone();
                    config.ensure_section(section_clone.clone(), false);
                    let key_owned = key.into_owned();
                    let entry = Entry {
                        section: section_clone.clone(),
                        key: key_owned.clone(),
                        value: value.into_owned(),
                        comment,
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
        let mut line_accumulator = String::new();
        let mut current_section: Option<String> = None;

        loop {
            buffer.clear();
            let bytes_read = reader.read_line(&mut buffer)?;
            if bytes_read == 0 {
                if !line_accumulator.trim().is_empty() {
                    if let Some(comment) = standalone_comment(&line_accumulator) {
                        config.standalone_comments.push(comment);
                    } else if let Some(item) = parse_line(&line_accumulator) {
                        handle_item(&mut config, &mut current_section, item);
                    }
                }
                break;
            }
            line_accumulator.push_str(&buffer);
            if line_continues(&mut line_accumulator) {
                continue;
            }
            if let Some(comment) = standalone_comment(&line_accumulator) {
                config.standalone_comments.push(comment);
            } else if let Some(item) = parse_line(&line_accumulator) {
                handle_item(&mut config, &mut current_section, item);
            }
            line_accumulator.clear();
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
            existing.value = value;
            return;
        }
        let entry = Entry {
            section: section_owned.clone(),
            key: key.clone(),
            value,
            comment: None,
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
            writeln!(writer, "{comment}")?;
        }
        let mut current_section: Option<&Option<String>> = None;
        for entry in self.entries.values() {
            let section_ref = &entry.section;
            if current_section != Some(section_ref) {
                if let Some(section_name) = section_ref.as_ref() {
                    let commented = self
                        .section_meta
                        .get(section_ref)
                        .map(|meta| meta.commented)
                        .unwrap_or(false);
                    if commented {
                        writeln!(writer, "#[{}]", section_name)?;
                    } else {
                        writeln!(writer, "[{}]", section_name)?;
                    }
                } else if current_section.is_some() {
                    writeln!(writer)?;
                }
                current_section = Some(section_ref);
            }
            if let Some(comment) = &entry.comment {
                // Keep entry comments inline, which is the form the parser
                // can associate with the same name node on the next load.
                // Emitting a standalone `#...` line silently detached it
                // during an Options -> advanced editor save/reload cycle.
                writeln!(
                    writer,
                    "{} = {} #{}",
                    entry.key,
                    quote_value(&entry.value),
                    comment
                )?;
            } else {
                writeln!(writer, "{} = {}", entry.key, quote_value(&entry.value))?;
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

fn quote_value(value: &str) -> String {
    if !value.contains(|ch: char| ch.is_whitespace() || ch.is_control() || ch == '#' || ch == '"') {
        return value.to_string();
    }
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for character in value.chars() {
        match character {
            '\u{7}' => quoted.push_str("\\a"),
            '\u{8}' => quoted.push_str("\\b"),
            '\u{c}' => quoted.push_str("\\f"),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            '\u{b}' => quoted.push_str("\\v"),
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            character if character.is_control() => {
                quoted.push('\\');
                quoted.push_str(&format!("{:o}", character as u32));
            }
            character => quoted.push(character),
        }
    }
    quoted.push('"');
    quoted
}

fn line_continues(line: &mut String) -> bool {
    let mut end = line.len();
    while end > 0 && matches!(line.as_bytes()[end - 1], b'\n' | b'\r') {
        end -= 1;
    }
    while end > 0 {
        let ch = line.as_bytes()[end - 1] as char;
        if ch.is_whitespace() && ch != '\\' {
            end -= 1;
        } else {
            break;
        }
    }
    if end == 0 {
        return false;
    }
    let mut backslash_count = 0;
    let mut cursor = end;
    while cursor > 0 && line.as_bytes()[cursor - 1] == b'\\' {
        backslash_count += 1;
        cursor -= 1;
    }
    let continuation = backslash_count % 2 == 1;
    if continuation {
        while matches!(line.chars().last(), Some('\n') | Some('\r')) {
            line.pop();
        }
        while let Some(last) = line.chars().last() {
            if last.is_whitespace() && last != '\\' {
                line.pop();
            } else {
                break;
            }
        }
        if matches!(line.as_bytes().last(), Some(b'\\')) {
            line.pop();
        }
        line.push('\n');
    }
    continuation
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use tempfile::tempdir;

    #[test]
    fn parse_basic_config() {
        let data = b"Name = <i>Player</i> # main user\nEnabled=true\n\n";
        let mut cursor = Cursor::new(&data[..]);
        let cfg = Config::from_reader(&mut cursor).unwrap();
        assert_eq!(cfg.get("Name"), Some("<i>Player</i>"));
        assert_eq!(cfg.get_bool("Enabled"), Some(true));
        let entry = cfg
            .entries
            .values()
            .find(|entry| entry.key == "Name")
            .unwrap();
        assert_eq!(entry.comment.as_deref(), Some("main user"));
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
    fn save_reload_preserves_continued_and_escaped_quoted_values() {
        let data = b"[Vendor]\nMultiline=first\\\nsecond\nTitle=\"Alice \\\"The #1\\\"\" # keep\n";
        let mut cursor = Cursor::new(&data[..]);
        let cfg = Config::from_reader(&mut cursor).unwrap();
        assert_eq!(
            cfg.get_in(Some("Vendor"), "Multiline"),
            Some("first\nsecond")
        );
        assert_eq!(
            cfg.get_in(Some("Vendor"), "Title"),
            Some("Alice \"The #1\"")
        );

        let serialized = cfg.to_string().unwrap();
        assert!(serialized.contains("Multiline = \"first\\nsecond\""));
        assert!(serialized.contains("Title = \"Alice \\\"The #1\\\"\" #keep"));
        let mut cursor = Cursor::new(serialized.as_bytes());
        let reloaded = Config::from_reader(&mut cursor).unwrap();
        assert_eq!(
            reloaded.get_in(Some("Vendor"), "Multiline"),
            Some("first\nsecond")
        );
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
        let path = dir.path().join("legacyclonk.config");
        cfg.save(&path).unwrap();
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
        assert!(serialized.contains("Channel = \"#clonken,#legacyclonk\""));
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
    fn updating_an_existing_value_preserves_its_comment() {
        let data = b"[General]\nName=Old # keep this note\n";
        let mut reader = Cursor::new(&data[..]);
        let mut config = Config::from_reader(&mut reader).expect("parse config");

        config.set_in(Some("General"), "Name", "New");

        let entry = config
            .iter()
            .find(|entry| entry.section.as_deref() == Some("General") && entry.key == "Name")
            .expect("updated entry");
        assert_eq!(entry.value, "New");
        assert_eq!(entry.comment.as_deref(), Some("keep this note"));
        let serialized = config.to_string().expect("serialize config");
        assert!(serialized.contains("Name = New #keep this note"));
        let mut reloaded = Cursor::new(serialized.as_bytes());
        let reloaded = Config::from_reader(&mut reloaded).expect("reload config");
        assert_eq!(
            reloaded
                .iter()
                .find(|entry| entry.key == "Name")
                .and_then(|entry| entry.comment.as_deref()),
            Some("keep this note")
        );
    }

    #[test]
    fn standalone_comments_survive_config_rewrites() {
        let data = b"# first note\n[General]\nName=Old\n// trailing note\n";
        let mut reader = Cursor::new(&data[..]);
        let mut config = Config::from_reader(&mut reader).expect("parse config");
        config.set_in(Some("General"), "Name", "New");

        let serialized = config.to_string().expect("serialize config");
        assert!(serialized.contains("# first note\n"));
        assert!(serialized.contains("// trailing note\n"));
        let mut reader = Cursor::new(serialized.as_bytes());
        let reloaded = Config::from_reader(&mut reader).expect("reload config");
        let rewritten = reloaded.to_string().expect("rewrite config");
        assert!(rewritten.contains("# first note\n"));
        assert!(rewritten.contains("// trailing note\n"));
    }

    #[test]
    fn parse_multiline_values() {
        let data = b"[General]\nDescription=First line\\\nsecond line\\\nthird line\n";
        let mut cursor = Cursor::new(&data[..]);
        let cfg = Config::from_reader(&mut cursor).unwrap();
        let value = cfg.get_in(Some("General"), "Description").unwrap();
        assert_eq!(value, "First line\nsecond line\nthird line");
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
