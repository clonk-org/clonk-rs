use std::fs;
use std::path::Path;

use clonk_engine::{Engine, Scenario};
use image::{Rgba, RgbaImage};

fn tempdir() -> std::io::Result<tempfile::TempDir> {
    tempfile::Builder::new().prefix("lc-test-").tempdir()
}

const BASIC_SCRIPT: &str = r#"
global func Initialize(state, random) { return 0; }

global func Step(state, frame, random) { return 0; }
"#;

fn write_png(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let mut image = RgbaImage::new(8, 8);
    for pixel in image.pixels_mut() {
        *pixel = Rgba([200, 32, 32, 255]);
    }
    image.save(path)?;
    Ok(())
}

#[test]
fn manifest_definition_loads_graphics_image() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let scenario_dir = temp.path();
    let definition_dir = scenario_dir.join("Definitions").join("Test.ocd");
    fs::create_dir_all(&definition_dir)?;

    fs::write(
        definition_dir.join("DefCore.txt"),
        br#"[DefCore]
id=TEST
Name=Manifest Test
Category=C4D_Object
"#,
    )?;
    fs::write(definition_dir.join("Script.c"), BASIC_SCRIPT)?;
    write_png(&definition_dir.join("Graphics.png"))?;

    let scenario_json = r#"{
        "name": "Manifest Graphics",
        "definitions": [
            {
                "id": "TEST",
                "name": "Manifest Test",
                "script": "Definitions/Test.ocd/Script.c",
                "default_action": "Idle",
                "actions": {
                    "Idle": {}
                }
            }
        ],
        "initial_objects": [
            {
                "definition": "TEST",
                "position": [0, 0]
            }
        ]
    }"#;
    fs::write(scenario_dir.join("Scenario.json"), scenario_json)?;

    let scenario = Scenario::load_from_path(scenario_dir)?;
    let mut engine = Engine::new();
    scenario.apply(&mut engine)?;

    assert!(
        engine.definition_sprite_image("TEST", None).is_some(),
        "expected manifest definition sprites to load"
    );

    Ok(())
}

#[test]
fn manifest_movement_explicitly_opts_into_float_profile() -> Result<(), Box<dyn std::error::Error>>
{
    let temp = tempdir()?;
    let scenario_dir = temp.path();
    // CATEGORY_OBJECT keeps the fixture mobile; the JSON manifest encodes the
    // same C4 category value used by the native object path.
    fs::write(scenario_dir.join("Script.c"), BASIC_SCRIPT)?;
    fs::write(
        scenario_dir.join("Scenario.json"),
        r#"{
            "physics": { "gravity": 0 },
            "definitions": [
                {
                    "id": "TEST",
                    "script": "Script.c",
                    "default_action": "Float",
                    "actions": {
                        "Float": { "procedure": "float", "length": 1, "delay": 1 }
                    },
                    "movement": {
                        "float": { "speed": 6, "acceleration": 2 }
                    }
                }
            ],
            "initial_objects": [
                {
                    "definition": "TEST",
                    "velocity": [2, -2],
                    "category": 16
                }
            ]
        }"#,
    )?;

    let scenario = Scenario::load_from_path(scenario_dir)?;
    let mut engine = Engine::new();
    let objects = scenario.apply(&mut engine)?;
    assert_eq!(
        engine
            .definition("TEST")
            .map(|definition| definition.movement_profile().float_acceleration),
        Some(2),
        "explicit MovementManifest must reach the compiled synthetic definition",
    );
    assert_eq!(
        engine
            .definition("TEST")
            .map(|definition| definition.physical().float),
        Some(0),
        "the synthetic definition must not gain a Float physical",
    );
    engine.tick_without_snapshot()?;

    let snapshot = engine
        .object_snapshot(objects[0])
        .ok_or_else(|| std::io::Error::other("manifest object was not retained"))?;
    // An explicit movement manifest opts into the synthetic command profile;
    // native DFA_FLOAT bounds remain available when the manifest is omitted
    // (src/C4Object.cpp:5291-5309).
    assert_eq!(snapshot.velocity, clonk_engine::Vector2::new(2, -2));

    let native_dir = scenario_dir.join("native");
    fs::create_dir(&native_dir)?;
    fs::write(native_dir.join("Script.c"), BASIC_SCRIPT)?;
    fs::write(
        native_dir.join("Scenario.json"),
        r#"{
            "physics": { "gravity": 0 },
            "definitions": [
                {
                    "id": "TEST",
                    "script": "Script.c",
                    "default_action": "Float",
                    "actions": {
                        "Float": { "procedure": "float", "length": 1, "delay": 1 }
                    }
                }
            ],
            "initial_objects": [
                {
                    "definition": "TEST",
                    "velocity": [2, -2],
                    "category": 16
                }
            ]
        }"#,
    )?;

    let native_scenario = Scenario::load_from_path(&native_dir)?;
    let mut native_engine = Engine::new();
    let native_objects = native_scenario.apply(&mut native_engine)?;
    native_engine.tick_without_snapshot()?;
    let native_snapshot = native_engine
        .object_snapshot(native_objects[0])
        .ok_or_else(|| std::io::Error::other("native manifest object was not retained"))?;
    // C++ clamps absent [Physical] Float to FIXED100(0), so both axes stop.
    assert_eq!(native_snapshot.velocity, clonk_engine::Vector2::ZERO);

    Ok(())
}
