//! Client-side planning for the resources carried by initial JoinData.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use lc_engine::{NetworkResourceCore, PLAYER_INFO_FLAG_IN_SCENARIO_FILE, PLAYER_INFO_FLAG_REMOVED};
use thiserror::Error;

use crate::{
    resolve_local_resource, JoinDataEnvelope, LocalResourceMatch, LocalResourceResolution,
    LocalResourceResolutionError, NonLoadableResourceMismatch,
};

/// Candidate paths to search, in C++ search order, for each resource ID.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClientBootstrapLocalCandidates {
    by_resource_id: BTreeMap<i32, Vec<PathBuf>>,
}

impl ClientBootstrapLocalCandidates {
    pub fn insert(&mut self, resource_id: i32, candidates: Vec<PathBuf>) -> Option<Vec<PathBuf>> {
        self.by_resource_id.insert(resource_id, candidates)
    }

    fn get(&self, resource_id: i32) -> &[PathBuf] {
        self.by_resource_id
            .get(&resource_id)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientBootstrapResourceRole {
    Scenario,
    Dynamic,
    GameResource,
    Player,
}

impl ClientBootstrapResourceRole {
    pub fn is_required(self) -> bool {
        !matches!(self, Self::Player)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientBootstrapResourceSource {
    Local(LocalResourceMatch),
    Download,
    UnavailableNonLoadable(NonLoadableResourceMismatch),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientBootstrapResourcePlan {
    pub role: ClientBootstrapResourceRole,
    pub core: NetworkResourceCore,
    pub source: ClientBootstrapResourceSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientBootstrapPlan {
    resources: Vec<ClientBootstrapResourcePlan>,
}

impl ClientBootstrapPlan {
    pub fn resources(&self) -> &[ClientBootstrapResourcePlan] {
        &self.resources
    }

    pub fn downloads(&self) -> impl Iterator<Item = &ClientBootstrapResourcePlan> {
        self.resources
            .iter()
            .filter(|resource| matches!(resource.source, ClientBootstrapResourceSource::Download))
    }

    pub fn is_ready(&self, completed_downloads: &BTreeSet<i32>) -> bool {
        self.resources
            .iter()
            .filter(|resource| resource.role.is_required())
            .all(|resource| match &resource.source {
                ClientBootstrapResourceSource::Local(_) => true,
                ClientBootstrapResourceSource::Download => {
                    completed_downloads.contains(&resource.core.id)
                }
                ClientBootstrapResourceSource::UnavailableNonLoadable(_) => false,
            })
    }
}

#[derive(Debug, Error)]
pub enum ClientBootstrapPlanError {
    #[error("local resolution failed for network resource {resource_id}: {source}")]
    LocalResolution {
        resource_id: i32,
        #[source]
        source: LocalResourceResolutionError,
    },
    #[error("required {role:?} network resource {resource_id} is non-loadable and unavailable")]
    MissingRequiredNonLoadable {
        role: ClientBootstrapResourceRole,
        resource_id: i32,
        filename: Vec<u8>,
    },
}

/// Plans initial JoinData resources without mutating a network session.
pub fn plan_client_bootstrap(
    join_data: &JoinDataEnvelope,
    local_candidates: &ClientBootstrapLocalCandidates,
    standalone_directory: impl AsRef<Path>,
) -> Result<ClientBootstrapPlan, ClientBootstrapPlanError> {
    let standalone_directory = standalone_directory.as_ref();
    let mut resources = Vec::new();

    // HandleJoinData registers GameRes, dynamic, and player resources in this
    // order (src/C4Network2.cpp:1612-1620). Scenario registration is completed
    // by InitClient (src/C4Network2.cpp:320-322).
    for core in &join_data.parameters.game_resources {
        resources.push(plan_resource(
            ClientBootstrapResourceRole::GameResource,
            core,
            local_candidates.get(core.id),
            standalone_directory,
        )?);
    }
    resources.push(plan_resource(
        ClientBootstrapResourceRole::Dynamic,
        &join_data.dynamic,
        local_candidates.get(join_data.dynamic.id),
        standalone_directory,
    )?);
    for player in join_data
        .parameters
        .player_infos
        .clients
        .iter()
        .flat_map(|client| &client.players)
        .filter(|player| {
            player.flags & (PLAYER_INFO_FLAG_REMOVED | PLAYER_INFO_FLAG_IN_SCENARIO_FILE) == 0
        })
    {
        if let Some(core) = &player.resource {
            resources.push(plan_resource(
                ClientBootstrapResourceRole::Player,
                core,
                local_candidates.get(core.id),
                standalone_directory,
            )?);
        }
    }
    let scenario = &join_data.parameters.scenario;
    resources.push(plan_resource(
        ClientBootstrapResourceRole::Scenario,
        scenario,
        local_candidates.get(scenario.id),
        standalone_directory,
    )?);

    Ok(ClientBootstrapPlan { resources })
}

fn plan_resource(
    role: ClientBootstrapResourceRole,
    core: &NetworkResourceCore,
    local_candidates: &[PathBuf],
    standalone_directory: &Path,
) -> Result<ClientBootstrapResourcePlan, ClientBootstrapPlanError> {
    let source = match resolve_local_resource(core, local_candidates, standalone_directory)
        .map_err(|source| ClientBootstrapPlanError::LocalResolution {
            resource_id: core.id,
            source,
        })? {
        LocalResourceResolution::Local(local) => ClientBootstrapResourceSource::Local(local),
        LocalResourceResolution::LoadRemote => ClientBootstrapResourceSource::Download,
        LocalResourceResolution::FatalNonLoadable(mismatch) if role.is_required() => {
            return Err(ClientBootstrapPlanError::MissingRequiredNonLoadable {
                role,
                resource_id: mismatch.resource_id,
                filename: mismatch.filename,
            });
        }
        LocalResourceResolution::FatalNonLoadable(mismatch) => {
            ClientBootstrapResourceSource::UnavailableNonLoadable(mismatch)
        }
    };
    Ok(ClientBootstrapResourcePlan {
        role,
        core: core.clone(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    use lc_engine::{ControlPlayerInfoEntry, LegacyCString, PLAYER_INFO_FLAG_HAS_RESOURCE};

    use super::*;
    use crate::{
        ClientPlayerInfosSnapshot, JoinClientRegistrySnapshot, JoinGameParametersEnvelope,
        JoinTeamListSnapshot, NetworkStatus, PlayerInfoListSnapshot,
    };

    static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn resolves_contents_identical_local_resource_before_downloading() {
        // AddByCore first attempts SetByCore and downloads only if no local
        // contents-identical resource exists (src/C4Network2Res.cpp:1473-1516,
        // 441-493).
        let directory = TestDirectory::new();
        let system_path = directory.root.join("System.c4g");
        let system_bytes = b"contents-identical system";
        fs::write(&system_path, system_bytes).unwrap();
        let scenario = resource_core(1, 7, true, b"Tutorial.c4s", b"scenario");
        let dynamic = resource_core(2, 8, true, b"DynTutorial.c4s", b"dynamic");
        let system = resource_core(5, 9, false, b"System.c4g", system_bytes);
        let join_data = join_data(scenario, dynamic, vec![system], None);
        let mut candidates = ClientBootstrapLocalCandidates::default();
        candidates.insert(9, vec![system_path.clone()]);

        let plan = plan_client_bootstrap(&join_data, &candidates, &directory.standalone).unwrap();

        let system_plan = plan
            .resources()
            .iter()
            .find(|resource| resource.core.id == 9)
            .unwrap();
        let ClientBootstrapResourceSource::Local(local) = &system_plan.source else {
            panic!("contents-identical System should be local");
        };
        assert_eq!(local.source_path(), system_path);
        assert!(!plan.downloads().any(|resource| resource.core.id == 9));
    }

    #[test]
    fn rejects_missing_required_non_loadable_game_resource() {
        // A client fails game-resource initialization when no identical local
        // resource exists and the host marked it non-loadable; System gets the
        // dedicated fatal path (src/C4GameParameters.cpp:125-160;
        // src/C4Network2Res.cpp:1473-1516).
        let directory = TestDirectory::new();
        let scenario = resource_core(1, 7, true, b"Tutorial.c4s", b"scenario");
        let dynamic = resource_core(2, 8, true, b"DynTutorial.c4s", b"dynamic");
        let system = resource_core(5, 9, false, b"System.c4g", b"system");
        let join_data = join_data(scenario, dynamic, vec![system], None);

        let error = plan_client_bootstrap(
            &join_data,
            &ClientBootstrapLocalCandidates::default(),
            &directory.standalone,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            ClientBootstrapPlanError::MissingRequiredNonLoadable {
                role: ClientBootstrapResourceRole::GameResource,
                resource_id: 9,
                filename,
            } if filename == b"System.c4g"
        ));
    }

    #[test]
    fn readiness_waits_for_all_required_downloads_but_not_player_resources() {
        // Startup retrieves scenario and dynamic, then waits for every GameRes
        // before initialization (src/C4Network2.cpp:619-633;
        // src/C4Game.cpp:2526-2555). Player resource failure instead clears
        // that player's resource flag (src/C4PlayerInfo.cpp:275-292).
        let directory = TestDirectory::new();
        let scenario_path = directory.root.join("Tutorial.c4s");
        let scenario_bytes = b"local scenario";
        fs::write(&scenario_path, scenario_bytes).unwrap();
        let scenario = resource_core(1, 7, true, b"Tutorial.c4s", scenario_bytes);
        let dynamic = resource_core(2, 8, true, b"DynTutorial.c4s", b"dynamic");
        let definitions = resource_core(4, 9, true, b"Objects.c4d", b"definitions");
        let player = resource_core(3, 10, true, b"Player.c4p", b"player");
        let join_data = join_data(scenario, dynamic, vec![definitions], Some(player));
        let mut candidates = ClientBootstrapLocalCandidates::default();
        candidates.insert(7, vec![scenario_path]);

        let plan = plan_client_bootstrap(&join_data, &candidates, &directory.standalone).unwrap();

        assert_eq!(
            plan.downloads()
                .map(|resource| resource.core.id)
                .collect::<Vec<_>>(),
            vec![9, 8, 10]
        );
        assert!(!plan.is_ready(&BTreeSet::new()));
        assert!(!plan.is_ready(&BTreeSet::from([8])));
        assert!(plan.is_ready(&BTreeSet::from([8, 9])));
    }

    fn join_data(
        scenario: NetworkResourceCore,
        dynamic: NetworkResourceCore,
        game_resources: Vec<NetworkResourceCore>,
        player_resource: Option<NetworkResourceCore>,
    ) -> JoinDataEnvelope {
        let players = player_resource.map_or_else(Vec::new, |resource| {
            vec![ClientPlayerInfosSnapshot {
                client_id: 2,
                flags: 0,
                players: vec![ControlPlayerInfoEntry {
                    flags: PLAYER_INFO_FLAG_HAS_RESOURCE,
                    resource: Some(resource),
                    ..Default::default()
                }],
            }]
        });
        let player_infos = PlayerInfoListSnapshot {
            last_player_id: 0,
            clients: players,
        };
        JoinDataEnvelope {
            client_id: 2,
            start_control_tick: 0,
            status: NetworkStatus {
                state: 0,
                control_mode: 0,
                target_tick: -1,
            },
            dynamic,
            parameters: JoinGameParametersEnvelope {
                random_seed: 0,
                startup_player_count: 0,
                max_players: 8,
                use_fair_crew: false,
                fair_crew_forced: false,
                fair_crew_strength: 0,
                allow_debug: true,
                is_network_game: true,
                control_rate: 2,
                auto_frame_skip: true,
                rules: Vec::new(),
                goals: Vec::new(),
                league: LegacyCString::default(),
                league_address: LegacyCString::default(),
                title: LegacyCString::default(),
                scenario,
                game_resources,
                player_infos,
                restore_player_infos: PlayerInfoListSnapshot {
                    last_player_id: 0,
                    clients: Vec::new(),
                },
                teams: JoinTeamListSnapshot {
                    active: 1,
                    custom: 0,
                    allow_hostility_change: 1,
                    allow_team_switch: 0,
                    auto_generate_teams: 1,
                    last_team_id: 0,
                    team_distribution: 0,
                    team_colors: 0,
                    max_script_players: 0,
                    script_player_names: LegacyCString::default(),
                    random_team_count: 0,
                    teams: Vec::new(),
                },
                clients: JoinClientRegistrySnapshot {
                    clients: Vec::new(),
                    local_client_id: None,
                },
            },
        }
    }

    fn resource_core(
        resource_type: u8,
        id: i32,
        loadable: bool,
        filename: &[u8],
        contents: &[u8],
    ) -> NetworkResourceCore {
        let crc = crc32(0, contents);
        NetworkResourceCore {
            resource_type,
            id,
            derived_id: -1,
            loadable,
            file_size: if loadable {
                contents.len() as u32
            } else {
                u32::MAX
            },
            file_crc: if loadable { crc } else { u32::MAX },
            chunk_size: 100 * 1024,
            contents_crc: crc,
            file_sha: None,
            filename: LegacyCString::from_bytes(filename.to_vec()).unwrap(),
            author: LegacyCString::default(),
        }
    }

    fn crc32(initial: u32, data: &[u8]) -> u32 {
        let mut crc = initial ^ u32::MAX;
        for byte in data {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                let mask = 0_u32.wrapping_sub(crc & 1);
                crc = (crc >> 1) ^ (0xedb8_8320 & mask);
            }
        }
        crc ^ u32::MAX
    }

    struct TestDirectory {
        root: PathBuf,
        standalone: PathBuf,
    }

    impl TestDirectory {
        fn new() -> Self {
            let unique = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "legacyclonk-client-bootstrap-{}-{unique}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).unwrap();
            let standalone = root.join("Network");
            Self { root, standalone }
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}
