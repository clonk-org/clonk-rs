//! Client-side planning for the resources carried by initial JoinData.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use clonk_engine::{
    LegacyCString, NetworkResourceCore, PLAYER_INFO_FLAG_HAS_RESOURCE,
    PLAYER_INFO_FLAG_IN_SCENARIO_FILE, PLAYER_INFO_FLAG_REMOVED,
};
use thiserror::Error;

use crate::local_resource_resolution::{
    resolve_local_resource_candidates_with_group_maker, LocalResourceCandidate,
};
use crate::{
    JoinDataEnvelope, LocalResourceMatch, LocalResourceResolution, LocalResourceResolutionError,
    NonLoadableResourceMismatch,
};

/// Candidate paths to search, in C++ search order, for each resource ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientBootstrapLocalCandidates {
    by_resource_id: BTreeMap<i32, Vec<PathBuf>>,
    search_roots: Vec<PathBuf>,
    max_search_recursion: usize,
}

impl Default for ClientBootstrapLocalCandidates {
    fn default() -> Self {
        Self {
            by_resource_id: BTreeMap::new(),
            search_roots: Vec::new(),
            // Config.Network.MaxResSearchRecursion defaults to one
            // (src/C4Config.cpp:238-240).
            max_search_recursion: 1,
        }
    }
}

impl ClientBootstrapLocalCandidates {
    pub fn insert(&mut self, resource_id: i32, candidates: Vec<PathBuf>) -> Option<Vec<PathBuf>> {
        self.by_resource_id.insert(resource_id, candidates)
    }

    /// Tries a canonical resource location first without discarding fallbacks.
    pub fn prioritize(&mut self, resource_id: i32, candidate: impl Into<PathBuf>) {
        let candidate = candidate.into();
        let candidates = self.by_resource_id.entry(resource_id).or_default();
        candidates.retain(|existing| existing != &candidate);
        candidates.insert(0, candidate);
    }

    pub fn set_max_search_recursion(&mut self, max_search_recursion: usize) {
        self.max_search_recursion = max_search_recursion;
    }

    pub fn max_search_recursion(&self) -> usize {
        self.max_search_recursion
    }

    pub fn extend_from_roots(
        &mut self,
        _join_data: &JoinDataEnvelope,
        roots: impl IntoIterator<Item = impl AsRef<Path>>,
    ) {
        self.extend_search_roots(roots);
    }

    pub fn extend_search_roots(&mut self, roots: impl IntoIterator<Item = impl AsRef<Path>>) {
        for root in roots {
            let root = root.as_ref().to_path_buf();
            if !self.search_roots.contains(&root) {
                self.search_roots.push(root);
            }
        }
    }

    fn for_core(
        &self,
        core: &NetworkResourceCore,
        work_path: &Path,
    ) -> Vec<LocalResourceCandidate> {
        let mut candidates = self
            .by_resource_id
            .get(&core.id)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(LocalResourceCandidate::exact)
            .collect();
        let filename_bytes =
            &core.filename.as_bytes()[c4_filename_start(core.filename.as_bytes())..];
        let filename = c4_filename_path(core.filename.as_bytes());
        for root in &self.search_roots {
            append_cpp_search_candidates(
                &mut candidates,
                root,
                &filename,
                filename_bytes,
                work_path,
                self.max_search_recursion,
            );
        }
        candidates
    }
}

fn append_cpp_search_candidates(
    candidates: &mut Vec<LocalResourceCandidate>,
    search_root: &Path,
    c4_filename: &Path,
    c4_filename_bytes: &[u8],
    work_path: &Path,
    max_search_recursion: usize,
) {
    append_unique(
        candidates,
        LocalResourceCandidate::with_lookup_name(
            search_root.join(c4_filename),
            c4_filename_bytes.to_vec(),
        ),
    );
    let basename_start = c4_filename_bytes
        .iter()
        .rposition(|byte| is_directory_separator(*byte))
        .map_or(0, |separator| separator + 1);
    if basename_start != 0 && basename_start != c4_filename_bytes.len() {
        let basename_bytes = &c4_filename_bytes[basename_start..];
        let basename = bytes_path(basename_bytes);
        append_unique(
            candidates,
            LocalResourceCandidate::with_lookup_name(
                search_root.join(&basename),
                basename_bytes.to_vec(),
            ),
        );
    }
    append_cpp_recursive_search(
        candidates,
        search_root,
        c4_filename,
        work_path,
        max_search_recursion,
        0,
    );
}

fn append_cpp_recursive_search(
    candidates: &mut Vec<LocalResourceCandidate>,
    search_root: &Path,
    c4_filename: &Path,
    work_path: &Path,
    max_search_recursion: usize,
    recursion: usize,
) {
    if recursion >= max_search_recursion {
        return;
    }
    let Ok(entries) = std::fs::read_dir(search_root) else {
        return;
    };
    for path in entries.filter_map(Result::ok).map(|entry| entry.path()) {
        if !path.is_dir()
            || !c4_directory_has_no_extension(&path)
            || paths_identical(&path, work_path)
        {
            continue;
        }
        append_unique(
            candidates,
            LocalResourceCandidate::exact(path.join(c4_filename)),
        );
        append_cpp_recursive_search(
            candidates,
            &path,
            c4_filename,
            work_path,
            max_search_recursion,
            recursion + 1,
        );
    }
}

fn append_unique(candidates: &mut Vec<LocalResourceCandidate>, candidate: LocalResourceCandidate) {
    if !candidates
        .iter()
        .any(|existing| existing.path() == candidate.path())
    {
        candidates.push(candidate);
    }
}

fn paths_identical(left: &Path, right: &Path) -> bool {
    let Ok(left) = left.canonicalize() else {
        return false;
    };
    let Ok(right) = right.canonicalize() else {
        return false;
    };
    #[cfg(windows)]
    {
        left.to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy())
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

fn c4_directory_has_no_extension(path: &Path) -> bool {
    path.file_name()
        .map(|name| clonk_resources::path_to_legacy_bytes(Path::new(name)))
        .is_some_and(|name| {
            name.iter()
                .rposition(|byte| *byte == b'.')
                .is_none_or(|dot| dot + 1 == name.len())
        })
}

fn c4_filename_start(path: &[u8]) -> usize {
    let mut filename_start = 0;
    for (index, byte) in path.iter().copied().enumerate() {
        if !is_directory_separator(byte) {
            continue;
        }
        if index >= 4 && path[index - 4..index - 1].eq_ignore_ascii_case(b".c4") {
            return filename_start;
        }
        filename_start = index + 1;
    }
    filename_start
}

fn is_directory_separator(byte: u8) -> bool {
    byte == b'/' || cfg!(windows) && byte == b'\\'
}

fn c4_filename_path(filename: &[u8]) -> PathBuf {
    bytes_path(&filename[c4_filename_start(filename)..])
}

fn bytes_path(bytes: &[u8]) -> PathBuf {
    // C4Group::Open converts backslashes to the native directory separator
    // after GetC4Filename/GetFilename have applied their platform-specific
    // lexical rules. Keep lookup names raw, but normalize the filesystem
    // path passed to the group opener on every platform.
    let bytes = bytes
        .iter()
        .map(|byte| {
            if *byte == b'\\' {
                std::path::MAIN_SEPARATOR as u8
            } else {
                *byte
            }
        })
        .collect::<Vec<_>>();
    clonk_resources::path_from_legacy_bytes(&bytes)
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
    /// The installed System used by the Rust runtime for C++ cross-build
    /// compatibility. Unlike `Local`, this does not claim ContentsCRC identity.
    TrustedLocalSystem(PathBuf),
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
                ClientBootstrapResourceSource::Local(_)
                | ClientBootstrapResourceSource::TrustedLocalSystem(_) => true,
                ClientBootstrapResourceSource::Download => {
                    completed_downloads.contains(&resource.core.id)
                }
                ClientBootstrapResourceSource::UnavailableNonLoadable(_) => false,
            })
    }
}

pub(crate) struct ClientBootstrapPlanner {
    local_candidates: ClientBootstrapLocalCandidates,
    standalone_directory: PathBuf,
    group_maker: Vec<u8>,
    resources: Vec<ClientBootstrapResourcePlan>,
    registered_resource_ids: BTreeSet<i32>,
    initialized_game_resources: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct ClientBootstrapResolver {
    local_candidates: ClientBootstrapLocalCandidates,
    standalone_directory: PathBuf,
    group_maker: LegacyCString,
    trusted_local_system_path: Option<PathBuf>,
}

impl ClientBootstrapResolver {
    pub(crate) fn new(
        local_candidates: &ClientBootstrapLocalCandidates,
        standalone_directory: impl Into<PathBuf>,
    ) -> Self {
        Self::new_with_group_maker(
            local_candidates,
            standalone_directory,
            LegacyCString::default(),
        )
    }

    pub(crate) fn new_with_group_maker(
        local_candidates: &ClientBootstrapLocalCandidates,
        standalone_directory: impl Into<PathBuf>,
        group_maker: LegacyCString,
    ) -> Self {
        Self {
            local_candidates: local_candidates.clone(),
            standalone_directory: standalone_directory.into(),
            group_maker,
            trusted_local_system_path: None,
        }
    }

    pub(crate) fn with_trusted_local_system_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.trusted_local_system_path = Some(path.into());
        self
    }

    pub(crate) fn resolve(
        &self,
        role: ClientBootstrapResourceRole,
        core: &NetworkResourceCore,
    ) -> Result<ClientBootstrapResourcePlan, ClientBootstrapPlanError> {
        let result = plan_resource(
            role,
            core,
            &self.local_candidates,
            &self.standalone_directory,
            self.group_maker.as_bytes(),
        );
        let Err(error) = result else {
            return result;
        };
        let trusted_system = matches!(
            error,
            ClientBootstrapPlanError::MissingRequiredNonLoadable {
                role: ClientBootstrapResourceRole::GameResource,
                ..
            }
        ) && core.resource_type == crate::HostResourceType::System as u8;
        let trusted_path = trusted_system
            .then_some(self.trusted_local_system_path.as_ref())
            .flatten()
            .filter(|path| clonk_resources::Group::open(path).is_ok());
        trusted_path.map_or(Err(error), |path| {
            Ok(ClientBootstrapResourcePlan {
                role,
                core: core.clone(),
                source: ClientBootstrapResourceSource::TrustedLocalSystem(path.clone()),
            })
        })
    }
}

impl ClientBootstrapPlanner {
    #[cfg(test)]
    pub(crate) fn new(
        local_candidates: &ClientBootstrapLocalCandidates,
        standalone_directory: impl Into<PathBuf>,
    ) -> Self {
        Self::new_with_group_maker(local_candidates, standalone_directory, b"")
    }

    pub(crate) fn new_with_group_maker(
        local_candidates: &ClientBootstrapLocalCandidates,
        standalone_directory: impl Into<PathBuf>,
        group_maker: &[u8],
    ) -> Self {
        Self {
            local_candidates: local_candidates.clone(),
            standalone_directory: standalone_directory.into(),
            group_maker: group_maker.to_vec(),
            resources: Vec::new(),
            registered_resource_ids: BTreeSet::new(),
            initialized_game_resources: 0,
        }
    }

    fn plan_registration(
        &mut self,
        role: ClientBootstrapResourceRole,
        core: &NetworkResourceCore,
    ) -> Result<Option<ClientBootstrapResourcePlan>, ClientBootstrapPlanError> {
        // AddByCore returns an already registered ID without comparing the
        // incoming core again (src/C4Network2Res.cpp:1473-1477).
        if self.registered_resource_ids.contains(&core.id) {
            return Ok(None);
        }
        let resource = plan_resource(
            role,
            core,
            &self.local_candidates,
            &self.standalone_directory,
            &self.group_maker,
        )?;
        if !matches!(
            resource.source,
            ClientBootstrapResourceSource::UnavailableNonLoadable(_)
        ) {
            self.registered_resource_ids.insert(core.id);
        }
        Ok(Some(resource))
    }

    /// Mirrors the resource work performed inside HandleJoinData before
    /// Clients.SendAddresses (src/C4Network2.cpp:1612-1622).
    pub(crate) fn plan_before_addresses(
        &mut self,
        join_data: &mut JoinDataEnvelope,
    ) -> Result<(), ClientBootstrapPlanError> {
        for core in &join_data.parameters.game_resources {
            match self.plan_registration(ClientBootstrapResourceRole::GameResource, core) {
                Ok(resource) => {
                    if let Some(resource) = resource {
                        self.resources.push(resource);
                    }
                    self.initialized_game_resources += 1;
                }
                // HandleJoinData deliberately ignores this first aggregate
                // GameRes.InitNetwork result. The outer InitClient retries the
                // failed entry after addresses have been sent.
                Err(_) => break,
            }
        }

        if let Some(dynamic) =
            self.plan_registration(ClientBootstrapResourceRole::Dynamic, &join_data.dynamic)?
        {
            self.resources.push(dynamic);
        }

        for player in join_data
            .parameters
            .player_infos
            .clients
            .iter_mut()
            .flat_map(|client| &mut client.players)
        {
            let flags = player.flags;
            if flags & PLAYER_INFO_FLAG_REMOVED != 0 || flags & PLAYER_INFO_FLAG_HAS_RESOURCE == 0 {
                continue;
            }
            if flags & PLAYER_INFO_FLAG_IN_SCENARIO_FILE != 0 {
                clear_player_resource(player);
                continue;
            }
            let Some(core) = player.resource.as_ref() else {
                clear_player_resource(player);
                continue;
            };
            match self.plan_registration(ClientBootstrapResourceRole::Player, core) {
                Ok(Some(ClientBootstrapResourcePlan {
                    source: ClientBootstrapResourceSource::UnavailableNonLoadable(_),
                    ..
                }))
                | Err(_) => clear_player_resource(player),
                Ok(Some(resource)) => self.resources.push(resource),
                Ok(None) => {}
            }
        }
        Ok(())
    }

    /// Mirrors outer InitClient's final Parameters.InitNetwork call after
    /// HandleJoinData has announced addresses (src/C4Network2.cpp:329-331;
    /// src/C4GameParameters.cpp:539-547).
    pub(crate) fn plan_after_addresses(
        mut self,
        join_data: &JoinDataEnvelope,
    ) -> Result<ClientBootstrapPlan, ClientBootstrapPlanError> {
        if let Some(scenario) = self.plan_registration(
            ClientBootstrapResourceRole::Scenario,
            &join_data.parameters.scenario,
        )? {
            self.resources.push(scenario);
        }
        for core in join_data
            .parameters
            .game_resources
            .iter()
            .skip(self.initialized_game_resources)
        {
            if let Some(resource) =
                self.plan_registration(ClientBootstrapResourceRole::GameResource, core)?
            {
                self.resources.push(resource);
            }
        }
        Ok(ClientBootstrapPlan {
            resources: self.resources,
        })
    }
}

pub(crate) fn clear_player_resource(player: &mut clonk_engine::ControlPlayerInfoEntry) {
    player.flags &= !PLAYER_INFO_FLAG_HAS_RESOURCE;
    // C++ retains a private stale ResCore, but excludes it from every later
    // serialization. Rust represents that wire-visible state with None
    // (src/C4PlayerInfo.cpp:257-292).
    player.resource = None;
}

#[derive(Debug, Error)]
pub enum ClientBootstrapPlanError {
    #[error("local resolution failed for network resource {resource_id}: {source}")]
    LocalResolution {
        resource_id: i32,
        #[source]
        source: LocalResourceResolutionError,
    },
    #[error(
        "required {role:?} network resource {resource_id} `{}` is non-loadable and unavailable",
        String::from_utf8_lossy(.filename)
    )]
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
    plan_client_bootstrap_with_group_maker(join_data, local_candidates, standalone_directory, b"")
}

/// Plans initial resources with the process-wide C4Group maker used when a
/// local player candidate must be packed or optimized.
pub fn plan_client_bootstrap_with_group_maker(
    join_data: &JoinDataEnvelope,
    local_candidates: &ClientBootstrapLocalCandidates,
    standalone_directory: impl AsRef<Path>,
    group_maker: &[u8],
) -> Result<ClientBootstrapPlan, ClientBootstrapPlanError> {
    let mut join_data = join_data.clone();
    let mut planner = ClientBootstrapPlanner::new_with_group_maker(
        local_candidates,
        standalone_directory.as_ref().to_path_buf(),
        group_maker,
    );
    planner.plan_before_addresses(&mut join_data)?;
    planner.plan_after_addresses(&join_data)
}

fn plan_resource(
    role: ClientBootstrapResourceRole,
    core: &NetworkResourceCore,
    local_candidates: &ClientBootstrapLocalCandidates,
    standalone_directory: &Path,
    group_maker: &[u8],
) -> Result<ClientBootstrapResourcePlan, ClientBootstrapPlanError> {
    let candidates = local_candidates.for_core(core, standalone_directory);
    let source = match resolve_local_resource_candidates_with_group_maker(
        core,
        &candidates,
        standalone_directory,
        group_maker,
    )
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

    use clonk_engine::{ControlPlayerInfoEntry, LegacyCString, PLAYER_INFO_FLAG_HAS_RESOURCE};

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
    fn searches_extensionless_directories_below_each_local_root() {
        // SetByCore searches the executable root's extensionless directories
        // for the C4-relative resource name (src/C4Network2Res.cpp:441-493;
        // src/StdFile.cpp:67-81).
        let directory = TestDirectory::new();
        let expansion = directory.root.join("Expansion");
        fs::create_dir(&expansion).unwrap();
        let definitions_path = expansion.join("Objects.c4d");
        let definitions_bytes = b"contents-identical definitions";
        fs::write(&definitions_path, definitions_bytes).unwrap();
        let scenario = resource_core(1, 7, true, b"Tutorial.c4s", b"scenario");
        let dynamic = resource_core(2, 8, true, b"DynTutorial.c4s", b"dynamic");
        let definitions = resource_core(4, 9, false, b"Objects.c4d", definitions_bytes);
        let join_data = join_data(scenario, dynamic, vec![definitions], None);
        let mut candidates = ClientBootstrapLocalCandidates::default();
        candidates.extend_from_roots(&join_data, [&directory.root]);

        let plan = plan_client_bootstrap(&join_data, &candidates, &directory.standalone).unwrap();

        let definitions = plan
            .resources()
            .iter()
            .find(|resource| resource.core.id == 9)
            .unwrap();
        assert!(matches!(
            &definitions.source,
            ClientBootstrapResourceSource::Local(local)
                if local.source_path() == definitions_path
        ));
    }

    #[test]
    fn honors_the_configured_extensionless_directory_recursion_limit() {
        // SetByCore descends only extensionless directories and stops at
        // Config.Network.MaxResSearchRecursion (src/C4Network2Res.cpp:467-490;
        // src/C4Config.cpp:238-240).
        let directory = TestDirectory::new();
        let nested = directory.root.join("Expansion").join("Missions");
        fs::create_dir_all(&nested).unwrap();
        let definitions_path = nested.join("Objects.c4d");
        let definitions_bytes = b"deep definitions";
        fs::write(&definitions_path, definitions_bytes).unwrap();
        let scenario = resource_core(1, 7, true, b"Tutorial.c4s", b"scenario");
        let dynamic = resource_core(2, 8, true, b"DynTutorial.c4s", b"dynamic");
        let definitions = resource_core(4, 9, false, b"Objects.c4d", definitions_bytes);
        let join_data = join_data(scenario, dynamic, vec![definitions], None);
        let mut candidates = ClientBootstrapLocalCandidates::default();
        candidates.extend_from_roots(&join_data, [&directory.root]);

        assert!(plan_client_bootstrap(&join_data, &candidates, &directory.standalone).is_err());

        candidates.set_max_search_recursion(2);
        let plan = plan_client_bootstrap(&join_data, &candidates, &directory.standalone).unwrap();
        let definitions = plan
            .resources()
            .iter()
            .find(|resource| resource.core.id == 9)
            .unwrap();
        assert!(matches!(
            &definitions.source,
            ClientBootstrapResourceSource::Local(local)
                if local.source_path() == definitions_path
        ));
    }

    #[test]
    fn excludes_the_network_work_path_from_recursive_local_search() {
        // SetByCore explicitly omits Config.Network.WorkPath while walking
        // extensionless directories (src/C4Network2Res.cpp:478-490;
        // src/StdFile.cpp:696-705).
        let directory = TestDirectory::new();
        fs::create_dir(&directory.standalone).unwrap();
        let cached_path = directory.standalone.join("Objects.c4d");
        let definitions_bytes = b"cached definitions";
        fs::write(&cached_path, definitions_bytes).unwrap();
        let scenario = resource_core(1, 7, true, b"Tutorial.c4s", b"scenario");
        let dynamic = resource_core(2, 8, true, b"DynTutorial.c4s", b"dynamic");
        let definitions = resource_core(4, 9, false, b"Objects.c4d", definitions_bytes);
        let join_data = join_data(scenario, dynamic, vec![definitions], None);
        let mut candidates = ClientBootstrapLocalCandidates::default();
        candidates.extend_from_roots(&join_data, [&directory.root]);

        let error =
            plan_client_bootstrap(&join_data, &candidates, &directory.standalone).unwrap_err();

        assert!(matches!(
            error,
            ClientBootstrapPlanError::MissingRequiredNonLoadable { resource_id: 9, .. }
        ));
    }

    #[test]
    fn prioritizing_the_canonical_system_keeps_fallback_candidates() {
        // SetByCore treats a failed direct SetByFile probe as a miss and keeps
        // searching for an identical resource (src/C4Network2Res.cpp:441-493).
        let directory = TestDirectory::new();
        let canonical = directory.root.join("System.c4g");
        let fallback = directory.root.join("FallbackSystem.c4g");
        fs::write(&canonical, b"wrong system").unwrap();
        let system_bytes = b"contents-identical system";
        fs::write(&fallback, system_bytes).unwrap();
        let scenario = resource_core(1, 7, true, b"Tutorial.c4s", b"scenario");
        let dynamic = resource_core(2, 8, true, b"DynTutorial.c4s", b"dynamic");
        let system = resource_core(5, 9, false, b"System.c4g", system_bytes);
        let join_data = join_data(scenario, dynamic, vec![system], None);
        let mut candidates = ClientBootstrapLocalCandidates::default();
        candidates.insert(9, vec![fallback.clone()]);
        candidates.prioritize(9, canonical);

        let plan = plan_client_bootstrap(&join_data, &candidates, &directory.standalone).unwrap();

        let system = plan
            .resources()
            .iter()
            .find(|resource| resource.core.id == 9)
            .unwrap();
        assert!(matches!(
            &system.source,
            ClientBootstrapResourceSource::Local(local) if local.source_path() == fallback
        ));
    }

    #[test]
    fn recursive_search_keeps_the_complete_nested_c4_filename() {
        // GetC4Filename retains the path beginning at the first `.c4*`
        // directory. Recursive probes append that complete suffix; the
        // basename retry remains at ExePath (src/StdFile.cpp:67-81;
        // src/C4Network2Res.cpp:460-490).
        let directory = TestDirectory::new();
        let expansion = directory.root.join("Expansion");
        fs::create_dir(&expansion).unwrap();
        let misplaced = expansion.join("Castle.c4s");
        let scenario_bytes = b"misplaced nested scenario";
        fs::write(&misplaced, scenario_bytes).unwrap();
        let scenario = resource_core(1, 7, false, b"Easy.c4f/Castle.c4s", scenario_bytes);
        let dynamic = resource_core(2, 8, true, b"DynCastle.c4s", b"dynamic");
        let join_data = join_data(scenario, dynamic, Vec::new(), None);
        let mut candidates = ClientBootstrapLocalCandidates::default();
        candidates.extend_from_roots(&join_data, [&directory.root]);

        let error =
            plan_client_bootstrap(&join_data, &candidates, &directory.standalone).unwrap_err();

        assert!(matches!(
            error,
            ClientBootstrapPlanError::MissingRequiredNonLoadable {
                role: ClientBootstrapResourceRole::Scenario,
                resource_id: 7,
                ..
            }
        ));
    }

    #[test]
    fn generated_local_candidates_retain_cpp_lexical_probe_names() {
        let directory = TestDirectory::new();
        let core = resource_core(1, 7, false, b"Easy.c4f/Castle.c4s", b"scenario");
        let mut candidates = ClientBootstrapLocalCandidates::default();
        candidates.extend_search_roots([&directory.root]);

        let probes = candidates.for_core(&core, &directory.standalone);

        assert_eq!(probes[0].path(), directory.root.join("Easy.c4f/Castle.c4s"));
        assert_eq!(probes[0].lookup_name(), b"Easy.c4f/Castle.c4s");
        assert_eq!(probes[1].path(), directory.root.join("Castle.c4s"));
        assert_eq!(probes[1].lookup_name(), b"Castle.c4s");
    }

    #[cfg(unix)]
    #[test]
    fn generated_local_candidates_normalize_backslashes_only_for_physical_probes() {
        // GetC4Filename/GetFilename do not split backslashes on Unix, but
        // C4Group::Open subsequently converts them to native separators
        // before touching the filesystem (src/StdFile.cpp:41-81;
        // src/C4Group.cpp:660-668).
        let directory = TestDirectory::new();
        let core = resource_core(1, 7, false, b"Defs\\Objects.c4d", b"definitions");
        let mut candidates = ClientBootstrapLocalCandidates::default();
        candidates.extend_search_roots([&directory.root]);

        let probes = candidates.for_core(&core, &directory.standalone);

        assert_eq!(probes[0].path(), directory.root.join("Defs/Objects.c4d"));
        assert_eq!(probes[0].lookup_name(), b"Defs\\Objects.c4d");
        assert!(!probes
            .iter()
            .any(|probe| probe.path() == directory.root.join("Objects.c4d")));
    }

    #[test]
    fn retries_a_nested_c4_resource_by_basename_at_the_search_root() {
        // After the complete C4-relative path misses, SetByCore retries only
        // GetFilename at ExePath (src/C4Network2Res.cpp:460-466;
        // src/StdFile.cpp:41-54,67-81).
        let directory = TestDirectory::new();
        let basename_path = directory.root.join("Castle.c4s");
        let scenario_bytes = b"basename scenario";
        fs::write(&basename_path, scenario_bytes).unwrap();
        let scenario = resource_core(1, 7, false, b"Easy.c4f/Castle.c4s", scenario_bytes);
        let dynamic = resource_core(2, 8, true, b"DynCastle.c4s", b"dynamic");
        let join_data = join_data(scenario, dynamic, Vec::new(), None);
        let mut candidates = ClientBootstrapLocalCandidates::default();
        candidates.extend_from_roots(&join_data, [&directory.root]);

        let plan = plan_client_bootstrap(&join_data, &candidates, &directory.standalone).unwrap();

        let scenario = plan
            .resources()
            .iter()
            .find(|resource| resource.core.id == 7)
            .unwrap();
        assert!(matches!(
            &scenario.source,
            ClientBootstrapResourceSource::Local(local)
                if local.source_path() == basename_path
        ));
    }

    #[cfg(unix)]
    #[test]
    fn preserves_non_utf8_bytes_while_extracting_the_c4_filename() {
        use std::os::unix::ffi::OsStrExt;

        // C++ resource cores and GetC4Filename operate on the original path
        // bytes; they do not perform an UTF-8 conversion before SetByFile
        // (src/StdFile.cpp:41-81; src/C4Network2Res.cpp:441-445).
        let filename = c4_filename_path(b"prefix/\xff.c4f/Castle.c4s");

        assert_eq!(filename.as_os_str().as_bytes(), b"\xff.c4f/Castle.c4s");
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn resolves_non_utf8_bytes_in_c4_resource_filenames() {
        use std::os::unix::ffi::OsStringExt;

        // C++ resource cores and GetC4Filename operate on the original path
        // bytes; they do not perform an UTF-8 conversion before SetByFile
        // (src/StdFile.cpp:41-81; src/C4Network2Res.cpp:441-445).
        let directory = TestDirectory::new();
        let c4_folder = std::ffi::OsString::from_vec(b"\xff.c4f".to_vec());
        let c4_folder = directory.root.join(c4_folder);
        fs::create_dir(&c4_folder).unwrap();
        let scenario_path = c4_folder.join("Castle.c4s");
        let scenario_bytes = b"non-utf8 scenario";
        fs::write(&scenario_path, scenario_bytes).unwrap();
        let scenario = resource_core(1, 7, false, b"\xff.c4f/Castle.c4s", scenario_bytes);
        let dynamic = resource_core(2, 8, true, b"DynCastle.c4s", b"dynamic");
        let join_data = join_data(scenario, dynamic, Vec::new(), None);
        let mut candidates = ClientBootstrapLocalCandidates::default();
        candidates.extend_from_roots(&join_data, [&directory.root]);

        let plan = plan_client_bootstrap(&join_data, &candidates, &directory.standalone).unwrap();

        let scenario = plan
            .resources()
            .iter()
            .find(|resource| resource.core.id == 7)
            .unwrap();
        assert!(matches!(
            &scenario.source,
            ClientBootstrapResourceSource::Local(local)
                if local.source_path() == scenario_path
        ));
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
    fn trusted_local_system_does_not_relax_nonloadable_definitions() {
        // AddLoad rejects every non-loadable core after SetByCore fails; the
        // Rust/C++ System boundary must not change Definitions behavior
        // (src/C4Network2Res.cpp:441-493,1473-1507;
        // src/C4GameParameters.cpp:125-160).
        let directory = TestDirectory::new();
        let local_system = directory.root.join("System.c4g");
        fs::create_dir(&local_system).unwrap();
        fs::write(local_system.join("Local.c"), b"local System").unwrap();
        let definitions = resource_core(4, 10, false, b"Objects.c4d", b"host definitions");
        let resolver = ClientBootstrapResolver::new(
            &ClientBootstrapLocalCandidates::default(),
            &directory.standalone,
        )
        .with_trusted_local_system_path(local_system);

        let error = resolver
            .resolve(ClientBootstrapResourceRole::GameResource, &definitions)
            .unwrap_err();

        assert!(matches!(
            error,
            ClientBootstrapPlanError::MissingRequiredNonLoadable {
                role: ClientBootstrapResourceRole::GameResource,
                resource_id: 10,
                filename,
            } if filename == b"Objects.c4d"
        ));
    }

    #[test]
    fn invalid_trusted_local_system_does_not_relax_nonloadable_system() {
        // C++ cannot load NRT_System from the network. The Rust compatibility
        // boundary therefore requires an actually openable process-local
        // System group rather than merely trusting a configured path
        // (src/C4Network2Res.cpp:1458-1461,1473-1507).
        let directory = TestDirectory::new();
        let missing_system = directory.root.join("MissingSystem.c4g");
        let system = resource_core(
            crate::HostResourceType::System as u8,
            2,
            false,
            b"System.c4g",
            b"C++ host System",
        );
        let resolver = ClientBootstrapResolver::new(
            &ClientBootstrapLocalCandidates::default(),
            &directory.standalone,
        )
        .with_trusted_local_system_path(missing_system);

        let error = resolver
            .resolve(ClientBootstrapResourceRole::GameResource, &system)
            .unwrap_err();

        assert!(matches!(
            error,
            ClientBootstrapPlanError::MissingRequiredNonLoadable {
                role: ClientBootstrapResourceRole::GameResource,
                resource_id: 2,
                filename,
            } if filename == b"System.c4g"
        ));
    }

    #[test]
    fn early_game_resource_failure_is_ignored_until_after_dynamic_and_scenario() {
        // HandleJoinData ignores its first GameRes.InitNetwork result, then
        // requires Dynamic. Outer InitClient later checks Scenario before it
        // retries GameRes (src/C4Network2.cpp:1612-1618,329-331;
        // src/C4GameParameters.cpp:539-547).
        let directory = TestDirectory::new();
        let scenario = resource_core(1, 7, true, b"Tutorial.c4s", b"scenario");
        let dynamic = resource_core(2, 8, false, b"Dynamic.c4d", b"dynamic");
        let system = resource_core(5, 9, false, b"System.c4g", b"system");
        let mut join_data = join_data(scenario, dynamic, vec![system], None);
        let mut planner = ClientBootstrapPlanner::new(
            &ClientBootstrapLocalCandidates::default(),
            directory.standalone.clone(),
        );

        let error = planner.plan_before_addresses(&mut join_data).unwrap_err();

        assert!(matches!(
            error,
            ClientBootstrapPlanError::MissingRequiredNonLoadable {
                role: ClientBootstrapResourceRole::Dynamic,
                resource_id: 8,
                ..
            }
        ));
    }

    #[test]
    fn final_phase_retries_the_first_unfinished_game_resource() {
        // The first aggregate failure stops the early GameRes pass; the final
        // pass resumes at that entry after Scenario initialization
        // (src/C4GameParameters.cpp:237-247,539-547).
        let directory = TestDirectory::new();
        let scenario = resource_core(1, 7, true, b"Tutorial.c4s", b"scenario");
        let dynamic = resource_core(2, 8, true, b"Dynamic.c4d", b"dynamic");
        let definitions = resource_core(4, 9, false, b"Objects.c4d", b"definitions");
        let mut join_data = join_data(scenario, dynamic, vec![definitions], None);
        let mut planner = ClientBootstrapPlanner::new(
            &ClientBootstrapLocalCandidates::default(),
            directory.standalone.clone(),
        );

        planner.plan_before_addresses(&mut join_data).unwrap();
        let error = planner.plan_after_addresses(&join_data).unwrap_err();

        assert!(matches!(
            error,
            ClientBootstrapPlanError::MissingRequiredNonLoadable {
                role: ClientBootstrapResourceRole::GameResource,
                resource_id: 9,
                ..
            }
        ));
    }

    #[test]
    fn later_registration_can_satisfy_the_final_game_resource_retry_by_id() {
        // AddByCore returns an existing resource ID without rechecking its
        // core. Thus Dynamic can satisfy a GameRes ID whose early resolution
        // failed before the final retry (src/C4Network2Res.cpp:1473-1477).
        let directory = TestDirectory::new();
        let scenario = resource_core(1, 7, true, b"Tutorial.c4s", b"scenario");
        let dynamic = resource_core(2, 9, true, b"Dynamic.c4d", b"dynamic");
        let definitions = resource_core(4, 9, false, b"Objects.c4d", b"definitions");
        let mut join_data = join_data(scenario, dynamic, vec![definitions], None);
        let mut planner = ClientBootstrapPlanner::new(
            &ClientBootstrapLocalCandidates::default(),
            directory.standalone.clone(),
        );

        planner.plan_before_addresses(&mut join_data).unwrap();
        let plan = planner.plan_after_addresses(&join_data).unwrap();

        assert_eq!(
            plan.resources()
                .iter()
                .map(|resource| (resource.role, resource.core.id))
                .collect::<Vec<_>>(),
            vec![
                (ClientBootstrapResourceRole::Dynamic, 9),
                (ClientBootstrapResourceRole::Scenario, 7),
            ]
        );
    }

    #[test]
    fn player_resource_flags_follow_cpp_load_resource_decision_order() {
        // Removed/no-resource entries are untouched. InScenario and failed
        // active resources clear HasRes without aborting the join
        // (src/C4PlayerInfo.cpp:275-292).
        let directory = TestDirectory::new();
        let scenario = resource_core(1, 7, true, b"Tutorial.c4s", b"scenario");
        let dynamic = resource_core(2, 8, true, b"Dynamic.c4d", b"dynamic");
        let mut join_data = join_data(scenario, dynamic, Vec::new(), None);
        let unavailable = resource_core(3, 10, false, b"Unavailable.c4p", b"missing");
        let in_scenario = resource_core(3, 11, false, b"Scenario.c4p", b"scenario player");
        let removed = resource_core(3, 12, false, b"Removed.c4p", b"removed player");
        let no_flag = resource_core(3, 13, false, b"NoFlag.c4p", b"no flag player");
        join_data.parameters.player_infos.clients = vec![ClientPlayerInfosSnapshot {
            client_id: 1,
            flags: 0,
            players: vec![
                ControlPlayerInfoEntry {
                    flags: PLAYER_INFO_FLAG_HAS_RESOURCE,
                    resource: Some(unavailable),
                    ..Default::default()
                },
                ControlPlayerInfoEntry {
                    flags: PLAYER_INFO_FLAG_HAS_RESOURCE | PLAYER_INFO_FLAG_IN_SCENARIO_FILE,
                    resource: Some(in_scenario),
                    ..Default::default()
                },
                ControlPlayerInfoEntry {
                    flags: PLAYER_INFO_FLAG_HAS_RESOURCE
                        | PLAYER_INFO_FLAG_IN_SCENARIO_FILE
                        | PLAYER_INFO_FLAG_REMOVED,
                    resource: Some(removed.clone()),
                    ..Default::default()
                },
                ControlPlayerInfoEntry {
                    flags: 0,
                    resource: Some(no_flag.clone()),
                    ..Default::default()
                },
            ],
        }];
        let mut planner = ClientBootstrapPlanner::new(
            &ClientBootstrapLocalCandidates::default(),
            directory.standalone.clone(),
        );

        planner.plan_before_addresses(&mut join_data).unwrap();

        let players = &join_data.parameters.player_infos.clients[0].players;
        assert_eq!(players[0].flags & PLAYER_INFO_FLAG_HAS_RESOURCE, 0);
        assert_eq!(players[0].resource, None);
        assert_eq!(players[1].flags & PLAYER_INFO_FLAG_HAS_RESOURCE, 0);
        assert_eq!(players[1].resource, None);
        assert_ne!(players[2].flags & PLAYER_INFO_FLAG_HAS_RESOURCE, 0);
        assert_eq!(players[2].resource, Some(removed));
        assert_eq!(players[3].flags, 0);
        assert_eq!(players[3].resource, Some(no_flag));
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
                player_infos,
                restore_player_infos: PlayerInfoListSnapshot::default(),
                teams: JoinTeamListSnapshot {
                    active: 1,
                    allow_hostility_change: 1,
                    auto_generate_teams: 1,
                    ..Default::default()
                },
                clients: JoinClientRegistrySnapshot::default(),
                ..Default::default()
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
                "clonk-rust-client-bootstrap-{}-{unique}",
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
