use clonk_engine::landscape::PixelGrid;
use clonk_engine::{
    command::CommandId, math::itofix_prec, ActionSpec, ActionState, CommandDirection, Definition,
    DefinitionTargetRect, Direction, Engine, EngineError, JoinPlayerConfig, Landscape,
    ObjectId, ObjectUpdate, ObjectVertex, PhysicsSettings, ShapeAttachRecord, SpawnConfig, Vector2,
    CATEGORY_OBJECT, CATEGORY_STATIC_BACK, CNAT_BOTTOM,
};
use clonk_resources::PhysicalInfo;
use clonk_script::Value;
use std::collections::HashMap;

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
fn explicit_foreign_rotation_functions_use_the_targets_live_exact_state() {
    // FnSetRDir/FnGetRDir and FnSetR/FnGetR take an optional explicit pObj;
    // C++ reads and writes that object's live rdir/r rather than the calling
    // object's scope (C4Script.cpp:711-722,738-746,1161-1189). Distinct
    // caller/target seeds make receiver-state leakage observable, while the
    // same-call reads pin synchronous visibility of the foreign writes.
    let script = r#"#strict
protected func Probe(pTarget)
{
    var before_rdir = GetRDir(pTarget, 10);
    var before_r = GetR(pTarget);
    var set_rdir = SetRDir(-47, pTarget, 10);
    var after_rdir = GetRDir(pTarget, 10);
    var set_r = SetR(271, pTarget);
    var after_r = GetR(pTarget);
    return [before_rdir, before_r, set_rdir, after_rdir,
            set_r, after_r, GetRDir(0, 10), GetR()];
}
"#;

    let mut definition =
        Definition::from_script("ROTN", "Foreign rotation probe", script).expect("script compiles");
    definition.set_rotateable(360);

    let mut engine = Engine::with_seed(17);
    engine
        .register_definition(definition)
        .expect("probe definition registers");
    let caller = engine
        .spawn_object(
            SpawnConfig::new("ROTN")
                .with_category(CATEGORY_OBJECT)
                .with_rotation(-29)
                .with_rotation_velocity(itofix_prec(91, 10)),
        )
        .expect("caller spawns");
    let target = engine
        .spawn_object(
            SpawnConfig::new("ROTN")
                .with_category(CATEGORY_OBJECT)
                .with_rotation(137)
                .with_rotation_velocity(itofix_prec(23, 10)),
        )
        .expect("target spawns");

    let caller_index = engine.find_object_index(caller).expect("caller exists");
    let result = engine
        .call_object_function(caller_index, "Probe", vec![Value::Object(target.as_u64())])
        .expect("foreign rotation probe runs");
    assert_eq!(
        result,
        Value::Array(vec![
            Value::Int(23),
            Value::Int(137),
            Value::Bool(true),
            Value::Int(-47),
            Value::Bool(true),
            Value::Int(-89),
            Value::Int(91),
            Value::Int(-29),
        ]),
        "explicit foreign calls must observe the target while bare calls retain the caller"
    );

    let snapshot = engine.snapshot();
    let target_state = snapshot.object(target).expect("target remains live");
    assert_eq!(target_state.rotation, 271);
    assert_eq!(target_state.rotation_velocity, Some(itofix_prec(-47, 10)));
    let caller_state = snapshot.object(caller).expect("caller remains live");
    assert_eq!(caller_state.rotation, -29);
    assert_eq!(caller_state.rotation_velocity, Some(itofix_prec(91, 10)));
}

#[test]
fn adjust_walk_rotation_targets_foreign_object_and_updates_only_that_rdir() {
    // FnAdjustWalkRotation defaults nil to cthr->Obj but otherwise invokes
    // C4Object::AdjustWalkRotation on the supplied live object directly
    // (C4Script.cpp:5433-5442). Distinct initial rotation/rdir values prove
    // that the foreign scope, definition shape and attachment record are used
    // without leaking the write back into the callback object.
    let script = r#"#strict
protected func Probe(pTarget)
{
    return AdjustWalkRotation(20, 20, 100, pTarget);
}
"#;
    let mut actions = HashMap::new();
    actions.insert(
        "Walk".to_string(),
        ActionSpec::default()
            .with_procedure("walk")
            .with_attach(CNAT_BOTTOM),
    );
    let mut definition =
        Definition::from_script("WROT", "Foreign walk rotation probe", script)
            .expect("probe compiles");
    definition.configure_actions(Some("Walk".to_string()), actions);
    definition.set_rotateable(45);
    definition.set_shape_vertices(vec![ObjectVertex::new(0, 0).with_cnat(CNAT_BOTTOM)]);

    let mut engine = Engine::with_seed(17);
    let mut surface = vec![25; 32];
    surface.extend(vec![5; 32]);
    engine.set_landscape(Landscape::new(64, surface).expect("slope landscape builds"));
    engine
        .register_definition(definition)
        .expect("probe definition registers");

    let attached = ShapeAttachRecord {
        mat_valid: true,
        mat_vehicle: false,
        x: 30,
        y: 15,
        vtx: 0,
    };
    let mut caller_config = SpawnConfig::new("WROT")
        .with_category(CATEGORY_OBJECT)
        .with_action(ActionState::new("Walk"))
        .with_rotation_velocity(itofix_prec(41, 100));
    caller_config.shape_attach = Some(attached);
    let caller = engine
        .spawn_object(caller_config)
        .expect("caller spawns");

    let mut target_config = SpawnConfig::new("WROT")
        .with_category(CATEGORY_OBJECT)
        .with_action(ActionState::new("Walk"))
        .with_rotation(6)
        .with_rotation_velocity(itofix_prec(23, 100));
    target_config.shape_attach = Some(attached);
    let target = engine
        .spawn_object(target_config)
        .expect("target spawns");
    for object in [caller, target] {
        engine
            .apply_object_update(
                object,
                ObjectUpdate {
                    t_attach: Some(CNAT_BOTTOM),
                    ..ObjectUpdate::new()
                },
            )
            .expect("walk attachment latches for the probe frame");
    }

    let caller_index = engine.find_object_index(caller).expect("caller exists");
    assert_eq!(
        engine
            .call_object_function(
                caller_index,
                "Probe",
                vec![Value::Object(target.as_u64())],
            )
            .expect("foreign walk rotation call runs"),
        Value::Bool(true)
    );

    let snapshot = engine.snapshot();
    assert_eq!(
        snapshot
            .object(target)
            .expect("target remains live")
            .rotation_velocity,
        Some(itofix_prec(-15, 100)),
        "the target's rotation and sampled floor slope drive its rdir"
    );
    assert_eq!(
        snapshot
            .object(caller)
            .expect("caller remains live")
            .rotation_velocity,
        Some(itofix_prec(41, 100)),
        "the callback object's rdir is untouched"
    );

    assert_eq!(
        engine
            .call_object_function(
                caller_index,
                "Probe",
                vec![Value::Object(u64::MAX)],
            )
            .expect("missing target is a normal false result"),
        Value::Bool(false)
    );
}

#[test]
fn create_object_rotation_writes_are_live_and_fold_into_the_spawn() {
    // C4Game::NewObject inserts the object synchronously. The object returned
    // by CreateObject can therefore be passed straight to the rotation
    // functions, read back in the same VM call, and must retain those writes
    // after Rust materializes its deferred SpawnConfig.
    let script = r#"#strict
protected func Probe()
{
    var spawned = CreateObject(CHLD, 0, 0, -1);
    var set_r = SetR(271, spawned);
    var set_rdir = SetRDir(-47, spawned, 10);
    return [spawned, set_r, set_rdir, GetR(spawned), GetRDir(spawned, 10)];
}
"#;

    let mut engine = Engine::with_seed(18);
    let mut child = Definition::from_script("CHLD", "Pending rotation child", "#strict\n")
        .expect("child compiles");
    child.set_rotateable(360);
    engine
        .register_definition(child)
        .expect("child definition registers");
    engine
        .register_definition(
            Definition::from_script("CALL", "Pending rotation caller", script)
                .expect("caller compiles"),
        )
        .expect("caller definition registers");
    let caller = engine
        .spawn_object(SpawnConfig::new("CALL").with_category(CATEGORY_OBJECT))
        .expect("caller spawns");

    let result = engine
        .call_object_function(
            engine.find_object_index(caller).expect("caller exists"),
            "Probe",
            Vec::new(),
        )
        .expect("CreateObject rotation probe runs");
    let Value::Array(values) = result else {
        panic!("Probe returns its rotation observations");
    };
    assert_eq!(values.len(), 5);
    let spawned = match values[0] {
        Value::Object(raw) => ObjectId::new(raw),
        ref other => panic!("CreateObject returns an object, got {other:?}"),
    };
    assert_eq!(
        &values[1..],
        &[
            Value::Bool(true),
            Value::Bool(true),
            Value::Int(-89),
            Value::Int(-47),
        ],
        "pending-object rotation writes are immediately readable"
    );

    let snapshot = engine.snapshot();
    let spawned_state = snapshot.object(spawned).expect("spawn materializes");
    assert_eq!(spawned_state.definition_id, "CHLD");
    assert_eq!(spawned_state.rotation, 271);
    assert_eq!(spawned_state.rotation_velocity, Some(itofix_prec(-47, 10)));
    assert!(spawned_state.mobile, "SetRDir mobilizes the new object");
}

#[test]
fn construction_reads_spawn_rotation_velocity_before_object_insertion() {
    // Engine-owned SpawnConfig creation invokes Construction while the new
    // Object is not yet in Engine::objects. C++ nevertheless exposes its
    // initialized live rdir through GetRDir; the callback must not fall back
    // to a zero world-snapshot baseline.
    let script = r#"#strict
local construction_rdir;
protected func Construction()
{
    construction_rdir = GetRDir(0, 100);
}
"#;
    let mut definition =
        Definition::from_script("CRDV", "Construction rdir probe", script).expect("probe compiles");
    definition.set_rotateable(360);

    let mut engine = Engine::with_seed(19);
    engine
        .register_definition(definition)
        .expect("probe definition registers");
    let initial_rdir = itofix_prec(-37, 100);
    let object = engine
        .spawn_object(
            SpawnConfig::new("CRDV")
                .with_category(CATEGORY_OBJECT)
                .with_rotation_velocity(initial_rdir),
        )
        .expect("probe object spawns");

    let snapshot = engine.object_snapshot(object).expect("probe remains live");
    assert_eq!(snapshot.rotation_velocity, Some(initial_rdir));
    assert_eq!(
        snapshot.local_vars.get("construction_rdir"),
        Some(&Value::Int(-37)),
        "Construction sees SpawnConfig's exact pre-insertion rdir"
    );
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
                Value::Nil,
            ]),
            Value::Array(vec![
                Value::Bool(false),
                Value::Int(9),
                Value::Int(2),
                Value::Int(20),
                Value::Nil,
            ]),
            Value::Int(2),
            Value::Int(2),
            Value::Int(10),
            Value::Nil,
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
fn sim_flight_optional_precision_default_follows_caller_strictness() {
    let script = |strict_level: u8, precision: &str| {
        format!(
            r#"#strict {strict_level}
            func Probe()
            {{
                var unset;
                var x = 2, y = 2, xdir = 20, ydir = 10;
                var result = SimFlight(x, y, xdir, ydir, unset, unset, unset, {precision});
                return [result, x, y, xdir, ydir];
            }}
            "#
        )
    };
    let expected_default = Value::Array(vec![
        Value::Bool(true),
        Value::Int(11),
        Value::Int(8),
        Value::Int(20),
        Value::Int(20),
    ]);

    for falsy in ["false", "0"] {
        let legacy = call_probe_result(
            &script(2, falsy),
            raster_landscape(20, 12, |_, y| u8::from(y >= 8)),
            PhysicsSettings::new(100, 20, -20),
        )
        .unwrap_or_else(|error| panic!("strict-2 SimFlight({falsy}) failed: {error}"));
        assert_eq!(
            legacy,
            expected_default,
            "below strict 3, {falsy} is absent and uses precision 10"
        );

        let error = call_probe_result(
            &script(3, falsy),
            raster_landscape(20, 12, |_, y| u8::from(y >= 8)),
            PhysicsSettings::new(100, 20, -20),
        )
        .expect_err("strict 3 preserves explicit zero precision");
        let EngineError::Script { source, .. } = error else {
            panic!("unexpected strict-3 SimFlight error: {error}");
        };
        assert!(
            source.to_string().contains("precision must not be zero"),
            "unexpected strict-3 SimFlight script error: {source}"
        );
    }
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
            rounds: 0,
            rounds_won: 0,
            rounds_lost: 0,
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
    let clonk = engine.crew_cursor(joined.number()).expect("CLNK joins");
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

#[test]
fn real_tutorial09_clnk_command_jump_dives_into_deep_water() {
    // ObjectComJump predicts from the bottom CNAT vertex and selects Dive
    // instead of ObjectActionJump when the flight lands in liquid that is at
    // least nine pixels deep (C4ObjectCom.cpp:280-307;
    // C4Movement.cpp:623-670). Tutorial09 is the shipped swimming tutorial.
    let mut engine = load_tutorial(9, 0);
    let joined = engine
        .join_player(JoinPlayerConfig {
            name: "Dive tester".to_string(),
            player_info_id: 0,
            score: 0,
            rounds: 0,
            rounds_won: 0,
            rounds_lost: 0,
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
        .expect("Tutorial09 player joins");
    let clonk = engine.crew_cursor(joined.number()).expect("CLNK joins");
    engine.set_landscape(raster_landscape_with_densities(
        240,
        100,
        vec![0, 25],
        |_, y| u8::from(y >= 50),
    ));
    engine.set_physics(PhysicsSettings::new(100, 20, -20));
    engine
        .apply_object_update(
            clonk,
            ObjectUpdate::new()
                .with_position(Vector2::new(120, 40))
                .with_velocity(Vector2::ZERO)
                .with_action("Walk")
                .with_direction(Direction::Right)
                .with_command_direction(CommandDirection::Right),
        )
        .expect("place CLNK one pixel above the water by its bottom vertex");

    engine
        .player_object_command(joined.number(), CommandId::Jump, None, 0, 0)
        .expect("queue C4CMD_Jump");
    engine.tick_without_snapshot().expect("execute ObjectComJump");

    let snapshot = engine.object_snapshot(clonk).expect("CLNK survives");
    assert_eq!(snapshot.action.name, "Dive");
    assert_eq!(snapshot.velocity, Vector2::new(2, -4));
}

#[test]
fn script_jump_native_uses_object_com_jump_deep_water_dive() {
    // FnJump calls ObjectComJump directly (C4Script.cpp:358-363), so it
    // must take the same SimFlightHitsLiquid/ObjectActionDive branch as a
    // queued C4CMD_Jump (C4ObjectCom.cpp:280-307).
    let mut actions = HashMap::new();
    actions.insert(
        "Walk".to_string(),
        ActionSpec::default().with_procedure("walk"),
    );
    actions.insert(
        "Jump".to_string(),
        ActionSpec::default().with_procedure("flight"),
    );
    actions.insert(
        "Dive".to_string(),
        ActionSpec::default().with_procedure("swim"),
    );
    let mut definition = Definition::from_script(
        "DVER",
        "Script dive probe",
        "#strict\nfunc Probe() { return Jump(); }\n",
    )
    .expect("probe compiles");
    definition.configure_actions(Some("Walk".to_string()), actions);
    definition.set_shape_vertices(vec![ObjectVertex::new(0, 9).with_cnat(CNAT_BOTTOM)]);
    definition.set_physical(PhysicalInfo {
        walk: 70_000,
        jump: 40_000,
        ..Default::default()
    });

    let mut engine = Engine::new();
    engine.set_landscape(raster_landscape_with_densities(
        240,
        100,
        vec![0, 25],
        |_, y| u8::from(y >= 50),
    ));
    engine.set_physics(PhysicsSettings::new(100, 20, -20));
    engine
        .register_definition(definition)
        .expect("probe registers");
    let object = engine
        .spawn_object(
            SpawnConfig::new("DVER")
                // The bottom vertex starts at y=50 inside liquid. C++ first
                // simulates at most ten frames to escape into air, then
                // continues the landing probe (C4Movement.cpp:657-664).
                .with_position(Vector2::new(120, 41))
                .with_action(ActionState::new("Walk"))
                .with_direction(Direction::Right)
                .with_command_direction(CommandDirection::Right),
        )
        .expect("probe spawns");
    let index = engine.find_object_index(object).expect("probe exists");

    assert_eq!(
        engine
            .call_object_function(index, "Probe", Vec::new())
            .expect("Probe calls native Jump"),
        Value::Bool(true)
    );
    let snapshot = engine.object_snapshot(object).expect("probe survives");
    assert_eq!(snapshot.action.name, "Dive");
    assert_eq!(snapshot.velocity, Vector2::new(2, -4));
}

#[test]
fn script_jump_native_missing_dive_action_falls_back_to_jump() {
    // ObjectComJump only returns from the predicted-liquid branch when
    // ObjectActionDive succeeds. A definition without a Dive ActMap entry
    // must continue to ObjectActionJump (C4ObjectCom.cpp:297-312;
    // C4Object::SetActionByName miss returns false, C4Object.cpp:4218-4234).
    // Gold Rush BISO has Walk/Jump but no Dive and reaches this path at the
    // pinned frame-3902 TimerCall.
    let mut actions = HashMap::new();
    actions.insert(
        "Walk".to_string(),
        ActionSpec::default().with_procedure("walk"),
    );
    actions.insert(
        "Jump".to_string(),
        ActionSpec::default().with_procedure("flight"),
    );
    let mut definition = Definition::from_script(
        "NDVE",
        "No Dive jump probe",
        "#strict\nfunc Probe() { return Jump(); }\n",
    )
    .expect("probe compiles");
    definition.configure_actions(Some("Walk".to_string()), actions);
    definition.set_shape_vertices(vec![ObjectVertex::new(0, 9).with_cnat(CNAT_BOTTOM)]);
    definition.set_physical(PhysicalInfo {
        walk: 70_000,
        jump: 40_000,
        ..Default::default()
    });

    let mut engine = Engine::new();
    engine.set_landscape(raster_landscape_with_densities(
        240,
        100,
        vec![0, 25],
        |_, y| u8::from(y >= 50),
    ));
    engine.set_physics(PhysicsSettings::new(100, 20, -20));
    engine
        .register_definition(definition)
        .expect("probe registers");
    let object = engine
        .spawn_object(
            SpawnConfig::new("NDVE")
                .with_position(Vector2::new(120, 41))
                .with_action(ActionState::new("Walk"))
                .with_direction(Direction::Right)
                .with_command_direction(CommandDirection::Right),
        )
        .expect("probe spawns");
    let index = engine.find_object_index(object).expect("probe exists");

    assert_eq!(
        engine
            .call_object_function(index, "Probe", Vec::new())
            .expect("Probe calls native Jump"),
        Value::Bool(true)
    );
    let snapshot = engine.object_snapshot(object).expect("probe survives");
    assert_eq!(snapshot.action.name, "Jump");
    assert_eq!(snapshot.velocity, Vector2::new(2, -4));
}

#[test]
fn script_jump_native_respects_contact_density_dive_gate() {
    // ObjectComJump only attempts SimFlightHitsLiquid when the live shape's
    // ContactDensity is greater than C4M_Liquid (C4ObjectCom.cpp:297-305).
    // A liquid-contact object therefore takes the ordinary Jump fallback.
    let mut actions = HashMap::new();
    actions.insert(
        "Walk".to_string(),
        ActionSpec::default().with_procedure("walk"),
    );
    actions.insert(
        "Jump".to_string(),
        ActionSpec::default().with_procedure("flight"),
    );
    actions.insert(
        "Dive".to_string(),
        ActionSpec::default().with_procedure("swim"),
    );
    let mut definition = Definition::from_script(
        "LDVR",
        "Liquid contact probe",
        "#strict\nfunc Probe() { return Jump(); }\n",
    )
    .expect("probe compiles");
    definition.configure_actions(Some("Walk".to_string()), actions);
    definition.set_shape_vertices(vec![ObjectVertex::new(0, 9).with_cnat(CNAT_BOTTOM)]);
    definition.set_contact_density(25);
    definition.set_physical(PhysicalInfo {
        walk: 70_000,
        jump: 40_000,
        ..Default::default()
    });

    let mut engine = Engine::new();
    engine.set_landscape(raster_landscape_with_densities(
        240,
        100,
        vec![0, 25],
        |_, y| u8::from(y >= 50),
    ));
    engine.set_physics(PhysicsSettings::new(100, 20, -20));
    engine
        .register_definition(definition)
        .expect("probe registers");
    let object = engine
        .spawn_object(
            SpawnConfig::new("LDVR")
                .with_position(Vector2::new(120, 40))
                .with_action(ActionState::new("Walk"))
                .with_direction(Direction::Right)
                .with_command_direction(CommandDirection::Right),
        )
        .expect("probe spawns");
    let index = engine.find_object_index(object).expect("probe exists");

    assert_eq!(
        engine
            .call_object_function(index, "Probe", Vec::new())
            .expect("Probe calls native Jump"),
        Value::Bool(true)
    );
    assert_eq!(
        engine.object_snapshot(object).expect("probe survives").action.name,
        "Jump"
    );
    assert_eq!(
        engine
            .object_snapshot(object)
            .expect("probe survives")
            .contact_density,
        25
    );
}

#[test]
fn script_set_contact_density_changes_the_same_call_jump_gate() {
    // FnSetContactDensity writes the live C4Shape field immediately
    // (C4Script.cpp:1286-1291). The following FnJump in the same script call
    // must observe 25 and skip the dive branch (C4ObjectCom.cpp:297-305).
    let mut actions = HashMap::new();
    actions.insert(
        "Walk".to_string(),
        ActionSpec::default().with_procedure("walk"),
    );
    actions.insert(
        "Jump".to_string(),
        ActionSpec::default().with_procedure("flight"),
    );
    actions.insert(
        "Dive".to_string(),
        ActionSpec::default().with_procedure("swim"),
    );
    let script = r#"
#strict
func ProbeLow()
{
    SetContactDensity(C4M_Liquid);
    return Jump();
}
func ProbeSolid()
{
    SetContactDensity(C4M_Solid);
    return Jump();
}
"#;
    let mut definition =
        Definition::from_script("SCDN", "Contact density probe", script).expect("probe compiles");
    definition.configure_actions(Some("Walk".to_string()), actions);
    definition.set_shape_vertices(vec![ObjectVertex::new(0, 9).with_cnat(CNAT_BOTTOM)]);
    definition.set_physical(PhysicalInfo {
        walk: 70_000,
        jump: 40_000,
        ..Default::default()
    });
    let restored_definition = definition.clone();

    let mut engine = Engine::new();
    engine.set_landscape(raster_landscape_with_densities(
        240,
        100,
        vec![0, 25],
        |_, y| u8::from(y >= 50),
    ));
    engine.set_physics(PhysicsSettings::new(100, 20, -20));
    engine
        .register_definition(definition)
        .expect("probe registers");
    let object = engine
        .spawn_object(
            SpawnConfig::new("SCDN")
                .with_position(Vector2::new(120, 40))
                .with_action(ActionState::new("Walk"))
                .with_direction(Direction::Right)
                .with_command_direction(CommandDirection::Right),
        )
        .expect("probe spawns");
    let index = engine.find_object_index(object).expect("probe exists");

    assert_eq!(
        engine
            .call_object_function(index, "ProbeLow", Vec::new())
            .expect("ProbeLow changes the live shape then jumps"),
        Value::Bool(true)
    );
    assert_eq!(
        engine.object_snapshot(object).expect("probe survives").action.name,
        "Jump"
    );
    assert_eq!(
        engine
            .object_snapshot(object)
            .expect("probe survives")
            .contact_density,
        25
    );

    // C4Shape::CompileFunc stores ContactDensity in the object's embedded
    // Shape (C4Shape.cpp:495-510), so the per-object value survives a state
    // round trip independently of the definition's default.
    let state = engine.capture_state();
    let mut restored = Engine::new();
    restored
        .register_definition(restored_definition)
        .expect("probe registers after restore");
    restored.restore_state(&state).expect("state restores");
    assert_eq!(
        restored
            .object_snapshot(object)
            .expect("restored probe survives")
            .contact_density,
        25
    );

    restored
        .apply_object_update(
            object,
            ObjectUpdate::new()
                .with_position(Vector2::new(120, 40))
                .with_velocity(Vector2::ZERO)
                .with_action("Walk")
                .with_direction(Direction::Right)
                .with_command_direction(CommandDirection::Right),
        )
        .expect("reset restored probe");
    let index = restored
        .find_object_index(object)
        .expect("restored probe exists");
    assert_eq!(
        restored
            .call_object_function(index, "ProbeSolid", Vec::new())
            .expect("ProbeSolid restores the live shape then jumps"),
        Value::Bool(true)
    );
    let snapshot = restored.object_snapshot(object).expect("probe survives");
    assert_eq!(snapshot.action.name, "Dive");
    assert_eq!(snapshot.contact_density, 50);
}

#[test]
fn live_contact_density_controls_movement_contact_with_liquid() {
    // C4Shape::ContactCheck compares landscape density against the live
    // Shape.ContactDensity (C4Shape.cpp:83-156; C4Movement.cpp:337-470).
    // This is the mechanism used by Fantasy's WalkOnLiquid spell, which sets
    // the target to C4M_Liquid and later restores C4M_Solid
    // (WalkOnLiquid.c4d/Script.c:105,131).
    let mut actions = HashMap::new();
    actions.insert(
        "Flight".to_string(),
        ActionSpec::default().with_procedure("flight"),
    );
    let mut definition = Definition::from_script(
        "WALK",
        "Liquid attachment probe",
        "#strict\nfunc Enable() { return SetContactDensity(C4M_Liquid); }\n",
    )
    .expect("probe compiles");
    definition.configure_actions(Some("Flight".to_string()), actions);
    definition.set_category(CATEGORY_OBJECT);
    definition.set_shape_vertices(vec![ObjectVertex::new(0, 0).with_cnat(CNAT_BOTTOM)]);

    let mut engine = Engine::new();
    engine.set_landscape(raster_landscape_with_densities(
        80,
        60,
        vec![0, 25],
        |_, y| u8::from(y >= 20),
    ));
    engine.set_physics(PhysicsSettings::new(100, 20, -20));
    engine
        .register_definition(definition)
        .expect("probe registers");
    let walker = engine
        .spawn_object(
            SpawnConfig::new("WALK")
                .with_position(Vector2::new(20, 18))
                .with_velocity(Vector2::new(0, 3))
                .with_mobile(true)
                .with_action(ActionState::new("Flight")),
        )
        .expect("liquid walker spawns");
    let falling_control = engine
        .spawn_object(
            SpawnConfig::new("WALK")
                .with_position(Vector2::new(40, 18))
                .with_velocity(Vector2::new(0, 3))
                .with_mobile(true)
                .with_action(ActionState::new("Flight")),
        )
        .expect("solid-density control spawns");
    let walker_index = engine.find_object_index(walker).expect("walker exists");
    assert_eq!(
        engine
            .call_object_function(walker_index, "Enable", Vec::new())
            .expect("WalkOnLiquid-style mutation succeeds"),
        Value::Bool(true)
    );

    engine.tick_without_snapshot().expect("movement contact probe ticks");

    let walker = engine.object_snapshot(walker).expect("walker survives");
    let falling = engine
        .object_snapshot(falling_control)
        .expect("control survives");
    assert_eq!(walker.contact_density, 25);
    assert_eq!(
        walker.position.y, 19,
        "liquid-density shape stops at the liquid surface"
    );
    assert!(
        falling.position.y > walker.position.y,
        "solid-density control should pass through density-25 liquid: {falling:?}"
    );
}

#[test]
fn script_jump_native_runs_on_action_jump_before_hardcoded_launch() {
    // FnJump delegates to ObjectComJump (C4Script.cpp:358-363), whose
    // non-dive fallback calls OnActionJump with precision 100 before any
    // hardcoded action or velocity write (C4ObjectCom.cpp:48-61,280-307).
    let mut actions = HashMap::new();
    actions.insert(
        "Walk".to_string(),
        ActionSpec::default().with_procedure("walk"),
    );
    actions.insert(
        "Jump".to_string(),
        ActionSpec::default().with_procedure("flight"),
    );
    let script = r#"
#strict
local jump_calls, jump_xdir, jump_ydir, jump_by_com, allow_hardcoded;
protected func OnActionJump(int xdir, int ydir, bool by_com)
{
    jump_calls++;
    jump_xdir = xdir;
    jump_ydir = ydir;
    jump_by_com = by_com;
    return !allow_hardcoded;
}
func Probe() { return Jump(); }
func ProbeFallback() { allow_hardcoded = true; return Jump(); }
"#;
    let mut definition =
        Definition::from_script("JHOK", "Jump hook probe", script).expect("probe compiles");
    definition.configure_actions(Some("Walk".to_string()), actions);
    definition.set_physical(PhysicalInfo {
        walk: 70_000,
        jump: 40_000,
        ..Default::default()
    });

    let mut engine = Engine::new();
    engine
        .register_definition(definition)
        .expect("probe registers");
    let object = engine
        .spawn_object(
            SpawnConfig::new("JHOK")
                .with_action(ActionState::new("Walk"))
                .with_direction(Direction::Right)
                .with_command_direction(CommandDirection::Right),
        )
        .expect("probe spawns");
    let index = engine.find_object_index(object).expect("probe exists");

    assert_eq!(
        engine
            .call_object_function(index, "Probe", Vec::new())
            .expect("Probe calls native Jump"),
        Value::Bool(true)
    );
    let snapshot = engine.object_snapshot(object).expect("probe survives");
    assert_eq!(snapshot.action.name, "Walk");
    assert_eq!(snapshot.velocity, Vector2::ZERO);
    assert_eq!(snapshot.local_vars.get("jump_calls"), Some(&Value::Int(1)));
    assert_eq!(snapshot.local_vars.get("jump_xdir"), Some(&Value::Int(196)));
    assert_eq!(snapshot.local_vars.get("jump_ydir"), Some(&Value::Int(-400)));
    assert_eq!(snapshot.local_vars.get("jump_by_com"), Some(&Value::Bool(true)));

    let index = engine.find_object_index(object).expect("probe remains");
    assert_eq!(
        engine
            .call_object_function(index, "ProbeFallback", Vec::new())
            .expect("false hook allows the hardcoded jump"),
        Value::Bool(true)
    );
    let snapshot = engine.object_snapshot(object).expect("probe survives");
    assert_eq!(snapshot.action.name, "Jump");
    assert_eq!(snapshot.velocity, Vector2::new(2, -4));
    assert!(snapshot.mobile);
    assert_eq!(snapshot.local_vars.get("jump_calls"), Some(&Value::Int(2)));
}
