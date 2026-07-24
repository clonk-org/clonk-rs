pub mod dev_feedback;
#[allow(dead_code)]
pub mod real_scenario;
pub mod virtual_player;

pub type PreparedScenarioSubcase = (&'static str, fn(&real_scenario::PreparedInstalledScenario));
pub type ScenarioSubcase = (&'static str, fn(&clonk_engine::Scenario));
