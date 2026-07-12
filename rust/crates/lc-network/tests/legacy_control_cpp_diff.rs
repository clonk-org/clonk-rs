use std::fs;
use std::path::PathBuf;
use std::process::Command;

use lc_engine::{ControlPacket as EngineControlPacket, JoinPlayerSource};
use lc_network::{decode_control_payload, encode_control_payload};

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
        "legacyclonk-control-codec-{}.bin",
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
