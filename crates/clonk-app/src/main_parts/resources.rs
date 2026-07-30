//! `main.rs` — scenario/definition resolution and network host preparation.
//!
//! A contiguous slice moved verbatim from the crate root; it stays part of
//! the same binary crate, re-exported from `main.rs` so every path resolves.

use super::*;

impl FrontendScenario {
    pub(crate) fn from_command_line(path: &Path) -> Self {
        let title = path
            .file_stem()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .unwrap_or("Command-line scenario")
            .to_string();
        Self {
            identifier: path.to_string_lossy().into_owned(),
            title,
            description: None,
            kind: ScenarioKind::Scenario,
            is_editable: false,
            is_playable: true,
            mission_access: None,
            path: Some(path.to_path_buf()),
            source_paths: vec![path.to_path_buf()],
            root_label: None,
            preview: None,
            title_picture: None,
            children: Vec::new(),
            folder_index: None,
            icon_index: None,
            difficulty: None,
            author: None,
            version: None,
            local_only: None,
            allow_user_change: None,
            definition_modules: Vec::new(),
        }
    }

    pub(crate) fn to_ui_entry(&self) -> ScenarioEntry {
        let preview = self
            .preview
            .clone()
            .or_else(|| Some(generate_preview_placeholder(self.kind, &self.title)));
        ScenarioEntry {
            identifier: self.identifier.clone(),
            title: self.title.clone(),
            description: self.description.clone(),
            kind: self.kind,
            is_editable: self.is_editable,
            is_playable: self.is_playable,
            location: self.location_label(),
            preview,
        }
    }

    pub(crate) fn from_resource(entry: resource_scenario::ScenarioEntry, root_label: &str) -> Self {
        let resource_scenario::ScenarioEntry {
            identifier,
            path,
            title,
            description,
            kind,
            is_editable,
            is_playable,
            mission_access,
            preview,
            title_picture,
            children,
            folder_index,
            icon_index,
            difficulty,
            author,
            version,
            local_only,
            allow_user_change,
            definition_modules,
        } = entry;

        let kind = match kind {
            resource_scenario::ScenarioEntryKind::Scenario => ScenarioKind::Scenario,
            resource_scenario::ScenarioEntryKind::Folder => ScenarioKind::Folder,
            resource_scenario::ScenarioEntryKind::Editor => ScenarioKind::Editor,
        };

        let children = children
            .into_iter()
            .map(|child| FrontendScenario::from_resource(child, root_label))
            .collect();

        let to_image = |preview: resource_scenario::ScenarioPreview| {
            let (width, height, pixels) = preview.into_arc();
            ImageData::from_arc(width, height, pixels)
        };
        let preview = preview.map(to_image);
        let title_picture = title_picture.map(to_image);

        let source_paths = vec![path.clone()];
        Self {
            identifier,
            title,
            description,
            kind,
            is_editable,
            is_playable,
            mission_access,
            path: Some(path),
            source_paths,
            root_label: Some(root_label.to_string()),
            preview,
            title_picture,
            children,
            folder_index,
            icon_index,
            difficulty,
            author,
            version,
            local_only,
            allow_user_change,
            definition_modules,
        }
    }

    /// Mission-only visibility predicate shared by list rows and map-folder
    /// button construction. Player-count failures deliberately do not hide
    /// map buttons in C++.
    pub(crate) fn has_mission_access(&self, access: &MissionAccessStore) -> bool {
        self.mission_access
            .as_deref()
            .is_none_or(|password| password.is_empty() || access.contains(password))
    }

    pub(crate) fn location_label(&self) -> Option<String> {
        if let Some(path) = self.path.as_ref() {
            if let Some(root) = self.root_label.as_ref() {
                let components: Vec<&str> = self
                    .identifier
                    .split('/')
                    .filter(|component| !component.is_empty())
                    .collect();
                let relative = if components.is_empty() {
                    String::new()
                } else {
                    components.join(" / ")
                };
                if relative.is_empty() {
                    return Some(root.clone());
                }
                return Some(format!("{root} / {relative}"));
            }
            return Some(path.display().to_string());
        }
        if self.path.is_none() && matches!(self.kind, ScenarioKind::Scenario) {
            return Some("Built-in Rust sandbox".to_string());
        }
        None
    }

    pub(crate) fn fallback() -> Self {
        Self {
            identifier: "rust_sandbox".to_string(),
            title: FALLBACK_SCENARIO_TITLE.to_string(),
            description: Some("Spawn a Rust-driven walker in a flat test landscape.".to_string()),
            kind: ScenarioKind::Scenario,
            is_editable: true,
            is_playable: true,
            mission_access: None,
            path: None,
            source_paths: Vec::new(),
            root_label: None,
            preview: Some(generate_preview_placeholder(
                ScenarioKind::Scenario,
                FALLBACK_SCENARIO_TITLE,
            )),
            title_picture: None,
            children: Vec::new(),
            folder_index: None,
            icon_index: None,
            difficulty: None,
            author: None,
            version: None,
            local_only: None,
            allow_user_change: None,
            definition_modules: Vec::new(),
        }
    }
}

pub(crate) fn merge_frontend_scenarios(
    entries: Vec<FrontendScenario>,
    alphabetical_sorting: bool,
) -> Vec<FrontendScenario> {
    let mut result: Vec<FrontendScenario> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();

    for entry in entries {
        if let Some(&existing_idx) = index.get(&entry.identifier) {
            let existing = &mut result[existing_idx];
            if existing.kind == entry.kind {
                if is_container_kind(&existing.kind) && is_container_kind(&entry.kind) {
                    merge_container(existing, entry, alphabetical_sorting);
                } else {
                    merge_leaf(existing, entry);
                }
            } else {
                tracing::warn!(
                    identifier = %existing.identifier,
                    existing_kind = ?existing.kind,
                    incoming_kind = ?entry.kind,
                    "scenario catalog contained identifier with mismatched kinds; keeping existing entry"
                );
            }
            continue;
        }
        index.insert(entry.identifier.clone(), result.len());
        result.push(entry);
    }

    sort_frontend_entries(&mut result, alphabetical_sorting);
    result
}

fn merge_leaf(existing: &mut FrontendScenario, mut incoming: FrontendScenario) {
    merge_metadata(existing, &mut incoming);
}

fn merge_container(
    existing: &mut FrontendScenario,
    mut incoming: FrontendScenario,
    alphabetical_sorting: bool,
) {
    merge_metadata(existing, &mut incoming);
    merge_children(
        &mut existing.children,
        incoming.children,
        alphabetical_sorting,
    );
    sort_frontend_entries(&mut existing.children, alphabetical_sorting);
}

fn merge_metadata(existing: &mut FrontendScenario, incoming: &mut FrontendScenario) {
    let provenance = existing
        .path
        .iter()
        .chain(existing.source_paths.iter())
        .chain(incoming.path.iter())
        .chain(incoming.source_paths.iter())
        .cloned()
        .collect::<Vec<_>>();
    existing.source_paths.clear();
    let mut seen_paths = HashSet::new();
    for path in provenance {
        if seen_paths.insert(scenario_root_key(&path)) {
            existing.source_paths.push(path);
        }
    }
    if existing.description.is_none() {
        existing.description = incoming.description.take();
    }
    if existing.preview.is_none() {
        existing.preview = incoming.preview.take();
    }
    if existing.path.is_none() {
        existing.path = incoming.path.take();
    }
    if existing.root_label.is_none() {
        existing.root_label = incoming.root_label.take();
    }
    existing.is_editable |= incoming.is_editable;
    existing.is_playable |= incoming.is_playable;
    if existing.folder_index.is_none() {
        existing.folder_index = incoming.folder_index;
    }
    if existing.icon_index.is_none() {
        existing.icon_index = incoming.icon_index;
    }
    if existing.difficulty.is_none() {
        existing.difficulty = incoming.difficulty;
    }
    if existing.title_picture.is_none() {
        existing.title_picture = incoming.title_picture.take();
    }
    if existing.author.is_none() {
        existing.author = incoming.author.take();
    }
    if existing.version.is_none() {
        existing.version = incoming.version.take();
    }
    if existing.local_only.is_none() {
        existing.local_only = incoming.local_only;
    }
    if existing.allow_user_change.is_none() {
        existing.allow_user_change = incoming.allow_user_change;
    }
    if existing.definition_modules.is_empty() {
        existing.definition_modules = std::mem::take(&mut incoming.definition_modules);
    }
}

fn merge_children(
    existing_children: &mut Vec<FrontendScenario>,
    incoming_children: Vec<FrontendScenario>,
    alphabetical_sorting: bool,
) {
    if incoming_children.is_empty() {
        return;
    }

    let mut index: HashMap<String, usize> = existing_children
        .iter()
        .enumerate()
        .map(|(idx, child)| (child.identifier.clone(), idx))
        .collect();

    for child in incoming_children {
        if let Some(&existing_idx) = index.get(&child.identifier) {
            if is_container_kind(&existing_children[existing_idx].kind)
                && is_container_kind(&child.kind)
            {
                let existing_child = &mut existing_children[existing_idx];
                merge_container(existing_child, child, alphabetical_sorting);
            }
            continue;
        }
        index.insert(child.identifier.clone(), existing_children.len());
        existing_children.push(child);
    }
}

fn is_container_kind(kind: &ScenarioKind) -> bool {
    matches!(kind, ScenarioKind::Folder | ScenarioKind::Editor)
}

pub(crate) fn sort_frontend_entries(entries: &mut [FrontendScenario], alphabetical_sorting: bool) {
    entries.sort_by(|a, b| compare_frontend_entries(a, b, alphabetical_sorting));
    for entry in entries.iter_mut() {
        sort_frontend_entries(&mut entry.children, alphabetical_sorting);
    }
}

pub(crate) fn override_frontend_scenario_title(
    entries: &mut [FrontendScenario],
    identifier: &str,
    title: &str,
    alphabetical_sorting: bool,
) -> bool {
    let mut changed = false;
    for entry in entries.iter_mut() {
        if entry.identifier == identifier {
            entry.title = title.to_string();
            changed = true;
        }
        changed |= override_frontend_scenario_title(
            &mut entry.children,
            identifier,
            title,
            alphabetical_sorting,
        );
    }
    if changed {
        sort_frontend_entries(entries, alphabetical_sorting);
    }
    changed
}

fn compare_frontend_entries(
    a: &FrontendScenario,
    b: &FrontendScenario,
    alphabetical_sorting: bool,
) -> Ordering {
    let a_is_folder = matches!(a.kind, ScenarioKind::Folder);
    let b_is_folder = matches!(b.kind, ScenarioKind::Folder);
    if a_is_folder != b_is_folder {
        return if a_is_folder {
            Ordering::Less
        } else {
            Ordering::Greater
        };
    }

    if !alphabetical_sorting {
        let a_folder_index = a.folder_index.unwrap_or(0);
        let b_folder_index = b.folder_index.unwrap_or(0);
        if a_folder_index != 0 || b_folder_index != 0 {
            if a_folder_index == 0 {
                return Ordering::Greater;
            }
            if b_folder_index == 0 {
                return Ordering::Less;
            }
            match a_folder_index.cmp(&b_folder_index) {
                Ordering::Equal => {}
                other => return other,
            }
        }
    }

    if let Some(icon) = a.icon_index {
        if (2..=11).contains(&icon) {
            let other_icon = b.icon_index.unwrap_or(-1);
            let diff = icon - other_icon;
            if diff != 0 {
                return diff.cmp(&0);
            }
        }
    }

    if !alphabetical_sorting {
        let a_difficulty = a.difficulty.unwrap_or(0);
        let b_difficulty = b.difficulty.unwrap_or(0);
        if a_difficulty != 0 || b_difficulty != 0 {
            if a_difficulty == 0 {
                return Ordering::Greater;
            }
            if b_difficulty == 0 {
                return Ordering::Less;
            }
            match a_difficulty.cmp(&b_difficulty) {
                Ordering::Equal => {}
                other => return other,
            }
        }
    }

    let title_order = compare_case_insensitive(&a.title, &b.title);
    if title_order != Ordering::Equal {
        return title_order;
    }

    compare_case_insensitive(&a.identifier, &b.identifier)
}

fn compare_case_insensitive(a: &str, b: &str) -> Ordering {
    a.to_ascii_lowercase().cmp(&b.to_ascii_lowercase())
}

pub(crate) struct InstallDefinitionResolver {
    app_paths: Option<Arc<AppPaths>>,
    language_packs: LanguagePacks,
}

impl InstallDefinitionResolver {
    pub(crate) fn new(app_paths: Option<Arc<AppPaths>>) -> Self {
        let language_packs = app_paths
            .as_deref()
            .map(classic_language_packs)
            .unwrap_or_default();
        Self {
            app_paths,
            language_packs,
        }
    }

    fn sanitize_identifier(identifier: &str) -> Option<PathBuf> {
        let mut slice = identifier.trim();
        if slice.is_empty() {
            return None;
        }
        slice = slice.trim_matches(|c| c == '"' || c == '\'');
        let normalized = slice.replace('\\', "/");
        let absolute = path_from_group_name_bytes(&clonk_script::c4_string_bytes(&normalized));
        if absolute.is_absolute() {
            return Some(absolute);
        }
        let mut slice = normalized.as_str();
        while let Some(stripped) = slice.strip_prefix("./") {
            slice = stripped;
        }
        slice = slice.trim_matches('/');
        if slice.is_empty() {
            return None;
        }
        Some(path_from_group_name_bytes(&clonk_script::c4_string_bytes(
            slice,
        )))
    }

    fn open_and_push(
        path: &Path,
        groups: &mut Vec<Group>,
        seen: &mut HashSet<PathBuf>,
    ) -> Result<(), ScenarioError> {
        match Group::open(path) {
            Ok(group) => Self::push_group(groups, seen, group),
            Err(err) if Self::should_ignore_error(&err) => {}
            Err(err) => return Err(ScenarioError::Resources(err)),
        }
        Ok(())
    }

    fn push_group(groups: &mut Vec<Group>, seen: &mut HashSet<PathBuf>, group: Group) {
        let root = group.root().to_path_buf();
        if seen.insert(root) {
            groups.push(group);
        }
    }

    fn should_ignore_error(err: &GroupError) -> bool {
        matches!(
            err,
            GroupError::Missing(_) | GroupError::NotDirectory(_) | GroupError::EntryNotFound(_)
        ) || matches!(err, GroupError::Io(io_err) if io_err.kind() == io::ErrorKind::NotFound)
    }
    fn executable_data_bases(&self) -> Vec<PathBuf> {
        let Some(paths) = self.app_paths.as_deref() else {
            return Vec::new();
        };
        let mut bases = Vec::new();
        if let Some(content) = paths.content_dir() {
            bases.push(content.to_path_buf());
        }
        bases.extend([
            paths.planet_dir().to_path_buf(),
            paths.install_root().to_path_buf(),
            paths.system_group_path().to_path_buf(),
        ]);
        let mut seen = HashSet::new();
        bases.retain(|path| seen.insert(scenario_root_key(path)));
        bases
    }

    fn c4f_parent_paths(path: &Path) -> Vec<PathBuf> {
        let mut parents = Vec::new();
        let mut current = if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("c4f"))
        {
            Some(path)
        } else {
            path.parent()
        };
        while let Some(parent) = current.filter(|candidate| {
            candidate
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("c4f"))
        }) {
            parents.push(parent.to_path_buf());
            current = parent.parent();
        }
        parents.reverse();
        parents
    }

    fn scenario_origin(&self, scenario: &Group) -> Option<PathBuf> {
        let source = read_group_file_case_insensitive(scenario, "Scenario.txt")?;
        let mut reader = io::Cursor::new(source);
        let config = Config::from_reader(&mut reader).ok()?;
        let raw = config.get_in(Some("Head"), "Origin")?.trim();
        if raw.is_empty() {
            return None;
        }
        let relative = raw.replace('\\', "/");
        let relative = relative.trim_start_matches('/');
        self.executable_data_bases()
            .into_iter()
            .map(|base| base.join(relative))
            .find(|candidate| Group::open(candidate).is_ok())
    }

    fn push_material_child(parent: &Group, groups: &mut Vec<Group>) -> Result<(), ScenarioError> {
        if let Some(materials) = open_child_flexible(parent, Path::new("Material.c4g"))
            .map_err(ScenarioError::Resources)?
        {
            groups.push(materials);
        }
        Ok(())
    }

    fn push_graphics_child(
        parent: &Group,
        groups: &mut Vec<Group>,
        seen: &mut HashSet<PathBuf>,
    ) -> Result<(), ScenarioError> {
        match open_child_flexible(parent, Path::new("Graphics.c4g")) {
            Ok(Some(graphics)) => Self::push_group(groups, seen, graphics),
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(
                    root = %parent.root().display(),
                    %error,
                    "failed to open optional registered Graphics.c4g"
                );
            }
        }
        Ok(())
    }

    fn app_definition_bases(&self) -> Vec<PathBuf> {
        // C4GameResList opens every explicit DefinitionFilename from the
        // process ExePath. AppPaths may project that installed data directory
        // across source-checkout roots, but user/scenario directories are not
        // fallback search locations for an explicit pack.
        self.executable_data_bases()
    }

    fn append_relative_at(
        base: &Path,
        relative: &Path,
        groups: &mut Vec<Group>,
        seen: &mut HashSet<PathBuf>,
    ) -> Result<(), ScenarioError> {
        if let Ok(group) = Group::open(base) {
            match open_child_flexible(&group, relative) {
                Ok(Some(child)) => Self::push_group(groups, seen, child),
                Ok(None) => {}
                // Ancestor search may walk a shared temp directory. An
                // unrelated entry disappearing during that scan is still a
                // simple miss for this candidate base.
                Err(error) if Self::should_ignore_error(&error) => {}
                Err(error) => return Err(ScenarioError::Resources(error)),
            }
        }

        let candidate = base.join(relative);
        match Group::open(&candidate) {
            Ok(group) => Self::push_group(groups, seen, group),
            Err(err) if Self::should_ignore_error(&err) => {}
            Err(err) => return Err(ScenarioError::Resources(err)),
        }
        Ok(())
    }

    fn resolve_definition_groups_ordered(
        &self,
        scenario: &Group,
        identifier: &str,
        _explicit_roots_first: bool,
    ) -> Result<Vec<Group>, ScenarioError> {
        let Some(relative) = Self::sanitize_identifier(identifier) else {
            return Err(ScenarioError::LegacyDefinitionNotFound {
                path: identifier.to_string(),
            });
        };
        let mut groups = Vec::new();
        let mut seen = HashSet::new();

        if relative.is_absolute() {
            Self::open_and_push(&relative, &mut groups, &mut seen)?;
        } else {
            // DefinitionFilenames are opened once from executable-data roots.
            // Folder/scenario-local resources are appended by clonk-engine's
            // separate outer-to-inner InitDefs pass and cannot rescue a
            // missing explicit vector entry (C4Game.cpp:181-213,3961-3994).
            let bases = if self.app_paths.is_some() {
                self.app_definition_bases()
            } else {
                let mut ancestors = scenario
                    .root()
                    .ancestors()
                    .map(Path::to_path_buf)
                    .collect::<Vec<_>>();
                ancestors.reverse();
                ancestors
            };
            let mut base_seen = HashSet::new();
            for base in bases {
                if base_seen.insert(base.clone()) {
                    Self::append_relative_at(&base, &relative, &mut groups, &mut seen)?;
                    if !groups.is_empty() {
                        return Ok(groups);
                    }
                }
            }
        }

        // Explicit DefinitionFilenames are opened directly from ExePath.
        // Folder/scenario-local definitions are a separate InitDefs pass in
        // clonk-engine and must not rescue a missing external vector entry here.

        if groups.is_empty() {
            Err(ScenarioError::LegacyDefinitionNotFound {
                path: identifier.to_string(),
            })
        } else {
            Ok(groups)
        }
    }
}

impl LegacyDefinitionResolver for InstallDefinitionResolver {
    fn resolve_definition_groups(
        &self,
        scenario: &Group,
        identifier: &str,
    ) -> Result<Vec<Group>, ScenarioError> {
        self.resolve_definition_groups_ordered(scenario, identifier, true)
    }

    fn resolve_language_packs(&self, _scenario: &Group) -> Result<LanguagePacks, ScenarioError> {
        Ok(self.language_packs.clone())
    }

    fn resolve_material_groups(&self, scenario: &Group) -> Result<Vec<Group>, ScenarioError> {
        // RegisterParentFolders assigns increasing priority from outermost to
        // innermost. Origin parents are registered later and therefore win
        // equal-priority ties (C4GroupSet.cpp:238-318).
        let mut registrations = Vec::new();
        for (registration_order, path) in [
            Some(scenario.root().to_path_buf()),
            self.scenario_origin(scenario),
        ]
        .into_iter()
        .flatten()
        .enumerate()
        {
            if registration_order != 0
                && scenario_root_key(&path) == scenario_root_key(scenario.root())
            {
                continue;
            }
            for (folder_priority, parent) in Self::c4f_parent_paths(&path).into_iter().enumerate() {
                registrations.push((folder_priority, registration_order, parent));
            }
        }
        registrations
            .sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1)));

        let mut groups = Vec::new();
        for (_, _, parent_path) in registrations {
            let parent =
                open_group_path_for_folder_map(&parent_path).map_err(ScenarioError::Resources)?;
            Self::push_material_child(&parent, &mut groups)?;
        }

        // Extra.c4g root has lower priority than folders but higher priority
        // than the final executable-data Material.c4g. Per-definition Extra
        // children register only after OpenScenario has snapshotted GameRes,
        // so they deliberately do not enter this simulation material chain.
        for base in self.executable_data_bases() {
            let extra_path = base.join("Extra.c4g");
            match Group::open(&extra_path) {
                Ok(extra) => {
                    Self::push_material_child(&extra, &mut groups)?;
                    break;
                }
                Err(error) if Self::should_ignore_error(&error) => {}
                Err(error) => return Err(ScenarioError::Resources(error)),
            }
        }

        if let Some(paths) = self.app_paths.as_deref() {
            if let Some(global) = candidate_material_paths(paths).into_iter().next() {
                groups.push(Group::open(global).map_err(ScenarioError::Resources)?);
            }
        }
        Ok(groups)
    }

    fn resolve_graphics_groups(&self, scenario: &Group) -> Result<Vec<Group>, ScenarioError> {
        self.resolve_graphics_groups_with_definition_roots(scenario, &[])
    }

    fn resolve_graphics_groups_with_definition_roots(
        &self,
        scenario: &Group,
        definition_roots: &[Group],
    ) -> Result<Vec<Group>, ScenarioError> {
        // RegisterMainGroups mirrors the active Game.GroupSet priority:
        // scenario-local Graphics.c4g, inner-to-outer scenario/origin folders,
        // Extra.c4g's graphics, definition-pack roots, and finally the base
        // Graphics.c4g (C4GraphicsResource.cpp:351-380;
        // C4Game.cpp:2432-2442; C4GroupSet.cpp:87-110,238-318).
        let mut groups = Vec::new();
        let mut seen = HashSet::new();
        Self::push_graphics_child(scenario, &mut groups, &mut seen)?;

        let mut registrations = Vec::new();
        for (registration_order, path) in [
            Some(scenario.root().to_path_buf()),
            self.scenario_origin(scenario),
        ]
        .into_iter()
        .flatten()
        .enumerate()
        {
            if registration_order != 0
                && scenario_root_key(&path) == scenario_root_key(scenario.root())
            {
                continue;
            }
            for (folder_priority, parent) in Self::c4f_parent_paths(&path).into_iter().enumerate() {
                registrations.push((folder_priority, registration_order, parent));
            }
        }
        registrations
            .sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1)));
        for (_, _, parent_path) in registrations {
            let parent =
                open_group_path_for_folder_map(&parent_path).map_err(ScenarioError::Resources)?;
            Self::push_graphics_child(&parent, &mut groups, &mut seen)?;
        }

        for base in self.executable_data_bases() {
            let extra_path = base.join("Extra.c4g");
            match Group::open(&extra_path) {
                Ok(extra) => {
                    // Extra.Init registers one matching child per final
                    // DefinitionFilenames entry. RegisterMainGroups reverses
                    // the equal-priority child order a second time, so the
                    // earlier activated child is the earlier graphics source.
                    for definition_root in definition_roots {
                        let Some(name) = definition_root.root().file_name() else {
                            continue;
                        };
                        let child = match open_child_flexible(&extra, Path::new(name)) {
                            Ok(Some(child)) => child,
                            Ok(None) => continue,
                            Err(error) => {
                                tracing::warn!(
                                    extra = %extra.root().display(),
                                    definition = %name.to_string_lossy(),
                                    %error,
                                    "failed to open activated Extra.c4g graphics group"
                                );
                                continue;
                            }
                        };
                        match open_child_flexible(&child, Path::new("Graphics.c4g")) {
                            Ok(Some(graphics)) => groups.push(graphics),
                            Ok(None) => {}
                            Err(error) => {
                                tracing::warn!(
                                    root = %child.root().display(),
                                    %error,
                                    "failed to open optional Extra definition Graphics.c4g"
                                );
                            }
                        }
                    }
                    Self::push_graphics_child(&extra, &mut groups, &mut seen)?;
                    break;
                }
                Err(error) if Self::should_ignore_error(&error) => {}
                Err(error) => return Err(ScenarioError::Resources(error)),
            }
        }

        // Definition roots are registered at equal priority in selected-vector
        // order. Game.GroupSet reverses that order, and RegisterMainGroups'
        // target registration reverses it again, so the first selected pack is
        // the first graphics lookup source. Keep duplicates: C++ reopens and
        // registers every NRT_Definitions entry independently. A child that
        // cannot be opened is silently skipped by RegisterGroups.
        for definition_root in definition_roots {
            if let Ok(Some(graphics)) =
                open_child_flexible(definition_root, Path::new("Graphics.c4g"))
            {
                groups.push(graphics);
            }
        }

        if let Some(paths) = self.app_paths.as_deref() {
            let global_path = paths.planet_dir().join("Graphics.c4g");
            match Group::open(&global_path) {
                Ok(global) => Self::push_group(&mut groups, &mut seen, global),
                Err(error) if Self::should_ignore_error(&error) => {}
                Err(error) => return Err(ScenarioError::Resources(error)),
            }
        } else {
            match self.resolve_definition_groups_ordered(scenario, "Graphics.c4g", true) {
                Ok(fallbacks) => {
                    for fallback in fallbacks {
                        Self::push_group(&mut groups, &mut seen, fallback);
                    }
                }
                Err(ScenarioError::LegacyDefinitionNotFound { .. }) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(groups)
    }
}

pub(crate) fn open_child_flexible(
    group: &Group,
    relative: &Path,
) -> Result<Option<Group>, GroupError> {
    match group.open_child(relative) {
        Ok(child) => Ok(Some(child)),
        Err(err) => match err {
            GroupError::EntryNotFound(_) | GroupError::Missing(_) | GroupError::NotDirectory(_) => {
                match open_child_case_insensitive(group, relative) {
                    Ok(child) => Ok(Some(child)),
                    Err(GroupError::EntryNotFound(_)) => Ok(None),
                    Err(other) => Err(other),
                }
            }
            other => Err(other),
        },
    }
}

fn open_child_case_insensitive(group: &Group, relative: &Path) -> Result<Group, GroupError> {
    let mut current = group.clone();
    let mut consumed = PathBuf::new();

    for component in relative.components() {
        match component {
            Component::Normal(name) => {
                let target = name.to_string_lossy().to_ascii_lowercase();
                let entries = current.entries()?;
                let matched = entries.into_iter().find(|entry| {
                    if entry.relative_path.components().count() != 1 {
                        return false;
                    }
                    entry
                        .relative_path
                        .file_name()
                        .and_then(|candidate| candidate.to_str())
                        .map(|candidate| candidate.eq_ignore_ascii_case(&target))
                        .unwrap_or(false)
                });

                let entry = match matched {
                    Some(entry) => entry,
                    None => {
                        let mut missing = consumed.clone();
                        missing.push(name);
                        return Err(GroupError::EntryNotFound(missing));
                    }
                };

                consumed.push(&entry.relative_path);
                current = current.open_child(&entry.relative_path)?;
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                let mut invalid = consumed.clone();
                invalid.push(component.as_os_str());
                return Err(GroupError::EntryNotFound(invalid));
            }
        }
    }

    Ok(current)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SavedScenarioInfo {
    pub(crate) identifier: String,
    pub(crate) title: String,
    pub(crate) description: Option<String>,
    pub(crate) path: Option<PathBuf>,
    #[serde(default)]
    pub(crate) root_label: Option<String>,
    pub(crate) is_editable: bool,
    pub(crate) is_playable: bool,
    pub(crate) label: String,
    pub(crate) fallback_ground: i32,
    pub(crate) sandbox: bool,
}

impl SavedScenarioInfo {
    pub(crate) fn from_frontend(
        frontend: &FrontendScenario,
        label: &str,
        fallback_ground: i32,
    ) -> Self {
        Self {
            identifier: frontend.identifier.clone(),
            title: frontend.title.clone(),
            description: frontend.description.clone(),
            path: frontend.path.clone(),
            root_label: frontend.root_label.clone(),
            is_editable: frontend.is_editable,
            is_playable: frontend.is_playable,
            label: label.to_string(),
            fallback_ground,
            sandbox: frontend.path.is_none(),
        }
    }

    pub(crate) fn to_frontend(&self) -> FrontendScenario {
        FrontendScenario {
            identifier: self.identifier.clone(),
            title: self.title.clone(),
            description: self.description.clone(),
            kind: ScenarioKind::Scenario,
            is_editable: self.is_editable,
            is_playable: self.is_playable,
            mission_access: None,
            path: self.path.clone(),
            source_paths: Vec::new(),
            root_label: self.root_label.clone(),
            preview: Some(generate_preview_placeholder(
                ScenarioKind::Scenario,
                &self.title,
            )),
            title_picture: None,
            children: Vec::new(),
            folder_index: None,
            icon_index: None,
            difficulty: None,
            author: None,
            version: None,
            local_only: None,
            allow_user_change: None,
            definition_modules: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct SaveFileVersion {
    major: u16,
    minor: u16,
    patch: u16,
}

impl SaveFileVersion {
    pub(crate) const fn new(major: u16, minor: u16, patch: u16) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    const fn major(self) -> u16 {
        self.major
    }

    fn parse_str(input: &str) -> Result<Self, String> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err("save file version cannot be empty".to_string());
        }

        let mut components = trimmed.split('.').collect::<Vec<_>>();
        if components.len() > 3 {
            return Err(format!(
                "save file version `{trimmed}` has too many components"
            ));
        }

        while components.len() < 3 {
            components.push("0");
        }

        let major = Self::parse_component(components[0], "major")?;
        let minor = Self::parse_component(components[1], "minor")?;
        let patch = Self::parse_component(components[2], "patch")?;
        Ok(Self::new(major, minor, patch))
    }

    fn from_numeric(value: u64) -> Result<Self, String> {
        if value > u16::MAX as u64 {
            return Err(format!(
                "legacy save file version `{value}` exceeds supported range"
            ));
        }
        Ok(Self::new(value as u16, 0, 0))
    }

    fn parse_component(component: &str, name: &str) -> Result<u16, String> {
        if component.is_empty() {
            return Err(format!("save file version has empty {name} component"));
        }
        component
            .parse::<u16>()
            .map_err(|_| format!("save file version `{component}` is not a valid {name} number"))
    }
}

impl fmt::Display for SaveFileVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl Serialize for SaveFileVersion {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for SaveFileVersion {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        struct SaveFileVersionVisitor;

        impl<'de> Visitor<'de> for SaveFileVersionVisitor {
            type Value = SaveFileVersion;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a semantic version string like \"1.0.0\" or legacy integer")
            }

            fn visit_u64<E>(self, value: u64) -> std::result::Result<Self::Value, E>
            where
                E: de::Error,
            {
                SaveFileVersion::from_numeric(value).map_err(E::custom)
            }

            fn visit_i64<E>(self, value: i64) -> std::result::Result<Self::Value, E>
            where
                E: de::Error,
            {
                if value < 0 {
                    return Err(E::invalid_value(
                        Unexpected::Signed(value),
                        &"non-negative version number",
                    ));
                }
                self.visit_u64(value as u64)
            }

            fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E>
            where
                E: de::Error,
            {
                SaveFileVersion::parse_str(value).map_err(E::custom)
            }

            fn visit_string<E>(self, value: String) -> std::result::Result<Self::Value, E>
            where
                E: de::Error,
            {
                self.visit_str(&value)
            }
        }

        deserializer.deserialize_any(SaveFileVersionVisitor)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SavedGameFile {
    pub(crate) version: SaveFileVersion,
    pub(crate) saved_at_seconds: u64,
    pub(crate) scenario: SavedScenarioInfo,
    /// Effective external definition vector. C++ stores this in exact saves
    /// and reconstructs DefinitionFilenames before restoring runtime state.
    #[serde(default)]
    pub(crate) definition_load: Option<ScenarioDefinitionLoad>,
    pub(crate) focus_id: Option<ObjectId>,
    #[serde(default)]
    pub(crate) user_label: Option<String>,
    /// C++ serializes Game.IsMusicEnabled independently of RXMusic. Option
    /// preserves backward compatibility with Rust saves predating the field.
    #[serde(default)]
    pub(crate) runtime_music_enabled: Option<bool>,
    /// Byte-exact C4 `SavePlayerInfos.txt` projection belonging to the saved
    /// source image. `Some([])` means the native component was absent; `None`
    /// identifies older Rust saves that must reconstruct it from the current
    /// takeover roster.
    #[serde(default)]
    pub(crate) source_save_player_infos: Option<Vec<u8>>,
    /// Exact `Strings.txt` enumeration produced when this virtual savegame
    /// was written. `None` identifies older Rust saves.
    #[serde(default)]
    pub(crate) source_string_table: Option<Vec<Vec<u8>>>,
    /// Raw sidecar thumbnail loaded with this JSON save. Native stores the
    /// same frame>0 image inside the exact savegame as `Title.png`.
    #[serde(skip, default)]
    pub(crate) source_title_png: Option<Vec<u8>>,
    pub(crate) engine_state: EngineState,
}

#[derive(Debug, Clone, Deserialize)]
struct SavedGameHeader {
    #[allow(dead_code)]
    version: SaveFileVersion,
    saved_at_seconds: u64,
    scenario: SavedScenarioInfo,
    #[serde(default)]
    user_label: Option<String>,
}

struct SaveMigration {
    from: SaveFileVersion,
    to: SaveFileVersion,
    apply: fn(SavedGameFile) -> Result<SavedGameFile>,
}

const SAVE_MIGRATIONS: &[SaveMigration] = &[];

pub(crate) fn migrate_save_file(save: SavedGameFile) -> Result<SavedGameFile> {
    if save.version == SAVE_FILE_VERSION {
        return Ok(save);
    }

    if save.version.major() > SAVE_FILE_VERSION.major() {
        anyhow::bail!(
            "quick save requires clonk-app {} or newer (current engine {})",
            save.version,
            SAVE_FILE_VERSION
        );
    }

    if save.version.major() < SAVE_FILE_VERSION.major() {
        anyhow::bail!(
            "quick save version {} cannot be loaded by this engine (current {})",
            save.version,
            SAVE_FILE_VERSION
        );
    }

    apply_save_migrations(save)
}

fn apply_save_migrations(mut save: SavedGameFile) -> Result<SavedGameFile> {
    let mut applied = 0usize;
    while save.version < SAVE_FILE_VERSION {
        if let Some(migration) = SAVE_MIGRATIONS
            .iter()
            .find(|candidate| candidate.from == save.version)
        {
            tracing::info!(
                from = %migration.from,
                to = %migration.to,
                "applying quick save migration"
            );
            save = (migration.apply)(save)?;
            applied = applied
                .checked_add(1)
                .ok_or_else(|| anyhow!("quick save migration overflow"))?;
            if applied > SAVE_MIGRATIONS.len() {
                anyhow::bail!("detected cycle in quick save migrations");
            }
            continue;
        }

        tracing::warn!(
            from = %save.version,
            to = %SAVE_FILE_VERSION,
            "no explicit migration for quick save version; assuming backward compatibility"
        );
        save.version = SAVE_FILE_VERSION;
    }

    Ok(save)
}

pub(crate) fn cached_app_paths() -> std::result::Result<Arc<AppPaths>, PathsError> {
    cached_app_paths_with_config_file(None)
}

pub(crate) fn cached_app_paths_with_config_file(
    explicit_config_file: Option<&Path>,
) -> std::result::Result<Arc<AppPaths>, PathsError> {
    #[cfg(test)]
    let _env_guard = crate::tests::env_lock().lock();
    let mut cache = APP_PATH_CACHE.lock().unwrap();
    if let Some(result) = cache.as_ref() {
        return result.clone();
    }

    let discovered = AppPaths::discover_with_config_file(explicit_config_file).map(Arc::new);
    *cache = Some(discovered.clone());
    discovered
}

pub(crate) fn reset_cached_app_paths() {
    let mut cache = APP_PATH_CACHE.lock().unwrap();
    *cache = None;
}

pub(crate) fn resolve_save_directory() -> PathBuf {
    match cached_app_paths() {
        Ok(paths) => paths.user_data_dir().join(SAVE_DIR_NAME),
        Err(_) => PathBuf::from(SAVE_DIR_NAME),
    }
}

pub(crate) fn configured_savegame_directory(paths: Option<&AppPaths>) -> PathBuf {
    let Some(paths) = paths else {
        // The built-in pathless Rust sandbox has no C4Config/ExePath
        // provenance. Keep its existing app-data fallback explicit.
        return resolve_save_directory();
    };

    let configured = match load_classic_loader_config(paths) {
        Ok(Some(config)) => classic_loader_config_value(&config, "SaveGameFolder")
            .unwrap_or(DEFAULT_CLASSIC_SAVE_GAME_FOLDER)
            .to_string(),
        Ok(None) => DEFAULT_CLASSIC_SAVE_GAME_FOLDER.to_string(),
        Err(error) => {
            tracing::warn!(
                %error,
                "failed to read configured savegame folder; using classic default"
            );
            DEFAULT_CLASSIC_SAVE_GAME_FOLDER.to_string()
        }
    };
    let configured = PathBuf::from(configured);
    if configured.is_absolute() {
        configured
    } else {
        // C4Game::QuickSave switches to Config.General.ExePath before using
        // the (usually relative) Config.General.SaveGameFolder value.
        paths.install_root().join(configured)
    }
}

fn cpp_filename_only(name: &str) -> &str {
    match name.rfind('.') {
        Some(dot) if dot + 1 < name.len() => &name[..dot],
        _ => name,
    }
}

pub(crate) fn looks_like_cpp_integer(value: &str) -> bool {
    let digits = value
        .as_bytes()
        .strip_prefix(b"+")
        .or_else(|| value.as_bytes().strip_prefix(b"-"))
        .unwrap_or(value.as_bytes());
    !digits.is_empty() && digits.iter().all(u8::is_ascii_digit)
}

fn is_c4_folder_group(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, extension)| extension.eq_ignore_ascii_case("c4f"))
}

fn classic_savegame_scenario_name_from_parts(
    leaf: &str,
    parent: Option<&str>,
    fallback_title: &str,
) -> String {
    let stem = cpp_filename_only(leaf);

    if looks_like_cpp_integer(stem) {
        return parent
            .filter(|parent| is_c4_folder_group(parent))
            .map(cpp_filename_only)
            .unwrap_or(stem)
            .to_string();
    }

    let stripped = stem.trim_end_matches(|character: char| character.is_ascii_digit());
    if stripped.is_empty() {
        // LooksLikeInteger catches every ASCII digit-only stem. This fallback
        // only protects synthetic/pathless fixtures with no usable group name.
        sanitize_save_label(fallback_title)
    } else {
        stripped.to_string()
    }
}

pub(crate) fn classic_savegame_scenario_name(scenario: &FrontendScenario) -> String {
    // Game.ScenarioFilename is authoritative in C++. The catalog identifier
    // is only a Rust-only fallback for the pathless sandbox.
    if let Some(path) = scenario.path.as_deref() {
        if let Some(leaf) = path.file_name().map(|name| name.to_string_lossy()) {
            let parent = path
                .parent()
                .and_then(Path::file_name)
                .map(|name| name.to_string_lossy());
            return classic_savegame_scenario_name_from_parts(
                leaf.as_ref(),
                parent.as_deref(),
                &scenario.title,
            );
        }
    }

    let logical_components = scenario
        .identifier
        .split(['/', '\\'])
        .filter(|component| !component.is_empty())
        .collect::<Vec<_>>();
    let leaf = logical_components.last().copied().unwrap_or("");
    let parent =
        (logical_components.len() >= 2).then(|| logical_components[logical_components.len() - 2]);
    classic_savegame_scenario_name_from_parts(leaf, parent, &scenario.title)
}

pub(crate) fn classic_savegame_slot_path(root: &Path, scenario_name: &str, slot: u8) -> PathBuf {
    root.join(format!("{scenario_name}.c4f"))
        .join(format!("{scenario_name}{slot}.c4s"))
}

pub(crate) fn c4group_is_group(path: &Path) -> bool {
    Group::open(path).is_ok()
}

pub(crate) fn classic_save_folder_language(paths: Option<&AppPaths>) -> Vec<u8> {
    let config = load_native_config_bytes(paths);
    clonk_app_netplay::configured_native_value(&config, "General", "Language")
        .filter(|language| !language.is_empty())
        .map(|language| language.as_bytes().iter().copied().take(2).collect())
        .unwrap_or_else(|| {
            classic_loader_system_language()
                .unwrap_or("US")
                .as_bytes()
                .to_vec()
        })
}

pub(crate) fn ensure_classic_save_folder(path: &Path, language: &[u8], title: &[u8]) -> Result<()> {
    fs::create_dir_all(path)
        .with_context(|| format!("create classic save folder {}", path.display()))?;
    let title_path = path.join("Title.txt");
    if title_path.exists() {
        return Ok(());
    }
    let mut payload = language.iter().copied().take(2).collect::<Vec<_>>();
    payload.push(b':');
    payload.extend_from_slice(title);
    let mut file = File::create(&title_path)
        .with_context(|| format!("create classic save title {}", title_path.display()))?;
    file.write_all(&payload)
        .with_context(|| format!("write classic save title {}", title_path.display()))?;
    file.flush()
        .with_context(|| format!("flush classic save title {}", title_path.display()))
}

pub(crate) fn ensure_save_directory() -> Result<PathBuf> {
    let dir = resolve_save_directory();
    fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create save directory at {}", dir.display()))?;
    Ok(dir)
}

pub(crate) fn default_quick_save_path() -> PathBuf {
    resolve_save_directory().join(QUICK_SAVE_FILE)
}

pub(crate) fn existing_quick_save_path() -> Option<PathBuf> {
    let path = default_quick_save_path();
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

fn quick_save_exists() -> bool {
    existing_quick_save_path().is_some()
}

pub(crate) fn is_save_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("lcsave"))
        .unwrap_or(false)
}

fn any_saved_games_exist() -> bool {
    let dir = resolve_save_directory();
    match fs::read_dir(&dir) {
        Ok(entries) => entries.flatten().any(|entry| {
            let path = entry.path();
            is_save_file(&path)
        }),
        Err(_) => quick_save_exists(),
    }
}

pub(crate) fn load_install_material_library(paths: Option<&AppPaths>) -> Option<Arc<MaterialSet>> {
    let paths = paths?;

    let mut seen = HashSet::new();
    for candidate in candidate_material_paths(paths) {
        if !candidate.exists() {
            continue;
        }
        let key = candidate.to_string_lossy().to_ascii_lowercase();
        if !seen.insert(key) {
            continue;
        }
        match try_materials_from_path(&candidate) {
            Ok(set) if !set.is_empty() => {
                let count = set.len();
                tracing::info!(path = %candidate.display(), count, "loaded material definitions");
                return Some(Arc::new(set));
            }
            Ok(_) => {
                tracing::debug!(path = %candidate.display(), "material candidate contained no definitions");
            }
            Err(clonk_resources::MaterialError::NotFound) => {}
            Err(err) => {
                tracing::debug!(path = %candidate.display(), error = %err, "material discovery attempt failed");
            }
        }
    }
    tracing::info!("no install material definitions found; using sandbox defaults");
    None
}

fn candidate_material_paths(paths: &AppPaths) -> Vec<PathBuf> {
    // Direct group paths ONLY — never bare roots: a root candidate makes
    // the loaders walk the WHOLE planet/content tree recursively on every
    // scenario activation (the dominant load-time cost after music).
    // C++ opens the known Material.c4g groups directly
    // (C4Game::OpenScenario's material chain).
    let mut candidates = Vec::new();
    let mut seen = HashSet::new();

    let mut group_bases = Vec::new();
    if let Some(content) = paths.content_dir() {
        group_bases.push(content.to_path_buf());
    }
    group_bases.extend([
        paths.planet_dir().to_path_buf(),
        paths.install_root().to_path_buf(),
        paths.system_group_path().to_path_buf(),
    ]);

    for base in group_bases {
        let path = base.join("Material.c4g");
        let key = scenario_root_key(&path);
        if path.exists() && seen.insert(key) {
            candidates.push(path);
        }
    }
    candidates
}

pub(crate) fn material_value_array<const N: usize>(values: Option<Vec<i32>>) -> [u8; N] {
    let mut result = [0; N];
    values
        .into_iter()
        .flatten()
        .take(N)
        .enumerate()
        .for_each(|(index, value)| result[index] = value as u8);
    result
}

fn material_int_array<const N: usize>(values: Option<Vec<i32>>) -> [i32; N] {
    let mut result = [0; N];
    values
        .into_iter()
        .flatten()
        .take(N)
        .enumerate()
        .for_each(|(index, value)| result[index] = value);
    result
}

pub(crate) fn material_render_placement(material: &clonk_resources::MaterialDefinition) -> i32 {
    if let Some(placement) = material
        .int("Placement")
        .filter(|placement| *placement != 0)
    {
        return placement;
    }
    let density = material.int("Density").unwrap_or(0);
    if density >= 50 {
        let mut placement = 30;
        if !material.bool_flag("DigFree").unwrap_or(false) {
            placement += 20;
        }
        if !material.bool_flag("BlastFree").unwrap_or(false) {
            placement += 10;
        }
        if !material
            .bool_flag("Dig2ObjectRequest")
            .or_else(|| material.bool_flag("Dig2ObjectOnRequestOnly"))
            .unwrap_or(false)
        {
            placement += 10;
        }
        placement
    } else if density >= 25 {
        10
    } else {
        5
    }
}

pub(crate) fn material_render_info(
    material: &clonk_resources::MaterialDefinition,
) -> clonk_frontend::MaterialRenderInfo {
    let color = material_value_array(
        material
            .int_list("ColorX")
            .or_else(|| material.int_list("Color")),
    );
    let alpha = material_value_array(material.int_list("Alpha"));
    let texture_overlay = material.value("TextureOverlay").and_then(|value| {
        let value = value.trim();
        (!value.is_empty()).then(|| value.to_string())
    });
    let pxs_gfx = material.value("PXSGfx").and_then(|value| {
        let value = value.trim();
        (!value.is_empty()).then(|| value.to_string())
    });
    let pxs_gfx_rect = material_int_array(material.int_list("PXSGfxRt"));
    let pxs_gfx_size = material.int("PXSGfxSize").unwrap_or(pxs_gfx_rect[2]);
    clonk_frontend::MaterialRenderInfo::new(
        color,
        alpha,
        texture_overlay,
        material.int("OverlayType").unwrap_or(0),
        material.int("Density").unwrap_or(0),
    )
    .with_placement(material_render_placement(material))
    .with_pxs_graphics(pxs_gfx, pxs_gfx_rect, pxs_gfx_size)
}

/// Render sources in C4Game order: scenario-local first, then either the exact
/// synchronized NRT_Material vector or the offline installation search chain.
fn resolved_material_groups_with_paths(
    scenario_path: &Path,
    authoritative_external_groups: Option<&[Group]>,
    app_paths: Option<&AppPaths>,
) -> Vec<Group> {
    let mut groups = Vec::new();
    if let Ok(scenario) = open_group_path_for_folder_map(scenario_path) {
        if let Ok(Some(group)) = open_child_flexible(&scenario, Path::new("Material.c4g")) {
            groups.push(group);
        }
        if let Some(authoritative_external_groups) = authoritative_external_groups {
            groups.extend(authoritative_external_groups.iter().cloned());
            return groups;
        }
        let resolver = InstallDefinitionResolver::new(app_paths.cloned().map(Arc::new));
        match resolver.resolve_material_groups(&scenario) {
            Ok(external) => groups.extend(external),
            Err(error) => {
                tracing::error!(
                    %error,
                    path = %scenario_path.display(),
                    "failed to resolve the classic material resource chain"
                );
            }
        }
    }
    groups
}

#[derive(Clone)]
struct AdmittedMaterialGroup {
    group: Group,
    materials: bool,
    textures: bool,
}

fn read_group_file_case_insensitive(group: &Group, name: &str) -> Option<Vec<u8>> {
    group.read_file(name).ok().or_else(|| {
        group.entries().ok().and_then(|entries| {
            entries
                .into_iter()
                .find(|entry| {
                    !entry.is_directory
                        && entry
                            .relative_path
                            .file_name()
                            .and_then(|candidate| candidate.to_str())
                            .map(|candidate| candidate.eq_ignore_ascii_case(name))
                            .unwrap_or(false)
                })
                .and_then(|entry| group.read_file(entry.relative_path).ok())
        })
    })
}

fn admit_material_texture_names(group: &Group, inventory: &mut Vec<String>) -> usize {
    let entries = group.entries().unwrap_or_default();
    let mut admitted = 0;
    for extension in [b".png".as_slice(), b".bmp".as_slice()] {
        for entry in &entries {
            if entry.is_directory
                || entry.name_bytes.len() < extension.len()
                || !entry.name_bytes[entry.name_bytes.len() - extension.len()..]
                    .eq_ignore_ascii_case(extension)
            {
                continue;
            }
            let stem_end = entry
                .name_bytes
                .iter()
                .position(|byte| *byte == b'.')
                .unwrap_or(entry.name_bytes.len());
            let full_stem = clonk_script::c4_string_from_bytes(&entry.name_bytes[..stem_end]);
            if inventory
                .iter()
                .any(|stored| clonk_resources::material::c4_names_equal(stored, &full_stem))
            {
                continue;
            }
            if extension.eq_ignore_ascii_case(b".bmp")
                && group
                    .read_entry_bytes_exact(entry)
                    .ok()
                    .and_then(|bytes| clonk_resources::bitmap::IndexedBitmap::decode(&bytes).ok())
                    .is_none()
            {
                continue;
            }
            inventory.push(clonk_resources::material::truncate_c4m_name(&full_stem));
            admitted += 1;
        }
    }
    admitted
}

fn admitted_material_groups_with_paths(
    scenario_path: &Path,
    authoritative_external_groups: Option<&[Group]>,
    app_paths: Option<&AppPaths>,
) -> Vec<AdmittedMaterialGroup> {
    let groups = resolved_material_groups_with_paths(
        scenario_path,
        authoritative_external_groups,
        app_paths,
    );
    let mut admitted = Vec::new();
    let mut seen_materials = HashSet::new();
    let mut seen_textures = Vec::new();
    let mut load_materials = true;
    let mut load_textures = true;
    for (index, group) in groups.into_iter().enumerate() {
        if !load_materials && !load_textures {
            break;
        }
        let flags = if index == 0 {
            read_group_file_case_insensitive(&group, "TexMap.txt")
                .map(|source| clonk_resources::texmap::TextureMap::parse_bytes(&source))
                .unwrap_or_default()
        } else {
            let Some(source) = read_group_file_case_insensitive(&group, "TexMap.txt") else {
                break;
            };
            clonk_resources::texmap::TextureMap::parse_flags_bytes(&source)
        };
        let current_materials = load_materials;
        let current_textures = load_textures;
        let mut next_materials = flags.overload_materials;
        let mut next_textures = flags.overload_textures;
        if current_materials {
            let fresh = clonk_resources::MaterialLibrary::from_group(&group)
                .ok()
                .map(|library| {
                    library
                        .iter()
                        .filter(|material| {
                            seen_materials
                                .insert(clonk_resources::material::c4_name_key(material.name()))
                        })
                        .count()
                })
                .unwrap_or(0);
            if fresh == 0 {
                next_materials = true;
            }
        }
        if current_textures {
            let fresh = admit_material_texture_names(&group, &mut seen_textures);
            if fresh == 0 {
                next_textures = true;
            }
        }
        admitted.push(AdmittedMaterialGroup {
            group,
            materials: current_materials,
            textures: current_textures,
        });
        load_materials = next_materials;
        load_textures = next_textures;
    }
    admitted
}

pub(crate) fn load_material_render_info(
    scenario_path: &Path,
    authoritative_external_groups: Option<&[Group]>,
) -> HashMap<String, clonk_frontend::MaterialRenderInfo> {
    let app_paths = cached_app_paths().ok();
    load_material_render_info_with_paths(
        scenario_path,
        authoritative_external_groups,
        app_paths.as_deref(),
    )
}

pub(crate) fn load_material_render_info_with_paths(
    scenario_path: &Path,
    authoritative_external_groups: Option<&[Group]>,
    app_paths: Option<&AppPaths>,
) -> HashMap<String, clonk_frontend::MaterialRenderInfo> {
    let mut render_info = HashMap::new();
    for source in
        admitted_material_groups_with_paths(scenario_path, authoritative_external_groups, app_paths)
    {
        if !source.materials {
            continue;
        }
        let group = source.group;
        let Ok(library) = clonk_resources::MaterialLibrary::from_group(&group) else {
            continue;
        };
        for material in library.iter() {
            let name = clonk_resources::material::c4_name_key(material.name());
            render_info
                .entry(name)
                .or_insert_with(|| material_render_info(material));
        }
    }
    render_info
}

pub(crate) fn absorb_material_texture_group(
    group: &Group,
    textures: &mut HashMap<String, MaterialTextureSurface>,
    inventory: &mut Vec<String>,
) {
    let Ok(entries) = group.entries() else {
        return;
    };
    for extension in [b".png".as_slice(), b".bmp".as_slice()] {
        for entry in &entries {
            if entry.is_directory
                || entry.name_bytes.len() < extension.len()
                || !entry.name_bytes[entry.name_bytes.len() - extension.len()..]
                    .eq_ignore_ascii_case(extension)
            {
                continue;
            }
            let stem_end = entry
                .name_bytes
                .iter()
                .position(|byte| *byte == b'.')
                .unwrap_or(entry.name_bytes.len());
            let full_stem = clonk_script::c4_string_from_bytes(&entry.name_bytes[..stem_end]);
            if inventory
                .iter()
                .any(|stored| clonk_resources::material::c4_names_equal(stored, &full_stem))
            {
                continue;
            }
            let bytes = group.read_entry_bytes_exact(entry).ok();
            let is_bmp = extension.eq_ignore_ascii_case(b".bmp");
            let indexed = if is_bmp {
                let Some(mut bitmap) = bytes
                    .as_deref()
                    .and_then(|bytes| clonk_resources::bitmap::IndexedBitmap::decode(bytes).ok())
                else {
                    continue;
                };
                // CSurface8::AllowColor(0, 2, true) retains zero and folds
                // every other out-of-range palette index into the triplet.
                for index in &mut bitmap.indices {
                    if *index > 2 {
                        *index %= 3;
                    }
                }
                Some(bitmap)
            } else {
                None
            };
            let decoded = if is_bmp {
                None
            } else {
                bytes.as_deref().and_then(|bytes| {
                    image::load_from_memory_with_format(bytes, image::ImageFormat::Png).ok()
                })
            };
            let fixed_name = clonk_resources::material::truncate_c4m_name(&full_stem);
            inventory.push(fixed_name.clone());
            let fixed_key = clonk_resources::material::c4_name_key(&fixed_name);
            textures.remove(&fixed_key);
            if let Some(bitmap) = indexed {
                textures.insert(
                    fixed_key,
                    MaterialTextureSurface::surface8(bitmap.width, bitmap.height, bitmap.indices),
                );
                continue;
            }
            // GroupReadSurfacePNG admits a non-null Surface32 even if ReadPNG
            // failed. Retain an empty surface so lookup, overlay fallback and
            // graphical-PXS eligibility still see that native identity.
            let image = decoded.map_or_else(
                || ImageData::new(0, 0, Vec::new()),
                |decoded| {
                    let rgba = decoded.into_rgba8();
                    let (width, height) = rgba.dimensions();
                    let image = clonk_resources::GraphicsImage::new(width, height, rgba.into_raw());
                    ImageData::new(image.width(), image.height(), image.pixels().to_vec())
                },
            );
            textures.insert(fixed_key, MaterialTextureSurface::surface32(image));
        }
    }
}

pub(crate) fn load_scenario_material_textures(
    scenario_path: &Path,
    authoritative_external_groups: Option<&[Group]>,
) -> HashMap<String, MaterialTextureSurface> {
    let app_paths = cached_app_paths().ok();
    load_scenario_material_textures_with_paths(
        scenario_path,
        authoritative_external_groups,
        app_paths.as_deref(),
    )
}

pub(crate) fn load_scenario_material_textures_with_paths(
    scenario_path: &Path,
    authoritative_external_groups: Option<&[Group]>,
    app_paths: Option<&AppPaths>,
) -> HashMap<String, MaterialTextureSurface> {
    let mut textures = HashMap::new();
    let mut inventory = Vec::new();
    for source in
        admitted_material_groups_with_paths(scenario_path, authoritative_external_groups, app_paths)
    {
        if !source.textures {
            continue;
        }
        absorb_material_texture_group(&source.group, &mut textures, &mut inventory);
    }
    textures
}

fn try_materials_from_path(path: &Path) -> Result<MaterialSet, clonk_resources::MaterialError> {
    let group = Group::open(path)?;
    let library = clonk_resources::MaterialLibrary::from_group(&group)?;
    Ok(MaterialSet::from_resource_library(&library))
}

pub(crate) fn sky_render_state_from_config(config: &SkyConfig) -> SkyRenderState {
    let image = config
        .surface
        .as_ref()
        .map(|image| ImageData::from_arc(image.width(), image.height(), image.clone_pixels()));
    SkyRenderState::new(config.settings.clone(), image)
}

pub(crate) fn current_unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ClassicCalendarTime {
    pub(crate) day: i32,
    pub(crate) month: i32,
    pub(crate) year: i32,
    pub(crate) hour: i32,
    pub(crate) minute: i32,
}

#[derive(Clone, Copy)]
pub(crate) enum ClassicSaveDescriptionKind {
    Savegame,
    Record,
}

fn utc_calendar_time_now() -> ClassicCalendarTime {
    let now = OffsetDateTime::now_utc();
    ClassicCalendarTime {
        day: i32::from(now.day()),
        month: i32::from(u8::from(now.month())),
        year: now.year(),
        hour: i32::from(now.hour()),
        minute: i32::from(now.minute()),
    }
}

/// C4GameSave uses C `localtime` for save descriptions. Use the re-entrant
/// platform counterpart so the Rust console records the same local calendar
/// values without introducing another date/time dependency.
#[cfg(all(unix, target_pointer_width = "64"))]
pub(crate) fn classic_calendar_time_now() -> ClassicCalendarTime {
    #[repr(C)]
    struct Ctm {
        sec: i32,
        min: i32,
        hour: i32,
        mday: i32,
        mon: i32,
        year: i32,
        wday: i32,
        yday: i32,
        isdst: i32,
        gmtoff: i64,
        zone: *const std::ffi::c_char,
    }
    unsafe extern "C" {
        fn localtime_r(time: *const i64, result: *mut Ctm) -> *mut Ctm;
    }

    let timestamp = i64::try_from(current_unix_timestamp()).unwrap_or(i64::MAX);
    let mut raw = std::mem::MaybeUninit::<Ctm>::uninit();
    // SAFETY: `raw` points to writable storage for the platform's 64-bit
    // POSIX `struct tm`; `timestamp` remains alive for the call.
    let result = unsafe { localtime_r(&timestamp, raw.as_mut_ptr()) };
    if result.is_null() {
        return utc_calendar_time_now();
    }
    // SAFETY: a non-null `localtime_r` result initialized the supplied value.
    let raw = unsafe { raw.assume_init() };
    ClassicCalendarTime {
        day: raw.mday,
        month: raw.mon.saturating_add(1),
        year: raw.year.saturating_add(1900),
        hour: raw.hour,
        minute: raw.min,
    }
}

#[cfg(windows)]
pub(crate) fn classic_calendar_time_now() -> ClassicCalendarTime {
    #[repr(C)]
    struct Ctm {
        sec: i32,
        min: i32,
        hour: i32,
        mday: i32,
        mon: i32,
        year: i32,
        wday: i32,
        yday: i32,
        isdst: i32,
    }
    unsafe extern "C" {
        fn _localtime64_s(result: *mut Ctm, time: *const i64) -> i32;
    }

    let timestamp = i64::try_from(current_unix_timestamp()).unwrap_or(i64::MAX);
    let mut raw = std::mem::MaybeUninit::<Ctm>::uninit();
    // SAFETY: `_localtime64_s` receives valid output and timestamp pointers.
    if unsafe { _localtime64_s(raw.as_mut_ptr(), &timestamp) } != 0 {
        return utc_calendar_time_now();
    }
    // SAFETY: a zero return initialized the supplied value.
    let raw = unsafe { raw.assume_init() };
    ClassicCalendarTime {
        day: raw.mday,
        month: raw.mon.saturating_add(1),
        year: raw.year.saturating_add(1900),
        hour: raw.hour,
        minute: raw.min,
    }
}

#[cfg(not(any(all(unix, target_pointer_width = "64"), windows)))]
pub(crate) fn classic_calendar_time_now() -> ClassicCalendarTime {
    utc_calendar_time_now()
}

pub(crate) fn classic_rtf_charset_code(charset: &str) -> u8 {
    match charset.to_ascii_uppercase().as_str() {
        "SHIFTJIS" => 128,
        "HANGUL" => 129,
        "JOHAB" => 130,
        "CHINESEBIG5" => 136,
        "GREEK" => 161,
        "TURKISH" => 162,
        "VIETNAMESE" => 163,
        "HEBREW" => 177,
        "ARABIC" => 178,
        "BALTIC" => 186,
        "RUSSIAN" => 204,
        "THAI" => 222,
        "EASTEUROPE" => 238,
        _ => 0,
    }
}

pub(crate) fn developer_console_definition_description_path(
    module: &[u8],
    paths: Option<&AppPaths>,
) -> Vec<u8> {
    let Some(paths) = paths else {
        return module.to_vec();
    };
    // Config.AtExeRelativePath is a raw prefix comparison against exactly
    // General.ExePath (content_dir when present, otherwise install_root). It
    // intentionally does not require a path-component boundary.
    let root = paths.content_dir().unwrap_or(paths.install_root());
    let mut root = path_to_legacy_bytes(root);
    let separator = std::path::MAIN_SEPARATOR as u8;
    if !root.ends_with(&[separator]) {
        root.push(separator);
    }
    let matches = module.get(..root.len()).is_some_and(|prefix| {
        if cfg!(windows) {
            prefix.eq_ignore_ascii_case(&root)
        } else {
            prefix == root
        }
    });
    if matches {
        let mut offset = root.len();
        if module.get(offset) == Some(&separator) {
            offset += 1;
        }
        return module[offset..].to_vec();
    }
    module.to_vec()
}

pub(crate) fn append_description_player_names(
    output: &mut Vec<u8>,
    players: &[&clonk_engine::ControlPlayerInfoEntry],
) {
    for (index, player) in players.iter().enumerate() {
        if index != 0 {
            output.extend_from_slice(b", ");
        }
        output.extend_from_slice(control_player_effective_name(player));
    }
}

fn normalize_cpp_random_seed(seed: i32) -> u64 {
    u64::from(seed as u32)
}

/// C's `atoi` accepts leading ASCII whitespace and a signed decimal prefix,
/// returns zero when there are no digits, and ignores the remaining suffix.
pub(crate) fn legacy_atoi_i32(raw: &str) -> i32 {
    let bytes = raw.as_bytes();
    let mut index = 0;
    while bytes.get(index).is_some_and(u8::is_ascii_whitespace) {
        index += 1;
    }
    let negative = match bytes.get(index) {
        Some(b'-') => {
            index += 1;
            true
        }
        Some(b'+') => {
            index += 1;
            false
        }
        _ => false,
    };
    let digits_start = index;
    let mut value = 0_i64;
    while let Some(digit) = bytes
        .get(index)
        .and_then(|byte| byte.is_ascii_digit().then_some(*byte))
    {
        value = value
            .saturating_mul(10)
            .saturating_add(i64::from(digit - b'0'));
        index += 1;
    }
    if index == digits_start {
        return 0;
    }
    if negative {
        value = value.saturating_neg();
    }
    value as i32
}

pub(crate) fn resolve_offline_round_random_seed(
    parameter_seed: Option<i32>,
    unix_seconds: u64,
    pin: Option<&str>,
) -> u64 {
    parameter_seed.map_or_else(
        || match pin {
            Some(pin) if !pin.is_empty() => normalize_cpp_random_seed(legacy_atoi_i32(pin)),
            _ => u64::from(unix_seconds as u32),
        },
        normalize_cpp_random_seed,
    )
}

pub(crate) fn current_offline_round_random_seed(parameter_seed: Option<i32>) -> u64 {
    let pin = std::env::var_os("LC_PIN_SEED");
    let pin = pin.as_deref().map(|value| value.to_string_lossy());
    resolve_offline_round_random_seed(parameter_seed, current_unix_timestamp(), pin.as_deref())
}

pub(crate) fn format_startup_crew_birthday(seconds: i32) -> String {
    if seconds == 0 {
        return String::new();
    }
    OffsetDateTime::from_unix_timestamp(i64::from(seconds))
        .ok()
        .and_then(|datetime| {
            datetime
                .format(&format_description!("[day].[month].[year] [hour]:[minute]"))
                .ok()
        })
        .unwrap_or_default()
}

pub(crate) fn startup_rank_icon(sheet: &ImageData, rank: i32) -> Option<ImageData> {
    let size = sheet.height();
    if size == 0 {
        return None;
    }
    let phases = (sheet.width() / size).max(1);
    let phase = u32::try_from(rank.max(0)).unwrap_or_default() % phases;
    let source_x = phase * size;
    let mut pixels = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        let start = ((y * sheet.width() + source_x) * 4) as usize;
        let end = start + (size * 4) as usize;
        pixels.extend_from_slice(sheet.pixels().get(start..end)?);
    }
    Some(ImageData::new(size, size, pixels))
}

pub(crate) fn sanitize_save_label(label: &str) -> String {
    let mut result = String::new();
    let mut last_was_separator = false;
    for ch in label.chars() {
        if result.len() >= 64 {
            break;
        }
        if ch.is_ascii_alphanumeric() {
            result.push(ch);
            last_was_separator = false;
        } else if (ch.is_ascii_whitespace() || matches!(ch, '-' | '_'))
            && !last_was_separator
            && !result.is_empty()
        {
            result.push('_');
            last_was_separator = true;
        }
    }
    let trimmed = result.trim_matches('_');
    if trimmed.is_empty() {
        "save".to_string()
    } else {
        trimmed.to_string()
    }
}

pub(crate) fn unique_save_path(dir: &Path, base: &str) -> PathBuf {
    let mut index = 0u32;
    loop {
        let candidate = if index == 0 {
            dir.join(format!("{}.lcsave", base))
        } else {
            dir.join(format!("{}_{:02}.lcsave", base, index))
        };
        if !candidate.exists() {
            return candidate;
        }
        index = index.saturating_add(1);
    }
}

pub(crate) fn next_recording_index(dir: &Path) -> io::Result<u32> {
    if !dir.exists() {
        return Ok(1);
    }
    let count = fs::read_dir(dir)?.try_fold(0_u32, |count, entry| {
        let entry = entry?;
        let file_name = entry.file_name();
        let is_scenario = Path::new(&file_name)
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("c4s"));
        Ok::<_, io::Error>(count.saturating_add(u32::from(is_scenario)))
    })?;
    Ok(count.saturating_add(1))
}

pub(crate) fn sanitize_record_name(raw: &str) -> String {
    let without_digits = raw.trim_end_matches(|character: char| character.is_ascii_digit());
    if without_digits.is_empty() && !raw.is_empty() {
        // C4Record's backwards pointer loop stops at the first byte, so an
        // all-numeric basename retains its leading character.
        raw.chars().next().into_iter().collect()
    } else {
        without_digits.to_string()
    }
}

pub(crate) fn encode_surface_to_png(surface: &Surface) -> Result<Vec<u8>> {
    encode_rgba_png(surface.width(), surface.height(), surface.pixels())
}

pub(crate) fn encode_presented_save_thumbnail(
    width: u32,
    height: u32,
    rgba: &[u8],
) -> Result<Vec<u8>> {
    anyhow::ensure!(width != 0 && height != 0, "save thumbnail source is empty");
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .context("save thumbnail source dimensions overflow")?;
    anyhow::ensure!(
        rgba.len() == expected,
        "save thumbnail source has {} bytes, expected {expected}",
        rgba.len()
    );
    // A 2-tap bilinear sample at >19x minification is point sampling: thin
    // scenery aliased into noise instead of averaging down to a tint. Average
    // every source pixel that lands in a destination cell instead.
    let reduced = clonk_graphics::surface::downsample_rgba_box(
        rgba,
        width,
        height,
        SAVE_THUMBNAIL_WIDTH,
        SAVE_THUMBNAIL_HEIGHT,
    )
    .context("save thumbnail source is not a valid RGBA frame")?;
    encode_rgba_png(SAVE_THUMBNAIL_WIDTH, SAVE_THUMBNAIL_HEIGHT, &reduced)
}

pub(crate) fn encode_rgba_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>> {
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .context("RGBA PNG dimensions overflow")?;
    anyhow::ensure!(
        rgba.len() == expected,
        "RGBA PNG frame has {} bytes, expected {expected}",
        rgba.len()
    );
    let mut buffer = Vec::new();
    {
        let mut encoder = Encoder::new(&mut buffer, width, height);
        encoder.set_color(ColorType::Rgba);
        encoder.set_depth(BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .context("failed to initialise PNG encoder")?;
        writer
            .write_image_data(rgba)
            .context("failed to encode PNG surface")?;
        writer.finish().context("failed to finish PNG encoding")?;
    }
    Ok(buffer)
}

fn screenshot_directories(paths: Option<&AppPaths>) -> (PathBuf, PathBuf) {
    let Some(paths) = paths else {
        let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        return (root.join("Screenshots"), root);
    };
    (paths.screenshot_dir(), paths.install_root().to_path_buf())
}

pub(crate) fn next_screenshot_path(directory: &Path) -> PathBuf {
    let mut index = 1_u64;
    loop {
        let candidate = directory.join(format!("Screenshot{index:03}.png"));
        if !candidate.exists() {
            return candidate;
        }
        index = index.saturating_add(1);
    }
}

pub(crate) fn encode_screenshot_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>> {
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .context("screenshot dimensions overflow")?;
    anyhow::ensure!(
        rgba.len() == expected,
        "screenshot frame has {} bytes, expected {expected}",
        rgba.len()
    );
    let mut rgb = Vec::with_capacity(expected / 4 * 3);
    for pixel in rgba.chunks_exact(4) {
        rgb.extend_from_slice(&pixel[..3]);
    }

    let mut buffer = Vec::new();
    {
        let mut encoder = Encoder::new(&mut buffer, width, height);
        encoder.set_color(ColorType::Rgb);
        encoder.set_depth(BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .context("failed to initialise screenshot PNG encoder")?;
        writer
            .write_image_data(&rgb)
            .context("failed to encode screenshot PNG")?;
        writer.finish().context("failed to finish screenshot PNG")?;
    }
    Ok(buffer)
}

pub(crate) fn prepare_numbered_screenshot_path(paths: Option<&AppPaths>) -> (PathBuf, Result<()>) {
    let (preferred, fallback) = screenshot_directories(paths);
    // `C4Config::AtScreenshotPath` attempts one directory creation and falls
    // back to ExePath when it fails, rather than building a tree
    // (C4Config.cpp:1381-1390).
    let directory = crate::output_folders::resolve_screenshot_directory(&preferred, &fallback);
    let result = if directory == fallback && preferred != fallback {
        tracing::warn!(
            path = %preferred.display(),
            "could not create the screenshot folder; falling back to the install root"
        );
        fs::create_dir_all(&fallback).with_context(|| {
            format!(
                "failed to create screenshot fallback at {}",
                fallback.display()
            )
        })
    } else {
        Ok(())
    };
    let path = next_screenshot_path(&directory);
    (path, result)
}

pub(crate) fn write_screenshot(path: &Path, width: u32, height: u32, rgba: &[u8]) -> Result<()> {
    let png = encode_screenshot_png(width, height, rgba)?;
    let mut file = File::create(path)
        .with_context(|| format!("failed to create screenshot at {}", path.display()))?;
    file.write_all(&png)
        .with_context(|| format!("failed to write screenshot at {}", path.display()))?;
    file.flush()
        .with_context(|| format!("failed to flush screenshot at {}", path.display()))?;
    Ok(())
}

pub(crate) fn scaled_screenshot_extent(extent: u32, scale: f32) -> Result<u32> {
    anyhow::ensure!(
        scale.is_finite() && scale > 0.0,
        "invalid screenshot scale {scale}"
    );
    let scaled = (f64::from(extent) * f64::from(scale)).ceil();
    anyhow::ensure!(
        scaled >= 1.0 && scaled <= f64::from(u32::MAX),
        "scaled screenshot extent is out of range"
    );
    Ok(scaled as u32)
}

pub(crate) fn load_save_entry(path: &Path) -> Result<SaveEntry> {
    let file =
        File::open(path).with_context(|| format!("failed to open save file {}", path.display()))?;
    let header: SavedGameHeader = serde_json::from_reader(file)
        .with_context(|| format!("failed to parse save metadata from {}", path.display()))?;
    let is_quick_save = path
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.eq_ignore_ascii_case(QUICK_SAVE_FILE))
        .unwrap_or(false);
    let display_name = header
        .user_label
        .clone()
        .filter(|label| !label.trim().is_empty())
        .or_else(|| {
            if is_quick_save {
                Some("Quick Save".to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| header.scenario.title.clone());
    Ok(SaveEntry {
        display_name,
        saved_at_seconds: header.saved_at_seconds,
        path: path.to_path_buf(),
    })
}

/// `StdCompilerINIRead::Boolean` (StdCompiler.cpp:692-715): a leading `1`/`0`
/// not followed by another digit, or a case-sensitive `true`/`false` prefix.
/// Anything else signals not-found, so the caller keeps the field's adapted
/// default. No trimming, no case folding, and no `yes`/`on` aliases — C++ reads
/// the raw value in place.
pub(crate) fn parse_native_config_bool(raw: &str) -> Option<bool> {
    let value = raw.as_bytes();
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

/// Callers that have no adapted default of their own. C4Config's own fields
/// keep theirs through [`parse_native_config_bool`].
pub(crate) fn parse_config_bool(raw: &str) -> bool {
    parse_native_config_bool(raw).unwrap_or(false)
}

const DEFAULT_IRC_SERVER: &str = "irc.euirc.net";
const DEFAULT_IRC_CHANNELS: &str = "#clonken,#legacyclonk";

/// Typed projection of C4ConfigIRC plus the startup disclaimer preference.
/// The password deliberately is not configuration-backed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct IrcSettings {
    pub(crate) server: String,
    pub(crate) nick: String,
    pub(crate) real_name: String,
    pub(crate) channel: String,
    pub(crate) hide_dangerous_warning: bool,
}

impl Default for IrcSettings {
    fn default() -> Self {
        Self {
            server: DEFAULT_IRC_SERVER.to_string(),
            nick: String::new(),
            real_name: String::new(),
            channel: DEFAULT_IRC_CHANNELS.to_string(),
            hide_dangerous_warning: false,
        }
    }
}

impl IrcSettings {
    pub(crate) fn from_config(config: &[u8]) -> Self {
        let text = |section, key| {
            native_config_text(config, section, key).map(|value| {
                clonk_resources::decode_legacy_system_text(&clonk_script::c4_string_bytes(&value))
            })
        };
        let server = text("IRC", "Server2")
            .as_deref()
            .unwrap_or(DEFAULT_IRC_SERVER)
            .to_string();
        let mut nick = text("IRC", "Nick")
            .as_deref()
            .unwrap_or_default()
            .to_string();
        if nick.is_empty() {
            nick = text("Network", "Nick")
                .as_deref()
                .unwrap_or_default()
                .to_string();
        }
        let real_name = text("IRC", "RealName")
            .as_deref()
            .unwrap_or_default()
            .to_string();
        let channel = text("IRC", "Channel")
            .as_deref()
            .unwrap_or(DEFAULT_IRC_CHANNELS)
            .to_string();
        let hide_dangerous_warning = text("Startup", "HideMsgIRCDangerous")
            .as_deref()
            .and_then(parse_classic_loader_bool)
            .unwrap_or(false);
        Self {
            server,
            nick,
            real_name,
            channel,
            hide_dangerous_warning,
        }
    }

    pub(crate) fn login(&self) -> clonk_frontend::startup_netdlg::NetDlgChatLogin {
        clonk_frontend::startup_netdlg::NetDlgChatLogin {
            server: self.server.clone(),
            nick: self.nick.clone(),
            password: String::new(),
            real_name: self.real_name.clone(),
            channel: self.channel.clone(),
        }
    }
}

pub(crate) fn load_irc_settings(paths: Option<&AppPaths>) -> IrcSettings {
    IrcSettings::from_config(&load_native_config_bytes(paths))
}

pub(crate) fn load_startup_alphabetical_sorting(paths: Option<&AppPaths>) -> bool {
    paths
        .and_then(|paths| Config::load(paths.config_file()).ok())
        .and_then(|config| {
            config
                .get_in(Some("Startup"), "AlphabeticalSorting")
                .and_then(parse_native_config_bool)
        })
        .unwrap_or(false)
}

pub(crate) fn load_startup_last_portrait_folder_index(paths: Option<&AppPaths>) -> Option<usize> {
    native_config_text(
        &load_native_config_bytes(paths),
        "Startup",
        "LastPortraitFolderIdx",
    )
    .as_deref()
    .and_then(parse_classic_loader_i32)
    .and_then(|index| usize::try_from(index).ok())
}

pub(crate) fn repair_rust_truncated_masterserver_urls(config_path: &Path) -> io::Result<bool> {
    let mut config = Config::load(config_path)?;
    let mut repaired = false;
    for key in ["ServerAddress", "AlternateServerAddress"] {
        if config
            .get_in(Some("Network"), key)
            .is_some_and(|value| matches!(value.trim(), "http:" | "https:"))
        {
            config.set_in(Some("Network"), key, OFFICIAL_LEAGUE_SERVER);
            repaired = true;
        }
    }
    if repaired {
        save_config_preserving_native_general_booleans(&config, config_path, None, None)?;
    }
    Ok(repaired)
}

pub(crate) fn load_display_flags(paths: Option<&AppPaths>) -> DisplayFlags {
    let mut flags = DisplayFlags::default();
    let Some(config) = paths.and_then(|paths| Config::load(paths.config_file()).ok()) else {
        return flags;
    };
    let graphics_bool = |key: &str, fallback: bool| {
        config
            .get_in(Some("Graphics"), key)
            .and_then(parse_native_config_bool)
            .unwrap_or(fallback)
    };
    let general_bool = |key: &str, fallback: bool| {
        config
            .get_in(Some("General"), key)
            .and_then(parse_native_config_bool)
            .unwrap_or(fallback)
    };
    flags.player_names = graphics_bool("ShowCrewNames", flags.player_names);
    flags.clonk_names = graphics_bool("ShowCrewCNames", flags.clonk_names);
    flags.portraits = graphics_bool("ShowPortraits", flags.portraits);
    flags.show_commands = graphics_bool("ShowCommands", flags.show_commands);
    flags.show_command_keys = graphics_bool("ShowCommandKeys", flags.show_command_keys);
    flags.show_player_hud_always =
        graphics_bool("ShowPlayerHUDAlways", flags.show_player_hud_always);
    flags.splitscreen_dividers = config
        .get_in(Some("Graphics"), "SplitscreenDividers")
        .and_then(|value| value.trim().parse::<i32>().ok())
        .map(|value| value != 0)
        .unwrap_or(flags.splitscreen_dividers);
    flags.fire_particles = graphics_bool("FireParticles", flags.fire_particles);
    flags.clock = graphics_bool("ShowClock", flags.clock);
    flags.fps = general_bool("FPS", flags.fps);
    // C++ keeps the raw configured value and clamps it only where the camera
    // divides by it (C4Config.cpp:381-388; C4Viewport.cpp:1195-1207).
    flags.scroll_smooth = config
        .get_in(Some("General"), "ScrollSmooth")
        .and_then(|value| value.trim().parse::<i32>().ok())
        .unwrap_or(flags.scroll_smooth);
    flags.white_chat = general_bool("UseWhiteIngameChat", flags.white_chat);
    if let Some(mode) = config.get_in(Some("Graphics"), "UpperBoard") {
        flags.upper_board = match mode.trim().to_ascii_lowercase().as_str() {
            "hide" => UpperBoardMode::Hide,
            "small" => UpperBoardMode::Small,
            "mini" => UpperBoardMode::Mini,
            _ => UpperBoardMode::Full,
        };
    }
    flags
}

pub(crate) fn frontend_upper_board_mode(
    mode: UpperBoardMode,
) -> clonk_frontend::hud::UpperBoardMode {
    match mode {
        UpperBoardMode::Hide => clonk_frontend::hud::UpperBoardMode::Hide,
        UpperBoardMode::Full => clonk_frontend::hud::UpperBoardMode::Full,
        UpperBoardMode::Small => clonk_frontend::hud::UpperBoardMode::Small,
        UpperBoardMode::Mini => clonk_frontend::hud::UpperBoardMode::Mini,
    }
}

pub(crate) fn load_white_lobby_chat(paths: Option<&AppPaths>) -> bool {
    paths
        .and_then(|paths| Config::load(paths.config_file()).ok())
        .and_then(|config| {
            config
                .get_in(Some("General"), "UseWhiteLobbyChat")
                .and_then(parse_native_config_bool)
        })
        .unwrap_or(false)
}

pub(crate) fn load_graphics_smoke_level(paths: Option<&AppPaths>) -> i32 {
    paths
        .and_then(|paths| Config::load(paths.config_file()).ok())
        .and_then(|config| {
            config
                .get_in(Some("Graphics"), "SmokeLevel")
                .and_then(|value| value.trim().parse::<i32>().ok())
        })
        .unwrap_or(clonk_engine::DEFAULT_SMOKE_LEVEL)
}

pub(crate) fn load_graphics_color_animation(paths: Option<&AppPaths>) -> bool {
    let Some(config) = paths.and_then(|paths| Config::load(paths.config_file()).ok()) else {
        return false;
    };
    ["ColorAnimation", "Shader"].into_iter().all(|key| {
        config
            .get_in(Some("Graphics"), key)
            .and_then(parse_native_config_bool)
            .unwrap_or(false)
    })
}

pub(crate) fn load_show_folder_maps(paths: Option<&AppPaths>) -> bool {
    paths
        .and_then(|paths| Config::load(paths.config_file()).ok())
        .and_then(|config| {
            config
                .get_in(Some("Graphics"), "ShowFolderMaps")
                .and_then(parse_native_config_bool)
        })
        .unwrap_or(true)
}

pub(crate) fn load_recording_flag(paths: Option<&AppPaths>) -> bool {
    let Some(paths) = paths else {
        return false;
    };
    let config_path = paths.config_file();
    match Config::load(&config_path) {
        Ok(config) => config
            .get_in(Some("General"), "Record")
            .and_then(parse_native_config_bool)
            .unwrap_or(false),
        Err(err) => {
            if err.kind() != io::ErrorKind::NotFound {
                tracing::warn!(
                    error = %err,
                    path = %config_path.display(),
                    "failed to read record setting from config"
                );
            }
            false
        }
    }
}

pub(crate) fn parse_gamepad_gui_control(raw: &str) -> bool {
    match raw.trim().parse::<i32>() {
        Ok(value) => value != 0,
        Err(error) => {
            tracing::warn!(
                value = raw,
                %error,
                "invalid Controls.GamepadGuiControl; disabling GUI gamepad input"
            );
            false
        }
    }
}

pub(crate) fn load_gamepad_gui_control(paths: Option<&AppPaths>) -> bool {
    paths
        .and_then(|paths| Config::load(paths.config_file()).ok())
        .and_then(|config| {
            config
                .get_in(Some("Controls"), "GamepadGuiControl")
                .map(parse_gamepad_gui_control)
        })
        .unwrap_or(false)
}

/// Native `Config.General.NoCrew` (`C4Config.cpp:384`) drives the scen-sel
/// fair-crew icon. The field name is `FairCrew`, but the serialized key is not.
pub(crate) fn load_fair_crew_flag(paths: Option<&AppPaths>) -> bool {
    let config = load_native_config_bytes(paths);
    clonk_app_netplay::configured_native_boolean(&config, "General", "NoCrew").unwrap_or(false)
}

/// Native `C4Application::DoInit` constructs `C4GamePadControl` only when
/// `Config.General.GamepadEnabled` is true. `C4ConfigGeneral` defaults the
/// field on, so a missing or malformed serialized value retains that default.
pub(crate) fn configured_gamepads_enabled(config: &[u8]) -> bool {
    clonk_app_netplay::configured_native_boolean(config, "General", "GamepadEnabled")
        .unwrap_or(true)
}

pub(crate) fn load_gamepads_enabled(paths: Option<&AppPaths>) -> bool {
    configured_gamepads_enabled(&load_native_config_bytes(paths))
}

pub(crate) fn save_config_preserving_native_general_booleans(
    config: &Config,
    path: &Path,
    updated_gamepads_enabled: Option<bool>,
    updated_always_debug: Option<bool>,
) -> io::Result<()> {
    // The UTF-8 convenience writer emits `Key = value`, but native Boolean
    // values must begin immediately after `=`. Preserve or explicitly replace
    // these process-start flags whenever Rust rewrites the complete file.
    let existing = if updated_gamepads_enabled.is_none() || updated_always_debug.is_none() {
        match fs::read(path) {
            Ok(config) => Some(config),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        }
    } else {
        None
    };
    let gamepads_enabled = updated_gamepads_enabled.or_else(|| {
        existing.as_deref().and_then(|config| {
            clonk_app_netplay::configured_native_boolean(config, "General", "GamepadEnabled")
        })
    });
    let always_debug = updated_always_debug.or_else(|| {
        existing.as_deref().and_then(|config| {
            clonk_app_netplay::configured_native_boolean(config, "General", "DebugMode")
        })
    });
    config.save(path)?;
    let mut updates = Vec::new();
    if let Some(enabled) = gamepads_enabled {
        updates.push((
            "GamepadEnabled",
            clonk_app_netplay::NativeConfigValue::RawAscii(if enabled { "true" } else { "false" }),
        ));
    }
    if let Some(enabled) = always_debug {
        updates.push((
            "DebugMode",
            clonk_app_netplay::NativeConfigValue::RawAscii(if enabled { "true" } else { "false" }),
        ));
    }
    if !updates.is_empty() {
        let saved = fs::read(path)?;
        let updated =
            clonk_app_netplay::update_configured_native_values(&saved, "General", &updates)?;
        fs::write(path, updated)?;
    }
    Ok(())
}

pub(crate) fn persist_dirty_gamepad_axis_calibration(
    paths: &AppPaths,
    bindings: &mut GamepadBindings,
) -> io::Result<()> {
    if !bindings.axis_calibration_dirty() {
        return Ok(());
    }
    let path = paths.config_file();
    let config = match fs::read(&path) {
        Ok(config) => config,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error),
    };
    let config = update_dirty_gamepad_axis_calibration_config(&config, bindings)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, config)?;
    bindings.mark_axis_calibration_persisted();
    Ok(())
}

pub(crate) fn update_dirty_gamepad_axis_calibration_config(
    config: &[u8],
    bindings: &GamepadBindings,
) -> io::Result<Vec<u8>> {
    if !bindings.axis_calibration_dirty() {
        return Ok(config.to_vec());
    }
    let mut config = config.to_vec();
    for (gamepad, calibrations) in bindings.axis_calibrations().iter().enumerate() {
        let mut fields = Vec::with_capacity(calibrations.len() * 3);
        for (axis, calibration) in calibrations.iter().enumerate() {
            fields.push((format!("Axis{axis}Min"), calibration.min.to_string()));
            fields.push((format!("Axis{axis}Max"), calibration.max.to_string()));
            fields.push((
                format!("Axis{axis}Calibrated"),
                calibration.calibrated.to_string(),
            ));
        }
        let updates = fields
            .iter()
            .map(|(key, value)| {
                (
                    key.as_str(),
                    clonk_app_netplay::NativeConfigValue::RawAscii(value.as_str()),
                )
            })
            .collect::<Vec<_>>();
        config = clonk_app_netplay::update_configured_native_values(
            &config,
            &format!("Gamepad{gamepad}"),
            &updates,
        )?;
    }
    Ok(config)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StartupDefinitionPaths {
    pub(crate) selector_root: PathBuf,
    /// Physical prefix used by `C4Game::OpenScenario`. Its trailing separator
    /// is significant because C++ concatenates it with each module verbatim.
    pub(crate) active_custom_root: Option<PathBuf>,
}

pub(crate) fn startup_definition_paths(paths: &AppPaths) -> io::Result<StartupDefinitionPaths> {
    let configured_text = match fs::read(paths.config_file()) {
        Ok(config) => {
            native_config_text(&config, "General", "DefinitionPath").filter(|path| !path.is_empty())
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let configured_bytes = configured_text
        .as_ref()
        .map(|path| clonk_script::c4_string_bytes(path));
    let configured_path = configured_bytes
        .as_ref()
        .map(|path| path_from_group_name_bytes(&normalize_legacy_path_bytes(path.clone())));
    // AppPaths maps an installed C++ ExePath data layout to `content/` in a
    // source checkout; packaged layouts fall back to the install root.
    let exe_data_root = paths.content_dir().unwrap_or(paths.install_root());
    let executable_prefix = path_with_trailing_native_separator(exe_data_root);
    let selector_root = configured_text
        .as_ref()
        // C4Config::AtExePath is literal concatenation even when DefinitionPath
        // starts with a root or drive marker.
        .map(|path| {
            concatenate_legacy_path(&executable_prefix, &clonk_script::c4_string_bytes(path))
        })
        .unwrap_or_else(|| exe_data_root.to_path_buf());
    let active_custom_root = configured_bytes
        .as_deref()
        .and_then(definition_path_directory_probe)
        .and_then(|probe| {
            let process_resolved = legacy_definition_path_uses_process_resolution(&probe);
            let probe_path = if process_resolved {
                path_from_group_name_bytes(&probe)
            } else {
                let mut full = path_to_legacy_bytes(&executable_prefix);
                full.extend_from_slice(&probe);
                path_from_group_name_bytes(&full)
            };
            probe_path.is_dir().then(|| {
                if process_resolved {
                    configured_path
                        .clone()
                        .expect("configured bytes have a configured path")
                } else {
                    selector_root.clone()
                }
            })
        });
    Ok(StartupDefinitionPaths {
        selector_root,
        active_custom_root,
    })
}

pub(crate) fn definition_path_directory_probe(path: &[u8]) -> Option<Vec<u8>> {
    let mut path = path.to_vec();
    if path.last() == Some(&(std::path::MAIN_SEPARATOR as u8)) {
        path.pop();
    }
    if path.last().is_some_and(|byte| matches!(byte, b'/' | b'\\')) {
        path.pop();
    }
    (!path.is_empty()).then_some(path)
}

fn legacy_definition_path_uses_process_resolution(path: &[u8]) -> bool {
    if cfg!(windows) {
        path.first()
            .is_some_and(|byte| matches!(byte, b'/' | b'\\'))
            || path.get(1) == Some(&b':')
    } else {
        path.first() == Some(&b'/')
    }
}

pub(crate) fn enumerate_startup_definition_files(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if entry
            .path()
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("c4d"))
        {
            files.push(entry.path());
        }
    }
    Ok(files)
}

pub(crate) fn definition_selector_entries(
    root: &Path,
) -> io::Result<Vec<clonk_frontend::definition_sel::DefinitionSelEntry>> {
    enumerate_startup_definition_files(root).map(|paths| {
        paths
            .into_iter()
            .filter_map(|path| {
                let filename = path.file_name()?.to_string_lossy().into_owned();
                Some(clonk_frontend::definition_sel::DefinitionSelEntry::new(
                    path.to_string_lossy().into_owned(),
                    filename,
                ))
            })
            .collect()
    })
}

pub(crate) fn scenario_fixed_definition_modules(scenario: &FrontendScenario) -> Vec<String> {
    let mut modules = if scenario.local_only.unwrap_or(false) {
        Vec::new()
    } else {
        scenario.definition_modules.clone()
    };
    if modules.is_empty() {
        modules.push("Objects.c4d".to_string());
    }
    modules
}

pub(crate) fn preflight_offline_startup(
    path: &Path,
) -> Result<clonk_engine::scenario::OfflineScenarioStartupPreflight, ScenarioError> {
    let group = open_group_path_for_folder_map(path)?;
    Scenario::preflight_offline_startup_from_group(&group)
}

pub(crate) fn load_scenario_with_definition_load(
    path: &Path,
    resolver: &InstallDefinitionResolver,
    languages: &[String],
    definition_load: &ScenarioDefinitionLoad,
) -> Result<Scenario, ScenarioError> {
    load_scenario_with_definition_load_and_progress(
        path,
        resolver,
        languages,
        definition_load,
        |_, _| {},
    )
}

pub(crate) fn load_scenario_with_definition_load_and_progress<F>(
    path: &Path,
    resolver: &InstallDefinitionResolver,
    languages: &[String],
    definition_load: &ScenarioDefinitionLoad,
    mut progress: F,
) -> Result<Scenario, ScenarioError>
where
    F: FnMut(i32, &'static str),
{
    let group = open_group_path_for_folder_map(path)?;
    match definition_load {
        ScenarioDefinitionLoad::Fixed {
            modules,
            definition_root,
        } => Scenario::load_from_group_with_languages_and_definition_selection_and_prefix_and_progress(
            &group,
            resolver,
            languages,
            &[] as &[String],
            Some(modules.as_slice()),
            definition_root.as_deref(),
            &mut progress,
        ),
        ScenarioDefinitionLoad::Seed {
            modules,
            definition_root,
        } => Scenario::load_from_group_with_languages_and_definition_selection_and_prefix_and_progress(
            &group,
            resolver,
            languages,
            modules,
            None,
            definition_root.as_deref(),
            &mut progress,
        ),
    }
}

pub(crate) fn load_scenario_with_definition_load_and_startup_player_count(
    path: &Path,
    resolver: &InstallDefinitionResolver,
    languages: &[String],
    definition_load: &ScenarioDefinitionLoad,
    startup_player_count: i32,
) -> Result<Scenario, ScenarioError> {
    load_scenario_with_definition_load_and_seed_and_startup_player_count(
        path,
        resolver,
        languages,
        definition_load,
        0,
        startup_player_count,
    )
}

fn load_scenario_with_definition_load_and_seed_and_startup_player_count(
    path: &Path,
    resolver: &InstallDefinitionResolver,
    languages: &[String],
    definition_load: &ScenarioDefinitionLoad,
    random_seed: u64,
    startup_player_count: i32,
) -> Result<Scenario, ScenarioError> {
    load_scenario_with_definition_load_and_seed_and_startup_player_count_and_progress(
        path,
        resolver,
        languages,
        definition_load,
        random_seed,
        startup_player_count,
        |_, _| {},
    )
}

pub(crate) fn load_scenario_with_definition_load_and_seed_and_startup_player_count_and_progress<F>(
    path: &Path,
    resolver: &InstallDefinitionResolver,
    languages: &[String],
    definition_load: &ScenarioDefinitionLoad,
    random_seed: u64,
    startup_player_count: i32,
    mut progress: F,
) -> Result<Scenario, ScenarioError>
where
    F: FnMut(i32, &'static str),
{
    let group = open_group_path_for_folder_map(path)?;
    match definition_load {
        ScenarioDefinitionLoad::Fixed {
            modules,
            definition_root,
        } => Scenario::load_from_group_with_languages_and_seed_and_definition_selection_and_startup_player_count_and_prefix_and_progress(
            &group,
            resolver,
            languages,
            random_seed,
            &[] as &[String],
            Some(modules.as_slice()),
            definition_root.as_deref(),
            startup_player_count,
            &mut progress,
        ),
        ScenarioDefinitionLoad::Seed {
            modules,
            definition_root,
        } => Scenario::load_from_group_with_languages_and_seed_and_definition_selection_and_startup_player_count_and_prefix_and_progress(
            &group,
            resolver,
            languages,
            random_seed,
            modules,
            None,
            definition_root.as_deref(),
            startup_player_count,
            &mut progress,
        ),
    }
}

const AUTHORITATIVE_WORLDGEN_SEED_ATTEMPTS: u32 = 256;

pub(crate) fn load_fresh_scenario_with_valid_generated_landscape<F>(
    path: &Path,
    resolver: &InstallDefinitionResolver,
    languages: &[String],
    definition_load: &ScenarioDefinitionLoad,
    initial_random_seed: u64,
    startup_player_count: i32,
    mut progress: F,
) -> std::result::Result<(Scenario, u64), String>
where
    F: FnMut(i32, &'static str),
{
    let mut random_seed = u64::from(initial_random_seed as u32);
    for rejected in 0..AUTHORITATIVE_WORLDGEN_SEED_ATTEMPTS {
        let scenario =
            load_scenario_with_definition_load_and_seed_and_startup_player_count_and_progress(
                path,
                resolver,
                languages,
                definition_load,
                random_seed,
                startup_player_count,
                &mut progress,
            )
            .map_err(|error| error.to_string())?;
        if !scenario.generated_landscape_requires_seed_retry() {
            if rejected != 0 {
                tracing::info!(
                    initial_random_seed = initial_random_seed as u32,
                    accepted_random_seed = random_seed as u32,
                    rejected_seeds = rejected,
                    "selected a contained SkyParcour landscape"
                );
            }
            return Ok((scenario, random_seed));
        }

        tracing::debug!(
            rejected_random_seed = random_seed as u32,
            "SkyParcour generation exposed movable Water"
        );
        progress(92, "Retrying malformed generated landscape");
        random_seed = u64::from((random_seed as u32).wrapping_add(1));
    }

    Err(format!(
        "no contained SkyParcour landscape was generated in {} attempts starting at seed {}",
        AUTHORITATIVE_WORLDGEN_SEED_ATTEMPTS, initial_random_seed as u32
    ))
}

pub(crate) fn load_options_program_state(
    paths: Option<&AppPaths>,
    resources: Option<&HashMap<String, String>>,
) -> clonk_frontend::startup_options_dlg::ProgramSheetState {
    let mut state = clonk_frontend::startup_options_dlg::ProgramSheetState::default();
    let config = paths.and_then(|paths| Config::load(paths.config_file()).ok());
    state.font_face = config
        .as_ref()
        .and_then(|config| config.get_in(Some("General"), "FontName"))
        .filter(|face| !face.is_empty())
        .unwrap_or("Endeavour")
        .to_string();
    state.font_size = config
        .as_ref()
        .and_then(|config| config.get_in(Some("General"), "FontSize"))
        .and_then(|size| size.trim().parse::<i32>().ok())
        .filter(|size| *size > 0)
        .unwrap_or(14)
        .to_string();
    state.white_chat_ingame = config
        .as_ref()
        .and_then(|config| config.get_in(Some("General"), "UseWhiteIngameChat"))
        .and_then(parse_native_config_bool)
        .unwrap_or(false);
    state.white_chat_lobby = config
        .as_ref()
        .and_then(|config| config.get_in(Some("General"), "UseWhiteLobbyChat"))
        .and_then(parse_native_config_bool)
        .unwrap_or(false);
    state.show_log_timestamps = config
        .as_ref()
        .and_then(|config| config.get_in(Some("General"), "ShowLogTimestamps"))
        .and_then(parse_native_config_bool)
        .unwrap_or(false);
    state.preloading = config
        .as_ref()
        .and_then(|config| config.get_in(Some("General"), "Preloading"))
        .and_then(parse_native_config_bool)
        .unwrap_or(!cfg!(target_os = "macos"));
    let fair_crew_strength = config
        .as_ref()
        .and_then(|config| config.get_in(Some("General"), "DefCrewStrength"))
        .and_then(|strength| strength.trim().parse::<i32>().ok())
        .unwrap_or(1_000);
    state.set_fair_crew_strength(fair_crew_strength);

    let language = config
        .as_ref()
        .and_then(|config| config.get_in(Some("General"), "Language"))
        .unwrap_or(&state.language)
        .to_string();
    let language_ex = paths
        .and_then(AppPaths::language_override)
        .or_else(|| {
            config
                .as_ref()
                .and_then(|config| config.get_in(Some("General"), "LanguageEx"))
        })
        .unwrap_or(&state.language_ex)
        .to_string();
    let language_infos = paths
        .map(|paths| {
            let system = Group::open(paths.system_group_path()).ok();
            classic_language_packs(paths).language_infos(system.as_ref())
        })
        .unwrap_or_else(|| state.language_infos.clone());
    if let Some(resources) = resources {
        state.no_language_info = resources
            .get("IDS_CTL_NOLANGINFO")
            .cloned()
            .unwrap_or_else(|| "[Undefined: IDS_CTL_NOLANGINFO]".to_string());
    }
    state.set_language_catalog(language, language_ex, language_infos);
    state
}

pub(crate) fn load_show_log_timestamps(paths: Option<&AppPaths>) -> bool {
    paths
        .and_then(|paths| Config::load(paths.config_file()).ok())
        .and_then(|config| {
            config
                .get_in(Some("General"), "ShowLogTimestamps")
                .and_then(parse_native_config_bool)
        })
        .unwrap_or(false)
}

pub(crate) fn load_message_board_enabled(paths: Option<&AppPaths>) -> bool {
    paths
        .and_then(|paths| Config::load(paths.config_file()).ok())
        .and_then(|config| {
            config
                .get_in(Some("Graphics"), "MsgBoard")
                .and_then(parse_native_config_bool)
        })
        .unwrap_or(true)
}

pub(crate) fn load_options_sound_state(
    audio: Option<&AudioContext>,
) -> clonk_frontend::startup_options_dlg::SoundSheetState {
    let Some(audio) = audio else {
        return clonk_frontend::startup_options_dlg::SoundSheetState::default();
    };
    clonk_frontend::startup_options_dlg::SoundSheetState::new(
        audio.options.menu_music_enabled,
        audio.options.menu_sound_enabled,
        audio.options.music_enabled,
        audio.options.sound_enabled,
        audio.options.music_volume_percent() as u8,
        audio.options.sound_volume_percent() as u8,
    )
}

pub(crate) fn load_options_graphics_state(
    paths: Option<&AppPaths>,
) -> clonk_frontend::startup_options_graphics::GraphicsSheetState {
    use clonk_frontend::startup_options_graphics::{GraphicsDisplayMode, GraphicsSheetState};

    let config = paths.and_then(|paths| Config::load(paths.config_file()).ok());
    let boolean = |key: &str, fallback: bool| {
        config
            .as_ref()
            .and_then(|config| config.get_in(Some("Graphics"), key))
            .and_then(parse_native_config_bool)
            .unwrap_or(fallback)
    };
    let integer = |key: &str, fallback: i32| {
        config
            .as_ref()
            .and_then(|config| config.get_in(Some("Graphics"), key))
            .and_then(|value| value.trim().parse::<i32>().ok())
            .unwrap_or(fallback)
    };
    let display = DisplayOptions::load(paths);
    GraphicsSheetState::new(
        match display.mode {
            DisplayMode::Fullscreen => GraphicsDisplayMode::Fullscreen,
            DisplayMode::Window => GraphicsDisplayMode::Window,
        },
        display.scale_percent(),
        boolean("AddNewCrewPortraits", true),
        boolean("SaveDefaultPortraits", true),
        boolean("AutoFrameSkip", true),
        boolean("ShowFolderMaps", true),
        boolean("DisableGamma", false),
        integer("SmokeLevel", clonk_engine::DEFAULT_SMOKE_LEVEL),
        boolean("FireParticles", true),
    )
}

pub(crate) fn load_options_network_state(
    paths: Option<&AppPaths>,
) -> clonk_frontend::startup_options_network::NetworkSheetState {
    use clonk_frontend::startup_options_network::NetworkSheetState;

    let config = paths.and_then(|paths| Config::load(paths.config_file()).ok());
    let value = |section: &str, key: &str| {
        config
            .as_ref()
            .and_then(|config| config.get_in(Some(section), key))
    };
    let ports = load_network_ports(paths);
    let boolean = |section: &str, key: &str, fallback: bool| {
        value(section, key)
            .and_then(parse_native_config_bool)
            .unwrap_or(fallback)
    };
    NetworkSheetState::new(
        [
            i32::from(ports.tcp),
            i32::from(ports.udp),
            i32::from(ports.reference),
            i32::from(ports.discovery),
        ],
        boolean("Network", "UseAlternateServer", false),
        value("Network", "AlternateServerAddress")
            .unwrap_or(OFFICIAL_LEAGUE_SERVER)
            .to_string(),
        boolean("Network", "EnableAutomaticUpdate", true),
        boolean("Network", "EnableUPnP", true),
        value("Network", "LocalName")
            .unwrap_or("Unknown")
            .to_string(),
        value("Network", "Nick").unwrap_or("").to_string(),
        boolean("Startup", "HideMsgNoOfficialLeague", false),
    )
}

pub(crate) fn load_options_control_state(
    keyboard: &KeyboardBindings,
    gamepad: &GamepadBindings,
    connected_gamepads: usize,
    gamepad_gui_control: bool,
) -> clonk_frontend::startup_options_controls::ControlSheetState {
    let keyboard_labels = std::array::from_fn(|set| {
        std::array::from_fn(|control| {
            ControlBindingId::ALL
                .get(control)
                .and_then(|id| keyboard.key_for_set(set, *id))
                .map(format_key_label)
                .unwrap_or_default()
        })
    });
    let gamepad_labels = std::array::from_fn(|set| {
        std::array::from_fn(|control| {
            ControlBindingId::ALL
                .get(control)
                .map(|id| gamepad.key_label_for_set(set, *id))
                .unwrap_or_default()
        })
    });
    clonk_frontend::startup_options_controls::ControlSheetState::new(
        keyboard_labels,
        gamepad_labels,
        connected_gamepads,
        gamepad_gui_control,
    )
}

pub(crate) fn load_native_config_bytes(paths: Option<&AppPaths>) -> Vec<u8> {
    paths
        .and_then(|paths| fs::read(paths.config_file()).ok())
        .unwrap_or_default()
}

pub(crate) fn materialized_save_description_language(config: &[u8]) -> Vec<u8> {
    match clonk_app_netplay::configured_native_value(config, "General", "Language") {
        Some(value) => value
            .as_bytes()
            .iter()
            .copied()
            .take_while(|byte| *byte != b',')
            .take(2)
            .collect::<Vec<_>>(),
        // C4Config materializes its system-language default only when the
        // field is absent. An explicitly stored empty first segment (for
        // example `Language=,DE`) survives through SCopyUntil unchanged.
        None => classic_loader_system_language()
            .unwrap_or("US")
            .as_bytes()
            .to_vec(),
    }
}

pub(crate) fn configured_process_group_maker(config: &[u8]) -> LegacyCString {
    clonk_app_netplay::configured_native_value(config, "General", "Name").unwrap_or_default()
}

/// C4Game's `Application.isFullScreen` distinguishes the graphical client from
/// `/console`; it is unrelated to the OS window display mode. Console mode
/// enables DebugMode by default, while the graphical client requires the
/// persisted AlwaysDebug switch. Both remain gated by Parameters.AllowDebug.
fn arm_engine_debug_mode(engine: &mut Engine, config: &[u8], console_mode: bool) {
    let always_debug = clonk_app_netplay::configured_native_boolean(config, "General", "DebugMode")
        .unwrap_or(false);
    engine.set_debug_mode((console_mode || always_debug) && engine.allow_debug());
}

pub(crate) fn arm_graphical_engine_debug_mode(engine: &mut Engine, config: &[u8]) {
    arm_engine_debug_mode(engine, config, false);
}

pub(crate) fn arm_configured_graphical_engine_debug_mode(
    engine: &mut Engine,
    paths: Option<&AppPaths>,
) {
    arm_graphical_engine_debug_mode(engine, &load_native_config_bytes(paths));
}

pub(crate) fn arm_configured_engine_debug_mode(
    engine: &mut Engine,
    paths: Option<&AppPaths>,
    console_mode: bool,
) {
    arm_engine_debug_mode(engine, &load_native_config_bytes(paths), console_mode);
}

pub(crate) fn configured_allow_scripting_in_replays(config: &[u8]) -> bool {
    clonk_app_netplay::configured_native_boolean(config, "General", "AllowScriptingInReplays")
        .unwrap_or(false)
}

pub(crate) fn configured_auto_frame_skip(config: &[u8]) -> bool {
    clonk_app_netplay::configured_native_boolean(config, "Graphics", "AutoFrameSkip")
        .unwrap_or(true)
}

/// Freeze C4GameParameters at game activation. Network JoinData is
/// authoritative; otherwise an embedded Parameters.txt overrides the current
/// process configuration exactly as C4GameParameters::Load does.
pub(crate) fn frozen_auto_frame_skip(
    configured: bool,
    embedded_parameters: Option<bool>,
    synchronized_parameters: Option<bool>,
) -> bool {
    synchronized_parameters
        .or(embedded_parameters)
        .unwrap_or(configured)
}

pub(crate) fn configured_console_script_strictness(
    config: &[u8],
) -> clonk_engine::ScriptStrictness {
    let Some(value) =
        clonk_app_netplay::configured_native_scalar(config, "Developer", "ConsoleScriptStrictness")
    else {
        return clonk_engine::ScriptStrictness::Strict3;
    };
    let value = value
        .iter()
        .position(|byte| !matches!(byte, b' ' | b'\t'))
        .map_or(&value[value.len()..], |start| &value[start..]);

    if let Some(strictness) = native_console_script_strictness_number(value) {
        return strictness;
    }

    let identifier_end = value
        .iter()
        .position(|byte| !(byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')))
        .unwrap_or(value.len());
    match &value[..identifier_end] {
        b"NonStrict" => clonk_engine::ScriptStrictness::NonStrict,
        b"Strict1" => clonk_engine::ScriptStrictness::Strict1,
        b"Strict2" => clonk_engine::ScriptStrictness::Strict2,
        b"Strict3" | b"MaxStrict" => clonk_engine::ScriptStrictness::Strict3,
        _ => clonk_engine::ScriptStrictness::Strict3,
    }
}

fn native_console_script_strictness_number(value: &[u8]) -> Option<clonk_engine::ScriptStrictness> {
    let hexadecimal = value.starts_with(b"0x") || value.starts_with(b"0X");
    let (negative, mut cursor, radix) = if hexadecimal {
        (false, 2, 16)
    } else {
        match value.first() {
            Some(b'+') => (false, 1, 10),
            Some(b'-') => (true, 1, 10),
            _ => (false, 0, 10),
        }
    };
    let digits_start = cursor;
    let c_ulong_max = if std::mem::size_of::<std::os::raw::c_ulong>() == 4 {
        u128::from(u32::MAX)
    } else {
        u128::from(u64::MAX)
    };
    let mut magnitude = 0u128;
    let mut overflow = false;
    while let Some(digit) = value.get(cursor).and_then(|byte| match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' if radix == 16 => Some(byte - b'a' + 10),
        b'A'..=b'F' if radix == 16 => Some(byte - b'A' + 10),
        _ => None,
    }) {
        if digit >= radix {
            break;
        }
        let digit = u128::from(digit);
        if !overflow {
            if magnitude > (c_ulong_max - digit) / u128::from(radix) {
                magnitude = c_ulong_max;
                overflow = true;
            } else {
                magnitude = magnitude * u128::from(radix) + digit;
            }
        }
        cursor += 1;
    }
    if cursor == digits_start {
        // With base 16, strtoul still consumes the leading zero when `0x` is
        // not followed by a hexadecimal digit.
        if hexadecimal {
            magnitude = 0;
        } else {
            return None;
        }
    }
    let unsigned_int = if overflow {
        c_ulong_max as u32
    } else if negative {
        0u32.wrapping_sub(magnitude as u32)
    } else {
        magnitude as u32
    };
    let native_byte = unsigned_int.min(u32::from(u8::MAX)) as u8;
    let ordinal = native_byte.min(3);
    Some(match ordinal {
        0 => clonk_engine::ScriptStrictness::NonStrict,
        1 => clonk_engine::ScriptStrictness::Strict1,
        2 => clonk_engine::ScriptStrictness::Strict2,
        _ => clonk_engine::ScriptStrictness::Strict3,
    })
}

pub(crate) fn classic_command_line_definition_modules(
    config: &[u8],
    definition_files: &[PathBuf],
) -> Vec<String> {
    let mut modules = native_config_text(config, "General", "Definitions")
        .map(|definitions| {
            let mut modules = definitions
                .split(';')
                .map(str::to_string)
                .collect::<Vec<_>>();
            if definitions.is_empty() || definitions.ends_with(';') {
                modules.pop();
            }
            modules
        })
        .unwrap_or_default();
    modules.extend(
        definition_files
            .iter()
            .map(|path| path_as_legacy_text(path)),
    );
    modules
}

pub(crate) fn configured_fair_crew_strength(config: &[u8]) -> i32 {
    native_config_text(config, "General", "DefCrewStrength")
        .and_then(|value| value.trim().parse::<i32>().ok())
        .unwrap_or(1000)
}

/// Carries a native C++ byte string through a Rust `String` without treating
/// valid UTF-8-shaped byte sequences as Unicode. Encoding this projection via
/// `encode_legacy_script_text` recovers every original byte.
pub(crate) fn native_bytes_as_legacy_text(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len());
    for byte in bytes {
        text.push_str(&clonk_script::c4_string_from_bytes(std::slice::from_ref(
            byte,
        )));
    }
    text
}

pub(crate) fn native_config_text(config: &[u8], section: &str, key: &str) -> Option<String> {
    clonk_app_netplay::configured_native_value(config, section, key)
        .map(|value| native_bytes_as_legacy_text(value.as_bytes()))
}

/// The two path strings passed by C4GameSave::SaveCore to
/// C4SDefinitions::SetModules. AppPaths maps an installed ExePath layout to
/// `content/` in a source checkout; packaged layouts use the install root.
pub(crate) fn game_save_definition_paths(
    paths: Option<&AppPaths>,
    native_config: &[u8],
) -> (String, String) {
    let executable_path = paths
        .map(|paths| paths.content_dir().unwrap_or(paths.install_root()))
        .map(|path| {
            let mut path = path_as_legacy_text(path);
            if !path.ends_with(std::path::MAIN_SEPARATOR) {
                path.push(std::path::MAIN_SEPARATOR);
            }
            path
        })
        .unwrap_or_default();
    let definition_path =
        native_config_text(native_config, "General", "DefinitionPath").unwrap_or_default();
    (executable_path, definition_path)
}

pub(crate) const DEFAULT_NETWORK_TCP_PORT: u16 = 11_112;
pub(crate) const DEFAULT_NETWORK_UDP_PORT: u16 = 11_113;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NetworkPorts {
    pub(crate) tcp: u16,
    pub(crate) udp: u16,
    pub(crate) discovery: u16,
    pub(crate) reference: u16,
}

pub(crate) fn sanitized_network_ports(config: &[u8]) -> NetworkPorts {
    let configured_port = |key: &str, default: u16| {
        native_config_text(config, "Network", key).map_or(default, |value| {
            value
                .trim()
                .parse::<i64>()
                .ok()
                .and_then(|value| u16::try_from(value).ok())
                .unwrap_or(0)
        })
    };
    let mut ports = NetworkPorts {
        tcp: configured_port("PortTCP", DEFAULT_NETWORK_TCP_PORT),
        udp: configured_port("PortUDP", DEFAULT_NETWORK_UDP_PORT),
        discovery: configured_port("PortDiscovery", clonk_network::DEFAULT_DISCOVERY_PORT),
        reference: configured_port("PortRefServer", clonk_network::DEFAULT_REFERENCE_PORT),
    };

    if ports.tcp > 0 && ports.tcp == ports.reference {
        tracing::warn!(
            "Network TCP port and reference server port both set to same value - increasing reference server port!"
        );
        ports.reference = ports
            .reference
            .checked_add(1)
            .unwrap_or(clonk_network::DEFAULT_REFERENCE_PORT);
    }
    if ports.udp > 0 && ports.udp == ports.discovery {
        tracing::warn!(
            "Network UDP port and LAN game discovery port both set to same value - increasing discovery port!"
        );
        ports.discovery = ports
            .discovery
            .checked_add(1)
            .unwrap_or(clonk_network::DEFAULT_DISCOVERY_PORT);
    }

    ports
}

/// The directory `C4Network2Res` stages network resources in: dynamic groups,
/// received files and temporary download artifacts all live beneath
/// `Config.Network.WorkPath` (C4Config.cpp:527-533,1369-1374;
/// C4Network2Res.cpp:1709-1775), which defaults to `Network`.
///
/// The configured value is a *relative* name under the network cache. An empty,
/// absolute, root-anchored, non-ASCII or parent-traversing value cannot address
/// a directory outside the cache, so it falls back to the native default rather
/// than staging somewhere the user did not ask for.
pub(crate) fn network_work_directory_name(paths: Option<&AppPaths>) -> String {
    let configured = native_config_text(&load_native_config_bytes(paths), "Network", "WorkPath")
        .map(|value| value.trim().to_string())
        .unwrap_or_default();
    let candidate = Path::new(&configured);
    let safe = !configured.is_empty()
        && configured.is_ascii()
        && !candidate.is_absolute()
        && candidate
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)));
    if safe {
        configured
    } else {
        "Network".to_string()
    }
}

pub(crate) fn network_work_directory(paths: Option<&AppPaths>) -> Option<PathBuf> {
    paths.map(|paths| {
        paths
            .cache_dir()
            .join(network_work_directory_name(Some(paths)))
    })
}

/// `Config.General.ThreadPoolThreadCount`, default 8 (C4Config.cpp:406-408).
/// Windows builds its pool from the system default instead, so the key is
/// non-Windows only (C4Application.cpp:152-159).
#[cfg(not(windows))]
pub(crate) fn load_thread_pool_thread_count(paths: Option<&AppPaths>) -> usize {
    native_config_text(
        &load_native_config_bytes(paths),
        "General",
        "ThreadPoolThreadCount",
    )
    .as_deref()
    .and_then(|value| crate::parse_startup_config_integer(value.as_bytes()))
    .and_then(|value| usize::try_from(value).ok())
    .filter(|workers| *workers > 0)
    .unwrap_or(clonk_app_netplay::network::DEFAULT_NETWORK_RUNTIME_WORKER_THREADS)
}

/// The developer console's own remembered position (`C4Console.cpp:1278-1284`).
/// Read from the `Console` section, never from the game window's geometry keys.
pub(crate) fn load_console_window_position(
    paths: Option<&AppPaths>,
) -> Option<crate::console_window_position::ConsoleWindowPlacement> {
    native_config_text(
        &load_native_config_bytes(paths),
        crate::console_window_position::CONSOLE_POSITION_SECTION,
        crate::console_window_position::CONSOLE_POSITION_KEY,
    )
    .as_deref()
    .and_then(crate::console_window_position::parse_console_position)
}

/// `C4Console::StorePosition` (`C4Console.cpp:154-159`): the position alone,
/// because `GetPositionData` sets `storeSize = false`.
pub(crate) fn store_console_window_position(paths: &AppPaths, x: i32, y: i32) -> io::Result<()> {
    let value = crate::console_window_position::format_console_position(x, y);
    persist_native_config_values(
        paths,
        crate::console_window_position::CONSOLE_POSITION_SECTION,
        &[(
            crate::console_window_position::CONSOLE_POSITION_KEY,
            clonk_app_netplay::NativeConfigValue::RawAscii(&value),
        )],
    )
}

/// `Graphics.VerboseObjectLoading`, default 0 (C4Config.cpp:453). Gates the
/// definition and particle loading diagnostics in `clonk-engine`.
pub(crate) fn load_verbose_object_loading(paths: Option<&AppPaths>) -> i32 {
    native_config_text(
        &load_native_config_bytes(paths),
        "Graphics",
        "VerboseObjectLoading",
    )
    .as_deref()
    .and_then(|value| crate::parse_startup_config_integer(value.as_bytes()))
    .unwrap_or(0)
}

/// `C4ConfigGraphics::RenderInactive` bits (C4Config.h:128-129).
pub(crate) const RENDER_INACTIVE_FULLSCREEN: u32 = 1 << 0;
pub(crate) const RENDER_INACTIVE_CONSOLE: u32 = 1 << 1;

/// `Graphics.RenderInactive`, a bitmask whose adapted default is `Console`
/// alone (C4Config.cpp:481). `StartDrawing` refuses to draw while the
/// application is inactive unless the bit for the *active shell* is set
/// (C4GraphicsSystem.cpp:96-106).
pub(crate) fn load_render_inactive_mask(paths: Option<&AppPaths>) -> u32 {
    native_config_text(
        &load_native_config_bytes(paths),
        "Graphics",
        "RenderInactive",
    )
    .as_deref()
    .and_then(|value| crate::parse_startup_config_integer(value.as_bytes()))
    .map_or(RENDER_INACTIVE_CONSOLE, |value| value as u32)
}

/// `C4GraphicsSystem::StartDrawing`'s inactive gate: an active window always
/// draws; an inactive one only when its own shell's bit is set.
pub(crate) fn render_inactive_allows_drawing(
    mask: u32,
    window_active: bool,
    console_shell: bool,
) -> bool {
    if window_active {
        return true;
    }
    let bit = if console_shell {
        RENDER_INACTIVE_CONSOLE
    } else {
        RENDER_INACTIVE_FULLSCREEN
    };
    mask & bit != 0
}

/// `C4ConfigLogging` (C4Config.cpp:699-718): the `[Logging]` stdout level plus
/// one nested section per component, each holding a `LogLevel`. Returns the
/// `EnvFilter` directive they describe, or `None` when nothing is configured.
pub(crate) fn load_logging_config_directive(paths: Option<&AppPaths>) -> Option<String> {
    let config = paths.and_then(|paths| Config::load(paths.config_file()).ok())?;
    let stdout_level = config
        .get_in(Some("Logging"), "LogLevelStdout")
        .map(str::to_string);
    let component_levels = clonk_logging::LOGGING_COMPONENTS
        .iter()
        .filter_map(|(component, _)| {
            config
                .get_in(Some(component), "LogLevel")
                .map(|level| (*component, level.to_string()))
        })
        .collect::<Vec<_>>();
    let borrowed = component_levels
        .iter()
        .map(|(component, level)| (*component, level.as_str()))
        .collect::<Vec<_>>();
    clonk_logging::logging_config_directive(stdout_level.as_deref(), &borrowed)
}

pub(crate) fn load_network_ports(paths: Option<&AppPaths>) -> NetworkPorts {
    sanitized_network_ports(&load_native_config_bytes(paths))
}

pub(crate) fn load_network_startup_settings(paths: Option<&AppPaths>) -> (bool, u16) {
    let config = load_native_config_bytes(paths);
    let masterserver_signup = native_config_text(&config, "Network", "MasterServerSignUp")
        .as_deref()
        .and_then(parse_native_config_bool)
        .unwrap_or(true);
    let ports = sanitized_network_ports(&config);
    (masterserver_signup, ports.tcp)
}

/// `Config.Network.MaxResSearchRecursion`, default 1 (C4Config.cpp:527-533).
/// `C4Network2Res::SearchLocal` uses it both for the first basename lookup and
/// as the hard recursion limit while walking candidate folders
/// (C4Network2Res.cpp:460-490). A missing or unparsable value keeps the native
/// default of one folder; a negative value cannot deepen the search.
pub(crate) fn load_max_resource_search_recursion(paths: Option<&AppPaths>) -> usize {
    native_config_text(
        &load_native_config_bytes(paths),
        "Network",
        "MaxResSearchRecursion",
    )
    .and_then(|value| value.trim().parse::<i32>().ok())
    .map_or(1, |value| usize::try_from(value.max(0)).unwrap_or(1))
}

pub(crate) fn load_network_reference_port(paths: Option<&AppPaths>) -> u16 {
    load_network_ports(paths).reference
}

pub(crate) fn load_prepared_league_host_config(
    paths: Option<&AppPaths>,
    league_server_signup: bool,
) -> prepared_host_bootstrap::PreparedLeagueHostConfig {
    let config = load_native_config_bytes(paths);
    let raw_value = |section: &str, key: &str| native_config_text(&config, section, key);
    let integer = |section: &str, key: &str, default: i32| {
        raw_value(section, key)
            .and_then(|value| value.trim().parse::<i32>().ok())
            .unwrap_or(default)
    };
    let server = load_network_search_settings(paths);
    prepared_host_bootstrap::PreparedLeagueHostConfig {
        endpoint: server.master_server_url,
        transport: clonk_network::LeagueHttpTransportConfig {
            language_charset: raw_value("General", "LanguageCharset").unwrap_or_default(),
            language_sequence: paths
                .and_then(AppPaths::language_override)
                .map(str::to_string)
                .or_else(|| raw_value("General", "LanguageEx"))
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| startup_language_sequence(paths).join(",")),
            // C++ defaults Network.UseCurl to true (C4Config.cpp:561).
            http_backend: clonk_network::HttpBackend::from_use_curl(
                integer("Network", "UseCurl", 1) != 0,
            ),
        },
        update_period_secs: i64::from(integer("Network", "MasterReferencePeriod", 120)),
        league_server_signup,
    }
}

pub(crate) fn lobby_ready_check_cooldown_from_config(
    config: Option<&Config>,
) -> LobbyReadyCheckCooldown {
    let seconds = config
        .and_then(|config| config.get_in(Some("Cooldowns"), "ReadyCheck"))
        .and_then(|value| value.trim().parse::<i64>().ok())
        .unwrap_or(DEFAULT_LOBBY_READY_CHECK_COOLDOWN_SECONDS);
    LobbyReadyCheckCooldown::from_config_seconds(seconds)
}

pub(crate) fn load_lobby_ready_check_cooldown(paths: Option<&AppPaths>) -> LobbyReadyCheckCooldown {
    let config = load_native_config_bytes(paths);
    let seconds = native_config_text(&config, "Cooldowns", "ReadyCheck")
        .and_then(|value| value.trim().parse::<i64>().ok())
        .unwrap_or(DEFAULT_LOBBY_READY_CHECK_COOLDOWN_SECONDS);
    LobbyReadyCheckCooldown::from_config_seconds(seconds)
}

pub(crate) fn ready_check_toasts_enabled_from_config(config: &[u8]) -> bool {
    clonk_app_netplay::configured_native_boolean(config, "Toasts", "ReadyCheck").unwrap_or(true)
}

pub(crate) fn load_ready_check_toasts_enabled(paths: Option<&AppPaths>) -> bool {
    ready_check_toasts_enabled_from_config(&load_native_config_bytes(paths))
}

pub(crate) fn load_sound_command_cooldown(paths: Option<&AppPaths>) -> Duration {
    let config = load_native_config_bytes(paths);
    let seconds = native_config_text(&config, "Cooldowns", "SoundCommand")
        .and_then(|value| value.trim().parse::<i64>().ok())
        .unwrap_or(0)
        .max(0);
    Duration::from_secs(seconds as u64)
}

pub(crate) fn build_network_host_preparation(
    app: &GameApp,
    scenario: &FrontendScenario,
    definition_load: &ScenarioDefinitionLoad,
    effective_definition_modules: &[String],
    definition_resources: &[clonk_network::HostInitialResourceSource],
    staged_definition_paths: Option<(&str, &str)>,
    staged_identity: Option<(&str, &str)>,
) -> Result<NetworkHostPreparation> {
    let scenario_path = scenario
        .path
        .clone()
        .ok_or_else(|| anyhow!("scenario `{}` has no filesystem path", scenario.title))?;
    let config_bytes = load_native_config_bytes(app.app_paths.as_ref());
    let network_ports = sanitized_network_ports(&config_bytes);
    let raw_value = |section: &str, key: &str| native_config_text(&config_bytes, section, key);
    let value =
        |section: &str, key: &str| raw_value(section, key).map(|value| value.trim().to_owned());
    let integer = |section: &str, key: &str, default: i32| {
        value(section, key)
            .and_then(|value| value.parse::<i32>().ok())
            .unwrap_or(default)
    };
    let boolean = |section: &str, key: &str, default: bool| {
        value(section, key)
            .and_then(|value| parse_native_config_bool(&value))
            .unwrap_or(default)
    };
    let (definition_executable_path, definition_path) = staged_definition_paths
        .map(|(executable, definitions)| (executable.to_owned(), definitions.to_owned()))
        .unwrap_or_else(|| game_save_definition_paths(app.app_paths.as_ref(), &config_bytes));
    let definition_executable_root =
        path_from_group_name_bytes(&clonk_script::c4_string_bytes(&definition_executable_path));

    let mut install_roots = Vec::new();
    if let Some(paths) = app.app_paths.as_ref() {
        if let Some(content) = paths.content_dir() {
            install_roots.push(content.to_path_buf());
        }
        install_roots.push(paths.planet_dir().to_path_buf());
        for candidate in [
            paths.scenario_dir(),
            paths.install_root().join("Scenarios"),
            paths.install_root().join("scenarios"),
        ] {
            if scenario_path.starts_with(&candidate) {
                install_roots.insert(0, candidate);
                break;
            }
        }
    } else {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .context("resolve repository root for network host")?;
        install_roots.extend([
            repository.clone(),
            repository.join("content"),
            repository.join("planet"),
        ]);
    }
    if !install_roots
        .iter()
        .any(|root| scenario_path.starts_with(root))
    {
        let parent = scenario_path
            .parent()
            .ok_or_else(|| anyhow!("scenario path has no parent: {}", scenario_path.display()))?;
        install_roots.insert(0, parent.to_path_buf());
    }
    install_roots.retain(|root| root.is_dir());
    let mut seen_roots = HashSet::new();
    install_roots.retain(|root| seen_roots.insert(root.clone()));

    let network_work_path = value("Network", "WorkPath")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "Network".to_string());
    let network_directory = network_work_directory(app.app_paths.as_ref()).unwrap_or_else(|| {
        std::env::temp_dir().join(format!("clonk-rust-network-{}", std::process::id()))
    });
    let (host_name, host_nick) = if let Some((name, nick)) = staged_identity {
        (name.to_owned(), nick.to_owned())
    } else if let Some(paths) = app.app_paths.as_ref() {
        let (name, nick, _) = load_classic_lobby_identity(paths)?;
        (name, nick)
    } else {
        let name = sanitize_classic_lobby_name(
            &sanitize_classic_lobby_name(&app.player_name, "host network name", false)?,
            "host network name",
            false,
        )?;
        let nick = sanitize_classic_lobby_name(
            &sanitize_classic_lobby_name(&name, "host network nick", false)?,
            "host network nick",
            false,
        )?;
        (name, nick)
    };
    let max_load_file_size = value("Network", "MaxLoadFileSize")
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(100 * 1024 * 1024);
    let configured_players = app
        .app_paths
        .as_ref()
        .map(|paths| {
            snapshot_effective_client_player_selection(paths, &app.classic_command_line)
                .map(|selection| load_snapshotted_client_players(paths, &selection))
        })
        .transpose()?;
    let player_sources = if let Some(configured) = configured_players.as_ref() {
        // C4Game copies the configured module string before networking; the
        // alphabetically sorted startup dialog model is presentation only
        // (src/C4Game.cpp:361-364; src/C4PlayerInfo.cpp:357-395).
        configured
            .players()
            .iter()
            .map(|player| PreparedHostPlayerSource {
                resource: clonk_network::HostInitialResourceSource {
                    path: player.source_path().to_path_buf(),
                    lookup_name: player.resource_lookup_name().clone(),
                    opened_name: player.resource_opened_name().clone(),
                    wire_name: player.resource_wire_name().clone(),
                    virtual_group_bytes: None,
                },
                identity: Some(PreparedHostPlayerIdentity {
                    player_name: player.player_name().clone(),
                    network_color: player.network_color(),
                    alternate_color: player.alternate_color(),
                }),
            })
            .collect()
    } else {
        app.startup_player_files
            .iter()
            .filter(|player| player.render_model.activated)
            .map(|player| {
                let wire_name = clonk_engine::LegacyCString::from_bytes(
                    player.file_name.as_bytes().to_vec(),
                )
                .ok_or_else(|| anyhow!("selected player filename contains an interior NUL"))?;
                let opened_name = clonk_engine::LegacyCString::from_bytes(
                    resource_path_identity::opened_group_name(
                        &player.path,
                        wire_name.as_bytes(),
                        &definition_executable_root,
                    ),
                )
                .ok_or_else(|| anyhow!("opened player filename contains an interior NUL"))?;
                Ok(PreparedHostPlayerSource::from(
                    clonk_network::HostInitialResourceSource {
                        path: player.path.clone(),
                        lookup_name: wire_name.clone(),
                        opened_name,
                        wire_name,
                        virtual_group_bytes: None,
                    },
                ))
            })
            .collect::<Result<Vec<_>>>()?
    };
    let mut network_comment = app.scenario_game_options.values().comment.clone();
    // VAL_Comment preserves whitespace and truncates to C4MaxComment bytes
    // (src/C4InputValidation.cpp:156-158; src/C4Constants.h:28).
    let mut network_comment_bytes = clonk_resources::encode_legacy_script_text(&network_comment)
        .ok_or_else(|| anyhow!("Network.Comment is not representable as Windows-1252"))?;
    network_comment_bytes.truncate(256);
    network_comment = native_bytes_as_legacy_text(&network_comment_bytes);
    let group_maker = configured_players
        .as_ref()
        .map(|configured| native_bytes_as_legacy_text(configured.group_maker().as_bytes()))
        .unwrap_or_else(|| value("General", "Name").unwrap_or_default());
    let master_server_signup = app.scenario_game_options.values().master_server_signup;
    let league_server_signup = app.scenario_game_options.values().league_server_signup;
    let league = (master_server_signup || league_server_signup)
        .then(|| load_prepared_league_host_config(app.app_paths.as_ref(), league_server_signup));
    let (initial_definition_modules, fixed_definition_modules, selector_definition_root) =
        match definition_load {
            ScenarioDefinitionLoad::Seed {
                modules,
                definition_root,
            } => (modules.clone(), None, definition_root.clone()),
            ScenarioDefinitionLoad::Fixed {
                modules,
                definition_root,
            } => (Vec::new(), Some(modules.clone()), definition_root.clone()),
        };

    Ok(NetworkHostPreparation {
        scenario_path,
        install_roots,
        effective_definition_modules: effective_definition_modules.to_vec(),
        definition_resources: definition_resources.to_vec(),
        initial_definition_modules,
        fixed_definition_modules,
        selector_definition_root,
        definition_executable_path,
        definition_path,
        languages: startup_language_sequence(app.app_paths.as_ref()),
        language_packs: app
            .app_paths
            .as_ref()
            .map(classic_language_packs)
            .unwrap_or_default(),
        network_work_path,
        network_directory,
        group_maker,
        host_name,
        host_nick,
        network_password: app.scenario_game_options.values().password.clone(),
        network_comment,
        netpuncher_address: raw_value("Network", "PuncherAddress")
            .unwrap_or_else(|| "netpuncher.openclonk.org:11115".to_string()),
        generated_team_name_template: app.generated_team_name_template.clone(),
        player_sources,
        config: prepared_host_bootstrap::PreparedHostBootstrapConfig {
            // CNM_Async, diverging from C++'s CNM_Decentral default. See the
            // PORT_STATUS divergence entry for the measurements.
            control_mode: integer("Network", "ControlMode", 2),
            control_rate: integer("Network", "ControlRate", 2),
            async_max_wait: integer("Network", "AsyncMaxWait", 2),
            fair_crew: app.startup_view_flags.fair_crew,
            fair_crew_strength: integer("General", "DefCrewStrength", 1_000),
            auto_frame_skip: boolean("Graphics", "AutoFrameSkip", true),
            max_load_file_size,
            no_runtime_join: app
                .classic_command_line
                .runtime_join
                .map(|runtime_join| !runtime_join)
                .unwrap_or_else(|| boolean("Network", "NoRuntimeJoin", true)),
            enable_upnp: boolean("Network", "EnableUPnP", true),
            network_tcp_port: app
                .classic_command_line
                .tcp_port
                .unwrap_or(network_ports.tcp),
            network_udp_port: app
                .classic_command_line
                .udp_port
                .unwrap_or(network_ports.udp),
        },
        league,
    })
}

pub(crate) fn load_network_search_settings(
    paths: Option<&AppPaths>,
) -> clonk_network::NetworkGameSearchConfig {
    let config = load_native_config_bytes(paths);
    let network_ports = sanitized_network_ports(&config);
    let value = |key| native_config_text(&config, "Network", key);
    let internet_enabled = value("MasterServerSignUp")
        .as_deref()
        .and_then(parse_native_config_bool)
        .unwrap_or(true);
    let use_alternate = value("UseAlternateServer")
        .as_deref()
        .and_then(parse_native_config_bool)
        .unwrap_or(false);
    let server_key = if use_alternate {
        "AlternateServerAddress"
    } else {
        "ServerAddress"
    };
    let master_server_url = value(server_key)
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| match value {
            "http:" | "https:" => clonk_network::DEFAULT_MASTER_SERVER_URL,
            value => value,
        })
        .unwrap_or(clonk_network::DEFAULT_MASTER_SERVER_URL)
        .to_string();
    clonk_network::NetworkGameSearchConfig {
        internet_enabled,
        use_alternate_server: use_alternate,
        master_server_url,
        discovery_port: network_ports.discovery,
    }
}

pub(crate) fn load_reference_query_settings(
    paths: Option<&AppPaths>,
) -> clonk_network::ReferenceQueryConfig {
    let config = load_native_config_bytes(paths);
    let value = |key| native_config_text(&config, "General", key);
    let language_charset = value("LanguageCharset").unwrap_or_default();
    let language_sequence = paths
        .and_then(AppPaths::language_override)
        .map(str::to_string)
        .or_else(|| value("LanguageEx"))
        .filter(|sequence| !sequence.is_empty())
        .unwrap_or_else(|| startup_language_sequence(paths).join(","));
    // Network.UseCurl picks the HTTP client implementation
    // (C4Network2Reference.cpp:410-413). C++ defaults it to true
    // (C4Config.cpp:561), so an absent or unparsable key keeps the curl policy.
    let http_backend = clonk_network::HttpBackend::from_use_curl(
        native_config_text(&config, "Network", "UseCurl")
            .and_then(|value| value.trim().parse::<i32>().ok())
            .map(|value| value != 0)
            .unwrap_or(true),
    );
    clonk_network::ReferenceQueryConfig {
        language_charset,
        language_sequence,
        http_backend,
    }
}

pub(crate) fn load_league_auth_settings(
    paths: Option<&AppPaths>,
) -> clonk_network::LeagueAuthRequestHead {
    let config = load_native_config_bytes(paths);
    let value = |key| {
        clonk_app_netplay::configured_native_value(&config, "Network", key).unwrap_or_default()
    };
    clonk_network::LeagueAuthRequestHead {
        account: value("LeagueNick"),
        // C4Config deliberately omits LeaguePassword from its compiler. It
        // exists only in Config.Network for the lifetime of this process.
        password: LegacyCString::default(),
        new_account: clonk_engine::LegacyCString::default(),
        new_password: clonk_engine::LegacyCString::default(),
    }
}

pub(crate) fn load_league_auto_login(paths: Option<&AppPaths>) -> bool {
    let config = load_native_config_bytes(paths);
    clonk_app_netplay::configured_native_value(&config, "Network", "LeagueAutoLogin")
        .and_then(|value| parse_native_config_bool(&legacy_presentation_text(value.as_bytes())))
        .unwrap_or(true)
}

pub(crate) fn load_network_nick(paths: Option<&AppPaths>) -> LegacyCString {
    let config = load_native_config_bytes(paths);
    clonk_app_netplay::configured_native_value(&config, "Network", "Nick").unwrap_or_default()
}

/// Curl's C++ HTTP backend exposes only `Uri::Part::Host` to the signup
/// caption. Keep schemes, paths, credentials and numeric ports out of that
/// presentation without changing the exact endpoint used by the worker.
pub(crate) fn league_server_name(endpoint: &str) -> String {
    let authority = endpoint
        .split_once("://")
        .map_or(endpoint, |(_, remainder)| remainder)
        .split('/')
        .next()
        .unwrap_or_default()
        .rsplit_once('@')
        .map_or_else(
            || {
                endpoint
                    .split_once("://")
                    .map_or(endpoint, |(_, remainder)| remainder)
                    .split('/')
                    .next()
                    .unwrap_or_default()
            },
            |(_, host)| host,
        );
    if let Some(host) = authority.strip_prefix('[') {
        return host
            .split_once(']')
            .map_or(host, |(host, _)| host)
            .to_string();
    }
    authority
        .rsplit_once(':')
        .filter(|(_, port)| !port.is_empty() && port.bytes().all(|byte| byte.is_ascii_digit()))
        .map_or(authority, |(host, _)| host)
        .to_string()
}

pub(crate) fn retain_client_league_server_name(
    network_mode: Option<&mut NetworkMode>,
    league_address: &LegacyCString,
) -> String {
    let server_name = league_server_name(&legacy_presentation_text(league_address.as_bytes()));
    if let Some(NetworkMode::Client(settings)) = network_mode {
        settings.league_server_name.clone_from(&server_name);
    }
    server_name
}

pub(crate) fn retained_client_league_server_name(network_mode: Option<&NetworkMode>) -> String {
    match network_mode {
        Some(NetworkMode::Client(settings)) => settings.league_server_name.clone(),
        Some(NetworkMode::Host(_)) | None => String::new(),
    }
}
pub(crate) fn load_network_advertiser_settings(
    paths: Option<&AppPaths>,
) -> clonk_network::NetworkGameAdvertiserConfig {
    #[cfg(test)]
    if paths.is_none() {
        return clonk_network::NetworkGameAdvertiserConfig {
            discovery_port: 0,
            reference_port: Some(0),
            language_charset: String::new(),
        };
    }
    let config = load_native_config_bytes(paths);
    let ports = sanitized_network_ports(&config);
    clonk_network::NetworkGameAdvertiserConfig {
        discovery_port: ports.discovery,
        reference_port: (ports.reference != 0).then_some(ports.reference),
        language_charset: native_config_text(&config, "General", "LanguageCharset")
            .unwrap_or_default(),
    }
}

pub(crate) fn sanitize_classic_lobby_name(
    value: &str,
    field: &str,
    allow_empty: bool,
) -> Result<String> {
    let native = clonk_resources::encode_legacy_script_text(value)
        .ok_or_else(|| anyhow!("{field} is not representable as Windows-1252"))?;
    let native = LegacyCString::from_bytes(native)
        .ok_or_else(|| anyhow!("{field} contains an interior NUL"))?;
    let sanitized = if allow_empty {
        clonk_network::validate_name_allow_empty(native)
    } else {
        clonk_network::validate_name_no_empty(native)
    };
    Ok(native_bytes_as_legacy_text(sanitized.as_bytes()))
}

#[cfg(unix)]
fn system_hostname_bytes() -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;

    gethostname::gethostname().as_os_str().as_bytes().to_vec()
}

#[cfg(windows)]
fn system_hostname_bytes() -> Vec<u8> {
    // Match C4Config's Winsock narrow-byte call instead of transcoding the
    // physical DNS hostname through Rust's UTF-16 `OsString` representation.
    #[link(name = "ws2_32")]
    unsafe extern "system" {
        fn WSAStartup(version: u16, data: *mut std::ffi::c_void) -> i32;
        fn WSACleanup() -> i32;
        fn gethostname(name: *mut std::ffi::c_char, length: i32) -> i32;
    }

    let mut winsock_data = std::mem::MaybeUninit::<[u64; 64]>::uninit();
    if unsafe { WSAStartup(0x0202, winsock_data.as_mut_ptr().cast()) } != 0 {
        return Vec::new();
    }
    let mut hostname = [0_u8; 26];
    let result = unsafe { gethostname(hostname.as_mut_ptr().cast(), 25) };
    unsafe {
        WSACleanup();
    }
    if result != 0 {
        return Vec::new();
    }
    let length = hostname
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(hostname.len());
    hostname[..length].to_vec()
}

fn classic_hostname_fallback(hostname: &[u8]) -> String {
    // C4Config passes a 25-byte destination span to gethostname. Keep the
    // largest defined NUL-terminated payload instead of depending on the
    // platform-specific overlength behavior of that C call.
    let hostname = hostname
        .iter()
        .copied()
        .take_while(|byte| *byte != 0)
        .take(24)
        .collect::<Vec<_>>();
    if hostname.is_empty() {
        "Unknown".to_string()
    } else {
        native_bytes_as_legacy_text(&hostname)
    }
}

pub(crate) fn load_classic_lobby_identity(paths: &AppPaths) -> Result<(String, String, i32)> {
    load_classic_lobby_identity_with_hostname_provider(paths, system_hostname_bytes)
}

pub(crate) fn load_classic_lobby_identity_with_hostname(
    paths: &AppPaths,
    hostname: &[u8],
) -> Result<(String, String, i32)> {
    load_classic_lobby_identity_with_hostname_provider(paths, || hostname.to_vec())
}

pub(crate) fn load_classic_lobby_identity_with_hostname_provider(
    paths: &AppPaths,
    hostname: impl FnOnce() -> Vec<u8>,
) -> Result<(String, String, i32)> {
    let config = match fs::read(paths.config_file()) {
        Ok(config) => config,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error).context("cannot read lobby configuration"),
    };
    let value = |section, key| {
        clonk_app_netplay::configured_native_value(&config, section, key)
            .map(|value| native_bytes_as_legacy_text(value.as_bytes()))
    };
    let network_name = |key| {
        clonk_app_netplay::configured_native_dynamic_value(&config, "Network", key)
            .map(|value| native_bytes_as_legacy_text(value.as_bytes()))
    };
    let configured_local_name = sanitize_classic_lobby_name(
        &network_name("LocalName").unwrap_or_else(|| "Unknown".to_string()),
        "Network.LocalName",
        false,
    )?;
    let local_name = if configured_local_name == "Unknown" {
        classic_hostname_fallback(&hostname())
    } else {
        configured_local_name
    };
    let local_name = sanitize_classic_lobby_name(&local_name, "Network.LocalName", false)?;
    let configured_nick = sanitize_classic_lobby_name(
        &network_name("Nick").unwrap_or_default(),
        "Network.Nick",
        true,
    )?;
    let nick = if configured_nick.is_empty() {
        let nick = sanitize_classic_lobby_name(&local_name, "Network.Nick", false)?;
        sanitize_classic_lobby_name(&nick, "Network.Nick", false)?
    } else {
        sanitize_classic_lobby_name(&configured_nick, "Network.Nick", false)?
    };
    let countdown = match value("Lobby", "CountdownTime") {
        Some(value) => value
            .trim()
            .parse::<i32>()
            .context("Lobby.CountdownTime is not a C++ int32")?,
        None => 5,
    };
    Ok((local_name, nick, countdown))
}

pub(crate) fn load_scenario_game_option_values(paths: Option<&AppPaths>) -> GameOptionValues {
    let config = load_native_config_bytes(paths);
    let bool_value = |section: &str, key: &str, default| {
        native_config_text(&config, section, key)
            .as_deref()
            .and_then(parse_native_config_bool)
            .unwrap_or(default)
    };
    let string_value = |section: &str, key: &str, default: &str| {
        clonk_app_netplay::configured_native_value(&config, section, key)
            .map(|value| native_bytes_as_legacy_text(value.as_bytes()))
            .unwrap_or_else(|| default.to_string())
    };
    let fair_crew_strength = configured_fair_crew_strength(&config);
    GameOptionValues {
        master_server_signup: bool_value("Network", "MasterServerSignUp", true),
        league_server_signup: bool_value("Network", "LeagueServerSignUp", false),
        password: String::new(),
        last_password: string_value("Network", "LastPassword", "Wipf"),
        comment: string_value("Network", "Comment", ""),
        fair_crew: clonk_app_netplay::configured_native_boolean(&config, "General", "NoCrew")
            .unwrap_or(false),
        fair_crew_strength,
        record: bool_value("General", "Record", false),
        ..GameOptionValues::default()
    }
}

pub(crate) fn scenario_fair_crew_constraint(
    scenario: Option<&FrontendScenario>,
) -> FairCrewConstraint {
    let Some(path) = scenario.and_then(|scenario| scenario.path.as_deref()) else {
        return FairCrewConstraint::Free;
    };
    let Some(source) = Group::open(path)
        .ok()
        .and_then(|group| read_group_file_case_insensitive(&group, "Scenario.txt"))
    else {
        return FairCrewConstraint::Free;
    };
    let mut reader = io::Cursor::new(source);
    let forced = Config::from_reader(&mut reader)
        .ok()
        .and_then(|config| {
            config
                .get_in(Some("Head"), "ForcedNoCrew")
                .and_then(|value| value.trim().parse::<i32>().ok())
        })
        .unwrap_or(0);
    match forced {
        1 => FairCrewConstraint::ForceFair,
        2 => FairCrewConstraint::ForceNormal,
        _ => FairCrewConstraint::Free,
    }
}

pub(crate) fn persist_config_value(
    paths: &AppPaths,
    section: &str,
    key: &str,
    value: impl Into<String>,
) -> io::Result<()> {
    let path = paths.config_file();
    let mut config = match Config::load(&path) {
        Ok(config) => config,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Config::new(),
        Err(error) => return Err(error),
    };
    config.set_in(Some(section), key, value);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    save_config_preserving_native_general_booleans(&config, &path, None, None)
}

pub(crate) fn persist_irc_login_settings(
    paths: &AppPaths,
    login: &clonk_frontend::startup_netdlg::NetDlgChatLogin,
) -> io::Result<()> {
    let native_value = |field: &str, value: &str| {
        clonk_resources::encode_legacy_script_text(value).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("IRC {field} is not representable in the classic Windows-1252 config"),
            )
        })
    };
    let nick = native_value("nickname", &login.nick)?;
    let real_name = native_value("real name", &login.real_name)?;
    let channel = native_value("channel", &login.channel)?;
    persist_native_config_values(
        paths,
        "IRC",
        &[
            (
                "Nick",
                clonk_app_netplay::NativeConfigValue::CppEscapedString(&nick),
            ),
            (
                "RealName",
                clonk_app_netplay::NativeConfigValue::CppEscapedString(&real_name),
            ),
            (
                "Channel",
                clonk_app_netplay::NativeConfigValue::CppEscapedString(&channel),
            ),
        ],
    )
}

pub(crate) fn persist_irc_warning_preference(paths: &AppPaths, checked: bool) -> io::Result<()> {
    persist_native_config_values(
        paths,
        "Startup",
        &[(
            "HideMsgIRCDangerous",
            clonk_app_netplay::NativeConfigValue::RawAscii(if checked { "1" } else { "0" }),
        )],
    )
}

pub(crate) fn persist_startup_portrait_location(paths: &AppPaths, index: usize) -> io::Result<()> {
    let index = index.to_string();
    persist_native_config_values(
        paths,
        "Startup",
        &[(
            "LastPortraitFolderIdx",
            clonk_app_netplay::NativeConfigValue::RawAscii(&index),
        )],
    )
}

pub(crate) fn persist_native_config_values(
    paths: &AppPaths,
    section: &str,
    updates: &[(&str, clonk_app_netplay::NativeConfigValue<'_>)],
) -> io::Result<()> {
    let path = paths.config_file();
    let config = match fs::read(&path) {
        Ok(config) => config,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error),
    };
    let updated = clonk_app_netplay::update_configured_native_values(&config, section, updates)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, updated)
}

pub(crate) fn persist_league_account_preference(
    paths: &AppPaths,
    account: &LegacyCString,
) -> io::Result<()> {
    persist_native_config_values(
        paths,
        "Network",
        &[(
            "LeagueNick",
            clonk_app_netplay::NativeConfigValue::CppEscapedString(account.as_bytes()),
        )],
    )
}

pub(crate) fn apply_startup_options_config(
    config: &mut Config,
    program: &clonk_frontend::startup_options_dlg::ProgramSheetState,
    audio_options: Option<&AudioOptions>,
    graphics: &clonk_frontend::startup_options_graphics::GraphicsSheetState,
    network: &clonk_frontend::startup_options_network::NetworkSheetState,
    keyboard_bindings: &KeyboardBindings,
    gamepad_bindings: &GamepadBindings,
    gamepad_gui_control: bool,
) {
    config.set_in(Some("General"), "Language", &program.language);
    config.set_in(Some("General"), "LanguageEx", &program.language_ex);
    config.set_in(Some("General"), "FontName", &program.font_face);
    config.set_in(Some("General"), "FontSize", &program.font_size);
    let bool_string = |value: bool| i32::from(value).to_string();
    config.set_in(
        Some("General"),
        "UseWhiteIngameChat",
        bool_string(program.white_chat_ingame),
    );
    config.set_in(
        Some("General"),
        "UseWhiteLobbyChat",
        bool_string(program.white_chat_lobby),
    );
    config.set_in(
        Some("General"),
        "ShowLogTimestamps",
        bool_string(program.show_log_timestamps),
    );
    config.set_in(
        Some("General"),
        "Preloading",
        bool_string(program.preloading),
    );
    config.set_in(
        Some("General"),
        "DefCrewStrength",
        program.fair_crew_strength.to_string(),
    );
    if let Some(audio_options) = audio_options {
        audio_options.write_startup_sound_config(config);
    }
    config.set_in(
        Some("Graphics"),
        "DisplayMode",
        graphics.display_mode.config_value(),
    );
    config.set_in(
        Some("Graphics"),
        "Scale",
        graphics.applied_scale_percent.to_string(),
    );
    for (key, value) in [
        ("AddNewCrewPortraits", graphics.add_new_crew_portraits),
        ("SaveDefaultPortraits", graphics.save_default_portraits),
        ("AutoFrameSkip", graphics.auto_frame_skip),
        ("ShowFolderMaps", graphics.show_folder_maps),
        ("DisableGamma", graphics.disable_gamma),
        ("FireParticles", graphics.fire_particles),
    ] {
        config.set_in(Some("Graphics"), key, bool_string(value));
    }
    config.set_in(
        Some("Graphics"),
        "SmokeLevel",
        graphics.smoke_level.to_string(),
    );
    use clonk_frontend::startup_options_network::{NetworkPortId, NetworkPortState};
    let port_value = |port: &NetworkPortState| port.config_value().to_string();
    for (key, id) in [
        ("PortTCP", NetworkPortId::Tcp),
        ("PortUDP", NetworkPortId::Udp),
        ("PortRefServer", NetworkPortId::Reference),
        ("PortDiscovery", NetworkPortId::Discovery),
    ] {
        config.set_in(Some("Network"), key, port_value(network.port(id)));
    }
    config.set_in(
        Some("Network"),
        "UseAlternateServer",
        bool_string(network.use_alternate_server),
    );
    config.set_in(
        Some("Network"),
        "AlternateServerAddress",
        &network.alternate_server_address,
    );
    config.set_in(
        Some("Network"),
        "EnableAutomaticUpdate",
        bool_string(network.automatic_update),
    );
    config.set_in(
        Some("Network"),
        "EnableUPnP",
        bool_string(network.enable_upnp),
    );
    config.set_in(Some("Network"), "LocalName", &network.local_name);
    config.set_in(Some("Network"), "Nick", network.stored_nick());
    config.set_in(
        Some("Startup"),
        "HideMsgNoOfficialLeague",
        bool_string(network.hide_no_official_league_notice),
    );
    config.set_in(
        Some("Controls"),
        "GamepadGuiControl",
        bool_string(gamepad_gui_control),
    );
    keyboard_bindings.write_to_config(config);
    gamepad_bindings.write_to_config(config);
}

pub(crate) fn persist_startup_options_config(
    paths: &AppPaths,
    program: &clonk_frontend::startup_options_dlg::ProgramSheetState,
    audio_options: Option<&AudioOptions>,
    graphics: &clonk_frontend::startup_options_graphics::GraphicsSheetState,
    network: &clonk_frontend::startup_options_network::NetworkSheetState,
    keyboard_bindings: &KeyboardBindings,
    gamepad_bindings: &GamepadBindings,
    gamepad_gui_control: bool,
) -> io::Result<()> {
    let path = paths.config_file();
    let mut config = match Config::load(&path) {
        Ok(config) => config,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Config::new(),
        Err(error) => return Err(error),
    };
    apply_startup_options_config(
        &mut config,
        program,
        audio_options,
        graphics,
        network,
        keyboard_bindings,
        gamepad_bindings,
        gamepad_gui_control,
    );
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    save_config_preserving_native_general_booleans(&config, &path, None, None)
}

pub(crate) fn load_participants_label(paths: Option<&AppPaths>) -> String {
    participants_label_with_pending(paths, None)
}

/// As [`load_participants_label`], but `pending` — an unflushed
/// `General.Participants` — wins over the file.
///
/// `C4StartupMainDlg::UpdateParticipants` reads the in-memory
/// `Config.General.Participants` (`C4StartupMainDlg.cpp:174-200`), so a
/// concurrent writer to the config file cannot change what it displays. Any
/// reader of a deferred key has to consult memory first for that to hold.
pub(crate) fn participants_label_with_pending(
    paths: Option<&AppPaths>,
    pending: Option<&str>,
) -> String {
    // C++ C4StartupMainDlg::UpdateParticipants (C4StartupMainDlg.cpp:174-200):
    // IDS_DESC_PLRS ("Players: ") + comma-separated player file basenames
    // without extension, or IDS_DLG_NOPLAYERSSELECTED ("none selected").
    let mut label = String::from("Players: ");
    let Some(paths) = paths else {
        label.push_str("none selected");
        return label;
    };

    let config_path = paths.config_file();
    match Config::load(&config_path) {
        Ok(config) => {
            let entries = pending
                .map(|raw| raw.to_owned())
                .or_else(|| {
                    config
                        .get_in(Some("General"), "Participants")
                        .map(str::to_owned)
                })
                .map(|raw| raw.split(';').map(str::to_owned).collect::<Vec<_>>())
                .unwrap_or_default();
            let mut names = Vec::new();
            for entry in entries {
                let trimmed = entry.trim().trim_matches('"');
                if trimmed.is_empty() {
                    continue;
                }
                let name = Path::new(trimmed)
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| trimmed.to_string());
                if !name.is_empty() {
                    names.push(name);
                }
            }
            if names.is_empty() {
                label.push_str("none selected");
            } else {
                label.push_str(&names.join(", "));
            }
        }
        Err(err) => {
            if err.kind() != io::ErrorKind::NotFound {
                tracing::warn!(
                    error = %err,
                    path = %config_path.display(),
                    "failed to read participants from config"
                );
            }
            label.push_str("none selected");
        }
    }

    label
}

pub(crate) fn startup_participant_references(paths: &AppPaths) -> io::Result<Vec<String>> {
    Ok(startup_participant_indexed_references(paths)?
        .into_iter()
        .map(|(_, reference)| reference)
        .collect())
}

/// `SModuleCount`: spaces do not start a module, semicolons reset the module
/// boundary, and every other byte starts at most one module.
pub(crate) fn c4_module_count(raw: &str) -> i32 {
    let mut count = 0_i32;
    let mut new_module = true;
    for byte in raw.bytes() {
        match byte {
            b' ' => {}
            b';' => new_module = true,
            _ if new_module => {
                count = count.saturating_add(1);
                new_module = false;
            }
            _ => new_module = false,
        }
    }
    count
}

pub(crate) fn startup_participant_module_count(paths: &AppPaths) -> io::Result<i32> {
    let config = match Config::load(paths.config_file()) {
        Ok(config) => config,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error),
    };
    Ok(config
        .get_in(Some("General"), "Participants")
        .map(c4_module_count)
        .unwrap_or(0))
}

fn startup_participant_indexed_references(paths: &AppPaths) -> io::Result<Vec<(usize, String)>> {
    let config = match Config::load(paths.config_file()) {
        Ok(config) => config,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    Ok(config
        .get_in(Some("General"), "Participants")
        .into_iter()
        .flat_map(|raw| raw.split(';'))
        .enumerate()
        .filter_map(|(raw_index, entry)| {
            let entry = entry.trim().trim_matches('"');
            (!entry.is_empty()).then(|| (raw_index, entry.to_string()))
        })
        .collect())
}

pub(crate) fn update_startup_participant_config(
    paths: &AppPaths,
    update: impl FnOnce(&mut Vec<String>),
) -> io::Result<()> {
    let config = match Config::load(paths.config_file()) {
        Ok(config) => config,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Config::new(),
        Err(error) => return Err(error),
    };
    let mut entries = config
        .get_in(Some("General"), "Participants")
        .into_iter()
        .flat_map(|raw| raw.split(';'))
        .map(|entry| entry.trim().trim_matches('"'))
        .filter(|entry| !entry.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    update(&mut entries);
    save_validated_startup_participant_config(paths, config, entries)
}

pub(crate) fn validate_startup_participant_config(paths: &AppPaths) -> io::Result<()> {
    let config = match Config::load(paths.config_file()) {
        Ok(config) => config,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let entries = config
        .get_in(Some("General"), "Participants")
        .into_iter()
        .flat_map(|raw| raw.split(';'))
        .map(|entry| entry.trim().trim_matches('"'))
        .filter(|entry| !entry.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    save_validated_startup_participant_config(paths, config, entries)
}

fn save_validated_startup_participant_config(
    paths: &AppPaths,
    mut config: Config,
    entries: Vec<String>,
) -> io::Result<()> {
    let mut validated = Vec::with_capacity(entries.len());
    for entry in entries {
        if startup_participant_reference_is_valid(paths, &entry)
            && !validated
                .iter()
                .any(|accepted: &String| accepted.eq_ignore_ascii_case(&entry))
        {
            validated.push(entry);
        }
    }
    config.set_in(Some("General"), "Participants", validated.join(";"));
    let config_path = paths.config_file();
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)?;
    }
    save_config_preserving_native_general_booleans(&config, &config_path, None, None)
}

pub(crate) fn remove_startup_participant_config(
    paths: &AppPaths,
    raw_index: usize,
) -> io::Result<Option<String>> {
    let config_path = paths.config_file();
    let config = match Config::load(&config_path) {
        Ok(config) => config,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Config::new(),
        Err(error) => return Err(error),
    };
    let raw = config
        .get_in(Some("General"), "Participants")
        .unwrap_or_default()
        .to_string();
    let target = raw
        .split(';')
        .nth(raw_index)
        .map(|entry| entry.trim().trim_matches('"').to_string());
    let mut entries = raw
        .split(';')
        .map(|entry| entry.trim().trim_matches('"'))
        .filter(|entry| !entry.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    if let Some(target) = target.as_deref() {
        if let Some(index) = entries
            .iter()
            .position(|entry| entry.eq_ignore_ascii_case(target))
        {
            entries.remove(index);
        }
    }
    save_validated_startup_participant_config(paths, config, entries)?;
    Ok(target)
}

pub(crate) fn startup_player_path(config: &Config) -> PathBuf {
    config
        .get_in(Some("General"), "PlayerPath")
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .unwrap_or_default()
}

pub(crate) fn startup_player_search_paths(paths: &AppPaths, config: &Config) -> Vec<PathBuf> {
    let player_path = startup_player_path(config);
    if player_path.is_absolute() {
        vec![player_path]
    } else {
        // AppPaths represents the installed ExePath plus the two developer
        // build ExePath variants already used by player discovery. Each root
        // is still scanned non-recursively and in native directory order.
        [
            paths.install_root().to_path_buf(),
            paths.install_root().join("build"),
            paths.install_root().join("build-arm64-native"),
        ]
        .into_iter()
        .map(|root| root.join(&player_path))
        .collect()
    }
}

fn startup_participant_reference(player_path: &Path, path: &Path, name: &str) -> String {
    if player_path.is_absolute() {
        path.to_string_lossy().into_owned()
    } else {
        player_path.join(name).to_string_lossy().into_owned()
    }
}

fn startup_participant_reference_is_valid(paths: &AppPaths, reference: &str) -> bool {
    if !Path::new(reference)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("c4p"))
    {
        return false;
    }
    let path = Path::new(reference);
    if path.is_absolute() {
        path.exists()
    } else {
        [
            paths.install_root().to_path_buf(),
            paths.install_root().join("build"),
            paths.install_root().join("build-arm64-native"),
        ]
        .into_iter()
        .any(|root| root.join(path).exists())
    }
}

fn is_visible_startup_player_file_name(name: &str) -> bool {
    !name.starts_with('.')
        && Path::new(name)
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("c4p"))
}

pub(crate) fn startup_player_file_exists(paths: &AppPaths) -> io::Result<bool> {
    let config = match Config::load(paths.config_file()) {
        Ok(config) => config,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Config::new(),
        Err(error) => return Err(error),
    };
    let mut first_error = None;
    for search_path in startup_player_search_paths(paths, &config) {
        let entries = match fs::read_dir(search_path) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => {
                first_error.get_or_insert(error);
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    first_error.get_or_insert(error);
                    continue;
                }
            };
            let name = entry.file_name().to_string_lossy().into_owned();
            if is_visible_startup_player_file_name(&name) {
                return Ok(true);
            }
        }
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(false),
    }
}

fn startup_player_file_references(paths: &AppPaths) -> io::Result<Vec<String>> {
    let config = match Config::load(paths.config_file()) {
        Ok(config) => config,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Config::new(),
        Err(error) => return Err(error),
    };
    let player_path = startup_player_path(&config);
    let mut references = Vec::new();
    for search_path in startup_player_search_paths(paths, &config) {
        let entries = match fs::read_dir(search_path) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if !is_visible_startup_player_file_name(&name) {
                continue;
            }
            let reference = startup_participant_reference(&player_path, &entry.path(), &name);
            if !references
                .iter()
                .any(|known: &String| known.eq_ignore_ascii_case(&reference))
            {
                references.push(reference);
            }
        }
    }
    Ok(references)
}

fn startup_participant_display_name(reference: &str) -> String {
    let file_name = reference.rsplit(['/', '\\']).next().unwrap_or(reference);
    Path::new(file_name)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(file_name)
        .to_string()
}

pub(crate) fn startup_participant_add_entries(
    paths: &AppPaths,
) -> Vec<ContextMenuEntry<AppContextMenuCommand>> {
    let active = match startup_participant_references(paths) {
        Ok(active) => active,
        Err(error) => {
            tracing::error!(%error, "failed to read active startup participants");
            return Vec::new();
        }
    };
    match startup_player_file_references(paths) {
        Ok(players) => players
            .into_iter()
            .filter(|player| {
                !active
                    .iter()
                    .any(|entry| entry.eq_ignore_ascii_case(player))
            })
            .map(|player| {
                ContextMenuEntry::new(startup_participant_display_name(&player))
                    .with_tooltip("Let this player join in next game")
                    .with_icon(ContextMenuIcon::Phase(9))
                    .with_action(AppContextMenuCommand::AddStartupParticipant(player))
            })
            .collect(),
        Err(error) => {
            tracing::error!(%error, "failed to enumerate players for participants context menu");
            Vec::new()
        }
    }
}

pub(crate) fn startup_participant_remove_entries(
    paths: &AppPaths,
) -> Vec<ContextMenuEntry<AppContextMenuCommand>> {
    match startup_participant_indexed_references(paths) {
        Ok(entries) => entries
            .into_iter()
            .map(|(raw_index, reference)| {
                ContextMenuEntry::new(startup_participant_display_name(&reference))
                    .with_tooltip("Remove this player from participation list")
                    .with_icon(ContextMenuIcon::Phase(9))
                    .with_action(AppContextMenuCommand::RemoveStartupParticipant(raw_index))
            })
            .collect(),
        Err(error) => {
            tracing::error!(%error, "failed to read participants context menu");
            Vec::new()
        }
    }
}

/// Loads the first selected local player file, mirroring
/// `C4Game::Init` -> `C4ClientPlayerInfos(Game.PlayerFilenames)`
/// (C4Game.cpp:362-366; C4PlayerInfo.cpp:357-390).
pub(crate) fn load_selected_player_file(paths: Option<&AppPaths>) -> Option<PlayerFile> {
    let paths = paths?;
    let config_path = paths.config_file();
    let config = match Config::load(&config_path) {
        Ok(config) => config,
        Err(err) => {
            if err.kind() != io::ErrorKind::NotFound {
                tracing::warn!(
                    error = %err,
                    path = %config_path.display(),
                    "failed to read selected player from config"
                );
            }
            return None;
        }
    };
    let participant = config
        .get_in(Some("General"), "Participants")?
        .split(';')
        .map(|entry| entry.trim().trim_matches('"'))
        .find(|entry| !entry.is_empty())?;
    let participant_path = Path::new(participant);

    let mut candidates = Vec::new();
    if participant_path.is_absolute() {
        candidates.push(participant_path.to_path_buf());
    } else {
        // C++ stores participant names relative to ExePath. AppPaths uses
        // the repository/install root, so accept that direct form first.
        candidates.push(paths.install_root().join(participant_path));

        if let Some(player_path) = config
            .get_in(Some("General"), "PlayerPath")
            .map(str::trim)
            .filter(|path| !path.is_empty())
        {
            let player_root = Path::new(player_path);
            let player_root = if player_root.is_absolute() {
                player_root.to_path_buf()
            } else {
                paths.install_root().join(player_root)
            };
            candidates.push(player_root.join(participant_path));
        }

        // Developer builds keep the C++ executable and its relative .c4p
        // files here; this is the ExePath equivalent for the Rust binary.
        candidates.push(paths.install_root().join("build").join(participant_path));
        candidates.push(
            paths
                .install_root()
                .join("build-arm64-native")
                .join(participant_path),
        );
    }

    candidates.dedup();
    for candidate in candidates {
        if !candidate.exists() {
            continue;
        }
        match PlayerFile::load_from_path(&candidate) {
            Ok(player) => return Some(player),
            Err(err) => tracing::warn!(
                error = %err,
                path = %candidate.display(),
                "failed to load selected player file"
            ),
        }
    }

    tracing::warn!(participant, "selected player file was not found");
    None
}

pub(crate) fn overlay_text_needs_update(current: &str, default_prefix: &str) -> bool {
    current.is_empty() || current.starts_with(default_prefix)
}

pub(crate) fn c4_presentation_text(text: &str) -> String {
    legacy_presentation_text(&clonk_script::c4_string_bytes(text))
}

pub(crate) fn legacy_presentation_text(bytes: &[u8]) -> String {
    clonk_resources::decode_legacy_script_text(bytes)
}

pub(crate) fn league_result_message(message: &str) -> LegacyCString {
    let mut bytes = clonk_resources::encode_legacy_script_text(message)
        .unwrap_or_else(|| message.as_bytes().to_vec());
    bytes.retain(|byte| *byte != 0);
    LegacyCString::from_bytes(bytes)
        .expect("filtered league result message contains no interior NUL")
}

/// `C4ObjectInfo::Draw` resolves the portrait tint from the object passed as
/// `pOfObj`, not from the viewport player. A missing live player leaves the
/// native `0xffffffff` fallback in place, suppressing the owner-color layer.
pub(crate) fn cursor_portrait_owner_color(snapshot: &SimulationSnapshot, owner: i32) -> u32 {
    let Some(player) = snapshot.players.iter().find(|player| player.id == owner) else {
        return u32::MAX;
    };
    let packed = player.color.map_or(0, |color| {
        u32::from(color.r) << 16 | u32::from(color.g) << 8 | u32::from(color.b)
    });
    // C4Surface::SetClr substitutes the legacy blue owner color for zero.
    if packed == 0 {
        0xff
    } else {
        packed
    }
}

#[derive(Clone)]
pub(crate) struct CursorPortraitImages {
    pub(crate) base: ImageData,
    pub(crate) owner_overlay: Option<ImageData>,
}

/// Prepares the two surfaces consumed by `C4DefGraphics::DrawClr`. Keeping
/// them separate lets the HUD scale/filter the base and owner overlay in the
/// same two passes as C++.
pub(crate) fn cursor_portrait_images(
    image: clonk_engine::DefinitionPictureImage,
) -> CursorPortraitImages {
    let width = image.width();
    let height = image.height();
    let mask = image.color_mask();
    let pixels = image.into_pixels();
    let Some(mask) = mask else {
        return CursorPortraitImages {
            base: ImageData::from_arc(width, height, pixels),
            owner_overlay: None,
        };
    };
    let Some(pixel_count) = usize::try_from(u64::from(width) * u64::from(height)).ok() else {
        return CursorPortraitImages {
            base: ImageData::from_arc(width, height, pixels),
            owner_overlay: None,
        };
    };
    if pixels.len() != pixel_count.saturating_mul(4) {
        return CursorPortraitImages {
            base: ImageData::from_arc(width, height, pixels),
            owner_overlay: None,
        };
    }

    let mut base = pixels.to_vec();

    if mask.len() == pixel_count {
        // CreateColorByOwner moves each detected pixel wholly out of the base
        // surface. The retained scalar is the gray owner-overlay intensity.
        let mut overlay = vec![255; base.len()];
        for pixel in overlay.chunks_exact_mut(4) {
            pixel[3] = 0;
        }
        for ((base, overlay), gray) in base
            .chunks_exact_mut(4)
            .zip(overlay.chunks_exact_mut(4))
            .zip(mask.iter().copied())
        {
            if gray == 0 {
                continue;
            }
            overlay[..3].fill(gray);
            overlay[3] = base[3];
            // SetPixDw canonicalizes the 0xffffffff clear to transparent
            // black on the main surface (src/C4Surface.cpp:728-735).
            base.copy_from_slice(&[0, 0, 0, 0]);
        }
        CursorPortraitImages {
            base: ImageData::new(width, height, base),
            owner_overlay: Some(ImageData::new(width, height, overlay)),
        }
    } else if mask.len() == base.len() {
        // An explicit OverlayN.png is the complete second RGBA surface.
        CursorPortraitImages {
            base: ImageData::new(width, height, base),
            owner_overlay: Some(ImageData::new(width, height, mask.to_vec())),
        }
    } else {
        CursorPortraitImages {
            base: ImageData::from_arc(width, height, pixels),
            owner_overlay: None,
        }
    }
}

pub(crate) fn player_join_board_line(player_name: &str) -> String {
    const PREFIX: &str = "Player join: ";
    format!("{PREFIX}{}", c4_presentation_text(player_name))
}

pub(crate) fn lobby_chat_selection(view: &LobbyChatEditView) -> Option<std::ops::Range<usize>> {
    view.selection.and_then(|(anchor, caret)| {
        let start = anchor.min(caret);
        let end = anchor.max(caret);
        (start != end).then_some(start..end)
    })
}

pub(crate) fn lobby_chat_delete_selection(view: &mut LobbyChatEditView) -> bool {
    let Some(range) = lobby_chat_selection(view) else {
        return false;
    };
    let start = range.start;
    view.text.replace_range(range, "");
    view.caret = start;
    view.selection = None;
    true
}

pub(crate) fn lobby_chat_clear_preserving_scroll(view: &mut LobbyChatEditView) {
    let horizontal_scroll = view.horizontal_scroll;
    let cursor_visible = if view.text.is_empty() {
        view.cursor_visible
    } else {
        true
    };
    *view = LobbyChatEditView {
        horizontal_scroll,
        cursor_visible,
        ..LobbyChatEditView::default()
    };
}

pub(crate) fn lobby_chat_context_entries(
    view: &LobbyChatEditView,
    clipboard_available: bool,
) -> Vec<ContextMenuEntry<AppContextMenuCommand>> {
    let labels = InputDialogContextLabels::default();
    let entry = |label: &str, tooltip: &str, command: LobbyChatContextCommand| {
        ContextMenuEntry::new(label)
            .with_tooltip(tooltip)
            .with_icon(ContextMenuIcon::None)
            .with_action(AppContextMenuCommand::LobbyChat(command))
    };
    let selection = lobby_chat_selection(view);
    let mut entries = Vec::new();
    if selection.is_some() {
        entries.push(entry(
            &labels.cut,
            &labels.cut_tooltip,
            LobbyChatContextCommand::Cut,
        ));
        entries.push(entry(
            &labels.copy,
            &labels.copy_tooltip,
            LobbyChatContextCommand::Copy,
        ));
    }
    if clipboard_available {
        entries.push(entry(
            &labels.paste,
            &labels.paste_tooltip,
            LobbyChatContextCommand::Paste,
        ));
    }
    if selection.is_some() {
        entries.push(entry(
            &labels.clear,
            &labels.clear_tooltip,
            LobbyChatContextCommand::Clear,
        ));
    }
    let whole_text_selected = selection
        .as_ref()
        .is_some_and(|range| range.start == 0 && range.end == view.text.len());
    if !view.text.is_empty() && !whole_text_selected {
        entries.push(entry(
            &labels.select_all,
            &labels.select_all_tooltip,
            LobbyChatContextCommand::SelectAll,
        ));
    }
    entries
}

fn lobby_chat_insert_text_impl(
    view: &mut LobbyChatEditView,
    text: &str,
    preserve_control_characters: bool,
) -> bool {
    const CPP_EDIT_MAX_BYTES: usize = 254;
    lobby_chat_delete_selection(view);
    let mut remaining =
        CPP_EDIT_MAX_BYTES.saturating_sub(clonk_script::c4_string_byte_len(&view.text));
    let mut sanitized = String::new();
    for character in text
        .chars()
        .take_while(|character| *character != '\0')
        .filter(|character| preserve_control_characters || !character.is_ascii_control())
    {
        let character = if character == '|' { '¦' } else { character };
        let width = clonk_script::c4_string_byte_len(&character.to_string());
        if width > remaining {
            break;
        }
        sanitized.push(character);
        remaining -= width;
    }
    view.text.insert_str(view.caret, &sanitized);
    view.caret += sanitized.len();
    view.selection = None;
    view.cursor_visible = true;
    !sanitized.is_empty()
}

pub(crate) fn lobby_chat_insert_text(view: &mut LobbyChatEditView, text: &str) -> bool {
    lobby_chat_insert_text_impl(view, text, false)
}

fn lobby_chat_insert_pasted_text(view: &mut LobbyChatEditView, text: &str) -> bool {
    lobby_chat_insert_text_impl(view, text, true)
}

fn lobby_chat_character_at(
    view: &LobbyChatEditView,
    control_x: i32,
    font: &clonk_graphics::clonk_font::ClonkFont,
) -> usize {
    let mut previous_width = 0;
    for (index, character) in view.text.char_indices() {
        let end = index + character.len_utf8();
        let width = font.measure(&view.text[..end], false).0;
        if width - (width - previous_width) / 2 >= control_x {
            return index;
        }
        previous_width = width;
    }
    view.text.len()
}

pub(crate) fn lobby_chat_scroll_caret_in_view(
    view: &mut LobbyChatEditView,
    layout: &LobbyLayout,
    font: &clonk_graphics::clonk_font::ClonkFont,
) {
    let client_width = (layout.chat_edit.w - 8).max(0);
    if client_width < 5 {
        return;
    }
    let caret_x =
        font.measure(&view.text[..view.caret], false).0 + font.measure("\u{a6}", false).0 / 2;
    if caret_x < view.horizontal_scroll && view.horizontal_scroll > 0 {
        view.horizontal_scroll = caret_x.saturating_sub(2).max(0);
    }
    if caret_x > view.horizontal_scroll
        && caret_x > client_width.saturating_add(view.horizontal_scroll)
    {
        view.horizontal_scroll =
            caret_x.saturating_sub(client_width) + i32::from(view.caret < view.text.len()) * 2;
    }
}

fn lobby_chat_pointer_character(
    view: &LobbyChatEditView,
    point: GuiPoint,
    layout: &LobbyLayout,
    font: &clonk_graphics::clonk_font::ClonkFont,
) -> usize {
    let control_x = point.x.floor() as i32 - (layout.chat_edit.x + 4) + view.horizontal_scroll;
    lobby_chat_character_at(view, control_x, font)
}

pub(crate) fn lobby_chat_apply_pointer_selection(
    view: &mut LobbyChatEditView,
    point: GuiPoint,
    layout: &LobbyLayout,
    font: &clonk_graphics::clonk_font::ClonkFont,
    begin: bool,
    retained_anchor: Option<usize>,
) -> (usize, usize) {
    let previous_caret = view.caret;
    let position = lobby_chat_pointer_character(view, point, layout, font);
    let anchor = if begin {
        position
    } else {
        view.selection
            .map(|(anchor, _)| anchor)
            .or(retained_anchor)
            .unwrap_or(view.caret)
    };
    view.caret = position;
    view.selection = (anchor != position).then_some((anchor, position));
    view.cursor_visible = true;
    if previous_caret != position {
        lobby_chat_scroll_caret_in_view(view, layout, font);
    }
    (position, anchor)
}

pub(crate) fn lobby_chat_apply_double_click(
    view: &mut LobbyChatEditView,
    point: GuiPoint,
    layout: &LobbyLayout,
    font: &clonk_graphics::clonk_font::ClonkFont,
) {
    let previous_caret = view.caret;
    let is_spacer = |character: Option<char>| {
        character.is_none_or(|character| {
            character.is_ascii() && !character.is_ascii_alphanumeric() && character != '_'
        })
    };
    let mut position = lobby_chat_pointer_character(view, point, layout, font);
    let character = view.text[position..].chars().next();
    if is_spacer(character) {
        if position == 0 {
            return;
        }
        let previous = lobby_chat_previous_boundary(&view.text, position);
        if is_spacer(view.text[previous..position].chars().next()) {
            return;
        }
        position = previous;
    }

    let mut start = position;
    while start > 0 {
        let previous = lobby_chat_previous_boundary(&view.text, start);
        if is_spacer(view.text[previous..start].chars().next()) {
            break;
        }
        start = previous;
    }
    let mut end = lobby_chat_next_boundary(&view.text, position);
    while end < view.text.len() {
        let next = lobby_chat_next_boundary(&view.text, end);
        if is_spacer(view.text[end..next].chars().next()) {
            break;
        }
        end = next;
    }
    view.caret = end;
    view.selection = Some((start, end));
    view.cursor_visible = true;
    if previous_caret != end {
        lobby_chat_scroll_caret_in_view(view, layout, font);
    }
}

pub(crate) fn lobby_chat_insert_primary_text(view: &mut LobbyChatEditView, text: &str) -> bool {
    const CPP_EDIT_MAX_BYTES: usize = 254;
    lobby_chat_delete_selection(view);
    let mut remaining =
        CPP_EDIT_MAX_BYTES.saturating_sub(clonk_script::c4_string_byte_len(&view.text));
    let mut inserted = String::new();
    for character in text.chars().take_while(|character| *character != '\0') {
        let width = clonk_script::c4_string_byte_len(&character.to_string());
        if width > remaining {
            break;
        }
        inserted.push(character);
        remaining -= width;
    }
    view.text.insert_str(view.caret, &inserted);
    view.caret += inserted.len();
    view.selection = None;
    view.cursor_visible = true;
    !inserted.is_empty()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LobbyChatPasteMode {
    Lobby,
    Running,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct LobbyChatPasteOutcome {
    close: bool,
    pub(crate) completed_lines: usize,
    pub(crate) stopped: bool,
}

pub(crate) fn lobby_chat_paste_attempts_insertion(clipboard: &str) -> bool {
    clipboard
        .split_once('\0')
        .map_or(clipboard, |(head, _)| head)
        .chars()
        .any(|character| !matches!(character, '\r' | '\n'))
}

pub(crate) fn lobby_chat_paste_text<E>(
    view: &mut LobbyChatEditView,
    clipboard: &str,
    mode: LobbyChatPasteMode,
    mut scroll_inserted_text: impl FnMut(&mut LobbyChatEditView),
    mut finish_input: impl FnMut(String) -> Result<bool, E>,
) -> Result<LobbyChatPasteOutcome, E> {
    let mut outcome = LobbyChatPasteOutcome::default();
    let mut rest = clipboard
        .split_once('\0')
        .map_or(clipboard, |(head, _)| head);
    while let Some(line_break) = rest.find(['\r', '\n']) {
        if line_break == 0 {
            rest = &rest[1..];
            continue;
        }
        if lobby_chat_insert_pasted_text(view, &rest[..line_break]) {
            scroll_inserted_text(view);
        }
        rest = &rest[line_break + 1..];
        let submission = view.text.clone();
        match mode {
            LobbyChatPasteMode::Lobby => lobby_chat_clear_preserving_scroll(view),
            LobbyChatPasteMode::Running if !rest.is_empty() => {
                view.caret = view.text.len();
                view.selection = (!view.text.is_empty()).then_some((0, view.text.len()));
                view.cursor_visible = true;
            }
            LobbyChatPasteMode::Running => {
                outcome.close = true;
            }
        }
        outcome.completed_lines += 1;
        if !finish_input(submission)? {
            outcome.stopped = true;
            return Ok(outcome);
        }
        if outcome.close {
            return Ok(outcome);
        }
    }
    if !rest.is_empty() && lobby_chat_insert_pasted_text(view, rest) {
        scroll_inserted_text(view);
    }
    Ok(outcome)
}

fn lobby_chat_previous_boundary(text: &str, at: usize) -> usize {
    text[..at]
        .char_indices()
        .next_back()
        .map(|(index, _)| index)
        .unwrap_or(0)
}

fn lobby_chat_next_boundary(text: &str, at: usize) -> usize {
    text[at..]
        .chars()
        .next()
        .map(|character| at + character.len_utf8())
        .unwrap_or(text.len())
}

fn lobby_chat_word_target(view: &LobbyChatEditView, direction: i32) -> usize {
    let is_spacer = |character: char| {
        character.is_ascii() && !character.is_ascii_alphanumeric() && character != '_'
    };
    if direction < 0 {
        let mut cursor = view.caret;
        let mut nonspace_found = false;
        while cursor > 0 {
            let previous = lobby_chat_previous_boundary(&view.text, cursor);
            let character = view.text[previous..cursor]
                .chars()
                .next()
                .expect("non-empty character slice");
            if is_spacer(character) {
                if nonspace_found {
                    break;
                }
            } else {
                nonspace_found = true;
            }
            cursor = previous;
        }
        cursor
    } else {
        let mut cursor = view.caret;
        let mut space_found = false;
        while cursor < view.text.len() {
            let next = lobby_chat_next_boundary(&view.text, cursor);
            let character = view.text[cursor..next]
                .chars()
                .next()
                .expect("non-empty character slice");
            if is_spacer(character) {
                space_found = true;
            } else if space_found {
                break;
            }
            cursor = next;
        }
        cursor
    }
}

pub(crate) fn lobby_chat_apply_edit_key(
    view: &mut LobbyChatEditView,
    key: LobbyChatEditKey,
    modifiers: LobbyChatKeyModifiers,
) -> bool {
    // C4GUI::Edit::KeyCursorOp returns immediately after deleting an active
    // selection. Every other recognized cursor operation reaches
    // ScrollCursorInView, even when it does not move the caret.
    let deleted_selection_without_scroll =
        matches!(key, LobbyChatEditKey::Backspace | LobbyChatEditKey::Delete)
            && lobby_chat_selection(view).is_some();
    let old_caret = view.caret;
    let target = match key {
        LobbyChatEditKey::Left => Some(if modifiers.control {
            lobby_chat_word_target(view, -1)
        } else {
            lobby_chat_previous_boundary(&view.text, view.caret)
        }),
        LobbyChatEditKey::Right => Some(if modifiers.control {
            lobby_chat_word_target(view, 1)
        } else {
            lobby_chat_next_boundary(&view.text, view.caret)
        }),
        LobbyChatEditKey::Home => Some(0),
        LobbyChatEditKey::End => Some(view.text.len()),
        LobbyChatEditKey::Backspace => {
            if !lobby_chat_delete_selection(view) && !modifiers.shift && view.caret > 0 {
                let start = if modifiers.control {
                    lobby_chat_word_target(view, -1)
                } else {
                    lobby_chat_previous_boundary(&view.text, view.caret)
                };
                view.text.replace_range(start..view.caret, "");
                view.caret = start;
            }
            view.selection = None;
            None
        }
        LobbyChatEditKey::Delete => {
            if !lobby_chat_delete_selection(view)
                && !modifiers.shift
                && view.caret < view.text.len()
            {
                let end = if modifiers.control {
                    lobby_chat_word_target(view, 1)
                } else {
                    lobby_chat_next_boundary(&view.text, view.caret)
                };
                view.text.replace_range(view.caret..end, "");
            }
            view.selection = None;
            None
        }
    };
    if let Some(target) = target {
        if modifiers.shift {
            let anchor = view
                .selection
                .map(|(anchor, _)| anchor)
                .unwrap_or(old_caret);
            view.caret = target;
            view.selection = (anchor != target).then_some((anchor, target));
        } else {
            view.caret = target;
            view.selection = None;
        }
    }
    view.cursor_visible = true;
    !deleted_selection_without_scroll
}

fn legacy_message_whitespace(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\r' | b'\n')
}

pub(crate) fn legacy_sscanf_decimal_prefix(value: &[u8]) -> Option<i32> {
    let value = value
        .iter()
        .position(|byte| !legacy_message_whitespace(*byte))
        .map(|start| &value[start..])?;
    let sign = usize::from(matches!(value.first(), Some(b'+' | b'-')));
    let digits = value[sign..]
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    if digits == 0 {
        return None;
    }
    std::str::from_utf8(&value[..sign + digits])
        .ok()?
        .parse()
        .ok()
}

pub(crate) fn legacy_sscanf_hex_prefix(value: &[u8]) -> Option<u32> {
    let value = value
        .iter()
        .position(|byte| !legacy_message_whitespace(*byte))
        .map(|start| &value[start..])?;
    let negative = value.first() == Some(&b'-');
    let sign = usize::from(matches!(value.first(), Some(b'+' | b'-')));
    let digits_start = if value
        .get(sign..sign.saturating_add(2))
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"0x"))
    {
        sign + 2
    } else {
        sign
    };
    let digits = value[digits_start..]
        .iter()
        .take_while(|byte| byte.is_ascii_hexdigit())
        .count();
    if digits == 0 {
        return None;
    }
    let magnitude = u32::from_str_radix(
        std::str::from_utf8(&value[digits_start..digits_start + digits]).ok()?,
        16,
    )
    .ok()?;
    Some(if negative {
        magnitude.wrapping_neg()
    } else {
        magnitude
    })
}

fn legacy_prefix_no_case(value: &[u8], prefix: &[u8]) -> bool {
    value
        .get(..prefix.len())
        .is_some_and(|candidate| candidate.eq_ignore_ascii_case(prefix))
}

pub(crate) fn is_team_message_syntax(text: &str) -> bool {
    let raw = clonk_script::c4_string_bytes(text);
    raw.first() == Some(&b'^')
        || legacy_prefix_no_case(&raw, b"team:")
        || legacy_prefix_no_case(&raw, b"/team ")
}

pub(crate) fn control_player_effective_name(
    player: &clonk_engine::ControlPlayerInfoEntry,
) -> &[u8] {
    if !player.league_account.is_empty() {
        player.league_account.as_bytes()
    } else if !player.forced_name.is_empty() {
        player.forced_name.as_bytes()
    } else {
        player.name.as_bytes()
    }
}

pub(crate) fn parse_lobby_message_control(text: &str) -> Result<Option<MessageControlData>> {
    const C4_MAX_MESSAGE: usize = 256;
    let raw = clonk_script::c4_string_bytes(text);
    if raw.is_empty() {
        return Ok(None);
    }
    let (message_type, mut message) = if raw[0] == b'^' {
        (MESSAGE_TYPE_TEAM, &raw[1..])
    } else if legacy_prefix_no_case(&raw, b"team:") {
        (MESSAGE_TYPE_TEAM, &raw[5..])
    } else if legacy_prefix_no_case(&raw, b"/team ") {
        (MESSAGE_TYPE_TEAM, &raw[6..])
    } else if legacy_prefix_no_case(&raw, b"/me ") {
        (MESSAGE_TYPE_ME, &raw[4..])
    } else if legacy_prefix_no_case(&raw, b"/sound ") {
        (MESSAGE_TYPE_SOUND, &raw[7..])
    } else if raw.eq_ignore_ascii_case(b"/alert") {
        (MESSAGE_TYPE_ALERT, &raw[raw.len()..])
    } else if legacy_prefix_no_case(&raw, b"/alert ") {
        (MESSAGE_TYPE_ALERT, &raw[7..])
    } else if raw[0] == b'/' {
        anyhow::bail!("unsupported classic lobby chat command");
    } else {
        (MESSAGE_TYPE_NORMAL, raw.as_slice())
    };
    while message
        .first()
        .copied()
        .is_some_and(legacy_message_whitespace)
    {
        message = &message[1..];
    }
    while message
        .last()
        .copied()
        .is_some_and(legacy_message_whitespace)
    {
        message = &message[..message.len() - 1];
    }
    if message.is_empty() && message_type != MESSAGE_TYPE_ALERT {
        return Ok(None);
    }
    let message = message[..message.len().min(C4_MAX_MESSAGE)].to_vec();
    let message = clonk_engine::LegacyCString::from_bytes(message)
        .context("classic lobby chat contains an interior NUL")?;
    Ok(Some(MessageControlData {
        message_type,
        player: -1,
        to_player: -1,
        message,
        by_client: -1,
    }))
}

pub(crate) fn parse_running_message_control(
    text: &str,
    player: i32,
    cinematic: bool,
    players: &SimulationSnapshot,
) -> Result<Option<MessageControlData>> {
    const C4_MAX_MESSAGE: usize = 256;
    let raw = clonk_script::c4_string_bytes(text);
    if raw.is_empty() {
        return Ok(None);
    }

    let mut to_player = -1;
    let (message_type, mut message) = if raw[0] == b'^' {
        (MESSAGE_TYPE_TEAM, raw[1..].to_vec())
    } else if legacy_prefix_no_case(&raw, b"team:") {
        (MESSAGE_TYPE_TEAM, raw[5..].to_vec())
    } else if legacy_prefix_no_case(&raw, b"/team ") {
        (MESSAGE_TYPE_TEAM, raw[6..].to_vec())
    } else if legacy_prefix_no_case(&raw, b"/private ") {
        let rest = &raw[9..];
        let Some(space) = rest.iter().position(|byte| *byte == b' ') else {
            return Ok(None);
        };
        let target_len = space.min(30);
        let target = &rest[..target_len];
        let Some(target_player) = players
            .players
            .iter()
            .find(|candidate| clonk_script::c4_string_bytes(&candidate.name) == target)
        else {
            return Ok(None);
        };
        to_player = target_player.id;
        (MESSAGE_TYPE_PRIVATE, rest[target_len + 1..].to_vec())
    } else if legacy_prefix_no_case(&raw, b"/me ") {
        (MESSAGE_TYPE_ME, raw[4..].to_vec())
    } else if legacy_prefix_no_case(&raw, b"/sound ") {
        (MESSAGE_TYPE_SOUND, raw[7..].to_vec())
    } else if raw.eq_ignore_ascii_case(b"/alert") {
        (MESSAGE_TYPE_ALERT, Vec::new())
    } else if legacy_prefix_no_case(&raw, b"/alert ") {
        (MESSAGE_TYPE_ALERT, raw[7..].to_vec())
    } else if raw[0] == b'"' {
        let mut message = raw;
        if message.last() != Some(&b'"') {
            message.push(b'"');
        }
        (MESSAGE_TYPE_SAY, message)
    } else if raw[0] == b'/' {
        anyhow::bail!("unsupported classic game chat command");
    } else {
        (MESSAGE_TYPE_NORMAL, raw)
    };

    while message
        .first()
        .copied()
        .is_some_and(legacy_message_whitespace)
    {
        message.remove(0);
    }
    while message
        .last()
        .copied()
        .is_some_and(legacy_message_whitespace)
    {
        message.pop();
    }
    if cinematic && message_type == MESSAGE_TYPE_SAY && message.len() >= 2 {
        message.remove(0);
        message.pop();
    }
    if message.is_empty() && message_type != MESSAGE_TYPE_ALERT {
        return Ok(None);
    }
    message.truncate(C4_MAX_MESSAGE);
    let message = clonk_engine::LegacyCString::from_bytes(message)
        .context("classic game chat contains an interior NUL")?;
    Ok(Some(MessageControlData {
        message_type,
        player,
        to_player,
        message,
        by_client: -1,
    }))
}

pub(crate) fn build_game_over_dialog(
    snapshot: &SimulationSnapshot,
    teams: &[clonk_engine::TeamInfo],
    auto_generate_teams: bool,
    local_owner: i32,
    screen_width: u32,
    host_or_cinematic_film: bool,
    title: String,
    next_mission: &clonk_engine::NextMissionState,
    mut goal_presentation: impl FnMut(&str, bool) -> (Option<ImageData>, String),
    mut player_big_icon: impl FnMut(i32) -> Option<ImageData>,
    // `C4PlayerInfo::getLeagueScore()` per PlayerInfo ID: the score carried
    // into this round, which UpdateScoreLabel reads from the live info rather
    // than from the frozen result (src/C4PlayerInfoListBox.cpp:380-401).
    mut player_league_score: impl FnMut(i32) -> Option<i32>,
    // `GetJoinedInfo()`'s lobby colour (src/C4PlayerInfoListBox.cpp:701-716):
    // the row's own colour for a free savegame player, otherwise the colour of
    // the `Game.RestorePlayerInfos` entry it took over, and `None` when the
    // row is not a savegame join at all.
    mut player_joined_color: impl FnMut(i32) -> Option<u32>,
    // `Game.DrawTextSpecImage(icon, IconSpec, team colour)` for a declared
    // team `IconSpec` (src/C4PlayerInfoListBox.cpp:1028-1031).
    mut team_icon: impl FnMut(&str, u32) -> Option<ImageData>,
    // `C4PlayerInfo::getLeagueRankSymbol()`.
    mut player_league_rank_symbol: impl FnMut(i32) -> Option<i32>,
    // `Game.Parameters.isLeague()`.
    league: bool,
) -> GameOverState {
    // C4GameOverDlg freezes C4RoundResults into presentation state; player
    // results are joined through C4PlayerInfo::ID, not the runtime player
    // number (C4GameOverDlg.cpp:145-220; C4PlayerInfoListBox.cpp:132-143,
    // 344-425,1529-1592).
    let goals = snapshot
        .round_results
        .goals
        .iter()
        .map(|definition_id| {
            let fulfilled = snapshot
                .round_results
                .fulfilled_goals
                .contains(definition_id);
            let (picture, tooltip) = goal_presentation(definition_id, fulfilled);
            EvaluationGoal {
                definition_id: definition_id.clone(),
                fulfilled,
                tooltip,
                picture,
            }
        })
        .collect();

    // Preserve player-info order in the frozen model. The layout performs the
    // winner/loser grouping for the unified list; fixed-team lists filter this
    // source order independently (C4PlayerInfoListBox.cpp:1529-1592).
    let mut players = Vec::new();
    for state in &snapshot.players {
        let Some(result) = snapshot
            .round_results
            .players
            .iter()
            .find(|result| result.player_info_id == state.player_info_id)
        else {
            continue;
        };
        let color = state
            .color
            .map(|RgbColor { r, g, b }| Color::opaque(r, g, b))
            .unwrap_or_else(|| default_owner_color(state.id));
        let won = state.team.map_or(state.won, |team_id| {
            if teams.iter().any(|team| team.id == team_id) {
                snapshot
                    .players
                    .iter()
                    .any(|candidate| candidate.team == Some(team_id) && candidate.won)
            } else {
                state.won
            }
        });
        players.push(EvaluationPlayer {
            player_info_id: state.player_info_id,
            team_id: state.team,
            name: if state.name.trim().is_empty() {
                format!("Player {}", state.id)
            } else {
                c4_presentation_text(&state.name)
            },
            won,
            color_dw: u32::from(color.r) << 16 | u32::from(color.g) << 8 | u32::from(color.b),
            total_playing_time: result.total_playing_time,
            score_old: if snapshot.round_results.hide_settlement_score {
                -1
            } else {
                result.score_old
            },
            score_new: (!snapshot.round_results.hide_settlement_score)
                .then_some(result.score_new)
                .flatten(),
            custom_evaluation_strings: c4_presentation_text(&result.custom_evaluation_strings),
            big_icon: player_big_icon(state.player_info_id),
            // C++ treats a zero league score as absent (`pInfo->getLeagueScore()`
            // is used as a boolean at src/C4PlayerInfoListBox.cpp:380).
            league_score_old: player_league_score(state.player_info_id).filter(|score| *score != 0),
            league_score_gain: (result.league_score_gain >= 0).then_some(result.league_score_gain),
            league_score_new: (result.league_score_new >= 0).then_some(result.league_score_new),
            joined_color_dw: player_joined_color(state.player_info_id),
            // The frozen result's rank wins while its league score is valid;
            // otherwise the live info's rank symbol, and zero hides the icon
            // (src/C4PlayerInfoListBox.cpp:439-456).
            league_rank_symbol: u8::try_from(
                (result.league_score_new >= 0)
                    .then_some(result.league_rank_symbol_new)
                    .filter(|symbol| *symbol != 0)
                    .or_else(|| player_league_rank_symbol(state.player_info_id))
                    .unwrap_or(0),
            )
            .ok()
            .filter(|symbol| *symbol != 0),
        });
    }
    let separate_team_ids =
        (teams.len() == 2 && !auto_generate_teams).then(|| [teams[0].id, teams[1].id]);
    // `C4Team::HasWon()` is what recolours a TeamListItem's caption
    // (src/C4PlayerInfoListBox.cpp:1100-1115). A team has won when any of its
    // players did, which is the same rule the per-player `won` projection
    // above already applies in the other direction.
    let evaluation_teams = teams
        .iter()
        .map(|team| clonk_app_menus::game_over::EvaluationTeam {
            id: team.id,
            name: c4_presentation_text(&team.name),
            color_dw: team.color,
            icon: team
                .icon_spec
                .as_deref()
                .and_then(|spec| team_icon(spec, team.color)),
            won: snapshot
                .players
                .iter()
                .any(|player| player.team == Some(team.id) && player.won),
        });
    let evaluation = EvaluationViewModel::new(goals, players)
        .with_dialog_context(
            c4_presentation_text(&snapshot.round_results.custom_evaluation_strings),
            separate_team_ids,
        )
        .with_team_order(teams.iter().map(|team| team.id))
        .with_teams(evaluation_teams)
        .with_league(league);

    // Keep the asset-less fallback usable, but derive it from the same frozen
    // evaluation instead of treating every still-Active player as a winner or
    // showing the unrelated in-round Points/Wealth/Value counters.
    let entries = evaluation
        .players()
        .map(|player| {
            let runtime = snapshot
                .players
                .iter()
                .find(|state| state.player_info_id == player.player_info_id);
            GameOverEntry {
                player_id: runtime.map_or(player.player_info_id, |state| state.id),
                name: player.name.clone(),
                outcome: if player.won {
                    GameOverOutcome::Victory
                } else {
                    GameOverOutcome::Defeat
                },
                wealth: 0,
                score: player.score_new.unwrap_or(player.score_old),
                value: 0,
                is_local: runtime.is_some_and(|state| state.id == local_owner),
                color: Some(Color::opaque(
                    ((player.color_dw >> 16) & 0xff) as u8,
                    ((player.color_dw >> 8) & 0xff) as u8,
                    (player.color_dw & 0xff) as u8,
                )),
            }
        })
        .collect();
    let next_mission = (!next_mission.path.is_empty()).then(|| NextMissionButton {
        label: next_mission.text.clone(),
        description: next_mission.description.clone(),
    });
    let mut dialog = GameOverState::with_next_mission(
        title,
        entries,
        screen_width,
        next_mission,
        host_or_cinematic_film,
    );
    dialog.set_evaluation(evaluation);
    dialog
}

pub(crate) fn configured_control_key_names(
    bindings: &KeyboardBindings,
) -> HashMap<i32, Vec<ControlKeyName>> {
    (0..4_usize)
        .map(|control_set| {
            let names = ControlBindingId::ALL
                .iter()
                .map(|&binding| {
                    let label = bindings
                        .key_for_set(control_set, binding)
                        .map(format_key_label)
                        .unwrap_or_default();
                    ControlKeyName::new(label.clone(), label)
                })
                .collect();
            (
                i32::try_from(control_set).expect("keyboard control set index fits i32"),
                names,
            )
        })
        .collect()
}

pub(crate) fn project_startup_irc_snapshot(
    server: &str,
    snapshot: clonk_network::IrcClientSnapshot,
) -> clonk_frontend::startup_netdlg::NetDlgChatSnapshot {
    use clonk_frontend::startup_netdlg::{
        NetDlgChatChannel, NetDlgChatConnectionState, NetDlgChatMessage, NetDlgChatMessageKind,
        NetDlgChatSnapshot, NetDlgChatUser,
    };

    let unread_index = snapshot.unread_index;
    let connection_state = match snapshot.connection_state {
        clonk_network::IrcConnectionState::Disconnected => NetDlgChatConnectionState::Disconnected,
        clonk_network::IrcConnectionState::Connecting => NetDlgChatConnectionState::Connecting,
        clonk_network::IrcConnectionState::Connected => NetDlgChatConnectionState::Connected,
    };
    let channels = snapshot
        .channels
        .into_iter()
        .map(|channel| NetDlgChatChannel {
            name: clonk_resources::decode_legacy_system_text(&channel.name),
            topic: clonk_resources::decode_legacy_system_text(&channel.topic),
            users: channel
                .users
                .into_iter()
                .map(|user| NetDlgChatUser {
                    prefix: clonk_resources::decode_legacy_system_text(&user.prefix),
                    name: clonk_resources::decode_legacy_system_text(&user.name),
                })
                .collect(),
        })
        .collect();
    let messages = snapshot
        .messages
        .into_iter()
        .map(|message| {
            let is_channel = message.is_channel();
            let kind = match message.message_type {
                clonk_network::IrcMessageType::Server => NetDlgChatMessageKind::Server,
                clonk_network::IrcMessageType::Status => NetDlgChatMessageKind::Status,
                clonk_network::IrcMessageType::Message => NetDlgChatMessageKind::Message,
                clonk_network::IrcMessageType::Notice => NetDlgChatMessageKind::Notice,
                clonk_network::IrcMessageType::Action => NetDlgChatMessageKind::Action,
            };
            NetDlgChatMessage {
                kind,
                source: clonk_resources::decode_legacy_system_text(&message.source),
                target: clonk_resources::decode_legacy_system_text(&message.target),
                text: clonk_resources::decode_legacy_system_text(&message.data),
                is_channel,
            }
        })
        .collect();
    NetDlgChatSnapshot {
        connection_state,
        server: server.to_string(),
        nick: clonk_resources::decode_legacy_system_text(&snapshot.nick),
        channels,
        messages,
        unread_index,
        last_error: snapshot.last_error,
    }
}

pub(crate) fn encode_startup_irc_text(text: &str) -> Option<Vec<u8>> {
    clonk_resources::encode_legacy_script_text(text)
}

pub(crate) fn project_startup_irc_command(
    command: clonk_frontend::startup_netdlg::NetDlgChatCommand,
) -> Option<clonk_network::IrcCommand> {
    use clonk_frontend::startup_netdlg::NetDlgChatCommand;

    Some(match command {
        NetDlgChatCommand::Quit { reason } => clonk_network::IrcCommand::Quit {
            reason: encode_startup_irc_text(&reason)?,
        },
        NetDlgChatCommand::Join { channel } => clonk_network::IrcCommand::Join {
            channel: encode_startup_irc_text(&channel)?,
        },
        NetDlgChatCommand::Part { channel } => clonk_network::IrcCommand::Part {
            channel: encode_startup_irc_text(&channel)?,
        },
        NetDlgChatCommand::Message { target, text } => clonk_network::IrcCommand::Message {
            target: encode_startup_irc_text(&target)?,
            text: encode_startup_irc_text(&text)?,
        },
        NetDlgChatCommand::Notice { target, text } => clonk_network::IrcCommand::Notice {
            target: encode_startup_irc_text(&target)?,
            text: encode_startup_irc_text(&text)?,
        },
        NetDlgChatCommand::Action { target, text } => clonk_network::IrcCommand::Action {
            target: encode_startup_irc_text(&target)?,
            text: encode_startup_irc_text(&text)?,
        },
        NetDlgChatCommand::Raw(line) => {
            clonk_network::IrcCommand::Raw(encode_startup_irc_text(&line)?)
        }
        NetDlgChatCommand::ChangeNick { nick } => clonk_network::IrcCommand::ChangeNick {
            nick: encode_startup_irc_text(&nick)?,
        },
        // Opening a query tab is presentation-only; the first message is sent
        // separately if the user enters one.
        NetDlgChatCommand::OpenQuery { .. } => return None,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SynchronizedPlayerFilePolicy {
    Skip,
    BlockedRemote,
    Persist { local_control: bool },
}

/// The two-stage eligibility in `C4PlayerList::SynchronizeLocalFiles` and
/// `C4Player::Save`: eliminated/script players are successful no-ops, while
/// an ineligible remote player reaches Save and contributes a false result.
pub(crate) fn synchronized_player_file_policy(
    status: clonk_engine::PlayerStatus,
    script_player: bool,
    at_client: i32,
    local_client_id: i32,
    league: bool,
    max_players: Option<i32>,
) -> SynchronizedPlayerFilePolicy {
    if script_player
        || matches!(
            status,
            clonk_engine::PlayerStatus::Eliminated | clonk_engine::PlayerStatus::Surrendered
        )
    {
        return SynchronizedPlayerFilePolicy::Skip;
    }
    let local_control = at_client == local_client_id;
    if !local_control && (league || max_players.is_some_and(|max_players| max_players <= 0)) {
        return SynchronizedPlayerFilePolicy::BlockedRemote;
    }
    SynchronizedPlayerFilePolicy::Persist { local_control }
}
