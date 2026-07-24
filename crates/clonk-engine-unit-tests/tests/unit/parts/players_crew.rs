    #[test]
    fn spawn_object_tracks_owner() {
        let mut engine = Engine::with_seed(99);
        engine
            .register_definition(build_definition())
            .expect("definition registers");

        let id = engine
            .spawn_object(
                SpawnConfig::new("Test")
                    .with_position(Vector2::new(0, 0))
                    .with_owner(2),
            )
            .expect("spawn succeeds");

        let snapshot = engine.object_snapshot(id).expect("snapshot available");
        assert_eq!(snapshot.owner, 2);
    }

    #[test]
    fn crew_members_enumerates_owned_crew() {
        let mut engine = Engine::with_seed(0);
        let mut definition = build_definition();
        definition.set_crew_member(true);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let crew_owner_one = engine
            .spawn_object(
                SpawnConfig::new("Test")
                    .with_alive(true)
                    .with_owner(1)
                    .with_crew_member(true),
            )
            .expect("spawn succeeds");
        let crew_owner_two = engine
            .spawn_object(
                SpawnConfig::new("Test")
                    .with_alive(true)
                    .with_owner(2)
                    .with_crew_member(true),
            )
            .expect("spawn succeeds");
        engine
            .spawn_object(
                SpawnConfig::new("Test")
                    .with_owner(1)
                    .with_crew_member(false),
            )
            .expect("spawn succeeds");

        let mut owner_one_members = engine.crew_members(1);
        owner_one_members.sort_by_key(|id| id.as_u64());
        assert_eq!(owner_one_members, vec![crew_owner_one]);

        assert_eq!(engine.crew_members(2), vec![crew_owner_two]);
        assert!(engine.crew_members(3).is_empty());
    }

    #[test]
    fn select_crew_tracks_selection_and_cursor() {
        let mut engine = Engine::with_seed(0);
        let mut definition = build_definition();
        definition.set_crew_member(true);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let first = engine
            .spawn_object(
                SpawnConfig::new("Test")
                    .with_alive(true)
                    .with_owner(1)
                    .with_crew_member(true),
            )
            .expect("spawn succeeds");
        let second = engine
            .spawn_object(
                SpawnConfig::new("Test")
                    .with_alive(true)
                    .with_owner(1)
                    .with_crew_member(true),
            )
            .expect("spawn succeeds");

        engine
            .select_crew(1, vec![first])
            .expect("selection succeeds");

        assert_eq!(engine.selected_crew(1), vec![first]);
        assert_eq!(engine.crew_cursor(1), Some(first));

        engine
            .select_crew(1, vec![second])
            .expect("second selection succeeds");

        let mut selected = engine.selected_crew(1);
        selected.sort_by_key(|id| id.as_u64());
        assert_eq!(selected, vec![first, second]);
        assert_eq!(engine.crew_cursor(1), Some(first));
    }

    #[test]
    fn selected_is_persisted_on_the_object_like_cpp() -> Result<(), EngineError> {
        // C4Object::Select is object-owned state and CompileFunc writes it
        // as `Selected`, independently of C4Player::Cursor
        // (C4Object.h:153; C4Object.cpp:2800).
        let mut engine = Engine::with_seed(0);
        let mut definition = build_definition();
        definition.set_crew_member(true);
        engine.register_definition(definition)?;
        engine.register_player(PlayerConfig::new(1, "Selector"))?;
        let crew = engine.spawn_object(
            SpawnConfig::new("Test")
                .with_alive(true)
                .with_owner(1)
                .with_crew_member(true),
        )?;
        engine.select_crew(1, [crew])?;

        let encoded = serde_json::to_value(engine.capture_state())
            .expect("engine state serializes to a JSON value");
        assert_eq!(
            encoded["objects"][0]["snapshot"]["selected"].as_bool(),
            Some(true)
        );
        Ok(())
    }

    #[test]
    fn selected_crew_follows_player_roster_not_object_number() -> Result<(), EngineError> {
        // FnGetCursor walks C4Player::Crew link order and tests each object's
        // Select bit (C4Script.cpp:2905-2928). Object Number is unrelated to
        // that ordering in loaded games.
        let mut engine = Engine::with_seed(0);
        let mut definition = build_definition();
        definition.set_crew_member(true);
        engine.register_definition(definition)?;
        engine.register_player(PlayerConfig::new(1, "Roster"))?;
        let high = engine.spawn_object(
            SpawnConfig::new("Test")
                .with_id(ObjectId::new(500))
                .with_alive(true)
                .with_owner(1)
                .with_crew_member(true),
        )?;
        let low = engine.spawn_object(
            SpawnConfig::new("Test")
                .with_id(ObjectId::new(7))
                .with_alive(true)
                .with_owner(1)
                .with_crew_member(true),
        )?;
        engine.select_crew(1, [high, low])?;

        assert_eq!(engine.selected_crew(1), engine.player(1).unwrap().crew());
        assert_eq!(engine.selected_crew(1), vec![low, high]);
        Ok(())
    }

    #[test]
    fn select_crew_callbacks_are_synchronous_and_see_object_bits() -> Result<(), EngineError> {
        // FnSelectCrew's no-adjust branch invokes C4Object::DoSelect/
        // UnSelect directly (C4Script.cpp:2965-2975). The ordinary branch
        // then runs AdjustCursorCommand, whose exact callback order is target
        // UnSelect(false), old-cursor UnSelect(true), new-cursor
        // DoSelect(false) (C4Player.cpp:1996-2006, 1235-1258).
        let script = r#"#strict
static iSelectionLog;
local iCode, iVisibility;

func SetCode(iValue) { iCode = iValue; return 1; }
func ResetSelectionLog() { iSelectionLog = 0; return 1; }
func ReadSelectionLog() { return iSelectionLog; }
func RunSelectCrew(iPlayer, pTarget, fSelect, fNoAdjust)
{
    return SelectCrew(iPlayer, pTarget, fSelect, fNoAdjust);
}

func CrewSelection(fUnselect, fCursor)
{
    var iDigit = 0;
    if (iCode == 1 && fUnselect && !fCursor) iDigit = 1;
    if (iCode == 1 && fUnselect && fCursor) iDigit = 2;
    if (iCode == 2 && !fUnselect && !fCursor) iDigit = 3;
    if (iDigit) iSelectionLog = iSelectionLog * 10 + iDigit;

    // FnGetCursor scans the live C4Object::Select bits directly
    // (C4Script.cpp:2905-2928), proving the bit changed before callback.
    if (!fCursor && !fUnselect && GetCursor(GetOwner(), 1) == this())
        iVisibility = iVisibility * 10 + 1;
    if (!fCursor && fUnselect && !GetCursor(GetOwner(), 1))
        iVisibility = iVisibility * 10 + 2;
    return 1;
}
"#;
        let mut engine = Engine::with_seed(0);
        let mut definition = Definition::from_script("SELC", "Select callback", script)?;
        definition.set_crew_member(true);
        engine.register_definition(definition)?;
        engine.register_player(PlayerConfig::new(1, "Selector"))?;
        let a = engine.spawn_object(
            SpawnConfig::new("SELC")
                .with_owner(1)
                .with_alive(true)
                .with_crew_member(true),
        )?;
        let b = engine.spawn_object(
            SpawnConfig::new("SELC")
                .with_owner(1)
                .with_alive(true)
                .with_crew_member(true),
        )?;

        let call = |engine: &mut Engine, id: ObjectId, name: &str, args: Vec<Value>| {
            let index = engine.find_object_index(id).expect("object exists");
            engine.call_object_function(index, name, args)
        };
        call(&mut engine, a, "SetCode", vec![Value::Int(1)])?;
        call(&mut engine, b, "SetCode", vec![Value::Int(2)])?;
        engine.select_crew(1, [a])?;
        engine.set_crew_cursor(1, Some(a))?;
        call(&mut engine, a, "ResetSelectionLog", Vec::new())?;

        // No cursor adjustment: B appears as selected index 1 inside its
        // select callback and disappears before its unselect callback.
        call(
            &mut engine,
            a,
            "RunSelectCrew",
            vec![
                Value::Int(1),
                Value::Object(b.as_u64()),
                Value::Bool(true),
                Value::Bool(true),
            ],
        )?;
        call(
            &mut engine,
            a,
            "RunSelectCrew",
            vec![
                Value::Int(1),
                Value::Object(b.as_u64()),
                Value::Bool(false),
                Value::Bool(true),
            ],
        )?;
        let b_index = engine.find_object_index(b).expect("B exists");
        assert_eq!(
            engine.objects[b_index].state.local_vars.get("iVisibility"),
            Some(&Value::Int(12))
        );
        assert!(!engine.objects[b_index].state.selected);

        // Preselect B without adjustment, then normally unselect A. The
        // shared log pins target/old-cursor/new-cursor callback order.
        call(
            &mut engine,
            a,
            "RunSelectCrew",
            vec![
                Value::Int(1),
                Value::Object(b.as_u64()),
                Value::Bool(true),
                Value::Bool(true),
            ],
        )?;
        call(&mut engine, a, "ResetSelectionLog", Vec::new())?;
        call(
            &mut engine,
            a,
            "RunSelectCrew",
            vec![
                Value::Int(1),
                Value::Object(a.as_u64()),
                Value::Bool(false),
                Value::Bool(false),
            ],
        )?;
        assert_eq!(
            call(&mut engine, a, "ReadSelectionLog", Vec::new())?,
            Value::Int(123)
        );
        assert_eq!(engine.crew_cursor(1), Some(b));
        assert_eq!(engine.selected_crew(1), vec![b]);
        Ok(())
    }

    #[test]
    fn set_cursor_accepts_magi_helper_and_callbacks_see_new_cursor() -> Result<(), EngineError> {
        // FnSetCursor accepts any active object; C4Player::SetCursor writes
        // Cursor before old/new cursor-only callbacks (C4Script.cpp:2945-2958;
        // C4Player.cpp:1831-1845). Magi's ComboMenu/Selector/Aimer depend on
        // a noncrew helper cursor and fNoSelectCrew preserving Select bits.
        let script = r#"#strict
local iCursorSeen;
func ResetSeen() { iCursorSeen = 0; return 1; }
func RunSetCursor(iPlayer, pTarget, fNoSelectCrew)
{
    return SetCursor(iPlayer, pTarget, true, true, fNoSelectCrew);
}
func RunSelectCrew(iPlayer, pTarget, fSelect, fNoAdjust)
{
    return SelectCrew(iPlayer, pTarget, fSelect, fNoAdjust);
}
func CrewSelection(fUnselect, fCursor)
{
    if (fCursor && GetCursor(GetOwner()) == this())
        iCursorSeen = iCursorSeen * 10 + 1;
    if (fCursor && GetCursor(GetOwner()) != this())
        iCursorSeen = iCursorSeen * 10 + 2;
    return 1;
}
"#;
        let mut engine = Engine::with_seed(0);
        let mut crew_definition = Definition::from_script("MAGE", "Mage", script)?;
        crew_definition.set_crew_member(true);
        engine.register_definition(crew_definition)?;
        engine.register_definition(Definition::from_script("HELP", "Combo helper", script)?)?;
        engine.register_player(PlayerConfig::new(1, "Mage"))?;
        let mage = engine.spawn_object(
            SpawnConfig::new("MAGE")
                .with_owner(1)
                .with_alive(true)
                .with_crew_member(true),
        )?;
        let helper = engine.spawn_object(
            SpawnConfig::new("HELP")
                .with_owner(1)
                .with_alive(true),
        )?;
        engine.select_crew(1, [mage])?;
        engine.set_crew_cursor(1, Some(mage))?;

        let mage_index = engine.find_object_index(mage).expect("mage exists");
        engine.call_object_function(mage_index, "ResetSeen", Vec::new())?;
        let helper_index = engine.find_object_index(helper).expect("helper exists");
        engine.call_object_function(helper_index, "ResetSeen", Vec::new())?;
        engine.call_object_function(
            mage_index,
            "RunSetCursor",
            vec![
                Value::Int(1),
                Value::Object(helper.as_u64()),
                Value::Bool(true),
            ],
        )?;
        assert_eq!(engine.crew_cursor(1), Some(helper));
        assert!(!engine.objects[helper_index].state.selected);
        assert_eq!(
            engine.objects[mage_index].state.local_vars.get("iCursorSeen"),
            Some(&Value::Int(2)),
            "old cursor observes the already-installed helper cursor"
        );
        assert_eq!(
            engine.objects[helper_index]
                .state
                .local_vars
                .get("iCursorSeen"),
            Some(&Value::Int(1)),
            "new helper observes itself as cursor"
        );

        // Magi's helper then unselects the mage without cursor adjustment.
        engine.call_object_function(
            helper_index,
            "RunSelectCrew",
            vec![
                Value::Int(1),
                Value::Object(mage.as_u64()),
                Value::Bool(false),
                Value::Bool(true),
            ],
        )?;
        assert_eq!(engine.crew_cursor(1), Some(helper));
        assert!(!engine.objects[mage_index].state.selected);
        Ok(())
    }

    #[test]
    fn disabling_cursor_clears_select_silently_then_adjusts() -> Result<(), EngineError> {
        // FnSetCrewEnabled clears Select without CrewSelection(false/true,
        // false). If the disabled object was Cursor, AdjustCursorCommand may
        // subsequently emit old-cursor (true,true) and new-cursor
        // (false,false) callbacks (C4Script.cpp:4814-4836).
        let script = r#"#strict
local iCursorCallbacks, iSelectCallbacks;
func RunSelectCrew(iPlayer, pTarget, fSelect, fNoAdjust)
{
    return SelectCrew(iPlayer, pTarget, fSelect, fNoAdjust);
}
func DisableAndCount(iPlayer, pTarget)
{
    SetCrewEnabled(false, pTarget);
    return GetSelectCount(iPlayer);
}
func SetEnabled(fEnabled, pTarget)
{
    return SetCrewEnabled(fEnabled, pTarget);
}
func ResetCallbacks()
{
    iCursorCallbacks = iSelectCallbacks = 0;
    return 1;
}
func CrewSelection(fUnselect, fCursor)
{
    if (fCursor) iCursorCallbacks = iCursorCallbacks + 1;
    else iSelectCallbacks = iSelectCallbacks + 1;
    return 1;
}
"#;
        let mut engine = Engine::with_seed(0);
        let mut definition = Definition::from_script("DSBL", "Disable crew", script)?;
        definition.set_crew_member(true);
        engine.register_definition(definition)?;
        engine.register_player(PlayerConfig::new(1, "Selector"))?;
        let a = engine.spawn_object(
            SpawnConfig::new("DSBL")
                .with_owner(1)
                .with_alive(true)
                .with_crew_member(true),
        )?;
        let b = engine.spawn_object(
            SpawnConfig::new("DSBL")
                .with_owner(1)
                .with_alive(true)
                .with_crew_member(true),
        )?;
        engine.select_crew(1, [a])?;
        engine.set_crew_cursor(1, Some(a))?;
        let call = |engine: &mut Engine, id: ObjectId, name: &str, args: Vec<Value>| {
            let index = engine.find_object_index(id).expect("object exists");
            engine.call_object_function(index, name, args)
        };
        call(
            &mut engine,
            b,
            "RunSelectCrew",
            vec![
                Value::Int(1),
                Value::Object(b.as_u64()),
                Value::Bool(true),
                Value::Bool(true),
            ],
        )?;
        engine.tick_player_systems()?;
        assert_eq!(engine.player(1).expect("player").select_count(), 2);
        call(&mut engine, a, "ResetCallbacks", Vec::new())?;
        call(&mut engine, b, "ResetCallbacks", Vec::new())?;

        assert_eq!(
            call(
                &mut engine,
                b,
                "DisableAndCount",
                vec![Value::Int(1), Value::Object(a.as_u64())],
            )?,
            Value::Int(2),
            "same-call GetSelectCount retains the cached pre-disable count"
        );
        assert_eq!(engine.player(1).expect("player").select_count(), 2);
        engine.tick_player_systems()?;
        assert_eq!(
            engine.player(1).expect("player").select_count(),
            1,
            "the next Player::Execute refreshes the cache"
        );
        let a_index = engine.find_object_index(a).expect("A exists");
        let b_index = engine.find_object_index(b).expect("B exists");
        assert!(engine.objects[a_index].state.crew_disabled);
        assert!(!engine.objects[a_index].state.selected);
        assert_eq!(engine.crew_cursor(1), Some(b));
        assert_eq!(
            engine.objects[a_index]
                .state
                .local_vars
                .get("iCursorCallbacks"),
            Some(&Value::Int(1))
        );
        assert_eq!(
            engine.objects[a_index]
                .state
                .local_vars
                .get("iSelectCallbacks"),
            Some(&Value::Nil),
            "the silent Select clear did not emit a noncursor callback"
        );
        assert_eq!(
            engine.objects[b_index]
                .state
                .local_vars
                .get("iSelectCallbacks"),
            Some(&Value::Int(1))
        );

        // Disabled DoSelect is callback-less; UnSelect still calls.
        call(&mut engine, a, "ResetCallbacks", Vec::new())?;
        for select in [true, false] {
            call(
                &mut engine,
                b,
                "RunSelectCrew",
                vec![
                    Value::Int(1),
                    Value::Object(a.as_u64()),
                    Value::Bool(select),
                    Value::Bool(true),
                ],
            )?;
        }
        assert_eq!(
            engine.objects[a_index]
                .state
                .local_vars
                .get("iSelectCallbacks"),
            Some(&Value::Int(1))
        );

        // SetCrewEnabled's bool parameter uses C4Value::getBool, so every
        // nonzero integer enables the crew object (C4Script.cpp:4814-4836;
        // C4Value.h:161,325-331).
        assert_eq!(
            call(
                &mut engine,
                b,
                "SetEnabled",
                vec![Value::Int(2), Value::Object(a.as_u64())],
            )?,
            Value::Bool(true)
        );
        assert!(!engine.objects[a_index].state.crew_disabled);
        Ok(())
    }

    #[test]
    fn crew_selection_callback_errors_are_fail_safe() -> Result<(), EngineError> {
        // C4Object::Call is fail-safe by default: the Select write and the
        // callback's pre-error writes persist, and the selecting script
        // continues (C4Object.h:240; C4Object.cpp:2224-2227, 5815-5824).
        let script = r#"#strict
local iCallbackCalls, iAfter;
func Run(iPlayer, pTarget)
{
    SelectCrew(iPlayer, pTarget, true, true);
    iAfter = 1;
    return 1;
}
func CrewSelection()
{
    iCallbackCalls = iCallbackCalls + 1;
    MissingCrewSelectionFunction();
}
"#;
        let mut engine = Engine::with_seed(0);
        let mut definition = Definition::from_script("SAFE", "Fail-safe selection", script)?;
        definition.set_crew_member(true);
        engine.register_definition(definition)?;
        engine.register_player(PlayerConfig::new(1, "Selector"))?;
        let caller = engine.spawn_object(
            SpawnConfig::new("SAFE")
                .with_owner(1)
                .with_alive(true),
        )?;
        let target = engine.spawn_object(
            SpawnConfig::new("SAFE")
                .with_owner(1)
                .with_alive(true),
        )?;
        let caller_index = engine.find_object_index(caller).expect("caller exists");
        assert_eq!(
            engine.call_object_function(
                caller_index,
                "Run",
                vec![Value::Int(1), Value::Object(target.as_u64())],
            )?,
            Value::Int(1)
        );
        let caller_index = engine.find_object_index(caller).expect("caller exists");
        let target_index = engine.find_object_index(target).expect("target exists");
        assert!(engine.objects[target_index].state.selected);
        assert_eq!(
            engine.objects[target_index]
                .state
                .local_vars
                .get("iCallbackCalls"),
            Some(&Value::Int(1))
        );
        assert_eq!(
            engine.objects[caller_index].state.local_vars.get("iAfter"),
            Some(&Value::Int(1))
        );
        Ok(())
    }

    #[test]
    fn register_player_populates_snapshot_state() -> Result<(), EngineError> {
        let mut engine = Engine::new();
        engine.register_player(PlayerConfig::new(1, "Alice").with_wealth(75))?;
        let mut definition = Definition::from_script("Walker", "Walker", PASSIVE_PLAYER_SCRIPT)?;
        definition.set_crew_member(true);
        engine.register_definition(definition)?;
        let crew = engine.spawn_object(
            SpawnConfig::new("Walker")
                .with_alive(true)
                .with_owner(1)
                .with_crew_member(true)
                .with_position(Vector2::new(0, 0)),
        )?;
        assert_eq!(engine.player(1).unwrap().crew(), &[crew]);
        let snapshot = engine.snapshot();
        let player_state = snapshot
            .players
            .iter()
            .find(|state| state.id == 1)
            .expect("player state present");
        assert_eq!(player_state.name, "Alice");
        assert_eq!(player_state.wealth, 75);
        assert_eq!(player_state.status, PlayerStatus::Active);
        assert_eq!(player_state.crew, vec![crew]);
        Ok(())
    }

    #[test]
    fn player_asset_value_accounts_for_owned_objects() -> Result<(), EngineError> {
        let mut engine = Engine::new();
        engine.register_player(
            PlayerConfig::new(1, "Miner")
                .with_wealth(25)
                .with_points(10),
        )?;

        let mut definition = Definition::from_script("Ore", "Ore", PASSIVE_PLAYER_SCRIPT)?;
        definition.set_value(60);
        engine.register_definition(definition)?;

        engine.spawn_object(SpawnConfig::new("Ore").with_owner(1))?;

        engine.update_player_asset_values()?;

        let player = engine.player(1).expect("player present");
        assert_eq!(player.value(), 95);
        assert_eq!(
            player.value_gain(),
            60,
            "the post-FinalInit ore is a real gain over the initial 35"
        );
        assert_eq!(player.objects_owned(), 1);
        Ok(())
    }

    #[test]
    fn player_asset_value_uses_cpp_get_value_chain_on_tick35() -> Result<(), EngineError> {
        let mut engine = Engine::new();
        engine.register_player(
            PlayerConfig::new(1, "Builder")
                .with_wealth(7)
                .with_points(10),
        )?;

        let mut crew = Definition::from_script("Crew", "Crew", "#strict 2\n")?;
        crew.set_crew_member(true);
        engine.register_definition(crew)?;
        engine.spawn_object(
            SpawnConfig::new("Crew")
                .with_owner(1)
                .with_alive(true)
                .with_crew_member(true),
        )?;

        let mut half_built =
            Definition::from_script("HalfBuilt", "Half-built", "#strict 2\n")?;
        half_built.set_value(101);
        half_built.set_category(CATEGORY_STRUCTURE);
        engine.register_definition(half_built)?;
        engine.spawn_object(
            SpawnConfig::new("HalfBuilt")
                .with_owner(1)
                .with_construction(FULL_CON / 2),
        )?;

        let mut object_override = Definition::from_script(
            "ObjectOverride",
            "Object override",
            r#"#strict 2
protected func CalcValue(object base, int player)
{
    if (base) return 900;
    return 40 + player;
}
public func ReadCachedValue(int player) { return GetPlrValue(player); }
public func ReadCachedGain(int player) { return GetPlrValueGain(player); }
"#,
        )?;
        object_override.set_value(999);
        engine.register_definition(object_override)?;
        let value_probe = engine.spawn_object(
            SpawnConfig::new("ObjectOverride").with_owner(1),
        )?;

        let mut definition_override = Definition::from_script(
            "DefinitionOverride",
            "Definition override",
            r#"#strict 2
protected func CalcDefValue(object base, int player)
{
    if (base) return 800;
    return 30 + player;
}
"#,
        )?;
        definition_override.set_value(888);
        engine.register_definition(definition_override)?;
        engine.spawn_object(SpawnConfig::new("DefinitionOverride").with_owner(1))?;

        for _ in 0..34 {
            engine.tick_without_snapshot()?;
        }
        assert_eq!(engine.frame(), 34);
        let probe_index = engine
            .find_object_index(value_probe)
            .expect("value probe remains active");
        assert_eq!(
            engine.call_object_function(
                probe_index,
                "ReadCachedValue",
                vec![Value::Int(1)],
            )?,
            Value::Int(17),
            "GetPlrValue stays at the FinalInit points-plus-wealth baseline"
        );
        let probe_index = engine
            .find_object_index(value_probe)
            .expect("value probe remains active");
        assert_eq!(
            engine.call_object_function(
                probe_index,
                "ReadCachedGain",
                vec![Value::Int(1)],
            )?,
            Value::Int(0)
        );

        engine.tick_without_snapshot()?;
        assert_eq!(engine.frame(), 35);
        let probe_index = engine
            .find_object_index(value_probe)
            .expect("value probe remains active");
        assert_eq!(
            engine.call_object_function(
                probe_index,
                "ReadCachedValue",
                vec![Value::Int(1)],
            )?,
            Value::Int(139),
            "17 baseline + 50 half-built + 41 CalcValue + 31 CalcDefValue"
        );
        let player = engine.player(1).expect("player remains active");
        assert_eq!(player.initial_value(), 17);
        assert_eq!(player.value_gain(), 122);
        assert_eq!(player.objects_owned(), 4);
        Ok(())
    }

    #[test]
    fn calc_value_sees_the_live_partial_player_value_on_each_tick35() -> Result<(), EngineError> {
        let mut engine = Engine::new();
        engine.register_player(
            PlayerConfig::new(1, "Builder")
                .with_wealth(7)
                .with_points(10),
        )?;

        let mut crew = Definition::from_script("Crew", "Crew", "#strict 2\n")?;
        crew.set_crew_member(true);
        engine.register_definition(crew)?;
        engine.spawn_object(
            SpawnConfig::new("Crew")
                .with_owner(1)
                .with_alive(true)
                .with_crew_member(true),
        )?;

        let mut mirror = Definition::from_script(
            "ValueMirror",
            "Value mirror",
            r#"#strict 2
protected func CalcValue(object base, int player)
{
    return GetPlrValue(player);
}
"#,
        )?;
        mirror.set_value(999);
        engine.register_definition(mirror)?;
        engine.spawn_object(SpawnConfig::new("ValueMirror").with_owner(1))?;

        for _ in 0..35 {
            engine.tick_without_snapshot()?;
        }
        assert_eq!(engine.player(1).expect("player remains").value(), 34);

        for _ in 0..35 {
            engine.tick_without_snapshot()?;
        }
        let player = engine.player(1).expect("player remains");
        assert_eq!(engine.frame(), 70);
        assert_eq!(
            player.value(),
            34,
            "each pass resets the live accumulator to points plus wealth before CalcValue"
        );
        assert_eq!(player.value_gain(), 17);
        assert_eq!(player.objects_owned(), 2);
        Ok(())
    }

    #[test]
    fn calc_value_created_later_master_link_is_valued_in_the_same_pass(
    ) -> Result<(), EngineError> {
        let mut engine = Engine::new();
        engine.register_player(PlayerConfig::new(1, "Builder"))?;

        let mut child = Definition::from_script("CHLD", "Child", "#strict 2\n")?;
        child.set_value(5);
        child.set_category(CATEGORY_STRUCTURE);
        engine.register_definition(child)?;

        let mut parent = Definition::from_script(
            "PARN",
            "Parent",
            r#"#strict 2
local created;
protected func CalcValue(object base, int player)
{
    if (!created)
    {
        created = 1;
        CreateObject(CHLD, 0, 0, player);
    }
    return 10;
}
"#,
        )?;
        parent.set_category(CATEGORY_OBJECT);
        engine.register_definition(parent)?;
        engine.spawn_object(SpawnConfig::new("PARN").with_owner(1))?;

        for _ in 0..35 {
            engine.tick_without_snapshot()?;
        }

        let player = engine.player(1).expect("player remains");
        assert_eq!(player.value(), 15);
        assert_eq!(player.value_gain(), 15);
        assert_eq!(player.objects_owned(), 2);
        Ok(())
    }

    fn lifecycle_join_config(
        name: &str,
        crew: Vec<player_file::CrewInfo>,
    ) -> JoinPlayerConfig {
        JoinPlayerConfig {
            name: name.to_string(),
            player_info_id: 1,
            score: 0,
            rounds: 0,
            rounds_won: 0,
            rounds_lost: 0,
            total_playing_time: 0,
            team: None,
            color_dw: 0xff0000,
            pref_color: 0,
            pref_position: 0,
            crew,
            control_style: false,
            auto_context_menu: false,
            startup_player_count: 1,
        }
    }

    #[test]
    fn player_lifecycle_runtime_control_is_visible_to_preinitialize_and_survives_join(
    ) -> Result<(), EngineError> {
        // C4Player::InitControl runs before PreInitializePlayer, so reflection
        // in that callback already sees final Control/MouseControl
        // (C4Player.cpp:323-347,1871-1918).
        let script = r#"#strict 2
static pre_control, pre_mouse;
global func PreInitializePlayer(int player)
{
    pre_control = GetPlayerVal("Control", 0, player);
    pre_mouse = GetPlayerVal("MouseControl", 0, player);
    return 1;
}
"#;
        let mut engine = Engine::with_seed(0);
        engine.install_scenario_script_with_convention("Runtime control", script, true)?;

        let runtime_control = PlayerRuntimeControl::new(6, 1);
        let joined = engine
            .join_player_with_runtime_control(
                lifecycle_join_config("Controller", Vec::new()),
                runtime_control,
            )?
            .number();

        let snapshot = engine.snapshot();
        assert_eq!(
            snapshot.script_globals.named.get("pre_control"),
            Some(&Value::Int(6))
        );
        assert_eq!(
            snapshot.script_globals.named.get("pre_mouse"),
            Some(&Value::Int(1))
        );
        let player = engine.player(joined).expect("joined player remains");
        assert_eq!(player.control_set(), 6);
        assert_eq!(player.mouse_control(), 1);
        Ok(())
    }

    #[test]
    fn player_lifecycle_profile_extra_data_is_visible_to_preinitialize() -> Result<(), EngineError> {
        // C4Player::Init loads its inherited C4PlayerInfoCore before
        // PreInitializePlayer, so GetPlrExtraData observes profile slots in the
        // first callback (C4Player.cpp:267-284,323-347).
        let script = r#"#strict 2
static pre_profile_value;
global func PreInitializePlayer(int player)
{
    pre_profile_value = GetPlrExtraData(player, "Loaded");
    return 1;
}
"#;
        let mut engine = Engine::with_seed(0);
        engine.install_scenario_script_with_convention("Profile preinit", script, true)?;
        let core = player_file::PlayerInfoCoreState {
            extra_data: vec![("Loaded".to_string(), Value::Int(73))],
            ..player_file::PlayerInfoCoreState::default()
        };

        let joined = engine
            .join_player_with_profile_core(
                lifecycle_join_config("Runtime name", Vec::new()),
                PlayerAtClient::HOST,
                "Local",
                None,
                PlayerRuntimeControl::NONE,
                core,
            )?
            .number();

        assert_eq!(
            engine.snapshot().script_globals.named.get("pre_profile_value"),
            Some(&Value::Int(73))
        );
        assert_eq!(
            engine.player(joined).expect("joined player").player_info_core(),
            Some(&player_file::PlayerInfoCoreState {
                extra_data: vec![("Loaded".to_string(), Value::Int(73))],
                ..player_file::PlayerInfoCoreState::default()
            })
        );
        Ok(())
    }

    #[test]
    fn player_file_loads_player_and_crew_extra_data_into_join_state() -> Result<(), EngineError> {
        // C4Player::Load compiles both ordered maps before C4Player::Init;
        // the player map is visible in PreInitializePlayer and the exact
        // C4ObjectInfo is already attached when ready-crew Construction runs
        // (C4Player.cpp:267-347,481-570; C4Object.cpp:158-195).
        let temp = tempfile::tempdir().expect("tempdir");
        let profile_path = temp.path().join("ExtraData.c4p");
        std::fs::create_dir_all(&profile_path).expect("profile directory");

        let mut engine = Engine::with_seed(0);
        engine.register_definition(simple_definition("MARK"))?;
        let marker = engine.spawn_object(SpawnConfig::new("MARK"))?;
        let marker_number = marker.as_u64();

        std::fs::write(
            profile_path.join("Player.txt"),
            format!(
                "[Player]\nName=Loaded\nExtraData=0x9;Nil=A0,Int=i-7,Raw=b7,Flag=b1,Badge=I1145851719,Target=O{marker_number},Hex=i0x10,Legacy=7,Unknown=x7\n\n[Preferences]\nColorDw=255\n"
            ),
        )
        .expect("write player core");
        let crew_path = profile_path.join("Loaded.c4i");
        std::fs::create_dir_all(&crew_path).expect("crew directory");
        std::fs::write(
            crew_path.join("ObjectInfo.txt"),
            format!(
                "[ObjectInfo]\nid=CRWD\nName=Loaded Crew\nParticipation=1\nExtraData=6;CrewInt=i41,CrewNil=A0,CrewRaw=b7,CrewFlag=b1,CrewBadge=I1145851719,CrewObject=O{marker_number}\n\n[Physical]\nEnergy=43000\nBreath=31000\n"
            ),
        )
        .expect("write crew core");
        let malformed_path = profile_path.join("Malformed.c4i");
        std::fs::create_dir_all(&malformed_path).expect("malformed crew directory");
        std::fs::write(
            malformed_path.join("ObjectInfo.txt"),
            "[ObjectInfo]\nid=BRKN\nName=Malformed\nExtraData=2;Good=i1,Bad Name=i2\n",
        )
        .expect("write malformed crew core");

        let resolution = player_file::PersistedC4ValueResolution {
            strings: clonk_script::new_string_registrations(),
            object_numbers: HashSet::from([marker_number]),
        };
        let group = clonk_resources::Group::open(&profile_path).expect("open player group");
        let loaded = player_file::PlayerFile::load_with_portraits_and_value_resolution(
            &group,
            true,
            &resolution,
        )
        .expect("load player file");

        let expected_player = vec![
            ("Nil".to_string(), Value::Nil),
            ("Int".to_string(), Value::Int(-7)),
            ("Raw".to_string(), Value::RawBool(7)),
            ("Flag".to_string(), Value::Bool(true)),
            ("Badge".to_string(), Value::C4Id("GOLD".to_string())),
            ("Target".to_string(), Value::Object(marker_number)),
            ("Hex".to_string(), Value::Int(16)),
            ("Legacy".to_string(), Value::Int(7)),
            ("Unknown".to_string(), Value::Int(7)),
        ];
        assert_eq!(loaded.info_core.extra_data, expected_player);
        let loaded_crew = loaded
            .crew
            .iter()
            .find(|info| info.id == "CRWD")
            .expect("loaded crew info");
        let expected_crew_energy = loaded_crew.physical.energy;
        assert_ne!(
            expected_crew_energy,
            PhysicalInfo::default().energy,
            "fixture must distinguish attached info physicals from the definition fallback"
        );
        let expected_crew = vec![
            ("CrewInt".to_string(), Value::Int(41)),
            ("CrewNil".to_string(), Value::Nil),
            ("CrewRaw".to_string(), Value::RawBool(7)),
            ("CrewFlag".to_string(), Value::Bool(true)),
            ("CrewBadge".to_string(), Value::C4Id("GOLD".to_string())),
            ("CrewObject".to_string(), Value::Object(marker_number)),
        ];
        assert_eq!(loaded_crew.extra_data, expected_crew);
        assert!(
            loaded
                .crew
                .iter()
                .find(|info| info.id == "BRKN")
                .expect("malformed crew still loads")
                .extra_data
                .is_empty(),
            "a malformed default-adapted map resets as a whole"
        );
        let malformed_player_path = temp.path().join("MalformedPlayer.c4p");
        std::fs::create_dir_all(&malformed_player_path)
            .expect("malformed player directory");
        std::fs::write(
            malformed_player_path.join("Player.txt"),
            "[Player]\nName=Malformed Player\nExtraData=2;Good=i1,Bad Name=i2\n",
        )
        .expect("write malformed player core");
        assert!(
            player_file::PlayerFile::load_from_path(&malformed_player_path)
                .expect("malformed default-adapted player map still loads")
                .info_core
                .extra_data
                .is_empty()
        );

        let mut crew_definition = Definition::from_script(
            "CRWD",
            "ExtraData crew",
            r#"#strict 2
protected func Construction()
{
    SetPlrExtraData(GetOwner(), "ConstructionSeen", GetCrewExtraData(0, "CrewInt"));
    SetPlrExtraData(GetOwner(), "ConstructionEnergy", GetEnergy());
    SetCrewExtraData(0, "CrewInt", 99);
    return 1;
}
protected func Initialize()
{
    SetPlrExtraData(GetOwner(), "InitializeSeen", GetCrewExtraData(0, "CrewInt"));
    return 1;
}
protected func Recruitment(int player)
{
    SetPlrExtraData(player, "RecruitmentSeen", GetCrewExtraData(0, "CrewInt"));
    return 1;
}
"#,
        )?;
        crew_definition.set_c4_callback_convention(true);
        crew_definition.set_category(CATEGORY_LIVING);
        crew_definition.set_crew_member(true);
        engine.register_definition(crew_definition)?;
        engine.set_use_fair_crew(false);
        let mut start = PlayerStart::default();
        start.ready_crew = vec![("CRWD".to_string(), 1)];
        start.enforce_position = true;
        engine.set_player_starts(vec![start]);
        engine.install_scenario_script_with_convention(
            "ExtraData join callbacks",
            r#"#strict 2
static pre_nil, pre_int, pre_raw, pre_flag, pre_badge, pre_target;
static initialize_player_seen;
global func PreInitializePlayer(int player)
{
    pre_nil = GetPlrExtraData(player, "Nil");
    pre_int = GetPlrExtraData(player, "Int");
    pre_raw = GetPlrExtraData(player, "Raw");
    pre_flag = GetPlrExtraData(player, "Flag");
    pre_badge = GetPlrExtraData(player, "Badge");
    pre_target = GetPlrExtraData(player, "Target");
    return 1;
}
global func InitializePlayer(int player)
{
    initialize_player_seen = GetCrewExtraData(GetCrew(player), "CrewInt");
    return 1;
}
"#,
            true,
        )?;

        let core = loaded.exact_info_core();
        let joined = engine
            .join_player_with_profile_core(
                JoinPlayerConfig {
                    name: loaded.name.clone(),
                    player_info_id: 1,
                    score: loaded.score,
                    rounds: loaded.rounds,
                    rounds_won: loaded.rounds_won,
                    rounds_lost: loaded.rounds_lost,
                    total_playing_time: loaded.total_playing_time,
                    team: None,
                    color_dw: loaded.normalized_preferred_color(),
                    pref_color: loaded.pref_color,
                    pref_position: loaded.pref_position,
                    crew: loaded.crew.clone(),
                    control_style: loaded.pref_control_style,
                    auto_context_menu: loaded.pref_auto_context_menu,
                    startup_player_count: 1,
                },
                PlayerAtClient::HOST,
                "Local",
                None,
                PlayerRuntimeControl::NONE,
                core,
            )?
            .number();

        let snapshot = engine.snapshot();
        let globals = &snapshot.script_globals.named;
        assert_eq!(globals.get("pre_nil"), Some(&Value::Nil));
        assert_eq!(globals.get("pre_int"), Some(&Value::Int(-7)));
        assert_eq!(globals.get("pre_raw"), Some(&Value::RawBool(7)));
        assert_eq!(globals.get("pre_flag"), Some(&Value::Bool(true)));
        assert_eq!(
            globals.get("pre_badge"),
            Some(&Value::C4Id("GOLD".to_string()))
        );
        assert_eq!(globals.get("pre_target"), Some(&Value::Object(marker_number)));
        assert_eq!(globals.get("initialize_player_seen"), Some(&Value::Int(99)));

        let crew = engine.player(joined).expect("joined player remains").crew()[0];
        let state = engine.capture_state();
        let player_extra_data = &state
            .players
            .iter()
            .find(|player| player.id == joined)
            .expect("joined player state remains")
            .extra_data;
        for (slot, expected) in [
            ("ConstructionSeen", Value::Int(41)),
            (
                "ConstructionEnergy",
                Value::Int(expected_crew_energy / 1_000),
            ),
            ("InitializeSeen", Value::Int(99)),
            ("RecruitmentSeen", Value::Int(99)),
        ] {
            assert_eq!(
                player_extra_data
                    .iter()
                    .find(|(name, _)| name == slot)
                    .map(|(_, value)| value),
                Some(&expected),
                "callback slot {slot}"
            );
        }
        assert_eq!(
            engine.object_snapshot(crew).expect("ready crew remains").energy,
            expected_crew_energy
        );
        let mut expected_mutated_crew = expected_crew;
        expected_mutated_crew[0].1 = Value::Int(99);
        assert_eq!(
            engine
                .crew_object_info(crew)
                .expect("ready crew retains info")
                .extra_data,
            expected_mutated_crew
        );
        assert_eq!(
            state.crew_info_rosters[&joined]
                .iter()
                .find(|info| info.id == "CRWD")
                .expect("roster entry remains")
                .extra_data,
            expected_mutated_crew
        );
        Ok(())
    }

    #[test]
    fn player_lifecycle_restore_reapplies_autostop_to_inactive_crew() -> Result<(), EngineError> {
        // ApplyForcedControl clears buffered input whenever ControlStyle
        // changes and, when switching to AutoStop, clears ComDir on every
        // inactive owned crew object (C4Player.cpp:2369-2391).
        let mut engine = Engine::new();
        let mut definition = simple_definition("REST");
        definition.set_crew_member(true);
        engine.register_definition(definition)?;
        let crew = engine.spawn_object(
            SpawnConfig::new("REST")
                .with_owner(1)
                .with_status(ObjectStatus::Inactive)
                .with_command_direction(CommandDirection::Right),
        )?;
        engine.register_player(PlayerConfig::new(1, "Saved"))?;
        {
            let player = engine.player_mut(1)?;
            player.control.last_com = i32::from(COM_RIGHT);
            player.control.pressed_coms = 0x3ff;
        }

        engine.reinitialize_player_after_restore(
            1,
            PlayerAtClient::HOST,
            "Local",
            "Current",
            PlayerRuntimeControl::NONE,
            false,
            false,
            true,
            false,
        )?;

        let player = engine.player(1).expect("player remains");
        assert!(player.control.control_style);
        assert_eq!(player.control.last_com, 0);
        assert_eq!(player.control.pressed_coms, 0);
        assert_eq!(
            engine
                .object_snapshot(crew)
                .expect("inactive crew remains")
                .command_direction,
            CommandDirection::Stop
        );
        Ok(())
    }

    #[test]
    fn player_lifecycle_repeated_surrender_does_not_restart_retire_delay(
    ) -> Result<(), EngineError> {
        // C4Player::Surrender returns immediately when already surrendered,
        // so a repeated request cannot restart the 60-frame RetireDelay
        // (C4Player.cpp:971-979; C4Player.cpp:238-243).
        let mut engine = Engine::new();
        engine.register_player(PlayerConfig::new(1, "Surrendering"))?;
        engine.set_player_surrendered(1, true)?;

        for _ in 0..10 {
            engine.tick_player_systems()?;
        }
        engine.set_player_surrendered(1, true)?;
        for _ in 0..49 {
            engine.tick_player_systems()?;
        }

        assert!(engine.player(1).is_some(), "player retires after frame 60");
        engine.tick_player_systems()?;
        assert!(
            engine.player(1).is_none(),
            "repeat surrender must not extend the original retire delay"
        );
        Ok(())
    }

    #[test]
    fn player_lifecycle_fresh_autostop_clears_only_owned_inactive_crew_definitions(
    ) -> Result<(), EngineError> {
        // Fresh InitControl also transitions from the default FreeScroll
        // style. Switching to AutoStop clears ComDir only for owned objects
        // in the inactive list whose definitions are CrewMember
        // (C4Player.cpp:2369-2391).
        let mut join_engine = Engine::new();
        let mut crew_definition = simple_definition("AUTO");
        crew_definition.set_crew_member(true);
        join_engine.register_definition(crew_definition)?;
        join_engine.register_definition(simple_definition("ITEM"))?;

        let owned_inactive_crew = join_engine.spawn_object(
            SpawnConfig::new("AUTO")
                .with_owner(0)
                .with_status(ObjectStatus::Inactive)
                .with_command_direction(CommandDirection::Right),
        )?;
        let owned_active_crew = join_engine.spawn_object(
            SpawnConfig::new("AUTO")
                .with_owner(0)
                .with_command_direction(CommandDirection::Right),
        )?;
        let foreign_inactive_crew = join_engine.spawn_object(
            SpawnConfig::new("AUTO")
                .with_owner(9)
                .with_status(ObjectStatus::Inactive)
                .with_command_direction(CommandDirection::Right),
        )?;
        let owned_inactive_noncrew = join_engine.spawn_object(
            SpawnConfig::new("ITEM")
                .with_owner(0)
                .with_status(ObjectStatus::Inactive)
                .with_command_direction(CommandDirection::Right),
        )?;

        let mut join = lifecycle_join_config("Fresh AutoStop", Vec::new());
        join.control_style = true;
        let player = join_engine.join_player(join)?.number();
        assert!(join_engine.player(player).unwrap().control.control_style);
        assert_eq!(
            join_engine
                .object_snapshot(owned_inactive_crew)
                .unwrap()
                .command_direction,
            CommandDirection::Stop
        );
        for object in [
            owned_active_crew,
            foreign_inactive_crew,
            owned_inactive_noncrew,
        ] {
            assert_eq!(
                join_engine
                    .object_snapshot(object)
                    .unwrap()
                    .command_direction,
                CommandDirection::Right
            );
        }

        let mut register_engine = Engine::new();
        let mut crew_definition = simple_definition("AUTO");
        crew_definition.set_crew_member(true);
        register_engine.register_definition(crew_definition)?;
        let registered_inactive_crew = register_engine.spawn_object(
            SpawnConfig::new("AUTO")
                .with_owner(7)
                .with_status(ObjectStatus::Inactive)
                .with_command_direction(CommandDirection::Left),
        )?;
        register_engine.set_forced_control_style(Some(true));
        register_engine.register_player(PlayerConfig::new(7, "Forced AutoStop"))?;
        assert!(
            register_engine
                .player(7)
                .unwrap()
                .control
                .control_style
        );
        assert_eq!(
            register_engine
                .object_snapshot(registered_inactive_crew)
                .unwrap()
                .command_direction,
            CommandDirection::Stop
        );
        Ok(())
    }

    #[test]
    fn player_lifecycle_review_mouse_fog_initializes_between_player_callbacks(
    ) -> Result<(), EngineError> {
        // InitControl exposes MouseControl before PreInitializePlayer. The
        // automatic, unforced mouse FoW begins later in ScenarioInit but still
        // before InitializePlayer; an explicit SetFoW(false) in pre-init sets
        // bForceFogOfWar and wins (C4Player.cpp:323-348,759-769,815-824).
        let script = r#"#strict 2
static pre_mouse, pre_fog, init_fog;
static forced_pre_fog, forced_init_fog;
global func PreInitializePlayer(int player)
{
    if (player == 0)
    {
        pre_mouse = GetPlayerVal("MouseControl", 0, player);
        pre_fog = GetPlayerVal("FogOfWar", 0, player);
    }
    else
    {
        forced_pre_fog = GetPlayerVal("FogOfWar", 0, player);
        SetFoW(false, player);
    }
    return 1;
}
global func InitializePlayer(int player)
{
    if (player == 0) init_fog = GetPlayerVal("FogOfWar", 0, player);
    else forced_init_fog = GetPlayerVal("FogOfWar", 0, player);
    return 1;
}
"#;
        let mut engine = Engine::with_seed(0);
        engine.install_scenario_script_with_convention("Mouse FoW order", script, true)?;

        let automatic = engine
            .join_player_with_runtime_control(
                lifecycle_join_config("Automatic FoW", Vec::new()),
                PlayerRuntimeControl::new(0, 1),
            )?
            .number();
        let mut forced_config = lifecycle_join_config("Forced-off FoW", Vec::new());
        forced_config.player_info_id = 2;
        let forced = engine
            .join_player_with_runtime_control(forced_config, PlayerRuntimeControl::new(1, 1))?
            .number();

        let globals = &engine.snapshot().script_globals.named;
        assert_eq!(globals.get("pre_mouse"), Some(&Value::Int(1)));
        assert_eq!(globals.get("pre_fog"), Some(&Value::Bool(false)));
        assert_eq!(globals.get("init_fog"), Some(&Value::Bool(true)));
        assert_eq!(globals.get("forced_pre_fog"), Some(&Value::Bool(false)));
        assert_eq!(globals.get("forced_init_fog"), Some(&Value::Bool(false)));

        let automatic = engine.player(automatic).expect("automatic player remains");
        assert!(automatic.fog_of_war());
        assert!(!automatic.force_fog_of_war());
        let forced = engine.player(forced).expect("forced player remains");
        assert!(!forced.fog_of_war());
        assert!(forced.force_fog_of_war());
        Ok(())
    }

    #[test]
    fn player_lifecycle_review_final_init_preserves_or_derives_cursor_in_cpp_order(
    ) -> Result<(), EngineError> {
        // InitializePlayer runs before FinalInit. FinalInit preserves an
        // explicit cursor; only a missing cursor triggers AdjustCursorCommand,
        // which searches selected crew before all crew (C4Player.cpp:769-798,
        // 1235-1258).
        let join = |initialize_body: &str| -> Result<(Engine, i32, ObjectId, ObjectId), EngineError> {
            let mut engine = Engine::with_seed(0);
            for id in ["HIGH", "LOWR"] {
                let mut definition = simple_definition(id);
                definition.set_crew_member(true);
                engine.register_definition(definition)?;
            }
            let mut start = PlayerStart::default();
            start.ready_crew = vec![("HIGH".to_string(), 1), ("LOWR".to_string(), 1)];
            engine.set_player_starts(vec![start]);
            let script = format!(
                "#strict 2\nglobal func InitializePlayer(int player)\n{{\n    {initialize_body}\n    return 1;\n}}\n"
            );
            engine.install_scenario_script_with_convention("Cursor order", &script, true)?;
            let crew_info = |id: &str, name: &str, rank: i32| player_file::CrewInfo {
                id: id.to_string(),
                name: name.to_string(),
                death_message: String::new(),
                core: Default::default(),
                rank,
                rank_name: match rank {
                    5 => "Lieutenant Colonel",
                    1 => "Ensign",
                    _ => "Clonk",
                }
                .to_string(),
                experience: 0,
                rounds: 0,
                physical: PhysicalInfo::default(),
                death_count: 0,
                total_playing_time: 0,
                birthday: 0,
                age: 0,
                participation: 1,
                in_action: false,
                was_in_action: false,
                in_action_time: 0,
                has_died: false,
                extra_data: Vec::new(),
                portraits: Default::default(),
            };
            let player = engine
                .join_player(lifecycle_join_config(
                    "Cursor order",
                    vec![
                        crew_info("HIGH", "High rank", 5),
                        crew_info("LOWR", "Low rank", 1),
                    ],
                ))?
                .number();
            let crew = engine.player(player).expect("player remains").crew();
            let low = crew
                .iter()
                .copied()
                .find(|id| {
                    engine
                        .crew_object_info(*id)
                        .is_some_and(|info| info.definition_id.as_str() == "LOWR")
                })
                .expect("low-rank crew exists");
            let high = crew
                .iter()
                .copied()
                .find(|id| {
                    engine
                        .crew_object_info(*id)
                        .is_some_and(|info| info.definition_id.as_str() == "HIGH")
                })
                .expect("high-rank crew exists");
            Ok((engine, player, low, high))
        };

        let (explicit, player, low, high) = join(
            "SelectCrew(player, FindObject(HIGH), true, true); SelectCrew(player, FindObject(LOWR), true, true); SetCursor(player, FindObject(LOWR), true, true, true);",
        )?;
        assert_ne!(low, high);
        assert_eq!(explicit.selected_crew(player).len(), 2);
        assert_eq!(explicit.crew_cursor(player), Some(low));
        assert_eq!(explicit.player(player).expect("player").cursor(), Some(low));

        let (selected, player, low, high) = join(
            "SelectCrew(player, FindObject(LOWR), true, true);",
        )?;
        assert_ne!(low, high);
        assert_eq!(selected.selected_crew(player), vec![low]);
        assert_eq!(selected.crew_cursor(player), Some(low));
        assert_eq!(selected.player(player).expect("player").cursor(), Some(low));
        Ok(())
    }

    #[test]
    fn player_lifecycle_review_team_homebase_sync_follows_initialize_player(
    ) -> Result<(), EngineError> {
        // ScenarioInit installs the joining player's PlayerStart material,
        // broadcasts InitializePlayer, and only then copies the team leader's
        // homebase state (C4Player.cpp:702-711,769-775,349-350).
        let script = r#"#strict 2
static leader_brick, follower_ore, follower_brick;
global func InitializePlayer(int player)
{
    if (player == 0) leader_brick = GetHomebaseMaterial(player, BRCK);
    else
    {
        follower_ore = GetHomebaseMaterial(player, ORE1);
        follower_brick = GetHomebaseMaterial(player, BRCK);
    }
    return 1;
}
"#;
        let mut engine = Engine::with_seed(0);
        engine.register_definition(simple_definition("BRCK"))?;
        engine.register_definition(simple_definition("ORE1"))?;
        engine.set_team_home_base_rule(true);
        engine.set_teams(vec![TeamInfo::new(1, "Team", 0x00f4_0000)]);
        let mut leader_start = PlayerStart::default();
        leader_start.home_base_material = vec![("BRCK".to_string(), 7)];
        let mut follower_start = PlayerStart::default();
        follower_start.home_base_material = vec![("ORE1".to_string(), 3)];
        engine.set_player_starts(vec![leader_start, follower_start]);
        engine.install_scenario_script_with_convention("Team homebase order", script, true)?;

        let mut leader_config = lifecycle_join_config("Leader", Vec::new());
        leader_config.team = Some(1);
        let leader = engine.join_player(leader_config)?.number();
        let mut follower_config = lifecycle_join_config("Follower", Vec::new());
        follower_config.player_info_id = 2;
        follower_config.team = Some(1);
        let follower = engine.join_player(follower_config)?.number();

        let globals = &engine.snapshot().script_globals.named;
        assert_eq!(globals.get("leader_brick"), Some(&Value::Int(7)));
        assert_eq!(globals.get("follower_ore"), Some(&Value::Int(3)));
        assert_eq!(globals.get("follower_brick"), Some(&Value::Int(0)));
        let leader = engine.player(leader).expect("leader remains");
        let follower = engine.player(follower).expect("follower remains");
        assert_eq!(leader.home_base_material().get("BRCK"), Some(&7));
        assert_eq!(follower.home_base_material().get("BRCK"), Some(&7));
        assert!(!follower.home_base_material().contains_key("ORE1"));
        Ok(())
    }

    #[test]
    fn player_lifecycle_view_delays_arm_only_at_cpp_boundaries_and_decay_in_the_same_execute(
    ) -> Result<(), EngineError> {
        // C4Player::UpdateValue runs on Tick35, then ViewValue is decremented
        // at the end of that same Execute (C4Player.cpp:228-241). DoPoints and
        // DoWealth arm their counters immediately even for a zero delta
        // (C4Player.cpp:905-914,1824-1828; C4Script.cpp:2762-2765).
        let mut engine = Engine::new();
        let mut valuable = simple_definition("VALU");
        valuable.set_value(75);
        engine.register_definition(valuable)?;
        engine.install_scenario_script_with_convention(
            "Score delay",
            "#strict 2\nglobal func AwardScore(int player) { return DoScore(player, 0); }\n",
            true,
        )?;
        engine.register_player(PlayerConfig::new(1, "Valuer"))?;
        engine.spawn_object(SpawnConfig::new("VALU").with_owner(1))?;

        for _ in 0..34 {
            engine.tick_without_snapshot()?;
        }
        assert_eq!(engine.snapshot().frame, 34);
        assert_eq!(
            engine.player(1).expect("player remains").view_value(),
            0,
            "asset changes do not refresh the cached value before Tick35"
        );

        engine.tick_without_snapshot()?;
        assert_eq!(engine.snapshot().frame, 35);
        assert_eq!(
            engine.player(1).expect("player remains").view_value(),
            99,
            "Tick35 arms to 100 before the same Execute decays to 99"
        );

        engine.call_scenario_script_function("AwardScore", vec![Value::Int(1)])?;
        engine.adjust_player_wealth(1, 0)?;
        let player = engine.player(1).expect("player remains");
        assert_eq!(player.view_value(), 100);
        assert_eq!(player.view_wealth(), 100);

        engine.tick_without_snapshot()?;
        let player = engine.player(1).expect("player remains");
        assert_eq!(player.view_value(), 99);
        assert_eq!(player.view_wealth(), 99);
        Ok(())
    }

    #[test]
    fn player_lifecycle_execute_control_keeps_last_com_visible_during_single_callback(
    ) -> Result<(), EngineError> {
        // ExecuteControl dispatches the delayed COM_Single synchronously and
        // only clears LastCom after the callback returns (C4Player.cpp:
        // 1215-1229). Reflection in that callback still sees the plain com.
        let mut engine = Engine::with_seed(0);
        let mut crew_definition = Definition::from_script(
            "LCOM",
            "Last com crew",
            r#"#strict 2
func ControlUpSingle()
{
    SetPlrExtraData(GetOwner(), "seen_last_com", GetPlayerVal("LastCom", 0, GetOwner()));
    return true;
}
"#,
        )?;
        crew_definition.set_crew_member(true);
        engine.register_definition(crew_definition)?;
        engine.register_player(PlayerConfig::new(1, "Buffered input"))?;
        let crew = engine.spawn_object(
            SpawnConfig::new("LCOM")
                .with_owner(1)
                .with_alive(true)
                .with_crew_member(true),
        )?;
        engine.select_crew(1, [crew])?;
        engine.set_crew_cursor(1, Some(crew))?;
        {
            let player = engine.player_mut(1)?;
            player.control.last_com = i32::from(COM_UP);
            player.control.last_com_delay = 100;
        }

        engine.tick_player_systems()?;

        let player = engine.player(1).expect("player remains");
        assert_eq!(player.control.last_com, 0);
        assert_eq!(player.control.last_com_delay, 0);
        assert_eq!(
            player
                .to_state()
                .extra_data
                .iter()
                .find(|(name, _)| name == "seen_last_com")
                .map(|(_, value)| value),
            Some(&Value::Int(i32::from(COM_UP)))
        );
        Ok(())
    }

    #[test]
    fn player_lifecycle_select_count_is_a_saved_cache_refreshed_at_player_execute(
    ) -> Result<(), EngineError> {
        // UpdateCounts is the first Player::Execute step; selection changes do
        // not rewrite SelectCount synchronously (C4Player.cpp:206-210,
        // 1667-1677). CompileFunc saves the cached integer (:1594).
        let mut engine = Engine::with_seed(0);
        let mut crew_definition = simple_definition("SCNT");
        crew_definition.set_crew_member(true);
        engine.register_definition(crew_definition.clone())?;
        engine.register_player(PlayerConfig::new(1, "Selector"))?;
        let first = engine.spawn_object(
            SpawnConfig::new("SCNT")
                .with_owner(1)
                .with_alive(true)
                .with_crew_member(true),
        )?;
        let second = engine.spawn_object(
            SpawnConfig::new("SCNT")
                .with_owner(1)
                .with_alive(true)
                .with_crew_member(true),
        )?;
        engine.select_crew(1, [first, second])?;
        assert_eq!(engine.player(1).expect("player").select_count(), 0);

        engine.tick_player_systems()?;
        assert_eq!(engine.player(1).expect("player").select_count(), 2);
        engine.deselect_crew(1, [first]);
        assert_eq!(
            engine.player(1).expect("player").select_count(),
            2,
            "selection mutation leaves the cache stale until Player::Execute"
        );

        let saved = engine.capture_state();
        let mut restored = Engine::with_seed(1);
        restored.register_definition(crew_definition)?;
        restored.restore_state(&saved)?;
        assert_eq!(restored.selected_crew(1), vec![second]);
        assert_eq!(
            restored.player(1).expect("restored player").select_count(),
            2,
            "snapshot restore preserves the serialized cache"
        );
        restored.finalize_restored_players()?;
        assert_eq!(restored.player(1).expect("player").select_count(), 1);
        Ok(())
    }

    #[test]
    fn player_lifecycle_startup_hint_clears_through_object_com_and_object_command(
    ) -> Result<(), EngineError> {
        // ObjectCom and ObjectCommand clear ShowStartup after the eliminated
        // guard (C4Player.cpp:1368-1404).
        let mut engine = Engine::with_seed(0);
        let mut crew_definition = simple_definition("HINT");
        crew_definition.set_crew_member(true);
        engine.register_definition(crew_definition)?;
        engine.register_player(PlayerConfig::new(1, "Hints"))?;
        let crew = engine.spawn_object(
            SpawnConfig::new("HINT")
                .with_owner(1)
                .with_alive(true)
                .with_crew_member(true),
        )?;
        engine.select_crew(1, [crew])?;
        engine.set_crew_cursor(1, Some(crew))?;
        assert!(engine.player(1).expect("player").show_startup());

        engine.player_in_com(1, COM_UP, 0)?;
        assert!(!engine.player(1).expect("player").show_startup());

        engine.player_mut(1)?.set_show_startup(true);
        engine.player_object_command(1, CommandId::Wait, None, 0, 0)?;
        assert!(!engine.player(1).expect("player").show_startup());
        Ok(())
    }

    #[test]
    fn player_lifecycle_captain_assigns_in_final_init_round_trips_and_clears_with_object(
    ) -> Result<(), EngineError> {
        // FinalInit assigns the highest-rank active crew member only while a
        // KILC exists; ClearPointers clears an exact Captain match
        // (C4Player.cpp:57-82,793-802,1003-1019).
        let mut engine = Engine::with_seed(0);
        let mut kilc_definition = simple_definition("KILC");
        kilc_definition.set_crew_member(true);
        engine.register_definition(kilc_definition.clone())?;
        let kilc = engine.spawn_object(
            SpawnConfig::new("KILC")
                .with_owner(0)
                .with_alive(true)
                .with_crew_member(true),
        )?;
        let joined = engine
            .join_player(lifecycle_join_config("Captain", Vec::new()))?
            .number();
        assert_eq!(engine.player(joined).expect("player").captain(), Some(kilc));

        let saved = engine.capture_state();
        let mut restored = Engine::with_seed(1);
        restored.register_definition(kilc_definition)?;
        restored.restore_state(&saved)?;
        assert_eq!(restored.player(joined).expect("player").captain(), Some(kilc));

        restored.apply_object_update(
            kilc,
            ObjectUpdate::new().with_status(ObjectStatus::Deleted),
        )?;
        assert_eq!(restored.player(joined).expect("player").captain(), None);
        Ok(())
    }

    #[test]
    fn player_lifecycle_crew_created_counts_new_info_but_not_loaded_info_reuse(
    ) -> Result<(), EngineError> {
        // C4ObjectInfoList::Load/Add do not increment iNumCreated; New does so
        // once after Add succeeds (C4ObjectInfoList.cpp:56-90,144-184).
        let setup = |engine: &mut Engine| -> Result<(), EngineError> {
            let mut crew_definition = simple_definition("CRNW");
            crew_definition.set_crew_member(true);
            engine.register_definition(crew_definition)?;
            let mut start = PlayerStart::default();
            start.ready_crew = vec![("CRNW".to_string(), 1)];
            engine.set_player_starts(vec![start]);
            Ok(())
        };

        let mut created = Engine::with_seed(0);
        setup(&mut created)?;
        let created_player = created
            .join_player(lifecycle_join_config("New info", Vec::new()))?
            .number();
        assert_eq!(created.player(created_player).expect("player").crew_created(), 1);
        assert_eq!(created.capture_state().players[0].crew_created, 1);

        let loaded_info = player_file::CrewInfo {
            id: "CRNW".to_string(),
            name: "Existing".to_string(),
            death_message: String::new(),
            core: Default::default(),
            rank: 0,
            rank_name: "Clonk".to_string(),
            experience: 0,
            rounds: 0,
            physical: PhysicalInfo::default(),
            death_count: 0,
            total_playing_time: 0,
            birthday: 0,
            age: 0,
            participation: 1,
            in_action: false,
            was_in_action: false,
            in_action_time: 0,
            has_died: false,
            extra_data: Vec::new(),
            portraits: Default::default(),
        };
        let mut reused = Engine::with_seed(1);
        setup(&mut reused)?;
        let reused_player = reused
            .join_player(lifecycle_join_config("Loaded info", vec![loaded_info]))?
            .number();
        assert_eq!(reused.player(reused_player).expect("player").crew_created(), 0);
        assert_eq!(reused.capture_state().players[0].crew_created, 0);
        Ok(())
    }

    #[test]
    fn shipped_hazard_shuttle_scores_both_driver_owner_transfers_in_order(
    ) -> Result<(), EngineError> {
        // SHTL::DriverIsOwner awards the old owner its value, transfers
        // ownership, then charges the new owner (Hazard Shuttle Script.c:
        // 239-245). Starting near the upper bound makes the two calls
        // observably non-reversible: 99_950 + 150 clamps to 100_000, then
        // subtracting 150 leaves 99_850.
        let content = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content");
        let group = clonk_resources::Group::open(
            content.join("Hazard.c4d/Vehicles.c4d/Shuttle.c4d"),
        )
        .expect("open shipped Shuttle definition");
        let resource = ResourceDefinitionData::load(&group)
            .expect("load shipped Shuttle definition");

        let mut engine = Engine::new();
        engine.register_player(PlayerConfig::new(0, "Owner").with_points(99_950))?;
        engine.register_definition(simple_definition("WEPN"))?;
        engine.register_definition(Definition::from_resource(&resource)?)?;
        engine.register_definition(simple_definition("CLNK"))?;
        engine.resolve_includes()?;

        let shuttle = engine.spawn_object(
            SpawnConfig::new("SHTL")
                .with_owner(0)
                .with_loaded(true),
        )?;
        let driver = engine.spawn_object(SpawnConfig::new("CLNK").with_owner(0))?;
        let shuttle_index = engine
            .find_object_index(shuttle)
            .expect("shuttle exists");
        engine.call_object_function(
            shuttle_index,
            "DriverIsOwner",
            vec![object_reference_value(driver)],
        )?;

        assert_eq!(
            engine.player(0).expect("owner remains").points(),
            99_850
        );
        Ok(())
    }

    #[test]
    fn player_cursor_tracks_selection_changes() -> Result<(), EngineError> {
        let mut engine = Engine::new();
        engine.register_player(PlayerConfig::new(1, "Cursor"))?;
        let mut definition =
            Definition::from_script("CursorCrew", "CursorCrew", PASSIVE_PLAYER_SCRIPT)?;
        definition.set_crew_member(true);
        engine.register_definition(definition)?;
        let crew = engine.spawn_object(
            SpawnConfig::new("CursorCrew")
                .with_owner(1)
                .with_crew_member(true)
                .with_position(Vector2::new(0, 0)),
        )?;
        engine.select_crew(1, [crew])?;
        assert_eq!(engine.player(1).unwrap().cursor(), Some(crew));
        let snapshot = engine.snapshot();
        let cursor = snapshot
            .players
            .iter()
            .find(|state| state.id == 1)
            .and_then(|state| state.cursor);
        assert_eq!(cursor, Some(crew));
        Ok(())
    }

    #[test]
    fn deselect_crew_updates_cursor() {
        let mut engine = Engine::with_seed(0);
        let mut definition = build_definition();
        definition.set_crew_member(true);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let first = engine
            .spawn_object(
                SpawnConfig::new("Test")
                    .with_alive(true)
                    .with_owner(1)
                    .with_crew_member(true),
            )
            .expect("spawn succeeds");
        let second = engine
            .spawn_object(
                SpawnConfig::new("Test")
                    .with_alive(true)
                    .with_owner(1)
                    .with_crew_member(true),
            )
            .expect("spawn succeeds");

        engine
            .select_crew(1, vec![first, second])
            .expect("selection succeeds");

        engine.deselect_crew(1, vec![first]);
        assert_eq!(engine.selected_crew(1), vec![second]);
        assert_eq!(engine.crew_cursor(1), Some(second));

        engine.deselect_crew(1, vec![second]);
        // AdjustCursorCommand never leaves an active crew roster without a
        // selected cursor: if no Select remains it chooses the high-rank
        // active crew and DoSelect()s it (C4Player.cpp:1235-1258).
        assert_eq!(engine.selected_crew(1), vec![first]);
        assert_eq!(engine.crew_cursor(1), Some(first));
    }

    #[test]
    fn set_cursor_is_cursor_only_like_cpp() {
        let mut engine = Engine::with_seed(0);
        let mut definition = build_definition();
        definition.set_crew_member(true);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let first = engine
            .spawn_object(
                SpawnConfig::new("Test")
                    .with_alive(true)
                    .with_owner(1)
                    .with_crew_member(true),
            )
            .expect("spawn succeeds");
        let second = engine
            .spawn_object(
                SpawnConfig::new("Test")
                    .with_alive(true)
                    .with_owner(1)
                    .with_crew_member(true),
            )
            .expect("spawn succeeds");

        engine
            .select_crew(1, vec![first])
            .expect("selection succeeds");

        engine
            .set_crew_cursor(1, Some(second))
            .expect("cursor assignment succeeds");

        // C4Player::SetCursor calls DoSelect(true): cursor callbacks run but
        // the object's Select bit is untouched (C4Player.cpp:1831-1845;
        // C4Object.cpp:5815-5824).
        assert_eq!(engine.selected_crew(1), vec![first]);
        assert_eq!(engine.crew_cursor(1), Some(second));
    }

    #[test]
    fn select_crew_rejects_wrong_owner() {
        let mut engine = Engine::with_seed(0);
        let mut definition = build_definition();
        definition.set_crew_member(true);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let owned = engine
            .spawn_object(
                SpawnConfig::new("Test")
                    .with_alive(true)
                    .with_owner(1)
                    .with_crew_member(true),
            )
            .expect("spawn succeeds");
        let other_owner = engine
            .spawn_object(
                SpawnConfig::new("Test")
                    .with_alive(true)
                    .with_owner(2)
                    .with_crew_member(true),
            )
            .expect("spawn succeeds");

        engine
            .select_crew(1, vec![owned])
            .expect("selection succeeds");

        let error = engine
            .select_crew(1, vec![other_owner])
            .expect_err("selection should fail");
        match error {
            EngineError::CrewSelection { owner, detail } => {
                assert_eq!(owner, 1);
                assert!(detail.contains("owned by"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn selection_pruned_after_object_destroyed() {
        let mut engine = Engine::with_seed(0);
        let mut definition = build_definition();
        definition.set_crew_member(true);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let crew = engine
            .spawn_object(
                SpawnConfig::new("Test")
                    .with_alive(true)
                    .with_owner(1)
                    .with_crew_member(true),
            )
            .expect("spawn succeeds");

        engine
            .select_crew(1, vec![crew])
            .expect("selection succeeds");

        engine
            .queue_object_command(
                crew,
                QueuedCommand::immediate(ObjectUpdate::new()).with_destroy(true),
            )
            .expect("queue succeeds");

        engine.tick_without_snapshot().expect("tick succeeds");

        assert!(engine.selected_crew(1).is_empty());
        assert_eq!(engine.crew_cursor(1), None);
    }

    #[test]
    fn crew_role_assignment_requires_valid_owner() {
        let mut engine = Engine::with_seed(0);
        let mut definition = build_definition();
        definition.set_crew_member(true);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let owned = engine
            .spawn_object(
                SpawnConfig::new("Test")
                    .with_alive(true)
                    .with_owner(1)
                    .with_crew_member(true),
            )
            .expect("spawn succeeds");
        let other_owner = engine
            .spawn_object(
                SpawnConfig::new("Test")
                    .with_alive(true)
                    .with_owner(2)
                    .with_crew_member(true),
            )
            .expect("spawn succeeds");

        engine
            .set_crew_role(1, owned, CrewRole::from("builder"))
            .expect("role assignment succeeds");

        let error = engine
            .set_crew_role(1, other_owner, CrewRole::from("builder"))
            .expect_err("assignment should fail");
        match error {
            EngineError::CrewRole { owner, detail } => {
                assert_eq!(owner, 1);
                assert!(detail.contains("owned"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn crew_roles_removed_when_object_destroyed() {
        let mut engine = Engine::with_seed(0);
        let mut definition = build_definition();
        definition.set_crew_member(true);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let crew = engine
            .spawn_object(
                SpawnConfig::new("Test")
                    .with_alive(true)
                    .with_owner(1)
                    .with_crew_member(true),
            )
            .expect("spawn succeeds");

        engine
            .set_crew_role(1, crew, CrewRole::from("scout"))
            .expect("role assignment succeeds");

        engine
            .queue_object_command(
                crew,
                QueuedCommand::immediate(ObjectUpdate::new()).with_destroy(true),
            )
            .expect("queue succeeds");

        engine.tick_without_snapshot().expect("tick succeeds");

        assert!(engine.crew_role_assignments(1).is_empty());
    }

    #[test]
    fn apply_command_targets_role_assignments() {
        let mut engine = Engine::with_seed(0);
        let mut definition = build_definition();
        definition.set_crew_member(true);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let first = engine
            .spawn_object(
                SpawnConfig::new("Test")
                    .with_alive(true)
                    .with_owner(1)
                    .with_crew_member(true),
            )
            .expect("spawn succeeds");
        let second = engine
            .spawn_object(
                SpawnConfig::new("Test")
                    .with_alive(true)
                    .with_owner(1)
                    .with_crew_member(true),
            )
            .expect("spawn succeeds");

        engine
            .set_crew_role(1, first, CrewRole::from("builder"))
            .expect("role assignment succeeds");
        engine
            .set_crew_role(1, second, CrewRole::from("builder"))
            .expect("role assignment succeeds");

        engine
            .apply_command(
                1,
                CrewCommandTarget::role("builder"),
                ObjectUpdate::new().with_energy(42),
            )
            .expect("command routes");

        assert_eq!(engine.object_snapshot(first).unwrap().energy, 42);
        assert_eq!(engine.object_snapshot(second).unwrap().energy, 42);
    }

    #[test]
    fn apply_command_uses_engine_order_for_selection() {
        let script = r#"
        global func Initialize(state, random) { return 0; }
        global func Step(state, frame, random) { return 0; }
        global func OnIdleAbort(state, action) { return 0; }
        global func OnWalkStart(state, action) { return 0; }
        "#;

        let call_log: Arc<Mutex<Vec<(String, i32)>>> = Arc::new(Mutex::new(Vec::new()));
        let mut hooks = DebuggerHooks::new();
        {
            let call_log = Arc::clone(&call_log);
            hooks.set_on_call(move |name, args| {
                if name == "OnIdleAbort" || name == "OnWalkStart" {
                    if let Some(Value::Proplist(state)) = args.first() {
                        if let Some(Value::Int(id)) = state.get("id") {
                            call_log.lock().unwrap().push((name.to_string(), *id));
                        }
                    }
                }
            });
        }

        let mut definition =
            Definition::from_script("Crew", "Crew", script).expect("script compiles");
        definition.set_debugger_hooks(hooks);
        definition.set_crew_member(true);
        let mut actions = HashMap::new();
        actions.insert(
            "Idle".to_string(),
            ActionSpec::default().with_abort_call("OnIdleAbort"),
        );
        actions.insert(
            "Walk".to_string(),
            ActionSpec::default().with_start_call("OnWalkStart"),
        );
        definition.configure_actions(Some("Idle".to_string()), actions);

        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let first = engine
            .spawn_object(
                SpawnConfig::new("Crew")
                    .with_alive(true)
                    .with_owner(1)
                    .with_crew_member(true)
                    .with_id(ObjectId::new(200)),
            )
            .expect("first spawn succeeds");
        let second = engine
            .spawn_object(
                SpawnConfig::new("Crew")
                    .with_alive(true)
                    .with_owner(1)
                    .with_crew_member(true)
                    .with_id(ObjectId::new(100)),
            )
            .expect("second spawn succeeds");

        engine
            .select_crew(1, vec![first, second])
            .expect("selection succeeds");

        engine
            .apply_command(
                1,
                CrewCommandTarget::selection(),
                ObjectUpdate::new().with_action("Walk"),
            )
            .expect("command applies");

        let log = call_log.lock().unwrap().clone();
        // C4Object::SetAction executes StartCall before AbortCall
        // (C4Object.cpp:4172-4208), preserving crew-selection order outside
        // each object's callback pair.
        let expected = vec![
            ("OnWalkStart".to_string(), 200),
            ("OnIdleAbort".to_string(), 200),
            ("OnWalkStart".to_string(), 100),
            ("OnIdleAbort".to_string(), 100),
        ];
        assert_eq!(log, expected);
    }

    #[test]
    fn capture_state_preserves_crew_roles() {
        let mut engine = Engine::with_seed(0);
        let mut definition = build_definition();
        definition.set_crew_member(true);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let crew = engine
            .spawn_object(
                SpawnConfig::new("Test")
                    .with_alive(true)
                    .with_owner(1)
                    .with_crew_member(true),
            )
            .expect("spawn succeeds");

        engine
            .set_crew_role(1, crew, CrewRole::from("pilot"))
            .expect("role assignment succeeds");

        let state = engine.capture_state();

        let mut restored = Engine::with_seed(0);
        let mut restored_definition = build_definition();
        restored_definition.set_crew_member(true);
        restored
            .register_definition(restored_definition)
            .expect("definition registers");
        restored.restore_state(&state).expect("state restores");

        let assignments = restored.crew_role_assignments(1);
        assert_eq!(
            assignments.get(&crew).map(|role| role.as_str()),
            Some("pilot")
        );
    }

    #[test]
    fn round_results_state_defaults_and_round_trips_without_evaluation() {
        // C4RoundResultsPlayer defaults both settlement scores to -1 and the
        // remaining game data to zero (C4RoundResults.h:63-69). The round
        // container starts with no goals/players and zero time
        // (C4RoundResults.cpp:249-259).
        let mut engine = Engine::new();
        assert_eq!(engine.round_results, RoundResultsState::default());

        let encoded_default = serde_json::to_value(engine.capture_state())
            .unwrap_or_else(|error| panic!("default state serializes: {error}"));
        assert!(encoded_default.get("round_results").is_none());
        let encoded_snapshot = serde_json::to_value(engine.snapshot())
            .unwrap_or_else(|error| panic!("default snapshot serializes: {error}"));
        assert!(encoded_snapshot.get("round_results").is_none());

        let expected = RoundResultsState {
            goals: vec!["GOLD".to_string(), "WIPF".to_string()],
            fulfilled_goals: vec!["GOLD".to_string()],
            playing_time_seconds: 731,
            hide_settlement_score: true,
            league_performance: -37,
            custom_evaluation_strings: "First line|Second line".to_string(),
            players: vec![RoundResultsPlayerState {
                player_info_id: 41,
                total_playing_time: 1_234,
                score_old: 150,
                score_new: Some(250),
                league_progress_data: None,
                league_performance: 0,
                custom_evaluation_strings: "First note   Second note".to_string(),
                ..RoundResultsPlayerState::default()
            }],
            ..RoundResultsState::default()
        };
        engine.round_results = expected.clone();

        let snapshot = engine.snapshot();
        assert_eq!(snapshot.round_results, expected);
        let state = EngineState::from_snapshot(&snapshot);
        assert_eq!(state.round_results, expected);

        let json = state
            .to_json_string()
            .unwrap_or_else(|error| panic!("round results serialize: {error}"));
        let decoded = EngineState::from_json_str(&json)
            .unwrap_or_else(|error| panic!("round results deserialize: {error}"));
        assert_eq!(decoded.round_results, expected);

        let mut restored = Engine::new();
        restored
            .restore_state(&decoded)
            .unwrap_or_else(|error| panic!("round results restore: {error}"));
        assert_eq!(restored.snapshot().round_results, expected);
    }

    #[test]
    fn shipped_hazard_teams_do_evaluation_records_both_player_lines() {
        // Execute Hazard's actual TEAM::DoEvaluation body while bypassing
        // its scoreboard-heavy Initialize. Player number and player-info id
        // deliberately match here: their distinction is covered by CLO-221,
        // while this regression isolates AddEvaluationData and its ordered
        // accumulation of the two shipped calls.
        let content = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../content");
        let group = clonk_resources::Group::open(
            content.join("Hazard.c4d/Goals.c4d/Teams.c4d"),
        )
        .expect("open shipped Hazard Teams definition");
        let resource = ResourceDefinitionData::load(&group)
            .expect("load shipped Hazard Teams definition");

        let mut engine = Engine::with_seed(0);
        engine
            .register_player(
                PlayerConfig::new(1, "Hazard evaluator").with_player_info_id(1),
            )
            .expect("Hazard evaluator registers");
        engine
            .register_definition(
                Definition::from_resource(&resource)
                    .expect("shipped Hazard Teams script compiles"),
            )
            .expect("shipped Hazard Teams definition registers");

        let teams = engine
            .spawn_object(
                SpawnConfig::new("TEAM")
                    .with_loaded(true)
                    .with_local_vars(HashMap::from([
                        (
                            "aKill".to_string(),
                            Value::Array(vec![Value::Nil, Value::Int(12)]),
                        ),
                        (
                            "aDeath".to_string(),
                            Value::Array(vec![Value::Nil, Value::Int(3)]),
                        ),
                    ])),
            )
            .expect("loaded Hazard Teams goal spawns without Initialize");
        let teams_index = engine
            .find_object_index(teams)
            .expect("Hazard Teams goal exists");
        engine
            .call_object_function(teams_index, "DoEvaluation", vec![Value::Int(1)])
            .expect("shipped Hazard Teams DoEvaluation completes");

        let result = engine
            .round_results
            .players
            .iter()
            .find(|result| result.player_info_id == 1)
            .expect("Hazard evaluator has a round-results row");
        assert_eq!(
            result.custom_evaluation_strings,
            "{{PIWP}}$Kills$: 12   {{KAMB}}$Death$: 3"
        );
    }

    #[test]
    fn shipped_hazard_chooser_selects_the_lowest_client_id() {
        // CHOS::ChoosePlayer identifies the host through the shipped
        // GetPlrClientNr wrapper. Join the remote player first so a nil or
        // player-number reflection cannot accidentally produce the C++ pick:
        // client 0 must beat client 7 even though its player number is 1.
        let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..");
        let system_group = clonk_resources::Group::open(repository.join("planet/System.c4g"))
            .expect("open shipped planet System.c4g");
        let get_x_val = system_group
            .read_file("GetXVal.c")
            .expect("read shipped GetXVal wrappers");
        let get_x_val = String::from_utf8_lossy(&get_x_val).into_owned();
        let chooser_group = clonk_resources::Group::open(
            repository.join("content/Hazard.c4d/Rules.c4d/Chooser.c4d"),
        )
        .expect("open shipped Hazard Chooser definition");
        let chooser_resource = ResourceDefinitionData::load(&chooser_group)
            .expect("load shipped Hazard Chooser definition");

        let mut engine = Engine::with_seed(0);
        engine.set_network_game(true);
        engine.set_landscape(Landscape::flat(64, 64));
        assert_eq!(
            engine.install_global_scripts(&[(
                "planet/System.c4g/GetXVal.c".to_string(),
                get_x_val,
            )]),
            1,
            "shipped GetXVal wrappers install"
        );
        engine
            .register_definition(
                Definition::from_resource(&chooser_resource)
                    .expect("shipped Hazard Chooser script compiles"),
            )
            .expect("shipped Hazard Chooser definition registers");

        let player_config = |name: &str, player_info_id: i32, pref_color: i32| {
            JoinPlayerConfig {
                name: name.to_string(),
                player_info_id,
                score: 0,
                rounds: 0,
                rounds_won: 0,
                rounds_lost: 0,
                total_playing_time: 0,
                team: None,
                color_dw: if pref_color == 0 { 0xff0000 } else { 0x0000ff },
                pref_color,
                pref_position: 0,
                crew: Vec::new(),
                control_style: false,
                auto_context_menu: false,
                startup_player_count: 2,
            }
        };
        let info = ControlPlayerInfoEntry::default();
        let remote = engine
            .join_player_at_client_with_info_and_name(
                player_config("Remote player", 1, 0),
                PlayerAtClient::new(7),
                "Remote Client",
                &info,
            )
            .expect("remote player joins first");
        let host = engine
            .join_player_at_client_with_info_and_name(
                player_config("Host player", 2, 1),
                PlayerAtClient::HOST,
                "Host Client",
                &info,
            )
            .expect("host player joins second");
        assert_eq!((remote.number(), host.number()), (0, 1));
        assert_eq!(
            engine.player(0).map(Player::at_client_name),
            Some("Remote Client")
        );
        assert_eq!(engine.player(0).map(Player::color_index), Some(0));
        assert_eq!(engine.player(1).map(Player::at_client_name), Some("Host Client"));
        assert_eq!(engine.player(1).map(Player::color_index), Some(1));

        let chooser = engine
            .spawn_object(
                SpawnConfig::new("CHOS")
                    .with_loaded(true)
                    .with_local_vars(HashMap::from([(
                        "iChoosingPlr".to_string(),
                        Value::Int(-1),
                    )])),
            )
            .expect("loaded Hazard Chooser spawns without Initialize");
        let chooser_index = engine
            .find_object_index(chooser)
            .expect("Hazard Chooser exists");
        assert_eq!(
            engine
                .call_object_function(chooser_index, "ChoosePlayer", Vec::new())
                .expect("shipped Hazard ChoosePlayer completes"),
            Value::Int(1)
        );
        assert_eq!(
            engine.objects[chooser_index]
                .state
                .local_vars
                .get("iChoosingPlr"),
            Some(&Value::Int(1))
        );
    }

    #[test]
    fn legacy_state_without_round_results_restores_cpp_defaults() {
        let state = Engine::new().capture_state();
        let mut value = serde_json::to_value(state)
            .unwrap_or_else(|error| panic!("state serializes: {error}"));
        value
            .as_object_mut()
            .unwrap_or_else(|| panic!("engine state is an object"))
            .remove("round_results");

        let decoded: EngineState = serde_json::from_value(value)
            .unwrap_or_else(|error| panic!("legacy state deserializes: {error}"));
        assert_eq!(decoded.round_results, RoundResultsState::default());
    }

    #[test]
    fn game_clock_and_player_id_counter_round_trip_with_restore_baselines() {
        // C4Game defaults Time/TimeGo to zero/false and persists Time
        // separately from FrameCounter (C4Game.cpp:1762-1779,1939-1955).
        // C4Player::GameJoinTime is Local-NoSave and is re-established from
        // Game.Time after load (C4Player.cpp:389-390,1556-1567).
        // LastPlayerID is persisted and repaired against existing player IDs
        // (C4PlayerInfo.cpp:1733-1742,1785-1794); removed players can remain
        // represented only in RoundResults (C4PlayerList.cpp:231-242).
        let mut engine = Engine::new();
        assert_eq!(engine.game_time(), 0);
        assert!(!engine.time_go);
        assert_eq!(engine.last_player_info_id, 0);

        let default_state = serde_json::to_value(engine.capture_state())
            .unwrap_or_else(|error| panic!("default state serializes: {error}"));
        assert!(default_state.get("game_time").is_none());
        assert!(default_state.get("last_player_info_id").is_none());
        let legacy: EngineState = serde_json::from_value(default_state)
            .unwrap_or_else(|error| panic!("legacy state deserializes: {error}"));
        assert_eq!(legacy.game_time, 0);
        assert_eq!(legacy.last_player_info_id, 0);
        let default_snapshot = serde_json::to_value(engine.snapshot())
            .unwrap_or_else(|error| panic!("default snapshot serializes: {error}"));
        assert!(default_snapshot.get("game_time").is_none());

        engine.game_time = 731;
        engine.time_go = true;
        engine.last_player_info_id = 61;
        engine.round_results.players.push(RoundResultsPlayerState {
            player_info_id: 57,
            ..RoundResultsPlayerState::default()
        });
        engine.players.insert(
            2,
            PlayerConfig::new(2, "Profile")
                .with_player_info_id(41)
                .with_score(250)
                .with_total_playing_time(1_234)
                .build(),
        );
        assert_eq!(engine.player(2).expect("player").game_join_time(), 0);

        let snapshot = engine.snapshot();
        assert_eq!(snapshot.game_time, 731);
        let from_snapshot = EngineState::from_snapshot(&snapshot);
        assert_eq!(from_snapshot.game_time, 731);
        assert_eq!(from_snapshot.last_player_info_id, 57);

        let encoded = engine
            .capture_state()
            .to_json_string()
            .unwrap_or_else(|error| panic!("state serializes: {error}"));
        assert!(!encoded.contains("game_join_time"));
        assert!(!encoded.contains("time_go"));
        let decoded = EngineState::from_json_str(&encoded)
            .unwrap_or_else(|error| panic!("state deserializes: {error}"));
        assert_eq!(decoded.game_time, 731);
        assert_eq!(decoded.last_player_info_id, 61);

        let mut restored = Engine::new();
        restored
            .restore_state(&decoded)
            .unwrap_or_else(|error| panic!("state restores: {error}"));
        assert_eq!(restored.game_time(), 731);
        assert!(!restored.time_go);
        assert_eq!(restored.last_player_info_id, 61);
        let player = restored.player(2).expect("restored player");
        assert_eq!(player.game_join_time(), 731);
        assert_eq!(player.score(), 250);
        assert_eq!(
            player.total_playing_time(),
            1_965,
            "save projects the 731-second current stint"
        );

        let mut stale_counter = decoded;
        stale_counter.last_player_info_id = 17;
        let mut repaired = Engine::new();
        repaired
            .restore_state(&stale_counter)
            .unwrap_or_else(|error| panic!("stale counter state restores: {error}"));
        assert_eq!(repaired.last_player_info_id, 57);
    }

    #[test]
    fn join_config_propagates_existing_player_info_and_profile_values() {
        // ID allocation belongs to C4PlayerInfoList::AssignPlayerIDs
        // (C4PlayerInfo.cpp:781-799). This structural seam only carries an
        // already-assigned ID and profile core into C4Player::Init.
        let mut engine = Engine::new();
        let joined = engine
            .join_player(JoinPlayerConfig {
                name: "Profile".to_string(),
                player_info_id: 41,
                score: 250,
                rounds: 11,
                rounds_won: 7,
                rounds_lost: 4,
                total_playing_time: 1_234,
                team: None,
                color_dw: 0xff0000,
                pref_color: 0,
                pref_position: 0,
                crew: Vec::new(),
                control_style: false,
                auto_context_menu: false,
                startup_player_count: 1,
            })
            .unwrap_or_else(|error| panic!("player joins: {error}"));

        let player = engine.player(joined.number()).expect("joined player");
        assert_eq!(player.player_info_id(), 41);
        assert_eq!(player.score(), 250);
        assert_eq!(
            (player.rounds(), player.rounds_won(), player.rounds_lost()),
            (11, 7, 4)
        );
        assert_eq!(player.total_playing_time(), 1_234);
        assert_eq!(
            player.game_join_time(),
            0,
            "join baseline uses current game time"
        );
        assert_eq!(engine.last_player_info_id, 41);
    }

    #[test]
    fn player_id_hosts_distinguish_player_info_ids_from_numbers() -> Result<(), EngineError> {
        const PROBE: &str = r#"#strict
public func ReadIDs(int first, int second)
{
    return [
        GetPlayerID(first),
        GetPlayerID(second),
        GetPlayerID(99),
        GetPlayerVal("ID", 0, first),
        GetPlayerVal("Index", 0, first),
        GetPlayerVal("ID", 0, second),
        GetPlayerVal("Index", 0, second)
    ];
}
"#;
        let config = |name: &str, player_info_id| JoinPlayerConfig {
            name: name.to_string(),
            player_info_id,
            score: 0,
            rounds: 0,
            rounds_won: 0,
            rounds_lost: 0,
            total_playing_time: 0,
            team: None,
            color_dw: 0x00ff_0000,
            pref_color: 0,
            pref_position: 0,
            crew: Vec::new(),
            control_style: false,
            auto_context_menu: false,
            startup_player_count: 2,
        };

        let mut engine = Engine::new();
        engine.set_landscape(Landscape::flat(64, 48));
        let first = engine.join_player(config("First", 41))?.number();
        let second = engine.join_player(config("Second", 99))?.number();
        assert_eq!((first, second), (0, 1));

        engine.register_definition(Definition::from_script("PROB", "Probe", PROBE)?)?;
        let probe = engine.spawn_object(SpawnConfig::new("PROB").with_loaded(true))?;
        let probe_index = engine.find_object_index(probe).expect("probe exists");
        assert_eq!(
            engine.call_object_function(
                probe_index,
                "ReadIDs",
                vec![Value::Int(first), Value::Int(second)],
            )?,
            Value::Array(vec![
                Value::Int(41),
                Value::Int(99),
                Value::Nil,
                Value::Int(41),
                Value::Int(0),
                Value::Int(99),
                Value::Int(1),
            ])
        );
        Ok(())
    }

    #[test]
    fn game_ticks_latch_exactly_one_second_increment() {
        // C4Game::Ticks only sets TimeGo; the external Sec1Timer consumes
        // that bool and increments Time once (C4Game.cpp:1755-1759,
        // 1899-1913). Multiple frames coalesce, as do timer pulses without
        // another executed frame.
        let mut engine = Engine::new();
        engine.sec1_timer();
        assert_eq!(engine.game_time(), 0);

        engine.tick_without_snapshot().expect("first tick");
        engine.tick_without_snapshot().expect("second tick");
        assert_eq!(engine.game_time(), 0, "frames are not seconds");
        engine.sec1_timer();
        assert_eq!(engine.game_time(), 1);
        engine.sec1_timer();
        assert_eq!(engine.game_time(), 1, "latch was already consumed");

        engine.tick_without_snapshot().expect("third tick");
        engine.sec1_timer();
        assert_eq!(engine.game_time(), 2);
    }

    #[test]
    fn register_and_join_allocate_player_info_ids_and_anchor_game_time() {
        // C4PlayerInfoList::AssignPlayerIDs allocates only zero IDs from the
        // monotonically increasing counter (C4PlayerInfo.cpp:781-799), and
        // C4Player::Init anchors GameJoinTime to current Game.Time after the
        // profile loads (C4Player.cpp:246-390).
        let mut registered = Engine::new();
        registered.game_time = 37;
        registered
            .register_player(PlayerConfig::new(7, "First"))
            .expect("first player registers");
        assert_eq!(registered.player(7).expect("first").player_info_id(), 1);
        assert_eq!(registered.player(7).expect("first").game_join_time(), 37);

        registered
            .register_player(PlayerConfig::new(8, "Explicit").with_player_info_id(9))
            .expect("explicit player registers");
        registered
            .register_player(PlayerConfig::new(9, "Next"))
            .expect("next player registers");
        assert_eq!(registered.player(8).expect("explicit").player_info_id(), 9);
        assert_eq!(registered.player(9).expect("next").player_info_id(), 10);
        assert_eq!(registered.last_player_info_id, 10);

        let config = |name: &str, player_info_id| JoinPlayerConfig {
            name: name.to_string(),
            player_info_id,
            score: 0,
            rounds: 0,
            rounds_won: 0,
            rounds_lost: 0,
            total_playing_time: 0,
            team: None,
            color_dw: 0xff0000,
            pref_color: 0,
            pref_position: 0,
            crew: Vec::new(),
            control_style: false,
            auto_context_menu: false,
            startup_player_count: 1,
        };
        let mut joined = Engine::new();
        joined.game_time = 55;
        let first = joined.join_player(config("First", 0)).expect("first joins");
        let explicit = joined
            .join_player(config("Explicit", 12))
            .expect("explicit joins");
        let next = joined.join_player(config("Next", 0)).expect("next joins");
        assert_eq!(
            joined.player(first.number()).expect("first").player_info_id(),
            1
        );
        assert_eq!(
            joined.player(first.number()).expect("first").game_join_time(),
            55
        );
        assert_eq!(
            joined
                .player(explicit.number())
                .expect("explicit")
                .player_info_id(),
            12
        );
        assert_eq!(
            joined.player(next.number()).expect("next").player_info_id(),
            13
        );
        assert_eq!(joined.last_player_info_id, 13);
    }

    #[test]
    fn capture_projects_current_stint_without_mutating_live_player() {
        // C4Player::LocalSync/Evaluate add Game.Time-GameJoinTime exactly
        // once (C4Player.cpp:930-968,2080-2096). capture_state has `&self`,
        // so project that delta into non-evaluated PlayerState while leaving
        // the live baseline untouched; evaluated totals are already final.
        let mut engine = Engine::new();
        engine.game_time = 15;
        let mut active = PlayerConfig::new(1, "Active")
            .with_player_info_id(1)
            .with_total_playing_time(100)
            .build();
        active.set_game_join_time(7);
        engine.players.insert(1, active);

        let mut evaluated = Player::from_state(PlayerState {
            id: 2,
            player_info_id: 2,
            evaluated: true,
            total_playing_time: 200,
            ..PlayerState::default()
        });
        evaluated.set_game_join_time(7);
        engine.players.insert(2, evaluated);

        for _ in 0..2 {
            let state = engine.capture_state();
            let active = state
                .players
                .iter()
                .find(|player| player.id == 1)
                .expect("active state");
            let evaluated = state
                .players
                .iter()
                .find(|player| player.id == 2)
                .expect("evaluated state");
            assert_eq!(active.total_playing_time, 108);
            assert_eq!(evaluated.total_playing_time, 200);
        }
        let live = engine.player(1).expect("live active player");
        assert_eq!(live.total_playing_time(), 100);
        assert_eq!(live.game_join_time(), 7);

        let state = engine.capture_state();
        let mut restored = Engine::new();
        restored.restore_state(&state).expect("state restores");
        let restored_player = restored.player(1).expect("restored player");
        assert_eq!(restored_player.total_playing_time(), 108);
        assert_eq!(restored_player.game_join_time(), 15);
    }

    #[test]
    fn engine_state_from_snapshot_allows_resuming_simulation() {
        let mut engine = Engine::with_seed(42);
        engine
            .register_definition(build_definition())
            .expect("definition registers");

        engine
            .spawn_object(
                SpawnConfig::new("Test")
                    .with_position(Vector2::new(5, -3))
                    .with_velocity(Vector2::new(2, -1))
                    .with_energy(75),
            )
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("first tick succeeds");
        let expected_next = engine.tick().expect("second tick succeeds");

        let state = EngineState::from_snapshot(&snapshot);

        let mut restored = Engine::with_seed(1234);
        restored
            .register_definition(build_definition())
            .expect("definition registers");
        restored.restore_state(&state).expect("state restores");

        let mut resumed = restored.tick().expect("tick after restore succeeds");
        assert_eq!(
            resumed.audio,
            vec![
                AudioCommand::SetMusicPlaylist {
                    playlist: None,
                    restart: false,
                },
                AudioCommand::SetMusicLevel { level: 100 },
            ]
        );
        resumed.audio.clear();
        assert_eq!(resumed, expected_next);
    }

    #[test]
    fn restore_snapshot_wrapper_matches_state_restore() {
        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(build_definition())
            .expect("definition registers");

        engine
            .spawn_object(SpawnConfig::new("Test").with_velocity(Vector2::new(1, 0)))
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("first tick succeeds");
        let expected_next = engine.tick().expect("second tick succeeds");

        let mut restored = Engine::with_seed(0);
        restored
            .register_definition(build_definition())
            .expect("definition registers");
        restored
            .restore_snapshot(&snapshot)
            .expect("snapshot restores");

        let mut resumed = restored.tick().expect("tick after restore succeeds");
        assert_eq!(
            resumed.audio,
            vec![
                AudioCommand::SetMusicPlaylist {
                    playlist: None,
                    restart: false,
                },
                AudioCommand::SetMusicLevel { level: 100 },
            ]
        );
        resumed.audio.clear();
        assert_eq!(resumed, expected_next);
    }

    #[test]
    fn snapshot_round_trip_preserves_sub_pixel_velocity() {
        // Sub-pixel velocity (raw 16.16 fractions below one whole pixel) must
        // survive a snapshot save/restore. C++ persists both the integer mirror
        // and the fixed value (`C4Object.cpp:2742`); the integer-only path would
        // round the velocity to whole pixels (fixtoi) and lose the fraction.
        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(build_definition())
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("Test").with_position(Vector2::new(5, 5)))
            .expect("spawn succeeds");
        let idx = engine.find_object_index(id).expect("object exists");
        // x: pure sub-pixel (rounds to 0 px); y: 1 px + sub-pixel fraction.
        engine.objects[idx].set_fixed_velocity(FixedVec2::new(
            C4Fixed::from_raw(300),
            C4Fixed::from_raw(70000),
        ));
        // dir writes mobilize (FnSetXDir/FnSetYDir, C4Script.cpp:705,732)
        engine.objects[idx].state.mobile = true;

        let snapshot = engine.snapshot();

        let mut restored = Engine::with_seed(0);
        restored
            .register_definition(build_definition())
            .expect("definition registers");
        restored
            .restore_snapshot(&snapshot)
            .expect("snapshot restores");

        let ridx = restored
            .find_object_index(id)
            .expect("restored object exists");
        assert_eq!(restored.objects[ridx].fixed_velocity.x.val(), 300);
        assert_eq!(restored.objects[ridx].fixed_velocity.y.val(), 70000);
    }

    #[test]
    fn json_save_load_preserves_sub_pixel_velocity() {
        // The save-game path serializes through JSON; sub-pixel velocity must
        // survive serialize -> deserialize -> restore so a reloaded game stays
        // in lockstep with one that ran continuously.
        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(build_definition())
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("Test").with_position(Vector2::new(5, 5)))
            .expect("spawn succeeds");
        let idx = engine.find_object_index(id).expect("object exists");
        engine.objects[idx].set_fixed_velocity(FixedVec2::new(
            C4Fixed::from_raw(300),
            C4Fixed::from_raw(70000),
        ));
        // dir writes mobilize (FnSetXDir/FnSetYDir, C4Script.cpp:705,732)
        engine.objects[idx].state.mobile = true;

        let json = engine
            .capture_state()
            .to_json_string()
            .expect("state serializes");

        let mut restored = Engine::with_seed(0);
        restored
            .register_definition(build_definition())
            .expect("definition registers");
        let state = EngineState::from_json_str(&json).expect("state deserializes");
        restored.restore_state(&state).expect("state restores");

        let ridx = restored
            .find_object_index(id)
            .expect("restored object exists");
        assert_eq!(restored.objects[ridx].fixed_velocity.x.val(), 300);
        assert_eq!(restored.objects[ridx].fixed_velocity.y.val(), 70000);
    }

    #[test]
    fn snapshot_round_trip_preserves_raw_signed_and_fractional_rotation() {
        // C++ saves Rotation and FixR verbatim (C4Object.cpp:2769,2789).
        // DoMovement keeps a left lean as a negative angle, and stopping
        // rdir does not discard a remaining sub-degree fix_r fraction.
        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(build_definition())
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("Test").with_position(Vector2::new(5, 5)))
            .expect("spawn succeeds");
        let idx = engine.find_object_index(id).expect("object exists");
        engine.objects[idx].state.rotation = -9;
        let independent_fix_r = C4Fixed::from_raw(itofix(-10).val() + 300);
        assert_ne!(
            fixtoi(independent_fix_r),
            -9,
            "test must detect deriving r from fix_r"
        );
        engine.objects[idx].fixed_rotation = independent_fix_r;
        engine.objects[idx].rotation_velocity = C4Fixed::ZERO;

        let snapshot = engine.snapshot();
        let saved = snapshot.object(id).expect("snapshot object exists");
        assert_eq!(saved.rotation, -9, "raw signed r is not normalized");
        assert_eq!(
            saved
                .fixed_rotation
                .expect("fractional fix_r is retained")
                .val(),
            independent_fix_r.val()
        );
        assert_eq!(saved.rotation_velocity, None);

        let mut restored = Engine::with_seed(0);
        restored
            .register_definition(build_definition())
            .expect("definition registers");
        restored
            .restore_snapshot(&snapshot)
            .expect("snapshot restores");
        let ridx = restored.find_object_index(id).expect("object restores");
        assert_eq!(restored.objects[ridx].state.rotation, -9);
        assert_eq!(
            restored.objects[ridx].fixed_rotation.val(),
            independent_fix_r.val()
        );
    }

    #[test]
    fn snapshot_round_trip_preserves_rotation_velocity() {
        // A spinning object's angular velocity (rdir) and rotation accumulator
        // (fix_r) must survive save/restore so a reloaded game keeps turning in
        // lockstep — mirroring C++ persisting rdir/fix_r.
        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(build_definition())
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("Test").with_position(Vector2::new(5, 5)))
            .expect("spawn succeeds");
        let idx = engine.find_object_index(id).expect("object exists");
        // 1.0 deg/frame angular velocity, mid-rotation with a sub-degree fix_r.
        engine.objects[idx].rotation_velocity = itofix(1);
        // SetRDir mobilizes (C4Script.cpp:718)
        engine.objects[idx].state.mobile = true;
        engine.objects[idx].fixed_rotation = C4Fixed::from_raw(327680 + 300);

        let json = engine
            .capture_state()
            .to_json_string()
            .expect("state serializes");

        let mut restored = Engine::with_seed(0);
        restored
            .register_definition(build_definition())
            .expect("definition registers");
        let state = EngineState::from_json_str(&json).expect("state deserializes");
        restored.restore_state(&state).expect("state restores");

        let ridx = restored
            .find_object_index(id)
            .expect("restored object exists");
        assert_eq!(
            restored.objects[ridx].rotation_velocity.val(),
            itofix(1).val()
        );
        assert_eq!(restored.objects[ridx].fixed_rotation.val(), 327680 + 300);
    }

    #[test]
    fn crew_elimination_marks_owner_after_last_crew_destroyed() {
        let mut engine = Engine::with_seed(0);
        let mut definition = build_definition();
        definition.set_crew_member(true);
        definition.set_category(CATEGORY_LIVING);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let owner_one = engine
            .spawn_object(
                SpawnConfig::new("Test")
                    .with_alive(true)
                    .with_owner(1)
                    .with_crew_member(true),
            )
            .expect("spawn succeeds");
        engine
            .spawn_object(
                SpawnConfig::new("Test")
                    .with_alive(true)
                    .with_owner(2)
                    .with_crew_member(true),
            )
            .expect("spawn succeeds");

        assert!(engine.eliminated_owners().is_empty());

        engine
            .queue_object_command(
                owner_one,
                QueuedCommand::immediate(ObjectUpdate::new()).with_destroy(true),
            )
            .expect("queue succeeds");

        // C4Player::CheckElimination runs on the Tick35 boundary only
        // (C4Player.cpp:225-235): the crewless owner survives frames 1-34
        // (the C++ recruit-in-the-window grace) and eliminates at 35.
        engine.tick_without_snapshot().expect("tick succeeds");
        assert!(
            !engine.is_owner_eliminated(1),
            "no elimination before the Tick35 boundary"
        );
        for _ in 1..35 {
            engine.tick_without_snapshot().expect("tick succeeds");
        }
        assert!(engine.is_owner_eliminated(1));
        assert_eq!(engine.eliminated_owners(), vec![1]);
        assert!(!engine.is_owner_eliminated(2));
    }

    #[test]
    fn crew_elimination_is_one_way_like_cpp() {
        // C4Player::Eliminate never reverts (C4Player.cpp:1684 "Already
        // eliminated safety", 2015-2017) — new crew after elimination does
        // NOT restore the player.
        let mut engine = Engine::with_seed(0);
        let mut definition = build_definition();
        definition.set_crew_member(true);
        definition.set_category(CATEGORY_LIVING);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let owner = engine
            .spawn_object(
                SpawnConfig::new("Test")
                    .with_alive(true)
                    .with_owner(1)
                    .with_crew_member(true),
            )
            .expect("spawn succeeds");

        engine
            .queue_object_command(
                owner,
                QueuedCommand::immediate(ObjectUpdate::new()).with_destroy(true),
            )
            .expect("queue succeeds");
        for _ in 0..35 {
            engine.tick_without_snapshot().expect("tick succeeds");
        }
        assert!(engine.is_owner_eliminated(1));

        engine
            .spawn_object(
                SpawnConfig::new("Test")
                    .with_alive(true)
                    .with_owner(1)
                    .with_crew_member(true),
            )
            .expect("spawn succeeds");
        for _ in 0..35 {
            engine.tick_without_snapshot().expect("tick succeeds");
        }

        assert!(engine.is_owner_eliminated(1), "elimination is one-way");
        assert_eq!(engine.eliminated_owners(), vec![1]);
    }

    #[test]
    fn capture_state_preserves_crew_selection() {
        let mut engine = Engine::with_seed(0);
        let mut definition = build_definition();
        definition.set_crew_member(true);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let first = engine
            .spawn_object(
                SpawnConfig::new("Test")
                    .with_alive(true)
                    .with_owner(1)
                    .with_crew_member(true),
            )
            .expect("spawn succeeds");
        let second = engine
            .spawn_object(
                SpawnConfig::new("Test")
                    .with_alive(true)
                    .with_owner(1)
                    .with_crew_member(true),
            )
            .expect("spawn succeeds");

        engine
            .select_crew(1, vec![first, second])
            .expect("selection succeeds");
        engine
            .set_crew_cursor(1, Some(second))
            .expect("cursor assignment succeeds");

        let mut state = engine.capture_state();
        assert!(
            state.objects.iter().all(|object| object.snapshot.selected),
            "C4Object::Select is persisted on every selected object"
        );
        // Simulate a pre-object-bit state: the old per-player selection list
        // remains a supported import projection.
        for object in &mut state.objects {
            object.snapshot.selected = false;
        }

        let mut restored = Engine::with_seed(5);
        let mut definition = build_definition();
        definition.set_crew_member(true);
        restored
            .register_definition(definition)
            .expect("definition registers");
        restored.restore_state(&state).expect("restore succeeds");

        let mut restored_selected = restored.selected_crew(1);
        restored_selected.sort_by_key(|id| id.as_u64());
        assert_eq!(restored_selected, vec![first, second]);
        assert_eq!(restored.crew_cursor(1), Some(second));
    }

    #[test]
    fn capture_state_preserves_elimination_status() {
        let mut engine = Engine::with_seed(0);
        let mut definition = build_definition();
        definition.set_crew_member(true);
        definition.set_category(CATEGORY_LIVING);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let eliminated = engine
            .spawn_object(
                SpawnConfig::new("Test")
                    .with_alive(true)
                    .with_owner(1)
                    .with_crew_member(true),
            )
            .expect("spawn succeeds");
        engine
            .spawn_object(
                SpawnConfig::new("Test")
                    .with_alive(true)
                    .with_owner(2)
                    .with_crew_member(true),
            )
            .expect("spawn succeeds");

        engine
            .queue_object_command(
                eliminated,
                QueuedCommand::immediate(ObjectUpdate::new()).with_destroy(true),
            )
            .expect("queue succeeds");
        // Elimination lands at the Tick35 boundary (C4Player.cpp:225-235).
        for _ in 0..35 {
            engine.tick_without_snapshot().expect("tick succeeds");
        }

        assert!(engine.is_owner_eliminated(1));
        assert!(!engine.is_owner_eliminated(2));

        let state = engine.capture_state();

        let mut restored = Engine::with_seed(5);
        let mut definition = build_definition();
        definition.set_crew_member(true);
        definition.set_category(CATEGORY_LIVING);
        restored
            .register_definition(definition)
            .expect("definition registers");
        restored.restore_state(&state).expect("restore succeeds");

        assert!(restored.is_owner_eliminated(1));
        assert!(!restored.is_owner_eliminated(2));

        restored
            .spawn_object(
                SpawnConfig::new("Test")
                    .with_alive(true)
                    .with_owner(1)
                    .with_crew_member(true),
            )
            .expect("spawn succeeds");

        // One-way like C4Player::Eliminate (C4Player.cpp:2015-2017).
        assert!(restored.is_owner_eliminated(1));
    }

    #[test]
    fn transfer_zone_set_for_a_vanished_owner_drops_instead_of_aborting() {
        // C++ has no failure mode here: C4TransferZones entries die with
        // their object, so a deferred Set whose owner is gone must drop
        // (warn) rather than abort the batch (the AH_Predator apply
        // regression class).
        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(build_definition())
            .expect("definition registers");
        engine
            .apply_transfer_zone_commands(vec![TransferZoneCommand::Set {
                owner: ObjectId::new(9999),
                rect: TransferZoneRect {
                    x: 0,
                    y: 0,
                    width: 8,
                    height: 8,
                },
            }])
            .expect("missing owner drops, never errors");
        assert!(engine.capture_state().transfer_zones.is_empty());
    }

    #[test]
    fn scenario_batch_transfer_zone_lands_on_an_object_spawned_in_the_same_batch() {
        // C4Game::NewObject adds the object to Game.Objects BEFORE the
        // creation callbacks fire ("From now on, object is ready to be
        // used in scripts!", C4Game.cpp:1115-1131), so a SetTransferZone
        // during the scenario Initialize (FnSetTransferZone ->
        // Game.TransferZones.Set, C4Script.cpp:3145-3149) always finds
        // its owner live. The deferred batch apply must materialize the
        // batch's spawns before its transfer-zone commands land (the
        // AH_Predator door zones: owners 53/69/84/86 spawned in the very
        // batch whose zones were dropped).
        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(build_definition())
            .expect("definition registers");
        let owner = ObjectId::new(53);
        let batch = ScenarioBatch {
            spawns: vec![SpawnConfig::new("Test").with_id(owner)],
            transfer_zones: vec![TransferZoneCommand::Set {
                owner,
                rect: TransferZoneRect {
                    x: 630,
                    y: 620,
                    width: 12,
                    height: 20,
                },
            }],
            ..ScenarioBatch::default()
        };
        engine.apply_scenario_batch(batch).expect("batch applies");
        let zones = engine.capture_state().transfer_zones;
        assert_eq!(zones.len(), 1, "the zone must land on the fresh spawn");
        assert_eq!(zones[0].owner, owner);
    }

    #[test]
    fn capture_state_preserves_transfer_zones() {
        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(build_definition())
            .expect("definition registers");
        let object_id = engine
            .spawn_object(SpawnConfig::new("Test"))
            .expect("spawn succeeds");

        engine
            .set_transfer_zone(
                object_id,
                TransferZoneRect {
                    x: 12,
                    y: -3,
                    width: 8,
                    height: 10,
                },
            )
            .expect("set transfer zone succeeds");

        let state = engine.capture_state();
        assert_eq!(state.transfer_zones.len(), 1);
        let zone = &state.transfer_zones[0];
        assert_eq!(zone.owner, object_id);
        assert_eq!(zone.x, 12);
        assert_eq!(zone.y, -3);
        assert_eq!(zone.width, 8);
        assert_eq!(zone.height, 10);

        let mut restored = Engine::with_seed(3);
        restored
            .register_definition(build_definition())
            .expect("definition registers");
        restored.restore_state(&state).expect("restore succeeds");
        let snapshot = restored.snapshot();
        assert_eq!(snapshot.transfer_zones.len(), 1);
        let restored_zone = &snapshot.transfer_zones[0];
        assert_eq!(restored_zone.owner, object_id);
        assert_eq!(restored_zone.x, 12);
        assert_eq!(restored_zone.y, -3);
        assert_eq!(restored_zone.width, 8);
        assert_eq!(restored_zone.height, 10);
    }

    #[test]
    fn tracks_action_state_changes() {
        let source = r#"#strict 3
        global func Initialize(state, random) {
            return { action = "Walk" };
        }

        global func Step(state, frame, random) {
            if (frame == 1) {
                return { action = { name = "Jump", phase = 3 } };
            }
            return nil;
        }
        "#;

        let mut engine = Engine::with_seed(7);
        let mut definition =
            Definition::from_script("Actor", "Actor", source).expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert("Walk".to_string(), ActionSpec::default());
        actions.insert(
            "Jump".to_string(),
            ActionSpec::default().with_length(10).with_delay(1),
        );
        definition.configure_actions(Some("Walk".to_string()), actions);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("Actor"))
            .expect("spawn succeeds");

        let snapshot = engine
            .object_snapshot(id)
            .expect("object snapshot available");
        assert_eq!(snapshot.action.name, "Walk");
        assert_eq!(snapshot.action.phase, 0);

        let snapshot = engine.tick().expect("first tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.action.name, "Jump");
        assert_eq!(object.action.phase, 3);

        let snapshot = engine.tick().expect("second tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.action.name, "Jump");
        assert_eq!(object.action.phase, 4);
    }

    #[test]
    fn spawns_additional_objects_from_step() {
        let source = r#"#strict 3
        global func Initialize(state, random) {
            return { energy = 42 };
        }

        global func Step(state, frame, random) {
            if (frame == 1) {
                return {
                    spawn = [
                        { definition = "Test", position = [state.position[0] + 5, state.position[1]], velocity = [0, 0], energy = 10, crew_member = false }
                    ]
                };
            }
            return nil;
        }
        "#;
        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(Definition::from_script("Test", "Test", source).unwrap())
            .expect("definition registers");
        engine.set_physics(PhysicsSettings::new(0, 20, -20));

        let id = engine
            .spawn_object(SpawnConfig::new("Test"))
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.position, Vector2::new(0, 0));
        assert_eq!(object.energy, 42);
        assert_eq!(snapshot.objects.len(), 2, "spawned child should exist");

        let spawned = snapshot
            .objects
            .iter()
            .find(|obj| obj.id != id)
            .expect("child object present");
        assert_eq!(spawned.position, Vector2::new(5, 0));
        assert_eq!(spawned.energy, 42);
        assert!(!spawned.crew_member);
    }

    #[test]
    fn produces_deterministic_snapshots() {
        let source = r#"#strict 3
        global func Step(state, frame, random) {
            var new_y = state.position[1] + (random % 3) - 1;
            return { velocity = [state.velocity[0], new_y - state.position[1]] };
        }
        "#;
        let definition = Definition::from_script("Mover", "Mover", source).unwrap();

        let mut engine_a = Engine::with_seed(7);
        engine_a
            .register_definition(definition)
            .expect("definition registers");
        let mut engine_b = Engine::with_seed(7);
        engine_b
            .register_definition(Definition::from_script("Mover", "Mover", source).unwrap())
            .expect("definition registers");

        let id_a = engine_a
            .spawn_object(
                SpawnConfig::new("Mover")
                    .with_position(Vector2::new(0, 0))
                    .with_velocity(Vector2::new(1, 0)),
            )
            .unwrap();
        let id_b = engine_b
            .spawn_object(
                SpawnConfig::new("Mover")
                    .with_position(Vector2::new(0, 0))
                    .with_velocity(Vector2::new(1, 0)),
            )
            .unwrap();

        for _ in 0..5 {
            let snap_a = engine_a.tick().unwrap();
            let snap_b = engine_b.tick().unwrap();
            let obj_a = snap_a.object(id_a).unwrap();
            let obj_b = snap_b.object(id_b).unwrap();
            assert_eq!(obj_a.position, obj_b.position);
            assert_eq!(obj_a.velocity, obj_b.velocity);
        }
    }

    #[test]
    fn clamps_objects_to_landscape_surface() {
        let script = r#"
        global func Step(state, frame, random) {
            return 0;
        }
        "#;
        let mut definition =
            Definition::from_script("Static", "Static", script).expect("script compiles");
        definition.set_shape_vertices(vec![ObjectVertex::new(0, 0)]);
        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine.set_landscape(Landscape::flat(16, 5));

        let id = engine
            .spawn_object(
                SpawnConfig::new("Static")
                    .with_position(Vector2::new(4, 12))
                    .with_velocity(Vector2::new(0, 3)),
            )
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.position, Vector2::new(4, 5));
        assert_eq!(object.velocity, Vector2::new(0, 0));
    }

    #[test]
    fn applies_effect_stack_operations() {
        let source = r#"#strict 3
        global func Initialize(state, random) {
            return {
                effects = [
                    { op = "add", name = "Heal", priority = 150, interval = 2 }
                ]
            };
        }

        global func Step(state, frame, random) {
            if (frame == 1) {
                return {
                    effects = [
                        { op = "add", name = "Boost", priority = 50, interval = 3, timer = 1 }
                    ]
                };
            }
            if (frame == 2) {
                return { effects = [ { op = "remove", name = "Heal" } ] };
            }
            return nil;
        }
        "#;

        let mut engine = Engine::with_seed(0);
        let definition =
            Definition::from_script("Actor", "Actor", source).expect("script compiles");
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("Actor"))
            .expect("spawn succeeds");

        let snapshot = engine.object_snapshot(id).expect("snapshot available");
        assert_eq!(snapshot.effects.len(), 1);
        assert_eq!(snapshot.effects[0].name, "Heal");
        assert_eq!(snapshot.effects[0].priority, 150);
        assert_eq!(snapshot.effects[0].interval, 2);
        assert_eq!(snapshot.effects[0].timer, 0);

        let snapshot = engine.tick().expect("first tick succeeds");
        let object = snapshot.object(id).expect("object present");
        // C++ list order ascends by |priority| (C4Effect.cpp:80-94).
        assert_eq!(object.effects.len(), 2);
        assert_eq!(object.effects[0].name, "Boost");
        assert_eq!(object.effects[0].timer, 1);
        assert_eq!(object.effects[1].name, "Heal");
        assert_eq!(object.effects[1].timer, 1);

        let snapshot = engine.tick().expect("second tick succeeds");
        let object = snapshot.object(id).expect("object present");
        let boost = object
            .effects
            .iter()
            .find(|effect| effect.name == "Boost")
            .expect("Boost remains active");
        assert_eq!(boost.priority, 50);
        assert_eq!(boost.timer, 2);
        assert!(object
            .effects
            .iter()
            .any(|effect| effect.name == "Heal" && effect.priority == 0));

        let snapshot = engine.tick().expect("third tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(
            object
                .effects
                .iter()
                .map(|effect| effect.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Boost"],
            "the next Execute unlinks the dead Heal node"
        );
    }

    // The AmmoHud pair: AHUD#1's Initialize counts its own def
    // (excluding itself) and creates the partner when none exists; the
    // partner's Initialize then counts 1 and stops (AmmoHud.c4d:17-18,22;
    // C++ NEWOBJ 1422/1423).
    #[test]
    fn initialize_time_self_count_spawns_exactly_one_partner_like_cpp() {
        let script = r#"#strict
local pOld;
local fSecond;
protected func Initialize()
{
  SetPosition(56,86);
  Local(0) = Local(1) = 0;
  SetCategory(C4D_StaticBack|C4D_Parallax|C4D_Foreground|C4D_MouseIgnore|C4D_IgnoreFoW);
  SetVisibility(VIS_Owner);
  SetAction("AmmoHud");
  if(!(fSecond=HudCount()))
    CreateObject(GetID(),0,0,GetOwner());
}
protected func HudCount() { return(ObjectCount(GetID(),0,0,0,0,0,0,0,0,GetOwner())); }
"#;
        let mut engine = Engine::with_seed(0);
        let mut hud = Definition::from_script("AHUD", "Hud", script).expect("compiles");
        hud.configure_actions(
            None,
            HashMap::from([("AmmoHud".to_string(), ActionSpec::default())]),
        );
        engine.register_definition(hud).expect("hud registers");
        // Spawn through the REAL creation path (CreateObject host fn):
        // C4Game::NewObject adds the object to the list BEFORE the
        // Construction/Initialize calls, so HudCount sees the first hud
        // while the partner initializes (C4Game.cpp:1107-1127).
        let caller = Definition::from_script(
            "CALL",
            "Caller",
            "#strict\nfunc Trigger() { CreateObject(AHUD, 0, 0, -1); return(1); }\n",
        )
        .expect("caller compiles");
        engine
            .register_definition(caller)
            .expect("caller registers");
        let id = engine
            .spawn_object(SpawnConfig::new("CALL").with_category(CATEGORY_OBJECT))
            .expect("caller spawns");
        let idx = engine.find_object_index(id).expect("caller exists");
        engine
            .call_object_function(idx, "Trigger", Vec::new())
            .expect("trigger runs");
        let count = engine
            .objects
            .iter()
            .filter(|object| object.definition_id == "AHUD")
            .count();
        assert_eq!(
            count, 2,
            "one spawn yields the pair, no more (AmmoHud.c4d:17)"
        );
    }

    // C4Object::Init: `if (Category & C4D_Living) Alive = 1; if (Alive)
    // Energy = GetPhysical()->Energy` (C4Object.cpp:191-192) — energy is
    // the RAW physical scale (C4MaxPhysical = 100000), not a percent.
    // GoldRush oracle: bandits read 25000 (SetPhysical temporary), crew
    // 55000 (rank-1 PromotionUpdate), DefCore [Physical] Energy=50000.
    #[test]
    fn alive_spawns_start_at_the_physical_energy_like_cpp() {
        let mut engine = Engine::with_seed(0);
        let mut living = simple_definition("CLNK");
        living.set_physical(PhysicalInfo {
            energy: 50_000,
            ..PhysicalInfo::default()
        });
        engine
            .register_definition(living)
            .expect("living registers");
        engine
            .register_definition(simple_definition("ROCK"))
            .expect("rock registers");

        let clonk = engine
            .spawn_object(SpawnConfig::new("CLNK").with_category(CATEGORY_OBJECT | CATEGORY_LIVING))
            .expect("clonk spawns");
        let idx = engine.find_object_index(clonk).expect("clonk exists");
        assert_eq!(
            engine.objects[idx].state.energy, 50_000,
            "alive spawn: Energy = GetPhysical()->Energy (C4Object.cpp:192)"
        );

        let rock = engine
            .spawn_object(SpawnConfig::new("ROCK").with_category(CATEGORY_OBJECT))
            .expect("rock spawns");
        let idx = engine.find_object_index(rock).expect("rock exists");
        assert_eq!(
            engine.objects[idx].state.energy, 0,
            "non-living: Energy stays 0"
        );

        // Loaded objects compile Energy= verbatim (C4Object.cpp:2754).
        let loaded = engine
            .spawn_object(
                SpawnConfig::new("CLNK")
                    .with_category(CATEGORY_OBJECT | CATEGORY_LIVING)
                    .with_energy(23_456)
                    .with_loaded(true),
            )
            .expect("loaded spawns");
        let idx = engine.find_object_index(loaded).expect("loaded exists");
        assert_eq!(engine.objects[idx].state.energy, 23_456);
    }

    // FnDoEnergy: `if (!fExact) iChange *= C4MaxPhysical/100` (=1000,
    // C4Object.cpp:1347) and clamps 0..GetPhysical()->Energy; FnGetEnergy
    // reads back `100 * Energy / C4MaxPhysical` — scripts always see
    // percent while the object stores the raw physical scale
    // (C4Script.cpp FnGetEnergy).
    #[test]
    fn do_energy_and_get_energy_use_the_cpp_scales() {
        let script = r#"#strict
local iRead;
func Hurt() {
    DoEnergy(-3);
    iRead = GetEnergy();
    return(1);
}
func HurtExact() {
    DoEnergy(-500, 0, 1);
    return(1);
}
func Overheal() {
    DoEnergy(100);
    return(1);
}
"#;
        let mut engine = Engine::with_seed(0);
        let mut living = Definition::from_script("CLNK", "Clonk", script).expect("compiles");
        living.set_physical(PhysicalInfo {
            energy: 50_000,
            ..PhysicalInfo::default()
        });
        engine.register_definition(living).expect("registers");
        let id = engine
            .spawn_object(SpawnConfig::new("CLNK").with_category(CATEGORY_OBJECT | CATEGORY_LIVING))
            .expect("spawns");
        let idx = engine.find_object_index(id).expect("exists");

        engine
            .call_object_function(idx, "Hurt", Vec::new())
            .expect("hurt runs");
        let idx = engine.find_object_index(id).expect("exists");
        assert_eq!(
            engine.objects[idx].state.energy, 47_000,
            "DoEnergy(-3) removes 3% = 3000 raw (C4Object.cpp:1347)"
        );
        assert_eq!(
            engine.objects[idx].state.local_vars.get("iRead"),
            Some(&Value::Int(47)),
            "GetEnergy returns 100*E/C4MaxPhysical"
        );

        engine
            .call_object_function(idx, "HurtExact", Vec::new())
            .expect("exact runs");
        let idx = engine.find_object_index(id).expect("exists");
        assert_eq!(
            engine.objects[idx].state.energy, 46_500,
            "fExact skips the percent conversion"
        );

        engine
            .call_object_function(idx, "Overheal", Vec::new())
            .expect("overheal runs");
        let idx = engine.find_object_index(id).expect("exists");
        assert_eq!(
            engine.objects[idx].state.energy, 50_000,
            "clamped to GetPhysical()->Energy (C4Object.cpp:1361)"
        );
    }

    // FnDoBreath defaults a nil target to cthr->Obj, scales script points by
    // C4MaxPhysical/100 and clamps to GetPhysical()->Breath
    // (C4Script.cpp:502-506; C4Object.cpp:1406-1413). The following
    // GetBreath in the SAME callback reads the live write (:1143-1146).
    #[test]
    fn do_breath_updates_and_reads_back_the_local_target_like_cpp() {
        let script = r#"#strict
func Refill() {
    DoBreath(100);
    return(GetBreath());
}
"#;
        let mut definition =
            Definition::from_script("CLNK", "Clonk", script).expect("script compiles");
        definition.set_physical(PhysicalInfo {
            breath: 50_000,
            ..PhysicalInfo::default()
        });
        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(definition)
            .expect("definition registers");
        let clonk = engine
            .spawn_object(SpawnConfig::new("CLNK").with_category(CATEGORY_OBJECT))
            .expect("clonk spawns");
        let clonk_idx = engine.find_object_index(clonk).expect("clonk exists");
        engine.objects[clonk_idx].state.breath = 20_000;

        let result = engine
            .call_object_function(clonk_idx, "Refill", Vec::new())
            .expect("DoBreath runs");

        let clonk_idx = engine.find_object_index(clonk).expect("clonk exists");
        assert_eq!(result, Value::Int(50), "same-call GetBreath sees the cap");
        assert_eq!(engine.objects[clonk_idx].state.breath, 50_000);
    }

    // FnDoBreath honors an explicit foreign pObj instead of cthr->Obj
    // (C4Script.cpp:502-506). The foreign scope is live immediately, so a
    // same-callback GetBreath(pObj) observes the staged change.
    #[test]
    fn do_breath_updates_and_reads_back_a_foreign_target_like_cpp() {
        let actor_script = r#"#strict
func RefillOther() {
    var target = FindObject(VCTM);
    DoBreath(10, target);
    return(GetBreath(target));
}
"#;
        let mut victim =
            Definition::from_script("VCTM", "Victim", "#strict\n").expect("victim compiles");
        victim.set_physical(PhysicalInfo {
            breath: 50_000,
            ..PhysicalInfo::default()
        });
        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(
                Definition::from_script("ACTR", "Actor", actor_script)
                    .expect("actor compiles"),
            )
            .expect("actor registers");
        engine
            .register_definition(victim)
            .expect("victim registers");
        let actor = engine
            .spawn_object(SpawnConfig::new("ACTR").with_category(CATEGORY_OBJECT))
            .expect("actor spawns");
        let victim = engine
            .spawn_object(SpawnConfig::new("VCTM").with_category(CATEGORY_OBJECT))
            .expect("victim spawns");
        let victim_idx = engine.find_object_index(victim).expect("victim exists");
        engine.objects[victim_idx].state.breath = 20_000;

        let actor_idx = engine.find_object_index(actor).expect("actor exists");
        let result = engine
            .call_object_function(actor_idx, "RefillOther", Vec::new())
            .expect("foreign DoBreath runs");

        let victim_idx = engine.find_object_index(victim).expect("victim exists");
        assert_eq!(result, Value::Int(30), "foreign live read sees 30000");
        assert_eq!(engine.objects[victim_idx].state.breath, 30_000);
    }

    #[test]
    fn check_energy_need_chain_reports_the_calling_consumer_like_cpp() {
        // FnEnergyCheck writes C4Object::NeedEnergy, then
        // CheckEnergyNeedChain tests the current object's consumer bit and
        // that flag before following any lines (C4Script.cpp:185-208,
        // 1832-1849).
        let script = r#"#strict
func NeedsPower() {
    EnergyCheck(1);
    return(CheckEnergyNeedChain());
}
"#;
        let mut engine = Engine::with_seed(0);
        engine.set_structures_need_energy(true);
        let mut consumer =
            Definition::from_script("ELEV", "Elevator", script).expect("consumer compiles");
        consumer.set_line_connect(LINE_CONNECT_POWER_CONSUMER);
        engine
            .register_definition(consumer)
            .expect("consumer registers");
        let id = engine
            .spawn_object(SpawnConfig::new("ELEV").with_category(CATEGORY_STRUCTURE))
            .expect("consumer spawns");
        let idx = engine.find_object_index(id).expect("consumer exists");

        assert_eq!(
            engine
                .call_object_function(idx, "NeedsPower", Vec::new())
                .expect("energy-chain probe runs"),
            Value::Bool(true)
        );
    }

    #[test]
    fn check_energy_need_chain_follows_power_lines_and_breaks_cycles_like_cpp() {
        // The recursive helper records each visited object, then scans
        // Game.Objects.First-to-last for active PWRL objects whose
        // Action.Target is the current node and recurses through Target2
        // (C4Script.cpp:185-207). The first two lines form a cycle; the
        // later branch must still reach the needy consumer.
        let mut engine = Engine::with_seed(0);
        engine.set_structures_need_energy(true);
        let plant = Definition::from_script(
            "POWR",
            "Power Plant",
            "#strict\nfunc Probe() { return(CheckEnergyNeedChain()); }\n\
             func ProbeTarget(object target) { return(CheckEnergyNeedChain(target, 123)); }\n\
             func ProbeNil() { return(CheckEnergyNeedChain(0)); }\n\
             func ProbeFalse() { return(CheckEnergyNeedChain(false)); }\n",
        )
        .expect("plant compiles");
        engine.register_definition(plant).expect("plant registers");
        engine
            .register_definition(simple_definition("RELY"))
            .expect("relay registers");
        let mut consumer = Definition::from_script(
            "ELEV",
            "Elevator",
            "#strict\nfunc Arm() { return(EnergyCheck(1)); }\n\
             func Disarm() { return(EnergyCheck(0)); }\n",
        )
        .expect("consumer compiles");
        consumer.set_line_connect(LINE_CONNECT_POWER_CONSUMER);
        engine
            .register_definition(consumer)
            .expect("consumer registers");
        let mut line = simple_definition("PWRL");
        line.configure_actions(
            None,
            HashMap::from([("Connect".to_string(), ActionSpec::default())]),
        );
        engine.register_definition(line).expect("line registers");
        let mut wire = simple_definition("WIRE");
        wire.configure_actions(
            None,
            HashMap::from([("Connect".to_string(), ActionSpec::default())]),
        );
        engine.register_definition(wire).expect("wire registers");

        let plant = engine
            .spawn_object(SpawnConfig::new("POWR").with_category(CATEGORY_STRUCTURE))
            .expect("plant spawns");
        let relay = engine
            .spawn_object(SpawnConfig::new("RELY").with_category(CATEGORY_STRUCTURE))
            .expect("relay spawns");
        let consumer = engine
            .spawn_object(SpawnConfig::new("ELEV").with_category(CATEGORY_STRUCTURE))
            .expect("consumer spawns");
        let connect = |definition: &str, from, to| {
            let mut action = ActionState::new("Connect");
            action.target = Some(from);
            action.target2 = Some(to);
            SpawnConfig::new(definition)
                .with_category(CATEGORY_OBJECT)
                .with_action(action)
        };
        engine
            .spawn_object(connect("PWRL", plant, relay))
            .expect("first cycle line spawns");
        engine
            .spawn_object(connect("PWRL", relay, plant))
            .expect("second cycle line spawns");
        engine
            .spawn_object(
                connect("PWRL", plant, consumer).with_status(ObjectStatus::Inactive),
            )
            .expect("inactive line spawns");
        engine
            .spawn_object(connect("WIRE", plant, consumer))
            .expect("non-power line spawns");

        let consumer_idx = engine
            .find_object_index(consumer)
            .expect("consumer exists");
        assert_eq!(
            engine
                .call_object_function(consumer_idx, "Arm", Vec::new())
                .expect("EnergyCheck runs"),
            Value::Bool(false)
        );
        let plant_idx = engine.find_object_index(plant).expect("plant exists");
        assert_eq!(
            engine
                .call_object_function(plant_idx, "Probe", Vec::new())
                .expect("decoy-only probe runs"),
            Value::Bool(false),
            "inactive PWRL and active non-PWRL objects are ignored"
        );
        engine
            .spawn_object(connect("PWRL", plant, consumer))
            .expect("active consumer line spawns");
        assert_eq!(
            engine
                .call_object_function(plant_idx, "Probe", Vec::new())
                .expect("recursive probe runs"),
            Value::Bool(true)
        );
        assert_eq!(
            engine
                .call_object_function(
                    plant_idx,
                    "ProbeTarget",
                    vec![Value::Object(consumer.as_u64())],
                )
                .expect("explicit-target probe runs"),
            Value::Bool(true),
            "the declared object parameter is honored and surplus args are ignored"
        );
        assert_eq!(
            engine
                .call_object_function(plant_idx, "ProbeNil", Vec::new())
                .expect("nil-target probe runs"),
            Value::Bool(true),
            "zero converts to a nil object parameter and defaults to the caller"
        );
        assert_eq!(
            engine
                .call_object_function(plant_idx, "ProbeFalse", Vec::new())
                .expect("false-target probe runs"),
            Value::Bool(true),
            "false is Set0 before C4Object* conversion and defaults to the caller"
        );
        let consumer_snapshot = engine
            .object_snapshot(consumer)
            .expect("consumer snapshot exists");
        assert!(
            consumer_snapshot.need_energy,
            "NeedEnergy is part of the persisted object snapshot (C4Object.cpp:2805)"
        );
        assert_eq!(
            engine
                .call_object_function(consumer_idx, "Disarm", Vec::new())
                .expect("clearing EnergyCheck runs"),
            Value::Bool(true)
        );
        assert_eq!(
            engine
                .call_object_function(plant_idx, "Probe", Vec::new())
                .expect("cleared recursive probe runs"),
            Value::Bool(false),
            "EnergyCheck's success branch clears NeedEnergy (C4Script.cpp:1842-1849)"
        );
    }

    // DoEnergy to zero kills: AssignDeath fires when a nonzero energy
    // reaches 0 (C4Object.cpp:1363) — including through the HOST DoEnergy
    // fold, not just engine damage paths.
    #[test]
    fn host_do_energy_to_zero_assigns_death_like_cpp() {
        let script = r#"#strict
func Slay() { DoEnergy(-100); return(1); }
"#;
        let mut engine = Engine::with_seed(0);
        let mut living = Definition::from_script("CLNK", "Clonk", script).expect("compiles");
        living.set_physical(PhysicalInfo {
            energy: 50_000,
            ..PhysicalInfo::default()
        });
        engine.register_definition(living).expect("registers");
        let id = engine
            .spawn_object(SpawnConfig::new("CLNK").with_category(CATEGORY_OBJECT | CATEGORY_LIVING))
            .expect("spawns");
        let idx = engine.find_object_index(id).expect("exists");
        engine
            .call_object_function(idx, "Slay", Vec::new())
            .expect("slay runs");
        let idx = engine.find_object_index(id).expect("exists");
        assert_eq!(engine.objects[idx].state.energy, 0);
        assert!(
            !engine.objects[idx].state.alive,
            "energy zero from nonzero -> AssignDeath (C4Object.cpp:1363)"
        );
    }

    // C4Game::NewObject adds the object to Game.Objects BEFORE the
    // Construction/Initialize callbacks run (C4Game.cpp:1115-1131), so
    // FnSetTransferZone's Game.TransferZones.Set (C4Script.cpp:3151-3156)
    // succeeds from the object's own Initialize — WZKP's
    // UpdateTransferZone. The rust spawn applies callback batches before
    // insertion, so the zone command must defer to after the push instead
    // of failing UnknownObject.
    #[test]
    fn set_transfer_zone_from_initialize_registers_on_spawn_like_cpp() {
        let script = r#"#strict
func Initialize() { SetTransferZone(-4, -38, 37, 82); return(1); }
"#;
        let mut engine = Engine::with_seed(0);
        let keep = Definition::from_script("WZKP", "WizardKeep", script).expect("compiles");
        engine.register_definition(keep).expect("registers");
        let id = engine
            .spawn_object(
                SpawnConfig::new("WZKP")
                    .with_position(Vector2::new(100, 200))
                    .with_category(CATEGORY_OBJECT),
            )
            .expect("spawn survives the mid-Initialize transfer zone");
        let snapshot = engine.snapshot();
        let zone = snapshot
            .transfer_zones
            .iter()
            .find(|zone| zone.owner == id)
            .expect("the Initialize transfer zone registered");
        assert_eq!(
            (zone.x, zone.y, zone.width, zone.height),
            (96, 162, 37, 82),
            "iX/iY are object-relative (C4Script.cpp:3154)"
        );
    }

    #[test]
    fn set_object_status_deactivation_clears_transfer_zones_for_both_pointer_modes() {
        let controller_script = r#"#strict 2
func Deactivate(object target, bool clear_pointers)
{
    return SetObjectStatus(2, target, clear_pointers);
}
"#;
        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(
                Definition::from_script("CTRL", "Controller", controller_script)
                    .expect("controller compiles"),
            )
            .expect("controller registers");
        engine
            .register_definition(
                Definition::from_script("ZONE", "Zone owner", "#strict 2\n")
                    .expect("zone owner compiles"),
            )
            .expect("zone owner registers");
        let controller = engine
            .spawn_object(SpawnConfig::new("CTRL"))
            .expect("controller spawns");

        for (offset, clear_pointers) in [(0, false), (20, true)] {
            let target = engine
                .spawn_object(
                    SpawnConfig::new("ZONE").with_position(Vector2::new(50 + offset, 50)),
                )
                .expect("zone owner spawns");
            engine
                .set_transfer_zone(
                    target,
                    TransferZoneRect {
                        x: 45 + offset,
                        y: 40,
                        width: 10,
                        height: 20,
                    },
                )
                .expect("zone registers");

            let controller_index = engine
                .find_object_index(controller)
                .expect("controller remains");
            assert_eq!(
                engine
                    .call_object_function(
                        controller_index,
                        "Deactivate",
                        vec![object_reference_value(target), Value::Bool(clear_pointers)],
                    )
                    .expect("deactivation runs"),
                Value::Bool(true)
            );
            assert_eq!(
                engine.object_snapshot(target).expect("target remains").status,
                ObjectStatus::Inactive
            );
            assert!(
                engine
                    .snapshot()
                    .transfer_zones
                    .iter()
                    .all(|zone| zone.owner != target),
                "StatusDeactivate({clear_pointers}) clears its zone before returning"
            );
        }
    }

    #[test]
    fn status_deactivate_clear_exits_containment_before_clearing_pointers() {
        let target_script = r#"#strict 2
local callback_order, holder, pointers_visible, callback_status;

public func Prepare(object pHolder)
{
    holder = pHolder;
    return true;
}

public func Record(int step)
{
    callback_order = callback_order * 10 + step;
    if (GetActionTarget(0, holder) == this()) pointers_visible += 1;
    if (GetCommand(holder, 1) == this()) pointers_visible += 1;
    callback_status = GetObjectStatus();
    SetTransferZone(-1, -1, 2, 2);
    return true;
}

protected func Ejection(object child)
{
    Record(1);
    SetPosition(70, 80);
    return true;
}

protected func Departure(object outer)
{
    Record(4);
    return true;
}
"#;
        let child_script = r#"#strict 2
protected func Departure(object old_container)
{
    old_container->Record(2);
    return true;
}
"#;
        let outer_script = r#"#strict 2
protected func Ejection(object item)
{
    item->Record(3);
    return true;
}
"#;
        let controller_script = r#"#strict 2
public func Deactivate(object target)
{
    return SetObjectStatus(2, target, true);
}
"#;

        let mut engine = Engine::with_seed(82);
        for (id, name, script) in [
            ("L82T", "Target", target_script),
            ("L82C", "Child", child_script),
            ("L82O", "Outer", outer_script),
            ("L82R", "Controller", controller_script),
            ("L82H", "Holder", "#strict 2\n"),
        ] {
            let mut definition =
                Definition::from_script(id, name, script).expect("definition compiles");
            definition.set_c4_callback_convention(true);
            engine
                .register_definition(definition)
                .expect("definition registers");
        }

        let outer = engine
            .spawn_object(SpawnConfig::new("L82O"))
            .expect("outer spawns");
        let target = engine
            .spawn_object(
                SpawnConfig::new("L82T")
                    .with_loaded(true)
                    .with_container(outer)
                    .with_position(Vector2::new(40, 50))
                    .with_fixed_velocity(FixedVec2::new(itofix(3), itofix(-4)))
                    .with_rotation(37)
                    .with_fixed_rotation(itofix(37))
                    .with_rotation_velocity(itofix(2))
                    .with_in_liquid(true)
                    .with_mobile(false),
            )
            .expect("target spawns in the outer container");
        let child = engine
            .spawn_object(
                SpawnConfig::new("L82C")
                    .with_loaded(true)
                    .with_container(target)
                    .with_position(Vector2::new(7, 8))
                    .with_fixed_velocity(FixedVec2::new(itofix(-5), itofix(6)))
                    .with_rotation(23)
                    .with_fixed_rotation(itofix(23))
                    .with_rotation_velocity(itofix(-3))
                    .with_in_liquid(true)
                    .with_mobile(false),
            )
            .expect("child spawns in the target");
        let mut holder_action = ActionState::new("Idle");
        holder_action.target = Some(target);
        let holder = engine
            .spawn_object(SpawnConfig::new("L82H").with_action(holder_action))
            .expect("holder spawns");
        let holder_index = engine.find_object_index(holder).expect("holder exists");
        engine.objects[holder_index]
            .commands
            .push_back(CommandRequest::new(CommandId::Follow).with_target(Some(target)))
            .expect("holder command queues");
        let controller = engine
            .spawn_object(SpawnConfig::new("L82R"))
            .expect("controller spawns");

        let target_index = engine.find_object_index(target).expect("target exists");
        assert_eq!(
            engine
                .call_object_function(
                    target_index,
                    "Prepare",
                    vec![object_reference_value(holder)],
                )
                .expect("target records the holder"),
            Value::Bool(true)
        );
        engine
            .set_transfer_zone(
                target,
                TransferZoneRect {
                    x: 39,
                    y: 49,
                    width: 2,
                    height: 2,
                },
            )
            .expect("initial zone registers");

        let controller_index = engine
            .find_object_index(controller)
            .expect("controller exists");
        assert_eq!(
            engine
                .call_object_function(
                    controller_index,
                    "Deactivate",
                    vec![object_reference_value(target)],
                )
                .expect("clear-mode deactivation runs"),
            Value::Bool(true)
        );

        let target_state = engine.object_snapshot(target).expect("target remains");
        let child_state = engine.object_snapshot(child).expect("child remains");
        let outer_state = engine.object_snapshot(outer).expect("outer remains");
        assert_eq!(target_state.status, ObjectStatus::Inactive);
        assert_eq!(target_state.container, None);
        assert!(target_state.contents.is_empty());
        assert!(!outer_state.contents.contains(&target));
        assert_eq!(child_state.container, None);
        assert_eq!(child_state.position, Vector2::new(40, 50));
        assert_eq!(target_state.position, Vector2::new(70, 80));
        assert_eq!(
            target_state.local_vars.get("callback_order"),
            Some(&Value::Int(1234)),
            "child Ejection/Departure precede target Ejection/Departure"
        );
        assert_eq!(
            target_state.local_vars.get("pointers_visible"),
            Some(&Value::Int(8)),
            "all four callbacks run before the object-pointer sweep"
        );
        assert_eq!(
            target_state.local_vars.get("callback_status"),
            Some(&Value::Int(ObjectStatus::Inactive.to_script_value()))
        );

        for id in [target, child] {
            let index = engine.find_object_index(id).expect("exited object remains");
            let object = &engine.objects[index];
            assert_eq!(object.state.rotation, 0);
            assert_eq!(object.fixed_rotation, C4Fixed::ZERO);
            assert_eq!(object.fixed_velocity, FixedVec2::ZERO);
            assert_eq!(object.state.velocity, Vector2::ZERO);
            assert_eq!(object.rotation_velocity, C4Fixed::ZERO);
            assert!(object.state.mobile);
            assert!(!object.state.in_liquid);
        }

        let holder_index = engine.find_object_index(holder).expect("holder remains");
        assert_eq!(engine.objects[holder_index].state.action.target, None);
        assert_eq!(
            engine.objects[holder_index]
                .commands
                .command_views()
                .first()
                .expect("holder command remains")
                .target,
            None
        );
        assert!(
            engine
                .snapshot()
                .transfer_zones
                .iter()
                .all(|zone| zone.owner != target),
            "Game.ClearPointers clears zones created by the exit callbacks"
        );
    }

    #[test]
    fn status_deactivate_clear_tracks_cpp_contents_iterator_across_reentry() {
        let child_script = r#"#strict 2
local reenter;

public func Arm()
{
    reenter = true;
    return true;
}

protected func Departure(object old_container)
{
    if (reenter) Enter(old_container);
    return true;
}
"#;
        let controller_script = r#"#strict 2
public func Deactivate(object target)
{
    return SetObjectStatus(2, target, true);
}
"#;
        let mut engine = Engine::with_seed(82);
        for (id, name, script) in [
            ("L8IT", "Target", "#strict 2\n"),
            ("L8IC", "Child", child_script),
            ("L8IR", "Controller", controller_script),
        ] {
            let mut definition =
                Definition::from_script(id, name, script).expect("definition compiles");
            definition.set_c4_callback_convention(true);
            engine
                .register_definition(definition)
                .expect("definition registers");
        }

        let target = engine
            .spawn_object(SpawnConfig::new("L8IT"))
            .expect("target spawns");
        let skipped = engine
            .spawn_object(SpawnConfig::new("L8IC").with_container(target))
            .expect("successor child spawns first");
        let reentered = engine
            .spawn_object(SpawnConfig::new("L8IC").with_container(target))
            .expect("reentering child spawns second");
        assert_eq!(
            engine.object_snapshot(target).expect("target exists").contents,
            [reentered, skipped],
            "stContents inserts a same-definition child at the cluster head"
        );
        let reentered_index = engine
            .find_object_index(reentered)
            .expect("reentering child exists");
        assert_eq!(
            engine
                .call_object_function(reentered_index, "Arm", vec![])
                .expect("reentry arms"),
            Value::Bool(true)
        );
        let controller = engine
            .spawn_object(SpawnConfig::new("L8IR"))
            .expect("controller spawns");
        let controller_index = engine
            .find_object_index(controller)
            .expect("controller exists");
        assert_eq!(
            engine
                .call_object_function(
                    controller_index,
                    "Deactivate",
                    vec![object_reference_value(target)],
                )
                .expect("deactivation runs"),
            Value::Bool(true)
        );

        let target_state = engine.object_snapshot(target).expect("target remains");
        assert_eq!(target_state.status, ObjectStatus::Inactive);
        assert_eq!(
            target_state.contents,
            [reentered, skipped],
            "the iterator stays on the original successor link: re-entry cannot alias the removed link"
        );
        assert_eq!(
            engine
                .object_snapshot(reentered)
                .expect("reentered child remains")
                .container,
            Some(target)
        );
        assert_eq!(
            engine
                .object_snapshot(skipped)
                .expect("skipped successor remains")
                .container,
            Some(target)
        );
    }

    #[test]
    fn status_activate_relists_and_updates_position_before_update_transfer_zone() {
        let zone_script = r#"#strict 2
local callback_status, callback_master, callback_sector;

func UpdateTransferZone()
{
    callback_status = GetObjectStatus();
    callback_master = FindObject2(Find_ID(ZONE)) == this();
    callback_sector = FindObject2(Find_ID(ZONE), Find_AtPoint(0, 0)) == this();
    SetTransferZone(-4, -3, 8, 6);
}
"#;
        let controller_script = r#"#strict 2
func SetStatus(object target, int status)
{
    return SetObjectStatus(status, target, false);
}

func FindZones()
{
    return FindObjects(Find_ID(ZONE));
}
"#;
        let mut engine = Engine::with_seed(0);
        engine.set_landscape(Landscape::flat(200, 200));
        let mut zone = Definition::from_script("ZONE", "Zone owner", zone_script)
            .expect("zone owner compiles");
        zone.set_category(CATEGORY_OBJECT);
        zone.set_shape_rect(Some(DefinitionRect::new(-5, -5, 10, 10)));
        engine.register_definition(zone).expect("zone owner registers");
        engine
            .register_definition(
                Definition::from_script("CTRL", "Controller", controller_script)
                    .expect("controller compiles"),
            )
            .expect("controller registers");

        let target = engine
            .spawn_object(SpawnConfig::new("ZONE").with_position(Vector2::new(50, 50)))
            .expect("target spawns first");
        let peer = engine
            .spawn_object(SpawnConfig::new("ZONE").with_position(Vector2::new(150, 150)))
            .expect("same-definition peer spawns second");
        let controller = engine
            .spawn_object(SpawnConfig::new("CTRL"))
            .expect("controller spawns");
        let call_status = |engine: &mut Engine, status| {
            let index = engine
                .find_object_index(controller)
                .expect("controller remains");
            engine
                .call_object_function(
                    index,
                    "SetStatus",
                    vec![object_reference_value(target), Value::Int(status)],
                )
                .expect("status call runs")
        };

        assert_eq!(call_status(&mut engine, 2), Value::Bool(true));
        assert_eq!(
            engine.object_snapshot(target).expect("target remains").status,
            ObjectStatus::Inactive
        );
        assert_eq!(call_status(&mut engine, 1), Value::Bool(true));

        let target_state = engine.object_snapshot(target).expect("target reactivates");
        assert_eq!(target_state.status, ObjectStatus::Normal);
        assert_eq!(
            target_state.local_vars.get("callback_status"),
            Some(&Value::Int(1)),
            "the callback runs after Status becomes normal"
        );
        assert_eq!(
            target_state.local_vars.get("callback_master"),
            Some(&Value::Bool(true)),
            "the callback runs after stMain re-listing"
        );
        assert_eq!(
            target_state.local_vars.get("callback_sector"),
            Some(&Value::Bool(true)),
            "the callback runs after UpdatePos restores the sector link"
        );
        let zone = engine
            .snapshot()
            .transfer_zones
            .into_iter()
            .find(|zone| zone.owner == target)
            .expect("UpdateTransferZone re-registers the zone");
        assert_eq!(
            (zone.x, zone.y, zone.width, zone.height),
            (
                target_state.position.x - 4,
                target_state.position.y - 3,
                8,
                6,
            )
        );
        let controller_index = engine
            .find_object_index(controller)
            .expect("controller remains");
        assert_eq!(
            engine
                .call_object_function(controller_index, "FindZones", vec![])
                .expect("post-fold FindObjects runs"),
            Value::Array(vec![
                object_reference_value(target),
                object_reference_value(peer),
            ]),
            "the authoritative list keeps the fresh stMain insertion order"
        );
    }

    #[test]
    fn failed_initialize_keeps_pre_error_creations_and_burned_ids_like_cpp() {
        // C4AulExec errors abort the call but roll NOTHING back
        // (C4AulExec.cpp:1318-1342): a CreateObject before the error has
        // already run C4Game::NewObject, so the child object exists and
        // `Number = ++ObjectEnumerationIndex` (C4Game.cpp:1119) stays
        // advanced — the burned number is never re-minted. AH_Predator's
        // HZCK Initialize creates its helper, then dies on the missing
        // GetHUD; C++ keeps the helper AND the number, so the next
        // creation (CHOS's TIM1) gets a fresh id.
        let script = r#"#strict
func Initialize() { CreateObject(CHLD, 0, 0, -1); UnknownFn(); return(1); }
"#;
        let mut engine = Engine::with_seed(0);
        let parent = Definition::from_script("PRNT", "Parent", script).expect("compiles");
        engine.register_definition(parent).expect("registers");
        let child = Definition::from_script("CHLD", "Child", "#strict\n").expect("compiles");
        engine.register_definition(child).expect("registers");
        let parent_id = engine
            .spawn_object(SpawnConfig::new("PRNT").with_category(CATEGORY_OBJECT))
            .expect("the erroring Initialize is a fail-safe game call");
        let snapshot = engine.snapshot();
        let child_object = snapshot
            .objects
            .iter()
            .find(|object| object.definition_id.as_str() == "CHLD")
            .expect("the pre-error CreateObject persists like C++");
        assert_eq!(
            child_object.id.as_u64(),
            parent_id.as_u64() + 1,
            "the in-flight preview id materializes verbatim"
        );
        let next = engine
            .spawn_object(SpawnConfig::new("CHLD").with_category(CATEGORY_OBJECT))
            .expect("spawns");
        assert_eq!(
            next.as_u64(),
            parent_id.as_u64() + 2,
            "the burned id is never re-minted"
        );
    }

    #[test]
    fn nested_pending_creation_keeps_initialize_locals_after_materialization_like_cpp() {
        // FnCreateObject enters C4Game::NewObject (C4Script.cpp:1886-1902),
        // which adds the new object to Game.Objects BEFORE running its
        // Construction/Initialize callbacks (C4Game.cpp:1121-1138). Thus a
        // child created from another object's Initialize has live object
        // locals immediately, and references written by the child's own
        // Initialize remain callable after the outer creation returns.
        let parent_script = r#"#strict
func Initialize() { CreateObject(CHLD, 0, 0, -1); }
"#;
        let child_script = r#"#strict
local helper;
func Initialize() { helper = CreateObject(HELP, 0, 0, -1); }
func Probe() { return helper->Ping(); }
"#;
        let helper_script = r#"#strict
func Ping() { return 17; }
"#;

        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(
                Definition::from_script("PRNT", "Parent", parent_script).expect("parent compiles"),
            )
            .expect("parent registers");
        engine
            .register_definition(
                Definition::from_script("CHLD", "Child", child_script).expect("child compiles"),
            )
            .expect("child registers");
        engine
            .register_definition(
                Definition::from_script("HELP", "Helper", helper_script)
                    .expect("helper compiles"),
            )
            .expect("helper registers");

        engine
            .spawn_object(SpawnConfig::new("PRNT").with_category(CATEGORY_OBJECT))
            .expect("outer creation succeeds");

        let child = engine
            .snapshot()
            .objects
            .into_iter()
            .find(|object| object.definition_id == "CHLD")
            .expect("nested child materialized");
        let helper = engine
            .snapshot()
            .objects
            .into_iter()
            .find(|object| object.definition_id == "HELP")
            .expect("helper materialized");
        assert_eq!(
            child.local_vars.get("helper"),
            Some(&Value::Object(helper.id.as_u64())),
            "the pending child's Initialize local survives materialization"
        );
        let child_index = engine
            .find_object_index(child.id)
            .expect("child remains callable");
        assert_eq!(
            engine
                .call_object_function(child_index, "Probe", Vec::new())
                .expect("the persisted helper reference is callable"),
            Value::Int(17)
        );
    }

    #[test]
    fn parent_creation_queue_observes_prior_live_mutation_like_cpp() {
        // FnSetXDir writes the live target immediately (C4Script.cpp:697-705),
        // and the later FnCreateObject synchronously enters NewObject and its
        // Initialize callback (C4Script.cpp:1886-1902; C4Game.cpp:1121-1138).
        // Therefore CHLD's GetXDir (C4Script.cpp:1168-1174) must observe the
        // marker mutation represented by PRNT's returned outcome before its
        // queued child runs Initialize.
        let child_script = r#"#strict
local seen;
func Initialize() { seen = GetXDir(FindObject(MARK), 100); }
"#;

        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(simple_definition("MARK"))
            .expect("marker registers");
        engine
            .register_definition(
                Definition::from_script("CHLD", "Child", child_script).expect("child compiles"),
            )
            .expect("child registers");

        let marker_id = engine
            .spawn_object(SpawnConfig::new("MARK").with_category(CATEGORY_OBJECT))
            .expect("already-live marker spawns");

        let mut update = ObjectUpdate::default();
        update.fixed_velocity_x = Some(math::itofix_prec(300, 100));
        let parent_outcome = compat::NestedObjectOutcome {
            assign_death: None,
            object_id: marker_id,
            effects: Vec::new(),
            update: Some(update),
            commands: Vec::new(),
            command_operations: Vec::new(),
            destroy: false,
            contents_orders: Vec::new(),
        };
        engine
            .process_spawn_queue_with_outcomes(
                vec![SpawnConfig::new("CHLD").with_category(CATEGORY_OBJECT)],
                vec![parent_outcome],
            )
            .expect("parent's child queue succeeds");

        let child = engine
            .snapshot()
            .objects
            .into_iter()
            .find(|object| object.definition_id == "CHLD")
            .expect("nested child materialized");
        assert_eq!(
            child.local_vars.get("seen"),
            Some(&Value::Int(300)),
            "the child Initialize sees the earlier synchronous marker write"
        );
    }

    #[test]
    fn retained_pending_outcome_applies_before_later_initializer_like_cpp() {
        // Each C4Game::NewObject inserts and fully initializes one object
        // before FnCreateObject returns (C4Game.cpp:1121-1138). A mutation
        // made to that object is therefore live before a later CreateObject
        // enters the next object's Initialize (C4Script.cpp:1886-1902).
        let child_script = r#"#strict
local flag;
func Read() { return flag; }
"#;
        let observer_script = r#"#strict
local seen;
func Initialize() { seen = FindObject(CHLD)->Read(); }
"#;

        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(
                Definition::from_script("CHLD", "Child", child_script).expect("child compiles"),
            )
            .expect("child registers");
        engine
            .register_definition(
                Definition::from_script("OBSV", "Observer", observer_script)
                    .expect("observer compiles"),
            )
            .expect("observer registers");

        let child_id = ObjectId::new(40);
        let observer_id = ObjectId::new(41);
        let mut update = ObjectUpdate::default();
        update.local_vars = Some(HashMap::from([("flag".to_string(), Value::Int(99))]));
        let retained = compat::NestedObjectOutcome {
            assign_death: None,
            object_id: child_id,
            effects: Vec::new(),
            update: Some(update),
            commands: Vec::new(),
            command_operations: Vec::new(),
            destroy: false,
            contents_orders: Vec::new(),
        };

        engine
            .process_spawn_queue_with_outcomes(
                vec![
                    SpawnConfig::new("CHLD")
                        .with_id(child_id)
                        .with_category(CATEGORY_OBJECT),
                    SpawnConfig::new("OBSV")
                        .with_id(observer_id)
                        .with_category(CATEGORY_OBJECT),
                ],
                vec![retained],
            )
            .expect("creation queue succeeds");

        let observer = engine
            .object_snapshot(observer_id)
            .expect("observer materialized");
        assert_eq!(
            observer.local_vars.get("seen"),
            Some(&Value::Int(99)),
            "the retained child mutation commits before the observer Initialize"
        );
    }

    #[test]
    fn creation_callback_contents_order_waits_for_parent_and_children() {
        // C++ links PRNT before Construction, then CreateContents links each
        // child synchronously. The final ShiftContents therefore rotates the
        // StaticBack pistol ahead of both ordinary items. Rust materializes
        // the parent and its callback-created children later, so the raw-list
        // outcome must remain pending until every referenced id exists.
        let parent_script = r#"#strict
func Construction()
{
  CreateContents(GOLD);
  CreateContents(ROCK);
  CreateContents(PSTL);
  ShiftContents(0, true, PSTL);
  return(1);
}
"#;
        let mut engine = Engine::with_seed(0);
        let mut parent =
            Definition::from_script("PRNT", "Parent", parent_script).expect("parent compiles");
        parent.set_category(CATEGORY_OBJECT);
        engine
            .register_definition(parent)
            .expect("parent registers");
        for id in ["ROCK", "GOLD"] {
            let mut item = simple_definition(id);
            item.set_category(CATEGORY_OBJECT);
            engine.register_definition(item).expect("item registers");
        }
        let mut pistol = simple_definition("PSTL");
        pistol.set_category(CATEGORY_STATIC_BACK);
        engine
            .register_definition(pistol)
            .expect("pistol registers");

        let parent = engine
            .spawn_object(SpawnConfig::new("PRNT").with_category(CATEGORY_OBJECT))
            .expect("parent and callback-created contents materialize");
        let contents = engine
            .object_snapshot(parent)
            .expect("parent remains live")
            .contents
            .into_iter()
            .map(|child| {
                engine
                    .object_snapshot(child)
                    .expect("created child remains live")
                    .definition_id
            })
            .collect::<Vec<_>>();
        assert_eq!(
            contents,
            ["PSTL", "ROCK", "GOLD"],
            "the retained callback-final list overrides generic spawn insertion"
        );
    }

    #[test]
    fn failed_construction_keeps_pre_error_creations_and_burned_ids_like_cpp() {
        // Same no-rollback contract as the Initialize twin, on the
        // Construction callback C4Object::Init fires first
        // (C4Object.cpp:198-215 via C4Game::NewObject): C4AulExec errors
        // abort the call but keep every prior side effect
        // (C4AulExec.cpp:1318-1342) and `++ObjectEnumerationIndex`
        // (C4Game.cpp:1119) never rewinds.
        let script = r#"#strict
func Construction() { CreateObject(CHLD, 0, 0, -1); UnknownFn(); return(1); }
"#;
        let mut engine = Engine::with_seed(0);
        let parent = Definition::from_script("PRNT", "Parent", script).expect("compiles");
        engine.register_definition(parent).expect("registers");
        let child = Definition::from_script("CHLD", "Child", "#strict\n").expect("compiles");
        engine.register_definition(child).expect("registers");
        let parent_id = engine
            .spawn_object(SpawnConfig::new("PRNT").with_category(CATEGORY_OBJECT))
            .expect("the erroring Construction is a fail-safe game call");
        let snapshot = engine.snapshot();
        let child_object = snapshot
            .objects
            .iter()
            .find(|object| object.definition_id.as_str() == "CHLD")
            .expect("the pre-error CreateObject persists like C++");
        assert_eq!(
            child_object.id.as_u64(),
            parent_id.as_u64() + 1,
            "the in-flight preview id materializes verbatim"
        );
        let next = engine
            .spawn_object(SpawnConfig::new("CHLD").with_category(CATEGORY_OBJECT))
            .expect("spawns");
        assert_eq!(
            next.as_u64(),
            parent_id.as_u64() + 2,
            "the burned id is never re-minted"
        );
    }

    // SkiesOfFire InitializePlayer refills the crew's magic:
    // `clonk->DoMagicEnergy(clonk->GetPhysical("Magic")/2000)`
    // (SkiesOfFire.c4s/Script.c:14) — routed through Fantasy
    // NoMagicEnergy.c4d's `global func DoMagicEnergy` (Script.c:16-28),
    // which chains to the ENGINE fn via inherited when the NMGE rule is
    // absent. FnDoMagicEnergy (C4Script.cpp:517-544) must exist under the
    // override, its write must fold onto engine state, and
    // FnGetMagicEnergy (:546-550) must read it back in
    // MagicPhysicalFactor units.
    #[test]
    fn host_do_magic_energy_folds_through_the_global_override_like_cpp() {
        let script = r#"#strict
func Refill() { return(DoMagicEnergy(25)); }
func ReadBack() { return(GetMagicEnergy()); }
"#;
        let global = r#"#strict
global func DoMagicEnergy(int iChange, object pObject, bool fAllowPartial)
{
  return(inherited(iChange, pObject, fAllowPartial));
}
"#;
        let mut engine = Engine::with_seed(0);
        engine.install_global_scripts(&[("NoMagicEnergy".to_string(), global.to_string())]);
        let mut mage = Definition::from_script("MAGE", "Mage", script).expect("compiles");
        mage.set_physical(PhysicalInfo {
            magic: 50_000,
            ..PhysicalInfo::default()
        });
        engine.register_definition(mage).expect("registers");
        let id = engine
            .spawn_object(SpawnConfig::new("MAGE").with_category(CATEGORY_OBJECT))
            .expect("spawns");
        let idx = engine.find_object_index(id).expect("exists");
        assert_eq!(
            engine
                .call_object_function(idx, "Refill", Vec::new())
                .expect("refill runs"),
            Value::Bool(true)
        );
        let idx = engine.find_object_index(id).expect("exists");
        assert_eq!(
            engine.objects[idx].state.magic_energy, 25_000,
            "the scope write folds onto engine state"
        );
        assert_eq!(
            engine
                .call_object_function(idx, "ReadBack", Vec::new())
                .expect("readback runs"),
            Value::Int(25)
        );
    }

    // GoldRush DoInitialize pins NPCs in place: `while(pObj =
    // FindObjectOwner(0,-1,0,0,0,0,OCF_CrewMember,0,0,pObj))
    // AddEffect("StayThere",...)` (Goldrush.c4s/Script.c:34-35) - the
    // owner filter is NO_OWNER, the OCF filter needs the crew bit on
    // ALIVE unowned NPCs, and pFindNext drives the iteration.
    #[test]
    fn find_object_owner_iterates_unowned_crew_like_cpp() {
        let script = r#"#strict
func Sweep() {
    var i, pObj;
    while(pObj = FindObjectOwner(0,-1,0,0,0,0,OCF_CrewMember,0,0,pObj)) {
        AddEffect("StayThere", pObj, 1, 35, pObj);
        ++i;
    }
    return(i);
}
"#;
        let mut engine = Engine::with_seed(0);
        let mut npc = Definition::from_script("NPCX", "Npc", "#strict\n").expect("npc compiles");
        npc.set_crew_member(true);
        engine.register_definition(npc).expect("npc registers");
        let caller = Definition::from_script("CALL", "Caller", script).expect("caller compiles");
        engine
            .register_definition(caller)
            .expect("caller registers");

        for x in [10, 40] {
            engine
                .spawn_object(
                    SpawnConfig::new("NPCX")
                        .with_category(CATEGORY_OBJECT)
                        .with_position(Vector2::new(x, 10))
                        .with_owner(-1)
                        .with_crew_member(true)
                        .with_alive(true),
                )
                .expect("npc spawns");
        }
        let id = engine
            .spawn_object(SpawnConfig::new("CALL").with_category(CATEGORY_OBJECT))
            .expect("caller spawns");
        let idx = engine.find_object_index(id).expect("caller exists");
        let swept = engine
            .call_object_function(idx, "Sweep", Vec::new())
            .expect("sweep runs");
        assert_eq!(swept, Value::Int(2), "both unowned crew NPCs iterated");
        let pinned = engine
            .objects
            .iter()
            .filter(|object| {
                object.definition_id == "NPCX"
                    && object.state.effects.iter().any(|e| e.name == "StayThere")
            })
            .count();
        assert_eq!(pinned, 2, "StayThere lands on every NPC");
    }

    // The REAL GoldRush chain is one level deeper: the SCENARIO script
    // (Script1 -> StartMovie, a global func from the Talker def) does
    // PrivateCall(CreateObject(_TLK), "DoStartMovie") and THAT loop
    // AddEffects foreign targets (Talker.c4d/Script.c:118-138) — a
    // nested call inside a nested call from a non-object scope.
    #[test]
    fn scenario_private_call_blesses_foreign_objects_like_cpp() {
        let talker_script = r#"#strict
global func StartTheMovie() {
    return(PrivateCall(CreateObject(TALK), "DoBless"));
}
private func DoBless() {
    var o;
    while (o = FindObject(0, 0,0,0,0, OCF_Alive, 0,0, 0, o))
        AddEffect("Divinity", o, 200, 1);
    return(1);
}
"#;
        let scenario_script = r#"#strict
protected func Script1() { StartTheMovie(); }
"#;
        let mut engine = Engine::with_seed(0);
        let talker =
            Definition::from_script("TALK", "Talker", talker_script).expect("talker compiles");
        engine
            .register_definition(talker)
            .expect("talker registers");
        let mut animal_def = simple_definition("ANML");
        // Alive targets are livings: OCF_Alive needs Category & C4D_Living
        // (SetOCF, C4Object.cpp:600-605).
        animal_def.set_category(CATEGORY_LIVING);
        engine
            .register_definition(animal_def)
            .expect("animal registers");
        engine
            .install_scenario_script_with_convention("Goldrush", scenario_script, true)
            .expect("scenario script loads");

        let animal = engine
            .spawn_object(
                SpawnConfig::new("ANML")
                    .with_position(Vector2::new(40, 40))
                    .with_alive(true),
            )
            .expect("animal spawns");

        engine.scenario_script_go = true;
        for _ in 0..20 {
            engine.tick_without_snapshot().expect("tick");
        }

        let idx = engine.find_object_index(animal).expect("animal exists");
        let effects = engine.objects[idx].state.effects.clone();
        assert!(
            effects
                .iter()
                .any(|effect| effect.name == "Divinity" && effect.priority == 200),
            "Divinity lands through the scenario->foreign chain: {effects:?}"
        );
    }

    #[test]
    fn object_property_and_index_write_declared_foreign_named_locals() {
        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(
                Definition::from_script(
                    "BANK",
                    "Bank",
                    "#strict 3\n\
                     local money;\n\
                     func ReadMoney() { return money; }",
                )
                .expect("bank compiles"),
            )
            .expect("bank registers");
        engine
            .register_definition(
                Definition::from_script(
                    "CLLR",
                    "Caller",
                    "#strict 3\n\
                     func Write(other) {\n\
                         other.money = 5;\n\
                         other[\"money\"] += 2;\n\
                         return [other.money, other[\"money\"]];\n\
                     }\n\
                     func ReadMissing(other) { return other.missing; }\n\
                     func BadKey(other) { return other[42]; }",
                )
                .expect("caller compiles"),
            )
            .expect("caller registers");

        let bank = engine
            .spawn_object(SpawnConfig::new("BANK"))
            .expect("bank spawns");
        let caller = engine
            .spawn_object(SpawnConfig::new("CLLR"))
            .expect("caller spawns");
        let caller_index = engine.find_object_index(caller).expect("caller exists");
        let target = Value::Object(bank.as_u64());
        assert_eq!(
            engine
                .call_object_function(caller_index, "Write", vec![target.clone()])
                .expect("object-local writes succeed"),
            Value::Array(vec![Value::Int(7), Value::Int(7)]),
        );

        let bank_index = engine.find_object_index(bank).expect("bank exists");
        assert_eq!(
            engine
                .call_object_function(bank_index, "ReadMoney", Vec::new())
                .expect("target reads the persisted local"),
            Value::Int(7),
        );
        let caller_index = engine.find_object_index(caller).expect("caller exists");
        assert_eq!(
            engine
                .call_object_function(caller_index, "ReadMissing", vec![target.clone()])
                .expect("missing local read succeeds"),
            Value::Nil,
        );
        let bank_index = engine.find_object_index(bank).expect("bank exists");
        assert!(
            !engine.objects[bank_index]
                .state
                .local_vars
                .contains_key("missing"),
            "a missing object local reads nil without being materialized",
        );

        let caller_index = engine.find_object_index(caller).expect("caller exists");
        let error = engine
            .call_object_function(caller_index, "BadKey", vec![target])
            .expect_err("non-string object index must fail");
        match error {
            EngineError::Script { source, .. } => assert!(
                source
                    .to_string()
                    .contains("indexed access on object: only string keys are allowed"),
                "got: {source}",
            ),
            other => panic!("expected script error, got {other:?}"),
        }
    }

    // FnLocal (C4Script.cpp:3423-3433) returns `pObj->Local[iIndex].
    // GetRef()` — the two-argument form reads AND writes a FOREIGN
    // object's numbered Local slot through the reference. The GoldRush
    // rifle chain depends on it: WINC::ControlThrow does
    // `Local(0, GetCrosshair(pClonk)) = 84` and ActualizePhase reads
    // `Local(0, GetCrosshair(pClonk))` (Winchester.c4d/Script.c:19,119).
    #[test]
    fn foreign_numbered_local_reads_and_writes_through_like_cpp() {
        let cross_script = r#"#strict
"#;
        let rider_script = r#"#strict
local iRead;
func Probe(pOther) {
    Local(0, pOther) = 84;
    iRead = Local(0, pOther);
    return(1);
}
"#;
        let mut engine = Engine::with_seed(0);
        let cross =
            Definition::from_script("WCHR", "Cross", cross_script).expect("cross compiles");
        engine.register_definition(cross).expect("cross registers");
        let rider =
            Definition::from_script("RIDR", "Rider", rider_script).expect("rider compiles");
        engine.register_definition(rider).expect("rider registers");

        let cross_id = engine
            .spawn_object(SpawnConfig::new("WCHR").with_category(CATEGORY_OBJECT))
            .expect("cross spawns");
        let rider_id = engine
            .spawn_object(SpawnConfig::new("RIDR").with_category(CATEGORY_OBJECT))
            .expect("rider spawns");

        let idx = engine.find_object_index(rider_id).expect("rider exists");
        engine
            .call_object_function(idx, "Probe", vec![Value::Object(cross_id.as_u64())])
            .expect("Probe runs");

        let idx = engine.find_object_index(rider_id).expect("rider exists");
        assert_eq!(
            engine.objects[idx].state.local_vars.get("iRead"),
            Some(&Value::Int(84)),
            "the cross-object read sees the cross-object write \
             (FnLocal by-reference, C4Script.cpp:3423-3433)"
        );
        let cross_idx = engine.find_object_index(cross_id).expect("cross exists");
        assert_eq!(
            engine.objects[cross_idx].state.local_vars.get("__local_0"),
            Some(&Value::Int(84)),
            "the write landed in the TARGET's numbered slot 0"
        );
    }

    // `g_pIntroHorse->SetGait(3)` (M_Mov_Intro.c:19): an arrow call to a
    // PRIVATE function on another object, with an argument. CR resolves
    // it (C4AulExec object calls) and the argument arrives intact.
    #[test]
    fn arrow_call_to_private_function_passes_arguments() {
        let horse_script = r#"#strict
local iGot;
private func SetGait(inGait) {
    iGot = inGait;
    return(1);
}
"#;
        let rider_script = r#"#strict
func Probe(pOther) {
    pOther->SetGait(3);
    return(1);
}
"#;
        let mut engine = Engine::with_seed(0);
        let horse =
            Definition::from_script("HRSE", "Horse", horse_script).expect("horse compiles");
        engine.register_definition(horse).expect("horse registers");
        let rider =
            Definition::from_script("RIDR", "Rider", rider_script).expect("rider compiles");
        engine.register_definition(rider).expect("rider registers");

        let horse_id = engine
            .spawn_object(SpawnConfig::new("HRSE").with_category(CATEGORY_OBJECT))
            .expect("horse spawns");
        let rider_id = engine
            .spawn_object(SpawnConfig::new("RIDR").with_category(CATEGORY_OBJECT))
            .expect("rider spawns");

        let idx = engine.find_object_index(rider_id).expect("rider exists");
        engine
            .call_object_function(idx, "Probe", vec![Value::Object(horse_id.as_u64())])
            .expect("Probe runs");

        let horse_idx = engine.find_object_index(horse_id).expect("horse exists");
        assert_eq!(
            engine.objects[horse_idx].state.local_vars.get("iGot"),
            Some(&Value::Int(3)),
            "the arrow-call argument arrives in the private function"
        );
    }

    // C4Aul mutates the LIVE object: a nested call's own-local write is    // C4Aul mutates the LIVE object: a nested call's own-local write is
    // visible to a DEEPER synchronous call on the same object within the
    // same outer call. The Talker's DoStartMovie sets `sMovName` and
    // ends with AddEffect("Movie", this(), 1, ...) whose synchronous
    // FxMovieStart reads LocalN("sMovName") to PrivateCall
    // Mov<Name>Start (Talker.c4d/Script.c:123-177).
    #[test]
    fn in_flight_local_writes_are_visible_to_deeper_calls_like_cpp() {
        let talker_script = r#"#strict
local sName;
local iSaw;
func Begin() {
    sName = "Intro";
    AddEffect("Movie", this(), 1, 10, this());
    return(1);
}
func FxMovieStart(pTarget, iNumber, fTemp) {
    LocalN("iSaw", pTarget) = LocalN("sName", pTarget);
    return(0);
}
"#;
        let scenario_script = r#"#strict
protected func Script1() { PrivateCall(CreateObject(TALK), "Begin"); }
"#;
        let mut engine = Engine::with_seed(0);
        let talker =
            Definition::from_script("TALK", "Talker", talker_script).expect("talker compiles");
        engine
            .register_definition(talker)
            .expect("talker registers");
        engine
            .install_scenario_script_with_convention("Fixture", scenario_script, true)
            .expect("scenario script loads");

        engine.scenario_script_go = true;
        for _ in 0..20 {
            engine.tick_without_snapshot().expect("tick");
        }

        let talker_idx = engine
            .objects
            .iter()
            .position(|object| object.definition_id.as_str() == "TALK")
            .expect("talker exists");
        let saw = engine.objects[talker_idx]
            .state
            .local_vars
            .get("iSaw")
            .cloned();
        assert_eq!(
            saw,
            Some(Value::String("Intro".to_string().into())),
            "the synchronous FxMovieStart sees the in-flight sName write"
        );
    }

    // `pPlayer->FindObject(HORS, 0, 0, -1, -1)` (M_Mov_Intro.c:16): an    // `pPlayer->FindObject(HORS, 0, 0, -1, -1)` (M_Mov_Intro.c:16): an
    // object-TARGETED engine-function call — C++ resolves the ENGINE
    // FindObject with cthr->Obj = the target (C4AulExec object calls fall
    // back to engine functions, C4AulExec.cpp:1259-1261), so the closest
    // search runs caller-relative to the TARGET (FnFindObject adjusts
    // x/y by cthr->Obj, C4Script.cpp:2115-2121).
    #[test]
    fn object_targeted_host_find_object_uses_target_as_caller() {
        let caller_script = r#"#strict
func Probe(pOther) {
    var pFound = pOther->FindObject(ANML, 0, 0, -1, -1);
    if (pFound) FoundIt(pFound);
    return(1);
}
func FoundIt(pObj) { RemoveObject(pObj); return(1); }
"#;
        let mut engine = Engine::with_seed(0);
        let caller =
            Definition::from_script("CALR", "Caller", caller_script).expect("caller compiles");
        engine
            .register_definition(caller)
            .expect("caller registers");
        let mut animal_def = simple_definition("ANML");
        animal_def.set_category(CATEGORY_OBJECT);
        engine
            .register_definition(animal_def)
            .expect("animal registers");
        let mut probe_def = simple_definition("PROB");
        probe_def.set_category(CATEGORY_OBJECT);
        engine
            .register_definition(probe_def)
            .expect("probe registers");

        let caller_id = engine
            .spawn_object(
                SpawnConfig::new("CALR")
                    .with_position(Vector2::new(10, 10))
                    .with_category(CATEGORY_OBJECT),
            )
            .expect("caller spawns");
        let other = engine
            .spawn_object(SpawnConfig::new("PROB").with_position(Vector2::new(500, 40)))
            .expect("probe spawns");
        let animal = engine
            .spawn_object(SpawnConfig::new("ANML").with_position(Vector2::new(510, 40)))
            .expect("animal spawns");

        let idx = engine.find_object_index(caller_id).expect("caller exists");
        engine
            .call_object_function(idx, "Probe", vec![Value::Object(other.as_u64())])
            .expect("Probe runs");

        let removed = engine
            .find_object_index(animal)
            .map(|index| engine.objects[index].destroyed)
            .unwrap_or(true);
        assert!(
            removed,
            "the target-relative closest search finds the animal next to pOther"
        );
    }

    // Time.c4d/Script.c `Initialized` (and Driftwood.c4d): `while(pOther =
    // FindObject(GetID())) RemoveObject(pOther);` — C4Object::AssignRemoval
    // sets Status=0 IMMEDIATELY (C4Object.cpp:282) and C4Game::FindObject
    // skips Status==0 objects (C4Game.cpp:1360-1365), so the dedup loop
    // removes each duplicate exactly once and terminates within ONE script
    // call. The Rust copy-in/copy-out seam must read removals through the
    // nested-scope staging or the loop never ends (the Tropical/Alchemy/
    // Funnel/Ashlands + GoldenCanyon/ArcticOcean/Arctic join hang).
    #[test]
    fn same_call_remove_object_drops_out_of_find_object() {
        let script = r#"#strict
func Dedup() {
    var pOther, n;
    while ((pOther = FindObject(TIMR)) && n < 32) {
        RemoveObject(pOther);
        n++;
    }
    return(n);
}
"#;
        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(
                Definition::from_script("TIMR", "Timer", script).expect("timer compiles"),
            )
            .expect("timer registers");
        let keeper = engine
            .spawn_object(SpawnConfig::new("TIMR").with_position(Vector2::new(10, 10)))
            .expect("keeper spawns");
        let dup_a = engine
            .spawn_object(SpawnConfig::new("TIMR").with_position(Vector2::new(20, 10)))
            .expect("dup spawns");
        let dup_b = engine
            .spawn_object(SpawnConfig::new("TIMR").with_position(Vector2::new(30, 10)))
            .expect("dup spawns");

        let idx = engine.find_object_index(keeper).expect("keeper exists");
        let result = engine
            .call_object_function(idx, "Dedup", Vec::new())
            .expect("Dedup runs");
        assert_eq!(
            result,
            Value::Int(2),
            "each duplicate is found+removed exactly once (C4Object.cpp:282, C4Game.cpp:1365)"
        );
        for id in [dup_a, dup_b] {
            let destroyed = engine
                .find_object_index(id)
                .map(|index| {
                    engine.objects[index].destroyed
                        || engine.objects[index].state.status == ObjectStatus::Deleted
                })
                .unwrap_or(true);
            assert!(destroyed, "the duplicate's removal was committed");
        }
        let keeper_alive = engine
            .find_object_index(keeper)
            .map(|index| engine.objects[index].state.status == ObjectStatus::Normal)
            .unwrap_or(false);
        assert!(
            keeper_alive,
            "the caller survives (FindObject excludes cthr->Obj, C4Script.cpp:2115-2131)"
        );
    }

    // Basement72.c4d (BAS7) `MoveOutClonk`: `while(Stuck(pObj) &&
    // Inside(GetY(pObj)-GetY(),-15,+5)) SetPosition(GetX(pObj),
    // GetY(pObj)-1,pObj);` — FnSetPosition force-positions ANY pObj live
    // (C4Script.cpp:462-477, pObj->ForcePosition) and FnGetX/FnGetY/FnStuck
    // read the live x/y (C4Script.cpp:1197,1292,1858-1862), so every
    // iteration sees the previous SetPosition and the loop walks the stuck
    // object upward out of the ground. The Rust seam must both APPLY the
    // foreign SetPosition and READ it back within the same call (the
    // SkyIslands/Tutorial04/07/10/FoggyCliffs/Mountains join hang).
    #[test]
    fn same_call_foreign_set_position_is_visible_to_get_y_and_stuck() {
        let script = r#"#strict
func MoveOut(pObj) {
    var n;
    while (Stuck(pObj) && Inside(GetY(pObj)-GetY(), -15, 5) && n < 64) {
        SetPosition(GetX(pObj), GetY(pObj)-1, pObj);
        n++;
    }
    return(n);
}
"#;
        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(
                Definition::from_script("BASE", "Basement", script).expect("basement compiles"),
            )
            .expect("basement registers");
        engine
            .register_definition(simple_definition("CLNK"))
            .expect("clonk registers");
        // Solid ground from y >= 20 in every column.
        engine.set_landscape(Landscape::new(100, vec![20; 100]).expect("landscape constructs"));

        let base = engine
            .spawn_object(
                SpawnConfig::new("BASE")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(50, 28)),
            )
            .expect("basement spawns");
        let stuck_clonk = engine
            .spawn_object(
                SpawnConfig::new("CLNK")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(50, 25))
                    .with_vertices(vec![ObjectVertex::new(0, 0)]),
            )
            .expect("clonk spawns");

        let idx = engine.find_object_index(base).expect("basement exists");
        let result = engine
            .call_object_function(idx, "MoveOut", vec![Value::Object(stuck_clonk.as_u64())])
            .expect("MoveOut runs");
        assert_eq!(
            result,
            Value::Int(6),
            "y walks 25→19 one pixel per iteration, Stuck turns false above the surface \
             (C4Script.cpp:462-477,1858-1862)"
        );
        let final_position = engine
            .object_snapshot(stuck_clonk)
            .expect("clonk snapshot available")
            .position;
        assert_eq!(
            final_position,
            Vector2::new(50, 19),
            "the foreign ForcePosition writes commit to the world"
        );
    }

    // TotemHunt _PLO `DoPlrLaunch`: `while (Contents()) {
    // SetCrewEnabled(1, Contents()); Exit(Contents(), x-GetX(), y-GetY()); }`
    // — C4Object::Exit removes the object from its container's Contents
    // list IMMEDIATELY (C4Object.cpp:1529-1533, `Contents.Remove(this);
    // Contained = nullptr`), so FnContents never returns an exited object
    // again within the same call and the eject loop terminates (the
    // TotemHunt tick hang).
    #[test]
    fn same_call_exit_drops_out_of_contents() {
        let script = r#"#strict
func Eject() {
    var n;
    while (Contents() && n < 32) {
        Exit(Contents());
        n++;
    }
    return(n);
}
"#;
        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(
                Definition::from_script("CONT", "Container", script).expect("container compiles"),
            )
            .expect("container registers");
        engine
            .register_definition(simple_definition("ITEM"))
            .expect("item registers");
        let container = engine
            .spawn_object(
                SpawnConfig::new("CONT")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(40, 40)),
            )
            .expect("container spawns");
        let item_a = engine
            .spawn_object(
                SpawnConfig::new("ITEM")
                    .with_category(CATEGORY_OBJECT)
                    .with_container(container),
            )
            .expect("item spawns");
        let item_b = engine
            .spawn_object(
                SpawnConfig::new("ITEM")
                    .with_category(CATEGORY_OBJECT)
                    .with_container(container),
            )
            .expect("item spawns");

        let idx = engine.find_object_index(container).expect("container exists");
        let result = engine
            .call_object_function(idx, "Eject", Vec::new())
            .expect("Eject runs");
        assert_eq!(
            result,
            Value::Int(2),
            "each content is ejected exactly once (C4Object.cpp:1529-1533)"
        );
        for id in [item_a, item_b] {
            let contained = engine
                .object_snapshot(id)
                .expect("item snapshot available")
                .container;
            assert_eq!(contained, None, "the Exit committed to the world");
        }
    }

    // C++ mutates the ONE live C4Object mid-call: after a nested call
    // stages SetOwner/SetXDir/SetAlive on a foreign object, later host
    // reads in the SAME outer call (GetOwner/GetXDir/GetOCF through the
    // world view) see the staged values — owner (C4Object.cpp:5495-5500),
    // xdir (C4Script.cpp:697-708 / 1163-1167), and the OCF bits SetOCF
    // re-derives synchronously on the alive change (C4Object::AssignDeath
    // -> SetOCF, C4Object.cpp:600-622).
    #[test]
    fn mid_call_reads_see_staged_owner_velocity_and_ocf() {
        let prober_script = r#"#strict
local iOwn, iXd, iOcfAlive;
public func Probe(pB) {
  pB->Prep();
  iOwn = GetOwner(pB);
  iXd = GetXDir(pB);
  iOcfAlive = GetOCF(pB) & OCF_Alive;
  return(1);
}
"#;
        let victim_script = r#"#strict
public func Prep() {
  SetOwner(5);
  SetXDir(30);
  SetAlive(0);
  return(1);
}
"#;
        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(
                Definition::from_script("Prob", "Prober", prober_script).expect("script compiles"),
            )
            .expect("prober registers");
        engine
            .register_definition(
                Definition::from_script("Vict", "Victim", victim_script).expect("script compiles"),
            )
            .expect("victim registers");
        engine
            .register_player(PlayerConfig::new(5, "Owner"))
            .expect("owner registers");
        let prober = engine
            .spawn_object(SpawnConfig::new("Prob").with_category(CATEGORY_OBJECT))
            .expect("prober spawns");
        let victim = engine
            .spawn_object(
                SpawnConfig::new("Vict")
                    .with_category(CATEGORY_OBJECT | CATEGORY_LIVING)
                    .with_alive(true),
            )
            .expect("victim spawns");
        let victim_idx = engine.find_object_index(victim).expect("victim exists");
        assert_ne!(
            engine.objects[victim_idx].state.ocf & clonk_engine::ocf::ALIVE,
            0,
            "sanity: the living victim starts with OCF_Alive"
        );

        let idx = engine.find_object_index(prober).expect("prober exists");
        engine
            .call_object_function(idx, "Probe", vec![Value::Object(victim.as_u64())])
            .expect("probe runs");

        let idx = engine.find_object_index(prober).expect("prober exists");
        let locals = &engine.objects[idx].state.local_vars;
        assert_eq!(
            locals.get("iOwn"),
            Some(&Value::Int(5)),
            "GetOwner sees the staged SetOwner mid-call"
        );
        assert_eq!(
            locals.get("iXd"),
            Some(&Value::Int(30)),
            "GetXDir sees the staged SetXDir mid-call"
        );
        assert_eq!(
            locals.get("iOcfAlive"),
            Some(&Value::Int(0)),
            "GetOCF drops OCF_Alive after the staged SetAlive(0) (SetOCF runs synchronously in C++)"
        );
    }

    // FnExit (C4Script.cpp:372-388): the optional position args are
    // CALLER-relative (`tx += cthr->Obj->x`), the y target gets the
    // subject's Shape.y added, and C4Object::Exit writes position,
    // rotation and the three dirs unconditionally (C4Object.cpp:
    // 1549-1553: `x = iX; y = iY; r = iR; xdir = iXDir; ...`), with
    // rdir scaled `itofix(trdir) / 10`.
    #[test]
    fn exit_applies_caller_relative_position_and_dir_args_like_cpp() {
        let script = r#"#strict
public func Launch(pItem) {
    return(Exit(pItem, 10, 5, 90, 3, -2, 20));
}
"#;
        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(
                Definition::from_script("CONT", "Container", script).expect("container compiles"),
            )
            .expect("container registers");
        let mut item_def = simple_definition("ITEM");
        item_def.set_shape_rect(Some(DefinitionRect::new(-2, -3, 4, 6)));
        engine
            .register_definition(item_def)
            .expect("item registers");
        let container = engine
            .spawn_object(
                SpawnConfig::new("CONT")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(40, 40)),
            )
            .expect("container spawns");
        let item = engine
            .spawn_object(
                SpawnConfig::new("ITEM")
                    .with_category(CATEGORY_OBJECT)
                    .with_container(container),
            )
            .expect("item spawns");

        let idx = engine.find_object_index(container).expect("container exists");
        let result = engine
            .call_object_function(idx, "Launch", vec![Value::Object(item.as_u64())])
            .expect("launch runs");
        assert_eq!(result, Value::Bool(true), "the contained item exits");

        let item_idx = engine.find_object_index(item).expect("item exists");
        let state = &engine.objects[item_idx].state;
        assert_eq!(state.container, None, "the Exit committed");
        assert_eq!(
            state.position,
            Vector2::new(50, 42),
            "x = caller.x + tx, y = caller.y + ty + Shape.y (40+5-3)"
        );
        assert_eq!(state.rotation, 90, "r = tr (C4Object.cpp:1552)");
        assert_eq!(
            engine.objects[item_idx].fixed_velocity,
            FixedVec2::new(itofix(3), itofix(-2)),
            "xdir/ydir = itofix(txdir/tydir)"
        );
        assert_eq!(
            engine.objects[item_idx].rotation_velocity,
            itofix(20) / 10,
            "rdir = itofix(trdir) / 10 (C4Script.cpp:388)"
        );
    }

    #[test]
    fn script_exit_runs_bounds_check_contacts_before_ejection_and_departure(
    ) -> Result<(), EngineError> {
        // FnExit freezes caller-relative x/y, then reads the subject's live
        // Shape.y after CancelAttach and delegates to C4Object::Exit. Exit
        // unlinks before BoundsCheck; ContactLeft therefore sees the old
        // position and motion with only xdir zeroed. The clamped position and
        // requested motion are installed before Ejection and Departure.
        let container_script = r#"#strict
protected func Ejection(pItem)
{
    return(pItem->RecordEjection());
}
"#;
        let item_script = r#"#strict
local order;
local contact_x, contact_y, contact_xdir, contact_ydir, contact_contained, contact_shape_y;
local ejection_x, ejection_y, ejection_xdir, ejection_ydir, ejection_contained;
local departure_x, departure_y, departure_xdir, departure_ydir, departure_contained;

protected func ContactLeft()
{
    var no_value;
    order = order * 10 + 1;
    contact_x = GetX(); contact_y = GetY();
    contact_xdir = GetXDir(); contact_ydir = GetYDir();
    contact_contained = !!Contained();
    contact_shape_y = GetObjectVal("Offset", no_value, no_value, 1);
    return(1);
}

public func RecordEjection()
{
    order = order * 10 + 2;
    ejection_x = GetX(); ejection_y = GetY();
    ejection_xdir = GetXDir(); ejection_ydir = GetYDir();
    ejection_contained = !!Contained();
    return(1);
}

protected func Departure(pOldContainer)
{
    order = order * 10 + 3;
    departure_x = GetX(); departure_y = GetY();
    departure_xdir = GetXDir(); departure_ydir = GetYDir();
    departure_contained = !!Contained();
    return(1);
}

public func Leave()
{
    var no_container;
    SetShape(-4, -5, 8, 10);
    return(Exit(no_container, -100, 10, 90, 3, -2, 20));
}
"#;

        let mut container = Definition::from_script("CONT", "Container", container_script)?;
        container.set_c4_callback_convention(true);
        let mut item = Definition::from_script("ITEM", "Item", item_script)?;
        item.set_c4_callback_convention(true);
        item.set_shape_rect(Some(DefinitionRect::new(-2, -3, 4, 6)));
        item.set_border_bound(C4D_BORDER_SIDES | C4D_BORDER_TOP);
        item.set_contact_function_calls(true);

        let mut engine = Engine::with_seed(5);
        engine.set_landscape(Landscape::flat(100, 100));
        engine.register_definition(container)?;
        engine.register_definition(item)?;
        let container = engine.spawn_object(
            SpawnConfig::new("CONT").with_position(Vector2::new(30, 40)),
        )?;
        let item = engine.spawn_object(
            SpawnConfig::new("ITEM")
                .with_container(container)
                .with_rotation(11)
                .with_in_liquid(true)
                .with_mobile(false),
        )?;
        let item_idx = engine.find_object_index(item).expect("item exists");
        engine.objects[item_idx].set_fixed_velocity(FixedVec2::new(itofix(8), itofix(9)));

        assert_eq!(
            engine.call_object_function(item_idx, "Leave", Vec::new())?,
            Value::Bool(true)
        );

        let item_idx = engine.find_object_index(item).expect("item remains");
        let object = &engine.objects[item_idx];
        assert_eq!(object.state.container, None);
        assert_eq!(object.state.position, Vector2::new(4, 45));
        assert_eq!(object.fixed_position, FixedVec2::new(itofix(4), itofix(45)));
        assert_eq!(object.state.rotation, 90);
        assert_eq!(object.fixed_velocity, FixedVec2::new(itofix(3), itofix(-2)));
        assert_eq!(object.rotation_velocity, itofix(20) / 10);
        assert!(object.state.mobile, "Exit sets Mobile");
        assert!(!object.state.in_liquid, "Exit clears InLiquid");

        let locals = &object.state.local_vars;
        for (name, expected) in [
            ("order", 123),
            ("contact_x", 30),
            ("contact_y", 40),
            ("contact_xdir", 0),
            ("contact_ydir", 90),
            ("contact_shape_y", -5),
            ("ejection_x", 4),
            ("ejection_y", 45),
            ("ejection_xdir", 30),
            ("ejection_ydir", -20),
            ("departure_x", 4),
            ("departure_y", 45),
            ("departure_xdir", 30),
            ("departure_ydir", -20),
        ] {
            assert_eq!(locals.get(name), Some(&Value::Int(expected)), "{name}");
        }
        for name in [
            "contact_contained",
            "ejection_contained",
            "departure_contained",
        ] {
            assert_eq!(locals.get(name), Some(&Value::Bool(false)), "{name}");
        }
        Ok(())
    }

    #[test]
    fn script_exit_runs_ejection_then_departure_and_reports_reentry_like_cpp() {
        // C4Object::Exit clears Contained, calls the old container's
        // Ejection(object), then the object's Departure(container), and only
        // afterwards returns `!Contained` (C4Object.cpp:1532-1563). Ejection
        // may synchronously re-enter the object, making Exit return false;
        // Departure still runs and observes the re-established relation.
        let container_script = r#"#strict
local ejected;
func Ejection(pItem)
{
    ejected = 1;
    Enter(this(), pItem);
}
"#;
        let item_script = r#"#strict
local departure_saw_reentry;
func Departure(pOldContainer)
{
    departure_saw_reentry = (Contained() == pOldContainer);
}
func Leave()
{
    return Exit();
}
"#;
        let mut engine = Engine::with_seed(3);
        engine
            .register_definition(
                Definition::from_script("CONT", "Container", container_script)
                    .expect("container compiles"),
            )
            .expect("container registers");
        engine
            .register_definition(
                Definition::from_script("ITEM", "Item", item_script).expect("item compiles"),
            )
            .expect("item registers");
        let container = engine
            .spawn_object(SpawnConfig::new("CONT").with_category(CATEGORY_OBJECT))
            .expect("container spawns");
        let item = engine
            .spawn_object(
                SpawnConfig::new("ITEM")
                    .with_category(CATEGORY_OBJECT)
                    .with_container(container),
            )
            .expect("item spawns");

        let item_idx = engine.find_object_index(item).expect("item exists");
        let result = engine
            .call_object_function(item_idx, "Leave", vec![])
            .expect("Leave runs");
        assert_eq!(result, Value::Bool(false), "callback re-entry makes Exit fail");

        let container_idx = engine
            .find_object_index(container)
            .expect("container remains");
        let item_idx = engine.find_object_index(item).expect("item remains");
        assert_eq!(engine.objects[item_idx].state.container, Some(container));
        assert_eq!(
            engine.objects[container_idx].state.local_vars.get("ejected"),
            Some(&Value::Int(1)),
            "Ejection ran"
        );
        assert_eq!(
            engine.objects[item_idx]
                .state
                .local_vars
                .get("departure_saw_reentry"),
            Some(&Value::Bool(true)),
            "Departure ran after Ejection and saw the re-entry"
        );
    }

    #[test]
    fn script_exit_cancels_attach_before_containment_and_position_writes() {
        let driver_script = r#"#strict
public func Leave(object target, int x, int y, int r)
{
    return Exit(target, x, y, r);
}
"#;
        let item_script = r#"#strict
local abort_count, abort_container, abort_x, abort_y, abort_r, abort_phase, abort_saw_idle, abort_random;
protected func AttachAbort(int phase)
{
    abort_count++;
    abort_container = Contained();
    abort_x = GetX();
    abort_y = GetY();
    abort_r = GetR();
    abort_phase = phase;
    abort_saw_idle = ActIdle();
    SetShape(-4, -6, 8, 12);
    abort_random = Random(360);
}
"#;

        let mut engine = Engine::with_seed(4);
        engine
            .register_definition(
                Definition::from_script("DRVR", "Driver", driver_script)
                    .expect("driver compiles"),
            )
            .expect("driver registers");
        let mut item_definition =
            Definition::from_script("ITEM", "Item", item_script).expect("item compiles");
        item_definition.set_c4_callback_convention(true);
        item_definition.configure_actions(
            Some("Attach".to_string()),
            HashMap::from([
                (
                    "Attach".to_string(),
                    ActionSpec::default()
                        .with_procedure("ATTACH")
                        .with_abort_call("AttachAbort"),
                ),
                (
                    "Walk".to_string(),
                    ActionSpec::default()
                        .with_procedure("WALK")
                        .with_abort_call("AttachAbort"),
                ),
            ]),
        );
        engine
            .register_definition(item_definition)
            .expect("item registers");

        let driver = engine
            .spawn_object(SpawnConfig::new("DRVR").with_position(Vector2::new(100, 200)))
            .expect("driver spawns");
        let driver_index = engine.find_object_index(driver).expect("driver index");

        let mut attach_action = ActionState::new("Attach");
        attach_action.phase = 3;
        let loose = engine
            .spawn_object(
                SpawnConfig::new("ITEM")
                    .with_position(Vector2::new(30, 40))
                    .with_rotation(11)
                    .with_action(attach_action)
                    .with_loaded(true),
            )
            .expect("loose attached item spawns");
        let mut expected_rng = engine.debug_rng_clone();
        let _discarded_rotation = expected_rng.random(360);
        let expected_abort_random = expected_rng.random(360);
        let result = engine
            .call_object_function(
                driver_index,
                "Leave",
                vec![
                    object_reference_value(loose),
                    Value::Int(10),
                    Value::Int(5),
                    Value::Int(-1),
                ],
            )
            .expect("uncontained Exit runs");
        assert_eq!(result, Value::Bool(false));
        let loose = engine.object_snapshot(loose).expect("loose item remains");
        assert_eq!(loose.action.name, "Idle");
        assert_eq!(loose.position, Vector2::new(30, 40));
        assert_eq!(loose.rotation, 11);
        assert_eq!(loose.local_vars.get("abort_count"), Some(&Value::Int(1)));
        assert_eq!(loose.local_vars.get("abort_container"), Some(&Value::Nil));
        assert_eq!(loose.local_vars.get("abort_phase"), Some(&Value::Int(3)));
        assert_eq!(loose.local_vars.get("abort_saw_idle"), Some(&Value::Bool(true)));
        assert_eq!(
            loose.local_vars.get("abort_random"),
            Some(&Value::Int(expected_abort_random)),
            "tr=-1 consumes Random(360) before CancelAttach's AbortCall even when Exit then fails"
        );
        let mut observed_rng = engine.debug_rng_clone();
        assert_eq!(
            observed_rng.random(360),
            expected_rng.random(360),
            "the failed uncontained Exit and AbortCall consumed exactly two draws"
        );

        let mut attach_action = ActionState::new("Attach");
        attach_action.phase = 7;
        let contained = engine
            .spawn_object(
                SpawnConfig::new("ITEM")
                    .with_position(Vector2::new(30, 40))
                    .with_rotation(11)
                    .with_container(driver)
                    .with_action(attach_action)
                    .with_loaded(true),
            )
            .expect("contained attached item spawns");
        let result = engine
            .call_object_function(
                driver_index,
                "Leave",
                vec![
                    object_reference_value(contained),
                    Value::Int(10),
                    Value::Int(5),
                    Value::Int(90),
                ],
            )
            .expect("contained Exit runs");
        assert_eq!(result, Value::Bool(true));
        let contained = engine
            .object_snapshot(contained)
            .expect("contained item remains");
        assert_eq!(contained.action.name, "Idle");
        assert_eq!(contained.container, None);
        assert_eq!(
            contained.position,
            Vector2::new(110, 199),
            "FnExit reads the subject's Shape.y after CancelAttach's AbortCall"
        );
        assert_eq!(contained.rotation, 90);
        assert_eq!(
            contained.local_vars.get("abort_container"),
            Some(&object_reference_value(driver))
        );
        assert_eq!(contained.local_vars.get("abort_x"), Some(&Value::Int(30)));
        assert_eq!(contained.local_vars.get("abort_y"), Some(&Value::Int(40)));
        assert_eq!(contained.local_vars.get("abort_r"), Some(&Value::Int(11)));
        assert_eq!(contained.local_vars.get("abort_phase"), Some(&Value::Int(7)));

        let walking = engine
            .spawn_object(
                SpawnConfig::new("ITEM")
                    .with_position(Vector2::new(50, 60))
                    .with_container(driver)
                    .with_action(ActionState::new("Walk"))
                    .with_loaded(true),
            )
            .expect("walking item spawns");
        let result = engine
            .call_object_function(
                driver_index,
                "Leave",
                vec![
                    object_reference_value(walking),
                    Value::Int(10),
                    Value::Int(5),
                    Value::Int(90),
                ],
            )
            .expect("non-attach Exit runs");
        assert_eq!(result, Value::Bool(true));
        let walking = engine.object_snapshot(walking).expect("walking item remains");
        assert_eq!(walking.action.name, "Walk");
        assert_eq!(walking.local_vars.get("abort_count"), None);
    }

    // C4Object::Enter adds the object to the container's Contents list
    // IMMEDIATELY (`Contents.Add(this, C4ObjectList::stContents)`,
    // C4Object.cpp:1601-1605) — the mirror of the same-call Exit shrink
    // above: a Collect/Enter loop must see the list GROW within the call
    // (FnContents/FnContentsCount read the live list).
    #[test]
    fn same_call_enter_appends_to_contents() {
        let script = r#"#strict
local iDuring, pFirst;
public func Take(pItem) {
    Enter(this(), pItem);
    iDuring = ContentsCount();
    pFirst = Contents(0);
    return(iDuring);
}
"#;
        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(
                Definition::from_script("CONT", "Container", script).expect("container compiles"),
            )
            .expect("container registers");
        engine
            .register_definition(simple_definition("ITEM"))
            .expect("item registers");
        let container = engine
            .spawn_object(
                SpawnConfig::new("CONT")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(40, 40)),
            )
            .expect("container spawns");
        let item = engine
            .spawn_object(
                SpawnConfig::new("ITEM")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(10, 10)),
            )
            .expect("item spawns");

        let idx = engine.find_object_index(container).expect("container exists");
        let result = engine
            .call_object_function(idx, "Take", vec![Value::Object(item.as_u64())])
            .expect("take runs");
        assert_eq!(
            result,
            Value::Int(1),
            "the same-call Enter is visible to ContentsCount (C4Object.cpp:1601-1605)"
        );
        let idx = engine.find_object_index(container).expect("container exists");
        assert_eq!(
            engine.objects[idx].state.local_vars.get("pFirst"),
            Some(&Value::Object(item.as_u64())),
            "Contents(0) returns the just-entered item mid-call"
        );
        assert_eq!(
            engine.objects[idx].state.contents,
            vec![item],
            "the Enter committed to the world"
        );
    }

    // Dragon Rock's Redefine3 creates the replacement Clonk and immediately
    // calls `pNew->GrabContents(this())` (Drachenfels.c4s/Script.c:153-159).
    // FnGrabContents defaults pTo to the receiver and C4Object::GrabContents
    // snapshots the source list, then Enter()s each still-live child into the
    // receiver (C4Script.cpp:320-327; C4Object.cpp:6162-6171). Enter leaves
    // Owner untouched and stContents insertion reverses this equal-id pair.
    #[test]
    fn grab_contents_on_fresh_receiver_transfers_inventory_like_cpp() {
        let old_script = r#"#strict
public func Replace() {
    var pNew = CreateObject(NEWK, 0, 0, GetOwner());
    return(pNew->GrabContents(this()));
}
"#;
        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(
                Definition::from_script("OLDK", "Old Knight", old_script)
                    .expect("old knight compiles"),
            )
            .expect("old knight registers");
        engine
            .register_definition(simple_definition("NEWK"))
            .expect("new knight registers");
        engine
            .register_definition(simple_definition("ITEM"))
            .expect("item registers");

        let old = engine
            .spawn_object(
                SpawnConfig::new("OLDK")
                    .with_category(CATEGORY_OBJECT | CATEGORY_LIVING)
                    .with_alive(true)
                    .with_owner(7),
            )
            .expect("old knight spawns");
        let first = engine
            .spawn_object(
                SpawnConfig::new("ITEM")
                    .with_category(CATEGORY_OBJECT)
                    .with_owner(3)
                    .with_container(old),
            )
            .expect("first item spawns");
        let second = engine
            .spawn_object(
                SpawnConfig::new("ITEM")
                    .with_category(CATEGORY_OBJECT)
                    .with_owner(4)
                    .with_container(old),
            )
            .expect("second item spawns");
        let old_idx = engine.find_object_index(old).expect("old knight exists");
        assert_eq!(engine.objects[old_idx].state.contents, vec![second, first]);

        let result = engine
            .call_object_function(old_idx, "Replace", Vec::new())
            .expect("replacement call runs");
        assert_eq!(result, Value::Bool(true));

        let new_idx = engine
            .objects
            .iter()
            .position(|object| object.definition_id == "NEWK")
            .expect("replacement exists");
        let new = engine.objects[new_idx].id;
        let old_idx = engine.find_object_index(old).expect("old knight remains");
        assert!(engine.objects[old_idx].state.contents.is_empty());
        assert_eq!(
            engine.objects[new_idx].state.contents,
            vec![first, second],
            "the copied source order is fed through runtime stContents insertion"
        );
        let first_idx = engine.find_object_index(first).expect("first item exists");
        let second_idx = engine.find_object_index(second).expect("second item exists");
        assert_eq!(engine.objects[first_idx].state.container, Some(new));
        assert_eq!(engine.objects[second_idx].state.container, Some(new));
        assert_eq!(engine.objects[first_idx].state.owner, 3, "Owner is unchanged");
        assert_eq!(engine.objects[second_idx].state.owner, 4, "Owner is unchanged");
    }

    // GrabContents reports whether the bulk operation itself was valid, not
    // whether every Enter succeeded (FnGrabContents returns true after the
    // void C4Object::GrabContents call, C4Script.cpp:320-327). Enter still
    // runs each child's RejectEntrance gate (C4Object.cpp:1564).
    #[test]
    fn grab_contents_keeps_rejected_children_but_still_reports_true_like_cpp() {
        let source_script = r#"#strict
public func MoveTo(pDestination) { return(pDestination->GrabContents(this())); }
public func MoveToSelf() { return(GrabContents(this())); }
"#;
        let rejected_script = r#"#strict
protected func RejectEntrance(pContainer) { return(1); }
"#;
        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(
                Definition::from_script("SRCE", "Source", source_script)
                    .expect("source compiles"),
            )
            .expect("source registers");
        engine
            .register_definition(simple_definition("DEST"))
            .expect("destination registers");
        let mut rejected =
            Definition::from_script("NOPE", "Rejected", rejected_script)
                .expect("rejected item compiles");
        rejected.set_c4_callback_convention(true);
        engine
            .register_definition(rejected)
            .expect("rejected item registers");

        let source = engine
            .spawn_object(SpawnConfig::new("SRCE").with_category(CATEGORY_OBJECT))
            .expect("source spawns");
        let destination = engine
            .spawn_object(SpawnConfig::new("DEST").with_category(CATEGORY_OBJECT))
            .expect("destination spawns");
        let rejected = engine
            .spawn_object(
                SpawnConfig::new("NOPE")
                    .with_category(CATEGORY_OBJECT)
                    .with_container(source),
            )
            .expect("rejected item spawns");

        let source_idx = engine.find_object_index(source).expect("source exists");
        let result = engine
            .call_object_function(
                source_idx,
                "MoveTo",
                vec![object_reference_value(destination)],
            )
            .expect("bulk move runs");
        assert_eq!(result, Value::Bool(true), "an individual veto is ignored");
        let rejected_idx = engine
            .find_object_index(rejected)
            .expect("rejected item exists");
        assert_eq!(engine.objects[rejected_idx].state.container, Some(source));

        let source_idx = engine.find_object_index(source).expect("source exists");
        assert_eq!(
            engine
                .call_object_function(source_idx, "MoveToSelf", Vec::new())
                .expect("self move runs"),
            Value::Bool(false),
            "pTo == from is rejected before the bulk operation"
        );
    }

    #[test]
    fn collect_and_grab_contents_transfers_run_full_exit_bounds_before_enter(
    ) -> Result<(), EngineError> {
        // Both Collect and GrabContents reach C4Object::Enter, whose transfer
        // arm calls the ordinary callback-enabled Exit(x,y). Pin the complete
        // Exit boundary here so neither script host can regress to a raw
        // containment unlink that skips BoundsCheck.
        let source_script = r#"#strict
protected func Ejection(pItem) { return(pItem->MarkEjection()); }
"#;
        let destination_script = r#"#strict
public func Take(pItem) { return(Collect(pItem)); }
public func TakeAll(pSource) { return(GrabContents(pSource)); }
protected func RejectCollect(idItem, pItem) { return(0); }
protected func Collection2(pItem) { return(pItem->MarkCollection2()); }
protected func Collection(pItem) { return(pItem->MarkCollection()); }
"#;
        let item_script = r#"#strict
local order;
local left_x, left_y, left_xdir, left_ydir, left_contained;
local top_x, top_y, top_xdir, top_ydir, top_contained;
local ejection_x, ejection_y, ejection_xdir, ejection_ydir, ejection_contained;
local collection2_x, collection2_y, collection2_xdir, collection2_ydir;
local entrance_x, entrance_y, entrance_xdir, entrance_ydir;

protected func RejectEntrance(pContainer)
{
    order = order * 10 + 1;
    return(0);
}

protected func ContactLeft()
{
    order = order * 10 + 2;
    left_x = GetX(); left_y = GetY();
    left_xdir = GetXDir(); left_ydir = GetYDir();
    left_contained = !!Contained();
    return(1);
}

protected func ContactTop()
{
    order = order * 10 + 3;
    top_x = GetX(); top_y = GetY();
    top_xdir = GetXDir(); top_ydir = GetYDir();
    top_contained = !!Contained();
    return(1);
}

public func MarkEjection()
{
    order = order * 10 + 4;
    ejection_x = GetX(); ejection_y = GetY();
    ejection_xdir = GetXDir(); ejection_ydir = GetYDir();
    ejection_contained = !!Contained();
    return(1);
}

protected func Departure(pOldContainer)
{
    order = order * 10 + 5;
    return(1);
}

public func MarkCollection2()
{
    order = order * 10 + 6;
    collection2_x = GetX(); collection2_y = GetY();
    collection2_xdir = GetXDir(); collection2_ydir = GetYDir();
    return(1);
}

protected func Entrance(pContainer)
{
    order = order * 10 + 7;
    entrance_x = GetX(); entrance_y = GetY();
    entrance_xdir = GetXDir(); entrance_ydir = GetYDir();
    return(1);
}

public func MarkCollection()
{
    order = order * 10 + 8;
    return(1);
}
"#;

        let mut source = Definition::from_script("SRCE", "Source", source_script)?;
        source.set_c4_callback_convention(true);
        let mut destination =
            Definition::from_script("DEST", "Destination", destination_script)?;
        destination.set_c4_callback_convention(true);
        destination.set_collection_rect(Some(DefinitionRect::new(-10, -10, 20, 20)));
        let mut item = Definition::from_script("ITEM", "Item", item_script)?;
        item.set_c4_callback_convention(true);
        item.set_collectible(true);
        item.set_shape_rect(Some(DefinitionRect::new(-4, -3, 8, 6)));
        item.set_border_bound(C4D_BORDER_SIDES | C4D_BORDER_TOP);
        item.set_contact_function_calls(true);

        let mut engine = Engine::with_seed(6);
        engine.set_landscape(Landscape::flat(100, 100));
        engine.register_definition(source)?;
        engine.register_definition(destination)?;
        engine.register_definition(item)?;
        let source = engine.spawn_object(
            SpawnConfig::new("SRCE").with_position(Vector2::new(-20, -20)),
        )?;
        let destination = engine.spawn_object(
            SpawnConfig::new("DEST")
                .with_position(Vector2::new(60, 70))
                .with_velocity(Vector2::new(3, -2)),
        )?;
        let collected = engine.spawn_object(
            SpawnConfig::new("ITEM").with_container(source),
        )?;
        let grabbed = engine.spawn_object(
            SpawnConfig::new("ITEM").with_container(source),
        )?;
        let inactive = engine.spawn_object(
            SpawnConfig::new("ITEM")
                .with_container(source)
                .with_status(ObjectStatus::Inactive),
        )?;
        for target in [collected, grabbed, inactive] {
            let index = engine.find_object_index(target).expect("item exists");
            engine.objects[index].set_fixed_velocity(FixedVec2::new(itofix(8), itofix(9)));
        }

        let destination_idx = engine
            .find_object_index(destination)
            .expect("destination exists");
        assert_eq!(
            engine.call_object_function(
                destination_idx,
                "Take",
                vec![object_reference_value(collected)],
            )?,
            Value::Bool(true)
        );
        let destination_idx = engine
            .find_object_index(destination)
            .expect("destination remains");
        assert_eq!(
            engine.call_object_function(
                destination_idx,
                "TakeAll",
                vec![object_reference_value(source)],
            )?,
            Value::Bool(true)
        );

        for (target, expected_order) in [
            (collected, 12_345_678),
            (grabbed, 1_234_567),
            (inactive, 1_234_567),
        ] {
            let index = engine.find_object_index(target).expect("item remains");
            let object = &engine.objects[index];
            assert_eq!(object.state.container, Some(destination));
            assert_eq!(object.state.position, Vector2::new(60, 70));
            assert_eq!(object.fixed_velocity, FixedVec2::new(itofix(3), itofix(-2)));
            let (entered_x, entered_y, entered_xdir, entered_ydir) = if target == collected {
                // Collect calls Enter with fCopyMotion=false; its callbacks
                // see Exit's clamped zero-motion state and its own tail copies
                // destination motion only after Collection/Hit.
                (4, 3, 0, 0)
            } else {
                // GrabContents uses ordinary Enter, whose CopyMotion precedes
                // Collection2 and Entrance.
                (60, 70, 30, -20)
            };
            let locals = &object.state.local_vars;
            for (name, expected) in [
                ("order", expected_order),
                ("left_x", -20),
                ("left_y", -20),
                ("left_xdir", 0),
                ("left_ydir", 90),
                ("top_x", -20),
                ("top_y", -20),
                ("top_xdir", 0),
                ("top_ydir", 0),
                ("ejection_x", 4),
                ("ejection_y", 3),
                ("ejection_xdir", 0),
                ("ejection_ydir", 0),
                ("collection2_x", entered_x),
                ("collection2_y", entered_y),
                ("collection2_xdir", entered_xdir),
                ("collection2_ydir", entered_ydir),
                ("entrance_x", entered_x),
                ("entrance_y", entered_y),
                ("entrance_xdir", entered_xdir),
                ("entrance_ydir", entered_ydir),
            ] {
                assert_eq!(locals.get(name), Some(&Value::Int(expected)), "{name}");
            }
            for name in ["left_contained", "top_contained", "ejection_contained"] {
                assert_eq!(locals.get(name), Some(&Value::Bool(false)), "{name}");
            }
        }
        let inactive_idx = engine
            .find_object_index(inactive)
            .expect("inactive item remains");
        assert_eq!(
            engine.objects[inactive_idx].state.status,
            ObjectStatus::Inactive,
            "raw nonzero Status remains eligible for GrabContents -> Enter"
        );
        Ok(())
    }
