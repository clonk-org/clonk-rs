//! Player restore-info projection for an exact runtime network save.
//!
//! This is the Rust counterpart of
//! `C4PlayerInfoList::SetAsRestoreInfos(PlayerInfos, true, true, true, true)`.

use clonk_engine::savegame_association::legacy_basename;
use std::collections::{BTreeSet, HashSet};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use clonk_engine::{
    ClientCoreControlData, ControlPlayerInfoEntry, LegacyCString, LiveC4SaveComponents,
    LiveC4SavePlayerPolicy, LiveC4SavePolicy, PLAYER_INFO_FLAG_HAS_RESOURCE,
    PLAYER_INFO_TYPE_SCRIPT, PLAYER_INFO_TYPE_USER,
};
use clonk_network::{
    LiveNetworkDynamic, LiveNetworkDynamicComponent, LiveNetworkDynamicSpec, PlayerInfoListSnapshot,
};
use clonk_resources::{Group, MutableGroup};

/// One live player group that must be serialized into the runtime dynamic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeJoinPlayerGroupTarget {
    pub client_id: i32,
    pub player_info_id: i32,
    pub player_type: u8,
    pub game_number: i32,
    pub filename: LegacyCString,
}

/// Restore infos plus the ordered player-group writes needed by the save.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeJoinRestorePlan {
    pub restore_infos: PlayerInfoListSnapshot,
    pub player_groups: Vec<RuntimeJoinPlayerGroupTarget>,
}

impl RuntimeJoinRestorePlan {
    /// Reject the impossible exact-save state where `SavePlayerInfos.txt`
    /// would retain a joined row without the corresponding `Game.txt`
    /// `Player<ID>` section.
    pub fn validate_for_live_save(
        &self,
        policy: LiveC4SavePolicy<'_>,
        live_player_info_ids: impl IntoIterator<Item = i32>,
    ) -> Result<()> {
        if !policy.is_exact() {
            return Ok(());
        }
        let live_player_info_ids = live_player_info_ids.into_iter().collect::<HashSet<_>>();
        let missing = self
            .restore_infos
            .clients
            .iter()
            .flat_map(|client| &client.players)
            .map(|player| player.id)
            .filter(|player| !live_player_info_ids.contains(player))
            .collect::<BTreeSet<_>>();
        anyhow::ensure!(
            missing.is_empty(),
            "exact save has joined SavePlayerInfos IDs without matching Game.txt Player<ID> sections: {}",
            missing
                .into_iter()
                .map(|player| player.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
        Ok(())
    }
}

/// One already-serialized small `.c4p` child selected by the restore plan.
#[derive(Debug)]
pub struct SerializedRuntimeJoinPlayerGroup {
    pub filename: LegacyCString,
    pub group: MutableGroup,
}

/// Clone live `PlayerInfos` into the exact restore-info shape used by a
/// runtime network save.
pub fn set_as_runtime_join_restore_infos(
    live_clients: &[ClientCoreControlData],
    player_infos: &PlayerInfoListSnapshot,
) -> RuntimeJoinRestorePlan {
    set_as_live_save_restore_infos(
        live_clients,
        player_infos,
        true,
        clonk_engine::LiveC4SavePolicy::RuntimeNetwork.player_policy(),
    )
}

/// General `C4PlayerInfoList::SetAsRestoreInfos` projection for scenario,
/// savegame, record, and runtime-network saves. `network_enabled` selects the
/// native user-player filename branch after the copied filename is cleared.
pub fn set_as_live_save_restore_infos(
    live_clients: &[ClientCoreControlData],
    player_infos: &PlayerInfoListSnapshot,
    network_enabled: bool,
    policy: LiveC4SavePlayerPolicy,
) -> RuntimeJoinRestorePlan {
    let mut restore_infos = player_infos.clone();
    let mut player_groups = Vec::new();

    restore_infos.clients.retain_mut(|client| {
        let client_id = client.client_id;
        let client_name = live_clients
            .iter()
            .find(|core| core.client_id == client_id)
            .map(|core| core.name.as_bytes())
            .unwrap_or(&b"Unknown"[..]);

        client.players.retain_mut(|player| {
            if !player.is_joined() {
                return false;
            }

            let (keep, embed, filename) = match player.player_type {
                PLAYER_INFO_TYPE_USER => {
                    // SetAsRestoreInfos clears Filename before this branch.
                    // Offline C++ then derives the basename from that already
                    // empty field, so no user child is written. Network saves
                    // instead use the published resource and client prefix.
                    let embed = policy.embed_user_player_files && network_enabled;
                    (
                        policy.save_user_players,
                        embed,
                        if embed {
                            runtime_user_player_filename(client_name, player)
                        } else {
                            LegacyCString::default()
                        },
                    )
                }
                PLAYER_INFO_TYPE_SCRIPT => {
                    let embed = policy.embed_script_player_files;
                    (
                        policy.save_script_players,
                        embed,
                        if embed {
                            runtime_script_player_filename(player.id)
                        } else {
                            LegacyCString::default()
                        },
                    )
                }
                _ => return false,
            };
            if !keep {
                return false;
            }

            player.filename = filename.clone();
            player.flags &= !PLAYER_INFO_FLAG_HAS_RESOURCE;
            player.resource = None;
            if embed {
                player_groups.push(RuntimeJoinPlayerGroupTarget {
                    client_id,
                    player_info_id: player.id,
                    player_type: player.player_type,
                    game_number: player.game_number,
                    filename,
                });
            }
            true
        });

        !client.players.is_empty()
    });

    RuntimeJoinRestorePlan {
        restore_infos,
        player_groups,
    }
}

/// Assemble the application-owned pieces of `C4GameSaveNetwork(false)` and
/// pack the resulting runtime dynamic.
///
/// `C4ComponentHost::Save` does nothing for an unmodified Title, Info or
/// Script component. `LiveC4SaveComponents` has already applied those
/// modified-bit gates, so optional values emitted here are authoritative.
pub fn compose_runtime_join_dynamic(
    group_filename: String,
    maker: Vec<u8>,
    parameters: Vec<u8>,
    save: LiveC4SaveComponents,
    restore_infos: &PlayerInfoListSnapshot,
    player_groups: Vec<SerializedRuntimeJoinPlayerGroup>,
) -> Result<LiveNetworkDynamic> {
    let LiveC4SaveComponents {
        scenario_txt,
        title_txt: _,
        game_txt,
        objects_txt,
        strings_txt,
        value_enumeration: _,
        landscape_bmp,
        landscape_png,
        diff_landscape_bmp,
        map_bmp,
        material_group,
        mat_map_txt,
        pxs_c4b,
        mass_mover_c4b,
        delete_sky_entry: _,
        teams_txt,
        round_results_txt,
        info_txt,
        script_c,
        deleted_components: _,
        component_host_mutations: _,
        scenario_sections,
        deleted_scenario_sections: _,
        scenario_section_mutations: _,
    } = save;

    let mut components = Vec::new();
    if let Some(info) = info_txt {
        push_file(&mut components, &info.name, info.payload);
    }
    if !game_txt.is_empty() {
        push_file(&mut components, "Game.txt", game_txt);
    }
    push_optional_file(&mut components, "Teams.txt", teams_txt);
    for section in scenario_sections {
        push_raw_child(&mut components, section.name, section.payload)?;
    }
    if let Some(raw_group) = material_group {
        push_raw_child(&mut components, "Material.c4g".to_string(), raw_group)?;
    }
    push_file(&mut components, "MatMap.txt", mat_map_txt);
    push_optional_file(&mut components, "Landscape.bmp", landscape_bmp);
    push_optional_file(&mut components, "Landscape.png", landscape_png);
    push_optional_file(&mut components, "DiffLandscape.bmp", diff_landscape_bmp);
    push_optional_file(&mut components, "Map.bmp", map_bmp);
    push_optional_file(&mut components, "PXS.c4b", pxs_c4b);
    push_optional_file(&mut components, "MassMover.c4b", mass_mover_c4b);
    push_optional_file(&mut components, "Strings.txt", strings_txt);
    push_file(&mut components, "Objects.txt", objects_txt);
    push_optional_file(&mut components, "RoundResults.txt", round_results_txt);
    if let Some(script) = script_c {
        push_file(&mut components, &script.name, script.payload);
    }

    if !restore_infos.clients.is_empty() {
        let restore_infos = clonk_network::encode_player_info_list_ini(restore_infos)
            .context("serialize runtime SavePlayerInfos.txt")?;
        push_file(&mut components, "SavePlayerInfos.txt", restore_infos);
    }
    for player in player_groups {
        let name = std::str::from_utf8(player.filename.as_bytes())
            .context("runtime player group filename is not ASCII-safe")?
            .to_owned();
        components.push(LiveNetworkDynamicComponent::Child {
            name,
            group: player.group,
        });
    }

    clonk_network::compose_live_network_dynamic(LiveNetworkDynamicSpec {
        group_filename,
        maker,
        parameters,
        scenario: scenario_txt,
        components,
    })
    .context("compose synchronized runtime network dynamic")
}

fn push_raw_child(
    components: &mut Vec<LiveNetworkDynamicComponent>,
    name: String,
    raw_group: Vec<u8>,
) -> Result<()> {
    let source = Group::from_raw_memory(PathBuf::from(&name), raw_group.clone())
        .with_context(|| format!("open serialized live {name} child"))?;
    let contents_crc = source
        .contents_crc()
        .with_context(|| format!("hash serialized live {name} child"))?;
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as u32;
    components.push(LiveNetworkDynamicComponent::PackedChild {
        name,
        raw_group,
        contents_crc,
        time,
        executable: false,
    });
    Ok(())
}

fn push_file(components: &mut Vec<LiveNetworkDynamicComponent>, name: &str, payload: Vec<u8>) {
    components.push(LiveNetworkDynamicComponent::File {
        name: name.to_string(),
        payload,
    });
}

fn push_optional_file(
    components: &mut Vec<LiveNetworkDynamicComponent>,
    name: &str,
    payload: Option<Vec<u8>>,
) {
    if let Some(payload) = payload {
        push_file(components, name, payload);
    }
}

fn runtime_user_player_filename(
    client_name: &[u8],
    player: &ControlPlayerInfoEntry,
) -> LegacyCString {
    // SetAsRestoreInfos clears the copied player-info filename before asking
    // GetLocalJoinFilename. The live resource therefore supplies the basename;
    // a malformed/resource-less joined user contributes an empty basename,
    // never the pre-copy filename.
    let source = player
        .resource
        .as_ref()
        .map(|resource| resource.filename.as_bytes())
        .unwrap_or_default();
    let basename = legacy_basename(source);
    let mut filename = Vec::with_capacity(client_name.len() + 1 + basename.len());
    filename.extend_from_slice(client_name);
    filename.push(b'-');
    filename.extend_from_slice(basename);
    LegacyCString::from_bytes(encode_local_group_filename(&filename))
        .expect("legacy client and player filenames cannot contain NUL")
}

fn runtime_script_player_filename(player_info_id: i32) -> LegacyCString {
    LegacyCString::from_bytes(format!("ScriptPlr-{player_info_id}.c4p").into_bytes())
        .expect("script restore filename is static ASCII")
}

fn encode_local_group_filename(filename: &[u8]) -> Vec<u8> {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    let mut encoded = Vec::with_capacity(filename.len());
    for byte in filename.iter().copied() {
        if byte == b'%' {
            encoded.extend_from_slice(b"%25");
        } else if byte >= 0x80 {
            encoded.push(b'%');
            encoded.push(HEX[usize::from(byte >> 4)]);
            encoded.push(HEX[usize::from(byte & 0x0f)]);
        } else {
            encoded.push(byte);
        }
    }
    encoded
}

#[cfg(all(
    test,
    any(not(feature = "app-test-shard-mode"), feature = "app-test-shard-5",),
))]
mod tests {
    use clonk_engine::{
        LiveC4SaveNamedComponent, NetworkResourceCore, PLAYER_INFO_FLAG_INVISIBLE,
        PLAYER_INFO_FLAG_JOINED, PLAYER_INFO_FLAG_NO_SCENARIO_INIT, PLAYER_INFO_FLAG_REMOVED,
        PLAYER_INFO_TYPE_NONE,
    };
    use clonk_network::ClientPlayerInfosSnapshot;

    use super::*;

    fn legacy(bytes: &[u8]) -> LegacyCString {
        LegacyCString::from_bytes(bytes.to_vec()).expect("test legacy string")
    }

    fn client_core(client_id: i32, name: &[u8]) -> ClientCoreControlData {
        ClientCoreControlData {
            client_id,
            name: legacy(name),
            ..Default::default()
        }
    }

    fn player(id: i32, player_type: u8, flags: u16, filename: &[u8]) -> ControlPlayerInfoEntry {
        ControlPlayerInfoEntry {
            id,
            player_type,
            flags,
            filename: legacy(filename),
            game_number: id + 100,
            name: legacy(b"Player"),
            ..Default::default()
        }
    }

    fn snapshot(clients: Vec<ClientPlayerInfosSnapshot>) -> PlayerInfoListSnapshot {
        PlayerInfoListSnapshot {
            last_player_id: 99,
            clients,
        }
    }

    fn live_save_components(
        modified_components: bool,
        material_group: Vec<u8>,
        scenario_section: Vec<u8>,
    ) -> LiveC4SaveComponents {
        LiveC4SaveComponents {
            scenario_txt: b"scenario".to_vec(),
            title_txt: None,
            game_txt: b"game".to_vec(),
            objects_txt: b"objects".to_vec(),
            strings_txt: Some(b"strings".to_vec()),
            value_enumeration: Default::default(),
            landscape_bmp: Some(b"landscape".to_vec()),
            landscape_png: None,
            diff_landscape_bmp: None,
            map_bmp: None,
            material_group: Some(material_group),
            mat_map_txt: b"matmap".to_vec(),
            pxs_c4b: None,
            mass_mover_c4b: None,
            delete_sky_entry: false,
            teams_txt: Some(b"teams".to_vec()),
            round_results_txt: Some(b"round".to_vec()),
            info_txt: modified_components.then(|| LiveC4SaveNamedComponent {
                name: "Info.txt".to_owned(),
                payload: b"info".to_vec(),
            }),
            script_c: modified_components.then(|| LiveC4SaveNamedComponent {
                name: "Script.c".to_owned(),
                payload: b"script".to_vec(),
            }),
            deleted_components: Vec::new(),
            component_host_mutations: Vec::new(),
            scenario_sections: vec![LiveC4SaveNamedComponent {
                name: "SectNext.c4g".to_string(),
                payload: scenario_section,
            }],
            deleted_scenario_sections: Vec::new(),
            scenario_section_mutations: Vec::new(),
        }
    }

    fn packed_runtime_dynamic_with_game(
        modified_components: bool,
        game_txt: Vec<u8>,
        include_restore_infos: bool,
    ) -> Group {
        let mut material = MutableGroup::new("Material.c4g");
        material
            .add_file("TexMap.txt", b"material".to_vec())
            .expect("material entry");
        let mut section = MutableGroup::new("SectNext.c4g");
        section
            .add_file("Objects.txt", b"section".to_vec())
            .expect("section entry");
        let mut player_group = MutableGroup::new("Host-Alice.c4p");
        player_group
            .add_file("Player.txt", b"player".to_vec())
            .expect("player entry");
        let mut save = live_save_components(
            modified_components,
            material.pack_raw().expect("raw material group"),
            section.pack_raw().expect("raw scenario section"),
        );
        save.game_txt = game_txt;
        let restore_infos = if include_restore_infos {
            snapshot(vec![ClientPlayerInfosSnapshot {
                client_id: 1,
                flags: 0,
                players: vec![player(
                    1,
                    PLAYER_INFO_TYPE_USER,
                    PLAYER_INFO_FLAG_JOINED,
                    b"Host-Alice.c4p",
                )],
            }])
        } else {
            snapshot(Vec::new())
        };
        let player_groups = include_restore_infos
            .then_some(SerializedRuntimeJoinPlayerGroup {
                filename: legacy(b"Host-Alice.c4p"),
                group: player_group,
            })
            .into_iter()
            .collect();
        let dynamic = compose_runtime_join_dynamic(
            "Network/DynScenario.c4s".to_string(),
            b"Maker".to_vec(),
            b"parameters".to_vec(),
            save,
            &restore_infos,
            player_groups,
        )
        .expect("runtime dynamic");
        Group::from_memory(PathBuf::from("DynScenario.c4s"), dynamic.packed_bytes)
            .expect("open runtime dynamic")
    }

    fn packed_runtime_dynamic(modified_components: bool) -> Group {
        packed_runtime_dynamic_with_game(modified_components, b"game".to_vec(), true)
    }

    #[test]
    fn runtime_dynamic_composes_engine_and_app_owned_children() {
        let dynamic = packed_runtime_dynamic(true);

        assert_eq!(dynamic.read_file("Parameters.txt").unwrap(), b"parameters");
        assert_eq!(dynamic.read_file("Scenario.txt").unwrap(), b"scenario");
        assert!(!dynamic.exists("TitleUS.txt"));
        assert_eq!(dynamic.read_file("Info.txt").unwrap(), b"info");
        assert_eq!(dynamic.read_file("Script.c").unwrap(), b"script");
        assert!(dynamic.exists("SavePlayerInfos.txt"));
        assert_eq!(
            dynamic
                .open_child("Material.c4g")
                .unwrap()
                .read_file("TexMap.txt")
                .unwrap(),
            b"material"
        );
        assert_eq!(
            dynamic
                .open_child("SectNext.c4g")
                .unwrap()
                .read_file("Objects.txt")
                .unwrap(),
            b"section"
        );
        assert_eq!(
            dynamic
                .open_child("Host-Alice.c4p")
                .unwrap()
                .read_file("Player.txt")
                .unwrap(),
            b"player"
        );
    }

    #[test]
    fn runtime_dynamic_omits_unmodified_component_hosts() {
        let dynamic = packed_runtime_dynamic(false);

        assert!(!dynamic.exists("TitleUS.txt"));
        assert!(!dynamic.exists("Info.txt"));
        assert!(!dynamic.exists("Script.c"));
    }

    #[test]
    fn runtime_dynamic_omits_empty_game_component() {
        let dynamic = packed_runtime_dynamic_with_game(false, Vec::new(), true);

        assert!(!dynamic.exists("Game.txt"));
    }

    #[test]
    fn runtime_dynamic_omits_empty_restore_info_component() {
        let dynamic = packed_runtime_dynamic_with_game(false, b"game".to_vec(), false);

        assert!(!dynamic.exists("SavePlayerInfos.txt"));
    }

    #[test]
    fn local_group_filename_encoding_matches_cpp_replacement_order() {
        assert_eq!(
            encode_local_group_filename(b"A%\x80\xaf\xff.c4p"),
            b"A%25%80%af%ff.c4p"
        );
    }

    #[test]
    fn projection_filters_nonjoined_removed_and_unknown_player_types() {
        let players = snapshot(vec![
            ClientPlayerInfosSnapshot {
                client_id: 1,
                flags: 0x11,
                players: vec![
                    player(1, PLAYER_INFO_TYPE_USER, PLAYER_INFO_FLAG_JOINED, b"A.c4p"),
                    player(2, PLAYER_INFO_TYPE_USER, 0, b"B.c4p"),
                    player(
                        3,
                        PLAYER_INFO_TYPE_USER,
                        PLAYER_INFO_FLAG_JOINED | PLAYER_INFO_FLAG_REMOVED,
                        b"C.c4p",
                    ),
                    player(4, PLAYER_INFO_TYPE_NONE, PLAYER_INFO_FLAG_JOINED, b"D.c4p"),
                ],
            },
            ClientPlayerInfosSnapshot {
                client_id: 2,
                flags: 0x22,
                players: vec![player(5, PLAYER_INFO_TYPE_USER, 0, b"E.c4p")],
            },
            ClientPlayerInfosSnapshot {
                client_id: 3,
                flags: 0x33,
                players: vec![player(
                    6,
                    PLAYER_INFO_TYPE_USER,
                    PLAYER_INFO_FLAG_JOINED,
                    b"F.c4p",
                )],
            },
        ]);

        let plan = set_as_runtime_join_restore_infos(
            &[client_core(1, b"One"), client_core(3, b"Three")],
            &players,
        );

        assert_eq!(plan.restore_infos.last_player_id, 99);
        assert_eq!(
            plan.restore_infos
                .clients
                .iter()
                .map(|client| (client.client_id, client.flags))
                .collect::<Vec<_>>(),
            vec![(1, 0x11), (3, 0x33)]
        );
        assert_eq!(
            plan.restore_infos
                .clients
                .iter()
                .flat_map(|client| &client.players)
                .map(|player| player.id)
                .collect::<Vec<_>>(),
            vec![1, 6]
        );
        assert_eq!(
            plan.player_groups
                .iter()
                .map(|target| target.player_info_id)
                .collect::<Vec<_>>(),
            vec![1, 6]
        );
    }

    #[test]
    fn restore_plan_rejects_a_joined_player_without_a_runtime_section() {
        // SetAsRestoreInfos keeps only IsJoined rows, while exact Game.txt
        // serialization emits one Player<ID> section per live C4Player
        // (src/C4PlayerInfo.cpp:1637-1665; src/C4Game.cpp:1987-1994).
        let plan = set_as_runtime_join_restore_infos(
            &[client_core(1, b"One")],
            &snapshot(vec![ClientPlayerInfosSnapshot {
                client_id: 1,
                flags: 0,
                players: vec![player(
                    3,
                    PLAYER_INFO_TYPE_USER,
                    PLAYER_INFO_FLAG_JOINED,
                    b"Three.c4p",
                )],
            }]),
        );

        let error = plan
            .validate_for_live_save(clonk_engine::LiveC4SavePolicy::RuntimeNetwork, [1, 2])
            .expect_err("joined player 3 has no matching runtime section");

        assert_eq!(
            error.to_string(),
            "exact save has joined SavePlayerInfos IDs without matching Game.txt Player<ID> sections: 3"
        );
    }

    #[test]
    fn nonexact_scenario_restore_plan_needs_no_runtime_player_section() {
        // C4GameSaveScenario saves script-player restore rows but does not
        // save exact runtime Player<ID> sections (src/C4GameSave.h:117-131).
        let plan = set_as_live_save_restore_infos(
            &[],
            &snapshot(vec![ClientPlayerInfosSnapshot {
                client_id: 0,
                flags: 0,
                players: vec![player(
                    3,
                    PLAYER_INFO_TYPE_SCRIPT,
                    PLAYER_INFO_FLAG_JOINED,
                    b"Script.c4p",
                )],
            }]),
            false,
            clonk_engine::LiveC4SavePolicy::Scenario {
                force_exact_landscape: false,
            }
            .player_policy(),
        );

        plan.validate_for_live_save(
            clonk_engine::LiveC4SavePolicy::Scenario {
                force_exact_landscape: false,
            },
            [],
        )
        .expect("nonexact scenarios intentionally omit runtime player sections");
    }

    #[test]
    fn user_restore_uses_resource_basename_then_discards_resource() {
        let mut joined = player(
            7,
            PLAYER_INFO_TYPE_USER,
            PLAYER_INFO_FLAG_JOINED
                | PLAYER_INFO_FLAG_HAS_RESOURCE
                | PLAYER_INFO_FLAG_NO_SCENARIO_INIT,
            b"/local/Original.c4p",
        );
        joined.team = 4;
        joined.color = 0x0012_3456;
        joined.auth_id = legacy(b"auth");
        joined.resource = Some(NetworkResourceCore {
            id: 55,
            filename: legacy(b"/network/Published.c4p"),
            ..Default::default()
        });
        let mut expected = joined.clone();
        expected.filename = legacy(b"Client-Published.c4p");
        expected.flags &= !PLAYER_INFO_FLAG_HAS_RESOURCE;
        expected.resource = None;

        let plan = set_as_runtime_join_restore_infos(
            &[client_core(4, b"Client")],
            &snapshot(vec![ClientPlayerInfosSnapshot {
                client_id: 4,
                flags: 0x45,
                players: vec![joined],
            }]),
        );

        assert_eq!(plan.restore_infos.clients[0].players[0], expected);
        assert_eq!(
            plan.player_groups,
            vec![RuntimeJoinPlayerGroupTarget {
                client_id: 4,
                player_info_id: 7,
                player_type: PLAYER_INFO_TYPE_USER,
                game_number: 107,
                filename: legacy(b"Client-Published.c4p"),
            }]
        );
    }

    #[test]
    fn user_restore_basename_accepts_both_c4_path_separators() {
        assert_eq!(legacy_basename(b"C:\\Players\\Alice.c4p"), b"Alice.c4p");
        assert_eq!(legacy_basename(b"Players/Bob.c4p"), b"Bob.c4p");
    }

    #[test]
    fn missing_resource_observes_the_already_cleared_restore_filename() {
        let plan = set_as_runtime_join_restore_infos(
            &[],
            &snapshot(vec![ClientPlayerInfosSnapshot {
                client_id: 77,
                flags: 0,
                players: vec![player(
                    8,
                    PLAYER_INFO_TYPE_USER,
                    PLAYER_INFO_FLAG_JOINED,
                    b"/players/Local.c4p",
                )],
            }]),
        );

        assert_eq!(
            plan.restore_infos.clients[0].players[0].filename.as_bytes(),
            b"Unknown-"
        );
        assert_eq!(plan.player_groups[0].filename.as_bytes(), b"Unknown-");
    }

    #[test]
    fn offline_record_user_restore_keeps_empty_filename_without_player_child() {
        let mut joined = player(
            9,
            PLAYER_INFO_TYPE_USER,
            PLAYER_INFO_FLAG_JOINED | PLAYER_INFO_FLAG_HAS_RESOURCE,
            b"/players/Original.c4p",
        );
        joined.resource = Some(NetworkResourceCore {
            id: 91,
            filename: legacy(b"/network/Published.c4p"),
            ..Default::default()
        });

        let plan = set_as_live_save_restore_infos(
            &[client_core(4, b"Client")],
            &snapshot(vec![ClientPlayerInfosSnapshot {
                client_id: 4,
                flags: 0,
                players: vec![joined],
            }]),
            false,
            clonk_engine::LiveC4SavePolicy::Record.player_policy(),
        );

        let restored = &plan.restore_infos.clients[0].players[0];
        assert!(restored.filename.as_bytes().is_empty());
        assert_eq!(restored.flags & PLAYER_INFO_FLAG_HAS_RESOURCE, 0);
        assert_eq!(restored.resource, None);
        assert!(plan.player_groups.is_empty());
    }

    #[test]
    fn script_restore_uses_id_filename_and_discards_resource() {
        let mut script = player(
            17,
            PLAYER_INFO_TYPE_SCRIPT,
            PLAYER_INFO_FLAG_JOINED | PLAYER_INFO_FLAG_HAS_RESOURCE | PLAYER_INFO_FLAG_INVISIBLE,
            b"Ignored.c4p",
        );
        script.game_number = 3;
        script.resource = Some(NetworkResourceCore {
            id: 71,
            filename: legacy(b"AlsoIgnored.c4p"),
            ..Default::default()
        });

        let plan = set_as_runtime_join_restore_infos(
            &[],
            &snapshot(vec![ClientPlayerInfosSnapshot {
                client_id: 5,
                flags: 0x81,
                players: vec![script],
            }]),
        );

        let restored = &plan.restore_infos.clients[0].players[0];
        assert_eq!(restored.filename.as_bytes(), b"ScriptPlr-17.c4p");
        assert_eq!(restored.flags & PLAYER_INFO_FLAG_HAS_RESOURCE, 0);
        assert_ne!(restored.flags & PLAYER_INFO_FLAG_INVISIBLE, 0);
        assert_eq!(restored.resource, None);
        assert_eq!(
            plan.player_groups[0],
            RuntimeJoinPlayerGroupTarget {
                client_id: 5,
                player_info_id: 17,
                player_type: PLAYER_INFO_TYPE_SCRIPT,
                game_number: 3,
                filename: legacy(b"ScriptPlr-17.c4p"),
            }
        );
    }
}
