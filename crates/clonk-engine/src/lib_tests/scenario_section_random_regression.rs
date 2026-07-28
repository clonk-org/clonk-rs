use super::*;
use crate::landscape::{
    LandscapeRasterState, RuntimeTexMapLookup, RuntimeTexMapMaterial, RuntimeTexMapState,
};

fn section(name: &str, width: u32, base_extinguish_enabled: bool) -> scenario::ScenarioSectionSpec {
    scenario::ScenarioSectionSpec {
        name: name.to_string(),
        source_group: None,
        landscape: Some(vehicle_section_landscape(width, 40)),
        landscape_systems: scenario::ScenarioLandscapeSystems::default(),
        exact_landscape: false,
        texmap_lookups: Vec::new(),
        resynthesize_static_map: false,
        map_creator: None,
        s2_overload: None,
        gravity: scenario::LegacyC4SVal::new(100, 0, 10, 200),
        post_init_map_callbacks: map_creator_s2::PostInitMapCallbacks::default(),
        keep_map_creator: false,
        no_initialize: false,
        objects: Vec::new(),
        scenario_values: scenario::ScenarioValueStore::default(),
        environment: EnvironmentSettings::default(),
        base_reject_entrance_enabled: true,
        base_extinguish_enabled,
    }
}

fn vehicle_section_landscape(width: u32, height: u32) -> Landscape {
    let mut landscape =
        Landscape::new(width, vec![0; width as usize]).expect("section landscape builds");
    landscape.set_world_height(height as i32);
    landscape.set_pixel_grid(landscape::PixelGrid::new(
        width,
        height,
        vec![0; (width * height) as usize],
        vec![0, 100, 100],
        vec![None, Some("Earth".into()), Some("Vehicle".into())],
        vec![None; 3],
    ));
    landscape
}

fn vehicle_section(name: &str, landscape: Landscape) -> scenario::ScenarioSectionSpec {
    scenario::ScenarioSectionSpec {
        name: name.to_string(),
        source_group: None,
        landscape: Some(landscape),
        landscape_systems: scenario::ScenarioLandscapeSystems::default(),
        exact_landscape: false,
        texmap_lookups: Vec::new(),
        resynthesize_static_map: false,
        map_creator: None,
        s2_overload: None,
        gravity: scenario::LegacyC4SVal::new(100, 0, 10, 200),
        post_init_map_callbacks: map_creator_s2::PostInitMapCallbacks::default(),
        keep_map_creator: false,
        no_initialize: false,
        objects: Vec::new(),
        scenario_values: scenario::ScenarioValueStore::default(),
        environment: EnvironmentSettings::default(),
        base_reject_entrance_enabled: true,
        base_extinguish_enabled: true,
    }
}

fn two_pixel_solid_mask_definition(id: &str, second_alpha: u8) -> Definition {
    let mut definition =
        Definition::from_script(id, "Capture mask", "").expect("solid-mask definition compiles");
    definition.set_shape_rect(Some(DefinitionRect::new(0, 0, 2, 1)));
    definition.set_solid_mask(Some(DefinitionTargetRect::new(0, 0, 2, 1, 0, 0)));
    definition.set_sprite_image(Some(DefinitionSpriteImage {
        width: 2,
        height: 1,
        pixels: Arc::from([0, 0, 0, 255, 0, 0, 0, second_alpha]),
        color_mask: None,
    }));
    definition
}

fn resumed_non_main_root_engine() -> Engine {
    let mut engine = Engine::with_seed(5);
    engine.configure_scenario_sections(&[section("Cave", 80, true), section("Main", 120, true)]);
    engine.set_landscape(vehicle_section_landscape(80, 40));
    engine
        .apply_initial_network_game_data(&InitialNetworkGameData {
            current_scenario_section: "Cave".to_string(),
            ..InitialNetworkGameData::default()
        })
        .expect("runtime-join section state applies");
    engine
}

#[test]
fn synthetic_section_without_a_group_uses_its_explicit_object_fallback() {
    let mut next = section("next", 120, true);
    next.objects.push(scenario::ScenarioSpawn {
        handle: Some("900".to_string()),
        container_handle: None,
        contents_handles: Vec::new(),
        info_name: None,
        config: SpawnConfig::new("SYNO")
            .with_id(ObjectId::new(900))
            .with_position(Vector2::new(12, 34))
            .with_loaded(true),
    });

    let mut engine = Engine::with_seed(3);
    engine
        .register_script_definition("SYNO", "Synthetic object", "")
        .expect("synthetic definition registers");
    engine.configure_scenario_sections(&[section("main", 80, true), next]);
    engine.set_landscape(vehicle_section_landscape(80, 40));

    assert!(engine
        .load_scenario_section("next", 0, Vec::new())
        .expect("synthetic section loads"));
    let object = engine
        .object_snapshot(ObjectId::new(900))
        .expect("explicit fallback object loads");
    assert_eq!(object.definition_id, "SYNO");
    assert_eq!(object.position, Vector2::new(12, 34));

    assert!(engine
        .load_scenario_section("main", 1, Vec::new())
        .expect("synthetic section landscape freezes"));
    assert!(engine
        .load_scenario_section("next", 0, Vec::new())
        .expect("synthetic landscape-only section reopens"));
    assert!(
        engine.object_snapshot(ObjectId::new(900)).is_some(),
        "a groupless landscape-only freeze retains explicit object templates"
    );
}

#[test]
fn resumed_non_main_implicit_root_cannot_reopen_after_unsaved_departure() {
    let mut engine = resumed_non_main_root_engine();

    assert!(engine
        .load_scenario_section("Main", 0, Vec::new())
        .expect("named Main section loads"));
    assert_eq!(engine.debug_current_scenario_section(), "Main");
    assert!(
        !engine
            .load_scenario_section("Cave", 0, Vec::new())
            .expect("missing native group is an ordinary false result"),
        "an implicit non-main root has no Filename for GetGroupfile"
    );
    assert_eq!(engine.debug_current_scenario_section(), "Main");
    assert_eq!(engine.landscape().map(Landscape::width), Some(120));
}

#[test]
fn resumed_non_main_implicit_root_reopens_after_saved_departure() {
    let mut engine = resumed_non_main_root_engine();

    assert!(engine
        .load_scenario_section("Main", 3, Vec::new())
        .expect("named Main section loads after freezing Cave"));
    assert!(
        engine
            .scenario_sections
            .get("cave")
            .is_some_and(|section| section.frozen_group.is_some()),
        "C4S_SAVE_LANDSCAPE | C4S_SAVE_OBJECTS creates the temp group"
    );
    assert!(engine
        .load_scenario_section("Cave", 0, Vec::new())
        .expect("frozen Cave section reloads"));
    assert_eq!(engine.debug_current_scenario_section(), "Cave");
    assert_eq!(engine.landscape().map(Landscape::width), Some(80));
}

#[test]
fn section_object_save_enumerates_active_and_inactive_compiler_caches() {
    // Objects.Save(false, false) writes active objects only, but native
    // Enumerate/Denumerate still visits the inactive list on both sides
    // of decompilation (C4GameObjects.cpp:691-713).
    let mut engine = Engine::with_seed(11);
    engine
        .register_definition(Definition::from_script("ITEM", "Item", "").expect("item compiles"))
        .expect("item registers");
    engine.configure_scenario_sections(&[section("main", 80, true), section("next", 100, true)]);
    engine.set_landscape(vehicle_section_landscape(80, 40));

    let active = engine
        .spawn_object(SpawnConfig::new("ITEM"))
        .expect("active object spawns");
    let inactive = engine
        .spawn_object(SpawnConfig::new("ITEM").with_status(ObjectStatus::Inactive))
        .expect("inactive object spawns");
    let preserved = engine
        .spawn_object(SpawnConfig::new("ITEM"))
        .expect("preserved object spawns");
    let inactive_number = i32::try_from(inactive.as_u64()).expect("small fixture id");
    let active_index = engine.find_object_index(active).expect("active index");
    engine.objects[active_index].state.action.target = Some(inactive);
    engine.objects[active_index].compiler_cache.action_target1 = 991;
    let inactive_index = engine.find_object_index(inactive).expect("inactive index");
    engine.objects[inactive_index].state.action.target = Some(inactive);
    engine.objects[inactive_index].compiler_cache.action_target1 = 992;
    let preserved_index = engine
        .find_object_index(preserved)
        .expect("preserved index");
    let off_list = ObjectId::new(999);
    engine.objects[preserved_index].state.action.target = Some(off_list);
    engine.objects[preserved_index].state.layer = Some(off_list);
    engine.objects[preserved_index]
        .compiler_cache
        .action_target1 = 993;
    engine.objects[preserved_index].compiler_cache.layer = 994;

    assert!(engine
        .load_scenario_section("next", 2, vec![inactive, preserved])
        .expect("departing objects freeze"));

    let frozen = engine
        .scenario_sections
        .get("main")
        .and_then(|section| section.frozen_group.clone())
        .expect("departing main section freezes");
    let group =
        clonk_resources::Group::from_raw_memory(std::path::PathBuf::from("Sectmain.c4g"), frozen)
            .expect("frozen section group opens");
    let objects_txt = group
        .read_entry_bytes("Objects.txt")
        .expect("frozen Objects.txt reads");
    assert!(
        String::from_utf8_lossy(&objects_txt)
            .contains(&format!("ActionTarget1={inactive_number}\r\n")),
        "active rows decompile the enumerated cache word",
    );

    let inactive_index = engine
        .find_object_index(inactive)
        .expect("inactive object survives section switch");
    assert_eq!(
        engine.objects[inactive_index].compiler_cache.action_target1, inactive_number,
        "inactive wrappers receive the same enumeration side effect",
    );
    assert_eq!(
        engine.objects[inactive_index].state.action.target,
        Some(inactive),
        "inactive wrapper denumerates through the shared number table",
    );
    let preserved_index = engine
        .find_object_index(preserved)
        .expect("preserved object survives section switch");
    assert_eq!(engine.objects[preserved_index].state.action.target, None);
    assert_eq!(engine.objects[preserved_index].state.layer, None);
    assert_eq!(
        engine.objects[preserved_index]
            .compiler_cache
            .action_target1,
        0,
    );
    assert_eq!(engine.objects[preserved_index].compiler_cache.layer, 0);
}

#[test]
fn section_landscape_init_refixes_the_exact_synced_rng_ledger() {
    let seed = 7;
    let mut engine = Engine::with_seed(seed);
    engine.configure_scenario_sections(&[section("main", 100, true), section("next", 240, false)]);
    engine.set_landscape(vehicle_section_landscape(100, 40));

    engine.rng.random(31);
    engine.rng.rnd3();
    let unknown_before = engine.rng.clone();
    assert!(!engine
        .load_scenario_section("missing", 0, Vec::new())
        .expect("unknown section is not an engine error"));
    assert_eq!(
        engine.rng, unknown_before,
        "the known-section gate precedes FixRandom"
    );

    engine.rng.random(17);
    engine.rng.rnd3();
    assert!(engine
        .load_scenario_section("next", 0, Vec::new())
        .expect("known section loads"));
    assert_eq!(engine.landscape().expect("section landscape").width(), 240);
    assert!(
        !engine.base_extinguish_enabled,
        "the target section projects its BaseFunctionality mask"
    );

    let mut expected = LcgRng::seed_from_u64(seed);
    let _ = expected.random(1);
    expected.trace = engine.rng.trace;
    assert_eq!(engine.rng.count, 501);
    assert_eq!(engine.rng.rnd3_ptr(), 0);
    assert_eq!(
        engine.rng, expected,
        "section ScenarioInit consumes gravity immediately after the second FixRandom"
    );
}

#[test]
fn section_load_preserves_runtime_landscape_state_and_matches_mode_quirk() {
    let mut main = Landscape::flat(8, 4);
    let mut live_texmap = RuntimeTexMapState::default();
    live_texmap.densities[1] = 100;
    live_texmap.material_names[1] = Some("Earth".to_owned());
    live_texmap.texture_names[1] = Some("Rough".to_owned());
    live_texmap.match_texture_names[1] = Some("Rough".to_owned());
    live_texmap.shapes[1] = Some(crate::chunky::ChunkShape::Flat);
    live_texmap.materials = vec![
        RuntimeTexMapMaterial {
            name: "Earth".to_owned(),
            density: 100,
            shape: crate::chunky::ChunkShape::Flat,
        },
        RuntimeTexMapMaterial {
            name: "Water".to_owned(),
            density: 25,
            shape: crate::chunky::ChunkShape::Flat,
        },
    ];
    live_texmap.texture_inventory =
        vec!["Rough".to_owned(), "Smooth".to_owned(), "Liquid".to_owned()];
    live_texmap.entries_added = true;
    main.set_raster_state(LandscapeRasterState::new(7, -7, live_texmap));
    main.set_modulation(0xaabb_ccdd);
    assert!(main.set_mode(LANDSCAPE_MODE_STATIC));
    let mut next = Landscape::flat(9, 4);
    let mut target_texmap = RuntimeTexMapState::default();
    target_texmap.densities[1] = 25;
    target_texmap.material_names[1] = Some("Water".to_owned());
    target_texmap.texture_names[1] = Some("Smooth".to_owned());
    target_texmap.match_texture_names[1] = Some("Smooth".to_owned());
    target_texmap.shapes[1] = Some(crate::chunky::ChunkShape::Flat);
    target_texmap.entries_added = true;
    next.set_pixel_grid(landscape::PixelGrid::new(
        9,
        4,
        [vec![1], vec![0; 35]].concat(),
        target_texmap.densities.clone(),
        target_texmap.material_names.clone(),
        target_texmap.texture_names.clone(),
    ));
    next.set_raster_state(LandscapeRasterState::new(1, 999, target_texmap.clone()));
    assert!(next.set_mode(LANDSCAPE_MODE_STATIC));
    let mut exact = Landscape::flat(10, 4);
    exact.set_raster_state(LandscapeRasterState::new(
        1,
        999,
        RuntimeTexMapState::default(),
    ));
    assert!(exact.set_mode(LANDSCAPE_MODE_EXACT));
    let mut exact_section = vehicle_section("exact", exact);
    exact_section.exact_landscape = true;
    let mut next_section = vehicle_section("next", next);
    next_section.texmap_lookups = vec![RuntimeTexMapLookup {
        material_texture: "Water-Smooth".to_owned(),
        default_texture: None,
        eager_index: 1,
    }];
    let mut raw_static = Landscape::flat(1, 1);
    raw_static.set_pixel_grid(landscape::PixelGrid::new(
        1,
        1,
        vec![1],
        target_texmap.densities.clone(),
        target_texmap.material_names.clone(),
        target_texmap.texture_names.clone(),
    ));
    let mut raw_state = LandscapeRasterState::new(1, 999, target_texmap);
    raw_state.set_map(&clonk_resources::bitmap::IndexedBitmap {
        width: 1,
        height: 1,
        indices: vec![1],
    });
    raw_static.set_raster_state(raw_state);
    raw_static.save_initial().unwrap();
    assert!(raw_static.set_mode(LANDSCAPE_MODE_STATIC));
    let mut raw_section = vehicle_section("raw", raw_static);
    raw_section.resynthesize_static_map = true;

    let mut engine = Engine::with_seed(17);
    engine.configure_scenario_sections(&[
        vehicle_section("main", main.clone()),
        next_section,
        exact_section,
        raw_section,
    ]);
    engine.set_landscape(main);
    assert!(!engine.landscape().unwrap().map_changed());

    assert!(engine
        .load_scenario_section("exact", 0, Vec::new())
        .expect("exact section loads"));
    let landscape = engine.landscape().unwrap();
    assert!(landscape.map_changed());
    assert_eq!(landscape.map_seed(), -7);
    assert_eq!(landscape.modulation(), 0xaabb_ccdd);
    assert!(landscape.texture_map_entries_added());
    assert_eq!(landscape.mode(), LANDSCAPE_MODE_UNDEFINED);
    assert_eq!(
        landscape.raster_state().unwrap().map_zoom(),
        7,
        "exact section overloads retain the live MapZoom"
    );
    assert_eq!(engine.physics().gravity, 100);

    assert!(engine
        .load_scenario_section("exact", 0, Vec::new())
        .expect("current exact section reloads"));
    assert_eq!(
        engine.landscape().unwrap().mode(),
        LANDSCAPE_MODE_EXACT,
        "an undefined pre-load mode permits the post-Clear exact assignment"
    );

    assert!(engine
        .load_scenario_section("next", 0, Vec::new())
        .expect("next section loads"));
    let landscape = engine.landscape().unwrap();
    assert!(landscape.map_changed());
    assert_eq!(landscape.map_seed(), -7);
    assert_eq!(landscape.modulation(), 0xaabb_ccdd);
    assert!(landscape.texture_map_entries_added());
    assert_eq!(landscape.mode(), LANDSCAPE_MODE_UNDEFINED);
    assert_eq!(landscape.grid_byte_at(0, 0), Some(2));
    let texmap = landscape.raster_state().unwrap().texmap();
    assert_eq!(texmap.material_names[1].as_deref(), Some("Earth"));
    assert_eq!(texmap.material_names[2].as_deref(), Some("Water"));
    assert_eq!(
        InitialNetworkGameData::from_engine(&engine)
            .expect("section runtime state captures")
            .current_scenario_section,
        "next"
    );

    assert!(engine
        .load_scenario_section("raw", 0, Vec::new())
        .expect("raw static section loads"));
    let landscape = engine.landscape().unwrap();
    assert_eq!(landscape.grid_byte_at(0, 0), Some(1));
    assert_eq!(
        landscape.raster_state().unwrap().texmap().material_names[1].as_deref(),
        Some("Earth"),
        "raw Map.bmp bytes are interpreted against the live global slot"
    );
}

#[test]
fn saved_section_exact_reload_reseeds_the_diff_baseline() {
    let mut main = vehicle_section_landscape(4, 4);
    main.save_initial().expect("base Surface8 captures");
    main.grid_write_byte(1, 1, 1);
    let next = vehicle_section_landscape(4, 4);

    let mut engine = Engine::with_seed(23);
    engine.configure_scenario_sections(&[
        vehicle_section("main", main.clone()),
        vehicle_section("next", next),
    ]);
    engine.set_landscape(main);

    assert!(engine
        .load_scenario_section("next", 1, Vec::new())
        .expect("main landscape saves while leaving"));
    assert!(engine
        .load_scenario_section("main", 0, Vec::new())
        .expect("saved exact main reloads"));
    assert_eq!(
        engine
            .landscape()
            .unwrap()
            .save_diff(false)
            .expect("saved exact landscape has pInitial"),
        None,
        "the just-saved full surface is the new diff baseline"
    );
}

#[test]
fn saved_section_freezes_changed_map_before_exact_reload_discards_it() {
    let mut texmap = RuntimeTexMapState::default();
    texmap.densities[1] = 100;
    texmap.material_names[1] = Some("Earth".to_owned());
    let mut raster = LandscapeRasterState::new(1, 7, texmap);
    raster.set_map(&clonk_resources::bitmap::IndexedBitmap {
        width: 2,
        height: 2,
        indices: vec![1; 4],
    });
    raster.set_map_changed();

    let mut main = vehicle_section_landscape(4, 4);
    main.set_raster_state(raster);
    let next = vehicle_section_landscape(4, 4);
    let mut engine = Engine::with_seed(24);
    engine.configure_scenario_sections(&[
        vehicle_section("main", main.clone()),
        vehicle_section("next", next),
    ]);
    engine.set_landscape(main);

    assert!(engine
        .load_scenario_section("next", 1, Vec::new())
        .expect("changed main landscape saves"));
    let frozen = engine
        .scenario_sections
        .get("main")
        .and_then(|section| section.frozen_group.clone())
        .expect("departing main section freezes");
    let group =
        clonk_resources::Group::from_raw_memory(std::path::PathBuf::from("Sectmain.c4g"), frozen)
            .expect("frozen section group opens");
    assert!(
        group.exists("Map.bmp"),
        "C4Landscape::Save persists the changed retained map"
    );

    assert!(engine
        .load_scenario_section("main", 0, Vec::new())
        .expect("saved exact main reloads"));
    let mut probe = clonk_resources::MutableGroup::new("Probe.c4g");
    assert!(!engine
        .landscape()
        .expect("reloaded exact landscape")
        .save_changed_c4_map(engine.materials(), &mut probe)
        .expect("post-reload map probe succeeds"));
}

#[test]
fn empty_section_overload_retains_landscape_and_skips_second_init() {
    let mut main = Landscape::flat(8, 4);
    main.set_raster_state(LandscapeRasterState::new(
        1,
        -7,
        RuntimeTexMapState::default(),
    ));
    main.set_modulation(0x1122_3344);
    assert!(main.set_mode(LANDSCAPE_MODE_STATIC));
    let mut empty = vehicle_section("empty", Landscape::flat(1, 1));
    empty.landscape = None;
    empty.gravity = scenario::LegacyC4SVal::new(150, 0, 10, 200);

    let mut engine = Engine::with_seed(29);
    engine.configure_scenario_sections(&[vehicle_section("main", main.clone()), empty]);
    engine.set_landscape(main);
    engine.set_physics(PhysicsSettings::new(77, 12, -20));
    engine.rng.random(9);

    assert!(engine
        .load_scenario_section("empty", 0, Vec::new())
        .expect("empty overload succeeds"));
    let landscape = engine.landscape().unwrap();
    assert_eq!(landscape.width(), 8);
    assert_eq!(landscape.map_seed(), -7);
    assert_eq!(landscape.modulation(), 0x1122_3344);
    assert_eq!(landscape.mode(), LANDSCAPE_MODE_STATIC);
    assert!(landscape.map_changed());
    assert_eq!(engine.physics().gravity, 77);
    assert_eq!(engine.rng.count, 500, "only the first FixRandom runs");
}

#[test]
fn section_save_landscape_removes_and_restore_reputs_solid_masks_like_cpp() {
    // C4S_SAVE_LANDSCAPE serializes the plane only while every solid
    // mask is temporarily removed (C4Game.cpp:4137-4147). Returning to
    // the section loads that clean plane and re-puts the saved gate, so
    // opening it later restores Earth rather than a permanent MCVehic.
    let mut main_landscape = vehicle_section_landscape(20, 20);
    main_landscape.grid_write_byte(10, 10, 1);
    let next_landscape = vehicle_section_landscape(20, 20);

    let mut gate = Definition::from_script(
        "SCGT",
        "Section gate",
        r#"
                #strict 2
                public func OpenMask() { return SetSolidMask(0, 0, 0, 0); }
            "#,
    )
    .expect("section gate compiles");
    gate.set_shape_rect(Some(DefinitionRect::new(0, 0, 1, 1)));
    gate.set_solid_mask(Some(DefinitionTargetRect::new(0, 0, 1, 1, 0, 0)));
    gate.set_sprite_image(Some(DefinitionSpriteImage {
        width: 1,
        height: 1,
        pixels: Arc::from([0, 0, 0, 255]),
        color_mask: None,
    }));

    let mut engine = Engine::with_seed(31);
    engine.configure_scenario_sections(&[
        vehicle_section("main", main_landscape.clone()),
        vehicle_section("next", next_landscape),
    ]);
    engine.set_landscape(main_landscape);
    engine
        .register_definition(gate)
        .expect("section gate registers");
    let gate = engine
        .spawn_object(
            SpawnConfig::new("SCGT")
                .with_position(Vector2::new(10, 10))
                .with_loaded(true),
        )
        .expect("section gate spawns");
    let gate_index = engine.find_object_index(gate).expect("section gate exists");
    engine.update_solid_mask(gate_index);
    assert_eq!(
        engine
            .landscape()
            .expect("main landscape")
            .grid_byte_at(10, 10),
        Some(2)
    );

    assert!(engine
        .load_scenario_section("next", 3, Vec::new())
        .expect("next section loads"));
    assert!(engine
        .load_scenario_section("main", 3, Vec::new())
        .expect("main section reloads"));
    assert_eq!(
        engine
            .landscape()
            .expect("restored main landscape")
            .grid_byte_at(10, 10),
        Some(2),
        "restored gate must be put over the clean section plane"
    );

    let gate_index = engine.find_object_index(gate).expect("saved gate restores");
    engine
        .call_object_function(gate_index, "OpenMask", Vec::new())
        .expect("saved gate opens");
    assert_eq!(
        engine
            .landscape()
            .expect("opened main landscape")
            .grid_byte_at(10, 10),
        Some(1),
        "opening the restored gate must reveal its original Earth byte"
    );
}

#[test]
fn section_save_preserves_overlapping_inactive_solid_mask_like_cpp() {
    // RemoveSolidMasks walks only active objects, but each Remove repairs
    // every overlapping linked C4SolidMask newest-first. Runtime
    // deactivation retains that link; an Objects.txt-loaded inactive row
    // never acquired one (C4Object.cpp:5987-5995;
    // C4SolidMask.cpp:231-274).
    const OVERLAP_X: i32 = 2;
    const ACTIVE_X: i32 = 6;
    const ALL_ACTIVE_X: i32 = 10;
    const LOADED_INACTIVE_X: i32 = 14;
    const STANDALONE_INACTIVE_X: i32 = 18;

    let mut main_landscape = vehicle_section_landscape(24, 20);
    for x in [
        OVERLAP_X,
        ACTIVE_X,
        ALL_ACTIVE_X,
        LOADED_INACTIVE_X,
        STANDALONE_INACTIVE_X,
    ] {
        main_landscape.grid_write_byte(x, 10, 1);
        main_landscape.grid_write_byte(x + 1, 10, 1);
    }
    let next_landscape = vehicle_section_landscape(24, 20);

    let mut engine = Engine::with_seed(71);
    engine.configure_scenario_sections(&[
        vehicle_section("main", main_landscape.clone()),
        vehicle_section("next", next_landscape),
    ]);
    engine.set_landscape(main_landscape);
    engine
        .register_definition(two_pixel_solid_mask_definition("SMFL", 255))
        .expect("full mask registers");
    engine
        .register_definition(two_pixel_solid_mask_definition("SMHF", 0))
        .expect("half mask registers");

    let _overlap_owner = engine
        .spawn_object(
            SpawnConfig::new("SMFL")
                .with_position(Vector2::new(OVERLAP_X, 10))
                .with_loaded(true),
        )
        .expect("older overlap owner spawns");
    let overlap_survivor = engine
        .spawn_object(
            SpawnConfig::new("SMHF")
                .with_position(Vector2::new(OVERLAP_X, 10))
                .with_loaded(true),
        )
        .expect("newer overlap survivor spawns");
    let _active = engine
        .spawn_object(
            SpawnConfig::new("SMFL")
                .with_position(Vector2::new(ACTIVE_X, 10))
                .with_loaded(true),
        )
        .expect("non-overlap active mask spawns");
    let _all_active_owner = engine
        .spawn_object(
            SpawnConfig::new("SMFL")
                .with_position(Vector2::new(ALL_ACTIVE_X, 10))
                .with_loaded(true),
        )
        .expect("all-active owner spawns");
    let _all_active_second = engine
        .spawn_object(
            SpawnConfig::new("SMHF")
                .with_position(Vector2::new(ALL_ACTIVE_X, 10))
                .with_loaded(true),
        )
        .expect("all-active overlap spawns");
    let loaded_inactive = engine
        .spawn_object(
            SpawnConfig::new("SMFL")
                .with_position(Vector2::new(LOADED_INACTIVE_X, 10))
                .with_status(ObjectStatus::Inactive)
                .with_loaded(true),
        )
        .expect("loaded inactive mask owner spawns");
    let standalone_inactive = engine
        .spawn_object(
            SpawnConfig::new("SMHF")
                .with_position(Vector2::new(STANDALONE_INACTIVE_X, 10))
                .with_loaded(true),
        )
        .expect("standalone runtime-inactive owner spawns");

    let all_active_capture = engine.capture_state();
    let all_active_landscape = all_active_capture
        .landscape
        .as_ref()
        .expect("all-active landscape captured");
    for x in [OVERLAP_X, ACTIVE_X, ALL_ACTIVE_X, STANDALONE_INACTIVE_X] {
        assert_eq!(all_active_landscape.grid_byte_at(x, 10), Some(1));
        assert_eq!(all_active_landscape.grid_byte_at(x + 1, 10), Some(1));
    }

    for object in [overlap_survivor, standalone_inactive] {
        engine
            .apply_object_update(
                object,
                ObjectUpdate {
                    status: Some(ObjectStatus::Inactive),
                    ..ObjectUpdate::default()
                },
            )
            .expect("runtime deactivation applies");
        let index = engine
            .find_object_index(object)
            .expect("runtime-inactive object remains");
        assert!(
            engine.objects[index].solid_mask_bake.is_some(),
            "runtime deactivation retains the existing C4SolidMask"
        );
    }
    let loaded_index = engine
        .find_object_index(loaded_inactive)
        .expect("loaded inactive object remains");
    assert!(
        engine.objects[loaded_index].solid_mask_bake.is_none(),
        "loaded inactive objects must not be synthesized into survivors"
    );

    let capture = engine.capture_state();
    let captured = capture.landscape.as_ref().expect("landscape captured");
    assert_eq!(
        [
            captured.grid_byte_at(OVERLAP_X, 10),
            captured.grid_byte_at(OVERLAP_X + 1, 10),
            captured.grid_byte_at(ACTIVE_X, 10),
            captured.grid_byte_at(ACTIVE_X + 1, 10),
            captured.grid_byte_at(ALL_ACTIVE_X, 10),
            captured.grid_byte_at(ALL_ACTIVE_X + 1, 10),
            captured.grid_byte_at(LOADED_INACTIVE_X, 10),
            captured.grid_byte_at(LOADED_INACTIVE_X + 1, 10),
            captured.grid_byte_at(STANDALONE_INACTIVE_X, 10),
            captured.grid_byte_at(STANDALONE_INACTIVE_X + 1, 10),
        ],
        [
            Some(2),
            Some(1),
            Some(1),
            Some(1),
            Some(1),
            Some(1),
            Some(1),
            Some(1),
            Some(2),
            Some(1),
        ],
        "only the two runtime-inactive half masks survive the active-list bracket"
    );

    assert!(engine
        .load_scenario_section(
            "next",
            1,
            vec![overlap_survivor, loaded_inactive, standalone_inactive],
        )
        .expect("section landscape saves"));
    let saved = engine
        .scenario_sections
        .get("main")
        .and_then(|section| section.landscape.as_ref())
        .expect("departing landscape retained");
    assert_eq!(saved.grid_byte_at(OVERLAP_X, 10), Some(2));
    assert_eq!(saved.grid_byte_at(OVERLAP_X + 1, 10), Some(1));
    assert_eq!(saved.grid_byte_at(ACTIVE_X, 10), Some(1));
    assert_eq!(saved.grid_byte_at(ALL_ACTIVE_X, 10), Some(1));
    assert_eq!(saved.grid_byte_at(LOADED_INACTIVE_X, 10), Some(1));
    assert_eq!(saved.grid_byte_at(STANDALONE_INACTIVE_X, 10), Some(2));
    assert_eq!(saved.grid_byte_at(STANDALONE_INACTIVE_X + 1, 10), Some(1));
}

#[test]
fn section_save_landscape_restores_c4b_pxs_and_consolidated_movers() {
    let library = clonk_resources::MaterialLibrary::parse(
        r#"
            [Material Earth]
            Name=Earth
            Density=100

            [Material Water]
            Name=Water
            Density=25
            "#,
    )
    .expect("materials parse");
    let materials = MaterialSet::from_resource_library(&library);
    let earth = materials.id_of("Earth").expect("Earth exists");
    let water = materials.id_of("Water").expect("Water exists");
    let mut engine = Engine::with_seed(37);
    engine.set_materials(materials);
    engine.configure_scenario_sections(&[section("main", 20, true), section("next", 20, true)]);
    engine.set_landscape(vehicle_section_landscape(20, 40));
    assert!(engine.pxs_system.create_at(
        3,
        4,
        pxs::Pxs {
            mat: earth,
            x: C4Fixed::from_raw(98_304),
            y: C4Fixed::from_raw(-147_456),
            xdir: C4Fixed::from_raw(8_192),
            ydir: C4Fixed::from_raw(-32_768),
        },
    ));
    engine.mass_movers.fill_slot(
        5,
        mass_mover::MassMover {
            mat: water,
            x: 3,
            y: 7,
        },
    );
    engine.mass_movers.fill_slot(
        19,
        mass_mover::MassMover {
            mat: water,
            x: 9,
            y: 11,
        },
    );
    engine.pxs_system.note_executed();

    assert!(engine
        .load_scenario_section("next", 1, Vec::new())
        .expect("next section loads"));
    assert_eq!(engine.pxs_system.count(), 0);
    assert_eq!(engine.pxs_system.execute_count(), 0);
    assert_eq!(engine.mass_movers.live_movers(), 0);
    assert!(engine
        .load_scenario_section("main", 1, Vec::new())
        .expect("main section reloads"));

    let pixel = engine
        .pxs_system
        .peek_slot(0, 4)
        .expect("packed PXS restores");
    assert_eq!(pixel.mat, earth);
    assert_eq!(
        [
            pixel.x.val(),
            pixel.y.val(),
            pixel.xdir.val(),
            pixel.ydir.val()
        ],
        [98_304, -147_456, 8_192, -32_768]
    );
    assert!(!engine.pxs_system.chunk_allocated(3));
    assert_eq!(engine.mass_movers.create_ptr(), 0);
    assert_eq!(engine.mass_movers.count(), 2);
    assert_eq!(
        engine.mass_movers.slot(0),
        Some(mass_mover::MassMover {
            mat: water,
            x: 3,
            y: 7,
        })
    );
    assert_eq!(
        engine.mass_movers.slot(1),
        Some(mass_mover::MassMover {
            mat: water,
            x: 9,
            y: 11,
        })
    );
    assert_eq!(engine.mass_movers.slot(5), None);
    assert_eq!(engine.mass_movers.slot(19), None);
}

#[test]
fn section_without_landscape_or_components_retains_pxs_and_movers() {
    let library =
        clonk_resources::MaterialLibrary::parse("[Material Earth]\nName=Earth\nDensity=100\n")
            .expect("materials parse");
    let materials = MaterialSet::from_resource_library(&library);
    let earth = materials.id_of("Earth").expect("Earth exists");
    let mut next = section("next", 20, true);
    next.landscape = None;
    let mut engine = Engine::with_seed(41);
    engine.set_materials(materials);
    engine.configure_scenario_sections(&[section("main", 20, true), next]);
    engine.set_landscape(Landscape::flat(20, 40));
    assert!(engine.pxs_system.create_at(
        0,
        6,
        pxs::Pxs {
            mat: earth,
            x: itofix(2),
            y: itofix(3),
            xdir: C4Fixed::ZERO,
            ydir: C4Fixed::ZERO,
        },
    ));
    engine.mass_movers.fill_slot(
        4,
        mass_mover::MassMover {
            mat: earth,
            x: 2,
            y: 3,
        },
    );

    assert!(engine
        .load_scenario_section("next", 0, Vec::new())
        .expect("section loads"));
    assert_eq!(
        engine.landscape().map(|landscape| landscape.width()),
        Some(20),
        "a section without a map keeps the departing landscape"
    );
    assert!(engine.pxs_system.peek_slot(0, 6).is_some());
    assert_eq!(
        engine.mass_movers.slot(4).map(|mover| (mover.x, mover.y)),
        Some((2, 3))
    );
}
