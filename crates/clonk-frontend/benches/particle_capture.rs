use clonk_engine::{
    particles::{ParticleDefCore, ParticleDrawProc},
    FloatVector2, ParticleLayer, ParticleSnapshot,
};
use clonk_frontend::{
    CursorAtlas, DefinitionSprite, GraphicsSystem, HudGraphics, ImageData, ParticleFacet,
    ParticleRenderDefinition,
};
use clonk_graphics::{BitmapFont, GammaRamp, GpuBlend, GpuCommand};
use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use std::collections::HashMap;
use std::hint::black_box;
use std::sync::Arc;
use std::time::Duration;

const FIRE_PARTICLES: usize = 1_000;
const FIRE2_PARTICLES: usize = 1_000;
const PARTICLES: usize = FIRE_PARTICLES + FIRE2_PARTICLES;
const EXTENT: [u32; 2] = [800, 600];

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

fn fixture() -> (GraphicsSystem, Vec<ParticleSnapshot>, GammaRamp) {
    let mut graphics = GraphicsSystem::new(
        EXTENT[0],
        EXTENT[1],
        EXTENT[1] as i32,
        "particle capture benchmark",
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

fn bench_particle_capture(c: &mut Criterion) {
    let (mut graphics, particles, gamma) = fixture();
    let scene = graphics.capture_global_definition_particles_for_benchmark(&particles, &gamma);
    let instances = |command: &GpuCommand| match command {
        GpuCommand::Quad { .. } => 1,
        GpuCommand::SpriteBatch { quads, .. } => quads.len(),
        _ => 0,
    };
    let blended_instances = |blend| {
        scene
            .commands
            .iter()
            .filter(|command| match command {
                GpuCommand::Quad {
                    blend: command_blend,
                    ..
                }
                | GpuCommand::SpriteBatch {
                    blend: command_blend,
                    ..
                } => *command_blend == blend,
                _ => false,
            })
            .map(instances)
            .sum::<usize>()
    };
    assert_eq!(scene.commands.len(), 2);
    assert_eq!(
        scene.commands.iter().map(instances).sum::<usize>(),
        PARTICLES
    );
    assert_eq!(scene.textures.len(), 2);
    assert_eq!(blended_instances(GpuBlend::Normal), FIRE_PARTICLES);
    assert_eq!(blended_instances(GpuBlend::Additive), FIRE2_PARTICLES);

    let mut group = c.benchmark_group("particle_capture");
    group.throughput(Throughput::Elements(PARTICLES as u64));
    group.bench_function("2000_fire_and_fire2", |b| {
        b.iter(|| {
            let scene =
                graphics.capture_global_definition_particles_for_benchmark(&particles, &gamma);
            black_box(scene);
        });
    });
    group.finish();

    c.bench_function("particle_capture_empty", |b| {
        b.iter(|| {
            let scene = graphics.capture_global_definition_particles_for_benchmark(&[], &gamma);
            black_box(scene);
        });
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .warm_up_time(Duration::from_secs(2))
        .measurement_time(Duration::from_secs(5))
        .sample_size(20);
    targets = bench_particle_capture
}
criterion_main!(benches);
