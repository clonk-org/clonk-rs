// Level discipline for the engine's fail-safe script path.

/// Records the level and message of every event emitted while it is installed,
/// so a test can assert on severity rather than on text alone.
#[derive(Clone, Default)]
struct LevelRecorder(std::sync::Arc<std::sync::Mutex<Vec<(tracing::Level, String)>>>);

impl LevelRecorder {
    fn events(&self) -> Vec<(tracing::Level, String)> {
        self.0.lock().expect("recorder is not poisoned").clone()
    }

    fn levels_mentioning(&self, needle: &str) -> Vec<tracing::Level> {
        self.events()
            .into_iter()
            .filter(|(_, message)| message.contains(needle))
            .map(|(level, _)| level)
            .collect()
    }
}

impl<S: tracing::Subscriber> tracing_subscriber::layer::Layer<S> for LevelRecorder {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut message = String::new();
        event.record(
            &mut |field: &tracing::field::Field, value: &dyn std::fmt::Debug| {
                if field.name() == "message" {
                    message = format!("{value:?}");
                }
            },
        );
        self.0
            .lock()
            .expect("recorder is not poisoned")
            .push((*event.metadata().level(), message));
    }
}

/// Drive a tick-driven callback that errors on every interval — the shape that
/// produces a log line per effect callback per frame from one buggy script.
fn run_failing_effect_ticks(recorder: LevelRecorder) {
    use tracing_subscriber::layer::SubscriberExt;

    let script = r#"#strict 3
    global func Initialize(state, random) {
        return { effects = [ { op = "add", name = "Broken", interval = 1 } ] };
    }

    global func FxBrokenTimer(state, effect, timer) {
        return ThisHostFunctionDoesNotExist();
    }

    global func Step(state, frame, random) {
        return nil;
    }
    "#;
    let subscriber = tracing_subscriber::registry().with(recorder);
    tracing::subscriber::with_default(subscriber, || {
        let mut engine = Engine::with_seed(11);
        engine
            .register_script_definition("Actor", "Actor", script)
            .expect("definition registers");
        engine
            .spawn_object(SpawnConfig::new("Actor"))
            .expect("spawn succeeds");
        for _ in 1..=3 {
            engine
                .tick_without_snapshot()
                .expect("the tick survives the Fx error");
        }
    });
}

/// Drive the creation callbacks `CreateObject` fires, with a `Construction`
/// that raises a runtime error — the shape whose only diagnostic is the one
/// the host function itself writes.
fn run_failing_created_construction(recorder: LevelRecorder) {
    use tracing_subscriber::layer::SubscriberExt;

    let parent_script = r#"#strict
func Construction() { CreateObject(CHLD, 0, 0, -1); return(1); }
"#;
    let child_script = r#"#strict
func Construction() { UnknownFn(); return(1); }
"#;
    let subscriber = tracing_subscriber::registry().with(recorder);
    tracing::subscriber::with_default(subscriber, || {
        let mut engine = Engine::with_seed(0);
        let parent = Definition::from_script("PRNT", "Parent", parent_script).expect("compiles");
        engine.register_definition(parent).expect("registers");
        let child = Definition::from_script("CHLD", "Child", child_script).expect("compiles");
        engine.register_definition(child).expect("registers");
        engine
            .spawn_object(SpawnConfig::new("PRNT").with_category(CATEGORY_OBJECT))
            .expect("the erroring Construction is a fail-safe game call");
    });
}

#[test]
fn a_tolerated_creation_callback_error_is_reported_with_its_frames() {
    // The creation callbacks `CreateObject` fires run through the same
    // fail-safe C4AulExec unwind as every other engine call
    // (C4Object.cpp:198-215, C4AulExec.cpp:1335-1346), so they owe a player
    // the same report: the error above, its ` by: ` trace below. Logging them
    // at debug and without frames drops the failure entirely at the default
    // `info` filter — and these host functions are where five of the fourteen
    // errors that blank `Melees.c4f/Queron3.c4s` surface.
    let recorder = LevelRecorder::default();
    run_failing_created_construction(recorder.clone());

    let levels = recorder.levels_mentioning("fail-safe");
    assert!(
        !levels.is_empty(),
        "the creation callback failure should be reported: {:?}",
        recorder.events()
    );
    assert!(
        levels.iter().all(|level| *level == tracing::Level::ERROR),
        "the tolerated creation error is filtered out by default: {levels:?}"
    );
    assert!(
        recorder
            .events()
            .iter()
            .any(|(_, message)| message.contains(" by: ")),
        "the creation callback failure was reported without a trace: {:?}",
        recorder.events()
    );
}

#[test]
fn a_tolerated_script_error_outranks_the_frames_that_trace_it() {
    // C4AulExec's fail-safe unwind reports the error first and its call frames
    // beneath it: `C4AulError::show` logs the message at `err`
    // (`src/C4Aul.cpp:32-37`), then every context dumps a " by: " line at info
    // (`src/C4AulExec.cpp:1335-1346`). Logging the message *below* its own
    // frames inverts that, and because the default filter is `info` the player
    // is left with a stack trace and no error to explain it.
    let recorder = LevelRecorder::default();
    run_failing_effect_ticks(recorder.clone());

    let frames = recorder.levels_mentioning(" by: ");
    assert!(
        !frames.is_empty(),
        "the tolerated error should still be traced: {:?}",
        recorder.events()
    );
    assert!(
        frames.iter().all(|level| *level == tracing::Level::INFO),
        "call frames reported off info: {frames:?}"
    );

    let levels = recorder.levels_mentioning("fail-safe");
    assert!(
        !levels.is_empty(),
        "the fail-safe path should still be reported somewhere: {:?}",
        recorder.events()
    );
    assert!(
        levels.iter().all(|level| *level == tracing::Level::ERROR),
        "the tolerated error is filtered out below its own frames: {levels:?}"
    );
}
