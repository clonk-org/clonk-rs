use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use clonk_engine::{LegacyCString, NetworkResourceCore};
use clonk_network::{
    HostResourceType, ResourceCatalogAction, ResourceDataPacket, ResourceDiscoverPacket,
    ResourceFileOwnership, ResourcePacket, ResourceRequestPacket, ResourceStatusPacket,
    ResourceTransferBackend, ResourceTransferEvent,
};

fn core(
    resource_id: i32,
    filename: &[u8],
    file_size: u32,
    chunk_size: u32,
    file_crc: u32,
) -> NetworkResourceCore {
    NetworkResourceCore {
        id: resource_id,
        loadable: true,
        file_size,
        file_crc,
        chunk_size,
        filename: LegacyCString::from_bytes(filename.to_vec()).unwrap(),
        ..NetworkResourceCore::default()
    }
}

fn derivable_core(resource_id: i32) -> NetworkResourceCore {
    NetworkResourceCore {
        resource_type: HostResourceType::Scenario as u8,
        ..core(resource_id, b"scenario.c4s", 5, 2, 0x8bd6_88e8)
    }
}

#[test]
fn cpp_host_and_client_exchange_all_chunks_without_sockets() {
    // OnStatus starts one load; every successful OnChunk either ends the load
    // or calls StartNewLoads (src/C4Network2Res.cpp:886-940,1017-1122).
    // SendChunk/Set/AddTo carry bytes at chunk*ChunkSize using the stock 100 KiB
    // data cap (src/C4Network2Res.cpp:848-865,1230-1318).
    let host_directory = TestDirectory::new("host");
    let client_directory = TestDirectory::new("client");
    let source = host_directory.path().join("scenario.c4s");
    fs::write(&source, b"local").unwrap();
    let resource_core = core(7, b"scenario.c4s", 5, 2, 0x8bd6_88e8);
    let mut host = ResourceTransferBackend::new(0, host_directory.path()).unwrap();
    host.register_local_complete(
        resource_core.clone(),
        &source,
        ResourceFileOwnership::Persistent,
        true,
    )
    .unwrap();
    let mut client = ResourceTransferBackend::new(1, client_directory.path()).unwrap();
    let destination = client
        .register_remote_loadable(resource_core.clone())
        .unwrap();
    let mut safe_random = |_| 0;

    let discover = only_transport(client.on_peer_connected(0, 0, &mut safe_random).unwrap());
    let mut wire = VecDeque::from([(1, 0, discover)]);
    let mut completed = Vec::new();
    let mut progress = Vec::new();

    while let Some((from, to, packet)) = wire.pop_front() {
        let events = if to == 0 {
            host.on_packet(from, &packet, 0, &mut safe_random).unwrap()
        } else {
            client
                .on_packet(from, &packet, 0, &mut safe_random)
                .unwrap()
        };
        for event in events {
            match event {
                ResourceTransferEvent::Transport(ResourceCatalogAction::SendToPeer {
                    peer_id,
                    packet,
                }) => wire.push_back((to, peer_id, packet)),
                ResourceTransferEvent::Completed {
                    resource_id,
                    core,
                    path,
                } => completed.push((resource_id, core, path)),
                ResourceTransferEvent::Progress {
                    resource_id,
                    present_percent,
                } => progress.push((resource_id, present_percent)),
                ResourceTransferEvent::Transport(ResourceCatalogAction::Broadcast { .. }) => {
                    panic!("the direct exchange should not broadcast")
                }
                ResourceTransferEvent::LoadFailed { .. } => {
                    panic!("unexpected terminal resource event")
                }
                ResourceTransferEvent::Transport(action) => {
                    panic!("backend leaked internal action: {action:?}")
                }
            }
        }
    }

    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].0, 7);
    assert_eq!(completed[0].1, resource_core);
    assert_eq!(completed[0].2, destination);
    assert_eq!(progress, vec![(7, 33), (7, 66), (7, 100)]);
    assert_eq!(fs::read(destination).unwrap(), b"local");
    assert!(client.catalog().outstanding_load_count(7) == 0);
}

#[test]
fn cpp_refill_stops_at_three_requests_per_peer_and_nineteen_total() {
    // StartLoad refuses the third existing load at a peer and the twentieth
    // total load (src/C4Network2Res.cpp:1042-1110).
    let directory = TestDirectory::new("limits");
    let resource_core = core(8, b"large.c4s", 30, 1, 0);
    let mut backend = ResourceTransferBackend::new(1, directory.path()).unwrap();
    backend.register_remote_loadable(resource_core).unwrap();
    let status = ResourcePacket::Status(ResourceStatusPacket {
        resource_id: 8,
        chunks: clonk_network::ResourceChunkAvailability {
            chunk_count: 30,
            ranges: vec![clonk_network::ResourceChunkRange {
                start: 0,
                length: 30,
            }],
        },
    });
    let mut safe_random = |_| 0;
    let mut requests = Vec::new();

    for peer_id in 10..17 {
        requests.extend(requests_from(
            backend
                .on_packet(peer_id, &status, 0, &mut safe_random)
                .unwrap(),
        ));
    }
    requests.extend(requests_from(
        backend
            .process_actions(
                [ResourceCatalogAction::RefillRequests { resource_id: 8 }],
                0,
                &mut safe_random,
            )
            .unwrap(),
    ));

    let mut by_peer = HashMap::<i32, usize>::new();
    requests.iter().for_each(|(peer_id, _)| {
        *by_peer.entry(*peer_id).or_default() += 1;
    });
    assert_eq!(requests.len(), 19);
    assert_eq!(backend.catalog().outstanding_load_count(8), 19);
    assert!(by_peer.values().all(|count| *count <= 3));
    assert_eq!(by_peer.values().copied().max(), Some(3));
    assert_eq!(backend.remove_at_client(0), 1);
}

#[test]
fn cpp_status_schedules_one_request_with_an_eligible_chunk_draw() {
    // OnStatus calls StartLoad once, and GetChunkToRetrieve bounds SafeRandom
    // by the eligible chunk count (src/C4Network2Res.cpp:886-909,254-271).
    let directory = TestDirectory::new("single-request");
    let mut backend = ResourceTransferBackend::new(1, directory.path()).unwrap();
    backend
        .register_remote_loadable(core(10, b"four.c4s", 4, 1, 0))
        .unwrap();
    let status = ResourcePacket::Status(ResourceStatusPacket {
        resource_id: 10,
        chunks: clonk_network::ResourceChunkAvailability {
            chunk_count: 4,
            ranges: vec![clonk_network::ResourceChunkRange {
                start: 0,
                length: 4,
            }],
        },
    });
    let mut bounds = Vec::new();
    let mut safe_random = |upper_bound| {
        bounds.push(upper_bound);
        2
    };

    let requests = requests_from(backend.on_packet(7, &status, 0, &mut safe_random).unwrap());

    assert_eq!(bounds, vec![4]);
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].0, 7);
    assert_eq!(requests[0].1.chunk, 2);
}

#[test]
fn cpp_host_derivation_preserves_old_bytes_and_broadcasts_the_finished_core() {
    // Derive rescues a standalone that aliases the mutable source; FinishDerive
    // binds the rewritten source to the new ID and broadcasts that exact core
    // (src/C4Network2Res.cpp:718-823).
    let directory = TestDirectory::new("derive-host");
    let source = directory.path().join("scenario.c4s");
    fs::write(&source, b"local").unwrap();
    let parent = derivable_core(4);
    let mut backend = ResourceTransferBackend::new(0, directory.path()).unwrap();
    backend
        .register_local_complete(parent, &source, ResourceFileOwnership::Persistent, true)
        .unwrap();

    let derivation = backend
        .begin_derive(4, &source, ResourceFileOwnership::Persistent, 17)
        .unwrap();
    let rescued = backend.path(4).unwrap().to_path_buf();
    assert_ne!(rescued, source);
    assert_eq!(fs::read(&rescued).unwrap(), b"local");

    fs::write(&source, b"changed").unwrap();
    let (derived, events) = backend.finish_derive(derivation, 11).unwrap();

    assert_eq!(derived.id, 11);
    assert_eq!(derived.derived_id, 4);
    assert_eq!(
        events,
        vec![
            ResourceTransferEvent::Completed {
                resource_id: 11,
                core: derived.clone(),
                path: source.clone(),
            },
            ResourceTransferEvent::Transport(ResourceCatalogAction::Broadcast {
                packet: ResourcePacket::Derive(derived.clone()),
            }),
        ]
    );
    assert_eq!(backend.core(4).unwrap().id, 4);
    assert_eq!(backend.core(11), Some(&derived));
    assert_eq!(fs::read(backend.path(4).unwrap()).unwrap(), b"local");
    assert_eq!(fs::read(backend.path(11).unwrap()).unwrap(), b"changed");
}

#[test]
fn cpp_matching_derive_rebinds_the_complete_file_without_downloading() {
    // Receiving PID_NetResDerive finishes a matching anonymous SetDerived
    // entry in place and marks every chunk present without CRC verification
    // (src/C4Network2Res.cpp:526-546,778-823,1584-1593).
    let directory = TestDirectory::new("derive-receive");
    let source = directory.path().join("scenario.c4s");
    fs::write(&source, b"local").unwrap();
    let mut backend = ResourceTransferBackend::new(1, directory.path()).unwrap();
    backend
        .register_local_complete(
            derivable_core(4),
            &source,
            ResourceFileOwnership::Persistent,
            true,
        )
        .unwrap();
    let _derivation = backend
        .begin_derive(4, &source, ResourceFileOwnership::Persistent, 17)
        .unwrap();
    fs::write(&source, b"changed").unwrap();
    let derived = NetworkResourceCore {
        resource_type: HostResourceType::Scenario as u8,
        id: 11,
        derived_id: 4,
        loadable: true,
        file_size: 7,
        file_crc: 0xdead_beef,
        chunk_size: 2,
        filename: LegacyCString::from_bytes(b"scenario.c4s".to_vec()).unwrap(),
        ..NetworkResourceCore::default()
    };
    let mut safe_random = |_| panic!("derive does not use SafeRandom");

    let events = backend
        .on_packet(
            0,
            &ResourcePacket::Derive(derived.clone()),
            18,
            &mut safe_random,
        )
        .unwrap();

    assert_eq!(
        events,
        vec![ResourceTransferEvent::Completed {
            resource_id: 11,
            core: derived.clone(),
            path: source.clone(),
        }]
    );
    assert_eq!(backend.core(11), Some(&derived));
    assert_eq!(backend.path(11), Some(source.as_path()));
    assert_eq!(fs::read(backend.path(11).unwrap()).unwrap(), b"changed");
    let chunks = backend.catalog().local_chunks(11).unwrap();
    assert!(chunks.is_complete());
    assert_eq!(chunks.present_chunk_count(), chunks.chunk_count());
    assert_eq!(backend.catalog().outstanding_load_count(11), 0);
}

#[test]
fn cpp_unmatched_derive_packet_is_a_silent_no_op() {
    // HandlePacket scans only matching anonymous resources and otherwise does
    // nothing (src/C4Network2Res.cpp:1584-1593).
    let directory = TestDirectory::new("derive-unmatched");
    let mut backend = ResourceTransferBackend::new(1, directory.path()).unwrap();
    let derived = NetworkResourceCore {
        id: 11,
        derived_id: 4,
        ..NetworkResourceCore::default()
    };
    let mut safe_random = |_| panic!("derive does not use SafeRandom");

    let events = backend
        .on_packet(0, &ResourcePacket::Derive(derived), 18, &mut safe_random)
        .unwrap();

    assert!(events.is_empty());
    assert!(backend.core(11).is_none());
    assert!(backend.path(11).is_none());
}

#[test]
fn cpp_load_failure_removes_temporary_storage_and_reports_it() {
    // A failed DoLoad marks the resource removed; clearing a loading resource
    // deletes its temporary file (src/C4Network2Res.cpp:943-1002,1621-1629).
    let directory = TestDirectory::new("failure");
    let resource_core = core(9, b"missing.c4s", 1, 1, 0);
    let mut backend = ResourceTransferBackend::new(1, directory.path()).unwrap();
    let path = backend.register_remote_loadable(resource_core).unwrap();
    let mut safe_random = |_| 0;

    let initial = backend.on_timer(0, &mut safe_random).unwrap();
    assert!(matches!(
        initial.as_slice(),
        [ResourceTransferEvent::Transport(
            ResourceCatalogAction::Broadcast {
                packet: ResourcePacket::Discover(_)
            }
        )]
    ));
    let events = backend.on_timer(11, &mut safe_random).unwrap();

    assert_eq!(
        events,
        vec![ResourceTransferEvent::LoadFailed { resource_id: 9 }]
    );
    assert!(!path.exists());
    assert!(backend.path(9).is_none());
    assert!(backend.core(9).is_none());
}

#[test]
fn cpp_delayed_expiry_clears_backend_state_and_only_unlinks_temporary_files() {
    // OnShareFree drops a removed resource strictly after the 60-second
    // grace window. C4Network2Res::Clear unlinks temporary/standalone copies
    // but leaves persistent source files alone (src/C4Network2Res.cpp:
    // 983-1002,1655-1675).
    let directory = TestDirectory::new("delayed-expiry");
    let mut backend = ResourceTransferBackend::new(1, directory.path()).unwrap();

    let temporary_id = 0x0002_0001;
    let temporary_path = backend
        .register_remote_loadable(core(temporary_id, b"remote.c4s", 1, 1, 0))
        .unwrap();

    let persistent_id = 0x0002_0002;
    let persistent_path = directory.path().join("persistent.c4s");
    fs::write(&persistent_path, b"local").unwrap();
    backend
        .register_local_complete(
            core(persistent_id, b"persistent.c4s", 5, 2, 0x8bd6_88e8),
            &persistent_path,
            ResourceFileOwnership::Persistent,
            true,
        )
        .unwrap();

    let logical_id = 0x0002_0003;
    let logical_path = directory.path().join("logical.c4g");
    fs::write(&logical_path, b"logical").unwrap();
    let mut logical_core = core(logical_id, b"logical.c4g", 7, 1, 0);
    logical_core.loadable = false;
    backend
        .register_local_logical(logical_core, &logical_path)
        .unwrap();

    let mut safe_random = |_| 0;
    let _ = backend.on_timer(100, &mut safe_random).unwrap();
    assert!(backend.remove_resource(temporary_id));
    assert!(backend.remove_resource(persistent_id));
    assert!(backend.remove_resource(logical_id));

    assert!(backend.on_timer(160, &mut safe_random).unwrap().is_empty());
    for resource_id in [temporary_id, persistent_id, logical_id] {
        assert!(backend.core(resource_id).is_some());
        assert!(backend.path(resource_id).is_some());
    }
    assert!(temporary_path.exists());
    assert!(persistent_path.exists());
    assert!(logical_path.exists());

    assert!(backend.on_timer(161, &mut safe_random).unwrap().is_empty());
    for resource_id in [temporary_id, persistent_id, logical_id] {
        assert!(backend.core(resource_id).is_none());
        assert!(backend.path(resource_id).is_none());
    }
    assert!(!temporary_path.exists());
    assert!(persistent_path.exists());
    assert!(logical_path.exists());
}

#[test]
fn cpp_backend_forwards_packet_time_to_removed_resource_retention() {
    // OnDiscover refreshes iLastReqTime, and OnShareFree measures its strict
    // 60-second retention window from that packet activity
    // (src/C4Network2Res.cpp:877-884,1655-1673).
    let directory = TestDirectory::new("packet-time");
    let source = directory.path().join("scenario.c4s");
    fs::write(&source, b"local").unwrap();
    let resource_id = 0x0002_0001;
    let mut backend = ResourceTransferBackend::new(0, directory.path()).unwrap();
    backend
        .register_local_complete(
            core(resource_id, b"scenario.c4s", 5, 2, 0x8bd6_88e8),
            &source,
            ResourceFileOwnership::Persistent,
            true,
        )
        .unwrap();
    let mut safe_random = |_| 0;
    backend.on_timer(100, &mut safe_random).unwrap();
    assert_eq!(backend.remove_at_client(2), 1);

    backend
        .on_packet(
            7,
            &ResourcePacket::Discover(ResourceDiscoverPacket {
                resource_ids: vec![resource_id],
            }),
            150,
            &mut safe_random,
        )
        .unwrap();

    backend.on_timer(161, &mut safe_random).unwrap();
    assert!(backend.catalog().contains_resource(resource_id));
    backend.on_timer(211, &mut safe_random).unwrap();
    assert!(!backend.catalog().contains_resource(resource_id));
}

#[test]
fn cpp_resource_chunk_failure_drops_bad_stores_and_continues_batch() {
    // AddTo failures do not abort OnChunk's surrounding work. The chunk
    // bitmap changes only after a complete write, and later queued actions
    // still run (src/C4Network2Res.cpp:911-941,1263-1319).
    let directory = TestDirectory::new("bad-store-batch");
    let mut backend = ResourceTransferBackend::new(1, directory.path()).unwrap();
    let path = backend
        .register_remote_loadable(core(12, b"two.c4s", 2, 1, 0))
        .unwrap();
    let mut safe_random = |_| 0;

    let initial = requests_from(
        backend
            .on_packet(
                7,
                &ResourcePacket::Status(ResourceStatusPacket {
                    resource_id: 12,
                    chunks: clonk_network::ResourceChunkAvailability {
                        chunk_count: 2,
                        ranges: vec![clonk_network::ResourceChunkRange {
                            start: 0,
                            length: 2,
                        }],
                    },
                }),
                0,
                &mut safe_random,
            )
            .unwrap(),
    );
    assert_eq!(initial.len(), 1);
    assert_eq!(initial[0].1.chunk, 0);

    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    let trailing = ResourceCatalogAction::Broadcast {
        packet: ResourcePacket::Discover(ResourceDiscoverPacket {
            resource_ids: vec![12],
        }),
    };
    let io_events = backend
        .process_actions(
            [
                ResourceCatalogAction::StoreChunk {
                    peer_id: 7,
                    packet: ResourceDataPacket {
                        resource_id: 12,
                        chunk: 0,
                        data: vec![b'A'],
                    },
                },
                trailing.clone(),
            ],
            0,
            &mut safe_random,
        )
        .expect("write-open failure is dropped without aborting the batch");
    assert_eq!(
        io_events,
        [
            ResourceTransferEvent::Transport(ResourceCatalogAction::SendToPeer {
                peer_id: 7,
                packet: ResourcePacket::Request(ResourceRequestPacket {
                    resource_id: 12,
                    chunk: 1,
                }),
            }),
            ResourceTransferEvent::Transport(trailing),
        ]
    );
    assert!(!backend.catalog().local_chunks(12).unwrap().contains(0));
    fs::remove_dir(&path).unwrap();
    fs::write(&path, []).unwrap();

    let events = backend
        .process_actions(
            [
                ResourceCatalogAction::StoreChunk {
                    peer_id: 7,
                    packet: ResourceDataPacket {
                        resource_id: 12,
                        chunk: 2,
                        data: vec![b'X'],
                    },
                },
                ResourceCatalogAction::StoreChunk {
                    peer_id: 7,
                    packet: ResourceDataPacket {
                        resource_id: 12,
                        chunk: 2,
                        data: Vec::new(),
                    },
                },
                ResourceCatalogAction::StoreChunk {
                    peer_id: 7,
                    packet: ResourceDataPacket {
                        resource_id: 12,
                        chunk: 0,
                        data: vec![b'A'],
                    },
                },
                ResourceCatalogAction::StoreChunk {
                    peer_id: 7,
                    packet: ResourceDataPacket {
                        resource_id: 12,
                        chunk: 1,
                        data: vec![b'B'],
                    },
                },
            ],
            0,
            &mut safe_random,
        )
        .expect("peer-controlled bad chunks are dropped");

    assert!(matches!(
        events.as_slice(),
        [
            ResourceTransferEvent::Progress {
                resource_id: 12,
                present_percent: 50,
            },
            ResourceTransferEvent::Progress {
                resource_id: 12,
                present_percent: 100,
            },
            ResourceTransferEvent::Completed {
                resource_id: 12,
                ..
            }
        ]
    ));
    assert_eq!(fs::read(path).unwrap(), b"AB");
}

#[test]
fn cpp_resource_chunk_failure_retains_load_until_timeout_refill() {
    // Failed AddTo leaves the original load wait and timestamp untouched.
    // DoLoad expires it at 60 seconds and StartNewLoads immediately issues a
    // replacement (src/C4Network2Res.cpp:911-941,943-969).
    let directory = TestDirectory::new("bad-store-timeout");
    let mut backend = ResourceTransferBackend::new(1, directory.path()).unwrap();
    backend
        .register_remote_loadable(core(13, b"one.c4s", 1, 1, 0))
        .unwrap();
    let status = ResourcePacket::Status(ResourceStatusPacket {
        resource_id: 13,
        chunks: clonk_network::ResourceChunkAvailability {
            chunk_count: 1,
            ranges: vec![clonk_network::ResourceChunkRange {
                start: 0,
                length: 1,
            }],
        },
    });
    let mut safe_random = |_| 0;

    let initial = requests_from(backend.on_packet(7, &status, 10, &mut safe_random).unwrap());
    assert_eq!(
        initial,
        [(
            7,
            ResourceRequestPacket {
                resource_id: 13,
                chunk: 0
            }
        )]
    );
    assert_eq!(backend.catalog().outstanding_load_count(13), 1);

    let dropped = backend
        .on_packet(
            7,
            &ResourcePacket::Data(ResourceDataPacket {
                resource_id: 13,
                chunk: 0,
                data: vec![b'X', b'Y'],
            }),
            11,
            &mut safe_random,
        )
        .expect("oversized requested chunk is non-fatal");
    assert!(dropped.is_empty());
    assert_eq!(backend.catalog().outstanding_load_count(13), 1);

    let before_timeout = backend.on_timer(69, &mut safe_random).unwrap();
    assert!(!before_timeout.iter().any(is_resource_request));
    assert_eq!(backend.catalog().outstanding_load_count(13), 1);

    let at_timeout = backend.on_timer(70, &mut safe_random).unwrap();
    let retries = at_timeout
        .iter()
        .filter_map(|event| match event {
            ResourceTransferEvent::Transport(ResourceCatalogAction::SendToPeer {
                peer_id,
                packet: ResourcePacket::Request(request),
            }) => Some((*peer_id, *request)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        retries,
        [(
            7,
            ResourceRequestPacket {
                resource_id: 13,
                chunk: 0
            }
        )]
    );
    assert_eq!(backend.catalog().outstanding_load_count(13), 1);
}

#[test]
fn cpp_resource_chunk_failure_serves_empty_data_after_read_error() {
    // SendChunk ignores Set's return after validating the request, so an
    // open/read failure still sends the ids with an empty data buffer and the
    // rest of the action batch continues (src/C4Network2Res.cpp:848-865).
    let directory = TestDirectory::new("bad-serve");
    let source = directory.path().join("scenario.c4s");
    fs::write(&source, b"local").unwrap();
    let mut backend = ResourceTransferBackend::new(0, directory.path()).unwrap();
    backend
        .register_local_complete(
            core(14, b"scenario.c4s", 5, 2, 0x8bd6_88e8),
            &source,
            ResourceFileOwnership::Persistent,
            true,
        )
        .unwrap();
    fs::remove_file(&source).unwrap();
    let trailing = ResourceCatalogAction::Broadcast {
        packet: ResourcePacket::Discover(ResourceDiscoverPacket {
            resource_ids: vec![14],
        }),
    };
    let mut safe_random = |_| 0;

    let events = backend
        .process_actions(
            [
                ResourceCatalogAction::ServeChunk {
                    peer_id: 7,
                    resource_id: 14,
                    chunk: 0,
                },
                trailing.clone(),
            ],
            0,
            &mut safe_random,
        )
        .expect("read failure becomes an empty data packet");

    assert_eq!(
        events,
        vec![
            ResourceTransferEvent::Transport(ResourceCatalogAction::SendToPeer {
                peer_id: 7,
                packet: ResourcePacket::Data(ResourceDataPacket {
                    resource_id: 14,
                    chunk: 0,
                    data: Vec::new(),
                }),
            }),
            ResourceTransferEvent::Transport(trailing),
        ]
    );
}

fn only_transport(events: Vec<ResourceTransferEvent>) -> ResourcePacket {
    let [ResourceTransferEvent::Transport(ResourceCatalogAction::SendToPeer { packet, .. })] =
        events.as_slice()
    else {
        panic!("expected exactly one directed transport event, got {events:?}")
    };
    packet.clone()
}

fn requests_from(events: Vec<ResourceTransferEvent>) -> Vec<(i32, ResourceRequestPacket)> {
    events
        .into_iter()
        .map(|event| match event {
            ResourceTransferEvent::Transport(ResourceCatalogAction::SendToPeer {
                peer_id,
                packet: ResourcePacket::Request(request),
            }) => (peer_id, request),
            event => panic!("expected a directed request, got {event:?}"),
        })
        .collect()
}

fn is_resource_request(event: &ResourceTransferEvent) -> bool {
    matches!(
        event,
        ResourceTransferEvent::Transport(ResourceCatalogAction::SendToPeer {
            packet: ResourcePacket::Request(_),
            ..
        })
    )
}

static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    fn new(label: &str) -> Self {
        let unique = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "clonk-rust-resource-backend-{label}-{}-{unique}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
