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
    render_cpu: Option<std::time::Duration>,
    update_allocation_calls: u64,
    update_allocation_bytes: u64,
    render_allocation_calls: u64,
    render_allocation_bytes: u64,
    rasterized_tiles: u64,
    reused_tiles: u64,
    render_max_allocation_bytes: u64,
    render_frame_sized_allocation_calls: u64,
    camera_origin: [f32; 2],
}

#[cfg(target_os = "linux")]
fn frame_profile_process_cpu_time() -> Option<std::time::Duration> {
    #[repr(C)]
    struct Timespec {
        seconds: std::ffi::c_long,
        nanoseconds: std::ffi::c_long,
    }
    unsafe extern "C" {
        fn clock_gettime(clock_id: std::ffi::c_int, time: *mut Timespec) -> std::ffi::c_int;
    }
    let mut time = Timespec {
        seconds: 0,
        nanoseconds: 0,
    };
    // Linux CLOCK_PROCESS_CPUTIME_ID includes the sole Rayon worker as well
    // as the calling thread. The initialized output has the native C layout.
    let result = unsafe { clock_gettime(2, &mut time) };
    (result == 0).then(|| std::time::Duration::new(time.seconds as u64, time.nanoseconds as u32))
}

#[cfg(not(target_os = "linux"))]
fn frame_profile_process_cpu_time() -> Option<std::time::Duration> {
    None
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
    RetainedCpu,
    RetainedCpuRedraw,
    RetainedCpuScroll,
    RetainedCpuLightning,
}

impl FrameProfilePath {
    fn label(self) -> &'static str {
        match self {
            Self::Software => "software",
            Self::Retained => "retained",
            Self::RetainedCpu => "retained_cpu",
            Self::RetainedCpuRedraw => "retained_cpu_redraw",
            Self::RetainedCpuScroll => "retained_cpu_scroll",
            Self::RetainedCpuLightning => "retained_cpu_lightning",
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
            app.render_immediate_oracle(frame).test_value();
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
        FrameProfilePath::RetainedCpu
        | FrameProfilePath::RetainedCpuRedraw
        | FrameProfilePath::RetainedCpuScroll
        | FrameProfilePath::RetainedCpuLightning => {
            if matches!(
                path,
                FrameProfilePath::RetainedCpuScroll | FrameProfilePath::RetainedCpuLightning
            ) {
                retained_cpu_stress_input(
                    app,
                    path == FrameProfilePath::RetainedCpuScroll,
                    path == FrameProfilePath::RetainedCpuLightning,
                );
            }
            if path == FrameProfilePath::RetainedCpuRedraw {
                for renderer in &mut app.presentation.cpu_scene_renderers {
                    renderer.invalidate();
                }
            }
            app.render(frame).test_value();
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
    let ((render, render_cpu), render_allocation_calls, render_allocation_bytes) =
        measure_app_profile_allocations(|| {
            let cpu_started = frame_profile_process_cpu_time();
            let started = std::time::Instant::now();
            present_frame(app, path, frame);
            let wall = started.elapsed();
            let cpu = cpu_started
                .zip(frame_profile_process_cpu_time())
                .map(|(started, finished)| finished.saturating_sub(started));
            (wall, cpu)
        });
    let camera_origin = app
        .rendering
        .graphics
        .active_viewport_projections()
        .first()
        .map_or([0.0; 2], |viewport| {
            [viewport.content_origin_x, viewport.content_origin_y]
        });
    FrameProfileSample {
        render_max_allocation_bytes: PROFILE_MAX_ALLOCATION_BYTES.load(AtomicOrdering::Relaxed),
        render_frame_sized_allocation_calls: PROFILE_FRAME_SIZED_ALLOCATION_CALLS
            .load(AtomicOrdering::Relaxed),
        camera_origin,
        update,
        snapshot,
        render,
        render_cpu,
        update_allocation_calls,
        update_allocation_bytes,
        render_allocation_calls,
        render_allocation_bytes,
        rasterized_tiles: app
            .presentation
            .cpu_scene_renderers
            .iter()
            .map(|r| r.stats().rasterized_tiles as u64)
            .sum(),
        reused_tiles: app
            .presentation
            .cpu_scene_renderers
            .iter()
            .map(|r| r.stats().reused_tiles as u64)
            .sum(),
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
    if let Some(directory) = std::env::var_os("CLONK_FRAME_PROFILE_OUTPUT") {
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&directory).test_value();
        let name = scenario_key.rsplit('/').next().test_value();
        let mut raw = String::from("sample,update_ns,snapshot_ns,render_wall_ns,render_process_cpu_ns,update_allocation_calls,update_allocation_bytes,render_allocation_calls,render_allocation_bytes,rasterized_tiles,reused_tiles,render_max_allocation_bytes,render_frame_sized_allocation_calls,camera_x,camera_y\n");
        for (index, sample) in samples.iter().enumerate() {
            use std::fmt::Write;
            writeln!(
                &mut raw,
                "{index},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
                sample.update.as_nanos(),
                sample.snapshot.as_nanos(),
                sample.render.as_nanos(),
                sample
                    .render_cpu
                    .map_or_else(String::new, |cpu| cpu.as_nanos().to_string()),
                sample.update_allocation_calls,
                sample.update_allocation_bytes,
                sample.render_allocation_calls,
                sample.render_allocation_bytes,
                sample.rasterized_tiles,
                sample.reused_tiles,
                sample.render_max_allocation_bytes,
                sample.render_frame_sized_allocation_calls,
                sample.camera_origin[0],
                sample.camera_origin[1]
            )
            .test_value();
        }
        std::fs::write(directory.join(format!("{name}-{}.csv", path.label())), raw).test_value();
    }
    eprintln!("scenario_frame_profile_extra scenario={scenario_key} path={} render_max_allocation_bytes={} render_frame_sized_allocation_calls_total={} camera_x_range={:?} camera_y_range={:?}", path.label(), samples.iter().map(|sample| sample.render_max_allocation_bytes).max().unwrap_or(0), samples.iter().map(|sample| sample.render_frame_sized_allocation_calls).sum::<u64>(), samples.iter().map(|sample| sample.camera_origin[0]).fold([f32::INFINITY,f32::NEG_INFINITY],|r,v|[r[0].min(v),r[1].max(v)]), samples.iter().map(|sample| sample.camera_origin[1]).fold([f32::INFINITY,f32::NEG_INFINITY],|r,v|[r[0].min(v),r[1].max(v)]));
    let snapshot = app.engine.snapshot();
    eprintln!(
        "scenario_frame_profile scenario={scenario_key} path={} window={FRAME_PROFILE_WIDTH}x{FRAME_PROFILE_HEIGHT} \
samples={} objects={} particles={} landscape={}x{} \
frame_p50_ms={:.3} frame_p95_ms={:.3} frame_p99_ms={:.3} \
update_p50_ms={:.3} update_p95_ms={:.3} \
snapshot_p50_ms={:.3} \
render_p50_ms={:.3} render_p95_ms={:.3} render_p99_ms={:.3} \
render_cpu_p50_ms={:.3} render_cpu_p95_ms={:.3} render_cpu_p99_ms={:.3} \
update_allocation_calls_mean={} update_allocation_bytes_mean={} \
render_allocation_calls_mean={} render_allocation_bytes_mean={} rasterized_tiles_mean={} reused_tiles_mean={}",
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
        frame_profile_percentile(samples, 0.50, |sample| sample.render_cpu.unwrap_or_default()),
        frame_profile_percentile(samples, 0.95, |sample| sample.render_cpu.unwrap_or_default()),
        frame_profile_percentile(samples, 0.99, |sample| sample.render_cpu.unwrap_or_default()),
        frame_profile_mean(samples, |sample| sample.update_allocation_calls),
        frame_profile_mean(samples, |sample| sample.update_allocation_bytes),
        frame_profile_mean(samples, |sample| sample.render_allocation_calls),
        frame_profile_mean(samples, |sample| sample.render_allocation_bytes),
        frame_profile_mean(samples, |sample| sample.rasterized_tiles),
        frame_profile_mean(samples, |sample| sample.reused_tiles),
    );
    #[cfg(target_os = "linux")]
    eprintln!(
        "scenario_frame_cpu scenario={scenario_key} path={} render_cpu_p50_ms={:.3} render_cpu_p95_ms={:.3} render_cpu_p99_ms={:.3}",
        path.label(),
        frame_profile_percentile(samples, 0.50, |sample| sample.render_cpu.unwrap_or_default()),
        frame_profile_percentile(samples, 0.95, |sample| sample.render_cpu.unwrap_or_default()),
        frame_profile_percentile(samples, 0.99, |sample| sample.render_cpu.unwrap_or_default()),
    );
}

fn profile_one_pass(
    prepared: &PreparedRealInstalledScenario,
    scenario_key: &str,
    path: FrameProfilePath,
) {
    struct SeedGuard;
    impl Drop for SeedGuard {
        fn drop(&mut self) {
            clonk_engine::particles::clear_presentation_safe_random_seed();
        }
    }
    let _seed = SeedGuard;
    crate::seed_classic_safe_random(587);
    clonk_engine::particles::install_presentation_safe_random_seed(587);
    let mut fixture = prepared.instantiate_with_window(
        "Frame Profile",
        false,
        FRAME_PROFILE_WIDTH,
        FRAME_PROFILE_HEIGHT,
    );
    let app = &mut fixture.app;
    let mut frame = vec![0_u8; FRAME_PROFILE_WIDTH as usize * FRAME_PROFILE_HEIGHT as usize * 4];
    if path == FrameProfilePath::RetainedCpuScroll {
        prepare_retained_cpu_observer(app);
    }
    if matches!(
        path,
        FrameProfilePath::RetainedCpuScroll | FrameProfilePath::RetainedCpuLightning
    ) {
        app.render(&mut frame).test_value();
    }
    for _ in 0..FRAME_PROFILE_WARMUP_FRAMES {
        app.test_update();
        present_frame(app, path, &mut frame);
    }
    // Pixel evidence is deliberately opt-in: disk I/O between frames is not
    // part of a timing run. Concatenated RGBA frames permit an exact `cmp`
    // against another revision, rather than comparing only a final screenshot.
    let output = std::env::var_os("LC_FRAME_PROFILE_OUTPUT").map(std::path::PathBuf::from);
    let stem = format!("{}-{}", scenario_key.replace('/', "_"), path.label());
    if let Some(output) = &output {
        std::fs::create_dir_all(output).test_value();
    }
    let mut pixels = output.as_ref().and_then(|output| {
        (path == FrameProfilePath::Software
            && std::env::var_os("LC_FRAME_PROFILE_PIXELS").is_some())
        .then(|| {
            std::io::BufWriter::new(
                std::fs::File::create(output.join(format!("{stem}.rgba"))).test_value(),
            )
        })
    });
    let mut digest = 0xcbf2_9ce4_8422_2325_u64;
    let reference = std::env::var_os("CLONK_FRAME_PROFILE_SPRITE_REFERENCE").is_some();
    let mut comparison = (path == FrameProfilePath::Software
        && std::env::var_os("CLONK_FRAME_PROFILE_COMPARE_RGBA").is_some())
    .then(|| vec![0; frame.len()]);
    let samples = (0..FRAME_PROFILE_MEASURED_FRAMES)
        .map(|_| {
            let sample = frame_profile_sample(app, path, &mut frame);
            if let Some(comparison) = comparison.as_mut() {
                // Separate correctness run: the second render is untimed, but
                // can affect caches, so never report it as a timing experiment.
                clonk_frontend::set_software_sprite_span_reference(!reference);
                present_frame(app, path, comparison);
                clonk_frontend::set_software_sprite_span_reference(reference);
                assert_eq!(
                    frame, *comparison,
                    "scalar/span RGBA mismatch: {scenario_key}"
                );
            }
            if path == FrameProfilePath::Software {
                // Outside the timers: compare every measured frame's exact RGBA
                // output between the scalar and span runs of one executable.
                digest = frame.iter().fold(digest, |hash, byte| {
                    (hash ^ u64::from(*byte)).wrapping_mul(0x100_0000_01b3)
                });
            }
            if let Some(pixels) = &mut pixels {
                std::io::Write::write_all(pixels, &frame).test_value();
            }
            sample
        })
        .collect::<Vec<_>>();
    if let Some(mut pixels) = pixels {
        std::io::Write::flush(&mut pixels).test_value();
    }
    if let Some(output) = output {
        let mut csv = String::from("frame,update_ns,snapshot_ns,render_ns");
        #[cfg(target_os = "linux")]
        csv.push_str(",render_cpu_ns");
        csv.push('\n');
        for (index, sample) in samples.iter().enumerate() {
            use std::fmt::Write;
            write!(
                csv,
                "{},{},{},{}",
                FRAME_PROFILE_WARMUP_FRAMES + index + 1,
                sample.update.as_nanos(),
                sample.snapshot.as_nanos(),
                sample.render.as_nanos(),
            )
            .test_value();
            #[cfg(target_os = "linux")]
            write!(csv, ",{}", sample.render_cpu.unwrap_or_default().as_nanos()).test_value();
            csv.push('\n');
        }
        std::fs::write(output.join(format!("{stem}.csv")), csv).test_value();
    }
    report_frame_profile(scenario_key, path, app, &samples);
    if let Some(directory) = std::env::var_os("CLONK_FRAME_PROFILE_OUTPUT_DIR") {
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&directory).test_value();
        let mut report = serde_json::json!({
            "scenario": scenario_key,
            "path": path.label(),
            "sprite_reference": reference,
            "compared_rgba": comparison.is_some(),
            "seed": app.engine.random_seed(),
            "width": FRAME_PROFILE_WIDTH,
            "height": FRAME_PROFILE_HEIGHT,
            "warmup": FRAME_PROFILE_WARMUP_FRAMES,
            "samples": FRAME_PROFILE_MEASURED_FRAMES,
            "measured_rgba_fnv64": format!("{digest:016x}"),
            "update_ns": samples.iter().map(|sample| sample.update.as_nanos() as u64).collect::<Vec<_>>(),
            "snapshot_ns": samples.iter().map(|sample| sample.snapshot.as_nanos() as u64).collect::<Vec<_>>(),
            "render_ns": samples.iter().map(|sample| sample.render.as_nanos() as u64).collect::<Vec<_>>(),
        });
        #[cfg(target_os = "linux")]
        {
            report["render_cpu_ns"] = serde_json::to_value(
                samples
                    .iter()
                    .map(|sample| sample.render_cpu.unwrap_or_default().as_nanos() as u64)
                    .collect::<Vec<_>>(),
            )
            .test_value();
        }
        let filename = format!(
            "{}-{}-{}.json",
            scenario_key.replace('/', "_"),
            path.label(),
            if reference { "reference" } else { "optimized" }
        );
        std::fs::write(
            directory.join(filename),
            serde_json::to_vec_pretty(&report).test_value(),
        )
        .test_value();
    }
    if path == FrameProfilePath::Software {
        eprintln!("scenario_frame_pixels scenario={scenario_key} seed={} measured_rgba_fnv64={digest:016x}", app.engine.random_seed());
    }
    clonk_engine::particles::clear_presentation_safe_random_seed();
}

fn profile_one_scenario(scenario_key: &str) {
    let prepared = PreparedRealInstalledScenario::new(scenario_key);
    for path in [
        FrameProfilePath::Software,
        FrameProfilePath::Retained,
        FrameProfilePath::RetainedCpu,
        FrameProfilePath::RetainedCpuRedraw,
    ] {
        if std::env::var("CLONK_FRAME_PROFILE_PATH")
            .map_or(true, |selected| selected == path.label())
        {
            profile_one_pass(&prepared, scenario_key, path);
        }
    }
}

#[test]
#[ignore = "manual production-path frame profiling probe; reports per-stage timings"]
fn scenario_frame_profile() {
    let reference = std::env::var_os("CLONK_FRAME_PROFILE_SPRITE_REFERENCE").is_some();
    clonk_frontend::set_software_sprite_span_reference(reference);
    eprintln!(
        "scenario_frame_mode sprite_reference={reference} load_average={}",
        std::fs::read_to_string("/proc/loadavg")
            .unwrap_or_default()
            .trim()
    );
    let selected = std::env::var("CLONK_FRAME_PROFILE_SCENARIO").ok();
    assert!(
        std::env::var("CLONK_FRAME_PROFILE_PATH").map_or(true, |selected| [
            "software",
            "retained",
            "retained_cpu",
            "retained_cpu_redraw"
        ]
        .contains(&selected.as_str())),
        "unknown profile path"
    );
    assert!(
        selected
            .as_ref()
            .is_none_or(|selected| FRAME_PROFILE_SCENARIOS.contains(&selected.as_str())),
        "unknown profile scenario"
    );
    for scenario_key in FRAME_PROFILE_SCENARIOS {
        if selected
            .as_ref()
            .is_none_or(|selected| selected == scenario_key)
        {
            profile_one_scenario(scenario_key);
        }
    }
}

#[test]
#[ignore = "manual retained CPU timing and exact production capture probe"]
fn scenario_cpu_scene_profile() {
    struct PresentationSeedGuard;
    impl Drop for PresentationSeedGuard {
        fn drop(&mut self) {
            clonk_engine::particles::clear_presentation_safe_random_seed();
        }
    }
    let _seed_guard = PresentationSeedGuard;
    let prepared = PreparedRealInstalledScenario::new(FRAME_PROFILE_SCENARIOS[2]);
    crate::seed_classic_safe_random(587);
    clonk_engine::particles::install_presentation_safe_random_seed(587);
    let mut oracle = prepared.instantiate_with_window(
        "CPU scene",
        false,
        FRAME_PROFILE_WIDTH,
        FRAME_PROFILE_HEIGHT,
    );
    crate::seed_classic_safe_random(587);
    clonk_engine::particles::install_presentation_safe_random_seed(587);
    let mut retained = prepared.instantiate_with_window(
        "CPU scene",
        false,
        FRAME_PROFILE_WIDTH,
        FRAME_PROFILE_HEIGHT,
    );
    let mut expected = vec![0; FRAME_PROFILE_WIDTH as usize * FRAME_PROFILE_HEIGHT as usize * 4];
    let mut actual = expected.clone();
    let mut renderer = clonk_graphics::CpuSceneRenderer::default();
    let warmup = std::env::var("CLONK_CPU_SCENE_PROFILE_WARMUP_FRAMES")
        .map(|value| value.parse::<usize>().test_value())
        .unwrap_or(FRAME_PROFILE_WARMUP_FRAMES);
    for _ in 0..warmup {
        oracle.app.test_update();
        retained.app.test_update();
        oracle
            .app
            .render_immediate_oracle(&mut expected)
            .test_value();
        retained.app.render(&mut actual).test_value();
    }
    for tick in 0..20 {
        oracle.app.test_update();
        retained.app.test_update();
        assert!(
            oracle.app.snapshot == retained.app.snapshot,
            "different input snapshots at tick {tick}"
        );
        oracle
            .app
            .render_immediate_oracle(&mut expected)
            .test_value();
        let gamma = retained.app.retained_gpu_frame_gamma();
        let gamma_mode =
            retained_gpu_gamma_mode(retained.app.rendering.graphics.advanced_renderer_config());
        let scene = retained
            .app
            .capture_retained_logical_gpu_frame(
                clonk_graphics::GpuPresentation::identity(
                    FRAME_PROFILE_WIDTH,
                    FRAME_PROFILE_HEIGHT,
                ),
                &gamma,
                gamma_mode,
                false,
            )
            .test_value();
        assert_eq!(scene.layers.len(), 1);
        let cpu_started = frame_profile_process_cpu_time();
        let started = std::time::Instant::now();
        renderer.invalidate();
        renderer
            .render(&scene.layers[0].scene, &mut actual)
            .test_value();
        eprintln!(
            "retained_cpu_capture tick={tick} elapsed_ns={} cpu_ns={:?}",
            started.elapsed().as_nanos(),
            cpu_started
                .zip(frame_profile_process_cpu_time())
                .map(|(start, end)| end.saturating_sub(start).as_nanos())
        );
        let mismatch = actual
            .iter()
            .zip(&expected)
            .position(|(actual, expected)| actual != expected);
        assert_eq!(
            mismatch,
            None,
            "tick={tick} first={:?}",
            mismatch.map(|offset| (
                offset / 4 % FRAME_PROFILE_WIDTH as usize,
                offset / 4 / FRAME_PROFILE_WIDTH as usize,
                &actual[offset / 4 * 4..offset / 4 * 4 + 4],
                &expected[offset / 4 * 4..offset / 4 * 4 + 4]
            ))
        );
    }
}

#[test]
#[ignore = "manual 500-frame exact comparison of all three production scenario captures"]
fn scenario_cpu_production_capture_profile() {
    assert_retained_cpu_real_scenarios_match_immediate_captures(500);
}

#[test]
#[ignore = "manual fast-scroll and full-screen lightning output profiling probe"]
fn scenario_cpu_stress_profile() {
    let scenario = FRAME_PROFILE_SCENARIOS[2];
    let prepared = PreparedRealInstalledScenario::new(scenario);
    for path in [
        FrameProfilePath::RetainedCpuScroll,
        FrameProfilePath::RetainedCpuLightning,
    ] {
        profile_one_pass(&prepared, scenario, path);
    }
}
