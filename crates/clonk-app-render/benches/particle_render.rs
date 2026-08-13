use clonk_app_render::gpu_renderer::RetainedGpuRenderer;
use clonk_engine::{
    particles::{ParticleDefCore, ParticleDrawProc},
    FloatVector2, ParticleLayer, ParticleSnapshot,
};
use clonk_frontend::{
    CursorAtlas, DefinitionSprite, GraphicsSystem, HudGraphics, ImageData, ParticleFacet,
    ParticleRenderDefinition,
};
use clonk_graphics::{BitmapFont, GammaRamp, GpuPresentation, GpuScene};
use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use std::collections::HashMap;
use std::hint::black_box;
use std::sync::Arc;
use std::time::{Duration, Instant};

const FIRE_PARTICLES: usize = 1_000;
const FIRE2_PARTICLES: usize = 1_000;
const PARTICLE_QUADS: usize = FIRE_PARTICLES + FIRE2_PARTICLES;
const FRAME_EXTENT: [u32; 2] = [800, 600];
const AMORTIZED_FRAMES: u32 = 16;

fn definition(
    name: &str,
    image: ImageData,
    facet: ParticleFacet,
    length: i32,
    additive: bool,
) -> ParticleRenderDefinition {
    ParticleRenderDefinition {
        image,
        facet,
        length,
        aspect: 1.0,
        core: ParticleDefCore {
            name: name.to_owned(),
            additive: i32::from(additive),
            attach: 1,
            ..ParticleDefCore::default()
        },
        draw_proc: ParticleDrawProc::Std,
    }
}

fn particle(definition_id: &str, index: usize, life: i32) -> ParticleSnapshot {
    let column = index % 50;
    let row = index / 50;
    ParticleSnapshot {
        definition_id: definition_id.to_owned(),
        position: FloatVector2::new(10.0 + column as f32 * 15.0, 10.0 + row as f32 * 14.0),
        velocity: FloatVector2::new(0.0, 0.0),
        life,
        parameter_a: 8.0,
        parameter_b: 0x00ff_ffff,
        layer: ParticleLayer::Global,
        pxs_fixed: None,
        pxs_slot: None,
    }
}

fn capture_fixture() -> (GraphicsSystem, Vec<ParticleSnapshot>, GammaRamp) {
    let mut graphics = GraphicsSystem::new(
        FRAME_EXTENT[0],
        FRAME_EXTENT[1],
        FRAME_EXTENT[1] as i32,
        "particle render benchmark",
        Arc::new(BitmapFont::new()),
        Arc::new(HashMap::<String, DefinitionSprite>::new()),
        Arc::new(CursorAtlas::empty()),
        Arc::new(HudGraphics::default()),
    );
    graphics.set_particle_sprites(Arc::new(HashMap::from([
        (
            "Fire".to_owned(),
            definition(
                "Fire",
                ImageData::new(26, 26, vec![255; 26 * 26 * 4]),
                ParticleFacet::new(0, 0, 26, 26),
                1,
                false,
            ),
        ),
        (
            "Fire2".to_owned(),
            definition(
                "Fire2",
                ImageData::new(256, 32, vec![255; 256 * 32 * 4]),
                ParticleFacet::new(0, 0, 32, 32),
                8,
                true,
            ),
        ),
    ])));
    let particles = (0..FIRE_PARTICLES)
        .map(|index| particle("Fire", index, 0))
        .chain((0..FIRE2_PARTICLES).map(|index| particle("Fire2", index, index as i32 % 8)))
        .collect();
    (graphics, particles, GammaRamp::identity())
}

fn particle_scene() -> GpuScene {
    let (mut graphics, particles, gamma) = capture_fixture();
    graphics.capture_global_definition_particles_for_benchmark(&particles, &gamma)
}

fn empty_scene() -> GpuScene {
    let (mut graphics, _, gamma) = capture_fixture();
    graphics.capture_global_definition_particles_for_benchmark(&[], &gamma)
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
        .expect("particle benchmark requires a wgpu adapter");
    let descriptor = wgpu::DeviceDescriptor {
        label: Some("lc_particle_benchmark_device"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
        ..Default::default()
    };
    let (device, queue) = runtime
        .block_on(adapter.request_device(&descriptor))
        .expect("request particle benchmark device");
    (runtime, device, queue)
}

fn submit_frame(
    renderer: &mut RetainedGpuRenderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target_view: &wgpu::TextureView,
    scene: &GpuScene,
    presentation: &GpuPresentation,
) {
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("lc_particle_benchmark_encoder"),
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
        .expect("render particle benchmark scene");
    queue.submit([encoder.finish()]);
    black_box(renderer.last_stats());
}

fn render_completed_frame(
    renderer: &mut RetainedGpuRenderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target_view: &wgpu::TextureView,
    scene: &GpuScene,
    presentation: &GpuPresentation,
) {
    submit_frame(renderer, device, queue, target_view, scene, presentation);
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("wait for particle benchmark submission");
}

fn bench_particle_render(c: &mut Criterion) {
    let (_runtime, device, queue) = benchmark_device();
    let mut renderer = RetainedGpuRenderer::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    let scene = particle_scene();
    let presentation = GpuPresentation::identity(FRAME_EXTENT[0], FRAME_EXTENT[1]);
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("lc_particle_benchmark_target"),
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

    render_completed_frame(
        &mut renderer,
        &device,
        &queue,
        &target_view,
        &scene,
        &presentation,
    );
    assert_eq!(renderer.last_stats().draw_calls, 2);

    let mut group = c.benchmark_group("particle_render");
    group.throughput(Throughput::Elements(PARTICLE_QUADS as u64));
    group.bench_function("2000_fire_and_fire2", |b| {
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
    group.bench_function("2000_fire_and_fire2_amortized", |b| {
        b.iter_custom(|iterations| {
            let start = Instant::now();
            for _ in 0..iterations {
                for _ in 0..AMORTIZED_FRAMES {
                    submit_frame(
                        &mut renderer,
                        &device,
                        &queue,
                        &target_view,
                        &scene,
                        &presentation,
                    );
                }
                device
                    .poll(wgpu::PollType::wait_indefinitely())
                    .expect("wait for amortized particle submissions");
            }
            start.elapsed() / AMORTIZED_FRAMES
        });
    });
    group.finish();

    let empty = empty_scene();
    c.bench_function("particle_render_empty_frame", |b| {
        b.iter(|| {
            render_completed_frame(
                &mut renderer,
                &device,
                &queue,
                &target_view,
                &empty,
                &presentation,
            );
        });
    });
    c.bench_function("particle_render_empty_frame_amortized", |b| {
        b.iter_custom(|iterations| {
            let start = Instant::now();
            for _ in 0..iterations {
                for _ in 0..AMORTIZED_FRAMES {
                    submit_frame(
                        &mut renderer,
                        &device,
                        &queue,
                        &target_view,
                        &empty,
                        &presentation,
                    );
                }
                device
                    .poll(wgpu::PollType::wait_indefinitely())
                    .expect("wait for amortized empty submissions");
            }
            start.elapsed() / AMORTIZED_FRAMES
        });
    });

    let (mut integrated_graphics, integrated_particles, integrated_gamma) = capture_fixture();
    let mut pipeline_group = c.benchmark_group("particle_pipeline");
    pipeline_group.throughput(Throughput::Elements(PARTICLE_QUADS as u64));
    pipeline_group.bench_function("2000_fire_and_fire2_amortized", |b| {
        b.iter_custom(|iterations| {
            let start = Instant::now();
            for _ in 0..iterations {
                for _ in 0..AMORTIZED_FRAMES {
                    let scene = integrated_graphics
                        .capture_global_definition_particles_for_benchmark(
                            &integrated_particles,
                            &integrated_gamma,
                        );
                    submit_frame(
                        &mut renderer,
                        &device,
                        &queue,
                        &target_view,
                        &scene,
                        &presentation,
                    );
                }
                device
                    .poll(wgpu::PollType::wait_indefinitely())
                    .expect("wait for amortized particle-pipeline submissions");
            }
            start.elapsed() / AMORTIZED_FRAMES
        });
    });
    pipeline_group.bench_function("empty_amortized", |b| {
        b.iter_custom(|iterations| {
            let start = Instant::now();
            for _ in 0..iterations {
                for _ in 0..AMORTIZED_FRAMES {
                    let scene = integrated_graphics
                        .capture_global_definition_particles_for_benchmark(&[], &integrated_gamma);
                    submit_frame(
                        &mut renderer,
                        &device,
                        &queue,
                        &target_view,
                        &scene,
                        &presentation,
                    );
                }
                device
                    .poll(wgpu::PollType::wait_indefinitely())
                    .expect("wait for amortized empty-pipeline submissions");
            }
            start.elapsed() / AMORTIZED_FRAMES
        });
    });
    pipeline_group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .warm_up_time(Duration::from_secs(2))
        .measurement_time(Duration::from_secs(5))
        .sample_size(20);
    targets = bench_particle_render
}
criterion_main!(benches);
