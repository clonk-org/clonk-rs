//! Pure C4Group mutation for developer-console scenario and savegame writes.
//!
//! The engine serializes simulation-owned components. This module applies
//! those components to the scenario copy while retaining the source entries
//! that native `C4GameSave` leaves untouched.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use clonk_engine::{
    LiveC4CrewProfileCleanup, LiveC4SaveComponentMutation, LiveC4SaveComponents,
    LiveC4SaveLandscapeMutation, LiveC4SavePolicy, LiveC4SavePreLandscapeComponents,
    LiveC4SaveScenarioSectionMutation,
};
use clonk_resources::{Group, GroupEntry, MutableGroup, MutableGroupChildMut};

use crate::runtime_join_save::SerializedRuntimeJoinPlayerGroup;

const C4FLS_PLAYER: &str = "Player.txt|Portrait.png|Portrait.bmp|*.c4i";

/// Whether a failed physical add aborts a folder-backed live save.
///
/// Packed C4Groups defer every add to `Close`; folder groups execute it at
/// the call site.  The journal retains the caller's treatment of that return
/// value so direct folder replay can stop at the same component boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderSaveAddFailure {
    Fatal,
    Ignore,
}

/// One physical mutation performed immediately by a native `GRPF_Folder`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FolderSaveMutation {
    /// `C4Group::Delete`, including wildcard and segmented file specs.
    DeletePattern { pattern: Vec<u8> },
    /// `C4Group::DeleteEntry`, which uses the literal child path for folders.
    DeleteEntry { name: Vec<u8> },
    /// `C4Group::Add`/`Move` of an ordinary component buffer.
    PutFile {
        name: Vec<u8>,
        payload: Vec<u8>,
        failure: FolderSaveAddFailure,
    },
    /// `C4Group::Add`/`Move` of a packed child-group image.
    PutChild {
        name: Vec<u8>,
        raw_image: Vec<u8>,
        failure: FolderSaveAddFailure,
    },
    /// `C4Landscape::SaveTextures`' in-place Material.c4g update.
    MergeMaterialGroup { raw_patch: Vec<u8> },
}

/// Ordered folder mutations accumulated alongside the packed-group mirror.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderSaveJournal {
    enabled: bool,
    mutations: Vec<FolderSaveMutation>,
}

impl Default for FolderSaveJournal {
    fn default() -> Self {
        Self {
            enabled: true,
            mutations: Vec::new(),
        }
    }
}

impl FolderSaveJournal {
    /// A zero-copy sink for packed targets, whose mutations remain virtual
    /// until C4Group close and therefore need no folder replay.
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            mutations: Vec::new(),
        }
    }

    pub fn mutations(&self) -> &[FolderSaveMutation] {
        &self.mutations
    }

    pub fn delete_pattern(&mut self, pattern: impl AsRef<[u8]>) {
        if !self.enabled {
            return;
        }
        self.mutations.push(FolderSaveMutation::DeletePattern {
            pattern: pattern.as_ref().to_vec(),
        });
    }

    pub fn delete_entry(&mut self, name: impl AsRef<[u8]>) {
        if !self.enabled {
            return;
        }
        self.mutations.push(FolderSaveMutation::DeleteEntry {
            name: name.as_ref().to_vec(),
        });
    }

    pub fn put_file(
        &mut self,
        name: impl AsRef<[u8]>,
        payload: impl AsRef<[u8]>,
        failure: FolderSaveAddFailure,
    ) {
        if !self.enabled {
            return;
        }
        self.mutations.push(FolderSaveMutation::PutFile {
            name: name.as_ref().to_vec(),
            payload: payload.as_ref().to_vec(),
            failure,
        });
    }

    pub fn put_child(
        &mut self,
        name: impl AsRef<[u8]>,
        raw_image: impl AsRef<[u8]>,
        failure: FolderSaveAddFailure,
    ) {
        if !self.enabled {
            return;
        }
        self.mutations.push(FolderSaveMutation::PutChild {
            name: name.as_ref().to_vec(),
            raw_image: raw_image.as_ref().to_vec(),
            failure,
        });
    }

    pub fn merge_material_group(&mut self, raw_patch: impl AsRef<[u8]>) {
        if !self.enabled {
            return;
        }
        self.mutations.push(FolderSaveMutation::MergeMaterialGroup {
            raw_patch: raw_patch.as_ref().to_vec(),
        });
    }
}

/// Overlay a freshly serialized live player onto a copy of its local profile.
///
/// Native `C4Player::Save` first copies a local `.c4p`, then rewrites
/// `Player.txt` and each saved crew child in that copy. Consequently custom
/// root files and custom files inside an existing `.c4i` survive. The live
/// serializer intentionally emits only simulation-owned entries, so replacing
/// whole crew children here would lose that local profile data.
///
/// The returned group retains the original header and maker. Neither input is
/// modified if opening or merging a serialized child fails.
pub fn overlay_live_player_group(
    original: &Group,
    serialized_live_player: &MutableGroup,
) -> Result<MutableGroup> {
    overlay_live_player_group_with_cleanup(original, serialized_live_player, &[])
}

/// Overlay a local profile and apply the destructive crew-entry operations
/// that `C4ObjectInfo::Save` performs on the copied child groups.
pub fn overlay_live_player_group_with_cleanup(
    original: &Group,
    serialized_live_player: &MutableGroup,
    crew_cleanup: &[LiveC4CrewProfileCleanup],
) -> Result<MutableGroup> {
    let serialized_image = serialized_live_player
        .pack_raw()
        .context("pack serialized live player overlay")?;
    let serialized =
        Group::from_raw_memory(PathBuf::from("SerializedLivePlayer.c4p"), serialized_image)
            .context("open serialized live player overlay")?;
    let mut merged =
        overlay_group_copy(original, &serialized, "player profile", Some(crew_cleanup))?;
    // C4PlayerInfoCore::Save replaces Player.txt and removes the obsolete
    // binary core even though it is not part of the new serialization.
    merged.remove_entry("C4Player.c4b");
    // C4Player::Save(C4Group &, ...) explicitly performs this sort after
    // saving the core and crew, independent of the global C4CFN_FLS table.
    merged.sort(C4FLS_PLAYER);
    Ok(merged)
}

fn overlay_group_copy(
    original: &Group,
    overlay: &Group,
    context: &str,
    crew_cleanup: Option<&[LiveC4CrewProfileCleanup]>,
) -> Result<MutableGroup> {
    let mut merged =
        MutableGroup::from_group(original).with_context(|| format!("copy original {context}"))?;
    let original_entries = original
        .entries()
        .with_context(|| format!("enumerate original {context}"))?;

    for entry in overlay
        .entries()
        .with_context(|| format!("enumerate serialized {context}"))?
    {
        let entry_name = String::from_utf8_lossy(&entry.name_bytes).into_owned();
        if entry.is_directory {
            let overlay_child = open_child_entry_exact(overlay, &entry)
                .with_context(|| format!("open serialized {context} child {entry_name}"))?;
            let profile_cleanup = crew_cleanup.and_then(|cleanup| {
                cleanup
                    .iter()
                    .find(|cleanup| cleanup.filename.eq_ignore_ascii_case(&entry.name_bytes))
            });
            let original_name = profile_cleanup
                .filter(|cleanup| !cleanup.original_filename.is_empty())
                .map_or(entry.name_bytes.as_slice(), |cleanup| {
                    cleanup.original_filename.as_slice()
                });
            let original_child = original_entries.iter().find(|candidate| {
                candidate.is_directory && candidate.name_bytes.eq_ignore_ascii_case(original_name)
            });
            let child_context = format!("{context} child {entry_name}");
            let mut child = if let Some(original_child) = original_child {
                let original_child = open_child_entry_exact(original, original_child)
                    .with_context(|| format!("open original {child_context}"))?;
                overlay_group_copy(&original_child, &overlay_child, &child_context, None)?
            } else {
                MutableGroup::from_group(&overlay_child)
                    .with_context(|| format!("copy serialized {child_context}"))?
            };
            if let Some(cleanup) = profile_cleanup {
                // C4ObjectInfo.cpp:240 gates both deletions on the presence
                // of Portrait.png. A lone overlay or legacy BMP survives.
                if cleanup.remove_default_portrait_png
                    && child
                        .entry_names()
                        .iter()
                        .any(|name| name.eq_ignore_ascii_case("Portrait.png"))
                {
                    child.remove_entry("Portrait.png");
                    child.remove_entry("PortraitOverlay.png");
                }
                if cleanup.remove_rank_png {
                    child.remove_entry("Rank.png");
                }
                if !cleanup.original_filename.is_empty()
                    && !cleanup
                        .original_filename
                        .eq_ignore_ascii_case(&cleanup.filename)
                {
                    // C4ObjectInfo::Save renames the copied child before
                    // extracting and rewriting it. Remove the old root name
                    // after merging its arbitrary profile-only contents.
                    merged.remove_entry_bytes(&cleanup.original_filename);
                }
            }
            merged
                .add_child_bytes_with_metadata(
                    entry.name_bytes,
                    child,
                    entry.time,
                    entry.executable,
                )
                .with_context(|| format!("write serialized {child_context}"))?;
        } else {
            let data = overlay
                .read_entry_bytes_exact(&entry)
                .with_context(|| format!("read serialized {context} entry {entry_name}"))?;
            merged
                .add_file_bytes_with_metadata(entry.name_bytes, data, entry.time, entry.executable)
                .with_context(|| format!("write serialized {context} entry {entry_name}"))?;
        }
    }
    Ok(merged)
}

fn open_child_entry_exact(group: &Group, entry: &GroupEntry) -> Result<Group> {
    if group.is_directory() {
        return group
            .open_child(&entry.relative_path)
            .context("open directory child group");
    }
    let data = group
        .read_entry_bytes_exact(entry)
        .context("read packed child group image")?;
    Group::from_raw_memory(
        PathBuf::from(String::from_utf8_lossy(&entry.name_bytes).into_owned()),
        data,
    )
    .context("open packed child group image")
}

/// Compose the exact `C4GameSave::SaveDesc` RTF envelope around the already
/// localized savegame description lines. Native strings are bytes in the
/// configured Clonk charset; RTF escaping therefore operates on bytes too.
pub fn serialize_savegame_description(
    title: &[u8],
    charset_code: u8,
    lines: &[(Vec<u8>, bool)],
) -> Vec<u8> {
    let mut description = format!(
        "{{\\rtf1\\ansi\\ansicpg1252\\deff0\\deflang1031{{\\fonttbl {{\\f0\\fnil\\fcharset{charset_code} Times New Roman;}}}}\r\n\\uc1\\pard\\ulnone\\b\\f0\\fs20 "
    )
    .into_bytes();
    append_rtf_escaped(&mut description, title);
    description.extend_from_slice(b"\\par\r\n\\b0\\fs16\\par\r\n");
    for (line, terminate_paragraph) in lines {
        append_rtf_escaped(&mut description, line);
        if *terminate_paragraph {
            description.extend_from_slice(b"\\par\r\n");
        }
    }
    // `EndOfFile` is spelled `"\\x020"` by C4Strings.h. C/C++ consumes all
    // three hexadecimal digits, so the terminating byte is a space (0x20).
    description.extend_from_slice(b"\r\n}\r\n ");
    description
}

/// Substitute the integer formats used by IDS_DESC_DATE and
/// IDS_DESC_DURATION while preserving every other native byte.
pub fn format_resource_integers(template: &[u8], arguments: &[i32]) -> Vec<u8> {
    let mut output = Vec::with_capacity(template.len());
    let mut cursor = 0;
    let mut argument = 0;
    while cursor < template.len() {
        if template[cursor] != b'%' {
            output.push(template[cursor]);
            cursor += 1;
            continue;
        }
        if template.get(cursor + 1) == Some(&b'%') {
            output.push(b'%');
            cursor += 2;
            continue;
        }

        let start = cursor;
        cursor += 1;
        let zero_pad = template.get(cursor) == Some(&b'0');
        if zero_pad {
            cursor += 1;
        }
        let width_start = cursor;
        while template.get(cursor).is_some_and(u8::is_ascii_digit) {
            cursor += 1;
        }
        let Some(conversion) = template.get(cursor).copied() else {
            output.extend_from_slice(&template[start..]);
            break;
        };
        if !matches!(conversion, b'd' | b'i') || argument >= arguments.len() {
            output.extend_from_slice(&template[start..=cursor]);
            cursor += 1;
            continue;
        }
        let width = std::str::from_utf8(&template[width_start..cursor])
            .ok()
            .and_then(|width| width.parse::<usize>().ok())
            .unwrap_or(0);
        let value = arguments[argument];
        argument += 1;
        let formatted = if zero_pad && width > 0 {
            format!("{value:0width$}")
        } else if width > 0 {
            format!("{value:width$}")
        } else {
            value.to_string()
        };
        output.extend_from_slice(formatted.as_bytes());
        cursor += 1;
    }
    output
}

/// Substitute byte-string `%s` fields without transcoding the localized
/// resource or its arguments.
pub fn format_resource_strings(template: &[u8], arguments: &[&[u8]]) -> Vec<u8> {
    let mut output = Vec::with_capacity(template.len());
    let mut cursor = 0;
    let mut argument = 0;
    while cursor < template.len() {
        if template[cursor] != b'%' {
            output.push(template[cursor]);
            cursor += 1;
            continue;
        }
        match template.get(cursor + 1).copied() {
            Some(b'%') => {
                output.push(b'%');
                cursor += 2;
            }
            Some(b's') if argument < arguments.len() => {
                output.extend_from_slice(arguments[argument]);
                argument += 1;
                cursor += 2;
            }
            Some(_) => {
                output.extend_from_slice(&template[cursor..cursor + 2]);
                cursor += 2;
            }
            None => {
                output.push(b'%');
                cursor += 1;
            }
        }
    }
    output
}

fn append_rtf_escaped(output: &mut Vec<u8>, input: &[u8]) {
    for byte in input.iter().copied() {
        if matches!(byte, b'\\' | b'{' | b'}') {
            output.push(b'\\');
        }
        output.push(byte);
    }
}

/// Apply the app-owned half of a non-initial live C4 save to a copied group.
///
/// `landscape_is_static` is supplied separately because an unchanged static
/// landscape and a dynamic landscape both produce no landscape component,
/// but only the static path deletes a copied legacy `Landscape.bmp`.
pub fn apply_live_save_to_group(
    group: &mut MutableGroup,
    policy: LiveC4SavePolicy<'_>,
    save: &LiveC4SaveComponents,
    save_player_infos: &[u8],
    player_groups: Vec<SerializedRuntimeJoinPlayerGroup>,
    landscape_is_static: bool,
) -> Result<()> {
    // C4GameSave mutates its open destination component-by-component. A late
    // failure deliberately leaves all earlier writes and deletions visible.
    apply_live_save_runtime_components_to_group(group, policy, save, landscape_is_static)?;
    apply_live_save_player_infos_to_group(group, save_player_infos)?;
    for player in player_groups {
        add_live_save_player_group(group, player)?;
    }
    Ok(())
}

/// Commit the exact destination prefix which precedes a fallible landscape
/// save. This preserves C4GameSave's observable partial-write behavior while
/// the engine still stops its object enumeration at the native failure point.
pub fn apply_live_save_pre_landscape_to_group(
    group: &mut MutableGroup,
    policy: LiveC4SavePolicy<'_>,
    save: &LiveC4SavePreLandscapeComponents,
) -> Result<()> {
    let mut journal = FolderSaveJournal::disabled();
    apply_live_save_pre_landscape_to_group_recorded(group, policy, save, &mut journal)
}

pub fn apply_live_save_pre_landscape_to_group_recorded(
    group: &mut MutableGroup,
    policy: LiveC4SavePolicy<'_>,
    save: &LiveC4SavePreLandscapeComponents,
    journal: &mut FolderSaveJournal,
) -> Result<()> {
    apply_pre_landscape_components(
        group,
        policy,
        &save.scenario_txt,
        save.game_txt.as_deref(),
        &save.scenario_section_mutations,
        journal,
    )?;
    apply_landscape_mutations(group, &save.landscape_mutations, journal)
}

pub fn apply_live_save_runtime_components_to_group(
    group: &mut MutableGroup,
    policy: LiveC4SavePolicy<'_>,
    save: &LiveC4SaveComponents,
    landscape_is_static: bool,
) -> Result<()> {
    let mut journal = FolderSaveJournal::disabled();
    apply_live_save_runtime_components_to_group_recorded(
        group,
        policy,
        save,
        landscape_is_static,
        &mut journal,
    )
}

pub fn apply_live_save_runtime_components_to_group_recorded(
    group: &mut MutableGroup,
    policy: LiveC4SavePolicy<'_>,
    save: &LiveC4SaveComponents,
    landscape_is_static: bool,
    journal: &mut FolderSaveJournal,
) -> Result<()> {
    apply_pre_landscape_components(
        group,
        policy,
        &save.scenario_txt,
        Some(&save.game_txt),
        &save.scenario_section_mutations,
        journal,
    )?;

    // SaveEnumeration runs only for an exact/forced landscape. The presence
    // of MatMap.txt is therefore also the delete gate for empty PXS and
    // MassMover components. Without that gate, ordinary scenario saves leave
    // all three copied entries untouched.
    let saves_auxiliary_landscape = !save.mat_map_txt.is_empty();
    let exact_landscape =
        save.landscape_bmp.is_some() || save.landscape_png.is_some() || save.delete_sky_entry;
    if exact_landscape {
        if save.delete_sky_entry {
            // C4CFN_Sky is the extensionless literal `Sky`. C4Group::Delete
            // does not expand that name to Sky.bmp/Sky.png.
            delete_pattern(group, journal, "Sky");
        }
        put_optional_file_recorded(
            group,
            journal,
            "Landscape.bmp",
            save.landscape_bmp.as_deref(),
        )?;
        put_optional_file_recorded(
            group,
            journal,
            "Landscape.png",
            save.landscape_png.as_deref(),
        )?;
        put_optional_file_recorded(group, journal, "Map.bmp", save.map_bmp.as_deref())?;
        if let Some(material_patch) = save.material_group.as_deref() {
            merge_material_patch_recorded(group, journal, material_patch)?;
        }
    } else if saves_auxiliary_landscape {
        // Forced non-exact saves write DiffLandscape, changed Map, and
        // textures before the auxiliary systems.
        put_optional_file_recorded(
            group,
            journal,
            "DiffLandscape.bmp",
            save.diff_landscape_bmp.as_deref(),
        )?;
        put_optional_file_recorded(group, journal, "Map.bmp", save.map_bmp.as_deref())?;
        if let Some(material_patch) = save.material_group.as_deref() {
            merge_material_patch_recorded(group, journal, material_patch)?;
        }
    } else if landscape_is_static {
        // The ordinary static branch starts by removing the obsolete full
        // landscape, then writes Map and any changed textures.
        delete_entry(group, journal, "Landscape.bmp");
        put_optional_file_recorded(group, journal, "Map.bmp", save.map_bmp.as_deref())?;
        if let Some(material_patch) = save.material_group.as_deref() {
            merge_material_patch_recorded(group, journal, material_patch)?;
        }
    }

    if saves_auxiliary_landscape {
        // PXS and MassMover precede the material-number enumeration.
        put_or_delete_generated_file(group, journal, "PXS.c4b", save.pxs_c4b.as_deref())?;
        put_or_delete_generated_file(
            group,
            journal,
            "MassMover.c4b",
            save.mass_mover_c4b.as_deref(),
        )?;
        put_file_recorded(
            group,
            journal,
            "MatMap.txt",
            &save.mat_map_txt,
            FolderSaveAddFailure::Fatal,
        )?;
    }
    if landscape_is_static && saves_auxiliary_landscape {
        // A forced exact save of a static landscape reaches this native
        // cleanup only after the complete exact block succeeds.
        delete_entry(group, journal, "Landscape.bmp");
    }

    // C4StringTable::Save is a no-op for an empty enumeration; it does not
    // delete a copied Strings.txt. Objects.txt is always rewritten.
    put_optional_file_recorded(group, journal, "Strings.txt", save.strings_txt.as_deref())?;
    put_component_nofail_recorded(group, journal, "Objects.txt", &save.objects_txt);

    // RoundResults::Save runs only when user restore infos are enabled. It
    // deletes a stale component before omitting an empty decompilation.
    if policy.player_policy().save_user_players {
        replace_optional_file_recorded(
            group,
            journal,
            "RoundResults.txt",
            save.round_results_txt.as_deref(),
        )?;
    }

    // Teams follow RoundResults. C4TeamList::Save deletes first, but ignores
    // the C4Group Add return just like the optional component hosts below.
    delete_entry(group, journal, "Teams.txt");
    if let Some(teams) = save.teams_txt.as_deref() {
        put_component_nofail_recorded(group, journal, "Teams.txt", teams);
    }

    // Modified Script/Title/Info hosts run atomically in this order and are
    // nonfatal. Unmodified copied components remain untouched.
    for mutation in &save.component_host_mutations {
        match mutation {
            LiveC4SaveComponentMutation::Delete { name } => {
                delete_pattern(group, journal, name);
            }
            LiveC4SaveComponentMutation::Replace(component) => {
                put_component_nofail_recorded(group, journal, &component.name, &component.payload);
            }
        }
    }

    Ok(())
}

pub fn apply_live_save_player_infos_to_group(
    group: &mut MutableGroup,
    save_player_infos: &[u8],
) -> Result<()> {
    let mut journal = FolderSaveJournal::disabled();
    apply_live_save_player_infos_to_group_recorded(group, save_player_infos, &mut journal)
}

pub fn apply_live_save_player_infos_to_group_recorded(
    group: &mut MutableGroup,
    save_player_infos: &[u8],
    journal: &mut FolderSaveJournal,
) -> Result<()> {
    // C4PlayerInfoList::Save deletes before checking emptiness or compiling.
    delete_entry(group, journal, "SavePlayerInfos.txt");
    add_live_save_player_infos_after_delete_to_group_recorded(group, save_player_infos, journal)
}

pub fn add_live_save_player_infos_after_delete_to_group_recorded(
    group: &mut MutableGroup,
    save_player_infos: &[u8],
    journal: &mut FolderSaveJournal,
) -> Result<()> {
    if !save_player_infos.is_empty() {
        // Its C4Group::Add result is ignored. Decompilation failures remain
        // fatal in the engine serializer before this mutating phase begins.
        put_component_nofail_recorded(group, journal, "SavePlayerInfos.txt", save_player_infos);
    }
    Ok(())
}

pub fn add_live_save_player_group(
    group: &mut MutableGroup,
    player: SerializedRuntimeJoinPlayerGroup,
) -> Result<()> {
    let mut journal = FolderSaveJournal::disabled();
    add_live_save_player_group_recorded(group, player, &mut journal)
}

pub fn add_live_save_player_group_recorded(
    group: &mut MutableGroup,
    player: SerializedRuntimeJoinPlayerGroup,
    journal: &mut FolderSaveJournal,
) -> Result<()> {
    if journal.enabled {
        let raw_image = player
            .group
            .pack_raw()
            .context("compose serialized player child group")?;
        journal.put_child(
            player.filename.as_bytes(),
            raw_image,
            FolderSaveAddFailure::Fatal,
        );
    }
    group
        .add_child_bytes(player.filename.as_bytes().to_vec(), player.group)
        .context("add serialized player child group")
}

fn apply_pre_landscape_components(
    group: &mut MutableGroup,
    policy: LiveC4SavePolicy<'_>,
    scenario_txt: &[u8],
    game_txt: Option<&[u8]>,
    scenario_section_mutations: &[LiveC4SaveScenarioSectionMutation],
    journal: &mut FolderSaveJournal,
) -> Result<()> {
    // SaveCore precedes every cleanup and runtime component write.
    put_file_recorded(
        group,
        journal,
        "Scenario.txt",
        scenario_txt,
        FolderSaveAddFailure::Fatal,
    )?;

    // C4GameSave::Save always removes every previously embedded player file
    // before SetAsRestoreInfos selects the new children.
    journal.delete_pattern("*.c4p");
    remove_matching_entries(group, |name| ascii_ends_with(name, ".c4p"));

    // Preserve the native cleanup set exactly. In particular, current C++
    // removes Title.bmp and Icon.bmp but not their PNG counterparts.
    if !policy.keeps_title_components() {
        journal.delete_pattern("Title.bmp");
        journal.delete_pattern("Icon.bmp");
        journal.delete_pattern("Desc*.rtf");
        journal.delete_pattern("Title*.txt|Title.txt");
        journal.delete_pattern("Info.txt");
        remove_matching_entries(group, |name| {
            name.eq_ignore_ascii_case("Title.bmp")
                || name.eq_ignore_ascii_case("Icon.bmp")
                || name.eq_ignore_ascii_case("Info.txt")
                || (ascii_starts_with(name, "Title") && ascii_ends_with(name, ".txt"))
                || (ascii_starts_with(name, "Desc") && ascii_ends_with(name, ".rtf"))
        });
    }
    let Some(game_txt) = game_txt else {
        // SaveData failed before it could replace Game.txt. SaveCore and the
        // cleanup above are nevertheless retained when the owned group closes.
        return Ok(());
    };
    // C4Game::SaveData explicitly deletes an empty Game.txt, including for a
    // non-exact scenario save with no Script or global Effects state.
    if game_txt.is_empty() {
        delete_pattern(group, journal, "Game.txt");
    } else {
        put_file_recorded(
            group,
            journal,
            "Game.txt",
            game_txt,
            FolderSaveAddFailure::Fatal,
        )?;
    }

    for mutation in scenario_section_mutations {
        match mutation {
            LiveC4SaveScenarioSectionMutation::Delete { name } => {
                delete_entry(group, journal, name);
            }
            LiveC4SaveScenarioSectionMutation::Replace(section) => {
                // SaveScenarioSections deletes before Add and ignores Add's
                // result for each section in linked-list traversal order.
                delete_entry(group, journal, &section.name);
                if let Err(error) =
                    put_raw_child_recorded(group, journal, &section.name, &section.payload)
                {
                    tracing::warn!(
                        %error,
                        section = section.name,
                        "failed to write modified live scenario section"
                    );
                }
            }
        }
    }
    Ok(())
}

fn apply_landscape_mutations(
    group: &mut MutableGroup,
    mutations: &[LiveC4SaveLandscapeMutation],
    journal: &mut FolderSaveJournal,
) -> Result<()> {
    for mutation in mutations {
        match mutation {
            LiveC4SaveLandscapeMutation::DeleteEntry { name } => {
                delete_entry(group, journal, name);
            }
            LiveC4SaveLandscapeMutation::PutFile { name, payload } => {
                put_file_recorded(group, journal, name, payload, FolderSaveAddFailure::Fatal)?;
            }
            LiveC4SaveLandscapeMutation::MergeMaterialGroup { payload } => {
                merge_material_patch_recorded(group, journal, payload)?;
            }
        }
    }
    Ok(())
}

fn delete_pattern(group: &mut MutableGroup, journal: &mut FolderSaveJournal, pattern: &str) {
    journal.delete_pattern(pattern);
    group.remove_entry(pattern);
}

fn delete_entry(group: &mut MutableGroup, journal: &mut FolderSaveJournal, name: &str) {
    journal.delete_entry(name);
    group.remove_entry(name);
}

fn put_file_recorded(
    group: &mut MutableGroup,
    journal: &mut FolderSaveJournal,
    name: &str,
    payload: &[u8],
    failure: FolderSaveAddFailure,
) -> Result<()> {
    journal.put_file(name, payload, failure);
    group
        .add_file(name, payload.to_vec())
        .with_context(|| format!("write live save component {name}"))
}

fn put_component_nofail_recorded(
    group: &mut MutableGroup,
    journal: &mut FolderSaveJournal,
    name: &str,
    payload: &[u8],
) {
    if let Err(error) =
        put_file_recorded(group, journal, name, payload, FolderSaveAddFailure::Ignore)
    {
        // Several native callers deliberately ignore the group result,
        // including Objects, Teams, player infos and component hosts.
        tracing::warn!(%error, component = name, "failed to write nonfatal live component");
    }
}

fn put_optional_file_recorded(
    group: &mut MutableGroup,
    journal: &mut FolderSaveJournal,
    name: &str,
    payload: Option<&[u8]>,
) -> Result<()> {
    if let Some(payload) = payload {
        put_file_recorded(group, journal, name, payload, FolderSaveAddFailure::Fatal)?;
    }
    Ok(())
}

fn replace_optional_file_recorded(
    group: &mut MutableGroup,
    journal: &mut FolderSaveJournal,
    name: &str,
    payload: Option<&[u8]>,
) -> Result<()> {
    delete_entry(group, journal, name);
    put_optional_file_recorded(group, journal, name, payload)
}

fn put_or_delete_generated_file(
    group: &mut MutableGroup,
    journal: &mut FolderSaveJournal,
    name: &str,
    payload: Option<&[u8]>,
) -> Result<()> {
    if let Some(payload) = payload {
        put_file_recorded(group, journal, name, payload, FolderSaveAddFailure::Fatal)
    } else {
        delete_pattern(group, journal, name);
        Ok(())
    }
}

fn put_raw_child_recorded(
    group: &mut MutableGroup,
    journal: &mut FolderSaveJournal,
    name: &str,
    payload: &[u8],
) -> Result<()> {
    // SaveScenarioSections ignores Add's return after deleting the old child.
    journal.put_child(name, payload, FolderSaveAddFailure::Ignore);
    let source = Group::from_raw_memory(PathBuf::from(name), payload.to_vec())
        .with_context(|| format!("open serialized live child {name}"))?;
    let contents_crc = source
        .contents_crc()
        .with_context(|| format!("hash serialized live child {name}"))?;
    group
        .add_packed_child_with_metadata(
            name,
            payload.to_vec(),
            contents_crc,
            unix_time_now(),
            false,
        )
        .with_context(|| format!("write serialized live child {name}"))
}

fn merge_material_patch_recorded(
    group: &mut MutableGroup,
    journal: &mut FolderSaveJournal,
    payload: &[u8],
) -> Result<()> {
    journal.merge_material_group(payload);
    let patch = Group::from_raw_memory(PathBuf::from("Material.c4g"), payload.to_vec())
        .context("open serialized Material.c4g patch")?;
    let has_material = group
        .entry_names()
        .iter()
        .any(|name| name.eq_ignore_ascii_case("Material.c4g"));
    if !has_material {
        let contents_crc = patch
            .contents_crc()
            .context("hash serialized Material.c4g patch")?;
        group
            .add_packed_child_with_metadata(
                "Material.c4g",
                payload.to_vec(),
                contents_crc,
                unix_time_now(),
                false,
            )
            .context("write new Material.c4g")?;
        return Ok(());
    }

    match group
        .child_mut("Material.c4g")
        .context("open copied Material.c4g")?
    {
        MutableGroupChildMut::Child(material) => merge_group_entries(material, &patch),
        MutableGroupChildMut::File => {
            bail!("copied Material.c4g is an ordinary file, not a child C4Group")
        }
        MutableGroupChildMut::Missing => unreachable!("Material.c4g existence was checked"),
    }
}

fn merge_group_entries(target: &mut MutableGroup, patch: &Group) -> Result<()> {
    for entry in patch.entries().context("enumerate Material.c4g patch")? {
        if entry.is_directory {
            let child = patch.open_child(&entry.relative_path).with_context(|| {
                format!(
                    "open Material.c4g patch child {}",
                    entry.relative_path.display()
                )
            })?;
            let child = MutableGroup::from_group(&child).with_context(|| {
                format!(
                    "copy Material.c4g patch child {}",
                    entry.relative_path.display()
                )
            })?;
            target
                .add_child_bytes_with_metadata(
                    entry.name_bytes,
                    child,
                    entry.time,
                    entry.executable,
                )
                .context("merge Material.c4g child entry")?;
        } else {
            let data = patch.read_entry_bytes_exact(&entry).with_context(|| {
                format!(
                    "read Material.c4g patch entry {}",
                    entry.relative_path.display()
                )
            })?;
            target
                .add_file_bytes_with_metadata(entry.name_bytes, data, entry.time, entry.executable)
                .context("merge Material.c4g file entry")?;
        }
    }
    Ok(())
}

fn remove_matching_entries(group: &mut MutableGroup, predicate: impl Fn(&str) -> bool) {
    let names = group
        .entry_names()
        .into_iter()
        .filter(|name| predicate(name))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    for name in names {
        group.remove_entry(&name);
    }
}

fn ascii_starts_with(value: &str, prefix: &str) -> bool {
    value
        .as_bytes()
        .get(..prefix.len())
        .is_some_and(|candidate| candidate.eq_ignore_ascii_case(prefix.as_bytes()))
}

fn ascii_ends_with(value: &str, suffix: &str) -> bool {
    value
        .as_bytes()
        .get(value.len().saturating_sub(suffix.len())..)
        .is_some_and(|candidate| candidate.eq_ignore_ascii_case(suffix.as_bytes()))
}

fn unix_time_now() -> u32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as u32
}

#[cfg(test)]
mod tests {
    use clonk_engine::{LegacyCString, LiveC4SaveNamedComponent, LiveC4ValueEnumeration};

    use super::*;

    fn child(name: &str, file: &str, payload: &[u8]) -> MutableGroup {
        let mut child = MutableGroup::new(name);
        child
            .add_file(file, payload.to_vec())
            .expect("fixture child file");
        child
    }

    fn raw_child(name: &str, file: &str, payload: &[u8]) -> Vec<u8> {
        child(name, file, payload)
            .pack_raw()
            .expect("fixture child packs")
    }

    fn player(name: &[u8], payload: &[u8]) -> SerializedRuntimeJoinPlayerGroup {
        SerializedRuntimeJoinPlayerGroup {
            filename: LegacyCString::from_bytes(name.to_vec()).expect("fixture player name"),
            group: child("Player.c4p", "Player.txt", payload),
        }
    }

    fn save_components() -> LiveC4SaveComponents {
        LiveC4SaveComponents {
            scenario_txt: b"new scenario".to_vec(),
            title_txt: None,
            game_txt: Vec::new(),
            objects_txt: b"new objects".to_vec(),
            strings_txt: None,
            value_enumeration: LiveC4ValueEnumeration::default(),
            landscape_bmp: None,
            landscape_png: None,
            diff_landscape_bmp: None,
            map_bmp: None,
            material_group: Some(raw_child("Material.c4g", "TexMap.txt", b"new texmap")),
            mat_map_txt: b"new matmap".to_vec(),
            pxs_c4b: None,
            mass_mover_c4b: None,
            delete_sky_entry: false,
            teams_txt: Some(Vec::new()),
            round_results_txt: None,
            info_txt: None,
            script_c: None,
            deleted_components: Vec::new(),
            component_host_mutations: Vec::new(),
            scenario_sections: vec![LiveC4SaveNamedComponent {
                name: "SectOther.c4g".to_owned(),
                payload: raw_child("SectOther.c4g", "Objects.txt", b"other section"),
            }],
            deleted_scenario_sections: Vec::new(),
            scenario_section_mutations: vec![
                LiveC4SaveScenarioSectionMutation::Delete {
                    name: "SectMain.c4g".to_owned(),
                },
                LiveC4SaveScenarioSectionMutation::Replace(LiveC4SaveNamedComponent {
                    name: "SectOther.c4g".to_owned(),
                    payload: raw_child("SectOther.c4g", "Objects.txt", b"other section"),
                }),
            ],
        }
    }

    fn reopen(group: &MutableGroup) -> Group {
        Group::from_raw_memory(
            PathBuf::from("Saved.c4s"),
            group.pack_raw().expect("saved group packs"),
        )
        .expect("saved group reopens")
    }

    #[test]
    fn savegame_description_matches_native_rtf_envelope_and_escaping() {
        let description = serialize_savegame_description(
            br"A {saved}\game",
            204,
            &[(b"Game saved 20.7.2026 01:02.".to_vec(), true)],
        );

        assert_eq!(
            description,
            b"{\\rtf1\\ansi\\ansicpg1252\\deff0\\deflang1031{\\fonttbl {\\f0\\fnil\\fcharset204 Times New Roman;}}\r\n\\uc1\\pard\\ulnone\\b\\f0\\fs20 A \\{saved\\}\\\\game\\par\r\n\\b0\\fs16\\par\r\nGame saved 20.7.2026 01:02.\\par\r\n\r\n}\r\n "
        );
    }

    #[test]
    fn description_integer_formatter_supports_classic_widths_and_percent() {
        assert_eq!(
            format_resource_integers(b"%i.%i.%i %02d:%02d %%", &[20, 7, 2026, 1, 2]),
            b"20.7.2026 01:02 %"
        );
        assert_eq!(
            format_resource_integers(b"Playing time: %02d:%02d:%02d.", &[3, 4, 5]),
            b"Playing time: 03:04:05."
        );
        assert_eq!(
            format_resource_strings(b"Engine %s: %% %s", &[b"362", b"League"]),
            b"Engine 362: % League"
        );
    }

    #[test]
    fn exact_save_applies_native_deletes_and_merges_material_patch() {
        let mut group = MutableGroup::new("Saved.c4s");
        for (name, payload) in [
            ("Game.txt", b"old game".as_slice()),
            ("Title.bmp", b"old title bitmap"),
            ("Title.png", b"preserved title png"),
            ("Icon.bmp", b"old icon bitmap"),
            ("Icon.png", b"preserved icon png"),
            ("TitleUS.txt", b"old title text"),
            ("DescUS.rtf", b"old description"),
            ("Info.txt", b"old info"),
            ("Script.c", b"preserved script"),
            ("Strings.txt", b"preserved strings"),
            ("Teams.txt", b"old teams"),
            ("RoundResults.txt", b"old results"),
            ("Landscape.bmp", b"old landscape"),
            ("Map.bmp", b"preserved map"),
            ("PXS.c4b", b"old pxs"),
            ("MassMover.c4b", b"old movers"),
            ("PlayerInfos.txt", b"preserved initial infos"),
            ("SavePlayerInfos.txt", b"old restore infos"),
        ] {
            group
                .add_file(name, payload.to_vec())
                .expect("fixture root file");
        }
        let mut material = child("Material.c4g", "Earth.c4m", b"earth material");
        material
            .add_file("TexMap.txt", b"old texmap".to_vec())
            .expect("old texmap");
        group
            .add_child("Material.c4g", material)
            .expect("fixture material child");
        group
            .add_child("Old.c4p", child("Old.c4p", "Player.txt", b"old player"))
            .expect("fixture old player");
        group
            .add_child(
                "SectMain.c4g",
                child("SectMain.c4g", "Objects.txt", b"current section"),
            )
            .expect("fixture current section");
        group
            .add_child(
                "SectKeep.c4g",
                child("SectKeep.c4g", "Objects.txt", b"kept section"),
            )
            .expect("fixture kept section");

        apply_live_save_to_group(
            &mut group,
            LiveC4SavePolicy::Savegame {
                target_group_name: "Saved.c4s",
            },
            &save_components(),
            b"new restore infos",
            vec![player(b"New.c4p", b"new player")],
            true,
        )
        .expect("exact save mutates copied group");

        let saved = reopen(&group);
        for removed in [
            "Game.txt",
            "Title.bmp",
            "Icon.bmp",
            "TitleUS.txt",
            "DescUS.rtf",
            "Info.txt",
            "RoundResults.txt",
            "Landscape.bmp",
            "PXS.c4b",
            "MassMover.c4b",
            "Old.c4p",
            "SectMain.c4g",
        ] {
            assert!(!saved.exists(removed), "{removed} should be deleted");
        }
        for (name, expected) in [
            ("Title.png", b"preserved title png".as_slice()),
            ("Icon.png", b"preserved icon png"),
            ("Script.c", b"preserved script"),
            ("Strings.txt", b"preserved strings"),
            ("Map.bmp", b"preserved map"),
            ("PlayerInfos.txt", b"preserved initial infos"),
            ("Scenario.txt", b"new scenario"),
            ("Objects.txt", b"new objects"),
            ("MatMap.txt", b"new matmap"),
            ("SavePlayerInfos.txt", b"new restore infos"),
        ] {
            assert_eq!(saved.read_file(name).expect(name), expected);
        }
        assert!(saved.exists("Teams.txt"));
        assert!(saved.read_file("Teams.txt").unwrap().is_empty());
        assert!(saved.open_child("New.c4p").is_ok());
        assert!(saved.open_child("SectKeep.c4g").is_ok());
        assert!(saved.open_child("SectOther.c4g").is_ok());
        let material = saved.open_child("Material.c4g").expect("material child");
        assert_eq!(material.read_file("Earth.c4m").unwrap(), b"earth material");
        assert_eq!(material.read_file("TexMap.txt").unwrap(), b"new texmap");
    }

    #[test]
    fn scenario_save_preserves_nonexact_components_and_current_section() {
        let mut group = MutableGroup::new("Scenario.c4s");
        for (name, payload) in [
            ("Title.bmp", b"title bitmap".as_slice()),
            ("TitleUS.txt", b"old title"),
            ("Info.txt", b"old info"),
            ("RoundResults.txt", b"old results"),
            ("PXS.c4b", b"old pxs"),
            ("MassMover.c4b", b"old movers"),
            ("MatMap.txt", b"old matmap"),
            ("Landscape.bmp", b"legacy static landscape"),
        ] {
            group
                .add_file(name, payload.to_vec())
                .expect("fixture root file");
        }
        group
            .add_child(
                "SectMain.c4g",
                child("SectMain.c4g", "Objects.txt", b"current section"),
            )
            .expect("fixture current section");
        group
            .add_child("Old.c4p", child("Old.c4p", "Player.txt", b"old player"))
            .expect("fixture old player");

        let mut save = save_components();
        let title = LiveC4SaveNamedComponent {
            name: "TitleUS.txt".to_owned(),
            payload: b"new title".to_vec(),
        };
        save.title_txt = Some(title.clone());
        save.component_host_mutations = vec![LiveC4SaveComponentMutation::Replace(title)];
        save.material_group = None;
        save.mat_map_txt.clear();
        save.scenario_sections.clear();
        save.scenario_section_mutations.clear();

        apply_live_save_to_group(
            &mut group,
            LiveC4SavePolicy::Scenario {
                force_exact_landscape: false,
            },
            &save,
            b"script restore infos",
            vec![player(b"ScriptPlr-1.c4p", b"script player")],
            true,
        )
        .expect("scenario save mutates copied group");

        let saved = reopen(&group);
        assert_eq!(saved.read_file("Title.bmp").unwrap(), b"title bitmap");
        assert_eq!(saved.read_file("TitleUS.txt").unwrap(), b"new title");
        assert_eq!(saved.read_file("Info.txt").unwrap(), b"old info");
        assert_eq!(saved.read_file("RoundResults.txt").unwrap(), b"old results");
        assert_eq!(saved.read_file("PXS.c4b").unwrap(), b"old pxs");
        assert_eq!(saved.read_file("MassMover.c4b").unwrap(), b"old movers");
        assert_eq!(saved.read_file("MatMap.txt").unwrap(), b"old matmap");
        assert!(!saved.exists("Landscape.bmp"));
        assert!(!saved.exists("Old.c4p"));
        assert!(saved.open_child("ScriptPlr-1.c4p").is_ok());
        assert!(saved.open_child("SectMain.c4g").is_ok());
    }

    #[test]
    fn exact_save_preserves_stale_section_child_without_serialized_delete_mutation() {
        let mut group = MutableGroup::new("Saved.c4s");
        group
            .add_child(
                "SectMain.c4g",
                child("SectMain.c4g", "Objects.txt", b"stale child"),
            )
            .expect("fixture stale section child");
        let mut save = save_components();
        save.scenario_section_mutations.retain(|mutation| {
            !matches!(
                mutation,
                LiveC4SaveScenarioSectionMutation::Delete { name }
                    if name.eq_ignore_ascii_case("SectMain.c4g")
            )
        });

        apply_live_save_to_group(
            &mut group,
            LiveC4SavePolicy::Savegame {
                target_group_name: "Saved.c4s",
            },
            &save,
            b"restore infos",
            Vec::new(),
            false,
        )
        .expect("exact save without section registry");

        assert!(reopen(&group).open_child("SectMain.c4g").is_ok());
    }

    #[test]
    fn empty_restore_roster_deletes_copied_save_player_infos() {
        let mut group = MutableGroup::new("Saved.c4s");
        group
            .add_file("SavePlayerInfos.txt", b"stale roster".to_vec())
            .expect("fixture restore roster");

        apply_live_save_to_group(
            &mut group,
            LiveC4SavePolicy::Savegame {
                target_group_name: "Saved.c4s",
            },
            &save_components(),
            &[],
            Vec::new(),
            false,
        )
        .expect("empty restore roster is applied");

        assert!(!reopen(&group).exists("SavePlayerInfos.txt"));
    }

    #[test]
    fn invalid_modified_scenario_section_is_deleted_and_nonfatal() {
        let mut group = MutableGroup::new("Saved.c4s");
        group
            .add_child(
                "SectBroken.c4g",
                child("SectBroken.c4g", "Objects.txt", b"stale section"),
            )
            .expect("fixture stale section child");
        let mut save = save_components();
        let broken = LiveC4SaveNamedComponent {
            name: "SectBroken.c4g".to_owned(),
            payload: b"not a packed group".to_vec(),
        };
        save.scenario_sections = vec![broken.clone()];
        save.scenario_section_mutations = vec![LiveC4SaveScenarioSectionMutation::Replace(broken)];

        apply_live_save_to_group(
            &mut group,
            LiveC4SavePolicy::Savegame {
                target_group_name: "Saved.c4s",
            },
            &save,
            b"restore infos",
            Vec::new(),
            false,
        )
        .expect("unchecked native section Add remains nonfatal");

        let saved = reopen(&group);
        assert!(!saved.exists("SectBroken.c4g"));
        assert_eq!(saved.read_file("Objects.txt").unwrap(), b"new objects");
    }

    #[test]
    fn no_sky_exact_save_deletes_only_the_extensionless_entry() {
        let mut group = MutableGroup::new("Saved.c4s");
        group.add_file("Sky", b"legacy sky".to_vec()).unwrap();
        group.add_file("Sky.png", b"image sky".to_vec()).unwrap();
        let mut save = save_components();
        save.delete_sky_entry = true;

        apply_live_save_to_group(
            &mut group,
            LiveC4SavePolicy::Savegame {
                target_group_name: "Saved.c4s",
            },
            &save,
            b"restore infos",
            Vec::new(),
            false,
        )
        .unwrap();

        let saved = reopen(&group);
        assert!(!saved.exists("Sky"));
        assert_eq!(saved.read_file("Sky.png").unwrap(), b"image sky");
    }

    #[test]
    fn local_player_overlay_is_recursive_preserves_extras_and_uses_cpp_sort() {
        let mut original = MutableGroup::new("Profile.c4p");
        original
            .add_file("Extras.dat", b"root extra".to_vec())
            .expect("original root extra");
        original
            .add_file("Portrait.png", b"profile portrait".to_vec())
            .expect("original profile portrait");
        original
            .add_file("Player.txt", b"old player core".to_vec())
            .expect("original player core");
        original
            .add_file("C4Player.c4b", b"obsolete binary core".to_vec())
            .expect("original legacy player core");
        let mut original_custom = child("Custom.c4g", "Shared.txt", b"old shared");
        original_custom
            .add_file("Keep.dat", b"nested extra".to_vec())
            .expect("original nested extra");
        let mut original_crew = child("Crew.c4i", "ObjectInfo.txt", b"old crew core");
        original_crew
            .add_file("Notes.txt", b"crew extra".to_vec())
            .expect("original crew extra");
        original_crew
            .add_child("Custom.c4g", original_custom)
            .expect("original nested group");
        original
            .add_child("Crew.c4i", original_crew)
            .expect("original crew");
        original
            .add_child(
                "Retired.c4i",
                child("Retired.c4i", "ObjectInfo.txt", b"unrelated crew"),
            )
            .expect("unrelated original crew");
        let original = Group::from_raw_memory(
            PathBuf::from("Profile.c4p"),
            original.pack_raw().expect("original profile packs"),
        )
        .expect("original profile opens");

        let mut live = MutableGroup::new("Profile.c4p");
        let mut live_custom = child("Custom.c4g", "Shared.txt", b"new shared");
        live_custom
            .add_file("Added.dat", b"new nested file".to_vec())
            .expect("new nested file");
        let mut live_crew = child("Crew.c4i", "ObjectInfo.txt", b"new crew core");
        live_crew
            .add_file("Rank.png", b"new rank".to_vec())
            .expect("new rank image");
        live_crew
            .add_child("Custom.c4g", live_custom)
            .expect("live nested group");
        live.add_child("Crew.c4i", live_crew)
            .expect("live existing crew");
        live.add_child("New.c4i", child("New.c4i", "ObjectInfo.txt", b"new crew"))
            .expect("live new crew");
        live.add_file("Player.txt", b"new player core".to_vec())
            .expect("live player core");

        let merged = overlay_live_player_group(&original, &live).expect("overlay local profile");
        assert_eq!(
            merged.entry_names(),
            [
                "Player.txt",
                "Portrait.png",
                "Crew.c4i",
                "New.c4i",
                "Retired.c4i",
                "Extras.dat",
            ]
        );

        // The transactional helper must not mutate the source profile.
        assert_eq!(
            original.read_file("Player.txt").unwrap(),
            b"old player core"
        );
        let merged = Group::from_raw_memory(
            PathBuf::from("Profile.c4p"),
            merged.pack_raw().expect("merged profile packs"),
        )
        .expect("merged profile opens");
        assert_eq!(merged.read_file("Player.txt").unwrap(), b"new player core");
        assert_eq!(
            merged.read_file("Portrait.png").unwrap(),
            b"profile portrait"
        );
        assert_eq!(merged.read_file("Extras.dat").unwrap(), b"root extra");
        assert!(!merged.exists("C4Player.c4b"));
        assert!(merged.open_child("Retired.c4i").is_ok());
        assert!(merged.open_child("New.c4i").is_ok());
        let crew = merged.open_child("Crew.c4i").expect("merged existing crew");
        assert_eq!(crew.read_file("ObjectInfo.txt").unwrap(), b"new crew core");
        assert_eq!(crew.read_file("Notes.txt").unwrap(), b"crew extra");
        assert_eq!(crew.read_file("Rank.png").unwrap(), b"new rank");
        let custom = crew.open_child("Custom.c4g").expect("merged nested group");
        assert_eq!(custom.read_file("Shared.txt").unwrap(), b"new shared");
        assert_eq!(custom.read_file("Keep.dat").unwrap(), b"nested extra");
        assert_eq!(custom.read_file("Added.dat").unwrap(), b"new nested file");
    }

    #[test]
    fn local_player_overlay_applies_native_omitted_asset_deletions() {
        let mut original = MutableGroup::new("Profile.c4p");
        let mut crew = child("Crew.c4i", "ObjectInfo.txt", b"old core");
        for (name, payload) in [
            ("Portrait.png", b"old portrait".as_slice()),
            ("PortraitOverlay.png", b"old overlay".as_slice()),
            ("Portrait.bmp", b"legacy portrait".as_slice()),
            ("Rank.png", b"old rank".as_slice()),
            ("Keep.dat", b"custom data".as_slice()),
        ] {
            crew.add_file(name, payload.to_vec()).expect("crew asset");
        }
        original.add_child("Crew.c4i", crew).expect("original crew");

        let mut lone_overlay = child("Lone.c4i", "ObjectInfo.txt", b"old lone core");
        lone_overlay
            .add_file("PortraitOverlay.png", b"lone overlay".to_vec())
            .expect("lone overlay");
        original
            .add_child("Lone.c4i", lone_overlay)
            .expect("lone-overlay crew");
        let original = Group::from_raw_memory(
            PathBuf::from("Profile.c4p"),
            original.pack_raw().expect("original profile packs"),
        )
        .expect("original profile opens");

        let mut live = MutableGroup::new("Profile.c4p");
        live.add_file("Player.txt", b"live player".to_vec())
            .expect("live player core");
        live.add_child(
            "Crew.c4i",
            child("Crew.c4i", "ObjectInfo.txt", b"live core"),
        )
        .expect("live crew");
        live.add_child(
            "Lone.c4i",
            child("Lone.c4i", "ObjectInfo.txt", b"live lone core"),
        )
        .expect("live lone-overlay crew");

        let cleanup = [
            LiveC4CrewProfileCleanup {
                filename: b"Crew.c4i".to_vec(),
                original_filename: b"Crew.c4i".to_vec(),
                roster_index: 0,
                remove_default_portrait_png: true,
                remove_rank_png: true,
            },
            LiveC4CrewProfileCleanup {
                filename: b"Lone.c4i".to_vec(),
                original_filename: b"Lone.c4i".to_vec(),
                roster_index: 1,
                remove_default_portrait_png: true,
                remove_rank_png: false,
            },
        ];
        let merged = overlay_live_player_group_with_cleanup(&original, &live, &cleanup)
            .expect("overlay local profile with cleanup");
        let merged = Group::from_raw_memory(
            PathBuf::from("Profile.c4p"),
            merged.pack_raw().expect("merged profile packs"),
        )
        .expect("merged profile opens");
        let crew = merged.open_child("Crew.c4i").expect("merged crew");
        assert!(!crew.exists("Portrait.png"));
        assert!(!crew.exists("PortraitOverlay.png"));
        assert!(!crew.exists("Rank.png"));
        assert_eq!(crew.read_file("Portrait.bmp").unwrap(), b"legacy portrait");
        assert_eq!(crew.read_file("Keep.dat").unwrap(), b"custom data");
        assert_eq!(crew.read_file("ObjectInfo.txt").unwrap(), b"live core");
        assert_eq!(
            merged
                .open_child("Lone.c4i")
                .unwrap()
                .read_file("PortraitOverlay.png")
                .unwrap(),
            b"lone overlay",
            "C++ gates overlay deletion on an existing Portrait.png"
        );
    }

    #[test]
    fn local_player_overlay_moves_profile_extras_with_renamed_crew() {
        let mut original = MutableGroup::new("Profile.c4p");
        let mut old_crew = child("Old Hero.c4i", "ObjectInfo.txt", b"old core");
        old_crew
            .add_file("Keep.dat", b"profile-only data".to_vec())
            .expect("profile-only crew data");
        original
            .add_child("Old Hero.c4i", old_crew)
            .expect("old crew");
        let original = Group::from_raw_memory(
            PathBuf::from("Profile.c4p"),
            original.pack_raw().expect("original profile packs"),
        )
        .expect("original profile opens");

        let mut live = MutableGroup::new("Profile.c4p");
        live.add_file("Player.txt", b"live player".to_vec())
            .expect("live player core");
        live.add_child(
            "Renamed Hero.c4i",
            child("Renamed Hero.c4i", "ObjectInfo.txt", b"live core"),
        )
        .expect("renamed live crew");
        let cleanup = [LiveC4CrewProfileCleanup {
            filename: b"Renamed Hero.c4i".to_vec(),
            original_filename: b"Old Hero.c4i".to_vec(),
            roster_index: 0,
            remove_default_portrait_png: false,
            remove_rank_png: false,
        }];

        let merged = overlay_live_player_group_with_cleanup(&original, &live, &cleanup)
            .expect("overlay renamed local crew");
        let merged = Group::from_raw_memory(
            PathBuf::from("Profile.c4p"),
            merged.pack_raw().expect("merged profile packs"),
        )
        .expect("merged profile opens");
        assert!(!merged.exists("Old Hero.c4i"));
        let crew = merged
            .open_child("Renamed Hero.c4i")
            .expect("renamed crew opens");
        assert_eq!(crew.read_file("ObjectInfo.txt").unwrap(), b"live core");
        assert_eq!(crew.read_file("Keep.dat").unwrap(), b"profile-only data");
    }

    #[test]
    fn local_player_overlay_replaces_conflicting_entry_kinds() {
        let mut original = MutableGroup::new("Profile.c4p");
        original
            .add_file("Crew.c4i", b"not a group".to_vec())
            .expect("original crew-shaped file");
        original
            .add_child("Player.txt", child("Player.txt", "Old.dat", b"old child"))
            .expect("original player-shaped child");
        original
            .add_file("Keep.dat", b"preserved".to_vec())
            .expect("original sibling");
        let original = Group::from_raw_memory(
            PathBuf::from("Profile.c4p"),
            original.pack_raw().expect("original profile packs"),
        )
        .expect("original profile opens");

        let mut live = MutableGroup::new("Profile.c4p");
        live.add_file("Player.txt", b"live core".to_vec())
            .expect("live player core");
        live.add_child(
            "Crew.c4i",
            child("Crew.c4i", "ObjectInfo.txt", b"live crew"),
        )
        .expect("live crew group");

        let merged = overlay_live_player_group(&original, &live).expect("overlay local profile");
        let merged = Group::from_raw_memory(
            PathBuf::from("Profile.c4p"),
            merged.pack_raw().expect("merged profile packs"),
        )
        .expect("merged profile opens");
        assert_eq!(merged.read_file("Player.txt").unwrap(), b"live core");
        assert_eq!(merged.read_file("Keep.dat").unwrap(), b"preserved");
        assert_eq!(
            merged
                .open_child("Crew.c4i")
                .unwrap()
                .read_file("ObjectInfo.txt")
                .unwrap(),
            b"live crew"
        );
    }

    #[test]
    fn material_file_error_retains_native_prefix_mutations() {
        let mut group = MutableGroup::new("Scenario.c4s");
        group
            .add_file("Material.c4g", b"not a group".to_vec())
            .expect("fixture material file");
        let error = apply_live_save_to_group(
            &mut group,
            LiveC4SavePolicy::Savegame {
                target_group_name: "Scenario.c4s",
            },
            &save_components(),
            b"restore infos",
            Vec::new(),
            false,
        )
        .expect_err("ordinary Material.c4g file must reject the patch");

        assert!(error.to_string().contains("ordinary file"));
        let group = reopen(&group);
        assert_eq!(group.read_file("Scenario.txt").unwrap(), b"new scenario");
        assert!(!group.exists("Game.txt"));
        assert!(group.exists("SectOther.c4g"));
        assert_eq!(group.read_file("Material.c4g").unwrap(), b"not a group");
        assert!(!group.exists("MatMap.txt"));
        assert!(!group.exists("Objects.txt"));
        assert!(!group.exists("Teams.txt"));
    }

    #[test]
    fn failed_static_texture_save_replays_landscape_prefix_in_native_order() {
        let mut group = MutableGroup::new("Scenario.c4s");
        group
            .add_file("Landscape.bmp", b"stale exact landscape".to_vec())
            .unwrap();
        group.add_file("Map.bmp", b"stale map".to_vec()).unwrap();
        group
            .add_file("Material.c4g", b"not a group".to_vec())
            .unwrap();
        let partial = LiveC4SavePreLandscapeComponents {
            scenario_txt: b"new scenario".to_vec(),
            game_txt: Some(Vec::new()),
            scenario_sections: Vec::new(),
            landscape_mutations: vec![
                LiveC4SaveLandscapeMutation::DeleteEntry {
                    name: "Landscape.bmp".to_owned(),
                },
                LiveC4SaveLandscapeMutation::PutFile {
                    name: "Map.bmp".to_owned(),
                    payload: b"new map".to_vec(),
                },
            ],
            deleted_scenario_sections: Vec::new(),
            scenario_section_mutations: Vec::new(),
        };

        apply_live_save_pre_landscape_to_group(
            &mut group,
            LiveC4SavePolicy::Scenario {
                force_exact_landscape: false,
            },
            &partial,
        )
        .expect("native failure prefix applies");

        let group = reopen(&group);
        assert!(!group.exists("Landscape.bmp"));
        assert_eq!(group.read_file("Map.bmp").unwrap(), b"new map");
        assert_eq!(group.read_file("Material.c4g").unwrap(), b"not a group");
        assert!(!group.exists("Objects.txt"));
    }

    #[test]
    fn folder_journal_keeps_native_scenario_section_interleaving() {
        let mut save = save_components();
        save.material_group = None;
        save.mat_map_txt.clear();
        save.scenario_section_mutations = vec![
            LiveC4SaveScenarioSectionMutation::Replace(LiveC4SaveNamedComponent {
                name: "SectZulu.c4g".to_owned(),
                payload: raw_child("SectZulu.c4g", "Objects.txt", b"zulu"),
            }),
            LiveC4SaveScenarioSectionMutation::Delete {
                name: "SectMain.c4g".to_owned(),
            },
            LiveC4SaveScenarioSectionMutation::Replace(LiveC4SaveNamedComponent {
                name: "SectAlpha.c4g".to_owned(),
                payload: raw_child("SectAlpha.c4g", "Objects.txt", b"alpha"),
            }),
        ];
        let mut group = MutableGroup::new("Scenario.c4s");
        let mut journal = FolderSaveJournal::default();

        apply_live_save_runtime_components_to_group_recorded(
            &mut group,
            LiveC4SavePolicy::Record,
            &save,
            false,
            &mut journal,
        )
        .expect("record ordered section mutations");

        let section_actions = journal
            .mutations()
            .iter()
            .filter_map(|mutation| match mutation {
                FolderSaveMutation::DeleteEntry { name } if name.starts_with(b"Sect") => {
                    Some(format!("delete:{}", String::from_utf8_lossy(name)))
                }
                FolderSaveMutation::PutChild { name, .. } if name.starts_with(b"Sect") => {
                    Some(format!("put:{}", String::from_utf8_lossy(name)))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            section_actions,
            [
                "delete:SectZulu.c4g",
                "put:SectZulu.c4g",
                "delete:SectMain.c4g",
                "delete:SectAlpha.c4g",
                "put:SectAlpha.c4g",
            ]
        );
        let objects_index = journal
            .mutations()
            .iter()
            .position(|mutation| {
                matches!(
                    mutation,
                    FolderSaveMutation::PutFile {
                        name,
                        failure: FolderSaveAddFailure::Ignore,
                        ..
                    } if name.as_slice() == b"Objects.txt"
                )
            })
            .expect("Objects.txt add is nonfatal");
        let teams_index = journal
            .mutations()
            .iter()
            .position(|mutation| {
                matches!(
                    mutation,
                    FolderSaveMutation::DeleteEntry { name }
                        if name.as_slice() == b"Teams.txt"
                )
            })
            .expect("later Teams mutation remains queued");
        assert!(objects_index < teams_index);
    }
}
