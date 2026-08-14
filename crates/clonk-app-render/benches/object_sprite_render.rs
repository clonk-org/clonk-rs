use clonk_app_render::gpu_renderer::{GpuRendererStats, RetainedGpuRenderer};
use clonk_graphics::{
    Color, GammaRamp, GpuBlend, GpuCommand, GpuGammaMode, GpuObjectSprite, GpuOuterModulation,
    GpuPresentation, GpuSampler, GpuScene, GpuSceneRecorder, GpuTextureId, GpuTextureResource,
    GpuVertex,
};
use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use std::hint::black_box;
use std::sync::Arc;
use std::time::{Duration, Instant};

const OBJECTS: usize = 1_000;
const PHASES: usize = 20;
const FACE_SIZE: u32 = 15;
const SHEET_WIDTH: u32 = FACE_SIZE * PHASES as u32;
const SHEET_HEIGHT: u32 = 110;
const NATIVE_TILE_SIZE: f32 = 128.0;
const FRAME_EXTENT: [u32; 2] = [800, 600];
const OBJECT_COLUMNS: usize = 40;
const ADJACENT_RESOURCE_RUNS: usize = 1;
const AMORTIZED_FRAMES: u32 = 16;

fn st5b_texture(texture: GpuTextureId) -> GpuTextureResource {
    let mut pixels = Vec::with_capacity((SHEET_WIDTH * SHEET_HEIGHT * 4) as usize);
    for y in 0..SHEET_HEIGHT {
        for x in 0..SHEET_WIDTH {
            let phase = x / FACE_SIZE;
            let local_x = x % FACE_SIZE;
            pixels.extend_from_slice(&[
                (48 + phase * 9) as u8,
                (64 + local_x * 11) as u8,
                (80 + y * 10) as u8,
                255,
            ]);
        }
    }
    GpuTextureResource::immutable_rgba(
        texture,
        SHEET_WIDTH,
        SHEET_HEIGHT,
        Arc::from(pixels.into_boxed_slice()),
    )
}

fn packed_modulation(object: usize, corner: usize) -> u32 {
    let transparency = ((object * 5 + corner * 3) % 32) as u32;
    let red = (144 + (object * 7 + corner * 11) % 112) as u32;
    let green = (128 + (object * 3 + corner * 17) % 128) as u32;
    let blue = (136 + (object * 13 + corner * 5) % 120) as u32;
    (transparency << 24) | (red << 16) | (green << 8) | blue
}

fn object_sprite(index: usize) -> GpuObjectSprite {
    let column = index % OBJECT_COLUMNS;
    let row = index / OBJECT_COLUMNS;
    let left = 4.0 + column as f32 * 18.0;
    let top = 4.0 + row as f32 * 18.0;
    let right = left + FACE_SIZE as f32;
    let bottom = top + FACE_SIZE as f32;
    let mut positions = [
        [left, top, 1.0],
        [right, top, 1.0],
        [left, bottom, 1.0],
        [right, bottom, 1.0],
    ];

    let phase = index % PHASES;
    let source_left = phase as f32 * FACE_SIZE as f32 / SHEET_WIDTH as f32;
    let source_right = (phase + 1) as f32 * FACE_SIZE as f32 / SHEET_WIDTH as f32;
    let source_top = 0.0;
    let source_bottom = FACE_SIZE as f32 / SHEET_HEIGHT as f32;
    // FlipDir is a destination transform in the object snapshot. Exercise its
    // independent vertical counterpart too while keeping the authored 15x15
    // source facet on the real 300x110 ST5B sheet.
    if !index.is_multiple_of(2) {
        positions.swap(0, 1);
        positions.swap(2, 3);
    }
    if !(index / 2).is_multiple_of(2) {
        positions.swap(0, 2);
        positions.swap(1, 3);
    }

    let sampler = if index.is_multiple_of(2) {
        GpuSampler::Nearest
    } else {
        GpuSampler::Linear
    };
    GpuObjectSprite::new(
        positions,
        [source_left, source_top, source_right, source_bottom],
        std::array::from_fn(|corner| packed_modulation(index, corner)),
        sampler,
        if sampler == GpuSampler::Linear {
            NATIVE_TILE_SIZE
        } else {
            0.0
        },
        index.is_multiple_of(7),
        GpuOuterModulation::Combine,
    )
}

fn normalized_c4(packed: u32) -> [f32; 4] {
    [
        ((packed >> 16) & 0xff) as f32 / 255.0,
        ((packed >> 8) & 0xff) as f32 / 255.0,
        (packed & 0xff) as f32 / 255.0,
        (packed >> 24) as f32 / 255.0,
    ]
}

fn generic_quad(texture: GpuTextureId, sprite: GpuObjectSprite) -> GpuCommand {
    let uv = [
        [sprite.uv[0], sprite.uv[1]],
        [sprite.uv[2], sprite.uv[1]],
        [sprite.uv[0], sprite.uv[3]],
        [sprite.uv[2], sprite.uv[3]],
    ];
    let vertices = std::array::from_fn(|index| {
        let vertex = GpuVertex::new(
            sprite.positions[index],
            uv[index],
            normalized_c4(sprite.modulation[index]),
        )
        .with_outer_modulation(GpuOuterModulation::Combine);
        if sprite.sampler() == GpuSampler::Linear {
            vertex.with_sample_tile(0.0, 0.0, sprite.sample_tile_size)
        } else {
            vertex
        }
    });
    GpuCommand::Quad {
        texture,
        owner_mask: None,
        vertices,
        clip: None,
        blend: GpuBlend::Normal,
        base_mod2: sprite.mod2(),
        owner_mod2: false,
        sampler: sprite.sampler(),
        gamma: false,
    }
}

fn scenes() -> (GpuScene, GpuScene) {
    let texture = GpuTextureId::fresh();
    let resource = st5b_texture(texture);
    let sprites = (0..OBJECTS).map(object_sprite).collect::<Vec<_>>();

    assert!(std::mem::size_of::<GpuObjectSprite>() <= 96);
    assert_eq!(sprites.len(), OBJECTS);
    assert_eq!(
        sprites
            .iter()
            .filter(|sprite| sprite.sampler() == GpuSampler::Linear)
            .count(),
        OBJECTS / 2,
    );
    assert_eq!(
        sprites
            .iter()
            .filter(|sprite| sprite.positions[0][0] > sprite.positions[1][0])
            .count(),
        OBJECTS / 2,
    );
    assert_eq!(
        sprites
            .iter()
            .filter(|sprite| sprite.positions[0][1] > sprite.positions[2][1])
            .count(),
        OBJECTS / 2,
    );
    assert_eq!(
        sprites
            .iter()
            .map(|sprite| sprite.modulation)
            .collect::<std::collections::HashSet<_>>()
            .len(),
        OBJECTS,
        "each benchmark object needs distinct corner modulation"
    );
    let mut phases_seen = [false; PHASES];
    for sprite in &sprites {
        let source_left = sprite.uv[0].min(sprite.uv[2]);
        let phase = (source_left * PHASES as f32).round() as usize;
        phases_seen[phase] = true;
    }
    assert!(phases_seen.into_iter().all(|seen| seen));

    let mut recorder = GpuSceneRecorder::default();
    recorder.add_texture(resource.clone());
    for sprite in &sprites {
        recorder.push_object_sprite(texture, *sprite, None, GpuBlend::Normal, false);
    }
    let mut compact = recorder.into_scene(
        FRAME_EXTENT,
        Color::opaque(16, 24, 32),
        &GammaRamp::identity(),
    );
    compact.gamma_mode = GpuGammaMode::Disabled;
    assert_eq!(compact.commands.len(), ADJACENT_RESOURCE_RUNS);
    let [GpuCommand::ObjectBatch {
        sprites: compact_sprites,
        ..
    }] = compact.commands.as_slice()
    else {
        panic!("one adjacent ST5B resource run did not form one object batch");
    };
    assert_eq!(compact_sprites.len(), OBJECTS);

    let mut generic = compact.clone();
    generic.commands = sprites
        .iter()
        .copied()
        .map(|sprite| generic_quad(texture, sprite))
        .collect();
    assert_eq!(generic.commands.len(), OBJECTS);
    (compact, generic)
}

fn benchmark_device() -> (tokio::runtime::Runtime, wgpu::Device, wgpu::Queue) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build object-sprite benchmark runtime");
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
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
        .expect("object-sprite benchmark requires a wgpu adapter");
    println!("object-sprite benchmark adapter: {:?}", adapter.get_info());
    let descriptor = wgpu::DeviceDescriptor {
        label: Some("lc_object_sprite_benchmark_device"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
        ..Default::default()
    };
    let (device, queue) = runtime
        .block_on(adapter.request_device(&descriptor))
        .expect("request object-sprite benchmark device");
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
        label: Some("lc_object_sprite_benchmark_encoder"),
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
        .expect("render object-sprite benchmark scene");
    let stats = renderer.last_stats();
    queue.submit([encoder.finish()]);
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("wait for object-sprite benchmark submission");
    black_box(stats)
}

fn bench_object_sprite_render(c: &mut Criterion) {
    let (compact, generic) = scenes();
    let (_runtime, device, queue) = benchmark_device();
    let presentation = GpuPresentation::identity(FRAME_EXTENT[0], FRAME_EXTENT[1]);
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("lc_object_sprite_benchmark_target"),
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
    let mut compact_renderer =
        RetainedGpuRenderer::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    let mut generic_renderer =
        RetainedGpuRenderer::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);

    let compact_stats = render_completed_frame(
        &mut compact_renderer,
        &device,
        &queue,
        &target_view,
        &compact,
        &presentation,
    );
    assert_eq!(compact_stats.draw_calls, ADJACENT_RESOURCE_RUNS);
    assert_eq!(compact_stats.total_draw_calls, ADJACENT_RESOURCE_RUNS + 1);
    assert_eq!(compact_stats.object_sprite_instances, OBJECTS);
    assert_eq!(compact_stats.quad_instances, 0);
    assert_eq!(compact_stats.quad_instance_upload_bytes, 0);
    assert_eq!(
        compact_stats.object_sprite_upload_bytes,
        OBJECTS * std::mem::size_of::<GpuObjectSprite>(),
    );
    assert!(compact_stats.object_sprite_upload_bytes <= OBJECTS * 96);

    let generic_stats = render_completed_frame(
        &mut generic_renderer,
        &device,
        &queue,
        &target_view,
        &generic,
        &presentation,
    );
    assert_eq!(generic_stats.draw_calls, OBJECTS);
    assert_eq!(generic_stats.total_draw_calls, OBJECTS + 1);
    assert_eq!(generic_stats.quad_instances, OBJECTS);
    assert_eq!(generic_stats.object_sprite_instances, 0);
    assert_eq!(generic_stats.object_sprite_upload_bytes, 0);
    assert!(generic_stats.quad_instance_upload_bytes > compact_stats.object_sprite_upload_bytes);

    println!(
        "object sprite benchmark raw stats: compact={compact_stats:?}, generic={generic_stats:?}"
    );

    let mut group = c.benchmark_group("object_sprite_render");
    group.throughput(Throughput::Elements(OBJECTS as u64));
    group.bench_function("compact_1000_st5b", |b| {
        b.iter(|| {
            render_completed_frame(
                &mut compact_renderer,
                &device,
                &queue,
                &target_view,
                &compact,
                &presentation,
            )
        });
    });
    group.bench_function("compact_1000_st5b_amortized", |b| {
        b.iter_custom(|iterations| {
            let start = Instant::now();
            for _ in 0..iterations {
                for _ in 0..AMORTIZED_FRAMES {
                    let mut encoder =
                        device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("lc_object_sprite_benchmark_encoder"),
                        });
                    compact_renderer
                        .render(
                            &device,
                            &queue,
                            &mut encoder,
                            &target_view,
                            &compact,
                            &presentation,
                            false,
                        )
                        .expect("render compact object-sprite benchmark scene");
                    queue.submit([encoder.finish()]);
                    black_box(compact_renderer.last_stats());
                }
                device
                    .poll(wgpu::PollType::wait_indefinitely())
                    .expect("wait for amortized compact object-sprite submissions");
            }
            start.elapsed() / AMORTIZED_FRAMES
        });
    });
    group.bench_function("generic_1000_st5b", |b| {
        b.iter(|| {
            render_completed_frame(
                &mut generic_renderer,
                &device,
                &queue,
                &target_view,
                &generic,
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
    targets = bench_object_sprite_render
}
criterion_main!(benches);
