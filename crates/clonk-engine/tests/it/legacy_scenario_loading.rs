use crate::support::EngineTestExt;
use std::fs;

use clonk_engine::effect::EffectVarValue;
use clonk_engine::scenario::LegacyDefinitionResolver;
use clonk_engine::{
    Engine, Landscape, ObjectId, ObjectStatus, Scenario, ScenarioError, SpawnConfig, Vector2,
};
use clonk_resources::Group;
use clonk_script::Value;
use image::{Rgba, RgbaImage};

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

    let resource = clonk_resources::definition::Definition::load(&Group::open(temp.path())?)?;
    assert_eq!(resource.core.name.as_deref(), Some("Bar "));
    assert_eq!(resource.core.timer_call.as_deref(), Some("Foo "));

    let definition = clonk_engine::Definition::from_resource(&resource)?;
    assert!(definition.has_function("Foo"));
    assert!(!definition.has_function("Foo "));
    let mut engine = Engine::new();
    engine.register_definition(definition)?;
    let object = engine.spawn_object(SpawnConfig::new("RCTA"))?;
    engine.tick_without_snapshot()?;

    let snapshot = engine.test_object_snapshot(object);
    assert_eq!(snapshot.timer, 0, "Timer=1 reached the callback gate");
    assert_eq!(snapshot.local_vars.get("iFired"), None);
    Ok(())
}

#[test]
fn empty_def_core_name_survives_both_engine_load_paths() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let scenario_dir = temp.path();

    for (directory, id, name_field) in [
        ("Empty.ocd", "EMTY", "Name=\n"),
        ("Missing.ocd", "MISS", ""),
    ] {
        let definition_dir = scenario_dir.join(directory);
        fs::create_dir(&definition_dir)?;
        fs::write(
            definition_dir.join("DefCore.txt"),
            format!("[DefCore]\nid={id}\n{name_field}Category=C4D_Object\n"),
        )?;
        fs::write(
            definition_dir.join("Script.c"),
            format!(
                "#strict\npublic func ProbeName() {{ return [GetName(), GetName(0, {id}), GetDefCoreVal(\"Name\", \"DefCore\", {id})]; }}\n"
            ),
        )?;
        write_definition_graphics(&definition_dir)?;
    }

    let empty_group = Group::open(scenario_dir.join("Empty.ocd"))?;
    let empty_resource = clonk_resources::definition::Definition::load(&empty_group)?;
    let missing_group = Group::open(scenario_dir.join("Missing.ocd"))?;
    let missing_resource = clonk_resources::definition::Definition::load(&missing_group)?;
    assert_eq!(empty_resource.core.name.as_deref(), Some(""));
    assert_eq!(missing_resource.core.name, None);

    let mut direct = Engine::new();
    direct.register_definition(clonk_engine::Definition::from_resource(&empty_resource)?)?;
    direct.register_definition(clonk_engine::Definition::from_resource(&missing_resource)?)?;

    let assert_names =
        |engine: &mut Engine, id: &str, expected: &str| -> Result<(), Box<dyn std::error::Error>> {
            assert_eq!(engine.definition_name(id), Some(expected));
            let object = engine.spawn_object(SpawnConfig::new(id))?;
            let index = engine.test_object_index(object);
            let expected = Value::String(expected.to_string().into());
            assert_eq!(
                engine.call_object_function(index, "ProbeName", Vec::new())?,
                Value::Array(vec![expected.clone(), expected.clone(), expected])
            );
            Ok(())
        };
    assert_names(&mut direct, "EMTY", "")?;
    assert_names(&mut direct, "MISS", "Undefined")?;

    fs::write(
        scenario_dir.join("Scenario.txt"),
        "[Head]\nTitle=Definition name defaults\n\n[Definitions]\nDefinition1=Empty.ocd\nDefinition2=Missing.ocd\n",
    )?;
    let scenario = Scenario::load_from_path_with(scenario_dir, &LocalDefinitionResolver)?;
    let mut legacy = Engine::new();
    scenario.apply(&mut legacy)?;
    assert_names(&mut legacy, "EMTY", "")?;
    assert_names(&mut legacy, "MISS", "Undefined")?;

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

    let landscape = crate::support::TestValueExt::test_value(engine.landscape());
    // No MapZoom key → the C4S default of 10 (C4Scenario.cpp:307,353):
    // the rendered map is 40x40 and ground starts at map row 1 → y=10;
    // C4Landscape::Init pads both dimensions to the 100px minimum.
    assert_eq!(
        (landscape.width(), landscape.estimated_height()),
        (100, 100)
    );
    assert_eq!(landscape.surface_height(0), Some(10));
    assert_eq!(landscape.surface_height(39), Some(10));
    assert_eq!(
        landscape.surface_height(40),
        Some(100),
        "right padding is sky"
    );

    assert!(engine.definition_ids().any(|id| id == "TEST"));
    let snapshot = engine.snapshot();
    let spawned = crate::support::TestValueExt::test_value(snapshot.object(ObjectId::new(100)));
    assert_eq!(spawned.definition_id, "TEST");
    assert_eq!(spawned.position, Vector2::new(50, 60));

    Ok(())
}

#[test]
fn legacy_objects_names_are_exact_case_like_cpp() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let scenario_dir = temp.path();
    let definition_dir = scenario_dir.join("Objects.ocd");
    fs::create_dir_all(&definition_dir)?;
    fs::write(
        definition_dir.join("DefCore.txt"),
        "[DefCore]\nid=CASE\nName=Case Probe\nCategory=16\nWidth=8\nHeight=8\n",
    )?;
    fs::write(definition_dir.join("Script.c"), "#strict\n")?;
    write_definition_graphics(&definition_dir)?;
    fs::write(
        scenario_dir.join("Scenario.txt"),
        "[Head]\nTitle=Exact Objects Names\nSaveGame=1\nNoInitialize=1\n\n\
         [Definitions]\nDefinition1=Objects.ocd\n",
    )?;
    fs::write(
        scenario_dir.join("Objects.txt"),
        concat!(
            // Only exact root [Object] sections are compiled.
            "[object]\n",
            "id=CASE\n",
            "Number=not-a-number\n\n",
            // Object names are exact too. These malformed/wrong values must
            // remain unused, including the deliberately lowercase-only id.
            "[Object]\n",
            "id=CASE\n",
            "Number=1\n",
            "Status=2\n",
            "ID=NOPE\n",
            "number=99\n",
            "owner=not-a-number\n",
            "category=not-a-number\n",
            "energy=not-a-number\n",
            "x=not-a-number\n",
            "PhysicalTemporary=1\n",
            "[physical]\n",
            "Energy=777\n\n",
            // An exact Physical scope does not accept wrong-case fields, and
            // a wrong-case Commands sibling is not attached to the object.
            "[Object]\n",
            "id=CASE\n",
            "Number=2\n",
            "Status=2\n",
            "PhysicalTemporary=1\n",
            "[Physical]\n",
            "energy=888\n",
            "changes=Energy=123\n",
            "[commands]\n",
            "Command1=$2,Wait,i0,0,0,0,0,0,0,0,0,0,0,0,0,wrong-section\n\n",
            // Exact Commands likewise requires exact, canonical CommandN.
            "[Object]\n",
            "id=CASE\n",
            "Number=3\n",
            "Status=2\n",
            "[Commands]\n",
            "command1=$2,Wait,i0,0,0,0,0,0,0,0,0,0,0,0,0,wrong-key\n",
            "Command01=$2,Wait,i0,0,0,0,0,0,0,0,0,0,0,0,0,wrong-index\n\n",
            // Correctly cased native names retain their parsed values across
            // the Object and both adjacent nested scopes.
            "[Object]\n",
            "id=CASE\n",
            "Number=4\n",
            "Status=2\n",
            "Category=17\n",
            "Energy=1234\n",
            "X=40\n",
            "Y=50\n",
            "PhysicalTemporary=1\n",
            "[Physical]\n",
            "Energy=777\n",
            "Changes=Energy=321\n",
            "[Commands]\n",
            "Command1=$2,Wait,i0,0,0,0,0,0,0,0,0,0,0,0,0,exact\n",
        ),
    )?;

    let scenario = Scenario::load_from_path_with(scenario_dir, &LocalDefinitionResolver)?;
    let mut engine = Engine::with_seed(0);
    scenario.apply(&mut engine)?;

    assert!(engine.object_snapshot(ObjectId::new(99)).is_none());
    let wrong_object_names = engine.test_object_snapshot(ObjectId::new(1));
    assert_eq!(wrong_object_names.category, 0);
    assert_eq!(wrong_object_names.energy, 0);
    assert_eq!(wrong_object_names.position, Vector2::new(0, 0));
    assert_eq!(
        wrong_object_names
            .temporary_physical
            .expect("the exact PhysicalTemporary flag still creates defaults")
            .energy,
        0,
        "wrong-case [physical] is not followed"
    );

    let wrong_nested_names = engine.test_object_snapshot(ObjectId::new(2));
    assert_eq!(
        wrong_nested_names
            .temporary_physical
            .expect("exact [Physical] creates temporary physicals")
            .energy,
        0,
        "wrong-case physical field is unused"
    );
    assert!(wrong_nested_names.physical_changes.is_empty());
    assert!(wrong_nested_names.command_stack.is_empty());

    assert!(engine
        .test_object_snapshot(ObjectId::new(3))
        .command_stack
        .is_empty());

    let exact = engine.test_object_snapshot(ObjectId::new(4));
    assert_eq!(exact.category, 17);
    assert_eq!(exact.energy, 1234);
    assert_eq!(exact.position, Vector2::new(40, 50));
    assert_eq!(
        exact
            .temporary_physical
            .expect("exact [Physical] values load")
            .energy,
        777
    );
    assert_eq!(exact.physical_changes, [("Energy".to_string(), 321)]);
    assert_eq!(exact.command_stack.command_names(), ["Wait"]);
    Ok(())
}

fn write_effect_restore_fixture(path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    let definition = path.join("Carrier.ocd");
    fs::create_dir_all(&definition)?;
    fs::write(
        definition.join("DefCore.txt"),
        "[DefCore]\nid=CARR\nName=Carrier\nCategory=16\nWidth=8\nHeight=8\n",
    )?;
    fs::write(
        definition.join("Script.c"),
        "#strict\n\
         local start_calls, timer_calls, timer_time;\n\
         func FxRestoredStart(pTarget, iNumber, fTemporary)\n\
         {\n\
             start_calls = 1;\n\
             return(1);\n\
         }\n\
         func FxRestoredTimer(pTarget, iNumber, iTime)\n\
         {\n\
             timer_calls = 1;\n\
             timer_time = iTime;\n\
             return(1);\n\
         }\n",
    )?;
    write_definition_graphics(&definition)?;
    fs::write(
        path.join("Scenario.txt"),
        "[Head]\nTitle=Effect restore\nSaveGame=1\nNoInitialize=1\n\n\
         [Definitions]\nDefinition1=Carrier.ocd\n",
    )?;
    let raw_id = clonk_script::c4_id_raw("CARR") as i32;
    fs::write(
        path.join("Objects.txt"),
        format!(
            "[Object]\n\
             id=CARR\nNumber=1\nStatus=1\nCategory=16\nX=20\nY=20\n\
             Effects=Later(3,200,40,0,0,NONE),\
             Restored(7,10,5,3,2,WRNG)[10;A0,A1000000002,i-7,b2,o2,O1000000002,I{raw_id},S0,a[4;i1,O2,S0,A0],m[2;i7=S0;O2=I{raw_id}]]\n\n\
             [Object]\n\
             id=CARR\nNumber=2\nStatus=1\nCategory=16\nX=30\nY=20\n"
        ),
    )?;
    fs::write(path.join("Strings.txt"), b"saved text\r\n")?;
    Ok(())
}

#[test]
fn legacy_objects_restore_effect_chain_and_variables() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    write_effect_restore_fixture(temp.path())?;
    let scenario = Scenario::load_from_path_with(temp.path(), &LocalDefinitionResolver)?;
    let mut engine = Engine::with_seed(0);
    scenario.apply(&mut engine)?;

    let effects = engine.test_object_snapshot(ObjectId::new(1)).effects;
    assert_eq!(
        effects
            .iter()
            .map(|effect| effect.name.as_str())
            .collect::<Vec<_>>(),
        ["Later", "Restored"],
        "compiled linked-list order is not priority-sorted"
    );
    assert_eq!(
        (
            effects[0].number,
            effects[0].priority,
            effects[0].timer,
            effects[0].interval,
            effects[0].command_target,
            effects[0].command_id.as_deref(),
        ),
        (3, 200, 40, 0, None, None)
    );
    assert_eq!(
        (
            effects[1].number,
            effects[1].priority,
            effects[1].timer,
            effects[1].interval,
            effects[1].command_target,
            effects[1].command_id.as_deref(),
        ),
        (7, 10, 5, 3, Some(2), Some("CARR")),
        "a resolved command object refreshes the stale saved definition ID"
    );
    assert!(effects.iter().all(|effect| effect.start_dispatched));

    let expected_map = clonk_script::ValueMap::from([
        (
            clonk_script::Value::Int(7),
            clonk_script::Value::String("saved text".into()),
        ),
        (
            clonk_script::Value::Object(2),
            clonk_script::Value::C4Id("CARR".to_string()),
        ),
    ]);
    assert_eq!(
        effects[1].vars,
        vec![
            EffectVarValue::Nil,
            EffectVarValue::Object(2),
            EffectVarValue::Int(-7),
            EffectVarValue::RawBool(2),
            EffectVarValue::Object(2),
            EffectVarValue::Object(2),
            EffectVarValue::C4Id("CARR".to_string()),
            EffectVarValue::String("saved text".into()),
            EffectVarValue::Array(vec![
                EffectVarValue::Int(1),
                EffectVarValue::Object(2),
                EffectVarValue::String("saved text".into()),
                EffectVarValue::Nil,
            ]),
            EffectVarValue::Proplist(expected_map),
        ]
    );

    let target = engine.test_object_snapshot(ObjectId::new(2));
    assert_eq!(
        target
            .local_vars
            .get("start_calls")
            .cloned()
            .unwrap_or(clonk_script::Value::Nil),
        clonk_script::Value::Nil,
        "compiled restoration binds callbacks without executing Start"
    );

    let mut saved = engine.capture_state();
    let saved_carrier = crate::support::TestValueExt::test_value(
        saved
            .objects
            .iter_mut()
            .find(|object| object.snapshot.id == ObjectId::new(1)),
    );
    saved_carrier.snapshot.effects[1].command_id = Some("WRNG".to_string());
    engine.restore_state(&saved)?;
    assert_eq!(
        engine.test_object_snapshot(ObjectId::new(1)).effects[1]
            .command_id
            .as_deref(),
        Some("CARR"),
        "state restoration runs the same callback-assignment pass"
    );

    engine.tick_without_snapshot()?;

    let effects = engine.test_object_snapshot(ObjectId::new(1)).effects;
    assert_eq!(
        effects
            .iter()
            .map(|effect| (effect.name.as_str(), effect.timer))
            .collect::<Vec<_>>(),
        [("Later", 41), ("Restored", 6)],
        "timers resume from the saved time in linked-list order"
    );
    let target = engine.test_object_snapshot(ObjectId::new(2));
    assert_eq!(
        target
            .local_vars
            .get("start_calls")
            .cloned()
            .unwrap_or(clonk_script::Value::Nil),
        clonk_script::Value::Nil
    );
    assert_eq!(
        target.local_vars.get("timer_calls"),
        Some(&clonk_script::Value::Int(1))
    );
    assert_eq!(
        target.local_vars.get("timer_time"),
        Some(&clonk_script::Value::Int(6))
    );
    Ok(())
}

#[test]
fn initial_network_global_effect_refreshes_resolved_target_id(
) -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    write_effect_restore_fixture(temp.path())?;
    let scenario = Scenario::load_from_path_with(temp.path(), &LocalDefinitionResolver)?;
    let game_data = clonk_engine::parse_initial_network_game_data(
        b"[Effects]\r\nGlobalEffects=Probe(9,100,4,0,2,WRNG)[1;O1]\r\n",
    );
    let mut engine = Engine::with_seed(0);
    scenario.apply_before_network_final_init_with_game_data(&mut engine, &game_data, None, None)?;

    let effects = engine.capture_state().global_effects;
    assert_eq!(effects.len(), 1);
    assert_eq!(effects[0].command_target, Some(2));
    assert_eq!(effects[0].command_id.as_deref(), Some("CARR"));
    assert_eq!(effects[0].vars, vec![EffectVarValue::Object(1)]);
    assert!(effects[0].start_dispatched);
    Ok(())
}

#[test]
fn scenario_section_effects_resolve_retained_object_references(
) -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let definition = temp.path().join("Carrier.ocd");
    fs::create_dir_all(&definition)?;
    fs::write(
        definition.join("DefCore.txt"),
        "[DefCore]\nid=CARR\nName=Carrier\nCategory=16\nWidth=8\nHeight=8\n",
    )?;
    fs::write(definition.join("Script.c"), "#strict\n")?;
    write_definition_graphics(&definition)?;
    fs::write(
        temp.path().join("Scenario.txt"),
        "[Head]\nTitle=Section effects\nNoInitialize=1\n\n\
         [Definitions]\nDefinition1=Carrier.ocd\n",
    )?;
    fs::write(
        temp.path().join("Script.c"),
        "#strict\nfunc Switch() { return LoadScenarioSection(\"Next\", 0); }\n",
    )?;
    fs::write(
        temp.path().join("Objects.txt"),
        "[Object]\nid=CARR\nNumber=42\nStatus=2\nX=10\nY=10\n",
    )?;
    let section = temp.path().join("SectNext.c4g");
    fs::create_dir_all(&section)?;
    fs::write(
        section.join("Objects.txt"),
        "[Object]\n\
         id=CARR\nNumber=500\nStatus=1\nX=20\nY=10\n\
         Effects=Probe(6,100,9,0,42,WRNG)[3;O42,a[1;O42],m[1;i1=O42]]\n",
    )?;

    let scenario = Scenario::load_from_path_with(temp.path(), &LocalDefinitionResolver)?;
    let mut engine = Engine::with_seed(0);
    scenario.apply(&mut engine)?;
    assert_eq!(
        engine.test_object_snapshot(ObjectId::new(42)).status,
        ObjectStatus::Inactive
    );

    engine.call_scenario_script_function("Switch", Vec::new())?;

    assert_eq!(engine.debug_current_scenario_section(), "Next");
    let mut effects = engine.test_object_snapshot(ObjectId::new(500)).effects;
    let effect = effects.remove(0);
    assert_eq!(effect.command_target, Some(42));
    assert_eq!(effect.command_id.as_deref(), Some("CARR"));
    assert_eq!(
        effect.vars,
        vec![
            EffectVarValue::Object(42),
            EffectVarValue::Array(vec![EffectVarValue::Object(42)]),
            EffectVarValue::Proplist(clonk_script::ValueMap::from([(
                clonk_script::Value::Int(1),
                clonk_script::Value::Object(42),
            )])),
        ],
        "retained inactive objects participate in recursive denumeration"
    );
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

    let library = clonk_resources::MaterialLibrary::parse(
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
    let source = crate::support::TestValueExt::test_value(engine.materials().id_of("Source"));
    let support = crate::support::TestValueExt::test_value(engine.materials().id_of("Support"));

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
    let grid =
        clonk_engine::landscape::PixelGrid::new(7, 10, bytes, densities, names, vec![None; 128]);
    let mut landscape = Landscape::new(7, vec![10; 7])?;
    landscape.set_world_height(10);
    landscape.set_pixel_grid(grid);
    engine.set_landscape(landscape);

    engine.call_scenario_script_function("ProbeInsert", Vec::new())?;
    let landscape = crate::support::TestValueExt::test_value(engine.landscape());
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
    let resource = clonk_resources::definition::Definition::load(&group)?;
    let definition = clonk_engine::Definition::from_resource(&resource)?;
    let mut engine = Engine::new();
    engine.register_definition(definition)?;
    let object = engine.spawn_object(clonk_engine::SpawnConfig::new("DVTX"))?;
    let snapshot = engine.test_object_snapshot(object);

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
    let scenario_object = scenario_engine.spawn_object(clonk_engine::SpawnConfig::new("DVTX"))?;
    let scenario_snapshot = scenario_engine.test_object_snapshot(scenario_object);
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
    let resource = clonk_resources::definition::Definition::load(&group)?;
    let definition = clonk_engine::Definition::from_resource(&resource)?;
    let mut engine = Engine::new();
    engine.register_definition(definition.clone())?;
    let object = engine.spawn_object(clonk_engine::SpawnConfig::new("DVTS"))?;
    assert_eq!(engine.object_snapshot(object).unwrap().vertices.len(), 1);

    let json = engine.capture_state().to_json_string()?;
    let state = clonk_engine::EngineState::from_json_str(&json)?;
    let mut restored = Engine::new();
    restored.register_definition(definition)?;
    restored.restore_state(&state)?;
    restored.tick_without_snapshot()?;

    let snapshot = restored.test_object_snapshot(object);
    assert_eq!(snapshot.vertices.len(), 2);
    assert_eq!(
        (snapshot.vertices[1].cnat, snapshot.vertices[1].friction),
        (10, 250),
    );
    Ok(())
}
