//! `scenario` — moved verbatim from the parent module.
//!
//! Structural only: same crate, same items, same bodies.

use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::scenario) struct LegacyHead {
    pub(in crate::scenario) icon: i32,
    pub(in crate::scenario) title: String,
    pub(in crate::scenario) loader: String,
    pub(in crate::scenario) font: String,
    pub(in crate::scenario) version: [i32; 5],
    pub(in crate::scenario) difficulty: i32,
    pub(in crate::scenario) max_player: i32,
    pub(in crate::scenario) max_player_league: i32,
    pub(in crate::scenario) min_player: i32,
    pub(in crate::scenario) save_game: i32,
    pub(in crate::scenario) replay: i32,
    pub(in crate::scenario) film: i32,
    pub(in crate::scenario) disable_mouse: i32,
    pub(in crate::scenario) no_initialize: i32,
    pub(in crate::scenario) random_seed: i32,
    pub(in crate::scenario) forced_auto_context_menu: i32,
    pub(in crate::scenario) forced_control_style: i32,
    pub(in crate::scenario) engine: String,
    pub(in crate::scenario) mission_access: String,
    pub(in crate::scenario) network_game: bool,
    pub(in crate::scenario) network_runtime_join: bool,
    pub(in crate::scenario) forced_gfx_mode: i32,
    pub(in crate::scenario) forced_fair_crew: i32,
    pub(in crate::scenario) fair_crew_strength: i32,
    pub(in crate::scenario) origin: Option<String>,
}

impl Default for LegacyHead {
    fn default() -> Self {
        Self {
            icon: 18,
            title: "Default Title".to_string(),
            loader: String::new(),
            font: String::new(),
            version: [0; 5],
            difficulty: 0,
            max_player: 12,
            max_player_league: 12,
            min_player: 0,
            save_game: 0,
            replay: 0,
            film: 0,
            disable_mouse: 0,
            no_initialize: 0,
            random_seed: 0,
            forced_auto_context_menu: -1,
            forced_control_style: -1,
            engine: String::new(),
            mission_access: String::new(),
            network_game: false,
            network_runtime_join: false,
            forced_gfx_mode: 0,
            forced_fair_crew: 0,
            fair_crew_strength: 0,
            origin: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::scenario) struct LegacyDefinitions {
    pub(in crate::scenario) local_only: bool,
    pub(in crate::scenario) allow_user_change: bool,
    pub(in crate::scenario) definitions: Vec<String>,
    /// Exact strings retained in C4Scenario for StdCompiler reflection.
    /// `definitions` remains path-normalized for the Rust resource resolver.
    pub(in crate::scenario) reflected_definitions: Option<Vec<String>>,
    pub(in crate::scenario) skip_defs: LegacyIdList,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::scenario) struct LegacyRealism {
    pub(in crate::scenario) construction_needs_material: bool,
    pub(in crate::scenario) structures_need_energy: bool,
    pub(in crate::scenario) value_overloads: LegacyIdList,
    pub(in crate::scenario) landscape_push_pull: i32,
    pub(in crate::scenario) landscape_insert_thrust: i32,
    pub(in crate::scenario) base_functionality: i32,
    pub(in crate::scenario) base_regenerate_energy_price: i32,
}

impl Default for LegacyRealism {
    fn default() -> Self {
        Self {
            construction_needs_material: false,
            structures_need_energy: true,
            value_overloads: Vec::new(),
            landscape_push_pull: 0,
            landscape_insert_thrust: 0,
            base_functionality: BASEFUNC_DEFAULT,
            base_regenerate_energy_price: BASE_REGENERATE_ENERGY_PRICE,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::scenario) struct LegacyGame {
    pub(in crate::scenario) mode: i32,
    pub(in crate::scenario) elimination: i32,
    pub(in crate::scenario) cooperative_goal: i32,
    pub(in crate::scenario) create_objects: LegacyIdList,
    pub(in crate::scenario) clear_objects: LegacyIdList,
    pub(in crate::scenario) clear_materials: LegacyNameList,
    pub(in crate::scenario) value_gain: i32,
    pub(in crate::scenario) enable_remove_flag: bool,
    pub(in crate::scenario) realism: LegacyRealism,
    pub(in crate::scenario) goals: LegacyIdList,
    pub(in crate::scenario) rules: LegacyIdList,
    pub(in crate::scenario) fow_color: u32,
}

impl Default for LegacyGame {
    fn default() -> Self {
        Self {
            mode: 0,
            elimination: 1,
            cooperative_goal: 0,
            create_objects: Vec::new(),
            clear_objects: Vec::new(),
            clear_materials: Vec::new(),
            value_gain: 0,
            enable_remove_flag: false,
            realism: LegacyRealism::default(),
            goals: Vec::new(),
            rules: Vec::new(),
            fow_color: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::scenario) struct LegacyPlayer {
    pub(in crate::scenario) standard_crew: Option<String>,
    pub(in crate::scenario) clonks: LegacyC4SVal,
    pub(in crate::scenario) wealth: LegacyC4SVal,
    pub(in crate::scenario) position: [i32; 2],
    pub(in crate::scenario) enforce_position: i32,
    pub(in crate::scenario) crew: LegacyIdList,
    pub(in crate::scenario) buildings: LegacyIdList,
    pub(in crate::scenario) vehicles: LegacyIdList,
    pub(in crate::scenario) material: LegacyIdList,
    pub(in crate::scenario) knowledge: LegacyIdList,
    pub(in crate::scenario) home_base_material: LegacyIdList,
    pub(in crate::scenario) home_base_production: LegacyIdList,
    pub(in crate::scenario) magic: LegacyIdList,
}

impl Default for LegacyPlayer {
    fn default() -> Self {
        Self {
            standard_crew: None,
            clonks: LegacyC4SVal::new(1, 0, 1, 10),
            wealth: LegacyC4SVal::new(0, 0, 0, 250),
            position: [-1, -1],
            enforce_position: 0,
            crew: Vec::new(),
            buildings: Vec::new(),
            vehicles: Vec::new(),
            material: Vec::new(),
            knowledge: Vec::new(),
            home_base_material: Vec::new(),
            home_base_production: Vec::new(),
            magic: Vec::new(),
        }
    }
}

/// `C4S_MaxPlayer` (C4Scenario.h): four `[PlayerN]` start slots; a joining
/// player uses slot `Number % C4S_MaxPlayer` (C4Player.cpp:673).
pub const MAX_PLAYER_STARTS: usize = 4;

/// One `C4SPlrStart` slot (compiled at C4Scenario.cpp:276-291), retained
/// after `Scenario::apply` because `C4Player::ScenarioInit`
/// (C4Player.cpp:670-777) consumes it at JOIN time, not load time. ID lists
/// keep their file order — placement iterates them in order, drawing from
/// the synced RNG per entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerStart {
    /// `StandardCrew` (old crew spec; C4ID_None when absent).
    pub native_crew: Option<String>,
    /// `Clonks` — the old-spec crew COUNT C4SVal.
    pub crew_count: LegacyC4SVal,
    /// `Wealth`.
    pub wealth: LegacyC4SVal,
    /// `Position` (map coordinates; -1 = unset).
    pub position: [i32; 2],
    /// `EnforcePosition`.
    pub enforce_position: bool,
    /// `Crew` — the new-spec ready-crew ID list.
    pub ready_crew: Vec<(String, i32)>,
    /// `Buildings`.
    pub ready_base: Vec<(String, i32)>,
    /// `Vehicles`.
    pub ready_vehic: Vec<(String, i32)>,
    /// `Material`.
    pub ready_material: Vec<(String, i32)>,
    /// `Knowledge`.
    pub build_knowledge: Vec<(String, i32)>,
    /// `HomeBaseMaterial`.
    pub home_base_material: Vec<(String, i32)>,
    /// `HomeBaseProduction`.
    pub home_base_production: Vec<(String, i32)>,
    /// `Magic`.
    pub magic: Vec<(String, i32)>,
}

impl Default for PlayerStart {
    fn default() -> Self {
        PlayerStart::from_legacy(&LegacyPlayer::default())
    }
}

impl PlayerStart {
    pub(in crate::scenario) fn from_legacy(player: &LegacyPlayer) -> Self {
        // C4IDList::Entry starts at count zero and only compiles a count when
        // the textual entry has an `=` separator (C4IDList.cpp:239-253).
        let id_list = |entries: &LegacyIdList| {
            entries
                .iter()
                .map(|entry| (entry.id.clone(), entry.count.unwrap_or(0)))
                .collect()
        };
        Self {
            native_crew: player.standard_crew.clone(),
            crew_count: player.clonks,
            wealth: player.wealth,
            position: player.position,
            enforce_position: player.enforce_position != 0,
            ready_crew: id_list(&player.crew),
            ready_base: id_list(&player.buildings),
            ready_vehic: id_list(&player.vehicles),
            ready_material: id_list(&player.material),
            build_knowledge: id_list(&player.knowledge),
            home_base_material: id_list(&player.home_base_material),
            home_base_production: id_list(&player.home_base_production),
            magic: id_list(&player.magic),
        }
    }

    pub(in crate::scenario) fn slots_from_legacy(players: &[LegacyPlayer]) -> Vec<PlayerStart> {
        (0..MAX_PLAYER_STARTS)
            .map(|index| {
                players
                    .get(index)
                    .map(PlayerStart::from_legacy)
                    .unwrap_or_default()
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::scenario) struct LegacyLandscape {
    pub(in crate::scenario) exact_landscape: bool,
    pub(in crate::scenario) vegetation: LegacyIdList,
    pub(in crate::scenario) vegetation_level: LegacyC4SVal,
    pub(in crate::scenario) in_earth: LegacyIdList,
    pub(in crate::scenario) in_earth_level: LegacyC4SVal,
    pub(in crate::scenario) sky: Option<String>,
    pub(in crate::scenario) sky_fade: [i32; 6],
    pub(in crate::scenario) no_sky: bool,
    pub(in crate::scenario) bottom_open: bool,
    pub(in crate::scenario) top_open: bool,
    pub(in crate::scenario) left_open: i32,
    pub(in crate::scenario) right_open: i32,
    pub(in crate::scenario) auto_scan_side_open: bool,
    pub(in crate::scenario) map_width: LegacyC4SVal,
    pub(in crate::scenario) map_height: LegacyC4SVal,
    pub(in crate::scenario) map_zoom: LegacyC4SVal,
    pub(in crate::scenario) amplitude: LegacyC4SVal,
    pub(in crate::scenario) phase: LegacyC4SVal,
    pub(in crate::scenario) period: LegacyC4SVal,
    pub(in crate::scenario) random: LegacyC4SVal,
    pub(in crate::scenario) material: String,
    pub(in crate::scenario) liquid: String,
    pub(in crate::scenario) liquid_level: LegacyC4SVal,
    pub(in crate::scenario) map_player_extend: bool,
    pub(in crate::scenario) layers: LegacyNameList,
    pub(in crate::scenario) gravity: LegacyC4SVal,
    pub(in crate::scenario) no_scan: bool,
    pub(in crate::scenario) keep_map_creator: bool,
    pub(in crate::scenario) sky_scroll_mode: i32,
    pub(in crate::scenario) new_style_landscape: i32,
    pub(in crate::scenario) fow_resolution: i32,
    pub(in crate::scenario) shade_materials: bool,
}

impl Default for LegacyLandscape {
    fn default() -> Self {
        Self {
            exact_landscape: false,
            vegetation: Vec::new(),
            vegetation_level: LegacyC4SVal::new(50, 30, 0, 100),
            in_earth: Vec::new(),
            in_earth_level: LegacyC4SVal::new(50, 0, 0, 100),
            sky: None,
            sky_fade: [0; 6],
            no_sky: false,
            bottom_open: false,
            top_open: true,
            left_open: 0,
            right_open: 0,
            auto_scan_side_open: true,
            map_width: LegacyC4SVal::new(100, 0, 64, 250),
            map_height: LegacyC4SVal::new(50, 0, 40, 250),
            map_zoom: LegacyC4SVal::new(10, 0, 5, 15),
            amplitude: LegacyC4SVal::new(0, 0, 0, 100),
            phase: LegacyC4SVal::new(50, 0, 0, 100),
            period: LegacyC4SVal::new(15, 0, 0, 100),
            random: LegacyC4SVal::new(0, 0, 0, 100),
            material: "Earth".to_string(),
            liquid: "Water".to_string(),
            liquid_level: LegacyC4SVal::new(0, 0, 0, 100),
            map_player_extend: false,
            layers: Vec::new(),
            gravity: LegacyC4SVal::new(100, 0, 10, 200),
            no_scan: false,
            keep_map_creator: false,
            sky_scroll_mode: 0,
            new_style_landscape: 0,
            fow_resolution: DEFAULT_FOW_RESOLUTION,
            shade_materials: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::scenario) struct LegacyWeather {
    pub(in crate::scenario) climate: LegacyC4SVal,
    pub(in crate::scenario) start_season: LegacyC4SVal,
    pub(in crate::scenario) year_speed: LegacyC4SVal,
    pub(in crate::scenario) rain: LegacyC4SVal,
    pub(in crate::scenario) wind: LegacyC4SVal,
    pub(in crate::scenario) lightning: LegacyC4SVal,
    pub(in crate::scenario) precipitation: String,
    pub(in crate::scenario) no_gamma: bool,
}

impl Default for LegacyWeather {
    fn default() -> Self {
        Self {
            climate: LegacyC4SVal::new(50, 10, 0, 100),
            start_season: LegacyC4SVal::new(50, 50, 0, 100),
            year_speed: LegacyC4SVal::new(50, 0, 0, 100),
            rain: LegacyC4SVal::new(0, 0, 0, 100),
            wind: LegacyC4SVal::new(0, 70, -100, 100),
            lightning: LegacyC4SVal::new(0, 0, 0, 100),
            precipitation: "Water".to_string(),
            no_gamma: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::scenario) struct LegacyDisasters {
    pub(in crate::scenario) meteorite: LegacyC4SVal,
    pub(in crate::scenario) volcano: LegacyC4SVal,
    pub(in crate::scenario) earthquake: LegacyC4SVal,
}

impl Default for LegacyDisasters {
    fn default() -> Self {
        Self {
            meteorite: LegacyC4SVal::new(0, 0, 0, 100),
            volcano: LegacyC4SVal::new(0, 0, 0, 100),
            earthquake: LegacyC4SVal::new(0, 0, 0, 100),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::scenario) struct LegacyAnimals {
    pub(in crate::scenario) free_life: LegacyIdList,
    pub(in crate::scenario) earth_nest: LegacyIdList,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::scenario) struct LegacyEnvironment {
    pub(in crate::scenario) objects: LegacyIdList,
}

/// One primitive exposed by `GetValByStdCompiler` while reflecting
/// `Game.C4S` (`C4Script.cpp:3997-4148,4244-4250`).  Keep this distinct from
/// `clonk_script::Value`: scenario loading must not know about VM ownership or
/// string interning, and the host boundary performs the final conversion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ScenarioValue {
    Int(i32),
    Bool(bool),
    String(String),
    C4Id(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ScenarioValueEntry {
    pub(crate) name: String,
    pub(crate) values: Vec<ScenarioValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ScenarioValueSection {
    pub(crate) name: String,
    pub(crate) entries: Vec<ScenarioValueEntry>,
}
