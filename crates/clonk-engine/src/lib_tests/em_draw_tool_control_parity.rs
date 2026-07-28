use super::*;
use crate::chunky::ChunkShape;
use crate::landscape::{
    LandscapeRasterState, PixelGrid, RuntimeTexMapMaterial, RuntimeTexMapState,
};

fn bytes(value: &str) -> LegacyCString {
    LegacyCString::from_bytes(value.as_bytes().to_vec()).expect("fixture is NUL-free")
}

fn control(action: u8) -> EmDrawToolControlData {
    EmDrawToolControlData {
        action,
        mode: LANDSCAPE_MODE_EXACT,
        grade: 1,
        material: bytes("Earth"),
        texture: bytes("Rough"),
        by_client: 7,
        ..Default::default()
    }
}

fn editor_engine(seed: u64) -> Engine {
    const WIDTH: u32 = 32;
    const HEIGHT: u32 = 32;
    let library = clonk_resources::MaterialLibrary::parse(
            "[Material Earth]\nName=Earth\nColor=1,2,3,40,50,60,10,20,30\nDensity=80\nMaxSlide=0\nTextureOverlay=Rough\n\n\
             [Material Water]\nName=Water\nColor=4,5,6,70,80,90,100,110,120\nDensity=25\nTextureOverlay=Smooth\n",
    )
    .expect("material fixture parses");
    let materials = MaterialSet::from_resource_library(&library);

    let mut densities = vec![0; 128];
    densities[1] = 80;
    densities[3] = 25;
    let mut material_names = vec![None; 128];
    material_names[1] = Some("Earth".to_string());
    material_names[3] = Some("Water".to_string());
    let mut texture_names = vec![None; 128];
    texture_names[1] = Some("Rough".to_string());
    texture_names[3] = Some("Liquid".to_string());
    let mut match_texture_names = texture_names.clone();
    match_texture_names[3] = Some("Smooth".to_string());
    let mut shapes = vec![None; 128];
    shapes[1] = Some(ChunkShape::from_shape(0));
    shapes[3] = Some(ChunkShape::from_shape(0));
    let texmap = RuntimeTexMapState {
        densities: densities.clone(),
        material_names: material_names.clone(),
        texture_names: texture_names.clone(),
        match_texture_names,
        shapes,
        materials: vec![
            RuntimeTexMapMaterial {
                name: "Earth".to_string(),
                density: 80,
                shape: ChunkShape::from_shape(0),
            },
            RuntimeTexMapMaterial {
                name: "Water".to_string(),
                density: 25,
                shape: ChunkShape::from_shape(0),
            },
        ],
        texture_inventory: vec![
            "Rough".to_string(),
            "Smooth".to_string(),
            "Liquid".to_string(),
        ],
        default_material_entries: vec![("Earth".to_string(), 1), ("Water".to_string(), 3)],
        material_crossmap_entries: Vec::new(),
        overload_materials: true,
        overload_textures: true,
        ..Default::default()
    };
    let map = clonk_resources::bitmap::IndexedBitmap {
        width: WIDTH,
        height: HEIGHT,
        indices: vec![0; (WIDTH * HEIGHT) as usize],
    };
    let mut raster = LandscapeRasterState::new(1, 0, texmap);
    raster.set_map(&map);
    let grid = PixelGrid::new(
        WIDTH,
        HEIGHT,
        map.indices.clone(),
        densities,
        material_names,
        texture_names,
    );
    let mut landscape = Landscape::new(WIDTH, vec![HEIGHT as i32; WIDTH as usize])
        .expect("landscape fixture builds");
    landscape.set_world_height(HEIGHT as i32);
    landscape.set_pixel_grid(grid);
    landscape.set_raster_state(raster);

    let mut engine = Engine::with_seed(seed);
    engine.set_materials(materials);
    engine.set_landscape(landscape);
    engine
}

#[test]
fn set_mode_and_exact_primitives_match_surface8_geometry_and_ift() {
    let mut engine = editor_engine(11);
    assert_eq!(engine.landscape().unwrap().mode(), LANDSCAPE_MODE_UNDEFINED);
    assert!(engine.execute_em_draw_tool_control(&EmDrawToolControlData {
        action: EMDT_SET_MODE,
        mode: LANDSCAPE_MODE_EXACT,
        ..EmDrawToolControlData::default()
    }));

    let mut brush = control(EMDT_BRUSH);
    brush.x = 5;
    brush.y = 5;
    brush.ift = true;
    assert!(engine.execute_em_draw_tool_control(&brush));
    assert_eq!(engine.debug_landscape_byte(4, 5), Some(0x81));
    assert_eq!(engine.debug_landscape_byte(5, 5), Some(0x81));
    assert_eq!(engine.debug_landscape_byte(5, 4), Some(0));

    let mut line = control(EMDT_LINE);
    line.x = 8;
    line.y = 8;
    line.x2 = 10;
    line.y2 = 8;
    assert!(engine.execute_em_draw_tool_control(&line));
    for x in 7..=10 {
        assert_eq!(engine.debug_landscape_byte(x, 8), Some(1));
    }

    let mut rect = control(EMDT_RECT);
    rect.x = 14;
    rect.y = 14;
    rect.x2 = 12;
    rect.y2 = 12;
    rect.grade = -99;
    rect.ift = true;
    assert!(engine.execute_em_draw_tool_control(&rect));
    for y in 12..=14 {
        for x in 12..=14 {
            assert_eq!(engine.debug_landscape_byte(x, y), Some(0x81));
        }
    }

    brush.material = bytes("Sky");
    brush.texture = bytes("ignored-but-required");
    assert!(engine.execute_em_draw_tool_control(&brush));
    assert_eq!(engine.debug_landscape_byte(4, 5), Some(0));
    assert_eq!(engine.debug_landscape_byte(5, 5), Some(0));
}

#[test]
fn fill_uses_y_then_x_draws_and_exactly_two_draws_per_grade() {
    let mut engine = editor_engine(23);
    let _ = engine
        .landscape
        .as_mut()
        .unwrap()
        .set_mode(LANDSCAPE_MODE_EXACT);
    let mut fill = control(EMDT_FILL);
    fill.x = 14;
    fill.y = 12;
    fill.grade = 5;
    fill.texture = LegacyCString::default();

    let before_count = engine.rng.count;
    let mut mirror = engine.rng.clone();
    let mut expected = Vec::new();
    for _ in 0..fill.grade {
        let r2 = fill.y + mirror.random(fill.grade) - fill.grade / 2;
        let r1 = fill.x + mirror.random(fill.grade) - fill.grade / 2;
        expected.push((r1, r2 + 1));
    }
    assert!(expected.iter().any(|(x, y)| *x - fill.x != *y - 1 - fill.y));

    assert!(engine.execute_em_draw_tool_control(&fill));
    assert_eq!(engine.rng, mirror);
    assert_eq!(engine.rng.count - before_count, 2 * fill.grade);
    let actual = engine
        .pxs_system
        .iter_slots()
        .map(|(_, _, pxs)| (fixtoi(pxs.x), fixtoi(pxs.y)))
        .collect::<Vec<_>>();
    assert_eq!(
        actual, expected,
        "each PXS keeps the y-first coordinate pair"
    );
}

#[test]
fn league_mode_mismatch_and_invalid_fill_inputs_are_rng_free_noops() {
    let mut undefined = editor_engine(28);
    let mut undefined_brush = control(EMDT_BRUSH);
    undefined_brush.mode = LANDSCAPE_MODE_UNDEFINED;
    undefined_brush.material = bytes("Missing");
    undefined_brush.texture = bytes("Missing");
    assert!(undefined.execute_em_draw_tool_control(&undefined_brush));
    assert_eq!(undefined.debug_landscape_byte(0, 0), Some(0));

    let mut engine = editor_engine(29);
    let _ = engine
        .landscape
        .as_mut()
        .unwrap()
        .set_mode(LANDSCAPE_MODE_EXACT);
    let mut fill = control(EMDT_FILL);
    fill.grade = 3;
    fill.x = 10;
    fill.y = 10;
    let before = engine.rng.clone();

    engine.set_league_game(true);
    assert!(!engine.execute_em_draw_tool_control(&fill));
    assert_eq!(engine.rng, before);
    engine.set_league_game(false);

    fill.mode = LANDSCAPE_MODE_STATIC;
    assert!(!engine.execute_em_draw_tool_control(&fill));
    assert_eq!(engine.rng, before);
    fill.mode = LANDSCAPE_MODE_EXACT;

    fill.material = LegacyCString::default();
    assert!(!engine.execute_em_draw_tool_control(&fill));
    assert_eq!(engine.rng, before);
    fill.material = bytes("Sky");
    assert!(!engine.execute_em_draw_tool_control(&fill));
    assert_eq!(engine.rng, before);
    fill.material = bytes("Earth");
    fill.grade = 0;
    assert!(engine.execute_em_draw_tool_control(&fill));
    assert_eq!(engine.rng, before);
}

#[test]
fn static_rect_updates_the_retained_map_and_exact_to_static_restores_it() {
    let mut engine = editor_engine(31);
    let _ = engine
        .landscape
        .as_mut()
        .unwrap()
        .set_mode(LANDSCAPE_MODE_STATIC);

    // A static radius-one brush uses Map::SetPix, not the asymmetric
    // two-pixel CSurface8 circle used in Exact mode.
    let mut brush = control(EMDT_BRUSH);
    brush.mode = LANDSCAPE_MODE_STATIC;
    brush.x = 8;
    brush.y = 8;
    brush.grade = 0;
    assert!(engine.execute_em_draw_tool_control(&brush));
    let retained = engine
        .landscape
        .as_ref()
        .and_then(Landscape::raster_state)
        .and_then(LandscapeRasterState::map)
        .expect("static editor map is retained");
    assert_eq!(retained.index_at(8, 8), Some(1));
    assert_eq!(retained.index_at(7, 8), Some(0));

    // MapToLandscape redraws only the primitive's affected rectangle;
    // unrelated runtime Surface8 changes outside it must survive.
    engine
        .landscape
        .as_mut()
        .unwrap()
        .grid_write_byte(25, 25, 0x81);
    let mut rect = control(EMDT_RECT);
    rect.mode = LANDSCAPE_MODE_STATIC;
    rect.x = 2;
    rect.y = 2;
    rect.x2 = 4;
    rect.y2 = 4;
    assert!(engine.execute_em_draw_tool_control(&rect));
    assert_ne!(engine.debug_landscape_byte(3, 3), Some(0));
    assert_eq!(engine.debug_landscape_byte(25, 25), Some(0x81));

    let _ = engine
        .landscape
        .as_mut()
        .unwrap()
        .set_mode(LANDSCAPE_MODE_EXACT);
    let mut sky = control(EMDT_RECT);
    sky.material = bytes("Sky");
    sky.x = 3;
    sky.y = 3;
    sky.x2 = 3;
    sky.y2 = 3;
    assert!(engine.execute_em_draw_tool_control(&sky));
    assert_eq!(engine.debug_landscape_byte(3, 3), Some(0));

    assert!(engine.execute_em_draw_tool_control(&EmDrawToolControlData {
        action: EMDT_SET_MODE,
        mode: LANDSCAPE_MODE_STATIC,
        ..EmDrawToolControlData::default()
    }));
    assert_ne!(engine.debug_landscape_byte(3, 3), Some(0));
}

#[test]
fn static_draw_marks_and_saves_map_bmp_with_store_map_palette_colors() {
    let mut exact_without_map = Engine::new();
    let mut exact_landscape = Landscape::flat(2, 2);
    exact_landscape.set_raster_state(LandscapeRasterState::new(
        1,
        0,
        RuntimeTexMapState::default(),
    ));
    exact_landscape.set_map_changed();
    exact_without_map.set_landscape(exact_landscape);
    assert!(!exact_without_map
        .save_changed_c4_landscape_map(&mut clonk_resources::MutableGroup::new("Scenario.c4s",))
        .expect("fMapChanged without a retained Map is a no-op"));

    let mut engine = editor_engine(37);
    let _ = engine
        .landscape
        .as_mut()
        .unwrap()
        .set_mode(LANDSCAPE_MODE_STATIC);
    engine
        .landscape
        .as_mut()
        .unwrap()
        .save_initial()
        .expect("initial Surface8 captures");
    assert!(!engine.landscape().unwrap().map_changed());

    let mut untouched = clonk_resources::MutableGroup::new("Scenario.c4s");
    assert!(!engine
        .save_changed_c4_landscape_map(&mut untouched)
        .expect("unchanged map save is a no-op"));
    assert!(!untouched
        .entry_names()
        .iter()
        .any(|name| name.eq_ignore_ascii_case("Map.bmp")));

    let mut brush = control(EMDT_BRUSH);
    brush.mode = LANDSCAPE_MODE_STATIC;
    brush.x = 8;
    brush.y = 8;
    brush.grade = 0;
    assert!(engine.execute_em_draw_tool_control(&brush));
    assert!(engine.landscape().unwrap().map_changed());

    let mut saved = clonk_resources::MutableGroup::new("Scenario.c4s");
    saved
        .add_file("LANDSCAPE.BMP", b"stale full surface".to_vec())
        .unwrap();
    engine
        .save_c4_static_landscape(&mut saved)
        .expect("static scenario components save");
    let mut gated = clonk_resources::MutableGroup::new("Scenario.c4s");
    assert!(engine
        .save_changed_c4_landscape_map(&mut gated)
        .expect("changed map saves"));
    assert!(
        engine.landscape().unwrap().map_changed(),
        "save does not clear the gate"
    );

    let root = clonk_resources::Group::from_memory(
        std::path::PathBuf::from("Scenario.c4s"),
        saved.pack_raw().expect("scenario packs"),
    )
    .expect("scenario reopens");
    assert!(!root.exists("Landscape.bmp"));
    let bytes = root.read_file("Map.bmp").expect("SaveMap uses Map.bmp");
    let map = clonk_resources::bitmap::IndexedBitmap::decode(&bytes).expect("map decodes");
    assert_eq!(map.index_at(8, 8), Some(1));
    assert_eq!(&bytes[54..58], &[252, 196, 192, 0], "sky palette is BGRA");
    assert_eq!(&bytes[58..62], &[30, 20, 10, 0]);
    let ift_offset = 54 + 129 * 4;
    assert_eq!(&bytes[ift_offset..ift_offset + 4], &[60, 50, 40, 0]);
    assert_eq!(&bytes[54 + 127 * 4..54 + 128 * 4], &[0, 0, 0, 0]);
    assert_eq!(&bytes[54 + 255 * 4..54 + 256 * 4], &[0, 0, 0, 0]);

    let mut diff_group = clonk_resources::MutableGroup::new("Scenario.c4s");
    assert!(engine
        .save_c4_landscape_diff(&mut diff_group, false)
        .expect("changed Surface8 diff saves"));
    let diff_group = clonk_resources::Group::from_memory(
        std::path::PathBuf::from("Scenario.c4s"),
        diff_group.pack_raw().unwrap(),
    )
    .unwrap();
    assert!(diff_group.exists("Map.bmp"));
    let diff = diff_group.read_file("DiffLandscape.bmp").unwrap();
    assert_eq!(&diff[58..62], &[3, 2, 1, 0]);
    assert_eq!(&diff[54 + 129 * 4..54 + 130 * 4], &[3, 2, 1, 0]);
}

#[test]
fn runtime_texmap_save_gates_creates_and_mutates_material_child() {
    let mut engine = editor_engine(41);
    let mut untouched = clonk_resources::MutableGroup::new("Scenario.c4s");
    assert!(!engine
        .save_c4_landscape_textures(&mut untouched)
        .expect("clean texmap save is a no-op"));
    assert!(!untouched
        .entry_names()
        .iter()
        .any(|name| name.eq_ignore_ascii_case("Material.c4g")));
    let mut clean_material = clonk_resources::MutableGroup::new("Material.c4g");
    clean_material
        .add_file("TexMap.txt", b"leave stale bytes alone".to_vec())
        .unwrap();
    clean_material
        .add_file("Sentinel.bin", b"untouched".to_vec())
        .unwrap();
    let mut clean_root = clonk_resources::MutableGroup::new("Scenario.c4s");
    clean_root
        .add_child("Material.c4g", clean_material)
        .unwrap();
    assert!(!engine
        .save_c4_landscape_textures(&mut clean_root)
        .expect("clean existing child is untouched"));
    let clean_root = clonk_resources::Group::from_memory(
        std::path::PathBuf::from("Scenario.c4s"),
        clean_root.pack_raw().unwrap(),
    )
    .unwrap();
    let clean_material = clean_root.open_child("Material.c4g").unwrap();
    assert_eq!(
        clean_material.read_file("TexMap.txt").unwrap(),
        b"leave stale bytes alone"
    );
    assert_eq!(
        clean_material.read_file("Sentinel.bin").unwrap(),
        b"untouched"
    );

    let slot = engine
        .landscape
        .as_mut()
        .unwrap()
        .raster_state_mut()
        .unwrap()
        .texmap_mut()
        .get_index("Earth", Some("Smooth"), true);
    assert_eq!(slot, 2);
    assert!(engine.landscape().unwrap().texture_map_entries_added());

    let expected = b"# Automatically generated texture map\r\n# Contains material-texture-combinations added at runtime\r\n# Import materials from global file as well\r\nOverloadMaterials\r\n# Import textures from global file as well\r\nOverloadTextures\r\n\r\n1=Earth-Rough\r\n2=Earth-Smooth\r\n3=Water-Smooth\r\n";

    let mut fresh = clonk_resources::MutableGroup::new("Scenario.c4s");
    assert!(engine
        .save_c4_landscape_textures(&mut fresh)
        .expect("fresh Material child saves"));
    let fresh = clonk_resources::Group::from_memory(
        std::path::PathBuf::from("Scenario.c4s"),
        fresh.pack_raw().expect("fresh scenario packs"),
    )
    .expect("fresh scenario reopens");
    assert_eq!(
        fresh
            .open_child("Material.c4g")
            .expect("fresh material child opens")
            .read_file("TexMap.txt")
            .expect("fresh texmap reads"),
        expected
    );

    let mut old_material = clonk_resources::MutableGroup::new("Material.c4g");
    old_material
        .add_file("Earth.c4m", b"sentinel".to_vec())
        .unwrap();
    old_material
        .add_file("texmap.TXT", b"stale".to_vec())
        .unwrap();
    let mut old_root = clonk_resources::MutableGroup::new("Scenario.c4s");
    old_root.add_child("Material.c4g", old_material).unwrap();
    let source = clonk_resources::Group::from_memory(
        std::path::PathBuf::from("Scenario.c4s"),
        old_root.pack_raw().expect("old scenario packs"),
    )
    .expect("old scenario opens");
    let mut rewritten = clonk_resources::MutableGroup::from_group(&source).unwrap();
    assert!(engine
        .save_c4_landscape_textures(&mut rewritten)
        .expect("existing Material child mutates"));
    let rewritten = clonk_resources::Group::from_memory(
        std::path::PathBuf::from("Scenario.c4s"),
        rewritten.pack_raw().expect("rewritten scenario packs"),
    )
    .expect("rewritten scenario opens");
    let material = rewritten
        .open_child("Material.c4g")
        .expect("rewritten material child opens");
    assert_eq!(material.read_file("Earth.c4m").unwrap(), b"sentinel");
    assert_eq!(material.read_file("TexMap.txt").unwrap(), expected);
    assert!(
        engine.landscape().unwrap().texture_map_entries_added(),
        "save does not clear the gate"
    );

    let mut file_root = clonk_resources::MutableGroup::new("Scenario.c4s");
    file_root
        .add_file("Material.c4g", b"ordinary file".to_vec())
        .unwrap();
    assert!(matches!(
        engine.save_c4_landscape_textures(&mut file_root),
        Err(LandscapePersistenceError::MaterialGroupIsFile)
    ));

    let mut moved = editor_engine(42);
    let (succeeded, _) = moved
        .landscape
        .as_mut()
        .unwrap()
        .raster_state_mut()
        .unwrap()
        .texmap_mut()
        .set_texture_index("Earth-Rough", 5, false);
    assert!(succeeded);
    assert!(moved.landscape().unwrap().texture_map_entries_added());

    let mut removed = editor_engine(43);
    let cleared = removed
        .landscape
        .as_mut()
        .unwrap()
        .raster_state_mut()
        .unwrap()
        .texmap_mut()
        .remove_unused_entries([true; 128]);
    assert!(cleared.is_empty());
    assert!(removed.landscape().unwrap().texture_map_entries_added());
}
