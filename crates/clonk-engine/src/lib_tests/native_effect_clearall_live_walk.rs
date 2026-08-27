use super::*;
use crate::lib_test_support::{spawn_fixture, EngineTestExt};

#[test]
fn native_death_clear_all_observes_a_renamed_lower_effect() {
    // C4Effect::ClearAll recurses to the tail before each Fx*Stop call, but
    // each recursive frame still dispatches through that node's live callback
    // pointers (C4Effect.cpp:407-425). The high-priority Stop renames the
    // lower effect before its frame resumes, so the lower call must use FxNewStop.
    let mut definition = test_definition(
        "NCLR",
        "Native death ClearAll live walk",
        r#"#strict 3
local trace;

public func Arm()
{
    trace = 0;
    AddEffect("Old", this(), 100, 0, this());
    AddEffect("High", this(), 200, 0, this());
    return true;
}

public func ReadTrace() { return trace; }

func FxHighStop(object target, int number, int reason)
{
    trace = trace * 10 + 1;
    ChangeEffect("Old", target, 0, "New", 0);
    return 0;
}

func FxOldStop(object target, int number, int reason)
{
    trace = trace * 10 + 9;
    return 0;
}

func FxNewStop(object target, int number, int reason)
{
    trace = trace * 10 + 2;
    return 0;
}
"#,
    );
    definition.set_c4_callback_convention(true);
    definition.configure_actions(
        Some("Idle".to_owned()),
        HashMap::from([
            ("Idle".to_owned(), ActionSpec::default()),
            ("Dead".to_owned(), ActionSpec::default()),
        ]),
    );

    let mut engine = Engine::with_seed(31);
    engine.register_test_definition(definition);
    let object = spawn_fixture!(
        engine,
        "NCLR",
        with_action: ActionState::new("Idle"),
        with_alive: true
    );
    let index = engine.test_object_index(object);
    assert_eq!(
        engine.call_test_object_function(index, "Arm", Vec::new()),
        Value::Bool(true)
    );

    crate::TestValueExt::test_value(engine.assign_death(index, false));

    assert_eq!(
        engine.call_test_object_function(index, "ReadTrace", Vec::new()),
        Value::Int(12)
    );
}

#[test]
fn native_death_clear_all_threads_spawned_objects_between_stop_callbacks() {
    // C4Effect::ClearAll runs every Stop synchronously in one recursive walk,
    // and C4Game::CreateObject links its object before returning. A lower Stop
    // therefore sees and mutates an object created by the higher Stop
    // (C4Effect.cpp:407-425; C4Game.cpp:1121-1138).
    let mut carrier = test_definition(
        "NCSP",
        "Native death ClearAll spawn visibility",
        r#"#strict 3
local trace;

public func Arm()
{
    trace = 0;
    AddEffect("Low", this(), 100, 0, this());
    AddEffect("Middle", this(), 200, 0, this());
    AddEffect("High", this(), 300, 0, this());
    return true;
}

public func ReadTrace() { return trace; }

func FxHighStop(object target, int number, int reason)
{
    CreateObject(MARK, 0, 0, -1);
    return 0;
}

func FxMiddleStop(object target, int number, int reason)
{
    var spawned = FindObject(MARK);
    if (!spawned) return 0;
    spawned->DoDamage(37);
    return 0;
}

func FxLowStop(object target, int number, int reason)
{
    var spawned = FindObject(MARK);
    if (!spawned) return 0;
    trace = ((spawned->GetDamage() * 100000 + spawned->GetCon() * 10 + spawned->GetAlive()) * 10
        + spawned->GetKiller() + 1) * 10 + InLiquid(spawned) * 2 + Stuck(spawned);
    return 0;
}
"#,
    );
    carrier.set_c4_callback_convention(true);
    carrier.configure_actions(
        Some("Idle".to_owned()),
        HashMap::from([
            ("Idle".to_owned(), ActionSpec::default()),
            ("Dead".to_owned(), ActionSpec::default()),
        ]),
    );

    let mut engine = Engine::with_seed(32);
    let library = crate::TestValueExt::test_value(clonk_resources::MaterialLibrary::parse(
        "[Material Water]\nName=Water\nDensity=30\nInstable=1\n",
    ));
    engine.configure_materials_from_library(&library);
    let water = crate::TestValueExt::test_value(engine.materials.id_of("Water"));
    let mut landscape = Landscape::flat(8, 12);
    landscape.set_liquid_column(1, vec![LiquidSegment::with_material(5, 9, Some(water))]);
    engine.set_landscape(landscape);
    crate::TestValueExt::test_value(engine.register_player(PlayerConfig::new(1, "P1")));
    engine.register_test_definition(carrier);
    let mut marker = test_definition(
        "MARK",
        "Spawn marker",
        "#strict 3\nfunc Construction() { SetPosition(1, 6); SetContactDensity(20); SetAlive(true); SetKiller(1); return 0; }\n",
    );
    marker.set_shape_vertices(vec![ObjectVertex::new(0, 0)]);
    engine.register_test_definition(marker);
    let object = spawn_fixture!(
        engine,
        "NCSP",
        with_action: ActionState::new("Idle"),
        with_alive: true
    );
    let index = engine.test_object_index(object);
    assert_eq!(
        engine.call_test_object_function(index, "Arm", Vec::new()),
        Value::Bool(true)
    );

    crate::TestValueExt::test_value(engine.assign_death(index, false));

    assert_eq!(
        engine.call_test_object_function(index, "ReadTrace", Vec::new()),
        Value::Int(370_100_123)
    );
    let spawned = engine
        .objects
        .iter()
        .find(|object| object.definition_id == "MARK")
        .expect("the higher Stop creates the marker");
    assert_eq!(spawned.state.damage, 37);
    assert!(spawned.state.alive);
}

#[test]
fn native_death_clear_all_preserves_creation_time_master_order() {
    // C4Game::CreateObject links each object immediately using its category
    // at that instant. A later SetCategory marks the first spawn Unsorted but
    // does not move its link before the lower Stop walks Objects.First
    // (C4Game.cpp:1121-1138; C4ObjectList.cpp:134-175).
    let mut carrier = test_definition(
        "NCSO",
        "Native ClearAll spawn order",
        r#"#strict 3
local expected_number, saw_expected_first;

public func Arm()
{
    expected_number = saw_expected_first = 0;
    AddEffect("Low", this(), 100, 0, this());
    AddEffect("High", this(), 200, 0, this());
    return true;
}

public func ReadOrder() { return saw_expected_first; }

func FxHighStop(object target, int number, int reason)
{
    var first = CreateObject(HIOR, 0, 0, -1);
    expected_number = ObjectNumber(first);
    CreateObject(LOOR, 0, 0, -1);
    SetCategory(4, first);
    return 0;
}

func FxLowStop(object target, int number, int reason)
{
    saw_expected_first = ObjectNumber(FindObject()) == expected_number;
    return 0;
}
"#,
    );
    carrier.set_c4_callback_convention(true);
    carrier.configure_actions(
        Some("Idle".to_owned()),
        HashMap::from([
            ("Idle".to_owned(), ActionSpec::default()),
            ("Dead".to_owned(), ActionSpec::default()),
        ]),
    );
    let mut high = test_definition("HIOR", "High category spawn", "");
    high.set_category(CATEGORY_OBJECT);
    let mut low = test_definition("LOOR", "Low category spawn", "");
    low.set_category(CATEGORY_STRUCTURE);

    let mut engine = Engine::with_seed(33);
    engine.register_test_definition(carrier);
    engine.register_test_definition(high);
    engine.register_test_definition(low);
    let object = spawn_fixture!(
        engine,
        "NCSO",
        with_action: ActionState::new("Idle"),
        with_alive: true
    );
    let index = engine.test_object_index(object);
    assert_eq!(
        engine.call_test_object_function(index, "Arm", Vec::new()),
        Value::Bool(true)
    );

    crate::TestValueExt::test_value(engine.assign_death(index, false));

    assert_eq!(
        engine.call_test_object_function(index, "ReadOrder", Vec::new()),
        Value::Bool(true)
    );
}

#[test]
fn native_death_clear_all_preserves_inactive_list_transition_order() {
    // StatusDeactivate removes/adds the exact live links immediately. The
    // second deactivation of `first` therefore moves it behind `second` in
    // InactiveObjects even though both objects' callback-final status is the
    // same (C4Object.cpp:5987-6007; C4ObjectList.cpp:134-175).
    let mut carrier = test_definition(
        "NCIO",
        "Native ClearAll inactive order",
        r#"#strict 3
local first, second;

public func Arm(object a, object b)
{
    first = a;
    second = b;
    AddEffect("High", this(), 100, 0, this());
    return true;
}

func FxHighStop(object target, int number, int reason)
{
    SetObjectStatus(C4OS_INACTIVE, first);
    SetObjectStatus(C4OS_INACTIVE, second);
    SetObjectStatus(C4OS_NORMAL, first);
    SetObjectStatus(C4OS_INACTIVE, first);
    return 0;
}
"#,
    );
    carrier.set_c4_callback_convention(true);
    carrier.configure_actions(
        Some("Idle".to_owned()),
        HashMap::from([
            ("Idle".to_owned(), ActionSpec::default()),
            ("Dead".to_owned(), ActionSpec::default()),
        ]),
    );

    let mut engine = Engine::with_seed(34);
    engine.register_test_definition(carrier);
    engine.register_test_definition(test_definition("INAC", "Inactive marker", ""));
    let first = spawn_fixture!(engine, "INAC");
    let second = spawn_fixture!(engine, "INAC");
    let carrier = spawn_fixture!(
        engine,
        "NCIO",
        with_action: ActionState::new("Idle"),
        with_alive: true
    );
    let index = engine.test_object_index(carrier);
    assert_eq!(
        engine.call_test_object_function(
            index,
            "Arm",
            vec![
                Value::Object(first.as_u64()),
                Value::Object(second.as_u64())
            ],
        ),
        Value::Bool(true)
    );

    crate::TestValueExt::test_value(engine.assign_death(index, false));

    assert_eq!(
        engine
            .execution
            .inactive
            .iter()
            .rev()
            .copied()
            .collect::<Vec<_>>(),
        vec![first, second]
    );
}

#[test]
fn pending_carrier_effect_start_preserves_creation_time_master_order() {
    // The port's undispatched starting EffectState drives the same FxStart
    // callback that native AddEffect runs inline. Each nested CreateObject is
    // linked before the next statement; changing the first child's category
    // afterwards marks it Unsorted without moving it (C4Game.cpp:1121-1138;
    // C4ObjectList.cpp:134-175; C4Object.h:303-311).
    let mut carrier = test_definition(
        "PCFX",
        "Pending carrier effect start",
        r#"#strict 3
func FxSpawnStart()
{
    var first = CreateObject(HIOR, 0, 0, -1);
    CreateObject(LOOR, 0, 0, -1);
    SetCategory(1, first);
    return 0;
}
"#,
    );
    carrier.set_c4_callback_convention(true);
    let mut high = test_definition("HIOR", "High category child", "");
    high.set_category(CATEGORY_OBJECT);
    let mut low = test_definition("LOOR", "Low category child", "");
    low.set_category(CATEGORY_STRUCTURE);

    let mut engine = Engine::with_seed(35);
    engine.register_test_definition(carrier);
    engine.register_test_definition(high);
    engine.register_test_definition(low);
    let carrier_id = ObjectId::new(1);
    spawn_fixture!(
        engine,
        "PCFX",
        with_id: carrier_id,
        add_effect: EffectState::new("Spawn")
            .with_priority(100)
            .with_command_target(Some(carrier_id.as_u64() as i32))
    );

    let children = engine
        .execution
        .exec_list
        .iter()
        .rev()
        .filter_map(|id| {
            let object = &engine.objects[engine.test_object_index(*id)];
            matches!(object.definition_id.as_str(), "HIOR" | "LOOR")
                .then_some(object.definition_id.as_str())
        })
        .collect::<Vec<_>>();
    assert_eq!(children, vec!["HIOR", "LOOR"]);
}

#[test]
fn pending_effect_children_finish_before_the_next_spawn_queue_member() {
    // NewObject completes every nested CreateObject before returning to its
    // caller, so a later queued sibling must be linked only after the effect's
    // exact callback-final object-list snapshot has committed
    // (C4Game.cpp:1085-1142; C4ObjectList.cpp:134-175).
    let mut carrier = test_definition(
        "PQFX",
        "Queued pending effect",
        r#"#strict 3
func FxSpawnStart()
{
    var first = CreateObject(QHIO, 0, 0, -1);
    CreateObject(QLOW, 0, 0, -1);
    SetCategory(1, first);
    return 0;
}
"#,
    );
    carrier.set_c4_callback_convention(true);
    let mut high = test_definition("QHIO", "Queued high child", "");
    high.set_category(CATEGORY_OBJECT);
    let mut low = test_definition("QLOW", "Queued low child", "");
    low.set_category(CATEGORY_STRUCTURE);
    let late = test_definition("LATE", "Unrelated later spawn", "");

    let mut engine = Engine::with_seed(36);
    engine.register_test_definition(carrier);
    engine.register_test_definition(high);
    engine.register_test_definition(low);
    engine.register_test_definition(late);
    let carrier_id = ObjectId::new(1);
    crate::TestValueExt::test_value(engine.process_spawn_queue(vec![
        SpawnConfig::new("PQFX").with_id(carrier_id).add_effect(
            EffectState::new("Spawn")
                .with_priority(100)
                .with_command_target(Some(carrier_id.as_u64() as i32)),
        ),
        SpawnConfig::new("LATE"),
    ]));

    let definitions = engine
        .execution
        .exec_list
        .iter()
        .rev()
        .filter_map(|id| {
            let definition = engine.objects[engine.test_object_index(*id)]
                .definition_id
                .as_str();
            matches!(definition, "QHIO" | "QLOW" | "LATE").then_some(definition)
        })
        .collect::<Vec<_>>();
    let child_order = definitions
        .iter()
        .copied()
        .filter(|definition| matches!(*definition, "QHIO" | "QLOW"))
        .collect::<Vec<_>>();
    assert_eq!(child_order, vec!["QHIO", "QLOW"]);
    assert!(definitions.contains(&"LATE"));
}

#[test]
fn pending_effect_zero_construction_removes_its_unmaterialized_list_link() {
    // CreateConstruction registers the zero-construction object first, then
    // DoCon(0) removes it and NewObject returns nil. Its number remains spent,
    // but no Game.Objects or sector link survives (C4Game.cpp:1085-1128;
    // C4Object.cpp:1513-1517; C4GameObjects.cpp:54-70).
    let mut carrier = test_definition(
        "PZFX",
        "Pending zero-construction effect",
        r#"#strict 3
func FxCancelStart()
{
    CreateConstruction(ZERO, 0, 0, -1, 0);
    return 0;
}
"#,
    );
    carrier.set_c4_callback_convention(true);
    let canceled = test_definition("ZERO", "Canceled zero construction", "");

    let mut engine = Engine::with_seed(37);
    engine.register_test_definition(carrier);
    engine.register_test_definition(canceled);
    let carrier_id = ObjectId::new(1);
    let spawned = engine.spawn_object(
        SpawnConfig::new("PZFX").with_id(carrier_id).add_effect(
            EffectState::new("Cancel")
                .with_priority(100)
                .with_command_target(Some(carrier_id.as_u64() as i32)),
        ),
    );

    assert_eq!(crate::TestValueExt::test_value(spawned), carrier_id);
    assert!(engine
        .objects
        .iter()
        .all(|object| object.definition_id.as_str() != "ZERO"));
}
