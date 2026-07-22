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
                .tap(VirtualKeyCode::X)
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
            VirtualKeyCode::Z,
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
                        .tap(VirtualKeyCode::X)
                        .expect("physical X drops debris-carrying CLNK from Hangle");
                } else if carried_action.starts_with("Scale") {
                    let direction = app
                        .engine
                        .object_snapshot(clonk)
                        .expect("debris-carrying CLNK survives Scale")
                        .direction;
                    let detach = if direction == Direction::Left {
                        VirtualKeyCode::C
                    } else {
                        VirtualKeyCode::Z
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
                    (VirtualKeyCode::Z, Direction::Left)
                } else {
                    (VirtualKeyCode::C, Direction::Right)
                };
                let (away, away_direction) = if toward == VirtualKeyCode::Z {
                    (VirtualKeyCode::C, Direction::Right)
                } else {
                    (VirtualKeyCode::Z, Direction::Left)
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
                    .tap(VirtualKeyCode::A)
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
                    .tap(VirtualKeyCode::X)
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
                    VirtualKeyCode::C
                } else {
                    VirtualKeyCode::Z
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
                VirtualKeyCode::Z
            } else {
                VirtualKeyCode::C
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
            .press(VirtualKeyCode::C)
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
                    .release(VirtualKeyCode::C)
                    .expect("release physical C on blast-pocket Scale");
                keyboard
                    .press(VirtualKeyCode::C)
                    .expect("repress physical C on blast-pocket Scale");
            } else if landed || left_scale_in_flight {
                AppVirtualKeyboard::new(app)
                    .tap(VirtualKeyCode::S)
                    .expect("physical S jumps out of the blast pocket");
            }
            previous_action = action;
            app.update()
                .unwrap_or_else(|error| panic!("advance {milestone}: {error}"));
        }
        AppVirtualKeyboard::new(app)
            .release(VirtualKeyCode::C)
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
                        .press(VirtualKeyCode::X)
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
                        .release(VirtualKeyCode::X)
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
                    VirtualKeyCode::C
                } else {
                    VirtualKeyCode::Z
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
                    VirtualKeyCode::C
                } else {
                    VirtualKeyCode::Z
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
                .tap(VirtualKeyCode::X)
                .expect("first physical X primes fresh ELEC grab");
            keyboard
                .tap(VirtualKeyCode::X)
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
            VirtualKeyCode::S,
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
                .tap(VirtualKeyCode::X)
                .expect("first physical X primes surface ELEC release");
            keyboard
                .tap(VirtualKeyCode::X)
                .expect("second physical X releases ELEC at surface");
        }
        advance_app_until(app, "CLNK releases ELEC at the surface", 60, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk")
        });

        AppVirtualKeyboard::new(app)
            .press(VirtualKeyCode::Z)
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
                    .release(VirtualKeyCode::Z)
                    .expect("release physical Z on HUT3 surface Scale");
                keyboard
                    .press(VirtualKeyCode::Z)
                    .expect("repress physical Z on HUT3 surface Scale");
            } else if landed || left_scale_in_flight {
                AppVirtualKeyboard::new(app)
                    .tap(VirtualKeyCode::S)
                    .expect("physical S jumps the HUT3 surface lip");
            }
            previous_action = action;
            app.update().expect("advance HUT3 surface crossing");
        }
        AppVirtualKeyboard::new(app)
            .release(VirtualKeyCode::Z)
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
            VirtualKeyCode::C,
            &format!("{subject} steps into HUT3 entrance"),
            80,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.position.x >= 62)
            },
        );
        AppVirtualKeyboard::new(app)
            .tap(VirtualKeyCode::S)
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
            .tap(VirtualKeyCode::A)
            .expect("physical A exits Tutorial07 HUT3");
        advance_app_until(app, "empty CLNK exits Tutorial07 HUT3", 60, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.container != Some(hut))
        });

        AppVirtualKeyboard::new(app)
            .press(VirtualKeyCode::C)
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
                    .release(VirtualKeyCode::C)
                    .expect("release physical C on empty surface Scale");
                keyboard
                    .press(VirtualKeyCode::C)
                    .expect("repress physical C on empty surface Scale");
            } else if landed || left_scale_in_flight {
                AppVirtualKeyboard::new(app)
                    .tap(VirtualKeyCode::S)
                    .expect("physical S jumps the empty surface lip");
            }
            previous_action = action;
            app.update().expect("advance empty surface crossing");
        }
        AppVirtualKeyboard::new(app)
            .release(VirtualKeyCode::C)
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
            VirtualKeyCode::X,
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
            VirtualKeyCode::Z,
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
                .tap(VirtualKeyCode::X)
                .expect("first physical X primes empty ELEC grab");
            keyboard
                .tap(VirtualKeyCode::X)
                .expect("second physical X grabs ELEC for another descent");
        }
        advance_app_until(app, "empty CLNK grabs ELEC", 60, |app| {
            app.engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Push" && object.action.target == Some(elevator_case)
            })
        });
        hold_app_key_until(
            app,
            VirtualKeyCode::D,
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
                .tap(VirtualKeyCode::X)
                .expect("first physical X primes underground release");
            keyboard
                .tap(VirtualKeyCode::X)
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
                .press(VirtualKeyCode::C)
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
                    .press(VirtualKeyCode::S)
                    .expect("physical S starts climbing the GOLD tunnel wall");
                keyboard
                    .release(VirtualKeyCode::C)
                    .expect("release physical C while climbing the GOLD tunnel wall");
            }
            app.update()
                .expect("advance the GOLD tunnel-wall control transition");
            {
                let mut keyboard = AppVirtualKeyboard::new(app);
                keyboard
                    .release(VirtualKeyCode::S)
                    .expect("release physical S to refresh the GOLD wall climb");
                keyboard
                    .press(VirtualKeyCode::S)
                    .expect("repress physical S to climb the GOLD tunnel wall");
            }
        } else {
            AppVirtualKeyboard::new(app)
                .press(VirtualKeyCode::S)
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
            .release(VirtualKeyCode::S)
            .expect("release physical S at the GOLD tunnel throat");
        let throat = app
            .engine
            .object_snapshot(clonk)
            .expect("GOLD-carrying CLNK reaches the tunnel throat");
        let away = if throat.direction == Direction::Left {
            VirtualKeyCode::C
        } else {
            VirtualKeyCode::Z
        };
        {
            let mut keyboard = AppVirtualKeyboard::new(app);
            keyboard
                .press(VirtualKeyCode::S)
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
            .release(VirtualKeyCode::S)
            .expect("release physical S beyond the GOLD tunnel throat");

        if app
            .engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Hangle")
        {
            hold_app_key_until(
                app,
                VirtualKeyCode::C,
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
                    .tap(VirtualKeyCode::X)
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
                        .tap(VirtualKeyCode::Z)
                        .expect("physical Z launches from the deep GOLD shoulder");
                    keyboard
                        .tap(VirtualKeyCode::X)
                        .expect("physical X clears the deep GOLD shoulder contact");
                    keyboard
                        .press(VirtualKeyCode::S)
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
                    .release(VirtualKeyCode::S)
                    .expect("release physical S at the diagonal GOLD tunnel roof");
                hold_app_key_until(
                    app,
                    VirtualKeyCode::C,
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
                    .press(VirtualKeyCode::C)
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
                                .release(VirtualKeyCode::C)
                                .expect("release physical C on the original GOLD shoulder");
                            right_held = false;
                        }
                        let away = if clonk_now.direction == Direction::Left {
                            VirtualKeyCode::C
                        } else {
                            VirtualKeyCode::Z
                        };
                        AppVirtualKeyboard::new(app)
                            .tap(away)
                            .expect("physical direction releases the original GOLD shoulder");
                    } else if landed || left_scale_in_flight {
                        if !right_held {
                            AppVirtualKeyboard::new(app)
                                .press(VirtualKeyCode::C)
                                .expect("physical C resumes the original GOLD-shoulder climb");
                            right_held = true;
                        }
                        AppVirtualKeyboard::new(app)
                            .tap(VirtualKeyCode::S)
                            .expect("physical S jumps the original GOLD shoulder");
                    } else if action == "Hangle" && previous_action != "Hangle" {
                        AppVirtualKeyboard::new(app)
                            .tap(VirtualKeyCode::X)
                            .expect("physical X drops from the original GOLD shoulder ceiling");
                    }
                    previous_action = action;
                    app.update()
                        .expect("advance the original GOLD-shoulder climb");
                }
                if right_held {
                    AppVirtualKeyboard::new(app)
                        .release(VirtualKeyCode::C)
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
            VirtualKeyCode::C,
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
            .tap(VirtualKeyCode::X)
            .expect("physical X grabs ELEC for a GOLD sale trip");
        advance_app_until(app, "GOLD sale trip grabs ELEC", 60, |app| {
            app.engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Push" && object.action.target == Some(elevator_case)
            })
        });
        hold_app_key_until(
            app,
            VirtualKeyCode::S,
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
                .tap(VirtualKeyCode::X)
                .expect("first physical X releases ELEC on a GOLD trip");
            keyboard
                .tap(VirtualKeyCode::X)
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
            .press(VirtualKeyCode::C)
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
                    .release(VirtualKeyCode::C)
                    .expect("release physical C on a shaft-lip Scale");
                keyboard
                    .press(VirtualKeyCode::C)
                    .expect("repress physical C on a shaft-lip Scale");
            } else if landed || left_scale_in_flight {
                AppVirtualKeyboard::new(app)
                    .tap(VirtualKeyCode::S)
                    .expect("physical S jumps the Tutorial04 shaft lip");
            }
            previous_action = action;
            app.update()
                .expect("advance a GOLD trip over the shaft lip");
        }
        AppVirtualKeyboard::new(app)
            .release(VirtualKeyCode::C)
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
            VirtualKeyCode::Z,
            "GOLD sale trip aligns with HUT2",
            80,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.position.x <= 570)
            },
        );
        AppVirtualKeyboard::new(app)
            .tap(VirtualKeyCode::S)
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
                .tap(VirtualKeyCode::X)
                .expect("physical X advances HUT2 context to Exit");
        }
        assert_eq!(
            app_selected_object_menu_item(app).map(|item| item.caption.as_str()),
            Some("Exit"),
            "physical context navigation must select HUT2 Exit"
        );
        AppVirtualKeyboard::new(app)
            .tap(VirtualKeyCode::A)
            .expect("physical A exits HUT2 for another GOLD trip");
        advance_app_until(app, "empty CLNK exits HUT2", 60, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.container != Some(hut))
        });

        hold_app_key_until(
            app,
            VirtualKeyCode::Z,
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
                VirtualKeyCode::X,
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
                .tap(VirtualKeyCode::X)
                .expect("first physical X primes returning ELEC grab");
            keyboard
                .tap(VirtualKeyCode::X)
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
                VirtualKeyCode::C
            } else {
                VirtualKeyCode::Z
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
                    VirtualKeyCode::Z
                } else {
                    VirtualKeyCode::C
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
                .tap(VirtualKeyCode::X)
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
            VirtualKeyCode::D,
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
                .tap(VirtualKeyCode::X)
                .expect("first physical X releases ELEC underground");
            keyboard
                .tap(VirtualKeyCode::X)
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
            .tap(VirtualKeyCode::A)
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
            .tap(VirtualKeyCode::A)
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
            .tap(VirtualKeyCode::D)
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
                        .release(VirtualKeyCode::S)
                        .expect("release physical S after blast-pocket recovery");
                }
                if drifting {
                    AppVirtualKeyboard::new(app)
                        .release(VirtualKeyCode::C)
                        .expect("release physical C after blast-pocket recovery");
                }
                return;
            }
            if action.starts_with("Scale") && !previous_action.starts_with("Scale") {
                if drifting {
                    AppVirtualKeyboard::new(app)
                        .release(VirtualKeyCode::C)
                        .expect("release physical C before scaling out of blast pocket");
                    drifting = false;
                }
                AppVirtualKeyboard::new(app)
                    .press(VirtualKeyCode::S)
                    .expect("physical S scales during blast-pocket recovery");
                climbing = true;
            } else if !action.starts_with("Scale")
                && previous_action.starts_with("Scale")
                && climbing
            {
                AppVirtualKeyboard::new(app)
                    .release(VirtualKeyCode::S)
                    .expect("release physical S after Scale corner");
                climbing = false;
            } else if action == "Hangle" {
                if climbing {
                    AppVirtualKeyboard::new(app)
                        .release(VirtualKeyCode::S)
                        .expect("release physical S before leaving Hangle");
                    climbing = false;
                }
                if drifting {
                    AppVirtualKeyboard::new(app)
                        .release(VirtualKeyCode::C)
                        .expect("release physical C before leaving Hangle");
                }
                AppVirtualKeyboard::new(app)
                    .tap(VirtualKeyCode::X)
                    .expect("physical X drops from blast-pocket Hangle");
                app.update().expect("advance one Hangle-drop frame");
                AppVirtualKeyboard::new(app)
                    .press(VirtualKeyCode::C)
                    .expect("physical C drifts during blast-pocket recovery");
                drifting = true;
            }
            previous_action = action;
            app.update()
                .expect("advance Tutorial04 blast-pocket recovery");
        }
        if climbing {
            AppVirtualKeyboard::new(app)
                .release(VirtualKeyCode::S)
                .expect("release physical S after failed blast-pocket recovery");
        }
        if drifting {
            AppVirtualKeyboard::new(app)
                .release(VirtualKeyCode::C)
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
            VirtualKeyCode::Z,
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
                .tap(VirtualKeyCode::X)
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
            .press(VirtualKeyCode::Z)
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
            .release(VirtualKeyCode::Z)
            .expect("release physical Z at the replacement blast face");
        if face_x == 414
            && app
                .engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.y > 403)
        {
            hold_app_key_until(
                app,
                VirtualKeyCode::S,
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
                VirtualKeyCode::C
            } else {
                VirtualKeyCode::Z
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
                    .tap(VirtualKeyCode::D)
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
            let replacement_dig_vertical = VirtualKeyCode::X;
            {
                let mut keyboard = AppVirtualKeyboard::new(app);
                keyboard
                    .press(VirtualKeyCode::Z)
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
                    .release(VirtualKeyCode::Z)
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
                VirtualKeyCode::Z,
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
                    VirtualKeyCode::X,
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
                VirtualKeyCode::Z,
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
            .tap(VirtualKeyCode::A)
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
                VirtualKeyCode::C
            } else {
                VirtualKeyCode::Z
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
                        .tap(VirtualKeyCode::C)
                        .expect("physical C clears the throw double-click buffer");
                    app.update()
                        .expect("advance the incidental-item throw facing control");
                    AppVirtualKeyboard::new(app)
                        .tap(VirtualKeyCode::A)
                        .expect("physical A throws incidental blast-pocket material");
                    AppVirtualKeyboard::new(app)
                        .press(VirtualKeyCode::Z)
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
                        .release(VirtualKeyCode::Z)
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
                    VirtualKeyCode::Z,
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
                VirtualKeyCode::Z,
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
                .tap(VirtualKeyCode::A)
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
            .press(VirtualKeyCode::C)
            .expect("physical C retreats from replacement TFLN");
        let mut retreat_key = VirtualKeyCode::C;
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
                        .tap(VirtualKeyCode::Z)
                        .expect("physical Z reverses replacement retreat Scale buffer");
                    keyboard
                        .press(VirtualKeyCode::C)
                        .expect("physical C detaches replacement retreat Scale");
                    retreat_key = VirtualKeyCode::C;
                } else {
                    AppVirtualKeyboard::new(app)
                        .press(VirtualKeyCode::S)
                        .expect("physical S climbs replacement retreat Scale");
                    retreat_key = VirtualKeyCode::S;
                }
            } else if !action.starts_with("Scale")
                && previous_action.starts_with("Scale")
                && retreat_key == VirtualKeyCode::S
            {
                let mut keyboard = AppVirtualKeyboard::new(app);
                keyboard
                    .release(VirtualKeyCode::S)
                    .expect("release physical S after replacement retreat Scale");
                keyboard
                    .press(VirtualKeyCode::C)
                    .expect("resume physical C replacement-TFLN retreat");
                retreat_key = VirtualKeyCode::C;
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

    #[test]
    fn real_hazard_scenario_gui_sheet_overrides_apply_and_reach_running() {
        let user_data = tempdir().expect("isolated Hazard override user data");
        let (_paths_guard, paths) = exact_loader_test_paths(user_data.path(), None);
        configure_test_startup_participant(&paths, user_data.path());
        let audio_options = AudioOptions {
            sound_enabled: false,
            music_enabled: false,
            menu_music_enabled: false,
            menu_sound_enabled: false,
            ..AudioOptions::default()
        };
        let mut app = GameApp::new(
            320,
            200,
            audio_options,
            Some(&paths),
            RuntimeConfig {
                player_owner: 1,
                player_name: "Hazard GUI parity".to_string(),
                network: None,
                record_enabled: false,
            },
        )
        .expect("initialize Hazard override app");
        wait_for_menu(&mut app);
        let pristine_scroll = app
            .assets
            .startup_dialog_images
            .get("GUIScroll.png")
            .expect("pristine startup scroll sheet")
            .clone();
        let scenario =
            resolve_next_mission_scenario(&app.scenario_catalog, "Hazard.c4f/Tutorial.c4s")
                .expect("Hazard tutorial is present in the real scenario catalog");

        // The user repro: starting any Hazard map used to refuse during
        // loading with a GlobalGuiBootstrapResources boundary because the
        // folder's Graphics.c4g overrides GUICaption/GUIScroll/GUIProgress.
        // C++ instead applies those overrides (C4GraphicsResource::Init →
        // C4GUI::Resource::Load over the registered set).
        app.start_scenario(scenario).expect("start Hazard tutorial");
        wait_for_running_with_attempts(&mut app, 2_400);

        assert!(app.effective_global_gui_failures().is_empty());
        app.assets
            .require_classic_global_gui_bootstrap_resources(&HashMap::new())
            .expect("running Hazard keeps the global GUI bundle boundary-clean");
        for stem in ["GUICaption", "GUIScroll", "GUIProgress"] {
            let source = app
                .assets
                .active_gui_sheet_sources
                .get(stem)
                .unwrap_or_else(|| panic!("{stem} must be rebound while Hazard runs"));
            assert!(
                source.contains("Hazard.c4f") && source.contains("Graphics.c4g"),
                "{stem} must be won by the Hazard folder pack: {source}"
            );
        }
        let running_scroll = app
            .assets
            .startup_dialog_images
            .get("GUIScroll.png")
            .expect("running scroll sheet")
            .clone();
        assert_ne!(
            running_scroll.pixels(),
            pristine_scroll.pixels(),
            "the Hazard scroll sheet must replace the global surface"
        );
        assert!(
            app.assets.message_dialog_resources().is_some(),
            "running dialogs resolve from the rebound sheets"
        );

        app.return_to_menu();
        assert!(app.assets.active_gui_sheet_sources.is_empty());
        assert_eq!(
            app.assets
                .startup_dialog_images
                .get("GUIScroll.png")
                .expect("restored scroll sheet")
                .pixels()
                .as_ptr(),
            pristine_scroll.pixels().as_ptr(),
            "teardown must restore the pristine startup scroll sheet"
        );
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

    #[test]
    fn real_alchemy_mouse_subcases_batch_1() {
        let prepared =
            PreparedRealInstalledScenario::new("Fantasy.c4f/Alchemy.c4s");
        let mut failures = Vec::new();
        run_real_alchemy_app_subcase(
            "right_click_positions_classic_context_magic_menu",
            &mut failures,
            || l068_real_alchemy_right_click_positions_classic_context_magic_menu(&prepared),
        );
        run_real_alchemy_app_subcase(
            "right_drag_frame_drops_all_selected_carryables",
            &mut failures,
            || real_alchemy_right_drag_frame_drops_all_selected_carryables(&prepared),
        );
        assert_no_real_alchemy_app_subcase_failures(failures);
    }

    #[test]
    fn real_alchemy_mouse_subcases_batch_2() {
        let prepared =
            PreparedRealInstalledScenario::new("Fantasy.c4f/Alchemy.c4s");
        let mut failures = Vec::new();
        run_real_alchemy_app_subcase(
            "control_right_drag_puts_carryable_into_hut",
            &mut failures,
            || real_alchemy_control_right_drag_puts_carryable_into_hut(&prepared),
        );
        run_real_alchemy_app_subcase(
            "right_drag_rectangle_replaces_crew_selection",
            &mut failures,
            || real_alchemy_right_drag_rectangle_replaces_crew_selection(&prepared),
        );
        run_real_alchemy_app_subcase(
            "left_double_click_gets_carryable_like_cpp_mouse_control",
            &mut failures,
            || real_alchemy_left_double_click_gets_carryable_like_cpp_mouse_control(&prepared),
        );
        assert_no_real_alchemy_app_subcase_failures(failures);
    }

    fn run_real_alchemy_app_subcase(
        name: &'static str,
        failures: &mut Vec<&'static str>,
        subcase: impl FnOnce(),
    ) {
        eprintln!("running Alchemy app subcase `{name}`");
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(subcase)).is_err() {
            eprintln!("Alchemy app subcase `{name}` failed; continuing batch");
            failures.push(name);
        }
    }

    fn assert_no_real_alchemy_app_subcase_failures(failures: Vec<&str>) {
        assert!(
            failures.is_empty(),
            "Alchemy app subcase(s) failed: {}",
            failures.join(", ")
        );
    }

    fn l068_real_alchemy_right_click_positions_classic_context_magic_menu(
        prepared: &PreparedRealInstalledScenario,
    ) {
        // C4MouseControl issues C4CMD_Context on right-up with the clicked
        // MCLK as Target2. The command installs classic style-1 context on
        // the selected mage; entering ContextMagic opens the shipped spell
        // menu (C4MouseControl.cpp:1230-1263; C4Command.cpp:1076-1090;
        // MagiClonk.c4d/Script.c:190-199).
        let mut app = prepared.instantiate("Alchemy mouse context parity", false);
        let owner = app.local_owner;
        let mage = app
            .engine
            .crew_cursor(owner)
            .expect("Alchemy starts with a selected mage");
        assert_eq!(
            app.engine
                .object_snapshot(mage)
                .expect("mage remains live")
                .definition_id,
            "MCLK"
        );
        assert_eq!(
            app.engine
                .object_snapshot(mage)
                .expect("mage remains live")
                .magic_energy,
            0,
            "Alchemy's NMGE rule leaves raw mana at zero, so C++ draws no HUD mana bar"
        );

        // Scenario join leaves crew inside the home base with the same
        // queued Exit command as C++ startup. Let that command finish before
        // exercising a world click: contained objects are deliberately not
        // mouse targets in C4Game::FindVisObject.
        for _ in 0..80 {
            if app
                .engine
                .object_snapshot(mage)
                .expect("mage remains live")
                .container
                .is_none()
            {
                break;
            }
            app.update().expect("execute startup Exit command");
        }
        assert!(
            app.engine
                .object_snapshot(mage)
                .expect("mage remains live")
                .container
                .is_none(),
            "Alchemy mage exits the home base before a world context click"
        );

        let mut frame = vec![0_u8; 320 * 200 * 4];
        app.render(&mut frame).expect("establish Alchemy viewport");
        let rendered_mage = app
            .snapshot
            .object(mage)
            .cloned()
            .expect("mage is present in app snapshot");
        assert_ne!(
            rendered_mage.ocf, 0,
            "live MCLK carries a targetable cached OCF"
        );
        let (screen_x, screen_y) = app
            .graphics
            .world_to_screen(
                owner,
                app.engine
                    .object_snapshot(mage)
                    .expect("mage snapshot")
                    .position,
            )
            .expect("mage is in the local viewport");
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(screen_x),
            f64::from(screen_y),
        ))
        .expect("move pointer over mage");
        assert_eq!(
            app.graphics
                .object_at_point(&app.snapshot, owner, GuiPoint::new(screen_x, screen_y),),
            Some(mage),
            "C++ front-to-back object picking selects the topmost MCLK",
        );
        let pointer = app.ingame_pointer.expect("right-click retains viewport pointer");
        let projection = app
            .graphics
            .active_viewport_projections()
            .into_iter()
            .find(|viewport| viewport.owner == owner)
            .expect("Alchemy owner viewport projection");
        let (click_x, click_y) = ingame_pointer_viewport_pixel(pointer, projection);
        assert_ne!(click_x, 0, "fixture must enter C++'s free-alignment branch");
        assert_ne!(click_y, 0, "fixture must enter C++'s free-alignment branch");
        let click_location = Vector2::new(click_x, click_y);

        app.handle_right_mouse_button(ElementState::Pressed)
            .expect("right-down stores no command");
        assert!(app.engine.cursor_object_menu(owner).is_none());
        app.handle_right_mouse_button(ElementState::Released)
            .expect("right-up queues C4CMD_Context");
        app.update().expect("execute the context command");

        assert!(
            app.object_menu.is_none(),
            "mouse context must use the classic engine menu, not the app fallback"
        );
        let context = app
            .engine
            .cursor_object_menu(owner)
            .expect("right-up opens the mage context menu")
            .1
            .clone();
        assert_eq!(context.style, 1);
        assert!(!context.permanent);
        assert_eq!(
            context.location,
            Some(click_location),
            "the synchronized Context command keeps logical viewport-local Tx/Ty"
        );
        let magic_index = context
            .items
            .iter()
            .position(|item| item.command.contains("ContextMagic"))
            .unwrap_or_else(|| {
                panic!(
                    "MCLK context contains ContextMagic; action={:?}; items={:?}",
                    app.engine
                        .object_snapshot(mage)
                        .expect("mage remains live")
                        .action,
                    context.items
                )
            });

        let viewport = app.graphics.viewport_rect(owner).expect("Alchemy viewport");
        app.render(&mut frame)
            .expect("render the freely aligned context menu");
        let latched_screen = app
            .script_menu_presentations
            .get(&owner)
            .and_then(|state| state.location)
            .expect("free context location is latched after layout");
        let latched_local = Vector2::new(
            latched_screen.0.saturating_sub(viewport.x),
            latched_screen.1.saturating_sub(viewport.y),
        );
        assert!(
            latched_local.x <= click_location.x && latched_local.y <= click_location.y,
            "right/bottom edges may clamp the menu back into the viewport"
        );
        assert_eq!(
            app.ingame_menu_gfx
                .as_ref()
                .and_then(|gfx| gfx.menu_location),
            Some(latched_screen),
            "viewport-local coordinates are translated exactly once for drawing"
        );

        let mut moved_context = context.clone();
        let moved_x = latched_local.x.saturating_sub(4);
        assert_ne!(
            moved_x, latched_local.x,
            "fixture must leave room for relocation"
        );
        moved_context.location = Some(Vector2::new(moved_x, latched_local.y));
        app.engine
            .apply_object_update(
                mage,
                ObjectUpdate {
                    menu: Some(Some(moved_context.clone())),
                    ..ObjectUpdate::default()
                },
            )
            .expect("reopen the same context identity at another click");
        app.render(&mut frame)
            .expect("render the relocated context menu");
        assert_eq!(
            app.script_menu_presentations
                .get(&owner)
                .and_then(|state| state.location),
            Some((
                viewport.x.saturating_add(moved_x),
                viewport.y.saturating_add(latched_local.y),
            )),
            "a new click location invalidates the prior presentation latch"
        );

        let mut tall_context = moved_context;
        tall_context.location = Some(Vector2::new(
            viewport.width as i32 - 1,
            viewport.height as i32 - 1,
        ));
        tall_context.items.push(context.items[magic_index].clone());
        app.engine
            .apply_object_update(
                mage,
                ObjectUpdate {
                    menu: Some(Some(tall_context.clone())),
                    ..ObjectUpdate::default()
                },
            )
            .expect("install a taller edge-clamped context refill");
        app.render(&mut frame).expect("render the taller context");
        let edge_latched = app
            .script_menu_presentations
            .get(&owner)
            .and_then(|state| state.location)
            .expect("edge location clamps and latches");
        tall_context.items.pop();
        app.engine
            .apply_object_update(
                mage,
                ObjectUpdate {
                    menu: Some(Some(tall_context)),
                    ..ObjectUpdate::default()
                },
            )
            .expect("apply a shrinking context refill");
        app.render(&mut frame).expect("render the smaller context");
        assert_eq!(
            app.script_menu_presentations
                .get(&owner)
                .and_then(|state| state.location),
            Some(edge_latched),
            "C++ refills retain the first post-clamp rcBounds position"
        );

        app.engine
            .apply_object_update(
                mage,
                ObjectUpdate {
                    menu: Some(Some(context.clone())),
                    ..ObjectUpdate::default()
                },
            )
            .expect("restore the live context before selecting ContextMagic");

        app.dispatch_control_event(ControlEvent::RawPlayerControl {
            command: clonk_engine::COM_MENU_SELECT,
            data: i32::try_from(magic_index).expect("context index fits i32"),
        })
        .expect("select ContextMagic");
        app.dispatch_control_event(ControlEvent::RawPlayerControl {
            command: clonk_engine::COM_MENU_ENTER,
            data: 0,
        })
        .expect("enter ContextMagic");

        let spell_menu = app
            .engine
            .cursor_object_menu(owner)
            .expect("ContextMagic opens the shipped spell menu")
            .1;
        assert_eq!(
            spell_menu.extra,
            clonk_engine::ObjectMenuExtra::Components,
            "ALCO+NMGE uses C4MN_Extra_Components, never a mana footer"
        );
        let raise_gravity = spell_menu
            .items
            .iter()
            .find(|item| item.item_id == "MGUP")
            .expect("Alchemy's shipped Raise Gravity spell is player-accessible");
        assert_eq!(
            raise_gravity.components,
            [clonk_engine::ObjectMenuComponent {
                definition_id: "IROC".to_string(),
                count: 1,
            }],
            "Alchemy shows MGUP's ingredient recipe instead of mana"
        );
    }

    fn real_alchemy_right_drag_rectangle_replaces_crew_selection(
        prepared: &PreparedRealInstalledScenario,
    ) {
        // A right-down on ordinary landscape stores the down position. Once
        // motion exceeds C4MC_DragSensitivity, C4MouseControl enters
        // C4MC_Drag_Selecting; right-up sends CID_PlrSelect rather than a
        // context click (C4MouseControl.cpp:910-930,1009-1037,795-817,
        // 1160-1171). Exercise the actual app pointer/button path so the
        // platform event split cannot collapse the drag back into RightUp.
        let mut app = prepared.instantiate("Alchemy right drag parity", false);
        let owner = app.local_owner;
        let original = app
            .engine
            .crew_cursor(owner)
            .expect("Alchemy starts with a selected mage");
        advance_app_until(
            &mut app,
            "Alchemy MCLK finishes its startup Exit",
            160,
            |app| {
                app.engine.object_snapshot(original).is_some_and(|object| {
                    object.container.is_none() && object.command_stack.is_empty()
                })
            },
        );

        let mut frame = vec![0_u8; 320 * 200 * 4];
        app.snapshot = app.engine.snapshot();
        app.render(&mut frame).expect("establish Alchemy viewport");
        let (original_x, original_y) = app
            .graphics
            .world_to_screen(
                owner,
                app.engine
                    .object_snapshot(original)
                    .expect("original mage remains live")
                    .position,
            )
            .expect("original mage is visible");
        let target_pointer = (45..155)
            .step_by(10)
            .flat_map(|y| (45..275).step_by(10).map(move |x| (x, y)))
            .find_map(|(x, y)| {
                let point = GuiPoint::new(x as f32, y as f32);
                let start = GuiPoint::new(x as f32 - 24.0, y as f32 - 24.0);
                let pointer = app.graphics.viewport_point_at(point)?;
                let start_pointer = app.graphics.viewport_point_at(start)?;
                (pointer.owner == owner
                    && start_pointer.owner == owner
                    && (point.x - original_x).abs() > 50.0
                    && (point.y - original_y).abs() > 30.0
                    && app
                        .graphics
                        .object_at_point(&app.snapshot, owner, point)
                        .is_none()
                    && app
                        .graphics
                        .object_at_point(&app.snapshot, owner, start)
                        .is_none())
                .then_some(pointer)
            })
            .expect("Alchemy viewport has an empty drag target away from the original mage");
        let target_position = Vector2::new(
            target_pointer.world.x.round() as i32,
            target_pointer.world.y.round() as i32,
        );
        let replacement = app
            .engine
            .spawn_object(
                SpawnConfig::new("MCLK")
                    .with_position(target_position)
                    .with_owner(owner)
                    .with_crew_member(true),
            )
            .expect("spawn a second shipped mage");

        app.update()
            .expect("advance the spawned mage through its first OCF refresh");
        app.snapshot = app.engine.snapshot();
        app.render(&mut frame).expect("render the second mage");
        let target_position = app
            .engine
            .object_snapshot(replacement)
            .expect("second mage remains live")
            .position;
        let (target_x, target_y) = app
            .graphics
            .world_to_screen(owner, target_position)
            .expect("second mage is visible");
        let target = GuiPoint::new(target_x, target_y);
        let start = GuiPoint::new(target.x - 24.0, target.y - 24.0);
        assert_eq!(
            app.graphics.object_at_point(&app.snapshot, owner, target),
            Some(replacement),
            "right-up lands on the second mage, which would expose a collapsed context click"
        );
        assert_eq!(
            app.graphics.object_at_point(&app.snapshot, owner, start),
            None,
            "right-down begins on ordinary landscape"
        );

        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(start.x),
            f64::from(start.y),
        ))
        .expect("move to right-drag start");
        app.handle_right_mouse_button(ElementState::Pressed)
            .expect("physical right-down");
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(target.x),
            f64::from(target.y),
        ))
        .expect("drag across the replacement mage");
        let drag = app
            .ingame_right_mouse_state
            .expect("crew selection drag remains live");
        assert_eq!(drag.motion.selection_kind, IngameDragSelectionKind::Crew);
        assert_eq!(
            app.ingame_selection_candidates(drag.motion),
            vec![replacement],
            "C4MouseControl's transient Selection contains the framed crew"
        );
        app.handle_right_mouse_button(ElementState::Released)
            .expect("physical right-up");

        assert_eq!(
            app.engine.selected_crew(owner),
            vec![replacement],
            "CID_PlrSelect replaces, rather than extends, the previous crew selection"
        );
        assert_eq!(app.engine.crew_cursor(owner), Some(replacement));
        assert!(
            app.engine.cursor_object_menu(owner).is_none(),
            "a completed selection drag must not fall through to C4CMD_Context"
        );
    }

    fn real_alchemy_right_drag_frame_drops_all_selected_carryables(
        prepared: &PreparedRealInstalledScenario,
    ) {
        // An object-only landscape frame remains in C4MouseControl::Selection
        // after right-up. Dragging either selected object then sends one Set
        // command followed by Append commands for the remaining objects
        // (C4MouseControl.cpp:626-645,795-817,909-968,1160-1201;
        // C4Player.cpp:1397-1450). Exercise the physical app events twice so
        // neither selection nor moving can collapse into a context click.
        let mut app = prepared.instantiate("Alchemy object drag parity", false);
        let owner = app.local_owner;
        let mage = app
            .engine
            .crew_cursor(owner)
            .expect("Alchemy starts with a selected mage");
        advance_app_until(
            &mut app,
            "Alchemy MCLK finishes its startup Exit",
            160,
            |app| {
                app.engine.object_snapshot(mage).is_some_and(|object| {
                    object.container.is_none() && object.command_stack.is_empty()
                })
            },
        );

        app.snapshot = app.engine.snapshot();
        app.refresh_focus();
        let mut frame = vec![0_u8; 320 * 200 * 4];
        app.render(&mut frame).expect("establish Alchemy viewport");
        let viewport = app
            .graphics
            .viewport_rect(owner)
            .expect("local Alchemy viewport");
        let (mage_x, mage_y) = app
            .graphics
            .world_to_screen(
                owner,
                app.engine
                    .object_snapshot(mage)
                    .expect("mage remains live")
                    .position,
            )
            .expect("mage is visible");
        let anchor = (50..150)
            .step_by(10)
            .flat_map(|y| (50..250).step_by(10).map(move |x| (x, y)))
            .find_map(|(x, y)| {
                let point = GuiPoint::new(x as f32, y as f32);
                let pointer = app.graphics.viewport_point_at(point)?;
                (pointer.owner == owner
                    && point.x >= viewport.x as f32 + 30.0
                    && point.x <= (viewport.x + viewport.width as i32) as f32 - 55.0
                    && point.y >= viewport.y as f32 + 30.0
                    && point.y <= (viewport.y + viewport.height as i32) as f32 - 30.0
                    && (point.x - mage_x).abs() > 70.0
                    && (point.y - mage_y).abs() > 35.0
                    && app
                        .graphics
                        .object_at_point(&app.snapshot, owner, point)
                        .is_none())
                .then_some(ingame_pointer_world_pixel(pointer))
            })
            .expect("Alchemy viewport has room for an object-only drag frame");
        let layer = app
            .engine
            .object_snapshot(mage)
            .expect("mage remains live")
            .layer;
        let spawn_bag = |app: &mut GameApp, position: Vector2| {
            let spawn = layer
                .map(|layer| {
                    SpawnConfig::new("ALC_")
                        .with_position(position)
                        .with_layer(layer)
                })
                .unwrap_or_else(|| SpawnConfig::new("ALC_").with_position(position));
            app.engine
                .spawn_object(spawn)
                .expect("spawn shipped carryable alchemy bag")
        };
        let first_bag = spawn_bag(&mut app, anchor);
        let second_bag = spawn_bag(&mut app, Vector2::new(anchor.x + 20, anchor.y));
        for bag in [first_bag, second_bag] {
            assert_ne!(
                app.engine
                    .object_snapshot(bag)
                    .expect("spawned bag remains live")
                    .ocf
                    & clonk_engine::ocf::CARRYABLE,
                0,
                "the regression target uses the shipped carryable definition"
            );
        }

        app.snapshot = app.engine.snapshot();
        app.render(&mut frame).expect("render both carryable bags");
        let (first_x, first_y) = app
            .graphics
            .world_to_screen(owner, anchor)
            .expect("first bag is visible");
        let (second_x, second_y) = app
            .graphics
            .world_to_screen(owner, Vector2::new(anchor.x + 20, anchor.y))
            .expect("second bag is visible");
        let frame_start = GuiPoint::new(first_x.min(second_x) - 24.0, first_y.min(second_y) - 24.0);
        let frame_end = GuiPoint::new(first_x.max(second_x) + 24.0, first_y.max(second_y) + 24.0);
        for point in [frame_start, frame_end] {
            assert!(
                app.graphics
                    .viewport_point_at(point)
                    .is_some_and(|pointer| pointer.owner == owner),
                "selection frame endpoint remains in the local viewport"
            );
            assert_eq!(
                app.graphics.object_at_point(&app.snapshot, owner, point),
                None,
                "selection begins and ends on landscape"
            );
        }

        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(frame_start.x),
            f64::from(frame_start.y),
        ))
        .expect("move to object-frame start");
        app.handle_right_mouse_button(ElementState::Pressed)
            .expect("physical frame right-down");
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(frame_end.x),
            f64::from(frame_end.y),
        ))
        .expect("drag frame across both bags");
        let drag = app
            .ingame_right_mouse_state
            .expect("object selection drag remains live");
        assert_eq!(drag.motion.selection_kind, IngameDragSelectionKind::Objects);
        assert_eq!(
            app.ingame_selection_candidates(drag.motion),
            vec![second_bag, first_bag],
            "object marks retain C++ Game.Objects newest-first order"
        );
        app.handle_right_mouse_button(ElementState::Released)
            .expect("physical frame right-up retains object selection");
        assert!(
            app.engine
                .object_snapshot(mage)
                .expect("mage remains live")
                .command_stack
                .is_empty(),
            "object-frame selection is local and sends no player command"
        );

        let first_bag_point = (viewport.y..viewport.y + viewport.height as i32)
            .flat_map(|y| {
                (viewport.x..viewport.x + viewport.width as i32)
                    .map(move |x| GuiPoint::new(x as f32, y as f32))
            })
            .find(|point| {
                app.graphics.object_at_point(&app.snapshot, owner, *point) == Some(first_bag)
            })
            .expect("first selected bag has a visible C++ pick point");
        let drop_pointer = (viewport.y..viewport.y + viewport.height as i32)
            .flat_map(|y| {
                (viewport.x..viewport.x + viewport.width as i32)
                    .map(move |x| GuiPoint::new(x as f32, y as f32))
            })
            .find_map(|point| {
                let pointer = app.graphics.viewport_point_at(point)?;
                let world = ingame_pointer_world_pixel(pointer);
                let landscape = app.engine.landscape()?;
                let ground_y = (world.y..landscape.estimated_height())
                    .find(|y| landscape.is_solid_at(world.x, *y))?;
                (pointer.owner == owner
                    && (point.x - first_bag_point.x).abs() > 12.0
                    && !landscape.is_solid_at(world.x, world.y)
                    && (ground_y - world.y).abs() <= 5
                    && app
                        .graphics
                        .object_at_point(&app.snapshot, owner, point)
                        .is_none())
                .then_some((point, world))
            })
            .expect("visible landscape contains a C++ Drop cursor point");

        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(first_bag_point.x),
            f64::from(first_bag_point.y),
        ))
        .expect("move over one selected bag");
        app.handle_right_mouse_button(ElementState::Pressed)
            .expect("physical moving right-down");
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(drop_pointer.0.x),
            f64::from(drop_pointer.0.y),
        ))
        .expect("drag selected bags to a Drop cursor point");
        app.handle_right_mouse_button(ElementState::Released)
            .expect("physical moving right-up sends object commands");

        let commands = app
            .engine
            .object_snapshot(mage)
            .expect("mage remains live")
            .command_stack
            .command_views();
        assert_eq!(commands.len(), 2, "both framed bags receive commands");
        assert!(commands.iter().all(|command| command.name == "Drop"));
        assert_eq!(
            commands
                .iter()
                .map(|command| command.target)
                .collect::<Vec<_>>(),
            vec![Some(second_bag), Some(first_bag)],
            "Game.Objects main-list order is preserved through Set then Append"
        );
        assert!(commands.iter().all(|command| {
            command.tx == Some(drop_pointer.1.x) && command.ty == Some(drop_pointer.1.y)
        }));
        assert!(app.engine.cursor_object_menu(owner).is_none());
    }

    fn real_alchemy_control_right_drag_puts_carryable_into_hut(
        prepared: &PreparedRealInstalledScenario,
    ) {
        // With Control held, C4MouseControl::DragMoving replaces the ordinary
        // Drop/Throw cursor with Put over an OCF_Container. Right-up sends a
        // C4CMD_Put whose Target is that container and whose Target2 is the
        // dragged object (C4MouseControl.cpp:833-850,1171-1201). Exercise the
        // physical pointer/modifier/button route with shipped ALC_/AHUT defs.
        let mut app = prepared.instantiate("Alchemy control-drag Put parity", false);
        let owner = app.local_owner;
        let mage = app
            .engine
            .crew_cursor(owner)
            .expect("Alchemy starts with a selected mage");
        advance_app_until(
            &mut app,
            "Alchemy MCLK finishes its startup Exit",
            160,
            |app| {
                app.engine.object_snapshot(mage).is_some_and(|object| {
                    object.container.is_none() && object.command_stack.is_empty()
                })
            },
        );

        let hut = app
            .engine
            .snapshot()
            .objects
            .iter()
            .find(|object| object.definition_id == "AHUT" && object.owner == owner)
            .map(|object| object.id)
            .expect("Alchemy starts with the player's shipped AHUT");
        assert_ne!(
            app.engine
                .object_snapshot(hut)
                .expect("AHUT remains live")
                .ocf
                & clonk_engine::ocf::CONTAINER,
            0,
            "AHUT is the C++ OCF_Container Put target"
        );

        app.snapshot = app.engine.snapshot();
        app.refresh_focus();
        let mut frame = vec![0_u8; 320 * 200 * 4];
        app.render(&mut frame).expect("establish Alchemy viewport");
        let viewport = app
            .graphics
            .viewport_rect(owner)
            .expect("local Alchemy viewport");
        let hut_point = (viewport.y..viewport.y + viewport.height as i32)
            .flat_map(|y| {
                (viewport.x..viewport.x + viewport.width as i32)
                    .map(move |x| GuiPoint::new(x as f32 + 0.5, y as f32 + 0.5))
            })
            .find(|point| app.graphics.object_at_point(&app.snapshot, owner, *point) == Some(hut))
            .expect("AHUT has a visible C++ pick point");
        let bag_pointer = (viewport.y..viewport.y + viewport.height as i32)
            .step_by(4)
            .flat_map(|y| {
                (viewport.x..viewport.x + viewport.width as i32)
                    .step_by(4)
                    .map(move |x| GuiPoint::new(x as f32 + 0.5, y as f32 + 0.5))
            })
            .find_map(|point| {
                let pointer = app.graphics.viewport_point_at(point)?;
                (pointer.owner == owner
                    && (point.x - hut_point.x).abs() > 24.0
                    && (point.y - hut_point.y).abs() > 12.0
                    && app
                        .graphics
                        .object_at_point(&app.snapshot, owner, point)
                        .is_none())
                .then_some(pointer)
            })
            .expect("Alchemy viewport has an empty bag spawn point away from AHUT");
        let bag_position = ingame_pointer_world_pixel(bag_pointer);
        let mut bag_spawn = SpawnConfig::new("ALC_").with_position(bag_position);
        if let Some(layer) = app
            .engine
            .object_snapshot(mage)
            .expect("mage remains live")
            .layer
        {
            bag_spawn = bag_spawn.with_layer(layer);
        }
        let bag = app
            .engine
            .spawn_object(bag_spawn)
            .expect("spawn the shipped carryable alchemy bag");

        app.snapshot = app.engine.snapshot();
        app.render(&mut frame).expect("render the dragged bag");
        let bag_point = (viewport.y..viewport.y + viewport.height as i32)
            .flat_map(|y| {
                (viewport.x..viewport.x + viewport.width as i32)
                    .map(move |x| GuiPoint::new(x as f32, y as f32))
            })
            .find(|point| app.graphics.object_at_point(&app.snapshot, owner, *point) == Some(bag))
            .expect("ALC_ has a visible C++ pick point");

        app.handle_modifiers_changed(ModifiersState::CTRL)
            .expect("set Control modifier");
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(bag_point.x),
            f64::from(bag_point.y),
        ))
        .expect("move over the shipped bag");
        app.handle_right_mouse_button(ElementState::Pressed)
            .expect("physical Control-right-down");
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(hut_point.x),
            f64::from(hut_point.y),
        ))
        .expect("drag the bag over AHUT");
        app.handle_right_mouse_button(ElementState::Released)
            .expect("physical Control-right-up");
        app.handle_modifiers_changed(ModifiersState::empty())
            .expect("clear Control modifier");

        let commands = app
            .engine
            .object_snapshot(mage)
            .expect("mage remains live")
            .command_stack
            .command_views();
        assert_eq!(commands.len(), 1, "the drag emits exactly one Put");
        assert_eq!(commands[0].name, "Put");
        assert_eq!(commands[0].target, Some(hut));
        assert_eq!(commands[0].target2, Some(bag));
        assert_eq!(commands[0].tx, None);
        assert_eq!(commands[0].ty, None);
        assert!(app.engine.cursor_object_menu(owner).is_none());
    }

    fn real_alchemy_left_double_click_gets_carryable_like_cpp_mouse_control(
        prepared: &PreparedRealInstalledScenario,
    ) {
        // C4MouseControl's first ordinary left-up replaces the selected crew's
        // stack with MoveTo. A second left-down inside the platform's 400 ms
        // double-click window is delivered as LeftDouble instead: an Object
        // cursor replaces that command with C4CMD_Get and the following left-up
        // is ignored (C4FullScreen.cpp:327-350; C4MouseControl.cpp:817-830,
        // 982-1004,1101-1155).
        let mut app = prepared.instantiate("Alchemy mouse pickup parity", false);
        let owner = app.local_owner;
        let mage = app
            .engine
            .crew_cursor(owner)
            .expect("Alchemy starts with a selected mage");
        advance_app_until(
            &mut app,
            "Alchemy MCLK finishes its startup Exit",
            160,
            |app| {
                app.engine.object_snapshot(mage).is_some_and(|object| {
                    object.container.is_none() && object.command_stack.is_empty()
                })
            },
        );

        app.snapshot = app.engine.snapshot();
        app.refresh_focus();
        let mut frame = vec![0_u8; 320 * 200 * 4];
        app.render(&mut frame).expect("establish Alchemy viewport");
        let empty_pointer = (40..180)
            .step_by(20)
            .flat_map(|y| (20..300).step_by(20).map(move |x| (x, y)))
            .find_map(|(x, y)| {
                let point = GuiPoint::new(x as f32, y as f32);
                let pointer = app.graphics.viewport_point_at(point)?;
                (pointer.owner == owner
                    && app
                        .graphics
                        .object_at_point(&app.snapshot, owner, point)
                        .is_none())
                .then_some(pointer)
            })
            .expect("Alchemy viewport contains an empty world point");
        let bag_position = Vector2::new(
            empty_pointer.world.x.round() as i32,
            empty_pointer.world.y.round() as i32,
        );
        let mut bag_spawn = SpawnConfig::new("ALC_").with_position(bag_position);
        if let Some(layer) = app
            .engine
            .object_snapshot(mage)
            .expect("mage remains live")
            .layer
        {
            bag_spawn = bag_spawn.with_layer(layer);
        }
        let bag = app
            .engine
            .spawn_object(bag_spawn)
            .expect("spawn the shipped carryable alchemy bag");
        let bag_snapshot = app
            .engine
            .object_snapshot(bag)
            .expect("spawned bag remains live");
        assert_ne!(
            bag_snapshot.ocf & clonk_engine::ocf::CARRYABLE,
            0,
            "the regression target uses the shipped carryable definition"
        );

        // FindVisObject's OCF filter is part of the pick itself. A newer
        // foreground object with no primary mouse OCF must therefore be
        // skipped rather than blocking the carryable object behind it.
        let mut blocker =
            Definition::from_script("MBLK", "Mouse blocker", "#strict\n")
                .expect("blocker compiles");
        blocker.set_category(clonk_engine::CATEGORY_OBJECT);
        blocker.set_shape_rect(Some(clonk_engine::DefinitionRect::new(-3, -3, 6, 6)));
        app.engine
            .register_definition(blocker)
            .expect("register foreground blocker");
        let mut blocker_spawn = SpawnConfig::new("MBLK").with_position(bag_position);
        if let Some(layer) = bag_snapshot.layer {
            blocker_spawn = blocker_spawn.with_layer(layer);
        }
        let blocker = app
            .engine
            .spawn_object(blocker_spawn)
            .expect("spawn foreground non-primary blocker");

        app.snapshot = app.engine.snapshot();
        app.render(&mut frame).expect("establish Alchemy viewport");
        let viewport = app
            .graphics
            .viewport_rect(owner)
            .expect("local Alchemy viewport");
        let bag_point = (viewport.y..viewport.y + viewport.height as i32)
            .flat_map(|y| {
                (viewport.x..viewport.x + viewport.width as i32)
                    .map(move |x| GuiPoint::new(x as f32 + 0.5, y as f32 + 0.5))
            })
            .find(|point| {
                app.graphics.object_at_point(&app.snapshot, owner, *point) == Some(blocker)
                    && app.ingame_primary_mouse_target(owner, *point) == Some(bag)
            })
            .expect("the primary OCF pick sees the bag behind a foreground blocker");
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(bag_point.x),
            f64::from(bag_point.y),
        ))
        .expect("move pointer over carryable bag");
        let click_world = ingame_pointer_world_pixel(
            app.ingame_pointer
                .expect("C++-quantized bag point maps into the local viewport"),
        );
        assert_eq!(
            app.graphics
                .object_at_point(&app.snapshot, owner, bag_point),
            Some(blocker),
            "the unfiltered foreground pick sees the newer blocker",
        );
        assert_eq!(
            app.ingame_primary_mouse_target(owner, bag_point),
            Some(bag),
            "the primary mouse OCF pick skips that blocker and resolves the carryable",
        );

        app.handle_mouse_button(ElementState::Pressed)
            .expect("first left-down");
        app.handle_mouse_button(ElementState::Released)
            .expect("first left-up");
        let first_click = app
            .engine
            .object_snapshot(mage)
            .expect("mage remains live after first click")
            .command_stack
            .command_views();
        assert_eq!(first_click.len(), 1);
        assert_eq!(first_click[0].name, "MoveTo");
        assert_eq!(first_click[0].target, None);
        assert_eq!(first_click[0].tx, Some(click_world.x));
        assert_eq!(first_click[0].ty, Some(click_world.y));

        app.handle_mouse_button(ElementState::Pressed)
            .expect("second left-down becomes LeftDouble");
        let double_click = app
            .engine
            .object_snapshot(mage)
            .expect("mage remains live after double click")
            .command_stack
            .command_views();
        assert_eq!(double_click.len(), 1);
        assert_eq!(double_click[0].name, "Get");
        assert_eq!(double_click[0].target, Some(bag));
        assert_eq!(double_click[0].tx, None);
        assert_eq!(double_click[0].ty, None);

        app.handle_mouse_button(ElementState::Released)
            .expect("post-double left-up is ignored");
        assert_eq!(
            app.engine
                .object_snapshot(mage)
                .expect("mage remains live after ignored release")
                .command_stack
                .command_views(),
            double_click,
            "the post-double release must not overwrite Get with MoveTo"
        );
    }

    #[test]
    fn real_tutorial06_elevator_rider_view_target_and_camera_stay_continuous() {
        // This is the short form of the real Tutorial06 app route below: use
        // its shipped CLNK/ELEV/ELEC definitions and the normal app snapshot
        // -> viewport -> renderer path, while opening only the test shaft so
        // the carriage can run for a small deterministic frame window.
        let mut app = real_tutorial_app(6, "Tutorial 6 elevator camera");
        let owner = app.local_owner;
        let rider = app
            .engine
            .crew_cursor(owner)
            .expect("Tutorial06 starts with a selected CLNK");
        advance_app_until(
            &mut app,
            "Tutorial06 selected CLNK completes its startup Exit",
            160,
            |app| {
                app.engine.object_snapshot(rider).is_some_and(|object| {
                    object.container.is_none() && object.action.name == "Walk"
                })
            },
        );

        app.engine
            .execute_shake_circle_operation(Vector2::new(332, 250), 180);
        let elevator = app
            .engine
            .spawn_object(
                SpawnConfig::new("ELEV")
                    .with_position(Vector2::new(332, 150))
                    .with_owner(owner),
            )
            .expect("spawn shipped Tutorial06 ELEV");
        let first = app.engine.snapshot();
        let elevator = first.object(elevator).expect("ELEV survives Initialize");
        let case_id = elevator
            .action
            .target
            .expect("real ELEV Initialize creates and targets ELEC");
        let case = first.object(case_id).expect("real ELEC exists");
        assert_eq!(case.definition_id, "ELEC");

        // CLNK's bottom vertex is y+9 and ELEC's shipped mask begins at
        // case y+11. Put the selected crew exactly on that platform and use
        // the real PUSH action target. C4SolidMask then carries it by every
        // case delta before its own movement pass (C4SolidMask.cpp:178-195,
        // 276-305), just as in the full physical-key route.
        let rider_offset = Vector2::new(0, 2);
        let rider_action = clonk_engine::ActionUpdate::default()
            .with_name("Push")
            .with_target(Some(case_id));
        app.engine
            .apply_object_update(
                rider,
                ObjectUpdate::new()
                    .with_position(Vector2::new(
                        case.position.x + rider_offset.x,
                        case.position.y + rider_offset.y,
                    ))
                    .with_velocity(Vector2::ZERO)
                    .with_command_direction(CommandDirection::Stop)
                    .with_action_update(rider_action),
            )
            .expect("attach selected CLNK to real ELEC");
        // Wait is the real ELEC FLOAT action. A downward comdir plus an
        // initial live velocity exercises ordinary fixed-point movement and
        // solid-mask rider restoration without invoking a test-only mover.
        app.engine
            .apply_object_update(
                case_id,
                ObjectUpdate::new()
                    .with_action("Wait")
                    .with_velocity(Vector2::new(0, 1))
                    .with_command_direction(CommandDirection::Down),
            )
            .expect("start real ELEC moving down the opened shaft");

        // The setup mutations above stand in for the object phase. C++
        // copies the selected ViewCursor position into ViewX/ViewY in the
        // later player phase (C4Player.cpp:200-209,1693-1713).
        app.engine
            .tick_player_systems()
            .expect("refresh rider view after fixture setup");

        app.focus_id = Some(rider);
        app.snapshot = app.engine.snapshot();
        app.refresh_focus();
        let initial_snapshot = app.snapshot.clone();
        let initial_inputs = collect_viewport_inputs(&initial_snapshot)
            .expect("real Tutorial06 player has an authoritative viewport");
        assert_eq!(initial_inputs.len(), 1);
        assert_eq!(
            initial_inputs[0]
                .focus
                .expect("player viewport focus")
                .id,
            rider
        );
        assert_eq!(
            initial_inputs[0].center,
            app.snapshot.object(rider).expect("initial rider").position,
            "C4Player::UpdateView follows the live ViewCursor position"
        );
        app.graphics
            .render_frame(&initial_snapshot, &initial_inputs);

        let initial_case = app
            .snapshot
            .object(case_id)
            .expect("initial moving ELEC")
            .position;
        let initial_rider = app
            .snapshot
            .object(rider)
            .expect("initial attached CLNK")
            .position;
        let initial_world_origin = app
            .graphics
            .world_to_screen(owner, Vector2::ZERO)
            .expect("initial viewport maps world origin")
            .1;
        let initial_rider_screen = app
            .graphics
            .world_to_screen(owner, initial_rider)
            .expect("initial viewport maps rider")
            .1;
        let mut samples = vec![(
            initial_case.y,
            initial_rider.y,
            initial_world_origin,
            initial_rider_screen,
        )];

        for frame in 1..=12 {
            app.update()
                .unwrap_or_else(|error| panic!("advance elevator frame {frame}: {error}"));
            let case = app
                .snapshot
                .object(case_id)
                .unwrap_or_else(|| panic!("ELEC survives frame {frame}"))
                .clone();
            let rider_now = app
                .snapshot
                .object(rider)
                .unwrap_or_else(|| panic!("CLNK survives frame {frame}"))
                .clone();
            assert_eq!(
                (rider_now.action.name.as_str(), rider_now.action.target),
                ("Push", Some(case_id)),
                "real PUSH attachment survives frame {frame}"
            );
            assert!(
                (rider_now.position.y - case.position.y - rider_offset.y).abs() <= 1,
                "rider and carriage cannot diverge on frame {frame}: rider={rider_now:?}, case={case:?}"
            );

            let render_snapshot = app.snapshot.clone();
            let inputs = collect_viewport_inputs(&render_snapshot)
                .expect("real Tutorial06 player keeps an authoritative viewport");
            assert_eq!(inputs.len(), 1, "one local viewport on frame {frame}");
            assert_eq!(
                inputs[0].focus.expect("player viewport focus").id,
                rider
            );
            assert_eq!(
                inputs[0].center, rider_now.position,
                "the app must present the rider's current frame position to C4Viewport on frame {frame}"
            );
            app.graphics.render_frame(&render_snapshot, &inputs);
            let world_origin = app
                .graphics
                .world_to_screen(owner, Vector2::ZERO)
                .unwrap_or_else(|| panic!("viewport maps world origin on frame {frame}"))
                .1;
            let rider_screen = app
                .graphics
                .world_to_screen(owner, rider_now.position)
                .unwrap_or_else(|| panic!("viewport maps rider on frame {frame}"))
                .1;
            samples.push((
                case.position.y,
                rider_now.position.y,
                world_origin,
                rider_screen,
            ));
        }

        assert!(
            samples.last().expect("final sample").0 > samples[0].0,
            "the real ELEC must move during the sample: {samples:?}"
        );
        for pair in samples.windows(2) {
            let [before, after] = pair else {
                unreachable!()
            };
            assert!(
                after.0 >= before.0 && after.1 >= before.1,
                "carriage/rider reversed between frames: {before:?} -> {after:?}"
            );
            assert!(
                after.2 <= before.2,
                "the fixed-point C4Viewport camera reversed between frames: {before:?} -> {after:?}"
            );
            assert!(
                after.3 >= before.3,
                "the rider jittered backwards on screen: {before:?} -> {after:?}"
            );
        }
    }

    #[test]
    fn overlay_text_helper_respects_custom_text() {
        assert!(overlay_text_needs_update("", "FRAME "));
        assert!(overlay_text_needs_update("FRAME 00005", "FRAME "));
        assert!(!overlay_text_needs_update("Inventory open", "FRAME "));

        assert!(overlay_text_needs_update("", "ENERGY "));
        assert!(overlay_text_needs_update(
            "ENERGY 100 DAMAGE 000 OWNER 1",
            "ENERGY "
        ));
        assert!(!overlay_text_needs_update("Paused", "ENERGY "));

        assert_eq!(
            c4_presentation_text(&clonk_script::c4_string_from_bytes(&[0xe9])),
            "\u{e9}"
        );

        let raw_name = clonk_script::c4_string_from_bytes(&[0xe9]);
        assert_eq!(player_join_board_line(&raw_name), "Player join: \u{e9}");
    }

    #[test]
    fn real_tutorial01_message_render_subcases_batch() {
        let prepared =
            PreparedRealInstalledScenario::new("Tutorial.c4f/Tutorial01.c4s");
        let mut failures = Vec::new();
        run_real_tutorial01_app_subcase(
            "renders_cpp_decorated_portrait_message",
            &mut failures,
            || real_tutorial01_renders_cpp_decorated_portrait_message(&prepared),
        );
        run_real_tutorial01_app_subcase(
            "scale_three_message_commits_native_pixels_after_filtered_base",
            &mut failures,
            || scale_three_tutorial_message_commits_native_pixels_after_filtered_base(&prepared),
        );
        assert_no_real_tutorial01_app_subcase_failures(failures);
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

    fn real_tutorial01_renders_cpp_decorated_portrait_message(
        prepared: &PreparedRealInstalledScenario,
    ) {
        // TutorialMessage reaches C4GameMessage::Draw as a permanent
        // player-global message with DECO framing and an SCLK portrait
        // (Tutorial.c4f/System.c4g/Tutorial.c:22-31;
        // src/C4GameMessage.cpp:99-170).
        let _lock = env_lock().lock();
        let mut app = prepared.instantiate("Tutorial message parity", true);
        advance_app_until(&mut app, "Tutorial01 welcome message", 180, |app| {
            app_tutorial_message_contains(app, "Welcome to the world of Clonk.")
        });
        let message = app
            .snapshot
            .hud
            .messages
            .iter()
            .find(|message| {
                message
                    .lines
                    .iter()
                    .any(|line| line == "Welcome to the world of Clonk.")
            })
            .expect("shipped Tutorial01 welcome message")
            .clone();
        assert_eq!(message.kind, MessageKind::GlobalPlayer);
        assert_eq!(message.player, Some(app.local_owner));
        assert_eq!(message.target, None);
        assert_eq!(message.lines, ["Welcome to the world of Clonk."]);
        assert_eq!(message.offset, Vector2::new(50, 50));
        assert_eq!(message.color, 0xffff_ffff);
        assert_eq!(message.flags, 0x718);
        assert_eq!(message.width, Some(30));
        assert_eq!(message.decoration.as_deref(), Some("DECO"));
        assert_eq!(
            message.portrait.as_deref(),
            Some("Portrait:SCLK::0000ff::1")
        );

        let decoration = message
            .frame_decoration
            .as_ref()
            .expect("C4GameMessage snapshots DECO at creation");
        assert_eq!(decoration.source_definition, "DECO");
        assert_eq!(decoration.background_color, 0x8032_3232);
        assert_eq!(
            (
                decoration.border_top,
                decoration.border_left,
                decoration.border_right,
                decoration.border_bottom,
            ),
            (0, 0, 0, 0)
        );
        let facets = [
            decoration.top_left.as_ref(),
            decoration.top.as_ref(),
            decoration.top_right.as_ref(),
            decoration.right.as_ref(),
            decoration.bottom_right.as_ref(),
            decoration.bottom.as_ref(),
            decoration.bottom_left.as_ref(),
            decoration.left.as_ref(),
        ]
        .map(|facet| {
            let facet = facet.expect("Tutorial01 DECO contains all eight frame facets");
            (
                facet.x,
                facet.y,
                facet.width,
                facet.height,
                facet.target_x,
                facet.target_y,
            )
        });
        assert_eq!(
            facets,
            [
                (0, 0, 16, 16, -8, -7),
                (16, 0, 58, 12, 0, -7),
                (74, 0, 16, 16, -7, -7),
                (74, 16, 16, 58, -7, 0),
                (74, 74, 16, 16, -7, -8),
                (16, 76, 58, 16, 0, -6),
                (0, 74, 16, 16, -8, -8),
                (0, 16, 16, 58, -8, 0),
            ]
        );

        app.resize(1152, 644)
            .expect("resize to the reported logical surface");
        hold_message_board_for_frame_comparison(&mut app);
        let messages = std::mem::take(&mut app.snapshot.hud.messages);
        let mut warm = vec![0_u8; 1152 * 644 * 4];
        app.render(&mut warm)
            .expect("warm the message-free presentation state");
        let frame_gamma = app
            .graphics
            .active_gamma_ramp(&app.snapshot.environment.gamma);
        let mut baseline = vec![0_u8; 1152 * 644 * 4];
        app.render(&mut baseline)
            .expect("render the message-free Tutorial01 baseline");
        app.snapshot.hud.messages = messages;
        let mut rendered = vec![0_u8; 1152 * 644 * 4];
        app.render(&mut rendered)
            .expect("classic Tutorial01 C4GameMessage renders");

        let viewport = app
            .graphics
            .active_viewport_projections()
            .into_iter()
            .find(|viewport| viewport.owner == app.local_owner)
            .expect("local Tutorial01 viewport")
            .rect;
        assert_eq!(viewport, Rect::new(216, 56, 720, 560));
        let fonts = app
            .assets
            .clonk_fonts
            .as_deref()
            .expect("classic FontRegular");
        assert_eq!(
            fonts.text.measure("Welcome to the world of Clonk.", true),
            (194, 22)
        );

        let core_frame = Rect::new(576, 106, 278, 64);
        let deco_envelope = Rect::new(568, 99, 295, 81);
        let inside = |rect: Rect, x: i32, y: i32| {
            x >= rect.x
                && x < rect.x + rect.width as i32
                && y >= rect.y
                && y < rect.y + rect.height as i32
        };
        let changed = rendered
            .chunks_exact(4)
            .zip(baseline.chunks_exact(4))
            .enumerate()
            .filter_map(|(index, (actual, before))| (actual != before).then_some(index))
            .collect::<Vec<_>>();
        assert!(!changed.is_empty(), "the C4GameMessage contributes pixels");
        assert!(changed.iter().all(|index| {
            let x = (*index % 1152) as i32;
            let y = (*index / 1152) as i32;
            inside(viewport, x, y) && inside(deco_envelope, x, y)
        }));
        assert!(
            changed.iter().any(|index| {
                let x = (*index % 1152) as i32;
                let y = (*index / 1152) as i32;
                !inside(core_frame, x, y)
            }),
            "real DECO facets extend outside the core frame"
        );

        let pixel = |frame: &[u8], x: usize, y: usize| {
            let offset = (y * 1152 + x) * 4;
            Color::new(
                frame[offset],
                frame[offset + 1],
                frame[offset + 2],
                frame[offset + 3],
            )
        };
        assert_eq!(
            pixel(&rendered, 572, 100),
            clonk_frontend::gamma_encode_fragment(Color::opaque(126, 66, 23), &frame_gamma),
            "the opaque top-left DECO texel must draw outside the core frame"
        );

        let mut expected_gap = Surface::new(1, 1, clonk_graphics::PixelFormat::Rgba8888);
        expected_gap
            .set_pixel(0, 0, pixel(&baseline, 645, 130))
            .expect("seed the gap background");
        clonk_frontend::classic_gui::draw_engine_box(
            &mut expected_gap,
            0,
            0,
            0,
            0,
            0x8032_3232,
            Some(&frame_gamma),
        );
        assert_eq!(
            pixel(&rendered, 645, 130),
            expected_gap.get_pixel(0, 0).expect("blended gap pixel"),
            "the ten-pixel portrait/text gap contains only DECO background"
        );
    }

    fn scale_three_tutorial_message_commits_native_pixels_after_filtered_base(
        prepared: &PreparedRealInstalledScenario,
    ) {
        // FontRegular is rebuilt with Application.GetScale(), but its public
        // geometry remains in GUI units. Ordinary frame/portrait pixels pass
        // through GL_LINEAR first and native glyphs are then drawn into the
        // physical viewport (C4Fonts.cpp:158-173; StdFont.cpp:319-352,841-842;
        // C4Viewport.cpp:852-854).
        let _lock = env_lock().lock();
        let mut app = prepared.instantiate("Native tutorial message parity", true);
        advance_app_until(&mut app, "Tutorial01 welcome message", 180, |app| {
            app_tutorial_message_contains(app, "Welcome to the world of Clonk.")
        });
        app.configure_native_startup_fonts(3.0, false);
        assert!(app.can_defer_native_game_messages(3.0));

        let gamma = app
            .graphics
            .active_gamma_ramp(&app.snapshot.environment.gamma);
        let mut presenter = clonk_scaling::FramePresenter::new(3.0, 960, 598);
        let mut output = vec![0_u8; 960 * 598 * 4];
        let refreshed = presenter
            .present(&mut output, |frame| {
                app.render_for_presentation(frame, false, false, true)
            })
            .expect("render filtered base before Tutorial01 message");
        assert!(refreshed);
        let filtered_base = output.clone();

        app.render_native_game_messages(&mut output, presenter.presentation_geometry(), &gamma)
            .expect("render native Tutorial01 message text");
        assert_ne!(
            output, filtered_base,
            "the physical C4GameMessage pass must contribute message pixels"
        );

        // A 320x200 logical surface creates a nominal 960x600 lower-left GL
        // viewport in a 960x598 framebuffer, clipping two physical rows from
        // the top. Native message pixels must retain that offset and the
        // owning C4Viewport clip.
        let viewport = app
            .graphics
            .active_viewport_projections()
            .into_iter()
            .find(|viewport| viewport.owner == app.local_owner)
            .expect("local Tutorial01 viewport")
            .rect;
        let physical_viewport = Rect::new(
            viewport.x * 3,
            viewport.y * 3 - 2,
            viewport.width * 3,
            viewport.height * 3,
        );
        let changed = output
            .chunks_exact(4)
            .zip(filtered_base.chunks_exact(4))
            .enumerate()
            .filter_map(|(index, (native, base))| (native != base).then_some(index));
        let mut changed_count = 0;
        for index in changed {
            changed_count += 1;
            let point = Rect::new((index % 960) as i32, (index / 960) as i32, 1, 1);
            assert!(
                physical_viewport.intersection(point).is_some(),
                "native message pixel ({}, {}) escaped its viewport clip",
                point.x,
                point.y
            );
        }
        assert!(changed_count > 0);

        let solid = [17_u8, 29, 43, 255];
        let mut nominal = solid
            .into_iter()
            .cycle()
            .take(960 * 600 * 4)
            .collect::<Vec<_>>();
        let mut clipped = solid
            .into_iter()
            .cycle()
            .take(960 * 598 * 4)
            .collect::<Vec<_>>();
        let nominal_geometry =
            clonk_scaling::FramePresenter::new(3.0, 960, 600).presentation_geometry();
        let clipped_geometry =
            clonk_scaling::FramePresenter::new(3.0, 960, 598).presentation_geometry();
        app.render_native_game_messages(&mut nominal, nominal_geometry, &gamma)
            .expect("render nominal native-message probe");
        app.render_native_game_messages(&mut clipped, clipped_geometry, &gamma)
            .expect("render clipped native-message probe");
        for y in 0..598_usize {
            let clipped_row = &clipped[y * 960 * 4..(y + 1) * 960 * 4];
            let nominal_row = &nominal[(y + 2) * 960 * 4..(y + 3) * 960 * 4];
            assert_eq!(
                clipped_row,
                nominal_row,
                "the 598-row framebuffer must clip nominal physical row {}",
                y + 2
            );
        }
    }

    #[test]
    fn real_tutorial09_hud_names_subcases_batch() {
        let prepared =
            PreparedRealInstalledScenario::new("Tutorial.c4f/Tutorial09.c4s");
        let mut failures = Vec::new();
        run_real_tutorial09_app_subcase(
            "temporary_breath_physical_renders_the_cpp_hud_bar",
            &mut failures,
            || tutorial09_real_temporary_breath_physical_renders_the_cpp_hud_bar(&prepared),
        );
        run_real_tutorial09_app_subcase(
            "system_names_preserve_cpp_ready_conkit_route",
            &mut failures,
            || app_tutorial09_system_names_preserve_cpp_ready_conkit_route(&prepared),
        );
        assert_no_real_tutorial09_app_subcase_failures(failures);
    }

    fn run_real_tutorial09_app_subcase(
        name: &'static str,
        failures: &mut Vec<&'static str>,
        subcase: impl FnOnce(),
    ) {
        eprintln!("running Tutorial09 app subcase `{name}`");
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(subcase)).is_err() {
            eprintln!("Tutorial09 app subcase `{name}` failed; continuing batch");
            failures.push(name);
        }
    }

    fn assert_no_real_tutorial09_app_subcase_failures(failures: Vec<&str>) {
        assert!(
            failures.is_empty(),
            "Tutorial09 app subcase(s) failed: {}",
            failures.join(", ")
        );
    }

    fn tutorial09_real_temporary_breath_physical_renders_the_cpp_hud_bar(
        prepared: &PreparedRealInstalledScenario,
    ) {
        // Tutorial09 raises the ready CLNK's temporary Breath physical to
        // 250000 without rewriting its current 50000 breath
        // (Tutorial09.c4s/Script.c:18-23; C4Script.cpp:584-598;
        // C4Object.cpp:192-195). C4Viewport therefore draws the cyan breath
        // bar because 0 < Breath < GetPhysical()->Breath
        // (C4Viewport.cpp:920-943; C4Object.cpp:2728-2731).
        let mut app = prepared.instantiate("Breath HUD parity", false);
        wait_for_running(&mut app);
        app.update().expect("Tutorial09 first running frame");

        let clonk = app
            .snapshot
            .players
            .iter()
            .find(|player| player.id == app.local_owner)
            .and_then(|player| player.cursor)
            .expect("Tutorial09 local cursor CLNK");
        let object = app
            .snapshot
            .object(clonk)
            .expect("Tutorial09 cursor remains in the snapshot");
        let current_breath = object.breath;
        let capacity = app
            .engine
            .find_object_index(clonk)
            .map(|index| app.engine.object_physical(index).breath)
            .expect("Tutorial09 cursor has resolved physicals");
        assert_eq!(current_breath, 50_000, "CLNK keeps its birth breath");
        assert_eq!(capacity, 250_000, "Tutorial09 installs AquaClonk capacity");

        let overlays = {
            let game_app = &mut app.app;
            collect_player_overlays(
                &mut game_app.engine,
                &game_app.snapshot,
                Some(clonk),
                &game_app.bindings,
                &game_app.gamepad_bindings,
            )
        };
        let crew = overlays
            .iter()
            .find(|player| player.owner == app.local_owner)
            .and_then(|player| player.crew.iter().find(|crew| crew.object_id == clonk))
            .expect("Tutorial09 cursor reaches the HUD overlay");
        assert_eq!(crew.breath, 50_000);
        assert_eq!(crew.breath_capacity, 250_000);
        assert!(crew.breath != 0 && crew.breath < crew.breath_capacity);

        hold_message_board_for_frame_comparison(&mut app);

        // The stock EnergyBars.png is split into six 8px columns and three
        // 12px cap/tile rows (C4GraphicsResource.cpp:231-241). With portraits
        // enabled, an energy bar already occupying slot zero, and no magic,
        // the breath bar occupies x=5+(8+1), y=35+10+10, h=200-95. Its
        // filled pixels come from cyan columns 4/5 selected by bar_idx=2
        // (C4Facet.cpp:334-387).
        let hud = app.graphics.hud_graphics();
        let bars = hud.energy_bars.as_ref().expect("stock EnergyBars.png");
        assert_eq!((bars.width(), bars.height()), (48, 36));
        let mut surface = Surface::new(320, 200, PixelFormat::Rgba8888);
        clonk_frontend::hud::draw_level_bar(
            &mut surface,
            &hud,
            clonk_graphics::Rect::new(0, 0, 320, 200),
            clonk_frontend::hud::HudBarKind::Breath,
            1,
            crew.breath,
            crew.breath_capacity,
            true,
        );

        let painted = surface
            .pixels()
            .chunks_exact(4)
            .enumerate()
            .filter(|(_, pixel)| pixel[3] != 0)
            .map(|(index, pixel)| {
                (
                    (index % 320) as i32,
                    (index / 320) as i32,
                    [pixel[0], pixel[1], pixel[2]],
                )
            })
            .collect::<Vec<_>>();
        assert!(!painted.is_empty(), "real cyan breath asset draws pixels");
        assert!(painted
            .iter()
            .all(|(x, y, _)| (14..22).contains(x) && (55..160).contains(y)));
        assert!(
            painted.iter().any(|(_, y, [r, g, b])| *y >= 139
                && *g > r.saturating_add(20)
                && *b > r.saturating_add(20)),
            "the lower 20% uses the stock cyan filled breath column"
        );

        // Exercise the complete GameApp -> GraphicsOverlay -> render_frame
        // seam with the real scenario and graphics. Setting current breath to
        // capacity suppresses only C++'s `Breath < GetPhysical()->Breath`
        // predicate; restoring 50000 must add fragments exclusively inside
        // the compact second bar slot (C4Viewport.cpp:924-943).
        let mut frame = vec![0; app.graphics.surface().pixels().len()];
        app.snapshot
            .objects
            .iter_mut()
            .find(|object| object.id == clonk)
            .expect("Tutorial09 cursor remains mutable")
            .breath = capacity;
        app.render_running(&mut frame, false)
            .expect("render Tutorial09 with full breath");
        app.render_running(&mut frame, false)
            .expect("stabilize Tutorial09 full-breath frame");
        let without_breath = frame.clone();

        app.snapshot
            .objects
            .iter_mut()
            .find(|object| object.id == clonk)
            .expect("Tutorial09 cursor remains mutable")
            .breath = current_breath;
        app.render_running(&mut frame, false)
            .expect("render Tutorial09 with partial breath");
        let with_breath = frame.clone();

        app.snapshot
            .objects
            .iter_mut()
            .find(|object| object.id == clonk)
            .expect("Tutorial09 cursor remains mutable")
            .breath = capacity;
        app.render_running(&mut frame, false)
            .expect("render Tutorial09 with breath suppressed again");
        assert_eq!(
            frame, without_breath,
            "the stationary real frame is otherwise deterministic"
        );

        let viewport = app
            .graphics
            .viewport_rect(app.local_owner)
            .expect("Tutorial09 local viewport");
        let bar_x = viewport.x + 14;
        let bar_y = viewport.y + 55;
        let bar_height = viewport.height as i32 - 95;
        assert!(bar_height > 0, "C++ viewport height gate permits HUD bars");
        let fill_y = bar_y + bar_height - current_breath * bar_height / capacity;
        let changed = with_breath
            .chunks_exact(4)
            .zip(without_breath.chunks_exact(4))
            .enumerate()
            .filter(|(_, (with, without))| with != without)
            .map(|(index, (pixel, _))| {
                (
                    (index % 320) as i32,
                    (index / 320) as i32,
                    [pixel[0], pixel[1], pixel[2]],
                )
            })
            .collect::<Vec<_>>();
        assert!(
            !changed.is_empty(),
            "partial real Tutorial09 breath paints the HUD"
        );
        assert!(
            changed.iter().all(|(x, y, _)| {
                (bar_x..bar_x + 8).contains(x) && (bar_y..bar_y + bar_height).contains(y)
            }),
            "breath-only fragments stay inside the C++ bar rectangle: {changed:?}"
        );
        assert!(
            changed.iter().any(|(_, y, _)| *y < fill_y),
            "the empty breath source column paints above yBar"
        );
        assert!(
            changed.iter().any(|(_, y, [r, g, b])| {
                *y >= fill_y && *g > r.saturating_add(10) && *b > r.saturating_add(10)
            }),
            "the cyan filled source column paints at and below yBar"
        );
    }

    #[test]
    fn app_virtual_keyboard_routes_cpp_player_one_keys_without_arrow_aliases() {
        // C++ keyboard set one maps movement to S/Z/X/C and does not alias
        // the arrow keys (C4Config.cpp:624-635). Exercise those physical
        // keys through GameApp rather than injecting logical ControlEvents.
        let mut app = new_running_sandbox_app();
        let mut keyboard = AppVirtualKeyboard::new(&mut app);
        let auto_stop = keyboard.player_control().control_style;

        for (key, com) in [
            (VirtualKeyCode::S, clonk_engine::COM_UP),
            (VirtualKeyCode::Z, clonk_engine::COM_LEFT),
            (VirtualKeyCode::X, clonk_engine::COM_DOWN),
            (VirtualKeyCode::C, clonk_engine::COM_RIGHT),
        ] {
            keyboard.press(key).expect("physical key press");
            assert_ne!(
                keyboard.player_control().pressed_coms & (1 << com),
                0,
                "{key:?} must reach the matching C4Player::InCom bit"
            );
            keyboard.release(key).expect("physical key release");
            assert_eq!(
                keyboard.player_control().pressed_coms & (1 << com) == 0,
                auto_stop,
                "{key:?} release must follow the local player's control style"
            );
        }

        let before_arrows = keyboard.player_control();
        for key in [
            VirtualKeyCode::Up,
            VirtualKeyCode::Left,
            VirtualKeyCode::Down,
            VirtualKeyCode::Right,
        ] {
            keyboard.press(key).expect("unbound arrow press");
            keyboard.release(key).expect("unbound arrow release");
        }
        assert_eq!(keyboard.player_control(), before_arrows);
    }

    #[test]
    fn app_virtual_keyboard_completes_real_tutorial01_route() {
        // Drive Tutorial01 through the same physical keyboard-one boundary as
        // C++: A/S/D/Z/X/C are Throw/Up/Dig/Left/Down/Right
        // (C4Config.cpp:624-635). The complete real script requires FLAG
        // delivery through HUT2's context menu, buffered DigSingle plus live
        // DownLeft/Left steering to GOLD, and a physical return climb before
        // SCRG fulfills (Tutorial01/Script.c:61-182; C4Player.cpp:1213-1229,
        // 1490-1554; C4Object.cpp:3618-3628,3645-3651,3743-3754).
        let mut app = real_tutorial_app(1, "Tutorial 1 app virtual player");
        let clonk = app
            .engine
            .crew_cursor(app.local_owner)
            .expect("Tutorial01 selected CLNK");
        let hut = app_object_with_definition(&app, "HUT2").expect("Tutorial01 HUT2");

        advance_app_until(
            &mut app,
            "Tutorial01 CLNK lands in the valley",
            180,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.action.name == "Walk")
            },
        );
        advance_app_until(
            &mut app,
            "Tutorial01 creates FLAG and points left",
            500,
            |app| {
                app_object_with_definition(app, "FLAG").is_some()
                    && app_tutorial_message_contains(app, "hill to your left")
            },
        );
        let flag = app_object_with_definition(&app, "FLAG").expect("Tutorial01 FLAG");

        // Held Z supplies horizontal jump momentum. Each physical S tap is
        // separated by twelve app ticks, beyond C4DoubleClick's ten-tick
        // window, and its release must preserve the still-held Z bit.
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .press(VirtualKeyCode::Z)
                .expect("physical Z toward FLAG");
        }
        for _ in 0..30 {
            let clonk_now = app
                .engine
                .object_snapshot(clonk)
                .expect("CLNK survives left-hill route");
            if app_clonk_carries(&app, clonk, "FLAG") || clonk_now.position.x <= 25 {
                break;
            }
            if clonk_now.action.name == "Walk" {
                let mut keyboard = AppVirtualKeyboard::new(&mut app);
                keyboard
                    .tap(VirtualKeyCode::S)
                    .expect("physical S jumps toward FLAG");
                assert_ne!(
                    keyboard.player_control().pressed_coms & (1 << clonk_engine::COM_LEFT),
                    0,
                    "releasing S must preserve held Z/Left"
                );
            }
            for _ in 0..12 {
                app.update().expect("advance left-hill jump");
            }
        }
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .release(VirtualKeyCode::Z)
                .expect("release physical Z at FLAG");
        }
        if !app_clonk_carries(&app, clonk, "FLAG") {
            advance_app_until(&mut app, "CLNK lands beside FLAG", 80, |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.action.name == "Walk" && object.position.x <= 40)
            });
            {
                let mut keyboard = AppVirtualKeyboard::new(&mut app);
                keyboard
                    .press(VirtualKeyCode::C)
                    .expect("physical C collects FLAG");
            }
            advance_app_until(&mut app, "CLNK naturally collects FLAG", 40, |app| {
                app_clonk_carries(app, clonk, "FLAG")
            });
            AppVirtualKeyboard::new(&mut app)
                .release(VirtualKeyCode::C)
                .expect("release physical C after FLAG pickup");
        }
        assert_eq!(
            app.engine
                .object_snapshot(flag)
                .expect("collected FLAG")
                .container,
            Some(clonk)
        );
        assert!(
            app_cursor_inventory_contains(&mut app, clonk, "FLAG"),
            "the collected FLAG must reach the rendered cursor inventory"
        );
        app.snapshot.hud.messages.clear();
        let mut rendered = vec![0_u8; 320 * 200 * 4];
        app.render(&mut rendered)
            .expect("render Tutorial01 with FLAG inventory");

        advance_app_until(&mut app, "Tutorial01 points toward the cabin", 500, |app| {
            app_tutorial_message_contains(app, "cabin on the hill to your right")
        });
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .press(VirtualKeyCode::C)
                .expect("physical C toward HUT2");
        }
        for _ in 0..90 {
            let clonk_now = app
                .engine
                .object_snapshot(clonk)
                .expect("CLNK survives cabin route");
            if clonk_now.position.x >= 558 {
                break;
            }
            if clonk_now.action.name == "Walk" {
                let mut keyboard = AppVirtualKeyboard::new(&mut app);
                keyboard
                    .tap(VirtualKeyCode::S)
                    .expect("physical S jumps toward HUT2");
                assert_ne!(
                    keyboard.player_control().pressed_coms & (1 << clonk_engine::COM_RIGHT),
                    0,
                    "releasing S must preserve held C/Right"
                );
            }
            for _ in 0..12 {
                app.update().expect("advance cabin jump");
            }
        }
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::C)
            .expect("release physical C beside HUT2");
        advance_app_until(&mut app, "CLNK lands beside HUT2", 60, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk")
        });
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::Z)
            .expect("physical Z aligns with HUT2 entrance");
        advance_app_until(&mut app, "CLNK aligns with HUT2 entrance", 20, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.x <= 570)
        });
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .release(VirtualKeyCode::Z)
                .expect("release physical Z at HUT2 entrance");
            keyboard
                .tap(VirtualKeyCode::S)
                .expect("physical S enters HUT2");
        }
        advance_app_until(&mut app, "FLAG-carrying CLNK enters HUT2", 40, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.container == Some(hut))
        });

        // C++ inserts Put first while the contained CLNK carries FLAG
        // (C4ObjectMenu.cpp:335-359). Physical A becomes MenuEnter rather than
        // a world Throw while this cursor menu is active.
        advance_app_until(&mut app, "HUT2 context Put menu", 20, |app| {
            app.engine
                .cursor_object_menu(app.local_owner)
                .is_some_and(|(_, menu)| {
                    menu.selection == 0
                        && menu.items.first().is_some_and(|item| item.caption == "Put")
                })
        });
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::A)
            .expect("physical A puts FLAG into HUT2");
        advance_app_until(&mut app, "FLAG enters HUT2", 80, |app| {
            app.engine
                .object_snapshot(flag)
                .is_some_and(|object| object.container == Some(hut))
        });
        advance_app_until(&mut app, "FLAG makes HUT2 the player base", 80, |app| {
            app.engine
                .object_snapshot(hut)
                .is_some_and(|object| object.base == app.local_owner)
        });
        advance_app_until(
            &mut app,
            "Tutorial01 Exit prompt and context row",
            450,
            |app| {
                app_tutorial_message_contains(app, "select 'Exit'")
                    && app
                        .engine
                        .cursor_object_menu(app.local_owner)
                        .is_some_and(|(_, menu)| {
                            menu.items.iter().any(|item| item.caption == "Exit")
                        })
            },
        );

        // Script148 highlights physical X/Down plus A. Move down through the
        // real context rows, including any Buy/Sell rows enabled by the base,
        // rather than selecting Exit by index or mutating menu state.
        let context_items = app
            .engine
            .cursor_object_menu(app.local_owner)
            .expect("HUT2 context with Exit")
            .1
            .items
            .len();
        for _ in 0..=context_items {
            let exit_selected = app
                .engine
                .cursor_object_menu(app.local_owner)
                .and_then(|(_, menu)| {
                    usize::try_from(menu.selection)
                        .ok()
                        .map(|index| (menu, index))
                })
                .and_then(|(menu, index)| menu.items.get(index))
                .is_some_and(|item| item.caption == "Exit");
            if exit_selected {
                break;
            }
            AppVirtualKeyboard::new(&mut app)
                .tap(VirtualKeyCode::X)
                .expect("physical X navigates toward Exit");
        }
        assert!(
            app.engine
                .cursor_object_menu(app.local_owner)
                .and_then(|(_, menu)| usize::try_from(menu.selection)
                    .ok()
                    .map(|index| (menu, index)))
                .and_then(|(menu, index)| menu.items.get(index))
                .is_some_and(|item| item.caption == "Exit"),
            "physical X must select the real Exit row"
        );
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::A)
            .expect("physical A activates Exit");
        advance_app_until(&mut app, "CLNK exits HUT2", 60, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.container.is_none())
        });

        advance_app_until(
            &mut app,
            "Tutorial01 creates GOLD and sends CLNK to the valley",
            120,
            |app| {
                app_object_with_definition(app, "GOLD").is_some()
                    && app_tutorial_message_contains(app, "back into the valley")
            },
        );
        let gold = app_object_with_definition(&app, "GOLD").expect("Tutorial01 GOLD");
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::Z)
            .expect("physical Z returns to the lesson valley");
        advance_app_until(
            &mut app,
            "CLNK reaches the digging lesson area",
            260,
            |app| {
                app.engine.object_snapshot(clonk).is_some_and(|object| {
                    (150..250).contains(&object.position.x)
                        && (250..350).contains(&object.position.y)
                })
            },
        );
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::Z)
            .expect("release physical Z in lesson valley");
        advance_app_until(&mut app, "Tutorial01 enables digging", 160, |app| {
            app_tutorial_message_contains(app, "start a digging process")
                && app
                    .engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.temporary_physical.is_none())
        });

        // D is buffered until C4DoubleClick (10) expires. Do not press X/Z
        // early: C4Player::InCom would flush the pending DigSingle immediately
        // on a different press (C4Player.cpp:1522-1536).
        let dig_press_frame = app.engine.frame();
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::D)
            .expect("physical D starts buffered DigSingle");
        advance_app_until(&mut app, "CLNK starts real Dig action", 30, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Dig")
        });
        assert!(
            app.engine.frame().saturating_sub(dig_press_frame) > 10,
            "physical D must wait through C4DoubleClick before DigSingle"
        );
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .press(VirtualKeyCode::X)
                .expect("physical X steers Dig down");
            keyboard
                .press(VirtualKeyCode::Z)
                .expect("physical Z adds leftward Dig steering");
            let control = keyboard.player_control();
            assert_ne!(control.pressed_coms & (1 << clonk_engine::COM_DOWN), 0);
            assert_ne!(control.pressed_coms & (1 << clonk_engine::COM_LEFT), 0);
            assert_eq!(
                keyboard
                    .engine()
                    .object_snapshot(clonk)
                    .expect("CLNK after X+Z")
                    .command_direction,
                CommandDirection::DownLeft
            );
        }
        advance_app_until(&mut app, "diagonal Dig reaches GOLD depth", 140, |app| {
            app_clonk_carries(app, clonk, "GOLD")
                || app
                    .engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.position.y >= 320)
        });
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .release(VirtualKeyCode::X)
                .expect("release physical X while Z remains held");
            let control = keyboard.player_control();
            assert_eq!(control.pressed_coms & (1 << clonk_engine::COM_DOWN), 0);
            assert_ne!(control.pressed_coms & (1 << clonk_engine::COM_LEFT), 0);
            let clonk_now = keyboard
                .engine()
                .object_snapshot(clonk)
                .expect("CLNK after partial Dig release");
            assert_eq!(clonk_now.action.name, "Dig");
            assert_eq!(clonk_now.command_direction, CommandDirection::Left);
        }
        advance_app_until(
            &mut app,
            "leftward Dig naturally collects GOLD",
            180,
            |app| app_clonk_carries(app, clonk, "GOLD"),
        );
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::Z)
            .expect("release physical Z after GOLD pickup");
        assert_eq!(
            app.engine
                .object_snapshot(gold)
                .expect("collected GOLD")
                .container,
            Some(clonk)
        );
        advance_app_until(
            &mut app,
            "CLNK stops digging after GOLD pickup",
            30,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.action.name == "Walk")
            },
        );
        assert!(
            app_cursor_inventory_contains(&mut app, clonk, "GOLD"),
            "the collected GOLD must reach the rendered cursor inventory"
        );
        // Typed C4GameMessage rejection has its own regression; isolate this
        // inventory-render assertion from that unported overlay.
        app.snapshot.hud.messages.clear();
        app.render(&mut rendered)
            .expect("render Tutorial01 with GOLD inventory");

        // Walk out of the excavated tunnel, then preserve held physical C
        // while reacting to the same Walk/Scale/Jump transitions as the
        // engine virtual route. Re-pressing C on entry to DFA_SCALE supplies
        // the edge C++ uses to let go or climb; an S tap on landing/flight
        // transitions jumps clear without assigning position or action
        // (C4Object.cpp:3618-3628,4823-4855).
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::C)
            .expect("physical C walks out of the GOLD tunnel");
        advance_app_until(
            &mut app,
            "GOLD-carrying CLNK exits the tunnel",
            180,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.position.x >= 215)
            },
        );
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::C)
            .expect("release physical C outside the GOLD tunnel");
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::C)
            .expect("physical C starts the return climb");
        let mut previous_action = String::new();
        for _ in 0..1_800 {
            let clonk_now = app
                .engine
                .object_snapshot(clonk)
                .expect("GOLD-carrying CLNK survives the return");
            if clonk_now.position.x >= 558 {
                break;
            }
            let action = clonk_now.action.name.clone();
            let entered_scale =
                action.starts_with("Scale") && !previous_action.starts_with("Scale");
            let left_scale_in_flight = action == "Jump" && previous_action.starts_with("Scale");
            let landed = action == "Walk" && previous_action != "Walk";
            if entered_scale {
                let mut keyboard = AppVirtualKeyboard::new(&mut app);
                keyboard
                    .release(VirtualKeyCode::C)
                    .expect("release physical C on Scale transition");
                keyboard
                    .press(VirtualKeyCode::C)
                    .expect("re-press physical C on Scale transition");
            } else if landed || left_scale_in_flight {
                let mut keyboard = AppVirtualKeyboard::new(&mut app);
                keyboard
                    .tap(VirtualKeyCode::S)
                    .expect("physical S advances the return climb");
                assert_ne!(
                    keyboard.player_control().pressed_coms & (1 << clonk_engine::COM_RIGHT),
                    0,
                    "releasing S must preserve held C during the return climb"
                );
            }
            previous_action = action;
            app.update().expect("advance Tutorial01 return climb");
        }
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::C)
            .expect("release physical C on the cabin hill");
        assert!(
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.x >= 558),
            "the GOLD-carrying CLNK must reach the cabin hill naturally"
        );
        advance_app_until(
            &mut app,
            "GOLD-carrying CLNK lands beside HUT2",
            60,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.action.name == "Walk")
            },
        );
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::Z)
            .expect("physical Z aligns GOLD-carrying CLNK with HUT2");
        advance_app_until(
            &mut app,
            "GOLD-carrying CLNK aligns with HUT2 entrance",
            60,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.position.x <= 570)
            },
        );
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .release(VirtualKeyCode::Z)
                .expect("release physical Z at HUT2 entrance");
            keyboard
                .tap(VirtualKeyCode::S)
                .expect("physical S enters HUT2 with GOLD");
        }
        advance_app_until(&mut app, "GOLD-carrying CLNK enters HUT2", 60, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.container == Some(hut))
        });

        advance_app_until(&mut app, "Tutorial01 selects Tutorial02", 240, |app| {
            app.engine.next_mission().path == r"Tutorial.c4f\Tutorial02.c4s"
        });
        advance_app_until(&mut app, "Tutorial01 reaches GameOver", 320, |app| {
            app.snapshot.game_over && app.game_over_dialog.is_some()
        });
        assert!(
            app.snapshot
                .round_results
                .fulfilled_goals
                .iter()
                .any(|goal| goal == "SCRG"),
            "Tutorial01 must fulfill its real SCRG before GameOver"
        );
        assert_eq!(
            app.engine.next_mission().path,
            r"Tutorial.c4f\Tutorial02.c4s"
        );
        // The typed C4GameMessage guard has a dedicated regression.
        app.snapshot.hud.messages.clear();
        app.render(&mut rendered)
            .expect("render Tutorial01 GameOver through GameApp");
    }

    #[test]
    fn app_virtual_keyboard_completes_real_tutorial02_route() {
        // The real window path maps keyboard-set-one X/X to Grab and S to Up.
        // While the Clonk pushes BALN, Jump'n'Run ControlUpdate follows held
        // S/X state and keeps DFA_PUSH attached to its moving solid mask; X/X
        // then falls through to UnGrab. Physical C/Z/D/A/S complete all three
        // LOAM bridges, recover FLAG and return it through HUT3's Put menu
        // (C4Object.cpp:3321-3338,3682-3724,4581-4652,5058-5114;
        // Tutorial02.c4s/Script.c:58-214).
        let mut app = real_tutorial_app(2, "Tutorial 2 virtual player");

        let clonk = app
            .engine
            .crew_cursor(app.local_owner)
            .expect("Tutorial02 selected CLNK");
        let balloon = app
            .engine
            .snapshot()
            .objects
            .into_iter()
            .find(|object| object.definition_id == "BALN")
            .expect("Tutorial02 BALN")
            .id;
        let hut = app_object_with_definition(&app, "HUT3").expect("Tutorial02 HUT3");
        let loam_menu_identification =
            serde_json::from_value(serde_json::json!({ "C4Id": "LMMS" }))
                .expect("LOAM menu identification deserializes");

        for _ in 0..160 {
            let clonk_ready = app
                .engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.container.is_none() && object.action.name == "Walk");
            let balloon_ready = app
                .engine
                .object_snapshot(balloon)
                .is_some_and(|object| object.container.is_none());
            if clonk_ready && balloon_ready {
                break;
            }
            app.update().expect("advance Tutorial02 startup");
        }
        assert!(
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.container.is_none() && object.action.name == "Walk"),
            "Tutorial02 CLNK exits the starting base through app frames"
        );

        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard.press(VirtualKeyCode::X).expect("first physical X");
            keyboard
                .release(VirtualKeyCode::X)
                .expect("release first physical X");
            keyboard
                .press(VirtualKeyCode::X)
                .expect("second physical X");
        }
        for _ in 0..80 {
            if app.engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Push" && object.action.target == Some(balloon)
            }) {
                break;
            }
            app.update().expect("advance physical Grab command");
        }
        let pushing = app.engine.object_snapshot(clonk).expect("CLNK after X/X");
        let balloon_before = app
            .engine
            .object_snapshot(balloon)
            .expect("BALN before lift");
        assert_eq!(
            (pushing.action.name.as_str(), pushing.action.target),
            ("Push", Some(balloon)),
            "physical X/X must grab BALN through GameApp"
        );
        let platform_delta_y = pushing.position.y - balloon_before.position.y;

        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .release(VirtualKeyCode::X)
                .expect("release second physical X");
            keyboard
                .press(VirtualKeyCode::S)
                .expect("physical S while pushing BALN");
        }
        for lift_frame in 1..=20 {
            app.update()
                .expect("advance BALN lift through app scheduler");
            let clonk_now = app.engine.object_snapshot(clonk).expect("CLNK during lift");
            let balloon_now = app
                .engine
                .object_snapshot(balloon)
                .expect("BALN during lift");
            assert_eq!(
                (clonk_now.action.name.as_str(), clonk_now.action.target),
                ("Push", Some(balloon)),
                "DFA_PUSH must retain BALN on app lift frame {lift_frame}"
            );
            assert!(
                (clonk_now.position.y - balloon_now.position.y - platform_delta_y).abs() <= 1,
                "CLNK must remain on BALN's platform on app lift frame {lift_frame}; \
                 initial delta={platform_delta_y}, clonk={clonk_now:?}, balloon={balloon_now:?}"
            );
        }
        assert!(
            app.engine
                .object_snapshot(balloon)
                .expect("BALN after lift")
                .position
                .y
                < balloon_before.position.y,
            "physical S must lift BALN"
        );

        // The engine-only Tutorial02 replay deliberately joins a classic
        // player. This app fixture is the fresh-player Jump'n'Run default, so
        // release S (rather than a delayed X Single) supplies Stop through
        // BALN::ControlUpdate (C4Object.cpp:3327-3337;
        // Balloon.c4d/Script.c:60-78).
        for lift_frame in 21..=180 {
            if app
                .engine
                .object_snapshot(balloon)
                .is_some_and(|object| object.position.y <= 150)
            {
                break;
            }
            app.update().expect("advance BALN to flight corridor");
            let clonk_now = app
                .engine
                .object_snapshot(clonk)
                .expect("CLNK during remaining lift");
            let balloon_now = app
                .engine
                .object_snapshot(balloon)
                .expect("BALN during remaining lift");
            assert_eq!(
                (clonk_now.action.name.as_str(), clonk_now.action.target),
                ("Push", Some(balloon)),
                "DFA_PUSH must retain BALN on app lift frame {lift_frame}"
            );
            assert!(
                (clonk_now.position.y - balloon_now.position.y - platform_delta_y).abs() <= 1,
                "CLNK must remain on BALN's platform on app lift frame {lift_frame}"
            );
        }
        assert!(
            app.engine
                .object_snapshot(balloon)
                .is_some_and(|object| object.position.y <= 150),
            "held physical S must reach Tutorial02's flight corridor"
        );
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            assert!(
                keyboard.player_control().control_style,
                "the isolated fresh player must use Jump'n'Run/AutoStop control"
            );
            keyboard
                .release(VirtualKeyCode::S)
                .expect("release physical S in flight corridor");
            assert_eq!(
                keyboard
                    .engine()
                    .object_snapshot(balloon)
                    .expect("BALN after S release")
                    .command_direction,
                CommandDirection::Stop,
                "Jump'n'Run S release must stop vertical BALN control"
            );
        }

        // Stop intentionally retains BALN's wind-driven drift. Coast east
        // while continuously pinning the Push target and platform delta.
        for coast_frame in 1..=600 {
            if app
                .engine
                .object_snapshot(balloon)
                .is_some_and(|object| object.position.x >= 520)
            {
                break;
            }
            app.update().expect("coast BALN toward far island");
            let clonk_now = app
                .engine
                .object_snapshot(clonk)
                .expect("CLNK while coasting");
            let balloon_now = app
                .engine
                .object_snapshot(balloon)
                .expect("BALN while coasting");
            assert_eq!(
                (clonk_now.action.name.as_str(), clonk_now.action.target),
                ("Push", Some(balloon)),
                "DFA_PUSH must retain BALN on coast frame {coast_frame}"
            );
            assert!(
                (clonk_now.position.y - balloon_now.position.y - platform_delta_y).abs() <= 1,
                "CLNK must remain on BALN's platform on coast frame {coast_frame}; \
                 initial delta={platform_delta_y}, clonk={clonk_now:?}, balloon={balloon_now:?}"
            );
        }
        assert!(
            app.engine
                .object_snapshot(balloon)
                .is_some_and(|object| object.position.x >= 520),
            "stopped BALN must coast to the far-island longitude; balloon={:?}",
            app.engine.object_snapshot(balloon)
        );

        // In Jump'n'Run control, held physical X supplies Down immediately;
        // releasing X restores Stop. This intentionally does not use the
        // classic route's delayed DownSingle toggle.
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .press(VirtualKeyCode::X)
                .expect("hold physical X to descend");
            assert_eq!(
                keyboard
                    .engine()
                    .object_snapshot(balloon)
                    .expect("BALN after X press")
                    .command_direction,
                CommandDirection::Down
            );
        }
        for descent_frame in 1..=240 {
            let in_gate = app.engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Push"
                    && object.action.target == Some(balloon)
                    && (450..710).contains(&object.position.x)
                    && (250..320).contains(&object.position.y)
            });
            if in_gate {
                break;
            }
            app.update().expect("descend BALN toward far island");
            let clonk_now = app
                .engine
                .object_snapshot(clonk)
                .expect("CLNK while descending");
            let balloon_now = app
                .engine
                .object_snapshot(balloon)
                .expect("BALN while descending");
            assert_eq!(
                (clonk_now.action.name.as_str(), clonk_now.action.target),
                ("Push", Some(balloon)),
                "DFA_PUSH must retain BALN on descent frame {descent_frame}"
            );
            assert!(
                (clonk_now.position.y - balloon_now.position.y - platform_delta_y).abs() <= 1,
                "CLNK must remain on BALN's platform on descent frame {descent_frame}"
            );
        }
        assert!(
            app.engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Push"
                    && object.action.target == Some(balloon)
                    && (450..710).contains(&object.position.x)
                    && (250..320).contains(&object.position.y)
            }),
            "held physical X must reach Tutorial02 Script3's far-island gate"
        );
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .release(VirtualKeyCode::X)
                .expect("release physical X at far island");
            assert_eq!(
                keyboard
                    .engine()
                    .object_snapshot(balloon)
                    .expect("BALN after X release")
                    .command_direction,
                CommandDirection::Stop
            );
        }

        // Release does not clear C4Player::LastCom. Eleven app updates let the
        // prior X press leave C4DoubleClick's window before the instructed X/X;
        // otherwise the first new X could become the stale press's Double.
        for _ in 0..11 {
            app.update()
                .expect("wait out descent X double-click buffer");
            let clonk_now = app
                .engine
                .object_snapshot(clonk)
                .expect("CLNK while awaiting release prompt");
            let balloon_now = app
                .engine
                .object_snapshot(balloon)
                .expect("BALN while awaiting release prompt");
            assert_eq!(
                (clonk_now.action.name.as_str(), clonk_now.action.target),
                ("Push", Some(balloon))
            );
            assert!((clonk_now.position.y - balloon_now.position.y - platform_delta_y).abs() <= 1);
        }
        advance_app_until(&mut app, "Tutorial02 balloon-release prompt", 30, |app| {
            app_tutorial_message_contains(app, "Let go of the balloon")
        });
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .tap(VirtualKeyCode::X)
                .expect("first physical X of ungrab double");
            keyboard
                .tap(VirtualKeyCode::X)
                .expect("second physical X of ungrab double");
        }
        advance_app_until(&mut app, "CLNK lands on the far island", 100, |app| {
            app.engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Walk"
                    && (450..710).contains(&object.position.x)
                    && (270..320).contains(&object.position.y)
            })
        });
        // The Jump'n'Run descent can land between the real material objects
        // instead of contacting one immediately like the classic route. Let
        // Script20 expose the actual next instruction, while still accepting
        // either natural FLAG/LOAM contact if the drift produced one.
        advance_app_until(
            &mut app,
            "Tutorial02 post-flight collectible prompt or contact",
            450,
            |app| {
                app_clonk_carries(app, clonk, "FLAG")
                    || app_clonk_carries(app, clonk, "LOAM")
                    || app_tutorial_message_contains(app, "Please drop the flag for now")
                    || app_tutorial_message_contains(app, "Pick up one of the loam chunks")
            },
        );

        // Contact may deterministically choose FLAG or one of four LOAM
        // objects. Script30 requires a real world Throw when FLAG wins; face
        // the island center with physical Z, then use physical A.
        if app_clonk_carries(&app, clonk, "FLAG") {
            advance_app_until(&mut app, "Tutorial02 temporary FLAG prompt", 450, |app| {
                app_tutorial_message_contains(app, "Please drop the flag for now")
            });
            AppVirtualKeyboard::new(&mut app)
                .press(VirtualKeyCode::Z)
                .expect("physical Z faces island center");
            advance_app_until(
                &mut app,
                "CLNK faces left before throwing FLAG",
                30,
                |app| {
                    app.engine
                        .object_snapshot(clonk)
                        .is_some_and(|object| object.direction == Direction::Left)
                },
            );
            {
                let mut keyboard = AppVirtualKeyboard::new(&mut app);
                keyboard
                    .release(VirtualKeyCode::Z)
                    .expect("release physical Z before FLAG throw");
                assert_eq!(
                    keyboard
                        .engine()
                        .object_snapshot(clonk)
                        .expect("CLNK before FLAG throw")
                        .direction,
                    Direction::Left
                );
                keyboard
                    .tap(VirtualKeyCode::A)
                    .expect("physical world-A throws temporary FLAG");
            }
            advance_app_until(&mut app, "FLAG leaves CLNK inventory", 30, |app| {
                !app_clonk_carries(app, clonk, "FLAG")
            });
        }

        if !app_clonk_carries(&app, clonk, "LOAM") {
            advance_app_until(
                &mut app,
                "Tutorial02 LOAM pickup prompt or contact",
                450,
                |app| {
                    app_tutorial_message_contains(app, "Pick up one of the loam chunks")
                        || app_clonk_carries(app, clonk, "LOAM")
                },
            );
            if !app_clonk_carries(&app, clonk, "LOAM") {
                let direction_to_loam = |app: &GameApp| {
                    let clonk_x = app
                        .engine
                        .object_snapshot(clonk)
                        .expect("CLNK survives Tutorial02 landing")
                        .position
                        .x;
                    let loam_x = app
                        .engine
                        .snapshot()
                        .objects
                        .into_iter()
                        .filter(|object| object.definition_id == "LOAM")
                        .min_by_key(|object| (object.position.x - clonk_x).abs())
                        .expect("Tutorial02 keeps a loose LOAM chunk")
                        .position
                        .x;
                    if clonk_x < loam_x {
                        VirtualKeyCode::C
                    } else {
                        VirtualKeyCode::Z
                    }
                };
                let toward_first_object = direction_to_loam(&app);
                hold_app_key_until(
                    &mut app,
                    toward_first_object,
                    "CLNK naturally collects the first island object",
                    120,
                    |app| {
                        app_clonk_carries(app, clonk, "LOAM")
                            || app_clonk_carries(app, clonk, "FLAG")
                    },
                );
                if app_clonk_carries(&app, clonk, "FLAG") {
                    advance_app_until(&mut app, "Tutorial02 temporary FLAG prompt", 450, |app| {
                        app_tutorial_message_contains(app, "Please drop the flag for now")
                    });
                    AppVirtualKeyboard::new(&mut app)
                        .press(VirtualKeyCode::C)
                        .expect("physical C faces away from the LOAM");
                    advance_app_until(
                        &mut app,
                        "CLNK faces right before throwing FLAG",
                        30,
                        |app| {
                            app.engine
                                .object_snapshot(clonk)
                                .is_some_and(|object| object.direction == Direction::Right)
                        },
                    );
                    {
                        let mut keyboard = AppVirtualKeyboard::new(&mut app);
                        keyboard
                            .release(VirtualKeyCode::C)
                            .expect("release physical C before FLAG throw");
                        keyboard
                            .tap(VirtualKeyCode::A)
                            .expect("physical world-A throws temporary FLAG away from LOAM");
                    }
                    advance_app_until(&mut app, "FLAG leaves CLNK inventory", 30, |app| {
                        !app_clonk_carries(app, clonk, "FLAG")
                    });
                }
                if !app_clonk_carries(&app, clonk, "LOAM") {
                    let toward_loam = direction_to_loam(&app);
                    hold_app_key_until(
                        &mut app,
                        toward_loam,
                        "CLNK naturally collects LOAM",
                        120,
                        |app| app_clonk_carries(app, clonk, "LOAM"),
                    );
                }
            }
        }
        assert!(app_clonk_carries(&app, clonk, "LOAM"));
        assert!(
            app_cursor_inventory_contains(&mut app, clonk, "LOAM"),
            "the collected LOAM must reach the cursor inventory presentation"
        );

        // Script40..42 moves the player to the left bridge position, observes
        // LMMS, and asks for its Diagonal left row. AutoStop Z release already
        // stops the CLNK, so no classic-only Down stop is injected here.
        advance_app_until(&mut app, "Tutorial02 move-left prompt", 240, |app| {
            app_tutorial_message_contains(app, "Now move to the very left edge")
        });
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::Z)
            .expect("physical Z walks to first bridge position");
        advance_app_until(&mut app, "Tutorial02 first bridge position", 120, |app| {
            app.engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Walk" && (488..=490).contains(&object.position.x)
            })
        });
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::Z)
            .expect("release physical Z at first bridge position");
        advance_app_until(&mut app, "Tutorial02 double-Dig prompt", 180, |app| {
            app_tutorial_message_contains(app, "Press the 'dig' key twice quickly")
        });
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .tap(VirtualKeyCode::D)
                .expect("first physical D for LOAM activation");
            keyboard
                .tap(VirtualKeyCode::D)
                .expect("second physical D for LOAM activation");
        }
        advance_app_until(&mut app, "LOAM opens LMMS", 10, |app| {
            app.engine
                .cursor_object_menu(app.local_owner)
                .is_some_and(|(_, menu)| menu.identification == loam_menu_identification)
        });
        advance_app_until(&mut app, "Tutorial02 Diagonal left prompt", 180, |app| {
            app_tutorial_message_contains(app, "Select the option 'diagonal left'")
        });
        app.snapshot.hud.messages.clear();
        let mut rendered = vec![0_u8; 320 * 200 * 4];
        app.render(&mut rendered)
            .expect("render Tutorial02 LOAM construction menu");
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::Z)
            .expect("physical Z selects Diagonal left");
        let selected = app
            .engine
            .cursor_object_menu(app.local_owner)
            .and_then(|(_, menu)| {
                usize::try_from(menu.selection)
                    .ok()
                    .map(|index| (menu, index))
            })
            .and_then(|(menu, index)| menu.items.get(index))
            .map(|item| item.caption.as_str());
        assert_eq!(selected, Some("Diagonal left"));
        let bridge_start = app
            .engine
            .object_snapshot(clonk)
            .expect("CLNK before first LOAM bridge")
            .position;
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::A)
            .expect("physical A starts Diagonal left bridge");
        advance_app_until(&mut app, "CLNK starts first LOAM Bridge", 10, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Bridge")
        });
        assert_eq!(
            app.engine
                .object_snapshot(clonk)
                .expect("CLNK at first LOAM Bridge start")
                .position,
            bridge_start,
            "physical menu inputs must start Bridge without positioning the CLNK"
        );

        // C++ advances the moving UpLeft bridge first at Action.Time 6, then
        // moves sixteen (-1,-1) steps before returning to Walk
        // (C4Object.cpp:4581-4652,4755-4756).
        for _ in 0..6 {
            app.update().expect("advance first LOAM Bridge step");
        }
        let first_bridge_step = app
            .engine
            .object_snapshot(clonk)
            .expect("CLNK survives first LOAM Bridge step");
        assert_eq!(first_bridge_step.action.name, "Bridge");
        assert_eq!(first_bridge_step.action.time, 6);
        assert_eq!(
            first_bridge_step.action.data, 0x0064_0110,
            "LOAM must request C++'s moving, non-wall Earth bridge"
        );
        assert_eq!(
            first_bridge_step.position,
            Vector2::new(bridge_start.x - 1, bridge_start.y - 1)
        );
        advance_app_until(&mut app, "first UpLeft bridge completes", 114, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk")
        });
        let first_bridge_end = app
            .engine
            .object_snapshot(clonk)
            .expect("CLNK after first bridge")
            .position;
        assert_eq!(
            (
                first_bridge_end.x - bridge_start.x,
                first_bridge_end.y - bridge_start.y,
            ),
            (-16, -16)
        );
        advance_app_until(&mut app, "Tutorial02 three-bridge prompt", 180, |app| {
            app_tutorial_message_contains(app, "build three diagonal bridges")
        });

        // Cross back over bridge one for LOAM2, release C to stop, then return
        // with Z to its upper-left endpoint. Every fresh LMMS begins at row 7;
        // exactly one physical Z selects row 6, Diagonal left.
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::C)
            .expect("physical C crosses bridge one for LOAM2");
        advance_app_until(&mut app, "CLNK collects LOAM2", 220, |app| {
            app_clonk_carries(app, clonk, "LOAM")
        });
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::C)
            .expect("release physical C after LOAM2 pickup");
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::Z)
            .expect("physical Z returns to bridge-one endpoint");
        advance_app_until(
            &mut app,
            "CLNK returns to bridge-one endpoint",
            220,
            |app| {
                app.engine.object_snapshot(clonk).is_some_and(|object| {
                    object.container.is_none()
                        && object.action.name == "Walk"
                        && object.position.x <= first_bridge_end.x
                })
            },
        );
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::Z)
            .expect("release physical Z at bridge-one endpoint");
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .tap(VirtualKeyCode::D)
                .expect("first physical D for LOAM2");
            keyboard
                .tap(VirtualKeyCode::D)
                .expect("second physical D for LOAM2");
        }
        advance_app_until(&mut app, "LOAM2 opens LMMS", 20, |app| {
            app.engine
                .cursor_object_menu(app.local_owner)
                .is_some_and(|(_, menu)| menu.identification == loam_menu_identification)
        });
        assert_eq!(
            app.engine
                .cursor_object_menu(app.local_owner)
                .map(|(_, menu)| menu.selection),
            Some(7)
        );
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::Z)
            .expect("physical Z selects LOAM2 Diagonal left");
        assert_eq!(
            app.engine
                .cursor_object_menu(app.local_owner)
                .and_then(|(_, menu)| {
                    usize::try_from(menu.selection)
                        .ok()
                        .and_then(|index| menu.items.get(index))
                })
                .map(|item| item.caption.as_str()),
            Some("Diagonal left")
        );
        let second_bridge_start = app
            .engine
            .object_snapshot(clonk)
            .expect("CLNK before second bridge")
            .position;
        assert!(
            (second_bridge_start.x - first_bridge_end.x).abs() <= 1
                && (second_bridge_start.y - first_bridge_end.y).abs() <= 1,
            "bridge two must continue bridge one; first_end={first_bridge_end:?}, second_start={second_bridge_start:?}"
        );
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::A)
            .expect("physical A starts second bridge");
        advance_app_until(&mut app, "CLNK starts second Bridge", 10, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Bridge")
        });
        advance_app_until(&mut app, "second UpLeft bridge completes", 114, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk")
        });
        let second_bridge_end = app
            .engine
            .object_snapshot(clonk)
            .expect("CLNK after second bridge")
            .position;
        assert_eq!(
            (
                second_bridge_end.x - second_bridge_start.x,
                second_bridge_end.y - second_bridge_start.y,
            ),
            (-16, -16)
        );

        // Cross both spans for LOAM3. FLAG may be encountered first after the
        // earlier Script30 throw; face right with a physical C frame, throw it
        // using world A, finish Throw, then continue to adjacent LOAM.
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::C)
            .expect("physical C crosses two bridges for LOAM3");
        advance_app_until(&mut app, "CLNK reaches LOAM3 or FLAG", 260, |app| {
            app_clonk_carries(app, clonk, "LOAM") || app_clonk_carries(app, clonk, "FLAG")
        });
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::C)
            .expect("release physical C at far-island material");
        if app_clonk_carries(&app, clonk, "FLAG") {
            AppVirtualKeyboard::new(&mut app)
                .press(VirtualKeyCode::C)
                .expect("physical C faces right before rethrowing FLAG");
            advance_app_until(
                &mut app,
                "CLNK faces right before rethrowing FLAG",
                30,
                |app| {
                    app.engine
                        .object_snapshot(clonk)
                        .is_some_and(|object| object.direction == Direction::Right)
                },
            );
            {
                let mut keyboard = AppVirtualKeyboard::new(&mut app);
                keyboard
                    .release(VirtualKeyCode::C)
                    .expect("release physical C before rethrowing FLAG");
                assert_eq!(
                    keyboard
                        .engine()
                        .object_snapshot(clonk)
                        .expect("CLNK before rethrowing FLAG")
                        .direction,
                    Direction::Right
                );
                keyboard
                    .tap(VirtualKeyCode::A)
                    .expect("physical world-A rethrows FLAG");
            }
            advance_app_until(&mut app, "recollected FLAG leaves CLNK", 30, |app| {
                !app_clonk_carries(app, clonk, "FLAG")
            });
            advance_app_until(&mut app, "CLNK finishes rethrowing FLAG", 30, |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.action.name == "Walk")
            });
            AppVirtualKeyboard::new(&mut app)
                .press(VirtualKeyCode::C)
                .expect("physical C continues to LOAM3");
            advance_app_until(&mut app, "CLNK collects LOAM3", 100, |app| {
                app_clonk_carries(app, clonk, "LOAM")
            });
            AppVirtualKeyboard::new(&mut app)
                .release(VirtualKeyCode::C)
                .expect("release physical C after LOAM3 pickup");
        }
        assert!(app_clonk_carries(&app, clonk, "LOAM"));
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::Z)
            .expect("physical Z returns to bridge-two endpoint");
        advance_app_until(
            &mut app,
            "CLNK returns to bridge-two endpoint",
            260,
            |app| {
                app.engine.object_snapshot(clonk).is_some_and(|object| {
                    object.container.is_none()
                        && object.action.name == "Walk"
                        && object.position.x <= second_bridge_end.x
                })
            },
        );
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::Z)
            .expect("release physical Z at bridge-two endpoint");
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .tap(VirtualKeyCode::D)
                .expect("first physical D for LOAM3");
            keyboard
                .tap(VirtualKeyCode::D)
                .expect("second physical D for LOAM3");
        }
        advance_app_until(&mut app, "LOAM3 opens LMMS", 20, |app| {
            app.engine
                .cursor_object_menu(app.local_owner)
                .is_some_and(|(_, menu)| menu.identification == loam_menu_identification)
        });
        assert_eq!(
            app.engine
                .cursor_object_menu(app.local_owner)
                .map(|(_, menu)| menu.selection),
            Some(7)
        );
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::Z)
            .expect("physical Z selects LOAM3 Diagonal left");
        assert_eq!(
            app.engine
                .cursor_object_menu(app.local_owner)
                .and_then(|(_, menu)| {
                    usize::try_from(menu.selection)
                        .ok()
                        .and_then(|index| menu.items.get(index))
                })
                .map(|item| item.caption.as_str()),
            Some("Diagonal left")
        );
        let third_bridge_start = app
            .engine
            .object_snapshot(clonk)
            .expect("CLNK before third bridge")
            .position;
        assert!(
            (third_bridge_start.x - second_bridge_end.x).abs() <= 1
                && (third_bridge_start.y - second_bridge_end.y).abs() <= 1,
            "bridge three must continue bridge two; second_end={second_bridge_end:?}, third_start={third_bridge_start:?}"
        );
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::A)
            .expect("physical A starts third bridge");
        advance_app_until(&mut app, "CLNK starts third Bridge", 10, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Bridge")
        });
        advance_app_until(&mut app, "third UpLeft bridge completes", 114, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk")
        });
        let third_bridge_end = app
            .engine
            .object_snapshot(clonk)
            .expect("CLNK after third bridge")
            .position;
        assert_eq!(
            (
                third_bridge_end.x - third_bridge_start.x,
                third_bridge_end.y - third_bridge_start.y,
            ),
            (-16, -16)
        );
        let three_bridge_delta = (
            third_bridge_end.x - bridge_start.x,
            third_bridge_end.y - bridge_start.y,
        );
        assert!(
            (three_bridge_delta.0 + 48).abs() <= 2
                && (three_bridge_delta.1 + 48).abs() <= 2
                && (360..445).contains(&third_bridge_end.x)
                && (240..290).contains(&third_bridge_end.y),
            "three contiguous bridges must reach Script81; delta={three_bridge_delta:?}, end={third_bridge_end:?}"
        );
        advance_app_until(&mut app, "Tutorial02 close-enough prompt", 180, |app| {
            app_tutorial_message_contains(app, "close enough to jump")
        });

        // Walk back over all three bridges for FLAG. Four LOAM chunks exist
        // for three spans, so throw a spare left with world A before continuing
        // right to FLAG; inventory slot zero must then be FLAG.
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::C)
            .expect("physical C returns to far-island pickup");
        advance_app_until(&mut app, "CLNK reaches FLAG or spare LOAM", 420, |app| {
            app_clonk_carries(app, clonk, "FLAG") || app_clonk_carries(app, clonk, "LOAM")
        });
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::C)
            .expect("release physical C at final pickup");
        if app_clonk_carries(&app, clonk, "LOAM") {
            AppVirtualKeyboard::new(&mut app)
                .press(VirtualKeyCode::Z)
                .expect("physical Z faces left before spare LOAM throw");
            advance_app_until(
                &mut app,
                "CLNK faces left before spare LOAM throw",
                30,
                |app| {
                    app.engine
                        .object_snapshot(clonk)
                        .is_some_and(|object| object.direction == Direction::Left)
                },
            );
            {
                let mut keyboard = AppVirtualKeyboard::new(&mut app);
                keyboard
                    .release(VirtualKeyCode::Z)
                    .expect("release physical Z before spare LOAM throw");
                assert_eq!(
                    keyboard
                        .engine()
                        .object_snapshot(clonk)
                        .expect("CLNK before spare LOAM throw")
                        .direction,
                    Direction::Left
                );
                keyboard
                    .tap(VirtualKeyCode::A)
                    .expect("physical world-A throws spare LOAM");
            }
            advance_app_until(&mut app, "spare LOAM leaves CLNK", 30, |app| {
                !app_clonk_carries(app, clonk, "LOAM")
            });
            advance_app_until(&mut app, "CLNK finishes throwing spare LOAM", 30, |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.action.name == "Walk")
            });
        }
        if !app_clonk_carries(&app, clonk, "FLAG") {
            AppVirtualKeyboard::new(&mut app)
                .press(VirtualKeyCode::C)
                .expect("physical C continues to FLAG");
            advance_app_until(&mut app, "CLNK collects FLAG", 180, |app| {
                app_clonk_carries(app, clonk, "FLAG")
            });
            AppVirtualKeyboard::new(&mut app)
                .release(VirtualKeyCode::C)
                .expect("release physical C after FLAG pickup");
        }
        let flag = app
            .engine
            .object_snapshot(clonk)
            .and_then(|object| object.contents.first().copied())
            .expect("FLAG occupies CLNK inventory slot zero");
        assert_eq!(
            app.engine
                .object_snapshot(flag)
                .expect("carried FLAG")
                .definition_id,
            "FLAG"
        );

        // Keep physical Z held over all three bridges and both jumps home. S
        // release must preserve the held Left bit on each jump.
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::Z)
            .expect("physical Z starts FLAG return");
        advance_app_until(
            &mut app,
            "FLAG-carrying CLNK reaches bridge endpoint",
            420,
            |app| {
                app.engine.object_snapshot(clonk).is_some_and(|object| {
                    object.action.name == "Walk" && object.position.x <= third_bridge_end.x
                })
            },
        );
        let first_return_jump_frame = app.engine.frame();
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .tap(VirtualKeyCode::S)
                .expect("physical S jumps to center island");
            assert_ne!(
                keyboard.player_control().pressed_coms & (1 << clonk_engine::COM_LEFT),
                0,
                "first S release must preserve held Z"
            );
        }
        advance_app_until(
            &mut app,
            "FLAG-carrying CLNK lands on center island",
            140,
            |app| {
                app.engine.object_snapshot(clonk).is_some_and(|object| {
                    object.action.name == "Walk" && (290..390).contains(&object.position.x)
                })
            },
        );
        advance_app_until(
            &mut app,
            "CLNK reaches center-island jump edge",
            120,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.action.name == "Walk" && object.position.x <= 310)
            },
        );
        // The Jump'n'Run center hop can finish inside C4DoubleClick's ten-tick
        // window. Keep Z held, but do not let the second physical S become an
        // ignored COM_Up_D and turn the intended jump into a walk-off fall.
        while app.engine.frame().saturating_sub(first_return_jump_frame) <= 10 {
            app.update()
                .expect("wait out first return S double-click buffer");
            assert_eq!(
                app.engine
                    .object_snapshot(clonk)
                    .expect("CLNK waits at center-island jump edge")
                    .action
                    .name,
                "Walk"
            );
        }
        let second_jump_start = app
            .engine
            .object_snapshot(clonk)
            .expect("CLNK at center-island jump edge")
            .position;
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .tap(VirtualKeyCode::S)
                .expect("physical S jumps to home island");
            assert_ne!(
                keyboard.player_control().pressed_coms & (1 << clonk_engine::COM_LEFT),
                0,
                "second S release must preserve held Z"
            );
        }
        app.update().expect("execute second physical S jump");
        let launched = app
            .engine
            .object_snapshot(clonk)
            .expect("CLNK after second S executes");
        assert_eq!(launched.action.name, "Jump");
        assert!(
            launched.velocity.y < 0,
            "second physical S must launch upward; clonk={launched:?}"
        );
        for _ in 0..160 {
            if app
                .engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk" && object.position.x <= 230)
            {
                break;
            }
            app.update().expect("advance second FLAG return jump");
        }
        let home_landing = app
            .engine
            .object_snapshot(clonk)
            .expect("FLAG-carrying CLNK after second return jump");
        assert!(
            home_landing.action.name == "Walk" && home_landing.position.x <= 230,
            "FLAG-carrying CLNK must land from {second_jump_start:?}; clonk={home_landing:?}"
        );
        let hut_position = app
            .engine
            .object_snapshot(hut)
            .expect("HUT3 survives Tutorial02")
            .position;
        advance_app_until(
            &mut app,
            "FLAG-carrying CLNK reaches HUT3 entrance",
            160,
            |app| {
                app.engine.object_snapshot(clonk).is_some_and(|object| {
                    object.action.name == "Walk"
                        && (hut_position.x + 2..hut_position.x + 19).contains(&object.position.x)
                        && (hut_position.y + 4..hut_position.y + 25).contains(&object.position.y)
                })
            },
        );
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::Z)
            .expect("release physical Z at HUT3 entrance");
        assert_eq!(
            app.engine
                .object_snapshot(hut)
                .expect("HUT3 before FLAG return")
                .base,
            -1,
            "HUT3 must not be a base while FlyBase FLAG is absent"
        );
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::S)
            .expect("physical S enters HUT3");
        advance_app_until(&mut app, "FLAG-carrying CLNK enters HUT3", 80, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.container == Some(hut))
        });

        // AutoContextMenu inserts Put first for the contained FLAG. Physical A
        // is therefore MenuEnter/Put, not a direct contained Throw
        // (C4Player.cpp:1502-1513; C4ObjectMenu.cpp:335-359).
        advance_app_until(&mut app, "HUT3 auto-context Put row", 20, |app| {
            app.engine
                .cursor_object_menu(app.local_owner)
                .is_some_and(|(_, menu)| {
                    menu.selection == 0
                        && menu.items.first().is_some_and(|item| item.caption == "Put")
                })
        });
        advance_app_until(&mut app, "Tutorial02 FLAG Put prompt", 240, |app| {
            app_tutorial_message_contains(app, "Press 'throw' to put the flag")
        });
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::A)
            .expect("physical A puts FLAG into HUT3");
        advance_app_until(&mut app, "FLAG enters HUT3", 80, |app| {
            app.engine
                .object_snapshot(flag)
                .is_some_and(|object| object.container == Some(hut))
        });
        advance_app_until(&mut app, "HUT3 restores the player base", 80, |app| {
            app.engine
                .object_snapshot(hut)
                .is_some_and(|object| object.base == app.local_owner)
        });
        advance_app_until(&mut app, "Tutorial02 selects Tutorial03", 180, |app| {
            app.engine.next_mission().path == r"Tutorial.c4f\Tutorial03.c4s"
        });
        advance_app_until(&mut app, "Tutorial02 reaches GameOver", 320, |app| {
            app.snapshot.game_over && app.game_over_dialog.is_some()
        });
        assert!(
            app.snapshot
                .round_results
                .fulfilled_goals
                .iter()
                .any(|goal| goal == "SCRG"),
            "Tutorial02 must fulfill SCRG before GameOver"
        );
        assert_eq!(
            app.engine.next_mission().path,
            r"Tutorial.c4f\Tutorial03.c4s"
        );
        // Typed C4GameMessage rejection has its own regression; isolate this
        // GameOver-render assertion from that unported overlay.
        app.snapshot.hud.messages.clear();
        app.render(&mut rendered)
            .expect("render Tutorial02 GameOver through GameApp");
    }

    #[test]
    fn app_virtual_keyboard_completes_real_tutorial03_route() {
        // Tutorial03 teaches the permanent building-menu sequence after the
        // Clonk enters HUT3: C4MN_Context=14 exposes Contents/Buy/Sell/Info/Exit
        // before the player selects Buy (Tutorial03.c4s/Script.c:106-145;
        // C4Object.cpp:1919-1980,3034-3048; C4ObjectMenu.cpp:361-427). Drive
        // C then S through GameApp's physical keyboard boundary so this also
        // covers the real key map and ObjectComUp entrance path.
        let mut app = real_tutorial_app(3, "Tutorial 3 app virtual player");
        assert!(
            !app.mouse_control,
            "Tutorial03 DisableMouse=1 must suppress player mouse control and the menu close X like C++ (C4Player.cpp:1907-1912; C4Menu.cpp:1270-1276)"
        );
        assert!(
            !app.option_flags(app.local_owner).mouse_shown,
            "DisableMouse must remove the in-game Options entry like C++ (C4MainMenu.cpp:563-571)"
        );

        let clonk = app
            .engine
            .crew_cursor(app.local_owner)
            .expect("Tutorial03 selected CLNK");
        let hut = app
            .engine
            .snapshot()
            .objects
            .into_iter()
            .find(|object| object.definition_id == "HUT3")
            .expect("Tutorial03 HUT3")
            .id;
        for _ in 0..360 {
            let ready = app
                .engine
                .object_snapshot(hut)
                .is_some_and(|object| object.base == app.local_owner)
                && app.engine.object_snapshot(clonk).is_some_and(|object| {
                    object.container.is_none() && object.action.name == "Walk"
                });
            if ready {
                break;
            }
            app.update().expect("advance Tutorial03 startup");
        }
        assert!(
            app.engine
                .object_snapshot(hut)
                .is_some_and(|object| { object.base == app.local_owner }),
            "Tutorial03 ready HUT3 must become the local player's base"
        );
        assert!(
            app.engine.object_snapshot(clonk).is_some_and(|object| {
                object.container.is_none() && object.action.name == "Walk"
            }),
            "Tutorial03 CLNK must exit the starting base through app frames"
        );

        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .press(VirtualKeyCode::C)
                .expect("physical C walks right");
        }
        for _ in 0..40 {
            let at_entrance = app
                .engine
                .object_snapshot(hut)
                .zip(app.engine.object_snapshot(clonk))
                .is_some_and(|(hut, clonk)| clonk.position.x >= hut.position.x + 2);
            if at_entrance {
                break;
            }
            app.update().expect("walk to HUT3 entrance");
        }
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .release(VirtualKeyCode::C)
                .expect("release physical C");
            keyboard
                .press(VirtualKeyCode::S)
                .expect("physical S enters HUT3");
            keyboard
                .release(VirtualKeyCode::S)
                .expect("release physical S");
        }
        for _ in 0..40 {
            if app
                .engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.container == Some(hut))
            {
                break;
            }
            app.update().expect("advance HUT3 entrance command");
        }
        assert_eq!(
            app.engine
                .object_snapshot(clonk)
                .expect("CLNK after physical S")
                .container,
            Some(hut),
            "physical C/S route must enter HUT3 through GameApp"
        );

        for _ in 0..20 {
            if app.engine.cursor_object_menu(app.local_owner).is_some() {
                break;
            }
            app.update().expect("wait for HUT3 auto-context menu");
        }
        let (_, menu) = app
            .engine
            .cursor_object_menu(app.local_owner)
            .expect("HUT3 exposes its app-visible auto-context menu");
        let context_identification = serde_json::from_value(serde_json::json!({ "Int": 14 }))
            .expect("integer menu identification deserializes");
        let buy_identification = serde_json::from_value(serde_json::json!({ "Int": 4 }))
            .expect("buy menu identification deserializes");
        let contents_identification = serde_json::from_value(serde_json::json!({ "Int": 18 }))
            .expect("contents menu identification deserializes");
        assert_eq!(menu.identification, context_identification);
        assert_eq!(
            menu.caption, "Cabin",
            "C4Def::Load must replace HUT3's DefCore fallback with Names.txt US localization (C4Def.cpp:635-639)"
        );
        assert_eq!(
            menu.items
                .iter()
                .map(|item| item.caption.as_str())
                .collect::<Vec<_>>(),
            vec!["Contents", "Buy", "Sell", "Info", "Exit"]
        );
        let mut rendered = vec![0_u8; 320 * 200 * 4];
        app.render(&mut rendered)
            .expect("render Tutorial03 context menu through the app");
        advance_app_until(&mut app, "Tutorial03 Buy-menu prompt", 240, |app| {
            app_tutorial_message_contains(app, "Select option 'Buy'")
        });

        // Physical X is the classic down control and physical A is Throw;
        // while a menu is open C4Player::InCom translates them to MenuDown
        // and MenuEnter (C4Player.cpp:1502-1513). This is the exact Tutorial03
        // input path from Context -> Buy, without mutating menu state.
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .press(VirtualKeyCode::X)
                .expect("physical X selects Buy");
            keyboard
                .release(VirtualKeyCode::X)
                .expect("release physical X");
            keyboard
                .press(VirtualKeyCode::A)
                .expect("physical A enters Buy");
            keyboard
                .release(VirtualKeyCode::A)
                .expect("release physical A");
        }
        for _ in 0..20 {
            let buy_menu_open = app
                .engine
                .cursor_object_menu(app.local_owner)
                .is_some_and(|(_, menu)| menu.identification == buy_identification);
            if buy_menu_open {
                break;
            }
            app.update().expect("advance physical Buy selection");
        }
        let (_, buy_menu) = app
            .engine
            .cursor_object_menu(app.local_owner)
            .expect("physical X/A opens Tutorial03 Buy menu");
        assert_eq!(buy_menu.identification, buy_identification);
        assert_eq!(
            buy_menu.title_symbol,
            clonk_engine::ObjectMenuSymbol::Buy {
                owner: app
                    .engine
                    .object_snapshot(hut)
                    .expect("Tutorial03 HUT3 remains active")
                    .owner,
            },
            "C4MN_Buy title uses the contained building owner (C4Object.cpp:1919-1928)"
        );
        assert_eq!(
            buy_menu.extra,
            clonk_engine::ObjectMenuExtra::Value,
            "C4MN_Buy exposes selected value in its footer"
        );
        assert_eq!(
            buy_menu
                .items
                .iter()
                .map(|item| (item.caption.as_str(), item.count, item.value))
                .collect::<Vec<_>>(),
            vec![("Buy Lorry", 1, Some(20))]
        );
        assert_eq!(
            buy_menu.items[0].info_caption,
            "Useful to transport large amounts of material. Holds up to 50 items.",
            "C4ObjectMenu::Refill passes each Buy definition's localized description to C4MenuItem (C4ObjectMenu.cpp:219-233)"
        );
        app.snapshot.hud.messages.clear();
        app.render(&mut rendered)
            .expect("render Tutorial03 Buy menu through the app");
        advance_app_until(&mut app, "Tutorial03 buy-LORY prompt", 240, |app| {
            app_tutorial_message_contains(app, "Buy a lorry")
        });

        // Buy the selected LORY with physical A/Throw. C++ leaves the
        // permanent Buy menu open and refills its C4IDList row at count zero
        // after C4Player::Buy consumes wealth and creates the object inside
        // the base (C4Command.cpp:2005-2035; C4ObjectMenu.cpp:124-129,207-237).
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard.tap(VirtualKeyCode::A).expect("buy selected LORY");
        }
        for _ in 0..20 {
            let bought = app
                .engine
                .snapshot()
                .objects
                .into_iter()
                .any(|object| object.definition_id == "LORY" && object.container == Some(hut));
            if bought
                && app
                    .engine
                    .player(app.local_owner)
                    .is_some_and(|player| player.wealth() == 5)
            {
                break;
            }
            app.update().expect("advance physical LORY purchase");
        }
        let lorry = app
            .engine
            .snapshot()
            .objects
            .into_iter()
            .find(|object| object.definition_id == "LORY")
            .expect("physical A buys Tutorial03 LORY")
            .id;
        assert_eq!(
            app.engine
                .object_snapshot(lorry)
                .expect("bought LORY")
                .container,
            Some(hut)
        );
        let player = app.engine.player(app.local_owner).expect("local player");
        assert_eq!(player.wealth(), 5);
        assert_eq!(player.home_base_material().get("LORY"), Some(&0));
        let (_, buy_menu) = app
            .engine
            .cursor_object_menu(app.local_owner)
            .expect("permanent Buy menu remains after purchase");
        assert_eq!(buy_menu.identification, buy_identification);
        assert_eq!(buy_menu.items[0].count, 0);
        advance_app_until(&mut app, "Tutorial03 close-Buy prompt", 240, |app| {
            app_tutorial_message_contains(app, "close the buy menu")
        });

        // D closes Buy back to auto-context; A activates its first Contents
        // row, then A activates LORY out of HUT3. These remain ordinary
        // physical controls translated by C4Player::InCom while a menu is
        // active (C4Player.cpp:1502-1513; C4ObjectMenu.cpp:279-326).
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard.tap(VirtualKeyCode::D).expect("close Buy menu");
        }
        for _ in 0..20 {
            if app
                .engine
                .cursor_object_menu(app.local_owner)
                .is_some_and(|(_, menu)| menu.identification == context_identification)
            {
                break;
            }
            app.update().expect("restore context after Buy");
        }
        advance_app_until(&mut app, "Tutorial03 Contents prompt", 240, |app| {
            app_tutorial_message_contains(app, "select 'Contents'")
        });
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard.tap(VirtualKeyCode::A).expect("open Contents");
        }
        for _ in 0..20 {
            if app
                .engine
                .cursor_object_menu(app.local_owner)
                .is_some_and(|(_, menu)| menu.identification == contents_identification)
            {
                break;
            }
            app.update().expect("open HUT3 Contents");
        }
        let (_, contents_menu) = app
            .engine
            .cursor_object_menu(app.local_owner)
            .expect("physical D/A opens Contents menu");
        assert_eq!(
            contents_menu
                .items
                .iter()
                .map(|item| (item.caption.as_str(), item.item_id.as_str()))
                .collect::<Vec<_>>(),
            vec![("Activate Lorry", "LORY")]
        );
        // Typed C4GameMessage rejection has its own regression; isolate this
        // Contents-render assertion from that unported overlay.
        app.snapshot.hud.messages.clear();
        app.render(&mut rendered)
            .expect("render Tutorial03 Contents menu through the app");
        advance_app_until(&mut app, "Tutorial03 activate-LORY prompt", 240, |app| {
            app_tutorial_message_contains(app, "Activate the lorry")
        });
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard.tap(VirtualKeyCode::A).expect("activate LORY");
        }
        for _ in 0..40 {
            if app
                .engine
                .object_snapshot(lorry)
                .is_some_and(|object| object.container.is_none())
            {
                break;
            }
            app.update().expect("exit LORY from HUT3");
        }
        assert!(
            app.engine
                .object_snapshot(lorry)
                .is_some_and(|object| object.container.is_none()),
            "Contents activation must exit LORY from HUT3"
        );
        advance_app_until(&mut app, "Tutorial03 leave-HUT3 prompt", 240, |app| {
            app_tutorial_message_contains(app, "exit the hut")
        });

        // Close Contents, then close the restored context menu. Its C++ close
        // command is Exit, so the tutorial-taught two physical D presses exit
        // the building without menu-selection shortcuts (C4Object.cpp:
        // 2044-2062; C4Menu.cpp:317-331; Tutorial03.c4s/Script.c:191-200).
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard.tap(VirtualKeyCode::D).expect("close Contents");
        }
        for _ in 0..20 {
            if app
                .engine
                .cursor_object_menu(app.local_owner)
                .is_some_and(|(_, menu)| menu.identification == context_identification)
            {
                break;
            }
            app.update().expect("restore context after Contents");
        }
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .tap(VirtualKeyCode::D)
                .expect("close context through Exit command");
        }
        for _ in 0..40 {
            if app
                .engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.container.is_none())
            {
                break;
            }
            app.update().expect("exit CLNK from HUT3");
        }
        assert!(
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.container.is_none()),
            "physical D/D route must exit CLNK from HUT3"
        );

        // Once both objects are outside, Tutorial03 teaches the complete real
        // production route: LORY to SAWM, TRE2 through SAWM, ORE1 into LORY,
        // then LORY into FNDR. The engine replay uses the same fresh-player
        // Jump'n'Run/AutoContext preferences, so retain its physical bounds
        // exactly at GameApp::handle_key (Tutorial03.c4s/Script.c:204-284;
        // C4Object.cpp:3573-3740; C4ObjectCom.cpp:247-278).
        advance_app_until(
            &mut app,
            "Tutorial03 closes HUT3's cursor menu",
            20,
            |app| app.engine.cursor_object_menu(app.local_owner).is_none(),
        );
        assert!(
            app.engine.cursor_object_menu(app.local_owner).is_none(),
            "no engine cursor menu may intercept the first world X"
        );
        assert!(
            !app.menu_controls_active_for(app.local_owner),
            "no app menu may intercept the first world X"
        );
        let sawmill = app_object_with_definition(&app, "SAWM").expect("Tutorial03 SAWM");
        let foundry = app_object_with_definition(&app, "FNDR").expect("Tutorial03 FNDR");
        let tree = app
            .engine
            .snapshot()
            .objects
            .into_iter()
            .filter(|object| object.definition_id == "TRE2")
            .min_by_key(|object| (object.position.x - 167).abs())
            .expect("Tutorial03 first full TRE2 near x=167")
            .id;

        advance_app_until(&mut app, "Tutorial03 LORY grab prompt", 180, |app| {
            app_tutorial_message_contains(app, "once to grab the lorry")
        });
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::X)
            .expect("physical X grabs LORY");
        advance_app_until(&mut app, "physical X grabs LORY", 40, |app| {
            app.engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Push" && object.action.target == Some(lorry)
            })
        });
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::Z)
            .expect("physical Z pushes LORY left");
        advance_app_until(&mut app, "LORY reaches the sawmill chute", 240, |app| {
            app.engine.object_snapshot(lorry).is_some_and(|lorry| {
                (194..=218).contains(&lorry.position.x) && (257..=277).contains(&lorry.position.y)
            })
        });
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::Z)
            .expect("release physical Z at the sawmill chute");
        advance_app_until(&mut app, "Tutorial03 LORY release prompt", 180, |app| {
            app_tutorial_message_contains(app, "again to let go of the lorry")
        });
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::X)
            .expect("physical X releases LORY");
        advance_app_until(&mut app, "physical X releases LORY", 40, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk")
        });

        advance_app_until(&mut app, "Tutorial03 first-tree prompt", 180, |app| {
            app_tutorial_message_contains(app, "first tree on the left")
        });
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::Z)
            .expect("physical Z walks to TRE2");
        advance_app_until(
            &mut app,
            "CLNK stands inside the first TRE2 shape",
            120,
            |app| {
                app.engine
                    .object_snapshot(tree)
                    .zip(app.engine.object_snapshot(clonk))
                    .is_some_and(|(tree, clonk)| {
                        (tree.position.x - 20..=tree.position.x + 20).contains(&clonk.position.x)
                            && (tree.position.y - 28..=tree.position.y + 28)
                                .contains(&clonk.position.y)
                    })
            },
        );
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::Z)
            .expect("release physical Z at TRE2");
        advance_app_until(&mut app, "Tutorial03 double-Dig prompt", 180, |app| {
            app_tutorial_message_contains(app, "twice quickly to start chopping")
        });

        // Two immediate physical D taps synthesize COM_Dig_D and must choose
        // Chop, not Script20's intentional too-slow Dig recovery branch
        // (C4Player.cpp:1522-1536; Tutorial03.c4s/Script.c:36-63).
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard.tap(VirtualKeyCode::D).expect("first physical D");
            keyboard.tap(VirtualKeyCode::D).expect("second physical D");
        }
        advance_app_until(&mut app, "physical D/D starts Chop", 80, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Chop")
        });
        advance_app_until(&mut app, "TRE2 is chopped into a vehicle", 800, |app| {
            app.engine
                .object_snapshot(tree)
                .is_some_and(|object| object.category & clonk_engine::CATEGORY_VEHICLE != 0)
        });
        advance_app_until(&mut app, "Tutorial03 felled-tree grab prompt", 180, |app| {
            app_tutorial_message_contains(app, "grab the felled tree")
                && app
                    .engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.action.name == "Walk")
        });
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::X)
            .expect("physical X grabs felled TRE2");
        advance_app_until(&mut app, "physical X grabs felled TRE2", 80, |app| {
            app.engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Push" && object.action.target == Some(tree)
            })
        });
        advance_app_until(&mut app, "Tutorial03 SAWM tree prompt", 180, |app| {
            app_tutorial_message_contains(app, "Push the tree over to the sawmill")
        });
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::C)
            .expect("physical C pushes TRE2 right");
        advance_app_until(&mut app, "TRE2 reaches the SAWM gate", 240, |app| {
            app.engine.object_snapshot(tree).is_some_and(|tree| {
                (239..=259).contains(&tree.position.x) && (254..=279).contains(&tree.position.y)
            })
        });
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::C)
            .expect("release physical C at SAWM");
        advance_app_until(&mut app, "Tutorial03 SAWM Up prompt", 180, |app| {
            app_tutorial_message_contains(app, "press 'up' to push it into the sawmill")
        });
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::S)
            .expect("physical S pushes TRE2 into SAWM");
        advance_app_until(&mut app, "SAWM consumes TRE2", 240, |app| {
            app.engine.object_snapshot(tree).is_none()
        });
        advance_app_until(&mut app, "SAWM's five WOOD enter LORY", 600, |app| {
            app.engine
                .snapshot()
                .objects
                .into_iter()
                .filter(|object| object.definition_id == "WOOD" && object.container == Some(lorry))
                .count()
                >= 5
        });
        assert!(
            app.engine.object_snapshot(sawmill).is_some(),
            "SAWM must survive after consuming TRE2"
        );

        advance_app_until(&mut app, "Tutorial03 creates ORE1", 180, |app| {
            app_tutorial_message_contains(app, "dig out the chunk of ore")
                && app_object_with_definition(app, "ORE1").is_some()
        });
        let ore = app_object_with_definition(&app, "ORE1").expect("Tutorial03 ORE1");
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::C)
            .expect("physical C walks to ORE1");
        advance_app_until(&mut app, "CLNK reaches the ORE1 digging face", 600, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.x >= 480)
        });
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::C)
            .expect("release physical C at ORE1");

        // A single D is buffered to COM_Dig_S after C4DoubleClick. Wait for
        // Dig before pressing X+C so another physical command cannot flush the
        // pending single early (C4Player.cpp:1215-1229,1522-1531).
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::D)
            .expect("physical D starts ORE1 dig");
        advance_app_until(&mut app, "CLNK starts digging toward ORE1", 30, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Dig")
        });
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .press(VirtualKeyCode::X)
                .expect("physical X supplies Dig down");
            keyboard
                .press(VirtualKeyCode::C)
                .expect("physical C supplies Dig right");
        }
        advance_app_until(&mut app, "real dig tunnel collects ORE1", 300, |app| {
            app.engine
                .object_snapshot(ore)
                .is_some_and(|object| object.container == Some(clonk))
        });
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .release(VirtualKeyCode::X)
                .expect("release physical X after ORE1 pickup");
            keyboard
                .release(VirtualKeyCode::C)
                .expect("release physical C after ORE1 pickup");
        }
        advance_app_until(&mut app, "ORE1-carrying CLNK finishes Dig", 80, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk")
        });

        advance_app_until(&mut app, "Tutorial03 ORE1 throw prompt", 180, |app| {
            app_tutorial_message_contains(app, "Throw the chunk of ore into the lorry")
        });
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::Z)
            .expect("physical Z returns to LORY");
        advance_app_until(&mut app, "CLNK reaches LORY's right side", 800, |app| {
            app.engine
                .object_snapshot(clonk)
                .zip(app.engine.object_snapshot(lorry))
                .is_some_and(|(clonk, lorry)| {
                    clonk.position.x >= lorry.position.x + 40
                        && clonk.position.x <= lorry.position.x + 42
                })
        });
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::Z)
            .expect("release physical Z beside LORY");
        assert!(
            app.engine.cursor_object_menu(app.local_owner).is_none(),
            "no engine cursor menu may intercept the world A throw"
        );
        assert!(
            !app.menu_controls_active_for(app.local_owner),
            "no app menu may intercept the world A throw"
        );
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::A)
            .expect("physical A throws ORE1 into LORY");
        advance_app_until(&mut app, "ORE1 enters LORY", 180, |app| {
            app.engine
                .object_snapshot(ore)
                .is_some_and(|object| object.container == Some(lorry))
        });

        advance_app_until(&mut app, "Tutorial03 FNDR prompt", 240, |app| {
            app_tutorial_message_contains(
                app,
                "grab the lorry and push it into the gate of the foundry",
            )
        });
        advance_app_until(&mut app, "CLNK finishes the real Throw", 80, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk")
        });
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::Z)
            .expect("physical Z returns to LORY's grab area");
        advance_app_until(&mut app, "CLNK returns to LORY's grab area", 160, |app| {
            app.engine
                .object_snapshot(clonk)
                .zip(app.engine.object_snapshot(lorry))
                .is_some_and(|(clonk, lorry)| clonk.position.x <= lorry.position.x + 10)
        });
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::Z)
            .expect("release physical Z at LORY");
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::X)
            .expect("physical X grabs loaded LORY");
        advance_app_until(&mut app, "CLNK grabs loaded LORY", 60, |app| {
            app.engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Push" && object.action.target == Some(lorry)
            })
        });

        // S while pushing invokes ObjectComEnter on LORY. Its real Entrance
        // callback transfers ORE1 and WOOD into FNDR before metal production
        // (C4Object.cpp:3702-3710; Lorry.c4d/Script.c:82-91).
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::C)
            .expect("physical C pushes loaded LORY to FNDR");
        advance_app_until(&mut app, "loaded LORY reaches the FNDR gate", 400, |app| {
            app.engine.object_snapshot(lorry).is_some_and(|lorry| {
                (356..=376).contains(&lorry.position.x) && (253..=279).contains(&lorry.position.y)
            })
        });
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::C)
            .expect("release physical C at FNDR");
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::S)
            .expect("physical S pushes LORY into FNDR");
        advance_app_until(&mut app, "loaded LORY enters FNDR", 120, |app| {
            app.engine
                .object_snapshot(lorry)
                .is_some_and(|object| object.container == Some(foundry))
        });
        advance_app_until(&mut app, "Tutorial03 explains FNDR", 240, |app| {
            app_tutorial_message_contains(app, "foundry processes ore and fuel into metal")
        });
        advance_app_until(&mut app, "FNDR produces METL", 600, |app| {
            app_object_with_definition(app, "METL").is_some()
        });
        advance_app_until(&mut app, "Tutorial03 explains METL", 240, |app| {
            app_tutorial_message_contains(app, "Metal can be used to build vehicles")
        });
        advance_app_until(&mut app, "Tutorial03 selects Tutorial04", 240, |app| {
            app.engine.next_mission().path == r"Tutorial.c4f\Tutorial04.c4s"
        });
        advance_app_until(&mut app, "Tutorial03 reaches GameOver", 320, |app| {
            app.snapshot.game_over && app.game_over_dialog.is_some()
        });
        assert!(
            app.snapshot
                .round_results
                .fulfilled_goals
                .iter()
                .any(|goal| goal == "SCRG"),
            "Tutorial03 must fulfill SCRG before GameOver"
        );
        assert_eq!(
            app.engine.next_mission().path,
            r"Tutorial.c4f\Tutorial04.c4s"
        );
        assert!(
            resolve_next_mission_scenario(
                &app.scenario_catalog,
                &app.engine.next_mission().path,
            )
            .is_some(),
            "the focused real-scenario catalog retains Tutorial04 navigation"
        );
        // The typed C4GameMessage guard has a dedicated regression.
        app.snapshot.hud.messages.clear();
        app.render(&mut rendered)
            .expect("render Tutorial03 GameOver through GameApp");
    }

    #[test]
    #[ignore = "over-constrained virtual-play driver; not a production parity oracle"]
    fn app_virtual_keyboard_completes_tutorial04_and_selects_tutorial05() {
        // Tutorial04 teaches the complete physical-key route from HUT2 and
        // CNKT through construction, elevator operation, mining, five GOLD
        // sales, SCRG fulfillment and Tutorial05 selection. Keep every state
        // transition behind GameApp::handle_key so this covers C++ key mapping,
        // menu conversion, DigDouble synthesis, movement and one-slot inventory
        // behavior at the actual app boundary
        // (Tutorial04.c4s/Script.c:40-234; C4Player.cpp:1490-1554;
        // C4ObjectMenu.cpp:279-435).
        let mut app = real_tutorial_app_with_roster(4, "Tutorial 4 app virtual player");
        assert!(
            !app.mouse_control,
            "Tutorial04 DisableMouse=1 must suppress player mouse control"
        );
        assert!(
            !app.option_flags(app.local_owner).mouse_shown,
            "Tutorial04 DisableMouse=1 must remove the in-game mouse Options entry"
        );

        let clonk = app
            .engine
            .crew_cursor(app.local_owner)
            .expect("Tutorial04 selected CLNK");
        let initial = app.engine.snapshot();
        let hut = initial
            .objects
            .iter()
            .find(|object| object.definition_id == "HUT2")
            .expect("Tutorial04 HUT2")
            .id;
        let conkit = initial
            .objects
            .iter()
            .find(|object| object.definition_id == "CNKT")
            .expect("Tutorial04 ready CNKT")
            .id;
        let context_identification = serde_json::from_value(serde_json::json!({ "Int": 14 }))
            .expect("context identification deserializes");
        let contents_identification = serde_json::from_value(serde_json::json!({ "Int": 18 }))
            .expect("contents identification deserializes");
        let construction_identification =
            serde_json::from_value(serde_json::json!({ "C4Id": "CXCN" }))
                .expect("construction identification deserializes");

        advance_app_until(&mut app, "Tutorial04 ready base and Clonk", 180, |app| {
            app.engine
                .object_snapshot(hut)
                .is_some_and(|object| object.base == app.local_owner)
                && app.engine.object_snapshot(clonk).is_some_and(|object| {
                    object.container.is_none() && object.action.name == "Walk"
                })
        });
        assert_eq!(
            app.engine
                .object_snapshot(hut)
                .expect("ready HUT2")
                .position,
            Vector2::new(586, 242),
            "seed-zero Tutorial04 must retain the HUT2 position used by its entrance lesson"
        );
        assert_eq!(
            app.engine
                .object_snapshot(conkit)
                .expect("ready CNKT")
                .container,
            Some(hut),
            "the real ready CNKT must begin inside HUT2"
        );
        advance_app_until(&mut app, "Tutorial04 enter-home-base prompt", 240, |app| {
            app_tutorial_message_contains(app, "Enter your home base")
        });

        // HUT2's seed-zero entrance is [568,584) x [250,267). Walk to its
        // center with physical Z, release it, then use physical S/Up so
        // ObjectComUp chooses Enter before Jump (Hut2 DefCore Entrance;
        // C4ObjectCom.cpp:335-350).
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .press(VirtualKeyCode::Z)
                .expect("physical Z walks toward HUT2");
        }
        advance_app_until(&mut app, "CLNK aligned with HUT2 entrance", 30, |app| {
            app.engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Walk"
                    && (574..578).contains(&object.position.x)
                    && (250..267).contains(&object.position.y)
            })
        });
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .release(VirtualKeyCode::Z)
                .expect("release physical Z at HUT2");
            keyboard
                .tap(VirtualKeyCode::S)
                .expect("physical S enters HUT2");
        }
        advance_app_until(&mut app, "CLNK entered HUT2", 50, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.container == Some(hut))
        });
        advance_app_until(&mut app, "Tutorial04 Contents prompt", 240, |app| {
            app_tutorial_message_contains(app, "select 'Contents'")
        });
        advance_app_until(&mut app, "HUT2 auto-context menu", 30, |app| {
            app.engine
                .cursor_object_menu(app.local_owner)
                .is_some_and(|(_, menu)| menu.identification == context_identification)
        });
        {
            let (_, menu) = app
                .engine
                .cursor_object_menu(app.local_owner)
                .expect("HUT2 context menu");
            assert_eq!(menu.selection, 0);
            assert_eq!(
                menu.items.first().map(|item| item.caption.as_str()),
                Some("Contents")
            );
        }

        // A is Throw/MenuEnter. The real context's first row opens Contents;
        // ready-material insertion keeps the later-created CNKT before FLAG.
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .tap(VirtualKeyCode::A)
                .expect("physical A opens HUT2 Contents");
        }
        advance_app_until(&mut app, "HUT2 Contents menu", 20, |app| {
            app.engine
                .cursor_object_menu(app.local_owner)
                .is_some_and(|(_, menu)| menu.identification == contents_identification)
        });
        advance_app_until(
            &mut app,
            "Tutorial04 take-construction-kit prompt",
            240,
            |app| app_tutorial_message_contains(app, "Take the construction kit"),
        );
        {
            let (_, menu) = app
                .engine
                .cursor_object_menu(app.local_owner)
                .expect("HUT2 Contents remains open");
            assert_eq!(menu.selection, 0);
            assert_eq!(
                menu.items.first().map(|item| item.item_id.as_str()),
                Some("CNKT"),
                "physical A must target the real first Contents row"
            );
        }
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .tap(VirtualKeyCode::A)
                .expect("physical A takes CNKT");
        }
        advance_app_until(&mut app, "CLNK carries CNKT", 60, |app| {
            app.engine
                .object_snapshot(conkit)
                .is_some_and(|object| object.container == Some(clonk))
        });
        advance_app_until(
            &mut app,
            "Tutorial04 close-menu-and-exit prompt",
            240,
            |app| app_tutorial_message_contains(app, "close the menu and exit"),
        );

        // Physical D closes Contents. AutoContextMenu returns on the next
        // player tick with the carried-CNKT Put row selected; physical S wraps
        // that first row to Exit and A activates it through ordinary menu
        // controls (C4Menu.cpp:433-480,1040-1069).
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .tap(VirtualKeyCode::D)
                .expect("physical D closes Contents");
        }
        advance_app_until(&mut app, "HUT2 context restored", 30, |app| {
            app.engine
                .cursor_object_menu(app.local_owner)
                .is_some_and(|(_, menu)| menu.identification == context_identification)
        });
        {
            let (_, menu) = app
                .engine
                .cursor_object_menu(app.local_owner)
                .expect("HUT2 context restored around carried CNKT");
            assert_eq!(menu.selection, 0);
            assert_eq!(
                menu.items.first().map(|item| item.caption.as_str()),
                Some("Put")
            );
            assert_eq!(
                menu.items.last().map(|item| item.caption.as_str()),
                Some("Exit")
            );
        }
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .tap(VirtualKeyCode::S)
                .expect("physical S wraps context selection to Exit");
            keyboard
                .tap(VirtualKeyCode::A)
                .expect("physical A activates context Exit");
        }
        advance_app_until(&mut app, "CNKT-carrying CLNK exited HUT2", 60, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.container.is_none() && object.action.name == "Walk")
        });
        advance_app_until(&mut app, "Tutorial04 clear-area prompt", 240, |app| {
            app_tutorial_message_contains(app, "clear area to the left")
        });

        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .press(VirtualKeyCode::Z)
                .expect("physical Z walks to elevator site");
        }
        advance_app_until(&mut app, "CLNK reached elevator site", 120, |app| {
            app.engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Walk" && (490..=510).contains(&object.position.x)
            })
        });
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .release(VirtualKeyCode::Z)
                .expect("release physical Z at elevator site");
        }
        advance_app_until(&mut app, "Tutorial04 double-Dig prompt", 240, |app| {
            app_tutorial_message_contains(app, "twice quickly to open the construction menu")
        });
        assert!(
            app.engine
                .snapshot()
                .objects
                .iter()
                .all(|object| object.definition_id != "ELEV"),
            "Tutorial04 must not have an ELEV before CNKT activation"
        );

        // Two complete physical D edges inside C4DoubleClick's window become
        // COM_Dig_D. CNKT::Activate opens CXCN and fills its one known ELEV
        // row from GetPlrKnowledge without any menu or inventory mutation.
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .tap(VirtualKeyCode::D)
                .expect("first physical D at elevator site");
            keyboard
                .tap(VirtualKeyCode::D)
                .expect("second physical D at elevator site");
        }
        advance_app_until(&mut app, "CNKT CXCN menu", 20, |app| {
            app.engine
                .cursor_object_menu(app.local_owner)
                .is_some_and(|(_, menu)| menu.identification == construction_identification)
        });
        advance_app_until(&mut app, "Tutorial04 create-ELEV prompt", 240, |app| {
            app_tutorial_message_contains(app, "Create an elevator construction site")
        });
        {
            let (_, menu) = app
                .engine
                .cursor_object_menu(app.local_owner)
                .expect("physical D/D opens CNKT construction menu");
            assert_eq!(menu.identification, construction_identification);
            assert_eq!(menu.symbol_id, "CXCN");
            assert_eq!(menu.command_object, Some(conkit));
            assert_eq!(menu.extra, clonk_engine::ObjectMenuExtra::Components);
            assert_eq!(menu.selection, 0);
            assert_eq!(menu.items.len(), 1);
            assert_eq!(menu.items[0].item_id, "ELEV");
            assert_eq!(menu.items[0].caption, "Construction: Elevator");
            assert_eq!(
                menu.items[0].components,
                vec![
                    clonk_engine::ObjectMenuComponent {
                        definition_id: "WOOD".to_string(),
                        count: 4,
                    },
                    clonk_engine::ObjectMenuComponent {
                        definition_id: "METL".to_string(),
                        count: 2,
                    },
                ],
                "the app-visible CXCN row must retain ELEV's C++ component order"
            );
        }
        app.snapshot.hud.messages.clear();
        let mut rendered = vec![0_u8; 320 * 200 * 4];
        app.render(&mut rendered)
            .expect("render Tutorial04 CXCN through the app");

        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .tap(VirtualKeyCode::A)
                .expect("physical A creates ELEV construction");
        }
        advance_app_until(
            &mut app,
            "ELEV construction created and CNKT consumed",
            30,
            |app| {
                let elevator_exists = app
                    .engine
                    .snapshot()
                    .objects
                    .iter()
                    .any(|object| object.definition_id == "ELEV" && object.status.is_active());
                let conkit_removed = app
                    .engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| !object.contents.contains(&conkit));
                elevator_exists && conkit_removed
            },
        );
        let elevator = app
            .engine
            .snapshot()
            .objects
            .into_iter()
            .find(|object| object.definition_id == "ELEV" && object.status.is_active())
            .expect("physical A creates active ELEV");
        assert_eq!(elevator.owner, app.local_owner);
        assert!((490..=510).contains(&elevator.position.x));
        assert!(
            (1..100_000).contains(&elevator.construction),
            "CNKT must create an incomplete construction site"
        );
        assert!(
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| !object.contents.contains(&conkit)),
            "CreateConstructionSite consumes the real CNKT"
        );
        assert!(
            app.engine.cursor_object_menu(app.local_owner).is_none(),
            "removing CNKT closes the menu it owns"
        );

        // Down at an OCF_Construct object queues Build before Grab, and the
        // Build procedure advances construction until ELEV creates ELEC
        // (C4ObjectCom.cpp:573-588,690-697; C4Object.cpp:5010-5043;
        // Tutorial04.c4s/Script.c:119-137).
        advance_app_until(&mut app, "Tutorial04 build-ELEV prompt", 240, |app| {
            app_tutorial_message_contains(app, "press 'down' to start working")
        });
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::X)
            .expect("physical X starts ELEV construction");
        advance_app_until(&mut app, "CLNK starts building ELEV", 30, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Build")
        });
        advance_app_until(&mut app, "ELEV finishes and creates ELEC", 720, |app| {
            app_object_with_definition(app, "ELEC").is_some()
                && app
                    .engine
                    .object_snapshot(elevator.id)
                    .is_some_and(|object| object.construction == 100_000)
        });
        let elevator_case =
            app_object_with_definition(&app, "ELEC").expect("completed ELEV creates ELEC");

        advance_app_until(&mut app, "Tutorial04 grab-ELEC prompt", 240, |app| {
            app_tutorial_message_contains(app, "Grab the elevator case")
        });
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::X)
            .expect("physical X grabs ELEC");
        advance_app_until(&mut app, "CLNK grabs ELEC", 60, |app| {
            app.engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Push" && object.action.target == Some(elevator_case)
            })
        });

        // ELEC maps held Dig to downward travel and held Up to upward travel;
        // Tutorial04 observes the real crew positions before changing prompts
        // (Tutorial04.c4s/Script.c:139-166).
        advance_app_until(&mut app, "Tutorial04 drill-shaft prompt", 240, |app| {
            app_tutorial_message_contains(app, "Hold down the 'dig' key")
        });
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::D,
            "ELEC drills CLNK to the shaft bottom",
            360,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.position.y >= 340)
            },
        );
        advance_app_until(&mut app, "Tutorial04 ride-up prompt", 240, |app| {
            app_tutorial_message_contains(app, "ride the elevator back up")
        });
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::S,
            "ELEC carries CLNK back to the surface",
            240,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.position.y <= 270)
            },
        );
        advance_app_until(&mut app, "Tutorial04 let-go prompt", 240, |app| {
            app_tutorial_message_contains(app, "Let go of the elevator case")
        });
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .tap(VirtualKeyCode::X)
                .expect("first physical X to ungrab ELEC");
            keyboard
                .tap(VirtualKeyCode::X)
                .expect("second physical X to ungrab ELEC");
        }
        advance_app_until(&mut app, "CLNK lets go of ELEC", 60, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name != "Push")
        });
        advance_app_until(&mut app, "Tutorial04 spawns surface TFLN", 240, |app| {
            app_tutorial_message_contains(app, "Walk back to the cabin")
                && app_object_with_definition(app, "TFLN").is_some()
        });
        let first_flint = app_object_with_definition(&app, "TFLN")
            .expect("preserve Tutorial04's exact first TFLN identity");

        // The shaft lip alternates Jump and Scale. Re-emitting physical Right
        // on Scale and physical Up after landing follows the C++ transitions;
        // the exiting TFLN is collected naturally before its fuse expires
        // (C4Object.cpp:3618-3628,4284-4299,4823-4855;
        // Tutorial04.c4s/Script.c:167-179).
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::C)
            .expect("physical C exits the shaft toward TFLN");
        let mut previous_action = String::new();
        for _ in 0..60 {
            if app_clonk_carries(&app, clonk, "TFLN") {
                break;
            }
            let clonk_now = app
                .engine
                .object_snapshot(clonk)
                .expect("CLNK survives the shaft exit");
            let action = clonk_now.action.name;
            let entered_scale =
                action.starts_with("Scale") && !previous_action.starts_with("Scale");
            let left_scale_in_flight = action == "Jump" && previous_action.starts_with("Scale");
            let landed = action == "Walk" && previous_action != "Walk";
            if entered_scale {
                let mut keyboard = AppVirtualKeyboard::new(&mut app);
                keyboard
                    .release(VirtualKeyCode::C)
                    .expect("release physical C on Scale");
                keyboard
                    .press(VirtualKeyCode::C)
                    .expect("repress physical C on Scale");
            } else if (landed || left_scale_in_flight) && clonk_now.position.x < 550 {
                AppVirtualKeyboard::new(&mut app)
                    .tap(VirtualKeyCode::S)
                    .expect("physical S jumps out of the shaft");
            }
            previous_action = action;
            app.update().expect("advance CLNK toward surface TFLN");
        }
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::C)
            .expect("release physical C after TFLN pickup");
        assert!(
            app_clonk_carries(&app, clonk, "TFLN"),
            "CLNK must naturally collect the real exiting TFLN"
        );

        hold_app_key_until(
            &mut app,
            VirtualKeyCode::Z,
            "TFLN-carrying CLNK returns toward ELEC",
            120,
            |app| {
                app_tutorial_message_contains(app, "Ride back down into the mine")
                    || app
                        .engine
                        .object_snapshot(clonk)
                        .zip(app.engine.object_snapshot(elevator_case))
                        .is_some_and(|(clonk, elevator)| {
                            clonk.position.x <= elevator.position.x + 5
                        })
            },
        );
        advance_app_until(&mut app, "Tutorial04 TFLN ride-down prompt", 240, |app| {
            app_tutorial_message_contains(app, "Ride back down into the mine")
                && app_clonk_carries(app, clonk, "TFLN")
        });

        let (clonk_x, elevator_x) = app
            .engine
            .object_snapshot(clonk)
            .zip(app.engine.object_snapshot(elevator_case))
            .map(|(clonk, elevator)| (clonk.position.x, elevator.position.x))
            .expect("CLNK and ELEC survive the surface return");
        if clonk_x < elevator_x - 5 {
            hold_app_key_until(
                &mut app,
                VirtualKeyCode::C,
                "CLNK aligns with ELEC from the left",
                120,
                |app| {
                    app.engine
                        .object_snapshot(clonk)
                        .zip(app.engine.object_snapshot(elevator_case))
                        .is_some_and(|(clonk, elevator)| {
                            (clonk.position.x - elevator.position.x).abs() <= 5
                        })
                },
            );
        } else if clonk_x > elevator_x + 5 {
            hold_app_key_until(
                &mut app,
                VirtualKeyCode::Z,
                "CLNK aligns with ELEC from the right",
                120,
                |app| {
                    app.engine
                        .object_snapshot(clonk)
                        .zip(app.engine.object_snapshot(elevator_case))
                        .is_some_and(|(clonk, elevator)| {
                            (clonk.position.x - elevator.position.x).abs() <= 5
                        })
                },
            );
        }
        if let Some(clonk_now) = app.engine.object_snapshot(clonk) {
            if clonk_now.action.name.starts_with("Scale") {
                let away = if clonk_now.direction == Direction::Left {
                    VirtualKeyCode::C
                } else {
                    VirtualKeyCode::Z
                };
                AppVirtualKeyboard::new(&mut app)
                    .tap(away)
                    .expect("physical direction leaves Scale beside ELEC");
            }
        }
        advance_app_until(
            &mut app,
            "TFLN-carrying CLNK settles beside ELEC",
            120,
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
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::X)
            .expect("physical X grabs ELEC with TFLN");
        advance_app_until(&mut app, "TFLN-carrying CLNK grabs ELEC", 60, |app| {
            app.engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Push" && object.action.target == Some(elevator_case)
            })
        });
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::D,
            "ELEC carries TFLN-carrying CLNK underground",
            360,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.position.y >= 340)
            },
        );
        advance_app_until(&mut app, "Tutorial04 gold-tunnel prompt", 240, |app| {
            app_tutorial_message_contains(app, "Dig a tunnel all the way")
        });
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .tap(VirtualKeyCode::X)
                .expect("first physical X to ungrab underground");
            keyboard
                .tap(VirtualKeyCode::X)
                .expect("second physical X to ungrab underground");
        }
        advance_app_until(&mut app, "CLNK lets go underground", 60, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk")
        });

        // A physical Dig press followed by held Left+Down steers DFA_DIG into
        // Script175's 80x80 gold rectangle. Bottom contact redirects the first
        // diagonal to Left, so a fresh Down edge restores DownLeft exactly as
        // C++ does (Tutorial04.c4s/Script.c:180-207;
        // C4ObjectCom.cpp:353-362; C4Object.cpp:3573-3631,4354-4368).
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::D)
            .expect("physical D starts the gold tunnel");
        advance_app_until(&mut app, "CLNK starts digging toward GOLD", 30, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Dig")
        });
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .press(VirtualKeyCode::Z)
                .expect("physical Z steers Dig left");
            keyboard
                .press(VirtualKeyCode::X)
                .expect("physical X steers Dig down");
        }
        for _ in 0..12 {
            app.update().expect("advance initial tunnel Dig");
        }
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .release(VirtualKeyCode::X)
                .expect("release physical X to refresh diagonal Dig");
            keyboard
                .press(VirtualKeyCode::X)
                .expect("repress physical X for diagonal Dig");
        }
        let mut reached_gold_face = false;
        for _ in 0..360 {
            let clonk_now = app
                .engine
                .object_snapshot(clonk)
                .expect("CLNK survives the real gold tunnel");
            if (clonk_now.action.name == "Dig" && clonk_now.position.x <= 432)
                || (clonk_now.action.name == "Walk"
                    && (357..437).contains(&clonk_now.position.x)
                    && (348..440).contains(&clonk_now.position.y))
            {
                reached_gold_face = true;
                break;
            }
            if clonk_now.command_direction == CommandDirection::Left {
                let mut keyboard = AppVirtualKeyboard::new(&mut app);
                keyboard
                    .release(VirtualKeyCode::X)
                    .expect("release physical X after bottom redirect");
                keyboard
                    .press(VirtualKeyCode::X)
                    .expect("repress physical X toward GOLD");
            }
            app.update().expect("advance real gold tunnel");
        }
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .release(VirtualKeyCode::X)
                .expect("release physical X at the gold face");
            keyboard
                .release(VirtualKeyCode::Z)
                .expect("release physical Z at the gold face");
        }
        assert!(
            reached_gold_face,
            "physical-key Dig must naturally stop at Tutorial04's solid-GOLD face; clonk={:?}",
            app.engine.object_snapshot(clonk)
        );
        advance_app_until(&mut app, "Tutorial04 blast-GOLD prompt", 120, |app| {
            app_tutorial_message_contains(app, "struck solid gold")
        });
        advance_app_until(&mut app, "CLNK stops Dig at the gold face", 40, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk")
        });

        let safe_x = app
            .engine
            .object_snapshot(clonk)
            .expect("CLNK survives the gold tunnel")
            .position
            .x
            + 24;
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::C,
            "TFLN-carrying CLNK reaches a safe throwing distance",
            80,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.position.x >= safe_x)
            },
        );
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::Z)
            .expect("physical Z faces CLNK toward the gold vein");
        app.update()
            .expect("settle left-facing CLNK before TFLN throw");
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::A)
            .expect("physical A throws TFLN toward GOLD");
        advance_app_until(&mut app, "TFLN leaves CLNK inventory", 30, |app| {
            !app_clonk_carries(app, clonk, "TFLN")
        });
        for _ in 0..180 {
            if app_object_with_definition(&app, "GOLD").is_some() {
                break;
            }
            app.update().expect("advance first TFLN toward GOLD blast");
        }
        assert!(
            app_object_with_definition(&app, "GOLD").is_some(),
            "first TFLN must free real GOLD objects"
        );
        assert!(
            app.engine.object_snapshot(first_flint).is_none(),
            "the exact first TFLN must detonate"
        );
        for _ in 0..100 {
            app.update().expect("settle the first real GOLD blast");
        }
        if app
            .engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Hangle")
        {
            AppVirtualKeyboard::new(&mut app)
                .tap(VirtualKeyCode::X)
                .expect("physical X drops CLNK from the tunnel ceiling");
            advance_app_until(&mut app, "CLNK drops into the GOLD pocket", 60, |app| {
                app.engine.object_snapshot(clonk).is_some_and(|object| {
                    object.action.name == "Walk" || object.action.name.starts_with("Scale")
                })
            });
        }
        app_collect_one_gold_around_blast_debris(&mut app, clonk);
        let sold_gold = app
            .engine
            .object_snapshot(clonk)
            .expect("GOLD-carrying CLNK survives collection")
            .contents
            .into_iter()
            .find(|&object_id| {
                app.engine
                    .object_snapshot(object_id)
                    .is_some_and(|object| object.definition_id == "GOLD")
            })
            .expect("CLNK carries the exact GOLD object to be sold");
        let wealth_before_sale = app
            .engine
            .player(app.local_owner)
            .expect("local player exists before sale")
            .wealth();

        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::C)
            .expect("physical C returns GOLD-carrying CLNK to ELEC");
        let mut returned_to_elevator = false;
        let mut previous_action = String::new();
        for _ in 0..360 {
            let clonk_now = app
                .engine
                .object_snapshot(clonk)
                .expect("GOLD-carrying CLNK survives the tunnel return");
            returned_to_elevator =
                app.engine
                    .object_snapshot(elevator_case)
                    .is_some_and(|elevator| {
                        clonk_now.action.name == "Walk"
                            && (clonk_now.position.x - elevator.position.x).abs() <= 5
                    });
            if returned_to_elevator {
                break;
            }
            let action = clonk_now.action.name;
            if action.starts_with("Scale") && !previous_action.starts_with("Scale") {
                let mut keyboard = AppVirtualKeyboard::new(&mut app);
                keyboard
                    .release(VirtualKeyCode::C)
                    .expect("release physical C on tunnel Scale");
                keyboard
                    .press(VirtualKeyCode::C)
                    .expect("repress physical C to leave tunnel Scale");
            } else if action == "Hangle" && previous_action != "Hangle" {
                AppVirtualKeyboard::new(&mut app)
                    .tap(VirtualKeyCode::X)
                    .expect("physical X drops from the tunnel ceiling");
            }
            previous_action = action;
            app.update().expect("advance GOLD-carrying CLNK to ELEC");
        }
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::C)
            .expect("release physical C beside ELEC");
        assert!(
            returned_to_elevator,
            "GOLD-carrying CLNK must return to ELEC; clonk={:?}, elevator={:?}",
            app.engine.object_snapshot(clonk),
            app.engine.object_snapshot(elevator_case)
        );
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::X)
            .expect("physical X grabs ELEC with GOLD");
        advance_app_until(&mut app, "GOLD-carrying CLNK grabs ELEC", 60, |app| {
            app.engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Push" && object.action.target == Some(elevator_case)
            })
        });
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::S,
            "ELEC raises GOLD-carrying CLNK to the surface",
            300,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.position.y <= 270)
            },
        );
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .tap(VirtualKeyCode::X)
                .expect("first physical X to ungrab ELEC with GOLD");
            keyboard
                .tap(VirtualKeyCode::X)
                .expect("second physical X to ungrab ELEC with GOLD");
        }
        advance_app_until(&mut app, "GOLD-carrying CLNK lets go of ELEC", 60, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name != "Push")
        });

        // Cross the surface lip using only held Right and real Up jump edges,
        // then enter HUT2 through its actual entrance. BaseAutoSell removes the
        // GOLD and increments wealth by five (C4Object.cpp:3618-3628,
        // 4284-4299,4823-4855; Tutorial04.c4s/Script.c:214-234).
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::C)
            .expect("physical C climbs the shaft lip with GOLD");
        let mut previous_action = String::new();
        for _ in 0..240 {
            let clonk_now = app
                .engine
                .object_snapshot(clonk)
                .expect("GOLD-carrying CLNK survives the shaft climb");
            if clonk_now.position.x >= 558 {
                break;
            }
            let action = clonk_now.action.name;
            let entered_scale =
                action.starts_with("Scale") && !previous_action.starts_with("Scale");
            let left_scale_in_flight = action == "Jump" && previous_action.starts_with("Scale");
            let landed = action == "Walk" && previous_action != "Walk";
            if entered_scale {
                let mut keyboard = AppVirtualKeyboard::new(&mut app);
                keyboard
                    .release(VirtualKeyCode::C)
                    .expect("release physical C on shaft Scale");
                keyboard
                    .press(VirtualKeyCode::C)
                    .expect("repress physical C on shaft Scale");
            } else if landed || left_scale_in_flight {
                AppVirtualKeyboard::new(&mut app)
                    .tap(VirtualKeyCode::S)
                    .expect("physical S jumps the shaft lip");
            }
            previous_action = action;
            app.update()
                .expect("advance GOLD-carrying CLNK over the shaft lip");
        }
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::C)
            .expect("release physical C on the cabin hill");
        assert!(
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.x >= 558),
            "GOLD-carrying CLNK must reach the cabin hill"
        );
        advance_app_until(
            &mut app,
            "GOLD-carrying CLNK lands beside HUT2",
            80,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.action.name == "Walk")
            },
        );
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::Z,
            "GOLD-carrying CLNK aligns with HUT2's entrance",
            80,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.position.x <= 570)
            },
        );
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::S)
            .expect("physical S enters HUT2 with GOLD");
        advance_app_until(&mut app, "GOLD-carrying CLNK enters HUT2", 60, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.container == Some(hut))
        });
        advance_app_until(&mut app, "HUT2 sells the first GOLD chunk", 80, |app| {
            app.engine
                .player(app.local_owner)
                .is_some_and(|player| player.wealth() == wealth_before_sale + 5)
                && app.engine.object_snapshot(sold_gold).is_none()
        });
        assert_eq!(
            app.engine
                .player(app.local_owner)
                .expect("local player survives sale")
                .wealth(),
            wealth_before_sale + 5,
            "GOLD value five is added exactly once (C4Object.cpp:970-997; C4Player.cpp:866-897; GOLD DefCore.txt:13,18)"
        );
        assert!(
            app.engine.object_snapshot(sold_gold).is_none(),
            "BaseAutoSell must remove the sold GOLD object"
        );
        assert!(
            !app_clonk_carries(&app, clonk, "GOLD"),
            "BaseAutoSell must remove the first GOLD from CLNK"
        );

        // Script200 creates three replacement TFLNs in HUT2 after the first
        // sale, then Script201/250 asks the contained Clonk to take one and
        // earn 25 gold points (Tutorial04.c4s/Script.c:214-231). Drive the
        // actual context/Contents menus: Down selects the TFLN row and
        // Special2 chooses Command2/EnterAll (C4Menu.cpp:433-440,498-523,
        // 1047-1054).
        advance_app_until(
            &mut app,
            "Tutorial04 creates three replacement TFLNs in HUT2",
            400,
            |app| {
                app_tutorial_message_contains(app, "more T-Flints")
                    && app_object_contents_count(app, hut, "TFLN") == 3
            },
        );
        advance_app_until(
            &mut app,
            "Tutorial04 replacement-flint Contents prompt",
            400,
            |app| app_tutorial_message_contains(app, "Select 'Contents'"),
        );
        advance_app_until(&mut app, "HUT2 replacement-flint context menu", 30, |app| {
            app.engine
                .cursor_object_menu(app.local_owner)
                .is_some_and(|(_, menu)| menu.identification == context_identification)
        });
        {
            let (_, menu) = app
                .engine
                .cursor_object_menu(app.local_owner)
                .expect("HUT2 replacement-flint context menu");
            assert_eq!(menu.selection, 0);
            assert_eq!(
                menu.items.first().map(|item| item.caption.as_str()),
                Some("Contents")
            );
        }
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::A)
            .expect("physical A opens replacement-flint Contents");
        advance_app_until(&mut app, "HUT2 replacement-flint Contents", 30, |app| {
            app.engine
                .cursor_object_menu(app.local_owner)
                .is_some_and(|(_, menu)| menu.identification == contents_identification)
        });

        let contents_rows = app
            .engine
            .cursor_object_menu(app.local_owner)
            .expect("replacement-flint Contents menu")
            .1
            .items
            .len();
        for _ in 0..contents_rows {
            if app_selected_object_menu_item(&app).is_some_and(|item| item.item_id == "TFLN") {
                break;
            }
            AppVirtualKeyboard::new(&mut app)
                .tap(VirtualKeyCode::X)
                .expect("physical X selects the next Contents row");
        }
        assert_eq!(
            app_selected_object_menu_item(&app).map(|item| item.item_id.as_str()),
            Some("TFLN"),
            "physical Down navigation must select the replacement TFLN row"
        );
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::F)
            .expect("physical F takes all replacement TFLNs that fit");
        advance_app_until(
            &mut app,
            "C++ nonspecial capacity keeps one TFLN on CLNK and two in HUT2",
            120,
            |app| {
                app_object_contents_count(app, clonk, "TFLN") == 1
                    && app_object_contents_count(app, hut, "TFLN") == 2
            },
        );

        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::D)
            .expect("physical D closes replacement-flint Contents");
        advance_app_until(&mut app, "HUT2 context restored after TFLN", 30, |app| {
            app.engine
                .cursor_object_menu(app.local_owner)
                .is_some_and(|(_, menu)| menu.identification == context_identification)
        });
        {
            let (_, menu) = app
                .engine
                .cursor_object_menu(app.local_owner)
                .expect("HUT2 context restored around carried TFLN");
            assert_eq!(menu.selection, 0);
            assert_eq!(
                menu.items.first().map(|item| item.caption.as_str()),
                Some("Put")
            );
            assert_eq!(
                menu.items.last().map(|item| item.caption.as_str()),
                Some("Exit")
            );
        }
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .tap(VirtualKeyCode::S)
                .expect("physical S wraps replacement-flint context to Exit");
            keyboard
                .tap(VirtualKeyCode::A)
                .expect("physical A exits HUT2 with replacement TFLN");
        }
        advance_app_until(&mut app, "replacement-TFLN CLNK exits HUT2", 60, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.container.is_none())
        });
        advance_app_until(
            &mut app,
            "Tutorial04 states its 25-gold objective",
            640,
            |app| app_tutorial_message_contains(app, "Gain 25"),
        );
        assert_eq!(
            app.engine
                .player(app.local_owner)
                .expect("local player survives replacement-flint withdrawal")
                .wealth(),
            wealth_before_sale + 5,
            "withdrawing replacement TFLN must not change the 25-gold objective's wealth"
        );
        assert!(
            app_clonk_carries(&app, clonk, "TFLN"),
            "CLNK must exit with exactly one usable replacement TFLN"
        );
        let replacement_flint = app
            .engine
            .object_snapshot(clonk)
            .expect("replacement-TFLN CLNK survives HUT2 exit")
            .contents
            .into_iter()
            .find(|item| {
                app.engine
                    .object_snapshot(*item)
                    .is_some_and(|item| item.definition_id == "TFLN")
            })
            .expect("preserve the exact replacement TFLN withdrawn from HUT2");

        // Return over the real shaft lip, grab the same ELEC, and descend to
        // the first blast tunnel. C++ directs movement to the pushed ELEC and
        // turns Dig into its Down control; DownDouble releases it at the floor
        // (C4Player.cpp:1397-1443,1453-1553; C4Object.cpp:3321-3337,
        // 3520-3567; Tutorial04.c4s/Script.c:214-234).
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::Z,
            "replacement-TFLN CLNK returns to ELEC",
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
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::C)
            .expect("physical C crosses the replacement-flint shaft lip");
        let mut previous_action = String::new();
        for _ in 0..120 {
            let clonk_now = app
                .engine
                .object_snapshot(clonk)
                .expect("replacement-TFLN CLNK survives the shaft lip");
            if clonk_now.action.name == "Walk" && clonk_now.position.x >= 505 {
                break;
            }
            let action = clonk_now.action.name;
            let entered_scale =
                action.starts_with("Scale") && !previous_action.starts_with("Scale");
            let left_scale_in_flight = action == "Jump" && previous_action.starts_with("Scale");
            let landed = action == "Walk" && previous_action != "Walk";
            if entered_scale {
                let mut keyboard = AppVirtualKeyboard::new(&mut app);
                keyboard
                    .release(VirtualKeyCode::C)
                    .expect("release physical C on replacement-flint Scale");
                keyboard
                    .press(VirtualKeyCode::C)
                    .expect("repress physical C on replacement-flint Scale");
            } else if landed || left_scale_in_flight {
                AppVirtualKeyboard::new(&mut app)
                    .tap(VirtualKeyCode::S)
                    .expect("physical S jumps the replacement-flint shaft lip");
            }
            previous_action = action;
            app.update()
                .expect("advance replacement-TFLN CLNK across shaft lip");
        }
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::C)
            .expect("release physical C after replacement-flint shaft lip");
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::Z,
            "replacement-TFLN CLNK stands beside ELEC",
            80,
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
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::X)
            .expect("physical X grabs ELEC with the replacement TFLN");
        advance_app_until(
            &mut app,
            "replacement-TFLN CLNK grabs exact ELEC",
            60,
            |app| {
                app.engine.object_snapshot(clonk).is_some_and(|object| {
                    object.action.name == "Push" && object.action.target == Some(elevator_case)
                })
            },
        );
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::D,
            "ELEC carries replacement-TFLN CLNK underground",
            360,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.position.y >= 340)
            },
        );
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .tap(VirtualKeyCode::X)
                .expect("first physical X releases ELEC underground with TFLN");
            keyboard
                .tap(VirtualKeyCode::X)
                .expect("second physical X releases ELEC underground with TFLN");
        }
        advance_app_until(
            &mut app,
            "replacement-TFLN CLNK lets go underground",
            60,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.action.name == "Walk")
            },
        );
        app_tutorial04_blast_next_gold_face(&mut app, clonk, replacement_flint, 414, 2);
        // The fixed corrected MapSeed face releases exactly two GOLD objects;
        // the helper pins that C++ output rather than accepting any growth.
        for _ in 0..120 {
            app.update()
                .expect("settle the replacement-TFLN blast pocket");
        }
        if app
            .engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Hangle")
        {
            AppVirtualKeyboard::new(&mut app)
                .tap(VirtualKeyCode::X)
                .expect("physical X drops CLNK into the second blast pocket");
            advance_app_until(&mut app, "CLNK drops into second blast pocket", 60, |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.action.name == "Walk")
            });
        }

        app_collect_one_gold_around_blast_debris(&mut app, clonk);
        assert_eq!(
            app_object_contents_count(&app, clonk, "GOLD"),
            1,
            "C++ one-nonspecial-slot CLNK must carry exactly one GOLD per trip"
        );

        // Sell one chunk, then physically withdraw a second replacement TFLN.
        // Its fixed corrected-seed face releases exactly one more GOLD object;
        // the already exposed loose chunks supply the remaining sale trips.
        app_carry_tutorial04_gold_to_hut(&mut app, clonk, elevator_case, hut, 10);
        let additional_flint = app_take_tutorial04_flint_from_hut(&mut app, clonk, hut);
        app_return_tutorial04_from_hut_to_tunnel(&mut app, clonk, elevator_case, hut);
        app_tutorial04_blast_next_gold_face(&mut app, clonk, additional_flint, 402, 1);
        for _ in 0..120 {
            app.update()
                .expect("settle the additional-TFLN blast pocket");
        }
        if app
            .engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Hangle")
        {
            AppVirtualKeyboard::new(&mut app)
                .tap(VirtualKeyCode::X)
                .expect("physical X drops CLNK into the final blast pocket");
            advance_app_until(&mut app, "CLNK drops into final blast pocket", 60, |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.action.name == "Walk")
            });
        }
        app_collect_one_gold_around_blast_debris(&mut app, clonk);
        assert_eq!(app_object_contents_count(&app, clonk, "GOLD"), 1);

        app_carry_tutorial04_gold_to_hut(&mut app, clonk, elevator_case, hut, 15);
        app_return_tutorial04_from_hut_to_tunnel(&mut app, clonk, elevator_case, hut);
        app_collect_one_gold_around_blast_debris(&mut app, clonk);
        assert_eq!(app_object_contents_count(&app, clonk, "GOLD"), 1);

        // The first three trips sold value-five chunks. The fixed face-414
        // and face-402 outputs leave enough exact loose GOLD for two more
        // physical ELEC/HUT2 round trips; each trip preserves the exact GOLD
        // identity until BaseAutoSell removes it. Script251 may fulfill SCRG
        // only after wealth reaches 25 and then selects Tutorial05
        // (Tutorial04.c4s/Script.c:227-234; C4Object.cpp:970-997;
        // C4Player.cpp:866-897).
        for sold_chunks in 4..=5 {
            let target_wealth = sold_chunks * 5;
            app_carry_tutorial04_gold_to_hut(&mut app, clonk, elevator_case, hut, target_wealth);
            if sold_chunks < 5 {
                app_return_tutorial04_from_hut_to_tunnel(&mut app, clonk, elevator_case, hut);
                app_collect_one_gold_around_blast_debris(&mut app, clonk);
                assert_eq!(
                    app_object_contents_count(&app, clonk, "GOLD"),
                    1,
                    "each return trip must collect exactly one real GOLD"
                );
            }
        }
        advance_app_until(&mut app, "Tutorial04 selects Tutorial05", 640, |app| {
            app.engine.next_mission().path == r"Tutorial.c4f\Tutorial05.c4s"
        });
        advance_app_until(
            &mut app,
            "Tutorial04 fulfilled goal reaches GameOver",
            320,
            |app| app.engine.snapshot().game_over,
        );
        assert!(
            app.engine
                .snapshot()
                .round_results
                .fulfilled_goals
                .iter()
                .any(|goal| goal == "SCRG"),
            "Tutorial04 must fulfill its exact SCRG before selecting Tutorial05"
        );
        assert_eq!(
            app.engine.next_mission().path,
            r"Tutorial.c4f\Tutorial05.c4s"
        );
    }

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
        let constructor = app
            .engine
            .crew_cursor(app.local_owner)
            .expect("Tutorial05 starts on the constructor CLNK");
        let elevator =
            app_object_with_definition(&app, "ELEV").expect("Tutorial05 creates its partial ELEV");
        let valley_cata = app_object_with_definition_near_x(&app, "CATA", 240)
            .expect("Tutorial05 creates the valley CATA");
        let hill_cata = app_object_with_definition_near_x(&app, "CATA", 540)
            .expect("Tutorial05 creates the right-hill CATA");
        let wood = app_object_with_definition_near_x(&app, "WOOD", 280)
            .expect("Tutorial05 creates the valley WOOD");
        let metal = app_object_with_definition_near_x(&app, "METL", 285)
            .expect("Tutorial05 creates the valley METL");
        let home_base =
            app_object_with_definition(&app, "HUT3").expect("Tutorial05 creates its real HUT3");
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
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::E)
            .expect("physical E selects the valley CLNK");
        let valley = app
            .engine
            .crew_cursor(app.local_owner)
            .expect("Tutorial05 has a valley CLNK");
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
        let valley_x = app
            .engine
            .object_snapshot(valley)
            .expect("selected valley CLNK survives")
            .position
            .x;
        let wood_x = app
            .engine
            .object_snapshot(wood)
            .expect("valley WOOD survives")
            .position
            .x;
        let toward_wood = if wood_x < valley_x {
            VirtualKeyCode::Z
        } else {
            VirtualKeyCode::C
        };
        hold_app_key_until(
            &mut app,
            toward_wood,
            "the valley CLNK naturally collects the first material",
            160,
            |app| app_clonk_carries(app, valley, "WOOD") || app_clonk_carries(app, valley, "METL"),
        );
        if app_clonk_carries(&app, valley, "METL") {
            AppVirtualKeyboard::new(&mut app)
                .press(VirtualKeyCode::Z)
                .expect("physical Z faces away from valley WOOD");
            advance_app_until(&mut app, "valley CLNK faces left with METL", 30, |app| {
                app.engine
                    .object_snapshot(valley)
                    .is_some_and(|object| object.direction == Direction::Left)
            });
            {
                let mut keyboard = AppVirtualKeyboard::new(&mut app);
                keyboard
                    .release(VirtualKeyCode::Z)
                    .expect("release physical Z before METL throw");
                keyboard
                    .tap(VirtualKeyCode::A)
                    .expect("physical A sets METL aside for the second relay");
            }
            advance_app_until(&mut app, "METL leaves valley CLNK inventory", 30, |app| {
                app.engine
                    .object_snapshot(metal)
                    .is_some_and(|object| object.container.is_none())
            });
            let valley_x = app
                .engine
                .object_snapshot(valley)
                .expect("valley CLNK survives METL throw")
                .position
                .x;
            let wood_x = app
                .engine
                .object_snapshot(wood)
                .expect("valley WOOD survives METL throw")
                .position
                .x;
            let toward_wood = if wood_x < valley_x {
                VirtualKeyCode::Z
            } else {
                VirtualKeyCode::C
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
            VirtualKeyCode::Z,
            "the WOOD-carrying valley CLNK reaches CATA",
            160,
            |app| {
                app.engine
                    .object_snapshot(valley)
                    .zip(app.engine.object_snapshot(valley_cata))
                    .is_some_and(|(clonk, cata)| {
                        clonk.action.name == "Walk"
                            && (clonk.position.x - cata.position.x).abs() <= 8
                    })
            },
        );
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::X)
            .expect("physical X grabs the valley CATA");
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
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::A)
            .expect("physical A loads valley WOOD into CATA");
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

        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::X)
            .expect("hold physical X to tension CATA");
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::X)
            .expect("OS repeat while physical X remains held");
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
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::X)
            .expect("release physical X at full CATA tension");
        let tensioned = app
            .engine
            .object_snapshot(valley_cata)
            .expect("valley CATA survives tension release");
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

        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::A)
            .expect("physical A fires the fully tensioned CATA");
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
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::E)
            .expect("physical E selects the right-hill CLNK");
        let catapult_clonk = app
            .engine
            .crew_cursor(app.local_owner)
            .expect("Tutorial05 has a right-hill CLNK");
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
        let wood_x = app
            .engine
            .object_snapshot(wood)
            .expect("flung valley WOOD survives")
            .position
            .x;
        let catapult_clonk_x = app
            .engine
            .object_snapshot(catapult_clonk)
            .expect("right-hill CLNK survives")
            .position
            .x;
        let collect_key = if wood_x < catapult_clonk_x {
            VirtualKeyCode::Z
        } else {
            VirtualKeyCode::C
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
        let hill_cata_x = app
            .engine
            .object_snapshot(hill_cata)
            .expect("right-hill CATA survives collection")
            .position
            .x;
        let catapult_clonk_x = app
            .engine
            .object_snapshot(catapult_clonk)
            .expect("right-hill CLNK survives collection")
            .position
            .x;
        let reach_hill_cata = if hill_cata_x < catapult_clonk_x {
            VirtualKeyCode::Z
        } else {
            VirtualKeyCode::C
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
                        clonk.action.name == "Walk"
                            && (clonk.position.x - cata.position.x).abs() <= 12
                    })
            },
        );
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::X)
            .expect("physical X grabs the right-hill CATA");
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
                VirtualKeyCode::Z,
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
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::A)
            .expect("physical A loads the exact WOOD into the right-hill CATA");
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

        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::X)
            .expect("hold physical X to tension the right-hill CATA");
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
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::X)
            .expect("release physical X at full right-hill CATA tension");
        let tensioned = app
            .engine
            .object_snapshot(hill_cata)
            .expect("right-hill CATA survives tension release");
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

        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::A)
            .expect("physical A fires the fully tensioned right-hill CATA");
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
        let delivered = app
            .engine
            .object_snapshot(wood)
            .expect("twice-flung valley WOOD survives delivery");
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
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::E)
            .expect("physical E wraps selection to the constructor CLNK");
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
            let constructor_x = app
                .engine
                .object_snapshot(constructor)
                .expect("constructor CLNK survives selection")
                .position
                .x;
            let delivered_x = app
                .engine
                .object_snapshot(wood)
                .expect("delivered WOOD survives")
                .position
                .x;
            let collect_key = if delivered_x < constructor_x {
                VirtualKeyCode::Z
            } else {
                VirtualKeyCode::C
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
        let elevator_x = app
            .engine
            .object_snapshot(elevator)
            .expect("Tutorial05 elevator survives Script85")
            .position
            .x;
        let constructor_x = app
            .engine
            .object_snapshot(constructor)
            .expect("Tutorial05 constructor survives Script85")
            .position
            .x;
        if (constructor_x - elevator_x).abs() > 4 {
            let approach_key = if elevator_x < constructor_x {
                VirtualKeyCode::Z
            } else {
                VirtualKeyCode::C
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
            keyboard
                .tap(VirtualKeyCode::X)
                .expect("first physical X primes DownSingle");
            keyboard
                .tap(VirtualKeyCode::X)
                .expect("second physical X emits DownDouble at ELEV");
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
        let first_delivery = app
            .engine
            .object_snapshot(elevator)
            .expect("Tutorial05 ELEV survives its first delivered component");
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
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::E)
            .expect("physical E selects the valley CLNK for the second relay");
        assert_eq!(app.engine.crew_cursor(app.local_owner), Some(valley));
        if app
            .engine
            .object_snapshot(valley)
            .is_some_and(|object| object.action.name == "Push")
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .tap(VirtualKeyCode::X)
                .expect("first physical X primes valley DownSingle");
            keyboard
                .tap(VirtualKeyCode::X)
                .expect("second physical X ungrabs the valley CATA");
        }
        advance_app_until(&mut app, "valley CLNK releases its CATA", 80, |app| {
            app.engine
                .object_snapshot(valley)
                .is_some_and(|object| object.action.name == "Walk")
        });
        let valley_x = app
            .engine
            .object_snapshot(valley)
            .expect("valley CLNK survives first relay")
            .position
            .x;
        let metal_x = app
            .engine
            .object_snapshot(metal)
            .expect("untouched Tutorial05 METL survives first relay")
            .position
            .x;
        let collect_metal_key = if metal_x < valley_x {
            VirtualKeyCode::Z
        } else {
            VirtualKeyCode::C
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

        let valley_x = app
            .engine
            .object_snapshot(valley)
            .expect("METL-carrying valley CLNK survives")
            .position
            .x;
        let valley_cata_x = app
            .engine
            .object_snapshot(valley_cata)
            .expect("valley CATA survives first shot")
            .position
            .x;
        let return_to_valley_cata = if valley_cata_x < valley_x {
            VirtualKeyCode::Z
        } else {
            VirtualKeyCode::C
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
                        clonk.action.name == "Walk"
                            && (clonk.position.x - cata.position.x).abs() <= 8
                    })
            },
        );
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::X)
            .expect("physical X regrabs the valley CATA");
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
                VirtualKeyCode::C,
                "valley CATA faces the right hill for its second shot",
                40,
                |app| {
                    app.engine
                        .object_snapshot(valley_cata)
                        .is_some_and(|object| object.direction == Direction::Right)
                },
            );
        }
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::A)
            .expect("physical A loads the original METL into valley CATA");
        advance_app_until(&mut app, "original METL enters valley CATA", 80, |app| {
            app.engine
                .object_snapshot(metal)
                .is_some_and(|object| object.container == Some(valley_cata))
        });
        assert_eq!(app_object_contents_count(&app, valley_cata, "METL"), 1);
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::X)
            .expect("hold physical X for the valley CATA's second shot");
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
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::X)
            .expect("release physical X at full valley METL tension");
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::A)
            .expect("physical A fires METL toward the right hill");
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
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::E)
            .expect("physical E selects the right-hill CLNK for METL");
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
            keyboard
                .tap(VirtualKeyCode::X)
                .expect("first physical X primes hill DownSingle");
            keyboard
                .tap(VirtualKeyCode::X)
                .expect("second physical X ungrabs the hill CATA");
        }
        advance_app_until(&mut app, "right-hill CLNK releases its CATA", 80, |app| {
            app.engine
                .object_snapshot(catapult_clonk)
                .is_some_and(|object| object.action.name == "Walk")
        });
        let hill_clonk_x = app
            .engine
            .object_snapshot(catapult_clonk)
            .expect("right-hill CLNK survives first relay")
            .position
            .x;
        let landed_metal_x = app
            .engine
            .object_snapshot(metal)
            .expect("right-hill METL survives landing")
            .position
            .x;
        let collect_hill_metal = if landed_metal_x < hill_clonk_x {
            VirtualKeyCode::Z
        } else {
            VirtualKeyCode::C
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

        let hill_clonk_x = app
            .engine
            .object_snapshot(catapult_clonk)
            .expect("METL-carrying hill CLNK survives")
            .position
            .x;
        let hill_cata_x = app
            .engine
            .object_snapshot(hill_cata)
            .expect("hill CATA survives first shot")
            .position
            .x;
        let return_to_hill_cata = if hill_cata_x < hill_clonk_x {
            VirtualKeyCode::Z
        } else {
            VirtualKeyCode::C
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
                        clonk.action.name == "Walk"
                            && (clonk.position.x - cata.position.x).abs() <= 8
                    })
            },
        );
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::X)
            .expect("physical X regrabs the hill CATA");
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
                VirtualKeyCode::Z,
                "hill CATA faces the cabin for its METL shot",
                40,
                |app| {
                    app.engine
                        .object_snapshot(hill_cata)
                        .is_some_and(|object| object.direction == Direction::Left)
                },
            );
        }
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::A)
            .expect("physical A loads the original METL into hill CATA");
        advance_app_until(&mut app, "original METL enters hill CATA", 80, |app| {
            app.engine
                .object_snapshot(metal)
                .is_some_and(|object| object.container == Some(hill_cata))
        });
        assert_eq!(app_object_contents_count(&app, hill_cata, "METL"), 1);
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::X)
            .expect("hold physical X for the hill CATA's METL shot");
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
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::X)
            .expect("release physical X at full hill METL tension");
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::A)
            .expect("physical A fires METL toward the cabin hill");
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
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::E)
            .expect("physical E wraps selection to constructor for METL");
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
            let constructor_x = app
                .engine
                .object_snapshot(constructor)
                .expect("constructor survives METL relay")
                .position
                .x;
            let metal_x = app
                .engine
                .object_snapshot(metal)
                .expect("cabin-hill METL survives landing")
                .position
                .x;
            let collect_cabin_metal = if metal_x < constructor_x {
                VirtualKeyCode::Z
            } else {
                VirtualKeyCode::C
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
        let constructor_x = app
            .engine
            .object_snapshot(constructor)
            .expect("METL-carrying constructor survives")
            .position
            .x;
        let elevator_x = app
            .engine
            .object_snapshot(elevator)
            .expect("partial ELEV survives second relay")
            .position
            .x;
        if (constructor_x - elevator_x).abs() > 4 {
            let return_to_elevator = if elevator_x < constructor_x {
                VirtualKeyCode::Z
            } else {
                VirtualKeyCode::C
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
            keyboard
                .tap(VirtualKeyCode::X)
                .expect("first physical X primes final DownSingle");
            keyboard
                .tap(VirtualKeyCode::X)
                .expect("second physical X builds ELEV with METL");
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
        let completed = app
            .engine
            .object_snapshot(elevator)
            .expect("completed Tutorial05 ELEV survives");
        assert_eq!(completed.components.get("WOOD"), Some(&4));
        assert_eq!(completed.components.get("METL"), Some(&2));
        assert_eq!(completed.construction, 100_000);
        let carriage = app_object_with_definition(&app, "ELEC")
            .expect("completed ELEV creates its real ELEC carriage");
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
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::X)
            .expect("physical X grabs Tutorial05's real ELEC");
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
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::D)
            .expect("hold physical D to start ELEC shaft drilling");
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
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::D)
            .expect("release physical D after Tutorial05 accepts Drill");
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
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::E)
            .expect("first physical E advances toward the right-hill CLNK");
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::E)
            .expect("second physical E selects the right-hill CLNK");
        assert_eq!(
            app.engine.crew_cursor(app.local_owner),
            Some(catapult_clonk)
        );
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::Z,
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
            VirtualKeyCode::Z,
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
            keyboard
                .press(VirtualKeyCode::Z)
                .expect("hold physical Z for the right-hill shaft-lip jump");
            keyboard
                .tap(VirtualKeyCode::S)
                .expect("physical S jumps the right-hill CLNK across the shaft lip");
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
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::Z)
            .expect("release physical Z after the right-hill lip jump");
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
                VirtualKeyCode::X,
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

        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::E)
            .expect("first physical E advances toward the valley CLNK");
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::E)
            .expect("second physical E selects the valley CLNK");
        assert_eq!(app.engine.crew_cursor(app.local_owner), Some(valley));
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::Z,
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
            keyboard
                .press(VirtualKeyCode::Z)
                .expect("hold physical Z for the valley shaft-lip jump");
            keyboard
                .tap(VirtualKeyCode::S)
                .expect("physical S jumps the valley CLNK across the shaft lip");
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
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::Z)
            .expect("release physical Z after the valley lip jump");
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
                VirtualKeyCode::X,
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

        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::X)
            .expect("physical X makes the valley CLNK grab ELEC");
        if app
            .engine
            .object_snapshot(valley)
            .is_some_and(|object| object.action.name != "Push")
        {
            AppVirtualKeyboard::new(&mut app)
                .tap(VirtualKeyCode::X)
                .expect("second physical X completes valley DownDouble");
        }
        advance_app_until(&mut app, "valley CLNK grabs ELEC for ascent", 120, |app| {
            app.engine.object_snapshot(valley).is_some_and(|object| {
                object.action.name == "Push" && object.action.target == Some(carriage)
            })
        });
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::E)
            .expect("physical E selects the boarded right-hill CLNK");
        assert_eq!(
            app.engine.crew_cursor(app.local_owner),
            Some(catapult_clonk)
        );
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::C,
            "right-hill CLNK centers on ELEC before grabbing",
            40,
            |app| {
                app.engine
                    .object_snapshot(catapult_clonk)
                    .is_some_and(|object| object.position.x >= 161 && object.action.name == "Walk")
            },
        );
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::X)
            .expect("physical X makes the right-hill CLNK grab ELEC");
        if app
            .engine
            .object_snapshot(catapult_clonk)
            .is_some_and(|object| object.action.name != "Push")
        {
            AppVirtualKeyboard::new(&mut app)
                .tap(VirtualKeyCode::X)
                .expect("second physical X completes right-hill DownDouble");
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
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::E)
            .expect("physical E returns to the constructor");
        assert_eq!(app.engine.crew_cursor(app.local_owner), Some(constructor));
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::X)
            .expect("physical X makes the constructor grab ELEC");
        if app
            .engine
            .object_snapshot(constructor)
            .is_some_and(|object| object.action.name != "Push")
        {
            AppVirtualKeyboard::new(&mut app)
                .tap(VirtualKeyCode::X)
                .expect("second physical X completes constructor DownDouble");
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
            keyboard
                .tap(VirtualKeyCode::W)
                .expect("first physical W primes CursorToggleSingle");
            keyboard
                .tap(VirtualKeyCode::W)
                .expect("second physical W selects all Tutorial05 Clonks");
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
            VirtualKeyCode::S,
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
                keyboard
                    .tap(VirtualKeyCode::X)
                    .unwrap_or_else(|error| panic!("first X releasing {caption}: {error}"));
                keyboard
                    .tap(VirtualKeyCode::X)
                    .unwrap_or_else(|error| panic!("second X releasing {caption}: {error}"));
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
                AppVirtualKeyboard::new(&mut app)
                    .tap(VirtualKeyCode::E)
                    .unwrap_or_else(|error| panic!("select next Clonk after {caption}: {error}"));
            }
        }

        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .press(VirtualKeyCode::Z)
                .expect("hold Z for right-hill CLNK's top-lip jump");
            keyboard
                .tap(VirtualKeyCode::S)
                .expect("physical S jumps the right-hill CLNK over the top lip");
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
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::Z)
            .expect("release Z after right-hill top-lip jump");
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
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::E)
            .expect("physical E returns to constructor at shaft top");
        assert_eq!(app.engine.crew_cursor(app.local_owner), Some(constructor));
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .press(VirtualKeyCode::Z)
                .expect("hold Z for constructor's top-lip jump");
            keyboard
                .tap(VirtualKeyCode::S)
                .expect("physical S jumps constructor over the top lip");
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
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::Z)
            .expect("release Z after constructor top-lip jump");
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
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::E)
            .expect("physical E selects valley CLNK for the top lip");
        assert_eq!(app.engine.crew_cursor(app.local_owner), Some(valley));
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::C,
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
            keyboard
                .press(VirtualKeyCode::Z)
                .expect("hold Z for valley CLNK's top-lip jump");
            keyboard
                .tap(VirtualKeyCode::S)
                .expect("physical S jumps valley CLNK over the top lip");
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
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::Z)
            .expect("release Z after valley top-lip jump");
        advance_app_until(
            &mut app,
            "valley CLNK lands on the cabin plateau",
            160,
            |app| {
                app.engine.object_snapshot(valley).is_some_and(|object| {
                    object.action.name == "Walk"
                        && object.position.x < 155
                        && object.position.y <= 115
                })
            },
        );
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .tap(VirtualKeyCode::W)
                .expect("first plateau W primes CursorToggleSingle");
            keyboard
                .tap(VirtualKeyCode::W)
                .expect("second plateau W reselects all Clonks");
        }
        assert_eq!(app.engine.crew_cursor(app.local_owner), Some(constructor));
        assert!(
            [constructor, valley, catapult_clonk]
                .into_iter()
                .all(|clonk| app.engine.selected_crew(app.local_owner).contains(&clonk)),
            "all exact Tutorial05 Clonks must be reselected on the plateau"
        );

        let hut_x = app
            .engine
            .object_snapshot(home_base)
            .expect("Tutorial05 HUT3 survives")
            .position
            .x;
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::Z,
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
            VirtualKeyCode::X,
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
            VirtualKeyCode::C,
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
        let hut_position = app
            .engine
            .object_snapshot(home_base)
            .expect("Tutorial05 HUT3 survives its approaching crew")
            .position;
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
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::E)
            .expect("physical E selects valley CLNK at HUT3");
        assert_eq!(app.engine.crew_cursor(app.local_owner), Some(valley));
        if app
            .engine
            .object_snapshot(valley)
            .is_some_and(|object| object.position.x > hut_position.x + 18)
        {
            hold_app_key_until(
                &mut app,
                VirtualKeyCode::Z,
                "valley CLNK steps fully inside HUT3's entrance",
                20,
                |app| {
                    app.engine
                        .object_snapshot(valley)
                        .is_some_and(|object| object.position.x <= hut_position.x + 18)
                },
            );
        }
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::E)
            .expect("physical E selects right-hill CLNK at HUT3");
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
                VirtualKeyCode::Z,
                "right-hill CLNK steps fully inside HUT3's entrance",
                20,
                |app| {
                    app.engine
                        .object_snapshot(catapult_clonk)
                        .is_some_and(|object| object.position.x <= hut_position.x + 18)
                },
            );
        }
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::E)
            .expect("physical E returns to constructor at HUT3");
        assert_eq!(app.engine.crew_cursor(app.local_owner), Some(constructor));
        if app
            .engine
            .object_snapshot(constructor)
            .is_some_and(|object| object.position.x > hut_position.x + 18)
        {
            hold_app_key_until(
                &mut app,
                VirtualKeyCode::Z,
                "constructor steps fully inside HUT3's entrance",
                20,
                |app| {
                    app.engine
                        .object_snapshot(constructor)
                        .is_some_and(|object| object.position.x <= hut_position.x + 18)
                },
            );
        }
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::S)
            .expect("physical S queues constructor Enter HUT3");
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::E)
            .expect("physical E returns to valley CLNK for queued HUT3 entry");
        assert_eq!(app.engine.crew_cursor(app.local_owner), Some(valley));
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::S)
            .expect("physical S queues valley CLNK Enter HUT3");
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::E)
            .expect("physical E returns to right-hill CLNK for queued HUT3 entry");
        assert_eq!(
            app.engine.crew_cursor(app.local_owner),
            Some(catapult_clonk)
        );
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::S)
            .expect("physical S queues right-hill CLNK Enter HUT3");
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

    #[test]
    #[ignore = "over-constrained virtual-play driver; not a production parity oracle"]
    fn app_virtual_keyboard_completes_tutorial06_and_selects_tutorial07() {
        // Tutorial06 creates the real CRYS, waits for it to become contained,
        // then launches FXQ1 and calls ShakeFree(60,160,50) before displaying
        // the instability warning
        // (content/Tutorial.c4f/Tutorial06.c4s/Script.c:8-37).
        // Keep collection behind GameApp::handle_key: C++ collects when the
        // carryable crosses the crew member's collection rectangle, and
        // ShakeFree clears every DigFree pixel in its circle
        // (src/C4GameObjects.cpp:155-196; src/C4Landscape.cpp:928-938,999-1009).
        // The rest of this route keeps every construction, drainage, CRYS
        // transfer, and sale behind those same physical app-key boundaries.
        let mut app = real_tutorial_app(6, "Tutorial 6 app virtual player");
        let first_clonk = app
            .engine
            .crew_cursor(app.local_owner)
            .expect("Tutorial06 starts with a crew cursor");
        let crystal = app_object_with_definition(&app, "CRYS")
            .expect("Tutorial06 creates its real collectible CRYS");
        assert_eq!(
            app.engine
                .object_snapshot(crystal)
                .expect("Tutorial06 CRYS survives initialization")
                .container,
            None
        );
        assert!(
            app.engine.debug_landscape_is_solid(60, 150),
            "Tutorial06 pit probe must start as solid Earth"
        );

        advance_app_until(
            &mut app,
            "Tutorial06 asks the first CLNK to collect CRYS",
            400,
            |app| app_tutorial_message_contains(app, "collect the crystal"),
        );
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::Z,
            "the first CLNK naturally collects the exact Tutorial06 CRYS",
            800,
            |app| {
                app.engine
                    .object_snapshot(crystal)
                    .is_some_and(|object| object.container == Some(first_clonk))
            },
        );
        assert_eq!(
            app.engine
                .object_snapshot(crystal)
                .expect("collected Tutorial06 CRYS survives")
                .container,
            Some(first_clonk),
            "physical Z must collect Tutorial06's original CRYS"
        );

        advance_app_until(
            &mut app,
            "Tutorial06 ShakeFree opens its scripted pit",
            120,
            |app| !app.engine.debug_landscape_is_solid(60, 150),
        );
        assert!(
            !app.engine.debug_landscape_is_solid(60, 150),
            "ShakeFree(60,160,50) must clear the (60,150) Earth pixel"
        );
        assert!(
            app_object_with_definition(&app, "FXQ1").is_some(),
            "Tutorial06 must launch its real earthquake object"
        );
        advance_app_until(
            &mut app,
            "Tutorial06 advances to its instability warning",
            300,
            |app| app_tutorial_message_contains(app, "area seems to be unstable"),
        );

        hold_app_key_until(
            &mut app,
            VirtualKeyCode::C,
            "the CRYS-carrying CLNK reaches the trapped cavern",
            800,
            |app| {
                app.engine
                    .object_snapshot(first_clonk)
                    .is_some_and(|object| object.position.x >= 160 && object.position.y >= 350)
            },
        );
        advance_app_until(
            &mut app,
            "Tutorial06 asks the other CLNK to build ELEV",
            2_400,
            |app| app_tutorial_message_contains(app, "With the other clonk"),
        );
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::E)
            .expect("physical E selects Tutorial06's surface CLNK");
        let builder = app
            .engine
            .crew_cursor(app.local_owner)
            .expect("CursorRight selects Tutorial06's surface CLNK");
        assert_ne!(
            builder, first_clonk,
            "physical CursorRight must leave the CRYS-carrying CLNK"
        );
        let hut = app_object_with_definition(&app, "HUT3")
            .expect("Tutorial06 creates the player's exact HUT3");
        let conkit = app
            .engine
            .snapshot()
            .objects
            .into_iter()
            .find(|object| object.definition_id == "CNKT" && object.container == Some(hut))
            .expect("Tutorial06 puts one exact CNKT in HUT3")
            .id;
        let context_identification = serde_json::from_value(serde_json::json!({ "Int": 14 }))
            .expect("context identification deserializes");
        let contents_identification = serde_json::from_value(serde_json::json!({ "Int": 18 }))
            .expect("contents identification deserializes");
        let construction_identification =
            serde_json::from_value(serde_json::json!({ "C4Id": "CXCN" }))
                .expect("construction identification deserializes");

        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::S)
            .expect("physical S enters Tutorial06 HUT3");
        advance_app_until(&mut app, "surface CLNK enters exact HUT3", 80, |app| {
            app.engine
                .object_snapshot(builder)
                .is_some_and(|object| object.container == Some(hut))
        });
        advance_app_until(&mut app, "HUT3 opens its context menu", 30, |app| {
            app.engine
                .cursor_object_menu(app.local_owner)
                .is_some_and(|(_, menu)| menu.identification == context_identification)
        });
        app_navigate_object_menu_to(&mut app, "Contents", |item| item.caption == "Contents");
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::A)
            .expect("physical A opens HUT3 Contents");
        advance_app_until(&mut app, "HUT3 opens its real Contents menu", 30, |app| {
            app.engine
                .cursor_object_menu(app.local_owner)
                .is_some_and(|(_, menu)| menu.identification == contents_identification)
        });
        app_navigate_object_menu_to(&mut app, "CNKT", |item| item.item_id == "CNKT");
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::A)
            .expect("physical A takes Tutorial06's exact CNKT");
        advance_app_until(&mut app, "surface CLNK takes exact CNKT", 80, |app| {
            app.engine
                .object_snapshot(conkit)
                .is_some_and(|object| object.container == Some(builder))
        });
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::D)
            .expect("physical D closes HUT3 Contents");
        advance_app_until(&mut app, "HUT3 restores its context menu", 30, |app| {
            app.engine
                .cursor_object_menu(app.local_owner)
                .is_some_and(|(_, menu)| menu.identification == context_identification)
        });
        app_navigate_object_menu_to(&mut app, "Exit", |item| item.caption == "Exit");
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::A)
            .expect("physical A exits HUT3 with CNKT");
        advance_app_until(&mut app, "CNKT-carrying CLNK exits exact HUT3", 80, |app| {
            app.engine
                .object_snapshot(builder)
                .is_some_and(|object| object.container.is_none() && object.action.name == "Walk")
        });
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::C,
            "builder reaches the ELEV construction site",
            100,
            |app| {
                app.engine
                    .object_snapshot(builder)
                    .is_some_and(|object| (329..=333).contains(&object.position.x))
            },
        );
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .tap(VirtualKeyCode::D)
                .expect("first physical D primes CNKT activation");
            keyboard
                .tap(VirtualKeyCode::D)
                .expect("second physical D activates CNKT");
        }
        advance_app_until(
            &mut app,
            "CNKT opens its ELEV construction menu",
            30,
            |app| {
                app.engine
                    .cursor_object_menu(app.local_owner)
                    .is_some_and(|(_, menu)| menu.identification == construction_identification)
            },
        );
        app_navigate_object_menu_to(&mut app, "ELEV", |item| item.item_id == "ELEV");
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::A)
            .expect("physical A creates Tutorial06 ELEV");
        advance_app_until(&mut app, "exact Tutorial06 ELEV is created", 30, |app| {
            app_object_with_definition(app, "ELEV").is_some()
        });
        let elevator = app_object_with_definition(&app, "ELEV")
            .expect("preserve Tutorial06's exact ELEV identity");
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .tap(VirtualKeyCode::X)
                .expect("first physical X leaves ELEV construction");
            keyboard
                .tap(VirtualKeyCode::X)
                .expect("second physical X leaves ELEV construction");
        }
        advance_app_until(
            &mut app,
            "ELEV finishes and creates its exact ELEC",
            3_000,
            |app| {
                app_object_with_definition(app, "ELEC").is_some()
                    && app
                        .engine
                        .object_snapshot(elevator)
                        .is_some_and(|object| object.construction == 100_000)
            },
        );
        advance_app_until(
            &mut app,
            "automatic Energy command connects ELEV to POWR",
            1_200,
            |app| app_object_with_definition(app, "PWRL").is_some(),
        );

        advance_app_until(
            &mut app,
            "Tutorial06 points at the surface coal",
            300,
            |app| app_tutorial_message_contains(app, "dig out a few pieces of coal"),
        );
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::C,
            "surface CLNK reaches the coal seam",
            300,
            |app| {
                app.engine
                    .object_snapshot(builder)
                    .is_some_and(|object| object.position.x >= 440)
            },
        );
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::Z,
            "surface CLNK steps off the coal wall",
            40,
            |app| {
                app.engine
                    .object_snapshot(builder)
                    .is_some_and(|object| object.action.name == "Walk" && object.position.x <= 420)
            },
        );
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .press(VirtualKeyCode::C)
                .expect("physical C holds the surface CLNK toward coal");
            keyboard
                .tap(VirtualKeyCode::D)
                .expect("physical D starts digging coal");
        }
        advance_app_until(&mut app, "surface CLNK starts digging coal", 30, |app| {
            app.engine
                .object_snapshot(builder)
                .is_some_and(|object| object.action.name == "Dig")
        });
        for _ in 0..300 {
            if app
                .engine
                .snapshot()
                .objects
                .iter()
                .filter(|object| object.definition_id == "COAL")
                .count()
                >= 3
            {
                break;
            }
            app.update().expect("advance Tutorial06 coal Dig");
        }
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::C)
            .expect("release physical C after Tutorial06 coal Dig");
        let coal = app
            .engine
            .snapshot()
            .objects
            .into_iter()
            .filter(|object| object.definition_id == "COAL")
            .map(|object| (object.id, object.position, object.container))
            .collect::<Vec<_>>();
        assert!(
            coal.len() >= 3,
            "real coal seam must yield three chunks; builder={:?}, coal={coal:?}",
            app.engine.object_snapshot(builder)
        );
        advance_app_until(&mut app, "Tutorial06 asks for coal in POWR", 300, |app| {
            app_tutorial_message_contains(app, "Throw the coal chunks")
        });
        advance_app_until(&mut app, "coal miner returns to Walk", 80, |app| {
            app.engine
                .object_snapshot(builder)
                .is_some_and(|object| object.action.name == "Walk")
        });
        let power_plant = app_object_with_definition(&app, "POWR")
            .expect("preserve Tutorial06's exact POWR identity");
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::Z,
            "COAL-carrying CLNK reaches POWR's chute",
            160,
            |app| {
                app.engine
                    .object_snapshot(builder)
                    .is_some_and(|object| object.position.x <= 424)
            },
        );
        let carried_coal = app
            .engine
            .object_snapshot(builder)
            .expect("Tutorial06 builder survives at POWR's chute")
            .contents
            .into_iter()
            .next()
            .filter(|object_id| {
                app.engine
                    .object_snapshot(*object_id)
                    .is_some_and(|object| object.definition_id == "COAL")
            })
            .expect("the builder's exact first content is real COAL at the chute");
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::A)
            .expect("physical A throws the exact COAL into POWR");
        advance_app_until(
            &mut app,
            "exact thrown COAL leaves the builder's inventory",
            60,
            |app| {
                app.engine
                    .object_snapshot(carried_coal)
                    .is_none_or(|coal| coal.container != Some(builder))
            },
        );
        advance_app_until(
            &mut app,
            "exact POWR starts burning the thrown COAL",
            180,
            |app| {
                app.engine
                    .object_snapshot(power_plant)
                    .is_some_and(|object| object.action.name == "Burning")
            },
        );
        advance_app_until(
            &mut app,
            "Tutorial06 asks the builder to drill the elevator shaft",
            300,
            |app| app_tutorial_message_contains(app, "drill an elevator shaft"),
        );
        let elevator_case = app_object_with_definition(&app, "ELEC")
            .expect("preserve Tutorial06's exact ELEC identity");
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::Z,
            "builder returns to the elevator shaft",
            180,
            |app| {
                app.engine
                    .object_snapshot(builder)
                    .is_some_and(|object| object.position.x <= 340)
            },
        );
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::X)
            .expect("physical X calls ELEC to the waiting builder");
        for _ in 0..12 {
            app.update()
                .expect("wait out ELEC call double-click window");
        }
        advance_app_until(&mut app, "ELEC returns to the builder", 600, |app| {
            app.engine
                .object_snapshot(builder)
                .zip(app.engine.object_snapshot(elevator_case))
                .is_some_and(|(builder, elevator_case)| {
                    elevator_case.action.name == "Wait"
                        && (builder.position.y - elevator_case.position.y).abs() <= 24
                })
        });
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::S)
            .expect("physical S jumps toward ELEC");
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::Z,
            "builder jumps onto ELEC's center",
            80,
            |app| {
                app.engine
                    .object_snapshot(builder)
                    .is_some_and(|object| object.position.x <= 331)
            },
        );
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::X)
            .expect("physical X centers the builder on ELEC");
        advance_app_until(&mut app, "builder lands centered on ELEC", 80, |app| {
            app.engine.object_snapshot(builder).is_some_and(|object| {
                object.action.name == "Walk" && (327..=333).contains(&object.position.x)
            })
        });
        for _ in 0..12 {
            app.update()
                .expect("wait out ELEC grab double-click window");
        }
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::X)
            .expect("physical X grabs ELEC under AutoStopControl");
        advance_app_until(&mut app, "builder grabs exact ELEC", 100, |app| {
            app.engine.object_snapshot(builder).is_some_and(|object| {
                object.action.name == "Push" && object.action.target == Some(elevator_case)
            })
        });

        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::D)
            .expect("held physical D starts ELEC drilling");
        advance_app_until(&mut app, "ELEC starts drilling the real shaft", 80, |app| {
            app.engine
                .object_snapshot(elevator_case)
                .is_some_and(|object| object.action.name == "Drill")
        });
        app.update().expect("advance one exact ELEC drill frame");
        assert!(
            app.engine.object_snapshot(builder).is_some_and(|object| {
                object.action.name == "Push" && object.action.target == Some(elevator_case)
            }),
            "the exact builder must stay attached to drilling ELEC"
        );
        advance_app_until(
            &mut app,
            "ELEC carries builder to Tutorial06's lower cavern",
            1_200,
            |app| {
                app.engine
                    .object_snapshot(builder)
                    .is_some_and(|object| object.position.y >= 300)
            },
        );
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::D)
            .expect("release physical D at Tutorial06's lower cavern");
        advance_app_until(
            &mut app,
            "Tutorial06 introduces the flooded passage",
            600,
            |app| app_tutorial_message_contains(app, "get the water out of the way"),
        );
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::D)
            .expect("held physical D resumes ELEC drilling");
        advance_app_until(
            &mut app,
            "ELEC resumes drilling below the flooded shelf",
            80,
            |app| {
                app.engine
                    .object_snapshot(elevator_case)
                    .is_some_and(|object| object.action.name == "Drill")
            },
        );
        advance_app_until(
            &mut app,
            "ELEC drills to the drainage-tunnel level",
            600,
            |app| {
                app.engine
                    .object_snapshot(elevator_case)
                    .is_some_and(|object| object.position.y >= 325)
            },
        );
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::D)
            .expect("release physical D at drainage level");
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .tap(VirtualKeyCode::X)
                .expect("first physical X primes lower ELEC release");
            keyboard
                .tap(VirtualKeyCode::X)
                .expect("second physical X releases lower ELEC");
        }
        advance_app_until(
            &mut app,
            "builder releases exact ELEC in lower cavern",
            80,
            |app| {
                app.engine
                    .object_snapshot(builder)
                    .is_some_and(|object| object.action.name == "Walk")
            },
        );

        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::D)
            .expect("physical D starts the dry basin approach");
        advance_app_until(
            &mut app,
            "builder starts passage above the water shelf",
            40,
            |app| {
                app.engine
                    .object_snapshot(builder)
                    .is_some_and(|object| object.action.name == "Dig")
            },
        );
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .press(VirtualKeyCode::Z)
                .expect("physical Z aims the dry approach left");
            keyboard
                .press(VirtualKeyCode::X)
                .expect("physical X aims the dry approach down-left");
        }
        advance_app_until(&mut app, "dry approach reaches basin wall", 100, |app| {
            app.engine
                .object_snapshot(builder)
                .is_some_and(|object| object.position.x <= 320)
        });
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .release(VirtualKeyCode::X)
                .expect("release physical X at dry basin wall");
            keyboard
                .release(VirtualKeyCode::Z)
                .expect("release physical Z at dry basin wall");
        }
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::X)
            .expect("physical X stops builder at dry basin wall");
        advance_app_until(&mut app, "builder stops at dry basin wall", 40, |app| {
            app.engine
                .object_snapshot(builder)
                .is_some_and(|object| object.action.name == "Walk")
        });
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::D)
            .expect("physical D starts the diagonal upper passage");
        advance_app_until(
            &mut app,
            "builder starts diagonal basin passage",
            40,
            |app| {
                app.engine
                    .object_snapshot(builder)
                    .is_some_and(|object| object.action.name == "Dig")
            },
        );
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .press(VirtualKeyCode::Z)
                .expect("physical Z aims the upper passage left");
            keyboard
                .press(VirtualKeyCode::S)
                .expect("physical S aims the upper passage up-left");
        }
        advance_app_until(
            &mut app,
            "builder pre-clears the dry upper passage",
            120,
            |app| {
                app.engine
                    .object_snapshot(builder)
                    .is_some_and(|object| object.action.name == "Dig" && object.position.x <= 290)
            },
        );
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .release(VirtualKeyCode::S)
                .expect("release physical S after upper-passage Dig");
            keyboard
                .release(VirtualKeyCode::Z)
                .expect("release physical Z after upper-passage Dig");
        }
        advance_app_until(
            &mut app,
            "builder stops before breaching upper passage",
            40,
            |app| {
                app.engine
                    .object_snapshot(builder)
                    .is_some_and(|object| object.action.name == "Walk")
            },
        );
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::C,
            "builder returns from the dry upper passage",
            140,
            |app| {
                app.engine
                    .object_snapshot(builder)
                    .is_some_and(|object| object.position.x >= 327)
            },
        );
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::Z,
            "builder aligns with the lower basin wall",
            100,
            |app| {
                app.engine
                    .object_snapshot(builder)
                    .is_some_and(|object| object.action.name == "Walk" && object.position.x <= 330)
            },
        );
        advance_app_until(
            &mut app,
            "lower-shelf builder comes to a full stop",
            40,
            |app| {
                app.engine
                    .object_snapshot(builder)
                    .is_some_and(|object| object.velocity.x == 0)
            },
        );
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::D)
            .expect("physical D starts lower basin drain");
        advance_app_until(&mut app, "builder starts lower basin drain", 40, |app| {
            app.engine
                .object_snapshot(builder)
                .is_some_and(|object| object.action.name == "Dig")
        });
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .press(VirtualKeyCode::Z)
                .expect("physical Z aims lower drain left");
            keyboard
                .press(VirtualKeyCode::X)
                .expect("physical X aims lower drain down-left");
        }
        advance_app_until(&mut app, "lower drain reaches dry basin wall", 100, |app| {
            app.engine
                .object_snapshot(builder)
                .is_some_and(|object| object.action.name == "Dig" && object.position.x <= 305)
        });
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .release(VirtualKeyCode::X)
                .expect("release physical X before opening lower drain");
            keyboard
                .release(VirtualKeyCode::Z)
                .expect("release physical Z before opening lower drain");
        }
        advance_app_until(
            &mut app,
            "builder stops before opening lower drain",
            40,
            |app| {
                app.engine
                    .object_snapshot(builder)
                    .is_some_and(|object| object.action.name == "Walk")
            },
        );
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::D)
            .expect("physical D resumes lower basin drain");
        advance_app_until(&mut app, "builder resumes lower basin drain", 40, |app| {
            app.engine
                .object_snapshot(builder)
                .is_some_and(|object| object.action.name == "Dig")
        });
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::Z)
            .expect("held physical Z aims lower drain left");
        advance_app_until(
            &mut app,
            "lower drain reaches its diagonal turn",
            80,
            |app| {
                app.engine
                    .object_snapshot(builder)
                    .is_some_and(|object| object.action.name == "Dig" && object.position.x <= 298)
            },
        );
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::S)
            .expect("held physical S turns lower drain up-left");
        assert_eq!(
            app.engine
                .object_snapshot(builder)
                .expect("builder survives lower-drain steering")
                .command_direction,
            CommandDirection::UpLeft,
            "held physical Left+Up must turn the lower drain up-left"
        );
        advance_app_until(
            &mut app,
            "diagonal passage opens the basin drain",
            240,
            |app| {
                app.engine
                    .object_snapshot(builder)
                    .is_some_and(|object| object.action.name == "Swim")
            },
        );
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .release(VirtualKeyCode::Z)
                .expect("release physical Z after entering basin water");
            keyboard
                .release(VirtualKeyCode::S)
                .expect("release physical S after entering basin water");
        }
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::S,
            "rescuer swims straight up through the pre-cleared passage",
            160,
            |app| {
                app.engine
                    .object_snapshot(builder)
                    .is_some_and(|object| object.position.y <= 317)
            },
        );
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::C,
            "rescuer swims right of the basin drain",
            120,
            |app| {
                app.engine
                    .object_snapshot(builder)
                    .is_some_and(|object| object.position.x >= 310)
            },
        );
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::C,
            "rescuer traverses the dry east passage",
            240,
            |app| {
                app.engine
                    .object_snapshot(builder)
                    .is_some_and(|object| object.position.x >= 380)
            },
        );
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::Z,
            "rescuer centers below the east cavern handhold",
            40,
            |app| {
                app.engine
                    .object_snapshot(builder)
                    .is_some_and(|object| object.position.x <= 374)
            },
        );
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::S,
            "rescuer rises toward the east cavern wall",
            160,
            |app| {
                app.engine
                    .object_snapshot(builder)
                    .is_some_and(|object| object.position.y <= 319)
            },
        );
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::C,
            "rescuer reaches the east cavern Scale",
            160,
            |app| {
                app.engine
                    .object_snapshot(builder)
                    .is_some_and(|object| object.action.name.starts_with("Scale"))
            },
        );
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::S,
            "rescuer climbs into the dry east cavern",
            160,
            |app| {
                app.engine
                    .object_snapshot(builder)
                    .is_some_and(|object| object.action.name == "Walk")
            },
        );
        if app_clonk_carries(&app, builder, "COAL") {
            let incidental_coal = app
                .engine
                .object_snapshot(builder)
                .expect("rescuer survives outside drain")
                .contents
                .into_iter()
                .find(|object_id| {
                    app.engine
                        .object_snapshot(*object_id)
                        .is_some_and(|object| object.definition_id == "COAL")
                })
                .expect("rescuer carries exact incidental COAL");
            AppVirtualKeyboard::new(&mut app)
                .tap(VirtualKeyCode::A)
                .expect("physical A drops incidental COAL outside drain");
            advance_app_until(&mut app, "rescuer drops exact incidental COAL", 60, |app| {
                app.engine
                    .object_snapshot(incidental_coal)
                    .is_none_or(|coal| coal.container != Some(builder))
            });
        }
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::X)
            .expect("physical X stops rescuer above drained outlet");
        advance_app_until(
            &mut app,
            "lower outlet drains the upper passage",
            1_200,
            |app| !app.engine.debug_landscape_is_liquid(290, 310),
        );
        advance_app_until(
            &mut app,
            "rescuer stands above the drained outlet",
            80,
            |app| {
                app.engine
                    .object_snapshot(builder)
                    .is_some_and(|object| object.action.name == "Walk" && object.velocity.x == 0)
            },
        );
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::E)
            .expect("physical E selects the CRYS-carrying CLNK");
        assert_eq!(
            app.engine.crew_cursor(app.local_owner),
            Some(first_clonk),
            "physical CursorRight must select the exact trapped CLNK"
        );
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .tap(VirtualKeyCode::C)
                .expect("physical C faces trapped CLNK toward escape tunnel");
            keyboard
                .tap(VirtualKeyCode::X)
                .expect("physical X settles trapped CLNK at escape tunnel");
            keyboard
                .tap(VirtualKeyCode::D)
                .expect("physical D starts trapped CLNK's escape tunnel");
        }
        advance_app_until(
            &mut app,
            "trapped CLNK starts an escape tunnel",
            40,
            |app| {
                app.engine
                    .object_snapshot(first_clonk)
                    .is_some_and(|object| object.action.name == "Dig")
            },
        );
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .press(VirtualKeyCode::C)
                .expect("held physical C aims escape Dig right");
            keyboard
                .press(VirtualKeyCode::S)
                .expect("held physical S aims escape Dig up-right");
        }
        advance_app_until(
            &mut app,
            "trapped CLNK's escape tunnel reaches the lower cavern",
            400,
            |app| {
                app.engine
                    .object_snapshot(first_clonk)
                    .is_some_and(|object| object.action.name == "Walk" && object.position.x >= 200)
            },
        );
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .release(VirtualKeyCode::S)
                .expect("release physical S in the drained basin");
            keyboard
                .release(VirtualKeyCode::C)
                .expect("release physical C in the lower cavern");
        }
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::C,
            "escaped CLNK reaches the lower-cavern wall or flooded basin",
            80,
            |app| {
                app.engine
                    .object_snapshot(first_clonk)
                    .is_some_and(|object| {
                        object.action.name == "Scale" || object.action.name == "Swim"
                    })
            },
        );
        if app
            .engine
            .object_snapshot(first_clonk)
            .is_some_and(|object| object.action.name == "Scale")
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .tap(VirtualKeyCode::Z)
                .expect("physical Z releases the escaped CLNK from the cavern wall");
            advance_app_until(
                &mut app,
                "escaped CLNK releases the cavern wall",
                60,
                |app| {
                    app.engine
                        .object_snapshot(first_clonk)
                        .is_some_and(|object| object.action.name == "Walk")
                },
            );
            AppVirtualKeyboard::new(&mut app)
                .tap(VirtualKeyCode::D)
                .expect("physical D resumes digging east");
            advance_app_until(&mut app, "escaped CLNK resumes digging east", 40, |app| {
                app.engine
                    .object_snapshot(first_clonk)
                    .is_some_and(|object| object.action.name == "Dig")
            });
            {
                let mut keyboard = AppVirtualKeyboard::new(&mut app);
                keyboard
                    .press(VirtualKeyCode::C)
                    .expect("held physical C aims the resumed Dig right");
                keyboard
                    .press(VirtualKeyCode::S)
                    .expect("held physical S aims the resumed Dig up-right");
            }
            advance_app_until(
                &mut app,
                "escaped CLNK reaches the flooded basin",
                400,
                |app| {
                    app.engine
                        .object_snapshot(first_clonk)
                        .is_some_and(|object| object.action.name == "Swim")
                },
            );
            {
                let mut keyboard = AppVirtualKeyboard::new(&mut app);
                keyboard
                    .release(VirtualKeyCode::S)
                    .expect("release physical S in the flooded basin");
                keyboard
                    .release(VirtualKeyCode::C)
                    .expect("release physical C in the flooded basin");
            }
        }
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::C,
            "the CRYS carrier reaches the upper-passage wall",
            600,
            |app| {
                app.engine
                    .object_snapshot(first_clonk)
                    .is_some_and(|object| {
                        object.position.x >= 280 && object.action.name.starts_with("Scale")
                    })
            },
        );
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::S,
            "the CRYS carrier climbs beside the opened upper passage",
            160,
            |app| {
                app.engine
                    .object_snapshot(first_clonk)
                    .is_some_and(|object| object.position.y <= 306)
            },
        );
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::C,
            "the CRYS carrier traverses the opened upper passage",
            240,
            |app| {
                app.engine
                    .object_snapshot(first_clonk)
                    .is_some_and(|object| object.position.x >= 300)
            },
        );
        for _ in 0..2 {
            if !app_clonk_carries(&app, first_clonk, "CRYS") {
                break;
            }
            let previous_count = app
                .engine
                .object_snapshot(first_clonk)
                .expect("escaped CRYS carrier survives")
                .contents
                .len();
            {
                let mut keyboard = AppVirtualKeyboard::new(&mut app);
                keyboard
                    .tap(VirtualKeyCode::C)
                    .expect("physical C faces dropped content toward the rescuer");
                keyboard
                    .tap(VirtualKeyCode::A)
                    .expect("physical A drops one escaped-CLNK content");
            }
            advance_app_until(
                &mut app,
                "escaped CLNK drops one carried object",
                60,
                |app| {
                    app.engine
                        .object_snapshot(first_clonk)
                        .is_some_and(|object| object.contents.len() < previous_count)
                },
            );
            for _ in 0..12 {
                app.update()
                    .expect("wait out CRYS transfer double-click window");
            }
        }
        assert!(
            app.engine
                .object_snapshot(crystal)
                .is_some_and(|crystal| crystal.container != Some(first_clonk)),
            "the escaped CLNK must release the exact CRYS"
        );

        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::E)
            .expect("physical E returns to the elevator builder");
        assert_eq!(
            app.engine.crew_cursor(app.local_owner),
            Some(builder),
            "physical CursorRight must return to the exact builder"
        );
        if app
            .engine
            .object_snapshot(builder)
            .is_some_and(|object| object.action.name == "Scale")
        {
            AppVirtualKeyboard::new(&mut app)
                .tap(VirtualKeyCode::C)
                .expect("physical C releases the builder from the eastern cavern wall");
            advance_app_until(
                &mut app,
                "builder releases the eastern cavern wall",
                80,
                |app| {
                    app.engine
                        .object_snapshot(builder)
                        .is_some_and(|object| object.action.name != "Scale")
                },
            );
        }
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::Z,
            "builder grabs the eastern drain wall",
            160,
            |app| {
                app.engine
                    .object_snapshot(builder)
                    .is_some_and(|object| object.action.name == "Scale")
            },
        );
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::X,
            "builder descends beside the drain",
            180,
            |app| {
                app.engine.object_snapshot(builder).is_some_and(|object| {
                    object.action.name == "Walk" || object.action.name == "Swim"
                })
            },
        );
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::Z,
            "builder re-enters the drain",
            100,
            |app| {
                app.engine
                    .object_snapshot(builder)
                    .is_some_and(|object| object.action.name == "Swim")
            },
        );
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::X,
            "builder dives to the dropped CRYS",
            80,
            |app| {
                app.engine
                    .object_snapshot(builder)
                    .is_some_and(|object| object.position.y >= 325)
            },
        );
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::Z,
            "builder returns above the dropped CRYS",
            180,
            |app| {
                app.engine
                    .object_snapshot(builder)
                    .is_some_and(|object| object.position.x <= 305)
            },
        );
        if !app_clonk_carries(&app, builder, "CRYS") {
            hold_app_key_until(
                &mut app,
                VirtualKeyCode::X,
                "builder descends to the dropped CRYS",
                120,
                |app| {
                    app_clonk_carries(app, builder, "CRYS")
                        || app
                            .engine
                            .object_snapshot(builder)
                            .is_some_and(|object| object.position.y >= 328)
                },
            );
            let crystal_x = app
                .engine
                .object_snapshot(crystal)
                .expect("dropped CRYS survives in the drain")
                .position
                .x;
            let builder_x = app
                .engine
                .object_snapshot(builder)
                .expect("builder survives the drain dive")
                .position
                .x;
            let toward_crystal = if crystal_x < builder_x {
                VirtualKeyCode::Z
            } else {
                VirtualKeyCode::C
            };
            hold_app_key_until(
                &mut app,
                toward_crystal,
                "builder retrieves CRYS from the drain",
                80,
                |app| app_clonk_carries(app, builder, "CRYS"),
            );
        }
        assert!(
            app.engine
                .object_snapshot(crystal)
                .is_some_and(|crystal| crystal.container == Some(builder)),
            "builder retrieves exact CRYS from drain"
        );
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::C,
            "CRYS-carrying builder reaches ELEC shaft",
            80,
            |app| {
                app.engine
                    .object_snapshot(builder)
                    .is_some_and(|object| object.position.x >= 300)
            },
        );
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::S,
            "CRYS-carrying builder surfaces by ELEC",
            180,
            |app| {
                app.engine
                    .object_snapshot(builder)
                    .is_some_and(|object| object.action.name == "Walk" && object.position.y <= 316)
            },
        );
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::C,
            "CRYS-carrying builder centers over ELEC",
            80,
            |app| {
                app.engine
                    .object_snapshot(builder)
                    .is_some_and(|object| object.position.x >= 329)
            },
        );
        advance_app_until(
            &mut app,
            "CRYS-carrying builder settles on ELEC",
            80,
            |app| {
                app.engine.object_snapshot(builder).is_some_and(|object| {
                    object.action.name == "Walk" && (327..=333).contains(&object.position.x)
                })
            },
        );
        for _ in 0..12 {
            app.update()
                .expect("wait out CRYS-carrying ELEC grab buffer");
        }
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::X)
            .expect("physical X grabs ELEC under AutoStopControl");
        advance_app_until(
            &mut app,
            "CRYS-carrying builder grabs exact ELEC",
            100,
            |app| {
                app.engine.object_snapshot(builder).is_some_and(|object| {
                    object.action.name == "Push" && object.action.target == Some(elevator_case)
                })
            },
        );
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::S,
            "ELEC carries exact CRYS back to surface",
            600,
            |app| {
                app.engine
                    .object_snapshot(elevator_case)
                    .is_some_and(|object| object.position.y <= 160)
            },
        );
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .tap(VirtualKeyCode::X)
                .expect("first physical X primes surface ELEC release");
            keyboard
                .tap(VirtualKeyCode::X)
                .expect("second physical X releases surface ELEC");
        }
        advance_app_until(
            &mut app,
            "CRYS-carrying builder releases exact ELEC",
            80,
            |app| {
                app.engine
                    .object_snapshot(builder)
                    .is_some_and(|object| object.action.name == "Walk")
            },
        );
        let hut_x = app
            .engine
            .object_snapshot(hut)
            .expect("Tutorial06 keeps exact HUT3")
            .position
            .x;
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::Z,
            "CRYS-carrying builder returns to HUT3",
            120,
            |app| {
                app.engine
                    .object_snapshot(builder)
                    .is_some_and(|object| object.position.x <= hut_x + 4)
            },
        );
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::S)
            .expect("physical S enters HUT3 with exact CRYS");
        advance_app_until(
            &mut app,
            "CRYS-carrying builder enters exact HUT3",
            80,
            |app| {
                app.engine
                    .object_snapshot(builder)
                    .is_some_and(|object| object.container == Some(hut))
            },
        );
        advance_app_until(
            &mut app,
            "Tutorial06 asks player to sell CRYS",
            240,
            |app| app_tutorial_message_contains(app, "Sell the crystal"),
        );
        advance_app_until(&mut app, "HUT3 opens context for carried CRYS", 30, |app| {
            app.engine
                .cursor_object_menu(app.local_owner)
                .is_some_and(|(_, menu)| menu.identification == context_identification)
        });
        app_navigate_object_menu_to(&mut app, "Put", |item| item.caption == "Put");
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::A)
            .expect("physical A puts exact CRYS into HUT3");
        advance_app_until(
            &mut app,
            "context Put transfers exact CRYS into HUT3",
            40,
            |app| {
                app.engine
                    .object_snapshot(crystal)
                    .is_some_and(|object| object.container == Some(hut))
            },
        );
        advance_app_until(
            &mut app,
            "HUT3 restores context after putting CRYS",
            30,
            |app| {
                app.engine
                    .cursor_object_menu(app.local_owner)
                    .is_some_and(|(_, menu)| menu.identification == context_identification)
            },
        );
        app_navigate_object_menu_to(&mut app, "Sell", |item| item.caption == "Sell");
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::A)
            .expect("physical A opens HUT3 Sell menu");
        let sell_identification = serde_json::from_value(serde_json::json!({ "Int": 5 }))
            .expect("sell identification deserializes");
        advance_app_until(&mut app, "HUT3 opens its real Sell menu", 30, |app| {
            app.engine
                .cursor_object_menu(app.local_owner)
                .is_some_and(|(_, menu)| menu.identification == sell_identification)
        });
        app_navigate_object_menu_to(&mut app, "CRYS", |item| item.item_id == "CRYS");
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::A)
            .expect("physical A sells Tutorial06's exact CRYS");
        advance_app_until(
            &mut app,
            "selling CRYS removes exact objective object",
            60,
            |app| app.engine.object_snapshot(crystal).is_none(),
        );
        advance_app_until(&mut app, "Tutorial06 selects Tutorial07", 320, |app| {
            app.engine.next_mission().path == r"Tutorial.c4f\Tutorial07.c4s"
        });
        advance_app_until(
            &mut app,
            "Tutorial06 fulfilled goal reaches GameOver",
            320,
            |app| app.snapshot.game_over && app.game_over_dialog.is_some(),
        );
        assert!(
            app.snapshot
                .round_results
                .fulfilled_goals
                .iter()
                .any(|goal| goal == "SCRG"),
            "Tutorial06 must record fulfilled SCRG before GameOver"
        );
        assert!(
            app.engine.object_snapshot(crystal).is_none(),
            "Tutorial06's exact CRYS must be sold before SCRG fulfillment"
        );
        assert_eq!(
            app.engine.next_mission().path,
            r"Tutorial.c4f\Tutorial07.c4s"
        );
    }

    #[test]
    #[ignore = "over-constrained virtual tutorial driver; excluded from parity gates"]
    fn app_virtual_keyboard_completes_real_tutorial07_route() {
        // Script2..12 introduces the shipped CRYS/BALN/GOLD/FLNT route before
        // handing control back at "Good luck!" (Tutorial07.c4s/Script.c:
        // 36-90). From there every transition below enters through
        // GameApp::handle_key with the fresh player's AutoStop bindings:
        // S enters, X grabs/releases, D lowers ELEC, Z/C move, A throws, and F
        // executes the Contents menu's secondary Get command. The FLNT Hit ->
        // Explode(18) path must clear terrain, not merely spawn loose objects
        // (C4ObjectCom.cpp:335-350; C4Menu.cpp:433-440,498-523;
        // planet/System.c4g/Explode.c:10-22,58-65;
        // C4Landscape.cpp:1022-1061).
        let mut app = real_tutorial_app_with_roster(7, "Tutorial 7 app virtual player");
        let owner = app.local_owner;
        let clonk = app
            .engine
            .crew_cursor(owner)
            .expect("Tutorial07 starts with one cursor CLNK");
        let hut =
            app_object_with_definition(&app, "HUT3").expect("Tutorial07 creates the player's HUT3");
        let player = app
            .engine
            .snapshot()
            .players
            .into_iter()
            .find(|player| player.id == owner)
            .expect("Tutorial07 local player state");
        assert!(
            player.control.control_style,
            "fresh app players use C++ AutoStopControl"
        );

        advance_app_until(
            &mut app,
            "Tutorial07 presents its final route prompt",
            2_000,
            |app| app_tutorial_message_contains(app, "Good luck"),
        );
        assert!(app
            .engine
            .object_snapshot(clonk)
            .is_some_and(|object| { object.container.is_none() && object.action.name == "Walk" }));

        let context_identification = serde_json::from_value(serde_json::json!({ "Int": 14 }))
            .expect("context identification deserializes");
        let contents_identification = serde_json::from_value(serde_json::json!({ "Int": 18 }))
            .expect("contents identification deserializes");
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::S)
            .expect("physical S enters Tutorial07 HUT3");
        advance_app_until(&mut app, "Tutorial07 CLNK enters HUT3", 60, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.container == Some(hut))
        });
        advance_app_until(&mut app, "HUT3 opens its context menu", 30, |app| {
            app.engine
                .cursor_object_menu(owner)
                .is_some_and(|(_, menu)| menu.identification == context_identification)
        });
        app_navigate_object_menu_to(&mut app, "Contents", |item| item.caption == "Contents");
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::A)
            .expect("physical A opens Tutorial07 HUT3 Contents");
        advance_app_until(&mut app, "HUT3 opens its real Contents menu", 30, |app| {
            app.engine
                .cursor_object_menu(owner)
                .is_some_and(|(_, menu)| menu.identification == contents_identification)
        });
        app_navigate_object_menu_to(&mut app, "FLNT", |item| item.item_id == "FLNT");
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::F)
            .expect("physical F takes one Tutorial07 FLNT");
        advance_app_until(&mut app, "C++ Get keeps one FLNT", 120, |app| {
            app_object_contents_count(app, clonk, "FLNT") == 1
                && app_object_contents_count(app, hut, "FLNT") == 1
        });

        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::D)
            .expect("physical D closes HUT3 Contents");
        advance_app_until(&mut app, "HUT3 restores its context menu", 30, |app| {
            app.engine
                .cursor_object_menu(owner)
                .is_some_and(|(_, menu)| menu.identification == context_identification)
        });
        app_navigate_object_menu_to(&mut app, "Exit", |item| item.caption == "Exit");
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::A)
            .expect("physical A exits HUT3 with FLNT");
        advance_app_until(&mut app, "FLNT-carrying CLNK exits HUT3", 60, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.container.is_none() && object.action.name == "Walk")
        });

        let elevator_case =
            app_object_with_definition(&app, "ELEC").expect("Tutorial07 places a ready ELEC");
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::C,
            "FLNT-carrying CLNK reaches ELEC",
            100,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .zip(app.engine.object_snapshot(elevator_case))
                    .is_some_and(|(clonk, elevator)| {
                        (clonk.position.x - elevator.position.x).abs() <= 5
                    })
            },
        );
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::X)
            .expect("physical X grabs Tutorial07 ELEC");
        advance_app_until(&mut app, "CLNK grabs Tutorial07 ELEC", 60, |app| {
            app.engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Push" && object.action.target == Some(elevator_case)
            })
        });
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::D,
            "ELEC lowers CLNK to Tutorial07 GOLD layer",
            360,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.position.y >= 300)
            },
        );
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .tap(VirtualKeyCode::X)
                .expect("first physical X primes ELEC release");
            keyboard
                .tap(VirtualKeyCode::X)
                .expect("second physical X releases ELEC");
        }
        advance_app_until(&mut app, "CLNK releases ELEC at GOLD layer", 60, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk")
        });
        // Releasing ELEC used COM_DOWN double-click handling. Let that
        // window expire before approaching the wall so the following
        // physical COM_THROW remains Throw and the Clonk does not idle into
        // Scale while waiting at the blast pocket.
        for _ in 0..11 {
            app.update().expect("wait out first FLNT throw buffer");
        }
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::Z,
            "CLNK reaches Tutorial07 first GOLD-side blast pocket",
            80,
            |app| {
                app.engine.object_snapshot(clonk).is_some_and(|object| {
                    object.action.name == "Walk"
                        && object.position.x <= 120
                        && object.position.y >= 300
                })
            },
        );
        assert_eq!(
            AppVirtualKeyboard::new(&mut app)
                .player_control()
                .last_com_down_double,
            0,
            "first FLNT throw starts after the C++ down-double latch expires"
        );

        let first_flint = app
            .engine
            .object_snapshot(clonk)
            .and_then(|clonk| {
                clonk.contents.into_iter().find(|item| {
                    app.engine
                        .object_snapshot(*item)
                        .is_some_and(|item| item.definition_id == "FLNT")
                })
            })
            .expect("first Tutorial07 FLNT is ready to throw");
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::A)
            .expect("physical A throws the first FLNT");
        // Execute Throw before the retreat control can clear its command.
        // The same update may also run the first contact explosion, so keep
        // the pre-update landscape instead of hiding this transition behind
        // a generic inventory wait.
        let mut detonation = None;
        for _ in 0..60 {
            let before = app.engine.object_snapshot(first_flint).and_then(|flint| {
                app.engine
                    .landscape()
                    .cloned()
                    .map(|landscape| (flint.position, landscape))
            });
            app.update().expect("execute the first Tutorial07 FLNT throw");
            if let Some((center, before)) = before {
                if app.engine.object_snapshot(first_flint).is_none() {
                    detonation = app
                        .engine
                        .landscape()
                        .cloned()
                        .map(|after| (center, before, after));
                    break;
                }
            }
            if app
                .engine
                .object_snapshot(first_flint)
                .is_none_or(|flint| flint.container != Some(clonk))
            {
                break;
            }
        }
        assert!(
            app.engine
                .object_snapshot(first_flint)
                .is_none_or(|flint| flint.container != Some(clonk)),
            "physical A executes Throw before retreat"
        );
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::C)
            .expect("hold physical C to retreat from the first FLNT");
        if detonation.is_none() {
            for _ in 0..180 {
                let before = app.engine.object_snapshot(first_flint).and_then(|flint| {
                    app.engine
                        .landscape()
                        .cloned()
                        .map(|landscape| (flint.position, landscape))
                });
                app.update().expect("advance first Tutorial07 FLNT fuse");
                if let Some((center, before)) = before {
                    if app.engine.object_snapshot(first_flint).is_none() {
                        detonation = app
                            .engine
                            .landscape()
                            .cloned()
                            .map(|after| (center, before, after));
                        break;
                    }
                }
            }
        }
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::C)
            .expect("release physical C after the first FLNT");
        let (center, before, after) = detonation.unwrap_or_else(|| {
            panic!(
                "physical app route observes first FLNT detonation; flint={:?}; clonk={:?}; control={:?}",
                app.engine.object_snapshot(first_flint),
                app.engine.object_snapshot(clonk),
                AppVirtualKeyboard::new(&mut app).player_control(),
            )
        });
        assert!(
            app.engine.object_snapshot(first_flint).is_none(),
            "the real first FLNT is consumed by its explosion"
        );
        assert_eq!(
            app.engine
                .object_snapshot(clonk)
                .expect("CLNK survives first FLNT")
                .command_direction,
            CommandDirection::Stop,
            "AutoStop key-up stops the physical retreat"
        );

        let before_grid = before.pixel_grid().expect("pre-blast Tutorial07 Surface8");
        let after_grid = after.pixel_grid().expect("post-blast Tutorial07 Surface8");
        const FLINT_RADIUS: i32 = 18;
        const DENSITY_SOLID: i32 = 50;
        let mut changed_pixels = 0;
        let mut removed_solid_pixels = 0;
        for y_offset in -FLINT_RADIUS..=FLINT_RADIUS {
            let line_width =
                ((FLINT_RADIUS * FLINT_RADIUS - y_offset * y_offset) as f64).sqrt() as i32;
            let y = center.y + y_offset;
            for x_offset in -line_width..line_width + i32::from(line_width == 0) {
                let x = center.x + x_offset;
                changed_pixels +=
                    usize::from(before_grid.byte_at(x, y) != after_grid.byte_at(x, y));
                removed_solid_pixels += usize::from(
                    before_grid.density_at(x, y).unwrap_or(0) >= DENSITY_SOLID
                        && after_grid.density_at(x, y).unwrap_or(0) < DENSITY_SOLID,
                );
            }
        }
        assert!(after_grid.revision() > before_grid.revision());
        assert!(
            changed_pixels > 0 && removed_solid_pixels > 0,
            "first app-driven FLNT opens terrain (changed={changed_pixels}, removed_solid={removed_solid_pixels})"
        );

        // The shipped route requires a second ordinary HUT3/ELEC trip because
        // CLNK has one nonspecial inventory slot. The second Explode(18)
        // exposes the GOLD objects; four normal cabin sales fund the WRKS
        // production menu described by Script4/8/10 (Tutorial07.c4s/Script.c:
        // 48-78; Clonk.c4d/Script.c:738-763).
        app_tutorial07_return_to_hut(
            &mut app,
            clonk,
            elevator_case,
            hut,
            false,
            "CLNK after first FLNT blast",
            None,
        );
        advance_app_until(&mut app, "HUT3 opens for the second FLNT", 30, |app| {
            app.engine
                .cursor_object_menu(owner)
                .is_some_and(|(_, menu)| menu.identification == context_identification)
        });
        app_navigate_object_menu_to(&mut app, "Contents", |item| item.caption == "Contents");
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::A)
            .expect("physical A reopens HUT3 Contents");
        advance_app_until(&mut app, "HUT3 exposes its second FLNT", 30, |app| {
            app.engine
                .cursor_object_menu(owner)
                .is_some_and(|(_, menu)| menu.identification == contents_identification)
        });
        app_navigate_object_menu_to(&mut app, "second FLNT", |item| item.item_id == "FLNT");
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::A)
            .expect("physical A takes the second Tutorial07 FLNT");
        advance_app_until(&mut app, "CLNK carries the second FLNT", 120, |app| {
            app_object_contents_count(app, clonk, "FLNT") == 1
                && app_object_contents_count(app, hut, "FLNT") == 0
        });
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::D)
            .expect("physical D closes HUT3 Contents after second FLNT");
        app_tutorial07_exit_hut_and_descend_to_gold(&mut app, clonk, elevator_case, hut);
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::Z,
            "second FLNT reaches Tutorial07 marked GOLD seam",
            80,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.position.x <= 92)
            },
        );
        let attached = app
            .engine
            .object_snapshot(clonk)
            .filter(|object| {
                object.action.name == "Hangle" || object.action.name.starts_with("Scale")
            })
            .map(|object| (object.action.name, object.direction));
        if let Some((action, direction)) = attached {
            let let_go = if action.starts_with("Scale") {
                if direction == Direction::Left {
                    VirtualKeyCode::C
                } else {
                    VirtualKeyCode::Z
                }
            } else {
                VirtualKeyCode::X
            };
            AppVirtualKeyboard::new(&mut app)
                .tap(let_go)
                .expect("physical control drops CLNK before second FLNT throw");
            advance_app_until(
                &mut app,
                "CLNK lands before second FLNT throw",
                120,
                |app| {
                    app.engine
                        .object_snapshot(clonk)
                        .is_some_and(|object| object.action.name == "Walk")
                },
            );
        }
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::Z,
            "CLNK faces the GOLD seam before second FLNT throw",
            30,
            |app| {
                app.engine.object_snapshot(clonk).is_some_and(|object| {
                    object.action.name == "Walk" && object.direction == Direction::Left
                })
            },
        );
        for _ in 0..11 {
            app.update().expect("wait out pre-throw key buffer");
        }
        let second_flint = app
            .engine
            .object_snapshot(clonk)
            .and_then(|clonk| {
                clonk.contents.into_iter().find(|item| {
                    app.engine
                        .object_snapshot(*item)
                        .is_some_and(|item| item.definition_id == "FLNT")
                })
            })
            .expect("second Tutorial07 FLNT is ready to throw");
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::A)
            .expect("physical A throws the second FLNT");
        advance_app_until(&mut app, "second FLNT leaves CLNK inventory", 60, |app| {
            app.engine
                .object_snapshot(second_flint)
                .is_none_or(|flint| flint.container != Some(clonk))
        });
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::C,
            "second Tutorial07 FLNT detonates",
            180,
            |app| app.engine.object_snapshot(second_flint).is_none(),
        );
        app_tutorial07_climb_right_out_of_blast_pocket(
            &mut app,
            clonk,
            120,
            "CLNK retreats from the second FLNT blast",
        );
        assert!(
            app.engine
                .snapshot()
                .objects
                .iter()
                .any(|object| object.definition_id == "GOLD"),
            "two real FLNT blasts expose GOLD objects"
        );

        hold_app_key_until(
            &mut app,
            VirtualKeyCode::Z,
            "CLNK reaches the far wall of the exposed GOLD pocket",
            200,
            |app| {
                app_clonk_carries(app, clonk, "GOLD")
                    || app.engine.object_snapshot(clonk).is_some_and(|object| {
                        object.position.x <= 50 && object.action.name.starts_with("Scale")
                    })
            },
        );
        if !app_clonk_carries(&app, clonk, "GOLD") {
            for _ in 0..12 {
                app.update()
                    .expect("wait out the far-wall GOLD-pocket control buffer");
            }
            hold_app_key_until(
                &mut app,
                VirtualKeyCode::C,
                "CLNK naturally collects the first exposed GOLD chunk",
                200,
                |app| app_clonk_carries(app, clonk, "GOLD"),
            );
        }
        app_tutorial07_return_to_hut(
            &mut app,
            clonk,
            elevator_case,
            hut,
            false,
            "first GOLD-carrying CLNK",
            None,
        );
        // ExecLife buys the empty base's first 100 energy for five wealth.
        // The first sale therefore funds the base rather than BALN.
        for target_wealth in [5, 10, 15, 20] {
            app_tutorial07_exit_hut_and_descend_to_gold(&mut app, clonk, elevator_case, hut);
            hold_app_key_until(
                &mut app,
                VirtualKeyCode::C,
                "empty CLNK sweeps the far side of the GOLD seam",
                160,
                |app| {
                    app_clonk_carries(app, clonk, "GOLD")
                        || app
                            .engine
                            .object_snapshot(clonk)
                            .is_some_and(|object| object.position.x >= 165)
                },
            );
            if !app_clonk_carries(&app, clonk, "GOLD") {
                hold_app_key_until(
                    &mut app,
                    VirtualKeyCode::Z,
                    "CLNK naturally collects another GOLD chunk",
                    180,
                    |app| app_clonk_carries(app, clonk, "GOLD"),
                );
            }
            app_tutorial07_return_to_hut(
                &mut app,
                clonk,
                elevator_case,
                hut,
                false,
                "GOLD-carrying CLNK",
                Some(target_wealth),
            );
        }
        assert_eq!(
            app.engine
                .player(owner)
                .expect("funded Tutorial07 player")
                .wealth(),
            20,
            "five exact GOLD sales fund base energy and BALN production"
        );

        let workshop =
            app_object_with_definition(&app, "WRKS").expect("Tutorial07 creates the player's WRKS");
        advance_app_until(&mut app, "HUT3 restores after fifth GOLD sale", 30, |app| {
            app.engine
                .cursor_object_menu(owner)
                .is_some_and(|(_, menu)| menu.identification == context_identification)
        });
        app_navigate_object_menu_to(&mut app, "Exit", |item| item.caption == "Exit");
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::A)
            .expect("physical A exits funded CLNK from HUT3");
        advance_app_until(&mut app, "funded CLNK exits HUT3", 60, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.container != Some(hut))
        });
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::C)
            .expect("physical C starts funded walk toward WRKS");
        let mut previous_action = app
            .engine
            .object_snapshot(clonk)
            .expect("funded CLNK survives HUT3 exit")
            .action
            .name;
        for _ in 0..360 {
            let clonk_now = app
                .engine
                .object_snapshot(clonk)
                .expect("funded CLNK survives WRKS walk");
            if clonk_now.position.x >= 155 {
                break;
            }
            let action = clonk_now.action.name;
            let entered_scale =
                action.starts_with("Scale") && !previous_action.starts_with("Scale");
            let left_scale_in_flight = action == "Jump" && previous_action.starts_with("Scale");
            let landed = action == "Walk" && previous_action != "Walk";
            if entered_scale {
                let mut keyboard = AppVirtualKeyboard::new(&mut app);
                keyboard
                    .release(VirtualKeyCode::C)
                    .expect("release physical C on WRKS Scale");
                keyboard
                    .press(VirtualKeyCode::C)
                    .expect("repress physical C on WRKS Scale");
            } else if landed || left_scale_in_flight {
                AppVirtualKeyboard::new(&mut app)
                    .tap(VirtualKeyCode::S)
                    .expect("physical S jumps toward WRKS");
            }
            previous_action = action;
            app.update().expect("advance funded WRKS walk");
        }
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::C)
            .expect("release physical C at WRKS");
        assert!(app
            .engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.position.x >= 155));
        advance_app_until(&mut app, "funded CLNK lands in WRKS entrance", 120, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk")
        });
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::Z,
            "funded CLNK aligns with WRKS entrance",
            100,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.position.x <= 160)
            },
        );
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::S)
            .expect("physical S enters Tutorial07 WRKS");
        advance_app_until(&mut app, "funded CLNK enters WRKS", 60, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.container == Some(workshop))
        });
        advance_app_until(&mut app, "WRKS opens its context menu", 30, |app| {
            app.engine
                .cursor_object_menu(owner)
                .is_some_and(|(_, menu)| menu.identification == context_identification)
        });
        app_navigate_object_menu_to(&mut app, "Production", |item| item.caption == "Production");
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::A)
            .expect("physical A opens WRKS Production");
        let production_identification =
            serde_json::from_value(serde_json::json!({ "C4Id": "CXCN" }))
                .expect("production identification deserializes");
        advance_app_until(&mut app, "WRKS opens real Production menu", 30, |app| {
            app.engine
                .cursor_object_menu(owner)
                .is_some_and(|(_, menu)| menu.identification == production_identification)
        });
        app_navigate_object_menu_to(&mut app, "BALN", |item| item.item_id == "BALN");
        assert_eq!(
            app_selected_object_menu_item(&app).map(|item| item.item_id.as_str()),
            Some("BALN"),
            "physical menu navigation reaches the shipped BALN production prompt"
        );

        // Production executes the selected menu row, consumes the ordinary
        // WRKS components and runs C4Command::Build until the finished vehicle
        // receives Exit (C4Command.cpp:823-899). The remaining controls mirror
        // a real player: X/X boards or leaves BALN, held S climbs, and normal
        // C/Z walking collects CRYS. Script14 observes that containment and
        // advances to the sailboat prompt without any test-side state write
        // (Tutorial07.c4s/Script.c:92-99).
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::A)
            .expect("physical A starts BALN production");
        advance_app_until(&mut app, "WRKS creates real BALN construction", 80, |app| {
            app_object_with_definition(app, "BALN").is_some()
        });
        let balloon = app_object_with_definition(&app, "BALN")
            .expect("Tutorial07 BALN construction keeps its identity");
        advance_app_until(
            &mut app,
            "WRKS completes BALN through normal production",
            2_400,
            |app| {
                app.engine
                    .object_snapshot(balloon)
                    .is_some_and(|object| object.construction == 100_000)
            },
        );
        advance_app_until(&mut app, "completed BALN exits WRKS", 160, |app| {
            app.engine
                .object_snapshot(balloon)
                .is_some_and(|object| object.container.is_none())
        });
        if app
            .engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.container.is_some())
        {
            if app.engine.cursor_object_menu(owner).is_none() {
                AppVirtualKeyboard::new(&mut app)
                    .tap(VirtualKeyCode::S)
                    .expect("physical S restores WRKS context after production");
                advance_app_until(&mut app, "WRKS restores context after BALN", 30, |app| {
                    app.engine
                        .cursor_object_menu(owner)
                        .is_some_and(|(_, menu)| menu.identification == context_identification)
                });
            }
            app_navigate_object_menu_to(&mut app, "Exit", |item| item.caption == "Exit");
            AppVirtualKeyboard::new(&mut app)
                .tap(VirtualKeyCode::A)
                .expect("physical A exits BALN builder from WRKS");
        }
        advance_app_until(&mut app, "BALN builder exits WRKS", 100, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.container.is_none() && object.action.name == "Walk")
        });

        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .tap(VirtualKeyCode::X)
                .expect("first physical X primes BALN boarding");
            keyboard
                .tap(VirtualKeyCode::X)
                .expect("second physical X boards BALN");
        }
        advance_app_until(&mut app, "CLNK boards produced BALN", 100, |app| {
            app.engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Push" && object.action.target == Some(balloon)
            })
        });
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::S)
            .expect("held physical S climbs in BALN");
        // The 50px BALN must clear Landscape.bmp's rough crystal shelf near
        // y=130 before Stop/ClearDir's downward coast. The pushing CLNK rides
        // about 12px below BALN's origin, so y=80 retains a safe margin.
        advance_app_until(&mut app, "BALN climbs to CRYS flight level", 180, |app| {
            app.engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Push"
                    && object.action.target == Some(balloon)
                    && object.position.y <= 80
            })
        });
        AppVirtualKeyboard::new(&mut app)
            .release(VirtualKeyCode::S)
            .expect("release physical S at CRYS flight level");
        assert_eq!(
            app.engine
                .object_snapshot(balloon)
                .expect("BALN after physical S release")
                .command_direction,
            CommandDirection::Stop,
            "physical S release immediately stops BALN through ControlUpdate"
        );
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::X)
            .expect("physical X down-tap returns BALN to Stop");
        assert_eq!(
            app.engine
                .object_snapshot(balloon)
                .expect("BALN after physical X tap")
                .command_direction,
            CommandDirection::Stop,
            "physical X release immediately restores BALN Stop through ControlUpdate"
        );
        for _ in 0..11 {
            app.update().expect("wait out BALN vertical-control buffer");
        }
        // BALN/CLNK stop at x=565 against the near face of the crystal
        // cliff. Requiring the object anchor to cross x=570 makes arrival
        // depend on a favorable wind impulse instead of physical contact.
        const CRYSTAL_CLIFF_X: i32 = 565;
        for coast_frame in 0..900 {
            if app.engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Push"
                    && object.action.target == Some(balloon)
                    && object.position.x >= CRYSTAL_CLIFF_X
            }) {
                break;
            }
            app.update()
                .expect("advance BALN toward opposite CRYS cliff");
            let balloon_now = app
                .engine
                .object_snapshot(balloon)
                .expect("BALN during opposite-cliff coast");
            if balloon_now.command_direction == CommandDirection::Down {
                // BALN::CheckWindY deliberately changes Stop to Down when the
                // drifting balloon reaches solid ground (Balloon.c4d/Script.c:
                // 134-143). At this route's cliff contact a player must hold
                // Up again to clear the lip; keep that correction on the same
                // physical-key boundary as live play.
                assert_eq!(
                    app.engine
                        .snapshot()
                        .players
                        .into_iter()
                        .find(|player| player.id == owner)
                        .expect("Tutorial07 player survives BALN coast")
                        .control
                        .pressed_coms,
                    0,
                    "BALN's ground-triggered Down is not a stuck physical key"
                );
                AppVirtualKeyboard::new(&mut app)
                    .press(VirtualKeyCode::S)
                    .expect("physical S raises BALN away from the cliff contact");
                advance_app_until(
                    &mut app,
                    "BALN clears the CRYS cliff lip under held physical S",
                    240,
                    |app| {
                        app.engine
                            .object_snapshot(balloon)
                            .is_some_and(|object| object.position.y <= 90)
                    },
                );
                AppVirtualKeyboard::new(&mut app)
                    .release(VirtualKeyCode::S)
                    .expect("release physical S above the CRYS cliff lip");
                assert_eq!(
                    app.engine
                        .object_snapshot(balloon)
                        .expect("BALN after corrective physical S release")
                        .command_direction,
                    CommandDirection::Stop,
                    "corrective S release returns BALN to Stop"
                );
            } else {
                assert_eq!(
                    balloon_now.command_direction,
                    CommandDirection::Stop,
                    "BALN has an unexpected coast direction on frame {coast_frame}"
                );
            }
        }
        assert!(
            app.engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Push"
                    && object.action.target == Some(balloon)
                    && object.position.x >= CRYSTAL_CLIFF_X
            }),
            "BALN reaches opposite CRYS cliff; clonk={:?}; balloon={:?}; control={:?}",
            app.engine.object_snapshot(clonk),
            app.engine.object_snapshot(balloon),
            app.engine
                .snapshot()
                .players
                .into_iter()
                .find(|player| player.id == owner)
                .map(|player| player.control),
        );

        let crystal =
            app_object_with_definition(&app, "CRYS").expect("Tutorial07 creates objective CRYS");
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .tap(VirtualKeyCode::X)
                .expect("first physical X primes BALN exit");
            keyboard
                .tap(VirtualKeyCode::X)
                .expect("second physical X leaves BALN");
        }
        advance_app_until(&mut app, "CLNK lands on CRYS cliff", 180, |app| {
            app.engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Walk"
                    && object.container.is_none()
                    && object.position.x >= CRYSTAL_CLIFF_X
            })
        });
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::C,
            "CLNK crosses the objective CRYS",
            120,
            |app| {
                app_clonk_carries(app, clonk, "CRYS")
                    || app
                        .engine
                        .object_snapshot(clonk)
                        .is_some_and(|object| object.position.x >= 650)
            },
        );
        if !app_clonk_carries(&app, clonk, "CRYS") {
            hold_app_key_until(
                &mut app,
                VirtualKeyCode::Z,
                "CLNK naturally collects objective CRYS",
                180,
                |app| app_clonk_carries(app, clonk, "CRYS"),
            );
        }
        assert_eq!(
            app.engine
                .object_snapshot(crystal)
                .expect("objective CRYS survives collection")
                .container,
            Some(clonk),
            "physical walking puts Tutorial07 CRYS in CLNK inventory"
        );
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::C,
            "CRYS-carrying CLNK steps fully onto the cliff",
            120,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.position.x >= 650 && object.action.name == "Walk")
            },
        );
        advance_app_until(
            &mut app,
            "Tutorial07 advances to the CRYS-side sailboat prompt",
            240,
            |app| app_tutorial_message_contains(app, "Dig through the earth"),
        );
        // Starting DownLeft at the irregular crest can clear the Clonk's
        // bottom attachment against a one-pixel seam. Cut left while the
        // shelf still supports DFA_DIG, then add Down after the seam. These
        // are the same balanced AutoStop key edges a live player uses; no
        // object position or terrain state is written by the test.
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::D)
            .expect("physical D arms the sailboat tunnel");
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::Z)
            .expect("held physical Z starts the sailboat tunnel left");
        advance_app_until(
            &mut app,
            "CRYS-carrying CLNK starts the sailboat tunnel",
            1,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.action.name == "Dig")
            },
        );
        advance_app_until(
            &mut app,
            "horizontal tunnel clears the irregular crest seam",
            120,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.action.name == "Dig" && object.position.x <= 635)
            },
        );
        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::X)
            .expect("held physical X turns the sailboat tunnel down-left");
        advance_app_until(
            &mut app,
            "diagonal tunnel opens toward the sailboat cave",
            260,
            |app| {
                app.engine.object_snapshot(clonk).is_some_and(|object| {
                    object.position.y >= 290
                        || (object.action.name == "Walk" && object.position.x <= 575)
                })
            },
        );
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .release(VirtualKeyCode::X)
                .expect("release physical X at the first tunnel exit");
            keyboard
                .release(VirtualKeyCode::Z)
                .expect("release physical Z at the first tunnel exit");
        }
        assert!(
            app.engine.object_snapshot(clonk).is_some_and(|object| {
                object.position.x <= 575 || object.position.y >= 290
            }),
            "physical digging opens the diagonal tunnel toward SLBS; clonk={:?}",
            app.engine.object_snapshot(clonk)
        );

        let sailboat = app_object_with_definition(&app, "SLBS")
            .or_else(|| app_object_with_definition(&app, "SLBT"))
            .expect("Tutorial07 creates its return sailboat");
        for lip in 1..=12 {
            let clonk_now = app
                .engine
                .object_snapshot(clonk)
                .expect("CRYS-carrying CLNK survives above SLBS");
            let sailboat_x = app
                .engine
                .object_snapshot(sailboat)
                .expect("SLBS survives the cave-lip descent")
                .position
                .x;
            if clonk_now.position.y >= 290 || clonk_now.position.x <= sailboat_x {
                break;
            }
            if clonk_now.action.name == "Jump" {
                let toward_sailboat = if clonk_now.position.x < sailboat_x {
                    VirtualKeyCode::C
                } else {
                    VirtualKeyCode::Z
                };
                hold_app_key_until(
                    &mut app,
                    toward_sailboat,
                    &format!("CRYS-carrying CLNK steers across cave lip {lip}"),
                    180,
                    |app| {
                        app.engine.object_snapshot(clonk).is_some_and(|object| {
                            object.position.y >= 290
                                || matches!(
                                    object.action.name.as_str(),
                                    "Walk" | "Scale" | "ScaleDown"
                                )
                        })
                    },
                );
            } else if clonk_now.action.name.starts_with("Scale") {
                hold_app_key_until(
                    &mut app,
                    VirtualKeyCode::X,
                    &format!("CRYS-carrying CLNK descends cave lip {lip}"),
                    180,
                    |app| {
                        app.engine.object_snapshot(clonk).is_some_and(|object| {
                            object.position.y >= 290 || object.action.name == "Walk"
                        })
                    },
                );
            } else {
                hold_app_key_until(
                    &mut app,
                    VirtualKeyCode::Z,
                    &format!("CRYS-carrying CLNK reaches cave lip {lip}"),
                    120,
                    |app| {
                        app.engine.object_snapshot(clonk).is_some_and(|object| {
                            object.position.y >= 290
                                || object.position.x <= sailboat_x
                                || object.action.name.starts_with("Scale")
                                || object.action.name == "Jump"
                        })
                    },
                );
            }
        }
        for segment in 1..=20 {
            let start_position = app
                .engine
                .object_snapshot(clonk)
                .expect("CLNK survives between sailboat-cave ledges")
                .position;
            if start_position.y >= 290 {
                break;
            }
            AppVirtualKeyboard::new(&mut app)
                .tap(VirtualKeyCode::D)
                .unwrap_or_else(|error| panic!("physical D starts cave dig {segment}: {error}"));
            AppVirtualKeyboard::new(&mut app)
                .press(VirtualKeyCode::X)
                .unwrap_or_else(|error| panic!("physical X aims cave dig {segment}: {error}"));
            if segment >= 2 {
                AppVirtualKeyboard::new(&mut app)
                    .press(VirtualKeyCode::C)
                    .unwrap_or_else(|error| {
                        panic!("physical C aims cave dig {segment} right: {error}")
                    });
            }
            advance_app_until(
                &mut app,
                &format!("CLNK starts sailboat-cave dig segment {segment}"),
                1,
                |app| {
                    app.engine
                        .object_snapshot(clonk)
                        .is_some_and(|object| object.action.name == "Dig")
                },
            );
            assert_eq!(
                app.engine
                    .object_snapshot(clonk)
                    .expect("CLNK retains requested cave-dig heading")
                    .command_direction,
                if segment == 1 {
                    CommandDirection::Down
                } else {
                    CommandDirection::DownRight
                },
                "physical keys choose cave-dig heading for segment {segment}"
            );
            advance_app_until(
                &mut app,
                &format!("cave dig segment {segment} reaches air or SLBS"),
                180,
                |app| {
                    app.engine.object_snapshot(clonk).is_some_and(|object| {
                        object.position.y >= 290 || object.action.name == "Walk"
                    })
                },
            );
            if segment >= 2 {
                AppVirtualKeyboard::new(&mut app)
                    .release(VirtualKeyCode::C)
                    .unwrap_or_else(|error| {
                        panic!("release physical C after cave dig {segment}: {error}")
                    });
            }
            AppVirtualKeyboard::new(&mut app)
                .release(VirtualKeyCode::X)
                .unwrap_or_else(|error| {
                    panic!("release physical X after cave dig {segment}: {error}")
                });
            if segment == 2 {
                for lip in 1..=12 {
                    let step_start_y = app
                        .engine
                        .object_snapshot(clonk)
                        .expect("CLNK survives the tunnel descent")
                        .position
                        .y;
                    if step_start_y >= 290 {
                        break;
                    }
                    if app
                        .engine
                        .object_snapshot(clonk)
                        .is_some_and(|object| object.action.name == "Walk")
                    {
                        hold_app_key_until(
                            &mut app,
                            VirtualKeyCode::C,
                            &format!("CLNK walks off tunnel lip {lip}"),
                            120,
                            |app| {
                                app.engine.object_snapshot(clonk).is_some_and(|object| {
                                    object.position.y >= 290
                                        || matches!(object.action.name.as_str(), "Jump" | "Scale")
                                })
                            },
                        );
                    }
                    advance_app_until(
                        &mut app,
                        &format!("CLNK clears or catches tunnel lip {lip}"),
                        30,
                        |app| {
                            app.engine.object_snapshot(clonk).is_some_and(|object| {
                                object.position.y >= 290
                                    || object.action.name == "Scale"
                                    || (object.action.name == "Walk"
                                        && object.position.y > step_start_y)
                            })
                        },
                    );
                    let after_lip = app
                        .engine
                        .object_snapshot(clonk)
                        .expect("CLNK survives the tunnel lip");
                    if after_lip.position.y >= 290 {
                        break;
                    }
                    if after_lip.action.name == "Walk"
                        && after_lip.position.y > step_start_y
                    {
                        continue;
                    }
                    AppVirtualKeyboard::new(&mut app)
                        .press(VirtualKeyCode::X)
                        .unwrap_or_else(|error| {
                            panic!("physical X scales below tunnel lip {lip}: {error}")
                        });
                    advance_app_until(
                        &mut app,
                        &format!("CLNK scales below tunnel lip {lip}"),
                        120,
                        |app| {
                            app.engine.object_snapshot(clonk).is_some_and(|object| {
                                object.position.y >= 290
                                    || (object.action.name == "Walk"
                                        && object.position.y > step_start_y)
                            })
                        },
                    );
                    AppVirtualKeyboard::new(&mut app)
                        .release(VirtualKeyCode::X)
                        .unwrap_or_else(|error| {
                            panic!("release physical X below tunnel lip {lip}: {error}")
                        });
                }
            }
        }
        assert!(
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.y >= 290),
            "physical digging descends into the sailboat cave; clonk={:?}; sailboat={:?}",
            app.engine.object_snapshot(clonk),
            app.engine.object_snapshot(sailboat)
        );

        for approach in 1..=12 {
            let (clonk_now, sailboat_now) = app
                .engine
                .object_snapshot(clonk)
                .zip(app.engine.object_snapshot(sailboat))
                .expect("CLNK and SLBS survive the cave approach");
            if clonk_now.action.name == "Walk"
                && (clonk_now.position.x - sailboat_now.position.x).abs() <= 5
                && (clonk_now.position.y - sailboat_now.position.y).abs() <= 20
            {
                break;
            }
            if clonk_now.action.name.starts_with("Scale") {
                if clonk_now.position.y > sailboat_now.position.y + 10 {
                    hold_app_key_until(
                        &mut app,
                        VirtualKeyCode::S,
                        &format!("CLNK climbs toward SLBS on approach {approach}"),
                        180,
                        |app| {
                            app.engine
                                .object_snapshot(clonk)
                                .zip(app.engine.object_snapshot(sailboat))
                                .is_some_and(|(clonk, sailboat)| {
                                    clonk.position.y <= sailboat.position.y + 10
                                        || clonk.action.name == "Walk"
                                })
                        },
                    );
                } else {
                    let away_from_wall = if clonk_now.direction == Direction::Left {
                        VirtualKeyCode::C
                    } else {
                        VirtualKeyCode::Z
                    };
                    AppVirtualKeyboard::new(&mut app)
                        .tap(away_from_wall)
                        .unwrap_or_else(|error| {
                            panic!("physical key leaves scale on approach {approach}: {error}")
                        });
                }
                continue;
            }
            if clonk_now.action.name == "Jump" {
                let toward_sailboat = if clonk_now.position.x < sailboat_now.position.x {
                    VirtualKeyCode::C
                } else {
                    VirtualKeyCode::Z
                };
                hold_app_key_until(
                    &mut app,
                    toward_sailboat,
                    &format!("CLNK lands during SLBS approach {approach}"),
                    120,
                    |app| {
                        app.engine.object_snapshot(clonk).is_some_and(|object| {
                            matches!(object.action.name.as_str(), "Walk" | "Scale" | "ScaleDown")
                        })
                    },
                );
                continue;
            }
            let horizontal = if clonk_now.position.x < sailboat_now.position.x - 5 {
                VirtualKeyCode::C
            } else {
                VirtualKeyCode::Z
            };
            hold_app_key_until(
                &mut app,
                horizontal,
                &format!("CLNK closes on SLBS during approach {approach}"),
                180,
                |app| {
                    app.engine
                        .object_snapshot(clonk)
                        .zip(app.engine.object_snapshot(sailboat))
                        .is_some_and(|(clonk, sailboat)| {
                            ((clonk.position.x - sailboat.position.x).abs() <= 5
                                && (clonk.position.y - sailboat.position.y).abs() <= 20)
                                || clonk.action.name != "Walk"
                        })
                },
            );
        }
        assert!(
            app.engine
                .object_snapshot(clonk)
                .zip(app.engine.object_snapshot(sailboat))
                .is_some_and(|(clonk, sailboat)| {
                    clonk.action.name == "Walk"
                        && (clonk.position.x - sailboat.position.x).abs() <= 5
                        && (clonk.position.y - sailboat.position.y).abs() <= 20
                }),
            "physical cave traversal reaches SLBS; clonk={:?}; sailboat={:?}",
            app.engine.object_snapshot(clonk),
            app.engine.object_snapshot(sailboat)
        );
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .tap(VirtualKeyCode::X)
                .expect("first physical X primes SLBS grab");
            keyboard
                .tap(VirtualKeyCode::X)
                .expect("second physical X grabs SLBS");
        }
        advance_app_until(&mut app, "CRYS-carrying CLNK grabs SLBS", 100, |app| {
            app.engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Push" && object.action.target == Some(sailboat)
            })
        });
        advance_app_until(&mut app, "Tutorial07 asks CLNK to sail home", 120, |app| {
            app_tutorial_message_contains(app, "Use the boat to sail back home")
        });

        // Held physical Z is forwarded through SLBS::ControlUpdate and its
        // ordinary Wind2Sail action. Reach the home cave, release Z to Stop,
        // then leave the boat with the normal X/X ungrab gesture.
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::Z,
            "SLBS reaches Tutorial07's home cave",
            900,
            |app| {
                app.engine
                    .object_snapshot(sailboat)
                    .is_some_and(|object| object.position.x <= 210)
            },
        );
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .tap(VirtualKeyCode::X)
                .expect("first physical X primes home-cave SLBS release");
            keyboard
                .tap(VirtualKeyCode::X)
                .expect("second physical X leaves SLBS at home");
        }
        advance_app_until(&mut app, "CLNK steps off SLBS at home", 100, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk")
        });
        hold_app_key_until(
            &mut app,
            VirtualKeyCode::Z,
            "CRYS-carrying CLNK walks from SLBS into the home cave",
            160,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.position.x <= 170)
            },
        );
        advance_app_until(&mut app, "CLNK stands inside the home cave", 60, |app| {
            app.engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk")
        });
        assert_eq!(
            app.engine
                .object_snapshot(crystal)
                .expect("objective CRYS survives the return crossing")
                .container,
            Some(clonk),
            "CRYS remains in CLNK inventory on reaching the home cave"
        );

        hold_app_key_until(
            &mut app,
            VirtualKeyCode::Z,
            "CRYS-carrying CLNK reaches the home blast-pocket wall",
            160,
            |app| {
                app.engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.position.x <= 70)
            },
        );
        app_tutorial07_return_to_hut(
            &mut app,
            clonk,
            elevator_case,
            hut,
            true,
            "CRYS-carrying CLNK",
            None,
        );

        // Script18 unwraps CRYS through CLNK into HUT3. Use HUT3's ordinary
        // context Put and Sell menus; Script19 fulfills SCRG only after the
        // selected sale removes the exact objective object
        // (Tutorial07.c4s/Script.c:113-127).
        advance_app_until(
            &mut app,
            "Tutorial07 asks the player to sell CRYS",
            240,
            |app| app_tutorial_message_contains(app, "Sell the crystal"),
        );
        advance_app_until(&mut app, "HUT3 opens context for carried CRYS", 30, |app| {
            app.engine
                .cursor_object_menu(owner)
                .is_some_and(|(_, menu)| menu.identification == context_identification)
        });
        app_navigate_object_menu_to(&mut app, "Put", |item| item.caption == "Put");
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::A)
            .expect("physical A transfers CRYS through HUT3 context Put");
        advance_app_until(
            &mut app,
            "context Put transfers CRYS into HUT3",
            40,
            |app| {
                app.engine
                    .object_snapshot(crystal)
                    .is_some_and(|object| object.container == Some(hut))
            },
        );
        advance_app_until(
            &mut app,
            "HUT3 restores context after putting CRYS",
            30,
            |app| {
                app.engine
                    .cursor_object_menu(owner)
                    .is_some_and(|(_, menu)| menu.identification == context_identification)
            },
        );
        app_navigate_object_menu_to(&mut app, "Sell", |item| item.caption == "Sell");
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::A)
            .expect("physical A opens HUT3 Sell menu");
        let sell_identification = serde_json::from_value(serde_json::json!({ "Int": 5 }))
            .expect("sell identification deserializes");
        advance_app_until(&mut app, "HUT3 opens its real Sell menu", 30, |app| {
            app.engine
                .cursor_object_menu(owner)
                .is_some_and(|(_, menu)| menu.identification == sell_identification)
        });
        app_navigate_object_menu_to(&mut app, "CRYS", |item| item.item_id == "CRYS");
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::A)
            .expect("physical A sells the objective CRYS");
        advance_app_until(&mut app, "selling CRYS removes the objective", 60, |app| {
            app.engine.object_snapshot(crystal).is_none()
        });
        advance_app_until(&mut app, "Tutorial07 selects Tutorial08", 320, |app| {
            app.engine.next_mission().path == r"Tutorial.c4f\Tutorial08.c4s"
        });
        advance_app_until(
            &mut app,
            "Tutorial07 fulfilled goal reaches GameOver",
            320,
            |app| app.snapshot.game_over && app.game_over_dialog.is_some(),
        );
        assert!(
            app.snapshot
                .round_results
                .fulfilled_goals
                .iter()
                .any(|goal| goal == "SCRG"),
            "Tutorial07 records its fulfilled SCRG goal"
        );
        assert_eq!(
            app.engine.next_mission().path,
            r"Tutorial.c4f\Tutorial08.c4s"
        );
        assert!(
            app.engine.object_snapshot(crystal).is_none(),
            "Tutorial07 CRYS is sold before SCRG fulfillment"
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
            VirtualKeyCode::Z,
            VirtualKeyCode::C,
            VirtualKeyCode::S,
            VirtualKeyCode::X,
        ] {
            AppVirtualKeyboard::new(app)
                .release(key)
                .unwrap_or_else(|error| {
                    panic!("clear physical {key:?} before WIPF {delivery}: {error}")
                });
        }
        for _ in 0..12 {
            app.update().expect("wait out WIPF sweep key buffer");
        }

        let mut search_key = if app
            .engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.position.x < 400)
        {
            VirtualKeyCode::C
        } else {
            VirtualKeyCode::Z
        };
        AppVirtualKeyboard::new(app)
            .press(search_key)
            .unwrap_or_else(|error| {
                panic!("physical direction starts WIPF {delivery} surface sweep: {error}")
            });
        for _ in 0..4_000 {
            app.update().expect("advance Tutorial08 WIPF surface sweep");
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
                AppVirtualKeyboard::new(app)
                    .release(search_key)
                    .expect("release WIPF sweep before clearing incidental material");
                let behind = if search_key == VirtualKeyCode::C {
                    VirtualKeyCode::Z
                } else {
                    VirtualKeyCode::C
                };
                AppVirtualKeyboard::new(app)
                    .tap(behind)
                    .expect("physical direction faces away from the WIPF sweep");
                AppVirtualKeyboard::new(app)
                    .tap(VirtualKeyCode::A)
                    .expect("physical A throws incidental WIPF-sweep material");
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
                    app.update().expect("clear WIPF-sweep throw key buffer");
                }
                AppVirtualKeyboard::new(app)
                    .press(search_key)
                    .expect("resume WIPF sweep after incidental material");
            }
            let x = app
                .engine
                .object_snapshot(clonk)
                .expect("surface-searching CLNK remains observable")
                .position
                .x;
            let next_key = if x <= 8 {
                VirtualKeyCode::C
            } else if x >= 790 {
                VirtualKeyCode::Z
            } else {
                search_key
            };
            if next_key != search_key {
                AppVirtualKeyboard::new(app)
                    .release(search_key)
                    .expect("release physical direction at surface boundary");
                for _ in 0..12 {
                    app.update().expect("wait out WIPF sweep turn buffer");
                }
                search_key = next_key;
                AppVirtualKeyboard::new(app)
                    .press(search_key)
                    .expect("reverse physical direction for WIPF sweep");
            }
        }
        AppVirtualKeyboard::new(app)
            .release(search_key)
            .expect("release physical direction after catching WIPF");
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
            app.update().expect("wait out WIPF search key buffer");
        }
        let toward_lorry = app
            .engine
            .object_snapshot(clonk)
            .zip(app.engine.object_snapshot(lorry))
            .map(|(clonk, lorry)| {
                if clonk.position.x < lorry.position.x {
                    VirtualKeyCode::C
                } else {
                    VirtualKeyCode::Z
                }
            })
            .expect("CLNK and LORY survive the WIPF search");
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
            keyboard
                .tap(VirtualKeyCode::X)
                .expect("first physical X primes LORY grab");
            keyboard
                .tap(VirtualKeyCode::X)
                .expect("second physical X grabs LORY");
        }
        advance_app_until(app, "WIPF-carrying CLNK grabs LORY", 80, |app| {
            app.engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Push" && object.action.target == Some(lorry)
            })
        });
        AppVirtualKeyboard::new(app)
            .tap(VirtualKeyCode::A)
            .unwrap_or_else(|error| panic!("physical A loads WIPF {delivery} into LORY: {error}"));
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
            keyboard
                .tap(VirtualKeyCode::X)
                .expect("first physical X primes LORY release");
            keyboard
                .tap(VirtualKeyCode::X)
                .expect("second physical X releases LORY");
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
        let clonk = app
            .engine
            .crew_cursor(owner)
            .expect("Tutorial08 starts with one cursor CLNK");
        let lorry =
            app_object_with_definition(&app, "LORY").expect("Tutorial08 creates LORY beside HUT3");
        let hut =
            app_object_with_definition(&app, "HUT3").expect("Tutorial08 creates the player's HUT3");
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
                object.definition_id == "WIPF"
                    && object.container.is_none()
                    && object.position.y > 450
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
            VirtualKeyCode::C
        } else {
            VirtualKeyCode::Z
        };
        AppVirtualKeyboard::new(&mut app)
            .press(search_key)
            .expect("physical direction starts the WIPF surface sweep");
        for _ in 0..4_000 {
            app.update().expect("advance Tutorial08 WIPF surface sweep");
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
                AppVirtualKeyboard::new(&mut app)
                    .release(search_key)
                    .expect("release surface sweep before throwing incidental material");
                let behind = if search_key == VirtualKeyCode::C {
                    VirtualKeyCode::Z
                } else {
                    VirtualKeyCode::C
                };
                AppVirtualKeyboard::new(&mut app)
                    .tap(behind)
                    .expect("physical direction faces away from the WIPF sweep");
                AppVirtualKeyboard::new(&mut app)
                    .tap(VirtualKeyCode::A)
                    .expect("physical A throws incidental surface material");
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
                    app.update().expect("clear incidental throw key buffer");
                }
                AppVirtualKeyboard::new(&mut app)
                    .press(search_key)
                    .expect("resume WIPF surface sweep after incidental material");
            }
            let x = app
                .engine
                .object_snapshot(clonk)
                .expect("surface-searching CLNK remains observable")
                .position
                .x;
            let next_key = if x <= 8 {
                VirtualKeyCode::C
            } else if x >= 790 {
                VirtualKeyCode::Z
            } else {
                search_key
            };
            if next_key != search_key {
                AppVirtualKeyboard::new(&mut app)
                    .release(search_key)
                    .expect("release physical direction at surface boundary");
                for _ in 0..12 {
                    app.update().expect("wait out WIPF sweep turn buffer");
                }
                search_key = next_key;
                AppVirtualKeyboard::new(&mut app)
                    .press(search_key)
                    .expect("reverse physical direction for WIPF sweep");
            }
        }
        AppVirtualKeyboard::new(&mut app)
            .release(search_key)
            .expect("release physical direction after catching WIPF");
        let caught_wipf = app
            .engine
            .object_snapshot(clonk)
            .and_then(|object| object.contents.first().copied())
            .expect("physical surface sweep catches one WIPF");
        assert_eq!(
            app.engine
                .object_snapshot(caught_wipf)
                .expect("caught Tutorial08 object survives")
                .definition_id,
            "WIPF",
            "first caught surface object is a WIPF"
        );

        for key in [
            VirtualKeyCode::Z,
            VirtualKeyCode::C,
            VirtualKeyCode::S,
            VirtualKeyCode::X,
        ] {
            AppVirtualKeyboard::new(&mut app)
                .release(key)
                .unwrap_or_else(|error| panic!("clear physical {key:?} before LORY: {error}"));
        }
        for _ in 0..12 {
            app.update().expect("wait out WIPF search key buffer");
        }
        let toward_lorry = app
            .engine
            .object_snapshot(clonk)
            .zip(app.engine.object_snapshot(lorry))
            .map(|(clonk, lorry)| {
                if clonk.position.x < lorry.position.x {
                    VirtualKeyCode::C
                } else {
                    VirtualKeyCode::Z
                }
            })
            .expect("CLNK and LORY survive the first WIPF search");
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
            keyboard
                .tap(VirtualKeyCode::X)
                .expect("first physical X primes LORY grab");
            keyboard
                .tap(VirtualKeyCode::X)
                .expect("second physical X grabs LORY");
        }
        advance_app_until(&mut app, "WIPF-carrying CLNK grabs LORY", 80, |app| {
            app.engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Push" && object.action.target == Some(lorry)
            })
        });
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::A)
            .expect("physical A loads carried WIPF into LORY");
        advance_app_until(&mut app, "first caught WIPF enters LORY", 60, |app| {
            app.engine
                .object_snapshot(caught_wipf)
                .is_some_and(|object| object.container == Some(lorry))
        });
        {
            let mut keyboard = AppVirtualKeyboard::new(&mut app);
            keyboard
                .tap(VirtualKeyCode::X)
                .expect("first physical X primes LORY release");
            keyboard
                .tap(VirtualKeyCode::X)
                .expect("second physical X releases LORY");
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
                .filter(|object| {
                    object.definition_id == "WIPF" && object.container == Some(lorry)
                })
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
                object.definition_id == "WIPF"
                    && object.container.is_none()
                    && object.position.y > 450
            })
            .count();
        assert_eq!(
            subterranean_wipfs, 4,
            "C++ seed leaves four live WIPFs in underground caves"
        );
        for _ in 0..12 {
            app.update().expect("wait out sixth LORY release buffer");
        }
        let toward_lorry = app
            .engine
            .object_snapshot(clonk)
            .zip(app.engine.object_snapshot(lorry))
            .map(|(clonk, lorry)| {
                if clonk.position.x < lorry.position.x {
                    VirtualKeyCode::C
                } else {
                    VirtualKeyCode::Z
                }
            })
            .expect("CLNK and six-WIPF LORY remain observable");
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
            keyboard
                .tap(VirtualKeyCode::X)
                .expect("first physical X primes loaded LORY grab");
            keyboard
                .tap(VirtualKeyCode::X)
                .expect("second physical X grabs loaded LORY");
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
                    VirtualKeyCode::C
                } else {
                    VirtualKeyCode::Z
                }
            })
            .expect("loaded LORY and HUT3 remain observable");
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
        AppVirtualKeyboard::new(&mut app)
            .tap(VirtualKeyCode::S)
            .expect("physical S enters six-WIPF LORY into HUT3");
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
            VirtualKeyCode::C,
            "physical C collects Tutorial09 CNKT",
            30,
            |app| app_clonk_carries(app, clonk, "CNKT"),
        );
    }

    #[test]
    fn real_tutorial01_saved_game_music_subcases_batch() {
        let prepared =
            PreparedRealInstalledScenario::new("Tutorial.c4f/Tutorial01.c4s");
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
        let paths = cached_app_paths().expect("test app paths");
        paths.ensure_user_dirs().expect("test config directory");
        fs::write(paths.config_file(), "[General]\nLanguageEx=US\n")
            .expect("configure restore language");
        let scenario = app
            .active_scenario
            .clone()
            .expect("real tutorial remains active");
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
        let save: SavedGameFile = serde_json::from_str(
            &serde_json::to_string(&save).expect("serialize tutorial save"),
        )
        .expect("deserialize tutorial save");
        app.audio
            .as_mut()
            .expect("test audio")
            .set_scenario_music_level(Some(73));

        app.apply_loaded_game(save).expect("restore tutorial save");

        assert_eq!(app.engine.capture_state().music_level, 25);
        let audio = app.audio.as_ref().expect("test audio");
        let control = lock_unpoisoned(&audio.music_control);
        assert_eq!(control.scenario_level, Some(25));
        assert!(
            (control.effective_volume() - audio.options.music_volume * 0.25).abs()
                < f32::EPSILON
        );
    }

    fn saved_game_resume_uses_default_playlist_but_preserves_saved_filter(
        prepared: &PreparedRealInstalledScenario,
    ) {
        let mut app = prepared.instantiate("Music resume ordering parity", false);
        let paths = cached_app_paths().expect("test app paths");
        paths.ensure_user_dirs().expect("test config directory");
        fs::write(paths.config_file(), "[General]\nLanguageEx=US\n")
            .expect("configure restore language");
        let scenario = app
            .active_scenario
            .clone()
            .expect("real tutorial remains active");
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
        app.audio
            .as_mut()
            .expect("test audio")
            .options
            .music_enabled = true;

        app.apply_loaded_game(save).expect("restore tutorial save");

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

        app.snapshot = app.engine.tick().expect("first restored tick succeeds");
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
