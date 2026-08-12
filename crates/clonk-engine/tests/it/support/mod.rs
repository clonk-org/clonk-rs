use clonk_engine::{Definition, Engine, ObjectId, ObjectSnapshot, PlayerConfig, SpawnConfig};
use clonk_script::Value;

pub trait TestValueExt<T> {
    fn test_value(self) -> T;
}

impl<T> TestValueExt<T> for Option<T> {
    #[track_caller]
    fn test_value(self) -> T {
        Option::expect(self, "integration-test value exists")
    }
}

impl<T, E: std::fmt::Debug> TestValueExt<T> for Result<T, E> {
    #[track_caller]
    fn test_value(self) -> T {
        Result::expect(self, "integration-test operation succeeds")
    }
}

pub trait EngineTestExt {
    fn register_test_definition(&mut self, definition: Definition);
    fn register_test_script_definition(&mut self, id: &str, name: &str, script: &str);
    fn register_test_player(&mut self, player: PlayerConfig);
    fn spawn_test_object(&mut self, config: SpawnConfig) -> ObjectId;
    fn test_object_snapshot(&self, object: ObjectId) -> ObjectSnapshot;
    fn test_object_index(&self, object: ObjectId) -> usize;
    fn call_test_object_function(
        &mut self,
        index: usize,
        function: &str,
        args: Vec<Value>,
    ) -> Value;
    fn call_test_scenario_script_function(&mut self, function: &str, args: Vec<Value>);
}

impl EngineTestExt for Engine {
    #[track_caller]
    fn register_test_definition(&mut self, definition: Definition) {
        crate::support::TestValueExt::test_value(self.register_definition(definition));
    }

    #[track_caller]
    fn register_test_script_definition(&mut self, id: &str, name: &str, script: &str) {
        crate::support::TestValueExt::test_value(self.register_script_definition(id, name, script));
    }

    #[track_caller]
    fn register_test_player(&mut self, player: PlayerConfig) {
        crate::support::TestValueExt::test_value(self.register_player(player));
    }

    #[track_caller]
    fn spawn_test_object(&mut self, config: SpawnConfig) -> ObjectId {
        crate::support::TestValueExt::test_value(self.spawn_object(config))
    }

    #[track_caller]
    fn test_object_snapshot(&self, object: ObjectId) -> ObjectSnapshot {
        crate::support::TestValueExt::test_value(self.object_snapshot(object))
    }

    #[track_caller]
    fn test_object_index(&self, object: ObjectId) -> usize {
        crate::support::TestValueExt::test_value(self.find_object_index(object))
    }

    #[track_caller]
    fn call_test_object_function(
        &mut self,
        index: usize,
        function: &str,
        args: Vec<Value>,
    ) -> Value {
        crate::support::TestValueExt::test_value(self.call_object_function(index, function, args))
    }

    #[track_caller]
    fn call_test_scenario_script_function(&mut self, function: &str, args: Vec<Value>) {
        crate::support::TestValueExt::test_value(
            self.call_scenario_script_function(function, args),
        );
    }
}

pub mod dev_feedback;
#[allow(dead_code)]
pub mod real_scenario;
pub mod virtual_player;

pub type PreparedScenarioSubcase = (&'static str, fn(&real_scenario::PreparedInstalledScenario));
pub type ScenarioSubcase = (&'static str, fn(&clonk_engine::Scenario));
