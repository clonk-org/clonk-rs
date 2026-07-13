use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use lc_engine::{LegacyCString, NetworkResourceCore};
use lc_network::{
    ResourceCatalogAction, ResourceFileOwnership, ResourcePacket, ResourceRequestPacket,
    ResourceStatusPacket, ResourceTransferBackend, ResourceTransferEvent,
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
                ResourceTransferEvent::Transport(ResourceCatalogAction::Broadcast { .. }) => {
                    panic!("the direct exchange should not broadcast")
                }
                ResourceTransferEvent::LoadFailed { .. }
                | ResourceTransferEvent::FinishDerivedUnsupported { .. } => {
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
        chunks: lc_network::ResourceChunkAvailability {
            chunk_count: 30,
            ranges: vec![lc_network::ResourceChunkRange {
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
        chunks: lc_network::ResourceChunkAvailability {
            chunk_count: 4,
            ranges: vec![lc_network::ResourceChunkRange {
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
fn cpp_derive_action_remains_explicit_until_group_delta_support_exists() {
    // HandlePacket delegates matching anonymous resources to FinishDerive;
    // this backend must not silently claim that C4Group delta work has run
    // (src/C4Network2Res.cpp:1584-1593).
    let directory = TestDirectory::new("derive");
    let mut backend = ResourceTransferBackend::new(1, directory.path()).unwrap();
    let derived = NetworkResourceCore {
        id: 11,
        derived_id: 4,
        ..NetworkResourceCore::default()
    };
    let mut safe_random = |_| panic!("derive does not use SafeRandom");

    let events = backend
        .process_actions(
            [ResourceCatalogAction::FinishDerived {
                core: derived.clone(),
            }],
            0,
            &mut safe_random,
        )
        .unwrap();

    assert_eq!(
        events,
        vec![ResourceTransferEvent::FinishDerivedUnsupported { core: derived }]
    );
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

static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    fn new(label: &str) -> Self {
        let unique = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "legacyclonk-resource-backend-{label}-{}-{unique}",
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
