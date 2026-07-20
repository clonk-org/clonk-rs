use std::fs;

use image::{Rgba, RgbaImage};
use lc_engine::scenario::LegacyDefinitionResolver;
use lc_engine::{Engine, Landscape, ObjectId, Scenario, ScenarioError, SpawnConfig, Vector2};
use lc_resources::Group;

fn tempdir() -> std::io::Result<tempfile::TempDir> {
    tempfile::Builder::new().prefix("lc-test-").tempdir()
}

const BASIC_SCRIPT: &str = r#"
global func Initialize(state, random) { return 0; }

global func Step(state, frame, random) { return 0; }
"#;

#[test]
fn defcore_rct_all_timer_call_trailing_space_misses_exact_runtime_lookup(
) -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    fs::write(
        temp.path().join("DefCore.txt"),
        "[DefCore]\nid=RCTA\nName= Bar \nCategory=C4D_Object\nTimer=1\nTimerCall=Foo \n",
    )?;
    fs::write(
        temp.path().join("Script.c"),
        "#strict 2\nlocal iFired;\nfunc Foo() { iFired = 1; return 1; }\n",
    )?;

    let resource = lc_resources::definition::Definition::load(&Group::open(temp.path())?)?;
    assert_eq!(resource.core.name.as_deref(), Some("Bar "));
    assert_eq!(resource.core.timer_call.as_deref(), Some("Foo "));

    let definition = lc_engine::Definition::from_resource(&resource)?;
    assert!(definition.has_function("Foo"));
    assert!(!definition.has_function("Foo "));
    let mut engine = Engine::new();
    engine.register_definition(definition)?;
    let object = engine.spawn_object(SpawnConfig::new("RCTA"))?;
    engine.tick_without_snapshot()?;

    let snapshot = engine
        .object_snapshot(object)
        .expect("timer object survives");
    assert_eq!(snapshot.timer, 0, "Timer=1 reached the callback gate");
    assert_eq!(snapshot.local_vars.get("iFired"), None);
    Ok(())
}

fn write_definition_graphics(path: &std::path::Path) -> Result<(), image::ImageError> {
    RgbaImage::from_pixel(1, 1, Rgba([1, 2, 3, 255])).save(path.join("Graphics.png"))
}

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
    write_definition_graphics(&definition_dir)?;

    let resolver = LocalDefinitionResolver;
    let scenario = Scenario::load_from_path_with(scenario_dir, &resolver)?;
    assert!(scenario.has_initial_objects());

    let mut visited = Vec::new();
    scenario.visit_definition_groups(|id, _| visited.push(id.to_string()));
    assert_eq!(visited, vec!["TEST"]);

    let mut engine = Engine::new();
    scenario.apply(&mut engine)?;

    let landscape = engine.landscape().expect("legacy Map.bmp should load");
    // No MapZoom key → the C4S default of 10 (C4Scenario.cpp:307,353):
    // the rendered map is 40x40 and ground starts at map row 1 → y=10;
    // C4Landscape::Init pads both dimensions to the 100px minimum.
    assert_eq!((landscape.width(), landscape.estimated_height()), (100, 100));
    assert_eq!(landscape.surface_height(0), Some(10));
    assert_eq!(landscape.surface_height(39), Some(10));
    assert_eq!(landscape.surface_height(40), Some(100), "right padding is sky");

    assert!(engine.definition_ids().any(|id| id == "TEST"));
    let snapshot = engine.snapshot();
    let spawned = snapshot
        .object(ObjectId::new(100))
        .expect("object from Objects.txt");
    assert_eq!(spawned.definition_id, "TEST");
    assert_eq!(spawned.position, Vector2::new(50, 60));

    Ok(())
}

#[test]
fn legacy_scenario_landscape_insert_thrust_zero_controls_script_insert_material(
) -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let scenario_dir = temp.path();

    // A minimal Map.bmp lets the legacy scenario complete landscape init;
    // the focused pixel plane is installed after apply so this test isolates
    // the realism-key handoff and the scenario-script InsertMaterial fold.
    let mut map = RgbaImage::new(2, 2);
    for y in 0..2 {
        for x in 0..2 {
            map.put_pixel(
                x,
                y,
                if y == 0 {
                    Rgba([12, 34, 56, 255])
                } else {
                    Rgba([160, 120, 80, 255])
                },
            );
        }
    }
    map.save(scenario_dir.join("Map.bmp"))?;
    let definition_dir = scenario_dir.join("Objects.ocd");
    fs::create_dir_all(&definition_dir)?;
    fs::write(
        definition_dir.join("DefCore.txt"),
        "[DefCore]\nid=DUMY\nName=Dummy\nCategory=C4D_Object\n",
    )?;
    fs::write(definition_dir.join("Script.c"), BASIC_SCRIPT)?;
    write_definition_graphics(&definition_dir)?;
    fs::write(
        scenario_dir.join("Scenario.txt"),
        "[Head]\nTitle=Insert thrust off\n\n[Definitions]\nDefinition1=Objects.ocd\n\n[Game]\nLandscapeInsertThrust=0\n\n[Landscape]\nMapZoom=10\n",
    )?;
    fs::write(
        scenario_dir.join("Script.c"),
        r#"
        #strict 2
        func ProbeInsert()
        {
            return InsertMaterial(Material("Source"), 3, 5);
        }
        "#,
    )?;

    let scenario = Scenario::load_from_path_with(scenario_dir, &LocalDefinitionResolver)?;
    let mut engine = Engine::with_seed(25);
    // Prove Scenario::apply actively installs zero; relying on Engine's
    // false default would let a missing key handoff pass this regression.
    engine.set_landscape_insert_thrust(true);
    scenario.apply(&mut engine)?;

    let library = lc_resources::MaterialLibrary::parse(
        r#"
        [Material Source]
        Name=Source
        Density=50
        MaxSlide=0

        [Material Old]
        Name=Old
        Density=25
        MaxSlide=0

        [Material Support]
        Name=Support
        Density=100
        MaxSlide=0
        "#,
    )?;
    // This path also invalidates the script host's shared material table;
    // set_materials alone is intended for pre-script synthetic fixtures.
    engine.configure_materials_from_library(&library);
    let source = engine.materials().id_of("Source").expect("Source exists");
    let support = engine
        .materials()
        .id_of("Support")
        .expect("Support exists");

    let mut densities = vec![0i32; 128];
    densities[10] = 50;
    densities[20] = 25;
    densities[30] = 100;
    let mut names: Vec<Option<String>> = vec![None; 128];
    names[10] = Some("Source".into());
    names[20] = Some("Old".into());
    names[30] = Some("Support".into());
    let mut bytes = vec![0u8; 7 * 10];
    bytes[5 * 7 + 3] = 20;
    bytes[6 * 7 + 3] = 30;
    let grid = lc_engine::landscape::PixelGrid::new(
        7,
        10,
        bytes,
        densities,
        names,
        vec![None; 128],
    );
    let mut landscape = Landscape::new(7, vec![10; 7])?;
    landscape.set_world_height(10);
    landscape.set_pixel_grid(grid);
    engine.set_landscape(landscape);

    engine.call_scenario_script_function("ProbeInsert", Vec::new())?;
    let landscape = engine.landscape().expect("landscape remains set");
    assert_eq!(landscape.material_at(3, 5), Some(source));
    assert_eq!(landscape.grid_byte_at(3, 5), Some(10));
    assert_eq!(
        landscape.material_at(3, 4),
        None,
        "the scenario's LandscapeInsertThrust=0 suppresses displaced Old"
    );
    assert_eq!(landscape.grid_byte_at(3, 4), Some(0));
    assert_eq!(landscape.material_at(3, 6), Some(support));
    assert_eq!(engine.pxs_system.count(), 0);
    Ok(())
}

#[test]
fn fresh_resource_object_keeps_dormant_defcore_vertex_attributes(
) -> Result<(), Box<dyn std::error::Error>> {
    // C4Object::Init assigns the complete C4Def::Shape, including all 30
    // fixed slots (src/C4Object.cpp:201-207). AddVertex then overwrites only
    // X/Y at VtxNum, exposing the dormant definition CNAT/friction unchanged
    // (src/C4Shape.cpp:26-31).
    let temp = tempdir()?;
    let definition_dir = temp.path().join("Objects.ocd");
    fs::create_dir(&definition_dir)?;
    fs::write(
        definition_dir.join("DefCore.txt"),
        "[DefCore]\nid=DVTX\nCategory=C4D_Object\nVertices=1\n\
         VertexX=3,30\nVertexY=4,40\nVertexCNAT=8,10\n\
         VertexFriction=100,250\n",
    )?;
    fs::write(
        definition_dir.join("Script.c"),
        "#strict\nfunc Initialize() { AddVertex(70, 80); return(1); }\n",
    )?;
    write_definition_graphics(&definition_dir)?;

    let group = Group::open(&definition_dir)?;
    let resource = lc_resources::definition::Definition::load(&group)?;
    let definition = lc_engine::Definition::from_resource(&resource)?;
    let mut engine = Engine::new();
    engine.register_definition(definition)?;
    let object = engine.spawn_object(lc_engine::SpawnConfig::new("DVTX"))?;
    let snapshot = engine.object_snapshot(object).expect("fresh DVTX survives");

    assert_eq!(snapshot.vertices.len(), 2);
    assert_eq!(
        (
            snapshot.vertices[1].x,
            snapshot.vertices[1].y,
            snapshot.vertices[1].cnat,
            snapshot.vertices[1].friction,
        ),
        (70, 80, 10, 250),
    );

    // The legacy Scenario path first projects resource cores through its
    // private ScenarioDefinition model; it must not collapse the fixed slots
    // back to the active prefix at that seam.
    fs::write(
        temp.path().join("Scenario.txt"),
        "[Head]\nTitle=Dormant Vertex Slots\n\n[Definitions]\nDefinition1=Objects.ocd\n",
    )?;
    let scenario = Scenario::load_from_path_with(temp.path(), &LocalDefinitionResolver)?;
    let mut scenario_engine = Engine::new();
    scenario.apply(&mut scenario_engine)?;
    let scenario_object = scenario_engine.spawn_object(lc_engine::SpawnConfig::new("DVTX"))?;
    let scenario_snapshot = scenario_engine
        .object_snapshot(scenario_object)
        .expect("scenario DVTX survives");
    assert_eq!(
        (
            scenario_snapshot.vertices[1].x,
            scenario_snapshot.vertices[1].y,
            scenario_snapshot.vertices[1].cnat,
            scenario_snapshot.vertices[1].friction,
        ),
        (70, 80, 10, 250),
    );
    Ok(())
}

#[test]
fn dormant_defcore_vertex_attributes_survive_save_before_add(
) -> Result<(), Box<dyn std::error::Error>> {
    // C4Shape::CompileFunc serializes all fixed arrays, not just VtxNum
    // (src/C4Shape.cpp:496-509). Saving before a later AddVertex therefore
    // cannot discard dormant definition attributes.
    let temp = tempdir()?;
    fs::write(
        temp.path().join("DefCore.txt"),
        "[DefCore]\nid=DVTS\nCategory=C4D_Object\nTimer=1\nTimerCall=Timer\n\
         Vertices=1\nVertexX=3,30\nVertexY=4,40\nVertexCNAT=8,10\n\
         VertexFriction=100,250\n",
    )?;
    fs::write(
        temp.path().join("Script.c"),
        "#strict\nfunc Timer() { if (GetVertexNum() == 1) AddVertex(70, 80); return(1); }\n",
    )?;

    let group = Group::open(temp.path())?;
    let resource = lc_resources::definition::Definition::load(&group)?;
    let definition = lc_engine::Definition::from_resource(&resource)?;
    let mut engine = Engine::new();
    engine.register_definition(definition.clone())?;
    let object = engine.spawn_object(lc_engine::SpawnConfig::new("DVTS"))?;
    assert_eq!(engine.object_snapshot(object).unwrap().vertices.len(), 1);

    let json = engine.capture_state().to_json_string()?;
    let state = lc_engine::EngineState::from_json_str(&json)?;
    let mut restored = Engine::new();
    restored.register_definition(definition)?;
    restored.restore_state(&state)?;
    restored.tick_without_snapshot()?;

    let snapshot = restored
        .object_snapshot(object)
        .expect("restored DVTS survives");
    assert_eq!(snapshot.vertices.len(), 2);
    assert_eq!(
        (snapshot.vertices[1].cnat, snapshot.vertices[1].friction),
        (10, 250),
    );
    Ok(())
}
