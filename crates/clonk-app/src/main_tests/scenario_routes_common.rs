// Spliced into `mod tests` (src/main_tests.rs) via include!: a bare item
// sequence, not a child module, so test ids stay `tests::<fn>`.

    fn advance_app_until(
        app: &mut GameApp,
        milestone: &str,
        max_ticks: u32,
        mut reached: impl FnMut(&GameApp) -> bool,
    ) {
        advance_app_until_erased(app, milestone, max_ticks, &mut reached);
    }

    #[inline(never)]
    fn advance_app_until_erased(
        app: &mut GameApp,
        milestone: &str,
        max_ticks: u32,
        reached: &mut dyn FnMut(&GameApp) -> bool,
    ) {
        if reached(app) {
            return;
        }
        for _ in 0..max_ticks {
            app.update()
                .unwrap_or_else(|error| panic!("{milestone}: {error}"));
            if reached(app) {
                return;
            }
        }
        panic!(
            "timed out after {max_ticks} app ticks waiting for {milestone} at frame {}; cursor={:?}",
            app.engine.frame(),
            app.engine
                .crew_cursor(app.local_owner)
                .and_then(|cursor| app.engine.object_snapshot(cursor))
        );
    }

    fn hold_app_key_until(
        app: &mut GameApp,
        key: VirtualKeyCode,
        milestone: &str,
        max_ticks: u32,
        mut reached: impl FnMut(&GameApp) -> bool,
    ) {
        hold_app_key_until_erased(app, key, milestone, max_ticks, &mut reached);
    }

    #[inline(never)]
    fn hold_app_key_until_erased(
        app: &mut GameApp,
        key: VirtualKeyCode,
        milestone: &str,
        max_ticks: u32,
        reached: &mut dyn FnMut(&GameApp) -> bool,
    ) {
        AppVirtualKeyboard::new(app)
            .press(key)
            .unwrap_or_else(|error| panic!("press physical {key:?} for {milestone}: {error}"));
        advance_app_until_erased(app, milestone, max_ticks, reached);
        AppVirtualKeyboard::new(app)
            .release(key)
            .unwrap_or_else(|error| panic!("release physical {key:?} after {milestone}: {error}"));
    }

    fn app_tutorial_message_contains(app: &GameApp, needle: &str) -> bool {
        app.snapshot
            .hud
            .messages
            .iter()
            .any(|message| message.lines.iter().any(|line| line.contains(needle)))
    }

    fn app_object_with_definition(app: &GameApp, definition: &str) -> Option<ObjectId> {
        app.engine
            .snapshot()
            .objects
            .into_iter()
            .find(|object| object.definition_id == definition)
            .map(|object| object.id)
    }

    fn app_object_with_definition_near_x(
        app: &GameApp,
        definition: &str,
        expected_x: i32,
    ) -> Option<ObjectId> {
        app.engine
            .snapshot()
            .objects
            .into_iter()
            .filter(|object| object.definition_id == definition)
            .min_by_key(|object| (object.position.x - expected_x).abs())
            .map(|object| object.id)
    }

    fn app_clonk_carries(app: &GameApp, clonk: ObjectId, definition: &str) -> bool {
        app.engine.object_snapshot(clonk).is_some_and(|clonk| {
            clonk.contents.iter().any(|item| {
                app.engine
                    .object_snapshot(*item)
                    .is_some_and(|item| item.definition_id == definition)
            })
        })
    }

    fn app_object_contents_count(app: &GameApp, container: ObjectId, definition: &str) -> usize {
        app.engine
            .object_snapshot(container)
            .map_or(0, |container| {
                container
                    .contents
                    .iter()
                    .filter(|object_id| {
                        app.engine
                            .object_snapshot(**object_id)
                            .is_some_and(|object| object.definition_id == definition)
                    })
                    .count()
            })
    }

    fn app_selected_object_menu_item(app: &GameApp) -> Option<&clonk_engine::ObjectMenuItem> {
        app.engine
            .cursor_object_menu(app.local_owner)
            .and_then(|(_, menu)| {
                usize::try_from(menu.selection)
                    .ok()
                    .and_then(|selection| menu.items.get(selection))
            })
    }

    fn app_navigate_object_menu_to(
        app: &mut GameApp,
        target: &str,
        mut matches: impl FnMut(&clonk_engine::ObjectMenuItem) -> bool,
    ) {
        let item_count = app
            .engine
            .cursor_object_menu(app.local_owner)
            .unwrap_or_else(|| panic!("object menu exists while selecting {target}"))
            .1
            .items
            .len();
        for _ in 0..item_count {
            if app_selected_object_menu_item(app).is_some_and(&mut matches) {
                return;
            }
            AppVirtualKeyboard::new(app)
                .tap(VirtualKeyCode::KeyX)
                .unwrap_or_else(|error| panic!("physical X selects {target}: {error}"));
        }
        panic!("physical menu navigation could not select {target}");
    }

    fn app_collect_one_gold_around_blast_debris(app: &mut GameApp, clonk: ObjectId) {
        // Blasts also expose collectible ROCK. C++ CLNK has exactly one
        // nonspecial slot, so a real player must throw incidental debris away
        // before GOLD can enter (Clonk.c4d/Script.c:738-763). Keep all motion,
        // scale/hangle release and Throw/Drop transitions on physical keys
        // (C4Object.cpp:3618-3640; C4ObjectCom.cpp:625-675).
        hold_app_key_until(
            app,
            VirtualKeyCode::KeyZ,
            "CLNK first approaches exposed GOLD",
            120,
            |app| {
                app.engine.object_snapshot(clonk).is_some_and(|object| {
                    app_clonk_carries(app, clonk, "GOLD")
                        || !object.contents.is_empty()
                        || object.action.name == "Hangle"
                        || object.action.name.starts_with("Scale")
                })
            },
        );

        for _ in 0..8 {
            if app_clonk_carries(app, clonk, "GOLD") {
                return;
            }

            let carried_action = app
                .engine
                .object_snapshot(clonk)
                .expect("CLNK survives carried blast debris")
                .action
                .name;
            if !app
                .engine
                .object_snapshot(clonk)
                .expect("CLNK survives carried blast debris")
                .contents
                .is_empty()
                && carried_action != "Walk"
            {
                if carried_action == "Hangle" {
                    AppVirtualKeyboard::new(app)
                        .tap(VirtualKeyCode::KeyX)
                        .expect("physical X drops debris-carrying CLNK from Hangle");
                } else if carried_action.starts_with("Scale") {
                    let direction = app
                        .engine
                        .object_snapshot(clonk)
                        .expect("debris-carrying CLNK survives Scale")
                        .direction;
                    let detach = if direction == Direction::Left {
                        VirtualKeyCode::KeyC
                    } else {
                        VirtualKeyCode::KeyZ
                    };
                    AppVirtualKeyboard::new(app)
                        .tap(detach)
                        .expect("physical direction drops debris-carrying CLNK from Scale");
                }
                advance_app_until(app, "debris-carrying CLNK lands before throw", 100, |app| {
                    app.engine
                        .object_snapshot(clonk)
                        .is_some_and(|object| object.action.name == "Walk")
                });
                continue;
            }

            if app
                .engine
                .object_snapshot(clonk)
                .is_some_and(|object| !object.contents.is_empty())
            {
                let start_x = app
                    .engine
                    .object_snapshot(clonk)
                    .expect("CLNK survives incidental debris pickup")
                    .position
                    .x;
                let nearest_gold_x = app
                    .engine
                    .snapshot()
                    .objects
                    .into_iter()
                    .filter(|object| object.definition_id == "GOLD" && object.container.is_none())
                    .min_by_key(|object| (object.position.x - start_x).abs())
                    .expect("exposed GOLD remains after incidental debris pickup")
                    .position
                    .x;
                let (toward, toward_direction) = if nearest_gold_x < start_x {
                    (VirtualKeyCode::KeyZ, Direction::Left)
                } else {
                    (VirtualKeyCode::KeyC, Direction::Right)
                };
                let (away, away_direction) = if toward == VirtualKeyCode::KeyZ {
                    (VirtualKeyCode::KeyC, Direction::Right)
                } else {
                    (VirtualKeyCode::KeyZ, Direction::Left)
                };

                hold_app_key_until(
                    app,
                    away,
                    "CLNK faces away from GOLD before throwing debris",
                    20,
                    |app| {
                        app.engine
                            .object_snapshot(clonk)
                            .is_some_and(|object| object.direction == away_direction)
                    },
                );
                advance_app_until(app, "CLNK stops before throwing blast debris", 80, |app| {
                    app.engine.object_snapshot(clonk).is_some_and(|object| {
                        object.action.name == "Walk" && object.velocity.x == 0
                    })
                });
                AppVirtualKeyboard::new(app)
                    .tap(VirtualKeyCode::KeyA)
                    .expect("physical A throws incidental blast debris away");
                advance_app_until(app, "CLNK throws incidental blast debris", 30, |app| {
                    app.engine
                        .object_snapshot(clonk)
                        .is_some_and(|object| object.contents.is_empty())
                });
                hold_app_key_until(
                    app,
                    toward,
                    "CLNK leaves thrown debris and moves toward GOLD",
                    60,
                    |app| {
                        app_clonk_carries(app, clonk, "GOLD")
                            || app.engine.object_snapshot(clonk).is_some_and(|object| {
                                object.action.name == "Hangle"
                                    || object.action.name.starts_with("Scale")
                                    || if toward_direction == Direction::Left {
                                        object.position.x <= start_x - 12
                                    } else {
                                        object.position.x >= start_x + 12
                                    }
                            })
                    },
                );
                if app_clonk_carries(app, clonk, "GOLD") {
                    return;
                }
            }

            let clonk_now = app
                .engine
                .object_snapshot(clonk)
                .expect("CLNK survives GOLD collection route");
            let action = clonk_now.action.name;
            if action == "Hangle" {
                AppVirtualKeyboard::new(app)
                    .tap(VirtualKeyCode::KeyX)
                    .expect("physical X drops CLNK from the blast-pocket ceiling");
                advance_app_until(
                    app,
                    "CLNK drops from Hangle in the blast pocket",
                    100,
                    |app| {
                        app.engine.object_snapshot(clonk).is_some_and(|object| {
                            object.action.name == "Walk" || object.action.name.starts_with("Scale")
                        })
                    },
                );
                continue;
            }
            if action.starts_with("Scale") {
                let let_go = if clonk_now.direction == Direction::Left {
                    VirtualKeyCode::KeyC
                } else {
                    VirtualKeyCode::KeyZ
                };
                let scale_position = clonk_now.position;
                for _ in 0..12 {
                    app.update()
                        .expect("wait out double-click before releasing the blast wall");
                }
                AppVirtualKeyboard::new(app)
                    .press(let_go)
                    .expect("physical direction lets CLNK go of the blast wall");
                advance_app_until(app, "CLNK lets go into the blast pocket", 120, |app| {
                    app_clonk_carries(app, clonk, "GOLD")
                        || app.engine.object_snapshot(clonk).is_some_and(|object| {
                            object.action.name == "Walk"
                                || object.action.name == "Hangle"
                                || (object.action.name.starts_with("Scale")
                                    && object.position != scale_position)
                        })
                });
                AppVirtualKeyboard::new(app)
                    .release(let_go)
                    .expect("release physical direction after leaving the blast wall");
                continue;
            }
            if action != "Walk" {
                advance_app_until(app, "CLNK settles in the blast pocket", 100, |app| {
                    app.engine.object_snapshot(clonk).is_some_and(|object| {
                        object.action.name == "Walk"
                            || object.action.name == "Hangle"
                            || object.action.name.starts_with("Scale")
                    })
                });
                continue;
            }

            let target = app
                .engine
                .snapshot()
                .objects
                .into_iter()
                .filter(|object| object.definition_id == "GOLD" && object.container.is_none())
                .min_by_key(|object| {
                    (object.position.x - clonk_now.position.x).abs()
                        + (object.position.y - clonk_now.position.y).abs()
                })
                .expect("exposed GOLD remains in the blast pocket");
            let toward = if target.position.x < clonk_now.position.x {
                VirtualKeyCode::KeyZ
            } else {
                VirtualKeyCode::KeyC
            };
            hold_app_key_until(
                app,
                toward,
                "CLNK advances toward exposed GOLD",
                220,
                |app| {
                    app_clonk_carries(app, clonk, "GOLD")
                        || app.engine.object_snapshot(clonk).is_some_and(|object| {
                            object.action.name == "Hangle"
                                || object.action.name.starts_with("Scale")
                                || !object.contents.is_empty()
                        })
                },
            );
        }

        assert!(
            app_clonk_carries(app, clonk, "GOLD"),
            "physical-key blast-pocket route must collect one GOLD; clonk={:?}",
            app.engine.object_snapshot(clonk)
        );
    }

    fn app_tutorial07_climb_right_out_of_blast_pocket(
        app: &mut GameApp,
        clonk: ObjectId,
        target_x: i32,
        milestone: &str,
    ) {
        AppVirtualKeyboard::new(app)
            .press(VirtualKeyCode::KeyC)
            .unwrap_or_else(|error| panic!("press physical C for {milestone}: {error}"));
        let mut previous_action = String::new();
        for _ in 0..300 {
            let clonk_now = app
                .engine
                .object_snapshot(clonk)
                .unwrap_or_else(|| panic!("CLNK survives {milestone}"));
            if clonk_now.position.x >= target_x {
                break;
            }
            let action = clonk_now.action.name;
            let entered_scale =
                action.starts_with("Scale") && !previous_action.starts_with("Scale");
            let left_scale_in_flight = action == "Jump" && previous_action.starts_with("Scale");
            let landed = action == "Walk" && previous_action != "Walk";
            if entered_scale {
                let mut keyboard = AppVirtualKeyboard::new(app);
                keyboard
                    .release(VirtualKeyCode::KeyC)
                    .expect("release physical C on blast-pocket Scale");
                keyboard
                    .press(VirtualKeyCode::KeyC)
                    .expect("repress physical C on blast-pocket Scale");
            } else if landed || left_scale_in_flight {
                AppVirtualKeyboard::new(app)
                    .tap(VirtualKeyCode::KeyS)
                    .expect("physical S jumps out of the blast pocket");
            }
            previous_action = action;
            app.update()
                .unwrap_or_else(|error| panic!("advance {milestone}: {error}"));
        }
        AppVirtualKeyboard::new(app)
            .release(VirtualKeyCode::KeyC)
            .unwrap_or_else(|error| panic!("release physical C after {milestone}: {error}"));
        assert!(
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.x >= target_x),
            "{milestone}; clonk={:?}",
            app.engine.object_snapshot(clonk)
        );
    }

    fn app_tutorial07_return_to_hut(
        app: &mut GameApp,
        clonk: ObjectId,
        elevator_case: ObjectId,
        hut: ObjectId,
        settle_in_shaft: bool,
        subject: &str,
        target_wealth: Option<i32>,
    ) {
        app_tutorial07_climb_right_out_of_blast_pocket(
            app,
            clonk,
            105,
            &format!("{subject} climbs out of the blast pocket"),
        );
        if settle_in_shaft {
            let mut previous_action = String::new();
            for _ in 0..160 {
                let action = app
                    .engine
                    .object_snapshot(clonk)
                    .expect("returning Tutorial07 CLNK survives the shaft landing")
                    .action
                    .name;
                if action == "Walk" {
                    break;
                }
                if action == "Hangle" && previous_action != "Hangle" {
                    // Physical X is COM_Down. Jump'n'Run's DFA_HANGLE arm
                    // sends it directly through ObjectComLetGo. The cave has
                    // two ceilings, so release each newly entered Hangle
                    // before the final objective return's elevator alignment.
                    AppVirtualKeyboard::new(app)
                        .press(VirtualKeyCode::KeyX)
                        .expect("press physical X to release returning CLNK from Hangle");
                    for _ in 0..4 {
                        app.update()
                            .expect("apply held physical X to returning Hangle CLNK");
                        if app
                            .engine
                            .object_snapshot(clonk)
                            .is_none_or(|object| object.action.name != "Hangle")
                        {
                            break;
                        }
                    }
                    AppVirtualKeyboard::new(app)
                        .release(VirtualKeyCode::KeyX)
                        .expect("release physical X after returning CLNK lets go");
                    previous_action = action;
                    continue;
                }
                previous_action = action;
                app.update()
                    .expect("settle returning Tutorial07 CLNK in elevator shaft");
            }
            assert!(
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.action.name == "Walk"),
                "{subject} lets go and lands in the elevator shaft; clonk={:?}",
                app.engine.object_snapshot(clonk)
            );
        }
        for _ in 0..11 {
            app.update().expect("wait out blast-pocket key buffer");
        }
        let descent_key = app
            .engine
            .object_snapshot(clonk)
            .zip(app.engine.object_snapshot(elevator_case))
            .map(|(clonk, elevator)| {
                if clonk.position.x <= elevator.position.x {
                    VirtualKeyCode::KeyC
                } else {
                    VirtualKeyCode::KeyZ
                }
            })
            .expect("CLNK and ELEC survive the blast-pocket descent");
        hold_app_key_until(
            app,
            descent_key,
            &format!("{subject} aligns above ELEC"),
            160,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .zip(app.engine.object_snapshot(elevator_case))
                    .is_some_and(|(clonk, elevator)| {
                        clonk.action.name == "Walk"
                            && (clonk.position.x - elevator.position.x).abs() <= 25
                    })
            },
        );
        // ELEC's C++ FindWaitingClonk accepts this WALK Clonk only after the
        // released horizontal control has set its ComDir to Stop.
        advance_app_until(
            app,
            &format!("{subject} descends beside ELEC"),
            160,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .zip(app.engine.object_snapshot(elevator_case))
                    .is_some_and(|(clonk, elevator)| {
                        clonk.action.name == "Walk"
                            && (clonk.position.y - elevator.position.y).abs() <= 20
                    })
            },
        );
        let align_key = app
            .engine
            .object_snapshot(clonk)
            .zip(app.engine.object_snapshot(elevator_case))
            .map(|(clonk, elevator)| {
                if clonk.position.x < elevator.position.x {
                    VirtualKeyCode::KeyC
                } else {
                    VirtualKeyCode::KeyZ
                }
            })
            .expect("CLNK and ELEC survive the HUT3 return");
        hold_app_key_until(
            app,
            align_key,
            &format!("{subject} aligns with ELEC"),
            80,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .zip(app.engine.object_snapshot(elevator_case))
                    .is_some_and(|(clonk, elevator)| {
                        (clonk.position.x - elevator.position.x).abs() <= 5
                    })
            },
        );
        advance_app_until(app, &format!("{subject} lands beside ELEC"), 80, |app| {
            app.engine
                .object_snapshot(clonk)
                .zip(app.engine.object_snapshot(elevator_case))
                .is_some_and(|(clonk, elevator)| {
                    (clonk.position.x - elevator.position.x).abs() <= 5
                })
        });
        for _ in 0..12 {
            app.update()
                .expect("wait out Tutorial07 ELEC grab double-click buffer");
        }
        {
            let mut keyboard = AppVirtualKeyboard::new(app);
            keyboard
                .tap(VirtualKeyCode::KeyX)
                .expect("first physical X primes fresh ELEC grab");
            keyboard
                .tap(VirtualKeyCode::KeyX)
                .expect("second physical X grabs ELEC");
        }
        advance_app_until(app, &format!("{subject} grabs ELEC"), 60, |app| {
            app.engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Push" && object.action.target == Some(elevator_case)
            })
        });
        let elevator_start_y = app
            .engine
            .object_snapshot(elevator_case)
            .expect("ELEC survives before its surface ascent")
            .position
            .y;
        hold_app_key_until(
            app,
            VirtualKeyCode::KeyS,
            &format!("ELEC raises {subject} to the surface"),
            360,
            |app| {
                app.engine
                    .object_snapshot(elevator_case)
                    // Wait for SetMoveTo(RangeTop) to halt naturally. The
                    // Clonk y<=205 proxy releases Up before ELEC's C++ top
                    // stop and leaves the surface bridge open.
                    .is_some_and(|object| {
                        object.position.y < elevator_start_y && object.action.name == "Wait"
                    })
            },
        );
        {
            let mut keyboard = AppVirtualKeyboard::new(app);
            keyboard
                .tap(VirtualKeyCode::KeyX)
                .expect("first physical X primes surface ELEC release");
            keyboard
                .tap(VirtualKeyCode::KeyX)
                .expect("second physical X releases ELEC at surface");
        }
        advance_app_until(app, "CLNK releases ELEC at the surface", 60, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk")
        });

        AppVirtualKeyboard::new(app)
            .press(VirtualKeyCode::KeyZ)
            .expect("physical Z starts the HUT3 surface crossing");
        let mut previous_action = String::new();
        for _ in 0..300 {
            let clonk_now = app
                .engine
                .object_snapshot(clonk)
                .expect("returning Tutorial07 CLNK survives the surface lip");
            if clonk_now.position.x <= 70 {
                break;
            }
            let action = clonk_now.action.name;
            let entered_scale =
                action.starts_with("Scale") && !previous_action.starts_with("Scale");
            let left_scale_in_flight = action == "Jump" && previous_action.starts_with("Scale");
            let landed = action == "Walk" && previous_action != "Walk";
            if entered_scale {
                let mut keyboard = AppVirtualKeyboard::new(app);
                keyboard
                    .release(VirtualKeyCode::KeyZ)
                    .expect("release physical Z on HUT3 surface Scale");
                keyboard
                    .press(VirtualKeyCode::KeyZ)
                    .expect("repress physical Z on HUT3 surface Scale");
            } else if landed || left_scale_in_flight {
                AppVirtualKeyboard::new(app)
                    .tap(VirtualKeyCode::KeyS)
                    .expect("physical S jumps the HUT3 surface lip");
            }
            previous_action = action;
            app.update().expect("advance HUT3 surface crossing");
        }
        AppVirtualKeyboard::new(app)
            .release(VirtualKeyCode::KeyZ)
            .expect("release physical Z beside HUT3");
        assert!(
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.x <= 70),
            "{subject} crosses the surface lip"
        );
        advance_app_until(app, &format!("{subject} lands beside HUT3"), 120, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk")
        });
        hold_app_key_until(
            app,
            VirtualKeyCode::KeyC,
            &format!("{subject} steps into HUT3 entrance"),
            80,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.position.x >= 62)
            },
        );
        AppVirtualKeyboard::new(app)
            .tap(VirtualKeyCode::KeyS)
            .expect("physical S enters Tutorial07 HUT3");
        advance_app_until(app, &format!("{subject} enters HUT3"), 60, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.container == Some(hut))
        });
        if let Some(target_wealth) = target_wealth {
            advance_app_until(
                app,
                &format!("HUT3 auto-sells GOLD to wealth {target_wealth}"),
                80,
                |app| {
                    app.engine
                        .player(app.local_owner)
                        .is_some_and(|player| player.wealth() >= target_wealth)
                },
            );
        }
    }

    fn app_tutorial07_exit_hut_and_descend_to_gold(
        app: &mut GameApp,
        clonk: ObjectId,
        elevator_case: ObjectId,
        hut: ObjectId,
    ) {
        let context_identification = serde_json::from_value(serde_json::json!({ "Int": 14 }))
            .expect("context identification deserializes");
        advance_app_until(app, "HUT3 restores its context menu", 30, |app| {
            app.engine
                .cursor_object_menu(app.local_owner)
                .is_some_and(|(_, menu)| menu.identification == context_identification)
        });
        app_navigate_object_menu_to(app, "Exit", |item| item.caption == "Exit");
        AppVirtualKeyboard::new(app)
            .tap(VirtualKeyCode::KeyA)
            .expect("physical A exits Tutorial07 HUT3");
        advance_app_until(app, "empty CLNK exits Tutorial07 HUT3", 60, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.container != Some(hut))
        });

        AppVirtualKeyboard::new(app)
            .press(VirtualKeyCode::KeyC)
            .expect("physical C starts crossing the empty surface lip");
        let mut previous_action = app
            .engine
            .object_snapshot(clonk)
            .expect("empty Tutorial07 CLNK survives HUT3 exit")
            .action
            .name;
        for _ in 0..300 {
            let clonk_now = app
                .engine
                .object_snapshot(clonk)
                .expect("empty Tutorial07 CLNK survives the shaft lip");
            if clonk_now.position.x >= 105 {
                break;
            }
            let action = clonk_now.action.name;
            let entered_scale =
                action.starts_with("Scale") && !previous_action.starts_with("Scale");
            let left_scale_in_flight = action == "Jump" && previous_action.starts_with("Scale");
            let landed = action == "Walk" && previous_action != "Walk";
            if entered_scale {
                let mut keyboard = AppVirtualKeyboard::new(app);
                keyboard
                    .release(VirtualKeyCode::KeyC)
                    .expect("release physical C on empty surface Scale");
                keyboard
                    .press(VirtualKeyCode::KeyC)
                    .expect("repress physical C on empty surface Scale");
            } else if landed || left_scale_in_flight {
                AppVirtualKeyboard::new(app)
                    .tap(VirtualKeyCode::KeyS)
                    .expect("physical S jumps the empty surface lip");
            }
            previous_action = action;
            app.update().expect("advance empty surface crossing");
        }
        AppVirtualKeyboard::new(app)
            .release(VirtualKeyCode::KeyC)
            .expect("release physical C beyond the empty surface lip");
        assert!(app
            .engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.position.x >= 105));
        for _ in 0..11 {
            app.update().expect("wait out surface key buffer");
        }
        hold_app_key_until(
            app,
            VirtualKeyCode::KeyX,
            "empty CLNK descends beside ELEC",
            160,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .zip(app.engine.object_snapshot(elevator_case))
                    .is_some_and(|(clonk, elevator)| {
                        clonk.action.name == "Walk"
                            && (clonk.position.y - elevator.position.y).abs() <= 20
                    })
            },
        );
        hold_app_key_until(
            app,
            VirtualKeyCode::KeyZ,
            "empty CLNK aligns with ELEC",
            80,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .zip(app.engine.object_snapshot(elevator_case))
                    .is_some_and(|(clonk, elevator)| {
                        (clonk.position.x - elevator.position.x).abs() <= 5
                    })
            },
        );
        for _ in 0..12 {
            app.update()
                .expect("wait out empty Tutorial07 ELEC grab buffer");
        }
        {
            let mut keyboard = AppVirtualKeyboard::new(app);
            keyboard
                .tap(VirtualKeyCode::KeyX)
                .expect("first physical X primes empty ELEC grab");
            keyboard
                .tap(VirtualKeyCode::KeyX)
                .expect("second physical X grabs ELEC for another descent");
        }
        advance_app_until(app, "empty CLNK grabs ELEC", 60, |app| {
            app.engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Push" && object.action.target == Some(elevator_case)
            })
        });
        hold_app_key_until(
            app,
            VirtualKeyCode::KeyD,
            "ELEC lowers empty CLNK to the GOLD seam",
            360,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.position.y >= 325)
            },
        );
        {
            let mut keyboard = AppVirtualKeyboard::new(app);
            keyboard
                .tap(VirtualKeyCode::KeyX)
                .expect("first physical X primes underground release");
            keyboard
                .tap(VirtualKeyCode::KeyX)
                .expect("second physical X releases ELEC underground");
        }
        advance_app_until(app, "empty CLNK releases ELEC underground", 60, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk")
        });
    }

    fn app_climb_from_tutorial04_gold_pocket(
        app: &mut GameApp,
        clonk: ObjectId,
        deepened_pocket: bool,
    ) {
        if app.engine.object_snapshot(clonk).is_some_and(|object| {
            (!deepened_pocket || object.position.x >= 455)
                && object.position.y <= 403
                && !object.action.name.starts_with("Scale")
        }) {
            return;
        }

        if app
            .engine
            .object_snapshot(clonk)
            .is_some_and(|object| !object.action.name.starts_with("Scale"))
        {
            AppVirtualKeyboard::new(app)
                .press(VirtualKeyCode::KeyC)
                .expect("physical C carries GOLD to the tunnel wall");
            advance_app_until(
                app,
                "GOLD-carrying CLNK reaches the tunnel wall",
                360,
                |app| {
                    app.engine
                        .object_snapshot(clonk)
                        .is_some_and(|object| object.action.name.starts_with("Scale"))
                },
            );
            {
                let mut keyboard = AppVirtualKeyboard::new(app);
                keyboard
                    .press(VirtualKeyCode::KeyS)
                    .expect("physical S starts climbing the GOLD tunnel wall");
                keyboard
                    .release(VirtualKeyCode::KeyC)
                    .expect("release physical C while climbing the GOLD tunnel wall");
            }
            app.update()
                .expect("advance the GOLD tunnel-wall control transition");
            {
                let mut keyboard = AppVirtualKeyboard::new(app);
                keyboard
                    .release(VirtualKeyCode::KeyS)
                    .expect("release physical S to refresh the GOLD wall climb");
                keyboard
                    .press(VirtualKeyCode::KeyS)
                    .expect("repress physical S to climb the GOLD tunnel wall");
            }
        } else {
            AppVirtualKeyboard::new(app)
                .press(VirtualKeyCode::KeyS)
                .expect("physical S starts the GOLD tunnel-wall climb");
        }
        advance_app_until(
            app,
            "GOLD-carrying CLNK scales to the tunnel throat",
            360,
            |app| {
                app.engine.object_snapshot(clonk).is_some_and(|object| {
                    (object.position.y <= 403 && object.action.name.starts_with("Scale"))
                        || (object.position.y <= 418 && object.action.name == "Walk")
                })
            },
        );
        AppVirtualKeyboard::new(app)
            .release(VirtualKeyCode::KeyS)
            .expect("release physical S at the GOLD tunnel throat");
        let throat = app
            .engine
            .object_snapshot(clonk)
            .expect("GOLD-carrying CLNK reaches the tunnel throat");
        let away = if throat.direction == Direction::Left {
            VirtualKeyCode::KeyC
        } else {
            VirtualKeyCode::KeyZ
        };
        {
            let mut keyboard = AppVirtualKeyboard::new(app);
            keyboard
                .press(VirtualKeyCode::KeyS)
                .expect("physical S holds at the GOLD tunnel throat");
            keyboard
                .tap(away)
                .expect("physical direction releases the GOLD tunnel throat");
        }
        advance_app_until(
            app,
            "GOLD-carrying CLNK leaves the tunnel throat",
            120,
            |app| {
                app.engine.object_snapshot(clonk).is_some_and(|object| {
                    object.action.name == "Hangle" || object.action.name == "Walk"
                })
            },
        );
        AppVirtualKeyboard::new(app)
            .release(VirtualKeyCode::KeyS)
            .expect("release physical S beyond the GOLD tunnel throat");

        if app
            .engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Hangle")
        {
            hold_app_key_until(
                app,
                VirtualKeyCode::KeyC,
                "GOLD-carrying CLNK traverses the upper tunnel ceiling",
                120,
                |app| {
                    app.engine.object_snapshot(clonk).is_some_and(|object| {
                        object.position.x >= 465 || object.action.name != "Hangle"
                    })
                },
            );
            if app
                .engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Hangle")
            {
                AppVirtualKeyboard::new(app)
                    .tap(VirtualKeyCode::KeyX)
                    .expect("physical X drops the GOLD carrier from the upper ceiling");
            }
        }

        if app
            .engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.position.y > 403)
        {
            if deepened_pocket {
                for _ in 0..12 {
                    app.update()
                        .expect("wait out the deep GOLD-pocket key buffer");
                }
                {
                    let mut keyboard = AppVirtualKeyboard::new(app);
                    keyboard
                        .tap(VirtualKeyCode::KeyZ)
                        .expect("physical Z launches from the deep GOLD shoulder");
                    keyboard
                        .tap(VirtualKeyCode::KeyX)
                        .expect("physical X clears the deep GOLD shoulder contact");
                    keyboard
                        .press(VirtualKeyCode::KeyS)
                        .expect("physical S reaches the diagonal GOLD tunnel roof");
                }
                advance_app_until(
                    app,
                    "GOLD-carrying CLNK reaches the diagonal tunnel roof",
                    60,
                    |app| {
                        app.engine.object_snapshot(clonk).is_some_and(|object| {
                            object.action.name == "Hangle"
                                || (object.action.name == "Jump" && object.position.y <= 410)
                                || (object.position.x >= 455 && object.position.y <= 403)
                        })
                    },
                );
                AppVirtualKeyboard::new(app)
                    .release(VirtualKeyCode::KeyS)
                    .expect("release physical S at the diagonal GOLD tunnel roof");
                hold_app_key_until(
                    app,
                    VirtualKeyCode::KeyC,
                    "GOLD-carrying CLNK traverses the diagonal tunnel roof",
                    180,
                    |app| {
                        app.engine.object_snapshot(clonk).is_some_and(|object| {
                            object.position.x >= 455 && object.position.y <= 403
                        })
                    },
                );
            } else {
                AppVirtualKeyboard::new(app)
                    .press(VirtualKeyCode::KeyC)
                    .expect("physical C starts the original GOLD-shoulder climb");
                let mut right_held = true;
                let mut previous_action = String::new();
                for _ in 0..360 {
                    let clonk_now = app
                        .engine
                        .object_snapshot(clonk)
                        .expect("GOLD-carrying CLNK survives the original shoulder");
                    let action = clonk_now.action.name;
                    if clonk_now.position.y <= 403 && !action.starts_with("Scale") {
                        break;
                    }
                    let entered_scale =
                        action.starts_with("Scale") && !previous_action.starts_with("Scale");
                    let left_scale_in_flight =
                        action == "Jump" && previous_action.starts_with("Scale");
                    let landed = action == "Walk" && previous_action != "Walk";
                    if entered_scale {
                        if right_held {
                            AppVirtualKeyboard::new(app)
                                .release(VirtualKeyCode::KeyC)
                                .expect("release physical C on the original GOLD shoulder");
                            right_held = false;
                        }
                        let away = if clonk_now.direction == Direction::Left {
                            VirtualKeyCode::KeyC
                        } else {
                            VirtualKeyCode::KeyZ
                        };
                        AppVirtualKeyboard::new(app)
                            .tap(away)
                            .expect("physical direction releases the original GOLD shoulder");
                    } else if landed || left_scale_in_flight {
                        if !right_held {
                            AppVirtualKeyboard::new(app)
                                .press(VirtualKeyCode::KeyC)
                                .expect("physical C resumes the original GOLD-shoulder climb");
                            right_held = true;
                        }
                        AppVirtualKeyboard::new(app)
                            .tap(VirtualKeyCode::KeyS)
                            .expect("physical S jumps the original GOLD shoulder");
                    } else if action == "Hangle" && previous_action != "Hangle" {
                        AppVirtualKeyboard::new(app)
                            .tap(VirtualKeyCode::KeyX)
                            .expect("physical X drops from the original GOLD shoulder ceiling");
                    }
                    previous_action = action;
                    app.update()
                        .expect("advance the original GOLD-shoulder climb");
                }
                if right_held {
                    AppVirtualKeyboard::new(app)
                        .release(VirtualKeyCode::KeyC)
                        .expect("release physical C beyond the original GOLD shoulder");
                }
            }
        }
        assert!(
            app.engine.object_snapshot(clonk).is_some_and(|object| {
                (!deepened_pocket || object.position.x >= 455)
                    && object.position.y <= 403
                    && !object.action.name.starts_with("Scale")
            }),
            "GOLD-carrying CLNK climbs back to the upper tunnel; clonk={:?}",
            app.engine.object_snapshot(clonk)
        );
    }

    fn app_carry_tutorial04_gold_to_hut(
        app: &mut GameApp,
        clonk: ObjectId,
        elevator_case: ObjectId,
        hut: ObjectId,
        target_wealth: i32,
    ) {
        let carried_gold = app
            .engine
            .object_snapshot(clonk)
            .expect("Tutorial04 CLNK survives before a GOLD sale trip")
            .contents
            .into_iter()
            .find(|item| {
                app.engine
                    .object_snapshot(*item)
                    .is_some_and(|item| item.definition_id == "GOLD")
            })
            .expect("Tutorial04 CLNK carries one exact GOLD object to sell");

        app_climb_from_tutorial04_gold_pocket(app, clonk, target_wealth > 10);
        hold_app_key_until(
            app,
            VirtualKeyCode::KeyC,
            "GOLD-carrying CLNK returns to ELEC",
            180,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .zip(app.engine.object_snapshot(elevator_case))
                    .is_some_and(|(clonk, elevator)| {
                        clonk.action.name == "Walk"
                            && (clonk.position.x - elevator.position.x).abs() <= 5
                    })
            },
        );

        AppVirtualKeyboard::new(app)
            .tap(VirtualKeyCode::KeyX)
            .expect("physical X grabs ELEC for a GOLD sale trip");
        advance_app_until(app, "GOLD sale trip grabs ELEC", 60, |app| {
            app.engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Push" && object.action.target == Some(elevator_case)
            })
        });
        hold_app_key_until(
            app,
            VirtualKeyCode::KeyS,
            "ELEC raises a GOLD sale trip",
            300,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.position.y <= 270)
            },
        );
        {
            let mut keyboard = AppVirtualKeyboard::new(app);
            keyboard
                .tap(VirtualKeyCode::KeyX)
                .expect("first physical X releases ELEC on a GOLD trip");
            keyboard
                .tap(VirtualKeyCode::KeyX)
                .expect("second physical X releases ELEC on a GOLD trip");
        }
        advance_app_until(app, "GOLD sale trip lets go of ELEC", 60, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name != "Push")
        });

        // Cross the raw shaft lip with the same physical Right/Jump sequence
        // used by the first GOLD trip (C4Object.cpp:3618-3628,4284-4299,
        // 4823-4855).
        AppVirtualKeyboard::new(app)
            .press(VirtualKeyCode::KeyC)
            .expect("physical C starts crossing the surface shaft lip");
        let mut previous_action = String::new();
        for _ in 0..240 {
            let clonk_now = app
                .engine
                .object_snapshot(clonk)
                .expect("GOLD-carrying CLNK survives the surface shaft lip");
            if clonk_now.position.x >= 558 {
                break;
            }
            let action = clonk_now.action.name;
            let entered_scale =
                action.starts_with("Scale") && !previous_action.starts_with("Scale");
            let left_scale_in_flight = action == "Jump" && previous_action.starts_with("Scale");
            let landed = action == "Walk" && previous_action != "Walk";
            if entered_scale {
                let mut keyboard = AppVirtualKeyboard::new(app);
                keyboard
                    .release(VirtualKeyCode::KeyC)
                    .expect("release physical C on a shaft-lip Scale");
                keyboard
                    .press(VirtualKeyCode::KeyC)
                    .expect("repress physical C on a shaft-lip Scale");
            } else if landed || left_scale_in_flight {
                AppVirtualKeyboard::new(app)
                    .tap(VirtualKeyCode::KeyS)
                    .expect("physical S jumps the Tutorial04 shaft lip");
            }
            previous_action = action;
            app.update()
                .expect("advance a GOLD trip over the shaft lip");
        }
        AppVirtualKeyboard::new(app)
            .release(VirtualKeyCode::KeyC)
            .expect("release physical C on HUT2's hill");
        assert!(
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.x >= 558),
            "{target_wealth}-wealth GOLD trip must reach HUT2's hill"
        );
        advance_app_until(app, "GOLD sale trip lands beside HUT2", 80, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk")
        });
        hold_app_key_until(
            app,
            VirtualKeyCode::KeyZ,
            "GOLD sale trip aligns with HUT2",
            80,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.position.x <= 570)
            },
        );
        AppVirtualKeyboard::new(app)
            .tap(VirtualKeyCode::KeyS)
            .expect("physical S enters HUT2 with one GOLD");
        advance_app_until(app, "GOLD sale trip enters HUT2", 60, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.container == Some(hut))
        });
        advance_app_until(app, "HUT2 auto-sells one exact GOLD", 80, |app| {
            app.engine
                .player(app.local_owner)
                .is_some_and(|player| player.wealth() >= target_wealth)
                && app.engine.object_snapshot(carried_gold).is_none()
        });
        assert_eq!(
            app.engine
                .player(app.local_owner)
                .expect("Tutorial04 player survives a GOLD sale")
                .wealth(),
            target_wealth,
            "each physical HUT2 trip must sell exactly one value-five GOLD"
        );
        assert!(
            app.engine.object_snapshot(carried_gold).is_none(),
            "HUT2 must remove the exact GOLD object sold at {target_wealth} wealth"
        );
    }

    fn app_return_tutorial04_from_hut_to_tunnel(
        app: &mut GameApp,
        clonk: ObjectId,
        elevator_case: ObjectId,
        hut: ObjectId,
    ) {
        let context_identification = serde_json::from_value(serde_json::json!({ "Int": 14 }))
            .expect("context identification deserializes");
        advance_app_until(app, "HUT2 restores context after selling GOLD", 30, |app| {
            app.engine
                .cursor_object_menu(app.local_owner)
                .is_some_and(|(_, menu)| menu.identification == context_identification)
        });
        let menu_rows = app
            .engine
            .cursor_object_menu(app.local_owner)
            .expect("HUT2 context exists after a GOLD sale")
            .1
            .items
            .len();
        for _ in 0..menu_rows {
            if app_selected_object_menu_item(app).is_some_and(|item| item.caption == "Exit") {
                break;
            }
            AppVirtualKeyboard::new(app)
                .tap(VirtualKeyCode::KeyX)
                .expect("physical X advances HUT2 context to Exit");
        }
        assert_eq!(
            app_selected_object_menu_item(app).map(|item| item.caption.as_str()),
            Some("Exit"),
            "physical context navigation must select HUT2 Exit"
        );
        AppVirtualKeyboard::new(app)
            .tap(VirtualKeyCode::KeyA)
            .expect("physical A exits HUT2 for another GOLD trip");
        advance_app_until(app, "empty CLNK exits HUT2", 60, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.container != Some(hut))
        });

        hold_app_key_until(
            app,
            VirtualKeyCode::KeyZ,
            "empty CLNK returns to the surface shaft",
            180,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .zip(app.engine.object_snapshot(elevator_case))
                    .is_some_and(|(clonk, elevator)| {
                        (clonk.position.x - elevator.position.x).abs() <= 5
                    })
            },
        );
        if app.engine.object_snapshot(clonk).is_some_and(|object| {
            object.action.name == "Hangle" || object.action.name.starts_with("Scale")
        }) {
            hold_app_key_until(
                app,
                VirtualKeyCode::KeyX,
                "returning CLNK descends onto ELEC",
                120,
                |app| {
                    app.engine
                        .object_snapshot(clonk)
                        .is_some_and(|object| object.action.name == "Walk")
                },
            );
        }
        assert!(
            app.engine
                .object_snapshot(clonk)
                .zip(app.engine.object_snapshot(elevator_case))
                .is_some_and(|(clonk, elevator)| {
                    (clonk.position.x - elevator.position.x).abs() <= 5
                }),
            "returning CLNK stands beside ELEC; clonk={:?}, elevator={:?}",
            app.engine.object_snapshot(clonk),
            app.engine.object_snapshot(elevator_case)
        );
        for _ in 0..12 {
            app.update()
                .expect("wait out returning Tutorial04 ELEC grab buffer");
        }
        {
            let mut keyboard = AppVirtualKeyboard::new(app);
            keyboard
                .tap(VirtualKeyCode::KeyX)
                .expect("first physical X primes returning ELEC grab");
            keyboard
                .tap(VirtualKeyCode::KeyX)
                .expect("second physical X grabs ELEC for another GOLD trip");
        }
        for _ in 0..60 {
            if app.engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Push" && object.action.target == Some(elevator_case)
            }) {
                break;
            }
            if app
                .engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name.starts_with("Scale"))
            {
                break;
            }
            app.update().expect("grab ELEC for another GOLD trip");
        }
        if app
            .engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name.starts_with("Scale"))
        {
            let detach = if app
                .engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.direction == Direction::Left)
            {
                VirtualKeyCode::KeyC
            } else {
                VirtualKeyCode::KeyZ
            };
            AppVirtualKeyboard::new(app)
                .tap(detach)
                .expect("physical opposite direction releases ELEC-side Scale");
            advance_app_until(app, "returning CLNK lands after Scale release", 80, |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.action.name == "Walk")
            });
            let clonk_x = app
                .engine
                .object_snapshot(clonk)
                .expect("returning CLNK survives Scale release")
                .position
                .x;
            let elevator_x = app
                .engine
                .object_snapshot(elevator_case)
                .expect("ELEC survives returning Scale release")
                .position
                .x;
            if (clonk_x - elevator_x).abs() > 8 {
                let toward_elevator = if elevator_x < clonk_x {
                    VirtualKeyCode::KeyZ
                } else {
                    VirtualKeyCode::KeyC
                };
                hold_app_key_until(
                    app,
                    toward_elevator,
                    "returning CLNK realigns with ELEC",
                    80,
                    |app| {
                        app.engine
                            .object_snapshot(clonk)
                            .zip(app.engine.object_snapshot(elevator_case))
                            .is_some_and(|(clonk, elevator)| {
                                (clonk.position.x - elevator.position.x).abs() <= 8
                            })
                    },
                );
            }
            AppVirtualKeyboard::new(app)
                .tap(VirtualKeyCode::KeyX)
                .expect("physical X grabs ELEC after Scale release");
            advance_app_until(app, "returning CLNK grabs ELEC", 60, |app| {
                app.engine.object_snapshot(clonk).is_some_and(|object| {
                    object.action.name == "Push" && object.action.target == Some(elevator_case)
                })
            });
        }
        assert!(
            app.engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Push" && object.action.target == Some(elevator_case)
            }),
            "returning CLNK grabs ELEC; clonk={:?}, elevator={:?}",
            app.engine.object_snapshot(clonk),
            app.engine.object_snapshot(elevator_case)
        );
        hold_app_key_until(
            app,
            VirtualKeyCode::KeyD,
            "ELEC carries the empty CLNK underground",
            360,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.position.y >= 340)
            },
        );
        {
            let mut keyboard = AppVirtualKeyboard::new(app);
            keyboard
                .tap(VirtualKeyCode::KeyX)
                .expect("first physical X releases ELEC underground");
            keyboard
                .tap(VirtualKeyCode::KeyX)
                .expect("second physical X releases ELEC underground");
        }
        advance_app_until(app, "empty CLNK lets go underground", 60, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk")
        });
    }

    fn app_take_tutorial04_flint_from_hut(
        app: &mut GameApp,
        clonk: ObjectId,
        hut: ObjectId,
    ) -> ObjectId {
        let context_identification = serde_json::from_value(serde_json::json!({ "Int": 14 }))
            .expect("context identification deserializes");
        let contents_identification = serde_json::from_value(serde_json::json!({ "Int": 18 }))
            .expect("contents identification deserializes");
        advance_app_until(
            app,
            "HUT2 restores context before TFLN withdrawal",
            30,
            |app| {
                app.engine
                    .cursor_object_menu(app.local_owner)
                    .is_some_and(|(_, menu)| menu.identification == context_identification)
            },
        );
        app_navigate_object_menu_to(app, "Contents", |item| item.caption == "Contents");
        AppVirtualKeyboard::new(app)
            .tap(VirtualKeyCode::KeyA)
            .expect("physical A opens HUT2 Contents for another TFLN");
        advance_app_until(app, "HUT2 TFLN Contents menu", 30, |app| {
            app.engine
                .cursor_object_menu(app.local_owner)
                .is_some_and(|(_, menu)| menu.identification == contents_identification)
        });
        app_navigate_object_menu_to(app, "TFLN", |item| item.item_id == "TFLN");
        let hut_flints_before = app_object_contents_count(app, hut, "TFLN");
        assert!(
            hut_flints_before > 0,
            "HUT2 retains another replacement TFLN"
        );
        AppVirtualKeyboard::new(app)
            .tap(VirtualKeyCode::KeyA)
            .expect("physical A takes one additional TFLN");
        advance_app_until(app, "CLNK carries one additional TFLN", 60, |app| {
            app_object_contents_count(app, clonk, "TFLN") == 1
                && app_object_contents_count(app, hut, "TFLN") + 1 == hut_flints_before
        });
        let flint = app
            .engine
            .object_snapshot(clonk)
            .expect("TFLN-carrying Tutorial04 CLNK survives")
            .contents
            .into_iter()
            .find(|item| {
                app.engine
                    .object_snapshot(*item)
                    .is_some_and(|item| item.definition_id == "TFLN")
            })
            .expect("physical Contents withdrawal preserves the exact TFLN");
        AppVirtualKeyboard::new(app)
            .tap(VirtualKeyCode::KeyD)
            .expect("physical D closes TFLN Contents");
        advance_app_until(
            app,
            "HUT2 context returns after TFLN withdrawal",
            30,
            |app| {
                app.engine
                    .cursor_object_menu(app.local_owner)
                    .is_some_and(|(_, menu)| menu.identification == context_identification)
            },
        );
        flint
    }

    fn app_recover_tutorial04_clonk(
        app: &mut GameApp,
        clonk: ObjectId,
        milestone: &str,
        max_ticks: u32,
        allow_attachment: bool,
    ) {
        let mut previous_action = String::new();
        let mut climbing = false;
        let mut drifting = false;
        for _ in 0..max_ticks {
            let clonk_now = app
                .engine
                .object_snapshot(clonk)
                .expect("Tutorial04 CLNK survives blast-pocket recovery");
            let action = clonk_now.action.name;
            if action == "Walk"
                || (allow_attachment && (action == "Hangle" || action.starts_with("Scale")))
            {
                if climbing {
                    AppVirtualKeyboard::new(app)
                        .release(VirtualKeyCode::KeyS)
                        .expect("release physical S after blast-pocket recovery");
                }
                if drifting {
                    AppVirtualKeyboard::new(app)
                        .release(VirtualKeyCode::KeyC)
                        .expect("release physical C after blast-pocket recovery");
                }
                return;
            }
            if action.starts_with("Scale") && !previous_action.starts_with("Scale") {
                if drifting {
                    AppVirtualKeyboard::new(app)
                        .release(VirtualKeyCode::KeyC)
                        .expect("release physical C before scaling out of blast pocket");
                    drifting = false;
                }
                AppVirtualKeyboard::new(app)
                    .press(VirtualKeyCode::KeyS)
                    .expect("physical S scales during blast-pocket recovery");
                climbing = true;
            } else if !action.starts_with("Scale")
                && previous_action.starts_with("Scale")
                && climbing
            {
                AppVirtualKeyboard::new(app)
                    .release(VirtualKeyCode::KeyS)
                    .expect("release physical S after Scale corner");
                climbing = false;
            } else if action == "Hangle" {
                if climbing {
                    AppVirtualKeyboard::new(app)
                        .release(VirtualKeyCode::KeyS)
                        .expect("release physical S before leaving Hangle");
                    climbing = false;
                }
                if drifting {
                    AppVirtualKeyboard::new(app)
                        .release(VirtualKeyCode::KeyC)
                        .expect("release physical C before leaving Hangle");
                }
                AppVirtualKeyboard::new(app)
                    .tap(VirtualKeyCode::KeyX)
                    .expect("physical X drops from blast-pocket Hangle");
                app.update().expect("advance one Hangle-drop frame");
                AppVirtualKeyboard::new(app)
                    .press(VirtualKeyCode::KeyC)
                    .expect("physical C drifts during blast-pocket recovery");
                drifting = true;
            }
            previous_action = action;
            app.update()
                .expect("advance Tutorial04 blast-pocket recovery");
        }
        if climbing {
            AppVirtualKeyboard::new(app)
                .release(VirtualKeyCode::KeyS)
                .expect("release physical S after failed blast-pocket recovery");
        }
        if drifting {
            AppVirtualKeyboard::new(app)
                .release(VirtualKeyCode::KeyC)
                .expect("release physical C after failed blast-pocket recovery");
        }
        assert!(
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk"),
            "{milestone}; clonk={:?}",
            app.engine.object_snapshot(clonk)
        );
    }

    fn app_tutorial04_blast_next_gold_face(
        app: &mut GameApp,
        clonk: ObjectId,
        flint: ObjectId,
        face_x: i32,
        expected_gold_growth: usize,
    ) {
        hold_app_key_until(
            app,
            VirtualKeyCode::KeyZ,
            "replacement-TFLN CLNK returns to the blast pocket",
            180,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.position.x <= 452 && object.position.y >= 365)
            },
        );
        if app.engine.object_snapshot(clonk).is_some_and(|object| {
            object.action.name != "Hangle" && !object.action.name.starts_with("Scale")
        }) {
            AppVirtualKeyboard::new(app)
                .tap(VirtualKeyCode::KeyX)
                .expect("physical X stabilizes the replacement-TFLN CLNK");
        }
        app_recover_tutorial04_clonk(
            app,
            clonk,
            "replacement-TFLN CLNK stabilizes at blast distance",
            60,
            true,
        );
        let gold_before = app
            .engine
            .snapshot()
            .objects
            .iter()
            .filter(|object| object.definition_id == "GOLD")
            .count();

        AppVirtualKeyboard::new(app)
            .press(VirtualKeyCode::KeyZ)
            .expect("physical Z approaches the replacement blast face");
        for _ in 0..120 {
            if app.engine.object_snapshot(clonk).is_some_and(|object| {
                if face_x == 414 {
                    object.position.y <= 403 && object.action.name.starts_with("Scale")
                } else {
                    object.position.x <= face_x
                }
            }) {
                break;
            }
            app.update().expect("approach the replacement blast face");
        }
        AppVirtualKeyboard::new(app)
            .release(VirtualKeyCode::KeyZ)
            .expect("release physical Z at the replacement blast face");
        if face_x == 414
            && app
                .engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.y > 403)
        {
            hold_app_key_until(
                app,
                VirtualKeyCode::KeyS,
                "replacement-TFLN CLNK climbs to the first blast ledge",
                80,
                |app| {
                    app.engine
                        .object_snapshot(clonk)
                        .is_some_and(|object| object.position.y <= 403)
                },
            );
        }

        if face_x < 414
            && app.engine.object_snapshot(clonk).is_some_and(|object| {
                object.position.x > face_x
                    && (object.action.name == "Hangle" || object.action.name.starts_with("Scale"))
            })
        {
            let attached = app
                .engine
                .object_snapshot(clonk)
                .expect("attached CLNK reaches the replacement blast face");
            let away = if attached.direction == Direction::Left {
                VirtualKeyCode::KeyC
            } else {
                VirtualKeyCode::KeyZ
            };
            AppVirtualKeyboard::new(app)
                .tap(away)
                .expect("physical direction releases the deep-blast attachment");
            app_recover_tutorial04_clonk(
                app,
                clonk,
                "replacement-TFLN CLNK lands inside the deepened pocket",
                120,
                false,
            );
        }

        if app
            .engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.position.x > face_x && object.action.name == "Walk")
        {
            for _ in 0..3 {
                app_recover_tutorial04_clonk(
                    app,
                    clonk,
                    "replacement-TFLN CLNK recovers before lip Dig",
                    80,
                    false,
                );
                AppVirtualKeyboard::new(app)
                    .tap(VirtualKeyCode::KeyD)
                    .expect("physical D starts Dig through the blast-pocket lip");
                for _ in 0..30 {
                    if app
                        .engine
                        .object_snapshot(clonk)
                        .is_some_and(|object| object.action.name == "Dig")
                    {
                        break;
                    }
                    app.update().expect("start blast-pocket lip Dig");
                }
                if app
                    .engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.action.name == "Dig")
                {
                    break;
                }
            }
            assert!(
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.action.name == "Dig"),
                "replacement-TFLN CLNK starts real lip Dig; clonk={:?}",
                app.engine.object_snapshot(clonk)
            );
            let replacement_dig_vertical = VirtualKeyCode::KeyX;
            {
                let mut keyboard = AppVirtualKeyboard::new(app);
                keyboard
                    .press(VirtualKeyCode::KeyZ)
                    .expect("physical Z steers replacement Dig left");
                keyboard
                    .press(replacement_dig_vertical)
                    .expect("physical vertical key steers replacement Dig");
            }
            for tick in 0..120 {
                if tick > 0
                    && app
                        .engine
                        .object_snapshot(clonk)
                        .is_some_and(|object| object.action.name != "Dig")
                {
                    break;
                }
                if tick == 12 {
                    let mut keyboard = AppVirtualKeyboard::new(app);
                    keyboard
                        .release(replacement_dig_vertical)
                        .expect("release physical vertical key to refresh replacement Dig");
                    keyboard
                        .press(replacement_dig_vertical)
                        .expect("repress physical vertical key for replacement Dig");
                }
                app.update()
                    .expect("advance replacement Dig to the next face");
            }
            {
                let mut keyboard = AppVirtualKeyboard::new(app);
                keyboard
                    .release(replacement_dig_vertical)
                    .expect("release physical vertical key after replacement Dig");
                keyboard
                    .release(VirtualKeyCode::KeyZ)
                    .expect("release physical Z after replacement Dig");
            }
            app_recover_tutorial04_clonk(
                app,
                clonk,
                "replacement Dig reaches the next blast face",
                60,
                false,
            );
            hold_app_key_until(
                app,
                VirtualKeyCode::KeyZ,
                "replacement-TFLN CLNK grips the deepened face",
                80,
                |app| {
                    app.engine.object_snapshot(clonk).is_some_and(|object| {
                        object.action.name == "Hangle" || object.action.name.starts_with("Scale")
                    })
                },
            );
            if face_x > 390
                && app
                    .engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.action.name.starts_with("Scale"))
            {
                for _ in 0..12 {
                    app.update()
                        .expect("wait out double-click before descending replacement face");
                }
                hold_app_key_until(
                    app,
                    VirtualKeyCode::KeyX,
                    "replacement-TFLN CLNK descends the deepened face",
                    80,
                    |app| {
                        app.engine.object_snapshot(clonk).is_some_and(|object| {
                            object.position.y >= 438 || !object.action.name.starts_with("Scale")
                        })
                    },
                );
            }
        }
        app_recover_tutorial04_clonk(
            app,
            clonk,
            "replacement-TFLN CLNK stands before the GOLD vein",
            80,
            face_x != 402,
        );
        let attached = app.engine.object_snapshot(clonk).is_some_and(|object| {
            object.action.name == "Hangle" || object.action.name.starts_with("Scale")
        });
        if !attached {
            hold_app_key_until(
                app,
                VirtualKeyCode::KeyZ,
                "replacement-TFLN CLNK faces the vein",
                30,
                |app| {
                    app.engine.object_snapshot(clonk).is_some_and(|object| {
                        object.action.name == "Walk" && object.direction == Direction::Left
                    })
                },
            );
            app.update()
                .expect("settle the left-facing replacement throw");
        }
        for _ in 0..12 {
            app.update()
                .expect("wait out replacement-TFLN double-click window");
        }
        AppVirtualKeyboard::new(app)
            .tap(VirtualKeyCode::KeyA)
            .expect("physical A throws the exact replacement TFLN");
        advance_app_until(
            app,
            "exact replacement TFLN leaves CLNK inventory",
            30,
            |app| {
                app.engine
                    .object_snapshot(flint)
                    .is_some_and(|flint| flint.container.is_none())
                    && !app_clonk_carries(app, clonk, "TFLN")
            },
        );
        if face_x == 402 && attached {
            let attached = app
                .engine
                .object_snapshot(clonk)
                .expect("attached replacement-TFLN CLNK survives its first drop");
            let away = if attached.direction == Direction::Left {
                VirtualKeyCode::KeyC
            } else {
                VirtualKeyCode::KeyZ
            };
            AppVirtualKeyboard::new(app)
                .tap(away)
                .expect("physical direction releases CLNK after the first face402 drop");
        }
        if face_x < 414 && !attached {
            let incidental = app
                .engine
                .object_snapshot(clonk)
                .and_then(|object| object.contents.first().copied());
            if let Some(incidental) = incidental {
                for _ in 0..2 {
                    AppVirtualKeyboard::new(app)
                        .tap(VirtualKeyCode::KeyC)
                        .expect("physical C clears the throw double-click buffer");
                    app.update()
                        .expect("advance the incidental-item throw facing control");
                    AppVirtualKeyboard::new(app)
                        .tap(VirtualKeyCode::KeyA)
                        .expect("physical A throws incidental blast-pocket material");
                    AppVirtualKeyboard::new(app)
                        .press(VirtualKeyCode::KeyZ)
                        .expect("physical Z leaves thrown incidental material");
                    for _ in 0..12 {
                        if app
                            .engine
                            .object_snapshot(incidental)
                            .is_none_or(|object| object.container != Some(clonk))
                        {
                            break;
                        }
                        app.update()
                            .expect("move away from thrown incidental material");
                    }
                    AppVirtualKeyboard::new(app)
                        .release(VirtualKeyCode::KeyZ)
                        .expect("release physical Z after clearing incidental material");
                    if app
                        .engine
                        .object_snapshot(incidental)
                        .is_none_or(|object| object.container != Some(clonk))
                    {
                        break;
                    }
                }
            }
        }
        if face_x < 414
            && (!attached || face_x == 402)
            && app
                .engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.contents.is_empty())
        {
            if face_x == 402 && attached {
                hold_app_key_until(
                    app,
                    VirtualKeyCode::KeyZ,
                    "CLNK returns to the lit face402 TFLN",
                    40,
                    |app| {
                        app.engine
                            .object_snapshot(flint)
                            .is_some_and(|flint| flint.container == Some(clonk))
                    },
                );
            } else {
                for _ in 0..40 {
                    if app
                        .engine
                        .object_snapshot(flint)
                        .is_some_and(|flint| flint.container == Some(clonk))
                    {
                        break;
                    }
                    app.update().expect("wait for CLNK to recatch lit TFLN");
                }
            }
            assert!(
                app.engine
                    .object_snapshot(flint)
                    .is_some_and(|flint| flint.container == Some(clonk)),
                "CLNK catches the lit replacement TFLN at the GOLD face; clonk={:?}, flint={:?}",
                app.engine.object_snapshot(clonk),
                app.engine.object_snapshot(flint)
            );
            hold_app_key_until(
                app,
                VirtualKeyCode::KeyZ,
                "lit-TFLN CLNK grips the GOLD face",
                40,
                |app| {
                    app.engine
                        .object_snapshot(clonk)
                        .is_some_and(|object| object.action.name.starts_with("Scale"))
                },
            );
            advance_app_until(
                app,
                "replacement TFLN fuse burns down at the GOLD face",
                50,
                |app| {
                    app.engine
                        .object_snapshot(flint)
                        .is_some_and(|flint| flint.action.time >= 48)
                },
            );
            AppVirtualKeyboard::new(app)
                .tap(VirtualKeyCode::KeyA)
                .expect("physical A drops the nearly spent TFLN at the GOLD face");
            advance_app_until(
                app,
                "nearly spent replacement TFLN drops at the GOLD face",
                10,
                |app| {
                    app.engine
                        .object_snapshot(flint)
                        .is_some_and(|flint| flint.container.is_none())
                },
            );
        }
        AppVirtualKeyboard::new(app)
            .press(VirtualKeyCode::KeyC)
            .expect("physical C retreats from replacement TFLN");
        let mut retreat_key = VirtualKeyCode::KeyC;
        let mut previous_action = String::new();
        for _ in 0..180 {
            if app.engine.object_snapshot(flint).is_none() {
                break;
            }
            let clonk_now = app
                .engine
                .object_snapshot(clonk)
                .expect("CLNK survives replacement-TFLN retreat");
            let action = clonk_now.action.name;
            if action.starts_with("Scale") && !previous_action.starts_with("Scale") {
                AppVirtualKeyboard::new(app)
                    .release(retreat_key)
                    .expect("release held physical key on replacement retreat Scale");
                if clonk_now.direction == Direction::Left {
                    let mut keyboard = AppVirtualKeyboard::new(app);
                    keyboard
                        .tap(VirtualKeyCode::KeyZ)
                        .expect("physical Z reverses replacement retreat Scale buffer");
                    keyboard
                        .press(VirtualKeyCode::KeyC)
                        .expect("physical C detaches replacement retreat Scale");
                    retreat_key = VirtualKeyCode::KeyC;
                } else {
                    AppVirtualKeyboard::new(app)
                        .press(VirtualKeyCode::KeyS)
                        .expect("physical S climbs replacement retreat Scale");
                    retreat_key = VirtualKeyCode::KeyS;
                }
            } else if !action.starts_with("Scale")
                && previous_action.starts_with("Scale")
                && retreat_key == VirtualKeyCode::KeyS
            {
                let mut keyboard = AppVirtualKeyboard::new(app);
                keyboard
                    .release(VirtualKeyCode::KeyS)
                    .expect("release physical S after replacement retreat Scale");
                keyboard
                    .press(VirtualKeyCode::KeyC)
                    .expect("resume physical C replacement-TFLN retreat");
                retreat_key = VirtualKeyCode::KeyC;
            }
            previous_action = action;
            app.update()
                .expect("advance replacement-TFLN fuse and retreat");
        }
        AppVirtualKeyboard::new(app)
            .release(retreat_key)
            .expect("release physical replacement-TFLN retreat control");
        assert!(
            app.engine.object_snapshot(flint).is_none(),
            "the exact replacement TFLN must detonate"
        );
        app_recover_tutorial04_clonk(
            app,
            clonk,
            "replacement-TFLN CLNK recovers after the blast",
            60,
            true,
        );
        advance_app_until(
            app,
            &format!("replacement TFLN frees another GOLD chunk at face {face_x}"),
            180,
            |app| {
                app.engine
                    .snapshot()
                    .objects
                    .iter()
                    .filter(|object| object.definition_id == "GOLD")
                    .count()
                    > gold_before
            },
        );
        let gold_after = app
            .engine
            .snapshot()
            .objects
            .iter()
            .filter(|object| object.definition_id == "GOLD")
            .count();
        assert_eq!(
            gold_after,
            gold_before + expected_gold_growth,
            "fixed corrected-seed face {face_x} must free exactly {expected_gold_growth} GOLD object(s)"
        );
    }

    fn app_cursor_inventory_contains(app: &mut GameApp, clonk: ObjectId, definition: &str) -> bool {
        let mut overlays = collect_player_overlays(
            &mut app.engine,
            &app.snapshot,
            Some(clonk),
            &app.bindings,
            &app.gamepad_bindings,
        );
        populate_crew_inventories(
            &app.engine,
            &app.snapshot,
            &mut overlays,
            clonk_frontend::AdvancedRendererConfig::DEFAULT,
        );
        overlays
            .iter()
            .flat_map(|player| &player.crew)
            .find(|crew| crew.object_id == clonk)
            .is_some_and(|crew| {
                crew.inventory
                    .iter()
                    .any(|item| item.definition_id == definition && item.picture.is_some())
            })
    }

    fn real_tutorial_app(tutorial: u8, player_name: &str) -> RealTutorialApp {
        real_installed_scenario_app(
            &format!("Tutorial.c4f/Tutorial{tutorial:02}.c4s"),
            player_name,
        )
    }

    fn real_tutorial_app_with_roster(tutorial: u8, player_name: &str) -> RealTutorialApp {
        real_installed_scenario_app_with_roster(
            &format!("Tutorial.c4f/Tutorial{tutorial:02}.c4s"),
            player_name,
            true,
        )
    }

    fn app_tutorial09_system_names_preserve_cpp_ready_conkit_route(
        prepared: &PreparedRealInstalledScenario,
    ) {
        // C4Game::InitScriptEngine loads System.c4g/Names.txt before players
        // join. C4ObjectInfoCore::Default consumes its synchronized name draw
        // before PlaceReadyCrew's position draw, leaving the seed-zero CLNK
        // just left of CNKT so the shipped rightward lesson route collects it
        // (C4Game.cpp:2767-2792; C4InfoCore.cpp:34-55;
        // C4Player.cpp:481-520).
        let mut app = prepared.instantiate("Tutorial 9 app name parity", false);
        let clonk = app
            .engine
            .crew_cursor(app.local_owner)
            .expect("Tutorial09 starts with one cursor CLNK");
        assert_eq!(
            app.engine
                .object_snapshot(clonk)
                .expect("Tutorial09 CLNK survives startup")
                .position,
            Vector2::new(278, 130),
            "System names keep the C++ seed-zero ready-crew placement"
        );
        advance_app_until(&mut app, "Tutorial09 asks for an igloo", 240, |app| {
            app_tutorial_message_contains(app, "build an igloo")
        });
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::KeyC,
            "physical C collects Tutorial09 CNKT",
            30,
            |app| app_clonk_carries(app, clonk, "CNKT"),
        );
    }

    fn run_real_tutorial01_app_subcase(
        name: &'static str,
        failures: &mut Vec<&'static str>,
        subcase: impl FnOnce(),
    ) {
        eprintln!("running Tutorial01 app subcase `{name}`");
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(subcase)).is_err() {
            eprintln!("Tutorial01 app subcase `{name}` failed; continuing batch");
            failures.push(name);
        }
    }

    fn assert_no_real_tutorial01_app_subcase_failures(failures: Vec<&str>) {
        assert!(
            failures.is_empty(),
            "Tutorial01 app subcase(s) failed: {}",
            failures.join(", ")
        );
    }
