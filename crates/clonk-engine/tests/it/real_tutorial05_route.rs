#![allow(dead_code)]

use std::error::Error;

use crate::support::real_scenario::{prepare_installed_scenario, PreparedInstalledScenario};
use crate::support::virtual_player::VirtualPlayer;
use clonk_engine::math::{fixed100, FixedVec2};
use clonk_engine::{
    Direction, EffectVarValue, Engine, JoinPlayerConfig, ObjectId, ObjectUpdate, PlayerState,
    COM_CURSOR_RIGHT, COM_CURSOR_TOGGLE, COM_DIG, COM_DOWN, COM_LEFT, COM_RIGHT, COM_THROW, COM_UP,
    OWNER_NONE, PLAYER_VIEW_MODE_CURSOR, PLAYER_VIEW_MODE_TARGET,
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

fn clonk_carries(engine: &Engine, clonk: ObjectId, definition: &str) -> bool {
    engine.object_snapshot(clonk).is_some_and(|clonk| {
        clonk.contents.iter().any(|item| {
            engine
                .object_snapshot(*item)
                .is_some_and(|item| item.definition_id == definition)
        })
    })
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

#[test]
#[ignore = "over-constrained virtual tutorial driver; excluded from parity gates"]
fn tutorial05_virtual_player_completes_the_real_tutorial_route() -> Result<(), Box<dyn Error>> {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();
    let prepared = prepare_installed_scenario("Tutorial.c4f/Tutorial05.c4s", 0);
    let (mut engine, owner) = instantiate_tutorial05(&prepared);
    let constructor = engine
        .crew_cursor(owner)
        .expect("Tutorial05 starts on its constructor CLNK");
    let elevator = object_with_definition(&engine, "ELEV").expect("Tutorial05 creates ELEV");
    let hut = object_with_definition(&engine, "HUT3").expect("Tutorial05 creates HUT3");
    let mut wood = object_with_definition_near_x(&engine, "WOOD", 280)
        .expect("Tutorial05 creates the valley WOOD");
    let mut metal = object_with_definition_near_x(&engine, "METL", 285)
        .expect("Tutorial05 creates the valley METL");
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

    // C4Player::CursorRight follows the stMain crew order frozen by the
    // checkpoint above, selecting Tutorial05's valley_clnk without assigning
    // cursor state (C4Player.cpp:1261-1275; Tutorial05/Script.c:34-40,67-79).
    player.tap(COM_CURSOR_RIGHT)?;
    let valley = player
        .engine()
        .crew_cursor(owner)
        .expect("CursorRight selects the valley CLNK");
    player.assert_milestone("CursorRight selects Tutorial05's valley CLNK", |engine| {
        valley != constructor
            && engine.object_snapshot(valley).is_some_and(|object| {
                (200..300).contains(&object.position.x) && object.position.y >= 350
            })
    })?;

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
        "the valley CLNK naturally collects METL",
        160,
        |engine| clonk_carries(engine, valley, "METL"),
    )?;
    let carried = player
        .engine()
        .object_snapshot(valley)
        .and_then(|object| object.contents.first().copied())
        .expect("the valley CLNK carries the canonical METL");
    assert_eq!(carried, metal, "the C++ sorted-list winner is exact METL");
    std::mem::swap(&mut wood, &mut metal);
    let first_definition = "METL".to_owned();
    let remaining_definition = "WOOD".to_owned();
    let first_component_total = 2;
    player.wait_until("Tutorial05 points back to the valley CATA", 240, |engine| {
        tutorial_message_contains(engine, "stand in front of the catapult")
    })?;
    let valley_cata = object_with_definition_near_x(player.engine(), "CATA", 240)
        .expect("Tutorial05 creates its valley CATA");
    player.hold_until(
        COM_LEFT,
        "the METL-carrying valley CLNK returns to CATA",
        160,
        |engine| {
            engine
                .object_snapshot(valley)
                .zip(engine.object_snapshot(valley_cata))
                .is_some_and(|(clonk, cata)| {
                    clonk.action.name == "Walk" && (clonk.position.x - cata.position.x).abs() <= 6
                })
        },
    )?;
    player.double_tap(COM_DOWN)?;
    player.wait_until("the valley CLNK grabs the real CATA", 80, |engine| {
        engine.object_snapshot(valley).is_some_and(|object| {
            object.action.name == "Push" && object.action.target == Some(valley_cata)
        })
    })?;

    player.wait_until(
        "Tutorial05 asks the valley CLNK to load CATA",
        300,
        |engine| tutorial_message_contains(engine, "Press 'throw' to load the catapult"),
    )?;
    player.tap(COM_THROW)?;
    player.wait_until(
        "METL enters the valley CATA through a real Throw",
        80,
        |engine| {
            engine
                .object_snapshot(wood)
                .is_some_and(|object| object.container == Some(valley_cata))
        },
    )?;
    player.wait_until(
        "Tutorial05 asks the valley CLNK to tension CATA",
        300,
        |engine| tutorial_message_contains(engine, "fully tensioned"),
    )?;

    // Classic CATA::ControlDig calls AimDown once per physical press. Leaving
    // the ten-frame double-click window between presses supplies the six
    // configurations which CATA::ControlConf records as full tension
    // (Objects.c4d/Vehicles.c4d/Catapult.c4d/Script.c:134-147;
    // planet/System.c4g/JumpAndRun.c:29-50,67-76).
    for _ in 0..6 {
        player.tap(COM_DIG)?;
        player.ticks(12)?;
    }
    player.assert_milestone(
        "the valley CATA reaches its full six-phase tension",
        |engine| {
            engine
                .object_snapshot(valley_cata)
                .is_some_and(|object| object.action.name == "Ready" && object.action.phase == 6)
        },
    )?;
    player.tap(COM_THROW)?;
    player.wait_until(
        "the real valley CATA flings METL to the right hill",
        400,
        |engine| {
            engine.object_snapshot(wood).is_some_and(|object| {
                object.container.is_none()
                    && (460..640).contains(&object.position.x)
                    && (150..290).contains(&object.position.y)
            })
        },
    )?;
    player.wait_until("Tutorial05 asks for the right-hill CLNK", 300, |engine| {
        tutorial_message_contains(engine, "switch to the clonk on the right hill")
    })?;
    player.tap(COM_CURSOR_RIGHT)?;
    let catapult_clonk = player
        .engine()
        .crew_cursor(owner)
        .expect("second CursorRight selects the right-hill CLNK");
    player.assert_milestone(
        "second CursorRight selects Tutorial05's right-hill CLNK",
        |engine| {
            catapult_clonk != constructor
                && catapult_clonk != valley
                && engine
                    .object_snapshot(catapult_clonk)
                    .is_some_and(|object| object.position.x >= 450 && object.position.y < 350)
        },
    )?;
    player.wait_until(
        "the flung METL descends into the right-hill collection corridor",
        120,
        |engine| {
            engine.object_snapshot(wood).is_some_and(|object| {
                object.container.is_none()
                    && (460..640).contains(&object.position.x)
                    && object.position.y >= 215
            })
        },
    )?;
    let wood_x = player
        .engine()
        .object_snapshot(wood)
        .expect("flung METL survives")
        .position
        .x;
    let catapult_clonk_x = player
        .engine()
        .object_snapshot(catapult_clonk)
        .expect("right-hill CLNK survives")
        .position
        .x;
    let collect_direction = if wood_x < catapult_clonk_x {
        COM_LEFT
    } else {
        COM_RIGHT
    };
    player.hold_until(
        collect_direction,
        "the right-hill CLNK naturally collects the flung METL",
        200,
        |engine| clonk_carries(engine, catapult_clonk, first_definition.as_str()),
    )?;

    player.wait_until("Tutorial05 points at the right-hill CATA", 300, |engine| {
        tutorial_message_contains(engine, "grab the other catapult")
    })?;
    let hill_cata = object_with_definition_near_x(player.engine(), "CATA", 540)
        .expect("Tutorial05 creates its right-hill CATA");
    let hill_cata_x = player
        .engine()
        .object_snapshot(hill_cata)
        .expect("right-hill CATA survives")
        .position
        .x;
    let catapult_clonk_x = player
        .engine()
        .object_snapshot(catapult_clonk)
        .expect("right-hill CLNK survives")
        .position
        .x;
    let reach_hill_cata = if hill_cata_x < catapult_clonk_x {
        COM_LEFT
    } else {
        COM_RIGHT
    };
    player.hold_until(
        reach_hill_cata,
        "the METL-carrying right-hill CLNK reaches CATA",
        180,
        |engine| {
            engine
                .object_snapshot(catapult_clonk)
                .zip(engine.object_snapshot(hill_cata))
                .is_some_and(|(clonk, cata)| {
                    clonk.action.name == "Walk" && (clonk.position.x - cata.position.x).abs() <= 12
                })
        },
    )?;
    player.double_tap(COM_DOWN)?;
    player.wait_until("the right-hill CLNK grabs its real CATA", 80, |engine| {
        engine
            .object_snapshot(catapult_clonk)
            .is_some_and(|object| {
                object.action.name == "Push" && object.action.target == Some(hill_cata)
            })
    })?;
    player.wait_until(
        "Tutorial05 evaluates the right-hill CATA direction",
        300,
        |engine| {
            tutorial_message_contains(engine, "Turn the catapult around")
                || tutorial_message_contains(engine, "load the catapult")
        },
    )?;
    if player
        .engine()
        .object_snapshot(hill_cata)
        .is_some_and(|object| object.direction != Direction::Left)
    {
        player.hold_until(
            COM_LEFT,
            "pushing left turns the right-hill CATA toward the cabin",
            40,
            |engine| {
                engine
                    .object_snapshot(hill_cata)
                    .is_some_and(|object| object.direction == Direction::Left)
            },
        )?;
    }
    // Script68 waits for the pusher's real ComDir to stop. In classic
    // controls releasing Left does not stop DFA_PUSH, so send the distinct
    // Down control that maps to ComDir Stop while pushing.
    player.tap(COM_DOWN)?;
    player.wait_until(
        "Tutorial05 asks the right-hill CLNK to load CATA",
        300,
        |engine| tutorial_message_contains(engine, "load the catapult"),
    )?;
    player.tap(COM_THROW)?;
    player.wait_until("METL enters the right-hill CATA", 80, |engine| {
        engine
            .object_snapshot(wood)
            .is_some_and(|object| object.container == Some(hill_cata))
    })?;
    player.wait_until(
        "Tutorial05 asks for the shot toward the cabin",
        300,
        |engine| tutorial_message_contains(engine, "Fling the material"),
    )?;
    for _ in 0..6 {
        player.tap(COM_DIG)?;
        player.ticks(12)?;
    }
    player.assert_milestone("the right-hill CATA reaches full tension", |engine| {
        engine
            .object_snapshot(hill_cata)
            .is_some_and(|object| object.action.name == "Ready" && object.action.phase == 6)
    })?;
    player.tap(COM_THROW)?;
    player.wait_until(
        "the right-hill CATA flings METL to the cabin hill",
        400,
        |engine| {
            engine.object_snapshot(wood).is_some_and(|object| {
                object.container.is_none()
                    && (0..220).contains(&object.position.x)
                    && (0..140).contains(&object.position.y)
            })
        },
    )?;
    player.wait_until("Tutorial05 asks for the constructor CLNK", 300, |engine| {
        tutorial_message_contains(engine, "switch back to the clonk near the cabin")
    })?;
    player.tap(COM_CURSOR_RIGHT)?;
    player.assert_milestone(
        "third CursorRight wraps to Tutorial05's constructor CLNK",
        |engine| engine.crew_cursor(owner) == Some(constructor),
    )?;
    player.wait_until(
        "the flung METL descends into the cabin-hill collection corridor",
        120,
        |engine| {
            engine.object_snapshot(wood).is_some_and(|object| {
                object.container.is_none()
                    && (0..220).contains(&object.position.x)
                    && object.position.y >= 75
            })
        },
    )?;
    let wood_x = player
        .engine()
        .object_snapshot(wood)
        .expect("twice-flung METL survives")
        .position
        .x;
    let constructor_x = player
        .engine()
        .object_snapshot(constructor)
        .expect("constructor CLNK survives")
        .position
        .x;
    let collect_direction = if wood_x < constructor_x {
        COM_LEFT
    } else {
        COM_RIGHT
    };
    player.hold_until(
        collect_direction,
        "the constructor CLNK naturally collects the twice-flung METL",
        180,
        |engine| clonk_carries(engine, constructor, first_definition.as_str()),
    )?;
    player.wait_until(
        "Tutorial05 asks the constructor to continue ELEV",
        300,
        |engine| tutorial_message_contains(engine, "continue work on the elevator"),
    )?;
    let elevator_x = player
        .engine()
        .object_snapshot(elevator)
        .expect("ELEV construction survives")
        .position
        .x;
    let constructor_x = player
        .engine()
        .object_snapshot(constructor)
        .expect("constructor CLNK survives")
        .position
        .x;
    let reach_elevator = if elevator_x < constructor_x {
        COM_LEFT
    } else {
        COM_RIGHT
    };
    player.hold_until(
        reach_elevator,
        "the METL-carrying constructor reaches ELEV",
        160,
        |engine| {
            engine
                .object_snapshot(constructor)
                .zip(engine.object_snapshot(elevator))
                .is_some_and(|(clonk, elevator)| {
                    clonk.action.name == "Walk"
                        && (clonk.position.x - elevator.position.x).abs() <= 10
                })
        },
    )?;
    player.double_tap(COM_DOWN)?;
    player.wait_until("ELEV consumes METL and stalls for WOOD", 600, |engine| {
        engine.object_snapshot(elevator).is_some_and(|object| {
            object.construction == 80_000
                && object.components.get(first_definition.as_str()) == Some(&first_component_total)
                && engine.object_snapshot(wood).is_none()
        })
    })?;

    player.wait_until(
        "Tutorial05 asks for the remaining material relay",
        300,
        |engine| tutorial_message_contains(engine, "transport the remaining material"),
    )?;
    player.tap(COM_CURSOR_RIGHT)?;
    player.assert_milestone(
        "CursorRight returns to Tutorial05's valley CLNK",
        |engine| engine.crew_cursor(owner) == Some(valley),
    )?;
    player.double_tap(COM_DOWN)?;
    player.wait_until("the valley CLNK releases CATA for the WOOD", 80, |engine| {
        engine
            .object_snapshot(valley)
            .is_some_and(|object| object.action.name == "Walk")
    })?;
    let metal_x = player
        .engine()
        .object_snapshot(metal)
        .expect("valley WOOD survives")
        .position
        .x;
    let valley_x = player
        .engine()
        .object_snapshot(valley)
        .expect("valley CLNK survives")
        .position
        .x;
    let collect_direction = if metal_x < valley_x {
        COM_LEFT
    } else {
        COM_RIGHT
    };
    player.hold_until(
        collect_direction,
        "the valley CLNK naturally collects WOOD",
        180,
        |engine| clonk_carries(engine, valley, remaining_definition.as_str()),
    )?;
    let valley_cata_x = player
        .engine()
        .object_snapshot(valley_cata)
        .expect("valley CATA survives")
        .position
        .x;
    let valley_x = player
        .engine()
        .object_snapshot(valley)
        .expect("valley CLNK survives")
        .position
        .x;
    let reach_valley_cata = if valley_cata_x < valley_x {
        COM_LEFT
    } else {
        COM_RIGHT
    };
    player.hold_until(
        reach_valley_cata,
        "the WOOD-carrying valley CLNK returns to CATA",
        180,
        |engine| {
            engine
                .object_snapshot(valley)
                .zip(engine.object_snapshot(valley_cata))
                .is_some_and(|(clonk, cata)| {
                    clonk.action.name == "Walk" && (clonk.position.x - cata.position.x).abs() <= 6
                })
        },
    )?;
    player.double_tap(COM_DOWN)?;
    player.wait_until("the valley CLNK re-grabs CATA with WOOD", 80, |engine| {
        engine.object_snapshot(valley).is_some_and(|object| {
            object.action.name == "Push" && object.action.target == Some(valley_cata)
        })
    })?;
    if player
        .engine()
        .object_snapshot(valley_cata)
        .is_some_and(|object| object.direction != Direction::Right)
    {
        player.hold_until(
            COM_RIGHT,
            "pushing right turns the valley CATA toward the right hill for WOOD",
            40,
            |engine| {
                engine
                    .object_snapshot(valley_cata)
                    .is_some_and(|object| object.direction == Direction::Right)
            },
        )?;
    }
    player.wait_until(
        "the valley CATA resets after its METL shot",
        160,
        |engine| {
            engine
                .object_snapshot(valley_cata)
                .is_some_and(|object| object.action.name == "Ready")
        },
    )?;
    player.tap(COM_THROW)?;
    player.wait_until("WOOD enters the valley CATA", 80, |engine| {
        engine
            .object_snapshot(metal)
            .is_some_and(|object| object.container == Some(valley_cata))
    })?;
    for _ in 0..6 {
        if player
            .engine()
            .object_snapshot(valley_cata)
            .is_some_and(|object| object.action.phase == 6)
        {
            break;
        }
        player.tap(COM_DIG)?;
        player.ticks(12)?;
    }
    player.assert_milestone("the valley CATA retains full tension for WOOD", |engine| {
        engine
            .object_snapshot(valley_cata)
            .is_some_and(|object| object.action.phase == 6)
    })?;
    player.tap(COM_THROW)?;
    player.wait_until(
        "the valley CATA flings WOOD to the right hill",
        400,
        |engine| {
            engine.object_snapshot(metal).is_some_and(|object| {
                object.container.is_none()
                    && (460..640).contains(&object.position.x)
                    && (150..290).contains(&object.position.y)
            })
        },
    )?;

    player.tap(COM_CURSOR_RIGHT)?;
    player.assert_milestone(
        "CursorRight selects the right-hill CLNK for WOOD",
        |engine| engine.crew_cursor(owner) == Some(catapult_clonk),
    )?;
    player.double_tap(COM_DOWN)?;
    player.wait_until("the right-hill CLNK releases CATA for WOOD", 80, |engine| {
        engine
            .object_snapshot(catapult_clonk)
            .is_some_and(|object| object.action.name == "Walk")
    })?;
    // C++ collection is based on the carryable crossing the crew member's
    // collection rectangle, independent of the object's Mobile flag
    // (C4GameObjects.cpp:155-196). Start moving under the descending WOOD
    // instead of waiting for a physics-internal rest flag.
    player.wait_until(
        "the flung WOOD descends into the right-hill collection corridor",
        120,
        |engine| {
            engine.object_snapshot(metal).is_some_and(|object| {
                object.container.is_none()
                    && (460..640).contains(&object.position.x)
                    && object.position.y >= 215
            })
        },
    )?;
    // The second projectile can still carry horizontal speed when it enters
    // this corridor. Follow its live position instead of committing to the
    // side it occupied on the first frame: a landscape bounce may otherwise
    // send it behind a Clonk that keeps walking away. Every steering edge is
    // still a physical C4Player::InCom input, and collection still has to
    // happen through C4GameObjects::CrossCheck.
    let mut pursuit_direction = None;
    for _ in 0..180 {
        if player
            .engine()
            .object_snapshot(metal)
            .is_some_and(|object| object.container == Some(catapult_clonk))
        {
            break;
        }
        let metal_x = player
            .engine()
            .object_snapshot(metal)
            .expect("flung WOOD survives during collection")
            .position
            .x;
        let catapult_clonk_x = player
            .engine()
            .object_snapshot(catapult_clonk)
            .expect("right-hill CLNK survives during collection")
            .position
            .x;
        let desired_direction = match metal_x - catapult_clonk_x {
            ..=-6 => Some(COM_LEFT),
            6.. => Some(COM_RIGHT),
            _ => pursuit_direction,
        };
        if desired_direction != pursuit_direction {
            if let Some(direction) = pursuit_direction {
                player.release(direction)?;
            }
            if let Some(direction) = desired_direction {
                player.press(direction)?;
            }
            pursuit_direction = desired_direction;
        }
        player.ticks(1)?;
    }
    if let Some(direction) = pursuit_direction {
        player.release(direction)?;
    }
    player.assert_milestone(
        "the right-hill CLNK naturally collects flung WOOD",
        |engine| {
            engine
                .object_snapshot(metal)
                .is_some_and(|object| object.container == Some(catapult_clonk))
        },
    )?;
    let hill_cata_x = player
        .engine()
        .object_snapshot(hill_cata)
        .expect("right-hill CATA survives")
        .position
        .x;
    let catapult_clonk_x = player
        .engine()
        .object_snapshot(catapult_clonk)
        .expect("right-hill CLNK survives")
        .position
        .x;
    let reach_hill_cata = if hill_cata_x < catapult_clonk_x {
        COM_LEFT
    } else {
        COM_RIGHT
    };
    player.hold_until(
        reach_hill_cata,
        "the WOOD-carrying right-hill CLNK returns to CATA",
        180,
        |engine| {
            engine
                .object_snapshot(catapult_clonk)
                .zip(engine.object_snapshot(hill_cata))
                .is_some_and(|(clonk, cata)| {
                    let distance = (clonk.position.x - cata.position.x).abs();
                    (clonk.action.name == "Walk" && distance <= 6)
                        || (clonk.action.name == "Scale" && distance <= 32)
                })
        },
    )?;
    // Inline C4Command::Drop -> ObjectComPutTake timing gives the second
    // projectile the C++ route and can leave this Clonk attached to the
    // short wall immediately beside CATA. Classic left control intentionally
    // stops a left-facing scaler, so descend with a real physical control and
    // finish the approach on foot instead of relying on the old delayed-Put
    // trajectory to miss the wall.
    if player
        .engine()
        .object_snapshot(catapult_clonk)
        .is_some_and(|clonk| clonk.action.name == "Scale")
    {
        player.hold_until(
            COM_DOWN,
            "the WOOD-carrying right-hill CLNK descends beside CATA",
            80,
            |engine| {
                engine
                    .object_snapshot(catapult_clonk)
                    .is_some_and(|clonk| clonk.action.name != "Scale")
            },
        )?;
        player.hold_until(
            reach_hill_cata,
            "the WOOD-carrying right-hill CLNK finishes the CATA approach",
            120,
            |engine| {
                engine
                    .object_snapshot(catapult_clonk)
                    .zip(engine.object_snapshot(hill_cata))
                    .is_some_and(|(clonk, cata)| {
                        clonk.action.name == "Walk"
                            && (clonk.position.x - cata.position.x).abs() <= 6
                    })
            },
        )?;
    }
    player.double_tap(COM_DOWN)?;
    player.wait_until(
        "the right-hill CLNK re-grabs CATA with WOOD",
        80,
        |engine| {
            engine
                .object_snapshot(catapult_clonk)
                .is_some_and(|object| {
                    object.action.name == "Push" && object.action.target == Some(hill_cata)
                })
        },
    )?;
    if player
        .engine()
        .object_snapshot(hill_cata)
        .is_some_and(|object| object.direction != Direction::Left)
    {
        player.hold_until(
            COM_LEFT,
            "pushing left turns the right-hill CATA toward the cabin for WOOD",
            40,
            |engine| {
                engine
                    .object_snapshot(hill_cata)
                    .is_some_and(|object| object.direction == Direction::Left)
            },
        )?;
    }
    player.wait_until(
        "the right-hill CATA resets after its METL shot",
        160,
        |engine| {
            engine
                .object_snapshot(hill_cata)
                .is_some_and(|object| object.action.name == "Ready")
        },
    )?;
    player.tap(COM_THROW)?;
    player.wait_until("WOOD enters the right-hill CATA", 80, |engine| {
        engine
            .object_snapshot(metal)
            .is_some_and(|object| object.container == Some(hill_cata))
    })?;
    for _ in 0..6 {
        if player
            .engine()
            .object_snapshot(hill_cata)
            .is_some_and(|object| object.action.phase == 6)
        {
            break;
        }
        player.tap(COM_DIG)?;
        player.ticks(12)?;
    }
    player.assert_milestone(
        "the right-hill CATA retains full tension for WOOD",
        |engine| {
            engine
                .object_snapshot(hill_cata)
                .is_some_and(|object| object.action.phase == 6)
        },
    )?;
    player.tap(COM_THROW)?;
    player.wait_until(
        "the right-hill CATA flings WOOD to the cabin hill",
        400,
        |engine| {
            engine.object_snapshot(metal).is_some_and(|object| {
                object.container.is_none()
                    && (0..220).contains(&object.position.x)
                    && (0..140).contains(&object.position.y)
            })
        },
    )?;

    player.tap(COM_CURSOR_RIGHT)?;
    player.assert_milestone(
        "CursorRight returns to the constructor for WOOD",
        |engine| engine.crew_cursor(owner) == Some(constructor),
    )?;
    // As on the right hill, move to collect once the carryable reaches the
    // crew's vertical collection corridor; WOOD can keep its Mobile bit
    // while resting against the landscape.
    player.wait_until(
        "the twice-flung WOOD descends into the cabin-hill collection corridor",
        120,
        |engine| {
            engine.object_snapshot(metal).is_some_and(|object| {
                object.container.is_none()
                    && (0..220).contains(&object.position.x)
                    && object.position.y >= 75
            })
        },
    )?;
    // Take over from Script1's retained Build command and walk through the
    // delivered component. A regular direct movement clears commands, and
    // collection is driven by the carryable crossing the crew rectangle
    // (C4Object.cpp:3381-3383; C4GameObjects.cpp:155-196).
    let metal_x = player
        .engine()
        .object_snapshot(metal)
        .expect("twice-flung WOOD survives")
        .position
        .x;
    let constructor_x = player
        .engine()
        .object_snapshot(constructor)
        .expect("constructor CLNK survives")
        .position
        .x;
    let collect_direction = if metal_x < constructor_x {
        COM_LEFT
    } else {
        COM_RIGHT
    };
    player.hold_until(
        collect_direction,
        "the constructor naturally collects the twice-flung WOOD",
        240,
        |engine| clonk_carries(engine, constructor, remaining_definition.as_str()),
    )?;
    let elevator_x = player
        .engine()
        .object_snapshot(elevator)
        .expect("ELEV construction survives")
        .position
        .x;
    let constructor_x = player
        .engine()
        .object_snapshot(constructor)
        .expect("constructor CLNK survives")
        .position
        .x;
    let reach_elevator = if elevator_x < constructor_x {
        COM_LEFT
    } else {
        COM_RIGHT
    };
    player.hold_until(
        reach_elevator,
        "the WOOD-carrying constructor reaches ELEV",
        180,
        |engine| {
            engine
                .object_snapshot(constructor)
                .zip(engine.object_snapshot(elevator))
                .is_some_and(|(clonk, elevator)| {
                    clonk.action.name == "Walk"
                        && (clonk.position.x - elevator.position.x).abs() <= 10
                })
        },
    )?;
    player.double_tap(COM_DOWN)?;
    player.wait_until(
        "WOOD completes the real ELEV and creates ELEC",
        720,
        |engine| {
            engine
                .object_snapshot(elevator)
                .is_some_and(|object| object.construction == 100_000)
                && object_with_definition(engine, "ELEC").is_some()
        },
    )?;
    let elevator_case =
        object_with_definition(player.engine(), "ELEC").expect("completed ELEV creates ELEC");
    player.wait_until(
        "Tutorial05 asks the constructor to grab ELEC",
        300,
        |engine| tutorial_message_contains(engine, "Grab the elevator case"),
    )?;
    player.double_tap(COM_DOWN)?;
    player.wait_until("the constructor grabs the real ELEC", 80, |engine| {
        engine.object_snapshot(constructor).is_some_and(|object| {
            object.action.name == "Push" && object.action.target == Some(elevator_case)
        })
    })?;
    player.wait_until(
        "Tutorial05 asks the constructor to drill the shaft",
        300,
        |engine| tutorial_message_contains(engine, "start shaft drilling"),
    )?;

    // Classic ELEC exposes drilling on ControlDigDouble. Keep the second
    // physical press held: releasing it calls ControlDigReleased/Halt before
    // the scenario timer can observe Drill (Case.c4d/Script.c:346-359,
    // 612-631; Tutorial05/Script.c:265-286).
    player.tap(COM_DIG)?;
    player.press(COM_DIG)?;
    player.wait_until("ELEC enters its real Drill action", 80, |engine| {
        engine
            .object_snapshot(elevator_case)
            .is_some_and(|object| object.action.name == "Drill")
    })?;
    player.assert_milestone(
        "the constructor is pushing ELEC as drilling starts",
        |engine| {
            engine.object_snapshot(constructor).is_some_and(|object| {
                object.action.name == "Push" && object.action.target == Some(elevator_case)
            })
        },
    )?;
    player.ticks(1)?;
    // Script162 observes ELEC itself, not the constructor's transient Grab
    // command state (Tutorial05/Script.c:280-286). The case's Drilling
    // StartCall keeps Drill while the nearby Push object exists
    // (Objects.c4d/Structures.c4d/Elevator.c4d/Case.c4d/Script.c:256-269).
    player.assert_milestone(
        "ELEC remains in Drill for the scenario callback",
        |engine| {
            engine
                .object_snapshot(elevator_case)
                .is_some_and(|object| object.action.name == "Drill")
        },
    )?;
    player.wait_until(
        "Tutorial05 asks to gather every CLNK in the valley",
        600,
        |engine| tutorial_message_contains(engine, "gather all clonks"),
    )?;
    player.release(COM_DIG)?;

    player.tap(COM_CURSOR_RIGHT)?;
    player.tap(COM_CURSOR_RIGHT)?;
    player.assert_milestone(
        "two CursorRight controls select the right-hill CLNK",
        |engine| engine.crew_cursor(owner) == Some(catapult_clonk),
    )?;
    player.hold_until(
        COM_LEFT,
        "the right-hill CLNK descends into the valley",
        500,
        |engine| {
            engine
                .object_snapshot(catapult_clonk)
                .is_some_and(|object| object.position.y >= 350 && object.action.name == "Walk")
        },
    )?;
    player.wait_until(
        "all three CLNKs stand at the valley bottom",
        600,
        |engine| {
            [constructor, valley, catapult_clonk]
                .into_iter()
                .all(|clonk| {
                    engine.object_snapshot(clonk).is_some_and(|object| {
                        object.position.y >= 350 && object.action.name == "Walk"
                    })
                })
        },
    )?;
    player.wait_until(
        "Tutorial05 asks for double toggle-selection",
        300,
        |engine| tutorial_message_contains(engine, "toggle selection"),
    )?;

    // Script210 asks the player to gather the crew before SelectAll. Move the
    // two valley-side Clonks over the drilled shaft lip individually; doing
    // this after SelectAll would make their Follow commands compete for the
    // narrow platform (Tutorial05/Script.c:303-313).
    player.hold_until(
        COM_LEFT,
        "the right-hill CLNK reaches the drilled shaft lip",
        180,
        |engine| {
            engine
                .object_snapshot(catapult_clonk)
                .is_some_and(|object| object.position.x <= 220 && object.action.name == "Walk")
        },
    )?;
    player.press(COM_LEFT)?;
    player.tap(COM_UP)?;
    player.wait_until(
        "the right-hill CLNK jumps across the shaft lip",
        80,
        |engine| {
            engine
                .object_snapshot(catapult_clonk)
                .is_some_and(|object| object.position.x <= 174)
        },
    )?;
    player.release(COM_LEFT)?;
    // Classic release keeps the current ComDir; Down is the explicit stop
    // control (C4Object.cpp:3406-3556).
    player.tap(COM_DOWN)?;
    player.wait_until("the right-hill CLNK reaches ELEC", 160, |engine| {
        engine
            .object_snapshot(catapult_clonk)
            .zip(engine.object_snapshot(elevator_case))
            .is_some_and(|(clonk, case)| {
                (clonk.position.x - case.position.x).abs() <= 18
                    && (clonk.position.y - case.position.y).abs() <= 22
            })
    })?;
    if let Some(clonk) = player.engine().object_snapshot(catapult_clonk) {
        if clonk.action.name != "Walk" {
            player.hold_until(
                COM_DOWN,
                "the right-hill CLNK scales down onto ELEC",
                160,
                |engine| {
                    engine
                        .object_snapshot(catapult_clonk)
                        .zip(engine.object_snapshot(elevator_case))
                        .is_some_and(|(clonk, case)| {
                            clonk.action.name == "Walk"
                                && (clonk.position.x - case.position.x).abs() <= 18
                                && (clonk.position.y - case.position.y).abs() <= 22
                        })
                },
            )?;
        }
    }

    player.tap(COM_CURSOR_RIGHT)?;
    player.tap(COM_CURSOR_RIGHT)?;
    player.assert_milestone(
        "two CursorRight controls select the valley CLNK",
        |engine| engine.crew_cursor(owner) == Some(valley),
    )?;
    player.hold_until(
        COM_LEFT,
        "the valley CLNK reaches the drilled shaft lip",
        240,
        |engine| {
            engine
                .object_snapshot(valley)
                .is_some_and(|object| object.position.x <= 220 && object.action.name == "Walk")
        },
    )?;
    player.press(COM_LEFT)?;
    player.tap(COM_UP)?;
    player.wait_until("the valley CLNK jumps across the shaft lip", 80, |engine| {
        engine
            .object_snapshot(valley)
            .is_some_and(|object| object.position.x <= 174)
    })?;
    player.release(COM_LEFT)?;
    player.tap(COM_DOWN)?;
    player.wait_until("the valley CLNK reaches ELEC", 160, |engine| {
        engine
            .object_snapshot(valley)
            .zip(engine.object_snapshot(elevator_case))
            .is_some_and(|(clonk, case)| {
                (clonk.position.x - case.position.x).abs() <= 18
                    && (clonk.position.y - case.position.y).abs() <= 22
            })
    })?;
    if let Some(clonk) = player.engine().object_snapshot(valley) {
        if clonk.action.name != "Walk" {
            player.hold_until(
                COM_DOWN,
                "the valley CLNK scales down onto ELEC",
                160,
                |engine| {
                    engine
                        .object_snapshot(valley)
                        .zip(engine.object_snapshot(elevator_case))
                        .is_some_and(|(clonk, case)| {
                            clonk.action.name == "Walk"
                                && (clonk.position.x - case.position.x).abs() <= 18
                                && (clonk.position.y - case.position.y).abs() <= 22
                        })
                },
            )?;
        }
    }

    player.double_tap(COM_DOWN)?;
    player.wait_until("the valley CLNK grabs ELEC for the ascent", 120, |engine| {
        engine.object_snapshot(valley).is_some_and(|object| {
            object.action.name == "Push" && object.action.target == Some(elevator_case)
        })
    })?;
    player.tap(COM_CURSOR_RIGHT)?;
    player.assert_milestone(
        "CursorRight selects the boarded right-hill CLNK",
        |engine| engine.crew_cursor(owner) == Some(catapult_clonk),
    )?;
    player.hold_until(
        COM_RIGHT,
        "the right-hill CLNK centers on ELEC before grabbing",
        40,
        |engine| {
            engine
                .object_snapshot(catapult_clonk)
                .is_some_and(|object| object.position.x >= 161 && object.action.name == "Walk")
        },
    )?;
    player.double_tap(COM_DOWN)?;
    player.wait_until(
        "the right-hill CLNK grabs ELEC for the ascent",
        120,
        |engine| {
            engine
                .object_snapshot(catapult_clonk)
                .is_some_and(|object| {
                    object.action.name == "Push" && object.action.target == Some(elevator_case)
                })
        },
    )?;
    player.tap(COM_CURSOR_RIGHT)?;
    player.assert_milestone("CursorRight returns to the constructor", |engine| {
        engine.crew_cursor(owner) == Some(constructor)
    })?;
    player.double_tap(COM_DOWN)?;
    player.wait_until("the constructor grabs ELEC for the ascent", 120, |engine| {
        engine.object_snapshot(constructor).is_some_and(|object| {
            object.action.name == "Push" && object.action.target == Some(elevator_case)
        })
    })?;

    player.double_tap(COM_CURSOR_TOGGLE)?;
    player.assert_milestone(
        "double toggle-selection selects all Tutorial05 CLNKs",
        |engine| {
            [constructor, valley, catapult_clonk]
                .into_iter()
                .all(|clonk| {
                    engine
                        .object_snapshot(clonk)
                        .is_some_and(|object| object.selected)
                })
        },
    )?;
    player.wait_until(
        "Tutorial05 asks all CLNKs to return to HUT3",
        300,
        |engine| {
            tutorial_message_contains(engine, "move all clonks back into the home base")
                && engine.object_snapshot(hut).is_some()
        },
    )?;

    player.hold_until(
        COM_UP,
        "ELEC carries the selected crew to the cabin hill",
        600,
        |engine| {
            engine
                .object_snapshot(elevator_case)
                .is_some_and(|object| object.position.y <= 105)
        },
    )?;
    player.wait_until(
        "all selected CLNKs arrive at the shaft top",
        240,
        |engine| {
            [constructor, valley, catapult_clonk]
                .into_iter()
                .all(|clonk| {
                    engine
                        .object_snapshot(clonk)
                        .is_some_and(|object| object.position.y <= 130)
                })
        },
    )?;
    player.double_tap(COM_DOWN)?;
    player.wait_until("the constructor releases ELEC at the top", 80, |engine| {
        engine
            .object_snapshot(constructor)
            .is_some_and(|object| object.action.name == "Walk")
    })?;
    player.tap(COM_CURSOR_RIGHT)?;
    player.assert_milestone("CursorRight selects the valley CLNK at the top", |engine| {
        engine.crew_cursor(owner) == Some(valley)
    })?;
    player.double_tap(COM_DOWN)?;
    player.wait_until("the valley CLNK releases ELEC at the top", 80, |engine| {
        engine
            .object_snapshot(valley)
            .is_some_and(|object| object.action.name == "Walk")
    })?;
    player.tap(COM_CURSOR_RIGHT)?;
    player.assert_milestone(
        "CursorRight selects the right-hill CLNK at the top",
        |engine| engine.crew_cursor(owner) == Some(catapult_clonk),
    )?;
    player.double_tap(COM_DOWN)?;
    player.wait_until(
        "the right-hill CLNK releases ELEC at the top",
        80,
        |engine| {
            engine
                .object_snapshot(catapult_clonk)
                .is_some_and(|object| object.action.name == "Walk")
        },
    )?;
    player.press(COM_LEFT)?;
    player.tap(COM_UP)?;
    player.wait_until(
        "the right-hill CLNK jumps over the shaft-top lip",
        80,
        |engine| {
            engine
                .object_snapshot(catapult_clonk)
                .is_some_and(|object| object.position.x <= 145)
        },
    )?;
    player.release(COM_LEFT)?;
    player.wait_until(
        "the right-hill CLNK lands on the cabin plateau",
        160,
        |engine| {
            engine
                .object_snapshot(catapult_clonk)
                .is_some_and(|object| {
                    object.action.name == "Walk"
                        && object.position.x < 155
                        && object.position.y <= 115
                })
        },
    )?;
    player.tap(COM_CURSOR_RIGHT)?;
    player.assert_milestone(
        "CursorRight returns to the constructor at the top",
        |engine| engine.crew_cursor(owner) == Some(constructor),
    )?;
    player.press(COM_LEFT)?;
    player.tap(COM_UP)?;
    player.wait_until(
        "the constructor jumps over the shaft-top lip",
        80,
        |engine| {
            engine
                .object_snapshot(constructor)
                .is_some_and(|object| object.position.x <= 145)
        },
    )?;
    player.release(COM_LEFT)?;
    player.wait_until(
        "the constructor lands on the cabin plateau",
        160,
        |engine| {
            engine.object_snapshot(constructor).is_some_and(|object| {
                object.action.name == "Walk" && object.position.x < 155 && object.position.y <= 115
            })
        },
    )?;
    player.tap(COM_CURSOR_RIGHT)?;
    player.assert_milestone(
        "CursorRight selects the valley CLNK for the top lip",
        |engine| engine.crew_cursor(owner) == Some(valley),
    )?;
    player.hold_until(
        COM_RIGHT,
        "the valley CLNK takes a run-up on ELEC",
        40,
        |engine| {
            engine
                .object_snapshot(valley)
                .is_some_and(|object| object.position.x >= 169 && object.action.name == "Walk")
        },
    )?;
    player.press(COM_LEFT)?;
    player.tap(COM_UP)?;
    player.wait_until(
        "the valley CLNK jumps over the shaft-top lip",
        80,
        |engine| {
            engine
                .object_snapshot(valley)
                .is_some_and(|object| object.position.x <= 145)
        },
    )?;
    player.release(COM_LEFT)?;
    player.wait_until(
        "the valley CLNK lands on the cabin plateau",
        160,
        |engine| {
            engine.object_snapshot(valley).is_some_and(|object| {
                object.action.name == "Walk" && object.position.x < 155 && object.position.y <= 115
            })
        },
    )?;
    player.double_tap(COM_CURSOR_TOGGLE)?;
    player.assert_milestone(
        "all Tutorial05 CLNKs are reselected on the plateau",
        |engine| {
            engine.crew_cursor(owner) == Some(constructor)
                && [constructor, valley, catapult_clonk]
                    .into_iter()
                    .all(|clonk| {
                        engine
                            .object_snapshot(clonk)
                            .is_some_and(|object| object.selected)
                    })
        },
    )?;

    let hut_position = player
        .engine()
        .object_snapshot(hut)
        .expect("Tutorial05 HUT3 survives")
        .position;
    player.hold_until(
        COM_LEFT,
        "the selected crew follows the constructor to HUT3",
        360,
        |engine| {
            engine
                .object_snapshot(constructor)
                .is_some_and(|object| object.position.x <= hut_position.x + 19)
        },
    )?;
    player.hold_until(
        COM_DOWN,
        "the constructor descends from the HUT3 wall",
        80,
        |engine| {
            engine
                .object_snapshot(constructor)
                .is_some_and(|object| object.action.name == "Walk")
        },
    )?;
    player.hold_until(
        COM_RIGHT,
        "the constructor walks into the HUT3 entrance",
        80,
        |engine| {
            engine.object_snapshot(constructor).is_some_and(|object| {
                object.action.name == "Walk"
                    && (hut_position.x + 2..=hut_position.x + 19).contains(&object.position.x)
            })
        },
    )?;
    player.tap(COM_UP)?;
    player.wait_until("all three selected CLNKs enter HUT3", 360, |engine| {
        [constructor, valley, catapult_clonk]
            .into_iter()
            .all(|clonk| {
                engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.container == Some(hut))
            })
    })?;
    player.wait_until(
        "Tutorial05 fulfills SCRG and reaches GameOver",
        600,
        Engine::is_game_over,
    )?;
    player.assert_milestone("Tutorial05 records its fulfilled SCRG goal", |engine| {
        engine
            .snapshot()
            .round_results
            .fulfilled_goals
            .iter()
            .any(|goal| goal == "SCRG")
    })?;

    Ok(())
}
