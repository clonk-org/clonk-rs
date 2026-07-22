//! Composition of a C++-compatible initial network dynamic C4Group.

use clonk_engine::{
    serialize_initial_network_game, InitialNetworkGameData, InitialNetworkGameError, Scenario,
    ScenarioError,
};
use clonk_resources::{c4group_file_crc, MutableGroup, MutableGroupError};
use thiserror::Error;

use crate::initial_network_parameters::{
    serialize_initial_network_parameters, InitialNetworkParametersError,
    InitialNetworkScenarioDefaults,
};
use crate::legacy::JoinGameParametersEnvelope;

const SCENARIO_ENTRY: &str = "Scenario.txt";
const GAME_ENTRY: &str = "Game.txt";
const PARAMETERS_ENTRY: &str = "Parameters.txt";
// C4GroupMaxMaker (`src/C4Group.h:54`).
const GROUP_MAKER_MAX_BYTES: usize = 30;

#[derive(Debug, Clone)]
pub struct InitialNetworkDynamicSpec<'a> {
    pub group_filename: &'a str,
    /// Native C4 bytes copied into the packed group header.
    pub maker: &'a [u8],
    pub scenario: &'a Scenario,
    pub scenario_title: &'a str,
    pub definition_modules: &'a [String],
    /// Native `Config.General.ExePath`, including its trailing separator.
    pub definition_executable_path: &'a str,
    /// Native `Config.General.DefinitionPath` (relative or absolute).
    pub definition_path: &'a str,
    pub scenario_origin: &'a str,
    pub game: &'a InitialNetworkGameData,
    pub original_game_text: Option<&'a [u8]>,
    pub parameters: &'a JoinGameParametersEnvelope,
    pub scenario_defaults: &'a InitialNetworkScenarioDefaults,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitialNetworkDynamicEntry {
    pub name: &'static str,
    pub payload: Vec<u8>,
    pub contents_crc: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitialNetworkDynamic {
    pub group_filename: String,
    /// Exact C-string body retained by the packed C4Group header.
    pub maker: Vec<u8>,
    pub packed_bytes: Vec<u8>,
    pub file_size: u32,
    pub file_crc: u32,
    pub contents_crc: u32,
    /// Entries in final `C4FLS_Scenario` order.
    pub entries: Vec<InitialNetworkDynamicEntry>,
}

#[derive(Debug, Error)]
pub enum InitialNetworkDynamicError {
    #[error("initial Scenario.txt serialization failed: {0}")]
    Scenario(#[from] ScenarioError),
    #[error("initial Game.txt serialization failed: {0}")]
    Game(#[from] InitialNetworkGameError),
    #[error("initial Parameters.txt serialization failed: {0}")]
    Parameters(#[from] InitialNetworkParametersError),
    #[error("initial network C4Group composition failed: {0}")]
    Group(#[from] MutableGroupError),
    #[error("network dynamic filename does not select C4FLS_Scenario sorting: {0}")]
    InvalidGroupFilename(String),
    #[error("composed C4Group lost metadata for {0}")]
    MissingEntryMetadata(&'static str),
    #[error("packed network dynamic has {0} bytes; C++ FileSize is uint32")]
    PackedFileTooLarge(usize),
}

/// Runs the three initial-save serializers and packs their payloads using the
/// same C4Group maker and scenario sort order as `C4GameSaveNetwork(true)`.
pub fn compose_initial_network_dynamic(
    spec: InitialNetworkDynamicSpec<'_>,
) -> Result<InitialNetworkDynamic, InitialNetworkDynamicError> {
    if !selects_scenario_sort(spec.group_filename) {
        return Err(InitialNetworkDynamicError::InvalidGroupFilename(
            spec.group_filename.to_owned(),
        ));
    }

    let mut group = MutableGroup::new(spec.group_filename);
    if !spec.maker.is_empty() {
        group.set_maker_bytes(spec.maker);
    }

    // SaveCore writes Parameters before Scenario; SaveData follows with Game.
    // Preserve that timestamp-producing sequence even though Close later sorts
    // entries into Scenario/Game/Parameters order (C4GameSave.cpp:58-108,
    // 465-515; C4Components.h:C4FLS_Scenario).
    let parameters = serialize_initial_network_parameters(spec.parameters, spec.scenario_defaults)?;
    group.add_file(PARAMETERS_ENTRY, parameters.clone())?;

    let scenario = spec.scenario.serialize_initial_network_scenario(
        spec.scenario_title,
        spec.definition_modules,
        spec.definition_executable_path,
        spec.definition_path,
        spec.scenario_origin,
    )?;
    group.add_file(SCENARIO_ENTRY, scenario.clone())?;

    let game = serialize_initial_network_game(spec.game, spec.original_game_text)?;
    if let Some(payload) = game.as_ref() {
        group.add_file(GAME_ENTRY, payload.clone())?;
    }

    let mut entries = Vec::with_capacity(2 + usize::from(game.is_some()));
    entries.push(dynamic_entry(&group, SCENARIO_ENTRY, scenario)?);
    if let Some(game) = game {
        entries.push(dynamic_entry(&group, GAME_ENTRY, game)?);
    }
    entries.push(dynamic_entry(&group, PARAMETERS_ENTRY, parameters)?);

    let contents_crc = group.contents_crc();
    let packed_bytes = group.pack()?;
    let file_size = u32::try_from(packed_bytes.len())
        .map_err(|_| InitialNetworkDynamicError::PackedFileTooLarge(packed_bytes.len()))?;
    let file_crc = c4group_file_crc(&packed_bytes);
    Ok(InitialNetworkDynamic {
        group_filename: spec.group_filename.to_owned(),
        maker: packed_maker(spec.maker),
        packed_bytes,
        file_size,
        file_crc,
        contents_crc,
        entries,
    })
}

fn packed_maker(maker: &[u8]) -> Vec<u8> {
    let length = maker
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(maker.len())
        .min(GROUP_MAKER_MAX_BYTES);
    maker[..length].to_vec()
}

fn dynamic_entry(
    group: &MutableGroup,
    name: &'static str,
    payload: Vec<u8>,
) -> Result<InitialNetworkDynamicEntry, InitialNetworkDynamicError> {
    let contents_crc = group
        .entry_crc(name)
        .ok_or(InitialNetworkDynamicError::MissingEntryMetadata(name))?;
    Ok(InitialNetworkDynamicEntry {
        name,
        payload,
        contents_crc,
    })
}

fn selects_scenario_sort(filename: &str) -> bool {
    filename
        .rsplit(['/', '\\'])
        .next()
        .is_some_and(|filename| filename.to_ascii_lowercase().ends_with(".c4s"))
}
