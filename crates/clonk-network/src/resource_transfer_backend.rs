//! Operational backend for the stock C4 network-resource state machine.
//!
//! `ResourceCatalog` deliberately describes filesystem and transport effects.
//! This type executes the filesystem effects, feeds successful writes back into
//! the catalog, and leaves socket delivery as typed events for its caller.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::fmt;
use std::path::{Path, PathBuf};

use clonk_engine::NetworkResourceCore;

use crate::{
    build_host_resource_core, ChunkStoreOutcome, ChunkWriteOutcome, HostResourceCoreError,
    HostResourceCoreSpec, HostResourceType, ResourceCatalog, ResourceCatalogAction,
    ResourceDataPacket, ResourceFileOwnership, ResourceFileStore, ResourceFileStoreError,
    ResourcePacket, ResourceRegistration,
};

/// Opaque handle for the file protected by [`ResourceTransferBackend::begin_derive`].
///
/// The handle is consumed when the control host finishes and announces the
/// derivation. Non-host peers retain only the anonymous backend state and
/// finish it when the matching `PID_NetResDerive` arrives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceDerivation {
    parent_resource_id: i32,
    source_path: PathBuf,
    source_ownership: ResourceFileOwnership,
}

impl ResourceDerivation {
    pub fn parent_resource_id(&self) -> i32 {
        self.parent_resource_id
    }
}

/// Externally observable work produced after all local catalog actions run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceTransferEvent {
    /// A stock catalog transport action, preserved without translation.
    Transport(ResourceCatalogAction),
    /// One successfully stored chunk reports the local completion percentage.
    Progress {
        resource_id: i32,
        present_percent: u8,
    },
    /// The final chunk made a remotely loaded resource complete.
    Completed {
        resource_id: i32,
        core: NetworkResourceCore,
        path: PathBuf,
    },
    /// Discovery/load timeout removed the temporary resource file.
    LoadFailed { resource_id: i32 },
}

#[derive(Debug)]
pub enum ResourceTransferError {
    FileStore(ResourceFileStoreError),
    DuplicateResource(i32),
    CatalogRegistrationRejected(i32),
    MissingCore(i32),
    MissingPath(i32),
    UnsupportedResourceType(u8),
    DerivedResourceNotLoadable(i32),
    CatalogDerivationMissing(i32),
    ResourceCore(HostResourceCoreError),
    ChunkIndexOverflow(u32),
    CatalogRejectedStoredChunk {
        resource_id: i32,
        chunk: u32,
        outcome: ChunkStoreOutcome,
    },
    CompletionStateMismatch {
        resource_id: i32,
        file_complete: bool,
        catalog_outcome: ChunkStoreOutcome,
    },
}

impl fmt::Display for ResourceTransferError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FileStore(error) => write!(formatter, "resource storage failed: {error}"),
            Self::DuplicateResource(resource_id) => {
                write!(formatter, "resource {resource_id} is already registered")
            }
            Self::CatalogRegistrationRejected(resource_id) => {
                write!(formatter, "catalog rejected resource {resource_id}")
            }
            Self::MissingCore(resource_id) => {
                write!(formatter, "resource {resource_id} has no retained core")
            }
            Self::MissingPath(resource_id) => {
                write!(formatter, "resource {resource_id} has no retained path")
            }
            Self::UnsupportedResourceType(resource_type) => {
                write!(formatter, "resource type {resource_type} cannot be derived")
            }
            Self::DerivedResourceNotLoadable(resource_id) => {
                write!(formatter, "derived resource {resource_id} has no standalone")
            }
            Self::CatalogDerivationMissing(resource_id) => {
                write!(formatter, "resource {resource_id} has no anonymous derivation")
            }
            Self::ResourceCore(error) => {
                write!(formatter, "derived resource build failed: {error}")
            }
            Self::ChunkIndexOverflow(chunk) => {
                write!(formatter, "resource chunk {chunk} does not fit an int32")
            }
            Self::CatalogRejectedStoredChunk {
                resource_id,
                chunk,
                outcome,
            } => write!(
                formatter,
                "catalog rejected stored chunk {chunk} for resource {resource_id}: {outcome:?}"
            ),
            Self::CompletionStateMismatch {
                resource_id,
                file_complete,
                catalog_outcome,
            } => write!(
                formatter,
                "resource {resource_id} completion mismatch: file={file_complete}, catalog={catalog_outcome:?}"
            ),
        }
    }
}

impl std::error::Error for ResourceTransferError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::FileStore(error) => Some(error),
            Self::ResourceCore(error) => Some(error),
            _ => None,
        }
    }
}

impl From<ResourceFileStoreError> for ResourceTransferError {
    fn from(error: ResourceFileStoreError) -> Self {
        Self::FileStore(error)
    }
}

impl From<HostResourceCoreError> for ResourceTransferError {
    fn from(error: HostResourceCoreError) -> Self {
        Self::ResourceCore(error)
    }
}

/// Owns the synchronized resource catalog, its files, and their full cores.
#[derive(Debug)]
pub struct ResourceTransferBackend {
    catalog: ResourceCatalog,
    files: ResourceFileStore,
    cores: HashMap<i32, NetworkResourceCore>,
    local_sources: HashMap<i32, PathBuf>,
}

impl ResourceTransferBackend {
    pub fn new(
        local_client_id: i32,
        resource_directory: impl AsRef<Path>,
    ) -> Result<Self, ResourceTransferError> {
        Ok(Self {
            catalog: ResourceCatalog::new(local_client_id),
            files: ResourceFileStore::new(resource_directory)?,
            cores: HashMap::new(),
            local_sources: HashMap::new(),
        })
    }

    /// Registers a binary-compatible standalone as a complete local source.
    pub fn register_local_complete(
        &mut self,
        core: NetworkResourceCore,
        path: impl AsRef<Path>,
        ownership: ResourceFileOwnership,
        binary_compatible: bool,
    ) -> Result<(), ResourceTransferError> {
        self.ensure_unregistered(core.id)?;
        self.files.register_local_complete(&core, path, ownership)?;
        if !self.catalog.register(ResourceRegistration::from_core(
            &core,
            binary_compatible,
            false,
        )) {
            let _ = self.files.remove(core.id);
            return Err(ResourceTransferError::CatalogRegistrationRejected(core.id));
        }
        self.cores.insert(core.id, core);
        Ok(())
    }

    /// Registers one host-published resource. C++ keeps NRT_System and any
    /// over-limit definition logically present even though they have no
    /// standalone/chunks; loadable resources retain the complete-file path.
    pub fn register_hosted_resource(
        &mut self,
        core: NetworkResourceCore,
        path: impl AsRef<Path>,
        ownership: ResourceFileOwnership,
        binary_compatible: bool,
    ) -> Result<(), ResourceTransferError> {
        if core.loadable {
            self.register_local_complete(core, path, ownership, binary_compatible)
        } else {
            self.register_local_logical(core, path)
        }
    }

    /// Registers local logical data whose standalone bytes must not be served
    /// as the official core. This covers contents-identical repacks and the
    /// explicit Rust/C++ local-System compatibility boundary.
    pub fn register_local_logical(
        &mut self,
        core: NetworkResourceCore,
        path: impl AsRef<Path>,
    ) -> Result<(), ResourceTransferError> {
        self.ensure_unregistered(core.id)?;
        if !self
            .catalog
            .register(ResourceRegistration::from_core(&core, false, false))
        {
            return Err(ResourceTransferError::CatalogRegistrationRejected(core.id));
        }
        self.local_sources
            .insert(core.id, path.as_ref().to_path_buf());
        self.cores.insert(core.id, core);
        Ok(())
    }

    /// Creates the exclusive temporary file used while loading a remote core.
    pub fn register_remote_loadable(
        &mut self,
        core: NetworkResourceCore,
    ) -> Result<PathBuf, ResourceTransferError> {
        self.ensure_unregistered(core.id)?;
        let path = self.files.create_remote(&core)?;
        // SetLoad assigns the temporary path to szStandalone, which makes the
        // in-progress resource binary-compatible unconditionally
        // (src/C4Network2Res.cpp:496-523,553-560).
        if !self
            .catalog
            .register(ResourceRegistration::from_core(&core, true, true))
        {
            let _ = self.files.remove(core.id);
            return Err(ResourceTransferError::CatalogRegistrationRejected(core.id));
        }
        self.cores.insert(core.id, core);
        Ok(path)
    }

    pub fn catalog(&self) -> &ResourceCatalog {
        &self.catalog
    }

    /// Narrows or restores the per-peer window on the catalog that schedules
    /// this backend's downloads. A client that has a backend downloads through
    /// it, so this is the only reachable place to apply the in-game narrowing;
    /// see [`ResourceCatalog::set_max_loads_per_peer`].
    pub fn set_max_loads_per_peer(&mut self, max_loads_per_peer: usize) {
        self.catalog.set_max_loads_per_peer(max_loads_per_peer);
    }

    /// Reconciles a request that could not be placed on the selected peer's
    /// transport. The catalog owns the pre-send reservation and native refill
    /// policy; filesystem state is unaffected.
    pub fn on_request_send_failed<F>(
        &mut self,
        peer_id: i32,
        request: &crate::ResourceRequestPacket,
        now_seconds: u64,
        unavailable_peers: &BTreeSet<i32>,
        safe_random: F,
    ) -> Vec<ResourceCatalogAction>
    where
        F: FnMut(usize) -> usize,
    {
        self.catalog.on_request_send_failed(
            peer_id,
            request,
            now_seconds,
            unavailable_peers,
            safe_random,
        )
    }

    pub fn core(&self, resource_id: i32) -> Option<&NetworkResourceCore> {
        self.cores.get(&resource_id)
    }

    pub fn path(&self, resource_id: i32) -> Option<&Path> {
        self.files
            .path(resource_id)
            .or_else(|| self.local_sources.get(&resource_id).map(PathBuf::as_path))
    }

    /// Protects a complete resource before its mutable source is rewritten.
    ///
    /// This mirrors `C4Network2Res::Derive`: the old serving bytes are rescued
    /// when necessary and an anonymous `SetDerived` catalog entry is created.
    pub fn begin_derive(
        &mut self,
        parent_resource_id: i32,
        source_path: impl AsRef<Path>,
        source_ownership: ResourceFileOwnership,
        now_seconds: u64,
    ) -> Result<ResourceDerivation, ResourceTransferError> {
        if !self.cores.contains_key(&parent_resource_id) {
            return Err(ResourceTransferError::MissingCore(parent_resource_id));
        }
        let source_path = source_path.as_ref().to_path_buf();
        self.files
            .begin_derive(parent_resource_id, &source_path, source_ownership)?;
        self.catalog
            .register_anonymous_derived_at(parent_resource_id, true, now_seconds);
        Ok(ResourceDerivation {
            parent_resource_id,
            source_path,
            source_ownership,
        })
    }

    /// Rebuilds and announces a locally-authored anonymous derivation.
    ///
    /// The caller supplies the already-allocated resource ID. The returned
    /// transport event contains the exact `PID_NetResDerive` core emitted by
    /// C++ `FinishDerive`.
    pub fn finish_derive(
        &mut self,
        derivation: ResourceDerivation,
        resource_id: i32,
    ) -> Result<(NetworkResourceCore, Vec<ResourceTransferEvent>), ResourceTransferError> {
        self.ensure_unregistered(resource_id)?;
        let parent = self
            .cores
            .get(&derivation.parent_resource_id)
            .cloned()
            .ok_or(ResourceTransferError::MissingCore(
                derivation.parent_resource_id,
            ))?;
        let resource_type = host_resource_type(parent.resource_type)?;
        let publication = build_host_resource_core(
            &derivation.source_path,
            self.files.root(),
            HostResourceCoreSpec::new_with_raw_group_maker(
                resource_type,
                resource_id,
                parent.filename,
                parent.author,
            )
            .with_source_ownership(derivation.source_ownership),
        )?;
        let mut core = publication.core;
        core.derived_id = derivation.parent_resource_id;
        let path = publication.standalone_path.ok_or(
            ResourceTransferError::DerivedResourceNotLoadable(resource_id),
        )?;
        let ownership = publication.standalone_ownership.ok_or(
            ResourceTransferError::DerivedResourceNotLoadable(resource_id),
        )?;
        self.files
            .replace_pending_derived_file(derivation.parent_resource_id, path, ownership)?;
        let actions = self.catalog.finish_local_derived(&core);
        if actions.is_empty() {
            return Err(ResourceTransferError::CatalogDerivationMissing(
                derivation.parent_resource_id,
            ));
        }
        let mut no_random = |_| 0;
        let events = self.process_actions(actions, 0, &mut no_random)?;
        Ok((core, events))
    }

    /// Marks resources in the departed client's ID namespace for the
    /// catalog's delayed removal, matching `C4Network2ResList::RemoveAtClient`.
    pub fn remove_at_client(&mut self, client_id: i32) -> usize {
        self.catalog.remove_at_client(client_id)
    }

    /// Marks one resource removed without deleting its retained file. C++
    /// keeps the entry alive for delayed cleanup after `Remove`.
    pub fn remove_resource(&mut self, resource_id: i32) -> bool {
        self.catalog.remove_resource(resource_id)
    }

    pub fn on_peer_connected<F>(
        &mut self,
        peer_id: i32,
        now_seconds: u64,
        safe_random: &mut F,
    ) -> Result<Vec<ResourceTransferEvent>, ResourceTransferError>
    where
        F: FnMut(usize) -> usize,
    {
        self.process_actions(
            self.catalog.on_peer_connected(peer_id),
            now_seconds,
            safe_random,
        )
    }

    pub fn on_packet<F>(
        &mut self,
        peer_id: i32,
        packet: &ResourcePacket,
        now_seconds: u64,
        safe_random: &mut F,
    ) -> Result<Vec<ResourceTransferEvent>, ResourceTransferError>
    where
        F: FnMut(usize) -> usize,
    {
        let actions = self.catalog.on_packet_at(peer_id, packet, now_seconds);
        self.process_actions(actions, now_seconds, safe_random)
    }

    pub fn on_timer<F>(
        &mut self,
        now_seconds: u64,
        safe_random: &mut F,
    ) -> Result<Vec<ResourceTransferEvent>, ResourceTransferError>
    where
        F: FnMut(usize) -> usize,
    {
        let (actions, expired_resource_ids) =
            self.catalog.on_timer_with_expired_resource_ids(now_seconds);
        let events = self.process_actions(actions, now_seconds, safe_random)?;
        for resource_id in expired_resource_ids {
            self.clear_expired_resource(resource_id)?;
        }
        Ok(events)
    }

    /// Executes catalog effects to quiescence. Generated transport work stays
    /// byte-for-byte represented by the original `ResourceCatalogAction`.
    pub fn process_actions<F>(
        &mut self,
        actions: impl IntoIterator<Item = ResourceCatalogAction>,
        now_seconds: u64,
        safe_random: &mut F,
    ) -> Result<Vec<ResourceTransferEvent>, ResourceTransferError>
    where
        F: FnMut(usize) -> usize,
    {
        let mut pending = actions.into_iter().collect::<VecDeque<_>>();
        let mut events = Vec::new();
        while let Some(action) = pending.pop_front() {
            match action {
                action @ (ResourceCatalogAction::Broadcast { .. }
                | ResourceCatalogAction::SendToPeer { .. }) => {
                    events.push(ResourceTransferEvent::Transport(action));
                }
                ResourceCatalogAction::ServeChunk {
                    peer_id,
                    resource_id,
                    chunk,
                } => {
                    // C4Network2Res::SendChunk and C4Network2ResChunk::Set read
                    // at chunk*ChunkSize with the fixed 100 KiB data cap
                    // (src/C4Network2Res.cpp:848-865,1230-1260).
                    let data = match self.files.read_chunk(resource_id, chunk) {
                        Ok(data) => data,
                        // C4Network2Res::SendChunk ignores Set's return value:
                        // once id/range/connection preconditions passed, a
                        // failed open, seek, or read is sent as an empty data
                        // packet (src/C4Network2Res.cpp:848-865,1230-1260).
                        Err(ResourceFileStoreError::Io(_)) => Vec::new(),
                        Err(error) => return Err(error.into()),
                    };
                    events.push(ResourceTransferEvent::Transport(
                        ResourceCatalogAction::SendToPeer {
                            peer_id,
                            packet: ResourcePacket::Data(ResourceDataPacket {
                                resource_id,
                                chunk,
                                data,
                            }),
                        },
                    ));
                }
                ResourceCatalogAction::StoreChunk { packet, .. } => {
                    // AddTo writes first; OnChunk only records and schedules
                    // after that succeeds (src/C4Network2Res.cpp:911-940,
                    // 1263-1318).
                    let write_outcome =
                        match self
                            .files
                            .write_chunk(packet.resource_id, packet.chunk, &packet.data)
                        {
                            Ok(outcome) => outcome,
                            Err(error) if is_discarded_store_error(&error) => {
                                // AddTo failure leaves the matching load wait and
                                // timestamp untouched, but OnChunk still calls
                                // StartNewLoads for other free slots.
                                pending.push_front(ResourceCatalogAction::RefillRequests {
                                    resource_id: packet.resource_id,
                                });
                                continue;
                            }
                            Err(error) => return Err(error.into()),
                        };
                    if matches!(write_outcome, ChunkWriteOutcome::WrittenOutsideChunkRange) {
                        pending.push_front(ResourceCatalogAction::RefillRequests {
                            resource_id: packet.resource_id,
                        });
                        continue;
                    }
                    let Ok(chunk) = i32::try_from(packet.chunk) else {
                        pending.push_front(ResourceCatalogAction::RefillRequests {
                            resource_id: packet.resource_id,
                        });
                        continue;
                    };
                    let catalog_outcome =
                        self.catalog.record_chunk_stored(packet.resource_id, chunk);
                    match catalog_outcome {
                        ChunkStoreOutcome::Stored => {
                            if let Some(progress) =
                                resource_progress_event(&self.catalog, packet.resource_id)
                            {
                                events.push(progress);
                            }
                            if matches!(
                                write_outcome,
                                ChunkWriteOutcome::Stored { complete: true, .. }
                            ) {
                                return Err(ResourceTransferError::CompletionStateMismatch {
                                    resource_id: packet.resource_id,
                                    file_complete: true,
                                    catalog_outcome,
                                });
                            }
                            pending.push_front(ResourceCatalogAction::RefillRequests {
                                resource_id: packet.resource_id,
                            });
                        }
                        ChunkStoreOutcome::Completed => {
                            if let Some(progress) =
                                resource_progress_event(&self.catalog, packet.resource_id)
                            {
                                events.push(progress);
                            }
                            if !matches!(
                                write_outcome,
                                ChunkWriteOutcome::Stored { complete: true, .. }
                            ) {
                                return Err(ResourceTransferError::CompletionStateMismatch {
                                    resource_id: packet.resource_id,
                                    file_complete: false,
                                    catalog_outcome,
                                });
                            }
                            let core =
                                self.cores.get(&packet.resource_id).cloned().ok_or(
                                    ResourceTransferError::MissingCore(packet.resource_id),
                                )?;
                            let path = self
                                .files
                                .path(packet.resource_id)
                                .map(Path::to_path_buf)
                                .ok_or(ResourceTransferError::MissingPath(
                                packet.resource_id,
                            ))?;
                            events.push(ResourceTransferEvent::Completed {
                                resource_id: packet.resource_id,
                                core,
                                path,
                            });
                        }
                        outcome @ (ChunkStoreOutcome::UnknownResource
                        | ChunkStoreOutcome::NotLoading
                        | ChunkStoreOutcome::InvalidChunk) => {
                            return Err(ResourceTransferError::CatalogRejectedStoredChunk {
                                resource_id: packet.resource_id,
                                chunk: packet.chunk,
                                outcome,
                            });
                        }
                    }
                }
                ResourceCatalogAction::ScheduleFromPeer {
                    resource_id,
                    peer_id,
                } => {
                    // OnStatus calls StartLoad exactly once. GetChunkToRetrieve
                    // draws from the eligible chunks (src/C4Network2Res.cpp:
                    // 886-909,1066-1110).
                    let eligible =
                        eligible_chunk_count(&self.catalog, resource_id, peer_id, now_seconds);
                    if eligible != 0 {
                        let choice = safe_random(eligible) % eligible;
                        if let Some(request) =
                            self.catalog
                                .schedule_request(resource_id, peer_id, choice, now_seconds)
                        {
                            pending.push_back(request);
                        }
                    }
                }
                ResourceCatalogAction::RefillRequests { resource_id } => {
                    // StartNewLoads shuffles peers and fills all available slots
                    // (src/C4Network2Res.cpp:1017-1064).
                    let refill =
                        self.catalog
                            .refill_requests(resource_id, now_seconds, &mut *safe_random);
                    // StartNewLoads sends every generated request before
                    // OnChunk returns to later packet/action work.
                    refill
                        .into_iter()
                        .rev()
                        .for_each(|action| pending.push_front(action));
                }
                ResourceCatalogAction::FinishDerived { core } => {
                    let path = self.files.finish_derived(&core)?;
                    self.cores.insert(core.id, core.clone());
                    events.push(ResourceTransferEvent::Completed {
                        resource_id: core.id,
                        core,
                        path,
                    });
                }
                ResourceCatalogAction::ResourceLoadFailed { resource_id } => {
                    self.clear_expired_resource(resource_id)?;
                    events.push(ResourceTransferEvent::LoadFailed { resource_id });
                }
            }
        }
        Ok(events)
    }

    fn ensure_unregistered(&self, resource_id: i32) -> Result<(), ResourceTransferError> {
        if self.cores.contains_key(&resource_id) {
            Err(ResourceTransferError::DuplicateResource(resource_id))
        } else {
            Ok(())
        }
    }

    fn clear_expired_resource(&mut self, resource_id: i32) -> Result<(), ResourceTransferError> {
        let file_result = if self.files.path(resource_id).is_some() {
            self.files.remove(resource_id).map(|_| ())
        } else {
            Ok(())
        };
        self.cores.remove(&resource_id);
        self.local_sources.remove(&resource_id);
        file_result?;
        Ok(())
    }
}

fn resource_progress_event(
    catalog: &ResourceCatalog,
    resource_id: i32,
) -> Option<ResourceTransferEvent> {
    let chunks = catalog.local_chunks(resource_id)?;
    let total = chunks.chunk_count();
    if total <= 0 {
        return None;
    }
    let present_percent =
        (i64::from(chunks.present_chunk_count()) * 100 / i64::from(total)).clamp(0, 100) as u8;
    Some(ResourceTransferEvent::Progress {
        resource_id,
        present_percent,
    })
}

fn host_resource_type(resource_type: u8) -> Result<HostResourceType, ResourceTransferError> {
    match resource_type {
        1 => Ok(HostResourceType::Scenario),
        2 => Ok(HostResourceType::Dynamic),
        3 => Ok(HostResourceType::Player),
        4 => Ok(HostResourceType::Definitions),
        5 => Ok(HostResourceType::System),
        6 => Ok(HostResourceType::Material),
        other => Err(ResourceTransferError::UnsupportedResourceType(other)),
    }
}

fn is_discarded_store_error(error: &ResourceFileStoreError) -> bool {
    matches!(
        error,
        // These are the false returns from C4Network2ResChunk::AddTo: peer
        // bounds validation and local open/seek/write failures. OnChunk drops
        // all of them without changing logical chunk/load bookkeeping.
        ResourceFileStoreError::Io(_)
            | ResourceFileStoreError::ChunkOutOfRange { .. }
            | ResourceFileStoreError::ChunkExceedsFile { .. }
            | ResourceFileStoreError::ShortWrite { .. }
    )
}

/// `ResourceCatalog` intentionally keeps its eligible-set representation
/// private. Probe clones let this executor provide the exact bounded
/// `SafeRandom` input without changing catalog state or widening that API.
fn eligible_chunk_count(
    catalog: &ResourceCatalog,
    resource_id: i32,
    peer_id: i32,
    now_seconds: u64,
) -> usize {
    (0..)
        .take_while(|choice| {
            catalog
                .clone()
                .schedule_request(resource_id, peer_id, *choice, now_seconds)
                .is_some()
        })
        .count()
}
