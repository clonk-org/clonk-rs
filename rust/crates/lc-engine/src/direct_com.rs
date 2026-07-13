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
    COM_MENU_LAST, COM_MENU_CLOSE, COM_MENU_DOWN, COM_MENU_ENTER, COM_MENU_ENTER_ALL,
    COM_MENU_LEFT, COM_MENU_NAVIGATION1, COM_MENU_NAVIGATION2, COM_MENU_RIGHT, COM_MENU_SELECT,
    COM_MENU_SHOW_TEXT, COM_MENU_UP, COM_NONE, COM_RELEASE_FIRST, COM_RELEASE_LAST,
    COM_RELEASE_OFFSET, COM_RIGHT, COM_SINGLE, COM_SPECIAL, COM_SPECIAL2, COM_THROW, COM_UP,
    COM_WHEEL_DOWN, COM_WHEEL_UP, C4MN_ADJUST_POSITION,
};
use crate::math::{self, itofix};
use crate::{
    ocf, C4Fixed, CommandDirection, Direction, Engine, EngineError, FixedVec2, Landscape, ObjectId,
    MouseDragSource, Value, Vector2, CATEGORY_MOUSE_SELECT,
};

/// `C4DoubleClick` (C4Constants.h:156): frames within which a repeated com
/// becomes a COM_Double, and after which a buffered com flushes as
/// COM_Single.
pub const C4_DOUBLE_CLICK: i32 = 10;

#[derive(Clone, Copy)]
enum PlayerObjectCommandMode {
    Set,
    Add,
    Append,
}

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

/// `SimFlight` (C4Movement.cpp:623-653): fixed-point frame integration with
/// sign-step pixel traversal and an inclusive density contact interval.
pub(crate) fn sim_flight_to_density(
    position: &mut FixedVec2,
    velocity: &mut FixedVec2,
    density_min: i32,
    density_max: i32,
    mut iterations: i32,
    gravity: crate::C4Fixed,
    width: i32,
    height: i32,
    density_at: &impl Fn(i32, i32) -> i32,
) -> bool {
    let mut x = crate::math::fixtoi(position.x);
    let mut y = crate::math::fixtoi(position.y);
    loop {
        if iterations == 0 {
            return false;
        }
        iterations = iterations.wrapping_sub(1);
        position.x += velocity.x;
        position.y += velocity.y;
        let target_x = crate::math::fixtoi(position.x);
        let target_y = crate::math::fixtoi(position.y);
        if !(0..=width).contains(&target_x) || target_y >= height {
            return false;
        }

        let contact = loop {
            x += (target_x - x).signum();
            y += (target_y - y).signum();
            if (density_min..=density_max).contains(&density_at(x, y)) {
                break true;
            }
            if x == target_x && y == target_y {
                break false;
            }
        };
        velocity.y += gravity;
        if contact {
            *position = FixedVec2::from_ints(x, y);
            return true;
        }
    }
}

/// `Distance` (C4Math.cpp:22-31), used by the mouse throwing-position
/// trajectory probe.
fn mouse_c4_distance(first: Vector2, second: Vector2) -> i32 {
    let dx = i64::from(first.x) - i64::from(second.x);
    let dy = i64::from(first.y) - i64::from(second.y);
    let squared = dx * dx + dy * dy;
    let mut distance = (squared as f64).sqrt() as i64;
    if distance * distance < squared {
        distance += 1;
    }
    if distance * distance > squared {
        distance -= 1;
    }
    distance as i32
}

/// `TrajectoryDistance` (C4Landscape.cpp:2055-2068): follow a fixed-point
/// ballistic path until it leaves the landscape or strikes solid terrain and
/// retain the closest whole-pixel distance to the mouse target.
fn mouse_trajectory_distance(
    landscape: &Landscape,
    start: Vector2,
    mut velocity: FixedVec2,
    target: Vector2,
    gravity: C4Fixed,
) -> i32 {
    let mut closest = mouse_c4_distance(start, target);
    let mut position = FixedVec2::from_ints(start.x, start.y);
    let width = i32::try_from(landscape.width()).unwrap_or(i32::MAX);
    let height = landscape.estimated_height();
    loop {
        let pixel = Vector2::new(math::fixtoi(position.x), math::fixtoi(position.y));
        if !(0..width).contains(&pixel.x)
            || !(0..height).contains(&pixel.y)
            || landscape.is_solid_at(pixel.x, pixel.y)
        {
            return closest;
        }
        closest = closest.min(mouse_c4_distance(pixel, target));
        position.x += velocity.x;
        position.y += velocity.y;
        velocity.y += gravity;
    }
}

/// `FindThrowingPosition` (C4Landscape.cpp:2070-2100), reduced to the success
/// predicate needed by `C4MouseControl::DragMoving`.
fn mouse_has_throwing_position(
    landscape: &Landscape,
    target: Vector2,
    velocity: FixedVec2,
    height: i32,
    gravity: C4Fixed,
) -> bool {
    let width = i32::try_from(landscape.width()).unwrap_or(i32::MAX);
    let Some(mut y) = landscape.semi_above_solid(target.x, target.y) else {
        return false;
    };
    if !(-50..=50).contains(&(y - target.y)) {
        return false;
    }
    let direction = if velocity.x > C4Fixed::ZERO { -1 } else { 1 };
    let mut x = target.x;
    for _ in 0..=60 {
        if !(0..width).contains(&x) {
            return false;
        }
        let Some(surface_y) = landscape.semi_above_solid(x, y) else {
            return false;
        };
        y = surface_y;
        if mouse_trajectory_distance(
            landscape,
            Vector2::new(x, y - height),
            velocity,
            target,
            gravity,
        ) <= 2
        {
            return true;
        }
        x += direction;
    }
    false
}

impl Engine {
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
            player.control.last_com = COM_NONE;
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
        self.refill_player_contents_menus()?;
        self.open_player_auto_context_menus()?;
        Ok(())
    }

    /// Player-menu execution notices refill-container content-count
    /// changes after objects have run (C4Player.cpp:206-212;
    /// C4ObjectMenu.cpp:448-459). Rebuild Get/Contents menus before the
    /// AutoContextMenu tail so exited vehicles disappear immediately.
    fn refill_player_contents_menus(&mut self) -> Result<(), EngineError> {
        let mut pending = self
            .players
            .keys()
            .filter_map(|owner| self.crew_cursor(*owner))
            .filter_map(|crew_id| {
                let crew_index = self.find_object_index(crew_id)?;
                let menu = self.objects[crew_index].state.menu.as_ref()?;
                let identification = match menu.identification {
                    Value::Int(17) => 17,
                    Value::Int(18) => 18,
                    _ => return None,
                };
                let container_id = self.objects[crew_index].state.container?;
                Some((crew_id, container_id, identification))
            })
            .collect::<Vec<_>>();
        pending.sort_unstable_by_key(|(crew_id, _, _)| crew_id.as_u64());
        for (crew_id, container_id, identification) in pending {
            let Some(crew_index) = self.find_object_index(crew_id) else {
                continue;
            };
            let Some(container_index) = self.find_object_index(container_id) else {
                continue;
            };
            self.open_container_contents_menu(crew_index, container_index, identification)?;
        }
        Ok(())
    }

    /// C4Player::Execute's cursor AutoContextMenu tail
    /// (C4Player.cpp:206-212; C4Object.cpp:2044-2062).
    fn open_player_auto_context_menus(&mut self) -> Result<(), EngineError> {
        let mut owners = self
            .players
            .iter()
            .filter_map(|(&owner, player)| player.control.auto_context_menu.then_some(owner))
            .collect::<Vec<_>>();
        owners.sort_unstable();
        for owner in owners {
            let Some(crew_index) = self
                .crew_cursor(owner)
                .and_then(|crew| self.find_object_index(crew))
            else {
                continue;
            };
            if !self.objects[crew_index].commands.is_empty()
                || self.objects[crew_index].state.menu.is_some()
                || !self.objects[crew_index].state.crew_member
            {
                continue;
            }
            let Some(base_index) = self.objects[crew_index]
                .state
                .container
                .and_then(|base| self.find_object_index(base))
            else {
                continue;
            };
            let auto_context = self
                .definitions
                .get(&self.objects[base_index].definition_id)
                .is_some_and(|definition| definition.auto_context_menu());
            if auto_context {
                self.open_context_menu(crew_index, base_index, true)?;
            }
        }
        Ok(())
    }

    /// Internal C4MN_Context refill for an arbitrary target. Automatic
    /// contained menus are permanent; mouse C4CMD_Context menus are not
    /// (C4Object.cpp:1961-1980; C4ObjectMenu.cpp:328-435).
    pub(crate) fn open_context_menu(
        &mut self,
        crew_index: usize,
        base_index: usize,
        permanent: bool,
    ) -> Result<(), EngineError> {
        const C4MN_ITEM_NO_COUNT: i32 = 12_345_678;
        let crew_id = self.objects[crew_index].id;
        let crew_owner = self.objects[crew_index].state.owner;
        let crew_contents = self.objects[crew_index].state.contents.clone();
        let first_carried_definition = crew_contents
            .first()
            .and_then(|object_id| self.find_object_index(*object_id))
            .map(|index| self.objects[index].definition_id.clone());
        let base = &self.objects[base_index];
        let base_id = base.id;
        let base_definition = base.definition_id.clone();
        let base_player = base.state.base;
        let base_is_container = base.state.ocf & ocf::CONTAINER != 0;
        let mut items = Vec::new();
        let item =
            |caption: &str,
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

        if base_is_container && self.objects[crew_index].state.container == Some(base_id) {
            if let Some(first_carried_definition) = first_carried_definition {
                let command2 = if crew_contents.len() > 1
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
                    picture_snapshot: None,
                    picture_object: None,
                    components: Vec::new(),
                    selectable: true,
                    value: None,
                    text_display_progress: -1,
                });
            }
            items.push(item(
                "Contents",
                format!(
                    "SetCommand(this,\"Get\",Object({}),0,0,,2)&&ExecuteCommand()",
                    base_id.as_u64()
                ),
                base_definition.clone(),
                crate::ObjectMenuSymbol::Definition,
            ));
        }
        if self.players.contains_key(&base_player)
            && !self.players_hostile(base_player, crew_owner)
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
        // AddContextFunctions(target): effective `Context*` functions with a
        // description block are evaluated on the target and inserted before
        // Info/Exit (C4ObjectMenu.cpp:398-399,670-682). The menu command runs
        // on the crew and ProtectedCall dispatches back to the target.
        let context_functions = self
            .definitions
            .get(&base_definition)
            .map(|definition| definition.script_context_functions())
            .unwrap_or_default();
        for context in context_functions {
            let image = context.image.as_deref().unwrap_or("NONE");
            let enabled = match context.condition.as_deref() {
                Some(condition) => {
                    let value = self.call_object_function(
                        base_index,
                        condition,
                        vec![
                            compat::object_reference_value(crew_id),
                            Value::C4Id(image.to_owned()),
                        ],
                    )?;
                    compat::value_raw_truthy(&value)
                }
                None => true,
            };
            if !enabled {
                continue;
            }
            items.push(crate::ObjectMenuItem {
                caption: context.label,
                info_caption: crate::normalize_menu_info_caption(
                    context.description.unwrap_or_default(),
                ),
                command: format!(
                    "ProtectedCall(Object({}),\"{}\",this)",
                    base_id.as_u64(),
                    context.function
                ),
                command2: String::new(),
                count: C4MN_ITEM_NO_COUNT,
                item_id: image.to_owned(),
                symbol: crate::ObjectMenuSymbol::Definition,
                image: crate::ObjectMenuImage::default(),
                presentation_definition_id: None,
                picture_snapshot: None,
                picture_object: None,
                components: Vec::new(),
                selectable: true,
                value: None,
                text_display_progress: -1,
            });
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
        let selection = i32::from(!items.is_empty()) - 1;
        let caption = self
            .definitions
            .get(&base_definition)
            .map(|definition| definition.name().to_string())
            .unwrap_or_else(|| base_definition.clone());

        let _ = self.close_object_menu(crew_id, true)?;
        let Some(crew_index) = self.find_object_index(crew_id) else {
            return Ok(());
        };
        self.objects[crew_index].state.menu = Some(crate::ObjectMenuState {
            caption,
            symbol_id: base_definition,
            title_symbol: crate::ObjectMenuSymbol::default(),
            identification: Value::Int(14),
            style: 1,
            equal_item_height: false,
            permanent,
            extra: crate::ObjectMenuExtra::default(),
            extra_data: 0,
            selection,
            user_menu: false,
            command_object: Some(crew_id),
            items,
            columns: 1,
            lines: 0,
            text_progressing: false,
            decoration: None,
        });
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
            extra: crate::ObjectMenuExtra::default(),
            extra_data: 0,
            selection: -1,
            user_menu: false,
            command_object: Some(crew_id),
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

    /// `C4Player::ObjectCom` (C4Player.cpp:1367-1390): commit the cursor
    /// selection on regular coms, then route the com to the cursor object
    /// with an updated controller.
    fn player_object_com(&mut self, owner: i32, com: u8, data: i32) -> Result<(), EngineError> {
        // Eliminated (:1369).
        if self.is_owner_eliminated(owner) {
            return Ok(());
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
    pub(super) fn object_do_select(
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
    pub(super) fn object_un_select(
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
    pub(super) fn player_set_cursor(
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

    /// The player's crew roster in C4Player::Crew order (join order), with
    /// only active objects like the C++ list after ClearPointers.
    fn player_crew_roster(&self, owner: i32) -> Vec<ObjectId> {
        self.players
            .get(&owner)
            .map(|player| player.crew().to_vec())
            .unwrap_or_else(|| self.crew_members(owner))
            .into_iter()
            .filter(|id| {
                self.find_object_index(*id)
                    .is_some_and(|index| self.objects[index].state.status.is_active())
            })
            .collect()
    }

    /// `C4Player::GetHiRankActiveCrew` (C4Player.cpp:1003-1021): without
    /// the crew-info rank model every member ranks -1, so the FIRST
    /// eligible roster entry wins the strict `iRank > iHighestRank` race.
    fn player_hi_rank_active_crew(&self, owner: i32, select_only: bool) -> Option<ObjectId> {
        let selected = self.selected_crew(owner);
        self.player_crew_roster(owner)
            .into_iter()
            .filter(|id| {
                self.find_object_index(*id)
                    .is_some_and(|index| !self.objects[index].state.crew_disabled)
            })
            .find(|id| !select_only || selected.contains(id))
    }

    /// `C4Player::AdjustCursorCommand` (C4Player.cpp:1235-1258).
    pub(super) fn player_adjust_cursor_command(&mut self, owner: i32) -> Result<(), EngineError> {
        // Find hirank Select, else any (:1240-1245).
        let hi_rank = self
            .player_hi_rank_active_crew(owner, true)
            .or_else(|| self.player_hi_rank_active_crew(owner, false));
        let previous = self.crew_cursor(owner);
        if previous != hi_rank {
            self.crew_selection
                .entry(owner)
                .or_default()
                .set_cursor(hi_rank);
        }
        if let Some(player) = self.players.get_mut(&owner) {
            player.set_cursor(hi_rank);
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
        if let Some(cursor_index) = hi_rank.and_then(|id| self.find_object_index(id)) {
            self.object_do_select(cursor_index, owner, false)?;
        }
        if let Some(player) = self.players.get_mut(&owner) {
            player.control.cursor_flash = 30;
        }
        Ok(())
    }

    /// `C4Player::CursorRight` (C4Player.cpp:1261-1275).
    fn player_cursor_right(&mut self, owner: i32) -> Result<(), EngineError> {
        self.player_cursor_step(owner, false)
    }

    /// `C4Player::CursorLeft` (C4Player.cpp:1278-1293).
    fn player_cursor_left(&mut self, owner: i32) -> Result<(), EngineError> {
        self.player_cursor_step(owner, true)
    }

    fn player_cursor_step(&mut self, owner: i32, backwards: bool) -> Result<(), EngineError> {
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

    /// `C4MouseControl::SendPlayerSelectNext` followed by the one-object
    /// `C4ControlPlayerSelect::Execute`: advance in crew-list order and
    /// replace the selection immediately (C4MouseControl.cpp:1284-1300;
    /// C4Control.cpp:341-369).
    pub fn player_mouse_select_next(&mut self, owner: i32) -> Result<bool, EngineError> {
        if !self.players.contains_key(&owner) {
            return Ok(false);
        }
        let roster = self.player_crew_roster(owner);
        let eligible = |engine: &Self, id: ObjectId| {
            engine.find_object_index(id).is_some_and(|index| {
                engine.objects[index].state.status.is_active()
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
        let Some(next) = next else {
            return Ok(false);
        };

        self.player_unselect_crew(owner)?;
        let Some(index) = self.find_object_index(next) else {
            return Ok(false);
        };
        if !self.objects[index].state.status.is_active() {
            return Ok(false);
        }
        self.object_do_select(index, owner, false)?;
        self.player_adjust_cursor_command(owner)?;
        if let Some(player) = self.players.get_mut(&owner) {
            player.control.cursor_selection = 0;
            player.control.cursor_toggled = 0;
            player.control.select_flash = 30;
        }
        Ok(true)
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
    pub fn mouse_drag_carryables_in_rect(
        &self,
        first: Vector2,
        second: Vector2,
    ) -> Vec<ObjectId> {
        let min_x = first.x.min(second.x);
        let max_x = first.x.max(second.x);
        let min_y = first.y.min(second.y);
        let max_y = first.y.max(second.y);
        // `exec_list` is the reverse of C++'s main list (lib.rs:11372-11380).
        self.exec_list
            .iter()
            .rev()
            .filter_map(|id| self.find_object_index(*id).map(|index| &self.objects[index]))
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
    pub fn mouse_drag_carryable_command(
        &self,
        owner: i32,
        position: Vector2,
    ) -> Option<CommandId> {
        let landscape = self.landscape.as_ref()?;
        if landscape.is_liquid_at(position.x, position.y) {
            return Some(CommandId::Drop);
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
            return Some(CommandId::Drop);
        }

        let (throw_force, throw_height, cursor_x) = self
            .crew_cursor(owner)
            .and_then(|id| self.find_object_index(id))
            .map(|index| {
                (
                    math::val_by_physical(400, self.object_physical(index).throw),
                    self.objects[index]
                        .current_shape_rect()
                        .map(|rect| rect.height)
                        .unwrap_or(20),
                    self.objects[index].state.position.x,
                )
            })
            .unwrap_or((math::val_by_physical(400, 50_000), 20, position.x));
        let preferred_direction = if cursor_x > position.x { -1 } else { 1 };
        [preferred_direction, -preferred_direction]
            .into_iter()
            .any(|direction| {
                mouse_has_throwing_position(
                    landscape,
                    position,
                    FixedVec2::new(throw_force * direction, -throw_force),
                    throw_height,
                    self.physics.gravity_as_c4fixed(),
                )
            })
            .then_some(CommandId::Throw)
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
        let target_ocf = self.object_ocf_for_pos(index, point);
        let definition = self.definitions.get(&object.definition_id)?;

        // UpdateCursorTarget installs Grab first and then lets Carryable
        // replace it (C4MouseControl.cpp:486-501).
        let mut source = (target_ocf & ocf::GRAB != 0 && definition.grab() == 1)
            .then_some(MouseDragSource::Vehicle);
        if target_ocf & ocf::CARRYABLE != 0 {
            source = Some(MouseDragSource::Carryable);
        }

        // These cursor decisions occur after Carryable and therefore make
        // DragNone ignore the object as a moving-drag source.
        if target_ocf & ocf::CHOP != 0 {
            let width = object
                .current_shape_rect()
                .map(|shape| shape.width)
                .unwrap_or(0);
            let dx = point.x - object.state.position.x;
            let dy = point.y - object.state.position.y;
            if (-width / 3..=width / 3).contains(&dx)
                && (-width / 2..=width / 3).contains(&dy)
            {
                source = None;
            }
        }
        let hostile_alive = target_ocf & ocf::ALIVE != 0
            && self
                .players
                .get(&owner)
                .zip(self.players.get(&object.state.owner))
                .is_some_and(|(player, target_owner)| {
                    player.id() != target_owner.id()
                        && (player.is_hostile_towards(target_owner.id())
                            || target_owner.is_hostile_towards(player.id()))
                });
        if target_ocf & (ocf::ENTRANCE | ocf::CONSTRUCT) != 0
            || object.state.category & CATEGORY_MOUSE_SELECT != 0
            || target_ocf & ocf::ALIVE != 0 && self.player_crew_roster(owner).contains(&target)
            || hostile_alive
        {
            source = None;
        }

        // The nearby jump cursor is evaluated last and overrides every
        // object cursor (C4MouseControl.cpp:522-534).
        if self
            .crew_cursor(owner)
            .and_then(|cursor| self.find_object_index(cursor))
            .is_some_and(|cursor_index| {
                let cursor = &self.objects[cursor_index];
                if cursor.state.container.is_some()
                    || self.object_procedure(cursor_index) != ActionProcedure::Walk
                {
                    return false;
                }
                let dx = point.x - cursor.state.position.x;
                let dy = point.y - cursor.state.position.y;
                (-25..=-10).contains(&dy)
                    && ((-15..=-1).contains(&dx) || (1..=15).contains(&dx))
            })
        {
            return None;
        }
        source
    }

    /// The moving-drag class for a copied viewport region target. Regions
    /// use cached OCF_Carryable but the definition's raw Grab=1 value rather
    /// than the world cursor's position-filtered OCF (C4MouseControl.cpp:
    /// 942-961).
    pub fn mouse_region_drag_source(&self, target: ObjectId) -> Option<MouseDragSource> {
        let index = self.find_object_index(target)?;
        let object = &self.objects[index];
        if !object.state.status.is_active() {
            return None;
        }
        if object.state.ocf & ocf::CARRYABLE != 0 {
            return Some(MouseDragSource::Carryable);
        }
        self.definitions
            .get(&object.definition_id)
            .filter(|definition| definition.grab() == 1)
            .map(|_| MouseDragSource::Vehicle)
    }

    /// Build C4MouseControl's local Selection when dragging from a viewport
    /// region. A right drag expands a contained target to every live object
    /// with the exact same C4ID, preserving forward Contents order; otherwise
    /// it contains only the copied region target (C4MouseControl.cpp:942-961).
    pub fn mouse_region_drag_objects(
        &self,
        target: ObjectId,
        right_button: bool,
    ) -> Vec<ObjectId> {
        if self.mouse_region_drag_source(target).is_none() {
            return Vec::new();
        }
        let Some(index) = self.find_object_index(target) else {
            return Vec::new();
        };
        let object = &self.objects[index];
        let Some(container) = right_button.then_some(object.state.container).flatten() else {
            return vec![target];
        };
        let Some(container_index) = self.find_object_index(container) else {
            return vec![target];
        };
        let same_id = self.objects[container_index]
            .state
            .contents
            .iter()
            .copied()
            .filter(|candidate| {
                self.find_object_index(*candidate).is_some_and(|candidate_index| {
                    let candidate = &self.objects[candidate_index];
                    candidate.state.status.is_active()
                        && candidate.definition_id == object.definition_id
                })
            })
            .collect::<Vec<_>>();
        if same_id.len() > 1 {
            same_id
        } else {
            vec![target]
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
        let requested = requested.into_iter().collect::<Vec<_>>();
        let selected = self
            .player_crew_roster(owner)
            .into_iter()
            .filter(|id| requested.contains(id))
            .collect::<Vec<_>>();

        self.player_unselect_crew(owner)?;
        for id in selected {
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
        Ok(true)
    }

    /// `C4Player::UnselectCrew` (C4Player.cpp:1295-1306).
    pub(super) fn player_unselect_crew(&mut self, owner: i32) -> Result<(), EngineError> {
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
    fn player_select_single_by_cursor(&mut self, owner: i32) -> Result<(), EngineError> {
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
    fn player_cursor_toggle(&mut self, owner: i32) -> Result<(), EngineError> {
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
    fn player_select_all_crew(&mut self, owner: i32) -> Result<(), EngineError> {
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
        self.pending_audio.push(crate::AudioCommand::PlaySound {
            name: "Ding".to_string(),
            target: None,
            volume: 100,
            looped: false,
            multiple: false,
            custom_falloff: None,
        });
        Ok(())
    }

    /// `C4Player::UpdateSelectionToggleStatus` (C4Player.cpp:1355-1365).
    fn player_update_selection_toggle_status(&mut self, owner: i32) -> Result<(), EngineError> {
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
    fn object_menu_control(
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
                let auto_context_exit = !menu.user_menu
                    && menu.permanent
                    && menu.identification == Value::Int(14);
                if self.close_object_menu(object_id, false)? && auto_context_exit {
                    // C4Object::AutoContextMenu's CloseCommand is invoked
                    // only for a control close (C4Menu.cpp:327-331), not
                    // when another menu force-replaces the context menu.
                    let owner = self.objects[index].state.owner;
                    self.player_object_command(owner, CommandId::Exit, None, 0, 0)?;
                    self.execute_object_command_now(object_id)?;
                }
            }
            COM_MENU_LEFT => {
                let delta = if menu.selection - 1 < 0 {
                    menu.items.len() as i32 - 1 - menu.selection
                } else {
                    -1
                };
                self.move_object_menu_selection(index, delta)?;
            }
            COM_MENU_RIGHT => {
                let delta = if menu.selection + 1 >= menu.items.len() as i32 {
                    -menu.selection
                } else {
                    1
                };
                self.move_object_menu_selection(index, delta)?;
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
    fn enter_internal_context_put(
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
    fn enter_internal_context_exit(
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

    fn player_context_put(
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
            if !self.objects[index].state.status.is_active() {
                continue;
            }
            let mut contents = self.objects[index].state.contents.clone();
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
                PlayerObjectCommandMode::Set,
            )?;
        }
        Ok(())
    }

    /// `C4Menu::MoveSelection` (C4Menu.cpp:535-555): advance in fixed
    /// increments until a selectable item is found, without crossing the
    /// menu bounds.
    fn move_object_menu_selection(
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

    /// `C4Menu::SetSelection(..., fDoCalls=true)` +
    /// `C4ObjectMenu::OnSelectionChanged` (C4Menu.cpp:557-594;
    /// C4ObjectMenu.cpp:93-104).
    fn set_object_menu_selection(
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
        let Some(command_index) = menu
            .command_object
            .and_then(|command_object| self.find_object_index(command_object))
            .filter(|&command_index| {
                self.definitions
                    .get(&self.objects[command_index].definition_id)
                    .is_some_and(|definition| definition.has_function("OnMenuSelection"))
            })
        else {
            // CB_Scenario selection callbacks remain part of the scenario
            // script-menu gap; a missing callback is a silent C++ miss.
            return Ok(());
        };
        let args = vec![Value::Int(menu.selection), compat::object_reference_value(object_id)];
        if let Err(error) = self.call_object_function(command_index, "OnMenuSelection", args) {
            tracing::warn!(
                %error,
                "script error in OnMenuSelection; continuing like the C++ fail-safe exec"
            );
        }
        Ok(())
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
    fn object_push_com(&mut self, index: usize, com: u8) -> Result<(), EngineError> {
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
                let _ = self.object_call_control(
                    target_index,
                    controller,
                    com,
                    Some(clonk_id),
                )?;
            }
        }
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
    fn auto_stop_push_com(&mut self, index: usize, com: u8) -> Result<(), EngineError> {
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
                let _ = self.object_call_control(
                    target_index,
                    controller,
                    com,
                    Some(clonk_id),
                )?;
            }
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
        let sf = self.object_has_function(container_index, &function);
        // New definitions may overload hardcoded controls; old definitions
        // receive the callback only after those controls have run
        // (C4Object.cpp:3246-3251,3284-3291).
        let call_sf_early = self
            .definitions
            .get(&self.objects[container_index].definition_id)
            .is_none_or(|definition| definition.version_at_least([4, 9, 1, 3]));
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
            if sf {
                if let Some(container_index) = self
                    .objects
                    .get(index)
                    .and_then(|object| object.state.container)
                    .and_then(|id| self.find_object_index(id))
                {
                    let clonk_ref = compat::object_reference_value(self.objects[index].id);
                    let _ = self.contained_call(container_index, &function, &[clonk_ref])?;
                }
            }
            self.contained_control_update(index, com, controller)?;
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
    /// target. C4Object owns this permanent menu directly.
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
        if buy {
            self.open_base_buy_menu(index, container_index)?;
        } else {
            self.open_base_sell_menu(index, container_index)?;
        }
        Ok(())
    }

    /// C4Object::ActivateMenu(C4MN_Buy) plus the immediate
    /// C4ObjectMenu::SetRefillObject/Refill pass (C4Object.cpp:1919-1930;
    /// C4ObjectMenu.cpp:207-237).
    pub(crate) fn open_base_buy_menu(
        &mut self,
        crew_index: usize,
        base_index: usize,
    ) -> Result<(), EngineError> {
        let crew_id = self.objects[crew_index].id;
        let previous_selection = self.objects[crew_index]
            .state
            .menu
            .as_ref()
            .filter(|menu| menu.identification == Value::Int(4))
            .map(|menu| menu.selection);
        let base_id = self.objects[base_index].id;
        let base_player = self.objects[base_index].state.base;
        let base_owner = self.objects[base_index].state.owner;
        let mut material = self
            .players
            .get(&base_player)
            .map(|player| {
                player
                    .home_base_material()
                    .iter()
                    .map(|(definition_id, count)| (definition_id.clone(), *count))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        // C4IDList has a stable order. Player currently stores this list in
        // a HashMap, so impose a deterministic order until that model is
        // replaced by the C++ ordered list.
        material.sort_by(|(left, _), (right, _)| left.cmp(right));
        let items = material
            .into_iter()
            .filter_map(|(definition_id, count)| {
                let definition = self.definitions.get(&definition_id)?;
                let count = i32::try_from(count).unwrap_or(i32::MAX);
                let command = format!(
                    "AppendCommand(this,\"Buy\",Object({}),1,0,,0,{})&&ExecuteCommand()",
                    base_id.as_u64(), definition_id
                );
                let command2 = format!(
                    "AppendCommand(this,\"Buy\",Object({}),{},0,,0,{})&&ExecuteCommand()",
                    base_id.as_u64(), count, definition_id
                );
                Some(crate::ObjectMenuItem {
                    caption: format!("Buy {}", definition.name()),
                    info_caption: crate::normalize_menu_info_caption(
                        definition.description().unwrap_or_default(),
                    ),
                    command,
                    command2,
                    count,
                    item_id: definition_id,
                    symbol: crate::ObjectMenuSymbol::default(),
                    image: crate::ObjectMenuImage::default(),
                    presentation_definition_id: None,
                    picture_snapshot: None,
                    picture_object: None,
                    components: Vec::new(),
                    selectable: true,
                    value: Some(definition.value()),
                    text_display_progress: -1,
                })
            })
            .collect::<Vec<_>>();
        // C4ObjectMenu rebuilds Buy rows with ClearItems(false), preserving
        // the numeric slot. C4Menu::AdjustSelection keeps it when valid and
        // otherwise walks backward to the final selectable row
        // (C4ObjectMenu.cpp:207-237; C4Menu.cpp:947-988,1014-1038).
        let selection = if items.is_empty() {
            -1
        } else {
            let last = i32::try_from(items.len() - 1).unwrap_or(i32::MAX);
            previous_selection.unwrap_or(0).clamp(0, last)
        };

        let _ = self.close_object_menu(crew_id, true)?;
        let Some(crew_index) = self.find_object_index(crew_id) else {
            return Ok(());
        };
        self.objects[crew_index].state.menu = Some(crate::ObjectMenuState {
            caption: "There is nothing to buy.".to_string(),
            symbol_id: String::new(),
            title_symbol: crate::ObjectMenuSymbol::Buy { owner: base_owner },
            identification: Value::Int(4),
            style: 0,
            equal_item_height: false,
            permanent: true,
            extra: crate::ObjectMenuExtra::Value,
            extra_data: 0,
            selection,
            user_menu: false,
            command_object: Some(crew_id),
            items,
            columns: 5,
            lines: 0,
            text_progressing: false,
            decoration: None,
        });
        Ok(())
    }

    /// The row partition produced by `C4ObjectListIterator::GetNext` for an
    /// object menu: category-ineligible entries are invisible, same-ID chunks
    /// retain list order, and only `CanConcatPictureWith`-equal objects share
    /// a row (C4ObjectList.cpp:849-903).
    fn object_menu_picture_groups(
        &self,
        contents: &[ObjectId],
        category_mask: i32,
    ) -> Vec<(ObjectId, i32)> {
        let eligible = contents
            .iter()
            .filter_map(|object_id| {
                let index = self.find_object_index(*object_id)?;
                let object = &self.objects[index];
                (!object.destroyed
                    && object.state.status.is_active()
                    && object.state.category & category_mask != 0)
                    .then_some(*object_id)
                    .and_then(|object_id| {
                        self.object_snapshot(object_id)
                            .map(|snapshot| (object_id, snapshot))
                    })
            })
            .collect::<Vec<_>>();
        let mut groups = Vec::new();
        let mut chunk_start = 0usize;
        while chunk_start < eligible.len() {
            let chunk_definition = eligible[chunk_start].1.definition_id.as_str();
            let chunk_end = eligible[chunk_start..]
                .iter()
                .position(|(_, object)| object.definition_id != chunk_definition)
                .map(|offset| chunk_start + offset)
                .unwrap_or(eligible.len());

            for current in chunk_start..chunk_end {
                // GetNext's duplicate scan is deliberately directional:
                // each prior object asks whether it concatenates `current`.
                if (chunk_start..current).any(|prior| {
                    self.can_concat_picture_with(&eligible[prior].1, &eligible[current].1)
                }) {
                    continue;
                }
                // Its piCount scan uses the reverse direction: each later
                // object asks whether it concatenates the representative.
                let count = 1usize
                    + (current + 1..chunk_end)
                        .filter(|later| {
                            self.can_concat_picture_with(
                                &eligible[*later].1,
                                &eligible[current].1,
                            )
                        })
                        .count();
                // C4ObjectMenu prefers the first live fully-constructed
                // same-ID object when the iterator's representative is
                // incomplete and the two pictures concatenate. This changes
                // the picture/primary command target, not piCount
                // (C4ObjectMenu.cpp:182-199,252-271,292-321;
                // C4ObjectList.cpp:271-281).
                let representative = if eligible[current].1.ocf & crate::ocf::FULL_CON == 0 {
                    contents
                        .iter()
                        .filter_map(|candidate| {
                            let index = self.find_object_index(*candidate)?;
                            let object = &self.objects[index];
                            (!object.destroyed
                                && object.state.status.is_active()
                                && object.definition_id == chunk_definition
                                && object.state.ocf & crate::ocf::FULL_CON != 0)
                                .then(|| self.object_snapshot(*candidate))
                                .flatten()
                                .filter(|snapshot| {
                                    self.can_concat_picture_with(snapshot, &eligible[current].1)
                                })
                                .map(|_| *candidate)
                        })
                        .next()
                        .unwrap_or(eligible[current].0)
                } else {
                    eligible[current].0
                };
                groups.push((
                    representative,
                    i32::try_from(count).unwrap_or(i32::MAX),
                ));
            }
            chunk_start = chunk_end;
        }

        groups
    }

    /// `C4ObjectList::ObjectCount(id)` counts every live same-ID content,
    /// independently of the category/picture group used for the visible row
    /// (C4ObjectList.cpp:320-329; C4ObjectMenu.cpp:266-267,317-319).
    fn live_contents_definition_count(&self, contents: &[ObjectId], definition_id: &str) -> i32 {
        let count = contents
            .iter()
            .filter_map(|candidate| self.find_object_index(*candidate))
            .filter(|&candidate| {
                let candidate = &self.objects[candidate];
                !candidate.destroyed
                    && candidate.state.status.is_active()
                    && candidate.definition_id == definition_id
            })
            .count();
        i32::try_from(count).unwrap_or(i32::MAX)
    }

    /// `ClearItems(false)` keeps the numeric slot. `checkIDSelection` first
    /// accepts that slot when its C4ID survived, otherwise finds the first row
    /// carrying the old C4ID; `AdjustSelection` supplies the numeric fallback
    /// (C4ObjectMenu.cpp:147-164; C4Menu.cpp:975-1017).
    fn refilled_object_menu_selection(
        items: &[crate::ObjectMenuItem],
        previous_selection: Option<i32>,
        selected_definition: Option<&str>,
    ) -> i32 {
        if items.is_empty() {
            return -1;
        }
        if let (Some(previous), Some(selected)) = (previous_selection, selected_definition) {
            if usize::try_from(previous)
                .ok()
                .and_then(|selection| items.get(selection))
                .is_some_and(|item| item.item_id == selected)
            {
                return previous;
            }
        }
        if let Some(selection) = selected_definition
            .and_then(|selected| items.iter().position(|item| item.item_id == selected))
            .and_then(|selection| i32::try_from(selection).ok())
        {
            return selection;
        }
        let last = i32::try_from(items.len() - 1).unwrap_or(i32::MAX);
        previous_selection.unwrap_or(0).clamp(0, last)
    }

    /// C4Object::ActivateMenu(C4MN_Sell) plus C4ObjectMenu's immediate
    /// refill over the base's stContents list (C4Object.cpp:1932-1943;
    /// C4ObjectMenu.cpp:238-277).
    pub(crate) fn open_base_sell_menu(
        &mut self,
        crew_index: usize,
        base_index: usize,
    ) -> Result<(), EngineError> {
        const CATEGORY_TRADE_LIVING: i32 = 1 << 16;
        let crew_id = self.objects[crew_index].id;
        let (previous_selection, selected_definition) = self.objects[crew_index]
            .state
            .menu
            .as_ref()
            .filter(|menu| menu.identification == Value::Int(5))
            .map(|menu| {
                let selected_definition = usize::try_from(menu.selection)
                    .ok()
                    .and_then(|selection| menu.items.get(selection))
                    .map(|item| item.item_id.clone());
                (Some(menu.selection), selected_definition)
            })
            .unwrap_or((None, None));
        let base_id = self.objects[base_index].id;
        let base_owner = self.objects[base_index].state.owner;
        let base_definition = self.objects[base_index].definition_id.clone();
        let contents = self.objects[base_index].state.contents.clone();
        let sell_category = crate::CATEGORY_STATIC_BACK
            | crate::CATEGORY_STRUCTURE
            | crate::CATEGORY_VEHICLE
            | crate::CATEGORY_OBJECT
            | CATEGORY_TRADE_LIVING;
        let mut items = Vec::new();

        for (item_id, count) in self.object_menu_picture_groups(&contents, sell_category) {
            let Some(item_index) = self.find_object_index(item_id) else {
                continue;
            };
            let item = &self.objects[item_index];
            let definition_id = item.definition_id.clone();
            let all_count = self.live_contents_definition_count(&contents, &definition_id);
            let Some(definition) = self.definitions.get(&definition_id) else {
                continue;
            };
            let command = format!(
                "AppendCommand(this,\"Sell\",Object({}),1,0,Object({}),0,{})&&ExecuteCommand()",
                base_id.as_u64(),
                item_id.as_u64(),
                definition_id
            );
            let command2 = format!(
                "AppendCommand(this,\"Sell\",Object({}),{},0,,0,{})&&ExecuteCommand()",
                base_id.as_u64(),
                all_count,
                definition_id
            );
            items.push(crate::ObjectMenuItem {
                caption: format!("Sell {}", definition.name()),
                info_caption: crate::normalize_menu_info_caption(
                    definition.description().unwrap_or_default(),
                ),
                command,
                command2,
                count,
                item_id: definition_id,
                symbol: crate::ObjectMenuSymbol::default(),
                image: crate::ObjectMenuImage::default(),
                presentation_definition_id: None,
                picture_snapshot: None,
                picture_object: Some(item_id),
                components: Vec::new(),
                selectable: true,
                value: Some(definition.value()),
                text_display_progress: -1,
            });
        }

        // ClearItems(false) leaves C++'s numeric selection in place while
        // checkIDSelection restores the selected C4ID after refill. If that
        // C4ID vanished, AdjustSelection keeps the old slot when valid and
        // otherwise walks backward to the final row (C4ObjectMenu.cpp:
        // 147-164,238-275; C4Menu.cpp:975-1017).
        let selection = Self::refilled_object_menu_selection(
            &items,
            previous_selection,
            selected_definition.as_deref(),
        );
        let base_name = self
            .definitions
            .get(&base_definition)
            .map(|definition| definition.name().to_string())
            .unwrap_or_else(|| base_definition.clone());
        let _ = self.close_object_menu(crew_id, true)?;
        let Some(crew_index) = self.find_object_index(crew_id) else {
            return Ok(());
        };
        self.objects[crew_index].state.menu = Some(crate::ObjectMenuState {
            caption: format!("{} is empty.", base_name),
            symbol_id: String::new(),
            title_symbol: crate::ObjectMenuSymbol::Sell { owner: base_owner },
            identification: Value::Int(5),
            style: 0,
            equal_item_height: false,
            permanent: true,
            extra: crate::ObjectMenuExtra::Value,
            extra_data: 0,
            selection,
            user_menu: false,
            command_object: Some(crew_id),
            items,
            columns: 5,
            lines: 0,
            text_progressing: false,
            decoration: None,
        });
        Ok(())
    }

    /// C4Object::ActivateMenu(C4MN_Get/C4MN_Contents) plus the immediate
    /// contents refill (C4Object.cpp:1945-1959; C4ObjectMenu.cpp:279-326).
    pub(crate) fn open_container_contents_menu(
        &mut self,
        crew_index: usize,
        container_index: usize,
        identification: i32,
    ) -> Result<(), EngineError> {
        const CATEGORY_TRADE_LIVING: i32 = 1 << 16;
        let crew_id = self.objects[crew_index].id;
        let (previous_selection, selected_definition) = self.objects[crew_index]
            .state
            .menu
            .as_ref()
            .filter(|menu| menu.identification == Value::Int(identification))
            .map(|menu| {
                let selected_definition = usize::try_from(menu.selection)
                    .ok()
                    .and_then(|selection| menu.items.get(selection))
                    .map(|item| item.item_id.clone());
                (Some(menu.selection), selected_definition)
            })
            .unwrap_or((None, None));
        let container_id = self.objects[container_index].id;
        let container_definition = self.objects[container_index].definition_id.clone();
        let has_entrance = self.objects[container_index].state.ocf & ocf::ENTRANCE != 0;
        let contents = self.objects[container_index].state.contents.clone();
        let mut items = Vec::new();
        let get_category = crate::CATEGORY_STATIC_BACK
            | crate::CATEGORY_STRUCTURE
            | crate::CATEGORY_VEHICLE
            | crate::CATEGORY_OBJECT
            | CATEGORY_TRADE_LIVING;

        for (item_id, count) in self.object_menu_picture_groups(&contents, get_category) {
            let Some(item_index) = self.find_object_index(item_id) else {
                continue;
            };
            let item = &self.objects[item_index];
            let definition_id = item.definition_id.clone();
            let all_count = self.live_contents_definition_count(&contents, &definition_id);
            let carryable = item.state.ocf & ocf::CARRYABLE != 0;
            let get = carryable || !has_entrance;
            let command_name = if get { "Get" } else { "Activate" };
            let item_definition = self.definitions.get(&definition_id);
            let item_name = item_definition
                .map(|definition| definition.name())
                .unwrap_or(definition_id.as_str());
            let info_caption = item_definition
                .and_then(|definition| definition.description())
                .map(crate::normalize_menu_info_caption)
                .unwrap_or_default();
            let command = format!(
                "SetCommand(this, \"{}\", Object({})) && ExecuteCommand()",
                command_name,
                item_id.as_u64()
            );
            let command2 = (all_count > 1)
                .then(|| {
                    format!(
                        "SetCommand(this, \"{}\", , {},0, Object({}), {}) && ExecuteCommand()",
                        command_name,
                        all_count,
                        container_id.as_u64(),
                        definition_id
                    )
                })
                .unwrap_or_default();
            items.push(crate::ObjectMenuItem {
                caption: format!("{} {}", command_name, item_name),
                info_caption,
                command,
                command2,
                count,
                item_id: definition_id,
                symbol: crate::ObjectMenuSymbol::default(),
                image: crate::ObjectMenuImage::default(),
                presentation_definition_id: None,
                picture_snapshot: None,
                picture_object: Some(item_id),
                components: Vec::new(),
                selectable: true,
                value: None,
                text_display_progress: -1,
            });
        }

        let selection = Self::refilled_object_menu_selection(
            &items,
            previous_selection,
            selected_definition.as_deref(),
        );
        let container_name = self
            .definitions
            .get(&container_definition)
            .map(|definition| definition.name().to_string())
            .unwrap_or_else(|| container_definition.clone());
        let _ = self.close_object_menu(crew_id, true)?;
        let Some(crew_index) = self.find_object_index(crew_id) else {
            return Ok(());
        };
        self.objects[crew_index].state.menu = Some(crate::ObjectMenuState {
            caption: format!("{} is empty.", container_name),
            symbol_id: container_definition,
            title_symbol: crate::ObjectMenuSymbol::default(),
            identification: Value::Int(identification),
            style: 0,
            equal_item_height: false,
            permanent: true,
            extra: crate::ObjectMenuExtra::default(),
            extra_data: 0,
            selection,
            user_menu: false,
            command_object: Some(crew_id),
            items,
            columns: 5,
            lines: 0,
            text_progressing: false,
            decoration: None,
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

    /// `DrawCommandQuery`'s function-presence and `Method=` filter
    /// (C4ScriptHost.cpp:95-118; C4Object.cpp:2938-2951). C4Aul functions
    /// default to `All`; an unknown Method value also falls back to `All`
    /// (C4AulLink.cpp:200; C4AulParse.cpp:355-367).
    fn object_control_command_is_visible(
        &self,
        index: usize,
        controller: i32,
        function: &str,
    ) -> bool {
        let Some(control_style) = self
            .players
            .get(&controller)
            .map(|player| player.control.control_style)
        else {
            return false;
        };
        let Some(function) = self
            .definitions
            .get(&self.objects[index].definition_id)
            .and_then(|definition| definition.script.functions().get(function))
        else {
            return false;
        };
        let method = function.description.as_deref().and_then(|description| {
            description.split('|').find_map(|segment| {
                let (key, value) = segment.split_once('=')?;
                key.trim()
                    .eq_ignore_ascii_case("Method")
                    .then(|| value.trim())
            })
        });
        match method {
            Some(method) if method.eq_ignore_ascii_case("None") => false,
            Some(method) if method.eq_ignore_ascii_case("Classic") => !control_style,
            Some(method) if method.eq_ignore_ascii_case("JumpAndRun") => control_style,
            _ => true,
        }
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
    /// current front cannot concat-picture with, using the full definition,
    /// color, graphics, name, and overlay rules; select it via
    /// DirectComContents.
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
        let Some(front) = self.object_snapshot(front_id) else {
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
            let Some(candidate) = self.object_snapshot(candidate_id) else {
                continue;
            };
            if !self.can_concat_picture_with(&front, &candidate) {
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
                    multiple: false,
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
    fn object_com_stop(&mut self, index: usize) -> Result<bool, EngineError> {
        let definition_id = self.objects[index].definition_id.clone();
        self.object_com_stop_action(index, &definition_id)
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
        if physical.can_dig == 0 || !self.force_action_with_calls(index, &definition_id, "Dig")? {
            return Ok(false);
        }
        // ObjectActionDig resets the Dig2Object request (:143).
        self.objects[index].state.action.data = 0;
        Ok(true)
    }

    /// `ObjectComDigDouble` (C4ObjectCom.cpp:531-571) — "activation":
    /// contents Activate, linekit construction, chop, then own Activate.
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
            // LNKT's script may decline activation; C++ then performs its
            // engine-side ObjectComLineConstruction fallback (:542-547).
            if self
                .find_object_index(contents_id)
                .is_some_and(|contents_index| {
                    self.objects[contents_index].definition_id == "LNKT"
                })
                && self.object_com_line_construction(index, contents_id)?
            {
                return Ok(());
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

    /// Carried-LNKT half of `ObjectComLineConstruction`
    /// (C4ObjectCom.cpp:429-528): finish a live line attached to the kit,
    /// or choose and create a new line at the full-con structure under the
    /// Clonk. The no-kit line-pickup half (:392-427) remains separate work.
    fn object_com_line_construction(
        &mut self,
        clonk_index: usize,
        linekit_id: ObjectId,
    ) -> Result<bool, EngineError> {
        let Some(linekit_index) = self.find_object_index(linekit_id) else {
            return Ok(false);
        };
        if self.objects[linekit_index].definition_id != "LNKT"
            || self.objects[linekit_index].destroyed
        {
            return Ok(false);
        }

        let clonk_id = self.objects[clonk_index].id;
        let position = self.objects[clonk_index].state.position;
        let active_line = self.objects.iter().position(|object| {
            !object.destroyed
                && object.state.status.is_active()
                && object.state.action.name == "Connect"
                && (object.state.action.target == Some(linekit_id)
                    || object.state.action.target2 == Some(linekit_id))
        });

        let Some((structure_index, structure_id, structure_ocf)) =
            self.at_object(position, ocf::LINE_CONSTRUCT, Some(clonk_id))
        else {
            return Ok(false);
        };
        if structure_ocf & ocf::LINE_CONSTRUCT == 0 {
            return Ok(false);
        }

        if let Some(line_index) = active_line {
            let first = self.objects[line_index].state.action.target;
            let second = self.objects[line_index].state.action.target2;
            if first == Some(structure_id) || second == Some(structure_id) {
                let _ = self.objects[line_index].mark_destroyed();
                self.update_sector_for_index(line_index);
                self.note_objects_changed();
                return Ok(true);
            }

            let line_type = self
                .definitions
                .get(&self.objects[line_index].definition_id)
                .map(|definition| definition.line())
                .unwrap_or_default();
            let line_connect = self
                .definitions
                .get(&self.objects[structure_index].definition_id)
                .map(|definition| definition.line_connect())
                .unwrap_or_default();
            let connect_ok = match line_type {
                1 => {
                    line_connect
                        & (crate::LINE_CONNECT_POWER_INPUT | crate::LINE_CONNECT_POWER_OUTPUT)
                        != 0
                }
                2 => line_connect & crate::LINE_CONNECT_LIQUID_OUTPUT != 0,
                3 => line_connect & crate::LINE_CONNECT_LIQUID_INPUT != 0,
                _ => false,
            };
            if !connect_ok {
                return Ok(false);
            }

            if first == Some(linekit_id) {
                self.objects[line_index].state.action.target = Some(structure_id);
            }
            if second == Some(linekit_id) {
                self.objects[line_index].state.action.target2 = Some(structure_id);
            }
            self.objects[clonk_index]
                .state
                .contents
                .retain(|&id| id != linekit_id);
            self.objects[linekit_index].state.container = None;
            let _ = self.objects[linekit_index].mark_destroyed();
            self.update_sector_for_index(linekit_index);
            self.note_objects_changed();
            return Ok(true);
        }

        let line_connect = self
            .definitions
            .get(&self.objects[structure_index].definition_id)
            .map(|definition| definition.line_connect())
            .unwrap_or_default();
        let has_connected_line = |engine: &Self, definition_id: &str| {
            engine.objects.iter().any(|object| {
                !object.destroyed
                    && object.definition_id == definition_id
                    && object.state.action.name == "Connect"
                    && object.state.action.target == Some(structure_id)
            })
        };
        let line_definition = if line_connect & crate::LINE_CONNECT_LIQUID_PUMP != 0
            && !has_connected_line(self, "SPIP")
        {
            Some("SPIP")
        } else if line_connect & crate::LINE_CONNECT_LIQUID_OUTPUT != 0
            && !has_connected_line(self, "DPIP")
        {
            Some("DPIP")
        } else if line_connect & crate::LINE_CONNECT_POWER_OUTPUT != 0 {
            Some("PWRL")
        } else {
            None
        };
        let Some(line_definition) = line_definition else {
            return Ok(false);
        };
        if !self.definitions.contains_key(line_definition) {
            return Ok(false);
        }

        let owner = self.objects[clonk_index].state.owner;
        let line_position = self.objects[structure_index].state.position;
        let line_id = self.spawn_object_with_initial_lifecycle(
            crate::SpawnConfig::new(line_definition)
                .with_position(line_position)
                .with_owner(owner),
            Some(structure_id),
        )?;
        let Some(line_index) = line_id.and_then(|id| self.find_object_index(id)) else {
            return Ok(false);
        };
        let line = &mut self.objects[line_index];
        line.state.action.name = "Connect".to_owned();
        line.state.action.phase = 0;
        line.state.action.ticks = 0;
        line.state.action.time = 0;
        line.state.action.target = Some(structure_id);
        line.state.action.target2 = Some(linekit_id);
        Ok(true)
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

    /// `C4Command::Jump` followed by `ObjectComJump` (C4Command.cpp:
    /// 1056-1067; C4ObjectCom.cpp:280-307). This stays live because
    /// ObjectActionJump synchronously invokes the object's OnActionJump hook.
    pub(crate) fn execute_jump_command(
        &mut self,
        object_id: ObjectId,
        tx: i32,
    ) -> Result<(), EngineError> {
        let Some(index) = self.find_object_index(object_id) else {
            return Ok(());
        };
        // Tx==0 is the C++ sentinel: do not reinterpret it as world x=0.
        if tx != 0 {
            let x = self.objects[index].state.position.x;
            let direction = if tx < x {
                Some(Direction::Left)
            } else if tx > x {
                Some(Direction::Right)
            } else {
                None
            };
            if let Some(direction) = direction {
                let definition_id = self.objects[index].definition_id.clone();
                self.set_command_action_direction(index, &definition_id, direction)?;
            }
        }
        let _ = self.object_com_jump(index)?;
        // C4Command::Jump calls Finish(true) only after ObjectComJump and its
        // synchronous OnActionJump callback return (C4Command.cpp:1064-1067).
        if let Some(index) = self.find_object_index(object_id) {
            self.objects[index]
                .commands
                .finish_front_if(CommandId::Jump);
        }
        Ok(())
    }

    /// `ObjectComJump` (C4ObjectCom.cpp:280-307): predict a deep-liquid
    /// landing from the shape's bottom vertex before falling back to the
    /// script-overridable regular jump.
    fn object_com_jump(&mut self, index: usize) -> Result<bool, EngineError> {
        if self.object_procedure(index) != ActionProcedure::Walk {
            return Ok(false);
        }
        let launch = crate::command::object_com_jump_launch(
            self.objects[index].state.construction,
            self.object_physical(index),
            self.objects[index].state.command_direction,
            self.objects[index].state.direction,
        );
        // ObjectComJump reads pObj->Shape.ContactDensity, not Def->Shape
        // (C4ObjectCom.cpp:297-305). SetContactDensity therefore changes the
        // dive gate independently for every live object.
        let contact_density = self.objects[index].state.contact_density;
        if contact_density > 25
            && self.object_com_jump_hits_liquid(index, launch)
            && self.object_action_dive(index, launch.x, launch.y)?
        {
            return Ok(true);
        }
        self.object_action_jump_com(index, launch.x, launch.y, true)
    }

    /// `SimFlightHitsLiquid` (C4Movement.cpp:657-670), including the
    /// ten-frame escape when the bottom vertex already starts in water.
    fn object_com_jump_hits_liquid(&self, index: usize, launch: FixedVec2) -> bool {
        let Some(object) = self.objects.get(index) else {
            return false;
        };
        // Despite the name, C4Shape::GetBottomVertex selects the CNAT_Bottom
        // vertex with the smallest VtxY (C4Shape.cpp:445-455).
        let bottom = object
            .state
            .vertices
            .iter()
            .filter(|vertex| vertex.cnat & crate::CNAT_BOTTOM != 0)
            .min_by_key(|vertex| vertex.y);
        let mut position = object.fixed_position;
        if let Some(bottom) = bottom {
            position.x += bottom.x;
            position.y += bottom.y;
        }
        let mut velocity = launch;
        let Some(landscape) = self.landscape.as_ref() else {
            return false;
        };
        let solid_mask_indices = (0..self.objects.len()).collect::<Vec<_>>();
        let solid_masks = self.solid_masks_for_movement(&solid_mask_indices);
        let density_at =
            |x, y| crate::movement_density_at(landscape, &self.materials, &solid_masks, None, x, y);
        let width = landscape.width() as i32;
        let height = landscape.estimated_height();
        let gravity = self.physics.gravity_as_c4fixed();
        let liquid = |density| (25..50).contains(&density);

        if liquid(density_at(
            crate::math::fixtoi(position.x),
            crate::math::fixtoi(position.y),
        )) && !sim_flight_to_density(
            &mut position,
            &mut velocity,
            0,
            24,
            10,
            gravity,
            width,
            height,
            &density_at,
        ) {
            return false;
        }
        if !sim_flight_to_density(
            &mut position,
            &mut velocity,
            25,
            100,
            -1,
            gravity,
            width,
            height,
            &density_at,
        ) {
            return false;
        }
        let x = crate::math::fixtoi(position.x);
        let y = crate::math::fixtoi(position.y);
        liquid(density_at(x, y)) && liquid(density_at(x, y + 9))
    }

    /// `ObjectActionDive` (C4ObjectCom.cpp:63-72): unlike a regular jump,
    /// Dive has no OnActionJump callback.
    fn object_action_dive(
        &mut self,
        index: usize,
        xdir: crate::C4Fixed,
        ydir: crate::C4Fixed,
    ) -> Result<bool, EngineError> {
        let definition_id = self.objects[index].definition_id.clone();
        if !self.action_with_calls(index, &definition_id, "Dive")? {
            return Ok(false);
        }
        let object = &mut self.objects[index];
        object.fixed_velocity = FixedVec2::new(xdir, ydir);
        object.state.velocity = Vector2::new(crate::math::fixtoi(xdir), crate::math::fixtoi(ydir));
        object.state.mobile = true;
        object.frame_t_attach &= !crate::CNAT_BOTTOM;
        object.state.t_attach &= !crate::CNAT_BOTTOM;
        Ok(true)
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
        if !self.action_with_calls(index, &definition_id, "Jump")? {
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
        object.state.t_attach &= !crate::CNAT_BOTTOM;
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
        if !self.object_action_stand(index, &definition_id)? {
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
    #[doc(hidden)]
    pub fn player_object_command(
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
        let mode = if ranged {
            PlayerObjectCommandMode::Add
        } else {
            PlayerObjectCommandMode::Set
        };
        self.player_crew_object_command(owner, command, target, None, tx, ty, mode, ranged)
    }

    /// `C4MouseControl::ButtonUpDragMoving`: issue one independent carryable
    /// Drop/Throw command per locally selected object. The first packet uses
    /// C4P_Command_Set and every later packet uses C4P_Command_Append, so each
    /// selected crew member handles every object in mouse-list order
    /// (C4MouseControl.cpp:1171-1201; C4Player.cpp:1397-1450).
    pub fn player_mouse_drag_objects<I>(
        &mut self,
        owner: i32,
        command: CommandId,
        objects: I,
        position: Vector2,
    ) -> Result<bool, EngineError>
    where
        I: IntoIterator<Item = ObjectId>,
    {
        if !self.players.contains_key(&owner)
            || !matches!(command, CommandId::Drop | CommandId::Throw)
        {
            return Ok(false);
        }
        let mut mode = PlayerObjectCommandMode::Set;
        let mut issued = false;
        for target in objects {
            let active = self.find_object_index(target).is_some_and(|index| {
                self.objects[index].state.status.is_active()
            });
            if !active {
                continue;
            }
            self.player_update_selection_toggle_status(owner)?;
            self.player_crew_object_command(
                owner,
                command,
                Some(target),
                None,
                position.x,
                position.y,
                mode,
                false,
            )?;
            mode = PlayerObjectCommandMode::Append;
            issued = true;
        }
        Ok(issued)
    }

    /// Control-modified carryable drag onto an `OCF_Container`: each packet
    /// is `Put(Target=container, Target2=dragged object, X=Y=0)`. The first
    /// object replaces the crew command stack and the rest append in mouse
    /// selection order; Shift makes the first packet append as well
    /// (C4MouseControl.cpp:742-768,1171-1219).
    pub fn player_mouse_drag_put<I>(
        &mut self,
        owner: i32,
        objects: I,
        container: ObjectId,
        append_to_existing: bool,
    ) -> Result<bool, EngineError>
    where
        I: IntoIterator<Item = ObjectId>,
    {
        if !self.players.contains_key(&owner)
            || !self.find_object_index(container).is_some_and(|index| {
                let object = &self.objects[index];
                object.state.status.is_active() && object.state.ocf & ocf::CONTAINER != 0
            })
        {
            return Ok(false);
        }
        let mut mode = if append_to_existing {
            PlayerObjectCommandMode::Append
        } else {
            PlayerObjectCommandMode::Set
        };
        let mut issued = false;
        for object in objects {
            let active = self.find_object_index(object).is_some_and(|index| {
                self.objects[index].state.status.is_active()
            });
            if !active {
                continue;
            }
            self.player_update_selection_toggle_status(owner)?;
            self.player_crew_object_command(
                owner,
                CommandId::Put,
                Some(container),
                Some(object),
                0,
                0,
                mode,
                false,
            )?;
            mode = PlayerObjectCommandMode::Append;
            issued = true;
        }
        Ok(issued)
    }

    /// Issue ButtonUpDragMoving's vehicle commands. Every selected Grab=1
    /// object receives `PushTo(Target=vehicle, Target2=optional container)`
    /// at the release coordinates; the first packet is Set and later packets
    /// Append, while Shift makes the first packet Append too
    /// (C4MouseControl.cpp:1171-1227).
    pub fn player_mouse_drag_vehicles<I>(
        &mut self,
        owner: i32,
        vehicles: I,
        position: Vector2,
        put_target: Option<ObjectId>,
        append_to_existing: bool,
    ) -> Result<bool, EngineError>
    where
        I: IntoIterator<Item = ObjectId>,
    {
        if !self.players.contains_key(&owner) {
            return Ok(false);
        }
        let put_target = put_target.filter(|target| {
            self.find_object_index(*target).is_some_and(|index| {
                let object = &self.objects[index];
                object.state.status.is_active() && object.state.ocf & ocf::CONTAINER != 0
            })
        });
        let mut mode = if append_to_existing {
            PlayerObjectCommandMode::Append
        } else {
            PlayerObjectCommandMode::Set
        };
        let mut issued = false;
        for vehicle in vehicles {
            let active_vehicle = self.find_object_index(vehicle).is_some_and(|index| {
                let object = &self.objects[index];
                object.state.status.is_active()
                    && self
                        .definitions
                        .get(&object.definition_id)
                        .is_some_and(|definition| definition.grab() == 1)
            });
            if !active_vehicle {
                continue;
            }
            self.player_update_selection_toggle_status(owner)?;
            self.player_crew_object_command(
                owner,
                CommandId::PushTo,
                Some(vehicle),
                put_target,
                position.x,
                position.y,
                mode,
                false,
            )?;
            mode = PlayerObjectCommandMode::Append;
            issued = true;
        }
        Ok(issued)
    }

    /// Mouse `C4CMD_Context`: unlike ordinary PlayerObjectCommand, the
    /// clicked object occupies Target2 while Target remains null, and Add
    /// mode does not apply the ±15 cursor range (C4MouseControl.cpp:
    /// 1253-1260; C4Player.cpp:1397-1451).
    pub fn player_context_command(
        &mut self,
        owner: i32,
        target: ObjectId,
    ) -> Result<bool, EngineError> {
        if !self.players.contains_key(&owner) {
            return Ok(false);
        }
        self.player_update_selection_toggle_status(owner)?;
        self.player_crew_object_command(
            owner,
            CommandId::Context,
            None,
            Some(target),
            0,
            0,
            PlayerObjectCommandMode::Add,
            false,
        )
    }

    /// `C4Player::ObjectCommand` (C4Player.cpp:1397-1443): apply to all
    /// selected crew in cursor range except the target, then always to the
    /// cursor. `ranged` mirrors C4P_Command_Add|C4P_Command_Range.
    fn player_crew_object_command(
        &mut self,
        owner: i32,
        command: CommandId,
        target: Option<ObjectId>,
        target2: Option<ObjectId>,
        tx: i32,
        ty: i32,
        mode: PlayerObjectCommandMode,
        ranged: bool,
    ) -> Result<bool, EngineError> {
        let cursor = self.crew_cursor(owner);
        let cursor_position = cursor
            .and_then(|id| self.find_object_index(id))
            .map(|index| self.objects[index].state.position);
        let selected = self.selected_crew(owner);
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
            self.object_command_to_obj(index, command, target, target2, tx, ty, mode)?;
        }
        // Always apply to cursor, even if it's not selected (:1436-1439).
        if let Some(cursor_id) = cursor {
            if !cursor_processed && Some(cursor_id) != target {
                if let Some(index) = self.find_object_index(cursor_id) {
                    if self.objects[index].state.status.is_active() {
                        self.object_command_to_obj(
                            index, command, target, target2, tx, ty, mode,
                        )?;
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
        target2: Option<ObjectId>,
        tx: i32,
        ty: i32,
        mode: PlayerObjectCommandMode,
    ) -> Result<(), EngineError> {
        let request = CommandRequest::new(command)
            .with_target(target)
            .with_target2(target2)
            .with_tx((tx != 0).then_some(tx))
            .with_ty((ty != 0).then_some(ty))
            .with_mode(CommandMode::Base);
        match mode {
            PlayerObjectCommandMode::Add => {
                // C4P_Command_Add → AddCommand(..., fAppend=false): push front
                // without clearing (C4Command.cpp AddCommand semantics).
                self.objects[index]
                    .apply_command_operations([CommandOperation::PushFront(request)]);
                return Ok(());
            }
            PlayerObjectCommandMode::Append => {
                // C4P_Command_Append → AddCommand(..., fAppend=true): retain
                // the independent command sequence in list order.
                self.objects[index]
                    .apply_command_operations([CommandOperation::PushBack(request)]);
                return Ok(());
            }
            PlayerObjectCommandMode::Set => {}
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
            target2
                .map(compat::object_reference_value)
                .unwrap_or(Value::Nil),
            Value::Int(0),
        ];
        let overloaded = self
            .contained_call(index, "ControlCommand", &args)
            .map(|value| compat::value_raw_truthy(&value))
            .unwrap_or(false);
        if overloaded {
            return Ok(());
        }
        // Inside vehicle control overload (:3947-3961): the container's
        // ControlCommand with the clonk appended in slot 7.
        if let Some(container_index) = self
            .objects
            .get(index)
            .and_then(|object| object.state.container)
            .and_then(|id| self.find_object_index(id))
        {
            let inside = self
                .definitions
                .get(&self.objects[container_index].definition_id)
                .is_some_and(|definition| {
                    definition.vehicle_control() & crate::VEHICLE_CONTROL_INSIDE != 0
                });
            if inside {
                let controller = self.objects[index].state.controller;
                self.objects[container_index].state.controller = controller;
                let mut vehicle_args = args.to_vec();
                vehicle_args.push(compat::object_reference_value(object_id));
                let consumed = self
                    .contained_call(container_index, "ControlCommand", &vehicle_args)
                    .map(|value| compat::value_raw_truthy(&value))
                    .unwrap_or(false);
                if consumed {
                    return Ok(());
                }
            }
        }
        // Outside vehicle control overload (:3962-3974): the pushed
        // target's ControlCommand, plain six args.
        if self.object_procedure(index) == ActionProcedure::Push {
            if let Some(target_index) = self.objects[index]
                .state
                .action
                .target
                .and_then(|id| self.find_object_index(id))
            {
                let outside = self
                    .definitions
                    .get(&self.objects[target_index].definition_id)
                    .is_some_and(|definition| {
                        definition.vehicle_control() & crate::VEHICLE_CONTROL_OUTSIDE != 0
                    });
                if outside {
                    let controller = self.objects[index].state.controller;
                    self.objects[target_index].state.controller = controller;
                    let consumed = self
                        .contained_call(target_index, "ControlCommand", &args)
                        .map(|value| compat::value_raw_truthy(&value))
                        .unwrap_or(false);
                    if consumed {
                        return Ok(());
                    }
                }
            }
        }
        self.objects[index].apply_command_operations([CommandOperation::PushFront(request)]);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ActionSpec, ActionState, Definition, MovementProfile, PhysicalInfo, PhysicsSettings,
        PlayerConfig, SpawnConfig,
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
        actions.insert(
            "Push".to_string(),
            ActionSpec::default().with_procedure("push"),
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
    fn cursor_script_menu_consumes_controls_before_gameplay_like_cpp() {
        // C4Player::InCom converts regular cursor-menu input before the
        // single/double machinery (C4Player.cpp:1502-1513), then
        // C4Object::DirectCom gives Menu->Control first refusal
        // (C4Object.cpp:3363-3371). Dragon Rock depends on this ordering:
        // its mandatory difficulty/type menus must complete before Up can
        // become ObjectComUp/Jump.
        let script = r#"
        local chosen;
        func OpenMenu() {
            CreateMenu(WIPF, this(), this(), 0, "Choose");
            AddMenuItem("First", "Choose(1)", WIPF, this());
            AddMenuItem("Second", "Choose(2)", WIPF, this());
            return 1;
        }
        func Choose(value) { chosen = value; return 1; }
        func MenuQueryCancel() { return 1; }
        "#;
        let mut engine = Engine::new();
        register_clonk(&mut engine, "CLNK", script);
        engine
            .register_player(PlayerConfig::new(1, "Test"))
            .expect("player");
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        let index = engine.find_object_index(crew).expect("crew exists");
        engine
            .call_object_function(index, "OpenMenu", Vec::new())
            .expect("menu opens");

        let menu = engine
            .debug_object_menu(crew.as_u64())
            .expect("crew exists")
            .expect("menu open");
        assert_eq!(menu.selection, 0);

        engine.player_in_com(1, COM_RIGHT, 0).expect("menu right");
        let menu = engine
            .debug_object_menu(crew.as_u64())
            .expect("crew exists")
            .expect("menu stays open");
        assert_eq!(menu.selection, 1, "Right navigates the script menu");
        assert_eq!(
            engine.object_snapshot(crew).expect("crew snapshot").command_direction,
            CommandDirection::Stop,
            "menu navigation must not leak into gameplay steering"
        );
        engine
            .player_in_com(1, COM_RIGHT + COM_RELEASE_OFFSET, 0)
            .expect("menu right release");
        assert_eq!(
            engine
                .debug_object_menu(crew.as_u64())
                .expect("crew exists")
                .expect("menu stays open")
                .selection,
            1,
            "the raw release neither navigates again nor leaks"
        );

        engine.player_in_com(1, COM_DIG, 0).expect("menu close");
        assert!(
            engine
                .debug_object_menu(crew.as_u64())
                .expect("crew exists")
                .is_some(),
            "MenuQueryCancel may deny the soft close"
        );

        engine.player_in_com(1, COM_THROW, 0).expect("menu enter");
        assert_eq!(engine.debug_object_menu(crew.as_u64()), Some(None));
        let index = engine.find_object_index(crew).expect("crew survives");
        assert_eq!(
            engine.objects[index].state.local_vars.get("chosen"),
            Some(&Value::Int(2)),
            "Enter executes the selected command"
        );
        engine
            .player_in_com(1, COM_THROW + COM_RELEASE_OFFSET, 0)
            .expect("throw release is ignored");

        engine.player_in_com(1, COM_UP, 0).expect("gameplay up");
        engine.tick().expect("queued jump executes");
        assert_eq!(
            engine.object_snapshot(crew).expect("crew snapshot").action.name,
            "Jump",
            "once the mandatory menu closes, Up reaches ObjectComUp"
        );
    }

    #[test]
    fn menu_show_text_reveals_every_progressive_row_like_cpp() {
        // C4Menu::Control(COM_MenuShowText) calls SetTextProgress(-1),
        // revealing all rows without activating a command (C4Menu.cpp:
        // 477-480). This command is already converted and synchronized.
        let script = r#"
        func OpenMenu() {
            CreateMenu(CLNK, this(), this(), 0, "", 0, 3);
            AddMenuItem("First", "", NONE, this());
            AddMenuItem("Continue", "Choose", CLNK, this());
            AddMenuItem("Last", "", NONE, this());
            return SetMenuTextProgress(0, this());
        }
        "#;
        let mut engine = Engine::new();
        register_clonk(&mut engine, "CLNK", script);
        engine
            .register_player(PlayerConfig::new(1, "Test"))
            .expect("player");
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        let index = engine.find_object_index(crew).expect("crew exists");
        engine
            .call_object_function(index, "OpenMenu", Vec::new())
            .expect("menu opens");
        assert!(
            engine
                .debug_object_menu(crew.as_u64())
                .expect("crew exists")
                .expect("menu open")
                .text_progressing
        );

        engine
            .player_in_com(1, COM_MENU_SHOW_TEXT, 0)
            .expect("show text command");
        let menu = engine
            .debug_object_menu(crew.as_u64())
            .expect("crew exists")
            .expect("menu stays open");
        assert!(!menu.text_progressing);
        assert!(
            menu.items
                .iter()
                .all(|item| item.text_display_progress == -1)
        );
    }

    #[test]
    fn empty_script_menu_ignores_explicit_select_like_cpp() {
        // C4Menu::Control guards COM_MenuSelect with ItemCount before
        // SetSelection and its callback (C4Menu.cpp:474-476).
        let script = r#"
        local selection_calls;
        func OpenMenu() {
            selection_calls = 0;
            CreateMenu(WIPF, this(), this(), 0, "Empty");
            return 1;
        }
        func OnMenuSelection() { selection_calls++; return 1; }
        "#;
        let mut engine = Engine::new();
        register_clonk(&mut engine, "CLNK", script);
        engine
            .register_player(PlayerConfig::new(1, "Test"))
            .expect("player");
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        let index = engine.find_object_index(crew).expect("crew exists");
        engine
            .call_object_function(index, "OpenMenu", Vec::new())
            .expect("empty menu opens");

        engine
            .player_in_com(1, COM_MENU_SELECT, 0)
            .expect("menu select is accepted");
        let index = engine.find_object_index(crew).expect("crew survives");
        assert_eq!(
            engine.objects[index].state.local_vars.get("selection_calls"),
            Some(&Value::Int(0)),
            "an empty menu must not run OnMenuSelection"
        );
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
    fn old_pushed_target_receives_classic_control_after_clonk_fallback() {
        // Before 4.9.5 pushed targets receive ControlLeft only after the
        // Clonk's DFA_PUSH fallback has moved it (src/C4Object.cpp:3520-3568).
        // The callback return value cannot consume that earlier fallback.
        let vehicle = r#"
#strict
protected func ControlLeft(pByClonk) { DoDamage(1); return(1); }
"#;
        let mut engine = Engine::new();
        register_clonk(&mut engine, "CLNK", "#strict\n");
        let mut lorry =
            Definition::from_script("LORY", "Lorry", vehicle).expect("lorry compiles");
        lorry.set_version([4, 9, 4, 9, 0]);
        engine.register_definition(lorry).expect("register lorry");
        engine
            .register_player(PlayerConfig::new(1, "Test"))
            .expect("player");
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        let lorry = engine
            .spawn_object(SpawnConfig::new("LORY"))
            .expect("spawn lorry");
        let crew_index = engine.find_object_index(crew).expect("crew exists");
        engine.objects[crew_index].state.action.name = "Push".to_string();
        engine.objects[crew_index].state.action.target = Some(lorry);

        engine.player_in_com(1, COM_LEFT, 0).expect("in_com");

        assert_eq!(
            engine
                .object_snapshot(crew)
                .expect("crew survives")
                .command_direction,
            CommandDirection::Left,
            "the old target's truthy late callback cannot consume movement"
        );
        assert_eq!(
            engine.object_snapshot(lorry).expect("lorry survives").damage,
            1,
            "the old target still receives ControlLeft after movement"
        );
    }

    #[test]
    fn version_4_9_5_pushed_target_consumes_classic_and_autostop_fallbacks() {
        // At 4.9.5 the target callback moves before both DFA_PUSH fallback
        // switches, and a truthy return consumes the control
        // (src/C4Object.cpp:3520-3568,3682-3738).
        let vehicle = r#"
#strict
protected func ControlLeft(pByClonk) { DoDamage(1); return(1); }
"#;
        for control_style in [false, true] {
            let mut engine = Engine::new();
            register_clonk(&mut engine, "CLNK", "#strict\n");
            let mut lorry =
                Definition::from_script("LORY", "Lorry", vehicle).expect("lorry compiles");
            lorry.set_version([4, 9, 5, 0, 0]);
            engine.register_definition(lorry).expect("register lorry");
            engine
                .register_player(PlayerConfig::new(1, "Test"))
                .expect("player");
            engine
                .players
                .get_mut(&1)
                .expect("player exists")
                .control
                .control_style = control_style;
            let crew = spawn_crew(&mut engine, "CLNK", 1);
            let lorry = engine
                .spawn_object(SpawnConfig::new("LORY"))
                .expect("spawn lorry");
            let crew_index = engine.find_object_index(crew).expect("crew exists");
            engine.objects[crew_index].state.action.name = "Push".to_string();
            engine.objects[crew_index].state.action.target = Some(lorry);

            engine.player_in_com(1, COM_LEFT, 0).expect("in_com");

            assert_eq!(
                engine
                    .object_snapshot(crew)
                    .expect("crew survives")
                    .command_direction,
                CommandDirection::Stop,
                "the modern target consumes the control for style={control_style}"
            );
            assert_eq!(
                engine.object_snapshot(lorry).expect("lorry survives").damage,
                1
            );
        }
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
    fn queued_jump_runs_live_on_action_jump_before_hardcoded_launch() {
        // C4Command::Jump calls live ObjectComJump (C4Command.cpp:1056-1067),
        // whose ObjectActionJump first calls the object-owned fail-safe hook
        // OnActionJump(xdir*100, ydir*100, true). A truthy result suppresses
        // the hardcoded Jump action and velocity assignment
        // (C4ObjectCom.cpp:48-61,280-307).
        let script = r#"
#strict
local jump_calls, jump_xdir, jump_ydir, jump_by_com;
protected func OnActionJump(int xdir, int ydir, bool by_com)
{
    jump_calls++;
    jump_xdir = xdir;
    jump_ydir = ydir;
    jump_by_com = by_com;
    return true;
}
"#;
        let mut engine = Engine::new();
        register_clonk(&mut engine, "CLNK", script);
        engine
            .register_player(PlayerConfig::new(1, "Test"))
            .expect("player");
        let crew = spawn_crew(&mut engine, "CLNK", 1);

        engine.player_in_com(1, COM_UP, 0).expect("queue jump");
        engine.tick().expect("execute queued jump");

        let snapshot = engine.object_snapshot(crew).expect("crew survives");
        assert_eq!(snapshot.action.name, "Walk");
        assert_eq!(snapshot.velocity, Vector2::ZERO);
        assert!(snapshot.command_stack.command_names().is_empty());
        let index = engine.find_object_index(crew).expect("crew exists");
        let locals = &engine.objects[index].state.local_vars;
        assert_eq!(locals.get("jump_calls"), Some(&Value::Int(1)));
        assert_eq!(locals.get("jump_xdir"), Some(&Value::Int(-196)));
        assert_eq!(locals.get("jump_ydir"), Some(&Value::Int(-400)));
        assert_eq!(locals.get("jump_by_com"), Some(&Value::Bool(true)));
    }

    #[test]
    fn queued_jump_honors_no_other_action_selected_by_false_hook() {
        // ObjectActionJump uses ordinary SetActionByName("Jump"), not a
        // forced transition. A false OnActionJump may therefore select a
        // NoOtherAction action that rejects the hardcoded jump
        // (C4ObjectCom.cpp:48-61; C4Object.cpp:4111-4115).
        let script = r#"
#strict
protected func OnActionJump()
{
    SetAction("Locked");
    return false;
}
"#;
        let mut engine = Engine::new();
        let mut definition = Definition::from_script("CLNK", "CLNK", script)
            .expect("script compiles");
        let mut actions = clonk_actions();
        actions.insert(
            "Locked".to_string(),
            ActionSpec::default()
                .with_procedure("walk")
                .with_no_other_action(true),
        );
        definition.configure_actions(Some("Walk".to_string()), actions);
        definition.set_movement_profile(MovementProfile::default());
        definition.set_physical(PhysicalInfo {
            walk: 70_000,
            jump: 40_000,
            ..Default::default()
        });
        engine.register_definition(definition).expect("register");
        engine
            .register_player(PlayerConfig::new(1, "Test"))
            .expect("player");
        let crew = spawn_crew(&mut engine, "CLNK", 1);

        engine.player_in_com(1, COM_UP, 0).expect("queue jump");
        engine.tick().expect("execute queued jump");

        let snapshot = engine.object_snapshot(crew).expect("crew survives");
        assert_eq!(snapshot.action.name, "Locked");
        assert_eq!(snapshot.velocity, Vector2::ZERO);
        assert!(snapshot.command_stack.command_names().is_empty());
    }

    #[test]
    fn object_com_jump_clears_script_visible_bottom_attachment() {
        // ObjectActionJump clears Action.t_attach's bottom bit immediately,
        // before C4Command::Finish and ControlCommandFinished
        // (C4ObjectCom.cpp:54-61; C4Object.cpp:3997-4008).
        let mut engine = Engine::new();
        register_clonk(&mut engine, "CLNK", "#strict\n");
        engine
            .register_player(PlayerConfig::new(1, "Test"))
            .expect("player");
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        let index = engine.find_object_index(crew).expect("crew exists");
        engine.objects[index].state.t_attach = crate::CNAT_BOTTOM | crate::CNAT_LEFT;
        engine.objects[index].frame_t_attach = crate::CNAT_BOTTOM | crate::CNAT_LEFT;

        engine
            .execute_jump_command(crew, 0)
            .expect("execute live jump command");

        let index = engine.find_object_index(crew).expect("crew survives");
        assert_eq!(engine.objects[index].state.t_attach, crate::CNAT_LEFT);
        assert_eq!(engine.objects[index].frame_t_attach, crate::CNAT_LEFT);
    }

    #[test]
    fn script_native_jump_applies_mobile_and_bottom_unstick() {
        // FnJump delegates synchronously to ObjectComJump, whose regular
        // fallback sets Mobile and clears CNAT_Bottom after installing Jump
        // (C4Script.cpp:358-363; C4ObjectCom.cpp:48-61,280-307).
        let mut engine = Engine::new();
        register_clonk(
            &mut engine,
            "CLNK",
            "#strict\nfunc Probe() { return Jump(); }\n",
        );
        engine
            .register_player(PlayerConfig::new(1, "Test"))
            .expect("player");
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        let index = engine.find_object_index(crew).expect("crew exists");
        engine.objects[index].state.t_attach = crate::CNAT_BOTTOM | crate::CNAT_LEFT;
        engine.objects[index].frame_t_attach = crate::CNAT_BOTTOM | crate::CNAT_LEFT;

        assert_eq!(
            engine
                .call_object_function(index, "Probe", Vec::new())
                .expect("Probe calls native Jump"),
            Value::Bool(true)
        );

        let index = engine.find_object_index(crew).expect("crew survives");
        assert_eq!(engine.objects[index].state.action.name, "Jump");
        assert!(engine.objects[index].state.mobile);
        assert_eq!(engine.objects[index].state.t_attach, crate::CNAT_LEFT);
        assert_eq!(engine.objects[index].frame_t_attach, crate::CNAT_LEFT);
    }

    #[test]
    fn queued_jump_target_direction_obeys_current_action_direction_count() {
        // C4Command::Jump targets through C4Object::SetDir. Directions=1
        // rejects DIR_Right, even though Tx lies to the object's right
        // (C4Command.cpp:1058-1063; C4Object.cpp:4235-4253).
        let mut engine = Engine::new();
        register_clonk(&mut engine, "CLNK", "#strict\n");
        engine
            .register_player(PlayerConfig::new(1, "Test"))
            .expect("player");
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        let index = engine.find_object_index(crew).expect("crew exists");
        let target_x = engine.objects[index].state.position.x + 10;
        engine.objects[index].apply_command_operations([CommandOperation::PushFront(
            CommandRequest::new(CommandId::Jump).with_tx(Some(target_x)),
        )]);

        engine.tick().expect("execute targeted jump");

        assert_eq!(
            engine.object_snapshot(crew).expect("crew survives").direction,
            Direction::Left
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
    fn dig_double_with_linekit_starts_and_connects_power_line() {
        // When LNKT's script Activate does not consume DigDouble, C++ falls
        // through to ObjectComLineConstruction: a full-con structure under
        // the Clonk with C4D_Power_Output starts PWRL from that structure to
        // the carried kit (C4ObjectCom.cpp:542-547,487-528).
        let mut engine = Engine::new();
        register_clonk(&mut engine, "CLNK", "#strict\n");

        let linekit = Definition::from_script("LNKT", "Linekit", "#strict\n")
            .expect("linekit compiles");
        engine
            .register_definition(linekit)
            .expect("linekit registers");

        let mut generator =
            Definition::from_script("POWR", "Generator", "#strict\n").expect("generator compiles");
        generator.set_shape_rect(Some(crate::DefinitionRect::new(-20, -20, 40, 40)));
        generator.set_line_connect(crate::LINE_CONNECT_POWER_OUTPUT);
        engine
            .register_definition(generator)
            .expect("generator registers");
        let mut consumer =
            Definition::from_script("CONS", "Consumer", "#strict\n").expect("consumer compiles");
        consumer.set_shape_rect(Some(crate::DefinitionRect::new(-20, -20, 40, 40)));
        consumer.set_line_connect(crate::LINE_CONNECT_POWER_INPUT);
        engine
            .register_definition(consumer)
            .expect("consumer registers");

        let mut line = Definition::from_script("PWRL", "Power line", "#strict\n")
            .expect("line compiles");
        line.set_line(1);
        line.set_shape_vertices(vec![
            crate::ObjectVertex {
                x: 0,
                y: 0,
                cnat: 0,
                friction: 0,
            },
            crate::ObjectVertex {
                x: 0,
                y: 0,
                cnat: 0,
                friction: 0,
            },
        ]);
        line.configure_actions(
            Some("Connect".to_owned()),
            HashMap::from([(
                "Connect".to_owned(),
                ActionSpec::default().with_procedure("connect"),
            )]),
        );
        engine.register_definition(line).expect("line registers");

        engine
            .register_player(PlayerConfig::new(1, "Test"))
            .expect("player");
        let generator = engine
            .spawn_object(
                // NewObject's initial DoCon keeps the supplied bottom at
                // y=120, yielding a full-con centre at y=100.
                SpawnConfig::new("POWR").with_position(Vector2::new(100, 120)),
            )
            .expect("generator spawns");
        let consumer = engine
            .spawn_object(
                SpawnConfig::new("CONS").with_position(Vector2::new(200, 120)),
            )
            .expect("consumer spawns");
        let crew = engine
            .spawn_object(
                SpawnConfig::new("CLNK")
                    .with_owner(1)
                    .with_crew_member(true)
                    .with_position(Vector2::new(100, 100))
                    .with_action(ActionState::new("Walk")),
            )
            .expect("crew spawns");
        engine.select_crew(1, vec![crew]).expect("select");
        engine.set_crew_cursor(1, Some(crew)).expect("cursor");
        let kit = engine
            .spawn_object(
                SpawnConfig::new("LNKT")
                    .with_owner(1)
                    .with_container(crew),
            )
            .expect("kit spawns");
        let generator_index = engine
            .find_object_index(generator)
            .expect("generator remains live");
        assert_ne!(
            engine.object_ocf_at_index(generator_index) & ocf::LINE_CONSTRUCT,
            0,
            "full-con power output advertises OCF_LineConstruct"
        );
        assert!(
            engine
                .at_object(Vector2::new(100, 100), ocf::LINE_CONSTRUCT, Some(crew))
                .is_some(),
            "the generator is under the Clonk's line-construction point"
        );

        engine.player_in_com(1, COM_DIG, 0).expect("first press");
        engine.player_in_com(1, COM_DIG, 0).expect("second press");

        let power_line = engine
            .snapshot()
            .objects
            .into_iter()
            .find(|object| object.definition_id == "PWRL")
            .expect("DigDouble creates PWRL");
        assert_eq!(power_line.action.name, "Connect");
        assert_eq!(power_line.action.target, Some(generator));
        assert_eq!(power_line.action.target2, Some(kit));

        // At the other endpoint, the same real DigDouble accepts PWRL on a
        // C4D_Power_Input structure, swaps the kit endpoint, exits the kit,
        // and removes it (C4ObjectCom.cpp:429-484).
        engine
            .player_in_com(1, COM_DIG + COM_RELEASE_OFFSET, 0)
            .expect("release first double");
        let crew_index = engine.find_object_index(crew).expect("crew remains live");
        engine.objects[crew_index].set_position(Vector2::new(200, 100));
        engine.update_sector_for_index(crew_index);
        engine.player_in_com(1, COM_DIG, 0).expect("third press");
        engine
            .player_in_com(1, COM_DIG + COM_RELEASE_OFFSET, 0)
            .expect("third release");
        engine.player_in_com(1, COM_DIG, 0).expect("fourth press");

        let connected = engine
            .object_snapshot(power_line.id)
            .expect("power line remains live after connection");
        assert_eq!(connected.action.target, Some(generator));
        assert_eq!(connected.action.target2, Some(consumer));
        assert!(
            engine
                .find_object_index(kit)
                .is_none_or(|index| engine.objects[index].destroyed),
            "the connected LNKT is removed"
        );
        assert!(!engine
            .object_snapshot(crew)
            .expect("crew remains live")
            .contents
            .contains(&kit));
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
        // At 4.9.1.3+, a falsy ContainedLeft still reaches the hardcoded
        // Take/Take2 tail (C4Object.cpp:3246-3251,3293-3302).
        let mut engine = Engine::new();
        register_clonk(&mut engine, "CLNK", "#strict\n");
        let mut hut_def = Definition::from_script(
            "HUT1",
            "Hut",
            "#strict\nprotected func ContainedLeft(pByClonk) { return(0); }\n",
        )
        .expect("hut compiles");
        hut_def.set_version([4, 9, 1, 3, 0]);
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

    #[test]
    fn old_contained_left_function_suppresses_take_even_when_falsy() {
        // Before 4.9.1.3 any ContainedLeft function suppresses the Take
        // fallback because the callback runs after hardcoded controls
        // (src/C4Object.cpp:3284-3302).
        let mut engine = Engine::new();
        register_clonk(&mut engine, "CLNK", "#strict\n");
        let mut hut_def = Definition::from_script(
            "HUT1",
            "Hut",
            "#strict\nprotected func ContainedLeft(pByClonk) { DoDamage(1); return(0); }\n",
        )
        .expect("hut compiles");
        hut_def.set_version([4, 9, 1, 2, 0]);
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

        assert_eq!(engine.object_snapshot(hut).expect("hut survives").damage, 1);
        assert!(
            engine
                .object_snapshot(crew)
                .expect("crew survives")
                .command_stack
                .command_names()
                .is_empty(),
            "the presence of an old late ContainedLeft suppresses Take"
        );
    }

    #[test]
    fn contained_throw_executes_the_new_command_immediately() {
        // C4Object::ContainedControl evaluates
        // `PlayerObjectCommand(..., C4CMD_Throw) && ExecuteCommand()` in
        // one control call (C4Object.cpp:3280-3282). The completed Throw
        // command is therefore gone before control returns.
        for command in [COM_THROW, COM_THROW_D] {
            let mut engine = Engine::new();
            register_clonk(&mut engine, "CLNK", "#strict\n");
            engine
                .register_definition(
                    Definition::from_script("HUT1", "Hut", "#strict\n")
                        .expect("hut compiles"),
                )
                .expect("register");
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

            engine.player_in_com(1, command, 0).expect("throw");

            let snapshot = engine.object_snapshot(crew).expect("snapshot");
            assert!(
                snapshot.command_stack.is_empty(),
                "ContainedControl executes and clears Throw synchronously"
            );
        }
    }

    #[test]
    fn kayak_contained_throw_queues_the_explicit_activate_menu() {
        // Reduced shipped KAJO::ContainedThrow: a full kayak queues an
        // Activate command on its contained Clonk with the kayak in Target2
        // (FarWorlds.../Kajak.c4d/Occupied.c4d/Script.c:123-133).
        let kayak = r#"
#strict 2
protected func ContainedThrow(object clonk)
{
    return AddCommand(clonk, "Activate", 0, 0, 0, this());
}
"#;
        let mut engine = Engine::new();
        register_clonk(&mut engine, "CLNK", "#strict\n");
        engine
            .register_definition(
                Definition::from_script("KAJO", "Occupied kayak", kayak)
                    .expect("kayak compiles"),
            )
            .expect("register kayak");
        engine
            .register_player(PlayerConfig::new(1, "Paddler"))
            .expect("register player");
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        let kayak = engine
            .spawn_object(SpawnConfig::new("KAJO").with_owner(1))
            .expect("spawn kayak");
        engine
            .apply_object_update(crew, crate::ObjectUpdate::new().with_container(kayak))
            .expect("enter kayak");

        engine
            .player_in_com(1, COM_THROW, 0)
            .expect("run ContainedThrow");
        assert_eq!(
            engine
                .object_snapshot(crew)
                .expect("crew survives")
                .command_stack
                .command_names(),
            ["Activate"]
        );

        engine
            .execute_object_command_now(crew)
            .expect("execute Activate");

        assert!(engine
            .object_snapshot(crew)
            .expect("crew survives")
            .command_stack
            .is_empty());
        assert_eq!(
            engine.pending_menu_requests,
            [crate::MenuRequest {
                crew_id: crew,
                owner: 1,
                kind: crate::MenuRequestKind::ActivateTarget { container: kayak },
            }]
        );
    }

    #[test]
    fn activate_target_reject_contents_force_closes_the_prior_menu() {
        let mut engine = Engine::new();
        register_clonk(&mut engine, "CLNK", "#strict\n");
        engine
            .register_definition(
                Definition::from_script(
                    "REJT",
                    "Rejecting container",
                    "#strict 2\nprotected func RejectContents() { return true; }\n",
                )
                .expect("container compiles"),
            )
            .expect("register container");
        engine
            .register_player(PlayerConfig::new(1, "Test"))
            .expect("register player");
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        let container = engine
            .spawn_object(SpawnConfig::new("REJT"))
            .expect("spawn container");
        let crew_index = engine.find_object_index(crew).expect("crew exists");
        engine
            .open_context_menu(crew_index, crew_index, false)
            .expect("open prior menu");
        assert!(engine
            .debug_object_menu(crew.as_u64())
            .expect("crew exists")
            .is_some());
        engine.objects[crew_index].apply_command_operations([
            CommandOperation::PushFront(
                CommandRequest::new(CommandId::Activate).with_target2(Some(container)),
            ),
        ]);

        engine
            .execute_object_command_now(crew)
            .expect("execute rejected Activate");

        assert_eq!(engine.debug_object_menu(crew.as_u64()), Some(None));
        assert!(engine.pending_menu_requests.is_empty());
        assert!(engine
            .object_snapshot(crew)
            .expect("crew survives")
            .command_stack
            .is_empty());
    }

    #[test]
    fn contained_throw_puts_carried_item_into_container_immediately() {
        // ContainedControl executes C4CMD_Throw synchronously
        // (C4Object.cpp:3280-3282); C4Command::Throw delegates a contained
        // Clonk to ObjectComPutTake, which puts its first content into the
        // containing object (C4Command.cpp:966-970; C4ObjectCom.cpp:700-712).
        let mut engine = Engine::new();
        register_clonk(&mut engine, "CLNK", "#strict\n");
        engine
            .register_definition(
                Definition::from_script("HUT1", "Hut", "#strict\n").expect("hut compiles"),
            )
            .expect("register hut");
        engine
            .register_definition(
                Definition::from_script("FLAG", "Flag", "#strict\n").expect("flag compiles"),
            )
            .expect("register flag");
        engine
            .register_player(PlayerConfig::new(1, "Test"))
            .expect("player");
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        let hut = engine
            .spawn_object(SpawnConfig::new("HUT1"))
            .expect("spawn hut");
        let flag = engine
            .spawn_object(SpawnConfig::new("FLAG").with_container(crew))
            .expect("spawn carried flag");
        engine
            .apply_object_update(crew, crate::ObjectUpdate::new().with_container(hut))
            .expect("enter hut");

        engine.player_in_com(1, COM_THROW, 0).expect("throw");

        let flag = engine.object_snapshot(flag).expect("flag snapshot");
        assert_eq!(
            flag.container,
            Some(hut),
            "the carried flag is put into the hut before control returns"
        );
        assert!(
            engine
                .object_snapshot(crew)
                .expect("crew snapshot")
                .command_stack
                .is_empty(),
            "the synchronous Throw command has finished"
        );
    }

    /// Crew contained in a VehicleControl=Inside vehicle whose script is
    /// `vehicle_script`.
    fn inside_vehicle_fixture(engine: &mut Engine, vehicle_script: &str) -> (ObjectId, ObjectId) {
        register_clonk(engine, "CLNK", "#strict\n");
        let mut lorry =
            Definition::from_script("LORY", "Lorry", vehicle_script).expect("lorry compiles");
        lorry.set_vehicle_control(crate::VEHICLE_CONTROL_INSIDE);
        engine.register_definition(lorry).expect("register");
        engine
            .register_player(PlayerConfig::new(1, "Test"))
            .expect("player");
        let crew = spawn_crew(engine, "CLNK", 1);
        let lorry = engine
            .spawn_object(SpawnConfig::new("LORY"))
            .expect("spawn lorry");
        engine
            .apply_object_update(crew, crate::ObjectUpdate::new().with_container(lorry))
            .expect("enter lorry");
        (crew, lorry)
    }

    #[test]
    fn inside_vehicle_control_command_overloads_set_command() {
        // SetCommand's inside vehicle control overload (C4Object.cpp:
        // 3947-3961): a Contained def with C4D_VehicleControl_Inside gets
        // ControlCommand(name, target, tx, ty, target2, data, this) — the
        // CLONK rides in slot 7 — and a truthy return consumes the command.
        let vehicle = r#"
#strict
protected func ControlCommand(szCommand, pTarget, iTx, iTy, pTarget2, iData, pByObj) {
  if (pByObj) return(1);
  return(0);
}
"#;
        let mut engine = Engine::new();
        let (crew, lorry) = inside_vehicle_fixture(&mut engine, vehicle);

        engine.player_in_com(1, COM_DOWN, 0).expect("in_com");
        let snapshot = engine.object_snapshot(crew).expect("snapshot");
        assert!(
            snapshot.command_stack.command_names().is_empty(),
            "the vehicle's ControlCommand consumed the Exit SetCommand"
        );
        let lorry_index = engine.find_object_index(lorry).expect("lorry exists");
        assert_eq!(
            engine.objects[lorry_index].state.controller, 1,
            "Contained->Controller = Controller (C4Object.cpp:3950)"
        );
    }

    #[test]
    fn inside_vehicle_falsy_control_command_keeps_the_exit() {
        // A falsy overload falls through to the hardcoded push
        // (C4Object.cpp:3976-3977).
        let vehicle = r#"
#strict
protected func ControlCommand() { return(0); }
"#;
        let mut engine = Engine::new();
        let (crew, _) = inside_vehicle_fixture(&mut engine, vehicle);

        engine.player_in_com(1, COM_DOWN, 0).expect("in_com");
        let snapshot = engine.object_snapshot(crew).expect("snapshot");
        assert_eq!(snapshot.command_stack.command_names(), vec!["Exit"]);
    }

    #[test]
    fn outside_vehicle_control_command_overloads_pushed_set_command() {
        // The outside twin (C4Object.cpp:3962-3974): while pushing a
        // C4D_VehicleControl_Outside target, its ControlCommand (six args,
        // no clonk slot) may consume the Set command.
        let vehicle = r#"
#strict
protected func ControlCommand(szCommand) { return(1); }
"#;
        let mut engine = Engine::new();
        register_clonk(&mut engine, "CLNK", "#strict\n");
        let mut lorry =
            Definition::from_script("LORY", "Lorry", vehicle).expect("lorry compiles");
        lorry.set_vehicle_control(crate::VEHICLE_CONTROL_OUTSIDE);
        engine.register_definition(lorry).expect("register");
        engine
            .register_player(PlayerConfig::new(1, "Test"))
            .expect("player");
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        let lorry = engine
            .spawn_object(SpawnConfig::new("LORY"))
            .expect("spawn lorry");
        let crew_index = engine.find_object_index(crew).expect("crew exists");
        engine.objects[crew_index].state.action.name = "Push".to_string();
        engine.objects[crew_index].state.action.target = Some(lorry);

        engine
            .player_object_command(1, CommandId::Exit, None, 0, 0)
            .expect("exit command");
        let snapshot = engine.object_snapshot(crew).expect("snapshot");
        assert!(
            snapshot.command_stack.command_names().is_empty(),
            "the pushed vehicle's ControlCommand consumed the command"
        );
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
        assert_eq!(
            engine
                .debug_object_menu(crew.as_u64())
                .expect("crew exists")
                .expect("COM_Up opens a menu")
                .identification,
            Value::Int(4),
            "COM_Up activates C4MN_Buy on the clonk"
        );
        assert!(
            engine.pending_menu_requests.is_empty(),
            "C4Object::ActivateMenu is engine-owned, not an app-side request"
        );
        assert_eq!(engine.object_snapshot(crew).expect("crew").container, Some(hut));
    }

    #[test]
    fn contained_buy_menu_refills_from_the_base_players_material() {
        // C4Object::ActivateMenu(C4MN_Buy) creates a permanent menu on the
        // clonk (C4Object.cpp:1919-1930), and C4ObjectMenu::Refill adds the
        // base player's HomeBaseMaterial with its count, value and Buy
        // commands (C4ObjectMenu.cpp:207-237).
        let mut engine = Engine::new();
        let (crew, hut) = contained_base_fixture(&mut engine, 1);
        let hut_index = engine.find_object_index(hut).expect("hut exists");
        engine.objects[hut_index].state.owner = 7;
        let mut lorry =
            Definition::from_script("LORY", "Lorry", "#strict\n").expect("lorry compiles");
        lorry.set_value(25);
        lorry.set_description(Some("Carries cargo.".to_string()));
        engine.register_definition(lorry).expect("register lorry");
        engine
            .set_player_home_base_material(1, HashMap::from([("LORY".to_string(), 1)]))
            .expect("home-base material");

        engine.player_in_com(1, COM_UP, 0).expect("in_com");

        let menu = engine
            .debug_object_menu(crew.as_u64())
            .expect("crew exists")
            .expect("buy menu opens");
        assert_eq!(menu.identification, Value::Int(4), "C4MN_Buy");
        assert_eq!(
            menu.title_symbol,
            crate::ObjectMenuSymbol::Buy { owner: 7 },
            "C4Object::ActivateMenu composes C4MN_Buy with pTarget->Owner (C4Object.cpp:1919-1928; C4Menu.cpp:43-65)"
        );
        assert_eq!(
            menu.extra,
            crate::ObjectMenuExtra::Value,
            "C4MN_Buy enables C4MN_Extra_Value (C4Object.cpp:1926; C4Menu.cpp:843-907)"
        );
        assert!(menu.permanent);
        assert_eq!(menu.command_object, Some(crew));
        assert_eq!(menu.selection, 0);
        assert_eq!(menu.items.len(), 1);
        let item = &menu.items[0];
        assert_eq!(item.caption, "Buy Lorry");
        assert_eq!(item.count, 1);
        assert_eq!(item.item_id, "LORY");
        assert_eq!(item.value, Some(25));
        assert_eq!(item.info_caption, "Carries cargo.");
        assert_eq!(
            item.command,
            format!(
                "AppendCommand(this,\"Buy\",Object({}),1,0,,0,LORY)&&ExecuteCommand()",
                hut.as_u64()
            )
        );
        assert_eq!(item.command2, item.command);
    }

    #[test]
    fn contained_buy_menu_enter_purchases_and_refills() {
        // C4Player::InCom converts Throw to MenuEnter while a menu is open
        // (C4Player.cpp:1502-1513; C4Menu.cpp:1051-1057). The Buy row then
        // queues and executes C4CMD_Buy against Target->Base, consuming its
        // stock and the buyer's wealth (C4Command.cpp:2005-2035), while the
        // permanent menu refills (C4ObjectMenu.cpp:124-129,207-237).
        let mut engine = Engine::new();
        let (crew, hut) = contained_base_fixture(&mut engine, 1);
        let mut lorry =
            Definition::from_script("LORY", "Lorry", "#strict\n").expect("lorry compiles");
        lorry.set_value(25);
        engine.register_definition(lorry).expect("register lorry");
        engine.set_player_wealth(1, 25).expect("wealth");
        engine
            .set_player_home_base_material(1, HashMap::from([("LORY".to_string(), 1)]))
            .expect("home-base material");

        engine.player_in_com(1, COM_UP, 0).expect("open buy menu");
        engine
            .player_in_com(1, COM_THROW, 0)
            .expect("enter selected row");

        let player = engine.player(1).expect("player");
        assert_eq!(
            player.wealth(),
            0,
            "post-enter command stack: {:?}",
            engine
                .object_snapshot(crew)
                .expect("crew snapshot")
                .command_stack
                .command_names()
        );
        assert_eq!(
            player.home_base_material().get("LORY"),
            Some(&0),
            "C4Player::Buy leaves the C4IDList entry at zero"
        );
        let snapshot = engine.snapshot();
        let bought = snapshot
            .objects
            .iter()
            .find(|object| object.definition_id == "LORY" && object.status.is_active())
            .expect("bought lorry exists");
        assert_eq!(bought.owner, 1);
        assert_eq!(bought.container, Some(hut));
        let menu = engine
            .debug_object_menu(crew.as_u64())
            .expect("crew exists")
            .expect("permanent buy menu remains");
        assert_eq!(menu.identification, Value::Int(4));
        assert_eq!(menu.items.len(), 1, "zero-count IDs remain visible");
        assert_eq!(menu.items[0].item_id, "LORY");
        assert_eq!(menu.items[0].count, 0);
        assert_eq!(menu.selection, 0);
    }

    #[test]
    fn contained_buy_menu_refill_preserves_the_numeric_selection() {
        // C4ObjectMenu::DoRefillInternal uses ClearItems(false), so the Buy
        // menu keeps its numeric selection while stock is rebuilt. The outer
        // C4Menu::RefillInternal then only adjusts it if that slot stopped
        // being selectable (C4ObjectMenu.cpp:207-237; C4Menu.cpp:947-988,
        // 1014-1038).
        let mut engine = Engine::new();
        let (crew, _hut) = contained_base_fixture(&mut engine, 1);
        for (id, name) in [("FLAG", "Flag"), ("LORY", "Lorry")] {
            let mut definition =
                Definition::from_script(id, name, "#strict\n").expect("item compiles");
            definition.set_value(1);
            engine
                .register_definition(definition)
                .expect("register item");
        }
        engine.set_player_wealth(1, 2).expect("wealth");
        engine
            .set_player_home_base_material(
                1,
                HashMap::from([("FLAG".to_string(), 1), ("LORY".to_string(), 1)]),
            )
            .expect("home-base material");

        engine.player_in_com(1, COM_UP, 0).expect("open buy menu");
        engine
            .player_in_com(1, COM_RIGHT, 0)
            .expect("select second stock row");
        assert_eq!(
            engine
                .debug_object_menu(crew.as_u64())
                .expect("crew exists")
                .expect("buy menu remains open")
                .selection,
            1
        );

        engine
            .player_in_com(1, COM_THROW, 0)
            .expect("buy selected row and refill");

        let menu = engine
            .debug_object_menu(crew.as_u64())
            .expect("crew exists")
            .expect("permanent buy menu remains open");
        assert_eq!(menu.selection, 1);
        assert_eq!(menu.items[1].item_id, "LORY");
        assert_eq!(menu.items[1].count, 0);
    }

    #[test]
    fn player_execute_opens_the_contained_buildings_auto_context_menu() {
        // C4Player::Execute calls Cursor->AutoContextMenu after controls
        // (C4Player.cpp:206-212). A crew member inside an opted-in building
        // with the player's preference enabled gets a permanent C4MN_Context
        // menu populated in Contents/Buy/Sell/Exit order
        // (C4Object.cpp:2044-2062; C4ObjectMenu.cpp:328-435).
        let mut engine = Engine::new();
        register_clonk(&mut engine, "CLNK", "#strict\n");
        let mut hut =
            Definition::from_script("HUT3", "Hut", "#strict\n").expect("hut compiles");
        hut.set_category(crate::CATEGORY_STRUCTURE);
        hut.set_entrance_rect(Some(crate::DefinitionRect::new(-10, -10, 20, 20)));
        hut.set_auto_context_menu(true);
        engine.register_definition(hut).expect("register hut");
        engine
            .register_player(PlayerConfig::new(1, "Test"))
            .expect("player");
        engine.player_mut(1).expect("player").control.auto_context_menu = true;
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        let crew_index = engine.find_object_index(crew).expect("crew exists");
        engine.objects[crew_index].state.category = crate::CATEGORY_LIVING;
        let hut = engine
            .spawn_object(SpawnConfig::new("HUT3"))
            .expect("spawn hut");
        let hut_index = engine.find_object_index(hut).expect("hut exists");
        engine.objects[hut_index].state.base = 1;
        engine
            .apply_object_update(crew, crate::ObjectUpdate::new().with_container(hut))
            .expect("enter hut");

        engine.execute_player_controls().expect("player execute");

        let menu = engine
            .debug_object_menu(crew.as_u64())
            .expect("crew exists")
            .expect("context menu opens");
        assert_eq!(menu.identification, Value::Int(14), "C4MN_Context");
        assert_eq!(menu.style, 1, "C4MN_Style_Context");
        assert!(menu.permanent);
        assert!(!menu.user_menu);
        assert_eq!(menu.command_object, Some(crew));
        assert_eq!(menu.columns, 1);
        assert_eq!(
            menu.items
                .iter()
                .map(|item| item.caption.as_str())
                .collect::<Vec<_>>(),
            vec!["Contents", "Buy", "Sell", "Exit"]
        );
    }

    #[test]
    fn contained_context_runs_script_declared_context_function() {
        // C4MN_Context inserts target `Context*` functions between the base
        // rows and Info/Exit. Their leading description block supplies the
        // caption/image/condition, and Enter executes ProtectedCall on the
        // target (C4ObjectMenu.cpp:398-399,670-682;
        // C4AulParse.cpp:309-380). This is WRKS::ContextConstruction's real
        // Tutorial07 path, reduced to one deterministic menu callback.
        let mut engine = Engine::new();
        register_clonk(&mut engine, "CLNK", "#strict\n");
        let mut workshop = Definition::from_script(
            "WRKS",
            "Workshop",
            r#"
#strict 2
public func ContextConstruction(caller) {
    [Production|Image=CXCN|Condition=IsBuilt|Desc=Build a vehicle.]
    return CreateMenu(CXCN, caller, this(), 1, "No knowledge");
}
protected func IsBuilt() { return GetCon() >= 100; }
"#,
        )
        .expect("workshop compiles");
        workshop.set_category(crate::CATEGORY_STRUCTURE);
        workshop.set_entrance_rect(Some(crate::DefinitionRect::new(-10, -10, 20, 20)));
        workshop.set_auto_context_menu(true);
        engine
            .register_definition(workshop)
            .expect("register workshop");
        engine
            .register_player(PlayerConfig::new(1, "Test"))
            .expect("player");
        engine.player_mut(1).expect("player").control.auto_context_menu = true;
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        let workshop = engine
            .spawn_object(SpawnConfig::new("WRKS"))
            .expect("spawn workshop");
        engine
            .apply_object_update(
                crew,
                crate::ObjectUpdate::new().with_container(workshop),
            )
            .expect("enter workshop");

        engine.execute_player_controls().expect("open context menu");

        let menu = engine
            .debug_object_menu(crew.as_u64())
            .expect("crew exists")
            .expect("context menu opens");
        assert_eq!(
            menu.items
                .iter()
                .map(|item| item.caption.as_str())
                .collect::<Vec<_>>(),
            vec!["Contents", "Production", "Exit"]
        );
        engine
            .player_in_com(1, COM_RIGHT, 0)
            .expect("select Production");
        engine
            .player_in_com(1, COM_THROW, 0)
            .expect("execute ContextConstruction");
        assert_eq!(
            engine
                .debug_object_menu(crew.as_u64())
                .expect("crew exists")
                .expect("production menu opens")
                .identification,
            Value::C4Id("CXCN".to_owned())
        );
    }

    #[test]
    fn clonk_context_construction_emits_the_native_menu_request() {
        // Reduced shipped CLNK::ContextConstruction: its definition-less
        // SetCommand followed by synchronous ExecuteCommand opens
        // C4MN_Construction and finishes the command successfully
        // (Objects.c4d/Crew.c4d/Clonk.c4d/Script.c:628-634).
        let script = r#"
#strict 2
public func ContextConstruction(object caller)
{
    [Construction|Image=CXCN|Desc=Construct a building.]
    SetCommand(this(), "Construct");
    ExecuteCommand();
    return 1;
}
"#;
        let mut engine = Engine::new();
        let mut definition =
            Definition::from_script("CLNK", "Clonk", script).expect("clonk compiles");
        definition.configure_actions(Some("Walk".to_string()), clonk_actions());
        definition.set_movement_profile(MovementProfile::default());
        definition.set_physical(PhysicalInfo {
            can_construct: 1,
            ..Default::default()
        });
        engine
            .register_definition(definition)
            .expect("register clonk");
        engine
            .register_player(PlayerConfig::new(1, "Builder"))
            .expect("register player");
        let crew = spawn_crew(&mut engine, "CLNK", 1);

        assert!(engine
            .player_context_command(1, crew)
            .expect("queue context command"));
        engine
            .execute_object_command_now(crew)
            .expect("open context menu");
        let construction_index = engine
            .debug_object_menu(crew.as_u64())
            .expect("crew exists")
            .expect("context menu opens")
            .items
            .iter()
            .position(|item| item.command.contains("ContextConstruction"))
            .expect("construction row") as i32;
        for _ in 0..construction_index {
            engine
                .player_in_com(1, COM_RIGHT, 0)
                .expect("select construction row");
        }

        engine
            .player_in_com(1, COM_THROW, 0)
            .expect("execute ContextConstruction");

        assert_eq!(engine.debug_object_menu(crew.as_u64()), Some(None));
        assert!(engine
            .object_snapshot(crew)
            .expect("crew survives")
            .command_stack
            .command_names()
            .is_empty());
        assert_eq!(
            engine.pending_menu_requests,
            [crate::MenuRequest {
                crew_id: crew,
                owner: 1,
                kind: crate::MenuRequestKind::Construction,
            }]
        );
    }

    #[test]
    fn mouse_context_command_targets_self_and_opens_classic_nonpermanent_menu() {
        // C4MouseControl passes the clicked object as Target2 with Add mode;
        // self-targeting must not exclude the cursor as ordinary Target does.
        // C4Command::Context then installs non-permanent C4MN_Context
        // (C4MouseControl.cpp:1253-1260; C4Command.cpp:1076-1090).
        let mut engine = Engine::new();
        register_clonk(
            &mut engine,
            "MCLK",
            r#"
#strict 2
public func ContextMagic(object caller)
{
    [Magic|Image=MCMS|Desc=Open the spell menu.]
    return 1;
}
"#,
        );
        engine
            .register_player(PlayerConfig::new(1, "Mage"))
            .expect("player");
        let mage = spawn_crew(&mut engine, "MCLK", 1);

        assert!(engine
            .player_context_command(1, mage)
            .expect("queue mouse context command"));
        assert_eq!(
            engine
                .object_snapshot(mage)
                .expect("mage snapshot")
                .command_stack
                .command_names(),
            ["Context"]
        );
        engine
            .execute_object_command_now(mage)
            .expect("execute context command");
        assert!(
            engine.pending_menu_requests.is_empty(),
            "context requests must be consumed into the native menu: {:?}",
            engine.pending_menu_requests
        );

        let menu = engine
            .debug_object_menu(mage.as_u64())
            .expect("mage exists")
            .expect("classic context menu opens");
        assert_eq!(menu.identification, Value::Int(14));
        assert_eq!(menu.style, 1);
        assert!(!menu.permanent);
        assert_eq!(menu.command_object, Some(mage));
        assert!(menu.items.iter().any(|item| {
            item.caption == "Magic"
                && item.command.contains("ContextMagic")
                && item.command.contains(&mage.as_u64().to_string())
        }));
    }

    #[test]
    fn auto_context_put_row_deposits_the_first_carried_object() {
        // C4MN_Context starts with Put when the command object is carrying
        // something inside a container (C4ObjectMenu.cpp:335-359). Because
        // it is the first selected row, Throw enters it and immediately
        // executes the Put command on the contained Clonk.
        let mut engine = Engine::new();
        register_clonk(&mut engine, "CLNK", "#strict\n");
        let mut hut =
            Definition::from_script("HUT2", "Hut", "#strict\n").expect("hut compiles");
        hut.set_category(crate::CATEGORY_STRUCTURE);
        hut.set_entrance_rect(Some(crate::DefinitionRect::new(-10, -10, 20, 20)));
        hut.set_auto_context_menu(true);
        engine.register_definition(hut).expect("register hut");
        engine
            .register_definition(
                Definition::from_script("FLAG", "Flag", "#strict\n")
                    .expect("flag compiles"),
            )
            .expect("register flag");
        engine
            .register_player(PlayerConfig::new(1, "Test"))
            .expect("player");
        engine.player_mut(1).expect("player").control.auto_context_menu = true;
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        let hut = engine
            .spawn_object(SpawnConfig::new("HUT2"))
            .expect("spawn hut");
        let flag = engine
            .spawn_object(SpawnConfig::new("FLAG").with_container(crew))
            .expect("spawn carried flag");
        engine
            .apply_object_update(crew, crate::ObjectUpdate::new().with_container(hut))
            .expect("enter hut");

        engine.execute_player_controls().expect("player execute");

        let menu = engine
            .debug_object_menu(crew.as_u64())
            .expect("crew exists")
            .expect("context menu opens");
        assert_eq!(menu.selection, 0);
        assert_eq!(menu.items.first().map(|item| item.caption.as_str()), Some("Put"));
        assert_eq!(menu.items[0].symbol, crate::ObjectMenuSymbol::Put);
        assert_eq!(
            menu.items[0].command,
            format!(
                "PlayerObjectCommand(1, \"Put\", Object({}), 0, 0) && ExecuteCommand()",
                hut.as_u64()
            )
        );

        engine
            .player_in_com(1, COM_THROW, 0)
            .expect("enter selected Put row");

        assert_eq!(
            engine.object_snapshot(flag).expect("flag snapshot").container,
            Some(hut),
            "the selected Put row deposits the carried flag"
        );

        let second_flag = engine
            .spawn_object(SpawnConfig::new("FLAG").with_container(crew))
            .expect("spawn second carried flag");
        let third_flag = engine
            .spawn_object(SpawnConfig::new("FLAG").with_container(crew))
            .expect("spawn third carried flag");
        engine
            .tick()
            .expect("complete primary Put and reopen context menu");
        let menu = engine
            .debug_object_menu(crew.as_u64())
            .expect("crew exists")
            .expect("context menu reopens");
        assert_eq!(
            menu.items[0].command2,
            format!(
                "PlayerObjectCommand(1, \"Put\", Object({}), 1000, 0) && ExecuteCommand()",
                hut.as_u64()
            )
        );

        engine
            .player_in_com(1, COM_SPECIAL2, 0)
            .expect("enter Put-all command");
        let command = engine
            .object_snapshot(crew)
            .expect("crew snapshot")
            .command_stack
            .command_views()
            .into_iter()
            .next()
            .expect("Put-all remains active after its first transfer");
        assert_eq!(command.name, "Put");
        assert_eq!(command.tx, Some(2));
        let resolved_item = command
            .target2
            .expect("C++ Put resolves its live Target2 on execute");
        assert!(
            [second_flag, third_flag].contains(&resolved_item),
            "Put-all resolves one of the carried flags"
        );
        engine.tick().expect("advance Put count");
        engine.tick().expect("put the remaining carried object");
        engine.tick().expect("complete Put-all command");

        for flag in [second_flag, third_flag] {
            assert_eq!(
                engine.object_snapshot(flag).expect("flag snapshot").container,
                Some(hut),
                "Put-all deposits every carried object"
            );
        }
        assert!(
            engine
                .object_snapshot(crew)
                .expect("crew snapshot")
                .command_stack
                .is_empty(),
            "Put-all finishes after observing the final item in the target"
        );
    }

    #[test]
    fn selecting_auto_context_exit_row_exits_the_building() {
        // C4MN_Context's Exit row runs PlayerObjectCommand("Exit") and
        // ExecuteCommand on the menu object (C4ObjectMenu.cpp:426-433).
        let mut engine = Engine::new();
        register_clonk(&mut engine, "CLNK", "#strict\n");
        let mut hut =
            Definition::from_script("HUT3", "Hut", "#strict\n").expect("hut compiles");
        hut.set_category(crate::CATEGORY_STRUCTURE);
        hut.set_entrance_rect(Some(crate::DefinitionRect::new(-10, -10, 20, 20)));
        hut.set_auto_context_menu(true);
        engine.register_definition(hut).expect("register hut");
        engine
            .register_player(PlayerConfig::new(1, "Test"))
            .expect("player");
        engine.player_mut(1).expect("player").control.auto_context_menu = true;
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        let hut = engine
            .spawn_object(SpawnConfig::new("HUT3"))
            .expect("spawn hut");
        let hut_index = engine.find_object_index(hut).expect("hut exists");
        engine.objects[hut_index].state.base = 1;
        // The context Exit row is immediate only while the door is open;
        // otherwise C++ first asks ActivateEntrance (C4Command.cpp:624-665).
        engine.objects[hut_index].state.entrance_status = true;
        engine
            .apply_object_update(crew, crate::ObjectUpdate::new().with_container(hut))
            .expect("enter hut");
        engine.execute_player_controls().expect("open context menu");

        for _ in 0..3 {
            engine.player_in_com(1, COM_RIGHT, 0).expect("navigate");
        }
        let menu = engine
            .debug_object_menu(crew.as_u64())
            .expect("crew exists")
            .expect("context menu remains open");
        assert_eq!(menu.items[menu.selection as usize].caption, "Exit");

        engine
            .player_in_com(1, COM_THROW, 0)
            .expect("enter Exit row");

        assert_eq!(
            engine.object_snapshot(crew).expect("crew survives").container,
            None,
            "the selected Exit row executes the real Exit command"
        );
        assert_eq!(engine.debug_object_menu(crew.as_u64()), Some(None));
    }

    #[test]
    fn contained_context_buy_entry_opens_the_buy_menu() {
        // The C4MN_Context Buy row runs a data-less C4CMD_Buy, which opens
        // C4MN_Buy on its Target before succeeding (C4ObjectMenu.cpp:
        // 376-387; C4Command.cpp:1987-2004). Menu controls are converted
        // ahead of gameplay by C4Player::InCom (C4Player.cpp:1502-1513).
        let mut engine = Engine::new();
        register_clonk(&mut engine, "CLNK", "#strict\n");
        let mut hut =
            Definition::from_script("HUT3", "Hut", "#strict\n").expect("hut compiles");
        hut.set_category(crate::CATEGORY_STRUCTURE);
        hut.set_entrance_rect(Some(crate::DefinitionRect::new(-10, -10, 20, 20)));
        hut.set_auto_context_menu(true);
        engine.register_definition(hut).expect("register hut");
        let mut lorry =
            Definition::from_script("LORY", "Lorry", "#strict\n").expect("lorry compiles");
        lorry.set_value(25);
        engine.register_definition(lorry).expect("register lorry");
        engine
            .register_player(PlayerConfig::new(1, "Test"))
            .expect("player");
        engine.player_mut(1).expect("player").control.auto_context_menu = true;
        engine
            .set_player_home_base_material(1, HashMap::from([("LORY".to_string(), 1)]))
            .expect("home-base material");
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        let hut = engine
            .spawn_object(SpawnConfig::new("HUT3"))
            .expect("spawn hut");
        let hut_index = engine.find_object_index(hut).expect("hut exists");
        engine.objects[hut_index].state.base = 1;
        engine
            .apply_object_update(crew, crate::ObjectUpdate::new().with_container(hut))
            .expect("enter hut");
        engine.execute_player_controls().expect("player execute");

        engine.player_in_com(1, COM_RIGHT, 0).expect("select Buy");
        engine.player_in_com(1, COM_THROW, 0).expect("enter Buy");

        let menu = engine
            .debug_object_menu(crew.as_u64())
            .expect("crew exists")
            .expect("buy menu opens");
        assert_eq!(menu.identification, Value::Int(4), "C4MN_Buy");
        assert_eq!(menu.items.len(), 1);
        assert_eq!(menu.items[0].item_id, "LORY");
    }

    #[test]
    fn contained_context_info_entry_opens_the_info_menu() {
        // The Context Info row executes ShowInfo(target), which calls
        // ActivateMenu(C4MN_Info) on the command object and adds the
        // target's info string (C4ObjectMenu.cpp:410-423;
        // C4Script.cpp:3332-3336; C4Object.cpp:2008-2027).
        let mut engine = Engine::new();
        register_clonk(&mut engine, "CLNK", "#strict\n");
        let mut hut =
            Definition::from_script("HUT3", "Hut", "#strict\n").expect("hut compiles");
        hut.set_category(crate::CATEGORY_STRUCTURE);
        hut.set_entrance_rect(Some(crate::DefinitionRect::new(-10, -10, 20, 20)));
        hut.set_auto_context_menu(true);
        hut.set_description(Some("A sturdy wooden hut.".to_string()));
        engine.register_definition(hut).expect("register hut");
        engine
            .register_player(PlayerConfig::new(1, "Test"))
            .expect("player");
        engine.player_mut(1).expect("player").control.auto_context_menu = true;
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        let hut = engine
            .spawn_object(SpawnConfig::new("HUT3"))
            .expect("spawn hut");
        let hut_index = engine.find_object_index(hut).expect("hut exists");
        engine.objects[hut_index].state.base = 1;
        engine
            .apply_object_update(crew, crate::ObjectUpdate::new().with_container(hut))
            .expect("enter hut");
        engine.execute_player_controls().expect("player execute");
        for _ in 0..3 {
            engine.player_in_com(1, COM_RIGHT, 0).expect("navigate");
        }

        engine.player_in_com(1, COM_THROW, 0).expect("open Info");

        let menu = engine
            .debug_object_menu(crew.as_u64())
            .expect("crew exists")
            .expect("Info menu opens");
        assert_eq!(menu.identification, Value::Int(15), "C4MN_Info");
        assert_eq!(menu.style, 2, "C4MN_Style_Info");
        assert!(menu.permanent);
        assert_eq!(menu.title_symbol, crate::ObjectMenuSymbol::InfoTitle);
        assert_eq!(menu.selection, 0);
        assert_eq!(menu.items.len(), 1);
        assert_eq!(menu.items[0].caption, "Hut");
        assert_eq!(menu.items[0].info_caption, "A sturdy wooden hut.");
        assert!(menu.items[0].selectable);
        assert_eq!(menu.items[0].picture_object, Some(hut));
    }

    #[test]
    fn object_info_menu_appends_script_and_native_effect_info_in_list_order() {
        let mut engine = Engine::new();
        register_clonk(&mut engine, "CLNK", "#strict\n");
        let mut target = Definition::from_script(
            "TARG",
            "Target",
            "#strict\nfunc FxGlowInfo(object target, int number) { return \"Glowing.\"; }\n",
        )
        .expect("target compiles");
        target.set_description(Some("Base description.".to_string()));
        engine.register_definition(target).expect("register target");
        engine
            .register_player(PlayerConfig::new(1, "Test"))
            .expect("player");
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        let target = engine
            .spawn_object(SpawnConfig::new("TARG"))
            .expect("spawn target");
        let target_index = engine.find_object_index(target).expect("target exists");
        let mut glow = crate::EffectState::new("Glow");
        glow.number = 7;
        glow.command_target = Some(target.as_u64() as i32);
        let mut fire = crate::EffectState::new(crate::C4FX_FIRE);
        fire.number = 8;
        fire.command_target = Some(target.as_u64() as i32);
        engine.objects[target_index].state.effects = vec![glow, fire];

        let crew_index = engine.find_object_index(crew).expect("crew exists");
        engine
            .open_object_info_menu(crew_index, target_index)
            .expect("Info opens");

        let menu = engine
            .debug_object_menu(crew.as_u64())
            .expect("crew exists")
            .expect("Info menu exists");
        assert_eq!(
            menu.items[0].info_caption,
            "Base description.|Glowing.|{{FLAM}} The object burns."
        );
    }

    #[test]
    fn menu_info_caption_matches_cpp_buffer_and_line_normalization() {
        let source = format!("A\nB\rC{}", "x".repeat(600));
        let normalized = crate::normalize_menu_info_caption(source);
        assert_eq!(normalized.len(), 512);
        assert!(normalized.starts_with("A B|C"));
    }

    #[test]
    fn contained_context_contents_entry_opens_the_contents_menu() {
        // The C4MN_Context Contents row runs C4CMD_Get with Data=2,
        // which immediately activates C4MN_Contents on the target
        // (C4ObjectMenu.cpp:361-373; C4Command.cpp:1129-1135).
        let mut engine = Engine::new();
        register_clonk(&mut engine, "CLNK", "#strict\n");
        let mut hut =
            Definition::from_script("HUT3", "Hut", "#strict\n").expect("hut compiles");
        hut.set_category(crate::CATEGORY_STRUCTURE);
        hut.set_entrance_rect(Some(crate::DefinitionRect::new(-10, -10, 20, 20)));
        hut.set_auto_context_menu(true);
        engine.register_definition(hut).expect("register hut");
        let mut lorry =
            Definition::from_script("LORY", "Lorry", "#strict\n").expect("lorry compiles");
        lorry.set_category(crate::CATEGORY_VEHICLE);
        lorry.set_description(Some("Carries cargo.".to_string()));
        engine.register_definition(lorry).expect("register lorry");
        engine
            .register_player(PlayerConfig::new(1, "Test"))
            .expect("player");
        engine.player_mut(1).expect("player").control.auto_context_menu = true;
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        let crew_index = engine.find_object_index(crew).expect("crew exists");
        engine.objects[crew_index].state.category = crate::CATEGORY_LIVING;
        let hut = engine
            .spawn_object(SpawnConfig::new("HUT3"))
            .expect("spawn hut");
        let hut_index = engine.find_object_index(hut).expect("hut exists");
        engine.objects[hut_index].state.base = 1;
        // This assertion exercises C4CMD_Exit's open-door branch. C4Object
        // initializes EntranceStatus to false; HUT3's DOOR script opens it
        // before an object can leave (C4Object.cpp:116;
        // C4Command.cpp:624-650).
        engine.objects[hut_index].state.entrance_status = true;
        let lorry = engine
            .spawn_object(SpawnConfig::new("LORY").with_container(hut))
            .expect("spawn contained lorry");
        engine
            .apply_object_update(crew, crate::ObjectUpdate::new().with_container(hut))
            .expect("enter hut");
        engine.execute_player_controls().expect("player execute");

        engine
            .player_in_com(1, COM_THROW, 0)
            .expect("enter Contents");

        let menu = engine
            .debug_object_menu(crew.as_u64())
            .expect("crew exists")
            .expect("contents menu opens");
        assert_eq!(menu.identification, Value::Int(18), "C4MN_Contents");
        assert_eq!(
            menu.items.len(),
            1,
            "contents rows: {:?}",
            menu.items
                .iter()
                .map(|item| item.item_id.as_str())
                .collect::<Vec<_>>()
        );
        assert_eq!(menu.items[0].item_id, "LORY");
        assert_eq!(menu.items[0].info_caption, "Carries cargo.");
        assert!(
            menu.items[0]
                .command
                .contains(&format!("\"Activate\", Object({})", lorry.as_u64())),
            "non-carryable vehicles activate out of the base"
        );

        engine
            .player_in_com(1, COM_THROW, 0)
            .expect("activate the selected lorry");
        let lorry_after_activate = engine.object_snapshot(lorry).expect("lorry survives");
        assert_eq!(
            lorry_after_activate.container,
            Some(hut),
            "C4CMD_Activate arms the target's Exit command before it runs"
        );
        assert_eq!(
            lorry_after_activate.command_stack.command_names(),
            vec!["Exit"]
        );
        engine.tick().expect("target Exit command frame");
        assert_eq!(
            engine.object_snapshot(lorry).expect("lorry survives").container,
            None,
            "the vehicle exits on its own object execution"
        );
        let menu = engine
            .debug_object_menu(crew.as_u64())
            .expect("crew exists")
            .expect("permanent contents menu refills");
        assert_eq!(menu.identification, Value::Int(18));
        assert!(menu.items.is_empty());
    }

    #[test]
    fn contents_refill_preserves_the_selected_definition() {
        // C4ObjectMenu::Refill stores the selected item's C4ID and
        // checkIDSelection restores it after rebuilding the rows
        // (C4ObjectMenu.cpp:274,325,448-458).
        let mut engine = Engine::new();
        register_clonk(&mut engine, "CLNK", "#strict\n");
        let mut hut =
            Definition::from_script("HUT3", "Hut", "#strict\n").expect("hut compiles");
        hut.set_category(crate::CATEGORY_STRUCTURE);
        hut.set_entrance_rect(Some(crate::DefinitionRect::new(-10, -10, 20, 20)));
        hut.set_auto_context_menu(true);
        engine.register_definition(hut).expect("register hut");
        for (id, name) in [("LORY", "Lorry"), ("FLAG", "Flag")] {
            let mut definition =
                Definition::from_script(id, name, "#strict\n").expect("item compiles");
            definition.set_category(crate::CATEGORY_VEHICLE);
            engine
                .register_definition(definition)
                .expect("register item");
        }
        engine
            .register_player(PlayerConfig::new(1, "Test"))
            .expect("player");
        engine.player_mut(1).expect("player").control.auto_context_menu = true;
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        let hut = engine
            .spawn_object(SpawnConfig::new("HUT3"))
            .expect("spawn hut");
        let hut_index = engine.find_object_index(hut).expect("hut exists");
        engine.objects[hut_index].state.base = 1;
        engine
            .spawn_object(SpawnConfig::new("LORY").with_container(hut))
            .expect("spawn lorry");
        engine
            .spawn_object(SpawnConfig::new("FLAG").with_container(hut))
            .expect("spawn flag");
        engine
            .apply_object_update(crew, crate::ObjectUpdate::new().with_container(hut))
            .expect("enter hut");
        engine.execute_player_controls().expect("player execute");
        engine
            .player_in_com(1, COM_THROW, 0)
            .expect("open Contents");
        engine.player_in_com(1, COM_RIGHT, 0).expect("select second");
        let selected_definition = engine
            .debug_object_menu(crew.as_u64())
            .expect("crew exists")
            .expect("contents menu is open")
            .items[1]
            .item_id
            .clone();

        engine.execute_player_controls().expect("refill frame");

        let menu = engine
            .debug_object_menu(crew.as_u64())
            .expect("crew exists")
            .expect("contents menu remains open");
        assert_eq!(menu.selection, 1);
        assert_eq!(menu.items[1].item_id, selected_definition);
    }

    #[test]
    fn contained_context_sell_entry_opens_the_grouped_sell_menu() {
        // The C4MN_Context Sell row runs a data-less C4CMD_Sell and opens
        // C4MN_Sell. Refill walks the base's stContents order, groups
        // equal definitions, and carries both preferred-object and bulk
        // commands (C4ObjectMenu.cpp:238-277; C4Command.cpp:2040-2057).
        let mut engine = Engine::new();
        register_clonk(&mut engine, "CLNK", "#strict\n");
        let mut hut =
            Definition::from_script("HUT3", "Hut", "#strict\n").expect("hut compiles");
        hut.set_category(crate::CATEGORY_STRUCTURE);
        hut.set_entrance_rect(Some(crate::DefinitionRect::new(-10, -10, 20, 20)));
        hut.set_auto_context_menu(true);
        engine.register_definition(hut).expect("register hut");
        let mut flag =
            Definition::from_script("FLAG", "Flag", "#strict\n").expect("flag compiles");
        flag.set_category(crate::CATEGORY_OBJECT);
        flag.set_value(100);
        flag.set_description(Some("Marks a base.".to_string()));
        engine.register_definition(flag).expect("register flag");
        let mut lorry =
            Definition::from_script("LORY", "Lorry", "#strict\n").expect("lorry compiles");
        lorry.set_category(crate::CATEGORY_VEHICLE);
        lorry.set_value(20);
        lorry.set_description(Some("Carries cargo.".to_string()));
        engine.register_definition(lorry).expect("register lorry");
        engine
            .register_player(PlayerConfig::new(1, "Test"))
            .expect("player");
        engine.player_mut(1).expect("player").control.auto_context_menu = true;
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        let crew_index = engine.find_object_index(crew).expect("crew exists");
        engine.objects[crew_index].state.category = crate::CATEGORY_LIVING;
        let hut = engine
            .spawn_object(SpawnConfig::new("HUT3"))
            .expect("spawn hut");
        let hut_index = engine.find_object_index(hut).expect("hut exists");
        engine.objects[hut_index].state.owner = 8;
        engine.objects[hut_index].state.base = 1;
        let first_flag = engine
            .spawn_object(SpawnConfig::new("FLAG").with_container(hut))
            .expect("spawn first flag");
        let second_flag = engine
            .spawn_object(SpawnConfig::new("FLAG").with_container(hut))
            .expect("spawn second flag");
        let lorry = engine
            .spawn_object(SpawnConfig::new("LORY").with_container(hut))
            .expect("spawn lorry");
        engine
            .apply_object_update(crew, crate::ObjectUpdate::new().with_container(hut))
            .expect("enter hut");
        engine.execute_player_controls().expect("player execute");

        engine.player_in_com(1, COM_RIGHT, 0).expect("select Buy");
        engine.player_in_com(1, COM_RIGHT, 0).expect("select Sell");
        engine.player_in_com(1, COM_THROW, 0).expect("enter Sell");

        let menu = engine
            .debug_object_menu(crew.as_u64())
            .expect("crew exists")
            .expect("sell menu opens");
        assert_eq!(menu.identification, Value::Int(5), "C4MN_Sell");
        assert_eq!(
            menu.title_symbol,
            crate::ObjectMenuSymbol::Sell { owner: 8 },
            "C4Object::ActivateMenu composes C4MN_Sell with pTarget->Owner (C4Object.cpp:1932-1941; C4Menu.cpp:43-70)"
        );
        assert_eq!(
            menu.extra,
            crate::ObjectMenuExtra::Value,
            "C4MN_Sell enables C4MN_Extra_Value (C4Object.cpp:1938; C4Menu.cpp:843-907)"
        );
        assert_eq!(
            menu.items
                .iter()
                .map(|item| (item.item_id.as_str(), item.count, item.value))
                .collect::<Vec<_>>(),
            vec![("FLAG", 2, Some(100)), ("LORY", 1, Some(20))]
        );
        assert_eq!(
            menu.items
                .iter()
                .map(|item| item.info_caption.as_str())
                .collect::<Vec<_>>(),
            vec!["Marks a base.", "Carries cargo."]
        );
        assert!(menu.items[0].command.contains(&format!(
            "Object({})",
            second_flag.as_u64()
        )) || menu.items[0]
            .command
            .contains(&format!("Object({})", first_flag.as_u64())));
        assert!(menu.items[0].command2.contains(",2,0,,0,FLAG"));
        assert!(menu.items[1]
            .command
            .contains(&format!("Object({})", lorry.as_u64())));

        engine
            .player_in_com(1, COM_THROW, 0)
            .expect("sell the selected flag");
        assert_eq!(engine.player(1).expect("player").wealth(), 100);
        assert_eq!(
            engine
                .player(1)
                .expect("player")
                .home_base_material()
                .get("FLAG"),
            Some(&1)
        );
        let menu = engine
            .debug_object_menu(crew.as_u64())
            .expect("crew exists")
            .expect("permanent sell menu refills");
        assert_eq!(menu.identification, Value::Int(5));
        assert_eq!(
            menu.items
                .iter()
                .map(|item| (item.item_id.as_str(), item.count))
                .collect::<Vec<_>>(),
            vec![("FLAG", 1), ("LORY", 1)]
        );
    }

    #[test]
    fn contents_and_sell_refills_group_only_cpp_concatable_pictures() {
        // C4MN_Sell and C4MN_Contents both enumerate the target's stContents
        // through C4ObjectListIterator (C4ObjectMenu.cpp:238-275,279-326).
        // That iterator emits a separate row for same-ID objects unless
        // C4Object::CanConcatPictureWith succeeds (C4ObjectList.cpp:849-903;
        // C4Object.cpp:6173-6213). The row count is the concat group count,
        // while command2 deliberately keeps Contents.ObjectCount(id), i.e. the
        // count of every same-ID object (C4ObjectMenu.cpp:266-271,317-321).
        let mut engine = Engine::new();
        register_clonk(&mut engine, "CLNK", "#strict\n");
        let mut hut =
            Definition::from_script("HUT3", "Hut", "#strict\n").expect("hut compiles");
        hut.set_category(crate::CATEGORY_STRUCTURE);
        hut.set_entrance_rect(Some(crate::DefinitionRect::new(-10, -10, 20, 20)));
        engine.register_definition(hut).expect("register hut");
        let mut flint = Definition::from_script("TFLN", "T-Flint", "#strict\n")
            .expect("flint compiles");
        flint.set_category(crate::CATEGORY_OBJECT);
        flint.set_collectible(true);
        flint.set_value(15);
        engine.register_definition(flint).expect("register flint");
        engine
            .register_player(PlayerConfig::new(1, "Test"))
            .expect("player");
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        let crew_index = engine.find_object_index(crew).expect("crew exists");
        let hut = engine
            .spawn_object(SpawnConfig::new("HUT3"))
            .expect("spawn hut");
        let hut_index = engine.find_object_index(hut).expect("hut exists");
        engine.objects[hut_index].state.base = 1;
        let idle = engine
            .spawn_object(SpawnConfig::new("TFLN").with_container(hut))
            .expect("spawn idle flint");
        let activated = engine
            .spawn_object(SpawnConfig::new("TFLN").with_container(hut))
            .expect("spawn activated flint");
        engine
            .apply_object_update(
                activated,
                crate::ObjectUpdate {
                    picture_rect: Some(crate::DefinitionRect::new(0, 76, 64, 64)),
                    ..crate::ObjectUpdate::default()
                },
            )
            .expect("activated flint changes its picture");
        assert!(!engine.can_concat_picture_with(
            &engine.object_snapshot(idle).expect("idle flint exists"),
            &engine
                .object_snapshot(activated)
                .expect("activated flint exists"),
        ));

        engine
            .open_base_sell_menu(crew_index, hut_index)
            .expect("open sell menu");
        let sell = engine
            .debug_object_menu(crew.as_u64())
            .expect("crew exists")
            .expect("sell menu opens");
        assert_eq!(
            sell.items
                .iter()
                .map(|item| (item.item_id.as_str(), item.count))
                .collect::<Vec<_>>(),
            vec![("TFLN", 1), ("TFLN", 1)],
            "different per-object pictures occupy separate C++ menu rows"
        );
        assert!(sell
            .items
            .iter()
            .all(|item| item.command2.contains(",2,0,,0,TFLN")));
        let ordered_flints = engine.object_snapshot(hut).expect("hut exists").contents;
        assert_eq!(ordered_flints.len(), 2);
        assert_eq!(
            sell.items
                .iter()
                .map(|item| item.picture_object)
                .collect::<Vec<_>>(),
            ordered_flints.iter().copied().map(Some).collect::<Vec<_>>(),
            "C4ObjectMenu draws each row from the representative returned by C4ObjectListIterator (C4ObjectMenu.cpp:246-264; C4ObjectList.cpp:849-903)"
        );
        for (row, object) in sell.items.iter().zip(&ordered_flints) {
            assert!(row
                .command
                .contains(&format!("Object({})", object.as_u64())));
        }
        engine
            .player_in_com(1, COM_RIGHT, 0)
            .expect("select second picture row");
        engine
            .open_base_sell_menu(crew_index, hut_index)
            .expect("refill sell menu");
        assert_eq!(
            engine
                .debug_object_menu(crew.as_u64())
                .expect("crew exists")
                .expect("sell menu remains open")
                .selection,
            1,
            "same-ID picture rows keep C++'s surviving numeric selection"
        );

        engine
            .open_container_contents_menu(crew_index, hut_index, 18)
            .expect("open contents menu");
        let contents = engine
            .debug_object_menu(crew.as_u64())
            .expect("crew exists")
            .expect("contents menu opens");
        assert_eq!(
            contents
                .items
                .iter()
                .map(|item| (item.item_id.as_str(), item.count))
                .collect::<Vec<_>>(),
            vec![("TFLN", 1), ("TFLN", 1)]
        );
        assert_eq!(
            contents
                .items
                .iter()
                .map(|item| item.picture_object)
                .collect::<Vec<_>>(),
            ordered_flints.iter().copied().map(Some).collect::<Vec<_>>(),
            "C4ObjectMenu calls Picture2Facet on each Get/Contents representative (C4ObjectMenu.cpp:286-313)"
        );
        assert!(contents.items.iter().all(|item| {
            item.command2.contains(&format!(
                "SetCommand(this, \"Get\", , 2,0, Object({}), TFLN)",
                hut.as_u64()
            ))
        }));
        engine
            .player_in_com(1, COM_RIGHT, 0)
            .expect("select second contents picture row");
        engine
            .open_container_contents_menu(crew_index, hut_index, 18)
            .expect("refill contents menu");
        assert_eq!(
            engine
                .debug_object_menu(crew.as_u64())
                .expect("crew exists")
                .expect("contents menu remains open")
                .selection,
            1
        );
    }

    #[test]
    fn sell_refill_prefers_a_full_construction_picture_representative() {
        // After C4ObjectListIterator fixes the row count, C4ObjectMenu replaces
        // an incomplete representative with the first full-construction object
        // only when their pictures concatenate. The replacement supplies both
        // Picture2Facet and the primary Sell command target; the count remains
        // the original concat-group count (C4ObjectMenu.cpp:246-271).
        let mut engine = Engine::new();
        register_clonk(&mut engine, "CLNK", "#strict\n");
        let mut hut =
            Definition::from_script("HUT3", "Hut", "#strict\n").expect("hut compiles");
        hut.set_category(crate::CATEGORY_STRUCTURE);
        engine.register_definition(hut).expect("register hut");
        let mut flint = Definition::from_script("TFLN", "T-Flint", "#strict\n")
            .expect("flint compiles");
        flint.set_category(crate::CATEGORY_OBJECT);
        flint.set_value(15);
        engine.register_definition(flint).expect("register flint");
        engine
            .register_player(PlayerConfig::new(1, "Test"))
            .expect("player");
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        let crew_index = engine.find_object_index(crew).expect("crew exists");
        let hut = engine
            .spawn_object(SpawnConfig::new("HUT3"))
            .expect("spawn hut");
        let hut_index = engine.find_object_index(hut).expect("hut exists");
        engine.objects[hut_index].state.base = 1;
        let full = engine
            .spawn_object(SpawnConfig::new("TFLN").with_container(hut))
            .expect("spawn full flint");
        let incomplete = engine
            .spawn_object(
                SpawnConfig::new("TFLN")
                    .with_construction(crate::FULL_CON / 2)
                    .with_container(hut),
            )
            .expect("spawn incomplete flint");
        assert_eq!(
            engine.object_snapshot(hut).expect("hut exists").contents,
            vec![incomplete, full],
            "the incomplete object is the iterator's initial representative"
        );

        engine
            .open_base_sell_menu(crew_index, hut_index)
            .expect("open sell menu");
        let menu = engine
            .debug_object_menu(crew.as_u64())
            .expect("crew exists")
            .expect("sell menu opens");
        assert_eq!(menu.items.len(), 1);
        assert_eq!(menu.items[0].count, 2);
        assert_eq!(menu.items[0].picture_object, Some(full));
        assert!(menu.items[0]
            .command
            .contains(&format!("Object({})", full.as_u64())));
    }

    #[test]
    fn sell_refill_preserves_the_selected_definition_and_numeric_fallback() {
        // C4ObjectMenu's C4MN_Sell refill remembers the selected C4ID. If
        // that definition remains, checkIDSelection restores its row; if it
        // disappears, C4Menu::AdjustSelection keeps the old numeric slot
        // when that slot is still valid (C4ObjectMenu.cpp:147-164,238-275;
        // C4Menu.cpp:943-973,993-1017).
        let mut engine = Engine::new();
        register_clonk(&mut engine, "CLNK", "#strict\n");
        let mut hut =
            Definition::from_script("HUT3", "Hut", "#strict\n").expect("hut compiles");
        hut.set_category(crate::CATEGORY_STRUCTURE);
        hut.set_entrance_rect(Some(crate::DefinitionRect::new(-10, -10, 20, 20)));
        hut.set_auto_context_menu(true);
        engine.register_definition(hut).expect("register hut");
        for (id, name, category, value) in [
            ("FLAG", "Flag", crate::CATEGORY_OBJECT, 100),
            ("LORY", "Lorry", crate::CATEGORY_VEHICLE, 20),
            ("BARL", "Barrel", crate::CATEGORY_STRUCTURE, 5),
        ] {
            let mut definition =
                Definition::from_script(id, name, "#strict\n").expect("item compiles");
            definition.set_category(category);
            definition.set_value(value);
            engine
                .register_definition(definition)
                .expect("register item");
        }
        engine
            .register_player(PlayerConfig::new(1, "Test"))
            .expect("player");
        engine.player_mut(1).expect("player").control.auto_context_menu = true;
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        let crew_index = engine.find_object_index(crew).expect("crew exists");
        engine.objects[crew_index].state.category = crate::CATEGORY_LIVING;
        let hut = engine
            .spawn_object(SpawnConfig::new("HUT3"))
            .expect("spawn hut");
        let hut_index = engine.find_object_index(hut).expect("hut exists");
        engine.objects[hut_index].state.base = 1;
        engine
            .spawn_object(SpawnConfig::new("FLAG").with_container(hut))
            .expect("spawn flag");
        engine
            .spawn_object(SpawnConfig::new("LORY").with_container(hut))
            .expect("spawn first lorry");
        engine
            .spawn_object(SpawnConfig::new("LORY").with_container(hut))
            .expect("spawn second lorry");
        engine
            .spawn_object(SpawnConfig::new("BARL").with_container(hut))
            .expect("spawn barrel");
        engine
            .apply_object_update(crew, crate::ObjectUpdate::new().with_container(hut))
            .expect("enter hut");
        engine.execute_player_controls().expect("player execute");

        engine.player_in_com(1, COM_RIGHT, 0).expect("select Buy");
        engine.player_in_com(1, COM_RIGHT, 0).expect("select Sell");
        engine.player_in_com(1, COM_THROW, 0).expect("enter Sell");
        engine
            .player_in_com(1, COM_RIGHT, 0)
            .expect("select the non-first LORY group");
        engine
            .player_in_com(1, COM_THROW, 0)
            .expect("sell one lorry");

        let menu = engine
            .debug_object_menu(crew.as_u64())
            .expect("crew exists")
            .expect("sell menu remains open");
        assert_eq!(menu.selection, 1);
        assert_eq!(menu.items[1].item_id, "LORY");
        assert_eq!(menu.items[1].count, 1);

        engine
            .player_in_com(1, COM_THROW, 0)
            .expect("sell the last lorry");

        let menu = engine
            .debug_object_menu(crew.as_u64())
            .expect("crew exists")
            .expect("sell menu remains open");
        assert_eq!(
            menu.items
                .iter()
                .map(|item| item.item_id.as_str())
                .collect::<Vec<_>>(),
            vec!["FLAG", "BARL"]
        );
        assert_eq!(menu.selection, 1);
        assert_eq!(menu.items[1].item_id, "BARL");
    }

    #[test]
    fn closing_auto_context_menu_exits_the_building() {
        // AutoContextMenu installs a close command that issues Exit for
        // selected clonks; COM_MenuClose invokes it after closing
        // (C4Object.cpp:2044-2062; C4Menu.cpp:317-331).
        let mut engine = Engine::new();
        register_clonk(&mut engine, "CLNK", "#strict\n");
        let mut hut =
            Definition::from_script("HUT3", "Hut", "#strict\n").expect("hut compiles");
        hut.set_category(crate::CATEGORY_STRUCTURE);
        hut.set_entrance_rect(Some(crate::DefinitionRect::new(-10, -10, 20, 20)));
        hut.set_auto_context_menu(true);
        engine.register_definition(hut).expect("register hut");
        engine
            .register_player(PlayerConfig::new(1, "Test"))
            .expect("player");
        engine.player_mut(1).expect("player").control.auto_context_menu = true;
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        let hut = engine
            .spawn_object(SpawnConfig::new("HUT3"))
            .expect("spawn hut");
        let hut_index = engine.find_object_index(hut).expect("hut exists");
        engine.objects[hut_index].state.base = 1;
        // Model the already-open HUT3 door. With EntranceStatus=false C++
        // asks ActivateEntrance and leaves Exit pending instead
        // (C4Command.cpp:624-665).
        engine.objects[hut_index].state.entrance_status = true;
        engine
            .apply_object_update(crew, crate::ObjectUpdate::new().with_container(hut))
            .expect("enter hut");
        engine.execute_player_controls().expect("player execute");

        engine.player_in_com(1, COM_DIG, 0).expect("close menu");

        let crew_snapshot = engine.object_snapshot(crew).expect("crew survives");
        assert_eq!(
            crew_snapshot.container, None,
            "the context close command exits"
        );
        assert_eq!(
            engine.debug_object_menu(crew.as_u64()),
            Some(None),
            "the context menu remains closed"
        );
    }

    #[test]
    fn contained_com_dig_opens_the_base_sell_menu() {
        // ContainedControl COM_Dig (C4Object.cpp:3275-3280): the sell menu
        // twin, gated on BASEFUNC_Sell.
        let mut engine = Engine::new();
        let (crew, hut) = contained_base_fixture(&mut engine, 1);

        engine.player_in_com(1, COM_DIG, 0).expect("in_com");
        assert_eq!(
            engine
                .debug_object_menu(crew.as_u64())
                .expect("crew exists")
                .expect("COM_Dig opens a menu")
                .identification,
            Value::Int(5),
            "COM_Dig activates C4MN_Sell on the clonk"
        );
        assert!(engine.pending_menu_requests.is_empty());
        assert_eq!(engine.object_snapshot(crew).expect("crew").container, Some(hut));
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
        let mut hut_def = Definition::from_script("HUT1", "Hut", hut).expect("hut compiles");
        hut_def.set_version([4, 9, 1, 3, 0]);
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

    #[test]
    fn old_contained_script_runs_after_hardcoded_exit_and_cannot_consume_it() {
        // Before 4.9.1.3 C4Object::ContainedControl queues its hardcoded
        // action first, then calls Contained<Com> and ignores its return
        // value (src/C4Object.cpp:3246-3316).
        let hut = r#"
#strict
protected func ContainedDown(pByClonk) { DoDamage(1); return(1); }
"#;
        let mut engine = Engine::new();
        register_clonk(&mut engine, "CLNK", "#strict\n");
        let mut hut_def = Definition::from_script("HUT1", "Hut", hut).expect("hut compiles");
        hut_def.set_version([4, 9, 1, 2, 0]);
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

        assert_eq!(engine.object_snapshot(hut).expect("hut survives").damage, 1);
        assert_eq!(
            engine
                .object_snapshot(crew)
                .expect("crew survives")
                .command_stack
                .command_names(),
            vec!["Exit"],
            "the truthy late callback cannot consume the already-queued exit"
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
    fn wheel_shift_separates_same_definition_pictures_that_cannot_concat() {
        // ShiftContents does not merely compare definition IDs: it advances
        // to the first item for which C4Object::CanConcatPictureWith is false
        // (C4Object.cpp:5751-5775,6173-6213). Different ColorMod values split
        // an otherwise identical stack when APS_Color is not enabled.
        let mut engine = Engine::new();
        register_clonk(&mut engine, "CLNK", "#strict\n");
        engine
            .register_definition(
                Definition::from_script("ROCK", "Rock", "#strict\n").expect("item compiles"),
            )
            .expect("item registers");
        engine
            .register_player(PlayerConfig::new(1, "Test"))
            .expect("player");
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        let plain = engine
            .spawn_object(SpawnConfig::new("ROCK").with_container(crew))
            .expect("plain rock spawns");
        let tinted = engine
            .spawn_object(
                SpawnConfig::new("ROCK")
                    .with_container(crew)
                    .with_color_modulation(0x0080_8080),
            )
            .expect("tinted rock spawns");
        let index = engine.find_object_index(crew).expect("crew exists");
        engine.objects[index].state.contents = vec![plain, tinted];

        engine.player_in_com(1, COM_WHEEL_DOWN, 0).expect("wheel");
        assert_eq!(
            contents(&engine, crew),
            vec![tinted, plain],
            "non-concatenable same-definition picture becomes the new front"
        );
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

    /// Three equal-definition crew members for cursor-com cycling. C++
    /// stMain order is newest-first, while the cursor is deliberately put on
    /// the oldest member to exercise both link walking and wrap-around.
    fn crew_trio(engine: &mut Engine) -> [ObjectId; 3] {
        register_clonk(engine, "CLNK", "#strict\n");
        engine
            .register_player(PlayerConfig::new(1, "Test"))
            .expect("player");
        let spawn = |engine: &mut Engine| {
            engine
                .spawn_object(
                    SpawnConfig::new("CLNK")
                        .with_owner(1)
                        .with_crew_member(true)
                        .with_action(ActionState::new("Walk")),
                )
                .expect("spawn crew")
        };
        let trio = [spawn(engine), spawn(engine), spawn(engine)];
        engine.select_crew(1, vec![trio[0]]).expect("select");
        engine.set_crew_cursor(1, Some(trio[0])).expect("cursor");
        trio
    }

    fn control_state(engine: &Engine, owner: i32) -> &crate::player::PlayerControlState {
        &engine.players.get(&owner).expect("player").control
    }

    #[test]
    fn cursor_right_cycles_the_crew_in_roster_order_skipping_disabled() {
        // C4Player::CursorRight (C4Player.cpp:1261-1275): the next crew
        // link with Status and !CrewDisabled becomes the cursor;
        // CursorFlash = 30 and CursorSelection = 1.
        let mut engine = Engine::new();
        let [_, b, c] = crew_trio(&mut engine);
        let b_index = engine.find_object_index(b).expect("b exists");
        engine.objects[b_index].state.crew_disabled = true;

        engine.player_in_com(1, COM_CURSOR_RIGHT, 0).expect("in_com");
        assert_eq!(
            engine.crew_cursor(1),
            Some(c),
            "the disabled middle crew is skipped (C4Player.cpp:1267)"
        );
        assert_eq!(control_state(&engine, 1).cursor_flash, 30);
        assert_eq!(control_state(&engine, 1).cursor_selection, 1);
    }

    #[test]
    fn mouse_free_right_click_selects_only_the_next_crew() {
        // C4MouseControl::SendPlayerSelectNext queues a one-object
        // CID_PlrSelect, whose C4Player::SelectCrew immediately replaces the
        // whole selection. It is not COM_CursorRight's pending selection mode
        // (C4MouseControl.cpp:1284-1300; C4Control.cpp:341-369).
        let mut engine = Engine::new();
        let [a, b, c] = crew_trio(&mut engine);
        engine.select_crew(1, [a, b, c]).expect("select all");
        engine.set_crew_cursor(1, Some(a)).expect("oldest cursor");

        assert!(engine
            .player_mouse_select_next(1)
            .expect("mouse select-next control"));
        assert_eq!(engine.crew_cursor(1), Some(c));
        assert_eq!(engine.selected_crew(1), vec![c]);
        assert_eq!(control_state(&engine, 1).cursor_selection, 0);
        assert_eq!(control_state(&engine, 1).cursor_toggled, 0);
        assert_eq!(control_state(&engine, 1).select_flash, 30);
    }

    #[test]
    fn mouse_right_drag_frame_skips_disabled_crew_and_replaces_selection() {
        // UpdateCrewSelection compares crew origins against an inclusive
        // frame and CID_PlrSelect executes C4Player::SelectCrew, which first
        // unselects the old set (C4MouseControl.cpp:610-624,1160-1171;
        // C4Player.cpp:1848-1862).
        let mut engine = Engine::new();
        let [a, b, c] = crew_trio(&mut engine);
        for (id, position) in [
            (a, Vector2::new(5, 5)),
            (b, Vector2::new(10, 10)),
            (c, Vector2::new(15, 15)),
        ] {
            let index = engine.find_object_index(id).expect("crew exists");
            engine.objects[index].state.position = position;
        }
        let b_index = engine.find_object_index(b).expect("middle crew exists");
        engine.objects[b_index].state.crew_disabled = true;

        assert_eq!(
            engine.mouse_drag_crew_in_rect(1, Vector2::ZERO, Vector2::new(10, 10)),
            vec![a],
            "the max edge is inclusive, but CrewDisabled is not selectable"
        );
        engine.select_crew(1, [a, b, c]).expect("select all first");
        engine
            .player_mouse_select_crew(1, [c])
            .expect("execute CID_PlrSelect semantics");
        assert_eq!(engine.selected_crew(1), vec![c]);
        assert_eq!(engine.crew_cursor(1), Some(c));
        assert_eq!(control_state(&engine, 1).select_flash, 30);
    }

    #[test]
    fn mouse_object_frame_uses_main_list_order_and_caps_selection_at_twenty() {
        // UpdateObjectSelection walks Game.Objects.First, adds with stNone,
        // and breaks at 20 (C4MouseControl.cpp:626-645). Same-definition
        // runtime objects are newest-first in that master list.
        let mut engine = Engine::new();
        let mut item =
            Definition::from_script("ITEM", "Item", "#strict\n").expect("item compiles");
        item.set_collectible(true);
        engine.register_definition(item).expect("register item");
        let items = (0..22)
            .map(|x| {
                engine
                    .spawn_object(
                        SpawnConfig::new("ITEM").with_position(Vector2::new(x, x)),
                    )
                    .expect("spawn carryable")
            })
            .collect::<Vec<_>>();

        let selected = engine.mouse_drag_carryables_in_rect(
            Vector2::ZERO,
            Vector2::new(30, 30),
        );
        assert_eq!(selected.len(), 20);
        assert_eq!(selected, items[2..].iter().rev().copied().collect::<Vec<_>>());
    }

    #[test]
    fn mouse_carryable_cursor_distinguishes_drop_solid_and_throw_points() {
        // DragMoving selects Drop within five pixels of ground, no moving
        // command in solid, and Throw when FindThrowingPosition reaches a
        // free-air target (C4MouseControl.cpp:849-878).
        let mut engine = Engine::new();
        let mut landscape = Landscape::flat(100, 50);
        landscape.set_world_height(100);
        engine.set_landscape(landscape);
        engine.set_physics(PhysicsSettings::new(100, 12, -20));

        assert_eq!(
            engine.mouse_drag_carryable_command(1, Vector2::new(20, 45)),
            Some(CommandId::Drop)
        );
        assert_eq!(
            engine.mouse_drag_carryable_command(1, Vector2::new(20, 50)),
            None
        );
        assert_eq!(
            engine.mouse_drag_carryable_command(1, Vector2::new(70, 20)),
            Some(CommandId::Throw)
        );
    }

    #[test]
    fn mouse_dragged_objects_queue_set_then_append_in_selection_order() {
        // ButtonUpDragMoving sends C4P_Command_Set for the first selected
        // object and C4P_Command_Append thereafter (C4MouseControl.cpp:
        // 1171-1201; C4Player.cpp:1445-1450).
        let mut engine = Engine::new();
        let (crew, first) = drop_window_fixture(&mut engine);
        let second = engine
            .spawn_object(SpawnConfig::new("GOLD").with_container(crew))
            .expect("spawn second item");

        assert!(engine
            .player_mouse_drag_objects(
                1,
                CommandId::Drop,
                [second, first],
                Vector2::new(25, 30),
            )
            .expect("mouse object controls execute"));
        let commands = engine
            .object_snapshot(crew)
            .expect("crew remains live")
            .command_stack
            .command_views();
        assert_eq!(commands.len(), 2);
        assert_eq!(commands[0].name, "Drop");
        assert_eq!(commands[0].target, Some(second));
        assert_eq!(commands[0].tx, Some(25));
        assert_eq!(commands[0].ty, Some(30));
        assert_eq!(commands[1].name, "Drop");
        assert_eq!(commands[1].target, Some(first));
    }

    #[test]
    fn mouse_control_drag_put_targets_container_and_appends_items_in_order() {
        // UpdatePutTarget chooses only OCF_Container objects, then
        // ButtonUpDragMoving sends Put(Target=container, Target2=item): Set
        // for the first item and Append for each following item
        // (C4MouseControl.cpp:742-768,1171-1201).
        let mut engine = Engine::new();
        let (crew, first) = drop_window_fixture(&mut engine);
        let second = engine
            .spawn_object(SpawnConfig::new("GOLD").with_container(crew))
            .expect("spawn second item");
        let mut hut = Definition::from_script("HUT1", "Hut", "#strict\n")
            .expect("hut compiles");
        hut.set_grab_put_get(crate::GRAB_PUT_GET_PUT);
        engine.register_definition(hut).expect("register hut");
        let hut = engine
            .spawn_object(SpawnConfig::new("HUT1"))
            .expect("spawn hut");

        assert!(engine
            .player_mouse_drag_put(1, [second, first], hut, false)
            .expect("mouse Put controls execute"));
        let commands = engine
            .object_snapshot(crew)
            .expect("crew remains live")
            .command_stack
            .command_views();
        assert_eq!(commands.len(), 2);
        assert!(commands.iter().all(|command| command.name == "Put"));
        assert!(commands.iter().all(|command| command.target == Some(hut)));
        assert_eq!(commands[0].target2, Some(second));
        assert_eq!(commands[1].target2, Some(first));
        assert!(commands.iter().all(|command| command.tx.is_none()));
        assert!(commands.iter().all(|command| command.ty.is_none()));
    }

    #[test]
    fn mouse_vehicle_drag_requires_grab_one_and_carryable_wins() {
        // DragNone starts a landscape vehicle drag only for the Grab/Ungrab
        // cursor and Def->Grab == 1. DragMoving checks OCF_Carryable first,
        // so a hybrid object remains an item drag (C4MouseControl.cpp:
        // 922-941,833-889).
        let mut engine = Engine::new();
        engine
            .register_player(PlayerConfig::new(1, "Test"))
            .expect("player");
        register_clonk(&mut engine, "CLNK", "#strict\n");
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        let crew_index = engine.find_object_index(crew).expect("crew exists");
        engine.objects[crew_index].state.position = Vector2::new(100, 100);

        let mut vehicle = Definition::from_script("VEH1", "Vehicle", "#strict\n")
            .expect("vehicle compiles");
        vehicle.set_grab(1);
        vehicle.set_category(crate::CATEGORY_VEHICLE);
        engine.register_definition(vehicle).expect("register vehicle");
        let mut grab_only = Definition::from_script("VEH2", "Grab-only", "#strict\n")
            .expect("grab-only compiles");
        grab_only.set_grab(2);
        grab_only.set_category(crate::CATEGORY_VEHICLE);
        engine
            .register_definition(grab_only)
            .expect("register grab-only");
        let mut hybrid = Definition::from_script("VEH3", "Hybrid", "#strict\n")
            .expect("hybrid compiles");
        hybrid.set_grab(1);
        hybrid.set_category(crate::CATEGORY_VEHICLE);
        hybrid.set_collectible(true);
        engine.register_definition(hybrid).expect("register hybrid");
        let mut site = Definition::from_script("SITE", "Site", "#strict\n")
            .expect("site compiles");
        site.set_grab(1);
        site.set_category(crate::CATEGORY_VEHICLE);
        site.set_collectible(true);
        site.set_constructable(true);
        engine.register_definition(site).expect("register site");

        let vehicle = engine
            .spawn_object(SpawnConfig::new("VEH1").with_position(Vector2::new(10, 10)))
            .expect("spawn vehicle");
        let grab_only = engine
            .spawn_object(SpawnConfig::new("VEH2").with_position(Vector2::new(20, 10)))
            .expect("spawn grab-only");
        let hybrid = engine
            .spawn_object(SpawnConfig::new("VEH3").with_position(Vector2::new(30, 10)))
            .expect("spawn hybrid");
        let site = engine
            .spawn_object(
                SpawnConfig::new("SITE")
                    .with_position(Vector2::new(40, 10))
                    .with_construction(crate::FULL_CON / 2),
            )
            .expect("spawn construction site");

        assert_eq!(
            engine.mouse_world_drag_source(1, vehicle, Vector2::new(10, 10)),
            Some(crate::MouseDragSource::Vehicle)
        );
        assert_eq!(
            engine.mouse_world_drag_source(1, grab_only, Vector2::new(20, 10)),
            None,
            "Grab=2 has a Grab cursor but cannot enter C4MC_Drag_Moving"
        );
        assert_eq!(
            engine.mouse_world_drag_source(1, hybrid, Vector2::new(30, 10)),
            Some(crate::MouseDragSource::Carryable),
            "OCF_Carryable is evaluated before the vehicle branch"
        );
        assert_eq!(
            engine.mouse_world_drag_source(1, site, Vector2::new(40, 10)),
            None,
            "the later Build cursor overrides Carryable and Grab"
        );
    }

    #[test]
    fn mouse_right_drag_region_expands_same_id_in_contents_order() {
        // A right drag from a viewport inventory region selects every object
        // with the target's ID in its containing object's forward Contents
        // list; a single/left drag keeps only the region target
        // (C4MouseControl.cpp:942-961).
        let mut engine = Engine::new();
        let mut container = Definition::from_script("CONT", "Container", "#strict\n")
            .expect("container compiles");
        container.set_grab_put_get(crate::GRAB_PUT_GET_GET);
        engine
            .register_definition(container)
            .expect("register container");
        let mut item = Definition::from_script("ITEM", "Item", "#strict\n")
            .expect("item compiles");
        item.set_collectible(true);
        engine.register_definition(item).expect("register item");
        let mut other = Definition::from_script("OTHR", "Other", "#strict\n")
            .expect("other compiles");
        other.set_collectible(true);
        engine.register_definition(other).expect("register other");

        let container = engine
            .spawn_object(SpawnConfig::new("CONT"))
            .expect("spawn container");
        let first = engine
            .spawn_object(SpawnConfig::new("ITEM").with_container(container))
            .expect("spawn first item");
        engine
            .spawn_object(SpawnConfig::new("OTHR").with_container(container))
            .expect("spawn other item");
        let second = engine
            .spawn_object(SpawnConfig::new("ITEM").with_container(container))
            .expect("spawn second item");
        let third = engine
            .spawn_object(SpawnConfig::new("ITEM").with_container(container))
            .expect("spawn third item");

        assert_eq!(
            engine.mouse_region_drag_objects(first, false),
            vec![first],
            "non-right region drags keep one object"
        );
        assert_eq!(
            engine.mouse_region_drag_objects(first, true),
            vec![third, second, first],
            "runtime stContents is newest-first inside the same-ID cluster"
        );
    }

    #[test]
    fn mouse_dragged_vehicles_queue_push_to_set_then_append() {
        // ButtonUpDragMoving emits PushTo(Target=vehicle, Target2=putTarget)
        // at the release coordinates. The first command is Set and following
        // vehicles are Append; Shift makes the first Append too
        // (C4MouseControl.cpp:1171-1227).
        let mut engine = Engine::new();
        register_clonk(&mut engine, "CLNK", "#strict\n");
        let mut vehicle = Definition::from_script("VEH1", "Vehicle", "#strict\n")
            .expect("vehicle compiles");
        vehicle.set_grab(1);
        engine.register_definition(vehicle).expect("register vehicle");
        let mut container = Definition::from_script("CONT", "Container", "#strict\n")
            .expect("container compiles");
        container.set_grab_put_get(crate::GRAB_PUT_GET_PUT);
        engine
            .register_definition(container)
            .expect("register container");
        engine
            .register_player(PlayerConfig::new(1, "Test"))
            .expect("player");
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        let first = engine
            .spawn_object(SpawnConfig::new("VEH1"))
            .expect("spawn first vehicle");
        let second = engine
            .spawn_object(SpawnConfig::new("VEH1"))
            .expect("spawn second vehicle");
        let destination = engine
            .spawn_object(SpawnConfig::new("CONT"))
            .expect("spawn destination");

        assert!(engine
            .player_mouse_drag_vehicles(
                1,
                [second, first],
                Vector2::new(70, 80),
                Some(destination),
                false,
            )
            .expect("vehicle commands execute"));
        let commands = engine
            .object_snapshot(crew)
            .expect("crew remains live")
            .command_stack
            .command_views();
        assert_eq!(commands.len(), 2);
        assert!(commands.iter().all(|command| command.name == "PushTo"));
        assert_eq!(commands[0].target, Some(second));
        assert_eq!(commands[1].target, Some(first));
        assert!(commands
            .iter()
            .all(|command| command.target2 == Some(destination)));
        assert!(commands
            .iter()
            .all(|command| command.tx == Some(70) && command.ty == Some(80)));

        assert!(engine
            .player_mouse_drag_vehicles(
                1,
                [first],
                Vector2::new(90, 100),
                None,
                true,
            )
            .expect("Shift-append vehicle command executes"));
        let commands = engine
            .object_snapshot(crew)
            .expect("crew remains live")
            .command_stack
            .command_views();
        assert_eq!(commands.len(), 3, "Shift preserves both prior commands");
        assert_eq!(commands[2].name, "PushTo");
        assert_eq!(commands[2].target, Some(first));
        assert_eq!(commands[2].target2, None);
        assert_eq!(commands[2].tx, Some(90));
        assert_eq!(commands[2].ty, Some(100));
    }

    #[test]
    fn cursor_left_steps_to_the_previous_master_order_crew_member() {
        // C4Player::CursorLeft (C4Player.cpp:1278-1293): equal-definition
        // crew links are newest-first, so the member before the oldest is
        // the middle-created Clonk.
        let mut engine = Engine::new();
        let [_, b, _] = crew_trio(&mut engine);

        engine.player_in_com(1, COM_CURSOR_LEFT, 0).expect("in_com");
        assert_eq!(engine.crew_cursor(1), Some(b));
    }

    #[test]
    fn cursor_toggle_in_selection_mode_toggles_the_cursor_select() {
        // After a cursor com CursorSelection = 1, so CursorToggle flips the
        // cursor's Select and arms CursorToggled (C4Player.cpp:1322-1327).
        let mut engine = Engine::new();
        let [a, _, c] = crew_trio(&mut engine);

        engine.player_in_com(1, COM_CURSOR_RIGHT, 0).expect("right");
        assert_eq!(engine.crew_cursor(1), Some(c));
        engine.player_in_com(1, COM_CURSOR_TOGGLE, 0).expect("toggle");
        assert_eq!(
            engine.selected_crew(1),
            vec![c, a],
            "the new cursor's Select toggled ON"
        );
        assert_eq!(control_state(&engine, 1).cursor_toggled, 1);
        assert_eq!(control_state(&engine, 1).select_flash, 30);
    }

    #[test]
    fn regular_com_after_cursor_move_selects_single_by_cursor() {
        // UpdateSelectionToggleStatus (C4Player.cpp:1355-1365) runs on the
        // next regular com (C4Player::ObjectCom, :1378-1379): an untoggled
        // CursorSelection commits SelectSingleByCursor — only the cursor
        // stays selected.
        let mut engine = Engine::new();
        let [_, _, c] = crew_trio(&mut engine);

        engine.player_in_com(1, COM_CURSOR_RIGHT, 0).expect("right");
        engine.player_in_com(1, COM_DOWN, 0).expect("down");
        assert_eq!(
            engine.selected_crew(1),
            vec![c],
            "SelectSingleByCursor unselected the rest (C4Player.cpp:1308-1317)"
        );
        assert_eq!(engine.crew_cursor(1), Some(c));
        assert_eq!(control_state(&engine, 1).cursor_selection, 0);
    }

    #[test]
    fn cursor_toggle_double_selects_all_crew() {
        // COM_CursorToggle_D → SelectAllCrew (C4Player.cpp:1485,
        // 1341-1353): everyone Select, flags reset, Ding.
        let mut engine = Engine::new();
        let [a, b, c] = crew_trio(&mut engine);

        engine.player_in_com(1, COM_CURSOR_TOGGLE, 0).expect("first");
        engine.player_in_com(1, COM_CURSOR_TOGGLE, 0).expect("second");
        let mut selected = engine.selected_crew(1);
        selected.sort_by_key(|id| id.as_u64());
        assert_eq!(selected, vec![a, b, c]);
        assert_eq!(control_state(&engine, 1).cursor_selection, 0);
        assert_eq!(control_state(&engine, 1).cursor_toggled, 0);
        assert!(
            engine.pending_audio.iter().any(|command| matches!(
                command,
                crate::AudioCommand::PlaySound { name, .. } if name == "Ding"
            )),
            "SelectAllCrew plays Ding (C4Player.cpp:1352)"
        );
    }

    #[test]
    fn pure_cursor_toggle_flips_select_on_the_whole_crew() {
        // Without CursorSelection the toggle flips every non-disabled
        // crew's Select (C4Player.cpp:1329-1336) and re-adjusts the cursor
        // to the hirank Select (AdjustCursorCommand, :1235-1258).
        let mut engine = Engine::new();
        let [a, b, c] = crew_trio(&mut engine);

        engine.player_in_com(1, COM_CURSOR_TOGGLE, 0).expect("toggle");
        // a was selected -> off; b, c were unselected -> on.
        assert_eq!(engine.selected_crew(1), vec![c, b]);
        assert_eq!(
            engine.crew_cursor(1),
            Some(c),
            "AdjustCursorCommand moves the cursor to the first Select"
        );
        let _ = a;
    }

    #[test]
    fn cursor_com_script_override_consumes_the_cycling() {
        // C4Player::DirectCom's cursor half (C4Player.cpp:1457-1475): a
        // truthy ControlCursorRight on the cursor object consumes the com
        // before any cycling.
        let script = r#"
#strict
protected func ControlCursorRight() { return(1); }
"#;
        let mut engine = Engine::new();
        register_clonk(&mut engine, "CLNK", script);
        engine
            .register_player(PlayerConfig::new(1, "Test"))
            .expect("player");
        let a = spawn_crew(&mut engine, "CLNK", 1);
        let b = engine
            .spawn_object(
                SpawnConfig::new("CLNK")
                    .with_owner(1)
                    .with_crew_member(true)
                    .with_action(ActionState::new("Walk")),
            )
            .expect("spawn b");

        engine.player_in_com(1, COM_CURSOR_RIGHT, 0).expect("in_com");
        assert_eq!(
            engine.crew_cursor(1),
            Some(a),
            "the override kept the cursor in place"
        );
        let _ = b;
    }

    #[test]
    fn jump_and_run_down_keeps_grabbing_a_target_with_a_down_command() {
        // AutoStopDirectCom's DFA_PUSH/COM_Down branch retains the grab when
        // DrawCommandQuery exposes a JumpAndRun ControlDown command
        // (C4Object.cpp:3712-3721). The callback may legitimately be falsy;
        // its command metadata, not its return value, owns this gate.
        let vehicle = r#"
#strict
protected func ControlDown(pCaller)
{
  [$CtrlDown$|Method=JumpAndRun]
  DoDamage(1);
}
"#;
        let mut engine = Engine::new();
        register_clonk(&mut engine, "CLNK", "#strict\n");
        engine
            .register_definition(
                Definition::from_script("DRCK", "Derrick", vehicle)
                    .expect("derrick compiles"),
            )
            .expect("register derrick");
        engine
            .register_player(PlayerConfig::new(1, "Test"))
            .expect("player");
        engine
            .players
            .get_mut(&1)
            .expect("player exists")
            .control
            .control_style = true;
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        let derrick = engine
            .spawn_object(SpawnConfig::new("DRCK"))
            .expect("spawn derrick");
        let crew_index = engine.find_object_index(crew).expect("crew exists");
        engine.objects[crew_index].state.action.name = "Push".to_string();
        engine.objects[crew_index].state.action.target = Some(derrick);

        engine.player_in_com(1, COM_DOWN, 0).expect("in_com");

        let derrick_index = engine
            .find_object_index(derrick)
            .expect("derrick exists");
        assert_eq!(
            engine.objects[derrick_index].state.damage, 1,
            "the target callback still runs first"
        );
        let snapshot = engine.object_snapshot(crew).expect("crew snapshot");
        assert_eq!(snapshot.action.name, "Push");
        assert_eq!(snapshot.action.target, Some(derrick));
    }

    #[test]
    fn old_pushed_target_receives_autostop_control_after_clonk_fallback() {
        // AutoStopDirectCom uses the same 4.9.5 target-version boundary as
        // classic DFA_PUSH: old ControlLeft runs after AutoStopUpdateComDir
        // and cannot consume it (src/C4Object.cpp:3682-3738).
        let vehicle = r#"
#strict
protected func ControlLeft(pByClonk) { DoDamage(1); return(1); }
"#;
        let mut engine = Engine::new();
        register_clonk(&mut engine, "CLNK", "#strict\n");
        let mut lorry =
            Definition::from_script("LORY", "Lorry", vehicle).expect("lorry compiles");
        lorry.set_version([4, 9, 4, 9, 0]);
        engine.register_definition(lorry).expect("register lorry");
        engine
            .register_player(PlayerConfig::new(1, "Test"))
            .expect("player");
        engine
            .players
            .get_mut(&1)
            .expect("player exists")
            .control
            .control_style = true;
        let crew = spawn_crew(&mut engine, "CLNK", 1);
        let lorry = engine
            .spawn_object(SpawnConfig::new("LORY"))
            .expect("spawn lorry");
        let crew_index = engine.find_object_index(crew).expect("crew exists");
        engine.objects[crew_index].state.action.name = "Push".to_string();
        engine.objects[crew_index].state.action.target = Some(lorry);

        engine.player_in_com(1, COM_LEFT, 0).expect("in_com");

        assert_eq!(
            engine
                .object_snapshot(crew)
                .expect("crew survives")
                .command_direction,
            CommandDirection::Left,
            "the old target's truthy late callback cannot consume auto-stop movement"
        );
        assert_eq!(
            engine.object_snapshot(lorry).expect("lorry survives").damage,
            1,
            "the old target still receives ControlLeft after movement"
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
