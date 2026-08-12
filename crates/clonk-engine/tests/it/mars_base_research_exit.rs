use crate::support::real_scenario::load_installed_scenario;
use clonk_core::log_target::SCRIPT_DEBUG_LOG_TARGET;
use clonk_engine::{
    command::{CommandId, CommandMode, CommandRequest},
    Engine, ObjectId, PlayerConfig, SpawnConfig, Vector2,
};
use clonk_script::Value;
use std::fmt;
use std::sync::{Arc, Mutex};
use tracing::field::{Field, Visit};
use tracing::{subscriber, Level};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
use tracing_subscriber::registry::Registry;

const PLAYER: i32 = 1;

#[derive(Clone)]
struct WarningLayer {
    messages: Arc<Mutex<Vec<String>>>,
}

impl<S> Layer<S> for WarningLayer
where
    S: tracing::Subscriber,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        if *event.metadata().level() != Level::WARN
            || event.metadata().target() != SCRIPT_DEBUG_LOG_TARGET
        {
            return;
        }
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);
        if let Some(message) = visitor.message {
            crate::support::TestValueExt::test_value(self.messages.lock()).push(message);
        }
    }
}

#[derive(Default)]
struct MessageVisitor {
    message: Option<String>,
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.message = Some(format!("{value:?}").trim_matches('"').to_owned());
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_owned());
        }
    }
}

fn capture_script_warnings<T>(run: impl FnOnce() -> T) -> (T, Vec<String>) {
    let messages = Arc::new(Mutex::new(Vec::new()));
    let subscriber = Registry::default().with(WarningLayer {
        messages: Arc::clone(&messages),
    });
    let result = subscriber::with_default(subscriber, run);
    let captured = crate::support::TestValueExt::test_value(messages.lock()).clone();
    (result, captured)
}

fn call(engine: &mut Engine, object: ObjectId, function: &str, args: Vec<Value>) -> Value {
    let index = crate::support::TestValueExt::test_value(engine.find_object_index(object));
    engine
        .call_object_function(index, function, args)
        .unwrap_or_else(|error| panic!("{function} executes: {error}"))
}

#[test]
fn cancelling_mars_base_research_unlocks_the_door() {
    // ClonkMars Base.c4d/Script.c:240-245 treats its Research effect's
    // interval as the lock state. C++ FnChangeEffect stores a requested zero
    // interval (src/C4Script.cpp:5516-5543), and C4Effect::Execute then skips
    // timer callbacks for it (src/C4Effect.cpp:339-357).
    let mut engine = load_installed_scenario("ClonkMars.c4f/01_Fossae.c4s", 0);
    crate::support::TestValueExt::test_value(
        engine.register_player(PlayerConfig::new(PLAYER, "Mars research tester")),
    );

    let base = crate::support::TestValueExt::test_value(
        engine.spawn_object(
            SpawnConfig::new("BASE")
                .with_owner(PLAYER)
                .with_position(Vector2::new(300, 200)),
        ),
    );
    let researcher = crate::support::TestValueExt::test_value(
        engine.spawn_object(
            SpawnConfig::new("SCNK")
                .with_loaded(true)
                .with_owner(PLAYER)
                .with_controller(PLAYER)
                .with_alive(true)
                .with_crew_member(true)
                .with_container(base),
        ),
    );
    let bystander = crate::support::TestValueExt::test_value(
        engine.spawn_object(
            SpawnConfig::new("SCNK")
                .with_loaded(true)
                .with_owner(PLAYER)
                .with_controller(PLAYER)
                .with_alive(true)
                .with_crew_member(true)
                .with_container(base),
        ),
    );

    assert_eq!(
        call(
            &mut engine,
            base,
            "StartResearch",
            vec![
                Value::C4Id("BASE".to_string()),
                Value::Object(researcher.as_u64()),
                Value::Int(0),
            ],
        ),
        Value::Nil,
        "starting the real Base research callback succeeds"
    );
    assert!(
        call(&mut engine, base, "IsResearching", Vec::new(),).as_bool(),
        "the running Research effect locks the Base door"
    );

    assert!(
        engine
            .execute_context_menu(base, "ContextCancelResearch")
            .expect("selecting Stop research executes"),
        "the shipped Stop research context action handles the selection"
    );
    assert!(
        !call(&mut engine, base, "IsResearching", Vec::new()).as_bool(),
        "a paused Research effect no longer locks the Base door"
    );
    assert!(
        call(
            &mut engine,
            base,
            "CanOpen",
            vec![Value::Object(researcher.as_u64())],
        )
        .as_bool(),
        "the contained researching clonk can trigger the now-unlocked door"
    );

    for clonk in [researcher, bystander] {
        let index = crate::support::TestValueExt::test_value(engine.find_object_index(clonk));
        crate::support::TestValueExt::test_value(
            engine.objects[index]
                .commands
                .push_back(CommandRequest::new(CommandId::Exit).with_mode(CommandMode::Base)),
        );
    }
    let (_, warnings) = capture_script_warnings(|| {
        for _ in 0..30 {
            crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
            if [researcher, bystander].into_iter().all(|clonk| {
                engine
                    .object_snapshot(clonk)
                    .is_some_and(|snapshot| snapshot.container.is_none())
            }) {
                break;
            }
        }
    });
    // C++ reports parameter-conversion diagnostics through DebugLog, not the
    // ordinary script Log stream (src/C4AulExec.cpp:1345-1362).
    assert!(
        warnings.iter().any(|warning| {
            warning.contains(
                r#"call to "FxResearchEffect" parameter 5: got "string", but expected "id"!"#,
            )
        }),
        "Exit must reach SetOverlayAction -> AddEffect(IntOverlayAction) -> the real Research checker through DebugLog; got warnings {warnings:?}"
    );
    assert!(
        warnings.iter().any(|warning| {
            warning.contains(
                r#"call to "FxResearchEffect" parameter 6: got "int", but expected "object"!"#,
            )
        }),
        "the second legacy conversion warning must use the same DebugLog route; got warnings {warnings:?}"
    );
    for clonk in [researcher, bystander] {
        assert_eq!(
            engine
                .object_snapshot(clonk)
                .expect("Spaceclonk survives Exit")
                .container,
            None,
            "both the researcher and other contained clonks leave after research is paused"
        );
    }
}
