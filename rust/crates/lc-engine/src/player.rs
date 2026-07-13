use crate::{DefinitionId, ObjectId, RgbColor, Vector2};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

pub(crate) const MAX_HOME_BASE_MATERIAL: u32 = 25;
const MAX_SET_WEALTH: i32 = 100_000;
const MAX_WEALTH_ADJUSTMENT: i32 = 10_000;
const MAX_SCORE: i32 = 100_000;
const MIN_SCORE: i32 = -100_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum PlayerStatus {
    #[default]
    Inactive,
    Active,
    Eliminated,
    TeamSelection,
    Surrendered,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayerViewport {
    #[serde(default)]
    pub focus: Option<ObjectId>,
    pub center: Vector2,
    #[serde(default = "default_zoom")]
    pub zoom: f32,
}

impl PlayerViewport {
    pub fn new(center: Vector2) -> Self {
        Self {
            focus: None,
            center,
            zoom: default_zoom(),
        }
    }

    pub fn with_focus(mut self, focus: Option<ObjectId>) -> Self {
        self.focus = focus;
        self
    }

    pub fn with_zoom(mut self, zoom: f32) -> Self {
        self.zoom = zoom.max(0.0);
        self
    }
}

fn default_zoom() -> f32 {
    1.0
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PlayerState {
    pub id: i32,
    /// Unique `C4Player::ID` linking this runtime player to `C4PlayerInfo`;
    /// distinct from the in-round `C4Player::Number` stored in `id`
    /// (C4Player.h:67-70).
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub player_info_id: i32,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub status: PlayerStatus,
    #[serde(default)]
    pub team: Option<i32>,
    #[serde(default)]
    pub surrendered: bool,
    /// Result flag projected from the linked `C4PlayerInfo::PIF_Won` state
    /// (`C4PlayerInfo.h:63,219-237`).
    #[serde(default, skip_serializing_if = "is_false")]
    pub won: bool,
    /// `C4Player::Evaluated`, kept separate from game-over so evaluation is
    /// idempotent (`C4Player.cpp:930-970`).
    #[serde(default, skip_serializing_if = "is_false")]
    pub evaluated: bool,
    #[serde(default)]
    pub wealth: i32,
    #[serde(default)]
    pub points: i32,
    /// Persistent settlement score from `C4PlayerInfoCore`, not the in-round
    /// `Points` counter (`C4InfoCore.h:202-204`).
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub score: i32,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub total_playing_time: i32,
    #[serde(default)]
    pub value: i32,
    #[serde(default)]
    pub initial_value: i32,
    #[serde(default)]
    pub value_gain: i32,
    #[serde(default)]
    pub objects_owned: u32,
    #[serde(default)]
    pub initial_value_set: bool,
    #[serde(default)]
    pub knowledge: Vec<DefinitionId>,
    /// Ordered `C4Player::Magic` ID list (C4Player.h:114). Unlike build
    /// knowledge, script indexed lookups observe this list's order.
    #[serde(default)]
    pub magic: Vec<DefinitionId>,
    #[serde(default)]
    pub inventory: HashMap<DefinitionId, u32>,
    #[serde(default)]
    pub cursor: Option<ObjectId>,
    #[serde(default)]
    pub viewports: Vec<PlayerViewport>,
    /// Runtime-only `C4Viewport::ViewOffsX/Y` presentation displacement.
    /// Scripts such as FXQ1 use this independently of the player's saved
    /// `ViewX/ViewY` center (C4Viewport.cpp:1183-1214).
    #[serde(default, skip_serializing_if = "is_zero_vector")]
    pub view_offset: Vector2,
    #[serde(default)]
    pub crew: Vec<ObjectId>,
    #[serde(default)]
    pub home_base_material: HashMap<DefinitionId, u32>,
    #[serde(default)]
    pub home_base_production: HashMap<DefinitionId, u32>,
    #[serde(default)]
    pub production_delay: u32,
    #[serde(default)]
    pub production_unit: u32,
    #[serde(default)]
    pub color: Option<RgbColor>,
    /// Saved `C4Player::fFogOfWar` setting (C4Player.cpp:1580).
    #[serde(default)]
    pub fog_of_war: bool,
    /// Whether fog of war was explicitly forced instead of selected by mouse
    /// control (C4Player::bForceFogOfWar, C4Player.cpp:1581).
    #[serde(default)]
    pub force_fog_of_war: bool,
    /// C4Player::ShowControlPos: tutorial-selected placement for the command
    /// hint strip (FnSetPlrShowControlPos, C4Script.cpp:2561-2566).
    #[serde(default)]
    pub show_control_position: i32,
    /// C4Player::ShowControl: three ten-bit layers selecting command hints,
    /// their key labels, and blinking labels (C4Viewport.cpp:1424-1439).
    #[serde(default)]
    pub show_control: i32,
    /// Players this player declared hostility against (C4Player::Hostility,
    /// queried by C4PlayerList::Hostile, C4PlayerList.cpp:82-92). Sorted for
    /// deterministic serialization.
    #[serde(default)]
    pub hostility: Vec<i32>,
    /// Direct-com input state (C4Player.h:118-121, serialized by
    /// C4Player::CompileFunc "LastCom"/"LastComDel"/"LastComDownDouble"/
    /// "PressedComs"/"AutoStopControl"/"CursorFlash", C4Player.cpp:1596-1604).
    #[serde(default)]
    pub control: PlayerControlState,
    /// C4Player::ExtraData (C4ValueMapData) — the script-managed named
    /// slots of Fn[Set/Get]PlrExtraData (C4Script.cpp:4692-4747). Only
    /// nil/int/bool/id values store; insertion order preserved like the
    /// C4ValueMapNames list.
    #[serde(default)]
    pub extra_data: Vec<(String, lc_script::Value)>,
}

/// C4Player's per-player direct-com bookkeeping (C4Player.h:118-121):
/// the LastCom single/double synthesis buffer, the pressed-com bitmask
/// for Jump'n'Run control, and the cursor/select flash timers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PlayerControlState {
    /// `LastCom` — the com buffered for COM_Single/COM_Double synthesis
    /// (C4Player::InCom, C4Player.cpp:1522-1536).
    #[serde(default)]
    pub last_com: u8,
    /// `LastComDelay` — frames since LastCom was buffered; > C4DoubleClick
    /// flushes the COM_Single (C4Player::Execute, C4Player.cpp:1215-1229).
    #[serde(default)]
    pub last_com_delay: i32,
    /// `LastComDownDouble` — countdown after a COM_Down_D that converts the
    /// next throw command to a drop (PlayerObjectCommand,
    /// C4ObjectCom.cpp:1024-1036).
    #[serde(default)]
    pub last_com_down_double: i32,
    /// `PressedComs` — bit per held plain com (C4Player::InCom,
    /// C4Player.cpp:1520-1521, 1541-1548).
    #[serde(default)]
    pub pressed_coms: i32,
    /// `ControlStyle` — Jump'n'Run (AutoStopControl) control when true
    /// (C4Player.cpp:2373; default 0 = classic, C4InfoCore.cpp:84).
    #[serde(default)]
    pub control_style: bool,
    /// Effective C4Player::AutoContextMenu preference after the scenario
    /// ForcedAutoContextMenu override (C4Player.cpp:2369-2375).
    #[serde(default)]
    pub auto_context_menu: bool,
    /// `CursorFlash` — frames the cursor arrow above the cursor clonk stays
    /// visible (C4Game::DrawCursors gate, C4Game.cpp:1863; set to 30 on
    /// cursor changes, decremented in C4Player::Execute, C4Player.cpp:242).
    #[serde(default)]
    pub cursor_flash: i32,
    /// `SelectFlash` — frames the crew select marks stay visible
    /// (C4Object::Draw gate, C4Object.cpp:2497-2502).
    #[serde(default)]
    pub select_flash: i32,
    /// `CursorSelection` — armed to 1 by CursorLeft/CursorRight
    /// (C4Player.cpp:1274,1292); the next regular com commits the pick via
    /// UpdateSelectionToggleStatus (:1355-1365).
    #[serde(default)]
    pub cursor_selection: i32,
    /// `CursorToggled` — set when CursorToggle ran in selection mode
    /// (C4Player.cpp:1326); switches the commit from SelectSingleByCursor
    /// to AdjustCursorCommand.
    #[serde(default)]
    pub cursor_toggled: i32,
}

#[derive(Debug, Clone)]
pub struct Player {
    id: i32,
    player_info_id: i32,
    name: String,
    status: PlayerStatus,
    team: Option<i32>,
    surrendered: bool,
    won: bool,
    evaluated: bool,
    wealth: i32,
    points: i32,
    score: i32,
    total_playing_time: i32,
    /// `C4Player::GameJoinTime`: local runtime baseline, deliberately absent
    /// from PlayerState/save data (C4Player.h:78; C4Player.cpp:389-390).
    game_join_time: i32,
    /// `C4Player::RetireDelay`: runtime-only, set to C4RetireDelay by
    /// Eliminate and decremented once per player Execute (C4Player.cpp:
    /// 2015-2021, 239; C4Constants.h:36).
    retire_delay: i32,
    value: i32,
    initial_value: i32,
    value_gain: i32,
    objects_owned: u32,
    initial_value_set: bool,
    knowledge: HashSet<DefinitionId>,
    magic: Vec<DefinitionId>,
    inventory: HashMap<DefinitionId, u32>,
    cursor: Option<ObjectId>,
    viewports: Vec<PlayerViewport>,
    view_offset: Vector2,
    crew: Vec<ObjectId>,
    home_base_material: HashMap<DefinitionId, u32>,
    home_base_production: HashMap<DefinitionId, u32>,
    production_delay: u32,
    production_unit: u32,
    color: Option<RgbColor>,
    fog_of_war: bool,
    force_fog_of_war: bool,
    pub(crate) show_control_position: i32,
    pub(crate) show_control: i32,
    hostility: HashSet<i32>,
    /// The indexed player color chosen at ScenarioInit
    /// (C4Player.cpp:678-685; C4PlayerList::ColorTaken scans it). -1 until
    /// the join assigns one.
    color_index: i32,
    /// The startup position slot taken at ScenarioInit
    /// (C4Player.cpp:717-732; C4PlayerList::PositionTaken). -1 when unset.
    position_index: i32,
    /// Direct-com input state (C4Player.h:118-121).
    #[doc(hidden)] pub control: PlayerControlState,
    /// C4Player::ExtraData named slots (Fn[Set/Get]PlrExtraData).
    pub(crate) extra_data: Vec<(String, lc_script::Value)>,
}

impl Player {
    pub fn new(id: i32, name: impl Into<String>) -> Self {
        Self {
            id,
            player_info_id: 0,
            name: name.into(),
            status: PlayerStatus::Active,
            team: None,
            surrendered: false,
            won: false,
            evaluated: false,
            wealth: 0,
            points: 0,
            score: 0,
            total_playing_time: 0,
            game_join_time: 0,
            retire_delay: 0,
            value: 0,
            initial_value: 0,
            value_gain: 0,
            objects_owned: 0,
            initial_value_set: false,
            knowledge: HashSet::new(),
            magic: Vec::new(),
            inventory: HashMap::new(),
            cursor: None,
            viewports: Vec::new(),
            view_offset: Vector2::ZERO,
            crew: Vec::new(),
            home_base_material: HashMap::new(),
            home_base_production: HashMap::new(),
            production_delay: 0,
            production_unit: 0,
            color: None,
            fog_of_war: false,
            force_fog_of_war: false,
            show_control_position: 0,
            show_control: 0,
            hostility: HashSet::new(),
            color_index: -1,
            position_index: -1,
            control: PlayerControlState::default(),
            extra_data: Vec::new(),
        }
    }

    /// Whether this player uses Jump'n'Run/AutoStop control. Local key-up
    /// routing consults this just as C4Game::LocalControlKeyUp reads
    /// `C4Player::ControlStyle` (C4Game.cpp:3578-3592).
    pub fn control_style(&self) -> bool {
        self.control.control_style
    }

    pub fn from_config(config: PlayerConfig) -> Self {
        let PlayerConfig {
            id,
            player_info_id,
            name,
            status,
            team,
            surrendered,
            wealth,
            points,
            score,
            total_playing_time,
            value,
            initial_value,
            value_gain,
            objects_owned,
            initial_value_set,
            knowledge,
            magic,
            inventory,
            cursor,
            viewports,
            home_base_material,
            home_base_production,
            production_delay,
            production_unit,
            color,
        } = config;
        Self {
            id,
            player_info_id,
            name,
            status,
            team,
            surrendered,
            won: false,
            evaluated: false,
            wealth,
            points,
            score,
            total_playing_time,
            game_join_time: 0,
            retire_delay: 0,
            value,
            initial_value,
            value_gain,
            objects_owned,
            initial_value_set,
            knowledge: knowledge.into_iter().collect(),
            magic,
            inventory,
            cursor,
            viewports,
            view_offset: Vector2::ZERO,
            crew: Vec::new(),
            home_base_material,
            home_base_production,
            production_delay,
            production_unit,
            color,
            fog_of_war: false,
            force_fog_of_war: false,
            show_control_position: 0,
            show_control: 0,
            hostility: HashSet::new(),
            color_index: -1,
            position_index: -1,
            control: PlayerControlState::default(),
            extra_data: Vec::new(),
        }
    }

    pub fn from_state(state: PlayerState) -> Self {
        let PlayerState {
            id,
            player_info_id,
            name,
            status,
            team,
            surrendered,
            won,
            evaluated,
            wealth,
            points,
            score,
            total_playing_time,
            value,
            initial_value,
            value_gain,
            objects_owned,
            initial_value_set,
            knowledge,
            magic,
            inventory,
            cursor,
            viewports,
            view_offset,
            crew,
            home_base_material,
            home_base_production,
            production_delay,
            production_unit,
            color,
            fog_of_war,
            force_fog_of_war,
            show_control_position,
            show_control,
            hostility,
            control,
            extra_data,
        } = state;
        Self {
            id,
            player_info_id,
            name,
            status,
            team,
            surrendered,
            won,
            evaluated,
            wealth,
            points,
            score,
            total_playing_time,
            game_join_time: 0,
            retire_delay: 0,
            value,
            initial_value,
            value_gain,
            objects_owned,
            initial_value_set,
            knowledge: knowledge.into_iter().collect(),
            magic,
            inventory,
            cursor,
            viewports,
            view_offset,
            crew,
            home_base_material,
            home_base_production,
            production_delay,
            production_unit,
            color,
            fog_of_war,
            force_fog_of_war,
            show_control_position,
            show_control,
            hostility: hostility.into_iter().collect(),
            color_index: -1,
            position_index: -1,
            control,
            extra_data,
        }
    }

    pub fn to_state(&self) -> PlayerState {
        let mut knowledge: Vec<_> = self.knowledge.iter().cloned().collect();
        knowledge.sort();
        PlayerState {
            id: self.id,
            player_info_id: self.player_info_id,
            name: self.name.clone(),
            status: self.status,
            team: self.team,
            surrendered: self.surrendered,
            won: self.won,
            evaluated: self.evaluated,
            wealth: self.wealth,
            points: self.points,
            score: self.score,
            total_playing_time: self.total_playing_time,
            value: self.value,
            initial_value: self.initial_value,
            value_gain: self.value_gain,
            objects_owned: self.objects_owned,
            initial_value_set: self.initial_value_set,
            knowledge,
            magic: self.magic.clone(),
            inventory: self.inventory.clone(),
            cursor: self.cursor,
            viewports: self.viewports.clone(),
            view_offset: self.view_offset,
            crew: self.crew.clone(),
            home_base_material: self.home_base_material.clone(),
            home_base_production: self.home_base_production.clone(),
            production_delay: self.production_delay,
            production_unit: self.production_unit,
            color: self.color,
            fog_of_war: self.fog_of_war,
            force_fog_of_war: self.force_fog_of_war,
            show_control_position: self.show_control_position,
            show_control: self.show_control,
            hostility: {
                let mut hostility: Vec<i32> = self.hostility.iter().copied().collect();
                hostility.sort_unstable();
                hostility
            },
            control: self.control,
            extra_data: self.extra_data.clone(),
        }
    }

    /// Declare or revoke hostility toward another player
    /// (C4Player::Hostility set, fed into C4PlayerList::Hostile).
    pub fn set_hostile_towards(&mut self, opponent: i32, hostile: bool) {
        if hostile {
            self.hostility.insert(opponent);
        } else {
            self.hostility.remove(&opponent);
        }
    }

    pub fn is_hostile_towards(&self, opponent: i32) -> bool {
        self.hostility.contains(&opponent)
    }

    pub fn id(&self) -> i32 {
        self.id
    }

    pub fn player_info_id(&self) -> i32 {
        self.player_info_id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn set_name(&mut self, name: impl Into<String>) {
        self.name = name.into();
    }

    pub fn status(&self) -> PlayerStatus {
        self.status
    }

    pub fn set_status(&mut self, status: PlayerStatus) {
        if matches!(status, PlayerStatus::Eliminated) {
            self.surrendered = false;
        }
        self.status = status;
    }

    /// C4Player::Eliminate's one-way state transition and 60-frame retire
    /// delay (C4Player.cpp:2015-2021; C4Constants.h:36).
    pub(crate) fn eliminate(&mut self) -> bool {
        if self.status == PlayerStatus::Eliminated {
            return false;
        }
        self.surrendered = false;
        self.status = PlayerStatus::Eliminated;
        self.retire_delay = 60;
        true
    }

    /// Player::Execute decrements the delay before C4PlayerList retires at
    /// most one ready eliminated player after the player loop.
    pub(crate) fn advance_retire_delay(&mut self) -> bool {
        if self.retire_delay > 0 {
            self.retire_delay -= 1;
        }
        self.status == PlayerStatus::Eliminated && self.retire_delay == 0
    }

    pub fn team(&self) -> Option<i32> {
        self.team
    }

    pub fn set_team(&mut self, team: Option<i32>) {
        self.team = team;
    }

    pub fn fog_of_war(&self) -> bool {
        self.fog_of_war
    }

    pub fn force_fog_of_war(&self) -> bool {
        self.force_fog_of_war
    }

    /// Explicitly enable or disable fog of war. Unlike automatic mouse-control
    /// selection, either value forces the setting (C4Player.cpp:815-824).
    pub fn set_fog_of_war(&mut self, enabled: bool) {
        self.fog_of_war = enabled;
        self.force_fog_of_war = true;
    }

    pub fn surrendered(&self) -> bool {
        self.surrendered
    }

    pub(crate) fn mark_won(&mut self) {
        self.won = true;
    }

    /// C4Player::Evaluate's cooperative settlement-score and profile-time
    /// update. Returns the old/new score pair exactly once.
    pub(crate) fn evaluate(
        &mut self,
        average_value_gain: i32,
        game_time: i32,
    ) -> Option<(i32, i32)> {
        if self.evaluated {
            return None;
        }
        self.won = !matches!(
            self.status,
            PlayerStatus::Eliminated | PlayerStatus::Surrendered
        ) && !self.surrendered;
        let score_old = self.score;
        let success_bonus = if self.won { 100 } else { 0 };
        let final_score = average_value_gain
            .max(0)
            .wrapping_add(success_bonus);
        self.score = self.score.wrapping_add(final_score);
        self.total_playing_time = self
            .total_playing_time
            .wrapping_add(game_time.wrapping_sub(self.game_join_time));
        self.evaluated = true;
        Some((score_old, self.score))
    }

    pub fn set_surrendered(&mut self, surrendered: bool) {
        self.surrendered = surrendered;
        if surrendered {
            self.status = PlayerStatus::Surrendered;
        } else if self.status == PlayerStatus::Surrendered {
            self.status = PlayerStatus::Active;
        }
    }

    pub fn wealth(&self) -> i32 {
        self.wealth
    }

    pub fn set_wealth(&mut self, wealth: i32) {
        self.wealth = wealth.clamp(0, MAX_SET_WEALTH);
    }

    pub fn adjust_wealth(&mut self, delta: i32) -> i32 {
        let updated = (self.wealth as i64 + i64::from(delta))
            .clamp(0, i64::from(MAX_WEALTH_ADJUSTMENT)) as i32;
        self.wealth = updated;
        self.wealth
    }

    pub fn points(&self) -> i32 {
        self.points
    }

    pub fn score(&self) -> i32 {
        self.score
    }

    pub fn total_playing_time(&self) -> i32 {
        self.total_playing_time
    }

    #[doc(hidden)]
    pub fn game_join_time(&self) -> i32 {
        self.game_join_time
    }

    #[doc(hidden)]
    pub fn set_game_join_time(&mut self, game_time: i32) {
        self.game_join_time = game_time;
    }

    pub fn set_points(&mut self, points: i32) -> i32 {
        self.points = points.clamp(MIN_SCORE, MAX_SCORE);
        self.points
    }

    pub fn adjust_points(&mut self, delta: i32) -> i32 {
        let updated = (self.points as i64 + i64::from(delta))
            .clamp(i64::from(MIN_SCORE), i64::from(MAX_SCORE)) as i32;
        self.points = updated;
        self.points
    }

    pub fn value(&self) -> i32 {
        self.value
    }

    pub fn value_gain(&self) -> i32 {
        self.value_gain
    }

    pub fn initial_value(&self) -> i32 {
        self.initial_value
    }

    pub fn objects_owned(&self) -> u32 {
        self.objects_owned
    }

    pub fn update_asset_value(&mut self, value: i32, objects_owned: u32) {
        self.value = value;
        self.objects_owned = objects_owned;
        if !self.initial_value_set {
            self.initial_value = value;
            self.initial_value_set = true;
        }
        let gain = i64::from(self.value) - i64::from(self.initial_value);
        self.value_gain = gain.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
    }

    pub fn reset_initial_value(&mut self) {
        self.initial_value = self.value;
        self.initial_value_set = true;
        self.value_gain = 0;
    }

    pub fn knowledge(&self) -> impl Iterator<Item = &DefinitionId> {
        self.knowledge.iter()
    }

    pub fn grant_knowledge(&mut self, definition_id: DefinitionId) {
        self.knowledge.insert(definition_id);
    }

    pub fn revoke_knowledge(&mut self, definition_id: &DefinitionId) {
        self.knowledge.remove(definition_id);
    }

    pub fn magic(&self) -> impl Iterator<Item = &DefinitionId> {
        self.magic.iter()
    }

    pub fn set_magic(&mut self, magic: Vec<DefinitionId>) {
        self.magic = magic;
    }

    pub fn grant_magic(&mut self, definition_id: DefinitionId) {
        if !self.magic.iter().any(|entry| entry == &definition_id) {
            self.magic.push(definition_id);
        }
    }

    pub fn revoke_magic(&mut self, definition_id: &DefinitionId) {
        if let Some(index) = self.magic.iter().position(|entry| entry == definition_id) {
            self.magic.remove(index);
        }
    }

    pub fn inventory(&self) -> &HashMap<DefinitionId, u32> {
        &self.inventory
    }

    pub fn set_inventory_item(&mut self, definition_id: DefinitionId, quantity: u32) {
        if quantity == 0 {
            self.inventory.remove(&definition_id);
        } else {
            self.inventory.insert(definition_id, quantity);
        }
    }

    pub fn adjust_inventory_item(&mut self, definition_id: DefinitionId, delta: i32) -> u32 {
        let current = self.inventory.get(&definition_id).copied().unwrap_or(0);
        let updated = if delta >= 0 {
            current.saturating_add(delta as u32)
        } else {
            let decrease = delta.checked_abs().unwrap_or(i32::MAX) as u32;
            current.saturating_sub(decrease)
        };
        if updated == 0 {
            self.inventory.remove(&definition_id);
        } else {
            self.inventory.insert(definition_id.clone(), updated);
        }
        updated
    }

    pub fn cursor(&self) -> Option<ObjectId> {
        self.cursor
    }

    pub fn set_cursor(&mut self, cursor: Option<ObjectId>) {
        self.cursor = cursor;
    }

    pub fn viewports(&self) -> &[PlayerViewport] {
        &self.viewports
    }

    pub fn view_offset(&self) -> Vector2 {
        self.view_offset
    }

    pub fn set_view_offset(&mut self, offset: Vector2) {
        self.view_offset = offset;
    }

    pub fn replace_viewports(&mut self, viewports: Vec<PlayerViewport>) {
        self.viewports = viewports;
    }

    pub fn set_viewport(&mut self, index: usize, viewport: PlayerViewport) {
        if self.viewports.len() <= index {
            self.viewports
                .resize_with(index + 1, || PlayerViewport::new(Vector2::ZERO));
        }
        self.viewports[index] = viewport;
    }

    pub fn crew(&self) -> &[ObjectId] {
        &self.crew
    }

    pub fn color_index(&self) -> i32 {
        self.color_index
    }

    pub fn set_color_index(&mut self, index: i32) {
        self.color_index = index;
    }

    pub fn position_index(&self) -> i32 {
        self.position_index
    }

    pub fn set_position_index(&mut self, index: i32) {
        self.position_index = index;
    }

    pub fn set_crew(&mut self, crew: Vec<ObjectId>) {
        self.crew = crew;
    }

    pub fn home_base_material(&self) -> &HashMap<DefinitionId, u32> {
        &self.home_base_material
    }

    pub fn set_home_base_material(&mut self, material: HashMap<DefinitionId, u32>) {
        self.home_base_material = material
            .into_iter()
            .filter(|(_, count)| *count > 0)
            .collect();
    }

    pub fn home_base_production(&self) -> &HashMap<DefinitionId, u32> {
        &self.home_base_production
    }

    pub fn set_home_base_production(&mut self, production: HashMap<DefinitionId, u32>) {
        self.home_base_production = production
            .into_iter()
            .filter(|(_, count)| *count > 0)
            .collect();
    }

    pub fn adjust_home_base_material(&mut self, definition_id: DefinitionId, delta: i32) -> u32 {
        if delta >= 0 {
            let entry = self
                .home_base_material
                .entry(definition_id.clone())
                .or_insert(0);
            let added = delta as u32;
            *entry = entry.saturating_add(added).min(MAX_HOME_BASE_MATERIAL);
            if *entry == 0 {
                self.home_base_material.remove(&definition_id);
                0
            } else {
                *entry
            }
        } else {
            let decrease = delta.saturating_abs() as u32;
            if let Some(entry) = self.home_base_material.get_mut(&definition_id) {
                if *entry <= decrease {
                    // C4Player::Buy calls DecreaseIDCount(id, false): the
                    // C4IDList slot survives at zero so the permanent Buy
                    // menu keeps its row (C4Player.cpp:850-852;
                    // C4IDList.cpp:121-137).
                    *entry = 0;
                    0
                } else {
                    *entry -= decrease;
                    *entry
                }
            } else {
                0
            }
        }
    }

    pub fn adjust_home_base_production(&mut self, definition_id: DefinitionId, delta: i32) -> u32 {
        if delta >= 0 {
            let entry = self
                .home_base_production
                .entry(definition_id.clone())
                .or_insert(0);
            let added = delta as u32;
            *entry = entry.saturating_add(added);
            if *entry == 0 {
                self.home_base_production.remove(&definition_id);
                0
            } else {
                *entry
            }
        } else {
            let decrease = delta.saturating_abs() as u32;
            if let Some(entry) = self.home_base_production.get_mut(&definition_id) {
                if *entry <= decrease {
                    self.home_base_production.remove(&definition_id);
                    0
                } else {
                    *entry -= decrease;
                    *entry
                }
            } else {
                0
            }
        }
    }

    pub fn advance_home_base_production(&mut self) -> bool {
        if self.home_base_production.is_empty() {
            return false;
        }
        self.production_delay = self.production_delay.saturating_add(1);
        if self.production_delay < 60 {
            return false;
        }
        self.production_delay = 0;
        self.production_unit = self.production_unit.wrapping_add(1);
        let mut changed = false;
        for (definition_id, &count) in self.home_base_production.iter() {
            if count == 0 {
                continue;
            }
            let frequency = (11_i32 - count as i32).clamp(1, 10) as u32;
            if frequency == 0 {
                continue;
            }
            if self.production_unit % frequency == 0 {
                let entry = self
                    .home_base_material
                    .entry(definition_id.clone())
                    .or_insert(0);
                if *entry < MAX_HOME_BASE_MATERIAL {
                    *entry += 1;
                    changed = true;
                }
            }
        }
        changed
    }

    pub fn sync_home_base_material_from(&mut self, other: &Player) {
        self.home_base_material = other.home_base_material.clone();
    }

    pub fn production_delay(&self) -> u32 {
        self.production_delay
    }

    pub fn set_production_delay(&mut self, delay: u32) {
        self.production_delay = delay;
    }

    pub fn production_unit(&self) -> u32 {
        self.production_unit
    }

    pub fn set_production_unit(&mut self, unit: u32) {
        self.production_unit = unit;
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn is_zero_i32(value: &i32) -> bool {
    *value == 0
}

fn is_zero_vector(value: &Vector2) -> bool {
    *value == Vector2::ZERO
}

#[derive(Debug, Clone)]
pub struct PlayerConfig {
    id: i32,
    player_info_id: i32,
    name: String,
    status: PlayerStatus,
    team: Option<i32>,
    surrendered: bool,
    wealth: i32,
    points: i32,
    score: i32,
    total_playing_time: i32,
    value: i32,
    initial_value: i32,
    value_gain: i32,
    objects_owned: u32,
    initial_value_set: bool,
    knowledge: Vec<DefinitionId>,
    magic: Vec<DefinitionId>,
    inventory: HashMap<DefinitionId, u32>,
    cursor: Option<ObjectId>,
    viewports: Vec<PlayerViewport>,
    home_base_material: HashMap<DefinitionId, u32>,
    home_base_production: HashMap<DefinitionId, u32>,
    production_delay: u32,
    production_unit: u32,
    color: Option<RgbColor>,
}

impl PlayerConfig {
    pub fn new(id: i32, name: impl Into<String>) -> Self {
        Self {
            id,
            player_info_id: 0,
            name: name.into(),
            status: PlayerStatus::Active,
            team: None,
            surrendered: false,
            wealth: 0,
            points: 0,
            score: 0,
            total_playing_time: 0,
            value: 0,
            initial_value: 0,
            value_gain: 0,
            objects_owned: 0,
            initial_value_set: false,
            knowledge: Vec::new(),
            magic: Vec::new(),
            inventory: HashMap::new(),
            cursor: None,
            viewports: Vec::new(),
            home_base_material: HashMap::new(),
            home_base_production: HashMap::new(),
            production_delay: 0,
            production_unit: 0,
            color: None,
        }
    }

    pub fn with_status(mut self, status: PlayerStatus) -> Self {
        self.status = status;
        self
    }

    pub fn with_player_info_id(mut self, player_info_id: i32) -> Self {
        self.player_info_id = player_info_id;
        self
    }

    pub fn with_team(mut self, team: Option<i32>) -> Self {
        self.team = team;
        self
    }

    pub fn with_surrendered(mut self, surrendered: bool) -> Self {
        self.surrendered = surrendered;
        self
    }

    pub fn with_wealth(mut self, wealth: i32) -> Self {
        self.wealth = wealth;
        self
    }

    pub fn with_points(mut self, points: i32) -> Self {
        self.points = points;
        self
    }

    pub fn with_score(mut self, score: i32) -> Self {
        self.score = score;
        self
    }

    pub fn with_total_playing_time(mut self, total_playing_time: i32) -> Self {
        self.total_playing_time = total_playing_time;
        self
    }

    pub fn with_initial_value(mut self, value: i32) -> Self {
        self.initial_value = value;
        self.value = value;
        self.value_gain = 0;
        self.initial_value_set = true;
        self
    }

    pub fn with_objects_owned(mut self, objects_owned: u32) -> Self {
        self.objects_owned = objects_owned;
        self
    }

    pub fn with_knowledge<I>(mut self, knowledge: I) -> Self
    where
        I: IntoIterator<Item = DefinitionId>,
    {
        self.knowledge = knowledge.into_iter().collect();
        self
    }

    pub fn with_magic<I>(mut self, magic: I) -> Self
    where
        I: IntoIterator<Item = DefinitionId>,
    {
        self.magic = magic.into_iter().collect();
        self
    }

    pub fn with_inventory(mut self, inventory: HashMap<DefinitionId, u32>) -> Self {
        self.inventory = inventory;
        self
    }

    pub fn with_cursor(mut self, cursor: Option<ObjectId>) -> Self {
        self.cursor = cursor;
        self
    }

    pub fn with_viewports<I>(mut self, viewports: I) -> Self
    where
        I: IntoIterator<Item = PlayerViewport>,
    {
        self.viewports = viewports.into_iter().collect();
        self
    }

    pub fn with_home_base_material(mut self, material: HashMap<DefinitionId, u32>) -> Self {
        self.home_base_material = material
            .into_iter()
            .filter(|(_, count)| *count > 0)
            .collect();
        self
    }

    pub fn with_home_base_production(mut self, production: HashMap<DefinitionId, u32>) -> Self {
        self.home_base_production = production
            .into_iter()
            .filter(|(_, count)| *count > 0)
            .collect();
        self
    }

    pub fn with_production_delay(mut self, delay: u32) -> Self {
        self.production_delay = delay;
        self
    }

    pub fn with_production_unit(mut self, unit: u32) -> Self {
        self.production_unit = unit;
        self
    }

    pub fn with_color(mut self, color: Option<RgbColor>) -> Self {
        self.color = color;
        self
    }

    pub fn id(&self) -> i32 {
        self.id
    }

    pub(crate) fn player_info_id(&self) -> i32 {
        self.player_info_id
    }

    pub fn build(self) -> Player {
        Player::from_config(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_fog_of_war_forces_override_and_persists_both_flags() {
        // C4Player::SetFoW (C4Player.cpp:815-824) always forces the setting,
        // including when disabling fog of war; CompileFunc persists both flags
        // as FogOfWar and ForceFogOfWar (C4Player.cpp:1580-1581).
        let mut player = Player::new(1, "Player");
        assert!(!player.fog_of_war());
        assert!(!player.force_fog_of_war());
        let configured = PlayerConfig::new(2, "Configured").build();
        assert!(!configured.fog_of_war());
        assert!(!configured.force_fog_of_war());

        player.set_fog_of_war(true);
        let enabled = player.to_state();
        assert!(enabled.fog_of_war);
        assert!(enabled.force_fog_of_war);

        let mut restored = Player::from_state(enabled);
        restored.set_fog_of_war(false);
        let disabled = restored.to_state();
        assert!(!disabled.fog_of_war);
        assert!(disabled.force_fog_of_war);
    }

    #[test]
    fn persisted_fog_of_war_flags_default_to_false_when_absent() {
        let state: PlayerState = serde_json::from_str(r#"{"id":2}"#)
            .unwrap_or_else(|error| panic!("valid legacy player state: {error}"));

        assert!(!state.fog_of_war);
        assert!(!state.force_fog_of_war);
    }

    #[test]
    fn evaluation_fields_default_when_absent_and_round_trip() {
        // C4Player persists its distinct PlayerInfo ID and Evaluated flag
        // (C4Player.cpp:1567-1571), while Score and TotalPlayingTime belong
        // to C4PlayerInfoCore (C4InfoCore.cpp:156-160). Keep all four distinct
        // from the in-round player number and Points fields.
        let legacy: PlayerState = serde_json::from_str(r#"{"id":2,"points":17}"#)
            .unwrap_or_else(|error| panic!("valid legacy player state: {error}"));
        assert_eq!(legacy.player_info_id, 0);
        assert!(!legacy.won);
        assert!(!legacy.evaluated);
        assert_eq!(legacy.score, 0);
        assert_eq!(legacy.total_playing_time, 0);

        let evaluated = PlayerState {
            id: 2,
            player_info_id: 41,
            won: true,
            evaluated: true,
            score: 250,
            total_playing_time: 1_234,
            ..PlayerState::default()
        };
        assert_eq!(Player::from_state(evaluated.clone()).to_state(), evaluated);
    }

    #[test]
    fn default_evaluation_fields_do_not_change_serialized_player_shape() {
        let value = serde_json::to_value(PlayerState {
            id: 2,
            ..PlayerState::default()
        })
        .unwrap_or_else(|error| panic!("player state serializes: {error}"));

        for field in [
            "player_info_id",
            "won",
            "evaluated",
            "score",
            "total_playing_time",
        ] {
            assert!(value.get(field).is_none(), "unexpected default field {field}");
        }
    }

    #[test]
    fn player_config_carries_profile_values_but_not_the_join_clock() {
        // C4Player::Init copies the linked C4PlayerInfo ID before loading the
        // C4PlayerInfoCore profile values (C4Player.cpp:246-276). The local
        // GameJoinTime baseline is runtime-only and starts at zero until Init
        // assigns Game.Time (C4Player.cpp:389-390,1075).
        let player = PlayerConfig::new(2, "Profile")
            .with_player_info_id(41)
            .with_score(250)
            .with_total_playing_time(1_234)
            .build();

        let state = player.to_state();
        assert_eq!(state.player_info_id, 41);
        assert_eq!(state.score, 250);
        assert_eq!(state.total_playing_time, 1_234);
        assert_eq!(player.game_join_time(), 0);

        let restored = Player::from_state(state);
        assert_eq!(restored.game_join_time(), 0);
    }
}
