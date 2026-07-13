use std::fs;
use std::path::PathBuf;
use std::process::Command;

use lc_engine::{
    ControlPacket as EngineControlPacket, JoinPlayerSource, CLIENT_UPDATE_ACTIVATE,
    PLAYER_INFO_FLAG_HAS_RESOURCE,
};
use lc_network::{
    decode_control_entry_payload, decode_control_payload, decode_player_info_update_payload,
    encode_control_entry_payload, encode_control_payload, encode_player_info_update_payload,
};

#[test]
#[ignore = "requires a C++ clonk binary built with USE_RUST_ENGINE_VALIDATION"]
fn synchronized_client_activation_matches_cpp_control_packet_codec() {
    // The host sends C4ControlClientUpdate as one CDT_Sync C4IDPacket. Its
    // conditional Activate body is Type, ClientID, Data, then ByClient
    // (src/C4Network2.cpp:1553-1571; src/C4Control.cpp:626-633;
    // src/C4Network2IO.cpp:1787-1793).
    let oracle = std::env::var_os("LC_CPP_CONTROL_ORACLE")
        .map(PathBuf::from)
        .expect("LC_CPP_CONTROL_ORACLE points to the validation-enabled C++ executable");
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/client_update_activate.ini")
        .canonicalize()
        .expect("C++ ClientUpdate fixture exists");
    let output = std::env::temp_dir().join(format!(
        "legacyclonk-control-packet-client-update-{}.bin",
        std::process::id()
    ));

    let result = Command::new(oracle)
        .args(["--control-packet-codec-oracle", "1"])
        .arg(fixture)
        .arg(&output)
        .output()
        .expect("C++ synchronized-control codec oracle starts");
    assert!(
        result.status.success(),
        "C++ oracle failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let cpp_bytes = fs::read(&output).expect("C++ oracle output is readable");
    let _ = fs::remove_file(&output);

    assert_eq!(cpp_bytes.first(), Some(&1));
    let control = decode_control_entry_payload(&cpp_bytes[1..])
        .expect("Rust decodes the C++ C4IDPacket payload");
    let EngineControlPacket::ClientUpdate(update) = &control else {
        panic!("expected one synchronized ClientUpdate control, got {control:?}");
    };
    assert_eq!(
        (update.update_type, update.client_id, update.data, update.by_client),
        (CLIENT_UPDATE_ACTIVATE, 3, 1, 0)
    );
    assert_eq!(
        encode_control_entry_payload(&control)
            .expect("Rust re-encodes the synchronized ClientUpdate"),
        &cpp_bytes[1..]
    );
}

#[test]
#[ignore = "requires a C++ clonk binary built with USE_RUST_ENGINE_VALIDATION"]
fn synchronized_client_removal_matches_cpp_control_packet_codec() {
    // The host sends C4ControlClientRemove as CDT_Sync. Its C4IDPacket body is
    // ClientID, a byte-preserving NUL string reason, and ByClient
    // (src/C4Client.cpp:293-304; src/C4Control.cpp:682-687;
    // src/C4Network2IO.cpp:1787-1793).
    let oracle = std::env::var_os("LC_CPP_CONTROL_ORACLE")
        .map(PathBuf::from)
        .expect("LC_CPP_CONTROL_ORACLE points to the validation-enabled C++ executable");
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/client_remove.ini")
        .canonicalize()
        .expect("C++ ClientRemove fixture exists");
    let output = std::env::temp_dir().join(format!(
        "legacyclonk-control-packet-client-remove-{}.bin",
        std::process::id()
    ));

    let result = Command::new(oracle)
        .args(["--control-packet-codec-oracle", "1"])
        .arg(fixture)
        .arg(&output)
        .output()
        .expect("C++ synchronized-control codec oracle starts");
    assert!(
        result.status.success(),
        "C++ oracle failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let cpp_bytes = fs::read(&output).expect("C++ oracle output is readable");
    let _ = fs::remove_file(&output);

    assert_eq!(cpp_bytes.first(), Some(&1));
    let control = decode_control_entry_payload(&cpp_bytes[1..])
        .expect("Rust decodes the C++ C4IDPacket payload");
    let EngineControlPacket::ClientRemove(remove) = &control else {
        panic!("expected one synchronized ClientRemove control, got {control:?}");
    };
    assert_eq!((remove.client_id, remove.by_client), (3, 0));
    assert_eq!(remove.reason.as_bytes(), b"bye");
    assert_eq!(
        encode_control_entry_payload(&control)
            .expect("Rust re-encodes the synchronized ClientRemove"),
        &cpp_bytes[1..]
    );
}

#[test]
#[ignore = "requires a C++ clonk binary built with USE_RUST_ENGINE_VALIDATION"]
fn player_info_update_request_matches_cpp_packet_codec() {
    // PID_PlayerInfoUpdReq (0x16) serializes C4ClientPlayerInfos directly and
    // carries no C4ControlPacket ByClient field (src/C4PacketBase.h:121-123;
    // src/C4PlayerInfo.cpp:601-630,1800-1803).
    let oracle = std::env::var_os("LC_CPP_CONTROL_ORACLE")
        .map(PathBuf::from)
        .expect("LC_CPP_CONTROL_ORACLE points to the validation-enabled C++ executable");
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/player_info_update_request_add.ini")
        .canonicalize()
        .expect("C++ PlayerInfo update request fixture exists");
    let output = std::env::temp_dir().join(format!(
        "legacyclonk-player-info-update-request-{}.bin",
        std::process::id()
    ));

    let result = Command::new(oracle)
        .arg("--player-info-update-codec-oracle")
        .arg(fixture)
        .arg(&output)
        .output()
        .expect("C++ PlayerInfo update request oracle starts");
    assert!(
        result.status.success(),
        "C++ oracle failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let cpp_bytes = fs::read(&output).expect("C++ oracle output is readable");
    let _ = fs::remove_file(&output);

    assert_eq!(cpp_bytes.first(), Some(&0x16));
    let request = decode_player_info_update_payload(&cpp_bytes[1..])
        .expect("Rust decodes the C++ PlayerInfo update request");
    assert_eq!((request.client_id, request.flags), (3, 1));
    let [player] = request.players.as_slice() else {
        panic!("expected one requested player, got {:?}", request.players);
    };
    assert_eq!((player.name.as_bytes(), player.id), (b"P".as_slice(), 0));
    assert_eq!(
        encode_player_info_update_payload(&request)
            .expect("Rust re-encodes the PlayerInfo update request"),
        &cpp_bytes[1..]
    );
}

#[test]
#[ignore = "requires a C++ clonk binary built with USE_RUST_ENGINE_VALIDATION"]
fn direct_player_info_matches_cpp_control_packet_codec() {
    // C4PacketControlPkt serializes its one-byte delivery before one C4IDPacket;
    // PlayerInfo is sent with CDT_Direct by the host during admission
    // (src/C4Network2IO.cpp:1787-1793;
    // src/C4Network2Players.cpp:232-239).
    let oracle = std::env::var_os("LC_CPP_CONTROL_ORACLE")
        .map(PathBuf::from)
        .expect("LC_CPP_CONTROL_ORACLE points to the validation-enabled C++ executable");
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/player_info_minimal.ini")
        .canonicalize()
        .expect("C++ PlayerInfo fixture exists");
    let output = std::env::temp_dir().join(format!(
        "legacyclonk-control-packet-player-info-{}.bin",
        std::process::id()
    ));

    let result = Command::new(oracle)
        .args(["--control-packet-codec-oracle", "2"])
        .arg(fixture)
        .arg(&output)
        .output()
        .expect("C++ direct-control codec oracle starts");
    assert!(
        result.status.success(),
        "C++ oracle failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let cpp_bytes = fs::read(&output).expect("C++ oracle output is readable");
    let _ = fs::remove_file(&output);

    assert_eq!(cpp_bytes.first(), Some(&2));
    let control = decode_control_entry_payload(&cpp_bytes[1..])
        .expect("Rust decodes the C++ C4IDPacket payload");
    let EngineControlPacket::PlayerInfo(info) = &control else {
        panic!("expected one direct PlayerInfo control, got {control:?}");
    };
    assert_eq!((info.client_id, info.by_client), (3, 4));
    assert_eq!(
        encode_control_entry_payload(&control).expect("Rust re-encodes the direct PlayerInfo"),
        &cpp_bytes[1..]
    );
}

#[test]
#[ignore = "requires a C++ clonk binary built with USE_RUST_ENGINE_VALIDATION"]
fn embedded_join_player_matches_cpp_control_codec() {
    // The C++ oracle parses the semantic C4Control fixture and serializes a
    // real C4GameControlPacket (src/C4GameControlNetwork.cpp:855-872;
    // src/C4Control.cpp:852-863). Rust must decode that live output without a
    // hand-authored binary fixture standing in for the golden implementation.
    let oracle = std::env::var_os("LC_CPP_CONTROL_ORACLE")
        .map(PathBuf::from)
        .expect("LC_CPP_CONTROL_ORACLE points to the validation-enabled C++ executable");
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/join_player_embedded.ini")
        .canonicalize()
        .expect("C++ control fixture exists");
    let output = std::env::temp_dir().join(format!(
        "legacyclonk-control-codec-join-{}.bin",
        std::process::id()
    ));

    let result = Command::new(oracle)
        .args(["--control-codec-oracle", "4", "64"])
        .arg(fixture)
        .arg(&output)
        .output()
        .expect("C++ control codec oracle starts");
    assert!(
        result.status.success(),
        "C++ oracle failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let cpp_bytes = fs::read(&output).expect("C++ oracle output is readable");
    let _ = fs::remove_file(&output);

    let frame = decode_control_payload(&cpp_bytes).expect("Rust decodes live C++ bytes");
    assert_eq!((frame.client_id, frame.tick), (4, 64));
    let [EngineControlPacket::JoinPlayer(join)] = frame.controls.as_slice() else {
        panic!("expected one JoinPlayer control, got {:?}", frame.controls);
    };
    assert_eq!(join.filename.as_bytes(), b"Players/P\x80");
    assert_eq!((join.at_client, join.info_id, join.by_client), (-1, 64, 4));
    assert_eq!(
        join.source,
        JoinPlayerSource::Embedded(vec![0xaa, 0x00, 0xcc])
    );
    assert_eq!(
        encode_control_payload(&frame).expect("Rust re-encodes the C++ control"),
        cpp_bytes
    );
}

#[test]
#[ignore = "requires a C++ clonk binary built with USE_RUST_ENGINE_VALIDATION"]
fn minimal_player_info_matches_cpp_control_codec() {
    // C4ControlPlayerInfo wraps C4ClientPlayerInfos and serializes its players
    // before the base ByClient field (src/C4Control.cpp:1284-1288;
    // src/C4PlayerInfo.cpp:177-268,601-633).
    let oracle = std::env::var_os("LC_CPP_CONTROL_ORACLE")
        .map(PathBuf::from)
        .expect("LC_CPP_CONTROL_ORACLE points to the validation-enabled C++ executable");
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/player_info_minimal.ini")
        .canonicalize()
        .expect("C++ control fixture exists");
    let output = std::env::temp_dir().join(format!(
        "legacyclonk-control-codec-player-info-{}.bin",
        std::process::id()
    ));

    let result = Command::new(oracle)
        .args(["--control-codec-oracle", "4", "7"])
        .arg(fixture)
        .arg(&output)
        .output()
        .expect("C++ control codec oracle starts");
    assert!(
        result.status.success(),
        "C++ oracle failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let cpp_bytes = fs::read(&output).expect("C++ oracle output is readable");
    let _ = fs::remove_file(&output);

    let frame = decode_control_payload(&cpp_bytes).expect("Rust decodes live C++ bytes");
    assert_eq!((frame.client_id, frame.tick), (4, 7));
    let [EngineControlPacket::PlayerInfo(info)] = frame.controls.as_slice() else {
        panic!("expected one PlayerInfo control, got {:?}", frame.controls);
    };
    assert_eq!((info.client_id, info.flags, info.by_client), (3, 0, 4));
    let [player] = info.players.as_slice() else {
        panic!("expected one player info, got {:?}", info.players);
    };
    assert_eq!(player.name.as_bytes(), b"P");
    assert_eq!(player.id, 7);
    assert_eq!(player.player_type, lc_engine::PLAYER_INFO_TYPE_USER);
    assert_eq!((player.flags, player.team), (0, 0));
    assert_eq!(
        (player.color, player.original_color),
        (0x0011_2233, 0x0011_2233)
    );
    assert_eq!(
        (
            player.game_number,
            player.game_join_frame,
            player.game_part_frame,
        ),
        (-1, -1, -1)
    );
    assert_eq!(player.extra_data, *b"NONE");
    assert_eq!(player.resource, None);
    assert_eq!(
        encode_control_payload(&frame).expect("Rust re-encodes the C++ PlayerInfo control"),
        cpp_bytes
    );
}

#[test]
#[ignore = "requires a C++ clonk binary built with USE_RUST_ENGINE_VALIDATION"]
fn resource_join_player_matches_cpp_control_codec() {
    // C4ControlJoinPlayer selects C4Network2ResCore when ByRes is true;
    // loadable cores conditionally carry file size/CRC/chunk fields
    // (src/C4Control.cpp:852-863; src/C4Network2Res.cpp:114-143).
    let oracle = std::env::var_os("LC_CPP_CONTROL_ORACLE")
        .map(PathBuf::from)
        .expect("LC_CPP_CONTROL_ORACLE points to the validation-enabled C++ executable");
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/join_player_resource.ini")
        .canonicalize()
        .expect("C++ control fixture exists");
    let output = std::env::temp_dir().join(format!(
        "legacyclonk-control-codec-resource-join-{}.bin",
        std::process::id()
    ));

    let result = Command::new(oracle)
        .args(["--control-codec-oracle", "4", "7"])
        .arg(fixture)
        .arg(&output)
        .output()
        .expect("C++ control codec oracle starts");
    assert!(
        result.status.success(),
        "C++ oracle failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let cpp_bytes = fs::read(&output).expect("C++ oracle output is readable");
    let _ = fs::remove_file(&output);

    let frame = decode_control_payload(&cpp_bytes).expect("Rust decodes live C++ bytes");
    let [EngineControlPacket::JoinPlayer(join)] = frame.controls.as_slice() else {
        panic!("expected one JoinPlayer control, got {:?}", frame.controls);
    };
    let JoinPlayerSource::Resource(resource) = &join.source else {
        panic!("expected resource-backed join, got {:?}", join.source);
    };
    assert_eq!((join.at_client, join.info_id, join.by_client), (2, 9, 4));
    assert_eq!(
        (resource.resource_type, resource.id, resource.derived_id),
        (3, 17, -1)
    );
    assert!(resource.loadable);
    assert_eq!(
        (
            resource.file_size,
            resource.file_crc,
            resource.chunk_size,
            resource.contents_crc,
        ),
        (1234, 0x1234_5678, 1024, 0x9abc_def0)
    );
    assert_eq!(resource.file_sha, None);
    assert_eq!(resource.filename.as_bytes(), b"Players/Tyler.c4p");
    assert_eq!(resource.author.as_bytes(), b"Host/Tyler");
    assert_eq!(
        encode_control_payload(&frame).expect("Rust re-encodes the C++ resource control"),
        cpp_bytes
    );
}

#[test]
#[ignore = "requires a C++ clonk binary built with USE_RUST_ENGINE_VALIDATION"]
fn sha_resource_join_player_matches_cpp_control_codec() {
    // C4Network2ResCore prefixes the optional SHA with a packed naming count,
    // then StdHexAdapt serializes its 20-byte digest
    // (src/C4Network2Res.cpp:137-142; src/StdAdaptors.h:1001-1055).
    let oracle = std::env::var_os("LC_CPP_CONTROL_ORACLE")
        .map(PathBuf::from)
        .expect("LC_CPP_CONTROL_ORACLE points to the validation-enabled C++ executable");
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/join_player_resource_sha.ini")
        .canonicalize()
        .expect("C++ SHA resource fixture exists");
    let output = std::env::temp_dir().join(format!(
        "legacyclonk-control-codec-resource-sha-{}.bin",
        std::process::id()
    ));

    let result = Command::new(oracle)
        .args(["--control-codec-oracle", "4", "7"])
        .arg(fixture)
        .arg(&output)
        .output()
        .expect("C++ control codec oracle starts");
    assert!(
        result.status.success(),
        "C++ oracle failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let cpp_bytes = fs::read(&output).expect("C++ oracle output is readable");
    let _ = fs::remove_file(&output);

    let frame = decode_control_payload(&cpp_bytes).expect("Rust decodes live C++ SHA bytes");
    let [EngineControlPacket::JoinPlayer(join)] = frame.controls.as_slice() else {
        panic!("expected one JoinPlayer control, got {:?}", frame.controls);
    };
    let JoinPlayerSource::Resource(resource) = &join.source else {
        panic!("expected resource-backed join, got {:?}", join.source);
    };
    assert_eq!(
        resource.file_sha,
        Some([
            0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
            0xee, 0xff, 0x10, 0x20, 0x30, 0x40,
        ])
    );
    assert_eq!(
        encode_control_payload(&frame).expect("Rust re-encodes the C++ SHA resource control"),
        cpp_bytes
    );
}

#[test]
#[ignore = "requires a C++ clonk binary built with USE_RUST_ENGINE_VALIDATION"]
fn resource_player_info_matches_cpp_control_codec() {
    // C4PlayerInfo serializes ResCore after the league fields whenever its
    // synchronized HasResource flag is set (src/C4PlayerInfo.cpp:177-268).
    let oracle = std::env::var_os("LC_CPP_CONTROL_ORACLE")
        .map(PathBuf::from)
        .expect("LC_CPP_CONTROL_ORACLE points to the validation-enabled C++ executable");
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/player_info_resource.ini")
        .canonicalize()
        .expect("C++ resource PlayerInfo fixture exists");
    let output = std::env::temp_dir().join(format!(
        "legacyclonk-control-codec-player-info-resource-{}.bin",
        std::process::id()
    ));

    let result = Command::new(oracle)
        .args(["--control-codec-oracle", "4", "7"])
        .arg(fixture)
        .arg(&output)
        .output()
        .expect("C++ control codec oracle starts");
    assert!(
        result.status.success(),
        "C++ oracle failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let cpp_bytes = fs::read(&output).expect("C++ oracle output is readable");
    let _ = fs::remove_file(&output);

    let frame = decode_control_payload(&cpp_bytes).expect("Rust decodes live C++ PlayerInfo bytes");
    let [EngineControlPacket::PlayerInfo(info)] = frame.controls.as_slice() else {
        panic!("expected one PlayerInfo control, got {:?}", frame.controls);
    };
    let [player] = info.players.as_slice() else {
        panic!("expected one player info, got {:?}", info.players);
    };
    assert_ne!(player.flags & PLAYER_INFO_FLAG_HAS_RESOURCE, 0);
    let resource = player.resource.as_ref().expect("player resource core");
    assert_eq!(
        (resource.resource_type, resource.id, resource.derived_id),
        (3, 17, -1)
    );
    assert_eq!(
        (
            resource.file_size,
            resource.file_crc,
            resource.chunk_size,
            resource.contents_crc,
        ),
        (1234, 0x1234_5678, 1024, 0x9abc_def0)
    );
    assert_eq!(resource.file_sha, None);
    assert_eq!(resource.filename.as_bytes(), b"Players/Tyler.c4p");
    assert_eq!(resource.author.as_bytes(), b"Host/Tyler");
    assert_eq!(
        encode_control_payload(&frame).expect("Rust re-encodes the C++ resource PlayerInfo"),
        cpp_bytes
    );
}
