use clonk_app_render::gpu_renderer::RetainedGpuRenderer;
use clonk_engine::{FloatVector2, ParticleLayer, ParticleSnapshot};
use clonk_frontend::{
    CursorAtlas, DefinitionSprite, GraphicsSystem, HudGraphics, MaterialRenderInfo,
};
use clonk_graphics::{BitmapFont, GammaRamp, GpuCommand, GpuPresentation, GpuScene};
use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::HashMap;
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const PARTICLES: usize = 2_000;
const FRAME_EXTENT: [u32; 2] = [800, 600];
const ONE: i32 = 1 << 16;
/// Every covered physical pixel must fit this budget (clonk-org/clonk-rs#270).
const FRAGMENT_BYTE_BUDGET: usize = 40;

static COUNT_ALLOCATIONS: AtomicBool = AtomicBool::new(false);
static ALLOCATION_CALLS: AtomicUsize = AtomicUsize::new(0);
static ALLOCATION_BYTES: AtomicUsize = AtomicUsize::new(0);

struct CountingAllocator;

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNT_ALLOCATIONS.load(Ordering::Relaxed) {
            ALLOCATION_CALLS.fetch_add(1, Ordering::Relaxed);
            ALLOCATION_BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        }
        // SAFETY: this wrapper forwards the caller's allocation contract.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if COUNT_ALLOCATIONS.load(Ordering::Relaxed) {
            ALLOCATION_CALLS.fetch_add(1, Ordering::Relaxed);
            ALLOCATION_BYTES.fetch_add(layout.size(), Ordering::Relaxed);
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
            ALLOCATION_BYTES.fetch_add(new_size, Ordering::Relaxed);
        }
        // SAFETY: this wrapper forwards the caller's reallocation contract.
        unsafe { System.realloc(pointer, layout, new_size) }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AllocationStats {
    calls: usize,
    bytes: usize,
}

fn measure_allocations<T>(capture: impl FnOnce() -> T) -> (T, AllocationStats) {
    ALLOCATION_CALLS.store(0, Ordering::Relaxed);
    ALLOCATION_BYTES.store(0, Ordering::Relaxed);
    COUNT_ALLOCATIONS.store(true, Ordering::SeqCst);
    let result = capture();
    COUNT_ALLOCATIONS.store(false, Ordering::SeqCst);
    (
        result,
        AllocationStats {
            calls: ALLOCATION_CALLS.load(Ordering::Relaxed),
            bytes: ALLOCATION_BYTES.load(Ordering::Relaxed),
        },
    )
}

fn pxs(fixed: [i32; 4], slot: u32) -> ParticleSnapshot {
    ParticleSnapshot {
        definition_id: "material/pxs/rain".to_owned(),
        position: FloatVector2::new(fixed[0] as f32 / 65_536.0, fixed[1] as f32 / 65_536.0),
        velocity: FloatVector2::new(fixed[2] as f32 / 65_536.0, fixed[3] as f32 / 65_536.0),
        life: 0,
        parameter_a: 0.0,
        parameter_b: 0,
        layer: ParticleLayer::Global,
        pxs_fixed: Some(fixed),
        pxs_slot: Some(slot),
    }
}

/// A deterministic spread over every old-style PXS raster case.
///
/// One index maps to one case, so the mix is fixed across runs and machines:
/// stationary points, axis-aligned and diagonal velocity lines travelled in
/// both directions, a sub-pixel velocity whose endpoints land on one pixel,
/// and particles straddling the viewport edge so the renderer has to clip.
fn particles() -> Vec<ParticleSnapshot> {
    (0..PARTICLES)
        .map(|index| {
            let column = (index % 40) as i32;
            let row = (index / 40) as i32;
            let x = 24 + column * 19;
            let y = 24 + row * 11;
            let velocity = match index % 8 {
                // Stationary: fixtoi of both components is zero, so C4PXS draws
                // a point rather than a velocity line.
                0 => [0, 0],
                1 => [3 * ONE, 0],
                2 => [-3 * ONE, 0],
                3 => [0, 4 * ONE],
                4 => [0, -4 * ONE],
                5 => [3 * ONE, 2 * ONE],
                6 => [-3 * ONE, 2 * ONE],
                // Sub-pixel in the minor axis: still a moving PXS, but a very
                // short segment whose ends can select the same pixel.
                _ => [ONE, ONE / 8],
            };
            let (x, y) = match index % 37 {
                // Straddle each viewport edge so the fragment walk has to clip.
                0 => (2, y),
                11 => (FRAME_EXTENT[0] as i32 - 2, y),
                23 => (x, 2),
                31 => (x, FRAME_EXTENT[1] as i32 - 2),
                _ => (x, y),
            };
            pxs(
                [x * ONE, y * ONE, velocity[0], velocity[1]],
                index as u32 % 500,
            )
        })
        .collect()
}

/// A uniform moving burst: every particle takes the velocity-line path, so the
/// whole burst stays one retained run.
fn storm_particles(count: usize) -> Vec<ParticleSnapshot> {
    (0..count)
        .map(|index| {
            let column = (index % 40) as i32;
            let row = (index / 40) as i32;
            pxs(
                [
                    (24 + column * 19) * ONE,
                    (24 + row * 11) * ONE,
                    2 * ONE,
                    3 * ONE,
                ],
                index as u32 % 500,
            )
        })
        .collect()
}

fn graphics() -> GraphicsSystem {
    let mut graphics = GraphicsSystem::new(
        FRAME_EXTENT[0],
        FRAME_EXTENT[1],
        FRAME_EXTENT[1] as i32,
        "pxs render benchmark",
        Arc::new(BitmapFont::new()),
        Arc::new(HashMap::<String, DefinitionSprite>::new()),
        Arc::new(CursorAtlas::empty()),
        Arc::new(HudGraphics::default()),
    );
    graphics.set_material_render_info(Arc::new(HashMap::from([(
        "rain".to_owned(),
        MaterialRenderInfo::new([200, 100, 50, 0, 0, 0, 0, 0, 0], [0; 6], None, 0, 25),
    )])));
    graphics
}

fn covered_fragments(scene: &GpuScene) -> usize {
    scene
        .commands
        .iter()
        .map(|command| match command {
            GpuCommand::Solid { vertices, .. } => vertices.len(),
            _ => 0,
        })
        .sum()
}

/// Capture `particles` on a system already warmed on that same workload.
///
/// A capture reuses the previous frame's run lengths, so the steady state is
/// what a running game pays. Each workload therefore gets its own system: the
/// question is whether a big frame costs more allocations than a small one,
/// not whether a small frame can ride a big frame's reservations.
fn steady_state_capture(particles: &[ParticleSnapshot]) -> (GpuScene, AllocationStats) {
    let gamma = GammaRamp::identity();
    let mut graphics = graphics();
    for _ in 0..3 {
        black_box(graphics.capture_old_style_pxs_for_benchmark(particles, &gamma));
    }
    measure_allocations(|| graphics.capture_old_style_pxs_for_benchmark(particles, &gamma))
}

/// One retained run, whatever the particle count: a rain burst where every
/// particle moves. This is the shape the acceptance criterion is about, and it
/// must not allocate more often for more particles.
fn assert_capture_does_not_scale_with_particle_count() {
    let (one_scene, one) = steady_state_capture(&storm_particles(1));
    let (many_scene, many) = steady_state_capture(&storm_particles(PARTICLES));

    assert_eq!(one_scene.commands.len(), 1);
    assert_eq!(many_scene.commands.len(), 1);
    assert_eq!(
        many.calls, one.calls,
        "capture allocations scaled with the number of PXS"
    );
    println!("pxs capture allocations: 1={one:?}, {PARTICLES}={many:?}");
}

fn benchmark_device() -> (tokio::runtime::Runtime, wgpu::Device, wgpu::Queue) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build benchmark runtime");
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
        .expect("pxs benchmark requires a wgpu adapter");
    let descriptor = wgpu::DeviceDescriptor {
        label: Some("lc_pxs_benchmark_device"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
        ..Default::default()
    };
    let (device, queue) = runtime
        .block_on(adapter.request_device(&descriptor))
        .expect("request pxs benchmark device");
    (runtime, device, queue)
}

fn render_completed_frame(
    renderer: &mut RetainedGpuRenderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target_view: &wgpu::TextureView,
    scene: &GpuScene,
    presentation: &GpuPresentation,
) {
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("lc_pxs_benchmark_encoder"),
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
        .expect("render pxs benchmark scene");
    queue.submit([encoder.finish()]);
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("wait for pxs benchmark submission");
}

fn percentile(sorted: &[Duration], fraction: f64) -> Duration {
    let last = sorted.len() - 1;
    sorted[((last as f64) * fraction).round() as usize]
}

fn report_presentation_cost(
    label: &str,
    renderer: &mut RetainedGpuRenderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target_view: &wgpu::TextureView,
    scene: &GpuScene,
    presentation: &GpuPresentation,
) {
    let mut samples = Vec::with_capacity(120);
    for _ in 0..120 {
        let start = Instant::now();
        render_completed_frame(renderer, device, queue, target_view, scene, presentation);
        samples.push(start.elapsed());
    }
    samples.sort_unstable();
    let stats = renderer.last_stats();
    let per_fragment = stats
        .solid_rect_upload_bytes
        .checked_div(stats.solid_rect_instances)
        .unwrap_or(0);
    assert!(
        per_fragment <= FRAGMENT_BYTE_BUDGET,
        "{label}: a covered pixel cost {per_fragment} bytes"
    );
    println!(
        "pxs presentation cost [{label}]: p50={:?}, p95={:?}, p99={:?}, instances={}, \
         upload_bytes={}, bytes_per_fragment={per_fragment}, draw_calls={}",
        percentile(&samples, 0.50),
        percentile(&samples, 0.95),
        percentile(&samples, 0.99),
        stats.solid_rect_instances,
        stats.solid_rect_upload_bytes,
        stats.draw_calls,
    );
}

fn bench_pxs_render(c: &mut Criterion) {
    assert_capture_does_not_scale_with_particle_count();

    let particles = particles();
    let (scene, raster_case_stats) = steady_state_capture(&particles);
    println!(
        "pxs raster-case capture: runs={}, {raster_case_stats:?}",
        scene.commands.len()
    );
    let endpoints = covered_fragments(&scene);
    assert!(
        endpoints > PARTICLES,
        "fixture lost its velocity lines: {endpoints} captured vertices"
    );

    let (_runtime, device, queue) = benchmark_device();
    let mut renderer = RetainedGpuRenderer::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("lc_pxs_benchmark_target"),
        size: wgpu::Extent3d {
            width: FRAME_EXTENT[0],
            height: FRAME_EXTENT[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let presentation = GpuPresentation::identity(FRAME_EXTENT[0], FRAME_EXTENT[1]);

    render_completed_frame(
        &mut renderer,
        &device,
        &queue,
        &target_view,
        &scene,
        &presentation,
    );
    let stats = renderer.last_stats();
    assert!(stats.solid_rect_instances >= PARTICLES);
    assert_eq!(
        stats.quad_instance_upload_bytes, 0,
        "old-style PXS entered the generic quad path"
    );

    // Presentation scale changes the physical line width and the point
    // footprint, so every scale gets its own recorded cost.
    for scale in [1.0_f32, 1.5, 2.0] {
        let scaled = GpuPresentation {
            physical_extent: [
                (FRAME_EXTENT[0] as f32 * scale).ceil() as u32,
                (FRAME_EXTENT[1] as f32 * scale).ceil() as u32,
            ],
            scale,
            crop_top: 0,
        };
        let scaled_target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("lc_pxs_benchmark_scaled_target"),
            size: wgpu::Extent3d {
                width: scaled.physical_extent[0],
                height: scaled.physical_extent[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let scaled_view = scaled_target.create_view(&wgpu::TextureViewDescriptor::default());
        report_presentation_cost(
            &format!("scale_{scale}"),
            &mut renderer,
            &device,
            &queue,
            &scaled_view,
            &scene,
            &scaled,
        );
    }

    let mut group = c.benchmark_group("pxs_render");
    group.throughput(Throughput::Elements(PARTICLES as u64));
    group.bench_function("2000_old_style_pxs", |b| {
        b.iter(|| {
            render_completed_frame(
                &mut renderer,
                &device,
                &queue,
                &target_view,
                &scene,
                &presentation,
            );
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
    targets = bench_pxs_render
}
criterion_main!(benches);
