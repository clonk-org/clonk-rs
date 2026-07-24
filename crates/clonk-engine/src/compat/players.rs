use super::*;

fn parse_player_type_filter(value: Option<&Value>, function: &str) -> Result<i32, RuntimeError> {
    match value {
        Some(Value::Int(filter)) => Ok(*filter),
        Some(Value::Nil) | None => Ok(0),
        Some(other) => Err(RuntimeError::new(format!(
            "{}: expected int or nil for type filter, got {}",
            function,
            other.type_name()
        ))),
    }
}

fn player_type(player: &PlayerState) -> i32 {
    i32::from(if player.script_player {
        crate::PLAYER_INFO_TYPE_SCRIPT
    } else {
        crate::PLAYER_INFO_TYPE_USER
    })
}

fn player_type_matches(player: &PlayerState, filter: i32) -> bool {
    filter == 0 || filter == player_type(player)
}

pub(crate) fn script_player_extra_data(value: Option<&Value>) -> Result<[u8; 4], RuntimeError> {
    let Some(id) = parse_native_c4id_argument(value, "CreateScriptPlayer")? else {
        return Ok(*b"NONE");
    };
    let text = clonk_script::c4_id_text(&id);
    let bytes = clonk_script::c4_string_bytes(&text);
    if bytes.len() < 4 {
        return Ok(*b"NONE");
    }
    let mut extra = [0; 4];
    extra.copy_from_slice(&bytes[..4]);
    Ok(extra)
}

/// FnCreateScriptPlayer (C4Script.cpp:2877-2903): validate the name on
/// every peer, but only the control host emits the additive PlayerInfo
/// request. The actual join remains delayed in the control pipeline.
pub(crate) fn create_script_player(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 5 {
        return Err(RuntimeError::new(
            "CreateScriptPlayer expects at most 5 arguments: name, color, team, flags and extra data",
        ));
    }
    let name = match args.first().unwrap_or(&Value::Nil) {
        Value::String(name) if !name.is_empty() => Some(name),
        Value::String(_) | Value::Nil => None,
        other => {
            return Err(RuntimeError::new(format!(
                "CreateScriptPlayer: expected string for name, got {}",
                other.type_name()
            )));
        }
    };
    let color = value_to_i32(
        args.get(1).unwrap_or(&Value::Nil),
        "CreateScriptPlayer",
        "color",
    )? as u32
        & 0x00ff_ffff;
    let team = value_to_i32(
        args.get(2).unwrap_or(&Value::Nil),
        "CreateScriptPlayer",
        "team",
    )?;
    let source_flags = value_to_i32(
        args.get(3).unwrap_or(&Value::Nil),
        "CreateScriptPlayer",
        "flags",
    )?;
    let extra_data = script_player_extra_data(args.get(4))?;
    let Some(name) = name else {
        return Ok(Value::Bool(false));
    };

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Bool(true));
        };
        if !context.world.control_host {
            return Ok(Value::Bool(true));
        }
        let mut flags = 0;
        if source_flags & 1 != 0 {
            flags |= crate::PLAYER_INFO_FLAG_ATTRIBUTES_FIXED;
        }
        if source_flags & 2 != 0 {
            flags |= crate::PLAYER_INFO_FLAG_NO_SCENARIO_INIT;
        }
        if source_flags & 4 != 0 {
            flags |= crate::PLAYER_INFO_FLAG_NO_ELIMINATION_CHECK;
        }
        if source_flags & 8 != 0 {
            flags |= crate::PLAYER_INFO_FLAG_INVISIBLE;
        }
        let name = crate::LegacyCString::from_bytes(clonk_script::c4_string_bytes(name))
            .ok_or_else(|| {
                RuntimeError::new("CreateScriptPlayer: name contains an interior NUL")
            })?;
        context
            .world
            .player_info_updates
            .borrow_mut()
            .push(crate::PlayerInfoUpdateRequest {
                client_id: 0,
                flags: crate::CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                players: vec![crate::ControlPlayerInfoEntry {
                    name,
                    flags,
                    player_type: crate::PLAYER_INFO_TYPE_SCRIPT,
                    color,
                    original_color: color,
                    team,
                    extra_data,
                    ..Default::default()
                }],
            });
        Ok(Value::Bool(true))
    })
}

pub(crate) fn get_player_count(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "GetPlayerCount expects at most 1 argument: type",
        ));
    }
    let filter = parse_player_type_filter(args.first(), "GetPlayerCount")?;
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Int(0));
        };
        let count = context
            .player_ids()
            .iter()
            .filter(|id| {
                context
                    .player_state(**id)
                    .map(|player| player_type_matches(player, filter))
                    .unwrap_or(false)
            })
            .count();
        Ok(Value::Int(truncate_to_i32(count as u64)))
    })
}

/// `FnGetMaxPlayer` (C4Script.cpp:3693-3696): return the exact live
/// `Game.Parameters.MaxPlayers` integer, including a successful setter
/// earlier in the same VM call.
pub(crate) fn get_max_player(_args: &[Value]) -> Result<Value, RuntimeError> {
    HOST_CONTEXT.with(|cell| {
        Ok(Value::Int(
            cell.borrow()
                .as_ref()
                .map_or(0, |context| context.world.max_players()),
        ))
    })
}

/// `FnSetMaxPlayer` (C4Script.cpp:3698-3706): nonnegative values directly
/// replace `Game.Parameters.MaxPlayers`; negative values fail without a
/// write. The C++ host function returns `C4ValueInt`, not a script bool.
pub(crate) fn set_max_player(args: &[Value]) -> Result<Value, RuntimeError> {
    let max_players = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "SetMaxPlayer",
        "maximum player count",
    )?;
    if max_players < 0 {
        return Ok(Value::Int(0));
    }

    HOST_CONTEXT.with(|cell| {
        if let Some(context) = cell.borrow_mut().as_mut() {
            context.world.set_max_players(max_players);
            context.record_player_command(PlayerCommand::SetMaxPlayer { max_players });
        }
    });
    Ok(Value::Int(1))
}

pub(crate) fn get_player_by_index(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 2 {
        return Err(RuntimeError::new(
            "GetPlayerByIndex expects at most 2 arguments: index and type",
        ));
    }
    // C++ pads missing script args with zero: GetPlayerByIndex() is the
    // first player (GoldRush Script1's intro camera).
    let index = match args.first() {
        None | Some(Value::Nil) => 0,
        Some(value) => value_to_i32(value, "GetPlayerByIndex", "index")?,
    };
    let filter = parse_player_type_filter(args.get(1), "GetPlayerByIndex")?;
    if index < 0 {
        return Ok(Value::Int(OWNER_NONE));
    }
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Int(OWNER_NONE));
        };
        // C4PlayerList::GetByIndex walks the live player list only until the
        // requested matching entry (C4PlayerList.cpp:139-153). Preserve that
        // early-exit behavior instead of materializing every match: Race's
        // per-frame scoreboard calls this once for every player.
        let matching = context
            .player_ids()
            .iter()
            .copied()
            .filter(|id| {
                context
                    .player_state(*id)
                    .is_some_and(|player| player_type_matches(player, filter))
            })
            .nth(index as usize)
            .unwrap_or(OWNER_NONE);
        Ok(Value::Int(matching))
    })
}

/// `FnInitScenarioPlayer` (C4Script.cpp:5827-5832): run
/// `C4Player::ScenarioAndTeamInit` for any live C4PlayerInfo-backed player.
/// The copied host context predicts its team-validation result; the ordered
/// player command runs the authoritative initialization path when this VM
/// call folds back into the engine.
pub(crate) fn init_scenario_player(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 2 {
        return Err(RuntimeError::new(
            "InitScenarioPlayer expects at most 2 arguments: player and team",
        ));
    }
    let player_id = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "InitScenarioPlayer",
        "player",
    )?;
    let requested_team = value_to_i32(
        args.get(1).unwrap_or(&Value::Nil),
        "InitScenarioPlayer",
        "team",
    )?;

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Ok(Value::Bool(false));
        };
        let Some((player_info_id, current_team)) = context
            .player_state(player_id)
            .map(|player| (player.player_info_id, player.team))
        else {
            return Ok(Value::Bool(false));
        };
        // C4Player::ScenarioAndTeamInit gates only on GetInfo(). A fully
        // initialized player may run it again; status is not consulted.
        if player_info_id == 0 {
            return Ok(Value::Bool(false));
        }

        let accepted = match requested_team {
            -1 => {
                context.world.auto_generate_teams()
                    && context
                        .teams()
                        .iter()
                        .map(|team| team.id)
                        .fold(0, i32::max)
                        .checked_add(1)
                        .is_some()
            }
            0 => true,
            team => {
                context.teams().iter().any(|candidate| candidate.id == team)
                    && (current_team == Some(team) || !context.team_is_full(team))
            }
        };

        // Rejected choices still run ScenarioAndTeamInit: its
        // OnTeamSelectionFailed arm changes Pending back to TeamSelection.
        context.record_player_command(PlayerCommand::InitScenarioPlayer {
            player_id,
            team: requested_team,
        });
        Ok(Value::Bool(accepted))
    })
}

/// FnEliminatePlayer (C4Script.cpp:2823-2842): missing players and already
/// eliminated regular players fail; the direct flag removes through the
/// player-control path, while regular elimination starts the retire delay.
pub(crate) fn eliminate_player(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 2 {
        return Err(RuntimeError::new(
            "EliminatePlayer expects at most 2 arguments: player and direct removal flag",
        ));
    }
    let player_id = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "EliminatePlayer",
        "player",
    )?;
    let remove_direct = value_to_bool(
        args.get(1).unwrap_or(&Value::Nil),
        "EliminatePlayer",
        "direct removal flag",
    )?;
    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Ok(Value::Int(0));
        };
        let Some(player) = context.player_state(player_id) else {
            return Ok(Value::Int(0));
        };
        if remove_direct {
            // Every peer reports success for a valid player, but only the
            // control host appends CID_RemovePlr to Game.Input
            // (C4Script.cpp:2823-2833).
            if context.world.control_host {
                context.record_player_command(PlayerCommand::Remove { player_id });
            }
            return Ok(Value::Int(1));
        }
        if matches!(
            player.status,
            crate::PlayerStatus::Eliminated | crate::PlayerStatus::Surrendered
        ) || player.surrendered
        {
            return Ok(Value::Int(0));
        }
        if let Some(player) = context.player_state_mut(player_id) {
            player.status = crate::PlayerStatus::Eliminated;
            player.surrendered = false;
        }
        context.record_player_command(PlayerCommand::Eliminate { player_id });
        Ok(Value::Int(1))
    })
}

/// `FnSurrenderPlayer` (C4Script.cpp:2843-2850): resolve any live player,
/// reject one whose eliminated flag is already set, then run the ordinary
/// surrender transition without the network control's client authorization.
pub(crate) fn surrender_player(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "SurrenderPlayer expects at most 1 argument: player",
        ));
    }
    let player_id = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "SurrenderPlayer",
        "player",
    )?;
    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Ok(Value::Bool(false));
        };
        let can_surrender = context.player_state(player_id).is_some_and(|player| {
            !matches!(
                player.status,
                crate::PlayerStatus::Eliminated | crate::PlayerStatus::Surrendered
            ) && !player.surrendered
        });
        if !can_surrender {
            return Ok(Value::Bool(false));
        }

        // C++ changes both flags synchronously. Mirror that in the copied
        // host state so another call in this same VM invocation sees the
        // player as eliminated before the deferred command is folded back.
        if let Some(player) = context.player_state_mut(player_id) {
            player.status = crate::PlayerStatus::Surrendered;
            player.surrendered = true;
        }
        context.record_player_command(PlayerCommand::Surrender { player_id });
        Ok(Value::Bool(true))
    })
}

pub(crate) fn get_player_name(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "GetPlayerName expects at most 1 argument: player",
        ));
    }
    // An unfilled iPlr slot is nil -> 0 (C4AulExec parameter filling).
    let player_id = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "GetPlayerName",
        "player",
    )?;
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        let Some(player) = context.player_state(player_id) else {
            return Ok(Value::Nil);
        };
        Ok(Value::String(player.name.clone().into()))
    })
}

/// `FnGetTaggedPlayerName` colors a valid player's name with their 24-bit
/// display color after `C4GUI::MakeColorReadableOnBlack` has raised dark
/// colors to the legacy lightness floor (C4Script.cpp:1084-1091;
/// C4Gui.cpp:71-87).
pub(crate) fn get_tagged_player_name(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "GetTaggedPlayerName expects at most 1 argument: player",
        ));
    }
    let player_id = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "GetTaggedPlayerName",
        "player",
    )?;
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        let Some(player) = context.player_state(player_id) else {
            return Ok(Value::Nil);
        };
        let mut color = player
            .color
            .map(|color| {
                (u32::from(color.r) << 16) | (u32::from(color.g) << 8) | u32::from(color.b)
            })
            .unwrap_or(0);
        let red = (color >> 16) & 0xff;
        let green = (color >> 8) & 0xff;
        let blue = color & 0xff;
        let lightness = red * 50 + green * 87 + blue * 27;
        if lightness < 16_575 {
            let increment = (16_575 - lightness) / 164;
            color = ((red + increment).min(255) << 16)
                | ((green + increment).min(255) << 8)
                | (blue + increment).min(255);
        }
        Ok(Value::String(
            format!("<c {color:x}>{}</c>", player.name).into(),
        ))
    })
}

/// FnGetPlayerVal reflects the named C4Player fields serialized by
/// C4Player::CompileFunc (C4Script.cpp:4252-4263). The standard
/// GetPlrViewX/GetPlrViewY wrappers use the ViewX/ViewY entries
/// (planet/System.c4g/GetXVal.c:99-100; C4Player.cpp:1576-1577).
pub(crate) fn get_player_val(args: &[Value]) -> Result<Value, RuntimeError> {
    let Some(entry) = parse_optional_string(args.first(), "GetPlayerVal", "entry")? else {
        return Ok(Value::Nil);
    };
    let section = parse_optional_string(args.get(1), "GetPlayerVal", "section")?;
    let player_id = value_to_i32(args.get(2).unwrap_or(&Value::Nil), "GetPlayerVal", "player")?;
    let entry_index = value_to_i32(
        args.get(3).unwrap_or(&Value::Nil),
        "GetPlayerVal",
        "entry_nr",
    )?;
    if entry_index < 0 || !matches!(section.as_deref(), None | Some("") | Some("Player")) {
        return Ok(Value::Nil);
    }

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        let Some(player) = context.player_state(player_id) else {
            return Ok(Value::Nil);
        };
        let index = entry_index as usize;
        let indexed_id_list = |entries: Vec<(DefinitionId, i32)>| {
            entries
                .get(index / 2)
                .map(|(id, count)| {
                    if index % 2 == 0 {
                        Value::C4Id(id.clone())
                    } else {
                        Value::Int(*count)
                    }
                })
                .unwrap_or(Value::Nil)
        };
        let indexed_value = match entry.as_str() {
            "Hostile" => player
                .exact_hostility_entries()
                .get(index / 2)
                .map(|(raw_id, count)| {
                    if index % 2 == 0 {
                        Value::C4Id(render_cast_c4id(*raw_id))
                    } else {
                        Value::Int(*count)
                    }
                })
                .unwrap_or(Value::Nil),
            "HomeBaseMaterial" => indexed_id_list(player.exact_home_base_material_entries()),
            "HomeBaseProduction" => indexed_id_list(player.exact_home_base_production_entries()),
            "Knowledge" => indexed_id_list(player.exact_knowledge_entries()),
            "Magic" => indexed_id_list(player.exact_magic_entries()),
            "Crew" => player
                .crew
                .iter()
                .filter(|object_id| {
                    context
                        .get_world_object(**object_id)
                        .is_some_and(|object| object.status != ObjectStatus::Deleted)
                })
                .nth(index)
                .map(|object_id| Value::Int(truncate_to_i32(object_id.as_u64())))
                .unwrap_or(Value::Nil),
            _ => Value::Nil,
        };
        if !matches!(&indexed_value, Value::Nil)
            || matches!(
                entry.as_str(),
                "Hostile"
                    | "HomeBaseMaterial"
                    | "HomeBaseProduction"
                    | "Knowledge"
                    | "Magic"
                    | "Crew"
            )
        {
            return Ok(indexed_value);
        }
        if entry_index != 0 {
            return Ok(Value::Nil);
        }
        let view_center = player
            .view_center
            .or_else(|| player.viewports.first().map(|viewport| viewport.center))
            .or_else(|| {
                player
                    .cursor
                    .and_then(|cursor| context.get_world_object(cursor))
                    .map(|cursor| cursor.position())
            })
            .unwrap_or(Vector2::ZERO);
        let player_status = match player.status {
            crate::PlayerStatus::Inactive => 0,
            crate::PlayerStatus::TeamSelection => 2,
            crate::PlayerStatus::TeamSelectionPending => 3,
            crate::PlayerStatus::Active
            | crate::PlayerStatus::Eliminated
            | crate::PlayerStatus::Surrendered => 1,
        };
        let object_number =
            |object: Option<ObjectId>| object.map(|id| truncate_to_i32(id.as_u64())).unwrap_or(0);
        Ok(match entry.as_str() {
            "Status" => Value::Int(player_status),
            "AtClient" => Value::Int(player.at_client.get()),
            "AtClientName" => Value::String(
                player
                    .at_client_name
                    .clone()
                    .unwrap_or_else(|| "Local".to_string())
                    .into(),
            ),
            "Index" => Value::Int(player.id),
            "ID" => Value::Int(player.player_info_id),
            "Eliminated" => Value::Int(i32::from(
                matches!(
                    player.status,
                    crate::PlayerStatus::Eliminated | crate::PlayerStatus::Surrendered
                ) || player.surrendered,
            )),
            "Surrendered" => Value::Int(i32::from(
                player.surrendered || matches!(player.status, crate::PlayerStatus::Surrendered),
            )),
            "Evaluated" => Value::Bool(player.evaluated),
            "Color" => Value::Int(player.color_index.unwrap_or(-1)),
            "ColorDw" => Value::Int(player.exact_color_dw() as i32),
            "Control" => Value::Int(player.control_set),
            "MouseControl" => Value::Int(player.mouse_control),
            "AutoContextMenu" => Value::Int(i32::from(player.control.auto_context_menu)),
            "AutoStopControl" => Value::Int(i32::from(player.control.control_style)),
            "Position" => Value::Int(player.position_index.unwrap_or(-1)),
            "ViewMode" => Value::Int(player.view_mode),
            "ViewX" => Value::Int(view_center.x),
            "ViewY" => Value::Int(view_center.y),
            "ViewWealth" => Value::Int(player.view_wealth),
            "ViewValue" => Value::Int(player.view_value),
            "FogOfWar" => Value::Bool(player.fog_of_war),
            "ForceFogOfWar" => Value::Bool(player.force_fog_of_war),
            "ShowStartup" => Value::Bool(player.show_startup),
            "ShowControl" => Value::Int(player.show_control),
            "ShowControlPos" => Value::Int(player.show_control_position),
            "Wealth" => Value::Int(player.wealth),
            "Points" => Value::Int(player.points),
            "Value" => Value::Int(player.value),
            "InitialValue" => Value::Int(player.initial_value),
            "ValueGain" => Value::Int(player.value_gain),
            "ObjectsOwned" => Value::Int(player.objects_owned as i32),
            "ProductionDelay" => Value::Int(player.production_delay as i32),
            "ProductionUnit" => Value::Int(player.production_unit as i32),
            "SelectCount" => Value::Int(player.select_count),
            "SelectFlash" => Value::Int(player.control.select_flash),
            "CursorFlash" => Value::Int(player.control.cursor_flash),
            "Cursor" => Value::Int(object_number(player.cursor)),
            "ViewCursor" => Value::Int(object_number(player.view_cursor)),
            "Captain" => Value::Int(object_number(player.captain)),
            "LastCom" => Value::Int(player.control.last_com),
            "LastComDel" => Value::Int(player.control.last_com_delay),
            "PressedComs" => Value::Int(player.control.pressed_coms),
            "LastComDownDouble" => Value::Int(player.control.last_com_down_double),
            "CursorSelection" => Value::Int(player.control.cursor_selection),
            "CursorToggled" => Value::Int(player.control.cursor_toggled),
            "MessageStatus" => Value::Int(player.message_status),
            "MessageBuf" => Value::String(player.message_buf.clone().into()),
            "CrewCreated" => Value::Int(player.crew_created),
            _ => Value::Nil,
        })
    })
}

/// FnGetPlayerInfoCoreVal (C4Script.cpp:4266-4280): reflect a saved
/// C4PlayerInfoCore field. Only the control-style preference is represented
/// in PlayerState today; it is the path used by Hazard's weapon recharge.
pub(crate) fn get_player_info_core_val(args: &[Value]) -> Result<Value, RuntimeError> {
    let Some(entry) = parse_optional_string(args.first(), "GetPlayerInfoCoreVal", "entry")? else {
        return Ok(Value::Nil);
    };
    let section = parse_optional_string(args.get(1), "GetPlayerInfoCoreVal", "section")?
        .filter(|section| !section.is_empty());
    let player_id = value_to_i32(
        args.get(2).unwrap_or(&Value::Nil),
        "GetPlayerInfoCoreVal",
        "player",
    )?;
    let entry_index = value_to_i32(
        args.get(3).unwrap_or(&Value::Nil),
        "GetPlayerInfoCoreVal",
        "entry_nr",
    )?;
    if entry_index != 0
        || entry != "AutoStopControl"
        || !matches!(section.as_deref(), None | Some("Preferences"))
    {
        return Ok(Value::Nil);
    }

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        Ok(borrow
            .as_ref()
            .and_then(|context| context.player_state(player_id))
            .map(|player| Value::Int(i32::from(player.control.control_style)))
            .unwrap_or(Value::Nil))
    })
}

pub(crate) fn get_player_id(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "GetPlayerID expects at most 1 argument: player",
        ));
    }
    // An unfilled iPlr slot is nil -> 0 (C4AulExec parameter filling).
    let player_id = value_to_i32(args.first().unwrap_or(&Value::Nil), "GetPlayerID", "player")?;
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        Ok(context
            .player_state(player_id)
            .map_or(Value::Nil, |player| Value::Int(player.player_info_id)))
    })
}

pub(crate) fn get_player_team(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "GetPlayerTeam expects at most 1 argument: player",
        ));
    }
    // An unfilled iPlr slot is nil -> 0 (C4AulExec parameter filling).
    let player_id = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "GetPlayerTeam",
        "player",
    )?;
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        let Some(player) = context.player_state(player_id) else {
            return Ok(Value::Nil);
        };
        let team = player.team.unwrap_or_else(|| {
            if matches!(
                player.status,
                crate::PlayerStatus::TeamSelection | crate::PlayerStatus::TeamSelectionPending
            ) {
                -1
            } else {
                0
            }
        });
        Ok(Value::Int(team))
    })
}

/// FnSetPlayerTeam (C4Script.cpp:5730-5783): validate before callbacks,
/// broadcast the rejection hook before any mutation, then switch membership,
/// synchronize team-owned state, and finally broadcast OnTeamSwitch.
pub(crate) fn set_player_team(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 3 {
        return Err(RuntimeError::new(
            "SetPlayerTeam expects at most 3 arguments: player, team, no-calls",
        ));
    }
    let player_id = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "SetPlayerTeam",
        "player",
    )?;
    let requested_team = value_to_i32(args.get(1).unwrap_or(&Value::Nil), "SetPlayerTeam", "team")?;
    let no_calls = value_to_bool(
        args.get(2).unwrap_or(&Value::Nil),
        "SetPlayerTeam",
        "no-calls",
    )?;

    enum Preflight {
        Reject,
        AlreadyThere,
        Continue,
    }
    let preflight = HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Preflight::Reject;
        };
        // League refusal precedes even the same-team fast path.
        if context.world.league_game() {
            return Preflight::Reject;
        }
        let Some(player) = context.player_state(player_id) else {
            return Preflight::Reject;
        };
        if player.team.unwrap_or(0) == requested_team {
            return Preflight::AlreadyThere;
        }

        let join_allowed = if requested_team == -1 {
            context.world.auto_generate_teams()
        } else {
            context.teams().iter().any(|team| team.id == requested_team)
                && !context.team_is_full(requested_team)
        };
        if join_allowed {
            Preflight::Continue
        } else {
            Preflight::Reject
        }
    });
    match preflight {
        Preflight::Reject => return Ok(Value::Bool(false)),
        Preflight::AlreadyThere => return Ok(Value::Bool(true)),
        Preflight::Continue => {}
    }

    let reject_args = [Value::Int(player_id), Value::Int(requested_team)];
    if !no_calls
        && value_raw_truthy(&broadcast_global_callback(
            "RejectTeamSwitch",
            &reject_args,
            true,
        )?)
    {
        return Ok(Value::Bool(false));
    }

    let switched = HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow.as_mut()?;
        // RejectTeamSwitch may itself have changed the player. C++ searches
        // the old roster only after that callback, so capture the live value
        // here rather than the preflight snapshot.
        let old_team = context
            .player_state(player_id)
            .and_then(|player| player.team)
            .unwrap_or(0);

        let (selected_team, generated_team, generated_color) = match requested_team {
            -1 => {
                let (team, color) = context.generate_runtime_team();
                (team.clone(), team, color)
            }
            0 => (None, None, None),
            id => (
                context.teams().iter().find(|team| team.id == id).cloned(),
                None,
                None,
            ),
        };
        let team = selected_team.as_ref().map(|team| team.id);

        // AddPlayer appends the switcher, so any existing member remains the
        // home-base captain. Excluding the switcher also handles a nested
        // RejectTeamSwitch callback that already moved this same player.
        let home_base_material_entries = if !no_calls && context.team_home_base_rule() {
            team.and_then(|team| {
                let has_player_info_order = selected_team
                    .as_ref()
                    .is_some_and(|selected| selected.id == team && !selected.player_ids.is_empty());
                let captain = selected_team
                    .as_ref()
                    .filter(|selected| selected.id == team)
                    .and_then(|selected| selected.player_ids.first())
                    .and_then(|captain_info_id| {
                        context.player_ids().iter().copied().find(|member| {
                            *member != player_id
                                && context.player_state(*member).is_some_and(|player| {
                                    player.team == Some(team)
                                        && player.player_info_id == *captain_info_id
                                })
                        })
                    })
                    .or_else(|| {
                        // Legacy/synthetic team fixtures may not carry
                        // C4PlayerInfo IDs. Preserve their deterministic
                        // runtime-number fallback.
                        (!has_player_info_order).then(|| {
                            context.player_ids().iter().copied().find(|member| {
                                *member != player_id
                                    && context
                                        .player_state(*member)
                                        .is_some_and(|player| player.team == Some(team))
                            })
                        })?
                    });
                captain.and_then(|member| {
                    context
                        .player_state(member)
                        .map(PlayerState::exact_home_base_material_entries)
                })
            })
        } else {
            None
        };

        if let Some(player) = context.player_state_mut(player_id) {
            player.team = team;
        }

        let color = if context.world.team_colors() {
            selected_team.as_ref().and_then(|team| {
                if generated_team.is_some() {
                    generated_color
                } else {
                    Some(team.color)
                }
            })
        } else {
            None
        };
        if let Some(color) = color {
            context.set_player_color_preview(player_id, color);
        }
        if let Some(material) = home_base_material_entries.as_ref() {
            if let Some(player) = context.player_state_mut(player_id) {
                player.set_home_base_material_entries(material.clone());
            }
        }

        let synchronize_hostility = !no_calls && team.is_some();
        if let Some(team) = team.filter(|_| synchronize_hostility) {
            context.synchronize_team_hostility(player_id, team);
        }

        context.record_player_command(PlayerCommand::SetPlayerTeam {
            player_id,
            team,
            generated_team,
            color,
            home_base_material_entries,
            synchronize_hostility,
        });
        Some((team.unwrap_or(0), old_team))
    });
    let Some((new_team, old_team)) = switched else {
        return Ok(Value::Bool(false));
    };

    if !no_calls {
        let changed_args = [
            Value::Int(player_id),
            Value::Int(new_team),
            Value::Int(old_team),
        ];
        broadcast_global_callback("OnTeamSwitch", &changed_args, false)?;
    }
    Ok(Value::Bool(true))
}

pub(crate) fn get_team_count(_args: &[Value]) -> Result<Value, RuntimeError> {
    HOST_CONTEXT.with(|cell| {
        Ok(Value::Int(
            cell.borrow()
                .as_ref()
                .map(|context| truncate_to_i32(context.teams().len() as u64))
                .unwrap_or(0),
        ))
    })
}

/// FnGetTeamConfig (C4Script.cpp:5785-5801): expose the live C4TeamList
/// settings as integers. Boolean fields are C4ValueInt 0/1, not script bools.
pub(crate) fn get_team_config(args: &[Value]) -> Result<Value, RuntimeError> {
    let query = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "GetTeamConfig",
        "config value",
    )?;
    if !(1..=7).contains(&query) {
        error!(target: "clonk-script", "GetTeamConfig: Unknown config value: {query}");
        return Ok(Value::Nil);
    }
    Ok(HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.world.team_config_value(query))
            .map(Value::Int)
            .unwrap_or(Value::Nil)
    }))
}

pub(crate) fn get_team_by_index(args: &[Value]) -> Result<Value, RuntimeError> {
    let index = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "GetTeamByIndex",
        "index",
    )?;
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        Ok(usize::try_from(index)
            .ok()
            .and_then(|index| context.teams().get(index))
            .map_or(Value::Nil, |team| Value::Int(team.id)))
    })
}

pub(crate) fn get_team_color(args: &[Value]) -> Result<Value, RuntimeError> {
    let id = value_to_i32(args.first().unwrap_or(&Value::Nil), "GetTeamColor", "team")?;
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        Ok(context
            .teams()
            .iter()
            .find(|team| team.id == id)
            .map_or(Value::Nil, |team| Value::Int(team.color as i32)))
    })
}

pub(crate) fn get_team_name(args: &[Value]) -> Result<Value, RuntimeError> {
    let id = value_to_i32(args.first().unwrap_or(&Value::Nil), "GetTeamName", "team")?;
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        Ok(context
            .teams()
            .iter()
            .find(|team| team.id == id)
            .map_or(Value::Nil, |team| Value::String(team.name.clone().into())))
    })
}

pub(crate) fn get_player_type(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "GetPlayerType expects at most 1 argument: player",
        ));
    }
    // An unfilled iPlr slot is nil -> 0 (C4AulExec parameter filling).
    let player_id = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "GetPlayerType",
        "player",
    )?;
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        Ok(context
            .player_state(player_id)
            .map_or(Value::Nil, |player| Value::Int(player_type(player))))
    })
}

pub(crate) fn get_wealth(args: &[Value]) -> Result<Value, RuntimeError> {
    // An unfilled iPlr slot is nil -> 0 (C4AulExec parameter filling).
    let player_id = value_to_i32(args.first().unwrap_or(&Value::Nil), "GetWealth", "player")?;
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        let Some(player) = context.player_state(player_id) else {
            return Ok(Value::Nil);
        };
        Ok(Value::Int(player.wealth))
    })
}

/// `FnSetWealth` (C4Script.cpp:2761-2766): clamp-set to `0..=100000`,
/// false for invalid players.
pub(crate) fn set_wealth(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() || args.len() > 2 {
        return Err(RuntimeError::new(
            "SetWealth expects 2 arguments: player, value",
        ));
    }
    let player_id = value_to_i32(&args[0], "SetWealth", "player")?;
    let value = match args.get(1) {
        Some(Value::Int(value)) => *value,
        Some(Value::Nil) | None => 0,
        Some(other) => {
            return Err(RuntimeError::new(format!(
                "SetWealth: expected int for value, got {}",
                other.type_name()
            )));
        }
    };

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Ok(Value::Bool(false));
        };
        let Some(player) = context.player_state_mut(player_id) else {
            return Ok(Value::Bool(false));
        };
        let clamped = value.clamp(0, 100_000);
        player.wealth = clamped;
        context.record_player_command(PlayerCommand::SetWealth {
            player_id,
            value: clamped,
            show_change: false,
        });
        Ok(Value::Bool(true))
    })
}

pub(crate) fn set_player_hostility_declaration(
    player: &mut PlayerState,
    opponent: i32,
    hostile: bool,
) {
    player.set_hostility_entry(opponent, hostile);
}

/// FnHostile (C4Script.cpp:2511-2519): the ordinary form is symmetric — one
/// player's declaration makes both directions hostile — while the third
/// argument requests the directed declaration only.
pub(crate) fn hostile(args: &[Value]) -> Result<Value, RuntimeError> {
    let player = value_to_i32(args.first().unwrap_or(&Value::Nil), "Hostile", "player")?;
    let opponent = value_to_i32(args.get(1).unwrap_or(&Value::Nil), "Hostile", "opponent")?;
    let one_way = value_to_bool(
        args.get(2).unwrap_or(&Value::Nil),
        "Hostile",
        "one-way flag",
    )?;

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Bool(false));
        };
        let (Some(player_state), Some(opponent_state)) =
            (context.player_state(player), context.player_state(opponent))
        else {
            return Ok(Value::Bool(false));
        };
        if player == opponent {
            return Ok(Value::Bool(false));
        }
        let declared = player_state.is_hostile_towards(opponent);
        let hostile = declared || (!one_way && opponent_state.is_hostile_towards(player));
        Ok(Value::Bool(hostile))
    })
}

/// FnSetHostility (C4Script.cpp:2521-2537): reject callbacks run before
/// validation of the opponent, the declaration becomes visible immediately,
/// and OnHostilityChange runs afterward even when `no_calls` skipped rejection.
pub(crate) fn set_hostility(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 5 {
        return Err(RuntimeError::new(
            "SetHostility expects at most 5 arguments: player, opponent, hostile, silent, no-calls",
        ));
    }
    let player = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "SetHostility",
        "player",
    )?;
    let opponent = value_to_i32(
        args.get(1).unwrap_or(&Value::Nil),
        "SetHostility",
        "opponent",
    )?;
    let new_hostility = value_to_bool(
        args.get(2).unwrap_or(&Value::Nil),
        "SetHostility",
        "hostile",
    )?;
    let _silent = value_to_bool(args.get(3).unwrap_or(&Value::Nil), "SetHostility", "silent")?;
    let no_calls = value_to_bool(
        args.get(4).unwrap_or(&Value::Nil),
        "SetHostility",
        "no-calls",
    )?;

    let player_exists = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .is_some_and(|context| context.player_state(player).is_some())
    });
    if !player_exists {
        return Ok(Value::Bool(false));
    }

    let reject_args = [
        Value::Int(player),
        Value::Int(opponent),
        Value::Bool(new_hostility),
    ];
    if !no_calls
        && value_raw_truthy(&broadcast_global_callback(
            "RejectHostilityChange",
            &reject_args,
            true,
        )?)
    {
        return Ok(Value::Bool(false));
    }

    let old_hostility = HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return None;
        };
        if player == opponent || context.player_state(opponent).is_none() {
            return None;
        }
        let state = context.player_state_mut(player)?;
        let old = state.is_hostile_towards(opponent);
        state.set_hostility_entry(opponent, new_hostility);
        context.record_player_command(PlayerCommand::SetHostility {
            player_id: player,
            opponent,
            hostile: new_hostility,
        });
        Some(old)
    });
    let Some(old_hostility) = old_hostility else {
        return Ok(Value::Bool(false));
    };

    let changed_args = [
        Value::Int(player),
        Value::Int(opponent),
        Value::Bool(new_hostility),
        Value::Bool(old_hostility),
    ];
    broadcast_global_callback("OnHostilityChange", &changed_args, false)?;
    Ok(Value::Bool(true))
}

/// `FnSetFoW` (C4Script.cpp:3671-3678): validate the player, immediately
/// persist the requested flag through `C4Player::SetFoW`, and return an
/// integer success value. C4Aul nil-fills both omitted parameters.
pub(crate) fn set_fow(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 2 {
        return Err(RuntimeError::new(
            "SetFoW expects at most 2 arguments: enabled, player",
        ));
    }
    let enabled = value_to_bool(args.first().unwrap_or(&Value::Nil), "SetFoW", "enabled")?;
    let player_id = value_to_i32(args.get(1).unwrap_or(&Value::Nil), "SetFoW", "player")?;

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Ok(Value::Int(0));
        };
        let Some(player) = context.player_state_mut(player_id) else {
            return Ok(Value::Int(0));
        };
        player.fog_of_war = enabled;
        player.force_fog_of_war = true;
        context.record_player_command(PlayerCommand::SetFogOfWar { player_id, enabled });
        Ok(Value::Int(1))
    })
}

pub(crate) fn set_plr_show_control_pos(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 2 {
        return Err(RuntimeError::new(
            "SetPlrShowControlPos expects at most 2 arguments: player, position",
        ));
    }
    let player_id = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "SetPlrShowControlPos",
        "player",
    )?;
    let position = value_to_i32(
        args.get(1).unwrap_or(&Value::Nil),
        "SetPlrShowControlPos",
        "position",
    )?;
    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Ok(Value::Bool(false));
        };
        let Some(player) = context.player_state_mut(player_id) else {
            return Ok(Value::Bool(false));
        };
        player.show_control_position = position;
        context.record_player_command(PlayerCommand::SetShowControlPosition {
            player_id,
            position,
        });
        Ok(Value::Bool(true))
    })
}

pub(crate) fn set_plr_show_control(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 2 {
        return Err(RuntimeError::new(
            "SetPlrShowControl expects at most 2 arguments: player, controls",
        ));
    }
    let player_id = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "SetPlrShowControl",
        "player",
    )?;
    // C4Aul's parameter conversion maps missing/falsy C4String* arguments
    // to nil before FnStringPar turns them into the empty string
    // (C4AulExec.cpp:1370-1374; C4Script.cpp:78-81).
    let controls = match args.get(1) {
        Some(Value::String(controls)) => controls.as_ref(),
        Some(Value::Nil | Value::Int(0) | Value::Bool(false)) | None => "",
        Some(other) => {
            return Err(RuntimeError::new(format!(
                "SetPlrShowControl: expected string controls, got {}",
                other.type_name()
            )));
        }
    };
    let mask = string_bit_eval(controls);

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Ok(Value::Bool(false));
        };
        let Some(player) = context.player_state_mut(player_id) else {
            return Ok(Value::Bool(false));
        };
        player.show_control = mask;
        context.record_player_command(PlayerCommand::SetShowControl { player_id, mask });
        Ok(Value::Bool(true))
    })
}

/// FnSetPlrShowCommand (C4Script.cpp:2553-2559): set the exact command key
/// that blinks for a live player. The engine fold also requests the local
/// Config.Graphics.ShowCommands enable after this validated call.
pub(crate) fn set_plr_show_command(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 2 {
        return Err(RuntimeError::new(
            "SetPlrShowCommand expects at most 2 arguments: player, command",
        ));
    }
    let player_id = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "SetPlrShowCommand",
        "player",
    )?;
    let command = value_to_i32(
        args.get(1).unwrap_or(&Value::Nil),
        "SetPlrShowCommand",
        "command",
    )?;
    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Ok(Value::Bool(false));
        };
        if context.player_state(player_id).is_none() {
            return Ok(Value::Bool(false));
        }
        context.record_player_command(PlayerCommand::SetShowCommand { player_id, command });
        Ok(Value::Bool(true))
    })
}

/// The `StdCompiler::IsIdentifier` gate on extra-data names
/// (StdCompiler.cpp:92-100): alphanumerics, `_` and `-` only.
fn is_extra_data_identifier(name: &str) -> bool {
    name.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// `FnGetPlrExtraData` (C4Script.cpp:4734-4747): the named
/// C4Player::ExtraData slot; nil for invalid players and unknown names.
pub(crate) fn get_plr_extra_data(args: &[Value]) -> Result<Value, RuntimeError> {
    let player_id = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "GetPlrExtraData",
        "player",
    )?;
    let Some(Value::String(name)) = args.get(1) else {
        // A nil C4String* dereferences to no name — no slot matches.
        return Ok(Value::Nil);
    };
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        let Some(player) = context.player_state(player_id) else {
            return Ok(Value::Nil);
        };
        Ok(player
            .extra_data
            .iter()
            .find(|(slot, _)| slot == name)
            .map(|(_, value)| value.clone())
            .unwrap_or(Value::Nil))
    })
}

/// `FnGetCrewExtraData` (C4Script.cpp:4786-4800): read one exact-case
/// `C4ObjectInfoCore::ExtraData` slot. A nil crew defaults to the caller;
/// objects without Info and unknown names return nil.
pub(crate) fn get_crew_extra_data(args: &[Value]) -> Result<Value, RuntimeError> {
    let target = parse_object_reference_argument(
        args.first().unwrap_or(&Value::Nil),
        "GetCrewExtraData",
        "crew",
    )?;
    let Some(Value::String(name)) = args.get(1) else {
        return Ok(Value::Nil);
    };

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        let Some(target) = target.or(context.script_object_context) else {
            return Ok(Value::Nil);
        };
        // An existing scope is authoritative even when its Info was cleared
        // during this call; never fall back to the callback-entry world copy.
        let info = match context.object_scope(target) {
            Some(scope) => scope.info_core(),
            None => context.world.crew_infos.get(&target),
        };
        Ok(info
            .and_then(|info| info.extra_data.iter().find(|(slot, _)| slot == name))
            .map(|(_, value)| value.clone())
            .unwrap_or(Value::Nil))
    })
}

/// `FnSetCrewExtraData` (C4Script.cpp:4743-4784): validate the slot name and
/// serializable value type, then update the caller's (for nil) or explicit
/// object's C4ObjectInfo. Every failure returns nil.
pub(crate) fn set_crew_extra_data(args: &[Value]) -> Result<Value, RuntimeError> {
    let target = parse_object_reference_argument(
        args.first().unwrap_or(&Value::Nil),
        "SetCrewExtraData",
        "crew",
    )?;
    let Some(Value::String(name)) = args.get(1) else {
        return Ok(Value::Nil);
    };
    if name.is_empty() {
        return Ok(Value::Nil);
    }
    if !is_extra_data_identifier(name) {
        let escaped_name = name.replace('\\', "\\\\").replace('"', "\\\"");
        tracing::error!(
            target: "clonk-script",
            "SetCrewExtraData: Ignoring invalid data name \"{}\"! Only alphanumerics, _ and - are allowed.",
            escaped_name
        );
        return Ok(Value::Nil);
    }
    let data = args.get(2).cloned().unwrap_or(Value::Nil);

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Ok(Value::Nil);
        };
        let Some(target) = target.or(context.script_object_context) else {
            return Ok(Value::Nil);
        };
        if !context.ensure_object_scope(target) {
            return Ok(Value::Nil);
        }
        let (link, mut info) = {
            let Some(scope) = context.object_scope(target) else {
                return Ok(Value::Nil);
            };
            let Some(info) = scope.info_core().cloned() else {
                return Ok(Value::Nil);
            };
            (scope.info_link(), info)
        };
        // C4V_Any/Int/Bool/C4ID only, checked after Info
        // (C4Script.cpp:4757-4763).
        if !matches!(
            data,
            Value::Nil | Value::Int(_) | Value::Bool(_) | Value::C4Id(_)
        ) {
            return Ok(Value::Nil);
        }

        match info
            .extra_data
            .iter_mut()
            .find(|(slot, _)| slot == name.as_ref())
        {
            Some((_, value)) => *value = data.clone(),
            None => info
                .extra_data
                .push((name.as_ref().to_owned(), data.clone())),
        }
        context
            .object_scope_mut(target)
            .expect("ensured object scope")
            .set_info_core(Some(info));

        if let Some(link) = link {
            let mut state = context.world.crew_info_state.borrow_mut();
            if let Some(entry) = state.entries.get_mut(&link) {
                match entry
                    .extra_data
                    .iter_mut()
                    .find(|(slot, _)| slot == name.as_ref())
                {
                    Some((_, value)) => *value = data.clone(),
                    None => entry
                        .extra_data
                        .push((name.as_ref().to_owned(), data.clone())),
                }
            }
            for entries in state.idle.values_mut() {
                for (candidate, entry) in entries {
                    if *candidate != link {
                        continue;
                    }
                    match entry
                        .extra_data
                        .iter_mut()
                        .find(|(slot, _)| slot == name.as_ref())
                    {
                        Some((_, value)) => *value = data.clone(),
                        None => entry
                            .extra_data
                            .push((name.as_ref().to_owned(), data.clone())),
                    }
                }
            }
        }
        context.record_player_command(PlayerCommand::SetCrewExtraData {
            object_id: target,
            link,
            name: name.as_ref().to_owned(),
            value: data.clone(),
        });
        Ok(data)
    })
}

/// `FnSetPlrExtraData` (C4Script.cpp:4692-4732): validates the name
/// (IsIdentifier) and the payload type (nil/int/bool/id only), stores the
/// slot and returns the stored value; every failure yields nil.
pub(crate) fn set_plr_extra_data(args: &[Value]) -> Result<Value, RuntimeError> {
    let player_id = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "SetPlrExtraData",
        "player",
    )?;
    let Some(Value::String(name)) = args.get(1) else {
        return Ok(Value::Nil);
    };
    if name.is_empty() {
        return Ok(Value::Nil);
    }
    if !is_extra_data_identifier(name.as_ref()) {
        tracing::warn!(
            name = %name,
            "SetPlrExtraData: ignoring invalid data name; only alphanumerics, _ and - are allowed"
        );
        return Ok(Value::Nil);
    }
    let data = args.get(2).cloned().unwrap_or(Value::Nil);
    // C4V_Any/Int/Bool/C4ID only (C4Script.cpp:4706-4710).
    if !matches!(
        data,
        Value::Nil | Value::Int(_) | Value::Bool(_) | Value::C4Id(_)
    ) {
        return Ok(Value::Nil);
    }
    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Ok(Value::Nil);
        };
        let Some(player) = context.player_state_mut(player_id) else {
            return Ok(Value::Nil);
        };
        match player
            .extra_data
            .iter_mut()
            .find(|(slot, _)| slot == name.as_ref())
        {
            Some((_, value)) => *value = data.clone(),
            None => player
                .extra_data
                .push((name.as_ref().to_owned(), data.clone())),
        }
        context.record_player_command(PlayerCommand::SetExtraData {
            player_id,
            name: name.as_ref().to_owned(),
            value: data.clone(),
        });
        Ok(data)
    })
}

pub(crate) fn get_score(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "GetScore expects at most 1 argument: player",
        ));
    }
    // An unfilled iPlr slot is nil -> 0 (C4AulExec parameter filling).
    let player_id = value_to_i32(args.first().unwrap_or(&Value::Nil), "GetScore", "player")?;
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        let Some(player) = context.player_state(player_id) else {
            return Ok(Value::Nil);
        };
        Ok(Value::Int(player.points))
    })
}

/// `FnDoScore` (C4Script.cpp:2762-2766) -> `C4Player::DoPoints`
/// (C4Player.cpp:1824-1828): add and clamp Points, then return integer 1
/// for every valid player rather than the new total.
pub(crate) fn do_score(args: &[Value]) -> Result<Value, RuntimeError> {
    let player_id = value_to_i32(args.first().unwrap_or(&Value::Nil), "DoScore", "player")?;
    let change = value_to_i32(args.get(1).unwrap_or(&Value::Nil), "DoScore", "change")?;

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Ok(Value::Int(0));
        };
        let Some(player) = context.player_state_mut(player_id) else {
            return Ok(Value::Int(0));
        };
        let points = (i64::from(player.points) + i64::from(change)).clamp(-100_000, 100_000) as i32;
        player.points = points;
        player.view_value = 100;
        context.record_player_command(PlayerCommand::AdjustPoints {
            player_id,
            delta: change,
        });
        Ok(Value::Int(1))
    })
}

pub(crate) fn default_rank_name(names: &[String], rank: i32) -> Option<&str> {
    usize::try_from(rank)
        .ok()
        .and_then(|rank| names.get(rank).map(String::as_str))
}

pub(crate) fn apply_host_crew_experience(
    context: &mut EffectHostContext,
    target: ObjectId,
    change: i32,
) -> bool {
    if !context.ensure_object_scope(target) {
        return false;
    }

    let info_definition_physical = context
        .object_scope(target)
        .and_then(ObjectScopeContext::info_core)
        .and_then(|info| context.definition_metadata(info.definition_id.as_str()))
        .map(|metadata| metadata.physical);
    let Some((link, mut info, promoted)) = context.object_scope_mut(target).and_then(|scope| {
        let link = scope.info_link();
        let mut info = scope.info_core()?.clone();
        let promoted = crate::adjust_crew_experience(&mut info, change);
        scope.set_info_core(Some(info.clone()));
        if promoted {
            scope.set_info_rank(Some(info.rank));
            let physical = scope
                .info_physical
                .or(info_definition_physical)
                .unwrap_or(scope.definition_physical);
            scope.info_physical =
                Some(crate::promotion_updated_physical(physical, info.rank, None));
            scope.record_physicals();
        }
        Some((link, info, promoted))
    }) else {
        return true;
    };

    let promotion_rank_name = if promoted {
        match context.world.definition_rank_names.get(&info.definition_id) {
            Some(names) => usize::try_from(info.rank)
                .ok()
                .and_then(|rank| names.get(rank))
                .map(|name| name.into_owned()),
            None => {
                default_rank_name(&context.world.default_rank_names, info.rank).map(str::to_owned)
            }
        }
    } else {
        None
    };
    if let Some(rank_name) = promotion_rank_name.as_ref() {
        info.rank_name = rank_name.clone();
        if let Some(scope) = context.object_scope_mut(target) {
            scope.set_info_core(Some(info.clone()));
        }
    }

    // This projection is live throughout the VM call. A following
    // GrabObjectInfo/MakeCrewMember must see the changed pointer payload.
    if let Some(link) = link {
        let mut state = context.world.crew_info_state.borrow_mut();
        if let Some(entry) = state.entries.get_mut(&link) {
            entry.rank = info.rank;
            entry.rank_name = info.rank_name.clone();
            entry.experience = info.experience;
        }
        for entries in state.idle.values_mut() {
            for (candidate, entry) in entries {
                if *candidate == link {
                    entry.rank = info.rank;
                    entry.rank_name = info.rank_name.clone();
                    entry.experience = info.experience;
                }
            }
        }
    }

    context.record_player_command(PlayerCommand::AdjustCrewExperience {
        object_id: target,
        link,
        change,
    });

    if let Some(rank_name) = promotion_rank_name {
        let object_name = context
            .object_custom_name(target)
            .unwrap_or_else(|| info.name.clone());
        context.register_message(MessageCommand::Add(MessageSpec {
            kind: MessageKind::Target,
            text: format!("{object_name} is promoted|to {rank_name}!"),
            target: Some(target),
            player: None,
            offset: Vector2::ZERO,
            color: invert_rgba_alpha(LEGACY_DEFAULT_MESSAGE_COLOR),
            flags: 0,
            width: None,
            decoration: None,
            frame_decoration: None,
            portrait: None,
        }));
        context
            .audio_mut()
            .play_sound("Trumpet", Some(target), 100, false, false, None);
    }

    true
}

/// `FnDoCrewExp` -> `C4Object::DoExperience` (C4Script.cpp:4964-4972;
/// C4Object.cpp:1518-1529). The persistent info pointer is independent of
/// crew membership, ownership and liveness, so any resolved object succeeds;
/// an info-less object is simply a successful no-op.
pub(crate) fn do_crew_exp(args: &[Value]) -> Result<Value, RuntimeError> {
    let change = value_to_i32(args.first().unwrap_or(&Value::Nil), "DoCrewExp", "change")?;
    let target_id =
        parse_object_reference_argument(args.get(1).unwrap_or(&Value::Nil), "DoCrewExp", "target")?;

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Ok(Value::Bool(false));
        };
        let Some(target) = target_id.or_else(|| context.object_context().map(|object| object.id()))
        else {
            return Ok(Value::Bool(false));
        };
        Ok(Value::Bool(apply_host_crew_experience(
            context, target, change,
        )))
    })
}

pub(crate) fn get_plr_value(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "GetPlrValue expects at most 1 argument: player",
        ));
    }
    // An unfilled iPlr slot is nil -> 0 (C4AulExec parameter filling).
    let player_id = value_to_i32(args.first().unwrap_or(&Value::Nil), "GetPlrValue", "player")?;
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        let Some(player) = context.player_state(player_id) else {
            return Ok(Value::Nil);
        };
        Ok(Value::Int(player.value))
    })
}

pub(crate) fn get_plr_value_gain(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "GetPlrValueGain expects at most 1 argument: player",
        ));
    }
    // An unfilled iPlr slot is nil -> 0 (C4AulExec parameter filling).
    let player_id = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "GetPlrValueGain",
        "player",
    )?;
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        let Some(player) = context.player_state(player_id) else {
            return Ok(Value::Nil);
        };
        Ok(Value::Int(player.value_gain))
    })
}

/// FnSetPortrait (C4Script.cpp:5333-5350): current portrait and the optional
/// pNewPortrait override are fields of the target's C4ObjectInfo.
pub(crate) fn set_portrait(args: &[Value]) -> Result<Value, RuntimeError> {
    let name = parse_optional_string(args.first(), "SetPortrait", "portrait")?;
    let target =
        parse_object_reference_argument(args.get(1).unwrap_or(&Value::Nil), "SetPortrait", "obj")?;
    let source = parse_native_c4id_argument(args.get(2), "SetPortrait")?;
    let permanent = value_to_bool(
        args.get(3).unwrap_or(&Value::Nil),
        "SetPortrait",
        "permanent",
    )?;
    let copy_graphics = value_to_bool(
        args.get(4).unwrap_or(&Value::Nil),
        "SetPortrait",
        "copy graphics",
    )?;
    let Some(name) = name.filter(|name| !name.is_empty()) else {
        return Ok(Value::Bool(false));
    };
    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Ok(Value::Bool(true));
        };
        let active = context.object_context().map(|object| object.id());
        let Some(target) = target.or(active) else {
            return Ok(Value::Bool(false));
        };
        if !context.object_status_present(target) || !context.ensure_object_scope(target) {
            return Ok(Value::Bool(false));
        }
        let Some((link, mut info)) = context
            .object_scope(target)
            .and_then(|object| Some((object.info_link(), object.info_core()?.clone())))
        else {
            return Ok(Value::Bool(false));
        };

        let mut portraits = info.portraits.clone();
        if name == "none" {
            portraits.current = None;
            info.core.owned_portrait_source.clear();
            info.core.owned_portrait_name.clear();
            if permanent {
                portraits.permanent = CrewPermanentPortrait::ExplicitNone;
            }
        } else {
            // idSourceDef 0 falls back to the target's live definition.
            let Some(mut source) =
                source.or_else(|| context.object_effective_definition_id(target))
            else {
                return Ok(Value::Bool(false));
            };
            let Some(metadata) = context.world.definition_metadata(&source) else {
                return Ok(Value::Bool(false));
            };
            let mut names = metadata.portrait_names.clone();
            // C4ObjectInfo::SetPortrait rejects a definition without a
            // Portraits list before examining the custom special case.
            if names.is_empty() && name != "random" {
                return Ok(Value::Bool(false));
            }

            let mut assign_permanently = permanent;
            let mut copy = copy_graphics;
            let selected =
                if name == "custom"
                    && info.portraits.fallback.as_ref().is_some_and(|portrait| {
                        portrait.source.is_none() && portrait.name == "custom"
                    })
                {
                    // Relinking pCustomPortrait ignores both flags.
                    portraits.current = info.portraits.fallback.clone();
                    info.core.owned_portrait_source.clear();
                    info.core.owned_portrait_name.clear();
                    None
                } else {
                    let canonical_name = if name == "random" {
                        if names.is_empty() {
                            source = "CLNK".to_string();
                            let Some(metadata) = context.world.definition_metadata(&source) else {
                                return Ok(Value::Bool(false));
                            };
                            names = metadata.portrait_names.clone();
                            if names.is_empty() {
                                return Ok(Value::Bool(false));
                            }
                            assign_permanently = true;
                            copy = false;
                        }
                        let index = SCRIPT_SAFE_RNG
                            .with(|rng| rng.borrow_mut().random(names.len() as i32))
                            as usize;
                        names[index].clone()
                    } else {
                        let Some(canonical) = names
                            .iter()
                            .find(|candidate| candidate.eq_ignore_ascii_case(&name))
                        else {
                            return Ok(Value::Bool(false));
                        };
                        canonical.clone()
                    };
                    let selected = if copy {
                        info.core.owned_portrait_source = source.clone();
                        info.core.owned_portrait_name = canonical_name.clone();
                        CrewPortrait {
                            source: None,
                            name: "custom".to_string(),
                        }
                    } else {
                        info.core.owned_portrait_source.clear();
                        info.core.owned_portrait_name.clear();
                        CrewPortrait {
                            source: Some(DefinitionId::from(source.as_str())),
                            name: canonical_name,
                        }
                    };
                    portraits.current = Some(selected.clone());
                    Some(selected)
                };
            if let Some(selected) = selected.filter(|_| assign_permanently) {
                portraits.permanent = CrewPermanentPortrait::Assigned(selected);
            }
        }

        info.portraits = portraits.clone();
        let owned_portrait_source = info.core.owned_portrait_source.clone();
        let owned_portrait_name = info.core.owned_portrait_name.clone();
        let Some(object) = context.object_scope_mut(target) else {
            return Ok(Value::Bool(false));
        };
        object.set_info_core(Some(info));
        if let Some(link) = link {
            if let Some(entry) = context
                .world
                .crew_info_state
                .borrow_mut()
                .entries
                .get_mut(&link)
            {
                entry.portraits = portraits.clone();
                entry.core.owned_portrait_source = owned_portrait_source;
                entry.core.owned_portrait_name = owned_portrait_name;
            }
        }
        context.record_player_command(PlayerCommand::SetCrewInfoPortrait {
            object_id: target,
            link,
            portraits,
        });
        Ok(Value::Bool(true))
    })
}

fn resolve_current_portrait(
    context: &EffectHostContext,
    portrait: &CrewPortrait,
) -> Option<CrewPortrait> {
    let Some(source) = portrait.source.as_ref() else {
        return Some(portrait.clone());
    };
    let canonical_name = context
        .world
        .definition_metadata(source.as_str())
        .and_then(|metadata| {
            metadata
                .portrait_names
                .iter()
                .find(|name| name.eq_ignore_ascii_case(&portrait.name))
        })?
        .clone();
    Some(CrewPortrait {
        source: Some(source.clone()),
        name: canonical_name,
    })
}

/// FnGetPortrait (C4Script.cpp:5353-5399): permanent reads pNewPortrait
/// first and consult the saved/custom fallback only when that pointer is
/// absent. Current reads inspect the independently mutable Portrait field.
pub(crate) fn get_portrait(args: &[Value]) -> Result<Value, RuntimeError> {
    let target =
        parse_object_reference_argument(args.first().unwrap_or(&Value::Nil), "GetPortrait", "obj")?;
    let get_id = args.get(1).map(value_raw_truthy).unwrap_or(false);
    let permanent = args.get(2).map(value_raw_truthy).unwrap_or(false);
    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Ok(Value::Nil);
        };
        let active = context.object_context().map(|object| object.id());
        let Some(target) = target.or(active) else {
            return Ok(Value::Nil);
        };
        if !context.object_status_present(target) || !context.ensure_object_scope(target) {
            return Ok(Value::Nil);
        }
        let Some(info) = context
            .object_scope(target)
            .and_then(ObjectScopeContext::info_core)
        else {
            return Ok(Value::Nil);
        };
        let portrait = if permanent {
            match &info.portraits.permanent {
                CrewPermanentPortrait::Absent => info.portraits.fallback.clone(),
                CrewPermanentPortrait::ExplicitNone => None,
                CrewPermanentPortrait::Assigned(portrait) => Some(portrait.clone()),
            }
        } else {
            info.portraits
                .current
                .as_ref()
                .and_then(|portrait| resolve_current_portrait(context, portrait))
        };
        Ok(portrait
            .map(|portrait| {
                if get_id {
                    portrait
                        .source
                        .and_then(|source| definition_id_for_c4id(source.as_str()))
                        .map(Value::C4Id)
                        .unwrap_or(Value::Nil)
                } else {
                    Value::String(portrait.name.into())
                }
            })
            .unwrap_or(Value::Nil))
    })
}

/// FnSetPlrView (C4Script.cpp:2545-2550): switch a valid player to
/// C4PVM_Target and follow the supplied ViewTarget. C4Player::ViewCursor and
/// viewport HUD focus remain independent; the later player phase copies the
/// target position into the modeled viewport center.
pub(crate) fn set_plr_view(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 2 {
        return Err(RuntimeError::new(
            "SetPlrView expects at most 2 arguments: player and target",
        ));
    }
    let player_id = value_to_i32(args.first().unwrap_or(&Value::Nil), "SetPlrView", "player")?;
    let object = args
        .get(1)
        .map(|value| parse_object_reference_argument(value, "SetPlrView", "target"))
        .transpose()?
        .flatten();
    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Ok(Value::Bool(false));
        };
        let Some(player) = context.player_state_mut(player_id) else {
            return Ok(Value::Bool(false));
        };
        player.set_view_target(object);
        context.record_player_command(PlayerCommand::SetPlrView { player_id, object });
        Ok(Value::Bool(true))
    })
}

/// FnGetPlrViewMode (C4Script.cpp:2579-2584): local, non-recording games
/// expose C4Player::ViewMode; synchronized control and invalid players return
/// -1 because the process-local mode must not affect synchronized scripts.
pub(crate) fn get_plr_view_mode(args: &[Value]) -> Result<Value, RuntimeError> {
    let player_id = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "GetPlrViewMode",
        "player",
    )?;
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let mode = borrow
            .as_ref()
            .filter(|context| !context.world.control_sync_mode)
            .and_then(|context| context.player_state(player_id))
            .map_or(-1, |player| player.view_mode);
        Ok(Value::Int(mode))
    })
}

/// FnGetPlrView (C4Script.cpp:2586-2591): expose ViewTarget only while the
/// valid player is in C4PVM_Target. Unlike GetPlrViewMode, C++ does not hide
/// this pointer during synchronized execution.
pub(crate) fn get_plr_view(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "GetPlrView expects at most 1 argument: player",
        ));
    }
    let player_id = value_to_i32(args.first().unwrap_or(&Value::Nil), "GetPlrView", "player")?;
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let target = borrow
            .as_ref()
            .and_then(|context| context.player_state(player_id))
            .filter(|player| player.view_mode == crate::PLAYER_VIEW_MODE_TARGET)
            .and_then(|player| player.view_target);
        Ok(target.map(object_reference_value).unwrap_or(Value::Nil))
    })
}

/// FnSetFilmView (C4Script.cpp:5134-5148): validate the target even in live
/// games, where the call is a no-op. Replay execution hands the temporary
/// first-viewport player assignment to the embedding app.
pub(crate) fn set_film_view(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "SetFilmView expects at most 1 argument: player",
        ));
    }
    let player = value_to_i32(args.first().unwrap_or(&Value::Nil), "SetFilmView", "player")?;
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = borrow.as_ref();
        if player != OWNER_NONE
            && context.is_none_or(|context| context.player_state(player).is_none())
        {
            return Ok(Value::Bool(false));
        }
        if let Some(context) = context
            .filter(|context| context.world.replay_control && context.world.film_viewport_available)
        {
            context
                .world
                .viewport_presentation_requests
                .borrow_mut()
                .push(crate::ViewportPresentationRequest::SetFilmView { player });
        }
        Ok(Value::Bool(true))
    })
}

/// FnSetPlrViewRange (C4Script.cpp:3681-3691): persist the object's FoW
/// range, including the legacy low-range clamp unless `exact` is true.
pub(crate) fn set_plr_view_range(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut range = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "SetPlrViewRange",
        "range",
    )?;
    let target = parse_object_reference_argument(
        args.get(1).unwrap_or(&Value::Nil),
        "SetPlrViewRange",
        "obj",
    )?;
    let exact = value_to_bool(
        args.get(2).unwrap_or(&Value::Nil),
        "SetPlrViewRange",
        "exact",
    )?;
    if !exact && range > 0 && range < 128 {
        range = 128;
    }
    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Ok(Value::Int(0));
        };
        let target = target.or_else(|| context.object_context().map(ObjectScopeContext::id));
        let Some(target) = target else {
            return Ok(Value::Int(0));
        };
        if !context.ensure_object_scope(target) {
            return Ok(Value::Int(0));
        }
        Ok(Value::Int(i32::from(
            context.set_object_plr_view_range(target, range),
        )))
    })
}

/// FnGetPlrDownDouble (C4Script.cpp:2618-2622): the player's live
/// double-Down countdown. A missing player returns nil.
pub(crate) fn get_plr_down_double(args: &[Value]) -> Result<Value, RuntimeError> {
    let player_id = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "GetPlrDownDouble",
        "player",
    )?;
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        let Some(player) = context.player_state(player_id) else {
            return Ok(Value::Nil);
        };
        Ok(Value::Int(player.control.last_com_down_double))
    })
}

/// FnClearLastPlrCom (C4Script.cpp:2624-2635): clear only LastCom and
/// LastComDownDouble. C++ deliberately leaves LastComDelay and PressedComs.
pub(crate) fn clear_last_plr_com(args: &[Value]) -> Result<Value, RuntimeError> {
    let player_id = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "ClearLastPlrCom",
        "player",
    )?;
    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Ok(Value::Bool(false));
        };
        let Some(player) = context.player_state_mut(player_id) else {
            return Ok(Value::Bool(false));
        };
        player.control.last_com = 0;
        player.control.last_com_down_double = 0;
        context.record_player_command(PlayerCommand::ClearLastPlrCom { player_id });
        Ok(Value::Bool(true))
    })
}

pub(crate) fn sync_homebase_material_to_team_live(player: i32) {
    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return;
        };
        if !context.team_home_base_rule() {
            return;
        }
        let Some((team, material)) = context.player_state(player).and_then(|state| {
            state
                .team
                .map(|team| (team, state.exact_home_base_material_entries()))
        }) else {
            return;
        };
        let teammates = context
            .player_ids()
            .iter()
            .copied()
            .filter(|candidate| {
                *candidate != player
                    && context
                        .player_state(*candidate)
                        .and_then(|state| state.team)
                        == Some(team)
            })
            .collect::<Vec<_>>();
        for teammate in teammates {
            if let Some(state) = context.player_state_mut(teammate) {
                state.set_home_base_material_entries(material.clone());
            }
        }
        context
            .record_player_command(PlayerCommand::SyncHomeBaseMaterialToTeam { player_id: player });
    });
}

/// FnGetLeague (C4Script.cpp:3556-3561): the indexed semicolon-delimited
/// section of the exact Game.Parameters.League byte buffer.
pub(crate) fn get_league(args: &[Value]) -> Result<Value, RuntimeError> {
    let index = value_to_i32(args.first().unwrap_or(&Value::Nil), "GetLeague", "index")?;
    let Ok(index) = usize::try_from(index) else {
        return Ok(Value::Nil);
    };
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        let Some(section) = context
            .world
            .league_name
            .split(|byte| *byte == b';')
            .nth(index)
        else {
            return Ok(Value::Nil);
        };
        if section.is_empty() {
            Ok(Value::Nil)
        } else {
            Ok(Value::String(
                clonk_script::c4_string_from_bytes(section).into(),
            ))
        }
    })
}

/// FnGetLeagueScore (C4Script.cpp:5926-5935): return the exact signed score
/// stored on a persistent C4PlayerInfo. This lookup is independent of both
/// the display league name and `C4GameParameters::isLeague`.
pub(crate) fn get_league_score(args: &[Value]) -> Result<Value, RuntimeError> {
    // Typed C4Aul parameter conversion precedes context and ID validation.
    let player_info_id = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "GetLeagueScore",
        "player info ID",
    )?;

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        Ok(context
            .world
            .player_info_league_score(player_info_id)
            .map(Value::Int)
            .unwrap_or(Value::Nil))
    })
}

/// FnGetLeagueProgressData (C4Script.cpp:2869-2875): resolve the exact
/// persistent C4PlayerInfo row, but only when Game.Parameters.League is set.
pub(crate) fn get_league_progress_data(args: &[Value]) -> Result<Value, RuntimeError> {
    // Typed C4Aul parameter conversion precedes the league-name gate.
    let player_info_id = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "GetLeagueProgressData",
        "player info ID",
    )?;

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        if !context.world.league_name_configured() {
            return Ok(Value::Nil);
        }
        let Some(Some(bytes)) = context
            .world
            .player_info_league_progress_data(player_info_id)
        else {
            return Ok(Value::Nil);
        };
        Ok(Value::String(
            clonk_script::c4_string_from_bytes(bytes).into(),
        ))
    })
}

/// `FnSetLeagueProgressData` (C4Script.cpp:2860-2866): update the exact
/// retained C4PlayerInfo buffer when the display league name is configured.
pub(crate) fn set_league_progress_data(args: &[Value]) -> Result<Value, RuntimeError> {
    // Native parameter conversion is complete before either function gate.
    let data =
        parse_native_c4_string_argument(args.first(), "SetLeagueProgressData", "progress data")?
            .map(|text| clonk_script::c4_string_bytes(&text));
    let player_info_id = value_to_i32(
        args.get(1).unwrap_or(&Value::Nil),
        "SetLeagueProgressData",
        "player info ID",
    )?;

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Ok(Value::Bool(false));
        };
        if !context.world.league_name_configured()
            || !context.world.player_info_id_known(player_info_id)
        {
            return Ok(Value::Bool(false));
        }
        // The C++ mutation is synchronous: a getter later in this same VM
        // call must already observe it before the deferred Engine fold.
        let updated = context
            .world
            .set_player_info_league_progress_data(player_info_id, data.clone());
        debug_assert!(updated, "validated player-info ID must update");
        context.record_player_command(PlayerCommand::SetLeagueProgressData {
            player_info_id,
            data,
        });
        Ok(Value::Bool(true))
    })
}

pub(crate) fn get_hi_rank(args: &[Value]) -> Result<Value, RuntimeError> {
    // FnGetHiRank (C4Script.cpp:2792-2796) ->
    // C4Player::GetHiRankActiveCrew(false) (C4Player.cpp:1003-1020): walk
    // the crew in order, rank from the linked Info (no info = -1); only a
    // strictly higher rank replaces, so the first of equal ranks wins.
    let player_id = value_to_i32(args.first().unwrap_or(&Value::Nil), "GetHiRank", "player")?;
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        let Some(player) = context.player_state(player_id) else {
            return Ok(Value::Nil);
        };
        let mut best: Option<(u64, i32)> = None;
        for crew_id in &player.crew {
            if context.object_crew_disabled(*crew_id).unwrap_or(false) {
                continue;
            }
            let rank = match context.object_scope(*crew_id) {
                Some(scope) => scope.info_rank(),
                None => context.world.crew_rank(crew_id.as_u64()),
            }
            .unwrap_or(-1);
            match best {
                Some((_, best_rank)) if best_rank >= rank => {}
                _ => best = Some((crew_id.as_u64(), rank)),
            }
        }
        Ok(best
            .map(|(id, _)| object_reference_value(ObjectId::new(id)))
            .unwrap_or(Value::Nil))
    })
}

/// FnGetRank (C4Script.cpp:1378-1383): read the linked C4ObjectInfo rank.
/// A null object defaults to the executing object; objects without Info and
/// global calls without an object return nil.
pub(crate) fn get_rank(args: &[Value]) -> Result<Value, RuntimeError> {
    let target = args
        .first()
        .map(|value| parse_object_reference_argument(value, "GetRank", "obj"))
        .transpose()?
        .flatten();
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        let Some(target) = target.or_else(|| context.object_context().map(|object| object.id()))
        else {
            return Ok(Value::Nil);
        };
        let rank = match context.object_scope(target) {
            Some(scope) => scope.info_rank(),
            None => context.world.crew_rank(target.as_u64()),
        };
        Ok(rank.map(Value::Int).unwrap_or(Value::Nil))
    })
}

pub(crate) fn get_crew(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 2 {
        return Err(RuntimeError::new(
            "GetCrew expects at most 2 arguments: player and index",
        ));
    }
    // Unfilled iPlr/index slots are nil -> 0 (FnGetCrew, C4Script.cpp:2798);
    // SkiesOfFire's InitializePlayer calls GetCrew(iPlr) with no index.
    let player_id = value_to_i32(args.first().unwrap_or(&Value::Nil), "GetCrew", "player")?;
    let index = value_to_i32(args.get(1).unwrap_or(&Value::Nil), "GetCrew", "index")?;
    if index < 0 {
        return Ok(Value::Nil);
    }
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        let Some(player) = context.player_state(player_id) else {
            return Ok(Value::Nil);
        };
        let idx = index as usize;
        let Some(crew_id) = player.crew.get(idx) else {
            return Ok(Value::Nil);
        };
        Ok(object_reference_value(ObjectId::new(crew_id.as_u64())))
    })
}

pub(crate) fn get_crew_count(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "GetCrewCount expects at most 1 argument: player",
        ));
    }
    // An unfilled iPlr slot is nil -> 0 (C4AulExec parameter filling).
    let player_id = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "GetCrewCount",
        "player",
    )?;
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        let Some(player) = context.player_state(player_id) else {
            return Ok(Value::Nil);
        };
        Ok(Value::Int(truncate_to_i32(player.crew.len() as u64)))
    })
}

pub(crate) fn get_cursor_host(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 2 {
        return Err(RuntimeError::new(
            "GetCursor expects at most 2 arguments: player and optional index",
        ));
    }
    let player_id = value_to_i32(args.first().unwrap_or(&Value::Nil), "GetCursor", "player")?;
    let index = value_to_i32(args.get(1).unwrap_or(&Value::Nil), "GetCursor", "index")?;
    if index < 0 {
        return Ok(Value::Nil);
    }
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        let Some(player) = context.player_state(player_id) else {
            return Ok(Value::Nil);
        };
        if index == 0 {
            return Ok(player
                .cursor
                .map(object_reference_value)
                .unwrap_or(Value::Nil));
        }
        let mut remaining = index as usize;
        for crew_id in &player.crew {
            if player.cursor == Some(*crew_id) {
                continue;
            }
            let selected = context
                .get_world_object(*crew_id)
                .map(|object| object.selected && !object.crew_disabled)
                .unwrap_or_else(|| {
                    context
                        .world
                        .crew_selection
                        .get(&player_id)
                        .is_some_and(|selection| selection.selected.contains(crew_id))
                });
            if !selected {
                continue;
            }
            remaining -= 1;
            if remaining == 0 {
                return Ok(object_reference_value(*crew_id));
            }
        }
        Ok(Value::Nil)
    })
}

/// FnEditCursor (C4Script.cpp:3537-3541): process-local developer-console
/// state is hidden from every synchronized control mode. The C++ cursor
/// clears its raw pointer when an object is removed; mirror that guarantee
/// when a copied host context observes same-call removal.
pub(crate) fn edit_cursor(_args: &[Value]) -> Result<Value, RuntimeError> {
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        if context.world.control_sync_mode {
            return Ok(Value::Nil);
        }
        let Some(target) = context.world.edit_cursor_target else {
            return Ok(Value::Nil);
        };
        if context.removed_object_references.contains(&target)
            || !context
                .get_world_object(target)
                .is_some_and(|object| object.is_present())
        {
            return Ok(Value::Nil);
        }
        Ok(object_reference_value(target))
    })
}

pub(crate) fn get_view_cursor(args: &[Value]) -> Result<Value, RuntimeError> {
    // An unfilled iPlr slot is nil -> 0 (C4AulExec parameter filling).
    let player_id = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "GetViewCursor",
        "player",
    )?;
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        let Some(player) = context.player_state(player_id) else {
            return Ok(Value::Nil);
        };
        Ok(player
            .view_cursor
            .map(object_reference_value)
            .unwrap_or(Value::Nil))
    })
}

/// FnGetCaptain (C4Script.cpp:2939-2943): return the stored player Captain
/// pointer verbatim. FinalInit owns its one-time assignment; this query does
/// not recompute it from the cursor or current highest-ranked crew member.
pub(crate) fn get_captain(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "GetCaptain expects at most 1 argument: player",
        ));
    }
    let player_id = value_to_i32(args.first().unwrap_or(&Value::Nil), "GetCaptain", "player")?;
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        let Some(player) = context.player_state(player_id) else {
            return Ok(Value::Nil);
        };
        Ok(player
            .captain
            .map(object_reference_value)
            .unwrap_or(Value::Nil))
    })
}

/// FnSetViewCursor (C4Script.cpp:2954-2963): assign the camera-follow
/// pointer. Unlike SetCursor, C++ validates neither object Status nor crew
/// membership and performs no selection callbacks.
pub(crate) fn set_view_cursor(args: &[Value]) -> Result<Value, RuntimeError> {
    let player_id = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "SetViewCursor",
        "player",
    )?;
    let object = args
        .get(1)
        .map(|value| parse_object_reference_argument(value, "SetViewCursor", "object"))
        .transpose()?
        .flatten();
    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Ok(Value::Bool(false));
        };
        let Some(player) = context.player_state_mut(player_id) else {
            return Ok(Value::Bool(false));
        };
        player.set_view_cursor(object);
        context.record_player_command(PlayerCommand::SetViewCursor { player_id, object });
        Ok(Value::Bool(true))
    })
}

pub(crate) fn get_select_count(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "GetSelectCount expects at most 1 argument: player",
        ));
    }
    // An unfilled iPlr slot is nil -> 0 (C4AulExec parameter filling).
    let player_id = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "GetSelectCount",
        "player",
    )?;
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        let Some(player) = context.player_state(player_id) else {
            return Ok(Value::Nil);
        };
        // This is C4Player::SelectCount, not a live recount. UpdateCounts
        // refreshes it at the start of Player::Execute; selection changes
        // made later in the same callback remain stale until that boundary.
        Ok(Value::Int(player.select_count))
    })
}

pub(crate) fn get_homebase_material(args: &[Value]) -> Result<Value, RuntimeError> {
    let player_id = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "GetHomebaseMaterial",
        "player",
    )?;
    let definition = parse_native_c4id_argument(args.get(1), "GetHomebaseMaterial")?;
    let index = match args.get(2) {
        Some(Value::Nil) | None => 0,
        Some(value) => value_to_i32(value, "GetHomebaseMaterial", "index")?,
    };
    let category = match args.get(3) {
        Some(Value::Nil) | None => 0,
        Some(value) => value_to_i32(value, "GetHomebaseMaterial", "category")?,
    };

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        let Some(player) = context.player_state(player_id) else {
            return Ok(Value::Nil);
        };

        let entries = player.exact_home_base_material_entries();
        if let Some(definition) = definition {
            return Ok(Value::Int(home_base_id_count(&entries, &definition)));
        }

        Ok(home_base_id_by_index(&entries, category, index, context)
            .map(Value::C4Id)
            .unwrap_or(Value::Nil))
    })
}

pub(crate) fn get_homebase_production(args: &[Value]) -> Result<Value, RuntimeError> {
    let player_id = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "GetHomebaseProduction",
        "player",
    )?;
    let definition = parse_native_c4id_argument(args.get(1), "GetHomebaseProduction")?;
    let index = match args.get(2) {
        Some(Value::Nil) | None => 0,
        Some(value) => value_to_i32(value, "GetHomebaseProduction", "index")?,
    };
    let category = match args.get(3) {
        Some(Value::Nil) | None => 0,
        Some(value) => value_to_i32(value, "GetHomebaseProduction", "category")?,
    };

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        let Some(player) = context.player_state(player_id) else {
            return Ok(Value::Nil);
        };

        let entries = player.exact_home_base_production_entries();
        if let Some(definition) = definition {
            return Ok(Value::Int(home_base_id_count(&entries, &definition)));
        }

        Ok(home_base_id_by_index(&entries, category, index, context)
            .map(Value::C4Id)
            .unwrap_or(Value::Nil))
    })
}

pub(crate) fn get_plr_knowledge(args: &[Value]) -> Result<Value, RuntimeError> {
    let player_id = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "GetPlrKnowledge",
        "player",
    )?;
    let definition = parse_native_c4id_argument(args.get(1), "GetPlrKnowledge")?;
    let index = match args.get(2) {
        Some(Value::Nil) | None => 0,
        Some(value) => value_to_i32(value, "GetPlrKnowledge", "index")?,
    };
    let category = match args.get(3) {
        Some(Value::Nil) | None => 0,
        Some(value) => value_to_i32(value, "GetPlrKnowledge", "category")?,
    };

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        let Some(player) = context.player_state(player_id) else {
            return Ok(Value::Nil);
        };

        if let Some(definition) = definition {
            let known = player.knows_definition(&definition);
            return Ok(Value::Bool(known));
        }

        if index < 0 {
            return Ok(Value::Nil);
        }

        let filtered: Vec<DefinitionId> = player
            .exact_knowledge_entries()
            .into_iter()
            .filter_map(|(entry, _)| {
                let metadata = context.definition_metadata(&entry)?;
                if category != -1 && metadata.category & category == 0 {
                    return None;
                }
                Some(entry)
            })
            .collect();

        let idx = index as usize;
        if idx >= filtered.len() {
            return Ok(Value::Nil);
        }

        // FnGetPlrKnowledge's indexed form is a typed C4ID, not a string
        // (C4Script.cpp:2659-2666). Definition-call syntax (`id->Func`)
        // depends on preserving that type.
        Ok(Value::C4Id(filtered[idx].clone()))
    })
}

pub(crate) fn set_plr_knowledge(args: &[Value]) -> Result<Value, RuntimeError> {
    let player_id = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "SetPlrKnowledge",
        "player",
    )?;
    let definition = match parse_native_c4id_argument(args.get(1), "SetPlrKnowledge")? {
        Some(id) => id,
        None => return Ok(Value::Bool(false)),
    };
    let remove = value_to_bool(
        args.get(2).unwrap_or(&Value::Nil),
        "SetPlrKnowledge",
        "remove flag",
    )?;

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Ok(Value::Bool(false));
        };

        if remove {
            let Some(player) = context.player_state_mut(player_id) else {
                return Ok(Value::Bool(false));
            };
            if player.remove_knowledge_entry(&definition) {
                context.record_player_command(PlayerCommand::RevokeKnowledge {
                    player_id,
                    definition_id: definition,
                });
                Ok(Value::Bool(true))
            } else {
                Ok(Value::Bool(false))
            }
        } else {
            if context.definition_metadata(&definition).is_none() {
                return Ok(Value::Bool(false));
            }
            let player = match context.player_state_mut(player_id) {
                Some(player) => player,
                None => return Ok(Value::Bool(false)),
            };
            player.set_knowledge_entry(definition.clone());
            context.record_player_command(PlayerCommand::GrantKnowledge {
                player_id,
                definition_id: definition,
            });
            Ok(Value::Bool(true))
        }
    })
}

pub(crate) fn get_plr_magic(args: &[Value]) -> Result<Value, RuntimeError> {
    let player_id = value_to_i32(args.first().unwrap_or(&Value::Nil), "GetPlrMagic", "player")?;
    let definition = parse_native_c4id_argument(args.get(1), "GetPlrMagic")?;
    let index = value_to_i32(args.get(2).unwrap_or(&Value::Nil), "GetPlrMagic", "index")?;

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        let Some(player) = context.player_state(player_id) else {
            return Ok(Value::Nil);
        };

        if let Some(definition) = definition {
            return Ok(Value::Bool(player.knows_magic(&definition)));
        }
        if index < 0 {
            return Ok(Value::Nil);
        }

        Ok(player
            .exact_magic_entries()
            .into_iter()
            .filter(|(entry, _)| {
                context
                    .definition_metadata(entry)
                    .is_some_and(|metadata| metadata.category & crate::CATEGORY_MAGIC != 0)
            })
            .nth(index as usize)
            .map(|(id, _)| Value::C4Id(id))
            .unwrap_or(Value::Nil))
    })
}

pub(crate) fn set_plr_magic(args: &[Value]) -> Result<Value, RuntimeError> {
    let player_id = value_to_i32(args.first().unwrap_or(&Value::Nil), "SetPlrMagic", "player")?;
    let definition = match parse_native_c4id_argument(args.get(1), "SetPlrMagic")? {
        Some(id) => id,
        None => return Ok(Value::Int(0)),
    };
    let remove = value_to_bool(
        args.get(2).unwrap_or(&Value::Nil),
        "SetPlrMagic",
        "remove flag",
    )?;

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Ok(Value::Int(0));
        };

        if remove {
            let Some(player) = context.player_state_mut(player_id) else {
                return Ok(Value::Int(0));
            };
            if !player.remove_magic_entry(&definition) {
                return Ok(Value::Int(0));
            }
            context.record_player_command(PlayerCommand::RevokeMagic {
                player_id,
                definition_id: definition,
            });
            Ok(Value::Int(1))
        } else {
            if context.definition_metadata(&definition).is_none() {
                return Ok(Value::Int(0));
            }
            let Some(player) = context.player_state_mut(player_id) else {
                return Ok(Value::Int(0));
            };
            player.set_magic_entry(definition.clone());
            context.record_player_command(PlayerCommand::GrantMagic {
                player_id,
                definition_id: definition,
            });
            Ok(Value::Int(1))
        }
    })
}

pub(crate) fn do_homebase_material(args: &[Value]) -> Result<Value, RuntimeError> {
    let player_id = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "DoHomebaseMaterial",
        "player",
    )?;
    let definition = match parse_native_c4id_argument(args.get(1), "DoHomebaseMaterial")? {
        Some(id) => id,
        None => return Ok(Value::Bool(false)),
    };
    let change = match args.get(2) {
        Some(Value::Nil) | None => 0,
        Some(value) => value_to_i32(value, "DoHomebaseMaterial", "change")?,
    };

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Ok(Value::Bool(false));
        };

        if context.definition_metadata(&definition).is_none()
            && context.definition_category(&definition).is_none()
        {
            return Ok(Value::Bool(false));
        }

        let (team_id, updated_material) = {
            let player = match context.player_state_mut(player_id) {
                Some(player) => player,
                None => return Ok(Value::Bool(false)),
            };
            player.adjust_home_base_material_entry(definition.clone(), change);
            (player.team, player.exact_home_base_material_entries())
        };

        if context.team_home_base_rule() {
            if let Some(team) = team_id {
                let teammates: Vec<i32> = context
                    .player_ids()
                    .iter()
                    .copied()
                    .filter(|other_id| {
                        *other_id != player_id
                            && context.player_state(*other_id).and_then(|state| state.team)
                                == Some(team)
                    })
                    .collect();
                for other_id in teammates {
                    if let Some(member) = context.player_state_mut(other_id) {
                        member.set_home_base_material_entries(updated_material.clone());
                    }
                }
            }
        }

        context.record_player_command(PlayerCommand::AdjustHomeBaseMaterial {
            player_id,
            definition_id: definition,
            delta: change,
        });

        Ok(Value::Bool(true))
    })
}

pub(crate) fn do_homebase_production(args: &[Value]) -> Result<Value, RuntimeError> {
    let player_id = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "DoHomebaseProduction",
        "player",
    )?;
    let definition = match parse_native_c4id_argument(args.get(1), "DoHomebaseProduction")? {
        Some(id) => id,
        None => return Ok(Value::Bool(false)),
    };
    let change = match args.get(2) {
        Some(Value::Nil) | None => 0,
        Some(value) => value_to_i32(value, "DoHomebaseProduction", "change")?,
    };

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Ok(Value::Bool(false));
        };

        if context.definition_metadata(&definition).is_none()
            && context.definition_category(&definition).is_none()
        {
            return Ok(Value::Bool(false));
        }

        if context
            .player_state_mut(player_id)
            .map(|player| {
                player.adjust_home_base_production_entry(definition.clone(), change);
            })
            .is_none()
        {
            return Ok(Value::Bool(false));
        }

        context.record_player_command(PlayerCommand::AdjustHomeBaseProduction {
            player_id,
            definition_id: definition,
            delta: change,
        });

        Ok(Value::Bool(true))
    })
}

/// FnSetScoreboardData (C4Script.cpp:5881-5884): script order is row then
/// column; C4Scoreboard::SetCell receives column then row and returns void.
pub(crate) fn set_scoreboard_data(args: &[Value]) -> Result<Value, RuntimeError> {
    let row = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "SetScoreboardData",
        "row",
    )?;
    let column = value_to_i32(
        args.get(1).unwrap_or(&Value::Nil),
        "SetScoreboardData",
        "column",
    )?;
    let text = parse_optional_string(args.get(2), "SetScoreboardData", "text")?;
    let data = value_to_i32(
        args.get(3).unwrap_or(&Value::Nil),
        "SetScoreboardData",
        "data",
    )?;

    HOST_CONTEXT.with(|cell| {
        if let Some(context) = cell.borrow().as_ref() {
            context
                .world
                .scoreboard
                .borrow_mut()
                .set_cell(column, row, text, data);
            context
                .world
                .scoreboard_presentations
                .borrow_mut()
                .invalidate_layout();
        }
    });
    Ok(Value::Nil)
}

fn scoreboard_cell_keys(args: &[Value], function: &str) -> Result<(i32, i32), RuntimeError> {
    let row = value_to_i32(args.first().unwrap_or(&Value::Nil), function, "row")?;
    let column = value_to_i32(args.get(1).unwrap_or(&Value::Nil), function, "column")?;
    Ok((column, row))
}

/// FnGetScoreboardString (C4Script.cpp:5886-5889): a null cell string is
/// returned as nil; allocated empty strings remain strings.
pub(crate) fn get_scoreboard_string(args: &[Value]) -> Result<Value, RuntimeError> {
    let (column, row) = scoreboard_cell_keys(args, "GetScoreboardString")?;
    Ok(HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| {
                context
                    .world
                    .scoreboard
                    .borrow()
                    .cell_by_key(column, row)
                    .and_then(|cell| cell.text().map(str::to_string))
            })
            .map(|text| Value::String(text.into()))
            .unwrap_or(Value::Nil)
    }))
}

/// FnGetScoreboardData (C4Script.cpp:5891-5894): a missing cell reads zero.
pub(crate) fn get_scoreboard_data(args: &[Value]) -> Result<Value, RuntimeError> {
    let (column, row) = scoreboard_cell_keys(args, "GetScoreboardData")?;
    Ok(Value::Int(HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| {
                context
                    .world
                    .scoreboard
                    .borrow()
                    .cell_by_key(column, row)
                    .map(crate::ScoreboardCell::value)
            })
            .unwrap_or(0)
    })))
}

/// FnDoScoreboardShow (C4Script.cpp:5896-5908): the optional player number is
/// one-based; remote players report success without changing this client's
/// local dialog refcount.
pub(crate) fn do_scoreboard_show(args: &[Value]) -> Result<Value, RuntimeError> {
    let change = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "DoScoreboardShow",
        "change",
    )?;
    let for_player = value_to_i32(
        args.get(1).unwrap_or(&Value::Nil),
        "DoScoreboardShow",
        "player",
    )?;

    Ok(HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Value::Bool(false);
        };
        if for_player != 0 {
            let player = for_player.wrapping_sub(1);
            if !context.world.players.contains_key(&player) {
                return Value::Bool(false);
            }
            if !context.world.local_players.contains(&player) {
                return Value::Bool(true);
            }
        }
        // DoDlgShow returns before changing iDlgShow while C4GUI is invalid or
        // exclusive. The app activates this sink only after the final startup/
        // restore snapshot enters shared in-game GUI mode; game-over creation
        // suppression remains app-owned (C4Scoreboard.cpp:234-251).
        context
            .world
            .scoreboard_presentations
            .borrow_mut()
            .apply_show_change(&mut context.world.scoreboard.borrow_mut(), change);
        Value::Bool(true)
    }))
}

/// FnSortScoreboard (C4Script.cpp:5910-5913) delegates to the stable
/// C4Scoreboard row sort and reports whether the key exists.
pub(crate) fn sort_scoreboard(args: &[Value]) -> Result<Value, RuntimeError> {
    let column = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "SortScoreboard",
        "column",
    )?;
    let reverse = value_to_bool(
        args.get(1).unwrap_or(&Value::Nil),
        "SortScoreboard",
        "reverse",
    )?;
    Ok(Value::Bool(HOST_CONTEXT.with(|cell| {
        cell.borrow().as_ref().is_some_and(|context| {
            context
                .world
                .scoreboard
                .borrow_mut()
                .sort_by(column, reverse)
        })
    })))
}

/// `FnAddEvaluationData` (C4Script.cpp:5915-5924): append one nonempty
/// scenario string to either the global evaluation text (ID zero) or the row
/// identified by the persistent C4PlayerInfo ID.
pub(crate) fn add_evaluation_data(args: &[Value]) -> Result<Value, RuntimeError> {
    // Native parameter conversion happens before the body, so convert both
    // slots before taking the empty-text early return.
    let text = parse_optional_string(args.first(), "AddEvaluationData", "text")?;
    let player_info_id = value_to_i32(
        args.get(1).unwrap_or(&Value::Nil),
        "AddEvaluationData",
        "player info ID",
    )?;
    let Some(text) = text.filter(|text| !text.is_empty()) else {
        return Ok(Value::Bool(false));
    };

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Ok(Value::Bool(false));
        };
        if player_info_id != 0 && !context.world.player_info_id_known(player_info_id) {
            return Ok(Value::Bool(false));
        }
        context.record_player_command(PlayerCommand::AddEvaluationData {
            player_info_id,
            text,
        });
        Ok(Value::Bool(true))
    })
}

/// `FnHideSettlementScoreInEvaluation` (C4Script.cpp:5937-5940): overwrite
/// the round-results presentation flag and return the native void value.
pub(crate) fn hide_settlement_score_in_evaluation(args: &[Value]) -> Result<Value, RuntimeError> {
    let hide = value_to_bool(
        args.first().unwrap_or(&Value::Nil),
        "HideSettlementScoreInEvaluation",
        "hide",
    )?;
    HOST_CONTEXT.with(|cell| {
        if let Some(context) = cell.borrow_mut().as_mut() {
            context.record_player_command(PlayerCommand::HideSettlementScore { hide });
        }
    });
    Ok(Value::Nil)
}

/// `FnSetLeaguePerformance` (C4Script.cpp:2852-2858): in league games,
/// overwrite the global result slot (ID zero) or an exact persistent
/// C4PlayerInfo row. Both typed integer conversions precede the league gate.
pub(crate) fn set_league_performance(args: &[Value]) -> Result<Value, RuntimeError> {
    let score = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "SetLeaguePerformance",
        "score",
    )?;
    let player_info_id = value_to_i32(
        args.get(1).unwrap_or(&Value::Nil),
        "SetLeaguePerformance",
        "player info ID",
    )?;
    // C4Aul's typed two-parameter dispatch evaluates and discards surplus
    // arguments before entering the native function.

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Ok(Value::Bool(false));
        };
        if !context.world.league_game() {
            return Ok(Value::Bool(false));
        }
        if player_info_id != 0 && !context.world.player_info_id_known(player_info_id) {
            return Ok(Value::Bool(false));
        }
        context.record_player_command(PlayerCommand::SetLeaguePerformance {
            score,
            player_info_id,
        });
        Ok(Value::Bool(true))
    })
}

fn home_base_id_count(entries: &[(DefinitionId, i32)], definition: &DefinitionId) -> i32 {
    entries
        .iter()
        .find(|(candidate, _)| candidate == definition)
        .map(|(_, count)| *count)
        .unwrap_or(0)
}

fn home_base_id_by_index(
    entries: &[(DefinitionId, i32)],
    category: i32,
    index: i32,
    context: &EffectHostContext,
) -> Option<DefinitionId> {
    let index = usize::try_from(index).ok()?;
    entries
        .iter()
        .filter(|(definition_id, _)| {
            context
                .definition_category(definition_id)
                .is_some_and(|definition_category| {
                    category == -1 || definition_category & category != 0
                })
        })
        .nth(index)
        .map(|(definition_id, _)| definition_id.clone())
}

fn adjust_id_count(
    map: &mut HashMap<DefinitionId, u32>,
    definition_id: &DefinitionId,
    delta: i32,
    max: Option<u32>,
) -> u32 {
    match map.entry(definition_id.clone()) {
        Entry::Occupied(mut occupied) => {
            if delta >= 0 {
                let mut new_value = occupied.get().saturating_add(delta as u32);
                if let Some(limit) = max {
                    new_value = new_value.min(limit);
                }
                if new_value == 0 {
                    occupied.remove();
                    0
                } else {
                    occupied.insert(new_value);
                    new_value
                }
            } else {
                let current = *occupied.get();
                let decrease = delta.saturating_abs() as u32;
                if current <= decrease {
                    occupied.remove();
                    0
                } else {
                    let new_value = current - decrease;
                    occupied.insert(new_value);
                    new_value
                }
            }
        }
        Entry::Vacant(vacant) => {
            if delta <= 0 {
                0
            } else {
                let mut new_value = delta as u32;
                if let Some(limit) = max {
                    new_value = new_value.min(limit);
                }
                if new_value == 0 {
                    0
                } else {
                    vacant.insert(new_value);
                    new_value
                }
            }
        }
    }
}

const CFG_MAX_STRING: usize = 1024;

pub(crate) fn mission_access_contains(list: &str, password: &str) -> bool {
    let list = clonk_script::c4_string_bytes(list);
    let password = clonk_script::c4_string_bytes(password);
    list.split(|byte| *byte == b';').any(|module| {
        let start = module
            .iter()
            .position(|byte| *byte != b' ')
            .unwrap_or(module.len());
        let end = module
            .iter()
            .rposition(|byte| *byte != b' ')
            .map_or(start, |index| index + 1);
        c4_bytes_equal_no_case(&module[start..end], &password)
    })
}

/// FnGainMissionAccess (C4Script.cpp:2368-2373): the length guard precedes
/// case-insensitive SAddModule, whose duplicate/empty no-op still reports
/// success through this host function.
pub(crate) fn gain_mission_access(args: &[Value]) -> Result<Value, RuntimeError> {
    let password =
        parse_optional_string(args.first(), "GainMissionAccess", "password")?.unwrap_or_default();
    let granted = HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return false;
        };
        let mut access = context.world.mission_access.borrow_mut();
        if clonk_script::c4_string_byte_len(&access)
            .saturating_add(clonk_script::c4_string_byte_len(&password))
            .saturating_add(3)
            > CFG_MAX_STRING
        {
            return false;
        }
        if !password.is_empty() && !mission_access_contains(&access, &password) {
            if !access.is_empty() {
                access.push(';');
            }
            access.push_str(&password);
        }
        true
    });
    Ok(Value::Bool(granted))
}

/// FnGetMissionAccess (C4Script.cpp:3924-3933): query the same config-side,
/// case-insensitive semicolon module list; a null string is false.
pub(crate) fn get_mission_access(args: &[Value]) -> Result<Value, RuntimeError> {
    let Some(password) = parse_optional_string(args.first(), "GetMissionAccess", "password")?
    else {
        return Ok(Value::Bool(false));
    };
    Ok(Value::Bool(HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return false;
        };
        if context.world.control_sync_mode {
            tracing::warn!(
                target: "clonk-script",
                "using GetMissionAccess may cause desyncs when playing records!"
            );
        }
        let contains = mission_access_contains(&context.world.mission_access.borrow(), &password);
        contains
    })))
}

pub(crate) const DEFAULT_NEXT_MISSION_TEXT: &str = "&Next scenario";
pub(crate) const DEFAULT_NEXT_MISSION_DESCRIPTION: &str = "Continue with the next scenario.";

#[derive(Debug, Clone, PartialEq, Eq)]
#[doc(hidden)]
pub enum NextMissionCommand {
    Set {
        path: String,
        text: String,
        description: String,
    },
    Clear,
}

pub(crate) fn set_next_mission(args: &[Value]) -> Result<Value, RuntimeError> {
    let path = parse_optional_string(args.first(), "SetNextMission", "mission")?;
    let command = match path.filter(|path| !path.is_empty()) {
        Some(path) => NextMissionCommand::Set {
            path,
            text: parse_optional_string(args.get(1), "SetNextMission", "button text")?
                .unwrap_or_else(|| DEFAULT_NEXT_MISSION_TEXT.to_string()),
            description: parse_optional_string(args.get(2), "SetNextMission", "description")?
                .unwrap_or_else(|| DEFAULT_NEXT_MISSION_DESCRIPTION.to_string()),
        },
        None => NextMissionCommand::Clear,
    };
    HOST_CONTEXT.with(|cell| {
        if let Some(context) = cell.borrow_mut().as_mut() {
            context.next_mission_commands.push(command);
        }
    });
    Ok(Value::Nil)
}

/// `FnSetRestoreInfos` (C4Script.cpp:6116-6119): retain the unvalidated raw
/// mask for the runtime network-restart handoff and return native void.
pub(crate) fn set_restore_infos(args: &[Value]) -> Result<Value, RuntimeError> {
    let what = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "SetRestoreInfos",
        "restore mask",
    )?;
    HOST_CONTEXT.with(|cell| {
        if let Some(context) = cell.borrow_mut().as_mut() {
            context.record_player_command(PlayerCommand::SetRestoreInfos { what });
        }
    });
    Ok(Value::Nil)
}

pub(crate) fn resolve_target_player(context: &EffectHostContext, player_id: i32) -> Option<i32> {
    if player_id >= 0 && context.player_state(player_id).is_some() {
        Some(player_id)
    } else {
        None
    }
}

/// FnGrabObjectInfo (C4Script.cpp:2170-2176) -> C4Object::GrabInfo
/// (C4Object.cpp:5696-5726): `pTo` (default: the caller) takes pFrom's
/// exact info pointer, after both objects are removed from every player crew,
/// then registers the receiver with its unchanged Owner.
pub(crate) fn grab_object_info(args: &[Value]) -> Result<Value, RuntimeError> {
    let from = parse_object_reference_argument(
        args.first().unwrap_or(&Value::Nil),
        "GrabObjectInfo",
        "from",
    )?;
    let Some(from) = from else {
        return Ok(Value::Bool(false));
    };
    let to = parse_object_reference_argument(
        args.get(1).unwrap_or(&Value::Nil),
        "GrabObjectInfo",
        "to",
    )?;
    let result = HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return None;
        };
        let active = context.object_context().map(|object| object.id());
        let Some(to) = to.or(active) else {
            return None;
        };
        if !context.ensure_object_scope(from) || !context.ensure_object_scope(to) {
            return None;
        }
        if !context.object_status_present(from) || !context.object_status_present(to) {
            return None;
        }
        if from == to {
            return Some((to, None));
        }
        let donor_has_info = context
            .object_scope(from)
            .is_some_and(|scope| scope.info_core().is_some() || scope.info_rank().is_some());
        if !donor_has_info {
            return None;
        }

        // Retire and clear the receiver's old Info before either global
        // ClearPointers call.
        let receiver_link = context
            .object_scope(to)
            .and_then(ObjectScopeContext::info_link);
        if let Some(link) = receiver_link {
            if retire_host_crew_info(context, link) {
                context.record_player_command(PlayerCommand::RetireCrewInfo {
                    object_id: to,
                    link,
                });
            }
        }
        if let Some(scope) = context.object_scope_mut(to) {
            scope.set_info_rank(None);
            scope.set_info_link(None);
            scope.set_info_core(None);
            scope.info_physical = None;
            scope.record_physicals();
        }

        // ClearPointers may synchronously run CrewSelection through cursor
        // replacement. Release the host borrow so those nested callbacks see
        // and mutate the same live context, then re-read donor Info.
        drop(borrow);
        clear_player_object_pointers_host(from);
        clear_player_object_pointers_host(to);
        let mut borrow = cell.borrow_mut();
        let context = borrow.as_mut()?;
        if !context.ensure_object_scope(from) || !context.ensure_object_scope(to) {
            return None;
        }
        let (donor_core, donor_link, donor_physical, donor_definition_physical) = context
            .object_scope(from)
            .map(|scope| {
                (
                    scope.info_core().cloned().or_else(|| {
                        let rank = scope.info_rank()?;
                        let definition = scope.definition_id.as_deref()?;
                        let definition_id = DefinitionId::from(definition);
                        let rank_names = context.world.definition_rank_names.get(&definition_id);
                        let mut rank_name =
                            default_rank_name(&context.world.default_rank_names, rank)
                                .unwrap_or("Clonk")
                                .to_string();
                        let mut core = CrewInfoCoreFields {
                            type_name: context
                                .definition_metadata(definition)
                                .map(|metadata| crate::bounded_crew_type_name(&metadata.name))
                                .unwrap_or_else(|| "Clonk".to_string()),
                            ..CrewInfoCoreFields::default()
                        };
                        crate::update_custom_rank_fields(
                            &mut rank_name,
                            &mut core,
                            rank,
                            rank_names,
                            context.world.definition_rank_base(definition),
                        );
                        Some(CrewObjectInfo {
                            definition_id,
                            name: context
                                .definition_metadata(definition)
                                .map(|metadata| metadata.name.clone())
                                .unwrap_or_else(|| "Clonk".to_string()),
                            death_message: String::new(),
                            core,
                            rank,
                            rank_name,
                            experience: 0,
                            participation: 1,
                            rounds: 0,
                            death_count: 0,
                            total_playing_time: 0,
                            birthday: 0,
                            age: 0,
                            in_action_time: 0,
                            extra_data: Vec::new(),
                            portraits: CrewPortraitState::default(),
                        })
                    }),
                    scope.info_link(),
                    scope.info_physical,
                    scope.info_definition_physical,
                )
            })
            .unwrap_or((None, None, None, None));
        let mut donor_core = donor_core?;
        if let Some(scope) = context.object_scope_mut(from) {
            scope.set_crew_status_member(false);
            scope.set_info_rank(None);
            scope.set_info_link(None);
            scope.set_info_core(None);
            scope.info_physical = None;
            scope.record_physicals();
        }
        context.refresh_scope_fair_crew(from);

        let target_alive = context
            .object_scope(to)
            .is_some_and(ObjectScopeContext::alive);
        if target_alive {
            donor_core.rank_name = recruited_rank_name(
                &context.world,
                &donor_core.definition_id,
                donor_core.rank,
                &donor_core.rank_name,
            );
        }
        let linked = donor_link.filter(|link| {
            let valid = retire_host_crew_info(context, *link);
            if valid {
                context.record_player_command(PlayerCommand::RetireCrewInfo {
                    object_id: from,
                    link: *link,
                });
                relink_host_crew_info(context, *link, target_alive, !target_alive);
            }
            valid
        });
        if let Some(scope) = context.object_scope_mut(to) {
            scope.set_info_rank(Some(donor_core.rank));
            scope.set_info_link(linked);
            scope.set_info_core(Some(donor_core.clone()));
            scope.info_physical = donor_physical;
            scope.info_definition_physical = donor_definition_physical;
            scope.record_physicals();
        }
        context.refresh_scope_fair_crew(to);
        context.record_player_command(PlayerCommand::LinkCrewInfo {
            object_id: to,
            link: linked,
            info: donor_core,
            created_entry: None,
            recruit: target_alive,
            has_died: !target_alive,
        });

        let owner = context.object_scope(to).map(ObjectScopeContext::owner);
        let callback_owner = owner.filter(|owner| {
            context.player_state(*owner).is_some()
                && context.object_status_present(to)
                && context
                    .object_effective_definition_id(to)
                    .and_then(|definition| context.definition_metadata(&definition))
                    .is_some_and(|metadata| metadata.crew_member)
        });
        if let Some(owner) = callback_owner {
            context.insert_player_crew(owner, to);
            let view_range = context.object_scope_mut(to).map(|scope| {
                scope.set_controller(owner);
                scope.plr_view_range()
            });
            if view_range == Some(0) {
                context.set_object_plr_view_range(to, 500);
            } else if view_range.is_some() {
                context.actualize_object_plr_view_range(to);
            }
        }
        let donor_member = context.object_in_any_crew(from);
        let receiver_member = context.object_in_any_crew(to);
        if let Some(scope) = context.object_scope_mut(from) {
            scope.set_crew_status_member(donor_member);
        }
        if let Some(scope) = context.object_scope_mut(to) {
            scope.set_crew_status_member(receiver_member);
        }
        context.record_crew_rosters();
        Some((to, callback_owner))
    });

    let Some((to, callback_owner)) = result else {
        return Ok(Value::Bool(false));
    };
    if let Some(owner) = callback_owner {
        call_recruitment_callback(to, owner);
        HOST_CONTEXT.with(|cell| {
            let mut borrow = cell.borrow_mut();
            let Some(context) = borrow.as_mut() else {
                return;
            };
            let donor_member = context.object_in_any_crew(from);
            let receiver_member = context.object_in_any_crew(to);
            if let Some(scope) = context.object_scope_mut(from) {
                scope.set_crew_status_member(donor_member);
            }
            if let Some(scope) = context.object_scope_mut(to) {
                scope.set_crew_status_member(receiver_member);
            }
            context.record_crew_rosters();
        });
    }
    Ok(Value::Bool(true))
}

/// FnMakeCrewMember (C4Script.cpp:2164-2168) -> C4Player::MakeCrewMember
/// (C4Player.cpp:1167-1215): valid player + CrewMember def required; assigns
/// an exact idle/new info, joins the independent roster and calls Recruitment.

/// `pObj->Call(PSF_OnJoinCrew)` where PSF_OnJoinCrew is the fail-safe
/// `~Recruitment` name and C4Object::Call uses fPassError=false.
fn call_recruitment_callback(target: ObjectId, player: i32) {
    if let Some(Err(error)) =
        call_world_object_own_function(target, "Recruitment", &[Value::Int(player)])
    {
        tracing::warn!(
            object = %target,
            error = %error,
            "script error in Recruitment callback; continuing like C++"
        );
    }
}

pub(crate) fn retire_host_crew_info(context: &mut EffectHostContext, link: CrewInfoLink) -> bool {
    let (entry, order) = {
        let mut state = context.world.crew_info_state.borrow_mut();
        let Some(entry) = state.entries.get_mut(&link) else {
            return false;
        };
        entry.in_action = false;
        (
            entry.clone(),
            state
                .order
                .get(&link.player_id)
                .cloned()
                .unwrap_or_default(),
        )
    };
    let key = (link.player_id, entry.id.clone());
    let mut state = context.world.crew_info_state.borrow_mut();
    if context.world.definition_metadata(&entry.id).is_none() {
        state.idle.remove(&key);
        return true;
    }
    let rebuilt: Vec<_> = order
        .into_iter()
        .filter_map(|candidate| {
            let info = state.entries.get(&candidate)?;
            (info.id == entry.id && info.participation != 0 && !info.in_action && !info.has_died)
                .then(|| (candidate, info.clone()))
        })
        .collect();
    if rebuilt.is_empty() {
        state.idle.remove(&key);
    } else {
        state.idle.insert(key, rebuilt);
    }
    true
}

/// The `Info` arm of `C4Object::AssignDeath`: the persistent entry remains
/// linked to the corpse, but is marked dead, counted and retired before
/// contents/player pointers are cleared (C4Object.cpp:1185-1190).
pub(crate) fn assign_death_host_crew_info(context: &mut EffectHostContext, target: ObjectId) {
    let Some(link) = context
        .object_scope(target)
        .and_then(ObjectScopeContext::info_link)
    else {
        return;
    };
    let game_time = context.world.game_time();
    let (death_count, total_playing_time) = {
        let mut state = context.world.crew_info_state.borrow_mut();
        let Some(entry) = state.entries.get_mut(&link) else {
            return;
        };
        entry.has_died = true;
        entry.death_count = entry.death_count.wrapping_add(1);
        if entry.in_action {
            entry.total_playing_time = entry
                .total_playing_time
                .wrapping_add(game_time.wrapping_sub(entry.in_action_time));
            entry.in_action = false;
        }
        (entry.death_count, entry.total_playing_time)
    };
    let _ = retire_host_crew_info(context, link);
    if let Some(scope) = context.object_scope_mut(target) {
        if let Some(mut info) = scope.info_core().cloned() {
            info.death_count = death_count;
            info.total_playing_time = total_playing_time;
            scope.set_info_core(Some(info));
        }
    }
    context.record_player_command(PlayerCommand::AssignDeathCrewInfo {
        object_id: target,
        link,
    });
}

fn relink_host_crew_info(
    context: &mut EffectHostContext,
    link: CrewInfoLink,
    recruit: bool,
    has_died: bool,
) -> bool {
    let recruited_name = if recruit {
        let state = context.world.crew_info_state.borrow();
        state.entries.get(&link).map(|entry| {
            recruited_rank_name(
                &context.world,
                &DefinitionId::from(entry.id.as_str()),
                entry.rank,
                &entry.rank_name,
            )
        })
    } else {
        None
    };
    let mut state = context.world.crew_info_state.borrow_mut();
    let Some(entry) = state.entries.get_mut(&link) else {
        return false;
    };
    entry.has_died = has_died;
    if recruit && !entry.in_action {
        if let Some(rank_name) = recruited_name {
            entry.rank_name = rank_name;
        }
    }
    entry.in_action = recruit;
    entry.was_in_action |= recruit;
    let key = (link.player_id, entry.id.clone());
    if let Some(pool) = state.idle.get_mut(&key) {
        pool.retain(|(candidate, _)| *candidate != link);
    }
    true
}

fn recruited_rank_name(
    world: &HostWorldContext,
    definition_id: &DefinitionId,
    rank: i32,
    stored_rank_name: &str,
) -> String {
    world
        .definition_rank_names
        .get(definition_id)
        .filter(|names| !names.is_empty())
        .and_then(|names| {
            usize::try_from(rank)
                .ok()
                .and_then(|rank| names.get_or_last(rank))
                .map(|name| name.into_owned())
        })
        .unwrap_or_else(|| stored_rank_name.to_string())
}

fn recruit_or_create_crew_info(
    context: &mut EffectHostContext,
    player: i32,
    definition_id: &str,
) -> Result<
    Option<(
        CrewInfoLink,
        CrewObjectInfo,
        Option<crate::player_file::CrewInfo>,
        PhysicalInfo,
    )>,
    RuntimeError,
> {
    let key = (player, definition_id.to_string());
    let idle = {
        let mut state = context.world.crew_info_state.borrow_mut();
        let picked = state.idle.get_mut(&key).and_then(|pool| {
            let best = pool
                .iter()
                .enumerate()
                .fold(
                    None,
                    |best: Option<(usize, i32)>, (index, (_, info))| match best {
                        Some((_, best_exp)) if best_exp >= info.experience => best,
                        _ => Some((index, info.experience)),
                    },
                )
                .map(|(index, _)| index)?;
            Some(pool.remove(best))
        });
        if let Some((link, _)) = picked.as_ref() {
            if let Some(entry) = state.entries.get_mut(link) {
                entry.in_action = true;
                entry.was_in_action = true;
            }
        }
        picked
    };
    if let Some((link, mut entry)) = idle {
        entry.rank_name = recruited_rank_name(
            &context.world,
            &DefinitionId::from(entry.id.as_str()),
            entry.rank,
            &entry.rank_name,
        );
        if let Some(stored) = context
            .world
            .crew_info_state
            .borrow_mut()
            .entries
            .get_mut(&link)
        {
            stored.rank_name = entry.rank_name.clone();
        }
        let info = CrewObjectInfo {
            definition_id: DefinitionId::from(entry.id.as_str()),
            name: entry.name.clone(),
            death_message: entry.death_message.clone(),
            core: entry.core.clone(),
            rank: entry.rank,
            rank_name: entry.rank_name.clone(),
            experience: entry.experience,
            participation: entry.participation,
            rounds: entry.rounds,
            death_count: entry.death_count,
            total_playing_time: entry.total_playing_time,
            birthday: entry.birthday,
            age: entry.age,
            in_action_time: entry.in_action_time,
            extra_data: entry.extra_data.clone(),
            portraits: entry.portraits.clone(),
        };
        return Ok(Some((link, info, None, entry.physical)));
    }

    let names_source = context
        .world
        .definition_crew_names
        .get(definition_id)
        .cloned()
        .or_else(|| context.world.standard_crew_names.clone());
    const C4_MAX_NAME: usize = 30;
    let mut name = match names_source {
        Some(names) if names.to_ascii_lowercase().contains("names.txt") => {
            draw_context_random(1000)?;
            "Clonk".to_string()
        }
        Some(names) => {
            let newline_count = names.bytes().filter(|&byte| byte == b'\n').count() as i32;
            let segment_index = draw_context_random(newline_count)? as usize;
            let segment = names
                .split('\n')
                .nth(segment_index)
                .unwrap_or_default()
                .replace('\r', "");
            let cleaned: String = segment.trim().chars().take(C4_MAX_NAME).collect();
            if cleaned.is_empty() {
                "Clonk".to_string()
            } else {
                cleaned
            }
        }
        None => "Clonk".to_string(),
    };

    let physical = context
        .definition_metadata(definition_id)
        .map(|metadata| crate::crew_info_physical(metadata.physical, 0))
        .unwrap_or_default();
    let definition_id_key = DefinitionId::from(definition_id);
    let rank_names = context.world.definition_rank_names.get(&definition_id_key);
    let mut rank_name = "Clonk".to_string();
    let mut core = CrewInfoCoreFields {
        type_name: context
            .definition_metadata(definition_id)
            .map(|metadata| crate::bounded_crew_type_name(&metadata.name))
            .unwrap_or_else(|| "Clonk".to_string()),
        ..CrewInfoCoreFields::default()
    };
    crate::update_custom_rank_fields(
        &mut rank_name,
        &mut core,
        0,
        rank_names,
        context.world.definition_rank_base(definition_id),
    );
    let (link, entry) = {
        let mut state = context.world.crew_info_state.borrow_mut();
        {
            let names = state.roster_names.entry(player).or_default();
            let base = name.clone();
            let mut next_number = 2;
            while names.iter().any(|existing| {
                c4_bytes_equal_no_case(
                    &clonk_script::c4_string_bytes(existing),
                    &clonk_script::c4_string_bytes(&name),
                )
            }) {
                let digits = next_number.to_string();
                let keep = base
                    .chars()
                    .count()
                    .min(C4_MAX_NAME.saturating_sub(digits.len()));
                name = base.chars().take(keep).collect::<String>() + &digits;
                next_number += 1;
            }
            names.push(name.clone());
        }
        let roster_index = *state.next_indices.entry(player).or_insert(0);
        state.next_indices.insert(player, roster_index + 1);
        let link = CrewInfoLink {
            player_id: player,
            roster_index,
        };
        let entry = crate::player_file::CrewInfo {
            id: definition_id.to_string(),
            name,
            death_message: String::new(),
            core,
            rank: 0,
            rank_name,
            experience: 0,
            rounds: 0,
            physical,
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
            portraits: CrewPortraitState::default(),
        };
        state.entries.insert(link, entry.clone());
        state.order.entry(player).or_default().insert(0, link);
        (link, entry)
    };
    let info = CrewObjectInfo {
        definition_id: DefinitionId::from(entry.id.as_str()),
        name: entry.name.clone(),
        death_message: entry.death_message.clone(),
        core: entry.core.clone(),
        rank: entry.rank,
        rank_name: entry.rank_name.clone(),
        experience: entry.experience,
        participation: entry.participation,
        rounds: entry.rounds,
        death_count: entry.death_count,
        total_playing_time: entry.total_playing_time,
        birthday: entry.birthday,
        age: entry.age,
        in_action_time: entry.in_action_time,
        extra_data: entry.extra_data.clone(),
        portraits: entry.portraits.clone(),
    };
    if let Some(player) = context.player_state_mut(player) {
        player.crew_created = player.crew_created.wrapping_add(1);
    }
    Ok(Some((link, info, Some(entry), physical)))
}

/// Direct C4Player::MakeCrewMember mutation. Native callers such as Buy must
/// not dispatch a definition function named MakeCrewMember on the target.
pub(crate) fn make_crew_member_live(target: ObjectId, player: i32) -> Result<bool, RuntimeError> {
    let joined = HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Ok(false);
        };
        if context.player_state(player).is_none() {
            return Ok(false); // ValidPlr (C4Script.cpp:2166)
        }
        if !context.object_status_present(target) || !context.ensure_object_scope(target) {
            return Ok(false); // !pObj->Status (C4Player.cpp:1169)
        }
        let crew_def = context
            .object_effective_definition_id(target)
            .and_then(|definition| context.world.definition_metadata(&definition))
            .map(|metadata| metadata.crew_member)
            .unwrap_or(false);
        if !crew_def {
            return Ok(false); // Def->CrewMember required (C4Player.cpp:1170)
        }
        // C4Player::MakeCrewMember info selection (C4Player.cpp:1180-1199):
        // the gate is `!pObj->Info` — an object that already carries a crew
        // info skips; an idle info skips the New; otherwise
        // C4ObjectInfoList::New draws the name Random over the def's
        // ClonkNames (or the game standard names) — a synced ledger draw
        // that must fire INSIDE this call.
        let already_has_info = context
            .object_scope(target)
            .is_some_and(|object| object.info_rank().is_some());
        let assignment = if already_has_info {
            None
        } else {
            let Some(definition_id) = context.object_effective_definition_id(target) else {
                return Ok(false);
            };
            recruit_or_create_crew_info(context, player, definition_id.as_str())?
        };
        let view_range = {
            let Some(object) = context.object_scope_mut(target) else {
                return Ok(false);
            };
            object.set_crew_member(true);
            if let Some((link, info, _, info_physical)) = assignment.as_ref() {
                object.set_info_rank(Some(info.rank));
                object.set_info_link(Some(*link));
                object.set_info_core(Some(info.clone()));
                object.info_physical = Some(*info_physical);
                object.info_definition_physical = Some(object.definition_physical);
                object.record_physicals();
            }
            // C4Player::MakeCrewMember changes Controller, never Owner
            // (C4Player.cpp:1202-1204).
            object.set_controller(player);
            object.plr_view_range()
        };
        if view_range == 0 {
            context.set_object_plr_view_range(target, 500);
        } else {
            context.actualize_object_plr_view_range(target);
        }
        context.refresh_scope_fair_crew(target);
        if let Some((link, info, created_entry, _)) = assignment {
            context.record_player_command(PlayerCommand::LinkCrewInfo {
                object_id: target,
                link: Some(link),
                info,
                created_entry,
                recruit: true,
                has_died: false,
            });
        }
        // C4Player::MakeCrewMember inserts into the LIVE C4Player::Crew
        // before Recruitment returns (C4Player.cpp:1194-1209). Later calls
        // in the same scenario callback must therefore see the member via
        // SelectCrew/GetCrew. Crew-member definitions take stMain's ordinary
        // category/id branch: prefer the first equal category+id link, then
        // the first link whose relative category is <= the new one
        // (C4ObjectList.cpp:110-195).
        if !context.insert_player_crew(player, target) {
            return Ok(false);
        }
        context.record_crew_rosters();
        Ok(true)
    })?;
    if joined {
        // PSF_OnJoinCrew resolves to the script function "Recruitment"
        // (C4Script.h:107 `#define PSF_OnJoinCrew "~Recruitment"`), fired
        // inside MakeCrewMember (C4Player.cpp:1206-1209). C4Object::Call's
        // default fPassError=false makes it fail-safe.
        call_recruitment_callback(target, player);
    }
    Ok(joined)
}

pub(crate) fn make_crew_member(args: &[Value]) -> Result<Value, RuntimeError> {
    let explicit = parse_object_reference_argument(
        args.first().unwrap_or(&Value::Nil),
        "MakeCrewMember",
        "obj",
    )?;
    let player = parse_optional_i32(args.get(1), "MakeCrewMember", "player")?.unwrap_or(0);
    // FnMakeCrewMember passes pObj through unchanged; unlike several local
    // natives it never substitutes cthr->Obj for nil. AB_CALL only changes
    // the call context and likewise does not inject its receiver into pObj.
    let Some(target) = explicit else {
        return Ok(Value::Bool(false));
    };
    Ok(Value::Bool(make_crew_member_live(target, player)?))
}

/// `C4AulDefCastFunc<C4V_C4ID,C4V_Int>` (C4Script.cpp:6184-6195,
/// :7042): preserve the four-byte C4ID payload and change only its type tag.
pub(crate) fn scoreboard_col(args: &[Value]) -> Result<Value, RuntimeError> {
    let raw = parse_native_c4id_argument(args.first(), "ScoreboardCol")?
        .as_deref()
        .map(cast_c4id_payload)
        .unwrap_or(0) as i32;
    Ok(Value::Int(raw))
}

/// `C4ObjectInfoList::MakeValidName` (C4ObjectInfoList.cpp:93-101): keep the
/// requested name as the fixed base, replace its tail with 2, 3, ... and
/// choose the first case-insensitively unused candidate.
pub(crate) fn make_valid_crew_name(requested: &str, existing: &[String]) -> String {
    let requested_bytes = clonk_script::c4_string_bytes(requested);
    let mut candidate = requested.to_owned();
    let mut suffix = 2u64;
    while existing.iter().any(|name| {
        c4_bytes_equal_no_case(
            &clonk_script::c4_string_bytes(name),
            &clonk_script::c4_string_bytes(&candidate),
        )
    }) {
        let suffix_text = suffix.to_string();
        let keep = requested_bytes
            .len()
            .min(C4_MAX_NAME_BYTES.saturating_sub(suffix_text.len()));
        candidate = clonk_script::c4_string_from_bytes(&requested_bytes[..keep]);
        candidate.push_str(&suffix_text);
        suffix += 1;
    }
    candidate
}

/// FnGetPlrColorDw (C4Script.cpp:3658-3666): the player's resolved
/// C4Player::ColorDw; a missing player reads nil.
pub(crate) fn get_plr_color_dw(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() != 1 {
        return Err(RuntimeError::new(
            "GetPlrColorDw expects exactly 1 argument: player",
        ));
    }
    let player_id = value_to_i32(&args[0], "GetPlrColorDw", "player")?;
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        let Some(player) = context.player_state(player_id) else {
            return Ok(Value::Nil);
        };
        Ok(Value::Int(player.exact_color_dw() as i32))
    })
}

/// FnGetPlrControlName (C4Script.cpp:2568-2571): format the configured key
/// for the player's effective local control set. Configuration lookup
/// failures are a non-nil empty C4 string.
pub(crate) fn get_plr_control_name(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 3 {
        return Err(RuntimeError::new(
            "GetPlrControlName expects at most 3 arguments: player, control, short",
        ));
    }
    let player_id = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "GetPlrControlName",
        "player",
    )?;
    let control = value_to_i32(
        args.get(1).unwrap_or(&Value::Nil),
        "GetPlrControlName",
        "control",
    )?;
    let short = args.get(2).map(Value::as_bool).unwrap_or(false);

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let name = borrow
            .as_ref()
            .and_then(|context| {
                let control_set = context.player_state(player_id)?.control_set;
                context.world.control_key_name(control_set, control, short)
            })
            .unwrap_or_default();
        Ok(Value::String(name.to_string().into()))
    })
}

/// FnGetPlrJumpAndRunControl (C4Script.cpp:2579-2583): the player's
/// ControlStyle (0 classic / 1 Jump'n'Run, C4Player.cpp:2373); an absent
/// player yields -1. The return type is C4ValueInt — there is no nil path.
pub(crate) fn get_plr_jump_and_run_control(args: &[Value]) -> Result<Value, RuntimeError> {
    // An unfilled iPlr slot is nil -> 0 (C4AulExec parameter filling).
    let player_id = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "GetPlrJumpAndRunControl",
        "player",
    )?;
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let control_style = borrow
            .as_ref()
            .and_then(|context| context.player_state(player_id))
            .map(|player| i32::from(player.control.control_style));
        Ok(Value::Int(control_style.unwrap_or(-1)))
    })
}

/// FnGetCrewEnabled (C4Script.cpp:4813-4819): !CrewDisabled; nil/// FnGetCrewEnabled (C4Script.cpp:4813-4819): !CrewDisabled; nil
/// without an object.
pub(crate) fn get_crew_enabled(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut index = 0;
    let target =
        consume_optional_object_reference_argument(args, &mut index, "GetCrewEnabled", "obj")?;
    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        let active = context.object_context().map(|object| object.id());
        let target = target.or(active);
        let Some(target) = target else {
            return Ok(Value::Nil);
        };
        // A same-call SetCrewEnabled staged the flag on the scope.
        if Some(target) == active {
            if let Some(disabled) = context
                .object_context()
                .and_then(|object| object.pending_update.crew_disabled)
            {
                return Ok(Value::Bool(!disabled));
            }
        }
        match context.get_world_object(target) {
            Some(object) => Ok(Value::Bool(
                !object
                    .full_state()
                    .map(|state| state.crew_disabled)
                    .unwrap_or(false),
            )),
            None => Ok(Value::Nil),
        }
    })
}

/// C4Object::Call(PSF_CrewSelection) is fail-safe: callback errors abort the
/// callback but retain prior mutations and never abort the selecting script
/// (C4Object.cpp:5815-5832; C4AulExec.cpp:1318-1342).
fn call_crew_selection_callback(target: ObjectId, unselect: bool, cursor: bool) {
    if let Some(Err(error)) = call_world_object_own_function(
        target,
        "CrewSelection",
        &[Value::Bool(unselect), Value::Bool(cursor)],
    ) {
        tracing::warn!(
            object = %target,
            error = %error,
            "script error in crew selection callback; continuing like the C++ fail-safe exec"
        );
    }
}

/// C4Object::DoSelect (C4Object.cpp:5815-5824). Returns false only for an
/// unknown target; CrewDisabled is a successful callback-less no-op.
fn do_select_host_object(target: ObjectId, cursor: bool) -> bool {
    let disposition = HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow.as_mut()?;
        context.get_world_object(target)?;
        if context.object_crew_disabled(target).unwrap_or(false) {
            return Some(false);
        }
        if !cursor && !context.set_object_selected(target, true) {
            return None;
        }
        Some(true)
    });
    match disposition {
        None => false,
        Some(false) => true,
        Some(true) => {
            call_crew_selection_callback(target, false, cursor);
            true
        }
    }
}

/// C4Object::UnSelect (C4Object.cpp:5827-5832): unlike DoSelect it always
/// invokes the callback, including on CrewDisabled objects.
fn unselect_host_object(target: ObjectId, cursor: bool) -> bool {
    let known = HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return false;
        };
        if context.get_world_object(target).is_none() {
            return false;
        }
        cursor || context.set_object_selected(target, false)
    });
    if !known {
        return false;
    }
    call_crew_selection_callback(target, true, cursor);
    true
}

fn hi_rank_active_crew(
    context: &EffectHostContext,
    player: &PlayerState,
    selected: bool,
) -> Option<ObjectId> {
    let mut best = None;
    let mut highest_rank = -2;
    for &id in &player.crew {
        let Some(object) = context.get_world_object(id) else {
            continue;
        };
        let is_selected = context
            .object_scope(id)
            .map(ObjectScopeContext::selected)
            .unwrap_or(object.selected);
        if context.object_crew_disabled(id).unwrap_or(false) || (selected && !is_selected) {
            continue;
        }
        let rank = match context.object_scope(id) {
            Some(scope) => scope.info_rank(),
            None => context.world.crew_rank(id.as_u64()),
        }
        .unwrap_or(-1);
        if best.is_none() || rank > highest_rank {
            best = Some(id);
            highest_rank = rank;
        }
    }
    best
}

fn record_cursor_state(context: &mut EffectHostContext, player_id: i32) {
    let Some((object, control)) = context
        .player_state(player_id)
        .map(|player| (player.cursor, player.control))
    else {
        return;
    };
    context.record_player_command(PlayerCommand::SetCursor {
        player_id,
        object,
        control,
    });
}

/// C4PlayerList::ClearPointers in player-list order. Cursor replacement and
/// its selection callbacks run before the next player's pointers are cleared,
/// exactly as the C++ loop does.
pub(crate) fn clear_player_object_pointers_host(target: ObjectId) {
    let players = HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Vec::new();
        };
        context.player_ids().to_vec()
    });
    for player_id in players {
        let removed_cursor = HOST_CONTEXT.with(|cell| {
            let mut borrow = cell.borrow_mut();
            let Some(context) = borrow.as_mut() else {
                return false;
            };
            let Some(player) = context.player_state_mut(player_id) else {
                return false;
            };
            let removed_cursor = player.clear_object_pointers_before_cursor_adjust(target);
            context.record_player_command(PlayerCommand::ClearPlayerObjectPointersBeforeAdjust {
                player_id,
                object: target,
            });
            removed_cursor
        });
        if removed_cursor {
            adjust_cursor_host(player_id);
        }
        HOST_CONTEXT.with(|cell| {
            let mut borrow = cell.borrow_mut();
            let Some(context) = borrow.as_mut() else {
                return;
            };
            if let Some(player) = context.player_state_mut(player_id) {
                player.clear_object_pointers_after_cursor_adjust(target);
            }
            // C4Player::ClearPointers(..., false) removes this player's
            // runtime FoW link immediately. AssignDeath uses the separate
            // death helper below and deliberately retains it for decay.
            context
                .world
                .remove_player_fow_view_object(player_id, target);
            context.record_player_command(PlayerCommand::ClearPlayerObjectPointersAfterAdjust {
                player_id,
                object: target,
            });
        });
    }
}

/// The single-owner `C4Player::ClearPointers(object, true)` call made by
/// AssignDeath. Other players are deliberately untouched, and the owner's
/// FoWViewObjs entry is retained for normal dead-view decay
/// (C4Object.cpp:1194-1200; C4Player.cpp:57-82).
pub(crate) fn clear_owner_death_pointers_host(target: ObjectId, owner: i32) {
    let removed_cursor = HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return false;
        };
        let Some(player) = context.player_state_mut(owner) else {
            return false;
        };
        let removed_cursor = player.clear_object_pointers_before_cursor_adjust(target);
        context.record_player_command(PlayerCommand::ClearPlayerObjectPointersBeforeAdjust {
            player_id: owner,
            object: target,
        });
        removed_cursor
    });
    if removed_cursor {
        adjust_cursor_host(owner);
    }
    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return;
        };
        if let Some(player) = context.player_state_mut(owner) {
            player.clear_object_pointers_after_cursor_adjust(target);
        }
        context.record_player_command(PlayerCommand::ClearPlayerObjectPointersAfterAdjust {
            player_id: owner,
            object: target,
        });
    });
}

/// C4Player::AdjustCursorCommand (C4Player.cpp:1235-1258).
fn adjust_cursor_host(player_id: i32) -> bool {
    let Some((previous, next)) = HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow.as_mut()?;
        context.player_state_mut(player_id)?.reset_cursor_view();
        context.record_player_command(PlayerCommand::ResetCursorView { player_id });
        let player = context.player_state(player_id)?.clone();
        let next = hi_rank_active_crew(context, &player, true)
            .or_else(|| hi_rank_active_crew(context, &player, false));
        let previous = player.cursor;
        if previous != next {
            let focus = {
                let player = context.player_state_mut(player_id)?;
                player.cursor = next;
                player.resolved_view_object()
            };
            let position = focus.and_then(|object| {
                context
                    .object_scope(object)
                    .map(ObjectScopeContext::effective_position)
                    .or_else(|| {
                        context
                            .get_world_object(object)
                            .map(|object| object.position())
                    })
            });
            context.player_state_mut(player_id)?.update_view(position);
            context.record_player_command(PlayerCommand::UpdatePlayerView {
                player_id,
                position,
            });
        }
        Some((previous, next))
    }) else {
        return false;
    };

    if previous != next {
        if let Some(previous) = previous {
            unselect_host_object(previous, true);
        }
    }
    // A callback above may itself have changed Cursor. C++ reads the live
    // field again for the final DoSelect call.
    let cursor = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.player_state(player_id))
            .and_then(|player| player.cursor)
    });
    if let Some(cursor) = cursor {
        do_select_host_object(cursor, false);
    }
    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return;
        };
        if let Some(player) = context.player_state_mut(player_id) {
            player.control.cursor_flash = 30;
        }
        record_cursor_state(context, player_id);
    });
    true
}

fn select_crew_host_impl(
    player_id: i32,
    target: ObjectId,
    select: bool,
    no_cursor_adjust: bool,
) -> bool {
    let valid_player = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .is_some_and(|context| context.player_state(player_id).is_some())
    });
    if !valid_player {
        return false;
    }
    if no_cursor_adjust {
        return if select {
            do_select_host_object(target, false)
        } else {
            unselect_host_object(target, false)
        };
    }

    let is_crew = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.player_state(player_id))
            .is_some_and(|player| player.crew.contains(&target))
    });
    // C4Player::SelectCrew returns early for a target outside Crew, while
    // FnSelectCrew itself still reports success.
    if !is_crew {
        return true;
    }
    if select {
        do_select_host_object(target, false);
    } else {
        unselect_host_object(target, false);
    }
    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return;
        };
        if let Some(player) = context.player_state_mut(player_id) {
            player.control.select_flash = 30;
            player.control.cursor_selection = 0;
            player.control.cursor_toggled = 0;
        }
    });
    adjust_cursor_host(player_id)
}

pub(crate) fn select_crew_host(args: &[Value]) -> Result<Value, RuntimeError> {
    let player_id = value_to_i32(args.first().unwrap_or(&Value::Nil), "SelectCrew", "player")?;
    let target = args
        .get(1)
        .map(|value| parse_object_reference_argument(value, "SelectCrew", "object"))
        .transpose()?
        .flatten();
    let select = value_to_bool(args.get(2).unwrap_or(&Value::Nil), "SelectCrew", "select")?;
    let no_cursor_adjust = value_to_bool(
        args.get(3).unwrap_or(&Value::Nil),
        "SelectCrew",
        "no cursor adjust",
    )?;
    Ok(Value::Bool(target.is_some_and(|target| {
        select_crew_host_impl(player_id, target, select, no_cursor_adjust)
    })))
}

/// FnSetCrewStatus (C4Script.cpp:2984-2993) delegates to
/// C4Player::SetObjectCrewStatus (C4Player.cpp:2107-2136). Crew membership
/// is an ordered per-player list independent of Owner; the return type is
/// C4ValueInt, so expose exact integer 0/1 rather than a script bool.
pub(crate) fn set_crew_status(args: &[Value]) -> Result<Value, RuntimeError> {
    let player_id = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "SetCrewStatus",
        "player",
    )?;
    let in_crew = value_to_bool(
        args.get(1).unwrap_or(&Value::Nil),
        "SetCrewStatus",
        "in crew",
    )?;
    let explicit = parse_object_reference_argument(
        args.get(2).unwrap_or(&Value::Nil),
        "SetCrewStatus",
        "object",
    )?;

    let target = HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        borrow
            .as_ref()
            .and_then(|context| explicit.or(context.script_object_context))
    });
    let Some(target) = target else {
        return Ok(Value::Int(0));
    };

    if in_crew {
        let added = HOST_CONTEXT.with(|cell| {
            let mut borrow = cell.borrow_mut();
            let Some(context) = borrow.as_mut() else {
                return None;
            };
            let Some(player) = context.player_state(player_id) else {
                return None;
            };
            // SetObjectCrewStatus's idempotent fast path has no controller,
            // view-range or Recruitment side effects.
            if player.crew.contains(&target) {
                return Some(false);
            }
            if !context.ensure_object_scope(target)
                || !context.object_status_present(target)
                || !context
                    .object_effective_definition_id(target)
                    .and_then(|definition| context.definition_metadata(&definition))
                    .is_some_and(|metadata| metadata.crew_member)
            {
                return None;
            }
            let view_range = context.object_scope_mut(target).map(|scope| {
                scope.set_crew_status_member(true);
                // MakeCrewMember(pObj, false): Controller changes; Owner and
                // Info do not (C4Player.cpp:1167-1215).
                scope.set_controller(player_id);
                scope.plr_view_range()
            });
            if view_range == Some(0) {
                context.set_object_plr_view_range(target, 500);
            } else if view_range.is_some() {
                context.actualize_object_plr_view_range(target);
            }
            if !context.insert_player_crew(player_id, target) {
                return None;
            }
            context.record_crew_rosters();
            Some(true)
        });
        let Some(added) = added else {
            return Ok(Value::Int(0));
        };

        if added {
            call_recruitment_callback(target, player_id);
            HOST_CONTEXT.with(|cell| {
                let mut borrow = cell.borrow_mut();
                if let Some(context) = borrow.as_mut() {
                    let still_member = context.object_in_any_crew(target);
                    if let Some(scope) = context.object_scope_mut(target) {
                        scope.set_crew_status_member(still_member);
                    }
                    context.record_crew_rosters();
                }
            });
        }
        return Ok(Value::Int(1));
    }

    let removal = HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return None;
        };
        let player = context.player_state(player_id)?;
        if !player.crew.contains(&target) {
            // Already absent is a successful side-effect-free no-op.
            return Some((false, false));
        }
        if let Some(player) = context.player_state_mut(player_id) {
            player.crew.retain(|member| *member != target);
        }
        context.record_crew_rosters();
        Some((
            true,
            context
                .object_scope(target)
                .map(ObjectScopeContext::selected)
                .or_else(|| {
                    context
                        .get_world_object(target)
                        .map(|object| object.selected)
                })
                .unwrap_or(false),
        ))
    });
    let Some((was_present, selected)) = removal else {
        return Ok(Value::Int(0));
    };
    if !was_present {
        return Ok(Value::Int(1));
    }

    // Crew.Remove happens before UnSelect; Info remains attached while the
    // fail-safe CrewSelection(true,false) callback runs.
    if selected {
        unselect_host_object(target, false);
    }

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return;
        };
        if !context.ensure_object_scope(target) {
            return;
        }
        // A live explicit None must NOT fall back to the immutable entry
        // snapshot: CrewSelection may have moved/cleared Info synchronously.
        let info_link = match context.object_scope(target) {
            Some(scope) => scope.info_link(),
            None => context.world.crew_info_link(target),
        };
        if info_link.is_some_and(|link| link.player_id == player_id) {
            let link = info_link.expect("checked above");
            if retire_host_crew_info(context, link) {
                if let Some(scope) = context.object_scope_mut(target) {
                    scope.set_info_rank(None);
                    scope.set_info_link(None);
                    scope.set_info_core(None);
                    scope.info_physical = None;
                    scope.record_physicals();
                }
                context.record_player_command(PlayerCommand::RetireCrewInfo {
                    object_id: target,
                    link,
                });
            }
        }
        let still_member = context.object_in_any_crew(target);
        if let Some(scope) = context.object_scope_mut(target) {
            scope.set_crew_status_member(still_member);
        }
        // A CrewSelection callback may have re-entered SetCrewStatus. The
        // outer removal preserves that final roster but still performs the
        // Info retirement above, matching C++ ordering.
        context.record_crew_rosters();
    });
    Ok(Value::Int(1))
}

pub(crate) fn unselect_crew_host(args: &[Value]) -> Result<Value, RuntimeError> {
    let player_id = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "UnselectCrew",
        "player",
    )?;
    let Some((crew, cursor)) = HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = borrow.as_ref()?;
        let player = context.player_state(player_id)?;
        Some((player.crew.clone(), player.cursor))
    }) else {
        return Ok(Value::Bool(false));
    };
    for &id in &crew {
        let present = HOST_CONTEXT.with(|cell| {
            cell.borrow()
                .as_ref()
                .is_some_and(|context| context.object_status_present(id))
        });
        if present {
            unselect_host_object(id, false);
        }
    }
    if let Some(cursor) = cursor.filter(|cursor| !crew.contains(cursor)) {
        unselect_host_object(cursor, false);
    }
    Ok(Value::Bool(true))
}

/// FnSetCursor (C4Script.cpp:2951-2958): set the player cursor (and
/// crew selection unless fNoSelectCrew).
pub(crate) fn set_cursor_host(args: &[Value]) -> Result<Value, RuntimeError> {
    let player_id = value_to_i32(args.first().unwrap_or(&Value::Nil), "SetCursor", "player")?;
    let object = args
        .get(1)
        .map(|arg| parse_object_reference_argument(arg, "SetCursor", "obj"))
        .transpose()?
        .flatten();
    let no_select_mark = value_to_bool(
        args.get(2).unwrap_or(&Value::Nil),
        "SetCursor",
        "no select mark",
    )?;
    let no_select_arrow = value_to_bool(
        args.get(3).unwrap_or(&Value::Nil),
        "SetCursor",
        "no select arrow",
    )?;
    let no_select_crew = value_to_bool(
        args.get(4).unwrap_or(&Value::Nil),
        "SetCursor",
        "no select crew",
    )?;
    let Some((previous, disabled)) = HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context = borrow.as_mut()?;
        if context.world.player(player_id).is_none() {
            return None;
        }
        if object.is_some_and(|id| !context.object_status_present(id)) {
            return None;
        }
        let previous = context.player_state(player_id)?.cursor;
        let disabled = object.is_some_and(|id| context.object_crew_disabled(id).unwrap_or(false));
        if !disabled {
            context.player_state_mut(player_id)?.cursor = object;
        }
        Some((previous, disabled))
    }) else {
        return Ok(Value::Bool(false));
    };

    if !disabled {
        if previous != object {
            if let Some(previous) = previous {
                unselect_host_object(previous, true);
            }
        }
        // Like C++ SetCursor, read the live Cursor after the old callback.
        let current = HOST_CONTEXT.with(|cell| {
            cell.borrow()
                .as_ref()
                .and_then(|context| context.player_state(player_id))
                .and_then(|player| player.cursor)
        });
        if let Some(current) = current {
            do_select_host_object(current, true);
        }
        HOST_CONTEXT.with(|cell| {
            let mut borrow = cell.borrow_mut();
            let Some(context) = borrow.as_mut() else {
                return;
            };
            if let Some(player) = context.player_state_mut(player_id) {
                if !no_select_arrow {
                    player.control.cursor_flash = 30;
                }
                if !no_select_mark {
                    player.control.select_flash = 30;
                }
            }
            record_cursor_state(context, player_id);
        });
    }

    if !no_select_crew {
        if let Some(object) = object {
            select_crew_host_impl(player_id, object, true, false);
        }
    }
    Ok(Value::Bool(true))
}

pub(crate) fn set_crew_enabled(args: &[Value]) -> Result<Value, RuntimeError> {
    let enabled = value_to_bool(
        args.first().unwrap_or(&Value::Nil),
        "SetCrewEnabled",
        "enabled",
    )?;
    let target = args
        .get(1)
        .map(|arg| parse_object_reference_argument(arg, "SetCrewEnabled", "obj"))
        .transpose()?
        .flatten();
    let active = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.object_context().map(|object| object.id()))
    });
    if let Some(target) = target {
        if Some(target) != active {
            return match call_world_object_function(
                target,
                "SetCrewEnabled",
                &[Value::Bool(enabled)],
            ) {
                Some(result) => result,
                None => Ok(Value::Bool(false)),
            };
        }
    }
    let adjust_owner = HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return None;
        };
        let Some((id, owner)) = context.object_context_mut().map(|object| {
            object.pending_update.crew_disabled = Some(!enabled);
            if !enabled {
                // FnSetCrewEnabled clears Select silently; only a subsequent
                // cursor adjustment may emit cursor callbacks
                // (C4Script.cpp:4814-4836).
                object.set_selected(false);
            }
            (object.id(), object.owner())
        }) else {
            return None;
        };
        (!enabled
            && context
                .player_state(owner)
                .is_some_and(|player| player.cursor == Some(id)))
        .then_some(owner)
    });
    if let Some(owner) = adjust_owner {
        adjust_cursor_host(owner);
    }
    Ok(Value::Bool(adjust_owner.is_some() || active.is_some()))
}

/// `C4Player::UpdateSelectionToggleStatus` before ObjectCommand routing
/// (C4Player.cpp:1355-1365). Cursor helpers already provide the synchronous
/// CrewSelection callbacks and copied-player preview; the final record after
/// clearing the two latches makes the authoritative fold retain that state.
pub(crate) fn update_player_selection_toggle_status_host(player_id: i32) {
    let Some((cursor_selection, cursor_toggled)) = HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = borrow.as_ref()?;
        let player = context.player_state(player_id)?;
        Some((
            player.control.cursor_selection,
            player.control.cursor_toggled,
        ))
    }) else {
        return;
    };
    if cursor_selection == 0 {
        return;
    }

    if cursor_toggled != 0 {
        adjust_cursor_host(player_id);
    } else {
        // C4Player::SelectSingleByCursor: UnselectCrew, DoSelect(Cursor),
        // SelectFlash=30, then AdjustCursorCommand.
        let _ = unselect_crew_host(&[Value::Int(player_id)]);
        let cursor = HOST_CONTEXT.with(|cell| {
            cell.borrow()
                .as_ref()
                .and_then(|context| context.player_state(player_id))
                .and_then(|player| player.cursor)
        });
        if let Some(cursor) = cursor {
            do_select_host_object(cursor, false);
        }
        HOST_CONTEXT.with(|cell| {
            if let Some(context) = cell.borrow_mut().as_mut() {
                if let Some(player) = context.player_state_mut(player_id) {
                    player.control.select_flash = 30;
                }
            }
        });
        adjust_cursor_host(player_id);
    }

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return;
        };
        if let Some(player) = context.player_state_mut(player_id) {
            player.control.cursor_selection = 0;
            player.control.cursor_toggled = 0;
        }
        record_cursor_state(context, player_id);
    });
}

/// FnCrewMember (C4Script.cpp:1311-1315): return the target's literal
/// signed DefCore CrewMember value. A nil target defaults only to cthr->Obj;
/// a definition-owned/global frame without an object returns nil.
pub(crate) fn crew_member(args: &[Value]) -> Result<Value, RuntimeError> {
    let target_id = parse_object_reference_argument(
        args.first().unwrap_or(&Value::Nil),
        "CrewMember",
        "target",
    )?;
    // C4Aul's typed one-parameter dispatch evaluates and discards surplus
    // arguments before entering the native function.

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let Some(context) = borrow.as_ref() else {
            return Ok(Value::Nil);
        };
        let Some(target) = target_id.or(context.script_object_context) else {
            return Ok(Value::Nil);
        };
        let Some(definition) = context.object_effective_definition_id(target) else {
            return Ok(Value::Nil);
        };
        Ok(context
            .definition_metadata(&definition)
            .map(|metadata| Value::Int(metadata.crew_member_value))
            .unwrap_or(Value::Nil))
    })
}

/// FnSetViewOffset (C4Script.cpp:5676-5687): ValidPlr gate followed by a
/// process-local first-physical-viewport lookup. An absent viewport is a
/// successful sync-safe no-op; the app resolves the ordered request.
pub(crate) fn set_view_offset(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 3 {
        return Err(RuntimeError::new(
            "SetViewOffset expects at most 3 arguments: player, x, y",
        ));
    }
    let player = parse_optional_i32(args.first(), "SetViewOffset", "player")?.unwrap_or(0);
    let x = parse_optional_i32(args.get(1), "SetViewOffset", "x")?.unwrap_or(0);
    let y = parse_optional_i32(args.get(2), "SetViewOffset", "y")?.unwrap_or(0);

    HOST_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(context) = borrow.as_mut() else {
            return Ok(Value::Bool(false));
        };
        if context.player_state(player).is_none() {
            return Ok(Value::Bool(false));
        }
        if context.world.film_viewport_available {
            context
                .world
                .viewport_presentation_requests
                .borrow_mut()
                .push(crate::ViewportPresentationRequest::SetViewOffset {
                    player,
                    offset: Vector2::new(x, y),
                });
        }
        Ok(Value::Bool(true))
    })
}
