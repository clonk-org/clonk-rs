use std::env;
use std::path::{Path, PathBuf};

use lc_engine::scenario::{load_system_scripts, LegacyDefinitionResolver};
use lc_engine::{Engine, JoinPlayerConfig, Scenario, ScenarioError};
use lc_resources::{Group, MaterialLibrary};

struct ContentResolver {
    root: PathBuf,
}

impl LegacyDefinitionResolver for ContentResolver {
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

pub fn content_root() -> PathBuf {
    env::var_os("LC_CONTENT_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../content"))
}

/// Boot a repository scenario through the same prerequisites as the app:
/// the installed material library and planet System.c4g precede scenario
/// definition/script loading (C4Game.cpp:882-960,2591-2607,2764-2788).
///
/// Keeping this in test support makes virtual playthroughs exercise real
/// content without gaining a state-mutation shortcut.
pub fn load_installed_scenario(relative_path: impl AsRef<Path>, seed: u64) -> Engine {
    let content = content_root();
    let repository = content.parent().unwrap_or_else(|| {
        panic!(
            "content root `{}` has no repository parent",
            content.display()
        )
    });
    let scenario_path = content.join(relative_path);
    let scenario = Scenario::load_from_path_with_seed(
        &scenario_path,
        &ContentResolver {
            root: content.clone(),
        },
        seed,
    )
    .unwrap_or_else(|error| panic!("scenario `{}` loads: {error}", scenario_path.display()));

    let material_group = Group::open(content.join("Material.c4g"))
        .unwrap_or_else(|error| panic!("installed Material.c4g opens: {error}"));
    let materials = MaterialLibrary::from_group(&material_group)
        .unwrap_or_else(|error| panic!("installed Material.c4g loads: {error}"));
    let system_group = Group::open(repository.join("planet/System.c4g"))
        .unwrap_or_else(|error| panic!("planet System.c4g opens: {error}"));
    let system_scripts = load_system_scripts(&system_group)
        .unwrap_or_else(|error| panic!("planet System.c4g scripts load: {error}"));

    let mut engine = Engine::with_seed(seed);
    engine.configure_materials_from_library(&materials);
    engine.install_global_scripts(&system_scripts);
    engine.set_standard_names(
        system_group
            .read_file("Names.txt")
            .ok()
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned()),
    );
    scenario
        .apply(&mut engine)
        .unwrap_or_else(|error| panic!("scenario `{}` applies: {error}", scenario_path.display()));
    engine
}

pub fn load_tutorial(number: u8, seed: u64) -> Engine {
    load_installed_scenario(format!("Tutorial.c4f/Tutorial{number:02}.c4s"), seed)
}

pub fn join_local_player(engine: &mut Engine, name: impl Into<String>) -> i32 {
    engine
        .join_player(JoinPlayerConfig {
            name: name.into(),
            player_info_id: 0,
            score: 0,
            total_playing_time: 0,
            team: None,
            color_dw: 0xff_00_00,
            pref_color: 0,
            pref_position: 0,
            crew: Vec::new(),
            control_style: false,
            auto_context_menu: false,
            startup_player_count: 1,
        })
        .unwrap_or_else(|error| panic!("local virtual player joins: {error}"))
        .number
}
