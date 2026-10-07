use super::*;

#[test]
fn retained_cpu_translated_velocity_lines_clip_before_rasterizing() {
    // StdGL.cpp:893-933 submits the viewport-local velocity line; its local
    // target clips the line before selecting the half-open raster pixels.
    let mut oracle = Surface::new(64, 64, PixelFormat::Rgba8888);
    oracle.fill(Color::opaque(3, 3, 4));
    let mut immediate = Surface::new(64, 10, PixelFormat::Rgba8888);
    immediate.fill(Color::opaque(3, 3, 4));
    let draw = |surface: &mut Surface| {
        draw_primitives::draw_pxs_line(
            surface,
            (7.581665, -3.5301514),
            (7.39624, 9.577942),
            Color::new(0, 0, 3, 75),
            None,
            None,
        )
    };
    draw(&mut immediate);
    for y in 0..10 {
        for x in 0..64 {
            oracle
                .set_pixel(x, y + 50, immediate.get_pixel(x, y).unwrap())
                .unwrap();
        }
    }
    let mut child = Surface::new(64, 10, PixelFormat::Rgba8888);
    child.begin_gpu_scene_capture();
    draw(&mut child);
    let mut recorder = clonk_graphics::GpuSceneRecorder::default();
    recorder.append_translated(
        child.take_gpu_scene_capture().unwrap(),
        0,
        50,
        clonk_graphics::Rect::new(0, 0, 64, 10),
        None,
    );
    let scene = recorder.into_scene(
        [64, 64],
        Color::opaque(3, 3, 4),
        &clonk_graphics::GammaRamp::identity(),
    );
    let mut actual = vec![0; 64 * 64 * 4];
    clonk_graphics::CpuSceneRenderer::default()
        .render(&scene, &mut actual)
        .unwrap();
    let mismatch = actual
        .chunks_exact(4)
        .zip(oracle.pixels().chunks_exact(4))
        .enumerate()
        .find(|(_, (a, b))| a != b)
        .map(|(i, (a, b))| (i % 64, i / 64, a, b));
    assert!(mismatch.is_none(), "{mismatch:?}");
}

#[test]
fn retained_cpu_texture_indent_preserves_native_sampling() {
    // StdGL.cpp:471-527 restarts TexIndent inside each physical texture tile.
    let image = ImageData::new(5, 3, (0..60).map(|i| ((i * 73 + 17) % 256) as u8).collect());
    let config = AdvancedRendererConfig {
        tex_indent: 400,
        ..AdvancedRendererConfig::DEFAULT
    };
    for sampling in [BlitSampling::Nearest, BlitSampling::Linear] {
        let draw = |surface: &mut Surface| {
            surface.fill(Color::new(13, 27, 39, 67));
            draw_image_region_float_source(
                surface,
                &GuiRect::new(1.25, 1.75, 5.5, 4.5),
                &image,
                None,
                &FloatSourceRect {
                    x: 0.25,
                    y: 0.5,
                    width: 4.5,
                    height: 2.0,
                },
                sampling,
                false,
                None,
                SpriteBlitState {
                    renderer_config: config,
                    ..SpriteBlitState::normal()
                },
                None,
                None,
            );
        };
        let mut oracle = Surface::new(9, 8, PixelFormat::Rgba8888);
        draw(&mut oracle);
        let mut retained = Surface::new(9, 8, PixelFormat::Rgba8888);
        retained.begin_gpu_scene_capture();
        draw(&mut retained);
        let scene = retained.take_gpu_scene_capture().unwrap().into_scene(
            [9, 8],
            Color::transparent(),
            &clonk_graphics::GammaRamp::identity(),
        );
        let mut actual = vec![0; 9 * 8 * 4];
        clonk_graphics::CpuSceneRenderer::default()
            .render(&scene, &mut actual)
            .unwrap();
        assert_eq!(actual, oracle.pixels(), "sampling={sampling:?}");
    }
}

#[test]
fn retained_cpu_nearest_gui_keeps_compatibility_and_configured_sampling() {
    let image = ImageData::new(
        3,
        1,
        vec![10, 20, 30, 255, 40, 50, 60, 255, 70, 80, 90, 255],
    );
    // The compatibility GUI loop samples integer destination offsets.
    // Configured native blits project pixel centers (StdDDraw2.cpp:738-741).
    for (config, explicit_configured) in [
        (None, false),
        (Some(AdvancedRendererConfig::DEFAULT), false),
        (Some(AdvancedRendererConfig::DEFAULT), true),
        (
            Some(AdvancedRendererConfig {
                blit_offset: 25,
                ..AdvancedRendererConfig::DEFAULT
            }),
            false,
        ),
    ] {
        let draw = |surface: &mut Surface| {
            let _config = config.map(activate_advanced_renderer_config);
            surface.fill(Color::opaque(0, 0, 0));
            let destination = GuiRect::new(0.0, 0.0, 2.0, 1.0);
            if explicit_configured {
                draw_image_source_configured_on_surface(
                    surface,
                    &destination,
                    &image,
                    FloatSourceRect {
                        x: 0.0,
                        y: 0.0,
                        width: 3.0,
                        height: 1.0,
                    },
                    BlitSampling::Nearest,
                    None,
                    BilinearBlend::AlphaOver,
                    None,
                    config.unwrap_or_default(),
                );
            } else {
                draw_image(surface, &destination, &image);
            }
        };
        let mut oracle = Surface::new(2, 1, PixelFormat::Rgba8888);
        draw(&mut oracle);
        let mut retained = Surface::new(2, 1, PixelFormat::Rgba8888);
        retained.begin_gpu_scene_capture();
        draw(&mut retained);
        let scene = retained.take_gpu_scene_capture().unwrap().into_scene(
            [2, 1],
            Color::transparent(),
            &clonk_graphics::GammaRamp::identity(),
        );
        let mut actual = [0; 8];
        clonk_graphics::CpuSceneRenderer::default()
            .render(&scene, &mut actual)
            .unwrap();
        assert_eq!(
            actual,
            oracle.pixels(),
            "config={config:?}, explicit={explicit_configured}"
        );
    }
}

#[test]
fn retained_cpu_sky_rounds_daylight_texels_before_blending() {
    let image = ImageData::new(1, 1, vec![2, 2, 2, 255]);
    let make = || {
        GraphicsSystem::new(
            1,
            1,
            1,
            "CPU daylight differential",
            Arc::new(clonk_graphics::BitmapFont::new()),
            Arc::new(HashMap::new()),
            Arc::new(CursorAtlas::empty()),
            Arc::new(HudGraphics::default()),
        )
    };
    let mut oracle = make();
    oracle.draw_sky_tile_positions_with_parallel_rows(&image, &[(0, 0)], None, 0.75, None, false);
    let mut retained = make();
    retained.begin_gpu_scene_capture();
    retained.draw_sky_tile_positions_with_parallel_rows(&image, &[(0, 0)], None, 0.75, None, false);
    let scene = retained
        .finish_gpu_scene_capture(&clonk_graphics::GammaRamp::identity())
        .unwrap();
    let mut actual = [0; 4];
    clonk_graphics::CpuSceneRenderer::default()
        .render(&scene, &mut actual)
        .unwrap();
    assert_eq!(actual, oracle.surface().pixels());
}

#[test]
fn retained_cpu_scalar_sky_copies_integer_source_rows() {
    let image = ImageData::new(
        1,
        648,
        (0..648)
            .flat_map(|row| [(row % 251) as u8, (row % 239) as u8, (row % 227) as u8, 255])
            .collect(),
    );
    let make = || {
        GraphicsSystem::new(
            1,
            648,
            1,
            "CPU sky differential",
            Arc::new(clonk_graphics::BitmapFont::new()),
            Arc::new(HashMap::new()),
            Arc::new(CursorAtlas::empty()),
            Arc::new(HudGraphics::default()),
        )
    };
    let mut oracle = make();
    oracle.draw_sky_tile_positions_with_parallel_rows(&image, &[(0, 0)], None, 1.0, None, false);
    let mut retained = make();
    retained.begin_gpu_scene_capture();
    retained.draw_sky_tile_positions_with_parallel_rows(&image, &[(0, 0)], None, 1.0, None, false);
    let scene = retained
        .finish_gpu_scene_capture(&clonk_graphics::GammaRamp::identity())
        .unwrap();
    let mut actual = vec![0; 648 * 4];
    clonk_graphics::CpuSceneRenderer::default()
        .render(&scene, &mut actual)
        .unwrap();
    let mismatch = actual
        .iter()
        .zip(oracle.surface().pixels())
        .position(|(a, b)| a != b);
    assert_eq!(mismatch, None);
}

#[test]
fn retained_cpu_background_tiles_copy_integer_texels_and_alpha() {
    let image = ImageData::new(
        400,
        2,
        (0..800)
            .flat_map(|index| {
                [
                    (index % 251) as u8,
                    (index % 239) as u8,
                    (index % 227) as u8,
                    if index % 7 == 0 { 0 } else { 137 },
                ]
            })
            .collect(),
    );
    for gamma in [None, Some(clonk_graphics::GammaRamp::standard())] {
        let draw = |surface: &mut Surface| {
            surface.fill(Color::opaque(8, 12, 24));
            tile_image_on_surface(surface, &image, 0, -1, gamma.as_ref());
        };
        let mut oracle = Surface::new(1254, 3, PixelFormat::Rgba8888);
        draw(&mut oracle);
        let mut retained = Surface::new(1254, 3, PixelFormat::Rgba8888);
        retained.begin_gpu_scene_capture();
        draw(&mut retained);
        let scene = retained.take_gpu_scene_capture().unwrap().into_scene(
            [1254, 3],
            Color::transparent(),
            gamma
                .as_ref()
                .unwrap_or(&clonk_graphics::GammaRamp::identity()),
        );
        let mut actual = vec![0; 1254 * 3 * 4];
        clonk_graphics::CpuSceneRenderer::default()
            .render(&scene, &mut actual)
            .unwrap();
        assert_eq!(actual, oracle.pixels());
    }
}

#[test]
fn retained_cpu_native_filtering_canonicalizes_transparent_content() {
    // C4Surface.cpp:728-735 canonicalizes alpha-zero loaded texels before
    // native sprite filtering; physical padding remains transparent white.
    let image = ImageData::new(
        2,
        2,
        vec![
            100, 0, 0, 255, 255, 255, 255, 0, 100, 0, 0, 255, 255, 255, 255, 0,
        ],
    );
    let draw = |surface: &mut Surface| {
        surface.fill(Color::opaque(0, 0, 0));
        draw_image_region_float_source(
            surface,
            &GuiRect::new(0.0, 0.0, 1.0, 1.0),
            &image,
            None,
            &FloatSourceRect {
                x: 0.0,
                y: 0.0,
                width: 2.0,
                height: 2.0,
            },
            BlitSampling::Linear,
            false,
            None,
            SpriteBlitState::normal(),
            None,
            None,
        );
    };
    let mut oracle = Surface::new(1, 1, PixelFormat::Rgba8888);
    draw(&mut oracle);
    let mut retained = Surface::new(1, 1, PixelFormat::Rgba8888);
    retained.begin_gpu_scene_capture();
    draw(&mut retained);
    let scene = retained.take_gpu_scene_capture().unwrap().into_scene(
        [1, 1],
        Color::transparent(),
        &clonk_graphics::GammaRamp::identity(),
    );
    let mut actual = [0; 4];
    clonk_graphics::CpuSceneRenderer::default()
        .render(&scene, &mut actual)
        .unwrap();
    assert_eq!(actual, oracle.pixels());
}

#[test]
fn retained_cpu_gui_filtering_preserves_transparent_content_rgb() {
    // Generic GUI filtering retains raw texels; native runtime sprites
    // canonicalize transparent content separately (C4Surface.cpp:728-735).
    let image = ImageData::new(
        2,
        2,
        vec![
            100, 0, 0, 255, 255, 255, 255, 0, 100, 0, 0, 255, 255, 255, 255, 0,
        ],
    );
    let draw = |surface: &mut Surface| {
        surface.fill(Color::opaque(0, 0, 0));
        draw_image_bilinear(surface, &GuiRect::new(0.0, 0.0, 1.0, 1.0), &image, None);
    };
    let mut oracle = Surface::new(1, 1, PixelFormat::Rgba8888);
    draw(&mut oracle);
    let mut retained = Surface::new(1, 1, PixelFormat::Rgba8888);
    retained.begin_gpu_scene_capture();
    draw(&mut retained);
    let scene = retained.take_gpu_scene_capture().unwrap().into_scene(
        [1, 1],
        Color::transparent(),
        &clonk_graphics::GammaRamp::identity(),
    );
    let mut actual = [0; 4];
    clonk_graphics::CpuSceneRenderer::default()
        .render(&scene, &mut actual)
        .unwrap();
    assert_eq!(actual, oracle.pixels());
}

#[test]
fn retained_cpu_integer_source_sprites_keep_pixel_corner_sampling() {
    // StdDDraw2.cpp:738-741; the integer-source software path samples from
    // the integer destination offset before its divide/multiply stretch.
    let image = ImageData::new(
        3,
        1,
        vec![10, 20, 30, 255, 40, 50, 60, 255, 70, 80, 90, 255],
    );
    let draw = |surface: &mut Surface| {
        surface.fill(Color::opaque(0, 0, 0));
        draw_image_region(
            surface,
            &GuiRect::new(0.0, 0.0, 2.0, 1.0),
            &image,
            None,
            &SourceRect {
                x: 0,
                y: 0,
                width: 3,
                height: 1,
            },
            false,
            None,
            SpriteBlitState::normal(),
            None,
            None,
        );
    };
    let mut oracle = Surface::new(2, 1, PixelFormat::Rgba8888);
    draw(&mut oracle);
    let mut retained = Surface::new(2, 1, PixelFormat::Rgba8888);
    retained.begin_gpu_scene_capture();
    draw(&mut retained);
    let scene = retained.take_gpu_scene_capture().unwrap().into_scene(
        [2, 1],
        Color::transparent(),
        &clonk_graphics::GammaRamp::identity(),
    );
    let mut actual = [0; 8];
    clonk_graphics::CpuSceneRenderer::default()
        .render(&scene, &mut actual)
        .unwrap();
    assert_eq!(actual, oracle.pixels());
}

#[test]
fn retained_cpu_compact_fog_keeps_fractional_destination_and_final_endpoint() {
    // StdGL.cpp:471-527 samples fog in original source coordinates even
    // when the unadjusted object face rounds its raster destination.
    let image = ImageData::new(1, 1, vec![255, 255, 255, 255]);
    let mut map = ClrModMap::reset(1, 1, 3, 2, 0, 0, 0, 0, 0).unwrap();
    for (i, cell) in map.cells.iter_mut().enumerate() {
        *cell = ((i % map.width as usize) as u32 * 80) * 0x0001_0101;
    }
    let fog = FogDrawContext {
        map: Arc::new(map),
        zoom: 1.0,
    };
    let draw = |surface: &mut Surface| {
        surface.fill(Color::opaque(0, 0, 0));
        draw_object_image_region_float_source(
            surface,
            &GuiRect::new(-0.49, 0.0, 1.51, 1.0),
            &image,
            None,
            &FloatSourceRect {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            BlitSampling::Nearest,
            false,
            None,
            SpriteBlitState::normal(),
            None,
            Some(&fog),
        );
    };
    let mut oracle = Surface::new(2, 1, PixelFormat::Rgba8888);
    draw(&mut oracle);
    let mut retained = Surface::new(2, 1, PixelFormat::Rgba8888);
    retained.begin_gpu_scene_capture();
    draw(&mut retained);
    let scene = retained.take_gpu_scene_capture().unwrap().into_scene(
        [2, 1],
        Color::transparent(),
        &clonk_graphics::GammaRamp::identity(),
    );
    let mut actual = [0; 8];
    clonk_graphics::CpuSceneRenderer::default()
        .render(&scene, &mut actual)
        .unwrap();
    assert_eq!(actual, oracle.pixels());
}

#[test]
fn retained_cpu_fog_keeps_original_source_axis_arithmetic() {
    // StdGL.cpp:471-527 splits the original source rectangle into fog quads;
    // ModulateClr combines packed corner bytes before triangle interpolation.
    let image = ImageData::new(2430, 1880, [40, 83, 160, 255].repeat(2430 * 1880));
    let mut map = ClrModMap::reset(64, 64, 160, 160, 0, 0, 0, 0, 0).unwrap();
    map.cells.fill(0);
    map.cells[map.width as usize + 1] = 0x00b7_b7b7;
    let fog = FogDrawContext {
        map: Arc::new(map),
        zoom: 1.0,
    };
    let gamma = clonk_graphics::GammaRamp::standard();
    let draw = |surface: &mut Surface| {
        surface.fill(Color::opaque(7, 13, 23));
        draw_image_region_float_source(
            surface,
            &GuiRect::new(-1024.0, -396.0, 1200.0, 648.0),
            &image,
            None,
            &FloatSourceRect {
                x: 0.0,
                y: 1012.0,
                width: 1200.0,
                height: 648.0,
            },
            BlitSampling::Nearest,
            false,
            None,
            SpriteBlitState::normal(),
            Some(&gamma),
            Some(&fog),
        );
    };
    let mut oracle = Surface::new(150, 140, PixelFormat::Rgba8888);
    draw(&mut oracle);
    let mut retained = Surface::new(150, 140, PixelFormat::Rgba8888);
    retained.begin_gpu_scene_capture();
    draw(&mut retained);
    let scene = retained.take_gpu_scene_capture().unwrap().into_scene(
        [150, 140],
        Color::transparent(),
        &gamma,
    );
    let mut actual = vec![0; 150 * 140 * 4];
    clonk_graphics::CpuSceneRenderer::default()
        .render(&scene, &mut actual)
        .unwrap();
    let mismatch = actual.iter().zip(oracle.pixels()).position(|(a, b)| a != b);
    assert_eq!(
        mismatch,
        None,
        "first={:?}",
        mismatch.map(|i| (
            i / 4 % 150,
            i / 4 / 150,
            &actual[i / 4 * 4..i / 4 * 4 + 4],
            &oracle.pixels()[i / 4 * 4..i / 4 * 4 + 4]
        ))
    );
}

#[test]
fn retained_cpu_gui_filtering_keeps_source_scale_division() {
    // StdDDraw2.cpp:738-741 projects source edges with destination/source
    // scale; the software oracle inverts that scale before linear filtering.
    let image = ImageData::new(5, 3, (0..60).map(|i| ((i * 73 + 17) % 256) as u8).collect());
    let gamma = clonk_graphics::GammaRamp::standard();
    let draw = |surface: &mut Surface| {
        surface.fill(Color::opaque(56, 22, 1));
        draw_image_bilinear(
            surface,
            &GuiRect::new(565.0, 0.0, 148.0, 67.0),
            &image,
            Some(&gamma),
        );
    };
    let mut oracle = Surface::new(728, 67, PixelFormat::Rgba8888);
    draw(&mut oracle);
    let mut captured = Surface::new(728, 67, PixelFormat::Rgba8888);
    captured.begin_gpu_scene_capture();
    draw(&mut captured);
    let scene = captured.take_gpu_scene_capture().unwrap().into_scene(
        [728, 67],
        Color::transparent(),
        &gamma,
    );
    let mut actual = vec![0; 728 * 67 * 4];
    clonk_graphics::CpuSceneRenderer::default()
        .render(&scene, &mut actual)
        .unwrap();
    let mismatch = actual.iter().zip(oracle.pixels()).position(|(a, b)| a != b);
    assert_eq!(
        mismatch,
        None,
        "first={:?}",
        mismatch.map(|i| (
            i / 4 % 728,
            i / 4 / 728,
            &actual[i / 4 * 4..i / 4 * 4 + 4],
            &oracle.pixels()[i / 4 * 4..i / 4 * 4 + 4]
        ))
    );
}

#[test]
fn retained_cpu_landscape_keeps_native_liquid_phase_composition() {
    // StdGL.cpp:710-763 applies liquid modulation after Surface32 material
    // composition and before fog/color modulation and framebuffer blending.
    let landscape: Landscape = serde_json::from_value(serde_json::json!({
        "width": 2, "surface": [1, 1], "world_height": 1, "shade_materials": false,
        "pixels": { "width": 2, "height": 1, "bytes": "0102",
            "texture_names": [null, "Smooth", "Smooth"], "densities": [0, 25, 50],
            "material_names": [null, "Water", "Earth"] }
    }))
    .unwrap();
    let make = || {
        let mut graphics = GraphicsSystem::new(
            2,
            1,
            1,
            "CPU landscape differential",
            Arc::new(clonk_graphics::BitmapFont::new()),
            Arc::new(HashMap::new()),
            Arc::new(CursorAtlas::empty()),
            Arc::new(HudGraphics::default()),
        );
        graphics.set_material_textures(Arc::new(HashMap::from([(
            "smooth".into(),
            ImageData::new(1, 1, vec![128, 128, 128, 255]),
        )])));
        graphics.set_material_render_info(Arc::new(HashMap::from([
            (
                "water".into(),
                MaterialRenderInfo::new(
                    [120, 140, 160, 120, 140, 160, 120, 140, 160],
                    [0; 6],
                    None,
                    0,
                    25,
                ),
            ),
            (
                "earth".into(),
                MaterialRenderInfo::new(
                    [80, 100, 120, 80, 100, 120, 80, 100, 120],
                    [0; 6],
                    None,
                    0,
                    50,
                ),
            ),
        ])));
        graphics.set_liquid_animation(Some(ImageData::new(1, 1, vec![255, 128, 128, 255])));
        graphics
    };
    let mut oracle = make();
    let mut retained = make();
    for _ in 0..3 {
        oracle.surface_mut().fill(Color::opaque(9, 18, 27));
        assert!(oracle.draw_ground_textured(Some(&landscape), None));
        retained.begin_gpu_scene_capture();
        retained.surface_mut().fill(Color::opaque(9, 18, 27));
        assert!(retained.draw_ground_textured(Some(&landscape), None));
        let scene = retained
            .finish_gpu_scene_capture(&clonk_graphics::GammaRamp::identity())
            .unwrap();
        let mut actual = vec![0; 8];
        clonk_graphics::CpuSceneRenderer::default()
            .render(&scene, &mut actual)
            .unwrap();
        assert_eq!(actual, oracle.surface().pixels());
    }
}

#[test]
fn retained_cpu_plain_boxes_keep_integer_fragment_composition() {
    let draw = |surface: &mut Surface| {
        surface.fill(Color::opaque(0, 0, 0));
        draw_color_rect(
            surface,
            SurfaceRect::new(0, 0, 2, 2),
            Color::new(1, 1, 1, 128),
            None,
        );
        fill_rect_impl(
            surface,
            &GuiRect::new(2.0, 0.0, 2.0, 2.0),
            Color::new(1, 1, 1, 128),
            None,
            None,
        );
    };
    let mut oracle = Surface::new(4, 2, PixelFormat::Rgba8888);
    draw(&mut oracle);
    let mut captured = Surface::new(4, 2, PixelFormat::Rgba8888);
    captured.begin_gpu_scene_capture();
    draw(&mut captured);
    let scene = captured.take_gpu_scene_capture().unwrap().into_scene(
        [4, 2],
        Color::transparent(),
        &clonk_graphics::GammaRamp::identity(),
    );
    let mut actual = vec![0; 32];
    clonk_graphics::CpuSceneRenderer::default()
        .render(&scene, &mut actual)
        .unwrap();
    assert_eq!(actual, oracle.pixels());
}

#[test]
fn retained_cpu_transformed_sprite_uses_half_open_inverse_mapping() {
    // StdDDraw2.cpp:CBltTransform::TransformPoint maps the pixel centre back
    // into the half-open source rectangle before sampling.
    let image = ImageData::new(1, 1, vec![100, 150, 200, 255]);
    let transform = GraphicsTransform::set(0.0, -1.0, 2.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0);
    let draw = |surface: &mut Surface| {
        surface.fill(Color::opaque(0, 0, 0));
        draw_image_region_transformed_float_source(
            surface,
            (0.5, 0.5, 1.0, 1.0),
            &transform,
            &image,
            None,
            &FloatSourceRect {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            BlitSampling::Nearest,
            false,
            None,
            SpriteBlitState::normal(),
            None,
            None,
        );
    };
    let mut oracle = Surface::new(3, 3, PixelFormat::Rgba8888);
    draw(&mut oracle);
    let mut captured = Surface::new(3, 3, PixelFormat::Rgba8888);
    captured.begin_gpu_scene_capture();
    draw(&mut captured);
    let scene = captured.take_gpu_scene_capture().unwrap().into_scene(
        [3, 3],
        Color::transparent(),
        &clonk_graphics::GammaRamp::identity(),
    );
    let mut actual = vec![0; 36];
    clonk_graphics::CpuSceneRenderer::default()
        .render(&scene, &mut actual)
        .unwrap();
    assert_eq!(actual, oracle.pixels());
}

#[test]
fn retained_cpu_gamma_keeps_legacy_destination_alpha_truncation() {
    // StdGL.cpp:1081-1087 gamma affects RGB only; the native software
    // oracle preserves the legacy integer destination-alpha calculation.
    let image = ImageData::new(1, 1, vec![40, 80, 120, 128]);
    let gamma = clonk_graphics::GammaRamp::standard();
    let draw = |surface: &mut Surface| {
        surface.fill(Color::new(0, 0, 0, 2));
        draw_image_region_float_source(
            surface,
            &GuiRect::new(0.0, 0.0, 1.0, 1.0),
            &image,
            None,
            &FloatSourceRect {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            BlitSampling::Nearest,
            false,
            None,
            SpriteBlitState::normal(),
            Some(&gamma),
            None,
        );
    };
    let mut oracle = Surface::new(1, 1, PixelFormat::Rgba8888);
    draw(&mut oracle);
    let mut captured = Surface::new(1, 1, PixelFormat::Rgba8888);
    captured.begin_gpu_scene_capture();
    draw(&mut captured);
    let scene =
        captured
            .take_gpu_scene_capture()
            .unwrap()
            .into_scene([1, 1], Color::transparent(), &gamma);
    let mut actual = vec![0; 4];
    clonk_graphics::CpuSceneRenderer::default()
        .render(&scene, &mut actual)
        .unwrap();
    assert_eq!(actual, oracle.pixels());
}

#[test]
fn retained_cpu_scene_matches_immediate_sprite_sampling_and_modulation() {
    // PerformBlt selects sampling before source modulation and framebuffer
    // blending (src/StdGL.cpp:471-527,908). Keep the immediate path as oracle.
    let image = ImageData::new(5, 3, (0..60).map(|i| ((i * 73 + 17) % 256) as u8).collect());
    for sampling in [BlitSampling::Nearest, BlitSampling::Linear] {
        for modulation in [None, Some(0x4057_bde3)] {
            for mode in [
                0,
                C4GFXBLIT_ADDITIVE,
                C4GFXBLIT_MOD2,
                C4GFXBLIT_MOD2 | C4GFXBLIT_ADDITIVE,
            ] {
                let blit = SpriteBlitState {
                    mode,
                    modulation,
                    ..SpriteBlitState::normal()
                };
                let draw = |surface: &mut Surface| {
                    surface.fill(Color::new(13, 27, 39, 67));
                    draw_image_region_float_source(
                        surface,
                        &GuiRect::new(1.25, 1.75, 5.5, 4.5),
                        &image,
                        None,
                        &FloatSourceRect {
                            x: 0.25,
                            y: 0.5,
                            width: 4.5,
                            height: 2.0,
                        },
                        sampling,
                        false,
                        None,
                        blit,
                        None,
                        None,
                    );
                };
                let mut oracle = Surface::new(9, 8, PixelFormat::Rgba8888);
                draw(&mut oracle);
                let mut retained = Surface::new(9, 8, PixelFormat::Rgba8888);
                retained.begin_gpu_scene_capture();
                draw(&mut retained);
                let scene = retained.take_gpu_scene_capture().unwrap().into_scene(
                    [9, 8],
                    Color::transparent(),
                    &clonk_graphics::GammaRamp::identity(),
                );
                let mut actual = vec![0; 9 * 8 * 4];
                clonk_graphics::CpuSceneRenderer::default()
                    .render(&scene, &mut actual)
                    .unwrap();
                assert_eq!(
                    actual,
                    oracle.pixels(),
                    "sampling={sampling:?} modulation={modulation:?} mode={mode}"
                );
            }
        }
    }
}

#[test]
fn retained_cpu_nearest_gui_and_facets_preserve_source_arithmetic() {
    use clonk_graphics::Rect;
    let image = ImageData::new(
        200,
        1,
        (0..200).flat_map(|x| [x as u8, 0, 0, 255]).collect(),
    );
    for facet in [false, true] {
        let draw = |surface: &mut Surface| {
            if facet {
                crate::classic_gui::draw_facet_nearest(
                    surface,
                    &image,
                    Rect::new(0, 0, 200, 1),
                    Rect::new(0, 0, 200, 1),
                    None,
                );
            } else {
                draw_image(surface, &GuiRect::new(0.0, 0.0, 200.0, 1.0), &image);
            }
        };
        let mut oracle = Surface::new(200, 1, PixelFormat::Rgba8888);
        draw(&mut oracle);
        let mut retained = Surface::new(200, 1, PixelFormat::Rgba8888);
        retained.begin_gpu_scene_capture();
        draw(&mut retained);
        let scene = retained.take_gpu_scene_capture().unwrap().into_scene(
            [200, 1],
            Color::transparent(),
            &clonk_graphics::GammaRamp::identity(),
        );
        let mut actual = vec![0; 800];
        clonk_graphics::CpuSceneRenderer::default()
            .render(&scene, &mut actual)
            .unwrap();
        assert_eq!(actual, oracle.pixels(), "facet={facet}");
    }
}

#[test]
fn retained_cpu_classic_facet_keeps_rounded_filtered_gamma_before_blending() {
    let image = ImageData::new(
        2,
        2,
        vec![255, 1, 1, 135, 54, 0, 0, 0, 255, 1, 1, 135, 54, 0, 0, 0],
    );
    let ramp = clonk_graphics::GammaRamp::from_control_points([0x172b43, 0x698bad, 0xdff1fb]);
    for gamma in [None, Some(&ramp)] {
        let draw = |surface: &mut Surface| {
            surface.fill(Color::new(93, 38, 18, 111));
            crate::classic_gui::draw_facet_stretch(
                surface,
                &image,
                (0.0, 0.0, 2.0, 2.0),
                (0.0, 0.0, 1.0, 1.0),
                gamma,
            );
        };
        let mut oracle = Surface::new(1, 1, PixelFormat::Rgba8888);
        draw(&mut oracle);
        let mut retained = Surface::new(1, 1, PixelFormat::Rgba8888);
        retained.begin_gpu_scene_capture();
        draw(&mut retained);
        let scene = retained.take_gpu_scene_capture().unwrap().into_scene(
            [1, 1],
            Color::transparent(),
            &ramp,
        );
        let mut actual = [0; 4];
        clonk_graphics::CpuSceneRenderer::default()
            .render(&scene, &mut actual)
            .unwrap();
        assert_eq!(actual, oracle.pixels(), "gamma={}", gamma.is_some());
    }
}

#[test]
fn retained_cpu_compatibility_boxes_keep_rounded_gamma_and_alpha() {
    // StdDDraw2.cpp:1401-1404 and StdGL.cpp:846-891 pin the packed box.
    let gamma = clonk_graphics::GammaRamp::from_control_points([0x1f3f61, 0x6d8cac, 0xdcedfb]);
    for ramp in [None, Some(&gamma)] {
        for alpha in [79, 128, 176] {
            let draw = |surface: &mut Surface| {
                surface.fill(Color::new(79, 101, 139, 200));
                classic_gui::draw_engine_box(
                    surface,
                    0,
                    0,
                    1,
                    1,
                    ((255 - alpha) as u32) << 24 | 0x0032_3232,
                    ramp,
                );
            };
            let mut oracle = Surface::new(2, 2, PixelFormat::Rgba8888);
            draw(&mut oracle);
            let mut retained = Surface::new(2, 2, PixelFormat::Rgba8888);
            retained.begin_gpu_scene_capture();
            draw(&mut retained);
            let scene = retained.take_gpu_scene_capture().unwrap().into_scene(
                [2, 2],
                Color::transparent(),
                ramp.unwrap_or(&gamma),
            );
            let mut actual = [0; 16];
            clonk_graphics::CpuSceneRenderer::default()
                .render(&scene, &mut actual)
                .unwrap();
            assert_eq!(
                &actual,
                oracle.pixels(),
                "gamma={} alpha={alpha}",
                ramp.is_some()
            );
        }
    }
}

#[test]
fn retained_cpu_rotated_particles_sample_integer_corners() {
    // The existing immediate rotated-particle oracle samples integer x/y
    // for texels and x+0.5/y+0.5 for fog modulation.
    let image = ImageData::new(
        9,
        7,
        (0..9 * 7)
            .flat_map(|i| [i as u8 * 3, i as u8 * 2, i as u8, (i * 71 % 256) as u8])
            .collect(),
    );
    for rotation in [17.0, 71.0, 180.0, 237.0] {
        for flip_x in [false, true] {
            let draw = |surface: &mut Surface| {
                surface.fill(Color::opaque(23, 37, 59));
                draw_image_region_rotated(
                    surface,
                    12.25,
                    13.75,
                    17.0,
                    13.0,
                    &image,
                    None,
                    &SourceRect {
                        x: 0,
                        y: 0,
                        width: 9,
                        height: 7,
                    },
                    flip_x,
                    None,
                    rotation,
                    SpriteBlitState::normal(),
                    None,
                    None,
                );
            };
            let mut oracle = Surface::new(32, 32, PixelFormat::Rgba8888);
            draw(&mut oracle);
            let mut retained = Surface::new(32, 32, PixelFormat::Rgba8888);
            retained.begin_gpu_scene_capture();
            draw(&mut retained);
            let scene = retained.take_gpu_scene_capture().unwrap().into_scene(
                [32, 32],
                Color::transparent(),
                &clonk_graphics::GammaRamp::identity(),
            );
            let mut actual = vec![0; 32 * 32 * 4];
            clonk_graphics::CpuSceneRenderer::default()
                .render(&scene, &mut actual)
                .unwrap();
            let mismatch = actual
                .chunks_exact(4)
                .zip(oracle.pixels().chunks_exact(4))
                .enumerate()
                .find(|(_, (a, b))| a != b)
                .map(|(i, (a, b))| (i % 32, i / 32, a, b));
            assert!(
                mismatch.is_none(),
                "rotation={rotation} flip={flip_x} {mismatch:?}"
            );
        }
    }
}
