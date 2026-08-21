//! Pure client-side preparation for the scenario resources named by JoinData.

use std::path::PathBuf;

use clonk_engine::NetworkResourceCore;
use clonk_network::JoinDataEnvelope;
use thiserror::Error;

/// The C++ startup stage which is waiting for a complete network resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientStartResourceRole {
    Scenario,
    Dynamic,
    GameResource { index: usize },
}

/// The first resource that prevents the client from entering game loading.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("client start is waiting for {role:?} resource {}", core.id)]
pub struct PendingClientStartResource {
    pub role: ClientStartResourceRole,
    pub core: NetworkResourceCore,
}

/// One JoinData resource paired with the resource list's authoritative path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedClientStartResource {
    pub core: NetworkResourceCore,
    pub path: PathBuf,
}

/// The two resources C++ merges before it retrieves ordinary GameRes files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientScenarioResources {
    pub scenario: ResolvedClientStartResource,
    pub dynamic: ResolvedClientStartResource,
}

#[derive(Debug, Error)]
pub enum ClientNetworkScenarioError {
    #[error("failed to open completed scenario resource at {}: {source}", path.display())]
    OpenScenario {
        path: PathBuf,
        #[source]
        source: clonk_resources::GroupError,
    },
    #[error("failed to open completed dynamic resource at {}: {source}", path.display())]
    OpenDynamic {
        path: PathBuf,
        #[source]
        source: clonk_resources::GroupError,
    },
    #[error(transparent)]
    Combine(#[from] clonk_resources::NetworkScenarioError),
}

/// Resolves scenario first and synchronized dynamic second, as RetrieveScenario does.
pub fn resolve_client_scenario_resources(
    join_data: &JoinDataEnvelope,
    mut complete_path: impl FnMut(&NetworkResourceCore) -> Option<PathBuf>,
) -> Result<ClientScenarioResources, PendingClientStartResource> {
    let scenario = resolve_resource(
        ClientStartResourceRole::Scenario,
        &join_data.parameters.scenario,
        &mut complete_path,
    )?;
    let dynamic = resolve_resource(
        ClientStartResourceRole::Dynamic,
        &join_data.dynamic,
        &mut complete_path,
    )?;
    Ok(ClientScenarioResources { scenario, dynamic })
}

/// Resolves ordinary GameRes files in their synchronized Parameters order.
pub fn resolve_client_game_resources(
    join_data: &JoinDataEnvelope,
    mut complete_path: impl FnMut(&NetworkResourceCore) -> Option<PathBuf>,
) -> Result<Vec<ResolvedClientStartResource>, PendingClientStartResource> {
    join_data
        .parameters
        .game_resources
        .iter()
        .enumerate()
        .map(|(index, core)| {
            resolve_resource(
                ClientStartResourceRole::GameResource { index },
                core,
                &mut complete_path,
            )
        })
        .collect()
}

/// Opens the resource list's authoritative files and performs RetrieveScenario's merge.
pub fn compose_client_network_scenario(
    resources: &ClientScenarioResources,
    output_filename: &str,
    maker: &str,
) -> Result<Vec<u8>, ClientNetworkScenarioError> {
    compose_client_network_scenario_with_maker_bytes(resources, output_filename, maker.as_bytes())
}

pub fn compose_client_network_scenario_with_maker_bytes(
    resources: &ClientScenarioResources,
    output_filename: &str,
    maker: &[u8],
) -> Result<Vec<u8>, ClientNetworkScenarioError> {
    let scenario = clonk_resources::Group::open(&resources.scenario.path).map_err(|source| {
        ClientNetworkScenarioError::OpenScenario {
            path: resources.scenario.path.clone(),
            source,
        }
    })?;
    let dynamic = clonk_resources::Group::open(&resources.dynamic.path).map_err(|source| {
        ClientNetworkScenarioError::OpenDynamic {
            path: resources.dynamic.path.clone(),
            source,
        }
    })?;
    clonk_resources::combine_network_scenario_with_maker_bytes(
        &scenario,
        &dynamic,
        output_filename,
        maker,
    )
    .map_err(Into::into)
}

fn resolve_resource<F>(
    role: ClientStartResourceRole,
    core: &NetworkResourceCore,
    complete_path: &mut F,
) -> Result<ResolvedClientStartResource, PendingClientStartResource>
where
    F: FnMut(&NetworkResourceCore) -> Option<PathBuf>,
{
    complete_path(core)
        .map(|path| ResolvedClientStartResource {
            core: core.clone(),
            path,
        })
        .ok_or_else(|| PendingClientStartResource {
            role,
            core: core.clone(),
        })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use clonk_engine::{LegacyCString, NetworkResourceCore};
    use clonk_network::{
        JoinClientRegistrySnapshot, JoinDataEnvelope, JoinGameParametersEnvelope,
        JoinTeamListSnapshot, NetworkStatus, PlayerInfoListSnapshot,
    };

    use super::*;

    #[test]
    fn cpp_client_resolves_scenario_dynamic_then_game_resources() {
        // RetrieveScenario blocks on Parameters.Scenario before it asks for
        // ResDynamic; RetrieveFiles follows only after the combined scenario
        // has been opened (pristine 9ffa0a5d src/C4Network2.cpp:619-671;
        // src/C4Game.cpp:2532-2556).
        let join_data = join_data(resource(7), resource(8), vec![resource(9), resource(10)]);

        assert_eq!(
            resolve_client_scenario_resources(&join_data, |_| None),
            Err(PendingClientStartResource {
                role: ClientStartResourceRole::Scenario,
                core: resource(7),
            })
        );
        assert_eq!(
            resolve_client_scenario_resources(&join_data, |core| {
                (core.id == 7).then(|| path(core.id))
            }),
            Err(PendingClientStartResource {
                role: ClientStartResourceRole::Dynamic,
                core: resource(8),
            })
        );
        let ready = resolve_client_scenario_resources(&join_data, |core| Some(path(core.id)))
            .expect("scenario and dynamic are complete");
        assert_eq!((ready.scenario.core.id, ready.scenario.path), (7, path(7)));
        assert_eq!((ready.dynamic.core.id, ready.dynamic.path), (8, path(8)));

        assert_eq!(
            resolve_client_game_resources(&join_data, |_| None),
            Err(PendingClientStartResource {
                role: ClientStartResourceRole::GameResource { index: 0 },
                core: resource(9),
            })
        );
        assert_eq!(
            resolve_client_game_resources(&join_data, |core| {
                (core.id == 9).then(|| path(core.id))
            }),
            Err(PendingClientStartResource {
                role: ClientStartResourceRole::GameResource { index: 1 },
                core: resource(10),
            })
        );

        let ready = resolve_client_game_resources(&join_data, |core| Some(path(core.id)))
            .expect("ordinary game resources are complete");
        assert_eq!(
            ready
                .into_iter()
                .map(|resource| (resource.core.id, resource.path))
                .collect::<Vec<_>>(),
            vec![(9, path(9)), (10, path(10))]
        );
    }

    #[test]
    fn cpp_client_combines_completed_scenario_and_dynamic_paths() {
        // RetrieveScenario copies and unpacks both resources, merges dynamic
        // top-level entries over the scenario, then repacks Combined<ID>.c4s
        // (pristine 9ffa0a5d src/C4Network2.cpp:619-671).
        let directory = tempfile::tempdir().expect("temporary resource directory");
        let scenario_path = directory.path().join("Scenario.c4s");
        let dynamic_path = directory.path().join("Dynamic.c4s");
        let mut scenario = clonk_resources::MutableGroup::new("Scenario.c4s");
        scenario
            .add_file("Base.txt", b"base".to_vec())
            .expect("add base file");
        scenario
            .add_file("Replace.txt", b"old".to_vec())
            .expect("add old file");
        fs::write(&scenario_path, scenario.pack().expect("pack scenario")).expect("write scenario");
        let mut dynamic = clonk_resources::MutableGroup::new("Dynamic.c4s");
        dynamic
            .add_file("Replace.txt", b"new".to_vec())
            .expect("add replacement file");
        dynamic
            .add_file("Dynamic.txt", b"dynamic".to_vec())
            .expect("add dynamic file");
        fs::write(&dynamic_path, dynamic.pack().expect("pack dynamic")).expect("write dynamic");
        let resources = ClientScenarioResources {
            scenario: ResolvedClientStartResource {
                core: resource(7),
                path: scenario_path,
            },
            dynamic: ResolvedClientStartResource {
                core: resource(8),
                path: dynamic_path,
            },
        };

        let packed = compose_client_network_scenario(&resources, "Combined2.c4s", "Alice")
            .expect("compose completed resources");
        let combined = clonk_resources::Group::from_memory("Combined2.c4s".into(), packed)
            .expect("open combined group");

        assert_eq!(combined.read_file("Base.txt").unwrap(), b"base");
        assert_eq!(combined.read_file("Replace.txt").unwrap(), b"new");
        assert_eq!(combined.read_file("Dynamic.txt").unwrap(), b"dynamic");
        assert_eq!(combined.maker(), Some("Alice"));
    }

    fn path(id: i32) -> std::path::PathBuf {
        format!("/complete/Resource{id}.c4g").into()
    }

    fn resource(id: i32) -> NetworkResourceCore {
        NetworkResourceCore {
            id,
            loadable: true,
            filename: LegacyCString::from_bytes(format!("Resource{id}.c4g").into_bytes())
                .expect("fixture filename is NUL-free"),
            ..Default::default()
        }
    }

    fn join_data(
        scenario: NetworkResourceCore,
        dynamic: NetworkResourceCore,
        game_resources: Vec<NetworkResourceCore>,
    ) -> JoinDataEnvelope {
        let empty_player_infos = || PlayerInfoListSnapshot::default();
        JoinDataEnvelope {
            client_id: 2,
            start_control_tick: 0,
            status: NetworkStatus::new(0, 0, -1),
            dynamic,
            parameters: JoinGameParametersEnvelope {
                max_players: 8,
                allow_debug: true,
                is_network_game: true,
                control_rate: 2,
                auto_frame_skip: true,
                league_address: LegacyCString::default(),
                scenario,
                game_resources,
                player_infos: empty_player_infos(),
                restore_player_infos: empty_player_infos(),
                teams: JoinTeamListSnapshot::default(),
                clients: JoinClientRegistrySnapshot::default(),
                ..Default::default()
            },
        }
    }
}
