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
        if *event.metadata().level() != Level::WARN {
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
            self.message = Some(format!("{value:?}").trim_matches('"').to_string());
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
        }
    }
}

fn capture_warnings(run: impl FnOnce()) -> Vec<String> {
    let messages = Arc::new(Mutex::new(Vec::new()));
    let subscriber = Registry::default().with(WarningLayer {
        messages: Arc::clone(&messages),
    });
    subscriber::with_default(subscriber, run);
    let captured = messages.lock().unwrap().clone();
    captured
}

fn register(engine: &mut Engine, id: &str, source: &str) {
    engine
        .register_script_definition(id, id, source)
        .expect("fixture definition registers");
}

#[test]
fn definition_link_warns_once_and_disables_missing_actmap_and_timer_callbacks() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut hooks = DebuggerHooks::new();
    {
        let calls = Arc::clone(&calls);
        hooks.set_on_call(move |name, _| {
            if matches!(
                name,
                "MissingStart" | "MissingPhase" | "MissingEnd" | "MissingAbort" | "MissingTimer"
            ) {
                calls.lock().unwrap().push(name.to_string());
            }
        });
    }

    let probe = ActionSpec::default()
        .with_delay(1)
        .with_length(100)
        .with_start_call("MissingStart")
        .with_phase_call("MissingPhase")
        .with_end_call("MissingEnd")
        .with_abort_call("MissingAbort");
    let mut definition = Definition::from_script(
        "CBLK",
        "Callback linker",
        "#strict\npublic func ExerciseSetAction() { return SetAction(\"Probe\"); }\n",
    )
    .expect("missing-callback fixture compiles");
    definition.set_c4_callback_convention(true);
    definition.set_debugger_hooks(hooks);
    definition.configure_actions(None, HashMap::from([("Probe".to_string(), probe.clone())]));
    definition.configure_physical_actions(vec![("Probe".to_string(), probe)]);
    definition.set_timer(1);
    definition.set_timer_call(Some("MissingTimer".to_string()));

    let mut engine = Engine::new();
    engine
        .register_definition(definition)
        .expect("callback definition registers");
    engine.resolve_appends();
    let expected = [
        "Error getting Action Probe: StartCall function 'MissingStart'",
        "Error getting Action Probe: PhaseCall function 'MissingPhase'",
        "Error getting Action Probe: EndCall function 'MissingEnd'",
        "Error getting Action Probe: AbortCall function 'MissingAbort'",
        "Error getting TimerCall function 'MissingTimer'",
    ];
    let valid_source = r#"#strict
public func ExerciseSetAction() { return SetAction("Probe"); }
private func MissingStart() { return 1; }
private func MissingPhase() { return 1; }
private func MissingEnd() { return 1; }
private func MissingAbort(int old_phase) { return old_phase; }
private func MissingTimer() { return 1; }
"#;
    assert_eq!(
        capture_warnings(|| engine
            .resolve_includes()
            .expect("initial callback link succeeds")),
        expected,
    );

    let mut action = ActionState::new("Probe");
    action.act_map_index = Some(0);
    let object = engine
        .spawn_object(
            SpawnConfig::new("CBLK")
                .with_action(action)
                .with_loaded(true),
        )
        .expect("callback fixture spawns");
    let index = engine.find_object_index(object).expect("fixture exists");

    // Make every name dynamically resolvable without touching the base
    // script or relinking. C++ must keep using its five cached nulls.
    let injected =
        clonk_script::Script::compile_c4_string(valid_source).expect("injected callbacks compile");
    Arc::make_mut(
        &mut engine
            .definitions
            .get_mut("CBLK")
            .expect("callback definition exists")
            .script,
    )
    .add_script(injected);
    engine.invalidate_host_definition_tables();
    assert!(engine
        .definitions
        .get("CBLK")
        .expect("callback definition exists")
        .has_function("MissingTimer"));
    let runtime_warnings = capture_warnings(|| {
        engine
            .call_object_function(index, "ExerciseSetAction", Vec::new())
            .expect("missing synchronous Start/Abort callbacks are no-ops");
        engine
            .tick_without_snapshot()
            .expect("cached missing PhaseCall and TimerCall remain no-ops");
        engine
            .invoke_action_callback(
                index,
                ActionCallbackKind::End,
                "Probe",
                Some(0),
                None,
                None,
                None,
                None,
            )
            .expect("a cached missing EndCall is a no-op");
    });
    assert!(runtime_warnings.is_empty());
    assert!(calls.lock().unwrap().is_empty());

    assert_eq!(
        capture_warnings(|| engine.relink_scripts().expect("unchanged relink succeeds")),
        expected,
        "each missing physical slot warns once again on the next link",
    );

    assert!(
        capture_warnings(|| {
            assert!(engine
                .reload_definition_script("CBLK", valid_source)
                .expect("valid callback script reload succeeds"));
        })
        .is_empty(),
        "private callbacks satisfy AA_PRIVATE link lookup",
    );

    // Delete all callback names from the live lookup table without a
    // relink. The retained bodies must still run, including compat's
    // synchronous SetAction path.
    let driver_only = clonk_script::Script::compile_c4_string(
        "#strict\npublic func ExerciseSetAction() { return SetAction(\"Probe\"); }\n",
    )
    .expect("driver-only replacement compiles");
    Arc::make_mut(
        &mut engine
            .definitions
            .get_mut("CBLK")
            .expect("callback definition exists")
            .script,
    )
    .replace_script(driver_only, false);
    engine.invalidate_host_definition_tables();
    assert!(!engine
        .definitions
        .get("CBLK")
        .expect("callback definition exists")
        .has_function("MissingStart"));

    engine
        .call_object_function(index, "ExerciseSetAction", Vec::new())
        .expect("linked synchronous Start/Abort callbacks dispatch");
    engine
        .tick_without_snapshot()
        .expect("linked private PhaseCall and TimerCall dispatch");
    engine
        .invoke_action_callback(
            index,
            ActionCallbackKind::End,
            "Probe",
            Some(0),
            None,
            None,
            None,
            None,
        )
        .expect("linked private EndCall dispatches");
    assert_eq!(
        calls.lock().unwrap().as_slice(),
        [
            "MissingStart",
            "MissingAbort",
            "MissingPhase",
            "MissingTimer",
            "MissingEnd",
        ],
    );
}

#[test]
fn missing_include_warns_and_known_siblings_still_merge() {
    let mut engine = Engine::new();
    engine
        .register_definition(
            Definition::from_script(
                "KNWN",
                "Known parent",
                "public func ParentValue() { return 7; }",
            )
            .expect("known parent compiles"),
        )
        .expect("known parent registers");
    engine
        .register_definition(
            Definition::from_script(
                "CHLD",
                "Child",
                "#include KNWN\n#include MISS\npublic func OwnValue() { return 42; }",
            )
            .expect("child compiles"),
        )
        .expect("child registers");

    let messages = capture_warnings(|| {
        engine
            .resolve_includes()
            .expect("missing include is warning-only");
    });
    assert_eq!(messages, ["script to #include not found"]);

    let object = engine
        .spawn_object(SpawnConfig::new("CHLD"))
        .expect("child spawns after linking");
    let index = engine.find_object_index(object).expect("child exists");
    assert_eq!(
        engine
            .call_object_function(index, "OwnValue", Vec::new())
            .expect("own function remains callable"),
        Value::Int(42)
    );
    assert_eq!(
        engine
            .call_object_function(index, "ParentValue", Vec::new())
            .expect("known include still merged"),
        Value::Int(7)
    );
}

#[test]
fn appendto_nowarn_suppresses_only_its_missing_target_warning() {
    let mut quiet = Engine::new();
    register(
        &mut quiet,
        "APPA",
        "#appendto MISS nowarn\npublic func Quiet() { return 1; }",
    );
    register(&mut quiet, "TARG", "public func Own() { return 1; }");
    register(
        &mut quiet,
        "APPC",
        "#appendto TARG nowarn\npublic func Present() { return 1; }",
    );
    let quiet_messages = capture_warnings(|| quiet.resolve_appends());
    assert!(quiet_messages.is_empty());
    assert!(quiet.definition_script_has_function("TARG", "Present"));

    let mut loud = Engine::new();
    register(
        &mut loud,
        "APPB",
        "#appendto LOST\npublic func Loud() { return 1; }",
    );
    let loud_messages = capture_warnings(|| loud.resolve_appends());
    assert_eq!(loud_messages, ["script to #appendto not found"]);
}

/// The port's own `planet/System.c4g` appends target EkeReloaded-only
/// definitions (SF5B `EkeReloaded.c4d/Creatures.c4d/SFT.c4d`, RL5B
/// `EkeReloaded.c4d/Weapons.c4d/RocketLauncher.c4d`), which every other
/// scenario leaves unloaded. C4Aul's `nowarn` suffix is exactly the marker for
/// an optional target (C4AulLink.cpp:42-49), so an engine-global append that
/// omits it warns on every single launch.
#[test]
fn shipped_global_appends_stay_quiet_without_their_optional_targets() {
    let sources: Vec<(String, String)> = [
        (
            "EkeSftRelease.c",
            include_str!("../../../../planet/System.c4g/EkeSftRelease.c"),
        ),
        (
            "EkeGuidedMissile.c",
            include_str!("../../../../planet/System.c4g/EkeGuidedMissile.c"),
        ),
    ]
    .into_iter()
    .map(|(name, source)| (name.to_owned(), source.to_owned()))
    .collect();

    let mut engine = Engine::new();
    assert_eq!(engine.install_global_scripts(&sources), sources.len());

    let messages = capture_warnings(|| engine.resolve_appends());
    assert!(
        messages.is_empty(),
        "shipped global appends must not warn without EkeReloaded: {messages:?}"
    );
}

#[test]
fn circular_includes_follow_definition_load_order_and_warn_once() {
    for order in [["CYCA", "CYCB"], ["CYCB", "CYCA"]] {
        let mut engine = Engine::new();
        for id in order {
            let source = match id {
                "CYCA" => "#include CYCB\npublic func AOnly() { return 1; }",
                "CYCB" => "#include CYCA\npublic func BOnly() { return 2; }",
                _ => unreachable!(),
            };
            register(&mut engine, id, source);
        }

        let messages = capture_warnings(|| {
            engine.resolve_includes().expect("cycle resolves");
            engine.resolve_includes().expect("repeat resolve is stable");
        });
        assert_eq!(
            messages,
            ["Circular include chain detected - ignoring all includes!"],
            "registration order: {order:?}",
        );

        let first_function = if order[0] == "CYCA" { "AOnly" } else { "BOnly" };
        let second_function = if order[1] == "CYCA" { "AOnly" } else { "BOnly" };
        assert!(engine.definition_script_has_function(order[0], first_function));
        assert!(engine.definition_script_has_function(order[0], second_function));
        assert!(engine.definition_script_has_function(order[1], second_function));
        assert!(
            !engine.definition_script_has_function(order[1], first_function),
            "the later definition must skip its edge back to the DFS root"
        );
    }
}

#[test]
fn control_description_uses_effective_function_and_preserves_empty_first_segment() {
    let mut engine = Engine::new();
    register(
        &mut engine,
        "CDPA",
        "public func ControlSpecial() { [Parent caption|Image=CDPA] return 1; }",
    );
    register(
        &mut engine,
            "CDCH",
            "#include CDPA\npublic func ControlUp() { [|Image=CDCH] return 1; }\npublic func ControlDown() { return 1; }",
    );
    engine
        .resolve_includes()
        .expect("control descriptions link");

    assert_eq!(
        engine.definition_control_description("CDCH", "ControlSpecial"),
        Some("Parent caption".to_owned())
    );
    assert_eq!(
        engine.definition_control_description("CDCH", "ControlUp"),
        Some(String::new()),
        "a raw descriptor beginning with a separator suppresses name fallback"
    );
    assert_eq!(
        engine.definition_control_description("CDCH", "ControlDown"),
        None
    );
}

#[test]
fn longer_cycle_marks_the_root_resolved_for_later_backedges() {
    let mut engine = Engine::new();
    register(
        &mut engine,
        "CYCA",
        "#include CYCD\n#include CYCB\npublic func AOnly() { return 1; }",
    );
    register(
        &mut engine,
        "CYCB",
        "#include CYCC\npublic func BOnly() { return 2; }",
    );
    register(
        &mut engine,
        "CYCC",
        "#include CYCA\npublic func COnly() { return 3; }",
    );
    register(
        &mut engine,
        "CYCD",
        "#include CYCA\npublic func DOnly() { return 4; }",
    );

    let messages = capture_warnings(|| {
        engine.resolve_includes().expect("long cycle resolves");
        engine.resolve_includes().expect("repeat resolve is stable");
    });
    assert_eq!(
        messages,
        ["Circular include chain detected - ignoring all includes!"]
    );

    for function in ["AOnly", "BOnly", "COnly", "DOnly"] {
        assert!(engine.definition_script_has_function("CYCA", function));
    }
    assert!(engine.definition_script_has_function("CYCB", "BOnly"));
    assert!(engine.definition_script_has_function("CYCB", "COnly"));
    assert!(!engine.definition_script_has_function("CYCB", "AOnly"));
    assert!(engine.definition_script_has_function("CYCC", "COnly"));
    assert!(!engine.definition_script_has_function("CYCC", "AOnly"));
    for function in ["AOnly", "BOnly", "COnly", "DOnly"] {
        assert!(
            engine.definition_script_has_function("CYCD", function),
            "the later backedge sees the root's partially resolved function set"
        );
    }
}
