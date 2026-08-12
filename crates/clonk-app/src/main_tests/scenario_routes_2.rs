// Spliced into `mod tests` (src/main_tests.rs) via include!: a bare item
// sequence, not a child module, so test ids stay `tests::<fn>`.

#[test]
fn app_virtual_keyboard_flings_tutorial05_wood_to_the_right_hill() {
    // Tutorial05's first material relay selects the valley CLNK, collects
    // its real WOOD, loads the real valley CATA, tensions it and fires to
    // Script63's right-hill rectangle
    // (content/Tutorial.c4f/Tutorial05.c4s/Script.c:32-44,67-157).
    // Keep the complete route behind GameApp::handle_key: C++ cycles
    // crew on CursorRight, forwards pushed-object controls, and turns
    // held Jump'n'Run Down into CATA's eight-frame AimUpdate timer
    // (src/C4Player.cpp:1261-1275,1453-1473,1490-1553;
    // src/C4Object.cpp:3321-3337,3520-3567;
    // content/Objects.c4d/Vehicles.c4d/Catapult.c4d/Script.c:39-77,121-147;
    // planet/System.c4g/JumpAndRun.c:53-119).
    let mut app = real_tutorial_app(5, "Tutorial 5 app virtual player");
    let constructor = app.engine.test_crew_cursor(app.local_owner);
    let elevator = app_object_with_definition(&app, "ELEV").test_value();
    let valley_cata = app_object_with_definition_near_x(&app, "CATA", 240).test_value();
    let hill_cata = app_object_with_definition_near_x(&app, "CATA", 540).test_value();
    let wood = app_object_with_definition_near_x(&app, "WOOD", 280).test_value();
    let metal = app_object_with_definition_near_x(&app, "METL", 285).test_value();
    let home_base = app_object_with_definition(&app, "HUT3").test_value();
    assert_ne!(hill_cata, valley_cata);

    advance_app_until(
        &mut app,
        "Tutorial05 teaches selection after ELEV stalls",
        800,
        |app| {
            app_tutorial_message_contains(app, "'select right'")
                && app
                    .engine
                    .object_snapshot(elevator)
                    .is_some_and(|object| object.construction == 80_000)
        },
    );
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyE);
    let valley = app.engine.test_crew_cursor(app.local_owner);
    assert_ne!(valley, constructor);
    assert!(
        app.engine.object_snapshot(valley).is_some_and(|object| {
            (200..300).contains(&object.position.x) && object.position.y >= 350
        }),
        "physical CursorRight must select Tutorial05's real valley CLNK"
    );

    advance_app_until(
        &mut app,
        "Tutorial05 asks the valley CLNK to collect material",
        240,
        |app| app_tutorial_message_contains(app, "collect either the wood or the metal"),
    );
    let valley_x = app.engine.test_object_snapshot(valley).position.x;
    let wood_x = app.engine.test_object_snapshot(wood).position.x;
    let toward_wood = if wood_x < valley_x {
        VirtualKeyCode::KeyZ
    } else {
        VirtualKeyCode::KeyC
    };
    hold_app_key_until(
        &mut app,
        toward_wood,
        "the valley CLNK naturally collects the first material",
        160,
        |app| app_clonk_carries(app, valley, "WOOD") || app_clonk_carries(app, valley, "METL"),
    );
    if app_clonk_carries(&app, valley, "METL") {
        AppVirtualKeyboard::new(&mut app).press(VirtualKeyCode::KeyZ);
        advance_app_until(&mut app, "valley CLNK faces left with METL", 30, |app| {
            app.engine
                .object_snapshot(valley)
                .is_some_and(|object| object.direction == Direction::Left)
        });
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard.release(VirtualKeyCode::KeyZ);
            keyboard.tap(VirtualKeyCode::KeyA);
        }
        advance_app_until(&mut app, "METL leaves valley CLNK inventory", 30, |app| {
            app.engine
                .object_snapshot(metal)
                .is_some_and(|object| object.container.is_none())
        });
        let valley_x = app.engine.test_object_snapshot(valley).position.x;
        let wood_x = app.engine.test_object_snapshot(wood).position.x;
        let toward_wood = if wood_x < valley_x {
            VirtualKeyCode::KeyZ
        } else {
            VirtualKeyCode::KeyC
        };
        hold_app_key_until(
            &mut app,
            toward_wood,
            "the valley CLNK naturally collects WOOD",
            120,
            |app| app_clonk_carries(app, valley, "WOOD"),
        );
    }
    assert_eq!(
        app.engine
            .object_snapshot(wood)
            .expect("collected valley WOOD survives")
            .container,
        Some(valley)
    );
    advance_app_until(
        &mut app,
        "Tutorial05 points to the valley CATA",
        240,
        |app| app_tutorial_message_contains(app, "stand in front of the catapult"),
    );
    hold_app_key_until(
        &mut app,
        VirtualKeyCode::KeyZ,
        "the WOOD-carrying valley CLNK reaches CATA",
        160,
        |app| {
            app.engine
                .object_snapshot(valley)
                .zip(app.engine.object_snapshot(valley_cata))
                .is_some_and(|(clonk, cata)| {
                    clonk.action.name == "Walk" && (clonk.position.x - cata.position.x).abs() <= 8
                })
        },
    );
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyX);
    advance_app_until(&mut app, "the valley CLNK grabs the real CATA", 80, |app| {
        app.engine.object_snapshot(valley).is_some_and(|object| {
            object.action.name == "Push" && object.action.target == Some(valley_cata)
        })
    });

    advance_app_until(
        &mut app,
        "Tutorial05 asks the valley CLNK to load CATA",
        300,
        |app| app_tutorial_message_contains(app, "Press 'throw' to load the catapult"),
    );
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyA);
    advance_app_until(&mut app, "WOOD enters the real valley CATA", 80, |app| {
        app.engine
            .object_snapshot(wood)
            .is_some_and(|object| object.container == Some(valley_cata))
    });
    advance_app_until(
        &mut app,
        "Tutorial05 asks for full CATA tension",
        300,
        |app| app_tutorial_message_contains(app, "fully tensioned"),
    );

    AppVirtualKeyboard::new(&mut app).press(VirtualKeyCode::KeyX);
    AppVirtualKeyboard::new(&mut app).press(VirtualKeyCode::KeyX);
    assert!(
        app.engine.object_snapshot(valley).is_some_and(|object| {
            object.action.name == "Push" && object.action.target == Some(valley_cata)
        }),
        "repeated Down must not synthesize Down_D and ungrab the real CATA"
    );
    advance_app_until(&mut app, "valley CATA reaches Ready phase six", 80, |app| {
        app.engine
            .object_snapshot(valley_cata)
            .is_some_and(|object| object.action.name == "Ready" && object.action.phase == 6)
    });
    AppVirtualKeyboard::new(&mut app).release(VirtualKeyCode::KeyX);
    let tensioned = app.engine.test_object_snapshot(valley_cata);
    assert_eq!(
        (tensioned.action.name.as_str(), tensioned.action.phase),
        ("Ready", 6)
    );
    assert!(
        tensioned
            .effects
            .iter()
            .all(|effect| effect.name != "IntJnRAim" || effect.priority == 0),
        "physical X release must cancel CATA's aim timer without losing phase six"
    );
    assert!(tensioned
        .effects
        .iter()
        .any(|effect| effect.name == "IntJnRAim" && effect.priority == 0));

    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyA);
    assert_eq!(
        app.engine
            .object_snapshot(valley_cata)
            .expect("valley CATA survives firing")
            .action
            .name,
        "Fire"
    );
    advance_app_until(
        &mut app,
        "real valley CATA flings WOOD to the right hill",
        400,
        |app| {
            app.engine.object_snapshot(wood).is_some_and(|object| {
                object.container.is_none()
                    && (460..640).contains(&object.position.x)
                    && (150..290).contains(&object.position.y)
            })
        },
    );
    assert!(app
        .engine
        .object_snapshot(valley_cata)
        .expect("valley CATA survives the projectile flight")
        .effects
        .iter()
        .all(|effect| effect.name != "IntJnRAim"));
    advance_app_until(
        &mut app,
        "Tutorial05 advances to the right-hill CLNK",
        300,
        |app| app_tutorial_message_contains(app, "switch to the clonk on the right hill"),
    );

    // Script64 waits for the physical CursorRight selection, then
    // Script65 waits until that exact flung object enters catapult_clnk
    // before targeting hill_cata
    // (content/Tutorial.c4f/Tutorial05.c4s/Script.c:160-175).
    // C++ collection is driven by the carried object crossing the crew
    // member's collection rectangle, independent of its former flight
    // state (src/C4GameObjects.cpp:155-196). Keep both operations behind
    // the app's E/Z/C keyboard mapping.
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyE);
    let catapult_clonk = app.engine.test_crew_cursor(app.local_owner);
    assert_ne!(catapult_clonk, constructor);
    assert_ne!(catapult_clonk, valley);
    assert!(
        app.engine
            .object_snapshot(catapult_clonk)
            .is_some_and(|object| object.position.x >= 450 && object.position.y < 350),
        "second physical CursorRight must select Tutorial05's right-hill CLNK"
    );
    advance_app_until(
        &mut app,
        "Tutorial05 asks the right-hill CLNK to collect the flung material",
        300,
        |app| app_tutorial_message_contains(app, "Collect the material you just flung up"),
    );

    // C++ collection is independent of Mobile. With the rotation walk's
    // fixed-position remainder preserved, WOOD continues through the
    // collection corridor instead of first stopping inside it.
    advance_app_until(
        &mut app,
        "flung valley WOOD descends into the right-hill collection corridor",
        120,
        |app| {
            app.engine.object_snapshot(wood).is_some_and(|object| {
                object.container.is_none()
                    && (460..640).contains(&object.position.x)
                    && object.position.y >= 215
            })
        },
    );
    let wood_x = app.engine.test_object_snapshot(wood).position.x;
    let catapult_clonk_x = app.engine.test_object_snapshot(catapult_clonk).position.x;
    let collect_key = if wood_x < catapult_clonk_x {
        VirtualKeyCode::KeyZ
    } else {
        VirtualKeyCode::KeyC
    };
    hold_app_key_until(
        &mut app,
        collect_key,
        "the right-hill CLNK naturally collects the exact flung WOOD",
        200,
        |app| {
            app.engine
                .object_snapshot(wood)
                .is_some_and(|object| object.container == Some(catapult_clonk))
        },
    );
    assert_eq!(
        app.engine
            .object_snapshot(wood)
            .expect("collected flung WOOD survives")
            .container,
        Some(catapult_clonk),
        "physical Z/C movement must collect the original valley WOOD"
    );

    advance_app_until(
        &mut app,
        "Tutorial05 Script65 points at the right-hill CATA",
        300,
        |app| app_tutorial_message_contains(app, "grab the other catapult"),
    );
    assert_eq!(
        app.snapshot
            .script_globals
            .named
            .get("arrow_target")
            .and_then(|value| serde_json::to_value(value).ok()),
        Some(serde_json::json!({ "Object": hill_cata.as_u64() })),
        "SetArrowToObj must identify Tutorial05's real right-hill CATA"
    );

    // Script66-83 requires that same right-hill CLNK to grab hill_cata,
    // face it toward the cabin, load the exact collected object, and
    // fling it into FindObject's [0,220)x[0,140) cabin-hill rectangle
    // before it asks for the constructor selection
    // (content/Tutorial.c4f/Tutorial05.c4s/Script.c:178-228).
    // Keep turning, loading, held-X tension and firing on the physical
    // app route; CATA turns while pushed, admits the pushed crew's
    // carried object, advances Ready through AimUpdate, and ejects that
    // contained object from Fire
    // (content/Objects.c4d/Vehicles.c4d/Catapult.c4d/Script.c:39-77,121-163).
    let hill_cata_x = app.engine.test_object_snapshot(hill_cata).position.x;
    let catapult_clonk_x = app.engine.test_object_snapshot(catapult_clonk).position.x;
    let reach_hill_cata = if hill_cata_x < catapult_clonk_x {
        VirtualKeyCode::KeyZ
    } else {
        VirtualKeyCode::KeyC
    };
    hold_app_key_until(
        &mut app,
        reach_hill_cata,
        "the WOOD-carrying right-hill CLNK reaches CATA",
        180,
        |app| {
            app.engine
                .object_snapshot(catapult_clonk)
                .zip(app.engine.object_snapshot(hill_cata))
                .is_some_and(|(clonk, cata)| {
                    clonk.action.name == "Walk" && (clonk.position.x - cata.position.x).abs() <= 12
                })
        },
    );
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyX);
    advance_app_until(
        &mut app,
        "the right-hill CLNK grabs its real CATA",
        80,
        |app| {
            app.engine
                .object_snapshot(catapult_clonk)
                .is_some_and(|object| {
                    object.action.name == "Push" && object.action.target == Some(hill_cata)
                })
        },
    );
    advance_app_until(
        &mut app,
        "Tutorial05 evaluates the right-hill CATA direction",
        300,
        |app| {
            app_tutorial_message_contains(app, "Turn the catapult around")
                || app_tutorial_message_contains(app, "load the catapult")
        },
    );
    if app
        .engine
        .object_snapshot(hill_cata)
        .is_some_and(|object| object.direction != Direction::Left)
    {
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::KeyZ,
            "pushing left turns the right-hill CATA toward the cabin",
            40,
            |app| {
                app.engine
                    .object_snapshot(hill_cata)
                    .is_some_and(|object| object.direction == Direction::Left)
            },
        );
    }
    assert_eq!(
        app.engine
            .object_snapshot(hill_cata)
            .expect("right-hill CATA survives turning")
            .direction,
        Direction::Left,
        "the pushed right-hill CATA must face the cabin before loading"
    );
    advance_app_until(
        &mut app,
        "Tutorial05 asks the right-hill CLNK to load CATA",
        300,
        |app| app_tutorial_message_contains(app, "load the catapult"),
    );
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyA);
    advance_app_until(
        &mut app,
        "the exact valley WOOD enters the right-hill CATA",
        80,
        |app| {
            app.engine
                .object_snapshot(wood)
                .is_some_and(|object| object.container == Some(hill_cata))
        },
    );
    assert_eq!(
        app_object_contents_count(&app, hill_cata, "WOOD"),
        1,
        "the right-hill CATA must contain the original valley WOOD exactly once"
    );
    advance_app_until(
        &mut app,
        "Tutorial05 asks for the shot toward the cabin",
        300,
        |app| app_tutorial_message_contains(app, "Fling the material"),
    );

    AppVirtualKeyboard::new(&mut app).press(VirtualKeyCode::KeyX);
    advance_app_until(
        &mut app,
        "right-hill CATA reaches Ready phase six",
        80,
        |app| {
            app.engine
                .object_snapshot(hill_cata)
                .is_some_and(|object| object.action.name == "Ready" && object.action.phase == 6)
        },
    );
    AppVirtualKeyboard::new(&mut app).release(VirtualKeyCode::KeyX);
    let tensioned = app.engine.test_object_snapshot(hill_cata);
    assert_eq!(
        (tensioned.action.name.as_str(), tensioned.action.phase),
        ("Ready", 6)
    );
    assert!(
        tensioned
            .effects
            .iter()
            .all(|effect| effect.name != "IntJnRAim" || effect.priority == 0),
        "physical X release must cancel the right-hill CATA aim timer"
    );
    assert!(tensioned
        .effects
        .iter()
        .any(|effect| effect.name == "IntJnRAim" && effect.priority == 0));

    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyA);
    assert_eq!(
        app.engine
            .object_snapshot(hill_cata)
            .expect("right-hill CATA survives firing")
            .action
            .name,
        "Fire"
    );
    advance_app_until(
        &mut app,
        "the right-hill CATA flings the same WOOD to the cabin hill",
        400,
        |app| {
            app.engine.object_snapshot(wood).is_some_and(|object| {
                object.container.is_none()
                    && (0..220).contains(&object.position.x)
                    && (0..140).contains(&object.position.y)
            })
        },
    );
    assert!(app
        .engine
        .object_snapshot(hill_cata)
        .expect("right-hill CATA survives the projectile flight")
        .effects
        .iter()
        .all(|effect| effect.name != "IntJnRAim"));
    let delivered = app.engine.test_object_snapshot(wood);
    assert!(delivered.container.is_none());
    assert!((0..220).contains(&delivered.position.x));
    assert!((0..140).contains(&delivered.position.y));

    advance_app_until(
        &mut app,
        "Tutorial05 Script83 asks for the constructor CLNK",
        300,
        |app| app_tutorial_message_contains(app, "switch back to the clonk near the cabin"),
    );
    assert_eq!(
        app.snapshot
            .script_globals
            .named
            .get("arrow_target")
            .and_then(|value| serde_json::to_value(value).ok()),
        Some(serde_json::json!({ "Object": constructor.as_u64() })),
        "Script83 must target Tutorial05's constructor CLNK"
    );

    // Script84-85 cycles to constructor_clnk, waits for that same WOOD to
    // enter its collection, then points at the real ELEV construction site
    // (content/Tutorial.c4f/Tutorial05.c4s/Script.c:230-245). C++ wraps
    // CursorRight through the ordered crew list and collects carryables
    // only when they cross the selected crew's collection rectangle
    // (src/C4Player.cpp:1261-1275; src/C4GameObjects.cpp:140-196).
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyE);
    assert_eq!(
        app.engine.crew_cursor(app.local_owner),
        Some(constructor),
        "third physical CursorRight must wrap to Tutorial05's constructor CLNK"
    );
    advance_app_until(
        &mut app,
        "Tutorial05 asks the constructor CLNK to collect the delivered material",
        300,
        |app| app_tutorial_message_contains(app, "Collect the material you just flung up"),
    );
    advance_app_until(
        &mut app,
        "twice-flung WOOD descends into the cabin-hill collection corridor",
        120,
        |app| {
            app.engine.object_snapshot(wood).is_some_and(|object| {
                object.container.is_some()
                    || ((0..220).contains(&object.position.x) && object.position.y >= 75)
            })
        },
    );
    if app
        .engine
        .object_snapshot(wood)
        .is_some_and(|object| object.container != Some(constructor))
    {
        let constructor_x = app.engine.test_object_snapshot(constructor).position.x;
        let delivered_x = app.engine.test_object_snapshot(wood).position.x;
        let collect_key = if delivered_x < constructor_x {
            VirtualKeyCode::KeyZ
        } else {
            VirtualKeyCode::KeyC
        };
        hold_app_key_until(
            &mut app,
            collect_key,
            "constructor CLNK naturally collects the exact twice-flung WOOD",
            240,
            |app| {
                app.engine
                    .object_snapshot(wood)
                    .is_some_and(|object| object.container == Some(constructor))
            },
        );
    }
    assert_eq!(
        app.engine
            .object_snapshot(wood)
            .expect("constructor-collected WOOD survives")
            .container,
        Some(constructor),
        "physical Z/C movement must collect the original twice-flung WOOD"
    );
    advance_app_until(
        &mut app,
        "Tutorial05 Script85 points at the elevator construction site",
        300,
        |app| app_tutorial_message_contains(app, "continue work on the elevator"),
    );
    assert_eq!(
        app.snapshot
            .script_globals
            .named
            .get("arrow_target")
            .and_then(|value| serde_json::to_value(value).ok()),
        Some(serde_json::json!({ "Object": elevator.as_u64() })),
        "Script85 must target Tutorial05's real ELEV construction site"
    );

    // The tutorial's "down" is a double physical X. C++ turns the second
    // press inside C4DoubleClick into COM_Down_D, finds OCF_Construct at
    // the crew position, and queues Build on that exact object
    // (src/C4Player.cpp:1522-1536; src/C4ObjectCom.cpp:573-589).
    let elevator_x = app.engine.test_object_snapshot(elevator).position.x;
    let constructor_x = app.engine.test_object_snapshot(constructor).position.x;
    if (constructor_x - elevator_x).abs() > 4 {
        let approach_key = if elevator_x < constructor_x {
            VirtualKeyCode::KeyZ
        } else {
            VirtualKeyCode::KeyC
        };
        hold_app_key_until(
            &mut app,
            approach_key,
            "WOOD-carrying constructor reaches the elevator construction site",
            240,
            |app| {
                app.engine
                    .object_snapshot(constructor)
                    .zip(app.engine.object_snapshot(elevator))
                    .is_some_and(|(clonk, elevator)| {
                        (clonk.position.x - elevator.position.x).abs() <= 4
                    })
            },
        );
    }
    {
        let mut keyboard = AppVirtualKeyboard::new(&mut app);
        keyboard.tap(VirtualKeyCode::KeyX);
        keyboard.tap(VirtualKeyCode::KeyX);
    }
    // ELEV needs WOOD=4 and METL=2. C++ consumes the carried missing
    // component before checking every component's ratio at the current
    // construction percentage, so this first WOOD raises the ledger to
    // 4/1 but METL=1/2 keeps the 80% site stalled for Script110's second
    // relay (Elevator.c4d/DefCore.txt:16; src/C4Object.cpp:1682-1744;
    // content/Tutorial.c4f/Tutorial05.c4s/Script.c:248-253).
    advance_app_until(
        &mut app,
        "constructor contributes the exact twice-flung WOOD to ELEV",
        120,
        |app| app.engine.object_snapshot(wood).is_none(),
    );
    let first_delivery = app.engine.test_object_snapshot(elevator);
    assert_eq!(first_delivery.components.get("WOOD"), Some(&4));
    assert_eq!(first_delivery.components.get("METL"), Some(&1));
    assert_eq!(
        first_delivery.construction, 80_000,
        "ELEV needs its second METL before construction can pass 80%"
    );
    // C++ Build immediately queues Acquire(METL) and posts its
    // needed-material target message (C4Object.cpp:1682-1744). With the
    // shipped CATA GrabGet bit restored, that autonomous builder can now
    // pursue the relay cargo exactly like C++; this app-level physical
    // route has already proved both catapult shots and the first ELEV
    // delivery, so stop before competing with that command. The engine
    // Tutorial05 route separately covers completion and the carriage.
    advance_app_until(
        &mut app,
        "Tutorial05 queues C++ Acquire(METL) after its first elevator delivery",
        30,
        |app| {
            app.engine
                .object_snapshot(constructor)
                .is_some_and(|object| {
                    object.command_stack.command_views().iter().any(|command| {
                        command.name == "Acquire"
                            && command.data == CommandData::Text("METL".into())
                    })
                })
        },
    );
    if app
        .engine
        .object_snapshot(constructor)
        .is_some_and(|object| {
            object.command_stack.command_views().iter().any(|command| {
                command.name == "Acquire" && command.data == CommandData::Text("METL".into())
            })
        })
    {
        return;
    }

    // Script110 leaves the relay to the player and Script150 waits for the
    // ELEC created only by the completed ELEV
    // (content/Tutorial.c4f/Tutorial05.c4s/Script.c:248-258;
    // content/Objects.c4d/Structures.c4d/Elevator.c4d/Script.c:8-15).
    // Cycle to the real valley CLNK, ungrab its first-shot CATA with the
    // C++ DownDouble path, and collect the untouched METL through ordinary
    // movement/collection (src/C4Player.cpp:1261-1275,1522-1536;
    // src/C4Object.cpp:3520-3567; src/C4GameObjects.cpp:140-196).
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyE);
    assert_eq!(app.engine.crew_cursor(app.local_owner), Some(valley));
    if app
        .engine
        .object_snapshot(valley)
        .is_some_and(|object| object.action.name == "Push")
    {
        let mut keyboard = AppVirtualKeyboard::new(&mut app);
        keyboard.tap(VirtualKeyCode::KeyX);
        keyboard.tap(VirtualKeyCode::KeyX);
    }
    advance_app_until(&mut app, "valley CLNK releases its CATA", 80, |app| {
        app.engine
            .object_snapshot(valley)
            .is_some_and(|object| object.action.name == "Walk")
    });
    let valley_x = app.engine.test_object_snapshot(valley).position.x;
    let metal_x = app.engine.test_object_snapshot(metal).position.x;
    let collect_metal_key = if metal_x < valley_x {
        VirtualKeyCode::KeyZ
    } else {
        VirtualKeyCode::KeyC
    };
    hold_app_key_until(
        &mut app,
        collect_metal_key,
        "valley CLNK naturally collects the original METL",
        240,
        |app| {
            app.engine
                .object_snapshot(metal)
                .is_some_and(|object| object.container == Some(valley))
        },
    );
    assert_eq!(
        app.engine
            .object_snapshot(metal)
            .expect("valley-collected METL survives")
            .container,
        Some(valley)
    );

    let valley_x = app.engine.test_object_snapshot(valley).position.x;
    let valley_cata_x = app.engine.test_object_snapshot(valley_cata).position.x;
    let return_to_valley_cata = if valley_cata_x < valley_x {
        VirtualKeyCode::KeyZ
    } else {
        VirtualKeyCode::KeyC
    };
    hold_app_key_until(
        &mut app,
        return_to_valley_cata,
        "METL-carrying valley CLNK returns to CATA",
        240,
        |app| {
            app.engine
                .object_snapshot(valley)
                .zip(app.engine.object_snapshot(valley_cata))
                .is_some_and(|(clonk, cata)| {
                    clonk.action.name == "Walk" && (clonk.position.x - cata.position.x).abs() <= 8
                })
        },
    );
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyX);
    advance_app_until(&mut app, "valley CLNK regrabs its CATA", 80, |app| {
        app.engine.object_snapshot(valley).is_some_and(|object| {
            object.action.name == "Push" && object.action.target == Some(valley_cata)
        })
    });
    if app
        .engine
        .object_snapshot(valley_cata)
        .is_some_and(|object| object.direction != Direction::Right)
    {
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::KeyC,
            "valley CATA faces the right hill for its second shot",
            40,
            |app| {
                app.engine
                    .object_snapshot(valley_cata)
                    .is_some_and(|object| object.direction == Direction::Right)
            },
        );
    }
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyA);
    advance_app_until(&mut app, "original METL enters valley CATA", 80, |app| {
        app.engine
            .object_snapshot(metal)
            .is_some_and(|object| object.container == Some(valley_cata))
    });
    assert_eq!(app_object_contents_count(&app, valley_cata, "METL"), 1);
    AppVirtualKeyboard::new(&mut app).press(VirtualKeyCode::KeyX);
    advance_app_until(
        &mut app,
        "valley CATA restores full tension for METL",
        80,
        |app| {
            app.engine
                .object_snapshot(valley_cata)
                .is_some_and(|object| object.action.name == "Ready" && object.action.phase == 6)
        },
    );
    AppVirtualKeyboard::new(&mut app).release(VirtualKeyCode::KeyX);
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyA);
    advance_app_until(
        &mut app,
        "real valley CATA flings the original METL to the right hill",
        400,
        |app| {
            app.engine.object_snapshot(metal).is_some_and(|object| {
                object.container.is_none()
                    && (460..640).contains(&object.position.x)
                    && (150..290).contains(&object.position.y)
            })
        },
    );
    advance_app_until(
        &mut app,
        "second-relay METL settles on the right hill",
        300,
        |app| {
            app.engine.object_snapshot(metal).is_some_and(|object| {
                object.container.is_none()
                    && !object.mobile
                    && (460..640).contains(&object.position.x)
                    && (150..290).contains(&object.position.y)
            })
        },
    );

    // Mirror the physical relay through the already-used hill CATA. Its
    // script preserves iPhase across Fire -> Charge -> Ready, accepts the
    // pushed Clonk's carried object on Throw, and ejects that exact object
    // from Projectile (Catapult.c4d/Script.c:31-77,121-163;
    // src/C4Object.cpp:3520-3567).
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyE);
    assert_eq!(
        app.engine.crew_cursor(app.local_owner),
        Some(catapult_clonk)
    );
    if app
        .engine
        .object_snapshot(catapult_clonk)
        .is_some_and(|object| object.action.name == "Push")
    {
        let mut keyboard = AppVirtualKeyboard::new(&mut app);
        keyboard.tap(VirtualKeyCode::KeyX);
        keyboard.tap(VirtualKeyCode::KeyX);
    }
    advance_app_until(&mut app, "right-hill CLNK releases its CATA", 80, |app| {
        app.engine
            .object_snapshot(catapult_clonk)
            .is_some_and(|object| object.action.name == "Walk")
    });
    let hill_clonk_x = app.engine.test_object_snapshot(catapult_clonk).position.x;
    let landed_metal_x = app.engine.test_object_snapshot(metal).position.x;
    let collect_hill_metal = if landed_metal_x < hill_clonk_x {
        VirtualKeyCode::KeyZ
    } else {
        VirtualKeyCode::KeyC
    };
    hold_app_key_until(
        &mut app,
        collect_hill_metal,
        "right-hill CLNK naturally collects the original METL",
        240,
        |app| {
            app.engine
                .object_snapshot(metal)
                .is_some_and(|object| object.container == Some(catapult_clonk))
        },
    );
    assert_eq!(
        app.engine
            .object_snapshot(metal)
            .expect("right-hill-collected METL survives")
            .container,
        Some(catapult_clonk)
    );

    let hill_clonk_x = app.engine.test_object_snapshot(catapult_clonk).position.x;
    let hill_cata_x = app.engine.test_object_snapshot(hill_cata).position.x;
    let return_to_hill_cata = if hill_cata_x < hill_clonk_x {
        VirtualKeyCode::KeyZ
    } else {
        VirtualKeyCode::KeyC
    };
    hold_app_key_until(
        &mut app,
        return_to_hill_cata,
        "METL-carrying hill CLNK returns to CATA",
        240,
        |app| {
            app.engine
                .object_snapshot(catapult_clonk)
                .zip(app.engine.object_snapshot(hill_cata))
                .is_some_and(|(clonk, cata)| {
                    clonk.action.name == "Walk" && (clonk.position.x - cata.position.x).abs() <= 8
                })
        },
    );
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyX);
    advance_app_until(&mut app, "right-hill CLNK regrabs its CATA", 80, |app| {
        app.engine
            .object_snapshot(catapult_clonk)
            .is_some_and(|object| {
                object.action.name == "Push" && object.action.target == Some(hill_cata)
            })
    });
    if app
        .engine
        .object_snapshot(hill_cata)
        .is_some_and(|object| object.direction != Direction::Left)
    {
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::KeyZ,
            "hill CATA faces the cabin for its METL shot",
            40,
            |app| {
                app.engine
                    .object_snapshot(hill_cata)
                    .is_some_and(|object| object.direction == Direction::Left)
            },
        );
    }
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyA);
    advance_app_until(&mut app, "original METL enters hill CATA", 80, |app| {
        app.engine
            .object_snapshot(metal)
            .is_some_and(|object| object.container == Some(hill_cata))
    });
    assert_eq!(app_object_contents_count(&app, hill_cata, "METL"), 1);
    AppVirtualKeyboard::new(&mut app).press(VirtualKeyCode::KeyX);
    advance_app_until(
        &mut app,
        "hill CATA restores full tension for METL",
        80,
        |app| {
            app.engine
                .object_snapshot(hill_cata)
                .is_some_and(|object| object.action.name == "Ready" && object.action.phase == 6)
        },
    );
    AppVirtualKeyboard::new(&mut app).release(VirtualKeyCode::KeyX);
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyA);
    advance_app_until(
        &mut app,
        "real hill CATA flings the original METL to the cabin hill",
        400,
        |app| {
            app.engine.object_snapshot(metal).is_some_and(|object| {
                object.container.is_none()
                    && (0..220).contains(&object.position.x)
                    && (0..140).contains(&object.position.y)
            })
        },
    );

    // Wrap selection to constructor_clnk, collect the same METL, and use
    // the same physical DownDouble build path. C++ consumes METL=2/2 and
    // only then lets Build advance ELEV to FullCon, whose Initialize makes
    // ELEC (src/C4Object.cpp:1682-1762; src/C4ObjectCom.cpp:573-589;
    // Elevator.c4d/DefCore.txt:16; Elevator.c4d/Script.c:8-15).
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyE);
    assert_eq!(app.engine.crew_cursor(app.local_owner), Some(constructor));
    advance_app_until(
        &mut app,
        "second-relay METL settles on the cabin hill",
        300,
        |app| {
            app.engine
                .object_snapshot(metal)
                .is_some_and(|object| object.container.is_some() || !object.mobile)
        },
    );
    if app
        .engine
        .object_snapshot(metal)
        .is_some_and(|object| object.container != Some(constructor))
    {
        let constructor_x = app.engine.test_object_snapshot(constructor).position.x;
        let metal_x = app.engine.test_object_snapshot(metal).position.x;
        let collect_cabin_metal = if metal_x < constructor_x {
            VirtualKeyCode::KeyZ
        } else {
            VirtualKeyCode::KeyC
        };
        hold_app_key_until(
            &mut app,
            collect_cabin_metal,
            "constructor naturally collects the original METL",
            240,
            |app| {
                app.engine
                    .object_snapshot(metal)
                    .is_some_and(|object| object.container == Some(constructor))
            },
        );
    }
    assert_eq!(
        app.engine
            .object_snapshot(metal)
            .expect("constructor-collected METL survives")
            .container,
        Some(constructor)
    );
    let constructor_x = app.engine.test_object_snapshot(constructor).position.x;
    let elevator_x = app.engine.test_object_snapshot(elevator).position.x;
    if (constructor_x - elevator_x).abs() > 4 {
        let return_to_elevator = if elevator_x < constructor_x {
            VirtualKeyCode::KeyZ
        } else {
            VirtualKeyCode::KeyC
        };
        hold_app_key_until(
            &mut app,
            return_to_elevator,
            "METL-carrying constructor returns to ELEV",
            240,
            |app| {
                app.engine
                    .object_snapshot(constructor)
                    .zip(app.engine.object_snapshot(elevator))
                    .is_some_and(|(clonk, elevator)| {
                        (clonk.position.x - elevator.position.x).abs() <= 4
                    })
            },
        );
    }
    {
        let mut keyboard = AppVirtualKeyboard::new(&mut app);
        keyboard.tap(VirtualKeyCode::KeyX);
        keyboard.tap(VirtualKeyCode::KeyX);
    }
    advance_app_until(
        &mut app,
        "constructor contributes the exact METL to ELEV",
        120,
        |app| app.engine.object_snapshot(metal).is_none(),
    );
    advance_app_until(
        &mut app,
        "Tutorial05 creates its completed elevator carriage",
        600,
        |app| app_object_with_definition(app, "ELEC").is_some(),
    );
    let completed = app.engine.test_object_snapshot(elevator);
    assert_eq!(completed.components.get("WOOD"), Some(&4));
    assert_eq!(completed.components.get("METL"), Some(&2));
    assert_eq!(completed.construction, 100_000);
    let carriage = app_object_with_definition(&app, "ELEC").test_value();
    assert_eq!(completed.action.target, Some(carriage));
    advance_app_until(
        &mut app,
        "Tutorial05 Script160 asks for the completed elevator carriage",
        400,
        |app| app_tutorial_message_contains(app, "Grab the elevator case"),
    );

    // Script160-162 requires the constructor to grab that exact ELEC and
    // hold Jump'n'Run Dig until its ControlDig starts Drill; only then are
    // both tutorial confinement effects removed
    // (content/Tutorial.c4f/Tutorial05.c4s/Script.c:265-286;
    // Elevator.c4d/Case.c4d/Script.c:346-360,612-631;
    // src/C4Object.cpp:3520-3567).
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyX);
    advance_app_until(
        &mut app,
        "constructor grabs the completed elevator carriage",
        80,
        |app| {
            app.engine
                .object_snapshot(constructor)
                .is_some_and(|object| {
                    object.action.name == "Push" && object.action.target == Some(carriage)
                })
        },
    );
    advance_app_until(
        &mut app,
        "Tutorial05 Script161 asks for shaft drilling",
        300,
        |app| app_tutorial_message_contains(app, "Hold the 'dig' key"),
    );
    AppVirtualKeyboard::new(&mut app).press(VirtualKeyCode::KeyD);
    advance_app_until(
        &mut app,
        "real ELEC enters Drill through ControlDig",
        80,
        |app| {
            app.engine
                .object_snapshot(carriage)
                .is_some_and(|object| object.action.name == "Drill")
        },
    );
    advance_app_until(
        &mut app,
        "Tutorial05 Script162 releases its confinement effects",
        300,
        |app| {
            app.engine
                .object_snapshot(constructor)
                .zip(app.engine.object_snapshot(catapult_clonk))
                .is_some_and(|(constructor, catapult_clonk)| {
                    constructor
                        .effects
                        .iter()
                        .all(|effect| effect.name != "StayNearElev")
                        && catapult_clonk
                            .effects
                            .iter()
                            .all(|effect| effect.name != "StayNearCata")
                })
        },
    );
    advance_app_until(
        &mut app,
        "held physical D drills ELEC to the valley floor",
        1_200,
        |app| {
            app.engine
                .object_snapshot(carriage)
                .is_some_and(|object| object.position.y >= 350)
        },
    );
    AppVirtualKeyboard::new(&mut app).release(VirtualKeyCode::KeyD);
    advance_app_until(
        &mut app,
        "Tutorial05 Script200 asks to gather all Clonks",
        300,
        |app| app_tutorial_message_contains(app, "gather all clonks"),
    );
    // Script200 forces every CLNK back to Walk. Follow the engine's frozen
    // physical route across the drilled shaft lip before SelectAll; doing
    // this afterwards makes Follow commands contend for the narrow ELEC
    // platform (Tutorial05/Script.c:288-313; C4Player.cpp:1261-1293,
    // 1453-1553; C4Object.cpp:3406-3556).
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyE);
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyE);
    assert_eq!(
        app.engine.crew_cursor(app.local_owner),
        Some(catapult_clonk)
    );
    hold_app_key_until(
        &mut app,
        VirtualKeyCode::KeyZ,
        "right-hill CLNK descends into the valley",
        500,
        |app| {
            app.engine
                .object_snapshot(catapult_clonk)
                .is_some_and(|object| object.position.y >= 350 && object.action.name == "Walk")
        },
    );
    advance_app_until(
        &mut app,
        "all three exact Clonks stand at the valley bottom",
        600,
        |app| {
            [constructor, valley, catapult_clonk]
                .into_iter()
                .all(|clonk| {
                    app.engine.object_snapshot(clonk).is_some_and(|object| {
                        object.position.y >= 350 && object.action.name == "Walk"
                    })
                })
        },
    );
    advance_app_until(
        &mut app,
        "Tutorial05 Script210 asks for double-toggle selection",
        300,
        |app| app_tutorial_message_contains(app, "toggle selection"),
    );

    hold_app_key_until(
        &mut app,
        VirtualKeyCode::KeyZ,
        "right-hill CLNK reaches the drilled shaft lip",
        180,
        |app| {
            app.engine
                .object_snapshot(catapult_clonk)
                .is_some_and(|object| object.position.x <= 220 && object.action.name == "Walk")
        },
    );
    {
        let mut keyboard = AppVirtualKeyboard::new(&mut app);
        keyboard.press(VirtualKeyCode::KeyZ);
        keyboard.tap(VirtualKeyCode::KeyS);
    }
    advance_app_until(
        &mut app,
        "right-hill CLNK jumps across the shaft lip",
        80,
        |app| {
            app.engine
                .object_snapshot(catapult_clonk)
                .is_some_and(|object| object.position.x <= 174)
        },
    );
    AppVirtualKeyboard::new(&mut app).release(VirtualKeyCode::KeyZ);
    advance_app_until(&mut app, "right-hill CLNK reaches ELEC", 160, |app| {
        app.engine
            .object_snapshot(catapult_clonk)
            .zip(app.engine.object_snapshot(carriage))
            .is_some_and(|(clonk, carriage)| {
                (clonk.position.x - carriage.position.x).abs() <= 18
                    && (clonk.position.y - carriage.position.y).abs() <= 22
            })
    });
    if app
        .engine
        .object_snapshot(catapult_clonk)
        .is_some_and(|object| object.action.name != "Walk")
    {
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::KeyX,
            "right-hill CLNK scales down onto ELEC",
            160,
            |app| {
                app.engine
                    .object_snapshot(catapult_clonk)
                    .zip(app.engine.object_snapshot(carriage))
                    .is_some_and(|(clonk, carriage)| {
                        clonk.action.name == "Walk"
                            && (clonk.position.x - carriage.position.x).abs() <= 18
                            && (clonk.position.y - carriage.position.y).abs() <= 22
                    })
            },
        );
    }

    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyE);
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyE);
    assert_eq!(app.engine.crew_cursor(app.local_owner), Some(valley));
    hold_app_key_until(
        &mut app,
        VirtualKeyCode::KeyZ,
        "valley CLNK reaches the drilled shaft lip",
        240,
        |app| {
            app.engine
                .object_snapshot(valley)
                .is_some_and(|object| object.position.x <= 220 && object.action.name == "Walk")
        },
    );
    {
        let mut keyboard = AppVirtualKeyboard::new(&mut app);
        keyboard.press(VirtualKeyCode::KeyZ);
        keyboard.tap(VirtualKeyCode::KeyS);
    }
    advance_app_until(
        &mut app,
        "valley CLNK jumps across the shaft lip",
        80,
        |app| {
            app.engine
                .object_snapshot(valley)
                .is_some_and(|object| object.position.x <= 174)
        },
    );
    AppVirtualKeyboard::new(&mut app).release(VirtualKeyCode::KeyZ);
    advance_app_until(&mut app, "valley CLNK reaches ELEC", 160, |app| {
        app.engine
            .object_snapshot(valley)
            .zip(app.engine.object_snapshot(carriage))
            .is_some_and(|(clonk, carriage)| {
                (clonk.position.x - carriage.position.x).abs() <= 18
                    && (clonk.position.y - carriage.position.y).abs() <= 22
            })
    });
    if app
        .engine
        .object_snapshot(valley)
        .is_some_and(|object| object.action.name != "Walk")
    {
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::KeyX,
            "valley CLNK scales down onto ELEC",
            160,
            |app| {
                app.engine
                    .object_snapshot(valley)
                    .zip(app.engine.object_snapshot(carriage))
                    .is_some_and(|(clonk, carriage)| {
                        clonk.action.name == "Walk"
                            && (clonk.position.x - carriage.position.x).abs() <= 18
                            && (clonk.position.y - carriage.position.y).abs() <= 22
                    })
            },
        );
    }

    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyX);
    if app
        .engine
        .object_snapshot(valley)
        .is_some_and(|object| object.action.name != "Push")
    {
        AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyX);
    }
    advance_app_until(&mut app, "valley CLNK grabs ELEC for ascent", 120, |app| {
        app.engine.object_snapshot(valley).is_some_and(|object| {
            object.action.name == "Push" && object.action.target == Some(carriage)
        })
    });
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyE);
    assert_eq!(
        app.engine.crew_cursor(app.local_owner),
        Some(catapult_clonk)
    );
    hold_app_key_until(
        &mut app,
        VirtualKeyCode::KeyC,
        "right-hill CLNK centers on ELEC before grabbing",
        40,
        |app| {
            app.engine
                .object_snapshot(catapult_clonk)
                .is_some_and(|object| object.position.x >= 161 && object.action.name == "Walk")
        },
    );
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyX);
    if app
        .engine
        .object_snapshot(catapult_clonk)
        .is_some_and(|object| object.action.name != "Push")
    {
        AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyX);
    }
    advance_app_until(
        &mut app,
        "right-hill CLNK grabs ELEC for ascent",
        120,
        |app| {
            app.engine
                .object_snapshot(catapult_clonk)
                .is_some_and(|object| {
                    object.action.name == "Push" && object.action.target == Some(carriage)
                })
        },
    );
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyE);
    assert_eq!(app.engine.crew_cursor(app.local_owner), Some(constructor));
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyX);
    if app
        .engine
        .object_snapshot(constructor)
        .is_some_and(|object| object.action.name != "Push")
    {
        AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyX);
    }
    advance_app_until(&mut app, "constructor grabs ELEC for ascent", 120, |app| {
        app.engine
            .object_snapshot(constructor)
            .is_some_and(|object| {
                object.action.name == "Push" && object.action.target == Some(carriage)
            })
    });

    // W/W becomes COM_CursorToggle_D -> SelectAllCrew. Grab and Enter are
    // PlayerObjectCommands over that exact selection; the cursor directly
    // controls ELEC during the ascent (Script.c:303-330;
    // C4Player.cpp:1319-1353,1397-1443; C4ObjectCom.cpp:335-350;
    // C4Command.cpp:545-617).
    {
        let mut keyboard = AppVirtualKeyboard::new(&mut app);
        keyboard.tap(VirtualKeyCode::KeyW);
        keyboard.tap(VirtualKeyCode::KeyW);
    }
    let mut selected = app.engine.selected_crew(app.local_owner);
    selected.sort_by_key(|object| object.as_u64());
    let mut expected = vec![constructor, valley, catapult_clonk];
    expected.sort_by_key(|object| object.as_u64());
    assert_eq!(
        selected, expected,
        "physical W/W must select all exact crew"
    );
    advance_app_until(
        &mut app,
        "Tutorial05 Script211 asks all Clonks to return to HUT3",
        300,
        |app| app_tutorial_message_contains(app, "move all clonks back into the home base"),
    );
    hold_app_key_until(
        &mut app,
        VirtualKeyCode::KeyS,
        "ELEC carries the selected crew to the cabin hill",
        600,
        |app| {
            app.engine
                .object_snapshot(carriage)
                .is_some_and(|object| object.position.y <= 105)
        },
    );
    advance_app_until(
        &mut app,
        "all selected Clonks arrive at the shaft top",
        240,
        |app| {
            [constructor, valley, catapult_clonk]
                .into_iter()
                .all(|clonk| {
                    app.engine
                        .object_snapshot(clonk)
                        .is_some_and(|object| object.position.y <= 130)
                })
        },
    );

    for (clonk, caption) in [
        (constructor, "constructor"),
        (valley, "valley CLNK"),
        (catapult_clonk, "right-hill CLNK"),
    ] {
        assert_eq!(app.engine.crew_cursor(app.local_owner), Some(clonk));
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard.tap(VirtualKeyCode::KeyX);
            keyboard.tap(VirtualKeyCode::KeyX);
        }
        advance_app_until(
            &mut app,
            &format!("{caption} releases ELEC at the top"),
            80,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.action.name == "Walk")
            },
        );
        if clonk != catapult_clonk {
            AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyE);
        }
    }

    {
        let mut keyboard = AppVirtualKeyboard::new(&mut app);
        keyboard.press(VirtualKeyCode::KeyZ);
        keyboard.tap(VirtualKeyCode::KeyS);
    }
    advance_app_until(
        &mut app,
        "right-hill CLNK jumps over the shaft-top lip",
        80,
        |app| {
            app.engine
                .object_snapshot(catapult_clonk)
                .is_some_and(|object| object.position.x <= 145)
        },
    );
    AppVirtualKeyboard::new(&mut app).release(VirtualKeyCode::KeyZ);
    advance_app_until(
        &mut app,
        "right-hill CLNK lands on the cabin plateau",
        160,
        |app| {
            app.engine
                .object_snapshot(catapult_clonk)
                .is_some_and(|object| {
                    object.action.name == "Walk"
                        && object.position.x < 155
                        && object.position.y <= 115
                })
        },
    );
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyE);
    assert_eq!(app.engine.crew_cursor(app.local_owner), Some(constructor));
    {
        let mut keyboard = AppVirtualKeyboard::new(&mut app);
        keyboard.press(VirtualKeyCode::KeyZ);
        keyboard.tap(VirtualKeyCode::KeyS);
    }
    advance_app_until(
        &mut app,
        "constructor jumps over the shaft-top lip",
        80,
        |app| {
            app.engine
                .object_snapshot(constructor)
                .is_some_and(|object| object.position.x <= 145)
        },
    );
    AppVirtualKeyboard::new(&mut app).release(VirtualKeyCode::KeyZ);
    advance_app_until(
        &mut app,
        "constructor lands on the cabin plateau",
        160,
        |app| {
            app.engine
                .object_snapshot(constructor)
                .is_some_and(|object| {
                    object.action.name == "Walk"
                        && object.position.x < 155
                        && object.position.y <= 115
                })
        },
    );
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyE);
    assert_eq!(app.engine.crew_cursor(app.local_owner), Some(valley));
    hold_app_key_until(
        &mut app,
        VirtualKeyCode::KeyC,
        "valley CLNK takes a run-up on ELEC",
        40,
        |app| {
            app.engine
                .object_snapshot(valley)
                .is_some_and(|object| object.position.x >= 169 && object.action.name == "Walk")
        },
    );
    {
        let mut keyboard = AppVirtualKeyboard::new(&mut app);
        keyboard.press(VirtualKeyCode::KeyZ);
        keyboard.tap(VirtualKeyCode::KeyS);
    }
    advance_app_until(
        &mut app,
        "valley CLNK jumps over the shaft-top lip",
        80,
        |app| {
            app.engine
                .object_snapshot(valley)
                .is_some_and(|object| object.position.x <= 145)
        },
    );
    AppVirtualKeyboard::new(&mut app).release(VirtualKeyCode::KeyZ);
    advance_app_until(
        &mut app,
        "valley CLNK lands on the cabin plateau",
        160,
        |app| {
            app.engine.object_snapshot(valley).is_some_and(|object| {
                object.action.name == "Walk" && object.position.x < 155 && object.position.y <= 115
            })
        },
    );
    {
        let mut keyboard = AppVirtualKeyboard::new(&mut app);
        keyboard.tap(VirtualKeyCode::KeyW);
        keyboard.tap(VirtualKeyCode::KeyW);
    }
    assert_eq!(app.engine.crew_cursor(app.local_owner), Some(constructor));
    assert!(
        [constructor, valley, catapult_clonk]
            .into_iter()
            .all(|clonk| app.engine.selected_crew(app.local_owner).contains(&clonk)),
        "all exact Tutorial05 Clonks must be reselected on the plateau"
    );

    let hut_x = app.engine.test_object_snapshot(home_base).position.x;
    hold_app_key_until(
        &mut app,
        VirtualKeyCode::KeyZ,
        "selected crew follows constructor to HUT3",
        360,
        |app| {
            app.engine
                .object_snapshot(constructor)
                .is_some_and(|object| object.position.x <= hut_x + 19)
        },
    );
    hold_app_key_until(
        &mut app,
        VirtualKeyCode::KeyX,
        "constructor descends from the HUT3 wall",
        80,
        |app| {
            app.engine
                .object_snapshot(constructor)
                .is_some_and(|object| object.action.name == "Walk")
        },
    );
    hold_app_key_until(
        &mut app,
        VirtualKeyCode::KeyC,
        "constructor walks into HUT3's real entrance",
        80,
        |app| {
            app.engine
                .object_snapshot(constructor)
                .is_some_and(|object| {
                    object.action.name == "Walk"
                        && (hut_x + 2..=hut_x + 19).contains(&object.position.x)
                })
        },
    );
    let hut_position = app.engine.test_object_snapshot(home_base).position;
    advance_app_until(
        &mut app,
        "all selected followers reach HUT3's entrance rectangle",
        240,
        |app| {
            [constructor, valley, catapult_clonk]
                .into_iter()
                .all(|clonk| {
                    app.engine.object_snapshot(clonk).is_some_and(|object| {
                        (hut_position.x + 2..=hut_position.x + 19).contains(&object.position.x)
                            && (hut_position.y + 4..=hut_position.y + 21)
                                .contains(&object.position.y)
                    })
                })
        },
    );
    // HUT3's Entrance=2,4,17,21 is half-open in C4Shape::GetEntranceArea,
    // so x+19 is just outside. Nudge the valley follower inside x+18,
    // then cycle without another tick and queue physical Up on each exact
    // crew member before HUT3 opens its context menu (Hut3.c4d/
    // DefCore.txt:18; C4ObjectCom.cpp:335-350; C4Command.cpp:545-617).
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyE);
    assert_eq!(app.engine.crew_cursor(app.local_owner), Some(valley));
    if app
        .engine
        .object_snapshot(valley)
        .is_some_and(|object| object.position.x > hut_position.x + 18)
    {
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::KeyZ,
            "valley CLNK steps fully inside HUT3's entrance",
            20,
            |app| {
                app.engine
                    .object_snapshot(valley)
                    .is_some_and(|object| object.position.x <= hut_position.x + 18)
            },
        );
    }
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyE);
    assert_eq!(
        app.engine.crew_cursor(app.local_owner),
        Some(catapult_clonk)
    );
    if app
        .engine
        .object_snapshot(catapult_clonk)
        .is_some_and(|object| object.position.x > hut_position.x + 18)
    {
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::KeyZ,
            "right-hill CLNK steps fully inside HUT3's entrance",
            20,
            |app| {
                app.engine
                    .object_snapshot(catapult_clonk)
                    .is_some_and(|object| object.position.x <= hut_position.x + 18)
            },
        );
    }
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyE);
    assert_eq!(app.engine.crew_cursor(app.local_owner), Some(constructor));
    if app
        .engine
        .object_snapshot(constructor)
        .is_some_and(|object| object.position.x > hut_position.x + 18)
    {
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::KeyZ,
            "constructor steps fully inside HUT3's entrance",
            20,
            |app| {
                app.engine
                    .object_snapshot(constructor)
                    .is_some_and(|object| object.position.x <= hut_position.x + 18)
            },
        );
    }
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyS);
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyE);
    assert_eq!(app.engine.crew_cursor(app.local_owner), Some(valley));
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyS);
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyE);
    assert_eq!(
        app.engine.crew_cursor(app.local_owner),
        Some(catapult_clonk)
    );
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyS);
    advance_app_until(
        &mut app,
        "all three exact Clonks enter Tutorial05's real HUT3",
        360,
        |app| {
            [constructor, valley, catapult_clonk]
                .into_iter()
                .all(|clonk| {
                    app.engine
                        .object_snapshot(clonk)
                        .is_some_and(|object| object.container == Some(home_base))
                })
        },
    );
    advance_app_until(&mut app, "Tutorial05 selects Tutorial06", 400, |app| {
        app.engine.next_mission().path == r"Tutorial.c4f\Tutorial06.c4s"
    });
    advance_app_until(&mut app, "Tutorial05 reaches GameOver", 400, |app| {
        app.snapshot.game_over && app.game_over_dialog.is_some()
    });
    assert!(
        app.snapshot
            .round_results
            .fulfilled_goals
            .iter()
            .any(|goal| goal == "SCRG"),
        "Tutorial05 must fulfill its real SCRG before GameOver"
    );
    assert_eq!(
        app.engine.next_mission().path,
        r"Tutorial.c4f\Tutorial06.c4s"
    );
}

fn app_tutorial08_wipfs_in_lorry(app: &GameApp, lorry: ObjectId) -> usize {
    app.engine
        .snapshot()
        .objects
        .into_iter()
        .filter(|object| object.definition_id == "WIPF" && object.container == Some(lorry))
        .count()
}

fn app_tutorial08_catch_and_load_one_wipf(
    app: &mut GameApp,
    clonk: ObjectId,
    lorry: ObjectId,
    delivery: usize,
) -> ObjectId {
    for key in [
        VirtualKeyCode::KeyZ,
        VirtualKeyCode::KeyC,
        VirtualKeyCode::KeyS,
        VirtualKeyCode::KeyX,
    ] {
        AppVirtualKeyboard::new(app).release(key);
    }
    for _ in 0..12 {
        app.test_update();
    }

    let mut search_key = if app
        .engine
        .object_snapshot(clonk)
        .is_some_and(|object| object.position.x < 400)
    {
        VirtualKeyCode::KeyC
    } else {
        VirtualKeyCode::KeyZ
    };
    AppVirtualKeyboard::new(app).press(search_key);
    for _ in 0..4_000 {
        app.test_update();
        let carried = app
            .engine
            .object_snapshot(clonk)
            .and_then(|object| object.contents.first().copied());
        if let Some(carried) = carried {
            if app
                .engine
                .object_snapshot(carried)
                .is_some_and(|object| object.definition_id == "WIPF")
            {
                break;
            }
            AppVirtualKeyboard::new(app).release(search_key);
            let behind = if search_key == VirtualKeyCode::KeyC {
                VirtualKeyCode::KeyZ
            } else {
                VirtualKeyCode::KeyC
            };
            AppVirtualKeyboard::new(app).tap(behind);
            AppVirtualKeyboard::new(app).tap(VirtualKeyCode::KeyA);
            advance_app_until(
                app,
                "incidental WIPF-sweep material leaves CLNK",
                30,
                |app| {
                    app.engine
                        .object_snapshot(carried)
                        .is_none_or(|object| object.container != Some(clonk))
                },
            );
            for _ in 0..12 {
                app.test_update();
            }
            AppVirtualKeyboard::new(app).press(search_key);
        }
        let x = app.engine.test_object_snapshot(clonk).position.x;
        let next_key = if x <= 8 {
            VirtualKeyCode::KeyC
        } else if x >= 790 {
            VirtualKeyCode::KeyZ
        } else {
            search_key
        };
        if next_key != search_key {
            AppVirtualKeyboard::new(app).release(search_key);
            for _ in 0..12 {
                app.test_update();
            }
            search_key = next_key;
            AppVirtualKeyboard::new(app).press(search_key);
        }
    }
    AppVirtualKeyboard::new(app).release(search_key);
    let caught_wipf = app
            .engine
            .object_snapshot(clonk)
            .and_then(|object| object.contents.first().copied())
            .unwrap_or_else(|| {
                let free_wipfs = app
                    .engine
                    .snapshot()
                    .objects
                    .into_iter()
                    .filter(|object| {
                        object.definition_id == "WIPF" && object.container.is_none()
                    })
                    .collect::<Vec<_>>();
                panic!(
                    "physical surface sweep cannot catch WIPF {delivery}; clonk={:?}; free_wipfs={free_wipfs:?}",
                    app.engine.object_snapshot(clonk)
                )
            });
    assert_eq!(
        app.engine
            .object_snapshot(caught_wipf)
            .expect("caught Tutorial08 object survives")
            .definition_id,
        "WIPF",
        "caught surface object {delivery} is a WIPF"
    );

    for _ in 0..12 {
        app.test_update();
    }
    let toward_lorry = app
        .engine
        .object_snapshot(clonk)
        .zip(app.engine.object_snapshot(lorry))
        .map(|(clonk, lorry)| {
            if clonk.position.x < lorry.position.x {
                VirtualKeyCode::KeyC
            } else {
                VirtualKeyCode::KeyZ
            }
        })
        .test_value();
    hold_app_key_until(
        app,
        toward_lorry,
        &format!("WIPF {delivery}-carrying CLNK returns to LORY"),
        1_200,
        |app| {
            app.engine
                .object_snapshot(clonk)
                .zip(app.engine.object_snapshot(lorry))
                .is_some_and(|(clonk, lorry)| {
                    (clonk.position.x - lorry.position.x).abs() <= 8
                        && (clonk.position.y - lorry.position.y).abs() <= 18
                })
        },
    );
    {
        let mut keyboard = AppVirtualKeyboard::new(app);
        keyboard.tap(VirtualKeyCode::KeyX);
        keyboard.tap(VirtualKeyCode::KeyX);
    }
    advance_app_until(app, "WIPF-carrying CLNK grabs LORY", 80, |app| {
        app.engine.object_snapshot(clonk).is_some_and(|object| {
            object.action.name == "Push" && object.action.target == Some(lorry)
        })
    });
    AppVirtualKeyboard::new(app).tap(VirtualKeyCode::KeyA);
    advance_app_until(
        app,
        &format!("caught WIPF {delivery} enters LORY"),
        60,
        |app| {
            app.engine
                .object_snapshot(caught_wipf)
                .is_some_and(|object| object.container == Some(lorry))
        },
    );
    {
        let mut keyboard = AppVirtualKeyboard::new(app);
        keyboard.tap(VirtualKeyCode::KeyX);
        keyboard.tap(VirtualKeyCode::KeyX);
    }
    advance_app_until(app, "CLNK releases loaded LORY", 60, |app| {
        app.engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Walk")
    });
    caught_wipf
}

#[test]
fn app_virtual_keyboard_delivers_tutorial08_surface_wipfs_to_hut() {
    // Tutorial08 starts the real scenario RNG animal placement, creates
    // HUT3/LORY beside the player and asks for ten WIPFs. Ordinary Z/C
    // walking sweeps the actual surface, X/X grabs/releases LORY, and A
    // loads each carried animal. Every edge goes through GameApp::handle_key
    // under the fresh AutoStop player; neither animal nor vehicle state is
    // positioned by the test (Tutorial08.c4s/Script.c:18-69;
    // C4Object.cpp:3682-3724; Wipf.c4d/Script.c:263-271;
    // Lorry.c4d/Script.c:65-78).
    let mut app = real_tutorial_app(8, "Tutorial 8 app virtual player");
    let owner = app.local_owner;
    let clonk = app.engine.test_crew_cursor(owner);
    let lorry = app_object_with_definition(&app, "LORY").test_value();
    let hut = app_object_with_definition(&app, "HUT3").test_value();
    assert!(
        app.engine
            .player(owner)
            .is_some_and(|player| player.control_style()),
        "fresh app Tutorial08 player uses C++ AutoStopControl"
    );
    let mut cave_wipfs = app
        .engine
        .snapshot()
        .objects
        .into_iter()
        .filter(|object| {
            object.definition_id == "WIPF" && object.container.is_none() && object.position.y > 450
        })
        .map(|object| (object.id.as_u64(), object.position.x, object.position.y))
        .collect::<Vec<_>>();
    cave_wipfs.sort_unstable();
    assert_eq!(
        cave_wipfs,
        vec![
            (57, 569, 500),
            (58, 202, 619),
            (59, 393, 629),
            (64, 278, 631)
        ],
        "C++ RandomSeed 0 / MapSeed 59893 places four free WIPFs in these caves"
    );
    advance_app_until(&mut app, "Tutorial08 teaches catching WIPFs", 500, |app| {
        app_tutorial_message_contains(app, "catch them either by hand or with the lorry")
    });
    assert_eq!(
        app.engine
            .snapshot()
            .objects
            .into_iter()
            .filter(|object| object.definition_id == "WIPF")
            .count(),
        10,
        "real Tutorial08 activation places ten WIPFs"
    );

    let mut search_key = if app
        .engine
        .object_snapshot(clonk)
        .is_some_and(|object| object.position.x < 400)
    {
        VirtualKeyCode::KeyC
    } else {
        VirtualKeyCode::KeyZ
    };
    AppVirtualKeyboard::new(&mut app).press(search_key);
    for _ in 0..4_000 {
        app.test_update();
        let carried = app
            .engine
            .object_snapshot(clonk)
            .and_then(|object| object.contents.first().copied());
        if let Some(carried) = carried {
            if app
                .engine
                .object_snapshot(carried)
                .is_some_and(|object| object.definition_id == "WIPF")
            {
                break;
            }
            // Correct System-name RNG placement crosses one exposed
            // LOAM before the first WIPF. Face back and throw incidental
            // terrain behind the sweep, just as a player clears CLNK's
            // one ordinary inventory slot.
            AppVirtualKeyboard::new(&mut app).release(search_key);
            let behind = if search_key == VirtualKeyCode::KeyC {
                VirtualKeyCode::KeyZ
            } else {
                VirtualKeyCode::KeyC
            };
            AppVirtualKeyboard::new(&mut app).tap(behind);
            AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyA);
            advance_app_until(
                &mut app,
                "incidental surface material leaves CLNK inventory",
                30,
                |app| {
                    app.engine
                        .object_snapshot(carried)
                        .is_none_or(|object| object.container != Some(clonk))
                },
            );
            for _ in 0..12 {
                app.test_update();
            }
            AppVirtualKeyboard::new(&mut app).press(search_key);
        }
        let x = app.engine.test_object_snapshot(clonk).position.x;
        let next_key = if x <= 8 {
            VirtualKeyCode::KeyC
        } else if x >= 790 {
            VirtualKeyCode::KeyZ
        } else {
            search_key
        };
        if next_key != search_key {
            AppVirtualKeyboard::new(&mut app).release(search_key);
            for _ in 0..12 {
                app.test_update();
            }
            search_key = next_key;
            AppVirtualKeyboard::new(&mut app).press(search_key);
        }
    }
    AppVirtualKeyboard::new(&mut app).release(search_key);
    let caught_wipf = app
        .engine
        .object_snapshot(clonk)
        .and_then(|object| object.contents.first().copied())
        .test_value();
    assert_eq!(
        app.engine
            .object_snapshot(caught_wipf)
            .expect("caught Tutorial08 object survives")
            .definition_id,
        "WIPF",
        "first caught surface object is a WIPF"
    );

    for key in [
        VirtualKeyCode::KeyZ,
        VirtualKeyCode::KeyC,
        VirtualKeyCode::KeyS,
        VirtualKeyCode::KeyX,
    ] {
        AppVirtualKeyboard::new(&mut app).release(key);
    }
    for _ in 0..12 {
        app.test_update();
    }
    let toward_lorry = app
        .engine
        .object_snapshot(clonk)
        .zip(app.engine.object_snapshot(lorry))
        .map(|(clonk, lorry)| {
            if clonk.position.x < lorry.position.x {
                VirtualKeyCode::KeyC
            } else {
                VirtualKeyCode::KeyZ
            }
        })
        .test_value();
    hold_app_key_until(
        &mut app,
        toward_lorry,
        "WIPF-carrying CLNK returns to LORY",
        1_200,
        |app| {
            app.engine
                .object_snapshot(clonk)
                .zip(app.engine.object_snapshot(lorry))
                .is_some_and(|(clonk, lorry)| {
                    (clonk.position.x - lorry.position.x).abs() <= 8
                        && (clonk.position.y - lorry.position.y).abs() <= 18
                })
        },
    );
    {
        let mut keyboard = AppVirtualKeyboard::new(&mut app);
        keyboard.tap(VirtualKeyCode::KeyX);
        keyboard.tap(VirtualKeyCode::KeyX);
    }
    advance_app_until(&mut app, "WIPF-carrying CLNK grabs LORY", 80, |app| {
        app.engine.object_snapshot(clonk).is_some_and(|object| {
            object.action.name == "Push" && object.action.target == Some(lorry)
        })
    });
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyA);
    advance_app_until(&mut app, "first caught WIPF enters LORY", 60, |app| {
        app.engine
            .object_snapshot(caught_wipf)
            .is_some_and(|object| object.container == Some(lorry))
    });
    {
        let mut keyboard = AppVirtualKeyboard::new(&mut app);
        keyboard.tap(VirtualKeyCode::KeyX);
        keyboard.tap(VirtualKeyCode::KeyX);
    }
    advance_app_until(&mut app, "CLNK releases first-loaded LORY", 60, |app| {
        app.engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Walk")
    });
    assert_eq!(
        app.engine
            .snapshot()
            .objects
            .into_iter()
            .filter(|object| { object.definition_id == "WIPF" && object.container == Some(lorry) })
            .count(),
        2,
        "C++ rotation accumulation lets one nearby WIPF enter LORY before the first hand delivery"
    );
    // Keep delivering until every surface WIPF is aboard. CrossCheck is
    // live while the player walks: a roaming WIPF may independently
    // cross LORY's collection rectangle during the same trip that loads
    // the carried one. C++ therefore guarantees monotonic progress and
    // the exact final six, not one new content link per player trip.
    for delivery in 3..=6 {
        let before = app_tutorial08_wipfs_in_lorry(&app, lorry);
        if before >= 6 {
            break;
        }
        let caught = app_tutorial08_catch_and_load_one_wipf(&mut app, clonk, lorry, delivery);
        assert_eq!(
            app.engine
                .object_snapshot(caught)
                .expect("delivered Tutorial08 WIPF survives in LORY")
                .container,
            Some(lorry),
            "physical delivery {delivery} loads its exact WIPF into LORY"
        );
        let after = app_tutorial08_wipfs_in_lorry(&app, lorry);
        assert!(
            after > before && after <= 6,
            "physical delivery {delivery} advances the bounded C++ LORY stack: \
                 before={before}, after={after}"
        );
    }
    assert_eq!(
        app_tutorial08_wipfs_in_lorry(&app, lorry),
        6,
        "real default-seed surface sweep loads all six walkable WIPFs into LORY"
    );

    // C4Game::PlaceAnimal legitimately chose underground caves for the
    // remaining C++-seed animals; the app route must not relocate them.
    // Deliver the six physically reachable WIPFs through LORY's ordinary
    // Entrance first:
    // Push aligns the vehicle with HUT3, S enters it, and LORY::Entrance
    // calls HUT3::GrabContents (C4Game.cpp:3046-3105;
    // Lorry.c4d/Script.c:80-91; C4Object.cpp:3682-3701).
    let subterranean_wipfs = app
        .engine
        .snapshot()
        .objects
        .into_iter()
        .filter(|object| {
            object.definition_id == "WIPF" && object.container.is_none() && object.position.y > 450
        })
        .count();
    assert_eq!(
        subterranean_wipfs, 4,
        "C++ seed leaves four live WIPFs in underground caves"
    );
    for _ in 0..12 {
        app.test_update();
    }
    let toward_lorry = app
        .engine
        .object_snapshot(clonk)
        .zip(app.engine.object_snapshot(lorry))
        .map(|(clonk, lorry)| {
            if clonk.position.x < lorry.position.x {
                VirtualKeyCode::KeyC
            } else {
                VirtualKeyCode::KeyZ
            }
        })
        .test_value();
    hold_app_key_until(
        &mut app,
        toward_lorry,
        "CLNK returns to six-WIPF LORY",
        1_200,
        |app| {
            app.engine
                .object_snapshot(clonk)
                .zip(app.engine.object_snapshot(lorry))
                .is_some_and(|(clonk, lorry)| {
                    (clonk.position.x - lorry.position.x).abs() <= 8
                        && (clonk.position.y - lorry.position.y).abs() <= 18
                })
        },
    );
    {
        let mut keyboard = AppVirtualKeyboard::new(&mut app);
        keyboard.tap(VirtualKeyCode::KeyX);
        keyboard.tap(VirtualKeyCode::KeyX);
    }
    advance_app_until(&mut app, "CLNK grabs six-WIPF LORY", 80, |app| {
        app.engine.object_snapshot(clonk).is_some_and(|object| {
            object.action.name == "Push" && object.action.target == Some(lorry)
        })
    });
    let toward_hut = app
        .engine
        .object_snapshot(lorry)
        .zip(app.engine.object_snapshot(hut))
        .map(|(lorry, hut)| {
            if lorry.position.x < hut.position.x + 10 {
                VirtualKeyCode::KeyC
            } else {
                VirtualKeyCode::KeyZ
            }
        })
        .test_value();
    hold_app_key_until(
        &mut app,
        toward_hut,
        "six-WIPF LORY aligns with HUT3 entrance",
        600,
        |app| {
            app.engine
                .object_snapshot(lorry)
                .zip(app.engine.object_snapshot(hut))
                .is_some_and(|(lorry, hut)| {
                    let dx = lorry.position.x - hut.position.x;
                    let dy = lorry.position.y - hut.position.y;
                    (2..19).contains(&dx) && (4..25).contains(&dy)
                })
        },
    );
    AppVirtualKeyboard::new(&mut app).tap(VirtualKeyCode::KeyS);
    advance_app_until(&mut app, "loaded LORY enters HUT3", 80, |app| {
        app.engine
            .object_snapshot(lorry)
            .is_some_and(|object| object.container == Some(hut))
    });
    advance_app_until(&mut app, "LORY unloads six WIPFs into HUT3", 80, |app| {
        app.engine
            .snapshot()
            .objects
            .into_iter()
            .filter(|object| object.definition_id == "WIPF" && object.container == Some(hut))
            .count()
            == 6
    });
    assert_eq!(
        app_tutorial08_wipfs_in_lorry(&app, lorry),
        0,
        "LORY's C++ Entrance callback empties its surface WIPFs into HUT3"
    );
}

#[test]
fn real_tutorial01_saved_game_music_subcases_batch() {
    let prepared = PreparedRealInstalledScenario::new("Tutorial.c4f/Tutorial01.c4s");
    let mut failures = Vec::new();
    run_real_tutorial01_app_subcase(
        "saved_game_restores_music_level_after_scenario_reconfiguration",
        &mut failures,
        || saved_game_restores_music_level_after_scenario_reconfiguration(&prepared),
    );
    run_real_tutorial01_app_subcase(
        "saved_game_resume_uses_default_playlist_but_preserves_saved_filter",
        &mut failures,
        || saved_game_resume_uses_default_playlist_but_preserves_saved_filter(&prepared),
    );
    assert_no_real_tutorial01_app_subcase_failures(failures);
}

fn saved_game_restores_music_level_after_scenario_reconfiguration(
    prepared: &PreparedRealInstalledScenario,
) {
    let mut app = prepared.instantiate("MusicLevel restore parity", false);
    let paths = cached_app_paths().test_value();
    paths.ensure_user_dirs().test_value();
    fs::write(paths.config_file(), "[General]\nLanguageEx=US\n").test_value();
    let scenario = app.active_scenario.clone().test_value();
    let mut engine_state = app.engine.capture_state();
    engine_state.music_level = 25;
    let save = SavedGameFile {
        version: SAVE_FILE_VERSION,
        saved_at_seconds: 0,
        scenario: SavedScenarioInfo::from_frontend(
            &scenario,
            &app.scenario_label,
            app.fallback_ground,
        ),
        definition_load: app.active_definition_load.clone(),
        focus_id: app.focus_id,
        user_label: Some("restored music level".to_string()),
        runtime_music_enabled: Some(false),
        source_save_player_infos: None,
        source_string_table: None,
        source_title_png: None,
        engine_state,
    };
    let save: SavedGameFile =
        serde_json::from_str(&serde_json::to_string(&save).expect("serialize tutorial save"))
            .test_value();
    app.audio.test_mut().set_scenario_music_level(Some(73));

    app.apply_loaded_game(save).test_value();

    assert_eq!(app.engine.capture_state().music_level, 25);
    let audio = app.audio.test_ref();
    let control = lock_unpoisoned(&audio.music_control);
    assert_eq!(control.scenario_level, Some(25));
    assert!((control.effective_volume() - audio.options.music_volume * 0.25).abs() < f32::EPSILON);
}

fn saved_game_resume_uses_default_playlist_but_preserves_saved_filter(
    prepared: &PreparedRealInstalledScenario,
) {
    let mut app = prepared.instantiate("Music resume ordering parity", false);
    let paths = cached_app_paths().test_value();
    paths.ensure_user_dirs().test_value();
    fs::write(paths.config_file(), "[General]\nLanguageEx=US\n").test_value();
    let scenario = app.active_scenario.clone().test_value();
    let mut engine_state = app.engine.capture_state();
    engine_state.play_list = Some("Theme*".to_string());
    let save = SavedGameFile {
        version: SAVE_FILE_VERSION,
        saved_at_seconds: 0,
        scenario: SavedScenarioInfo::from_frontend(
            &scenario,
            &app.scenario_label,
            app.fallback_ground,
        ),
        definition_load: app.active_definition_load.clone(),
        focus_id: app.focus_id,
        user_label: Some("restored playlist".to_string()),
        runtime_music_enabled: Some(false),
        source_save_player_infos: None,
        source_string_table: None,
        source_title_png: None,
        engine_state,
    };
    app.audio.test_mut().options.music_enabled = true;

    app.apply_loaded_game(save).test_value();

    assert!(app.runtime_music_enabled, "RXMusic force-enables resume");
    assert_eq!(
        app.engine.capture_state().play_list.as_deref(),
        Some("Theme*"),
        "Game.PlayList remains available to script and the next save"
    );
    assert_eq!(
        app.audio
            .as_ref()
            .expect("test audio")
            .music_resolver
            .playlist
            .as_deref(),
        None,
        "PlayScenarioMusic installs the physical DEFAULT filter"
    );

    app.snapshot = app.engine.test_tick();
    assert!(
        app.snapshot
            .audio
            .iter()
            .all(|command| !matches!(command, AudioCommand::SetMusicPlaylist { .. })),
        "the delayed restore command cannot reinstall the saved filter"
    );
    app.update_audio();
    assert_eq!(
        app.audio
            .as_ref()
            .expect("test audio")
            .music_resolver
            .playlist
            .as_deref(),
        None
    );
    assert_eq!(
        app.engine.capture_state().play_list.as_deref(),
        Some("Theme*")
    );
}
