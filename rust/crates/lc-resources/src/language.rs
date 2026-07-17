use std::path::{Component, Path, PathBuf};

use crate::{Group, GroupError};

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
        let target = Path::new("System.c4g");
        let mut groups = Vec::with_capacity(self.packs.len().saturating_add(1));
        groups.push(local_system.clone());
        groups.extend(
            self.packs
                .iter()
                .rev()
                .filter_map(|pack| open_mirrored_group(pack, target)),
        );
        ComponentGroups { groups }
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
