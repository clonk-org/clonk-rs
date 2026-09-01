//! `C4Object::DirectCom` and the object-menu control it dispatches, plus
//! the procedure, push and AutoStop com arms.

use super::*;

impl Engine {
    /// `C4Object::DirectCom` (C4Object.cpp:3327-3557).
    pub(crate) fn object_direct_com(
        &mut self,
        index: usize,
        com: u8,
        data: i32,
    ) -> Result<(), EngineError> {
        let is_release = (COM_RELEASE_FIRST..=COM_RELEASE_LAST).contains(&com);
        let plain_com = if is_release {
            com - COM_RELEASE_OFFSET
        } else {
            com & !(COM_SINGLE | COM_DOUBLE)
        };
        let is_cursor = (COM_CURSOR_FIRST..=COM_CURSOR_LAST).contains(&plain_com);

        // We only want the script callbacks for cursor controls (:3339-3347).
        if is_cursor {
            let controller = self.objects[index].state.controller;
            if self.players.contains_key(&controller) {
                self.object_call_control(index, controller, com, None)?;
            }
            return Ok(());
        }

        // COM_Special and COM_Contents bypass an active object menu;
        // every other com goes to Menu->Control first, whose active-menu
        // return consumes even unrecognized raw/release coms
        // (C4Object.cpp:3349-3367; C4Menu.cpp:433-480).
        let bypass_menu = plain_com == COM_SPECIAL || com == COM_CONTENTS;
        if !bypass_menu && self.object_menu_control(index, com, data)? {
            return Ok(());
        }

        // Menu com leftovers from a menu closed before execution are
        // swallowed (C4Object.cpp:3369-3371).
        if (COM_MENU_NAVIGATION1..=COM_MENU_NAVIGATION2).contains(&com) {
            return Ok(());
        }

        // Decrease NoCollectDelay (:3359-3362): plain (non-Single/Double,
        // non-release) coms count the drop's collection delay down; the
        // ObjectComDrop arm that sets it lives with the command layer.
        if com & COM_SINGLE == 0 && com & COM_DOUBLE == 0 && !is_release {
            let delay = &mut self.objects[index].state.no_collect_delay;
            if *delay > 0 {
                *delay -= 1;
            }
        }

        // COM_Contents contents shift (:3364-3372): data carries the target
        // object NUMBER (not ID); the shift always runs on the target's
        // container, which is not necessarily this object.
        if com == COM_CONTENTS {
            let target_id = ObjectId::new(data as u64);
            if let Some(container_index) = self
                .find_object_index(target_id)
                .filter(|&target_index| self.objects[target_index].has_nonzero_status())
                .and_then(|target_index| self.objects[target_index].state.container)
                .and_then(|container_id| self.find_object_index(container_id))
            {
                self.object_direct_com_contents(container_index, target_id, true)?;
            }
            return Ok(());
        }

        // Contained control (except specials) (:3374-3379).
        if let Some(container) = self.objects[index].state.container {
            if plain_com != COM_SPECIAL
                && plain_com != COM_SPECIAL2
                && com != COM_WHEEL_UP
                && com != COM_WHEEL_DOWN
            {
                if let Some(container_index) = self.find_object_index(container) {
                    let controller = self.objects[index].state.controller;
                    self.objects[container_index].state.controller = controller;
                    self.object_contained_control(index, com)?;
                }
                return Ok(());
            }
        }

        // Regular DirectCom clears commands (:3381-3383).
        if com & (COM_SINGLE | COM_DOUBLE) == 0 && !is_release {
            self.objects[index].apply_command_operations([CommandOperation::Clear]);
        }

        // Object script override — CallControl runs for EVERY com (:3385-3389).
        let controller = self.objects[index].state.controller;
        let has_controller = self.players.contains_key(&controller);
        if has_controller && self.object_call_control(index, controller, com, None)? {
            return Ok(());
        }

        // Direct wheel control (:3391-3396): scroll contents.
        if com == COM_WHEEL_UP || com == COM_WHEEL_DOWN {
            self.object_shift_contents(index, com == COM_WHEEL_UP, true)?;
            return Ok(());
        }

        // Jump'n'Run control (:3398-3403).
        let control_style = self
            .players
            .get(&controller)
            .map(|player| player.control.control_style)
            .unwrap_or(false);
        if has_controller && control_style {
            return self.auto_stop_direct_com(index, com, data);
        }

        // Control by procedure (:3405-3556).
        self.object_procedure_com(index, com)
    }

    /// `C4Menu::Control` (C4Menu.cpp:433-480) for a script-created object
    /// menu. Returns false only when no menu is active.
    pub(in crate::direct_com) fn object_menu_control(
        &mut self,
        index: usize,
        com: u8,
        data: i32,
    ) -> Result<bool, EngineError> {
        let Some(menu) = self.objects[index].state.menu.clone() else {
            return Ok(false);
        };
        let object_id = self.objects[index].id;
        match com {
            COM_MENU_ENTER => {
                if !self.enter_internal_context_put(index, &menu, false)?
                    && !self.enter_internal_context_exit(index, &menu)?
                {
                    self.menu_user_enter(object_id, false)?;
                }
            }
            COM_MENU_ENTER_ALL => {
                if !self.enter_internal_context_put(index, &menu, true)?
                    && !self.enter_internal_context_exit(index, &menu)?
                {
                    self.menu_user_enter(object_id, true)?;
                }
            }
            COM_MENU_CLOSE => {
                let close_with_exit = menu.close_command == crate::ObjectMenuCloseCommand::Exit;
                if self.close_object_menu(object_id, false)? && close_with_exit {
                    // C4Object::AutoContextMenu's CloseCommand is invoked
                    // only for a control close (C4Menu.cpp:327-331), not
                    // when another menu force-replaces the context menu.
                    let owner = self.objects[index].state.owner;
                    self.player_object_command(owner, CommandId::Exit, None, 0, 0)?;
                    self.execute_object_command_now(object_id)?;
                }
            }
            COM_MENU_LEFT => {
                if !self.object_menu_step(object_id, &menu, -1)? {
                    let delta = if menu.selection - 1 < 0 {
                        menu.items.len() as i32 - 1 - menu.selection
                    } else {
                        -1
                    };
                    self.move_object_menu_selection(index, delta)?;
                }
            }
            COM_MENU_RIGHT => {
                if !self.object_menu_step(object_id, &menu, 1)? {
                    let delta = if menu.selection + 1 >= menu.items.len() as i32 {
                        -menu.selection
                    } else {
                        1
                    };
                    self.move_object_menu_selection(index, delta)?;
                }
            }
            COM_MENU_UP => {
                let columns = menu.columns;
                let mut delta = -columns;
                if menu.selection + delta < 0 && columns > 0 {
                    while menu.selection + delta + columns < menu.items.len() as i32 {
                        delta += columns;
                    }
                }
                self.move_object_menu_selection(index, delta)?;
            }
            COM_MENU_DOWN => {
                let columns = menu.columns;
                let mut delta = columns;
                if menu.selection + delta >= menu.items.len() as i32 && columns > 0 {
                    while menu.selection + delta - columns >= 0 {
                        delta -= columns;
                    }
                }
                self.move_object_menu_selection(index, delta)?;
            }
            COM_MENU_SELECT => {
                if !menu.items.is_empty() {
                    self.set_object_menu_selection(index, data & !C4MN_ADJUST_POSITION)?;
                }
            }
            COM_MENU_SHOW_TEXT => {
                if let Some(menu) = self.objects[index].state.menu.as_mut() {
                    menu.reveal_text();
                }
            }
            _ => {}
        }
        Ok(true)
    }

    /// Execute the engine-owned C4MN_Context Put row without routing its
    /// `PlayerObjectCommand` text through the script host-function table.
    /// C++ applies Put to every selected crew member, clamps the requested
    /// count to each inventory, and then synchronously executes the command
    /// object once (C4ObjectMenu.cpp:335-359; C4Player.cpp:1408-1423).
    pub(in crate::direct_com) fn enter_internal_context_put(
        &mut self,
        index: usize,
        menu: &crate::ObjectMenuState,
        right: bool,
    ) -> Result<bool, EngineError> {
        if menu.user_menu || !menu.permanent || menu.identification != Value::Int(14) {
            return Ok(false);
        }
        let Some(item) = usize::try_from(menu.selection)
            .ok()
            .and_then(|selection| menu.items.get(selection))
        else {
            return Ok(false);
        };
        if item.caption != "Put" {
            return Ok(false);
        }
        let object_id = self.objects[index].id;
        let owner = self.objects[index].state.owner;
        let Some(container) = self.objects[index].state.container else {
            return Ok(false);
        };
        let put_all = right && !item.command2.is_empty();
        self.player_context_put(owner, container, put_all)?;
        self.execute_object_command_now(object_id)?;
        Ok(true)
    }

    /// C4MN_Context's Exit row issues the player-wide Exit order, then
    /// executes it synchronously on the menu command object
    /// (C4ObjectMenu.cpp:426-433; C4ObjectCom.cpp:1013-1040).
    pub(in crate::direct_com) fn enter_internal_context_exit(
        &mut self,
        index: usize,
        menu: &crate::ObjectMenuState,
    ) -> Result<bool, EngineError> {
        if menu.user_menu || !menu.permanent || menu.identification != Value::Int(14) {
            return Ok(false);
        }
        let is_exit = usize::try_from(menu.selection)
            .ok()
            .and_then(|selection| menu.items.get(selection))
            .is_some_and(|item| item.symbol == crate::ObjectMenuSymbol::Exit);
        if !is_exit {
            return Ok(false);
        }
        let object_id = self.objects[index].id;
        let owner = self.objects[index].state.owner;
        self.player_object_command(owner, CommandId::Exit, None, 0, 0)?;
        self.execute_object_command_now(object_id)?;
        Ok(true)
    }

    pub(in crate::direct_com) fn player_context_put(
        &mut self,
        owner: i32,
        container: ObjectId,
        put_all: bool,
    ) -> Result<(), EngineError> {
        let cursor = self.crew_cursor(owner);
        let mut crew = self.selected_crew(owner);
        if let Some(cursor) = cursor.filter(|cursor| !crew.contains(cursor)) {
            crew.push(cursor);
        }
        for crew_id in crew {
            if crew_id == container {
                continue;
            }
            let Some(index) = self.find_object_index(crew_id) else {
                continue;
            };
            if !self.objects[index].has_nonzero_status() {
                continue;
            }
            let mut contents = self.objects[index]
                .state
                .contents
                .iter()
                .copied()
                .filter(|object_id| {
                    self.find_object_index(*object_id)
                        .is_some_and(|index| self.objects[index].has_nonzero_status())
                })
                .collect::<Vec<_>>();
            if !put_all {
                contents.truncate(1);
            }
            if contents.is_empty() {
                continue;
            }
            let count = if put_all {
                i32::try_from(contents.len()).unwrap_or(i32::MAX)
            } else {
                0
            };
            self.object_command_to_obj(
                index,
                CommandId::Put,
                Some(container),
                None,
                count,
                0,
                0,
                PlayerObjectCommandMode::Set,
                true,
            )?;
        }
        Ok(())
    }

    /// `C4Menu::MoveSelection` (C4Menu.cpp:535-555): advance in fixed
    /// increments until a selectable item is found, without crossing the
    /// menu bounds.
    pub(in crate::direct_com) fn move_object_menu_selection(
        &mut self,
        index: usize,
        delta: i32,
    ) -> Result<bool, EngineError> {
        if delta == 0 {
            return Ok(false);
        }
        let Some(menu) = self.objects[index].state.menu.as_ref() else {
            return Ok(false);
        };
        let mut selection = menu.selection;
        loop {
            selection += delta;
            let Some(item) = usize::try_from(selection)
                .ok()
                .and_then(|selection| menu.items.get(selection))
            else {
                return Ok(false);
            };
            if item.selectable {
                break;
            }
        }
        self.set_object_menu_selection(index, selection)?;
        Ok(true)
    }

    /// clonk-rs divergence: offer a horizontal menu com to the script before
    /// it becomes a selection move. Once `Columns == 1` — which every style
    /// but `C4MN_Style_Normal` forces (C4Menu.cpp:359-365) —
    /// `C4Menu::Control` gives COM_MenuLeft/Right exactly the deltas
    /// COM_MenuUp/Down already carry (C4Menu.cpp:433-457), so the horizontal
    /// pair says nothing the vertical pair does not. A user menu's own
    /// command object may claim them by implementing `OnMenuStep(iDelta,
    /// pMenuObject)` and returning true. Everything else — a menu that is not
    /// script-created, more than one column, no such function, a falsy
    /// return, a script error — falls through to the shipped selection move
    /// unchanged, so this is inert for content that does not ask for it.
    /// Dispatch mirrors `set_object_menu_selection` below, which ports
    /// `C4ObjectMenu::OnSelectionChanged` (C4ObjectMenu.cpp:93-104).
    pub(in crate::direct_com) fn object_menu_step(
        &mut self,
        object_id: ObjectId,
        menu: &crate::ObjectMenuState,
        delta: i32,
    ) -> Result<bool, EngineError> {
        if !menu.user_menu || menu.columns != 1 {
            return Ok(false);
        }
        let args = vec![Value::Int(delta), compat::object_reference_value(object_id)];
        let handled = if menu.scenario_callbacks {
            self.call_scenario_script_value("OnMenuStep", &args)?
        } else {
            let Some(command_object) = menu.command_object else {
                return Ok(false);
            };
            let Some(command_index) =
                self.find_object_index(command_object)
                    .filter(|&command_index| {
                        self.definitions
                            .get(&self.objects[command_index].definition_id)
                            .is_some_and(|definition| definition.has_function("OnMenuStep"))
                    })
            else {
                return Ok(false);
            };
            tolerate_script_error(self.call_object_function(command_index, "OnMenuStep", args))?
        };
        Ok(handled.is_some_and(|value| value.as_bool()))
    }

    /// `C4Menu::SetSelection(..., fDoCalls=true)` +
    /// `C4ObjectMenu::OnSelectionChanged` (C4Menu.cpp:557-594;
    /// C4ObjectMenu.cpp:93-104).
    pub(in crate::direct_com) fn set_object_menu_selection(
        &mut self,
        index: usize,
        requested: i32,
    ) -> Result<(), EngineError> {
        let object_id = self.objects[index].id;
        let Some(mut menu) = self.objects[index].state.menu.clone() else {
            return Ok(());
        };
        let selectable = usize::try_from(requested)
            .ok()
            .and_then(|selection| menu.items.get(selection))
            .is_some_and(|item| item.selectable);
        if (requested == -1 && menu.items.is_empty()) || selectable {
            menu.selection = requested;
            self.objects[index].state.menu = Some(menu.clone());
        }
        if !menu.user_menu {
            return Ok(());
        }
        let args = vec![
            Value::Int(menu.selection),
            compat::object_reference_value(object_id),
        ];
        if menu.scenario_callbacks {
            let _ = self.call_scenario_script_value("OnMenuSelection", &args)?;
        } else if let Some(command_object) = menu.command_object {
            let Some(command_index) =
                self.find_object_index(command_object)
                    .filter(|&command_index| {
                        self.definitions
                            .get(&self.objects[command_index].definition_id)
                            .is_some_and(|definition| definition.has_function("OnMenuSelection"))
                    })
            else {
                return Ok(());
            };
            let _ = tolerate_script_error(self.call_object_function(
                command_index,
                "OnMenuSelection",
                args,
            ))?;
        }
        Ok(())
    }

    /// The `switch (GetProcedure())` tail of C4Object::DirectCom
    /// (C4Object.cpp:3406-3556).
    pub(in crate::direct_com) fn object_procedure_com(
        &mut self,
        index: usize,
        com: u8,
    ) -> Result<(), EngineError> {
        let owner = self.objects[index].state.owner;
        match self.object_procedure(index) {
            ActionProcedure::Walk => match com {
                COM_LEFT => self.object_com_movement(index, CommandDirection::Left)?,
                COM_RIGHT => self.object_com_movement(index, CommandDirection::Right)?,
                COM_DOWN => self.object_com_movement(index, CommandDirection::Stop)?,
                COM_UP => {
                    self.object_com_up(index)?;
                }
                COM_DOWN_D => {
                    self.object_com_down_double(index)?;
                }
                COM_DIG_S => {
                    // (:3416-3421)
                    if self.object_com_dig(index)? {
                        let direction = self.objects[index].state.direction;
                        self.objects[index].state.command_direction = match direction {
                            Direction::Right => CommandDirection::DownRight,
                            _ => CommandDirection::DownLeft,
                        };
                    }
                }
                COM_DIG_D => self.object_com_dig_double(index)?,
                COM_THROW => {
                    self.player_object_command(owner, CommandId::Throw, None, 0, 0)?;
                }
                _ => {}
            },
            ActionProcedure::Flight | ActionProcedure::Kneel | ActionProcedure::Throw => {
                match com {
                    COM_LEFT => self.object_com_movement(index, CommandDirection::Left)?,
                    COM_RIGHT => self.object_com_movement(index, CommandDirection::Right)?,
                    COM_DOWN => self.object_com_movement(index, CommandDirection::Stop)?,
                    COM_THROW => {
                        self.player_object_command(owner, CommandId::Throw, None, 0, 0)?;
                    }
                    _ => {}
                }
            }
            ActionProcedure::Scale => match com {
                COM_LEFT => {
                    if self.objects[index].state.direction == Direction::Left {
                        self.object_com_movement(index, CommandDirection::Stop)?;
                    } else {
                        self.object_com_movement(index, CommandDirection::Left)?;
                        self.object_com_let_go(index, -1)?;
                    }
                }
                COM_RIGHT => {
                    if self.objects[index].state.direction == Direction::Right {
                        self.object_com_movement(index, CommandDirection::Stop)?;
                    } else {
                        self.object_com_movement(index, CommandDirection::Right)?;
                        self.object_com_let_go(index, 1)?;
                    }
                }
                COM_UP => self.object_com_movement(index, CommandDirection::Up)?,
                COM_DOWN => self.object_com_movement(index, CommandDirection::Down)?,
                COM_THROW => {
                    self.player_object_command(owner, CommandId::Drop, None, 0, 0)?;
                }
                _ => {}
            },
            ActionProcedure::Hang => match com {
                COM_LEFT => self.object_com_movement(index, CommandDirection::Left)?,
                COM_RIGHT => self.object_com_movement(index, CommandDirection::Right)?,
                COM_UP => self.object_com_movement(index, CommandDirection::Stop)?,
                COM_DOWN => {
                    self.object_com_let_go(index, 0)?;
                }
                COM_THROW => {
                    self.player_object_command(owner, CommandId::Drop, None, 0, 0)?;
                }
                _ => {}
            },
            ActionProcedure::Dig => match com {
                COM_LEFT => {
                    // COMD_UpRight(2)..COMD_Left(7) rotates one step clockwise
                    // (:3468).
                    let com_dir = self.objects[index]
                        .state
                        .command_direction
                        .to_script_value();
                    if (2..=7).contains(&com_dir) {
                        if let Some(next) = CommandDirection::from_script_value(com_dir + 1) {
                            self.objects[index].state.command_direction = next;
                        }
                    }
                }
                COM_RIGHT => {
                    // COMD_Right(3)..COMD_UpLeft(8) rotates one step
                    // counter-clockwise (:3469).
                    let com_dir = self.objects[index]
                        .state
                        .command_direction
                        .to_script_value();
                    if (3..=8).contains(&com_dir) {
                        if let Some(next) = CommandDirection::from_script_value(com_dir - 1) {
                            self.objects[index].state.command_direction = next;
                        }
                    }
                }
                COM_DOWN => {
                    self.object_com_stop(index)?;
                }
                COM_DIG_D => self.object_com_dig_double(index)?,
                COM_DIG_S => {
                    // Dig mat 2 object request (:3472).
                    let data = self.objects[index].state.action.data;
                    self.objects[index].state.action.data = i32::from(data == 0);
                }
                _ => {}
            },
            ActionProcedure::Swim => match com {
                COM_LEFT => self.object_com_movement(index, CommandDirection::Left)?,
                COM_RIGHT => self.object_com_movement(index, CommandDirection::Right)?,
                COM_UP => {
                    self.object_com_movement(index, CommandDirection::Up)?;
                    self.object_com_up(index)?;
                }
                COM_DOWN => self.object_com_movement(index, CommandDirection::Down)?,
                COM_THROW => {
                    self.player_object_command(owner, CommandId::Drop, None, 0, 0)?;
                }
                COM_DIG_D => self.object_com_dig_double(index)?,
                _ => {}
            },
            ActionProcedure::Bridge | ActionProcedure::Build | ActionProcedure::Chop => {
                if com == COM_DOWN {
                    self.object_com_stop(index)?;
                }
            }
            ActionProcedure::Fight => match com {
                COM_LEFT => self.object_com_movement(index, CommandDirection::Left)?,
                COM_RIGHT => self.object_com_movement(index, CommandDirection::Right)?,
                COM_DOWN => {
                    self.object_com_stop(index)?;
                }
                _ => {}
            },
            ActionProcedure::Push => self.object_push_com(index, com)?,
            _ => {}
        }
        Ok(())
    }

    /// DFA_PUSH branch of DirectCom (C4Object.cpp:3506-3555).
    pub(in crate::direct_com) fn object_push_com(
        &mut self,
        index: usize,
        com: u8,
    ) -> Result<(), EngineError> {
        let owner = self.objects[index].state.owner;
        let controller = self.objects[index].state.controller;
        let target = self.objects[index].state.action.target;
        let target_index = target.and_then(|id| self.find_object_index(id));
        // New grab-control model: objects version >= 4,9,5,0 may overload
        // control of grabbing clonks (:3508-3518).
        let grab_control_overload = if let Some(target_index) = target_index {
            self.objects[target_index].state.controller = controller;
            self.definitions
                .get(&self.objects[target_index].definition_id)
                .is_none_or(|definition| definition.version_at_least([4, 9, 5, 0]))
        } else {
            false
        };
        // Call object control first in case it overloads (:3520-3523).
        if grab_control_overload {
            if let Some(target_index) = target_index {
                let clonk_id = self.objects[index].id;
                if self.object_call_control(target_index, controller, com, Some(clonk_id))? {
                    return Ok(());
                }
            }
        }
        // Clonk direct control (:3525-3549).
        match com {
            COM_LEFT => self.object_com_movement(index, CommandDirection::Left)?,
            COM_RIGHT => self.object_com_movement(index, CommandDirection::Right)?,
            COM_UP => {
                // Target -> enter, else comdir up for target straightening
                // (:3529-3536).
                if self.object_com_enter(target_index)? {
                    self.object_com_movement(index, CommandDirection::Stop)?;
                } else {
                    self.object_com_movement(index, CommandDirection::Up)?;
                }
            }
            COM_DOWN => self.object_com_movement(index, CommandDirection::Stop)?,
            COM_DOWN_D => {
                self.object_com_ungrab(index)?;
            }
            COM_THROW_D => {
                // Avoid breaking objects with non-default ControlThrow
                // (:3539-3544): with the overload active and a target without
                // its own ControlThrow the double falls through to Throw.
                let target_has_control_throw = target_index
                    .map(|target_index| self.object_has_function(target_index, "ControlThrow"))
                    .unwrap_or(true);
                if grab_control_overload && !target_has_control_throw {
                    self.player_object_command(owner, CommandId::Throw, None, 0, 0)?;
                }
            }
            COM_THROW => {
                self.player_object_command(owner, CommandId::Throw, None, 0, 0)?;
            }
            _ => {}
        }
        // Action target call control late for old objects (:3550-3553).
        // Re-read Action.Target because the hardcoded fallback may have
        // changed or cleared it before this call.
        if !grab_control_overload {
            if let Some(target_index) = self
                .objects
                .get(index)
                .and_then(|object| object.state.action.target)
                .and_then(|id| self.find_object_index(id))
            {
                let clonk_id = self.objects[index].id;
                let _ = self.object_call_control(target_index, controller, com, Some(clonk_id))?;
            }
        }
        Ok(())
    }

    /// `C4Object::AutoStopDirectCom` (C4Object.cpp:3559-3727) — the
    /// Jump'n'Run per-procedure fallbacks.
    pub(in crate::direct_com) fn auto_stop_direct_com(
        &mut self,
        index: usize,
        com: u8,
        _data: i32,
    ) -> Result<(), EngineError> {
        let owner = self.objects[index].state.owner;
        let controller = self.objects[index].state.controller;
        match self.object_procedure(index) {
            ActionProcedure::Walk => match com {
                COM_UP => {
                    self.object_com_up(index)?;
                }
                COM_DOWN => {
                    // Inhibit ControlDownSingle on freshly grabbed objects
                    // (:3569-3573).
                    if self.object_com_down_double(index)? {
                        if let Some(player) = self.players.get_mut(&controller) {
                            player.control.last_com = i32::from(COM_NONE);
                        }
                    }
                }
                COM_DIG_S => {
                    self.object_com_dig(index)?;
                }
                COM_DIG_D => self.object_com_dig_double(index)?,
                COM_THROW => {
                    self.player_object_command(owner, CommandId::Throw, None, 0, 0)?;
                }
                _ => self.auto_stop_update_com_dir(index)?,
            },
            ActionProcedure::Flight => match com {
                COM_THROW => {
                    // Drop when pressing left, right or down (:3584-3590).
                    let pressed = self
                        .players
                        .get(&controller)
                        .map(|player| player.control.pressed_coms)
                        .unwrap_or(0);
                    let drop_mask = (1 << COM_LEFT) | (1 << COM_RIGHT) | (1 << COM_DOWN);
                    if pressed & drop_mask != 0 {
                        self.player_object_command(owner, CommandId::Drop, None, 0, 0)?;
                    } else {
                        self.player_object_command(owner, CommandId::Throw, None, 0, 0)?;
                    }
                }
                _ => self.auto_stop_update_com_dir(index)?,
            },
            ActionProcedure::Kneel | ActionProcedure::Throw => match com {
                COM_THROW => {
                    self.player_object_command(owner, CommandId::Throw, None, 0, 0)?;
                }
                _ => self.auto_stop_update_com_dir(index)?,
            },
            ActionProcedure::Scale => match com {
                COM_LEFT => {
                    if self.objects[index].state.direction == Direction::Right {
                        self.object_com_let_go(index, -1)?;
                    } else {
                        self.auto_stop_update_com_dir(index)?;
                    }
                }
                COM_RIGHT => {
                    if self.objects[index].state.direction == Direction::Left {
                        self.object_com_let_go(index, 1)?;
                    } else {
                        self.auto_stop_update_com_dir(index)?;
                    }
                }
                COM_DIG => {
                    // (:3615; note the C++ fallthrough into COM_Throw's drop.)
                    let xdirf = if self.objects[index].state.direction == Direction::Left {
                        1
                    } else {
                        -1
                    };
                    self.object_com_let_go(index, xdirf)?;
                    self.player_object_command(owner, CommandId::Drop, None, 0, 0)?;
                }
                COM_THROW => {
                    self.player_object_command(owner, CommandId::Drop, None, 0, 0)?;
                }
                _ => self.auto_stop_update_com_dir(index)?,
            },
            ActionProcedure::Hang => match com {
                COM_DOWN | COM_DIG => {
                    self.object_com_let_go(index, 0)?;
                }
                COM_THROW => {
                    self.player_object_command(owner, CommandId::Drop, None, 0, 0)?;
                }
                _ => self.auto_stop_update_com_dir(index)?,
            },
            ActionProcedure::Dig => match com {
                COM_THROW | COM_DIG => {
                    let data = self.objects[index].state.action.data;
                    self.objects[index].state.action.data = i32::from(data == 0);
                }
                _ => self.auto_stop_update_com_dir(index)?,
            },
            ActionProcedure::Swim => match com {
                COM_UP => {
                    self.auto_stop_update_com_dir(index)?;
                    self.object_com_up(index)?;
                }
                COM_THROW => {
                    self.player_object_command(owner, CommandId::Drop, None, 0, 0)?;
                }
                COM_DIG_D => self.object_com_dig_double(index)?,
                _ => self.auto_stop_update_com_dir(index)?,
            },
            ActionProcedure::Bridge | ActionProcedure::Build | ActionProcedure::Chop => {
                if com == COM_DOWN {
                    self.object_com_stop(index)?;
                }
            }
            ActionProcedure::Fight => match com {
                COM_DOWN => {
                    self.object_com_stop(index)?;
                }
                _ => self.auto_stop_update_com_dir(index)?,
            },
            ActionProcedure::Push => self.auto_stop_push_com(index, com)?,
            _ => {}
        }
        Ok(())
    }

    /// DFA_PUSH branch of AutoStopDirectCom (C4Object.cpp:3668-3725).
    pub(in crate::direct_com) fn auto_stop_push_com(
        &mut self,
        index: usize,
        com: u8,
    ) -> Result<(), EngineError> {
        let owner = self.objects[index].state.owner;
        let controller = self.objects[index].state.controller;
        let target = self.objects[index].state.action.target;
        let target_index = target.and_then(|id| self.find_object_index(id));
        let grab_control_overload = target_index.is_some_and(|target_index| {
            self.definitions
                .get(&self.objects[target_index].definition_id)
                .is_none_or(|definition| definition.version_at_least([4, 9, 5, 0]))
        });
        if grab_control_overload {
            if let Some(target_index) = target_index {
                let clonk_id = self.objects[index].id;
                if self.object_call_control(target_index, controller, com, Some(clonk_id))? {
                    return Ok(());
                }
            }
        }
        match com {
            COM_UP => {
                if self.object_com_enter(target_index)? {
                    self.object_com_movement(index, CommandDirection::Stop)?;
                } else {
                    self.auto_stop_update_com_dir(index)?;
                }
            }
            COM_DOWN => {
                // C++ queries the three Down command slots and only ungrabs
                // when none is visible for this player's control style
                // (C4Object.cpp:3712-3721; C4Object.cpp:2938-2951).
                let target_has_down_command = target_index.is_some_and(|target_index| {
                    ["ControlDown", "ControlDownSingle", "ControlDownDouble"]
                        .iter()
                        .any(|function| {
                            self.object_control_command_is_visible(
                                target_index,
                                controller,
                                function,
                            )
                        })
                });
                if target_index.is_some() && !target_has_down_command {
                    self.object_com_ungrab(index)?;
                }
            }
            COM_DOWN_D => {
                self.object_com_ungrab(index)?;
            }
            COM_THROW_D => {
                let target_has_control_throw = target_index
                    .map(|target_index| self.object_has_function(target_index, "ControlThrow"))
                    .unwrap_or(true);
                if grab_control_overload && !target_has_control_throw {
                    self.player_object_command(owner, CommandId::Drop, None, 0, 0)?;
                }
            }
            COM_THROW => {
                self.player_object_command(owner, CommandId::Drop, None, 0, 0)?;
            }
            _ => self.auto_stop_update_com_dir(index)?,
        }
        if !grab_control_overload {
            if let Some(target_index) = self
                .objects
                .get(index)
                .and_then(|object| object.state.action.target)
                .and_then(|id| self.find_object_index(id))
            {
                let clonk_id = self.objects[index].id;
                let _ = self.object_call_control(target_index, controller, com, Some(clonk_id))?;
            }
        }
        Ok(())
    }

    /// `C4Object::AutoStopUpdateComDir` (C4Object.cpp:3729-3741).
    pub(in crate::direct_com) fn auto_stop_update_com_dir(
        &mut self,
        index: usize,
    ) -> Result<(), EngineError> {
        let controller = self.objects[index].state.controller;
        let Some(player) = self.players.get(&controller) else {
            return Ok(());
        };
        if self.crew_cursor(controller) != Some(self.objects[index].id) {
            return Ok(());
        }
        let new_com_dir = coms_to_com_dir(player.control.pressed_coms);
        if self.objects[index].state.command_direction == new_com_dir {
            return Ok(());
        }
        if new_com_dir == CommandDirection::Stop
            && self.object_procedure(index) == ActionProcedure::Dig
        {
            self.object_com_stop(index)?;
            return Ok(());
        }
        self.object_com_movement(index, new_com_dir)
    }

    /// `C4Object::ContainedControl` (C4Object.cpp:3219-3305).
    pub(crate) fn object_contained_control(
        &mut self,
        index: usize,
        com: u8,
    ) -> Result<bool, EngineError> {
        let Some(container_id) = self.objects[index].state.container else {
            return Ok(false);
        };
        let Some(container_index) = self.find_object_index(container_id) else {
            return Ok(false);
        };
        // Check if object is about to exit a structure (:3223-3230).
        if (com == COM_LEFT || com == COM_RIGHT)
            && self.objects[index].commands.front_command_name() == Some("Exit")
            && self.objects[container_index].state.category & crate::CATEGORY_STRUCTURE != 0
        {
            return Ok(false);
        }
        let owner = self.objects[index].state.owner;
        let controller = self.objects[index].state.controller;
        let function = format!("Contained{}", com_name_raw(com));
        let callback_definition_id = self.objects[container_index].definition_id.clone();
        let sf = self.object_script_callback(container_index, &function);
        // New definitions may overload hardcoded controls; old definitions
        // receive the callback only after those controls have run
        // (C4Object.cpp:3246-3251,3284-3291).
        let call_sf_early = self
            .definitions
            .get(&self.objects[container_index].definition_id)
            .is_none_or(|definition| definition.version_at_least([4, 9, 1, 3]));
        let mut result = false;
        if call_sf_early {
            if let Some(sf) = sf.as_ref() {
                let clonk_ref = compat::object_reference_value(self.objects[index].id);
                let value = self.contained_direct_callback(
                    Some(container_index),
                    &callback_definition_id,
                    sf,
                    &[clonk_ref],
                )?;
                if compat::value_raw_truthy(&value) {
                    result = true;
                }
            }
            // AutoStopControl: notify container about the control update
            // (:3242-3249).
            self.contained_control_update(index, com, controller)?;
        }
        if result {
            return Ok(true);
        }

        // Hardcoded actions (:3253-3281).
        match com {
            COM_DOWN => {
                self.player_object_command(owner, CommandId::Exit, None, 0, 0)?;
            }
            COM_THROW_D => {
                // Avoid breaking objects with non-default ContainedThrow
                // (:3259-3265): only fall through when no such override.
                let container_index_now = self
                    .objects
                    .get(index)
                    .and_then(|object| object.state.container)
                    .and_then(|id| self.find_object_index(id));
                let has_contained_throw = container_index_now
                    .map(|idx| self.object_has_function(idx, "ContainedThrow"))
                    .unwrap_or(false);
                if !has_contained_throw {
                    let object_id = self.objects[index].id;
                    self.player_object_command(owner, CommandId::Throw, None, 0, 0)?;
                    self.execute_object_command_now(object_id)?;
                }
            }
            COM_THROW => {
                // `PlayerObjectCommand(...) && ExecuteCommand()`
                // (C4Object.cpp:3280-3282):
                // execute the calling clonk's freshly queued command before
                // returning from ContainedControl.
                let object_id = self.objects[index].id;
                self.player_object_command(owner, CommandId::Throw, None, 0, 0)?;
                self.execute_object_command_now(object_id)?;
            }
            COM_UP => {
                // Base buy menu (:3269-3274): ValidPlr(Contained->Base),
                // not hostile, BASEFUNC_Buy → ActivateMenu(C4MN_Buy).
                self.contained_base_menu(index, /* buy */ true)?;
            }
            COM_DIG => {
                // Base sell menu (:3275-3280): the BASEFUNC_Sell twin.
                self.contained_base_menu(index, /* buy */ false)?;
            }
            _ => {}
        }
        if !call_sf_early {
            if let Some(sf) = sf.as_ref() {
                let container_index = self
                    .objects
                    .get(index)
                    .and_then(|object| object.state.container)
                    .and_then(|id| self.find_object_index(id));
                let clonk_ref = compat::object_reference_value(self.objects[index].id);
                let _ = self.contained_direct_callback(
                    container_index,
                    &callback_definition_id,
                    sf,
                    &[clonk_ref],
                )?;
            }
            self.contained_control_update(index, com, controller)?;
        }
        // Take/Take2 (:3293-3302).
        if sf.is_none() || call_sf_early {
            match com {
                COM_LEFT => {
                    self.player_object_command(owner, CommandId::Take, None, 0, 0)?;
                }
                COM_RIGHT => {
                    self.player_object_command(owner, CommandId::Take2, None, 0, 0)?;
                }
                _ => {}
            }
        }
        Ok(true)
    }

    /// The base buy/sell menu arms of ContainedControl
    /// (C4Object.cpp:3269-3280): ValidPlr(Contained->Base), not hostile to
    /// the clonk's Owner, and the scenario's BASEFUNC bit set →
    /// ActivateMenu(C4MN_Buy/C4MN_Sell) on the clonk with the container as
    /// target. C4Object owns this permanent menu directly.
    pub(in crate::direct_com) fn contained_base_menu(
        &mut self,
        index: usize,
        buy: bool,
    ) -> Result<(), EngineError> {
        // Re-resolve the container: the early Contained{Com} script may
        // have moved the clonk.
        let Some(container_index) = self
            .objects
            .get(index)
            .and_then(|object| object.state.container)
            .and_then(|id| self.find_object_index(id))
        else {
            return Ok(());
        };
        let base = self.objects[container_index].state.base;
        if !self.players.contains_key(&base) {
            return Ok(());
        }
        let owner = self.objects[index].state.owner;
        if self.players_hostile(owner, base) {
            return Ok(());
        }
        let enabled = if buy {
            self.base_buy_enabled
        } else {
            self.base_sell_enabled
        };
        if !enabled {
            return Ok(());
        }
        if buy {
            self.open_base_buy_menu(index, container_index)?;
        } else {
            self.open_base_sell_menu(index, container_index)?;
        }
        Ok(())
    }
}
