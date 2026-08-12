use super::*;

trait TestEngineExt {
    fn test_object_index(&self, object: ObjectId) -> usize;
    fn register_test_definition(&mut self, definition: Definition);
    fn register_test_player(&mut self, player: PlayerConfig);
    fn register_test_script_definition(&mut self, id: &str, name: &str, script: &str);
    fn spawn_test_object(&mut self, config: SpawnConfig) -> ObjectId;
}

impl TestEngineExt for Engine {
    #[track_caller]
    fn test_object_index(&self, object: ObjectId) -> usize {
        crate::TestValueExt::test_value(self.find_object_index(object))
    }

    #[track_caller]
    fn register_test_definition(&mut self, definition: Definition) {
        crate::TestValueExt::test_value(self.register_definition(definition));
    }

    #[track_caller]
    fn register_test_player(&mut self, player: PlayerConfig) {
        crate::TestValueExt::test_value(self.register_player(player));
    }

    #[track_caller]
    fn register_test_script_definition(&mut self, id: &str, name: &str, script: &str) {
        crate::TestValueExt::test_value(self.register_script_definition(id, name, script));
    }

    #[track_caller]
    fn spawn_test_object(&mut self, config: SpawnConfig) -> ObjectId {
        crate::TestValueExt::test_value(self.spawn_object(config))
    }
}

fn put_fixture_engine() -> Engine {
    let actor_script = r#"#strict
local tracked, nested_throw, reject_contents_seen, menu_after_nested, menu_selection_after_nested;
local put_depth, finish_nested_throw, command_after_nested, menu_during_value, menu_selection_during_value;
local mutate_menu_during_value, spawn_menu_item, reexecute_same_throw, command_after_same_reentry;
local command_during_throw_start, departure_seen;
public func NoteRejectContents()
{
  reject_contents_seen = 1;
  return(1);
}
public func NoteDeparture()
{
  departure_seen = 1;
  return(1);
}
public func RunOutsideThrow()
{
  AddCommand(this, "Throw");
  ExecuteCommand();
  return(1);
}
protected func StartThrow()
{
  command_during_throw_start = GetCommand();
  return(1);
}
protected func CalcValue(pInBase, iForPlr)
{
  menu_during_value = GetMenu();
  menu_selection_during_value = GetMenuSelection();
  if (mutate_menu_during_value) SetMenuSize(3, 4);
  if (spawn_menu_item)
  {
    spawn_menu_item = 0;
    var added = CreateObject(NITM, 0, 0, -1);
    if (added) added->Enter(pInBase);
  }
  return(7);
}
protected func Put()
{
  put_depth++;
  if (put_depth == 2 && finish_nested_throw)
  {
    FinishCommand(this, 1, 0);
    ExecuteCommand();
  }
  if (put_depth == 1 && nested_throw)
  {
    AddCommand(this, "Throw");
    ExecuteCommand();
    menu_after_nested = GetMenu();
    menu_selection_after_nested = GetMenuSelection();
    command_after_nested = GetCommand();
  }
  if (put_depth == 1 && reexecute_same_throw)
  {
    ExecuteCommand();
    command_after_same_reentry = GetCommand();
  }
  tracked->Mark(5);
  put_depth--;
  return(1);
}
"#;
    let target_script = r#"#strict
local reject, reject_contents_actor, nested_put_actor, nested_put_item, nested_put_depth;
protected func RejectContents()
{
  if (reject_contents_actor) reject_contents_actor->NoteRejectContents();
  return(0);
}
protected func RejectCollect(idItem, pItem)
{
  pItem->Mark(2);
  if (nested_put_actor && !nested_put_depth)
  {
    nested_put_depth = 1;
    SetCommand(nested_put_actor, "Put", this(), 0, 0, nested_put_item);
    ExecuteCommand(nested_put_actor);
    nested_put_depth = 0;
    return(1);
  }
  return(reject);
}
protected func Collection2(pItem)
{
  pItem->Mark(3);
  return(1);
}
protected func Collection(pItem, fPut)
{
  pItem->Mark(6);
  return(1);
}
"#;
    let item_script = r#"#strict
local callback_order;
public func Mark(iStep)
{
  callback_order = callback_order * 10 + iStep;
  return(1);
}
protected func RejectEntrance(pTarget)
{
  Mark(1);
  return(0);
}
protected func Entrance(pTarget)
{
  Mark(4);
  return(1);
}
protected func Departure(pTarget)
{
  pTarget->NoteDeparture();
  return(1);
}
"#;

    let mut actor = test_definition("ACTR", "Actor", actor_script);
    actor.set_c4_callback_convention(true);
    actor.configure_actions(
        None,
        HashMap::from([
            (
                "Push".to_string(),
                ActionSpec::default().with_procedure("PUSH"),
            ),
            (
                "Walk".to_string(),
                ActionSpec::default().with_procedure("WALK"),
            ),
            (
                "Throw".to_string(),
                ActionSpec::default()
                    .with_procedure("THROW")
                    .with_start_call("StartThrow"),
            ),
        ]),
    );
    let mut target = test_definition("TARG", "Target", target_script);
    target.set_c4_callback_convention(true);
    target.set_grab_put_get(GRAB_PUT_GET_PUT);
    let mut item = test_definition("ITEM", "Item", item_script);
    item.set_c4_callback_convention(true);
    item.set_collectible(true);

    let mut engine = Engine::with_seed(118);
    engine.register_test_definition(actor);
    engine.register_test_definition(target);
    engine.register_test_definition(item);
    engine
}

fn spawn_push_put_triplet(engine: &mut Engine, reject: bool) -> (ObjectId, ObjectId, ObjectId) {
    let target = engine.spawn_test_object(
        SpawnConfig::new("TARG")
            .with_position(Vector2::new(80, 40))
            .with_velocity(Vector2::new(3, -2))
            .with_local_vars(HashMap::from([(
                "reject".to_string(),
                Value::Int(i32::from(reject)),
            )])),
    );
    let mut push = ActionState::new("Push");
    push.target = Some(target);
    let actor = engine.spawn_test_object(
        SpawnConfig::new("ACTR")
            .with_position(Vector2::new(20, 40))
            .with_action(push),
    );
    let item = engine.spawn_test_object(SpawnConfig::new("ITEM").with_container(actor));
    let actor_index = engine.test_object_index(actor);
    engine.objects[actor_index]
        .state
        .local_vars
        .insert("tracked".to_string(), object_reference_value(item));
    crate::TestValueExt::test_value(
        engine.objects[actor_index].commands.push_front(
            CommandRequest::new(CommandId::Put)
                .with_target(Some(target))
                .with_target2(Some(item))
                .with_ty(Some(1)),
        ),
    );
    (actor, item, target)
}

fn spawn_contained_put_take_triplet(
    engine: &mut Engine,
    reject: bool,
    command: CommandId,
) -> (ObjectId, ObjectId, ObjectId) {
    let target = engine.spawn_test_object(SpawnConfig::new("TARG").with_local_vars(HashMap::from(
        [("reject".to_string(), Value::Int(i32::from(reject)))],
    )));
    let actor = engine.spawn_test_object(SpawnConfig::new("ACTR").with_container(target));
    let item = engine.spawn_test_object(SpawnConfig::new("ITEM").with_container(actor));
    let actor_index = engine.test_object_index(actor);
    engine.objects[actor_index]
        .state
        .local_vars
        .insert("tracked".to_string(), object_reference_value(item));
    crate::TestValueExt::test_value(
        engine.objects[actor_index]
            .commands
            .push_front(CommandRequest::new(command).with_target(Some(item))),
    );
    (actor, item, target)
}

#[test]
fn object_com_put_accepts_an_explicit_non_content_object() {
    let mut engine = put_fixture_engine();
    let target = engine.spawn_test_object(SpawnConfig::new("TARG"));
    let actor = engine.spawn_test_object(SpawnConfig::new("ACTR"));
    let item = engine.spawn_test_object(SpawnConfig::new("ITEM"));
    let actor_index = engine.test_object_index(actor);
    engine.objects[actor_index]
        .state
        .local_vars
        .insert("tracked".to_string(), object_reference_value(item));

    assert!(engine
        .try_object_com_put(actor, target, item)
        .expect("ObjectComPut executes"));
    let item_state = &engine.objects[engine.test_object_index(item)].state;
    assert_eq!(item_state.container, Some(target));
    assert_eq!(
        item_state.local_vars.get("callback_order"),
        Some(&Value::Int(123_456))
    );
}

#[test]
fn empty_put_take_opens_menu_on_a_retained_status_zero_target() {
    let mut engine = put_fixture_engine();
    let target = engine.spawn_test_object(SpawnConfig::new("TARG"));
    let actor = engine.spawn_test_object(SpawnConfig::new("ACTR").with_container(target));
    let target_index = engine.test_object_index(target);
    let _ = engine.objects[target_index].mark_destroyed();

    assert_eq!(
        engine
            .try_object_com_put_take(actor, target, None)
            .expect("ObjectComPutTake executes"),
        ObjectComPutTakeOutcome::Finished
    );
    let menu = crate::TestValueExt::test_value(
        engine.objects[engine.test_object_index(actor)]
            .state
            .menu
            .as_ref(),
    );
    assert_eq!(menu.identification, Value::Int(6));
    assert_eq!(menu.refill_object, Some(target));
}

#[test]
fn exit_unlinks_a_retained_status_zero_object() {
    let mut engine = put_fixture_engine();
    let container = engine.spawn_test_object(SpawnConfig::new("TARG"));
    let item = engine.spawn_test_object(SpawnConfig::new("ITEM").with_container(container));
    let item_index = engine.test_object_index(item);
    let position = engine.objects[item_index].state.position;
    let rotation = engine.objects[item_index].state.rotation;
    let _ = engine.objects[item_index].mark_destroyed();

    assert!(engine
        .exit_object_at_position_with_zero_motion(item, container, position, rotation)
        .expect("Exit executes"));
    assert_eq!(
        engine.objects[engine.test_object_index(item)]
            .state
            .container,
        None
    );
    assert!(!engine.objects[engine.test_object_index(container)]
        .state
        .contents
        .contains(&item));
}

#[test]
fn assign_removal_exit_contents_continues_after_a_child_reenters() {
    let mut engine = put_fixture_engine();
    let reentering_script = r#"#strict
local reenter_target;
protected func Departure(pTarget)
{
  if (reenter_target) Enter(reenter_target);
  return(1);
}
"#;
    let mut reentering = test_definition("RITM", "Reentering item", reentering_script);
    reentering.set_c4_callback_convention(true);
    engine.register_test_definition(reentering);
    let destination = engine.spawn_test_object(SpawnConfig::new("TARG"));
    let parent = engine.spawn_test_object(SpawnConfig::new("ACTR"));
    let first = engine.spawn_test_object(SpawnConfig::new("RITM").with_container(parent));
    let second = engine.spawn_test_object(SpawnConfig::new("RITM").with_container(parent));
    let parent_index = engine.test_object_index(parent);
    let first_link = engine.objects[parent_index].state.contents[0];
    let sibling = if first_link == first { second } else { first };
    let first_link_index = engine.test_object_index(first_link);
    engine.objects[first_link_index].state.local_vars.insert(
        "reenter_target".to_string(),
        object_reference_value(destination),
    );

    assert!(engine
        .assign_object_removal_with_contents(parent, true)
        .expect("AssignRemoval executes"));
    assert_eq!(
        engine.objects[engine.test_object_index(first_link)]
            .state
            .container,
        Some(destination)
    );
    assert_eq!(
        engine.objects[engine.test_object_index(sibling)]
            .state
            .container,
        None,
        "the loop re-reads Contents.First even when the prior Exit returned false"
    );
}

#[test]
fn put_event_runs_object_com_put_callbacks_and_ty_ungrab_only_on_success() {
    let mut engine = put_fixture_engine();
    let (actor, item, target) = spawn_push_put_triplet(&mut engine, false);

    crate::TestValueExt::test_value(engine.execute_object_command_now(actor));

    let item_index = engine.test_object_index(item);
    let target_index = engine.test_object_index(target);
    assert_eq!(engine.objects[item_index].state.container, Some(target));
    assert_eq!(
        engine.objects[item_index]
            .state
            .local_vars
            .get("callback_order"),
        Some(&Value::Int(123_456)),
        "RejectEntrance -> RejectCollect -> Collection2 -> Entrance -> Put -> Collection"
    );
    assert_eq!(
        engine.objects[item_index].state.position,
        engine.objects[target_index].state.position
    );
    assert_eq!(
        engine.objects[item_index].fixed_velocity,
        engine.objects[target_index].fixed_velocity
    );
    let actor_index = engine.test_object_index(actor);
    assert_eq!(
        engine.objects[actor_index]
            .commands
            .snapshot()
            .command_names(),
        ["UnGrab", "Put"],
        "Ty queues interval-zero UnGrab above the still-running Put"
    );
    assert_eq!(
        engine.objects[actor_index]
            .commands
            .snapshot()
            .command_views()[1]
            .ty,
        Some(1)
    );

    let (rejected_actor, rejected_item, _) = spawn_push_put_triplet(&mut engine, true);
    crate::TestValueExt::test_value(engine.execute_object_command_now(rejected_actor));
    let rejected_item_index = engine.test_object_index(rejected_item);
    assert_eq!(
        engine.objects[rejected_item_index].state.container,
        Some(rejected_actor)
    );
    assert_eq!(
        engine.objects[rejected_item_index]
            .state
            .local_vars
            .get("callback_order"),
        Some(&Value::Int(12))
    );
    let rejected_actor_index = engine.test_object_index(rejected_actor);
    assert!(
        engine.objects[rejected_actor_index]
            .commands
            .snapshot()
            .is_empty(),
        "helper failure finishes Put and must not queue Ty-UnGrab"
    );
}

#[test]
fn contained_throw_and_drop_run_object_com_put_callbacks_before_finishing() {
    for command in [CommandId::Throw, CommandId::Drop] {
        let mut accepted = put_fixture_engine();
        let (actor, item, target) = spawn_contained_put_take_triplet(&mut accepted, false, command);
        crate::TestValueExt::test_value(accepted.execute_object_command_now(actor));

        let item_state = &accepted.objects[accepted.test_object_index(item)].state;
        assert_eq!(item_state.container, Some(target), "{command:?} transfers");
        assert_eq!(
            item_state.local_vars.get("callback_order"),
            Some(&Value::Int(123_456)),
                "{command:?}: RejectEntrance -> RejectCollect -> Collection2 -> Entrance -> Put -> Collection"
        );
        assert!(
            accepted.objects[accepted.test_object_index(actor)]
                .commands
                .snapshot()
                .is_empty(),
            "{command:?} finishes after the callback tail"
        );

        let mut rejected = put_fixture_engine();
        let (actor, item, _) = spawn_contained_put_take_triplet(&mut rejected, true, command);
        crate::TestValueExt::test_value(rejected.execute_object_command_now(actor));
        let item_state = &rejected.objects[rejected.test_object_index(item)].state;
        assert_eq!(
            item_state.container,
            Some(actor),
            "{command:?} honors RejectCollect"
        );
        assert_eq!(
            item_state.local_vars.get("callback_order"),
            Some(&Value::Int(12)),
            "Put/Collection must not run after RejectCollect"
        );
        assert!(
            rejected.objects[rejected.test_object_index(actor)]
                .commands
                .snapshot()
                .is_empty(),
            "{command:?} ignores the helper result and still finishes"
        );
    }
}

#[test]
fn command_references_accept_inactive_live_put_take_helpers_like_cpp() {
    for command in [CommandId::Throw, CommandId::Drop] {
        let mut engine = put_fixture_engine();
        let target = engine.spawn_test_object(SpawnConfig::new("TARG"));
        let actor = engine.spawn_test_object(SpawnConfig::new("ACTR").with_container(target));
        let deleted = engine.spawn_test_object(SpawnConfig::new("ITEM").with_container(actor));
        let inactive = engine.spawn_test_object(SpawnConfig::new("ITEM").with_container(actor));

        let deleted_index = engine.test_object_index(deleted);
        let _ = engine.objects[deleted_index].mark_destroyed();
        let inactive_index = engine.test_object_index(inactive);
        engine.objects[inactive_index].state.status = ObjectStatus::Inactive;
        let actor_index = engine.test_object_index(actor);
        engine.objects[actor_index].state.contents = vec![deleted, inactive];
        crate::TestValueExt::test_value(
            engine.objects[actor_index]
                .commands
                .push_front(CommandRequest::new(command)),
        );

        crate::TestValueExt::test_value(engine.execute_object_command_now(actor));

        assert_eq!(
            engine
                .object_snapshot(inactive)
                .expect("inactive item remains")
                .container,
            Some(target),
            "{command:?} skips Status==0 and retains C4OS_INACTIVE"
        );
        assert_eq!(
            engine
                .object_snapshot(deleted)
                .expect("tombstone remains until end-of-tick cleanup")
                .container,
            Some(actor),
            "the deleted contents hole is never selected"
        );
    }
}

#[test]
fn command_references_accept_inactive_sell_candidates_like_cpp() {
    let mut engine = put_fixture_engine();
    engine.register_test_player(PlayerConfig::new(1, "Seller"));
    let base = engine.spawn_test_object(SpawnConfig::new("TARG"));
    let base_index = engine.test_object_index(base);
    engine.objects[base_index].state.base = 1;
    let actor = engine.spawn_test_object(SpawnConfig::new("ACTR").with_owner(1));
    let deleted = engine.spawn_test_object(SpawnConfig::new("ITEM").with_container(base));
    let inactive = engine.spawn_test_object(SpawnConfig::new("ITEM").with_container(base));

    let deleted_index = engine.test_object_index(deleted);
    let _ = engine.objects[deleted_index].mark_destroyed();
    let inactive_index = engine.test_object_index(inactive);
    engine.objects[inactive_index].state.status = ObjectStatus::Inactive;
    engine.objects[base_index].state.contents = vec![deleted, inactive];

    assert_eq!(
        engine.command_sell_candidate(actor, base, "ITEM", Some(inactive)),
        Some((1, inactive)),
        "an explicit inactive Target2 remains preferred"
    );
    assert_eq!(
        engine.command_sell_candidate(actor, base, "ITEM", Some(deleted)),
        Some((1, inactive)),
        "a deleted Target2 falls back to Contents.Find's inactive entry"
    );
}

#[test]
fn nested_put_take_does_not_consume_the_outer_put_result_marker() {
    let mut engine = put_fixture_engine();
    crate::TestValueExt::test_value(engine.definitions.get_mut(&DefinitionId::from("TARG")))
        .set_collection_limit(1);
    let (actor, first_item, target) = spawn_push_put_triplet(&mut engine, false);
    let second_item = engine.spawn_test_object(SpawnConfig::new("ITEM").with_container(actor));
    let actor_index = engine.test_object_index(actor);
    engine.objects[actor_index]
        .state
        .local_vars
        .insert("nested_throw".to_string(), Value::Int(1));

    crate::TestValueExt::test_value(engine.execute_object_command_now(actor));

    assert_eq!(
        engine
            .object_snapshot(first_item)
            .expect("first item remains")
            .container,
        Some(target),
        "the outer Put succeeds before its Put callback"
    );
    assert_eq!(
        engine
            .object_snapshot(second_item)
            .expect("second item remains")
            .container,
        Some(actor),
        "the nested Throw's put is rejected by the now-full target"
    );
    let actor_index = engine.test_object_index(actor);
    assert_eq!(
        engine.objects[actor_index]
            .commands
            .snapshot()
            .command_names(),
        ["UnGrab", "Put"],
        "the nested false PutTake result must not fail the outer successful Put"
    );

    crate::TestValueExt::test_value(engine.execute_object_command_now(actor));
    crate::TestValueExt::test_value(engine.execute_object_command_now(actor));
    assert!(
        engine.objects[engine.test_object_index(actor)]
            .commands
            .snapshot()
            .is_empty(),
        "the outer Put retained and consumed its own success result"
    );
}

#[test]
fn recursive_same_kind_put_keeps_each_callback_result_with_its_emitter() {
    let mut engine = put_fixture_engine();
    let target = engine.spawn_test_object(SpawnConfig::new("TARG"));
    let actor = engine.spawn_test_object(SpawnConfig::new("ACTR").with_container(target));
    let outer_item = engine.spawn_test_object(SpawnConfig::new("ITEM").with_container(actor));
    let replacement_item = engine.spawn_test_object(SpawnConfig::new("ITEM").with_container(actor));
    let target_index = engine.test_object_index(target);
    engine.objects[target_index].state.local_vars.extend([
        (
            "nested_put_actor".to_string(),
            object_reference_value(actor),
        ),
        (
            "nested_put_item".to_string(),
            object_reference_value(replacement_item),
        ),
    ]);
    let actor_index = engine.test_object_index(actor);
    engine.objects[actor_index].state.local_vars.insert(
        "tracked".to_string(),
        object_reference_value(replacement_item),
    );
    crate::TestValueExt::test_value(
        engine.objects[actor_index].commands.push_front(
            CommandRequest::new(CommandId::Put)
                .with_target(Some(target))
                .with_target2(Some(outer_item)),
        ),
    );

    crate::TestValueExt::test_value(engine.execute_object_command_now(actor));

    assert_eq!(
        engine
            .object_snapshot(outer_item)
            .expect("outer item remains")
            .container,
        Some(actor),
        "the outer RejectCollect result stays with the detached outer Put"
    );
    assert_eq!(
        engine
            .object_snapshot(replacement_item)
            .expect("replacement item remains")
            .container,
        Some(target),
        "recursive ExecuteCommand completes the replacement helper synchronously"
    );
    let actor_index = engine.test_object_index(actor);
    let commands = engine.objects[actor_index].commands.command_views();
    let [replacement] = commands.as_slice() else {
        panic!("replacement Put must remain for its next native Execute: {commands:?}");
    };
    assert_eq!(replacement.name, "Put");
    assert_eq!(replacement.target, Some(target));
    assert_eq!(replacement.target2, Some(replacement_item));
    assert!(
        !replacement.finished,
        "the outer failure must not consume the successful replacement Put"
    );
}

#[test]
fn removed_nested_throw_does_not_finish_the_outer_throw_instance() {
    let mut engine = put_fixture_engine();
    let target = engine.spawn_test_object(SpawnConfig::new("TARG"));
    let actor = engine.spawn_test_object(SpawnConfig::new("ACTR").with_container(target));
    let first_item = engine.spawn_test_object(SpawnConfig::new("ITEM").with_container(actor));
    let second_item = engine.spawn_test_object(SpawnConfig::new("ITEM").with_container(actor));
    let actor_index = engine.test_object_index(actor);
    engine.objects[actor_index].state.local_vars.extend([
        ("tracked".to_string(), object_reference_value(first_item)),
        ("nested_throw".to_string(), Value::Int(1)),
        ("finish_nested_throw".to_string(), Value::Int(1)),
    ]);
    crate::TestValueExt::test_value(
        engine.objects[actor_index]
            .commands
            .push_front(CommandRequest::new(CommandId::Throw).with_target(Some(first_item))),
    );

    crate::TestValueExt::test_value(engine.execute_object_command_now(actor));

    let actor_index = engine.test_object_index(actor);
    assert_eq!(
        engine.objects[actor_index]
            .state
            .local_vars
            .get("command_after_nested"),
        Some(&Value::String("Throw".to_string().into())),
        "removing the inner Throw while its helper returns leaves the outer instance live"
    );
    assert!(
        engine.objects[actor_index].commands.snapshot().is_empty(),
        "the outer Throw finishes only when its own helper returns"
    );
    for item in [first_item, second_item] {
        assert_eq!(
            engine
                .object_snapshot(item)
                .expect("item remains")
                .container,
            Some(target)
        );
    }
}

#[test]
fn callback_execute_command_reenters_the_same_in_flight_throw() {
    let mut engine = put_fixture_engine();
    let target = engine.spawn_test_object(SpawnConfig::new("TARG"));
    let actor = engine.spawn_test_object(SpawnConfig::new("ACTR").with_container(target));
    let item = engine.spawn_test_object(SpawnConfig::new("ITEM").with_container(actor));
    let actor_index = engine.test_object_index(actor);
    engine.objects[actor_index].state.local_vars.extend([
        ("tracked".to_string(), object_reference_value(item)),
        ("reexecute_same_throw".to_string(), Value::Int(1)),
    ]);
    crate::TestValueExt::test_value(
        engine.objects[actor_index]
            .commands
            .push_front(CommandRequest::new(CommandId::Throw).with_target(Some(item))),
    );

    crate::TestValueExt::test_value(engine.execute_object_command_now(actor));

    let actor_index = engine.test_object_index(actor);
    assert_eq!(
        engine.objects[actor_index]
            .state
            .local_vars
            .get("command_after_same_reentry"),
        Some(&Value::String("Get".to_string().into())),
        "the in-flight Throw reexecutes and queues Get after its requested item moved"
    );
    assert_eq!(
        engine.objects[actor_index]
            .commands
            .snapshot()
            .command_names(),
        ["Get", "Throw"],
        "the reentrant child remains above the exact finished outer Throw"
    );
}

#[test]
fn script_execute_command_runs_outside_throw_callbacks_before_returning() {
    let mut engine = put_fixture_engine();
    let actor =
        engine.spawn_test_object(SpawnConfig::new("ACTR").with_action(ActionState::new("Walk")));
    let item = engine.spawn_test_object(SpawnConfig::new("ITEM").with_container(actor));
    let actor_index = engine.test_object_index(actor);
    engine.objects[actor_index]
        .state
        .local_vars
        .insert("tracked".to_string(), object_reference_value(item));

    crate::TestValueExt::test_value(engine.call_object_function(
        actor_index,
        "RunOutsideThrow",
        Vec::new(),
    ));

    let actor_index = engine.test_object_index(actor);
    let actor_state = &engine.objects[actor_index].state;
    assert_eq!(
        actor_state.local_vars.get("command_during_throw_start"),
        Some(&Value::String("Throw".to_string().into())),
        "StartCall sees the exact executing Throw before Finish(true)"
    );
    assert_eq!(
        actor_state.local_vars.get("departure_seen"),
        Some(&Value::Int(1)),
        "Departure runs before script ExecuteCommand returns"
    );
    assert!(engine.objects[actor_index].commands.snapshot().is_empty());
    assert_eq!(
        engine
            .object_snapshot(item)
            .expect("item remains")
            .container,
        None
    );
}

#[test]
fn script_throw_from_no_other_action_dig_uses_callbackful_object_com_stop() {
    let script = r#"#strict
public func RunExecute()
{
  ExecuteCommand();
  return(1);
}
"#;
    let mut actor = test_definition("NDIG", "NoOtherAction digger", script);
    actor.set_c4_callback_convention(true);
    actor.configure_actions(
        None,
        HashMap::from([
            (
                "Dig".to_string(),
                ActionSpec::default()
                    .with_procedure("DIG")
                    .with_no_other_action(true),
            ),
            (
                "Walk".to_string(),
                ActionSpec::default().with_procedure("WALK"),
            ),
            (
                "Throw".to_string(),
                ActionSpec::default().with_procedure("THROW"),
            ),
        ]),
    );
    let mut item = test_definition("NDIT", "NoOtherAction item", "#strict");
    item.set_collectible(true);

    let mut engine = Engine::with_seed(119);
    engine.register_test_definition(actor);
    engine.register_test_definition(item);
    let actor = engine.spawn_test_object(
        SpawnConfig::new("NDIG")
            .with_action(ActionState::new("Dig"))
            .with_velocity(Vector2::new(7, -3)),
    );
    let item = engine.spawn_test_object(SpawnConfig::new("NDIT").with_container(actor));
    let actor_index = engine.test_object_index(actor);
    crate::TestValueExt::test_value(
        engine.objects[actor_index]
            .commands
            .push_front(CommandRequest::new(CommandId::Throw).with_target(Some(item))),
    );

    crate::TestValueExt::test_value(engine.call_object_function(
        actor_index,
        "RunExecute",
        Vec::new(),
    ));

    let actor_index = engine.test_object_index(actor);
    assert_eq!(engine.objects[actor_index].state.action.name, "Dig");
    assert_eq!(
        engine.objects[actor_index].fixed_velocity,
        FixedVec2::from_ints(7, -3),
        "failed Idle/Walk transitions do not zero the digging velocity"
    );
    assert_eq!(
        engine.objects[actor_index].state.command_direction,
        CommandDirection::Stop,
        "ObjectActionStand writes ComDir before its rejected Walk"
    );
    assert_eq!(
        engine
            .object_snapshot(item)
            .expect("item remains")
            .container,
        Some(actor),
        "ObjectComThrow rejects the still-Dig procedure"
    );
    assert!(engine.objects[actor_index].commands.snapshot().is_empty());
}

#[test]
fn script_drop_from_dig_runs_object_com_stop_callbacks_before_dropping() {
    let script = r#"#strict
local callback_order;
public func RunExecute()
{
  ExecuteCommand();
  return(1);
}
protected func AbortDig()
{
  callback_order = callback_order * 10 + 1;
  return(1);
}
protected func StartWalk()
{
  callback_order = callback_order * 10 + 2;
  return(1);
}
"#;
    let mut actor = test_definition("DDIG", "Digging dropper", script);
    actor.set_c4_callback_convention(true);
    actor.configure_actions(
        None,
        HashMap::from([
            (
                "Dig".to_string(),
                ActionSpec::default()
                    .with_procedure("DIG")
                    .with_abort_call("AbortDig"),
            ),
            (
                "Walk".to_string(),
                ActionSpec::default()
                    .with_procedure("WALK")
                    .with_start_call("StartWalk"),
            ),
        ]),
    );
    let mut item = test_definition("DDIT", "Dropped item", "#strict");
    item.set_collectible(true);

    let mut engine = Engine::with_seed(121);
    engine.register_test_definition(actor);
    engine.register_test_definition(item);
    let actor = engine.spawn_test_object(
        SpawnConfig::new("DDIG")
            .with_action(ActionState::new("Dig"))
            .with_velocity(Vector2::new(7, -3)),
    );
    let item = engine.spawn_test_object(SpawnConfig::new("DDIT").with_container(actor));
    let actor_index = engine.test_object_index(actor);
    crate::TestValueExt::test_value(
        engine.objects[actor_index]
            .commands
            .push_front(CommandRequest::new(CommandId::Drop).with_target(Some(item))),
    );

    crate::TestValueExt::test_value(engine.call_object_function(
        actor_index,
        "RunExecute",
        Vec::new(),
    ));

    let actor_index = engine.test_object_index(actor);
    assert_eq!(engine.objects[actor_index].state.action.name, "Walk");
    assert_eq!(engine.objects[actor_index].fixed_velocity, FixedVec2::ZERO);
    assert_eq!(
        engine.objects[actor_index]
            .state
            .local_vars
            .get("callback_order"),
        Some(&Value::Int(12)),
        "Dig Abort and Walk Start finish before ObjectComDrop"
    );
    assert_eq!(
        engine
            .object_snapshot(item)
            .expect("item remains")
            .container,
        None
    );
    assert!(engine.objects[actor_index].commands.snapshot().is_empty());
}

#[test]
fn ungrab_command_respects_no_other_action_and_suppresses_release_callbacks() {
    let actor_script = r#"#strict
local grab_calls;
protected func Grab(object target, bool grab)
{
  grab_calls++;
  return(1);
}
"#;
    let target_script = r#"#strict
local grabbed_calls;
protected func Grabbed(object actor, bool grab)
{
  grabbed_calls++;
  return(1);
}
"#;
    let mut actor = test_definition("UNGA", "Locked pusher", actor_script);
    actor.set_c4_callback_convention(true);
    actor.configure_actions(
        None,
        HashMap::from([
            (
                "Push".to_string(),
                ActionSpec::default()
                    .with_procedure("PUSH")
                    .with_no_other_action(true),
            ),
            (
                "Walk".to_string(),
                ActionSpec::default().with_procedure("WALK"),
            ),
        ]),
    );
    let mut target = test_definition("UNGT", "Push target", target_script);
    target.set_c4_callback_convention(true);

    let mut engine = Engine::new();
    engine.register_test_definition(actor);
    engine.register_test_definition(target);
    let target = engine.spawn_test_object(SpawnConfig::new("UNGT"));
    let mut push = ActionState::new("Push");
    push.target = Some(target);
    let actor = engine.spawn_test_object(
        SpawnConfig::new("UNGA")
            .with_action(push)
            .with_velocity(Vector2::new(4, -2)),
    );
    let actor_index = engine.test_object_index(actor);
    engine.objects[actor_index].state.command_direction = CommandDirection::Left;
    crate::TestValueExt::test_value(
        engine.objects[actor_index]
            .commands
            .push_front(CommandRequest::new(CommandId::UnGrab)),
    );

    crate::TestValueExt::test_value(engine.execute_object_command_now(actor));

    let actor_index = engine.test_object_index(actor);
    assert_eq!(engine.objects[actor_index].state.action.name, "Push");
    assert_eq!(
        engine.objects[actor_index].state.action.target,
        Some(target)
    );
    assert_eq!(
        engine.objects[actor_index].fixed_velocity,
        FixedVec2::from_ints(4, -2)
    );
    assert_eq!(
        engine.objects[actor_index].state.command_direction,
        CommandDirection::Stop,
        "C4Command::UnGrab writes Stop even when ObjectComUnGrab fails"
    );
    assert_eq!(
        engine.objects[actor_index]
            .state
            .local_vars
            .get("grab_calls"),
        None
    );
    let target_index = engine.test_object_index(target);
    assert_eq!(
        engine.objects[target_index]
            .state
            .local_vars
            .get("grabbed_calls"),
        None
    );
    assert!(engine.objects[actor_index].commands.snapshot().is_empty());
}

#[test]
fn object_com_ungrab_soft_closes_menu_before_release_callbacks() {
    let actor_script = r#"#strict
local deny, query_calls, grab_calls, menu_during_grab;
public func OpenMenu() { return CreateMenu(WIPF, this(), this(), 0, "Choose"); }
public func SetDeny(value) { deny = value; return(1); }
public func ReadMenu() { return GetMenu(); }
protected func MenuQueryCancel()
{
  query_calls++;
  return deny;
}
protected func Grab(object target, bool grab)
{
  grab_calls++;
  menu_during_grab = GetMenu();
  return(1);
}
"#;
    let target_script = r#"#strict
local grabbed_calls, menu_during_grabbed;
protected func Grabbed(object actor, bool grab)
{
  grabbed_calls++;
  menu_during_grabbed = actor->ReadMenu();
  return(1);
}
"#;
    let mut actor = test_definition("UGMA", "Menu pusher", actor_script);
    actor.set_c4_callback_convention(true);
    actor.configure_actions(
        None,
        HashMap::from([
            (
                "Push".to_string(),
                ActionSpec::default().with_procedure("PUSH"),
            ),
            (
                "Walk".to_string(),
                ActionSpec::default().with_procedure("WALK"),
            ),
        ]),
    );
    let mut target = test_definition("UGMT", "Menu push target", target_script);
    target.set_c4_callback_convention(true);

    let mut engine = Engine::new();
    engine.register_test_definition(actor);
    engine.register_test_definition(target);
    let target = engine.spawn_test_object(SpawnConfig::new("UGMT"));
    let mut push = ActionState::new("Push");
    push.target = Some(target);
    let actor = engine.spawn_test_object(
        SpawnConfig::new("UGMA")
            .with_action(push)
            .with_velocity(Vector2::new(4, -2)),
    );
    let actor_index = engine.test_object_index(actor);
    engine.objects[actor_index].state.command_direction = CommandDirection::Left;

    let call_actor = |engine: &mut Engine, name: &str, args: Vec<Value>| {
        let actor_index = engine.test_object_index(actor);
        crate::TestValueExt::test_value(engine.call_object_function(actor_index, name, args))
    };
    assert_eq!(
        call_actor(&mut engine, "OpenMenu", Vec::new()),
        Value::Bool(true)
    );
    assert_eq!(
        call_actor(&mut engine, "SetDeny", vec![Value::Int(1)]),
        Value::Int(1)
    );

    let actor_index = engine.test_object_index(actor);
    assert!(!engine
        .object_com_ungrab(actor_index)
        .expect("denied ungrab returns"));
    let actor_index = engine.test_object_index(actor);
    assert_eq!(engine.objects[actor_index].state.action.name, "Walk");
    assert_eq!(engine.objects[actor_index].fixed_velocity, FixedVec2::ZERO);
    assert_eq!(
        engine.objects[actor_index].state.command_direction,
        CommandDirection::Stop
    );
    assert!(engine.objects[actor_index].state.menu.is_some());
    assert_eq!(
        engine.objects[actor_index]
            .state
            .local_vars
            .get("query_calls"),
        Some(&Value::Int(1))
    );
    assert_eq!(
        engine.objects[actor_index]
            .state
            .local_vars
            .get("grab_calls"),
        Some(&Value::Nil),
        "a denied close stops before Grab(false)"
    );
    let target_index = engine.test_object_index(target);
    assert_eq!(
        engine.objects[target_index]
            .state
            .local_vars
            .get("grabbed_calls"),
        None,
        "a denied close stops before Grabbed(false)"
    );

    assert_eq!(
        call_actor(&mut engine, "SetDeny", vec![Value::Int(0)]),
        Value::Int(1)
    );
    let actor_index = engine.test_object_index(actor);
    let mut push = ActionState::new("Push");
    push.target = Some(target);
    engine.objects[actor_index].state.action = push;
    assert!(engine
        .object_com_ungrab(actor_index)
        .expect("allowed ungrab succeeds"));

    let actor_index = engine.test_object_index(actor);
    assert_eq!(engine.objects[actor_index].state.action.name, "Walk");
    assert!(engine.objects[actor_index].state.menu.is_none());
    assert_eq!(
        engine.objects[actor_index]
            .state
            .local_vars
            .get("query_calls"),
        Some(&Value::Int(2))
    );
    assert_eq!(
        engine.objects[actor_index]
            .state
            .local_vars
            .get("grab_calls"),
        Some(&Value::Int(1))
    );
    assert_eq!(
        engine.objects[actor_index]
            .state
            .local_vars
            .get("menu_during_grab"),
        Some(&Value::Int(0)),
        "Grab(false) observes the menu already closed"
    );
    let target_index = engine.test_object_index(target);
    assert_eq!(
        engine.objects[target_index]
            .state
            .local_vars
            .get("grabbed_calls"),
        Some(&Value::Int(1))
    );
    assert_eq!(
        engine.objects[target_index]
            .state
            .local_vars
            .get("menu_during_grabbed"),
        Some(&Value::Int(0)),
        "Grabbed(false) observes the menu already closed"
    );
}

#[test]
fn object_com_ungrab_uses_raw_status_gates_after_grab_callback() {
    let actor_script = r#"#strict
local grab_calls, remove_self;
protected func Grab(object target, bool grab)
{
  grab_calls++;
  if (!grab && remove_self) RemoveObject();
  return(1);
}
"#;
    let target_script = r#"#strict
local grabbed_calls;
protected func Grabbed(object actor, bool grab)
{
  grabbed_calls++;
  return(1);
}
"#;
    let mut actor = test_definition("UGRA", "Pusher", actor_script);
    actor.set_c4_callback_convention(true);
    actor.configure_actions(
        None,
        HashMap::from([
            (
                "Push".to_string(),
                ActionSpec::default().with_procedure("PUSH"),
            ),
            (
                "Walk".to_string(),
                ActionSpec::default().with_procedure("WALK"),
            ),
        ]),
    );
    let mut target = test_definition("UGRT", "Target", target_script);
    target.set_c4_callback_convention(true);
    let mut engine = Engine::new();
    engine.register_test_definition(actor);
    engine.register_test_definition(target);

    let inactive_target =
        engine.spawn_test_object(SpawnConfig::new("UGRT").with_status(ObjectStatus::Inactive));
    let mut push = ActionState::new("Push");
    push.target = Some(inactive_target);
    let first_actor = engine.spawn_test_object(SpawnConfig::new("UGRA").with_action(push));
    let first_actor_index = engine.test_object_index(first_actor);
    assert!(engine
        .object_com_ungrab(first_actor_index)
        .expect("inactive target ungrab succeeds"));
    let first_actor_index = engine.test_object_index(first_actor);
    assert_eq!(engine.objects[first_actor_index].state.action.name, "Walk");
    assert_eq!(
        engine.objects[first_actor_index]
            .state
            .local_vars
            .get("grab_calls"),
        Some(&Value::Int(1))
    );
    let inactive_target_index = engine.test_object_index(inactive_target);
    assert_eq!(
        engine.objects[inactive_target_index]
            .state
            .local_vars
            .get("grabbed_calls"),
        Some(&Value::Int(1)),
        "inactive Status is nonzero and still receives Grabbed(false)"
    );

    let live_target = engine.spawn_test_object(SpawnConfig::new("UGRT"));
    let mut push = ActionState::new("Push");
    push.target = Some(live_target);
    let removing_actor = engine.spawn_test_object(SpawnConfig::new("UGRA").with_action(push));
    let removing_actor_index = engine.test_object_index(removing_actor);
    engine.objects[removing_actor_index]
        .state
        .local_vars
        .insert("remove_self".to_string(), Value::Int(1));
    assert!(engine
        .object_com_ungrab(removing_actor_index)
        .expect("callback-removing ungrab returns"));
    assert_eq!(
        engine
            .object_snapshot(removing_actor)
            .expect("removed actor storage remains")
            .status,
        ObjectStatus::Deleted
    );
    let live_target_index = engine.test_object_index(live_target);
    assert_eq!(
        engine.objects[live_target_index]
            .state
            .local_vars
            .get("grabbed_calls"),
        None,
        "actor Status=0 after Grab(false) suppresses target Grabbed(false)"
    );
}

#[test]
fn script_targeted_throw_runs_turn_action_before_object_com_throw() {
    let script = r#"#strict
local turn_started, throw_saw_turn;
public func RunExecute()
{
  ExecuteCommand();
  return(1);
}
protected func StartTurn()
{
  turn_started = 1;
  return(1);
}
protected func StartThrow()
{
  throw_saw_turn = turn_started;
  return(1);
}
"#;
    let mut actor = test_definition("TTRN", "Turning thrower", script);
    actor.set_c4_callback_convention(true);
    actor.set_shape_rect(Some(DefinitionRect::new(-8, -10, 16, 20)));
    actor.set_physical(PhysicalInfo {
        throw: 50_000,
        ..PhysicalInfo::default()
    });
    actor.configure_actions(
        None,
        HashMap::from([
            (
                "Walk".to_string(),
                ActionSpec::default()
                    .with_procedure("WALK")
                    .with_directions(2)
                    .with_turn_action("Turn"),
            ),
            (
                "Turn".to_string(),
                ActionSpec::default()
                    .with_procedure("WALK")
                    .with_directions(2)
                    .with_start_call("StartTurn"),
            ),
            (
                "Throw".to_string(),
                ActionSpec::default()
                    .with_procedure("THROW")
                    .with_start_call("StartThrow"),
            ),
        ]),
    );
    let mut item = test_definition("TIT2", "Turning throw item", "#strict");
    item.set_collectible(true);

    let mut engine = Engine::with_seed(120);
    let mut landscape = Landscape::flat(200, 100);
    landscape.set_world_height(150);
    engine.set_landscape(landscape);
    engine.register_test_definition(actor);
    engine.register_test_definition(item);
    let actor = engine.spawn_test_object(
        SpawnConfig::new("TTRN")
            .with_position(Vector2::new(99, 99))
            .with_direction(Direction::Left)
            .with_action(ActionState::new("Walk")),
    );
    let item = engine.spawn_test_object(SpawnConfig::new("TIT2").with_container(actor));
    let actor_index = engine.test_object_index(actor);
    crate::TestValueExt::test_value(
        engine.objects[actor_index].commands.push_front(
            CommandRequest::new(CommandId::Throw)
                .with_target(Some(item))
                .with_tx(Some(100))
                .with_ty(Some(70)),
        ),
    );

    crate::TestValueExt::test_value(engine.call_object_function(
        actor_index,
        "RunExecute",
        Vec::new(),
    ));

    let actor_index = engine.test_object_index(actor);
    assert_eq!(
        engine.objects[actor_index].state.direction,
        Direction::Right
    );
    assert_eq!(
        engine.objects[actor_index]
            .state
            .local_vars
            .get("turn_started"),
        Some(&Value::Int(1))
    );
    assert_eq!(
        engine.objects[actor_index]
            .state
            .local_vars
            .get("throw_saw_turn"),
        Some(&Value::Int(1)),
        "SetDir's TurnAction completes before Throw's StartCall"
    );
    assert_eq!(
        engine
            .object_snapshot(item)
            .expect("item remains")
            .container,
        None
    );
    assert!(engine.objects[actor_index].commands.snapshot().is_empty());
}

#[test]
fn object_action_throw_runs_full_exit_bounds_motion_and_callbacks() {
    let actor_script = r#"#strict
protected func Ejection(pObject)
{
  pObject->RecordThrowStep(2);
  return(1);
}
"#;
    let item_script = r#"#strict
local throw_order;
public func RecordThrowStep(iStep)
{
  throw_order = throw_order * 10 + iStep;
  return(1);
}
protected func ContactTop()
{
  RecordThrowStep(1);
  return(0);
}
protected func Departure(pContainer)
{
  RecordThrowStep(3);
  return(1);
}
"#;

    let mut engine = Engine::with_seed(118);
    let mut actor = test_definition("TACT", "Throw actor", actor_script);
    actor.set_c4_callback_convention(true);
    actor.configure_actions(
        Some("Walk".to_string()),
        HashMap::from([
            (
                "Walk".to_string(),
                ActionSpec::default().with_procedure("WALK"),
            ),
            (
                "Throw".to_string(),
                ActionSpec::default().with_procedure("THROW"),
            ),
        ]),
    );
    actor.set_physical(PhysicalInfo {
        throw: 50_000,
        ..PhysicalInfo::default()
    });
    engine.register_test_definition(actor);

    let mut item = test_definition("TITM", "Thrown item", item_script);
    item.set_c4_callback_convention(true);
    item.set_contact_function_calls(true);
    item.set_border_bound(C4D_BORDER_TOP);
    item.set_shape_rect(Some(DefinitionRect::new(0, 0, 4, 4)));
    engine.register_test_definition(item);

    let actor = engine.spawn_test_object(
        SpawnConfig::new("TACT")
            .with_position(Vector2::new(20, 0))
            .with_direction(Direction::Right)
            .with_action(ActionState::new("Walk")),
    );
    let item = engine.spawn_test_object(SpawnConfig::new("TITM").with_container(actor));
    let item_index = engine.test_object_index(item);
    engine.objects[item_index].state.shape_override = Some(DefinitionRect::new(0, 0, 6, 6));
    engine.objects[item_index].shape_rect = Some(DefinitionRect::new(0, 0, 6, 6));
    engine.objects[item_index].fixed_velocity = FixedVec2::new(itofix(7), itofix(-9));

    assert!(engine
        .try_object_action_throw(actor, item)
        .expect("ObjectActionThrow succeeds"));

    let force = math::val_by_physical(400, 50_000);
    let item_index = engine.test_object_index(item);
    let item = &engine.objects[item_index];
    assert_eq!(item.state.container, None);
    assert_eq!(item.state.position, Vector2::new(20, 0));
    assert_eq!(item.fixed_velocity, FixedVec2::new(force, -force));
    assert_eq!(item.rotation_velocity, force);
    assert_eq!(item.state.shape_override, None);
    assert_eq!(
        item.state.local_vars.get("throw_order"),
        Some(&Value::Int(123)),
        "BoundsCheck ContactTop runs before Ejection and Departure"
    );
}

#[test]
fn nested_empty_put_take_runs_reject_contents_and_opens_menu_before_return() {
    let mut engine = put_fixture_engine();
    let mut new_item = test_definition("NITM", "New menu item", "#strict");
    new_item.set_category(CATEGORY_STRUCTURE);
    engine.register_test_definition(new_item);
    let target = engine.spawn_test_object(SpawnConfig::new("TARG"));
    let actor = engine.spawn_test_object(
        SpawnConfig::new("ACTR")
            .with_container(target)
            .with_category(CATEGORY_OBJECT),
    );
    let item = engine.spawn_test_object(SpawnConfig::new("ITEM").with_container(actor));
    let actor_index = engine.test_object_index(actor);
    engine.objects[actor_index].state.local_vars.extend([
        ("tracked".to_string(), object_reference_value(item)),
        ("nested_throw".to_string(), Value::Int(1)),
        ("mutate_menu_during_value".to_string(), Value::Int(1)),
        ("spawn_menu_item".to_string(), Value::Int(1)),
    ]);
    let target_index = engine.test_object_index(target);
    engine.objects[target_index].state.local_vars.insert(
        "reject_contents_actor".to_string(),
        object_reference_value(actor),
    );
    crate::TestValueExt::test_value(
        engine.objects[actor_index].commands.push_front(
            CommandRequest::new(CommandId::Put)
                .with_target(Some(target))
                .with_target2(Some(item)),
        ),
    );

    crate::TestValueExt::test_value(engine.execute_object_command_now(actor));

    let actor_index = engine.test_object_index(actor);
    let actor_state = &engine.objects[actor_index].state;
    assert_eq!(
        actor_state.local_vars.get("reject_contents_seen"),
        Some(&Value::Int(1)),
        "RejectContents runs inside nested ExecuteCommand"
    );
    assert_eq!(
        actor_state.local_vars.get("menu_after_nested"),
        Some(&Value::Int(6)),
        "GetMenu observes C4MN_Activate before ExecuteCommand returns"
    );
    assert_eq!(
        actor_state.local_vars.get("menu_selection_after_nested"),
        Some(&Value::Int(0)),
        "the activate menu has already refilled and selected its first row"
    );
    assert_eq!(
        actor_state.local_vars.get("menu_during_value"),
        Some(&Value::Int(6)),
        "CalcValue sees the already-installed Activate menu during refill"
    );
    assert_eq!(
        actor_state.local_vars.get("menu_selection_during_value"),
        Some(&Value::Int(-1)),
        "selection adjustment stays frozen until the full refill completes"
    );
    let menu = crate::TestValueExt::test_value(actor_state.menu.as_ref());
    assert_eq!(menu.identification, Value::Int(6));
    assert_eq!(menu.refill_object, Some(target));
    assert_eq!((menu.columns, menu.lines), (3, 4));
    assert_eq!(
        menu.items
            .iter()
            .find(|entry| entry.item_id == "ACTR")
            .and_then(|entry| entry.value),
        Some(7)
    );
    assert!(
        menu.items.iter().any(|entry| entry.item_id == "NITM"),
        "an item entered after the iterator's current row is included in the same refill"
    );
    assert_eq!(
        engine.objects[actor_index]
            .commands
            .snapshot()
            .command_names(),
        ["Put"],
        "the nested Throw finished without disturbing the outer Put"
    );
}

#[test]
fn object_com_put_without_grab_put_drops_with_full_physics_only_when_down_double_is_armed() {
    for (down_double, should_drop) in [(0, false), (-3, true)] {
        let mut engine = put_fixture_engine();
        let actor_definition = crate::TestValueExt::test_value(engine.definitions.get_mut("ACTR"));
        actor_definition.set_shape_rect(Some(DefinitionRect::new(-8, -10, 16, 20)));
        actor_definition.set_physical(PhysicalInfo {
            throw: 50_000,
            ..PhysicalInfo::default()
        });
        crate::TestValueExt::test_value(engine.definitions.get_mut("ITEM"))
            .set_shape_rect(Some(DefinitionRect::new(0, 0, 4, 4)));
        engine.register_test_script_definition("NOPU", "No put", "#strict");
        engine.register_test_player(PlayerConfig::new(1, "PutTake owner"));
        crate::TestValueExt::test_value(engine.player_mut(1))
            .control
            .last_com_down_double = down_double;
        let target = engine.spawn_test_object(SpawnConfig::new("NOPU"));
        let mut push = ActionState::new("Push");
        push.target = Some(target);
        let actor = engine.spawn_test_object(
            SpawnConfig::new("ACTR")
                .with_owner(1)
                .with_position(Vector2::new(20, 40))
                .with_velocity(Vector2::new(-2, 0))
                .with_command_direction(CommandDirection::Right)
                .with_action(push),
        );
        let item = engine.spawn_test_object(SpawnConfig::new("ITEM").with_container(actor));
        assert_eq!(
            engine
                .try_object_com_put(actor, target, item)
                .expect("ObjectComPut attempt executes"),
            should_drop,
            "LastComDownDouble={down_double}"
        );

        let actor_index = engine.test_object_index(actor);
        let item_index = engine.test_object_index(item);
        if should_drop {
            let force = math::val_by_physical(400, 50_000);
            assert_eq!(engine.objects[item_index].state.container, None);
            assert_eq!(
                engine.objects[item_index].state.position,
                Vector2::new(
                    engine.objects[actor_index].state.position.x + 8,
                    engine.objects[actor_index].state.position.y + 6,
                ),
                "drop uses the live actor/item shapes for its exit position"
            );
            assert_eq!(
                engine.objects[item_index].fixed_velocity,
                FixedVec2::new(force, C4Fixed::ZERO),
                "rightward drop applies the physical throw force"
            );
            assert_eq!(engine.objects[actor_index].state.no_collect_delay, 2);
            assert_eq!(engine.objects[actor_index].state.action.name, "Walk");
            assert_eq!(
                engine.objects[actor_index]
                    .state
                    .local_vars
                    .get("departure_seen"),
                Some(&Value::Int(1)),
                "the drop runs the item's Departure callback"
            );
        } else {
            assert_eq!(engine.objects[item_index].state.container, Some(actor));
            assert_eq!(engine.objects[actor_index].state.no_collect_delay, 0);
            assert_eq!(engine.objects[actor_index].state.action.name, "Push");
            assert_eq!(
                engine.objects[actor_index]
                    .state
                    .local_vars
                    .get("departure_seen"),
                None,
                "a disarmed failed put must not begin the drop callback sequence"
            );
        }
    }
}

#[test]
fn empty_contained_throw_and_drop_open_the_activate_menu_before_finishing() {
    for command in [CommandId::Throw, CommandId::Drop] {
        let mut engine = put_fixture_engine();
        let target = engine.spawn_test_object(SpawnConfig::new("TARG"));
        let actor = engine.spawn_test_object(SpawnConfig::new("ACTR").with_container(target));
        let actor_index = engine.test_object_index(actor);
        crate::TestValueExt::test_value(
            engine.objects[actor_index]
                .commands
                .push_front(CommandRequest::new(command)),
        );

        crate::TestValueExt::test_value(engine.execute_object_command_now(actor));

        let actor_index = engine.test_object_index(actor);
        let actor_state = &engine.objects[actor_index].state;
        let menu = crate::TestValueExt::test_value(actor_state.menu.as_ref());
        assert_eq!(menu.identification, Value::Int(6));
        assert_eq!(menu.refill_object, Some(target));
        assert!(
            engine.objects[actor_index].commands.snapshot().is_empty(),
            "{command:?} finishes after opening the menu"
        );
    }
}

#[test]
fn empty_pushing_throw_and_drop_open_get_only_for_grab_get_targets() {
    for command in [CommandId::Throw, CommandId::Drop] {
        let mut engine = put_fixture_engine();
        let mut target_definition = test_definition("GETT", "Get target", "#strict");
        target_definition.set_grab_put_get(GRAB_PUT_GET_GET);
        engine.register_test_definition(target_definition);
        let target = engine.spawn_test_object(SpawnConfig::new("GETT"));
        let mut push = ActionState::new("Push");
        push.target = Some(target);
        let actor = engine.spawn_test_object(SpawnConfig::new("ACTR").with_action(push));
        let actor_index = engine.test_object_index(actor);
        crate::TestValueExt::test_value(
            engine.objects[actor_index]
                .commands
                .push_front(CommandRequest::new(command)),
        );

        crate::TestValueExt::test_value(engine.execute_object_command_now(actor));

        let actor_index = engine.test_object_index(actor);
        let menu = crate::TestValueExt::test_value(engine.objects[actor_index].state.menu.as_ref());
        assert_eq!(menu.identification, Value::Int(13));
        assert_eq!(menu.refill_object, Some(target));
        assert!(engine.objects[actor_index].commands.snapshot().is_empty());

        let mut denied = put_fixture_engine();
        let target = denied.spawn_test_object(SpawnConfig::new("TARG"));
        let mut push = ActionState::new("Push");
        push.target = Some(target);
        let actor = denied.spawn_test_object(SpawnConfig::new("ACTR").with_action(push));
        let actor_index = denied.test_object_index(actor);
        crate::TestValueExt::test_value(
            denied.objects[actor_index]
                .commands
                .push_front(CommandRequest::new(command)),
        );
        crate::TestValueExt::test_value(denied.execute_object_command_now(actor));
        let actor_index = denied.test_object_index(actor);
        assert!(denied.objects[actor_index].state.menu.is_none());
        assert!(denied.objects[actor_index].commands.snapshot().is_empty());
    }
}
