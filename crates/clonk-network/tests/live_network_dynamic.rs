use std::path::{Path, PathBuf};

use clonk_network::{
    compose_live_network_dynamic, LiveNetworkDynamicComponent, LiveNetworkDynamicSpec,
};
use clonk_resources::{Group, MutableGroup};

#[test]
fn live_dynamic_preserves_fresh_and_opaque_child_groups() {
    let mut material = MutableGroup::new("Material.c4g");
    material
        .add_file("TexMap.txt", b"Earth=Earth-earth".to_vec())
        .unwrap();

    let mut player = MutableGroup::new("Host.c4p");
    player
        .add_file("Player.txt", b"[Player]\r\nName=Host\r\n".to_vec())
        .unwrap();
    let player_contents_crc = player.contents_crc();
    let player_raw_group = player.pack_raw().unwrap();

    let dynamic = compose_live_network_dynamic(LiveNetworkDynamicSpec {
        group_filename: "DynRuntime.c4s".to_string(),
        maker: b"RuntimeHost".to_vec(),
        parameters: b"[Parameters]\r\nControlRate=2\r\n".to_vec(),
        scenario: b"[Head]\r\nSaveGame=1\r\nNetworkGame=1\r\n".to_vec(),
        components: vec![
            LiveNetworkDynamicComponent::File {
                name: "Game.txt".to_string(),
                payload: b"[Game]\r\nControlTick=42\r\n".to_vec(),
            },
            LiveNetworkDynamicComponent::Child {
                name: "Material.c4g".to_string(),
                group: material,
            },
            LiveNetworkDynamicComponent::PackedChild {
                name: "Host.c4p".to_string(),
                raw_group: player_raw_group,
                contents_crc: player_contents_crc,
                time: 0x1234_5678,
                executable: true,
            },
        ],
    })
    .unwrap();

    let packed = Group::from_memory(
        PathBuf::from("DynRuntime.c4s"),
        dynamic.packed_bytes.clone(),
    )
    .unwrap();
    assert_eq!(
        packed
            .open_child("Material.c4g")
            .unwrap()
            .read_file("TexMap.txt")
            .unwrap(),
        b"Earth=Earth-earth"
    );
    assert_eq!(
        packed
            .open_child("Host.c4p")
            .unwrap()
            .read_file("Player.txt")
            .unwrap(),
        b"[Player]\r\nName=Host\r\n"
    );

    let entries = packed.entries().unwrap();
    let player_entry = entries
        .iter()
        .find(|entry| entry.relative_path == Path::new("Host.c4p"))
        .unwrap();
    assert!(player_entry.is_directory);
    assert_eq!(player_entry.time, 0x1234_5678);
    assert!(player_entry.executable);
    assert_eq!(player_entry.stored_crc, player_contents_crc);
}

#[test]
fn live_dynamic_returns_exact_scenario_sort_order() {
    let dynamic = compose_live_network_dynamic(LiveNetworkDynamicSpec {
        group_filename: "DynRuntime.c4s".to_string(),
        maker: b"RuntimeHost".to_vec(),
        parameters: Vec::new(),
        scenario: Vec::new(),
        components: vec![
            LiveNetworkDynamicComponent::File {
                name: "Objects.txt".to_string(),
                payload: Vec::new(),
            },
            LiveNetworkDynamicComponent::File {
                name: "Game.txt".to_string(),
                payload: Vec::new(),
            },
            LiveNetworkDynamicComponent::File {
                name: "Strings.txt".to_string(),
                payload: Vec::new(),
            },
            LiveNetworkDynamicComponent::File {
                name: "Landscape.png".to_string(),
                payload: Vec::new(),
            },
        ],
    })
    .unwrap();

    assert_eq!(
        dynamic
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        [
            "Scenario.txt",
            "Game.txt",
            "Parameters.txt",
            "Landscape.png",
            "Strings.txt",
            "Objects.txt",
        ]
    );
}

/// `C4Group::Close` copies the process maker only when its first byte is
/// nonzero (`C4Group.cpp:955`), so an empty `Config.General.Name` leaves a new
/// group's native default in the header. The published metadata has to describe
/// the bytes that are actually there: `C4Network2Res::SetByGroup` reads the
/// maker back off the group, and the host rejects a runtime dynamic whose
/// rebuilt core disagrees with the composed metadata.
#[test]
fn live_dynamic_reports_the_maker_its_packed_bytes_carry() {
    let dynamic = compose_live_network_dynamic(LiveNetworkDynamicSpec {
        group_filename: "DynRuntime.c4s".to_string(),
        maker: Vec::new(),
        parameters: b"[Parameters]\r\nControlRate=2\r\n".to_vec(),
        scenario: b"[Head]\r\nSaveGame=1\r\nNetworkGame=1\r\n".to_vec(),
        components: vec![LiveNetworkDynamicComponent::File {
            name: "Game.txt".to_string(),
            payload: b"[Game]\r\nControlTick=42\r\n".to_vec(),
        }],
    })
    .unwrap();

    let packed = Group::from_memory(
        PathBuf::from("DynRuntime.c4s"),
        dynamic.packed_bytes.clone(),
    )
    .unwrap();
    assert_eq!(packed.maker_bytes(), Some(dynamic.maker.as_slice()));
}
