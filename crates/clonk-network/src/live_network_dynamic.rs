//! Composition of a synchronized runtime-join dynamic C4Group.

use std::collections::HashSet;

use clonk_resources::{c4group_file_crc, MutableGroup, MutableGroupError};
use thiserror::Error;

const SCENARIO_ENTRY: &str = "Scenario.txt";
const PARAMETERS_ENTRY: &str = "Parameters.txt";
// C4GroupMaxMaker (`src/C4Group.h:54`).
const GROUP_MAKER_MAX_BYTES: usize = 30;
// `C4FLS_Scenario` (`src/C4Components.h:140`). Keep this explicit here so
// the returned entry metadata has the same order as the packed group, rather
// than merely relying on MutableGroup's filename-selected Close-time sort.
const C4FLS_SCENARIO: &str = "Loader*.bmp|Loader*.png|Loader*.jpeg|Loader*.jpg|Fonts.txt|Scenario.txt|Title*.txt|Info.txt|Desc*.rtf|Icon.png|Icon.bmp|Game.txt|StringTbl*.txt|Teams.txt|Parameters.txt|Info.txt|Sect*.c4g|Music.c4g|*.mid|*.wav|Desc*.rtf|Title.bmp|Title.png|*.c4d|Material.c4g|MatMap.txt|Landscape.bmp|Landscape.png|DiffLandscape.bmp|Sky.bmp|Sky.png|Sky.jpeg|Sky.jpg|PXS.c4b|MassMover.c4b|CtrlRec.c4b|Strings.txt|Objects.txt|RoundResults.txt|Author.txt|Version.txt|Names.txt|*.c4d|Script.c|Script*.c|System.c4g";

#[derive(Debug, Clone, PartialEq, Eq)]
// This is a public, construction-oriented API. Boxing `MutableGroup` would
// break callers and add an allocation to every composed child solely to make
// the enum's uncommon variants closer in size.
#[allow(clippy::large_enum_variant)]
pub enum LiveNetworkDynamicComponent {
    File {
        name: String,
        payload: Vec<u8>,
    },
    /// A freshly composed child group. The parent stores its raw group image,
    /// not a standalone gzip envelope.
    Child {
        name: String,
        group: MutableGroup,
    },
    /// An opaque raw child image with its retained parent-entry metadata.
    PackedChild {
        name: String,
        raw_group: Vec<u8>,
        contents_crc: u32,
        time: u32,
        executable: bool,
    },
}

impl LiveNetworkDynamicComponent {
    fn name(&self) -> &str {
        match self {
            Self::File { name, .. } | Self::Child { name, .. } | Self::PackedChild { name, .. } => {
                name
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveNetworkDynamicSpec {
    pub group_filename: String,
    /// Native C4 bytes copied into the packed group header.
    pub maker: Vec<u8>,
    pub parameters: Vec<u8>,
    pub scenario: Vec<u8>,
    /// Remaining save components in `C4GameSave::SaveData` write order.
    pub components: Vec<LiveNetworkDynamicComponent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveNetworkDynamicEntry {
    pub name: String,
    pub contents_crc: u32,
}

/// Packed runtime dynamic plus the metadata needed to publish it as NRT_Dynamic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveNetworkDynamic {
    pub group_filename: String,
    /// Exact C-string body retained by the packed C4Group header.
    pub maker: Vec<u8>,
    pub packed_bytes: Vec<u8>,
    pub file_size: u32,
    pub file_crc: u32,
    pub contents_crc: u32,
    /// Entries in final `C4FLS_Scenario` order.
    pub entries: Vec<LiveNetworkDynamicEntry>,
}

impl From<crate::InitialNetworkDynamic> for LiveNetworkDynamic {
    fn from(dynamic: crate::InitialNetworkDynamic) -> Self {
        Self {
            group_filename: dynamic.group_filename,
            maker: dynamic.maker,
            packed_bytes: dynamic.packed_bytes,
            file_size: dynamic.file_size,
            file_crc: dynamic.file_crc,
            contents_crc: dynamic.contents_crc,
            entries: dynamic
                .entries
                .into_iter()
                .map(|entry| LiveNetworkDynamicEntry {
                    name: entry.name.to_owned(),
                    contents_crc: entry.contents_crc,
                })
                .collect(),
        }
    }
}

#[derive(Debug, Error)]
pub enum LiveNetworkDynamicError {
    #[error("live network C4Group composition failed: {0}")]
    Group(#[from] MutableGroupError),
    #[error("network dynamic filename does not select C4FLS_Scenario sorting: {0}")]
    InvalidGroupFilename(String),
    #[error("live network component duplicates required entry {0}")]
    ReservedComponent(String),
    #[error("live network component name is duplicated: {0}")]
    DuplicateComponent(String),
    #[error("composed C4Group lost metadata for {0}")]
    MissingEntryMetadata(String),
    #[error("packed network dynamic has {0} bytes; C++ FileSize is uint32")]
    PackedFileTooLarge(usize),
}

/// Packs already-serialized live save components in the same two-stage order
/// as `C4GameSaveNetwork(false)`: Parameters/Scenario are written by SaveCore,
/// the caller's ordered components by SaveData, and Close applies
/// `C4FLS_Scenario` to the final directory.
pub fn compose_live_network_dynamic(
    spec: LiveNetworkDynamicSpec,
) -> Result<LiveNetworkDynamic, LiveNetworkDynamicError> {
    if !selects_scenario_sort(&spec.group_filename) {
        return Err(LiveNetworkDynamicError::InvalidGroupFilename(
            spec.group_filename,
        ));
    }

    validate_component_names(&spec.components)?;

    let mut group = MutableGroup::new(spec.group_filename.clone());
    if !spec.maker.is_empty() {
        group.set_maker_bytes(&spec.maker);
    }
    group.add_file(PARAMETERS_ENTRY, spec.parameters)?;
    group.add_file(SCENARIO_ENTRY, spec.scenario)?;
    for component in spec.components {
        match component {
            LiveNetworkDynamicComponent::File { name, payload } => {
                group.add_file(name, payload)?;
            }
            LiveNetworkDynamicComponent::Child { name, group: child } => {
                group.add_child(name, child)?;
            }
            LiveNetworkDynamicComponent::PackedChild {
                name,
                raw_group,
                contents_crc,
                time,
                executable,
            } => {
                group.add_packed_child_with_metadata(
                    name,
                    raw_group,
                    contents_crc,
                    time,
                    executable,
                )?;
            }
        }
    }

    // MutableGroup::pack also selects this list from the .c4s filename. Sort
    // explicitly so the public metadata order exactly describes packed bytes.
    let _ = group.sort(C4FLS_SCENARIO);
    let entries = group
        .entry_names()
        .into_iter()
        .map(|name| {
            let contents_crc = group
                .entry_crc(name)
                .ok_or_else(|| LiveNetworkDynamicError::MissingEntryMetadata(name.to_owned()))?;
            Ok(LiveNetworkDynamicEntry {
                name: name.to_owned(),
                contents_crc,
            })
        })
        .collect::<Result<Vec<_>, LiveNetworkDynamicError>>()?;
    let contents_crc = group.contents_crc();
    let packed_bytes = group.pack()?;
    let file_size = u32::try_from(packed_bytes.len())
        .map_err(|_| LiveNetworkDynamicError::PackedFileTooLarge(packed_bytes.len()))?;
    let file_crc = c4group_file_crc(&packed_bytes);

    Ok(LiveNetworkDynamic {
        group_filename: spec.group_filename,
        maker: packed_maker(&spec.maker),
        packed_bytes,
        file_size,
        file_crc,
        contents_crc,
        entries,
    })
}

fn validate_component_names(
    components: &[LiveNetworkDynamicComponent],
) -> Result<(), LiveNetworkDynamicError> {
    let mut names = HashSet::with_capacity(components.len());
    for component in components {
        let name = component.name();
        if name.eq_ignore_ascii_case(PARAMETERS_ENTRY) || name.eq_ignore_ascii_case(SCENARIO_ENTRY)
        {
            return Err(LiveNetworkDynamicError::ReservedComponent(name.to_owned()));
        }
        let folded = name.to_ascii_lowercase();
        if !names.insert(folded) {
            return Err(LiveNetworkDynamicError::DuplicateComponent(name.to_owned()));
        }
    }
    Ok(())
}

fn packed_maker(maker: &[u8]) -> Vec<u8> {
    let length = maker
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(maker.len())
        .min(GROUP_MAKER_MAX_BYTES);
    maker[..length].to_vec()
}

fn selects_scenario_sort(filename: &str) -> bool {
    filename
        .rsplit(['/', '\\'])
        .next()
        .is_some_and(|filename| filename.to_ascii_lowercase().ends_with(".c4s"))
}
