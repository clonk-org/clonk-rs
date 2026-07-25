//! `scenario` — moved verbatim from the parent module.
//!
//! Structural only: same crate, same items, same bodies.

use super::*;

impl LegacyHead {
    fn apply_entries(&mut self, entries: &[(String, String)]) -> Result<(), ScenarioError> {
        let has_max_player_league = entries
            .iter()
            .any(|(key, _)| key.eq_ignore_ascii_case("MaxPlayerLeague"));
        let mut seen_fields = HashSet::new();
        for (key, value) in entries {
            let key_lower = key.to_ascii_lowercase();
            // StdCompilerINIRead resolves the first same-name child. A later
            // duplicate neither overwrites it nor exposes a parse failure.
            if !seen_fields.insert(key_lower.clone()) {
                continue;
            }
            let raw = value.trim();
            match key_lower.as_str() {
                "icon" => {
                    self.icon = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "title" => {
                    if !raw.is_empty() {
                        self.title = raw.to_string();
                    }
                }
                "loader" => {
                    if !raw.is_empty() {
                        self.loader = raw.to_string();
                    }
                }
                "font" => {
                    if !raw.is_empty() {
                        self.font = raw.to_string();
                    }
                }
                "version" => {
                    self.version = parse_legacy_version(key, raw)?;
                }
                "difficulty" => {
                    self.difficulty = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "maxplayer" => {
                    self.max_player = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "maxplayerleague" => {
                    self.max_player_league = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "minplayer" => {
                    self.min_player = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "savegame" => {
                    self.save_game = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "replay" => {
                    self.replay = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "film" => {
                    self.film = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "disablemouse" => {
                    self.disable_mouse = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "noinitialize" => {
                    self.no_initialize = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "randomseed" => {
                    self.random_seed = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "forcedautocontextmenu" => {
                    self.forced_auto_context_menu = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "forcedautostopcontrol" => {
                    self.forced_control_style = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "engine" => {
                    if !raw.is_empty() {
                        self.engine = raw.to_string();
                    }
                }
                "missionaccess" => {
                    if !raw.is_empty() {
                        self.mission_access = truncate_legacy_c4_string(raw.to_string(), 512);
                    }
                }
                "networkgame" => {
                    self.network_game = parse_bool_field(key, raw)?;
                }
                "networkruntimejoin" => {
                    self.network_runtime_join = parse_bool_field(key, raw)?;
                }
                "forcedgfxmode" => {
                    self.forced_gfx_mode = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "forcednocrew" => {
                    self.forced_fair_crew = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "defcrewstrength" => {
                    self.fair_crew_strength = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "origin" => {
                    if raw.is_empty() {
                        self.origin = None;
                    } else {
                        self.origin = Some(raw.to_string());
                    }
                }
                _ => {}
            }
        }
        // mkNamingAdapt(MaxPlayerLeague, ..., MaxPlayer) is compiled after
        // MaxPlayer, so an omitted league limit inherits the parsed regular
        // limit rather than C4S_MaxPlayerDefault.
        if !has_max_player_league {
            self.max_player_league = self.max_player;
        }
        Ok(())
    }
}

impl LegacyDefinitions {
    fn apply_entries(&mut self, entries: &[(String, String)]) -> Result<(), ScenarioError> {
        // StdCompilerINIRead keeps the first same-name value in a section.
        // In particular, later scalar duplicates must neither overwrite a
        // valid value nor surface a parse error that C++ never observes.
        if let Some((key, value)) = entries.iter().find(|(key, _)| key == "LocalOnly") {
            self.local_only = parse_bool_field(key, value.trim())?;
        }
        if let Some((key, value)) = entries.iter().find(|(key, _)| key == "AllowUserChange") {
            self.allow_user_change = parse_bool_field(key, value.trim())?;
        }
        if let Some((key, value)) = entries.iter().find(|(key, _)| key == "SkipDefs") {
            self.skip_defs = parse_legacy_id_list(key, value.trim())?;
        }
        // C4SDefinitions::CompileFunc first compiles the comma-separated
        // modern container. Only when that is empty does it query exactly
        // Definition1 through Definition10, one literal module per slot.
        let reflected_definitions = entries
            .iter()
            .find(|(key, _)| key == "Definitions")
            .map(|(_, value)| clonk_resources::scenario::parse_c4s_string_list(value))
            .unwrap_or_default();
        let mut definitions = reflected_definitions
            .iter()
            .map(|value| value.replace('\\', "/"))
            .collect::<Vec<_>>();
        let mut reflected_definitions = reflected_definitions;
        if definitions.is_empty() {
            for index in 1..=10 {
                let key = format!("Definition{index}");
                let Some(raw) = entries
                    .iter()
                    .find(|(entry_key, _)| entry_key == &key)
                    // mkStringAdaptA uses RCT_All: skip leading spaces/tabs,
                    // then retain every byte through the line ending,
                    // including quotes and trailing spaces.
                    .map(|(_, value)| value.trim_start_matches([' ', '\t'].as_ref()))
                    .filter(|value| !value.is_empty())
                else {
                    continue;
                };
                reflected_definitions.push(raw.to_string());
                definitions.push(normalize_definition_path(raw));
            }
        }
        self.reflected_definitions = Some(reflected_definitions);
        self.definitions = definitions;
        Ok(())
    }
}

impl LegacyGame {
    fn clear_old_goals(&mut self) {
        self.create_objects.clear();
        self.clear_objects.clear();
        self.clear_materials.clear();
        self.value_gain = 0;
    }

    /// C4SGame::ConvertGoals, including its in-place selector resets and
    /// ClearOldGoals side effects (C4Scenario.cpp:503-545).
    fn convert_goals_after_load(&mut self) {
        if matches!(self.mode, 1 | 2) {
            set_legacy_id_count(&mut self.goals, "MELE", 1);
            self.clear_old_goals();
        }
        self.mode = 0;

        match self.cooperative_goal {
            1 => {
                set_legacy_id_count(&mut self.goals, "GLDM", 1);
                self.clear_old_goals();
            }
            2 => {
                set_legacy_id_count(&mut self.goals, "MNTK", 1);
                self.clear_old_goals();
            }
            3 => {
                let value_gain = (self.value_gain / 100).max(1);
                set_legacy_id_count(&mut self.goals, "VALG", value_gain);
                self.clear_old_goals();
            }
            _ => {}
        }
        self.cooperative_goal = 0;

        if self.realism.construction_needs_material {
            set_legacy_id_count(&mut self.rules, "CNMT", 1);
        }
        self.realism.construction_needs_material = false;
        if self.realism.structures_need_energy {
            set_legacy_id_count(&mut self.rules, "ENRG", 1);
        }
        self.realism.structures_need_energy = false;
        if self.enable_remove_flag {
            set_legacy_id_count(&mut self.rules, "FGRV", 1);
        }
        self.enable_remove_flag = false;

        match self.elimination {
            0 => set_legacy_id_count(&mut self.rules, "KILC", 1),
            2 => set_legacy_id_count(&mut self.rules, "CTFL", 1),
            _ => {}
        }
        self.elimination = 1;

        if legacy_id_count(&self.rules, "CTFL") != 0 {
            set_legacy_id_count(&mut self.rules, "FGRV", 1);
        }
    }

    fn apply_entries(&mut self, entries: &[(String, String)]) -> Result<(), ScenarioError> {
        for (key, value) in entries {
            let key_lower = key.to_ascii_lowercase();
            let raw = value.trim();
            match key_lower.as_str() {
                "mode" => {
                    self.mode = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "elimination" => {
                    self.elimination = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "cooperativegoal" => {
                    self.cooperative_goal = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "createobjects" => {
                    self.create_objects = parse_legacy_id_list(key, raw)?;
                }
                "clearobjects" => {
                    self.clear_objects = parse_legacy_id_list(key, raw)?;
                }
                "clearmaterials" => {
                    self.clear_materials = parse_legacy_name_list(key, value)?;
                }
                "valuegain" => {
                    self.value_gain = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "enableremoveflag" => {
                    self.enable_remove_flag = parse_bool_field(key, raw)?;
                }
                "structneedmaterial" => {
                    self.realism.construction_needs_material = parse_bool_field(key, raw)?;
                }
                "structneedenergy" => {
                    self.realism.structures_need_energy = parse_bool_field(key, raw)?;
                }
                "valueoverloads" => {
                    self.realism.value_overloads = parse_legacy_id_list(key, raw)?;
                }
                "landscapepushpull" => {
                    self.realism.landscape_push_pull = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "landscapeinsertthrust" => {
                    self.realism.landscape_insert_thrust = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "basefunctionality" => {
                    self.realism.base_functionality = parse_base_functionality(key, raw)?;
                }
                "baseregenerateenergyprice" => {
                    self.realism.base_regenerate_energy_price = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "goals" => {
                    self.goals = parse_legacy_id_list(key, raw)?;
                }
                "rules" => {
                    self.rules = parse_legacy_id_list(key, raw)?;
                }
                "fowcolor" => {
                    self.fow_color = parse_std_u32(raw).ok_or_else(|| {
                        ScenarioError::LegacyParse(format!("invalid value `{raw}` for `{key}`"))
                    })?;
                }
                _ => {}
            }
        }
        Ok(())
    }
}

impl LegacyPlayer {
    pub(in crate::scenario) fn apply_entries(&mut self, entries: &[(String, String)]) -> Result<(), ScenarioError> {
        for (key, value) in entries {
            let key_lower = key.to_ascii_lowercase();
            let raw = value.trim();
            match key_lower.as_str() {
                "standardcrew" => {
                    let id = raw
                        .bytes()
                        .take_while(|byte| is_std_identifier_byte(*byte))
                        .take(4)
                        .map(char::from)
                        .collect::<String>();
                    self.standard_crew =
                        (id.len() == 4 && id != "NONE" && id != "0000").then_some(id);
                }
                "clonks" => {
                    self.clonks = parse_legacy_c4s_value(key, raw, LegacyC4SVal::new(1, 0, 1, 10))?;
                }
                "wealth" => {
                    self.wealth =
                        parse_legacy_c4s_value(key, raw, LegacyC4SVal::new(0, 0, 0, 250))?;
                }
                "position" => {
                    self.position = parse_position(key, raw)?;
                }
                "enforceposition" => {
                    self.enforce_position = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "crew" => {
                    self.crew = parse_legacy_id_list(key, raw)?;
                }
                "buildings" => {
                    self.buildings = parse_legacy_id_list(key, raw)?;
                }
                "vehicles" => {
                    self.vehicles = parse_legacy_id_list(key, raw)?;
                }
                "material" => {
                    self.material = parse_legacy_id_list(key, raw)?;
                }
                "knowledge" => {
                    self.knowledge = parse_legacy_id_list(key, raw)?;
                }
                "homebasematerial" => {
                    self.home_base_material = parse_legacy_id_list(key, raw)?;
                }
                "homebaseproduction" => {
                    self.home_base_production = parse_legacy_id_list(key, raw)?;
                }
                "magic" => {
                    self.magic = parse_legacy_id_list(key, raw)?;
                }
                _ => {}
            }
        }
        Ok(())
    }
}

impl LegacyLandscape {
    fn apply_entries(&mut self, entries: &[(String, String)]) -> Result<(), ScenarioError> {
        for (key, value) in entries {
            let key_lower = key.to_ascii_lowercase();
            let raw = value.trim();
            match key_lower.as_str() {
                "exactlandscape" => {
                    self.exact_landscape = parse_bool_field(key, raw)?;
                }
                "vegetation" => {
                    self.vegetation = parse_legacy_id_list(key, raw)?;
                }
                "vegetationlevel" => {
                    self.vegetation_level =
                        parse_legacy_c4s_value(key, raw, LegacyC4SVal::new(50, 30, 0, 100))?;
                }
                "inearth" => {
                    self.in_earth = parse_legacy_id_list(key, raw)?;
                }
                "inearthlevel" => {
                    self.in_earth_level =
                        parse_legacy_c4s_value(key, raw, LegacyC4SVal::new(50, 0, 0, 100))?;
                }
                "sky" => {
                    if raw.is_empty() {
                        self.sky = None;
                    } else {
                        self.sky = Some(raw.to_string());
                    }
                }
                "skyfade" => {
                    self.sky_fade = parse_i32_array::<6>(key, raw)?;
                }
                "nosky" => {
                    self.no_sky = parse_bool_field(key, raw)?;
                }
                "bottomopen" => {
                    self.bottom_open = parse_bool_field(key, raw)?;
                }
                "topopen" => {
                    self.top_open = parse_bool_field(key, raw)?;
                }
                "leftopen" => {
                    self.left_open = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "rightopen" => {
                    self.right_open = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "autoscansideopen" => {
                    self.auto_scan_side_open = parse_bool_field(key, raw)?;
                }
                "mapwidth" => {
                    self.map_width =
                        parse_legacy_c4s_value(key, raw, LegacyC4SVal::new(100, 0, 64, 250))?;
                }
                "mapheight" => {
                    self.map_height =
                        parse_legacy_c4s_value(key, raw, LegacyC4SVal::new(50, 0, 40, 250))?;
                }
                "mapzoom" => {
                    self.map_zoom =
                        parse_legacy_c4s_value(key, raw, LegacyC4SVal::new(10, 0, 5, 15))?;
                }
                "amplitude" => {
                    self.amplitude =
                        parse_legacy_c4s_value(key, raw, LegacyC4SVal::new(0, 0, 0, 100))?;
                }
                "phase" => {
                    self.phase =
                        parse_legacy_c4s_value(key, raw, LegacyC4SVal::new(50, 0, 0, 100))?;
                }
                "period" => {
                    self.period =
                        parse_legacy_c4s_value(key, raw, LegacyC4SVal::new(15, 0, 0, 100))?;
                }
                "random" => {
                    self.random =
                        parse_legacy_c4s_value(key, raw, LegacyC4SVal::new(0, 0, 0, 100))?;
                }
                "material" => {
                    if !raw.is_empty() {
                        self.material = raw.to_string();
                    }
                }
                "liquid" => {
                    if !raw.is_empty() {
                        self.liquid = raw.to_string();
                    }
                }
                "liquidlevel" => {
                    self.liquid_level =
                        parse_legacy_c4s_value(key, raw, LegacyC4SVal::new(0, 0, 0, 100))?;
                }
                "mapplayerextend" => {
                    self.map_player_extend = parse_bool_field(key, raw)?;
                }
                "layers" => {
                    self.layers = parse_legacy_name_list(key, value)?;
                }
                "gravity" => {
                    self.gravity =
                        parse_legacy_c4s_value(key, raw, LegacyC4SVal::new(100, 0, 10, 200))?;
                }
                "noscan" => {
                    self.no_scan = parse_bool_field(key, raw)?;
                }
                "keepmapcreator" => {
                    self.keep_map_creator = parse_bool_field(key, raw)?;
                }
                "skyscrollmode" => {
                    self.sky_scroll_mode = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "newstylelandscape" => {
                    self.new_style_landscape = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "fowres" => {
                    self.fow_resolution = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "shadematerials" => {
                    self.shade_materials = parse_bool_field(key, raw)?;
                }
                _ => {}
            }
        }
        Ok(())
    }
}

impl LegacyWeather {
    fn apply_entries(&mut self, entries: &[(String, String)]) -> Result<(), ScenarioError> {
        for (key, value) in entries {
            let key_lower = key.to_ascii_lowercase();
            let raw = value.trim();
            match key_lower.as_str() {
                "climate" => {
                    self.climate =
                        parse_legacy_c4s_value(key, raw, LegacyC4SVal::new(50, 10, 0, 100))?;
                }
                "startseason" => {
                    self.start_season =
                        parse_legacy_c4s_value(key, raw, LegacyC4SVal::new(50, 50, 0, 100))?;
                }
                "yearspeed" => {
                    self.year_speed =
                        parse_legacy_c4s_value(key, raw, LegacyC4SVal::new(50, 0, 0, 100))?;
                }
                "rain" => {
                    self.rain = parse_legacy_c4s_value(key, raw, LegacyC4SVal::new(0, 0, 0, 100))?;
                }
                "wind" => {
                    self.wind =
                        parse_legacy_c4s_value(key, raw, LegacyC4SVal::new(0, 70, -100, 100))?;
                }
                "lightning" => {
                    self.lightning =
                        parse_legacy_c4s_value(key, raw, LegacyC4SVal::new(0, 0, 0, 100))?;
                }
                "precipitation" => {
                    if !raw.is_empty() {
                        self.precipitation = raw.to_string();
                    }
                }
                "nogamma" => {
                    self.no_gamma = parse_bool_field(key, raw)?;
                }
                _ => {}
            }
        }
        Ok(())
    }
}

impl LegacyDisasters {
    fn apply_entries(&mut self, entries: &[(String, String)]) -> Result<(), ScenarioError> {
        for (key, value) in entries {
            let key_lower = key.to_ascii_lowercase();
            let raw = value.trim();
            match key_lower.as_str() {
                "meteorite" => {
                    self.meteorite =
                        parse_legacy_c4s_value(key, raw, LegacyC4SVal::new(0, 0, 0, 100))?;
                }
                "volcano" => {
                    self.volcano =
                        parse_legacy_c4s_value(key, raw, LegacyC4SVal::new(0, 0, 0, 100))?;
                }
                "earthquake" => {
                    self.earthquake =
                        parse_legacy_c4s_value(key, raw, LegacyC4SVal::new(0, 0, 0, 100))?;
                }
                _ => {}
            }
        }
        Ok(())
    }
}

impl LegacyAnimals {
    fn apply_entries(&mut self, entries: &[(String, String)]) -> Result<(), ScenarioError> {
        for (key, value) in entries {
            let key_lower = key.to_ascii_lowercase();
            let raw = value.trim();
            match key_lower.as_str() {
                "animal" => {
                    self.free_life = parse_legacy_id_list(key, raw)?;
                }
                "nest" => {
                    self.earth_nest = parse_legacy_id_list(key, raw)?;
                }
                _ => {}
            }
        }
        Ok(())
    }
}

impl LegacyEnvironment {
    fn apply_entries(&mut self, entries: &[(String, String)]) -> Result<(), ScenarioError> {
        for (key, value) in entries {
            let key_lower = key.to_ascii_lowercase();
            let raw = value.trim();
            if key_lower == "objects" {
                self.objects = parse_legacy_id_list(key, raw)?;
            }
        }
        Ok(())
    }
}

const CURRENT_SCENARIO_VERSION: [i32; 4] = {
    let [major, minor, patch, revision, _build] = clonk_core::version::ENGINE_VERSION;
    [major, minor, patch, revision]
};
pub(crate) const C4_MAX_TITLE: usize = 512;

impl LegacyScenarioCore {
    /// Returns the fully loaded C4Scenario state without mutating the retained
    /// parsed source. C4Scenario::Load performs this conversion immediately
    /// after Compile, before either parameters or SaveCore can observe it
    /// (C4Scenario.cpp:86-97).
    pub(in crate::scenario) fn after_load_conversion(&self) -> Self {
        let mut loaded = self.clone();
        loaded.game.convert_goals_after_load();
        loaded
    }

    fn initial_save_core(
        &self,
        scenario_title: &str,
        definition_modules: &[String],
        definition_executable_path: &str,
        definition_path: &str,
        scenario_origin: &str,
    ) -> Self {
        let mut saved = self.after_load_conversion();

        // C4GameSave::SaveCore updates the first four C4XVer components but
        // deliberately leaves the fifth (historic build component) intact
        // (C4GameSave.cpp:58-64).
        saved.head.version[..CURRENT_SCENARIO_VERSION.len()]
            .copy_from_slice(&CURRENT_SCENARIO_VERSION);
        // SCopy(..., C4MaxTitle) copies the native C string through the first
        // NUL and keeps at most C4MaxTitle bytes (C4GameSave.cpp:84;
        // C4Strings.cpp:67-81).
        saved.head.title = truncate_legacy_c4_string(scenario_title.to_owned(), C4_MAX_TITLE);
        saved.head.mission_access.clear();
        // SaveCore resets NetworkGame before the save specialization applies
        // its own flags. NetworkRuntimeJoin is deliberately retained here.
        saved.head.network_game = false;
        saved.head.forced_gfx_mode = 1;

        // C4SDefinitions::SetModules replaces the list and derives LocalOnly
        // from whether it is empty (C4Scenario.cpp:461-478).
        saved.definitions.definitions = set_legacy_definition_modules(
            definition_modules,
            definition_executable_path,
            definition_path,
        );
        saved.definitions.reflected_definitions = None;
        saved.definitions.local_only = definition_modules.is_empty();

        // GetSaveOrigin retains an existing origin; only an empty origin is
        // populated from the running scenario filename
        // (C4GameSave.cpp:93-101). C4SHead normalizes alternate separators
        // to the current platform while loading (C4Scenario.cpp:200-202).
        let origin = saved
            .head
            .origin
            .as_deref()
            .filter(|origin| !origin.is_empty())
            .unwrap_or(scenario_origin);
        saved.head.origin = (!origin.is_empty()).then(|| normalize_legacy_path(origin));

        // fInitial intentionally leaves NoInitialize and SaveGame unchanged
        // (C4GameSave.cpp:65-75).
        saved
    }

    pub(in crate::scenario) fn initial_network_save(
        &self,
        scenario_title: &str,
        definition_modules: &[String],
        definition_executable_path: &str,
        definition_path: &str,
        scenario_origin: &str,
    ) -> Self {
        let mut saved = self.initial_save_core(
            scenario_title,
            definition_modules,
            definition_executable_path,
            definition_path,
            scenario_origin,
        );
        saved.head.network_game = true;
        saved.head.network_runtime_join = false;
        saved
    }

    pub(in crate::scenario) fn initial_record_save(
        &self,
        record_title: &str,
        definition_modules: &[String],
        definition_executable_path: &str,
        definition_path: &str,
        scenario_origin: &str,
    ) -> Self {
        let mut saved = self.initial_save_core(
            record_title,
            definition_modules,
            definition_executable_path,
            definition_path,
            scenario_origin,
        );
        saved.head.replay = 1;
        saved.head.icon = 29;
        saved
    }

    pub(in crate::scenario) fn runtime_network_save(
        &self,
        scenario_title: &str,
        definition_modules: &[String],
        definition_executable_path: &str,
        definition_path: &str,
        scenario_origin: &str,
    ) -> Self {
        let mut saved = self.runtime_exact_save_core(
            scenario_title,
            definition_modules,
            definition_executable_path,
            definition_path,
            scenario_origin,
        );
        saved.head.network_game = true;
        saved.head.network_runtime_join = true;
        saved
    }

    pub(in crate::scenario) fn runtime_scenario_save(&self) -> Self {
        let mut saved = self.clone();
        saved.head.version[..CURRENT_SCENARIO_VERSION.len()]
            .copy_from_slice(&CURRENT_SCENARIO_VERSION);
        saved.head.no_initialize = 1;
        saved.head.save_game = 0;
        // SaveCore clears NetworkGame for every non-initial save, but does
        // not touch NetworkRuntimeJoin. Preserve that slightly surprising
        // distinction for scenarios that originated from a runtime dynamic.
        saved.head.network_game = false;
        saved.head.mission_access.clear();
        saved.head.forced_gfx_mode = 1;
        saved
    }

    pub(in crate::scenario) fn runtime_savegame(
        &self,
        scenario_title: &str,
        definition_modules: &[String],
        definition_executable_path: &str,
        definition_path: &str,
        scenario_origin: &str,
        icon: i32,
    ) -> Self {
        let mut saved = self.runtime_exact_save_core(
            scenario_title,
            definition_modules,
            definition_executable_path,
            definition_path,
            scenario_origin,
        );
        saved.head.icon = icon;
        saved
    }

    pub(in crate::scenario) fn runtime_record_save(
        &self,
        record_title: &str,
        definition_modules: &[String],
        definition_executable_path: &str,
        definition_path: &str,
        scenario_origin: &str,
    ) -> Self {
        let mut saved = self.runtime_exact_save_core(
            record_title,
            definition_modules,
            definition_executable_path,
            definition_path,
            scenario_origin,
        );
        saved.head.replay = 1;
        saved.head.icon = 29;
        saved
    }

    pub(in crate::scenario) fn runtime_exact_save_core(
        &self,
        scenario_title: &str,
        definition_modules: &[String],
        definition_executable_path: &str,
        definition_path: &str,
        scenario_origin: &str,
    ) -> Self {
        let mut saved = self.runtime_scenario_save();
        saved.head.title = truncate_legacy_c4_string(scenario_title.to_owned(), C4_MAX_TITLE);
        saved.head.save_game = 1;
        saved.definitions.definitions = set_legacy_definition_modules(
            definition_modules,
            definition_executable_path,
            definition_path,
        );
        saved.definitions.reflected_definitions = None;
        saved.definitions.local_only = definition_modules.is_empty();
        if saved.head.origin.as_deref().is_none_or(str::is_empty) {
            saved.head.origin =
                (!scenario_origin.is_empty()).then(|| normalize_legacy_path(scenario_origin));
        }
        saved
    }

    pub(in crate::scenario) fn serialize_section(&self, force_exact: bool, head_defaults: Option<[i32; 2]>) -> Vec<u8> {
        let mut writer = LegacyScenarioIniWriter::default();
        let [context_menu_default, control_style_default] = head_defaults.unwrap_or([
            self.head.forced_auto_context_menu,
            self.head.forced_control_style,
        ]);
        let mut head = Vec::new();
        push_value(&mut head, "NoInitialize", self.head.no_initialize, 0);
        push_value(&mut head, "RandomSeed", self.head.random_seed, 0);
        push_value(
            &mut head,
            "ForcedAutoContextMenu",
            self.head.forced_auto_context_menu,
            context_menu_default,
        );
        push_value(
            &mut head,
            "ForcedAutoStopControl",
            self.head.forced_control_style,
            control_style_default,
        );
        writer.push_section("Head", head);

        let mut game = serialize_legacy_game(&self.game);
        game.retain(|(name, _)| *name != "ValueOverloads");
        writer.push_section("Game", game);
        for index in 0..MAX_PLAYER_STARTS {
            let player = self.players.get(index).cloned().unwrap_or_default();
            writer.push_section(
                &format!("Player{}", index + 1),
                serialize_legacy_player(&player),
            );
        }
        let mut landscape = self.landscape.clone();
        landscape.exact_landscape |= force_exact;
        writer.push_section(
            "Landscape",
            serialize_legacy_landscape(&landscape, self.uses_new_landscape_defaults()),
        );
        writer.push_section("Animals", serialize_legacy_animals(&self.animals));
        writer.push_section("Weather", serialize_legacy_weather(&self.weather));
        writer.push_section("Disasters", serialize_legacy_disasters(&self.disasters));
        writer.push_section(
            "Environment",
            serialize_legacy_environment(&self.environment),
        );
        writer.finish()
    }

    pub(in crate::scenario) fn serialize(&self) -> Vec<u8> {
        let mut writer = LegacyScenarioIniWriter::default();
        writer.push_section("Head", serialize_legacy_head(&self.head));
        writer.push_section(
            "Definitions",
            serialize_legacy_definitions(&self.definitions),
        );
        writer.push_section("Game", serialize_legacy_game(&self.game));
        for index in 0..MAX_PLAYER_STARTS {
            let player = self.players.get(index).cloned().unwrap_or_default();
            writer.push_section(
                &format!("Player{}", index + 1),
                serialize_legacy_player(&player),
            );
        }
        writer.push_section(
            "Landscape",
            serialize_legacy_landscape(&self.landscape, self.uses_new_landscape_defaults()),
        );
        writer.push_section("Animals", serialize_legacy_animals(&self.animals));
        writer.push_section("Weather", serialize_legacy_weather(&self.weather));
        writer.push_section("Disasters", serialize_legacy_disasters(&self.disasters));
        writer.push_section(
            "Environment",
            serialize_legacy_environment(&self.environment),
        );
        writer.finish()
    }

    fn uses_new_landscape_defaults(&self) -> bool {
        self.head.version[0] == 0 || self.head.version >= [4, 6, 5, 0, 0]
    }
}

type LegacyIniFields = Vec<(&'static str, String)>;

#[derive(Default)]
struct LegacyScenarioIniWriter {
    output: Vec<u8>,
}

impl LegacyScenarioIniWriter {
    fn push_section(&mut self, name: &str, fields: LegacyIniFields) {
        if fields.is_empty() {
            return;
        }
        if !self.output.is_empty() {
            self.output.extend_from_slice(b"\r\n");
        }
        self.output.push(b'[');
        self.output.extend_from_slice(name.as_bytes());
        self.output.extend_from_slice(b"]\r\n");
        for (key, value) in fields {
            self.output.extend_from_slice(key.as_bytes());
            self.output.push(b'=');
            self.output
                .extend_from_slice(&clonk_script::c4_string_bytes(&value));
            self.output.extend_from_slice(b"\r\n");
        }
    }

    fn finish(self) -> Vec<u8> {
        self.output
    }
}

fn push_value<T>(fields: &mut LegacyIniFields, key: &'static str, value: T, default: T)
where
    T: fmt::Display + PartialEq,
{
    if value != default {
        fields.push((key, value.to_string()));
    }
}

pub(crate) fn push_raw_string(fields: &mut LegacyIniFields, key: &'static str, value: &str, default: &str) {
    if value != default {
        fields.push((key, value.to_owned()));
    }
}

fn push_i32_bool(fields: &mut LegacyIniFields, key: &'static str, value: bool, default: bool) {
    push_value(fields, key, i32::from(value), i32::from(default));
}

fn push_i32_array(fields: &mut LegacyIniFields, key: &'static str, values: &[i32], default: i32) {
    let count = values
        .iter()
        .rposition(|value| *value != default)
        .map_or(0, |index| index + 1);
    if count > 0 {
        fields.push((
            key,
            values[..count]
                .iter()
                .map(i32::to_string)
                .collect::<Vec<_>>()
                .join(","),
        ));
    }
}

fn push_c4s_value(
    fields: &mut LegacyIniFields,
    key: &'static str,
    value: LegacyC4SVal,
    default: LegacyC4SVal,
) {
    if value != default {
        fields.push((
            key,
            format!("{},{},{},{}", value.std, value.rnd, value.min, value.max),
        ));
    }
}

fn push_id_list(fields: &mut LegacyIniFields, key: &'static str, values: &LegacyIdList) {
    if !values.is_empty() {
        fields.push((key, format_id_list(values)));
    }
}

fn push_name_list(fields: &mut LegacyIniFields, key: &'static str, values: &LegacyNameList) {
    if !values.is_empty() {
        fields.push((key, format_name_list(values)));
    }
}

fn format_id_list(values: &LegacyIdList) -> String {
    values
        .iter()
        .map(|entry| format!("{}={}", entry.id, entry.count.unwrap_or(0)))
        .collect::<Vec<_>>()
        .join(";")
}

fn format_name_list(values: &LegacyNameList) -> String {
    values
        .iter()
        .map(|entry| format!("{}={}", entry.name, entry.count.unwrap_or(0)))
        .collect::<Vec<_>>()
        .join(";")
}

fn serialize_legacy_head(head: &LegacyHead) -> LegacyIniFields {
    let mut fields = Vec::new();
    // C4SHead::CompileFunc field order/defaults (C4Scenario.cpp:164-203).
    push_value(&mut fields, "Icon", head.icon, 18);
    push_raw_string(&mut fields, "Title", &head.title, "Default Title");
    push_raw_string(&mut fields, "Loader", &head.loader, "");
    push_raw_string(&mut fields, "Font", &head.font, "");
    push_i32_array(&mut fields, "Version", &head.version, 0);
    push_value(&mut fields, "Difficulty", head.difficulty, 0);
    push_value(&mut fields, "MaxPlayer", head.max_player, 12);
    push_value(
        &mut fields,
        "MaxPlayerLeague",
        head.max_player_league,
        head.max_player,
    );
    push_value(&mut fields, "MinPlayer", head.min_player, 0);
    push_value(&mut fields, "SaveGame", head.save_game, 0);
    push_value(&mut fields, "Replay", head.replay, 0);
    push_value(&mut fields, "Film", head.film, 0);
    push_value(&mut fields, "DisableMouse", head.disable_mouse, 0);
    push_value(&mut fields, "NoInitialize", head.no_initialize, 0);
    push_value(&mut fields, "RandomSeed", head.random_seed, 0);
    push_value(
        &mut fields,
        "ForcedAutoContextMenu",
        head.forced_auto_context_menu,
        -1,
    );
    push_value(
        &mut fields,
        "ForcedAutoStopControl",
        head.forced_control_style,
        -1,
    );
    push_raw_string(&mut fields, "Engine", &head.engine, "");
    push_raw_string(&mut fields, "MissionAccess", &head.mission_access, "");
    push_value(&mut fields, "NetworkGame", head.network_game, false);
    push_value(
        &mut fields,
        "NetworkRuntimeJoin",
        head.network_runtime_join,
        false,
    );
    push_value(&mut fields, "ForcedGfxMode", head.forced_gfx_mode, 0);
    push_value(&mut fields, "ForcedNoCrew", head.forced_fair_crew, 0);
    push_value(&mut fields, "DefCrewStrength", head.fair_crew_strength, 0);
    push_raw_string(
        &mut fields,
        "Origin",
        head.origin.as_deref().unwrap_or_default(),
        "",
    );
    fields
}

fn serialize_legacy_definitions(definitions: &LegacyDefinitions) -> LegacyIniFields {
    let mut fields = Vec::new();
    // C4SDefinitions::CompileFunc (C4Scenario.cpp:480-500).
    push_value(&mut fields, "LocalOnly", definitions.local_only, false);
    push_value(
        &mut fields,
        "AllowUserChange",
        definitions.allow_user_change,
        false,
    );
    if !definitions.definitions.is_empty() {
        fields.push((
            "Definitions",
            definitions
                .definitions
                .iter()
                .map(|module| escape_cpp_ini_string(module))
                .collect::<Vec<_>>()
                .join(","),
        ));
    }
    push_id_list(&mut fields, "SkipDefs", &definitions.skip_defs);
    fields
}

fn serialize_legacy_game(game: &LegacyGame) -> LegacyIniFields {
    let mut fields = Vec::new();
    // C4SGame::CompileFunc (C4Scenario.cpp:221-257).
    push_value(&mut fields, "Mode", game.mode, 0);
    push_value(&mut fields, "Elimination", game.elimination, 1);
    push_value(&mut fields, "CooperativeGoal", game.cooperative_goal, 0);
    push_id_list(&mut fields, "CreateObjects", &game.create_objects);
    push_id_list(&mut fields, "ClearObjects", &game.clear_objects);
    push_name_list(&mut fields, "ClearMaterials", &game.clear_materials);
    push_value(&mut fields, "ValueGain", game.value_gain, 0);
    push_value(
        &mut fields,
        "EnableRemoveFlag",
        game.enable_remove_flag,
        false,
    );
    push_value(
        &mut fields,
        "StructNeedMaterial",
        game.realism.construction_needs_material,
        false,
    );
    push_value(
        &mut fields,
        "StructNeedEnergy",
        game.realism.structures_need_energy,
        true,
    );
    push_id_list(&mut fields, "ValueOverloads", &game.realism.value_overloads);
    push_value(
        &mut fields,
        "LandscapePushPull",
        game.realism.landscape_push_pull,
        0,
    );
    push_value(
        &mut fields,
        "LandscapeInsertThrust",
        game.realism.landscape_insert_thrust,
        1,
    );
    if game.realism.base_functionality != BASEFUNC_DEFAULT {
        if let Some(value) = format_base_functionality(game.realism.base_functionality) {
            fields.push(("BaseFunctionality", value));
        }
    }
    push_value(
        &mut fields,
        "BaseRegenerateEnergyPrice",
        game.realism.base_regenerate_energy_price,
        BASE_REGENERATE_ENERGY_PRICE,
    );
    push_id_list(&mut fields, "Goals", &game.goals);
    push_id_list(&mut fields, "Rules", &game.rules);
    push_value(&mut fields, "FoWColor", game.fow_color, 0);
    fields
}

fn serialize_legacy_player(player: &LegacyPlayer) -> LegacyIniFields {
    let mut fields = Vec::new();
    // C4SPlrStart::CompileFunc (C4Scenario.cpp:276-291).
    if let Some(crew) = player.standard_crew.as_deref() {
        push_raw_string(&mut fields, "StandardCrew", crew, "");
    }
    push_c4s_value(
        &mut fields,
        "Clonks",
        player.clonks,
        LegacyC4SVal::new(1, 0, 1, 10),
    );
    push_c4s_value(
        &mut fields,
        "Wealth",
        player.wealth,
        LegacyC4SVal::new(0, 0, 0, 250),
    );
    push_i32_array(&mut fields, "Position", &player.position, -1);
    push_value(&mut fields, "EnforcePosition", player.enforce_position, 0);
    push_id_list(&mut fields, "Crew", &player.crew);
    push_id_list(&mut fields, "Buildings", &player.buildings);
    push_id_list(&mut fields, "Vehicles", &player.vehicles);
    push_id_list(&mut fields, "Material", &player.material);
    push_id_list(&mut fields, "Knowledge", &player.knowledge);
    push_id_list(&mut fields, "HomeBaseMaterial", &player.home_base_material);
    push_id_list(
        &mut fields,
        "HomeBaseProduction",
        &player.home_base_production,
    );
    push_id_list(&mut fields, "Magic", &player.magic);
    fields
}

fn serialize_legacy_landscape(
    landscape: &LegacyLandscape,
    shade_materials_default: bool,
) -> LegacyIniFields {
    let mut fields = Vec::new();
    // C4SLandscape::CompileFunc (C4Scenario.cpp:336-370). SaveCore has
    // already set a current engine version, so ShadeMaterials defaults true.
    push_value(
        &mut fields,
        "ExactLandscape",
        landscape.exact_landscape,
        false,
    );
    push_id_list(&mut fields, "Vegetation", &landscape.vegetation);
    push_c4s_value(
        &mut fields,
        "VegetationLevel",
        landscape.vegetation_level,
        LegacyC4SVal::new(50, 30, 0, 100),
    );
    push_id_list(&mut fields, "InEarth", &landscape.in_earth);
    push_c4s_value(
        &mut fields,
        "InEarthLevel",
        landscape.in_earth_level,
        LegacyC4SVal::new(50, 0, 0, 100),
    );
    push_raw_string(
        &mut fields,
        "Sky",
        landscape.sky.as_deref().unwrap_or_default(),
        "",
    );
    push_i32_array(&mut fields, "SkyFade", &landscape.sky_fade, 0);
    push_value(&mut fields, "NoSky", landscape.no_sky, false);
    push_value(&mut fields, "BottomOpen", landscape.bottom_open, false);
    push_value(&mut fields, "TopOpen", landscape.top_open, true);
    push_value(&mut fields, "LeftOpen", landscape.left_open, 0);
    push_value(&mut fields, "RightOpen", landscape.right_open, 0);
    push_value(
        &mut fields,
        "AutoScanSideOpen",
        landscape.auto_scan_side_open,
        true,
    );
    push_c4s_value(
        &mut fields,
        "MapWidth",
        landscape.map_width,
        LegacyC4SVal::new(100, 0, 64, 250),
    );
    push_c4s_value(
        &mut fields,
        "MapHeight",
        landscape.map_height,
        LegacyC4SVal::new(50, 0, 40, 250),
    );
    push_c4s_value(
        &mut fields,
        "MapZoom",
        landscape.map_zoom,
        LegacyC4SVal::new(10, 0, 5, 15),
    );
    push_c4s_value(
        &mut fields,
        "Amplitude",
        landscape.amplitude,
        LegacyC4SVal::new(0, 0, 0, 100),
    );
    push_c4s_value(
        &mut fields,
        "Phase",
        landscape.phase,
        LegacyC4SVal::new(50, 0, 0, 100),
    );
    push_c4s_value(
        &mut fields,
        "Period",
        landscape.period,
        LegacyC4SVal::new(15, 0, 0, 100),
    );
    push_c4s_value(
        &mut fields,
        "Random",
        landscape.random,
        LegacyC4SVal::new(0, 0, 0, 100),
    );
    push_raw_string(&mut fields, "Material", &landscape.material, "Earth");
    push_raw_string(&mut fields, "Liquid", &landscape.liquid, "Water");
    push_c4s_value(
        &mut fields,
        "LiquidLevel",
        landscape.liquid_level,
        LegacyC4SVal::new(0, 0, 0, 100),
    );
    push_value(
        &mut fields,
        "MapPlayerExtend",
        landscape.map_player_extend,
        false,
    );
    push_name_list(&mut fields, "Layers", &landscape.layers);
    push_c4s_value(
        &mut fields,
        "Gravity",
        landscape.gravity,
        LegacyC4SVal::new(100, 0, 10, 200),
    );
    push_value(&mut fields, "NoScan", landscape.no_scan, false);
    push_value(
        &mut fields,
        "KeepMapCreator",
        landscape.keep_map_creator,
        false,
    );
    push_value(&mut fields, "SkyScrollMode", landscape.sky_scroll_mode, 0);
    push_value(
        &mut fields,
        "NewStyleLandscape",
        landscape.new_style_landscape,
        0,
    );
    push_value(
        &mut fields,
        "FoWRes",
        landscape.fow_resolution,
        DEFAULT_FOW_RESOLUTION,
    );
    push_value(
        &mut fields,
        "ShadeMaterials",
        landscape.shade_materials,
        shade_materials_default,
    );
    fields
}

fn serialize_legacy_animals(animals: &LegacyAnimals) -> LegacyIniFields {
    let mut fields = Vec::new();
    // C4SAnimals::CompileFunc (C4Scenario.cpp:394-404).
    push_id_list(&mut fields, "Animal", &animals.free_life);
    push_id_list(&mut fields, "Nest", &animals.earth_nest);
    fields
}

fn serialize_legacy_weather(weather: &LegacyWeather) -> LegacyIniFields {
    let mut fields = Vec::new();
    // C4SWeather::CompileFunc (C4Scenario.cpp:372-392).
    push_c4s_value(
        &mut fields,
        "Climate",
        weather.climate,
        LegacyC4SVal::new(50, 10, 0, 100),
    );
    push_c4s_value(
        &mut fields,
        "StartSeason",
        weather.start_season,
        LegacyC4SVal::new(50, 50, 0, 100),
    );
    push_c4s_value(
        &mut fields,
        "YearSpeed",
        weather.year_speed,
        LegacyC4SVal::new(50, 0, 0, 100),
    );
    push_c4s_value(
        &mut fields,
        "Rain",
        weather.rain,
        LegacyC4SVal::new(0, 0, 0, 100),
    );
    push_c4s_value(
        &mut fields,
        "Wind",
        weather.wind,
        LegacyC4SVal::new(0, 70, -100, 100),
    );
    push_c4s_value(
        &mut fields,
        "Lightning",
        weather.lightning,
        LegacyC4SVal::new(0, 0, 0, 100),
    );
    push_raw_string(
        &mut fields,
        "Precipitation",
        &weather.precipitation,
        "Water",
    );
    push_value(&mut fields, "NoGamma", weather.no_gamma, true);
    fields
}

fn serialize_legacy_disasters(disasters: &LegacyDisasters) -> LegacyIniFields {
    let mut fields = Vec::new();
    // C4SDisasters::CompileFunc (C4Scenario.cpp:427-439).
    push_c4s_value(
        &mut fields,
        "Meteorite",
        disasters.meteorite,
        LegacyC4SVal::new(0, 0, 0, 100),
    );
    push_c4s_value(
        &mut fields,
        "Volcano",
        disasters.volcano,
        LegacyC4SVal::new(0, 0, 0, 100),
    );
    push_c4s_value(
        &mut fields,
        "Earthquake",
        disasters.earthquake,
        LegacyC4SVal::new(0, 0, 0, 100),
    );
    fields
}

fn serialize_legacy_environment(environment: &LegacyEnvironment) -> LegacyIniFields {
    let mut fields = Vec::new();
    // C4SEnvironment::CompileFunc (C4Scenario.cpp:406-414).
    push_id_list(&mut fields, "Objects", &environment.objects);
    fields
}

pub(in crate::scenario) fn format_base_functionality(value: i32) -> Option<String> {
    let entries = [
        ("BASEFUNC_AutoSellContents", BASEFUNC_AUTO_SELL_CONTENTS),
        ("BASEFUNC_RegenerateEnergy", BASEFUNC_REGENERATE_ENERGY),
        ("BASEFUNC_Buy", BASEFUNC_BUY),
        ("BASEFUNC_Sell", BASEFUNC_SELL),
        ("BASEFUNC_RejectEntrance", BASEFUNC_REJECT_ENTRANCE),
        ("BASEFUNC_Extinguish", BASEFUNC_EXTINGUISH),
        ("BASEFUNC_Default", BASEFUNC_DEFAULT),
    ];
    let mut remaining = value;
    let mut parts = Vec::new();
    for (name, mask) in entries {
        if remaining != 0 && (mask & remaining) == mask {
            parts.push(name.to_owned());
            remaining &= !mask;
        }
    }
    if remaining != 0 {
        parts.push(remaining.to_string());
    }
    (!parts.is_empty()).then(|| parts.join("|"))
}

fn escape_cpp_ini_string(value: &str) -> String {
    let value = clonk_script::c4_string_bytes(value);
    let mut escaped = Vec::with_capacity(value.len() + 2);
    escaped.push(b'"');
    let mut previous_was_numeric_escape = false;
    for byte in value {
        // StdCompilerINIWrite applies `isprint` to unsigned native bytes.
        // The legacy single-byte locale treats the upper printable block as
        // text too; preserving it here is what keeps C4 filenames byte-exact.
        let printable = byte.is_ascii_graphic() || byte == b' ' || byte >= 0xa0;
        if printable
            && byte != b'\\'
            && byte != b'"'
            && !(previous_was_numeric_escape && byte.is_ascii_digit())
        {
            escaped.push(byte);
            previous_was_numeric_escape = false;
            continue;
        }
        previous_was_numeric_escape = false;
        match byte {
            b'\x07' => escaped.extend_from_slice(b"\\a"),
            b'\x08' => escaped.extend_from_slice(b"\\b"),
            b'\x0c' => escaped.extend_from_slice(b"\\f"),
            b'\n' => escaped.extend_from_slice(b"\\n"),
            b'\r' => escaped.extend_from_slice(b"\\r"),
            b'\t' => escaped.extend_from_slice(b"\\t"),
            b'\x0b' => escaped.extend_from_slice(b"\\v"),
            b'"' => escaped.extend_from_slice(b"\\\""),
            b'\\' => escaped.extend_from_slice(b"\\\\"),
            byte => {
                escaped.push(b'\\');
                escaped.extend_from_slice(format!("{byte:o}").as_bytes());
                previous_was_numeric_escape = true;
            }
        }
    }
    escaped.push(b'"');
    clonk_script::c4_string_from_bytes(&escaped)
}

fn normalize_legacy_path(path: &str) -> String {
    if std::path::MAIN_SEPARATOR == '/' {
        path.replace('\\', "/")
    } else {
        path.replace('/', "\\")
    }
}

/// `C4SDefinitions::SetModules`: preserve every separator and redundant path
/// component, then strip ExePath and DefinitionPath in that exact order.
/// Native compares the requested prefix length case-insensitively and does
/// not require a component boundary (C4Scenario.cpp:461-478).
pub(in crate::scenario) fn set_legacy_definition_modules(
    modules: &[String],
    executable_path: &str,
    definition_path: &str,
) -> Vec<String> {
    let executable_path = clonk_script::c4_string_bytes(executable_path);
    let definition_path = clonk_script::c4_string_bytes(definition_path);

    modules
        .iter()
        .map(|module| {
            let mut bytes = clonk_script::c4_string_bytes(module);
            for prefix in [&executable_path, &definition_path] {
                if !prefix.is_empty()
                    && bytes.len() >= prefix.len()
                    && bytes[..prefix.len()]
                        .iter()
                        .zip(prefix.iter())
                        .all(|(left, right)| {
                            legacy_byte_capital(*left) == legacy_byte_capital(*right)
                        })
                {
                    bytes.drain(..prefix.len());
                }
            }
            clonk_script::c4_string_from_bytes(&bytes)
        })
        .collect()
}

fn legacy_byte_capital(byte: u8) -> u8 {
    match byte {
        b'a'..=b'z' => byte - 32,
        0xe4 => 0xc4,
        0xf6 => 0xd6,
        0xfc => 0xdc,
        _ => byte,
    }
}

impl LegacyScenarioCore {
    fn from_sections(
        sections: &HashMap<String, Vec<(String, String)>>,
    ) -> Result<Self, ScenarioError> {
        let mut core = LegacyScenarioCore::default();
        if let Some(entries) = sections.get("head") {
            core.head.apply_entries(entries)?;
            // MaxPlayerLeague's compile default is the already-read
            // MaxPlayer, not C4S_MaxPlayerDefault
            // (C4Scenario.cpp:177-179).
            if !entries
                .iter()
                .any(|(key, _)| key.eq_ignore_ascii_case("MaxPlayerLeague"))
            {
                core.head.max_player_league = core.head.max_player;
            }
        }
        if let Some(entries) = sections.get("definitions") {
            core.definitions.apply_entries(entries)?;
        }
        // C4SRealism::Default starts at zero, but the main-scenario compiler
        // defaults this field to one before applying any explicit value
        // (C4Scenario.cpp:416-425,237-238).
        core.game.realism.landscape_insert_thrust = 1;
        if let Some(entries) = sections.get("game") {
            core.game.apply_entries(entries)?;
        }
        // ShadeMaterials' absent-value default depends on the version that
        // Head compiled first (C4Scenario.cpp:120-133,336-370).
        core.landscape.shade_materials =
            core.head.version[0] == 0 || core.head.version >= [4, 6, 5, 0, 0];
        if let Some(entries) = sections.get("landscape") {
            core.landscape.apply_entries(entries)?;
        }
        if let Some(entries) = sections.get("weather") {
            core.weather.apply_entries(entries)?;
        }
        if let Some(entries) = sections.get("disasters") {
            core.disasters.apply_entries(entries)?;
        }
        if let Some(entries) = sections.get("animals") {
            core.animals.apply_entries(entries)?;
        }
        if let Some(entries) = sections.get("environment") {
            core.environment.apply_entries(entries)?;
        }

        for (section, entries) in sections {
            if !section.starts_with("player") {
                continue;
            }
            let Some(owner) = owner_index_from_section(section) else {
                continue;
            };
            if owner < 0 {
                continue;
            }
            let index = owner as usize;
            if core.players.len() <= index {
                core.players.resize(index + 1, LegacyPlayer::default());
            }
            core.players[index].apply_entries(entries)?;
        }

        Ok(core)
    }

    /// Compile a present section Scenario.txt with C4Scenario's `fSection`
    /// field set. Named fields in the compiled subset receive their naming
    /// defaults even when their whole INI section is absent; only the fields
    /// omitted by the C++ section compiler retain the main core's values
    /// (C4Scenario.cpp:120-134,164-204,221-257,441-445).
    fn compile_section(
        &self,
        sections: &HashMap<String, Vec<(String, String)>>,
    ) -> Result<Self, ScenarioError> {
        let mut core = LegacyScenarioCore::default();

        // Head compiles only these four fields in section mode. The two
        // forced-control defaults are statics captured from the main Head;
        // every other Head value survives unchanged.
        core.head = self.head.clone();
        core.head.no_initialize = 0;
        core.head.random_seed = 0;
        if let Some(entries) = sections.get("head") {
            let retained_max_player_league = core.head.max_player_league;
            let entries = entries
                .iter()
                .filter(|(key, _)| {
                    [
                        "NoInitialize",
                        "RandomSeed",
                        "ForcedAutoContextMenu",
                        "ForcedAutoStopControl",
                    ]
                    .iter()
                    .any(|allowed| key.eq_ignore_ascii_case(allowed))
                })
                .cloned()
                .collect::<Vec<_>>();
            core.head.apply_entries(&entries)?;
            // `LegacyHead::apply_entries` normally implements the main-file
            // MaxPlayerLeague fallback. That field is not compiled at all in
            // section mode, so preserve the already-loaded main value.
            core.head.max_player_league = retained_max_player_league;
        }

        // Definitions and ValueOverloads are not visited by the section
        // compiler at all.
        core.definitions = self.definitions.clone();
        core.game.realism.value_overloads = self.game.realism.value_overloads.clone();
        // This compiler default differs from C4SRealism::Default().
        core.game.realism.landscape_insert_thrust = 1;
        if let Some(entries) = sections.get("game") {
            let entries = entries
                .iter()
                .filter(|(key, _)| !key.eq_ignore_ascii_case("ValueOverloads"))
                .cloned()
                .collect::<Vec<_>>();
            core.game.apply_entries(&entries)?;
        }

        core.players = vec![LegacyPlayer::default(); MAX_PLAYER_STARTS];
        for index in 0..MAX_PLAYER_STARTS {
            if let Some(entries) = sections.get(&format!("player{}", index + 1)) {
                core.players[index].apply_entries(entries)?;
            }
        }

        // Version is a retained Head field, so ShadeMaterials' naming default
        // is derived from the main scenario version even for a section.
        core.landscape.shade_materials =
            core.head.version[0] == 0 || core.head.version >= [4, 6, 5, 0, 0];
        if let Some(entries) = sections.get("landscape") {
            core.landscape.apply_entries(entries)?;
        }
        if let Some(entries) = sections.get("animals") {
            core.animals.apply_entries(entries)?;
        }
        if let Some(entries) = sections.get("weather") {
            core.weather.apply_entries(entries)?;
        }
        if let Some(entries) = sections.get("disasters") {
            core.disasters.apply_entries(entries)?;
        }
        if let Some(entries) = sections.get("environment") {
            core.environment.apply_entries(entries)?;
        }

        apply_scenario_rct_all_strings(&mut core, sections, false);
        Ok(core)
    }
}

pub(in crate::scenario) fn parse_legacy_scenario_manifest(group: &Group) -> Result<LegacyScenarioManifest, ScenarioError> {
    let bytes = match read_group_file_case_insensitive(group, "Scenario.txt") {
        Ok(bytes) => bytes,
        Err(GroupError::EntryNotFound(_)) => return Err(ScenarioError::LegacyCoreMissing),
        Err(GroupError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
            return Err(ScenarioError::LegacyCoreMissing);
        }
        Err(error) => return Err(ScenarioError::Resources(error)),
    };

    // StdCompiler receives Scenario.txt as a C string. A packed component may
    // carry its terminating NUL in the stored size; anything after the first
    // NUL is invisible to C++ and must not influence loader metadata.
    let visible_len = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    let visible = &bytes[..visible_len];
    let text = clonk_script::c4_string_from_bytes(visible);
    let mut manifest = parse_legacy_scenario_text(&text)?;

    // Parse a byte-for-byte Latin-1 projection of just the title as well. The
    // INI grammar is ASCII, so this retains the exact value bytes while finding
    // the same Head field independently of the script string representation.
    // Avoid semantically compiling the duplicate projection: an unrelated
    // non-ASCII field must not create a second failure surface.
    let native_text = bytes_as_latin1_string(visible);
    let native_tree = LegacyIniTree::parse(&native_text);
    let native_title = native_tree
        .first_section(0, "Head")
        .and_then(|head| native_tree.value(head, "Title"))
        .map(parse_rct_all)
        .unwrap_or_else(|| "Default Title".to_string())
        .chars()
        .map(|character| character as u8)
        .collect::<Vec<_>>();
    manifest.head_title_native = LegacyCString::from_bytes(native_title);
    Ok(manifest)
}

pub(in crate::scenario) fn overlay_legacy_scenario_manifest(
    base: &LegacyScenarioManifest,
    overlay: LegacyScenarioManifest,
) -> Result<LegacyScenarioManifest, ScenarioError> {
    // Raw section entries must remain separate from the main file: landscape
    // and weather loaders also use their absence to select C++ defaults.
    let sections = overlay.sections;
    let core = base.core.compile_section(&sections)?;
    let ground_height_hint = derive_ground_height_hint(&sections);
    let definition_specs = core.definitions.definitions.clone();

    Ok(LegacyScenarioManifest {
        title: base.title.clone(),
        description: base.description.clone(),
        head_title_native: base.head_title_native.clone(),
        definition_specs,
        ground_height_hint,
        core,
        sections,
    })
}

pub(in crate::scenario) fn read_group_file_case_insensitive(group: &Group, name: &str) -> Result<Vec<u8>, GroupError> {
    try_read_group_file_case_insensitive(group, name)?
        .ok_or_else(|| GroupError::EntryNotFound(PathBuf::from(name)))
}

pub(in crate::scenario) fn read_optional_legacy_entry(group: &Group, name: &str) -> Result<Option<Vec<u8>>, ScenarioError> {
    match group.read_file(name) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(GroupError::EntryNotFound(_)) => Ok(None),
        Err(GroupError::Io(error)) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(ScenarioError::Resources(error)),
    }
}

/// Extracts an INI name with the same whitespace rules as
/// `StdCompilerINIRead::CreateNameTree`: spaces are name characters, while a
/// tab terminates the name and may be followed only by spaces or more tabs.
fn stdcompiler_ini_name(raw: &str) -> Option<&str> {
    let bytes = raw.as_bytes();
    if !bytes.first().is_some_and(u8::is_ascii_alphabetic) {
        return None;
    }

    let mut end = 0;
    while bytes
        .get(end)
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b' ' || *byte == b'_')
    {
        end += 1;
    }

    bytes[end..]
        .iter()
        .all(|byte| matches!(byte, b' ' | b'\t'))
        .then(|| &raw[..end])
}

pub(in crate::scenario) fn try_read_group_file_case_insensitive(
    group: &Group,
    name: &str,
) -> Result<Option<Vec<u8>>, GroupError> {
    let entry = group.entries()?.into_iter().find(|entry| {
        entry.relative_path.components().count() == 1
            && entry
                .relative_path
                .to_str()
                .is_some_and(|candidate| candidate.eq_ignore_ascii_case(name))
    });
    entry
        .map(|entry| group.read_file(entry.relative_path))
        .transpose()
}

pub(in crate::scenario) fn load_loader_scenario_title<S: AsRef<str>>(
    components: &ComponentGroups,
    languages: &[S],
) -> Result<Option<(String, LegacyCString)>, ScenarioError> {
    let candidates = languages
        .iter()
        .map(|language| format!("Title{}.txt", language.as_ref()))
        .chain(std::iter::once("Title.txt".to_string()));
    for candidate in candidates {
        let Some(component) = components.read(&candidate)? else {
            continue;
        };
        let source = component
            .bytes
            .split(|byte| *byte == 0)
            .next()
            .unwrap_or_default();
        for language in languages {
            let needle = format!("{}:", language.as_ref());
            if let Some(position) = cpp_ssearch_end(source, needle.as_bytes()) {
                let value = &source[position..];
                // C4ComponentHost first searches the complete remainder for
                // CR and only falls back to LF when no CR exists anywhere.
                let end = value
                    .iter()
                    .position(|byte| *byte == b'\r')
                    .or_else(|| value.iter().position(|byte| *byte == b'\n'))
                    .unwrap_or(value.len());
                let native = value[..end].to_vec();
                let presentation = decode_legacy_script_text(&native);
                let native = LegacyCString::from_bytes(native)
                    .expect("the title component was truncated before its first NUL");
                return Ok(Some((presentation, native)));
            }
        }
        // C4ComponentHost keeps the first existing component even when it
        // contains no requested language; Head.Title is then the fallback.
        return Ok(None);
    }
    Ok(None)
}

fn cpp_ssearch_end(source: &[u8], needle: &[u8]) -> Option<usize> {
    let mut matched = 0usize;
    for (index, byte) in source.iter().enumerate() {
        if *byte == needle[matched] {
            matched += 1;
        } else {
            // C++ SSearch does not reconsider the mismatching byte as the
            // beginning of a new partial match.
            matched = 0;
        }
        if matched >= needle.len() {
            return Some(index + 1);
        }
    }
    None
}

pub(in crate::scenario) fn validate_name_ex_no_empty(mut value: String) -> Result<String, ScenarioError> {
    value = value
        .trim_matches(|character: char| character.is_ascii_whitespace())
        .to_string();
    if value.is_empty() {
        return Ok("Unknown".to_string());
    }
    if value.len() > 120 {
        if !value.is_char_boundary(120) {
            return Err(ScenarioError::LoaderTitleTruncationBoundary { limit: 120 });
        }
        value.truncate(120);
    }
    Ok(value)
}

pub(in crate::scenario) fn validate_name_ex_no_empty_bytes(value: &[u8]) -> LegacyCString {
    let start = value
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(value.len());
    let end = value
        .iter()
        .rposition(|byte| !byte.is_ascii_whitespace())
        .map_or(start, |index| index + 1);
    let mut value = value[start..end].to_vec();
    if value.is_empty() {
        value.extend_from_slice(b"Unknown");
    }
    value.truncate(120);
    LegacyCString::from_bytes(value).expect("a Scenario.txt title contains no interior NUL")
}

/// Splits legacy compiler input on LF, CRLF, or lone CR without changing the
/// physical-line count of ordinary LF/CRLF files.
pub(in crate::scenario) fn legacy_ini_lines(source: &str) -> impl Iterator<Item = &str> {
    // `str::lines` recognizes LF and CRLF, but not a lone CR. Split LF first
    // to keep CRLF as one physical line, then split any remaining bare CRs.
    source.split_inclusive('\n').flat_map(|line| {
        let line = line.strip_suffix('\n').unwrap_or(line);
        let line = line.strip_suffix('\r').unwrap_or(line);
        line.split('\r')
    })
}

/// Reads the first exact `[Parameters] MaxPlayers` value. The compiler's
/// scenario-derived default is `C4S.Head.MaxPlayer`; Parameters.txt may
/// replace it before offline players are admitted (pristine 9ffa0a5d
/// src/C4GameParameters.cpp:408-422,553-558).
pub(in crate::scenario) fn parse_legacy_parameters_max_players(
    bytes: &[u8],
    scenario_default: i32,
) -> Result<i32, ScenarioError> {
    parse_legacy_parameters_i32(bytes, "MaxPlayers", scenario_default)
}

pub(in crate::scenario) fn parse_legacy_parameters_random_seed(
    bytes: &[u8],
    scenario_default: i32,
) -> Result<i32, ScenarioError> {
    parse_legacy_parameters_i32(bytes, "RandomSeed", scenario_default)
}

fn parse_legacy_parameters_i32(
    bytes: &[u8],
    field: &str,
    scenario_default: i32,
) -> Result<i32, ScenarioError> {
    let text = String::from_utf8_lossy(bytes);
    let mut in_parameters = false;
    let mut saw_parameters = false;

    for raw_line in legacy_ini_lines(&text) {
        let mut line = raw_line.trim();
        if line.is_empty()
            || line.starts_with(';')
            || line.starts_with('#')
            || line.starts_with("//")
        {
            continue;
        }
        if let Some(index) = line.find("//") {
            line = line[..index].trim_end();
        }
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let Some(section) = stdcompiler_ini_name(&line[1..line.len() - 1]) else {
                // Like CreateNameTree, an invalid header does not leave the
                // current section.
                continue;
            };
            if in_parameters {
                break;
            }
            in_parameters = section == "Parameters" && !saw_parameters;
            saw_parameters |= section == "Parameters";
            continue;
        }
        if !in_parameters {
            continue;
        }
        let Some((raw_key, raw_value)) = line.split_once('=') else {
            continue;
        };
        let Some(key) = stdcompiler_ini_name(raw_key.trim()) else {
            continue;
        };
        if key != field {
            continue;
        }
        return parse_i32(raw_value.trim()).map_err(|error| {
            ScenarioError::LegacyParse(format!(
                "invalid Parameters.txt {field} value `{}`: {error}",
                raw_value.trim()
            ))
        });
    }

    Ok(scenario_default)
}

pub(in crate::scenario) fn parse_legacy_scenario_text(text: &str) -> Result<LegacyScenarioManifest, ScenarioError> {
    const HEAD_KEYS: &[&str] = &[
        "Icon",
        "Title",
        "Description",
        "Loader",
        "Font",
        "Version",
        "Difficulty",
        "MaxPlayer",
        "MaxPlayerLeague",
        "MinPlayer",
        "SaveGame",
        "Replay",
        "Film",
        "DisableMouse",
        "NoInitialize",
        "RandomSeed",
        "ForcedAutoContextMenu",
        "ForcedAutoStopControl",
        "Engine",
        "MissionAccess",
        "NetworkGame",
        "NetworkRuntimeJoin",
        "ForcedGfxMode",
        "ForcedNoCrew",
        "DefCrewStrength",
        "Origin",
    ];
    const DEFINITION_KEYS: &[&str] = &[
        "LocalOnly",
        "AllowUserChange",
        "Definitions",
        "Definition1",
        "Definition2",
        "Definition3",
        "Definition4",
        "Definition5",
        "Definition6",
        "Definition7",
        "Definition8",
        "Definition9",
        "Definition10",
        "SkipDefs",
    ];
    const GAME_KEYS: &[&str] = &[
        "Mode",
        "Elimination",
        "CooperativeGoal",
        "CreateObjects",
        "ClearObjects",
        "ClearMaterials",
        "ValueGain",
        "EnableRemoveFlag",
        "StructNeedMaterial",
        "StructNeedEnergy",
        "ValueOverloads",
        "LandscapePushPull",
        "LandscapeInsertThrust",
        "BaseFunctionality",
        "BaseRegenerateEnergyPrice",
        "Goals",
        "Rules",
        "FoWColor",
    ];
    const PLAYER_KEYS: &[&str] = &[
        "StandardCrew",
        "Clonks",
        "Wealth",
        "Position",
        "EnforcePosition",
        "Crew",
        "Buildings",
        "Vehicles",
        "Material",
        "Knowledge",
        "HomeBaseMaterial",
        "HomeBaseProduction",
        "Magic",
    ];
    const LANDSCAPE_KEYS: &[&str] = &[
        "ExactLandscape",
        "Vegetation",
        "VegetationLevel",
        "InEarth",
        "InEarthLevel",
        "Sky",
        "SkyFade",
        "NoSky",
        "BottomOpen",
        "TopOpen",
        "LeftOpen",
        "RightOpen",
        "AutoScanSideOpen",
        "MapWidth",
        "MapHeight",
        "MapZoom",
        "Amplitude",
        "Phase",
        "Period",
        "Random",
        "Material",
        "Liquid",
        "LiquidLevel",
        "MapPlayerExtend",
        "Layers",
        "Gravity",
        "NoScan",
        "KeepMapCreator",
        "SkyScrollMode",
        "NewStyleLandscape",
        "FoWRes",
        "ShadeMaterials",
    ];
    const WEATHER_KEYS: &[&str] = &[
        "Climate",
        "StartSeason",
        "YearSpeed",
        "Rain",
        "Wind",
        "Lightning",
        "Precipitation",
        "NoGamma",
    ];

    let tree = LegacyIniTree::parse(text);
    let mut sections = HashMap::new();

    insert_validated_scenario_section::<LegacyHead>(
        &tree,
        &mut sections,
        "Head",
        "head",
        HEAD_KEYS,
        &["NetworkGame", "NetworkRuntimeJoin"],
        &["SaveGame", "Replay", "DisableMouse", "NoInitialize"],
        LegacyHead::apply_entries,
    );
    insert_validated_scenario_section::<LegacyDefinitions>(
        &tree,
        &mut sections,
        "Definitions",
        "definitions",
        DEFINITION_KEYS,
        &["LocalOnly", "AllowUserChange"],
        &[],
        LegacyDefinitions::apply_entries,
    );
    insert_validated_scenario_section::<LegacyGame>(
        &tree,
        &mut sections,
        "Game",
        "game",
        GAME_KEYS,
        &["EnableRemoveFlag", "StructNeedMaterial", "StructNeedEnergy"],
        &[],
        LegacyGame::apply_entries,
    );
    for player in 1..=MAX_PLAYER_STARTS {
        let source_name = format!("Player{player}");
        let storage_name = format!("player{player}");
        insert_validated_scenario_section::<LegacyPlayer>(
            &tree,
            &mut sections,
            &source_name,
            &storage_name,
            PLAYER_KEYS,
            &[],
            &["EnforcePosition"],
            LegacyPlayer::apply_entries,
        );
    }
    insert_validated_scenario_section::<LegacyLandscape>(
        &tree,
        &mut sections,
        "Landscape",
        "landscape",
        LANDSCAPE_KEYS,
        &[
            "ExactLandscape",
            "NoSky",
            "BottomOpen",
            "TopOpen",
            "AutoScanSideOpen",
            "MapPlayerExtend",
            "NoScan",
            "KeepMapCreator",
            "ShadeMaterials",
        ],
        &[],
        LegacyLandscape::apply_entries,
    );
    insert_validated_scenario_section::<LegacyWeather>(
        &tree,
        &mut sections,
        "Weather",
        "weather",
        WEATHER_KEYS,
        &["NoGamma"],
        &[],
        LegacyWeather::apply_entries,
    );
    insert_validated_scenario_section::<LegacyDisasters>(
        &tree,
        &mut sections,
        "Disasters",
        "disasters",
        &["Meteorite", "Volcano", "Earthquake"],
        &[],
        &[],
        LegacyDisasters::apply_entries,
    );
    insert_validated_scenario_section::<LegacyAnimals>(
        &tree,
        &mut sections,
        "Animals",
        "animals",
        &["Animal", "Nest"],
        &[],
        &[],
        LegacyAnimals::apply_entries,
    );
    insert_validated_scenario_section::<LegacyEnvironment>(
        &tree,
        &mut sections,
        "Environment",
        "environment",
        &["Objects"],
        &[],
        &[],
        LegacyEnvironment::apply_entries,
    );

    let title = sections
        .get("head")
        .and_then(|entries| find_rct_all_entry(entries, "Title"));
    let description = sections
        .get("head")
        .and_then(|entries| find_rct_all_entry(entries, "Description"));

    let ground_height_hint = derive_ground_height_hint(&sections);
    let mut core = LegacyScenarioCore::from_sections(&sections)?;
    apply_scenario_rct_all_strings(&mut core, &sections, true);
    let definition_specs = core.definitions.definitions.clone();

    Ok(LegacyScenarioManifest {
        title,
        description,
        head_title_native: None,
        definition_specs,
        ground_height_hint,
        core,
        sections,
    })
}

fn insert_validated_scenario_section<T: Default>(
    tree: &LegacyIniTree,
    sections: &mut HashMap<String, Vec<(String, String)>>,
    source_name: &str,
    storage_name: &str,
    allowed_keys: &[&str],
    bool_keys: &[&str],
    integer_bool_keys: &[&str],
    apply: fn(&mut T, &[(String, String)]) -> Result<(), ScenarioError>,
) {
    let Some(section) = tree.first_section(0, source_name) else {
        return;
    };
    let allowed = allowed_keys.iter().copied().collect::<HashSet<_>>();
    let mut seen = HashSet::new();
    let mut entries = Vec::new();
    for child in tree.nodes[section].children.iter().copied() {
        let node = &tree.nodes[child];
        if node.section || !allowed.contains(node.name.as_str()) || !seen.insert(node.name.clone())
        {
            continue;
        }
        let value = node.value.clone().unwrap_or_default();
        if bool_keys.contains(&node.name.as_str()) && parse_std_bool(&value).is_none() {
            continue;
        }
        if integer_bool_keys.contains(&node.name.as_str()) && parse_std_i32(&value).is_none() {
            continue;
        }
        let entry = (node.name.clone(), value);
        let mut probe = T::default();
        if apply(&mut probe, std::slice::from_ref(&entry)).is_ok() {
            entries.push(entry);
        }
    }
    sections.insert(storage_name.to_string(), entries);
}

fn find_rct_all_entry(entries: &[(String, String)], key: &str) -> Option<String> {
    entries
        .iter()
        .find(|(entry_key, _)| entry_key == key)
        .map(|(_, value)| parse_rct_all(value))
        .filter(|value| !value.is_empty())
}

fn apply_scenario_rct_all_strings(
    core: &mut LegacyScenarioCore,
    sections: &HashMap<String, Vec<(String, String)>>,
    compile_head: bool,
) {
    let raw = |section: &str, key: &str| {
        sections.get(section).and_then(|entries| {
            entries
                .iter()
                .find(|(entry_key, _)| entry_key == key)
                .map(|(_, value)| parse_rct_all(value))
        })
    };

    if compile_head {
        if let Some(value) = raw("head", "Title") {
            core.head.title = value;
        }
        if let Some(value) = raw("head", "Loader") {
            core.head.loader = value;
        }
        if let Some(value) = raw("head", "Font") {
            core.head.font = value;
        }
        if let Some(value) = raw("head", "Engine") {
            core.head.engine = value;
        }
        if let Some(value) = raw("head", "MissionAccess") {
            core.head.mission_access = truncate_legacy_c4_string(value, 512);
        }
        if let Some(value) = raw("head", "Origin") {
            core.head.origin = Some(validate_subpath_filename(value));
        }
    }
    if let Some(value) = raw("landscape", "Sky") {
        core.landscape.sky = (!value.is_empty()).then_some(value);
    }
    if let Some(value) = raw("landscape", "Material") {
        core.landscape.material = value;
    }
    if let Some(value) = raw("landscape", "Liquid") {
        core.landscape.liquid = value;
    }
    if let Some(value) = raw("weather", "Precipitation") {
        core.weather.precipitation = value;
    }
}

/// `C4InVal::VAL_SubPathFilename` plus C4SHead's platform separator
/// normalization (`C4Scenario.cpp:200-202`). Validation mutates bad input
/// rather than rejecting the scenario.
fn validate_subpath_filename(mut value: String) -> String {
    if value.is_empty() {
        value = "empty".to_string();
    }
    value = value.replace("..", "__");
    if value.starts_with('/') || value.starts_with('\\') {
        value.replace_range(..1, "_");
    }
    value = value
        .chars()
        .map(|character| match character {
            '*' | '?' | '<' | '>' | ';' | '|' | ':' => '_',
            '\\' if cfg!(not(windows)) => '/',
            '/' if cfg!(windows) => '\\',
            other => other,
        })
        .collect();
    value
}

pub(in crate::scenario) fn find_entry(entries: &[(String, String)], key: &str) -> Option<String> {
    find_entry_including_empty(entries, key)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(in crate::scenario) fn find_entry_including_empty<'a>(entries: &'a [(String, String)], key: &str) -> Option<&'a str> {
    entries
        .iter()
        .find(|(entry_key, _)| entry_key.eq_ignore_ascii_case(key))
        .map(|(_, value)| value.trim())
}

pub(in crate::scenario) fn normalize_definition_path(raw: &str) -> String {
    let mut trimmed = raw.trim().trim_matches(['"', '\''].as_ref());
    while let Some(stripped) = trimmed.strip_prefix("./") {
        trimmed = stripped;
    }
    while let Some(stripped) = trimmed.strip_prefix(".\\") {
        trimmed = stripped;
    }
    let normalized = trimmed.replace('\\', "/");
    normalized.trim_end_matches('/').to_string()
}

fn derive_ground_height_hint(sections: &HashMap<String, Vec<(String, String)>>) -> Option<i32> {
    let landscape = sections.get("landscape")?;
    let height = landscape
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("MapHeight"))
        .and_then(|(_, value)| parse_c4sval_std(value));
    let zoom = landscape
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("MapZoom"))
        .and_then(|(_, value)| parse_c4sval_std(value))
        .unwrap_or(1);
    height.map(|h| h.max(0).saturating_mul(zoom.max(1)))
}

fn parse_c4sval_std(value: &str) -> Option<i32> {
    let first = value.split(',').next()?.trim();
    if first.is_empty() {
        None
    } else {
        first.parse::<i32>().ok()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyC4SVal {
    pub std: i32,
    pub rnd: i32,
    pub min: i32,
    pub max: i32,
}

impl LegacyC4SVal {
    pub const fn new(std: i32, rnd: i32, min: i32, max: i32) -> Self {
        Self { std, rnd, min, max }
    }

    pub(crate) fn base(self) -> i32 {
        let (min, max) = ordered_bounds(self.min, self.max);
        self.std.clamp(min, max)
    }

    /// `C4SVal::Evaluate` (C4Scenario.cpp:43-46): one synced game-RNG draw,
    /// `BoundBy(Std + Random(2 * Rnd + 1) - Rnd, Min, Max)`. BoundBy makes no
    /// ordered-bounds assumption (Standard.h), so this avoids `clamp`'s
    /// min<=max panic.
    pub fn evaluate(self, rng: &mut crate::rng::LcgRng) -> i32 {
        let value = self.std + rng.random(2 * self.rnd + 1) - self.rnd;
        if value < self.min {
            self.min
        } else if value > self.max {
            self.max
        } else {
            value
        }
    }
}

const fn ordered_bounds(a: i32, b: i32) -> (i32, i32) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

pub(in crate::scenario) fn parse_legacy_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

/// `C4ComponentHost::LoadAppend` copies at most two native bytes from each
/// comma-separated language segment (C4ComponentHost.cpp:174-184).
fn legacy_script_language_code(language: &str) -> String {
    let code = clonk_script::c4_string_bytes(language);
    let visible = code
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(code.len());
    clonk_script::c4_string_from_bytes(&code[..visible.min(2)])
}

pub(in crate::scenario) fn load_legacy_scenario_script<S: AsRef<str>>(
    group: &Group,
    components: &ComponentGroups,
    languages: &[S],
) -> Result<Option<ScenarioScriptSource>, ScenarioError> {
    // C4CFN_Script is three independent LoadAppend segments. Each localized
    // segment restarts LanguageEx priority, and a failed read advances to the
    // next language without making scenario startup fail. The empty language
    // string still contributes one empty code, selecting Script.c a second
    // time through the Script{}.c segment (C4Components.h:55;
    // C4ComponentHost.cpp:155-220).
    const SCRIPT_SEGMENTS: [&str; 3] = ["Script.c", "Script{}.c", "C4Script{}.c"];
    let language_codes = languages
        .iter()
        .map(|language| legacy_script_language_code(language.as_ref()))
        .collect::<Vec<_>>();
    let mut assembled = Vec::new();
    for segment in SCRIPT_SEGMENTS {
        let selected = if segment.contains("{}") {
            if language_codes.is_empty() {
                group.read_file(segment.replacen("{}", "", 1)).ok()
            } else {
                language_codes
                    .iter()
                    .find_map(|code| group.read_file(segment.replacen("{}", code, 1)).ok())
            }
        } else {
            group.read_file(segment).ok()
        };
        let Some(bytes) = selected else {
            continue;
        };

        // LoadAppend prefixes every successfully read component, including
        // an empty one, and SCopy truncates only that component at its first
        // NUL before later segments are appended.
        assembled.push(b'\n');
        assembled.extend_from_slice(bytes.split(|byte| *byte == 0).next().unwrap_or_default());
    }

    let source = clonk_script::c4_string_from_bytes(&assembled);
    // C4ScriptHost passes the same two-byte LanguageEx segments to its
    // C4LangStringTable after component assembly.
    let source = localize_script_source_with_components(components, &source, &language_codes)?;
    // C4GameScriptHost exists even when every optional component is absent.
    // Retain that empty host and the canonical Script.c diagnostic name so
    // DirectExec/eval does not fall back to Game.ScriptEngine.
    Ok(Some(ScenarioScriptSource {
        name: group.root().join("Script.c").to_string_lossy().into_owned(),
        source,
        c4_args: true,
    }))
}

/// Byte-preserving C4LangStringTable::ReplaceStrings for Teams.txt. Unlike
/// C4ComponentHost, C4TeamList::Load does not call EnsureUnicode, so both the
/// source and replacement values remain in their original byte encoding
/// (C4Teams.cpp:614-655; C4LangStringTable.cpp:33-148).
fn localize_legacy_team_source<S: AsRef<str>>(
    components: &ComponentGroups,
    source: &[u8],
    languages: &[S],
) -> Result<Vec<u8>, GroupError> {
    let mut table = None;
    for candidate in std::iter::once("StringTbl.txt".to_owned()).chain(
        languages
            .iter()
            .map(|language| format!("StringTbl{}.txt", language.as_ref())),
    ) {
        if let Some(component) = components.read(candidate)? {
            table = Some(component.bytes);
            break;
        }
    }
    let Some(table) = table else {
        return Ok(source
            .split(|byte| *byte == 0)
            .next()
            .unwrap_or_default()
            .to_vec());
    };
    let table = table.split(|byte| *byte == 0).next().unwrap_or_default();
    let mut entries: Vec<(&[u8], &[u8])> = Vec::new();
    for line in table.split(|byte| matches!(*byte, b'\r' | b'\n')) {
        let Some(separator) = line.iter().position(|byte| *byte == b'=') else {
            continue;
        };
        let key = &line[..separator];
        if entries.iter().any(|(existing, _)| *existing == key) {
            continue;
        }
        entries.push((key, &line[separator + 1..]));
    }

    let source = source.split(|byte| *byte == 0).next().unwrap_or_default();
    let mut localized = Vec::with_capacity(source.len());
    let mut cursor = 0;
    while let Some(open_offset) = source[cursor..].iter().position(|byte| *byte == b'$') {
        let open = cursor + open_offset;
        let key_start = open + 1;
        let Some(close_offset) = source[key_start..].iter().position(|byte| *byte == b'$') else {
            break;
        };
        let close = key_start + close_offset;
        let key = &source[key_start..close];
        let valid = key.len() <= 30
            && key.iter().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(*byte, b'_' | b'~' | b'+' | b'-')
            });
        localized.extend_from_slice(&source[cursor..open]);
        if valid {
            if let Some((_, replacement)) = entries.iter().find(|(entry, _)| *entry == key) {
                localized.extend_from_slice(replacement);
            } else {
                localized.extend_from_slice(&source[open..=close]);
            }
        } else {
            localized.extend_from_slice(&source[open..=close]);
        }
        cursor = close + 1;
    }
    localized.extend_from_slice(&source[cursor..]);
    Ok(localized)
}

pub(in crate::scenario) fn load_initial_network_teams<S: AsRef<str>>(
    group: &Group,
    components: &ComponentGroups,
    languages: &[S],
) -> Result<(Vec<TeamInfo>, Option<LoadedLegacyTeamMetadata>), ScenarioError> {
    if !group.exists("Teams.txt") {
        return Ok((Vec::new(), None));
    }
    let source = group.read_file("Teams.txt")?;
    if source.is_empty() {
        // LoadEntryString rejects a zero-sized entry, selecting the same
        // scenario-derived path as a missing Teams.txt (C4Group.cpp:2243-2259;
        // C4Teams.cpp:619-647).
        return Ok((Vec::new(), None));
    }
    let source = localize_legacy_team_source(components, &source, languages)?;
    let source = bytes_as_latin1_string(&source);
    let loaded = parse_legacy_team_metadata_source(&source)?;
    let teams = team_infos_from_initial_network_metadata(&loaded.metadata);
    Ok((teams, Some(loaded)))
}

fn team_infos_from_initial_network_metadata(
    metadata: &InitialNetworkTeamMetadata,
) -> Vec<TeamInfo> {
    metadata
        .teams
        .iter()
        .map(|team| {
            TeamInfo::new(
                team.id,
                clonk_script::c4_string_from_bytes(team.name.as_bytes()),
                team.color,
            )
            .with_player_ids(
                team.player_ids
                    .iter()
                    .copied()
                    .filter(|player_id| *player_id > 0)
                    .collect(),
            )
            .with_player_start_index(team.player_start_index)
            .with_max_players(team.max_players)
            .with_icon_spec(clonk_script::c4_string_from_bytes(
                team.icon_spec.as_bytes(),
            ))
        })
        .collect()
}

fn apply_initial_network_team_strings(
    lobby: &mut ScenarioLobbyTeams,
    metadata: &InitialNetworkTeamMetadata,
) {
    lobby.script_player_names =
        clonk_script::c4_string_from_bytes(metadata.script_player_names.as_bytes());
    for (lobby_team, team) in lobby.teams.iter_mut().zip(&metadata.teams) {
        lobby_team.name = clonk_script::c4_string_from_bytes(team.name.as_bytes());
        let icon_spec = clonk_script::c4_string_from_bytes(team.icon_spec.as_bytes());
        lobby_team.icon_spec = (!icon_spec.is_empty()).then_some(icon_spec);
    }
}

#[derive(Default)]
struct LegacyTeamBuilder {
    id: i32,
    name: Vec<u8>,
    player_start_index: i32,
    player_count: i32,
    player_ids: Vec<i32>,
    color: u32,
    icon_spec: Vec<u8>,
    max_players: i32,
}

impl LegacyTeamBuilder {
    fn finish(self) -> Result<InitialNetworkTeam, ScenarioError> {
        let player_count = usize::try_from(self.player_count).map_err(|_| {
            ScenarioError::LegacyParse(format!(
                "Teams.txt team {} has negative PlayerCount {}",
                self.id, self.player_count
            ))
        })?;
        let mut player_ids = vec![-1; player_count];
        for (target, source) in player_ids.iter_mut().zip(self.player_ids) {
            *target = source;
        }
        Ok(InitialNetworkTeam {
            id: self.id,
            name: team_legacy_cstring(truncate_team_name(self.name), "Name")?,
            player_start_index: self.player_start_index,
            player_ids,
            color: self.color,
            icon_spec: team_legacy_cstring(self.icon_spec, "IconSpec")?,
            max_players: self.max_players,
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LegacyTeamSection {
    None,
    Teams,
    Team(usize),
    Other(usize),
}

pub(in crate::scenario) fn parse_legacy_team_metadata_source(
    source: &str,
) -> Result<LoadedLegacyTeamMetadata, ScenarioError> {
    let mut metadata = InitialNetworkTeamMetadata::teams_file_defaults();
    let mut section = LegacyTeamSection::None;
    let mut teams_indent = None;
    let mut current_team: Option<LegacyTeamBuilder> = None;
    let mut unsupported_team_distribution = None;

    for (index, raw_line) in legacy_ini_lines(source).enumerate() {
        let indent = raw_line
            .as_bytes()
            .iter()
            .take_while(|byte| matches!(**byte, b' ' | b'\t'))
            .count();
        let line = &raw_line[indent..];
        if line.is_empty()
            || line.starts_with(';')
            || line.starts_with('#')
            || line.starts_with("//")
        {
            continue;
        }
        if let Some(section_name) = line
            .strip_prefix('[')
            .and_then(|rest| rest.split_once(']').map(|(name, _)| name))
        {
            if let Some(team) = current_team.take() {
                metadata.teams.push(team.finish()?);
            }
            section = if section_name == "Teams" {
                teams_indent = Some(indent);
                LegacyTeamSection::Teams
            } else if section_name == "Team"
                && teams_indent.is_some_and(|teams_indent| indent > teams_indent)
            {
                current_team = Some(LegacyTeamBuilder::default());
                LegacyTeamSection::Team(indent)
            } else if teams_indent.is_some_and(|teams_indent| indent > teams_indent) {
                LegacyTeamSection::Other(indent)
            } else {
                teams_indent = None;
                LegacyTeamSection::None
            };
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let line_number = index + 1;
        match section {
            LegacyTeamSection::Teams
                if teams_indent.is_some_and(|teams_indent| indent + 1 > teams_indent) =>
            {
                apply_legacy_team_list_field(
                    &mut metadata,
                    &mut unsupported_team_distribution,
                    key,
                    value,
                    line_number,
                )?;
            }
            LegacyTeamSection::Team(team_indent) if indent + 1 > team_indent => {
                if let Some(team) = current_team.as_mut() {
                    apply_legacy_team_field(team, key, value, line_number)?;
                }
            }
            LegacyTeamSection::Other(child_indent) if indent + 1 > child_indent => {}
            LegacyTeamSection::Team(_) | LegacyTeamSection::Other(_) => {
                if let Some(team) = current_team.take() {
                    metadata.teams.push(team.finish()?);
                }
                section = LegacyTeamSection::Teams;
                if teams_indent.is_some_and(|teams_indent| indent + 1 > teams_indent) {
                    apply_legacy_team_list_field(
                        &mut metadata,
                        &mut unsupported_team_distribution,
                        key,
                        value,
                        line_number,
                    )?;
                }
            }
            LegacyTeamSection::None | LegacyTeamSection::Teams => {}
        }
    }
    if let Some(team) = current_team {
        metadata.teams.push(team.finish()?);
    }

    let largest_team_id = metadata.teams.iter().map(|team| team.id).fold(0, i32::max);
    metadata.last_team_id = metadata.last_team_id.max(largest_team_id);
    if metadata.teams.is_empty() {
        metadata.auto_generate_teams = true;
    }

    const DEFAULT_TEAM_COLORS: [u32; 10] = [
        0x00f4_0000,
        0x0000_c800,
        0x00fc_f41c,
        0x0020_20ff,
        0x00c4_8444,
        0x00ff_ffff,
        0x0084_8484,
        0x00ff_00ef,
        0x0000_ffff,
        0x0078_4830,
    ];
    let mut random_color_team_id = None;
    for team in &mut metadata.teams {
        if team.color != 0 {
            continue;
        }
        if let Some(color) = team
            .id
            .checked_sub(1)
            .and_then(|index| usize::try_from(index).ok())
            .and_then(|index| DEFAULT_TEAM_COLORS.get(index))
        {
            team.color = *color;
        } else if random_color_team_id.is_none() {
            // C++ calls process-global SafeRandom here, so scenario data alone
            // cannot reproduce the host's chosen snapshot color exactly
            // (C4Teams.cpp:181-218; C4PlayerInfoConflicts.cpp:36-41).
            random_color_team_id = Some(team.id);
        }
    }

    Ok(LoadedLegacyTeamMetadata {
        metadata,
        random_color_team_id,
        unsupported_team_distribution,
    })
}

fn apply_legacy_team_list_field(
    metadata: &mut InitialNetworkTeamMetadata,
    unsupported_team_distribution: &mut Option<u8>,
    key: &str,
    value: &str,
    line: usize,
) -> Result<(), ScenarioError> {
    match key {
        "Active" => {
            if let Some(value) = parse_team_bool(value) {
                metadata.active = value;
            }
        }
        "Custom" => {
            if let Some(value) = parse_team_bool(value) {
                metadata.custom = value;
            }
        }
        "AllowHostilityChange" => {
            if let Some(value) = parse_team_bool(value) {
                metadata.allow_hostility_change = value;
            }
        }
        "AllowTeamSwitch" => {
            if let Some(value) = parse_team_bool(value) {
                metadata.allow_team_switch = value;
            }
        }
        "AutoGenerateTeams" => {
            if let Some(value) = parse_team_bool(value) {
                metadata.auto_generate_teams = value;
            }
        }
        "LastTeamID" => metadata.last_team_id = parse_team_i32(key, value, line)?,
        "TeamDistribution" => {
            let (distribution, unsupported) = parse_team_distribution(value);
            if let Some(distribution) = distribution {
                metadata.team_distribution = distribution;
            }
            if unsupported.is_some() {
                *unsupported_team_distribution = unsupported;
            }
        }
        "TeamColors" => {
            if let Some(value) = parse_team_bool(value) {
                metadata.team_colors = value;
            }
        }
        "MaxScriptPlayers" => {
            metadata.max_script_players = parse_team_i32(key, value, line)?;
        }
        "ScriptPlayerNames" => {
            metadata.script_player_names = team_legacy_cstring(
                parse_team_escaped_bytes(value, line, key)?,
                "ScriptPlayerNames",
            )?;
        }
        "RandomTeamCount" => {
            metadata.random_team_count = parse_team_i32(key, value, line)?;
        }
        _ => {}
    }
    Ok(())
}

fn apply_legacy_team_field(
    team: &mut LegacyTeamBuilder,
    key: &str,
    value: &str,
    line: usize,
) -> Result<(), ScenarioError> {
    match key {
        "id" => team.id = parse_team_i32(key, value, line)?,
        "Name" => {
            team.name = latin1_string_as_bytes(value.trim_start_matches([' ', '\t']), line, key)?;
        }
        "PlrStartIndex" => team.player_start_index = parse_team_i32(key, value, line)?,
        "PlayerCount" => team.player_count = parse_team_i32(key, value, line)?,
        "Players" => {
            team.player_ids = value
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| parse_team_i32(key, value, line))
                .collect::<Result<_, _>>()?;
        }
        "Color" => {
            let parsed = parse_i64(value).map_err(|error| {
                team_parse_error(line, format!("invalid {key} `{value}`: {error}"))
            })?;
            team.color = u32::try_from(parsed).map_err(|_| {
                team_parse_error(line, format!("{key} `{value}` is outside uint32"))
            })?;
        }
        "IconSpec" => team.icon_spec = parse_team_escaped_bytes(value, line, key)?,
        "MaxPlayer" => team.max_players = parse_team_i32(key, value, line)?,
        _ => {}
    }
    Ok(())
}

pub(in crate::scenario) fn parse_team_bool(value: &str) -> Option<bool> {
    let bytes = value.as_bytes();
    if bytes.first() == Some(&b'1') && bytes.get(1).is_none_or(|byte| !byte.is_ascii_digit()) {
        Some(true)
    } else if bytes.first() == Some(&b'0') && bytes.get(1).is_none_or(|byte| !byte.is_ascii_digit())
    {
        Some(false)
    } else if bytes.starts_with(b"true") {
        Some(true)
    } else if bytes.starts_with(b"false") {
        Some(false)
    } else {
        None
    }
}

pub(in crate::scenario) fn load_legacy_teams<S: AsRef<str>>(
    group: &Group,
    components: &ComponentGroups,
    languages: &[S],
    core: &LegacyScenarioCore,
) -> Result<(Vec<TeamInfo>, ScenarioLobbyTeams), ScenarioError> {
    if !group.exists("Teams.txt") {
        return Ok((Vec::new(), derive_legacy_teams_default(core)));
    }
    let source = group.read_file("Teams.txt")?;
    if source.is_empty() {
        // C4Group::LoadEntryString rejects a zero-sized entry, so the lobby
        // projection must take the same scenario-derived branch as runtime.
        return Ok((Vec::new(), derive_legacy_teams_default(core)));
    }
    let source = localize_legacy_team_source(components, &source, languages)?;
    // C4Group::LoadEntryString and C4LangStringTable::ReplaceStrings keep
    // Teams.txt as native bytes. Parse the decoded projection only for the
    // existing lobby/configuration semantics, then replace every C4 string
    // with the byte-exact values used by C4TeamList and the script runtime.
    let exact = parse_legacy_team_metadata_source(&bytes_as_latin1_string(&source))?;
    let mut metadata = parse_legacy_teams_source(&decode_legacy_script_text(&source));
    apply_initial_network_team_strings(&mut metadata, &exact.metadata);
    let teams = team_infos_from_initial_network_metadata(&exact.metadata);
    Ok((teams, metadata))
}

#[derive(Debug)]
struct LegacyIniNode {
    name: String,
    value: Option<String>,
    section: bool,
    indent: isize,
    parent: Option<usize>,
    children: Vec<usize>,
}

#[derive(Debug)]
pub(in crate::scenario) struct LegacyIniTree {
    nodes: Vec<LegacyIniNode>,
}

impl LegacyIniTree {
    pub(in crate::scenario) fn parse(source: &str) -> Self {
        let mut tree = Self {
            nodes: vec![LegacyIniNode {
                name: String::new(),
                value: None,
                section: true,
                indent: -1,
                parent: None,
                children: Vec::new(),
            }],
        };
        let mut current = 0;
        for line in legacy_ini_lines(source) {
            let indent = line
                .as_bytes()
                .iter()
                .take_while(|byte| matches!(**byte, b' ' | b'\t'))
                .count();
            let bytes = line.as_bytes();
            let mut position = indent;
            let section = bytes.get(position) == Some(&b'[')
                && bytes.get(position + 1).is_some_and(u8::is_ascii_alphabetic);
            if section {
                position += 1;
            } else if !bytes.get(position).is_some_and(u8::is_ascii_alphabetic) {
                continue;
            }
            let name_start = position;
            while bytes
                .get(position)
                .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b' ' | b'_'))
            {
                position += 1;
            }
            let name = &line[name_start..position];
            while bytes
                .get(position)
                .is_some_and(|byte| matches!(*byte, b' ' | b'\t'))
            {
                position += 1;
            }
            let expected = if section { b']' } else { b'=' };
            if bytes.get(position) != Some(&expected) {
                continue;
            }
            position += 1;
            let node_indent = (indent + usize::from(!section)) as isize;
            while current != 0 && tree.nodes[current].indent >= node_indent {
                current = tree.nodes[current].parent.unwrap_or(0);
            }
            let index = tree.nodes.len();
            tree.nodes.push(LegacyIniNode {
                name: name.to_string(),
                value: (!section).then(|| line[position..].to_string()),
                section,
                indent: node_indent,
                parent: Some(current),
                children: Vec::new(),
            });
            tree.nodes[current].children.push(index);
            if section {
                current = index;
            }
        }
        tree
    }

    pub(in crate::scenario) fn first_section(&self, parent: usize, name: &str) -> Option<usize> {
        self.nodes[parent]
            .children
            .iter()
            .copied()
            .find(|index| self.nodes[*index].section && self.nodes[*index].name == name)
    }

    pub(in crate::scenario) fn sections(&self, parent: usize, name: &str) -> impl Iterator<Item = usize> + '_ {
        let name = name.to_string();
        self.nodes[parent]
            .children
            .iter()
            .copied()
            .filter(move |index| self.nodes[*index].section && self.nodes[*index].name == name)
    }

    pub(in crate::scenario) fn value(&self, parent: usize, name: &str) -> Option<&str> {
        self.nodes[parent]
            .children
            .iter()
            .copied()
            .find(|index| !self.nodes[*index].section && self.nodes[*index].name == name)
            .and_then(|index| self.nodes[index].value.as_deref())
    }

    fn has_value(&self, parent: usize, name: &str) -> bool {
        self.value(parent, name).is_some()
    }
}

pub(in crate::scenario) fn parse_legacy_teams_source(source: &str) -> ScenarioLobbyTeams {
    let tree = LegacyIniTree::parse(source);
    let Some(section) = tree.first_section(0, "Teams") else {
        return ScenarioLobbyTeams {
            source: ScenarioTeamsSource::TeamsFile,
            active: true,
            custom: true,
            allow_hostility_change: false,
            allow_team_switch: false,
            configured_auto_generate: false,
            auto_generate: true,
            configured_last_team_id: 0,
            last_team_id: 0,
            distribution: ScenarioTeamDistribution::Free,
            team_colors: false,
            max_script_players: 0,
            script_player_names: String::new(),
            random_team_count: 0,
            teams: Vec::new(),
        };
    };
    let configured_auto_generate = ini_bool(&tree, section, "AutoGenerateTeams", false);
    let configured_last_team_id = ini_i32(&tree, section, "LastTeamID", 0);
    let mut teams = Vec::new();
    for team_section in tree.sections(section, "Team") {
        let player_count = ini_i32(&tree, team_section, "PlayerCount", 0);
        teams.push(ScenarioLobbyTeam {
            id: ini_i32(&tree, team_section, "id", 0),
            name: truncate_legacy_string(ini_rct_all(&tree, team_section, "Name", ""), 30),
            player_start_index: ini_i32(&tree, team_section, "PlrStartIndex", 0),
            player_count,
            players: ini_i32_array(&tree, team_section, "Players", player_count, -1),
            configured_color: ini_u32(&tree, team_section, "Color", 0),
            icon_spec: {
                let value = ini_std_string(&tree, team_section, "IconSpec", "");
                (!value.is_empty()).then_some(value)
            },
            max_players: ini_i32(&tree, team_section, "MaxPlayer", 0),
        });
    }
    let mut metadata = ScenarioLobbyTeams {
        source: ScenarioTeamsSource::TeamsFile,
        active: ini_bool(&tree, section, "Active", true),
        custom: ini_bool(&tree, section, "Custom", true),
        allow_hostility_change: ini_bool(&tree, section, "AllowHostilityChange", false),
        allow_team_switch: ini_bool(&tree, section, "AllowTeamSwitch", false),
        configured_auto_generate,
        auto_generate: configured_auto_generate,
        configured_last_team_id,
        last_team_id: configured_last_team_id,
        distribution: ini_team_distribution(&tree, section),
        team_colors: ini_bool(&tree, section, "TeamColors", false),
        max_script_players: ini_i32(&tree, section, "MaxScriptPlayers", 0),
        script_player_names: ini_std_string(&tree, section, "ScriptPlayerNames", ""),
        random_team_count: ini_i32(&tree, section, "RandomTeamCount", 0),
        teams,
    };

    // C4TeamList::CompileFunc performs these two post-compile adjustments.
    if metadata.teams.is_empty() {
        metadata.auto_generate = true;
    }
    let largest = metadata.teams.iter().map(|team| team.id).fold(0, i32::max);
    metadata.last_team_id = metadata.last_team_id.max(largest);
    metadata
}

pub(in crate::scenario) fn derive_legacy_teams_default(core: &LegacyScenarioCore) -> ScenarioLobbyTeams {
    let melee = matches!(core.game.mode, 1 | 2)
        || core.game.goals.iter().any(|entry| {
            entry.id.eq_ignore_ascii_case("MELE") || entry.id.eq_ignore_ascii_case("MEL2")
        })
        || core
            .game
            .rules
            .iter()
            .any(|entry| entry.id.eq_ignore_ascii_case("RVLR"));
    ScenarioLobbyTeams {
        source: ScenarioTeamsSource::DerivedScenarioDefault,
        active: melee,
        custom: false,
        allow_hostility_change: true,
        allow_team_switch: false,
        configured_auto_generate: false,
        auto_generate: melee,
        configured_last_team_id: 0,
        last_team_id: 0,
        distribution: ScenarioTeamDistribution::Free,
        team_colors: false,
        max_script_players: 0,
        script_player_names: String::new(),
        random_team_count: 0,
        teams: Vec::new(),
    }
}

pub(in crate::scenario) fn legacy_game_is_melee_after_conversion(game: &LegacyGame) -> bool {
    matches!(game.mode, 1 | 2)
        || ["MELE", "MEL2"]
            .iter()
            .any(|id| first_legacy_id_count(&game.goals, id, 0) != 0)
}

pub(in crate::scenario) fn legacy_effective_min_players(core: &LegacyScenarioCore) -> i32 {
    if core.head.min_player != 0 {
        core.head.min_player
    } else if legacy_game_is_melee_after_conversion(&core.game) {
        2
    } else {
        1
    }
}

pub(in crate::scenario) fn load_legacy_game_parameter_overrides(
    group: &Group,
    defaults: &ScenarioGameParameterValues,
) -> Result<Option<ScenarioGameParameterOverrides>, ScenarioError> {
    if !group.exists("Parameters.txt") {
        return Ok(None);
    }
    let source = decode_legacy_script_text(&group.read_file("Parameters.txt")?);
    Ok(Some(parse_legacy_game_parameter_overrides(
        &source, defaults,
    )))
}

pub(in crate::scenario) fn load_savegame_definition_override(
    group: &Group,
    save_game: bool,
) -> Result<ScenarioSavegameDefinitionOverride, ScenarioError> {
    if !save_game {
        return Ok(ScenarioSavegameDefinitionOverride::None);
    }
    let Some(bytes) = try_read_group_file_case_insensitive(group, "Game.txt")? else {
        return Ok(ScenarioSavegameDefinitionOverride::None);
    };
    let source = decode_legacy_script_text(&bytes);
    let Some(position) = source.find("[DefinitionFiles]") else {
        return Ok(ScenarioSavegameDefinitionOverride::None);
    };
    let mut definition_lines = Vec::new();
    let mut found = false;
    for line in source[position..].lines().skip(1) {
        if line.starts_with("Definition") && line.contains('=') {
            found = true;
            definition_lines.push(line.to_string());
        } else if found {
            break;
        }
    }
    Ok(ScenarioSavegameDefinitionOverride::GameText { definition_lines })
}

pub(in crate::scenario) fn load_runtime_landscape_data(
    group: &Group,
    savegame_defaults: bool,
) -> Result<Option<LandscapeGameData>, ScenarioError> {
    Ok(
        match try_read_group_file_case_insensitive(group, "Game.txt")? {
            Some(bytes) => Some(parse_landscape_game_data(&bytes)),
            None if savegame_defaults => Some(LandscapeGameData::default()),
            None => None,
        },
    )
}

pub(in crate::scenario) fn load_runtime_current_scenario_section(group: &Group) -> Result<String, ScenarioError> {
    let current = try_read_group_file_case_insensitive(group, "Game.txt")?
        .map(|bytes| crate::parse_initial_network_game_data(&bytes).current_scenario_section)
        .unwrap_or_default();
    Ok(if current.is_empty() {
        "main".to_string()
    } else {
        current
    })
}

pub(in crate::scenario) fn load_legacy_round_results(
    group: &Group,
    melee: bool,
) -> Result<RoundResultsState, ScenarioError> {
    let Some(source) = try_read_group_file_case_insensitive(group, "RoundResults.txt")? else {
        // C4Game calls RoundResults.Init when the component is absent. Init
        // changes only this scenario-derived default on a freshly-cleared
        // game instance (C4Game.cpp:2477-2486; C4RoundResults.cpp:240-245).
        return Ok(RoundResultsState {
            hide_settlement_score: melee,
            ..RoundResultsState::default()
        });
    };

    RoundResultsState::from_legacy_ini(&source, melee)
        .map_err(ScenarioError::LegacyRoundResultsParse)
}

pub(in crate::scenario) fn parse_legacy_game_parameter_overrides(
    source: &str,
    defaults: &ScenarioGameParameterValues,
) -> ScenarioGameParameterOverrides {
    let tree = LegacyIniTree::parse(source);
    let section = tree.first_section(0, "Parameters");
    let mut overrides = ScenarioGameParameterOverrides {
        random_seed: None,
        max_players: None,
        startup_player_count: None,
        use_fair_crew: None,
        fair_crew_forced: None,
        fair_crew_strength: None,
        allow_debug: None,
        is_network_game: None,
        control_rate: None,
        auto_frame_skip: None,
        rules: None,
        goals: None,
        league: None,
        clients: Vec::new(),
    };
    let Some(section) = section else {
        return overrides;
    };
    overrides.random_seed = ini_optional_i32(&tree, section, "RandomSeed", defaults.random_seed);
    overrides.startup_player_count = ini_optional_i32(
        &tree,
        section,
        "StartupPlayerCount",
        defaults.startup_player_count,
    );
    overrides.max_players = ini_optional_i32(&tree, section, "MaxPlayers", defaults.max_players);
    overrides.use_fair_crew =
        ini_optional_bool(&tree, section, "UseFairCrew", defaults.use_fair_crew);
    overrides.fair_crew_forced =
        ini_optional_bool(&tree, section, "FairCrewForced", defaults.fair_crew_forced);
    overrides.fair_crew_strength = ini_optional_i32(
        &tree,
        section,
        "FairCrewStrength",
        defaults.fair_crew_strength,
    );
    overrides.allow_debug = ini_optional_bool(&tree, section, "AllowDebug", defaults.allow_debug);
    overrides.is_network_game =
        ini_optional_bool(&tree, section, "IsNetworkGame", defaults.is_network_game);
    overrides.control_rate = ini_optional_i32(&tree, section, "ControlRate", defaults.control_rate);
    overrides.auto_frame_skip =
        ini_optional_bool(&tree, section, "AutoFrameSkip", defaults.auto_frame_skip);
    overrides.rules = ini_optional_id_list(&tree, section, "Rules", &defaults.rules);
    overrides.goals = ini_optional_id_list(&tree, section, "Goals", &defaults.goals);
    overrides.league = tree
        .has_value(section, "League")
        .then(|| ini_std_string(&tree, section, "League", &defaults.league));
    overrides.clients = tree
        .sections(section, "Client")
        .map(|client| ScenarioLobbyClient {
            id: ini_i32(&tree, client, "ID", -1),
            activated: ini_bool(&tree, client, "Activated", false),
            observer: ini_bool(&tree, client, "Observer", false),
            name: ini_validated_client_name(&tree, client, "Name"),
            nick: ini_validated_client_name(&tree, client, "Nick"),
            lobby_ready: ini_bool(&tree, client, "LobbyReady", false),
        })
        .collect();
    // C4ClientList::Add inserts each compiled client by ascending ID.
    overrides.clients.sort_by_key(ScenarioLobbyClient::id);
    overrides
}

pub(in crate::scenario) fn game_parameter_defaults(core: &LegacyScenarioCore) -> ScenarioGameParameterValues {
    let (rules, goals) = converted_legacy_rules_and_goals(&core.game);
    ScenarioGameParameterValues {
        random_seed: core.head.random_seed,
        startup_player_count: 0,
        max_players: core.head.max_player,
        use_fair_crew: core.head.forced_fair_crew == 1,
        fair_crew_forced: core.head.forced_fair_crew != 0,
        fair_crew_strength: core.head.fair_crew_strength,
        allow_debug: true,
        is_network_game: false,
        control_rate: -1,
        auto_frame_skip: false,
        rules: lobby_id_entries(&rules),
        goals: lobby_id_entries(&goals),
        league: String::new(),
        clients: Vec::new(),
    }
}

fn converted_legacy_rules_and_goals(game: &LegacyGame) -> (LegacyIdList, LegacyIdList) {
    let mut rules = game.rules.clone();
    let mut goals = game.goals.clone();
    if matches!(game.mode, 1 | 2) {
        set_first_legacy_id(&mut goals, "MELE", 1);
    }
    match game.cooperative_goal {
        1 => set_first_legacy_id(&mut goals, "GLDM", 1),
        2 => set_first_legacy_id(&mut goals, "MNTK", 1),
        3 => set_first_legacy_id(&mut goals, "VALG", (game.value_gain / 100).max(1)),
        _ => {}
    }
    if game.realism.construction_needs_material {
        set_first_legacy_id(&mut rules, "CNMT", 1);
    }
    if game.realism.structures_need_energy {
        set_first_legacy_id(&mut rules, "ENRG", 1);
    }
    if game.enable_remove_flag {
        set_first_legacy_id(&mut rules, "FGRV", 1);
    }
    match game.elimination {
        0 => set_first_legacy_id(&mut rules, "KILC", 1),
        2 => {
            set_first_legacy_id(&mut rules, "CTFL", 1);
            set_first_legacy_id(&mut rules, "FGRV", 1);
        }
        _ => {}
    }
    if first_legacy_id_count(&rules, "CTFL", 0) != 0 {
        set_first_legacy_id(&mut rules, "FGRV", 1);
    }
    (rules, goals)
}

fn first_legacy_id_count(list: &LegacyIdList, id: &str, zero_default: i32) -> i32 {
    list.iter()
        .find(|entry| entry.id.eq_ignore_ascii_case(id))
        .map_or(0, |entry| match entry.count.unwrap_or(0) {
            0 => zero_default,
            count => count,
        })
}

fn set_first_legacy_id(list: &mut LegacyIdList, id: &str, count: i32) {
    if let Some(entry) = list
        .iter_mut()
        .find(|entry| entry.id.eq_ignore_ascii_case(id))
    {
        entry.count = Some(count);
    } else {
        list.push(LegacyIdEntry {
            id: id.to_string(),
            count: Some(count),
        });
    }
}

fn lobby_id_entries(list: &LegacyIdList) -> Vec<ScenarioLobbyIdEntry> {
    list.iter()
        .map(|entry| ScenarioLobbyIdEntry {
            id: entry.id.clone(),
            count: entry.count.unwrap_or(0),
        })
        .collect()
}

pub(in crate::scenario) fn ini_i32(tree: &LegacyIniTree, parent: usize, name: &str, default: i32) -> i32 {
    tree.value(parent, name)
        .and_then(parse_std_i32)
        .unwrap_or(default)
}

fn ini_optional_i32(tree: &LegacyIniTree, parent: usize, name: &str, default: i32) -> Option<i32> {
    tree.has_value(parent, name)
        .then(|| ini_i32(tree, parent, name, default))
}

pub(in crate::scenario) fn ini_u32(tree: &LegacyIniTree, parent: usize, name: &str, default: u32) -> u32 {
    tree.value(parent, name)
        .and_then(parse_std_i64)
        .map(|value| value as u32)
        .unwrap_or(default)
}

pub(in crate::scenario) fn ini_bool(tree: &LegacyIniTree, parent: usize, name: &str, default: bool) -> bool {
    tree.value(parent, name)
        .and_then(parse_std_bool)
        .unwrap_or(default)
}

fn ini_optional_bool(
    tree: &LegacyIniTree,
    parent: usize,
    name: &str,
    default: bool,
) -> Option<bool> {
    tree.has_value(parent, name)
        .then(|| ini_bool(tree, parent, name, default))
}

fn ini_rct_all(tree: &LegacyIniTree, parent: usize, name: &str, default: &str) -> String {
    tree.value(parent, name)
        .map(parse_rct_all)
        .unwrap_or_else(|| default.to_string())
}

fn ini_std_string(tree: &LegacyIniTree, parent: usize, name: &str, default: &str) -> String {
    tree.value(parent, name)
        .map(parse_std_string)
        .unwrap_or_else(|| default.to_string())
}

fn ini_validated_client_name(tree: &LegacyIniTree, parent: usize, name: &str) -> String {
    let Some(raw) = tree.value(parent, name) else {
        // A missing naming takes DefaultAdapt's empty default without running
        // ValidatedStdStrBuf::CompileFunc, so validation is deliberately not
        // applied here.
        return String::new();
    };
    let value = parse_std_string(raw).replace('{', "");
    let value = value.trim().to_string();
    if value.is_empty() {
        "Unknown".to_string()
    } else {
        truncate_legacy_string(value, 30)
    }
}

fn ini_i32_array(
    tree: &LegacyIniTree,
    parent: usize,
    name: &str,
    count: i32,
    default: i32,
) -> Vec<i32> {
    let Ok(count) = usize::try_from(count) else {
        return Vec::new();
    };
    let mut values = vec![default; count];
    if let Some(raw) = tree.value(parent, name) {
        let component_defaults = vec![default; count];
        compile_defaulted_i32_components(raw, &mut values, &component_defaults, true);
    }
    values
}

fn ini_optional_id_list(
    tree: &LegacyIniTree,
    parent: usize,
    name: &str,
    default: &[ScenarioLobbyIdEntry],
) -> Option<Vec<ScenarioLobbyIdEntry>> {
    let raw = tree.value(parent, name)?;
    Some(
        parse_legacy_id_list(name, parse_rct_all(raw).as_str())
            .map(|list| lobby_id_entries(&list))
            .unwrap_or_else(|_| default.to_vec()),
    )
}

fn ini_team_distribution(tree: &LegacyIniTree, parent: usize) -> ScenarioTeamDistribution {
    let Some(raw) = tree.value(parent, "TeamDistribution") else {
        return ScenarioTeamDistribution::Free;
    };
    if let Some(value) = parse_std_i64(raw) {
        let value = if value < 0 {
            u8::MAX
        } else {
            value.min(u8::MAX as i64) as u8
        };
        return match value {
            0 => ScenarioTeamDistribution::Free,
            1 => ScenarioTeamDistribution::Host,
            2 => ScenarioTeamDistribution::None,
            3 => ScenarioTeamDistribution::Random,
            4 => ScenarioTeamDistribution::RandomInvisible,
            value => ScenarioTeamDistribution::Numeric(value),
        };
    }
    match parse_identifier(raw) {
        Some("Free") => ScenarioTeamDistribution::Free,
        Some("Host") => ScenarioTeamDistribution::Host,
        Some("None") => ScenarioTeamDistribution::None,
        Some("Random") => ScenarioTeamDistribution::Random,
        Some("RandomInv") => ScenarioTeamDistribution::RandomInvisible,
        Some(name) => {
            tracing::warn!(name, "unknown legacy TeamDistribution; using Free");
            ScenarioTeamDistribution::Free
        }
        None => ScenarioTeamDistribution::Free,
    }
}

pub(in crate::scenario) fn parse_std_i32(raw: &str) -> Option<i32> {
    parse_std_i64(raw).and_then(|value| i32::try_from(value).ok())
}

pub(in crate::scenario) fn parse_std_u32(raw: &str) -> Option<u32> {
    let raw = raw.trim_start_matches([' ', '\t']);
    let bytes = raw.as_bytes();
    let mut cursor = 0;

    // StdCompilerINIRead selects hexadecimal only when the number itself
    // begins with 0x. A leading sign therefore keeps strtoul in base 10.
    let radix = if bytes.get(cursor) == Some(&b'0')
        && bytes
            .get(cursor + 1)
            .is_some_and(|byte| matches!(byte, b'x' | b'X'))
    {
        cursor += 2;
        16u32
    } else {
        10u32
    };
    let negative = if radix == 10 {
        match bytes.get(cursor) {
            Some(b'-') => {
                cursor += 1;
                true
            }
            Some(b'+') => {
                cursor += 1;
                false
            }
            _ => false,
        }
    } else {
        false
    };
    let digits_start = cursor;
    let mut magnitude = 0u128;
    while let Some(digit) = bytes.get(cursor).and_then(|byte| match byte {
        b'0'..=b'9' => Some(u32::from(*byte - b'0')),
        b'a'..=b'f' if radix == 16 => Some(u32::from(*byte - b'a') + 10),
        b'A'..=b'F' if radix == 16 => Some(u32::from(*byte - b'A') + 10),
        _ => None,
    }) {
        if digit >= radix {
            break;
        }
        magnitude = magnitude
            .saturating_mul(u128::from(radix))
            .saturating_add(u128::from(digit));
        cursor += 1;
    }
    if cursor == digits_start {
        // strtoul("0x", ..., 16) still consumes the leading zero.
        return (radix == 16).then_some(0);
    }

    let c_ulong_bits = std::mem::size_of::<std::os::raw::c_ulong>() * 8;
    let c_ulong_max = (1u128 << c_ulong_bits) - 1;
    let unsigned = if magnitude > c_ulong_max {
        c_ulong_max
    } else if negative {
        0u128.wrapping_sub(magnitude) & c_ulong_max
    } else {
        magnitude
    };
    Some(unsigned as u32)
}

pub(in crate::scenario) fn compile_defaulted_i32_components(
    raw: &str,
    values: &mut [i32],
    defaults: &[i32],
    fill_rest_on_separator_failure: bool,
) {
    debug_assert_eq!(values.len(), defaults.len());
    let mut position = 0;
    for index in 0..values.len() {
        if index != 0 && !consume_std_separator(raw, &mut position, b',') {
            if fill_rest_on_separator_failure {
                values[index..].copy_from_slice(&defaults[index..]);
            }
            break;
        }
        values[index] = parse_std_i32_prefix_at(raw, &mut position).unwrap_or(defaults[index]);
    }
}

pub(in crate::scenario) fn skip_std_whitespace(raw: &str, position: &mut usize) {
    while raw
        .as_bytes()
        .get(*position)
        .is_some_and(|byte| matches!(*byte, b' ' | b'\t'))
    {
        *position += 1;
    }
}

pub(in crate::scenario) fn consume_std_separator(raw: &str, position: &mut usize, separator: u8) -> bool {
    skip_std_whitespace(raw, position);
    if raw.as_bytes().get(*position) != Some(&separator) {
        return false;
    }
    *position += 1;
    true
}

pub(in crate::scenario) fn parse_std_i32_prefix_at(raw: &str, position: &mut usize) -> Option<i32> {
    skip_std_whitespace(raw, position);
    let start = *position;
    let bytes = raw.as_bytes();
    let signed = matches!(bytes.get(start), Some(b'+' | b'-'));
    let sign_length = usize::from(signed);
    let unsigned_start = start + sign_length;
    let hexadecimal = !signed
        && bytes.get(unsigned_start) == Some(&b'0')
        && matches!(bytes.get(unsigned_start + 1), Some(b'x' | b'X'));
    let digit_start = unsigned_start + usize::from(hexadecimal) * 2;
    if digit_start > bytes.len() {
        return None;
    }
    let digit_length = bytes[digit_start..]
        .iter()
        .take_while(|byte| {
            if hexadecimal {
                byte.is_ascii_hexdigit()
            } else {
                byte.is_ascii_digit()
            }
        })
        .count();
    if digit_length == 0 {
        return None;
    }
    let end = digit_start + digit_length;
    *position = end;
    let digits = std::str::from_utf8(&bytes[digit_start..end]).ok()?;
    let magnitude = i64::from_str_radix(digits, if hexadecimal { 16 } else { 10 }).ok()?;
    let signed_value = if bytes.get(start) == Some(&b'-') {
        magnitude.checked_neg()?
    } else {
        magnitude
    };
    i32::try_from(signed_value).ok()
}

fn parse_std_i64(raw: &str) -> Option<i64> {
    let raw = raw.trim_start_matches([' ', '\t']);
    let (sign, rest, had_sign) = if let Some(rest) = raw.strip_prefix('-') {
        (-1_i64, rest, true)
    } else if let Some(rest) = raw.strip_prefix('+') {
        (1_i64, rest, true)
    } else {
        (1_i64, raw, false)
    };
    let (radix, digits) = if !had_sign {
        rest.strip_prefix("0x")
            .or_else(|| rest.strip_prefix("0X"))
            .map_or((10, rest), |digits| (16, digits))
    } else {
        (10, rest)
    };
    let length = digits
        .bytes()
        .take_while(|byte| match radix {
            16 => byte.is_ascii_hexdigit(),
            _ => byte.is_ascii_digit(),
        })
        .count();
    if length == 0 {
        return None;
    }
    i64::from_str_radix(&digits[..length], radix)
        .ok()
        .and_then(|value| value.checked_mul(sign))
}

fn parse_std_bool(raw: &str) -> Option<bool> {
    if raw.starts_with('1') && !raw.as_bytes().get(1).is_some_and(u8::is_ascii_digit) {
        Some(true)
    } else if raw.starts_with('0') && !raw.as_bytes().get(1).is_some_and(u8::is_ascii_digit) {
        Some(false)
    } else if raw.starts_with("true") {
        Some(true)
    } else if raw.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

fn parse_team_i32(key: &str, value: &str, line: usize) -> Result<i32, ScenarioError> {
    parse_i32(value)
        .map_err(|error| team_parse_error(line, format!("invalid {key} `{value}`: {error}")))
}

pub(in crate::scenario) fn parse_team_distribution(value: &str) -> (Option<InitialNetworkTeamDistribution>, Option<u8>) {
    let value = value.trim_start_matches([' ', '\t']);
    let identifier_end = value
        .bytes()
        .position(|byte| !(byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')))
        .unwrap_or(value.len());
    let named = match &value[..identifier_end] {
        "Free" => Some(InitialNetworkTeamDistribution::Free),
        "Host" => Some(InitialNetworkTeamDistribution::Host),
        "None" => Some(InitialNetworkTeamDistribution::None),
        "Random" => Some(InitialNetworkTeamDistribution::Random),
        "RandomInv" => Some(InitialNetworkTeamDistribution::RandomInvisible),
        _ => None,
    };
    if named.is_some() {
        return (named, None);
    }

    let starts_numeric = value
        .as_bytes()
        .first()
        .is_some_and(|byte| byte.is_ascii_digit() || matches!(*byte, b'+' | b'-'));
    if !starts_numeric {
        return (None, None);
    }
    let parsed = parse_i64(value).unwrap_or(0);
    let numeric = if parsed < 0 {
        u8::MAX
    } else {
        parsed.min(i64::from(u8::MAX)) as u8
    };
    let known = match numeric {
        0 => Some(InitialNetworkTeamDistribution::Free),
        1 => Some(InitialNetworkTeamDistribution::Host),
        2 => Some(InitialNetworkTeamDistribution::None),
        3 => Some(InitialNetworkTeamDistribution::Random),
        4 => Some(InitialNetworkTeamDistribution::RandomInvisible),
        _ => None,
    };
    if known.is_some() {
        (known, None)
    } else {
        (None, Some(numeric))
    }
}

fn parse_team_escaped_bytes(
    value: &str,
    line: usize,
    field: &str,
) -> Result<Vec<u8>, ScenarioError> {
    // StdStrBuf's escaped reader falls back to RCT_All when the first byte is
    // not a quote. RCT_All skips leading space/tab but retains the tail
    // verbatim (StdCompiler.cpp:734-743,897-998).
    if !value.starts_with('"') {
        return latin1_string_as_bytes(value.trim_start_matches([' ', '\t']), line, field);
    }
    parse_legacy_object_name(value, line)
        .map(|value| value.unwrap_or_default())
        .map_err(|error| team_parse_error(line, format!("invalid {field}: {error}")))
        .and_then(|value| latin1_string_as_bytes(&value, line, field))
}

fn team_parse_error(line: usize, detail: String) -> ScenarioError {
    ScenarioError::LegacyParse(format!("Teams.txt line {line}: {detail}"))
}

fn truncate_team_name(mut name: Vec<u8>) -> Vec<u8> {
    const C4_MAX_NAME: usize = 30;
    if name.len() > C4_MAX_NAME {
        name.truncate(C4_MAX_NAME);
    }
    name
}

pub(in crate::scenario) fn bytes_as_latin1_string(bytes: &[u8]) -> String {
    bytes.iter().copied().map(char::from).collect()
}

fn latin1_string_as_bytes(value: &str, line: usize, field: &str) -> Result<Vec<u8>, ScenarioError> {
    value
        .chars()
        .map(|character| {
            u8::try_from(u32::from(character)).map_err(|_| {
                team_parse_error(
                    line,
                    format!(
                        "{field} contains a non-byte character U+{:04X}",
                        u32::from(character)
                    ),
                )
            })
        })
        .collect()
}

fn team_legacy_cstring(bytes: Vec<u8>, field: &str) -> Result<LegacyCString, ScenarioError> {
    LegacyCString::from_bytes(bytes).ok_or_else(|| {
        ScenarioError::LegacyParse(format!("Teams.txt {field} contains an interior NUL"))
    })
}

fn parse_identifier(raw: &str) -> Option<&str> {
    let raw = raw.trim_start_matches([' ', '\t']);
    let length = raw
        .bytes()
        .take_while(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'_' | b'-'))
        .count();
    (length > 0).then(|| &raw[..length])
}

fn parse_rct_all(raw: &str) -> String {
    raw.trim_start_matches([' ', '\t']).to_string()
}

fn truncate_legacy_string(value: String, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        value
    } else {
        String::from_utf8_lossy(&value.as_bytes()[..max_bytes]).into_owned()
    }
}

fn truncate_legacy_c4_string(value: String, max_bytes: usize) -> String {
    let bytes = clonk_script::c4_string_bytes(&value);
    let visible_len = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len())
        .min(max_bytes);
    clonk_script::c4_string_from_bytes(&bytes[..visible_len])
}

fn parse_std_string(raw: &str) -> String {
    match raw.strip_prefix('"') {
        Some(escaped) => decode_legacy_escaped_string(escaped),
        None => parse_rct_all(raw),
    }
}

fn decode_legacy_escaped_string(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut output = Vec::new();
    let mut index = 0;
    while let Some(&byte) = bytes.get(index) {
        if byte == b'"' {
            break;
        }
        if byte != b'\\' {
            output.push(byte);
            index += 1;
            continue;
        }
        index += 1;
        let Some(&escaped) = bytes.get(index) else {
            break;
        };
        index += 1;
        match escaped {
            b'a' => output.push(b'\x07'),
            b'b' => output.push(b'\x08'),
            b'f' => output.push(b'\x0c'),
            b'n' => output.push(b'\n'),
            b'r' => output.push(b'\r'),
            b't' => output.push(b'\t'),
            b'v' => output.push(b'\x0b'),
            b'\'' => output.push(b'\''),
            b'"' => output.push(b'"'),
            b'\\' => output.push(b'\\'),
            b'?' => output.push(b'?'),
            b'x' => {
                let start = index;
                while bytes.get(index).is_some_and(u8::is_ascii_hexdigit) {
                    index += 1;
                }
                if index == start {
                    output.push(b'x');
                } else if let Ok(hex) = std::str::from_utf8(&bytes[start..index]) {
                    output.push(u32::from_str_radix(hex, 16).unwrap_or(0) as u8);
                }
            }
            b'0'..=b'7' => {
                let mut value = u32::from(escaped - b'0');
                while let Some(next @ b'0'..=b'7') = bytes.get(index).copied() {
                    value = value * 8 + u32::from(next - b'0');
                    index += 1;
                }
                output.push(value as u8);
            }
            other => output.push(other),
        }
    }
    String::from_utf8_lossy(&output).into_owned()
}

/// Collects and localizes the scripts of a System.c4g group in the group's
/// existing entry order, matching C4Group::FindNextEntry and the shared
/// C4LangStringTable passed to every host (C4Game.cpp:2777-2791,3346-3355).
pub fn load_system_scripts(group: &Group) -> Result<Vec<(String, String)>, ScenarioError> {
    load_system_scripts_with_components(group, &ComponentGroups::local(group), &["US", "DE"])
}

/// Pack-aware System.c4g script loader. `components` must represent the
/// System group itself; script files remain local while its StringTbl is
/// selected through C4ComponentHost::LoadEx's local-plus-pack group set.
pub fn load_system_scripts_with_components<S: AsRef<str>>(
    group: &Group,
    components: &ComponentGroups,
    languages: &[S],
) -> Result<Vec<(String, String)>, ScenarioError> {
    let mut sources = Vec::new();
    for entry in group.entries()? {
        if !legacy_group_wildcard_match(b"*.c", &entry.name_bytes) {
            continue;
        }
        let name = clonk_script::c4_string_from_bytes(&entry.name_bytes);
        let bytes = match group.read_entry_bytes_exact(&entry) {
            Ok(bytes) => bytes,
            Err(_) => {
                // C4Game registers the host before C4ScriptHost::Load and
                // ignores a load failure, so later matching entries remain.
                sources.push((name, String::new()));
                continue;
            }
        };
        let source = clonk_script::c4_string_from_bytes(&bytes);
        let source = localize_script_source_with_components(components, &source, languages)?;
        sources.push((name, source));
    }
    Ok(sources)
}

/// The scenario's own System.c4g scripts, empty when the group has none
/// (C4Game::LoadScenarioScripts opens C4CFN_System as a child and loads
/// every C4CFN_ScriptFiles entry, C4Game.cpp:3317-3343).
pub(in crate::scenario) fn load_scenario_system_scripts<S: AsRef<str>>(
    group: &Group,
    language_packs: &LanguagePacks,
    scenario_origin: Option<&str>,
    languages: &[S],
) -> Result<Vec<(String, String)>, ScenarioError> {
    group
        .open_child(Path::new("System.c4g"))
        .ok()
        .map(|system| {
            let components = language_packs.component_groups(&system, Some(group), scenario_origin);
            load_system_scripts_with_components(&system, &components, languages)
        })
        .unwrap_or_else(|| Ok(Vec::new()))
}

/// Evaluate `[Landscape] MapZoom` with the C4S default
/// `C4SVal(10, 0, 5, 15)` (C4Scenario.cpp:307,353) against the local
/// FixRandom map-creation ledger.
pub(in crate::scenario) fn legacy_map_zoom(section: Option<&Vec<(String, String)>>, rng: &mut crate::rng::LcgRng) -> u32 {
    legacy_map_zoom_value(section).evaluate(rng) as u32
}

pub(in crate::scenario) fn legacy_map_zoom_value(section: Option<&Vec<(String, String)>>) -> LegacyC4SVal {
    let default = LegacyC4SVal::new(10, 0, 5, 15);
    section
        .and_then(|entries| find_entry_including_empty(entries, "mapzoom"))
        .and_then(|raw| parse_legacy_c4s_value("MapZoom", raw, default).ok())
        .unwrap_or(default)
}

pub(in crate::scenario) fn legacy_random_seed(fallback: u64) -> u64 {
    std::env::var("LC_RUST_ENGINE_RANDOM_SEED")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(fallback)
}
