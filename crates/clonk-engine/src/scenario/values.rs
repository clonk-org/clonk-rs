//! `scenario` — moved verbatim from the parent module.
//!
//! Structural only: same crate, same items, same bodies.

use super::*;

/// The fully defaulted `C4Scenario::CompileFunc` view retained at runtime for
/// `GetScenarioVal`.  Values stay in compiler traversal order because
/// `C4ValueGetCompiler` treats `entry_nr` as an index over primitive callbacks
/// (C4Script.cpp:3997-4006), including alternating ID/count list entries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[doc(hidden)]
pub struct ScenarioValueStore {
    pub(crate) sections: Vec<ScenarioValueSection>,
    #[serde(default)]
    core: LegacyScenarioCore,
    #[serde(default)]
    section_head_defaults: Option<[i32; 2]>,
}

impl Default for ScenarioValueStore {
    fn default() -> Self {
        let mut core = LegacyScenarioCore::default();
        // C4Scenario's main-file compiler default differs from
        // C4SRealism::Default (C4Scenario.cpp:237-238).
        core.game.realism.landscape_insert_thrust = 1;
        Self::from_runtime_core(&core, false)
    }
}

impl ScenarioValueStore {
    pub(crate) fn legacy_weather_init(&self) -> LegacyWeatherInit {
        LegacyWeatherInit {
            season: self.core.weather.start_season,
            year_speed: self.core.weather.year_speed,
            climate: self.core.weather.climate,
            wind: self.core.weather.wind,
            rain: self.core.weather.rain,
            precipitation: self.core.weather.precipitation.clone(),
            lightning: self.core.weather.lightning,
            meteorite: self.core.disasters.meteorite,
            volcano: self.core.disasters.volcano,
            earthquake: self.core.disasters.earthquake,
            no_initialize: self.core.head.no_initialize != 0,
            no_gamma: self.core.weather.no_gamma,
        }
    }

    pub(in crate::scenario) fn with_section_head_defaults(mut self, context: &LegacyHead) -> Self {
        self.section_head_defaults = Some([
            context.forced_auto_context_menu,
            context.forced_control_style,
        ]);
        self
    }

    pub(crate) fn serialize_runtime_network_save(
        &self,
        scenario_title: &str,
        definition_modules: &[String],
        definition_executable_path: &str,
        definition_path: &str,
        scenario_origin: &str,
    ) -> Vec<u8> {
        self.core
            .runtime_network_save(
                scenario_title,
                definition_modules,
                definition_executable_path,
                definition_path,
                scenario_origin,
            )
            .serialize()
    }

    /// `C4GameSaveScenario`'s non-initial Scenario.txt rewrite.  Unlike an
    /// exact save this deliberately keeps the scenario's own title,
    /// definition list and Origin while clearing only the fields changed by
    /// the common `C4GameSave::SaveCore` path.
    pub(crate) fn serialize_runtime_scenario_save(&self) -> Vec<u8> {
        self.core.runtime_scenario_save().serialize()
    }

    /// `C4GameSaveSavegame`'s non-initial Scenario.txt rewrite.  The caller
    /// supplies the already-derived icon because the native specialization
    /// obtains it from the destination group's trailing slot number.
    pub(crate) fn serialize_runtime_savegame(
        &self,
        scenario_title: &str,
        definition_modules: &[String],
        definition_executable_path: &str,
        definition_path: &str,
        scenario_origin: &str,
        icon: i32,
    ) -> Vec<u8> {
        self.core
            .runtime_savegame(
                scenario_title,
                definition_modules,
                definition_executable_path,
                definition_path,
                scenario_origin,
                icon,
            )
            .serialize()
    }

    /// Non-initial `C4GameSaveRecord` uses the synchronized exact-save core,
    /// then marks the scenario as a replay with the fixed record icon.
    pub(crate) fn serialize_runtime_record_save(
        &self,
        record_title: &str,
        definition_modules: &[String],
        definition_executable_path: &str,
        definition_path: &str,
        scenario_origin: &str,
    ) -> Vec<u8> {
        self.core
            .runtime_record_save(
                record_title,
                definition_modules,
                definition_executable_path,
                definition_path,
                scenario_origin,
            )
            .serialize()
    }

    /// Initial-record projection of an already-restored exact savegame.
    /// The runtime store, rather than the source `Scenario`, owns mutations
    /// made before the JSON save was written.
    pub(crate) fn serialize_initial_record_from_runtime_savegame(
        &self,
        record_title: &str,
        definition_modules: &[String],
        definition_executable_path: &str,
        definition_path: &str,
        scenario_origin: &str,
    ) -> Vec<u8> {
        self.core
            .runtime_exact_save_core(
                record_title,
                definition_modules,
                definition_executable_path,
                definition_path,
                scenario_origin,
            )
            .initial_record_save(
                record_title,
                definition_modules,
                definition_executable_path,
                definition_path,
                scenario_origin,
            )
            .serialize()
    }

    pub(crate) fn serialize_section_save(&self, force_exact: bool) -> Vec<u8> {
        self.core
            .serialize_section(force_exact, self.section_head_defaults)
    }

    pub(crate) fn no_sky(&self) -> bool {
        self.core.landscape.no_sky
    }

    #[cfg(test)]
    pub(crate) fn with_film_for_test(film: i32) -> Self {
        let mut core = LegacyScenarioCore::default();
        core.head.film = film;
        Self::from_runtime_core(&core, false)
    }

    #[cfg(test)]
    pub(crate) fn with_replay_film_for_test(replay: i32, film: i32) -> Self {
        let mut core = LegacyScenarioCore::default();
        core.head.replay = replay;
        core.head.film = film;
        Self::from_runtime_core(&core, false)
    }

    #[cfg(test)]
    pub(crate) fn with_value_gain_for_test(value_gain: i32) -> Self {
        let mut core = LegacyScenarioCore::default();
        core.game.value_gain = value_gain;
        Self::from_runtime_core(&core, false)
    }

    #[cfg(test)]
    pub(crate) fn with_landscape_push_pull_for_test(enabled: bool) -> Self {
        let mut core = LegacyScenarioCore::default();
        core.game.realism.landscape_push_pull = i32::from(enabled);
        Self::from_runtime_core(&core, false)
    }

    #[cfg(test)]
    pub(crate) fn with_no_sky_for_test(enabled: bool) -> Self {
        let mut core = LegacyScenarioCore::default();
        core.landscape.no_sky = enabled;
        Self::from_runtime_core(&core, false)
    }

    fn entry(name: &'static str, values: Vec<ScenarioValue>) -> ScenarioValueEntry {
        ScenarioValueEntry {
            name: name.to_string(),
            values,
        }
    }

    fn ints(values: impl IntoIterator<Item = i32>) -> Vec<ScenarioValue> {
        values.into_iter().map(ScenarioValue::Int).collect()
    }

    fn trimmed_ints<const N: usize>(values: [i32; N], default: i32) -> Vec<ScenarioValue> {
        let len = values
            .iter()
            .rposition(|value| *value != default)
            .map_or(0, |index| index + 1);
        Self::ints(values.into_iter().take(len))
    }

    fn c4s(value: LegacyC4SVal) -> Vec<ScenarioValue> {
        Self::ints([value.std, value.rnd, value.min, value.max])
    }

    fn c4id(value: &str) -> ScenarioValue {
        if value.len() != 4 || value == "NONE" || value == "0000" {
            ScenarioValue::C4Id(String::new())
        } else {
            ScenarioValue::C4Id(value.to_string())
        }
    }

    fn ids(values: &LegacyIdList) -> Vec<ScenarioValue> {
        values
            .iter()
            .flat_map(|entry| {
                [
                    Self::c4id(&entry.id),
                    ScenarioValue::Int(entry.count.unwrap_or(0)),
                ]
            })
            .collect()
    }

    fn names(values: &LegacyNameList) -> Vec<ScenarioValue> {
        values
            .iter()
            .flat_map(|entry| {
                [
                    ScenarioValue::String(entry.name.clone()),
                    ScenarioValue::Int(entry.count.unwrap_or(0)),
                ]
            })
            .collect()
    }

    fn from_core(core: &LegacyScenarioCore) -> Self {
        let head = &core.head;
        let mut sections = vec![ScenarioValueSection {
            name: "Head".to_string(),
            entries: vec![
                Self::entry("Icon", Self::ints([head.icon])),
                Self::entry("Title", vec![ScenarioValue::String(head.title.clone())]),
                Self::entry("Loader", vec![ScenarioValue::String(head.loader.clone())]),
                Self::entry("Font", vec![ScenarioValue::String(head.font.clone())]),
                Self::entry("Version", Self::trimmed_ints(head.version, 0)),
                Self::entry("Difficulty", Self::ints([head.difficulty])),
                // C4SHead::CompileFunc reflects a local, permanently-zero
                // compatibility value for the obsolete Access entry.
                Self::entry("Access", Self::ints([0])),
                Self::entry("MaxPlayer", Self::ints([head.max_player])),
                Self::entry("MaxPlayerLeague", Self::ints([head.max_player_league])),
                Self::entry("MinPlayer", Self::ints([head.min_player])),
                Self::entry("SaveGame", Self::ints([head.save_game])),
                Self::entry("Replay", Self::ints([head.replay])),
                Self::entry("Film", Self::ints([head.film])),
                Self::entry("DisableMouse", Self::ints([head.disable_mouse])),
                Self::entry("NoInitialize", Self::ints([head.no_initialize])),
                Self::entry("RandomSeed", Self::ints([head.random_seed])),
                Self::entry(
                    "ForcedAutoContextMenu",
                    Self::ints([head.forced_auto_context_menu]),
                ),
                Self::entry(
                    "ForcedAutoStopControl",
                    Self::ints([head.forced_control_style]),
                ),
                Self::entry("Engine", vec![ScenarioValue::String(head.engine.clone())]),
                Self::entry(
                    "MissionAccess",
                    vec![ScenarioValue::String(head.mission_access.clone())],
                ),
                Self::entry("NetworkGame", vec![ScenarioValue::Bool(head.network_game)]),
                Self::entry(
                    "NetworkRuntimeJoin",
                    vec![ScenarioValue::Bool(head.network_runtime_join)],
                ),
                Self::entry("ForcedGfxMode", Self::ints([head.forced_gfx_mode])),
                Self::entry("ForcedNoCrew", Self::ints([head.forced_fair_crew])),
                Self::entry("DefCrewStrength", Self::ints([head.fair_crew_strength])),
                Self::entry(
                    "Origin",
                    vec![ScenarioValue::String(
                        head.origin.clone().unwrap_or_default(),
                    )],
                ),
            ],
        }];

        let definitions = &core.definitions;
        sections.push(ScenarioValueSection {
            name: "Definitions".to_string(),
            entries: vec![
                Self::entry(
                    "LocalOnly",
                    vec![ScenarioValue::Bool(definitions.local_only)],
                ),
                Self::entry(
                    "AllowUserChange",
                    vec![ScenarioValue::Bool(definitions.allow_user_change)],
                ),
                Self::entry(
                    "Definitions",
                    definitions
                        .reflected_definitions
                        .as_ref()
                        .unwrap_or(&definitions.definitions)
                        .iter()
                        .cloned()
                        .map(ScenarioValue::String)
                        .collect(),
                ),
                Self::entry("SkipDefs", Self::ids(&definitions.skip_defs)),
            ],
        });

        let game = &core.game;
        sections.push(ScenarioValueSection {
            name: "Game".to_string(),
            entries: vec![
                Self::entry("Mode", Self::ints([game.mode])),
                Self::entry("Elimination", Self::ints([game.elimination])),
                Self::entry("CooperativeGoal", Self::ints([game.cooperative_goal])),
                Self::entry("CreateObjects", Self::ids(&game.create_objects)),
                Self::entry("ClearObjects", Self::ids(&game.clear_objects)),
                Self::entry("ClearMaterials", Self::names(&game.clear_materials)),
                Self::entry("ValueGain", Self::ints([game.value_gain])),
                Self::entry(
                    "EnableRemoveFlag",
                    vec![ScenarioValue::Bool(game.enable_remove_flag)],
                ),
                Self::entry(
                    "StructNeedMaterial",
                    vec![ScenarioValue::Bool(
                        game.realism.construction_needs_material,
                    )],
                ),
                Self::entry(
                    "StructNeedEnergy",
                    vec![ScenarioValue::Bool(game.realism.structures_need_energy)],
                ),
                Self::entry("ValueOverloads", Self::ids(&game.realism.value_overloads)),
                Self::entry(
                    "LandscapePushPull",
                    Self::ints([game.realism.landscape_push_pull]),
                ),
                Self::entry(
                    "LandscapeInsertThrust",
                    Self::ints([game.realism.landscape_insert_thrust]),
                ),
                Self::entry(
                    "BaseFunctionality",
                    Self::ints([game.realism.base_functionality]),
                ),
                Self::entry(
                    "BaseRegenerateEnergyPrice",
                    Self::ints([game.realism.base_regenerate_energy_price]),
                ),
                Self::entry("Goals", Self::ids(&game.goals)),
                Self::entry("Rules", Self::ids(&game.rules)),
                Self::entry("FoWColor", Self::ints([game.fow_color as i32])),
            ],
        });

        for index in 0..MAX_PLAYER_STARTS {
            let player = core.players.get(index).cloned().unwrap_or_default();
            sections.push(ScenarioValueSection {
                name: format!("Player{}", index + 1),
                entries: vec![
                    Self::entry(
                        "StandardCrew",
                        vec![Self::c4id(
                            player.standard_crew.as_deref().unwrap_or_default(),
                        )],
                    ),
                    Self::entry("Clonks", Self::c4s(player.clonks)),
                    Self::entry("Wealth", Self::c4s(player.wealth)),
                    Self::entry("Position", Self::trimmed_ints(player.position, -1)),
                    Self::entry("EnforcePosition", Self::ints([player.enforce_position])),
                    Self::entry("Crew", Self::ids(&player.crew)),
                    Self::entry("Buildings", Self::ids(&player.buildings)),
                    Self::entry("Vehicles", Self::ids(&player.vehicles)),
                    Self::entry("Material", Self::ids(&player.material)),
                    Self::entry("Knowledge", Self::ids(&player.knowledge)),
                    Self::entry("HomeBaseMaterial", Self::ids(&player.home_base_material)),
                    Self::entry(
                        "HomeBaseProduction",
                        Self::ids(&player.home_base_production),
                    ),
                    Self::entry("Magic", Self::ids(&player.magic)),
                ],
            });
        }

        let landscape = &core.landscape;
        sections.push(ScenarioValueSection {
            name: "Landscape".to_string(),
            entries: vec![
                Self::entry(
                    "ExactLandscape",
                    vec![ScenarioValue::Bool(landscape.exact_landscape)],
                ),
                Self::entry("Vegetation", Self::ids(&landscape.vegetation)),
                Self::entry("VegetationLevel", Self::c4s(landscape.vegetation_level)),
                Self::entry("InEarth", Self::ids(&landscape.in_earth)),
                Self::entry("InEarthLevel", Self::c4s(landscape.in_earth_level)),
                Self::entry(
                    "Sky",
                    vec![ScenarioValue::String(
                        landscape.sky.clone().unwrap_or_default(),
                    )],
                ),
                Self::entry("SkyFade", Self::trimmed_ints(landscape.sky_fade, 0)),
                Self::entry("NoSky", vec![ScenarioValue::Bool(landscape.no_sky)]),
                Self::entry(
                    "BottomOpen",
                    vec![ScenarioValue::Bool(landscape.bottom_open)],
                ),
                Self::entry("TopOpen", vec![ScenarioValue::Bool(landscape.top_open)]),
                Self::entry("LeftOpen", Self::ints([landscape.left_open])),
                Self::entry("RightOpen", Self::ints([landscape.right_open])),
                Self::entry(
                    "AutoScanSideOpen",
                    vec![ScenarioValue::Bool(landscape.auto_scan_side_open)],
                ),
                Self::entry("MapWidth", Self::c4s(landscape.map_width)),
                Self::entry("MapHeight", Self::c4s(landscape.map_height)),
                Self::entry("MapZoom", Self::c4s(landscape.map_zoom)),
                Self::entry("Amplitude", Self::c4s(landscape.amplitude)),
                Self::entry("Phase", Self::c4s(landscape.phase)),
                Self::entry("Period", Self::c4s(landscape.period)),
                Self::entry("Random", Self::c4s(landscape.random)),
                Self::entry(
                    "Material",
                    vec![ScenarioValue::String(landscape.material.clone())],
                ),
                Self::entry(
                    "Liquid",
                    vec![ScenarioValue::String(landscape.liquid.clone())],
                ),
                Self::entry("LiquidLevel", Self::c4s(landscape.liquid_level)),
                Self::entry(
                    "MapPlayerExtend",
                    vec![ScenarioValue::Bool(landscape.map_player_extend)],
                ),
                Self::entry("Layers", Self::names(&landscape.layers)),
                Self::entry("Gravity", Self::c4s(landscape.gravity)),
                Self::entry("NoScan", vec![ScenarioValue::Bool(landscape.no_scan)]),
                Self::entry(
                    "KeepMapCreator",
                    vec![ScenarioValue::Bool(landscape.keep_map_creator)],
                ),
                Self::entry("SkyScrollMode", Self::ints([landscape.sky_scroll_mode])),
                Self::entry(
                    "NewStyleLandscape",
                    Self::ints([landscape.new_style_landscape]),
                ),
                Self::entry("FoWRes", Self::ints([landscape.fow_resolution])),
                Self::entry(
                    "ShadeMaterials",
                    vec![ScenarioValue::Bool(landscape.shade_materials)],
                ),
            ],
        });

        sections.push(ScenarioValueSection {
            name: "Animals".to_string(),
            entries: vec![
                Self::entry("Animal", Self::ids(&core.animals.free_life)),
                Self::entry("Nest", Self::ids(&core.animals.earth_nest)),
            ],
        });

        let weather = &core.weather;
        sections.push(ScenarioValueSection {
            name: "Weather".to_string(),
            entries: vec![
                Self::entry("Climate", Self::c4s(weather.climate)),
                Self::entry("StartSeason", Self::c4s(weather.start_season)),
                Self::entry("YearSpeed", Self::c4s(weather.year_speed)),
                Self::entry("Rain", Self::c4s(weather.rain)),
                Self::entry("Wind", Self::c4s(weather.wind)),
                Self::entry("Lightning", Self::c4s(weather.lightning)),
                Self::entry(
                    "Precipitation",
                    vec![ScenarioValue::String(weather.precipitation.clone())],
                ),
                Self::entry("NoGamma", vec![ScenarioValue::Bool(weather.no_gamma)]),
            ],
        });

        sections.push(ScenarioValueSection {
            name: "Disasters".to_string(),
            entries: vec![
                Self::entry("Meteorite", Self::c4s(core.disasters.meteorite)),
                Self::entry("Volcano", Self::c4s(core.disasters.volcano)),
                Self::entry("Earthquake", Self::c4s(core.disasters.earthquake)),
            ],
        });

        sections.push(ScenarioValueSection {
            name: "Environment".to_string(),
            entries: vec![Self::entry("Objects", Self::ids(&core.environment.objects))],
        });

        Self {
            sections,
            core: core.clone(),
            section_head_defaults: None,
        }
    }

    /// Project the state visible to scripts after C4Scenario::Load,
    /// ConvertGoals, and the initial C4Landscape/C4Sky initialization, which
    /// all precede scenario `Initialize` (C4Scenario.cpp:86-97;
    /// C4Landscape.cpp:569-570,677; C4Sky.cpp:84-91).
    pub(in crate::scenario) fn from_runtime_core(
        core: &LegacyScenarioCore,
        has_sky_surface: bool,
    ) -> Self {
        let mut runtime = core.after_load_conversion();
        runtime.landscape.map_width.max = 10_000;
        runtime.landscape.map_height.max = 10_000;
        runtime.landscape.new_style_landscape = 2;
        if !has_sky_surface {
            runtime.landscape.sky = runtime.landscape.sky.map(|sky| sky.replace(',', ";"));
        }
        Self::from_core(&runtime)
    }

    /// `C4SGame::IsMelee`: inspect the post-ConvertGoals C4IDList and use
    /// the first exact MELE/MEL2 entry's count for each id.
    pub(crate) fn is_melee(&self) -> bool {
        let goals = self
            .sections
            .iter()
            .find(|section| section.name == "Game")
            .and_then(|section| section.entries.iter().find(|entry| entry.name == "Goals"))
            .map(|entry| entry.values.as_slice())
            .unwrap_or_default();

        ["MELE", "MEL2"].into_iter().any(|wanted| {
            goals
                .chunks(2)
                .find_map(|pair| {
                    let ScenarioValue::C4Id(id) = pair.first()? else {
                        return None;
                    };
                    (id == wanted).then(|| {
                        pair.get(1)
                            .and_then(|value| match value {
                                ScenarioValue::Int(count) => Some(*count),
                                _ => None,
                            })
                            .unwrap_or(0)
                    })
                })
                .is_some_and(|count| count != 0)
        })
    }

    pub(crate) fn landscape_push_pull(&self) -> bool {
        matches!(
            self.get("LandscapePushPull", Some("Game"), 0),
            Some(ScenarioValue::Int(value)) if *value != 0
        )
    }

    /// Runtime `Game.C4S.Game.FoWColor`, retaining the packed unsigned C4
    /// color bits even though `GetScenarioVal` exposes the primitive as an
    /// `int32_t`.
    pub(crate) fn fow_color(&self) -> u32 {
        match self.get("FoWColor", Some("Game"), 0) {
            Some(ScenarioValue::Int(value)) => *value as u32,
            _ => 0,
        }
    }

    /// Runtime `Game.C4S.Landscape.FoWRes`. The fully defaulted scenario
    /// compiler stores `CClrModAddMap::iDefResolutionX` (64) here.
    pub(crate) fn fow_resolution(&self) -> i32 {
        match self.get("FoWRes", Some("Landscape"), 0) {
            Some(ScenarioValue::Int(value)) => *value,
            _ => crate::DEFAULT_FOW_RESOLUTION,
        }
    }

    pub(crate) fn scenario_title(&self) -> &str {
        match self.get("Title", Some("Head"), 0) {
            Some(ScenarioValue::String(title)) => title,
            _ => "",
        }
    }

    /// Mirrors C4ValueGetCompiler's traversal: with no section, same-name
    /// fields in successive sections contribute to one primitive stream;
    /// with a section, only that named C4Scenario child is traversed.
    pub(crate) fn get(
        &self,
        entry: &str,
        section: Option<&str>,
        entry_nr: i32,
    ) -> Option<&ScenarioValue> {
        let mut remaining = usize::try_from(entry_nr).ok()?;
        for candidate in self
            .sections
            .iter()
            .filter(|candidate| section.is_none_or(|name| candidate.name == name))
        {
            // In the one-name form a root section with the requested name
            // becomes the active match. Its named children are then one
            // level too deep for haveCompleteMatch(), so a same-name child
            // (notably [Definitions].Definitions) is shadowed rather than
            // returned (C4Script.cpp:3958-3989).
            if section.is_none() && candidate.name == entry {
                continue;
            }
            for field in candidate.entries.iter().filter(|field| field.name == entry) {
                if remaining < field.values.len() {
                    return field.values.get(remaining);
                }
                remaining -= field.values.len();
            }
        }
        None
    }
}

pub(in crate::scenario) fn parse_bool_field(field: &str, raw: &str) -> Result<bool, ScenarioError> {
    if let Some(value) = parse_legacy_bool(raw) {
        return Ok(value);
    }
    match parse_i32(raw) {
        Ok(value) => Ok(value != 0),
        Err(err) => Err(ScenarioError::LegacyParse(format!(
            "invalid boolean for `{field}`: {err}"
        ))),
    }
}

/// C4IDList entries as (id, count) pairs — a bare id compiles count 0
/// (mkDefaultAdapt(count, 0), C4IDList.cpp:252).
pub(in crate::scenario) fn id_list_pairs(list: &LegacyIdList) -> Vec<(String, i32)> {
    list.iter()
        .map(|entry| (entry.id.clone(), entry.count.unwrap_or(0)))
        .collect()
}

pub(in crate::scenario) fn scenario_id_list_entries(
    list: &LegacyIdList,
) -> Vec<ScenarioIdListEntry> {
    list.iter()
        .map(|entry| ScenarioIdListEntry::new(entry.id.clone(), entry.count.unwrap_or(0)))
        .collect()
}

pub(in crate::scenario) fn set_legacy_id_count(list: &mut LegacyIdList, id: &str, count: i32) {
    if let Some(entry) = list.iter_mut().find(|entry| entry.id == id) {
        entry.count = Some(count);
    } else {
        list.push(LegacyIdEntry {
            id: id.to_owned(),
            count: Some(count),
        });
    }
}

pub(in crate::scenario) fn legacy_id_count(list: &LegacyIdList, id: &str) -> i32 {
    list.iter()
        .find(|entry| entry.id == id)
        .and_then(|entry| entry.count)
        .unwrap_or(0)
}

pub(in crate::scenario) fn legacy_id_count_or(
    list: &LegacyIdList,
    id: &str,
    zero_default: i32,
) -> i32 {
    list.iter()
        .find(|entry| entry.id == id)
        .map(|entry| entry.count.unwrap_or(0))
        .map(|count| if count == 0 { zero_default } else { count })
        .unwrap_or(0)
}

pub(in crate::scenario) fn parse_legacy_id_list(
    _field: &str,
    raw: &str,
) -> Result<LegacyIdList, ScenarioError> {
    let mut entries = Vec::new();
    let mut position = 0;
    let mut first = true;
    loop {
        if !first && !consume_std_separator(raw, &mut position, b';') {
            break;
        }
        first = false;

        skip_std_whitespace(raw, &mut position);
        let id_start = position;
        while position < raw.len()
            && position - id_start < 4
            && is_std_identifier_byte(raw.as_bytes()[position])
        {
            position += 1;
        }
        let id = &raw[id_start..position];
        if !looks_like_compiled_c4id(id) {
            // C4IDList::Entry throws after C4IDAdapt has read at most four
            // identifier bytes. StdSTLContainerAdapt keeps earlier entries
            // and stops before inserting this invalid one.
            break;
        }
        let count = if consume_std_separator(raw, &mut position, b'=') {
            Some(parse_std_i32_prefix_at(raw, &mut position).unwrap_or(0))
        } else {
            None
        };
        entries.push(LegacyIdEntry {
            id: id.to_string(),
            count,
        });
    }
    Ok(entries)
}

fn looks_like_compiled_c4id(id: &str) -> bool {
    if id.len() != 4 || id == "NONE" {
        return false;
    }
    if id.bytes().all(|byte| byte.is_ascii_digit()) {
        return id.parse::<u16>().is_ok_and(|id| id != 0);
    }
    id.bytes()
        .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
}

pub(in crate::scenario) fn is_std_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')
}

fn consume_name_list_separator(
    raw: &str,
    position: &mut Option<usize>,
    reenter: &mut Option<usize>,
    separator: u8,
) -> bool {
    // StdCompilerINIRead::Separator parks a mismatched cursor in pReenter;
    // the next separator attempt restores it, even after a defaulted value.
    if let Some(saved) = reenter.take() {
        *position = Some(saved);
    }
    let Some(mut cursor) = *position else {
        return false;
    };
    skip_std_whitespace(raw, &mut cursor);
    if raw.as_bytes().get(cursor) != Some(&separator) {
        *reenter = Some(cursor);
        *position = None;
        return false;
    }
    *position = Some(cursor + 1);
    true
}

pub(in crate::scenario) fn parse_legacy_name_list(
    _field: &str,
    raw: &str,
) -> Result<LegacyNameList, ScenarioError> {
    const C4_MAX_NAME_LIST: usize = 10;
    const C4_MAX_NAME: usize = 30;

    let mut entries = Vec::new();
    let mut position = Some(0);
    let mut reenter = None;
    for index in 0..C4_MAX_NAME_LIST {
        if index != 0 {
            consume_name_list_separator(raw, &mut position, &mut reenter, b';');
        }

        let name = if let Some(cursor) = position.as_mut() {
            skip_std_whitespace(raw, cursor);
            let name_start = *cursor;
            while *cursor < raw.len()
                && *cursor - name_start < C4_MAX_NAME
                && is_std_identifier_byte(raw.as_bytes()[*cursor])
            {
                *cursor += 1;
            }
            raw[name_start..*cursor].to_string()
        } else {
            String::new()
        };
        let has_count = consume_name_list_separator(raw, &mut position, &mut reenter, b'=');
        let count = if has_count {
            position
                .as_mut()
                .and_then(|cursor| parse_std_i32_prefix_at(raw, cursor))
                .unwrap_or(0)
        } else {
            0
        };
        if !name.is_empty() {
            entries.push(LegacyNameEntry {
                name,
                count: has_count.then_some(count),
            });
        }
    }
    Ok(entries)
}

pub(in crate::scenario) fn parse_legacy_version(
    _field: &str,
    raw: &str,
) -> Result<[i32; 5], ScenarioError> {
    let mut version = [0; 5];
    compile_defaulted_i32_components(raw, &mut version, &[0; 5], true);
    Ok(version)
}

fn parse_base_functionality_number(raw: &str, position: &mut usize) -> Option<i32> {
    skip_std_whitespace(raw, position);
    let bytes = raw.as_bytes();
    let number_start = *position;
    let mut cursor = number_start;
    // StdCompilerINIRead selects base 16 only for an unsigned token that
    // starts with 0x. A sign therefore makes `-0x10` decimal -0 plus junk.
    let radix =
        if bytes.get(cursor) == Some(&b'0') && matches!(bytes.get(cursor + 1), Some(b'x' | b'X')) {
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
        if radix == 16 {
            // strtol("0xG", ..., 16) still consumes the leading zero.
            *position = number_start + 1;
            return Some(0);
        }
        return None;
    }

    // strtol saturates to native C long; assigning that result to int32_t
    // then supplies the platform's ordinary modulo narrowing.
    let long_bits = std::mem::size_of::<std::os::raw::c_long>() * 8;
    let long_max = (1u128 << (long_bits - 1)) - 1;
    let long_min_magnitude = 1u128 << (long_bits - 1);
    let signed = if negative {
        if magnitude >= long_min_magnitude {
            -(long_min_magnitude as i128)
        } else {
            -(magnitude as i128)
        }
    } else {
        magnitude.min(long_max) as i128
    };
    *position = cursor;
    Some((signed as i64) as i32)
}

pub(in crate::scenario) fn parse_base_functionality(
    field: &str,
    raw: &str,
) -> Result<i32, ScenarioError> {
    if raw.trim().is_empty() {
        return Ok(BASEFUNC_DEFAULT);
    }

    let mut value = 0;
    let mut position = 0;
    loop {
        if let Some(flag) = parse_base_functionality_number(raw, &mut position) {
            value |= flag;
        } else {
            let start = position;
            while raw
                .as_bytes()
                .get(position)
                .is_some_and(|byte| is_std_identifier_byte(*byte))
            {
                position += 1;
            }
            if position == start {
                return Err(ScenarioError::LegacyParse(format!(
                    "missing BaseFunctionality token in `{field}`"
                )));
            }
            let entry = &raw[start..position];
            let flag = match entry {
                "BASEFUNC_Default" => BASEFUNC_DEFAULT,
                "BASEFUNC_AutoSellContents" => BASEFUNC_AUTO_SELL_CONTENTS,
                "BASEFUNC_RegenerateEnergy" => BASEFUNC_REGENERATE_ENERGY,
                "BASEFUNC_Buy" => BASEFUNC_BUY,
                "BASEFUNC_Sell" => BASEFUNC_SELL,
                "BASEFUNC_RejectEntrance" => BASEFUNC_REJECT_ENTRANCE,
                "BASEFUNC_Extinguish" => BASEFUNC_EXTINGUISH,
                _ => {
                    tracing::warn!(field, token = entry, "unknown BaseFunctionality bit name");
                    0
                }
            };
            value |= flag;
        }

        if !consume_std_separator(raw, &mut position, b'|') {
            break;
        }
    }
    Ok(value)
}

pub(in crate::scenario) fn parse_i32_array<const N: usize>(
    _field: &str,
    raw: &str,
) -> Result<[i32; N], ScenarioError> {
    let mut result = [0; N];
    compile_defaulted_i32_components(raw, &mut result, &[0; N], true);
    Ok(result)
}

pub(in crate::scenario) fn parse_position(
    _field: &str,
    raw: &str,
) -> Result<[i32; 2], ScenarioError> {
    let mut result = [-1, -1];
    compile_defaulted_i32_components(raw, &mut result, &[-1, -1], true);
    Ok(result)
}
