//! `C4Player::ObjectCom` cursor selection and the mouse-control routing
//! that resolves a world point to a crew member.

use super::*;

impl Engine {
    /// `C4Player::ObjectCom` (C4Player.cpp:1367-1390): commit the cursor
    /// selection on regular coms, then route the com to the cursor object
    /// with an updated controller.
    pub(in crate::direct_com) fn player_object_com(
        &mut self,
        owner: i32,
        com: u8,
        data: i32,
    ) -> Result<(), EngineError> {
        // Eliminated (:1369).
        if self.is_owner_eliminated(owner) {
            return Ok(());
        }
        // ObjectCom hides the startup control hint before selection commits
        // or any cursor callback can reflect the player (C4Player.cpp:
        // 1368-1379). Cursor-cycle DirectCom paths above deliberately bypass
        // this write.
        if let Some(player) = self.players.get_mut(&owner) {
            player.hide_startup();
        }
        // If regular com, update cursor & selection status (:1378-1379).
        let is_release = (COM_RELEASE_FIRST..=COM_RELEASE_LAST).contains(&com);
        if com & (COM_SINGLE | COM_DOUBLE) == 0 && !is_release {
            self.player_update_selection_toggle_status(owner)?;
        }
        self.ensure_cursor(owner)?;
        let Some(cursor) = self.crew_cursor(owner) else {
            return Ok(());
        };
        let Some(index) = self.find_object_index(cursor) else {
            return Ok(());
        };
        self.objects[index].state.controller = owner;
        self.object_direct_com(index, com, data)
    }

    // ---- Cursor selection model (C4Player.cpp:1235-1365) ------------------

    /// `C4Object::DoSelect` (C4Object.cpp:5815-5824): CrewDisabled guard,
    /// the Select flag unless cursor-only, and the `~CrewSelection(false,
    /// fCursor)` callback.
    pub(crate) fn object_do_select(
        &mut self,
        index: usize,
        _owner: i32,
        cursor_only: bool,
    ) -> Result<(), EngineError> {
        if self.objects[index].state.crew_disabled {
            return Ok(());
        }
        if !cursor_only {
            self.objects[index].state.selected = true;
        }
        self.contained_call(
            index,
            "CrewSelection",
            &[Value::Bool(false), Value::Bool(cursor_only)],
        )?;
        Ok(())
    }

    /// `C4Object::UnSelect` (C4Object.cpp:5826-5832).
    pub(crate) fn object_un_select(
        &mut self,
        index: usize,
        _owner: i32,
        cursor_only: bool,
    ) -> Result<(), EngineError> {
        if !cursor_only {
            self.objects[index].state.selected = false;
        }
        self.contained_call(
            index,
            "CrewSelection",
            &[Value::Bool(true), Value::Bool(cursor_only)],
        )?;
        Ok(())
    }

    /// `C4Player::SetCursor` (C4Player.cpp:1831-1847).
    pub(crate) fn player_set_cursor(
        &mut self,
        owner: i32,
        target: Option<ObjectId>,
        select_flash: bool,
        select_arrow: bool,
    ) -> Result<(), EngineError> {
        // Check disabled (:1834).
        let target_index = target.and_then(|target| self.find_object_index(target));
        if target.is_some() && target_index.is_none() {
            return Ok(());
        }
        if target_index.is_some_and(|index| self.objects[index].state.crew_disabled) {
            return Ok(());
        }
        let previous = self.crew_cursor(owner);
        let changed = previous != target;
        if let Some(target) = target {
            self.crew_selection
                .entry(owner)
                .or_default()
                .set_cursor(Some(target));
        } else {
            self.crew_selection.remove(&owner);
        }
        // Cursor is assigned before either callback (C4Player.cpp:1838), so
        // callback-side GetCursor observes the new object/null immediately.
        if let Some(player) = self.players.get_mut(&owner) {
            player.set_cursor(target);
        }
        // Unselect previous (:1841).
        if let Some(previous_index) = previous
            .filter(|_| changed)
            .and_then(|id| self.find_object_index(id))
        {
            self.object_un_select(previous_index, owner, true)?;
        }
        // Select object (:1843).
        if let Some(target_index) = target_index {
            self.object_do_select(target_index, owner, true)?;
        }
        if let Some(player) = self.players.get_mut(&owner) {
            if select_arrow {
                player.control.cursor_flash = 30;
            }
            if select_flash {
                player.control.select_flash = 30;
            }
        }
        Ok(())
    }

    /// The player's raw C4Player::Crew order. Inactive objects remain linked;
    /// only a deleted pointer disappears through ClearPointers.
    pub(in crate::direct_com) fn player_crew_roster(&self, owner: i32) -> Vec<ObjectId> {
        self.players
            .get(&owner)
            .map(|player| player.crew().to_vec())
            .unwrap_or_else(|| self.crew_members(owner))
            .into_iter()
            .filter(|id| {
                self.find_object_index(*id).is_some_and(|index| {
                    let object = &self.objects[index];
                    !object.destroyed && object.state.status != crate::ObjectStatus::Deleted
                })
            })
            .collect()
    }

    /// `C4Player::GetHiRankActiveCrew` (C4Player.cpp:1003-1021): the
    /// strictly highest-ranked eligible member wins. Equal ranks retain the
    /// FIRST eligible roster entry because the C++ loop only replaces its
    /// candidate for `iRank > iHighestRank`.
    pub(crate) fn player_hi_rank_active_crew(
        &self,
        owner: i32,
        select_only: bool,
    ) -> Option<ObjectId> {
        let selected = self.selected_crew(owner);
        let mut highest: Option<(ObjectId, i32)> = None;
        for id in self.player_crew_roster(owner) {
            let eligible = self
                .find_object_index(id)
                .is_some_and(|index| !self.objects[index].state.crew_disabled)
                && (!select_only || selected.contains(&id));
            if !eligible {
                continue;
            }
            let rank = self.crew_ranks.get(&id.as_u64()).copied().unwrap_or(-1);
            match highest {
                Some((_, highest_rank)) if highest_rank >= rank => {}
                _ => highest = Some((id, rank)),
            }
        }
        highest.map(|(id, _)| id)
    }

    /// `C4Player::AdjustCursorCommand` (C4Player.cpp:1235-1258).
    pub(crate) fn player_adjust_cursor_command(&mut self, owner: i32) -> Result<(), EngineError> {
        // ResetCursorView runs before the replacement search while the old
        // ViewCursor is still live (C4Player.cpp:1241-1244).
        if let Some(player) = self.players.get_mut(&owner) {
            player.reset_cursor_view();
        }
        // Find hirank Select, else any (:1240-1245).
        let hi_rank = self
            .player_hi_rank_active_crew(owner, true)
            .or_else(|| self.player_hi_rank_active_crew(owner, false));
        let previous = self
            .players
            .get(&owner)
            .and_then(|player| player.cursor())
            .or_else(|| self.crew_cursor(owner));
        if previous != hi_rank {
            self.crew_selection
                .entry(owner)
                .or_default()
                .set_cursor(hi_rank);
            if let Some(player) = self.players.get_mut(&owner) {
                player.set_cursor(hi_rank);
            }
            // UpdateView precedes both selection callbacks and resolves the
            // still-live ViewCursor before ClearPointers' suffix clears it.
            self.update_player_view(owner);
        }
        // UnSelect previous cursor (:1253).
        if let Some(previous_index) = previous
            .filter(|id| Some(*id) != hi_rank)
            .and_then(|id| self.find_object_index(id))
        {
            self.object_un_select(previous_index, owner, true)?;
        }
        // We have a cursor: do select it (:1255) — the non-cursor DoSelect
        // sets the Select flag too.
        let live_cursor = self
            .players
            .get(&owner)
            .and_then(|player| player.cursor())
            .or_else(|| self.crew_cursor(owner));
        if let Some(cursor_index) = live_cursor.and_then(|id| self.find_object_index(id)) {
            self.object_do_select(cursor_index, owner, false)?;
        }
        if let Some(player) = self.players.get_mut(&owner) {
            player.control.cursor_flash = 30;
        }
        Ok(())
    }

    /// `C4Player::CursorRight` (C4Player.cpp:1261-1275).
    pub(in crate::direct_com) fn player_cursor_right(
        &mut self,
        owner: i32,
    ) -> Result<(), EngineError> {
        self.player_cursor_step(owner, false)
    }

    /// `C4Player::CursorLeft` (C4Player.cpp:1278-1293).
    pub(in crate::direct_com) fn player_cursor_left(
        &mut self,
        owner: i32,
    ) -> Result<(), EngineError> {
        self.player_cursor_step(owner, true)
    }

    pub(in crate::direct_com) fn player_cursor_step(
        &mut self,
        owner: i32,
        backwards: bool,
    ) -> Result<(), EngineError> {
        let mut roster = self.player_crew_roster(owner);
        if backwards {
            roster.reverse();
        }
        let eligible = |engine: &Self, id: ObjectId| {
            engine
                .find_object_index(id)
                .is_some_and(|index| !engine.objects[index].state.crew_disabled)
        };
        // Walk on from the cursor's link; falling off the end rescans the
        // whole list from the front (C4Player.cpp:1264-1270).
        let next = self
            .crew_cursor(owner)
            .and_then(|cursor| roster.iter().position(|id| *id == cursor))
            .and_then(|position| {
                roster[position + 1..]
                    .iter()
                    .copied()
                    .find(|id| eligible(self, *id))
            })
            .or_else(|| roster.iter().copied().find(|id| eligible(self, *id)));
        if let Some(target) = next {
            self.player_set_cursor(owner, Some(target), false, true)?;
        }
        // Updates (:1272-1274).
        if let Some(player) = self.players.get_mut(&owner) {
            player.control.cursor_flash = 30;
            player.control.cursor_selection = 1;
        }
        Ok(())
    }

    /// The object number queued by `C4MouseControl::SendPlayerSelectNext`:
    /// advance in crew-list order, wrapping to the first eligible member
    /// (C4MouseControl.cpp:1284-1300).
    pub fn player_mouse_select_next_object(&self, owner: i32) -> Option<ObjectId> {
        if !self.players.contains_key(&owner) {
            return None;
        }
        let roster = self.player_crew_roster(owner);
        let eligible = |engine: &Self, id: ObjectId| {
            engine.find_object_index(id).is_some_and(|index| {
                !engine.objects[index].destroyed
                    && engine.objects[index].state.status != crate::ObjectStatus::Deleted
                    && !engine.objects[index].state.crew_disabled
            })
        };
        let next = self
            .crew_cursor(owner)
            .and_then(|cursor| roster.iter().position(|id| *id == cursor))
            .and_then(|position| {
                roster[position + 1..]
                    .iter()
                    .copied()
                    .find(|id| eligible(self, *id))
            })
            .or_else(|| roster.iter().copied().find(|id| eligible(self, *id)));
        next
    }

    /// Select-next followed by the synchronized one-object packet execution.
    pub fn player_mouse_select_next(&mut self, owner: i32) -> Result<bool, EngineError> {
        let Some(next) = self.player_mouse_select_next_object(owner) else {
            return Ok(false);
        };
        self.execute_player_select(&PlayerSelectControlData {
            player: owner,
            objects: vec![next.as_u64() as i32],
            by_client: -1,
        })
    }

    /// Crew objects inside C4MouseControl's landscape drag frame, in the
    /// player's stored crew-list order. The mouse oracle compares object
    /// origins (not shape rectangles), includes both frame edges, and skips
    /// CrewDisabled entries (C4MouseControl.cpp:610-624).
    pub fn mouse_drag_crew_in_rect(
        &self,
        owner: i32,
        first: Vector2,
        second: Vector2,
    ) -> Vec<ObjectId> {
        let min_x = first.x.min(second.x);
        let max_x = first.x.max(second.x);
        let min_y = first.y.min(second.y);
        let max_y = first.y.max(second.y);
        self.player_crew_roster(owner)
            .into_iter()
            .filter(|id| {
                self.find_object_index(*id).is_some_and(|index| {
                    let object = &self.objects[index];
                    !object.state.crew_disabled
                        && (min_x..=max_x).contains(&object.state.position.x)
                        && (min_y..=max_y).contains(&object.state.position.y)
                })
            })
            .collect()
    }

    /// Carryable objects inside C4MouseControl's landscape drag frame, in the
    /// C++ `Game.Objects` main-list order and capped at 20. Object origins are
    /// compared against inclusive frame edges; contained objects never enter
    /// this local mouse selection (C4MouseControl.cpp:626-645).
    pub fn mouse_drag_carryables_in_rect(&self, first: Vector2, second: Vector2) -> Vec<ObjectId> {
        let min_x = first.x.min(second.x);
        let max_x = first.x.max(second.x);
        let min_y = first.y.min(second.y);
        let max_y = first.y.max(second.y);
        // `exec_list` is the reverse of C++'s main list (lib.rs:11372-11380).
        self.execution
            .exec_list
            .iter()
            .rev()
            .filter_map(|id| {
                self.find_object_index(*id)
                    .map(|index| &self.objects[index])
            })
            .filter(|object| {
                object.state.status.is_active()
                    && object.state.ocf & ocf::CARRYABLE != 0
                    && object.state.container.is_none()
                    && (min_x..=max_x).contains(&object.state.position.x)
                    && (min_y..=max_y).contains(&object.state.position.y)
            })
            .map(|object| object.id)
            .take(20)
            .collect()
    }

    /// The carryable-object cursor selected by `C4MouseControl::DragMoving`
    /// at a world pixel. Control-modified Put is deliberately left to the
    /// separate region/container slice; ordinary liquid/near-ground Drop and
    /// ballistic Throw are exact (C4MouseControl.cpp:833-879;
    /// C4Landscape.cpp:2055-2100).
    pub fn mouse_drag_carryable_cursor(
        &mut self,
        owner: i32,
        position: Vector2,
    ) -> Option<MouseDragCarryableCursor> {
        {
            let landscape = self.landscape.as_ref()?;
            if landscape.is_liquid_at(position.x, position.y) {
                return Some(MouseDragCarryableCursor::Drop);
            }
            if landscape.is_solid_at(position.x, position.y) {
                return None;
            }

            let mut ground_y = position.y;
            let landscape_height = landscape.estimated_height();
            while ground_y < landscape_height && !landscape.is_solid_at(position.x, ground_y) {
                ground_y += 1;
            }
            if (ground_y - position.y).abs() <= 5 {
                return Some(MouseDragCarryableCursor::Drop);
            }
        }

        let throw_force = self
            .crew_cursor(owner)
            .and_then(|id| self.find_object_index(id))
            .map(|index| math::val_by_physical(400, self.object_physical(index).throw))
            .unwrap_or_else(|| math::val_by_physical(400, 50_000));
        // GetPhysical may fill a fair-crew projection and execute definition
        // script. Native re-reads pPlayer->Cursor separately for direction
        // and throwing height after that call (C4MouseControl.cpp:866-870).
        let (cursor_x, throw_height) = self
            .crew_cursor(owner)
            .and_then(|id| self.find_object_index(id))
            .map(|index| {
                (
                    self.objects[index].state.position.x,
                    self.objects[index]
                        .current_shape_rect()
                        .map(|rect| rect.height)
                        .unwrap_or(20),
                )
            })
            .unwrap_or((position.x, 20));
        let preferred_direction = if cursor_x > position.x { -1 } else { 1 };
        let landscape = self.landscape.as_ref()?;
        [preferred_direction, -preferred_direction]
            .into_iter()
            .find_map(|direction| {
                landscape
                    .find_throwing_position(
                        position,
                        FixedVec2::new(throw_force * direction, -throw_force),
                        throw_height,
                        self.physics.gravity_as_c4fixed(),
                    )
                    .map(|landing| MouseDragCarryableCursor::Throw { direction, landing })
            })
    }

    pub fn mouse_drag_carryable_command(
        &mut self,
        owner: i32,
        position: Vector2,
    ) -> Option<CommandId> {
        self.mouse_drag_carryable_cursor(owner, position)
            .map(|cursor| match cursor {
                MouseDragCarryableCursor::Drop => CommandId::Drop,
                MouseDragCarryableCursor::Throw { .. } => CommandId::Throw,
            })
    }

    pub(in crate::direct_com) fn mouse_world_point_is_solid(&self, point: Vector2) -> bool {
        let Some(landscape) = self.landscape.as_ref() else {
            return false;
        };
        landscape.is_solid_at(point.x, point.y)
            || self
                .ocf_solid_mask_overlay()
                .iter()
                .any(|mask| mask.contains(point.x, point.y))
    }

    /// Reproduce `C4MouseControl::UpdateCursorTarget`'s world-cursor
    /// priority. A picked object first replaces the landscape cursor with a
    /// crosshair; each later matching object cursor then overrides the
    /// previous one, and the nearby jump cursor is evaluated last
    /// (C4MouseControl.cpp:451-538).
    pub fn mouse_world_cursor(
        &self,
        owner: i32,
        target: Option<ObjectId>,
        point: Vector2,
        control_down: bool,
    ) -> MouseWorldCursor {
        let mut cursor = if self.mouse_world_point_is_solid(point) {
            MouseWorldCursor::Dig {
                material: control_down,
            }
        } else {
            MouseWorldCursor::Crosshair
        };

        if let Some(target) = target {
            let Some(index) = self.find_object_index(target) else {
                return cursor;
            };
            let object = &self.objects[index];
            if !object.state.status.is_active() || object.state.container.is_some() {
                return cursor;
            }

            // Any object admitted by the primary OCF pick suppresses the
            // landscape dig cursor, even if it was admitted by OCF_Exclusive
            // and has no actionable cursor of its own.
            cursor = MouseWorldCursor::Crosshair;
            let target_ocf = self.object_ocf_for_pos(index, point);

            // The first entrance check intentionally uses cached Entrance:
            // containers remain enterable across their whole shape. The
            // ordinary position-filtered Entrance check below runs later.
            if target_ocf & ocf::CONTAINER != 0 && object.state.ocf & ocf::ENTRANCE != 0 {
                cursor = MouseWorldCursor::Enter(target);
            }
            if target_ocf & ocf::GRAB != 0 {
                let pushing_target = self
                    .crew_cursor(owner)
                    .and_then(|cursor| self.find_object_index(cursor))
                    .is_some_and(|cursor_index| {
                        self.object_procedure(cursor_index) == ActionProcedure::Push
                            && self.objects[cursor_index].state.action.target == Some(target)
                    });
                cursor = if pushing_target {
                    MouseWorldCursor::Ungrab(target)
                } else {
                    MouseWorldCursor::Grab(target)
                };
            }
            if target_ocf & ocf::CARRYABLE != 0 {
                cursor = if target_ocf & ocf::IN_SOLID != 0 {
                    MouseWorldCursor::DigObject(target)
                } else {
                    MouseWorldCursor::Carryable(target)
                };
            }
            if target_ocf & ocf::CHOP != 0 {
                let width = object
                    .current_shape_rect()
                    .map(|shape| shape.width)
                    .unwrap_or(0);
                let dx = point.x - object.state.position.x;
                let dy = point.y - object.state.position.y;
                if (-width / 3..=width / 3).contains(&dx) && (-width / 2..=width / 3).contains(&dy)
                {
                    cursor = MouseWorldCursor::Chop(target);
                }
            }
            if target_ocf & ocf::ENTRANCE != 0 {
                cursor = MouseWorldCursor::Enter(target);
            }
            if target_ocf & ocf::CONSTRUCT != 0 {
                cursor = MouseWorldCursor::Build(target);
            }
            if target_ocf & ocf::ALIVE != 0 && self.player_crew_roster(owner).contains(&target) {
                cursor = MouseWorldCursor::Select(target);
            }
            if object.state.category & CATEGORY_MOUSE_SELECT != 0 {
                cursor = MouseWorldCursor::Select(target);
            }
            if object.state.ocf & ocf::ALIVE != 0
                && object.state.alive
                && self.players_hostile(owner, object.state.owner)
            {
                cursor = MouseWorldCursor::Attack(target);
            }
        }

        self.mouse_jump_cursor(owner, point).unwrap_or(cursor)
    }

    /// Build the exact `C4ControlPlayerCommand` produced by a world
    /// LeftDouble event. Selection and Jump cursors deliberately do not emit
    /// a double-click command (C4MouseControl.cpp:982-1007).
    pub fn mouse_left_double_command(
        &self,
        owner: i32,
        target: Option<ObjectId>,
        point: Vector2,
        control_down: bool,
        shift_down: bool,
    ) -> Option<PlayerCommandControlData> {
        if !self.players.contains_key(&owner) || self.is_owner_eliminated(owner) {
            return None;
        }

        let (action, target) = match self.mouse_world_cursor(owner, target, point, control_down) {
            MouseWorldCursor::Attack(target) => (MouseDoubleClickAction::Attack, Some(target)),
            MouseWorldCursor::Grab(target) => (MouseDoubleClickAction::Grab, Some(target)),
            MouseWorldCursor::Ungrab(target) => (MouseDoubleClickAction::Ungrab, Some(target)),
            MouseWorldCursor::Build(target) => (MouseDoubleClickAction::Build, Some(target)),
            MouseWorldCursor::Chop(target) => (MouseDoubleClickAction::Chop, Some(target)),
            MouseWorldCursor::Enter(target) => (MouseDoubleClickAction::Enter, Some(target)),
            MouseWorldCursor::Carryable(target) | MouseWorldCursor::DigObject(target) => {
                (MouseDoubleClickAction::Get, Some(target))
            }
            MouseWorldCursor::Dig { material } => (MouseDoubleClickAction::Dig { material }, None),
            MouseWorldCursor::Crosshair
            | MouseWorldCursor::Select(_)
            | MouseWorldCursor::JumpLeft
            | MouseWorldCursor::JumpRight => return None,
        };

        self.mouse_left_double_command_for_action(owner, action, target, point, shift_down)
    }

    /// Build LeftDouble from the cursor and target identities retained by the
    /// last Move/Tick5 refill (C4MouseControl.cpp:982-1007).
    pub fn mouse_left_double_command_for_action(
        &self,
        owner: i32,
        action: MouseDoubleClickAction,
        target: Option<ObjectId>,
        point: Vector2,
        shift_down: bool,
    ) -> Option<PlayerCommandControlData> {
        if !self.players.contains_key(&owner) || self.is_owner_eliminated(owner) {
            return None;
        }

        let target = target.map_or(0, |target| target.as_u64() as i32);
        let (command, x, y, target, data) = match action {
            MouseDoubleClickAction::Attack => (CommandId::Attack, point.x, point.y, target, 0),
            MouseDoubleClickAction::Grab => (CommandId::Grab, 0, 0, target, 0),
            MouseDoubleClickAction::Ungrab => (CommandId::UnGrab, point.x, point.y, target, 0),
            MouseDoubleClickAction::Build => (CommandId::Build, point.x, point.y, target, 0),
            MouseDoubleClickAction::Chop => (CommandId::Chop, point.x, point.y, target, 0),
            MouseDoubleClickAction::Enter => (CommandId::Enter, point.x, point.y, target, 0),
            MouseDoubleClickAction::Get => (CommandId::Get, 0, 0, target, 0),
            MouseDoubleClickAction::Dig { material } => {
                (CommandId::Dig, point.x, point.y, 0, i32::from(material))
            }
        };

        Some(PlayerCommandControlData {
            player: owner,
            command: command as i32,
            x,
            y,
            target,
            target2: 0,
            data,
            add_mode: C4P_COMMAND_SET | if shift_down { C4P_COMMAND_APPEND } else { 0 },
            by_client: -1,
        })
    }

    /// Whether `point` selects the nearby jump cursor for the player's cursor
    /// object. UpdateCursorTarget evaluates this after every object cursor, so
    /// it also overrides Select (C4MouseControl.cpp:522-534).
    pub(in crate::direct_com) fn mouse_jump_cursor(
        &self,
        owner: i32,
        point: Vector2,
    ) -> Option<MouseWorldCursor> {
        if !self.players.contains_key(&owner) {
            return None;
        }
        self.crew_cursor(owner)
            .and_then(|cursor| self.find_object_index(cursor))
            .and_then(|cursor_index| {
                let cursor = &self.objects[cursor_index];
                if cursor.state.container.is_some()
                    || self.object_procedure(cursor_index) != ActionProcedure::Walk
                {
                    return None;
                }
                let dx = point.x - cursor.state.position.x;
                let dy = point.y - cursor.state.position.y;
                if !(-25..=-10).contains(&dy) {
                    return None;
                }
                if (-15..=-1).contains(&dx) {
                    Some(MouseWorldCursor::JumpLeft)
                } else if (1..=15).contains(&dx) {
                    Some(MouseWorldCursor::JumpRight)
                } else {
                    None
                }
            })
    }

    pub fn mouse_jump_zone(&self, owner: i32, point: Vector2) -> bool {
        self.mouse_jump_cursor(owner, point).is_some()
    }

    /// Classify the down cursor which may start a world-object moving drag.
    /// This follows UpdateCursorTarget's OCF priority through the later
    /// Chop/Enter/Build/Select/Attack/Jump overrides, then DragNone's strict
    /// `Def->Grab == 1` vehicle gate (C4MouseControl.cpp:474-538,922-941).
    pub fn mouse_world_drag_source(
        &self,
        owner: i32,
        target: ObjectId,
        point: Vector2,
    ) -> Option<MouseDragSource> {
        if !self.players.contains_key(&owner) {
            return None;
        }
        let index = self.find_object_index(target)?;
        let object = &self.objects[index];
        if !object.state.status.is_active() || object.state.container.is_some() {
            return None;
        }
        let grab = self.definitions.get(&object.definition_id)?.grab();
        match self.mouse_world_cursor(owner, Some(target), point, false) {
            MouseWorldCursor::Carryable(_) | MouseWorldCursor::DigObject(_) => {
                Some(MouseDragSource::Carryable)
            }
            MouseWorldCursor::Grab(_) | MouseWorldCursor::Ungrab(_) if grab == 1 => {
                Some(MouseDragSource::Vehicle)
            }
            _ => None,
        }
    }

    /// The moving-drag class for a copied viewport region target. Regions
    /// use cached OCF_Carryable but the definition's raw Grab=1 value rather
    /// than the world cursor's position-filtered OCF (C4MouseControl.cpp:
    /// 942-961).
    pub fn mouse_region_drag_source(&self, target: ObjectId) -> Option<MouseDragSource> {
        let index = self.find_object_index(target)?;
        let object = &self.objects[index];
        if object.state.ocf & ocf::CARRYABLE != 0 {
            return Some(MouseDragSource::Carryable);
        }
        self.definitions
            .get(&object.definition_id)
            .filter(|definition| definition.grab() == 1)
            .map(|_| MouseDragSource::Vehicle)
    }

    /// Build C4MouseControl's local Selection when dragging from a viewport
    /// region. A right drag expands when `Contents.ObjectCount(id)` finds
    /// multiple nonzero-Status matches. The raw link walk then inserts via
    /// `C4ObjectList::Add`, which applies the same Status filter; otherwise
    /// it tries to add only the copied region target
    /// (C4MouseControl.cpp:942-961).
    pub fn mouse_region_drag_objects(&self, target: ObjectId, right_button: bool) -> Vec<ObjectId> {
        if self.mouse_region_drag_source(target).is_none() {
            return Vec::new();
        }
        let Some(index) = self.find_object_index(target) else {
            return Vec::new();
        };
        let object = &self.objects[index];
        let single_target = || {
            object
                .has_nonzero_status()
                .then_some(vec![target])
                .unwrap_or_default()
        };
        let Some(container) = right_button.then_some(object.state.container).flatten() else {
            return single_target();
        };
        let Some(container_index) = self.find_object_index(container) else {
            return single_target();
        };
        let same_id = self.objects[container_index]
            .state
            .contents
            .iter()
            .copied()
            .filter(|candidate| {
                self.find_object_index(*candidate)
                    .is_some_and(|candidate_index| {
                        let candidate = &self.objects[candidate_index];
                        candidate.has_nonzero_status()
                            && candidate.definition_id == object.definition_id
                    })
            })
            .collect::<Vec<_>>();
        if same_id.len() > 1 {
            same_id
        } else {
            single_target()
        }
    }

    /// Execute the crew half of `C4ControlPlayerSelect`: replace the current
    /// selection, adjust the cursor, and arm the selection flash. Requested
    /// ids are rechecked against the live crew roster at execution time
    /// (C4Control.cpp:341-369; C4Player.cpp:1848-1862).
    pub fn player_mouse_select_crew<I>(
        &mut self,
        owner: i32,
        requested: I,
    ) -> Result<bool, EngineError>
    where
        I: IntoIterator<Item = ObjectId>,
    {
        if !self.players.contains_key(&owner) {
            return Ok(false);
        }
        let objects = requested
            .into_iter()
            .filter_map(|id| i32::try_from(id.as_u64()).ok())
            .collect();
        self.execute_player_select(&PlayerSelectControlData {
            player: owner,
            objects,
            by_client: -1,
        })
    }

    /// `C4Player::UnselectCrew` (C4Player.cpp:1295-1306).
    pub(crate) fn player_unselect_crew(&mut self, owner: i32) -> Result<(), EngineError> {
        let cursor = self.crew_cursor(owner);
        let mut cursor_deselected = false;
        for id in self.player_crew_roster(owner) {
            if cursor == Some(id) {
                cursor_deselected = true;
            }
            if let Some(index) = self.find_object_index(id) {
                self.object_un_select(index, owner, false)?;
            }
        }
        // A cursor outside the crew unselects too (:1305).
        if let Some(cursor_index) = cursor
            .filter(|_| !cursor_deselected)
            .and_then(|id| self.find_object_index(id))
        {
            self.object_un_select(cursor_index, owner, false)?;
        }
        Ok(())
    }

    /// `C4Player::SelectSingleByCursor` (C4Player.cpp:1308-1317).
    pub(in crate::direct_com) fn player_select_single_by_cursor(
        &mut self,
        owner: i32,
    ) -> Result<(), EngineError> {
        self.player_unselect_crew(owner)?;
        if let Some(cursor_index) = self
            .crew_cursor(owner)
            .and_then(|id| self.find_object_index(id))
        {
            self.object_do_select(cursor_index, owner, false)?;
        }
        if let Some(player) = self.players.get_mut(&owner) {
            player.control.select_flash = 30;
        }
        self.player_adjust_cursor_command(owner)
    }

    /// `C4Player::CursorToggle` (C4Player.cpp:1319-1339).
    pub(in crate::direct_com) fn player_cursor_toggle(
        &mut self,
        owner: i32,
    ) -> Result<(), EngineError> {
        let cursor_selection = self
            .players
            .get(&owner)
            .map(|player| player.control.cursor_selection)
            .unwrap_or(0);
        if cursor_selection != 0 {
            // Selection mode: toggle cursor select (:1323-1327).
            if let Some(cursor) = self.crew_cursor(owner) {
                let selected = self
                    .find_object_index(cursor)
                    .is_some_and(|index| self.objects[index].state.selected);
                if let Some(index) = self.find_object_index(cursor) {
                    if selected {
                        self.object_un_select(index, owner, false)?;
                    } else {
                        self.object_do_select(index, owner, false)?;
                    }
                }
            }
            if let Some(player) = self.players.get_mut(&owner) {
                player.control.cursor_toggled = 1;
            }
        } else {
            // Pure toggle: toggle all Select (:1329-1336).
            for id in self.player_crew_roster(owner) {
                let Some(index) = self.find_object_index(id) else {
                    continue;
                };
                if self.objects[index].state.crew_disabled {
                    continue;
                }
                let selected = self.objects[index].state.selected;
                if selected {
                    self.object_un_select(index, owner, false)?;
                } else {
                    self.object_do_select(index, owner, false)?;
                }
            }
            self.player_adjust_cursor_command(owner)?;
        }
        if let Some(player) = self.players.get_mut(&owner) {
            player.control.select_flash = 30;
        }
        Ok(())
    }

    /// `C4Player::SelectAllCrew` (C4Player.cpp:1341-1353).
    pub(in crate::direct_com) fn player_select_all_crew(
        &mut self,
        owner: i32,
    ) -> Result<(), EngineError> {
        for id in self.player_crew_roster(owner) {
            if let Some(index) = self.find_object_index(id) {
                self.object_do_select(index, owner, false)?;
            }
        }
        self.player_adjust_cursor_command(owner)?;
        if let Some(player) = self.players.get_mut(&owner) {
            player.control.cursor_selection = 0;
            player.control.cursor_toggled = 0;
            player.control.select_flash = 30;
        }
        // Game display (:1352): the app is the local player's view.
        self.emit_audio_command(crate::AudioCommand::PlaySound {
            name: "Ding".to_string(),
            target: None,
            volume: 100,
            looped: false,
            multiple: false,
            custom_falloff: None,
            target_position: None,
        });
        Ok(())
    }

    /// `C4Player::UpdateSelectionToggleStatus` (C4Player.cpp:1355-1365).
    pub(in crate::direct_com) fn player_update_selection_toggle_status(
        &mut self,
        owner: i32,
    ) -> Result<(), EngineError> {
        let (cursor_selection, cursor_toggled) = self
            .players
            .get(&owner)
            .map(|player| {
                (
                    player.control.cursor_selection,
                    player.control.cursor_toggled,
                )
            })
            .unwrap_or((0, 0));
        if cursor_selection != 0 {
            if cursor_toggled != 0 {
                self.player_adjust_cursor_command(owner)?;
            } else {
                self.player_select_single_by_cursor(owner)?;
            }
        }
        if let Some(player) = self.players.get_mut(&owner) {
            player.control.cursor_selection = 0;
            player.control.cursor_toggled = 0;
        }
        Ok(())
    }
}
