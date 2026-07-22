//! C++-format live player files embedded by `C4GameSaveNetwork(false)`.
//!
//! `C4PlayerList::Save` creates a fresh `.c4p`, asks `C4Player::Save` for
//! `Player.txt` and every saved `C4ObjectInfo`, and finally applies
//! `C4FLS_Player`.  This module performs that operation from synchronized
//! engine state without passing through the private Rust JSON save format.

use std::collections::HashSet;
use std::rc::Rc;

use clonk_resources::{Group, MutableGroup, MutableGroupError, PhysicalInfo};
use thiserror::Error;

use crate::player_file::{CrewInfo, PlayerFile, PlayerInfoCoreState};
use crate::{Engine, EngineState, LiveC4ValueEncodeError, LiveC4ValueEnumeration, PlayerState};

const C4FLS_PLAYER: &str = "Player.txt|Portrait.png|Portrait.bmp|*.c4i";
const C4FLS_OBJECT: &str = "ObjectInfo.txt|Portrait.png|Portrait.bmp";

#[derive(Debug, Error)]
pub enum LiveC4PlayerError {
    #[error("live C4 player {0} does not exist")]
    PlayerNotFound(i32),
    #[error("{scope} extra-data name `{name}` is not a C4 identifier")]
    InvalidExtraDataName { scope: &'static str, name: String },
    #[error("failed to encode {scope} extra-data slot `{name}`: {source}")]
    ExtraDataValue {
        scope: &'static str,
        name: String,
        #[source]
        source: LiveC4ValueEncodeError,
    },
    #[error("{scope} has {count} extra-data slots, exceeding C4's signed count")]
    TooManyExtraDataEntries { scope: &'static str, count: usize },
    #[error("cannot encode retained {asset} image: {detail}")]
    ImageEncoding { asset: &'static str, detail: String },
    #[error(
        "crew `{crew}` owns a copied/custom portrait surface without retained pixels or a reconstruction source"
    )]
    UnreconstructablePortrait { crew: String },
    #[error("failed to inspect the copied local player profile: {0}")]
    LocalProfile(String),
    #[error(transparent)]
    Group(#[from] MutableGroupError),
}

/// Native C4Player::Save flags, process-local C4Config inputs consulted by
/// C4ObjectInfo::Save, and the localized C4PlayerInfoCore compiler default.
#[derive(Debug, Clone, Copy)]
pub struct LiveC4PlayerSaveOptions<'a> {
    /// Native `C4Player::Save`'s `fSavegame` flag. Regular external player
    /// files pass false and omit crew whose loaded definition sets
    /// `TemporaryCrew`; embedded savegame/record/network groups pass true.
    pub savegame: bool,
    /// Native `C4ObjectInfo::Save`'s `fStoreTiny` flag. Despite the name,
    /// embedded saves receive `C4PlayerList`'s `fStoreOnOriginal`: every
    /// reachable embedded scenario/savegame/record/network path passes false.
    /// Ordinary external remote-profile synchronization passes true.
    pub store_tiny: bool,
    pub add_new_crew_portraits: bool,
    pub save_default_portraits: bool,
    pub player_rank_name_default: &'a str,
}

/// Existing crew entries that native `C4ObjectInfo::Save` deletes while it
/// updates a copied local profile. Omission alone is not a C4Group deletion,
/// so the application applies this plan after overlaying serialized entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveC4CrewProfileCleanup {
    /// Final filename used by the serialized update.
    pub filename: Vec<u8>,
    /// Filename held by C4ObjectInfo before this save. Empty for new crew.
    pub original_filename: Vec<u8>,
    /// Index in the player's runtime C4ObjectInfo list.
    #[doc(hidden)]
    pub roster_index: usize,
    pub remove_default_portrait_png: bool,
    pub remove_rank_png: bool,
}

/// The external-player synchronization image and its local-profile cleanup.
/// Remote players are emitted as fresh tiny groups and have an empty cleanup
/// plan because no original entries are copied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveC4SynchronizedPlayerGroup {
    pub group: MutableGroup,
    pub crew_cleanup: Vec<LiveC4CrewProfileCleanup>,
}

impl<'a> Default for LiveC4PlayerSaveOptions<'a> {
    fn default() -> Self {
        Self {
            savegame: true,
            store_tiny: false,
            add_new_crew_portraits: true,
            save_default_portraits: true,
            player_rank_name_default: "Rank",
        }
    }
}

/// Serialize one live runtime player into the small `.c4p` child group used
/// by a non-initial `C4GameSaveNetwork` dynamic.
///
/// The live-engine overload also mirrors `C4ObjectInfoCore::Save(pDefs)` by
/// refreshing custom current/next-rank text from the loaded definition list.
pub fn serialize_live_c4_player(
    engine: &Engine,
    player_number: i32,
    filename: &[u8],
    maker: &[u8],
) -> Result<MutableGroup, LiveC4PlayerError> {
    serialize_live_c4_player_with_options(
        engine,
        player_number,
        filename,
        maker,
        LiveC4PlayerSaveOptions::default(),
    )
}

pub fn serialize_live_c4_player_with_options(
    engine: &Engine,
    player_number: i32,
    filename: &[u8],
    maker: &[u8],
    options: LiveC4PlayerSaveOptions<'_>,
) -> Result<MutableGroup, LiveC4PlayerError> {
    serialize_live_c4_player_with_options_and_value_encoding(
        engine,
        player_number,
        filename,
        maker,
        options,
        PlayerValueEncoding::CurrentIds,
    )
}

/// Rebuild the temporary player group used by `C4Player::Strip(..., true)`.
///
/// The caller has already loaded the source through [`PlayerFile`], so both
/// cores carry C++'s load-time normalization. The aggressive strip writes a
/// fresh group containing only canonical `Player.txt` and retained
/// `ObjectInfo.txt` files, drops crew without a currently loaded definition,
/// and refreshes definition-owned custom-rank fields at the save boundary.
pub fn serialize_aggressively_stripped_c4_player(
    engine: &Engine,
    player: &PlayerFile,
    filename: &[u8],
    maker: &[u8],
    player_rank_name_default: &str,
) -> Result<MutableGroup, LiveC4PlayerError> {
    let mut group = MutableGroup::new_bytes(filename.to_vec());
    if !maker.is_empty() {
        group.set_maker_bytes(maker);
    }
    group.add_file(
        "Player.txt",
        serialize_player_info_core(
            &player.exact_info_core(),
            player_rank_name_default,
            PlayerValueEncoding::CurrentIds,
        )?,
    )?;

    let mut used_filenames = Vec::with_capacity(player.crew.len());
    // PlayerFile retains C4ObjectInfoList::Load's discovery order. Native
    // Add prepends each entry and Save walks tail-to-head, restoring exactly
    // this order while flattening recursively discovered crew into the root.
    for source in &player.crew {
        let definition_id = clonk_script::c4_id_text(&source.id);
        let Some(definition) = engine.definition(&definition_id) else {
            continue;
        };
        let mut info = source.clone();
        crate::update_custom_rank_fields(
            &mut info.rank_name,
            &mut info.core,
            info.rank,
            definition.rank_names(),
            definition.rank_base(),
        );
        let child_name = retained_or_unique_crew_filename(&info, &used_filenames);
        let mut child = MutableGroup::new_bytes(child_name.clone());
        if !maker.is_empty() {
            child.set_maker_bytes(maker);
        }
        child.add_file(
            "ObjectInfo.txt",
            serialize_object_info(&info, PlayerValueEncoding::CurrentIds)?,
        )?;
        child.sort(C4FLS_OBJECT);
        group.add_child_bytes(child_name.clone(), child)?;
        used_filenames.push(child_name);
    }

    group.sort(C4FLS_PLAYER);
    Ok(group)
}

/// Serialize the ordinary external `C4Player::Save()` synchronization path.
///
/// C++ copies and updates the original group only for `LocalControl`. A
/// remote player is recreated in a fresh group with `fStoreTiny=true`, after
/// stripping crew whose definitions are not loaded. If the attempted local
/// copy is unavailable, native continues with a fresh, non-tiny group.
pub fn serialize_live_c4_player_for_synchronization(
    engine: &mut Engine,
    player_number: i32,
    filename: &[u8],
    maker: &[u8],
    local_control: bool,
    original_profile: Option<&Group>,
    options: LiveC4PlayerSaveOptions<'_>,
) -> Result<LiveC4SynchronizedPlayerGroup, LiveC4PlayerError> {
    let mut profile_entry_names = if local_control {
        original_profile
            .map(|profile| {
                profile
                    .entries()
                    .map_err(|error| LiveC4PlayerError::LocalProfile(error.to_string()))
                    .map(|entries| {
                        entries
                            .into_iter()
                            .map(|entry| entry.name_bytes)
                            .collect::<Vec<_>>()
                    })
            })
            .transpose()?
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    if !local_control && engine.player(player_number).is_some() {
        strip_unresolved_remote_crew_for_synchronization(engine, player_number);
    }
    let state = engine.capture_state_for_network_save();
    let player = state
        .players
        .iter()
        .find(|player| player.id == player_number)
        .ok_or(LiveC4PlayerError::PlayerNotFound(player_number))?;
    let synchronized = serialize_player_group_with_profile_policy(
        &state,
        player,
        filename,
        maker,
        options,
        PlayerValueEncoding::CurrentIds,
        |info| {
            let definition = engine.definition(&info.id);
            // Ordinary external files omit TemporaryCrew. Remote Strip also
            // removes unresolved definitions, while local files retain them.
            definition.is_some_and(|definition| definition.temporary_crew == 0)
                || (local_control && definition.is_none())
        },
        !local_control,
        |info, used| {
            if local_control {
                resolve_local_profile_crew_filename(info, &mut profile_entry_names)
            } else {
                retained_or_unique_crew_filename(info, used)
            }
        },
        |info| {
            if local_control {
                materialize_live_portrait(engine, info, options)?;
            } else {
                clear_portrait_payload(info);
            }

            let mut remove_rank_png = false;
            if let Some(definition) = engine.definition(&info.id) {
                crate::update_custom_rank_fields(
                    &mut info.rank_name,
                    &mut info.core,
                    info.rank,
                    definition.rank_names(),
                    definition.rank_base(),
                );
                if local_control {
                    remove_rank_png = definition.rank_symbols_image().is_none();
                    info.core.rank_png =
                        render_live_rank_symbol(engine, &info.id, info.rank)?.unwrap_or_default();
                } else {
                    info.core.rank_png.clear();
                }
            } else {
                info.core.rank_png.clear();
            }

            Ok(ProfileCrewMutation {
                track_local_profile: local_control,
                // C4ObjectInfo.cpp:240-247 only removes the copied PNG pair
                // when default portraits are disabled and the specification
                // is not a custom portrait. The overlay applies the further
                // native `FindEntry(Portrait.png)` gate.
                remove_default_portrait_png: local_control
                    && !options.save_default_portraits
                    && info.core.portrait_file != "custom",
                // With a loaded def but no pRankSymbols, C++ explicitly
                // deletes Rank.png. Missing defs and failed draws retain it.
                remove_rank_png: local_control && remove_rank_png,
            })
        },
    )?;

    // C4ObjectInfo::Save mutates Filename when a local child is first named
    // or successfully renamed. Retain that mutation so a later sync updates
    // the same child instead of recreating the obsolete filename.
    if local_control {
        if let Some(roster) = engine.crew_rosters.get_mut(&player_number) {
            for cleanup in &synchronized.crew_cleanup {
                if let Some(info) = roster.get_mut(cleanup.roster_index) {
                    info.core.original_filename =
                        clonk_script::c4_string_from_bytes(&cleanup.filename);
                }
            }
        }
    }
    Ok(synchronized)
}

/// `C4ObjectInfoList::Strip`: remote player files are temporary resumable
/// profiles, so their owning runtime roster permanently drops entries whose
/// definitions are not loaded before the save snapshot is taken.
#[doc(hidden)]
pub fn strip_unresolved_remote_crew_for_synchronization(engine: &mut Engine, player_number: i32) {
    let Some(roster) = engine.crew_rosters.get(&player_number) else {
        return;
    };
    let retained = roster
        .iter()
        .map(|info| engine.definition(&info.id).is_some())
        .collect::<Vec<_>>();
    if retained.iter().all(|retained| *retained) {
        return;
    }

    let mut old_to_new = vec![None; retained.len()];
    let mut retained_count = 0;
    for (old_index, retained) in retained.iter().copied().enumerate() {
        if retained {
            old_to_new[old_index] = Some(retained_count);
            retained_count += 1;
        }
    }

    let roster = engine
        .crew_rosters
        .get_mut(&player_number)
        .expect("the inspected crew roster still exists");
    let mut old_index = 0;
    roster.retain(|_| {
        let keep = retained[old_index];
        old_index += 1;
        keep
    });

    let order = engine.crew_info_order.entry(player_number).or_default();
    let mut seen = HashSet::with_capacity(retained_count);
    let mut remapped_order = order
        .iter()
        .filter_map(|old_index| old_to_new.get(*old_index).copied().flatten())
        .filter(|new_index| seen.insert(*new_index))
        .collect::<Vec<_>>();
    remapped_order.extend((0..retained_count).filter(|new_index| seen.insert(*new_index)));
    *order = remapped_order;

    // ControlCount belongs to the C4ObjectInfo pointer represented by the
    // roster link. Compacting the Vec must therefore move retained counters
    // to the new pointer-equivalent index and discard stripped entries.
    engine.crew_info_control_counts = std::mem::take(&mut engine.crew_info_control_counts)
        .into_iter()
        .filter_map(|(link, count)| {
            if link.player_id != player_number {
                return Some((link, count));
            }
            old_to_new
                .get(link.roster_index)
                .copied()
                .flatten()
                .map(|roster_index| {
                    (
                        crate::CrewInfoLink {
                            player_id: player_number,
                            roster_index,
                        },
                        count,
                    )
                })
        })
        .collect();

    let mut removed_objects = HashSet::new();
    Rc::make_mut(&mut engine.crew_info_links).retain(|object_id, link| {
        if link.player_id != player_number {
            return true;
        }
        let Some(new_index) = old_to_new.get(link.roster_index).copied().flatten() else {
            removed_objects.insert(*object_id);
            return false;
        };
        link.roster_index = new_index;
        true
    });
    if !removed_objects.is_empty() {
        Rc::make_mut(&mut engine.crew_object_infos)
            .retain(|object_id, _| !removed_objects.contains(object_id));
        Rc::make_mut(&mut engine.crew_ranks).retain(|object_id, _| {
            !removed_objects
                .iter()
                .any(|removed| removed.as_u64() == *object_id)
        });
        for object in &mut engine.objects {
            if removed_objects.contains(&object.id) {
                object.state.info_physical = None;
            }
        }
    }
}

/// Serialize with the C4StringTable enumeration produced by the enclosing
/// live scenario save. Native C++ enumerates once, then reuses those `S<n>`
/// IDs while writing every embedded player and crew group.
pub fn serialize_live_c4_player_with_options_and_enumeration(
    engine: &Engine,
    player_number: i32,
    filename: &[u8],
    maker: &[u8],
    options: LiveC4PlayerSaveOptions<'_>,
    enumeration: &LiveC4ValueEnumeration,
) -> Result<MutableGroup, LiveC4PlayerError> {
    serialize_live_c4_player_with_options_and_value_encoding(
        engine,
        player_number,
        filename,
        maker,
        options,
        PlayerValueEncoding::Synchronized(enumeration),
    )
}

fn serialize_live_c4_player_with_options_and_value_encoding(
    engine: &Engine,
    player_number: i32,
    filename: &[u8],
    maker: &[u8],
    options: LiveC4PlayerSaveOptions<'_>,
    value_encoding: PlayerValueEncoding<'_>,
) -> Result<MutableGroup, LiveC4PlayerError> {
    let state = engine.capture_state_for_network_save();
    let player = state
        .players
        .iter()
        .find(|player| player.id == player_number)
        .ok_or(LiveC4PlayerError::PlayerNotFound(player_number))?;
    serialize_player_group(
        &state,
        player,
        filename,
        maker,
        options,
        value_encoding,
        |info| should_serialize_crew_definition(engine, &info.id, options.savegame),
        |info| {
            if !options.store_tiny {
                materialize_live_portrait(engine, info, options)?;
            }
            if let Some(definition) = engine.definition(&info.id) {
                crate::update_custom_rank_fields(
                    &mut info.rank_name,
                    &mut info.core,
                    info.rank,
                    definition.rank_names(),
                    definition.rank_base(),
                );
                if !options.store_tiny {
                    info.core.rank_png =
                        render_live_rank_symbol(engine, &info.id, info.rank)?.unwrap_or_default();
                }
            } else {
                if !options.store_tiny {
                    info.core.rank_png.clear();
                }
            }
            Ok(())
        },
    )
}

/// Serialize a player selected by number from an already synchronized engine
/// snapshot.  Definition metadata is not present in `EngineState`, so the
/// snapshot's retained custom-rank fields are emitted verbatim.
pub fn serialize_live_c4_player_from_state(
    state: &EngineState,
    player_number: i32,
    filename: &[u8],
    maker: &[u8],
) -> Result<MutableGroup, LiveC4PlayerError> {
    let player = state
        .players
        .iter()
        .find(|player| player.id == player_number)
        .ok_or(LiveC4PlayerError::PlayerNotFound(player_number))?;
    serialize_live_c4_player_state(state, player, filename, maker)
}

/// Serialize an explicit runtime-player snapshot.  The caller may use this
/// when it already resolved the C4PlayerInfo/restore-info association.
pub fn serialize_live_c4_player_state(
    state: &EngineState,
    player: &PlayerState,
    filename: &[u8],
    maker: &[u8],
) -> Result<MutableGroup, LiveC4PlayerError> {
    serialize_player_group(
        state,
        player,
        filename,
        maker,
        LiveC4PlayerSaveOptions::default(),
        PlayerValueEncoding::CurrentIds,
        |_| true,
        |_| Ok(()),
    )
}

fn should_serialize_crew_definition(engine: &Engine, id: &str, savegame: bool) -> bool {
    // C4ObjectInfoList::Save only applies the TemporaryCrew filter to
    // regular player files. An unresolved definition is retained.
    savegame
        || engine
            .definition(id)
            .is_none_or(|definition| definition.temporary_crew == 0)
}

#[derive(Clone, Copy)]
enum PlayerValueEncoding<'a> {
    /// Ordinary C4Player::Save does not enumerate the global string table.
    /// It writes each C4String's current iEnumID verbatim, including -1.
    CurrentIds,
    /// Embedded player groups follow the enumeration captured by the
    /// enclosing synchronized scenario save.
    Synchronized(&'a LiveC4ValueEnumeration),
}

impl PlayerValueEncoding<'_> {
    fn encode_value(self, value: &clonk_script::Value) -> Result<String, LiveC4ValueEncodeError> {
        match self {
            Self::CurrentIds => Ok(crate::live_c4_save::encode_value_with_current_string_ids(
                value,
            )),
            Self::Synchronized(enumeration) => enumeration.encode_value(value),
        }
    }
}

fn serialize_player_group(
    state: &EngineState,
    player: &PlayerState,
    filename: &[u8],
    maker: &[u8],
    options: LiveC4PlayerSaveOptions<'_>,
    value_encoding: PlayerValueEncoding<'_>,
    include_crew: impl FnMut(&CrewInfo) -> bool,
    mut refresh_rank: impl FnMut(&mut CrewInfo) -> Result<(), LiveC4PlayerError>,
) -> Result<MutableGroup, LiveC4PlayerError> {
    Ok(serialize_player_group_with_profile_policy(
        state,
        player,
        filename,
        maker,
        options,
        value_encoding,
        include_crew,
        options.store_tiny,
        retained_or_unique_crew_filename,
        |info| {
            refresh_rank(info)?;
            Ok(ProfileCrewMutation::default())
        },
    )?
    .group)
}

#[derive(Debug, Clone, Copy, Default)]
struct ProfileCrewMutation {
    track_local_profile: bool,
    remove_default_portrait_png: bool,
    remove_rank_png: bool,
}

#[allow(clippy::too_many_arguments)]
fn serialize_player_group_with_profile_policy(
    state: &EngineState,
    player: &PlayerState,
    filename: &[u8],
    maker: &[u8],
    options: LiveC4PlayerSaveOptions<'_>,
    value_encoding: PlayerValueEncoding<'_>,
    mut include_crew: impl FnMut(&CrewInfo) -> bool,
    store_tiny: bool,
    mut resolve_filename: impl FnMut(&CrewInfo, &[Vec<u8>]) -> Vec<u8>,
    mut refresh_crew: impl FnMut(&mut CrewInfo) -> Result<ProfileCrewMutation, LiveC4PlayerError>,
) -> Result<LiveC4SynchronizedPlayerGroup, LiveC4PlayerError> {
    let mut group = MutableGroup::new_bytes(filename.to_vec());
    if !maker.is_empty() {
        group.set_maker_bytes(maker);
    }
    group.add_file(
        "Player.txt",
        serialize_player_core(player, options.player_rank_name_default, value_encoding)?,
    )?;

    let roster = state
        .crew_info_rosters
        .get(&player.id)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let order = normalized_roster_order(state, player.id, roster.len());
    let mut used_filenames = Vec::<Vec<u8>>::with_capacity(roster.len());
    let mut crew_cleanup = Vec::with_capacity(roster.len());

    // C4ObjectInfoList::Save walks GetLast()/GetPrevious(): the inverse of
    // its First->Next traversal.  This matters before sorting because the
    // first stripped-name collision keeps the unnumbered filename.
    for &index in order.iter().rev() {
        if !include_crew(&roster[index]) {
            continue;
        }
        let mut info = roster[index].clone();
        let mutation = refresh_crew(&mut info)?;
        let child_name = resolve_filename(&info, &used_filenames);
        let mut child = MutableGroup::new_bytes(child_name.clone());
        if !maker.is_empty() {
            child.set_maker_bytes(maker);
        }
        child.add_file(
            "ObjectInfo.txt",
            serialize_object_info(&info, value_encoding)?,
        )?;
        if !store_tiny {
            add_retained_portrait_files(&mut child, &info)?;
        }
        child.sort(C4FLS_OBJECT);
        group.add_child_bytes(child_name.clone(), child)?;
        if mutation.track_local_profile {
            crew_cleanup.push(LiveC4CrewProfileCleanup {
                filename: child_name.clone(),
                original_filename: c4_c_string_bytes(&info.core.original_filename, usize::MAX),
                roster_index: index,
                remove_default_portrait_png: mutation.remove_default_portrait_png,
                remove_rank_png: mutation.remove_rank_png,
            });
        }
        used_filenames.push(child_name);
    }

    group.sort(C4FLS_PLAYER);
    Ok(LiveC4SynchronizedPlayerGroup {
        group,
        crew_cleanup,
    })
}

fn normalized_roster_order(state: &EngineState, player: i32, roster_len: usize) -> Vec<usize> {
    let mut seen = HashSet::with_capacity(roster_len);
    let mut order = state
        .crew_info_order
        .get(&player)
        .into_iter()
        .flatten()
        .copied()
        .filter(|index| *index < roster_len && seen.insert(*index))
        .collect::<Vec<_>>();
    order.extend((0..roster_len).filter(|index| seen.insert(*index)));
    order
}

fn retained_or_unique_crew_filename(info: &CrewInfo, used: &[Vec<u8>]) -> Vec<u8> {
    let retained = c4_c_string_bytes(&info.core.original_filename, usize::MAX);
    if !retained.is_empty()
        && !used
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(&retained))
    {
        retained
    } else {
        unique_crew_filename(&info.name, used)
    }
}

/// Resolve C4ObjectInfo::Save's local copied-group rename path. `entries`
/// tracks the group after earlier crew in native reverse-list save order.
fn resolve_local_profile_crew_filename(info: &CrewInfo, entries: &mut Vec<Vec<u8>>) -> Vec<u8> {
    let original = c4_c_string_bytes(&info.core.original_filename, usize::MAX);
    let desired = unique_crew_filename(&info.name, &[]);
    let mut final_name = original.clone();

    if original.is_empty() {
        final_name = unique_crew_filename(&info.name, entries);
    } else if !original.eq_ignore_ascii_case(&desired)
        && !entries
            .iter()
            .any(|entry| entry.eq_ignore_ascii_case(&desired))
    {
        // Rename succeeds only if the old entry is present. A fresh group
        // (remote/embedded save) therefore retains Filename, while the copied
        // local profile adopts the name-derived filename.
        if let Some(index) = entries
            .iter()
            .position(|entry| entry.eq_ignore_ascii_case(&original))
        {
            entries[index] = desired.clone();
            final_name = desired;
        }
    }

    if !entries
        .iter()
        .any(|entry| entry.eq_ignore_ascii_case(&final_name))
    {
        entries.push(final_name.clone());
    }
    final_name
}

fn add_retained_portrait_files(
    child: &mut MutableGroup,
    info: &CrewInfo,
) -> Result<(), LiveC4PlayerError> {
    if !info.core.portrait_png.is_empty() {
        child.add_file("Portrait.png", info.core.portrait_png.clone())?;
        if !info.core.portrait_overlay_png.is_empty() {
            child.add_file(
                "PortraitOverlay.png",
                info.core.portrait_overlay_png.clone(),
            )?;
        }
    } else if !info.core.portrait_bmp.is_empty() {
        // A fresh C4ObjectInfo group writes an owned legacy BMP surface back
        // through C4Portrait::SavePNG.
        let decoded =
            image::load_from_memory_with_format(&info.core.portrait_bmp, image::ImageFormat::Bmp)
                .map_err(|error| LiveC4PlayerError::ImageEncoding {
                asset: "crew portrait",
                detail: error.to_string(),
            })?;
        child.add_file(
            "Portrait.png",
            encode_dynamic_png(decoded, "crew portrait")?,
        )?;
        if !info.core.portrait_overlay_png.is_empty() {
            child.add_file(
                "PortraitOverlay.png",
                info.core.portrait_overlay_png.clone(),
            )?;
        }
    }
    if !info.core.rank_png.is_empty() {
        child.add_file("Rank.png", info.core.rank_png.clone())?;
    }
    Ok(())
}

fn materialize_live_portrait(
    engine: &Engine,
    info: &mut CrewInfo,
    options: LiveC4PlayerSaveOptions<'_>,
) -> Result<(), LiveC4PlayerError> {
    use crate::CrewPermanentPortrait;

    if !options.save_default_portraits {
        clear_portrait_payload(info);
        return Ok(());
    }

    match info.portraits.permanent.clone() {
        CrewPermanentPortrait::ExplicitNone => {
            clear_portrait_payload(info);
            info.core.portrait_file = "none".to_string();
        }
        CrewPermanentPortrait::Assigned(portrait) => {
            if let Some(source) = portrait.source.as_ref() {
                clear_portrait_payload(info);
                materialize_definition_portrait(engine, info, source.as_str(), &portrait.name)?;
                info.core.portrait_file = if source.as_str() == info.id {
                    portrait.name
                } else {
                    format!("{}::{}", source.as_str(), portrait.name)
                };
            } else {
                // Owned graphics either came from a loaded custom payload or
                // from SetPortrait(copy=true), for which Rust retains the
                // immutable source in CrewInfoCoreFields.
                if info.core.portrait_png.is_empty()
                    && info.core.portrait_bmp.is_empty()
                    && !info.core.owned_portrait_source.is_empty()
                {
                    let source = info.core.owned_portrait_source.clone();
                    let name = info.core.owned_portrait_name.clone();
                    materialize_definition_portrait(engine, info, &source, &name)?;
                }
                if info.core.portrait_png.is_empty() && info.core.portrait_bmp.is_empty() {
                    return Err(LiveC4PlayerError::UnreconstructablePortrait {
                        crew: info.name.clone(),
                    });
                }
                info.core.portrait_file = "custom".to_string();
            }
        }
        CrewPermanentPortrait::Absent => {
            if !options.add_new_crew_portraits {
                clear_portrait_payload(info);
                return Ok(());
            }
            if info.core.portrait_png.is_empty() && info.core.portrait_bmp.is_empty() {
                if let Some(portrait) = info.portraits.current.clone() {
                    if let Some(source) = portrait.source {
                        materialize_definition_portrait(
                            engine,
                            info,
                            source.as_str(),
                            &portrait.name,
                        )?;
                    } else if !info.core.owned_portrait_source.is_empty() {
                        let source = info.core.owned_portrait_source.clone();
                        let name = info.core.owned_portrait_name.clone();
                        materialize_definition_portrait(engine, info, &source, &name)?;
                        if info.core.portrait_png.is_empty() && info.core.portrait_bmp.is_empty() {
                            return Err(LiveC4PlayerError::UnreconstructablePortrait {
                                crew: info.name.clone(),
                            });
                        }
                    } else {
                        return Err(LiveC4PlayerError::UnreconstructablePortrait {
                            crew: info.name.clone(),
                        });
                    }
                }
            }
        }
    }
    Ok(())
}

fn clear_portrait_payload(info: &mut CrewInfo) {
    info.core.portrait_png.clear();
    info.core.portrait_overlay_png.clear();
    info.core.portrait_bmp.clear();
}

fn materialize_definition_portrait(
    engine: &Engine,
    info: &mut CrewInfo,
    source: &str,
    name: &str,
) -> Result<(), LiveC4PlayerError> {
    let image = engine
        .definition_named_portrait_graphics_image(source, name)
        .or_else(|| {
            (name.eq_ignore_ascii_case("portrait1") || name.is_empty())
                .then(|| engine.definition_portrait_graphics_image(source))
                .flatten()
        });
    let Some(image) = image else {
        return Ok(());
    };
    let (png, overlay) = encode_definition_image(image, "definition portrait")?;
    info.core.portrait_png = png;
    info.core.portrait_overlay_png = overlay.unwrap_or_default();
    info.core.portrait_bmp.clear();
    Ok(())
}

fn encode_definition_image(
    image: crate::DefinitionPictureImage,
    asset: &'static str,
) -> Result<(Vec<u8>, Option<Vec<u8>>), LiveC4PlayerError> {
    let width = image.width();
    let height = image.height();
    let png = encode_rgba_png(width, height, &image.pixels(), asset)?;
    let overlay = image.color_mask().and_then(|mask| {
        let pixels = usize::try_from(u64::from(width) * u64::from(height)).ok()?;
        if mask.len() == pixels * 4 {
            Some(mask.to_vec())
        } else if mask.len() == pixels {
            let mut rgba = Vec::with_capacity(pixels * 4);
            for &coverage in mask.iter() {
                rgba.extend_from_slice(&[coverage, coverage, coverage, coverage]);
            }
            Some(rgba)
        } else {
            None
        }
    });
    let overlay = overlay
        .map(|pixels| encode_rgba_png(width, height, &pixels, "portrait overlay"))
        .transpose()?;
    Ok((png, overlay))
}

fn encode_rgba_png(
    width: u32,
    height: u32,
    pixels: &[u8],
    asset: &'static str,
) -> Result<Vec<u8>, LiveC4PlayerError> {
    let image = image::RgbaImage::from_raw(width, height, pixels.to_vec()).ok_or_else(|| {
        LiveC4PlayerError::ImageEncoding {
            asset,
            detail: format!(
                "{}x{} RGBA surface has {} bytes",
                width,
                height,
                pixels.len()
            ),
        }
    })?;
    encode_dynamic_png(image::DynamicImage::ImageRgba8(image), asset)
}

fn encode_dynamic_png(
    image: image::DynamicImage,
    asset: &'static str,
) -> Result<Vec<u8>, LiveC4PlayerError> {
    let mut cursor = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut cursor, image::ImageOutputFormat::Png)
        .map_err(|error| LiveC4PlayerError::ImageEncoding {
            asset,
            detail: error.to_string(),
        })?;
    Ok(cursor.into_inner())
}

fn render_live_rank_symbol(
    engine: &Engine,
    definition: &str,
    rank: i32,
) -> Result<Option<Vec<u8>>, LiveC4PlayerError> {
    let Some(strip) = engine.definition_rank_symbols_image(definition) else {
        return Ok(None);
    };
    let Some(base_count) = engine.definition_rank_symbol_count(definition) else {
        return Ok(None);
    };
    let size = strip.height();
    if size == 0 || base_count == 0 {
        return Ok(None);
    }
    let total_count = strip.width() / size;
    if total_count == 0 {
        return Ok(None);
    }
    let rank = rank.max(0) as u32;
    let mut base_phase = rank % base_count;
    let mut extension_phase = None;
    let mut use_captain = false;
    if rank / base_count != 0 {
        let requested = rank / base_count - 1 + base_count;
        if total_count > base_count {
            let phase = requested.min(total_count - 1);
            if requested >= total_count {
                base_phase = base_count - 1;
            }
            extension_phase = Some(phase);
        } else {
            use_captain = true;
        }
    }

    let strip_image =
        image::RgbaImage::from_raw(strip.width(), strip.height(), strip.pixels().to_vec())
            .ok_or_else(|| LiveC4PlayerError::ImageEncoding {
                asset: "rank symbol",
                detail: "definition rank strip has an invalid RGBA size".to_string(),
            })?;
    let x = base_phase.saturating_mul(size);
    let mut output = image::imageops::crop_imm(&strip_image, x, 0, size, size).to_image();
    if let Some(extension_phase) = extension_phase {
        let extension = image::imageops::crop_imm(
            &strip_image,
            extension_phase.saturating_mul(size),
            0,
            size,
            size,
        )
        .to_image();
        overlay_rank_extension(&mut output, &extension);
    } else if use_captain {
        const CAPTAIN_PNG: &[u8] = include_bytes!("../../../../planet/Graphics.c4g/Captain.png");
        let captain = image::load_from_memory_with_format(CAPTAIN_PNG, image::ImageFormat::Png)
            .map_err(|error| LiveC4PlayerError::ImageEncoding {
                asset: "captain rank extension",
                detail: error.to_string(),
            })?
            .into_rgba8();
        overlay_rank_extension(&mut output, &captain);
    }
    encode_dynamic_png(image::DynamicImage::ImageRgba8(output), "rank symbol").map(Some)
}

fn overlay_rank_extension(base: &mut image::RgbaImage, extension: &image::RgbaImage) {
    let width = base.width().saturating_mul(2) / 3;
    let height = base.height().saturating_mul(2) / 3;
    let extension = image::imageops::resize(
        extension,
        width.max(1),
        height.max(1),
        image::imageops::FilterType::Nearest,
    );
    image::imageops::overlay(base, &extension, 0, 0);
}

fn serialize_player_core(
    player: &PlayerState,
    rank_name_default: &str,
    value_encoding: PlayerValueEncoding<'_>,
) -> Result<Vec<u8>, LiveC4PlayerError> {
    let mut core = player
        .player_info_core
        .clone()
        .unwrap_or_else(|| fallback_player_info_core(player));
    // These fields live in the inherited C4PlayerInfoCore and continue to be
    // changed by the active C4Player. The runtime snapshot is authoritative.
    core.score = player.score;
    core.rounds = player.rounds;
    core.rounds_won = player.rounds_won;
    core.rounds_lost = player.rounds_lost;
    core.total_playing_time = player.total_playing_time;
    core.extra_data.clone_from(&player.extra_data);

    serialize_player_info_core(&core, rank_name_default, value_encoding)
}

fn serialize_player_info_core(
    core: &PlayerInfoCoreState,
    rank_name_default: &str,
    value_encoding: PlayerValueEncoding<'_>,
) -> Result<Vec<u8>, LiveC4PlayerError> {
    let mut player_lines = Vec::new();
    push_c4_string(&mut player_lines, "Name", &core.pref_name, "Neuling", 30);
    push_c4_string(&mut player_lines, "Comment", &core.comment, "", 256);
    push_i32(&mut player_lines, "Rank", core.rank, 0);
    push_c4_string(
        &mut player_lines,
        "RankName",
        &core.rank_name,
        rank_name_default,
        30,
    );
    push_i32(&mut player_lines, "Score", core.score, 0);
    push_i32(&mut player_lines, "Rounds", core.rounds, 0);
    push_i32(&mut player_lines, "RoundsWon", core.rounds_won, 0);
    push_i32(&mut player_lines, "RoundsLost", core.rounds_lost, 0);
    push_i32(
        &mut player_lines,
        "TotalPlayingTime",
        core.total_playing_time,
        0,
    );
    if !core.extra_data.is_empty() {
        player_lines.push(named_value_map_line(
            "ExtraData",
            &core.extra_data,
            "player",
            value_encoding,
        )?);
    }

    let mut preference_lines = Vec::new();
    push_i32(&mut preference_lines, "Color", core.pref_color, 0);
    push_u32(&mut preference_lines, "ColorDw", core.pref_color_dw, 0xff);
    push_u32(
        &mut preference_lines,
        "AlternateColorDw",
        core.pref_color2_dw,
        0,
    );
    push_i32(&mut preference_lines, "Control", core.pref_control, 1);
    push_i32(
        &mut preference_lines,
        "AutoStopControl",
        retained_preference_value(core.pref_control_style_value, core.pref_control_style),
        0,
    );
    // The compiler default is -1 ("inherit AutoStopControl"), but a loaded
    // C4PlayerInfoCore resolves that sentinel to a concrete zero/one before
    // it can be saved again.  Therefore this line is intentionally present
    // even when the effective preference is false.
    push_i32(
        &mut preference_lines,
        "AutoContextMenu",
        retained_preference_value(
            core.pref_auto_context_menu_value,
            core.pref_auto_context_menu,
        ),
        -1,
    );
    push_i32(&mut preference_lines, "Position", core.pref_position, 0);
    push_i32(
        &mut preference_lines,
        "Mouse",
        retained_preference_value(core.pref_mouse_value, core.pref_mouse),
        1,
    );

    let last = &core.last_round;
    let mut last_round_lines = Vec::new();
    if !last.title.is_empty() {
        push_escaped_c4_string(&mut last_round_lines, "Title", &last.title);
    }
    push_u32(&mut last_round_lines, "Date", last.date, 0);
    push_i32(&mut last_round_lines, "Duration", last.duration, 0);
    push_i32(&mut last_round_lines, "Won", last.won, 0);
    push_i32(&mut last_round_lines, "Score", last.score, 0);
    push_i32(&mut last_round_lines, "FinalScore", last.final_score, 0);
    push_i32(&mut last_round_lines, "TotalScore", last.total_score, 0);
    push_i32(&mut last_round_lines, "Bonus", last.bonus, 0);
    push_i32(&mut last_round_lines, "Level", last.level, 0);

    Ok(write_ini_sections([
        ("Player", player_lines),
        ("Preferences", preference_lines),
        ("LastRound", last_round_lines),
    ]))
}

fn fallback_player_info_core(player: &PlayerState) -> PlayerInfoCoreState {
    PlayerInfoCoreState {
        score: player.score,
        rounds: player.rounds,
        rounds_won: player.rounds_won,
        rounds_lost: player.rounds_lost,
        total_playing_time: player.total_playing_time,
        extra_data: player.extra_data.clone(),
        ..PlayerInfoCoreState::default()
    }
}

fn serialize_object_info(
    info: &CrewInfo,
    value_encoding: PlayerValueEncoding<'_>,
) -> Result<Vec<u8>, LiveC4PlayerError> {
    let mut object_lines = Vec::new();
    let definition = clonk_script::c4_id_text(&info.id);
    if clonk_script::c4_id_raw(&info.id) != 0 {
        push_bytes(
            &mut object_lines,
            "id",
            clonk_script::c4_string_bytes(&definition),
        );
    }
    push_c4_string(&mut object_lines, "Name", &info.name, "Clonk", 30);
    push_c4_string(
        &mut object_lines,
        "DeathMessage",
        &info.death_message,
        "",
        75,
    );
    push_c4_string(
        &mut object_lines,
        "PortraitFile",
        &info.core.portrait_file,
        "",
        36,
    );
    push_i32(&mut object_lines, "Rank", info.rank, 0);
    push_escaped_c4_string_default(&mut object_lines, "RankName", &info.rank_name, "Clonk");
    push_escaped_c4_string_default(
        &mut object_lines,
        "NextRankName",
        &info.core.next_rank_name,
        "",
    );
    push_c4_string(
        &mut object_lines,
        "TypeName",
        &info.core.type_name,
        "Clonk",
        31,
    );
    push_i32(&mut object_lines, "Participation", info.participation, 1);
    push_i32(&mut object_lines, "Experience", info.experience, 0);
    push_i32(&mut object_lines, "NextRankExp", info.core.next_rank_exp, 0);
    push_i32(&mut object_lines, "Rounds", info.rounds, 0);
    push_i32(&mut object_lines, "DeathCount", info.death_count, 0);
    push_i32(&mut object_lines, "Birthday", info.birthday, 0);
    push_i32(
        &mut object_lines,
        "TotalPlayingTime",
        info.total_playing_time,
        0,
    );
    push_i32(&mut object_lines, "Age", info.age, 0);
    if !info.extra_data.is_empty() {
        object_lines.push(named_value_map_line(
            "ExtraData",
            &info.extra_data,
            "crew",
            value_encoding,
        )?);
    }

    let physical_lines = physical_lines(&info.physical);
    Ok(write_ini_sections([
        ("ObjectInfo", object_lines),
        ("Physical", physical_lines),
    ]))
}

fn physical_lines(physical: &PhysicalInfo) -> Vec<Vec<u8>> {
    let mut lines = Vec::new();
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
        push_i32(&mut lines, name, value, 0);
    }
    lines
}

fn named_value_map_line(
    key: &str,
    entries: &[(String, clonk_script::Value)],
    scope: &'static str,
    value_encoding: PlayerValueEncoding<'_>,
) -> Result<Vec<u8>, LiveC4PlayerError> {
    let count =
        i32::try_from(entries.len()).map_err(|_| LiveC4PlayerError::TooManyExtraDataEntries {
            scope,
            count: entries.len(),
        })?;
    let mut line = format!("{key}={count}").into_bytes();
    if entries.is_empty() {
        return Ok(line);
    }
    line.push(b';');
    for (index, (name, value)) in entries.iter().enumerate() {
        let name_bytes = c4_c_string_bytes(name, usize::MAX);
        if !name_bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'_' | b'-'))
        {
            return Err(LiveC4PlayerError::InvalidExtraDataName {
                scope,
                name: name.clone(),
            });
        }
        if index != 0 {
            line.push(b',');
        }
        line.extend_from_slice(&name_bytes);
        line.push(b'=');
        let encoded = value_encoding.encode_value(value).map_err(|source| {
            LiveC4PlayerError::ExtraDataValue {
                scope,
                name: name.clone(),
                source,
            }
        })?;
        line.extend_from_slice(encoded.as_bytes());
    }
    Ok(line)
}

fn push_i32(lines: &mut Vec<Vec<u8>>, name: &str, value: i32, default: i32) {
    if value != default {
        lines.push(format!("{name}={value}").into_bytes());
    }
}

fn push_u32(lines: &mut Vec<Vec<u8>>, name: &str, value: u32, default: u32) {
    if value != default {
        lines.push(format!("{name}={value}").into_bytes());
    }
}

fn retained_preference_value(raw: i32, enabled: bool) -> i32 {
    if (raw != 0) == enabled {
        raw
    } else {
        i32::from(enabled)
    }
}

fn push_c4_string(
    lines: &mut Vec<Vec<u8>>,
    name: &str,
    value: &str,
    default: &str,
    max_bytes: usize,
) {
    let value = c4_c_string_bytes(value, max_bytes);
    // StdStringAdapt's fixed buffer is already bounded, but its default is a
    // separate C string and is compared at full length. This matters for a
    // localized default longer than the destination array (notably RankName).
    let default = c4_c_string_bytes(default, usize::MAX);
    if value != default {
        push_bytes(lines, name, value);
    }
}

fn push_bytes(lines: &mut Vec<Vec<u8>>, name: &str, value: Vec<u8>) {
    let mut line = Vec::with_capacity(name.len() + 1 + value.len());
    line.extend_from_slice(name.as_bytes());
    line.push(b'=');
    line.extend_from_slice(&value);
    lines.push(line);
}

fn push_escaped_c4_string(lines: &mut Vec<Vec<u8>>, name: &str, value: &str) {
    let bytes = c4_c_string_bytes(value, usize::MAX);
    let mut escaped = Vec::with_capacity(bytes.len() + 2);
    escaped.push(b'"');
    let mut previous_was_numeric_escape = false;
    for byte in bytes {
        let printable = (b' '..=b'~').contains(&byte);
        if printable
            && byte != b'\\'
            && byte != b'"'
            && !(previous_was_numeric_escape && byte.is_ascii_digit())
        {
            escaped.push(byte);
            previous_was_numeric_escape = false;
            continue;
        }
        previous_was_numeric_escape = false;
        match byte {
            0x07 => escaped.extend_from_slice(b"\\a"),
            0x08 => escaped.extend_from_slice(b"\\b"),
            0x0c => escaped.extend_from_slice(b"\\f"),
            b'\n' => escaped.extend_from_slice(b"\\n"),
            b'\r' => escaped.extend_from_slice(b"\\r"),
            b'\t' => escaped.extend_from_slice(b"\\t"),
            0x0b => escaped.extend_from_slice(b"\\v"),
            b'"' => escaped.extend_from_slice(b"\\\""),
            b'\\' => escaped.extend_from_slice(b"\\\\"),
            byte => {
                escaped.push(b'\\');
                escaped.extend_from_slice(format!("{byte:o}").as_bytes());
                previous_was_numeric_escape = true;
            }
        }
    }
    escaped.push(b'"');
    push_bytes(lines, name, escaped);
}

fn push_escaped_c4_string_default(
    lines: &mut Vec<Vec<u8>>,
    name: &str,
    value: &str,
    default: &str,
) {
    // StdStrBuf default comparison is length-aware, even though the INI
    // writer subsequently stops at the first NUL via strlen.
    if clonk_script::c4_string_bytes(value) != clonk_script::c4_string_bytes(default) {
        push_escaped_c4_string(lines, name, value);
    }
}

fn c4_c_string_bytes(value: &str, max_bytes: usize) -> Vec<u8> {
    let mut bytes = clonk_script::c4_string_bytes(value);
    if let Some(nul) = bytes.iter().position(|byte| *byte == 0) {
        bytes.truncate(nul);
    }
    bytes.truncate(max_bytes);
    bytes
}

fn write_ini_sections<const N: usize>(sections: [(&str, Vec<Vec<u8>>); N]) -> Vec<u8> {
    let mut output = Vec::new();
    for (name, lines) in sections {
        if lines.is_empty() {
            continue;
        }
        if !output.is_empty() {
            output.extend_from_slice(b"\r\n");
        }
        output.push(b'[');
        output.extend_from_slice(name.as_bytes());
        output.extend_from_slice(b"]\r\n");
        for line in lines {
            output.extend_from_slice(&line);
            output.extend_from_slice(b"\r\n");
        }
    }
    output
}

fn unique_crew_filename(name: &str, used: &[Vec<u8>]) -> Vec<u8> {
    let mut filename = make_filename_from_title(name);
    filename.extend_from_slice(b".c4i");
    while used
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(&filename))
    {
        filename.truncate(filename.len().saturating_sub(b".c4i".len()));
        let digit_start = filename
            .iter()
            .rposition(|byte| !byte.is_ascii_digit())
            .map_or(0, |index| index + 1);
        let number = std::str::from_utf8(&filename[digit_start..])
            .ok()
            .and_then(|digits| digits.parse::<i32>().ok())
            .unwrap_or(0)
            .wrapping_add(1);
        filename.truncate(digit_start);
        filename.extend_from_slice(number.to_string().as_bytes());
        filename.extend_from_slice(b".c4i");
    }
    filename
}

fn make_filename_from_title(title: &str) -> Vec<u8> {
    const STRIP: &[u8] = b"!\"\xa7%&/=?+*#:;<>\\.";
    // C4ObjectInfo::Save receives the fixed C4MaxName-sized Name field, not
    // an unbounded source string.
    let title = c4_c_string_bytes(title, 30);
    let mut filename = Vec::with_capacity(title.len());
    for byte in title {
        let whitespace = matches!(byte, b' ' | b'\t' | b'\r' | b'\n');
        let strip = if whitespace {
            filename.is_empty()
        } else {
            STRIP.contains(&byte)
        };
        if !strip {
            filename.push(byte);
        }
    }
    while filename
        .last()
        .is_some_and(|byte| matches!(*byte, b' ' | b'\t' | b'\r' | b'\n'))
    {
        filename.pop();
    }
    if filename.is_empty() {
        filename.extend_from_slice(b"unnamed");
    }
    filename
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_savegame_flag_controls_temporary_crew_omission() {
        let mut engine = Engine::new();
        let mut temporary = crate::Definition::from_script("TEMP", "Temporary", "")
            .expect("temporary definition compiles");
        temporary.temporary_crew = 1;
        engine
            .register_definition(temporary)
            .expect("temporary definition registers");
        engine
            .register_definition(
                crate::Definition::from_script("CREW", "Regular", "")
                    .expect("regular definition compiles"),
            )
            .expect("regular definition registers");

        assert!(!should_serialize_crew_definition(&engine, "TEMP", false));
        assert!(should_serialize_crew_definition(&engine, "TEMP", true));
        assert!(should_serialize_crew_definition(&engine, "CREW", false));
        assert!(
            should_serialize_crew_definition(&engine, "MISS", false),
            "C++ retains crew when C4Id2Def cannot resolve its definition"
        );
    }

    fn synchronized_crew(id: &str, filename: &str) -> CrewInfo {
        CrewInfo {
            id: id.to_string(),
            name: filename.trim_end_matches(".c4i").to_string(),
            death_message: String::new(),
            core: crate::CrewInfoCoreFields {
                original_filename: filename.to_string(),
                portrait_file: "Portrait1".to_string(),
                portrait_png: vec![1, 2, 3],
                portrait_overlay_png: vec![4, 5, 6],
                rank_png: vec![7, 8, 9],
                ..crate::CrewInfoCoreFields::default()
            },
            rank: 0,
            rank_name: "Clonk".to_string(),
            experience: 0,
            rounds: 0,
            physical: PhysicalInfo::default(),
            death_count: 0,
            total_playing_time: 0,
            birthday: 0,
            age: 0,
            participation: 1,
            in_action: false,
            was_in_action: false,
            in_action_time: 0,
            has_died: false,
            extra_data: Vec::new(),
            portraits: crate::CrewPortraitState::default(),
        }
    }

    #[test]
    fn explicit_tiny_flag_preserves_portrait_core_without_materializing_assets() {
        let mut engine = Engine::new();
        engine
            .register_player(crate::PlayerConfig::new(1, "Embedded"))
            .expect("player registers");
        let mut crew = synchronized_crew("MISS", "Crew.c4i");
        crew.core.portrait_file = "RetainedPortrait".to_string();
        crew.core.portrait_png.clear();
        crew.core.portrait_overlay_png.clear();
        crew.core.rank_png.clear();
        crew.portraits.permanent = crate::CrewPermanentPortrait::Assigned(crate::CrewPortrait {
            source: None,
            name: "custom".to_string(),
        });
        engine.crew_rosters.insert(1, vec![crew]);

        let tiny = serialize_live_c4_player_with_options(
            &engine,
            1,
            b"Embedded.c4p",
            b"Maker",
            LiveC4PlayerSaveOptions {
                store_tiny: true,
                ..LiveC4PlayerSaveOptions::default()
            },
        )
        .expect("tiny save must not materialize the pending custom portrait");
        let tiny = Group::from_raw_memory(
            std::path::PathBuf::from("Embedded.c4p"),
            tiny.pack_raw().expect("tiny player packs"),
        )
        .expect("tiny player opens");
        let crew = tiny.open_child("Crew.c4i").expect("crew group opens");
        let core = crew.read_file("ObjectInfo.txt").expect("crew core exists");
        assert!(core
            .windows(b"PortraitFile=RetainedPortrait".len())
            .any(|window| window == b"PortraitFile=RetainedPortrait"));
        assert!(!crew.exists("Portrait.png"));
        assert!(!crew.exists("PortraitOverlay.png"));
        assert!(!crew.exists("Rank.png"));

        assert!(
            matches!(
                serialize_live_c4_player(&engine, 1, b"Embedded.c4p", b"Maker"),
                Err(LiveC4PlayerError::UnreconstructablePortrait { .. })
            ),
            "embedded C4GameSave groups pass the non-tiny object-info flag"
        );
    }

    fn synchronized_object_info(info: &CrewInfo) -> crate::CrewObjectInfo {
        crate::CrewObjectInfo {
            definition_id: crate::DefinitionId::from(info.id.as_str()),
            name: info.name.clone(),
            death_message: info.death_message.clone(),
            core: info.core.clone(),
            rank: info.rank,
            rank_name: info.rank_name.clone(),
            experience: info.experience,
            participation: info.participation,
            rounds: info.rounds,
            death_count: info.death_count,
            total_playing_time: info.total_playing_time,
            birthday: info.birthday,
            age: info.age,
            in_action_time: info.in_action_time,
            extra_data: info.extra_data.clone(),
            portraits: info.portraits.clone(),
        }
    }

    #[test]
    fn local_profile_crew_rename_respects_existing_target_and_new_name_numbering() {
        let mut renamed = synchronized_crew("GOOD", "Old.c4i");
        renamed.name = "New".to_string();
        let mut entries = vec![b"Old.c4i".to_vec()];
        assert_eq!(
            resolve_local_profile_crew_filename(&renamed, &mut entries),
            b"New.c4i"
        );
        assert_eq!(entries, vec![b"New.c4i".to_vec()]);

        let mut blocked = synchronized_crew("GOOD", "Old.c4i");
        blocked.name = "New".to_string();
        let mut entries = vec![b"Old.c4i".to_vec(), b"New.c4i".to_vec()];
        assert_eq!(
            resolve_local_profile_crew_filename(&blocked, &mut entries),
            b"Old.c4i",
            "an occupied target suppresses C4Group::Rename"
        );

        let mut fresh = synchronized_crew("GOOD", "");
        fresh.name = "New".to_string();
        let mut entries = vec![b"New.c4i".to_vec(), b"New1.c4i".to_vec()];
        assert_eq!(
            resolve_local_profile_crew_filename(&fresh, &mut entries),
            b"New2.c4i"
        );
    }

    #[test]
    fn local_profile_serialization_reports_native_omitted_asset_cleanup() {
        let mut engine = Engine::new();
        engine
            .register_definition(
                crate::Definition::from_script("GOOD", "Crew", "").expect("definition compiles"),
            )
            .expect("definition registers");
        engine
            .register_player(crate::PlayerConfig::new(1, "Local").with_player_info_id(7))
            .expect("player registers");
        let mut live_crew = synchronized_crew("GOOD", "Old Crew.c4i");
        live_crew.name = "Renamed Crew".to_string();
        // C4Portrait_Custom is compared with case-sensitive SEqual. An
        // uppercase specification is not custom and must allow deletion.
        live_crew.core.portrait_file = "CUSTOM".to_string();
        engine.crew_rosters.insert(1, vec![live_crew]);
        let mut original = MutableGroup::new("Local.c4p");
        let mut original_crew = MutableGroup::new("Old Crew.c4i");
        original_crew
            .add_file("ObjectInfo.txt", b"old core".to_vec())
            .expect("original crew core");
        original
            .add_child("Old Crew.c4i", original_crew)
            .expect("original crew");
        let original = Group::from_raw_memory(
            std::path::PathBuf::from("Local.c4p"),
            original.pack_raw().expect("original profile packs"),
        )
        .expect("original profile opens");

        let synchronized = serialize_live_c4_player_for_synchronization(
            &mut engine,
            1,
            b"Local.c4p",
            b"Maker",
            true,
            Some(&original),
            LiveC4PlayerSaveOptions {
                savegame: false,
                save_default_portraits: false,
                ..LiveC4PlayerSaveOptions::default()
            },
        )
        .expect("local profile serializes");
        assert_eq!(
            synchronized.crew_cleanup,
            vec![LiveC4CrewProfileCleanup {
                filename: b"Renamed Crew.c4i".to_vec(),
                original_filename: b"Old Crew.c4i".to_vec(),
                roster_index: 0,
                remove_default_portrait_png: true,
                remove_rank_png: true,
            }]
        );
        let reopened = clonk_resources::Group::from_raw_memory(
            std::path::PathBuf::from("Local.c4p"),
            synchronized.group.pack_raw().expect("profile packs"),
        )
        .expect("profile opens");
        let crew = reopened.open_child("Renamed Crew.c4i").expect("crew opens");
        assert!(!crew.exists("Portrait.png"));
        assert!(!crew.exists("PortraitOverlay.png"));
        assert!(!crew.exists("Rank.png"));
        assert_eq!(
            engine.crew_rosters[&1][0].core.original_filename,
            "Renamed Crew.c4i"
        );
    }

    #[test]
    fn local_profile_without_original_is_fresh_full_profile() {
        let mut engine = Engine::new();
        engine
            .register_definition(
                crate::Definition::from_script("GOOD", "Crew", "").expect("definition compiles"),
            )
            .expect("definition registers");
        engine
            .register_player(crate::PlayerConfig::new(1, "Local").with_player_info_id(7))
            .expect("player registers");
        let mut live_crew = synchronized_crew("GOOD", "Old Crew.c4i");
        live_crew.name = "Renamed Crew".to_string();
        engine.crew_rosters.insert(1, vec![live_crew]);

        let synchronized = serialize_live_c4_player_for_synchronization(
            &mut engine,
            1,
            b"Local.c4p",
            b"Maker",
            true,
            None,
            LiveC4PlayerSaveOptions {
                savegame: false,
                ..LiveC4PlayerSaveOptions::default()
            },
        )
        .expect("a missing copied profile falls back to a fresh local group");
        assert_eq!(
            synchronized.crew_cleanup,
            vec![LiveC4CrewProfileCleanup {
                filename: b"Old Crew.c4i".to_vec(),
                original_filename: b"Old Crew.c4i".to_vec(),
                roster_index: 0,
                remove_default_portrait_png: false,
                remove_rank_png: true,
            }],
            "without a copied source entry, C4Group::Rename cannot adopt the name-derived filename"
        );
        let reopened = clonk_resources::Group::from_raw_memory(
            std::path::PathBuf::from("Local.c4p"),
            synchronized.group.pack_raw().expect("profile packs"),
        )
        .expect("profile opens");
        assert!(!reopened.exists("Renamed Crew.c4i"));
        let crew = reopened.open_child("Old Crew.c4i").expect("crew opens");
        assert_eq!(crew.read_file("Portrait.png").unwrap(), vec![1, 2, 3]);
        assert_eq!(
            crew.read_file("PortraitOverlay.png").unwrap(),
            vec![4, 5, 6]
        );
        assert_eq!(
            engine.crew_rosters[&1][0].core.original_filename,
            "Old Crew.c4i"
        );
    }

    #[test]
    fn remote_profile_serialization_is_fresh_tiny_and_strips_missing_defs() {
        let mut engine = Engine::new();
        engine
            .register_definition(
                crate::Definition::from_script("GOOD", "Crew", "").expect("definition compiles"),
            )
            .expect("definition registers");
        let mut temporary =
            crate::Definition::from_script("TEMP", "Temporary", "").expect("definition compiles");
        temporary.temporary_crew = 1;
        engine
            .register_definition(temporary)
            .expect("temporary definition registers");
        engine
            .register_definition(
                crate::Definition::from_script("MISS", "Missing", "")
                    .expect("missing definition compiles"),
            )
            .expect("missing definition initially registers");
        engine
            .register_player(crate::PlayerConfig::new(2, "Remote").with_player_info_id(8))
            .expect("player registers");
        let roster = vec![
            synchronized_crew("GOOD", "Valid.c4i"),
            synchronized_crew("MISS", "Missing.c4i"),
            synchronized_crew("TEMP", "Temporary.c4i"),
            synchronized_crew("GOOD", "Second.c4i"),
        ];
        engine.crew_rosters.insert(2, roster.clone());
        engine.crew_info_order.insert(2, vec![3, 1, 2, 0]);
        let valid = crate::ObjectId::new(101);
        let missing = engine
            .spawn_object(crate::SpawnConfig::new("MISS"))
            .expect("temporarily resolved crew object spawns");
        let missing_index = engine
            .find_object_index(missing)
            .expect("temporarily resolved crew object exists");
        engine.objects[missing_index].state.info_physical = Some(PhysicalInfo {
            walk: 99,
            ..PhysicalInfo::default()
        });
        engine
            .definitions
            .remove(&crate::DefinitionId::from("MISS"));
        let temporary = crate::ObjectId::new(103);
        let second = crate::ObjectId::new(104);
        for (object_id, roster_index) in [(valid, 0), (missing, 1), (temporary, 2), (second, 3)] {
            engine.crew_info_control_counts.insert(
                crate::CrewInfoLink {
                    player_id: 2,
                    roster_index,
                },
                10 + roster_index as i32,
            );
            Rc::make_mut(&mut engine.crew_info_links).insert(
                object_id,
                crate::CrewInfoLink {
                    player_id: 2,
                    roster_index,
                },
            );
            Rc::make_mut(&mut engine.crew_object_infos)
                .insert(object_id, synchronized_object_info(&roster[roster_index]));
            Rc::make_mut(&mut engine.crew_ranks).insert(object_id.as_u64(), roster_index as i32);
        }

        let synchronized = serialize_live_c4_player_for_synchronization(
            &mut engine,
            2,
            b"Remote.c4p",
            b"Maker",
            false,
            None,
            LiveC4PlayerSaveOptions {
                savegame: false,
                ..LiveC4PlayerSaveOptions::default()
            },
        )
        .expect("remote profile serializes");
        assert_eq!(
            engine.crew_rosters[&2]
                .iter()
                .map(|info| (info.id.as_str(), info.core.original_filename.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("GOOD", "Valid.c4i"),
                ("TEMP", "Temporary.c4i"),
                ("GOOD", "Second.c4i"),
            ],
            "remote Strip permanently removes only unresolved definitions"
        );
        assert_eq!(engine.crew_info_order[&2], vec![2, 1, 0]);
        assert_eq!(engine.crew_info_links[&valid].roster_index, 0);
        assert_eq!(engine.crew_info_links[&temporary].roster_index, 1);
        assert_eq!(engine.crew_info_links[&second].roster_index, 2);
        assert!(!engine.crew_info_links.contains_key(&missing));
        assert_eq!(
            engine
                .object_snapshot(missing)
                .expect("detached object remains live")
                .info_physical,
            None,
            "stripping its deleted C4ObjectInfo also drops trained physicals"
        );
        assert_eq!(
            engine.crew_info_control_counts,
            [
                (
                    crate::CrewInfoLink {
                        player_id: 2,
                        roster_index: 0,
                    },
                    10,
                ),
                (
                    crate::CrewInfoLink {
                        player_id: 2,
                        roster_index: 1,
                    },
                    12,
                ),
                (
                    crate::CrewInfoLink {
                        player_id: 2,
                        roster_index: 2,
                    },
                    13,
                ),
            ]
            .into_iter()
            .collect(),
            "runtime-only C4ObjectInfo counters follow the compacted pointers"
        );
        assert!(!engine.crew_object_infos.contains_key(&missing));
        assert!(!engine.crew_ranks.contains_key(&missing.as_u64()));
        for object_id in [valid, temporary, second] {
            let link = engine.crew_info_links[&object_id];
            assert_eq!(
                engine.crew_object_infos[&object_id].name,
                engine.crew_rosters[&link.player_id][link.roster_index].name
            );
        }
        assert!(synchronized.crew_cleanup.is_empty());
        let reopened = clonk_resources::Group::from_raw_memory(
            std::path::PathBuf::from("Remote.c4p"),
            synchronized.group.pack_raw().expect("profile packs"),
        )
        .expect("profile opens");
        assert_eq!(reopened.entries().expect("enumerate profile").len(), 3);
        assert!(reopened.exists("Player.txt"));
        assert!(reopened.exists("Valid.c4i"));
        assert!(reopened.exists("Second.c4i"));
        assert!(!reopened.exists("Missing.c4i"));
        assert!(!reopened.exists("Temporary.c4i"));
        let crew = reopened.open_child("Valid.c4i").expect("crew opens");
        assert_eq!(crew.entries().expect("enumerate crew").len(), 1);
        assert!(crew.exists("ObjectInfo.txt"));
    }

    #[test]
    fn player_core_uses_cpp_field_order_and_compile_defaults() {
        let mut player = PlayerState::default();
        player.name = "Runtime name".to_string();
        player.score = 9;
        player.rounds = 4;
        player.rounds_won = 3;
        player.rounds_lost = 1;
        player.total_playing_time = 77;
        player.extra_data = vec![
            ("Key".to_string(), clonk_script::Value::Int(12)),
            ("Flag".to_string(), clonk_script::Value::Bool(false)),
        ];
        player.player_info_core = Some(PlayerInfoCoreState {
            pref_name: "Ala Kadabra".to_string(),
            comment: "Ready".to_string(),
            rank: 2,
            rank_name: "Captain".to_string(),
            pref_color: 3,
            pref_color_dw: 0x12_34_56,
            pref_color2_dw: 0x65_43_21,
            pref_control: 0,
            pref_control_style: true,
            pref_auto_context_menu: false,
            pref_position: 4,
            pref_mouse: false,
            last_round: crate::player_file::PlayerLastRoundState {
                title: "Mine \"A\"\n".to_string(),
                date: 123,
                duration: 50,
                won: 1,
                score: 7,
                final_score: 8,
                total_score: 17,
                bonus: 1,
                level: 3,
            },
            ..PlayerInfoCoreState::default()
        });

        assert_eq!(
            serialize_player_core(&player, "Rank", PlayerValueEncoding::CurrentIds).unwrap(),
            b"[Player]\r\nName=Ala Kadabra\r\nComment=Ready\r\nRank=2\r\nRankName=Captain\r\nScore=9\r\nRounds=4\r\nRoundsWon=3\r\nRoundsLost=1\r\nTotalPlayingTime=77\r\nExtraData=2;Key=i12,Flag=b0\r\n\r\n[Preferences]\r\nColor=3\r\nColorDw=1193046\r\nAlternateColorDw=6636321\r\nControl=0\r\nAutoStopControl=1\r\nAutoContextMenu=0\r\nPosition=4\r\nMouse=0\r\n\r\n[LastRound]\r\nTitle=\"Mine \\\"A\\\"\\n\"\r\nDate=123\r\nDuration=50\r\nWon=1\r\nScore=7\r\nFinalScore=8\r\nTotalScore=17\r\nBonus=1\r\nLevel=3\r\n"
        );
    }

    #[test]
    fn fileless_player_core_keeps_cpp_defaults_instead_of_runtime_identity() {
        let player = PlayerState {
            name: "Runtime script alias".to_string(),
            color_index: Some(9),
            control_set: 7,
            score: 12,
            ..PlayerState::default()
        };

        let serialized = serialize_player_core(&player, "Rank", PlayerValueEncoding::CurrentIds)
            .expect("player core serializes");
        assert!(!serialized
            .windows(b"Runtime script alias".len())
            .any(|window| window == b"Runtime script alias"));
        assert!(!serialized.starts_with(b"Name="));
        assert!(!serialized
            .windows(b"\r\nName=".len())
            .any(|window| window == b"\r\nName="));
        assert!(serialized
            .windows(b"RankName=Rang".len())
            .any(|window| window == b"RankName=Rang"));
        assert!(serialized
            .windows(b"Score=12".len())
            .any(|window| window == b"Score=12"));
        assert!(serialized
            .windows(b"Control=0".len())
            .any(|window| window == b"Control=0"));
        assert!(!serialized
            .windows(b"Control=7".len())
            .any(|window| window == b"Control=7"));
    }

    #[test]
    fn object_info_serializes_core_physical_and_extra_data_in_cpp_order() {
        let info = CrewInfo {
            id: "CLNK".to_string(),
            name: "Veteran".to_string(),
            death_message: "Gone".to_string(),
            core: crate::CrewInfoCoreFields {
                portrait_file: "custom".to_string(),
                next_rank_name: "Captain".to_string(),
                type_name: "Clonk".to_string(),
                next_rank_exp: 5_196,
                ..crate::CrewInfoCoreFields::default()
            },
            rank: 2,
            rank_name: "Lieutenant".to_string(),
            experience: 900,
            rounds: 6,
            physical: PhysicalInfo {
                energy: 80_000,
                walk: 35_000,
                can_dig: 1,
                ..PhysicalInfo::default()
            },
            death_count: 7,
            total_playing_time: 17_999,
            birthday: 123,
            age: 7,
            participation: 1,
            in_action: true,
            was_in_action: true,
            in_action_time: 50,
            has_died: false,
            extra_data: vec![(
                "Badge".to_string(),
                clonk_script::Value::C4Id("GOLD".to_string()),
            )],
            portraits: crate::CrewPortraitState::default(),
        };

        assert_eq!(
            serialize_object_info(&info, PlayerValueEncoding::CurrentIds).unwrap(),
            b"[ObjectInfo]\r\nid=CLNK\r\nName=Veteran\r\nDeathMessage=Gone\r\nPortraitFile=custom\r\nRank=2\r\nRankName=\"Lieutenant\"\r\nNextRankName=\"Captain\"\r\nExperience=900\r\nNextRankExp=5196\r\nRounds=6\r\nDeathCount=7\r\nBirthday=123\r\nTotalPlayingTime=17999\r\nAge=7\r\nExtraData=1;Badge=I1145851719\r\n\r\n[Physical]\r\nEnergy=80000\r\nWalk=35000\r\nCanDig=1\r\n"
        );
    }

    #[test]
    fn fresh_player_group_round_trips_nondefault_core_and_retained_crew_assets() {
        let second = clonk_script::C4StringValue::loaded("second".to_string(), 0);
        let key = clonk_script::C4StringValue::loaded("key".to_string(), 1);
        let first = clonk_script::C4StringValue::loaded("first".to_string(), 2);
        let crew_value = clonk_script::C4StringValue::loaded("crew value".to_string(), 3);
        let mut player = PlayerState {
            id: 7,
            name: "Runtime alias".to_string(),
            score: 44,
            rounds: 5,
            rounds_won: 4,
            rounds_lost: 1,
            total_playing_time: 600,
            extra_data: vec![(
                "Live".to_string(),
                clonk_script::Value::Array(vec![
                    clonk_script::Value::String(second),
                    clonk_script::Value::Object(42),
                    clonk_script::Value::Proplist(clonk_script::ValueMap::from([(
                        clonk_script::Value::String(key),
                        clonk_script::Value::String(first),
                    )])),
                ]),
            )],
            player_info_core: Some(PlayerInfoCoreState {
                pref_name: "Profile name".to_string(),
                comment: "Comment".to_string(),
                rank: 3,
                rank_name: "Major".to_string(),
                pref_color: 5,
                pref_color_dw: 0x11_22_33,
                pref_color2_dw: 0x44_55_66,
                pref_control: 6,
                pref_control_style: true,
                pref_auto_context_menu: true,
                pref_position: 2,
                pref_mouse: false,
                last_round: crate::player_file::PlayerLastRoundState {
                    title: "Round one".to_string(),
                    date: 42,
                    duration: 300,
                    won: 1,
                    score: 10,
                    final_score: 110,
                    total_score: 144,
                    bonus: 100,
                    level: 0,
                },
                ..PlayerInfoCoreState::default()
            }),
            ..PlayerState::default()
        };
        // The runtime fields are the inherited C4PlayerInfoCore counters and
        // deliberately override stale retained values.
        player.score = 44;

        let portrait = encode_rgba_png(1, 1, &[1, 2, 3, 255], "test portrait").unwrap();
        let crew = CrewInfo {
            id: "CLNK".to_string(),
            name: "Renamed Hero".to_string(),
            death_message: String::new(),
            core: crate::CrewInfoCoreFields {
                original_filename: "Old Hero.c4i".to_string(),
                portrait_file: "custom".to_string(),
                portrait_png: portrait.clone(),
                ..crate::CrewInfoCoreFields::default()
            },
            rank: 1,
            rank_name: "Private".to_string(),
            experience: 20,
            rounds: 2,
            physical: PhysicalInfo::default(),
            death_count: 0,
            total_playing_time: 10,
            birthday: 11,
            age: 0,
            participation: 1,
            in_action: false,
            was_in_action: false,
            in_action_time: 0,
            has_died: false,
            extra_data: vec![("Crew".to_string(), clonk_script::Value::String(crew_value))],
            portraits: crate::CrewPortraitState::default(),
        };

        let mut state = Engine::new().capture_state();
        state.players.push(player.clone());
        state.crew_info_rosters.insert(player.id, vec![crew]);
        state.crew_info_order.insert(player.id, vec![0]);
        let group =
            serialize_live_c4_player_state(&state, &player, b"Profile.c4p", b"Test Maker").unwrap();
        let packed = group.pack_raw().unwrap();
        let reopened =
            clonk_resources::Group::from_memory(std::path::PathBuf::from("Profile.c4p"), packed)
                .unwrap();
        let strings = clonk_script::new_string_registrations();
        for (id, value) in [(0, "second"), (1, "key"), (2, "first"), (3, "crew value")] {
            clonk_script::register_loaded_c4_string(&strings, id, value);
        }
        let resolution = crate::player_file::PersistedC4ValueResolution {
            strings,
            object_numbers: std::collections::HashSet::from([42]),
        };
        let loaded = crate::player_file::PlayerFile::load_with_portraits_and_value_resolution(
            &reopened,
            true,
            &resolution,
        )
        .unwrap();

        assert_eq!(loaded.name, "Profile name");
        assert_eq!(loaded.info_core.comment, "Comment");
        assert_eq!(loaded.info_core.rank, 3);
        assert_eq!(loaded.info_core.rank_name, "Major");
        assert_eq!(loaded.score, 44);
        assert_eq!(loaded.info_core.extra_data, player.extra_data);
        assert_eq!(loaded.pref_color, 5);
        assert_eq!(loaded.pref_color_dw, 0x11_22_33);
        assert_eq!(loaded.pref_color2_dw, 0x44_55_66);
        assert_eq!(loaded.info_core.last_round.total_score, 144);
        assert_eq!(loaded.crew.len(), 1);
        assert_eq!(loaded.crew[0].core.original_filename, "Old Hero.c4i");
        assert_eq!(loaded.crew[0].core.portrait_png, portrait);
        assert_eq!(
            loaded.crew[0].extra_data,
            vec![(
                "Crew".to_string(),
                clonk_script::Value::String("crew value".to_string().into())
            )]
        );
    }

    #[test]
    fn standalone_player_save_writes_current_string_ids_without_enumerating() {
        let runtime = clonk_script::C4StringValue::from("runtime");
        let loaded = clonk_script::C4StringValue::loaded("loaded".to_string(), 7);
        let player = PlayerState {
            id: 7,
            extra_data: vec![
                (
                    "Runtime".to_string(),
                    clonk_script::Value::String(runtime.clone()),
                ),
                (
                    "Loaded".to_string(),
                    clonk_script::Value::String(loaded.clone()),
                ),
            ],
            ..PlayerState::default()
        };
        let mut state = Engine::new().capture_state();
        state.players.push(player.clone());

        let group = serialize_live_c4_player_state(&state, &player, b"Profile.c4p", b"Test Maker")
            .expect("standalone player serializes");

        assert_eq!(
            runtime.enum_id(),
            -1,
            "save must not enumerate runtime strings"
        );
        assert_eq!(loaded.enum_id(), 7, "save must retain loaded string IDs");
        let reopened = clonk_resources::Group::from_memory(
            std::path::PathBuf::from("Profile.c4p"),
            group.pack_raw().expect("player group packs"),
        )
        .expect("player group reopens");
        let player_txt = reopened.read_file("Player.txt").expect("Player.txt exists");
        assert!(player_txt
            .windows(b"ExtraData=2;Runtime=S-1,Loaded=S7".len())
            .any(|window| window == b"ExtraData=2;Runtime=S-1,Loaded=S7"));
    }

    #[test]
    fn filename_generation_matches_cpp_stripping_and_collision_numbering() {
        let first = unique_crew_filename("  A.l! ice ", &[]);
        let second = unique_crew_filename("Alice", std::slice::from_ref(&first));
        let third = unique_crew_filename("Alice1", &[first.clone(), second.clone()]);

        assert_eq!(first, b"Al ice.c4i");
        assert_eq!(second, b"Alice.c4i");
        assert_eq!(third, b"Alice1.c4i");

        let collision = unique_crew_filename("A.l! ice", &[first]);
        assert_eq!(collision, b"Al ice1.c4i");
    }

    #[test]
    fn c4_value_map_keeps_native_types_and_signed_ids() {
        let entries = vec![
            ("nil".to_string(), clonk_script::Value::Nil),
            ("int".to_string(), clonk_script::Value::Int(-7)),
            ("bool".to_string(), clonk_script::Value::Bool(true)),
            ("raw_bool".to_string(), clonk_script::Value::RawBool(7)),
            (
                "id".to_string(),
                clonk_script::Value::C4Id(clonk_script::c4_id_from_raw(0xffff_fffe)),
            ),
            (
                "zero_id".to_string(),
                clonk_script::Value::C4Id(clonk_script::c4_id_from_raw(0)),
            ),
        ];
        assert_eq!(
            named_value_map_line(
                "ExtraData",
                &entries,
                "test",
                PlayerValueEncoding::CurrentIds,
            )
            .unwrap(),
            b"ExtraData=6;nil=A0,int=i-7,bool=b1,raw_bool=b7,id=I-2,zero_id=I0"
        );
    }

    #[test]
    fn c4_value_map_reuses_scenario_ids_for_nested_live_values() {
        let first = clonk_script::C4StringValue::from("first");
        let second = clonk_script::C4StringValue::from("second");
        let key = clonk_script::C4StringValue::from("key");
        let enumeration = LiveC4ValueEnumeration::from_strings_in_id_order([
            first.clone(),
            second.clone(),
            key.clone(),
        ]);
        let entries = vec![(
            "Complex".to_string(),
            clonk_script::Value::Array(vec![
                clonk_script::Value::String(second),
                clonk_script::Value::Object(42),
                clonk_script::Value::Proplist(clonk_script::ValueMap::from([(
                    clonk_script::Value::String(key),
                    clonk_script::Value::String(first),
                )])),
            ]),
        )];

        assert_eq!(
            named_value_map_line(
                "ExtraData",
                &entries,
                "test",
                PlayerValueEncoding::Synchronized(&enumeration),
            )
            .unwrap(),
            b"ExtraData=1;Complex=a[3;S1,O42,m[1;S2=S0]]"
        );
    }

    #[test]
    fn player_core_preserves_noncanonical_integer_preferences() {
        let player = PlayerState {
            player_info_core: Some(PlayerInfoCoreState {
                pref_control_style: true,
                pref_control_style_value: 2,
                pref_auto_context_menu: true,
                pref_auto_context_menu_value: -2,
                pref_mouse: true,
                pref_mouse_value: 7,
                ..PlayerInfoCoreState::default()
            }),
            ..PlayerState::default()
        };

        let serialized = serialize_player_core(&player, "Rank", PlayerValueEncoding::CurrentIds)
            .expect("player core serializes");
        assert!(serialized
            .windows(b"AutoStopControl=2".len())
            .any(|window| { window == b"AutoStopControl=2" }));
        assert!(serialized
            .windows(b"AutoContextMenu=-2".len())
            .any(|window| { window == b"AutoContextMenu=-2" }));
        assert!(serialized
            .windows(b"Mouse=7".len())
            .any(|window| window == b"Mouse=7"));
    }

    #[test]
    fn player_rank_name_uses_the_process_local_compile_default() {
        let player = PlayerState {
            player_info_core: Some(PlayerInfoCoreState {
                rank_name: "Dienstgrad".to_string(),
                ..PlayerInfoCoreState::default()
            }),
            ..PlayerState::default()
        };

        let localized =
            serialize_player_core(&player, "Dienstgrad", PlayerValueEncoding::CurrentIds)
                .expect("localized player core serializes");
        assert!(!localized
            .windows(b"RankName=".len())
            .any(|window| window == b"RankName="));

        let english = serialize_player_core(&player, "Rank", PlayerValueEncoding::CurrentIds)
            .expect("English-default player core serializes");
        assert!(english
            .windows(b"RankName=Dienstgrad".len())
            .any(|window| window == b"RankName=Dienstgrad"));
    }

    #[test]
    fn overlong_localized_rank_default_is_not_truncated_for_comparison() {
        let bounded = "R".repeat(30);
        let localized_default = format!("{bounded} suffix");
        let player = PlayerState {
            player_info_core: Some(PlayerInfoCoreState {
                rank_name: bounded.clone(),
                ..PlayerInfoCoreState::default()
            }),
            ..PlayerState::default()
        };

        let serialized =
            serialize_player_core(&player, &localized_default, PlayerValueEncoding::CurrentIds)
                .expect("player core serializes");
        let expected = format!("RankName={bounded}\r\n");
        assert!(serialized
            .windows(expected.len())
            .any(|window| window == expected.as_bytes()));
    }

    #[test]
    fn stdstrbuf_defaults_compare_past_the_first_nul_before_writing() {
        let info = CrewInfo {
            id: "CLNK".to_string(),
            name: "Nul rank".to_string(),
            death_message: String::new(),
            rank_name: "Clonk\0retained suffix".to_string(),
            core: crate::CrewInfoCoreFields {
                next_rank_name: "\0retained suffix".to_string(),
                ..crate::CrewInfoCoreFields::default()
            },
            rank: 0,
            experience: 0,
            rounds: 0,
            physical: PhysicalInfo::default(),
            death_count: 0,
            total_playing_time: 0,
            birthday: 0,
            age: 0,
            participation: 1,
            in_action: false,
            was_in_action: false,
            in_action_time: 0,
            has_died: false,
            extra_data: Vec::new(),
            portraits: crate::CrewPortraitState::default(),
        };

        let serialized = serialize_object_info(&info, PlayerValueEncoding::CurrentIds)
            .expect("crew core serializes");
        assert!(serialized
            .windows(b"RankName=\"Clonk\"\r\n".len())
            .any(|window| window == b"RankName=\"Clonk\"\r\n"));
        assert!(serialized
            .windows(b"NextRankName=\"\"\r\n".len())
            .any(|window| window == b"NextRankName=\"\"\r\n"));
    }

    #[test]
    fn generated_crew_filename_uses_the_bounded_c4_name_field() {
        let bounded = "A".repeat(30);
        let overlong = format!("{bounded}B and ignored");
        let expected = format!("{bounded}.c4i").into_bytes();
        assert_eq!(unique_crew_filename(&overlong, &[]), expected);
    }

    #[test]
    fn portrait_save_options_match_cpp_new_portrait_gates() {
        fn retained_custom() -> CrewInfo {
            CrewInfo {
                id: "CLNK".to_string(),
                name: "Portrait owner".to_string(),
                death_message: String::new(),
                core: crate::CrewInfoCoreFields {
                    portrait_file: "custom".to_string(),
                    portrait_png: vec![1, 2, 3],
                    portrait_overlay_png: vec![4, 5, 6],
                    portrait_bmp: vec![7, 8, 9],
                    ..crate::CrewInfoCoreFields::default()
                },
                rank: 0,
                rank_name: "Clonk".to_string(),
                experience: 0,
                rounds: 0,
                physical: PhysicalInfo::default(),
                death_count: 0,
                total_playing_time: 0,
                birthday: 0,
                age: 0,
                participation: 1,
                in_action: false,
                was_in_action: false,
                in_action_time: 0,
                has_died: false,
                extra_data: Vec::new(),
                portraits: crate::CrewPortraitState::default(),
            }
        }

        let engine = Engine::new();
        let mut save_disabled = retained_custom();
        materialize_live_portrait(
            &engine,
            &mut save_disabled,
            LiveC4PlayerSaveOptions {
                save_default_portraits: false,
                ..LiveC4PlayerSaveOptions::default()
            },
        )
        .expect("disabled portrait save is valid");
        assert_eq!(save_disabled.core.portrait_file, "custom");
        assert!(save_disabled.core.portrait_png.is_empty());
        assert!(save_disabled.core.portrait_overlay_png.is_empty());
        assert!(save_disabled.core.portrait_bmp.is_empty());

        let mut add_disabled = retained_custom();
        materialize_live_portrait(
            &engine,
            &mut add_disabled,
            LiveC4PlayerSaveOptions {
                add_new_crew_portraits: false,
                ..LiveC4PlayerSaveOptions::default()
            },
        )
        .expect("disabled default-portrait addition is valid");
        assert_eq!(add_disabled.core.portrait_file, "custom");
        assert!(add_disabled.core.portrait_png.is_empty());

        let mut explicit_none = retained_custom();
        explicit_none.portraits.permanent = crate::CrewPermanentPortrait::ExplicitNone;
        materialize_live_portrait(
            &engine,
            &mut explicit_none,
            LiveC4PlayerSaveOptions {
                add_new_crew_portraits: false,
                ..LiveC4PlayerSaveOptions::default()
            },
        )
        .expect("an explicit pending portrait bypasses AddNewCrewPortraits");
        assert_eq!(explicit_none.core.portrait_file, "none");
        assert!(explicit_none.core.portrait_png.is_empty());
    }

    #[test]
    fn network_player_save_does_not_project_the_current_playing_stint() {
        let mut engine = Engine::new();
        engine.game_time = 90;
        engine
            .register_player(
                crate::PlayerConfig::new(0, "Profile")
                    .with_player_info_id(1)
                    .with_total_playing_time(40),
            )
            .expect("player registers");
        engine.player_mut(0).expect("player").set_game_join_time(10);

        let ordinary = engine.capture_state();
        assert_eq!(ordinary.players[0].total_playing_time, 120);
        let network = engine.capture_state_for_network_save();
        assert_eq!(network.players[0].total_playing_time, 40);
    }
}
