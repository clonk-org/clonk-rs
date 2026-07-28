// Contiguous slice 10 of 11 of the `compat::tests` battery, spliced by
// `include!` from compat.rs so every test id stays `compat::tests::*`.
// Mostly: players, object state, objects.

    #[test]
    fn assign_death_clears_preexisting_authoritative_command_queue() {
        // AssignDeath calls ClearCommands before contents, player pointers, and Death
        // (oracle-src-pinned src/C4Object.cpp:1180-1200,3873-3884).
        let mut definition = crate::Definition::from_script(
            "QDED",
            "Queued death",
            r#"#strict
public func Trigger()
{
    return Kill(this, true);
}
"#,
        )
        .expect("queued-death script compiles");
        definition.configure_actions(
            Some("Idle".to_string()),
            HashMap::from([
                ("Idle".to_string(), crate::ActionSpec::default()),
                ("Dead".to_string(), crate::ActionSpec::default()),
            ]),
        );

        let mut engine = crate::Engine::with_seed(0);
        engine
            .register_definition(definition)
            .expect("queued-death definition registers");
        let target = engine
            .spawn_object(SpawnConfig::new("QDED").with_alive(true))
            .expect("queued-death object spawns");
        let index = engine
            .find_object_index(target)
            .expect("queued-death object exists");
        engine.objects[index]
            .command_queue
            .push_back(QueuedCommand::new(7, ObjectUpdate::default()));

        assert_eq!(engine.objects[index].command_queue.len(), 1);
        assert_eq!(
            engine
                .call_object_function(index, "Trigger", Vec::new())
                .expect("queued-death trigger succeeds"),
            Value::Bool(true)
        );
        assert!(
            engine.objects[index].command_queue.is_empty(),
            "AssignDeath's ClearCommands clears commands already queued before Kill"
        );
    }

    #[test]
    fn foreign_death_callback_set_owner_rehomes_retained_fow_view() {
        // AssignDeath captures pPlr before ClearPointers, performs the
        // living/FoW retention test before Death, and then Death may change
        // Owner while the retained nonzero range remains. The foreign-object
        // copy-out must therefore move the FoW link to the new owner even
        // though PlrViewRange itself did not change
        // (oracle-src-pinned src/C4Object.cpp:1193-1204,5493-5522).
        let mut target_definition = crate::Definition::from_script(
            "FOWD",
            "FoW death",
            r#"#strict
protected func Death()
{
    SetOwner(1);
}
"#,
        )
        .expect("FoW-death script compiles");
        target_definition.set_category(crate::CATEGORY_OBJECT | crate::CATEGORY_LIVING);
        target_definition.configure_actions(
            Some("Idle".to_string()),
            HashMap::from([
                ("Idle".to_string(), crate::ActionSpec::default()),
                ("Dead".to_string(), crate::ActionSpec::default()),
            ]),
        );
        let caller_definition = crate::Definition::from_script(
            "FOWC",
            "FoW caller",
            r#"#strict
public func Trigger(target)
{
    return Kill(target, true);
}
"#,
        )
        .expect("FoW caller script compiles");

        let mut engine = crate::Engine::with_seed(0);
        for player in [0, 1] {
            engine
                .register_player(crate::PlayerConfig::new(player, format!("Player {player}")))
                .expect("FoW player registers");
        }
        engine
            .register_definition(target_definition)
            .expect("FoW-death definition registers");
        engine
            .register_definition(caller_definition)
            .expect("FoW caller definition registers");
        let target = engine
            .spawn_object(
                SpawnConfig::new("FOWD")
                    .with_owner(0)
                    .with_alive(true)
                    .with_plr_view_range(500),
            )
            .expect("FoW-death target spawns");
        let caller = engine
            .spawn_object(SpawnConfig::new("FOWC"))
            .expect("FoW caller spawns");
        assert!(engine
            .player(0)
            .is_some_and(|player| player.has_fow_view_object(target)));
        assert!(!engine
            .player(1)
            .is_some_and(|player| player.has_fow_view_object(target)));

        let caller_index = engine.find_object_index(caller).expect("FoW caller exists");
        assert_eq!(
            engine
                .call_object_function(
                    caller_index,
                    "Trigger",
                    vec![object_reference_value(target)],
                )
                .expect("foreign FoW death succeeds"),
            Value::Bool(true)
        );
        let target_state = engine
            .object_snapshot(target)
            .expect("dead FoW target remains allocated");
        assert_eq!(target_state.owner, 1);
        assert_eq!(
            target_state.plr_view_range, 500,
            "the original owner's live FoW link retains the dead living range before Death"
        );
        assert!(
            !engine
                .player(0)
                .is_some_and(|player| player.has_fow_view_object(target)),
            "Death's SetOwner removes the retained link from the original owner"
        );
        assert!(
            engine
                .player(1)
                .is_some_and(|player| player.has_fow_view_object(target)),
            "Death's SetOwner installs the retained link on the new owner"
        );
    }

    #[test]
    fn assign_death_cursor_replacement_resets_view_in_cpp_order_for_both_paths() {
        // Death ClearPointers removes Crew before AdjustCursorCommand resets/follows
        // the old view cursor, then clears ViewCursor and ViewTarget
        // (oracle-src-pinned src/C4Player.cpp:57-77,923-928,
        // 1235-1259,1692-1716; src/C4Object.cpp:1193-1195).
        fn run(foreign_vm_path: bool) -> (crate::Engine, ObjectId, ObjectId) {
            let mut crew_definition =
                crate::Definition::from_script("VCRE", "View crew", "#strict")
                    .expect("view-crew script compiles");
            crew_definition.set_category(crate::CATEGORY_OBJECT | crate::CATEGORY_LIVING);
            crew_definition.set_crew_member(true);
            crew_definition.configure_actions(
                Some("Idle".to_string()),
                HashMap::from([
                    ("Idle".to_string(), crate::ActionSpec::default()),
                    ("Dead".to_string(), crate::ActionSpec::default()),
                ]),
            );
            let caller_definition = crate::Definition::from_script(
                "VCRL",
                "View caller",
                r#"#strict
public func Trigger(target)
{
    return Kill(target, true);
}
"#,
            )
            .expect("view caller script compiles");

            let mut engine = crate::Engine::with_seed(0);
            engine
                .register_player(crate::PlayerConfig::new(0, "Player"))
                .expect("view player registers");
            engine
                .register_definition(crew_definition)
                .expect("view-crew definition registers");
            engine
                .register_definition(caller_definition)
                .expect("view caller definition registers");
            let dying = engine
                .spawn_object(
                    SpawnConfig::new("VCRE")
                        .with_owner(0)
                        .with_alive(true)
                        .with_crew_member(true)
                        .with_position(Vector2::new(40, 50)),
                )
                .expect("dying view crew spawns");
            let replacement = engine
                .spawn_object(
                    SpawnConfig::new("VCRE")
                        .with_owner(0)
                        .with_alive(true)
                        .with_crew_member(true)
                        .with_position(Vector2::new(140, 150)),
                )
                .expect("replacement view crew spawns");
            engine
                .select_crew(0, [dying, replacement])
                .expect("view crew selects");
            engine
                .set_crew_cursor(0, Some(dying))
                .expect("dying crew becomes cursor");
            {
                let player = engine.player_mut(0).expect("view player remains");
                player.set_view_cursor(Some(dying));
                player.set_view_target(Some(replacement));
                player.set_view_center(Vector2::new(1, 2));
                player.replace_viewports(vec![
                    PlayerViewport::new(Vector2::new(1, 2)).with_focus(Some(replacement))
                ]);
            }

            if foreign_vm_path {
                let caller = engine
                    .spawn_object(SpawnConfig::new("VCRL"))
                    .expect("view caller spawns");
                let caller_index = engine
                    .find_object_index(caller)
                    .expect("view caller exists");
                assert_eq!(
                    engine
                        .call_object_function(
                            caller_index,
                            "Trigger",
                            vec![object_reference_value(dying)],
                        )
                        .expect("foreign view death succeeds"),
                    Value::Bool(true)
                );
            } else {
                let dying_index = engine.find_object_index(dying).expect("dying crew exists");
                engine
                    .assign_death(dying_index, true)
                    .expect("direct view death succeeds");
            }
            (engine, dying, replacement)
        }

        for foreign_vm_path in [false, true] {
            let (engine, dying, replacement) = run(foreign_vm_path);
            let player = engine.player(0).expect("view player survives");
            assert_eq!(
                player.cursor(),
                Some(replacement),
                "AdjustCursorCommand selects the remaining crew"
            );
            assert_eq!(player.view_cursor(), None);
            assert_eq!(
                player.raw_view_mode(),
                crate::PLAYER_VIEW_MODE_CURSOR,
                "ResetCursorView runs before the dying ViewCursor is cleared"
            );
            assert_eq!(player.raw_view_target(), None);
            assert_eq!(
                player.view_center(),
                Vector2::new(40, 50),
                "UpdateView still follows the old ViewCursor at its native call point"
            );
            assert_eq!(player.viewports()[0].center, Vector2::new(40, 50));
            assert_eq!(
                player.viewports()[0].focus,
                Some(replacement),
                "the ClearPointers suffix retargets presentation focus after clearing ViewCursor"
            );
            assert!(
                !player.crew().contains(&dying),
                "the dying cursor leaves Crew before the replacement search"
            );
        }
    }

    #[test]
    fn assign_death_skips_death_callback_after_remove_death_destroys_target() {
        // RemoveDeath may AssignRemoval and zero Status; AssignDeath continues, but
        // C4Object::Call suppresses Death for a status-zero object
        // (oracle-src-pinned src/C4Effect.cpp:407-425;
        // src/C4Object.cpp:240-320,1164-1205,2224-2227).
        let stop_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let death_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed_stops = Arc::clone(&stop_calls);
        let observed_deaths = Arc::clone(&death_calls);
        let mut target_definition = crate::Definition::from_script(
            "TARG",
            "Target",
            r#"#strict
public func Setup()
{
    AddEffect("Vanish", this, 1, 0, this);
}
protected func FxVanishStop(target, number, reason)
{
    if (reason == 4) RemoveObject(target);
    return 0;
}
protected func Death()
{
    return 0;
}
"#,
        )
        .expect("removal target script compiles");
        target_definition.set_debugger_hooks(clonk_script::DebuggerHooks::new().with_on_call(
            move |name, _| match name {
                "FxVanishStop" => {
                    observed_stops.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
                "Death" => {
                    observed_deaths.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
                _ => {}
            },
        ));
        let caller_definition = crate::Definition::from_script(
            "CALL",
            "Caller",
            r#"#strict
public func Trigger(target)
{
    return Kill(target, true);
}
"#,
        )
        .expect("removal caller script compiles");

        let mut engine = crate::Engine::with_seed(0);
        engine
            .register_definition(target_definition)
            .expect("removal target registers");
        engine
            .register_definition(caller_definition)
            .expect("removal caller registers");
        let target = engine
            .spawn_object(SpawnConfig::new("TARG").with_alive(true))
            .expect("removal target spawns");
        let caller = engine
            .spawn_object(SpawnConfig::new("CALL"))
            .expect("removal caller spawns");
        let target_index = engine.find_object_index(target).expect("target exists");
        engine
            .call_object_function(target_index, "Setup", Vec::new())
            .expect("death-removal effect installs");
        let caller_index = engine.find_object_index(caller).expect("caller exists");

        assert_eq!(
            engine
                .call_object_function(
                    caller_index,
                    "Trigger",
                    vec![object_reference_value(target)],
                )
                .expect("foreign forced Kill returns"),
            Value::Bool(true)
        );
        assert!(
            engine.objects[target_index].destroyed,
            "the RemoveDeath callback's AssignRemoval folds immediately"
        );
        assert_eq!(
            engine
                .object_snapshot(target)
                .expect("the removed C++ object remains allocated as a tombstone")
                .status,
            ObjectStatus::Deleted,
            "AssignRemoval clears raw Status before the Kill call resumes"
        );
        assert_eq!(stop_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(
            death_calls.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "C4Object::Call is a no-op once raw Status is zero"
        );
    }

    #[test]
    fn assign_death_retries_live_contents_head_after_exit_reports_reentry() {
        // AssignDeath re-reads Contents.GetObject after every Exit; Exit reports
        // re-entry through its Ejection callback before returning
        // (oracle-src-pinned src/C4Object.cpp:1191-1192,1532-1563).
        let mut container_definition = crate::Definition::from_script(
            "CONT",
            "Container",
            r#"#strict
local ejections, death_ejections;
public func Trigger()
{
    Kill(this, true);
}
protected func Ejection(item)
{
    ejections++;
    if (ejections == 1) Enter(this, item);
}
protected func Death()
{
    death_ejections = ejections;
}
"#,
        )
        .expect("contents retry script compiles");
        container_definition.configure_actions(
            Some("Idle".to_string()),
            HashMap::from([
                ("Idle".to_string(), crate::ActionSpec::default()),
                ("Dead".to_string(), crate::ActionSpec::default()),
            ]),
        );
        let item_definition = crate::Definition::from_script("ITEM", "Item", "#strict")
            .expect("contents item compiles");

        let mut engine = crate::Engine::with_seed(0);
        engine
            .register_definition(container_definition)
            .expect("contents container registers");
        engine
            .register_definition(item_definition)
            .expect("contents item registers");
        let container = engine
            .spawn_object(SpawnConfig::new("CONT").with_alive(true))
            .expect("contents container spawns");
        let item = engine
            .spawn_object(SpawnConfig::new("ITEM").with_container(container))
            .expect("contained item spawns");
        let container_index = engine
            .find_object_index(container)
            .expect("contents container exists");

        engine
            .call_object_function(container_index, "Trigger", Vec::new())
            .expect("contents death succeeds");

        let container_state = engine
            .object_snapshot(container)
            .expect("dead container remains");
        assert_eq!(
            container_state.local_vars.get("ejections"),
            Some(&Value::Int(2)),
            "AssignDeath retries the re-entered live head"
        );
        assert_eq!(
            container_state.local_vars.get("death_ejections"),
            Some(&Value::Int(2)),
            "Death runs only after the contents list is empty"
        );
        assert_eq!(
            engine
                .object_snapshot(item)
                .expect("item remains")
                .container,
            None
        );
    }

    #[test]
    fn assign_death_replays_same_key_reentries_in_enter_order() {
        // AssignDeath re-reads the live first contents link after every Exit
        // (oracle-src-pinned src/C4Object.cpp:1191-1192).
        // C4ObjectList::Add(stContents) inserts
        // each re-entry before the first matching category/id link at the
        // instant Enter runs (src/C4ObjectList.cpp:147-175), so entering B then
        // A yields the later A as the next object to eject.
        let mut container_definition = crate::Definition::from_script(
            "CONT",
            "Container",
            r#"#strict
local seed, first, second, ejection_order;
public func Trigger(object a, object b, object marker)
{
    first = a;
    second = b;
    seed = marker;
    a->SetTag(1);
    b->SetTag(2);
    a->Prime();
    b->Prime();
    Kill(this, true);
    return ejection_order;
}
protected func Ejection(object item)
{
    if (item == seed)
    {
        Enter(this, second);
        Enter(this, first);
    }
    else
    {
        ejection_order = ejection_order * 10 + item->Tag();
    }
}
"#,
        )
        .expect("ordered contents script compiles");
        container_definition.configure_actions(
            Some("Idle".to_string()),
            HashMap::from([
                ("Idle".to_string(), crate::ActionSpec::default()),
                ("Dead".to_string(), crate::ActionSpec::default()),
            ]),
        );
        let item_definition = crate::Definition::from_script(
            "ITEM",
            "Item",
            r#"#strict
local tag;
public func SetTag(int value) { tag = value; }
public func Prime() { return tag; }
public func Tag() { return tag; }
"#,
        )
        .expect("ordered item script compiles");
        let seed_definition = crate::Definition::from_script("SEED", "Seed", "#strict")
            .expect("ordered seed script compiles");

        let mut engine = crate::Engine::with_seed(0);
        engine
            .register_definition(container_definition)
            .expect("ordered container registers");
        engine
            .register_definition(item_definition)
            .expect("ordered item registers");
        engine
            .register_definition(seed_definition)
            .expect("ordered seed registers");
        let container = engine
            .spawn_object(SpawnConfig::new("CONT").with_alive(true))
            .expect("ordered container spawns");
        let first = engine
            .spawn_object(SpawnConfig::new("ITEM"))
            .expect("first ordered item spawns");
        let second = engine
            .spawn_object(SpawnConfig::new("ITEM"))
            .expect("second ordered item spawns");
        let seed = engine
            .spawn_object(SpawnConfig::new("SEED").with_container(container))
            .expect("ordered seed contents spawn");
        let container_index = engine
            .find_object_index(container)
            .expect("ordered container exists");

        assert_eq!(
            engine
                .call_object_function(
                    container_index,
                    "Trigger",
                    vec![
                        object_reference_value(first),
                        object_reference_value(second),
                        object_reference_value(seed),
                    ],
                )
                .expect("ordered contents death succeeds"),
            Value::Int(12),
            "B then A re-entry must eject A then B like stContents"
        );
    }

    #[test]
    fn assign_death_does_not_reapply_an_old_contents_rotation_after_reentry() {
        // ScrollContents performs one raw remove-and-append
        // (oracle-src-pinned src/C4Script.cpp:1793-1804).
        // If that new front later exits and
        // re-enters, C4ObjectList::Add(stContents) chooses a fresh sorted
        // position (src/C4ObjectList.cpp:147-175); the old rotation is not a
        // persistent preference. AssignDeath observes that new head on its
        // next Contents.GetObject() (src/C4Object.cpp:1191-1192).
        let mut container_definition = crate::Definition::from_script(
            "CONT",
            "Container",
            r#"#strict
local low, reentered, ejection_order;
public func Trigger(object low_item)
{
    low = low_item;
    ScrollContents();
    Kill(this, true);
    return ejection_order;
}
protected func Ejection(object item)
{
    ejection_order = ejection_order * 10 + item->Tag();
    if (item == low && !reentered)
    {
        reentered = true;
        Enter(this, item);
    }
}
"#,
        )
        .expect("rotated contents script compiles");
        container_definition.configure_actions(
            Some("Idle".to_string()),
            HashMap::from([
                ("Idle".to_string(), crate::ActionSpec::default()),
                ("Dead".to_string(), crate::ActionSpec::default()),
            ]),
        );
        let mut high_definition = crate::Definition::from_script(
            "HIGH",
            "High",
            "#strict\npublic func Tag() { return 2; }",
        )
        .expect("high contents script compiles");
        high_definition.set_category(crate::CATEGORY_OBJECT);
        let mut low_definition = crate::Definition::from_script(
            "LOW",
            "Low",
            "#strict\npublic func Tag() { return 1; }",
        )
        .expect("low contents script compiles");
        low_definition.set_category(crate::CATEGORY_VEHICLE);

        let mut engine = crate::Engine::with_seed(0);
        engine
            .register_definition(container_definition)
            .expect("rotated container registers");
        engine
            .register_definition(high_definition)
            .expect("high contents register");
        engine
            .register_definition(low_definition)
            .expect("low contents register");
        let container = engine
            .spawn_object(SpawnConfig::new("CONT").with_alive(true))
            .expect("rotated container spawns");
        let low = engine
            .spawn_object(SpawnConfig::new("LOW").with_container(container))
            .expect("low contents spawn");
        let high = engine
            .spawn_object(SpawnConfig::new("HIGH").with_container(container))
            .expect("high contents spawn");
        let container_index = engine
            .find_object_index(container)
            .expect("rotated container exists");
        assert_eq!(
            engine.objects[container_index].state.contents,
            vec![high, low],
            "fixture begins in stContents category order"
        );

        assert_eq!(
            engine
                .call_object_function(
                    container_index,
                    "Trigger",
                    vec![object_reference_value(low)],
                )
                .expect("rotated contents death succeeds"),
            Value::Int(121),
            "the re-entered low item sorts behind high before its final Exit"
        );
    }

    #[test]
    fn death_callback_set_alive_survives_assign_deaths_final_set_ocf() {
        // Death runs before AssignDeath's final SetOCF, so Death's SetAlive refresh
        // remains visible after the lifecycle returns
        // (oracle-src-pinned src/C4Object.cpp:1199-1204;
        // src/C4Object.h:361; src/C4Script.cpp:814-818).
        let death_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed_deaths = Arc::clone(&death_calls);
        let mut definition = crate::Definition::from_script(
            "CLNK",
            "Clonk",
            r#"#strict
public func Trigger()
{
    Kill(this, true);
    return GetAlive();
}
protected func Death()
{
    SetAlive(true);
    return 0;
}
"#,
        )
        .expect("Death-revival script compiles");
        definition.set_category(crate::CATEGORY_OBJECT | crate::CATEGORY_LIVING);
        definition.set_crew_member(true);
        definition.configure_actions(
            Some("Idle".to_string()),
            HashMap::from([
                ("Idle".to_string(), crate::ActionSpec::default()),
                ("Dead".to_string(), crate::ActionSpec::default()),
            ]),
        );
        definition.set_debugger_hooks(clonk_script::DebuggerHooks::new().with_on_call(
            move |name, _| {
                if name == "Death" {
                    observed_deaths.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
            },
        ));

        let mut engine = crate::Engine::with_seed(0);
        engine
            .register_definition(definition)
            .expect("Death-revival definition registers");
        let clonk = engine
            .spawn_object(SpawnConfig::new("CLNK"))
            .expect("clonk spawns");
        let index = engine.find_object_index(clonk).expect("clonk exists");
        engine.objects[index].state.alive = true;
        engine.refresh_object_ocf(index);

        assert_eq!(
            engine
                .call_object_function(index, "Trigger", Vec::new())
                .expect("forced Kill returns after Death revival"),
            Value::Bool(true),
            "Death's SetAlive is visible after AssignDeath returns"
        );
        let object = &engine.objects[index];
        assert!(object.state.alive);
        assert_eq!(object.state.action.name, "Dead");
        assert!(
            !object.state.crew_member,
            "the owner roster projection is cleared independently"
        );
        assert_ne!(
            object.state.ocf & crate::ocf::ALIVE,
            0,
            "AssignDeath's final SetOCF acknowledges the callback's revival"
        );
        assert_ne!(
            object.state.ocf & crate::ocf::CREW_MEMBER,
            0,
            "revived OCF uses the definition CrewMember capability, not the cleared roster bit"
        );
        assert_eq!(
            death_calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "the revived object is not killed again during copy-out"
        );
    }

    #[test]
    fn foreign_death_revival_uses_definition_crew_capability_not_runtime_roster() {
        // C4Object::SetOCF reads Def->CrewMember, not whether any player's
        // Crew list still contains the object
        // (oracle-src-pinned src/C4Object.cpp:622-624,1193-1204).
        // AssignDeath clears the runtime roster before Death, and a foreign
        // target executes through a freshly materialized nested scope.
        let mut target_definition = crate::Definition::from_script(
            "CLNK",
            "Foreign revived Clonk",
            r#"#strict
protected func Death()
{
    SetAlive(true);
    return 0;
}
"#,
        )
        .expect("foreign revival target compiles");
        target_definition.set_category(crate::CATEGORY_OBJECT | crate::CATEGORY_LIVING);
        target_definition.set_crew_member(true);
        target_definition.configure_actions(
            Some("Idle".to_string()),
            HashMap::from([
                ("Idle".to_string(), crate::ActionSpec::default()),
                ("Dead".to_string(), crate::ActionSpec::default()),
            ]),
        );
        let caller_definition = crate::Definition::from_script(
            "CALL",
            "Foreign revival caller",
            r#"#strict
public func Trigger(target)
{
    Kill(target, true);
    return [GetAlive(target), GetOCF(target) & OCF_CrewMember];
}
"#,
        )
        .expect("foreign revival caller compiles");

        let mut engine = crate::Engine::with_seed(0);
        engine
            .register_definition(target_definition)
            .expect("foreign revival target registers");
        engine
            .register_definition(caller_definition)
            .expect("foreign revival caller registers");
        let caller = engine
            .spawn_object(SpawnConfig::new("CALL"))
            .expect("foreign revival caller spawns");
        let target = engine
            .spawn_object(
                SpawnConfig::new("CLNK")
                    .with_alive(true)
                    .with_crew_member(false),
            )
            .expect("foreign revival target spawns outside every runtime roster");
        let caller_index = engine
            .find_object_index(caller)
            .expect("foreign revival caller exists");

        assert_eq!(
            engine
                .call_object_function(
                    caller_index,
                    "Trigger",
                    vec![object_reference_value(target)],
                )
                .expect("foreign forced Kill returns after revival"),
            Value::Array(vec![
                Value::Bool(true),
                Value::Int(crate::ocf::CREW_MEMBER as i32),
            ]),
            "the caller immediately observes final SetOCF using Def->CrewMember"
        );
        let target_index = engine
            .find_object_index(target)
            .expect("foreign revived target remains");
        let object = &engine.objects[target_index];
        assert!(object.state.alive);
        assert!(
            !object.state.crew_member,
            "Death revival does not implicitly restore a player Crew link"
        );
        assert_ne!(
            object.state.ocf & crate::ocf::CREW_MEMBER,
            0,
            "runtime roster state must not replace the definition capability"
        );
    }

    #[test]
    fn assign_death_callback_recruit_actualizes_nonzero_fow_range_before_retention() {
        // AssignDeath tests the captured owner's live FoWViewObjs only after
        // ClearPointers has selected the replacement crew. That replacement
        // may re-recruit the dying object; MakeCrewMember must run its
        // nonzero-range PlrFoWActualize arm before AssignDeath performs the
        // retention test (oracle-src-pinned src/C4Object.cpp:1193-1198;
        // src/C4Player.cpp:1194-1199).
        let mut definition = crate::Definition::from_script(
            "CLNK",
            "Death recruitment",
            r#"#strict
local victim, death_view_range;
public func SetVictim(target) { victim = target; }
public func Trigger() { Kill(this, true); }
protected func CrewSelection(unselect, cursor)
{
    if (!unselect && !cursor && victim)
        SetCrewStatus(0, true, victim);
}
protected func Death()
{
    death_view_range = GetObjectVal("PlrViewRange", 0, this());
}
"#,
        )
        .expect("death-recruitment definition compiles");
        definition.set_category(crate::CATEGORY_OBJECT | crate::CATEGORY_LIVING);
        definition.set_crew_member(true);
        definition.configure_actions(
            Some("Idle".to_string()),
            HashMap::from([
                ("Idle".to_string(), crate::ActionSpec::default()),
                ("Dead".to_string(), crate::ActionSpec::default()),
            ]),
        );

        let mut engine = crate::Engine::with_seed(0);
        engine
            .register_player(crate::PlayerConfig::new(0, "Owner"))
            .expect("death-recruitment owner registers");
        engine
            .register_definition(definition)
            .expect("death-recruitment definition registers");
        let target = engine
            .spawn_object(
                SpawnConfig::new("CLNK")
                    .with_owner(0)
                    .with_alive(true)
                    .with_crew_member(true)
                    .with_plr_view_range(333),
            )
            .expect("dying crew target spawns");
        let replacement = engine
            .spawn_object(
                SpawnConfig::new("CLNK")
                    .with_owner(0)
                    .with_alive(true)
                    .with_crew_member(true),
            )
            .expect("replacement crew spawns");
        let replacement_index = engine
            .find_object_index(replacement)
            .expect("replacement crew exists");
        engine
            .call_object_function(
                replacement_index,
                "SetVictim",
                vec![object_reference_value(target)],
            )
            .expect("replacement remembers the dying target");
        engine
            .set_crew_cursor(0, Some(target))
            .expect("dying target becomes cursor");

        // Model a live nonzero PlrViewRange whose runtime FoW link was
        // cleared earlier. The replacement's recruitment must restore it
        // synchronously, before AssignDeath decides whether to zero range.
        engine
            .players
            .get_mut(&0)
            .expect("owner exists")
            .remove_fow_view_object(target);
        assert!(!engine
            .player(0)
            .expect("owner remains")
            .has_fow_view_object(target));

        let target_index = engine
            .find_object_index(target)
            .expect("dying target exists");
        engine
            .call_object_function(target_index, "Trigger", Vec::new())
            .expect("forced death completes");

        let target_state = engine
            .object_snapshot(target)
            .expect("dead recruited target remains");
        assert_eq!(target_state.plr_view_range, 333);
        assert_eq!(
            target_state.local_vars.get("death_view_range"),
            Some(&Value::Int(333)),
            "Death observes the retained range after callback recruitment"
        );
        assert!(
            engine
                .player(0)
                .expect("owner remains")
                .has_fow_view_object(target),
            "nonzero-range MakeCrewMember re-adds the live FoW link"
        );
        assert!(
            engine
                .player(0)
                .expect("owner remains")
                .crew()
                .contains(&target),
            "the cursor-replacement callback re-recruits the dying target"
        );
    }

    #[test]
    fn physicals_host_fns_follow_cpp_mode_semantics() {
        // SetPhysical/GetPhysical/TrainPhysical/ResetPhysical
        // (C4Script.cpp:552-688) on a non-crew object with all-zero
        // definition physicals.
        let (result, outcome) = with_object_host_context(|| {
            let walk = || Value::String("Walk".to_string().into());
            // TrainPhysical with neither temp mode nor info trains nothing
            // (C4Object.cpp:2136-2146).
            assert_eq!(
                train_physical(&[walk(), Value::Int(5), Value::Int(C4_MAX_PHYSICAL)])?,
                Value::Bool(false)
            );
            // Unknown physical name fails (C4Script.cpp:562).
            assert_eq!(
                set_physical(&[
                    Value::String("Bogus".to_string().into()),
                    Value::Int(1),
                    Value::Int(2)
                ])?,
                Value::Bool(false)
            );
            // PHYS_Current needs temp mode or an info (C4Script.cpp:569).
            assert_eq!(
                set_physical(&[walk(), Value::Int(1), Value::Int(0)])?,
                Value::Bool(false)
            );
            // PHYS_Permanent needs an info (C4Script.cpp:576).
            assert_eq!(
                set_physical(&[walk(), Value::Int(1), Value::Int(1)])?,
                Value::Bool(false)
            );
            // PHYS_Temporary reads need an info too (C4Script.cpp:680).
            assert_eq!(get_physical(&[walk(), Value::Int(2)])?, Value::Nil);
            // PHYS_Temporary write auto-enables temp mode
            // (C4Script.cpp:587-596).
            assert_eq!(
                set_physical(&[walk(), Value::Int(50_000), Value::Int(2)])?,
                Value::Bool(true)
            );
            assert_eq!(get_physical(&[walk(), Value::Int(0)])?, Value::Int(50_000));
            // PHYS_Current works while temp mode is on (C4Script.cpp:567-572).
            assert_eq!(
                set_physical(&[walk(), Value::Int(60_000), Value::Int(0)])?,
                Value::Bool(true)
            );
            // PHYS_StackTemporary registers the previous value
            // (C4Script.cpp:593-596).
            assert_eq!(
                set_physical(&[walk(), Value::Int(70_000), Value::Int(3)])?,
                Value::Bool(true)
            );
            // Training in temp mode trains the active value AND the stacked
            // previous one (C4InfoCore.cpp:309-317).
            assert_eq!(
                train_physical(&[walk(), Value::Int(5), Value::Int(C4_MAX_PHYSICAL)])?,
                Value::Bool(true)
            );
            assert_eq!(get_physical(&[walk(), Value::Int(0)])?, Value::Int(70_005));
            // Named reset restores the last stacked value
            // (C4Script.cpp:622-629; C4InfoCore.cpp:339-351) and keeps temp
            // mode because the set still deviates from the reference.
            assert_eq!(reset_physical(&[Value::Nil, walk()])?, Value::Bool(true));
            assert_eq!(get_physical(&[walk(), Value::Int(0)])?, Value::Int(60_005));
            // Full reset drops temp mode (C4Script.cpp:631-635)...
            assert_eq!(reset_physical(&[])?, Value::Bool(true));
            assert_eq!(get_physical(&[walk(), Value::Int(0)])?, Value::Int(0));
            // ...and resetting without temp mode fails (C4Script.cpp:619).
            assert_eq!(reset_physical(&[])?, Value::Bool(false));
            Ok(Value::Nil)
        });
        result.expect("physicals host fns run");
        // The scope records the final physical state for the engine — the
        // cleared temp mode must overwrite any prior engine-side state.
        let update = outcome.object_update.expect("physicals update recorded");
        let physicals = update.physicals.expect("physicals state recorded");
        assert_eq!(physicals.info, None);
        assert_eq!(physicals.temporary, None);
        assert_eq!(physicals.changes, Vec::<(String, i32)>::new());
    }

    #[test]
    fn get_physical_definition_form_uses_native_c4id_conversion_and_object_precedence() {
        let definition_magic = 80_000;
        let numeric_definition_magic = 42_000;
        let world = HostWorldContext::default().with_definition_metadata(Rc::new(HashMap::from([
            (
                DefinitionId::from("MAGE"),
                DefinitionMetadata {
                    physical: PhysicalInfo {
                        magic: definition_magic,
                        ..PhysicalInfo::default()
                    },
                    ..DefinitionMetadata::default()
                },
            ),
            (
                DefinitionId::from("0042"),
                DefinitionMetadata {
                    physical: PhysicalInfo {
                        magic: numeric_definition_magic,
                        ..PhysicalInfo::default()
                    },
                    ..DefinitionMetadata::default()
                },
            ),
        ])));

        let (result, _) = with_object_host_context_with_world(world, || {
            let magic = || Value::String("Magic".to_string().into());
            // Give the caller a temporary physical set, then lower its live
            // Magic to zero. GetPhysical(..., nil, GetID()) must still read
            // the raw definition value (C4Script.cpp:644-653).
            assert_eq!(
                set_physical(&[magic(), Value::Int(50_000), Value::Int(PHYS_TEMPORARY),])?,
                Value::Bool(true)
            );
            assert_eq!(
                set_physical(&[magic(), Value::Int(0), Value::Int(PHYS_CURRENT)])?,
                Value::Bool(true)
            );

            for (definition, expected) in [
                (Value::C4Id("MAGE".to_string()), definition_magic),
                (Value::Int(42), numeric_definition_magic),
            ] {
                for target in [Value::Nil, Value::Int(0)] {
                    assert_eq!(
                        get_physical(&[
                            magic(),
                            Value::Int(PHYS_CURRENT),
                            target,
                            definition.clone(),
                        ])?,
                        Value::Int(expected)
                    );
                }
            }

            // `idDef` is a native C4ID parameter. String -> C4ID is always
            // invalid and FnCnvInt2Id accepts only 0..=9999; an integer that
            // merely contains the packed MAGE bytes is therefore rejected
            // before FnGetPhysical executes (C4Value.cpp:469-478,550-561).
            for rejected in [
                Value::String("MAGE".to_string().into()),
                Value::Int(i32::from_le_bytes(*b"MAGE")),
                Value::Bool(true),
            ] {
                let error =
                    get_physical(&[magic(), Value::Int(PHYS_CURRENT), Value::Nil, rejected])
                        .expect_err("invalid native C4ID conversion must fail");
                assert!(error.message().contains("expected C4ID"));
            }

            // A real object argument wins over the conflicting definition id.
            assert_eq!(
                get_physical(&[
                    magic(),
                    Value::Int(PHYS_CURRENT),
                    object_reference_value(ObjectId::new(1)),
                    Value::C4Id("MAGE".to_string()),
                ])?,
                Value::Int(0)
            );
            Ok(Value::Nil)
        });

        result.expect("definition physical reads succeed");
    }

    #[test]
    fn get_physical_reads_an_explicit_foreign_crew_object() {
        // FnGetPhysical dereferences its explicit pObj without requiring it
        // to equal cthr->Obj (C4Script.cpp:638-688). CNKT::Activate relies on
        // this exact form to check the containing clonk's CanConstruct before
        // opening CXCN (Objects/.../Conkit.c4d/Script.c:5-21).
        let clonk = ObjectId::new(2);
        let physical = PhysicalInfo {
            can_construct: 1,
            ..PhysicalInfo::default()
        };
        let mut state = crate::preview_spawn_state(
            Vector2::ZERO,
            OWNER_NONE,
            OWNER_NONE,
            crate::CATEGORY_LIVING,
            FULL_CON,
            crate::CONTACT_DENSITY_SOLID,
            Vec::new(),
        );
        state.crew_member = true;
        state.info_physical = Some(physical);
        let world = HostWorldContext::from_objects([HostWorldObject::new(
            clonk,
            "CLNK",
            ObjectStatus::Normal,
            "Walk",
            None,
            None,
            Some("WALK".to_string()),
            OWNER_NONE,
            100,
            FULL_CON,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            None,
        )
        .with_full_state(Rc::new(state))])
        .with_definition_metadata(Rc::new(HashMap::from([(
            DefinitionId::from("CLNK"),
            DefinitionMetadata {
                crew_member: true,
                physical,
                ..DefinitionMetadata::default()
            },
        )])));

        let (result, _) = with_object_host_context_with_world(world, || {
            get_physical(&[
                Value::String("CanConstruct".to_string().into()),
                Value::Int(PHYS_CURRENT),
                object_reference_value(clonk),
            ])
        });

        assert_eq!(
            result.expect("foreign physical read succeeds"),
            Value::Int(1)
        );
    }

    #[test]
    fn global_set_and_train_physical_target_a_foreign_object_like_cpp() {
        // Definition-owned Fx* callbacks execute with cthr->Obj == null but
        // pass their carrier explicitly to SetPhysical/TrainPhysical. C++
        // mutates that object directly (C4Script.cpp:557-611).
        let mut engine = crate::Engine::with_seed(0);
        engine
            .register_definition(
                crate::Definition::from_script("CLNK", "Clonk", "#strict\n")
                    .expect("definition compiles"),
            )
            .expect("definition registers");
        let clonk = engine
            .spawn_object(crate::SpawnConfig::new("CLNK"))
            .expect("clonk spawns");
        let target = object_reference_value(clonk);

        let (result, outcome) = with_effect_context(
            None,
            &[],
            engine.host_world_context(),
            2,
            || -> Result<Value, RuntimeError> {
                assert_eq!(
                    set_physical(&[
                        Value::String("Walk".into()),
                        Value::Int(50_000),
                        Value::Int(PHYS_TEMPORARY),
                        target.clone(),
                    ])?,
                    Value::Bool(true)
                );
                assert_eq!(
                    train_physical(&[
                        Value::String("Walk".into()),
                        Value::Int(5),
                        Value::Int(C4_MAX_PHYSICAL),
                        target.clone(),
                    ])?,
                    Value::Bool(true)
                );
                get_physical(&[
                    Value::String("Walk".into()),
                    Value::Int(PHYS_CURRENT),
                    target,
                ])
            },
        );

        assert_eq!(
            result.expect("foreign physical writes succeed"),
            Value::Int(50_005)
        );
        let physicals = outcome
            .other_objects
            .iter()
            .find(|other| other.object_id == clonk)
            .and_then(|other| other.update.as_ref())
            .and_then(|update| update.physicals.as_ref())
            .expect("foreign physical writes are recorded");
        assert_eq!(
            physicals
                .temporary
                .as_ref()
                .and_then(|physical| physical.value_by_name("Walk")),
            Some(50_005)
        );
    }

    #[test]
    fn global_reset_physical_targets_a_foreign_object_like_cpp() {
        // FnResetPhysical uses its explicit pObj even when cthr->Obj is null
        // (C4Script.cpp:614-636). Tutorial01 Script160 relies on the global
        // `ResetPhysical(GetCrew())` form to unlock digging.
        let mut definition = crate::Definition::from_script("CLNK", "Clonk", "#strict\n")
            .expect("definition compiles");
        let permanent = PhysicalInfo {
            can_dig: C4_MAX_PHYSICAL,
            ..PhysicalInfo::default()
        };
        definition.set_physical(permanent);
        let mut engine = crate::Engine::with_seed(0);
        engine
            .register_definition(definition)
            .expect("definition registers");
        let clonk = engine
            .spawn_object(crate::SpawnConfig::new("CLNK"))
            .expect("clonk spawns");
        let index = engine.find_object_index(clonk).expect("clonk exists");
        engine.objects[index].state.temporary_physical = Some(PhysicalInfo {
            can_dig: 0,
            ..permanent
        });

        let (result, outcome) =
            with_effect_context(None, &[], engine.host_world_context(), 2, || {
                reset_physical(&[object_reference_value(clonk)])
            });

        assert_eq!(result.expect("ResetPhysical succeeds"), Value::Bool(true));
        let physicals = outcome
            .other_objects
            .iter()
            .find(|other| other.object_id == clonk)
            .and_then(|other| other.update.as_ref())
            .and_then(|update| update.physicals.as_ref())
            .expect("foreign physical reset is recorded");
        assert_eq!(physicals.temporary, None);
        assert!(physicals.changes.is_empty());
    }

    #[test]
    fn do_energy_applies_delta_and_clamps() {
        // The harness object carries 100 raw energy and NO physical:
        // DoEnergy(-25) = -25% = -25000 raw (C4Object.cpp:1347), and its
        // missing Physical Energy gives both changes a zero upper bound.
        let (result, outcome) = with_object_host_context(|| do_energy(&[Value::Int(-25)]));
        let value = result.expect("DoEnergy returns bool");
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.expect("energy update recorded");
        assert_eq!(update.energy, Some(0));

        let (result, outcome) = with_object_host_context(|| do_energy(&[Value::Int(50)]));
        let value = result.expect("DoEnergy returns bool");
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.expect("energy update recorded");
        assert_eq!(update.energy, Some(0));
    }

    #[test]
    fn do_energy_respects_target_argument() {
        let mut target = ValueMap::new();
        target.insert("id".into(), Value::Int(99));
        let args = [Value::Int(-10), Value::Proplist(target)];
        let (result, outcome) = with_object_host_context(|| do_energy(&args));
        let value = result.expect("DoEnergy returns bool");
        assert_eq!(value, Value::Bool(false));
        assert!(outcome.object_update.is_none());
    }

    #[test]
    fn do_damage_applies_delta_and_clamps() {
        let (result, outcome) = with_object_host_context(|| do_damage(&[Value::Int(15)]));
        let value = result.expect("DoDamage returns bool");
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.expect("damage update recorded");
        assert_eq!(update.damage, Some(15));

        let (result, outcome) = with_object_host_context(|| do_damage(&[Value::Int(-20)]));
        let value = result.expect("DoDamage returns bool");
        assert_eq!(value, Value::Bool(true));
        let update = outcome.object_update.expect("damage update recorded");
        assert_eq!(update.damage, Some(0));
    }

    #[test]
    fn do_damage_respects_target_argument() {
        let mut target = ValueMap::new();
        target.insert("id".into(), Value::Int(77));
        let args = [Value::Int(5), Value::Proplist(target)];
        let (result, outcome) = with_object_host_context(|| do_damage(&args));
        let value = result.expect("DoDamage returns bool");
        assert_eq!(value, Value::Bool(false));
        assert!(outcome.object_update.is_none());
    }

    #[test]
    fn do_energy_accepts_exact_flag() {
        let args = [Value::Int(0), Value::Nil, Value::Bool(true)];
        let (result, outcome) = with_effect_context(
            Some(object_host_context_with_physical_energy(
                DEFAULT_MAX_ENERGY,
                DEFAULT_MAX_ENERGY,
            )),
            &[],
            HostWorldContext::default(),
            1,
            || do_energy(&args),
        );
        let value = result.expect("DoEnergy returns bool");
        assert_eq!(value, Value::Bool(true));
        assert!(outcome
            .object_update
            .as_ref()
            .and_then(|update| update.energy)
            .is_some());
    }

    #[test]
    fn get_energy_returns_current_energy() {
        let (result, _) = with_effect_context(
            Some(HostObjectContext::new(
                ObjectId::new(1),
                None,
                ObjectStatus::Normal,
                // 75% of C4MaxPhysical - GetEnergy reads percent
                // (C4Script.cpp FnGetEnergy).
                75_000,
                OWNER_NONE,
                Vector2::ZERO,
                Vector2::ZERO,
                &[],
                "Idle",
                0,
                0,
                ActionLibrary::default(),
                Direction::Left,
                CommandDirection::Stop,
                0,
                None,
                None,
                &[],
                crate::FULL_CON,
            )),
            &[],
            HostWorldContext::default(),
            1,
            || get_energy(&[]),
        );

        let value = result.expect("GetEnergy succeeds");
        assert_eq!(value, Value::Int(75));
    }

    #[test]
    fn get_con_returns_current_construction() {
        let (result, _) = with_effect_context(
            Some(HostObjectContext {
                construction: (crate::FULL_CON / 2).max(0),
                ..idle_object_context()
            }),
            &[],
            HostWorldContext::default(),
            1,
            || get_con(&[]),
        );

        let value = result.expect("GetCon succeeds");
        assert_eq!(value, Value::Int(50));
    }

    #[test]
    fn do_con_adjusts_construction() {
        let (result, outcome) = with_object_host_context(|| do_con(&[Value::Int(-25)]));
        let value = result.expect("DoCon returns bool");
        assert_eq!(value, Value::Bool(true));
        let update = outcome
            .object_update
            .expect("DoCon should produce an object update");
        let expected = crate::FULL_CON - ((crate::FULL_CON * 25) / 100);
        assert_eq!(update.construction, Some(expected));
    }

    #[test]
    fn get_energy_reads_world_when_target_provided() {
        let world = HostWorldContext::from_objects(vec![HostWorldObject::new(
            ObjectId::new(55),
            "Dummy",
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            OWNER_NONE,
            33_000,
            crate::FULL_CON,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            None,
        )]);
        let args = [object_reference_value(ObjectId::new(55))];
        let (result, _) = with_effect_context(None, &[], world, 1, || get_energy(&args));

        let value = result.expect("GetEnergy target succeeds");
        assert_eq!(value, Value::Int(33));
    }

    #[test]
    fn get_energy_returns_nil_without_context() {
        let (result, _) =
            with_effect_context(
                None,
                &[],
                HostWorldContext::default(),
                1,
                || get_energy(&[]),
            );
        let value = result.expect("GetEnergy handles missing context");
        assert_eq!(value, Value::Nil);
    }

    #[test]
    fn get_energy_converts_raw_units_to_percent() {
        let (result, _) = with_effect_context(
            Some(HostObjectContext {
                id: ObjectId::new(3),
                energy: LEGACY_MAX_PHYSICAL / 2,
                ..idle_object_context()
            }),
            &[],
            HostWorldContext::default(),
            1,
            || get_energy(&[]),
        );

        let value = result.expect("GetEnergy converts raw energy");
        assert_eq!(value, Value::Int(50));
    }

    #[test]
    fn create_object_registers_spawn_and_returns_reference() {
        let args = [Value::C4Id("CLNK".into())];
        let (result, outcome) = with_object_host_context(|| create_object(&args));
        let value = result.expect("CreateObject succeeds");
        assert_eq!(value, object_reference_value(ObjectId::new(1)));
        assert_eq!(outcome.spawns.len(), 1);
        let spawn = &outcome.spawns[0];
        assert_eq!(spawn.definition_id, "CLNK");
        assert_eq!(spawn.position, Vector2::ZERO);
        // This direct native invocation has an object context but no script
        // caller, so FnCreateObject substitutes that object's owner (-1).
        assert_eq!(spawn.owner, OWNER_NONE);
        assert_eq!(spawn.id, Some(ObjectId::new(1)));
        assert_eq!(outcome.next_object_id, 2);
    }

    #[test]
    fn create_object_runs_construction_before_initial_growth_and_completion() {
        // NewObject exposes the object at raw (x,y), Con=0 while Construction
        // runs; only then does initial DoCon keep the shaped object's bottom
        // fixed and call Completion followed by Initialize
        // (C4Game.cpp:1102-1146; C4Object.cpp:1428-1515).
        let hut_script = r#"#strict
local pBasement, iConstructionCon, iConstructionY, iCompletionY, iInitializeY, iOrder;
local iCompletionWood, iCompletionMetal;

protected func Construction()
{
    iConstructionCon = GetCon();
    iConstructionY = GetY();
    iOrder = 1;
    SetComponent(WOOD, 0);
    pBasement = CreateObject(BASE, 0, 8, -1);
}

protected func Completion()
{
    iCompletionY = GetY();
    iCompletionWood = GetComponent(WOOD);
    iCompletionMetal = GetComponent(METL);
    iOrder = iOrder * 10 + 2;
}

protected func Initialize()
{
    iInitializeY = GetY();
    iOrder = iOrder * 10 + 3;
}
"#;
        let caller_script = r#"#strict
public func Seed() { return(CreateObject(HUT1, 100, 100, -1)); }
"#;
        let mut engine = crate::Engine::with_seed(1);
        let mut hut =
            crate::Definition::from_script("HUT1", "Hut", hut_script).expect("hut script compiles");
        hut.set_shape_rect(Some(DefinitionRect::new(-18, -24, 36, 40)));
        hut.set_components(vec![
            crate::DefinitionComponent {
                id: "WOOD".to_owned(),
                count: 4,
            },
            crate::DefinitionComponent {
                id: "METL".to_owned(),
                count: 2,
            },
        ]);
        engine.register_definition(hut).expect("hut registers");
        let mut basement = crate::Definition::from_script("BASE", "Basement", "#strict")
            .expect("basement script compiles");
        basement.set_shape_rect(Some(DefinitionRect::new(-4, -2, 8, 4)));
        engine
            .register_definition(basement)
            .expect("basement registers");
        let caller = crate::Definition::from_script("CALL", "Caller", caller_script)
            .expect("caller script compiles");
        engine
            .register_definition(caller)
            .expect("caller registers");
        let caller_id = engine
            .spawn_object(SpawnConfig::new("CALL"))
            .expect("caller spawns");
        let caller_index = engine.find_object_index(caller_id).expect("caller exists");

        let hut_value = engine
            .call_object_function(caller_index, "Seed", Vec::new())
            .expect("Seed runs");
        let hut_id = object_id_from_value(&hut_value).expect("Seed returns hut");
        let hut_index = engine.find_object_index(hut_id).expect("hut exists");
        let hut = &engine.objects[hut_index].state;
        assert_eq!(hut.position, Vector2::new(100, 84));
        assert_eq!(hut.construction, FULL_CON);
        assert_eq!(hut.local_vars.get("iConstructionCon"), Some(&Value::Int(0)));
        assert_eq!(hut.local_vars.get("iConstructionY"), Some(&Value::Int(100)));
        assert_eq!(hut.local_vars.get("iCompletionY"), Some(&Value::Int(84)));
        assert_eq!(hut.local_vars.get("iInitializeY"), Some(&Value::Int(84)));
        assert_eq!(hut.local_vars.get("iOrder"), Some(&Value::Int(123)));
        assert_eq!(hut.local_vars.get("iCompletionWood"), Some(&Value::Int(4)));
        assert_eq!(hut.local_vars.get("iCompletionMetal"), Some(&Value::Int(2)));
        assert_eq!(hut.components.get("WOOD"), Some(&4));
        assert_eq!(hut.components.get("METL"), Some(&2));

        let basement = engine
            .objects
            .iter()
            .find(|object| object.definition_id == "BASE")
            .expect("Construction creates basement");
        assert_eq!(
            basement.state.position,
            Vector2::new(100, 106),
            "nested CreateObject is relative to the parent's raw pre-growth position"
        );
    }

    #[test]
    fn create_contents_change_def_uses_the_new_definition_for_growth_and_callbacks() {
        let mut engine = crate::Engine::with_seed(3);
        let builder = crate::Definition::from_script(
            "BLDR",
            "Builder",
            "#strict\npublic func Make() { return CreateContents(OLD1); }",
        )
        .expect("builder script compiles");
        engine
            .register_definition(builder)
            .expect("builder registers");

        let mut old = crate::Definition::from_script(
            "OLD1",
            "Old",
            r#"#strict
protected func Construction() { ChangeDef(NEW1); }
protected func Completion() { SetComponent(METL, 9); }
"#,
        )
        .expect("old script compiles");
        old.set_components(vec![crate::DefinitionComponent {
            id: "METL".to_owned(),
            count: 3,
        }]);
        engine.register_definition(old).expect("old registers");

        let mut new = crate::Definition::from_script(
            "NEW1",
            "New",
            r#"#strict
local completion_wood, completion_metal, initialized;
protected func Completion()
{
    completion_wood = GetComponent(WOOD);
    completion_metal = GetComponent(METL);
}
protected func Initialize() { initialized = 1; }
"#,
        )
        .expect("new script compiles");
        new.set_components(vec![crate::DefinitionComponent {
            id: "WOOD".to_owned(),
            count: 2,
        }]);
        engine.register_definition(new).expect("new registers");
        for (id, name) in [("WOOD", "Wood"), ("METL", "Metal")] {
            engine
                .register_definition(
                    crate::Definition::from_script(id, name, "#strict")
                        .expect("component script compiles"),
                )
                .expect("component registers");
        }

        let builder = engine
            .spawn_object(SpawnConfig::new("BLDR"))
            .expect("builder spawns");
        let builder_index = engine.find_object_index(builder).expect("builder exists");
        let created = engine
            .call_object_function(builder_index, "Make", Vec::new())
            .expect("CreateContents succeeds");
        let created = object_id_from_value(&created).expect("created object returned");
        let created = engine
            .object_snapshot(created)
            .expect("created object survives");

        assert_eq!(created.definition_id, "NEW1");
        // ChangeDef keeps the old C4IDList's IDs. Initial DoCon then looks
        // up the new definition component counts by LIST INDEX, so OLD1's
        // first METL entry gains NEW1's first (WOOD) count.
        assert_eq!(created.components.get("WOOD"), None);
        assert_eq!(created.components.get("METL"), Some(&2));
        assert_eq!(
            created.local_vars.get("completion_wood"),
            Some(&Value::Int(0))
        );
        assert_eq!(
            created.local_vars.get("completion_metal"),
            Some(&Value::Int(2))
        );
        assert_eq!(created.local_vars.get("initialized"), Some(&Value::Int(1)));
    }

    #[test]
    fn compose_contents_consumes_same_call_contents_and_runs_cpp_creation_order() {
        let builder_script = r#"#strict
local missing_id, missing_count, removal_order, removal_reason, removal_id;
public func Build()
{
    var wood = CreateContents(WOOD);
    AddEffect("ComposeRemoval", wood, 1, 0, this());
    CreateContents(WOOD);
    CreateContents(METL);
    return ComposeContents(PROD);
}
public func Missing() { return ComposeContents(PROD); }
public func ProductConstructed() { removal_order = removal_order * 10 + 2; }
protected func FxComposeRemovalStop(target, number, reason)
{
    removal_order = removal_order * 10 + 1;
    removal_reason = reason;
    removal_id = GetID(target);
}
protected func BuildNeedsMaterial(component_id, count)
{
    missing_id = component_id;
    missing_count = count;
    return 1;
}
"#;
        let product_script = r#"#strict
local construction_x, construction_y, construction_con, creator_seen, order, start_calls;
protected func Construction(creator)
{
    creator->ProductConstructed();
    construction_x = GetX();
    construction_y = GetY();
    construction_con = GetCon();
    creator_seen = creator;
    order = 1;
    SetAction("Work");
}
protected func Started() { start_calls++; }
protected func Completion() { order = order * 10 + 2; }
protected func Initialize() { order = order * 10 + 3; }
"#;
        let mut engine = crate::Engine::with_seed(17);
        let builder = crate::Definition::from_script("BLDR", "Builder", builder_script)
            .expect("builder script compiles");
        engine
            .register_definition(builder)
            .expect("builder registers");
        for (id, name) in [("WOOD", "Wood"), ("METL", "Metal")] {
            let material = crate::Definition::from_script(id, name, "#strict")
                .expect("material script compiles");
            engine
                .register_definition(material)
                .expect("material registers");
        }
        let mut product = crate::Definition::from_script("PROD", "Product", product_script)
            .expect("product script compiles");
        // Construction runs while Con=0. C4Object::SetAction only keeps a
        // non-idle action there when the definition enables IncompleteActivity.
        product.set_incomplete_activity(true);
        product.configure_actions(
            None,
            HashMap::from([(
                "Work".to_owned(),
                ActionSpec::default().with_start_call("Started"),
            )]),
        );
        product.set_components(vec![
            crate::DefinitionComponent {
                id: "WOOD".to_owned(),
                count: 2,
            },
            crate::DefinitionComponent {
                id: "METL".to_owned(),
                count: 1,
            },
        ]);
        engine
            .register_definition(product)
            .expect("product registers");
        let builder_id = engine
            .spawn_object(
                SpawnConfig::new("BLDR")
                    .with_position(Vector2::new(300, 200))
                    .with_owner(3)
                    .with_controller(7),
            )
            .expect("builder spawns");
        let builder_index = engine
            .find_object_index(builder_id)
            .expect("builder exists");

        let product_value = engine
            .call_object_function(builder_index, "Build", Vec::new())
            .expect("same-call composition succeeds");
        let product_id = object_id_from_value(&product_value).expect("Build returns product");
        let product_index = engine
            .find_object_index(product_id)
            .expect("product survives");
        let product = &engine.objects[product_index].state;
        assert_eq!(product.container, Some(builder_id));
        assert_eq!(product.position, Vector2::new(300, 200));
        assert_eq!(product.controller, 7);
        assert_eq!(
            product.local_vars.get("construction_x"),
            Some(&Value::Int(50))
        );
        assert_eq!(
            product.local_vars.get("construction_y"),
            Some(&Value::Int(50))
        );
        assert_eq!(
            product.local_vars.get("construction_con"),
            Some(&Value::Int(0))
        );
        assert_eq!(
            product.local_vars.get("creator_seen"),
            Some(&object_reference_value(builder_id))
        );
        assert_eq!(product.local_vars.get("order"), Some(&Value::Int(123)));
        assert_eq!(product.local_vars.get("start_calls"), Some(&Value::Int(1)));
        assert_eq!(
            engine
                .objects
                .iter()
                .filter(|object| matches!(object.definition_id.as_str(), "WOOD" | "METL"))
                .count(),
            0,
            "AssignRemoval consumes every direct component"
        );

        let builder_index = engine
            .find_object_index(builder_id)
            .expect("builder remains");
        assert_eq!(
            engine
                .call_object_function(builder_index, "Missing", Vec::new())
                .expect("insufficient composition returns normally"),
            Value::Nil
        );
        let builder = &engine.objects[builder_index].state;
        assert_eq!(
            builder.local_vars.get("missing_id"),
            Some(&Value::C4Id("WOOD".into()))
        );
        assert_eq!(
            builder.local_vars.get("missing_count"),
            Some(&Value::Int(2))
        );
        assert_eq!(
            builder.local_vars.get("removal_order"),
            Some(&Value::Int(12))
        );
        assert_eq!(
            builder.local_vars.get("removal_reason"),
            Some(&Value::Int(3))
        );
        assert_eq!(
            builder.local_vars.get("removal_id"),
            Some(&Value::C4Id("WOOD".into()))
        );
    }

    #[test]
    fn compose_existing_contents_runs_exact_assign_removal_cleanup() {
        let builder_script = r#"#strict
local stop_order, lower_saw_upper, removal_reason, child_status, child_count, sibling_saved;
public func Install()
{
    var wood = FindContents(WOOD);
    AddEffect("Lower", wood, 100, 0, this());
    AddEffect("Upper", wood, 200, 0, this());
    return 1;
}
public func Build() { return ComposeContents(PROD); }
protected func FxUpperStop(target, number, reason)
{
    stop_order = stop_order * 10 + 2;
    removal_reason = reason;
    return -1;
}
protected func FxLowerStop(target, number, reason)
{
    stop_order = stop_order * 10 + 1;
    lower_saw_upper = !!GetEffect("Upper", target);
}
public func ComponentAborted() { stop_order = stop_order * 10 + 3; }
public func ChildObserved(parent, count)
{
    child_status = GetObjectStatus(parent);
    child_count = count;
}
public func SiblingSaved() { sibling_saved++; }
"#;
        let wood_script = r#"#strict
protected func RemovedAbort() { Contained()->ComponentAborted(); }
"#;
        let first_child_script = r#"#strict
protected func Destruction()
{
    var parent = Contained();
    var builder = Contained(parent);
    builder->ChildObserved(parent, ContentsCount(0, parent));
    var sibling = Contents(0, parent);
    if (sibling)
    {
        Enter(builder, sibling);
        builder->SiblingSaved();
    }
}
"#;
        let product_script = r#"#strict
local entrance_ocf;
protected func Entrance() { entrance_ocf = GetOCF(); }
"#;

        let mut engine = crate::Engine::with_seed(29);
        engine
            .register_definition(
                crate::Definition::from_script("BLDR", "Builder", builder_script)
                    .expect("builder compiles"),
            )
            .expect("builder registers");
        let mut wood =
            crate::Definition::from_script("WOOD", "Wood", wood_script).expect("wood compiles");
        wood.configure_actions(
            None,
            HashMap::from([(
                "Active".to_owned(),
                ActionSpec::default().with_abort_call("RemovedAbort"),
            )]),
        );
        engine.register_definition(wood).expect("wood registers");
        for (id, name, script) in [
            ("METL", "Metal", "#strict"),
            ("CHLD", "Child", first_child_script),
        ] {
            engine
                .register_definition(
                    crate::Definition::from_script(id, name, script).expect("component compiles"),
                )
                .expect("component registers");
        }
        let mut product = crate::Definition::from_script("PROD", "Product", product_script)
            .expect("product compiles");
        product.set_components(vec![
            crate::DefinitionComponent {
                id: "WOOD".to_owned(),
                count: 1,
            },
            crate::DefinitionComponent {
                id: "METL".to_owned(),
                count: 1,
            },
        ]);
        engine
            .register_definition(product)
            .expect("product registers");

        let builder = engine
            .spawn_object(SpawnConfig::new("BLDR"))
            .expect("builder spawns");
        let wood = engine
            .spawn_object(
                SpawnConfig::new("WOOD")
                    .with_container(builder)
                    .with_action(crate::ActionState::new("Active")),
            )
            .expect("wood spawns");
        let metal = engine
            .spawn_object(SpawnConfig::new("METL").with_container(builder))
            .expect("metal spawns");
        let saved_sibling = engine
            .spawn_object(SpawnConfig::new("CHLD").with_container(wood))
            .expect("older child spawns");
        let first_child = engine
            .spawn_object(SpawnConfig::new("CHLD").with_container(wood))
            .expect("front child spawns");
        let builder_index = engine.find_object_index(builder).expect("builder index");
        engine
            .call_object_function(builder_index, "Install", Vec::new())
            .expect("effects install");

        let product = engine
            .call_object_function(builder_index, "Build", Vec::new())
            .expect("composition runs");
        let product = object_id_from_value(&product).expect("product returned");
        for removed in [wood, metal, first_child] {
            assert!(
                engine
                    .object_snapshot(removed)
                    .is_none_or(|object| !object.status.is_active()),
                "consumed object {removed} is dead"
            );
        }
        let builder_state = &engine.objects[engine.find_object_index(builder).unwrap()].state;
        assert_eq!(
            engine
                .object_snapshot(saved_sibling)
                .expect("sibling survives")
                .container,
            Some(builder),
            "child callback state: {:?}",
            builder_state.local_vars
        );
        assert_eq!(
            builder_state.local_vars.get("stop_order"),
            Some(&Value::Int(213))
        );
        assert_eq!(
            builder_state.local_vars.get("lower_saw_upper"),
            Some(&Value::Bool(true))
        );
        assert_eq!(
            builder_state.local_vars.get("removal_reason"),
            Some(&Value::Int(3))
        );
        assert_eq!(
            builder_state.local_vars.get("child_status"),
            Some(&Value::Int(0))
        );
        assert_eq!(
            builder_state.local_vars.get("child_count"),
            Some(&Value::Int(1))
        );
        assert_eq!(
            builder_state.local_vars.get("sibling_saved"),
            Some(&Value::Int(1))
        );

        let product = engine.object_snapshot(product).expect("product survives");
        assert!(
            !product.mobile,
            "CopyMotion does not mobilize the new product"
        );
        assert_eq!(
            product
                .local_vars
                .get("entrance_ocf")
                .and_then(Value::as_c4_int)
                .unwrap_or_default() as u32
                & (ocf::NOT_CONTAINED | ocf::AVAILABLE),
            0,
            "Entrance observes the post-Enter cached OCF"
        );
    }

    #[test]
    fn create_contents_runs_the_cpp_base_auto_sell_tail_synchronously() {
        let mut engine = crate::Engine::with_seed(31);
        engine
            .register_player(crate::PlayerConfig::new(0, "Player"))
            .expect("player registers");
        engine
            .register_definition(
                crate::Definition::from_script(
                    "BASE",
                    "Base",
                    "#strict\npublic func MakeGold() { return CreateContents(GOLD); }",
                )
                .expect("base compiles"),
            )
            .expect("base registers");
        let mut gold =
            crate::Definition::from_script("GOLD", "Gold", "#strict").expect("gold compiles");
        gold.set_value(25);
        gold.set_base_auto_sell(true);
        gold.set_rebuyable(true);
        engine.register_definition(gold).expect("gold registers");

        let base = engine
            .spawn_object(SpawnConfig::new("BASE"))
            .expect("base spawns");
        let base_index = engine.find_object_index(base).expect("base index");
        engine.objects[base_index].state.base = 0;
        let result = engine
            .call_object_function(base_index, "MakeGold", Vec::new())
            .expect("CreateContents returns normally");
        let gold = object_id_from_value(&result).expect("native return keeps the C++ pointer");

        assert!(
            engine
                .object_snapshot(gold)
                .is_none_or(|object| !object.status.is_active()),
            "the auto-sold gold is removed before CreateContents returns"
        );
        let player = engine.player(0).expect("player remains");
        assert_eq!(player.wealth(), 25);
        assert_eq!(
            player.home_base_material().get(&DefinitionId::from("GOLD")),
            Some(&1)
        );
    }

    fn buy_host_fixture() -> (crate::Engine, ObjectId, ObjectId) {
        let caller_script = r#"#strict
public func BuyAt(int for_player, int pay_player, object target, bool show_errors)
{
    return Buy(ITEM, for_player, pay_player, target, show_errors);
}
public func BuyHere(int for_player, int pay_player, bool show_errors)
{
    return Buy(ITEM, for_player, pay_player, 0, show_errors);
}
public func BuyCrew(int for_player, int pay_player, object target)
{
    return Buy(CREW, for_player, pay_player, target, false);
}
"#;
        let item_script = r#"#strict
local purchase_player, purchase_base;
public func CalcDefValue(object base, int player)
{
    if (GetID() == ITEM) return 25;
    return 99;
}
public func Purchase(int player, object base)
{
    purchase_player = player;
    purchase_base = base;
}
"#;

        let mut engine = crate::Engine::with_seed(32);
        engine
            .register_player(crate::PlayerConfig::new(1, "Recipient").with_wealth(7))
            .expect("recipient registers");
        engine
            .register_player(
                crate::PlayerConfig::new(2, "Payer")
                    .with_wealth(100)
                    .with_home_base_material(HashMap::from([
                        (DefinitionId::from("ITEM"), 2),
                        (DefinitionId::from("CREW"), 1),
                    ])),
            )
            .expect("payer registers");
        engine
            .register_definition(
                crate::Definition::from_script("CALL", "Caller", caller_script)
                    .expect("caller compiles"),
            )
            .expect("caller registers");
        engine
            .register_definition(
                crate::Definition::from_script("BASE", "Base", "#strict").expect("base compiles"),
            )
            .expect("base registers");
        let mut item =
            crate::Definition::from_script("ITEM", "Item", item_script).expect("item compiles");
        item.set_value(99);
        engine.register_definition(item).expect("item registers");
        let mut crew = crate::Definition::from_script(
            "CREW",
            "Crew",
            r#"#strict
local order;
public func MakeCrewMember() { order = 9; return false; }
public func Recruitment(int player) { order = order * 10 + player; }
public func Purchase(int player, object base) { order = order * 10 + player; }
"#,
        )
        .expect("crew compiles");
        crew.set_value(10);
        crew.set_category(crate::CATEGORY_LIVING);
        crew.set_crew_member(true);
        engine.register_definition(crew).expect("crew registers");

        let caller = engine
            .spawn_object(
                SpawnConfig::new("CALL")
                    .with_owner(1)
                    .with_controller(1)
                    .with_position(Vector2::new(123, 234)),
            )
            .expect("caller spawns");
        let base = engine
            .spawn_object(
                SpawnConfig::new("BASE")
                    .with_owner(2)
                    .with_controller(2)
                    .with_position(Vector2::new(70, 80)),
            )
            .expect("base spawns");
        (engine, caller, base)
    }

    #[test]
    fn buy_host_charges_payer_and_places_the_created_object() {
        let (mut engine, caller, base) = buy_host_fixture();
        let caller_index = engine.find_object_index(caller).expect("caller index");

        let bought = engine
            .call_object_function(
                caller_index,
                "BuyAt",
                vec![
                    Value::Int(1),
                    Value::Int(2),
                    object_reference_value(base),
                    Value::Bool(false),
                ],
            )
            .expect("Buy with an explicit target runs");
        let bought = object_id_from_value(&bought).expect("Buy returns the item");
        let snapshot = engine.object_snapshot(bought).expect("bought item exists");
        assert_eq!(snapshot.owner, 1);
        assert_eq!(snapshot.controller, 2, "Enter copies base controller");
        assert_eq!(snapshot.container, Some(base));
        assert_eq!(snapshot.position, Vector2::new(70, 80));
        assert_eq!(
            snapshot.local_vars.get("purchase_player"),
            Some(&Value::Int(2))
        );
        assert_eq!(
            snapshot.local_vars.get("purchase_base"),
            Some(&object_reference_value(base))
        );
        assert_eq!(engine.player(1).expect("recipient remains").wealth(), 7);
        assert_eq!(engine.player(2).expect("payer remains").wealth(), 75);
        assert_eq!(
            engine
                .player(2)
                .expect("payer remains")
                .home_base_material()
                .get(&DefinitionId::from("ITEM")),
            Some(&1)
        );

        let bought = engine
            .call_object_function(
                caller_index,
                "BuyHere",
                vec![Value::Int(1), Value::Int(2), Value::Bool(false)],
            )
            .expect("Buy without a target runs");
        let bought = object_id_from_value(&bought).expect("Buy returns the second item");
        let snapshot = engine
            .object_snapshot(bought)
            .expect("second bought item exists");
        assert_eq!(snapshot.owner, 1);
        assert_eq!(snapshot.controller, 1);
        assert_eq!(snapshot.container, None);
        assert_eq!(snapshot.position, Vector2::new(123, 234));
        assert_eq!(
            snapshot.local_vars.get("purchase_base"),
            Some(&object_reference_value(caller))
        );
        assert_eq!(engine.player(2).expect("payer remains").wealth(), 50);
        assert_eq!(
            engine
                .player(2)
                .expect("payer remains")
                .home_base_material()
                .get(&DefinitionId::from("ITEM")),
            Some(&0),
            "DecreaseIDCount(false) retains the zero stock slot"
        );
    }

    #[test]
    fn buy_host_uses_native_crew_enrollment_before_purchase() {
        let (mut engine, caller, base) = buy_host_fixture();
        let caller_index = engine.find_object_index(caller).expect("caller index");
        let bought = engine
            .call_object_function(
                caller_index,
                "BuyCrew",
                vec![Value::Int(1), Value::Int(2), object_reference_value(base)],
            )
            .expect("crew Buy runs");
        let bought = object_id_from_value(&bought).expect("crew Buy returns the object");
        let snapshot = engine.object_snapshot(bought).expect("bought crew exists");
        assert!(snapshot.crew_member);
        assert_eq!(snapshot.owner, 1);
        assert_eq!(snapshot.controller, 1);
        assert_eq!(snapshot.container, Some(base));
        assert_eq!(
            snapshot.local_vars.get("order"),
            Some(&Value::Int(12)),
            "native Recruitment runs before Purchase and bypasses the same-name script override"
        );
        assert!(engine
            .player(1)
            .expect("recipient remains")
            .crew()
            .contains(&bought));
        assert_eq!(engine.player(2).expect("payer remains").wealth(), 90);
        assert_eq!(
            engine
                .player(2)
                .expect("payer remains")
                .home_base_material()
                .get(&DefinitionId::from("CREW")),
            Some(&0)
        );
    }

    #[test]
    fn buy_host_rejects_invalid_players_and_honors_silent_errors() {
        let (mut engine, caller, base) = buy_host_fixture();
        let caller_index = engine.find_object_index(caller).expect("caller index");
        let initial_objects = engine.snapshot().objects.len();

        for (for_player, pay_player) in [(99, 2), (1, 99)] {
            let rejected = engine
                .call_object_function(
                    caller_index,
                    "BuyAt",
                    vec![
                        Value::Int(for_player),
                        Value::Int(pay_player),
                        object_reference_value(base),
                        Value::Bool(true),
                    ],
                )
                .expect("invalid-player Buy returns normally");
            assert_eq!(rejected, Value::Nil);
        }
        assert_eq!(engine.snapshot().objects.len(), initial_objects);
        assert_eq!(engine.player(2).expect("payer remains").wealth(), 100);
        assert_eq!(
            engine
                .player(2)
                .expect("payer remains")
                .home_base_material()
                .get(&DefinitionId::from("ITEM")),
            Some(&2)
        );
        assert!(engine.snapshot().hud.messages.is_empty());
        assert!(engine.pending_audio.is_empty());

        engine
            .set_player_home_base_material(2, HashMap::new())
            .expect("empty stock installs");
        let unavailable = engine
            .call_object_function(
                caller_index,
                "BuyAt",
                vec![
                    Value::Int(1),
                    Value::Int(2),
                    object_reference_value(base),
                    Value::Bool(false),
                ],
            )
            .expect("silent unavailable-item Buy returns normally");
        assert_eq!(unavailable, Value::Nil);
        assert_eq!(engine.snapshot().objects.len(), initial_objects);
        assert!(engine.snapshot().hud.messages.is_empty());
        assert!(engine.pending_audio.is_empty());
        engine
            .set_player_home_base_material(2, HashMap::from([(DefinitionId::from("ITEM"), 2)]))
            .expect("stock restores");

        engine
            .set_player_wealth(2, 24)
            .expect("insufficient wealth installs");
        let rejected = engine
            .call_object_function(
                caller_index,
                "BuyAt",
                vec![
                    Value::Int(1),
                    Value::Int(2),
                    object_reference_value(base),
                    Value::Bool(false),
                ],
            )
            .expect("silent insufficient-wealth Buy returns normally");
        assert_eq!(rejected, Value::Nil);
        assert_eq!(engine.snapshot().objects.len(), initial_objects);
        assert_eq!(engine.player(2).expect("payer remains").wealth(), 24);
        assert_eq!(
            engine
                .player(2)
                .expect("payer remains")
                .home_base_material()
                .get(&DefinitionId::from("ITEM")),
            Some(&2)
        );
        assert!(engine.snapshot().hud.messages.is_empty());
        assert!(engine.pending_audio.is_empty());

        let rejected = engine
            .call_object_function(
                caller_index,
                "BuyAt",
                vec![
                    Value::Int(1),
                    Value::Int(2),
                    object_reference_value(base),
                    Value::Bool(true),
                ],
            )
            .expect("visible insufficient-wealth Buy returns normally");
        assert_eq!(rejected, Value::Nil);
        let messages = engine.snapshot().hud.messages;
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].kind, MessageKind::GlobalPlayer);
        assert_eq!(messages[0].player, Some(2));
        assert_eq!(messages[0].lines, vec!["Not enough money!"]);
        assert!(matches!(
            engine.pending_audio.as_slice(),
            [AudioCommand::PlaySound { name, target, .. }]
                if name == "Error" && *target == Some(base)
        ));
    }

    #[test]
    fn sell_host_runs_the_cpp_homebase_transaction_and_defaults_target() {
        let caller_script = r#"#strict
public func SellTarget(int player, object target) { return Sell(player, target); }
"#;
        let item_script = r#"#strict
local sale_base;
public func CalcValue(object base, int player)
{
    sale_base = base;
    if (!base) return(-1000);
    return(20 + player);
}
public func Sale(int player)
{
    if (!Contained())
        sale_base->RecordSale(GetWealth(player), GetHomebaseMaterial(player, VALU));
}
public func SellSelf(int player) { return Sell(player); }
"#;
        let base_script = r#"#strict
local sales, sale_wealth, sale_stock;
public func CalcSellValue(object item, int value) { return(value + 3); }
public func RecordSale(int wealth, int stock)
{
    sales++;
    sale_wealth = wealth;
    sale_stock = stock;
}
"#;

        let mut engine = crate::Engine::with_seed(32);
        engine
            .register_player(crate::PlayerConfig::new(0, "Player"))
            .expect("player registers");
        engine
            .register_definition(
                crate::Definition::from_script("CALL", "Caller", caller_script)
                    .expect("caller compiles"),
            )
            .expect("caller registers");
        engine
            .register_definition(
                crate::Definition::from_script("BASE", "Base", base_script).expect("base compiles"),
            )
            .expect("base registers");
        let mut item =
            crate::Definition::from_script("VALU", "Valuable", item_script).expect("item compiles");
        item.set_value(99);
        item.set_rebuyable(true);
        engine.register_definition(item).expect("item registers");

        let base = engine
            .spawn_object(SpawnConfig::new("BASE"))
            .expect("base spawns");
        let caller = engine
            .spawn_object(SpawnConfig::new("CALL"))
            .expect("caller spawns");
        let caller_index = engine.find_object_index(caller).expect("caller index");

        let explicit = engine
            .spawn_object(SpawnConfig::new("VALU").with_container(base))
            .expect("explicit target spawns");
        let sold = engine
            .call_object_function(
                caller_index,
                "SellTarget",
                vec![Value::Int(0), object_reference_value(explicit)],
            )
            .expect("explicit Sell runs");
        assert_eq!(sold, Value::Bool(true));
        assert!(
            engine
                .object_snapshot(explicit)
                .is_none_or(|object| !object.status.is_active()),
            "the explicitly sold object is removed"
        );
        assert_eq!(engine.player(0).expect("player remains").wealth(), 23);
        assert_eq!(
            engine
                .player(0)
                .expect("player remains")
                .home_base_material()
                .get(&DefinitionId::from("VALU")),
            Some(&1)
        );
        let base_snapshot = engine.object_snapshot(base).expect("base remains");
        assert_eq!(base_snapshot.local_vars.get("sales"), Some(&Value::Int(1)));
        assert_eq!(
            base_snapshot.local_vars.get("sale_wealth"),
            Some(&Value::Int(23))
        );
        assert_eq!(
            base_snapshot.local_vars.get("sale_stock"),
            Some(&Value::Int(1))
        );

        let invalid = engine
            .spawn_object(SpawnConfig::new("VALU").with_container(base))
            .expect("invalid-player target spawns");
        let rejected = engine
            .call_object_function(
                caller_index,
                "SellTarget",
                vec![Value::Int(99), object_reference_value(invalid)],
            )
            .expect("invalid-player Sell returns normally");
        assert_eq!(rejected, Value::Bool(false));
        let invalid_snapshot = engine
            .object_snapshot(invalid)
            .expect("rejected object remains");
        assert!(invalid_snapshot.status.is_active());
        assert_eq!(invalid_snapshot.container, Some(base));
        assert_eq!(engine.player(0).expect("player remains").wealth(), 23);
        assert_eq!(
            engine
                .player(0)
                .expect("player remains")
                .home_base_material()
                .get(&DefinitionId::from("VALU")),
            Some(&1)
        );
        assert_eq!(
            engine
                .object_snapshot(base)
                .expect("base remains")
                .local_vars
                .get("sales"),
            Some(&Value::Int(1))
        );

        engine
            .set_player_wealth(0, 9_990)
            .expect("near-cap wealth installs");
        let implicit = engine
            .spawn_object(SpawnConfig::new("VALU").with_container(base))
            .expect("implicit target spawns");
        let implicit_index = engine.find_object_index(implicit).expect("implicit index");
        let sold = engine
            .call_object_function(implicit_index, "SellSelf", vec![Value::Int(0)])
            .expect("implicit Sell runs");
        assert_eq!(sold, Value::Bool(true));
        assert!(
            engine
                .object_snapshot(implicit)
                .is_none_or(|object| !object.status.is_active()),
            "the calling object is removed"
        );
        assert_eq!(engine.player(0).expect("player remains").wealth(), 10_000);
        assert_eq!(
            engine
                .player(0)
                .expect("player remains")
                .home_base_material()
                .get(&DefinitionId::from("VALU")),
            Some(&2)
        );
        let base_snapshot = engine.object_snapshot(base).expect("base remains");
        assert_eq!(base_snapshot.local_vars.get("sales"), Some(&Value::Int(2)));
        assert_eq!(
            base_snapshot.local_vars.get("sale_wealth"),
            Some(&Value::Int(10_000))
        );
        assert_eq!(
            base_snapshot.local_vars.get("sale_stock"),
            Some(&Value::Int(2))
        );
    }

    fn place_animal_world(id: &str, placement: i32, landscape: Landscape) -> HostWorldContext {
        HostWorldContext::with_landscape(
            Vec::<HostWorldObject>::new(),
            Some(landscape),
            HashMap::from([(
                DefinitionId::from(id),
                DefinitionMetadata {
                    category: crate::CATEGORY_LIVING,
                    shape: Some(DefinitionRect::new(-2, -3, 4, 6)),
                    placement,
                    physical: PhysicalInfo {
                        energy: 50_000,
                        ..PhysicalInfo::default()
                    },
                    ..DefinitionMetadata::default()
                },
            )]),
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            1,
            false,
        )
    }

    #[test]
    fn place_animal_surface_uses_two_draws_and_find_solid_ground() {
        // C4D_Place_Surface draws global x then y, runs FindSolidGround with
        // Shape.Wdt, and calls creatorless CreateObject at the returned point
        // (C4Game.cpp:3028-3043; C4Landscape.cpp:1830-1857).
        let mut landscape = Landscape::flat(100, 60);
        landscape.set_world_height(100);
        let mut expected_rng = LcgRng::new(17);
        let start_x = expected_rng.random(100);
        let start_y = expected_rng.random(100);
        let raw = landscape
            .find_solid_ground(start_x, start_y, 4)
            .map(|(x, y)| Vector2::new(x, y))
            .expect("flat world has solid ground");
        let final_position = Vector2::new(
            raw.x,
            crate::docon_initial_center_y(
                Some(DefinitionRect::new(-2, -3, 4, 6)),
                false,
                0,
                FULL_CON,
                raw.y,
            ),
        );
        let guard = enter_random_context(LcgRng::new(17));
        let (result, outcome) = with_effect_context(
            None,
            &[],
            place_animal_world("SURF", 0, landscape),
            1,
            || place_animal(&[Value::C4Id("SURF".into())]),
        );
        let final_rng = guard.finish();

        assert_eq!(final_rng, expected_rng);
        assert_eq!(
            result.expect("PlaceAnimal surface succeeds"),
            object_reference_value(ObjectId::new(1))
        );
        let spawn = &outcome.spawns[0];
        assert_eq!(spawn.position, final_position);
        assert_eq!(
            spawn.fixed_position,
            Some(FixedVec2::from_ints(raw.x, raw.y))
        );
        assert_eq!(spawn.owner, OWNER_NONE);
        assert_eq!(spawn.controller, Some(OWNER_NONE));
        assert_eq!(spawn.construction, FULL_CON);
    }

    #[test]
    fn place_animal_liquid_uses_surface_then_deep_fallback_search() {
        // C4D_Place_Liquid draws global x/y, tries FindSurfaceLiquid before
        // FindLiquid, then adds Shape.Hgt/2 before creatorless CreateObject
        // (C4Game.cpp:3044-3052; C4Landscape.cpp:1860-1915).
        let mut landscape = Landscape::flat(100, 80);
        landscape.set_world_height(100);
        let water = crate::MaterialId::new(1).expect("water id");
        for x in 20..80 {
            for y in 40..80 {
                assert!(landscape.insert_liquid_at(x, y, Some(water)));
            }
        }
        let mut expected_rng = LcgRng::new(23);
        let mut expected_x = expected_rng.random(100);
        let mut expected_y = expected_rng.random(100);
        assert!(
            placement_find_surface_liquid(&landscape, &mut expected_x, &mut expected_y, 4, 6)
                || placement_find_liquid(&landscape, &mut expected_x, &mut expected_y, 4, 6)
        );
        let raw = Vector2::new(expected_x, expected_y + 3);
        let final_position = Vector2::new(
            raw.x,
            crate::docon_initial_center_y(
                Some(DefinitionRect::new(-2, -3, 4, 6)),
                false,
                0,
                FULL_CON,
                raw.y,
            ),
        );
        let guard = enter_random_context(LcgRng::new(23));
        let (result, outcome) = with_effect_context(
            None,
            &[],
            place_animal_world("FISH", 1, landscape),
            1,
            || place_animal(&[Value::C4Id("FISH".into())]),
        );
        let final_rng = guard.finish();

        assert_eq!(final_rng, expected_rng);
        assert_eq!(
            result.expect("PlaceAnimal liquid succeeds"),
            object_reference_value(ObjectId::new(1))
        );
        assert_eq!(outcome.spawns[0].position, final_position);
        assert_eq!(
            outcome.spawns[0].fixed_position,
            Some(FixedVec2::from_ints(raw.x, raw.y))
        );
    }

    #[test]
    fn place_animal_air_scans_first_semisolid_then_draws_height() {
        // C4D_Place_Air draws global x, scans down to the first semi-solid
        // row, then draws Random(y) (C4Game.cpp:3053-3060).
        let mut landscape = Landscape::flat(100, 60);
        landscape.set_world_height(100);
        let mut expected_rng = LcgRng::new(31);
        let expected_x = expected_rng.random(100);
        let ceiling = (0..100)
            .find(|&y| landscape.is_semi_solid_at(expected_x, y))
            .expect("flat ground is semi-solid");
        let raw = Vector2::new(expected_x, expected_rng.random(ceiling));
        let final_position = Vector2::new(
            raw.x,
            crate::docon_initial_center_y(
                Some(DefinitionRect::new(-2, -3, 4, 6)),
                false,
                0,
                FULL_CON,
                raw.y,
            ),
        );
        let guard = enter_random_context(LcgRng::new(31));
        let (result, outcome) = with_effect_context(
            None,
            &[],
            place_animal_world("BIRD", 2, landscape),
            1,
            || place_animal(&[Value::C4Id("BIRD".into())]),
        );
        let final_rng = guard.finish();

        assert_eq!(final_rng, expected_rng);
        assert_eq!(
            result.expect("PlaceAnimal air succeeds"),
            object_reference_value(ObjectId::new(1))
        );
        assert_eq!(outcome.spawns[0].position, final_position);
        assert_eq!(
            outcome.spawns[0].fixed_position,
            Some(FixedVec2::from_ints(raw.x, raw.y))
        );
    }

    #[test]
    fn place_animal_air_without_semisolid_uses_world_height() {
        // If the column contains no semi-solid pixel, the C++ scan reaches
        // GBackHgt and still calls Random(GBackHgt) before creating the
        // animal (C4Game.cpp:3053-3060).
        let mut landscape = Landscape::flat(100, 100);
        landscape.set_world_height(100);
        let mut expected_rng = LcgRng::new(37);
        let raw = Vector2::new(expected_rng.random(100), expected_rng.random(100));
        let guard = enter_random_context(LcgRng::new(37));
        let (result, outcome) = with_effect_context(
            None,
            &[],
            place_animal_world("BIRD", 2, landscape),
            1,
            || place_animal(&[Value::C4Id("BIRD".into())]),
        );
        let final_rng = guard.finish();

        assert_eq!(final_rng, expected_rng);
        assert_eq!(
            result.expect("PlaceAnimal sky-only air succeeds"),
            object_reference_value(ObjectId::new(1))
        );
        assert_eq!(
            outcome.spawns[0].fixed_position,
            Some(FixedVec2::from_ints(raw.x, raw.y))
        );
    }

    #[test]
    fn place_animal_invalid_modes_draw_nothing_and_air_row_zero_draws_once() {
        // C4Id2Def and unsupported Placement return before Random; the Air
        // arm draws x, then fails without Random(y) when row zero is already
        // semi-solid (C4Game.cpp:3028-3061).
        let mut landscape = Landscape::flat(20, 0);
        landscape.set_world_height(20);
        let definitions = HashMap::from([
            (
                DefinitionId::from("UNSP"),
                DefinitionMetadata {
                    placement: 9,
                    ..DefinitionMetadata::default()
                },
            ),
            (
                DefinitionId::from("AIR0"),
                DefinitionMetadata {
                    placement: 2,
                    shape: Some(DefinitionRect::new(-2, -3, 4, 6)),
                    ..DefinitionMetadata::default()
                },
            ),
            (
                DefinitionId::from("SRF0"),
                DefinitionMetadata {
                    placement: 0,
                    shape: Some(DefinitionRect::new(-2, -3, 4, 6)),
                    ..DefinitionMetadata::default()
                },
            ),
        ]);
        let world = HostWorldContext::with_landscape(
            Vec::<HostWorldObject>::new(),
            Some(landscape),
            definitions,
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            1,
            false,
        );
        let mut expected_rng = LcgRng::new(41);
        let _ = expected_rng.random(20);
        let _ = expected_rng.random(20);
        let _ = expected_rng.random(20);
        let expected_after = expected_rng.random(1_000);
        let guard = enter_random_context(LcgRng::new(41));
        let (result, outcome) = with_effect_context(None, &[], world, 1, || {
            Ok::<_, RuntimeError>(Value::Array(vec![
                place_animal(&[Value::C4Id("NONE".into())])?,
                place_animal(&[Value::C4Id("UNSP".into())])?,
                place_animal(&[Value::C4Id("AIR0".into())])?,
                place_animal(&[Value::C4Id("SRF0".into())])?,
                random(&[Value::Int(1_000)])?,
            ]))
        });
        let final_rng = guard.finish();

        assert_eq!(final_rng, expected_rng);
        assert_eq!(
            result.expect("PlaceAnimal failure paths succeed"),
            Value::Array(vec![
                Value::Nil,
                Value::Nil,
                Value::Nil,
                Value::Nil,
                Value::Int(expected_after),
            ])
        );
        assert!(outcome.spawns.is_empty());
    }

    #[test]
    fn place_animal_callbacks_are_synchronous_and_removal_returns_nil() {
        // CreateObject inserts the animal before Construction(nil) at Con=0,
        // then DoCon(FullCon,true) calls Completion and Initialize; removal
        // in Construction makes PlaceAnimal return null while consuming the
        // object number (C4Game.cpp:1102-1146,3028-3061;
        // C4Object.cpp:1428-1515).
        let animal_script = r#"#strict
local iConstructionCon, iConstructionY, iOrder;
protected func Construction() { iConstructionCon=GetCon(); iConstructionY=GetY(); iOrder=1; }
protected func Completion() { iOrder=iOrder*10+2; }
protected func Initialize() { iOrder=iOrder*10+3; }
public func ConstructionCon() { return(iConstructionCon); }
"#;
        let caller_script = r#"#strict
local iObservedCon;
public func Seed() { var child=PlaceAnimal(ANML); iObservedCon=child->ConstructionCon(); return(child); }
public func SeedRemoved() { return(PlaceAnimal(DIEA)); }
"#;
        let mut engine = crate::Engine::with_seed(53);
        let mut landscape = Landscape::flat(100, 60);
        landscape.set_world_height(100);
        engine.set_landscape(landscape);
        let mut animal = crate::Definition::from_script("ANML", "Animal", animal_script)
            .expect("animal script compiles");
        animal.set_category(crate::CATEGORY_LIVING);
        animal.set_shape_rect(Some(DefinitionRect::new(-2, -3, 4, 6)));
        animal.set_placement(2);
        engine
            .register_definition(animal)
            .expect("animal registers");
        let mut removed = crate::Definition::from_script(
            "DIEA",
            "Removed animal",
            "#strict\nprotected func Construction() { RemoveObject(); }",
        )
        .expect("removed animal script compiles");
        removed.set_category(crate::CATEGORY_LIVING);
        removed.set_shape_rect(Some(DefinitionRect::new(-2, -3, 4, 6)));
        removed.set_placement(2);
        engine
            .register_definition(removed)
            .expect("removed animal registers");
        let caller = crate::Definition::from_script("CALL", "Caller", caller_script)
            .expect("caller script compiles");
        engine
            .register_definition(caller)
            .expect("caller registers");
        let caller_id = engine
            .spawn_object(SpawnConfig::new("CALL").with_position(Vector2::new(1_000, 1_000)))
            .expect("caller spawns");
        let caller_index = engine.find_object_index(caller_id).expect("caller exists");

        let value = engine
            .call_object_function(caller_index, "Seed", Vec::new())
            .expect("Seed runs");
        let animal_id = object_id_from_value(&value).expect("Seed returns live animal");
        let animal_index = engine.find_object_index(animal_id).expect("animal exists");
        let animal = &engine.objects[animal_index].state;
        assert_eq!(animal.owner, OWNER_NONE);
        assert_eq!(animal.controller, OWNER_NONE);
        assert_eq!(animal.construction, FULL_CON);
        assert!((0..100).contains(&animal.position.x), "placement is global");
        assert_eq!(
            animal.local_vars.get("iConstructionCon"),
            Some(&Value::Int(0))
        );
        assert_eq!(animal.local_vars.get("iOrder"), Some(&Value::Int(123)));
        assert_eq!(
            animal.local_vars.get("iConstructionY"),
            Some(&Value::Int(animal.position.y + 3)),
            "Construction sees the raw pre-DoCon y"
        );
        let caller_index = engine.find_object_index(caller_id).expect("caller remains");
        assert_eq!(
            engine.objects[caller_index]
                .state
                .local_vars
                .get("iObservedCon"),
            Some(&Value::Int(0)),
            "Construction writes are visible before PlaceAnimal returns"
        );

        let before_next_id = engine.capture_state().next_object_id;
        let value = engine
            .call_object_function(caller_index, "SeedRemoved", Vec::new())
            .expect("SeedRemoved runs");
        assert_eq!(value, Value::Nil);
        assert!(engine
            .snapshot()
            .objects
            .iter()
            .all(|object| object.definition_id != "DIEA"));
        assert_eq!(engine.capture_state().next_object_id, before_next_id + 1);
    }

