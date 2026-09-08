// Opt-in production-path frame profile for clonk-org/clonk-rs#1541.
//
// The engine-only profiling in that issue reached `Engine::advance_tick` and
// `Engine::snapshot` and stopped there: it could not see viewport
// compositing, landscape lowering or presentation. This probe drives the same
// `GameApp::update` and `GameApp::render` a running window drives, at a
// realistic resolution on shipped scenarios, so both halves of the frame are
// measured on one machine in one run.
//
// It stays ignored: the output is evidence for a decision, not a portable
// wall-clock assertion. Run it in a release build with the
// `presentation-profile` feature and retain the uncaptured output with the
// revision fingerprints.

const FRAME_PROFILE_WARMUP_FRAMES: usize = 200;
const FRAME_PROFILE_MEASURED_FRAMES: usize = 300;
const FRAME_PROFILE_WIDTH: u32 = 1280;
const FRAME_PROFILE_HEIGHT: u32 = 720;
const FRAME_PROFILE_SCENARIOS: [&str; 3] = [
    "Missions.c4f/SevenKeys.c4s",
    "Collection.c4f/Magus.c4f/SkyBridge.c4s",
    "Collection.c4f/Puzzles.c4f/4_TowerOfMagic.c4s",
];

#[derive(Clone, Copy, Debug, Default)]
struct FrameProfileSample {
    update: std::time::Duration,
    snapshot: std::time::Duration,
    render: std::time::Duration,
    update_allocation_calls: u64,
    update_allocation_bytes: u64,
    render_allocation_calls: u64,
    render_allocation_bytes: u64,
}

impl FrameProfileSample {
    fn frame(&self) -> std::time::Duration {
        self.update + self.render
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FrameProfilePath {
    /// `GameApp::render` composing into a CPU frame buffer.
    Software,
    /// `GameApp::render_retained_gpu_frame` lowering into a `GpuScene`.
    Retained,
}

impl FrameProfilePath {
    fn label(self) -> &'static str {
        match self {
            Self::Software => "software",
            Self::Retained => "retained",
        }
    }
}

/// Present one frame through `path`.
///
/// The two paths share the frontend's landscape cache, so a frame drawn
/// through one leaves the other nothing to rebuild. Each pass therefore drives
/// exactly one of them, over its own fixture.
fn present_frame(app: &mut GameApp, path: FrameProfilePath, frame: &mut [u8]) {
    match path {
        FrameProfilePath::Software => {
            app.render(frame).test_value();
        }
        FrameProfilePath::Retained => {
            let rendered = app
                .render_retained_gpu_frame(clonk_graphics::GpuPresentation::identity(
                    FRAME_PROFILE_WIDTH,
                    FRAME_PROFILE_HEIGHT,
                ))
                .test_value();
            drop(rendered);
        }
    }
}

fn frame_profile_sample(
    app: &mut GameApp,
    path: FrameProfilePath,
    frame: &mut [u8],
) -> FrameProfileSample {
    let ((update, snapshot), update_allocation_calls, update_allocation_bytes) =
        measure_app_profile_allocations(|| {
            let started = std::time::Instant::now();
            app.test_update();
            (started.elapsed(), app.engine.snapshot_timings().total)
        });
    let (render, render_allocation_calls, render_allocation_bytes) =
        measure_app_profile_allocations(|| {
            let started = std::time::Instant::now();
            present_frame(app, path, frame);
            started.elapsed()
        });
    FrameProfileSample {
        update,
        snapshot,
        render,
        update_allocation_calls,
        update_allocation_bytes,
        render_allocation_calls,
        render_allocation_bytes,
    }
}

fn frame_profile_percentile(
    samples: &[FrameProfileSample],
    fraction: f64,
    field: impl Fn(&FrameProfileSample) -> std::time::Duration,
) -> f64 {
    let mut sorted = samples.iter().map(field).collect::<Vec<_>>();
    sorted.sort_unstable();
    if sorted.is_empty() {
        return 0.0;
    }
    let index = ((sorted.len() - 1) as f64 * fraction).round() as usize;
    sorted[index].as_secs_f64() * 1_000.0
}

fn frame_profile_mean(
    samples: &[FrameProfileSample],
    field: impl Fn(&FrameProfileSample) -> u64,
) -> u64 {
    if samples.is_empty() {
        return 0;
    }
    samples.iter().map(field).sum::<u64>() / samples.len() as u64
}

fn report_frame_profile(
    scenario_key: &str,
    path: FrameProfilePath,
    app: &GameApp,
    samples: &[FrameProfileSample],
) {
    let snapshot = app.engine.snapshot();
    eprintln!(
        "scenario_frame_profile scenario={scenario_key} path={} window={FRAME_PROFILE_WIDTH}x{FRAME_PROFILE_HEIGHT} \
samples={} objects={} particles={} landscape={}x{} \
frame_p50_ms={:.3} frame_p95_ms={:.3} frame_p99_ms={:.3} \
update_p50_ms={:.3} update_p95_ms={:.3} \
snapshot_p50_ms={:.3} \
render_p50_ms={:.3} render_p95_ms={:.3} render_p99_ms={:.3} \
update_allocation_calls_mean={} update_allocation_bytes_mean={} \
render_allocation_calls_mean={} render_allocation_bytes_mean={}",
        path.label(),
        samples.len(),
        snapshot.objects.len(),
        snapshot.particles.len(),
        snapshot
            .landscape
            .as_ref()
            .and_then(clonk_engine::landscape::Landscape::pixel_grid)
            .map_or(0, clonk_engine::landscape::PixelGrid::width),
        snapshot
            .landscape
            .as_ref()
            .and_then(clonk_engine::landscape::Landscape::pixel_grid)
            .map_or(0, clonk_engine::landscape::PixelGrid::height),
        frame_profile_percentile(samples, 0.50, FrameProfileSample::frame),
        frame_profile_percentile(samples, 0.95, FrameProfileSample::frame),
        frame_profile_percentile(samples, 0.99, FrameProfileSample::frame),
        frame_profile_percentile(samples, 0.50, |sample| sample.update),
        frame_profile_percentile(samples, 0.95, |sample| sample.update),
        frame_profile_percentile(samples, 0.50, |sample| sample.snapshot),
        frame_profile_percentile(samples, 0.50, |sample| sample.render),
        frame_profile_percentile(samples, 0.95, |sample| sample.render),
        frame_profile_percentile(samples, 0.99, |sample| sample.render),
        frame_profile_mean(samples, |sample| sample.update_allocation_calls),
        frame_profile_mean(samples, |sample| sample.update_allocation_bytes),
        frame_profile_mean(samples, |sample| sample.render_allocation_calls),
        frame_profile_mean(samples, |sample| sample.render_allocation_bytes),
    );
}

fn profile_one_pass(
    prepared: &PreparedRealInstalledScenario,
    scenario_key: &str,
    path: FrameProfilePath,
) {
    let mut fixture = prepared.instantiate_with_window(
        "Frame Profile",
        false,
        FRAME_PROFILE_WIDTH,
        FRAME_PROFILE_HEIGHT,
    );
    let app = &mut fixture.app;
    let mut frame = vec![0_u8; FRAME_PROFILE_WIDTH as usize * FRAME_PROFILE_HEIGHT as usize * 4];
    for _ in 0..FRAME_PROFILE_WARMUP_FRAMES {
        app.test_update();
        present_frame(app, path, &mut frame);
    }
    let samples = (0..FRAME_PROFILE_MEASURED_FRAMES)
        .map(|_| frame_profile_sample(app, path, &mut frame))
        .collect::<Vec<_>>();
    report_frame_profile(scenario_key, path, app, &samples);
}

fn profile_one_scenario(scenario_key: &str) {
    let prepared = PreparedRealInstalledScenario::new(scenario_key);
    for path in [FrameProfilePath::Software, FrameProfilePath::Retained] {
        profile_one_pass(&prepared, scenario_key, path);
    }
}

#[test]
#[ignore = "manual production-path frame profiling probe; reports per-stage timings"]
fn scenario_frame_profile() {
    for scenario_key in FRAME_PROFILE_SCENARIOS {
        profile_one_scenario(scenario_key);
    }
}
