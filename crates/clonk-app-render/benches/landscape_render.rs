use clonk_app_render::gpu_renderer::{GpuRendererStats, RetainedGpuRenderer};
use clonk_graphics::{
    Color, GammaRamp, GpuCommand, GpuGammaLut, GpuGammaMode, GpuPresentation, GpuScene,
    GpuTextureId, GpuTextureResource, GpuVertex,
};
use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use pixels::wgpu;
use std::hint::black_box;
use std::sync::Arc;
use std::time::Duration;

const FRAME_EXTENT: [u32; 2] = [3_840, 2_160];
const FOG_CHUNK_SIZE: u32 = 64;
const FOG_COLUMNS: usize = 60;
const FOG_ROWS: usize = 34;
const FOG_CHUNKS: usize = FOG_COLUMNS * FOG_ROWS;
const NO_BOX_FADES_COMMANDS: usize = FOG_CHUNKS * 2;
const BASE_EXTENT: [u32; 2] = [4_096, 4_096];

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

fn landscape_scene(base: GpuTextureId, pixels: Arc<[u8]>, no_box_fades: bool) -> GpuScene {
    let mut commands = Vec::with_capacity(if no_box_fades {
        NO_BOX_FADES_COMMANDS
    } else {
        FOG_CHUNKS
    });

    // LegacyClonk 7d43b47b7d789b533f32d005e64596e0a07019cd lowers a fogged
    // landscape blit to source-aligned chunks no larger than 64 pixels and
    // uses each strip triangle's provoking colour for NoBoxFades
    // (src/StdGL.cpp:667,710-763).
    for top in (0..FRAME_EXTENT[1]).step_by(FOG_CHUNK_SIZE as usize) {
        let bottom = top.saturating_add(FOG_CHUNK_SIZE).min(FRAME_EXTENT[1]);
        for left in (0..FRAME_EXTENT[0]).step_by(FOG_CHUNK_SIZE as usize) {
            let right = left.saturating_add(FOG_CHUNK_SIZE).min(FRAME_EXTENT[0]);
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

    let expected_commands = if no_box_fades {
        NO_BOX_FADES_COMMANDS
    } else {
        FOG_CHUNKS
    };
    assert_eq!(commands.len(), expected_commands);
    GpuScene {
        logical_extent: FRAME_EXTENT,
        clear: Color::opaque(8, 12, 24),
        gamma: GpuGammaLut::from_ramp(&GammaRamp::identity()),
        gamma_mode: GpuGammaMode::Disabled,
        textures: vec![base_texture(base, pixels)],
        commands,
    }
}

fn benchmark_device() -> (tokio::runtime::Runtime, wgpu::Device, wgpu::Queue) {
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
    let descriptor = wgpu::DeviceDescriptor {
        label: Some("lc_landscape_benchmark_device"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
        ..Default::default()
    };
    let (device, queue) = runtime
        .block_on(adapter.request_device(&descriptor))
        .expect("request landscape benchmark device");
    (runtime, device, queue)
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

fn assert_coalesced(stats: GpuRendererStats) {
    assert_eq!(stats.draw_calls, 1, "compatible chunks need one scene draw");
    assert_eq!(
        stats.total_draw_calls, 2,
        "the fixed final presentation is the only additional draw"
    );
}

fn bench_landscape_render(c: &mut Criterion) {
    let base = GpuTextureId::fresh();
    let pixels = base_pixels();
    let normal = landscape_scene(base, pixels.clone(), false);
    let no_box_fades = landscape_scene(base, pixels, true);
    let (_runtime, device, queue) = benchmark_device();
    let presentation = GpuPresentation::identity(FRAME_EXTENT[0], FRAME_EXTENT[1]);
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("lc_landscape_benchmark_target"),
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
    let mut normal_renderer =
        RetainedGpuRenderer::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    let mut no_box_fades_renderer =
        RetainedGpuRenderer::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);

    let normal_stats = render_completed_frame(
        &mut normal_renderer,
        &device,
        &queue,
        &target_view,
        &normal,
        &presentation,
    );
    let no_box_fades_stats = render_completed_frame(
        &mut no_box_fades_renderer,
        &device,
        &queue,
        &target_view,
        &no_box_fades,
        &presentation,
    );
    assert_coalesced(normal_stats);
    assert_coalesced(no_box_fades_stats);
    println!(
        "landscape renderer raw stats: extent={}x{}, fog_chunks={FOG_CHUNKS}, \
         normal_commands={}, no_box_fades_commands={}, normal={normal_stats:?}, \
         no_box_fades={no_box_fades_stats:?}, \
         timing_scope=prebuilt_scene_renderer_encode_submit_and_device_poll_wall_time",
        FRAME_EXTENT[0],
        FRAME_EXTENT[1],
        normal.commands.len(),
        no_box_fades.commands.len(),
    );

    let mut group = c.benchmark_group("landscape_render");
    group.throughput(Throughput::Elements(FOG_CHUNKS as u64));
    group.bench_function("fogged_4k_2040_chunks", |b| {
        b.iter(|| {
            render_completed_frame(
                &mut normal_renderer,
                &device,
                &queue,
                &target_view,
                &normal,
                &presentation,
            )
        });
    });
    group.throughput(Throughput::Elements(NO_BOX_FADES_COMMANDS as u64));
    group.bench_function("no_box_fades_4k_4080_commands", |b| {
        b.iter(|| {
            render_completed_frame(
                &mut no_box_fades_renderer,
                &device,
                &queue,
                &target_view,
                &no_box_fades,
                &presentation,
            )
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
