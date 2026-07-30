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

#[test]
fn a_fail_safe_callback_failure_is_not_a_warning() {
    // The fail-safe path is the designed, expected outcome of a script callback
    // that errors: the engine recovers and keeps ticking. Reporting a designed
    // outcome at `warn` — at or above the default filter — means one buggy
    // content script floods every user's log, and it drains `warn` of meaning
    // for the failures that really are abnormal.
    let recorder = LevelRecorder::default();
    run_failing_effect_ticks(recorder.clone());

    let levels = recorder.levels_mentioning("fail-safe");
    assert!(
        !levels.is_empty(),
        "the fail-safe path should still be reported somewhere: {:?}",
        recorder.events()
    );
    assert!(
        levels.iter().all(|level| *level == tracing::Level::DEBUG),
        "fail-safe recovery reported above debug: {levels:?}"
    );
}
