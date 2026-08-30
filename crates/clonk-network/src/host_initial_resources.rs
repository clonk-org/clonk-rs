//! Initial host resource publication in stock C++ ID and list order.

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use clonk_engine::{LegacyCString, NetworkResourceCore};
use thiserror::Error;

use crate::host_resource_core::{
    build_host_resource_core_with_prepared_directory, prepare_directory_standalone,
    PreparedDirectoryStandalone,
};
use crate::{
    HostConfig, HostJoinSnapshot, HostResourceCoreError, HostResourceCoreSpec,
    HostResourcePublication, HostResourceType, HostedResourceFile, InitialNetworkDynamic,
    JoinGameParametersEnvelope, ResourceFileOwnership, ResourceRegistration,
};

const MAX_TEMP_SUFFIX: u32 = 999;
const MAX_DIRECTORY_PREPARATION_WORKERS: usize = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostInitialResourceSource {
    pub path: PathBuf,
    /// Exact filename passed to `C4Network2ResList::AddByFile` for lookup.
    pub lookup_name: LegacyCString,
    /// Filename retained by the opened group and compared with later lookup
    /// names. This can differ by absoluteness, case, or wildcard expansion.
    pub opened_name: LegacyCString,
    pub wire_name: LegacyCString,
    /// Exact packed image for a group whose stable path is virtual inside a
    /// packed parent. Publication materializes it into the network directory.
    pub virtual_group_bytes: Option<Vec<u8>>,
}

#[derive(Debug, Clone)]
pub struct HostInitialResourcePublicationSpec {
    pub network_directory: PathBuf,
    pub group_maker: LegacyCString,
    pub max_load_file_size: u32,
    pub scenario: HostInitialResourceSource,
    pub definitions: Vec<HostInitialResourceSource>,
    pub system: HostInitialResourceSource,
    pub materials: Vec<HostInitialResourceSource>,
    pub players: Vec<HostInitialResourceSource>,
    pub dynamic: InitialNetworkDynamic,
    pub dynamic_wire_name: LegacyCString,
    pub parameters: JoinGameParametersEnvelope,
    pub dynamic_tick: i32,
}

#[derive(Debug, Clone)]
pub struct HostInitialResourcePublication {
    pub join_snapshot: HostJoinSnapshot,
    pub player_cores: Vec<NetworkResourceCore>,
    pub player_resource_sources: Vec<(PathBuf, NetworkResourceCore)>,
    pub resource_registrations: Vec<ResourceRegistration>,
    pub resource_directory: PathBuf,
    pub resource_files: Vec<HostedResourceFile>,
}

impl HostInitialResourcePublication {
    /// Moves the publication fields consumed by `start_host` into an existing
    /// host configuration without changing admission policy or transport
    /// settings.
    pub fn apply_to(self, config: &mut HostConfig) {
        config.initial_join_snapshot = Some(self.join_snapshot);
        config.resource_registrations = self.resource_registrations;
        config.resource_directory = Some(self.resource_directory);
        config.resource_files = self.resource_files;
        config.player_resource_sources = self.player_resource_sources;
    }
}

#[derive(Debug, Error)]
pub enum HostInitialResourcePublicationError {
    #[error("host resource ID space exhausted after {0} resources")]
    ResourceIdExhausted(usize),
    #[error("failed to publish {resource_type:?} resource {}: {source}", path.display())]
    ResourceCore {
        resource_type: HostResourceType,
        path: PathBuf,
        #[source]
        source: HostResourceCoreError,
    },
    #[error("could not create network resource directory: {0}")]
    NetworkDirectory(#[source] io::Error),
    #[error("could not materialize initial resource: {0}")]
    ResourceIo(#[source] io::Error),
    #[error("no free initial resource filename from 1 through 999")]
    NoResourceFilename,
    #[error(
        "materialized dynamic metadata differs: expected size/crc/contents {expected_size}/{expected_crc:08x}/{expected_contents_crc:08x}, got {actual_size}/{actual_crc:08x}/{actual_contents_crc:08x}"
    )]
    DynamicMetadataMismatch {
        expected_size: u32,
        expected_crc: u32,
        expected_contents_crc: u32,
        actual_size: u32,
        actual_crc: u32,
        actual_contents_crc: u32,
    },
}

/// Publishes initial host resources in the order in which C++ allocates IDs:
/// Scenario, Definitions*, System, Material*, Dynamic, Player*.
pub fn publish_host_initial_resources(
    spec: HostInitialResourcePublicationSpec,
) -> Result<HostInitialResourcePublication, HostInitialResourcePublicationError> {
    fs::create_dir_all(&spec.network_directory)
        .map_err(HostInitialResourcePublicationError::NetworkDirectory)?;
    let expected_count = 3_usize
        .checked_add(spec.definitions.len())
        .and_then(|count| count.checked_add(spec.materials.len()))
        .and_then(|count| count.checked_add(spec.players.len()))
        .ok_or(HostInitialResourcePublicationError::ResourceIdExhausted(
            usize::MAX,
        ))?;
    if expected_count > i32::MAX as usize {
        return Err(HostInitialResourcePublicationError::ResourceIdExhausted(
            expected_count,
        ));
    }

    let mut publications = SourcePublications::with_capacity(expected_count);

    let scenario_prepared =
        prepare_source_directory(&spec.scenario, spec.group_maker.as_bytes(), None);
    let scenario_core = publications.publish_or_reuse(
        &spec.scenario,
        HostResourceType::Scenario,
        &spec,
        scenario_prepared,
    )?;

    let mut game_resources =
        Vec::with_capacity(spec.definitions.len() + 1_usize.saturating_add(spec.materials.len()));
    prepare_bounded_in_order(
        &spec.definitions,
        MAX_DIRECTORY_PREPARATION_WORKERS,
        |definition| {
            prepare_source_directory(
                definition,
                spec.group_maker.as_bytes(),
                Some(u64::from(spec.max_load_file_size)),
            )
        },
        |definition, prepared| {
            let core = publications.publish_or_reuse(
                definition,
                HostResourceType::Definitions,
                &spec,
                prepared,
            )?;
            game_resources.push(core);
            Ok::<(), HostInitialResourcePublicationError>(())
        },
    )?;

    let system_core =
        publications.publish_or_reuse(&spec.system, HostResourceType::System, &spec, None)?;
    game_resources.push(system_core);

    for material in &spec.materials {
        let prepared = prepare_source_directory(material, spec.group_maker.as_bytes(), None);
        let core =
            publications.publish_or_reuse(material, HostResourceType::Material, &spec, prepared)?;
        game_resources.push(core);
    }

    let dynamic_path = materialize_resource(
        &spec.network_directory,
        spec.dynamic.group_filename.as_bytes(),
        &spec.dynamic.packed_bytes,
    )?;
    publications.temporary_files.track(dynamic_path.clone());
    let dynamic_wire_name = resolved_dynamic_wire_name(&spec.dynamic_wire_name, &dynamic_path);
    let dynamic_source = HostInitialResourceSource {
        lookup_name: dynamic_wire_name.clone(),
        opened_name: dynamic_wire_name.clone(),
        wire_name: dynamic_wire_name,
        path: dynamic_path,
        virtual_group_bytes: None,
    };
    let dynamic = publish_source(
        &dynamic_source,
        HostResourceType::Dynamic,
        publications.next_id,
        &spec,
        &mut publications.temporary_files,
        None,
    )?;
    validate_dynamic_metadata(&spec.dynamic, &dynamic.core)?;
    let dynamic_retained_name =
        retained_file_name(&dynamic_source, &dynamic, &spec.dynamic_wire_name);
    let dynamic_core = dynamic.core.clone();
    publications
        .published_sources
        .insert(dynamic_retained_name, dynamic_core.clone());
    push_publication(
        dynamic,
        &mut publications.registrations,
        &mut publications.resource_files,
    );
    publications.next_id += 1;

    let mut player_cores = Vec::with_capacity(spec.players.len());
    let mut player_resource_sources = Vec::with_capacity(spec.players.len());
    for player in &spec.players {
        let temporary_checkpoint = publications.temporary_files.checkpoint();
        match publications.publish_or_reuse(player, HostResourceType::Player, &spec, None) {
            Ok(core) => {
                player_resource_sources.push((player.path.clone(), core.clone()));
                player_cores.push(core);
            }
            Err(_) => {
                publications.temporary_files.rollback(temporary_checkpoint);
                // C4ClientPlayerInfos drops only the module whose
                // ResList.AddByFile failed and continues the participant
                // list. Required game resources remain all-or-error.
            }
        }
    }

    let mut parameters = spec.parameters.clone();
    parameters.scenario = scenario_core;
    parameters.game_resources = game_resources;
    let join_snapshot = HostJoinSnapshot {
        dynamic: dynamic_core,
        dynamic_tick: spec.dynamic_tick,
        parameters,
    };

    publications.temporary_files.disarm();
    Ok(HostInitialResourcePublication {
        join_snapshot,
        player_cores,
        player_resource_sources,
        resource_registrations: publications.registrations,
        resource_directory: spec.network_directory,
        resource_files: publications.resource_files,
    })
}

fn resolved_dynamic_wire_name(template: &LegacyCString, path: &Path) -> LegacyCString {
    let basename = path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("materialize_resource creates an ASCII basename");
    let prefix_len = template
        .as_bytes()
        .iter()
        .rposition(|byte| matches!(byte, b'/' | b'\\'))
        .map_or(0, |separator| separator + 1);
    let mut wire_name = Vec::with_capacity(prefix_len + basename.len());
    wire_name.extend_from_slice(&template.as_bytes()[..prefix_len]);
    wire_name.extend_from_slice(basename.as_bytes());
    LegacyCString::from_bytes(wire_name)
        .expect("a LegacyCString prefix and sanitized basename contain no NUL")
}

fn prepare_source_directory(
    source: &HostInitialResourceSource,
    group_maker: &[u8],
    max_source_size: Option<u64>,
) -> Option<PreparedDirectoryStandalone> {
    (source.virtual_group_bytes.is_none()
        && fs::metadata(&source.path).is_ok_and(|metadata| metadata.is_dir()))
    .then(|| prepare_directory_standalone(&source.path, group_maker, max_source_size))
    .and_then(Result::ok)
}

fn publish_source(
    source: &HostInitialResourceSource,
    resource_type: HostResourceType,
    resource_id: i32,
    spec: &HostInitialResourcePublicationSpec,
    temporary_files: &mut TemporaryFiles,
    prepared_directory: Option<PreparedDirectoryStandalone>,
) -> Result<HostResourcePublication, HostInitialResourcePublicationError> {
    let (source_path, source_ownership) = if let Some(bytes) = source.virtual_group_bytes.as_deref()
    {
        let path = materialize_resource(
            &spec.network_directory,
            source.opened_name.as_bytes(),
            bytes,
        )?;
        temporary_files.track(path.clone());
        (path, ResourceFileOwnership::Temporary)
    } else if resource_type == HostResourceType::Dynamic {
        (source.path.clone(), ResourceFileOwnership::Temporary)
    } else {
        (source.path.clone(), ResourceFileOwnership::Persistent)
    };
    let mut core_spec = HostResourceCoreSpec::new_with_raw_group_maker(
        resource_type,
        resource_id,
        source.wire_name.clone(),
        spec.group_maker.clone(),
    )
    .with_source_ownership(source_ownership)
    .with_standalone_name(source.opened_name.clone());
    if resource_type == HostResourceType::Definitions {
        core_spec = core_spec.with_max_load_file_size(spec.max_load_file_size);
    }
    let mut publication = build_host_resource_core_with_prepared_directory(
        &source_path,
        &spec.network_directory,
        core_spec,
        prepared_directory,
    )
    .map_err(|error| HostInitialResourcePublicationError::ResourceCore {
        resource_type,
        path: source.path.clone(),
        source: error,
    })?;
    if source_ownership == ResourceFileOwnership::Temporary && publication.standalone_path.is_none()
    {
        publication.standalone_path = Some(publication.source_path.clone());
        publication.standalone_ownership = Some(ResourceFileOwnership::Temporary);
    }
    if !spec.parameters.league_address.is_empty()
        && matches!(
            resource_type,
            HostResourceType::Scenario
                | HostResourceType::Definitions
                | HostResourceType::System
                | HostResourceType::Material
        )
    {
        // CalcHash ignores CalculateSHA's false return, retaining publication
        // without FileSHA when the physical read fails.
        let _ = publication.calculate_file_sha();
    }
    if publication.standalone_ownership == Some(ResourceFileOwnership::Temporary) {
        if let Some(path) = publication.standalone_path.as_ref() {
            temporary_files.track(path.clone());
        }
    }
    Ok(publication)
}

struct SourcePublications {
    next_id: i32,
    temporary_files: TemporaryFiles,
    published_sources: HashMap<LegacyCString, NetworkResourceCore>,
    registrations: Vec<ResourceRegistration>,
    resource_files: Vec<HostedResourceFile>,
}

impl SourcePublications {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            next_id: 0,
            temporary_files: TemporaryFiles::default(),
            published_sources: HashMap::with_capacity(capacity),
            registrations: Vec::with_capacity(capacity),
            resource_files: Vec::with_capacity(capacity),
        }
    }

    fn publish_or_reuse(
        &mut self,
        source: &HostInitialResourceSource,
        resource_type: HostResourceType,
        spec: &HostInitialResourcePublicationSpec,
        prepared_directory: Option<PreparedDirectoryStandalone>,
    ) -> Result<NetworkResourceCore, HostInitialResourcePublicationError> {
        // AddByFile searches the incoming source filename against the name
        // retained after each earlier group open. That comparison is
        // deliberately asymmetric for absolute, case-corrected and wildcard
        // aliases (src/C4Network2Res.cpp:373-424,1397-1405,1443-1449).
        if let Some(core) = self.published_sources.get(&source.lookup_name) {
            return Ok(core.clone());
        }

        // C4Network2ResList::AddByFile reserves nextResID before SetByFile
        // and GetStandalone. A failed player therefore leaves an ID hole even
        // though its row is skipped and later modules continue.
        let resource_id = self.next_id;
        self.next_id += 1;
        let publication = publish_source(
            source,
            resource_type,
            resource_id,
            spec,
            &mut self.temporary_files,
            prepared_directory,
        )?;
        let retained_name = retained_file_name(source, &publication, &spec.dynamic_wire_name);
        let core = publication.core.clone();
        self.published_sources.insert(retained_name, core.clone());
        push_publication(
            publication,
            &mut self.registrations,
            &mut self.resource_files,
        );
        Ok(core)
    }
}

/// `GetStandalone` rewrites `C4Network2Res::szFile` only when it packs a
/// physical directory. Packed files, virtual children of packed parents and
/// unloadable directories retain the name produced by the original group
/// open, which is the key later `AddByFile` calls compare against.
fn retained_file_name(
    source: &HostInitialResourceSource,
    publication: &HostResourcePublication,
    network_path_template: &LegacyCString,
) -> LegacyCString {
    let packed_physical_directory = source.virtual_group_bytes.is_none()
        && fs::metadata(&source.path).is_ok_and(|metadata| metadata.is_dir())
        && publication.standalone_path.is_some();
    if packed_physical_directory {
        let path = publication
            .standalone_path
            .as_deref()
            .expect("a packed physical directory has a standalone path");
        // The cache directory used by the Rust host is an implementation
        // detail. Native rewrites szFile through Config.Network.WorkPath, and
        // later AddByFile compares that logical lexical string exactly.
        resolved_dynamic_wire_name(network_path_template, path)
    } else {
        source.opened_name.clone()
    }
}

fn push_publication(
    publication: HostResourcePublication,
    registrations: &mut Vec<ResourceRegistration>,
    files: &mut Vec<HostedResourceFile>,
) {
    let binary_compatible = publication.core.loadable && publication.standalone_path.is_some();
    let path = publication
        .standalone_path
        .unwrap_or(publication.source_path);
    let ownership = publication
        .standalone_ownership
        .unwrap_or(ResourceFileOwnership::Persistent);
    registrations.push(ResourceRegistration::from_core(
        &publication.core,
        binary_compatible,
        false,
    ));
    files.push(HostedResourceFile {
        core: publication.core,
        path,
        ownership,
        binary_compatible,
    });
}

fn validate_dynamic_metadata(
    expected: &InitialNetworkDynamic,
    actual: &NetworkResourceCore,
) -> Result<(), HostInitialResourcePublicationError> {
    if actual.file_size == expected.file_size
        && actual.file_crc == expected.file_crc
        && actual.contents_crc == expected.contents_crc
    {
        return Ok(());
    }
    Err(
        HostInitialResourcePublicationError::DynamicMetadataMismatch {
            expected_size: expected.file_size,
            expected_crc: expected.file_crc,
            expected_contents_crc: expected.contents_crc,
            actual_size: actual.file_size,
            actual_crc: actual.file_crc,
            actual_contents_crc: actual.contents_crc,
        },
    )
}

fn materialize_resource(
    directory: &Path,
    group_filename: &[u8],
    data: &[u8],
) -> Result<PathBuf, HostInitialResourcePublicationError> {
    let basename = crate::host_resource_core::network_temp_basename(group_filename);
    for suffix in 1..=MAX_TEMP_SUFFIX {
        let candidate = crate::host_resource_core::network_temp_candidate(&basename, suffix);
        let candidate = String::from_utf8(candidate).expect("FindTempResFileName produces ASCII");
        let path = directory.join(candidate);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                if let Err(error) = file.write_all(data) {
                    let _ = fs::remove_file(&path);
                    return Err(HostInitialResourcePublicationError::ResourceIo(error));
                }
                return Ok(path);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(HostInitialResourcePublicationError::ResourceIo(error)),
        }
    }
    Err(HostInitialResourcePublicationError::NoResourceFilename)
}

#[derive(Default)]
struct TemporaryFiles {
    paths: Vec<PathBuf>,
    armed: bool,
}

impl TemporaryFiles {
    fn checkpoint(&self) -> usize {
        self.paths.len()
    }

    fn track(&mut self, path: PathBuf) {
        self.armed = true;
        self.paths.push(path);
    }

    fn disarm(&mut self) {
        self.armed = false;
    }

    fn rollback(&mut self, checkpoint: usize) {
        for path in self.paths.drain(checkpoint..) {
            let _ = fs::remove_file(path);
        }
    }
}

impl Drop for TemporaryFiles {
    fn drop(&mut self) {
        if self.armed {
            self.paths.iter().for_each(|path| {
                let _ = fs::remove_file(path);
            });
        }
    }
}

fn prepare_bounded_in_order<T, R, E>(
    items: &[T],
    worker_limit: usize,
    prepare: impl Fn(&T) -> R + Sync,
    mut commit: impl FnMut(&T, R) -> Result<(), E>,
) -> Result<(), E>
where
    T: Sync,
    R: Send,
{
    let worker_limit = worker_limit.max(1);
    for batch in items.chunks(worker_limit) {
        let prepared = std::thread::scope(|scope| {
            let prepare = &prepare;
            batch
                .iter()
                .map(|item| scope.spawn(move || prepare(item)))
                .collect::<Vec<_>>()
                .into_iter()
                .map(|worker| match worker.join() {
                    Ok(prepared) => prepared,
                    Err(panic) => std::panic::resume_unwind(panic),
                })
                .collect::<Vec<_>>()
        });
        for (item, prepared) in batch.iter().zip(prepared) {
            commit(item, prepared)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Barrier;

    use super::prepare_bounded_in_order;

    #[test]
    fn bounded_preparation_runs_two_workers_and_commits_in_source_order() {
        let items = [0_usize, 1, 2, 3];
        let barrier = Barrier::new(2);
        let active = AtomicUsize::new(0);
        let maximum = AtomicUsize::new(0);
        let mut committed = Vec::new();

        prepare_bounded_in_order(
            &items,
            2,
            |item| {
                let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                maximum.fetch_max(now, Ordering::SeqCst);
                barrier.wait();
                active.fetch_sub(1, Ordering::SeqCst);
                item * 10
            },
            |item, prepared| {
                committed.push((*item, prepared));
                Ok::<(), ()>(())
            },
        )
        .expect("commit prepared items");

        assert_eq!(maximum.load(Ordering::SeqCst), 2);
        assert_eq!(committed, [(0, 0), (1, 10), (2, 20), (3, 30)]);
    }

    #[test]
    fn bounded_preparation_reports_failures_in_source_order() {
        let items = [0_usize, 1];
        let barrier = Barrier::new(2);
        let prepared = AtomicUsize::new(0);
        let mut committed = Vec::new();

        let result = prepare_bounded_in_order(
            &items,
            2,
            |item| {
                prepared.fetch_add(1, Ordering::SeqCst);
                barrier.wait();
                Err::<(), _>(*item)
            },
            |item, result| {
                committed.push(*item);
                result
            },
        );

        assert_eq!(prepared.load(Ordering::SeqCst), 2);
        assert_eq!(committed, [0]);
        assert_eq!(result, Err(0));
    }
}
