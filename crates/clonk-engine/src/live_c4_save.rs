//! Typed C4 components captured at the synchronized runtime-join boundary.
//!
//! This is deliberately a component serializer, not a group writer. The
//! application owns `SavePlayerInfos`, embedded small player groups and final
//! C4Group metadata because those values are coupled to its player-info and
//! network-resource registries.

use std::collections::{HashMap, HashSet};
use std::io::Cursor;
use std::path::PathBuf;

use clonk_resources::bitmap::{BitmapError, IndexedBitmap};
use clonk_resources::{Group, GroupError, MutableGroup, MutableGroupEntryKind, MutableGroupError};
use clonk_script::{C4StringValue, Value};
use image::{DynamicImage, ImageOutputFormat, Rgba, RgbaImage};
use thiserror::Error;

use crate::command::{CommandData, LegacyCommandSave};
use crate::effect::{EffectState, EffectVarValue};
use crate::network_game_data::{
    serialize_initial_network_game, InitialNetworkCompiledSections, InitialNetworkGameData,
    InitialNetworkGameError,
};
use crate::player::PlayerState;
use crate::round_results::{
    RoundResultsNetworkResult, RoundResultsPlayerStatus, RoundResultsState,
};
use crate::scenario::ScenarioValueStore;
use crate::sky::{SkyFrame, SkyParallaxMode};
use crate::{
    Engine, Object, ObjectId, ObjectStatus, PhysicalInfo, ScoreboardState, TeamConfiguration,
    TeamInfo, LANDSCAPE_MODE_EXACT, LANDSCAPE_MODE_STATIC,
};

/// The concrete `C4GameSave` specialization whose component policy is being
/// applied.  The existing [`Engine::serialize_live_c4_save`] entry point is a
/// compatibility shorthand for [`LiveC4SavePolicy::RuntimeNetwork`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveC4SavePolicy<'a> {
    /// `C4GameSaveScenario`: keep authoring metadata, omit user-player state,
    /// and force a diff landscape only when the console requests it.
    Scenario { force_exact_landscape: bool },
    /// `C4GameSaveSavegame`: an exact resumable game. The destination name is
    /// needed for the native trailing-slot icon adjustment.
    Savegame { target_group_name: &'a str },
    /// Non-initial `C4GameSaveRecord`. `LiveC4SaveSpec::title` is the already
    /// formatted `NNN <scenario title> [<build>]` record title.
    Record,
    /// Non-initial `C4GameSaveNetwork`, used by runtime join.
    RuntimeNetwork,
}

/// The four independent `SetAsRestoreInfos` switches selected by a save
/// specialization.  The app owns the actual player-info list and child-group
/// writes, but must use this policy to avoid embedding user files in ordinary
/// savegames or retaining user players in saved scenarios.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveC4SavePlayerPolicy {
    pub save_user_players: bool,
    pub save_script_players: bool,
    pub embed_user_player_files: bool,
    pub embed_script_player_files: bool,
}

impl LiveC4SavePolicy<'_> {
    pub const fn is_exact(self) -> bool {
        matches!(
            self,
            Self::Savegame { .. } | Self::Record | Self::RuntimeNetwork
        )
    }

    pub const fn is_synchronized(self) -> bool {
        matches!(self, Self::Record | Self::RuntimeNetwork)
    }

    pub const fn keeps_title_components(self) -> bool {
        matches!(self, Self::Scenario { .. })
    }

    pub const fn copies_source_scenario(self) -> bool {
        !matches!(self, Self::RuntimeNetwork)
    }

    pub const fn saves_description(self) -> bool {
        matches!(self, Self::Savegame { .. })
    }

    pub const fn saves_game_title_image(self) -> bool {
        matches!(self, Self::Savegame { .. })
    }

    pub const fn creates_small_player_files(self) -> bool {
        matches!(self, Self::Record | Self::RuntimeNetwork)
    }

    pub const fn player_policy(self) -> LiveC4SavePlayerPolicy {
        match self {
            Self::Scenario { .. } => LiveC4SavePlayerPolicy {
                save_user_players: false,
                save_script_players: true,
                embed_user_player_files: false,
                embed_script_player_files: true,
            },
            Self::Savegame { .. } => LiveC4SavePlayerPolicy {
                save_user_players: true,
                save_script_players: true,
                embed_user_player_files: false,
                embed_script_player_files: true,
            },
            Self::Record | Self::RuntimeNetwork => LiveC4SavePlayerPolicy {
                save_user_players: true,
                save_script_players: true,
                embed_user_player_files: true,
                embed_script_player_files: true,
            },
        }
    }

    const fn forces_runtime_landscape(self) -> bool {
        match self {
            Self::Scenario {
                force_exact_landscape,
            } => force_exact_landscape,
            Self::Savegame { .. } | Self::Record | Self::RuntimeNetwork => true,
        }
    }

    const fn landscape_diff_is_sync_save(self) -> bool {
        !self.is_synchronized()
    }
}

/// Application-owned inputs that are not synchronized engine state.
#[derive(Debug, Clone, Copy)]
pub struct LiveC4SaveSpec<'a> {
    pub title: &'a str,
    pub definition_modules: &'a [String],
    /// Native `Config.General.ExePath`, including its trailing separator.
    pub definition_executable_path: &'a str,
    /// Native `Config.General.DefinitionPath` (relative or absolute).
    pub definition_path: &'a str,
    pub origin: &'a str,
    /// `Application.MusicSystem::IsMusicEnabled`; the playlist and level are
    /// engine state, while this process-local switch is not.
    pub music_enabled: bool,
    /// The copied destination contains an ordinary `Material.c4g` file. The
    /// serializer must expose this so a dirty texture save fails at native
    /// landscape order, before object enumeration begins.
    pub copied_material_group_is_file: bool,
    /// The three C4ComponentHost mutations. A modified host with null data is
    /// a deletion, which cannot be represented by an `Option<payload>`.
    pub title_component: LiveC4ComponentHost<'a>,
    pub info_component: LiveC4ComponentHost<'a>,
    pub script_component: LiveC4ComponentHost<'a>,
}

/// Borrowed modified component supplied by the application component host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveC4SaveComponentRef<'a> {
    pub name: &'a str,
    pub payload: &'a [u8],
}

/// Save projection of a retained C4ComponentHost filename/data pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LiveC4ComponentHost<'a> {
    #[default]
    Unmodified,
    Replace(LiveC4SaveComponentRef<'a>),
    Delete {
        name: &'a str,
    },
}

/// Owned file or raw child-group payload with its final component name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveC4SaveNamedComponent {
    pub name: String,
    pub payload: Vec<u8>,
}

/// Whether an ordered component is a root file or a raw nested C4Group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveC4SaveEntryKind {
    File,
    ChildGroup,
}

/// Borrowed ordered view used by the network dynamic composer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveC4SaveEntry<'a> {
    pub name: &'a str,
    pub payload: &'a [u8],
    pub kind: LiveC4SaveEntryKind,
}

/// Components represented by synchronized `clonk-engine` state.
///
/// `None` has C4GameSave's delete/omit meaning. `material_group`, when
/// present, is the raw uncompressed image stored for the `Material.c4g`
/// child entry, not a standalone gzip-wrapped group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveC4SaveComponents {
    pub scenario_txt: Vec<u8>,
    /// Always `None` for exact runtime-network saves.
    #[doc(hidden)]
    pub title_txt: Option<LiveC4SaveNamedComponent>,
    pub game_txt: Vec<u8>,
    pub objects_txt: Vec<u8>,
    pub strings_txt: Option<Vec<u8>>,
    /// Final C4StringTable enumeration shared with subsequently serialized
    /// embedded player/crew ExtraData. C++ enumerates once before saving the
    /// scenario and then reuses those IDs in C4PlayerList::Save.
    pub value_enumeration: LiveC4ValueEnumeration,
    pub landscape_bmp: Option<Vec<u8>>,
    pub landscape_png: Option<Vec<u8>>,
    pub diff_landscape_bmp: Option<Vec<u8>>,
    pub map_bmp: Option<Vec<u8>>,
    pub material_group: Option<Vec<u8>>,
    pub mat_map_txt: Vec<u8>,
    pub pxs_c4b: Option<Vec<u8>>,
    pub mass_mover_c4b: Option<Vec<u8>>,
    /// `C4Sky::Save` deletes the extensionless `C4CFN_Sky` entry when an
    /// exact landscape is saved for a `NoSky` scenario. The app owns the
    /// copied destination group, so it applies that deletion there.
    pub delete_sky_entry: bool,
    pub teams_txt: Option<Vec<u8>>,
    pub round_results_txt: Option<Vec<u8>>,
    pub info_txt: Option<LiveC4SaveNamedComponent>,
    pub script_c: Option<LiveC4SaveNamedComponent>,
    /// Actual retained filenames removed by modified hosts with null data.
    pub deleted_components: Vec<String>,
    /// Script, localized Title (non-exact only), and Info host mutations in
    /// native call order. This preserves mixed delete/replace failure order.
    pub component_host_mutations: Vec<LiveC4SaveComponentMutation>,
    /// Modified non-current `Sect*.c4g` images in native section-list order.
    /// Each payload is a raw uncompressed nested C4Group image.
    pub scenario_sections: Vec<LiveC4SaveNamedComponent>,
    /// Modified section targets whose native delete succeeded but whose
    /// ignored Add/copy failed. They remain absent in the closed group.
    pub deleted_scenario_sections: Vec<String>,
    /// Current-section deletes and modified-section delete/replacements in
    /// exact `Game.pScenarioSections` linked-list traversal order.
    pub scenario_section_mutations: Vec<LiveC4SaveScenarioSectionMutation>,
}

/// Components native has already committed when a later landscape operation
/// fails. The application applies this prefix to its destination C4Group
/// before it propagates the serializer error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveC4SavePreLandscapeComponents {
    pub scenario_txt: Vec<u8>,
    /// `None` means `C4Game::SaveData` failed before replacing Game.txt.
    /// SaveCore and the cleanup immediately following it have still run.
    pub game_txt: Option<Vec<u8>>,
    pub scenario_sections: Vec<LiveC4SaveNamedComponent>,
    pub deleted_scenario_sections: Vec<String>,
    pub scenario_section_mutations: Vec<LiveC4SaveScenarioSectionMutation>,
    /// Ordered destination mutations completed by `C4GameSave::SaveLandscape`
    /// before it reported an error.
    pub landscape_mutations: Vec<LiveC4SaveLandscapeMutation>,
}

/// One observable mutation already committed by native SaveLandscape before
/// a later operation failed. The application owns the copied root group and
/// replays this list before closing the failed save.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveC4SaveLandscapeMutation {
    DeleteEntry { name: String },
    PutFile { name: String, payload: Vec<u8> },
    MergeMaterialGroup { payload: Vec<u8> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveC4SaveComponentMutation {
    Delete { name: String },
    Replace(LiveC4SaveNamedComponent),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveC4SaveScenarioSectionMutation {
    Delete { name: String },
    Replace(LiveC4SaveNamedComponent),
}

/// Immutable C4StringTable ID assignment for one synchronized live save.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LiveC4ValueEnumeration {
    values: Vec<Vec<u8>>,
    /// C4Value object payloads are compiled through
    /// `Game.Objects.ObjectNumber`, not from the allocation's raw ID. `None`
    /// is retained for standalone reconstructed enumerations which have no
    /// live object-list context.
    object_numbers: Option<HashMap<u64, i32>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LiveC4ValueEncodeError {
    #[error("C4 string `{0}` was not included in the synchronized Strings.txt enumeration")]
    MissingString(String),
}

impl LiveC4ValueEnumeration {
    /// Reconstruct an immutable C4StringTable enumeration from Strings.txt
    /// line order. Each entry's position is its persisted `S<n>` ID.
    pub fn from_strings_in_id_order<T>(values: impl IntoIterator<Item = T>) -> Self
    where
        T: Into<C4StringValue>,
    {
        let mut enumeration = Self::default();
        for value in values {
            let value = value.into();
            let bytes = clonk_script::c4_string_bytes(&value);
            let enum_id = if let Some(index) = enumeration
                .values
                .iter()
                .position(|candidate| c4_string_table_bytes_equal(candidate, &bytes))
            {
                i32::try_from(index).unwrap_or(i32::MAX)
            } else {
                let enum_id = i32::try_from(enumeration.values.len()).unwrap_or(i32::MAX);
                enumeration.values.push(bytes);
                enum_id
            };
            value.set_enum_id(enum_id);
        }
        enumeration
    }

    pub fn encode_value(&self, value: &Value) -> Result<String, LiveC4ValueEncodeError> {
        encode_value_by(
            value,
            &mut |value| {
                let enum_id = value.enum_id();
                let bytes = clonk_script::c4_string_bytes(value);
                usize::try_from(enum_id)
                    .ok()
                    .and_then(|index| self.values.get(index))
                    .filter(|candidate| c4_string_table_bytes_equal(candidate, &bytes))
                    .map(|_| enum_id)
                    .ok_or_else(|| LiveC4ValueEncodeError::MissingString(value.to_string()))
            },
            &mut |object| self.object_number(object),
        )
    }

    pub fn strings_txt(&self) -> Option<Vec<u8>> {
        LegacyStringTable {
            values: self.values.clone(),
            object_numbers: self.object_numbers.clone(),
        }
        .encoded()
    }

    fn object_number(&self, object: u64) -> i32 {
        self.object_numbers.as_ref().map_or_else(
            || i32::try_from(object).unwrap_or(0),
            |numbers| numbers.get(&object).copied().unwrap_or(0),
        )
    }
}

impl LiveC4SaveComponents {
    /// Final `C4FLS_Scenario` order, including Scenario.txt. Optional entries
    /// are absent rather than represented by empty payloads.
    pub fn entries(&self) -> Vec<LiveC4SaveEntry<'_>> {
        let mut entries = vec![file_entry("Scenario.txt", &self.scenario_txt)];
        if let Some(title) = self.title_txt.as_ref() {
            entries.push(file_entry(&title.name, &title.payload));
        }
        if let Some(info) = self.info_txt.as_ref() {
            entries.push(file_entry(&info.name, &info.payload));
        }
        if !self.game_txt.is_empty() {
            entries.push(file_entry("Game.txt", &self.game_txt));
        }
        push_optional_file(&mut entries, "Teams.txt", self.teams_txt.as_deref());
        for section in &self.scenario_sections {
            entries.push(LiveC4SaveEntry {
                name: &section.name,
                payload: &section.payload,
                kind: LiveC4SaveEntryKind::ChildGroup,
            });
        }
        if let Some(payload) = self.material_group.as_deref() {
            entries.push(LiveC4SaveEntry {
                name: "Material.c4g",
                payload,
                kind: LiveC4SaveEntryKind::ChildGroup,
            });
        }
        if !self.mat_map_txt.is_empty() {
            entries.push(file_entry("MatMap.txt", &self.mat_map_txt));
        }
        push_optional_file(&mut entries, "Landscape.bmp", self.landscape_bmp.as_deref());
        push_optional_file(&mut entries, "Landscape.png", self.landscape_png.as_deref());
        push_optional_file(
            &mut entries,
            "DiffLandscape.bmp",
            self.diff_landscape_bmp.as_deref(),
        );
        push_optional_file(&mut entries, "Map.bmp", self.map_bmp.as_deref());
        push_optional_file(&mut entries, "PXS.c4b", self.pxs_c4b.as_deref());
        push_optional_file(
            &mut entries,
            "MassMover.c4b",
            self.mass_mover_c4b.as_deref(),
        );
        push_optional_file(&mut entries, "Strings.txt", self.strings_txt.as_deref());
        entries.push(file_entry("Objects.txt", &self.objects_txt));
        push_optional_file(
            &mut entries,
            "RoundResults.txt",
            self.round_results_txt.as_deref(),
        );
        if let Some(script) = self.script_c.as_ref() {
            entries.push(file_entry(&script.name, &script.payload));
        }
        entries
    }

    /// Merge app-owned SavePlayerInfos/player groups into the same final
    /// component view. C4Group Close performs the authoritative scenario
    /// sort; this helper supplies the otherwise-missing insertion surface.
    pub fn entries_with_app_owned<'a>(
        &'a self,
        save_player_infos: Option<&'a [u8]>,
        player_groups: &'a [LiveC4SaveEntry<'a>],
    ) -> Vec<LiveC4SaveEntry<'a>> {
        let mut entries = self.entries();
        push_optional_file(&mut entries, "SavePlayerInfos.txt", save_player_infos);
        entries.extend_from_slice(player_groups);
        entries
    }
}

fn file_entry<'a>(name: &'a str, payload: &'a [u8]) -> LiveC4SaveEntry<'a> {
    LiveC4SaveEntry {
        name,
        payload,
        kind: LiveC4SaveEntryKind::File,
    }
}

fn push_optional_file<'a>(
    entries: &mut Vec<LiveC4SaveEntry<'a>>,
    name: &'a str,
    payload: Option<&'a [u8]>,
) {
    if let Some(payload) = payload {
        entries.push(file_entry(name, payload));
    }
}

#[derive(Debug, Error)]
pub enum LiveC4SaveError {
    #[error("{source}")]
    AfterPreLandscape {
        source: Box<LiveC4SaveError>,
        partial: Box<LiveC4SavePreLandscapeComponents>,
    },
    #[error("failed to serialize live Game.txt: {0}")]
    Game(#[from] InitialNetworkGameError),
    #[error("live save requires a retained landscape")]
    MissingLandscape,
    #[error("live save requires an exact Surface8 pixel grid")]
    MissingPixelGrid,
    #[error("failed to encode a landscape bitmap: {0}")]
    Bitmap(#[from] BitmapError),
    #[error("failed to encode Landscape.png: {0}")]
    Png(#[from] image::ImageError),
    #[error("failed to serialize live landscape state: {0}")]
    Landscape(#[from] crate::LandscapePersistenceError),
    #[error("failed to compose a temporary C4Group: {0}")]
    GroupWrite(#[from] MutableGroupError),
    #[error("failed to inspect a temporary C4Group: {0}")]
    GroupRead(#[from] GroupError),
}

impl LiveC4SaveError {
    pub fn pre_landscape_components(&self) -> Option<&LiveC4SavePreLandscapeComponents> {
        match self {
            Self::AfterPreLandscape { partial, .. } => Some(partial),
            _ => None,
        }
    }

    pub fn root_cause(&self) -> &Self {
        match self {
            Self::AfterPreLandscape { source, .. } => source.root_cause(),
            _ => self,
        }
    }
}

/// The object-number table installed by `C4GameObjects::EnumeratePointers`.
///
/// Native pointer denumeration only searches the active and inactive object
/// lists. Objects retained elsewhere in the engine are therefore deliberately
/// absent from this table even when their allocation is still alive.
pub(super) struct LiveObjectPointerEnumeration {
    wrapper_ids: Vec<ObjectId>,
    objects_by_number: HashMap<i32, ObjectId>,
}

impl Engine {
    /// Run the string-table portion of `C4Game::SaveRuntimeData` for the
    /// standalone JSON save wrapper. The returned byte rows preserve the
    /// exact enumeration needed if that virtual savegame later starts an
    /// initial C4 record.
    pub fn enumerate_live_c4_string_table_for_save(&mut self) -> Vec<Vec<u8>> {
        let state = self.capture_state();
        let referenced_strings = collect_live_referenced_strings(self, &state);
        clonk_script::enumerate_c4_strings(&self.script_string_registrations, &referenced_strings)
    }

    /// Install the exact `Strings.txt` line order loaded with a virtual JSON
    /// save before its runtime state is restored.
    pub fn adopt_loaded_c4_string_table(&mut self, values: &[Vec<u8>]) {
        let registrations = clonk_script::new_string_registrations();
        for (index, value) in values.iter().enumerate() {
            let enum_id = i32::try_from(index).unwrap_or(i32::MAX);
            let value = clonk_script::c4_string_from_bytes(value);
            clonk_script::register_loaded_c4_string(&registrations, enum_id, &value);
        }
        self.adopt_legacy_string_table(registrations);
    }

    /// Build fInitial `Scenario.txt` from the restored runtime C4Scenario.
    /// This is used when Rust's JSON save has no packed C4 source group to
    /// copy before starting a record.
    pub fn serialize_initial_record_scenario_from_runtime_savegame(
        &self,
        record_title: &str,
        definition_modules: &[String],
        definition_executable_path: &str,
        definition_path: &str,
        scenario_origin: &str,
    ) -> Vec<u8> {
        self.scenario_values
            .serialize_initial_record_from_runtime_savegame(
                record_title,
                definition_modules,
                definition_executable_path,
                definition_path,
                scenario_origin,
            )
    }

    /// Capture the exact `C4Game::SaveData(..., fInitial=true, fSaveExact=true)`
    /// projection used when a record starts. Callers take this snapshot after
    /// scenario load but before Script.Initialize, matching C4Record::Start.
    pub fn capture_initial_record_game_data(
        &mut self,
        music_enabled: bool,
    ) -> Result<InitialNetworkGameData, InitialNetworkGameError> {
        let state = self.capture_state();
        // InitControl starts the initial record while PointersDenumerated is
        // still false. Native SaveData therefore does not EnumStrings and
        // writes every C4Value using its already-loaded enum ID. The copied
        // scenario's existing Strings.txt remains the matching table.
        let mut strings = LegacyStringTable::default();
        let object_numbers = self.live_object_numbers_for_save();
        strings.set_object_numbers(object_numbers.clone());

        let mut game = InitialNetworkGameData::from_engine_live(self)?;
        game.music_enabled = music_enabled;
        game.compiled_sections = InitialNetworkCompiledSections {
            script_engine: serialize_script_globals(
                &state.script_globals,
                &self.script_global_name_order(),
                self.scenario_script_go,
                self.scenario_script_counter,
                &mut strings,
            ),
            sky: state.sky.as_ref().and_then(serialize_sky),
            effects: serialize_effects(&state.global_effects, &mut strings),
            scoreboard: serialize_scoreboard(&state.scoreboard),
        };
        Ok(game)
    }

    /// Capture every live C4 component owned by the simulation at the native
    /// `C4Network2::OnGameSynchronized` boundary.
    ///
    /// The caller must invoke this after
    /// `execute_synchronize_control_before_network(false)` and before
    /// `execute_synchronize_control_after_network(true)`.
    pub fn serialize_live_c4_save(
        &mut self,
        spec: LiveC4SaveSpec<'_>,
    ) -> Result<LiveC4SaveComponents, LiveC4SaveError> {
        self.serialize_live_c4_save_with_policy(spec, LiveC4SavePolicy::RuntimeNetwork)
    }

    /// Capture live components using the selected native `C4GameSave`
    /// specialization. Application-owned copy/delete and player-file work is
    /// described by [`LiveC4SavePolicy`] and remains outside the simulation.
    pub fn serialize_live_c4_save_with_policy(
        &mut self,
        spec: LiveC4SaveSpec<'_>,
        policy: LiveC4SavePolicy<'_>,
    ) -> Result<LiveC4SaveComponents, LiveC4SaveError> {
        let state = self.capture_state();
        // C4Game::SaveRuntimeData enumerates the process-global C4StringTable
        // before any component is decompiled. Registration order, rather than
        // the later save traversal, therefore determines every `S<n>` ID.
        // Filter out dead registrations exactly as C4StringTable::EnumStrings
        // does, but include values in player groups saved after the scenario.
        let referenced_strings = collect_live_referenced_strings(self, &state);
        let mut strings =
            LegacyStringTable::from_enumerated_values(clonk_script::enumerate_c4_strings(
                &self.script_string_registrations,
                &referenced_strings,
            ));
        let game_object_numbers = self.live_object_numbers_for_save();
        strings.set_object_numbers(game_object_numbers.clone());
        // C4GameSave::SaveRuntimeData writes Strings.txt immediately after
        // EnumStrings and before Objects.Save performs its second enumeration.
        let value_enumeration = strings.enumeration();
        let strings_txt = strings.encoded();
        let scenario_txt = serialize_scenario_for_policy(&self.scenario_values, spec, policy);

        let script_engine = serialize_script_globals(
            &state.script_globals,
            &self.script_global_name_order(),
            self.scenario_script_go,
            self.scenario_script_counter,
            &mut strings,
        );
        let effects = serialize_effects(&state.global_effects, &mut strings);
        let game_txt = match (|| -> Result<Vec<u8>, LiveC4SaveError> {
            Ok(if policy.is_exact() {
                let mut game = InitialNetworkGameData::from_engine_live(self)?;
                game.music_enabled = spec.music_enabled;
                game.compiled_sections = InitialNetworkCompiledSections {
                    script_engine,
                    sky: state.sky.as_ref().and_then(serialize_sky),
                    effects,
                    scoreboard: serialize_scoreboard(&state.scoreboard),
                };
                let player_sections = serialize_players(self, &state.players, &mut strings);
                append_runtime_player_sections(
                    serialize_initial_network_game(&game, None)?.unwrap_or_default(),
                    &player_sections,
                )
            } else {
                serialize_non_exact_game(script_engine, effects)
            })
        })() {
            Ok(game_txt) => game_txt,
            Err(source) => {
                return Err(LiveC4SaveError::AfterPreLandscape {
                    source: Box::new(source),
                    partial: Box::new(LiveC4SavePreLandscapeComponents {
                        scenario_txt,
                        game_txt: None,
                        scenario_sections: Vec::new(),
                        deleted_scenario_sections: Vec::new(),
                        scenario_section_mutations: Vec::new(),
                        landscape_mutations: Vec::new(),
                    }),
                });
            }
        };
        // C4Game::SaveData enumerates and denumerates player/global-effect
        // wrappers before SaveRuntimeData reaches fallible section/landscape
        // writes. Preserve that observable failure-side-effect ordering.
        self.denumerate_game_save_pointer_fields(
            &game_object_numbers.keys().copied().collect::<HashSet<_>>(),
        );

        let (scenario_sections, deleted_scenario_sections, scenario_section_mutations) =
            if policy.is_exact() {
                serialize_scenario_sections(self, &mut strings)
            } else {
                (Vec::new(), Vec::new(), Vec::new())
            };
        let landscape = match serialize_landscape_for_policy(
            self,
            policy,
            spec.copied_material_group_is_file,
        ) {
            Ok(landscape) => landscape,
            Err(failure) => {
                return Err(LiveC4SaveError::AfterPreLandscape {
                    source: Box::new(failure.source),
                    partial: Box::new(LiveC4SavePreLandscapeComponents {
                        scenario_txt,
                        game_txt: Some(game_txt),
                        scenario_sections,
                        deleted_scenario_sections,
                        scenario_section_mutations,
                        landscape_mutations: failure.mutations,
                    }),
                });
            }
        };
        // Native reaches C4GameObjects::Save only after every exact section
        // and landscape write succeeded. Keep its enumerate/denumerate side
        // effects behind those fallible operations as well.
        let object_enumeration = self.enumerate_object_compiler_caches_for_save();
        let objects_txt = serialize_objects_for_save(
            self,
            &mut strings,
            matches!(policy, LiveC4SavePolicy::Scenario { .. }),
        );
        self.denumerate_object_compiler_caches_after_save(&object_enumeration);

        let mut deleted_components = Vec::new();
        let mut component_host_mutations = Vec::new();
        // SaveRuntimeData visits mutable hosts only after Teams, in
        // Script/Title/Info order.
        let script_c = materialize_component_host(
            spec.script_component,
            &mut deleted_components,
            &mut component_host_mutations,
        );
        let title_txt = policy
            .keeps_title_components()
            .then(|| {
                materialize_component_host(
                    spec.title_component,
                    &mut deleted_components,
                    &mut component_host_mutations,
                )
            })
            .flatten();
        let info_txt = materialize_component_host(
            spec.info_component,
            &mut deleted_components,
            &mut component_host_mutations,
        );

        Ok(LiveC4SaveComponents {
            scenario_txt,
            title_txt,
            game_txt,
            objects_txt,
            strings_txt,
            value_enumeration,
            landscape_bmp: landscape.landscape_bmp,
            landscape_png: landscape.landscape_png,
            diff_landscape_bmp: landscape.diff_landscape_bmp,
            map_bmp: landscape.map_bmp,
            material_group: landscape.material_group,
            mat_map_txt: landscape.mat_map_txt,
            pxs_c4b: landscape.pxs_c4b,
            mass_mover_c4b: landscape.mass_mover_c4b,
            delete_sky_entry: landscape.delete_sky_entry,
            teams_txt: serialize_teams(
                &state.teams,
                state
                    .team_configuration
                    .unwrap_or(self.team_state.team_configuration),
                state.team_last_team_id,
                state.team_max_script_players,
                &state.team_script_player_names,
                state.team_random_team_count,
            ),
            round_results_txt: policy
                .is_exact()
                .then(|| {
                    serialize_round_results(&state.round_results, self.scenario_values.is_melee())
                })
                .flatten(),
            info_txt,
            script_c,
            deleted_components,
            component_host_mutations,
            scenario_sections,
            deleted_scenario_sections,
            scenario_section_mutations,
        })
    }

    fn live_object_numbers_for_save(&self) -> HashMap<u64, i32> {
        let mut seen = HashSet::new();
        self.exec_list
            .iter()
            .chain(&self.inactive_exec_list)
            .copied()
            .filter(|id| seen.insert(*id) && self.find_object_index(*id).is_some())
            .filter_map(|id| {
                i32::try_from(id.as_u64())
                    .ok()
                    .map(|number| (id.as_u64(), number))
            })
            .collect()
    }

    fn denumerate_game_save_pointer_fields(&mut self, object_numbers: &HashSet<u64>) {
        for player in self.players.values_mut() {
            player.denumerate_live_save_pointer_fields(object_numbers);
        }
        for effect in &mut self.global_effects {
            denumerate_effect_command_target(effect, object_numbers);
        }
    }

    pub(super) fn enumerate_object_compiler_caches_for_save(
        &mut self,
    ) -> LiveObjectPointerEnumeration {
        let mut seen = HashSet::new();
        let listed_ids = self
            .exec_list
            .iter()
            .chain(&self.inactive_exec_list)
            .copied()
            .filter(|id| seen.insert(*id) && self.find_object_index(*id).is_some())
            .collect::<Vec<_>>();
        // C4ObjectList::ObjectNumber searches every link, including a status
        // zero object awaiting removal. C4ObjectList::Enumerate, however,
        // invokes EnumeratePointers only for live source wrappers.
        let wrapper_ids = listed_ids
            .iter()
            .copied()
            .filter(|id| {
                self.find_object_index(*id).is_some_and(|index| {
                    let object = &self.objects[index];
                    !object.destroyed && object.state.status != ObjectStatus::Deleted
                })
            })
            .collect::<Vec<_>>();
        let object_numbers = self
            .live_object_numbers_for_save()
            .into_iter()
            .map(|(id, number)| (ObjectId::new(id), number))
            .collect::<HashMap<_, _>>();
        let objects_by_number = object_numbers
            .iter()
            .map(|(id, number)| (*number, *id))
            .collect::<HashMap<_, _>>();
        let info_names = self
            .crew_object_infos
            .iter()
            .map(|(id, info)| (*id, info.name.clone()))
            .collect::<HashMap<_, _>>();
        let object_number = |target: Option<ObjectId>| {
            target
                .and_then(|target| object_numbers.get(&target).copied())
                .unwrap_or(0)
        };

        for id in &wrapper_ids {
            let Some(index) = self.find_object_index(*id) else {
                continue;
            };
            let object = &mut self.objects[index];
            object.compiler_cache.info = info_names.get(&object.id).cloned().unwrap_or_default();
            object.compiler_cache.contained = object_number(object.state.container);
            object.compiler_cache.action_target1 = object_number(object.state.action.target);
            object.compiler_cache.action_target2 = object_number(object.state.action.target2);
            object.compiler_cache.layer = object_number(object.state.layer);
        }

        LiveObjectPointerEnumeration {
            wrapper_ids,
            objects_by_number,
        }
    }

    pub(super) fn denumerate_object_compiler_caches_after_save(
        &mut self,
        enumeration: &LiveObjectPointerEnumeration,
    ) {
        for id in &enumeration.wrapper_ids {
            let Some(index) = self.find_object_index(*id) else {
                continue;
            };
            let object = &mut self.objects[index];
            object.state.container = enumeration
                .objects_by_number
                .get(&object.compiler_cache.contained)
                .copied();
            object.state.action.target = enumeration
                .objects_by_number
                .get(&object.compiler_cache.action_target1)
                .copied();
            object.state.action.target2 = enumeration
                .objects_by_number
                .get(&object.compiler_cache.action_target2)
                .copied();
            object.state.layer = enumeration
                .objects_by_number
                .get(&object.compiler_cache.layer)
                .copied();
        }

        // The remaining explicit pointer adapters participate in the same
        // native enumerate/denumerate pass. Ordinary C4Value::Object cells do
        // not: their compiler uses a temporary ObjectNumber and leaves the
        // live value untouched.
        let object_numbers = enumeration
            .objects_by_number
            .values()
            .map(|object| object.as_u64())
            .collect::<HashSet<_>>();
        let wrapper_ids = enumeration
            .wrapper_ids
            .iter()
            .copied()
            .collect::<HashSet<_>>();
        for object in &mut self.objects {
            if !wrapper_ids.contains(&object.id) {
                continue;
            }
            for effect in &mut object.state.effects {
                denumerate_effect_command_target(effect, &object_numbers);
            }
            for overlay in &mut object.state.graphics_overlays {
                if overlay
                    .overlay_object
                    .is_some_and(|target| !object_numbers.contains(&target.as_u64()))
                {
                    overlay.overlay_object = None;
                }
            }
            object
                .commands
                .denumerate_compiled_pointer_fields(&object_numbers);
        }
    }
}

fn materialize_component_host(
    host: LiveC4ComponentHost<'_>,
    deleted_components: &mut Vec<String>,
    mutations: &mut Vec<LiveC4SaveComponentMutation>,
) -> Option<LiveC4SaveNamedComponent> {
    match host {
        LiveC4ComponentHost::Unmodified => None,
        LiveC4ComponentHost::Replace(component) => {
            let component = LiveC4SaveNamedComponent {
                name: component.name.to_owned(),
                payload: component.payload.to_vec(),
            };
            mutations.push(LiveC4SaveComponentMutation::Replace(component.clone()));
            Some(component)
        }
        LiveC4ComponentHost::Delete { name } => {
            deleted_components.push(name.to_owned());
            mutations.push(LiveC4SaveComponentMutation::Delete {
                name: name.to_owned(),
            });
            None
        }
    }
}

fn denumerate_effect_command_target(effect: &mut EffectState, object_numbers: &HashSet<u64>) {
    if effect.command_target.is_some_and(|target| {
        u64::try_from(target)
            .ok()
            .is_none_or(|target| !object_numbers.contains(&target))
    }) {
        effect.command_target = None;
    }
}

fn serialize_non_exact_game(script_engine: Option<Vec<u8>>, effects: Option<Vec<u8>>) -> Vec<u8> {
    let mut output = script_engine.unwrap_or_default();
    if let Some(effects) = effects {
        if !output.is_empty() {
            if !output.ends_with(b"\r\n") {
                output.extend_from_slice(b"\r\n");
            }
            output.extend_from_slice(b"\r\n");
        }
        output.extend_from_slice(&effects);
    }
    output
}

#[derive(Default, Clone)]
struct LegacyStringTable {
    values: Vec<Vec<u8>>,
    object_numbers: Option<HashMap<u64, i32>>,
}

fn c4_string_table_prefix(bytes: &[u8]) -> &[u8] {
    &bytes[..bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len())]
}

// C4StringTable looks entries up with SEqual, so enumeration identity and the
// serialized Strings.txt payload both stop at the first C-string NUL.
fn c4_string_table_bytes_equal(left: &[u8], right: &[u8]) -> bool {
    c4_string_table_prefix(left) == c4_string_table_prefix(right)
}

impl LegacyStringTable {
    fn from_enumerated_values(values: Vec<Vec<u8>>) -> Self {
        Self {
            values,
            object_numbers: None,
        }
    }

    fn set_object_numbers(&mut self, object_numbers: HashMap<u64, i32>) {
        self.object_numbers = Some(object_numbers);
    }

    fn replace_object_numbers(
        &mut self,
        object_numbers: HashMap<u64, i32>,
    ) -> Option<HashMap<u64, i32>> {
        self.object_numbers.replace(object_numbers)
    }

    fn object_number(&self, object: u64) -> i32 {
        self.object_numbers.as_ref().map_or_else(
            || i32::try_from(object).unwrap_or(0),
            |numbers| numbers.get(&object).copied().unwrap_or(0),
        )
    }

    fn object_id_number(&self, object: Option<ObjectId>) -> i32 {
        object.map_or(0, |object| self.object_number(object.as_u64()))
    }

    fn legacy_object_number(&self, object: Option<i32>) -> i32 {
        object.map_or(0, |object| {
            u64::try_from(object)
                .ok()
                .map_or(0, |object| self.object_number(object))
        })
    }

    fn push_unique(&mut self, bytes: Vec<u8>) {
        if !self
            .values
            .iter()
            .any(|candidate| c4_string_table_bytes_equal(candidate, &bytes))
        {
            self.values.push(bytes);
        }
    }

    fn id_for(&mut self, value: &str) -> i32 {
        let bytes = clonk_script::c4_string_bytes(value);
        if let Some(index) = self
            .values
            .iter()
            .position(|candidate| c4_string_table_bytes_equal(candidate, &bytes))
        {
            return i32::try_from(index).unwrap_or(i32::MAX);
        }
        let id = i32::try_from(self.values.len()).unwrap_or(i32::MAX);
        self.values.push(bytes);
        id
    }

    fn encoded(&self) -> Option<Vec<u8>> {
        if self.values.is_empty() {
            return None;
        }
        // C4StringTable::Save calculates the C4Group entry size before it
        // removes embedded line feeds. The normalized payload is therefore
        // followed by one unused byte for every removed LF. Native leaves
        // bytes beyond the first terminating NUL unspecified; zero-fill that
        // tail so the observable allocation length remains deterministic.
        let encoded_size = self.values.iter().fold(0_usize, |size, value| {
            size.saturating_add(c4_string_table_prefix(value).len().saturating_add(2))
        });
        let mut output = Vec::new();
        for mut value in self.values.clone() {
            let c_string_len = c4_string_table_prefix(&value).len();
            value.truncate(c_string_len);
            value.retain(|byte| *byte != b'\n');
            for byte in &mut value {
                if *byte == b'\r' {
                    *byte = b'|';
                }
            }
            output.extend_from_slice(&value);
            output.extend_from_slice(b"\r\n");
        }
        output.resize(encoded_size, 0);
        Some(output)
    }

    fn finish(self) -> Option<Vec<u8>> {
        self.encoded()
    }

    fn enumeration(&self) -> LiveC4ValueEnumeration {
        LiveC4ValueEnumeration {
            values: self.values.clone(),
            object_numbers: self.object_numbers.clone(),
        }
    }
}

fn collect_live_referenced_strings(
    engine: &Engine,
    state: &crate::EngineState,
) -> Vec<C4StringValue> {
    fn push_string(strings: &mut Vec<C4StringValue>, value: &C4StringValue) {
        if !strings.iter().any(|candidate| candidate.ptr_eq(value)) {
            strings.push(value.clone());
        }
    }

    fn collect_value(strings: &mut Vec<C4StringValue>, item: &Value) {
        match item {
            Value::String(text) => push_string(strings, text),
            Value::Array(values) => {
                for item in values {
                    collect_value(strings, item);
                }
            }
            Value::Proplist(values) => {
                for (key, item) in values {
                    collect_value(strings, key);
                    collect_value(strings, item);
                }
                for item in values.hidden_values() {
                    collect_value(strings, item);
                }
            }
            Value::Nil
            | Value::Int(_)
            | Value::Bool(_)
            | Value::RawBool(_)
            | Value::C4Id(_)
            | Value::Object(_) => {}
        }
    }

    fn effect_value(strings: &mut Vec<C4StringValue>, value: &EffectVarValue) {
        match value {
            EffectVarValue::String(value) => push_string(strings, value),
            EffectVarValue::Array(values) => {
                for item in values {
                    effect_value(strings, item);
                }
            }
            EffectVarValue::Proplist(values) => {
                for (key, item) in values {
                    collect_value(strings, key);
                    collect_value(strings, item);
                }
                for item in values.hidden_values() {
                    collect_value(strings, item);
                }
            }
            EffectVarValue::Nil
            | EffectVarValue::Int(_)
            | EffectVarValue::Bool(_)
            | EffectVarValue::RawBool(_)
            | EffectVarValue::C4Id(_)
            | EffectVarValue::Object(_) => {}
        }
    }

    fn effects(strings: &mut Vec<C4StringValue>, effects: &[EffectState]) {
        for effect in effects {
            for item in &effect.vars {
                effect_value(strings, item);
            }
        }
    }

    fn object_values(strings: &mut Vec<C4StringValue>, object: &crate::PersistedObject) {
        for item in object.snapshot.local_vars.values() {
            collect_value(strings, item);
        }
        effects(strings, &object.snapshot.effects);
        for command in object.command_stack.legacy_save_commands() {
            if let Some(value) = command.view.tx_value.as_ref() {
                collect_value(strings, value);
            }
        }
    }

    fn spawn_values(strings: &mut Vec<C4StringValue>, spawn: &crate::SpawnConfig) {
        for item in spawn.local_vars.values() {
            collect_value(strings, item);
        }
        effects(strings, &spawn.effects);
        if let Some(commands) = spawn.command_stack.as_ref() {
            for command in commands.legacy_save_commands() {
                if let Some(value) = command.view.tx_value.as_ref() {
                    collect_value(strings, value);
                }
            }
        }
    }

    let mut strings = Vec::new();
    // Static constants are C4Values owned by C4AulScriptEngine and participate
    // in the same process-global string enumeration as mutable globals.
    let constant_cells = engine
        .script_global_consts
        .borrow()
        .values()
        .cloned()
        .collect::<Vec<_>>();
    for cell in constant_cells {
        collect_value(&mut strings, &cell.borrow());
    }
    for item in state.script_globals.numbered.values() {
        collect_value(&mut strings, item);
    }
    for item in state.script_globals.named.values() {
        collect_value(&mut strings, item);
    }
    effects(&mut strings, &state.global_effects);
    for object in &state.objects {
        object_values(&mut strings, object);
    }
    for player in &state.players {
        for (_, item) in &player.extra_data {
            collect_value(&mut strings, item);
        }
        if let Some(roster) = state.crew_info_rosters.get(&player.id) {
            for info in roster {
                for (_, item) in &info.extra_data {
                    collect_value(&mut strings, item);
                }
            }
        }
    }
    for section in engine.scenario_sections.values() {
        // A departed section owns a frozen Strings.txt/Objects.txt pair.
        // Its removed objects no longer hold C4String references in the live
        // table and must not leak their private values into the root save.
        if !section.modified || section.frozen_group.is_some() {
            continue;
        }
        if let Some(objects) = section.saved_objects.as_deref() {
            for object in objects {
                object_values(&mut strings, object);
            }
        } else {
            for spawn in &section.initial_objects {
                spawn_values(&mut strings, &spawn.config);
            }
        }
    }
    strings
}

#[derive(Default)]
struct TextComponentWriter {
    output: Vec<u8>,
}

impl TextComponentWriter {
    fn section(&mut self, indent: usize, name: &str) {
        if !self.output.is_empty() {
            self.output.extend_from_slice(b"\r\n");
        }
        self.output.extend(std::iter::repeat_n(b' ', indent));
        self.output.push(b'[');
        self.output.extend_from_slice(name.as_bytes());
        self.output.extend_from_slice(b"]\r\n");
    }

    fn field(&mut self, indent: usize, name: &str, value: impl AsRef<str>) {
        self.output.extend(std::iter::repeat_n(b' ', indent));
        self.output.extend_from_slice(name.as_bytes());
        self.output.push(b'=');
        self.output
            .extend_from_slice(&clonk_script::c4_string_bytes(value.as_ref()));
        self.output.extend_from_slice(b"\r\n");
    }

    fn field_bytes(&mut self, indent: usize, name: &str, value: &[u8]) {
        self.output.extend(std::iter::repeat_n(b' ', indent));
        self.output.extend_from_slice(name.as_bytes());
        self.output.push(b'=');
        self.output.extend_from_slice(value);
        self.output.extend_from_slice(b"\r\n");
    }

    fn finish(self) -> Vec<u8> {
        self.output
    }
}

fn quote_ini(value: &str) -> String {
    clonk_script::c4_string_from_bytes(&quote_ini_bytes(&clonk_script::c4_string_bytes(value)))
}

fn quote_ini_bytes(value: &[u8]) -> Vec<u8> {
    // StdCompilerINIWrite::StringN uses strlen even for std::string-backed
    // adapters. Match that C-string boundary before applying RCT_Escaped.
    let value = &value[..value
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(value.len())];
    let mut output = Vec::with_capacity(value.len().saturating_add(2));
    output.push(b'"');
    let mut last_was_numeric_escape = false;
    for &byte in value {
        if !(b' '..=b'~').contains(&byte)
            || byte == b'\\'
            || byte == b'"'
            || (last_was_numeric_escape && byte.is_ascii_digit())
        {
            last_was_numeric_escape = false;
            match byte {
                0x07 => output.extend_from_slice(b"\\a"),
                0x08 => output.extend_from_slice(b"\\b"),
                0x0c => output.extend_from_slice(b"\\f"),
                b'\n' => output.extend_from_slice(b"\\n"),
                b'\r' => output.extend_from_slice(b"\\r"),
                b'\t' => output.extend_from_slice(b"\\t"),
                0x0b => output.extend_from_slice(b"\\v"),
                b'"' => output.extend_from_slice(b"\\\""),
                b'\\' => output.extend_from_slice(b"\\\\"),
                _ => {
                    output.push(b'\\');
                    output.extend_from_slice(format!("{byte:o}").as_bytes());
                    last_was_numeric_escape = true;
                }
            }
        } else {
            output.push(byte);
            last_was_numeric_escape = false;
        }
    }
    output.push(b'"');
    output
}

fn encode_value_by<E>(
    value: &Value,
    id_for: &mut impl FnMut(&C4StringValue) -> Result<i32, E>,
    object_number: &mut impl FnMut(u64) -> i32,
) -> Result<String, E> {
    match value {
        Value::Nil => Ok("A0".to_owned()),
        Value::Int(value) => Ok(format!("i{value}")),
        Value::Bool(value) => Ok(format!("b{}", i32::from(*value))),
        Value::RawBool(value) => Ok(format!("b{}", *value as u32 as i32)),
        Value::String(value) => Ok(format!("S{}", id_for(value)?)),
        Value::C4Id(value) => {
            let raw = clonk_script::c4_id_raw(value) as u32 as i32;
            Ok(format!("I{raw}"))
        }
        Value::Object(value) => Ok(format!("O{}", object_number(*value))),
        Value::Array(values) => {
            let values = values
                .iter()
                .map(|value| encode_value_by(value, id_for, object_number))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(format!("a[{};{}]", values.len(), values.join(",")))
        }
        Value::Proplist(values) => {
            let values = values
                .iter()
                .map(|(key, value)| {
                    Ok(format!(
                        "{}={}",
                        encode_value_by(key, id_for, object_number)?,
                        encode_value_by(value, id_for, object_number)?
                    ))
                })
                .collect::<Result<Vec<_>, E>>()?;
            Ok(format!("m[{};{}]", values.len(), values.join(";")))
        }
    }
}

fn encode_value(value: &Value, _strings: &mut LegacyStringTable) -> String {
    encode_value_by(
        value,
        &mut |value| Ok::<_, std::convert::Infallible>(value.enum_id()),
        &mut |object| _strings.object_number(object),
    )
    .expect("infallible mutable string-table enumeration")
}

pub(crate) fn encode_value_with_current_string_ids(value: &Value) -> String {
    encode_value_by(
        value,
        &mut |value| Ok::<_, std::convert::Infallible>(value.enum_id()),
        &mut |object| i32::try_from(object).unwrap_or(0),
    )
    .expect("infallible mutable string-table enumeration")
}

fn encode_effect_value(value: &EffectVarValue, strings: &mut LegacyStringTable) -> String {
    match value {
        EffectVarValue::Nil => "A0".to_owned(),
        EffectVarValue::Int(value) => format!("i{value}"),
        EffectVarValue::Bool(value) => format!("b{}", i32::from(*value)),
        EffectVarValue::RawBool(value) => format!("b{}", *value as u32 as i32),
        EffectVarValue::String(value) => format!("S{}", value.enum_id()),
        EffectVarValue::C4Id(value) => {
            let raw = clonk_script::c4_id_raw(value) as u32 as i32;
            format!("I{raw}")
        }
        EffectVarValue::Object(value) => format!("O{}", strings.object_number(*value)),
        EffectVarValue::Array(values) => format!(
            "a[{};{}]",
            values.len(),
            values
                .iter()
                .map(|value| encode_effect_value(value, strings))
                .collect::<Vec<_>>()
                .join(",")
        ),
        EffectVarValue::Proplist(values) => format!(
            "m[{};{}]",
            values.len(),
            values
                .iter()
                .map(|(key, value)| format!(
                    "{}={}",
                    encode_value(key, strings),
                    encode_value(value, strings)
                ))
                .collect::<Vec<_>>()
                .join(";")
        ),
    }
}

fn serialize_scenario(values: &ScenarioValueStore, spec: LiveC4SaveSpec<'_>) -> Vec<u8> {
    serialize_scenario_for_policy(values, spec, LiveC4SavePolicy::RuntimeNetwork)
}

fn serialize_scenario_for_policy(
    values: &ScenarioValueStore,
    spec: LiveC4SaveSpec<'_>,
    policy: LiveC4SavePolicy<'_>,
) -> Vec<u8> {
    match policy {
        LiveC4SavePolicy::Scenario { .. } => values.serialize_runtime_scenario_save(),
        LiveC4SavePolicy::Savegame { target_group_name } => values.serialize_runtime_savegame(
            spec.title,
            spec.definition_modules,
            spec.definition_executable_path,
            spec.definition_path,
            spec.origin,
            savegame_icon(target_group_name),
        ),
        LiveC4SavePolicy::Record => values.serialize_runtime_record_save(
            spec.title,
            spec.definition_modules,
            spec.definition_executable_path,
            spec.definition_path,
            spec.origin,
        ),
        LiveC4SavePolicy::RuntimeNetwork => values.serialize_runtime_network_save(
            spec.title,
            spec.definition_modules,
            spec.definition_executable_path,
            spec.definition_path,
            spec.origin,
        ),
    }
}

fn savegame_icon(target_group_name: &str) -> i32 {
    let file_name = target_group_name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(target_group_name);
    let stem = file_name
        .rfind('.')
        .map_or(file_name, |extension| &file_name[..extension]);
    let digits_start = stem
        .char_indices()
        .rev()
        .find_map(|(index, character)| {
            (!character.is_ascii_digit()).then_some(index + character.len_utf8())
        })
        .unwrap_or(0);
    let slot = stem[digits_start..].parse::<i32>().ok();
    match slot {
        Some(slot @ 1..=10) => slot + 1,
        _ => 29,
    }
}

fn serialize_script_globals(
    globals: &crate::ScriptGlobalState,
    named_order: &[String],
    go: bool,
    counter: i32,
    strings: &mut LegacyStringTable,
) -> Option<Vec<u8>> {
    let mut writer = TextComponentWriter::default();
    writer.section(0, "Script");
    if go {
        writer.field(0, "Go", "true");
    }
    if counter != 0 {
        writer.field(0, "Counter", counter.to_string());
    }
    if let Some((&last, _)) = globals.numbered.last_key_value() {
        let size = usize::try_from(last.saturating_add(1)).unwrap_or(0);
        let values = (0..size)
            .map(|index| {
                globals
                    .numbered
                    .get(&(index as i32))
                    .map_or_else(|| "A0".to_owned(), |value| encode_value(value, strings))
            })
            .collect::<Vec<_>>();
        writer.field(0, "Globals", format!("{size};{}", values.join(",")));
    }
    if !globals.named.is_empty() {
        // C4ValueMapNames is append-only and the script linker adds static
        // declarations in link order. The BTreeMap snapshot is only a value
        // lookup; alphabetic traversal here would renumber GlobalNamed.
        let mut emitted = std::collections::HashSet::new();
        let mut values = Vec::with_capacity(globals.named.len());
        for name in named_order {
            if let Some(value) = globals.named.get(name) {
                emitted.insert(name.as_str());
                values.push(format!("{name}={}", encode_value(value, strings)));
            }
        }
        // Retain compatibility with old Rust snapshots whose names predate
        // the runtime declaration-order ledger.
        for (name, value) in &globals.named {
            if emitted.insert(name.as_str()) {
                values.push(format!("{name}={}", encode_value(value, strings)));
            }
        }
        writer.field(
            0,
            "GlobalNamed",
            format!("{};{}", values.len(), values.join(",")),
        );
    }
    let output = writer.finish();
    (output.as_slice() != b"[Script]\r\n").then_some(output)
}

fn append_runtime_player_sections(mut game_txt: Vec<u8>, player_sections: &[u8]) -> Vec<u8> {
    if player_sections.is_empty() {
        return game_txt;
    }
    // Runtime C4Game::CompileFunc decompiles C4PlayerList in the same pass,
    // so StdCompiler contributes the ordinary single blank section separator.
    // The initial-save `original_game_text` compatibility path adds two extra
    // CRLF pairs and must not be used here.
    if !game_txt.is_empty() {
        if !game_txt.ends_with(b"\r\n") {
            game_txt.extend_from_slice(b"\r\n");
        }
        game_txt.extend_from_slice(b"\r\n");
    }
    game_txt.extend_from_slice(player_sections);
    game_txt
}

fn serialize_sky(sky: &SkyFrame) -> Option<Vec<u8>> {
    let mut writer = TextComponentWriter::default();
    let fixed = sky.fixed.unwrap_or([
        crate::math::ftofix(sky.offset_x).val(),
        crate::math::ftofix(sky.offset_y).val(),
        crate::math::ftofix(sky.settings.base_xdir).val(),
        crate::math::ftofix(sky.settings.base_ydir).val(),
    ]);
    let modulation = sky.settings.modulation.unwrap_or(0x00ff_ffff);
    let par_mode = match sky.settings.parallax_mode {
        SkyParallaxMode::Fixed => 0,
        SkyParallaxMode::Wind => 1,
        SkyParallaxMode::Parallax => 2,
    };
    let back_enabled = sky.settings.back_color.is_some();
    let nondefault = fixed != [0; 4]
        || modulation != 0x00ff_ffff
        || sky.settings.parallax_x != 10
        || sky.settings.parallax_y != 10
        || par_mode != 0
        || sky.settings.back_color_raw != 0
        || back_enabled;
    if !nondefault {
        return None;
    }
    writer.section(0, "Sky");
    field_i32(&mut writer, 0, "X", fixed[0], 0);
    field_i32(&mut writer, 0, "Y", fixed[1], 0);
    field_i32(&mut writer, 0, "XDir", fixed[2], 0);
    field_i32(&mut writer, 0, "YDir", fixed[3], 0);
    field_u32(&mut writer, 0, "Modulation", modulation, 0x00ff_ffff);
    field_i32(&mut writer, 0, "ParX", sky.settings.parallax_x, 10);
    field_i32(&mut writer, 0, "ParY", sky.settings.parallax_y, 10);
    field_i32(&mut writer, 0, "ParMode", par_mode, 0);
    field_i32(
        &mut writer,
        0,
        "BackClr",
        sky.settings.back_color_raw as i32,
        0,
    );
    field_bool(&mut writer, 0, "BackClrEnabled", back_enabled, false);
    Some(writer.finish())
}

fn serialize_effect_chain(effects: &[EffectState], strings: &mut LegacyStringTable) -> String {
    effects
        .iter()
        .map(|effect| {
            let command_id = effect.command_id.as_deref().unwrap_or("NONE");
            let vars = effect
                .vars
                .iter()
                .map(|value| encode_effect_value(value, strings))
                .collect::<Vec<_>>()
                .join(",");
            let mut encoded = format!(
                "{}({},{},{},{},{},{})",
                effect.name,
                effect.number,
                effect.priority,
                effect.timer,
                effect.interval,
                strings.legacy_object_number(effect.command_target),
                command_id,
            );
            if !effect.vars.is_empty() {
                encoded.push_str(&format!("[{};{}]", effect.vars.len(), vars));
            }
            encoded
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn serialize_effects(effects: &[EffectState], strings: &mut LegacyStringTable) -> Option<Vec<u8>> {
    if effects.is_empty() {
        return None;
    }
    let mut writer = TextComponentWriter::default();
    writer.section(0, "Effects");
    writer.field(0, "GlobalEffects", serialize_effect_chain(effects, strings));
    Some(writer.finish())
}

fn serialize_scoreboard(scoreboard: &ScoreboardState) -> Option<Vec<u8>> {
    if scoreboard.is_default() {
        return None;
    }
    let mut writer = TextComponentWriter::default();
    writer.section(0, "Scoreboard");
    field_i32(
        &mut writer,
        0,
        "Rows",
        i32::try_from(scoreboard.row_count()).unwrap_or(i32::MAX),
        0,
    );
    field_i32(
        &mut writer,
        0,
        "Cols",
        i32::try_from(scoreboard.column_count()).unwrap_or(i32::MAX),
        0,
    );
    field_i32(&mut writer, 0, "DlgShow", scoreboard.show_count(), 0);
    for row in 0..scoreboard.row_count() {
        for column in 0..scoreboard.column_count() {
            let Some(cell) = scoreboard.cell(row, column) else {
                continue;
            };
            writer.field(
                0,
                &format!("Cell{column}_{row}String"),
                quote_ini(cell.text().unwrap_or_default()),
            );
            writer.field(
                0,
                &format!("Cell{column}_{row}Value"),
                cell.value().to_string(),
            );
        }
    }
    Some(writer.finish())
}

fn encode_id_list(entries: impl IntoIterator<Item = (String, i32)>) -> String {
    entries
        .into_iter()
        .map(|(id, count)| format!("{id}={count}"))
        .collect::<Vec<_>>()
        .join(";")
}

fn encode_object_list(engine: &Engine, entries: &[ObjectId]) -> String {
    entries
        .iter()
        .filter(|id| {
            engine.find_object_index(**id).is_some_and(|index| {
                let object = &engine.objects[index];
                !object.destroyed && object.state.status != ObjectStatus::Deleted
            })
        })
        .map(|id| i32::try_from(id.as_u64()).unwrap_or(0).to_string())
        .collect::<Vec<_>>()
        .join(";")
}

fn encode_section_object_list(objects: &[Object], entries: &[ObjectId]) -> String {
    entries
        .iter()
        .filter(|id| {
            objects.iter().any(|object| {
                object.id == **id
                    && !object.destroyed
                    && object.state.status != ObjectStatus::Deleted
            })
        })
        .map(|id| i32::try_from(id.as_u64()).unwrap_or(0).to_string())
        .collect::<Vec<_>>()
        .join(";")
}

fn serialize_players(
    engine: &Engine,
    players: &[PlayerState],
    strings: &mut LegacyStringTable,
) -> Vec<u8> {
    let mut writer = TextComponentWriter::default();
    for player in players {
        writer.section(0, &format!("Player{}", player.player_info_id));
        field_i32(&mut writer, 0, "Status", player.exact_status_value(), 0);
        field_i32(&mut writer, 0, "AtClient", player.at_client.get(), -1);
        let at_client_name = player.at_client_name.as_deref().unwrap_or("Local");
        if at_client_name != "Local" {
            writer.field(0, "AtClientName", at_client_name);
        }
        field_i32(&mut writer, 0, "Index", player.id, -1);
        field_i32(&mut writer, 0, "ID", player.player_info_id, 0);
        field_i32(
            &mut writer,
            0,
            "Eliminated",
            player.exact_eliminated_value(),
            0,
        );
        field_i32(
            &mut writer,
            0,
            "Surrendered",
            player.exact_surrendered_value(),
            0,
        );
        field_bool(&mut writer, 0, "Evaluated", player.evaluated, false);
        field_i32(
            &mut writer,
            0,
            "Color",
            player.color_index.unwrap_or(-1),
            -1,
        );
        let color = player.exact_color_dw();
        field_u32(&mut writer, 0, "ColorDw", color, 0);
        field_i32(&mut writer, 0, "Control", player.control_set, 0);
        field_i32(&mut writer, 0, "MouseControl", player.mouse_control, 0);
        field_i32(
            &mut writer,
            0,
            "AutoContextMenu",
            player.control.exact_auto_context_menu_value(),
            0,
        );
        field_i32(
            &mut writer,
            0,
            "AutoStopControl",
            player.control.exact_control_style_value(),
            0,
        );
        field_i32(
            &mut writer,
            0,
            "Position",
            player.position_index.unwrap_or(-1),
            0,
        );
        field_i32(&mut writer, 0, "ViewMode", player.view_mode, 0);
        let view = player.exact_view_center();
        field_i32(&mut writer, 0, "ViewX", view.x, 0);
        field_i32(&mut writer, 0, "ViewY", view.y, 0);
        field_i32(&mut writer, 0, "ViewWealth", player.view_wealth, 0);
        field_i32(&mut writer, 0, "ViewValue", player.view_value, 0);
        field_bool(&mut writer, 0, "FogOfWar", player.fog_of_war, false);
        field_bool(
            &mut writer,
            0,
            "ForceFogOfWar",
            player.force_fog_of_war,
            false,
        );
        field_bool(&mut writer, 0, "ShowStartup", player.show_startup, false);
        field_i32(&mut writer, 0, "ShowControl", player.show_control, 0);
        field_i32(
            &mut writer,
            0,
            "ShowControlPos",
            player.show_control_position,
            0,
        );
        field_i32(&mut writer, 0, "Wealth", player.wealth, 0);
        field_i32(&mut writer, 0, "Points", player.points, 0);
        field_i32(&mut writer, 0, "Value", player.value, 0);
        field_i32(&mut writer, 0, "InitialValue", player.initial_value, 0);
        field_i32(&mut writer, 0, "ValueGain", player.value_gain, 0);
        field_i32(
            &mut writer,
            0,
            "ObjectsOwned",
            player.objects_owned as i32,
            0,
        );
        let hostility = player.exact_hostility_entries();
        if !hostility.is_empty() {
            writer.field(
                0,
                "Hostile",
                encode_id_list(
                    hostility
                        .into_iter()
                        .map(|(id, count)| (id.to_string(), count)),
                ),
            );
        }
        field_i32(
            &mut writer,
            0,
            "ProductionDelay",
            player.production_delay as i32,
            0,
        );
        field_i32(
            &mut writer,
            0,
            "ProductionUnit",
            player.production_unit as i32,
            0,
        );
        field_i32(&mut writer, 0, "SelectCount", player.select_count, 0);
        field_i32(
            &mut writer,
            0,
            "SelectFlash",
            player.control.select_flash,
            0,
        );
        field_i32(
            &mut writer,
            0,
            "CursorFlash",
            player.control.cursor_flash,
            0,
        );
        field_object(
            &mut writer,
            0,
            "Cursor",
            strings.object_id_number(player.cursor),
        );
        field_object(
            &mut writer,
            0,
            "ViewCursor",
            strings.object_id_number(player.view_cursor),
        );
        field_object(
            &mut writer,
            0,
            "Captain",
            strings.object_id_number(player.captain),
        );
        field_i32(&mut writer, 0, "LastCom", player.control.last_com, 0);
        field_i32(
            &mut writer,
            0,
            "LastComDel",
            player.control.last_com_delay,
            0,
        );
        field_i32(
            &mut writer,
            0,
            "PressedComs",
            player.control.pressed_coms,
            0,
        );
        field_i32(
            &mut writer,
            0,
            "LastComDownDouble",
            player.control.last_com_down_double,
            0,
        );
        field_i32(
            &mut writer,
            0,
            "CursorSelection",
            player.control.cursor_selection,
            0,
        );
        field_i32(
            &mut writer,
            0,
            "CursorToggled",
            player.control.cursor_toggled,
            0,
        );
        field_i32(&mut writer, 0, "MessageStatus", player.message_status, 0);
        if !player.message_buf.is_empty() {
            writer.field(0, "MessageBuf", &player.message_buf);
        }
        for (name, entries) in [
            (
                "HomeBaseMaterial",
                player.exact_home_base_material_entries(),
            ),
            (
                "HomeBaseProduction",
                player.exact_home_base_production_entries(),
            ),
            ("Knowledge", player.exact_knowledge_entries()),
            ("Magic", player.exact_magic_entries()),
        ] {
            if !entries.is_empty() {
                writer.field(0, name, encode_id_list(entries));
            }
        }
        if !player.crew.is_empty() {
            let crew = encode_object_list(engine, &player.crew);
            if !crew.is_empty() {
                writer.field(0, "Crew", crew);
            }
        }
        field_i32(&mut writer, 0, "CrewCreated", player.crew_created, 0);
        if let Some(query) = player.message_board_queries.first() {
            writer.field(
                0,
                "MsgBoardQueries",
                format!(
                    "({},{},{})",
                    strings.object_id_number(query.target),
                    quote_ini(&query.prompt),
                    i32::from(query.uppercase)
                ),
            );
        }
    }
    writer.finish()
}

fn serialize_objects(engine: &Engine, strings: &mut LegacyStringTable) -> Vec<u8> {
    serialize_objects_for_save(engine, strings, false)
}

fn serialize_objects_for_save(
    engine: &Engine,
    strings: &mut LegacyStringTable,
    skip_user_player_objects: bool,
) -> Vec<u8> {
    fn serialize_list(
        engine: &Engine,
        ids: impl IntoIterator<Item = ObjectId>,
        strings: &mut LegacyStringTable,
        skip_user_player_objects: bool,
    ) -> Vec<u8> {
        let mut writer = TextComponentWriter::default();
        for id in ids {
            let Some(index) = engine.find_object_index(id) else {
                continue;
            };
            let object = &engine.objects[index];
            if object.destroyed || object.state.status == ObjectStatus::Deleted {
                continue;
            }
            if skip_user_player_objects
                && engine
                    .players
                    .get(&object.state.owner)
                    .is_some_and(|player| {
                        !player.is_script_player()
                            && (object.definition_id.as_str() == "FLAG"
                                || player.crew().contains(&object.id))
                    })
            {
                continue;
            }
            serialize_object(
                &mut writer,
                engine,
                object,
                engine.effective_object_mass(index),
                strings,
                None,
            );
        }
        writer.finish()
    }

    // C4ObjectList decompiles Last -> Prev. `exec_list` is already that
    // reversed master-list view (index zero executes/saves first).
    let mut output = serialize_list(
        engine,
        engine.exec_list.iter().copied(),
        strings,
        skip_user_player_objects,
    );
    // C4GameObjects::Save always decompiles the inactive list for runtime
    // saves and inserts this separator even when that list is empty.
    output.extend_from_slice(b"\r\n");
    output.extend_from_slice(&serialize_list(
        engine,
        engine.inactive_exec_list.iter().copied(),
        strings,
        skip_user_player_objects,
    ));
    output
}

fn serialize_object(
    writer: &mut TextComponentWriter,
    engine: &Engine,
    object: &Object,
    mass: i32,
    strings: &mut LegacyStringTable,
    section_objects: Option<&[Object]>,
) {
    let state = &object.state;
    writer.section(0, "Object");
    writer.field(0, "id", &object.definition_id);
    if let Some(name) = state.custom_name.as_deref().filter(|name| !name.is_empty()) {
        writer.field(0, "Name", quote_ini(name));
    }
    writer.field(0, "Number", object.id.to_string());
    if state.status != ObjectStatus::Normal {
        writer.field(0, "Status", state.status.to_script_value().to_string());
    }
    if !object.compiler_cache.info.is_empty() {
        serialize_object_info_name(writer, &object.compiler_cache.info);
    }
    field_i32(writer, 0, "Owner", state.owner, -1);
    field_i32(writer, 0, "Timer", state.timer, 0);
    field_i32(writer, 0, "Controller", state.controller, -1);
    field_i32(
        writer,
        0,
        "LastEngLossPlr",
        object.last_energy_loss_cause,
        -1,
    );
    field_i32(writer, 0, "Category", state.category, 0);
    field_i32(writer, 0, "X", state.position.x, 0);
    field_i32(writer, 0, "Y", state.position.y, 0);
    field_i32(writer, 0, "Rotation", state.rotation, 0);
    field_i32(writer, 0, "MotionX", object.motion_x, 0);
    field_i32(writer, 0, "MotionY", object.motion_y, 0);
    field_i32(
        writer,
        0,
        "LastSolidAtchFrame",
        object.last_attach_movement_frame,
        -1,
    );
    field_i32(writer, 0, "NoCollectDelay", state.no_collect_delay, 0);
    field_i32(writer, 0, "Base", state.base, -1);
    field_i32(writer, 0, "Size", state.construction, 0);
    field_i32(writer, 0, "OwnMass", state.own_mass, 0);
    field_i32(writer, 0, "Mass", mass, 0);
    field_i32(writer, 0, "Damage", state.damage, 0);
    field_i32(writer, 0, "Energy", state.energy, 0);
    field_i32(writer, 0, "MagicEnergy", state.magic_energy, 0);
    field_bool(writer, 0, "Alive", state.alive, false);
    field_i32(writer, 0, "Breath", state.breath, 0);
    field_i32(writer, 0, "FirePhase", state.fire_phase, 0);
    if state.color != 0 {
        writer.field(0, "Color", state.color.to_string());
        writer.field(0, "ColorDw", state.color.to_string());
    }

    let numbered_size = state
        .local_vars
        .keys()
        .filter_map(|name| name.strip_prefix("__local_")?.parse::<usize>().ok())
        .max()
        .map_or(0, |index| index.saturating_add(1));
    if numbered_size != 0 {
        let values = (0..numbered_size)
            .map(|index| {
                state
                    .local_vars
                    .get(&format!("__local_{index}"))
                    .map_or_else(|| "A0".to_owned(), |value| encode_value(value, strings))
            })
            .collect::<Vec<_>>();
        writer.field(0, "Locals", format!("{numbered_size};{}", values.join(",")));
    }

    field_fixed(writer, 0, "FixX", object.fixed_position.x.val());
    field_fixed(writer, 0, "FixY", object.fixed_position.y.val());
    field_fixed(writer, 0, "FixR", object.fixed_rotation.val());
    field_fixed(writer, 0, "XDir", object.fixed_velocity.x.val());
    field_fixed(writer, 0, "YDir", object.fixed_velocity.y.val());
    field_fixed(writer, 0, "RDir", object.rotation_velocity.val());

    if let Some(rect) = object.shape_rect {
        field_i32(writer, 0, "Width", rect.width, 0);
        field_i32(writer, 0, "Height", rect.height, 0);
        if rect.x != 0 || rect.y != 0 {
            let offset = if rect.y == 0 {
                rect.x.to_string()
            } else {
                format!("{},{}", rect.x, rect.y)
            };
            writer.field(0, "Offset", offset);
        }
    }
    field_i32(
        writer,
        0,
        "Vertices",
        i32::try_from(state.shape_vertices.active_count()).unwrap_or(30),
        0,
    );
    let slots = state.shape_vertices.slots();
    serialize_trimmed_shape_slots(writer, "VertexX", slots.iter().map(|vertex| vertex.x));
    serialize_trimmed_shape_slots(writer, "VertexY", slots.iter().map(|vertex| vertex.y));
    // C4Shape::VtxCNAT is an int32_t array. Runtime contact flags use the
    // same bits through u32, so cast back to the native signed spelling.
    serialize_trimmed_shape_slots(
        writer,
        "VertexCNAT",
        slots.iter().map(|vertex| vertex.cnat as i32),
    );
    serialize_trimmed_shape_slots(
        writer,
        "VertexFriction",
        slots.iter().map(|vertex| vertex.friction),
    );
    field_i32(writer, 0, "ContactDensity", state.contact_density, 50);
    field_i32(writer, 0, "FireTop", object.shape_fire_top, 0);
    field_i32(writer, 0, "AttachX", state.shape_attach.x, 0);
    field_i32(writer, 0, "AttachY", state.shape_attach.y, 0);
    field_i32(writer, 0, "AttachVtx", state.shape_attach.vtx, 0);
    field_bool(
        writer,
        0,
        "OwnVertices",
        object.own_shape_vertices.is_some(),
        false,
    );
    let definition_solid_mask = engine
        .definition(&object.definition_id)
        .and_then(crate::Definition::solid_mask);
    if let Some(mask) = state
        .solid_mask_override
        .filter(|mask| Some(*mask) != definition_solid_mask)
    {
        writer.field(
            0,
            "SolidMask",
            format!(
                "{},{},{},{},{},{}",
                mask.x, mask.y, mask.width, mask.height, mask.target_x, mask.target_y
            ),
        );
    }
    let picture = state.picture_rect;
    writer.field(
        0,
        "Picture",
        format!(
            "{},{},{},{}",
            picture.x, picture.y, picture.width, picture.height
        ),
    );
    field_bool(writer, 0, "Mobile", state.mobile, false);
    field_bool(writer, 0, "Selected", state.selected, false);
    field_bool(writer, 0, "OnFire", state.on_fire, false);
    field_bool(writer, 0, "InLiquid", state.in_liquid, false);
    field_bool(writer, 0, "EntranceStatus", state.entrance_status, false);
    field_bool(
        writer,
        0,
        "PhysicalTemporary",
        state.temporary_physical.is_some(),
        false,
    );
    field_bool(writer, 0, "NeedEnergy", state.need_energy, false);
    if state.ocf != 0 {
        writer.field(0, "OCF", state.ocf.to_string());
    }
    if !state.action.compiled_name().is_empty() {
        writer.field(0, "Action", state.action.compiled_name());
    }
    field_i32(writer, 0, "Dir", state.direction.to_script_value(), 0);
    field_i32(
        writer,
        0,
        "ComDir",
        state.command_direction.to_script_value(),
        0,
    );
    field_i32(writer, 0, "ActionTime", state.action.time, 0);
    field_i32(writer, 0, "ActionData", state.action.data, 0);
    field_i32(writer, 0, "Phase", state.action.phase, 0);
    field_i32(writer, 0, "PhaseDelay", state.action.ticks, 0);
    field_i32(writer, 0, "Contained", object.compiler_cache.contained, 0);
    field_i32(
        writer,
        0,
        "ActionTarget1",
        object.compiler_cache.action_target1,
        0,
    );
    field_i32(
        writer,
        0,
        "ActionTarget2",
        object.compiler_cache.action_target2,
        0,
    );
    if !state.component_order.is_empty() {
        writer.field(
            0,
            "Component",
            encode_id_list(
                state
                    .component_order
                    .iter()
                    .map(|id| (id.clone(), state.components.get(id).copied().unwrap_or(0))),
            ),
        );
    }
    if !state.contents.is_empty() {
        let contents = section_objects.map_or_else(
            || encode_object_list(engine, &state.contents),
            |objects| encode_section_object_list(objects, &state.contents),
        );
        if !contents.is_empty() {
            writer.field(0, "Contents", contents);
        }
    }
    field_i32(writer, 0, "PlrViewRange", state.plr_view_range, 0);
    field_i32(writer, 0, "Visibility", state.visibility, 0);
    let named_locals = engine
        .definition(&object.definition_id)
        .map(|definition| {
            definition
                .script
                .local_variable_names()
                .map(|name| {
                    let encoded = state
                        .local_vars
                        .get(name)
                        .map_or_else(|| "A0".to_owned(), |value| encode_value(value, strings));
                    (name.to_owned(), encoded)
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if !named_locals.is_empty() {
        writer.field(
            0,
            "LocalNamed",
            format!(
                "{};{}",
                named_locals.len(),
                named_locals
                    .into_iter()
                    .map(|(name, value)| format!("{name}={value}"))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        );
    } else {
        writer.field(0, "LocalNamed", "0");
    }
    if state.color_modulation != 0 {
        writer.field(0, "ColorMod", state.color_modulation.to_string());
    }
    if state.blit_mode != 0 {
        writer.field(0, "BlitMode", state.blit_mode.to_string());
    }
    field_bool(writer, 0, "CrewDisabled", state.crew_disabled, false);
    field_i32(writer, 0, "Layer", object.compiler_cache.layer, 0);
    if let Some(graphics) = state.base_graphics.as_ref().filter(|graphics| {
        graphics.definition != object.definition_id
            || graphics
                .graphics_name
                .as_deref()
                .is_some_and(|name| !name.is_empty())
    }) {
        writer.field(
            0,
            "Graphics",
            format!(
                "{}::{}",
                graphics.definition,
                graphics.graphics_name.as_deref().unwrap_or_default()
            ),
        );
    }
    if let Some(transform) = state.draw_transform {
        writer.field(0, "DrawTransform", serialize_draw_transform(transform));
    }
    if !state.effects.is_empty() {
        writer.field(
            0,
            "Effects",
            serialize_effect_chain(&state.effects, strings),
        );
    }
    if !state.graphics_overlays.is_empty() {
        writer.field(
            0,
            "GfxOverlay",
            state
                .graphics_overlays
                .iter()
                .map(|overlay| serialize_graphics_overlay(overlay, strings))
                .collect::<Vec<_>>()
                .join(";"),
        );
    }
    if let Some(physical) = state.temporary_physical.as_ref() {
        // C4Object::CompileFunc reaches this naming through FollowName, so it
        // is a same-level sibling of [Object], not a nested child section.
        // StdCompilerINIWrite also suppresses an entirely empty naming.
        if *physical != PhysicalInfo::default() || !state.physical_changes.is_empty() {
            writer.section(0, "Physical");
            serialize_physical(writer, 0, physical, &state.physical_changes);
        }
    }
    let commands = object.commands.legacy_save_commands();
    if !commands.is_empty() {
        writer.section(2, "Commands");
        for (command_index, command) in commands.iter().enumerate() {
            writer.field(
                2,
                &format!("Command{}", command_index + 1),
                serialize_command(command, strings),
            );
        }
    }
}

fn serialize_object_info_name(writer: &mut TextComponentWriter, name: &str) {
    writer.field(0, "Info", name);
}

fn serialize_trimmed_shape_slots(
    writer: &mut TextComponentWriter,
    name: &str,
    values: impl IntoIterator<Item = i32>,
) {
    let mut values = values.into_iter().collect::<Vec<_>>();
    while values.last() == Some(&0) {
        values.pop();
    }
    if !values.is_empty() {
        writer.field(
            0,
            name,
            values
                .into_iter()
                .map(|value| value.to_string())
                .collect::<Vec<_>>()
                .join(","),
        );
    }
}

fn field_i32(
    writer: &mut TextComponentWriter,
    indent: usize,
    name: &str,
    value: i32,
    default: i32,
) {
    if value != default {
        writer.field(indent, name, value.to_string());
    }
}

fn field_fixed(writer: &mut TextComponentWriter, indent: usize, name: &str, raw_value: i32) {
    if raw_value != 0 {
        writer.field(indent, name, format!("F{raw_value}"));
    }
}

fn field_u32(
    writer: &mut TextComponentWriter,
    indent: usize,
    name: &str,
    value: u32,
    default: u32,
) {
    if value != default {
        writer.field(indent, name, value.to_string());
    }
}

fn field_bool(
    writer: &mut TextComponentWriter,
    indent: usize,
    name: &str,
    value: bool,
    default: bool,
) {
    if value != default {
        writer.field(indent, name, if value { "true" } else { "false" });
    }
}

fn field_object(writer: &mut TextComponentWriter, indent: usize, name: &str, value: i32) {
    if value != 0 {
        writer.field(indent, name, value.to_string());
    }
}

fn serialize_graphics_overlay(
    overlay: &crate::ObjectGraphicsOverlay,
    strings: &LegacyStringTable,
) -> String {
    let graphics = overlay
        .definition
        .as_ref()
        .map_or_else(String::new, |definition| {
            format!(
                "{}::{}",
                definition,
                overlay.graphics_name.as_deref().unwrap_or_default()
            )
        });
    let transform = serialize_draw_transform(
        overlay
            .transform
            .unwrap_or_else(crate::DrawTransform::identity),
    );
    format!(
        "{},{},{},{},{},{},({}),{},{}",
        overlay.id,
        graphics,
        overlay.mode as i32,
        overlay.action.as_deref().unwrap_or_default(),
        overlay.blit_mode,
        overlay.phase,
        transform,
        overlay.color_modulation,
        strings.object_id_number(overlay.overlay_object)
    )
}

fn serialize_draw_transform(transform: crate::DrawTransform) -> String {
    let matrix = transform.matrix();
    let mut values = matrix[..6]
        .iter()
        .map(|value| format_legacy_float(*value))
        .collect::<Vec<_>>();
    values.push(transform.flip_dir().to_string());
    if matrix[6] != 0.0 || matrix[7] != 0.0 || matrix[8] != 1.0 {
        values.extend(matrix[6..].iter().map(|value| format_legacy_float(*value)));
    }
    values.join(",")
}

fn format_legacy_float(value: f32) -> String {
    if value.is_nan() {
        return "nan".to_owned();
    }
    if value == f32::INFINITY {
        return "inf".to_owned();
    }
    if value == f32::NEG_INFINITY {
        return "-inf".to_owned();
    }

    // fmt::sprintf("%g", float) uses six significant digits and selects
    // exponent notation after rounding when the exponent is below -4 or at
    // least the precision. Rust exposes fixed/scientific precision rather
    // than printf's significant-digit mode, so derive both from the rounded
    // six-digit scientific spelling.
    let scientific = format!("{value:.5e}");
    let (mantissa, exponent) = scientific
        .split_once('e')
        .expect("Rust scientific formatting always contains an exponent");
    let exponent = exponent
        .parse::<i32>()
        .expect("Rust scientific exponent is an integer");
    if !(-4..6).contains(&exponent) {
        let mantissa = trim_decimal_zeroes(mantissa);
        let sign = if exponent < 0 { '-' } else { '+' };
        return format!("{mantissa}e{sign}{:02}", exponent.unsigned_abs());
    }

    let fractional_digits = usize::try_from((5 - exponent).max(0)).unwrap_or(0);
    trim_decimal_zeroes(&format!("{value:.fractional_digits$}"))
}

fn trim_decimal_zeroes(value: &str) -> String {
    let Some(decimal) = value.find('.') else {
        return value.to_owned();
    };
    let mut end = value.len();
    while end > decimal + 1 && value.as_bytes()[end - 1] == b'0' {
        end -= 1;
    }
    if end == decimal + 1 {
        end = decimal;
    }
    value[..end].to_owned()
}

fn serialize_command(command: &LegacyCommandSave, strings: &mut LegacyStringTable) -> String {
    let tx = command.view.tx_value.as_ref().map_or_else(
        || {
            command.view.tx_definition.as_ref().map_or_else(
                || {
                    command
                        .view
                        .tx
                        .map_or_else(|| "A0".to_owned(), |value| format!("i{value}"))
                },
                |definition| format!("I{}", clonk_script::c4_id_raw(definition) as u32 as i32),
            )
        },
        |value| encode_value(value, strings),
    );
    let data = command
        .view
        .legacy_data
        .unwrap_or_else(|| match &command.view.data {
            CommandData::Integer(value) => *value,
            CommandData::Text(_) if command.view.name == "Call" => 0,
            CommandData::Text(value) => clonk_script::c4_id_raw(value) as u32 as i32,
            CommandData::None => 0,
        });
    format!(
        "$2,{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
        command.view.name,
        tx,
        command.view.ty.unwrap_or(0),
        strings.object_id_number(command.view.target),
        strings.object_id_number(command.view.target2),
        data,
        command.update_interval,
        command.evaluated,
        command.path_checked,
        command.finished,
        command.failures,
        command.retries,
        command.permit,
        command.base_mode,
        command.text,
    )
}

fn serialize_physical(
    writer: &mut TextComponentWriter,
    indent: usize,
    physical: &PhysicalInfo,
    changes: &[(String, i32)],
) {
    for (name, value) in [
        ("Energy", physical.energy),
        ("Breath", physical.breath),
        ("Walk", physical.walk),
        ("Jump", physical.jump),
        ("Scale", physical.scale),
        ("Hangle", physical.hangle),
        ("Dig", physical.dig),
        ("Swim", physical.swim),
        ("Throw", physical.throw),
        ("Push", physical.push),
        ("Fight", physical.fight),
        ("Magic", physical.magic),
        ("Float", physical.float),
        ("CanScale", physical.can_scale),
        ("CanHangle", physical.can_hangle),
        ("CanDig", physical.can_dig),
        ("CanConstruct", physical.can_construct),
        ("CanChop", physical.can_chop),
        ("CanFly", physical.can_fly),
        ("CorrosionResist", physical.corrosion_resist),
        ("BreatheWater", physical.breathe_water),
    ] {
        field_i32(writer, indent, name, value, 0);
    }
    if !changes.is_empty() {
        writer.field(
            indent,
            "Changes",
            changes
                .iter()
                .map(|(name, previous)| format!("{name}={previous}"))
                .collect::<Vec<_>>()
                .join(","),
        );
    }
}

enum SerializedScenarioSection {
    Frozen(LiveC4SaveNamedComponent),
    Rebuilt {
        name: String,
        group: MutableGroup,
        objects_txt: Option<Vec<u8>>,
    },
}

enum PendingScenarioSectionMutation {
    Delete { name: String, failed_add: bool },
    Replace(SerializedScenarioSection),
}

/// Materialize the same temporary group that C4Game::LoadScenarioSection
/// creates while leaving a section. The packed bytes are retained on the
/// section and copied unchanged by the later root save.
pub(super) fn freeze_scenario_section(
    engine: &Engine,
    section: &crate::RuntimeScenarioSection,
    write_landscape: bool,
    write_objects: bool,
) -> Result<Vec<u8>, LiveC4SaveError> {
    let group_name = format!("Sect{}.c4g", section.name);
    let mut group = if let Some(payload) = section.frozen_group.as_ref() {
        let source = Group::from_raw_memory(PathBuf::from(&group_name), payload.clone())?;
        MutableGroup::from_group(&source)?
    } else {
        extract_scenario_section_group(section, &group_name)?
    };

    if write_landscape {
        write_scenario_section_landscape(engine, section, &mut group)?;
    }
    if write_objects {
        group.remove_entry("Objects.txt");

        let state = engine.capture_state();
        let referenced_strings = collect_live_referenced_strings(engine, &state);
        // LoadScenarioSection calls Strings.Save before Objects.Save. The
        // latter starts by calling EnumStrings, so the section's Strings.txt
        // intentionally reflects the previous enumeration while Objects.txt
        // consumes the newly assigned IDs.
        let saved_string_values = clonk_script::save_current_c4_string_enumeration(
            &engine.script_string_registrations,
            &referenced_strings,
        );
        let saved_strings = LegacyStringTable::from_enumerated_values(saved_string_values);
        let mut strings =
            LegacyStringTable::from_enumerated_values(clonk_script::enumerate_c4_strings(
                &engine.script_string_registrations,
                &referenced_strings,
            ));
        strings.set_object_numbers(engine.live_object_numbers_for_save());
        let objects_txt = if let Some(objects) = section.saved_objects.as_deref() {
            serialize_persisted_objects(engine, objects, &section.saved_object_order, &mut strings)
        } else {
            serialize_initial_section_objects(engine, &section.initial_objects, &mut strings)
        };
        if let Some(payload) = saved_strings.encoded() {
            group.remove_entry("Strings.txt");
            group.add_file("Strings.txt", payload)?;
        }
        group.add_file("Objects.txt", objects_txt)?;
    }

    Ok(group.pack_raw()?)
}

fn extract_scenario_section_group(
    section: &crate::RuntimeScenarioSection,
    group_name: &str,
) -> Result<MutableGroup, LiveC4SaveError> {
    let Some(source) = section.source_group.as_ref() else {
        return Ok(MutableGroup::new(group_name));
    };
    if !section.source_is_scenario_root {
        return Ok(MutableGroup::from_group(source)?);
    }

    // The implicit C4ScenarioSection has an empty Filename regardless of its
    // visible name. EnsureTempStore does not rewrite the packed root scenario
    // into its section child: it creates a fresh temp group and extracts only
    // C4FLS_Section entries into it. Named sections retain their full group,
    // even when one happens to be called Main.
    let mut extracted = MutableGroup::new(group_name);
    for entry in source.entries()? {
        if entry.is_directory || !is_main_section_component_bytes(&entry.name_bytes) {
            continue;
        }
        let payload = source.read_entry_bytes_exact(&entry)?;
        extracted.add_file_bytes_with_metadata(
            entry.name_bytes,
            payload,
            entry.time,
            entry.executable,
        )?;
    }
    Ok(extracted)
}

fn write_scenario_section_landscape(
    engine: &Engine,
    section: &crate::RuntimeScenarioSection,
    group: &mut MutableGroup,
) -> Result<(), LiveC4SaveError> {
    remove_section_landscape_components(group);
    let has_exact_landscape = section.landscape.is_some();
    group.add_file(
        "Scenario.txt",
        serialize_section_scenario(&section.scenario_values, has_exact_landscape),
    )?;
    if let Some(landscape) = section.landscape.as_ref() {
        // C4Landscape::Save reaches C4Sky::Save only for an exact section
        // landscape. C4CFN_Sky is the literal extensionless entry `Sky`;
        // image entries such as Sky.png are deliberately preserved.
        if section.scenario_values.no_sky() {
            group.remove_entry("Sky");
        }
        let grid = landscape
            .pixel_grid()
            .ok_or(LiveC4SaveError::MissingPixelGrid)?;
        let bitmap = IndexedBitmap {
            width: grid.width(),
            height: grid.height(),
            indices: grid.bytes().to_vec(),
        };
        let mut palette = [[0_u8; 3]; 256];
        for (index, slot) in palette.iter_mut().enumerate() {
            *slot = landscape.surface8_palette_entry(index as u8).0;
        }
        group.add_file("Landscape.bmp", bitmap.encode_with_palette(&palette)?)?;
        group.add_file("Landscape.png", encode_landscape_png(landscape)?)?;
        landscape.save_changed_c4_map(engine.materials(), group)?;
        landscape.save_c4_textures(group)?;
    }
    if let Some(payload) = section
        .landscape_systems
        .pxs
        .as_ref()
        .and_then(crate::pxs::PxsSystem::to_c4b)
    {
        group.add_file("PXS.c4b", payload)?;
    }
    if let Some(payload) = section
        .landscape_systems
        .mass_movers
        .as_ref()
        .and_then(crate::mass_mover::MassMoverSet::to_c4b)
    {
        group.add_file("MassMover.c4b", payload)?;
    }
    Ok(())
}

fn serialize_scenario_sections(
    engine: &Engine,
    strings: &mut LegacyStringTable,
) -> (
    Vec<LiveC4SaveNamedComponent>,
    Vec<String>,
    Vec<LiveC4SaveScenarioSectionMutation>,
) {
    let current = engine.current_scenario_section.to_ascii_lowercase();
    let mut pending = Vec::with_capacity(engine.scenario_section_order.len());
    for key in &engine.scenario_section_order {
        let Some(section) = engine.scenario_sections.get(key) else {
            continue;
        };
        let group_name = format!("Sect{}.c4g", section.name);
        if engine.scenario_current_section_registered && *key == current {
            pending.push(PendingScenarioSectionMutation::Delete {
                name: group_name,
                failed_add: false,
            });
            continue;
        }
        if !section.modified {
            continue;
        }
        if let Some(payload) = section.frozen_group.as_ref() {
            pending.push(PendingScenarioSectionMutation::Replace(
                SerializedScenarioSection::Frozen(LiveC4SaveNamedComponent {
                    name: group_name,
                    payload: payload.clone(),
                }),
            ));
            continue;
        }
        let rebuilt = (|| -> Result<SerializedScenarioSection, LiveC4SaveError> {
            // The implicit root uses a fresh C4FLS_Section extraction; named
            // subsections retain their complete source group outside replaced
            // categories, independently of the visible section name.
            let mut group = extract_scenario_section_group(section, &group_name)?;

            // A generated section has no source group to extract, so synthesize
            // both categories. Loaded sections replace exactly the categories
            // requested by the changing section switch and retain the other one.
            let write_landscape = section.landscape_modified || section.source_group.is_none();
            let write_objects = section.objects_modified || section.source_group.is_none();

            if write_landscape {
                write_scenario_section_landscape(engine, section, &mut group)?;
            }

            let objects_txt = write_objects.then(|| {
                if let Some(objects) = section.saved_objects.as_deref() {
                    serialize_persisted_objects(
                        engine,
                        objects,
                        &section.saved_object_order,
                        strings,
                    )
                } else {
                    serialize_initial_section_objects(engine, &section.initial_objects, strings)
                }
            });
            if write_objects {
                group.remove_entry("Strings.txt");
                group.remove_entry("Objects.txt");
            }
            Ok(SerializedScenarioSection::Rebuilt {
                name: group_name.clone(),
                group,
                objects_txt,
            })
        })();
        match rebuilt {
            Ok(section) => pending.push(PendingScenarioSectionMutation::Replace(section)),
            Err(error) => {
                // SaveScenarioSections deletes the target then ignores Add's
                // return value. A broken temp section is therefore nonfatal.
                tracing::warn!(%error, section = %group_name, "failed to rebuild modified save section");
                pending.push(PendingScenarioSectionMutation::Delete {
                    name: group_name,
                    failed_add: true,
                });
            }
        }
    }

    // C4ScenarioSection::Save passes the one game-global string table to
    // every section object save. Encode it only after all section objects
    // have enumerated their values so every emitted section sees the same
    // final ID mapping.
    let strings_txt = strings.encoded();
    let mut output = Vec::new();
    let mut deleted = Vec::new();
    let mut mutations = Vec::with_capacity(pending.len());
    for mutation in pending {
        let section = match mutation {
            PendingScenarioSectionMutation::Delete { name, failed_add } => {
                if failed_add {
                    deleted.push(name.clone());
                }
                mutations.push(LiveC4SaveScenarioSectionMutation::Delete { name });
                continue;
            }
            PendingScenarioSectionMutation::Replace(section) => section,
        };
        match section {
            SerializedScenarioSection::Frozen(section) => output.push(section),
            SerializedScenarioSection::Rebuilt {
                name,
                mut group,
                objects_txt,
            } => {
                let packed = (|| -> Result<Vec<u8>, LiveC4SaveError> {
                    if let Some(objects_txt) = objects_txt {
                        if let Some(payload) = strings_txt.clone() {
                            group.add_file("Strings.txt", payload)?;
                        }
                        group.add_file("Objects.txt", objects_txt)?;
                    }
                    Ok(group.pack_raw()?)
                })();
                match packed {
                    Ok(payload) => output.push(LiveC4SaveNamedComponent { name, payload }),
                    Err(error) => {
                        tracing::warn!(%error, section = %name, "failed to pack modified save section");
                        deleted.push(name.clone());
                        mutations.push(LiveC4SaveScenarioSectionMutation::Delete { name });
                        continue;
                    }
                }
            }
        }
        let section = output
            .last()
            .expect("a successful section serialization appends one component")
            .clone();
        mutations.push(LiveC4SaveScenarioSectionMutation::Replace(section));
    }
    (output, deleted, mutations)
}

fn is_main_section_component_bytes(name: &[u8]) -> bool {
    [
        "Scenario.txt",
        "Game.txt",
        "Landscape.bmp",
        "Landscape.png",
        "Sky.bmp",
        "Sky.png",
        "Sky.jpeg",
        "Sky.jpg",
        "PXS.c4b",
        "MassMover.c4b",
        "CtrlRec.c4b",
        "Strings.txt",
        "Objects.txt",
    ]
    .iter()
    .any(|candidate| name.eq_ignore_ascii_case(candidate.as_bytes()))
}

fn remove_section_landscape_components(group: &mut MutableGroup) {
    for name in [
        "Scenario.txt",
        "Landscape.bmp",
        "Landscape.png",
        "PXS.c4b",
        "MassMover.c4b",
    ] {
        group.remove_entry(name);
    }
}

fn serialize_section_scenario(values: &ScenarioValueStore, force_exact: bool) -> Vec<u8> {
    values.serialize_section_save(force_exact)
}

fn serialize_persisted_objects(
    engine: &Engine,
    persisted: &[crate::PersistedObject],
    saved_order: &[ObjectId],
    strings: &mut LegacyStringTable,
) -> Vec<u8> {
    // LoadScenarioSection captures these snapshots only after
    // C4GameObjects::Enumerate/Denumerate has visited the complete active and
    // inactive master lists. Their compiler words therefore already contain
    // the native object numbers. Re-enumerating only the emitted active rows
    // would incorrectly clear references to inactive or preserved objects.
    let objects = persisted
        .iter()
        .filter_map(|object| restored_section_object(engine, object))
        .collect::<Vec<_>>();
    let by_id = objects
        .iter()
        .enumerate()
        .map(|(index, object)| (object.id, index))
        .collect::<HashMap<_, _>>();
    let snapshots = persisted
        .iter()
        .map(|object| (object.snapshot.id, &object.snapshot))
        .collect::<HashMap<_, _>>();
    let mut emitted = std::collections::HashSet::new();
    let mut writer = TextComponentWriter::default();
    for id in saved_order
        .iter()
        .chain(objects.iter().map(|object| &object.id))
    {
        if !emitted.insert(*id) {
            continue;
        }
        let Some(&index) = by_id.get(id) else {
            continue;
        };
        let object = &objects[index];
        if object.destroyed || object.state.status != ObjectStatus::Normal {
            continue;
        }
        let Some(snapshot) = snapshots.get(id).copied() else {
            continue;
        };
        if engine.is_user_player_object_snapshot(snapshot) {
            continue;
        }
        let mass = section_object_mass(engine, &objects, index, &mut HashSet::new());
        serialize_object(&mut writer, engine, object, mass, strings, Some(&objects));
    }
    writer.finish()
}

fn restored_section_object(engine: &Engine, persisted: &crate::PersistedObject) -> Option<Object> {
    let snapshot = &persisted.snapshot;
    let definition = engine.definitions.get(&snapshot.definition_id)?;
    let shape_template = crate::ObjectShapeTemplate::new(
        definition.shape_vertices().to_vec(),
        definition.shape_rect(),
        definition.fire_top(),
        definition.stretch_growth(),
        definition.rotateable(),
    )
    .with_line(definition.line());
    let mut object = Object::new(
        snapshot.id,
        snapshot.definition_id.clone(),
        crate::ObjectState {
            custom_name: snapshot.custom_name.clone(),
            position: snapshot.position,
            velocity: snapshot.velocity,
            script_fixed_position: None,
            script_fixed_velocity: None,
            script_rotation_velocity: snapshot.rotation_velocity,
            script_fixed_rotation: snapshot.fixed_rotation,
            rotation: snapshot.rotation,
            energy: snapshot.energy,
            need_energy: snapshot.need_energy,
            damage: snapshot.damage,
            magic_energy: snapshot.magic_energy,
            magic_capacity: snapshot.magic_capacity,
            construction: snapshot.construction,
            action: snapshot.action.clone(),
            direction: snapshot.direction,
            command_direction: snapshot.command_direction,
            effects: snapshot.effects.clone(),
            vertices: snapshot.vertices.clone(),
            shape_vertices: persisted
                .shape_vertices
                .clone()
                .unwrap_or_else(|| crate::ShapeVertexBuffer::from_active(&snapshot.vertices)),
            contact_density: snapshot.contact_density,
            container: snapshot.container,
            layer: snapshot.layer,
            visibility: snapshot.visibility,
            blit_mode: snapshot.blit_mode,
            contents: snapshot.contents.clone(),
            contents_link_generation: 0,
            components: snapshot.components.clone(),
            component_order: snapshot.component_order.clone(),
            status: snapshot.status,
            owner: snapshot.owner,
            controller: snapshot.controller,
            category: snapshot.category,
            crew_member: snapshot.crew_member,
            plr_view_range: snapshot.plr_view_range,
            selected: snapshot.selected,
            crew_disabled: persisted.crew_disabled,
            alive: snapshot.alive,
            base_graphics: snapshot.base_graphics.clone(),
            graphics_overlays: snapshot.graphics_overlays.clone(),
            draw_transform: snapshot.draw_transform,
            local_vars: snapshot.local_vars.clone(),
            in_liquid: snapshot.in_liquid,
            mobile: snapshot.mobile,
            solid_mask_override: persisted.solid_mask_override,
            timer: snapshot.timer,
            own_mass: snapshot.own_mass,
            on_fire: snapshot.on_fire,
            fire_phase: snapshot.fire_phase,
            fire_caused_by: snapshot.fire_caused_by,
            info_physical: snapshot.info_physical,
            temporary_physical: snapshot.temporary_physical,
            physical_changes: snapshot.physical_changes.clone(),
            breath: snapshot.breath,
            entrance_status: persisted.entrance_status,
            menu: None,
            color: snapshot.color,
            color_modulation: snapshot.color_modulation,
            picture_rect: snapshot.picture_rect,
            shape_override: snapshot.current_shape,
            ocf: snapshot.ocf,
            shape_attach: persisted.shape_attach,
            t_attach: 0,
            no_collect_delay: persisted.no_collect_delay,
            base: snapshot.base,
        },
        shape_template,
        snapshot.own_vertices.clone(),
    );
    object.compiled_mass = persisted.compiled_mass;
    object.compiled_mass_contents = snapshot.contents.clone();
    if let Some(rect) = snapshot.current_shape {
        object.shape_rect = Some(rect);
    }
    if let Some(fire_top) = snapshot.current_fire_top {
        object.shape_fire_top = fire_top;
    }
    object.motion_x = persisted.motion_x;
    object.motion_y = persisted.motion_y;
    object.compiler_cache = persisted.compiler_cache.clone();
    object.last_attach_movement_frame = persisted.last_attach_movement_frame;
    if let Some(value) = snapshot.fixed_position {
        object.fixed_position = value;
    }
    if let Some(value) = snapshot.fixed_velocity {
        object.fixed_velocity = value;
    }
    if let Some(value) = snapshot.fixed_rotation {
        object.fixed_rotation = value;
    }
    if let Some(value) = snapshot.rotation_velocity {
        object.rotation_velocity = value;
    }
    object.last_energy_loss_cause = snapshot.last_energy_loss_cause;
    object
        .commands
        .restore_from_snapshot(&persisted.command_stack);
    Some(object)
}

fn serialize_initial_section_objects(
    engine: &Engine,
    spawns: &[crate::scenario::ScenarioSpawn],
    strings: &mut LegacyStringTable,
) -> Vec<u8> {
    let mut objects = spawns
        .iter()
        .enumerate()
        .filter_map(|(index, spawn)| section_spawn_object(engine, &spawn.config, index))
        .collect::<Vec<_>>();
    let object_numbers = section_object_numbers(&objects);
    enumerate_section_object_caches(&mut objects, &object_numbers);
    let previous_object_numbers = strings.replace_object_numbers(object_numbers);
    let mut writer = TextComponentWriter::default();
    for (index, object) in objects.iter().enumerate().rev() {
        let mass = section_object_mass(engine, &objects, index, &mut HashSet::new());
        serialize_object(&mut writer, engine, object, mass, strings, Some(&objects));
    }
    let output = writer.finish();
    strings.object_numbers = previous_object_numbers;
    output
}

fn section_object_numbers(objects: &[Object]) -> HashMap<u64, i32> {
    objects
        .iter()
        .filter_map(|object| {
            i32::try_from(object.id.as_u64())
                .ok()
                .map(|number| (object.id.as_u64(), number))
        })
        .collect()
}

fn enumerate_section_object_caches(objects: &mut [Object], object_numbers: &HashMap<u64, i32>) {
    let object_number = |target: Option<ObjectId>| {
        target
            .and_then(|target| object_numbers.get(&target.as_u64()).copied())
            .unwrap_or(0)
    };
    for object in objects {
        if object.destroyed || object.state.status == ObjectStatus::Deleted {
            continue;
        }
        object.compiler_cache.contained = object_number(object.state.container);
        object.compiler_cache.action_target1 = object_number(object.state.action.target);
        object.compiler_cache.action_target2 = object_number(object.state.action.target2);
        object.compiler_cache.layer = object_number(object.state.layer);
    }
}

fn section_spawn_object(
    engine: &Engine,
    config: &crate::SpawnConfig,
    index: usize,
) -> Option<Object> {
    let definition = engine.definitions.get(&config.definition_id)?;
    let id = config
        .id
        .unwrap_or_else(|| ObjectId::new(u64::try_from(index).unwrap_or(0).saturating_add(1)));
    let shape_template = crate::ObjectShapeTemplate::new(
        definition.shape_vertices().to_vec(),
        definition.shape_rect(),
        definition.fire_top(),
        definition.stretch_growth(),
        definition.rotateable(),
    )
    .with_line(definition.line());
    let vertices = config
        .shape_vertices
        .as_ref()
        .map(crate::ShapeVertexBuffer::active_vec)
        .unwrap_or_else(|| config.vertices.clone());
    let shape_vertices = config
        .shape_vertices
        .clone()
        .unwrap_or_else(|| crate::ShapeVertexBuffer::from_active(&vertices));
    let components = config.components.clone().unwrap_or_default();
    let component_order = config.component_order.clone().unwrap_or_default();
    let mut object = Object::new(
        id,
        config.definition_id.clone(),
        crate::ObjectState {
            custom_name: config.custom_name.clone(),
            position: config.position,
            velocity: config.velocity,
            script_fixed_position: None,
            script_fixed_velocity: None,
            script_rotation_velocity: config.rotation_velocity,
            script_fixed_rotation: config.fixed_rotation,
            rotation: config.rotation,
            energy: config.energy.unwrap_or(0),
            need_energy: config.need_energy.unwrap_or(false),
            damage: 0,
            magic_energy: config.magic_energy.unwrap_or(0),
            magic_capacity: 0,
            construction: config.construction,
            action: config
                .action
                .clone()
                .unwrap_or_else(|| crate::ActionState::new(String::new())),
            direction: config.direction,
            command_direction: config.command_direction,
            effects: config.effects.clone(),
            vertices,
            shape_vertices,
            contact_density: config.contact_density.unwrap_or(50),
            container: config.container,
            layer: config.layer,
            visibility: config.visibility.unwrap_or(0),
            blit_mode: config.blit_mode.unwrap_or(0),
            contents: Vec::new(),
            contents_link_generation: 0,
            components,
            component_order,
            status: config.status.unwrap_or_default(),
            owner: config.owner,
            controller: config.controller.unwrap_or(-1),
            category: config.category.unwrap_or(0),
            crew_member: config.crew_member.unwrap_or(false),
            plr_view_range: config.plr_view_range.unwrap_or(0),
            selected: config.selected.unwrap_or(false),
            crew_disabled: false,
            alive: config.alive.unwrap_or(false),
            base_graphics: None,
            graphics_overlays: Vec::new(),
            draw_transform: None,
            local_vars: config.local_vars.clone(),
            in_liquid: config.in_liquid.unwrap_or(false),
            mobile: config.mobile.unwrap_or(false),
            solid_mask_override: config.solid_mask,
            timer: config.timer.unwrap_or(0),
            own_mass: 0,
            on_fire: false,
            fire_phase: 0,
            fire_caused_by: -1,
            info_physical: None,
            temporary_physical: None,
            physical_changes: Vec::new(),
            breath: 0,
            entrance_status: config.entrance_status.unwrap_or(false),
            menu: None,
            color: config.color.unwrap_or(0),
            color_modulation: config.color_modulation.unwrap_or(0),
            picture_rect: config.picture_rect.unwrap_or_default(),
            shape_override: config.shape_rect,
            ocf: 0,
            shape_attach: crate::ShapeAttachRecord::default(),
            t_attach: 0,
            no_collect_delay: 0,
            base: -1,
        },
        shape_template,
        config
            .owns_shape_vertices
            .unwrap_or(false)
            .then(|| config.vertices.clone()),
    );
    if config.loaded {
        object.compiled_mass = Some(config.compiled_mass.unwrap_or(0));
        object.compiled_mass_contents = object.state.contents.clone();
    }
    object.motion_x = config.motion_x;
    object.motion_y = config.motion_y;
    object.compiler_cache = config.compiler_cache.clone();
    object.last_attach_movement_frame = config.last_attach_movement_frame.unwrap_or(-1);
    if let Some(rect) = config.shape_rect {
        object.shape_rect = Some(rect);
    }
    if let Some(fire_top) = config.shape_fire_top {
        object.shape_fire_top = fire_top;
    }
    if let Some(value) = config.fixed_position {
        object.fixed_position = value;
    }
    if let Some(value) = config.fixed_velocity {
        object.fixed_velocity = value;
    }
    if let Some(value) = config.fixed_rotation {
        object.fixed_rotation = value;
    }
    if let Some(value) = config.rotation_velocity {
        object.rotation_velocity = value;
    }
    Some(object)
}

fn section_object_mass(
    engine: &Engine,
    objects: &[Object],
    index: usize,
    visiting: &mut HashSet<ObjectId>,
) -> i32 {
    let object = &objects[index];
    if let Some(mass) = object.compiled_mass {
        return mass;
    }
    if !visiting.insert(object.id) {
        return 1;
    }
    let (definition_mass, no_component_mass) = engine
        .definitions
        .get(&object.definition_id)
        .map(|definition| (definition.mass(), definition.no_component_mass()))
        .unwrap_or((0, false));
    let mut mass = ((definition_mass + object.state.own_mass)
        .saturating_mul(object.state.construction)
        / crate::FULL_CON)
        .max(1);
    if !no_component_mass {
        for content in &object.state.contents {
            if let Some(content_index) = objects.iter().position(|object| object.id == *content) {
                mass += section_object_mass(engine, objects, content_index, visiting);
            }
        }
    }
    visiting.remove(&object.id);
    mass
}

struct SerializedLandscape {
    landscape_bmp: Option<Vec<u8>>,
    landscape_png: Option<Vec<u8>>,
    diff_landscape_bmp: Option<Vec<u8>>,
    map_bmp: Option<Vec<u8>>,
    material_group: Option<Vec<u8>>,
    mat_map_txt: Vec<u8>,
    pxs_c4b: Option<Vec<u8>>,
    mass_mover_c4b: Option<Vec<u8>>,
    saves_auxiliary_systems: bool,
    delete_sky_entry: bool,
}

#[derive(Debug)]
struct SerializedLandscapeFailure {
    source: LiveC4SaveError,
    mutations: Vec<LiveC4SaveLandscapeMutation>,
}

fn serialize_landscape_for_policy(
    engine: &Engine,
    policy: LiveC4SavePolicy<'_>,
    copied_material_group_is_file: bool,
) -> Result<SerializedLandscape, SerializedLandscapeFailure> {
    let Some(landscape) = engine.landscape_without_solid_masks() else {
        if matches!(
            policy,
            LiveC4SavePolicy::Scenario {
                force_exact_landscape: false
            }
        ) {
            return Ok(SerializedLandscape {
                landscape_bmp: None,
                landscape_png: None,
                diff_landscape_bmp: None,
                map_bmp: None,
                material_group: None,
                mat_map_txt: Vec::new(),
                pxs_c4b: None,
                mass_mover_c4b: None,
                saves_auxiliary_systems: false,
                delete_sky_entry: false,
            });
        }
        return Err(SerializedLandscapeFailure {
            source: LiveC4SaveError::MissingLandscape,
            mutations: Vec::new(),
        });
    };
    let mut scratch = MutableGroup::new("Runtime.c4s");
    if copied_material_group_is_file {
        // The application owns the copied scenario group. Mirror only this
        // destination-shape hazard so SaveTextures observes it at the same
        // point as C++; remove the sentinel again after a clean no-op.
        scratch
            .add_file("Material.c4g", Vec::new())
            .map_err(|source| SerializedLandscapeFailure {
                source: source.into(),
                mutations: Vec::new(),
            })?;
    }
    let saves_auxiliary_systems =
        landscape.mode() == LANDSCAPE_MODE_EXACT || policy.forces_runtime_landscape();
    let delete_sky_entry =
        landscape.mode() == LANDSCAPE_MODE_EXACT && engine.scenario_values.no_sky();

    let mut mutations = Vec::new();
    if landscape.mode() == LANDSCAPE_MODE_EXACT {
        if delete_sky_entry {
            mutations.push(LiveC4SaveLandscapeMutation::DeleteEntry {
                name: "Sky".to_owned(),
            });
        }
        let grid = landscape
            .pixel_grid()
            .ok_or_else(|| SerializedLandscapeFailure {
                source: LiveC4SaveError::MissingPixelGrid,
                mutations: mutations.clone(),
            })?;
        let bitmap = IndexedBitmap {
            width: grid.width(),
            height: grid.height(),
            indices: grid.bytes().to_vec(),
        };
        let mut palette = [[0_u8; 3]; 256];
        for (index, slot) in palette.iter_mut().enumerate() {
            *slot = landscape.surface8_palette_entry(index as u8).0;
        }
        let landscape_bmp =
            bitmap
                .encode_with_palette(&palette)
                .map_err(|source| SerializedLandscapeFailure {
                    source: source.into(),
                    mutations: mutations.clone(),
                })?;
        scratch
            .add_file("Landscape.bmp", landscape_bmp.clone())
            .map_err(|source| SerializedLandscapeFailure {
                source: source.into(),
                mutations: mutations.clone(),
            })?;
        mutations.push(LiveC4SaveLandscapeMutation::PutFile {
            name: "Landscape.bmp".to_owned(),
            payload: landscape_bmp,
        });

        let landscape_png =
            encode_landscape_png(&landscape).map_err(|source| SerializedLandscapeFailure {
                source: source.into(),
                mutations: mutations.clone(),
            })?;
        scratch
            .add_file("Landscape.png", landscape_png.clone())
            .map_err(|source| SerializedLandscapeFailure {
                source: source.into(),
                mutations: mutations.clone(),
            })?;
        mutations.push(LiveC4SaveLandscapeMutation::PutFile {
            name: "Landscape.png".to_owned(),
            payload: landscape_png,
        });
        match engine.save_changed_c4_landscape_map(&mut scratch) {
            Ok(true) => append_scratch_file_mutation(&scratch, "Map.bmp", &mut mutations),
            Ok(false) => {}
            Err(source) => {
                return Err(SerializedLandscapeFailure {
                    source: source.into(),
                    mutations,
                });
            }
        }
        match engine.save_c4_landscape_textures(&mut scratch) {
            Ok(true) => append_scratch_material_mutation(&scratch, &mut mutations),
            Ok(false) => {}
            Err(source) => {
                return Err(SerializedLandscapeFailure {
                    source: source.into(),
                    mutations,
                });
            }
        }
    } else if policy.forces_runtime_landscape() {
        // Exact savegames and forced scenario saves use a full sync diff;
        // SyncSynchronized runtime-network saves use the 0xff-masked diff.
        if let Err(source) =
            engine.save_c4_landscape_diff(&mut scratch, policy.landscape_diff_is_sync_save())
        {
            append_scratch_file_mutation(&scratch, "DiffLandscape.bmp", &mut mutations);
            append_scratch_file_mutation(&scratch, "Map.bmp", &mut mutations);
            append_scratch_material_mutation(&scratch, &mut mutations);
            return Err(SerializedLandscapeFailure {
                source: source.into(),
                mutations,
            });
        }
        append_scratch_file_mutation(&scratch, "DiffLandscape.bmp", &mut mutations);
        append_scratch_file_mutation(&scratch, "Map.bmp", &mut mutations);
        append_scratch_material_mutation(&scratch, &mut mutations);
    } else if landscape.mode() == LANDSCAPE_MODE_STATIC {
        // C4GameSaveScenario always writes the current static Map.bmp. The
        // runtime `fMapChanged` gate belongs to exact landscape saves only.
        mutations.push(LiveC4SaveLandscapeMutation::DeleteEntry {
            name: "Landscape.bmp".to_owned(),
        });
        if let Err(source) = engine.save_c4_static_landscape(&mut scratch) {
            append_scratch_file_mutation(&scratch, "Map.bmp", &mut mutations);
            append_scratch_material_mutation(&scratch, &mut mutations);
            return Err(SerializedLandscapeFailure {
                source: source.into(),
                mutations,
            });
        }
        append_scratch_file_mutation(&scratch, "Map.bmp", &mut mutations);
        append_scratch_material_mutation(&scratch, &mut mutations);
    }
    let (pxs_c4b, mass_mover_c4b) = if saves_auxiliary_systems {
        let pxs = engine.pxs_system.to_c4b();
        mutations.push(LiveC4SaveLandscapeMutation::DeleteEntry {
            name: "PXS.c4b".to_owned(),
        });
        if let Some(payload) = pxs.as_ref() {
            mutations.push(LiveC4SaveLandscapeMutation::PutFile {
                name: "PXS.c4b".to_owned(),
                payload: payload.clone(),
            });
        }
        let mass_movers = engine.mass_movers.to_c4b();
        mutations.push(LiveC4SaveLandscapeMutation::DeleteEntry {
            name: "MassMover.c4b".to_owned(),
        });
        if let Some(payload) = mass_movers.as_ref() {
            mutations.push(LiveC4SaveLandscapeMutation::PutFile {
                name: "MassMover.c4b".to_owned(),
                payload: payload.clone(),
            });
        }
        engine
            .materials
            .save_enumeration(&mut scratch)
            .map_err(|source| SerializedLandscapeFailure {
                source: source.into(),
                mutations: mutations.clone(),
            })?;
        append_scratch_file_mutation(&scratch, "MatMap.txt", &mut mutations);
        if landscape.mode() == LANDSCAPE_MODE_STATIC {
            mutations.push(LiveC4SaveLandscapeMutation::DeleteEntry {
                name: "Landscape.bmp".to_owned(),
            });
        }
        (pxs, mass_movers)
    } else {
        (None, None)
    };
    if copied_material_group_is_file {
        scratch.remove_entry("Material.c4g");
    }

    let packed = scratch
        .pack_raw()
        .map_err(|source| SerializedLandscapeFailure {
            source: source.into(),
            mutations: mutations.clone(),
        })?;
    let group = Group::from_raw_memory(PathBuf::from("Runtime.c4s"), packed).map_err(|source| {
        SerializedLandscapeFailure {
            source: source.into(),
            mutations: mutations.clone(),
        }
    })?;
    Ok(SerializedLandscape {
        landscape_bmp: read_landscape_result(
            read_optional_group_entry(&group, "Landscape.bmp"),
            &mutations,
        )?,
        landscape_png: read_landscape_result(
            read_optional_group_entry(&group, "Landscape.png"),
            &mutations,
        )?,
        diff_landscape_bmp: read_landscape_result(
            read_optional_group_entry(&group, "DiffLandscape.bmp"),
            &mutations,
        )?,
        map_bmp: read_landscape_result(read_optional_group_entry(&group, "Map.bmp"), &mutations)?,
        material_group: read_landscape_result(
            read_optional_group_entry_raw(&group, "Material.c4g"),
            &mutations,
        )?,
        mat_map_txt: read_landscape_result(
            read_optional_group_entry(&group, "MatMap.txt"),
            &mutations,
        )?
        .unwrap_or_default(),
        pxs_c4b,
        mass_mover_c4b,
        saves_auxiliary_systems,
        delete_sky_entry,
    })
}

fn read_landscape_result<T>(
    result: Result<T, GroupError>,
    mutations: &[LiveC4SaveLandscapeMutation],
) -> Result<T, SerializedLandscapeFailure> {
    result.map_err(|source| SerializedLandscapeFailure {
        source: source.into(),
        mutations: mutations.to_vec(),
    })
}

fn append_scratch_file_mutation(
    scratch: &MutableGroup,
    name: &str,
    mutations: &mut Vec<LiveC4SaveLandscapeMutation>,
) {
    let Some(payload) = read_scratch_entry(scratch, name, false) else {
        return;
    };
    mutations.push(LiveC4SaveLandscapeMutation::PutFile {
        name: name.to_owned(),
        payload,
    });
}

fn append_scratch_material_mutation(
    scratch: &MutableGroup,
    mutations: &mut Vec<LiveC4SaveLandscapeMutation>,
) {
    if !matches!(
        scratch.entry_kind("Material.c4g"),
        Some(MutableGroupEntryKind::ChildGroup)
    ) {
        return;
    }
    let Some(payload) = read_scratch_entry(scratch, "Material.c4g", true) else {
        return;
    };
    mutations.push(LiveC4SaveLandscapeMutation::MergeMaterialGroup { payload });
}

fn read_scratch_entry(scratch: &MutableGroup, name: &str, raw: bool) -> Option<Vec<u8>> {
    let packed = scratch.pack_raw().ok()?;
    let group = Group::from_raw_memory(PathBuf::from("Runtime.c4s"), packed).ok()?;
    if raw {
        read_optional_group_entry_raw(&group, name).ok().flatten()
    } else {
        read_optional_group_entry(&group, name).ok().flatten()
    }
}

fn encode_landscape_png(landscape: &crate::Landscape) -> Result<Vec<u8>, image::ImageError> {
    let grid = landscape
        .pixel_grid()
        .expect("the live save checked the Surface8 grid");
    let mut image = RgbaImage::new(grid.width(), grid.height());
    for y in 0..grid.height() {
        for x in 0..grid.width() {
            let pixel = if let Some(color) = landscape.surface32_pixel_at(x as i32, y as i32) {
                Rgba([
                    ((color >> 16) & 0xff) as u8,
                    ((color >> 8) & 0xff) as u8,
                    (color & 0xff) as u8,
                    255_u8.wrapping_sub((color >> 24) as u8),
                ])
            } else {
                let index = grid.byte_at(x as i32, y as i32).unwrap_or(0);
                let (rgb, transparency, _) = landscape.surface8_palette_entry(index);
                Rgba([rgb[0], rgb[1], rgb[2], 255_u8.wrapping_sub(transparency)])
            };
            image.put_pixel(x, y, pixel);
        }
    }
    let mut output = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(image).write_to(&mut output, ImageOutputFormat::Png)?;
    Ok(output.into_inner())
}

fn read_optional_group_entry(group: &Group, name: &str) -> Result<Option<Vec<u8>>, GroupError> {
    if group.exists(name) {
        group.read_file(name).map(Some)
    } else {
        Ok(None)
    }
}

fn read_optional_group_entry_raw(group: &Group, name: &str) -> Result<Option<Vec<u8>>, GroupError> {
    if group.exists(name) {
        group.read_entry_bytes(name).map(Some)
    } else {
        Ok(None)
    }
}

fn serialize_teams(
    teams: &[TeamInfo],
    configuration: TeamConfiguration,
    last_team_id: i32,
    max_script_players: i32,
    script_player_names: &[u8],
    random_team_count: i32,
) -> Option<Vec<u8>> {
    let script_player_names = &script_player_names[..script_player_names
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(script_player_names.len())];
    let has_compiled_value = !configuration.active
        || !configuration.custom
        || configuration.allow_hostility_change
        || configuration.allow_team_switch
        || configuration.auto_generate_teams
        || last_team_id != 0
        || configuration.distribution != 0
        || configuration.team_colors
        || max_script_players != 0
        || !script_player_names.is_empty()
        || random_team_count != 0
        || !teams.is_empty();
    // C4TeamList::Save still creates Teams.txt, but INI naming sections are
    // lazy: the all-default list decompiles to a zero-byte component.
    if !has_compiled_value {
        return Some(Vec::new());
    }
    let mut writer = TextComponentWriter::default();
    writer.section(0, "Teams");
    field_bool(&mut writer, 0, "Active", configuration.active, true);
    field_bool(&mut writer, 0, "Custom", configuration.custom, true);
    field_bool(
        &mut writer,
        0,
        "AllowHostilityChange",
        configuration.allow_hostility_change,
        false,
    );
    field_bool(
        &mut writer,
        0,
        "AllowTeamSwitch",
        configuration.allow_team_switch,
        false,
    );
    field_bool(
        &mut writer,
        0,
        "AutoGenerateTeams",
        configuration.auto_generate_teams,
        false,
    );
    field_i32(&mut writer, 0, "LastTeamID", last_team_id, 0);
    let distribution = match configuration.distribution {
        0 => "Free",
        1 => "Host",
        2 => "None",
        3 => "Random",
        4 => "RandomInv",
        _ => "Free",
    };
    if distribution != "Free" {
        writer.field(0, "TeamDistribution", distribution);
    }
    field_bool(
        &mut writer,
        0,
        "TeamColors",
        configuration.team_colors,
        false,
    );
    field_i32(&mut writer, 0, "MaxScriptPlayers", max_script_players, 0);
    if !script_player_names.is_empty() {
        writer.field_bytes(
            0,
            "ScriptPlayerNames",
            &quote_ini_bytes(script_player_names),
        );
    }
    field_i32(&mut writer, 0, "RandomTeamCount", random_team_count, 0);
    for team in teams {
        writer.section(2, "Team");
        field_i32(&mut writer, 2, "id", team.id, 0);
        if !team.name.is_empty() {
            writer.field(2, "Name", &team.name);
        }
        field_i32(&mut writer, 2, "PlrStartIndex", team.player_start_index, 0);
        if !team.player_ids.is_empty() {
            writer.field(2, "PlayerCount", team.player_ids.len().to_string());
            writer.field(
                2,
                "Players",
                team.player_ids
                    .iter()
                    .map(i32::to_string)
                    .collect::<Vec<_>>()
                    .join(","),
            );
        }
        if team.color != 0 {
            writer.field(2, "Color", team.color.to_string());
        }
        if let Some(icon) = team.icon_spec.as_deref().filter(|icon| !icon.is_empty()) {
            writer.field(2, "IconSpec", quote_ini(icon));
        }
        field_i32(&mut writer, 2, "MaxPlayer", team.max_players, 0);
    }
    Some(writer.finish())
}

fn serialize_round_results(results: &RoundResultsState, melee: bool) -> Option<Vec<u8>> {
    if results.goal_counts.is_empty()
        && results.goals.is_empty()
        && results.playing_time_seconds == 0
        && results.hide_settlement_score == melee
        && results.custom_evaluation_strings.is_empty()
        && results.league_performance == 0
        && results.players.is_empty()
        && results.network_result_message.is_empty()
        && results.network_result.is_none()
    {
        return None;
    }
    let mut writer = TextComponentWriter::default();
    writer.section(0, "RoundResults");
    if !results.goal_counts.is_empty() || !results.goals.is_empty() {
        writer.field(
            0,
            "Goals",
            encode_id_list(if results.goal_counts.is_empty() {
                results
                    .goals
                    .iter()
                    .cloned()
                    .map(|goal| (goal, 1))
                    .collect::<Vec<_>>()
            } else {
                results.goal_counts.clone()
            }),
        );
    }
    if results.playing_time_seconds != 0 {
        writer.field(0, "PlayingTime", results.playing_time_seconds.to_string());
    }
    if results.hide_settlement_score != melee {
        field_bool(
            &mut writer,
            0,
            "HideSettlementScore",
            results.hide_settlement_score,
            melee,
        );
    }
    if !results.custom_evaluation_strings.is_empty() {
        writer.field(
            0,
            "CustomEvaluationStrings",
            quote_ini(&results.custom_evaluation_strings),
        );
    }
    field_i32(
        &mut writer,
        0,
        "LeaguePerformance",
        results.league_performance,
        0,
    );
    if !results.players.is_empty() {
        writer.section(2, "PlayerInfos");
        for player in &results.players {
            writer.section(4, "Player");
            field_i32(&mut writer, 4, "ID", player.player_info_id, 0);
            if player.total_playing_time != 0 {
                writer.field(4, "TotalPlayingTime", player.total_playing_time.to_string());
            }
            field_i32(&mut writer, 4, "SettlementScoreOld", player.score_old, -1);
            field_i32(
                &mut writer,
                4,
                "SettlementScoreNew",
                player.score_new.unwrap_or(-1),
                -1,
            );
            field_i32(&mut writer, 4, "Score", player.league_score_new, -1);
            field_i32(&mut writer, 4, "GameScore", player.league_score_gain, -1);
            field_i32(&mut writer, 4, "Rank", player.league_rank_new, 0);
            field_i32(
                &mut writer,
                4,
                "RankSymbol",
                player.league_rank_symbol_new,
                0,
            );
            if let Some(progress) = player.league_progress_data.as_deref() {
                writer.field_bytes(4, "LeagueProgressData", &quote_ini_bytes(progress));
            }
            match player.status {
                RoundResultsPlayerStatus::Unknown => {}
                RoundResultsPlayerStatus::Lost => writer.field(4, "Status", "Lost"),
                RoundResultsPlayerStatus::Won => writer.field(4, "Status", "Won"),
            }
        }
    }
    if !results.network_result_message.is_empty() {
        writer.field_bytes(
            0,
            "NetResult",
            &quote_ini_bytes(&results.network_result_message),
        );
    }
    if let Some(result) = results.network_result {
        writer.field(
            0,
            "NetResult",
            match result {
                RoundResultsNetworkResult::LeagueOk => "LeagueOK",
                RoundResultsNetworkResult::LeagueError => "LeagueError",
                RoundResultsNetworkResult::NetworkError => "NetError",
            },
        );
    }
    Some(writer.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PlayerStatus;

    fn section_spec(
        name: &str,
        source_group: Option<Group>,
    ) -> crate::scenario::ScenarioSectionSpec {
        crate::scenario::ScenarioSectionSpec {
            name: name.to_owned(),
            source_group,
            landscape: None,
            landscape_systems: crate::scenario::ScenarioLandscapeSystems::default(),
            exact_landscape: false,
            texmap_lookups: Vec::new(),
            resynthesize_static_map: false,
            map_creator: None,
            s2_overload: None,
            gravity: crate::scenario::LegacyC4SVal::new(100, 0, 10, 200),
            post_init_map_callbacks: crate::map_creator_s2::PostInitMapCallbacks::default(),
            keep_map_creator: false,
            no_initialize: false,
            objects: Vec::new(),
            scenario_values: ScenarioValueStore::default(),
            base_reject_entrance_enabled: true,
            base_extinguish_enabled: true,
            environment: crate::EnvironmentSettings::default(),
        }
    }

    #[test]
    fn loaded_initial_record_scenario_uses_restored_runtime_values() {
        let mut engine = Engine::new();
        engine.set_scenario_values(ScenarioValueStore::with_value_gain_for_test(321));

        let actual = engine.serialize_initial_record_scenario_from_runtime_savegame(
            "Loaded record",
            &[],
            "",
            "",
            "Loaded.c4s",
        );
        let actual = String::from_utf8(actual).expect("Scenario.txt is ASCII");

        assert!(actual.contains("ValueGain=321\r\n"));
        for expected in ["SaveGame=1\r\n", "NoInitialize=1\r\n", "Replay=1\r\n"] {
            assert!(actual.contains(expected), "missing {expected}");
        }
        assert!(actual.contains("Icon=29\r\n"));
    }

    #[test]
    fn virtual_save_restores_the_exact_loaded_string_enumeration_order() {
        let mut engine = Engine::new();
        engine.adopt_loaded_c4_string_table(&[
            b"zeta created first".to_vec(),
            b"alpha created second".to_vec(),
        ]);

        let values = engine.enumerate_live_c4_string_table_for_save();

        assert_eq!(
            values,
            vec![
                b"zeta created first".to_vec(),
                b"alpha created second".to_vec(),
            ]
        );
    }

    #[test]
    fn section_landscape_save_requires_a_native_surface8_plane() {
        let mut spec = section_spec("main", None);
        spec.landscape = Some(crate::Landscape::flat(2, 2));
        let mut engine = Engine::new();
        engine.configure_scenario_sections(&[spec]);
        let section = engine.scenario_sections.get("main").unwrap();

        assert!(matches!(
            freeze_scenario_section(&engine, section, true, false),
            Err(LiveC4SaveError::MissingPixelGrid)
        ));
    }

    #[test]
    fn ordinary_scenario_save_allows_an_absent_dynamic_landscape() {
        let saved = serialize_landscape_for_policy(
            &Engine::new(),
            LiveC4SavePolicy::Scenario {
                force_exact_landscape: false,
            },
            false,
        )
        .expect("a non-forced scenario has no landscape component to save");
        assert!(saved.landscape_bmp.is_none());
        assert!(saved.diff_landscape_bmp.is_none());
        assert!(saved.map_bmp.is_none());
        assert!(!saved.saves_auxiliary_systems);

        assert!(matches!(
            serialize_landscape_for_policy(&Engine::new(), LiveC4SavePolicy::RuntimeNetwork, false,),
            Err(SerializedLandscapeFailure {
                source: LiveC4SaveError::MissingLandscape,
                ..
            })
        ));
    }

    #[test]
    fn component_hosts_preserve_actual_names_and_modified_deletions() {
        let mut engine = Engine::new();
        let save = engine
            .serialize_live_c4_save_with_policy(
                LiveC4SaveSpec {
                    title: "Components",
                    definition_modules: &[],
                    definition_executable_path: "",
                    definition_path: "",
                    origin: "Components.c4s",
                    music_enabled: false,
                    copied_material_group_is_file: false,
                    title_component: LiveC4ComponentHost::Delete {
                        name: "TitleDE.txt",
                    },
                    info_component: LiveC4ComponentHost::Replace(LiveC4SaveComponentRef {
                        name: "InfoDE.txt",
                        payload: b"localized info",
                    }),
                    script_component: LiveC4ComponentHost::Delete { name: "ScriptDE.c" },
                },
                LiveC4SavePolicy::Scenario {
                    force_exact_landscape: false,
                },
            )
            .expect("component-host scenario saves");

        assert_eq!(
            save.info_txt,
            Some(LiveC4SaveNamedComponent {
                name: "InfoDE.txt".to_owned(),
                payload: b"localized info".to_vec(),
            })
        );
        assert_eq!(
            save.deleted_components,
            ["ScriptDE.c".to_owned(), "TitleDE.txt".to_owned()]
        );
        assert!(matches!(
            save.component_host_mutations.as_slice(),
            [
                LiveC4SaveComponentMutation::Delete { name: script },
                LiveC4SaveComponentMutation::Delete { name: title },
                LiveC4SaveComponentMutation::Replace(info),
            ] if script == "ScriptDE.c"
                && title == "TitleDE.txt"
                && info.name == "InfoDE.txt"
                && info.payload == b"localized info"
        ));
    }

    #[test]
    fn ordinary_static_scenario_save_always_writes_the_retained_map() {
        let mut landscape = crate::Landscape::flat(2, 2);
        assert!(landscape.set_mode(LANDSCAPE_MODE_STATIC));
        let mut raster = crate::landscape::LandscapeRasterState::new(
            1,
            0,
            crate::landscape::RuntimeTexMapState::default(),
        );
        raster.set_map(&IndexedBitmap {
            width: 2,
            height: 2,
            indices: vec![0, 1, 1, 0],
        });
        landscape.set_raster_state(raster);
        assert!(
            !landscape.map_changed(),
            "fixture map is deliberately clean"
        );
        let mut engine = Engine::new();
        engine.set_landscape(landscape);

        let saved = serialize_landscape_for_policy(
            &engine,
            LiveC4SavePolicy::Scenario {
                force_exact_landscape: false,
            },
            false,
        )
        .expect("unchanged static map still saves");
        assert!(saved.map_bmp.is_some());
    }

    #[test]
    fn ordinary_static_scenario_save_fails_without_a_retained_map() {
        let mut landscape = crate::Landscape::flat(2, 2);
        assert!(landscape.set_mode(LANDSCAPE_MODE_STATIC));
        let mut engine = Engine::new();
        engine.set_landscape(landscape);

        assert!(matches!(
            serialize_landscape_for_policy(
                &engine,
                LiveC4SavePolicy::Scenario {
                    force_exact_landscape: false,
                },
                false,
            ),
            Err(SerializedLandscapeFailure {
                source: LiveC4SaveError::Landscape(
                    crate::LandscapePersistenceError::MissingMap
                ),
                mutations,
            }) if matches!(
                mutations.as_slice(),
                [LiveC4SaveLandscapeMutation::DeleteEntry { name }]
                    if name == "Landscape.bmp"
            )
        ));
    }

    #[test]
    fn temporary_physical_changes_preserve_order_and_duplicates() {
        let mut writer = TextComponentWriter::default();
        writer.section(0, "Physical");
        serialize_physical(
            &mut writer,
            0,
            &PhysicalInfo::default(),
            &[
                ("Walk".to_owned(), 10),
                ("Energy".to_owned(), 20),
                ("Walk".to_owned(), 30),
            ],
        );
        assert_eq!(
            writer.finish(),
            b"[Physical]\r\nChanges=Walk=10,Energy=20,Walk=30\r\n"
        );
    }

    #[test]
    fn object_temporary_physical_section_is_follow_name_sibling() {
        let mut engine = Engine::new();
        engine
            .register_definition(
                crate::Definition::from_script("PHYS", "Physical", "")
                    .expect("definition compiles"),
            )
            .expect("definition registers");
        let mut config = crate::SpawnConfig::new("PHYS");
        config.temporary_physical = Some(PhysicalInfo {
            energy: 123,
            ..PhysicalInfo::default()
        });
        config.physical_changes = vec![("Energy".to_owned(), 77)];
        engine.spawn_object(config).expect("object spawns");

        let objects = String::from_utf8(serialize_objects(
            &engine,
            &mut LegacyStringTable::default(),
        ))
        .expect("Objects.txt is UTF-8");
        assert!(objects.contains("\r\n[Physical]\r\nEnergy=123\r\nChanges=Energy=77\r\n"));
        assert!(!objects.contains("\r\n  [Physical]"));
    }

    #[test]
    fn object_empty_temporary_physical_omits_empty_follow_name_section() {
        let mut engine = Engine::new();
        engine
            .register_definition(
                crate::Definition::from_script("ZERO", "Zero", "").expect("definition compiles"),
            )
            .expect("definition registers");
        let mut config = crate::SpawnConfig::new("ZERO");
        config.temporary_physical = Some(PhysicalInfo::default());
        engine.spawn_object(config).expect("object spawns");

        let objects = String::from_utf8(serialize_objects(
            &engine,
            &mut LegacyStringTable::default(),
        ))
        .expect("Objects.txt is UTF-8");
        assert!(objects.contains("\r\nPhysicalTemporary=true\r\n"));
        assert!(!objects.contains("[Physical]"));
    }

    #[test]
    fn scoreboard_emits_the_required_empty_string_for_every_cell() {
        let scoreboard = ScoreboardState::from_compiled_cells(
            1,
            2,
            0,
            vec![(None, 7), (Some(String::new()), -3)],
        )
        .expect("rectangular scoreboard");
        assert_eq!(
            serialize_scoreboard(&scoreboard).expect("nondefault scoreboard"),
            b"[Scoreboard]\r\nRows=1\r\nCols=2\r\nCell0_0String=\"\"\r\nCell0_0Value=7\r\nCell1_0String=\"\"\r\nCell1_0Value=-3\r\n"
        );
    }

    #[test]
    fn escaped_strings_match_std_compiler_for_controls_octal_and_nul() {
        assert_eq!(
            quote_ini_bytes(&[
                0x07, 0x08, 0x0c, b'\n', b'\r', b'\t', 0x0b, b'"', b'\\', 0x01, b'7', 0x80, 0,
                b'X',
            ]),
            b"\"\\a\\b\\f\\n\\r\\t\\v\\\"\\\\\\1\\67\\200\""
        );
    }

    #[test]
    fn native_booleans_use_canonical_words() {
        let mut writer = TextComponentWriter::default();
        field_bool(&mut writer, 0, "Enabled", true, false);
        field_bool(&mut writer, 0, "Disabled", false, true);
        assert_eq!(writer.finish(), b"Enabled=true\r\nDisabled=false\r\n");
    }

    #[test]
    fn raw_byte_fields_write_the_field_name_exactly_once() {
        let mut writer = TextComponentWriter::default();
        writer.field_bytes(2, "NetResult", b"\"raw\\200\"");
        assert_eq!(writer.finish(), b"  NetResult=\"raw\\200\"\r\n");
    }

    #[test]
    fn strings_txt_uses_cpp_first_nul_identity_and_payload_truncation() {
        let mut strings = LegacyStringTable::default();
        assert_eq!(strings.id_for("shared\0first suffix"), 0);
        assert_eq!(strings.id_for("shared\0second suffix"), 0);
        assert_eq!(strings.id_for("other\0suffix"), 1);
        assert_eq!(strings.encoded().unwrap(), b"shared\r\nother\r\n");

        let enumerated = C4StringValue::from("shared\0first suffix");
        let enumeration = LiveC4ValueEnumeration::from_strings_in_id_order([
            enumerated.clone(),
            C4StringValue::from("other\0suffix"),
        ]);
        assert_eq!(
            enumeration
                .encode_value(&Value::String(enumerated))
                .unwrap(),
            "S0"
        );
        assert!(
            enumeration
                .encode_value(&Value::String(C4StringValue::from("shared\0later suffix",)))
                .is_err(),
            "equal C-string text does not confer another C4String's enum identity"
        );
    }

    #[test]
    fn strings_txt_retains_cpp_pre_normalization_allocation_length() {
        let strings = LegacyStringTable::from_enumerated_values(vec![b"a\nb".to_vec()]);

        assert_eq!(
            strings.encoded().expect("nonempty string table"),
            b"ab\r\n\0",
            "C++ allocates from the original length before deleting LF"
        );
    }

    #[test]
    fn c4value_booleans_retain_noncanonical_union_payloads() {
        assert_eq!(
            encode_value(&Value::RawBool(7), &mut LegacyStringTable::default()),
            "b7"
        );
        #[cfg(target_pointer_width = "64")]
        {
            let raw = 1_usize << 32;
            assert_eq!(
                encode_value(
                    &Value::from_c4_bool_data_raw(raw),
                    &mut LegacyStringTable::default()
                ),
                "b0",
                "C4Value::CompileFunc persists only the low Data.Int"
            );
            assert_eq!(
                encode_effect_value(
                    &EffectVarValue::RawBool(raw),
                    &mut LegacyStringTable::default()
                ),
                "b0",
                "effect values use the same C4Value compiler boundary"
            );
        }
        assert_eq!(
            encode_effect_value(
                &EffectVarValue::RawBool((-3_i32) as u32 as usize),
                &mut LegacyStringTable::default()
            ),
            "b-3"
        );
    }

    #[test]
    fn every_live_pointer_writer_uses_the_cpp_object_number_boundary() {
        let mut engine = Engine::new();
        engine
            .register_definition(
                crate::Definition::from_script("REFS", "References", "")
                    .expect("definition compiles"),
            )
            .expect("definition registers");
        for id in 1..=4 {
            engine
                .spawn_object(crate::SpawnConfig::new("REFS").with_id(ObjectId::new(id)))
                .expect("object spawns");
        }
        engine.exec_list = vec![ObjectId::new(1), ObjectId::new(4)];
        engine.inactive_exec_list = vec![ObjectId::new(3)];
        engine.objects[2].state.status = ObjectStatus::Inactive;
        engine.objects[3].state.status = ObjectStatus::Deleted;

        let mut strings = LegacyStringTable::default();
        strings.set_object_numbers(engine.live_object_numbers_for_save());
        assert_eq!(encode_value(&Value::Object(2), &mut strings), "O0");
        assert_eq!(encode_value(&Value::Object(3), &mut strings), "O3");

        let effect = EffectState::new("Refs")
            .with_command_target(Some(2))
            .with_vars(vec![EffectVarValue::Object(2), EffectVarValue::Object(3)]);
        assert_eq!(
            serialize_effect_chain(&[effect], &mut strings),
            "Refs(0,100,0,0,0,NONE)[2;O0,O3]"
        );

        let command = LegacyCommandSave {
            view: crate::command::CommandView {
                name: "Call".to_owned(),
                target: Some(ObjectId::new(2)),
                tx: None,
                tx_value: Some(Value::Object(2)),
                tx_definition: None,
                ty: None,
                target2: Some(ObjectId::new(3)),
                data: CommandData::None,
                legacy_data: None,
                finished: false,
            },
            update_interval: 0,
            evaluated: 0,
            path_checked: 0,
            finished: 0,
            failures: 0,
            retries: 0,
            permit: 0,
            base_mode: 0,
            text: String::new(),
        };
        let command = serialize_command(&command, &mut strings);
        assert!(command.starts_with("$2,Call,O0,0,0,3,"));

        let overlay = crate::ObjectGraphicsOverlay::new(1, crate::GraphicsOverlayMode::Object)
            .with_overlay_object(Some(ObjectId::new(2)));
        assert!(serialize_graphics_overlay(&overlay, &strings).ends_with(",0"));

        let player = PlayerState {
            cursor: Some(ObjectId::new(2)),
            view_cursor: Some(ObjectId::new(3)),
            captain: Some(ObjectId::new(4)),
            crew: vec![ObjectId::new(2), ObjectId::new(4), ObjectId::new(3)],
            message_board_queries: vec![crate::MessageBoardQuery::new(
                Some(ObjectId::new(2)),
                "Prompt".to_owned(),
                false,
            )],
            ..PlayerState::default()
        };
        let player = String::from_utf8(serialize_players(&engine, &[player], &mut strings))
            .expect("player section is text");
        assert!(!player.contains("Cursor=2\r\n"));
        assert!(player.contains("ViewCursor=3\r\n"));
        assert!(player.contains("Captain=4\r\n"));
        assert!(player.contains("Crew=2;3\r\n"));
        assert!(player.contains("MsgBoardQueries=(0,\"Prompt\",0)\r\n"));
    }

    #[test]
    fn strings_and_named_globals_keep_native_registration_and_declaration_order() {
        let registrations = clonk_script::new_string_registrations();
        let zeta = C4StringValue::from("zeta");
        let dead = C4StringValue::from("dead");
        let alpha = C4StringValue::from("alpha");
        clonk_script::register_c4_string(&registrations, &zeta);
        clonk_script::register_c4_string(&registrations, &dead);
        clonk_script::register_c4_string(&registrations, &alpha);
        drop(dead);
        let mut strings = LegacyStringTable::from_enumerated_values(
            clonk_script::enumerate_c4_strings(&registrations, &[alpha.clone(), zeta.clone()]),
        );
        let globals = crate::ScriptGlobalState {
            numbered: Default::default(),
            named: std::collections::BTreeMap::from([
                ("Alpha".to_string(), Value::String(alpha)),
                ("Zed".to_string(), Value::String(zeta)),
            ]),
        };

        assert_eq!(
            serialize_script_globals(
                &globals,
                &["Zed".to_string(), "Alpha".to_string()],
                false,
                0,
                &mut strings,
            )
            .unwrap(),
            b"[Script]\r\nGlobalNamed=2;Zed=S0,Alpha=S1\r\n"
        );
        assert_eq!(strings.encoded().unwrap(), b"zeta\r\nalpha\r\n");
    }

    #[test]
    fn loaded_table_and_static_const_strings_remain_enumerable() {
        let mut engine = Engine::new();
        engine.set_legacy_string_table(HashMap::from([
            (1, "loaded one".to_string()),
            (0, "loaded zero".to_string()),
        ]));
        engine.script_global_consts.borrow_mut().insert(
            "SavedConst".to_string(),
            clonk_script::value_cell(Value::Array(vec![Value::String(
                "constant value".to_string().into(),
            )])),
        );

        let state = engine.capture_state();
        let referenced = collect_live_referenced_strings(&engine, &state);
        let strings = LegacyStringTable::from_enumerated_values(
            clonk_script::enumerate_c4_strings(&engine.script_string_registrations, &referenced),
        );
        assert_eq!(
            strings.encoded().unwrap(),
            b"loaded zero\r\nloaded one\r\nconstant value\r\n"
        );
    }

    #[test]
    fn effect_map_hidden_slots_keep_strings_live_for_root_enumeration() {
        let mut engine = Engine::new();
        engine.set_legacy_string_table(HashMap::from([(0, "hidden effect string".to_owned())]));
        let hidden = clonk_script::resolve_c4_string(&engine.script_string_registrations, 0)
            .expect("loaded string resolves into the effect map");
        let mut map = clonk_script::ValueMap::new();
        map.recycle_value_slot(Value::String(hidden));
        engine
            .global_effects
            .push(EffectState::new("HiddenMap").with_vars(vec![EffectVarValue::Proplist(map)]));

        let state = engine.capture_state();
        let referenced = collect_live_referenced_strings(&engine, &state);
        let mut strings = LegacyStringTable::from_enumerated_values(
            clonk_script::enumerate_c4_strings(&engine.script_string_registrations, &referenced),
        );
        assert_eq!(
            strings.encoded().as_deref(),
            Some(b"hidden effect string\r\n".as_slice())
        );
        let effects = serialize_effects(&state.global_effects, &mut strings)
            .expect("the effect component is present");
        assert!(
            String::from_utf8_lossy(&effects).contains("m[0;]"),
            "hidden slots affect lifetime but are not visible serialized entries"
        );
    }

    #[test]
    fn default_script_state_omits_the_empty_naming_section() {
        assert_eq!(
            serialize_script_globals(
                &crate::ScriptGlobalState::default(),
                &[],
                false,
                0,
                &mut LegacyStringTable::default(),
            ),
            None
        );
    }

    #[test]
    fn runtime_players_use_the_normal_compiler_section_separator() {
        assert_eq!(
            append_runtime_player_sections(
                b"[Game]\r\nTime=1\r\n".to_vec(),
                b"[Player1]\r\nStatus=1\r\n",
            ),
            b"[Game]\r\nTime=1\r\n\r\n[Player1]\r\nStatus=1\r\n"
        );
    }

    #[test]
    fn non_exact_game_contains_only_script_and_effect_components() {
        assert_eq!(
            serialize_non_exact_game(
                Some(b"[Script]\r\nCounter=3\r\n".to_vec()),
                Some(b"[Effects]\r\nGlobalEffects=Fx(1)\r\n".to_vec()),
            ),
            b"[Script]\r\nCounter=3\r\n\r\n[Effects]\r\nGlobalEffects=Fx(1)\r\n"
        );
        assert_eq!(
            serialize_non_exact_game(None, Some(b"[Effects]\r\nGlobalEffects=Fx(1)\r\n".to_vec()),),
            b"[Effects]\r\nGlobalEffects=Fx(1)\r\n"
        );
        assert!(serialize_non_exact_game(None, None).is_empty());
    }

    #[test]
    fn draw_transform_uses_printf_g_and_preserves_identity_presence() {
        assert_eq!(
            serialize_draw_transform(crate::DrawTransform::identity()),
            "1,0,0,0,1,0,1"
        );
        for (value, expected) in [
            (1.234_567_8, "1.23457"),
            (0.000_123_456_78, "0.000123457"),
            (0.000_012_345_678, "1.23457e-05"),
            (1_234_567.0, "1.23457e+06"),
            (999_999.9, "1e+06"),
            (-0.0, "-0"),
        ] {
            assert_eq!(format_legacy_float(value), expected, "value {value:?}");
        }
    }

    #[test]
    fn command_compile_func_uses_cpp_comma_separators() {
        let command = LegacyCommandSave {
            view: crate::command::CommandView {
                name: "MoveTo".to_owned(),
                target: Some(ObjectId::new(41)),
                tx: Some(12),
                tx_value: Some(Value::Int(12)),
                tx_definition: None,
                ty: Some(-3),
                target2: Some(ObjectId::new(42)),
                data: CommandData::Integer(7),
                legacy_data: None,
                finished: true,
            },
            update_interval: 5,
            evaluated: -2,
            path_checked: 0,
            finished: 7,
            failures: 2,
            retries: 3,
            permit: 0,
            base_mode: 1,
            text: "raw,text".to_owned(),
        };
        let mut strings = LegacyStringTable::default();
        assert_eq!(
            serialize_command(&command, &mut strings),
            "$2,MoveTo,i12,-3,41,42,7,5,-2,0,7,2,3,0,1,raw,text"
        );
    }

    #[test]
    fn call_command_preserves_tagged_tx_and_independent_data_word() {
        let payload = C4StringValue::from("payload");
        let command = LegacyCommandSave {
            view: crate::command::CommandView {
                name: "Call".to_owned(),
                target: Some(ObjectId::new(41)),
                tx: None,
                tx_value: Some(Value::Array(vec![
                    Value::String(payload.clone()),
                    Value::C4Id(clonk_script::c4_id_from_raw(0)),
                ])),
                tx_definition: None,
                ty: Some(-3),
                target2: Some(ObjectId::new(42)),
                data: CommandData::Text("DoThing".to_owned()),
                legacy_data: Some(37),
                finished: false,
            },
            update_interval: 5,
            evaluated: 1,
            path_checked: 0,
            finished: 0,
            failures: 0,
            retries: 3,
            permit: 0,
            base_mode: 1,
            text: "DoThing".to_owned(),
        };
        let mut strings = LegacyStringTable::default();
        assert_eq!(
            serialize_command(&command, &mut strings),
            "$2,Call,a[2;S-1,I0],-3,41,42,37,5,1,0,0,0,3,0,1,DoThing",
            "C4Value::CompileFunc writes a runtime string's current -1 enum ID verbatim"
        );
        assert!(
            strings.values.is_empty(),
            "serialization does not enumerate"
        );

        let registrations = clonk_script::new_string_registrations();
        clonk_script::register_c4_string(&registrations, &payload);
        strings = LegacyStringTable::from_enumerated_values(clonk_script::enumerate_c4_strings(
            &registrations,
            std::slice::from_ref(&payload),
        ));
        assert_eq!(
            serialize_command(&command, &mut strings),
            "$2,Call,a[2;S0,I0],-3,41,42,37,5,1,0,0,0,3,0,1,DoThing"
        );
        assert_eq!(strings.values, vec![b"payload".to_vec()]);
    }

    #[test]
    fn runtime_objects_keep_the_native_inactive_list_separator() {
        assert_eq!(
            serialize_objects(&Engine::new(), &mut LegacyStringTable::default()),
            b"\r\n"
        );
    }

    #[test]
    fn object_saves_follow_the_already_reversed_exec_ledgers() {
        fn numbers(bytes: &[u8]) -> Vec<u64> {
            String::from_utf8_lossy(bytes)
                .lines()
                .filter_map(|line| line.strip_prefix("Number=")?.parse().ok())
                .collect()
        }

        let mut engine = Engine::new();
        engine
            .register_definition(
                crate::Definition::from_script("ORDR", "Order", "")
                    .expect("fixture definition compiles"),
            )
            .expect("fixture definition registers");
        let ids = (0..5)
            .map(|_| {
                engine
                    .spawn_object(crate::SpawnConfig::new("ORDR"))
                    .expect("fixture object spawns")
            })
            .collect::<Vec<_>>();

        // Both vectors are Last -> Prev already. Saving must consume them
        // directly; reversing here would invert native execution order.
        engine.exec_list = vec![ids[2], ids[0], ids[1]];
        engine.inactive_exec_list = vec![ids[4], ids[3]];
        for id in &ids[3..] {
            let index = engine
                .find_object_index(*id)
                .expect("inactive object exists");
            engine.objects[index].state.status = ObjectStatus::Inactive;
        }
        assert_eq!(
            numbers(&serialize_objects(
                &engine,
                &mut LegacyStringTable::default()
            )),
            [ids[2], ids[0], ids[1], ids[4], ids[3]]
                .into_iter()
                .map(ObjectId::as_u64)
                .collect::<Vec<_>>()
        );

        let state = engine.capture_state();
        assert_eq!(
            numbers(&serialize_persisted_objects(
                &engine,
                &state.objects,
                &state.object_order,
                &mut LegacyStringTable::default(),
            )),
            [ids[2], ids[0], ids[1]]
                .into_iter()
                .map(ObjectId::as_u64)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn vertex_cnat_preserves_signed_bits_through_text_round_trip() {
        let mut definition =
            crate::Definition::from_script("CNAT", "Contact", "").expect("definition compiles");
        definition.set_shape_vertices(vec![crate::ObjectVertex::new(0, 0).with_cnat(u32::MAX)]);
        let mut engine = Engine::new();
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine
            .spawn_object(crate::SpawnConfig::new("CNAT"))
            .expect("fixture object spawns");

        let text = String::from_utf8(serialize_objects(
            &engine,
            &mut LegacyStringTable::default(),
        ))
        .expect("Objects.txt is UTF-8");
        let serialized = text
            .lines()
            .find_map(|line| line.strip_prefix("VertexCNAT="))
            .expect("nonzero CNAT is serialized");
        assert_eq!(serialized, "-1");
        assert_eq!(
            serialized.parse::<i32>().expect("native signed field") as u32,
            u32::MAX
        );
    }

    #[test]
    fn live_save_enumerates_only_listed_object_pointer_caches_before_serializing() {
        let mut engine = Engine::new();
        engine
            .register_definition(
                crate::Definition::from_script("CACH", "Cache", "").expect("definition compiles"),
            )
            .expect("definition registers");
        let object = engine
            .spawn_object(crate::SpawnConfig::new("CACH").with_id(ObjectId::new(1)))
            .expect("cache object spawns");
        let referenced = engine
            .spawn_object(crate::SpawnConfig::new("CACH").with_id(ObjectId::new(2)))
            .expect("referenced object spawns");
        let off_list = engine
            .spawn_object(crate::SpawnConfig::new("CACH").with_id(ObjectId::new(3)))
            .expect("off-list object spawns");
        let deleted = engine
            .spawn_object(crate::SpawnConfig::new("CACH").with_id(ObjectId::new(4)))
            .expect("deleted linked object spawns");
        engine.exec_list = vec![object, deleted];
        engine.inactive_exec_list = vec![referenced];
        let referenced_index = engine
            .find_object_index(referenced)
            .expect("referenced object index");
        engine.objects[referenced_index].state.status = ObjectStatus::Inactive;
        let deleted_index = engine
            .find_object_index(deleted)
            .expect("deleted object index");
        engine.objects[deleted_index].state.status = ObjectStatus::Deleted;
        engine.objects[deleted_index].compiler_cache.contained = 456;
        let index = engine
            .find_object_index(object)
            .expect("cache object index");
        engine.objects[index].state.container = Some(referenced);
        engine.objects[index].state.action.target = Some(off_list);
        engine.objects[index].state.action.target2 = Some(deleted);
        engine.objects[index].state.layer = Some(object);
        engine.objects[index].compiler_cache = crate::ObjectCompilerCache {
            info: "stale info".to_owned(),
            contained: -1,
            action_target1: 999,
            action_target2: 1_000_000_002,
            layer: -7,
        };
        let off_list_index = engine
            .find_object_index(off_list)
            .expect("off-list object index");
        engine.objects[off_list_index].compiler_cache.contained = 123;

        let enumeration = engine.enumerate_object_compiler_caches_for_save();

        assert_eq!(
            engine.objects[index].compiler_cache,
            crate::ObjectCompilerCache {
                info: String::new(),
                contained: 2,
                action_target1: 0,
                action_target2: 4,
                layer: 1,
            },
            "Objects.Save enumeration leaves every compiler cache refreshed",
        );
        assert_eq!(
            engine.objects[off_list_index].compiler_cache.contained, 123,
            "objects outside the active and inactive lists are not enumerated",
        );
        assert_eq!(
            engine.objects[deleted_index].compiler_cache.contained, 456,
            "status-zero wrappers remain numberable but are not enumerated",
        );

        let objects_txt = String::from_utf8(serialize_objects_for_save(
            &engine,
            &mut LegacyStringTable::default(),
            false,
        ))
        .expect("Objects.txt is UTF-8");
        assert!(objects_txt.contains("Contained=2\r\n"));
        assert!(objects_txt.contains("ActionTarget2=4\r\n"));
        assert!(objects_txt.contains("Layer=1\r\n"));
        assert!(!objects_txt.contains("ActionTarget1="));
        assert!(!objects_txt.contains("Info=stale info"));

        engine.denumerate_object_compiler_caches_after_save(&enumeration);
        assert_eq!(engine.objects[index].state.container, Some(referenced));
        assert_eq!(engine.objects[index].state.action.target, None);
        assert_eq!(engine.objects[index].state.action.target2, Some(deleted));
        assert_eq!(engine.objects[index].state.layer, Some(object));
    }

    #[test]
    fn root_live_save_enumerates_cache_words_before_writing_objects_txt() {
        let mut engine = Engine::new();
        engine
            .register_definition(
                crate::Definition::from_script("SAVE", "Save", "").expect("definition compiles"),
            )
            .expect("definition registers");
        let object = engine
            .spawn_object(crate::SpawnConfig::new("SAVE").with_id(ObjectId::new(1)))
            .expect("saved object spawns");
        let off_list = engine
            .spawn_object(crate::SpawnConfig::new("SAVE").with_id(ObjectId::new(2)))
            .expect("off-list object spawns");
        engine.exec_list = vec![object];
        engine.inactive_exec_list.clear();
        let index = engine
            .find_object_index(object)
            .expect("saved object index");
        engine.objects[index].state.action.target = Some(off_list);
        engine.objects[index].compiler_cache.action_target1 = 777;

        let definition_modules = Vec::new();
        let save = engine
            .serialize_live_c4_save_with_policy(
                LiveC4SaveSpec {
                    title: "Cache save",
                    definition_modules: &definition_modules,
                    definition_executable_path: "",
                    definition_path: "",
                    origin: "Cache.c4s",
                    music_enabled: false,
                    copied_material_group_is_file: false,
                    title_component: LiveC4ComponentHost::Unmodified,
                    info_component: LiveC4ComponentHost::Unmodified,
                    script_component: LiveC4ComponentHost::Unmodified,
                },
                LiveC4SavePolicy::Scenario {
                    force_exact_landscape: false,
                },
            )
            .expect("ordinary scenario save succeeds without a landscape");

        let objects_txt = String::from_utf8(save.objects_txt).expect("Objects.txt is UTF-8");
        assert!(!objects_txt.contains("ActionTarget1=777"));
        assert!(!objects_txt.contains("ActionTarget1=2"));
        assert_eq!(engine.objects[index].compiler_cache.action_target1, 0);
        assert_eq!(engine.objects[index].state.action.target, None);
    }

    #[test]
    fn failed_landscape_save_does_not_enumerate_object_wrappers() {
        let mut engine = Engine::new();
        engine
            .register_definition(
                crate::Definition::from_script("SAVE", "Save", "").expect("definition compiles"),
            )
            .expect("definition registers");
        let object = engine
            .spawn_object(crate::SpawnConfig::new("SAVE").with_id(ObjectId::new(1)))
            .expect("saved object spawns");
        let off_list = engine
            .spawn_object(crate::SpawnConfig::new("SAVE").with_id(ObjectId::new(2)))
            .expect("off-list object spawns");
        engine.exec_list = vec![object];
        let index = engine.find_object_index(object).unwrap();
        engine.objects[index].state.action.target = Some(off_list);
        engine.objects[index].compiler_cache.action_target1 = 777;

        let error = engine
            .serialize_live_c4_save_with_policy(
                LiveC4SaveSpec {
                    title: "Failed save",
                    definition_modules: &[],
                    definition_executable_path: "",
                    definition_path: "",
                    origin: "Failure.c4s",
                    music_enabled: false,
                    copied_material_group_is_file: false,
                    title_component: LiveC4ComponentHost::Unmodified,
                    info_component: LiveC4ComponentHost::Unmodified,
                    script_component: LiveC4ComponentHost::Unmodified,
                },
                LiveC4SavePolicy::RuntimeNetwork,
            )
            .expect_err("runtime save requires a landscape");
        assert!(matches!(
            error.root_cause(),
            LiveC4SaveError::MissingLandscape
        ));
        assert_eq!(engine.objects[index].compiler_cache.action_target1, 777);
        assert_eq!(engine.objects[index].state.action.target, Some(off_list));
    }

    #[test]
    fn copied_material_file_fails_before_object_enumeration_but_after_game_denumeration() {
        let mut engine = Engine::new();
        engine
            .register_definition(
                crate::Definition::from_script("SAVE", "Save", "").expect("definition compiles"),
            )
            .expect("definition registers");
        let object = engine
            .spawn_object(crate::SpawnConfig::new("SAVE").with_id(ObjectId::new(1)))
            .expect("saved object spawns");
        let off_list = engine
            .spawn_object(crate::SpawnConfig::new("SAVE").with_id(ObjectId::new(2)))
            .expect("off-list object spawns");
        engine.exec_list = vec![object];
        let index = engine
            .find_object_index(object)
            .expect("saved object index");
        engine.objects[index].state.action.target = Some(off_list);
        engine.objects[index].compiler_cache.action_target1 = 777;
        engine
            .global_effects
            .push(EffectState::new("Save").with_command_target(Some(2)));

        let mut texmap = crate::landscape::RuntimeTexMapState::default();
        texmap.entries_added = true;
        let mut raster = crate::landscape::LandscapeRasterState::new(1, 0, texmap);
        raster.set_map(&IndexedBitmap {
            width: 2,
            height: 2,
            indices: vec![0; 4],
        });
        let mut landscape = crate::Landscape::flat(2, 2);
        assert!(landscape.set_mode(LANDSCAPE_MODE_STATIC));
        landscape.set_raster_state(raster);
        engine.set_landscape(landscape);

        let error = engine
            .serialize_live_c4_save_with_policy(
                LiveC4SaveSpec {
                    title: "Failed material save",
                    definition_modules: &[],
                    definition_executable_path: "",
                    definition_path: "",
                    origin: "Failure.c4s",
                    music_enabled: false,
                    copied_material_group_is_file: true,
                    title_component: LiveC4ComponentHost::Unmodified,
                    info_component: LiveC4ComponentHost::Unmodified,
                    script_component: LiveC4ComponentHost::Unmodified,
                },
                LiveC4SavePolicy::Scenario {
                    force_exact_landscape: false,
                },
            )
            .expect_err("dirty textures cannot replace an ordinary Material.c4g file");

        assert!(matches!(
            error.root_cause(),
            LiveC4SaveError::Landscape(crate::LandscapePersistenceError::MaterialGroupIsFile)
        ));
        let partial = error
            .pre_landscape_components()
            .expect("failed landscape exposes the committed prefix");
        assert!(partial.game_txt.is_some());
        assert!(matches!(
            partial.landscape_mutations.as_slice(),
            [
                LiveC4SaveLandscapeMutation::DeleteEntry { name },
                LiveC4SaveLandscapeMutation::PutFile {
                    name: map_name,
                    payload
                }
            ] if name == "Landscape.bmp" && map_name == "Map.bmp" && !payload.is_empty()
        ));
        assert_eq!(engine.global_effects[0].command_target, None);
        assert_eq!(engine.objects[index].compiler_cache.action_target1, 777);
        assert_eq!(engine.objects[index].state.action.target, Some(off_list));
    }

    #[test]
    fn clean_texture_map_ignores_a_copied_material_file() {
        let mut raster = crate::landscape::LandscapeRasterState::new(
            1,
            0,
            crate::landscape::RuntimeTexMapState::default(),
        );
        raster.set_map(&IndexedBitmap {
            width: 2,
            height: 2,
            indices: vec![0; 4],
        });
        let mut landscape = crate::Landscape::flat(2, 2);
        assert!(landscape.set_mode(LANDSCAPE_MODE_STATIC));
        landscape.set_raster_state(raster);
        let mut engine = Engine::new();
        engine.set_landscape(landscape);

        let saved = serialize_landscape_for_policy(
            &engine,
            LiveC4SavePolicy::Scenario {
                force_exact_landscape: false,
            },
            true,
        )
        .expect("a clean texture map never opens the copied Material.c4g entry");
        assert!(saved.material_group.is_none());
    }

    #[test]
    fn c4fixed_fields_use_raw_f_syntax_and_elide_zero() {
        let mut writer = TextComponentWriter::default();
        field_fixed(&mut writer, 0, "FixX", 0);
        field_fixed(&mut writer, 0, "XDir", 65_536);
        field_fixed(&mut writer, 0, "RDir", -32_768);
        assert_eq!(writer.finish(), b"XDir=F65536\r\nRDir=F-32768\r\n");
    }

    #[test]
    fn runtime_scenario_uses_cpp_defaults_and_network_save_adjustments() {
        let modules = vec!["/OPT/GAME/Definitions/Objects.c4d".to_owned()];
        let bytes = serialize_scenario(
            &ScenarioValueStore::default(),
            LiveC4SaveSpec {
                title: "Runtime title",
                definition_modules: &modules,
                definition_executable_path: "/opt/game/",
                definition_path: "Definitions/",
                origin: "Folder\\Scenario.c4s",
                music_enabled: true,
                copied_material_group_is_file: false,
                title_component: LiveC4ComponentHost::Unmodified,
                info_component: LiveC4ComponentHost::Unmodified,
                script_component: LiveC4ComponentHost::Unmodified,
            },
        );
        for expected in [
            b"Title=Runtime title\r\n".as_slice(),
            b"Version=4,9,11\r\n",
            b"SaveGame=1\r\n",
            b"NoInitialize=1\r\n",
            b"NetworkGame=true\r\n",
            b"NetworkRuntimeJoin=true\r\n",
            b"ForcedGfxMode=1\r\n",
            b"Origin=Folder/Scenario.c4s\r\n",
            b"Definitions=\"Objects.c4d\"\r\n",
        ] {
            assert!(bytes
                .windows(expected.len())
                .any(|window| window == expected));
        }
        assert!(!bytes
            .windows(b"Icon=18".len())
            .any(|window| window == b"Icon=18"));
        assert!(!bytes
            .windows(b"MaxPlayer=12".len())
            .any(|window| window == b"MaxPlayer=12"));
    }

    #[test]
    fn save_policies_match_cpp_exactness_and_player_restore_switches() {
        let scenario = LiveC4SavePolicy::Scenario {
            force_exact_landscape: false,
        };
        let forced_scenario = LiveC4SavePolicy::Scenario {
            force_exact_landscape: true,
        };
        let savegame = LiveC4SavePolicy::Savegame {
            target_group_name: "Savegame.c4s",
        };
        let record = LiveC4SavePolicy::Record;
        let network = LiveC4SavePolicy::RuntimeNetwork;

        assert!(!scenario.is_exact());
        assert!(savegame.is_exact());
        assert!(record.is_exact());
        assert!(network.is_exact());
        assert!(!scenario.is_synchronized());
        assert!(!savegame.is_synchronized());
        assert!(record.is_synchronized());
        assert!(network.is_synchronized());
        assert!(!scenario.forces_runtime_landscape());
        assert!(forced_scenario.forces_runtime_landscape());
        assert!(savegame.landscape_diff_is_sync_save());
        assert!(!record.landscape_diff_is_sync_save());
        assert!(!network.landscape_diff_is_sync_save());
        assert_eq!(
            scenario.player_policy(),
            LiveC4SavePlayerPolicy {
                save_user_players: false,
                save_script_players: true,
                embed_user_player_files: false,
                embed_script_player_files: true,
            }
        );
        assert_eq!(
            savegame.player_policy(),
            LiveC4SavePlayerPolicy {
                save_user_players: true,
                save_script_players: true,
                embed_user_player_files: false,
                embed_script_player_files: true,
            }
        );
        assert_eq!(
            record.player_policy(),
            LiveC4SavePlayerPolicy {
                save_user_players: true,
                save_script_players: true,
                embed_user_player_files: true,
                embed_script_player_files: true,
            }
        );
        assert_eq!(
            network.player_policy(),
            LiveC4SavePlayerPolicy {
                save_user_players: true,
                save_script_players: true,
                embed_user_player_files: true,
                embed_script_player_files: true,
            }
        );
    }

    #[test]
    fn scenario_and_savegame_headers_use_their_cpp_savecore_policies() {
        let modules = vec!["/opt/game/Definitions/Objects.c4d".to_owned()];
        let spec = LiveC4SaveSpec {
            title: "Runtime title",
            definition_modules: &modules,
            definition_executable_path: "/opt/game/",
            definition_path: "Definitions/",
            origin: "Folder\\Scenario.c4s",
            music_enabled: true,
            copied_material_group_is_file: false,
            title_component: LiveC4ComponentHost::Unmodified,
            info_component: LiveC4ComponentHost::Unmodified,
            script_component: LiveC4ComponentHost::Unmodified,
        };
        let scenario = serialize_scenario_for_policy(
            &ScenarioValueStore::default(),
            spec,
            LiveC4SavePolicy::Scenario {
                force_exact_landscape: false,
            },
        );
        for expected in [
            b"Version=4,9,11\r\n".as_slice(),
            b"NoInitialize=1\r\n",
            b"ForcedGfxMode=1\r\n",
        ] {
            assert!(scenario
                .windows(expected.len())
                .any(|window| window == expected));
        }
        for absent in [
            b"Title=Runtime title\r\n".as_slice(),
            b"SaveGame=1\r\n",
            b"NetworkGame=true\r\n",
            b"NetworkRuntimeJoin=true\r\n",
            b"Origin=Folder/Scenario.c4s\r\n",
            b"Definitions=\"Objects.c4d\"\r\n",
        ] {
            assert!(!scenario
                .windows(absent.len())
                .any(|window| window == absent));
        }

        let savegame = serialize_scenario_for_policy(
            &ScenarioValueStore::default(),
            spec,
            LiveC4SavePolicy::Savegame {
                target_group_name: "/saves/Slot3.c4s",
            },
        );
        for expected in [
            b"Title=Runtime title\r\n".as_slice(),
            b"SaveGame=1\r\n",
            b"NoInitialize=1\r\n",
            b"Icon=4\r\n",
            b"Origin=Folder/Scenario.c4s\r\n",
            b"Definitions=\"Objects.c4d\"\r\n",
        ] {
            assert!(savegame
                .windows(expected.len())
                .any(|window| window == expected));
        }
        assert!(!savegame
            .windows(b"NetworkGame=true\r\n".len())
            .any(|window| window == b"NetworkGame=true\r\n"));
    }

    #[test]
    fn savegame_icon_follows_cpp_trailing_slot_number() {
        assert_eq!(savegame_icon("Save1.c4s"), 2);
        assert_eq!(savegame_icon("C:\\Games\\Save10.c4s"), 11);
        assert_eq!(savegame_icon("/tmp/Save11.c4s"), 29);
        assert_eq!(savegame_icon("Save.c4s"), 29);
    }

    #[test]
    fn runtime_record_header_uses_cpp_record_adjustments() {
        let modules = vec!["/opt/game/Definitions/Objects.c4d".to_owned()];
        let scenario = serialize_scenario_for_policy(
            &ScenarioValueStore::default(),
            LiveC4SaveSpec {
                title: "007 Runtime title [362]",
                definition_modules: &modules,
                definition_executable_path: "/opt/game/",
                definition_path: "Definitions/",
                origin: "Folder\\Scenario.c4s",
                music_enabled: true,
                copied_material_group_is_file: false,
                title_component: LiveC4ComponentHost::Unmodified,
                info_component: LiveC4ComponentHost::Unmodified,
                script_component: LiveC4ComponentHost::Unmodified,
            },
            LiveC4SavePolicy::Record,
        );
        for expected in [
            b"Title=007 Runtime title [362]\r\n".as_slice(),
            b"Icon=29\r\n",
            b"SaveGame=1\r\n",
            b"Replay=1\r\n",
            b"NoInitialize=1\r\n",
            b"Origin=Folder/Scenario.c4s\r\n",
            b"Definitions=\"Objects.c4d\"\r\n",
        ] {
            assert!(scenario
                .windows(expected.len())
                .any(|window| window == expected));
        }
        assert!(!scenario
            .windows(b"NetworkGame=true\r\n".len())
            .any(|window| window == b"NetworkGame=true\r\n"));
    }

    #[test]
    fn modified_main_section_replaces_only_objects_and_filters_to_c4fls_section() {
        let mut source = MutableGroup::new("Source.c4s");
        source.set_maker("Root Scenario Maker");
        source
            .add_file("Scenario.txt", b"original scenario".to_vec())
            .unwrap();
        source
            .add_file("Game.txt", b"original game".to_vec())
            .unwrap();
        source
            .add_file("Sky.png", b"original sky".to_vec())
            .unwrap();
        source
            .add_file("CtrlRec.c4b", b"original control".to_vec())
            .unwrap();
        source
            .add_file("Strings.txt", b"old string".to_vec())
            .unwrap();
        source
            .add_file("Objects.txt", b"old object".to_vec())
            .unwrap();
        source
            .add_file("Custom.bin", b"not a main section component".to_vec())
            .unwrap();
        let source =
            Group::from_raw_memory(PathBuf::from("Source.c4s"), source.pack_raw().unwrap())
                .unwrap();

        let mut engine = Engine::new();
        engine.configure_scenario_sections(&[
            section_spec("main", Some(source)),
            section_spec("elsewhere", None),
        ]);
        assert!(engine
            .load_scenario_section("elsewhere", 0, Vec::new())
            .expect("first section switch succeeds"));
        let section = engine.scenario_sections.get_mut("main").unwrap();
        section.modified = true;
        section.objects_modified = true;
        section.saved_objects = Some(Vec::new());

        let serialized = serialize_scenario_sections(&engine, &mut LegacyStringTable::default()).0;
        assert_eq!(serialized.len(), 1);
        let group =
            Group::from_raw_memory(PathBuf::from("Sectmain.c4g"), serialized[0].payload.clone())
                .unwrap();
        assert_eq!(
            group.read_file("Scenario.txt").unwrap(),
            b"original scenario"
        );
        assert_eq!(group.read_file("Game.txt").unwrap(), b"original game");
        assert_eq!(group.read_file("Sky.png").unwrap(), b"original sky");
        assert_eq!(group.read_file("CtrlRec.c4b").unwrap(), b"original control");
        assert_eq!(group.read_file("Objects.txt").unwrap(), b"");
        assert_eq!(group.maker(), Some("New C4Group"));
        assert!(!group.exists("Strings.txt"));
        assert!(!group.exists("Custom.bin"));
    }

    #[test]
    fn scenario_section_mutations_follow_native_prepend_order() {
        let raw_section = |name: &str| {
            let mut group = MutableGroup::new(format!("Sect{name}.c4g"));
            group
                .add_file("Objects.txt", name.as_bytes().to_vec())
                .unwrap();
            group.pack_raw().unwrap()
        };
        let mut engine = Engine::new();
        engine.configure_scenario_sections(&[
            section_spec("main", None),
            section_spec("Alpha", None),
            section_spec("beta", None),
            section_spec("Gamma", None),
        ]);
        assert!(engine
            .load_scenario_section("beta", 2, Vec::new())
            .expect("first section switch succeeds"));
        for name in ["alpha", "gamma"] {
            let section = engine.scenario_sections.get_mut(name).unwrap();
            section.modified = true;
            section.frozen_group = Some(raw_section(&section.name));
        }

        let (serialized, deleted, mutations) =
            serialize_scenario_sections(&engine, &mut LegacyStringTable::default());

        assert_eq!(
            serialized
                .iter()
                .map(|section| section.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Sectmain.c4g", "SectGamma.c4g", "SectAlpha.c4g"]
        );
        assert!(deleted.is_empty());
        assert!(matches!(
            &mutations[..],
            [
                LiveC4SaveScenarioSectionMutation::Replace(LiveC4SaveNamedComponent { name: main, .. }),
                LiveC4SaveScenarioSectionMutation::Replace(LiveC4SaveNamedComponent { name: gamma, .. }),
                LiveC4SaveScenarioSectionMutation::Delete { name: beta },
                LiveC4SaveScenarioSectionMutation::Replace(LiveC4SaveNamedComponent { name: alpha, .. }),
            ] if main == "Sectmain.c4g"
                && gamma == "SectGamma.c4g"
                && beta == "Sectbeta.c4g"
                && alpha == "SectAlpha.c4g"
        ));
    }

    #[test]
    fn exact_save_before_first_section_switch_has_no_current_pointer_delete() {
        let mut engine = Engine::new();
        engine.configure_scenario_sections(&[
            section_spec("main", None),
            section_spec("Other", None),
        ]);

        let (_, _, mutations) =
            serialize_scenario_sections(&engine, &mut LegacyStringTable::default());

        assert!(mutations.is_empty());
        assert!(!engine.debug_current_scenario_section_exists());
    }

    #[test]
    fn persisted_section_contents_resolve_against_the_section_object_list() {
        let mut source = MutableGroup::new("Sectarchive.c4g");
        source
            .add_file("Scenario.txt", b"section scenario".to_vec())
            .unwrap();
        let source =
            Group::from_raw_memory(PathBuf::from("Sectarchive.c4g"), source.pack_raw().unwrap())
                .unwrap();

        let mut engine = Engine::new();
        engine
            .register_definition(
                crate::Definition::from_script("TST1", "Section object", "").unwrap(),
            )
            .unwrap();
        let container = engine
            .spawn_object(crate::SpawnConfig::new("TST1").with_id(ObjectId::new(11)))
            .unwrap();
        let content = engine
            .spawn_object(crate::SpawnConfig::new("TST1").with_id(ObjectId::new(12)))
            .unwrap();
        let container_index = engine.find_object_index(container).unwrap();
        let content_index = engine.find_object_index(content).unwrap();
        engine.objects[container_index].state.contents = vec![content];
        engine.objects[content_index].state.container = Some(container);
        let persisted = engine.capture_state().objects;

        // Named sections own an independent object list after a switch. The
        // root list may no longer contain either allocation when the section
        // is saved.
        engine.objects.clear();
        engine.exec_list.clear();
        engine.inactive_exec_list.clear();
        engine.configure_scenario_sections(&[
            section_spec("main", None),
            section_spec("archive", Some(source)),
        ]);
        let section = engine.scenario_sections.get_mut("archive").unwrap();
        section.modified = true;
        section.objects_modified = true;
        section.saved_object_order = vec![container, content];
        section.saved_objects = Some(persisted);

        let serialized = serialize_scenario_sections(&engine, &mut LegacyStringTable::default()).0;
        let group = Group::from_raw_memory(
            PathBuf::from("Sectarchive.c4g"),
            serialized[0].payload.clone(),
        )
        .unwrap();
        let objects = String::from_utf8(group.read_file("Objects.txt").unwrap()).unwrap();

        assert!(objects.contains("Number=11\r\n"));
        assert!(objects.contains("Contents=12\r\n"));
    }

    #[test]
    fn no_sky_exact_section_save_deletes_only_extensionless_sky() {
        let mut source = MutableGroup::new("Sectnight.c4g");
        source
            .add_file("Scenario.txt", b"old scenario".to_vec())
            .unwrap();
        source.add_file("Sky", b"legacy sky".to_vec()).unwrap();
        source.add_file("Sky.bmp", b"old bitmap".to_vec()).unwrap();
        source.add_file("Sky.png", b"old png".to_vec()).unwrap();
        source.add_file("Sky.jpeg", b"old jpeg".to_vec()).unwrap();
        source.add_file("Sky.jpg", b"old jpg".to_vec()).unwrap();
        let source =
            Group::from_raw_memory(PathBuf::from("Sectnight.c4g"), source.pack_raw().unwrap())
                .unwrap();

        let mut spec = section_spec("night", Some(source));
        spec.scenario_values = ScenarioValueStore::with_no_sky_for_test(true);
        let mut landscape = crate::Landscape::flat(1, 1);
        assert!(landscape.set_mode(LANDSCAPE_MODE_EXACT));
        landscape.set_pixel_grid(crate::landscape::PixelGrid::new(
            1,
            1,
            vec![0],
            vec![0; 256],
            vec![None; 256],
            vec![None; 256],
        ));
        spec.landscape = Some(landscape);
        let mut engine = Engine::new();
        engine.configure_scenario_sections(&[section_spec("main", None), spec]);
        let section = engine.scenario_sections.get_mut("night").unwrap();
        section.modified = true;
        section.landscape_modified = true;

        let serialized = serialize_scenario_sections(&engine, &mut LegacyStringTable::default()).0;
        let group = Group::from_raw_memory(
            PathBuf::from("Sectnight.c4g"),
            serialized[0].payload.clone(),
        )
        .unwrap();
        assert!(!group.exists("Sky"));
        for name in ["Sky.bmp", "Sky.png", "Sky.jpeg", "Sky.jpg"] {
            assert!(group.exists(name), "{name} was over-deleted by NoSky");
        }
    }

    #[test]
    fn section_switch_freezes_the_departing_group_before_the_root_save() {
        let mut main = MutableGroup::new("Source.c4s");
        main.add_file("Scenario.txt", b"main scenario".to_vec())
            .unwrap();
        main.add_file("Strings.txt", b"old string\r\n".to_vec())
            .unwrap();
        main.add_file("Objects.txt", b"old object".to_vec())
            .unwrap();
        main.add_file("Custom.bin", b"not a section component".to_vec())
            .unwrap();
        let main =
            Group::from_raw_memory(PathBuf::from("Source.c4s"), main.pack_raw().unwrap()).unwrap();
        let mut next = MutableGroup::new("Sectnext.c4g");
        next.add_file("Scenario.txt", b"next scenario".to_vec())
            .unwrap();
        let next = Group::from_raw_memory(PathBuf::from("Sectnext.c4g"), next.pack_raw().unwrap())
            .unwrap();

        let mut engine = Engine::new();
        engine.configure_scenario_sections(&[
            section_spec("main", Some(main)),
            section_spec("next", Some(next)),
        ]);
        assert!(engine
            .load_scenario_section("next", 2, Vec::new())
            .expect("section switch succeeds"));
        let frozen = engine
            .scenario_sections
            .get("main")
            .and_then(|section| section.frozen_group.clone())
            .expect("departing main group freezes");

        // Prove final serialization is a byte copy, not a reconstruction
        // from the mutable retained section model.
        let section = engine.scenario_sections.get_mut("main").unwrap();
        section.source_group = None;
        section.saved_objects = None;
        section.initial_objects.clear();
        section.landscape_modified = true;
        let serialized = serialize_scenario_sections(&engine, &mut LegacyStringTable::default()).0;
        assert_eq!(serialized.len(), 1);
        assert_eq!(serialized[0].payload, frozen);

        let group = Group::from_raw_memory(PathBuf::from("Sectmain.c4g"), frozen).unwrap();
        assert_eq!(group.read_file("Objects.txt").unwrap(), b"");
        assert!(!group.exists("Custom.bin"));
    }

    #[test]
    fn non_main_named_implicit_root_freeze_extracts_only_section_components() {
        let mut source = MutableGroup::new("Source.c4s");
        source.set_maker("Root Scenario Maker");
        source
            .add_file("Scenario.txt", b"root scenario".to_vec())
            .unwrap();
        source
            .add_file("Custom.bin", b"root-only payload".to_vec())
            .unwrap();
        let source =
            Group::from_raw_memory(PathBuf::from("Source.c4s"), source.pack_raw().unwrap())
                .unwrap();

        let mut engine = Engine::new();
        engine.configure_scenario_sections(&[section_spec("Cave", Some(source))]);
        let section = engine.scenario_sections.get("cave").unwrap();
        let frozen = freeze_scenario_section(&engine, section, false, false).unwrap();
        let group = Group::from_raw_memory(PathBuf::from("SectCave.c4g"), frozen).unwrap();

        assert_eq!(group.read_file("Scenario.txt").unwrap(), b"root scenario");
        assert!(!group.exists("Custom.bin"));
        assert_eq!(group.maker(), Some("New C4Group"));
    }

    #[test]
    fn named_main_section_freeze_preserves_its_complete_source_group() {
        let mut root = MutableGroup::new("Source.c4s");
        root.add_file("Scenario.txt", b"root scenario".to_vec())
            .unwrap();
        let root =
            Group::from_raw_memory(PathBuf::from("Source.c4s"), root.pack_raw().unwrap()).unwrap();

        let mut named_main = MutableGroup::new("SectMain.c4g");
        named_main.set_maker("Named Section Maker");
        named_main
            .add_file("Scenario.txt", b"named main scenario".to_vec())
            .unwrap();
        named_main
            .add_file("Custom.bin", b"named section payload".to_vec())
            .unwrap();
        let named_main = Group::from_raw_memory(
            PathBuf::from("SectMain.c4g"),
            named_main.pack_raw().unwrap(),
        )
        .unwrap();

        let mut engine = Engine::new();
        engine.configure_scenario_sections(&[
            section_spec("Cave", Some(root)),
            section_spec("Main", Some(named_main)),
        ]);
        let section = engine.scenario_sections.get("main").unwrap();
        let frozen = freeze_scenario_section(&engine, section, false, false).unwrap();
        let group = Group::from_raw_memory(PathBuf::from("SectMain.c4g"), frozen).unwrap();

        assert_eq!(
            group.read_file("Scenario.txt").unwrap(),
            b"named main scenario"
        );
        assert_eq!(
            group.read_file("Custom.bin").unwrap(),
            b"named section payload"
        );
        assert_eq!(group.maker(), Some("Named Section Maker"));
    }

    #[test]
    fn section_switch_saves_strings_before_object_reenumeration() {
        let mut main = MutableGroup::new("Source.c4s");
        main.add_file("Scenario.txt", b"main scenario".to_vec())
            .unwrap();
        let main =
            Group::from_raw_memory(PathBuf::from("Source.c4s"), main.pack_raw().unwrap()).unwrap();
        let mut next = MutableGroup::new("Sectnext.c4g");
        next.add_file("Scenario.txt", b"next scenario".to_vec())
            .unwrap();
        let next = Group::from_raw_memory(PathBuf::from("Sectnext.c4g"), next.pack_raw().unwrap())
            .unwrap();

        let mut engine = Engine::new();
        engine.set_legacy_string_table(HashMap::from([(0, "loaded".to_owned())]));
        engine.script_global_consts.borrow_mut().insert(
            "RuntimeValue".to_owned(),
            clonk_script::value_cell(Value::String("created later".to_owned().into())),
        );
        engine.configure_scenario_sections(&[
            section_spec("main", Some(main)),
            section_spec("next", Some(next)),
        ]);

        assert!(engine
            .load_scenario_section("next", 2, Vec::new())
            .expect("section switch succeeds"));
        let frozen = engine
            .scenario_sections
            .get("main")
            .and_then(|section| section.frozen_group.clone())
            .expect("departing main group freezes");
        let group = Group::from_raw_memory(PathBuf::from("Sectmain.c4g"), frozen).unwrap();

        // LoadScenarioSection saves the table while the runtime string still
        // has iEnumID=-1. Objects.Save enumerates it only after this payload.
        assert_eq!(group.read_file("Strings.txt").unwrap(), b"loaded\r\n");
        let referenced = collect_live_referenced_strings(&engine, &engine.capture_state());
        assert_eq!(
            clonk_script::save_current_c4_string_enumeration(
                &engine.script_string_registrations,
                &referenced,
            ),
            [b"loaded".to_vec(), b"created later".to_vec()]
        );
    }

    #[test]
    fn section_string_save_noop_preserves_existing_component() {
        let mut main = MutableGroup::new("Source.c4s");
        main.add_file("Scenario.txt", b"main scenario".to_vec())
            .unwrap();
        main.add_file("Strings.txt", b"stale\r\n".to_vec()).unwrap();
        let main =
            Group::from_raw_memory(PathBuf::from("Source.c4s"), main.pack_raw().unwrap()).unwrap();

        let mut engine = Engine::new();
        engine.configure_scenario_sections(&[section_spec("main", Some(main))]);
        let section = engine.scenario_sections.get("main").unwrap();
        let frozen = freeze_scenario_section(&engine, section, false, true).unwrap();
        let group = Group::from_raw_memory(PathBuf::from("Sectmain.c4g"), frozen).unwrap();

        assert_eq!(group.read_file("Strings.txt").unwrap(), b"stale\r\n");
    }

    #[test]
    fn inactive_section_values_do_not_pollute_the_live_string_table() {
        let mut inactive = section_spec("inactive", None);
        inactive.objects.push(crate::scenario::ScenarioSpawn {
            handle: None,
            container_handle: None,
            contents_handles: Vec::new(),
            info_name: None,
            config: crate::SpawnConfig::new("TEST").with_local_vars(HashMap::from([(
                "value".to_owned(),
                Value::String("section-only".to_owned().into()),
            )])),
        });
        let mut engine = Engine::new();
        engine.configure_scenario_sections(&[inactive]);

        let strings = collect_live_referenced_strings(&engine, &engine.capture_state());
        assert!(!strings.iter().any(|value| value.as_ref() == "section-only"));

        // While freezing a modified section, its objects temporarily
        // participate in enumeration; after it owns a frozen group they do
        // not participate in the root save again.
        let section = engine.scenario_sections.get_mut("inactive").unwrap();
        section.modified = true;
        let strings = collect_live_referenced_strings(&engine, &engine.capture_state());
        assert!(strings.iter().any(|value| value.as_ref() == "section-only"));
        engine
            .scenario_sections
            .get_mut("inactive")
            .unwrap()
            .frozen_group = Some(Vec::new());
        let strings = collect_live_referenced_strings(&engine, &engine.capture_state());
        assert!(!strings.iter().any(|value| value.as_ref() == "section-only"));
    }

    #[test]
    fn player_whole_line_strings_are_raw_not_quoted() {
        let player = PlayerState {
            id: -1,
            at_client_name: Some("client name \"raw\"".to_owned()),
            message_buf: " message \\ raw ".to_owned(),
            ..PlayerState::default()
        };
        let bytes = serialize_players(&Engine::new(), &[player], &mut LegacyStringTable::default());
        assert!(bytes
            .windows(b"AtClientName=client name \"raw\"\r\n".len())
            .any(|window| window == b"AtClientName=client name \"raw\"\r\n"));
        assert!(bytes
            .windows(b"MessageBuf= message \\ raw \r\n".len())
            .any(|window| window == b"MessageBuf= message \\ raw \r\n"));
    }

    #[test]
    fn runtime_player_serializes_saved_view_center_independently_of_viewports() {
        let no_viewport = PlayerState {
            id: 1,
            view_center: Some(crate::Vector2::new(321, 654)),
            ..PlayerState::default()
        };
        let conflicting_viewport = PlayerState {
            id: 2,
            view_center: Some(crate::Vector2::new(777, 888)),
            viewports: vec![crate::PlayerViewport::new(crate::Vector2::new(11, 22))],
            ..PlayerState::default()
        };

        let bytes = serialize_players(
            &Engine::new(),
            &[no_viewport, conflicting_viewport],
            &mut LegacyStringTable::default(),
        );
        for expected in [
            b"ViewX=321\r\n".as_slice(),
            b"ViewY=654\r\n".as_slice(),
            b"ViewX=777\r\n".as_slice(),
            b"ViewY=888\r\n".as_slice(),
        ] {
            assert!(bytes
                .windows(expected.len())
                .any(|window| window == expected));
        }
        assert!(!bytes
            .windows(b"ViewX=11\r\n".len())
            .any(|window| window == b"ViewX=11\r\n"));
        assert!(!bytes
            .windows(b"ViewY=22\r\n".len())
            .any(|window| window == b"ViewY=22\r\n"));
    }

    #[test]
    fn runtime_player_emits_show_startup_exactly_once() {
        let player = PlayerState {
            show_startup: true,
            ..PlayerState::default()
        };
        let bytes = serialize_players(&Engine::new(), &[player], &mut LegacyStringTable::default());
        assert_eq!(
            bytes
                .windows(b"ShowStartup=true\r\n".len())
                .filter(|window| *window == b"ShowStartup=true\r\n")
                .count(),
            1
        );
    }

    #[test]
    fn runtime_player_integer_flags_survive_without_boolean_normalization() {
        let state = PlayerState {
            player_info_id: 7,
            status: PlayerStatus::Surrendered,
            status_value: Some(-7),
            surrendered: true,
            surrendered_value: -2,
            eliminated_value: -9,
            evaluated: true,
            control: crate::PlayerControlState {
                auto_context_menu: true,
                auto_context_menu_value: 7,
                control_style: true,
                control_style_value: -3,
                ..crate::PlayerControlState::default()
            },
            ..PlayerState::default()
        };

        let restored = crate::Player::from_state(state).to_state();
        assert_eq!(restored.status_value, Some(-7));
        assert_eq!(restored.eliminated_value, -9);
        assert_eq!(restored.surrendered_value, -2);
        assert_eq!(restored.control.auto_context_menu_value, 7);
        assert_eq!(restored.control.control_style_value, -3);

        let bytes = serialize_players(
            &Engine::new(),
            &[restored],
            &mut LegacyStringTable::default(),
        );
        for expected in [
            b"Status=-7\r\n".as_slice(),
            b"Eliminated=-9\r\n",
            b"Surrendered=-2\r\n".as_slice(),
            b"Evaluated=true\r\n",
            b"AutoContextMenu=7\r\n",
            b"AutoStopControl=-3\r\n",
        ] {
            assert!(bytes
                .windows(expected.len())
                .any(|window| window == expected));
        }
    }

    #[test]
    fn runtime_player_signed_counters_preserve_high_bits() {
        let player = PlayerState {
            objects_owned: u32::MAX,
            production_delay: 0x8000_0000,
            production_unit: 0xffff_fffe,
            ..PlayerState::default()
        };
        let text = String::from_utf8(serialize_players(
            &Engine::new(),
            &[player],
            &mut LegacyStringTable::default(),
        ))
        .expect("Game.txt player section is UTF-8");

        for (name, expected, original) in [
            ("ObjectsOwned", "-1", u32::MAX),
            ("ProductionDelay", "-2147483648", 0x8000_0000),
            ("ProductionUnit", "-2", 0xffff_fffe),
        ] {
            let prefix = format!("{name}=");
            let serialized = text
                .lines()
                .find_map(|line| line.strip_prefix(prefix.as_str()))
                .unwrap_or_else(|| panic!("{name} is serialized"));
            assert_eq!(serialized, expected);
            assert_eq!(
                serialized.parse::<i32>().expect("native signed field") as u32,
                original
            );
        }
    }

    #[test]
    fn object_info_name_uses_the_raw_whole_line_adapter() {
        let mut writer = TextComponentWriter::default();
        serialize_object_info_name(&mut writer, "Sir Clonk \"III\"");
        assert_eq!(writer.finish(), b"Info=Sir Clonk \"III\"\r\n");
    }

    #[test]
    fn round_result_buffers_use_escaped_strings_but_result_enum_does_not() {
        let results = RoundResultsState {
            goal_counts: vec![("ZERO".to_owned(), 0), ("DEBT".to_owned(), -2)],
            network_result: Some(RoundResultsNetworkResult::LeagueError),
            network_result_message: b"bad \\\"line\n\x80".to_vec(),
            players: vec![crate::round_results::RoundResultsPlayerState {
                status: RoundResultsPlayerStatus::Won,
                player_info_id: 7,
                league_progress_data: Some(b"p\\\"\r\n\x81".to_vec()),
                ..crate::round_results::RoundResultsPlayerState::default()
            }],
            ..RoundResultsState::default()
        };
        let bytes = serialize_round_results(&results, false).expect("nonempty results");
        assert!(bytes
            .windows(b"Goals=ZERO=0;DEBT=-2\r\n".len())
            .any(|window| window == b"Goals=ZERO=0;DEBT=-2\r\n"));
        assert!(bytes
            .windows(b"LeagueProgressData=\"p\\\\\\\"\\r\\n\\201\"\r\n".len())
            .any(|window| window == b"LeagueProgressData=\"p\\\\\\\"\\r\\n\\201\"\r\n"));
        assert!(bytes
            .windows(b"Status=Won\r\n".len())
            .any(|window| window == b"Status=Won\r\n"));
        assert!(bytes
            .windows(b"NetResult=\"bad \\\\\\\"line\\n\\200\"\r\nNetResult=LeagueError\r\n".len())
            .any(|window| {
                window == b"NetResult=\"bad \\\\\\\"line\\n\\200\"\r\nNetResult=LeagueError\r\n"
            }));
    }

    #[test]
    fn round_results_emptiness_uses_only_compiled_fields_and_melee_default() {
        let fulfilled_only = RoundResultsState {
            fulfilled_goals: vec!["SCRG".to_owned()],
            ..RoundResultsState::default()
        };
        assert_eq!(serialize_round_results(&fulfilled_only, false), None);

        let melee_override = RoundResultsState::default();
        assert_eq!(
            serialize_round_results(&melee_override, true).unwrap(),
            b"[RoundResults]\r\nHideSettlementScore=false\r\n"
        );
    }

    #[test]
    fn section_object_filter_skips_user_crew_but_keeps_script_crew() {
        let mut engine = Engine::new();
        engine
            .register_definition(
                crate::Definition::from_script("CLNK", "Crew", "")
                    .expect("fixture definition compiles"),
            )
            .expect("fixture definition registers");
        engine
            .register_player(crate::PlayerConfig::new(1, "User"))
            .expect("user registers");
        engine
            .register_player(crate::PlayerConfig::new(2, "Script"))
            .expect("script player registers");
        engine
            .player_mut(2)
            .expect("script player")
            .set_script_player(true);
        let user_object = engine
            .spawn_object(crate::SpawnConfig::new("CLNK").with_owner(1))
            .expect("user crew spawns");
        let script_object = engine
            .spawn_object(crate::SpawnConfig::new("CLNK").with_owner(2))
            .expect("script crew spawns");
        engine
            .player_mut(1)
            .expect("user")
            .set_crew(vec![user_object]);
        engine
            .player_mut(2)
            .expect("script player")
            .set_crew(vec![script_object]);
        let state = engine.capture_state();
        let bytes = serialize_persisted_objects(
            &engine,
            &state.objects,
            &state.object_order,
            &mut LegacyStringTable::default(),
        );
        let user_number = format!("Number={user_object}\r\n");
        let script_number = format!("Number={script_object}\r\n");
        assert!(!bytes
            .windows(user_number.len())
            .any(|window| window == user_number.as_bytes()));
        assert!(bytes
            .windows(script_number.len())
            .any(|window| window == script_number.as_bytes()));
    }

    #[test]
    fn saved_scenario_objects_skip_user_crew_but_keep_script_crew() {
        let mut engine = Engine::new();
        engine
            .register_definition(
                crate::Definition::from_script("CLNK", "Crew", "")
                    .expect("fixture definition compiles"),
            )
            .expect("fixture definition registers");
        engine
            .register_player(crate::PlayerConfig::new(1, "User"))
            .expect("user registers");
        engine
            .register_player(crate::PlayerConfig::new(2, "Script"))
            .expect("script player registers");
        engine
            .player_mut(2)
            .expect("script player")
            .set_script_player(true);
        let user_object = engine
            .spawn_object(crate::SpawnConfig::new("CLNK").with_owner(1))
            .expect("user crew spawns");
        let script_object = engine
            .spawn_object(crate::SpawnConfig::new("CLNK").with_owner(2))
            .expect("script crew spawns");
        engine
            .player_mut(1)
            .expect("user")
            .set_crew(vec![user_object]);
        engine
            .player_mut(2)
            .expect("script player")
            .set_crew(vec![script_object]);

        let bytes = serialize_objects_for_save(&engine, &mut LegacyStringTable::default(), true);
        let user_number = format!("Number={user_object}\r\n");
        let script_number = format!("Number={script_object}\r\n");
        assert!(!bytes
            .windows(user_number.len())
            .any(|window| window == user_number.as_bytes()));
        assert!(bytes
            .windows(script_number.len())
            .any(|window| window == script_number.as_bytes()));
    }

    #[test]
    fn section_object_snapshot_retains_every_private_compiler_field() {
        let mut engine = Engine::new();
        engine
            .register_definition(
                crate::Definition::from_script("ROCK", "Rock", "")
                    .expect("fixture definition compiles"),
            )
            .expect("fixture definition registers");
        let id = engine
            .spawn_object(crate::SpawnConfig::new("ROCK"))
            .expect("fixture object spawns");
        let index = engine.find_object_index(id).expect("object exists");
        let object = &mut engine.objects[index];
        object.state.no_collect_delay = 9;
        object.state.entrance_status = true;
        object.state.crew_disabled = true;
        object.state.shape_attach = crate::ShapeAttachRecord {
            mat_valid: true,
            mat_vehicle: false,
            x: 12,
            y: -3,
            vtx: 2,
        };
        object.compiler_cache = crate::ObjectCompilerCache {
            info: "Cached Crew".to_owned(),
            contained: 42,
            action_target1: -7,
            action_target2: 1_000_000_042,
            layer: 9,
        };
        object.last_attach_movement_frame = 77;

        let state = engine.capture_state();
        let bytes = serialize_persisted_objects(
            &engine,
            &state.objects,
            &state.object_order,
            &mut LegacyStringTable::default(),
        );
        for expected in [
            b"Info=Cached Crew\r\n".as_slice(),
            b"LastSolidAtchFrame=77\r\n".as_slice(),
            b"Contained=42\r\n",
            b"ActionTarget1=-7\r\n",
            b"ActionTarget2=1000000042\r\n",
            b"Layer=9\r\n",
            b"NoCollectDelay=9\r\n",
            b"AttachX=12\r\n",
            b"AttachY=-3\r\n",
            b"AttachVtx=2\r\n",
            b"EntranceStatus=true\r\n",
            b"CrewDisabled=true\r\n",
        ] {
            assert!(bytes
                .windows(expected.len())
                .any(|window| window == expected));
        }
    }

    #[test]
    fn section_object_writer_uses_the_loaded_mass_cache() {
        let mut definition =
            crate::Definition::from_script("MASS", "Mass", "").expect("definition compiles");
        definition.set_mass(100);
        let mut engine = Engine::new();
        engine
            .register_definition(definition)
            .expect("definition registers");
        let mut config = crate::SpawnConfig::new("MASS").with_loaded(true);
        config.compiled_mass = Some(777);
        engine.spawn_object(config).expect("loaded object spawns");

        let state = engine.capture_state();
        let bytes = serialize_persisted_objects(
            &engine,
            &state.objects,
            &state.object_order,
            &mut LegacyStringTable::default(),
        );
        assert!(bytes
            .windows(b"Mass=777\r\n".len())
            .any(|window| window == b"Mass=777\r\n"));
    }

    #[test]
    fn section_object_mass_has_no_nesting_depth_cutoff() {
        let mut definition =
            crate::Definition::from_script("MASS", "Mass", "").expect("definition compiles");
        definition.set_mass(10);
        let mut engine = Engine::new();
        engine
            .register_definition(definition)
            .expect("definition registers");

        let root = engine
            .spawn_object(crate::SpawnConfig::new("MASS"))
            .expect("root object spawns");
        let mut parent = root;
        for _ in 1..12 {
            parent = engine
                .spawn_object(crate::SpawnConfig::new("MASS").with_container(parent))
                .expect("nested object spawns");
        }

        let state = engine.capture_state();
        let objects = state
            .objects
            .iter()
            .filter_map(|object| restored_section_object(&engine, object))
            .collect::<Vec<_>>();
        let root_index = objects
            .iter()
            .position(|object| object.id == root)
            .expect("root is restored for section serialization");
        assert_eq!(
            section_object_mass(&engine, &objects, root_index, &mut HashSet::new()),
            120
        );
    }

    #[test]
    fn object_writer_omits_pointer_defaults_but_keeps_real_overrides() {
        let default_mask = crate::DefinitionTargetRect::new(0, 0, 4, 3, 1, 2);
        let mut definition =
            crate::Definition::from_script("MASK", "Mask", "").expect("definition compiles");
        definition.set_solid_mask(Some(default_mask));
        let mut engine = Engine::new();
        engine
            .register_definition(definition)
            .expect("definition registers");
        let id = engine
            .spawn_object(crate::SpawnConfig::new("MASK"))
            .expect("object spawns");
        let index = engine.find_object_index(id).expect("object exists");
        engine.objects[index].state.custom_name = Some(String::new());
        engine.objects[index].state.solid_mask_override = Some(default_mask);
        engine.objects[index].state.base_graphics = Some(crate::ObjectBaseGraphics {
            definition: "MASK".to_owned(),
            graphics_name: None,
            blit_mode: 0,
        });

        let defaults = String::from_utf8(serialize_objects(
            &engine,
            &mut LegacyStringTable::default(),
        ))
        .expect("Objects.txt is UTF-8");
        assert!(!defaults.lines().any(|line| line.starts_with("Name=")));
        assert!(!defaults.lines().any(|line| line.starts_with("SolidMask=")));
        assert!(!defaults.lines().any(|line| line.starts_with("Graphics=")));

        engine.objects[index].state.custom_name = Some("Named".to_owned());
        engine.objects[index].state.solid_mask_override =
            Some(crate::DefinitionTargetRect::new(1, 0, 4, 3, 1, 2));
        engine.objects[index].state.base_graphics = Some(crate::ObjectBaseGraphics {
            definition: "MASK".to_owned(),
            graphics_name: Some("Alternate".to_owned()),
            blit_mode: 0,
        });
        let overrides = String::from_utf8(serialize_objects(
            &engine,
            &mut LegacyStringTable::default(),
        ))
        .expect("Objects.txt is UTF-8");
        assert!(overrides.lines().any(|line| line == "Name=\"Named\""));
        assert!(overrides.lines().any(|line| line.starts_with("SolidMask=")));
        assert_eq!(
            overrides.lines().find(|line| line.starts_with("Graphics=")),
            Some("Graphics=MASK::Alternate")
        );
    }

    #[test]
    fn teams_emit_retained_compiler_metadata_with_escaped_script_names() {
        let bytes = serialize_teams(
            &[],
            TeamConfiguration::default(),
            9,
            3,
            b"Bot\\\"One\n\x80",
            -2,
        )
        .expect("team list serializes");
        assert!(bytes
            .windows(b"LastTeamID=9\r\n".len())
            .any(|window| window == b"LastTeamID=9\r\n"));
        assert!(bytes
            .windows(b"MaxScriptPlayers=3\r\nScriptPlayerNames=\"Bot\\\\\\\"One\\n\\200\"\r\nRandomTeamCount=-2\r\n".len())
            .any(|window| {
                window
                    == b"MaxScriptPlayers=3\r\nScriptPlayerNames=\"Bot\\\\\\\"One\\n\\200\"\r\nRandomTeamCount=-2\r\n"
            }));
    }

    #[test]
    fn all_default_team_list_is_a_present_zero_byte_component() {
        let compiler_defaults = TeamConfiguration {
            active: true,
            custom: true,
            allow_hostility_change: false,
            distribution: 0,
            allow_team_switch: false,
            auto_generate_teams: false,
            team_colors: false,
        };
        assert_eq!(
            serialize_teams(&[], compiler_defaults, 0, 0, b"", 0),
            Some(Vec::new())
        );
    }
}
