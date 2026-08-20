// This file is an item fragment included inside `mod tests`; the outer module
// supplies its indentation.
const EARTH_DIG_FREE_MATERIAL_SOURCE: &str = r#"
            [Material Earth]
            Name=Earth
            Density=100
            DigFree=1
        "#;

const EARTH_80_DIG_FREE_MATERIAL_SOURCE: &str = r#"
            [Material Earth]
            Name=Earth
            Density=80
            DigFree=1
            "#;

const EARTH_80_FRICTION_DIG_FREE_MATERIAL_SOURCE: &str = r#"
            [Material Earth]
            Name=Earth
            Density=80
            Friction=25
            DigFree=1
        "#;

const EARTH_AND_WATER_MATERIAL_SOURCE: &str = r#"
            [Material Earth]
            Name=Earth
            Density=100

            [Material Water]
            Name=Water
            Density=25
            Instable=1
        "#;

const EARTH_ROCK_DIG_OBJECT_MATERIAL_SOURCE: &str = r#"
            [Material Earth]
            Name=Earth
            Density=80
            DigFree=1
            Dig2Object=GEMA
            Dig2ObjectRatio=1

            [Material Rock]
            Name=Rock
            Density=100
            DigFree=1
            Dig2Object=GEMB
            Dig2ObjectRatio=1
        "#;

const EARTH_GRANITE_DIG_MATERIAL_SOURCE: &str = r#"
            [Material Earth]
            Name=Earth
            Density=80
            DigFree=1

            [Material Granite]
            Name=Granite
            Density=100
            DigFree=0
            "#;

const GRANITE_BLAST_SHIFT_MATERIAL_SOURCE: &str = r#"
            [Material Granite]
            Name=Granite
            Density=110
            Friction=35
            BlastShiftTo=Earth

            [Material Earth]
            Name=Earth
            Density=90
            Friction=25
        "#;

macro_rules! spawn_test {
    ($engine:expr, $definition:expr $(, $setter:ident: $value:expr)* $(,)?) => {
        ($engine).spawn_test_object(
            SpawnConfig::new($definition)$(.$setter($value))*
        )
    };
}

macro_rules! spawn {
    ($engine:expr, $definition:expr $(, $setter:ident: $value:expr)* $(,)?) => {
        ($engine).spawn_object(
            SpawnConfig::new($definition)$(.$setter($value))*
        )
    };
}

macro_rules! definition_fixture_case {
    ($seed:expr, $definition:expr, $id:expr $(, $setter:ident: $value:expr)* $(,)?) => {
        definition_fixture(
            $seed,
            $definition,
            SpawnConfig::new($id)$(.$setter($value))*,
        )
    };
}

macro_rules! physical {
    ($($field:ident: $value:expr),+ $(,)?) => {
        PhysicalInfo {
            $($field: $value,)+
            ..PhysicalInfo::default()
        }
    };
}

macro_rules! action_spec {
    (default $(, $setter:ident: $value:expr)* $(,)?) => {
        ActionSpec::default()$(.$setter($value))*
    };
}

macro_rules! movement_profile {
    ($($setter:ident: $value:expr),+ $(,)?) => {
        MovementProfile::default()$(.$setter($value))+
    };
}

macro_rules! action_definition_fixture {
    ($id:expr, $name:expr, $source:expr, $default:expr;
     $($action:expr => $spec:expr),+ $(,)?) => {
        action_definition(
            $id,
            $name,
            $source,
            $default,
            [$(($action, $spec)),+],
        )
    };
}

macro_rules! set_actions {
    ($definition:expr, $default:expr; $($action:expr => $spec:expr),+ $(,)?) => {
        set_test_actions(
            $definition,
            $default,
            [$(($action, $spec)),+],
        )
    };
}

macro_rules! assert_locals {
    ($object:expr; $($name:expr => $expected:expr $(, $message:expr)?;)+) => {{
        let object = &$object;
        $(assert_eq!(object.state.local_vars.get($name), $expected $(, $message)?);)+
    }};
}

macro_rules! assert_snapshot_locals {
    ($object:expr; $($name:expr => $expected:expr $(, $message:expr)?;)+) => {{
        let object = &$object;
        $(assert_eq!(object.local_vars.get($name), $expected $(, $message)?);)+
    }};
}

macro_rules! assert_values {
    ($($actual:expr => $expected:expr $(, $message:expr)?;)+) => {
        $(assert_eq!($actual, $expected $(, $message)?);)+
    };
}

fn set_test_actions<const N: usize>(
    definition: &mut Definition,
    default_action: Option<&str>,
    actions: [(&str, ActionSpec); N],
) {
    definition.configure_actions(
        default_action.map(str::to_owned),
        HashMap::from(actions.map(|(name, spec)| (name.to_owned(), spec))),
    );
}

fn action_definition<const N: usize>(
    id: impl Into<String>,
    name: impl Into<String>,
    source: &str,
    default_action: Option<&str>,
    actions: [(&str, ActionSpec); N],
) -> Definition {
    let mut definition = test_definition(id, name, source);
    set_test_actions(&mut definition, default_action, actions);
    definition
}

fn procedure_definition(
    id: &str,
    name: &str,
    source: &str,
    action: &str,
    procedure: &str,
) -> Definition {
    action_definition(
        id,
        name,
        source,
        Some(action),
        procedure_actions([(action, procedure)]),
    )
}

fn procedure_actions<'a, const N: usize>(
    actions: [(&'a str, &'a str); N],
) -> [(&'a str, ActionSpec); N] {
    actions.map(|(name, procedure)| (name, ActionSpec::for_procedure(procedure)))
}

fn movement_procedure_definition(id: &str, action: &str, procedure: &str) -> Definition {
    procedure_definition(id, id, PROCEDURE_MOVEMENT_SCRIPT, action, procedure)
}

fn movement_procedure_engine(seed: u64, id: &str, action: &str, procedure: &str) -> Engine {
    definition_engine(seed, movement_procedure_definition(id, action, procedure))
}

fn definition_engine(seed: u64, definition: Definition) -> Engine {
    let mut engine = Engine::with_seed(seed);
    engine.register_test_definition(definition);
    engine
}

macro_rules! action_definitions_engine {
    ($seed:expr; $($definition:expr),+ $(,)?) => {{
        let mut engine = Engine::with_seed($seed);
        $(engine.register_test_definition($definition);)+
        engine
    }};
}

fn action_horizontal_physics() -> PhysicsSettings {
    PhysicsSettings::new(0, 20, -20)
        .with_max_horizontal_speed(20)
        .test_value()
}

fn definition_fixture(
    seed: u64,
    definition: Definition,
    config: SpawnConfig,
) -> (Engine, ObjectId) {
    let mut engine = definition_engine(seed, definition);
    let id = engine.spawn_test_object(config);
    (engine, id)
}

fn test_object(engine: &Engine, id: ObjectId) -> &Object {
    &engine.objects[engine.test_object_index(id)]
}

fn targeted_action(name: &str, target: ObjectId) -> ActionState {
    let mut action = ActionState::new(name);
    action.target = Some(target);
    action
}

fn square_vertices(half_extent: i32) -> Vec<ObjectVertex> {
    vec![
        ObjectVertex::new(-half_extent, -half_extent),
        ObjectVertex::new(half_extent, -half_extent),
        ObjectVertex::new(half_extent, half_extent),
        ObjectVertex::new(-half_extent, half_extent),
    ]
}

fn action_materials(source: &str) -> MaterialSet {
    let library = MaterialLibrary::parse(source).test_value();
    MaterialSet::from_resource_library(&library)
}

fn action_materials_with_id(source: &str, name: &str) -> (MaterialSet, MaterialId) {
    let materials = action_materials(source);
    let id = materials.id_of(name).test_value();
    (materials, id)
}

fn action_resource_data(
    directory: &str,
    def_core: &[u8],
    act_map: &[u8],
    script: &[u8],
) -> ResourceDefinitionData {
    let temp = tempfile::tempdir().test_value();
    let definition_path = temp.path().join(directory);
    std::fs::create_dir(&definition_path).test_value();
    for (name, contents) in [
        ("DefCore.txt", def_core),
        ("ActMap.txt", act_map),
        ("Script.c", script),
    ] {
        std::fs::write(definition_path.join(name), contents).test_value();
    }
    let group = clonk_resources::Group::open(&definition_path).test_value();
    ResourceDefinitionData::load(&group).test_value()
}

fn dig_free_definition(id: &str, dig_free: i32) -> Definition {
    let mut definition = action_definition(
        id,
        "Digger",
        PROCEDURE_MOVEMENT_SCRIPT,
        Some("Dig"),
        [(
            "Dig",
            action_spec!(default, with_procedure: "dig", with_dig_free: dig_free),
        )],
    );
    definition.set_category(CATEGORY_OBJECT);
    definition.set_shape_vertices(vec![ObjectVertex::new(0, 1).with_cnat(CNAT_BOTTOM)]);
    definition.set_contact_density(50);
    definition
}

fn dig_gem_definition() -> Definition {
    test_definition(
        "GEM_",
        "Gem",
        "global func Initialize(state, random) { return 0; }\n",
    )
}

fn component(id: &str, count: i32) -> DefinitionComponent {
    DefinitionComponent {
        id: id.to_owned(),
        count,
    }
}

fn control_actor_fixture(script: &str) -> Result<(Engine, ObjectId), EngineError> {
    let mut definition = action_definition(
        "CLNK",
        "Clonk",
        script,
        Some("Idle"),
        procedure_actions([("Idle", "walk"), ("Dig", "dig")]),
    );
    definition.set_movement_profile(MovementProfile::default());

    let mut engine = Engine::new();
    engine.register_definition(definition)?;
    engine.register_player(PlayerConfig::new(1, "Test"))?;
    let object = spawn_test!(engine, "CLNK", with_owner: 1, with_crew_member: true, with_action: ActionState::new("Idle"));
    engine.set_crew_cursor(1, Some(object))?;
    Ok((engine, object))
}

fn script_object_fixture(
    seed: u64,
    id: impl Into<String>,
    name: impl Into<String>,
    source: &str,
    config: SpawnConfig,
) -> (Engine, ObjectId) {
    let mut engine = script_engine(seed, id, name, source);
    let object = engine.spawn_test_object(config);
    (engine, object)
}

fn script_engine(
    seed: u64,
    id: impl Into<String>,
    name: impl Into<String>,
    source: &str,
) -> Engine {
    let mut engine = Engine::with_seed(seed);
    engine.register_test_script_definition(id, name, source);
    engine
}

#[test]
fn flight_procedure_suppresses_gravity_and_wind() {
    let mut engine = movement_procedure_engine(1, "Glider", "Fly", "flight");

    let physics = PhysicsSettings::checked(4, 12, -20)
        .test_value()
        .with_max_horizontal_speed(24)
        .test_value();
    engine.set_physics(physics);
    engine.set_environment(EnvironmentSettings::new(5));

    let id = engine.spawn_test_object(SpawnConfig::new("Glider").with_category(CATEGORY_OBJECT));

    let object = tick_test_object(&mut engine, id);
    unit_assert_eq!(object.velocity.y => 0);
    unit_assert_eq!(object.velocity.x => 0);
    assert_values! {
        object.fixed_velocity.expect("gravity should remain sub-pixel").y.val() => 524;
    }
}

#[test]
fn flight_command_direction_updates_velocity() {
    let mut definition = movement_procedure_definition("Glider", "Fly", "flight");
    definition
        .set_movement_profile(movement_profile!(with_float_speed: 6, with_float_acceleration: 3));

    let mut engine = definition_engine(3, definition);
    engine.set_environment(EnvironmentSettings::new(0));

    let id = spawn_test!(engine, "Glider", with_category: CATEGORY_OBJECT, with_command_direction: CommandDirection::DownRight);

    let object = tick_test_object(&mut engine, id);
    // DFA_FLIGHT is gravity + Mobile only (C4Object.cpp:4875-4886):
    // ComDir never steers a flier, so only GravAccel accumulates.
    unit_assert_eq!(object.velocity => Vector2::new(0, 0));
    assert_values! {
        object.fixed_velocity.map(|velocity| velocity.y.val()).unwrap_or(0) => engine.physics.gravity_as_c4fixed().val();
    }
}

#[test]
fn float_procedure_reduces_gravity_pull() {
    let mut engine = movement_procedure_engine(2, "Balloon", "Float", "float");

    let physics = PhysicsSettings::checked(6, 20, -30)
        .test_value()
        .with_max_horizontal_speed(20)
        .test_value();
    engine.set_physics(physics);
    engine.set_environment(EnvironmentSettings::new(0));

    let id = engine.spawn_test_object(SpawnConfig::new("Balloon").with_category(CATEGORY_OBJECT));

    let object = tick_test_object(&mut engine, id);
    unit_assert_eq!(object.velocity.y => 0);
    // DFA_FLOAT never runs DoGravity (C4Object.cpp:5268-5290): a
    // floater with no ComDir input holds its velocity exactly.
    assert_values! {
        object.fixed_velocity.map(|velocity| velocity.y.val()).unwrap_or(0) => 0;
    }
}

#[test]
fn float_command_direction_updates_velocity() {
    let mut definition = movement_procedure_definition("Balloon", "Float", "float");
    // A direct setter is the unit-fixture equivalent of an explicit
    // MovementManifest; without it, DFA_FLOAT follows the native zero bound.
    definition
        .set_movement_profile(movement_profile!(with_float_speed: 6, with_float_acceleration: 2));

    let mut engine = definition_engine(5, definition);

    engine.set_environment(EnvironmentSettings::new(0));

    let id = spawn_test!(engine, "Balloon", with_category: CATEGORY_OBJECT, with_command_direction: CommandDirection::UpRight);

    let object = tick_test_object(&mut engine, id);
    unit_assert_eq!(object.velocity => Vector2::new(2, -2));
    // No gravity rides on DFA_FLOAT (C4Object.cpp:5268-5290): the
    // velocity is exactly the accumulated float acceleration.
    assert_values! {
        object.fixed_velocity.map(|velocity| velocity.y.val()).unwrap_or(object.velocity.y << 16) => -131072;
    }
}

#[test]
fn float_without_physical_or_movement_manifest_freezes_raw_velocity() {
    let definition = action_definition_fixture!(
        "Balloon",
        "Balloon",
        PROCEDURE_MOVEMENT_SCRIPT,
        Some("Float");
        "Float" => ActionSpec::default().with_procedure("FLOAT"),
    );

    let mut engine = definition_engine(5, definition);
    engine.set_environment(EnvironmentSettings::new(0));

    let id = spawn_test!(engine, "Balloon", with_category: CATEGORY_OBJECT, with_command_direction: CommandDirection::Stop);
    let index = engine.test_object_index(id);
    for _ in 0..2 {
        engine.objects[index].set_fixed_velocity(FixedVec2::new(
            C4Fixed::from_raw(123_456),
            C4Fixed::from_raw(-654_321),
        ));

        engine.apply_physics_at_index(index).test_value();

        // C4Object::ExecAction clamps both raw axes to FIXED100(Float), including
        // zero when [Physical] Float is absent (src/C4Object.cpp:5291-5309).
        assert_values! {
            engine.objects[index].fixed_velocity => FixedVec2::ZERO,
            "DFA_FLOAT with no Float physical must clamp both raw axes to zero \
             (oracle-src-pinned src/C4Object.cpp:5291-5309)";
        }
    }
}

#[test]
fn resource_float_without_float_physical_clamps_raw_velocity() {
    let resource = action_resource_data(
        "Sub.c4d",
        b"[DefCore]\nid=SUB1\nName=Submarine\n\n[Physical]\nEnergy=100000\nWalk=30000\nSwim=100000\nPush=30000\n",
        b"[Action]\nName=Grapple\nProcedure=FLOAT\nLength=12\nDelay=1\n\n[Action]\nName=Turn\nProcedure=FLOAT\nLength=24\nDelay=1\n",
        b"#strict\n",
    );
    assert_values! {
        Definition::from_resource(&resource).test_value().physical().float => 0;
    }
    let mut definition = test_definition("SUB1", "Submarine", "#strict\n");
    Engine::apply_resource_core(&mut definition, &resource.core);
    set_actions!(
        &mut definition, None;
        "Grapple" => action_spec!(default, with_procedure: "FLOAT", with_length: 12, with_delay: 1),
        "Turn" => action_spec!(default, with_procedure: "FLOAT", with_length: 24, with_delay: 1),
    );
    unit_assert_eq!(definition.physical().float => 0);

    let mut engine = definition_engine(0, definition);
    for action in ["Grapple", "Turn"] {
        let object = spawn_test!(engine, "SUB1", with_action: ActionState::new(action), with_category: CATEGORY_OBJECT, with_mobile: true);
        let index = engine.test_object_index(object);
        let approach_velocity =
            FixedVec2::new(C4Fixed::from_raw(123_456), C4Fixed::from_raw(-654_321));
        engine.objects[index].set_fixed_velocity(approach_velocity);

        engine.apply_physics_at_index(index).test_value();

        assert_values! {
            engine.objects[index].fixed_velocity => FixedVec2::ZERO,
            "{action} DFA_FLOAT clamps both axes to \
             FIXED100(Physical.Float)=0 (oracle-src-pinned \
             src/C4Object.cpp:5291-5310)";
        }
    }
}

#[test]
fn zero_physical_resource_float_clamps_raw_velocity() {
    let resource = action_resource_data(
        "Precipitation.c4d",
        b"[DefCore]\nid=FXP1\nName=Wolke\n",
        b"[Action]\nName=Process\nProcedure=FLOAT\nLength=15\nDelay=2\nNextAction=Process\n",
        b"#strict\n",
    );
    unit_assert_eq!(resource.core.physical => PhysicalInfo::default());

    let mut definition = test_definition("FXP1", "Wolke", "#strict\n");
    Engine::apply_resource_core(&mut definition, &resource.core);
    set_actions!(
        &mut definition, Some("Process");
        "Process" => action_spec!(default, with_procedure: "FLOAT", with_length: 15, with_delay: 2, with_next: "Process"),
    );

    let (mut engine, object) = definition_fixture_case!(0, definition, "FXP1", with_action: ActionState::new("Process"), with_category: CATEGORY_OBJECT, with_mobile: true);
    let index = engine.test_object_index(object);
    engine.objects[index].set_fixed_velocity(FixedVec2::new(
        C4Fixed::from_raw(123_456),
        C4Fixed::from_raw(-654_321),
    ));

    engine.apply_physics_at_index(index).test_value();

    assert_values! {
        engine.objects[index].fixed_velocity => FixedVec2::ZERO, concat!(
            "zero-default C4PhysicalInfo still clamps DFA_FLOAT to ",
            "FIXED100(Physical.Float)=0 (oracle-src-pinned ",
            "src/C4InfoCore.cpp:239-242; src/C4Object.cpp:5291-5310)"
        );
    }
}

#[test]
fn float_physical_preserves_hazard_bullet_velocity_above_synthetic_limit() {
    // Hazard's SHT1 Travel action uses DFA_FLOAT with Float=100000. C++
    // clamps xdir/ydir only to FIXED100(Float), then sets Mobile
    // (oracle-src-pinned src/C4Object.cpp:5291-5310); it has no global
    // 12 px/frame cap after the procedure. The raw velocity is a
    // representative Pistol Fire1 launch at 76 degrees.
    let script = format!("{PROCEDURE_MOVEMENT_SCRIPT}\nfunc Traveling() {{ return true; }}\n");
    let mut definition = action_definition_fixture!(
        "SHT1",
        "Shot",
        &script,
        Some("Travel");
        "Travel" => action_spec!(default, with_procedure: "FLOAT", with_delay: 1, with_length: 1, with_next: "Travel", with_start_call: "Traveling"),
    );
    definition.set_physical(physical! {
            float: 100_000
    });
    definition.set_incomplete_activity(true);

    let mut engine = definition_engine(0, definition);
    let launch_velocity = FixedVec2::new(C4Fixed::from_raw(1_592_524), C4Fixed::from_raw(-393_216));
    let bullet = engine.spawn_test_object(SpawnConfig::new("SHT1").with_category(CATEGORY_OBJECT));
    let bullet_idx = engine.test_object_index(bullet);
    engine.objects[bullet_idx].set_fixed_velocity(launch_velocity);
    unit_assert_eq!(engine.objects[bullet_idx].fixed_velocity => launch_velocity, "script launch keeps raw C4Fixed velocity before ExecAction");
    unit_assert_eq!(engine.objects[bullet_idx].state.action.name => "Travel");

    engine.apply_physics_at_index(bullet_idx).test_value();

    unit_assert_eq!(engine.objects[bullet_idx].fixed_velocity => launch_velocity, "DFA_FLOAT must not steepen the bullet by clamping only its horizontal speed");

    engine.tick_without_snapshot().test_value();
    let bullet_idx = engine.test_object_index(bullet);
    unit_assert_eq!(engine.objects[bullet_idx].fixed_velocity => launch_velocity, "callback outcome folds must preserve the same native DFA_FLOAT velocity");
}

#[test]
fn float_callback_uses_same_outcome_physical_before_terminal_clamp() {
    // SetPhysical mutates the live C++ object before the following
    // SetXDir/SetYDir calls return from the callback
    // (oracle-src-pinned src/C4Script.cpp:557-601). DFA_FLOAT then owns
    // the only speed bounds (src/C4Object.cpp:5291-5310).
    let script = format!(
        r#"{PROCEDURE_MOVEMENT_SCRIPT}
global func Step(state, frame, random) {{
    if (frame == 1) {{
        SetPhysical("Float", 100000, 2);
        SetXDir(243, this(), 10);
        SetYDir(-60, this(), 10);
    }}
    return 0;
}}

func ArmBullet() {{
    SetPhysical("Float", 100000, 2);
    SetXDir(243, this(), 10);
    SetYDir(-60, this(), 10);
}}
"#
    );
    let definition = action_definition_fixture!(
        "SHT1",
        "Shot",
        &script,
        Some("Travel");
        "Travel" => ActionSpec::default().with_procedure("FLOAT"),
    );

    let (mut engine, bullet) =
        definition_fixture_case!(0, definition, "SHT1", with_category: CATEGORY_OBJECT);
    let bullet_idx = engine.test_object_index(bullet);

    engine.call_test_object_function(bullet_idx, "ArmBullet", Vec::new());

    assert_values! {
        engine.objects[bullet_idx].state.temporary_physical.map(|physical| physical.float) => Some(100_000);
        (
            engine.objects[bullet_idx].fixed_velocity.x.val(),
            engine.objects[bullet_idx].fixed_velocity.y.val(),
        ) => (1_592_524, -393_216),
            "the fold must resolve Float after applying the callback's physical update";
    }

    let stepped_bullet =
        engine.spawn_test_object(SpawnConfig::new("SHT1").with_category(CATEGORY_OBJECT));
    engine.tick_without_snapshot().test_value();
    let stepped_idx = engine.test_object_index(stepped_bullet);
    assert_values! {
        (
            engine.objects[stepped_idx].fixed_velocity.x.val(),
            engine.objects[stepped_idx].fixed_velocity.y.val(),
        ) => (1_592_524, -393_216),
            "the Step fold must also resolve Float after its physical update";
    }
}

#[test]
fn swim_procedure_reduces_gravity_and_blocks_wind() {
    let mut engine = movement_procedure_engine(7, "Swimmer", "Swim", "swim");

    let physics = PhysicsSettings::checked(6, 20, -30)
        .test_value()
        .with_max_horizontal_speed(20)
        .test_value();
    engine.set_physics(physics);
    engine.set_environment(EnvironmentSettings::new(5));

    let id = engine.spawn_test_object(SpawnConfig::new("Swimmer"));

    let object = tick_test_object(&mut engine, id);
    unit_assert_eq!(object.velocity.y => 0);
    unit_assert_eq!(object.velocity.x => 0);
    // DFA_SWIM steers with SwimAccel only — no GravAccel component
    // (C4Object.cpp:4920-4985).
    assert_values! {
        object.fixed_velocity.map(|velocity| velocity.y.val()).unwrap_or(0) => 0;
    }
}

#[test]
fn swim_command_direction_updates_velocity_and_stop_decelerates() {
    let mut definition = movement_procedure_definition("Swimmer", "Swim", "swim");
    definition
        .set_movement_profile(movement_profile!(with_swim_speed: 10, with_swim_acceleration: 2));

    let mut engine = definition_engine(11, definition);

    let physics = PhysicsSettings::checked(0, 20, -20)
        .test_value()
        .with_max_horizontal_speed(20)
        .test_value();
    engine.set_physics(physics);
    engine.set_environment(EnvironmentSettings::new(0));

    let id = spawn_test!(engine, "Swimmer", with_command_direction: CommandDirection::DownRight);

    // C4Object InLiquid: these fixtures have no water — arm the flag
    // so the DFA_SWIM out-of-liquid exit (C4Object.cpp:4946-4956)
    // does not convert the swimmer to Walk.
    {
        let idx = engine.test_object_index(id);
        engine.objects[idx].state.in_liquid = true;
    }
    let object = tick_test_object(&mut engine, id);
    unit_assert_eq!(object.velocity => Vector2::new(2, 2));

    let object = tick_test_object(&mut engine, id);
    unit_assert_eq!(object.velocity => Vector2::new(4, 4));

    engine
        .apply_object_update(
            id,
            ObjectUpdate::new().with_command_direction(CommandDirection::Stop),
        )
        .test_value();

    let object = tick_test_object(&mut engine, id);
    unit_assert_eq!(object.velocity => Vector2::new(2, 2));

    let object = tick_test_object(&mut engine, id);
    unit_assert_eq!(object.velocity => Vector2::new(0, 0));
}

#[test]
fn lift_procedure_matches_cpp_mass_scaled_force_and_terminal_speeds() {
    let lifter_definition = build_lift_definition("Lifter");
    let mut crate_definition = build_idle_definition("Crate");
    crate_definition.set_mass(100);

    let mut engine = action_definitions_engine!(31; lifter_definition, crate_definition);
    // Deliberately tighter than Lift's +/-2 targets: C++ Lift does not
    // apply the generic terminal-speed clamp.
    engine.set_physics(PhysicsSettings::new(20, 1, -1));

    let target_id =
        engine.spawn_test_object(SpawnConfig::new("Crate").with_category(CATEGORY_OBJECT));
    let target_idx = engine.test_object_index(target_id);
    engine.objects[target_idx].set_fixed_velocity(FixedVec2::ZERO);
    engine.objects[target_idx].state.mobile = true;

    let lift_action = targeted_action("Lift", target_id);

    let lifter_id = spawn_test!(engine, "Lifter", with_category: CATEGORY_OBJECT, with_action: lift_action, with_command_direction: CommandDirection::Up);

    let lifter_idx = engine.test_object_index(lifter_id);
    let target_definition_id = engine.objects[target_idx].definition_id.clone();
    let target_actions = engine
        .definition(&target_definition_id)
        .test_value()
        .action_library()
        .clone();
    let mut expected_fix_y = engine.objects[target_idx].fixed_position.y.val();

    // C4Object.cpp:1847-1855,5269-5280: FIXED100(50)*100/Mass=0.5
    // works toward the constant +/-2 target without terminal-speed
    // clamping. Each step is followed by the target's C++ DoMovement
    // integration so both ydir and fix_y are pinned byte-for-byte.
    for expected in [-32_768, -65_536, -98_304, -131_072, -131_072] {
        engine.apply_physics_at_index(lifter_idx).test_value();
        unit_assert_eq!(engine.objects[target_idx].fixed_velocity.y.val() => expected);
        engine
            .exec_object_movement(target_idx, &target_actions, &target_definition_id, &[])
            .test_value();
        expected_fix_y += expected;
        unit_assert_eq!(engine.objects[target_idx].fixed_position.y.val() => expected_fix_y);
    }

    engine.objects[lifter_idx].state.command_direction = CommandDirection::Down;
    for expected in [
        -98_304, -65_536, -32_768, 0, 32_768, 65_536, 98_304, 131_072,
    ] {
        engine.apply_physics_at_index(lifter_idx).test_value();
        unit_assert_eq!(engine.objects[target_idx].fixed_velocity.y.val() => expected);
        engine
            .exec_object_movement(target_idx, &target_actions, &target_definition_id, &[])
            .test_value();
        expected_fix_y += expected;
        unit_assert_eq!(engine.objects[target_idx].fixed_position.y.val() => expected_fix_y);
    }

    engine.objects[lifter_idx].state.command_direction = CommandDirection::Stop;
    for expected in [98_304, 65_536, 32_768, 0, -2_621] {
        engine.apply_physics_at_index(lifter_idx).test_value();
        unit_assert_eq!(engine.objects[target_idx].fixed_velocity.y.val() => expected);
        engine
            .exec_object_movement(target_idx, &target_actions, &target_definition_id, &[])
            .test_value();
        expected_fix_y += expected;
        unit_assert_eq!(engine.objects[target_idx].fixed_position.y.val() => expected_fix_y);
    }
    // COMD_Stop's target is exactly -GravAccel. The target's own
    // DoGravity therefore cancels it byte-for-byte in the same cycle.
    let held_fix_y = engine.objects[target_idx].fixed_position.y;
    engine.apply_physics_at_index(target_idx).test_value();
    unit_assert_eq!(engine.objects[target_idx].fixed_velocity.y => C4Fixed::ZERO);
    engine
        .exec_object_movement(target_idx, &target_actions, &target_definition_id, &[])
        .test_value();
    unit_assert_eq!(engine.objects[target_idx].fixed_position.y => held_fix_y, "-GravAccel plus DoGravity produces no fix_y movement");

    // A second live mass proves this is the C++ division, not a
    // hard-coded half-pixel step: (Def Mass 100 + OwnMass 100) gives
    // 32768*100/200 = raw 16384.
    let mut heavy_definition = build_idle_definition("Heavy");
    heavy_definition.set_mass(100);
    engine.register_test_definition(heavy_definition);
    let heavy_id =
        engine.spawn_test_object(SpawnConfig::new("Heavy").with_position(Vector2::new(7, 9)));
    let heavy_lift = targeted_action("Lift", heavy_id);
    let heavy_lifter_id = spawn_test!(engine, "Lifter", with_action: heavy_lift, with_command_direction: CommandDirection::Up);
    let heavy_idx = engine.test_object_index(heavy_id);
    engine.objects[heavy_idx].state.own_mass = 100;
    engine.objects[heavy_idx].state.mobile = false;
    engine.objects[heavy_idx].fixed_velocity = FixedVec2::new(itofix(3), itofix(4));
    engine.objects[heavy_idx].fixed_position =
        FixedVec2::new(itofix(7) + fixed100(25), itofix(9) + fixed100(25));
    let heavy_lifter_idx = engine.test_object_index(heavy_lifter_id);
    engine.apply_physics_at_index(heavy_lifter_idx).test_value();
    unit_assert_eq!(engine.objects[heavy_idx].fixed_velocity.y.val() => -16_384);
    unit_assert_eq!(engine.objects[heavy_idx].fixed_velocity.x => C4Fixed::ZERO);
    unit_assert_eq!(engine.objects[heavy_idx].fixed_position => FixedVec2::from_ints(7, 9));
    unit_assert!(engine.objects[heavy_idx].state.mobile);
}

#[test]
fn lift_full_tick_matches_cpp_exec_order_for_raw_ydir_and_fix_y() {
    let lifter_definition = build_lift_definition("Lifter");
    let mut target_definition = build_idle_definition("Crate");
    target_definition.set_mass(100);
    let mut landscape = vehicle_grid_landscape(200, 200);
    landscape.set_world_height(200);

    let mut engine = Engine::with_seed(31);
    engine.set_landscape(landscape);
    engine.set_physics(PhysicsSettings::new(20, 20, -20));
    engine.register_test_definition(lifter_definition);
    engine.register_test_definition(target_definition);

    // Loaded objects execute in file order. The target therefore runs
    // DoGravity+DoMovement before the lifter applies its 0.5 force,
    // exactly matching C4Game::ExecObjects for this two-object oracle.
    let target_id = spawn_test!(engine, "Crate", with_category: CATEGORY_OBJECT, with_position: Vector2::new(50, 100), with_fixed_position: FixedVec2::from_ints(50, 100), with_mobile: true, with_loaded: true);
    let lift = targeted_action("Lift", target_id);
    spawn_test!(engine, "Lifter", with_category: CATEGORY_OBJECT, with_position: Vector2::new(50, 120), with_action: lift, with_command_direction: CommandDirection::Up, with_mobile: true, with_loaded: true);

    let initial_fix_y = itofix(100).val();
    for (expected_ydir, expected_fix_delta) in [
        (-30_147, 2_621),
        (-60_294, -24_905),
        (-90_441, -82_578),
        (-120_588, -170_398),
        (-131_072, -288_365),
    ] {
        engine.tick_without_snapshot().test_value();
        let target_idx = engine.test_object_index(target_id);
        unit_assert_eq!(engine.objects[target_idx].fixed_velocity.y.val() => expected_ydir);
        unit_assert_eq!(engine.objects[target_idx].fixed_position.y.val() => initial_fix_y + expected_fix_delta);
    }
}

#[test]
fn lift_contact_reports_stuck_except_for_gravity_hold() {
    let lifter_definition = build_lift_definition("Lifter");
    let target_script = r#"#strict
local stuck_calls;
func Stuck()
{
    stuck_calls = stuck_calls + 1;
}
"#;
    let mut target_definition = test_definition("Crate", "Crate", target_script);
    target_definition.set_mass(100);
    target_definition.set_shape_vertices(vec![ObjectVertex::new(8, 0).with_cnat(CNAT_RIGHT)]);
    target_definition.set_contact_density(50);

    let mut landscape = vehicle_grid_landscape(32, 32);
    landscape.set_world_height(32);
    landscape.grid_write_byte(16, 10, 1);

    let mut engine = Engine::with_seed(31);
    engine.set_landscape(landscape);
    engine.set_physics(PhysicsSettings::new(20, 20, -20));
    engine.register_test_definition(lifter_definition);
    engine.register_test_definition(target_definition);
    let target_id = spawn_test!(engine, "Crate", with_position: Vector2::new(8, 10), with_fixed_position: FixedVec2::from_ints(8, 10));
    let lift = targeted_action("Lift", target_id);
    let lifter_id = spawn_test!(engine, "Lifter", with_action: lift, with_command_direction: CommandDirection::Up);
    let lifter_idx = engine.test_object_index(lifter_id);
    let target_idx = engine.test_object_index(target_id);

    // Unlike Push, Lift runs ContactCheck on every non-hold call
    // (C4Object.cpp:1856-1862), with no Tick35 gate.
    engine.apply_physics_at_index(lifter_idx).test_value();
    unit_assert_eq!(engine.objects[target_idx].frame_t_contact => CNAT_RIGHT);
    assert_values! {
        engine.objects[target_idx].state.local_vars.get("stuck_calls") => Some(&Value::Int(1));
    }
    let message = engine
        .snapshot()
        .hud
        .messages
        .into_iter()
        .next()
        .test_value();
    unit_assert_eq!(message.kind => MessageKind::Target);
    unit_assert_eq!(message.target => Some(target_id));
    unit_assert_eq!(message.lines => vec!["Crate is stuck!"]);
    let message_id = message.id;

    // The exact -GravAccel hold bypasses ContactCheck altogether.
    engine.objects[target_idx].frame_t_contact = CNAT_LEFT;
    engine.objects[lifter_idx].state.command_direction = CommandDirection::Stop;
    engine.apply_physics_at_index(lifter_idx).test_value();
    unit_assert_eq!(engine.objects[target_idx].frame_t_contact => CNAT_LEFT);
    assert_values! {
        engine.objects[target_idx].state.local_vars.get("stuck_calls") => Some(&Value::Int(1));
    }
    let messages = engine.snapshot().hud.messages;
    unit_assert_eq!(messages.len() => 1);
    assert_values! {
        messages[0].id => message_id, "gravity hold must not replace the existing target message";
    }
}

#[test]
fn lift_top_callback_runs_on_lifter_before_its_gravity() {
    let lifter_script = r#"#strict
local lift_top_calls, lift_top_seen_time, lift_top_seen_y_dir, lift_top_reflected;
func LiftTop()
{
    lift_top_calls = lift_top_calls + 1;
    lift_top_seen_time = GetActTime();
    lift_top_seen_y_dir = GetYDir();
    lift_top_reflected = GetDefCoreVal("LiftTop", "DefCore", LIFT);
    SetGravity(40);
    SetYDir(5);
}
"#;
    let temp = tempfile::tempdir().test_value();
    let def_dir = temp.path().join("Lift.ocd");
    std::fs::create_dir(&def_dir).test_value();
    std::fs::write(
        def_dir.join("DefCore.txt"),
        b"[DefCore]\nid=LIFT\nName=Lifter\nCategory=C4D_Object\nLiftTop=20\n",
    )
    .test_value();
    std::fs::write(def_dir.join("Script.c"), lifter_script).test_value();
    std::fs::write(
        def_dir.join("ActMap.txt"),
        b"[Action]\nName=Lift\nProcedure=LIFT\nLength=1\nNextAction=Lift\n",
    )
    .test_value();
    let group = clonk_resources::Group::open(&def_dir).test_value();
    let resource = ResourceDefinitionData::load(&group).test_value();
    let lifter_definition = Definition::from_resource(&resource).test_value();
    unit_assert_eq!(lifter_definition.lift_top() => 20);
    let mut legacy_definition = test_definition("LEGC", "Legacy lifter", "#strict");
    Engine::apply_resource_core(&mut legacy_definition, &resource.core);
    assert_values! {
        legacy_definition.lift_top() => 20, "legacy scenario core mapping retains LiftTop";
    }
    let mut target_definition = build_idle_definition("Crate");
    target_definition.set_mass(100);

    let mut engine = Engine::with_seed(31);
    engine.set_physics(PhysicsSettings::new(20, 20, -20));
    engine.register_test_definition(lifter_definition);
    engine.register_test_definition(target_definition);
    let target_id =
        engine.spawn_test_object(SpawnConfig::new("Crate").with_position(Vector2::new(10, 31)));
    let lift = targeted_action("Lift", target_id);
    let lifter_id = spawn_test!(engine, "LIFT", with_category: CATEGORY_OBJECT, with_position: Vector2::new(10, 10), with_action: lift, with_command_direction: CommandDirection::Up, with_mobile: true);
    let lifter_idx = engine.test_object_index(lifter_id);

    // One pixel outside the inclusive Def->LiftTop threshold must not
    // call the hook even though the command direction is Up.
    engine.apply_physics_at_index(lifter_idx).test_value();
    assert_values! {
        engine.objects[lifter_idx].state.local_vars.get("lift_top_calls") => None;
    }

    let target_idx = engine.test_object_index(target_id);
    engine.objects[target_idx].state.position.y = 30;
    engine.objects[target_idx].fixed_position.y = itofix(30);
    engine.objects[lifter_idx].set_fixed_velocity(FixedVec2::ZERO);

    // Inclusive boundary and order from C4Object.cpp:5281-5289:
    // Action.Time has already advanced; LiftTop sees pre-gravity ydir,
    // changes Gravity to 40 and sets ydir=fixed10(5), then DoGravity
    // consumes the NEW raw GravAccel 5242 in this same call.
    engine.apply_physics_at_index(lifter_idx).test_value();
    assert_locals!(engine.objects[lifter_idx];
        "lift_top_calls" => Some(&Value::Int(1));
        "lift_top_seen_time" => Some(&Value::Int(2));
        "lift_top_seen_y_dir" => Some(&Value::Int(0));
        "lift_top_reflected" => Some(&Value::Int(20));
    );
    assert_values! {
        engine.objects[lifter_idx].fixed_velocity.y.val() => math::fixed10(5).val() + 5_242;
    }

    engine.objects[lifter_idx].state.command_direction = CommandDirection::Down;
    engine.apply_physics_at_index(lifter_idx).test_value();
    assert_values! {
        engine.objects[lifter_idx].state.local_vars.get("lift_top_calls") => Some(&Value::Int(1)),
            "the height alone does not fire LiftTop while moving down";
    }

    engine.objects[lifter_idx].state.command_direction = CommandDirection::Up;
    engine.apply_physics_at_index(lifter_idx).test_value();
    assert_values! {
        engine.objects[lifter_idx].state.local_vars.get("lift_top_calls") => Some(&Value::Int(2)),
            "LiftTop is level-triggered on every qualifying frame";
    }
}

#[test]
fn lift_procedure_resets_without_target() {
    let lifter_definition = build_lift_definition("Lifter");

    let (mut engine, lifter_id) = definition_fixture_case!(37, lifter_definition, "Lifter", with_action: ActionState::new("Lift"), with_command_direction: CommandDirection::Up);

    let lifter = tick_test_object(&mut engine, lifter_id);
    unit_assert_eq!(lifter.action.name => "Idle");
    unit_assert!(lifter.action.target.is_none());

    // Action.Time++ precedes the DFA_LIFT switch. If NoOtherAction
    // rejects SetAction(Idle), C++ returns with the increment retained.
    let mut locked_definition =
        test_definition("LockedLift", "LockedLift", PROCEDURE_MOVEMENT_SCRIPT);
    set_actions!(
        &mut locked_definition, Some("Idle");
        "Idle" => ActionSpec::default(),
        "Lift" => action_spec!(default, with_procedure: "lift", with_no_other_action: true),
    );
    let mut locked_engine = Engine::with_seed(37);
    locked_engine.register_test_definition(locked_definition);
    let locked_id = locked_engine
        .spawn_test_object(SpawnConfig::new("LockedLift").with_action(ActionState::new("Lift")));
    let locked_idx = locked_engine.test_object_index(locked_id);
    unit_assert!(locked_engine.apply_physics_at_index(locked_idx).expect("invalid locked lift resolves"));
    unit_assert_eq!(locked_engine.objects[locked_idx].state.action.name => "Lift");
    unit_assert_eq!(locked_engine.objects[locked_idx].state.action.time => 1);
}

#[test]
fn hang_procedure_locks_vertical_velocity() {
    let mut engine = movement_procedure_engine(11, "Clinger", "Hang", "hang");

    let physics = PhysicsSettings::checked(6, 20, -30)
        .test_value()
        .with_max_horizontal_speed(20)
        .test_value();
    engine.set_physics(physics);
    engine.set_environment(EnvironmentSettings::new(4));

    let id = spawn_test!(engine, "Clinger", with_velocity: Vector2::new(1, 5), with_position: Vector2::new(0, 0));

    let object = tick_test_object(&mut engine, id);
    unit_assert_eq!(object.velocity.y => 0);
    unit_assert_eq!(object.velocity.x => 0);
}

#[test]
fn set_bridge_action_data_updates_action_data() {
    let definition = procedure_definition(
        "Bridger",
        "Bridger",
        SET_BRIDGE_ACTION_DATA_SCRIPT,
        "Bridge",
        "bridge",
    );

    let mut engine = definition_engine(23, definition);

    let id = engine
        .spawn_test_object(SpawnConfig::new("Bridger").with_action(ActionState::new("Bridge")));

    let snapshot = engine.test_object_snapshot(id);
    unit_assert_eq!(snapshot.energy => 1);
    // The fixture has no loaded materials, so C4Action::SetBridgeData
    // clamps material 7 through Num-1 (-1) to the 0xff sentinel.
    let expected = encode_bridge_action_data(200, true, false, -1);
    unit_assert_eq!(snapshot.action.data => expected);
}

#[test]
fn set_bridge_action_data_returns_false_when_not_in_bridge_procedure() {
    let definition = action_definition_fixture!(
        "IdleActor",
        "IdleActor",
        SET_BRIDGE_ACTION_DATA_FAILURE_SCRIPT,
        Some("Idle");
        "Idle" => ActionSpec::default(),
    );

    let (engine, id) = definition_fixture_case!(41, definition, "IdleActor");

    let snapshot = engine.test_object_snapshot(id);
    unit_assert_eq!(snapshot.energy => 0);
    unit_assert_eq!(snapshot.action.data => 0);
}

#[test]
fn bridge_procedure_freezes_velocity_and_ignores_wind() {
    let mut engine = movement_procedure_engine(13, "Bridger", "Bridge", "bridge");

    engine.set_environment(EnvironmentSettings::new(6));

    let id = spawn_test!(engine, "Bridger", with_velocity: Vector2::new(8, -3), with_action: ActionState::new("Bridge"));

    let object = tick_test_object(&mut engine, id);
    unit_assert_eq!(object.velocity => Vector2::ZERO);
}

fn wall_bridge_test_engine(blocker: Option<(usize, usize)>) -> (Engine, MaterialId) {
    let mut definition = test_definition("Bridger", "Bridger", PROCEDURE_MOVEMENT_SCRIPT);
    definition.set_shape_rect(Some(DefinitionRect::new(-5, -10, 10, 20)));
    definition.set_shape_vertices(vec![ObjectVertex::new(0, 0)]);
    definition.set_contact_density(50);
    set_actions!(
        &mut definition, Some("Idle");
        "Idle" => ActionSpec::default(),
        "Bridge" => ActionSpec::default().with_procedure("BRIDGE"),
    );

    let (materials, earth) = action_materials_with_id(EARTH_80_DIG_FREE_MATERIAL_SOURCE, "Earth");

    let mut bytes = vec![0; 160 * 160];
    if let Some((x, y)) = blocker {
        bytes[y * 160 + x] = 1;
    }
    let grid = landscape::PixelGrid::new(
        160,
        160,
        bytes,
        vec![0, 80],
        vec![None, Some("Earth".into())],
        vec![None; 2],
    );
    let mut landscape = Landscape::new(160, vec![160; 160]).test_value();
    landscape.set_world_height(160);
    landscape.set_pixel_grid(grid);

    let mut engine = definition_engine(31, definition);
    engine.set_materials(materials);
    engine.set_landscape(landscape);
    engine.set_physics(PhysicsSettings::new(0, 20, -20));
    (engine, earth)
}

#[test]
fn wall_left_bridge_forces_stationary_progression() {
    // DoBridge locally clears fMoveClonk for wall-Left before calculating
    // dt, checking contact, or calling MovePosition (C4Object.cpp:4587-4590,
    // 4606,4629,4651). The stored action-data bit remains set.
    let (mut engine, earth) = wall_bridge_test_engine(Some((100, 79)));
    let encoded = encode_bridge_action_data(100, true, true, earth.index() as i32);
    let mut action = ActionState::new("Bridge");
    action.data = encoded;
    action.time = 3;
    let id = spawn_test!(engine, "Bridger", with_position: Vector2::new(100, 80), with_fixed_position: FixedVec2::from_ints(100, 80), with_command_direction: CommandDirection::Left, with_mobile: true, with_loaded: true);

    let index = engine.test_object_index(id);
    // C++ load starts at ActIdle and SetAction(BRIDGE) clears Data when
    // the procedure changes (C4Object.cpp:2867-2877,4106-4114). Stage
    // this running BRIDGE state after loading; the test targets DoBridge,
    // not the save loader.
    engine.objects[index].state.action = action;
    engine.apply_physics_at_index(index).test_value();

    let object = engine.test_object_snapshot(id);
    unit_assert_eq!(object.action.time => 4);
    unit_assert_eq!(object.action.data => encoded, "the override is local");
    unit_assert_eq!(object.position => Vector2::new(100, 80));
    unit_assert_eq!(engine.objects[index].fixed_position => FixedVec2::from_ints(100, 80));
    let landscape = engine.landscape().test_value();
    for x in 93..97 {
        unit_assert_eq!(landscape.material_at(x, 89) => Some(earth));
        unit_assert_eq!(landscape.material_at(x, 92) => None);
    }
}

#[test]
fn moving_wall_up_bridge_preserves_doubled_collision_retry() {
    // Wall-Up is the sole wall arm that keeps fMoveClonk. A blocked first
    // step converts the remaining 95 frames into a stationary 190-frame
    // roof at Action.Time 95 and redraws immediately (C4Object.cpp:4631-4645).
    let (mut engine, earth) = wall_bridge_test_engine(Some((101, 79)));
    let mut action = ActionState::new("Bridge");
    action.data = encode_bridge_action_data(100, true, true, earth.index() as i32);
    action.time = 4;
    let id = spawn_test!(engine, "Bridger", with_position: Vector2::new(100, 80), with_fixed_position: FixedVec2::from_ints(100, 80), with_direction: Direction::Right, with_command_direction: CommandDirection::Up, with_mobile: true, with_loaded: true);

    let index = engine.test_object_index(id);
    // A loaded ActIdle -> BRIDGE transition clears Action.Data in C++;
    // inject the already-running action afterward to isolate the blocked
    // DoBridge retry under test.
    engine.objects[index].state.action = action;
    engine.apply_physics_at_index(index).test_value();

    let object = engine.test_object_snapshot(id);
    unit_assert_eq!(object.action.time => 95);
    unit_assert_eq!(object.action.data => encode_bridge_action_data(190, false, true, earth.index() as i32));
    unit_assert_eq!(object.position => Vector2::new(100, 80));
    unit_assert_eq!(engine.objects[index].fixed_position => FixedVec2::from_ints(100, 80));
    let landscape = engine.landscape().test_value();
    for y in 67..70 {
        for x in 98..102 {
            unit_assert_eq!(landscape.material_at(x, y) => Some(earth));
        }
    }
}

#[test]
fn moving_up_left_bridge_uses_action_time_and_draws_cpp_rectangles() {
    // DoBridge (C4Object.cpp:4581-4652): Action.Time has already been
    // incremented when the procedure runs; a moving UpLeft bridge advances
    // at times 6,12,...,96, draws a 4x3 material rect at
    // (x-4, y+Shape.Hgt/2-1), and MovePosition(-1,-1)s the Clonk.
    let mut definition = test_definition("Bridger", "Bridger", PROCEDURE_MOVEMENT_SCRIPT);
    definition.set_shape_rect(Some(DefinitionRect::new(-5, -10, 10, 20)));
    set_actions!(
        &mut definition, Some("Idle");
        "Idle" => ActionSpec::default(),
        "Walk" => ActionSpec::for_procedure("WALK"),
        "Bridge" => ActionSpec::for_procedure("BRIDGE"),
    );

    let (materials, earth) = action_materials_with_id(EARTH_80_DIG_FREE_MATERIAL_SOURCE, "Earth");

    let mut engine = definition_engine(17, definition);
    engine.set_materials(materials);
    engine.set_physics(PhysicsSettings::new(0, 20, -20));
    let grid = landscape::PixelGrid::new(
        160,
        160,
        vec![0; 160 * 160],
        vec![0, 80],
        vec![None, Some("Earth".into())],
        vec![None; 2],
    );
    let mut landscape = Landscape::new(160, vec![160; 160]).test_value();
    landscape.set_world_height(160);
    landscape.set_pixel_grid(grid);
    engine.set_landscape(landscape);

    let mut action = ActionState::new("Bridge");
    action.data = encode_bridge_action_data(100, true, false, earth.index() as i32);

    let id = spawn_test!(engine, "Bridger", with_position: Vector2::new(100, 80), with_fixed_position: FixedVec2::from_ints(100, 80), with_direction: Direction::Right, with_command_direction: CommandDirection::UpLeft, with_mobile: true, with_loaded: true);
    let index = engine.test_object_index(id);
    // Save loading correctly clears BRIDGE data on its DFA_NONE ->
    // DFA_BRIDGE transition. This fixture needs a live, post-transition
    // action so Action.Time drives the C++ DoBridge cadence.
    engine.objects[index].state.action = action;

    for _ in 0..5 {
        engine.tick_without_snapshot().test_value();
    }
    let object = engine.test_object_snapshot(id);
    unit_assert_eq!(object.position => Vector2::new(100, 80));
    unit_assert_eq!(object.action.time => 5);

    engine.tick_without_snapshot().test_value();
    let object = engine.test_object_snapshot(id);
    unit_assert_eq!(object.position => Vector2::new(99, 79));
    let index = engine.test_object_index(id);
    unit_assert_eq!(engine.objects[index].fixed_position => FixedVec2::from_ints(99, 79));
    let landscape = engine.landscape().test_value();
    for y in 89..92 {
        for x in 96..100 {
            unit_assert_eq!(landscape.material_at(x, y) => Some(earth));
        }
    }

    for _ in 6..100 {
        engine.tick_without_snapshot().test_value();
    }

    let object = engine.test_object_snapshot(id);
    unit_assert_eq!(object.position => Vector2::new(84, 64));
    let index = engine.test_object_index(id);
    unit_assert_eq!(engine.objects[index].fixed_position => FixedVec2::from_ints(84, 64));
    unit_assert_eq!(object.direction => Direction::Left);
    unit_assert_eq!(object.action.name => "Walk", "ObjectActionStand selects Walk even though the fixture default is Idle");
    unit_assert_eq!(object.command_direction => CommandDirection::Stop);
    unit_assert_eq!(object.velocity => Vector2::ZERO);
    unit_assert_eq!(object.action.time => 0);
    let index = engine.test_object_index(id);
    unit_assert_eq!(engine.objects[index].frame_t_attach => CNAT_NONE);
    unit_assert_eq!(engine.objects[index].state.t_attach => CNAT_NONE);
}

#[test]
fn blocked_moving_bridge_retries_stationary_and_preserves_ift() {
    // DoBridge's moving collision arm (C4Object.cpp:4631-4646) tests the
    // candidate one pixel upward, converts the remaining duration to a
    // stationary bridge, resets Action.Time to zero, and recursively draws
    // that frame. DrawMaterialRect keeps the destination IFT bit
    // (C4Landscape.cpp:1064-1072).
    let mut definition = test_definition("Bridger", "Bridger", PROCEDURE_MOVEMENT_SCRIPT);
    definition.set_shape_rect(Some(DefinitionRect::new(-5, -10, 10, 20)));
    definition.set_shape_vertices(vec![ObjectVertex::new(0, 0)]);
    definition.set_contact_density(50);
    set_actions!(
        &mut definition, Some("Idle");
        "Idle" => ActionSpec::default(),
        "Bridge" => ActionSpec::default().with_procedure("BRIDGE"),
    );

    let (materials, earth) = action_materials_with_id(EARTH_GRANITE_DIG_MATERIAL_SOURCE, "Earth");

    let mut bytes = vec![0; 160 * 160];
    bytes[78 * 160 + 99] = 2; // candidate CheckContact(99, 78)
    bytes[92 * 160 + 93] = 0x80; // first stationary bridge pixel
    let grid = landscape::PixelGrid::new(
        160,
        160,
        bytes,
        vec![0, 80, 100],
        vec![None, Some("Earth".into()), Some("Granite".into())],
        vec![None; 3],
    );
    let mut landscape = Landscape::new(160, vec![160; 160]).test_value();
    landscape.set_world_height(160);
    landscape.set_pixel_grid(grid);

    let mut engine = definition_engine(19, definition);
    engine.set_materials(materials);
    engine.set_landscape(landscape);
    engine.set_physics(PhysicsSettings::new(0, 20, -20));

    let mut action = ActionState::new("Bridge");
    action.data = encode_bridge_action_data(100, true, false, earth.index() as i32);
    let id = spawn_test!(engine, "Bridger", with_position: Vector2::new(100, 80), with_command_direction: CommandDirection::UpLeft, with_mobile: true, with_loaded: true);
    let index = engine.test_object_index(id);
    // C4Object::CompileFunc cannot preserve Data while selecting a
    // different procedure from ActIdle. Stage the running action after
    // load so this test begins at the collision arm it verifies.
    engine.objects[index].state.action = action;

    for _ in 0..6 {
        engine.tick_without_snapshot().test_value();
    }

    let object = engine.test_object_snapshot(id);
    unit_assert_eq!(object.position => Vector2::new(100, 80));
    unit_assert_eq!(object.action.time => 0);
    let retry = BridgeParameters::from_action_data(object.action.data);
    unit_assert_eq!(retry.duration => 94);
    unit_assert!(!retry.move_clonk);
    assert_values! {
        engine.landscape().expect("landscape remains").grid_byte_at(93, 92) => Some(0x81),
            "stationary retry draws Earth while preserving tunnel IFT";
    }
}

#[test]
fn connect_procedure_freezes_velocity_and_ignores_wind() {
    let mut engine = movement_procedure_engine(29, "Connector", "Connect", "connect");

    engine.set_environment(EnvironmentSettings::new(10));

    let id = spawn_test!(engine, "Connector", with_velocity: Vector2::new(-7, 4), with_action: ActionState::new("Connect"));

    let object = tick_test_object(&mut engine, id);
    unit_assert_eq!(object.velocity => Vector2::ZERO);
}

#[test]
fn object_motion_ignores_wind() {
    // C++ wind reaches only PXS and particles via GBackWind
    // (C4PXS.cpp:67, C4Particles.cpp:652, C4Wrappers.h:189-192) —
    // nothing in C4Movement.cpp/C4Object.cpp ever applies the weather
    // wind to object velocities.
    let script = NOOP_DEFINITION_SCRIPT;

    let definition = test_definition("Crate", "Crate", script);

    let mut engine = definition_engine(4, definition);
    engine.set_physics(PhysicsSettings::new(0, 20, -20));
    engine.set_environment(EnvironmentSettings::new(80));

    let id = engine.spawn_test_object(SpawnConfig::new("Crate").with_position(Vector2::new(0, 0)));
    let idx = engine.test_object_index(id);

    engine.tick_without_snapshot().test_value();
    assert_values! {
        engine.objects[idx].fixed_velocity.x => C4Fixed::ZERO, "weather wind never drives object motion";
    }
}

#[test]
fn kneel_procedure_locks_vertical_velocity_and_blocks_wind() {
    let mut engine = movement_procedure_engine(19, "Kneeler", "Kneel", "kneel");

    engine.set_environment(EnvironmentSettings::new(8));

    let id = spawn_test!(engine, "Kneeler", with_velocity: Vector2::new(5, -4), with_action: ActionState::new("Kneel"));

    let object = tick_test_object(&mut engine, id);
    unit_assert_eq!(object.velocity.y => 0);
    unit_assert_eq!(object.velocity.x => 5);
}

#[test]
fn dig_procedure_zeroes_velocity_when_stopped() {
    let definition = movement_procedure_definition("Digger", "Dig", "dig");

    let mut engine = definition_engine(29, definition);

    engine.set_physics(PhysicsSettings::default());
    engine.set_environment(EnvironmentSettings::new(7));

    let initial_velocity = Vector2::new(4, -3);

    let id = spawn_test!(engine, "Digger", with_velocity: initial_velocity, with_action: ActionState::new("Dig"));

    let object = tick_test_object(&mut engine, id);
    unit_assert_eq!(object.velocity => Vector2::ZERO);
}

#[test]
fn control_command_invokes_object_script() -> Result<(), EngineError> {
    let script = r#"
global func Initialize(state, random) { return 0; }
func ControlDig() { SetAction("Dig"); return true; }
"#;
    let (mut engine, object_id) = control_actor_fixture(script)?;
    let handled = engine.handle_control_command(1, ControlCommand::Dig, CommandKind::Press)?;
    unit_assert!(handled, "control command should report handled");

    let snapshot = engine.snapshot();
    let object = snapshot.object(object_id).test_value();
    unit_assert_eq!(object.action.name => "Dig");
    Ok(())
}

#[test]
fn control_command_coerces_int_returns_like_cpp_bool_cast() -> Result<(), EngineError> {
    // C4Object::CallControl (C4Object.cpp:3300): the Control<Com> result
    // goes through `static_cast<bool>(Call(...))` — C4Value raw-data
    // truthiness (C4Value.h:76,183-185). Real content returns ints
    // (Clonk.c4d/Script.c:195-203 `return(1)` / `return(0)`); C++ never
    // rejects them.
    let script = r#"
global func Initialize(state, random) { return 0; }
func ControlDig() { SetAction("Dig"); return 1; }
func ControlThrow() { return 0; }
"#;
    let (mut engine, object_id) = control_actor_fixture(script)?;
    let handled = engine.handle_control_command(1, ControlCommand::Dig, CommandKind::Press)?;
    unit_assert!(handled, "return(1) is truthy like C++'s bool cast");
    let snapshot = engine.snapshot();
    assert_values! {
        snapshot.object(object_id).expect("object present").action.name => "Dig";
    }

    let handled = engine.handle_control_command(1, ControlCommand::Throw, CommandKind::Press)?;
    unit_assert!(!handled, "return(0) is falsy like C++'s bool cast");
    Ok(())
}

#[test]
fn control_dispatch_forwards_to_effects_via_effect_call_like_clnk() -> Result<(), EngineError> {
    // The verbatim CLNK Control2Effect chain (Clonk.c4d/Script.c:
    // 195-203, 860-875): ControlDig walks *Control* effects and feeds
    // each GetEffect number into EffectCall (FnEffectCall,
    // C4Script.cpp:5589-5601), which runs Fx<Name><CallFn> on the
    // effect's COMMAND TARGET (C4Effect::DoCall, C4Effect.cpp:439-456)
    // with (pTarget, iNumber, ...) arguments. TRPR/COWB hit exactly
    // this path in GoldRush.
    let clonk_script = r#"
#strict
protected func ControlDig()
{
  if (Control2Effect("ControlDig")) return(1);
  return(0);
}
private func Control2Effect(string szControl)
{
  var i = GetEffectCount(0, this()), iEffect;
  var res;
  while (i--)
  {
    iEffect = GetEffect("*Control*", this(), i);
    if ( GetEffect(0, this(), iEffect, 1) )
      res += EffectCall(this(), iEffect, szControl);
  }
  return(res);
}
"#;
    let gun_script = r#"
#strict
public func Arm()
{
  AddEffect("GunControl", FindObject(CLNK), 100, 0, this());
  return(1);
}
public func FxGunControlControlDig(pTarget, iNumber)
{
  // this() is the command target (the gun): mark it and echo the args.
  Enter(FindObject(BOXX));
  SetR(9);
  EffectVar(0, pTarget, iNumber) = 7;
  return(1);
}
"#;
    let mut clonk = procedure_definition("CLNK", "Clonk", clonk_script, "Idle", "walk");
    clonk.set_movement_profile(MovementProfile::default());
    let gun = test_definition("GUNX", "Gun", gun_script);

    let mut engine = Engine::new();
    engine.register_definition(clonk)?;
    engine.register_definition(gun)?;
    engine.register_definition(simple_definition("BOXX"))?;
    engine.register_player(PlayerConfig::new(1, "Test"))?;

    let clonk_id = spawn_test!(engine, "CLNK", with_owner: 1, with_crew_member: true, with_action: ActionState::new("Idle"));
    let gun_id = engine.spawn_test_object(SpawnConfig::new("GUNX").with_owner(1));
    let box_id = engine.spawn_test_object(SpawnConfig::new("BOXX"));
    engine.set_crew_cursor(1, Some(clonk_id))?;

    let armed = engine.execute_context_menu(gun_id, "Arm")?;
    unit_assert!(armed, "the gun installed its control effect");
    let snapshot = engine.snapshot();
    let clonk_effects = &snapshot.object(clonk_id).test_value().effects;
    unit_assert_eq!(clonk_effects.len() => 1, "GunControl effect attached");

    let handled = engine.handle_control_command(1, ControlCommand::Dig, CommandKind::Press)?;
    unit_assert!(handled, "EffectCall's 1 propagates through Control2Effect");

    let snapshot = engine.snapshot();
    assert_values! {
        snapshot.object(gun_id).expect("gun present").rotation => 9,
            "Fx callback ran with the command target as context";
        snapshot.object(gun_id).expect("gun present").container => Some(box_id),
            "omitted-subject Enter uses the effect command target's this()";
        snapshot.object(clonk_id).expect("clonk present").container => None,
            "the affected effect carrier is not FnEnter's cthr->Obj";
    }
    let clonk_effects = &snapshot.object(clonk_id).test_value().effects;
    assert_values! {
        clonk_effects[0].vars.first() => Some(&clonk_engine::effect::EffectVarValue::Int(7)),
            "the Fx callback received (pTarget, iNumber) and wrote the effect var";
    }
    Ok(())
}

#[test]
fn contained_clonk_routes_dig_to_the_container_like_cpp() -> Result<(), EngineError> {
    // C4Object::DirectCom (C4Object.cpp:3363-3367): a contained clonk
    // hands every non-Special com to the container -
    // `Contained->Controller = Controller; ContainedControl(byCom);
    // return;` - which runs the container's Contained<Com> script with
    // the clonk as parameter (sf->Exec(Contained, {C4VObj(this)}),
    // C4Object.cpp:3221,3230). The clonk's own Control<Com> is NOT
    // consulted. Specials bypass containment (:3364).
    let clonk_script = r#"
global func Initialize(state, random) { return 0; }
func ControlDig() { SetR(3); return 1; }
func ControlSpecial() { SetR(4); return 1; }
"#;
    let hut_script = r#"
global func Initialize(state, random) { return 0; }
func ContainedDig(pClonk) { SetR(5); return 1; }
"#;
    let mut clonk = procedure_definition("CLNK", "Clonk", clonk_script, "Idle", "walk");
    clonk.set_movement_profile(MovementProfile::default());
    let hut = test_definition("HUTX", "Hut", hut_script);

    let mut engine = Engine::new();
    engine.register_definition(clonk)?;
    engine.register_definition(hut)?;
    engine.register_player(PlayerConfig::new(1, "Test"))?;

    let hut_id = engine.spawn_test_object(SpawnConfig::new("HUTX"));
    let clonk_id = spawn_test!(engine, "CLNK", with_owner: 1, with_crew_member: true, with_action: ActionState::new("Idle"), with_container: hut_id);
    engine.set_crew_cursor(1, Some(clonk_id))?;

    let handled = engine.handle_control_command(1, ControlCommand::Dig, CommandKind::Press)?;
    unit_assert!(handled, "the container consumed the com (DirectCom returns)");
    let snapshot = engine.snapshot();
    assert_values! {
        snapshot.object(hut_id).expect("hut present").rotation => 5,
            "ContainedDig ran on the container";
    }
    unit_assert_ne!(snapshot.object(clonk_id).expect("clonk present").rotation => 3, "the clonk's ControlDig was bypassed");

    // Specials skip containment: the clonk's own override runs.
    engine.handle_control_command(1, ControlCommand::Special, CommandKind::Press)?;
    let snapshot = engine.snapshot();
    assert_values! {
        snapshot.object(clonk_id).expect("clonk present").rotation => 4,
            "ControlSpecial ran on the clonk despite containment";
    }
    Ok(())
}

#[test]
fn context_menu_callback_coerces_int_returns_like_cpp_bool_cast() -> Result<(), EngineError> {
    // C4Object::MenuCommand (C4Object.cpp:3732-3736): the executed menu
    // function's result goes through `static_cast<bool>(DirectExec(...))`
    // — raw truthiness. Context functions in real content return ints
    // (Waterskin.c4d/Script.c:110 `return(1)`).
    let script = r#"
global func Initialize(state, random) { return 0; }
func EmptyContainer() { SetR(7); return 1; }
"#;
    let definition = action_definition_fixture!(
        "WSKI",
        "Waterskin",
        script,
        Some("Idle");
        "Idle" => ActionSpec::default(),
    );

    let mut engine = Engine::with_seed(0);
    engine.register_definition(definition)?;
    let id = engine.spawn_test_object(SpawnConfig::new("WSKI").with_owner(1));

    let handled = engine.execute_context_menu(id, "EmptyContainer")?;
    unit_assert!(handled, "return(1) is truthy like C++'s bool cast");
    let snapshot = engine.test_object_snapshot(id);
    unit_assert_eq!(snapshot.rotation => 7, "the context function ran");
    Ok(())
}

#[test]
fn player_context_menu_includes_and_executes_legacy_annotated_function() -> Result<(), EngineError>
{
    // C4ObjectMenu::AddContextFunctions enumerates annotated Context*
    // functions, evaluates their Condition with (menu crew, image id),
    // and executes the selected function with that same menu crew object
    // (C4ObjectMenu.cpp:398-399,650-682). MagiClonk::ContextMagic uses
    // exactly this path and calls SetComDir on its pByObject argument.
    let script = r#"
#strict 2
public func ContextMagic(object pByObject)
{
  [Magic|Image=MCMS|Condition=ReadyToMagic|Desc=Cast a spell.]
  if (pByObject == this()) { SetR(7); return(1); }
  SetR(8);
  return(0);
}
protected func ReadyToMagic(object pByObject, id image)
{
  return(pByObject == this() && image == MCMS);
}
"#;
    let definition = test_definition("MAGE", "Mage", script);
    let mut engine = Engine::new();
    engine.register_definition(definition)?;
    let mage = engine.spawn_test_object(SpawnConfig::new("MAGE").with_owner(1));

    unit_assert_eq!(
        engine.context_menu_entries(mage)? =>
        vec![ContextMenuEntry {
            function: "ContextMagic".to_string(),
            label: "Magic".to_string(),
            description: Some("Cast a spell.".to_string()),
        }],
        "the app-facing context list includes legacy ContextMagic"
    );

    unit_assert!(engine.execute_context_menu(mage, "ContextMagic")?, "the legacy callback's integer return uses C4Value truthiness");
    assert_values! {
        engine.object_snapshot(mage).expect("mage snapshot").rotation => 7,
            "ContextMagic receives the live menu crew object, not a state proplist";
    }
    Ok(())
}

#[test]
fn object_function_this_is_the_current_object_not_nil() -> Result<(), EngineError> {
    // `this` used to evaluate to nil (vm.rs hardcoded Expr::This => Nil), so a
    // script that branches on `this` took the wrong path. Here SetAction is
    // gated on `this` being truthy: before the fix `this` was nil (falsy) and
    // the action stayed "Idle"; now `this` is the object reference so the
    // action becomes "Dig".
    let script = r#"
global func Initialize(state, random) { return 0; }
func ControlDig() { if (this) { SetAction("Dig"); } return true; }
"#;
    let (mut engine, object_id) = control_actor_fixture(script)?;
    let handled = engine.handle_control_command(1, ControlCommand::Dig, CommandKind::Press)?;
    unit_assert!(handled, "control command should report handled");

    let snapshot = engine.snapshot();
    let object = snapshot.object(object_id).test_value();
    unit_assert_eq!(object.action.name => "Dig", "`this` should be truthy (the current object), so the gated SetAction runs");
    Ok(())
}

#[test]
fn dig_procedure_carves_diggable_material() {
    let definition = dig_free_definition("Digger", 6);
    // C4D_StaticBack objects skip ExecMovement (C4Movement.cpp:553-567),
    // and DFA_DIG requires a bottom attachment (C4Object.cpp:4906-4911).
    let (materials, earth) =
        action_materials_with_id(EARTH_80_FRICTION_DIG_FREE_MATERIAL_SOURCE, "Earth");

    let mut engine = definition_engine(7, definition);
    engine.set_materials(materials);
    engine.set_landscape(Landscape::flat_with_material(32, 6, Some(earth)));

    let id = spawn_test!(engine, "Digger", with_position: Vector2::new(12, 4), with_action: ActionState::new("Dig"));

    let mut snapshot = engine.test_tick();
    for _ in 0..5 {
        snapshot = engine.test_tick();
    }

    let landscape = snapshot.landscape.as_ref().test_value();
    let center_height = landscape.surface()[12];
    let edge_height = landscape.surface()[2];
    unit_assert!(center_height > 6);
    unit_assert_eq!(edge_height => 6);

    let object = snapshot.object(id).test_value();
    unit_assert_eq!(object.action.name => "Dig");
}

#[test]
fn dig_procedure_stops_before_moving_without_bottom_attachment() {
    // DFA_DIG first calls Shape.Attach(..., CNAT_Bottom); failure runs
    // ObjectComStopDig and returns before assigning dig velocity
    // (src/C4Object.cpp:4906-4911; src/C4ObjectCom.cpp:776-784).
    let mut definition = action_definition_fixture!(
        "Digger",
        "Digger",
        PROCEDURE_MOVEMENT_SCRIPT,
        Some("Walk");
        "Walk" => ActionSpec::default().with_procedure("walk"),
        "Dig" => ActionSpec::default().with_procedure("dig").with_dig_free(6),
    );
    definition.set_category(CATEGORY_OBJECT);
    definition.set_shape_rect(Some(DefinitionRect::new(-1, -1, 2, 2)));
    definition.set_shape_vertices(vec![ObjectVertex::new(0, 1).with_cnat(CNAT_BOTTOM)]);
    definition.set_contact_density(50);
    definition.set_physical(physical! {
            dig: C4_MAX_PHYSICAL
    });

    let (materials, earth) = action_materials_with_id(EARTH_80_DIG_FREE_MATERIAL_SOURCE, "Earth");
    let mut engine = definition_engine(0, definition);
    engine.set_materials(materials);
    engine.set_landscape(Landscape::flat_with_material(32, 24, Some(earth)));
    engine.set_physics(PhysicsSettings::new(0, 20, -20));
    let start = Vector2::new(12, 4);
    let id = spawn_test!(engine, "Digger", with_position: start, with_action: ActionState::new("Dig"), with_command_direction: CommandDirection::UpLeft, with_mobile: true);
    let initial_position = engine.test_object_snapshot(id).position;

    let object = tick_test_object(&mut engine, id);
    unit_assert_eq!(object.action.name => "Walk");
    unit_assert_eq!(object.position => initial_position);
    unit_assert_eq!(object.velocity => Vector2::ZERO);
}

#[test]
fn dig_free_uses_post_steering_predicted_center_on_pixel_grid_like_cpp() {
    // DFA_DIG assigns xdir during ExecAction (C4Object.cpp:4906-4935),
    // then DoMovement digs at fixtoi(fix_x+xdir), fixtoi(fix_y+ydir)
    // on the authoritative landscape plane (C4Movement.cpp:227-245).
    let mut definition = test_definition("DGRR", "Digger", PROCEDURE_MOVEMENT_SCRIPT);
    definition.set_physical(physical! {
            dig: C4_MAX_PHYSICAL
    });
    definition.set_shape_rect(Some(DefinitionRect::new(-1, -1, 2, 2)));
    definition.set_shape_vertices(vec![ObjectVertex::new(0, 1).with_cnat(CNAT_BOTTOM)]);
    definition.set_contact_density(50);
    set_actions!(
        &mut definition, Some("Dig");
        "Dig" => action_spec!(default, with_procedure: "DIG", with_length: 16, with_delay: 15, with_next: "Dig", with_dig_free: 2),
    );

    let materials = action_materials(EARTH_GRANITE_DIG_MATERIAL_SOURCE);

    let mut bytes = vec![0_u8; 32 * 32];
    // With xdir=1.25, C++ predicts center x=11. Radius two's conditional
    // right edge reaches x=13; the obsolete pre-steering center reaches
    // x=7 on its left edge instead. Keep both pixels as sentinels.
    bytes[9 * 32 + 7] = 1;
    bytes[9 * 32 + 13] = 1;
    // A non-diggable support pixel keeps this a valid bottom attachment
    // for the C++ DFA_DIG precondition.
    bytes[12 * 32 + 10] = 2;
    let grid = landscape::PixelGrid::new(
        32,
        32,
        bytes,
        vec![0, 80, 100],
        vec![None, Some("Earth".into()), Some("Granite".into())],
        vec![None; 3],
    );
    let mut landscape = Landscape::new(32, vec![30; 32]).test_value();
    landscape.set_world_height(32);
    landscape.set_pixel_grid(grid);

    let mut engine = Engine::with_seed(0);
    engine.set_materials(materials);
    engine.set_landscape(landscape);
    engine.set_physics(PhysicsSettings::new(0, 20, -20));
    engine.register_test_definition(definition);
    spawn_test!(engine, "DGRR", with_category: CATEGORY_OBJECT, with_position: Vector2::new(10, 10), with_fixed_position: FixedVec2::from_ints(10, 10), with_action: ActionState::new("Dig"), with_command_direction: CommandDirection::Right, with_mobile: true, with_loaded: true);

    engine.tick_without_snapshot().test_value();
    let landscape = engine.landscape().test_value();
    assert_values! {
        landscape.grid_byte_at(13, 9) => Some(0), "the post-steering predicted circle clears its leading edge";
        landscape.grid_byte_at(7, 9) => Some(1), "the obsolete pre-steering circle must not clear its trailing sentinel";
        landscape.grid_byte_at(10, 12) => Some(2), "non-DigFree support remains solid";
    }
}

#[test]
fn dig_procedure_removes_surface_pixel_when_circle_touches_ground() -> Result<(), EngineError> {
    let definition = dig_free_definition("DGRR", 6);
    let (materials, earth) =
        action_materials_with_id(EARTH_80_FRICTION_DIG_FREE_MATERIAL_SOURCE, "Earth");

    let mut engine = definition_engine(13, definition);
    engine.set_materials(materials);
    engine.set_landscape(Landscape::flat_with_material(32, 20, Some(earth)));

    let position_y = 18;
    let column_x = 12;

    spawn_test!(engine, "DGRR", with_position: Vector2::new(column_x, position_y), with_action: ActionState::new("Dig"));

    for _ in 0..12 {
        engine.tick_without_snapshot().test_value();
    }

    let snapshot = engine.snapshot();
    let landscape = snapshot.landscape.as_ref().test_value();
    let height = landscape
        .surface()
        .get(column_x as usize)
        .copied()
        .test_value();
    unit_assert!(height > 20, "expected dig to raise surface beyond 20, got {height}");
    Ok(())
}

#[test]
fn dig_procedure_spawns_dig2object_when_ratio_reached() {
    let digger = dig_free_definition("DGRR", 6);
    let gem = dig_gem_definition();

    let material_source = r#"
            [Material Earth]
            Name=Earth
            Density=80
            Friction=25
            DigFree=1
            Dig2Object=GEM_
            Dig2ObjectRatio=3
        "#;
    let (materials, earth) = action_materials_with_id(material_source, "Earth");

    let mut engine = definition_engine(11, digger);
    engine.register_test_definition(gem);
    engine.set_materials(materials);
    engine.set_landscape(Landscape::flat_with_material(32, 6, Some(earth)));

    spawn_test!(engine, "DGRR", with_position: Vector2::new(12, 4), with_action: ActionState::new("Dig"));

    let mut spawned = false;
    for _ in 0..20 {
        let snapshot = engine.test_tick();
        if snapshot
            .objects
            .iter()
            .any(|object| object.definition_id == "GEM_")
        {
            spawned = true;
            break;
        }
    }

    unit_assert!(spawned, "expected Dig2Object conversion to spawn target definition");
}

#[test]
fn dig2object_rotation_uses_one_cpp_random_draw() {
    // C4Object::DigOutMaterialCast passes Random(360) to CreateObject
    // with the digger as creator and NO_OWNER (C4Object.cpp:4017-4030).
    // The creator supplies the layer and Construction argument. This
    // seed makes gen_range reject its first raw RngCore sample, exposing
    // the extra ledger draw.
    const SEED: u32 = 28;

    let material_source = r#"
            [Material Earth]
            Name=Earth
            Density=80
            DigFree=1
            Dig2Object=GEM_
            Dig2ObjectRatio=1
        "#;
    let materials = action_materials(material_source);

    let mut digger_definition = test_definition("DGRR", "Digger", "");
    digger_definition.set_shape_rect(Some(DefinitionRect::new(-2, 2, 4, 7)));
    let mut gem_definition = test_definition("GEM_", "Gem", "#strict 2\nlocal creator_seen;\nfunc Construction(pCreator) { creator_seen = pCreator; }\n");
    gem_definition.set_rotateable(1);
    let mut engine = action_definitions_engine!(0;
        digger_definition,
        gem_definition,
        simple_definition("LAYR"),
    );
    engine.set_materials(materials);

    let mut pixels = vec![0_u8; 25];
    pixels[2 * 5 + 2] = 10;
    let mut densities = vec![0_i32; 128];
    densities[10] = 80;
    let mut material_names = vec![None; 128];
    material_names[10] = Some("Earth".to_string());
    let grid = landscape::PixelGrid::new(5, 5, pixels, densities, material_names, vec![None; 128]);
    let mut landscape = Landscape::flat(5, 5);
    landscape.set_pixel_grid(grid);
    engine.set_landscape(landscape);

    let layer = engine.spawn_test_object(SpawnConfig::new("LAYR").with_loaded(true));
    let digger = spawn_test!(engine, "DGRR", with_position: Vector2::new(2, 2), with_owner: 7, with_layer: layer, with_loaded: true);
    engine.rng = LcgRng::new(SEED);
    let before = engine.debug_rng_clone();

    engine.apply_landscape_operations(vec![LandscapeOperation::DigRect {
        origin: Vector2::new(2, 2),
        width: 1,
        height: 1,
        requested: false,
        by_object: Some(digger),
    }]);

    let expected_hold = SEED.wrapping_mul(214_013).wrapping_add(2_531_011);
    let expected_rotation = ((expected_hold >> 16) % 360) as i32;
    let snapshot = engine.snapshot();
    let spawned = snapshot
        .objects
        .iter()
        .find(|object| object.definition_id == "GEM_")
        .test_value();
    unit_assert_eq!(spawned.rotation => expected_rotation);
    unit_assert_eq!(spawned.position => Vector2::new(2, 11));
    unit_assert_eq!(spawned.owner => OWNER_NONE);
    unit_assert_eq!(spawned.controller => OWNER_NONE);
    unit_assert_eq!(spawned.layer => Some(layer));
    assert_values! {
        spawned.local_vars.get("creator_seen") => Some(&object_reference_value(digger)),
            "Dig2Object Construction receives the digger as creator";
    }
    unit_assert_eq!(snapshot.rng.hold => expected_hold);
    unit_assert_eq!(snapshot.rng.count => before.count + 1);
}

#[test]
fn legacy_dig_conversion_recomputes_creator_geometry_between_materials() {
    let materials = action_materials(EARTH_ROCK_DIG_OBJECT_MATERIAL_SOURCE);
    let mut engine = Engine::with_seed(44);
    engine.set_materials(materials);
    let mut digger = test_definition("DGR3", "Digger", "#strict 3");
    digger.set_shape_rect(Some(DefinitionRect::new(0, 0, 1, 2)));
    let first = test_definition(
        "GEMA",
        "First gem",
        r#"#strict 3
    func Construction(object creator)
    {
        SetPosition(10, 20, creator);
        SetShape(-1, 3, 4, 7, creator);
    }
    "#,
    );
    engine.register_test_definition(digger);
    engine.register_test_definition(first);
    engine.register_test_definition(simple_definition("GEMB"));

    let grid = landscape::PixelGrid::new(
        2,
        1,
        vec![1, 2],
        vec![0, 80, 100],
        vec![None, Some("Earth".to_owned()), Some("Rock".to_owned())],
        vec![None; 3],
    );
    let mut landscape = Landscape::new(2, vec![1; 2]).test_value();
    landscape.set_world_height(1);
    landscape.set_pixel_grid(grid);
    engine.set_landscape(landscape);
    let digger = engine.spawn_test_object(SpawnConfig::new("DGR3"));

    engine.apply_landscape_operations(vec![LandscapeOperation::DigRect {
        origin: Vector2::ZERO,
        width: 2,
        height: 1,
        requested: false,
        by_object: Some(digger),
    }]);

    let first = engine
        .objects
        .iter()
        .find(|object| object.definition_id == "GEMA")
        .test_value();
    let second = engine
        .objects
        .iter()
        .find(|object| object.definition_id == "GEMB")
        .test_value();
    assert_values! {
        first.state.position => Vector2::ZERO,
            "initial NewObject growth preserves the raw y=0 shape bottom";
        second.state.position => Vector2::new(10, 30),
            "legacy movement/direct dig conversion observes prior lifecycle writes";
    }
}

#[test]
fn dig_procedure_spawns_at_most_one_dig2object_per_tick() {
    let digger = dig_free_definition("DGRR", 6);
    let gem = dig_gem_definition();

    let material_source = r#"
            [Material Earth]
            Name=Earth
            Density=80
            Friction=25
            DigFree=1
            Dig2Object=GEM_
            Dig2ObjectRatio=1
        "#;
    let (materials, earth) = action_materials_with_id(material_source, "Earth");

    let mut engine = definition_engine(13, digger);
    engine.register_test_definition(gem);
    engine.set_materials(materials);
    engine.set_landscape(Landscape::flat_with_material(32, 6, Some(earth)));

    spawn_test!(engine, "DGRR", with_position: Vector2::new(12, 4), with_action: ActionState::new("Dig"));

    let mut previous_count = 0;
    let mut observed_spawn = false;
    for _ in 0..20 {
        let snapshot = engine.test_tick();
        let current_count = snapshot
            .objects
            .iter()
            .filter(|object| object.definition_id == "GEM_")
            .count();
        if current_count > previous_count {
            assert_values! {
                current_count - previous_count => 1, "expected at most one Dig2Object spawn per tick";
            }
            observed_spawn = true;
            break;
        }
        previous_count = current_count;
    }

    unit_assert!(observed_spawn, "expected Dig2Object conversion to occur within 20 ticks");
}

#[test]
fn dig2object_request_only_requires_explicit_request() {
    fn build_digger_definition() -> Definition {
        dig_free_definition("DGRR", 6)
    }

    fn build_gem_definition() -> Definition {
        dig_gem_definition()
    }

    let material_source = r#"
            [Material Earth]
            Name=Earth
            Density=80
            Friction=25
            DigFree=1
            Dig2Object=GEM_
            Dig2ObjectRatio=1
            Dig2ObjectRequest=1
        "#;
    let (materials, earth) = action_materials_with_id(material_source, "Earth");

    // Without request flag set on the action we should not spawn anything.
    {
        let mut engine = definition_engine(19, build_digger_definition());
        engine.register_test_definition(build_gem_definition());
        engine.set_materials(materials.clone());
        engine.set_landscape(Landscape::flat_with_material(32, 6, Some(earth)));

        spawn_test!(engine, "DGRR", with_position: Vector2::new(12, 4), with_action: ActionState::new("Dig"));

        for _ in 0..20 {
            let snapshot = engine.test_tick();
            unit_assert!(!snapshot.objects.iter().any(|object| object.definition_id == "GEM_"), "expected no Dig2Object spawn without request");
        }
    }

    // With request flag set, the conversion should occur.
    {
        let mut engine = definition_engine(19, build_digger_definition());
        engine.register_test_definition(build_gem_definition());
        engine.set_materials(materials);
        engine.set_landscape(Landscape::flat_with_material(32, 6, Some(earth)));

        let mut requested_action = ActionState::new("Dig");
        requested_action.data = 1;
        spawn_test!(engine, "DGRR", with_position: Vector2::new(12, 4), with_action: requested_action);

        let mut spawned = false;
        for _ in 0..20 {
            let snapshot = engine.test_tick();
            if snapshot
                .objects
                .iter()
                .any(|object| object.definition_id == "GEM_")
            {
                spawned = true;
                break;
            }
        }

        unit_assert!(spawned, "expected Dig2Object conversion to respect request flag when set");
    }
}

#[test]
fn throw_procedure_zeroes_velocity() {
    let definition = action_definition_fixture!(
        "Thrower",
        "Thrower",
        PROCEDURE_MOVEMENT_SCRIPT,
        Some("Throw");
        "Idle" => ActionSpec::default(),
        "Throw" => ActionSpec::for_procedure("throw"),
    );

    let (mut engine, id) = definition_fixture_case!(17, definition, "Thrower", with_velocity: Vector2::new(6, -3), with_action: ActionState::new("Throw"));

    let object = tick_test_object(&mut engine, id);
    unit_assert_eq!(object.velocity => Vector2::ZERO);
    unit_assert_eq!(object.action.name => "Throw");
}

#[test]
fn object_action_throw_exits_content_after_action_gate() {
    // ObjectActionThrow computes force/facing, changes action without an
    // Action.Target argument, then Exit's the item with one Random(360)
    // draw (C4ObjectCom.cpp:120-137; C4Object.cpp:1532-1563).
    let mut clonk = test_definition("CLNK", "Clonk", "#strict 2\n");
    clonk.set_shape_rect(Some(DefinitionRect::new(-8, -10, 16, 20)));
    let mut physical = PhysicalInfo::default();
    physical.throw = 50_000;
    clonk.set_physical(physical);
    set_test_actions(
        &mut clonk,
        Some("Walk"),
        procedure_actions([("Walk", "walk"), ("Throw", "throw")]),
    );
    let item = test_definition("FLAG", "Flag", "#strict 2\n");

    let mut engine = action_definitions_engine!(7; clonk, item);
    let clonk_id = spawn_test!(engine, "CLNK", with_position: Vector2::new(100, 200), with_direction: Direction::Right, with_action: ActionState::new("Walk"));
    let flag_id = engine.spawn_test_object(SpawnConfig::new("FLAG").with_container(clonk_id));
    engine
        .apply_object_update(
            clonk_id,
            ObjectUpdate::new()
                .with_action_update(ActionUpdate::default().with_target(Some(flag_id))),
        )
        .test_value();

    let mut expected_rng = engine.debug_rng_clone();
    let expected_rotation = expected_rng.random(360);
    let before = engine.test_object_snapshot(clonk_id);
    let before_index = engine.test_object_index(clonk_id);
    let shape_top = engine.objects[before_index]
        .current_shape_rect()
        .map(|rect| rect.y)
        .unwrap_or(0);
    let expected_exit = Vector2::new(before.position.x, before.position.y + shape_top - 1);
    unit_assert!(engine.try_object_action_throw(clonk_id, flag_id).expect("throw succeeds"));

    let clonk = engine.test_object_snapshot(clonk_id);
    let flag = engine.test_object_snapshot(flag_id);
    let throw_force = math::val_by_physical(400, 50_000);
    unit_assert_eq!(clonk.action.name => "Throw");
    unit_assert_eq!(clonk.action.target => Some(flag_id));
    unit_assert!(clonk.contents.is_empty());
    unit_assert_eq!(flag.container => None);
    unit_assert_eq!(flag.position => expected_exit);
    unit_assert_eq!(flag.rotation => expected_rotation);
    let flag_index = engine.test_object_index(flag_id);
    unit_assert_eq!(engine.objects[flag_index].fixed_velocity => FixedVec2::new(throw_force, -throw_force));
    unit_assert_eq!(engine.objects[flag_index].rotation_velocity => throw_force);
    unit_assert_eq!(engine.debug_rng_clone() => expected_rng);
}

#[test]
fn scale_procedure_zeroes_horizontal_velocity() {
    let mut engine = movement_procedure_engine(23, "Scaler", "Scale", "scale");

    engine.set_environment(EnvironmentSettings::new(3));

    let id = spawn_test!(engine, "Scaler", with_velocity: Vector2::new(-7, 2), with_action: ActionState::new("Scale"));

    let object = tick_test_object(&mut engine, id);
    unit_assert_eq!(object.velocity.x => 0);
    unit_assert_eq!(object.velocity.y => 1);
}

#[test]
fn scale_command_direction_moves_up_when_pressing_wall_direction() {
    let mut definition = movement_procedure_definition("Scaler", "Scale", "scale");
    definition
        .set_movement_profile(movement_profile!(with_scale_speed: 6, with_scale_acceleration: 3));

    let (mut engine, id) = definition_fixture_case!(41, definition, "Scaler", with_direction: Direction::Left, with_command_direction: CommandDirection::Left, with_action: ActionState::new("Scale"));

    let object = tick_test_object(&mut engine, id);
    unit_assert_eq!(object.velocity => Vector2::new(0, -3));
    unit_assert_eq!(object.direction => Direction::Left);
}

#[test]
fn hangle_command_direction_updates_velocity_and_direction() {
    let mut definition = movement_procedure_definition("Hangler", "Hangle", "hang");
    definition
        .set_movement_profile(movement_profile!(with_hangle_speed: 5, with_hangle_acceleration: 2));

    let (mut engine, id) = definition_fixture_case!(43, definition, "Hangler", with_direction: Direction::Right, with_command_direction: CommandDirection::Left, with_action: ActionState::new("Hangle"));

    let object = tick_test_object(&mut engine, id);
    unit_assert_eq!(object.velocity => Vector2::new(-2, 0));
    unit_assert_eq!(object.direction => Direction::Left);
}

#[test]
fn dig_command_direction_sets_directional_velocity() {
    let mut definition = movement_procedure_definition("Digger", "Dig", "dig");
    definition.set_movement_profile(MovementProfile::default().with_dig_speed(6));

    let (mut engine, id) = definition_fixture_case!(47, definition, "Digger", with_direction: Direction::Right, with_command_direction: CommandDirection::DownLeft, with_action: ActionState::new("Dig"));

    let object = tick_test_object(&mut engine, id);
    unit_assert_eq!(object.velocity => Vector2::new(-6, 6));
    unit_assert_eq!(object.direction => Direction::Left);

    engine
        .apply_object_update(
            id,
            ObjectUpdate::new().with_command_direction(CommandDirection::Up),
        )
        .test_value();

    let object = tick_test_object(&mut engine, id);
    unit_assert_eq!(object.velocity => Vector2::new(-6, -3));
    unit_assert_eq!(object.direction => Direction::Left);
}

fn construction_builder_definition(id: &str, script: &str, no_other_action: bool) -> Definition {
    let mut definition = test_definition(id, id, script);
    definition.set_category(CATEGORY_OBJECT);
    definition.set_physical(physical! {
            can_construct: 100
    });
    set_actions!(
        &mut definition, Some("Idle");
        "Idle" => ActionSpec::default(),
        "Walk" => ActionSpec::default().with_procedure("WALK"),
        "Build" => action_spec!(default, with_procedure: "BUILD", with_length: 4, with_delay: 5, with_no_other_action: no_other_action),
    );
    definition
}

fn construction_target_definition(script: &str) -> Definition {
    let mut definition = test_definition("BTGT", "Build target", script);
    definition.set_category(CATEGORY_STRUCTURE);
    definition.set_mass(100);
    definition.set_shape_rect(Some(DefinitionRect::new(-10, -20, 20, 40)));
    definition
}

fn build_actor_definition(
    id: &str,
    name: &str,
    script: &str,
    default_action: &str,
    idle_action: &str,
) -> Definition {
    action_definition(
        id,
        name,
        script,
        Some(default_action),
        procedure_actions([(idle_action, "walk"), ("Build", "build")]),
    )
}

fn component_structure_definition(
    id: &str,
    name: &str,
    script: &str,
    constructable: bool,
    mass: Option<i32>,
    component_id: &str,
    component_count: i32,
) -> Definition {
    let mut definition = test_definition(id, name, script);
    definition.set_constructable(constructable);
    definition.set_category(CATEGORY_STRUCTURE);
    if let Some(mass) = mass {
        definition.set_mass(mass);
    }
    definition.set_components(vec![component(component_id, component_count)]);
    definition
}

#[test]
fn build_contained_target_requires_live_powered_build_container() {
    // DFA_BUILD's first target guard requires a contained target's live
    // container to be building without NeedEnergy (C4Object.cpp:5016-5020).
    // It precedes the full-target check and returns without stopping.
    for (label, container_action, need_energy, inactive, construction, expected_delta) in [
        ("idle", "Idle", false, false, 50_000, 0),
        ("needs energy", "Build", true, false, 50_000, 0),
        ("powered build", "Build", false, false, 50_000, 150),
        ("inactive powered build", "Build", false, true, 50_000, 150),
        ("idle full target", "Idle", false, false, FULL_CON, 0),
    ] {
        let mut engine = action_definitions_engine!(67;
            construction_builder_definition("BLDR", "", false),
            construction_builder_definition("BCON", "", false),
            construction_target_definition(""),
        );

        let container = spawn_test!(engine, "BCON", with_action: ActionState::new(container_action), with_need_energy: need_energy, with_position: Vector2::new(100, 200));
        if inactive {
            let container_idx = engine.test_object_index(container);
            engine.objects[container_idx].state.status = ObjectStatus::Inactive;
        }
        let target = spawn_test!(engine, "BTGT", with_construction: construction, with_position: Vector2::new(100, 200), with_container: container);
        let target_idx = engine.test_object_index(target);
        let target_position = engine.objects[target_idx].state.position;
        let shape = engine.objects[target_idx].current_shape_rect().test_value();
        let mut build = targeted_action("Build", target);
        build.phase = 2;
        build.ticks = 3;
        build.time = 41;
        let sentinel_velocity = FixedVec2::new(fixed100(125), fixed100(-75));
        let builder = spawn_test!(engine, "BLDR", with_position: Vector2::new(
                    target_position.x + shape.x,
                    target_position.y + shape.y,
                ), with_fixed_velocity: sentinel_velocity, with_command_direction: CommandDirection::Right, with_action: build);
        let builder_idx = engine.test_object_index(builder);

        let returned_early = engine.apply_physics_at_index(builder_idx).test_value();
        let target_state = engine.test_object_snapshot(target);
        let builder_state = test_object(&engine, builder);
        assert_values! {
            target_state.construction => construction.saturating_add(expected_delta).min(FULL_CON), "{label}";
            builder_state.state.action.name => "Build", "{label}";
            builder_state.state.action.time => 42, "{label}";
            builder_state.state.action.phase => 2, "{label}";
            builder_state.state.action.ticks => 3, "{label}";
        }
        if expected_delta == 0 {
            unit_assert!(returned_early, "{label} must return from ExecAction");
            unit_assert_eq!(builder_state.state.command_direction => CommandDirection::Right);
            unit_assert_eq!(builder_state.fixed_velocity => sentinel_velocity, "{label}");
            unit_assert_eq!(builder_state.frame_t_attach => CNAT_NONE, "{label}");
            unit_assert_eq!(builder_state.state.t_attach => CNAT_NONE, "{label}");
        } else {
            unit_assert!(!returned_early, "{label} must reach the phase tail");
            unit_assert_eq!(builder_state.fixed_velocity => FixedVec2::ZERO);
            unit_assert_eq!(builder_state.frame_t_attach => CNAT_BOTTOM, "{label}");
            unit_assert_eq!(builder_state.state.t_attach => CNAT_BOTTOM, "{label}");
        }
    }
}

#[test]
fn build_area_uses_live_shape_and_inclusive_vertical_margin() {
    // DFA_BUILD compares the builder against the target's live Shape and
    // uses inclusive Inside bounds, including Wdt and Hgt+16
    // (C4Object.cpp:5027-5032).
    for (label, position_case, should_build, no_other_action, inactive) in [
        ("inclusive bottom-right", 0, true, false, false),
        ("inactive inclusive bottom-right", 0, true, false, true),
        ("one past right", 1, false, false, false),
        ("one past bottom margin", 2, false, false, false),
        ("locked one past right", 1, false, true, false),
    ] {
        let mut engine = action_definitions_engine!(67;
            construction_builder_definition("BLDR", "", no_other_action),
            construction_target_definition(""),
        );
        let target = spawn_test!(engine, "BTGT", with_construction: 50_000, with_position: Vector2::new(100, 200));
        let target_idx = engine.test_object_index(target);
        engine.objects[target_idx].state.damage = 37;
        if inactive {
            engine.objects[target_idx].state.status = ObjectStatus::Inactive;
        }
        let target_position = engine.objects[target_idx].state.position;
        let shape = engine.objects[target_idx].current_shape_rect().test_value();
        let origin = Vector2::new(target_position.x + shape.x, target_position.y + shape.y);
        let builder_position = match position_case {
            0 => Vector2::new(origin.x + shape.width, origin.y + shape.height + 16),
            1 => Vector2::new(origin.x + shape.width + 1, origin.y),
            _ => Vector2::new(origin.x, origin.y + shape.height + 17),
        };
        let build = targeted_action("Build", target);
        let sentinel_velocity = FixedVec2::new(fixed100(200), fixed100(-100));
        let builder = spawn_test!(engine, "BLDR", with_position: builder_position, with_fixed_velocity: sentinel_velocity, with_command_direction: CommandDirection::Right, with_action: build);
        let builder_idx = engine.test_object_index(builder);

        let returned_early = engine.apply_physics_at_index(builder_idx).test_value();
        let target_state = engine.test_object_snapshot(target);
        let builder_state = test_object(&engine, builder);
        if should_build {
            unit_assert!(!returned_early, "{label}");
            unit_assert_eq!(target_state.construction => 51_500, "{label}");
            unit_assert_eq!(builder_state.state.action.name => "Build", "{label}");
            unit_assert_eq!(builder_state.fixed_velocity => FixedVec2::ZERO, "{label}");
        } else {
            unit_assert!(returned_early, "{label}");
            unit_assert_eq!(target_state.construction => 50_000, "{label}");
            unit_assert_eq!(target_state.damage => 37, "{label}");
            unit_assert_eq!(builder_state.state.command_direction => CommandDirection::Stop);
            if no_other_action {
                unit_assert_eq!(builder_state.state.action.name => "Build", "{label}");
                unit_assert_eq!(builder_state.fixed_velocity => sentinel_velocity, "{label}");
            } else {
                unit_assert_eq!(builder_state.state.action.name => "Walk", "{label}");
                unit_assert_eq!(builder_state.fixed_velocity => FixedVec2::ZERO, "{label}");
            }
            unit_assert_eq!(builder_state.frame_t_attach => CNAT_NONE, "{label}");
        }
    }

    // The range guard also precedes the completed-target branch. A full
    // internal target outside the live shape stops the builder but must
    // not receive SetCommand(Exit).
    let mut engine = action_definitions_engine!(68;
        construction_builder_definition("BLDR", "", false),
        construction_target_definition(""),
    );
    let builder = spawn_test!(engine, "BLDR", with_position: Vector2::new(100, 200), with_action: ActionState::new("Build"));
    let target = spawn_test!(engine, "BTGT", with_position: Vector2::new(100, 200), with_construction: FULL_CON, with_container: builder);
    let target_idx = engine.test_object_index(target);
    let target_position = engine.objects[target_idx].state.position;
    let shape = engine.objects[target_idx].current_shape_rect().test_value();
    engine.objects[target_idx].state.no_collect_delay = 2;
    engine.objects[target_idx]
        .commands
        .push_back(CommandRequest::new(CommandId::Wait).with_update_interval(90))
        .test_value();
    let builder_idx = engine.test_object_index(builder);
    engine.objects[builder_idx].state.action.target = Some(target);
    engine
        .apply_object_update(
            builder,
            ObjectUpdate::new().with_position(Vector2::new(
                target_position.x + shape.x + shape.width + 1,
                target_position.y + shape.y,
            )),
        )
        .test_value();
    let builder_idx = engine.test_object_index(builder);
    unit_assert!(engine.apply_physics_at_index(builder_idx).expect("out-of-range full Build executes"));
    let target_idx = engine.test_object_index(target);
    unit_assert_eq!(engine.objects[target_idx].state.no_collect_delay => 2);
    assert_values! {
        engine.objects[target_idx].commands.command_names() => vec!["Wait".to_string()],
            "area failure must run before completed-target Exit";
    }
}

#[test]
fn build_completed_internal_target_replaces_stack_with_base_exit() {
    // Target::Build returns true on the FullCon crossing. The next BUILD
    // tick stops, then calls plain SetCommand(Exit) on an internal target
    // (C4Object.cpp:5033-5043; SetCommand :3937-3985).
    let target_script = r#"#strict
local own_control_calls;
protected func ControlCommand() { own_control_calls++; return 1; }
"#;
    let mut engine = action_definitions_engine!(67;
        construction_builder_definition("BLDR", "", false),
        construction_target_definition(target_script),
    );
    let builder = spawn_test!(engine, "BLDR", with_position: Vector2::new(100, 200), with_action: ActionState::new("Build"));
    let target = spawn_test!(engine, "BTGT", with_position: Vector2::new(100, 200), with_construction: FULL_CON - 1, with_controller: 7, with_container: builder);
    let builder_idx = engine.test_object_index(builder);
    engine.objects[builder_idx].state.action.target = Some(target);
    let target_idx = engine.test_object_index(target);
    engine.objects[target_idx].state.controller = 7;
    engine.objects[target_idx].state.no_collect_delay = 2;
    engine.objects[target_idx]
        .commands
        .push_back(CommandRequest::new(CommandId::Wait).with_update_interval(90))
        .test_value();
    engine.objects[target_idx]
        .commands
        .push_back(CommandRequest::new(CommandId::MoveTo).with_tx(Some(10)))
        .test_value();

    unit_assert!(!engine.apply_physics_at_index(builder_idx).expect("crossing Build executes"));
    let target_idx = engine.test_object_index(target);
    assert_values! {
        engine.objects[target_idx].state.construction => FULL_CON;
        engine.objects[target_idx].commands.command_names() => vec!["Wait".to_string(), "MoveTo".to_string()],
            "the successful FullCon crossing does not issue Exit yet";
        engine.objects[target_idx].state.no_collect_delay => 2;
    }

    let builder_idx = engine.test_object_index(builder);
    unit_assert!(engine.apply_physics_at_index(builder_idx).expect("completed Build executes"));
    let builder_state = engine.test_object_snapshot(builder);
    let target_idx = engine.test_object_index(target);
    assert_values! {
        builder_state.action.name => "Walk";
        builder_state.command_direction => CommandDirection::Stop;
        engine.objects[target_idx].state.container => Some(builder);
        engine.objects[target_idx].state.no_collect_delay => 1;
        engine.objects[target_idx].commands.command_names() => vec!["Exit".to_string()],
            "SetCommand replaces the whole old stack";
    }
    let stack = serde_json::to_value(engine.objects[target_idx].commands.snapshot()).test_value();
    unit_assert_eq!(stack["commands"][0]["mode"] => serde_json::json!("Base"));
    unit_assert!(!engine.objects[target_idx].state.local_vars.contains_key("own_control_calls"), "plain SetCommand skips the target's own ControlCommand");
}

#[test]
fn build_completed_internal_target_honors_inside_vehicle_control() {
    let builder_script = r#"#strict
local control_calls, control_command, control_by, control_action;
protected func ControlCommand(command, target, tx, ty, target2, data, by)
{
    control_calls++;
    control_command = command;
    control_by = by;
    control_action = GetAction();
    return 1;
}
"#;
    let target_script = r#"#strict
local own_control_calls;
protected func ControlCommand() { own_control_calls++; return 1; }
"#;
    let mut builder_definition = construction_builder_definition("BLDR", builder_script, false);
    builder_definition.set_vehicle_control(VEHICLE_CONTROL_INSIDE);
    let mut engine = action_definitions_engine!(67;
        builder_definition,
        construction_target_definition(target_script),
    );
    let builder = spawn_test!(engine, "BLDR", with_position: Vector2::new(100, 200), with_controller: 1, with_action: ActionState::new("Build"));
    let target = spawn_test!(engine, "BTGT", with_position: Vector2::new(100, 200), with_construction: FULL_CON, with_controller: 7, with_container: builder);
    let builder_idx = engine.test_object_index(builder);
    engine.objects[builder_idx].state.action.target = Some(target);
    let target_idx = engine.test_object_index(target);
    // C4Object::Enter normally inherits a nonliving target's controller
    // from its container; seed a distinct saved/live value so the
    // SetCommand vehicle-overload transfer is observable.
    engine.objects[target_idx].state.controller = 7;
    engine.objects[target_idx].state.no_collect_delay = 2;
    engine.objects[target_idx]
        .commands
        .push_back(CommandRequest::new(CommandId::Wait).with_update_interval(90))
        .test_value();

    unit_assert!(engine.apply_physics_at_index(builder_idx).expect("completed Build executes"));

    let target_idx = engine.test_object_index(target);
    unit_assert_eq!(engine.objects[target_idx].state.no_collect_delay => 1);
    unit_assert!(engine.objects[target_idx].commands.command_names().is_empty(), "truthy inside control consumes Exit after clearing the old stack");
    unit_assert!(!engine.objects[target_idx].state.local_vars.contains_key("own_control_calls"), "plain SetCommand skips the target's own ControlCommand");
    let builder_idx = engine.test_object_index(builder);
    let builder_state = &engine.objects[builder_idx].state;
    unit_assert_eq!(builder_state.action.name => "Walk");
    unit_assert_eq!(builder_state.controller => 7);
    assert_snapshot_locals!(builder_state;
        "control_calls" => Some(&Value::Int(1));
        "control_command" => Some(&Value::String("Exit".to_string().into()));
        "control_by" => Some(&compat::object_reference_value(target));
        "control_action" => Some(&Value::String("Walk".to_string().into())),
            "ObjectComStop precedes target SetCommand";
    );
}

#[test]
fn build_procedure_requires_components_before_progress() -> Result<(), EngineError> {
    let script = NOOP_DEFINITION_SCRIPT;

    let mut builder_definition =
        build_actor_definition("Builder", "Builder", script, "Walk", "Walk");
    builder_definition.set_category(DEFAULT_CATEGORY);
    builder_definition.set_crew_member(true);
    builder_definition.set_mass(50);

    let structure_definition = component_structure_definition(
        "Structure",
        "Structure",
        script,
        true,
        Some(100),
        "Wood",
        1,
    );

    let mut material_definition = test_definition("Wood", "Wood", script);
    material_definition.set_mass(20);

    let mut engine = Engine::with_seed(7);
    engine.register_definition(builder_definition)?;
    engine.register_definition(structure_definition)?;
    engine.register_definition(material_definition)?;
    engine.set_construction_needs_material(true);

    // CreateConstruction sites enter the world at one percent; a
    // zero-construction NewObject is removed before return
    // (C4Game.cpp:1110-1129; C4Object.cpp:1513-1517).
    let structure_id =
        engine.spawn_test_object(SpawnConfig::new("Structure").with_construction(1_000));

    let build_state = targeted_action("Build", structure_id);
    let builder_id = spawn_test!(engine, "Builder", with_action: build_state, with_alive: true, with_crew_member: true, with_controller: 4, with_command_direction: CommandDirection::Right);

    let before = engine.test_object_snapshot(structure_id).construction;
    let snapshot = engine.tick()?;
    let after = snapshot.object(structure_id).test_value().construction;
    assert_values! {
        before => 1_000;
        after => 1_000, "construction should not progress without components";
        snapshot.object(builder_id).and_then(|builder| builder.action_procedure.as_deref()) => Some("walk"),
            "a material refusal must stop DFA_BUILD like ObjectComStop";
        snapshot.object(builder_id).map(|builder| builder.command_stack.command_names()) => Some(vec!["Acquire".to_owned()]),
            "C4Object::Build must queue Acquire for the first missing component";
        snapshot.object(builder_id).and_then(|builder| builder.command_stack.command_views().first().cloned()).map(|command| command.data) => Some(CommandData::Text("Wood".to_owned())),
            "Acquire must request the exact missing component (C4Object.cpp:1725-1747)";
        snapshot.hud.messages.len() => 1;
        snapshot.hud.messages[0].kind => MessageKind::Target;
        snapshot.hud.messages[0].target => Some(builder_id);
        snapshot.hud.messages[0].player => Some(4);
        snapshot.hud.messages[0].lines => vec!["Structure", "needs", "1x Wood"];
    }
    Ok(())
}

#[test]
fn automatic_construction_returns_collected_material_through_a_climbable_u_route() {
    struct FrontierDefinitionResolver {
        roots: Vec<std::path::PathBuf>,
    }

    impl clonk_engine::scenario::LegacyDefinitionResolver for FrontierDefinitionResolver {
        fn resolve_definition_groups(
            &self,
            _scenario: &clonk_resources::Group,
            identifier: &str,
        ) -> Result<Vec<clonk_resources::Group>, ScenarioError> {
            let relative = identifier.replace('\\', "/");
            self.roots
                .iter()
                .map(|root| root.join(&relative))
                .find(|candidate| candidate.exists())
                .map(clonk_resources::Group::open)
                .transpose()
                .map_err(ScenarioError::Resources)?
                .map(|group| vec![group])
                .ok_or(ScenarioError::LegacyDefinitionNotFound { path: relative })
        }
    }

    // Frontier exposes the point-path/traversal mismatch behind automatic
    // construction with unmodified CLNK, CST1 and ROCK definitions. The
    // pathfinder callback accepts point-clear rays
    // (C4Game.cpp:2288-2292,2671; C4PathFinder.cpp:545-550), but a WALK
    // command only steers horizontally and its high-angle jump is limited
    // to 40 pixels (C4Command.cpp:319-327,1874-1893). This
    // deliberately corrected route must therefore prove actor traversal,
    // not merely time out or postpone the same reachable material trip.
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let content = repository.join("content");
    let scenario_path = content.join("Missions.c4f/Frontier.c4s");
    let scenario = Scenario::load_from_path_with_seed(
        &scenario_path,
        &FrontierDefinitionResolver {
            roots: vec![repository.clone(), content.clone()],
        },
        0,
    )
    .test_value();
    let material_library = clonk_resources::MaterialLibrary::from_group(
        &clonk_resources::Group::open(content.join("Material.c4g")).test_value(),
    )
    .test_value();
    let system_scripts = clonk_engine::scenario::load_system_scripts(
        &clonk_resources::Group::open(repository.join("planet/System.c4g")).test_value(),
    )
    .test_value();

    let mut engine = Engine::with_seed(0);
    engine.configure_materials_from_library(&material_library);
    engine.install_global_scripts(&system_scripts);
    scenario.apply(&mut engine).test_value();
    let owner = engine
        .join_player(test_join_config("Route probe", 0, Vec::new()))
        .test_value()
        .number();
    let clonk = engine.crew_cursor(owner).test_value();

    let width = 320usize;
    let height = 220usize;
    let mut pixels = vec![0u8; width * height];
    for y in 100..height {
        for x in 0..width {
            let shaft = (40..=80).contains(&x) && y <= 175;
            let lower_tunnel = (40..=250).contains(&x) && (145..=175).contains(&y);
            if !shaft && !lower_tunnel {
                pixels[y * width + x] = 1;
            }
        }
    }
    let grid = clonk_engine::landscape::PixelGrid::new(
        width as u32,
        height as u32,
        pixels,
        vec![0, 80],
        vec![None, Some("Earth".to_string())],
        vec![None; 2],
    );
    let mut landscape = Landscape::new(width as u32, vec![100; width]).test_value();
    landscape.set_world_height(height as i32);
    landscape.set_pixel_grid(grid);
    engine.set_landscape(landscape);

    engine
        .apply_object_update(
            clonk,
            ObjectUpdate::new().with_position(Vector2::new(220, 90)),
        )
        .test_value();
    let initial_construction = 90_000;
    let site = spawn_test!(engine, "CST1", with_position: Vector2::new(220, 80), with_construction: initial_construction, with_owner: owner);
    let material =
        engine.spawn_test_object(SpawnConfig::new("ROCK").with_position(Vector2::new(180, 170)));
    engine.spawn_test_object(SpawnConfig::new("WOOD").with_container(clonk));
    engine
        .execute_player_command(
            owner,
            CommandId::Build as i32,
            0,
            0,
            site.as_u64() as i32,
            0,
            0,
            1,
        )
        .test_value();

    let mut collected_frame = None;
    let mut completed = None;
    let mut last_clonk = None;
    for route_frame in 0..800 {
        let snapshot = engine.test_tick();
        let clonk_state = snapshot.object(clonk).test_value();
        if collected_frame.is_none()
            && snapshot
                .object(material)
                .is_some_and(|rock| rock.container == Some(clonk))
        {
            collected_frame = Some(route_frame);
        }
        if snapshot
            .object(site)
            .is_some_and(|site| site.construction > initial_construction)
        {
            completed = Some((
                route_frame,
                clonk_state.clone(),
                snapshot.object(material).is_some(),
            ));
            break;
        }
        last_clonk = Some(clonk_state.clone());
    }

    let collected_frame = collected_frame.test_value();
    unit_assert!(collected_frame <= 350, "outbound ROCK collection must remain bounded, got frame {collected_frame}");
    let (completed_frame, clonk_state, material_survived) = completed.unwrap_or_else(|| {
        let clonk_state = last_clonk.test_value();
        panic!(
            "carried ROCK never returned to CST1; CLNK ended at {:?} with {:?}",
            clonk_state.position,
            clonk_state.command_stack.command_names()
        )
    });
    unit_assert!(completed_frame <= 700, "construction resumed too late");
    unit_assert!(!material_survived, "the collected ROCK must be consumed");
    unit_assert!(clonk_state.position.x >= 180 && clonk_state.position.y < 120, "construction must resume at the upper site, got {:?}", clonk_state.position);
}

#[test]
fn failed_build_command_does_not_duplicate_live_needed_material_message() -> Result<(), EngineError>
{
    // C4Object::Build first creates the target message. If its retained
    // Build command later fails, C4Command::Fail uses Append with
    // fNoDuplicates=true, so C4GameMessage::Append keeps the existing
    // identical text instead of drawing it twice (C4Object.cpp:1733-1747;
    // C4Command.cpp:2185-2194,2229-2235; C4GameMessage.cpp:73-83,315-328).
    let mut builder = build_actor_definition("BLDR", "Builder", "#strict", "Walk", "Walk");
    builder.set_crew_member(true);
    builder.set_physical(physical! {
            can_construct: 1
    });
    let site = component_structure_definition("SITE", "Site", "#strict", true, None, "WOOD", 1);

    let mut engine = Engine::with_seed(71);
    engine.register_definition(builder)?;
    engine.register_definition(site)?;
    engine.register_test_script_definition("WOOD", "Wood", "#strict");
    engine.set_construction_needs_material(true);

    let site_id = spawn!(engine, "SITE", with_construction: 1_000, with_ordered_components: vec![("WOOD".to_owned(), 0)])?;
    let action = targeted_action("Build", site_id);
    let builder_id = spawn!(engine, "BLDR", with_action: action, with_alive: true, with_crew_member: true, with_controller: 4)?;
    let builder_index = engine.test_object_index(builder_id);
    engine.objects[builder_index]
        .commands
        .push_front(
            CommandRequest::new(CommandId::Build)
                .with_target(Some(site_id))
                .with_mode(CommandMode::Base),
        )
        .test_value();

    let first = engine.tick()?;
    let first_message_id = first.hud.messages[0].id;
    assert_values! {
        first.hud.messages.len() => 1;
        first.hud.messages[0].lines => vec!["Site", "needs", "1x Wood"];
        first.hud.messages[0].player => Some(4);
        first.object(builder_id).expect("builder remains").command_stack.command_names() => vec!["Acquire", "Build"];
    }

    let builder_index = engine.test_object_index(builder_id);
    assert_values! {
        engine.objects[builder_index].commands.front_command_name() => Some("Acquire");
    }
    engine.objects[builder_index].commands.clear_front();
    unit_assert!(engine.objects[builder_index].commands.fail_front_if(CommandId::Build), "retained Build command is forced through its native failure tail");

    let failed = engine.tick()?;
    assert_values! {
        failed.hud.messages.len() => 1, "the failed Build retains exactly the original HUD message";
    }
    let material_messages = failed
        .hud
        .messages
        .iter()
        .filter(|message| {
            message.kind == MessageKind::Target
                && message.target == Some(builder_id)
                && message.lines == vec!["Site", "needs", "1x Wood"]
        })
        .collect::<Vec<_>>();
    assert_values! {
        material_messages.len() => 1,
            "C++ appends with duplicate suppression instead of rendering a second message";
        material_messages[0].id => first_message_id;
        material_messages[0].player => Some(4), "Append retains the original C4GameMessage metadata";
    }
    Ok(())
}

#[test]
fn build_procedure_noncrew_reports_material_without_acquire() -> Result<(), EngineError> {
    let script = NOOP_DEFINITION_SCRIPT;

    let builder_definition = build_actor_definition("Machine", "Machine", script, "Walk", "Walk");
    let structure_definition =
        component_structure_definition("Site", "Site", script, true, None, "Wood", 1);
    let material_definition = test_definition("Wood", "Wood", script);

    let mut engine = Engine::with_seed(8);
    engine.register_definition(builder_definition)?;
    engine.register_definition(structure_definition)?;
    engine.register_definition(material_definition)?;
    engine.set_construction_needs_material(true);

    let structure_id = spawn!(engine, "Site", with_construction: 1_000, with_ordered_components: vec![("Wood".to_owned(), 0)])?;
    let build_state = targeted_action("Build", structure_id);
    let builder_id = spawn!(engine, "Machine", with_action: build_state, with_controller: 6)?;

    let snapshot = engine.tick()?;
    let builder = snapshot.object(builder_id).test_value();
    unit_assert!(builder.command_stack.is_empty(), "noncrew builders must not receive Acquire");
    assert_values! {
        builder.action_procedure.as_deref() => Some("walk");
        snapshot.hud.messages.len() => 1;
        snapshot.hud.messages[0].kind => MessageKind::Target;
        snapshot.hud.messages[0].target => Some(builder_id);
        snapshot.hud.messages[0].player => Some(6);
        snapshot.hud.messages[0].lines => vec!["Site", "needs", "1x Wood"];
    }
    Ok(())
}

#[test]
fn build_needs_material_truthy_runs_after_grab_and_before_stop() -> Result<(), EngineError> {
    let builder_script = r#"#strict 2
local missing_id, missing_count, contents_seen, action_seen, callback_order;

protected func BuildNeedsMaterial(component_id, count)
{
    missing_id = component_id;
    missing_count = count;
    contents_seen = ContentsCount();
    action_seen = GetAction();
    callback_order = callback_order * 10 + 1;
    return 1;
}

protected func BuildAbort()
{
    callback_order = callback_order * 10 + 2;
}
"#;
    let mut builder_definition = test_definition("Bldr", "Builder", builder_script);
    builder_definition.set_c4_callback_convention(true);
    builder_definition.set_crew_member(true);
    set_actions!(
        &mut builder_definition, Some("Walk");
        "Walk" => ActionSpec::default().with_procedure("walk"),
        "Build" => action_spec!(default, with_procedure: "build", with_abort_call: "BuildAbort"),
    );

    let structure_definition =
        component_structure_definition("Site", "Structure", "#strict", true, Some(100), "Wood", 5);
    let material_definition = test_definition("Wood", "Wood", "#strict");
    let mut container_definition = test_definition("Cntn", "Container", "#strict");
    container_definition.set_category(CATEGORY_STRUCTURE);
    set_actions!(
        &mut container_definition, Some("Idle");
        "Idle" => ActionSpec::default(),
        "Build" => ActionSpec::default().with_procedure("build"),
    );

    let mut engine = Engine::with_seed(9);
    engine.register_definition(builder_definition)?;
    engine.register_definition(structure_definition)?;
    engine.register_definition(material_definition)?;
    engine.register_definition(container_definition)?;
    engine.set_construction_needs_material(true);

    let container_id =
        engine.spawn_object(SpawnConfig::new("Cntn").with_action(ActionState::new("Build")))?;
    let structure_id = spawn!(engine, "Site", with_construction: 75_000, with_ordered_components: vec![("Wood".to_owned(), 1)], with_container: container_id)?;
    let build_state = targeted_action("Build", structure_id);
    let builder_id =
        spawn!(engine, "Bldr", with_action: build_state, with_alive: true, with_crew_member: true)?;
    let wood_id = spawn!(engine, "Wood", with_construction: FULL_CON, with_container: builder_id)?;
    let container_wood_id =
        spawn!(engine, "Wood", with_construction: FULL_CON, with_container: container_id)?;

    let snapshot = engine.tick()?;
    let builder = snapshot.object(builder_id).test_value();
    assert_snapshot_locals!(builder;
        "missing_id" => Some(&Value::C4Id("Wood".into()));
        "missing_count" => Some(&Value::Int(2));
        "contents_seen" => Some(&Value::Int(0));
        "action_seen" => Some(&Value::String("Build".to_owned().into()));
        "callback_order" => Some(&Value::Int(12)),
            "BuildNeedsMaterial must run before ObjectComStop's abort callback";
    );
    unit_assert_eq!(builder.action_procedure.as_deref() => Some("walk"));
    unit_assert!(builder.command_stack.is_empty());
    unit_assert!(snapshot.hud.messages.is_empty());
    unit_assert!(snapshot.object(wood_id).is_none(), "grabbed material is consumed");
    unit_assert!(snapshot.object(container_wood_id).is_none(), "the construction-container pass also precedes the callback");
    let structure = snapshot.object(structure_id).test_value();
    unit_assert_eq!(structure.construction => 75_000);
    unit_assert_eq!(structure.components.get("Wood") => Some(3));
    Ok(())
}

#[test]
fn build_procedure_consumes_components_from_builder() -> Result<(), EngineError> {
    let script = NOOP_DEFINITION_SCRIPT;

    let mut builder_definition =
        build_actor_definition("Builder", "Builder", script, "Idle", "Idle");
    builder_definition.set_category(DEFAULT_CATEGORY);
    builder_definition.set_mass(50);
    builder_definition.set_physical(physical! {
            can_construct: 1
    });

    let structure_definition = component_structure_definition(
        "Structure",
        "Structure",
        script,
        true,
        Some(100),
        "Wood",
        1,
    );

    let mut material_definition = test_definition("Wood", "Wood", script);
    material_definition.set_mass(20);

    let mut engine = Engine::with_seed(11);
    engine.register_definition(builder_definition)?;
    engine.register_definition(structure_definition)?;
    engine.register_definition(material_definition)?;
    engine.set_construction_needs_material(true);

    let structure_id = engine.spawn_test_object(SpawnConfig::new("Structure").with_construction(0));

    let build_state = targeted_action("Build", structure_id);
    let builder_id = spawn_test!(engine, "Builder", with_action: build_state, with_command_direction: CommandDirection::Right);

    let wood_id = engine.spawn_test_object(SpawnConfig::new("Wood").with_construction(FULL_CON));
    engine
        .apply_object_update(wood_id, ObjectUpdate::new().with_container(builder_id))
        .test_value();

    let snapshot = engine.tick()?;
    let structure = snapshot.object(structure_id).test_value();
    unit_assert!(structure.construction > 0, "construction should advance when components are available");
    let components = structure.components.get("Wood");
    unit_assert_eq!(components => Some(1));
    unit_assert!(snapshot.object(wood_id).is_none(), "component should be consumed during build");
    Ok(())
}

#[test]
fn build_consumes_only_eligible_first_material_via_assign_removal() -> Result<(), EngineError> {
    let builder_script = r#"#strict
local removal_order, contents_seen, component_seen, contained_seen;

protected func ContentsDestruction(material)
{
    removal_order = removal_order * 10 + 1;
    contents_seen = ContentsCount(WOOD);
    component_seen = GetComponent(WOOD, 0, GetActionTarget());
    contained_seen = Contained(material) == this();
}

public func MaterialDestruction()
{
    removal_order = removal_order * 10 + 2;
}
"#;
    let material_script = r#"#strict
protected func Destruction()
{
    if (Contained()) Contained()->MaterialDestruction();
}
"#;

    let mut builder = build_actor_definition("BLDR", "Builder", builder_script, "Walk", "Walk");
    builder.set_c4_callback_convention(true);
    builder.set_physical(physical! {
            can_construct: 1
    });

    // At Con=10%, inserting one object exactly satisfies the material gate.
    // The following successful DoCon must retain one rather than auto-gaining
    // a second component while fNoComponentChange/fNeedMaterial is true.
    let site =
        component_structure_definition("SITE", "Site", "#strict", false, Some(100), "WOOD", 10);

    let wood = test_definition("WOOD", "Wood", material_script);
    let mut engine = Engine::with_seed(55);
    engine.register_definition(builder)?;
    engine.register_definition(site)?;
    engine.register_definition(wood)?;
    engine.set_construction_needs_material(true);

    let spawn_pair = |engine: &mut Engine| -> Result<(ObjectId, ObjectId), EngineError> {
        let site_id = spawn!(engine, "SITE", with_construction: 1_000, with_ordered_components: vec![("WOOD".to_owned(), 0)])?;
        let action = targeted_action("Build", site_id);
        let builder_id = spawn!(engine, "BLDR", with_action: action, with_command_direction: CommandDirection::Right)?;
        Ok((builder_id, site_id))
    };

    let (valid_builder, valid_site) = spawn_pair(&mut engine)?;
    let valid_wood =
        spawn!(engine, "WOOD", with_construction: FULL_CON, with_container: valid_builder)?;

    let (burning_builder, burning_site) = spawn_pair(&mut engine)?;
    let burning_a =
        spawn!(engine, "WOOD", with_construction: FULL_CON, with_container: burning_builder)?;
    let burning_b =
        spawn!(engine, "WOOD", with_construction: FULL_CON, with_container: burning_builder)?;
    let burning_head = engine
        .object_snapshot(burning_builder)
        .and_then(|builder| builder.contents.first().copied())
        .test_value();
    let mut fire = ObjectUpdate::new();
    fire.stage_ignite(0, 0);
    engine.apply_object_update(burning_head, fire)?;

    let (partial_builder, partial_site) = spawn_pair(&mut engine)?;
    let partial_a =
        spawn!(engine, "WOOD", with_construction: FULL_CON, with_container: partial_builder)?;
    let partial_b =
        spawn!(engine, "WOOD", with_construction: FULL_CON, with_container: partial_builder)?;
    let partial_head = engine
        .object_snapshot(partial_builder)
        .and_then(|builder| builder.contents.first().copied())
        .test_value();
    engine.apply_object_update(
        partial_head,
        ObjectUpdate::new().with_construction(FULL_CON / 2),
    )?;

    let snapshot = engine.tick()?;
    unit_assert!(snapshot.object(valid_wood).is_none());
    let valid_builder = snapshot.object(valid_builder).test_value();
    assert_snapshot_locals!(valid_builder;
        "removal_order" => Some(&Value::Int(12));
        "contents_seen" => Some(&Value::Int(0));
        "component_seen" => Some(&Value::Int(1));
        "contained_seen" => Some(&Value::Bool(true));
    );
    assert_values! {
        snapshot.object(valid_site).and_then(|site| site.components.get("WOOD")) => Some(1);
    }

    for (label, material) in [
        ("burning first", burning_a),
        ("burning duplicate", burning_b),
        ("partial first", partial_a),
        ("partial duplicate", partial_b),
    ] {
        unit_assert!(snapshot.object(material).is_some(), "{label} survives");
    }
    for site in [burning_site, partial_site] {
        let site = snapshot.object(site).test_value();
        unit_assert_eq!(site.construction => 1_000);
        unit_assert_eq!(site.components.get("WOOD") => Some(0));
    }
    Ok(())
}

#[test]
fn build_uses_definition_custom_components_with_builder_argument() -> Result<(), EngineError> {
    let builder_script = r#"#strict
local component_queries;

public func RecordComponentQuery()
{
    component_queries++;
}
"#;
    let site_script = r#"#strict
protected func GetCustomComponents(builder)
{
    builder->RecordComponentQuery();
    return [METL];
}
"#;

    let mut builder = build_actor_definition("BLDR", "Builder", builder_script, "Walk", "Walk");
    builder.set_physical(physical! {
            can_construct: 1
    });

    let site =
        component_structure_definition("SITE", "Site", site_script, false, Some(100), "WOOD", 1);

    let mut engine = Engine::with_seed(57);
    engine.register_definition(builder)?;
    engine.register_definition(site)?;
    engine.register_test_script_definition("WOOD", "Wood", "#strict");
    engine.register_test_script_definition("METL", "Metal", "#strict");
    engine.set_construction_needs_material(true);

    let site_id = spawn!(engine, "SITE", with_construction: 1_000, with_ordered_components: vec![("WOOD".to_owned(), 0)])?;
    let action = targeted_action("Build", site_id);
    let builder_id = engine.spawn_object(SpawnConfig::new("BLDR").with_action(action))?;
    let metal_id = spawn!(engine, "METL", with_construction: FULL_CON, with_container: builder_id)?;

    let snapshot = engine.tick()?;
    unit_assert!(snapshot.object(metal_id).is_none());
    let site = snapshot.object(site_id).test_value();
    unit_assert_eq!(site.construction => 2_500);
    unit_assert_eq!(site.components.get("METL") => Some(1));
    unit_assert_eq!(site.components.get("WOOD") => Some(0));
    assert_values! {
        snapshot.object(builder_id).and_then(|builder| builder.local_vars.get("component_queries")) => Some(&Value::Int(1)),
            "Build calls the definition hook once with the live builder";
    }
    Ok(())
}

#[test]
fn build_uses_can_construct_turn_to_docon_components_and_repair() -> Result<(), EngineError> {
    fn builder_definition(id: &str, can_construct: i32) -> Result<Definition, EngineError> {
        let mut definition = build_actor_definition(
            id,
            id,
            r#"#strict
        local turn_damage;
        public func RecordTurnDamage(value) { turn_damage = value; }
        "#,
            "Walk",
            "Walk",
        );
        definition.set_physical(physical! {
            can_construct: can_construct,
        });
        Ok(definition)
    }

    let mut site =
        component_structure_definition("SITE", "Site", "#strict", false, Some(100), "STON", 100);
    site.set_build_turn_to(Some("DONE".to_owned()));

    let mut engine = Engine::with_seed(56);
    engine.register_definition(builder_definition("FAST", 200)?)?;
    engine.register_definition(builder_definition("ZERO", 0)?)?;
    engine.register_definition(site)?;
    let mut done = test_definition(
        "DONE",
        "Done",
        r#"#strict
    protected func RejectEntrance(container)
    {
        container->RecordTurnDamage(GetDamage());
        return false;
    }
    "#,
    );
    done.set_c4_callback_convention(true);
    engine.register_definition(done)?;
    engine.register_test_script_definition("STON", "Stone", "#strict");

    let spawn_build = |engine: &mut Engine,
                       builder: &str|
     -> Result<(ObjectId, ObjectId), EngineError> {
        let site_id = spawn!(engine, "SITE", with_construction: 1_000, with_ordered_components: vec![("STON".to_owned(), 0)])?;
        engine.apply_object_update(site_id, ObjectUpdate::new().with_damage(77))?;
        let action = targeted_action("Build", site_id);
        let builder_id = engine.spawn_object(SpawnConfig::new(builder).with_action(action))?;
        Ok((builder_id, site_id))
    };
    let (fast_builder, fast_site) = spawn_build(&mut engine, "FAST")?;
    let (zero_builder, zero_site) = spawn_build(&mut engine, "ZERO")?;

    let full_site = spawn!(engine, "SITE", with_construction: 99_000, with_ordered_components: vec![("STON".to_owned(), 0)])?;
    let full_action = targeted_action("Build", full_site);
    engine.spawn_object(SpawnConfig::new("FAST").with_action(full_action))?;

    // A contained construction silently exits and re-enters its builder
    // during BuildTurnTo. The new definition's RejectEntrance observes
    // Damage before Build's following repair assignment.
    let mut internal_action = ActionState::new("Build");
    let internal_builder =
        engine.spawn_object(SpawnConfig::new("FAST").with_action(internal_action.clone()))?;
    let internal_site =
        spawn!(engine, "SITE", with_construction: 1_000, with_container: internal_builder)?;
    internal_action.target = Some(internal_site);
    let internal_builder_idx = engine.test_object_index(internal_builder);
    engine.objects[internal_builder_idx].state.action = internal_action;
    engine.apply_object_update(internal_site, ObjectUpdate::new().with_damage(77))?;

    let snapshot = engine.tick()?;
    let fast = snapshot.object(fast_site).test_value();
    assert_values! {
        fast.construction => 4_000;
        fast.components.get("STON") => Some(4);
        fast.definition_id => "DONE";
        fast.damage => 0;
        snapshot.object(fast_builder).map(|builder| builder.action.name.as_str()) => Some("Build");
    }

    let zero = snapshot.object(zero_site).test_value();
    assert_values! {
        zero.construction => 1_000;
        zero.components.get("STON") => Some(0);
        zero.definition_id => "SITE";
        zero.damage => 77;
        snapshot.object(zero_builder).and_then(|builder| builder.action_procedure.as_deref()) => Some("walk");
    }

    let full = snapshot.object(full_site).test_value();
    unit_assert_eq!(full.construction => FULL_CON);
    unit_assert_eq!(full.components.get("STON") => Some(100));

    let internal = snapshot.object(internal_site).test_value();
    assert_values! {
        internal.definition_id => "DONE";
        internal.damage => 0;
        snapshot.object(internal_builder).and_then(|builder| builder.local_vars.get("turn_damage")) => Some(&Value::Int(77)),
            "BuildTurnTo callbacks run before the successful-build repair write";
    }
    Ok(())
}

#[test]
fn applies_velocity_changes_from_step_callback() {
    let mut engine = definition_engine(123, build_definition());
    engine.set_physics(PhysicsSettings::new(0, 20, -20));

    let id = spawn_test!(engine, "Test", with_category: CATEGORY_OBJECT, with_position: Vector2::new(0, 0), with_velocity: Vector2::new(1, 0), with_energy: 50);

    let object = tick_test_object(&mut engine, id);
    unit_assert_eq!(object.position => Vector2::new(1, 0));
    unit_assert_eq!(object.velocity => Vector2::new(2, 0));

    let object = tick_test_object(&mut engine, id);
    unit_assert_eq!(object.position => Vector2::new(3, 0));
    unit_assert_eq!(object.velocity => Vector2::new(3, 0));
}
#[test]
fn push_procedure_without_target_resets_to_default() {
    let script = NOOP_DEFINITION_SCRIPT;

    let definition = action_definition(
        "Pusher",
        "Pusher",
        script,
        Some("Idle"),
        procedure_actions([("Idle", "walk"), ("Push", "push")]),
    );

    let mut engine = definition_engine(12, definition);

    let push_state = ActionState::new("Push");
    let id = spawn_test!(engine, "Pusher", with_action: push_state, with_command_direction: CommandDirection::Right);

    let object = tick_test_object(&mut engine, id);
    unit_assert_eq!(object.action.name => "Idle");
    unit_assert_eq!(object.velocity => Vector2::ZERO);
    unit_assert_eq!(object.command_direction => CommandDirection::Stop);
    let index = engine.test_object_index(id);
    assert_values! {
        engine.objects[index].state.t_attach & CNAT_BOTTOM => 0,
            "a native Push early return must not latch CNAT_Bottom";
        engine.objects[index].frame_t_attach => engine.objects[index].state.t_attach;
    }
}

#[test]
fn failed_push_stands_in_walk_and_adds_cpp_delay_command() {
    // Every DFA_PUSH failure calls StopActionDelayCommand: ObjectComStop
    // stands the Clonk in Walk, then a 50-frame Wait is added to the top
    // of its command stack (C4Object.cpp:4677-4681,5060-5094).
    let definition = action_definition_fixture!(
        "Pusher",
        "Pusher",
        "",
        Some("Idle");
        "Idle" => ActionSpec::default(),
        "Walk" => ActionSpec::default().with_procedure("walk"),
        "Push" => ActionSpec::default().with_procedure("push").with_delay(2),
    );

    let (mut engine, id) = definition_fixture_case!(13, definition, "Pusher", with_action: ActionState::new("Push"), with_command_direction: CommandDirection::Right);

    engine.tick_without_snapshot().test_value();

    let object = engine.test_object_snapshot(id);
    unit_assert_eq!(object.action.name => "Walk");
    assert_values! {
        (object.action.phase, object.action.ticks, object.action.time) => (0, 0, 0),
            "the failed Push return skips its stale phase tail";
    }
    unit_assert_eq!(object.velocity => Vector2::ZERO);
    unit_assert_eq!(object.position => Vector2::ZERO);
    unit_assert_eq!(object.command_direction => CommandDirection::Stop);
    let index = engine.test_object_index(id);
    unit_assert_eq!(engine.objects[index].commands.snapshot().command_names() => vec!["Wait".to_string()]);
    let stack = serde_json::to_value(engine.objects[index].commands.snapshot()).test_value();
    unit_assert_eq!(stack["commands"][0]["mode"] => serde_json::json!("SilentSub"));
    unit_assert_eq!(stack["commands"][0]["update_interval"] => serde_json::json!(50));
}

fn push_containment_engine(with_physical: bool, with_actor_turn: bool) -> Engine {
    let pusher_script = with_actor_turn.then_some(
        r#"#strict
local turn_starts, turn_start_dir, turn_sets_xdir;
protected func TurnStart()
{
    turn_starts = turn_starts + 1;
    turn_start_dir = GetDir();
    if (turn_sets_xdir) SetXDir(100, this(), 100);
    return true;
}
"#,
    );
    let mut pusher = test_definition(
        "PCPS",
        "Containment pusher",
        pusher_script.unwrap_or_default(),
    );
    pusher.set_c4_callback_convention(with_actor_turn);
    pusher.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
    if with_physical {
        pusher.set_physical(physical! {
                    walk: if with_actor_turn { 100 } else { 35_000 },
                    push: 45_000
        });
    } else {
        pusher
            .set_movement_profile(movement_profile!(with_walk_speed: 6, with_walk_acceleration: 3));
    }
    let mut push_action = ActionSpec::default().with_procedure("PUSH");
    if with_actor_turn {
        push_action = push_action
            .with_directions(2)
            .with_turn_action("Turn")
            .with_delay(1)
            .with_length(200);
    }
    let mut pusher_actions = HashMap::from([
        ("Idle".to_string(), ActionSpec::default()),
        (
            "Walk".to_string(),
            ActionSpec::default().with_procedure("WALK"),
        ),
        ("Push".to_string(), push_action),
    ]);
    if with_actor_turn {
        pusher_actions.insert(
            "Turn".to_string(),
            action_spec!(default, with_directions: 2, with_delay: 1, with_length: 200, with_start_call: "TurnStart"),
        );
    }
    pusher.configure_actions(Some("Idle".to_string()), pusher_actions);

    let mut target = test_definition("PCTG", "Containment target", "");
    target.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
    target.set_grab(1);
    target.set_mass(200);

    let mut engine = action_definitions_engine!(65; pusher, target);
    engine.set_physics(action_horizontal_physics());
    engine
}

fn spawn_push_direction_case(engine: &mut Engine, target_action: &str) -> (ObjectId, ObjectId) {
    let target_script = r#"#strict
local turn_starts, turn_start_dir;
public func ReadDirection() { return GetDir(); }
protected func TurnStart()
{
    turn_starts = turn_starts + 1;
    turn_start_dir = GetDir();
    return 1;
}
"#;
    let mut target = test_definition("PCDR", "Direction target", target_script);
    target.set_c4_callback_convention(true);
    target.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
    target.set_grab(1);
    target.set_mass(200);
    set_actions!(
        &mut target, Some("Idle");
        "Idle" => ActionSpec::default(),
        "Drive" => action_spec!(default, with_directions: 2, with_turn_action: "Turn"),
        "Turn" => action_spec!(default, with_directions: 2, with_start_call: "TurnStart"),
    );
    engine.register_test_definition(target);

    let target_id = engine.spawn_test_object(
        SpawnConfig::new("PCDR")
            .with_category(CATEGORY_VEHICLE)
            .with_position(Vector2::new(10, 0))
            .with_action(ActionState::new(target_action))
            .with_direction(Direction::Left)
            // Raw positive xdir that still rounds to integer zero:
            // Push/SetDir must inspect C4Fixed directly.
            .with_fixed_velocity(FixedVec2::new(C4Fixed::from_raw(12_345), C4Fixed::ZERO))
            .with_mobile(true)
            .with_loaded(true),
    );
    let push = targeted_action("Push", target_id);
    let pusher_id = spawn_test!(engine, "PCPS", with_category: CATEGORY_OBJECT, with_position: Vector2::ZERO, with_action: push, with_command_direction: CommandDirection::Right, with_loaded: true);
    (pusher_id, target_id)
}

#[test]
fn push_faces_from_a_positive_subpixel_xdir() {
    // DFA_PUSH tests the raw C4Fixed xdir before SetDir, which runs the
    // TurnAction while the old direction is still live even when the
    // whole-pixel velocity mirror is zero (C4Object.cpp:5103-5108).
    let mut engine = push_containment_engine(false, true);
    let (pusher_id, _) = spawn_push_direction_case(&mut engine, "Idle");
    let pusher_idx = engine.test_object_index(pusher_id);
    engine.objects[pusher_idx]
        .set_fixed_velocity(FixedVec2::new(C4Fixed::from_raw(-196_607), C4Fixed::ZERO));
    engine.objects[pusher_idx].state.mobile = true;

    let _ = engine.apply_physics_at_index(pusher_idx).test_value();

    assert_subpixel_actor_turned(&engine, pusher_id, 1);
}

#[test]
fn physical_push_runs_turn_action_for_a_positive_subpixel_xdir() {
    // Native DFA_PUSH writes the raw follow xdir before SetDir. With this
    // tiny Walk physical the result is raw +183 but still mirrors to zero;
    // SetDir nevertheless runs TurnAction exactly once
    // (C4Object.cpp:5103-5108).
    let mut engine = push_containment_engine(true, true);
    let (pusher_id, _) = spawn_push_direction_case(&mut engine, "Idle");
    let pusher_idx = engine.test_object_index(pusher_id);

    let _ = engine.apply_physics_at_index(pusher_idx).test_value();

    assert_subpixel_actor_turned(&engine, pusher_id, 183);
}

#[test]
fn push_latches_phase_advance_before_turn_action_mutates_xdir() {
    // DFA_PUSH assigns iPhaseAdvance from the raw follow xdir immediately
    // before SetDir. TurnAction may change live xdir, but the old phase value
    // remains latched for the tail of ExecAction (C4Object.cpp:5106-5108).
    let mut engine = push_containment_engine(true, true);
    let (pusher_id, _) = spawn_push_direction_case(&mut engine, "Idle");
    let pusher_idx = engine.test_object_index(pusher_id);
    engine.objects[pusher_idx]
        .state
        .local_vars
        .insert("turn_sets_xdir".to_string(), Value::Int(1));

    let _ = engine.test_tick();

    let pusher = test_object(&engine, pusher_id);
    unit_assert_eq!(pusher.fixed_velocity.x => itofix(1));
    unit_assert_eq!(pusher.state.action.name => "Turn");
    unit_assert_eq!(pusher.state.action.phase => 0);
}

#[test]
fn push_keeps_an_idle_targets_direction() {
    // C4Object::Push calls SetDir from the target's pre-force raw xdir,
    // but SetDir rejects ActIdle. The positive xdir still receives the
    // push force; only Action.Dir remains Left/zero.
    let mut engine = push_containment_engine(true, false);
    let (pusher_id, target_id) = spawn_push_direction_case(&mut engine, "Idle");
    let pusher_idx = engine.test_object_index(pusher_id);

    engine.apply_physics_at_index(pusher_idx).test_value();

    let target_idx = engine.test_object_index(target_id);
    unit_assert!(engine.objects[target_idx].fixed_velocity.x.val() > 12_345);
    unit_assert_eq!(engine.objects[target_idx].state.direction => Direction::Left);
    assert_values! {
        engine.call_object_function(target_idx, "ReadDirection", Vec::new()).expect("GetDir reads the idle target") => Value::Int(0);
    }
}

#[test]
fn push_runs_an_active_targets_turn_action_once() {
    // SetDir validates Drive's two directions, runs TurnAction before
    // assigning the new direction, then Push continues from the live
    // post-callback velocity.
    let mut engine = push_containment_engine(true, false);
    let (pusher_id, target_id) = spawn_push_direction_case(&mut engine, "Drive");
    let pusher_idx = engine.test_object_index(pusher_id);

    engine.apply_physics_at_index(pusher_idx).test_value();

    let target_idx = engine.test_object_index(target_id);
    assert_values! {
        engine.objects[target_idx].state.action.name => "Turn";
        engine.objects[target_idx].state.direction => Direction::Right;
        engine.objects[target_idx].state.local_vars.get("turn_starts") => Some(&Value::Int(1));
        engine.objects[target_idx].state.local_vars.get("turn_start_dir") => Some(&Value::Int(0)),
            "TurnAction Start observes the old direction";
    }

    let pusher_idx = engine.test_object_index(pusher_id);
    engine.apply_physics_at_index(pusher_idx).test_value();
    let target_idx = engine.test_object_index(target_id);
    assert_values! {
        engine.objects[target_idx].state.local_vars.get("turn_starts") => Some(&Value::Int(1)),
            "the TurnAction runs only for the facing change";
    }
}

fn spawn_grounded_procedure_turn_actor(
    engine: &mut Engine,
    definition_id: &str,
    action_name: &str,
    procedure: &str,
    target: Option<ObjectId>,
    facing: Direction,
) -> ObjectId {
    let script = r#"#strict
local turn_starts, turn_walked;
protected func TurnStart()
{
    turn_starts = turn_starts + 1;
    turn_walked = AdjustWalkRotation(20, 20, 100);
    return 1;
}
"#;
    let mut actor = test_definition(definition_id, definition_id, script);
    actor.set_c4_callback_convention(true);
    actor.set_rotateable(1);
    actor.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
    actor.set_physical(physical! {
            walk: 35_000,
            push: 45_000
    });
    set_actions!(
        &mut actor, Some("Idle");
        "Idle" => ActionSpec::default(),
        "Walk" => ActionSpec::default().with_procedure("WALK"),
        action_name => action_spec!(default, with_procedure: procedure, with_directions: 2, with_turn_action: "Turn"),
        "Turn" => action_spec!(default, with_directions: 2, with_start_call: "TurnStart"),
    );
    engine.register_test_definition(actor);

    let mut action = ActionState::new(action_name);
    action.target = target;
    spawn_test!(engine, definition_id, with_category: CATEGORY_OBJECT, with_position: Vector2::ZERO, with_action: action, with_direction: facing, with_command_direction: CommandDirection::Right, with_loaded: true)
}

fn arm_valid_shape_attach(engine: &mut Engine, id: ObjectId) {
    let idx = engine.test_object_index(id);
    engine.objects[idx].state.shape_attach = ShapeAttachRecord {
        mat_valid: true,
        mat_vehicle: false,
        x: 0,
        y: 0,
        vtx: 0,
    };
}

fn assert_turn_before_bottom_attach(
    engine: &Engine,
    actor: ObjectId,
    turn_message: &str,
    walked_message: Option<&str>,
    expected_attach: u32,
    attach_message: &str,
) {
    let actor = test_object(engine, actor);
    assert_values! {
        actor.state.local_vars.get("turn_starts") => Some(&Value::Int(1)), "{turn_message}";
    }
    if let Some(message) = walked_message {
        assert_values! {
            actor.state.local_vars.get("turn_walked") => Some(&Value::Bool(false)), "{message}";
        }
    } else {
        assert_values! {
            actor.state.local_vars.get("turn_walked") => Some(&Value::Bool(false));
        }
    }
    assert_values! {
        actor.state.t_attach & CNAT_BOTTOM => expected_attach, "{attach_message}";
        actor.frame_t_attach => actor.state.t_attach;
    }
}

#[test]
fn push_turn_action_sees_no_bottom_attach() {
    // C4Object.cpp:5106-5112: DFA_PUSH calls SetDir, then ORs CNAT_Bottom.
    // AdjustWalkRotation from TurnAction must therefore return false.
    let mut engine = push_containment_engine(true, false);
    let target_id = spawn_test!(engine, "PCTG", with_category: CATEGORY_VEHICLE, with_position: Vector2::new(10, 0), with_loaded: true);
    let pusher_id = spawn_grounded_procedure_turn_actor(
        &mut engine,
        "PTAT",
        "Push",
        "PUSH",
        Some(target_id),
        Direction::Left,
    );
    arm_valid_shape_attach(&mut engine, pusher_id);
    let pusher_idx = engine.test_object_index(pusher_id);

    engine.apply_physics_at_index(pusher_idx).test_value();

    assert_turn_before_bottom_attach(
        &engine,
        pusher_id,
        "Push SetDir must fire TurnAction",
        Some("TurnAction Start must see Action.t_attach without CNAT_Bottom"),
        CNAT_BOTTOM,
        "successful Push still grounds after SetDir",
    );
}

#[test]
fn pull_turn_action_sees_no_bottom_attach() {
    // C4Object.cpp:5189-5196: DFA_PULL calls SetDir, then ORs CNAT_Bottom.
    let mut engine = pull_failure_engine();
    let target_id = spawn_test!(engine, "L73W", with_category: CATEGORY_VEHICLE, with_position: Vector2::new(10, 0), with_loaded: true);
    let puller_id = spawn_grounded_procedure_turn_actor(
        &mut engine,
        "PLAT",
        "Pull",
        "PULL",
        Some(target_id),
        Direction::Left,
    );
    arm_valid_shape_attach(&mut engine, puller_id);
    let puller_idx = engine.test_object_index(puller_id);

    engine.apply_physics_at_index(puller_idx).test_value();

    assert_turn_before_bottom_attach(
        &engine,
        puller_id,
        "Pull SetDir must fire TurnAction",
        Some("TurnAction Start must see Action.t_attach without CNAT_Bottom"),
        CNAT_BOTTOM,
        "successful Pull still grounds after SetDir",
    );
}

#[test]
fn fight_turn_action_sees_no_bottom_attach() {
    // C4Object.cpp:5241-5259: DFA_FIGHT faces the target, then approaches,
    // then distance-checks, and only then ORs CNAT_Bottom.
    let mut engine = fight_failure_engine();
    let target_id = spawn_test!(engine, "L73O", with_category: CATEGORY_OBJECT, with_position: Vector2::new(8, 0), with_action: ActionState::new("Fight"), with_loaded: true);
    let fighter_id = spawn_grounded_procedure_turn_actor(
        &mut engine,
        "FTAT",
        "Fight",
        "FIGHT",
        Some(target_id),
        Direction::Left,
    );
    arm_valid_shape_attach(&mut engine, fighter_id);
    let fighter_idx = engine.test_object_index(fighter_id);

    engine.apply_physics_at_index(fighter_idx).test_value();

    assert_turn_before_bottom_attach(
        &engine,
        fighter_id,
        "Fight SetDir must fire TurnAction",
        Some("TurnAction Start must see Action.t_attach without CNAT_Bottom"),
        CNAT_BOTTOM,
        "in-range Fight still grounds after the distance check",
    );
}

#[test]
fn out_of_range_fight_never_attaches_bottom() {
    // C4Object.cpp:5244-5257: the distance return happens before the
    // CNAT_Bottom write, so an out-of-range Fight keeps the pre-procedure bits.
    let mut engine = fight_failure_engine();
    let target_id = spawn_test!(engine, "L73O", with_category: CATEGORY_OBJECT, with_position: Vector2::new(40, 0), with_action: ActionState::new("Fight"), with_loaded: true);
    let fighter_id = spawn_grounded_procedure_turn_actor(
        &mut engine,
        "FTAR",
        "Fight",
        "FIGHT",
        Some(target_id),
        Direction::Left,
    );
    arm_valid_shape_attach(&mut engine, fighter_id);
    let fighter_idx = engine.test_object_index(fighter_id);

    engine.apply_physics_at_index(fighter_idx).test_value();

    assert_turn_before_bottom_attach(
        &engine,
        fighter_id,
        "out-of-range Fight still faces the target through SetDir",
        None,
        0,
        "native early return must not latch CNAT_Bottom",
    );
}

#[test]
fn push_inside_action_target_stops_before_force_and_controller_transfer() {
    // DFA_PUSH checks no target first, then whether the PUSHER is inside
    // Action.Target, before calculating or applying any force
    // (C4Object.cpp:5058-5063). StopActionDelayCommand must leave the
    // existing stack below its pristine SilentSub Wait(50).
    let mut engine = push_containment_engine(true, false);
    let target_id = spawn_test!(engine, "PCTG", with_category: CATEGORY_VEHICLE, with_position: Vector2::new(10, 0), with_controller: 3, with_fixed_velocity: FixedVec2::new(C4Fixed::from_raw(12_345), C4Fixed::ZERO), with_mobile: true, with_loaded: true);
    let push = targeted_action("Push", target_id);
    let pusher_id = spawn_test!(engine, "PCPS", with_category: CATEGORY_OBJECT, with_position: Vector2::ZERO, with_container: target_id, with_controller: 7, with_action: push, with_command_direction: CommandDirection::Right, with_fixed_velocity: FixedVec2::new(
                C4Fixed::from_raw(54_321),
                C4Fixed::from_raw(7_654),
            ), with_mobile: true, with_loaded: true);
    let pusher_idx = engine.test_object_index(pusher_id);
    engine.objects[pusher_idx]
        .commands
        .push_back(
            CommandRequest::new(CommandId::MoveTo)
                .with_tx(Some(20))
                .with_ty(Some(0)),
        )
        .test_value();

    unit_assert!(engine.apply_physics_at_index(pusher_idx).expect("inside-target Push resolves"));

    let pusher = test_object(&engine, pusher_id);
    unit_assert_eq!(pusher.state.action.name => "Walk");
    unit_assert_eq!(pusher.state.command_direction => CommandDirection::Stop);
    unit_assert_eq!(pusher.fixed_velocity => FixedVec2::ZERO);
    unit_assert_eq!(pusher.state.container => Some(target_id));
    unit_assert_eq!(pusher.commands.command_names() => vec!["Wait".to_string(), "MoveTo".to_string()]);
    let stack = serde_json::to_value(pusher.commands.snapshot()).test_value();
    unit_assert_eq!(stack["commands"][0]["mode"] => serde_json::json!("SilentSub"));
    assert_values! {
        stack["commands"][0]["update_interval"] => serde_json::json!(50);
    }

    let target_idx = engine.test_object_index(target_id);
    unit_assert_eq!(engine.objects[target_idx].fixed_velocity.x.val() => 12_345);
    unit_assert_eq!(engine.objects[target_idx].state.controller => 3);
    assert_values! {
        pusher.state.t_attach & CNAT_BOTTOM => 0,
            "inside-target Push returns before the CNAT_Bottom write";
    }
}

#[test]
fn push_rejects_contained_target_on_zero_physical_fallback() {
    // C4Object::Push rejects every contained target before applying force
    // (C4Object.cpp:1785-1790). The zero-physical compatibility path does
    // not call push_object, so ExecAction must preserve that gate too.
    let mut engine = push_containment_engine(false, false);
    let pusher_id = spawn_test!(engine, "PCPS", with_category: CATEGORY_OBJECT, with_position: Vector2::ZERO, with_controller: 7, with_action: ActionState::new("Push"), with_command_direction: CommandDirection::Right, with_loaded: true);
    let target_id = spawn_test!(engine, "PCTG", with_category: CATEGORY_VEHICLE, with_position: Vector2::new(10, 0), with_container: pusher_id, with_controller: 3, with_fixed_velocity: FixedVec2::new(C4Fixed::from_raw(12_345), C4Fixed::ZERO), with_mobile: true, with_loaded: true);
    let pusher_idx = engine.test_object_index(pusher_id);
    engine.objects[pusher_idx].state.action.target = Some(target_id);

    unit_assert!(engine.apply_physics_at_index(pusher_idx).expect("contained-target Push resolves"));

    let pusher_idx = engine.test_object_index(pusher_id);
    let target_idx = engine.test_object_index(target_id);
    unit_assert_eq!(engine.objects[pusher_idx].state.action.name => "Walk");
    unit_assert_eq!(engine.objects[pusher_idx].commands.command_names() => vec!["Wait".to_string()]);
    unit_assert_eq!(engine.objects[target_idx].state.container => Some(pusher_id));
    unit_assert_eq!(engine.objects[target_idx].fixed_velocity.x.val() => 12_345);
    unit_assert_eq!(engine.objects[target_idx].state.controller => 3);
}

#[test]
fn push_from_unrelated_container_still_applies_force_to_inactive_target() {
    // `Contained == Action.Target` is identity, not a generic contained
    // check. Being inside some other object must leave PUSH unchanged,
    // and C4Object::Push accepts every nonzero target Status.
    let mut engine = push_containment_engine(true, false);
    let unrelated_id = spawn_test!(engine, "PCTG", with_category: CATEGORY_VEHICLE, with_position: Vector2::new(-30, 0), with_loaded: true);
    let target_id = spawn_test!(engine, "PCTG", with_category: CATEGORY_VEHICLE, with_position: Vector2::new(10, 0), with_controller: 3, with_loaded: true);
    let push = targeted_action("Push", target_id);
    let pusher_id = spawn_test!(engine, "PCPS", with_category: CATEGORY_OBJECT, with_position: Vector2::ZERO, with_container: unrelated_id, with_controller: 7, with_action: push, with_command_direction: CommandDirection::Right, with_loaded: true);
    let pusher_idx = engine.test_object_index(pusher_id);
    let target_idx = engine.test_object_index(target_id);
    engine.objects[target_idx].state.status = ObjectStatus::Inactive;

    engine.apply_physics_at_index(pusher_idx).test_value();

    let pusher_idx = engine.test_object_index(pusher_id);
    let target_idx = engine.test_object_index(target_id);
    unit_assert_eq!(engine.objects[pusher_idx].state.action.name => "Push");
    unit_assert_eq!(engine.objects[pusher_idx].state.container => Some(unrelated_id));
    unit_assert_eq!(engine.objects[target_idx].state.container => None);
    unit_assert!(engine.objects[pusher_idx].commands.is_empty());
    unit_assert_eq!(engine.objects[pusher_idx].fixed_velocity.x.val() => 64_225);
    unit_assert_eq!(engine.objects[target_idx].fixed_velocity.x.val() => 36_864);
    unit_assert_eq!(engine.objects[target_idx].state.controller => 7);
}

#[test]
fn push_procedure_moves_target_and_pusher() {
    let script = NOOP_DEFINITION_SCRIPT;

    let mut pusher_definition = action_definition_fixture!(
        "Pusher",
        "Pusher",
        script,
        Some("Idle");
        "Idle" => ActionSpec::for_procedure("walk"),
        "Push" => ActionSpec::for_procedure("push").with_directions(2),
    );
    pusher_definition
        .set_movement_profile(movement_profile!(with_walk_speed: 6, with_walk_acceleration: 3));

    let target_definition = procedure_definition("Crate", "Crate", script, "Idle", "walk");

    let mut engine = action_definitions_engine!(18; pusher_definition, target_definition);
    engine.set_physics(action_horizontal_physics());

    let target_id = spawn_test!(engine, "Crate", with_category: CATEGORY_OBJECT, with_position: Vector2::new(10, 0));
    let target_initial_position = engine.test_object_snapshot(target_id).position;

    let push_state = targeted_action("Push", target_id);

    let pusher_id = spawn_test!(engine, "Pusher", with_category: CATEGORY_OBJECT, with_position: Vector2::new(0, 0), with_action: push_state, with_command_direction: CommandDirection::Right);
    let pusher_idx = engine.test_object_index(pusher_id);
    engine.objects[pusher_idx]
        .set_fixed_velocity(FixedVec2::new(C4Fixed::from_raw(98304), C4Fixed::ZERO));
    // dir writes mobilize (FnSetXDir/FnSetYDir, C4Script.cpp:705,732)
    engine.objects[pusher_idx].state.mobile = true;

    let snapshot = engine.test_tick();
    let pusher = snapshot.object(pusher_id).test_value();
    unit_assert_eq!(pusher.action.name => "Push");
    unit_assert!(pusher.velocity.x > 0, "pusher should move forward");
    unit_assert_eq!(pusher.direction => Direction::Right);

    let target = snapshot.object(target_id).test_value();
    unit_assert!(target.velocity.x >= 0);
    let pusher_idx = engine.test_object_index(pusher_id);
    let target_idx = engine.test_object_index(target_id);
    unit_assert_eq!(engine.objects[pusher_idx].fixed_velocity.x.val() => 294912);
    unit_assert_eq!(engine.objects[target_idx].fixed_velocity.x.val() => 196608);

    let target_after = tick_test_object(&mut engine, target_id);
    unit_assert!(target_after.position.x > target_initial_position.x, "target should advance horizontally");
}

#[test]
fn pull_without_target_stops_in_walk_with_silent_wait() {
    let script = NOOP_DEFINITION_SCRIPT;

    let definition = action_definition(
        "Puller",
        "Puller",
        script,
        Some("Idle"),
        procedure_actions([("Idle", "walk"), ("Walk", "walk"), ("Pull", "pull")]),
    );

    let mut engine = definition_engine(3, definition);

    let pull_state = ActionState::new("Pull");
    let id = spawn_test!(engine, "Puller", with_action: pull_state, with_command_direction: CommandDirection::Right);
    let index = engine.test_object_index(id);
    engine.objects[index].set_fixed_velocity(FixedVec2::new(fixed100(125), fixed100(-75)));
    engine.objects[index]
        .commands
        .push_back(CommandRequest::new(CommandId::MoveTo).with_tx(Some(20)))
        .test_value();

    unit_assert!(engine.apply_physics_at_index(index).expect("targetless Pull resolves"));

    let object = test_object(&engine, id);
    unit_assert_eq!(object.state.action.name => "Walk");
    unit_assert_eq!(object.fixed_velocity => FixedVec2::ZERO);
    unit_assert_eq!(object.state.velocity => Vector2::ZERO);
    unit_assert_eq!(object.state.command_direction => CommandDirection::Stop);
    unit_assert_eq!(object.commands.command_names() => vec!["Wait".to_string(), "MoveTo".to_string()]);
    let stack = serde_json::to_value(object.commands.snapshot()).test_value();
    unit_assert_eq!(stack["commands"][0]["mode"] => serde_json::json!("SilentSub"));
    unit_assert_eq!(stack["commands"][0]["update_interval"] => serde_json::json!(50));
}

fn pull_failure_engine() -> Engine {
    let mut puller = test_definition("L73P", "Puller", "#strict");
    puller.set_category(CATEGORY_OBJECT);
    puller.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
    puller.set_physical(physical! {
            walk: 35_000,
            push: 45_000
    });
    set_actions!(
        &mut puller, Some("Idle");
        "Idle" => ActionSpec::default(),
        "Walk" => ActionSpec::default().with_procedure("WALK"),
        "Pull" => ActionSpec::default().with_procedure("PULL"),
    );

    let wagon_script = r#"#strict
local puller, action_seen;
public func Arm(object actor) { puller = actor; return true; }
protected func GrabLost()
{
    if (puller) action_seen = GetAction(puller);
    return true;
}
"#;
    let mut wagon = test_definition("L73W", "Wagon", wagon_script);
    wagon.set_category(CATEGORY_VEHICLE);
    wagon.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
    wagon.set_grab(1);
    wagon.set_mass(200);

    let mut rejected = test_definition("L73R", "Ungrabable wagon", "#strict");
    rejected.set_category(CATEGORY_VEHICLE);
    rejected.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
    rejected.set_grab(0);
    rejected.set_mass(200);

    let container = test_definition("L73C", "Container", "#strict");

    let mut engine = Engine::with_seed(73);
    engine.set_physics(PhysicsSettings::new(0, 20, -20));
    engine.register_test_definition(puller);
    engine.register_test_definition(wagon);
    engine.register_test_definition(rejected);
    engine.register_test_definition(container);
    engine
}

fn spawn_puller(
    engine: &mut Engine,
    target: ObjectId,
    position: Vector2,
    container: Option<ObjectId>,
    seed_tail: bool,
) -> ObjectId {
    let pull = targeted_action("Pull", target);
    let mut config = SpawnConfig::new("L73P")
        .with_category(CATEGORY_OBJECT)
        .with_position(position)
        .with_controller(7)
        .with_action(pull)
        .with_command_direction(CommandDirection::Right)
        .with_fixed_velocity(FixedVec2::new(fixed100(125), fixed100(-75)))
        .with_mobile(true);
    if let Some(container) = container {
        config = config.with_container(container);
    }
    let puller = engine.spawn_test_object(config);
    if seed_tail {
        let index = engine.test_object_index(puller);
        engine.objects[index]
            .commands
            .push_back(CommandRequest::new(CommandId::MoveTo).with_tx(Some(20)))
            .test_value();
    }
    puller
}

fn assert_l073_pull_stopped(
    engine: &Engine,
    puller: ObjectId,
    expected_commands: &[&str],
    label: &str,
) {
    let object = test_object(engine, puller);
    assert_values! {
        object.state.action.name => "Walk", "{label}";
        object.state.command_direction => CommandDirection::Stop, "{label}";
        object.fixed_velocity => FixedVec2::ZERO, "{label}";
        object.state.velocity => Vector2::ZERO, "{label}";
        object.commands.command_names() => expected_commands.iter().map(|name| (*name).to_string()).collect::<Vec<_>>(), "{label}";
    }
    let stack = serde_json::to_value(object.commands.snapshot()).test_value();
    assert_values! {
        stack["commands"][0]["mode"] => serde_json::json!("SilentSub"), "{label}";
        stack["commands"][0]["update_interval"] => serde_json::json!(50), "{label}";
    }
}

#[test]
fn physical_pull_target_failures_stop_in_walk_with_silent_wait() {
    #[derive(Clone, Copy)]
    enum Failure {
        InsideTarget,
        TargetContained,
        PushRejected,
    }

    for (label, failure) in [
        ("puller inside target", Failure::InsideTarget),
        ("target contained", Failure::TargetContained),
        ("target Push rejected", Failure::PushRejected),
    ] {
        let mut engine = pull_failure_engine();
        let containing = matches!(failure, Failure::TargetContained)
            .then(|| engine.spawn_test_object(SpawnConfig::new("L73C")));
        let target_definition = if matches!(failure, Failure::PushRejected) {
            "L73R"
        } else {
            "L73W"
        };
        let mut target_config = SpawnConfig::new(target_definition)
            .with_category(CATEGORY_VEHICLE)
            .with_position(Vector2::new(10, 0))
            .with_controller(3);
        if let Some(container) = containing {
            target_config = target_config.with_container(container);
        }
        let target = engine.spawn_test_object(target_config);
        let puller_container = matches!(failure, Failure::InsideTarget).then_some(target);
        let puller = spawn_puller(&mut engine, target, Vector2::ZERO, puller_container, true);
        let index = engine.test_object_index(puller);

        let _ = engine
            .apply_physics_at_index(index)
            .unwrap_or_else(|error| panic!("{label}: Pull failed: {error}"));

        assert_l073_pull_stopped(&engine, puller, &["Wait", "MoveTo"], label);
    }
}

#[test]
fn horse_like_pull_range_loss_stops_before_grab_lost() {
    let mut engine = pull_failure_engine();
    let wagon = spawn_test!(engine, "L73W", with_category: CATEGORY_VEHICLE, with_position: Vector2::ZERO, with_controller: 3);
    let horse = spawn_puller(&mut engine, wagon, Vector2::new(100, 0), None, true);
    let wagon_index = engine.test_object_index(wagon);
    engine.call_test_object_function(
        wagon_index,
        "Arm",
        vec![compat::object_reference_value(horse)],
    );

    engine.tick_without_snapshot().test_value();

    assert_l073_pull_stopped(
        &engine,
        horse,
        &["Wait", "MoveTo"],
        "horse range loss without PushTo",
    );
    let horse_index = engine.test_object_index(horse);
    unit_assert_eq!(engine.objects[horse_index].state.action.target => None);
    let wagon_index = engine.test_object_index(wagon);
    unit_assert_eq!(engine.objects[wagon_index].state.controller => 7);
    unit_assert_ne!(engine.objects[wagon_index].fixed_velocity.x => C4Fixed::ZERO);
    assert_values! {
        engine.objects[wagon_index].state.local_vars.get("action_seen") => Some(&Value::String("Walk".to_string().into())),
            "GrabLost observes StopActionDelayCommand first";
    }
}

#[test]
fn pull_range_loss_clears_back_to_push_to() {
    let mut engine = pull_failure_engine();
    let wagon = spawn_test!(engine, "L73W", with_category: CATEGORY_VEHICLE, with_position: Vector2::ZERO, with_controller: 3);
    let horse = spawn_puller(&mut engine, wagon, Vector2::new(100, 0), None, false);
    let wagon_index = engine.test_object_index(wagon);
    engine.call_test_object_function(
        wagon_index,
        "Arm",
        vec![compat::object_reference_value(horse)],
    );

    let horse_index = engine.test_object_index(horse);
    let commands = &mut engine.objects[horse_index].commands;
    commands
        .push_back(CommandRequest::new(CommandId::MoveTo).with_tx(Some(20)))
        .test_value();
    commands
        .push_back(CommandRequest::new(CommandId::PushTo).with_target(Some(wagon)))
        .test_value();
    commands
        .push_back(CommandRequest::new(CommandId::Wait).with_update_interval(90))
        .test_value();

    unit_assert!(engine.apply_physics_at_index(horse_index).expect("horse loses the distant wagon"));

    let horse = test_object(&engine, horse);
    unit_assert_eq!(horse.state.action.name => "Walk");
    unit_assert_eq!(horse.state.command_direction => CommandDirection::Stop);
    unit_assert_eq!(horse.fixed_velocity => FixedVec2::ZERO);
    unit_assert_eq!(horse.state.velocity => Vector2::ZERO);
    unit_assert_eq!(horse.state.action.target => None);
    assert_values! {
        horse.commands.command_names() => vec!["PushTo", "Wait"],
            "GrabLost removes the new delay and approach but preserves PushTo's tail";
    }
    let wagon_index = engine.test_object_index(wagon);
    assert_values! {
        engine.objects[wagon_index].state.local_vars.get("action_seen") => Some(&Value::String("Walk".to_string().into())),
            "StopActionDelayCommand precedes GrabLost";
    }
}

#[test]
fn pull_procedure_moves_target_and_puller() {
    let script = NOOP_DEFINITION_SCRIPT;

    let mut puller_definition = action_definition_fixture!(
        "Puller",
        "Puller",
        script,
        Some("Idle");
        "Idle" => ActionSpec::for_procedure("walk"),
        "Pull" => ActionSpec::for_procedure("pull").with_directions(2),
    );
    puller_definition
        .set_movement_profile(movement_profile!(with_walk_speed: 6, with_walk_acceleration: 3));

    let target_definition = procedure_definition("Crate", "Crate", script, "Idle", "walk");

    let mut engine = action_definitions_engine!(5; puller_definition, target_definition);
    engine.set_physics(action_horizontal_physics());

    let vertices = vec![
        ObjectVertex::new(-10, -10),
        ObjectVertex::new(10, -10),
        ObjectVertex::new(10, 10),
        ObjectVertex::new(-10, 10),
    ];

    let target_id = spawn_test!(engine, "Crate", with_category: CATEGORY_OBJECT, with_position: Vector2::new(0, 0), with_vertices: vertices.clone());
    let target_initial_position = engine.test_object_snapshot(target_id).position;

    let pull_state = targeted_action("Pull", target_id);

    let puller_id = spawn_test!(engine, "Puller", with_category: CATEGORY_OBJECT, with_position: Vector2::new(20, 0), with_vertices: vertices, with_action: pull_state, with_command_direction: CommandDirection::Right);
    let puller_idx = engine.test_object_index(puller_id);
    engine.objects[puller_idx]
        .set_fixed_velocity(FixedVec2::new(C4Fixed::from_raw(98304), C4Fixed::ZERO));
    // dir writes mobilize (FnSetXDir/FnSetYDir, C4Script.cpp:705,732)
    engine.objects[puller_idx].state.mobile = true;

    let snapshot = engine.test_tick();
    let puller = snapshot.object(puller_id).test_value();
    unit_assert_eq!(puller.action.name => "Pull");
    unit_assert!(puller.velocity.x > 0, "puller should move forward");
    unit_assert_eq!(puller.direction => Direction::Right);

    let target = snapshot.object(target_id).test_value();
    unit_assert!(target.velocity.x >= 0);
    let puller_idx = engine.test_object_index(puller_id);
    let target_idx = engine.test_object_index(target_id);
    unit_assert_eq!(engine.objects[puller_idx].fixed_velocity.x.val() => 294912);
    unit_assert_eq!(engine.objects[target_idx].fixed_velocity.x.val() => 196608);

    let target_after = tick_test_object(&mut engine, target_id);
    unit_assert!(target_after.position.x > target_initial_position.x, "target should advance horizontally",);
}

fn subpixel_pull_direction_case(with_physical: bool) -> (Engine, ObjectId) {
    let actor_script = r#"#strict
local turn_starts, turn_start_dir, turn_sets_xdir;
protected func TurnStart()
{
    turn_starts = turn_starts + 1;
    turn_start_dir = GetDir();
    if (turn_sets_xdir) SetXDir(100, this(), 100);
    return true;
}
"#;
    let mut puller = test_definition("SPUL", "Subpixel puller", actor_script);
    puller.set_c4_callback_convention(true);
    puller.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
    if with_physical {
        puller.set_physical(physical! {
                    walk: 100,
                    push: 45_000
        });
    } else {
        puller
            .set_movement_profile(movement_profile!(with_walk_speed: 6, with_walk_acceleration: 3));
    }
    set_actions!(
        &mut puller, Some("Idle");
        "Idle" => ActionSpec::default(),
        "Pull" => action_spec!(default, with_procedure: "PULL", with_directions: 2, with_turn_action: "Turn", with_delay: 1, with_length: 200),
        "Turn" => action_spec!(default, with_directions: 2, with_delay: 1, with_length: 200, with_start_call: "TurnStart"),
    );

    let mut target = test_definition("SPUT", "Subpixel pull target", "");
    target.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
    target.set_grab(1);
    target.set_mass(200);

    let mut engine = action_definitions_engine!(355; puller, target);
    engine.set_physics(action_horizontal_physics());
    let target_id = spawn_test!(engine, "SPUT", with_category: CATEGORY_VEHICLE, with_position: if with_physical {
        Vector2::new(12, 0)
    } else {
        Vector2::ZERO
    });
    let pull = targeted_action("Pull", target_id);
    let puller_id = spawn_test!(engine, "SPUL", with_category: CATEGORY_OBJECT, with_position: if with_physical {
                Vector2::ZERO
            } else {
                Vector2::new(20, 0)
            }, with_action: pull, with_direction: Direction::Left, with_command_direction: CommandDirection::Right, with_fixed_velocity: FixedVec2::new(C4Fixed::from_raw(-196_607), C4Fixed::ZERO), with_mobile: true);
    (engine, puller_id)
}

fn assert_subpixel_actor_turned(engine: &Engine, actor_id: ObjectId, expected_xdir: i32) {
    let actor = test_object(engine, actor_id);
    assert_values! {
        actor.fixed_velocity.x => C4Fixed::from_raw(expected_xdir);
        actor.state.velocity.x => 0;
        actor.state.action.name => "Turn";
        actor.state.direction => Direction::Right;
        actor.state.local_vars.get("turn_starts") => Some(&Value::Int(1));
        actor.state.local_vars.get("turn_start_dir") => Some(&Value::Int(0)), "TurnAction Start observes the old direction";
    }
}

#[test]
fn pull_faces_from_a_positive_subpixel_xdir() {
    // The zero-physical compatibility path still applies DFA_PULL's raw
    // C4Fixed SetDir semantics instead of reading the rounded velocity mirror
    // (C4Object.cpp:5186-5194).
    let (mut engine, puller_id) = subpixel_pull_direction_case(false);
    let puller_idx = engine.test_object_index(puller_id);

    let _ = engine.apply_physics_at_index(puller_idx).test_value();

    assert_subpixel_actor_turned(&engine, puller_id, 1);
}

#[test]
fn physical_pull_runs_turn_action_for_a_positive_subpixel_xdir() {
    // Native DFA_PULL assigns raw +366 in this geometry, then SetDir runs
    // TurnAction despite the whole-pixel velocity mirror remaining zero
    // (C4Object.cpp:5186-5194).
    let (mut engine, puller_id) = subpixel_pull_direction_case(true);
    let puller_idx = engine.test_object_index(puller_id);

    let _ = engine.apply_physics_at_index(puller_idx).test_value();

    assert_subpixel_actor_turned(&engine, puller_id, 366);
}

#[test]
fn pull_latches_phase_advance_before_turn_action_mutates_xdir() {
    // DFA_PULL starts from a zero phase baseline and updates it from raw xdir
    // immediately before SetDir. TurnAction's later xdir write does not change
    // that latched value (C4Object.cpp:5189-5192).
    let (mut engine, puller_id) = subpixel_pull_direction_case(true);
    let puller_idx = engine.test_object_index(puller_id);
    engine.objects[puller_idx]
        .state
        .local_vars
        .insert("turn_sets_xdir".to_string(), Value::Int(1));

    let _ = engine.test_tick();

    let puller = test_object(&engine, puller_id);
    unit_assert_eq!(puller.fixed_velocity.x => itofix(1));
    unit_assert_eq!(puller.state.action.name => "Turn");
    unit_assert_eq!(puller.state.action.phase => 0);
}

fn fight_failure_engine() -> Engine {
    let mut fighter = test_definition("L73F", "Fighter", "#strict");
    fighter.set_category(CATEGORY_OBJECT);
    fighter.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
    fighter.set_shape_vertices(square_vertices(8));
    fighter.set_physical(physical! {
            walk: 35_000
    });
    set_actions!(
        &mut fighter, Some("Idle");
        "Idle" => ActionSpec::default(),
        "Walk" => ActionSpec::default().with_procedure("WALK"),
        "Fight" => action_spec!(default, with_procedure: "FIGHT", with_directions: 2, with_flip_dir: 1),
    );

    let mut opponent = test_definition("L73O", "Opponent", "#strict");
    opponent.set_category(CATEGORY_OBJECT);
    opponent.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
    opponent.set_shape_vertices(square_vertices(8));
    set_actions!(
        &mut opponent, Some("Fight");
        "Fight" => ActionSpec::default().with_procedure("FIGHT"),
    );

    let mut passive = test_definition("L73N", "Passive", "#strict");
    passive.set_category(CATEGORY_OBJECT);
    passive.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
    set_actions!(
        &mut passive, Some("Idle");
        "Idle" => ActionSpec::default(),
    );

    let container = test_definition("L73D", "Closed container", "#strict");

    let mut engine = Engine::with_seed(73);
    engine.set_physics(PhysicsSettings::new(0, 20, -20));
    engine.register_test_definition(fighter);
    engine.register_test_definition(opponent);
    engine.register_test_definition(passive);
    engine.register_test_definition(container);
    engine
}

fn spawn_fighter(
    engine: &mut Engine,
    target: Option<ObjectId>,
    container: Option<ObjectId>,
) -> ObjectId {
    let mut fight = ActionState::new("Fight");
    fight.target = target;
    let mut config = SpawnConfig::new("L73F")
        .with_category(CATEGORY_OBJECT)
        .with_position(Vector2::ZERO)
        .with_action(fight)
        .with_command_direction(CommandDirection::Right)
        .with_fixed_velocity(FixedVec2::new(fixed100(125), fixed100(-75)))
        .with_mobile(true);
    if let Some(container) = container {
        config = config.with_container(container);
    }
    let fighter = engine.spawn_test_object(config);
    let index = engine.test_object_index(fighter);
    engine.objects[index]
        .commands
        .push_back(CommandRequest::new(CommandId::MoveTo).with_tx(Some(20)))
        .test_value();
    fighter
}

fn assert_l073_fighter_stands(engine: &Engine, fighter: ObjectId, label: &str) {
    let object = test_object(engine, fighter);
    assert_values! {
        object.state.action.name => "Walk", "{label}";
        object.state.command_direction => CommandDirection::Stop, "{label}";
        object.fixed_velocity => FixedVec2::ZERO, "{label}";
        object.state.velocity => Vector2::ZERO, "{label}";
        object.commands.command_names() => vec!["MoveTo".to_string()], "FIGHT failure must not add PULL's delayed Wait: {label}";
        object.state.t_attach & CNAT_BOTTOM => 0, "a native Fight early return must not latch CNAT_Bottom: {label}";
        object.frame_t_attach => object.state.t_attach, "{label}";
    }
}

#[test]
fn fight_without_target_stands_in_walk_without_wait() {
    let mut engine = fight_failure_engine();
    let fighter = spawn_fighter(&mut engine, None, None);
    let index = engine.test_object_index(fighter);

    unit_assert!(engine.apply_physics_at_index(index).expect("targetless Fight resolves"));

    assert_l073_fighter_stands(&engine, fighter, "no target");
}

#[test]
fn fight_target_door_and_range_failures_stand_without_wait() {
    #[derive(Clone, Copy)]
    enum Failure {
        TargetNotFighting,
        FighterBehindClosedDoor,
        TargetBehindClosedDoor,
        OutOfRange,
    }

    for (label, failure) in [
        ("target not fighting", Failure::TargetNotFighting),
        (
            "fighter behind closed door",
            Failure::FighterBehindClosedDoor,
        ),
        ("target behind closed door", Failure::TargetBehindClosedDoor),
        ("fight target out of range", Failure::OutOfRange),
    ] {
        let mut engine = fight_failure_engine();
        let closed_container = matches!(
            failure,
            Failure::FighterBehindClosedDoor | Failure::TargetBehindClosedDoor
        )
        .then(|| {
            spawn_test!(engine, "L73D", with_position: Vector2::ZERO, with_entrance_status: false)
        });
        let target_definition = if matches!(failure, Failure::TargetNotFighting) {
            "L73N"
        } else {
            "L73O"
        };
        let target_position = if matches!(failure, Failure::OutOfRange) {
            Vector2::new(40, 0)
        } else {
            Vector2::new(10, 0)
        };
        let mut target_config = SpawnConfig::new(target_definition)
            .with_category(CATEGORY_OBJECT)
            .with_position(target_position);
        if matches!(failure, Failure::TargetBehindClosedDoor) {
            target_config = target_config.with_container(closed_container.test_value());
        }
        if target_definition == "L73O" {
            target_config = target_config.with_action(ActionState::new("Fight"));
        }
        let target = engine.spawn_test_object(target_config);
        let fighter_container = if matches!(failure, Failure::FighterBehindClosedDoor) {
            closed_container
        } else {
            None
        };
        let fighter = spawn_fighter(&mut engine, Some(target), fighter_container);
        let index = engine.test_object_index(fighter);

        let _ = engine
            .apply_physics_at_index(index)
            .unwrap_or_else(|error| panic!("{label}: Fight failed: {error}"));

        assert_l073_fighter_stands(&engine, fighter, label);
    }
}

#[test]
fn fight_continues_through_open_container_mismatches() {
    #[derive(Clone, Copy)]
    enum ContainerCase {
        FighterInside,
        TargetInside,
        BothInsideDifferent,
        BothInsideSameClosed,
    }

    for (label, case) in [
        (
            "fighter inside an open container",
            ContainerCase::FighterInside,
        ),
        (
            "target inside an open container",
            ContainerCase::TargetInside,
        ),
        (
            "fighters inside different open containers",
            ContainerCase::BothInsideDifferent,
        ),
        (
            "fighters inside the same closed container",
            ContainerCase::BothInsideSameClosed,
        ),
    ] {
        let mut engine = fight_failure_engine();
        let spawn_container = |engine: &mut Engine, entrance_status| spawn_test!(engine, "L73D", with_position: Vector2::ZERO, with_entrance_status: entrance_status);
        let (fighter_container, target_container) = match case {
            ContainerCase::FighterInside => (Some(spawn_container(&mut engine, true)), None),
            ContainerCase::TargetInside => (None, Some(spawn_container(&mut engine, true))),
            ContainerCase::BothInsideDifferent => (
                Some(spawn_container(&mut engine, true)),
                Some(spawn_container(&mut engine, true)),
            ),
            ContainerCase::BothInsideSameClosed => {
                let container = spawn_container(&mut engine, false);
                (Some(container), Some(container))
            }
        };

        let mut target_config = SpawnConfig::new("L73O")
            .with_category(CATEGORY_OBJECT)
            .with_position(Vector2::new(10, 0))
            .with_action(ActionState::new("Fight"));
        if let Some(container) = target_container {
            target_config = target_config.with_container(container);
        }
        let target = engine.spawn_test_object(target_config);
        let fighter = spawn_fighter(&mut engine, Some(target), fighter_container);
        let index = engine.test_object_index(fighter);

        unit_assert!(
            !engine
                .apply_physics_at_index(index)
                .unwrap_or_else(|error| panic!("{label}: Fight failed: {error}")),
            "a continuing Fight must reach the normal ExecAction phase tail: {label}"
        );

        let fighter = &engine.objects[index];
        unit_assert_eq!(fighter.state.action.name => "Fight", "{label}");
    }
}

#[test]
fn fight_procedure_retains_inactive_action_target_like_cpp() {
    let mut engine = fight_failure_engine();
    let target = spawn_test!(engine, "L73O", with_category: CATEGORY_OBJECT, with_position: Vector2::new(10, 0), with_action: ActionState::new("Fight"));
    let target_index = engine.test_object_index(target);
    engine.objects[target_index].state.status = ObjectStatus::Inactive;
    let fighter = spawn_fighter(&mut engine, Some(target), None);
    let fighter_index = engine.test_object_index(fighter);

    unit_assert!(!engine.apply_physics_at_index(fighter_index).expect("Fight with inactive target executes"), "the retained fight reaches the ordinary phase tail");
    unit_assert_eq!(engine.objects[fighter_index].state.action.name => "Fight", "C4OS_INACTIVE does not clear Action.Target");
}

fn wide_vertex_fight_pair(separation: i32) -> (Engine, ObjectId, ObjectId) {
    let mut engine = fight_failure_engine();
    // Deliberately disagree with the 16px shape rect. DFA_FIGHT uses the
    // live Shape.Wdt for both its approach point and give-up distance,
    // never the span of the contact vertices.
    let wide_vertices = vec![
        ObjectVertex::new(-20, -8),
        ObjectVertex::new(20, -8),
        ObjectVertex::new(20, 8),
        ObjectVertex::new(-20, 8),
    ];
    let target = spawn_test!(engine, "L73O", with_category: CATEGORY_OBJECT, with_position: Vector2::new(separation, 0), with_vertices: wide_vertices.clone(), with_action: ActionState::new("Fight"));
    let fight = targeted_action("Fight", target);
    let fighter = spawn_test!(engine, "L73F", with_category: CATEGORY_OBJECT, with_position: Vector2::ZERO, with_vertices: wide_vertices, with_action: fight);
    (engine, fighter, target)
}

#[test]
fn fight_at_the_same_x_keeps_facing_despite_opposite_raw_velocity() {
    // DFA_FIGHT calls SetDir zero times when the target's whole-pixel x is
    // equal, then approaches using the retained facing. In particular, it
    // must not synthesize a same-direction FlipDir update
    // (C4Object.cpp:5241-5251).
    let (mut engine, fighter, _) = wide_vertex_fight_pair(0);
    let fighter_idx = engine.test_object_index(fighter);
    engine.objects[fighter_idx].state.direction = Direction::Right;
    engine.objects[fighter_idx].state.draw_transform = None;
    engine.objects[fighter_idx]
        .set_fixed_velocity(FixedVec2::new(C4Fixed::from_raw(-61_790), C4Fixed::ZERO));

    let _ = engine.apply_physics_at_index(fighter_idx).test_value();

    let fighter = &engine.objects[fighter_idx];
    unit_assert_eq!(fighter.fixed_velocity.x => C4Fixed::from_raw(-40_000));
    unit_assert_eq!(fighter.state.direction => Direction::Right);
    unit_assert_eq!(fighter.state.draw_transform => None, "SetDir was not called");
}

#[test]
fn fight_faces_its_target_instead_of_its_subpixel_xdir() {
    // DFA_FIGHT chooses direction from the target's whole-pixel position,
    // then approaches using that facing; xdir does not choose the direction
    // (C4Object.cpp:5241-5251).
    let (mut engine, fighter, _) = wide_vertex_fight_pair(-10);
    let fighter_idx = engine.test_object_index(fighter);
    engine.objects[fighter_idx].state.direction = Direction::Right;
    engine.objects[fighter_idx]
        .set_fixed_velocity(FixedVec2::new(C4Fixed::from_raw(61_790), C4Fixed::ZERO));

    let _ = engine.apply_physics_at_index(fighter_idx).test_value();

    let fighter = &engine.objects[fighter_idx];
    unit_assert_eq!(fighter.fixed_velocity.x => C4Fixed::from_raw(40_000));
    unit_assert_eq!(fighter.state.velocity.x => 1);
    unit_assert_eq!(fighter.state.direction => Direction::Left);
}

#[test]
fn fight_approach_uses_target_shape_rect_width() {
    let (mut engine, fighter, target) = wide_vertex_fight_pair(10);
    let fighter_idx = engine.test_object_index(fighter);
    let target_idx = engine.test_object_index(target);
    engine.objects[fighter_idx].state.shape_override = Some(DefinitionRect::new(-6, -8, 12, 16));
    assert_values! {
        engine.objects[fighter_idx].current_shape_rect().expect("fighter has a live shape rect").width => 12,
            "the approach point must not use the fighter width";
        engine.objects[target_idx].current_shape_rect().expect("target has a live shape rect").width => 16;
    }

    let _ = engine.apply_physics_at_index(fighter_idx).test_value();

    let fighter = &engine.objects[fighter_idx];
    assert_values! {
        fighter.state.action.name => "Fight";
        fighter.state.direction => Direction::Right;
        fighter.fixed_velocity.x => C4Fixed::ZERO,
            "target x=10 and Shape.Wdt=16 put the right-facing equilibrium at x=0";
    }
}

#[test]
fn fight_give_up_uses_inclusive_own_shape_rect_width() {
    for (target_width, separation, expected_action) in [
        (16, 16, "Fight"),
        (16, 17, "Walk"),
        (32, 16, "Fight"),
        (32, 17, "Walk"),
    ] {
        let (mut engine, fighter, target) = wide_vertex_fight_pair(separation);
        let fighter_idx = engine.test_object_index(fighter);
        let target_idx = engine.test_object_index(target);
        engine.objects[target_idx].state.shape_override =
            Some(DefinitionRect::new(-target_width / 2, -8, target_width, 16));
        assert_values! {
            engine.objects[fighter_idx].current_shape_rect().expect("fighter has a live shape rect").width => 16;
            engine.objects[target_idx].current_shape_rect().expect("target has a live shape rect").width => target_width;
        }

        let _ = engine
            .apply_physics_at_index(fighter_idx)
            .unwrap_or_else(|error| panic!("Fight separation {separation} failed: {error}"));

        let fighter = &engine.objects[fighter_idx];
        unit_assert_eq!(
                fighter.state.action.name => expected_action,
                "own Shape.Wdt=16 keeps distance 16 and gives up at 17, independent of target width {target_width}"
            );
        if separation == 17 {
            unit_assert_eq!(fighter.state.command_direction => CommandDirection::Stop);
            unit_assert_eq!(fighter.fixed_velocity => FixedVec2::ZERO);
            unit_assert_eq!(fighter.state.velocity => Vector2::ZERO);
        }
    }
}

fn attach_actor_definition(id: &str, script: &str, abort_call: Option<&str>) -> Definition {
    let mut definition = test_definition(id, id, script);
    definition.set_category(CATEGORY_OBJECT);
    definition.set_c4_callback_convention(true);
    definition.set_shape_vertices(vec![ObjectVertex::new(0, 0)]);
    let mut attach = ActionSpec::default().with_procedure("ATTACH");
    if let Some(abort_call) = abort_call {
        attach = attach.with_abort_call(abort_call);
    }
    set_actions!(
        &mut definition, Some("Idle");
        "Idle" => ActionSpec::default(),
        "Attach" => attach,
        "Marker" => ActionSpec::default(),
    );
    definition
}

fn point_definition(id: &str, script: &str) -> Definition {
    let mut definition = test_definition(id, id, script);
    definition.set_category(CATEGORY_OBJECT);
    definition.set_c4_callback_convention(true);
    definition.set_shape_vertices(vec![ObjectVertex::new(0, 0)]);
    definition
}

#[test]
fn attach_lost_target_sets_idle_before_lost_callback() {
    let script = r#"#strict 2
local callback_order, abort_action, lost_action;

protected func AttachAbort(int old_phase)
{
    callback_order = callback_order * 10 + 1;
    abort_action = GetAction();
    return 1;
}

protected func AttachTargetLost()
{
    callback_order = callback_order * 10 + 2;
    lost_action = GetAction();
    SetAction("Marker");
    return 1;
}
"#;
    let (mut engine, actor) = definition_fixture_case!(75, attach_actor_definition("L75A", script, Some("AttachAbort")), "L75A", with_action: ActionState::new("Attach"), with_loaded: true);
    let index = engine.test_object_index(actor);

    unit_assert!(engine.apply_physics_at_index(index).expect("lost Attach target resolves"));

    let object = test_object(&engine, actor);
    assert_locals!(object;
        "callback_order" => Some(&Value::Int(12)),
            "SetAction(ActIdle)'s Attach AbortCall precedes AttachTargetLost";
        "abort_action" => Some(&Value::String("Idle".to_string().into()));
        "lost_action" => Some(&Value::String("Idle".to_string().into()));
    );
    unit_assert_eq!(object.state.action.name => "Marker", "AttachTargetLost runs after the Idle transition and may replace it");
}

#[test]
fn attach_incomplete_target_respects_incomplete_activity() {
    let actor_script = r#"#strict 2
local lost_calls;
protected func AttachTargetLost() { lost_calls += 1; return 1; }
"#;
    let mut blocked = point_definition("L75N", "#strict 2");
    blocked.set_incomplete_activity(false);
    let mut allowed = point_definition("L75Y", "#strict 2");
    allowed.set_incomplete_activity(true);

    let mut engine = action_definitions_engine!(75;
        attach_actor_definition("L75I", actor_script, None),
        blocked,
        allowed,
    );

    for (target_definition, permits_attach, offset) in [("L75N", false, 0), ("L75Y", true, 20)] {
        let target_position = Vector2::new(50 + offset, 60);
        let target = spawn_test!(engine, target_definition, with_position: target_position, with_construction: FULL_CON / 2, with_loaded: true);
        let actor_position = Vector2::new(5 + offset, 6);
        let attach = targeted_action("Attach", target);
        let actor = spawn_test!(engine, "L75I", with_position: actor_position, with_action: attach, with_loaded: true);
        let index = engine.test_object_index(actor);

        let _ = engine.apply_physics_at_index(index).test_value();

        let object = test_object(&engine, actor);
        assert_values! {
            object.state.local_vars.get("lost_calls") => None,
                "an extant incomplete target is not a lost target";
        }
        if permits_attach {
            unit_assert_eq!(object.state.action.name => "Attach");
            unit_assert_eq!(object.state.position => target_position);
        } else {
            unit_assert_eq!(object.state.action.name => "Idle");
            unit_assert_eq!(object.state.position => actor_position);
            assert_values! {
                object.state.action.target => Some(target),
                    "SetAction(ActIdle) preserves an unsupplied target";
            }
        }
    }
}

#[test]
fn attach_forced_enter_callbacks_recheck_cleared_target() {
    let actor_script = r#"#strict 2
local callback_order, lost_action;

public func Mark(int step)
{
    callback_order = callback_order * 10 + step;
    return 1;
}

protected func RejectEntrance(object container)
{
    Mark(1);
    return 0;
}

protected func Entrance(object container)
{
    Mark(3);
    SetActionTargets();
    return 1;
}

protected func AttachTargetLost()
{
    Mark(4);
    lost_action = GetAction();
    return 1;
}
"#;
    let container_script = r#"#strict 2
protected func Collection2(object item) { item->Mark(2); return 1; }
"#;
    let mut engine = action_definitions_engine!(75;
        attach_actor_definition("L75E", actor_script, None),
        point_definition("L75C", container_script),
        point_definition("L75T", "#strict 2"),
    );

    let container = engine.spawn_test_object(SpawnConfig::new("L75C"));
    let target_position = Vector2::new(80, 90);
    let target = spawn_test!(engine, "L75T", with_position: target_position, with_container: container, with_loaded: true);
    let actor_position = Vector2::new(5, 6);
    let attach = targeted_action("Attach", target);
    let actor = spawn_test!(engine, "L75E", with_position: actor_position, with_action: attach, with_loaded: true);
    let index = engine.test_object_index(actor);

    let _ = engine.apply_physics_at_index(index).test_value();

    let object = test_object(&engine, actor);
    unit_assert_eq!(object.state.container => Some(container));
    assert_locals!(object;
        "callback_order" => Some(&Value::Int(1234)),
            "RejectEntrance -> Collection2 -> Entrance -> AttachTargetLost";
        "lost_action" => Some(&Value::String("Idle".to_string().into()));
    );
    unit_assert_eq!(object.state.action.name => "Idle");
    unit_assert_eq!(object.state.action.target => None);
    assert_values! {
        object.state.position => Vector2::ZERO,
            "Enter copies the container motion, and clearing the target prevents a later stale force-position";
    }
}

#[test]
fn attach_forced_exit_runs_ejection_and_departure() {
    let actor_script = r#"#strict 2
local callback_order;
public func Mark(int step) { callback_order = callback_order * 10 + step; return 1; }
protected func Departure(object container) { Mark(2); return 1; }
"#;
    let container_script = r#"#strict 2
protected func Ejection(object item) { item->Mark(1); return 1; }
"#;
    let mut engine = action_definitions_engine!(75;
        attach_actor_definition("L75X", actor_script, None),
        point_definition("L75O", container_script),
        point_definition("L75U", "#strict 2"),
    );

    let old_container = engine.spawn_test_object(SpawnConfig::new("L75O"));
    let target_position = Vector2::new(70, 80);
    let target = spawn_test!(engine, "L75U", with_position: target_position, with_loaded: true);
    let attach = targeted_action("Attach", target);
    let actor = spawn_test!(engine, "L75X", with_position: Vector2::new(7, 8), with_rotation: 37, with_fixed_rotation: itofix(37), with_rotation_velocity: itofix(4), with_container: old_container, with_action: attach, with_loaded: true);
    let index = engine.test_object_index(actor);

    let _ = engine.apply_physics_at_index(index).test_value();

    let object = test_object(&engine, actor);
    assert_values! {
        object.state.container => None;
        object.state.local_vars.get("callback_order") => Some(&Value::Int(12)), "Ejection precedes Departure";
        object.state.action.name => "Attach";
        object.state.action.target => Some(target);
        object.state.position => target_position;
        object.state.rotation => 37;
        object.fixed_rotation => itofix(37);
        object.rotation_velocity => C4Fixed::ZERO;
        object.fixed_velocity => FixedVec2::ZERO;
    }
}

#[test]
fn fight_procedure_moves_toward_target() {
    let script = NOOP_DEFINITION_SCRIPT;

    let mut fighter_definition = test_definition("Fighter", "Fighter", script);
    fighter_definition.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
    set_actions!(
        &mut fighter_definition, Some("Idle");
        "Idle" => ActionSpec::for_procedure("walk"),
        "Fight" => ActionSpec::for_procedure("fight").with_directions(2),
    );
    // DFA_FIGHT approaches with the Walk physical (C4Object.cpp:5225-5228),
    // not the movement profile. 35000 is the stock Clonk DefCore value.
    fighter_definition.set_physical(physical! {
            walk: 35_000
    });

    let mut opponent_definition = test_definition("Opponent", "Opponent", script);
    opponent_definition.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
    set_test_actions(
        &mut opponent_definition,
        Some("Idle"),
        procedure_actions([("Idle", "walk"), ("Fight", "fight")]),
    );

    let mut engine = action_definitions_engine!(33; fighter_definition, opponent_definition);
    engine.set_physics(action_horizontal_physics());

    let vertices = square_vertices(8);

    let opponent_id = spawn_test!(engine, "Opponent", with_category: CATEGORY_OBJECT, with_position: Vector2::new(12, 0), with_vertices: vertices.clone(), with_action: ActionState::new("Fight"));

    let fight_state = targeted_action("Fight", opponent_id);
    let fighter_id = spawn_test!(engine, "Fighter", with_category: CATEGORY_OBJECT, with_position: Vector2::new(0, 0), with_vertices: vertices.clone(), with_action: fight_state);
    let fighter_idx = engine.test_object_index(fighter_id);
    engine.objects[fighter_idx]
        .set_fixed_velocity(FixedVec2::new(C4Fixed::from_raw(98304), C4Fixed::ZERO));
    // dir writes mobilize (FnSetXDir/FnSetYDir, C4Script.cpp:705,732)
    engine.objects[fighter_idx].state.mobile = true;

    engine
        .apply_object_update(
            opponent_id,
            ObjectUpdate::new()
                .with_action_update(ActionUpdate::default().with_target(Some(fighter_id))),
        )
        .test_value();

    let fighter = tick_test_object(&mut engine, fighter_id);
    unit_assert_eq!(fighter.action.name => "Fight");
    unit_assert!(fighter.velocity.x > 0, "fighter should advance towards the opponent");
    unit_assert_eq!(fighter.direction => Direction::Right);
    unit_assert_eq!(fighter.velocity.y => 0);
    unit_assert!(fighter.position.x > 0, "fighter should have moved horizontally");
    let fighter_idx = engine.test_object_index(fighter_id);
    // C4Object.cpp:5221-5228: facing Right, stand-beside target_x at
    // 12 - 16/2 - 2 = 2; lLimit = ValByPhysical(95, 35000)
    // = itofix(35000*19, 2000000) = raw 21790; Towards steps the initial
    // raw 98304 down by one lLimit: 98304 - 21790 = 76514.
    unit_assert_eq!(engine.objects[fighter_idx].fixed_velocity.x.val() => 76514);
}

#[test]
fn fight_procedure_stands_when_target_not_fighting() {
    let script = NOOP_DEFINITION_SCRIPT;

    let mut fighter_definition = action_definition(
        "Fighter",
        "Fighter",
        script,
        Some("Idle"),
        procedure_actions([("Idle", "walk"), ("Walk", "walk"), ("Fight", "fight")]),
    );
    fighter_definition
        .set_movement_profile(movement_profile!(with_walk_speed: 6, with_walk_acceleration: 3));

    let passive_definition = procedure_definition("Passive", "Passive", script, "Idle", "walk");

    let mut engine = action_definitions_engine!(41; fighter_definition, passive_definition);

    let vertices = square_vertices(8);

    let passive_id = spawn_test!(engine, "Passive", with_position: Vector2::new(10, 0), with_vertices: vertices.clone(), with_action: ActionState::new("Idle"));

    let fight_state = targeted_action("Fight", passive_id);
    let fighter_id = spawn_test!(engine, "Fighter", with_position: Vector2::new(0, 0), with_vertices: vertices, with_action: fight_state);

    let fighter = tick_test_object(&mut engine, fighter_id);
    unit_assert_eq!(fighter.action.name => "Walk");
    unit_assert_eq!(fighter.velocity => Vector2::ZERO);
}

#[test]
fn fight_procedure_trains_fight_physical_on_tick5() {
    let script = NOOP_DEFINITION_SCRIPT;

    let mut fighter_definition = test_definition("Fighter", "Fighter", script);
    fighter_definition.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
    set_test_actions(
        &mut fighter_definition,
        Some("Fight"),
        procedure_actions([("Fight", "fight")]),
    );
    fighter_definition.set_physical(physical! {
            walk: 35_000,
            fight: 20_000
    });

    let mut opponent_definition = test_definition("Opponent", "Opponent", script);
    opponent_definition.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
    set_test_actions(
        &mut opponent_definition,
        Some("Fight"),
        procedure_actions([("Fight", "fight")]),
    );

    let mut engine = action_definitions_engine!(33; fighter_definition, opponent_definition);
    engine.set_physics(PhysicsSettings::new(0, 20, -20));

    let vertices = square_vertices(8);

    let opponent_id = spawn_test!(engine, "Opponent", with_position: Vector2::new(12, 0), with_vertices: vertices.clone(), with_action: ActionState::new("Fight"));
    let fight_state = targeted_action("Fight", opponent_id);
    let fighter_id = spawn_test!(engine, "Fighter", with_position: Vector2::new(0, 0), with_vertices: vertices, with_action: fight_state);
    let fighter_idx = engine.test_object_index(fighter_id);
    engine.objects[fighter_idx].state.temporary_physical = Some(physical! {
            walk: 35_000,
            fight: 20_000
    });
    engine
        .apply_object_update(
            opponent_id,
            ObjectUpdate::new()
                .with_action_update(ActionUpdate::default().with_target(Some(fighter_id))),
        )
        .test_value();

    // C4Object.cpp:5214-5216: `if (!Tick5) TrainPhysical(Fight, 1,
    // C4MaxPhysical)` — the gate fires on frames divisible by 5 only;
    // temporary physicals train whenever they exist (C4Object.cpp:2136-2146).
    for _ in 0..4 {
        engine.tick_without_snapshot().test_value();
    }
    assert_values! {
        engine.objects[fighter_idx].state.temporary_physical.expect("temporary physicals remain installed").fight => 20_000,
            "no training before the first Tick5 frame";
    }

    engine.tick_without_snapshot().test_value();
    let trained = engine.objects[fighter_idx]
        .state
        .temporary_physical
        .test_value();
    unit_assert_eq!(trained.fight => 20_001);
    unit_assert_eq!(trained.walk => 35_000, "other physicals copied untouched");
}

#[test]
fn fight_tick35_awards_experience_and_applies_one_native_promotion() {
    let mut fighter_definition = test_definition("CREW", "Crew", "");
    fighter_definition.set_crew_member(true);
    fighter_definition.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
    fighter_definition.set_line_connect(LINE_CONNECT_ENERGY_HOLDER);
    set_actions!(
        &mut fighter_definition, Some("Fight");
        "Fight" => ActionSpec::default().with_procedure("fight"),
    );
    fighter_definition.set_shape_vertices(square_vertices(8));

    let mut opponent_definition = test_definition("OPPN", "Opponent", "");
    opponent_definition.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
    set_actions!(
        &mut opponent_definition, Some("Fight");
        "Fight" => ActionSpec::default().with_procedure("fight"),
    );
    opponent_definition.set_shape_vertices(square_vertices(8));

    let mut engine = action_definitions_engine!(37; fighter_definition, opponent_definition);
    engine.set_physics(PhysicsSettings::new(0, 20, -20));
    join_test_crew_player(
        &mut engine,
        "Fighter owner",
        "CREW",
        1,
        vec![player_file::CrewInfo {
            id: "CREW".to_string(),
            name: "Rookie".to_string(),
            experience: 998,
            ..Default::default()
        }],
    );
    let fighter_id = engine.player(0).test_value().crew()[0];
    let fighter_index = engine.test_object_index(fighter_id);
    let position = engine.objects[fighter_index].state.position;

    let opponent_action = targeted_action("Fight", fighter_id);
    let opponent_id =
        spawn_test!(engine, "OPPN", with_position: position, with_action: opponent_action);
    engine
        .apply_object_update(
            fighter_id,
            ObjectUpdate::new().with_action_update(
                ActionUpdate::default()
                    .with_name("Fight")
                    .with_target(Some(opponent_id)),
            ),
        )
        .test_value();

    let raw_physical = physical! {
            energy: 10_000,
            breath: 12_345,
            walk: 0,
            jump: 23_456,
            can_fly: 7,
            corrosion_resist: 8,
            breathe_water: 9
    };
    engine.objects[fighter_index].state.info_physical = Some(raw_physical);
    engine.objects[fighter_index].state.energy = 12_000;
    engine.pending_audio.clear();

    for expected_frame in 1..35 {
        let snapshot = engine.test_tick();
        unit_assert_eq!(snapshot.frame => expected_frame);
        unit_assert!(snapshot.audio.iter().all(|command| !matches!(
            command,
            AudioCommand::PlaySound { name, target, .. }
                if name == "Trumpet" && *target == Some(fighter_id)
        )));
        assert_values! {
            engine.crew_object_info(fighter_id).expect("fighter keeps info").experience => 998,
                "non-Tick35 fight frames do not award experience";
        }
    }

    let promotion_frame = engine.test_tick();
    unit_assert_eq!(promotion_frame.frame => 35);
    let info = engine.crew_object_info(fighter_id).test_value();
    unit_assert_eq!((info.experience, info.rank) => (1_000, 1));
    unit_assert_eq!(info.rank_name => "Ensign");
    let state = engine.capture_state();
    let link = state.crew_info_links[&fighter_id];
    let roster = &state.crew_info_rosters[&link.player_id][link.roster_index];
    unit_assert_eq!((roster.experience, roster.rank) => (1_000, 1));
    unit_assert_eq!(roster.rank_name => "Ensign");

    let fighter = promotion_frame.object(fighter_id).test_value();
    assert_values! {
        fighter.energy => 12_000, "promotion does not heal live Energy";
    }
    let promoted = fighter.info_physical.test_value();
    unit_assert_eq!(promoted.energy => 55_000);
    assert_values! {
        (
            promoted.can_dig,
            promoted.can_chop,
            promoted.can_construct,
            promoted.can_scale,
            promoted.can_hangle,
        ) => (1, 1, 1, 1, 1);
    }
    unit_assert_eq!(promoted.breath => raw_physical.breath);
    unit_assert_eq!(promoted.walk => raw_physical.walk);
    unit_assert_eq!(promoted.jump => raw_physical.jump);
    unit_assert_eq!(promoted.can_fly => raw_physical.can_fly);
    unit_assert_eq!(promoted.corrosion_resist => raw_physical.corrosion_resist);
    unit_assert_eq!(promoted.breathe_water => raw_physical.breathe_water);

    let promotion_messages = promotion_frame
        .hud
        .messages
        .iter()
        .filter(|message| message.target == Some(fighter_id))
        .collect::<Vec<_>>();
    unit_assert_eq!(promotion_messages.len() => 1);
    unit_assert_eq!(promotion_messages[0].lines => ["Rookie is promoted".to_string(), "to Ensign!".to_string()]);
    unit_assert_eq!(
        promotion_frame
            .audio
            .iter()
            .filter(|command| matches!(
                command,
                AudioCommand::PlaySound {
                    name,
                    target,
                    volume: 100,
                    looped: false,
                    ..
                } if name == "Trumpet" && *target == Some(fighter_id)
            ))
            .count() =>
        1,
        "native promotion emits one Trumpet"
    );

    for expected_frame in 36..70 {
        let snapshot = engine.test_tick();
        unit_assert_eq!(snapshot.frame => expected_frame);
        assert_values! {
            engine.crew_object_info(fighter_id).expect("fighter keeps info").experience => 1_000,
                "experience stays fixed between Tick35 boundaries";
        }
    }

    let second_award = engine.test_tick();
    unit_assert_eq!(second_award.frame => 70);
    let info = engine.crew_object_info(fighter_id).test_value();
    unit_assert_eq!((info.experience, info.rank) => (1_002, 1));
    unit_assert!(second_award.audio.iter().all(|command| !matches!(
        command,
        AudioCommand::PlaySound { name, target, .. }
            if name == "Trumpet" && *target == Some(fighter_id)
    )));
}

/// `C4Object::SetAction` stops the outgoing action's ActMap sound and
/// starts the incoming one as an object-attached LOOP at volume 100
/// (C4Object.cpp:4149-4152, 4186-4190 — `StartSoundEffect(..., +1, 100,
/// this)`), both gated on the numeric action slot actually changing.
/// EkeReloaded's Uzi is the shape under test: `Shoot` declares
/// `Sound=UZ_Shoot` with `NextAction=Shoot`, so the burst must be one
/// continuous loop rather than silence or a per-frame retrigger.
#[test]
fn actmap_sound_loops_while_its_action_slot_stays_selected() {
    let uzi_sound = |snapshot: &SimulationSnapshot, id| {
        snapshot
                .audio
                .iter()
                .filter(|command| {
                    matches!(
                        command,
                        AudioCommand::PlaySound { name, target, .. } | AudioCommand::StopSound { name, target }
                            if name == "UZ_Shoot" && *target == Some(id)
                    )
                })
                .cloned()
                .collect::<Vec<_>>()
    };

    let definition = action_definition_fixture!(
        "Uzi",
        "Uzi",
        "func Initialize() { }",
        Some("Idle");
        "Idle" => ActionSpec::default(),
        "Shoot" => action_spec!(default, with_length: 1, with_delay: 1, with_next: "Shoot", with_sound: "UZ_Shoot"),
        "Burst" => action_spec!(default, with_length: 2, with_delay: 1, with_next: "Idle", with_sound: "UZ_Shoot"),
    );

    let (mut engine, shooter) = definition_fixture_case!(5, definition, "Uzi", with_category: CATEGORY_OBJECT, with_action: ActionState::new("Shoot"));

    // Entering the slot starts one attached loop at volume 100.
    let started = engine.test_tick();
    unit_assert!(
        matches!(
            uzi_sound(&started, shooter).as_slice(),
            [AudioCommand::PlaySound {
                volume: 100,
                looped: true,
                // StartSoundEffect calls NewInstance unconditionally
                // (C4SoundSystem.cpp:54-58); only FnSound gates on
                // IsSoundPlaying (C4Script.cpp:2317-2319).
                multiple: true,
                ..
            }]
        ),
        "entering Shoot starts exactly one looped attached sound, got {:?}",
        uzi_sound(&started, shooter)
    );

    // NextAction=Shoot re-selects the SAME numeric slot every frame, and
    // C++ gates both the stop and the start on `iAct != iLastAction`, so
    // the loop must keep running untouched.
    for frame in 0..8 {
        let snapshot = engine.test_tick();
        assert_values! {
            snapshot.object(shooter).expect("shooter present").action.name => "Shoot";
        }
        unit_assert!(
            uzi_sound(&snapshot, shooter).is_empty(),
            "frame {frame}: a same-slot NextAction must not retrigger the loop, got {:?}",
            uzi_sound(&snapshot, shooter)
        );
    }

    // Leaving the slot stops it, and Idle carries no sound of its own.
    let burst = spawn_test!(engine, "Uzi", with_category: CATEGORY_OBJECT, with_action: ActionState::new("Burst"));
    let burst_started = engine.test_tick();
    unit_assert!(
        matches!(
            uzi_sound(&burst_started, burst).as_slice(),
            [AudioCommand::PlaySound { looped: true, .. }]
        ),
        "entering Burst starts its loop, got {:?}",
        uzi_sound(&burst_started, burst)
    );
    let stopped = engine.test_tick();
    unit_assert_eq!(stopped.object(burst).expect("burst present").action.name => "Idle",);
    unit_assert!(
        matches!(
            uzi_sound(&stopped, burst).as_slice(),
            [AudioCommand::StopSound { .. }]
        ),
        "leaving the slot stops the loop exactly once, got {:?}",
        uzi_sound(&stopped, burst)
    );
}

/// `C4Object::SetAction` emits the outgoing ActMap sound stop before the new
/// action's start (C4Object.cpp:4149-4152, 4186-4190). A caller can select B
/// and then A again before the frame closes, so end-of-frame reconciliation
/// must retain that sequence rather than observing only the final A slot.
#[test]
fn actmap_sound_reconciles_an_intra_frame_action_round_trip() {
    let action_sound = |snapshot: &SimulationSnapshot, id| {
        snapshot
            .audio
            .iter()
            .filter(|command| {
                matches!(
                    command,
                    AudioCommand::PlaySound { name, target, .. }
                        | AudioCommand::StopSound { name, target }
                        if *target == Some(id) && matches!(name.as_str(), "A_Sound" | "B_Sound")
                )
            })
            .cloned()
            .collect::<Vec<_>>()
    };

    let definition = action_definition_fixture!(
        "RoundTrip",
        "Round trip",
        "func Initialize() { }",
        Some("Idle");
        "Idle" => ActionSpec::default(),
        "A" => ActionSpec::default().with_length(100).with_sound("A_Sound"),
        "B" => ActionSpec::default().with_length(100).with_sound("B_Sound"),
    );

    let (mut engine, object) = definition_fixture_case!(5, definition, "RoundTrip", with_category: CATEGORY_OBJECT, with_action: ActionState::new("A"));

    let started = engine.test_tick();
    unit_assert!(
        matches!(
            action_sound(&started, object).as_slice(),
            [AudioCommand::PlaySound { name, looped: true, .. }] if name == "A_Sound"
        ),
        "the initial A slot starts its loop"
    );

    engine
        .apply_object_update(object, ObjectUpdate::new().with_action("B"))
        .test_value();
    engine
        .apply_object_update(object, ObjectUpdate::new().with_action("A"))
        .test_value();
    let round_trip = engine.test_tick();
    unit_assert_eq!(
        action_sound(&round_trip, object) =>
        [
            AudioCommand::StopSound {
                name: "A_Sound".to_string(),
                target: Some(object),
            },
            AudioCommand::PlaySound {
                name: "B_Sound".to_string(),
                target: Some(object),
                volume: 100,
                looped: true,
                multiple: true,
                custom_falloff: None,
            },
            AudioCommand::StopSound {
                name: "B_Sound".to_string(),
                target: Some(object),
            },
            AudioCommand::PlaySound {
                name: "A_Sound".to_string(),
                target: Some(object),
                volume: 100,
                looped: true,
                multiple: true,
                custom_falloff: None,
            },
        ],
        "the reconciler preserves every A-to-B-to-A transition in order"
    );
}
