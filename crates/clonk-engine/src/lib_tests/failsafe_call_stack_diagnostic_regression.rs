use super::*;
use std::fmt;
use std::sync::{Arc, Mutex};
use tracing::field::{Field, Visit};
use tracing::{subscriber, Level};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
use tracing_subscriber::registry::Registry;

#[derive(Clone)]
struct WarningLayer {
    messages: Arc<Mutex<Vec<String>>>,
}

impl<S> Layer<S> for WarningLayer
where
    S: tracing::Subscriber,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        if !matches!(*event.metadata().level(), Level::WARN | Level::INFO) {
            return;
        }
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);
        if let Some(message) = visitor.message {
            self.messages.lock().unwrap().push(message);
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

fn capture_warnings<T>(run: impl FnOnce() -> T) -> (T, Vec<String>) {
    let messages = Arc::new(Mutex::new(Vec::new()));
    let subscriber = Registry::default().with(WarningLayer {
        messages: Arc::clone(&messages),
    });
    let result = subscriber::with_default(subscriber, run);
    let captured = messages.lock().unwrap().clone();
    (result, captured)
}

#[test]
fn failsafe_runtime_error_logs_cpp_call_stack_frames() {
    let script = r#"#strict 3
public func Outer(first, gap, tail) { return Middle(first, gap, tail); }
private func Middle(first, gap, tail) { return Inner(first, gap, tail); }
private func Inner(first, gap, tail) { return MissingFromInner(); }
public func EvalOuter() { return eval("MissingFromEval()"); }
public func Healthy() { return 9; }
public func Lone() { return MissingFromLone(); }
"#;
    let mut engine = Engine::with_seed(47);
    engine
        .register_script_definition("STAK", "Stack fixture", script)
        .expect("stack fixture registers");
    let object = engine
        .spawn_object(SpawnConfig::new("STAK"))
        .expect("stack fixture spawns");
    let index = engine.find_object_index(object).expect("object exists");

    let (recovered, warnings) = capture_warnings(|| {
        tolerate_script_error(engine.call_object_function(
            index,
            "Outer",
            vec![Value::Int(7), Value::Nil, Value::from("tail")],
        ))
    });
    assert_eq!(recovered.expect("script error is tolerated"), None);
    let frames = warnings
        .iter()
        .filter(|message| message.starts_with(" by: "))
        .collect::<Vec<_>>();
    assert_eq!(frames.len(), 3, "one native-style record per active frame");
    assert!(frames[0].contains("Inner(7,nil,\"tail\")"));
    assert!(frames[1].contains("Middle(7,nil,\"tail\")"));
    assert!(frames[2].contains("Outer(7,nil,\"tail\")"));
    assert!(frames
        .iter()
        .all(|frame| frame.contains("(obj Stack fixture #")));
    assert!(frames.iter().all(|frame| frame.contains("(STAK:")));

    let (recovered, warnings) = capture_warnings(|| {
        tolerate_script_error(engine.call_object_function(index, "EvalOuter", Vec::new()))
    });
    assert_eq!(recovered.expect("eval error is tolerated"), None);
    let frames = warnings
        .iter()
        .filter(|message| message.starts_with(" by: "))
        .collect::<Vec<_>>();
    assert_eq!(frames.len(), 2);
    assert!(frames[0].starts_with(" by: eval in STAK (obj Stack fixture #"));
    assert!(!frames[0].contains("()") && !frames[0].contains("(STAK:"));
    assert!(frames[1].contains("EvalOuter()"));
    assert!(frames[1].contains("(obj Stack fixture #"));
    assert!(frames[1].contains("(STAK:"));

    let raw_script = Arc::clone(
        &engine
            .definitions
            .get("STAK")
            .expect("definition exists")
            .script,
    );
    let raw_args = vec![Value::Int(8), Value::Nil, Value::from("raw")];
    let world = engine.host_world_context();
    let rng = engine.rng.clone();
    let frame = engine.frame;
    let global_effects = engine.global_effects.clone();
    let physics = engine.physics;
    let environment = engine.environment;
    let audio = engine.audio_registry.clone();
    let game_over = engine.game_over_triggered;
    let (raw_result, warnings) = capture_warnings(|| {
        ScenarioScript::execute_value_for_script(
            "STAK",
            Some("STAK".to_owned()),
            "Outer",
            &raw_args,
            world,
            rng,
            frame,
            &global_effects,
            physics,
            environment,
            audio,
            game_over,
            || raw_script.call_with_ref_args("Outer", &raw_args),
        )
    });
    assert!(raw_result.0.is_none());
    assert!(raw_result.5.is_some());
    let frames = warnings
        .iter()
        .filter(|message| message.starts_with(" by: "))
        .collect::<Vec<_>>();
    assert_eq!(frames.len(), 3, "raw fail-safe path logs every frame");
    assert!(frames
        .iter()
        .all(|frame| frame.contains("(def Stack fixture)")));
    assert!(frames.iter().all(|frame| !frame.contains("(obj ")));

    let mut scenario_host = ScriptEngine::new();
    scenario_host.set_script_name("Scenario");
    scenario_host.add_script(
        clonk_script::Script::compile_c4_string(
                "#strict 3\nfunc SceneOuter() { return SceneInner(); }\nfunc SceneInner() { return MissingScene(); }",
        )
        .expect("scenario fixture compiles"),
    );
    let (scenario_result, warnings) = capture_warnings(|| {
        ScenarioScript::execute_value_for_script(
            "Scenario",
            None,
            "SceneOuter",
            &[],
            engine.host_world_context(),
            engine.rng.clone(),
            engine.frame,
            &engine.global_effects,
            engine.physics,
            engine.environment,
            engine.audio_registry.clone(),
            engine.game_over_triggered,
            || scenario_host.call_with_ref_args("SceneOuter", &[]),
        )
    });
    assert!(scenario_result.0.is_none());
    let frames = warnings
        .iter()
        .filter(|message| message.starts_with(" by: "))
        .collect::<Vec<_>>();
    assert_eq!(frames.len(), 2);
    assert!(frames.iter().all(|frame| !frame.contains("(def ")));
    assert!(frames.iter().all(|frame| frame.contains("(Scenario:")));

    assert_eq!(
        engine
            .call_object_function(index, "Healthy", Vec::new())
            .expect("execution continues after fail-safe recovery"),
        Value::Int(9)
    );

    let (recovered, warnings) = capture_warnings(|| {
        tolerate_script_error(engine.call_object_function(index, "Lone", Vec::new()))
    });
    assert_eq!(recovered.expect("second script error is tolerated"), None);
    let frames = warnings
        .iter()
        .filter(|message| message.starts_with(" by: "))
        .collect::<Vec<_>>();
    assert_eq!(
        frames.len(),
        1,
        "recovered frames must not leak into later calls"
    );
    assert!(frames[0].contains("Lone()"));
    assert!(!frames[0].contains("Inner") && !frames[0].contains("Outer"));
}
