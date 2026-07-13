//! Operational backend for the stock C4 network-resource state machine.
//!
//! `ResourceCatalog` deliberately describes filesystem and transport effects.
//! This type executes the filesystem effects, feeds successful writes back into
//! the catalog, and leaves socket delivery as typed events for its caller.

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::path::{Path, PathBuf};

use lc_engine::NetworkResourceCore;

use crate::{
    ChunkStoreOutcome, ChunkWriteOutcome, ResourceCatalog, ResourceCatalogAction,
    ResourceDataPacket, ResourceFileOwnership, ResourceFileStore, ResourceFileStoreError,
    ResourcePacket, ResourceRegistration,
};

/// Externally observable work produced after all local catalog actions run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceTransferEvent {
    /// A stock catalog transport action, preserved without translation.
    Transport(ResourceCatalogAction),
    /// The final chunk made a remotely loaded resource complete.
    Completed {
        resource_id: i32,
        core: NetworkResourceCore,
        path: PathBuf,
    },
    /// Discovery/load timeout removed the temporary resource file.
    LoadFailed { resource_id: i32 },
    /// Derivation needs the C4Group delta algorithm and is intentionally left
    /// explicit until that algorithm has a verified Rust implementation.
    FinishDerivedUnsupported { core: NetworkResourceCore },
}

#[derive(Debug)]
pub enum ResourceTransferError {
    FileStore(ResourceFileStoreError),
    DuplicateResource(i32),
    CatalogRegistrationRejected(i32),
    MissingCore(i32),
    MissingPath(i32),
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
            _ => None,
        }
    }
}

impl From<ResourceFileStoreError> for ResourceTransferError {
    fn from(error: ResourceFileStoreError) -> Self {
        Self::FileStore(error)
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

    /// Registers contents-identical local data whose standalone bytes differ
    /// from the official core. It remains locally usable, but its catalog entry
    /// is not binary-compatible and therefore never serves transfer chunks.
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

    pub fn core(&self, resource_id: i32) -> Option<&NetworkResourceCore> {
        self.cores.get(&resource_id)
    }

    pub fn path(&self, resource_id: i32) -> Option<&Path> {
        self.files
            .path(resource_id)
            .or_else(|| self.local_sources.get(&resource_id).map(PathBuf::as_path))
    }

    /// Marks resources in the departed client's ID namespace for the
    /// catalog's delayed removal, matching `C4Network2ResList::RemoveAtClient`.
    pub fn remove_at_client(&mut self, client_id: i32) -> usize {
        self.catalog.remove_at_client(client_id)
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
        let actions = self.catalog.on_timer(now_seconds);
        self.process_actions(actions, now_seconds, safe_random)
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
                    let data = self.files.read_chunk(resource_id, chunk)?;
                    pending.push_back(ResourceCatalogAction::SendToPeer {
                        peer_id,
                        packet: ResourcePacket::Data(ResourceDataPacket {
                            resource_id,
                            chunk,
                            data,
                        }),
                    });
                }
                ResourceCatalogAction::StoreChunk { packet, .. } => {
                    // AddTo writes first; OnChunk only records and schedules
                    // after that succeeds (src/C4Network2Res.cpp:911-940,
                    // 1263-1318).
                    let write_outcome =
                        self.files
                            .write_chunk(packet.resource_id, packet.chunk, &packet.data)?;
                    let chunk = i32::try_from(packet.chunk)
                        .map_err(|_| ResourceTransferError::ChunkIndexOverflow(packet.chunk))?;
                    let catalog_outcome =
                        self.catalog.record_chunk_stored(packet.resource_id, chunk);
                    match catalog_outcome {
                        ChunkStoreOutcome::Stored => {
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
                            pending.push_back(ResourceCatalogAction::RefillRequests {
                                resource_id: packet.resource_id,
                            });
                        }
                        ChunkStoreOutcome::Completed => {
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
                        outcome => {
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
                    pending.extend(self.catalog.refill_requests(
                        resource_id,
                        now_seconds,
                        &mut *safe_random,
                    ));
                }
                ResourceCatalogAction::FinishDerived { core } => {
                    events.push(ResourceTransferEvent::FinishDerivedUnsupported { core });
                }
                ResourceCatalogAction::ResourceLoadFailed { resource_id } => {
                    self.files.remove(resource_id)?;
                    self.cores.remove(&resource_id);
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
