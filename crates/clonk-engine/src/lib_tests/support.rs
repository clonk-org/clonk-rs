use crate::*;

pub(crate) trait EngineTestExt {
    fn call_test_object_function(
        &mut self,
        index: usize,
        function: &str,
        args: Vec<Value>,
    ) -> Value;
    fn execute_test_object_command(&mut self, object: ObjectId);
    fn load_test_section(&mut self, name: &str, flags: i32, preserve_ids: Vec<ObjectId>) -> bool;
    fn queue_test_command(&mut self, index: usize, command: CommandRequest);
    fn register_test_definition(&mut self, definition: Definition);
    fn register_test_player(&mut self, player: PlayerConfig);
    fn register_test_script_definition(&mut self, id: &str, name: &str, script: &str);
    fn set_test_transfer_zone(&mut self, object: ObjectId, x: i32, y: i32, width: i32, height: i32);
    fn spawn_test_object(&mut self, config: SpawnConfig) -> ObjectId;
    fn test_object_index(&self, object: ObjectId) -> usize;
}

impl EngineTestExt for Engine {
    #[track_caller]
    fn call_test_object_function(
        &mut self,
        index: usize,
        function: &str,
        args: Vec<Value>,
    ) -> Value {
        crate::TestValueExt::test_value(self.call_object_function(index, function, args))
    }

    #[track_caller]
    fn execute_test_object_command(&mut self, object: ObjectId) {
        crate::TestValueExt::test_value(self.execute_object_command_now(object));
    }

    #[track_caller]
    fn load_test_section(&mut self, name: &str, flags: i32, preserve_ids: Vec<ObjectId>) -> bool {
        crate::TestValueExt::test_value(self.load_scenario_section(name, flags, preserve_ids))
    }

    #[track_caller]
    fn queue_test_command(&mut self, index: usize, command: CommandRequest) {
        crate::TestValueExt::test_value(self.objects[index].commands.push_front(command));
    }

    #[track_caller]
    fn register_test_definition(&mut self, definition: Definition) {
        crate::TestValueExt::test_value(self.register_definition(definition));
    }

    #[track_caller]
    fn register_test_player(&mut self, player: PlayerConfig) {
        crate::TestValueExt::test_value(self.register_player(player));
    }

    #[track_caller]
    fn register_test_script_definition(&mut self, id: &str, name: &str, script: &str) {
        crate::TestValueExt::test_value(self.register_script_definition(id, name, script));
    }

    #[track_caller]
    fn set_test_transfer_zone(
        &mut self,
        object: ObjectId,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    ) {
        crate::TestValueExt::test_value(self.set_transfer_zone(
            object,
            TransferZoneRect {
                x,
                y,
                width,
                height,
            },
        ));
    }

    #[track_caller]
    fn spawn_test_object(&mut self, config: SpawnConfig) -> ObjectId {
        crate::TestValueExt::test_value(self.spawn_object(config))
    }

    #[track_caller]
    fn test_object_index(&self, object: ObjectId) -> usize {
        crate::TestValueExt::test_value(self.find_object_index(object))
    }
}

macro_rules! spawn_fixture {
    ($engine:expr, $definition:expr $(, $method:ident: $value:expr)* $(,)?) => {
        crate::TestValueExt::test_value(
            $engine.spawn_object(crate::SpawnConfig::new($definition)$(.$method($value))*)
        )
    };
}

pub(crate) use spawn_fixture;

macro_rules! register_fixture {
    ($engine:expr, $id:expr, $name:expr, $source:expr $(, $method:ident($($argument:expr),* $(,)?))* $(,)?) => {{
        let mut definition = crate::test_definition($id, $name, $source);
        $(definition.$method($($argument),*);)*
        crate::TestValueExt::test_value($engine.register_definition(definition));
    }};
}

pub(crate) use register_fixture;
