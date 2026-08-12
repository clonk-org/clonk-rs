use std::error::Error;

use crate::support::real_scenario::{prepare_installed_scenario, PreparedInstalledScenario};
use crate::support::virtual_player::VirtualPlayer;
use clonk_engine::math::{fixed100, FixedVec2};
use clonk_engine::{
    Direction, EffectVarValue, Engine, JoinPlayerConfig, ObjectId, ObjectUpdate, PlayerState,
    COM_CURSOR_RIGHT, COM_DOWN, COM_LEFT, COM_RIGHT, COM_THROW, OWNER_NONE,
    PLAYER_VIEW_MODE_CURSOR, PLAYER_VIEW_MODE_TARGET,
};
use clonk_script::Value;

fn instantiate_tutorial05_with_controls(
    prepared: &PreparedInstalledScenario,
    control_style: bool,
    auto_context_menu: bool,
) -> (Engine, i32) {
    let mut engine = prepared.instantiate();
    let owner = engine
        .join_player(JoinPlayerConfig {
            name: "Tutorial 5 virtual player".to_owned(),
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
            control_style,
            auto_context_menu,
            startup_player_count: 1,
        })
        .expect("local Tutorial05 virtual player joins")
        .number();
    (engine, owner)
}
fn instantiate_tutorial05(prepared: &PreparedInstalledScenario) -> (Engine, i32) {
    instantiate_tutorial05_with_controls(prepared, false, false)
}

fn object_with_definition(engine: &Engine, definition: &str) -> Option<ObjectId> {
    engine.first_object_for_definition(definition)
}

fn object_with_definition_near_x(
    engine: &Engine,
    definition: &str,
    expected_x: i32,
) -> Option<ObjectId> {
    engine
        .snapshot()
        .objects
        .into_iter()
        .filter(|object| object.definition_id == definition)
        .min_by_key(|object| (object.position.x - expected_x).abs())
        .map(|object| object.id)
}

fn tutorial_message_contains(engine: &Engine, needle: &str) -> bool {
    engine.message_line_contains(needle)
}

fn player_state(engine: &Engine, owner: i32) -> PlayerState {
    engine
        .snapshot()
        .players
        .into_iter()
        .find(|player| player.id == owner)
        .expect("Tutorial05 player remains registered")
}

fn first_viewport_focus(engine: &Engine, owner: i32) -> Option<ObjectId> {
    player_state(engine, owner)
        .viewports
        .first()
        .and_then(|viewport| viewport.focus)
}

#[test]
fn tutorial05_jump_and_run_held_down_tensions_and_fires_real_catapult() -> Result<(), Box<dyn Error>>
{
    // C4Object::CallControl notifies the pushed CATA's ControlUpdate after
    // every Jump'n'Run input edge. CATA forwards the held ComDir to the real
    // JumpAndRun.c AimUpdate helper, whose IntJnRAim timer advances one Ready
    // phase every eight frames (C4Object.cpp:3313-3323;
    // Catapult.c4d/Script.c:121-163; planet/System.c4g/JumpAndRun.c:53-119).
    let prepared = prepare_installed_scenario("Tutorial.c4f/Tutorial05.c4s", 0);
    let (mut engine, owner) = instantiate_tutorial05_with_controls(&prepared, true, true);
    let constructor = engine
        .crew_cursor(owner)
        .expect("Tutorial05 starts on its constructor CLNK");
    let elevator = object_with_definition(&engine, "ELEV").expect("Tutorial05 creates ELEV");
    let valley_cata = object_with_definition_near_x(&engine, "CATA", 240)
        .expect("Tutorial05 creates its valley CATA");
    let metal = object_with_definition_near_x(&engine, "METL", 285)
        .expect("Tutorial05 creates its valley METL");
    let mut player = VirtualPlayer::new(&mut engine, owner);

    player.wait_until(
        "Tutorial05 teaches selection after ELEV stalls at eighty percent",
        800,
        |engine| {
            tutorial_message_contains(engine, "'select right'")
                && engine
                    .object_snapshot(elevator)
                    .is_some_and(|object| object.construction == 80_000)
        },
    )?;
    player.tap(COM_CURSOR_RIGHT)?;
    let valley = player
        .engine()
        .crew_cursor(owner)
        .expect("physical CursorRight selects the valley CLNK");
    player.assert_milestone(
        "physical CursorRight selects the real valley CLNK",
        |engine| {
            valley != constructor
                && engine.object_snapshot(valley).is_some_and(|object| {
                    (200..300).contains(&object.position.x) && object.position.y >= 350
                })
        },
    )?;

    player.wait_until(
        "Tutorial05 asks the valley CLNK to collect material",
        240,
        |engine| tutorial_message_contains(engine, "collect either the wood or the metal"),
    )?;
    // The default C++ route is deterministic: although Script.c creates WOOD
    // before METL, C4ObjectList's same-category sort inserts the later,
    // distinct METL before WOOD. CrossCheck visits that sorted sector list,
    // so the valley Clonk collects METL first (Tutorial05/Script.c:34-43;
    // C4ObjectList.cpp:110-222; C4GameObjects.cpp:150-191).
    player.hold_until(
        COM_RIGHT,
        "the AutoStop valley CLNK naturally collects the exact METL",
        160,
        |engine| {
            engine
                .object_snapshot(metal)
                .is_some_and(|object| object.container == Some(valley))
        },
    )?;
    player.wait_until("Tutorial05 points back to the valley CATA", 240, |engine| {
        tutorial_message_contains(engine, "stand in front of the catapult")
    })?;
    player.hold_until(
        COM_LEFT,
        "the METL-carrying AutoStop CLNK reaches the valley CATA",
        160,
        |engine| {
            engine
                .object_snapshot(valley)
                .zip(engine.object_snapshot(valley_cata))
                .is_some_and(|(clonk, cata)| {
                    clonk.action.name == "Walk" && (clonk.position.x - cata.position.x).abs() <= 12
                })
        },
    )?;
    player.tap(COM_DOWN)?;
    player.wait_until(
        "single AutoStop Down grabs the real valley CATA",
        80,
        |engine| {
            engine.object_snapshot(valley).is_some_and(|object| {
                object.action.name == "Push" && object.action.target == Some(valley_cata)
            })
        },
    )?;

    // DirectCom normally copies the operator Controller when the grab input
    // arrives, masking ExecAction's independent C++ assignment. Clear it,
    // then advance with no new input: every successful DFA_PUSH tick must
    // restore the pushing CLNK's Controller before later range checks
    // (C4Object.cpp:5076-5089). CATA::Fire uses this attribution for its
    // projectile and player-view target (Catapult.c4d/Script.c:34-77).
    drop(player);
    let mut clear_controller = ObjectUpdate::new();
    clear_controller.controller = Some(OWNER_NONE);
    engine.apply_object_update(valley_cata, clear_controller)?;
    let mut player = VirtualPlayer::new(&mut engine, owner);
    player.ticks(1)?;
    assert_eq!(
        player
            .engine()
            .object_snapshot(valley_cata)
            .expect("the real valley CATA survives sustained PUSH")
            .controller,
        owner,
        "sustained DFA_PUSH restores real CATA attribution without an input edge"
    );

    player.wait_until(
        "Tutorial05 asks the AutoStop CLNK to load CATA",
        300,
        |engine| tutorial_message_contains(engine, "Press 'throw' to load the catapult"),
    )?;
    player.tap(COM_THROW)?;
    player.wait_until(
        "physical Throw loads the exact METL into CATA",
        80,
        |engine| {
            engine
                .object_snapshot(metal)
                .is_some_and(|object| object.container == Some(valley_cata))
        },
    )?;
    player.wait_until(
        "Tutorial05 asks the AutoStop CLNK to tension CATA",
        300,
        |engine| tutorial_message_contains(engine, "fully tensioned"),
    )?;

    player.assert_milestone("the selected valley CLNK pushes the real CATA", |engine| {
        engine.object_snapshot(valley).is_some_and(|object| {
            object.action.name == "Push" && object.action.target == Some(valley_cata)
        })
    })?;

    player.press(COM_DOWN)?;
    let pressed = player
        .engine()
        .object_snapshot(valley_cata)
        .expect("valley CATA survives held Down");
    let pressed_effect = pressed
        .effects
        .iter()
        .find(|effect| effect.name == "IntJnRAim" && effect.priority != 0)
        .unwrap_or_else(|| {
            panic!(
                "held Jump'n'Run Down must synchronously create IntJnRAim; \
                 CATA action={:?}, CLNK action={:?}, CATA effects={:?}",
                pressed.action,
                player
                    .engine()
                    .object_snapshot(valley)
                    .map(|object| object.action),
                pressed.effects
            )
        });
    assert_eq!(pressed_effect.priority, 1, "AimUpdate effect priority");
    assert_eq!(pressed_effect.interval, 8, "CATA aim cadence");
    assert_eq!(pressed_effect.timer, 0, "new aim timer starts at zero");
    assert!(
        pressed_effect.start_dispatched,
        "priority-one FxIntJnRAimStart must run synchronously"
    );
    assert_eq!(
        pressed_effect.vars.first(),
        Some(&EffectVarValue::String("ControlConf".to_owned().into())),
        "FxIntJnRAimStart stores CATA's configuration callback; effect={pressed_effect:?}"
    );
    assert_eq!(pressed_effect.vars.get(1), Some(&EffectVarValue::Int(1)));
    assert_eq!(
        pressed_effect.vars.get(2),
        Some(&EffectVarValue::Object(valley.as_u64())),
        "FxIntJnRAimStart stores the pushing CLNK"
    );

    for elapsed in 1_i32..=52 {
        player.ticks(1)?;
        let cata = player
            .engine()
            .object_snapshot(valley_cata)
            .expect("valley CATA survives its aim timer");
        let effect = cata
            .effects
            .iter()
            .find(|effect| effect.name == "IntJnRAim" && effect.priority != 0)
            .unwrap_or_else(|| {
                panic!(
                    "IntJnRAim disappeared while Down remained held at tick {elapsed}; \
                     CATA action={:?}, CLNK action={:?}, effects={:?}",
                    cata.action,
                    player
                        .engine()
                        .object_snapshot(valley)
                        .map(|object| object.action),
                    cata.effects
                )
            });
        // ExecuteControl synthesizes DownSingle after C4DoubleClick. Its
        // CallControl still invokes ControlUpdate, so AimCancel recreates
        // IntJnRAim at elapsed 11 with timer zero while preserving phase 1
        // (C4Player.cpp:1215-1229; C4Object.cpp:3327-3337).
        let expected_timer = if elapsed <= 10 { elapsed } else { elapsed - 11 };
        let expected_phase = if elapsed < 8 {
            0
        } else if elapsed <= 10 {
            1
        } else {
            1 + ((elapsed - 11) / 8).min(5)
        };
        assert_eq!(
            effect.timer, expected_timer,
            "IntJnRAim timer mismatch at held-Down tick {elapsed}; CATA={cata:?}"
        );
        assert_eq!(
            cata.action.phase,
            expected_phase,
            "CATA must advance exactly at 8-frame boundaries while Down is held; \
             elapsed={elapsed}, effect={effect:?}, CLNK={:?}",
            player.engine().object_snapshot(valley)
        );
    }

    player.release(COM_DOWN)?;
    let released = player
        .engine()
        .object_snapshot(valley_cata)
        .expect("valley CATA survives Down release");
    assert!(
        released
            .effects
            .iter()
            .all(|effect| effect.name != "IntJnRAim" || effect.priority == 0),
        "released ControlUpdate/COMD_Stop must synchronously AimCancel; CATA={released:?}"
    );
    assert!(released
        .effects
        .iter()
        .any(|effect| effect.name == "IntJnRAim" && effect.priority == 0));
    assert_eq!(released.action.phase, 6, "release preserves full tension");

    player.tap(COM_THROW)?;
    assert_eq!(
        first_viewport_focus(player.engine(), owner),
        Some(valley),
        "CATA::Fire keeps the local player's view on its pushing CLNK until Projectile runs"
    );
    let firing = player
        .engine()
        .object_snapshot(valley_cata)
        .expect("valley CATA survives firing");
    assert_eq!(firing.action.name, "Fire", "ControlThrow starts Fire");
    assert_eq!(firing.action.phase, 1, "Fire starts at 7 - iPhase");
    assert_eq!(
        firing.action.target2,
        Some(valley),
        "local C++ GetPlrViewMode returns cursor mode 0, so CATA::Fire stores the pushing CLNK in action target 2 (C4Script.cpp:2579-2584; Catapult.c4d/Script.c:38-47)"
    );
    let pusher = player
        .engine()
        .object_snapshot(valley)
        .expect("pushing valley CLNK survives firing");
    assert!(
        pusher.command_stack.is_empty(),
        "truthy CATA::Fire must consume COM_THROW before the CLNK drop fallback; CLNK={pusher:?}"
    );
    player.wait_until(
        "real CATA Projectile ejects its METL after the Fire animation",
        20,
        |engine| {
            engine
                .object_snapshot(metal)
                .is_some_and(|object| object.container.is_none())
        },
    )?;
    assert!(player
        .engine()
        .object_snapshot(valley_cata)
        .expect("valley CATA survives projectile launch")
        .effects
        .iter()
        .all(|effect| effect.name != "IntJnRAim"));
    let launched = player
        .engine()
        .object_snapshot(metal)
        .expect("the exact loaded METL survives Projectile");
    assert_eq!(
        launched.controller, owner,
        "CATA::Projectile assigns the firing player to its exact payload"
    );
    assert!(
        launched.container.is_none()
            && launched.mobile
            && launched.velocity.x > 0
            && launched.velocity.y < 0,
        "full right-facing tension must eject the exact METL up and right; METL={launched:?}"
    );
    for flight_frame in 0..=2 {
        let metal_position = player
            .engine()
            .object_snapshot(metal)
            .expect("launched METL remains live")
            .position;
        let view = player_state(player.engine(), owner);
        let viewport = view.viewports.first().expect("player has a live viewport");
        assert_eq!(view.view_mode, PLAYER_VIEW_MODE_TARGET);
        assert_eq!(view.view_target, Some(metal));
        assert_eq!(
            viewport.focus,
            Some(valley),
            "SetPlrView does not replace the cursor/HUD focus"
        );
        assert_eq!(
            viewport.center, metal_position,
            "C4Player::UpdateView must track the exact METL center on flight frame {flight_frame}"
        );
        if flight_frame < 2 {
            player.ticks(1)?;
        }
    }

    let target_center = player_state(player.engine(), owner).viewports[0].center;
    let saved = player.engine().capture_state();
    let saved_player = saved
        .players
        .iter()
        .find(|state| state.id == owner)
        .expect("saved player exists");
    assert_eq!(saved_player.view_mode, PLAYER_VIEW_MODE_TARGET);
    assert_eq!(saved_player.view_target, None, "ViewTarget is NO-SAVE");
    assert_eq!(saved_player.viewports[0].focus, Some(valley));
    assert_eq!(saved_player.viewports[0].center, target_center);
    assert!(
        !saved.to_json_string()?.contains("view_target"),
        "serialized engine state must not contain the transient ViewTarget field"
    );
    let (mut restored, restored_owner) =
        instantiate_tutorial05_with_controls(&prepared, true, true);
    assert_eq!(restored_owner, owner);
    restored.restore_state(&saved)?;
    let restored_view = player_state(&restored, owner);
    assert_eq!(restored_view.view_mode, PLAYER_VIEW_MODE_TARGET);
    assert_eq!(restored_view.view_target, None);
    assert_eq!(restored_view.viewports[0].focus, Some(valley));
    assert_eq!(
        restored_view.viewports[0].center, target_center,
        "a target-mode save keeps ViewX/ViewY but must not resurrect ViewTarget"
    );

    let center_before_reset = player_state(player.engine(), owner).viewports[0].center;
    player.press(COM_LEFT)?;
    let reset = player_state(player.engine(), owner);
    assert_eq!(reset.view_mode, PLAYER_VIEW_MODE_CURSOR);
    assert_eq!(reset.view_target, None);
    assert_eq!(
        reset.viewports[0].center, center_before_reset,
        "ResetCursorView changes mode immediately but ViewX/ViewY update in the later player phase"
    );
    player.ticks(1)?;
    let valley_position = player
        .engine()
        .object_snapshot(valley)
        .expect("valley CLNK survives reset")
        .position;
    let reset = player_state(player.engine(), owner);
    assert_eq!(
        reset.viewports[0].focus,
        Some(valley),
        "cursor/HUD focus remains the selected valley CLNK"
    );
    assert_eq!(
        reset.viewports[0].center, valley_position,
        "the next player phase returns ViewX/ViewY from METL to the selected CLNK"
    );
    player.release(COM_LEFT)?;

    // Projectile exits the exact contained object with full-phase (+8,-12)
    // velocity, then applies its one synchronized RandomX deviation before
    // ordinary FLIGHT movement settles it on the right hill
    // (Catapult.c4d/Script.c:48-77; C4Movement.cpp:220-445).
    player.wait_until(
        "the AutoStop-fired METL crosses Tutorial05's right-hill rectangle",
        400,
        |engine| {
            engine.object_snapshot(metal).is_some_and(|object| {
                object.container.is_none()
                    && (460..640).contains(&object.position.x)
                    && (150..290).contains(&object.position.y)
            })
        },
    )?;
    player.wait_until(
        "the exact AutoStop-fired METL settles on the right hill",
        300,
        |engine| {
            engine.object_snapshot(metal).is_some_and(|object| {
                object.container.is_none()
                    && !object.mobile
                    && (460..640).contains(&object.position.x)
                    && (150..290).contains(&object.position.y)
            })
        },
    )?;

    Ok(())
}

#[test]
fn tutorial05_shared_scenario_subcases_batch_1() {
    let prepared = prepare_installed_scenario("Tutorial.c4f/Tutorial05.c4s", 0);
    let mut failures = Vec::new();

    run_tutorial05_subcase(
        "catapult_restores_its_partial_tension_after_firing",
        &mut failures,
        || {
            tutorial05_catapult_restores_its_partial_tension_after_firing(&prepared)
                .expect("partial-tension catapult subcase succeeds")
        },
    );
    run_tutorial05_subcase(
        "cpp_crew_order_starts_at_constructor_then_cycles_to_valley",
        &mut failures,
        || tutorial05_cpp_crew_order_starts_at_constructor_then_cycles_to_valley(&prepared),
    );

    assert_no_tutorial05_subcase_failures(failures);
}

#[test]
fn tutorial05_shared_scenario_subcases_batch_2() {
    let prepared = prepare_installed_scenario("Tutorial.c4f/Tutorial05.c4s", 0);
    let mut failures = Vec::new();

    run_tutorial05_subcase(
        "partial_elevator_starts_with_its_built_component_fraction",
        &mut failures,
        || tutorial05_partial_elevator_starts_with_its_built_component_fraction(&prepared),
    );
    run_tutorial05_subcase(
        "cnmt_rule_stalls_the_unfed_elevator_at_eighty_percent",
        &mut failures,
        || tutorial05_cnmt_rule_stalls_the_unfed_elevator_at_eighty_percent(&prepared),
    );

    assert_no_tutorial05_subcase_failures(failures);
}

fn run_tutorial05_subcase(
    name: &'static str,
    failures: &mut Vec<&'static str>,
    subcase: impl FnOnce(),
) {
    eprintln!("running shared Tutorial05 subcase `{name}`");
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(subcase)).is_err() {
        eprintln!("shared Tutorial05 subcase `{name}` failed; continuing batch");
        failures.push(name);
    }
}

fn assert_no_tutorial05_subcase_failures(failures: Vec<&str>) {
    assert!(
        failures.is_empty(),
        "Tutorial05 subcase(s) failed: {}",
        failures.join(", ")
    );
}

fn tutorial05_catapult_restores_its_partial_tension_after_firing(
    prepared: &PreparedInstalledScenario,
) -> Result<(), Box<dyn Error>> {
    // CATA stores every successful ControlConf phase in iPhase. Fire starts
    // at 7-iPhase, its ActMap transitions Fire -> Charge, and Charging stops
    // the rewind at that same iPhase (Catapult.c4d/Script.c:31-43,51-74,
    // 134-140; Catapult.c4d/ActMap.txt:11-32). Therefore a phase-three shot
    // launches at (+/-4,-6), with one shared RandomX(-50,+50) hundredth-pixel
    // deviation, and returns to Ready phase three rather than full tension.
    let (mut engine, _) = instantiate_tutorial05(prepared);
    let catapult = object_with_definition_near_x(&engine, "CATA", 240)
        .expect("Tutorial05 creates its real valley CATA");
    let payload = object_with_definition_near_x(&engine, "METL", 285)
        .expect("Tutorial05 creates its real valley METL");
    engine
        .apply_object_update(payload, ObjectUpdate::new().with_container(catapult))
        .expect("place the real METL inside the real CATA");
    let catapult_index = engine
        .find_object_index(catapult)
        .expect("the real CATA remains indexed");

    for expected_phase in 1..=3 {
        engine.call_object_function(catapult_index, "ControlConf", vec![Value::Int(1)])?;
        let tensioned = engine
            .object_snapshot(catapult)
            .expect("the real CATA survives ControlConf");
        assert_eq!(
            (tensioned.action.name.as_str(), tensioned.action.phase),
            ("Ready", expected_phase)
        );
        assert_eq!(
            tensioned.local_vars.get("iPhase"),
            Some(&Value::Int(expected_phase)),
            "ControlConf must retain the C++ launch phase"
        );
    }

    let direction = engine
        .object_snapshot(catapult)
        .expect("the partially tensioned CATA survives")
        .direction;
    assert_eq!(
        engine.call_object_function(catapult_index, "ControlThrow", Vec::new())?,
        Value::Int(1),
        "a tensioned, loaded CATA consumes Throw"
    );
    let firing = engine
        .object_snapshot(catapult)
        .expect("the real CATA enters Fire");
    assert_eq!(
        (firing.action.name.as_str(), firing.action.phase),
        ("Fire", 4),
        "CATA::Fire starts at 7-iPhase"
    );

    let launched = (0..4)
        .find_map(|_| {
            engine
                .tick_without_snapshot()
                .expect("advance the real CATA firing animation");
            engine
                .object_snapshot(payload)
                .filter(|object| object.container.is_none())
        })
        .expect("Fire's natural EndCall ejects the real METL");
    let fixed_velocity = launched
        .fixed_velocity
        .unwrap_or_else(|| FixedVec2::from_ints(launched.velocity.x, launched.velocity.y));
    let base_x = if direction == Direction::Right {
        400
    } else {
        -400
    };
    let deviation = (-50..=50)
        .find(|deviation| fixed100(base_x + deviation) == fixed_velocity.x)
        .expect("partial-tension xdir contains C++'s bounded RandomX deviation");
    assert_eq!(
        fixed_velocity.y,
        fixed100(-600 + deviation) + engine.physics().gravity_as_c4fixed(),
        "the same deviation adjusts xdir and ydir before the payload's one ordinary gravity step"
    );

    for _ in 0..16 {
        if engine
            .object_snapshot(catapult)
            .is_some_and(|object| object.action.name == "Ready" && object.action.phase == 3)
        {
            break;
        }
        engine.tick_without_snapshot()?;
    }
    let recharged = engine
        .object_snapshot(catapult)
        .expect("the real CATA survives its Charge action");
    assert_eq!(
        (recharged.action.name.as_str(), recharged.action.phase),
        ("Ready", 3),
        "Charge must restore the fired partial tension, not phase six"
    );
    assert_eq!(recharged.local_vars.get("iPhase"), Some(&Value::Int(3)));

    Ok(())
}

fn tutorial05_partial_elevator_starts_with_its_built_component_fraction(
    prepared: &PreparedInstalledScenario,
) {
    // NewObject's initial DoCon calls ComponentConGain
    // (C4Object.cpp:1428-1465, especially :1464; :519-526). At 80% the
    // real ELEV therefore already owns floor(4*80%) WOOD and floor(2*80%)
    // METL; the player only has to deliver the remaining one of each.
    let (engine, _) = instantiate_tutorial05(prepared);
    let elevator = engine
        .snapshot()
        .objects
        .into_iter()
        .find(|object| object.definition_id == "ELEV")
        .expect("Tutorial05 creates its elevator construction");
    assert_eq!(elevator.construction, 80_000);
    assert_eq!(elevator.components.get("WOOD"), Some(&3));
    assert_eq!(elevator.components.get("METL"), Some(&1));
}

// InitRules creates the scenario's CNMT object and UpdateRules maps its
// presence to C4RULE_ConstructionNeedsMaterial (C4Game.cpp:4016-4046).
// C4Object::Build then refuses to advance past the component ratio while no
// full-con material is available (C4Object.cpp:1690-1738). Tutorial05 relies
// on that stall before teaching the player to catapult WOOD and METL uphill.
fn tutorial05_cnmt_rule_stalls_the_unfed_elevator_at_eighty_percent(
    prepared: &PreparedInstalledScenario,
) {
    let (mut engine, _) = instantiate_tutorial05(prepared);
    let elevator = engine
        .snapshot()
        .objects
        .into_iter()
        .find(|object| object.definition_id == "ELEV")
        .expect("Tutorial05 creates its elevator construction");
    assert_eq!(elevator.construction, 80_000);

    // Script1 naturally commands the first CLNK to build. No controls or
    // state injection supply the missing fourth WOOD or second METL.
    for _ in 0..240 {
        engine
            .tick_without_snapshot()
            .expect("Tutorial05 opening frame");
    }

    let stalled = engine
        .object_snapshot(elevator.id)
        .expect("the elevator construction survives");
    assert_eq!(
        stalled.construction, 80_000,
        "CNMT must prevent free construction progress"
    );
}

fn tutorial05_cpp_crew_order_starts_at_constructor_then_cycles_to_valley(
    prepared: &PreparedInstalledScenario,
) {
    // PlaceReadyCrew adds each equal-definition CLNK to C4Player::Crew with
    // stMain ordering, so the newest recruit is first. Tutorial05 binds
    // GetCrew(plr,0) as the constructor and GetCrew(plr,1) as the valley
    // Clonk, then C4Player::AdjustCursorCommand chooses that first equal-rank
    // crew member (C4Player.cpp:481-570,1003-1020,1235-1258;
    // C4ObjectList.cpp:110-195; Tutorial05/Script.c:32-39).
    let (mut engine, owner) = instantiate_tutorial05(prepared);
    let constructor = engine
        .crew_cursor(owner)
        .and_then(|id| engine.object_snapshot(id))
        .expect("Tutorial05 starts with a crew cursor");
    assert!(
        constructor.position.x < 220 && constructor.position.y < 200,
        "the initial cursor must be the constructor beside ELEV, got {constructor:?}"
    );

    engine
        .player_in_com(owner, COM_CURSOR_RIGHT, 0)
        .expect("real CursorRight control succeeds");
    let valley = engine
        .crew_cursor(owner)
        .and_then(|id| engine.object_snapshot(id))
        .expect("CursorRight selects another crew member");
    assert!(
        (200..300).contains(&valley.position.x) && valley.position.y >= 350,
        "one CursorRight must select the valley Clonk, got {valley:?}"
    );
}
