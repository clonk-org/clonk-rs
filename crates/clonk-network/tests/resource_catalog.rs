#![allow(dead_code)] // Path-including the packet codec exposes helpers tested separately.

use crate::resource_catalog::{
    ChunkSet, ChunkStoreOutcome, PeerStatusOutcome, ResourceCatalog, ResourceCatalogAction,
    ResourceLoadPoll, ResourceRegistration, RESOURCE_MAX_LOAD_PER_PEER_IN_GAME,
    RESOURCE_MAX_LOAD_PER_PEER_PER_FILE,
};
use crate::resource_packet::{
    encode_resource_packet, ResourceChunkAvailability, ResourceChunkRange, ResourceDiscoverPacket,
    ResourcePacket, ResourceRequestPacket, ResourceStatusPacket,
};
use clonk_engine::NetworkResourceCore;

fn registration(resource_id: i32) -> ResourceRegistration {
    ResourceRegistration {
        resource_id,
        chunk_count: 1,
        binary_compatible: true,
        loading: false,
    }
}

#[test]
fn cpp_derive_only_finishes_anonymous_resources_with_the_matching_parent() {
    // HandlePacket only calls FinishDerive for anonymous resources whose
    // parent DerID matches. FinishDerive replaces the full core (and thus ID)
    // and marks every chunk present (src/C4Network2Res.cpp:806-822,1584-1593).
    let mut catalog = ResourceCatalog::new(0);
    assert!(catalog.register_anonymous_derived_at(4, false, 100));
    assert!(catalog.register_anonymous_derived(9, false));
    let derived = NetworkResourceCore {
        id: 11,
        derived_id: 4,
        loadable: true,
        file_size: 205,
        chunk_size: 100,
        ..NetworkResourceCore::default()
    };

    assert!(catalog
        .on_packet(
            7,
            &ResourcePacket::Derive(NetworkResourceCore {
                derived_id: 8,
                ..derived.clone()
            }),
        )
        .is_empty());
    assert_eq!(
        catalog.on_packet(7, &ResourcePacket::Derive(derived.clone())),
        vec![ResourceCatalogAction::FinishDerived {
            core: derived.clone(),
        }]
    );
    assert_eq!(catalog.resource_core(11), Some(&derived));
    assert_eq!(catalog.local_chunks(11), Some(&ChunkSet::complete(3)));
    assert_eq!(catalog.last_request_at(11), Some(100));
    assert_eq!(
        catalog.on_packet(
            7,
            &ResourcePacket::Discover(ResourceDiscoverPacket {
                resource_ids: vec![11],
            }),
        ),
        vec![ResourceCatalogAction::SendToPeer {
            peer_id: 7,
            packet: ResourcePacket::Status(ResourceStatusPacket {
                resource_id: 11,
                chunks: ResourceChunkAvailability {
                    chunk_count: 3,
                    ranges: vec![ResourceChunkRange {
                        start: 0,
                        length: 3,
                    }],
                },
            }),
        }]
    );

    // The matching anonymous entry is no longer anonymous; the unmatched one
    // remains eligible for its own parent's announcement.
    assert!(catalog
        .on_packet(7, &ResourcePacket::Derive(derived.clone()))
        .is_empty());
    assert_eq!(
        catalog
            .on_packet(
                7,
                &ResourcePacket::Derive(NetworkResourceCore {
                    id: 12,
                    derived_id: 9,
                    ..derived
                }),
            )
            .len(),
        1
    );
}

#[test]
fn local_finish_derive_rebinds_then_broadcasts_the_new_core() {
    let mut catalog = ResourceCatalog::new(0);
    assert!(catalog.register_anonymous_derived(4, false));
    let derived = NetworkResourceCore {
        id: 11,
        derived_id: 4,
        loadable: true,
        file_size: 205,
        chunk_size: 100,
        ..NetworkResourceCore::default()
    };

    assert_eq!(
        catalog.finish_local_derived(&derived),
        vec![
            ResourceCatalogAction::FinishDerived {
                core: derived.clone(),
            },
            ResourceCatalogAction::Broadcast {
                packet: ResourcePacket::Derive(derived.clone()),
            },
        ]
    );
    assert_eq!(catalog.resource_core(11), Some(&derived));
    assert_eq!(catalog.local_chunks(11), Some(&ChunkSet::complete(3)));

    let unmatched = NetworkResourceCore {
        id: 12,
        derived_id: 8,
        ..derived
    };
    assert!(catalog.finish_local_derived(&unmatched).is_empty());
}

#[test]
fn local_finish_derive_broadcast_matches_the_cpp_codec_fixture() {
    let mut catalog = ResourceCatalog::new(0);
    assert!(catalog.register_anonymous_derived(0x0102_0304, true));
    let core = NetworkResourceCore {
        resource_type: 2,
        id: -1,
        derived_id: 0x0102_0304,
        loadable: false,
        contents_crc: 0x1122_3344,
        filename: crate::c4(b"Scenario.c4s"),
        author: crate::c4(b"Alice"),
        ..NetworkResourceCore::default()
    };
    let actions = catalog.finish_local_derived(&core);
    let ResourceCatalogAction::Broadcast { packet } = &actions[1] else {
        panic!("local FinishDerive did not emit its derive broadcast");
    };
    let expected = [
        0x32, 0x02, 0xff, 0xff, 0xff, 0xff, 0x04, 0x03, 0x02, 0x01, 0x00, 0x44, 0x33, 0x22, 0x11,
        0x00, b'S', b'c', b'e', b'n', b'a', b'r', b'i', b'o', b'.', b'c', b'4', b's', 0x00, b'A',
        b'l', b'i', b'c', b'e', 0x00,
    ];

    assert_eq!(encode_resource_packet(packet).unwrap(), expected);
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
fn periodic_discovery_eventually_advertises_every_loading_resource() {
    // C++ starts each SendDiscover at pFirst, so its fixed 15-ID packet omits
    // the same older resources forever (src/C4Network2Res.cpp:1677-1699;
    // src/C4Network2IO.cpp:1745-1750). Repeated Rust broadcasts keep the packet
    // cap and traversal order but must advance to the resources left behind.
    let mut catalog = ResourceCatalog::new(0);
    (0..16).for_each(|resource_id| {
        assert!(catalog.register(ResourceRegistration {
            resource_id,
            chunk_count: 1,
            binary_compatible: false,
            loading: true,
        }));
    });

    let packets = [100, 101]
        .into_iter()
        .flat_map(|now_seconds| catalog.on_timer(now_seconds))
        .filter_map(|action| match action {
            ResourceCatalogAction::Broadcast {
                packet: ResourcePacket::Discover(discover),
            } => Some(discover.resource_ids),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(packets, vec![(1..16).rev().collect::<Vec<_>>(), vec![0]]);
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
fn cpp_non_binary_complete_registration_keeps_chunk_data_cleared() {
    // A contents-only SetByCore match keeps the official core, but a failed
    // GetStandalone leaves C4Network2Res::Chunks in its cleared zero-count
    // state (src/C4Network2Res.cpp:441-458,668-697).
    let mut catalog = ResourceCatalog::new(0);
    assert!(catalog.register(ResourceRegistration {
        resource_id: 7,
        chunk_count: 3,
        binary_compatible: false,
        loading: false,
    }));

    assert_eq!(catalog.local_chunks(7), Some(&ChunkSet::incomplete(0)));
}

#[test]
fn cpp_real_resource_discards_zero_chunk_logical_status() {
    // OnStatus rejects a sender's cleared logical-only ChunkCnt=0 against the
    // real resource's complete N-chunk state before recording that sender as
    // a source (src/C4Network2Res.cpp:886-909).
    let mut catalog = ResourceCatalog::new(0);
    assert!(catalog.register(ResourceRegistration {
        resource_id: 7,
        chunk_count: 3,
        binary_compatible: true,
        loading: false,
    }));
    let status = ResourceStatusPacket {
        resource_id: 7,
        chunks: ResourceChunkAvailability {
            chunk_count: 0,
            ranges: Vec::new(),
        },
    };

    assert_eq!(
        catalog.record_peer_status(31, &status),
        PeerStatusOutcome::ChunkCountMismatch
    );
    assert!(catalog.peer_ids(7).is_empty());
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
fn cpp_removed_resources_are_pruned_only_after_strictly_more_than_sixty_seconds() {
    // Remove only sets fRemoved and retains iLastReqTime. OnShareFree unlinks
    // the entry when elapsed time is strictly greater than 60 seconds
    // (src/C4Network2Res.cpp:825-829,1655-1673).
    let mut catalog = ResourceCatalog::new(0);
    assert!(catalog.register_at(registration(0x0002_0001), 100));
    assert_eq!(catalog.remove_at_client(2), 1);
    assert_eq!(catalog.last_request_at(0x0002_0001), Some(100));

    let (actions, expired) = catalog.on_timer_with_expired_resource_ids(160);
    assert!(actions.is_empty());
    assert!(expired.is_empty());
    assert!(catalog.contains_resource(0x0002_0001));
    let (actions, expired) = catalog.on_timer_with_expired_resource_ids(161);
    assert!(actions.is_empty());
    assert_eq!(expired, vec![0x0002_0001]);
    assert!(!catalog.contains_resource(0x0002_0001));
}

#[test]
fn cpp_zero_last_request_time_allows_immediate_pruning() {
    // Clear deliberately assigns iLastReqTime = 0 before releasing the shared
    // list lock, and OnShareFree treats zero as immediately deletable
    // (src/C4Network2Res.cpp:1528-1535,1655-1673).
    let resource_id = 0x0002_0001;
    let mut catalog = ResourceCatalog::new(0);
    assert!(catalog.register_at(registration(resource_id), 0));
    assert_eq!(catalog.remove_at_client(2), 1);

    catalog.on_timer(0);
    assert!(!catalog.contains_resource(resource_id));
}

#[test]
fn cpp_successful_discovery_refreshes_removed_resource_retention() {
    // OnDiscover refreshes iLastReqTime only after IsBinaryCompatible has
    // accepted the resource; removed entries are still handled by packet
    // lookup and can therefore remain alive (src/C4Network2Res.cpp:877-884,
    // 1557-1569,1655-1673).
    let mut catalog = ResourceCatalog::new(0);
    assert!(catalog.register_at(registration(0x0002_0001), 100));
    assert!(catalog.register_at(
        ResourceRegistration {
            resource_id: 0x0002_0002,
            chunk_count: 1,
            binary_compatible: false,
            loading: false,
        },
        100,
    ));
    assert_eq!(catalog.remove_at_client(2), 2);

    let actions = catalog.on_packet_at(
        7,
        &ResourcePacket::Discover(ResourceDiscoverPacket {
            resource_ids: vec![0x0002_0001, 0x0002_0002],
        }),
        150,
    );
    assert_eq!(actions.len(), 1);
    assert_eq!(catalog.last_request_at(0x0002_0001), Some(150));
    assert_eq!(catalog.last_request_at(0x0002_0002), Some(100));

    catalog.on_timer(161);
    assert!(catalog.contains_resource(0x0002_0001));
    assert!(!catalog.contains_resource(0x0002_0002));
}

#[test]
fn cpp_only_a_valid_chunk_serve_refreshes_removed_resource_retention() {
    // SendChunk refreshes iLastReqTime after confirming a binary-compatible
    // standalone, an in-range chunk, and a data connection. Invalid requests
    // return before touching the timestamp (src/C4Network2Res.cpp:848-865,
    // 1595-1605).
    let resource_id = 0x0002_0001;
    let mut catalog = ResourceCatalog::new(0);
    assert!(catalog.register_at(registration(resource_id), 100));
    assert_eq!(catalog.remove_at_client(2), 1);

    assert!(catalog
        .on_packet_at(
            7,
            &ResourcePacket::Request(ResourceRequestPacket {
                resource_id,
                chunk: 1,
            }),
            150,
        )
        .is_empty());
    assert_eq!(catalog.last_request_at(resource_id), Some(100));
    assert_eq!(
        catalog.on_packet_at(
            7,
            &ResourcePacket::Request(ResourceRequestPacket {
                resource_id,
                chunk: 0,
            }),
            150,
        ),
        vec![ResourceCatalogAction::ServeChunk {
            peer_id: 7,
            resource_id,
            chunk: 0,
        }]
    );
    assert_eq!(catalog.last_request_at(resource_id), Some(150));

    catalog.on_timer(210);
    assert!(catalog.contains_resource(resource_id));
    catalog.on_timer(211);
    assert!(!catalog.contains_resource(resource_id));
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
    // C++'s own thresholds: this test pins StartLoad's guards, not the port's
    // scaled lobby window (see `the_lobby_load_caps_hold_the_cpp_byte_window`).
    catalog.set_max_loads_per_peer(3);
    catalog.set_max_loads(20);
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
fn cpp_client_progress_is_chunk_weighted_across_reported_nonremoved_resources() {
    // GetClientProgress sums the peer's exact present/total chunks across all
    // non-removed resources. Resource ID ownership does not participate, and
    // resources without a status from this peer do not enter the denominator
    // (src/C4Network2Res.cpp:1208-1222,1795-1806).
    let mut catalog = ResourceCatalog::new(0);
    for (resource_id, chunk_count) in [((2 << 16) | 1, 2), ((9 << 16) | 2, 8), ((7 << 16) | 3, 100)]
    {
        assert!(catalog.register(ResourceRegistration {
            resource_id,
            chunk_count,
            binary_compatible: true,
            loading: false,
        }));
    }

    let peer_id = 7;
    assert_eq!(
        catalog.record_peer_status(
            peer_id,
            &ResourceStatusPacket {
                resource_id: (2 << 16) | 1,
                chunks: ResourceChunkAvailability {
                    chunk_count: 2,
                    ranges: vec![ResourceChunkRange {
                        start: 0,
                        length: 1,
                    }],
                },
            },
        ),
        PeerStatusOutcome::Recorded
    );
    assert_eq!(
        catalog.record_peer_status(
            peer_id,
            &ResourceStatusPacket {
                resource_id: (9 << 16) | 2,
                chunks: ResourceChunkAvailability {
                    chunk_count: 8,
                    ranges: vec![ResourceChunkRange {
                        start: 0,
                        length: 2,
                    }],
                },
            },
        ),
        PeerStatusOutcome::Recorded
    );

    assert_eq!(catalog.client_progress(peer_id), 30);
    assert!(catalog.remove_resource((9 << 16) | 2));
    assert_eq!(catalog.client_progress(peer_id), 50);
}

#[test]
fn cpp_client_progress_defaults_to_complete_and_is_isolated_per_peer() {
    let mut catalog = ResourceCatalog::new(0);
    assert_eq!(catalog.client_progress(5), 100);
    assert!(catalog.register(ResourceRegistration {
        resource_id: 20,
        chunk_count: 0,
        binary_compatible: true,
        loading: false,
    }));
    assert_eq!(
        catalog.record_peer_status(
            5,
            &ResourceStatusPacket {
                resource_id: 20,
                chunks: ResourceChunkAvailability {
                    chunk_count: 0,
                    ranges: Vec::new(),
                },
            },
        ),
        PeerStatusOutcome::Recorded
    );
    assert_eq!(catalog.client_progress(5), 100);

    assert!(catalog.register(ResourceRegistration {
        resource_id: 21,
        chunk_count: 4,
        binary_compatible: true,
        loading: false,
    }));
    for (peer_id, length) in [(5, 4), (9, 1)] {
        assert_eq!(
            catalog.record_peer_status(
                peer_id,
                &ResourceStatusPacket {
                    resource_id: 21,
                    chunks: ResourceChunkAvailability {
                        chunk_count: 4,
                        ranges: vec![ResourceChunkRange { start: 0, length }],
                    },
                },
            ),
            PeerStatusOutcome::Recorded
        );
    }

    assert_eq!(catalog.client_progress(5), 100);
    assert_eq!(catalog.client_progress(9), 25);
    assert_eq!(catalog.client_progress(11), 100);
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
        loadable: true,
        file_size: 0,
        chunk_size: 102_400,
        ..NetworkResourceCore::default()
    };
    assert_eq!(
        ResourceRegistration::from_core(&empty, true, false).chunk_count,
        0
    );

    let two_chunks = NetworkResourceCore {
        loadable: true,
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
    // C++'s own thresholds: this test pins StartLoad's guards, not the port's
    // scaled lobby window (see `the_lobby_load_caps_hold_the_cpp_byte_window`).
    catalog.set_max_loads_per_peer(3);
    catalog.set_max_loads(20);
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

#[test]
fn cpp_failed_one_shot_request_rolls_back_without_starting_a_refill_pass() {
    // OnStatus calls StartLoad once and ignores a failed send. Only callers
    // already inside StartNewLoads continue to another peer
    // (src/C4Network2Res.cpp:886-909,1048-1054,1090-1110).
    let mut catalog = ResourceCatalog::new(0);
    assert!(catalog.register(ResourceRegistration {
        resource_id: 131,
        chunk_count: 4,
        binary_compatible: false,
        loading: true,
    }));
    assert_eq!(
        catalog.record_peer_status(
            7,
            &ResourceStatusPacket {
                resource_id: 131,
                chunks: ResourceChunkAvailability {
                    chunk_count: 4,
                    ranges: vec![ResourceChunkRange {
                        start: 0,
                        length: 4,
                    }],
                },
            },
        ),
        PeerStatusOutcome::Recorded
    );
    let Some(ResourceCatalogAction::SendToPeer {
        peer_id,
        packet: ResourcePacket::Request(request),
    }) = catalog.schedule_request(131, 7, 0, 10)
    else {
        panic!("peer status should schedule one request");
    };
    assert_eq!(catalog.outstanding_load_count(131), 1);
    assert!(catalog
        .on_request_send_failed(
            peer_id,
            &request,
            10,
            &std::collections::BTreeSet::from([peer_id]),
            |_| 0,
        )
        .is_empty());
    assert_eq!(catalog.outstanding_load_count(131), 0);
    assert_eq!(catalog.peer_ids(131), vec![7]);
}

#[test]
fn the_per_peer_window_narrows_in_game_and_relaxes_in_the_lobby() {
    // Bulk and control share one strictly-ordered reliable-UDP stream whenever a
    // peer has no TCP route, so what can sit ahead of a control packet on that
    // connection is this cap times the chunk size. Measured through the real
    // reliable-UDP layer at 80ms/2% loss with a chunk on the same stream: three
    // outstanding costs control 63.1ms mean / 445ms worst, one costs 53.1ms /
    // 393ms, against 49.7ms / 80ms with no bulk at all.
    //
    // It is not simply one everywhere, because it also divides throughput by
    // three -- one chunk in flight per round trip turns a multi-megabyte
    // download into minutes on a 300ms link. The blocking only matters while
    // there is control to block, so the lobby keeps C++'s three.
    let mut catalog = ResourceCatalog::new(0);
    assert_eq!(
        catalog.max_loads_per_peer(),
        RESOURCE_MAX_LOAD_PER_PEER_PER_FILE,
        "a fresh catalog is in the lobby, where a fast join is what matters"
    );

    catalog.register(ResourceRegistration {
        resource_id: 40,
        chunk_count: 50,
        binary_compatible: false,
        loading: true,
    });
    let status = ResourceStatusPacket {
        resource_id: 40,
        chunks: ResourceChunkAvailability {
            chunk_count: 50,
            ranges: vec![ResourceChunkRange {
                start: 0,
                length: 50,
            }],
        },
    };
    assert_eq!(
        catalog.record_peer_status(1, &status),
        PeerStatusOutcome::Recorded
    );

    // Three fit in the lobby.
    for _ in 0..RESOURCE_MAX_LOAD_PER_PEER_PER_FILE {
        assert!(catalog.schedule_request(40, 1, 0, 100).is_some());
    }
    assert_eq!(catalog.schedule_request(40, 1, 0, 100), None);

    // Starting the game narrows the window; a peer already over the new cap is
    // simply not given more until its outstanding requests drain.
    catalog.set_max_loads_per_peer(RESOURCE_MAX_LOAD_PER_PEER_IN_GAME);
    assert_eq!(catalog.max_loads_per_peer(), 1);
    assert_eq!(catalog.schedule_request(40, 1, 0, 100), None);

    // A fresh peer gets exactly one.
    assert_eq!(
        catalog.record_peer_status(2, &status),
        PeerStatusOutcome::Recorded
    );
    assert!(catalog.schedule_request(40, 2, 0, 100).is_some());
    assert_eq!(
        catalog.schedule_request(40, 2, 0, 100),
        None,
        "in game, one chunk per peer is the whole window"
    );
}

/// The load caps count *chunks*, so they only describe a byte window together
/// with the chunk size. This port publishes a tenth of C++'s
/// `C4NetResChunkSize`, which would divide C++'s bandwidth-delay product by ten
/// and cap one resource at 30 KiB per round trip -- minutes for a definition
/// pack on an internet link. The caps are scaled by the same factor so the
/// window stays the byte-for-byte equivalent of C++'s
/// `C4NetResMaxLoadPerPeerPerFile` x `C4NetResChunkSize`
/// (`src/C4Network2Res.h:27`, `:32`, `:33`).
#[test]
fn the_lobby_load_caps_hold_the_cpp_byte_window() {
    const CPP_CHUNK_SIZE: usize = 100 * 1024;
    const CPP_MAX_LOAD_PER_PEER_PER_FILE: usize = 3;
    const CPP_MAX_LOADS: usize = 20;

    let chunk_size = usize::try_from(clonk_network::STOCK_CHUNK_SIZE).unwrap();
    assert_eq!(
        RESOURCE_MAX_LOAD_PER_PEER_PER_FILE * chunk_size,
        CPP_MAX_LOAD_PER_PEER_PER_FILE * CPP_CHUNK_SIZE,
        "per-peer window must stay C++'s 300 KiB"
    );
    assert_eq!(
        crate::resource_catalog::RESOURCE_MAX_LOADS * chunk_size,
        CPP_MAX_LOADS * CPP_CHUNK_SIZE,
        "per-resource window must stay C++'s 2 MiB"
    );
}
