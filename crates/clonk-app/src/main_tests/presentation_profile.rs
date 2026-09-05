// Opt-in production-path measurement for clonk-org/clonk-rs#1486.
//
// This probe drives the same `GameApp::handle_cursor_moved` and
// `GameApp::update` paths as the running application. It deliberately stays
// ignored: the output is evidence for a decision, not a portable wall-clock
// assertion. Run it in a release build with the `presentation-profile`
// feature and retain the uncaptured output with the revision fingerprints.

use std::process::Command;

const WARMUP_FRAMES: usize = 20;
const MEASURED_FRAMES: usize = 600;
const PROFILE_SEED: u64 = 0;
const PROFILE_LANDSCAPE_WIDTH: u32 = 20_000;
const PROFILE_LANDSCAPE_HEIGHT: i32 = 360;
const PROFILE_VIEW_CENTER_X: i32 = 10_000;
const PROFILE_BUSY_OBJECTS: usize = 256;
const PROFILE_WORKLOADS: [usize; 2] = [0, PROFILE_BUSY_OBJECTS];

#[derive(Clone, Copy, Debug)]
struct ProfileSample {
    elapsed: std::time::Duration,
    projection: std::time::Duration,
    projection_count: u64,
    allocation_calls: u64,
    allocation_bytes: u64,
}

struct ProfileFixture {
    app: GameApp,
    interior: GuiPoint,
    left: GuiPoint,
    right: GuiPoint,
}

fn physical(point: GuiPoint) -> PhysicalPosition<f64> {
    PhysicalPosition::new(f64::from(point.x), f64::from(point.y))
}

fn profile_fixture(busy_objects: usize) -> ProfileFixture {
    let mut app = new_running_sandbox_app();
    let owner = app.players.local_owner;
    let original_focus = app.engine.test_crew_cursor(owner);

    // The shipped fallback world is intentionally small. Give the real app
    // path enough horizontal room that 600 retained edge ticks remain
    // successful scrolls, while keeping the same 320x200 window and player
    // input routing as the production fixture.
    let landscape = Landscape::flat(PROFILE_LANDSCAPE_WIDTH, PROFILE_LANDSCAPE_HEIGHT);
    app.engine.set_landscape(landscape);
    app.engine
        .apply_object_update(
            original_focus,
            ObjectUpdate::new()
                .with_position(Vector2::new(
                    PROFILE_VIEW_CENTER_X,
                    PROFILE_LANDSCAPE_HEIGHT - 40,
                ))
                .with_velocity(Vector2::ZERO)
                .with_action("Idle"),
        )
        .test_value();
    let focus = if busy_objects == 0 {
        original_focus
    } else {
        let mut profile_definition =
            Definition::from_script("PRFB", "Presentation Profile Body", "#strict\n").test_value();
        profile_definition
            .set_category(clonk_engine::CATEGORY_STATIC_BACK | clonk_engine::CATEGORY_OBJECT);
        app.engine.register_test_definition(profile_definition);
        let mut focus = None;
        for index in 0..busy_objects {
            let lane = i32::try_from(index % 32).test_value();
            let row = i32::try_from(index / 32).test_value();
            let owner_for_object = if index == 0 {
                owner
            } else {
                clonk_engine::OWNER_NONE
            };
            let object = app.engine.spawn_test_object(
                SpawnConfig::new("PRFB")
                    .with_owner(owner_for_object)
                    .with_position(Vector2::new(
                        PROFILE_VIEW_CENTER_X - 124 + lane * 8,
                        PROFILE_LANDSCAPE_HEIGHT - 40 - row * 5,
                    ))
                    .with_velocity(Vector2::ZERO)
                    .with_crew_member(index == 0)
                    .with_mobile(false),
            );
            if index == 0 {
                focus = Some(object);
            }
        }
        focus.test_value()
    };
    app.engine.set_crew_cursor(owner, Some(focus)).test_value();
    app.engine
        .replace_player_viewports(
            owner,
            vec![clonk_engine::PlayerViewport::new(Vector2::new(
                PROFILE_VIEW_CENTER_X,
                PROFILE_LANDSCAPE_HEIGHT,
            ))
            .with_focus(Some(focus))],
        )
        .test_value();
    app.snapshot = app.engine.snapshot();

    let mut frame = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut frame);
    let rect = app.graphics.viewport_rect(owner).test_value();
    let center_y = (rect.y + rect.height as i32 / 2) as f32;
    let interior = GuiPoint::new((rect.x + rect.width as i32 / 2) as f32, center_y);
    let left = GuiPoint::new(rect.x as f32, center_y);
    let right = GuiPoint::new((rect.x + rect.width as i32 - 1) as f32, center_y);
    for point in [left, right] {
        assert!(
            app.ingame_viewport_region(owner, point).is_none(),
            "profile edge point must enter the world viewport"
        );
        assert!(
            viewport_edge_scroll(rect, point).is_some(),
            "profile edge point must arm native edge scrolling"
        );
    }
    assert_eq!(
        app.engine.random_seed(),
        PROFILE_SEED,
        "sandbox profile must retain its fixed engine seed"
    );

    ProfileFixture {
        app,
        interior,
        left,
        right,
    }
}

fn edge_for(fixture: &ProfileFixture, index: usize) -> GuiPoint {
    if index.is_multiple_of(2) {
        fixture.left
    } else {
        fixture.right
    }
}

fn profile_projection_time(
    observed_count: u64,
    required_count: u64,
    snapshot_time: std::time::Duration,
) -> std::time::Duration {
    if observed_count >= required_count {
        snapshot_time
    } else {
        std::time::Duration::ZERO
    }
}

fn sample_interior(fixture: &mut ProfileFixture) -> ProfileSample {
    reset_edge_scroll_profile_counters();
    let ((elapsed, projection), allocation_calls, allocation_bytes) =
        measure_app_profile_allocations(|| {
            let started = std::time::Instant::now();
            fixture.app.test_cursor(physical(fixture.interior));
            (started.elapsed(), std::time::Duration::ZERO)
        });
    ProfileSample {
        elapsed,
        projection,
        projection_count: edge_scroll_profile_projection_count(),
        allocation_calls,
        allocation_bytes,
    }
}

fn sample_interior_tick(fixture: &mut ProfileFixture) -> ProfileSample {
    fixture.app.test_cursor(physical(fixture.interior));
    reset_edge_scroll_profile_counters();
    let ((elapsed, projection), allocation_calls, allocation_bytes) =
        measure_app_profile_allocations(|| {
            let started = std::time::Instant::now();
            fixture.app.test_update();
            let projection_count = edge_scroll_profile_projection_count();
            (
                started.elapsed(),
                profile_projection_time(
                    projection_count,
                    1,
                    fixture.app.engine.snapshot_timings().total,
                ),
            )
        });
    ProfileSample {
        elapsed,
        projection,
        projection_count: edge_scroll_profile_projection_count(),
        allocation_calls,
        allocation_bytes,
    }
}

fn sample_first_edge(fixture: &mut ProfileFixture, edge: GuiPoint) -> ProfileSample {
    // Clear the retained edge state outside the measured event. The measured
    // operation is exactly the first move into the border.
    fixture.app.test_cursor(physical(fixture.interior));
    reset_edge_scroll_profile_counters();
    let ((elapsed, projection), allocation_calls, allocation_bytes) =
        measure_app_profile_allocations(|| {
            let started = std::time::Instant::now();
            fixture.app.test_cursor(physical(edge));
            let projection_count = edge_scroll_profile_projection_count();
            (
                started.elapsed(),
                profile_projection_time(
                    projection_count,
                    1,
                    fixture.app.engine.snapshot_timings().total,
                ),
            )
        });
    ProfileSample {
        elapsed,
        projection,
        projection_count: edge_scroll_profile_projection_count(),
        allocation_calls,
        allocation_bytes,
    }
}

fn sample_stationary_tick(fixture: &mut ProfileFixture) -> ProfileSample {
    reset_edge_scroll_profile_counters();
    let ((elapsed, projection), allocation_calls, allocation_bytes) =
        measure_app_profile_allocations(|| {
            let started = std::time::Instant::now();
            fixture.app.test_update();
            let projection_count = edge_scroll_profile_projection_count();
            (
                started.elapsed(),
                profile_projection_time(
                    projection_count,
                    1,
                    fixture.app.engine.snapshot_timings().total,
                ),
            )
        });
    ProfileSample {
        elapsed,
        projection,
        projection_count: edge_scroll_profile_projection_count(),
        allocation_calls,
        allocation_bytes,
    }
}

fn sample_same_frame(fixture: &mut ProfileFixture, edge: GuiPoint) -> ProfileSample {
    // The event is delivered before the scheduler tick, matching the window
    // loop's same-frame ordering. Both production paths are inside the one
    // elapsed sample and each projection is retained separately until their
    // sum is returned.
    fixture.app.test_cursor(physical(fixture.interior));
    reset_edge_scroll_profile_counters();
    let ((elapsed, projection), allocation_calls, allocation_bytes) =
        measure_app_profile_allocations(|| {
            let started = std::time::Instant::now();
            fixture.app.test_cursor(physical(edge));
            let event_projection_count = edge_scroll_profile_projection_count();
            let event_projection = profile_projection_time(
                event_projection_count,
                1,
                fixture.app.engine.snapshot_timings().total,
            );
            fixture.app.test_update();
            let tick_projection_count = edge_scroll_profile_projection_count();
            let tick_projection = profile_projection_time(
                tick_projection_count,
                2,
                fixture.app.engine.snapshot_timings().total,
            );
            (
                started.elapsed(),
                event_projection.saturating_add(tick_projection),
            )
        });
    ProfileSample {
        elapsed,
        projection,
        projection_count: edge_scroll_profile_projection_count(),
        allocation_calls,
        allocation_bytes,
    }
}

fn percentile_duration(samples: &[std::time::Duration], fraction: f64) -> std::time::Duration {
    if samples.is_empty() {
        return std::time::Duration::ZERO;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let index = ((sorted.len() - 1) as f64 * fraction).round() as usize;
    sorted[index]
}

fn percentile_u64(samples: &[u64], fraction: f64) -> u64 {
    if samples.is_empty() {
        return 0;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let index = ((sorted.len() - 1) as f64 * fraction).round() as usize;
    sorted[index]
}

fn percentile_projection_share(samples: &[ProfileSample], fraction: f64) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let mut shares = samples
        .iter()
        .map(|sample| {
            if sample.elapsed.is_zero() {
                0.0
            } else {
                sample.projection.as_secs_f64() / sample.elapsed.as_secs_f64() * 100.0
            }
        })
        .collect::<Vec<_>>();
    shares.sort_unstable_by(f64::total_cmp);
    let index = ((shares.len() - 1) as f64 * fraction).round() as usize;
    shares[index]
}

fn report_case(
    label: &str,
    samples: &[ProfileSample],
    expected_projection_count: u64,
    busy_objects: usize,
) {
    let elapsed = samples
        .iter()
        .map(|sample| sample.elapsed)
        .collect::<Vec<_>>();
    let projection = samples
        .iter()
        .map(|sample| sample.projection)
        .collect::<Vec<_>>();
    let allocation_calls = samples
        .iter()
        .map(|sample| sample.allocation_calls)
        .collect::<Vec<_>>();
    let allocation_bytes = samples
        .iter()
        .map(|sample| sample.allocation_bytes)
        .collect::<Vec<_>>();
    let projection_count = samples
        .iter()
        .map(|sample| sample.projection_count)
        .sum::<u64>();
    let expected = expected_projection_count.saturating_mul(samples.len() as u64);
    assert_eq!(
        projection_count, expected,
        "{label} projection count must match the production-path baseline"
    );
    eprintln!(
        "edge_scroll_snapshot_profile busy_objects={} case={label} samples={} expected_projection_count={} projection_count={} elapsed_p50_ms={:.6} elapsed_p95_ms={:.6} elapsed_p99_ms={:.6} projection_p50_ms={:.6} projection_p95_ms={:.6} projection_p99_ms={:.6} projection_share_p50_pct={:.3} projection_share_p95_pct={:.3} projection_share_p99_pct={:.3} allocation_calls_total={} allocation_calls_p50={} allocation_calls_p95={} allocation_calls_p99={} allocation_bytes_total={} allocation_bytes_p50={} allocation_bytes_p95={} allocation_bytes_p99={}",
        busy_objects,
        samples.len(),
        expected_projection_count,
        projection_count,
        percentile_duration(&elapsed, 0.50).as_secs_f64() * 1_000.0,
        percentile_duration(&elapsed, 0.95).as_secs_f64() * 1_000.0,
        percentile_duration(&elapsed, 0.99).as_secs_f64() * 1_000.0,
        percentile_duration(&projection, 0.50).as_secs_f64() * 1_000.0,
        percentile_duration(&projection, 0.95).as_secs_f64() * 1_000.0,
        percentile_duration(&projection, 0.99).as_secs_f64() * 1_000.0,
        percentile_projection_share(samples, 0.50),
        percentile_projection_share(samples, 0.95),
        percentile_projection_share(samples, 0.99),
        allocation_calls.iter().sum::<u64>(),
        percentile_u64(&allocation_calls, 0.50),
        percentile_u64(&allocation_calls, 0.95),
        percentile_u64(&allocation_calls, 0.99),
        allocation_bytes.iter().sum::<u64>(),
        percentile_u64(&allocation_bytes, 0.50),
        percentile_u64(&allocation_bytes, 0.95),
        percentile_u64(&allocation_bytes, 0.99),
    );
}

fn command_one_line(program: &str, args: &[&str]) -> String {
    Command::new(program)
        .args(args)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| {
            String::from_utf8_lossy(&output.stdout)
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        })
        .filter(|output| !output.is_empty())
        .unwrap_or_else(|| "unavailable".to_owned())
}

fn git_revision(path: &std::path::Path) -> String {
    Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .filter(|output| !output.is_empty())
        .unwrap_or_else(|| "unavailable".to_owned())
}

fn report_profile_metadata(app: &GameApp, busy_objects: usize) {
    if cfg!(debug_assertions) {
        panic!("edge-scroll profile must run with cargo nextest --release");
    }
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let content = workspace.join("content");
    let snapshot = app.engine.snapshot();
    let source_status = git_profile_status(&workspace);
    let hostname = gethostname::gethostname().to_string_lossy().into_owned();
    eprintln!(
        "edge_scroll_snapshot_profile metadata host={} os={} arch={} build_profile=release package_version={} rustc={} cargo={} source_commit={} source_status={} content_commit={} seed={} window=320x200 landscape={}x{} world_height={} busy_objects={} active_objects={} snapshot_objects={} particles={} warmup_frames={} measured_frames={} workload=GameApp::new_running_sandbox_app synthetic PRFB objects when busy_objects>0, original sandbox crew when busy_objects=0; cursor interior/first-edge/stationary-edge-tick/same-frame-edge-plus-tick; horizontal edges alternate per measured edge sample",
        hostname,
        std::env::consts::OS,
        std::env::consts::ARCH,
        env!("CARGO_PKG_VERSION"),
        command_one_line("rustc", &["-vV"]),
        command_one_line("cargo", &["--version"]),
        git_revision(&workspace),
        source_status,
        git_revision(&content),
        app.engine.random_seed(),
        PROFILE_LANDSCAPE_WIDTH,
        PROFILE_LANDSCAPE_HEIGHT,
        app.engine
            .landscape()
            .map_or(0, Landscape::estimated_height),
        busy_objects,
        app.engine.active_object_count(),
        snapshot.objects.len(),
        snapshot.particles.len(),
        WARMUP_FRAMES,
        MEASURED_FRAMES,
    );
}

fn report_fixture_state(label: &str, app: &GameApp) {
    let snapshot = app.engine.snapshot();
    eprintln!(
        "edge_scroll_snapshot_profile state={} busy_objects={} frame={} mode={:?} player_status={:?} crew={:?} game_over={} cursor={:?} active_objects={} snapshot_objects={} particles={}",
        label,
        app.engine
            .object_count_for_definition("PRFB"),
        app.engine.frame(),
        app.mode,
        app.engine.test_player(app.players.local_owner).status(),
        app.engine
            .test_player(app.players.local_owner)
            .crew(),
        app.engine.is_game_over(),
        app.engine.crew_cursor(app.players.local_owner),
        app.engine.active_object_count(),
        snapshot.objects.len(),
        snapshot.particles.len(),
    );
}

fn git_profile_status(path: &std::path::Path) -> String {
    let paths = [
        "crates/clonk-app/Cargo.toml",
        "crates/clonk-app/src/main.rs",
        "crates/clonk-app/src/main_tests.rs",
        "crates/clonk-app/src/main_tests/presentation_profile.rs",
        "docs/PERFORMANCE.md",
    ];
    Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["status", "--short", "--untracked-files=all", "--"])
        .args(paths)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| {
            let output = String::from_utf8_lossy(&output.stdout);
            let status = output
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>();
            if status.is_empty() {
                "clean".to_owned()
            } else {
                status.join("|")
            }
        })
        .unwrap_or_else(|| "unavailable".to_owned())
}

#[test]
#[ignore = "manual production-path profiling probe; reports timings and allocation counts"]
fn edge_scroll_snapshot_profile() {
    for busy_objects in PROFILE_WORKLOADS {
        let mut interior_fixture = profile_fixture(busy_objects);
        report_profile_metadata(&interior_fixture.app, busy_objects);

        for _ in 0..WARMUP_FRAMES {
            interior_fixture.app.test_update();
        }
        report_fixture_state("interior_after_warmup", &interior_fixture.app);
        let interior_samples = (0..MEASURED_FRAMES)
            .map(|_| sample_interior(&mut interior_fixture))
            .collect::<Vec<_>>();
        report_case("interior_pointer_move", &interior_samples, 0, busy_objects);

        let mut interior_tick_fixture = profile_fixture(busy_objects);
        interior_tick_fixture
            .app
            .test_cursor(physical(interior_tick_fixture.interior));
        for _ in 0..WARMUP_FRAMES {
            interior_tick_fixture.app.test_update();
        }
        report_fixture_state("interior_tick_after_warmup", &interior_tick_fixture.app);
        let interior_tick_samples = (0..MEASURED_FRAMES)
            .map(|_| sample_interior_tick(&mut interior_tick_fixture))
            .collect::<Vec<_>>();
        report_case(
            "interior_pointer_successive_tick",
            &interior_tick_samples,
            0,
            busy_objects,
        );

        let mut first_edge_fixture = profile_fixture(busy_objects);
        for _ in 0..WARMUP_FRAMES {
            first_edge_fixture.app.test_update();
        }
        report_fixture_state("first_edge_after_warmup", &first_edge_fixture.app);
        let mut first_edge_samples = Vec::with_capacity(MEASURED_FRAMES);
        for index in 0..MEASURED_FRAMES {
            let edge = edge_for(&first_edge_fixture, index);
            first_edge_samples.push(sample_first_edge(&mut first_edge_fixture, edge));
        }
        report_case("first_move_into_edge", &first_edge_samples, 0, busy_objects);

        let mut stationary_fixture = profile_fixture(busy_objects);
        stationary_fixture
            .app
            .test_cursor(physical(stationary_fixture.interior));
        stationary_fixture
            .app
            .test_cursor(physical(stationary_fixture.left));
        for _ in 0..WARMUP_FRAMES {
            stationary_fixture.app.test_update();
        }
        report_fixture_state("stationary_after_warmup", &stationary_fixture.app);
        let stationary_samples = (0..MEASURED_FRAMES)
            .map(|_| sample_stationary_tick(&mut stationary_fixture))
            .collect::<Vec<_>>();
        report_case(
            "stationary_pointer_successive_tick",
            &stationary_samples,
            0,
            busy_objects,
        );

        let mut same_frame_fixture = profile_fixture(busy_objects);
        for index in 0..WARMUP_FRAMES {
            same_frame_fixture
                .app
                .test_cursor(physical(same_frame_fixture.interior));
            same_frame_fixture
                .app
                .test_cursor(physical(edge_for(&same_frame_fixture, index)));
            same_frame_fixture.app.test_update();
        }
        report_fixture_state("same_frame_after_warmup", &same_frame_fixture.app);
        let mut same_frame_samples = Vec::with_capacity(MEASURED_FRAMES);
        for index in 0..MEASURED_FRAMES {
            let edge = edge_for(&same_frame_fixture, index);
            same_frame_samples.push(sample_same_frame(&mut same_frame_fixture, edge));
        }
        report_case(
            "edge_event_same_frame_as_tick",
            &same_frame_samples,
            0,
            busy_objects,
        );
    }
}
