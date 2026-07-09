//! The classic direct-control dispatch chain:
//! `C4Player::InCom` → `C4Player::DirectCom` → `C4Player::ObjectCom` →
//! `C4Object::DirectCom` (C4Player.cpp:1490-1554, 1453-1488, 1368-1390;
//! C4Object.cpp:3327-3557) plus the `ObjectCom*` per-procedure helpers
//! (C4ObjectCom.cpp). Coms are the raw C4Constants.h bytes (COM_Left=1 …)
//! with the COM_Single/COM_Double/release modifiers.

use crate::action::ActionProcedure;
use crate::command::{CommandId, CommandMode, CommandOperation, CommandRequest};
use crate::compat;
use crate::control::{
    COM_CLEAR_PRESSED_COMS, COM_CONTENTS, COM_CURSOR_FIRST, COM_CURSOR_LAST, COM_CURSOR_LEFT,
    COM_CURSOR_RIGHT, COM_CURSOR_TOGGLE, COM_DIG, COM_DOUBLE, COM_DOWN, COM_LEFT, COM_MENU_FIRST,
    COM_MENU_LAST, COM_MENU_NAVIGATION1, COM_MENU_NAVIGATION2, COM_NONE, COM_RELEASE_FIRST,
    COM_RELEASE_LAST, COM_RELEASE_OFFSET, COM_RIGHT, COM_SINGLE, COM_SPECIAL, COM_SPECIAL2,
    COM_THROW, COM_UP, COM_WHEEL_DOWN, COM_WHEEL_UP,
};
use crate::math::itofix;
use crate::{
    ocf, CommandDirection, Direction, Engine, EngineError, FixedVec2, ObjectId, Value, Vector2,
};

/// `C4DoubleClick` (C4Constants.h:156): frames within which a repeated com
/// becomes a COM_Double, and after which a buffered com flushes as
/// COM_Single.
pub const C4_DOUBLE_CLICK: i32 = 10;

/// `ComName(byCom)` (C4ObjectCom.cpp:800-852) for raw com bytes; feeds the
/// `Control{}`/`Contained{}` script callback names.
pub(crate) fn com_name_raw(com: u8) -> &'static str {
    const S: u8 = COM_SINGLE;
    const D: u8 = COM_DOUBLE;
    const R: u8 = COM_RELEASE_OFFSET;
    match com {
        COM_UP => "Up",
        c if c == COM_UP | S => "UpSingle",
        c if c == COM_UP | D => "UpDouble",
        c if c == COM_UP + R => "UpReleased",
        COM_DOWN => "Down",
        c if c == COM_DOWN | S => "DownSingle",
        c if c == COM_DOWN | D => "DownDouble",
        c if c == COM_DOWN + R => "DownReleased",
        COM_LEFT => "Left",
        c if c == COM_LEFT | S => "LeftSingle",
        c if c == COM_LEFT | D => "LeftDouble",
        c if c == COM_LEFT + R => "LeftReleased",
        COM_RIGHT => "Right",
        c if c == COM_RIGHT | S => "RightSingle",
        c if c == COM_RIGHT | D => "RightDouble",
        c if c == COM_RIGHT + R => "RightReleased",
        COM_DIG => "Dig",
        c if c == COM_DIG | S => "DigSingle",
        c if c == COM_DIG | D => "DigDouble",
        c if c == COM_DIG + R => "DigReleased",
        COM_THROW => "Throw",
        c if c == COM_THROW | S => "ThrowSingle",
        c if c == COM_THROW | D => "ThrowDouble",
        c if c == COM_THROW + R => "ThrowReleased",
        COM_SPECIAL => "Special",
        c if c == COM_SPECIAL | S => "SpecialSingle",
        c if c == COM_SPECIAL | D => "SpecialDouble",
        c if c == COM_SPECIAL + R => "SpecialReleased",
        COM_SPECIAL2 => "Special2",
        c if c == COM_SPECIAL2 | S => "Special2Single",
        c if c == COM_SPECIAL2 | D => "Special2Double",
        c if c == COM_SPECIAL2 + R => "Special2Released",
        COM_WHEEL_UP => "WheelUp",
        COM_WHEEL_DOWN => "WheelDown",
        COM_CURSOR_LEFT => "CursorLeft",
        c if c == COM_CURSOR_LEFT | S => "CursorLeftSingle",
        c if c == COM_CURSOR_LEFT | D => "CursorLeftDouble",
        c if c == COM_CURSOR_LEFT + R => "CursorLeftReleased",
        COM_CURSOR_TOGGLE => "CursorToggle",
        c if c == COM_CURSOR_TOGGLE | S => "CursorToggleSingle",
        c if c == COM_CURSOR_TOGGLE | D => "CursorToggleDouble",
        c if c == COM_CURSOR_TOGGLE + R => "CursorToggleReleased",
        COM_CURSOR_RIGHT => "CursorRight",
        c if c == COM_CURSOR_RIGHT | S => "CursorRightSingle",
        c if c == COM_CURSOR_RIGHT | D => "CursorRightDouble",
        c if c == COM_CURSOR_RIGHT + R => "CursorRightReleased",
        _ => "Undefined",
    }
}

/// `Coms2ComDir(iComs)` (C4ObjectCom.cpp:903-920): only the listed
/// direction-bit combinations map; everything else is COMD_Stop.
pub(crate) fn coms_to_com_dir(coms: i32) -> CommandDirection {
    let dir_coms =
        (1 << COM_LEFT) | (1 << COM_RIGHT) | (1 << COM_UP) | (1 << COM_DOWN);
    let up = 1 << COM_UP;
    let down = 1 << COM_DOWN;
    let left = 1 << COM_LEFT;
    let right = 1 << COM_RIGHT;
    match coms & dir_coms {
        c if c == up => CommandDirection::Up,
        c if c == up | right => CommandDirection::UpRight,
        c if c == right => CommandDirection::Right,
        c if c == down | right => CommandDirection::DownRight,
        c if c == down => CommandDirection::Down,
        c if c == down | left => CommandDirection::DownLeft,
        c if c == left => CommandDirection::Left,
        c if c == up | left => CommandDirection::UpLeft,
        _ => CommandDirection::Stop,
    }
}

/// The verbatim `switch (byCom)` labels of C4Object::DirectCom.
const COM_DOWN_D: u8 = COM_DOWN | COM_DOUBLE;
const COM_DIG_S: u8 = COM_DIG | COM_SINGLE;
const COM_DIG_D: u8 = COM_DIG | COM_DOUBLE;
const COM_THROW_D: u8 = COM_THROW | COM_DOUBLE;
const COM_DOWN_R: u8 = COM_DOWN + COM_RELEASE_OFFSET;

impl Engine {
    /// `C4Player::InCom` (C4Player.cpp:1490-1554): pressed-com bookkeeping
    /// plus COM_Single/COM_Double synthesis around the LastCom buffer.
    /// Cursor-object menu conversion (`Cursor->Menu->ConvertCom`, :1503-1508)
    /// is handled by the app-side object menu before events reach the engine.
    pub fn player_in_com(&mut self, owner: i32, com: u8, data: i32) -> Result<(), EngineError> {
        // Coms for unknown players are dropped like C4Game control routing
        // does when Players.Get fails.
        if !self.players.contains_key(&owner) {
            return Ok(());
        }
        if com == COM_CLEAR_PRESSED_COMS {
            let player = self.player_mut(owner)?;
            player.control.pressed_coms = 0;
            player.control.last_com = COM_NONE;
            return Ok(());
        }
        // Menu control: no single/double processing (C4Player.cpp:1510-1513).
        if (COM_MENU_FIRST..=COM_MENU_LAST).contains(&com) {
            return self.player_direct_com(owner, com, data);
        }
        let mut com = com;
        if !(COM_RELEASE_FIRST..=COM_RELEASE_LAST).contains(&com) {
            // ResetCursorView (C4Player.cpp:1518) is a viewport concern.
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
            if last_com != COM_NONE && last_com != com {
                self.player_direct_com(owner, last_com | COM_SINGLE, data)?;
                // AutoStopControl uses a single COM_Down instead of COM_Down_D
                // for drop (C4Player.cpp:1527-1530).
                if control_style && last_com == COM_DOWN {
                    self.player_mut(owner)?.control.last_com_down_double = C4_DOUBLE_CLICK;
                }
            }
            // Check LastCom buffer for COM_Double (C4Player.cpp:1532-1533).
            let player = self.player_mut(owner)?;
            if player.control.last_com == com {
                com |= COM_DOUBLE;
            }
            // Set before the DirectCom so scripts may clear it (:1534-1536).
            player.control.last_com = com;
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
        let mut pending_singles: Vec<(i32, u8)> = Vec::new();
        let mut owners: Vec<i32> = self.players.keys().copied().collect();
        owners.sort_unstable();
        for owner in owners {
            let Some(player) = self.players.get_mut(&owner) else {
                continue;
            };
            // CursorFlash/SelectFlash decrement (C4Player.cpp:242-243).
            if player.control.cursor_flash > 0 {
                player.control.cursor_flash -= 1;
            }
            if player.control.select_flash > 0 {
                player.control.select_flash -= 1;
            }
            // LastCom timeout (C4Player.cpp:1215-1229).
            if player.control.last_com != COM_NONE {
                player.control.last_com_delay += 1;
                if player.control.last_com_delay > C4_DOUBLE_CLICK {
                    let last_com = player.control.last_com;
                    player.control.last_com = COM_NONE;
                    player.control.last_com_delay = 0;
                    if last_com & COM_SINGLE == 0 {
                        pending_singles.push((owner, last_com | COM_SINGLE));
                    }
                }
            }
            // LastComDownDouble (C4Player.cpp:1231-1232).
            if player.control.last_com_down_double > 0 {
                player.control.last_com_down_double -= 1;
            }
        }
        for (owner, com) in pending_singles {
            self.player_direct_com(owner, com, 0)?;
        }
        Ok(())
    }

    /// `C4Player::DirectCom` (C4Player.cpp:1453-1488). The cursor coms'
    /// crew-cycling half (CursorLeft/CursorRight/CursorToggle/SelectAllCrew,
    /// :1481-1484) still lives in the frontend's InputDispatcher; the
    /// script-override half (`Cursor->CallControl`, :1457-1474) runs here.
    pub fn player_direct_com(&mut self, owner: i32, com: u8, data: i32) -> Result<(), EngineError> {
        let plain_cursor = matches!(
            com & !COM_DOUBLE,
            COM_CURSOR_LEFT | COM_CURSOR_RIGHT | COM_CURSOR_TOGGLE
        );
        if plain_cursor {
            if let Some(cursor) = self.crew_cursor(owner) {
                if let Some(index) = self.find_object_index(cursor) {
                    self.objects[index].state.controller = owner;
                    if self.object_call_control(index, owner, com, None)? {
                        return Ok(());
                    }
                }
            }
            // Crew cycling (C4Player.cpp:1481-1484) is frontend-handled.
            return Ok(());
        }
        // Everything else routes to the cursor object (C4Player.cpp:1486);
        // menu-com leftovers get swallowed in object_direct_com like
        // C4Object.cpp:3356-3357 (object menus live in the app layer).
        self.player_object_com(owner, com, data)
    }

    /// `C4Player::ObjectCom` (C4Player.cpp:1368-1390): route the com to the
    /// cursor object with an updated controller.
    fn player_object_com(&mut self, owner: i32, com: u8, data: i32) -> Result<(), EngineError> {
        self.ensure_cursor(owner)?;
        let Some(cursor) = self.crew_cursor(owner) else {
            return Ok(());
        };
        let Some(index) = self.find_object_index(cursor) else {
            return Ok(());
        };
        // UpdateSelectionToggleStatus (:1378-1379) belongs to the cursor
        // selection model, which the frontend still approximates.
        self.objects[index].state.controller = owner;
        self.object_direct_com(index, com, data)
    }

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

        // Menu control (:3350-3354) happens in the app-side object menu
        // before coms are dispatched to the engine. Menu com leftovers from
        // a closed menu are still swallowed (:3356-3357).
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

    /// The `switch (GetProcedure())` tail of C4Object::DirectCom
    /// (C4Object.cpp:3406-3556).
    fn object_procedure_com(&mut self, index: usize, com: u8) -> Result<(), EngineError> {
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
                            Direction::Left => CommandDirection::DownLeft,
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
                    let com_dir = self.objects[index].state.command_direction.to_script_value();
                    if (2..=7).contains(&com_dir) {
                        if let Some(next) = CommandDirection::from_script_value(com_dir + 1) {
                            self.objects[index].state.command_direction = next;
                        }
                    }
                }
                COM_RIGHT => {
                    // COMD_Right(3)..COMD_UpLeft(8) rotates one step
                    // counter-clockwise (:3469).
                    let com_dir = self.objects[index].state.command_direction.to_script_value();
                    if (3..=8).contains(&com_dir) {
                        if let Some(next) = CommandDirection::from_script_value(com_dir - 1) {
                            self.objects[index].state.command_direction = next;
                        }
                    }
                }
                COM_DOWN => {
                    self.object_com_stop(index);
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
                    self.object_com_stop(index);
                }
            }
            ActionProcedure::Fight => match com {
                COM_LEFT => self.object_com_movement(index, CommandDirection::Left)?,
                COM_RIGHT => self.object_com_movement(index, CommandDirection::Right)?,
                COM_DOWN => {
                    self.object_com_stop(index);
                }
                _ => {}
            },
            ActionProcedure::Push => self.object_push_com(index, com)?,
            _ => {}
        }
        Ok(())
    }

    /// DFA_PUSH branch of DirectCom (C4Object.cpp:3506-3555).
    fn object_push_com(&mut self, index: usize, com: u8) -> Result<(), EngineError> {
        let owner = self.objects[index].state.owner;
        let controller = self.objects[index].state.controller;
        let target = self.objects[index].state.action.target;
        let target_index = target.and_then(|id| self.find_object_index(id));
        // New grab-control model: objects version >= 4,9,5,0 may overload
        // control of grabbing clonks (:3508-3518). All engine-registered
        // defs are treated as modern until DefCore Version is parsed.
        let grab_control_overload = if let Some(target_index) = target_index {
            self.objects[target_index].state.controller = controller;
            true
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
                    .map(|target_index| {
                        self.object_has_function(target_index, "ControlThrow")
                    })
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
        // Action target call control late for old objects (:3550-3553): dead
        // until pre-4,9,5,0 def versions are modelled.
        Ok(())
    }

    /// `C4Object::AutoStopDirectCom` (C4Object.cpp:3559-3727) — the
    /// Jump'n'Run per-procedure fallbacks.
    fn auto_stop_direct_com(&mut self, index: usize, com: u8, _data: i32) -> Result<(), EngineError> {
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
                            player.control.last_com = COM_NONE;
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
                    self.object_com_stop(index);
                }
            }
            ActionProcedure::Fight => match com {
                COM_DOWN => {
                    self.object_com_stop(index);
                }
                _ => self.auto_stop_update_com_dir(index)?,
            },
            ActionProcedure::Push => self.auto_stop_push_com(index, com)?,
            _ => {}
        }
        Ok(())
    }

    /// DFA_PUSH branch of AutoStopDirectCom (C4Object.cpp:3668-3725).
    fn auto_stop_push_com(&mut self, index: usize, com: u8) -> Result<(), EngineError> {
        let owner = self.objects[index].state.owner;
        let controller = self.objects[index].state.controller;
        let target = self.objects[index].state.action.target;
        let target_index = target.and_then(|id| self.find_object_index(id));
        let grab_control_overload = target_index.is_some();
        if let Some(target_index) = target_index {
            let clonk_id = self.objects[index].id;
            if self.object_call_control(target_index, controller, com, Some(clonk_id))? {
                return Ok(());
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
                // The DrawCommandQuery gates (:3701-3704) skip the ungrab when
                // the target shows its own Down control in the command bar —
                // command-bar metadata is not modelled, so ungrab runs.
                if target_index.is_some() {
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
        Ok(())
    }

    /// `C4Object::AutoStopUpdateComDir` (C4Object.cpp:3729-3741).
    fn auto_stop_update_com_dir(&mut self, index: usize) -> Result<(), EngineError> {
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
            self.object_com_stop(index);
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
        let sf = self.object_has_function(container_index, &function);
        // All engine-registered defs count as >= 4,9,1,3 (fCallSfEarly,
        // :3233-3236) until DefCore Version is parsed.
        let call_sf_early = true;
        let mut result = false;
        if call_sf_early {
            if sf {
                let clonk_ref = compat::object_reference_value(self.objects[index].id);
                let value =
                    self.contained_call(container_index, &function, &[clonk_ref])?;
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
                    self.player_object_command(owner, CommandId::Throw, None, 0, 0)?;
                }
            }
            COM_THROW => {
                // `PlayerObjectCommand(...) && ExecuteCommand()` (:3267):
                // the queued command executes on the next command tick.
                self.player_object_command(owner, CommandId::Throw, None, 0, 0)?;
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
        // Take/Take2 (:3293-3302).
        if !sf || call_sf_early {
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
    /// target. The menu itself is app-side; the engine emits the request.
    fn contained_base_menu(&mut self, index: usize, buy: bool) -> Result<(), EngineError> {
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
        let base_id = self.objects[container_index].id;
        let kind = if buy {
            crate::MenuRequestKind::Buy { base: base_id }
        } else {
            crate::MenuRequestKind::Sell { base: base_id }
        };
        self.pending_menu_requests.push(crate::MenuRequest {
            crew_id: self.objects[index].id,
            owner,
            kind,
        });
        Ok(())
    }

    /// The `ContainedControlUpdate` notification for Jump'n'Run control
    /// (C4Object.cpp:3244-3249).
    fn contained_control_update(
        &mut self,
        index: usize,
        com: u8,
        controller: i32,
    ) -> Result<(), EngineError> {
        if com & (COM_SINGLE | COM_DOUBLE) != 0 {
            return Ok(());
        }
        let Some(player) = self.players.get(&controller) else {
            return Ok(());
        };
        if !player.control.control_style {
            return Ok(());
        }
        let pressed = player.control.pressed_coms;
        let Some(container_index) = self
            .objects
            .get(index)
            .and_then(|object| object.state.container)
            .and_then(|id| self.find_object_index(id))
        else {
            return Ok(());
        };
        let clonk_ref = compat::object_reference_value(self.objects[index].id);
        let args = [
            clonk_ref,
            Value::Int(coms_to_com_dir(pressed).to_script_value()),
            Value::Bool(pressed & (1 << COM_DIG) != 0),
            Value::Bool(pressed & (1 << COM_THROW) != 0),
        ];
        self.contained_call(container_index, "ContainedControlUpdate", &args)?;
        Ok(())
    }

    /// `C4Object::CallControl` (C4Object.cpp:3307-3325): the `Control{Com}`
    /// script override, C4Value-truthy, plus the Jump'n'Run ControlUpdate
    /// notification.
    fn object_call_control(
        &mut self,
        index: usize,
        controller: i32,
        com: u8,
        clonk_arg: Option<ObjectId>,
    ) -> Result<bool, EngineError> {
        let function = format!("Control{}", com_name_raw(com));
        let args: Vec<Value> = clonk_arg
            .map(|id| vec![compat::object_reference_value(id)])
            .into_iter()
            .flatten()
            .collect();
        let value = self.contained_call(index, &function, &args)?;
        let result = compat::value_raw_truthy(&value);
        // ControlUpdate for Jump'n'Run control (:3313-3323).
        let (control_style, pressed) = self
            .players
            .get(&controller)
            .map(|player| (player.control.control_style, player.control.pressed_coms))
            .unwrap_or((false, 0));
        if control_style {
            let first = clonk_arg
                .map(compat::object_reference_value)
                .unwrap_or_else(|| compat::object_reference_value(self.objects[index].id));
            let args = [
                first,
                Value::Int(coms_to_com_dir(pressed).to_script_value()),
                Value::Bool(pressed & (1 << COM_DIG) != 0),
                Value::Bool(pressed & (1 << COM_THROW) != 0),
                Value::Bool(pressed & (1 << COM_SPECIAL) != 0),
                Value::Bool(pressed & (1 << COM_SPECIAL2) != 0),
            ];
            self.contained_call(index, "ControlUpdate", &args)?;
        }
        Ok(result)
    }

    /// Fail-safe object script call used by the control chain: script
    /// errors log and the tick continues (C4AulExec fail-safe execution,
    /// C4AulExec.cpp:1318-1342). Missing functions return Nil like `Call`
    /// with the `~` prefix.
    fn contained_call(
        &mut self,
        index: usize,
        function: &str,
        args: &[Value],
    ) -> Result<Value, EngineError> {
        let definition_id = self.objects[index].definition_id.clone();
        let Some(library) = self
            .definitions
            .get(&definition_id)
            .map(|definition| definition.action_library().clone())
        else {
            return Ok(Value::Nil);
        };
        let object_id = self.objects[index].id;
        match self.call_movement_object_function(
            index,
            function,
            args,
            &library,
            object_id,
            &definition_id,
        ) {
            Ok(value) => Ok(value),
            Err(error) => {
                // Log the full cause chain — the outer wrap alone only names
                // the callback, not what failed inside it.
                let mut chain = error.to_string();
                let mut source = std::error::Error::source(&error);
                while let Some(cause) = source {
                    chain.push_str(": ");
                    chain.push_str(&cause.to_string());
                    source = std::error::Error::source(cause);
                }
                tracing::warn!(
                    definition = %definition_id,
                    function,
                    error = %chain,
                    "script error in control callback; continuing like the C++ fail-safe exec"
                );
                Ok(Value::Nil)
            }
        }
    }

    fn object_has_function(&self, index: usize, function: &str) -> bool {
        self.definitions
            .get(&self.objects[index].definition_id)
            .map(|definition| definition.script.has_function(function))
            .unwrap_or(false)
    }

    fn object_procedure(&self, index: usize) -> ActionProcedure {
        let Some(definition) = self.definitions.get(&self.objects[index].definition_id) else {
            return ActionProcedure::Undefined;
        };
        let library = definition.action_library();
        let action_name = &self.objects[index].state.action.name;
        if library.is_idle_action(action_name) {
            return ActionProcedure::Undefined;
        }
        library.procedure_for_action(action_name)
    }

    // ---- Contents shifting (C4Object.cpp:5751-5797) -----------------------

    /// `C4Object::ShiftContents` (C4Object.cpp:5751-5775): walk First->Next
    /// (or Last->Prev with `shift_back`) for the first ACTIVE item the
    /// current front cannot concat-picture with — approximated as a
    /// different definition, matching the FnShiftContents host semantics —
    /// and select it via DirectComContents.
    fn object_shift_contents(
        &mut self,
        index: usize,
        shift_back: bool,
        do_calls: bool,
    ) -> Result<bool, EngineError> {
        let contents = self.objects[index].state.contents.clone();
        let Some(front_id) = contents.first().copied() else {
            return Ok(false);
        };
        let Some(front_definition) = self
            .find_object_index(front_id)
            .map(|front_index| self.objects[front_index].definition_id.clone())
        else {
            return Ok(false);
        };
        let mut candidates: Vec<ObjectId> = contents[1..].to_vec();
        if shift_back {
            candidates.reverse();
        }
        for candidate_id in candidates {
            let Some(candidate_index) = self.find_object_index(candidate_id) else {
                continue;
            };
            if !self.objects[candidate_index].state.status.is_active() {
                continue;
            }
            if self.objects[candidate_index].definition_id != front_definition {
                // Object different: shift to this (C4Object.cpp:5768).
                self.object_direct_com_contents(index, candidate_id, do_calls)?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// `C4Object::DirectComContents` (C4Object.cpp:5777-5797): the
    /// ~ControlContents veto, the cyclic rotation to the front, and the
    /// ~Selection callback whose falsy return plays the Grab sound. The
    /// context-menu refill (:5792-5795) is app-side presentation.
    fn object_direct_com_contents(
        &mut self,
        index: usize,
        target_id: ObjectId,
        do_calls: bool,
    ) -> Result<(), EngineError> {
        // Safety: active and contained in this object (:5780).
        let Some(target_index) = self.find_object_index(target_id) else {
            return Ok(());
        };
        if !self.objects[target_index].state.status.is_active()
            || self.objects[target_index].state.container != Some(self.objects[index].id)
        {
            return Ok(());
        }
        // Desired object already at front? (:5782)
        if self.objects[index].state.contents.first() == Some(&target_id) {
            return Ok(());
        }
        // Select object via script? (:5784-5786)
        let target_definition = self.objects[target_index].definition_id.clone();
        if do_calls {
            let veto = self.contained_call(
                index,
                "ControlContents",
                &[Value::C4Id(target_definition.as_str().to_string())],
            )?;
            if compat::value_raw_truthy(&veto) {
                return Ok(());
            }
        }
        // Default action: the cyclic relink (C4ObjectList::ShiftContents,
        // C4ObjectList.cpp:815-833) — a no-op if the id left the list.
        let contents = &mut self.objects[index].state.contents;
        let Some(position) = contents.iter().position(|id| *id == target_id) else {
            return Ok(());
        };
        contents.rotate_left(position);
        // Selection sound (:5790): falsy ~Selection(container) on the new
        // front plays "Grab" at the container.
        if do_calls {
            let container_ref = compat::object_reference_value(self.objects[index].id);
            let selected = self.contained_call(target_index, "Selection", &[container_ref])?;
            if !compat::value_raw_truthy(&selected) {
                let container_id = self.objects[index].id;
                self.pending_audio.push(crate::AudioCommand::PlaySound {
                    name: "Grab".to_string(),
                    target: Some(container_id),
                    volume: 100,
                    looped: false,
                    custom_falloff: None,
                });
            }
        }
        Ok(())
    }

    // ---- ObjectCom* helpers (C4ObjectCom.cpp) -----------------------------

    /// `ObjectComMovement` (C4ObjectCom.cpp:220-237).
    fn object_com_movement(
        &mut self,
        index: usize,
        com_dir: CommandDirection,
    ) -> Result<(), EngineError> {
        self.objects[index].state.command_direction = com_dir;
        let owner = self.objects[index].state.owner;
        let self_id = self.objects[index].id;
        // Selected crew follows the moving cursor (:224).
        self.player_object_command(owner, CommandId::Follow, Some(self_id), 0, 0)?;
        // Direct turnaround if standing still (:226-235).
        let procedure = self.object_procedure(index);
        if self.objects[index].fixed_velocity.x.val() == 0
            && matches!(procedure, ActionProcedure::Walk | ActionProcedure::Hang)
        {
            match com_dir {
                CommandDirection::Left | CommandDirection::UpLeft | CommandDirection::DownLeft => {
                    self.objects[index].state.direction = Direction::Left;
                }
                CommandDirection::Right
                | CommandDirection::UpRight
                | CommandDirection::DownRight => {
                    self.objects[index].state.direction = Direction::Right;
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// `ObjectComStop` (C4ObjectCom.cpp:239-245): cease action, then stand.
    fn object_com_stop(&mut self, index: usize) -> bool {
        let definition_id = self.objects[index].definition_id.clone();
        self.object_action_stand(index, &definition_id)
    }

    /// `ObjectComUp` (C4ObjectCom.cpp:335-351): entrance first, then jump.
    fn object_com_up(&mut self, index: usize) -> Result<bool, EngineError> {
        let position = self.objects[index].state.position;
        let self_id = self.objects[index].id;
        let owner = self.objects[index].state.owner;
        if let Some((_, target_id, target_ocf)) =
            self.at_object(position, ocf::ENTRANCE, Some(self_id))
        {
            if target_ocf & ocf::ENTRANCE != 0 {
                return self.player_object_command(owner, CommandId::Enter, Some(target_id), 0, 0);
            }
        }
        if self.object_procedure(index) == ActionProcedure::Walk {
            return self.player_object_command(owner, CommandId::Jump, None, 0, 0);
        }
        Ok(false)
    }

    /// `ObjectComDig` (C4ObjectCom.cpp:353-362): CanDig gate + Dig action.
    /// The IDS_OBJ_NODIG message is display-only and not yet ported.
    fn object_com_dig(&mut self, index: usize) -> Result<bool, EngineError> {
        let physical = self.object_physical(index);
        let definition_id = self.objects[index].definition_id.clone();
        if physical.can_dig == 0 || !self.force_action_with_calls(index, &definition_id, "Dig") {
            return Ok(false);
        }
        // ObjectActionDig resets the Dig2Object request (:143).
        self.objects[index].state.action.data = 0;
        Ok(true)
    }

    /// `ObjectComDigDouble` (C4ObjectCom.cpp:531-571) — "activation":
    /// contents Activate, chop, then the own Activate call. Linekit line
    /// construction (:542-547, 560-567) is unported (see PORT_STATUS).
    fn object_com_dig_double(&mut self, index: usize) -> Result<(), EngineError> {
        let owner = self.objects[index].state.owner;
        let self_id = self.objects[index].id;
        // Contents activation — first contents object only (:537-539).
        if let Some(contents_id) = self.objects[index].state.contents.first().copied() {
            if let Some(contents_index) = self.find_object_index(contents_id) {
                let clonk_ref = compat::object_reference_value(self_id);
                let value = self.contained_call(contents_index, "Activate", &[clonk_ref])?;
                if compat::value_raw_truthy(&value) {
                    return Ok(());
                }
            }
        }
        // Chop (:549-558).
        let physical = self.object_physical(index);
        if physical.can_chop != 0 && self.object_procedure(index) != ActionProcedure::Swim {
            let position = self.objects[index].state.position;
            if let Some((_, target_id, target_ocf)) =
                self.at_object(position, ocf::CHOP, Some(self_id))
            {
                if target_ocf & ocf::CHOP != 0 {
                    self.player_object_command(owner, CommandId::Chop, Some(target_id), 0, 0)?;
                    return Ok(());
                }
            }
        }
        // Own activation call (:569-570).
        let self_ref = compat::object_reference_value(self_id);
        self.contained_call(index, "Activate", &[self_ref])?;
        Ok(())
    }

    /// `ObjectComDownDouble` (C4ObjectCom.cpp:573-589): build or grab what
    /// is at the object's position.
    fn object_com_down_double(&mut self, index: usize) -> Result<bool, EngineError> {
        let position = self.objects[index].state.position;
        let self_id = self.objects[index].id;
        let owner = self.objects[index].state.owner;
        if let Some((_, target_id, target_ocf)) =
            self.at_object(position, ocf::CONSTRUCT | ocf::GRAB, Some(self_id))
        {
            if target_ocf & ocf::CONSTRUCT != 0 {
                self.player_object_command(owner, CommandId::Build, Some(target_id), 0, 0)?;
                return Ok(true);
            }
            if target_ocf & ocf::GRAB != 0 {
                self.player_object_command(owner, CommandId::Grab, Some(target_id), 0, 0)?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// `ObjectComLetGo` (C4ObjectCom.cpp:310-314): jump off a wall/ceiling.
    fn object_com_let_go(&mut self, index: usize, xdirf: i32) -> Result<bool, EngineError> {
        self.object_action_jump_com(index, itofix(xdirf), crate::C4Fixed::from_raw(0), true)
    }

    /// `ObjectActionJump` (C4ObjectCom.cpp:48-61): the scripted OnActionJump
    /// override, then the hardcoded Jump action with launch velocity.
    fn object_action_jump_com(
        &mut self,
        index: usize,
        xdir: crate::C4Fixed,
        ydir: crate::C4Fixed,
        by_com: bool,
    ) -> Result<bool, EngineError> {
        let args = [
            Value::Int(crate::math::fixtoi_prec(xdir, 100)),
            Value::Int(crate::math::fixtoi_prec(ydir, 100)),
            Value::Bool(by_com),
        ];
        let value = self.contained_call(index, "OnActionJump", &args)?;
        if compat::value_raw_truthy(&value) {
            return Ok(true);
        }
        let definition_id = self.objects[index].definition_id.clone();
        if !self.force_action_with_calls(index, &definition_id, "Jump") {
            return Ok(false);
        }
        let object = &mut self.objects[index];
        object.fixed_velocity = FixedVec2::new(xdir, ydir);
        object.state.velocity = Vector2::new(
            crate::math::fixtoi(xdir),
            crate::math::fixtoi(ydir),
        );
        object.state.mobile = true;
        // Unstick from ground: attach-values were already determined for
        // this frame (:58-59).
        object.frame_t_attach &= !crate::CNAT_BOTTOM;
        Ok(true)
    }

    /// `ObjectComEnter` for the pushed target (C4ObjectCom.cpp:316-333):
    /// the vehicle enters the entrance at its own position via a plain
    /// SetCommand (no control overload).
    fn object_com_enter(&mut self, target_index: Option<usize>) -> Result<bool, EngineError> {
        let Some(target_index) = target_index else {
            return Ok(false);
        };
        // Def NoPushEnter (:321) is not parsed yet; standard vehicles allow
        // push-enter.
        let position = self.objects[target_index].state.position;
        let target_id = self.objects[target_index].id;
        if let Some((_, entrance_id, entrance_ocf)) =
            self.at_object(position, ocf::ENTRANCE, Some(target_id))
        {
            if entrance_ocf & ocf::ENTRANCE != 0 {
                // SetCommand: NoCollectDelay decrement, clear stack, push
                // (C4Object.h:214-219, C4Object.cpp:3939-3943 without the
                // fControl overloads).
                self.objects[target_index].apply_command_operations([
                    CommandOperation::DecrementNoCollectDelay,
                    CommandOperation::Clear,
                    CommandOperation::PushFront(
                        CommandRequest::new(CommandId::Enter).with_target(Some(entrance_id)),
                    ),
                ]);
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// `ObjectComUnGrab` (C4ObjectCom.cpp:261-278): stand up and release the
    /// grab with the Grab/Grabbed script notifications.
    fn object_com_ungrab(&mut self, index: usize) -> Result<bool, EngineError> {
        if self.object_procedure(index) != ActionProcedure::Push {
            return Ok(false);
        }
        let target = self.objects[index].state.action.target;
        let definition_id = self.objects[index].definition_id.clone();
        if !self.object_action_stand(index, &definition_id) {
            return Ok(false);
        }
        // CloseMenu (:269) is app-side.
        let target_ref = target
            .map(compat::object_reference_value)
            .unwrap_or(Value::Nil);
        self.contained_call(index, "Grab", &[target_ref, Value::Bool(false)])?;
        if let Some(target_index) = target.and_then(|id| self.find_object_index(id)) {
            let self_ref = compat::object_reference_value(self.objects[index].id);
            if self.objects[target_index].state.status.is_active() {
                self.contained_call(target_index, "Grabbed", &[self_ref, Value::Bool(false)])?;
            }
        }
        Ok(true)
    }

    // ---- Player command routing -------------------------------------------

    /// `PlayerObjectCommand` (C4ObjectCom.cpp:1013-1040) +
    /// `C4Player::ObjectCommand` (C4Player.cpp:1397-1443): route a control
    /// command to the selected crew (and always the cursor), with the
    /// classic down-double throw→drop conversion.
    pub(crate) fn player_object_command(
        &mut self,
        owner: i32,
        mut command: CommandId,
        target: Option<ObjectId>,
        tx: i32,
        ty: i32,
    ) -> Result<bool, EngineError> {
        let Some(player) = self.players.get_mut(&owner) else {
            return Ok(false);
        };
        // Adjust for old-style keyboard throw/drop control (:1018-1019).
        let ranged = matches!(command, CommandId::Throw | CommandId::Drop);
        if command == CommandId::Throw {
            let mut convert_to_drop = false;
            // Drop on down-down-throw (classic, :1024-1033).
            if player.control.last_com_down_double > 0 {
                convert_to_drop = true;
                player.control.last_com = COM_DOWN | COM_DOUBLE;
                player.control.last_com_down_double = C4_DOUBLE_CLICK;
            }
            // Jump'n'Run: drop on combined Down+Throw (:1034-1035).
            if player.control.control_style
                && player.control.pressed_coms & (1 << COM_DOWN) != 0
            {
                convert_to_drop = true;
            }
            if convert_to_drop {
                command = CommandId::Drop;
            }
        }
        self.player_crew_object_command(owner, command, target, tx, ty, ranged)
    }

    /// `C4Player::ObjectCommand` (C4Player.cpp:1397-1443): apply to all
    /// selected crew in cursor range except the target, then always to the
    /// cursor. `ranged` mirrors C4P_Command_Add|C4P_Command_Range.
    fn player_crew_object_command(
        &mut self,
        owner: i32,
        command: CommandId,
        target: Option<ObjectId>,
        tx: i32,
        ty: i32,
        ranged: bool,
    ) -> Result<bool, EngineError> {
        let cursor = self.crew_cursor(owner);
        let cursor_position = cursor
            .and_then(|id| self.find_object_index(id))
            .map(|index| self.objects[index].state.position);
        let selected: Vec<ObjectId> = self
            .crew_selection
            .get(&owner)
            .map(|selection| selection.selected().to_vec())
            .unwrap_or_default();
        let mut cursor_processed = false;
        for crew_id in selected {
            if Some(crew_id) == cursor {
                cursor_processed = true;
            }
            if Some(crew_id) == target {
                continue;
            }
            let Some(index) = self.find_object_index(crew_id) else {
                continue;
            };
            if !self.objects[index].state.status.is_active() {
                continue;
            }
            if ranged {
                // C4P_Command_Range: within ±15 of the cursor (:1412).
                let Some(cursor_position) = cursor_position else {
                    continue;
                };
                let position = self.objects[index].state.position;
                if (position.x - cursor_position.x).abs() > 15
                    || (position.y - cursor_position.y).abs() > 15
                {
                    continue;
                }
            }
            self.object_command_to_obj(index, command, target, tx, ty, ranged)?;
        }
        // Always apply to cursor, even if it's not selected (:1436-1439).
        if let Some(cursor_id) = cursor {
            if !cursor_processed && Some(cursor_id) != target {
                if let Some(index) = self.find_object_index(cursor_id) {
                    if self.objects[index].state.status.is_active() {
                        self.object_command_to_obj(index, command, target, tx, ty, ranged)?;
                    }
                }
            }
        }
        Ok(true)
    }

    /// `C4Player::ObjectCommand2Obj` (C4Player.cpp:1445-1451): Add-mode
    /// commands push in front of the stack, Set-mode commands replace it.
    /// The Set path is `C4Object::SetCommand` with fControl
    /// (C4Object.cpp:3923-3981): clear, then the soft menu close, then the
    /// `ControlCommand` script overload before the hardcoded push.
    fn object_command_to_obj(
        &mut self,
        index: usize,
        command: CommandId,
        target: Option<ObjectId>,
        tx: i32,
        ty: i32,
        add_mode: bool,
    ) -> Result<(), EngineError> {
        let request = CommandRequest::new(command)
            .with_target(target)
            .with_tx((tx != 0).then_some(tx))
            .with_ty((ty != 0).then_some(ty))
            .with_mode(CommandMode::Base);
        if add_mode {
            // C4P_Command_Add → AddCommand(..., fAppend=false): push front
            // without clearing (C4Command.cpp AddCommand semantics).
            self.objects[index].apply_command_operations([CommandOperation::PushFront(request)]);
            return Ok(());
        }
        // SetCommand: decrement NoCollectDelay (:3941-3942), then clear the
        // stack (:3943).
        self.objects[index].apply_command_operations([
            CommandOperation::DecrementNoCollectDelay,
            CommandOperation::Clear,
        ]);
        // Close menu — soft: `if (!CloseMenu(false)) return;`
        // (C4Object.cpp:3944-3946). A MenuQueryCancel denial aborts the
        // SetCommand with the stack already cleared. The query may run
        // script, so re-resolve the index afterwards.
        let object_id = self.objects[index].id;
        if !self.close_object_menu(object_id, false)? {
            return Ok(());
        }
        let Some(index) = self.find_object_index(object_id) else {
            return Ok(());
        };
        // Script overload (:3935-3942): `ControlCommand(name, target, tx,
        // ty, target2, data)`.
        let args = [
            Value::String(command.to_name().to_string()),
            target
                .map(compat::object_reference_value)
                .unwrap_or(Value::Nil),
            Value::Int(tx),
            Value::Int(ty),
            Value::Nil,
            Value::Int(0),
        ];
        let overloaded = self
            .contained_call(index, "ControlCommand", &args)
            .map(|value| compat::value_raw_truthy(&value))
            .unwrap_or(false);
        if overloaded {
            return Ok(());
        }
        // Inside vehicle control (:3948-3961) needs the def VehicleControl
        // flags, which are not parsed yet (see PORT_STATUS).
        self.objects[index].apply_command_operations([CommandOperation::PushFront(request)]);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ActionSpec, ActionState, Definition, MovementProfile, PhysicalInfo, PlayerConfig,
        SpawnConfig,
    };
    use std::collections::HashMap;

    fn clonk_actions() -> HashMap<String, ActionSpec> {
        let mut actions = HashMap::new();
        actions.insert(
            "Walk".to_string(),
            ActionSpec::default().with_procedure("walk"),
        );
        actions.insert(
            "Jump".to_string(),
            ActionSpec::default().with_procedure("flight"),
        );
        actions.insert(
            "Dig".to_string(),
            ActionSpec::default().with_procedure("dig"),
        );
        actions
    }

    fn register_clonk(engine: &mut Engine, id: &str, script: &str) {
        let mut definition = Definition::from_script(id, id, script).expect("script compiles");
        definition.configure_actions(Some("Walk".to_string()), clonk_actions());
        definition.set_movement_profile(MovementProfile::default());
        let physical = PhysicalInfo {
            walk: 70_000,
            jump: 40_000,
            dig: 40_000,
            can_dig: 1,
            ..Default::default()
        };
        definition.set_physical(physical);
        engine.register_definition(definition).expect("register");
    }

    fn spawn_crew(engine: &mut Engine, def: &str, owner: i32) -> ObjectId {
        let crew = engine
            .spawn_object(
                SpawnConfig::new(def)
                    .with_owner(owner)
                    .with_crew_member(true)
                    .with_action(ActionState::new("Walk")),
            )
            .expect("spawn crew");
        engine.select_crew(owner, vec![crew]).expect("select");
        engine.set_crew_cursor(owner, Some(crew)).expect("cursor");
        crew
    }

    /// A collector clonk + a collectible item inside it, ready for the
    /// drop→NoCollectDelay→recollect window tests.
    fn drop_window_fixture(engine: &mut Engine) -> (ObjectId, ObjectId) {
        let mut clonk =
            Definition::from_script("CLNK", "Clonk", "#strict\n").expect("clonk compiles");
        clonk.configure_actions(Some("Walk".to_string()), clonk_actions());
        clonk.set_movement_profile(MovementProfile::default());
        clonk.set_collection_rect(Some(crate::DefinitionRect::new(-8, -16, 16, 32)));
        engine.register_definition(clonk).expect("register clonk");
        let mut item = Definition::from_script("GOLD", "Gold", "#strict\n").expect("item compiles");
        item.set_collectible(true);
        engine.register_definition(item).expect("register item");
        engine
            .register_player(PlayerConfig::new(1, "Test"))
            .expect("player");
        let crew = spawn_crew(engine, "CLNK", 1);
        let item = engine
            .spawn_object(SpawnConfig::new("GOLD").with_container(crew))
            .expect("spawn item");
        (crew, item)
    }

    fn no_collect_delay(engine: &Engine, id: ObjectId) -> i32 {
        let index = engine.find_object_index(id).expect("object exists");
        engine.objects[index].state.no_collect_delay
    }

    #[test]
    fn drop_command_arms_no_collect_delay_and_clears_collection_ocf() {
        // ObjectComDrop (C4ObjectCom.cpp:668-671): after the item exits,
        // `cObj->NoCollectDelay = 2` and the immediate SetOCF drop the
        // dropper's OCF_Collection bit (SetOCF, C4Object.cpp:598-600).
        let mut engine = Engine::new();
        let (crew, item) = drop_window_fixture(&mut engine);

        engine
            .player_object_command(1, CommandId::Drop, None, 0, 0)
            .expect("drop command");
        engine.tick().expect("tick");
        let item_index = engine.find_object_index(item).expect("item exists");
        assert_eq!(
            engine.objects[item_index].state.container, None,
            "the drop exited the item"
        );
        assert_eq!(
            no_collect_delay(&engine, crew),
            2,
            "ObjectComDrop arms NoCollectDelay = 2 (C4ObjectCom.cpp:669)"
        );
        let crew_index = engine.find_object_index(crew).expect("crew exists");
        assert_eq!(
            engine.objects[crew_index].state.ocf & ocf::COLLECTION,
            0,
            "the post-drop SetOCF clears OCF_Collection (C4ObjectCom.cpp:671)"
        );
    }

    #[test]
    fn set_command_control_path_decrements_no_collect_delay() {
        // C4Object::SetCommand decrements NoCollectDelay at entry
        // (C4Object.cpp:3941-3942). A single COM_Up press in WALK counts
        // down twice: once in DirectCom (:3359-3362) and once in the Jump
        // command's SetCommand (ObjectComUp -> PlayerObjectCommand ->
        // ObjectCommand2Obj Set mode, C4Player.cpp:1450).
        let mut engine = Engine::new();
        let (crew, _) = drop_window_fixture(&mut engine);
        let index = engine.find_object_index(crew).expect("crew exists");
        engine.objects[index].state.no_collect_delay = 2;

        engine.player_in_com(1, COM_UP, 0).expect("in_com");
        assert_eq!(
            no_collect_delay(&engine, crew),
            0,
            "DirectCom + SetCommand each count the delay down once"
        );
    }

    #[test]
    fn script_set_command_decrements_no_collect_delay() {
        // FnSetCommand routes through C4Object::SetCommand
        // (C4Script.cpp:866), whose entry decrement (C4Object.cpp:3941-3942)
        // must also fire for script-issued commands.
        let script = r#"
#strict
public func DoWait() { SetCommand(this(), "Wait"); return(1); }
"#;
        let mut engine = Engine::new();
        register_clonk(&mut engine, "CLNK", script);
        engine
            .register_player(PlayerConfig::new(1, "Test"))
            .expect("player");
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        let index = engine.find_object_index(crew).expect("crew exists");
        engine.objects[index].state.no_collect_delay = 2;

        engine
            .call_object_function(index, "DoWait", Vec::new())
            .expect("DoWait runs");
        assert_eq!(
            no_collect_delay(&engine, crew),
            1,
            "script SetCommand counts the delay down once (C4Object.cpp:3941)"
        );
    }

    #[test]
    fn drop_window_closes_after_a_control_and_the_item_is_recollected() {
        // The full C++ window: drop arms NoCollectDelay = 2
        // (C4ObjectCom.cpp:669); the next plain control counts it down in
        // DirectCom (C4Object.cpp:3359-3362) AND in the resulting Set-mode
        // command's SetCommand (:3941-3942) — after ONE control the
        // collector's OCF_Collection returns and the Tick3 cross check
        // recollects the item (C4GameObjects.cpp:185-194).
        let mut engine = Engine::new();
        let (crew, item) = drop_window_fixture(&mut engine);

        engine
            .player_object_command(1, CommandId::Drop, None, 0, 0)
            .expect("drop command");
        for _ in 0..6 {
            engine.tick().expect("tick");
        }
        let item_index = engine.find_object_index(item).expect("item exists");
        assert_eq!(
            engine.objects[item_index].state.container, None,
            "armed delay keeps the item on the ground"
        );

        engine.player_in_com(1, COM_UP, 0).expect("in_com");
        for _ in 0..3 {
            engine.tick().expect("tick");
        }
        let item_index = engine.find_object_index(item).expect("item exists");
        assert_eq!(
            engine.objects[item_index].state.container,
            Some(crew),
            "one control closes the window and the cross check recollects"
        );
    }

    #[test]
    fn dropped_item_is_not_recollected_while_the_delay_is_armed() {
        // While NoCollectDelay > 0 the dropper never regains OCF_Collection
        // (SetOCF, C4Object.cpp:598), so the reverse-pass cross check
        // (C4GameObjects.cpp:185-194) leaves the dropped item alone across
        // any number of Tick3 frames.
        let mut engine = Engine::new();
        let (_, item) = drop_window_fixture(&mut engine);

        engine
            .player_object_command(1, CommandId::Drop, None, 0, 0)
            .expect("drop command");
        for _ in 0..9 {
            engine.tick().expect("tick");
        }
        let item_index = engine.find_object_index(item).expect("item exists");
        assert_eq!(
            engine.objects[item_index].state.container, None,
            "no control was issued, so the delay never counted down and the \
             item stays on the ground"
        );
    }

    #[test]
    fn com_name_matches_cpp_comname_table() {
        // ComName (C4ObjectCom.cpp:800-852).
        assert_eq!(com_name_raw(COM_LEFT), "Left");
        assert_eq!(com_name_raw(COM_LEFT | COM_SINGLE), "LeftSingle");
        assert_eq!(com_name_raw(COM_LEFT | COM_DOUBLE), "LeftDouble");
        assert_eq!(com_name_raw(COM_LEFT + COM_RELEASE_OFFSET), "LeftReleased");
        assert_eq!(com_name_raw(COM_DIG | COM_SINGLE), "DigSingle");
        assert_eq!(com_name_raw(COM_THROW | COM_DOUBLE), "ThrowDouble");
        assert_eq!(com_name_raw(COM_CURSOR_TOGGLE), "CursorToggle");
        assert_eq!(com_name_raw(0), "Undefined");
        assert_eq!(com_name_raw(COM_DIG | COM_SINGLE | COM_DOUBLE), "Undefined");
    }

    #[test]
    fn coms_to_com_dir_matches_cpp_table() {
        // Coms2ComDir (C4ObjectCom.cpp:903-920): only the eight listed
        // combinations map, everything else is COMD_Stop.
        assert_eq!(coms_to_com_dir(1 << COM_UP), CommandDirection::Up);
        assert_eq!(
            coms_to_com_dir((1 << COM_UP) | (1 << COM_RIGHT)),
            CommandDirection::UpRight
        );
        assert_eq!(coms_to_com_dir(1 << COM_LEFT), CommandDirection::Left);
        // Left+Right+Up is not a listed combination: stop, not up.
        assert_eq!(
            coms_to_com_dir((1 << COM_LEFT) | (1 << COM_RIGHT) | (1 << COM_UP)),
            CommandDirection::Stop
        );
        // Non-direction bits are masked off.
        assert_eq!(
            coms_to_com_dir((1 << COM_DIG) | (1 << COM_RIGHT)),
            CommandDirection::Right
        );
    }

    #[test]
    fn directional_control_left_script_override_consumes_the_com() {
        // CallControl runs for EVERY com (C4Object.cpp:3385-3389): a truthy
        // ControlLeft keeps the per-procedure fallback from running.
        let script = r#"
#strict
protected func ControlLeft() { return(1); }
"#;
        let mut engine = Engine::new();
        register_clonk(&mut engine, "CLNK", script);
        engine
            .register_player(PlayerConfig::new(1, "Test"))
            .expect("player");
        let crew = spawn_crew(&mut engine, "CLNK", 1);

        engine.player_in_com(1, COM_LEFT, 0).expect("in_com");
        let snapshot = engine.object_snapshot(crew).expect("snapshot");
        assert_eq!(
            snapshot.command_direction,
            CommandDirection::Stop,
            "a handled ControlLeft must not reach ObjectComMovement"
        );
    }

    #[test]
    fn walk_left_falls_back_to_object_com_movement() {
        // DFA_WALK COM_Left → ObjectComMovement(COMD_Left) with the direct
        // turnaround (C4Object.cpp:3411; C4ObjectCom.cpp:220-235).
        let script = r#"
#strict
protected func ControlLeft() { return(0); }
"#;
        let mut engine = Engine::new();
        register_clonk(&mut engine, "CLNK", script);
        engine
            .register_player(PlayerConfig::new(1, "Test"))
            .expect("player");
        let crew = spawn_crew(&mut engine, "CLNK", 1);

        engine.player_in_com(1, COM_LEFT, 0).expect("in_com");
        let snapshot = engine.object_snapshot(crew).expect("snapshot");
        assert_eq!(snapshot.command_direction, CommandDirection::Left);
        assert_eq!(
            snapshot.direction,
            Direction::Left,
            "standing turnaround flips the facing (C4ObjectCom.cpp:226-231)"
        );
    }

    #[test]
    fn walk_up_without_entrance_queues_a_jump_command() {
        // DFA_WALK COM_Up → ObjectComUp → PlayerObjectCommand(C4CMD_Jump)
        // (C4Object.cpp:3414; C4ObjectCom.cpp:335-351).
        let mut engine = Engine::new();
        register_clonk(&mut engine, "CLNK", "#strict\n");
        engine
            .register_player(PlayerConfig::new(1, "Test"))
            .expect("player");
        let crew = spawn_crew(&mut engine, "CLNK", 1);

        engine.player_in_com(1, COM_UP, 0).expect("in_com");
        let snapshot = engine.object_snapshot(crew).expect("snapshot");
        assert_eq!(
            snapshot.command_stack.command_names(),
            vec!["Jump".to_string()],
            "COM_Up in WALK issues the jump command"
        );
    }

    #[test]
    fn dig_key_press_starts_digging_after_the_single_timeout() {
        // Classic dig: press COM_Dig, nothing happens (only ControlDig).
        // After C4DoubleClick frames C4Player::Execute flushes
        // COM_Dig|COM_Single (C4Player.cpp:1215-1229) whose WALK fallback is
        // ObjectComDig + the diagonal ComDir (C4Object.cpp:3416-3421).
        let mut engine = Engine::new();
        register_clonk(&mut engine, "CLNK", "#strict\n");
        engine
            .register_player(PlayerConfig::new(1, "Test"))
            .expect("player");
        let crew = spawn_crew(&mut engine, "CLNK", 1);

        engine.player_in_com(1, COM_DIG, 0).expect("in_com");
        let snapshot = engine.object_snapshot(crew).expect("snapshot");
        assert_eq!(snapshot.action.name, "Walk", "no dig before the timeout");

        for _ in 0..=C4_DOUBLE_CLICK {
            engine.tick().expect("tick");
        }
        let snapshot = engine.object_snapshot(crew).expect("snapshot");
        assert_eq!(snapshot.action.name, "Dig");
        assert_eq!(
            snapshot.command_direction,
            CommandDirection::DownLeft,
            "digging aims down toward the facing - the spawn faces DIR_Left \
             like C++ (C4Object.cpp:3419)"
        );
    }

    #[test]
    fn dig_pressed_twice_activates_contents_via_dig_double() {
        // Two dig presses inside C4DoubleClick become COM_Dig_D
        // (C4Player::InCom, C4Player.cpp:1532-1533) → ObjectComDigDouble
        // activates the first contents object (C4ObjectCom.cpp:537-539).
        let clonk = r#"
#strict
"#;
        let scroll = r#"
#strict
public func Activate(pByClonk) { DoDamage(1); return(1); }
"#;
        let mut engine = Engine::new();
        register_clonk(&mut engine, "CLNK", clonk);
        let scroll_def =
            Definition::from_script("SCRL", "Scroll", scroll).expect("scroll compiles");
        engine.register_definition(scroll_def).expect("register");
        engine
            .register_player(PlayerConfig::new(1, "Test"))
            .expect("player");
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        let item = engine
            .spawn_object(SpawnConfig::new("SCRL").with_container(crew))
            .expect("spawn scroll");

        engine.player_in_com(1, COM_DIG, 0).expect("first press");
        engine.player_in_com(1, COM_DIG, 0).expect("second press");
        let snapshot = engine.object_snapshot(item).expect("scroll snapshot");
        assert_eq!(
            snapshot.damage, 1,
            "the contents object's Activate ran (C4ObjectCom.cpp:537-539)"
        );
    }

    #[test]
    fn throw_com_queues_throw_command_for_the_cursor() {
        // DFA_WALK COM_Throw → PlayerObjectCommand(C4CMD_Throw)
        // (C4Object.cpp:3423, C4ObjectCom.cpp:1013-1040).
        let mut engine = Engine::new();
        register_clonk(&mut engine, "CLNK", "#strict\n");
        engine
            .register_player(PlayerConfig::new(1, "Test"))
            .expect("player");
        let crew = spawn_crew(&mut engine, "CLNK", 1);

        engine.player_in_com(1, COM_THROW, 0).expect("in_com");
        let snapshot = engine.object_snapshot(crew).expect("snapshot");
        assert_eq!(snapshot.command_stack.command_names(), vec!["Throw"]);
    }

    #[test]
    fn down_double_after_throw_converts_to_drop() {
        // LastComDownDouble makes the next throw a drop
        // (PlayerObjectCommand, C4ObjectCom.cpp:1020-1036).
        let mut engine = Engine::new();
        register_clonk(&mut engine, "CLNK", "#strict\n");
        engine
            .register_player(PlayerConfig::new(1, "Test"))
            .expect("player");
        let crew = spawn_crew(&mut engine, "CLNK", 1);

        engine.player_in_com(1, COM_DOWN, 0).expect("down 1");
        engine.player_in_com(1, COM_DOWN, 0).expect("down 2");
        engine.player_in_com(1, COM_THROW, 0).expect("throw");
        let snapshot = engine.object_snapshot(crew).expect("snapshot");
        assert_eq!(
            snapshot.command_stack.command_names(),
            vec!["Drop"],
            "down-down-throw is the classic drop (C4ObjectCom.cpp:1024-1036)"
        );
    }

    #[test]
    fn contained_com_down_issues_exit_command() {
        // ContainedControl hardcoded COM_Down → PlayerObjectCommand(
        // C4CMD_Exit) (C4Object.cpp:3256-3258).
        let hut = r#"
#strict
"#;
        let mut engine = Engine::new();
        register_clonk(&mut engine, "CLNK", "#strict\n");
        let hut_def = Definition::from_script("HUT1", "Hut", hut).expect("hut compiles");
        engine.register_definition(hut_def).expect("register");
        engine
            .register_player(PlayerConfig::new(1, "Test"))
            .expect("player");
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        let hut = engine
            .spawn_object(SpawnConfig::new("HUT1"))
            .expect("spawn hut");
        engine
            .apply_object_update(
                crew,
                crate::ObjectUpdate::new().with_container(hut),
            )
            .expect("enter hut");

        engine.player_in_com(1, COM_DOWN, 0).expect("in_com");
        let snapshot = engine.object_snapshot(crew).expect("snapshot");
        assert_eq!(snapshot.command_stack.command_names(), vec!["Exit"]);
    }

    #[test]
    fn contained_com_left_issues_take_command() {
        // ContainedControl Take/Take2 tail (C4Object.cpp:3293-3302).
        let mut engine = Engine::new();
        register_clonk(&mut engine, "CLNK", "#strict\n");
        let hut_def = Definition::from_script("HUT1", "Hut", "#strict\n").expect("hut compiles");
        engine.register_definition(hut_def).expect("register");
        engine
            .register_player(PlayerConfig::new(1, "Test"))
            .expect("player");
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        let hut = engine
            .spawn_object(SpawnConfig::new("HUT1"))
            .expect("spawn hut");
        engine
            .apply_object_update(crew, crate::ObjectUpdate::new().with_container(hut))
            .expect("enter hut");

        engine.player_in_com(1, COM_LEFT, 0).expect("in_com");
        let snapshot = engine.object_snapshot(crew).expect("snapshot");
        assert_eq!(snapshot.command_stack.command_names(), vec!["Take"]);
    }

    /// Crew contained in a hut that is player `base`'s home base.
    fn contained_base_fixture(engine: &mut Engine, base: i32) -> (ObjectId, ObjectId) {
        register_clonk(engine, "CLNK", "#strict\n");
        let hut_def = Definition::from_script("HUT1", "Hut", "#strict\n").expect("hut compiles");
        engine.register_definition(hut_def).expect("register");
        engine
            .register_player(PlayerConfig::new(1, "Test"))
            .expect("player");
        if base != 1 {
            engine
                .register_player(PlayerConfig::new(base, "Host"))
                .expect("player 2");
        }
        let crew = spawn_crew(engine, "CLNK", 1);
        let hut = engine
            .spawn_object(SpawnConfig::new("HUT1"))
            .expect("spawn hut");
        let hut_index = engine.find_object_index(hut).expect("hut exists");
        engine.objects[hut_index].state.base = base;
        engine
            .apply_object_update(crew, crate::ObjectUpdate::new().with_container(hut))
            .expect("enter hut");
        (crew, hut)
    }

    #[test]
    fn contained_com_up_opens_the_base_buy_menu() {
        // ContainedControl COM_Up (C4Object.cpp:3269-3274): a valid,
        // non-hostile base with BASEFUNC_Buy opens the buy menu on the
        // clonk (ActivateMenu(C4MN_Buy), pTarget = Contained).
        let mut engine = Engine::new();
        let (crew, hut) = contained_base_fixture(&mut engine, 1);

        engine.player_in_com(1, COM_UP, 0).expect("in_com");
        assert!(
            engine.pending_menu_requests.iter().any(|request| {
                request.crew_id == crew
                    && request.owner == 1
                    && matches!(request.kind,
                        crate::MenuRequestKind::Buy { base } if base == hut)
            }),
            "COM_Up in a friendly base activates the buy menu"
        );
    }

    #[test]
    fn contained_com_dig_opens_the_base_sell_menu() {
        // ContainedControl COM_Dig (C4Object.cpp:3275-3280): the sell menu
        // twin, gated on BASEFUNC_Sell.
        let mut engine = Engine::new();
        let (crew, hut) = contained_base_fixture(&mut engine, 1);

        engine.player_in_com(1, COM_DIG, 0).expect("in_com");
        assert!(
            engine.pending_menu_requests.iter().any(|request| {
                request.crew_id == crew
                    && matches!(request.kind,
                        crate::MenuRequestKind::Sell { base } if base == hut)
            }),
            "COM_Dig in a friendly base activates the sell menu"
        );
    }

    #[test]
    fn hostile_or_disabled_bases_never_open_buy_menus() {
        // Hostile(Owner, Contained->Base) vetoes (C4Object.cpp:3271), as
        // does a cleared BASEFUNC_Buy bit (:3272).
        let mut engine = Engine::new();
        let (_, _) = contained_base_fixture(&mut engine, 2);
        engine.set_hostility(1, 2, true).expect("hostility");
        engine.player_in_com(1, COM_UP, 0).expect("in_com");
        assert!(
            engine.pending_menu_requests.is_empty(),
            "hostile bases sell nothing"
        );

        let mut engine = Engine::new();
        let (_, _) = contained_base_fixture(&mut engine, 1);
        engine.set_base_buy_enabled(false);
        engine.player_in_com(1, COM_UP, 0).expect("in_com");
        assert!(
            engine.pending_menu_requests.is_empty(),
            "BASEFUNC_Buy off keeps the menu closed"
        );
    }

    #[test]
    fn contained_script_override_beats_hardcoded_exit() {
        // fCallSfEarly containers run Contained<Com> first; a truthy result
        // consumes the com (C4Object.cpp:3239-3251).
        let hut = r#"
#strict
protected func ContainedDown(pByClonk) { return(1); }
"#;
        let mut engine = Engine::new();
        register_clonk(&mut engine, "CLNK", "#strict\n");
        let hut_def = Definition::from_script("HUT1", "Hut", hut).expect("hut compiles");
        engine.register_definition(hut_def).expect("register");
        engine
            .register_player(PlayerConfig::new(1, "Test"))
            .expect("player");
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        let hut = engine
            .spawn_object(SpawnConfig::new("HUT1"))
            .expect("spawn hut");
        engine
            .apply_object_update(crew, crate::ObjectUpdate::new().with_container(hut))
            .expect("enter hut");

        engine.player_in_com(1, COM_DOWN, 0).expect("in_com");
        let snapshot = engine.object_snapshot(crew).expect("snapshot");
        assert!(
            snapshot.command_stack.command_names().is_empty(),
            "the container consumed the com"
        );
    }

    /// Crew with three contents of distinct defs: front ROCK, then GOLD,
    /// then SKUL (front = `contents[0]`, the C4ObjectList First).
    fn wheel_fixture(engine: &mut Engine, clonk_script: &str) -> (ObjectId, [ObjectId; 3]) {
        register_clonk(engine, "CLNK", clonk_script);
        for id in ["ROCK", "GOLD", "SKUL"] {
            let def = Definition::from_script(id, id, "#strict\n").expect("item compiles");
            engine.register_definition(def).expect("register item");
        }
        engine
            .register_player(PlayerConfig::new(1, "Test"))
            .expect("player");
        let crew = spawn_crew(engine, "CLNK", 1);
        let items = ["ROCK", "GOLD", "SKUL"].map(|id| {
            engine
                .spawn_object(SpawnConfig::new(id).with_container(crew))
                .expect("spawn item")
        });
        let index = engine.find_object_index(crew).expect("crew exists");
        engine.objects[index].state.contents = items.to_vec();
        (crew, items)
    }

    fn contents(engine: &Engine, id: ObjectId) -> Vec<ObjectId> {
        let index = engine.find_object_index(id).expect("object exists");
        engine.objects[index].state.contents.clone()
    }

    #[test]
    fn wheel_down_shifts_contents_to_the_next_different_item() {
        // COM_WheelDown → ShiftContents(false, true) (C4Object.cpp:
        // 3391-3396): walk First->Next for the first item of a DIFFERENT
        // definition and rotate it to the front (C4Object.cpp:5751-5775).
        let mut engine = Engine::new();
        let (crew, [rock, gold, skul]) = wheel_fixture(&mut engine, "#strict\n");

        engine.player_in_com(1, COM_WHEEL_DOWN, 0).expect("wheel");
        assert_eq!(contents(&engine, crew), vec![gold, skul, rock]);
    }

    #[test]
    fn wheel_up_shifts_contents_back_to_the_last_different_item() {
        // COM_WheelUp → ShiftContents(true, true): walk from Contents.Last
        // backwards (C4Object.cpp:5757).
        let mut engine = Engine::new();
        let (crew, [rock, gold, skul]) = wheel_fixture(&mut engine, "#strict\n");

        engine.player_in_com(1, COM_WHEEL_UP, 0).expect("wheel");
        assert_eq!(contents(&engine, crew), vec![skul, rock, gold]);
    }

    #[test]
    fn wheel_shift_respects_the_control_contents_veto() {
        // DirectComContents runs ~ControlContents(id) first; a truthy
        // return takes over the selection (C4Object.cpp:5784-5786).
        let script = r#"
#strict
protected func ControlContents(idTarget) { return(1); }
"#;
        let mut engine = Engine::new();
        let (crew, [rock, gold, skul]) = wheel_fixture(&mut engine, script);

        engine.player_in_com(1, COM_WHEEL_DOWN, 0).expect("wheel");
        assert_eq!(
            contents(&engine, crew),
            vec![rock, gold, skul],
            "the container's ControlContents consumed the shift"
        );
    }

    #[test]
    fn com_contents_shifts_the_target_to_the_front_of_its_container() {
        // COM_Contents carries the target's object NUMBER in iData and the
        // shift runs on the target's CONTAINER (C4Object.cpp:3364-3372 ->
        // DirectComContents, :5777-5797).
        let mut engine = Engine::new();
        let (crew, [rock, gold, skul]) = wheel_fixture(&mut engine, "#strict\n");

        engine
            .player_in_com(1, COM_CONTENTS, skul.as_u64() as i32)
            .expect("contents com");
        assert_eq!(contents(&engine, crew), vec![skul, rock, gold]);
        // The new front had no ~Selection handler: the Grab sound plays at
        // the container (C4Object.cpp:5790).
        assert!(
            engine.pending_audio.iter().any(|command| matches!(
                command,
                crate::AudioCommand::PlaySound { name, target, .. }
                    if name == "Grab" && *target == Some(crew)
            )),
            "falsy Selection plays the Grab sound"
        );
    }

    #[test]
    fn release_without_registered_press_is_dropped() {
        // C4Player::InCom (C4Player.cpp:1541-1548): a release only counts
        // when its press bit is set.
        let script = r#"
#strict
protected func ControlLeftReleased() { SetComDir(COMD_Right()); return(1); }
"#;
        let mut engine = Engine::new();
        register_clonk(&mut engine, "CLNK", script);
        engine
            .register_player(PlayerConfig::new(1, "Test"))
            .expect("player");
        let crew = spawn_crew(&mut engine, "CLNK", 1);

        engine
            .player_in_com(1, COM_LEFT + COM_RELEASE_OFFSET, 0)
            .expect("in_com");
        let snapshot = engine.object_snapshot(crew).expect("snapshot");
        assert_eq!(
            snapshot.command_direction,
            CommandDirection::Stop,
            "unmatched releases never dispatch"
        );

        engine.player_in_com(1, COM_LEFT, 0).expect("press");
        engine
            .player_in_com(1, COM_LEFT + COM_RELEASE_OFFSET, 0)
            .expect("release");
        let snapshot = engine.object_snapshot(crew).expect("snapshot");
        assert_eq!(
            snapshot.command_direction,
            CommandDirection::Right,
            "a registered release dispatches ControlLeftReleased"
        );
    }

    #[test]
    fn classic_release_does_not_stop_the_walk() {
        // In classic control a released direction key changes nothing: the
        // per-procedure switch has no release cases (C4Object.cpp:3406-3556).
        let mut engine = Engine::new();
        register_clonk(&mut engine, "CLNK", "#strict\n");
        engine
            .register_player(PlayerConfig::new(1, "Test"))
            .expect("player");
        let crew = spawn_crew(&mut engine, "CLNK", 1);

        engine.player_in_com(1, COM_RIGHT, 0).expect("press");
        engine
            .player_in_com(1, COM_RIGHT + COM_RELEASE_OFFSET, 0)
            .expect("release");
        let snapshot = engine.object_snapshot(crew).expect("snapshot");
        assert_eq!(
            snapshot.command_direction,
            CommandDirection::Right,
            "classic control keeps walking until COM_Down stops it"
        );
    }
}
