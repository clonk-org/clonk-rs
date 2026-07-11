use lc_engine::landscape::PixelGrid;
use lc_engine::{
    Definition, DefinitionTargetRect, Engine, EngineError, JoinPlayerConfig, Landscape,
    ObjectUpdate, PhysicsSettings, SpawnConfig, Vector2, CATEGORY_STATIC_BACK,
};
use lc_script::Value;

use crate::support::real_scenario::load_tutorial;

fn call_probe(script: &str, landscape: Landscape, physics: PhysicsSettings) -> Value {
    call_probe_result(script, landscape, physics).expect("Probe runs")
}

fn call_probe_result(
    script: &str,
    landscape: Landscape,
    physics: PhysicsSettings,
) -> Result<Value, EngineError> {
    let mut engine = Engine::with_seed(0);
    engine.set_landscape(landscape);
    engine.set_physics(physics);
    engine
        .register_definition(
            Definition::from_script("TFLT", "SimFlight probe", script)
                .expect("probe script compiles"),
        )
        .expect("probe definition registers");
    let object = engine
        .spawn_object(SpawnConfig::new("TFLT").with_position(Vector2::new(1, 1)))
        .expect("probe object spawns");
    let index = engine.find_object_index(object).expect("probe exists");
    engine.call_object_function(index, "Probe", Vec::new())
}

fn raster_landscape_with_densities(
    width: u32,
    height: u32,
    densities: Vec<i32>,
    pixel: impl Fn(i32, i32) -> u8,
) -> Landscape {
    let mut bytes = Vec::with_capacity(width as usize * height as usize);
    for y in 0..height as i32 {
        for x in 0..width as i32 {
            bytes.push(pixel(x, y));
        }
    }
    let grid = PixelGrid::new(
        width,
        height,
        bytes,
        densities.clone(),
        vec![None; densities.len()],
        vec![None; densities.len()],
    );
    let mut landscape = Landscape::flat(width, height as i32);
    landscape.set_pixel_grid(grid);
    landscape.set_world_height(height as i32);
    landscape
}

fn raster_landscape(width: u32, height: u32, pixel: impl Fn(i32, i32) -> u8) -> Landscape {
    raster_landscape_with_densities(width, height, vec![0, 100], pixel)
}

#[test]
fn sim_flight_matches_cpp_diagonal_contact_vector() {
    // Frozen from C++ SimFlight (C4Movement.cpp:623-653) and FnSimFlight
    // (C4Script.cpp:5309-5330): diagonal travel advances both coordinates
    // by Sign(delta), then applies gravity even on the contact frame.
    let value = call_probe(
        r#"#strict
        global func Probe()
        {
            var x = 2, y = 2, xdir = 20, ydir = 10;
            var result = SimFlight(x, y, xdir, ydir, 50, 100, 20, 10);
            return [result, x, y, xdir, ydir];
        }
        "#,
        raster_landscape(20, 12, |_, y| u8::from(y >= 8)),
        PhysicsSettings::new(100, 20, -20),
    );

    assert_eq!(
        value,
        Value::Array(vec![
            Value::Bool(true),
            Value::Int(11),
            Value::Int(8),
            Value::Int(20),
            Value::Int(20),
        ])
    );
}

#[test]
fn sim_flight_cpp_vectors_cover_defaults_current_pixel_and_inclusive_bounds() {
    // Frozen C++ oracle vectors from FnSimFlight/SimFlight
    // (C4Script.cpp:5309-5330; C4Movement.cpp:623-653). These pin the
    // default 50..100/-1/10 arguments, the do-while current-pixel probe,
    // inclusive density endpoints, inclusive x==GBackWdt, and contact-frame
    // gravity.
    let defaults = call_probe(
        r#"#strict
        global func Probe()
        {
            var x = 2, y = 2, xdir = 20, ydir = 10;
            var result = SimFlight(x, y, xdir, ydir);
            return [result, x, y, xdir, ydir];
        }
        "#,
        raster_landscape(20, 12, |_, y| u8::from(y >= 8)),
        PhysicsSettings::new(100, 20, -20),
    );
    assert_eq!(
        defaults,
        Value::Array(vec![
            Value::Bool(true),
            Value::Int(11),
            Value::Int(8),
            Value::Int(20),
            Value::Int(20),
        ])
    );

    let current_pixel = call_probe(
        r#"#strict
        global func Probe()
        {
            var x = 5, y = 5, xdir = 0, ydir = 0;
            var result = SimFlight(x, y, xdir, ydir, 25, 25, 1, 100);
            return [result, x, y, xdir, ydir];
        }
        "#,
        raster_landscape_with_densities(12, 12, vec![0, 25], |x, y| u8::from(x == 5 && y == 5)),
        PhysicsSettings::new(100, 20, -20),
    );
    assert_eq!(
        current_pixel,
        Value::Array(vec![
            Value::Bool(true),
            Value::Int(5),
            Value::Int(5),
            Value::Int(0),
            Value::Int(20),
        ])
    );

    let right_bound = call_probe(
        r#"#strict
        global func Probe()
        {
            var x = 9, y = 5, xdir = 10, ydir = 0;
            var result = SimFlight(x, y, xdir, ydir, 100, 100, 1, 10);
            return [result, x, y, xdir, ydir];
        }
        "#,
        raster_landscape(10, 10, |_, _| 0),
        PhysicsSettings::new(0, 20, -20),
    );
    assert_eq!(
        right_bound,
        Value::Array(vec![
            Value::Bool(true),
            Value::Int(10),
            Value::Int(5),
            Value::Int(10),
            Value::Int(0),
        ])
    );
}

#[test]
fn sim_flight_failure_is_atomic() {
    // SimFlight writes nothing when its iteration/bounds checks fail
    // (C4Script.cpp:5324-5329; C4Movement.cpp:629,636).
    let value = call_probe(
        r#"#strict
        global func Probe()
        {
            var x = 2, y = 2, xdir = 10, ydir = 0;
            var exhausted = SimFlight(x, y, xdir, ydir, 50, 100, 2, 10);
            var after_exhausted = [exhausted, x, y, xdir, ydir];

            var bx = 9, by = 2, bxdir = 20, bydir = 0;
            var bounded = SimFlight(bx, by, bxdir, bydir, 50, 100, 2, 10);
            var after_bound = [bounded, bx, by, bxdir, bydir];

            return [after_exhausted, after_bound, x, y, xdir, ydir];
        }
        "#,
        raster_landscape(10, 10, |_, _| 0),
        PhysicsSettings::new(100, 20, -20),
    );

    assert_eq!(
        value,
        Value::Array(vec![
            Value::Array(vec![
                Value::Bool(false),
                Value::Int(2),
                Value::Int(2),
                Value::Int(10),
                Value::Int(0),
            ]),
            Value::Array(vec![
                Value::Bool(false),
                Value::Int(9),
                Value::Int(2),
                Value::Int(20),
                Value::Int(0),
            ]),
            Value::Int(2),
            Value::Int(2),
            Value::Int(10),
            Value::Int(0),
        ])
    );
}

#[test]
fn sim_flight_matches_cpp_native_argument_conversions() {
    // The first four native parameters are C4Value references; FnSimFlight
    // converts their current values with getInt(), where an unconvertible
    // referenced value becomes zero. Optional C4ValueInt parameters accept
    // Bool as 0/1 (C4Script.cpp:5309-5321; C4Value.h:159,317-321;
    // C4Value.cpp:488-598).
    let referenced_string = call_probe(
        r#"#strict
        global func Probe()
        {
            var x = "not an integer", y = 2, xdir = 10, ydir = 0;
            var result = SimFlight(x, y, xdir, ydir, 100, 100, 5, 10);
            return [result, x, y, xdir, ydir];
        }
        "#,
        raster_landscape(10, 10, |x, y| u8::from(x == 2 && y == 2)),
        PhysicsSettings::new(0, 20, -20),
    );
    assert_eq!(
        referenced_string,
        Value::Array(vec![
            Value::Bool(true),
            Value::Int(2),
            Value::Int(2),
            Value::Int(10),
            Value::Int(0),
        ])
    );

    let optional_bools = call_probe(
        r#"#strict
        global func Probe()
        {
            var x = 0, y = 1, xdir = 1, ydir = 0;
            var result = SimFlight(x, y, xdir, ydir, true, true, true, true);
            return [result, x, y, xdir, ydir];
        }
        "#,
        raster_landscape_with_densities(4, 4, vec![0, 1], |x, y| {
            u8::from(x == 1 && y == 1)
        }),
        PhysicsSettings::new(0, 20, -20),
    );
    assert_eq!(
        optional_bools,
        Value::Array(vec![
            Value::Bool(true),
            Value::Int(1),
            Value::Int(1),
            Value::Int(1),
            Value::Int(0),
        ])
    );
}

#[test]
fn sim_flight_requires_variable_references_like_cpp_native_dispatch() {
    // Native C4V_pC4Value parameters reject rvalues and omitted arguments
    // before FnSimFlight executes (C4AulExec.cpp:1363-1391;
    // C4Value.cpp:488-500).
    for script in [
        r#"#strict
        global func Probe()
        {
            var x = 0, y = 0, xdir = 0, ydir = 0;
            return SimFlight(x + 0, y, xdir, ydir);
        }
        "#,
        r#"#strict
        global func Probe() { return SimFlight(); }
        "#,
    ] {
        let error = call_probe_result(
            script,
            raster_landscape(4, 4, |_, _| 0),
            PhysicsSettings::new(0, 20, -20),
        )
        .expect_err("C++ native reference parameters reject non-lvalues");
        assert!(
            matches!(error, EngineError::Script { .. }),
            "unexpected SimFlight argument error: {error:?}"
        );
    }
}

#[test]
fn sim_flight_uses_vehicle_solid_mask_density() {
    // GBackDensity reads C4SolidMask's MCVehic pixels
    // (C4Wrappers.h:169-172; C4SolidMask.cpp:92-96). Column fixtures retain
    // masks as the same overlay used by object movement rather than baking a
    // Surface8 plane.
    let script = r#"#strict
    global func Probe()
    {
        var x = 2, y = 10, xdir = 10, ydir = 0;
        var result = SimFlight(x, y, xdir, ydir, 100, 100, 20, 10);
        return [result, x, y, xdir, ydir];
    }
    "#;
    let mut engine = Engine::with_seed(0);
    engine.set_landscape(Landscape::flat(30, 30));
    engine.set_physics(PhysicsSettings::new(0, 20, -20));
    engine
        .register_definition(
            Definition::from_script("TFLT", "SimFlight probe", script)
                .expect("probe script compiles"),
        )
        .expect("probe registers");
    let mut platform =
        Definition::from_script("PLAT", "Vehicle mask", "#strict\n").expect("mask compiles");
    platform.set_category(CATEGORY_STATIC_BACK);
    platform.set_solid_mask(Some(DefinitionTargetRect::new(0, 0, 1, 1, 0, 0)));
    engine
        .register_definition(platform)
        .expect("mask definition registers");
    engine
        .spawn_object(SpawnConfig::new("PLAT").with_position(Vector2::new(8, 10)))
        .expect("mask object spawns");
    let probe = engine
        .spawn_object(SpawnConfig::new("TFLT"))
        .expect("probe spawns");
    let value = engine
        .call_object_function(
            engine.find_object_index(probe).expect("probe exists"),
            "Probe",
            Vec::new(),
        )
        .expect("Probe runs");
    assert_eq!(
        value,
        Value::Array(vec![
            Value::Bool(true),
            Value::Int(8),
            Value::Int(10),
            Value::Int(10),
            Value::Int(0),
        ])
    );
}

#[test]
fn real_clnk_dolphin_jump_uses_sim_flight_to_select_dive() {
    // Real CLNK::DolphinJump predicts the jump's return to deep liquid with
    // SimFlight, then selects Dive (Objects.c4d/Crew.c4d/Clonk.c4d/Script.c:
    // 139-155). This executes the installed CLNK plus planet System.c4g.
    let mut engine = load_tutorial(1, 0);
    let joined = engine
        .join_player(JoinPlayerConfig {
            name: "Dolphin tester".to_string(),
            player_info_id: 0,
            score: 0,
            total_playing_time: 0,
            team: None,
            color_dw: 0xff_00_00,
            pref_color: 0,
            pref_position: 0,
            crew: Vec::new(),
            control_style: true,
            auto_context_menu: false,
            startup_player_count: 1,
        })
        .expect("Tutorial01 player joins");
    let clonk = engine.crew_cursor(joined.number).expect("CLNK joins");
    engine.set_landscape(raster_landscape_with_densities(
        80,
        80,
        vec![0, 25],
        |_, y| u8::from(y >= 30),
    ));
    engine.set_physics(PhysicsSettings::new(100, 20, -20));
    engine
        .apply_object_update(
            clonk,
            ObjectUpdate::new()
                .with_position(Vector2::new(40, 30))
                .with_velocity(Vector2::ZERO)
                .with_action("Swim"),
        )
        .expect("place CLNK at liquid surface");
    engine.debug_set_in_liquid(clonk, true);

    let index = engine.find_object_index(clonk).expect("CLNK exists");
    engine
        .call_object_function(index, "ControlUpDouble", Vec::new())
        .expect("real ControlUpDouble runs DolphinJump");
    assert_eq!(
        engine
            .object_snapshot(clonk)
            .expect("CLNK survives")
            .action
            .name,
        "Dive"
    );
}
