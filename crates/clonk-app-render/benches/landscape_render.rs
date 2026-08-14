use clonk_app_render::gpu_renderer::{
    GpuRendererStats, GpuTimestampPass, GpuTimestampSampleValidity, RetainedGpuRenderer,
};
use clonk_frontend::{CursorAtlas, DefinitionSprite, GraphicsSystem, HudGraphics};
use clonk_graphics::{
    BitmapFont, Color, GammaRamp, GpuCommand, GpuGammaLut, GpuGammaMode, GpuPresentation, GpuScene,
    GpuTextureId, GpuTextureResource, GpuVertex,
};
use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::HashMap;
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

const FRAME_800: [u32; 2] = [800, 600];
const FRAME_4K: [u32; 2] = [3_840, 2_160];
const FRAME_ONE_CHUNK: [u32; 2] = [64, 64];
const FOG_CHUNK_SIZE: u32 = 64;
const FOG_CHUNKS_800: usize = fog_chunk_count(FRAME_800);
const FOG_CHUNKS_4K: usize = fog_chunk_count(FRAME_4K);
const NO_BOX_FADES_COMMANDS_4K: usize = FOG_CHUNKS_4K * 2;
const LANDSCAPE_INSTANCE_STRIDE: usize = 72;
const LANDSCAPE_4K_INSTANCE_BYTES: usize = FOG_CHUNKS_4K * LANDSCAPE_INSTANCE_STRIDE;
const LANDSCAPE_4K_INSTANCE_BYTE_LIMIT: usize = 196 * 1024;
const BASE_EXTENT: [u32; 2] = [4_096, 4_096];
const GPU_TIMESTAMP_ATTEMPTS: usize = 8;

const fn fog_chunk_count(extent: [u32; 2]) -> usize {
    let columns = extent[0].div_ceil(FOG_CHUNK_SIZE) as usize;
    let rows = extent[1].div_ceil(FOG_CHUNK_SIZE) as usize;
    columns * rows
}

const _: () = {
    assert!(fog_chunk_count(FRAME_ONE_CHUNK) == 1);
    assert!(FOG_CHUNKS_800 == 130);
    assert!(FOG_CHUNKS_4K == 2_040);
    assert!(LANDSCAPE_4K_INSTANCE_BYTES == 146_880);
    assert!(LANDSCAPE_4K_INSTANCE_BYTES <= LANDSCAPE_4K_INSTANCE_BYTE_LIMIT);
};

static COUNT_ALLOCATIONS: AtomicBool = AtomicBool::new(false);
static ALLOCATION_CALLS: AtomicUsize = AtomicUsize::new(0);

struct CountingAllocator;

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNT_ALLOCATIONS.load(Ordering::Relaxed) {
            ALLOCATION_CALLS.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: this wrapper forwards the caller's allocation contract.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if COUNT_ALLOCATIONS.load(Ordering::Relaxed) {
            ALLOCATION_CALLS.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: this wrapper forwards the caller's allocation contract.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: `pointer` and `layout` came from the wrapped allocator.
        unsafe { System.dealloc(pointer, layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if COUNT_ALLOCATIONS.load(Ordering::Relaxed) {
            ALLOCATION_CALLS.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: this wrapper forwards the caller's reallocation contract.
        unsafe { System.realloc(pointer, layout, new_size) }
    }
}

fn measure_allocation_calls<T>(capture: impl FnOnce() -> T) -> (T, usize) {
    ALLOCATION_CALLS.store(0, Ordering::Relaxed);
    COUNT_ALLOCATIONS.store(true, Ordering::SeqCst);
    let result = capture();
    COUNT_ALLOCATIONS.store(false, Ordering::SeqCst);
    let calls = ALLOCATION_CALLS.load(Ordering::Relaxed);
    (result, calls)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct TimestampQueryStatus {
    requested: bool,
    supported: bool,
    enabled: bool,
}

impl TimestampQueryStatus {
    fn for_adapter(requested: bool, features: wgpu::Features) -> Self {
        let supported = features.contains(wgpu::Features::TIMESTAMP_QUERY);
        Self {
            requested,
            supported,
            enabled: requested && supported,
        }
    }
}

struct TimestampDevice {
    device: wgpu::Device,
    queue: wgpu::Queue,
}

struct BenchmarkDevices {
    _runtime: tokio::runtime::Runtime,
    device: wgpu::Device,
    queue: wgpu::Queue,
    timestamp: Option<TimestampDevice>,
    timestamp_status: TimestampQueryStatus,
    adapter_backend: wgpu::Backend,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct SceneGpuTimestampEvidence {
    duration_ns: f64,
    valid_samples: usize,
    invalid_period_samples: usize,
    counter_rollover_samples: usize,
    invalid_duration_samples: usize,
}

fn config_bool(raw: &str) -> bool {
    let value = raw.as_bytes();
    (value.first() == Some(&b'1') && !value.get(1).is_some_and(u8::is_ascii_digit))
        || value.starts_with(b"true")
}

fn timestamp_queries_requested() -> bool {
    std::env::var("LC_GPU_TIMESTAMP_QUERIES")
        .ok()
        .is_some_and(|value| config_bool(&value))
}

fn benchmark_graphics(extent: [u32; 2]) -> GraphicsSystem {
    GraphicsSystem::new(
        extent[0],
        extent[1],
        extent[1] as i32,
        "landscape capture benchmark",
        Arc::new(BitmapFont::new()),
        Arc::new(HashMap::<String, DefinitionSprite>::new()),
        Arc::new(CursorAtlas::empty()),
        Arc::new(HudGraphics::default()),
    )
}

fn assert_landscape_capture(scene: &GpuScene, extent: [u32; 2]) {
    assert_eq!(scene.logical_extent, extent);
    assert_eq!(scene.commands.len(), fog_chunk_count(extent));
    assert_eq!(scene.textures.len(), 1);
    assert!(
        scene
            .commands
            .iter()
            .all(|command| matches!(command, GpuCommand::Landscape { .. })),
        "the production fog capture must retain every native landscape chunk"
    );
}

fn warmed_capture_allocation_calls(extent: [u32; 2]) -> (GpuScene, usize) {
    let mut graphics = benchmark_graphics(extent);
    let gamma = GammaRamp::identity();
    for _ in 0..3 {
        let scene = graphics.capture_landscape_fog_for_benchmark(extent, &gamma);
        assert_landscape_capture(&scene, extent);
        black_box(scene);
    }
    let (scene, calls) =
        measure_allocation_calls(|| graphics.capture_landscape_fog_for_benchmark(extent, &gamma));
    assert_landscape_capture(&scene, extent);
    (scene, calls)
}

fn capture_allocation_probe() -> (GpuScene, GpuScene) {
    let (one_chunk, one_chunk_calls) = warmed_capture_allocation_calls(FRAME_ONE_CHUNK);
    let (normal_800, normal_800_calls) = warmed_capture_allocation_calls(FRAME_800);
    let (normal_4k, normal_4k_calls) = warmed_capture_allocation_calls(FRAME_4K);
    assert_eq!(
        normal_800_calls, normal_4k_calls,
        "warm capture allocation calls scaled from 130 to 2,040 fog chunks"
    );
    assert_eq!(
        one_chunk_calls, normal_4k_calls,
        "warm capture allocation calls scaled from one to 2,040 fog chunks"
    );
    println!(
        "landscape_capture allocation_calls warm_capture_ordinal=4 \
         one_chunk={one_chunk_calls} chunks_130={normal_800_calls} \
         chunks_2040={normal_4k_calls} scaling=flat"
    );
    black_box(one_chunk);
    (normal_800, normal_4k)
}

fn base_pixels() -> Arc<[u8]> {
    let mut pixels = Vec::with_capacity((BASE_EXTENT[0] * BASE_EXTENT[1] * 4) as usize);
    for y in 0..BASE_EXTENT[1] {
        for x in 0..BASE_EXTENT[0] {
            pixels.extend_from_slice(&[
                (32 + x.wrapping_mul(3)) as u8,
                (48 + y.wrapping_mul(5)) as u8,
                (64 + (x ^ y).wrapping_mul(7)) as u8,
                255,
            ]);
        }
    }
    Arc::from(pixels.into_boxed_slice())
}

fn base_texture(texture: GpuTextureId, pixels: Arc<[u8]>) -> GpuTextureResource {
    GpuTextureResource::immutable_rgba(texture, BASE_EXTENT[0], BASE_EXTENT[1], pixels)
}

fn fog_modulation(x: u32, y: u32) -> [f32; 4] {
    let red = 96 + (x / 8 + y / 16) % 160;
    let green = 96 + (x / 16 + y / 8) % 160;
    let blue = 96 + (x / 32 + y / 32) % 160;
    [
        red as f32 / 255.0,
        green as f32 / 255.0,
        blue as f32 / 255.0,
        0.0,
    ]
}

fn fog_quad(left: u32, top: u32, right: u32, bottom: u32) -> [GpuVertex; 4] {
    let vertex = |x, y| {
        GpuVertex::new(
            [x as f32, y as f32, 1.0],
            [
                x as f32 / BASE_EXTENT[0] as f32,
                y as f32 / BASE_EXTENT[1] as f32,
            ],
            fog_modulation(x, y),
        )
    };
    [
        vertex(left, top),
        vertex(right, top),
        vertex(left, bottom),
        vertex(right, bottom),
    ]
}

fn landscape_command(base: GpuTextureId, vertices: [GpuVertex; 4]) -> GpuCommand {
    GpuCommand::Landscape {
        base,
        liquid_mask: None,
        liquid: None,
        vertices,
        clip: None,
        phase: [0.0; 3],
        gamma: false,
    }
}

fn landscape_scene(
    extent: [u32; 2],
    base: GpuTextureId,
    pixels: Arc<[u8]>,
    no_box_fades: bool,
) -> GpuScene {
    let fog_chunks = fog_chunk_count(extent);
    let expected_commands = if no_box_fades {
        fog_chunks * 2
    } else {
        fog_chunks
    };
    let mut commands = Vec::with_capacity(if no_box_fades {
        expected_commands
    } else {
        fog_chunks
    });

    // LegacyClonk 7d43b47b7d789b533f32d005e64596e0a07019cd lowers a fogged
    // landscape blit to source-aligned chunks no larger than 64 pixels and
    // uses each strip triangle's provoking colour for NoBoxFades
    // (src/StdGL.cpp:667,710-763).
    for top in (0..extent[1]).step_by(FOG_CHUNK_SIZE as usize) {
        let bottom = top.saturating_add(FOG_CHUNK_SIZE).min(extent[1]);
        for left in (0..extent[0]).step_by(FOG_CHUNK_SIZE as usize) {
            let right = left.saturating_add(FOG_CHUNK_SIZE).min(extent[0]);
            let quad = fog_quad(left, top, right, bottom);
            if no_box_fades {
                let bottom_left = quad[2];
                let bottom_right = quad[3];
                commands.push(landscape_command(
                    base,
                    [
                        GpuVertex::new(quad[0].position, quad[0].uv, bottom_left.modulation),
                        GpuVertex::new(quad[1].position, quad[1].uv, bottom_left.modulation),
                        bottom_left,
                        bottom_left,
                    ],
                ));
                commands.push(landscape_command(
                    base,
                    [
                        GpuVertex::new(
                            bottom_left.position,
                            bottom_left.uv,
                            bottom_right.modulation,
                        ),
                        GpuVertex::new(quad[1].position, quad[1].uv, bottom_right.modulation),
                        bottom_right,
                        bottom_right,
                    ],
                ));
            } else {
                commands.push(landscape_command(base, quad));
            }
        }
    }

    assert_eq!(commands.len(), expected_commands);
    GpuScene {
        logical_extent: extent,
        clear: Color::opaque(8, 12, 24),
        gamma: GpuGammaLut::from_ramp(&GammaRamp::identity()),
        gamma_mode: GpuGammaMode::Disabled,
        textures: vec![base_texture(base, pixels)],
        commands,
    }
}

fn benchmark_devices() -> BenchmarkDevices {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build landscape benchmark runtime");
    let instance = wgpu::Instance::default();
    let adapter = runtime
        .block_on(async {
            let primary = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    compatible_surface: None,
                    force_fallback_adapter: false,
                })
                .await;
            if primary.is_ok() {
                primary
            } else {
                instance
                    .request_adapter(&wgpu::RequestAdapterOptions {
                        power_preference: wgpu::PowerPreference::LowPower,
                        compatible_surface: None,
                        force_fallback_adapter: true,
                    })
                    .await
            }
        })
        .expect("landscape benchmark requires a wgpu adapter");
    let timestamp_status =
        TimestampQueryStatus::for_adapter(timestamp_queries_requested(), adapter.features());
    let limits = wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits());
    let baseline_descriptor = wgpu::DeviceDescriptor {
        label: Some("lc_landscape_benchmark_feature_empty_device"),
        required_features: wgpu::Features::empty(),
        required_limits: limits.clone(),
        ..Default::default()
    };
    let (device, queue) = runtime
        .block_on(adapter.request_device(&baseline_descriptor))
        .expect("request feature-empty landscape benchmark device");
    assert_eq!(device.features(), wgpu::Features::empty());

    let timestamp = timestamp_status.enabled.then(|| {
        let descriptor = wgpu::DeviceDescriptor {
            label: Some("lc_landscape_benchmark_timestamp_device"),
            required_features: wgpu::Features::TIMESTAMP_QUERY,
            required_limits: limits,
            ..Default::default()
        };
        let (device, queue) = runtime
            .block_on(adapter.request_device(&descriptor))
            .expect("request timestamp-enabled landscape benchmark device");
        assert_eq!(device.features(), wgpu::Features::TIMESTAMP_QUERY);
        TimestampDevice { device, queue }
    });
    assert_eq!(timestamp.is_some(), timestamp_status.enabled);

    BenchmarkDevices {
        _runtime: runtime,
        device,
        queue,
        timestamp,
        timestamp_status,
        adapter_backend: adapter.get_info().backend,
    }
}

fn render_completed_frame(
    renderer: &mut RetainedGpuRenderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target_view: &wgpu::TextureView,
    scene: &GpuScene,
    presentation: &GpuPresentation,
) -> GpuRendererStats {
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("lc_landscape_benchmark_encoder"),
    });
    renderer
        .render(
            device,
            queue,
            &mut encoder,
            target_view,
            scene,
            presentation,
            false,
        )
        .expect("render landscape benchmark scene");
    let stats = renderer.last_stats();
    queue.submit([encoder.finish()]);
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("wait for landscape benchmark submission");
    black_box(stats)
}

fn benchmark_target(
    device: &wgpu::Device,
    extent: [u32; 2],
    label: &'static str,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: extent[0],
            height: extent[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

fn assert_compact(stats: GpuRendererStats, expected_instances: usize) {
    assert_eq!(stats.draw_calls, 1, "compatible chunks need one scene draw");
    assert_eq!(stats.landscape_draw_calls, 1);
    assert_eq!(stats.compatible_resource_runs, 1);
    assert_eq!(
        stats.total_draw_calls, 2,
        "the fixed final presentation is the only additional draw"
    );
    assert_eq!(stats.presentation_draw_calls, 1);
    assert!(stats.has_exact_draw_call_counts());
    assert_eq!(stats.generic_vertices, 0);
    assert_eq!(stats.generic_vertex_upload_bytes, 0);
    assert_eq!(stats.landscape_instances, expected_instances);
    assert_eq!(
        stats.landscape_instance_upload_bytes,
        expected_instances * LANDSCAPE_INSTANCE_STRIDE
    );
}

fn warmed_compact_stats(
    renderer: &mut RetainedGpuRenderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target_view: &wgpu::TextureView,
    scene: &GpuScene,
    presentation: &GpuPresentation,
    expected_instances: usize,
) -> GpuRendererStats {
    let warm = render_completed_frame(renderer, device, queue, target_view, scene, presentation);
    assert_compact(warm, expected_instances);
    let measured =
        render_completed_frame(renderer, device, queue, target_view, scene, presentation);
    assert_compact(measured, expected_instances);
    measured
}

fn scene_gpu_timestamp_evidence(
    renderer: &mut RetainedGpuRenderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target_view: &wgpu::TextureView,
    scene: &GpuScene,
    presentation: &GpuPresentation,
    expected_instances: usize,
) -> SceneGpuTimestampEvidence {
    assert!(renderer.timestamp_queries_enabled());

    let warm = render_completed_frame(renderer, device, queue, target_view, scene, presentation);
    assert_compact(warm, expected_instances);
    let warm_frame_id = warm
        .timestamp_frame_id
        .expect("timestamp-enabled renderer must identify its warm frame");
    let warm_frames = renderer
        .drain_timestamp_frames(device)
        .expect("drain warm landscape timestamp frame");
    assert_eq!(
        warm_frames.len(),
        1,
        "one warm submission must produce exactly one timestamp frame"
    );
    assert_eq!(warm_frames[0].frame_id, warm_frame_id);

    let mut valid_durations = Vec::with_capacity(GPU_TIMESTAMP_ATTEMPTS);
    let mut evidence = SceneGpuTimestampEvidence::default();
    let mut previous_frame_id = warm_frame_id;
    for _ in 0..GPU_TIMESTAMP_ATTEMPTS {
        let measured =
            render_completed_frame(renderer, device, queue, target_view, scene, presentation);
        assert_compact(measured, expected_instances);
        let frame_id = measured
            .timestamp_frame_id
            .expect("timestamp-enabled renderer must identify its frame");
        assert_eq!(
            frame_id,
            previous_frame_id + 1,
            "timestamp frame identifiers must advance once per submission"
        );
        previous_frame_id = frame_id;
        let frames = renderer
            .drain_timestamp_frames(device)
            .expect("drain measured landscape timestamp frame");
        assert_eq!(
            frames.len(),
            1,
            "one measured submission must produce exactly one timestamp frame"
        );
        let mut matching_frames = frames.iter().filter(|frame| frame.frame_id == frame_id);
        let frame = matching_frames
            .next()
            .expect("measured landscape timestamp frame must be present");
        assert!(
            matching_frames.next().is_none(),
            "timestamp frame identifiers must be unique"
        );
        let mut scene_samples = frame
            .passes
            .iter()
            .filter(|sample| sample.pass == GpuTimestampPass::Scene);
        let sample = scene_samples
            .next()
            .expect("timestamp frame must name the Scene pass");
        assert!(
            scene_samples.next().is_none(),
            "timestamp frame must contain exactly one Scene pass"
        );
        match sample.validity {
            GpuTimestampSampleValidity::Valid => {
                valid_durations.push(
                    sample
                        .duration_ns
                        .expect("a valid Scene timestamp sample must have a duration"),
                );
                evidence.valid_samples += 1;
            }
            GpuTimestampSampleValidity::InvalidPeriod => {
                assert!(sample.duration_ns.is_none());
                evidence.invalid_period_samples += 1;
            }
            GpuTimestampSampleValidity::CounterRollover => {
                assert!(sample.duration_ns.is_none());
                evidence.counter_rollover_samples += 1;
            }
            GpuTimestampSampleValidity::InvalidDuration => {
                assert!(sample.duration_ns.is_none());
                evidence.invalid_duration_samples += 1;
            }
        }
    }
    assert_eq!(
        evidence.valid_samples
            + evidence.invalid_period_samples
            + evidence.counter_rollover_samples
            + evidence.invalid_duration_samples,
        GPU_TIMESTAMP_ATTEMPTS
    );
    assert!(
        !valid_durations.is_empty(),
        "all {GPU_TIMESTAMP_ATTEMPTS} supported Scene timestamp attempts were invalid: \
         {evidence:?}"
    );
    valid_durations.sort_by(f64::total_cmp);
    evidence.duration_ns = valid_durations[valid_durations.len() / 2];
    evidence
}

fn print_render_evidence(
    workload: &str,
    extent: [u32; 2],
    fog_chunks: usize,
    source_commands: usize,
    stats: GpuRendererStats,
    scene_gpu_timestamp: Option<SceneGpuTimestampEvidence>,
) {
    let duration = scene_gpu_timestamp.map_or_else(
        || "unavailable".to_owned(),
        |evidence| format!("{:.3}", evidence.duration_ns),
    );
    let duration_stat = scene_gpu_timestamp.map_or("unavailable", |_| "upper_median_valid");
    let attempts = scene_gpu_timestamp.map_or(0, |_| GPU_TIMESTAMP_ATTEMPTS);
    let valid_samples = scene_gpu_timestamp.map_or(0, |evidence| evidence.valid_samples);
    let invalid_period_samples =
        scene_gpu_timestamp.map_or(0, |evidence| evidence.invalid_period_samples);
    let counter_rollover_samples =
        scene_gpu_timestamp.map_or(0, |evidence| evidence.counter_rollover_samples);
    let invalid_duration_samples =
        scene_gpu_timestamp.map_or(0, |evidence| evidence.invalid_duration_samples);
    println!(
        "landscape_render evidence workload={workload} extent={}x{} fog_chunks={fog_chunks} \
         source_commands={source_commands} landscape_instances={} instance_stride_bytes={} \
         landscape_instance_upload_bytes={} generic_vertices={} generic_vertex_upload_bytes={} \
         scene_draw_calls={} total_draw_calls={} stream_packing_upload_ns={} \
         gpu_timestamp_pass=Scene scene_gpu_duration_ns={duration} timestamp_attempts={attempts} \
         timestamp_duration_stat={duration_stat} \
         timestamp_valid_samples={valid_samples} \
         timestamp_invalid_period_samples={invalid_period_samples} \
         timestamp_counter_rollover_samples={counter_rollover_samples} \
         timestamp_invalid_duration_samples={invalid_duration_samples}",
        extent[0],
        extent[1],
        stats.landscape_instances,
        LANDSCAPE_INSTANCE_STRIDE,
        stats.landscape_instance_upload_bytes,
        stats.generic_vertices,
        stats.generic_vertex_upload_bytes,
        stats.draw_calls,
        stats.total_draw_calls,
        stats.cpu_stages.stream_packing_upload.as_nanos(),
    );
}

fn bench_landscape_render(c: &mut Criterion) {
    let (normal_800, normal_4k) = capture_allocation_probe();
    let gamma = GammaRamp::identity();
    let mut capture_800 = benchmark_graphics(FRAME_800);
    let mut capture_4k = benchmark_graphics(FRAME_4K);
    for _ in 0..3 {
        black_box(capture_800.capture_landscape_fog_for_benchmark(FRAME_800, &gamma));
        black_box(capture_4k.capture_landscape_fog_for_benchmark(FRAME_4K, &gamma));
    }
    let mut capture_group = c.benchmark_group("landscape_capture");
    capture_group.throughput(Throughput::Elements(FOG_CHUNKS_800 as u64));
    capture_group.bench_function("fogged_800x600_130_chunks", |b| {
        b.iter(|| black_box(capture_800.capture_landscape_fog_for_benchmark(FRAME_800, &gamma)));
    });
    capture_group.throughput(Throughput::Elements(FOG_CHUNKS_4K as u64));
    capture_group.bench_function("fogged_4k_2040_chunks", |b| {
        b.iter(|| black_box(capture_4k.capture_landscape_fog_for_benchmark(FRAME_4K, &gamma)));
    });
    capture_group.finish();
    drop(capture_800);
    drop(capture_4k);

    let base = GpuTextureId::fresh();
    let pixels = base_pixels();
    let no_box_fades_4k = landscape_scene(FRAME_4K, base, pixels, true);
    let devices = benchmark_devices();
    let device = &devices.device;
    let queue = &devices.queue;
    let presentation_800 = GpuPresentation::identity(FRAME_800[0], FRAME_800[1]);
    let presentation_4k = GpuPresentation::identity(FRAME_4K[0], FRAME_4K[1]);
    let (_target_800, target_800_view) =
        benchmark_target(device, FRAME_800, "lc_landscape_benchmark_target_800");
    let (_target_4k, target_4k_view) =
        benchmark_target(device, FRAME_4K, "lc_landscape_benchmark_target_4k");
    let mut renderer = RetainedGpuRenderer::new(device, queue, wgpu::TextureFormat::Rgba8Unorm);
    assert!(!renderer.timestamp_queries_enabled());

    let normal_800_stats = warmed_compact_stats(
        &mut renderer,
        device,
        queue,
        &target_800_view,
        &normal_800,
        &presentation_800,
        FOG_CHUNKS_800,
    );
    let normal_4k_stats = warmed_compact_stats(
        &mut renderer,
        device,
        queue,
        &target_4k_view,
        &normal_4k,
        &presentation_4k,
        FOG_CHUNKS_4K,
    );
    let no_box_fades_stats = warmed_compact_stats(
        &mut renderer,
        device,
        queue,
        &target_4k_view,
        &no_box_fades_4k,
        &presentation_4k,
        NO_BOX_FADES_COMMANDS_4K,
    );
    assert!(normal_800_stats.timestamp_frame_id.is_none());
    assert!(normal_4k_stats.timestamp_frame_id.is_none());
    assert!(no_box_fades_stats.timestamp_frame_id.is_none());
    assert_eq!(
        normal_4k_stats.landscape_instance_upload_bytes,
        LANDSCAPE_4K_INSTANCE_BYTES
    );
    assert!(normal_4k_stats.landscape_instance_upload_bytes <= LANDSCAPE_4K_INSTANCE_BYTE_LIMIT);

    let gpu_durations = devices.timestamp.as_ref().map(|timestamp| {
        let (_target_800, target_800_view) = benchmark_target(
            &timestamp.device,
            FRAME_800,
            "lc_landscape_timestamp_target_800",
        );
        let (_target_4k, target_4k_view) = benchmark_target(
            &timestamp.device,
            FRAME_4K,
            "lc_landscape_timestamp_target_4k",
        );
        let mut timestamp_renderer = RetainedGpuRenderer::new(
            &timestamp.device,
            &timestamp.queue,
            wgpu::TextureFormat::Rgba8Unorm,
        );
        let normal_800_timestamp = scene_gpu_timestamp_evidence(
            &mut timestamp_renderer,
            &timestamp.device,
            &timestamp.queue,
            &target_800_view,
            &normal_800,
            &presentation_800,
            FOG_CHUNKS_800,
        );
        let normal_4k_timestamp = scene_gpu_timestamp_evidence(
            &mut timestamp_renderer,
            &timestamp.device,
            &timestamp.queue,
            &target_4k_view,
            &normal_4k,
            &presentation_4k,
            FOG_CHUNKS_4K,
        );
        let no_box_fades_timestamp = scene_gpu_timestamp_evidence(
            &mut timestamp_renderer,
            &timestamp.device,
            &timestamp.queue,
            &target_4k_view,
            &no_box_fades_4k,
            &presentation_4k,
            NO_BOX_FADES_COMMANDS_4K,
        );
        let telemetry = timestamp_renderer.timestamp_telemetry();
        assert_eq!(telemetry.dropped_frames, 0);
        assert_eq!(telemetry.device_discontinuities, 0);
        (
            normal_800_timestamp,
            normal_4k_timestamp,
            no_box_fades_timestamp,
            telemetry,
        )
    });
    println!(
        "landscape_render timestamp_queries requested={} supported={} enabled={} \
         baseline_device_features={:?} timestamp_device_features={:?} adapter_backend={:?} \
         dropped_frames={} readback_errors={} device_discontinuities={}",
        devices.timestamp_status.requested,
        devices.timestamp_status.supported,
        devices.timestamp_status.enabled,
        device.features(),
        devices
            .timestamp
            .as_ref()
            .map_or(wgpu::Features::empty(), |timestamp| timestamp
                .device
                .features()),
        devices.adapter_backend,
        gpu_durations.map_or(0, |durations| durations.3.dropped_frames),
        gpu_durations.map_or(0, |durations| durations.3.readback_errors),
        gpu_durations.map_or(0, |durations| durations.3.device_discontinuities),
    );
    print_render_evidence(
        "fogged_800x600_130_chunks",
        FRAME_800,
        FOG_CHUNKS_800,
        normal_800.commands.len(),
        normal_800_stats,
        gpu_durations.map(|durations| durations.0),
    );
    print_render_evidence(
        "fogged_4k_2040_chunks",
        FRAME_4K,
        FOG_CHUNKS_4K,
        normal_4k.commands.len(),
        normal_4k_stats,
        gpu_durations.map(|durations| durations.1),
    );
    print_render_evidence(
        "no_box_fades_4k_4080_commands",
        FRAME_4K,
        FOG_CHUNKS_4K,
        no_box_fades_4k.commands.len(),
        no_box_fades_stats,
        gpu_durations.map(|durations| durations.2),
    );
    println!(
        "landscape renderer raw stats: normal_800={normal_800_stats:?}, \
         normal_4k={normal_4k_stats:?}, \
         no_box_fades={no_box_fades_stats:?}, \
         timing_scope=prebuilt_scene_renderer_encode_submit_and_device_poll_wall_time",
    );

    let mut group = c.benchmark_group("landscape_render");
    group.throughput(Throughput::Elements(FOG_CHUNKS_800 as u64));
    group.bench_function("fogged_800x600_130_chunks", |b| {
        b.iter(|| {
            let stats = render_completed_frame(
                &mut renderer,
                device,
                queue,
                &target_800_view,
                &normal_800,
                &presentation_800,
            );
            assert_compact(stats, FOG_CHUNKS_800);
        });
    });
    group.throughput(Throughput::Elements(FOG_CHUNKS_4K as u64));
    group.bench_function("fogged_4k_2040_chunks", |b| {
        b.iter(|| {
            let stats = render_completed_frame(
                &mut renderer,
                device,
                queue,
                &target_4k_view,
                &normal_4k,
                &presentation_4k,
            );
            assert_compact(stats, FOG_CHUNKS_4K);
        });
    });
    group.throughput(Throughput::Elements(NO_BOX_FADES_COMMANDS_4K as u64));
    group.bench_function("no_box_fades_4k_4080_commands", |b| {
        b.iter(|| {
            let stats = render_completed_frame(
                &mut renderer,
                device,
                queue,
                &target_4k_view,
                &no_box_fades_4k,
                &presentation_4k,
            );
            assert_compact(stats, NO_BOX_FADES_COMMANDS_4K);
        });
    });
    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .warm_up_time(Duration::from_secs(2))
        .measurement_time(Duration::from_secs(5))
        .sample_size(20);
    targets = bench_landscape_render
}
criterion_main!(benches);
