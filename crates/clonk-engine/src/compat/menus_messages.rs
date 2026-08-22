use super::*;
use clonk_core::log_target::{SCRIPT_LOG_TARGET, SCRIPT_PROFILER_TARGET, SCRIPT_TRACE_TARGET};

/// `FnTestMessageBoard` (C4Script.cpp:3564-3573): invalid players return
/// nil. The ordinary multi-query availability probe is always true for a
/// valid player; the explicit in-use form reports whether any query node is
/// retained, including an answered query awaiting its control packet.
pub(crate) fn test_message_board(args: &[Value]) -> Result<Value, RuntimeError> {
    let player_id = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "TestMessageBoard",
        "player",
    )?;
    let test_if_in_use = value_to_bool(
        args.get(1).unwrap_or(&Value::Nil),
        "TestMessageBoard",
        "test if in use",
    )?;

    HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let player = borrow
            .as_ref()
            .and_then(|context| context.player_state(player_id));
        let Some(player) = player else {
            return Ok(Value::Nil);
        };
        Ok(Value::Bool(
            !test_if_in_use || !player.message_board_queries.is_empty(),
        ))
    })
}

/// `FnCallMessageBoard` (C4Script.cpp:3575-3585): register one player query,
/// replacing the first query for the same callback object and appending the
/// replacement at the list tail.
pub(crate) fn call_message_board(args: &[Value]) -> Result<Value, RuntimeError> {
    let explicit_target = parse_object_reference_argument(
        args.first().unwrap_or(&Value::Nil),
        "CallMessageBoard",
        "callback object",
    )?;
    let uppercase = value_to_bool(
        args.get(1).unwrap_or(&Value::Nil),
        "CallMessageBoard",
        "uppercase",
    )?;
    let prompt =
        parse_optional_string(args.get(2), "CallMessageBoard", "query")?.unwrap_or_default();
    let player_id = value_to_i32(
        args.get(3).unwrap_or(&Value::Nil),
        "CallMessageBoard",
        "player",
    )?;

    with_host_context_mut(Ok(Value::Bool(false)), |context| {
        let target = explicit_target.or(context.script_object_context);
        if target.is_some_and(|target| !context.object_status_present(target)) {
            return Ok(Value::Bool(false));
        }
        let query = crate::MessageBoardQuery::new(target, prompt, uppercase);
        let Some(player) = context.player_state_mut(player_id) else {
            return Ok(Value::Bool(false));
        };
        player.call_message_board(query.clone());
        context.record_player_command(PlayerCommand::CallMessageBoard { player_id, query });
        Ok(Value::Bool(true))
    })
}

/// `FnAbortMessageBoard` (C4Script.cpp:3587-3597). Local non-network
/// cancellation synchronously executes an empty answer before the outer
/// Remove call, so an active query closes/removes but the builtin returns
/// false; network cancellation defers that answer and returns the Remove
/// result.
pub(crate) fn abort_message_board(args: &[Value]) -> Result<Value, RuntimeError> {
    let explicit_target = parse_object_reference_argument(
        args.first().unwrap_or(&Value::Nil),
        "AbortMessageBoard",
        "callback object",
    )?;
    let player_id = value_to_i32(
        args.get(1).unwrap_or(&Value::Nil),
        "AbortMessageBoard",
        "player",
    )?;

    with_host_context_mut(Ok(Value::Bool(false)), |context| {
        let target = explicit_target.or(context.script_object_context);
        if context.player_state(player_id).is_none() {
            return Ok(Value::Bool(false));
        }
        let active_local_cancel = !context.world.network_game()
            && context
                .world
                .active_message_board_input()
                .is_some_and(|input| input.player == player_id && input.target == target);
        let removed = context
            .player_state_mut(player_id)
            .is_some_and(|player| player.remove_message_board_query(target));
        context.record_player_command(PlayerCommand::AbortMessageBoard { player_id, target });
        Ok(Value::Bool(removed && !active_local_cancel))
    })
}

/// `FnOnMessageBoardAnswer` (C4Script.cpp:3599-3613): consume exactly one
/// query before dispatching `InputCallback(answer, player)`. An omitted/null
/// answer only clears the query, while an explicit empty string is delivered.
pub(crate) fn on_message_board_answer(args: &[Value]) -> Result<Value, RuntimeError> {
    let target = parse_object_reference_argument(
        args.first().unwrap_or(&Value::Nil),
        "OnMessageBoardAnswer",
        "callback object",
    )?;
    let player_id = value_to_i32(
        args.get(1).unwrap_or(&Value::Nil),
        "OnMessageBoardAnswer",
        "player",
    )?;
    let answer = parse_optional_string_value(args.get(2), "OnMessageBoardAnswer", "answer")?;

    let removed = with_host_context_mut(false, |context| {
        let removed = context
            .player_state_mut(player_id)
            .is_some_and(|player| player.remove_message_board_query(target));
        if removed {
            context.record_player_command(PlayerCommand::RemoveMessageBoardQuery {
                player_id,
                target,
            });
        }
        removed
    });
    if !removed {
        return Ok(Value::Bool(false));
    }
    let Some(answer) = answer else {
        return Ok(Value::Bool(true));
    };

    let callback_args = [Value::String(answer), Value::Int(player_id)];
    let callback = match target {
        Some(target) => call_world_object_own_function(target, "InputCallback", &callback_args),
        None => {
            let script = HOST_CONTEXT.with(|cell| {
                cell.borrow()
                    .as_ref()
                    .and_then(|context| context.world.scenario_script().cloned())
            });
            script.and_then(|script| {
                call_scoped_script_function(script, "InputCallback", &callback_args)
            })
        }
    };
    match callback {
        Some(Ok(value)) => Ok(Value::Bool(value_raw_truthy(&value))),
        Some(Err(error)) => Err(error),
        None => Ok(Value::Bool(false)),
    }
}

/// `FnAddMsgBoardCmd` (C4Script.cpp:5198-5217): register a first-wins custom
/// chat command. Unnamed DirectExec/eval callers may only install the
/// Identifier-restricted form; ordinary named functions may install all
/// three restriction variants.
pub(crate) fn add_msg_board_cmd(args: &[Value]) -> Result<Value, RuntimeError> {
    // Native parameter conversion happens before the function body, so
    // validate all three slots before applying either early-return gate.
    let command = parse_optional_string(args.first(), "AddMsgBoardCmd", "command")?;
    let script = parse_optional_string(args.get(1), "AddMsgBoardCmd", "script")?;
    let raw_restriction = value_to_i32(
        args.get(2).unwrap_or(&Value::Nil),
        "AddMsgBoardCmd",
        "restriction",
    )?;

    let (Some(command), Some(script)) = (command, script) else {
        return Ok(Value::Bool(false));
    };
    if raw_restriction != 2 && clonk_script::caller_is_temporary_script() != Some(false) {
        return Ok(Value::Bool(false));
    }
    let restriction = match raw_restriction {
        0 => crate::MessageBoardCommandRestriction::Escaped,
        1 => crate::MessageBoardCommandRestriction::Plain,
        2 => crate::MessageBoardCommandRestriction::Identifier,
        _ => return Ok(Value::Bool(false)),
    };

    HOST_CONTEXT.with(|cell| {
        if let Some(context) = cell.borrow_mut().as_mut() {
            context.record_player_command(PlayerCommand::AddMessageBoardCommand {
                command: crate::InitialNetworkMessageBoardCommand {
                    name: command,
                    script,
                    restriction,
                },
            });
        }
    });
    Ok(Value::Bool(true))
}

/// FnActivateGameGoalMenu (C4Script.cpp:5953-5960): a missing player fails;
/// every valid peer evaluates the goals, while the embedding runtime decides
/// whether its locally controlled player may build the actual menu.
pub(crate) fn activate_game_goal_menu(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.len() > 1 {
        return Err(RuntimeError::new(
            "ActivateGameGoalMenu expects at most 1 argument: player",
        ));
    }
    let player_id = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "ActivateGameGoalMenu",
        "player",
    )?;
    with_host_context_mut(Ok(Value::Int(0)), |context| {
        if context.player_state(player_id).is_none() {
            return Ok(Value::Int(0));
        }
        let open_menu = context.world.local_players.contains(&player_id);
        context.record_player_command(PlayerCommand::ActivateGameGoalMenu {
            player_id,
            open_menu,
        });
        Ok(Value::Int(1))
    })
}

/// Marks `menu_object`'s menu as close-querying; false when a query is
/// already running (the C4ObjectMenu::CloseQuerying recursion check).
pub(crate) fn begin_menu_close_query(menu_object: ObjectId) -> bool {
    CLOSE_QUERYING.with(|cell| cell.borrow_mut().insert(menu_object))
}

pub(crate) fn end_menu_close_query(menu_object: ObjectId) {
    CLOSE_QUERYING.with(|cell| {
        cell.borrow_mut().remove(&menu_object);
    });
}

/// C4ObjectMenu::IsCloseDenied (C4ObjectMenu.cpp:56-75): a USER menu asks
/// MenuQueryCancel(Selection, ParentObject) on the command object
/// (CB_Object) or the scenario script (CB_Scenario); a truthy answer keeps
/// the menu open. The CloseQuerying flag stops recursive queries.
fn menu_close_denied(menu_object: ObjectId, menu: &crate::ObjectMenuState) -> bool {
    if !menu.user_menu {
        return false;
    }
    if !begin_menu_close_query(menu_object) {
        return false;
    }
    let pars = [
        Value::Int(menu.selection),
        object_reference_value(menu_object),
    ];
    // A missing function is a silent miss (the "~" in PSF_MenuQueryCancel);
    // callee errors fall back to close-OK (C4Object::Call, fPassErrors
    // defaults false — the error logs and the call yields C4VNull).
    let denied = if menu.scenario_callbacks {
        HOST_CONTEXT
            .with(|cell| {
                cell.borrow()
                    .as_ref()
                    .and_then(|context| context.world.scenario_script().cloned())
            })
            .filter(|script| script.has_local_function("MenuQueryCancel"))
            .and_then(|script| call_scoped_script_function(script, "MenuQueryCancel", &pars))
    } else if let Some(command_object) = menu.command_object {
        call_world_object_own_function(command_object, "MenuQueryCancel", &pars)
    } else {
        None
    }
    .map(|result| result.map(|value| value.as_bool()).unwrap_or(false))
    .unwrap_or(false);
    end_menu_close_query(menu_object);
    denied
}

/// C4Object::CloseMenu (C4Object.cpp:2009-2017): force skips the
/// MenuQueryCancel query (C4Menu::TryClose, C4Menu.cpp:317-320); a denied
/// soft close keeps the menu and fails.
pub(crate) fn close_object_menu(target: ObjectId, force: bool) -> bool {
    let menu = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.object_menu(target))
    });
    let Some(menu) = menu else {
        return true; // no menu -> close OK
    };
    if !force && menu_close_denied(target, &menu) {
        return false;
    }
    HOST_CONTEXT.with(|cell| {
        cell.borrow_mut()
            .as_mut()
            .map(|context| context.set_object_menu(target, None))
            .unwrap_or(false)
    })
}

/// FnCreateMenu (C4Script.cpp:1426-1459) → C4ObjectMenu::Init
/// (C4ObjectMenu.cpp:86-91): closes any old menu (soft — MenuQueryCancel
/// may deny), then installs a fresh user menu with Identification =
/// idMenuID ? idMenuID : iSymbol, the given style, and permanence.
pub(crate) fn create_menu(args: &[Value]) -> Result<Value, RuntimeError> {
    let symbol = parse_native_c4id_argument(args.first(), "CreateMenu")?
        .map(Value::C4Id)
        .unwrap_or(Value::Nil);
    let symbol_id = c4id_text_of(&symbol);
    let menu_target = parse_object_reference_argument(
        args.get(1).unwrap_or(&Value::Nil),
        "CreateMenu",
        "menu object",
    )?;
    let explicit_command = parse_object_reference_argument(
        args.get(2).unwrap_or(&Value::Nil),
        "CreateMenu",
        "command object",
    )?;
    let extra = parse_optional_i32(args.get(3), "CreateMenu", "extra")?.unwrap_or(0);
    let caption = parse_optional_string(args.get(4), "CreateMenu", "caption")?.unwrap_or_default();
    let extra_data = parse_optional_i32(args.get(5), "CreateMenu", "extra data")?.unwrap_or(0);
    let raw_style = parse_optional_i32(args.get(6), "CreateMenu", "style")?.unwrap_or(0);
    let style = raw_style & 127;
    let permanent = args.get(7).map(value_raw_truthy).unwrap_or(false);
    let menu_id = parse_native_c4id_argument(args.get(8), "CreateMenu")?
        .map(Value::C4Id)
        .unwrap_or(Value::Nil);

    let active = active_object_id();
    let Some(target) = menu_target.or(active) else {
        return Ok(Value::Bool(false)); // !pMenuObj && !cthr->Obj
    };
    let command_object = explicit_command.or(active);
    let scenario_callbacks = command_object.is_none();
    // Object menu: validate the command object (C4Script.cpp:1433-1436);
    // no command object is the scenario-script-callback form.
    if let Some(command_object) = command_object {
        let command_present = with_host_context(false, |context| {
            context.object_status_present(command_object)
        });
        if !command_present {
            return Ok(Value::Bool(false));
        }
    }
    // Clear any old menu (C4Script.cpp:1447): a MenuQueryCancel denial
    // aborts the new menu.
    if !close_object_menu(target, false) {
        return Ok(Value::Bool(false));
    }
    let identification = if value_raw_truthy(&menu_id) {
        menu_id
    } else {
        symbol
    };
    let menu = crate::ObjectMenuState {
        caption,
        symbol_id,
        title_symbol: crate::ObjectMenuSymbol::default(),
        identification,
        // Style & C4MN_Style_BaseMask (C4Menu::InitMenu, C4Menu.cpp:359).
        style,
        equal_item_height: raw_style & 128 != 0,
        permanent,
        location: None,
        runtime_id: crate::direct_com::next_object_menu_runtime_id(),
        extra: crate::ObjectMenuExtra::from_legacy(extra),
        extra_data,
        internal_refill_token: 0,
        selection: -1,
        user_menu: true,
        command_object,
        scenario_callbacks,
        refill_object: None,
        refill_object_contents_count: 0,
        location_reset_generation: 0,
        items: Vec::new(),
        // InitMenu immediately chooses five columns for Normal and one
        // for every other style (C4Menu.cpp:359-365); Lines stays at its
        // C4Menu::Default zero until layout/SetMenuSize.
        columns: if style == 0 { 5 } else { 1 },
        lines: 0,
        text_progressing: false,
        decoration: None,
    };
    let stored = HOST_CONTEXT.with(|cell| {
        cell.borrow_mut()
            .as_mut()
            .map(|context| context.set_object_menu(target, Some(menu)))
            .unwrap_or(false)
    });
    Ok(Value::Bool(stored))
}

/// FnGetMenu (C4Script.cpp:1418-1424): the active menu's Identification;
/// C4MN_None (0) without one; C4ID(-1) without an object.
pub(crate) fn get_menu(args: &[Value]) -> Result<Value, RuntimeError> {
    let target =
        parse_object_reference_argument(args.first().unwrap_or(&Value::Nil), "GetMenu", "obj")?;
    let Some(target) = target.or(active_object_id()) else {
        return Ok(Value::Int(-1));
    };
    let menu = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.object_menu(target))
    });
    Ok(menu
        .map(|menu| menu.identification)
        .unwrap_or(Value::Int(0)))
}

/// FnShowInfo (C4Script.cpp:3332-3336): open C4MN_Info on the calling
/// object, using the explicit object or the caller itself as target.
pub(crate) fn show_info(args: &[Value]) -> Result<Value, RuntimeError> {
    let Some(command_object) = active_object_id() else {
        return Ok(Value::Bool(false));
    };
    let explicit =
        parse_object_reference_argument(args.first().unwrap_or(&Value::Nil), "ShowInfo", "object")?;
    let target = explicit.unwrap_or(command_object);
    let queued = with_host_context_mut(false, |context| {
        if context.get_world_object(target).is_none() {
            return false;
        }
        let owner = context
            .get_world_object(command_object)
            .map(|object| object.owner)
            .unwrap_or(OWNER_NONE);
        context.pending_menu_requests.push(MenuRequest {
            crew_id: command_object,
            owner,
            kind: MenuRequestKind::Info { target },
        });
        true
    });
    Ok(Value::Bool(queued))
}

/// FnGetMenuSelection (C4Script.cpp:4310-4316): -1 without an object or an
/// active menu, else C4Menu::GetSelection() — the raw Selection index
/// (C4Menu.cpp:612-615; -1 while nothing is selected).
pub(crate) fn get_menu_selection(args: &[Value]) -> Result<Value, RuntimeError> {
    let target = parse_object_reference_argument(
        args.first().unwrap_or(&Value::Nil),
        "GetMenuSelection",
        "obj",
    )?;
    let Some(target) = target.or(active_object_id()) else {
        return Ok(Value::Int(-1));
    };
    let menu = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.object_menu(target))
    });
    Ok(Value::Int(menu.map(|menu| menu.selection).unwrap_or(-1)))
}

/// The literal-text half of the FnAddMenuItem sprintf (C4Script.cpp:
/// 1567-1570, fmt::sprintf(dummy, parameter, 0/1)): specifiers consume the
/// two arguments positionally — the parameter text first (its slot was
/// rewritten "%d" -> "%s"), then the left/right-click flag.
fn sprintf_menu_command(format: &str, parameter: &str, click: i32) -> String {
    let click_text = click.to_string();
    let mut arguments = [parameter, click_text.as_str()].into_iter();
    let mut out = String::with_capacity(format.len());
    let mut chars = format.chars().peekable();
    while let Some(current) = chars.next() {
        if current != '%' {
            out.push(current);
            continue;
        }
        match chars.peek().copied() {
            Some('%') => {
                chars.next();
                out.push('%');
            }
            Some('s') | Some('d') => {
                chars.next();
                out.push_str(arguments.next().unwrap_or(""));
            }
            _ => out.push('%'),
        }
    }
    out
}

fn menu_components_from_custom(values: Vec<Value>) -> Vec<crate::ObjectMenuComponent> {
    let mut components = Vec::<crate::ObjectMenuComponent>::new();
    let mut current_id: Option<String> = None;
    let mut current_count = 0_i32;
    let store = |components: &mut Vec<crate::ObjectMenuComponent>, id: String, count: i32| {
        if let Some(component) = components
            .iter_mut()
            .find(|component| component.definition_id == id)
        {
            component.count = count;
        } else {
            components.push(crate::ObjectMenuComponent {
                definition_id: id,
                count,
            });
        }
    };

    for value in values {
        let Value::C4Id(id) = value else {
            continue;
        };
        let raw = cast_c4id_payload(&id);
        if raw == 0 {
            continue;
        }
        let id = clonk_script::c4_id_from_raw(raw);
        if current_id.as_deref().is_some_and(|current| current != id) {
            store(
                &mut components,
                current_id.take().unwrap_or_default(),
                current_count,
            );
            current_count = 0;
        }
        current_id = Some(id);
        current_count = current_count.saturating_add(1);
    }
    if let Some(id) = current_id {
        store(&mut components, id, current_count);
    }
    components
}

/// `C4Def::GetComponents`' array-to-`C4IDList` conversion
/// (C4Def.cpp:1322-1355). The engine expects equal ids to be contiguous;
/// a later non-contiguous run overwrites the earlier count while retaining
/// the id's original list position through `SetIDCount(..., true)`.
pub(crate) fn component_list_from_custom_array(values: &[Value]) -> Vec<(String, i32)> {
    let mut components = Vec::<(String, i32)>::new();
    let mut last_id = String::new();
    let mut count = 0_i32;

    let store = |components: &mut Vec<(String, i32)>, id: &str, count: i32| {
        if id.is_empty() || count == 0 {
            return;
        }
        if let Some((_, stored_count)) = components.iter_mut().find(|(stored, _)| stored == id) {
            *stored_count = count;
        } else {
            components.push((id.to_owned(), count));
        }
    };

    for (index, value) in values.iter().enumerate() {
        let current_id = match value {
            Value::C4Id(id) if cast_c4id_payload(id) != 0 => {
                clonk_script::c4_id_from_raw(cast_c4id_payload(id))
            }
            Value::Int(raw @ 1..=9999) => format!("{raw:04}"),
            _ => continue,
        };
        // C4Def::GetComponents keys this flush off the ORIGINAL array
        // index. If the first valid ID follows an invalid slot it inserts a
        // leading C4ID_None entry; GetNeededMatStr/ComposeContents/Split then
        // stop at that sentinel before observing any later requirements.
        if index != 0 && last_id.is_empty() && count == 0 {
            return Vec::new();
        }
        if index != 0 && current_id != last_id {
            store(&mut components, &last_id, count);
            count = 0;
        }
        last_id = current_id;
        count = count.saturating_add(1);
    }
    store(&mut components, &last_id, count);
    components
}

/// FnAddMenuItem (C4Script.cpp:1471-1734): appends one item to the menu
/// object's OPEN menu. Sim-observable pieces ported: the composed
/// left/right-click commands (new-style %d sprintf vs old-style
/// "Fn(ID,param[,click][,value])"), the caption's %s -> def-name splice,
/// count/no-count, C4MN_Add_PassValue, selectability, and the
/// first-selectable selection grab (C4Menu::AddItem, C4Menu.cpp:424).
/// Symbols are presentation, but their argument CHECKS still gate the
/// return value (:1626,1679,1690-1693,1705-1709).
fn text_spec_image_known(context: &EffectHostContext, spec: &str) -> bool {
    match parse_text_spec(spec) {
        Some(TextSpec::Definition { id, .. }) => context.definition_metadata(id).is_some(),
        Some(TextSpec::Portrait {
            definition_id,
            portrait_name,
            ..
        }) => context
            .definition_metadata(definition_id)
            .is_some_and(|metadata| {
                metadata
                    .portrait_names
                    .iter()
                    .any(|name| name.eq_ignore_ascii_case(portrait_name))
            }),
        Some(TextSpec::Icon(_)) => true,
        None => false,
    }
}

pub(crate) fn add_menu_item(args: &[Value]) -> Result<Value, RuntimeError> {
    let caption_arg = parse_optional_string(args.first(), "AddMenuItem", "caption")?;
    let command_arg =
        parse_optional_string(args.get(1), "AddMenuItem", "command")?.unwrap_or_default();
    let item_id = parse_native_c4id_argument(args.get(2), "AddMenuItem")?;
    let item_id_raw = item_id
        .as_deref()
        .map(cast_c4id_payload)
        .unwrap_or_default();
    // C4MenuItem stores the typed C4ID payload. C4IdText is only used below
    // while composing executable source; using it for storage aliases e.g.
    // packed b"1111" with the distinct numeric C4ID 1111.
    let stored_item_id = clonk_script::c4_id_from_raw(item_id_raw);
    let menu_target = parse_object_reference_argument(
        args.get(3).unwrap_or(&Value::Nil),
        "AddMenuItem",
        "menu object",
    )?;
    let mut count = parse_optional_i32(args.get(4), "AddMenuItem", "count")?.unwrap_or(0);
    let parameter = args.get(5).cloned().unwrap_or(Value::Nil);
    let mut info_caption =
        parse_optional_string(args.get(6), "AddMenuItem", "info caption")?.unwrap_or_default();
    let extra = parse_optional_i32(args.get(7), "AddMenuItem", "extra")?.unwrap_or(0);
    let xpar = args.get(8).cloned().unwrap_or(Value::Nil);
    let xpar2 = args.get(9).cloned().unwrap_or(Value::Nil);

    let Some(target) = menu_target.or(active_object_id()) else {
        return Ok(Value::Bool(false)); // !pMenuObj (C4Script.cpp:1474)
    };
    let (
        menu,
        presentation_definition_id,
        def_name,
        def_description,
        static_components,
        component_script,
    ) = with_host_context(
        (None, None, String::new(), String::new(), Vec::new(), None),
        |context| {
            // pDef = C4Id2Def(idItem), falling back to the menu object's own
            // def (C4Script.cpp:1488-1489).
            let item_definition_id = (item_id_raw != 0).then(|| stored_item_id.clone());
            let item_metadata = item_definition_id
                .as_deref()
                .and_then(|id| context.definition_metadata(id));
            let presentation_definition_id = item_metadata
                .and(item_definition_id.clone())
                .or_else(|| context.object_effective_definition_id(target));
            let def_name = presentation_definition_id
                .as_deref()
                .and_then(|id| context.definition_metadata(id))
                .map(|metadata| metadata.name.clone())
                .unwrap_or_default();
            let def_description = presentation_definition_id
                .as_deref()
                .and_then(|id| context.world.definition_description(id))
                .unwrap_or_default()
                .to_string();
            let static_components = item_metadata
                .map(|metadata| {
                    metadata
                        .components
                        .iter()
                        .map(|(definition_id, count)| crate::ObjectMenuComponent {
                            definition_id: definition_id.clone(),
                            count: *count,
                        })
                        .collect()
                })
                .unwrap_or_default();
            let component_script = item_definition_id
                .as_deref()
                .filter(|_| item_metadata.is_some())
                .and_then(|id| context.world.definition_script(id))
                .cloned();
            (
                context.object_menu(target),
                presentation_definition_id,
                def_name,
                def_description,
                static_components,
                component_script,
            )
        },
    );
    let Some(mut menu) = menu else {
        return Ok(Value::Bool(false)); // !pMenuObj->Menu (C4Script.cpp:1475)
    };
    let picture_symbol_size = if menu.style == 3 { 64 } else { 35 };

    // Compose the caption with the def name (C4Script.cpp:1492-1510).
    let mut caption = caption_arg
        .as_deref()
        .map(|text| text.replacen("%s", &def_name, 1))
        .unwrap_or_default();
    if info_caption.is_empty() && extra & 512 == 0 {
        info_caption = def_description;
    }
    info_caption = crate::normalize_menu_info_caption(info_caption);

    // Typed parameter -> command text (C4Script.cpp:1513-1546).
    let parameter_text = match &parameter {
        Value::Int(value) => value.to_string(),
        Value::Bool(flag) => if *flag { "true" } else { "false" }.to_string(),
        Value::RawBool(raw) => ((*raw as u32 as i32) != 0).to_string(),
        Value::C4Id(_) => c4id_text_of(&parameter),
        Value::Object(number) => format!("Object({number})"),
        Value::String(text) => format!("\"{text}\""),
        Value::Nil => "CastAny(0)".to_string(), // C4V_Any raw 0
        Value::Array(_) => {
            return Err(RuntimeError::new("array as parameter to AddMenuItem"));
        }
        Value::Proplist(_) => {
            return Err(RuntimeError::new("map as parameter to AddMenuItem"));
        }
    };

    // C4MN_Add_PassValue payload (C4Script.cpp:1549-1554).
    let own_value = (extra & 128 != 0).then(|| xpar2.as_c4_int().unwrap_or(0));

    // New style (any non-IsIdentifier char, C4Strings.cpp:36-45) vs old
    // style command composition (C4Script.cpp:1556-1597).
    let is_identifier = |c: char| c.is_ascii_alphanumeric() || matches!(c, '_' | '~' | '+' | '-');
    let (command, command2) = if command_arg.chars().any(|c| !is_identifier(c)) {
        let dummy = command_arg.replacen("%d", "%s", 1);
        (
            sprintf_menu_command(&dummy, &parameter_text, 0),
            sprintf_menu_command(&dummy, &parameter_text, 1),
        )
    } else if !command_arg.is_empty() {
        let id_text = clonk_script::c4_id_text(&stored_item_id);
        match own_value {
            Some(value) => (
                format!("{command_arg}({id_text},{parameter_text},0,{value})"),
                format!("{command_arg}({id_text},{parameter_text},1,{value})"),
            ),
            None => (
                format!("{command_arg}({id_text},{parameter_text})"),
                format!("{command_arg}({id_text},{parameter_text},1)"),
            ),
        }
    } else {
        (String::new(), String::new())
    };

    // Preserve the exact C4MN_Add_Img* recipe. Dialog portraits depend on
    // the pre-clear TextSpec caption surviving as an image source.
    let image = match extra & 127 {
        1 => {
            let rank = count;
            count = 0;
            crate::ObjectMenuImage::Rank { rank }
        }
        2 => crate::ObjectMenuImage::Indexed {
            index: xpar.as_c4_int().unwrap_or(0),
        },
        3 | 4 => {
            let Value::Object(number) = xpar else {
                return Ok(Value::Bool(false));
            };
            let object = ObjectId::new(number);
            if extra & 127 == 3 {
                crate::ObjectMenuImage::ObjectRank { object }
            } else {
                crate::ObjectMenuImage::Object { object }
            }
        }
        5 => {
            let Some(_) = caption_arg else {
                return Ok(Value::Bool(false));
            };
            let spec = caption.clone();
            let known = HOST_CONTEXT.with(|cell| {
                cell.borrow()
                    .as_ref()
                    .is_some_and(|context| text_spec_image_known(context, &spec))
            });
            if !known {
                return Ok(Value::Bool(false));
            }
            caption.clear();
            let raw_color = xpar.as_c4_int().unwrap_or(0) as u32;
            crate::ObjectMenuImage::TextSpec {
                spec,
                color: if raw_color == 0 { 0xff } else { raw_color },
            }
        }
        6 => crate::ObjectMenuImage::Color {
            color: xpar.as_c4_int().unwrap_or(0) as u32,
        },
        7 => {
            if extra & 128 != 0 {
                return Err(RuntimeError::new(
                    "AddMenuItem: C4MN_Add_ImgIndexedColor can not be used together with C4MN_Add_PassValue!",
                ));
            }
            crate::ObjectMenuImage::IndexedColor {
                index: xpar.as_c4_int().unwrap_or(0),
                color: xpar2.as_c4_int().unwrap_or(0) as u32,
            }
        }
        _ if item_id_raw == 0 => crate::ObjectMenuImage::None,
        _ => crate::ObjectMenuImage::Definition,
    };
    let picture_snapshot = match &image {
        crate::ObjectMenuImage::ObjectRank { object } => HOST_CONTEXT.with(|cell| {
            cell.borrow().as_ref().and_then(|context| {
                context.object_menu_picture_snapshot(*object, true, picture_symbol_size)
            })
        }),
        crate::ObjectMenuImage::Object { object } => HOST_CONTEXT.with(|cell| {
            cell.borrow().as_ref().and_then(|context| {
                context.object_menu_picture_snapshot(*object, false, picture_symbol_size)
            })
        }),
        _ => None,
    };

    // Zero count -> no count unless C4MN_Add_ForceCount (C4Script.cpp:1726).
    if count == 0 && extra & 256 == 0 {
        count = 12_345_678; // C4MN_Item_NoCount
    }
    let selectable = !command.is_empty();
    let components = match component_script.and_then(|script| {
        call_scoped_script_function(
            script,
            "GetCustomComponents",
            &[object_reference_value(target)],
        )
    }) {
        Some(result) => match result? {
            Value::Array(values) => menu_components_from_custom(values),
            _ => static_components,
        },
        None => static_components,
    };
    // First selectable item takes the selection, WITHOUT callbacks
    // (C4Menu::AddItem -> SetSelection(ItemCount-1, false, false)).
    if menu.internal_refill_token == 0 && menu.selection == -1 && selectable {
        menu.selection = menu.items.len() as i32;
    }
    menu.items.push(crate::ObjectMenuItem {
        caption,
        info_caption,
        command,
        command2,
        count,
        item_id: stored_item_id,
        symbol: crate::ObjectMenuSymbol::default(),
        image,
        presentation_definition_id,
        picture_snapshot,
        picture_object: None,
        components,
        selectable,
        value: own_value,
        text_display_progress: if menu.text_progressing { 0 } else { -1 },
    });
    let stored = HOST_CONTEXT.with(|cell| {
        cell.borrow_mut()
            .as_mut()
            .map(|context| context.set_object_menu(target, Some(menu)))
            .unwrap_or(false)
    });
    Ok(Value::Bool(stored))
}

/// C4ObjectMenu::OnSelectionChanged (C4ObjectMenu.cpp:93-104): user menus
/// fire OnMenuSelection(iNewSelection, ParentObject) on the command object
/// (CB_Object) or the scenario script (CB_Scenario); the result is
/// discarded, a missing function is a silent miss (PSF_MenuSelection is
/// "~"-prefixed), and callee errors log-and-continue (fPassErrors=false).
fn menu_selection_changed(menu_object: ObjectId, menu: &crate::ObjectMenuState) {
    if !menu.user_menu {
        return;
    }
    let pars = [
        Value::Int(menu.selection),
        object_reference_value(menu_object),
    ];
    let result = if menu.scenario_callbacks {
        HOST_CONTEXT
            .with(|cell| {
                cell.borrow()
                    .as_ref()
                    .and_then(|context| context.world.scenario_script().cloned())
            })
            .filter(|script| script.has_local_function("OnMenuSelection"))
            .and_then(|script| call_scoped_script_function(script, "OnMenuSelection", &pars))
    } else if let Some(command_object) = menu.command_object {
        call_world_object_own_function(command_object, "OnMenuSelection", &pars)
    } else {
        None
    };
    if let Some(Err(error)) = result {
        tracing::error!(
            %error,
            "script error in OnMenuSelection; continuing like the C++ fail-safe exec"
        );
        log_runtime_call_frames("", error.call_frames());
    }
}

/// FnSelectMenuItem (C4Script.cpp:1736-1741) → C4Menu::SetSelection
/// (C4Menu.cpp:557-594): moves the selection only onto SELECTABLE items
/// (or clears it on -1 in an empty menu), returns true whenever a menu is
/// active, and always runs the selection callback with the FINAL selection
/// (fDoCalls=true).
pub(crate) fn select_menu_item(args: &[Value]) -> Result<Value, RuntimeError> {
    let item = parse_optional_i32(args.first(), "SelectMenuItem", "item")?.unwrap_or(0);
    let target = parse_object_reference_argument(
        args.get(1).unwrap_or(&Value::Nil),
        "SelectMenuItem",
        "menu object",
    )?;
    let Some(target) = target.or(active_object_id()) else {
        return Ok(Value::Bool(false)); // !pMenuObj (C4Script.cpp:1738)
    };
    let menu = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.object_menu(target))
    });
    let Some(mut menu) = menu else {
        return Ok(Value::Bool(false)); // !pMenuObj->Menu (C4Script.cpp:1739)
    };
    let selectable = usize::try_from(item)
        .ok()
        .and_then(|index| menu.items.get(index))
        .map(|entry| entry.selectable)
        .unwrap_or(false);
    if (item == -1 && menu.items.is_empty()) || selectable {
        menu.selection = item;
    }
    let stored = HOST_CONTEXT.with(|cell| {
        cell.borrow_mut()
            .as_mut()
            .map(|context| context.set_object_menu(target, Some(menu.clone())))
            .unwrap_or(false)
    });
    if stored {
        menu_selection_changed(target, &menu);
    }
    Ok(Value::Bool(true))
}

/// FnClearMenuItems (C4Script.cpp:5149-5159) -> C4Menu::ClearItems(true)
/// (C4Menu.cpp:975-988): delete every item, reset the selection, and keep
/// the menu open. The `true` is fResetSelection; SetSelection receives
/// fDoCalls=false, so OnMenuSelection deliberately does not run.
pub(crate) fn clear_menu_items(args: &[Value]) -> Result<Value, RuntimeError> {
    let target = parse_object_reference_argument(
        args.first().unwrap_or(&Value::Nil),
        "ClearMenuItems",
        "obj",
    )?;
    let Some(target) = target.or(active_object_id()) else {
        return Ok(Value::Bool(false));
    };
    let menu = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.object_menu(target))
    });
    let Some(mut menu) = menu else {
        return Ok(Value::Bool(false));
    };
    menu.items.clear();
    menu.selection = -1;
    // ClearMenuItems calls C4Menu::ClearItems(true), which clears
    // LocationSet for every style even when a later AddMenuItem restores the
    // old final count (C4Script.cpp:5149-5159; C4Menu.cpp:975-987).
    menu.mark_location_reset();
    let stored = HOST_CONTEXT.with(|cell| {
        cell.borrow_mut()
            .as_mut()
            .map(|context| context.set_object_menu(target, Some(menu)))
            .unwrap_or(false)
    });
    Ok(Value::Bool(stored))
}

/// FnCloseMenu (C4Script.cpp:4309-4314): pObj->CloseMenu(true) — the
/// forced close never asks MenuQueryCancel and always reports success.
pub(crate) fn close_menu(args: &[Value]) -> Result<Value, RuntimeError> {
    let target =
        parse_object_reference_argument(args.first().unwrap_or(&Value::Nil), "CloseMenu", "obj")?;
    let Some(target) = target.or(active_object_id()) else {
        return Ok(Value::Bool(false));
    };
    Ok(Value::Bool(close_object_menu(target, true)))
}

/// FnSetMenuSize (C4Script.cpp:4483-4492): false without an active menu;
/// cols/rows clamp through BoundBy(0..50) into C4Menu::SetSize
/// (C4Menu.cpp:635-640), where a ZERO axis keeps the previous value. The
/// stored Columns/Lines drive the menu layout (presentation) — the
/// sim-observable pieces are this state and the bool return.
pub(crate) fn set_menu_size(args: &[Value]) -> Result<Value, RuntimeError> {
    let cols = parse_optional_i32(args.first(), "SetMenuSize", "cols")?.unwrap_or(0);
    let rows = parse_optional_i32(args.get(1), "SetMenuSize", "rows")?.unwrap_or(0);
    let target =
        parse_object_reference_argument(args.get(2).unwrap_or(&Value::Nil), "SetMenuSize", "obj")?;
    let Some(target) = target.or(active_object_id()) else {
        return Ok(Value::Bool(false)); // !pObj (C4Script.cpp:4486)
    };
    let menu = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.object_menu(target))
    });
    let Some(mut menu) = menu else {
        return Ok(Value::Bool(false)); // !pMnu || !IsActive (C4Script.cpp:4489)
    };
    let cols = cols.clamp(0, 50);
    let rows = rows.clamp(0, 50);
    if cols != 0 {
        menu.columns = cols;
    }
    if rows != 0 {
        menu.lines = rows;
    }
    HOST_CONTEXT.with(|cell| {
        cell.borrow_mut()
            .as_mut()
            .map(|context| context.set_object_menu(target, Some(menu)))
    });
    Ok(Value::Bool(true))
}

/// FnSetMenuDecoration (C4Script.cpp:1737-1748): NO cthr->Obj fallback —
/// a nil menu object fails even with a scope object. The deco def must be
/// known (FrameDecoration::SetByDef fails on C4Id2Def null,
/// C4GuiDialogs.cpp:113-114). SetByDef snapshots five definition callbacks
/// and eight ActMap facets immediately.
fn build_frame_decoration_snapshot(deco_id: &str) -> Option<crate::ObjectMenuFrameDecoration> {
    let (metadata, script) = HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = borrow.as_ref()?;
        Some((
            context.definition_metadata(deco_id).cloned()?,
            context.world.definition_script(deco_id).cloned()?,
        ))
    })?;
    let query = |suffix: &str| {
        let function = format!("FrameDecoration{suffix}");
        match call_scoped_script_function(Arc::clone(&script), &function, &[]) {
            Some(Ok(value)) => value.as_c4_int().unwrap_or(0),
            Some(Err(error)) => {
                tracing::error!(%error, function, "frame-decoration callback failed; using zero");
                log_runtime_call_frames("", error.call_frames());
                0
            }
            None => 0,
        }
    };
    let facet = |suffix: &str| {
        metadata
            .action_graphics
            .get(&format!("FrameDeco{suffix}"))
            .and_then(|graphics| graphics.facet.clone())
    };
    Some(crate::ObjectMenuFrameDecoration {
        source_definition: deco_id.to_string(),
        background_color: query("BackClr") as u32,
        border_top: query("BorderTop"),
        border_left: query("BorderLeft"),
        border_right: query("BorderRight"),
        border_bottom: query("BorderBottom"),
        top: facet("Top"),
        top_right: facet("TopRight"),
        right: facet("Right"),
        bottom_right: facet("BottomRight"),
        bottom: facet("Bottom"),
        bottom_left: facet("BottomLeft"),
        left: facet("Left"),
        top_left: facet("TopLeft"),
    })
}

pub(crate) fn set_menu_decoration(args: &[Value]) -> Result<Value, RuntimeError> {
    let Some(deco_id) = parse_custom_message_decoration(args.first(), "SetMenuDecoration")? else {
        return Ok(Value::Bool(false));
    };
    let target = parse_object_reference_argument(
        args.get(1).unwrap_or(&Value::Nil),
        "SetMenuDecoration",
        "menu object",
    )?;
    let Some(target) = target else {
        return Ok(Value::Bool(false)); // !pMenuObj (C4Script.cpp:1739)
    };
    let menu = with_host_context(None, |context| context.object_menu(target));
    let Some(mut menu) = menu else {
        return Ok(Value::Bool(false)); // !pMenuObj->Menu (C4Script.cpp:1739)
    };
    let Some(decoration) = build_frame_decoration_snapshot(&deco_id) else {
        return Ok(Value::Bool(false)); // SetByDef failed (C4Script.cpp:1741-1745)
    };
    menu.decoration = Some(decoration);
    HOST_CONTEXT.with(|cell| {
        cell.borrow_mut()
            .as_mut()
            .map(|context| context.set_object_menu(target, Some(menu)))
    });
    Ok(Value::Bool(true))
}

/// FnSetMenuTextProgress (C4Script.cpp:1750-1754): NO cthr->Obj fallback —
/// a nil menu object fails even with a scope object. With an active menu
/// C4Menu::SetTextProgress(n, fAdd=false) (C4Menu.cpp:1079-1111)
/// distributes the shared byte budget across each non-portrait row.
pub(crate) fn set_menu_text_progress(args: &[Value]) -> Result<Value, RuntimeError> {
    let progress =
        parse_optional_i32(args.first(), "SetMenuTextProgress", "progress")?.unwrap_or(0);
    let target = parse_object_reference_argument(
        args.get(1).unwrap_or(&Value::Nil),
        "SetMenuTextProgress",
        "menu object",
    )?;
    let Some(target) = target else {
        return Ok(Value::Bool(false)); // !pMenuObj (C4Script.cpp:1752)
    };
    let menu = HOST_CONTEXT.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|context| context.object_menu(target))
    });
    let Some(mut menu) = menu else {
        return Ok(Value::Bool(false)); // !pMenuObj->Menu / !IsActive
    };
    let _ = menu.set_text_progress(progress, false);
    HOST_CONTEXT.with(|cell| {
        cell.borrow_mut()
            .as_mut()
            .map(|context| context.set_object_menu(target, Some(menu)))
    });
    Ok(Value::Bool(true))
}

pub(crate) const LEGACY_DEFAULT_MESSAGE_COLOR: u32 = 0x00ff_ffff;

fn extract_speech_segment(raw: &str) -> Option<String> {
    let mut segments = raw.splitn(3, '$');
    segments.next()?;
    segments.next().map(|segment| segment.to_string())
}

fn extract_message_text(formatted: &str) -> String {
    formatted.split('$').next().unwrap_or("").to_string()
}

#[allow(clippy::too_many_arguments)]
fn message_fallback_spec(
    context: &EffectHostContext,
    function: &str,
    raw_message: &str,
    format_args: &[Value],
    kind: MessageKind,
    target: Option<ObjectId>,
    player: Option<i32>,
) -> Result<MessageSpec, RuntimeError> {
    let formatted =
        format_script_string_with_context(function, raw_message, format_args, Some(context))?;
    Ok(MessageSpec::new(kind, extract_message_text(&formatted))
        .with_target(target)
        .with_player(player)
        .with_color(invert_rgba_alpha(LEGACY_DEFAULT_MESSAGE_COLOR)))
}

/// Convert the native `C4ID idDeco` parameter of `FnCustomMessage`
/// (C4Script.cpp:5995). C++ accepts nil/falsy zero, direct C4ID values, and
/// integer IDs in `0..=9999`; String -> C4ID is always invalid
/// (C4Value.cpp:469-478,550-561).
fn parse_custom_message_decoration(
    value: Option<&Value>,
    function: &str,
) -> Result<Option<String>, RuntimeError> {
    parse_native_c4id_argument(value, function)
}

pub(crate) fn custom_message(args: &[Value]) -> Result<Value, RuntimeError> {
    let message = match args.first().unwrap_or(&Value::Nil) {
        Value::String(text) => Some(text.as_ref().to_owned()),
        Value::Nil => None,
        other => {
            return Err(RuntimeError::new(format!(
                "CustomMessage: expected string for message, got {}",
                other.type_name()
            )));
        }
    };

    let target = if let Some(arg) = args.get(1) {
        parse_object_reference_argument(arg, "CustomMessage", "target")?
    } else {
        None
    };

    // C4ValueInt parameter conversion maps both an unfilled slot and explicit
    // nil to integer zero. Only an explicit -1 is NO_OWNER
    // (C4Script.cpp:5995-6033; C4AulExec.cpp:1364-1396).
    let owner = value_to_i32(args.get(2).unwrap_or(&Value::Nil), "CustomMessage", "owner")?;

    let offset_x = match args.get(3) {
        Some(Value::Nil) | None => 0,
        Some(value) => value_to_i32(value, "CustomMessage", "x")?,
    };

    let offset_y = match args.get(4) {
        Some(Value::Nil) | None => 0,
        Some(value) => value_to_i32(value, "CustomMessage", "y")?,
    };

    let raw_color =
        parse_native_optional_i32(args.get(5), "CustomMessage", "color")?.map(|color| color as u32);

    let decoration = parse_custom_message_decoration(args.get(6), "CustomMessage")?;

    let portrait = match args.get(7) {
        Some(Value::Nil | Value::Int(0) | Value::Bool(false)) | None => None,
        Some(Value::String(name)) if !name.is_empty() => Some(name.as_ref().to_owned()),
        Some(other) => {
            return Err(RuntimeError::new(format!(
                "CustomMessage: expected string or nil for portrait, got {}",
                other.type_name()
            )));
        }
    };

    let flags = match args.get(8) {
        Some(Value::Nil) | None => 0,
        Some(value) => value_to_i32(value, "CustomMessage", "flags")? as u32,
    };

    let width = match args.get(9) {
        Some(Value::Nil) | None => None,
        Some(value) => Some(value_to_i32(value, "CustomMessage", "width")?),
    };

    let Some(message) = message else {
        return Ok(Value::Bool(false));
    };
    if let Some(id) = decoration.as_deref() {
        let known = HOST_CONTEXT.with(|cell| {
            let borrow = cell.borrow();
            let context = borrow.as_ref().ok_or_else(|| {
                RuntimeError::new("CustomMessage requires an active engine context")
            })?;
            Ok(context.world.definition_known(id))
        })?;
        // `FnCustomMessage` returns false before creating a message when
        // `idDeco && !C4Id2Def(idDeco)` (C4Script.cpp:6002).
        if known == Some(false) {
            return Ok(Value::Bool(false));
        }
    }
    ensure_single_flag(
        flags,
        HORIZONTAL_POSITION_FLAGS,
        "CustomMessage: Only one horizontal positioning flag allowed!",
    )?;
    ensure_single_flag(
        flags,
        VERTICAL_POSITION_FLAGS,
        "CustomMessage: Only one vertical positioning flag allowed!",
    )?;
    ensure_single_flag(
        flags,
        ALIGNMENT_FLAGS,
        "CustomMessage: Only one text alignment flag allowed!",
    )?;

    let color = invert_rgba_alpha(raw_color.unwrap_or(0x00ff_ffff));
    let kind = if target.is_some() {
        if owner != OWNER_NONE {
            MessageKind::TargetPlayer
        } else {
            MessageKind::Target
        }
    } else if owner != OWNER_NONE {
        MessageKind::GlobalPlayer
    } else {
        MessageKind::Global
    };

    let player = if owner == OWNER_NONE {
        None
    } else {
        Some(owner)
    };

    // C4GameMessageList::New returns after its clear operation for an empty
    // message, before C4GameMessage::Init constructs the FrameDecoration
    // (C4GameMessage.cpp:290-310). Snapshot SetByDef only when Init would run.
    let creates_message = if flags & FLAG_DROP_SPEECH != 0 {
        !message.split('$').next().unwrap_or("").is_empty()
    } else {
        !message.is_empty()
    };
    let frame_decoration = if creates_message {
        decoration
            .as_deref()
            .and_then(build_frame_decoration_snapshot)
    } else {
        None
    };

    let spec = MessageSpec {
        kind,
        text: message,
        target,
        player,
        offset: Vector2::new(offset_x, offset_y),
        color,
        flags,
        width,
        decoration,
        frame_decoration,
        portrait,
    };

    try_with_host_context_mut(
        "CustomMessage requires an active engine context",
        |context| {
            context.register_message(MessageCommand::Add(spec));
            Ok(Value::Bool(true))
        },
    )
}

enum LogLevel {
    Info,
    Debug,
}

fn log_internal(function: &str, args: &[Value], level: LogLevel) -> Result<Value, RuntimeError> {
    let format_str = match args.first().unwrap_or(&Value::Nil) {
        Value::String(text) => text.as_ref().to_owned(),
        Value::Nil => String::new(),
        other => {
            return Err(RuntimeError::new(format!(
                "{function}: expected string for message, got {}",
                other.type_name()
            )));
        }
    };

    let format_args = if args.len() > 1 { &args[1..] } else { &[] };
    let formatted = format_script_string(function, &format_str, format_args)?;

    match level {
        LogLevel::Info => info!(target: SCRIPT_LOG_TARGET, "{}", formatted),
        LogLevel::Debug => {
            debug!(target: clonk_core::log_target::SCRIPT_DEBUG_LOG_TARGET, "{}", formatted)
        }
    }

    Ok(Value::Bool(true))
}

pub(crate) fn log_message(args: &[Value]) -> Result<Value, RuntimeError> {
    log_internal("Log", args, LogLevel::Info)?;
    Ok(Value::Nil)
}

pub(crate) fn debug_log_message(args: &[Value]) -> Result<Value, RuntimeError> {
    log_internal("DebugLog", args, LogLevel::Debug)?;
    Ok(Value::Nil)
}

/// `FnFatalError` (C4Script.cpp:5962-5965): throw a user-framed script
/// execution error. A missing or nil native string pointer uses the C++
/// fallback text.
pub(crate) fn fatal_error(args: &[Value]) -> Result<Value, RuntimeError> {
    let message = parse_native_c4_string_argument(args.first(), "FatalError", "message")?;
    Err(RuntimeError::new(format!(
        "User error: {}",
        message.as_deref().unwrap_or("(no error)")
    )))
}

/// FnStartCallTrace (C4Script.cpp:5967-5971). The execution-local controller
/// is independent of immutable debugger hooks, so an in-flight native call
/// can arm tracing for the remainder of its caller's script frame.
pub(crate) fn start_call_trace(_args: &[Value]) -> Result<Value, RuntimeError> {
    if clonk_script::caller_host_identity().is_some() {
        clonk_script::start_call_trace(|message| {
            info!(target: SCRIPT_TRACE_TARGET, "{message}");
        });
    }
    Ok(Value::Nil)
}

/// FnStartScriptProfiler/FnStopScriptProfiler (C4Script.cpp:5973-5993).
pub(crate) fn start_script_profiler(args: &[Value]) -> Result<Value, RuntimeError> {
    let definition = parse_native_c4id_argument(args.first(), "StartScriptProfiler")?;
    let target = match definition.as_deref() {
        None => None,
        Some(definition) => {
            let script = HOST_CONTEXT.with(|cell| {
                cell.borrow()
                    .as_ref()
                    .and_then(|context| context.world.definition_script(definition).cloned())
            });
            let Some(script) = script else {
                // C4Id2Def failure leaves any active profiler run untouched.
                return Ok(Value::Bool(false));
            };
            Some(script.host_identity())
        }
    };
    clonk_script::start_script_profiler(target);
    Ok(Value::Bool(true))
}

pub(crate) fn stop_script_profiler(_args: &[Value]) -> Result<Value, RuntimeError> {
    if let Some(entries) = clonk_script::stop_script_profiler() {
        info!(target: SCRIPT_PROFILER_TARGET, "Profiler statistics:");
        info!(target: SCRIPT_PROFILER_TARGET, "==============================");
        for entry in entries {
            let function = if entry.direct_exec {
                entry.function
            } else {
                match entry.host_identity {
                    None => format!("global {}", entry.function),
                    Some(host_identity) => HOST_CONTEXT
                        .with(|cell| {
                            cell.borrow().as_ref().and_then(|context| {
                                context.world.script_for_host_identity(host_identity).map(
                                    |(host, definition, _)| match definition {
                                        Some(definition) => {
                                            format!("{definition}::{}", entry.function)
                                        }
                                        None if host == "Game.Script" => {
                                            format!("game {}", entry.function)
                                        }
                                        None => format!("{host}::{}", entry.function),
                                    },
                                )
                            })
                        })
                        .unwrap_or(entry.function),
                }
            };
            info!(
                target: SCRIPT_PROFILER_TARGET,
                "{:05}ms\t{}",
                entry.elapsed.as_millis(),
                function
            );
        }
        info!(target: SCRIPT_PROFILER_TARGET, "==============================");
    }
    Ok(Value::Nil)
}

pub(crate) fn message(args: &[Value]) -> Result<Value, RuntimeError> {
    let raw_message = match args.first().unwrap_or(&Value::Nil) {
        Value::String(text) => text.as_ref().to_owned(),
        Value::Nil => return Ok(Value::Bool(false)),
        other => {
            return Err(RuntimeError::new(format!(
                "Message: expected string for message, got {}",
                other.type_name()
            )));
        }
    };

    let target_raw = if let Some(arg) = args.get(1) {
        parse_object_reference_argument(arg, "Message", "target")?.map(|id| id.as_u64())
    } else {
        None
    };

    let format_args = if args.len() > 2 { &args[2..] } else { &[] };

    try_with_host_context_mut("Message requires an active engine context", |context| {
        let fallback = message_fallback_spec(
            context,
            "Message",
            &raw_message,
            format_args,
            if target_raw.is_some() {
                MessageKind::Target
            } else {
                MessageKind::Global
            },
            target_raw.map(ObjectId::new),
            None,
        );

        // FnMessage's pObj is only the text-message target. Speech is always
        // anchored to cthr->Obj (C4Script.cpp:2415-2427). The frontend must
        // finish NewInstance before deciding whether this fallback survives.
        if let Some(sound) = extract_speech_segment(&raw_message) {
            let speech_target = context.script_object_context;
            let (queued, pending_fallback) =
                context.try_play_speech(&sound, speech_target, fallback.as_ref().ok().cloned());
            if queued {
                if let Some(pending_fallback) = pending_fallback {
                    context.register_message(MessageCommand::PendingSpeech(pending_fallback));
                }
                // Formatting is side-effect free. Keep its deferred error
                // hidden exactly as C++ does when speech succeeds.
                return Ok(Value::Bool(true));
            }
        }

        context.register_message(MessageCommand::Add(fallback?));

        Ok(Value::Bool(true))
    })
}

pub(crate) fn player_message(args: &[Value]) -> Result<Value, RuntimeError> {
    let player_id = value_to_i32(
        args.first().unwrap_or(&Value::Nil),
        "PlayerMessage",
        "player",
    )?;
    let raw_message = match args.get(1).unwrap_or(&Value::Nil) {
        Value::String(text) => text.clone(),
        Value::Nil => return Ok(Value::Bool(false)),
        other => {
            return Err(RuntimeError::new(format!(
                "PlayerMessage: expected string for message, got {}",
                other.type_name()
            )));
        }
    };

    let target_raw = if let Some(arg) = args.get(2) {
        parse_object_reference_argument(arg, "PlayerMessage", "target")?.map(|id| id.as_u64())
    } else {
        None
    };

    let format_args = if args.len() > 3 { &args[3..] } else { &[] };

    try_with_host_context_mut(
        "PlayerMessage requires an active engine context",
        |context| {
            let kind = if target_raw.is_some() {
                MessageKind::TargetPlayer
            } else {
                MessageKind::GlobalPlayer
            };
            let fallback = message_fallback_spec(
                context,
                "PlayerMessage",
                &raw_message,
                format_args,
                kind,
                target_raw.map(ObjectId::new),
                Some(player_id),
            );
            if let Some(sound) = extract_speech_segment(&raw_message) {
                let speech_target = target_raw
                    .map(ObjectId::new)
                    .or(context.script_object_context);
                let (queued, pending_fallback) =
                    context.try_play_speech(&sound, speech_target, fallback.as_ref().ok().cloned());
                if queued {
                    if let Some(pending_fallback) = pending_fallback {
                        context.register_message(MessageCommand::PendingSpeech(pending_fallback));
                    }
                    return Ok(Value::Bool(true));
                }
            }

            // FnPlayerMessage carries iPlayer into C4GM_*Player verbatim;
            // unlike FnPlrMessage, it never gates through ValidPlr.
            context.register_message(MessageCommand::Add(fallback?));

            Ok(Value::Bool(true))
        },
    )
}

pub(crate) fn add_message(args: &[Value]) -> Result<Value, RuntimeError> {
    let raw_message = match args.first().unwrap_or(&Value::Nil) {
        Value::String(text) => text.clone(),
        Value::Nil => return Ok(Value::Bool(false)),
        other => {
            return Err(RuntimeError::new(format!(
                "AddMessage: expected string for message, got {}",
                other.type_name()
            )));
        }
    };

    let target_raw = if let Some(arg) = args.get(1) {
        parse_object_reference_argument(arg, "AddMessage", "target")?.map(|id| id.as_u64())
    } else {
        None
    };

    let format_args = if args.len() > 2 { &args[2..] } else { &[] };
    let formatted = format_script_string("AddMessage", &raw_message, format_args)?;

    try_with_host_context_mut("AddMessage requires an active engine context", |context| {
        let spec = MessageSpec::new(
            if target_raw.is_some() {
                MessageKind::Target
            } else {
                MessageKind::Global
            },
            formatted,
        )
        .with_target(target_raw.map(ObjectId::new))
        .with_player(if target_raw.is_some() {
            // FnAddMessage uses NO_OWNER for target messages and ANY_OWNER
            // for global messages (C4Script.cpp:2435-2441). Keep those
            // native values distinct at the storage boundary.
            MESSAGE_NO_OWNER
        } else {
            MESSAGE_ANY_OWNER
        })
        .with_color(invert_rgba_alpha(LEGACY_DEFAULT_MESSAGE_COLOR));
        // FnAddMessage calls C4GameMessageList::Append, not New with
        // C4GM_Multiple. The native call also passes fNoDuplicates=false.
        context.register_message(MessageCommand::Append {
            spec,
            no_duplicates: false,
        });

        Ok(Value::Bool(true))
    })
}

pub(crate) fn plr_message(args: &[Value]) -> Result<Value, RuntimeError> {
    let raw_message = match args.first().unwrap_or(&Value::Nil) {
        Value::String(text) => text.clone(),
        Value::Nil => return Ok(Value::Bool(false)),
        other => {
            return Err(RuntimeError::new(format!(
                "PlrMessage: expected string for message, got {}",
                other.type_name()
            )));
        }
    };

    let player_id = value_to_i32(args.get(1).unwrap_or(&Value::Nil), "PlrMessage", "player")?;
    let format_args = if args.len() > 2 { &args[2..] } else { &[] };

    try_with_host_context_mut("PlrMessage requires an active engine context", |context| {
        let resolved_player = resolve_target_player(context, player_id);

        let fallback = message_fallback_spec(
            context,
            "PlrMessage",
            &raw_message,
            format_args,
            if resolved_player.is_some() {
                MessageKind::GlobalPlayer
            } else {
                MessageKind::Global
            },
            None,
            resolved_player,
        );
        if let Some(sound) = extract_speech_segment(&raw_message) {
            let speech_target = context.script_object_context;
            let (queued, pending_fallback) =
                context.try_play_speech(&sound, speech_target, fallback.as_ref().ok().cloned());
            if queued {
                if let Some(pending_fallback) = pending_fallback {
                    context.register_message(MessageCommand::PendingSpeech(pending_fallback));
                }
                return Ok(Value::Bool(true));
            }
        }

        context.register_message(MessageCommand::Add(fallback?));

        Ok(Value::Bool(true))
    })
}

/// Run the callbackful half of an internal Activate/Get menu in the live VM
/// scope, including SetRefillObject's immediate full refill. No HOST_CONTEXT
/// borrow is held while the Activate builder calls GetValue/CalcValue.
pub(crate) fn preview_prepare_put_take_menu(request: MenuRequest) -> bool {
    let prepared = HOST_CONTEXT.with(|cell| {
        let borrow = cell.borrow();
        let context = borrow.as_ref()?;
        let target = match &request.kind {
            MenuRequestKind::Activate => context
                .get_world_object(request.crew_id)
                .and_then(|object| object.container()),
            MenuRequestKind::Get { container } => Some(*container),
            _ => None,
        }?;
        let reused_menu_identity = context
            .object_menu(request.crew_id)
            .map(|menu| menu.internal_refill_token)
            .filter(|identity| *identity != 0);
        Some((
            target,
            if matches!(&request.kind, MenuRequestKind::Activate) {
                6
            } else {
                13
            },
            reused_menu_identity,
        ))
    });
    let Some((target, identification, reused_menu_identity)) = prepared else {
        return false;
    };
    let _ = close_object_menu(request.crew_id, true);
    if call_object_own_fail_safe(target, "RejectContents", &[]).as_bool() {
        return false;
    }
    let menu = match identification {
        6 => crate::direct_com::build_activate_menu_state(
            &mut PreviewInternalObjectMenuSource,
            request.crew_id,
            target,
            false,
            reused_menu_identity,
        ),
        13 => crate::direct_com::build_container_contents_menu_state(
            &mut PreviewInternalObjectMenuSource,
            request.crew_id,
            target,
            13,
            false,
            reused_menu_identity,
        ),
        _ => unreachable!("PutTake only opens Activate/Get menus"),
    };
    let Ok(Some(menu)) = menu else {
        return false;
    };
    with_host_context_mut(false, |context| {
        context.set_object_menu(request.crew_id, Some(menu))
    })
}
