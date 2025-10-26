use std::fs;

use image::{Rgba, RgbaImage};
use lc_engine::scenario::LegacyDefinitionResolver;
use lc_engine::{Engine, ObjectId, Scenario, ScenarioError, Vector2};
use lc_resources::Group;
use tempfile::tempdir;

const BASIC_SCRIPT: &str = r#"
global func Initialize(state, random) { return nil; }

global func Step(state, frame, random) { return nil; }
"#;

struct LocalDefinitionResolver;

impl LegacyDefinitionResolver for LocalDefinitionResolver {
    fn resolve_definition_groups(
        &self,
        scenario: &Group,
        identifier: &str,
    ) -> Result<Vec<Group>, ScenarioError> {
        let normalized = identifier.replace('\\', "/");
        let group = scenario
            .open_child(&normalized)
            .map_err(ScenarioError::Resources)?;
        Ok(vec![group])
    }
}

#[test]
fn legacy_scenario_loads_map_objects_and_definitions() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let scenario_dir = temp.path();

    let mut map = RgbaImage::new(4, 4);
    let sky = Rgba([12, 34, 56, 255]);
    let ground = Rgba([160, 120, 80, 255]);
    for y in 0..4 {
        for x in 0..4 {
            let pixel = if y == 0 { sky } else { ground };
            map.put_pixel(x, y, pixel);
        }
    }
    map.save(scenario_dir.join("Map.bmp"))?;

    fs::write(
        scenario_dir.join("Scenario.txt"),
        "[Head]\nTitle=Legacy Loader Test\n\n[Definitions]\nDefinition1=Objects.ocd\n\n[Player1]\nCrew=TEST\nPosition=30, 40\n",
    )?;

    fs::write(
        scenario_dir.join("Objects.txt"),
        "[Object]\nid=TEST\nNumber=100\nX=50\nY=60\nOwner=0\n",
    )?;

    let definition_dir = scenario_dir.join("Objects.ocd");
    fs::create_dir_all(&definition_dir)?;
    fs::write(
        definition_dir.join("DefCore.txt"),
        "[DefCore]\nid=TEST\nName=Legacy Test Object\nCategory=C4D_Object\nCrewMember=1\n",
    )?;
    fs::write(definition_dir.join("Script.c"), BASIC_SCRIPT)?;

    let resolver = LocalDefinitionResolver;
    let scenario = Scenario::load_from_path_with(scenario_dir, &resolver)?;
    assert!(scenario.has_initial_objects());

    let mut visited = Vec::new();
    scenario.visit_definition_groups(|id, _| visited.push(id.to_string()));
    assert_eq!(visited, vec!["TEST"]);

    let mut engine = Engine::new();
    scenario.apply(&mut engine)?;

    let landscape = engine.landscape().expect("legacy Map.bmp should load");
    assert_eq!(landscape.width(), 4);
    assert_eq!(landscape.surface_height(0), Some(3));

    assert!(engine.definition_ids().any(|id| id == "TEST"));
    let snapshot = engine.snapshot();
    let spawned = snapshot
        .object(ObjectId::new(100))
        .expect("object from Objects.txt");
    assert_eq!(spawned.definition_id, "TEST");
    assert_eq!(spawned.position, Vector2::new(50, 60));

    Ok(())
}
