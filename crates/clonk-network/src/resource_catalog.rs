//! Deterministic state for LegacyClonk's stock resource-transfer protocol.
//!
//! This module intentionally owns no sockets, files, clocks, or RNG. Callers
//! provide timestamps and the result of the C++ `SafeRandom` draw so identical
//! inputs produce identical state transitions.

use std::collections::BTreeSet;

use crate::resource_packet::{
    ResourceChunkAvailability, ResourceChunkRange, ResourceDataPacket, ResourceDiscoverPacket,
    ResourcePacket, ResourceStatusPacket,
};
use clonk_engine::NetworkResourceCore;

pub const RESOURCE_MAX_LOAD_PER_PEER_PER_FILE: usize = 3;
/// Concurrent chunk requests across all resources (C++ `C4NetResMaxLoad`).
///
/// Kept at C++'s 20 rather than OpenClonk's 5: the swarm behaviour here is
/// pinned by tests against C++, and the smaller `STOCK_CHUNK_SIZE` already cuts
/// the bulk that can sit ahead of control from 2 MB to 200 KB without diverging.
pub const RESOURCE_MAX_LOADS: usize = 20;
pub const RESOURCE_LOAD_TIMEOUT_SECONDS: u64 = 60;
pub const RESOURCE_DELETE_TIME_SECONDS: u64 = 60;
pub const RESOURCE_DISCOVER_TIMEOUT_SECONDS: u64 = 10;
pub const RESOURCE_DISCOVER_INTERVAL_SECONDS: u64 = 1;
pub const RESOURCE_STATUS_INTERVAL_SECONDS: u64 = 1;
pub const RESOURCE_ID_ANONYMOUS: i32 = -2;

/// Canonical form of `C4Network2ResChunkData`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkSet {
    chunk_count: i32,
    present_chunk_count: i32,
    ranges: Vec<ResourceChunkRange>,
}

impl ChunkSet {
    pub fn incomplete(chunk_count: i32) -> Self {
        Self {
            chunk_count,
            present_chunk_count: 0,
            ranges: Vec::new(),
        }
    }

    pub fn complete(chunk_count: i32) -> Self {
        let mut chunks = Self::incomplete(chunk_count);
        if chunk_count > 0 {
            let _ = chunks.add_range(0, chunk_count);
        }
        chunks
    }

    pub fn from_wire(chunks: &ResourceChunkAvailability) -> Self {
        let mut result = Self::incomplete(chunks.chunk_count);
        chunks.ranges.iter().for_each(|range| {
            let _ = result.add_range(range.start, range.length);
        });
        result
    }

    /// Silently rejects invalid ranges, as the C++ oracle does.
    pub fn add_range(&mut self, start: i32, length: i32) -> bool {
        let Some(end) = start.checked_add(length) else {
            return false;
        };
        if start < 0 || length <= 0 || end > self.chunk_count {
            return false;
        }

        let first_merge = self
            .ranges
            .iter()
            .position(|range| range.start.saturating_add(range.length) >= start)
            .unwrap_or(self.ranges.len());
        let mut merged_start = start;
        let mut merged_end = end;
        let mut last_merge = first_merge;
        while let Some(range) = self.ranges.get(last_merge) {
            if range.start > merged_end {
                break;
            }
            merged_start = merged_start.min(range.start);
            merged_end = merged_end.max(range.start.saturating_add(range.length));
            self.present_chunk_count -= range.length;
            last_merge += 1;
        }
        self.ranges.drain(first_merge..last_merge);

        let merged = ResourceChunkRange {
            start: merged_start,
            length: merged_end - merged_start,
        };
        self.present_chunk_count += merged.length;
        self.ranges.insert(first_merge, merged);
        true
    }

    pub fn chunk_count(&self) -> i32 {
        self.chunk_count
    }

    pub fn present_chunk_count(&self) -> i32 {
        self.present_chunk_count
    }

    pub fn is_complete(&self) -> bool {
        self.present_chunk_count == self.chunk_count
    }

    pub fn ranges(&self) -> &[ResourceChunkRange] {
        &self.ranges
    }

    pub fn contains(&self, chunk: i32) -> bool {
        self.ranges
            .iter()
            .any(|range| chunk >= range.start && chunk < range.start.saturating_add(range.length))
    }

    pub fn to_wire(&self) -> ResourceChunkAvailability {
        ResourceChunkAvailability {
            chunk_count: self.chunk_count,
            ranges: self.ranges.clone(),
        }
    }
}

/// Metadata needed by the protocol state machine for one registered resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceRegistration {
    pub resource_id: i32,
    pub chunk_count: i32,
    pub binary_compatible: bool,
    pub loading: bool,
}

impl ResourceRegistration {
    pub fn from_core(core: &NetworkResourceCore, binary_compatible: bool, loading: bool) -> Self {
        let chunk_count = if core.loadable && core.file_size != 0 && core.chunk_size != 0 {
            ((core.file_size - 1) / core.chunk_size + 1) as i32
        } else {
            0
        };
        Self {
            resource_id: core.id,
            chunk_count,
            binary_compatible,
            loading,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResourceState {
    registration: ResourceRegistration,
    core: Option<NetworkResourceCore>,
    removed: bool,
    last_request_at: Option<u64>,
    local_chunks: ChunkSet,
    peer_chunks: Vec<PeerChunks>,
    discovery_started_at: Option<u64>,
    outstanding_loads: Vec<ScheduledLoad>,
    dirty: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PeerChunks {
    peer_id: i32,
    chunks: ChunkSet,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutstandingLoad {
    pub chunk: i32,
    pub peer_id: i32,
    pub started_at: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ScheduledLoad {
    load: OutstandingLoad,
    refill_on_send_failure: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerStatusOutcome {
    UnknownResource,
    ChunkCountMismatch,
    Recorded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceLoadPoll {
    UnknownResource,
    NotLoading,
    Active { expired: usize },
    DiscoverTimedOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkStoreOutcome {
    UnknownResource,
    NotLoading,
    InvalidChunk,
    Stored,
    Completed,
}

/// Filesystem- and transport-independent effects produced by catalog input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceCatalogAction {
    Broadcast {
        packet: ResourcePacket,
    },
    SendToPeer {
        peer_id: i32,
        packet: ResourcePacket,
    },
    ServeChunk {
        peer_id: i32,
        resource_id: i32,
        chunk: u32,
    },
    StoreChunk {
        peer_id: i32,
        packet: ResourceDataPacket,
    },
    ScheduleFromPeer {
        resource_id: i32,
        peer_id: i32,
    },
    RefillRequests {
        resource_id: i32,
    },
    FinishDerived {
        core: NetworkResourceCore,
    },
    ResourceLoadFailed {
        resource_id: i32,
    },
}

/// In-memory counterpart of `C4Network2ResList`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceCatalog {
    local_client_id: i32,
    next_resource_id: i32,
    resources: Vec<ResourceState>,
    last_discover_at: Option<u64>,
    last_status_at: Option<u64>,
}

impl ResourceCatalog {
    pub fn new(local_client_id: i32) -> Self {
        Self {
            local_client_id,
            next_resource_id: local_client_id.wrapping_shl(16),
            resources: Vec::new(),
            last_discover_at: None,
            last_status_at: None,
        }
    }

    /// Registers at the front, matching `C4Network2ResList::Add`.
    pub fn register(&mut self, registration: ResourceRegistration) -> bool {
        self.register_with_timestamp(registration, None)
    }

    /// Registers with the wall-clock second assigned by C++ SetByFile,
    /// SetByGroup, or SetLoad.
    pub fn register_at(&mut self, registration: ResourceRegistration, now_seconds: u64) -> bool {
        self.register_with_timestamp(registration, Some(now_seconds))
    }

    fn register_with_timestamp(
        &mut self,
        registration: ResourceRegistration,
        last_request_at: Option<u64>,
    ) -> bool {
        if self.resource(registration.resource_id).is_some() {
            return false;
        }
        let local_chunks = if registration.loading {
            ChunkSet::incomplete(registration.chunk_count)
        } else if registration.binary_compatible {
            ChunkSet::complete(registration.chunk_count)
        } else {
            // A contents-identical local resource whose standalone differs
            // keeps its logical core, but C++ leaves Chunks cleared when
            // GetStandalone fails the size/CRC check.
            ChunkSet::incomplete(0)
        };
        self.resources.insert(
            0,
            ResourceState {
                registration,
                core: None,
                removed: false,
                last_request_at,
                local_chunks,
                peer_chunks: Vec::new(),
                discovery_started_at: None,
                outstanding_loads: Vec::new(),
                dirty: !registration.loading,
            },
        );
        true
    }

    /// Registers the catalog-visible state created by
    /// `C4Network2Res::SetDerived`. Anonymous IDs are intentionally not
    /// unique: C++ can retain several derived files awaiting announcements.
    pub fn register_anonymous_derived(
        &mut self,
        parent_resource_id: i32,
        binary_compatible: bool,
    ) -> bool {
        self.register_anonymous_derived_with_timestamp(parent_resource_id, binary_compatible, None)
    }

    pub fn register_anonymous_derived_at(
        &mut self,
        parent_resource_id: i32,
        binary_compatible: bool,
        now_seconds: u64,
    ) -> bool {
        self.register_anonymous_derived_with_timestamp(
            parent_resource_id,
            binary_compatible,
            Some(now_seconds),
        )
    }

    fn register_anonymous_derived_with_timestamp(
        &mut self,
        parent_resource_id: i32,
        binary_compatible: bool,
        last_request_at: Option<u64>,
    ) -> bool {
        let core = NetworkResourceCore {
            id: RESOURCE_ID_ANONYMOUS,
            derived_id: parent_resource_id,
            ..NetworkResourceCore::default()
        };
        self.resources.insert(
            0,
            ResourceState {
                registration: ResourceRegistration {
                    resource_id: RESOURCE_ID_ANONYMOUS,
                    chunk_count: 0,
                    binary_compatible,
                    loading: false,
                },
                core: Some(core),
                removed: false,
                last_request_at,
                local_chunks: ChunkSet::incomplete(0),
                peer_chunks: Vec::new(),
                discovery_started_at: None,
                outstanding_loads: Vec::new(),
                dirty: false,
            },
        );
        true
    }

    /// Builds the exact stock discovery set in linked-list traversal order.
    pub fn discovery_packet(&self) -> ResourceDiscoverPacket {
        let mut packet = ResourceDiscoverPacket {
            resource_ids: Vec::new(),
        };
        self.resources
            .iter()
            .filter(|resource| !resource.removed)
            .take_while(|resource| packet.add_resource_id(resource.registration.resource_id))
            .for_each(drop);
        packet
    }

    pub fn local_client_id(&self) -> i32 {
        self.local_client_id
    }

    pub fn set_local_client_id(&mut self, local_client_id: i32) {
        let old_client_id = self.local_client_id;
        let id_difference = local_client_id.wrapping_sub(old_client_id).wrapping_shl(16);
        self.local_client_id = local_client_id;
        self.next_resource_id = self.next_resource_id.wrapping_add(id_difference);
        self.resources
            .iter_mut()
            .filter(|resource| resource.registration.resource_id >> 16 == old_client_id)
            .for_each(|resource| {
                resource.registration.resource_id = resource
                    .registration
                    .resource_id
                    .wrapping_add(id_difference);
                if let Some(core) = &mut resource.core {
                    core.id = core.id.wrapping_add(id_difference);
                }
            });
    }

    pub fn allocate_resource_id(&mut self) -> i32 {
        let namespace_end = self
            .local_client_id
            .wrapping_add(1)
            .wrapping_shl(16)
            .wrapping_sub(1);
        if self.next_resource_id >= namespace_end {
            self.next_resource_id = self.local_client_id.max(0).wrapping_shl(16);
        }
        while self.resource(self.next_resource_id).is_some() {
            self.next_resource_id = self.next_resource_id.wrapping_add(1);
        }
        let allocated = self.next_resource_id;
        self.next_resource_id = self.next_resource_id.wrapping_add(1);
        allocated
    }

    /// Marks every resource owned by `client_id` for delayed removal.
    pub fn remove_at_client(&mut self, client_id: i32) -> usize {
        let mut removed = 0;
        self.resources
            .iter_mut()
            .filter(|resource| {
                !resource.removed && resource.registration.resource_id >> 16 == client_id
            })
            .for_each(|resource| {
                resource.removed = true;
                removed += 1;
            });
        removed
    }

    /// Schedules one retained resource for removal, matching
    /// `C4Network2Res::Remove`.
    pub fn remove_resource(&mut self, resource_id: i32) -> bool {
        self.resource_mut(resource_id)
            .map(|resource| resource.removed = true)
            .is_some()
    }

    /// Mirrors `C4Network2ResList::OnClientConnect` without owning a socket.
    pub fn on_peer_connected(&self, peer_id: i32) -> Vec<ResourceCatalogAction> {
        let packet = self.discovery_packet();
        (!packet.resource_ids.is_empty())
            .then_some(ResourceCatalogAction::SendToPeer {
                peer_id,
                packet: ResourcePacket::Discover(packet),
            })
            .into_iter()
            .collect()
    }

    /// Applies one stock resource packet and returns transport/storage work.
    pub fn on_packet(
        &mut self,
        peer_id: i32,
        packet: &ResourcePacket,
    ) -> Vec<ResourceCatalogAction> {
        self.on_packet_with_timestamp(peer_id, packet, None)
    }

    /// Applies a packet with the wall-clock second used for C++
    /// `iLastReqTime` bookkeeping.
    pub fn on_packet_at(
        &mut self,
        peer_id: i32,
        packet: &ResourcePacket,
        now_seconds: u64,
    ) -> Vec<ResourceCatalogAction> {
        self.on_packet_with_timestamp(peer_id, packet, Some(now_seconds))
    }

    fn on_packet_with_timestamp(
        &mut self,
        peer_id: i32,
        packet: &ResourcePacket,
        now_seconds: Option<u64>,
    ) -> Vec<ResourceCatalogAction> {
        match packet {
            ResourcePacket::Discover(discover) => self
                .resources
                .iter_mut()
                .filter(|resource| {
                    resource.registration.binary_compatible
                        && discover
                            .resource_ids
                            .contains(&resource.registration.resource_id)
                })
                .map(|resource| {
                    if let Some(now_seconds) = now_seconds {
                        resource.last_request_at = Some(now_seconds);
                    }
                    ResourceCatalogAction::SendToPeer {
                        peer_id,
                        packet: ResourcePacket::Status(ResourceStatusPacket {
                            resource_id: resource.registration.resource_id,
                            chunks: resource.local_chunks.to_wire(),
                        }),
                    }
                })
                .collect(),
            ResourcePacket::Status(status) => {
                let recorded = self.record_peer_status(peer_id, status);
                (recorded == PeerStatusOutcome::Recorded
                    && self
                        .resource(status.resource_id)
                        .is_some_and(|resource| resource.registration.loading))
                .then_some(ResourceCatalogAction::ScheduleFromPeer {
                    resource_id: status.resource_id,
                    peer_id,
                })
                .into_iter()
                .collect()
            }
            ResourcePacket::Derive(core) => self.finish_matching_anonymous_derived(core, false),
            ResourcePacket::Request(request) => self
                .resource_mut(request.resource_id)
                .filter(|resource| {
                    resource.registration.binary_compatible
                        && request.chunk >= 0
                        && request.chunk < resource.registration.chunk_count
                })
                .map(|resource| {
                    if let Some(now_seconds) = now_seconds {
                        resource.last_request_at = Some(now_seconds);
                    }
                    ResourceCatalogAction::ServeChunk {
                        peer_id,
                        resource_id: request.resource_id,
                        chunk: request.chunk as u32,
                    }
                })
                .into_iter()
                .collect(),
            ResourcePacket::Data(data) => self
                .resource(data.resource_id)
                .filter(|resource| resource.registration.loading)
                .map(|_| ResourceCatalogAction::StoreChunk {
                    peer_id,
                    packet: data.clone(),
                })
                .into_iter()
                .collect(),
        }
    }

    /// Finishes a locally created derived resource and announces its new core.
    /// The announcement is emitted only when a matching anonymous resource was
    /// rebound, mirroring `C4Network2Res::FinishDerive`.
    pub fn finish_local_derived(
        &mut self,
        core: &NetworkResourceCore,
    ) -> Vec<ResourceCatalogAction> {
        let mut actions = self.finish_matching_anonymous_derived(core, true);
        if !actions.is_empty() {
            actions.push(ResourceCatalogAction::Broadcast {
                packet: ResourcePacket::Derive(core.clone()),
            });
        }
        actions
    }

    fn finish_matching_anonymous_derived(
        &mut self,
        core: &NetworkResourceCore,
        dirty: bool,
    ) -> Vec<ResourceCatalogAction> {
        if core.derived_id < 0 {
            return Vec::new();
        }
        self.resources
            .iter_mut()
            .filter(|resource| {
                resource.registration.resource_id == RESOURCE_ID_ANONYMOUS
                    && resource
                        .core
                        .as_ref()
                        .is_some_and(|anonymous| anonymous.derived_id == core.derived_id)
            })
            .map(|resource| {
                resource.registration = ResourceRegistration::from_core(core, true, false);
                resource.core = Some(core.clone());
                resource.local_chunks = ChunkSet::complete(resource.registration.chunk_count);
                resource.dirty = dirty;
                ResourceCatalogAction::FinishDerived { core: core.clone() }
            })
            .collect()
    }

    /// Produces the periodic protocol work from `C4Network2ResList::OnTimer`.
    pub fn on_timer(&mut self, now_seconds: u64) -> Vec<ResourceCatalogAction> {
        self.on_timer_with_expired_resource_ids(now_seconds).0
    }

    /// Produces periodic protocol work and reports entries unlinked by
    /// `C4Network2ResList::OnShareFree` after the timer releases the list.
    /// Filesystem-owning callers use the IDs to run `C4Network2Res::Clear`;
    /// pure protocol callers can continue using [`Self::on_timer`].
    pub fn on_timer_with_expired_resource_ids(
        &mut self,
        now_seconds: u64,
    ) -> (Vec<ResourceCatalogAction>, Vec<i32>) {
        let mut actions = Vec::new();
        self.resources
            .iter_mut()
            .filter(|resource| resource.last_request_at.is_none())
            .for_each(|resource| resource.last_request_at = Some(now_seconds));
        self.resources.iter_mut().for_each(|resource| {
            if !resource.registration.loading || resource.removed {
                return;
            }
            if !resource.outstanding_loads.is_empty() {
                let previous_count = resource.outstanding_loads.len();
                resource.outstanding_loads.retain(|load| {
                    now_seconds.saturating_sub(load.load.started_at) < RESOURCE_LOAD_TIMEOUT_SECONDS
                });
                if resource.outstanding_loads.len() < previous_count {
                    actions.push(ResourceCatalogAction::RefillRequests {
                        resource_id: resource.registration.resource_id,
                    });
                }
            } else if resource.discovery_started_at.is_some_and(|started_at| {
                now_seconds.saturating_sub(started_at) > RESOURCE_DISCOVER_TIMEOUT_SECONDS
            }) {
                resource.removed = true;
                actions.push(ResourceCatalogAction::ResourceLoadFailed {
                    resource_id: resource.registration.resource_id,
                });
            }
        });

        let discovery_due = self.last_discover_at.is_none_or(|last| {
            now_seconds.saturating_sub(last) >= RESOURCE_DISCOVER_INTERVAL_SECONDS
        });
        if discovery_due {
            self.resources
                .iter_mut()
                .filter(|resource| !resource.removed)
                .for_each(|resource| {
                    resource.discovery_started_at.get_or_insert(now_seconds);
                });
            let packet = self.discovery_packet();
            if !packet.resource_ids.is_empty() {
                self.last_discover_at = Some(now_seconds);
                actions.push(ResourceCatalogAction::Broadcast {
                    packet: ResourcePacket::Discover(packet),
                });
            }
        }

        let status_due = self.last_status_at.is_none_or(|last| {
            now_seconds.saturating_sub(last) >= RESOURCE_STATUS_INTERVAL_SECONDS
        });
        if status_due {
            let mut sent_status = false;
            self.resources
                .iter_mut()
                .filter(|resource| resource.dirty && !resource.removed)
                .for_each(|resource| {
                    resource.dirty = false;
                    sent_status = true;
                    actions.push(ResourceCatalogAction::Broadcast {
                        packet: ResourcePacket::Status(ResourceStatusPacket {
                            resource_id: resource.registration.resource_id,
                            chunks: resource.local_chunks.to_wire(),
                        }),
                    });
                });
            self.last_status_at = sent_status.then_some(now_seconds);
        }
        let mut expired_resource_ids = Vec::new();
        self.resources.retain(|resource| {
            let retained = !resource.removed
                || resource.last_request_at.is_some_and(|last_request_at| {
                    last_request_at != 0
                        && now_seconds.saturating_sub(last_request_at)
                            <= RESOURCE_DELETE_TIME_SECONDS
                });
            if !retained {
                expired_resource_ids.push(resource.registration.resource_id);
            }
            retained
        });
        (actions, expired_resource_ids)
    }

    pub fn mark_discovery_needed(&mut self, resource_id: i32, now_seconds: u64) -> bool {
        self.resource_mut(resource_id).is_some_and(|resource| {
            resource.discovery_started_at.get_or_insert(now_seconds);
            true
        })
    }

    pub fn discovery_started_at(&self, resource_id: i32) -> Option<u64> {
        self.resource(resource_id)
            .and_then(|resource| resource.discovery_started_at)
    }

    pub fn record_peer_status(
        &mut self,
        peer_id: i32,
        status: &crate::resource_packet::ResourceStatusPacket,
    ) -> PeerStatusOutcome {
        let Some(resource) = self.resource_mut(status.resource_id) else {
            return PeerStatusOutcome::UnknownResource;
        };
        // The C++ order is observable: even a malformed status prevents the
        // current ten-second discovery attempt from timing out.
        resource.discovery_started_at = None;
        if status.chunks.chunk_count != resource.local_chunks.chunk_count() {
            return PeerStatusOutcome::ChunkCountMismatch;
        }

        let chunks = ChunkSet::from_wire(&status.chunks);
        if let Some(peer) = resource
            .peer_chunks
            .iter_mut()
            .find(|peer| peer.peer_id == peer_id)
        {
            peer.chunks = chunks;
        } else {
            resource
                .peer_chunks
                .insert(0, PeerChunks { peer_id, chunks });
        }
        PeerStatusOutcome::Recorded
    }

    pub fn peer_ids(&self, resource_id: i32) -> Vec<i32> {
        self.resource(resource_id)
            .map(|resource| {
                resource
                    .peer_chunks
                    .iter()
                    .map(|peer| peer.peer_id)
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn peer_chunks(&self, resource_id: i32, peer_id: i32) -> Option<&ChunkSet> {
        self.resource(resource_id).and_then(|resource| {
            resource
                .peer_chunks
                .iter()
                .find(|peer| peer.peer_id == peer_id)
                .map(|peer| &peer.chunks)
        })
    }

    /// Returns one peer's chunk-weighted progress across every non-removed
    /// resource for which that peer has reported status. With no reported
    /// chunks, C++ treats the peer as complete.
    pub fn client_progress(&self, peer_id: i32) -> u8 {
        let (present_chunks, total_chunks) = self
            .resources
            .iter()
            .filter(|resource| !resource.removed)
            .filter_map(|resource| {
                resource
                    .peer_chunks
                    .iter()
                    .find(|peer| peer.peer_id == peer_id)
                    .map(|peer| &peer.chunks)
            })
            .fold((0_i64, 0_i64), |(present, total), chunks| {
                (
                    present + i64::from(chunks.present_chunk_count()),
                    total + i64::from(chunks.chunk_count()),
                )
            });

        if total_chunks == 0 {
            100
        } else {
            (present_chunks * 100 / total_chunks).clamp(0, 100) as u8
        }
    }

    /// Starts one request using the already-bounded result of C++
    /// `SafeRandom(eligible_chunk_count)` as `random_choice`.
    pub fn schedule_request(
        &mut self,
        resource_id: i32,
        peer_id: i32,
        random_choice: usize,
        now_seconds: u64,
    ) -> Option<ResourceCatalogAction> {
        self.schedule_request_with_failure_policy(
            resource_id,
            peer_id,
            random_choice,
            now_seconds,
            false,
        )
    }

    fn schedule_request_with_failure_policy(
        &mut self,
        resource_id: i32,
        peer_id: i32,
        random_choice: usize,
        now_seconds: u64,
        refill_on_send_failure: bool,
    ) -> Option<ResourceCatalogAction> {
        let resource = self.resource_mut(resource_id)?;
        if !resource.registration.loading
            || resource.outstanding_loads.len() + 1 >= RESOURCE_MAX_LOADS
        {
            return None;
        }
        let peer_load_count = resource
            .outstanding_loads
            .iter()
            .filter(|load| load.load.peer_id == peer_id)
            .count();
        if peer_load_count >= RESOURCE_MAX_LOAD_PER_PEER_PER_FILE {
            return None;
        }
        let available = resource
            .peer_chunks
            .iter()
            .find(|peer| peer.peer_id == peer_id)
            .map(|peer| &peer.chunks)?;
        let chunk = (0..resource.registration.chunk_count)
            .filter(|chunk| {
                available.contains(*chunk)
                    && !resource.local_chunks.contains(*chunk)
                    && !resource
                        .outstanding_loads
                        .iter()
                        .any(|load| load.load.chunk == *chunk)
            })
            .nth(random_choice)?;

        resource.outstanding_loads.insert(
            0,
            ScheduledLoad {
                load: OutstandingLoad {
                    chunk,
                    peer_id,
                    started_at: now_seconds,
                },
                refill_on_send_failure,
            },
        );
        Some(ResourceCatalogAction::SendToPeer {
            peer_id,
            packet: ResourcePacket::Request(crate::resource_packet::ResourceRequestPacket {
                resource_id,
                chunk,
            }),
        })
    }

    /// Mirrors `StartNewLoads`; the callback supplies each bounded
    /// `SafeRandom(range)` result, including the initial peer shuffle draws.
    pub fn refill_requests(
        &mut self,
        resource_id: i32,
        now_seconds: u64,
        safe_random: impl FnMut(usize) -> usize,
    ) -> Vec<ResourceCatalogAction> {
        self.refill_requests_excluding(resource_id, now_seconds, &BTreeSet::new(), safe_random)
    }

    /// Continues `StartNewLoads` after transport failures. C++ nulls failed
    /// peers in its local shuffled array for the remainder of that pass while
    /// retaining their advertised chunks for a future pass.
    pub fn refill_requests_excluding(
        &mut self,
        resource_id: i32,
        now_seconds: u64,
        unavailable_peers: &BTreeSet<i32>,
        mut safe_random: impl FnMut(usize) -> usize,
    ) -> Vec<ResourceCatalogAction> {
        let peer_ids = self
            .resource(resource_id)
            .map(|resource| {
                resource
                    .peer_chunks
                    .iter()
                    .filter(|peer| !unavailable_peers.contains(&peer.peer_id))
                    .map(|peer| peer.peer_id)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut shuffled = vec![None; peer_ids.len()];
        let peer_count = peer_ids.len();
        peer_ids
            .into_iter()
            .enumerate()
            .for_each(|(index, peer_id)| {
                let remaining = peer_count - index;
                let mut position = safe_random(remaining) % remaining;
                for slot in &mut shuffled {
                    if slot.is_none() {
                        if position == 0 {
                            *slot = Some(peer_id);
                            break;
                        }
                        position -= 1;
                    }
                }
            });

        let mut actions = Vec::new();
        while self.outstanding_load_count(resource_id) < RESOURCE_MAX_LOADS {
            let previous_count = self.outstanding_load_count(resource_id);
            for peer_id in shuffled.iter().flatten().copied() {
                let eligible_count = self.eligible_chunk_count(resource_id, peer_id);
                if eligible_count != 0 {
                    let choice = safe_random(eligible_count) % eligible_count;
                    if let Some(action) = self.schedule_request_with_failure_policy(
                        resource_id,
                        peer_id,
                        choice,
                        now_seconds,
                        true,
                    ) {
                        actions.push(action);
                    }
                }
                if self.outstanding_load_count(resource_id) > previous_count {
                    break;
                }
            }
            if self.outstanding_load_count(resource_id) == previous_count {
                break;
            }
        }
        actions
    }

    /// Rolls back the exact request reservation when its transport send
    /// failed. Requests created by `StartNewLoads` immediately continue that
    /// pass using the remaining peers; the one-shot `OnStatus` request does
    /// not start a refill pass in native code.
    pub fn on_request_send_failed(
        &mut self,
        peer_id: i32,
        request: &crate::resource_packet::ResourceRequestPacket,
        now_seconds: u64,
        unavailable_peers: &BTreeSet<i32>,
        safe_random: impl FnMut(usize) -> usize,
    ) -> Vec<ResourceCatalogAction> {
        let refill =
            {
                let Some(resource) = self.resource_mut(request.resource_id) else {
                    return Vec::new();
                };
                let Some(position) = resource.outstanding_loads.iter().position(|load| {
                    load.load.peer_id == peer_id && load.load.chunk == request.chunk
                }) else {
                    return Vec::new();
                };
                resource
                    .outstanding_loads
                    .remove(position)
                    .refill_on_send_failure
            };
        if !refill {
            return Vec::new();
        }
        self.refill_requests_excluding(
            request.resource_id,
            now_seconds,
            unavailable_peers,
            safe_random,
        )
    }

    pub fn outstanding_load_count(&self, resource_id: i32) -> usize {
        self.resource(resource_id)
            .map(|resource| resource.outstanding_loads.len())
            .unwrap_or(0)
    }

    fn eligible_chunk_count(&self, resource_id: i32, peer_id: i32) -> usize {
        let Some(resource) = self.resource(resource_id) else {
            return 0;
        };
        if !resource.registration.loading
            || resource.outstanding_loads.len() + 1 >= RESOURCE_MAX_LOADS
            || resource
                .outstanding_loads
                .iter()
                .filter(|load| load.load.peer_id == peer_id)
                .count()
                >= RESOURCE_MAX_LOAD_PER_PEER_PER_FILE
        {
            return 0;
        }
        let Some(available) = resource
            .peer_chunks
            .iter()
            .find(|peer| peer.peer_id == peer_id)
            .map(|peer| &peer.chunks)
        else {
            return 0;
        };
        (0..resource.registration.chunk_count)
            .filter(|chunk| {
                available.contains(*chunk)
                    && !resource.local_chunks.contains(*chunk)
                    && !resource
                        .outstanding_loads
                        .iter()
                        .any(|load| load.load.chunk == *chunk)
            })
            .count()
    }

    /// Mirrors the timeout portion of `C4Network2Res::DoLoad`. Request
    /// replacement remains an explicit scheduling step because C++ shuffles
    /// peers and chunks with `SafeRandom`.
    pub fn poll_load(&mut self, resource_id: i32, now_seconds: u64) -> ResourceLoadPoll {
        let Some(resource) = self.resource_mut(resource_id) else {
            return ResourceLoadPoll::UnknownResource;
        };
        if !resource.registration.loading {
            return ResourceLoadPoll::NotLoading;
        }
        if !resource.outstanding_loads.is_empty() {
            let previous_count = resource.outstanding_loads.len();
            resource.outstanding_loads.retain(|load| {
                now_seconds.saturating_sub(load.load.started_at) < RESOURCE_LOAD_TIMEOUT_SECONDS
            });
            return ResourceLoadPoll::Active {
                expired: previous_count - resource.outstanding_loads.len(),
            };
        }
        if resource.discovery_started_at.is_some_and(|started_at| {
            now_seconds.saturating_sub(started_at) > RESOURCE_DISCOVER_TIMEOUT_SECONDS
        }) {
            ResourceLoadPoll::DiscoverTimedOut
        } else {
            ResourceLoadPoll::Active { expired: 0 }
        }
    }

    /// Records a chunk only after the external file backend has written it.
    pub fn record_chunk_stored(&mut self, resource_id: i32, chunk: i32) -> ChunkStoreOutcome {
        let Some(resource) = self.resource_mut(resource_id) else {
            return ChunkStoreOutcome::UnknownResource;
        };
        if !resource.registration.loading {
            return ChunkStoreOutcome::NotLoading;
        }
        if !resource.local_chunks.add_range(chunk, 1) {
            return ChunkStoreOutcome::InvalidChunk;
        }
        resource.dirty = true;
        resource
            .outstanding_loads
            .retain(|load| load.load.chunk != chunk);
        if !resource.local_chunks.is_complete() {
            return ChunkStoreOutcome::Stored;
        }

        resource.registration.loading = false;
        resource.peer_chunks.clear();
        resource.outstanding_loads.clear();
        resource.discovery_started_at = None;
        ChunkStoreOutcome::Completed
    }

    pub fn resource_core(&self, resource_id: i32) -> Option<&NetworkResourceCore> {
        self.resource(resource_id)
            .and_then(|resource| resource.core.as_ref())
    }

    pub fn local_chunks(&self, resource_id: i32) -> Option<&ChunkSet> {
        self.resource(resource_id)
            .map(|resource| &resource.local_chunks)
    }

    pub fn last_request_at(&self, resource_id: i32) -> Option<u64> {
        self.resource(resource_id)
            .and_then(|resource| resource.last_request_at)
    }

    pub fn contains_resource(&self, resource_id: i32) -> bool {
        self.resource(resource_id).is_some()
    }

    fn resource(&self, resource_id: i32) -> Option<&ResourceState> {
        self.resources
            .iter()
            .find(|resource| resource.registration.resource_id == resource_id)
    }

    fn resource_mut(&mut self, resource_id: i32) -> Option<&mut ResourceState> {
        self.resources
            .iter_mut()
            .find(|resource| resource.registration.resource_id == resource_id)
    }
}
