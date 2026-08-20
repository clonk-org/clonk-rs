// These tests were written under clonk-engine's crate-level lint config (lib.rs
// top); mirror it here so the byte-identical code lints the same as it did
// inline, rather than surfacing style lints the move would otherwise expose.
#![allow(dead_code, unreachable_patterns, unused_variables)]
#![allow(
    clippy::doc_lazy_continuation,
    clippy::field_reassign_with_default,
    clippy::if_same_then_else,
    clippy::large_enum_variant,
    clippy::manual_clamp,
    clippy::match_like_matches_macro,
    clippy::needless_range_loop,
    clippy::question_mark,
    clippy::should_implement_trait,
    clippy::too_many_arguments,
    clippy::type_complexity,
    clippy::vec_init_then_push
)]

// clonk-engine's former inline unit tests (lib.rs `mod tests`). Kept in their own
// test-only package (not folded into tests/it) so this large harness can compile
// at a lower optimization level while clonk-engine remains fully optimized. A
// test-only edit here also recompiles just this leaf crate — measured ~10s ->
// ~2s versus editing the inline module, which dirtied lib.rs and cascaded
// through every downstream crate. The glob re-export lets the module body keep
// its original `use super::*;`; the submodule `pub use`s below cover items the
// glob (root-only) doesn't reach.
#[allow(unused_imports)]
pub use clonk_engine::action::DEFAULT_ACTION_NAME;
#[allow(unused_imports)]
pub use clonk_engine::command::{
    AcquireScriptResult, CommandData, CommandId, CommandMode, CommandRequest,
};
#[allow(unused_imports)]
pub use clonk_engine::compat::{object_reference_value, NestedObjectOutcome};
#[allow(unused_imports)]
pub use clonk_engine::compat::{LandscapeOperation, ObjectOrderCommand, PlayerCommand};
#[allow(unused_imports)]
pub use clonk_engine::effect::EffectCommand;
#[allow(unused_imports)]
pub use clonk_engine::math::{fixed100, fixtoi, itofix, FixedVec2};
#[allow(unused_imports)]
pub use clonk_engine::sector::{SectorKey, SECTOR_HEIGHT};
#[allow(unused_imports)]
pub use clonk_engine::transfer::{TransferZoneCommand, TransferZoneRect};
#[allow(unused_imports)]
pub use clonk_engine::*;
#[allow(unused_imports)]
pub use clonk_resources::{ResourceDefinition as ResourceDefinitionData, C4_MAX_PHYSICAL};
#[allow(unused_imports)]
pub use clonk_script::DebuggerHooks;

mod tests {
    use super::*;

    const NOOP_DEFINITION_SCRIPT: &str = r#"
        global func Initialize(state, random) {
            return 0;
        }

        global func Step(state, frame, random) {
            return 0;
        }
        "#;

    #[track_caller]
    fn test_definition(id: impl Into<String>, name: impl Into<String>, source: &str) -> Definition {
        Definition::from_script(id, name, source).expect("test definition compiles")
    }

    fn join_player_config(name: impl Into<String>) -> JoinPlayerConfig {
        JoinPlayerConfig {
            name: name.into(),
            player_info_id: 1,
            score: 0,
            rounds: 0,
            rounds_won: 0,
            rounds_lost: 0,
            total_playing_time: 0,
            team: None,
            color_dw: 0xff0000,
            pref_color: 0,
            pref_position: 0,
            crew: Vec::new(),
            control_style: false,
            auto_context_menu: false,
            startup_player_count: 1,
        }
    }

    macro_rules! spawn_fixture {
        ($engine:expr, $definition:expr $(, $setter:ident: $value:expr)* $(,)?) => {
            ($engine).spawn_test_object(
                SpawnConfig::new($definition)$(.$setter($value))*
            )
        };
    }

    macro_rules! unit_assert_eq {
        ($actual:expr => $expected:expr, $($message:tt)+) => {
            assert_eq!($actual, $expected, $($message)+)
        };
        ($actual:expr => $expected:expr $(,)?) => {
            assert_eq!($actual, $expected)
        };
    }

    macro_rules! unit_assert {
        ($condition:expr, $($message:tt)+) => {
            assert!($condition, $($message)+)
        };
        ($condition:expr $(,)?) => {
            assert!($condition)
        };
    }

    macro_rules! unit_assert_ne {
        ($actual:expr => $unexpected:expr, $($message:tt)+) => {
            assert_ne!($actual, $unexpected, $($message)+)
        };
        ($actual:expr => $unexpected:expr $(,)?) => {
            assert_ne!($actual, $unexpected)
        };
    }

    trait TestValueExt<T> {
        fn test_value(self) -> T;
    }

    impl<T> TestValueExt<T> for Option<T> {
        #[track_caller]
        fn test_value(self) -> T {
            self.expect("unit-test value exists")
        }
    }

    impl<T, E: std::fmt::Debug> TestValueExt<T> for Result<T, E> {
        #[track_caller]
        fn test_value(self) -> T {
            self.expect("unit-test operation succeeds")
        }
    }

    trait TestEngineExt {
        fn register_test_definition(&mut self, definition: Definition);
        fn register_test_player(&mut self, config: PlayerConfig);
        fn register_test_script_definition(
            &mut self,
            id: impl Into<String>,
            name: impl Into<String>,
            source: &str,
        );
        fn spawn_test_object(&mut self, config: SpawnConfig) -> ObjectId;
        fn test_object_index(&self, id: ObjectId) -> usize;
        fn test_object_snapshot(&self, id: ObjectId) -> ObjectSnapshot;
        fn test_tick(&mut self) -> SimulationSnapshot;
        fn call_test_object_function(
            &mut self,
            index: usize,
            function: &str,
            args: Vec<Value>,
        ) -> Value;
    }

    impl TestEngineExt for Engine {
        fn register_test_definition(&mut self, definition: Definition) {
            self.register_definition(definition)
                .expect("test definition registers");
        }

        fn register_test_player(&mut self, config: PlayerConfig) {
            self.register_player(config).expect("test player registers");
        }

        fn register_test_script_definition(
            &mut self,
            id: impl Into<String>,
            name: impl Into<String>,
            source: &str,
        ) {
            self.register_script_definition(id, name, source)
                .expect("test script definition registers");
        }

        fn spawn_test_object(&mut self, config: SpawnConfig) -> ObjectId {
            self.spawn_object(config).expect("test object spawns")
        }

        #[track_caller]
        fn test_object_index(&self, id: ObjectId) -> usize {
            self.find_object_index(id).expect("test object exists")
        }

        #[track_caller]
        fn test_object_snapshot(&self, id: ObjectId) -> ObjectSnapshot {
            self.object_snapshot(id)
                .expect("test object has a snapshot")
        }

        #[track_caller]
        fn test_tick(&mut self) -> SimulationSnapshot {
            self.tick().expect("test tick succeeds")
        }

        #[track_caller]
        fn call_test_object_function(
            &mut self,
            index: usize,
            function: &str,
            args: Vec<Value>,
        ) -> Value {
            self.call_object_function(index, function, args)
                .expect("test object function succeeds")
        }
    }

    // Area part files spliced into this same `tests` module: each part is a
    // bare item sequence (not a child module), so test ids stay `tests::<fn>`.
    include!("parts/fire_blast.rs");
    include!("parts/materials_pxs.rs");
    include!("parts/action_procedures.rs");
    include!("parts/find_funcs.rs");
    include!("parts/menus_commands.rs");
    include!("parts/physicals_seasons.rs");
    include!("parts/players_crew.rs");
    include!("parts/changedef_layers.rs");
    include!("parts/crewinfo_contents.rs");
    include!("parts/effects.rs");
    include!("parts/solidmask_shape.rs");
    include!("parts/ocf_rotation.rs");
    include!("parts/order_exec.rs");
    include!("parts/log_levels.rs");
}
