use super::*;
use crate::landscape::{
    LandscapeRasterState, RuntimeTexMapLookup, RuntimeTexMapMaterial, RuntimeTexMapState,
};
use crate::lib_test_support::{spawn_fixture, EngineTestExt};

fn section(name: &str, width: u32, base_extinguish_enabled: bool) -> scenario::ScenarioSectionSpec {
    let mut section = vehicle_section(name, vehicle_section_landscape(width, 40));
    section.base_extinguish_enabled = base_extinguish_enabled;
    section
}

fn vehicle_section_landscape(width: u32, height: u32) -> Landscape {
    let mut landscape =
        crate::TestValueExt::test_value(Landscape::new(width, vec![0; width as usize]));
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
    let mut definition = test_definition(id, "Capture mask", "");
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
    crate::TestValueExt::test_value(engine.apply_initial_network_game_data(
        &InitialNetworkGameData {
            current_scenario_section: "Cave".to_string(),
            ..InitialNetworkGameData::default()
        },
    ));
    engine
}

#[test]
fn scenario_section_switch_clears_global_effects_through_stop_callbacks_tail_first() {
    // C4Game::LoadScenarioSection removes the GLOBAL effect list through
    // C4Effect::ClearAll(nil, C4FxCall_RemoveClear), leaving the dead nodes
    // linked until Execute reaps them (C4Game.cpp:4202-4208). ClearAll recurses
    // through pNext before stopping the current node, so callbacks run from
    // highest to lowest priority and receive nil, their number, and reason 3
    // (C4Effect.cpp:407-425; C4Effects.h:49).
    let script = r#"#strict 3
static stop_order, stop_numbers, stop_reasons, stop_targets;

global func ArmSectionClear()
{
    stop_order = stop_numbers = stop_reasons = stop_targets = 0;
    return true;
}

global func FxSectionLowStop(object target, int number, int reason)
{
    stop_order = stop_order * 10 + 1;
    stop_numbers = stop_numbers * 10 + number;
    stop_reasons = stop_reasons * 10 + reason;
    stop_targets = stop_targets * 10 + !!target;
    return false;
}

global func FxSectionHighStop(object target, int number, int reason)
{
    stop_order = stop_order * 10 + 2;
    stop_numbers = stop_numbers * 10 + number;
    stop_reasons = stop_reasons * 10 + reason;
    stop_targets = stop_targets * 10 + !!target;
    return false;
}
"#;

    let mut engine = Engine::with_seed(17);
    engine.configure_scenario_sections(&[section("main", 80, true), section("next", 120, true)]);
    engine.set_landscape(vehicle_section_landscape(80, 40));
    assert_eq!(
        engine.install_global_scripts(&[("System.c4g/SectionEffects.c".into(), script.into())]),
        1
    );
    assert_eq!(
        crate::TestValueExt::test_value(engine.call_engine_global_function("ArmSectionClear", &[]),),
        Value::Bool(true)
    );
    let mut low = EffectState::new("SectionLow").with_priority(100);
    low.number = 1;
    let mut high = EffectState::new("SectionHigh").with_priority(200);
    high.number = 2;
    engine.global_effects = vec![low, high];
    assert_eq!(
        engine
            .global_effects()
            .iter()
            .map(|effect| (effect.name.as_str(), effect.number, effect.priority))
            .collect::<Vec<_>>(),
        vec![("SectionLow", 1, 100), ("SectionHigh", 2, 200)]
    );

    assert!(engine.load_test_section("next", 0, Vec::new()));

    let globals = engine.snapshot().script_globals.named;
    assert_eq!(globals.get("stop_order"), Some(&Value::Int(21)));
    assert_eq!(globals.get("stop_numbers"), Some(&Value::Int(21)));
    assert_eq!(globals.get("stop_reasons"), Some(&Value::Int(33)));
    assert_eq!(globals.get("stop_targets"), Some(&Value::Int(0)));
    assert_eq!(
        engine
            .global_effects()
            .iter()
            .map(|effect| (effect.name.as_str(), effect.number, effect.priority))
            .collect::<Vec<_>>(),
        vec![("SectionLow", 1, 0), ("SectionHigh", 2, 0)],
        "ClearAll marks each original effect dead but does not unlink it"
    );

    crate::TestValueExt::test_value(engine.tick_without_snapshot());
    assert!(engine.global_effects().is_empty());
}

#[test]
fn scenario_section_global_clear_observes_mutation_denial_addition_and_rng() {
    // ClearAll fixes only the recursive successor calls, not callback state:
    // each resumed node is live, a Stop denial restores that same node, and
    // effects added during Stop lie outside the original recursion
    // (C4Effect.cpp:407-425). Fail-safe Exec also commits synchronized Random
    // draws made by those callbacks (C4AulExec.cpp:1318-1342).
    let script = r#"#strict 3
static clear_trace, clear_draw;

global func ArmMutatingSectionClear()
{
    clear_trace = clear_draw = 0;
    return true;
}

global func FxMutatingHighStop(object target, int number, int reason)
{
    clear_trace = clear_trace * 10 + 1;
    clear_draw = Random(113);
    ChangeEffect("MutatingLow", target, 0, "MutatingRenamed", 0);
    AddEffect("MutatingBorn", target, 150, 0);
    return -1;
}

global func FxMutatingLowStop(object target, int number, int reason)
{
    clear_trace = clear_trace * 10 + 9;
    return 0;
}

global func FxMutatingRenamedStop(object target, int number, int reason)
{
    clear_trace = clear_trace * 10 + 2;
    return 0;
}

global func FxMutatingBornStop(object target, int number, int reason)
{
    clear_trace = clear_trace * 10 + 8;
    return 0;
}
"#;

    let mut engine = Engine::with_seed(41);
    engine.configure_scenario_sections(&[section("main", 80, true), section("next", 120, true)]);
    engine.set_landscape(vehicle_section_landscape(80, 40));
    assert_eq!(
        engine.install_global_scripts(&[("System.c4g/MutatingEffects.c".into(), script.into())]),
        1
    );
    assert_eq!(
        crate::TestValueExt::test_value(
            engine.call_engine_global_function("ArmMutatingSectionClear", &[]),
        ),
        Value::Bool(true)
    );
    let mut low = EffectState::new("MutatingLow").with_priority(100);
    low.number = 1;
    let mut high = EffectState::new("MutatingHigh").with_priority(200);
    high.number = 2;
    engine.global_effects = vec![low, high];
    let mut expected_rng = engine.rng.clone();
    let expected_draw = expected_rng.random(113);

    assert!(engine.load_test_section("next", 0, Vec::new()));

    let globals = engine.snapshot().script_globals.named;
    assert_eq!(globals.get("clear_trace"), Some(&Value::Int(12)));
    assert_eq!(globals.get("clear_draw"), Some(&Value::Int(expected_draw)));
    assert_eq!(
        engine
            .global_effects()
            .iter()
            .map(|effect| (effect.name.as_str(), effect.number, effect.priority))
            .collect::<Vec<_>>(),
        vec![
            ("MutatingRenamed", 1, 0),
            ("MutatingHigh", 2, 200),
            ("MutatingBorn", 3, 150),
        ],
        "the denied original and callback-added effect survive the clear"
    );
}

#[test]
fn scenario_section_global_stop_threads_its_spawn_before_target_init_deletes_it() {
    // LoadScenarioSection finishes AssignRemoval/ClearPointers/DeleteObjects
    // before global C4Effect::ClearAll. Stops therefore see no departing
    // active objects and synchronously share a newly created object. The
    // later InitGameSecondPart Objects.Clear(false) deletes that active spawn
    // without callbacks but does not reuse its number (C4Game.cpp:4190-4208,
    // 2642-2713; C4Effect.cpp:407-425; C4GameObjects.cpp:313-331).
    let script = r#"#strict 3
static clear_seen_count, clear_spawn_number, clear_seen_damage;

global func FxSectionHighStop(object target, int number, int reason)
{
    clear_seen_count = ObjectCount();
    clear_spawn_number = ObjectNumber(CreateObject(ITEM, 7, 9, -1));
    return 0;
}

global func FxSectionMiddleStop(object target, int number, int reason)
{
    var spawned = FindObject(ITEM);
    if (spawned) spawned->DoDamage(37);
    return 0;
}

global func FxSectionLowStop(object target, int number, int reason)
{
    var spawned = FindObject(ITEM);
    if (spawned) clear_seen_damage = spawned->GetDamage();
    return 0;
}
"#;

    let mut engine = Engine::with_seed(43);
    let mut item = test_definition(
        "ITEM",
        "Section item",
        r#"#strict 3
static direct_delete_trace;

func Initialize()
{
    AddEffect("Spawned", this(), 100, 0, this());
    return 0;
}

func Destruction()
{
    ++direct_delete_trace;
    return 0;
}

func FxSpawnedStop(object target, int number, int reason)
{
    direct_delete_trace += 10;
    return 0;
}

public func ReadDirectDeleteTrace()
{
    if (!direct_delete_trace) return 0;
    return direct_delete_trace;
}
"#,
    );
    item.set_c4_callback_convention(true);
    engine.register_test_definition(item);
    engine.register_test_definition(test_definition("DEPT", "Departing object", ""));
    engine.configure_scenario_sections(&[section("main", 80, true), section("next", 120, true)]);
    engine.set_landscape(vehicle_section_landscape(80, 40));
    assert_eq!(
        engine.install_global_scripts(&[("System.c4g/SectionObserver.c".into(), script.into())]),
        1
    );
    let departing = spawn_fixture!(engine, "DEPT");
    let mut low = EffectState::new("SectionLow").with_priority(100);
    low.number = 1;
    let mut middle = EffectState::new("SectionMiddle").with_priority(200);
    middle.number = 2;
    let mut high = EffectState::new("SectionHigh").with_priority(300);
    high.number = 3;
    engine.global_effects = vec![low, middle, high];

    assert!(engine.load_test_section("next", 0, Vec::new()));

    let globals = engine.snapshot().script_globals.named;
    assert_eq!(globals.get("clear_seen_count"), Some(&Value::Int(0)));
    assert_eq!(globals.get("clear_seen_damage"), Some(&Value::Int(37)));
    let spawned_number = globals
        .get("clear_spawn_number")
        .and_then(|value| match value {
            Value::Int(number) => Some(*number),
            _ => None,
        })
        .expect("global Stop records the spawned object number");
    assert_ne!(spawned_number, i32::try_from(departing.as_u64()).unwrap());
    assert!(engine.objects.iter().all(|object| {
        object.id.as_u64() != spawned_number as u64 && object.definition_id.as_str() != "ITEM"
    }));
    let following = spawn_fixture!(engine, "ITEM");
    assert_eq!(following.as_u64(), spawned_number as u64 + 1);
    let following_index = engine.test_object_index(following);
    assert_eq!(
        engine.call_test_object_function(following_index, "ReadDirectDeleteTrace", Vec::new()),
        Value::Int(0),
        "Objects.Clear(false) invokes neither Destruction nor effect Stop"
    );
}

#[test]
fn scenario_section_target_init_preserves_a_global_stop_spawn_made_inactive() {
    // The section InitGameSecondPart clear deletes only the active main list;
    // C4OS_INACTIVE objects remain in InactiveObjects (C4Game.cpp:2699-2704;
    // C4GameObjects.cpp:326-331).
    let script = r#"#strict 3
static inactive_spawn_number;

global func FxInactiveSpawnStop(object target, int number, int reason)
{
    var spawned = CreateObject(ITEM, 11, 13, -1);
    inactive_spawn_number = ObjectNumber(spawned);
    SetObjectStatus(C4OS_INACTIVE, spawned);
    return 0;
}
"#;

    let mut engine = Engine::with_seed(44);
    engine.register_test_definition(test_definition("ITEM", "Section item", ""));
    engine.configure_scenario_sections(&[section("main", 80, true), section("next", 120, true)]);
    engine.set_landscape(vehicle_section_landscape(80, 40));
    assert_eq!(
        engine.install_global_scripts(&[("System.c4g/InactiveSpawn.c".into(), script.into())]),
        1
    );
    let mut effect = EffectState::new("InactiveSpawn").with_priority(100);
    effect.number = 1;
    engine.global_effects = vec![effect];

    assert!(engine.load_test_section("next", 0, Vec::new()));

    let number = engine
        .snapshot()
        .script_globals
        .named
        .get("inactive_spawn_number")
        .and_then(|value| match value {
            Value::Int(number) => Some(*number),
            _ => None,
        })
        .expect("global Stop records its inactive spawn");
    assert!(engine.objects.iter().any(|object| {
        object.id.as_u64() == number as u64
            && object.state.status == ObjectStatus::Inactive
            && object.state.position == Vector2::new(11, 13)
    }));
}

#[test]
fn scenario_section_destruction_deactivation_removes_a_future_live_link() {
    // The first teardown loop follows the mutable Objects.First links. A
    // Destruction callback that deactivates a future object removes that
    // link immediately, so the loop never calls its Destruction and the
    // inactive object survives (C4Game.cpp:4190-4201;
    // C4Object.cpp:5987-6007; C4ObjectList.cpp:614-618).
    let mut definition = test_definition(
        "SDAC",
        "Section deactivation cursor",
        r#"#strict 3
local deactivate_target;
static destruction_calls;

public func Arm(object target)
{
    deactivate_target = target;
    destruction_calls = 0;
    return true;
}

public func ReadDestructionCalls() { return destruction_calls; }

func Destruction()
{
    ++destruction_calls;
    if (deactivate_target)
        SetObjectStatus(C4OS_INACTIVE, deactivate_target);
    return 0;
}
"#,
    );
    definition.set_c4_callback_convention(true);

    let mut engine = Engine::with_seed(45);
    engine.register_test_definition(definition);
    engine.configure_scenario_sections(&[section("main", 80, true), section("next", 120, true)]);
    engine.set_landscape(vehicle_section_landscape(80, 40));
    let victim = spawn_fixture!(engine, "SDAC");
    let killer = spawn_fixture!(engine, "SDAC");
    let killer_index = engine.test_object_index(killer);
    assert_eq!(
        engine
            .call_test_object_function(killer_index, "Arm", vec![object_reference_value(victim)],),
        Value::Bool(true)
    );

    assert!(engine.load_test_section("next", 0, Vec::new()));

    let victim_index = engine.test_object_index(victim);
    assert_eq!(
        engine.objects[victim_index].state.status,
        ObjectStatus::Inactive
    );
    assert_eq!(
        engine.call_test_object_function(victim_index, "ReadDestructionCalls", Vec::new()),
        Value::Int(1)
    );
}

#[test]
fn scenario_section_destruction_follows_objects_inserted_after_the_live_cursor() {
    // C4ObjectList::stMain inserts by descending category. During the raw
    // Objects.First walk, an object-category spawn lands before the current
    // vehicle link and misses the cursor, while a structure spawn lands after
    // it and receives AssignRemoval. The second pass directly deletes the
    // missed spawn without Destruction (C4ObjectList.cpp:134-175,220-222;
    // C4Game.cpp:4190-4201).
    let globals = r#"#strict 3
static section_cursor_trace;

global func ResetSectionCursorTrace()
{
    section_cursor_trace = 0;
    return true;
}

global func MarkSectionCursor(int mark)
{
    section_cursor_trace = section_cursor_trace * 10 + mark;
    return true;
}
"#;
    let mut current = test_definition(
        "SCUR",
        "Section cursor",
        r#"#strict 3
func Destruction()
{
    MarkSectionCursor(1);
    CreateObject(SBFR, 0, 0, -1);
    CreateObject(SAFT, 0, 0, -1);
    return 0;
}
"#,
    );
    current.set_category(CATEGORY_VEHICLE);
    current.set_c4_callback_convention(true);
    let mut before = test_definition(
        "SBFR",
        "Before cursor",
        "#strict 3\nfunc Destruction() { MarkSectionCursor(2); return 0; }\n",
    );
    before.set_category(CATEGORY_OBJECT);
    before.set_c4_callback_convention(true);
    let mut after = test_definition(
        "SAFT",
        "After cursor",
        "#strict 3\nfunc Destruction() { MarkSectionCursor(3); return 0; }\n",
    );
    after.set_category(CATEGORY_STRUCTURE);
    after.set_c4_callback_convention(true);

    let mut engine = Engine::with_seed(46);
    engine.register_test_definition(current);
    engine.register_test_definition(before);
    engine.register_test_definition(after);
    engine.configure_scenario_sections(&[section("main", 80, true), section("next", 120, true)]);
    engine.set_landscape(vehicle_section_landscape(80, 40));
    assert_eq!(
        engine.install_global_scripts(&[("System.c4g/SectionCursor.c".into(), globals.into())]),
        1
    );
    assert_eq!(
        crate::TestValueExt::test_value(
            engine.call_engine_global_function("ResetSectionCursorTrace", &[]),
        ),
        Value::Bool(true)
    );
    let _ = spawn_fixture!(engine, "SCUR");

    assert!(engine.load_test_section("next", 0, Vec::new()));

    assert_eq!(
        engine
            .snapshot()
            .script_globals
            .named
            .get("section_cursor_trace"),
        Some(&Value::Int(13))
    );
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
    crate::TestValueExt::test_value(engine.register_script_definition(
        "SYNO",
        "Synthetic object",
        "",
    ));
    engine.configure_scenario_sections(&[section("main", 80, true), next]);
    engine.set_landscape(vehicle_section_landscape(80, 40));

    assert!(engine.load_test_section("next", 0, Vec::new()));
    let object = crate::TestValueExt::test_value(engine.object_snapshot(ObjectId::new(900)));
    assert_eq!(object.definition_id, "SYNO");
    assert_eq!(object.position, Vector2::new(12, 34));

    assert!(engine.load_test_section("main", 1, Vec::new()));
    assert!(engine.load_test_section("next", 0, Vec::new()));
    assert!(
        engine.object_snapshot(ObjectId::new(900)).is_some(),
        "a groupless landscape-only freeze retains explicit object templates"
    );
}

#[test]
fn scenario_section_switch_preserves_physical_viewport_sound_routing() {
    // C4Game::LoadScenarioSection reinitializes the section and recalculates
    // the existing graphics viewports; it does not discard them
    // (src/C4Game.cpp:4220-4234). Sound must therefore still find a remote
    // player displayed by this process after the switch
    // (src/C4Script.cpp:2297-2309).
    let mut engine = Engine::with_seed(3);
    crate::TestValueExt::test_value(
        engine.register_player(PlayerConfig::new(1, "Remote observer target")),
    );
    engine.set_local_players([]);
    engine.set_physical_viewport_players([1]);
    crate::TestValueExt::test_value(engine.load_scenario_script_with_convention(
        "SectionViewport.c",
        "#strict 3\nfunc Probe() { Sound(\"SectionObserver\", true, nil, 100, 2); }\n",
        true,
    ));
    engine.configure_scenario_sections(&[section("main", 80, true), section("next", 120, true)]);
    engine.set_landscape(vehicle_section_landscape(80, 40));

    assert!(engine.load_test_section("next", 0, Vec::new()));
    crate::TestValueExt::test_value(engine.call_scenario_script_function("Probe", Vec::new()));

    assert!(engine.pending_audio.iter().any(|command| matches!(
        command,
        AudioCommand::PlaySound { name, .. } if name == "SectionObserver"
    )));
}

#[test]
fn scenario_section_switch_preserves_global_and_inactive_object_sounds() {
    #[derive(Default)]
    struct SectionSoundHost {
        instances: Vec<Option<ObjectId>>,
        detached: Vec<ObjectId>,
        clear_count: usize,
    }

    impl SynchronousSoundHost for SectionSoundHost {
        fn start_sound(&mut self, request: &LocalSoundStart, _world: &dyn LocalAudioWorld) -> bool {
            self.instances.push(request.target);
            true
        }

        fn stop_sound(&mut self, _name: &str, _target: Option<ObjectId>) {}

        fn set_sound_volume(
            &mut self,
            _name: &str,
            _target: Option<ObjectId>,
            _volume: i32,
            _world: &dyn LocalAudioWorld,
        ) {
        }

        fn detach_object_sounds(
            &mut self,
            target: ObjectId,
            _position: Vector2,
            _world: &dyn LocalAudioWorld,
        ) {
            self.instances.retain(|instance| *instance != Some(target));
            self.detached.push(target);
        }

        fn clear_sound_instances(&mut self) {
            self.instances.clear();
            self.clear_count += 1;
        }
    }

    let mut definition = test_definition("SND1", "Sound source", "");
    definition.configure_actions(
        Some("Idle".to_string()),
        HashMap::from([
            ("Idle".to_string(), ActionSpec::default()),
            ("Loop".to_string(), ActionSpec::default().with_sound("Loop")),
        ]),
    );
    let mut engine = Engine::with_seed(3);
    engine.register_test_definition(definition);
    engine.configure_scenario_sections(&[section("main", 80, true), section("next", 120, true)]);
    engine.set_landscape(vehicle_section_landscape(80, 40));
    let active = spawn_fixture!(engine, "SND1");
    let inactive = spawn_fixture!(
        engine,
        "SND1",
        with_status: ObjectStatus::Inactive,
        with_action: ActionState::new("Loop")
    );
    let inactive_index = engine.test_object_index(inactive);
    engine.objects[inactive_index].state.action = ActionState::new("Loop");
    engine.objects[inactive_index].active_action_sound = Some("Loop".to_string());
    engine.objects[inactive_index].action_sound_initialized = true;
    let host = Rc::new(RefCell::new(SectionSoundHost::default()));
    let registration = SynchronousSoundHostRegistration::new(&host);
    engine.configure_synchronous_sound_host(Some(registration.handle()));
    for target in [None, Some(active), Some(inactive)] {
        engine.emit_audio_command(AudioCommand::PlaySound {
            name: "Loop".into(),
            target,
            volume: 100,
            looped: true,
            multiple: true,
            custom_falloff: None,
            target_position: None,
        });
    }
    engine.drain_tick_presentation();
    assert_eq!(
        host.borrow().instances,
        vec![None, Some(active), Some(inactive)]
    );

    // LoadScenarioSection removes only active objects, and ClearPointers only
    // visits instances attached to each removed object. Global instances and
    // sounds on retained inactive objects survive (C4Game.cpp:4190-4201;
    // C4SoundSystem.cpp:89-95).
    assert!(engine.load_test_section("next", 0, vec![inactive]));
    assert_eq!(host.borrow().instances, vec![None, Some(inactive)]);
    engine.drain_tick_presentation();
    assert_eq!(host.borrow().instances, vec![None, Some(inactive)]);
    assert_eq!(host.borrow().detached, vec![active]);
    assert_eq!(host.borrow().clear_count, 0);
    let inactive_index = engine.test_object_index(inactive);
    assert!(engine.objects[inactive_index].action_sound_initialized);
    assert_eq!(
        engine.objects[inactive_index]
            .active_action_sound
            .as_deref(),
        Some("Loop"),
        "the retained ActMap loop must not be selected or started a second time",
    );

    let state = engine.capture_state();
    crate::TestValueExt::test_value(engine.restore_state(&state));
    assert!(host.borrow().instances.is_empty());
    assert_eq!(host.borrow().clear_count, 1);
}

#[test]
fn resumed_non_main_implicit_root_cannot_reopen_after_unsaved_departure() {
    let mut engine = resumed_non_main_root_engine();

    assert!(engine.load_test_section("Main", 0, Vec::new()));
    assert_eq!(engine.debug_current_scenario_section(), "Main");
    assert!(
        !engine.load_test_section("Cave", 0, Vec::new()),
        "an implicit non-main root has no Filename for GetGroupfile"
    );
    assert_eq!(engine.debug_current_scenario_section(), "Main");
    assert_eq!(engine.landscape().map(Landscape::width), Some(120));
}

#[test]
fn resumed_non_main_implicit_root_reopens_after_saved_departure() {
    let mut engine = resumed_non_main_root_engine();

    assert!(engine.load_test_section("Main", 3, Vec::new()));
    assert!(
        engine
            .scenario_sections
            .get("cave")
            .is_some_and(|section| section.frozen_group.is_some()),
        "C4S_SAVE_LANDSCAPE | C4S_SAVE_OBJECTS creates the temp group"
    );
    assert!(engine.load_test_section("Cave", 0, Vec::new()));
    assert_eq!(engine.debug_current_scenario_section(), "Cave");
    assert_eq!(engine.landscape().map(Landscape::width), Some(80));
}

#[test]
fn section_object_save_enumerates_active_and_inactive_compiler_caches() {
    // Objects.Save(false, false) writes active objects only, but native
    // Enumerate/Denumerate still visits the inactive list on both sides
    // of decompilation (C4GameObjects.cpp:691-713).
    let mut engine = Engine::with_seed(11);
    engine.register_test_definition(test_definition("ITEM", "Item", ""));
    engine.configure_scenario_sections(&[section("main", 80, true), section("next", 100, true)]);
    engine.set_landscape(vehicle_section_landscape(80, 40));

    let active = spawn_fixture!(engine, "ITEM");
    let inactive = spawn_fixture!(engine, "ITEM", with_status: ObjectStatus::Inactive);
    let preserved = spawn_fixture!(engine, "ITEM", with_status: ObjectStatus::Inactive);
    let inactive_number = crate::TestValueExt::test_value(i32::try_from(inactive.as_u64()));
    let active_index = engine.test_object_index(active);
    engine.objects[active_index].state.action.target = Some(inactive);
    engine.objects[active_index].compiler_cache.action_target1 = 991;
    let inactive_index = engine.test_object_index(inactive);
    engine.objects[inactive_index].state.action.target = Some(inactive);
    engine.objects[inactive_index].compiler_cache.action_target1 = 992;
    let preserved_index = engine.test_object_index(preserved);
    let off_list = ObjectId::new(999);
    engine.objects[preserved_index].state.action.target = Some(off_list);
    engine.objects[preserved_index].state.layer = Some(off_list);
    engine.objects[preserved_index]
        .compiler_cache
        .action_target1 = 993;
    engine.objects[preserved_index].compiler_cache.layer = 994;

    assert!(engine.load_test_section("next", 2, vec![inactive, preserved]));

    let frozen = crate::TestValueExt::test_value(
        engine
            .scenario_sections
            .get("main")
            .and_then(|section| section.frozen_group.clone()),
    );
    let group = crate::TestValueExt::test_value(clonk_resources::Group::from_raw_memory(
        std::path::PathBuf::from("Sectmain.c4g"),
        frozen,
    ));
    let objects_txt = crate::TestValueExt::test_value(group.read_entry_bytes("Objects.txt"));
    assert!(
        String::from_utf8_lossy(&objects_txt)
            .contains(&format!("ActionTarget1={inactive_number}\r\n")),
        "active rows decompile the enumerated cache word",
    );

    let inactive_index = engine.test_object_index(inactive);
    assert_eq!(
        engine.objects[inactive_index].compiler_cache.action_target1, inactive_number,
        "inactive wrappers receive the same enumeration side effect",
    );
    assert_eq!(
        engine.objects[inactive_index].state.action.target,
        Some(inactive),
        "inactive wrapper denumerates through the shared number table",
    );
    let preserved_index = engine.test_object_index(preserved);
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
    assert!(!engine.load_test_section("missing", 0, Vec::new()));
    assert_eq!(
        engine.rng, unknown_before,
        "the known-section gate precedes FixRandom"
    );

    engine.rng.random(17);
    engine.rng.rnd3();
    assert!(engine.load_test_section("next", 0, Vec::new()));
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
    crate::TestValueExt::test_value(raw_static.save_initial());
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

    assert!(engine.load_test_section("exact", 0, Vec::new()));
    let landscape = crate::TestValueExt::test_value(engine.landscape());
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

    assert!(engine.load_test_section("exact", 0, Vec::new()));
    assert_eq!(
        engine.landscape().unwrap().mode(),
        LANDSCAPE_MODE_EXACT,
        "an undefined pre-load mode permits the post-Clear exact assignment"
    );

    assert!(engine.load_test_section("next", 0, Vec::new()));
    let landscape = crate::TestValueExt::test_value(engine.landscape());
    assert!(landscape.map_changed());
    assert_eq!(landscape.map_seed(), -7);
    assert_eq!(landscape.modulation(), 0xaabb_ccdd);
    assert!(landscape.texture_map_entries_added());
    assert_eq!(landscape.mode(), LANDSCAPE_MODE_UNDEFINED);
    assert_eq!(landscape.grid_byte_at(0, 0), Some(2));
    let texmap = crate::TestValueExt::test_value(landscape.raster_state()).texmap();
    assert_eq!(texmap.material_names[1].as_deref(), Some("Earth"));
    assert_eq!(texmap.material_names[2].as_deref(), Some("Water"));
    assert_eq!(
        InitialNetworkGameData::from_engine(&engine)
            .expect("section runtime state captures")
            .current_scenario_section,
        "next"
    );

    assert!(engine.load_test_section("raw", 0, Vec::new()));
    let landscape = crate::TestValueExt::test_value(engine.landscape());
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
    crate::TestValueExt::test_value(main.save_initial());
    main.grid_write_byte(1, 1, 1);
    let next = vehicle_section_landscape(4, 4);

    let mut engine = Engine::with_seed(23);
    engine.configure_scenario_sections(&[
        vehicle_section("main", main.clone()),
        vehicle_section("next", next),
    ]);
    engine.set_landscape(main);

    assert!(engine.load_test_section("next", 1, Vec::new()));
    assert!(engine.load_test_section("main", 0, Vec::new()));
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

    assert!(engine.load_test_section("next", 1, Vec::new()));
    let frozen = crate::TestValueExt::test_value(
        engine
            .scenario_sections
            .get("main")
            .and_then(|section| section.frozen_group.clone()),
    );
    let group = crate::TestValueExt::test_value(clonk_resources::Group::from_raw_memory(
        std::path::PathBuf::from("Sectmain.c4g"),
        frozen,
    ));
    assert!(
        group.exists("Map.bmp"),
        "C4Landscape::Save persists the changed retained map"
    );

    assert!(engine.load_test_section("main", 0, Vec::new()));
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

    assert!(engine.load_test_section("empty", 0, Vec::new()));
    let landscape = crate::TestValueExt::test_value(engine.landscape());
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

    let mut gate = test_definition(
        "SCGT",
        "Section gate",
        r#"
            #strict 2
            public func OpenMask() { return SetSolidMask(0, 0, 0, 0); }
        "#,
    );
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
    engine.register_test_definition(gate);
    let gate =
        spawn_fixture!(engine, "SCGT", with_position: Vector2::new(10, 10), with_loaded: true);
    let gate_index = engine.test_object_index(gate);
    engine.update_solid_mask(gate_index);
    assert_eq!(
        engine
            .landscape()
            .expect("main landscape")
            .grid_byte_at(10, 10),
        Some(2)
    );

    assert!(engine.load_test_section("next", 3, Vec::new()));
    assert!(engine.load_test_section("main", 3, Vec::new()));
    assert_eq!(
        engine
            .landscape()
            .expect("restored main landscape")
            .grid_byte_at(10, 10),
        Some(2),
        "restored gate must be put over the clean section plane"
    );

    let gate_index = engine.test_object_index(gate);
    crate::TestValueExt::test_value(engine.call_object_function(
        gate_index,
        "OpenMask",
        Vec::new(),
    ));
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
    engine.register_test_definition(two_pixel_solid_mask_definition("SMFL", 255));
    engine.register_test_definition(two_pixel_solid_mask_definition("SMHF", 0));

    let _overlap_owner = spawn_fixture!(engine, "SMFL", with_position: Vector2::new(OVERLAP_X, 10), with_loaded: true);
    let overlap_survivor = spawn_fixture!(engine, "SMHF", with_position: Vector2::new(OVERLAP_X, 10), with_loaded: true);
    let _active = spawn_fixture!(engine, "SMFL", with_position: Vector2::new(ACTIVE_X, 10), with_loaded: true);
    let _all_active_owner = spawn_fixture!(engine, "SMFL", with_position: Vector2::new(ALL_ACTIVE_X, 10), with_loaded: true);
    let _all_active_second = spawn_fixture!(engine, "SMHF", with_position: Vector2::new(ALL_ACTIVE_X, 10), with_loaded: true);
    let loaded_inactive = spawn_fixture!(engine, "SMFL", with_position: Vector2::new(LOADED_INACTIVE_X, 10), with_status: ObjectStatus::Inactive, with_loaded: true);
    let standalone_inactive = spawn_fixture!(engine, "SMHF", with_position: Vector2::new(STANDALONE_INACTIVE_X, 10), with_loaded: true);

    let all_active_capture = engine.capture_state();
    let all_active_landscape =
        crate::TestValueExt::test_value(all_active_capture.landscape.as_ref());
    for x in [OVERLAP_X, ACTIVE_X, ALL_ACTIVE_X, STANDALONE_INACTIVE_X] {
        assert_eq!(all_active_landscape.grid_byte_at(x, 10), Some(1));
        assert_eq!(all_active_landscape.grid_byte_at(x + 1, 10), Some(1));
    }

    for object in [overlap_survivor, standalone_inactive] {
        crate::TestValueExt::test_value(engine.apply_object_update(
            object,
            ObjectUpdate {
                status: Some(ObjectStatus::Inactive),
                ..ObjectUpdate::default()
            },
        ));
        let index = engine.test_object_index(object);
        assert!(
            engine.objects[index].solid_mask_bake.is_some(),
            "runtime deactivation retains the existing C4SolidMask"
        );
    }
    let loaded_index = engine.test_object_index(loaded_inactive);
    assert!(
        engine.objects[loaded_index].solid_mask_bake.is_none(),
        "loaded inactive objects must not be synthesized into survivors"
    );

    let capture = engine.capture_state();
    let captured = crate::TestValueExt::test_value(capture.landscape.as_ref());
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

    assert!(engine.load_test_section(
        "next",
        1,
        vec![overlap_survivor, loaded_inactive, standalone_inactive],
    ));
    let saved = crate::TestValueExt::test_value(
        engine
            .scenario_sections
            .get("main")
            .and_then(|section| section.landscape.as_ref()),
    );
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
    let library = crate::TestValueExt::test_value(clonk_resources::MaterialLibrary::parse(
        r#"
            [Material Earth]
            Name=Earth
            Density=100

            [Material Water]
            Name=Water
            Density=25
            "#,
    ));
    let materials = MaterialSet::from_resource_library(&library);
    let earth = crate::TestValueExt::test_value(materials.id_of("Earth"));
    let water = crate::TestValueExt::test_value(materials.id_of("Water"));
    let mut engine = Engine::with_seed(37);
    engine.set_materials(materials);
    engine.configure_scenario_sections(&[section("main", 20, true), section("next", 20, true)]);
    engine.set_landscape(vehicle_section_landscape(20, 40));
    assert!(engine.pxs_system.create_at(
        3,
        4,
        pxs::Pxs {
            mat: earth.into(),
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

    assert!(engine.load_test_section("next", 1, Vec::new()));
    assert_eq!(engine.pxs_system.count(), 0);
    assert_eq!(engine.pxs_system.execute_count(), 0);
    assert_eq!(engine.mass_movers.live_movers(), 0);
    assert!(engine.load_test_section("main", 1, Vec::new()));

    let pixel = crate::TestValueExt::test_value(engine.pxs_system.peek_slot(0, 4));
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
    let library = crate::TestValueExt::test_value(clonk_resources::MaterialLibrary::parse(
        "[Material Earth]\nName=Earth\nDensity=100\n",
    ));
    let materials = MaterialSet::from_resource_library(&library);
    let earth = crate::TestValueExt::test_value(materials.id_of("Earth"));
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
            mat: earth.into(),
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

    assert!(engine.load_test_section("next", 0, Vec::new()));
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
