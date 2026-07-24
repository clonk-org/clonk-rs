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
