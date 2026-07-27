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
    let library = clonk_resources::MaterialLibrary::parse(
        "[Material Earth]\nName=Earth\nDensity=100\nDigFree=1\n\n\
             [Material Granite]\nName=Granite\nDensity=100\nDigFree=0\n",
    )
    .expect("materials parse");
    let materials = MaterialSet::from_resource_library(&library);
    let mut landscape = Landscape::new(40, vec![0; 40]).expect("landscape builds");
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
    let mut structure =
        Definition::from_script("HUT1", "Hut", "#strict\n").expect("structure compiles");
    structure.set_category(CATEGORY_STRUCTURE);
    structure.set_shape_rect(Some(DefinitionRect::new(-4, -8, 8, 8)));
    structure.set_basement(1);
    engine
        .register_definition(structure)
        .expect("structure registers");
    let mut oversize = Definition::from_script("OVER", "Oversize", "#strict\n")
        .expect("oversize structure compiles");
    oversize.set_category(CATEGORY_STRUCTURE);
    oversize.set_oversize(true);
    engine
        .register_definition(oversize)
        .expect("oversize structure registers");

    assert!(engine
        .execute_em_drop_def_control(&control(b"HUT1", 20, 30))
        .expect("drop executes"));
    let object = engine
        .first_active_object_for_definition("HUT1")
        .and_then(|id| engine.object_snapshot(id))
        .expect("structure is created");
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
    let oversize = engine
        .first_active_object_for_definition("OVER")
        .and_then(|id| engine.object_snapshot(id))
        .expect("oversize structure is created");
    assert_eq!(
        oversize.construction,
        FULL_CON * (FULL_CON / 100),
        "the literal FullCon percentage survives Oversize DoCon"
    );
}

#[test]
fn nonstructures_use_create_object_and_invalid_drops_are_noops() {
    let mut engine = Engine::new();
    let mut item = Definition::from_script("ITEM", "Item", "#strict\n").expect("item compiles");
    item.set_category(CATEGORY_OBJECT);
    engine.register_definition(item).expect("item registers");
    let mut numeric_underscore =
        Definition::from_script("1_AA", "Edge ID", "#strict\n").expect("edge ID compiles");
    numeric_underscore.set_category(CATEGORY_OBJECT);
    engine
        .register_definition(numeric_underscore)
        .expect("edge ID registers");

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
    let object = engine
        .first_active_object_for_definition("ITEM")
        .and_then(|id| engine.object_snapshot(id))
        .expect("item is created");
    assert_eq!(object.position, Vector2::new(7, 9));
    assert_eq!(object.owner, OWNER_NONE);
    assert_eq!(object.construction, FULL_CON);

    assert!(engine
        .execute_em_drop_def_control(&control(b"1_AA", i32::MIN, 11))
        .expect("numeric-underscore ID and INT_MIN coordinate execute"));
    let edge = engine
        .first_active_object_for_definition("1_AA")
        .and_then(|id| engine.object_snapshot(id))
        .expect("edge object is created");
    assert_eq!(edge.position, Vector2::new(i32::MIN, 11));
}
