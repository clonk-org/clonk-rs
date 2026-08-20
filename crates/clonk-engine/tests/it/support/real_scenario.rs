use std::env;
use std::path::{Path, PathBuf};

use clonk_engine::scenario::{load_system_scripts, LegacyDefinitionResolver};
use clonk_engine::{Engine, JoinPlayerConfig, JoinedPlayer, ObjectId, Scenario, ScenarioError};
use clonk_resources::{Group, MaterialLibrary};

struct ContentResolver {
    roots: Vec<PathBuf>,
}

struct RawContentResolver {
    root: PathBuf,
}

impl LegacyDefinitionResolver for RawContentResolver {
    fn resolve_definition_groups(
        &self,
        _scenario: &Group,
        identifier: &str,
    ) -> Result<Vec<Group>, ScenarioError> {
        Group::open(self.root.join(identifier.replace('\\', "/")))
            .map(|group| vec![group])
            .map_err(ScenarioError::Resources)
    }
}

impl LegacyDefinitionResolver for ContentResolver {
    fn resolve_definition_groups(
        &self,
        _scenario: &Group,
        identifier: &str,
    ) -> Result<Vec<Group>, ScenarioError> {
        let relative = identifier.replace('\\', "/");
        self.roots
            .iter()
            .map(|root| root.join(&relative))
            .find(|candidate| candidate.exists())
            .map(Group::open)
            .transpose()
            .map_err(ScenarioError::Resources)?
            .map(|group| vec![group])
            .ok_or_else(|| ScenarioError::LegacyDefinitionNotFound { path: relative })
    }

    /// C4GameParameters::Load publishes every registered parent folder's
    /// Material.c4g as NRT_Material ahead of the installed one, and
    /// C4Game::InitMaterialTexture walks exactly that chain. Scenario folders
    /// such as `ClonkMars.c4f` ship the textures their Landscape.txt names, so
    /// a chain that skips them renders an empty map
    /// (C4GameParameters.cpp:214-222; C4Game.cpp:901-977).
    fn resolve_material_groups(&self, scenario: &Group) -> Result<Vec<Group>, ScenarioError> {
        let mut groups = scenario
            .root()
            .ancestors()
            .skip(1)
            .take_while(|folder| {
                folder
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("c4f"))
            })
            .map(|folder| folder.join("Material.c4g"))
            .filter(|candidate| candidate.exists())
            .map(Group::open)
            .collect::<Result<Vec<_>, _>>()
            .map_err(ScenarioError::Resources)?;
        groups.extend(
            self.resolve_definition_groups(scenario, "Material.c4g")
                .or_else(|error| match error {
                    ScenarioError::LegacyDefinitionNotFound { .. } => Ok(Vec::new()),
                    error => Err(error),
                })?,
        );
        Ok(groups)
    }
}

pub fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub fn content_root() -> PathBuf {
    env::var_os("LC_CONTENT_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| repository_root().join("content"))
}

/// Load only the scenario and definition groups rooted in `content/`.
/// This preserves tests that intentionally bypass installed materials and
/// system scripts; use `load_installed_scenario` for app-like activation.
pub fn load_raw_content_scenario(
    relative_path: impl AsRef<Path>,
) -> Result<Scenario, ScenarioError> {
    let content = content_root();
    Scenario::load_from_path_with(
        content.join(relative_path),
        &RawContentResolver { root: content },
    )
}

/// Boot a repository scenario through the same prerequisites as the app:
/// the installed material library and planet System.c4g precede scenario
/// definition/script loading (C4Game.cpp:882-960,2591-2607,2764-2788).
///
/// Keeping this in test support makes virtual playthroughs exercise real
/// content without gaining a state-mutation shortcut.
/// What the load half of an activation spent, before `apply` begins.
///
/// `ActivationTimings` covers `Scenario::apply` and nothing before it, so for
/// a small scenario — where load is the larger half — the recorded stages
/// describe a minority of the interval (clonk-org/clonk-rs#293). These are the
/// four calls this harness makes to reach an applied engine, timed where they
/// are already made rather than by instrumenting the engine.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScenarioLoadTimings {
    /// `Scenario::load_from_path_with_seed`: group I/O, the scenario core, and
    /// every definition it resolves.
    pub scenario: std::time::Duration,
    /// `MaterialLibrary::from_group` over the installed `Material.c4g`.
    pub materials: std::time::Duration,
    /// `planet/System.c4g`: reading and parsing the global script hosts.
    pub system_scripts: std::time::Duration,
}

impl ScenarioLoadTimings {
    pub fn total(&self) -> std::time::Duration {
        self.scenario + self.materials + self.system_scripts
    }
}

pub struct PreparedInstalledScenario {
    seed: u64,
    scenario_path: PathBuf,
    scenario: Scenario,
    materials: MaterialLibrary,
    system_scripts: Vec<(String, String)>,
    standard_names: Option<String>,
    load_timings: ScenarioLoadTimings,
}

impl PreparedInstalledScenario {
    pub fn load_timings(&self) -> ScenarioLoadTimings {
        self.load_timings
    }

    pub fn generated_landscape_requires_seed_retry(&self) -> bool {
        self.scenario.generated_landscape_requires_seed_retry()
    }

    /// Apply the immutable parsed inputs to a fresh simulation instance.
    pub fn instantiate(&self) -> Engine {
        self.instantiate_with_system_scripts(self.system_scripts.clone())
    }

    /// Instantiate with one `planet/System.c4g` script left out, so an
    /// `#appendto` can be A/B'd against the shipped content it appends to.
    pub fn instantiate_without_system_script(&self, name: &str) -> Engine {
        let remaining: Vec<(String, String)> = self
            .system_scripts
            .iter()
            .filter(|(script_name, _)| script_name != name)
            .cloned()
            .collect();
        assert_eq!(
            remaining.len() + 1,
            self.system_scripts.len(),
            "System.c4g script `{name}` is installed exactly once"
        );
        self.instantiate_with_system_scripts(remaining)
    }

    fn instantiate_with_system_scripts(&self, scripts: Vec<(String, String)>) -> Engine {
        let mut engine = Engine::with_seed(self.seed);
        engine.configure_materials_from_library(&self.materials);
        engine.install_global_scripts(&scripts);
        engine.set_standard_names(self.standard_names.clone());
        self.scenario.apply(&mut engine).unwrap_or_else(|error| {
            panic!(
                "scenario `{}` applies: {error}",
                self.scenario_path.display()
            )
        });
        engine
    }
}

pub fn prepare_installed_scenario(
    relative_path: impl AsRef<Path>,
    seed: u64,
) -> PreparedInstalledScenario {
    let content = content_root();
    let content_install = content.parent().unwrap_or_else(|| {
        panic!(
            "content root `{}` has no repository parent",
            content.display()
        )
    });
    let bundled = repository_root();
    let relative_path = relative_path.as_ref();
    let scenario_path = [bundled.join(relative_path), content.join(relative_path)]
        .into_iter()
        .find(|candidate| candidate.exists())
        .unwrap_or_else(|| bundled.join(relative_path));
    let scenario_started = std::time::Instant::now();
    let scenario = Scenario::load_from_path_with_seed(
        &scenario_path,
        &ContentResolver {
            roots: vec![bundled, content.clone()],
        },
        seed,
    )
    .unwrap_or_else(|error| panic!("scenario `{}` loads: {error}", scenario_path.display()));
    let scenario_elapsed = scenario_started.elapsed();

    let materials_started = std::time::Instant::now();
    let material_group = Group::open(content.join("Material.c4g"))
        .unwrap_or_else(|error| panic!("installed Material.c4g opens: {error}"));
    let materials = MaterialLibrary::from_group(&material_group)
        .unwrap_or_else(|error| panic!("installed Material.c4g loads: {error}"));
    let materials_elapsed = materials_started.elapsed();

    let system_started = std::time::Instant::now();
    let system_group = Group::open(content_install.join("planet/System.c4g"))
        .unwrap_or_else(|error| panic!("planet System.c4g opens: {error}"));
    let system_scripts = load_system_scripts(&system_group)
        .unwrap_or_else(|error| panic!("planet System.c4g scripts load: {error}"));
    let system_elapsed = system_started.elapsed();
    let standard_names = system_group
        .read_file("Names.txt")
        .ok()
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned());

    PreparedInstalledScenario {
        seed,
        scenario_path,
        scenario,
        materials,
        system_scripts,
        standard_names,
        load_timings: ScenarioLoadTimings {
            scenario: scenario_elapsed,
            materials: materials_elapsed,
            system_scripts: system_elapsed,
        },
    }
}

pub fn load_installed_scenario(relative_path: impl AsRef<Path>, seed: u64) -> Engine {
    prepare_installed_scenario(relative_path, seed).instantiate()
}

pub fn load_tutorial(number: u8, seed: u64) -> Engine {
    load_installed_scenario(format!("Tutorial.c4f/Tutorial{number:02}.c4s"), seed)
}

pub fn load_tutorial_with_local_player(
    number: u8,
    seed: u64,
    name: impl Into<String>,
    control_style: bool,
    auto_context_menu: bool,
) -> (Engine, i32) {
    let mut engine = load_tutorial(number, seed);
    let player =
        join_local_player_with_preferences(&mut engine, name, control_style, auto_context_menu);
    (engine, player)
}

pub fn join_local_player_with_preferences(
    engine: &mut Engine,
    name: impl Into<String>,
    control_style: bool,
    auto_context_menu: bool,
) -> i32 {
    engine
        .join_player(local_player_config(name, control_style, auto_context_menu))
        .unwrap_or_else(|error| panic!("local virtual player joins: {error}"))
        .number()
}

pub fn join_initialized_local_player_details_with_preferences(
    engine: &mut Engine,
    name: impl Into<String>,
    control_style: bool,
    auto_context_menu: bool,
) -> JoinedPlayer {
    join_initialized_local_player_details(
        engine,
        local_player_config(name, control_style, auto_context_menu),
    )
}

pub fn join_local_player(engine: &mut Engine, name: impl Into<String>) -> i32 {
    join_initialized_local_player(engine, name, None)
}

/// Join a local virtual player whose team was already selected by the lobby.
/// Custom active team lists otherwise postpone C++ ScenarioInit until the
/// synchronized team-selection control executes (C4Player.cpp:299-320,
/// 111-157,344-349).
pub fn join_local_player_on_team(engine: &mut Engine, name: impl Into<String>, team: i32) -> i32 {
    join_initialized_local_player(engine, name, Some(team))
}

fn join_initialized_local_player(
    engine: &mut Engine,
    name: impl Into<String>,
    team: Option<i32>,
) -> i32 {
    let mut config = local_player_config(name, false, false);
    config.team = team;
    join_initialized_local_player_config(engine, config)
}

fn join_initialized_local_player_config(engine: &mut Engine, config: JoinPlayerConfig) -> i32 {
    join_initialized_local_player_details(engine, config).number
}

fn join_initialized_local_player_details(
    engine: &mut Engine,
    config: JoinPlayerConfig,
) -> JoinedPlayer {
    engine
        .join_player(config)
        .unwrap_or_else(|error| panic!("local virtual player joins: {error}"))
        .initialized()
        .unwrap_or_else(|| {
            panic!("local virtual player requires an explicit runtime team selection")
        })
}

fn local_player_config(
    name: impl Into<String>,
    control_style: bool,
    auto_context_menu: bool,
) -> JoinPlayerConfig {
    JoinPlayerConfig {
        name: name.into(),
        player_info_id: 0,
        score: 0,
        rounds: 0,
        rounds_won: 0,
        rounds_lost: 0,
        total_playing_time: 0,
        team: None,
        color_dw: 0xff_00_00,
        pref_color: 0,
        pref_position: 0,
        crew: Vec::new(),
        control_style,
        auto_context_menu,
        startup_player_count: 1,
    }
}

pub fn object_with_definition(engine: &Engine, definition: &str) -> Option<ObjectId> {
    engine.first_object_for_definition(definition)
}

pub fn object_with_definition_near_x(
    engine: &Engine,
    definition: &str,
    expected_x: i32,
) -> Option<ObjectId> {
    engine
        .snapshot()
        .objects
        .into_iter()
        .filter(|object| object.definition_id == definition)
        .min_by_key(|object| (object.position.x - expected_x).abs())
        .map(|object| object.id)
}

pub fn object_contents_count(engine: &Engine, object: ObjectId, definition: &str) -> usize {
    engine.object_snapshot(object).map_or(0, |object| {
        object
            .contents
            .iter()
            .filter(|item| {
                engine
                    .object_snapshot(**item)
                    .is_some_and(|item| item.definition_id == definition)
            })
            .count()
    })
}

pub fn clonk_contents_count(engine: &Engine, clonk: ObjectId, definition: &str) -> usize {
    object_contents_count(engine, clonk, definition)
}

pub fn clonk_carries(engine: &Engine, clonk: ObjectId, definition: &str) -> bool {
    engine.object_snapshot(clonk).is_some_and(|clonk| {
        clonk.contents.iter().any(|item| {
            engine
                .object_snapshot(*item)
                .is_some_and(|item| item.definition_id == definition)
        })
    })
}

pub fn tutorial_message_contains(engine: &Engine, needle: &str) -> bool {
    engine.message_line_contains(needle)
}
