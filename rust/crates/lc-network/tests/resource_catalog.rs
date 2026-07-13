#![allow(dead_code)] // Path-including the packet codec exposes helpers tested separately.

#[path = "../src/resource_catalog.rs"]
mod resource_catalog;
#[path = "../src/resource_packet.rs"]
mod resource_packet;

use lc_engine::NetworkResourceCore;
use resource_catalog::{
    ChunkSet, ChunkStoreOutcome, PeerStatusOutcome, ResourceCatalog, ResourceCatalogAction,
    ResourceLoadPoll, ResourceRegistration,
};
use resource_packet::{
    ResourceChunkAvailability, ResourceChunkRange, ResourceDiscoverPacket, ResourcePacket,
    ResourceRequestPacket, ResourceStatusPacket,
};

fn registration(resource_id: i32) -> ResourceRegistration {
    ResourceRegistration {
        resource_id,
        chunk_count: 1,
        binary_compatible: true,
        loading: false,
    }
}

#[test]
fn cpp_discovery_uses_reverse_registration_order_and_stops_at_fifteen_ids() {
    // C4Network2ResList::Add prepends each resource; SendDiscover traverses
    // pFirst and C4PacketResDiscover::AddDisID refuses the sixteenth ID
    // (src/C4Network2Res.cpp:1431-1441,1677-1699;
    // src/C4Network2IO.cpp:1745-1750).
    let mut catalog = ResourceCatalog::new(0);
    (0..20).for_each(|resource_id| {
        catalog.register(registration(resource_id));
    });

    assert_eq!(
        catalog.discovery_packet().resource_ids,
        (5..20).rev().collect::<Vec<_>>()
    );
}

#[test]
fn cpp_registration_reuses_an_existing_resource_id() {
    // AddByCore returns the existing list entry before allocating/inserting a
    // duplicate resource (src/C4Network2Res.cpp:1473-1477).
    let mut catalog = ResourceCatalog::new(0);
    assert!(catalog.register(registration(7)));
    assert!(!catalog.register(registration(7)));

    assert_eq!(catalog.discovery_packet().resource_ids, vec![7]);
}

#[test]
fn cpp_client_removal_marks_its_resource_namespace_removed() {
    // C4Network2ResList::RemoveAtClient marks every resource whose high-word
    // owner matches the departing client (src/C4Network2Res.cpp:1519-1525).
    let mut catalog = ResourceCatalog::new(0);
    catalog.register(registration(0x0002_0001));
    catalog.register(registration(0x0003_0001));

    assert_eq!(catalog.remove_at_client(2), 1);
    assert_eq!(catalog.discovery_packet().resource_ids, vec![0x0003_0001]);
}

#[test]
fn cpp_chunk_ranges_merge_overlap_and_adjacency_in_sorted_order() {
    // AddChunkRange inserts by start and MergeRanges joins overlapping or
    // adjacent ranges while subtracting overlap from iPresentChunkCnt
    // (src/C4Network2Res.cpp:204-231,274-292).
    let mut chunks = ChunkSet::incomplete(12);
    assert!(chunks.add_range(6, 2));
    assert!(chunks.add_range(2, 3));
    assert!(chunks.add_range(4, 3));
    assert!(chunks.add_range(3, 6));

    assert_eq!(chunks.present_chunk_count(), 7);
    assert_eq!(
        chunks.ranges(),
        &[ResourceChunkRange {
            start: 2,
            length: 7,
        }]
    );
}

#[test]
fn cpp_peer_status_resets_discovery_before_validating_chunk_count() {
    // OnStatus clears iDiscoverStartTime before rejecting a mismatched
    // ChunkCnt; a new peer is prepended and later updates retain its position
    // (src/C4Network2Res.cpp:886-908).
    let mut catalog = ResourceCatalog::new(0);
    catalog.register(ResourceRegistration {
        resource_id: 7,
        chunk_count: 8,
        binary_compatible: false,
        loading: true,
    });
    assert!(catalog.mark_discovery_needed(7, 100));

    let mismatch = ResourceStatusPacket {
        resource_id: 7,
        chunks: ResourceChunkAvailability {
            chunk_count: 9,
            ranges: Vec::new(),
        },
    };
    assert_eq!(
        catalog.record_peer_status(31, &mismatch),
        PeerStatusOutcome::ChunkCountMismatch
    );
    assert_eq!(catalog.discovery_started_at(7), None);

    let status = ResourceStatusPacket {
        resource_id: 7,
        chunks: ResourceChunkAvailability {
            chunk_count: 8,
            ranges: vec![ResourceChunkRange {
                start: 2,
                length: 3,
            }],
        },
    };
    assert_eq!(
        catalog.record_peer_status(31, &status),
        PeerStatusOutcome::Recorded
    );
    assert_eq!(catalog.peer_ids(7), vec![31]);
    assert_eq!(
        catalog.peer_chunks(7, 31).unwrap().ranges(),
        status.chunks.ranges
    );
}

#[test]
fn cpp_peer_connect_discovers_all_but_only_binary_compatible_resources_answer() {
    // OnClientConnect directs SendDiscover to the new peer. Discovery contains
    // all non-removed IDs, while HandlePacket answers only resources for which
    // IsBinaryCompatible succeeds (src/C4Network2Res.cpp:1540-1544,
    // 1557-1568,1677-1699).
    let mut catalog = ResourceCatalog::new(0);
    catalog.register(ResourceRegistration {
        resource_id: 10,
        chunk_count: 4,
        binary_compatible: true,
        loading: false,
    });
    catalog.register(ResourceRegistration {
        resource_id: 11,
        chunk_count: 2,
        binary_compatible: false,
        loading: false,
    });

    assert_eq!(
        catalog.on_peer_connected(5),
        vec![ResourceCatalogAction::SendToPeer {
            peer_id: 5,
            packet: ResourcePacket::Discover(ResourceDiscoverPacket {
                resource_ids: vec![11, 10],
            }),
        }]
    );

    assert_eq!(
        catalog.on_packet(
            5,
            &ResourcePacket::Discover(ResourceDiscoverPacket {
                resource_ids: vec![11, 10],
            }),
        ),
        vec![ResourceCatalogAction::SendToPeer {
            peer_id: 5,
            packet: ResourcePacket::Status(ResourceStatusPacket {
                resource_id: 10,
                chunks: ResourceChunkAvailability {
                    chunk_count: 4,
                    ranges: vec![ResourceChunkRange {
                        start: 0,
                        length: 4,
                    }],
                },
            }),
        }]
    );
}

#[test]
fn cpp_request_selection_excludes_inflight_chunks_and_preserves_limit_off_by_ones() {
    // GetChunkToRetrieve excludes local, unavailable, and already-loading
    // chunks, then indexes the sorted candidates with SafeRandom's result.
    // StartLoad counts existing requests before comparing `>= 3`, permitting
    // 3 requests per peer; its separate `iLoadCnt + 1 >= 20` guard permits
    // 19 total
    // (src/C4Network2Res.cpp:254-272,1066-1108; constants at
    // src/C4Network2Res.h:29-35).
    let mut catalog = ResourceCatalog::new(0);
    catalog.register(ResourceRegistration {
        resource_id: 70,
        chunk_count: 50,
        binary_compatible: false,
        loading: true,
    });
    for peer_id in 1..=10 {
        let status = ResourceStatusPacket {
            resource_id: 70,
            chunks: ResourceChunkAvailability {
                chunk_count: 50,
                ranges: vec![ResourceChunkRange {
                    start: 0,
                    length: 50,
                }],
            },
        };
        assert_eq!(
            catalog.record_peer_status(peer_id, &status),
            PeerStatusOutcome::Recorded
        );
    }

    assert_eq!(
        catalog.schedule_request(70, 1, 3, 100),
        Some(ResourceCatalogAction::SendToPeer {
            peer_id: 1,
            packet: ResourcePacket::Request(ResourceRequestPacket {
                resource_id: 70,
                chunk: 3,
            }),
        })
    );
    assert_eq!(
        catalog.schedule_request(70, 1, 0, 100),
        Some(ResourceCatalogAction::SendToPeer {
            peer_id: 1,
            packet: ResourcePacket::Request(ResourceRequestPacket {
                resource_id: 70,
                chunk: 0,
            }),
        })
    );
    assert!(catalog.schedule_request(70, 1, 0, 100).is_some());
    assert_eq!(catalog.schedule_request(70, 1, 0, 100), None);

    for peer_id in 2..=6 {
        assert!(catalog.schedule_request(70, peer_id, 0, 100).is_some());
        assert!(catalog.schedule_request(70, peer_id, 0, 100).is_some());
        assert!(catalog.schedule_request(70, peer_id, 0, 100).is_some());
    }
    assert!(catalog.schedule_request(70, 7, 0, 100).is_some());
    assert_eq!(catalog.outstanding_load_count(70), 19);
    assert_eq!(catalog.schedule_request(70, 7, 0, 100), None);
}

#[test]
fn cpp_load_and_discovery_timeouts_use_different_boundary_comparisons() {
    // C4Network2ResLoad expires at elapsed >= 60 seconds, but discovery fails
    // only at elapsed > 10 seconds and only when no loads were present at the
    // beginning of DoLoad (src/C4Network2Res.cpp:152-155,943-971).
    let mut catalog = ResourceCatalog::new(0);
    catalog.register(ResourceRegistration {
        resource_id: 80,
        chunk_count: 2,
        binary_compatible: false,
        loading: true,
    });
    let status = ResourceStatusPacket {
        resource_id: 80,
        chunks: ResourceChunkAvailability {
            chunk_count: 2,
            ranges: vec![ResourceChunkRange {
                start: 0,
                length: 2,
            }],
        },
    };
    catalog.record_peer_status(1, &status);
    assert!(catalog.mark_discovery_needed(80, 100));
    assert!(catalog.schedule_request(80, 1, 0, 100).is_some());

    assert_eq!(
        catalog.poll_load(80, 159),
        ResourceLoadPoll::Active { expired: 0 }
    );
    assert_eq!(
        catalog.poll_load(80, 160),
        ResourceLoadPoll::Active { expired: 1 }
    );
    assert_eq!(
        catalog.poll_load(80, 160),
        ResourceLoadPoll::DiscoverTimedOut
    );

    catalog.register(ResourceRegistration {
        resource_id: 81,
        chunk_count: 1,
        binary_compatible: false,
        loading: true,
    });
    assert!(catalog.mark_discovery_needed(81, 200));
    assert_eq!(
        catalog.poll_load(81, 210),
        ResourceLoadPoll::Active { expired: 0 }
    );
    assert_eq!(
        catalog.poll_load(81, 211),
        ResourceLoadPoll::DiscoverTimedOut
    );
}

#[test]
fn cpp_resource_ids_use_client_high_word_and_shift_when_local_id_changes() {
    // Init seeds iNextResID from clientID << 16; nextResID skips occupied IDs.
    // SetLocalID shifts both the counter and resources owned by the old client
    // (src/C4Network2Res.cpp:1345-1386).
    let mut catalog = ResourceCatalog::new(3);
    assert_eq!(catalog.allocate_resource_id(), 0x0003_0000);
    catalog.register(registration(0x0003_0001));
    catalog.register(registration(0x0009_0002));
    assert_eq!(catalog.allocate_resource_id(), 0x0003_0002);

    catalog.set_local_client_id(5);
    assert_eq!(catalog.local_client_id(), 5);
    assert_eq!(catalog.allocate_resource_id(), 0x0005_0003);
    assert_eq!(
        catalog.discovery_packet().resource_ids,
        vec![0x0009_0002, 0x0005_0001]
    );
}

#[test]
fn cpp_timer_broadcasts_discovery_each_second_and_dirty_status_once() {
    // OnTimer emits discovery when at least one non-removed resource exists
    // and >=1 second elapsed. Dirty resources also emit status, and SendStatus
    // clears the dirty flag before broadcasting (src/C4Network2Res.cpp:
    // 1621-1652; SendStatus at 831-845).
    let mut catalog = ResourceCatalog::new(0);
    catalog.register(ResourceRegistration {
        resource_id: 90,
        chunk_count: 2,
        binary_compatible: true,
        loading: false,
    });

    assert_eq!(
        catalog.on_timer(100),
        vec![
            ResourceCatalogAction::Broadcast {
                packet: ResourcePacket::Discover(ResourceDiscoverPacket {
                    resource_ids: vec![90],
                }),
            },
            ResourceCatalogAction::Broadcast {
                packet: ResourcePacket::Status(ResourceStatusPacket {
                    resource_id: 90,
                    chunks: ResourceChunkAvailability {
                        chunk_count: 2,
                        ranges: vec![ResourceChunkRange {
                            start: 0,
                            length: 2,
                        }],
                    },
                }),
            },
        ]
    );
    assert!(catalog.on_timer(100).is_empty());
    assert_eq!(
        catalog.on_timer(101),
        vec![ResourceCatalogAction::Broadcast {
            packet: ResourcePacket::Discover(ResourceDiscoverPacket {
                resource_ids: vec![90],
            }),
        }]
    );
}

#[test]
fn cpp_stored_chunk_clears_matching_loads_and_completion_clears_peer_state() {
    // A successful OnChunk marks status dirty, removes every request for that
    // chunk, and EndLoad clears peer/load/discovery state when all chunks are
    // present (src/C4Network2Res.cpp:911-940,1113-1131).
    let mut catalog = ResourceCatalog::new(0);
    catalog.register(ResourceRegistration {
        resource_id: 100,
        chunk_count: 2,
        binary_compatible: false,
        loading: true,
    });
    let status = ResourceStatusPacket {
        resource_id: 100,
        chunks: ResourceChunkAvailability {
            chunk_count: 2,
            ranges: vec![ResourceChunkRange {
                start: 0,
                length: 2,
            }],
        },
    };
    catalog.record_peer_status(1, &status);
    assert!(catalog.schedule_request(100, 1, 0, 10).is_some());
    assert_eq!(
        catalog.record_chunk_stored(100, 0),
        ChunkStoreOutcome::Stored
    );
    assert_eq!(catalog.outstanding_load_count(100), 0);
    assert_eq!(
        catalog.record_chunk_stored(100, 1),
        ChunkStoreOutcome::Completed
    );
    assert!(catalog.peer_ids(100).is_empty());
    assert_eq!(catalog.poll_load(100, 1000), ResourceLoadPoll::NotLoading);
}

#[test]
fn cpp_loading_status_prompts_an_immediate_request_from_that_peer() {
    // OnStatus stores valid availability and immediately calls StartLoad for
    // the reporting peer when the resource is loading
    // (src/C4Network2Res.cpp:886-909).
    let mut catalog = ResourceCatalog::new(0);
    catalog.register(ResourceRegistration {
        resource_id: 110,
        chunk_count: 3,
        binary_compatible: false,
        loading: true,
    });
    let packet = ResourcePacket::Status(ResourceStatusPacket {
        resource_id: 110,
        chunks: ResourceChunkAvailability {
            chunk_count: 3,
            ranges: vec![ResourceChunkRange {
                start: 0,
                length: 3,
            }],
        },
    });

    assert_eq!(
        catalog.on_packet(12, &packet),
        vec![ResourceCatalogAction::ScheduleFromPeer {
            resource_id: 110,
            peer_id: 12,
        }]
    );
}

#[test]
fn cpp_timer_refills_after_expiring_outstanding_requests() {
    // DoLoad calls StartNewLoads in the same timer pass when one or more
    // 60-second request timeouts were removed
    // (src/C4Network2Res.cpp:943-962).
    let mut catalog = ResourceCatalog::new(0);
    catalog.register(ResourceRegistration {
        resource_id: 120,
        chunk_count: 2,
        binary_compatible: false,
        loading: true,
    });
    let status = ResourceStatusPacket {
        resource_id: 120,
        chunks: ResourceChunkAvailability {
            chunk_count: 2,
            ranges: vec![ResourceChunkRange {
                start: 0,
                length: 2,
            }],
        },
    };
    catalog.record_peer_status(1, &status);
    assert!(catalog.schedule_request(120, 1, 0, 10).is_some());

    assert_eq!(
        catalog.on_timer(70),
        vec![
            ResourceCatalogAction::RefillRequests { resource_id: 120 },
            ResourceCatalogAction::Broadcast {
                packet: ResourcePacket::Discover(ResourceDiscoverPacket {
                    resource_ids: vec![120],
                }),
            },
        ]
    );
}

#[test]
fn cpp_registration_derives_chunk_count_from_core_ceiling_division() {
    // C4Network2ResCore::getChunkCnt uses
    // `(FileSize - 1) / ChunkSize + 1`, with zero for an empty file
    // (src/C4Network2Res.h:79-85).
    let empty = NetworkResourceCore {
        file_size: 0,
        chunk_size: 102_400,
        ..NetworkResourceCore::default()
    };
    assert_eq!(
        ResourceRegistration::from_core(&empty, true, false).chunk_count,
        0
    );

    let two_chunks = NetworkResourceCore {
        file_size: 102_401,
        chunk_size: 102_400,
        ..NetworkResourceCore::default()
    };
    assert_eq!(
        ResourceRegistration::from_core(&two_chunks, false, true).chunk_count,
        2
    );
}

#[test]
fn cpp_refill_shuffles_peers_then_fills_each_to_its_effective_limit() {
    // StartNewLoads shuffles the prepended ClientChunks list by choosing among
    // remaining slots, then repeatedly scans that fixed order. Because
    // StartLoad's peer guard allows three requests, so each peer receives three
    // before scanning advances (src/C4Network2Res.cpp:1017-1064,1066-1108).
    let mut catalog = ResourceCatalog::new(0);
    catalog.register(ResourceRegistration {
        resource_id: 130,
        chunk_count: 6,
        binary_compatible: false,
        loading: true,
    });
    let status = ResourceStatusPacket {
        resource_id: 130,
        chunks: ResourceChunkAvailability {
            chunk_count: 6,
            ranges: vec![ResourceChunkRange {
                start: 0,
                length: 6,
            }],
        },
    };
    for peer_id in 1..=3 {
        catalog.record_peer_status(peer_id, &status);
    }

    let mut draws = vec![1, 0, 0, 0, 0, 0, 0, 0, 0].into_iter();
    let actions = catalog.refill_requests(130, 50, |_| draws.next().unwrap());
    assert_eq!(draws.next(), None);
    let requests = actions
        .into_iter()
        .map(|action| match action {
            ResourceCatalogAction::SendToPeer {
                peer_id,
                packet: ResourcePacket::Request(request),
            } => (peer_id, request.chunk),
            action => panic!("unexpected action: {action:?}"),
        })
        .collect::<Vec<_>>();

    assert_eq!(
        requests,
        vec![(2, 0), (2, 1), (2, 2), (3, 3), (3, 4), (3, 5)]
    );
}
