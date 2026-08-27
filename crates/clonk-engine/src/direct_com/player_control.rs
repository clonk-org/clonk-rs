//! `C4ControlPlayerControl::Execute` through `C4Player::InCom`: control
//! counting, the per-frame control queue, and the player-owned menus
//! (object, context and object-info).

use super::*;

impl Engine {
    pub(crate) fn count_player_control(
        &mut self,
        owner: i32,
        control_type: CountedControlType,
        id: i32,
        count: i32,
    ) {
        // CountControl observes the cursor before InCom may switch it.
        let cursor = self.players.get(&owner).and_then(crate::Player::cursor);
        let new_action = self
            .players
            .get_mut(&owner)
            .is_some_and(|player| player.count_control(control_type, id, count));
        if !new_action {
            return;
        }
        let Some(cursor) = cursor.filter(|id| self.crew_object_infos.contains_key(id)) else {
            return;
        };
        self.count_crew_info_control(cursor, 1);
    }

    /// Increment one attached `C4ObjectInfo::ControlCount` once per native
    /// control point. Each resulting multiple of five invokes a distinct
    /// `DoExperience(1)` call (C4Command.cpp:1617-1622).
    pub(crate) fn count_crew_info_control(&mut self, object: ObjectId, gain: i32) {
        if gain <= 0 || !self.crew_object_infos.contains_key(&object) {
            return;
        }
        let Some(link) = self.crew_info_links.get(&object).copied() else {
            return;
        };
        for _ in 0..gain {
            let awards_experience = {
                let control_count = self.crew_info_control_counts.entry(link).or_default();
                *control_count = control_count.wrapping_add(1);
                *control_count % 5 == 0
            };
            if awards_experience {
                self.do_object_experience(object, 1);
            }
        }
    }

    /// Replay a synchronous host preview's runtime-only counter delta. Its
    /// corresponding experience calls are transported separately so rank,
    /// physical and presentation ordering stay identical to the preview.
    pub(crate) fn adjust_crew_info_control_count(&mut self, link: CrewInfoLink, gain: i32) {
        let control_count = self.crew_info_control_counts.entry(link).or_default();
        *control_count = control_count.wrapping_add(gain);
    }

    #[doc(hidden)]
    pub(crate) fn crew_info_control_count(&self, object: ObjectId) -> Option<i32> {
        let link = self.crew_info_links.get(&object)?;
        Some(
            self.crew_info_control_counts
                .get(link)
                .copied()
                .unwrap_or(0),
        )
    }

    /// `C4Player::InCom` (C4Player.cpp:1490-1554): pressed-com bookkeeping
    /// plus COM_Single/COM_Double synthesis around the LastCom buffer.
    pub fn player_in_com(&mut self, owner: i32, com: u8, data: i32) -> Result<(), EngineError> {
        // Coms for unknown players are dropped like C4Game control routing
        // does when Players.Get fails.
        if !self.players.contains_key(&owner) {
            return Ok(());
        }
        if com == COM_CLEAR_PRESSED_COMS {
            let player = self.player_mut(owner)?;
            player.control.pressed_coms = 0;
            player.control.last_com = i32::from(COM_NONE);
            return Ok(());
        }
        // Cursor menu ConvertCom (C4Player.cpp:1502-1508;
        // C4Menu.cpp:1040-1069). Only exact press coms convert: releases
        // remain raw and are discarded by the pressed-com guard below.
        let cursor_menu_active = self
            .crew_cursor(owner)
            .and_then(|cursor| self.find_object_index(cursor))
            .is_some_and(|index| self.objects[index].state.menu.is_some());
        let com = if cursor_menu_active {
            match com {
                COM_THROW => COM_MENU_ENTER,
                COM_DIG => COM_MENU_CLOSE,
                COM_SPECIAL2 => COM_MENU_ENTER_ALL,
                COM_UP => COM_MENU_UP,
                COM_LEFT => COM_MENU_LEFT,
                COM_DOWN => COM_MENU_DOWN,
                COM_RIGHT => COM_MENU_RIGHT,
                _ => com,
            }
        } else {
            com
        };
        // Menu control: no single/double processing (C4Player.cpp:1510-1513).
        if (COM_MENU_FIRST..=COM_MENU_LAST).contains(&com) {
            return self.player_direct_com(owner, com, data);
        }
        let mut com = com;
        if !(COM_RELEASE_FIRST..=COM_RELEASE_LAST).contains(&com) {
            // C4Player::ResetCursorView switches any target/scroll camera
            // back to cursor mode before dispatching a new press. Cursor
            // mode follows ViewCursor first, then Cursor, without changing
            // either logical pointer (C4Player.cpp:926-928,1518,1695-1712).
            self.player_mut(owner)?.reset_cursor_view();
            // Update state (C4Player.cpp:1520-1521).
            if (COM_RELEASE_FIRST - COM_RELEASE_OFFSET..=COM_RELEASE_LAST - COM_RELEASE_OFFSET)
                .contains(&com)
            {
                let player = self.player_mut(owner)?;
                player.control.pressed_coms |= 1 << com;
            }
            // Check LastCom buffer for prior COM_Single (C4Player.cpp:1522-1531).
            let (last_com, control_style) = {
                let player = self.player_mut(owner)?;
                (player.control.last_com, player.control.control_style)
            };
            if last_com != i32::from(COM_NONE) && last_com != i32::from(com) {
                // C++ stores LastCom as int32_t but DirectCom accepts uint8_t.
                // Preserve the full compiler word for comparisons and apply
                // the language-defined low-byte conversion only at dispatch.
                self.player_direct_com(owner, (last_com | i32::from(COM_SINGLE)) as u8, data)?;
                // AutoStopControl uses a single COM_Down instead of COM_Down_D
                // for drop (C4Player.cpp:1527-1530).
                if control_style && last_com == i32::from(COM_DOWN) {
                    self.player_mut(owner)?.control.last_com_down_double = C4_DOUBLE_CLICK;
                }
            }
            // Check LastCom buffer for COM_Double (C4Player.cpp:1532-1533).
            let player = self.player_mut(owner)?;
            if player.control.last_com == i32::from(com) {
                com |= COM_DOUBLE;
            }
            // Set before the DirectCom so scripts may clear it (:1534-1536).
            player.control.last_com = i32::from(com);
            player.control.last_com_delay = 0;
        } else {
            // KeyRelease: only when the press was registered (:1540-1548).
            let player = self.player_mut(owner)?;
            let bit = 1 << (com - COM_RELEASE_OFFSET);
            if player.control.pressed_coms & bit == 0 {
                return Ok(());
            }
            player.control.pressed_coms &= !bit;
        }
        // Pass regular/COM_Double byCom to player (:1550-1551).
        self.player_direct_com(owner, com, data)?;
        // LastComDownDouble process (:1552-1553).
        if com == COM_DOWN_D {
            self.player_mut(owner)?.control.last_com_down_double = C4_DOUBLE_CLICK;
        }
        Ok(())
    }

    /// The control half of `C4Player::Execute` (C4Player.cpp:242,
    /// 1215-1232): flash decrements, the LastCom COM_Single timeout and the
    /// LastComDownDouble countdown. Runs once per frame per player after
    /// object execution (C4Game.cpp:822 Players.Execute order).
    pub(crate) fn execute_player_controls(&mut self) -> Result<(), EngineError> {
        let mut owners: Vec<i32> = self.players.keys().copied().collect();
        owners.sort_unstable();
        for owner in owners {
            if self
                .players
                .get(&owner)
                .is_none_or(|player| player.status() == crate::PlayerStatus::Inactive)
            {
                continue;
            }
            self.execute_player_control_and_menu(owner)?;
            let _ = self.finish_player_execute_delays(owner);
        }
        Ok(())
    }

    /// The Tick1 control/menu/AutoContext portion of one C4Player::Execute.
    /// Callers perform UpdateCounts/UpdateView first and Tick35/delays after.
    pub(crate) fn execute_player_control_and_menu(
        &mut self,
        owner: i32,
    ) -> Result<(), EngineError> {
        let timed_out_last_com = {
            let Some(player) = self.players.get_mut(&owner) else {
                return Ok(());
            };
            if player.status() == crate::PlayerStatus::Inactive {
                return Ok(());
            }
            // LastCom timeout (C4Player.cpp:1215-1229).
            if player.control.last_com != i32::from(COM_NONE) {
                player.control.last_com_delay += 1;
                if player.control.last_com_delay > C4_DOUBLE_CLICK {
                    Some(player.control.last_com)
                } else {
                    None
                }
            } else {
                None
            }
        };
        if let Some(last_com) = timed_out_last_com {
            // C++ keeps LastCom visible during the synchronous COM_Single
            // callback and clears it only after DirectCom returns.
            if last_com & i32::from(COM_SINGLE) == 0 {
                self.player_direct_com(owner, (last_com | i32::from(COM_SINGLE)) as u8, 0)?;
            }
            if let Some(player) = self.players.get_mut(&owner) {
                player.control.last_com = i32::from(COM_NONE);
                player.control.last_com_delay = 0;
            }
        }
        if let Some(player) = self.players.get_mut(&owner) {
            // LastComDownDouble (C4Player.cpp:1231-1232).
            if player.control.last_com_down_double > 0 {
                player.control.last_com_down_double -= 1;
            }
        }
        self.refill_player_object_menu(owner)?;
        self.open_player_auto_context_menu(owner)?;
        Ok(())
    }

    /// Final delay tail of one C4Player::Execute. The return value mirrors the
    /// list-level retirement check, which runs only after every player has
    /// completed its Execute pass.
    pub(crate) fn finish_player_execute_delays(&mut self, owner: i32) -> bool {
        let Some(player) = self.players.get_mut(&owner) else {
            return false;
        };
        if player.status() == crate::PlayerStatus::Inactive {
            return false;
        }
        player.advance_runtime_delays();
        let ready_to_retire = player.advance_retire_delay();
        if player.control.cursor_flash > 0 {
            player.control.cursor_flash -= 1;
        }
        if player.control.select_flash > 0 {
            player.control.select_flash -= 1;
        }
        ready_to_retire
    }

    /// Player-menu execution notices refill-target content-count changes
    /// after objects have run and performs the shared 35-tick refill
    /// (C4Player.cpp:206-212; C4ObjectMenu.cpp:448-459). Rebuild every
    /// refill-driven internal object menu before the AutoContextMenu tail.
    pub(in crate::direct_com) fn refill_player_object_menu(
        &mut self,
        owner: i32,
    ) -> Result<(), EngineError> {
        if self
            .players
            .get(&owner)
            .is_none_or(|player| player.status() == crate::PlayerStatus::Inactive)
        {
            return Ok(());
        }
        let periodic_refill = self.frame.is_multiple_of(35);
        let Some(crew_id) = self.crew_cursor(owner) else {
            return Ok(());
        };
        let Some(crew_index) = self.find_object_index(crew_id) else {
            return Ok(());
        };
        let Some(menu) = self.objects[crew_index].state.menu.as_ref() else {
            return Ok(());
        };
        let identification = match menu.identification {
            Value::Int(4) => 4,
            Value::Int(5) => 5,
            Value::Int(6) => 6,
            Value::Int(13) => 13,
            Value::Int(14) => 14,
            Value::Int(18) => 18,
            _ => return Ok(()),
        };
        let refill_object = menu.refill_object;
        let previous_count = menu.refill_object_contents_count;
        let previous_item_count = menu.items.len();
        let previous_style = menu.style;
        let previous_runtime_id = menu.runtime_id;
        let previous_location_reset_generation = menu.location_reset_generation;
        let context_has_command_object = menu.command_object.is_some();
        let Some(container_id) = refill_object else {
            if periodic_refill {
                let _ = self.close_object_menu(crew_id, true)?;
            }
            return Ok(());
        };
        let Some(container_index) = self.find_object_index(container_id) else {
            if periodic_refill {
                let _ = self.close_object_menu(crew_id, true)?;
            }
            return Ok(());
        };
        let current_count = self.live_contents_count(&self.objects[container_index].state.contents);
        if !periodic_refill && current_count == previous_count {
            return Ok(());
        }
        // DoRefillInternal reports failure for these mode-specific invalid
        // states; C4Menu::Execute then closes directly, without consulting a
        // user-menu cancellation callback (C4ObjectMenu.cpp:207-242,328-334;
        // C4Menu.cpp:990-999).
        let refill_fails = match identification {
            4 => {
                !self.base_buy_enabled
                    || !self
                        .players
                        .contains_key(&self.objects[container_index].state.base)
            }
            5 => !self.base_sell_enabled,
            14 => !context_has_command_object,
            _ => false,
        };
        if refill_fails {
            let _ = self.close_object_menu(crew_id, true)?;
            return Ok(());
        }
        match identification {
            4 => self.refill_base_buy_menu(crew_index, container_index)?,
            5 => self.refill_base_sell_menu(crew_index, container_index)?,
            6 => self.open_activate_menu(crew_index, container_index)?,
            13 | 18 => {
                self.open_container_contents_menu(crew_index, container_index, identification)?;
            }
            14 => self.refill_context_menu(crew_index, container_index, current_count)?,
            _ => unreachable!("filtered refill-driven object-menu id"),
        }
        // C4Menu::RefillInternal clears LocationSet only after a successful
        // DoRefillInternal, and only when the final count grows (or a Context
        // menu shrinks). Ordinary AddMenuItem writes do not reach this path;
        // ClearMenuItems(true) already carries its own generation marker.
        if let Some(menu) = self.objects[crew_index]
            .state
            .menu
            .as_mut()
            .filter(|menu| menu.runtime_id == previous_runtime_id)
        {
            let invalidates_location = menu.items.len() > previous_item_count
                || (menu.items.len() < previous_item_count && previous_style == 1);
            if invalidates_location
                && menu.location_reset_generation == previous_location_reset_generation
            {
                menu.mark_location_reset();
            }
        }
        // C4ObjectMenu::Execute stores the observed count before invoking
        // the inherited refill. Store it on the still-matching menu after
        // each non-Context helper returns; Context does the write inside its
        // token-guarded frozen refill so callback-replaced menus stay clean.
        if identification != 14 {
            if let Some(menu) = self.objects[crew_index].state.menu.as_mut().filter(|menu| {
                menu.identification == Value::Int(identification)
                    && menu.refill_object == Some(container_id)
            }) {
                menu.refill_object_contents_count = current_count;
            }
        }
        Ok(())
    }

    pub(crate) fn open_player_auto_context_menu(&mut self, owner: i32) -> Result<(), EngineError> {
        let Some(crew_index) = self
            .crew_cursor(owner)
            .and_then(|crew| self.find_object_index(crew))
        else {
            return Ok(());
        };
        if !self
            .players
            .get(&owner)
            .is_some_and(|player| player.control.auto_context_menu)
            || !self.objects[crew_index].commands.is_empty()
            || self.objects[crew_index].state.menu.is_some()
            || !self.objects[crew_index].state.crew_member
        {
            return Ok(());
        }
        let Some(base_index) = self.objects[crew_index]
            .state
            .container
            .and_then(|base| self.find_object_index(base))
        else {
            return Ok(());
        };
        let auto_context = self
            .definitions
            .get(&self.objects[base_index].definition_id)
            .is_some_and(|definition| definition.auto_context_menu());
        if auto_context {
            self.open_context_menu(crew_index, base_index, true, None)?;
        }
        Ok(())
    }

    pub(in crate::direct_com) fn context_function_item(
        &self,
        function: &crate::ScriptContextFunction,
        caption: String,
        command: String,
        fallback_picture: Option<ObjectId>,
    ) -> crate::ObjectMenuItem {
        const C4MN_ITEM_NO_COUNT: i32 = 12_345_678;
        let image = function
            .image
            .as_deref()
            .filter(|image| !image.is_empty())
            .unwrap_or("NONE");
        let fallback_picture = (image == "NONE" || !self.definitions.contains_key(image))
            .then_some(fallback_picture)
            .flatten();
        let fallback_snapshot = fallback_picture
            .and_then(|object| self.find_object_index(object))
            .map(|index| {
                let object = &self.objects[index];
                crate::ObjectMenuPictureSnapshot {
                    definition_id: object.definition_id.clone(),
                    symbol_size: 35,
                    base_graphics: object.state.base_graphics.clone(),
                    graphics_overlays: object.state.graphics_overlays.clone(),
                    blit_mode: object.state.blit_mode,
                    color: object.state.color,
                    color_modulation: object.state.color_modulation,
                    picture_rect: object.state.picture_rect,
                    rank: None,
                }
            });
        crate::ObjectMenuItem {
            caption,
            info_caption: crate::normalize_menu_info_caption(
                function.description.clone().unwrap_or_default(),
            ),
            command,
            command2: String::new(),
            count: C4MN_ITEM_NO_COUNT,
            item_id: "NONE".to_owned(),
            symbol: crate::ObjectMenuSymbol::Definition,
            image: if image != "NONE" && self.definitions.contains_key(image) {
                crate::ObjectMenuImage::Indexed {
                    index: function.image_phase,
                }
            } else if let Some(object) = fallback_picture {
                crate::ObjectMenuImage::Object { object }
            } else {
                crate::ObjectMenuImage::None
            },
            presentation_definition_id: (image != "NONE" && self.definitions.contains_key(image))
                .then(|| image.to_owned()),
            picture_snapshot: fallback_snapshot,
            picture_object: None,
            components: Vec::new(),
            selectable: true,
            value: None,
            text_display_progress: -1,
        }
    }

    pub(in crate::direct_com) fn add_native_context_menu_item(
        &mut self,
        menu_object: ObjectId,
        item: crate::ObjectMenuItem,
    ) {
        let Some(menu_index) = self.find_object_index(menu_object) else {
            return;
        };
        let Some(menu) = self.objects[menu_index]
            .state
            .menu
            .as_mut()
            .filter(|menu| menu.identification == Value::Int(14))
        else {
            return;
        };
        if menu.internal_refill_token == 0 && menu.selection == -1 && item.selectable {
            menu.selection = menu.items.len() as i32;
        }
        menu.items.push(item);
    }

    pub(in crate::direct_com) fn record_context_function_item(
        &mut self,
        menu_object: ObjectId,
        publish: bool,
        items: &mut Vec<crate::ObjectMenuItem>,
        item: crate::ObjectMenuItem,
    ) {
        if publish {
            self.add_native_context_menu_item(menu_object, item.clone());
        }
        items.push(item);
    }

    pub(in crate::direct_com) fn context_condition_on_object(
        &mut self,
        object_index: usize,
        function: &crate::ScriptContextFunction,
        arguments: &str,
        label: &str,
    ) -> Result<bool, EngineError> {
        let Some(condition) = function.condition.as_deref() else {
            return Ok(true);
        };
        let source = format!("{condition}({arguments})");
        Ok(
            tolerate_script_error(self.direct_exec_on_object(object_index, &source, label))?
                .is_some_and(|value| compat::value_raw_truthy(&value)),
        )
    }

    pub(in crate::direct_com) fn global_script_menu_functions(
        &self,
        prefix: &str,
    ) -> Vec<crate::ScriptContextFunction> {
        let Some(global_functions) = self.global_script_functions.as_deref() else {
            return Vec::new();
        };
        let mut seen = HashSet::new();
        self.global_script_function_order
            .iter()
            .rev()
            .filter(|name| seen.insert(name.as_str()))
            .filter(|name| name.starts_with(prefix))
            .filter_map(|name| global_functions.get(name))
            .map(|function| {
                let mut metadata = crate::script_context_function_metadata(function);
                if metadata.condition.as_ref().is_some_and(|condition| {
                    !self.global_menu_condition_resolves(&function.name, condition)
                }) {
                    metadata.condition = None;
                }
                metadata
            })
            .collect()
    }

    /// All native classes of `C4ObjectMenu::AddContextFunctions`, in their
    /// C++ order: ActionContext, effect Fx*Context, AttachContext,
    /// Activate/ControlDigDouble, then target Context* functions
    /// (C4ObjectMenu.cpp:544-685).
    pub(in crate::direct_com) fn context_function_menu_items(
        &mut self,
        target_index: usize,
        menu_object: ObjectId,
        publish: bool,
    ) -> Result<Vec<crate::ObjectMenuItem>, EngineError> {
        let Some(menu_index) = self.find_object_index(menu_object) else {
            return Ok(Vec::new());
        };
        let target_id = self.objects[target_index].id;
        let target_definition = self.objects[target_index].definition_id.clone();
        let target_action = self.objects[target_index].state.action.clone();
        let target_action_active = self
            .definitions
            .get(&target_definition)
            .is_some_and(|definition| !definition.action_library().is_idle_state(&target_action));
        let target_action_target = self.objects[target_index].state.action.target;
        let mut items = Vec::new();

        // ActionContext functions of the target's first action target.
        if target_action_active {
            if let Some(action_target_index) =
                target_action_target.and_then(|id| self.find_object_index(id))
            {
                let action_target_id = self.objects[action_target_index].id;
                let definition_id = self.objects[action_target_index].definition_id.clone();
                let functions = self
                    .definitions
                    .get(&definition_id)
                    .map(|definition| definition.script_menu_functions("ActionContext"))
                    .unwrap_or_default();
                for function in functions {
                    let image = function.image.as_deref().unwrap_or("NONE");
                    let arguments = format!(
                        "Object({}), C4Id(\"{}\"), Object({})",
                        menu_object.as_u64(),
                        image,
                        target_id.as_u64()
                    );
                    if !self.context_condition_on_object(
                        action_target_index,
                        &function,
                        &arguments,
                        "ActionContextCondition",
                    )? {
                        continue;
                    }
                    let command = format!(
                        "ProtectedCall(Object({}),\"{}\",this,Object({}))",
                        action_target_id.as_u64(),
                        function.function,
                        target_id.as_u64()
                    );
                    let item = self.context_function_item(
                        &function,
                        function.label.clone(),
                        command,
                        None,
                    );
                    self.record_context_function_item(menu_object, publish, &mut items, item);
                }
            }
        }

        // Active effect context functions use the callback script selected
        // by the live command target object/id, or the global script host.
        let mut effect_cursor = None;
        loop {
            let effects = &self.objects[target_index].state.effects;
            let mut effect_index = crate::effect_frame_cursor_next_index(effects, effect_cursor);
            let Some(effect) = (loop {
                let Some(effect) = effects.get(effect_index).cloned() else {
                    break None;
                };
                effect_index += 1;
                if effect.priority > 0 {
                    break Some(effect);
                }
            }) else {
                break;
            };
            effect_cursor = Some(crate::EffectFrameCursor {
                number: effect.number,
                priority: effect.priority.unsigned_abs(),
            });
            let prefix = format!("Fx{}Context", effect.name);
            let command_target = effect.command_target.and_then(|number| {
                u64::try_from(number)
                    .ok()
                    .map(ObjectId::new)
                    .and_then(|id| self.find_object_index(id))
                    .filter(|&index| {
                        !self.objects[index].destroyed
                            && self.objects[index].state.status != crate::ObjectStatus::Deleted
                    })
            });
            let (functions, condition_object, callback_definition) =
                if let Some(command_index) = command_target {
                    let definition_id = self.objects[command_index].definition_id.clone();
                    if let Some(live_effect) = self.objects[target_index]
                        .state
                        .effects
                        .iter_mut()
                        .find(|live_effect| live_effect.number == effect.number)
                    {
                        // GetCallbackScript refreshes idCommandTarget while the
                        // object target is alive, preserving a definition
                        // fallback if a condition deletes that object.
                        live_effect.command_id = Some(definition_id.clone());
                    }
                    let functions = self
                        .definitions
                        .get(&definition_id)
                        .map(|definition| definition.script_menu_functions(&prefix))
                        .unwrap_or_default();
                    (functions, Some(command_index), Some(definition_id))
                } else if let Some(definition_id) = effect
                    .command_id
                    .clone()
                    .filter(|definition_id| self.definitions.contains_key(definition_id))
                {
                    let functions = self
                        .definitions
                        .get(&definition_id)
                        .map(|definition| definition.script_menu_functions(&prefix))
                        .unwrap_or_default();
                    (functions, None, Some(definition_id))
                } else {
                    (self.global_script_menu_functions(&prefix), None, None)
                };
            for function in functions {
                let image = function.image.as_deref().unwrap_or("NONE");
                let arguments = format!(
                    "Object({}),{},Object({}),C4Id(\"{}\")",
                    target_id.as_u64(),
                    effect.number,
                    menu_object.as_u64(),
                    image
                );
                let enabled = if let Some(command_index) = condition_object {
                    self.context_condition_on_object(
                        command_index,
                        &function,
                        &arguments,
                        "EffectContextCondition",
                    )?
                } else if let Some(definition_id) = callback_definition.as_deref() {
                    if let Some(condition) = function.condition.as_deref() {
                        let source = format!(
                            "DefinitionCall({}, \"{}\", {})",
                            definition_id, condition, arguments
                        );
                        tolerate_script_error(self.direct_exec_on_object(
                            menu_index,
                            &source,
                            "EffectContextCondition",
                        ))?
                        .is_some_and(|value| compat::value_raw_truthy(&value))
                    } else {
                        true
                    }
                } else if let Some(condition) = function.condition.as_deref() {
                    if let Some((script_name, script)) =
                        self.global_menu_callback_script(&function.function)
                    {
                        let source = format!("{condition}({arguments})");
                        let value = self.direct_exec_script_control_host(
                            &script_name,
                            script.as_ref(),
                            &source,
                            "EffectContextCondition",
                            None,
                        )?;
                        compat::value_raw_truthy(&value)
                    } else {
                        let source = format!("global->~{condition}({arguments})");
                        tolerate_script_error(self.direct_exec_on_object(
                            menu_index,
                            &source,
                            "EffectContextCondition",
                        ))?
                        .is_some_and(|value| compat::value_raw_truthy(&value))
                    }
                } else {
                    true
                };
                if !enabled {
                    continue;
                }
                let live_effect = self.objects[target_index]
                    .state
                    .effects
                    .iter()
                    .find(|live_effect| live_effect.number == effect.number)
                    .cloned();
                let live_command_target = live_effect
                    .as_ref()
                    .map_or(effect.command_target, |live_effect| {
                        live_effect.command_target
                    })
                    .and_then(|number| u64::try_from(number).ok())
                    .map(ObjectId::new)
                    .and_then(|id| self.find_object_index(id))
                    .filter(|&index| {
                        !self.objects[index].destroyed
                            && self.objects[index].state.status != crate::ObjectStatus::Deleted
                    });
                let live_definition = live_effect
                    .as_ref()
                    .and_then(|live_effect| live_effect.command_id.clone())
                    .filter(|definition_id| self.definitions.contains_key(definition_id))
                    .or_else(|| callback_definition.clone());
                let command = if let Some(command_index) = live_command_target {
                    let command_id = self.objects[command_index].id;
                    format!(
                        "ProtectedCall(Object({}),\"{}\",Object({}),{},Object({}),{})",
                        command_id.as_u64(),
                        function.function,
                        target_id.as_u64(),
                        effect.number,
                        menu_object.as_u64(),
                        image
                    )
                } else if let Some(definition_id) = live_definition.as_deref() {
                    format!(
                        "DefinitionCall({}, \"{}\", Object({}),{},Object({}),{})",
                        definition_id,
                        function.function,
                        target_id.as_u64(),
                        effect.number,
                        menu_object.as_u64(),
                        image
                    )
                } else {
                    format!(
                        "global->~{}(Object({}),{},Object({}),{})",
                        function.function,
                        target_id.as_u64(),
                        effect.number,
                        menu_object.as_u64(),
                        image
                    )
                };
                let item =
                    self.context_function_item(&function, function.label.clone(), command, None);
                self.record_context_function_item(menu_object, publish, &mut items, item);
            }
        }

        // AttachContext functions of every active DFA_ATTACH object whose
        // first action target is the context target, in global object order.
        let attached_objects = self
            .execution
            .exec_list
            .iter()
            .rev()
            .copied()
            .collect::<Vec<_>>();
        for attached_id in attached_objects {
            let Some(attached_index) = self.find_object_index(attached_id) else {
                continue;
            };
            let still_attached = {
                let object = &self.objects[attached_index];
                let action_active =
                    self.definitions
                        .get(&object.definition_id)
                        .is_some_and(|definition| {
                            !definition
                                .action_library()
                                .is_idle_state(&object.state.action)
                        });
                !object.destroyed
                    && object.state.status.is_active()
                    && object.state.action.target == Some(target_id)
                    && action_active
                    && self.object_procedure(attached_index) == ActionProcedure::Attach
            };
            if !still_attached {
                continue;
            }
            let definition_id = self.objects[attached_index].definition_id.clone();
            let functions = self
                .definitions
                .get(&definition_id)
                .map(|definition| definition.script_menu_functions("AttachContext"))
                .unwrap_or_default();
            for function in functions {
                let image = function.image.as_deref().unwrap_or("NONE");
                let arguments = format!(
                    "Object({}), C4Id(\"{}\"), Object({})",
                    menu_object.as_u64(),
                    image,
                    target_id.as_u64()
                );
                if !self.context_condition_on_object(
                    attached_index,
                    &function,
                    &arguments,
                    "AttachContextCondition",
                )? {
                    continue;
                }
                let command = format!(
                    "ProtectedCall(Object({}),\"{}\",this,Object({}))",
                    attached_id.as_u64(),
                    function.function,
                    target_id.as_u64()
                );
                let item =
                    self.context_function_item(&function, function.label.clone(), command, None);
                self.record_context_function_item(menu_object, publish, &mut items, item);
            }
        }

        // Exact Activate and ControlDigDouble rows, with the Context*
        // DescText duplicate scan performed independently for each row.
        for name in ["Activate", "ControlDigDouble"] {
            let eligible = if name == "Activate" {
                self.objects[target_index].state.container == Some(menu_object)
            } else {
                self.object_procedure(menu_index) == ActionProcedure::Push
                    && self.objects[menu_index].state.action.target == Some(target_id)
            };
            if !eligible {
                continue;
            }
            let Some(function) = self
                .definitions
                .get(&target_definition)
                .and_then(|definition| definition.script_menu_function(name))
            else {
                continue;
            };
            let image = function.image.as_deref().unwrap_or("NONE");
            let arguments = format!("Object({}), C4Id(\"{}\")", menu_object.as_u64(), image);
            if !self.context_condition_on_object(
                target_index,
                &function,
                &arguments,
                "ContextFunctionCondition",
            )? {
                continue;
            }
            let caption = if function.has_description {
                function.label.clone()
            } else {
                self.objects[target_index]
                    .state
                    .custom_name
                    .clone()
                    .or_else(|| {
                        self.crew_object_infos
                            .get(&target_id)
                            .map(|info| info.name.clone())
                    })
                    .or_else(|| {
                        self.definitions
                            .get(&target_definition)
                            .map(|definition| definition.name().to_owned())
                    })
                    .unwrap_or_else(|| target_definition.clone())
            };
            let duplicate_functions = self
                .definitions
                .get(&target_definition)
                .map(|definition| definition.script_menu_functions("Context"))
                .unwrap_or_default();
            let mut duplicate = false;
            for context in duplicate_functions {
                let context_image = context.image.as_deref().unwrap_or("NONE");
                let arguments = format!(
                    "Object({}), C4Id(\"{}\")",
                    menu_object.as_u64(),
                    context_image
                );
                if self.context_condition_on_object(
                    target_index,
                    &context,
                    &arguments,
                    "ContextFunctionCondition",
                )? && caption == context.label
                {
                    duplicate = true;
                }
            }
            if duplicate {
                continue;
            }
            let command = format!(
                "ProtectedCall(Object({}),\"{}\",this)",
                target_id.as_u64(),
                function.function
            );
            let item = self.context_function_item(&function, caption, command, Some(target_id));
            self.record_context_function_item(menu_object, publish, &mut items, item);
        }

        // Target Context* functions: crew members must be owned by the menu
        // object and living targets must still be alive.
        let target = &self.objects[target_index].state;
        let menu_owner = self.objects[menu_index].state.owner;
        if (target.ocf & ocf::CREW_MEMBER == 0 || target.owner == menu_owner)
            && (target.category & crate::CATEGORY_LIVING == 0 || target.alive)
        {
            let functions = self
                .definitions
                .get(&target_definition)
                .map(|definition| definition.script_menu_functions("Context"))
                .unwrap_or_default();
            for function in functions {
                let image = function.image.as_deref().unwrap_or("NONE");
                let arguments = format!("Object({}), C4Id(\"{}\")", menu_object.as_u64(), image);
                if !self.context_condition_on_object(
                    target_index,
                    &function,
                    &arguments,
                    "ContextCondition",
                )? {
                    continue;
                }
                let command = format!(
                    "ProtectedCall(Object({}),\"{}\",this)",
                    target_id.as_u64(),
                    function.function
                );
                let item =
                    self.context_function_item(&function, function.label.clone(), command, None);
                self.record_context_function_item(menu_object, publish, &mut items, item);
            }
        }
        Ok(items)
    }

    /// Internal C4MN_Context refill for an arbitrary target. Automatic
    /// contained menus are permanent; mouse C4CMD_Context menus are not
    /// (C4Object.cpp:1961-1980; C4ObjectMenu.cpp:328-435).
    pub(crate) fn open_context_menu(
        &mut self,
        crew_index: usize,
        base_index: usize,
        permanent: bool,
        location: Option<Vector2>,
    ) -> Result<(), EngineError> {
        self.build_context_menu(crew_index, base_index, permanent, false, 0)?;
        // C4Command::Context applies Free alignment and SetLocation only
        // after ActivateMenu returns, and only if a menu survived.
        if let Some(location) = location {
            if let Some(menu) = self
                .objects
                .get_mut(crew_index)
                .and_then(|object| object.state.menu.as_mut())
            {
                menu.location = Some(location);
            }
        }
        Ok(())
    }

    pub(in crate::direct_com) fn refill_context_menu(
        &mut self,
        crew_index: usize,
        base_index: usize,
        observed_contents_count: i32,
    ) -> Result<(), EngineError> {
        let permanent = self.objects[crew_index]
            .state
            .menu
            .as_ref()
            .is_some_and(|menu| menu.permanent);
        self.build_context_menu(
            crew_index,
            base_index,
            permanent,
            true,
            observed_contents_count,
        )
    }

    pub(in crate::direct_com) fn build_context_menu(
        &mut self,
        crew_index: usize,
        base_index: usize,
        permanent: bool,
        continue_existing: bool,
        observed_contents_count: i32,
    ) -> Result<(), EngineError> {
        const C4MN_ITEM_NO_COUNT: i32 = 12_345_678;
        let crew_id = self.objects[crew_index].id;
        let crew_owner = self.objects[crew_index].state.owner;
        let crew_contents = self.objects[crew_index].state.contents.clone();
        let present_crew_contents = crew_contents
            .into_iter()
            .filter(|object_id| {
                self.find_object_index(*object_id)
                    .is_some_and(|index| self.objects[index].has_nonzero_status())
            })
            .collect::<Vec<_>>();
        let first_carried_object = present_crew_contents.first().copied();
        let first_carried_definition = present_crew_contents
            .first()
            .and_then(|object_id| self.find_object_index(*object_id))
            .map(|index| self.objects[index].definition_id.clone());
        let base = &self.objects[base_index];
        let base_id = base.id;
        let base_definition = base.definition_id.clone();
        let base_owner = base.state.owner;
        let base_player = base.state.base;
        let base_is_container = base.state.ocf & ocf::CONTAINER != 0;
        let base_grab_put_get = self.definition_grab_put_get(&base_definition);
        let caption = self
            .definitions
            .get(&base_definition)
            .map(|definition| definition.name().to_string())
            .unwrap_or_else(|| base_definition.clone());
        let mut items = Vec::new();
        let item = |caption: &str,
                    command: String,
                    item_id: String,
                    symbol: crate::ObjectMenuSymbol| crate::ObjectMenuItem {
            caption: caption.to_string(),
            info_caption: String::new(),
            command,
            command2: String::new(),
            count: C4MN_ITEM_NO_COUNT,
            item_id,
            symbol,
            image: crate::ObjectMenuImage::default(),
            presentation_definition_id: None,
            picture_snapshot: None,
            picture_object: None,
            components: Vec::new(),
            selectable: true,
            value: None,
            text_display_progress: -1,
        };

        let continuing_menu = continue_existing
            .then(|| self.objects[crew_index].state.menu.clone())
            .flatten()
            .filter(|menu| {
                menu.identification == Value::Int(14) && menu.refill_object == Some(base_id)
            });
        let (refill_token, previous_token) = if let Some(mut menu) = continuing_menu {
            // DoRefillInternal uses ClearItems(false): the same menu shell and
            // its current Selection remain live while conditions run. The
            // token models the frozen window: row insertion must not select
            // the first item until RefillInternal's final AdjustSelection.
            let previous_token = menu.internal_refill_token;
            let refill_token = next_internal_object_menu_refill_token();
            menu.internal_refill_token = refill_token;
            menu.items.clear();
            self.objects[crew_index].state.menu = Some(menu);
            (refill_token, previous_token)
        } else {
            if continue_existing {
                return Ok(());
            }
            // ActivateMenu closes and initializes C4MN_Context before Refill
            // evaluates any scripted conditions. Keep the live menu installed
            // throughout the build so GetMenu/AddMenuItem/SelectMenuItem
            // observe the same partially populated menu as C++.
            let _ = self.close_object_menu(crew_id, true)?;
            let Some(crew_index) = self.find_object_index(crew_id) else {
                return Ok(());
            };
            let refill_token = next_internal_object_menu_refill_token();
            self.objects[crew_index].state.menu = Some(crate::ObjectMenuState {
                caption,
                symbol_id: base_definition.clone(),
                title_symbol: crate::ObjectMenuSymbol::default(),
                identification: Value::Int(14),
                style: 1,
                equal_item_height: false,
                permanent,
                location: None,
                runtime_id: next_internal_object_menu_refill_token(),
                extra: crate::ObjectMenuExtra::default(),
                extra_data: 0,
                internal_refill_token: refill_token,
                selection: -1,
                user_menu: false,
                command_object: Some(crew_id),
                scenario_callbacks: false,
                refill_object: Some(base_id),
                refill_object_contents_count: 0,
                location_reset_generation: 0,
                items: Vec::new(),
                columns: 1,
                lines: 0,
                text_progressing: false,
                decoration: None,
            });
            (refill_token, 0)
        };

        let crew_in_base = self.objects[crew_index].state.container == Some(base_id);
        let crew_pushing_base = self.object_procedure(crew_index) == ActionProcedure::Push
            && self.objects[crew_index].state.action.target == Some(base_id);
        if base_is_container
            && (crew_in_base
                || (crew_pushing_base && base_grab_put_get & crate::GRAB_PUT_GET_PUT != 0))
        {
            if let Some(first_carried_definition) = first_carried_definition {
                let command2 = if present_crew_contents.len() > 1
                    || self.selected_crew(crew_owner).len() > 1
                {
                    format!(
                        "PlayerObjectCommand({}, \"Put\", Object({}), 1000, 0) && ExecuteCommand()",
                        crew_owner,
                        base_id.as_u64()
                    )
                } else {
                    String::new()
                };
                items.push(crate::ObjectMenuItem {
                    caption: "Put".to_string(),
                    info_caption: String::new(),
                    command: format!(
                        "PlayerObjectCommand({}, \"Put\", Object({}), 0, 0) && ExecuteCommand()",
                        crew_owner,
                        base_id.as_u64()
                    ),
                    command2,
                    count: C4MN_ITEM_NO_COUNT,
                    item_id: first_carried_definition,
                    symbol: crate::ObjectMenuSymbol::Put,
                    image: crate::ObjectMenuImage::default(),
                    presentation_definition_id: None,
                    picture_snapshot: first_carried_object
                        .and_then(|object| self.native_object_menu_picture_snapshot(object)),
                    picture_object: None,
                    components: Vec::new(),
                    selectable: true,
                    value: None,
                    text_display_progress: -1,
                });
            }
        }
        if base_is_container
            && (crew_in_base
                || (crew_pushing_base && base_grab_put_get & crate::GRAB_PUT_GET_GET != 0)
                || (self.players.contains_key(&base_owner)
                    && !self.players_hostile(base_owner, crew_owner)))
        {
            let mut contents_item = item(
                "Contents",
                format!(
                    "SetCommand(this,\"Get\",Object({}),0,0,,2)&&ExecuteCommand()",
                    base_id.as_u64()
                ),
                base_definition.clone(),
                crate::ObjectMenuSymbol::Definition,
            );
            contents_item.picture_snapshot = self.native_object_menu_picture_snapshot(base_id);
            items.push(contents_item);
        }
        if self.players.contains_key(&base_player) && !self.players_hostile(base_player, crew_owner)
        {
            if self.base_buy_enabled {
                items.push(item(
                    "Buy",
                    format!(
                        "SetCommand(this,\"Buy\",Object({}))&&ExecuteCommand()",
                        base_id.as_u64()
                    ),
                    "NONE".to_string(),
                    crate::ObjectMenuSymbol::Buy { owner: base_player },
                ));
            }
            if self.base_sell_enabled {
                items.push(item(
                    "Sell",
                    format!(
                        "SetCommand(this,\"Sell\",Object({}))&&ExecuteCommand()",
                        base_id.as_u64()
                    ),
                    "NONE".to_string(),
                    crate::ObjectMenuSymbol::Sell { owner: base_player },
                ));
            }
        }
        // AddContextFunctions(target) inserts every native context-function
        // class before BuildInfo/Info/Exit (C4ObjectMenu.cpp:398-408,
        // 544-685).
        for item in items.drain(..) {
            self.add_native_context_menu_item(crew_id, item);
        }
        let _ = self.context_function_menu_items(base_index, crew_id, true)?;
        // AddContextFunctions' final branch exposes the menu Clonk's own
        // context actions when it is inside, pushing, or carrying the clicked
        // target. Building/grab contexts collapse more than two actions into a
        // Clonk submenu; inventory contexts inline every action
        // (C4ObjectMenu.cpp:687-713).
        let crew_container = self.objects[crew_index].state.container;
        let crew_action_target = self.objects[crew_index].state.action.target;
        let crew_is_alive = self.objects[crew_index].state.category & crate::CATEGORY_LIVING == 0
            || self.objects[crew_index].state.alive;
        let base_container = self.objects[base_index].state.container;
        let crew_related_to_target = crew_container == Some(base_id)
            || (self.object_procedure(crew_index) == ActionProcedure::Push
                && crew_action_target == Some(base_id))
            || base_container == Some(crew_id);
        if crew_id != base_id && crew_related_to_target && crew_is_alive {
            let submenu_threshold = (base_container != Some(crew_id)).then_some(2_usize);
            let crew_context_count = self
                .context_function_menu_items(crew_index, crew_id, false)?
                .len();
            if submenu_threshold.is_none_or(|threshold| crew_context_count <= threshold) {
                let _ = self.context_function_menu_items(crew_index, crew_id, true)?;
            } else {
                let crew_definition = self.objects[crew_index].definition_id.clone();
                let crew_name = self
                    .definitions
                    .get(&crew_definition)
                    .map(|definition| definition.name().to_string())
                    .unwrap_or_else(|| crew_definition.clone());
                let mut submenu = item(
                    &crew_name,
                    "SetCommand(this,\"Context\",,0,0,this)&&ExecuteCommand()".to_string(),
                    "NONE".to_string(),
                    crate::ObjectMenuSymbol::Definition,
                );
                submenu.presentation_definition_id = Some(crew_definition);
                submenu.info_caption =
                    "Opens a sub menu with command options for this clonk.".to_string();
                items.push(submenu);
            }
        }
        if self.objects[base_index].state.ocf & ocf::CONSTRUCT != 0
            && self.objects[crew_index].state.rotation == 0
            && self.construction_needs_material
        {
            items.push(item(
                "Construction material",
                format!(
                    "PlayerMessage(GetOwner(), Object({})->GetNeededMatStr(), Object({}))",
                    base_id.as_u64(),
                    base_id.as_u64()
                ),
                "NONE".to_string(),
                crate::ObjectMenuSymbol::Construction,
            ));
        }
        if self
            .definitions
            .get(&base_definition)
            .and_then(|definition| definition.description())
            .is_some()
        {
            items.push(item(
                "Info",
                format!("ShowInfo(Object({}))", base_id.as_u64()),
                base_definition.clone(),
                crate::ObjectMenuSymbol::Info,
            ));
        }
        if base_is_container && self.objects[crew_index].state.container == Some(base_id) {
            items.push(item(
                "Exit",
                "PlayerObjectCommand(GetOwner(),\"Exit\")&&ExecuteCommand()".to_string(),
                "NONE".to_string(),
                crate::ObjectMenuSymbol::Exit,
            ));
        }
        for item in items {
            self.add_native_context_menu_item(crew_id, item);
        }
        if let Some(menu) = self.objects[crew_index].state.menu.as_mut().filter(|menu| {
            menu.identification == Value::Int(14) && menu.internal_refill_token == refill_token
        }) {
            // RefillInternal's final AdjustSelection uses the selection
            // left by any refill-time callback, not the pre-refill value.
            menu.selection =
                internal_refilled_object_menu_selection(&menu.items, Some(menu.selection), None);
            if continue_existing && menu.refill_object == Some(base_id) {
                menu.refill_object_contents_count = observed_contents_count;
            }
            menu.internal_refill_token = previous_token;
        }
        Ok(())
    }

    /// ShowInfo -> C4Object::ActivateMenu(C4MN_Info): a permanent
    /// information-style menu with the target picture/name and info text
    /// (C4Script.cpp:3332-3336; C4Object.cpp:2008-2027).
    pub(crate) fn open_object_info_menu(
        &mut self,
        crew_index: usize,
        target_index: usize,
    ) -> Result<(), EngineError> {
        const C4MN_ITEM_NO_COUNT: i32 = 12_345_678;
        let crew_id = self.objects[crew_index].id;
        let target_id = self.objects[target_index].id;
        // C4Object::ActivateMenu closes and initializes the new Info menu
        // before evaluating GetInfoString while adding its first item.
        let _ = self.close_object_menu(crew_id, false)?;
        let Some(crew_index) = self.find_object_index(crew_id) else {
            return Ok(());
        };
        let Some(target_index) = self.find_object_index(target_id) else {
            return Ok(());
        };
        let definition_id = self.objects[target_index].definition_id.clone();
        let state = self.objects[target_index].script_state_snapshot();
        let (name, mut info_caption, action_library) = {
            let definition = self
                .definitions
                .get(&definition_id)
                .ok_or_else(|| EngineError::UnknownDefinition(definition_id.clone()))?;
            let name = state
                .custom_name
                .as_deref()
                .filter(|name| !name.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| definition.name().to_string());
            (
                name,
                definition.description().unwrap_or_default().to_string(),
                definition.action_library().clone(),
            )
        };
        self.objects[crew_index].state.menu = Some(crate::ObjectMenuState {
            caption: name.clone(),
            symbol_id: "NONE".to_string(),
            title_symbol: crate::ObjectMenuSymbol::InfoTitle,
            identification: Value::Int(15),
            style: 2,
            equal_item_height: false,
            permanent: true,
            location: None,
            runtime_id: next_internal_object_menu_refill_token(),
            extra: crate::ObjectMenuExtra::default(),
            extra_data: 0,
            internal_refill_token: 0,
            selection: -1,
            user_menu: false,
            command_object: Some(crew_id),
            scenario_callbacks: false,
            refill_object: None,
            refill_object_contents_count: 0,
            location_reset_generation: 0,
            items: Vec::new(),
            columns: 1,
            lines: 0,
            text_progressing: false,
            decoration: None,
        });

        let effect_call = {
            let definition = self
                .definitions
                .get(&definition_id)
                .ok_or_else(|| EngineError::UnknownDefinition(definition_id.clone()))?;
            definition.call_object_effect_info(
                &state,
                target_id,
                self.rng.clone(),
                &self.global_effects.clone(),
                self.physics,
                self.environment,
                self.frame,
                self.host_world_context(),
                self.game_over_triggered,
                self.audio_registry.clone(),
            )
        }?;
        let (effect_lines, outcome, audio, rng) = effect_call;
        self.rng = rng;
        self.audio_registry = audio;
        self.apply_action_callback_outcome(
            target_index,
            outcome,
            &action_library,
            target_id,
            &definition_id,
        )?;
        for line in effect_lines {
            if !info_caption.is_empty() {
                info_caption.push('|');
            }
            info_caption.push_str(&line);
        }
        let item = crate::ObjectMenuItem {
            caption: name.clone(),
            info_caption: crate::normalize_menu_info_caption(info_caption),
            command: String::new(),
            command2: String::new(),
            count: C4MN_ITEM_NO_COUNT,
            item_id: definition_id.clone(),
            symbol: crate::ObjectMenuSymbol::default(),
            image: crate::ObjectMenuImage::default(),
            presentation_definition_id: None,
            picture_snapshot: None,
            picture_object: Some(target_id),
            components: Vec::new(),
            selectable: true,
            value: None,
            text_display_progress: -1,
        };
        let Some(crew_index) = self.find_object_index(crew_id) else {
            return Ok(());
        };
        if let Some(menu) = self.objects[crew_index]
            .state
            .menu
            .as_mut()
            .filter(|menu| menu.identification == Value::Int(15))
        {
            menu.items.push(item);
            menu.selection = 0;
        }
        Ok(())
    }

    /// `C4Player::DirectCom` (C4Player.cpp:1453-1488): the cursor coms'
    /// script-override half (`Cursor->CallControl`, :1457-1475) and the
    /// crew-cycling dispatch (:1479-1485); everything else goes to the
    /// cursor object via ObjectCom.
    pub fn player_direct_com(&mut self, owner: i32, com: u8, data: i32) -> Result<(), EngineError> {
        let plain_cursor = matches!(
            com & !COM_DOUBLE,
            COM_CURSOR_LEFT | COM_CURSOR_RIGHT | COM_CURSOR_TOGGLE
        );
        if plain_cursor {
            // Cursor object override (:1457-1475).
            if !self.is_owner_eliminated(owner) {
                if let Some(index) = self
                    .crew_cursor(owner)
                    .and_then(|cursor| self.find_object_index(cursor))
                {
                    self.objects[index].state.controller = owner;
                    if self.object_call_control(index, owner, com, None)? {
                        if com & COM_DOUBLE == 0 {
                            self.player_update_selection_toggle_status(owner)?;
                        }
                        return Ok(());
                    }
                }
            }
            // Crew cycling (:1479-1485).
            match com & !COM_DOUBLE {
                COM_CURSOR_LEFT => self.player_cursor_left(owner)?,
                COM_CURSOR_RIGHT => self.player_cursor_right(owner)?,
                COM_CURSOR_TOGGLE => {
                    if com & COM_DOUBLE != 0 {
                        self.player_select_all_crew(owner)?;
                    } else {
                        self.player_cursor_toggle(owner)?;
                    }
                }
                _ => {}
            }
            return Ok(());
        }
        // Everything else routes to the cursor object (C4Player.cpp:1486);
        // menu-com leftovers get swallowed in object_direct_com like
        // C4Object.cpp:3356-3357 (object menus live in the app layer).
        self.player_object_com(owner, com, data)
    }
}
