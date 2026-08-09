use clonk_app_render::gpu_renderer::RetainedGpuRenderer;
use clonk_graphics::{
    Color, GammaRamp, GpuBlend, GpuCommand, GpuGammaLut, GpuGammaMode, GpuPresentation, GpuSampler,
    GpuScene, GpuTextureId, GpuTextureResource, GpuVertex,
};
use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use pixels::wgpu;
use std::hint::black_box;
use std::sync::Arc;
use std::time::{Duration, Instant};

const PARTICLE_QUADS: usize = 16_384;
const FRAME_EXTENT: [u32; 2] = [800, 600];
const AMORTIZED_FRAMES: u32 = 16;

fn particle_quad(index: usize) -> [GpuVertex; 4] {
    let x = (index % 200) as f32 * 4.0;
    let y = (index / 200 % 150) as f32 * 4.0;
    let alpha = ((index % 192) + 32) as f32 / 255.0;
    let modulation = [1.0, 0.75, 0.5, alpha];
    [
        GpuVertex::new([x, y, 1.0], [0.0, 0.0], modulation),
        GpuVertex::new([x + 8.0, y, 1.0], [1.0, 0.0], modulation),
        GpuVertex::new([x, y + 8.0, 1.0], [0.0, 1.0], modulation),
        GpuVertex::new([x + 8.0, y + 8.0, 1.0], [1.0, 1.0], modulation),
    ]
}

fn particle_scene() -> GpuScene {
    let texture = GpuTextureId::fresh();
    let commands = (0..PARTICLE_QUADS)
        .map(|index| GpuCommand::Quad {
            texture,
            owner_mask: None,
            vertices: particle_quad(index),
            clip: None,
            blend: GpuBlend::Additive,
            base_mod2: false,
            owner_mod2: false,
            sampler: GpuSampler::Linear,
            gamma: false,
        })
        .collect();
    GpuScene {
        logical_extent: FRAME_EXTENT,
        clear: Color::opaque(0, 0, 0),
        gamma: GpuGammaLut::from_ramp(&GammaRamp::standard()),
        gamma_mode: GpuGammaMode::Disabled,
        textures: vec![GpuTextureResource::immutable_rgba(
            texture,
            2,
            2,
            Arc::from(vec![255_u8; 16].into_boxed_slice()),
        )],
        commands,
    }
}

fn empty_scene() -> GpuScene {
    GpuScene {
        logical_extent: FRAME_EXTENT,
        clear: Color::opaque(0, 0, 0),
        gamma: GpuGammaLut::from_ramp(&GammaRamp::standard()),
        gamma_mode: GpuGammaMode::Disabled,
        textures: Vec::new(),
        commands: Vec::new(),
    }
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

    let mut group = c.benchmark_group("particle_render");
    group.throughput(Throughput::Elements(PARTICLE_QUADS as u64));
    group.bench_function("16384_additive_quads", |b| {
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
    group.bench_function("16384_additive_quads_amortized", |b| {
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
