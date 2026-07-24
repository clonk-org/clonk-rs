use std::path::{Component, Path, PathBuf};

use crate::{Group, GroupError};

/// The value selected by `C4ComponentHost::GetLanguageString`: C++ searches
/// the entire remainder for CR before falling back to LF.
pub(crate) fn component_language_string<'a>(text: &'a str, code: &str) -> Option<&'a str> {
    let needle = format!("{code}:");
    let position = text.find(&needle)?;
    let rest = &text[position + needle.len()..];
    let end = rest
        .find('\r')
        .or_else(|| rest.find('\n'))
        .unwrap_or(rest.len());
    Some(&rest[..end])
}

/// One `C4LanguageInfo` discovered from a `System.c4g/Language*.txt` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageInfo {
    /// Capitalized native filename bytes used by `C4Language::FindInfo`.
    pub code_bytes: [u8; 2],
    pub code: String,
    pub name: String,
    pub info: String,
    pub fallback: String,
    pub charset: String,
}

impl LanguageInfo {
    /// C++ compares the first two configured bytes through `CharCapital`.
    pub fn matches_code(&self, configured: &str) -> bool {
        let configured = configured.as_bytes();
        configured.len() >= 2
            && self
                .code_bytes
                .into_iter()
                .zip(configured.iter().copied())
                .all(|(left, right)| legacy_char_capital(left) == legacy_char_capital(right))
    }
}

/// External language packs registered from one or more `Language.c4g`
/// containers.
///
/// Each registered pack mirrors the logical group hierarchy below the
/// executable data root. Component loading keeps the local group first and
/// consults the mirrored pack groups only when the same candidate is absent
/// locally, matching `C4Language::GetPackGroups`/`C4ComponentHost::LoadEx`.
#[derive(Debug, Clone, Default)]
pub struct LanguagePacks {
    packs: Vec<Group>,
    logical_roots: Vec<PathBuf>,
}

impl LanguagePacks {
    /// Opens every available `Language.c4g` container and registers its
    /// immediate `*.c4g` children. Missing, malformed, or unopenable
    /// containers and children are skipped like `C4Language::Init`.
    pub fn discover(language_group_paths: &[PathBuf], logical_roots: &[PathBuf]) -> Self {
        let mut result = Self {
            packs: Vec::new(),
            logical_roots: logical_roots.to_vec(),
        };
        for path in language_group_paths {
            let Ok(container) = Group::open(path) else {
                continue;
            };
            result.register_from_group(&container);
        }
        result
    }

    /// Registers the immediate `*.c4g` children of an already-opened
    /// `Language.c4g` container.
    pub fn from_group(container: &Group, logical_roots: &[PathBuf]) -> Self {
        let mut result = Self {
            packs: Vec::new(),
            logical_roots: logical_roots.to_vec(),
        };
        result.register_from_group(container);
        result
    }

    pub fn is_empty(&self) -> bool {
        self.packs.is_empty()
    }

    /// Builds the local-first group set used by `C4ComponentHost::LoadEx`.
    ///
    /// Pack groups retain their original discovery order. When the current
    /// scenario has a `.c4s` Origin, groups below the scenario are looked up
    /// below the corresponding Origin path instead.
    pub fn component_groups(
        &self,
        local: &Group,
        scenario: Option<&Group>,
        origin: Option<&str>,
    ) -> ComponentGroups {
        // AtExeRelativePath leaves paths outside ExePath absolute. Origin
        // remapping still compares those absolute scenario/group paths and
        // may replace their common scenario prefix with a relative Origin.
        let mut target = self
            .logical_path(local.root())
            .unwrap_or_else(|| local.root().to_path_buf());

        if let (Some(scenario), Some(origin)) = (scenario, origin) {
            target = self.remap_scenario_origin(target, scenario, origin);
        }

        let mut groups = Vec::with_capacity(self.packs.len().saturating_add(1));
        groups.push(local.clone());
        groups.extend(
            self.packs
                .iter()
                .filter_map(|pack| open_mirrored_group(pack, &target)),
        );
        ComponentGroups { groups }
    }

    /// Builds the direct `C4Language::InitStringTable` search chain.
    ///
    /// `C4Language::Init` registers equal-priority packs at the front of its
    /// group set, so direct System.c4g lookup visits packs in reverse discovery
    /// order. `LoadEx` reverses that order once more; `component_groups` above
    /// therefore intentionally uses the original order instead.
    pub fn system_groups(&self, local_system: &Group) -> ComponentGroups {
        self.system_groups_with_optional_local(Some(local_system))
    }

    /// The same direct lookup chain when the installed `System.c4g` could
    /// not be opened. C++ still probes every registered language pack.
    pub fn system_groups_with_optional_local(
        &self,
        local_system: Option<&Group>,
    ) -> ComponentGroups {
        let target = Path::new("System.c4g");
        let mut groups = Vec::with_capacity(self.packs.len().saturating_add(1));
        groups.extend(local_system.cloned());
        groups.extend(
            self.packs
                .iter()
                .rev()
                .filter_map(|pack| open_mirrored_group(pack, target)),
        );
        ComponentGroups { groups }
    }

    /// `C4Language::InitInfos`: scan local System first, followed by each
    /// registered pack System in native priority and entry order. The first
    /// successfully loaded table for a case-insensitive two-byte code wins.
    pub fn language_infos(&self, local_system: Option<&Group>) -> Vec<LanguageInfo> {
        let mut infos = Vec::new();
        for group in self
            .system_groups_with_optional_local(local_system)
            .groups()
        {
            let Ok(entries) = group.entries() else {
                continue;
            };
            for entry in entries {
                let Some(code_bytes) = language_entry_code(&entry.name_bytes) else {
                    continue;
                };
                if infos
                    .iter()
                    .any(|info: &LanguageInfo| info.code_bytes == code_bytes)
                {
                    continue;
                }
                let Ok(bytes) = group.read_entry_bytes_exact(&entry) else {
                    continue;
                };
                infos.push(parse_language_info(code_bytes, &bytes));
            }
        }
        infos
    }

    fn register_from_group(&mut self, container: &Group) {
        let Ok(entries) = container.entries() else {
            return;
        };
        for entry in entries {
            if entry.relative_path.components().count() != 1
                || !has_extension(&entry.relative_path, "c4g")
            {
                continue;
            }
            let pack = container.open_child(&entry.relative_path).or_else(|_| {
                let bytes = container.read_entry_bytes_exact(&entry)?;
                Group::from_memory(container.root().join(&entry.relative_path), bytes)
            });
            if let Ok(pack) = pack {
                self.packs.push(pack);
            }
        }
    }

    fn logical_path(&self, full_path: &Path) -> Option<PathBuf> {
        self.logical_roots
            .iter()
            .filter_map(|root| {
                full_path
                    .strip_prefix(root)
                    .ok()
                    .map(|relative| (root.components().count(), relative.to_path_buf()))
            })
            .max_by_key(|(specificity, _)| *specificity)
            .map(|(_, relative)| relative)
    }

    fn remap_scenario_origin(&self, target: PathBuf, scenario: &Group, origin: &str) -> PathBuf {
        let normalized_origin = PathBuf::from(origin.replace('\\', "/"));
        if !has_extension(&normalized_origin, "c4s") {
            return target;
        }
        let scenario_path = self
            .logical_path(scenario.root())
            .unwrap_or_else(|| scenario.root().to_path_buf());
        let Ok(remainder) = target.strip_prefix(&scenario_path) else {
            return target;
        };
        normalized_origin.join(remainder)
    }
}

/// An ordered local-plus-language-pack component search chain.
#[derive(Debug, Clone)]
pub struct ComponentGroups {
    groups: Vec<Group>,
}

impl ComponentGroups {
    pub fn local(group: &Group) -> Self {
        Self {
            groups: vec![group.clone()],
        }
    }

    pub fn groups(&self) -> &[Group] {
        &self.groups
    }

    /// Reads one concrete component candidate from the first group that has
    /// it. Candidate ordering remains the caller's responsibility, just as in
    /// `C4ComponentHost::Load`; this method implements only group priority.
    pub fn read(&self, candidate: impl AsRef<Path>) -> Result<Option<LoadedComponent>, GroupError> {
        let candidate = candidate.as_ref();
        for group in &self.groups {
            if !group.exists(candidate) {
                continue;
            }
            // LoadEntryString fails for both an empty entry and a read error.
            // Finding the entry in this group still masks the same candidate
            // in lower-priority groups; the caller then advances to its next
            // language/filename candidate.
            let Ok(bytes) = group.read_file(candidate) else {
                return Ok(None);
            };
            if bytes.is_empty() {
                return Ok(None);
            }
            return Ok(Some(LoadedComponent {
                path: group.root().join(candidate),
                bytes,
            }));
        }
        Ok(None)
    }
}

/// A selected component and its full local-or-pack logical path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedComponent {
    pub bytes: Vec<u8>,
    pub path: PathBuf,
}

fn open_mirrored_group(pack: &Group, target: &Path) -> Option<Group> {
    let mut current = pack.clone();
    for component in target.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(name) => current = current.open_child(Path::new(name)).ok()?,
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(current)
}

fn has_extension(path: &Path, expected: &str) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case(expected))
}

fn legacy_char_capital(byte: u8) -> u8 {
    match byte {
        b'a'..=b'z' => byte - 32,
        0xe4 => 0xc4,
        0xf6 => 0xd6,
        0xfc => 0xdc,
        _ => byte,
    }
}

fn language_entry_code(name: &[u8]) -> Option<[u8; 2]> {
    const PREFIX: &[u8] = b"Language";
    const SUFFIX: &[u8] = b".txt";
    if name.len() < PREFIX.len() + SUFFIX.len()
        || !name[..PREFIX.len()].eq_ignore_ascii_case(PREFIX)
        || !name[name.len() - SUFFIX.len()..].eq_ignore_ascii_case(SUFFIX)
    {
        return None;
    }
    let stem = &name[..name.len() - SUFFIX.len()];
    let code = stem.get(stem.len().checked_sub(2)?..)?;
    Some([legacy_char_capital(code[0]), legacy_char_capital(code[1])])
}

fn parse_language_info(code_bytes: [u8; 2], bytes: &[u8]) -> LanguageInfo {
    let mut table = bytes.split(|byte| *byte == 0).next().unwrap_or_default();
    let mut entries = std::collections::HashMap::<&[u8], Vec<u8>>::new();
    while let Some(equals) = table.iter().position(|byte| *byte == b'=') {
        let key = &table[..equals];
        table = &table[equals + 1..];
        let value_end = table
            .iter()
            .position(|byte| matches!(*byte, b'\r' | b'\n'))
            .and_then(|line_end| {
                table[line_end..]
                    .iter()
                    .position(|byte| !matches!(*byte, b'\r' | b'\n'))
                    .map(|offset| line_end + offset)
            })
            .unwrap_or(table.len());
        let value_with_line_end = &table[..value_end];
        table = &table[value_end..];
        let value_end = value_with_line_end
            .iter()
            .rposition(|byte| !matches!(*byte, b'\r' | b'\n'))
            .map_or(0, |index| index + 1);
        let value = &value_with_line_end[..value_end];
        if entries.contains_key(key) {
            continue;
        }
        let mut value = value.to_vec();
        let mut cursor = 0;
        while cursor + 1 < value.len() {
            if value[cursor..].starts_with(b"\\n") {
                value.splice(cursor..cursor + 2, *b"\r\n");
                cursor += 2;
            } else {
                cursor += 1;
            }
        }
        entries.insert(key, value);
    }

    let get = |key: &'static [u8], replace_pipes: bool| {
        let mut value = entries.get(key).cloned().unwrap_or_else(|| {
            format!("[Undefined: {}]", String::from_utf8_lossy(key)).into_bytes()
        });
        if replace_pipes {
            value.iter_mut().for_each(|byte| {
                if *byte == b'|' {
                    *byte = b' ';
                }
            });
        }
        crate::decode_legacy_script_text(&value)
    };

    LanguageInfo {
        code_bytes,
        code: crate::decode_legacy_script_text(&code_bytes),
        name: get(b"IDS_LANG_NAME", true),
        info: get(b"IDS_LANG_INFO", true),
        fallback: get(b"IDS_LANG_FALLBACK", true),
        charset: get(b"IDS_LANG_CHARSET", false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MutableGroup;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn directory_packs_mirror_logical_paths_and_local_components_win() {
        let temp = tempdir().expect("temporary language fixture");
        let content = temp.path().join("content");
        let scenario_path = content.join("Missions.c4f/Goldmine.c4s");
        fs::create_dir_all(&scenario_path).expect("local scenario group");
        let scenario = Group::open(&scenario_path).expect("open local scenario");

        let container_path = temp.path().join("Language.c4g");
        let pack_scenario = container_path.join("Finnish.c4g/Missions.c4f/Goldmine.c4s");
        fs::create_dir_all(&pack_scenario).expect("mirrored language-pack scenario");
        fs::write(pack_scenario.join("TitleFI.txt"), b"FI:Pakattu\n")
            .expect("pack title component");

        let packs = LanguagePacks::discover(&[container_path], std::slice::from_ref(&content));
        assert!(!packs.is_empty());
        let groups = packs.component_groups(&scenario, None, None);
        let packed = groups
            .read("TitleFI.txt")
            .expect("read pack title")
            .expect("pack title exists");
        assert_eq!(packed.bytes, b"FI:Pakattu\n");
        assert_eq!(packed.path, pack_scenario.join("TitleFI.txt"));

        fs::write(scenario_path.join("TitleFI.txt"), b"FI:Paikallinen\n")
            .expect("local title component");
        let local = groups
            .read("TitleFI.txt")
            .expect("read local title")
            .expect("local title exists");
        assert_eq!(local.bytes, b"FI:Paikallinen\n");
        assert_eq!(local.path, scenario_path.join("TitleFI.txt"));
    }

    #[test]
    fn most_specific_root_and_scenario_origin_retain_the_nested_suffix() {
        let temp = tempdir().expect("temporary Origin fixture");
        let install = temp.path().join("install");
        let content = install.join("content");
        let scenario_path = content.join("Actual.c4s");
        let definition_path = scenario_path.join("Local.c4d");
        fs::create_dir_all(&definition_path).expect("scenario-local definition");
        let scenario = Group::open(&scenario_path).expect("open actual scenario");
        let definition = Group::open(&definition_path).expect("open local definition");

        let container_path = install.join("Language.c4g");
        let origin_definition = container_path.join("Pack.c4g/Archive.c4f/Original.c4s/Local.c4d");
        fs::create_dir_all(&origin_definition).expect("Origin-mirrored definition");
        fs::write(origin_definition.join("StringTblUS.txt"), b"Key=Packed\n")
            .expect("pack string table");

        let packs = LanguagePacks::discover(&[container_path], &[install.clone(), content.clone()]);
        assert_eq!(
            packs
                .component_groups(
                    &definition,
                    Some(&scenario),
                    Some("Archive.c4f\\Original.c4s"),
                )
                .read("StringTblUS.txt")
                .expect("read Origin-remapped table")
                .expect("Origin-remapped table exists")
                .bytes,
            b"Key=Packed\n"
        );
        assert!(packs
            .component_groups(&definition, Some(&scenario), None)
            .read("StringTblUS.txt")
            .expect("probe actual path")
            .is_none());
    }

    #[test]
    fn scenario_origin_remaps_an_absolute_group_outside_logical_roots() {
        let temp = tempdir().expect("temporary absolute-Origin fixture");
        let unrelated_root = temp.path().join("install");
        let scenario_path = temp.path().join("user/Actual.c4s");
        let definition_path = scenario_path.join("Local.c4d");
        fs::create_dir_all(&definition_path).expect("user scenario definition");
        let scenario = Group::open(&scenario_path).expect("open user scenario");
        let definition = Group::open(&definition_path).expect("open user definition");

        let container_path = unrelated_root.join("Language.c4g");
        let origin_definition = container_path.join("Pack.c4g/Archive.c4f/Original.c4s/Local.c4d");
        fs::create_dir_all(&origin_definition).expect("Origin-mirrored definition");
        fs::write(origin_definition.join("StringTblUS.txt"), b"Key=Packed\n")
            .expect("pack string table");
        let packs = LanguagePacks::discover(
            std::slice::from_ref(&container_path),
            std::slice::from_ref(&unrelated_root),
        );

        let loaded = packs
            .component_groups(
                &definition,
                Some(&scenario),
                Some("Archive.c4f/Original.c4s"),
            )
            .read("StringTblUS.txt")
            .expect("read remapped component")
            .expect("absolute scenario path remaps to Origin");
        assert_eq!(loaded.bytes, b"Key=Packed\n");
    }

    #[test]
    fn packed_language_container_and_child_groups_are_traversed() {
        let temp = tempdir().expect("temporary packed-language fixture");
        let mut system = MutableGroup::new("System.c4g");
        system
            .add_file("LanguageFI.txt", b"IDS_LANG_NAME=Suomi\n".to_vec())
            .expect("language table");
        let mut pack = MutableGroup::new("Finnish.c4g");
        pack.add_child("System.c4g", system)
            .expect("pack System group");
        let mut container = MutableGroup::new("Language.c4g");
        container
            .add_file("Finnish.c4g", pack.pack().expect("pack Finnish.c4g"))
            .expect("ordinary packed language entry");
        let container_path = temp.path().join("Language.c4g");
        fs::write(
            &container_path,
            container.pack().expect("pack Language.c4g"),
        )
        .expect("write packed Language.c4g");

        let local_system_path = temp.path().join("planet/System.c4g");
        fs::create_dir_all(&local_system_path).expect("local System group");
        let local_system = Group::open(local_system_path).expect("open local System group");
        let packs = LanguagePacks::discover(&[container_path], &[temp.path().join("planet")]);
        let loaded = packs
            .system_groups(&local_system)
            .read("LanguageFI.txt")
            .expect("read packed language table")
            .expect("packed language table exists");
        assert_eq!(loaded.bytes, b"IDS_LANG_NAME=Suomi\n");
    }

    #[test]
    fn system_lookup_reverses_pack_discovery_but_components_do_not() {
        let temp = tempdir().expect("temporary pack-order fixture");
        let local_system_path = temp.path().join("local/System.c4g");
        fs::create_dir_all(&local_system_path).expect("local System group");
        let local_system = Group::open(local_system_path).expect("open local System group");

        let make_pack = |name: &str, value: &[u8]| {
            let root = temp.path().join(name);
            fs::create_dir_all(root.join("System.c4g")).expect("pack System group");
            fs::write(root.join("System.c4g/LanguageFI.txt"), value).expect("pack language table");
            Group::open(root).expect("open language pack")
        };
        let first = make_pack("First.c4g", b"first");
        let second = make_pack("Second.c4g", b"second");
        let packs = LanguagePacks {
            packs: vec![first, second],
            logical_roots: vec![temp.path().join("local")],
        };

        let system = packs
            .system_groups(&local_system)
            .read("LanguageFI.txt")
            .expect("read direct language table")
            .expect("direct language table exists");
        assert_eq!(system.bytes, b"second");

        let component = packs
            .component_groups(&local_system, None, None)
            .read("LanguageFI.txt")
            .expect("read LoadEx component")
            .expect("LoadEx component exists");
        assert_eq!(component.bytes, b"first");
    }

    #[test]
    fn language_infos_preserve_cpp_priority_parsing_and_first_successful_code() {
        fn system(files: &[(&str, &[u8])]) -> Group {
            let mut group = MutableGroup::new("System.c4g");
            for &(name, bytes) in files {
                group
                    .add_file(name, bytes.to_vec())
                    .expect("add language table");
            }
            Group::from_memory(
                PathBuf::from("System.c4g"),
                group.pack().expect("pack System group"),
            )
            .expect("open packed System group")
        }

        fn pack(name: &str, files: &[(&str, &[u8])]) -> Group {
            let mut root = MutableGroup::new(name);
            let mut system = MutableGroup::new("System.c4g");
            for &(entry, bytes) in files {
                system
                    .add_file(entry, bytes.to_vec())
                    .expect("add pack language table");
            }
            root.add_child("System.c4g", system)
                .expect("add pack System group");
            Group::from_memory(
                PathBuf::from(name),
                root.pack().expect("pack language pack"),
            )
            .expect("open language pack")
        }

        let local = system(&[
            (
                "Language00US.txt",
                b"IDS_LANG_NAME=English\nIDS_LANG_INFO=\nIDS_LANG_FALLBACK=\n",
            ),
            ("LanguageDE.txt", b""),
            (
                "lAnGuAgEfi.TxT",
                b"IDS_LANG_NAME=Suomi|Local\n\
                  IDS_LANG_NAME=Ignored\n\
                  IDS_LANG_INFO=Line|one\\nLine two\n\
                  IDS_LANG_FALLBACK=DE|US\n\
                  IDS_LANG_CHARSET=UTF|8\0IDS_LANG_INFO=ignored",
            ),
        ]);
        let first = pack(
            "First.c4g",
            &[
                ("LanguageDE.txt", b"IDS_LANG_NAME=wrong pack\n"),
                ("LanguageNO.txt", b"IDS_LANG_NAME=Norsk\n"),
                ("LanguageUS.txt", b"IDS_LANG_NAME=wrong local duplicate\n"),
            ],
        );
        let second = pack(
            "Second.c4g",
            &[
                (
                    "LanguageDE.txt",
                    b"IDS_LANG_NAME=Deutsch aus zweitem Pack\n",
                ),
                ("LanguageSV.txt", b"IDS_LANG_NAME=Svenska\n"),
            ],
        );
        let packs = LanguagePacks {
            packs: vec![first, second],
            logical_roots: Vec::new(),
        };

        let infos = packs.language_infos(Some(&local));
        assert_eq!(
            infos
                .iter()
                .map(|info| info.code.as_str())
                .collect::<Vec<_>>(),
            vec!["US", "DE", "FI", "SV", "NO"]
        );

        let us = infos.iter().find(|info| info.code == "US").unwrap();
        assert_eq!(us.info, "");
        assert_eq!(us.charset, "[Undefined: IDS_LANG_CHARSET]");

        let fi = infos.iter().find(|info| info.code == "FI").unwrap();
        assert_eq!(fi.name, "Suomi Local");
        assert_eq!(fi.info, "Line one\r\nLine two");
        assert_eq!(fi.fallback, "DE US");
        assert_eq!(fi.charset, "UTF|8");

        let de = infos.iter().find(|info| info.code == "DE").unwrap();
        assert_eq!(de.name, "[Undefined: IDS_LANG_NAME]");

        let pack_only = packs.language_infos(None);
        assert_eq!(pack_only[0].code, "DE");
        assert_eq!(pack_only[0].name, "Deutsch aus zweitem Pack");

        assert_eq!(
            language_entry_code(b"Language\xe4x.txt"),
            Some([0xc4, b'X'])
        );
        let legacy = parse_language_info([0xc4, b'X'], b"");
        assert_eq!(legacy.code, "\u{00c4}X");
        let utf8 = parse_language_info([0xc3, 0xa4], b"");
        assert_eq!(utf8.code, "\u{00e4}");
        assert!(utf8.matches_code("\u{00e4} - UTF-8 filename"));
    }

    #[test]
    fn rejected_candidate_masks_lower_groups_but_not_the_next_candidate() {
        let temp = tempdir().expect("temporary component failure fixture");
        let local_path = temp.path().join("Local.c4s");
        let pack_path = temp.path().join("Pack.c4g/Local.c4s");
        fs::create_dir_all(&local_path).expect("local group");
        fs::create_dir_all(&pack_path).expect("pack group");
        fs::write(local_path.join("Empty.txt"), []).expect("empty local component");
        fs::create_dir(local_path.join("Unreadable.txt")).expect("unreadable local component");
        fs::write(pack_path.join("Empty.txt"), b"lower empty candidate")
            .expect("lower empty candidate");
        fs::write(
            pack_path.join("Unreadable.txt"),
            b"lower unreadable candidate",
        )
        .expect("lower unreadable candidate");
        fs::write(pack_path.join("Next.txt"), b"next candidate").expect("next candidate");
        let groups = ComponentGroups {
            groups: vec![
                Group::open(local_path).expect("open local group"),
                Group::open(pack_path).expect("open pack group"),
            ],
        };

        assert!(groups.read("Empty.txt").expect("empty lookup").is_none());
        assert!(groups
            .read("Unreadable.txt")
            .expect("unreadable lookup")
            .is_none());
        assert_eq!(
            groups
                .read("Next.txt")
                .expect("next lookup")
                .expect("next candidate exists")
                .bytes,
            b"next candidate"
        );
    }
}
