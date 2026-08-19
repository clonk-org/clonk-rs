use super::*;
use crate::landscape::PixelGrid;

fn control(id: &[u8; 4], x: i32, y: i32) -> EmDropDefControlData {
    EmDropDefControlData {
        id: *id,
        x,
        y,
        by_client: 7,
    }
}

#[test]
fn structures_use_full_create_construction_with_terrain_adjustment() {
    let library = crate::TestValueExt::test_value(clonk_resources::MaterialLibrary::parse(
        "[Material Earth]\nName=Earth\nDensity=100\nDigFree=1\n\n\
             [Material Granite]\nName=Granite\nDensity=100\nDigFree=0\n",
    ));
    let materials = MaterialSet::from_resource_library(&library);
    let mut landscape = crate::TestValueExt::test_value(Landscape::new(40, vec![0; 40]));
    landscape.set_world_height(40);
    landscape.set_pixel_grid(PixelGrid::new(
        40,
        40,
        vec![1; 40 * 40],
        vec![0, 100, 100],
        vec![None, Some("Earth".to_owned()), Some("Granite".to_owned())],
        vec![None; 3],
    ));

    let mut engine = Engine::new();
    engine.set_materials(materials);
    engine.set_landscape(landscape);
    let mut structure = test_definition("HUT1", "Hut", "#strict\n");
    structure.set_category(CATEGORY_STRUCTURE);
    structure.set_shape_rect(Some(DefinitionRect::new(-4, -8, 8, 8)));
    structure.set_basement(1);
    crate::TestValueExt::test_value(engine.register_definition(structure));
    let mut oversize = test_definition("OVER", "Oversize", "#strict\n");
    oversize.set_category(CATEGORY_STRUCTURE);
    oversize.set_oversize(true);
    crate::TestValueExt::test_value(engine.register_definition(oversize));

    assert!(engine
        .execute_em_drop_def_control(&control(b"HUT1", 20, 30))
        .expect("drop executes"));
    let object = crate::TestValueExt::test_value(
        engine
            .first_active_object_for_definition("HUT1")
            .and_then(|id| engine.object_snapshot(id)),
    );
    assert_eq!(object.owner, OWNER_NONE);
    assert_eq!(object.construction, FULL_CON);
    assert_eq!(engine.debug_landscape_material_name(20, 26), None);
    assert_eq!(
        engine.debug_landscape_material_name(20, 30).as_deref(),
        Some("Granite")
    );

    assert!(engine
        .execute_em_drop_def_control(&control(b"OVER", 4, 5))
        .expect("oversize drop executes"));
    let oversize = crate::TestValueExt::test_value(
        engine
            .first_active_object_for_definition("OVER")
            .and_then(|id| engine.object_snapshot(id)),
    );
    assert_eq!(
        oversize.construction,
        FULL_CON * (FULL_CON / 100),
        "the literal FullCon percentage survives Oversize DoCon"
    );
}

#[test]
fn nonstructures_use_create_object_and_invalid_drops_are_noops() {
    let mut engine = Engine::new();
    let mut item = test_definition("ITEM", "Item", "#strict\n");
    item.set_category(CATEGORY_OBJECT);
    crate::TestValueExt::test_value(engine.register_definition(item));
    let mut numeric_underscore = test_definition("1_AA", "Edge ID", "#strict\n");
    numeric_underscore.set_category(CATEGORY_OBJECT);
    crate::TestValueExt::test_value(engine.register_definition(numeric_underscore));

    assert!(!engine
        .execute_em_drop_def_control(&EmDropDefControlData::default())
        .expect("C4ID_None is ignored"));
    assert!(!engine
        .execute_em_drop_def_control(&control(b"MISS", 1, 2))
        .expect("unknown definition is ignored"));
    assert!(engine.first_active_object_for_definition("ITEM").is_none());

    engine.set_league_game(true);
    assert!(!engine
        .execute_em_drop_def_control(&control(b"ITEM", 7, 9))
        .expect("league drop is ignored"));
    assert!(engine.first_active_object_for_definition("ITEM").is_none());

    engine.set_league_game(false);
    assert!(engine
        .execute_em_drop_def_control(&control(b"ITEM", 7, 9))
        .expect("ordinary drop executes"));
    let object = crate::TestValueExt::test_value(
        engine
            .first_active_object_for_definition("ITEM")
            .and_then(|id| engine.object_snapshot(id)),
    );
    assert_eq!(object.position, Vector2::new(7, 9));
    assert_eq!(object.owner, OWNER_NONE);
    assert_eq!(object.construction, FULL_CON);

    assert!(engine
        .execute_em_drop_def_control(&control(b"1_AA", i32::MIN, 11))
        .expect("numeric-underscore ID and INT_MIN coordinate execute"));
    let edge = crate::TestValueExt::test_value(
        engine
            .first_active_object_for_definition("1_AA")
            .and_then(|id| engine.object_snapshot(id)),
    );
    assert_eq!(edge.position, Vector2::new(i32::MIN, 11));
}

/// `C4Game::DropFile` loads a dropped `.c4d` whose id the engine has never
/// seen and then looks the id up a *second* time
/// (`Defs.Load(szFilename, C4D_Load_RX, …) && (cdef = C4Id2Def(c_id))`,
/// `C4Game.cpp:1647-1651`). Without a runtime loader the port could only
/// resolve ids already in the loaded set, so a definition dropped from outside
/// it reported `IDS_CNS_DROPNODEF` — the arm C++ takes only when its own load
/// fails.
#[test]
fn a_definition_from_outside_the_loaded_set_loads_from_its_own_group() {
    let root = tempfile::tempdir().expect("temporary definition group");
    let group = root.path().join("Dropped.c4d");
    std::fs::create_dir(&group).expect("definition group directory");
    std::fs::write(
        group.join("DefCore.txt"),
        "[DefCore]\nid=DRPD\nVersion=4,9,8\nName=Dropped\nWidth=8\nHeight=8\n",
    )
    .expect("definition core");

    let mut engine = Engine::new();
    assert!(
        engine.definition("DRPD").is_none(),
        "the id is not in the loaded set to begin with, which is the case that used to fail"
    );

    assert!(
        engine.load_definition_from_path(&group),
        "the dropped group loads"
    );
    assert!(
        engine.definition("DRPD").is_some(),
        "and the second lookup C++ performs now resolves"
    );
}

/// The failure arm has to stay reachable: `IDS_CNS_DROPNODEF` is still what a
/// genuinely unloadable group reports.
#[test]
fn a_group_that_cannot_be_loaded_still_reports_no_definition() {
    let root = tempfile::tempdir().expect("temporary definition group");
    let group = root.path().join("Broken.c4d");
    std::fs::create_dir(&group).expect("definition group directory");
    // No `DefCore.txt` at all, so there is nothing to load an id from.

    let mut engine = Engine::new();
    assert!(
        !engine.load_definition_from_path(&group),
        "a group with no definition core does not load"
    );
    assert!(engine.definition("DRPD").is_none());
}
