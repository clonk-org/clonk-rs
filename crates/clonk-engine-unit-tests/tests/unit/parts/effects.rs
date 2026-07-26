    // FnGetCommand (C4Script.cpp:918-945): element 0 returns the C++
    // CommandName string of the requested stack entry; without commands
    // the call yields nil (never an error).
    #[test]
    fn get_command_returns_command_name_like_cpp() {
        let script = r#"
        global func Initialize(state, random) { return 0; }
        global func Step(state, frame, random) { return 0; }
        global func Arm() { SetCommand(this(), "Wait", 0, 0, 0, 0, 50); return 1; }
        global func Ask() { return GetCommand(); }
        "#;
        let mut engine = Engine::with_seed(9);
        engine
            .register_definition(
                Definition::from_script("Actor", "Actor", script).expect("script compiles"),
            )
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("Actor"))
            .expect("spawn succeeds");
        let idx = engine.find_object_index(id).expect("object exists");

        let before = engine
            .call_object_function(idx, "Ask", Vec::new())
            .expect("ask runs");
        assert_eq!(before, Value::Nil, "no command -> nil (C4Script.cpp:926)");

        engine
            .call_object_function(idx, "Arm", Vec::new())
            .expect("arm runs");
        let idx = engine.find_object_index(id).expect("object exists");
        let after = engine
            .call_object_function(idx, "Ask", Vec::new())
            .expect("ask runs");
        assert_eq!(
            after,
            Value::String("Wait".to_string().into()),
            "element 0 is the CommandName string (C4Script.cpp:931)"
        );
    }

    // C++ runs Fx* callbacks with fPassErrors=false: a script error in an
    // effect timer logs and the game continues — it must not kill the
    // simulation tick (the GoldRush bandit AI errored every FxTimer before
    // GetCommand existed and C++ kept running).
    #[test]
    fn effect_callback_script_error_is_fail_safe_like_cpp() {
        let script = r#"#strict 3
        global func Initialize(state, random) {
            return { effects = [ { op = "add", name = "Broken", interval = 1 } ] };
        }

        global func FxBrokenTimer(state, effect, timer) {
            return ThisHostFunctionDoesNotExist();
        }

        global func Step(state, frame, random) {
            return nil;
        }
        "#;
        let mut engine = Engine::with_seed(11);
        engine
            .register_definition(
                Definition::from_script("Actor", "Actor", script).expect("script compiles"),
            )
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("Actor"))
            .expect("spawn succeeds");

        for frame in 1..=3 {
            engine
                .tick_without_snapshot()
                .unwrap_or_else(|err| panic!("tick {frame} must survive the Fx error: {err}"));
        }
        // The erroring callback yields nil each interval — the effect is
        // NOT killed (C++ gets 0 back, not C4Fx_Execute_Kill).
        let idx = engine.find_object_index(id).expect("object exists");
        assert!(
            engine.objects[idx]
                .state
                .effects
                .iter()
                .any(|effect| effect.name == "Broken"),
            "the erroring effect stays installed"
        );
    }

    // Effect callbacks run through the same C4AulExec as any other call:
    // an arrow call onto ANOTHER object mutates that object for real —
    // C4Effect::Execute (C4Effect.cpp:342-360) does not sandbox nested
    // targets. GoldRush's f30 rifle load depends on it: FxOrderDefendTimer
    // calls WINC::ControlThrow, which ends in the rifle's own
    // RemoveObject() (Winchester.c4d/Script.c:29).
    #[test]
    fn effect_timer_nested_call_mutates_the_foreign_object() {
        let holder_script = r#"#strict
local iGot;
public func Boot() { AddEffect("Probe", this(), 1, 5, this()); return(1); }
func FxProbeTimer(pThis, iNumber) {
  var pItem = FindContents(ITEM);
  if (pItem) iGot = pItem->Consume();
  return(-1);
}
"#;
        let item_script = r#"#strict
public func Consume() { RemoveObject(); return(7); }
"#;
        let mut engine = Engine::with_seed(3);
        engine
            .register_definition(
                Definition::from_script("HOLD", "Holder", holder_script)
                    .expect("holder compiles"),
            )
            .expect("holder registers");
        engine
            .register_definition(
                Definition::from_script("ITEM", "Item", item_script).expect("item compiles"),
            )
            .expect("item registers");

        let holder = engine
            .spawn_object(SpawnConfig::new("HOLD").with_category(CATEGORY_OBJECT))
            .expect("holder spawns");
        let item = engine
            .spawn_object(
                SpawnConfig::new("ITEM")
                    .with_category(CATEGORY_OBJECT)
                    .with_container(holder),
            )
            .expect("item spawns");
        let idx = engine.find_object_index(holder).expect("holder exists");
        engine
            .call_object_function(idx, "Boot", Vec::new())
            .expect("boot runs");

        for _ in 0..6 {
            engine.tick_without_snapshot().expect("tick");
        }

        let idx = engine.find_object_index(holder).expect("holder exists");
        assert_eq!(
            engine.objects[idx].state.local_vars.get("iGot"),
            Some(&Value::Int(7)),
            "the nested call's return value reaches the effect callback"
        );
        assert!(
            engine.find_object_index(item).is_none(),
            "the foreign object's self-RemoveObject inside the effect \
             timer's nested call folds (C4Effect.cpp:342-360 exec)"
        );
    }

    // C++ mutates the live object mid-call: after a nested callback on
    // the suspended caller changes its action, a foreign GetAction /
    // GetPhase read sees the NEW values immediately. The GoldRush rifle
    // chain depends on this: WINC::CheckAmmo gates on
    // `GetAction(pClonk) ne "AimRifle"` right after FireRifle's
    // SetAction("AimRifle")+SetPhase(6) (Winchester.c4d/Script.c:292,
    // Cowboy.c4d/Script.c:442-443).
    #[test]
    fn foreign_action_reads_see_the_suspended_scopes_pending_action() {
        let holder_script = r#"#strict
local sSeen, iSeenPhase;
public func Boot() { AddEffect("Probe", this(), 1, 5, this()); return(1); }
func FxProbeTimer(pThis, iNumber) {
  var pItem = FindContents(ITEM);
  if (pItem) pItem->Poke(this());
  return(-1);
}
public func Rise() { SetAction("Rise"); SetPhase(6); return(1); }
"#;
        let item_script = r#"#strict
public func Poke(pClonk) {
  pClonk->~Rise();
  LocalN("sSeen", pClonk) = GetAction(pClonk);
  LocalN("iSeenPhase", pClonk) = GetPhase(pClonk);
  return(1);
}
"#;
        let mut engine = Engine::with_seed(3);
        let mut holder =
            Definition::from_script("HOLD", "Holder", holder_script).expect("holder compiles");
        holder.configure_actions(
            None,
            HashMap::from([("Rise".to_string(), ActionSpec::default().with_length(10))]),
        );
        engine.register_definition(holder).expect("holder registers");
        engine
            .register_definition(
                Definition::from_script("ITEM", "Item", item_script).expect("item compiles"),
            )
            .expect("item registers");

        let holder_id = engine
            .spawn_object(SpawnConfig::new("HOLD").with_category(CATEGORY_OBJECT))
            .expect("holder spawns");
        engine
            .spawn_object(
                SpawnConfig::new("ITEM")
                    .with_category(CATEGORY_OBJECT)
                    .with_container(holder_id),
            )
            .expect("item spawns");
        let idx = engine.find_object_index(holder_id).expect("holder exists");
        engine
            .call_object_function(idx, "Boot", Vec::new())
            .expect("boot runs");

        for _ in 0..6 {
            engine.tick_without_snapshot().expect("tick");
        }

        let idx = engine.find_object_index(holder_id).expect("holder exists");
        assert_eq!(
            engine.objects[idx].state.local_vars.get("sSeen"),
            Some(&Value::String("Rise".to_string().into())),
            "GetAction(pTarget) reads the in-flight action (C++ live state)"
        );
        assert_eq!(
            engine.objects[idx].state.local_vars.get("iSeenPhase"),
            Some(&Value::Int(6)),
            "GetPhase(pTarget) reads the in-flight phase (C++ live state)"
        );
    }

    // One live object (C4AulExec): the outer effect callback's own local
    // writes, a nested call's write-back to the caller, and a subsequent
    // outer READ of that write-back all see the same storage. GoldRush's
    // FxOrderDefendTimer writes pOrdrTarget around the nested
    // WINC::ControlThrow chain (Cowboy.c4d/Script.c:641-669).
    #[test]
    fn outer_effect_locals_and_nested_write_backs_share_live_storage() {
        let holder_script = r#"#strict
local iBefore, iFromItem, iAfter;
public func Boot() { AddEffect("Probe", this(), 1, 5, this()); return(1); }
func FxProbeTimer(pThis, iNumber) {
  iBefore = 1;
  var pItem = FindContents(ITEM);
  if (pItem) pItem->Tag(this());
  iAfter = iFromItem + 1;
  return(-1);
}
"#;
        let item_script = r#"#strict
public func Tag(pClonk) { LocalN("iFromItem", pClonk) = 7; return(1); }
"#;
        let mut engine = Engine::with_seed(3);
        engine
            .register_definition(
                Definition::from_script("HOLD", "Holder", holder_script)
                    .expect("holder compiles"),
            )
            .expect("holder registers");
        engine
            .register_definition(
                Definition::from_script("ITEM", "Item", item_script).expect("item compiles"),
            )
            .expect("item registers");

        let holder = engine
            .spawn_object(SpawnConfig::new("HOLD").with_category(CATEGORY_OBJECT))
            .expect("holder spawns");
        engine
            .spawn_object(
                SpawnConfig::new("ITEM")
                    .with_category(CATEGORY_OBJECT)
                    .with_container(holder),
            )
            .expect("item spawns");
        let idx = engine.find_object_index(holder).expect("holder exists");
        engine
            .call_object_function(idx, "Boot", Vec::new())
            .expect("boot runs");

        for _ in 0..6 {
            engine.tick_without_snapshot().expect("tick");
        }

        let idx = engine.find_object_index(holder).expect("holder exists");
        let locals = &engine.objects[idx].state.local_vars;
        assert_eq!(
            locals.get("iBefore"),
            Some(&Value::Int(1)),
            "the outer callback's own pre-nested write persists"
        );
        assert_eq!(
            locals.get("iFromItem"),
            Some(&Value::Int(7)),
            "the nested call's write-back to the caller persists"
        );
        assert_eq!(
            locals.get("iAfter"),
            Some(&Value::Int(8)),
            "the outer callback READS the nested write-back live \
             (C++ mutates the one live object mid-call)"
        );
    }

    // FnGetDir (C4Script.cpp:1118-1122): `if (!pObj) pObj = cthr->Obj;
    // if (!pObj) return {};` — the context object is only the DEFAULT; an
    // explicit target needs NO object context. GoldRush reads it from a
    // DEFINITION call: WINC->ActualizePhase(pClonk) computes the
    // crosshair vertex sign from GetDir(pClonk)
    // (Winchester.c4d/Script.c:118-121) — the f30 live wall showed every
    // rust crosshair at x=+40 because the Nil bail made iDir -1 for
    // Right-facing bandits too.
    #[test]
    fn foreign_get_dir_works_without_an_object_context() {
        let actor_script = r#"#strict
local iDir;
public func Boot() {
  iDir = HELP->ReadDir(this());
  return(1);
}
"#;
        let helper_script = r#"#strict
public func ReadDir(pClonk) { return(GetDir(pClonk)); }
"#;
        let mut engine = Engine::with_seed(3);
        engine
            .register_definition(
                Definition::from_script("Actr", "Actor", actor_script).expect("actor compiles"),
            )
            .expect("actor registers");
        engine
            .register_definition(
                Definition::from_script("HELP", "Helper", helper_script)
                    .expect("helper compiles"),
            )
            .expect("helper registers");
        let id = engine
            .spawn_object(
                SpawnConfig::new("Actr")
                    .with_category(CATEGORY_OBJECT)
                    .with_direction(Direction::Right),
            )
            .expect("actor spawns");
        let idx = engine.find_object_index(id).expect("actor exists");
        engine
            .call_object_function(idx, "Boot", Vec::new())
            .expect("boot runs");

        let idx = engine.find_object_index(id).expect("actor exists");
        assert_eq!(
            engine.objects[idx].state.local_vars.get("iDir"),
            Some(&Value::Int(1)),
            "GetDir(pObj) resolves the explicit target even from a \
             definition-call scope (C4Script.cpp:1120)"
        );
    }

    // C4Object::Enter (C4Object.cpp:1552-1612): a transfer EXITS first
    // (`if (Contained) if (!Exit(x, y))`, :1579) — and Exit mobilizes
    // (`Mobile = 1; InLiquid = 0;`, :1540-1541) — then fCopyMotion
    // (default true, C4Object.h:313) copies the NEW container's motion
    // IMMEDIATELY (:1598-1606, "so the position will be correct when OCF
    // is set"). GoldRush f60: AimAgain transfers the fresh CSHO from the
    // bandit into the WCHR crosshair (Cowboy.c4d/Script.c:270-273) — cpp
    // reports them at the crosshair's position with Mobile=1 the same
    // frame; rust left them at the bandit's spot with mobile=false.
    #[test]
    fn enter_transfer_mobilizes_and_copies_the_containers_motion_like_cpp() {
        let holder_script = r#"#strict
public func Stash(pItem, pBox) { Enter(pBox, pItem); return(1); }
"#;
        let mut engine = Engine::with_seed(3);
        engine
            .register_definition(
                Definition::from_script("HOLD", "Holder", holder_script)
                    .expect("holder compiles"),
            )
            .expect("holder registers");
        engine
            .register_definition(simple_definition("ITEM"))
            .expect("item registers");
        engine
            .register_definition(simple_definition("BOXX"))
            .expect("box registers");

        let holder = engine
            .spawn_object(
                SpawnConfig::new("HOLD")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(100, 50)),
            )
            .expect("holder spawns");
        let item = engine
            .spawn_object(
                SpawnConfig::new("ITEM")
                    .with_category(CATEGORY_OBJECT)
                    .with_container(holder),
            )
            .expect("item spawns");
        let idx = engine.find_object_index(item).expect("item exists");
        assert!(
            !engine.objects[idx].state.mobile,
            "a FIRST Enter (CreateContents birth) has no Exit — Mobile \
             stays 0 (C4Object::Init, C4Object.cpp:182-185)"
        );
        let boxx = engine
            .spawn_object(
                SpawnConfig::new("BOXX")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(300, 80)),
            )
            .expect("box spawns");

        let holder_idx = engine.find_object_index(holder).expect("holder exists");
        engine
            .call_object_function(
                holder_idx,
                "Stash",
                vec![
                    Value::Object(item.as_u64()),
                    Value::Object(boxx.as_u64()),
                ],
            )
            .expect("stash runs");

        let idx = engine.find_object_index(item).expect("item exists");
        assert_eq!(
            engine.objects[idx].state.container,
            Some(boxx),
            "the transfer landed"
        );
        assert!(
            engine.objects[idx].state.mobile,
            "the transfer's internal Exit mobilizes (C4Object.cpp:1540)"
        );
        assert_eq!(
            engine.objects[idx].state.position,
            Vector2::new(300, 80),
            "fCopyMotion snaps the position to the NEW container \
             immediately (C4Object.cpp:1598-1606)"
        );
    }

    #[test]
    fn script_enter_runs_the_cpp_veto_transfer_and_callback_pipeline() -> Result<(), EngineError> {
        let driver_script = r#"#strict
public func Put(pTarget, pObject) { return(Enter(pTarget, pObject)); }
"#;
        let item_script = r#"#strict
local callback_order, entrance_target, departure_target, shadow_called;

// A foreign Enter(target, object) must not redispatch by name on object.
public func Enter(pTarget) { shadow_called = 1; return(0); }

public func Mark(iStep)
{
  callback_order = callback_order * 10 + iStep;
  return(1);
}

protected func RejectEntrance(pTarget)
{
  return(GetID(pTarget) == DENY);
}

protected func Departure(pOldContainer)
{
  Mark(2);
  departure_target = pOldContainer;
  return(1);
}

protected func Entrance(pContainer)
{
  Mark(4);
  entrance_target = pContainer;
  return(1);
}
"#;
        let old_script = r#"#strict
protected func Ejection(pObject) { pObject->Mark(1); return(1); }
"#;
        let new_script = r#"#strict
protected func Collection2(pObject) { pObject->Mark(3); return(1); }
"#;
        let self_veto_script = r#"#strict
public func TryEnter(pTarget) { return(Enter(pTarget)); }
protected func RejectEntrance(pTarget) { return(1); }
"#;

        let mut engine = Engine::with_seed(3);
        engine.register_definition(Definition::from_script("DRV1", "Driver", driver_script)?)?;
        let mut item = Definition::from_script("ITEM", "Item", item_script)?;
        item.set_c4_callback_convention(true);
        engine.register_definition(item)?;
        let mut old = Definition::from_script("OLD1", "Old", old_script)?;
        old.set_c4_callback_convention(true);
        engine.register_definition(old)?;
        let mut new = Definition::from_script("NEW1", "New", new_script)?;
        new.set_c4_callback_convention(true);
        engine.register_definition(new)?;
        engine.register_definition(simple_definition("DENY"))?;
        let mut self_veto = Definition::from_script("VETO", "Veto", self_veto_script)?;
        self_veto.set_c4_callback_convention(true);
        engine.register_definition(self_veto)?;

        let driver = engine.spawn_object(SpawnConfig::new("DRV1"))?;
        let old = engine.spawn_object(SpawnConfig::new("OLD1"))?;
        let new = engine.spawn_object(SpawnConfig::new("NEW1").with_controller(7))?;
        let deny = engine.spawn_object(SpawnConfig::new("DENY"))?;
        let self_veto = engine.spawn_object(SpawnConfig::new("VETO"))?;
        let item = engine.spawn_object(
            SpawnConfig::new("ITEM")
                .with_container(old)
                .with_controller(2),
        )?;
        let driver_index = engine.find_object_index(driver).expect("driver exists");

        let self_veto_index = engine
            .find_object_index(self_veto)
            .expect("self-veto object exists");
        assert_eq!(
            engine.call_object_function(
                self_veto_index,
                "TryEnter",
                vec![object_reference_value(new)],
            )?,
            Value::Bool(false),
            "one-argument Enter defaults the subject to the calling object"
        );
        let self_veto_index = engine
            .find_object_index(self_veto)
            .expect("self-veto object exists");
        assert_eq!(engine.objects[self_veto_index].state.container, None);

        assert_eq!(
            engine.call_object_function(
                driver_index,
                "Put",
                vec![object_reference_value(deny), object_reference_value(item)],
            )?,
            Value::Bool(false),
            "RejectEntrance vetoes before the old containment is changed"
        );
        let item_index = engine.find_object_index(item).expect("item exists");
        assert_eq!(engine.objects[item_index].state.container, Some(old));
        assert_eq!(
            engine.objects[item_index]
                .state
                .local_vars
                .get("callback_order"),
            Some(&Value::Nil),
            "a veto fires no transfer or entry callbacks"
        );

        let driver_index = engine.find_object_index(driver).expect("driver exists");
        assert_eq!(
            engine.call_object_function(
                driver_index,
                "Put",
                vec![object_reference_value(new), object_reference_value(item)],
            )?,
            Value::Bool(true)
        );
        let item_index = engine.find_object_index(item).expect("item exists");
        let item_state = &engine.objects[item_index].state;
        assert_eq!(item_state.container, Some(new));
        assert_eq!(item_state.controller, 7, "nonliving entrants adopt control");
        assert_eq!(
            item_state.local_vars.get("callback_order"),
            Some(&Value::Int(1234)),
            "Ejection -> Departure -> Collection2 -> Entrance"
        );
        assert_eq!(
            item_state.local_vars.get("departure_target"),
            Some(&object_reference_value(old))
        );
        assert_eq!(
            item_state.local_vars.get("entrance_target"),
            Some(&object_reference_value(new))
        );
        assert_eq!(
            item_state.local_vars.get("shadow_called"),
            Some(&Value::Nil),
            "the explicit subject's script function named Enter is not called"
        );
        Ok(())
    }

    #[test]
    fn script_enter_uses_the_post_collection_container_and_fails_quietly_on_cycles(
    ) -> Result<(), EngineError> {
        let driver_script = r#"#strict
public func Put(pTarget, pObject) { return(Enter(pTarget, pObject)); }
"#;
        let item_script = r#"#strict
local entrance_count, entrance_target;

public func Enter(pTarget) { return(0); }
protected func RejectEntrance(pTarget) { return(0); }
protected func Entrance(pContainer)
{
  entrance_count += 1;
  entrance_target = pContainer;
  return(1);
}
"#;
        let redirect_script = r#"#strict
local destination;
public func Configure(pDestination) { destination = pDestination; return(1); }
protected func Collection2(pObject)
{
  Enter(destination, pObject);
  return(1);
}
"#;

        let mut engine = Engine::with_seed(3);
        engine.register_definition(Definition::from_script("DRV1", "Driver", driver_script)?)?;
        let mut item = Definition::from_script("ITEM", "Item", item_script)?;
        item.set_c4_callback_convention(true);
        engine.register_definition(item)?;
        let mut redirect = Definition::from_script("RDIR", "Redirect", redirect_script)?;
        redirect.set_c4_callback_convention(true);
        engine.register_definition(redirect)?;
        engine.register_definition(simple_definition("DEST"))?;
        engine.register_definition(simple_definition("CHLD"))?;

        let driver = engine.spawn_object(SpawnConfig::new("DRV1"))?;
        let redirect = engine.spawn_object(SpawnConfig::new("RDIR"))?;
        let destination = engine.spawn_object(SpawnConfig::new("DEST"))?;
        let item = engine.spawn_object(SpawnConfig::new("ITEM"))?;
        let redirect_index = engine
            .find_object_index(redirect)
            .expect("redirect exists");
        engine.call_object_function(
            redirect_index,
            "Configure",
            vec![object_reference_value(destination)],
        )?;

        let driver_index = engine.find_object_index(driver).expect("driver exists");
        assert_eq!(
            engine.call_object_function(
                driver_index,
                "Put",
                vec![
                    object_reference_value(redirect),
                    object_reference_value(item),
                ],
            )?,
            Value::Bool(true),
            "a callback-driven move after the initial link does not undo Enter's success"
        );
        let item_index = engine.find_object_index(item).expect("item exists");
        let item_state = &engine.objects[item_index].state;
        assert_eq!(item_state.container, Some(destination));
        assert_eq!(
            item_state.local_vars.get("entrance_target"),
            Some(&object_reference_value(destination)),
            "the outer Entrance callback receives the container left by Collection2"
        );
        assert_eq!(
            item_state.local_vars.get("entrance_count"),
            Some(&Value::Int(2)),
            "the nested Enter and then the original Enter each run Entrance"
        );

        let parent = engine.spawn_object(SpawnConfig::new("ITEM"))?;
        let child = engine.spawn_object(SpawnConfig::new("CHLD").with_container(parent))?;
        let driver_index = engine.find_object_index(driver).expect("driver exists");
        assert_eq!(
            engine.call_object_function(
                driver_index,
                "Put",
                vec![object_reference_value(child), object_reference_value(parent)],
            )?,
            Value::Bool(false),
            "a containment cycle is a quiet false"
        );
        let parent_index = engine.find_object_index(parent).expect("parent exists");
        let child_index = engine.find_object_index(child).expect("child exists");
        assert_eq!(engine.objects[parent_index].state.container, None);
        assert_eq!(engine.objects[child_index].state.container, Some(parent));

        let deleted = engine.spawn_object(SpawnConfig::new("DEST"))?;
        engine.apply_object_update(
            deleted,
            ObjectUpdate::new().with_status(ObjectStatus::Deleted),
        )?;
        let driver_index = engine.find_object_index(driver).expect("driver exists");
        assert_eq!(
            engine.call_object_function(
                driver_index,
                "Put",
                vec![object_reference_value(deleted), object_reference_value(child)],
            )?,
            Value::Bool(false),
            "a deleted target returns false after the entering object exits its old container"
        );
        let child_index = engine.find_object_index(child).expect("child remains");
        assert_eq!(
            engine.objects[child_index].state.container,
            None,
            "C4Object::Enter delays its raw Status gate until after Exit(x,y)"
        );
        Ok(())
    }

    // FnLocal returns a live reference (C4Script.cpp:3423-3433): a write
    // to a JUST-CREATED object's numbered slot persists into later frames.
    // GoldRush: WINC::ControlThrow creates the WCHR crosshair and stores
    // the aim angle in its slot 0 (`Local(0, GetCrosshair(pClonk)) = 84`,
    // Winchester.c4d/Script.c:18-19); thirty frames later ExecuteWatch
    // re-reads it for the vertex rewrite
    // (`Local(0,obj)`, Cowboy.c4d/Script.c:699-700) — the f60 live wall
    // showed rust reading nil there (Sin(0,40)=0 flattened the crosshair
    // offset to the owner's x).
    #[test]
    fn foreign_numbered_local_write_to_a_pending_object_persists() {
        let script = r#"#strict
local pCross;
local iGot;
public func Boot() { AddEffect("Probe", this(), 1, 5, this()); return(1); }
func FxProbeTimer(pThis, iNumber) {
  if (!pCross) {
    var pItem = FindContents(ITEM);
    if (pItem) pItem->Make(this());
    return(1);
  }
  iGot = Local(0, pCross);
  return(-1);
}
public func TakeCross(pObj) { pCross = pObj; return(1); }
"#;
        // The rifle shape: a NESTED call on a contained item creates the
        // marker, writes its slot 0 and removes itself (WINC::ControlThrow,
        // Winchester.c4d/Script.c:18-29).
        let item_script = r#"#strict
public func Make(pClonk) {
  var pCross = CreateObject(MARK, 0, 0, -1);
  Local(0, pCross) = 84;
  pClonk->TakeCross(pCross);
  RemoveObject();
  return(1);
}
"#;
        let mut engine = Engine::with_seed(3);
        engine
            .register_definition(
                Definition::from_script("Actr", "Actor", script).expect("script compiles"),
            )
            .expect("actor registers");
        engine
            .register_definition(
                Definition::from_script("ITEM", "Item", item_script).expect("item compiles"),
            )
            .expect("item registers");
        engine
            .register_definition(simple_definition("MARK"))
            .expect("marker registers");
        let id = engine
            .spawn_object(SpawnConfig::new("Actr").with_category(CATEGORY_OBJECT))
            .expect("actor spawns");
        engine
            .spawn_object(
                SpawnConfig::new("ITEM")
                    .with_category(CATEGORY_OBJECT)
                    .with_container(id),
            )
            .expect("item spawns");
        let idx = engine.find_object_index(id).expect("actor exists");
        engine
            .call_object_function(idx, "Boot", Vec::new())
            .expect("boot runs");

        for _ in 0..12 {
            engine.tick_without_snapshot().expect("tick");
        }

        let marker_idx = engine
            .objects
            .iter()
            .position(|object| object.definition_id == "MARK")
            .expect("marker exists");
        assert_eq!(
            engine.objects[marker_idx].state.local_vars.get("__local_0"),
            Some(&Value::Int(84)),
            "the write to the pending object's slot 0 landed on the \
             materialized object"
        );
        let idx = engine.find_object_index(id).expect("actor exists");
        assert_eq!(
            engine.objects[idx].state.local_vars.get("iGot"),
            Some(&Value::Int(84)),
            "a later callback reads the stored slot back \
             (FnLocal by-reference, C4Script.cpp:3423-3433)"
        );
    }

    // C4Object::Execute order (C4Object.cpp:1069-1090): ExecuteCommand ->
    // ExecAction -> ExecMovement -> particles -> pEffects->Execute -> …
    // Effect timers run AFTER the frame's action exec, so an action set
    // INSIDE a timer callback gets its first PhaseDelay increment the
    // NEXT frame (PhaseDelay += 1; advance at >= Delay,
    // C4Object.cpp:5458-5466). GoldRush f32 wall: the bandits' LoadRifle
    // (Delay=3, Bandit.c4d/ActMap.txt) must still be phase 0 two frames
    // after the OrderDefend SetAction — rust advanced one exec early by
    // running effect timers before ExecAction.
    #[test]
    fn action_set_in_an_effect_timer_starts_its_phase_cadence_next_frame() {
        let script = r#"#strict
public func Boot() { AddEffect("Probe", this(), 1, 5, this()); return(1); }
func FxProbeTimer(pThis, iNumber) {
  SetAction("Load");
  return(-1);
}
"#;
        let mut engine = Engine::with_seed(3);
        let mut actor =
            Definition::from_script("Actr", "Actor", script).expect("script compiles");
        actor.configure_actions(
            None,
            HashMap::from([(
                "Load".to_string(),
                ActionSpec::default().with_length(10).with_delay(3),
            )]),
        );
        engine.register_definition(actor).expect("actor registers");
        let id = engine
            .spawn_object(SpawnConfig::new("Actr").with_category(CATEGORY_OBJECT))
            .expect("actor spawns");
        let idx = engine.find_object_index(id).expect("actor exists");
        engine
            .call_object_function(idx, "Boot", Vec::new())
            .expect("boot runs");

        // The interval-5 timer fires on tick 5 and sets the action AFTER
        // that tick's ExecAction; ticks 6 and 7 increment PhaseDelay to
        // 1 and 2 (< Delay 3) — phase stays 0.
        for _ in 0..7 {
            engine.tick_without_snapshot().expect("tick");
        }
        let idx = engine.find_object_index(id).expect("actor exists");
        assert_eq!(engine.objects[idx].state.action.name, "Load");
        assert_eq!(
            engine.objects[idx].state.action.phase, 0,
            "two frames after the effect-timer SetAction the phase is \
             still 0 (first increment lands the FRAME AFTER entry — \
             pEffects->Execute follows ExecAction, C4Object.cpp:1073,1085)"
        );

        // Tick 8 is the third post-entry exec: PhaseDelay reaches 3 and
        // the phase advances (C4Object.cpp:5458-5466).
        engine.tick_without_snapshot().expect("tick");
        let idx = engine.find_object_index(id).expect("actor exists");
        assert_eq!(
            engine.objects[idx].state.action.phase, 1,
            "the third post-entry exec advances the phase"
        );
    }

    // C4Object::SetAction zeroes the phase UNCONDITIONALLY on every
    // successful call — `Action.Phase = Action.PhaseDelay = 0`
    // (C4Object.cpp:4132) sits outside the action-change guard;
    // FnSetAction's fDirect is only the NoOtherAction fForce
    // (C4Script.cpp:747-753). GoldRush pins it: FireRifle leaves AimRifle
    // at phase 6 (Cowboy.c4d/Script.c:443) and the immediate
    // SetAction("LoadRifle") (Cowboy:502) must enter at phase 0 — the
    // f30 live wall showed rust carrying the stale 6 into LoadRifle.
    #[test]
    fn set_action_resets_the_phase_like_cpp() {
        let script = r#"#strict
public func Boot() { AddEffect("Probe", this(), 1, 5, this()); return(1); }
func FxProbeTimer(pThis, iNumber) {
  SetAction("Aim");
  SetPhase(6);
  SetAction("Load");
  return(-1);
}
"#;
        let mut engine = Engine::with_seed(3);
        let mut actor =
            Definition::from_script("Actr", "Actor", script).expect("script compiles");
        actor.configure_actions(
            None,
            HashMap::from([
                ("Aim".to_string(), ActionSpec::default().with_length(10)),
                ("Load".to_string(), ActionSpec::default().with_length(10)),
            ]),
        );
        engine.register_definition(actor).expect("actor registers");
        let id = engine
            .spawn_object(SpawnConfig::new("Actr").with_category(CATEGORY_OBJECT))
            .expect("actor spawns");
        let idx = engine.find_object_index(id).expect("actor exists");
        engine
            .call_object_function(idx, "Boot", Vec::new())
            .expect("boot runs");

        for _ in 0..6 {
            engine.tick_without_snapshot().expect("tick");
        }

        let idx = engine.find_object_index(id).expect("actor exists");
        assert_eq!(
            engine.objects[idx].state.action.name, "Load",
            "the second SetAction landed"
        );
        assert_eq!(
            engine.objects[idx].state.action.phase, 0,
            "SetAction zeroes the phase (C4Object.cpp:4132) — the \
             pre-change SetPhase(6) must not leak into the new action"
        );
    }

    #[test]
    fn negative_effect_interval_fires_on_magnitude_multiples_like_cpp() {
        // C4Effect stores iIntervall verbatim, increments iTime, then uses
        // signed modulo (C4Effect.cpp:67,339-345). Thus -3 fires at the same
        // positive elapsed times as +3: exactly 3, 6, and 9 here.
        let script = r#"#strict 3
local iTimes;
public func Arm()
{
  iTimes = 0;
  return AddEffect("Negative", this(), 100, -3, this());
}
func FxNegativeTimer(pThis, iNumber, iTime)
{
  iTimes = iTimes * 10 + iTime;
  return 0;
}
"#;
        let mut engine = Engine::with_seed(143);
        engine
            .register_definition(
                Definition::from_script("NEG", "Negative interval", script)
                    .expect("definition compiles"),
            )
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("NEG").with_category(CATEGORY_OBJECT))
            .expect("object spawns");
        let index = engine.find_object_index(id).expect("object exists");

        assert_eq!(
            engine
                .call_object_function(index, "Arm", Vec::new())
                .expect("negative AddEffect interval succeeds"),
            Value::Int(1)
        );
        assert_eq!(engine.objects[index].state.effects[0].interval, -3);

        for (frame, expected) in [0, 0, 3, 3, 3, 36, 36, 36, 369]
            .into_iter()
            .enumerate()
        {
            engine.tick_without_snapshot().expect("tick succeeds");
            let index = engine.find_object_index(id).expect("object exists");
            assert_eq!(
                engine.objects[index].state.local_vars.get("iTimes"),
                Some(&Value::Int(expected)),
                "timer callback history after frame {}",
                frame + 1
            );
        }
        let index = engine.find_object_index(id).expect("object exists");
        assert_eq!(engine.objects[index].state.effects[0].timer, 9);
    }

    #[test]
    fn effect_timer_kill_semantics_follow_cpp() {
        // C4Effect::Execute (C4Effect.cpp:342-360): an FxTimer returning
        // C4Fx_Execute_Kill (-1, C4Effects.h:40) kills the effect; an
        // effect whose interval elapses with NO timer function is killed
        // too (the else arm :358-360); a zero interval never reaches it.
        let script = r#"#strict 3
        static normal_stop_reason;

        global func Initialize(state, random) {
            return { effects = [
                { op = "add", name = "Doomed", interval = 2 },
                { op = "add", name = "Mute", interval = 3 },
                { op = "add", name = "Inert", interval = 0 }
            ] };
        }

        global func FxDoomedTimer(state, effect, timer) {
            if (timer >= 4) {
                return CastBool(-1);
            }
            return nil;
        }

        global func FxDoomedStop(state, effect, int reason) {
            if (reason == 0) {
                normal_stop_reason = 1;
            }
            return nil;
        }

        global func Step(state, frame, random) {
            return nil;
        }
        "#;

        let call_log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let mut hooks = DebuggerHooks::new();
        {
            let call_log = Arc::clone(&call_log);
            hooks.set_on_call(move |name, _args| {
                call_log.lock().unwrap().push(name.to_string());
            });
        }

        let mut definition =
            Definition::from_script("Actor", "Actor", script).expect("script compiles");
        definition.set_debugger_hooks(hooks);

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(definition)
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("Actor"))
            .expect("spawn succeeds");

        let mut last = None;
        for _ in 0..8 {
            last = Some(engine.tick().expect("tick succeeds"));
        }
        let snapshot = last.expect("snapshot present");
        let object = snapshot.object(id).expect("object present");
        let names: Vec<&str> = object
            .effects
            .iter()
            .map(|effect| effect.name.as_str())
            .collect();
        assert_eq!(
            names,
            vec!["Inert"],
            "Doomed killed by -1 at iTime 4, Mute killed at its first \
             timerless gate, zero-interval Inert survives"
        );
        let calls = call_log.lock().unwrap().clone();
        let stop_calls = calls.iter().filter(|name| *name == "FxDoomedStop").count();
        // The C++ list orders new-before-equal ([Inert, Mute, Doomed],
        // C4Effect.cpp:80-94), so killing Mute at its timerless gate
        // temp-removes the upper Doomed — FxDoomedStop(fTemp) fires there
        // (C4Effect.cpp:370-374,489) — and Doomed's own -1 kill fires the
        // real Stop later.
        assert_eq!(
            stop_calls, 2,
            "one temp stop from Mute's kill bracket, one real stop from \
             Doomed's own kill"
        );
        assert_eq!(
            engine
                .snapshot()
                .script_globals
                .named
                .get("normal_stop_reason"),
            Some(&Value::Int(1)),
            "C4Effect::Kill omits iReason, and a strict-3 typed integer \
             parameter observes that missing slot as C4FxCall_Normal (0)"
        );
    }

    #[test]
    fn effect_timer_walk_exposes_old_time_and_removes_later_effect_inline() {
        // C4Effect::Execute advances one live list node at a time
        // (C4Effect.cpp:319-363). While A's timer runs, the later B still
        // has its old iTime. Removing B completes its Stop inline, so the
        // traversal skips B's timer and reaches C only afterwards.
        let script = r#"#strict 3
        local iOrder, iSeenB;

        func Install() {
            iOrder = 0;
            iSeenB = -1;
            AddEffect("A", this(), 100, 1, this());
            AddEffect("B", this(), 200, 1, this());
            AddEffect("C", this(), 300, 1, this());
        }

        func FxATimer(object target, int number, int time) {
            iOrder = iOrder * 10 + 1;
            iSeenB = GetEffect("B", target, 0, 6);
            RemoveEffect("B", target);
            return 0;
        }

        func FxBTimer() { iOrder = iOrder * 10 + 9; }
        func FxBStop() { iOrder = iOrder * 10 + 2; }
        func FxCTimer() { iOrder = iOrder * 10 + 3; }
        "#;

        let mut definition =
            Definition::from_script("OTW1", "Object timer walk", script)
                .expect("script compiles");
        definition.set_c4_callback_convention(true);

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(definition)
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("OTW1"))
            .expect("target spawns");
        let idx = engine.find_object_index(id).expect("target exists");
        engine
            .call_object_function(idx, "Install", Vec::new())
            .expect("effects install");

        engine.tick_without_snapshot().expect("timer frame succeeds");

        let object = engine.object_snapshot(id).expect("target remains live");
        assert_eq!(object.local_vars.get("iOrder"), Some(&Value::Int(123)));
        assert_eq!(
            object.local_vars.get("iSeenB"),
            Some(&Value::Int(0)),
            "A runs before C4Effect::Execute increments the later B"
        );
        assert!(
            !object
                .effects
                .iter()
                .any(|effect| effect.name == "B" && effect.priority != 0),
            "B's inline removal prevents its already-eligible timer"
        );
        for name in ["A", "C"] {
            assert_eq!(
                object
                    .effects
                    .iter()
                    .find(|effect| effect.name == name && effect.priority != 0)
                    .map(|effect| effect.timer),
                Some(1),
                "the surviving effect advances exactly once"
            );
        }
    }

    #[test]
    fn effect_timer_walk_executes_new_higher_effect_same_frame() {
        // A new higher-priority node is inserted after the current cursor.
        // C4Effect::Execute reaches it in the same live-list walk, advances
        // iTime to one, and immediately fires its interval-one timer.
        let script = r#"#strict 3
        local iOrder, iNewTime;

        func Install() {
            iOrder = 0;
            iNewTime = -1;
            AddEffect("A", this(), 100, 1, this());
        }

        func FxATimer(object target) {
            iOrder = iOrder * 10 + 1;
            if (!GetEffect("New", target))
                AddEffect("New", target, 200, 1, target);
            return 0;
        }

        func FxNewTimer(object target, int number, int time) {
            iOrder = iOrder * 10 + 2;
            iNewTime = time;
            return 0;
        }
        "#;

        let mut definition =
            Definition::from_script("OTW2", "Object timer insertion", script)
                .expect("script compiles");
        definition.set_c4_callback_convention(true);

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(definition)
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("OTW2"))
            .expect("target spawns");
        let idx = engine.find_object_index(id).expect("target exists");
        engine
            .call_object_function(idx, "Install", Vec::new())
            .expect("driver installs");

        engine.tick_without_snapshot().expect("timer frame succeeds");

        let object = engine.object_snapshot(id).expect("target remains live");
        assert_eq!(object.local_vars.get("iOrder"), Some(&Value::Int(12)));
        assert_eq!(object.local_vars.get("iNewTime"), Some(&Value::Int(1)));
        assert_eq!(
            object
                .effects
                .iter()
                .find(|effect| effect.name == "New" && effect.priority != 0)
                .map(|effect| effect.timer),
            Some(1),
            "the newly inserted node advances in the frame that created it"
        );
    }

    #[test]
    fn effect_timer_walk_executes_replacement_after_current_unlinks() {
        // The current callback may unlink its own node without callbacks and
        // immediately add a higher-priority replacement. The Rust list can
        // reuse the just-freed effect number, so the live traversal must not
        // treat that number alone as an already-run cursor identity.
        let script = r#"#strict 3
        local iOrder, iNewTime;

        func Install() {
            iOrder = 0;
            iNewTime = -1;
            AddEffect("A", this(), 100, 1, this());
        }

        func FxATimer(object target, int number) {
            iOrder = iOrder * 10 + 1;
            RemoveEffect(nil, target, number, true);
            AddEffect("New", target, 200, 1, target);
            return 0;
        }

        func FxNewTimer(object target, int number, int time) {
            iOrder = iOrder * 10 + 2;
            iNewTime = time;
            return 0;
        }
        "#;

        let mut definition =
            Definition::from_script("OTW4", "Object timer replacement", script)
                .expect("script compiles");
        definition.set_c4_callback_convention(true);

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(definition)
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("OTW4"))
            .expect("target spawns");
        let idx = engine.find_object_index(id).expect("target exists");
        engine
            .call_object_function(idx, "Install", Vec::new())
            .expect("driver installs");

        engine.tick_without_snapshot().expect("timer frame succeeds");

        let object = engine.object_snapshot(id).expect("target remains live");
        assert_eq!(object.local_vars.get("iOrder"), Some(&Value::Int(12)));
        assert_eq!(object.local_vars.get("iNewTime"), Some(&Value::Int(1)));
        assert_eq!(
            object
                .effects
                .iter()
                .find(|effect| effect.name == "New" && effect.priority != 0)
                .map(|effect| effect.timer),
            Some(1),
            "the replacement is reached even when it reuses the removed cursor's number"
        );
    }

    #[test]
    fn effect_timer_kill_drops_removed_upper_readd_event() {
        // A timer result of -1 performs Kill inline before traversal reaches
        // Upper. Lower's Stop removes the temporarily inactive Upper; the
        // stale TempReaddUpperEffects cursor must neither call Start(1) nor
        // let Upper's formerly eligible timer run (C4Effect.cpp:342-405).
        let script = r#"#strict 3
        local iOrder, iStaleReadds, iUpperTimers;

        func Install() {
            iOrder = iStaleReadds = iUpperTimers = 0;
            AddEffect("Lower", this(), 100, 1, this());
            AddEffect("Upper", this(), 200, 1, this());
        }

        func FxLowerTimer() {
            iOrder = iOrder * 10 + 1;
            return -1;
        }

        func FxLowerStop(object target) {
            iOrder = iOrder * 10 + 3;
            RemoveEffect("Upper", target);
            return 0;
        }

        func FxUpperTimer() {
            iOrder = iOrder * 10 + 9;
            ++iUpperTimers;
            return 0;
        }

        func FxUpperStop(object target, int number, int reason, bool temp) {
            if (reason == 1 && temp)
                iOrder = iOrder * 10 + 2;
            else
                iOrder = iOrder * 10 + 4;
            return 0;
        }

        func FxUpperStart(object target, int number, int temp) {
            // Killing an inactive effect also has a Start(2) callback; that
            // separate behavior is intentionally outside this regression.
            if (temp == 1) {
                iOrder = iOrder * 10 + 8;
                ++iStaleReadds;
            }
            return 0;
        }
        "#;

        let mut definition =
            Definition::from_script("OTW3", "Object timer kill walk", script)
                .expect("script compiles");
        definition.set_c4_callback_convention(true);

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(definition)
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("OTW3"))
            .expect("target spawns");
        let idx = engine.find_object_index(id).expect("target exists");
        engine
            .call_object_function(idx, "Install", Vec::new())
            .expect("effects install");

        engine.tick_without_snapshot().expect("timer frame succeeds");

        let object = engine.object_snapshot(id).expect("target remains live");
        assert_eq!(object.local_vars.get("iOrder"), Some(&Value::Int(1234)));
        assert_eq!(
            object.local_vars.get("iUpperTimers"),
            Some(&Value::Int(0))
        );
        assert_eq!(
            object.local_vars.get("iStaleReadds"),
            Some(&Value::Int(0))
        );
        assert!(
            !object
                .effects
                .iter()
                .any(|effect| effect.name == "Upper" && effect.priority != 0),
            "Upper may remain linked dead until Execute revisits it, but it must not be active"
        );
    }

    #[test]
    fn effect_timer_object_removal_aborts_walk_after_inline_clear_stops() {
        // C4Effect::Execute aborts its live-list walk as soon as the carrier
        // loses Status (C4Effect.cpp:342-353). AssignRemoval clears effects
        // synchronously from high to low before RemoveObject returns, so B's
        // timer never runs and neither Stop may be replayed after A resumes.
        let script = r#"#strict 3
        static iOrder, iATimers, iBTimers, iAStops, iBStops;
        static iAReason, iBReason;

        func Install() {
            iOrder = iATimers = iBTimers = iAStops = iBStops = 0;
            iAReason = iBReason = -1;
            AddEffect("A", this(), 100, 1, this());
            AddEffect("B", this(), 200, 1, this());
        }

        func FxATimer() {
            ++iATimers;
            iOrder = iOrder * 10 + 1;
            RemoveObject(this());
            iOrder = iOrder * 10 + 4;
            return 0;
        }

        func FxBTimer() {
            ++iBTimers;
            iOrder = iOrder * 10 + 9;
            return 0;
        }

        func FxAStop(object target, int number, int reason) {
            ++iAStops;
            iAReason = reason;
            iOrder = iOrder * 10 + 3;
            return 0;
        }

        func FxBStop(object target, int number, int reason) {
            ++iBStops;
            iBReason = reason;
            iOrder = iOrder * 10 + 2;
            return 0;
        }
        "#;

        let mut definition =
            Definition::from_script("OTW5", "Object timer carrier removal", script)
                .expect("script compiles");
        definition.set_c4_callback_convention(true);

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(definition)
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("OTW5"))
            .expect("target spawns");
        let idx = engine.find_object_index(id).expect("target exists");
        engine
            .call_object_function(idx, "Install", Vec::new())
            .expect("effects install");

        engine.tick_without_snapshot().expect("timer frame succeeds");

        assert!(engine.object_snapshot(id).is_none());
        let globals = engine.snapshot().script_globals.named;
        assert_eq!(globals.get("iOrder"), Some(&Value::Int(1234)));
        assert_eq!(globals.get("iATimers"), Some(&Value::Int(1)));
        assert_eq!(globals.get("iBTimers"), Some(&Value::Int(0)));
        assert_eq!(globals.get("iAStops"), Some(&Value::Int(1)));
        assert_eq!(globals.get("iBStops"), Some(&Value::Int(1)));
        assert_eq!(globals.get("iAReason"), Some(&Value::Int(3)));
        assert_eq!(globals.get("iBReason"), Some(&Value::Int(3)));
    }

    #[test]
    fn effect_callback_docon_removal_clears_effects_tail_first() {
        // Reaching zero construction inside an effect callback requests
        // object destruction through the callback outcome. C4Object's
        // AssignRemoval then runs C4Effect::ClearAll recursively, so the
        // highest-priority Stop precedes the lower-priority Stop
        // (C4Effect.cpp:407-412; C4Object.cpp:262-264).
        let script = r#"#strict 3
        static iStopOrder;

        func Install() {
            iStopOrder = 0;
            AddEffect("Destroyer", this(), 1, 1, this());
            AddEffect("Low", this(), 100, 0, this());
            AddEffect("High", this(), 200, 0, this());
        }

        func FxDestroyerTimer() {
            DoCon(-100);
            return 0;
        }

        func FxLowStop(object target, int number, int reason) {
            iStopOrder = iStopOrder * 10 + 1;
            return 0;
        }

        func FxHighStop(object target, int number, int reason) {
            iStopOrder = iStopOrder * 10 + 2;
            return 0;
        }
        "#;

        let mut definition =
            Definition::from_script("CLR1", "Callback construction removal", script)
                .expect("script compiles");
        definition.set_c4_callback_convention(true);

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(definition)
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("CLR1"))
            .expect("target spawns");
        let idx = engine.find_object_index(id).expect("target exists");
        engine
            .call_object_function(idx, "Install", Vec::new())
            .expect("effects install");

        engine.tick_without_snapshot().expect("destruction timer frame succeeds");

        assert!(engine.object_snapshot(id).is_none());
        assert_eq!(
            engine.snapshot().script_globals.named.get("iStopOrder"),
            Some(&Value::Int(21)),
            "ClearAll stops the highest-priority effect before the lower one"
        );
    }

    #[test]
    fn effect_death_stop_receives_reason_four_and_can_revive_target() {
        // AssignDeath clears effects with C4FxCall_RemoveDeath (4). Like
        // C4Effect::ClearAll, a Stop callback returning C4Fx_Stop_Deny (-1)
        // restores that effect; if the callback also revives the target,
        // ordinary death aborts (C4Object.cpp:1162-1170;
        // C4Effect.cpp:407-424).
        let script = r#"#strict 3
        func FxReprieveStop(target, number, int reason) {
            if (reason == 4) {
                SetAlive(true);
                return -1;
            }
            return 0;
        }
        "#;

        let mut definition =
            Definition::from_script("LIVG", "Living effect target", script)
                .expect("script compiles");
        let call_log: Arc<Mutex<Vec<(String, Vec<Value>)>>> = Arc::new(Mutex::new(Vec::new()));
        let mut hooks = DebuggerHooks::new();
        {
            let call_log = Arc::clone(&call_log);
            hooks.set_on_call(move |name, args| {
                call_log
                    .lock()
                    .unwrap()
                    .push((name.to_string(), args.to_vec()));
            });
        }
        definition.set_debugger_hooks(hooks);
        definition.set_c4_callback_convention(true);
        definition.set_category(CATEGORY_OBJECT | CATEGORY_LIVING);
        definition.configure_actions(
            Some("Idle".to_string()),
            HashMap::from([
                ("Idle".to_string(), ActionSpec::default()),
                ("Dead".to_string(), ActionSpec::default()),
            ]),
        );

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(definition)
            .expect("definition registers");
        let id = engine
            .spawn_object(
                SpawnConfig::new("LIVG")
                    .with_category(CATEGORY_OBJECT | CATEGORY_LIVING)
                    .with_alive(true)
                    .add_effect(EffectState::new("Reprieve").with_priority(100)),
            )
            .expect("living target spawns");
        let idx = engine.find_object_index(id).expect("target exists");
        let command_target = i32::try_from(id.as_u64()).expect("test object id fits C4 int");
        engine.objects[idx].state.effects[0].command_target = Some(command_target);

        engine.assign_death(idx, false).expect("death callbacks run");

        let object = engine.object_snapshot(id).expect("revived target remains");
        assert!(object.alive, "SetAlive in Fx*Stop aborts ordinary death");
        let calls = call_log.lock().unwrap();
        let stop_args = calls
            .iter()
            .find_map(|(name, args)| (name == "FxReprieveStop").then_some(args))
            .expect("death dispatch calls FxReprieveStop");
        assert_eq!(
            stop_args.get(2),
            Some(&Value::Int(4)),
            "Fx*Stop receives C4FxCall_RemoveDeath"
        );
        assert_eq!(
            object
                .effects
                .iter()
                .map(|effect| effect.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Reprieve"],
            "returning -1 restores the death-cleared effect"
        );
    }

    #[test]
    fn global_effect_timer_fires_with_nil_target_like_c4effect_execute() {
        // C4Game::Execute (C4Game.cpp:830-831): pGlobalEffects->
        // Execute(nullptr) runs right after ExecObjects; C4Effect::Execute
        // (C4Effect.cpp:339-345) advances iTime every frame and fires
        // Fx*Timer(pTarget=nil, iNumber, iTime) on elapsed intervals. The
        // callback resolves through Game.ScriptEngine when the effect has
        // no command target (C4Effect::DoCall, C4Effect.cpp:448-452).
        let script = r#"
        global func Initialize(state, random) {
            var no_target;
            AddEffect("WorldPulse", no_target, 200, 2);
            return 0;
        }

        global func FxWorldPulseTimer(target, number, time) {
            var no_target;
            // pObj is nullptr for global effects (C4Effect.cpp:345).
            if (target) { return 0; }
            EffectVar(0, no_target, number) = time;
            return 0;
        }

        global func Step(state, frame, random) { return 0; }
        "#;

        let definition =
            Definition::from_script("Actor", "Actor", script).expect("script compiles");
        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine
            .spawn_object(SpawnConfig::new("Actor"))
            .expect("spawn succeeds");

        assert_eq!(engine.global_effects().len(), 1);

        engine.tick_without_snapshot().expect("first tick succeeds");
        assert_eq!(
            engine.global_effects()[0].timer,
            1,
            "iTime advances every frame (C4Effect.cpp:340)"
        );
        assert_eq!(
            engine.global_effects()[0].var(0),
            EffectVarValue::Nil,
            "interval 2 has not elapsed at iTime 1 (C4Effect.cpp:342)"
        );

        engine.tick_without_snapshot().expect("second tick succeeds");
        assert_eq!(
            engine.global_effects()[0].var(0),
            EffectVarValue::Int(2),
            "Fx*Timer(nil, iNumber, iTime) fired at the elapsed interval \
             and its EffectVar write folded back"
        );

        engine.tick_without_snapshot().expect("third tick succeeds");
        engine.tick_without_snapshot().expect("fourth tick succeeds");
        assert_eq!(
            engine.global_effects()[0].var(0),
            EffectVarValue::Int(4),
            "the timer keeps firing on every elapsed interval"
        );
    }

    #[test]
    fn no_command_target_effect_uses_exact_engine_global_scope() {
        // C4Effect::GetCallbackScript falls back to Game.ScriptEngine when
        // both command-target fields are empty; the affected object's local
        // callback must not shadow that table (C4Effect.cpp:31-56). The
        // retained global function then resolves the ordinary Helper in its
        // own LinkedTo host and still has cthr->Def == nullptr.
        let definition_script = r#"#strict 2
local result;

func Arm()
{
    result = 0;
    AddEffect("NoTarget", this(), 100, 1);
    return true;
}

func Mark(value) { result = value; return true; }
func Read() { return result; }

// This local same-name callback is invisible from Game.ScriptEngine.
func FxNoTargetTimer(target, number, time)
{
    target->Mark(99);
    return 0;
}
"#;
        let global_script = r#"#strict 2
global func FxNoTargetTimer(target, number, time)
{
    return Helper(target);
}

func Helper(target)
{
    var no_value;
    if (GetActMapVal("Length", "Probe") == no_value)
        target->Mark(17);
    else
        target->Mark(98);
    return 0;
}
"#;

        let mut definition =
            Definition::from_script("FXGS", "Global scope probe", definition_script)
                .expect("definition script compiles");
        definition.set_c4_callback_convention(true);
        definition.configure_actions(
            Some("Probe".to_string()),
            HashMap::from([(
                "Probe".to_string(),
                ActionSpec {
                    length: Some(23),
                    ..ActionSpec::default()
                },
            )]),
        );

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(definition)
            .expect("definition registers");
        assert_eq!(
            engine.install_global_scripts(&[(
                "System.c4g/EffectScope.c".to_string(),
                global_script.to_string(),
            )]),
            1
        );
        let id = engine
            .spawn_object(SpawnConfig::new("FXGS"))
            .expect("probe spawns");
        let idx = engine.find_object_index(id).expect("probe exists");
        engine
            .call_object_function(idx, "Arm", Vec::new())
            .expect("effect arms");

        engine.tick_without_snapshot().expect("global effect callback runs");
        let idx = engine.find_object_index(id).expect("probe remains");
        assert_eq!(
            engine
                .call_object_function(idx, "Read", Vec::new())
                .expect("result reads"),
            Value::Int(17),
            "the global callback used its exact Helper and no implicit definition"
        );
    }

    #[test]
    fn command_target_global_effect_keeps_this_and_linked_helper_scope() {
        let definition_script = r#"#strict 2
local result;
func Arm() { result = 0; AddEffect("Commanded", this(), 100, 1, this()); return true; }
func Helper() { return 99; }
func Mark(value) { result = value; return true; }
func Read() { return result; }
"#;
        let global_script = r#"#strict 2
global func FxCommandedStart(target, number, temp)
{
    this()->Mark(Helper());
    return 0;
}
global func FxCommandedTimer(target, number, time)
{
    this()->Mark(Helper());
    return 0;
}
func Helper() { return 17; }
"#;
        let mut definition =
            Definition::from_script("FXCT", "Command-target probe", definition_script)
                .expect("definition compiles");
        definition.set_c4_callback_convention(true);
        let mut engine = Engine::with_seed(11);
        engine
            .register_definition(definition)
            .expect("definition registers");
        assert_eq!(
            engine.install_global_scripts(&[(
                "System.c4g/CommandedEffect.c".to_string(),
                global_script.to_string(),
            )]),
            1
        );
        let id = engine
            .spawn_object(SpawnConfig::new("FXCT"))
            .expect("probe spawns");
        let idx = engine.find_object_index(id).expect("probe exists");
        engine
            .call_object_function(idx, "Arm", Vec::new())
            .expect("effect arms");
        let idx = engine.find_object_index(id).expect("probe remains");
        assert_eq!(
            engine
                .call_object_function(idx, "Read", Vec::new())
                .expect("synchronous Start result reads"),
            Value::Int(17),
            "synchronous Fx*Start retained its System helper and command-target this"
        );

        engine.tick_without_snapshot().expect("command-target callback runs");
        let idx = engine.find_object_index(id).expect("probe remains");
        assert_eq!(
            engine
                .call_object_function(idx, "Read", Vec::new())
                .expect("result reads"),
            Value::Int(17),
            "the global SFunc retained its System helper and command-target this"
        );
    }

    #[test]
    fn scheduled_eval_uses_command_target_definition_scope() {
        // Helpers.c's global FxIntScheduleTimer runs eval() with the scheduled
        // object as `this` (planet/System.c4g/Helpers.c:110-132). FnEval then
        // selects cthr->Obj->Def->Script for DirectExec, so both the target's
        // named locals and its own functions resolve there (C4Script.cpp:
        // 4501-4513; C4AulExec.cpp:1658-1707).
        let definition_script = r#"#strict 2
local power, result;

func Arm()
{
    power = 50;
    result = 0;
    var effect = AddEffect("IntSchedule", this(), 1, 1, this());
    EffectVar(0, this(), effect) = "Explode(power)";
    return true;
}

func Explode(value) { result = value; return true; }
func Read() { return result; }
"#;
        let global_script = r#"#strict 2
global func FxIntScheduleTimer(target, number, time)
{
    eval(EffectVar(0, target, number));
    return -1;
}
"#;
        let mut definition =
            Definition::from_script("FXEV", "Scheduled eval target", definition_script)
                .expect("definition compiles");
        definition.set_c4_callback_convention(true);
        let mut engine = Engine::with_seed(13);
        engine
            .register_definition(definition)
            .expect("definition registers");
        assert_eq!(
            engine.install_global_scripts(&[(
                "System.c4g/Helpers.c".to_string(),
                global_script.to_string(),
            )]),
            1
        );
        let id = engine
            .spawn_object(SpawnConfig::new("FXEV"))
            .expect("target spawns");
        let idx = engine.find_object_index(id).expect("target exists");
        engine
            .call_object_function(idx, "Arm", Vec::new())
            .expect("schedule arms");

        engine.tick_without_snapshot().expect("scheduled eval runs");

        let idx = engine.find_object_index(id).expect("target remains");
        assert_eq!(
            engine
                .call_object_function(idx, "Read", Vec::new())
                .expect("result reads"),
            Value::Int(50),
            "eval resolves the target definition's local and function"
        );
        assert!(
            engine.objects[idx]
                .state
                .effects
                .iter()
                .all(|effect| effect.priority == 0),
            "the successful one-shot schedule removes its timer"
        );
    }

    #[test]
    fn explicit_global_eval_uses_scenario_script_scope() {
        // AB_CALLGLOBAL dispatches with null destination Obj/Def, so FnEval
        // selects Game.Script and DirectExec resolves scenario-local functions
        // there (C4AulExec.cpp:1216-1297; C4Script.cpp:4501-4513).
        let definition_script = r#"#strict 3
func Probe() { return global->eval("ScenarioHelper()"); }
"#;
        let scenario_script = r#"#strict 3
func ScenarioHelper() { return 73; }
"#;
        let mut engine = Engine::with_seed(17);
        engine
            .register_definition(
                Definition::from_script("GEVL", "Global eval caller", definition_script)
                    .expect("definition compiles"),
            )
            .expect("definition registers");
        engine
            .install_scenario_script_with_convention("Scenario", scenario_script, true)
            .expect("scenario script installs");
        let id = engine
            .spawn_object(SpawnConfig::new("GEVL"))
            .expect("caller spawns");
        let idx = engine.find_object_index(id).expect("caller exists");

        assert_eq!(
            engine
                .call_object_function(idx, "Probe", Vec::new())
                .expect("global eval resolves through Game.Script"),
            Value::Int(73)
        );
    }

    #[test]
    fn object_effect_uses_and_persists_foreign_command_target_locals() {
        // Every object-effect callback executes with pCommandTarget as its
        // C4Aul `this` (C4Effect.cpp:345). That object owns the live Local[]
        // storage even when pForObj is a different carrier.
        let script = r#"#strict 2
local counter;

func SetCounter(value) { counter = value; return true; }
func ReadCounter() { return counter; }
func ArmObjectEffect(target)
{
    AddEffect("ForeignObjectA", target, 100, 1, this());
    AddEffect("ForeignObjectB", target, 101, 1, this());
    return true;
}
func FxForeignObjectATimer(target, number, time)
{
    counter = counter + 1;
    return 0;
}
func FxForeignObjectBTimer(target, number, time)
{
    counter = counter + 1;
    return 0;
}
"#;
        let mut definition =
            Definition::from_script("FXFO", "Foreign object-effect target", script)
                .expect("definition compiles");
        definition.set_c4_callback_convention(true);

        let mut engine = Engine::with_seed(12);
        engine
            .register_definition(definition)
            .expect("definition registers");
        let command_target = engine
            .spawn_object(SpawnConfig::new("FXFO"))
            .expect("command target spawns");
        let carrier = engine
            .spawn_object(SpawnConfig::new("FXFO"))
            .expect("carrier spawns");
        let command_index = engine
            .find_object_index(command_target)
            .expect("command target exists");
        engine
            .call_object_function(command_index, "SetCounter", vec![Value::Int(40)])
            .expect("counter seeds");
        let command_index = engine
            .find_object_index(command_target)
            .expect("command target remains");
        engine
            .call_object_function(
                command_index,
                "ArmObjectEffect",
                vec![Value::Object(carrier.as_u64())],
            )
            .expect("foreign-target effect arms");

        engine.tick_without_snapshot().expect("foreign-target timer runs");
        let command_index = engine
            .find_object_index(command_target)
            .expect("command target remains");
        assert_eq!(
            engine
                .call_object_function(command_index, "ReadCounter", Vec::new())
                .expect("counter reads"),
            Value::Int(42),
            "the second same-tick callback observes the first callback's live local write"
        );
    }

    #[test]
    fn object_effect_error_keeps_foreign_command_target_local_writes() {
        // C4Aul's fail-safe effect Exec aborts on a runtime error without
        // rolling back writes already made to pCommandTarget. The following
        // callback in the same object-effect batch must see that live write.
        let script = r#"#strict 2
local counter;

func SetCounter(value) { counter = value; return true; }
func ReadCounter() { return counter; }
func ArmObjectEffectsWithError(target)
{
    AddEffect("ForeignObjectError", target, 100, 1, this());
    AddEffect("ForeignObjectAfterError", target, 101, 1, this());
    return true;
}
func FxForeignObjectErrorTimer(target, number, time)
{
    counter = counter + 1;
    NoSuchFunctionAnywhere();
    return 0;
}
func FxForeignObjectAfterErrorTimer(target, number, time)
{
    counter = counter + 1;
    return 0;
}
"#;
        let mut definition =
            Definition::from_script("FXOE", "Object-effect error target", script)
                .expect("definition compiles");
        definition.set_c4_callback_convention(true);

        let mut engine = Engine::with_seed(13);
        engine
            .register_definition(definition)
            .expect("definition registers");
        let command_target = engine
            .spawn_object(SpawnConfig::new("FXOE"))
            .expect("command target spawns");
        let carrier = engine
            .spawn_object(SpawnConfig::new("FXOE"))
            .expect("carrier spawns");
        let command_index = engine
            .find_object_index(command_target)
            .expect("command target exists");
        engine
            .call_object_function(command_index, "SetCounter", vec![Value::Int(40)])
            .expect("counter seeds");
        let command_index = engine
            .find_object_index(command_target)
            .expect("command target remains");
        engine
            .call_object_function(
                command_index,
                "ArmObjectEffectsWithError",
                vec![Value::Object(carrier.as_u64())],
            )
            .expect("foreign-target effects arm");

        engine.tick_without_snapshot().expect("fail-safe timer batch continues");
        let command_index = engine
            .find_object_index(command_target)
            .expect("command target remains");
        assert_eq!(
            engine
                .call_object_function(command_index, "ReadCounter", Vec::new())
                .expect("counter reads"),
            Value::Int(42),
            "the errored callback's pre-error local write remains live for the next callback"
        );
    }

    #[test]
    fn global_effect_uses_and_persists_command_target_locals() {
        // Game.pGlobalEffects still calls Exec(pCommandTarget, ...) while
        // passing nil as pForObj (C4Effect.cpp:345). The absence of a
        // carrier must not replace the command target's locals with empty
        // storage.
        let script = r#"#strict 2
local counter;

func SetCounter(value) { counter = value; return true; }
func ReadCounter() { return counter; }
func ArmGlobalEffect()
{
    var no_target;
    AddEffect("ForeignGlobalA", no_target, 100, 1, this());
    AddEffect("ForeignGlobalB", no_target, 101, 1, this());
    AddEffect("ForeignGlobalC", no_target, 102, 1, this());
    return true;
}
func FxForeignGlobalATimer(target, number, time)
{
    counter = counter + 1;
    return 0;
}
func FxForeignGlobalBTimer(target, number, time)
{
    counter = counter + 1;
    return 0;
}
func FxForeignGlobalCTimer(target, number, time)
{
    counter = counter + 1;
    return 0;
}
"#;
        let mut definition =
            Definition::from_script("FXFG", "Global-effect command target", script)
                .expect("definition compiles");
        definition.set_c4_callback_convention(true);

        let mut engine = Engine::with_seed(14);
        engine
            .register_definition(definition)
            .expect("definition registers");
        let command_target = engine
            .spawn_object(SpawnConfig::new("FXFG"))
            .expect("command target spawns");
        let command_index = engine
            .find_object_index(command_target)
            .expect("command target exists");
        engine
            .call_object_function(command_index, "SetCounter", vec![Value::Int(50)])
            .expect("counter seeds");
        let command_index = engine
            .find_object_index(command_target)
            .expect("command target remains");
        engine
            .call_object_function(command_index, "ArmGlobalEffect", Vec::new())
            .expect("global effect arms");

        engine.tick_without_snapshot().expect("global timer runs");
        let command_index = engine
            .find_object_index(command_target)
            .expect("command target remains");
        assert_eq!(
            engine
                .call_object_function(command_index, "ReadCounter", Vec::new())
                .expect("counter reads"),
            Value::Int(53),
            "each carrier-less callback observes earlier same-tick local writes"
        );
    }

    #[test]
    fn global_effect_error_keeps_command_target_local_writes() {
        // Global effects have no carrier, but their callbacks still execute
        // on pCommandTarget. An ordinary runtime error yields nil and keeps
        // the local mutation for the next global-effect callback.
        let script = r#"#strict 2
local counter;

func SetCounter(value) { counter = value; return true; }
func ReadCounter() { return counter; }
func ArmGlobalEffectsWithError()
{
    var no_target;
    AddEffect("ForeignGlobalError", no_target, 100, 1, this());
    AddEffect("ForeignGlobalAfterError", no_target, 101, 1, this());
    return true;
}
func FxForeignGlobalErrorTimer(target, number, time)
{
    counter = counter + 1;
    NoSuchFunctionAnywhere();
    return 0;
}
func FxForeignGlobalAfterErrorTimer(target, number, time)
{
    counter = counter + 1;
    return 0;
}
"#;
        let mut definition =
            Definition::from_script("FXGE", "Global-effect error target", script)
                .expect("definition compiles");
        definition.set_c4_callback_convention(true);

        let mut engine = Engine::with_seed(15);
        engine
            .register_definition(definition)
            .expect("definition registers");
        let command_target = engine
            .spawn_object(SpawnConfig::new("FXGE"))
            .expect("command target spawns");
        let command_index = engine
            .find_object_index(command_target)
            .expect("command target exists");
        engine
            .call_object_function(command_index, "SetCounter", vec![Value::Int(50)])
            .expect("counter seeds");
        let command_index = engine
            .find_object_index(command_target)
            .expect("command target remains");
        engine
            .call_object_function(command_index, "ArmGlobalEffectsWithError", Vec::new())
            .expect("global effects arm");

        engine.tick_without_snapshot().expect("fail-safe global timer batch continues");
        let command_index = engine
            .find_object_index(command_target)
            .expect("command target remains");
        assert_eq!(
            engine
                .call_object_function(command_index, "ReadCounter", Vec::new())
                .expect("counter reads"),
            Value::Int(52),
            "the errored global callback's pre-error local write remains live"
        );
    }

    #[test]
    fn global_effect_timer_runs_without_any_registered_definition() {
        let mut engine = Engine::with_seed(13);
        assert_eq!(
            engine.install_global_scripts(&[(
                "System.c4g/SoloEffect.c".to_string(),
                "global func FxSoloTimer(target, number, time) { var no_target; EffectVar(0, no_target, number) = time; return 0; }\n"
                    .to_string(),
            )]),
            1
        );
        let mut effect = EffectState::new("Solo").with_interval(1);
        effect.number = 1;
        let mut state = engine.capture_state();
        state.global_effects = vec![effect];
        engine.restore_state(&state).expect("effect-only state restores");

        engine.tick_without_snapshot().expect("definition-free global callback runs");
        assert_eq!(engine.global_effects().len(), 1);
        assert_eq!(engine.global_effects()[0].var(0), EffectVarValue::Int(1));
    }

    #[test]
    fn invalid_command_id_damage_falls_back_to_engine_global() {
        let mut engine = Engine::with_seed(17);
        assert_eq!(
            engine.install_global_scripts(&[(
                "System.c4g/DamageEffect.c".to_string(),
                "global func FxInvalidDamage(target, number, change, cause, caused_by) { return 0; }\n"
                    .to_string(),
            )]),
            1
        );
        engine
            .register_definition(simple_definition("DMGI"))
            .expect("definition registers");
        let id = engine
            .spawn_object(
                SpawnConfig::new("DMGI").add_effect(
                    EffectState::new("Invalid").with_command_id(Some("MISS")),
                ),
            )
            .expect("target spawns");
        let idx = engine.find_object_index(id).expect("target exists");

        engine
            .change_object_damage(idx, 10, 0, OWNER_NONE)
            .expect("damage callback runs");
        assert_eq!(engine.object_snapshot(id).expect("target remains").damage, 0);
    }

    #[test]
    fn same_callback_batch_change_def_rebinds_effect_stop() {
        let old_script = r#"#strict 2
local result;
func Arm() { result = 0; AddEffect("Swap", this(), 100, 1, this()); return true; }
func FxSwapTimer(target, number, time) { ChangeDef(FXNW); return -1; }
"#;
        let new_script = r#"#strict 2
local result;
func FxSwapStop(target, number, reason) { result = 17; return 0; }
func Read() { return result; }
"#;
        let mut old = Definition::from_script("FXOL", "Old effect host", old_script)
            .expect("old definition compiles");
        old.set_c4_callback_convention(true);
        let mut new = Definition::from_script("FXNW", "New effect host", new_script)
            .expect("new definition compiles");
        new.set_c4_callback_convention(true);
        let mut engine = Engine::with_seed(19);
        engine.register_definition(old).expect("old registers");
        engine.register_definition(new).expect("new registers");
        let id = engine
            .spawn_object(SpawnConfig::new("FXOL"))
            .expect("target spawns");
        let idx = engine.find_object_index(id).expect("target exists");
        engine
            .call_object_function(idx, "Arm", Vec::new())
            .expect("effect arms");

        engine.tick_without_snapshot().expect("timer and rebound Stop run");
        let snapshot = engine.object_snapshot(id).expect("target remains");
        assert_eq!(snapshot.definition_id, "FXNW");
        let idx = engine.find_object_index(id).expect("target remains indexed");
        assert_eq!(
            engine
                .call_object_function(idx, "Read", Vec::new())
                .expect("new definition reads"),
            Value::Int(17)
        );
    }

    #[test]
    fn scheduled_global_set_fow_kills_after_one_tick_and_persists_flags() {
        // Dragon Rock's Helpers.c Schedule path is an interval-1 global
        // IntSchedule effect. Its callback kills the effect after the
        // scheduled SetFoW succeeds; an unknown host call instead takes the
        // fail-safe error path and leaves the effect alive every frame.
        // FnSetFoW/C4Player::SetFoW: C4Script.cpp:3671-3678 and
        // C4Player.cpp:815-824.
        let script = r#"
        global func Initialize(state, random) {
            var no_target;
            var effect = AddEffect("IntSchedule", no_target, 200, 1);
            EffectVar(0, no_target, effect) = "SetFoW(true, 0)";
            return 0;
        }

        global func FxIntScheduleTimer(target, number, time) {
            eval(EffectVar(0, target, number));
            return -1;
        }

        global func Step(state, frame, random) { return 0; }
        "#;

        let definition =
            Definition::from_script("ELEV", "Elevator", script).expect("script compiles");
        let mut engine = Engine::with_seed(7);
        engine
            .register_player(PlayerConfig::new(0, "Player"))
            .expect("player registers");
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine
            .spawn_object(SpawnConfig::new("ELEV"))
            .expect("spawn succeeds");
        assert_eq!(engine.global_effects().len(), 1);

        engine.tick_without_snapshot().expect("scheduled callback succeeds");

        assert!(
            engine
                .global_effects()
                .iter()
                .all(|effect| effect.priority == 0),
            "the successful one-shot callback removes its schedule effect"
        );
        let player = engine.player(0).expect("player remains registered");
        assert!(player.fog_of_war());
        assert!(player.force_fog_of_war());
        let persisted = player.to_state();
        assert!(persisted.fog_of_war);
        assert!(persisted.force_fog_of_war);
    }

    #[test]
    fn set_plr_view_range_matches_cpp_targets_clamp_and_persistence() -> Result<(), EngineError> {
        // FnSetPlrViewRange defaults its object to the caller, clamps legacy
        // positive ranges below 128 unless fExact is set, preserves negative
        // values, and returns C4ValueInt success (C4Script.cpp:3681-3691).
        // ObjectSnapshot::plr_view_range is the Rust engine's persisted FoW
        // range projection; the broader visibility-map subsystem is separate.
        let object_script = r#"#strict 2
public func Probe(other) {
  var clamped_ok = SetPlrViewRange(50);
  var clamped = GetObjectVal("PlrViewRange", 0, this());
  var exact_ok = SetPlrViewRange(50, other, true);
  var exact = GetObjectVal("PlrViewRange", 0, other);
  var negative_ok = SetPlrViewRange(-1);
  var negative = GetObjectVal("PlrViewRange", 0, this());
  return [clamped_ok, clamped, exact_ok, exact, negative_ok, negative];
}
"#;
        let scenario_script = r#"#strict 2
func Probe(target) {
  var result = SetPlrViewRange(50);
  if (result == 0 && GetType(result) == C4V_Int)
    return SetPlrViewRange(66, target, true);
  return SetPlrViewRange(77, target, true);
}
"#;

        let mut engine = Engine::with_seed(23);
        engine.register_definition(
            Definition::from_script("VIEW", "View-range probe", object_script)
                .expect("view-range probe compiles"),
        )?;
        let caller = engine.spawn_object(SpawnConfig::new("VIEW"))?;
        let target = engine.spawn_object(SpawnConfig::new("VIEW"))?;

        let caller_index = engine.find_object_index(caller).expect("caller exists");
        let result = engine.call_object_function(
            caller_index,
            "Probe",
            vec![Value::Object(target.as_u64())],
        )?;
        assert_eq!(
            result,
            Value::Array(vec![
                Value::Int(1),
                Value::Int(128),
                Value::Int(1),
                Value::Int(50),
                Value::Int(1),
                Value::Int(-1),
            ]),
            "the host result and same-call PlrViewRange reflection match C++"
        );

        let snapshot = engine.snapshot();
        assert_eq!(
            snapshot.object(caller).map(|object| object.plr_view_range),
            Some(-1)
        );
        assert_eq!(
            snapshot.object(target).map(|object| object.plr_view_range),
            Some(50)
        );

        engine.install_scenario_script_with_convention(
            "Global view-range probe",
            scenario_script,
            true,
        )?;
        engine.call_scenario_script_function(
            "Probe",
            vec![Value::Object(target.as_u64())],
        )?;
        assert_eq!(
            engine
                .snapshot()
                .object(target)
                .map(|object| object.plr_view_range),
            Some(77),
            "strict-2 GetType reports C4V_Any for the falsy integer result"
        );
        Ok(())
    }

    #[test]
    fn global_effect_timer_kill_semantics_follow_cpp() {
        // C4Effect::Execute (C4Effect.cpp:342-357) with pObj=nullptr
        // (C4Game.cpp:831): an Fx*Timer returning C4Fx_Execute_Kill (-1,
        // C4Effects.h:40) kills the elapsed GLOBAL effect via
        // C4Effect::Kill, which fires Fx*Stop(nil, iNumber)
        // (C4Effect.cpp:389-392); an elapsed interval with NO timer
        // function kills too (the else arm :355-357); a zero interval
        // never fires.
        let script = r#"
        global func Initialize(state, random) {
            var no_target;
            AddEffect("Inert", no_target, 100, 0);
            AddEffect("Doomed", no_target, 150, 2);
            AddEffect("Mute", no_target, 200, 3);
            return 0;
        }

        global func FxDoomedTimer(target, number, time) {
            if (time >= 4) { return CastBool(-1); }
            return 0;
        }

        global func FxDoomedStop(target, number, reason, temp) {
            return 0;
        }

        global func Step(state, frame, random) { return 0; }
        "#;

        let call_log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let mut hooks = DebuggerHooks::new();
        {
            let call_log = Arc::clone(&call_log);
            hooks.set_on_call(move |name, _args| {
                call_log.lock().unwrap().push(name.to_string());
            });
        }

        let mut definition =
            Definition::from_script("Actor", "Actor", script).expect("script compiles");
        definition.set_debugger_hooks(hooks);

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine
            .spawn_object(SpawnConfig::new("Actor"))
            .expect("spawn succeeds");

        assert_eq!(engine.global_effects().len(), 3);
        for _ in 0..8 {
            engine.tick_without_snapshot().expect("tick succeeds");
        }

        let names: Vec<&str> = engine
            .global_effects()
            .iter()
            .map(|effect| effect.name.as_str())
            .collect();
        assert_eq!(
            names,
            vec!["Inert"],
            "Doomed killed by -1 at iTime 4, Mute killed at its first \
             timerless gate, zero-interval Inert survives"
        );
        let calls = call_log.lock().unwrap().clone();
        let stop_calls = calls.iter().filter(|name| *name == "FxDoomedStop").count();
        assert_eq!(
            stop_calls, 1,
            "C4Effect::Kill fires the real Fx*Stop(nil, iNumber) once"
        );
    }

    #[test]
    fn global_effect_timer_walk_exposes_old_time_and_removes_later_effect_inline() {
        // The global effect list uses the same one-node-at-a-time Execute
        // walk as an object list. GA sees GB's old iTime, removes GB and
        // completes its Stop before GC's later timer fires.
        let script = r#"#strict 3
        static iOrder, iSeenB;

        func Install() {
            iOrder = 0;
            iSeenB = -1;
            AddEffect("GA", nil, 100, 1, this());
            AddEffect("GB", nil, 200, 1, this());
            AddEffect("GC", nil, 300, 1, this());
        }

        global func FxGATimer(target, int number, int time) {
            iOrder = iOrder * 10 + 1;
            iSeenB = GetEffect("GB", nil, 0, 6);
            RemoveEffect("GB", nil);
            return 0;
        }

        global func FxGBTimer() { iOrder = iOrder * 10 + 9; }
        global func FxGBStop() { iOrder = iOrder * 10 + 2; }
        global func FxGCTimer() { iOrder = iOrder * 10 + 3; }
        "#;

        let mut definition =
            Definition::from_script("GTW1", "Global timer walk", script)
                .expect("script compiles");
        definition.set_c4_callback_convention(true);

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(definition)
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("GTW1"))
            .expect("command target spawns");
        let idx = engine.find_object_index(id).expect("command target exists");
        engine
            .call_object_function(idx, "Install", Vec::new())
            .expect("effects install");

        engine.tick_without_snapshot().expect("timer frame succeeds");

        let snapshot = engine.snapshot();
        assert_eq!(
            snapshot.script_globals.named.get("iOrder"),
            Some(&Value::Int(123))
        );
        assert_eq!(
            snapshot.script_globals.named.get("iSeenB"),
            Some(&Value::Int(0)),
            "GA runs before C4Effect::Execute increments the later GB"
        );
        assert!(
            !engine
                .global_effects()
                .iter()
                .any(|effect| effect.name == "GB" && effect.priority != 0),
            "GB's inline removal prevents its already-eligible timer"
        );
        for name in ["GA", "GC"] {
            assert_eq!(
                engine
                    .global_effects()
                    .iter()
                    .find(|effect| effect.name == name && effect.priority != 0)
                    .map(|effect| effect.timer),
                Some(1),
                "the surviving global effect advances exactly once"
            );
        }
    }

    #[test]
    fn global_effect_timer_walk_executes_new_higher_effect_same_frame() {
        // A global node inserted above the current cursor is reached by the
        // same Execute call and gets its first interval-one timer at iTime 1.
        let script = r#"#strict 3
        static iOrder, iNewTime;

        func Install() {
            iOrder = 0;
            iNewTime = -1;
            AddEffect("GA", nil, 100, 1, this());
        }

        global func FxGATimer(target) {
            iOrder = iOrder * 10 + 1;
            if (!GetEffect("GNew", nil))
                AddEffect("GNew", nil, 200, 1, this());
            return 0;
        }

        global func FxGNewTimer(target, int number, int time) {
            iOrder = iOrder * 10 + 2;
            iNewTime = time;
            return 0;
        }
        "#;

        let mut definition =
            Definition::from_script("GTW2", "Global timer insertion", script)
                .expect("script compiles");
        definition.set_c4_callback_convention(true);

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(definition)
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("GTW2"))
            .expect("command target spawns");
        let idx = engine.find_object_index(id).expect("command target exists");
        engine
            .call_object_function(idx, "Install", Vec::new())
            .expect("driver installs");

        engine.tick_without_snapshot().expect("timer frame succeeds");

        let snapshot = engine.snapshot();
        assert_eq!(
            snapshot.script_globals.named.get("iOrder"),
            Some(&Value::Int(12))
        );
        assert_eq!(
            snapshot.script_globals.named.get("iNewTime"),
            Some(&Value::Int(1))
        );
        assert_eq!(
            engine
                .global_effects()
                .iter()
                .find(|effect| effect.name == "GNew" && effect.priority != 0)
                .map(|effect| effect.timer),
            Some(1),
            "the newly inserted global node advances in its creation frame"
        );
    }

    #[test]
    fn global_effect_timer_kill_drops_removed_upper_readd_event() {
        // The global Execute walk also completes a timer-result Kill inline.
        // GLower's Stop removes inactive GUpper, invalidating both its later
        // timer turn and the pending Start(1) reactivation.
        let script = r#"#strict 3
        static iOrder, iStaleReadds, iUpperTimers;

        func Install() {
            iOrder = iStaleReadds = iUpperTimers = 0;
            AddEffect("GLower", nil, 100, 1, this());
            AddEffect("GUpper", nil, 200, 1, this());
        }

        global func FxGLowerTimer() {
            iOrder = iOrder * 10 + 1;
            return -1;
        }

        global func FxGLowerStop() {
            iOrder = iOrder * 10 + 3;
            RemoveEffect("GUpper", nil);
            return 0;
        }

        global func FxGUpperTimer() {
            iOrder = iOrder * 10 + 9;
            ++iUpperTimers;
            return 0;
        }

        global func FxGUpperStop(target, int number, int reason, bool temp) {
            if (reason == 1 && temp)
                iOrder = iOrder * 10 + 2;
            else
                iOrder = iOrder * 10 + 4;
            return 0;
        }

        global func FxGUpperStart(target, int number, int temp) {
            // Start(2) for an inactive-effect kill belongs to L026; this
            // test only rejects the stale ordinary temp readd Start(1).
            if (temp == 1) {
                iOrder = iOrder * 10 + 8;
                ++iStaleReadds;
            }
            return 0;
        }
        "#;

        let mut definition =
            Definition::from_script("GTW3", "Global timer kill walk", script)
                .expect("script compiles");
        definition.set_c4_callback_convention(true);

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(definition)
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("GTW3"))
            .expect("command target spawns");
        let idx = engine.find_object_index(id).expect("command target exists");
        engine
            .call_object_function(idx, "Install", Vec::new())
            .expect("effects install");

        engine.tick_without_snapshot().expect("timer frame succeeds");

        let snapshot = engine.snapshot();
        assert_eq!(
            snapshot.script_globals.named.get("iOrder"),
            Some(&Value::Int(1234))
        );
        assert_eq!(
            snapshot.script_globals.named.get("iUpperTimers"),
            Some(&Value::Int(0))
        );
        assert_eq!(
            snapshot.script_globals.named.get("iStaleReadds"),
            Some(&Value::Int(0))
        );
        assert!(
            !engine
                .global_effects()
                .iter()
                .any(|effect| effect.name == "GUpper" && effect.priority != 0),
            "GUpper may remain linked dead until Execute revisits it, but it must not be active"
        );
    }

    #[test]
    fn global_effect_stop_deny_recovers_like_cpp() {
        // C4Effect::Kill (C4Effect.cpp:389-396): an Fx*Stop returning
        // C4Fx_Stop_Deny (-1, C4Effects.h:42) refuses the removal — the
        // effect recovers its priority and stays in the GLOBAL list.
        let script = r#"
        global func Initialize(state, random) {
            var no_target;
            AddEffect("Stubborn", no_target, 100, 2);
            return 0;
        }

        global func FxStubbornTimer(target, number, time) {
            if (time >= 2) { return CastBool(-1); }
            return 0;
        }

        global func FxStubbornStop(target, number, reason, temp) {
            return CastBool(-1);
        }

        global func Step(state, frame, random) { return 0; }
        "#;

        let definition =
            Definition::from_script("Actor", "Actor", script).expect("script compiles");
        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine
            .spawn_object(SpawnConfig::new("Actor"))
            .expect("spawn succeeds");

        for _ in 0..6 {
            engine.tick_without_snapshot().expect("tick succeeds");
        }
        assert_eq!(
            engine.global_effects().len(),
            1,
            "the denied removal keeps the effect alive through repeated kills"
        );
        assert_eq!(engine.global_effects()[0].name, "Stubborn");
        assert_eq!(
            engine.global_effects()[0].timer,
            6,
            "iTime keeps advancing on the recovered effect"
        );
    }

    #[test]
    fn global_effect_kill_brackets_upper_effects_like_cpp() {
        // C4Effect::Kill (C4Effect.cpp:365-405): the real removal is
        // bracketed by temp-deactivating all upper effects
        // (TempRemoveUpperEffects, :370-374 — Fx*Stop with fTemp) and
        // reactivating them after the Stop (TempReaddUpperEffects, :404 —
        // Fx*Start(C4FxCall_Temp)). Execute(nullptr) kills pass
        // pObj=nullptr, so the GLOBAL list takes the same bracket.
        let script = r#"
        global func Initialize(state, random) {
            var no_target;
            AddEffect("Upper", no_target, 200, 0);
            AddEffect("Mute", no_target, 150, 3);
            return 0;
        }

        global func FxUpperStart(target, number, temp) { return 0; }
        global func FxUpperStop(target, number, reason, temp) { return 0; }

        global func Step(state, frame, random) { return 0; }
        "#;

        let call_log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let mut hooks = DebuggerHooks::new();
        {
            let call_log = Arc::clone(&call_log);
            hooks.set_on_call(move |name, _args| {
                call_log.lock().unwrap().push(name.to_string());
            });
        }

        let mut definition =
            Definition::from_script("Actor", "Actor", script).expect("script compiles");
        definition.set_debugger_hooks(hooks);

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine
            .spawn_object(SpawnConfig::new("Actor"))
            .expect("spawn succeeds");

        for _ in 0..4 {
            engine.tick_without_snapshot().expect("tick succeeds");
        }

        let names: Vec<&str> = engine
            .global_effects()
            .iter()
            .map(|effect| effect.name.as_str())
            .collect();
        assert_eq!(
            names,
            vec!["Upper"],
            "Mute dies at its timerless gate; the bracketed Upper survives"
        );
        let calls = call_log.lock().unwrap().clone();
        let stop_calls = calls.iter().filter(|name| *name == "FxUpperStop").count();
        let start_calls = calls.iter().filter(|name| *name == "FxUpperStart").count();
        assert_eq!(
            stop_calls, 1,
            "Mute's kill temp-removes the upper effect (Fx*Stop fTemp)"
        );
        assert_eq!(
            start_calls, 2,
            "one Fx*Start from the C4Effect ctor inside AddEffect \
             (C4Effect.cpp:128-129), one temp reactivation after the kill \
             (TempReaddUpperEffects, C4Effect.cpp:505)"
        );
    }

    #[test]
    fn nil_target_add_effect_dispatches_fx_start_like_c4effect_ctor() {
        // The C4Effect ctor runs Fx*Start synchronously inside FnAddEffect
        // (C4Effect.cpp:128-131): pFnStart->Exec(pCommandTarget,
        // {C4VObj(pForObj), C4VInt(iNumber), C4VInt(0), rVal1..rVal4}) —
        // pForObj is nullptr (nil) for a GLOBAL effect (ctor list select
        // :74). A C4Fx_Start_Deny (-1) return marks the effect dead: it is
        // deleted by the next Execute without a Stop callback
        // (:128-131 + Execute :328-336).
        let script = r#"
        global func Initialize(state, random) {
            var no_value;
            AddEffect("Flash", no_value, 200, 5, no_value, no_value, 42, 77);
            AddEffect("Vetoed", no_value, 200, 5);
            return 0;
        }

        global func FxFlashStart(target, number, temp, var1, var2) {
            var no_target;
            // pForObj is nil for global effects (C4Effect.cpp:129).
            if (target) { return 0; }
            if (temp) { return 0; }
            if (var2 != 77) { return -1; }
            EffectVar(0, no_target, number) = var1 + 1;
            return 0;
        }

        global func FxVetoedStart(target, number, temp) {
            // C4Effect reads the callback result through getInt(); a raw Bool
            // retains -1 in Data.Int and must still mean Start_Deny.
            return CastBool(-1);
        }

        global func FxVetoedStop(target, number, reason, temp) {
            var no_target;
            // must never fire: a Start-denied effect dies without Stop.
            EffectVar(0, no_target, number) = -99;
            return 0;
        }

        global func Step(state, frame, random) { return 0; }
        "#;

        let definition =
            Definition::from_script("Actor", "Actor", script).expect("script compiles");
        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine
            .spawn_object(SpawnConfig::new("Actor"))
            .expect("spawn succeeds");

        assert!(engine
            .global_effects()
            .iter()
            .any(|effect| effect.name == "Vetoed" && effect.priority == 0));
        let flash = engine
            .global_effects()
            .iter()
            .find(|effect| effect.name == "Flash")
            .expect("Flash remains active");
        assert_eq!(
            flash.var(0),
            EffectVarValue::Int(43),
            "Fx*Start(nil, iNumber, 0, rVal1, rVal2) ran synchronously \
             inside AddEffect and saw both constructor values"
        );
        assert_eq!(
            flash.vars(),
            &[EffectVarValue::Int(43)],
            "constructor rVals are transient; only the explicit EffectVar write persists"
        );
        engine.tick_without_snapshot().expect("global Execute cleans the dead node");
        assert_eq!(
            engine
                .global_effects()
                .iter()
                .map(|effect| effect.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Flash"]
        );
    }

    #[test]
    fn effect_list_orders_ascending_by_priority_magnitude() {
        // C4Effect registration (C4Effect.cpp:80-94): the new effect is
        // inserted after all effects with |iPriority| < iPrio and before the
        // first with |iPriority| >= iPrio — the list (and therefore the
        // execution order) ascends by priority magnitude, new-before-equal.
        let script = r#"#strict 3
        global func Initialize(state, random) {
            return { effects = [
                { op = "add", name = "High", priority = 150 },
                { op = "add", name = "Low", priority = 50 },
                { op = "add", name = "Mid", priority = 100 },
                { op = "add", name = "Mid2", priority = 100 }
            ] };
        }

        global func Step(state, frame, random) {
            return nil;
        }
        "#;

        let definition =
            Definition::from_script("Actor", "Actor", script).expect("script compiles");
        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(definition)
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("Actor"))
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        let names: Vec<&str> = object
            .effects
            .iter()
            .map(|effect| effect.name.as_str())
            .collect();
        assert_eq!(names, vec!["Low", "Mid2", "Mid", "High"]);
    }

    #[test]
    fn effect_callbacks_resolve_via_command_target_definition() {
        // C4Effect::GetCallbackScript: Fx* functions live in the command
        // target's def script (here via idCommandTarget — the sixth
        // AddEffect argument), NOT in the affected object's script. The
        // spell def's FxBuffTimer drains the host object's energy.
        let host_script = r#"
        global func Initialize(state, random) {
            AddEffect("Buff", this(), 100, 2, 0, SPEL);
            return 0;
        }

        global func Step(state, frame, random) {
            return 0;
        }
        "#;
        let spell_script = r#"
        global func FxBuffTimer(state, effect, timer) {
            DoEnergy(-5);
            return 0;
        }
        "#;

        let mut host = Definition::from_script("HOST", "Host", host_script).unwrap();
        host.set_physical(PhysicalInfo {
            energy: 50_000,
            ..PhysicalInfo::default()
        });
        let spell = Definition::from_script("SPEL", "Spell", spell_script).unwrap();
        let mut engine = Engine::with_seed(7);
        engine.register_definition(host).expect("host registers");
        engine.register_definition(spell).expect("spell registers");
        let id = engine
            .spawn_object(SpawnConfig::new("HOST").with_energy(50_000))
            .expect("spawn succeeds");

        for _ in 0..4 {
            engine.tick_without_snapshot().expect("tick succeeds");
        }
        let idx = engine.find_object_index(id).expect("object exists");
        assert_eq!(
            engine.objects[idx].state.effects.len(),
            1,
            "the spell-script timer keeps the effect alive (a miss would \
             kill it as timerless)"
        );
        assert_eq!(
            engine.objects[idx].state.energy, 40_000,
            "FxBuffTimer (DoEnergy(-5) = -5000 raw, C4Object.cpp:1347) ran \
             in the spell def's script at iTime 2 and 4"
        );
    }

    #[test]
    fn removed_command_target_silently_kills_object_and_global_effects() {
        // C4Game::ClearPointers -> C4Object/C4Effect::ClearPointers marks
        // every foreign effect using the removed object as its command
        // target dead without a Stop callback. The dead node remains
        // addressable by number until the list's next Execute pass, while
        // named lookup/counting skip it. A pure idCommandTarget is not an
        // object pointer and must survive the sweep.
        use std::sync::{Arc, Mutex};

        let carrier_script = r#"#strict 2
local object_no, global_no, id_no;

public func Arm(pTarget) {
    var no_target;
    id_no = AddEffect("DefinitionBound", this(), 100, 5, no_target, CMND);
    object_no = AddEffect("ObjectBound", this(), 100, 5, pTarget);
    global_no = AddEffect("GlobalBound", no_target, 100, 5, pTarget);
    return [object_no, global_no, id_no];
}

public func RemoveAndInspect(pTarget) {
    var no_value;
    RemoveObject(pTarget);
    return [
        GetEffect("ObjectBound", this()),
        GetEffectCount("ObjectBound", this()),
        GetEffect(no_value, this(), object_no, 1),
        GetEffect(no_value, this(), object_no, 4),
        GetEffect(no_value, this(), object_no, 5) == CMND,
        GetEffect("GlobalBound", no_value),
        GetEffectCount("GlobalBound", no_value),
        GetEffect(no_value, no_value, global_no, 1),
        GetEffect("DefinitionBound", this()),
        GetEffectCount(no_value, this()),
        GetEffect("DefinitionBound", this(), 0, 5) == CMND
    ];
}
"#;
        let command_target_script = r#"#strict 2
global func FxObjectBoundTimer(pTarget, iNumber, iTime) { return 0; }
global func FxObjectBoundDamage(pTarget, iNumber, iChange, iCause, iCausedBy) { return iChange; }
global func FxObjectBoundStop(pTarget, iNumber, iReason, fTemp) { return 0; }
global func FxGlobalBoundTimer(pTarget, iNumber, iTime) { return 0; }
global func FxGlobalBoundStop(pTarget, iNumber, iReason, fTemp) { return 0; }
global func FxDefinitionBoundTimer(pTarget, iNumber, iTime) { return 0; }
global func FxDefinitionBoundStop(pTarget, iNumber, iReason, fTemp) { return 0; }
"#;

        let calls: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let mut hooks = DebuggerHooks::new();
        {
            let calls = Arc::clone(&calls);
            hooks.set_on_call(move |name, _args| {
                calls.lock().unwrap().push(name.to_string());
            });
        }

        let carrier = Definition::from_script("CARR", "Carrier", carrier_script)
            .expect("carrier script compiles");
        let mut command_target =
            Definition::from_script("CMND", "Command target", command_target_script)
                .expect("command-target script compiles");
        command_target.set_debugger_hooks(hooks);

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(carrier)
            .expect("carrier registers");
        engine
            .register_definition(command_target)
            .expect("command target registers");
        let carrier_id = engine
            .spawn_object(SpawnConfig::new("CARR"))
            .expect("carrier spawns");
        let target_id = engine
            .spawn_object(SpawnConfig::new("CMND"))
            .expect("command target spawns");
        let carrier_idx = engine
            .find_object_index(carrier_id)
            .expect("carrier exists");

        assert_eq!(
            engine
                .call_object_function(
                    carrier_idx,
                    "Arm",
                    vec![object_reference_value(target_id)],
                )
                .expect("effects install"),
            Value::Array(vec![Value::Int(2), Value::Int(1), Value::Int(1)])
        );
        for _ in 0..5 {
            engine.tick_without_snapshot().expect("pre-removal tick succeeds");
        }
        {
            let calls = calls.lock().unwrap();
            for callback in [
                "FxObjectBoundTimer",
                "FxGlobalBoundTimer",
                "FxDefinitionBoundTimer",
            ] {
                assert_eq!(
                    calls.iter().filter(|name| name.as_str() == callback).count(),
                    1,
                    "{callback} is live before its object target is removed"
                );
            }
        }
        calls.lock().unwrap().clear();

        let carrier_idx = engine
            .find_object_index(carrier_id)
            .expect("carrier still exists");
        assert_eq!(
            engine
                .call_object_function(
                    carrier_idx,
                    "RemoveAndInspect",
                    vec![object_reference_value(target_id)],
                )
                .expect("target removal succeeds"),
            Value::Array(vec![
                Value::Nil,
                Value::Int(0),
                Value::String("ObjectBound".into()),
                Value::Nil,
                Value::Bool(true),
                Value::Nil,
                Value::Int(0),
                Value::String("GlobalBound".into()),
                Value::Int(1),
                Value::Int(1),
                Value::Bool(true),
            ]),
            "dead object/global nodes remain linked by number but disappear \
             from live lookup; the C4ID-commanded effect stays live"
        );

        let carrier_idx = engine
            .find_object_index(carrier_id)
            .expect("carrier remains after removal");
        engine
            .change_object_damage(carrier_idx, 5, 0, -1)
            .expect("damage traversal succeeds");
        for _ in 0..10 {
            engine.tick_without_snapshot().expect("post-removal tick succeeds");
        }

        let calls = calls.lock().unwrap();
        for callback in [
            "FxObjectBoundTimer",
            "FxObjectBoundDamage",
            "FxObjectBoundStop",
            "FxGlobalBoundTimer",
            "FxGlobalBoundStop",
        ] {
            assert_eq!(
                calls.iter().filter(|name| name.as_str() == callback).count(),
                0,
                "removed command target must suppress {callback}"
            );
        }
        assert_eq!(
            calls
                .iter()
                .filter(|name| name.as_str() == "FxDefinitionBoundTimer")
                .count(),
            2,
            "the C4ID-only command target remains scheduled across two intervals"
        );
        drop(calls);

        let carrier_idx = engine
            .find_object_index(carrier_id)
            .expect("carrier remains after ticks");
        assert_eq!(
            engine.objects[carrier_idx]
                .state
                .effects
                .iter()
                .map(|effect| effect.name.as_str())
                .collect::<Vec<_>>(),
            vec!["DefinitionBound"],
            "the next object Execute silently unlinks its dead node"
        );
        assert!(
            engine.global_effects().is_empty(),
            "the next global Execute silently unlinks its dead node"
        );
    }

    #[test]
    fn effect_check_chain_denies_new_effects() {
        // C4Effect::Check (C4Effect.cpp:97-116, 167-189): before a new
        // effect validates, existing effects with iPriority >= the new
        // priority get their Fx<Name>Effect callback with the new effect's
        // name — C4Fx_Effect_Deny (-1, C4Effects.h:36) blocks the creation
        // entirely (no Start, no Stop). Priority-1 effects skip the chain.
        let script = r#"#strict 3
        global func Initialize(state, random) {
            return { effects = [ { op = "add", name = "Armor", priority = 200, interval = 0 } ] };
        }

        global func FxArmorEffect(state, effect, new_name) {
            if (new_name == "Fire") {
                return -1;
            }
            return nil;
        }

        global func FxFireStart(state, effect) {
            return nil;
        }

        global func Step(state, frame, random) {
            if (frame == 2) {
                return { effects = [
                    { op = "add", name = "Fire", priority = 100, interval = 0 },
                    { op = "add", name = "Buff", priority = 100, interval = 0 }
                ] };
            }
            if (frame == 3) {
                return { effects = [ { op = "add", name = "Fire", priority = 1, interval = 0 } ] };
            }
            return nil;
        }
        "#;

        let definition =
            Definition::from_script("Actor", "Actor", script).expect("script compiles");
        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(definition)
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("Actor"))
            .expect("spawn succeeds");

        engine.tick_without_snapshot().expect("tick succeeds");
        let second = engine.tick().expect("tick succeeds");
        let object = second.object(id).expect("object present");
        let names: Vec<&str> = object
            .effects
            .iter()
            .map(|effect| effect.name.as_str())
            .collect();
        assert_eq!(
            names,
            vec!["Buff", "Armor"],
            "Armor denies Fire; Buff passes the chain"
        );

        let third = engine.tick().expect("tick succeeds");
        let object = third.object(id).expect("object present");
        assert!(
            object.effects.iter().any(|effect| effect.name == "Fire"),
            "priority-1 effects skip the check chain (C4Effect.cpp:170)"
        );
    }

    #[test]
    fn effect_check_deny_short_circuits_remaining_checkers() {
        // C4Effect::Check (C4Effect.cpp:283-285): the FIRST Fx<Name>Effect
        // answering C4Fx_Effect_Deny returns immediately — checkers later
        // in the chain (higher priority, asked in ascending list order) are
        // never called for the denied effect.
        let script = r#"#strict 3
        global func Initialize(state, random) {
            return { effects = [ { op = "add", name = "Armor", priority = 100, interval = 0 } ] };
        }

        global func FxArmorEffect(state, effect, new_name) {
            if (new_name == "Fire") {
                return -1;
            }
            return nil;
        }

        global func FxWatcherEffect(state, effect, new_name) {
            return nil;
        }

        global func Step(state, frame, random) {
            if (frame == 2) {
                return { effects = [ { op = "add", name = "Watcher", priority = 200, interval = 0 } ] };
            }
            if (frame == 3) {
                return { effects = [ { op = "add", name = "Fire", priority = 50, interval = 0 } ] };
            }
            return nil;
        }
        "#;

        let call_log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let mut hooks = DebuggerHooks::new();
        {
            let call_log = Arc::clone(&call_log);
            hooks.set_on_call(move |name, _args| {
                call_log.lock().unwrap().push(name.to_string());
            });
        }

        let mut definition =
            Definition::from_script("Actor", "Actor", script).expect("script compiles");
        definition.set_debugger_hooks(hooks);

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(definition)
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("Actor"))
            .expect("spawn succeeds");

        engine.tick_without_snapshot().expect("tick succeeds");
        engine.tick_without_snapshot().expect("tick succeeds");
        let third = engine.tick().expect("tick succeeds");
        let object = third.object(id).expect("object present");
        assert!(
            !object.effects.iter().any(|effect| effect.name == "Fire"),
            "Armor denies Fire"
        );

        let calls = call_log.lock().unwrap().clone();
        // Adding Fire asks Armor (100) first in ascending list order; its
        // deny must stop the chain before Watcher (200) is reached.
        assert!(
            calls.iter().any(|name| name == "FxArmorEffect"),
            "Armor is asked about Fire"
        );
        assert!(
            !calls.iter().any(|name| name == "FxWatcherEffect"),
            "the deny returns immediately (C4Effect.cpp:283-285) — Watcher \
             is never asked about the already-denied Fire"
        );
    }

    #[test]
    fn effect_check_chain_asks_same_name_effects() {
        // C4Effect::Check (C4Effect.cpp:278-282) has NO name filter: the
        // new effect is inserted BEFORE existing effects of equal priority
        // (new-before-equal, C4Effect.cpp:80-94), so an already-present
        // same-name effect sits in the pNext chain and its Fx<Name>Effect
        // callback IS asked about the new addition.
        let script = r#"#strict 3
        global func Initialize(state, random) {
            return { effects = [ { op = "add", name = "Guard", priority = 100, interval = 0 } ] };
        }

        global func FxGuardEffect(state, effect, new_name) {
            return nil;
        }

        global func Step(state, frame, random) {
            if (frame == 2) {
                return { effects = [ { op = "add", name = "Guard", priority = 100, interval = 0 } ] };
            }
            return nil;
        }
        "#;

        let call_log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let mut hooks = DebuggerHooks::new();
        {
            let call_log = Arc::clone(&call_log);
            hooks.set_on_call(move |name, _args| {
                call_log.lock().unwrap().push(name.to_string());
            });
        }

        let mut definition =
            Definition::from_script("Actor", "Actor", script).expect("script compiles");
        definition.set_debugger_hooks(hooks);

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(definition)
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("Actor"))
            .expect("spawn succeeds");

        engine.tick_without_snapshot().expect("tick succeeds");
        let second = engine.tick().expect("tick succeeds");
        let object = second.object(id).expect("object present");
        let names: Vec<&str> = object
            .effects
            .iter()
            .map(|effect| effect.name.as_str())
            .collect();
        assert_eq!(names, vec!["Guard", "Guard"], "same-name effects coexist");

        let calls = call_log.lock().unwrap().clone();
        assert_eq!(
            calls
                .iter()
                .filter(|name| *name == "FxGuardEffect")
                .count(),
            1,
            "the pre-existing Guard is asked about the new same-name Guard \
             (C4Effect.cpp:278-282 filters by priority only, not name)"
        );
    }

    #[test]
    fn effect_start_deny_drops_effect_without_stop() {
        // C4Fx_Start_Deny (-1, C4Effects.h:43): an FxStart returning it
        // marks the effect dead before it ever validates
        // (C4Effect.cpp:128-131) — it disappears without a Stop callback.
        let script = r#"#strict 3
        global func Initialize(state, random) {
            return { effects = [ { op = "add", name = "Denied", interval = 2 } ] };
        }

        global func FxDeniedStart(state, effect) {
            return -1;
        }

        global func FxDeniedTimer(state, effect, timer) {
            return nil;
        }

        global func FxDeniedStop(state, effect, reason) {
            return nil;
        }

        global func Step(state, frame, random) {
            return nil;
        }
        "#;

        let call_log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let mut hooks = DebuggerHooks::new();
        {
            let call_log = Arc::clone(&call_log);
            hooks.set_on_call(move |name, _args| {
                call_log.lock().unwrap().push(name.to_string());
            });
        }

        let mut definition =
            Definition::from_script("Actor", "Actor", script).expect("script compiles");
        definition.set_debugger_hooks(hooks);

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(definition)
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("Actor"))
            .expect("spawn succeeds");

        let mut last = None;
        for _ in 0..3 {
            last = Some(engine.tick().expect("tick succeeds"));
        }
        let snapshot = last.expect("snapshot present");
        let object = snapshot.object(id).expect("object present");
        assert!(object.effects.is_empty(), "denied effect never validates");

        let calls = call_log.lock().unwrap().clone();
        assert!(
            !calls.iter().any(|name| name == "FxDeniedTimer"),
            "denied effects never tick"
        );
        assert!(
            !calls.iter().any(|name| name == "FxDeniedStop"),
            "dead effects are deleted without the Stop callback"
        );
    }

    #[test]
    fn new_effect_start_temp_removes_and_readds_upper_effects() {
        // C4Effect ctor (C4Effect.cpp:118-133): when a new effect WITH an
        // Fx*Start validates, active higher-priority effects are
        // temp-deactivated first — Fx*Stop(reason temp, fTemp = true),
        // high to low (TempRemoveUpperEffects, C4Effect.cpp:473-492) — and
        // reactivated after the new Start via Fx*Start(C4FxCall_Temp),
        // low to high (TempReaddUpperEffects, C4Effect.cpp:494-510). A new
        // effect WITHOUT an Fx*Start skips the whole bracket
        // (`fRemoveUpper && pNext && pFnStart`, C4Effect.cpp:123).
        let script = r#"#strict 3
        global func Initialize(state, random) {
            return { effects = [ { op = "add", name = "Upper", priority = 200, interval = 0 } ] };
        }

        global func FxUpperStart(state, effect, temp) {
            return nil;
        }

        global func FxUpperStop(state, effect, reason, temp) {
            return nil;
        }

        global func FxLowerStart(state, effect) {
            return nil;
        }

        global func Step(state, frame, random) {
            if (frame == 2) {
                return { effects = [ { op = "add", name = "Lower", priority = 100, interval = 0 } ] };
            }
            if (frame == 3) {
                return { effects = [ { op = "add", name = "Plain", priority = 100, interval = 0 } ] };
            }
            return nil;
        }
        "#;

        let call_log: Arc<Mutex<Vec<(String, Vec<Value>)>>> = Arc::new(Mutex::new(Vec::new()));
        let mut hooks = DebuggerHooks::new();
        {
            let call_log = Arc::clone(&call_log);
            hooks.set_on_call(move |name, args| {
                call_log
                    .lock()
                    .unwrap()
                    .push((name.to_string(), args.to_vec()));
            });
        }

        let mut definition =
            Definition::from_script("Actor", "Actor", script).expect("script compiles");
        definition.set_debugger_hooks(hooks);

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(definition)
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("Actor"))
            .expect("spawn succeeds");

        engine.tick_without_snapshot().expect("tick succeeds");
        call_log.lock().unwrap().clear();

        let second = engine.tick().expect("tick succeeds");
        let object = second.object(id).expect("object present");
        let names: Vec<&str> = object
            .effects
            .iter()
            .map(|effect| effect.name.as_str())
            .collect();
        assert_eq!(names, vec!["Lower", "Upper"], "both effects survive");

        let calls: Vec<(String, Vec<Value>)> = call_log
            .lock()
            .unwrap()
            .iter()
            .filter(|(name, _)| name.starts_with("Fx"))
            .cloned()
            .collect();
        let call_names: Vec<&str> = calls.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            call_names,
            vec!["FxUpperStop", "FxLowerStart", "FxUpperStart"],
            "the validating Start is bracketed by the upper effect's temp \
             stop and temp readd (C4Effect.cpp:122-133)"
        );
        let (_, stop_args) = &calls[0];
        assert_eq!(
            stop_args.get(2),
            Some(&Value::Int(1)),
            "the temp stop's reason is C4FxCall_Temp (C4Effect.cpp:489)"
        );
        assert_eq!(
            stop_args.get(3),
            Some(&Value::Bool(true)),
            "fTemp = true (C4Effect.cpp:489)"
        );
        let (_, readd_args) = &calls[2];
        assert_eq!(
            readd_args.get(2),
            Some(&Value::Int(1)),
            "the temp readd's Start gets iTemp = C4FxCall_Temp \
             (C4Effect.cpp:505, C4Effects.h:47)"
        );

        call_log.lock().unwrap().clear();
        let third = engine.tick().expect("tick succeeds");
        let object = third.object(id).expect("object present");
        assert_eq!(object.effects.len(), 3, "Plain registers too");
        let calls = call_log.lock().unwrap().clone();
        assert!(
            !calls.iter().any(|(name, _)| name == "FxUpperStop"),
            "an effect without Fx*Start skips the temp bracket \
             (C4Effect.cpp:123)"
        );
    }

    #[test]
    fn effect_kill_temp_removes_and_readds_upper_effects() {
        // C4Effect::Kill (C4Effect.cpp:365-405): killing an active effect
        // temp-deactivates all upper effects first (C4Effect.cpp:370-374),
        // runs the victim's Fx*Stop, then reactivates the uppers
        // (C4Effect.cpp:404) — same bracket as the ctor, without an
        // Fx*Stop requirement on the victim.
        let script = r#"#strict 3
        global func Initialize(state, random) {
            return { effects = [ { op = "add", name = "Upper", priority = 200, interval = 0 } ] };
        }

        global func FxUpperStart(state, effect, temp) {
            return nil;
        }

        global func FxUpperStop(state, effect, int reason, temp) {
            return nil;
        }

        global func FxLowerStop(state, effect, int reason) {
            return nil;
        }

        global func Step(state, frame, random) {
            if (frame == 2) {
                return { effects = [ { op = "add", name = "Lower", priority = 100, interval = 0 } ] };
            }
            if (frame == 3) {
                return { effects = [ { op = "remove", name = "Lower" } ] };
            }
            return nil;
        }
        "#;

        let call_log: Arc<Mutex<Vec<(String, Vec<Value>)>>> = Arc::new(Mutex::new(Vec::new()));
        let mut hooks = DebuggerHooks::new();
        {
            let call_log = Arc::clone(&call_log);
            hooks.set_on_call(move |name, args| {
                call_log
                    .lock()
                    .unwrap()
                    .push((name.to_string(), args.to_vec()));
            });
        }

        let mut definition =
            Definition::from_script("Actor", "Actor", script).expect("script compiles");
        definition.set_debugger_hooks(hooks);

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(definition)
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("Actor"))
            .expect("spawn succeeds");

        engine.tick_without_snapshot().expect("tick succeeds");
        engine.tick_without_snapshot().expect("tick succeeds");
        call_log.lock().unwrap().clear();

        let third = engine.tick().expect("tick succeeds");
        let object = third.object(id).expect("object present");
        let names: Vec<&str> = object
            .effects
            .iter()
            .filter(|effect| effect.priority != 0)
            .map(|effect| effect.name.as_str())
            .collect();
        assert_eq!(names, vec!["Upper"], "only Lower is killed");
        assert!(object
            .effects
            .iter()
            .any(|effect| effect.name == "Lower" && effect.priority == 0));

        let calls: Vec<(String, Vec<Value>)> = call_log
            .lock()
            .unwrap()
            .iter()
            .filter(|(name, _)| name.starts_with("Fx"))
            .cloned()
            .collect();
        let call_names: Vec<&str> = calls.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            call_names,
            vec!["FxUpperStop", "FxLowerStop", "FxUpperStart"],
            "the kill is bracketed by the upper effect's temp stop and \
             temp readd (C4Effect.cpp:370-374,404)"
        );
        let (_, temp_stop_args) = &calls[0];
        assert_eq!(
            temp_stop_args.get(2),
            Some(&Value::Int(1)),
            "the bracket stop receives C4FxCall_Temp"
        );
        assert_eq!(
            temp_stop_args.get(3),
            Some(&Value::Bool(true)),
            "the bracket stop is temporary (fTemp, C4Effect.cpp:489)"
        );
        let (_, kill_stop_args) = &calls[1];
        assert_eq!(
            kill_stop_args.get(2),
            Some(&Value::Int(0)),
            "C4Effect::Kill omits the raw reason slot; strict-3 typed int \
             conversion exposes it as C4FxCall_Normal (0)"
        );

        let fourth = engine.tick().expect("next Execute succeeds");
        let object = fourth.object(id).expect("object present");
        assert_eq!(
            object
                .effects
                .iter()
                .map(|effect| effect.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Upper"]
        );
    }

    #[test]
    fn effect_check_annul_merges_into_accepting_effect() {
        // C4Effect::Check (C4Effect.cpp:287-291, 295-313): a checker
        // answering C4Fx_Effect_Annul (-2) accepts the new effect — the
        // new effect dies without Start or Stop callbacks, and the
        // acceptor's Fx<Name>Add merge seam receives the new effect's
        // name, timer interval and parameters (DoCall PSFS_FxAdd with
        // Par1 = name, Par2 = iTimer, rVal1.., C4Effect.cpp:300-301).
        let script = r#"#strict 3
        global func Initialize(state, random) {
            return { effects = [ { op = "add", name = "Shield", priority = 200, interval = 0 } ] };
        }

        global func FxShieldEffect(new_name, state, effect, unused) {
            if (new_name == "Fire") {
                return -2;
            }
            return nil;
        }

        global func FxShieldAdd(state, effect, new_name, new_interval, strength) {
            return nil;
        }

        global func FxFireStart(state, effect) {
            return nil;
        }

        global func FxFireStop(state, effect, reason) {
            return nil;
        }

        global func Step(state, frame, random) {
            if (frame == 2) {
                AddEffect("Fire", this(), 100, 7, nil, nil, 42);
            }
            return nil;
        }
        "#;

        let call_log: Arc<Mutex<Vec<(String, Vec<Value>)>>> = Arc::new(Mutex::new(Vec::new()));
        let mut hooks = DebuggerHooks::new();
        {
            let call_log = Arc::clone(&call_log);
            hooks.set_on_call(move |name, args| {
                call_log
                    .lock()
                    .unwrap()
                    .push((name.to_string(), args.to_vec()));
            });
        }

        let mut definition =
            Definition::from_script("Actor", "Actor", script).expect("script compiles");
        definition.set_debugger_hooks(hooks);

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(definition)
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("Actor"))
            .expect("spawn succeeds");

        engine.tick_without_snapshot().expect("tick succeeds");
        let second = engine.tick().expect("tick succeeds");
        let object = second.object(id).expect("object present");
        let names: Vec<&str> = object
            .effects
            .iter()
            .filter(|effect| effect.priority != 0)
            .map(|effect| effect.name.as_str())
            .collect();
        assert_eq!(
            names,
            vec!["Shield"],
            "the annulled Fire merges into Shield instead of registering"
        );
        assert!(object
            .effects
            .iter()
            .any(|effect| effect.name == "Fire" && effect.priority == 0));

        let calls = call_log.lock().unwrap().clone();
        let add_calls: Vec<&Vec<Value>> = calls
            .iter()
            .filter(|(name, _)| name == "FxShieldAdd")
            .map(|(_, args)| args)
            .collect();
        assert_eq!(add_calls.len(), 1, "the acceptor's Fx*Add runs once");
        let args = add_calls[0];
        assert_eq!(
            args.get(2),
            Some(&Value::String("Fire".to_string().into())),
            "Par1 is the new effect's name (C4Effect.cpp:300)"
        );
        assert_eq!(
            args.get(3),
            Some(&Value::Int(7)),
            "Par2 is the new effect's timer interval (C4Effect.cpp:300)"
        );
        assert_eq!(
            args.get(4),
            Some(&Value::Int(42)),
            "rVal1 carries the AddEffect parameter (C4Effect.cpp:301)"
        );
        assert!(
            !calls.iter().any(|(name, _)| name == "FxFireStart"),
            "the annulled effect never starts (it stays dead, C4Effect.cpp:108-115)"
        );
        assert!(
            !calls.iter().any(|(name, _)| name == "FxFireStop"),
            "the annulled effect dies without a Stop callback"
        );
    }

    #[test]
    fn effect_check_callback_receives_new_effect_parameters() {
        // Fx*Effect check calls carry the pending AddEffect's rVal1-4
        // (C4Effect.cpp:282) — a checker can decide on the parameters, not
        // just the name. Here Shield denies only strength-42 additions.
        let script = r#"#strict 3
        global func Initialize(state, random) {
            return { effects = [ { op = "add", name = "Shield", priority = 200, interval = 0 } ] };
        }

        // C4Effect::Check's historical ABI is unlike the other Fx calls:
        // [new name, target, checker number, nil, rVal1..4].
        global func FxShieldEffect(new_name, state, effect, unused, strength) {
            if (strength == 42) {
                return CastBool(-1);
            }
            return nil;
        }

        global func Step(state, frame, random) {
            if (frame == 2) {
                AddEffect("Fire", this(), 100, 0, nil, nil, 42);
            }
            if (frame == 3) {
                AddEffect("Frost", this(), 100, 0, nil, nil, 5);
            }
            if (frame == 4) {
                // With no script FxFireStart override, the native AddFunc
                // fallback consumes the same transient constructor payload
                // after Check succeeds.
                AddEffect("Fire", this(), 100, 0, nil, nil, 5);
            }
            return nil;
        }
        "#;

        let definition =
            Definition::from_script("Actor", "Actor", script).expect("script compiles");
        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(definition)
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("Actor"))
            .expect("spawn succeeds");

        engine.tick_without_snapshot().expect("tick succeeds");
        engine.tick_without_snapshot().expect("tick succeeds");
        let third = engine.tick().expect("tick succeeds");
        let object = third.object(id).expect("object present");
        let names: Vec<&str> = object
            .effects
            .iter()
            .map(|effect| effect.name.as_str())
            .collect();
        assert_eq!(
            names,
            vec!["Frost", "Shield"],
            "the checker saw strength 42 on Fire (denied) and 5 on Frost \
             (passed) — C4Effect.cpp:282 forwards rVal1-4"
        );
        let frost = object
            .effects
            .iter()
            .find(|effect| effect.name == "Frost")
            .expect("Frost survives the check chain");
        assert!(
            frost.vars().is_empty(),
            "the rVal strength reaches Fx*Effect without becoming an EffectVar"
        );

        let fourth = engine.tick().expect("native Fire Start tick succeeds");
        let object = fourth.object(id).expect("object remains present");
        assert!(object.on_fire, "native FxFireStart ignites the carrier");
        let fire = object
            .effects
            .iter()
            .find(|effect| effect.name == "Fire")
            .expect("passing Fire effect survives");
        assert_eq!(
            fire.vars(),
            &[
                EffectVarValue::Int(2),
                EffectVarValue::Int(5),
                EffectVarValue::Bool(false),
                EffectVarValue::Nil,
            ],
            "native Start translates rVals into Fire's explicit variables exactly once"
        );
    }

    #[test]
    fn deferred_fire_effect_command_runs_native_start_before_its_first_timer() {
        // Rust's initial-effect command is folded through the same Started
        // event used by a deferred AddEffect outcome. C4Effect constructs
        // Fire by calling its AddFunc-registered native Start immediately;
        // it must not wait for the first timer and reinterpret persistent
        // Fire vars as constructor arguments.
        let script = r#"#strict 3
        global func Initialize(state, random) {
            return { effects = [
                { op = "add", name = "Fire", priority = 100, interval = 1 }
            ] };
        }

        global func Step(state, frame, random) { return nil; }
        "#;

        let definition =
            Definition::from_script("Actor", "Actor", script).expect("script compiles");
        let mut engine = Engine::with_seed(19);
        engine
            .register_definition(definition)
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("Actor"))
            .expect("object spawns");
        let object = engine.object_snapshot(id).expect("object remains present");
        assert!(object.on_fire, "native Start ran during command folding");
        let fire = object
            .effects
            .iter()
            .find(|effect| effect.name == "Fire")
            .expect("Fire effect survives");
        assert_eq!(fire.timer, 0, "no timer frame has elapsed yet");
        assert!(fire.start_dispatched, "Start is complete before the first timer");
        assert_eq!(
            fire.vars(),
            &[
                EffectVarValue::Int(2),
                EffectVarValue::Int(0),
                EffectVarValue::Bool(false),
                EffectVarValue::Nil,
            ]
        );
    }

    #[test]
    fn queued_effect_constructor_values_reach_start_callback() {
        let script = r#"#strict 3
        global func FxProbeStart(state, effect, temp, first, second, third, fourth) {
            return nil;
        }

        global func Step(state, frame, random) { return nil; }
        "#;
        let calls: Arc<Mutex<Vec<Vec<Value>>>> = Arc::new(Mutex::new(Vec::new()));
        let mut hooks = DebuggerHooks::new();
        {
            let calls = Arc::clone(&calls);
            hooks.set_on_call(move |name, args| {
                if name == "FxProbeStart" {
                    calls.lock().unwrap().push(args.to_vec());
                }
            });
        }
        let mut definition =
            Definition::from_script("Actor", "Actor", script).expect("script compiles");
        definition.set_debugger_hooks(hooks);
        let mut engine = Engine::with_seed(23);
        engine
            .register_definition(definition)
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("Actor"))
            .expect("object spawns");
        let constructor_values = [
            Value::Int(21),
            Value::Bool(true),
            Value::String("payload".into()),
            Value::Array(vec![Value::Int(24)]),
        ];
        let effect = EffectState::new("Probe")
            .with_command_target(Some(id.as_u64() as i32));
        let command = QueuedCommand::immediate(ObjectUpdate::default()).with_effects(vec![
            EffectCommand::add_with_constructor_values(effect, constructor_values.clone()),
        ]);
        engine
            .queue_object_command(id, command)
            .expect("effect command queues");

        let snapshot = engine.tick().expect("queued effect executes");
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 1, "FxProbeStart runs exactly once");
        assert_eq!(calls[0].get(2), Some(&Value::Int(0)), "iTemp is false");
        assert_eq!(calls[0].get(3..7), Some(constructor_values.as_slice()));
        let effect = snapshot
            .object(id)
            .expect("object remains present")
            .effects
            .iter()
            .find(|effect| effect.name == "Probe")
            .expect("Probe effect survives");
        assert!(effect.start_dispatched);
        assert!(
            effect.vars().is_empty(),
            "constructor rVals never become persistent EffectVars"
        );
    }

    #[test]
    fn effect_annul_calls_temp_brackets_the_add_call() {
        // C4Fx_Effect_AnnulCalls (-3, C4Effects.h:38): like Annul, but the
        // acceptor's Fx*Add runs inside a temp remove/readd bracket of the
        // effects ABOVE the acceptor (C4Effect.cpp:297-304). Plain Annul
        // (-2) must not fire the bracket.
        let script = r#"#strict 3
        global func Initialize(state, random) {
            return { effects = [ { op = "add", name = "Shield", priority = 200, interval = 0 } ] };
        }

        global func FxShieldEffect(new_name, state, effect, unused) {
            if (new_name == "Fire") {
                return -3;
            }
            return nil;
        }

        global func FxShieldAdd(state, effect, new_name, new_interval) {
            return nil;
        }

        global func FxUpperStart(state, effect, temp) {
            return nil;
        }

        global func FxUpperStop(state, effect, reason, temp) {
            return nil;
        }

        global func Step(state, frame, random) {
            if (frame == 2) {
                return { effects = [ { op = "add", name = "Upper", priority = 300, interval = 0 } ] };
            }
            if (frame == 3) {
                AddEffect("Fire", this(), 100, 0);
            }
            return nil;
        }
        "#;

        let call_log: Arc<Mutex<Vec<(String, Vec<Value>)>>> = Arc::new(Mutex::new(Vec::new()));
        let mut hooks = DebuggerHooks::new();
        {
            let call_log = Arc::clone(&call_log);
            hooks.set_on_call(move |name, args| {
                call_log
                    .lock()
                    .unwrap()
                    .push((name.to_string(), args.to_vec()));
            });
        }

        let mut definition =
            Definition::from_script("Actor", "Actor", script).expect("script compiles");
        definition.set_debugger_hooks(hooks);

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(definition)
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("Actor"))
            .expect("spawn succeeds");

        engine.tick_without_snapshot().expect("tick succeeds");
        engine.tick_without_snapshot().expect("tick succeeds");
        call_log.lock().unwrap().clear();

        let third = engine.tick().expect("tick succeeds");
        let object = third.object(id).expect("object present");
        let names: Vec<&str> = object
            .effects
            .iter()
            .filter(|effect| effect.priority != 0)
            .map(|effect| effect.name.as_str())
            .collect();
        assert_eq!(
            names,
            vec!["Shield", "Upper"],
            "Fire merges into Shield; Upper is only temp-cycled"
        );
        assert!(object
            .effects
            .iter()
            .any(|effect| effect.name == "Fire" && effect.priority == 0));

        let calls: Vec<(String, Vec<Value>)> = call_log
            .lock()
            .unwrap()
            .iter()
            .filter(|(name, _)| name.starts_with("FxUpper") || name.starts_with("FxShieldAdd"))
            .cloned()
            .collect();
        let call_names: Vec<&str> = calls.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            call_names,
            vec!["FxUpperStop", "FxShieldAdd", "FxUpperStart"],
            "AnnulCalls brackets the Add with the acceptor's upper effects \
             (C4Effect.cpp:297-304)"
        );
        let (_, temp_stop_args) = &calls[0];
        assert_eq!(
            temp_stop_args.get(3),
            Some(&Value::Bool(true)),
            "the bracket stop is temporary (fTemp, C4Effect.cpp:489)"
        );
    }

    #[test]
    fn effect_add_returning_start_deny_kills_the_acceptor() {
        // C4Effect::Check (C4Effect.cpp:306-309): when the acceptor's
        // Fx*Add answers C4Fx_Start_Deny (-1), the ACCEPTOR itself is
        // killed (full Kill — its Stop callback runs) and the check
        // reports C4Fx_Effect_Annul. Neither effect survives.
        let script = r#"#strict 3
        global func Initialize(state, random) {
            return { effects = [ { op = "add", name = "Shield", priority = 200, interval = 0 } ] };
        }

        global func FxShieldEffect(new_name, state, effect, unused) {
            if (new_name == "Fire") {
                return -2;
            }
            return nil;
        }

        global func FxShieldAdd(state, effect, new_name, new_interval) {
            return CastBool(-1);
        }

        global func FxShieldStop(state, effect, reason) {
            return nil;
        }

        global func FxFireStart(state, effect) {
            return nil;
        }

        global func Step(state, frame, random) {
            if (frame == 2) {
                AddEffect("Fire", this(), 100, 7);
            }
            return nil;
        }
        "#;

        let call_log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let mut hooks = DebuggerHooks::new();
        {
            let call_log = Arc::clone(&call_log);
            hooks.set_on_call(move |name, _args| {
                call_log.lock().unwrap().push(name.to_string());
            });
        }

        let mut definition =
            Definition::from_script("Actor", "Actor", script).expect("script compiles");
        definition.set_debugger_hooks(hooks);

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(definition)
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("Actor"))
            .expect("spawn succeeds");

        engine.tick_without_snapshot().expect("tick succeeds");
        let second = engine.tick().expect("tick succeeds");
        let object = second.object(id).expect("object present");
        assert!(
            object.effects.iter().all(|effect| effect.priority == 0),
            "the Fire is annulled and Shield killed itself in its Add call; \
             both dead nodes remain linked until Execute (C4Effect.cpp:306-309,328-336)"
        );
        assert_eq!(
            object
                .effects
                .iter()
                .map(|effect| effect.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Fire", "Shield"]
        );

        let calls = call_log.lock().unwrap().clone();
        assert_eq!(
            calls
                .iter()
                .filter(|name| *name == "FxShieldStop")
                .count(),
            1,
            "the acceptor's Kill runs its Stop callback (C4Effect.cpp:390-392)"
        );
        assert!(
            !calls.iter().any(|name| name == "FxFireStart"),
            "the annulled effect still never starts"
        );
    }

    #[test]
    fn effect_stop_deny_recovers_the_effect() {
        // C4Fx_Stop_Deny (-1, C4Effects.h:42): an Fx*Stop answering it on
        // C4Effect::Kill refuses the removal — the effect's priority is
        // restored and it stays alive (C4Effect.cpp:389-396).
        let script = r#"#strict 3
        global func Initialize(state, random) {
            return { effects = [ { op = "add", name = "Sticky", priority = 100, interval = 0 } ] };
        }

        global func FxStickyStop(state, effect, reason) {
            return CastBool(-1);
        }

        global func Step(state, frame, random) {
            if (frame == 2) {
                return { effects = [ { op = "remove", name = "Sticky" } ] };
            }
            return nil;
        }
        "#;

        let call_log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let mut hooks = DebuggerHooks::new();
        {
            let call_log = Arc::clone(&call_log);
            hooks.set_on_call(move |name, _args| {
                call_log.lock().unwrap().push(name.to_string());
            });
        }

        let mut definition =
            Definition::from_script("Actor", "Actor", script).expect("script compiles");
        definition.set_debugger_hooks(hooks);

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(definition)
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("Actor"))
            .expect("spawn succeeds");

        engine.tick_without_snapshot().expect("tick succeeds");
        engine.tick_without_snapshot().expect("tick succeeds");
        let third = engine.tick().expect("tick succeeds");
        let object = third.object(id).expect("object present");
        let names: Vec<&str> = object
            .effects
            .iter()
            .map(|effect| effect.name.as_str())
            .collect();
        assert_eq!(
            names,
            vec!["Sticky"],
            "the denied removal recovers the effect (C4Effect.cpp:393-396)"
        );

        let calls = call_log.lock().unwrap().clone();
        assert_eq!(
            calls
                .iter()
                .filter(|name| *name == "FxStickyStop")
                .count(),
            1,
            "the Stop callback ran exactly once for the refused removal"
        );
    }

    #[test]
    fn effect_stop_deny_preserves_vars_and_equal_priority_position() {
        // C4Effect::Kill leaves the victim linked while Fx*Stop runs. If
        // Stop denies removal, Kill restores only iPriority on that same
        // node: EffectVar writes and its position among equal-priority
        // peers therefore both survive (C4Effect.cpp:389-402).
        let script = r#"#strict 3
        func Initialize(state, random) {
            return { effects = [
                { op = "add", name = "Older", priority = 100, interval = 0 },
                { op = "add", name = "Peer", priority = 100, interval = 0 }
            ] };
        }

        global func FxOlderStop(object target, int number, int reason) {
            EffectVar(0, target, number) = 77;
            return -1;
        }

        func Step(state, frame, random) {
            if (frame == 2) {
                return { effects = [ { op = "remove", name = "Older" } ] };
            }
            return nil;
        }
        "#;

        let mut definition =
            Definition::from_script("Actor", "Actor", script).expect("script compiles");
        definition.set_c4_callback_convention(true);
        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(definition)
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("Actor"))
            .expect("spawn succeeds");

        assert_eq!(
            engine
                .object_snapshot(id)
                .expect("object present")
                .effects
                .iter()
                .map(|effect| effect.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Peer", "Older"],
            "new equal-priority effects insert before older peers"
        );

        engine.tick_without_snapshot().expect("first tick succeeds");
        engine.tick_without_snapshot().expect("second tick succeeds");
        let third = engine.tick().expect("third tick succeeds");
        let object = third.object(id).expect("denied target remains present");
        assert_eq!(
            object
                .effects
                .iter()
                .map(|effect| effect.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Peer", "Older"],
            "denial restores the existing node without moving it before its equal-priority peer"
        );
        let older = object
            .effects
            .iter()
            .find(|effect| effect.name == "Older")
            .expect("denied effect remains present");
        assert_eq!(
            older.vars().first(),
            Some(&EffectVarValue::Int(77)),
            "EffectVar writes made by the denying Stop survive recovery"
        );
    }

    #[test]
    fn remove_effect_no_calls_skips_stop_callback() {
        let script = r#"
        func Initialize(state, random)
        {
            AddEffect("Pulse", this(), 100, 1);
            return 0;
        }

        global func FxPulseStart(object target, int effect)
        {
            return 0;
        }

        global func FxPulseTimer(object target, int effect, int timer)
        {
            if (GetEffect("Pulse", target))
            {
                RemoveEffect("Pulse", target, 0, true);
            }
            return 0;
        }

        global func FxPulseStop(object target, int effect, int reason)
        {
            return 0;
        }

        func Step(state, frame, random)
        {
            return 0;
        }
        "#;

        let call_log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let mut hooks = DebuggerHooks::new();
        {
            let call_log = Arc::clone(&call_log);
            hooks.set_on_call(move |name, _args| {
                call_log.lock().unwrap().push(name.to_string());
            });
        }

        let mut definition =
            Definition::from_script("Actor", "Actor", script).expect("script compiles");
        definition.set_debugger_hooks(hooks);
        definition.set_c4_callback_convention(true);

        let mut engine = Engine::with_seed(11);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("Actor"))
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert!(object
            .effects
            .iter()
            .any(|effect| effect.name == "Pulse" && effect.priority == 0));

        let snapshot = engine.tick().expect("next Execute succeeds");
        let object = snapshot.object(id).expect("object present");
        assert!(object.effects.is_empty());

        let calls = call_log.lock().unwrap().clone();
        let timer_calls = calls.iter().filter(|name| *name == "FxPulseTimer").count();
        let stop_calls = calls.iter().filter(|name| *name == "FxPulseStop").count();

        assert!(timer_calls >= 1);
        assert_eq!(stop_calls, 0);
    }

    // FnRemoveEffect's fDoNoCalls is a C++ bool parameter
    // (C4Script.cpp:5493): C4Value converts ints freely, and CR content
    // passes `1` — the GoldRush Talker's movie timer does
    // RemoveEffect("Movie", ..., 0, 1).
    #[test]
    fn remove_effect_accepts_int_no_calls_flag_like_cpp() {
        let script = r#"#strict
local iStopped;
public func Boot() { AddEffect("Pulse", this(), 1, 1, this()); return(1); }
func FxPulseTimer(pThis, iNumber) {
  RemoveEffect("Pulse", this(), 0, 1);
  return(1);
}
func FxPulseStop(pThis, iNumber, iReason) { iStopped = 1; return(1); }
"#;
        let mut engine = Engine::with_seed(11);
        engine
            .register_definition(
                Definition::from_script("Actr", "Actor", script).expect("script compiles"),
            )
            .expect("actor registers");
        let id = engine
            .spawn_object(SpawnConfig::new("Actr").with_category(CATEGORY_OBJECT))
            .expect("spawn succeeds");
        let idx = engine.find_object_index(id).expect("object exists");
        engine
            .call_object_function(idx, "Boot", Vec::new())
            .expect("boot runs");

        for _ in 0..2 {
            engine.tick_without_snapshot().expect("tick");
        }

        let idx = engine.find_object_index(id).expect("object exists");
        assert!(
            engine.objects[idx].state.effects.is_empty(),
            "the int flag converts like C++ and the effect is removed"
        );
        assert_ne!(
            engine.objects[idx].state.local_vars.get("iStopped"),
            Some(&Value::Int(1)),
            "a truthy no-calls flag skips FxPulseStop"
        );
    }

    #[test]
    fn change_effect_rebinds_the_timer_callback_to_the_new_name() {
        let script = r#"#strict
local iOldTimerCalls, iNewTimerCalls;
public func Boot() {
  AddEffect("IntFade", this(), 100, 50, this());
  return ChangeEffect("Int*", this(), 0, "IntFadeOut", 2);
}
func FxIntFadeTimer(pThis, iNumber, iTime) {
  ++iOldTimerCalls;
  return 0;
}
func FxIntFadeOutTimer(pThis, iNumber, iTime) {
  ++iNewTimerCalls;
  return 0;
}
"#;
        let mut engine = Engine::with_seed(11);
        engine
            .register_definition(
                Definition::from_script("Actr", "Actor", script).expect("script compiles"),
            )
            .expect("actor registers");
        let id = engine
            .spawn_object(SpawnConfig::new("Actr").with_category(CATEGORY_OBJECT))
            .expect("actor spawns");
        let idx = engine.find_object_index(id).expect("actor exists");
        assert_eq!(
            engine
                .call_object_function(idx, "Boot", Vec::new())
                .expect("ChangeEffect runs"),
            Value::Bool(true)
        );

        let effect = &engine.objects[idx].state.effects[0];
        assert_eq!(effect.name, "IntFadeOut");
        assert_eq!(effect.interval, 2);
        assert_eq!(effect.timer, 0);

        engine.tick_without_snapshot().expect("first timer tick");
        engine.tick_without_snapshot().expect("second timer tick");
        let idx = engine.find_object_index(id).expect("actor remains");
        assert_eq!(
            engine.objects[idx]
                .state
                .local_vars
                .get("iOldTimerCalls")
                .cloned()
                .unwrap_or(Value::Nil),
            Value::Nil,
            "the old callback is no longer bound"
        );
        assert_eq!(
            engine.objects[idx].state.local_vars.get("iNewTimerCalls"),
            Some(&Value::Int(1)),
            "the renamed callback fires at the new interval"
        );
    }

    #[test]
    fn queued_commands_can_spawn_and_destroy() {
        let mut engine = Engine::with_seed(42);
        let definition = Definition::from_script(
            "Dummy",
            "Dummy",
            "global func Step(state, frame, random) { return 0; }",
        )
        .expect("script compiles");
        engine
            .register_definition(definition)
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("Dummy"))
            .expect("spawn succeeds");

        let command = QueuedCommand::immediate(ObjectUpdate::default())
            .with_delay(1)
            .with_destroy(true)
            .with_spawns(vec![
                SpawnConfig::new("Dummy").with_position(Vector2::new(5, 5))
            ]);
        engine
            .queue_object_command(id, command)
            .expect("queue succeeds");

        let snapshot = engine.tick().expect("first tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(snapshot.objects.len(), 1);
        assert_eq!(object.id, id);

        let snapshot = engine.tick().expect("second tick succeeds");
        assert!(snapshot.object(id).is_none());
        assert_eq!(snapshot.objects.len(), 1);
        let new_object = &snapshot.objects[0];
        assert_ne!(new_object.id, id);
        assert_eq!(new_object.definition_id, "Dummy");
        assert_eq!(new_object.position, Vector2::new(5, 5));
    }

    #[test]
    fn recorder_playback_roundtrip_matches_engine() {
        let mut engine_a = Engine::with_seed(99);
        engine_a
            .register_definition(build_definition())
            .expect("definition registers");
        engine_a.set_landscape(Landscape::flat(32, 0));

        let spawn = SpawnConfig::new("Test")
            .with_position(Vector2::new(0, 0))
            .with_velocity(Vector2::new(1, 0));
        engine_a
            .spawn_object(spawn.clone())
            .expect("spawn succeeds");

        let mut recorder = Recorder::new();
        for _ in 0..5 {
            let snapshot = engine_a.tick().expect("tick succeeds");
            recorder.record(&snapshot);
        }
        let recording = recorder.into_recording();
        let serialized = recording.to_string().expect("serializes");

        let mut playback = Playback::from_str(&serialized).expect("playback loads");

        let mut engine_b = Engine::with_seed(99);
        engine_b
            .register_definition(build_definition())
            .expect("definition registers");
        engine_b.set_landscape(Landscape::flat(32, 0));
        engine_b.spawn_object(spawn).expect("spawn succeeds");

        for _ in 0..5 {
            let snapshot = engine_b.tick().expect("tick succeeds");
            playback
                .validate_snapshot(&snapshot)
                .expect("snapshots match");
        }
        playback.finish().expect("playback completed");
    }

    #[test]
    fn apply_object_update_overrides_velocity() {
        let mut engine = Engine::with_seed(1);
        engine
            .register_definition(build_definition())
            .expect("definition registers");
        let id = engine
            .spawn_object(
                SpawnConfig::new("Test")
                    .with_position(Vector2::new(0, 0))
                    .with_velocity(Vector2::new(0, 0)),
            )
            .expect("spawn succeeds");

        engine
            .apply_object_update(
                id,
                ObjectUpdate::new()
                    .with_velocity(Vector2::new(5, -3))
                    .with_owner(7),
            )
            .expect("update applies");

        let snapshot = engine.object_snapshot(id).expect("object snapshot");
        assert_eq!(snapshot.velocity, Vector2::new(5, -3));
        assert_eq!(snapshot.owner, 7);
    }

    #[test]
    fn object_update_layer_serde_preserves_clear_vs_unchanged() {
        // Queued/recorded updates must distinguish a missing layer field from
        // an explicit null that clears pLayer.
        let unchanged: ObjectUpdate = serde_json::from_str("{}").expect("unchanged decodes");
        assert_eq!(unchanged.layer, None);

        let clear_json = serde_json::to_string(&ObjectUpdate::new().clear_layer())
            .expect("layer clear encodes");
        let clear: ObjectUpdate = serde_json::from_str(&clear_json).expect("layer clear decodes");
        assert_eq!(clear.layer, Some(None));

        let layer = ObjectId::new(17);
        let set_json =
            serde_json::to_string(&ObjectUpdate::new().with_layer(layer)).expect("layer set encodes");
        let set: ObjectUpdate = serde_json::from_str(&set_json).expect("layer set decodes");
        assert_eq!(set.layer, Some(Some(layer)));
    }

    #[test]
    fn object_update_blit_mode_round_trips() {
        // C4Object::BlitMode is independent from SetGraphics' base/overlay
        // modes and must survive queued-update serialization.
        let encoded = serde_json::to_string(&ObjectUpdate::new().with_blit_mode(129))
            .expect("blit mode update encodes");
        let decoded: ObjectUpdate =
            serde_json::from_str(&encoded).expect("blit mode update decodes");
        assert_eq!(decoded.blit_mode, Some(129));
    }

    #[test]
    fn object_blit_mode_survives_spawn_update_and_state_restore() {
        let mut definition = build_definition();
        definition.set_blit_mode(2);
        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(definition.clone())
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("Test"))
            .expect("object spawns");
        assert_eq!(
            engine.object_snapshot(id).expect("object exists").blit_mode,
            2,
            "fresh objects inherit the definition mode"
        );

        engine
            .apply_object_update(id, ObjectUpdate::new().with_blit_mode(129))
            .expect("blit mode update applies");
        let encoded = engine
            .capture_state()
            .to_json_string()
            .expect("engine state encodes");
        let state = EngineState::from_json_str(&encoded).expect("engine state decodes");
        let mut restored = Engine::with_seed(0);
        restored
            .register_definition(definition)
            .expect("definition registers for restore");
        restored.restore_state(&state).expect("state restores");
        assert_eq!(
            restored
                .object_snapshot(id)
                .expect("restored object exists")
                .blit_mode,
            129
        );
    }

    #[test]
    fn apply_object_update_unknown_action_falls_back_to_default() {
        let mut definition = build_definition();
        let mut actions = HashMap::new();
        actions.insert("Idle".to_string(), ActionSpec::default());
        actions.insert("Run".to_string(), ActionSpec::default());
        definition.configure_actions(Some("Idle".to_string()), actions);

        let mut engine = Engine::with_seed(3);
        engine
            .register_definition(definition)
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("Test"))
            .expect("spawn succeeds");

        engine
            .apply_object_update(
                id,
                ObjectUpdate::new()
                    .with_action("Run")
                    .with_action_phase(2)
                    .with_action_ticks(5),
            )
            .expect("valid update applies");

        let snapshot = engine.object_snapshot(id).expect("snapshot");
        assert_eq!(snapshot.action.name, "Run");
        assert_eq!(snapshot.action.phase, 2);
        assert_eq!(snapshot.action.ticks, 5);

        engine
            .apply_object_update(
                id,
                ObjectUpdate::new()
                    .with_action("Ghost")
                    .with_action_phase(1)
                    .with_action_ticks(3),
            )
            .expect("invalid action handled");

        let snapshot = engine.object_snapshot(id).expect("snapshot");
        assert_eq!(snapshot.action.name, "Idle");
        assert_eq!(snapshot.action.phase, 0);
        assert_eq!(snapshot.action.ticks, 0);
    }

    #[test]
    fn spawn_config_unknown_action_falls_back_to_default() {
        let mut definition = build_definition();
        let mut actions = HashMap::new();
        actions.insert("Idle".to_string(), ActionSpec::default());
        definition.configure_actions(Some("Idle".to_string()), actions);

        let mut engine = Engine::with_seed(4);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let mut requested = ActionState::new("Ghost");
        requested.phase = 3;
        requested.ticks = 7;

        let id = engine
            .spawn_object(SpawnConfig::new("Test").with_action(requested))
            .expect("spawn succeeds");

        let snapshot = engine.object_snapshot(id).expect("snapshot");
        assert_eq!(snapshot.action.name, "Idle");
        assert_eq!(snapshot.action.phase, 0);
        assert_eq!(snapshot.action.ticks, 0);
    }

    #[test]
    fn initialize_with_unknown_action_falls_back_to_default() {
        let source = r#"#strict 3
        global func Initialize(state, random) {
            return { action = "Ghost" };
        }

        global func Step(state, frame, random) {
            return nil;
        }
        "#;

        let mut definition =
            Definition::from_script("Actor", "Actor", source).expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert("Walk".to_string(), ActionSpec::default());
        actions.insert("Idle".to_string(), ActionSpec::default());
        definition.configure_actions(Some("Walk".to_string()), actions);

        let mut engine = Engine::with_seed(5);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("Actor"))
            .expect("spawn succeeds");

        let snapshot = engine.object_snapshot(id).expect("snapshot");
        assert_eq!(snapshot.action.name, "Walk");
        assert_eq!(snapshot.action.phase, 0);
        assert_eq!(snapshot.action.ticks, 0);
    }

    #[test]
    fn apply_object_update_unknown_object_errors() {
        let mut engine = Engine::with_seed(1);
        let error = engine
            .apply_object_update(ObjectId::new(999), ObjectUpdate::default())
            .expect_err("update fails");
        match error {
            EngineError::UnknownObject(id) => assert_eq!(id.as_u64(), 999),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn custom_physics_settings_affect_integration() {
        let mut engine = Engine::with_seed(42);
        engine
            .register_definition(simple_definition("Test"))
            .expect("definition registers");
        engine.set_physics(PhysicsSettings::new(2, 6, -8));

        let id = engine
            .spawn_object(
                SpawnConfig::new("Test")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(0, 0))
                    .with_velocity(Vector2::new(0, 0)),
            )
            .expect("spawn succeeds");
        // Arm Mobile like a prior SetXDir(0) would (C4Script.cpp:705): the
        // golden pins the mobile-object integration math, not the Tick10
        // mobilization pulse.
        let idx = engine.find_object_index(id).expect("object exists");
        engine.objects[idx].state.mobile = true;

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.velocity.y, 0);
        assert_eq!(object.position.y, 0);
        assert_eq!(
            object
                .fixed_velocity
                .expect("custom gravity should remain sub-pixel")
                .y
                .val(),
            262
        );
        assert_eq!(
            object
                .fixed_position
                .expect("custom gravity movement should remain sub-pixel")
                .y
                .val(),
            262
        );
    }

    #[test]
    fn movement_out_of_bounds_removal_uses_strict_boundaries_and_cpp_exemptions() {
        // C4Object::ExecMovement tests `!Inside(x, 0, GBackWdt)` and
        // `y > GBackHgt` after movement/stabilization, then calls
        // AssignDeath(true) + AssignRemoval (src/C4Movement.cpp:598-617).
        // The inclusive x/y boundaries themselves still survive.
        let mut definition = simple_definition("FALL");
        definition.set_category(CATEGORY_LIVING);
        definition.set_shape_vertices(vec![ObjectVertex::new(0, 0).with_cnat(CNAT_BOTTOM)]);
        let mut bounded = simple_definition("BNDD");
        bounded.set_category(CATEGORY_LIVING);
        bounded.set_border_bound(C4D_BORDER_BOTTOM);
        let mut side_bounded = simple_definition("SNDB");
        side_bounded.set_category(CATEGORY_LIVING);
        side_bounded.set_border_bound(C4D_BORDER_SIDES);
        let mut static_back = simple_definition("STAT");
        static_back.set_category(CATEGORY_STATIC_BACK);
        let mut parallax = simple_definition("PARA");
        parallax.set_category(CATEGORY_OBJECT | CATEGORY_PARALLAX);
        let mut attached = simple_definition("ATCH");
        attached.set_category(CATEGORY_LIVING);
        attached.configure_actions(
            Some("Attach".to_string()),
            HashMap::from([(
                "Attach".to_string(),
                ActionSpec::default().with_procedure("Attach"),
            )]),
        );
        let mut engine = Engine::with_seed(42);
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine
            .register_definition(bounded)
            .expect("bounded definition registers");
        engine
            .register_definition(side_bounded)
            .expect("side-bounded definition registers");
        engine
            .register_definition(static_back)
            .expect("static definition registers");
        engine
            .register_definition(parallax)
            .expect("parallax definition registers");
        engine
            .register_definition(attached)
            .expect("attach definition registers");
        engine.set_physics(PhysicsSettings::new(0, 20, -20));
        let mut landscape = Landscape::flat(16, 20);
        landscape.set_world_height(20);
        landscape.set_border_open(0, 0, true, true);
        engine.set_landscape(landscape);

        let boundary = engine
            .spawn_object(
                SpawnConfig::new("FALL")
                    .with_category(CATEGORY_LIVING)
                    .with_position(Vector2::new(8, 20))
                    .with_mobile(true),
            )
            .expect("boundary object spawns");
        let below = engine
            .spawn_object(
                SpawnConfig::new("FALL")
                    .with_category(CATEGORY_LIVING)
                    .with_position(Vector2::new(8, 21))
                    .with_mobile(true),
            )
            .expect("below-bottom object spawns");
        let left_boundary = engine
            .spawn_object(
                SpawnConfig::new("FALL")
                    .with_category(CATEGORY_LIVING)
                    .with_position(Vector2::new(0, 8)),
            )
            .expect("left-boundary object spawns");
        let right_boundary = engine
            .spawn_object(
                SpawnConfig::new("FALL")
                    .with_category(CATEGORY_LIVING)
                    .with_position(Vector2::new(16, 8)),
            )
            .expect("right-boundary object spawns");
        let left_out = engine
            .spawn_object(
                SpawnConfig::new("FALL")
                    .with_category(CATEGORY_LIVING)
                    .with_position(Vector2::new(-1, 8)),
            )
            .expect("left-out object spawns");
        let right_out = engine
            .spawn_object(
                SpawnConfig::new("FALL")
                    .with_category(CATEGORY_LIVING)
                    .with_position(Vector2::new(17, 8)),
            )
            .expect("right-out object spawns");
        let crossing = engine
            .spawn_object(
                SpawnConfig::new("FALL")
                    .with_category(CATEGORY_LIVING)
                    .with_position(Vector2::new(10, 20))
                    .with_velocity(Vector2::new(0, 1))
                    .with_mobile(true),
            )
            .expect("crossing object spawns");
        let bounded = engine
            .spawn_object(
                SpawnConfig::new("BNDD")
                    .with_category(CATEGORY_LIVING)
                    .with_position(Vector2::new(6, 21))
                    .with_mobile(true),
            )
            .expect("border-bound object spawns");
        let side_bounded = engine
            .spawn_object(
                SpawnConfig::new("SNDB")
                    .with_category(CATEGORY_LIVING)
                    .with_position(Vector2::new(-1, 8)),
            )
            .expect("side-border-bound object spawns");
        let contained = engine
            .spawn_object(
                SpawnConfig::new("FALL")
                    .with_category(CATEGORY_LIVING)
                    .with_container(bounded),
            )
            .expect("contained object spawns");
        let static_back = engine
            .spawn_object(
                SpawnConfig::new("STAT")
                    .with_category(CATEGORY_STATIC_BACK)
                    .with_position(Vector2::new(4, 21))
                    .with_mobile(true),
            )
            .expect("StaticBack object spawns");
        let mut attach_action = ActionState::new("Attach");
        attach_action.target = Some(bounded);
        let attached = engine
            .spawn_object(
                SpawnConfig::new("ATCH")
                    .with_category(CATEGORY_LIVING)
                    .with_position(Vector2::new(6, 21))
                    .with_action(attach_action)
                    .with_mobile(true),
            )
            .expect("attached object spawns");
        let mut side_attach_action = ActionState::new("Attach");
        side_attach_action.target = Some(bounded);
        let side_attached = engine
            .spawn_object(
                SpawnConfig::new("ATCH")
                    .with_category(CATEGORY_LIVING)
                    .with_position(Vector2::new(-1, 8))
                    .with_action(side_attach_action)
                    .with_mobile(true),
            )
            .expect("side-attached object spawns");
        let hud_left = engine
            .spawn_object(
                SpawnConfig::new("PARA")
                    .with_category(CATEGORY_OBJECT | CATEGORY_PARALLAX)
                    .with_position(Vector2::new(-1, 8)),
            )
            .expect("horizontal HUD object spawns");
        let hud_far_left = engine
            .spawn_object(
                SpawnConfig::new("PARA")
                    .with_category(CATEGORY_OBJECT | CATEGORY_PARALLAX)
                    .with_position(Vector2::new(-17, 8)),
            )
            .expect("far-left HUD object spawns");
        let world_parallax_left = engine
            .spawn_object(
                SpawnConfig::new("PARA")
                    .with_category(CATEGORY_OBJECT | CATEGORY_PARALLAX)
                    .with_position(Vector2::new(-1, 8))
                    .with_local_vars(HashMap::from([(
                        "__local_0".to_string(),
                        Value::String(String::new().into()),
                    )])),
            )
            .expect("world-parallax object spawns");
        let hud_right = engine
            .spawn_object(
                SpawnConfig::new("PARA")
                    .with_category(CATEGORY_OBJECT | CATEGORY_PARALLAX)
                    .with_position(Vector2::new(17, 8)),
            )
            .expect("right-out HUD object spawns");
        let hud_bottom = engine
            .spawn_object(
                SpawnConfig::new("PARA")
                    .with_category(CATEGORY_OBJECT | CATEGORY_PARALLAX)
                    .with_position(Vector2::new(8, 21)),
            )
            .expect("below-bottom HUD object spawns");

        engine.tick_without_snapshot().expect("movement tick succeeds");

        let boundary = engine.object_snapshot(boundary).expect("boundary remains");
        assert!(boundary.alive);
        assert_eq!(boundary.status, ObjectStatus::Normal);
        assert!(
            engine.object_snapshot(below).is_none(),
            "AssignDeath and AssignRemoval complete in the out-of-bounds tick"
        );
        assert!(
            engine.object_snapshot(crossing).is_none(),
            "crossing from y == GBackHgt is removed in that movement tick"
        );
        assert!(engine.object_snapshot(left_boundary).is_some());
        assert!(engine.object_snapshot(right_boundary).is_some());
        assert!(
            engine.object_snapshot(left_out).is_none(),
            "ordinary x < 0 is removed"
        );
        assert!(
            engine.object_snapshot(right_out).is_none(),
            "ordinary x > GBackWdt is removed"
        );
        assert_eq!(
            engine
                .object_snapshot(bounded)
                .expect("Border_Bottom object remains")
                .status,
            ObjectStatus::Normal
        );
        assert_eq!(
            engine
                .object_snapshot(contained)
                .expect("contained object remains")
                .container,
            Some(bounded)
        );
        assert_eq!(
            engine
                .object_snapshot(side_bounded)
                .expect("Border_Sides object remains")
                .status,
            ObjectStatus::Normal
        );
        assert_eq!(
            engine
                .object_snapshot(static_back)
                .expect("StaticBack object remains")
                .status,
            ObjectStatus::Normal
        );
        let attached = engine
            .object_snapshot(attached)
            .expect("DFA_ATTACH object with a target remains");
        assert_eq!(attached.action.name, "Attach");
        assert_eq!(attached.action.target, Some(bounded));
        let side_attached = engine
            .object_snapshot(side_attached)
            .expect("DFA_ATTACH object also survives an out-of-bounds side");
        assert_eq!(side_attached.action.target, Some(bounded));
        assert!(
            engine.object_snapshot(hud_left).is_some(),
            "Local[0] == 0 HUD parallax survives the near left side"
        );
        assert!(
            engine.object_snapshot(hud_far_left).is_none(),
            "HUD parallax is removed beyond one landscape width left"
        );
        assert!(
            engine.object_snapshot(world_parallax_left).is_none(),
            "raw-nonzero Local[0] parallax is removed immediately at x < 0"
        );
        assert!(
            engine.object_snapshot(hud_right).is_none(),
            "all parallax objects are removed at x > GBackWdt"
        );
        assert!(
            engine.object_snapshot(hud_bottom).is_none(),
            "all parallax objects are removed at y > GBackHgt"
        );
    }

    #[test]
    fn out_of_bounds_callbacks_run_before_timer_and_step_despite_death_error() {
        // ExecMovement performs AssignDeath(true) then AssignRemoval before
        // C4Object::Execute can reach effects/life/timer
        // (src/C4Movement.cpp:613-617; src/C4Object.cpp:1069-1091).
        // Engine callbacks are fail-safe, so a Death error must still permit
        // Destruction and removal.
        let script = r#"
            static iOrder, iTimer, iStep, iEffectOrder, iDeathEffectOrder;
            func Death() {
                iOrder = 1;
                iDeathEffectOrder = iEffectOrder;
                iEffectOrder = 0;
                MissingDeathFunction();
            }
            func Destruction() { iOrder = iOrder * 10 + 2; }
            func FxLowStop(target, number, reason, temp) {
                iEffectOrder = iEffectOrder * 10 + 1;
            }
            func FxHighStop(target, number, reason, temp) {
                iEffectOrder = iEffectOrder * 10 + 2;
            }
            func After() { iTimer = 1; }
            func Step() { iStep = 1; }
        "#;
        let mut definition =
            Definition::from_script("HOOK", "Hook", script).expect("script compiles");
        definition.set_c4_callback_convention(true);
        definition.set_category(CATEGORY_LIVING);
        definition.set_shape_vertices(vec![ObjectVertex::new(0, 0).with_cnat(CNAT_BOTTOM)]);
        definition.set_timer(1);
        definition.set_timer_call(Some("After".to_string()));
        definition.configure_actions(
            Some("Idle".to_string()),
            HashMap::from([
                ("Idle".to_string(), ActionSpec::default()),
                ("Dead".to_string(), ActionSpec::default()),
            ]),
        );

        let mut engine = Engine::with_seed(43);
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine.set_physics(PhysicsSettings::new(0, 20, -20));
        let mut landscape = Landscape::flat(16, 20);
        landscape.set_world_height(20);
        landscape.set_border_open(0, 0, true, true);
        engine.set_landscape(landscape);
        let object = engine
            .spawn_object(
                SpawnConfig::new("HOOK")
                    .with_category(CATEGORY_LIVING)
                    .with_position(Vector2::new(8, 20))
                    .with_velocity(Vector2::new(0, 1))
                    .add_effect(EffectState::new("Low").with_priority(10))
                    .add_effect(EffectState::new("High").with_priority(100))
                    .with_mobile(true),
            )
            .expect("object spawns");
        let object_idx = engine.find_object_index(object).expect("object exists");
        let command_target = i32::try_from(object.as_u64()).expect("test object id fits C4 int");
        for effect in &mut engine.objects[object_idx].state.effects {
            effect.command_target = Some(command_target);
        }

        engine.tick_without_snapshot().expect("Death error is fail-safe");

        assert!(engine.object_snapshot(object).is_none());
        let globals = engine.snapshot().script_globals.named;
        assert_eq!(globals.get("iOrder"), Some(&Value::Int(12)));
        assert_eq!(
            globals.get("iDeathEffectOrder"),
            Some(&Value::Int(21)),
            "AssignDeath clears effects from high to low before Death"
        );
        assert_eq!(
            globals.get("iEffectOrder"),
            Some(&Value::Nil),
            "AssignRemoval skips the already-dead effect nodes after Death"
        );
        assert_eq!(globals.get("iTimer"), Some(&Value::Nil));
        assert_eq!(globals.get("iStep"), Some(&Value::Nil));
    }

    #[test]
    fn out_of_bounds_assign_removal_passes_clear_reason_to_effect_stop() {
        // A non-living object skips AssignDeath and reaches
        // AssignRemoval's ClearAll directly. C++ supplies
        // C4FxCall_RemoveClear (3) while the target is still live, before
        // setting Status=0 (C4Movement.cpp:613-614; C4Object.cpp:257-269).
        let script = r#"#strict 3
            static stop_reason;
            func FxWitnessStop(target, number, int reason) {
                stop_reason = reason;
                return 0;
            }
        "#;
        let mut definition =
            Definition::from_script("CLER", "Clear reason target", script)
                .expect("script compiles");
        definition.set_c4_callback_convention(true);
        definition.set_category(CATEGORY_OBJECT);

        let mut engine = Engine::with_seed(43);
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine.set_physics(PhysicsSettings::new(0, 0, 0));
        let mut landscape = Landscape::flat(16, 20);
        landscape.set_world_height(20);
        landscape.set_border_open(0, 0, true, true);
        engine.set_landscape(landscape);
        let object = engine
            .spawn_object(
                SpawnConfig::new("CLER")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(-1, 8))
                    .with_mobile(true)
                    .add_effect(EffectState::new("Witness").with_priority(100)),
            )
            .expect("object spawns");
        let object_idx = engine.find_object_index(object).expect("object exists");
        let command_target = i32::try_from(object.as_u64()).expect("test object id fits C4 int");
        engine.objects[object_idx].state.effects[0].command_target = Some(command_target);

        engine.tick_without_snapshot().expect("out-of-bounds removal succeeds");

        assert!(engine.object_snapshot(object).is_none());
        assert_eq!(
            engine.snapshot().script_globals.named.get("stop_reason"),
            Some(&Value::Int(3)),
            "AssignRemoval's Fx*Stop receives C4FxCall_RemoveClear"
        );
    }

    #[test]
    fn fixed_point_velocity_accumulates_sub_pixel_motion() {
        let mut engine = Engine::with_seed(42);
        engine
            .register_definition(simple_definition("Test"))
            .expect("definition registers");
        engine.set_physics(PhysicsSettings::new(0, 0, 0));

        let id = engine
            .spawn_object(
                SpawnConfig::new("Test")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(0, 0)),
            )
            .expect("spawn succeeds");
        let idx = engine.find_object_index(id).expect("object exists");
        engine.objects[idx]
            .set_fixed_velocity(FixedVec2::new(C4Fixed::from_raw(300), C4Fixed::ZERO));
        // dir writes mobilize (FnSetXDir/FnSetYDir, C4Script.cpp:705,732)
        engine.objects[idx].state.mobile = true;

        for _ in 0..109 {
            let snapshot = engine.tick().expect("tick succeeds");
            let object = snapshot.object(id).expect("object present");
            assert_eq!(object.position.x, 0);
        }

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        // `fixtoi` rounds to nearest: 110 * 300 = 33000, just over 0.5px.
        assert_eq!(object.position.x, 1);
        assert_eq!(object.velocity.x, 0);
    }

    #[test]
    fn gravity_accumulates_as_c4fixed_matching_cpp_golden() {
        // Mirrors parity/golden/parity_golden.json movement[0]: C4Movement.cpp:643
        // applies ydir += GravAccel with raw GravAccel 13107 each frame.
        let mut engine = Engine::with_seed(42);
        engine
            .register_definition(simple_definition("Test"))
            .expect("definition registers");
        engine.set_physics(PhysicsSettings::new(100, 200, -200));
        engine.set_environment(EnvironmentSettings::new(0));

        let id = engine
            .spawn_object(
                SpawnConfig::new("Test")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(0, 0)),
            )
            .expect("spawn succeeds");
        // Arm Mobile like a prior SetXDir(0) would (C4Script.cpp:705): the
        // golden pins the mobile-object gravity math, not the Tick10 pulse.
        let idx = engine.find_object_index(id).expect("object exists");
        engine.objects[idx].state.mobile = true;
        let expected_ydir = [13107, 26214, 39321, 52428, 65535];

        for raw_ydir in expected_ydir {
            engine.tick_without_snapshot().expect("tick succeeds");
            let idx = engine.find_object_index(id).expect("object exists");
            assert_eq!(engine.objects[idx].fixed_velocity.y.val(), raw_ydir);
        }
    }

    #[test]
    fn spawn_landscape_friction_applies_to_fixed_velocity() -> Result<(), EngineError> {
        let library = MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=100
            Friction=50
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");
        let mut engine = Engine::with_seed(17);
        engine.set_materials(materials);
        engine.set_landscape(Landscape::flat_with_material(20, 10, Some(earth)));
        engine.set_physics(
            PhysicsSettings::new(0, 20, -20)
                .with_max_horizontal_speed(20)
                .expect("horizontal speed valid"),
        );
        let definition = Definition::from_script(
            "Slider",
            "Slider",
            r#"
            global func Initialize(state, random) { SetXDir(15); return 0; }
            global func Step(state, frame, random) { return 0; }
            "#,
        )
        .expect("script compiles");
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id =
            engine.spawn_object(SpawnConfig::new("Slider").with_position(Vector2::new(5, 12)))?;
        let idx = engine.find_object_index(id).expect("object exists");

        // C4Game::NewObject performs NO landscape resolution: the object
        // spawns exactly where Init+DoCon put it — even inside solid —
        // and keeps the script-set velocity; contacts resolve in movement
        // (C4Game.cpp:1085-1127). The old spawn-time snap+friction was a
        // port-ism (it displaced the GoldRush wagon by 20px).
        assert_eq!(engine.objects[idx].state.position.y, 12);
        // SetXDir's default precision is 10 (FnSetXDir, C4Script.cpp:705):
        // 15 -> 1.5 px/frame.
        assert_eq!(
            engine.objects[idx].fixed_velocity.x.val(),
            98_304,
            "SetXDir(15) survives the spawn untouched"
        );
        assert_eq!(engine.objects[idx].state.velocity.x, 2);
        Ok(())
    }

    #[test]
    fn landscape_collision_preserves_fixed_x_and_zeroes_contact_y() {
        let mut definition = simple_definition("Crate");
        definition.set_shape_vertices(vec![ObjectVertex::new(0, 0)]);
        let mut engine = Engine::with_seed(19);
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine.set_landscape(Landscape::flat(20, 10));
        engine.set_physics(PhysicsSettings::new(0, 20, -20));

        let id = engine
            .spawn_object(SpawnConfig::new("Crate").with_position(Vector2::new(5, 0)))
            .expect("spawn succeeds");
        let idx = engine.find_object_index(id).expect("object exists");
        engine.objects[idx].set_position(Vector2::new(5, 12));
        engine.objects[idx].set_fixed_velocity(FixedVec2::new(
            C4Fixed::from_raw(300),
            C4Fixed::from_raw(70000),
        ));
        // dir writes mobilize (FnSetXDir/FnSetYDir, C4Script.cpp:705,732)
        engine.objects[idx].state.mobile = true;

        engine.apply_landscape_at_index(idx);

        assert_eq!(engine.objects[idx].state.position.y, 10);
        assert_eq!(engine.objects[idx].fixed_velocity.x.val(), 300);
        assert_eq!(engine.objects[idx].fixed_velocity.y, C4Fixed::ZERO);
        assert_eq!(engine.objects[idx].state.velocity, Vector2::ZERO);
    }

    #[test]
    fn per_pixel_horizontal_movement_stops_at_first_solid_column() {
        let library = MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=100
            "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");

        let mut definition = simple_definition("Crate");
        definition.set_shape_vertices(vec![ObjectVertex::new(0, 0).with_cnat(CNAT_RIGHT)]);
        definition.set_contact_density(50);
        let mut engine = Engine::with_seed(23);
        engine.set_materials(materials);
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine.set_physics(PhysicsSettings::new(0, 20, -20));
        let mut surface = vec![20; 12];
        surface[6] = 0;
        let mut landscape =
            Landscape::new_with_material(12, surface, Some(earth)).expect("landscape constructs");
        landscape.fill_solid_material(Some(earth));
        engine.set_landscape(landscape);

        let id = engine
            .spawn_object(
                SpawnConfig::new("Crate")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(4, 10)),
            )
            .expect("spawn succeeds");
        let idx = engine.find_object_index(id).expect("object exists");
        engine.objects[idx].set_fixed_velocity(FixedVec2::new(itofix(4), C4Fixed::ZERO));
        // dir writes mobilize (FnSetXDir/FnSetYDir, C4Script.cpp:705,732)
        engine.objects[idx].state.mobile = true;

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");

        assert_eq!(object.position, Vector2::new(5, 10));
        let idx = engine.find_object_index(id).expect("object exists");
        assert_eq!(engine.objects[idx].fixed_position.x, itofix(5));
        assert_eq!(
            engine.objects[idx].fixed_velocity.x,
            itofix(4) - fixed100(50)
        );
        assert_eq!(engine.objects[idx].fixed_velocity.y, -fixed100(50));
    }

    #[test]
    fn zero_vertex_objects_use_vertex_contact_and_border_bound_semantics() {
        use std::sync::{Arc, Mutex};

        fn zero_vertex_definition(
            id: &str,
            calls: Arc<Mutex<Vec<String>>>,
        ) -> Definition {
            let mut hooks = DebuggerHooks::new();
            hooks.set_on_call(move |name, _args| {
                if matches!(
                    name,
                    "ContactLeft"
                        | "ContactRight"
                        | "ContactTop"
                        | "ContactBottom"
                        | "Hit"
                        | "Hit2"
                        | "Hit3"
                ) {
                    calls.lock().unwrap().push(name.to_string());
                }
            });
            let mut definition = Definition::from_script(
                id,
                id,
                r#"
                global func ContactLeft() { return 0; }
                global func ContactRight() { return 0; }
                global func ContactTop() { return 0; }
                global func ContactBottom() { return 0; }
                global func Hit() { return 0; }
                global func Hit2() { return 0; }
                global func Hit3() { return 0; }
                "#,
            )
            .expect("zero-vertex movement script compiles");
            definition.set_debugger_hooks(hooks);
            definition.set_shape_rect(Some(DefinitionRect::new(-2, -3, 4, 6)));
            definition.set_shape_vertices(Vec::new());
            definition.set_contact_density(50);
            definition.set_contact_function_calls(true);
            definition
        }

        let library = MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=100
            Friction=100
            "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");

        let terrain_calls = Arc::new(Mutex::new(Vec::new()));
        let world_bound_calls = Arc::new(Mutex::new(Vec::new()));
        let layer_high_bound_calls = Arc::new(Mutex::new(Vec::new()));
        let layer_low_bound_calls = Arc::new(Mutex::new(Vec::new()));
        let terrain_definition =
            zero_vertex_definition("ZeroPass", Arc::clone(&terrain_calls));
        let mut world_bound_definition =
            zero_vertex_definition("ZeroWorld", Arc::clone(&world_bound_calls));
        world_bound_definition
            .set_border_bound(C4D_BORDER_SIDES | C4D_BORDER_TOP | C4D_BORDER_BOTTOM);
        let mut layer_definition = simple_definition("ZeroLayer");
        layer_definition.set_shape_rect(Some(DefinitionRect::new(-1, -2, 10, 12)));
        layer_definition.set_shape_vertices(Vec::new());
        layer_definition.set_border_bound(C4D_BORDER_LAYER);
        let layer_high_mover_definition =
            zero_vertex_definition("ZeroLayerHigh", Arc::clone(&layer_high_bound_calls));
        let layer_low_mover_definition =
            zero_vertex_definition("ZeroLayerLow", Arc::clone(&layer_low_bound_calls));

        let mut engine = Engine::with_seed(23);
        engine.set_materials(materials);
        let mut landscape = Landscape::flat_with_material(20, 10, Some(earth));
        landscape.set_world_height(30);
        engine.set_landscape(landscape);
        engine.set_physics(
            PhysicsSettings::new(0, 20, -20)
                .with_max_horizontal_speed(20)
                .expect("horizontal speed valid"),
        );
        engine
            .register_definition(terrain_definition)
            .expect("terrain passer registers");
        engine
            .register_definition(world_bound_definition)
            .expect("world-bound mover registers");
        engine
            .register_definition(layer_definition)
            .expect("layer registers");
        engine
            .register_definition(layer_high_mover_definition)
            .expect("high layer-bound mover registers");
        engine
            .register_definition(layer_low_mover_definition)
            .expect("low layer-bound mover registers");

        let layer = engine
            .spawn_object(
                SpawnConfig::new("ZeroLayer")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(10, 10))
                    .with_loaded(true),
            )
            .expect("layer spawns");
        let terrain = engine
            .spawn_object(
                SpawnConfig::new("ZeroPass")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(5, 8))
                    .with_velocity(Vector2::new(2, 4))
                    .with_mobile(true)
                    .with_loaded(true),
            )
            .expect("terrain passer spawns");
        let world_low = engine
            .spawn_object(
                SpawnConfig::new("ZeroWorld")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(3, 4))
                    .with_velocity(Vector2::new(-6, -6))
                    .with_mobile(true)
                    .with_loaded(true),
            )
            .expect("low world-bound mover spawns");
        let world_high = engine
            .spawn_object(
                SpawnConfig::new("ZeroWorld")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(17, 26))
                    .with_velocity(Vector2::new(6, 6))
                    .with_mobile(true)
                    .with_loaded(true),
            )
            .expect("high world-bound mover spawns");
        let layer_high_mover = engine
            .spawn_object(
                SpawnConfig::new("ZeroLayerHigh")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(16, 16))
                    .with_velocity(Vector2::new(5, 5))
                    .with_layer(layer)
                    .with_mobile(true)
                    .with_loaded(true),
            )
            .expect("high layer-bound mover spawns");
        let layer_low_mover = engine
            .spawn_object(
                SpawnConfig::new("ZeroLayerLow")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(12, 12))
                    .with_velocity(Vector2::new(-5, -5))
                    .with_layer(layer)
                    .with_mobile(true)
                    .with_loaded(true),
            )
            .expect("low layer-bound mover spawns");
        let terrain_idx = engine
            .find_object_index(terrain)
            .expect("terrain passer exists");
        engine.objects[terrain_idx].frame_t_contact = CNAT_LEFT;

        let snapshot = engine.tick().expect("zero-vertex movement tick succeeds");

        let terrain_snapshot = snapshot.object(terrain).expect("terrain passer remains");
        assert!(terrain_snapshot.vertices.is_empty());
        assert_eq!(terrain_snapshot.position, Vector2::new(7, 12));
        assert_eq!(terrain_snapshot.velocity, Vector2::new(2, 4));
        let terrain_idx = engine
            .find_object_index(terrain)
            .expect("terrain passer remains live");
        assert_eq!(
            engine.objects[terrain_idx].fixed_position,
            FixedVec2::from_ints(7, 12)
        );
        assert_eq!(
            engine.objects[terrain_idx].fixed_velocity,
            FixedVec2::from_ints(2, 4),
            "solid material friction must not be synthesized without vertices"
        );
        assert_eq!(engine.objects[terrain_idx].motion_x, 2);
        assert_eq!(engine.objects[terrain_idx].motion_y, 4);
        assert_eq!(engine.objects[terrain_idx].frame_t_contact, CNAT_NONE);
        assert!(
            terrain_calls.lock().unwrap().is_empty(),
            "empty terrain probes must not synthesize Contact* or Hit calls"
        );

        let world_low_snapshot = snapshot.object(world_low).expect("low mover remains");
        assert_eq!(world_low_snapshot.position, Vector2::new(2, 3));
        assert_eq!(world_low_snapshot.velocity, Vector2::ZERO);
        let world_low_idx = engine
            .find_object_index(world_low)
            .expect("low mover remains live");
        assert_eq!(
            engine.objects[world_low_idx].fixed_position,
            FixedVec2::from_ints(-3, -2),
            "TargetBounds clamps only the integer target"
        );

        let world_high_snapshot = snapshot.object(world_high).expect("high mover remains");
        assert_eq!(world_high_snapshot.position, Vector2::new(18, 27));
        assert_eq!(world_high_snapshot.velocity, Vector2::ZERO);
        let world_high_idx = engine
            .find_object_index(world_high)
            .expect("high mover remains live");
        assert_eq!(
            engine.objects[world_high_idx].fixed_position,
            FixedVec2::from_ints(23, 32)
        );

        let layer_high_snapshot = snapshot
            .object(layer_high_mover)
            .expect("high layer mover remains");
        assert_eq!(layer_high_snapshot.position, Vector2::new(17, 17));
        assert_eq!(layer_high_snapshot.velocity, Vector2::ZERO);
        let layer_high_idx = engine
            .find_object_index(layer_high_mover)
            .expect("high layer mover remains live");
        assert_eq!(
            engine.objects[layer_high_idx].fixed_position,
            FixedVec2::from_ints(21, 21)
        );

        let layer_low_snapshot = snapshot
            .object(layer_low_mover)
            .expect("low layer mover remains");
        assert_eq!(layer_low_snapshot.position, Vector2::new(11, 11));
        assert_eq!(layer_low_snapshot.velocity, Vector2::ZERO);
        let layer_low_idx = engine
            .find_object_index(layer_low_mover)
            .expect("low layer mover remains live");
        assert_eq!(
            engine.objects[layer_low_idx].fixed_position,
            FixedVec2::from_ints(7, 7)
        );

        assert_eq!(
            *world_bound_calls.lock().unwrap(),
            vec![
                "ContactLeft".to_string(),
                "ContactTop".to_string(),
                "ContactRight".to_string(),
                "ContactBottom".to_string(),
            ],
            "world bounds retain native movement and directional callback order"
        );
        assert_eq!(
            *layer_high_bound_calls.lock().unwrap(),
            vec!["ContactRight".to_string(), "ContactBottom".to_string()],
            "high layer bounds retain their native directional callbacks"
        );
        assert_eq!(
            *layer_low_bound_calls.lock().unwrap(),
            vec!["ContactLeft".to_string(), "ContactTop".to_string()],
            "low layer bounds retain their native directional callbacks"
        );
    }

    #[test]
    fn zero_vertex_attached_movement_runs_attachment_loss_action() {
        use std::sync::{Arc, Mutex};

        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut hooks = DebuggerHooks::new();
        {
            let calls = Arc::clone(&calls);
            hooks.set_on_call(move |name, _args| {
                if name == "OnJumpStart" || name == "OnSlideAbort" {
                    calls.lock().unwrap().push(name.to_string());
                }
            });
        }
        let mut definition = Definition::from_script(
            "ZeroAttach",
            "ZeroAttach",
            r#"
            global func OnSlideAbort() { return 0; }
            global func OnJumpStart() { return 0; }
            "#,
        )
        .expect("attachment script compiles");
        definition.set_debugger_hooks(hooks);
        definition.set_shape_vertices(Vec::new());
        definition.configure_actions(
            Some("Idle".to_string()),
            HashMap::from([
                ("Idle".to_string(), ActionSpec::default()),
                (
                    "Slide".to_string(),
                    ActionSpec::default()
                        .with_attach(CNAT_BOTTOM)
                        .with_abort_call("OnSlideAbort"),
                ),
                (
                    "Jump".to_string(),
                    ActionSpec::default().with_start_call("OnJumpStart"),
                ),
            ]),
        );

        let mut engine = Engine::with_seed(29);
        engine.set_landscape(Landscape::flat(20, 20));
        engine.set_physics(PhysicsSettings::new(0, 20, -20));
        engine
            .register_definition(definition)
            .expect("zero-vertex attached definition registers");
        let object = engine
            .spawn_object(
                SpawnConfig::new("ZeroAttach")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(5, 5))
                    .with_action(ActionState::new("Slide"))
                    .with_mobile(true),
            )
            .expect("attached object spawns");
        let index = engine.find_object_index(object).expect("object exists");
        engine.objects[index].frame_t_contact = CNAT_LEFT;
        engine.objects[index].state.shape_attach = ShapeAttachRecord {
            mat_valid: true,
            mat_vehicle: true,
            x: 9,
            y: 10,
            vtx: 3,
        };

        let snapshot = engine.tick().expect("attachment-loss tick succeeds");
        assert_eq!(
            snapshot.object(object).expect("object remains").action.name,
            "Jump"
        );
        assert_eq!(
            *calls.lock().unwrap(),
            vec!["OnJumpStart".to_string(), "OnSlideAbort".to_string()]
        );
        let index = engine.find_object_index(object).expect("object remains");
        assert_eq!(engine.objects[index].frame_t_contact, CNAT_NONE);
        assert!(!engine.objects[index].state.shape_attach.mat_valid);
        assert!(!engine.objects[index].state.shape_attach.mat_vehicle);
        assert_eq!(engine.objects[index].state.shape_attach.x, 9);
        assert_eq!(engine.objects[index].state.shape_attach.y, 10);
        assert_eq!(engine.objects[index].state.shape_attach.vtx, 3);
    }

    #[test]
    fn zero_vertex_rotation_uses_empty_contact_checks() {
        let mut definition = simple_definition("ZeroTurn");
        definition.set_shape_vertices(Vec::new());
        definition.set_rotateable(360);

        let mut engine = Engine::with_seed(31);
        engine.set_landscape(Landscape::flat(20, 20));
        engine.set_physics(PhysicsSettings::new(0, 20, -20));
        engine
            .register_definition(definition)
            .expect("zero-vertex rotating definition registers");
        let object = engine
            .spawn_object(
                SpawnConfig::new("ZeroTurn")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(5, 5))
                    .with_rotation_velocity(itofix(1))
                    .with_mobile(true),
            )
            .expect("rotating object spawns");
        let index = engine.find_object_index(object).expect("object exists");
        engine.objects[index].frame_t_contact = CNAT_LEFT;

        engine.tick_without_snapshot().expect("rotation tick succeeds");

        let index = engine.find_object_index(object).expect("object remains");
        assert_eq!(engine.objects[index].state.rotation, 5);
        assert_eq!(engine.objects[index].fixed_rotation, itofix(5));
        assert_eq!(engine.objects[index].frame_t_contact, CNAT_NONE);
    }

    #[test]
    fn spawn_initializes_vertices_from_definition_shape() {
        let mut definition = simple_definition("Rock");
        definition.set_shape_vertices(vec![
            ObjectVertex::new(-1, 1)
                .with_cnat(CNAT_BOTTOM)
                .with_friction(80),
            ObjectVertex::new(1, 1)
                .with_cnat(CNAT_BOTTOM)
                .with_friction(80),
        ]);

        let mut engine = Engine::with_seed(29);
        engine
            .register_definition(definition)
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("Rock"))
            .expect("spawn succeeds");

        let snapshot = engine.object_snapshot(id).expect("snapshot exists");
        assert_eq!(snapshot.vertices.len(), 2);
        assert_eq!(snapshot.vertices[0].x, -1);
        assert_eq!(snapshot.vertices[0].cnat, CNAT_BOTTOM);
        assert_eq!(snapshot.vertices[0].friction, 80);
    }
