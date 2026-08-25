use super::*;

pub(crate) fn draw_viewport_underlay(
    cache: &mut TiledUnderlayCache,
    surface: &mut Surface,
    background: Option<&ImageData>,
    origin_x: i32,
    origin_y: i32,
    gamma: Option<&clonk_graphics::GammaRamp>,
) {
    if let Some(background) = background {
        cache.draw(surface, background, origin_x, origin_y, gamma);
    } else {
        surface.fill(Color::opaque(0, 0, 0));
    }
}

pub(crate) fn present_viewport_content(
    destination: &mut Surface,
    viewport_underlay: Option<&mut Surface>,
    content: &mut Surface,
    rect: SurfaceRect,
    offset_x: i32,
    offset_y: i32,
) {
    if let Some(viewport_underlay) = viewport_underlay {
        blit_surface_from(viewport_underlay, content, offset_x, offset_y);
        blit_surface_from(destination, viewport_underlay, rect.x, rect.y);
    } else {
        debug_assert_eq!(offset_x, 0);
        debug_assert_eq!(offset_y, 0);
        debug_assert_eq!(content.width(), rect.width);
        debug_assert_eq!(content.height(), rect.height);
        blit_surface_from(destination, content, rect.x, rect.y);
    }
}

pub(crate) fn tile_image_on_surface(
    surface: &mut Surface,
    image: &ImageData,
    origin_x: i32,
    origin_y: i32,
    gamma: Option<&clonk_graphics::GammaRamp>,
) {
    let image_width = image.width() as usize;
    let image_height = image.height() as usize;
    let surface_width = surface.width() as usize;
    let surface_height = surface.height() as usize;
    if image_width == 0 || image_height == 0 || surface_width == 0 || surface_height == 0 {
        return;
    }
    if let Some(renderer_config) =
        active_advanced_renderer_config().filter(|config| config.has_adjusted_quad_geometry())
    {
        let Ok(image_width_i32) = i32::try_from(image_width) else {
            return;
        };
        let Ok(image_height_i32) = i32::try_from(image_height) else {
            return;
        };
        let first_x = -origin_x.rem_euclid(image_width_i32);
        let first_y = -origin_y.rem_euclid(image_height_i32);
        let mut target_y = first_y;
        while target_y < surface.height() as i32 {
            let mut target_x = first_x;
            while target_x < surface.width() as i32 {
                let clipped_left = target_x.max(0);
                let clipped_top = target_y.max(0);
                let clipped_right = target_x
                    .saturating_add(image_width_i32)
                    .min(surface.width() as i32);
                let clipped_bottom = target_y
                    .saturating_add(image_height_i32)
                    .min(surface.height() as i32);
                if clipped_left < clipped_right && clipped_top < clipped_bottom {
                    draw_image_source_configured_on_surface(
                        surface,
                        &GuiRect::new(
                            clipped_left as f32,
                            clipped_top as f32,
                            (clipped_right - clipped_left) as f32,
                            (clipped_bottom - clipped_top) as f32,
                        ),
                        image,
                        FloatSourceRect {
                            x: (clipped_left - target_x) as f32,
                            y: (clipped_top - target_y) as f32,
                            width: (clipped_right - clipped_left) as f32,
                            height: (clipped_bottom - clipped_top) as f32,
                        },
                        BlitSampling::Nearest,
                        gamma,
                        BilinearBlend::AlphaOver,
                        None,
                        renderer_config,
                    );
                }
                target_x = target_x.saturating_add(image_width_i32);
            }
            target_y = target_y.saturating_add(image_height_i32);
        }
        return;
    }
    if surface.is_gpu_scene_capture_active() {
        let start_x = origin_x.rem_euclid(image_width as i32);
        let start_y = origin_y.rem_euclid(image_height as i32);
        let source = SourceRect::new(0, 0, image_width as i32, image_height as i32);
        let renderer_config =
            active_advanced_renderer_config().unwrap_or(AdvancedRendererConfig::DEFAULT);
        let mut y = -start_y;
        while y < surface_height as i32 {
            let mut x = -start_x;
            while x < surface_width as i32 {
                draw_image_region(
                    surface,
                    &GuiRect::new(x as f32, y as f32, image_width as f32, image_height as f32),
                    image,
                    None,
                    &source,
                    false,
                    None,
                    SpriteBlitState::normal().with_renderer_config(renderer_config),
                    gamma,
                    None,
                );
                x += image_width as i32;
            }
            y += image_height as i32;
        }
        return;
    }
    let source = image.pixels();
    let source_stride = image_width.saturating_mul(4);
    let destination_stride = surface.stride();
    if source.len() < source_stride.saturating_mul(image_height)
        || destination_stride < surface_width.saturating_mul(4)
    {
        return;
    }
    let start_x = origin_x.rem_euclid(image_width as i32) as usize;
    let destination = surface.pixels_mut();
    for y in 0..surface_height {
        let source_y = (origin_y + y as i32).rem_euclid(image_height as i32) as usize;
        let source_row = &source[source_y * source_stride..(source_y + 1) * source_stride];
        let destination_row =
            &mut destination[y * destination_stride..y * destination_stride + surface_width * 4];
        let mut destination_x = 0;
        let mut source_x = start_x;
        while destination_x < surface_width {
            let copy_width = (image_width - source_x).min(surface_width - destination_x);
            let source_start = source_x * 4;
            let destination_start = destination_x * 4;
            if let Some(gamma) = gamma {
                for pixel in 0..copy_width {
                    let source = source_start + pixel * 4;
                    let destination = destination_start + pixel * 4;
                    let encoded = gamma_encode_fragment(
                        Color::new(
                            source_row[source],
                            source_row[source + 1],
                            source_row[source + 2],
                            source_row[source + 3],
                        ),
                        gamma,
                    );
                    destination_row[destination..destination + 4]
                        .copy_from_slice(&[encoded.r, encoded.g, encoded.b, encoded.a]);
                }
            } else {
                destination_row[destination_start..destination_start + copy_width * 4]
                    .copy_from_slice(&source_row[source_start..source_start + copy_width * 4]);
            }
            destination_x += copy_width;
            source_x = 0;
        }
    }
}

pub(crate) fn blit_surface(dst: &mut Surface, src: &Surface, offset_x: i32, offset_y: i32) {
    if src.width() == 0 || src.height() == 0 {
        return;
    }
    if dst.format() != src.format() {
        return;
    }
    if dst.is_gpu_scene_capture_active() {
        if src.is_gpu_scene_capture_active() {
            let _ = dst.append_gpu_scene_from(src, SurfacePoint::new(offset_x, offset_y));
        } else {
            let _ = dst.blit_region_ex(
                src,
                src.bounds(),
                SurfacePoint::new(offset_x, offset_y),
                Color::opaque(255, 255, 255),
                clonk_graphics::BlitMode::Normal,
            );
        }
        return;
    }
    if offset_x >= dst.width() as i32 || offset_y >= dst.height() as i32 {
        return;
    }

    let start_x = offset_x.max(0) as u32;
    let start_y = offset_y.max(0) as u32;
    if start_x >= dst.width() || start_y >= dst.height() {
        return;
    }

    let max_width = dst.width().saturating_sub(start_x);
    let max_height = dst.height().saturating_sub(start_y);
    let copy_width = src.width().min(max_width);
    let copy_height = src.height().min(max_height);
    if copy_width == 0 || copy_height == 0 {
        return;
    }

    let dst_stride = dst.stride();
    let src_stride = src.stride();
    if src.width() == 0 {
        return;
    }
    let bpp = src_stride / src.width() as usize;
    if bpp == 0 {
        return;
    }

    let dst_pixels = dst.pixels_mut();
    let src_pixels = src.pixels();

    for row in 0..copy_height {
        let dst_row = (start_y + row) as usize;
        let dst_offset = dst_row
            .saturating_mul(dst_stride)
            .saturating_add(start_x as usize * bpp);
        let src_offset = row as usize * src_stride;
        let len = copy_width as usize * bpp;
        if dst_offset + len > dst_pixels.len() || src_offset + len > src_pixels.len() {
            break;
        }
        dst_pixels[dst_offset..dst_offset + len]
            .copy_from_slice(&src_pixels[src_offset..src_offset + len]);
    }
}

/// Present a scratch surface that the caller no longer needs. Retained GPU
/// captures can transfer their recorder directly; CPU-backed or mixed paths
/// retain the ordinary read-only blit semantics.
pub(crate) fn blit_surface_from(
    dst: &mut Surface,
    src: &mut Surface,
    offset_x: i32,
    offset_y: i32,
) {
    if src.width() == 0 || src.height() == 0 || dst.format() != src.format() {
        return;
    }
    if dst.is_gpu_scene_capture_active() && src.is_gpu_scene_capture_active() {
        let _ = dst.append_gpu_scene_from_mut(src, SurfacePoint::new(offset_x, offset_y));
        return;
    }
    blit_surface(dst, src, offset_x, offset_y);
}

pub(crate) fn object_color(object: &ObjectSnapshot) -> Color {
    let mut hasher = DefaultHasher::new();
    object.definition_id.hash(&mut hasher);
    let hash = hasher.finish();

    let channel = |shift: u32| -> u8 {
        let component = ((hash >> shift) & 0x7F) as u8;
        component.saturating_add(64)
    };

    let base_r = channel(0);
    let base_g = channel(8);
    let base_b = channel(16);
    let low_tint = (200u8, 64u8, 64u8);
    let energy = object.energy.clamp(0, 100) as f32 / 100.0;
    let mix_channel = |base: u8, low: u8| -> u8 {
        let value = (low as f32 * (1.0 - energy) + base as f32 * energy)
            .round()
            .clamp(0.0, 255.0);
        value as u8
    };

    let mut r = mix_channel(base_r, low_tint.0);
    let mut g = mix_channel(base_g, low_tint.1);
    let mut b = mix_channel(base_b, low_tint.2);

    if !object.alive || !object.status.is_active() {
        let fade = 0.45f32;
        r = (r as f32 * fade).round() as u8;
        g = (g as f32 * fade).round() as u8;
        b = (b as f32 * fade).round() as u8;
    }

    Color::opaque(r, g, b)
}

#[cfg(test)]
pub(crate) fn c4_palette_color(index: u8) -> Color {
    GamePalette::default().color(index)
}

pub(crate) fn modulate_line_palette_color(color: Color, modulation: Option<u32>) -> Color {
    let Some(modulation) = modulation else {
        return color;
    };
    // DrawLineDw receives a packed C4 color (high byte is transparency),
    // and ClrByCurrentBlitMod runs the integer `ModulateClr` path before GL.
    // This is deliberately NOT the sprite shader's `/255` RGB and
    // saturating-sub opacity math: C++ uses `>>8` for RGB and screen-combines
    // transparency (src/StdColors.h:159-171; src/StdGL.cpp:893-933).
    let packed = (u32::from(255 - color.a) << 24)
        | (u32::from(color.r) << 16)
        | (u32::from(color.g) << 8)
        | u32::from(color.b);
    let modulated = split_c4_color(modulate_c4_colors(packed, modulation));
    Color::new(modulated[0], modulated[1], modulated[2], 255 - modulated[3])
}

fn prepared_gpu_solid_color(
    color: Color,
    blit: SpriteBlitState,
) -> ([f32; 4], GpuSolidOuterModulation) {
    match prepare_sprite_fragment(color, None, None, blit) {
        PreparedSpriteFragment::Legacy(color) => {
            (gpu_rgba(color), GpuSolidOuterModulation::PackedC4)
        }
        PreparedSpriteFragment::Shader { rgb, alpha } => (
            [
                rgb[0] / 255.0,
                rgb[1] / 255.0,
                rgb[2] / 255.0,
                alpha / 255.0,
            ],
            GpuSolidOuterModulation::SampledTexture,
        ),
        PreparedSpriteFragment::Layers { .. } => {
            unreachable!("unmasked solid primitives never have owner layers")
        }
    }
}

fn gpu_solid_vertex(position: (f32, f32), color: Color, blit: SpriteBlitState) -> GpuSolidVertex {
    let (color, outer_modulation) = prepared_gpu_solid_color(color, blit);
    GpuSolidVertex {
        position: [position.0, position.1, 1.0],
        color,
        outer_modulation,
    }
}

fn gpu_blend_for_blit(blit: SpriteBlitState) -> GpuBlend {
    if blit.mode & C4GFXBLIT_ADDITIVE != 0 {
        GpuBlend::Additive
    } else {
        GpuBlend::Normal
    }
}

fn draw_object_line_pixel(
    surface: &mut Surface,
    x: i32,
    y: i32,
    color: Color,
    blit: SpriteBlitState,
    gamma: Option<&clonk_graphics::GammaRamp>,
) {
    if x < 0 || y < 0 || x >= surface.width() as i32 || y >= surface.height() as i32 {
        return;
    }
    let source = prepare_sprite_fragment(color, None, None, blit);
    if source.alpha() == 0.0 {
        return;
    }
    if surface.is_gpu_scene_capture_active() {
        let (color, outer_modulation) = prepared_gpu_solid_color(color, blit);
        surface.push_gpu_command(GpuCommand::Solid {
            vertices: vec![GpuSolidVertex {
                position: [x as f32 + 0.5, y as f32 + 0.5, 1.0],
                color,
                outer_modulation,
            }],
            topology: GpuPrimitiveTopology::PointList,
            alpha_mode: GpuSolidAlphaMode::SourceOver,
            clip: surface.clip(),
            blend: gpu_blend_for_blit(blit),
            style: GpuSolidStyle::with_gamma(gamma.is_some_and(|gamma| !gamma.is_passthrough())),
        });
        return;
    }
    let destination = surface.get_pixel(x as u32, y as u32).unwrap_or_default();
    let output = composite_sprite_fragment(source, destination, blit, gamma);
    let _ = surface.set_pixel(x as u32, y as u32, output);
}

/// `C4FacetEx::DrawBolt`'s coarse facet cull and four `SafeRandom(7)-3`
/// draws. The returned points retain C++'s raw `pvtx` order:
/// start, end, jittered end, jittered start (src/C4FacetEx.cpp:61-80).
pub(crate) fn build_bolt_quad(
    start: (i32, i32),
    end: (i32, i32),
    width: i32,
    height: i32,
    rng: &mut SafeRng,
) -> Option<[(i32, i32); 4]> {
    if width <= 0 || height <= 0 {
        return None;
    }
    let inside_x = |x| (0..width).contains(&x);
    let inside_y = |y| (0..height).contains(&y);
    // Deliberately not a segment/side-aware cull: C++ rejects even a segment
    // spanning the whole facet when both endpoints are outside one axis.
    if (!inside_x(start.0) && !inside_x(end.0)) || (!inside_y(start.1) && !inside_y(end.1)) {
        return None;
    }

    let end_jitter = (rng.random(7) - 3, rng.random(7) - 3);
    let start_jitter = (rng.random(7) - 3, rng.random(7) - 3);
    Some([
        start,
        end,
        (end.0 + end_jitter.0, end.1 + end_jitter.1),
        (start.0 + start_jitter.0, start.1 + start_jitter.1),
    ])
}

fn triangle_edge(a: (f32, f32), b: (f32, f32), point: (f32, f32)) -> f64 {
    let (ax, ay) = (f64::from(a.0), f64::from(a.1));
    let (bx, by) = (f64::from(b.0), f64::from(b.1));
    let (px, py) = (f64::from(point.0), f64::from(point.1));
    (bx - ax) * (py - ay) - (by - ay) * (px - ax)
}

/// GL's top-left ownership rule in the renderer's y-down surface space.
fn triangle_top_left(a: (f32, f32), b: (f32, f32)) -> bool {
    b.1 < a.1 || (b.1 == a.1 && b.0 > a.0)
}

fn interpolate_color(start: Color, end: Color, amount: f32) -> Color {
    let amount = amount.clamp(0.0, 1.0);
    let channel = |start: u8, end: u8| {
        (f32::from(start) + (f32::from(end) - f32::from(start)) * amount)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    Color::new(
        channel(start.r, end.r),
        channel(start.g, end.g),
        channel(start.b, end.b),
        channel(start.a, end.a),
    )
}

fn interpolate_triangle_color(colors: [Color; 3], weights: [f64; 3]) -> Color {
    let channel = |select: fn(Color) -> u8| {
        colors
            .iter()
            .copied()
            .zip(weights)
            .map(|(color, weight)| f64::from(select(color)) * weight)
            .sum::<f64>()
            .round()
            .clamp(0.0, 255.0) as u8
    };
    Color::new(
        channel(|color| color.r),
        channel(|color| color.g),
        channel(|color| color.b),
        channel(|color| color.a),
    )
}

fn line_color_at(
    start: (f32, f32),
    end: (f32, f32),
    start_color: Color,
    end_color: Color,
    point: (i32, i32),
) -> Color {
    let direction = (end.0 - start.0, end.1 - start.1);
    let length_sq = direction.0 * direction.0 + direction.1 * direction.1;
    if length_sq <= f32::EPSILON {
        return start_color;
    }
    let offset = (point.0 as f32 - start.0, point.1 as f32 - start.1);
    let amount = (offset.0 * direction.0 + offset.1 * direction.1) / length_sq;
    interpolate_color(start_color, end_color, amount)
}

pub(crate) fn draw_object_triangle(
    surface: &mut Surface,
    vertices: [(f32, f32); 3],
    colors: [Color; 3],
    blit: SpriteBlitState,
    gamma: Option<&clonk_graphics::GammaRamp>,
) {
    let area = triangle_edge(vertices[0], vertices[1], vertices[2]);
    if area == 0.0 || surface.width() == 0 || surface.height() == 0 {
        return;
    }
    if surface.is_gpu_scene_capture_active() {
        let vertices = vertices
            .into_iter()
            .zip(colors)
            .map(|(position, color)| gpu_solid_vertex(position, color, blit))
            .collect();
        surface.push_gpu_command(GpuCommand::Solid {
            vertices,
            topology: GpuPrimitiveTopology::TriangleList,
            alpha_mode: GpuSolidAlphaMode::SourceOver,
            clip: surface.clip(),
            blend: gpu_blend_for_blit(blit),
            style: GpuSolidStyle::with_gamma(gamma.is_some_and(|gamma| !gamma.is_passthrough())),
        });
        return;
    }
    let orientation = if area > 0.0 { 1.0 } else { -1.0 };
    let min_x = vertices
        .iter()
        .map(|point| point.0)
        .fold(f32::INFINITY, f32::min)
        .floor()
        .max(0.0) as i32;
    let max_x = vertices
        .iter()
        .map(|point| point.0)
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil()
        .min(surface.width() as f32) as i32;
    let min_y = vertices
        .iter()
        .map(|point| point.1)
        .fold(f32::INFINITY, f32::min)
        .floor()
        .max(0.0) as i32;
    let max_y = vertices
        .iter()
        .map(|point| point.1)
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil()
        .min(surface.height() as f32) as i32;

    let edges = [
        (vertices[0], vertices[1]),
        (vertices[1], vertices[2]),
        (vertices[2], vertices[0]),
    ];
    for y in min_y..max_y {
        for x in min_x..max_x {
            let sample = (x as f32 + 0.5, y as f32 + 0.5);
            let covered = edges.iter().all(|&(a, b)| {
                let edge = triangle_edge(a, b, sample) * orientation;
                if edge > 0.0 {
                    true
                } else if edge < 0.0 {
                    false
                } else if orientation > 0.0 {
                    triangle_top_left(a, b)
                } else {
                    triangle_top_left(b, a)
                }
            });
            if covered {
                let weights = [
                    triangle_edge(vertices[1], vertices[2], sample) / area,
                    triangle_edge(vertices[2], vertices[0], sample) / area,
                    triangle_edge(vertices[0], vertices[1], sample) / area,
                ];
                let color = interpolate_triangle_color(colors, weights);
                draw_object_line_pixel(surface, x, y, color, blit, gamma);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_object_bolt_segment(
    surface: &mut Surface,
    start: (i32, i32),
    end: (i32, i32),
    logical_width: i32,
    logical_height: i32,
    zoom: f32,
    primary: Color,
    marker: Color,
    blit: SpriteBlitState,
    gamma: Option<&clonk_graphics::GammaRamp>,
    fog: Option<&FogDrawContext>,
    rng: &mut SafeRng,
) {
    let Some(points) = build_bolt_quad(start, end, logical_width, logical_height, rng) else {
        return;
    };
    let offset = blit.renderer_config.destination_offset();
    let unshifted = points.map(|(x, y)| (x as f32 * zoom, y as f32 * zoom));
    let output = unshifted.map(|(x, y)| (x + offset, y + offset));
    let mut colors = [primary, marker, marker, primary];
    if let Some(fog) = fog {
        for (color, point) in colors.iter_mut().zip(unshifted) {
            *color = fog.color_at_point(*color, point.0, point.1);
        }
    }
    if blit.renderer_config.no_box_fades {
        let normalized = normalize_quad_colors(colors);
        colors.fill(normalized);
    }
    // DrawQuadDw submits GL_TRIANGLE_STRIP as raw 0,1,3,2. Rasterize the two
    // triangles separately: a folded strip can legitimately overlap itself.
    draw_object_triangle(
        surface,
        [output[0], output[1], output[3]],
        [colors[0], colors[1], colors[3]],
        blit,
        gamma,
    );
    draw_object_triangle(
        surface,
        [output[3], output[1], output[2]],
        [colors[3], colors[1], colors[2]],
        blit,
        gamma,
    );
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_object_line_segment(
    surface: &mut Surface,
    start: (f32, f32),
    end: (f32, f32),
    primary: Color,
    marker: Color,
    blit: SpriteBlitState,
    gamma: Option<&clonk_graphics::GammaRamp>,
    fog: Option<&FogDrawContext>,
) {
    // CStdGL::DrawLineDw shifts integer vertices to pixel centers. GL's
    // diamond-exit rule makes each segment half-open at its final endpoint
    // (src/StdGL.cpp:893-933). C4FacetEx then paints the secondary-color
    // point at the ORIGINAL start, including for zero-length segments
    // (src/C4FacetEx.cpp:46-54).
    let primary_start = fog.map_or(primary, |fog| fog.color_at_point(primary, start.0, start.1));
    let primary_end = fog.map_or(primary, |fog| fog.color_at_point(primary, end.0, end.1));
    if surface.is_gpu_scene_capture_active() {
        // DrawLineDw accepts floats and only applies its +0.5 pixel-center
        // shift. Preserve subpixel viewport zoom instead of pre-rounding it.
        // A coincident pair must remain GL_LINES: diamond-exit then emits no
        // primary fragment, while the independent DrawPix marker below does.
        let line_start = (start.0 + 0.5, start.1 + 0.5);
        let line_end = (end.0 + 0.5, end.1 + 0.5);
        surface.push_gpu_command(GpuCommand::Solid {
            vertices: vec![
                gpu_solid_vertex(line_start, primary_start, blit),
                gpu_solid_vertex(line_end, primary_end, blit),
            ],
            topology: GpuPrimitiveTopology::LineList,
            alpha_mode: GpuSolidAlphaMode::SourceOver,
            clip: surface.clip(),
            blend: gpu_blend_for_blit(blit),
            style: GpuSolidStyle::with_gamma(gamma.is_some_and(|gamma| !gamma.is_passthrough())),
        });
        let marker = fog.map_or(marker, |fog| fog.color_at_point(marker, start.0, start.1));
        let (marker_color, outer_modulation) = prepared_gpu_solid_color(marker, blit);
        if marker_color[3] > 0.0 {
            surface.push_gpu_command(GpuCommand::Solid {
                vertices: vec![GpuSolidVertex {
                    position: [line_start.0, line_start.1, 1.0],
                    color: marker_color,
                    outer_modulation,
                }],
                topology: GpuPrimitiveTopology::PointList,
                alpha_mode: GpuSolidAlphaMode::SourceOver,
                clip: surface.clip(),
                blend: gpu_blend_for_blit(blit),
                style: GpuSolidStyle::with_gamma(
                    gamma.is_some_and(|gamma| !gamma.is_passthrough()),
                ),
            });
        }
        return;
    }
    if start != end {
        if let Some((clipped_start, clipped_end)) = clip_pxs_line(surface, start, end) {
            let (mut x0, mut y0) = (
                clipped_start.0.round() as i32,
                clipped_start.1.round() as i32,
            );
            let (x1, y1) = (clipped_end.0.round() as i32, clipped_end.1.round() as i32);
            if x0 == x1 && y0 == y1 {
                let color = line_color_at(start, end, primary_start, primary_end, (x0, y0));
                draw_object_line_pixel(surface, x0, y0, color, blit, gamma);
            } else {
                let dx = (x1 - x0).abs();
                let sx = if x0 < x1 { 1 } else { -1 };
                let dy = -(y1 - y0).abs();
                let sy = if y0 < y1 { 1 } else { -1 };
                let mut error = dx + dy;
                while x0 != x1 || y0 != y1 {
                    let color = line_color_at(start, end, primary_start, primary_end, (x0, y0));
                    draw_object_line_pixel(surface, x0, y0, color, blit, gamma);
                    let doubled = error * 2;
                    if doubled >= dy {
                        error += dy;
                        x0 += sx;
                    }
                    if doubled <= dx {
                        error += dx;
                        y0 += sy;
                    }
                }
            }
        }
    }
    let marker = fog.map_or(marker, |fog| fog.color_at_point(marker, start.0, start.1));
    draw_object_line_pixel(
        surface,
        start.0.round() as i32,
        start.1.round() as i32,
        marker,
        blit,
        gamma,
    );
}

pub(crate) fn draw_pxs_pixel(
    surface: &mut Surface,
    x: f32,
    y: f32,
    color: Color,
    gamma: Option<&clonk_graphics::GammaRamp>,
    fog: Option<&FogDrawContext>,
) {
    if !x.is_finite() || !y.is_finite() {
        return;
    }
    let color = fog.map_or(color, |fog| fog.color_at_point(color, x, y));
    if surface.is_gpu_scene_capture_active() {
        let clip = surface.clip();
        surface.push_gpu_solid_vertex(
            GpuSolidVertex {
                position: [x + 0.5, y + 0.5, 1.0],
                color: gpu_rgba(color),
                outer_modulation: GpuSolidOuterModulation::PackedC4,
            },
            GpuPrimitiveTopology::PointList,
            GpuSolidAlphaMode::SourceOver,
            clip,
            GpuBlend::Normal,
            GpuSolidStyle::with_gamma(gamma.is_some_and(|gamma| !gamma.is_passthrough())),
        );
        return;
    }
    // At application scale one, DrawPixInt's `(tx + 0.5, ty + 0.5)` point
    // rasterizes to these logical pixels. Retained capture above deliberately
    // defers physical coverage/culling until presentation scale is known.
    let raster_x = x.round() as i32;
    let raster_y = y.round() as i32;
    if raster_x < 0
        || raster_y < 0
        || raster_x >= surface.width() as i32
        || raster_y >= surface.height() as i32
    {
        return;
    }
    let background = surface
        .get_pixel(raster_x as u32, raster_y as u32)
        .unwrap_or_default();
    let blended = gamma.map_or_else(
        || blend_color_over(color, background),
        |gamma| gamma_blend_fragment_over(color, background, gamma),
    );
    let _ = surface.set_pixel(raster_x as u32, raster_y as u32, blended);
}

pub(crate) fn draw_pxs_line(
    surface: &mut Surface,
    start: (f32, f32),
    end: (f32, f32),
    color: Color,
    gamma: Option<&clonk_graphics::GammaRamp>,
    fog: Option<&FogDrawContext>,
) {
    // Integer raster counterpart of CStdGL::DrawLineDw's GL_LINES call. Its
    // vertices are shifted by 0.5, and GL's diamond-exit rule makes the
    // segment half-open at its final endpoint (StdGL.cpp:893-933).
    let original_start = start;
    let original_end = end;
    let start_color = fog.map_or(color, |fog| {
        fog.color_at_point(color, original_start.0, original_start.1)
    });
    let end_color = fog.map_or(color, |fog| {
        fog.color_at_point(color, original_end.0, original_end.1)
    });
    if surface.is_gpu_scene_capture_active() {
        // C4PXS supplies exact fixed-point floats to DrawLineDw. StdGL adds
        // 0.5 but does not round them; physical diamond-exit rasterization is
        // responsible for selecting pixels after the viewport transform.
        let start = (start.0 + 0.5, start.1 + 0.5);
        let end = (end.0 + 0.5, end.1 + 0.5);
        let clip = surface.clip();
        // A coincident GL_LINES pair is a fragmentless primitive, not a point.
        // Keeping both vertices preserves that final-end exclusion.
        surface.push_gpu_solid_vertex_pair(
            gpu_solid_vertex(start, start_color, SpriteBlitState::normal()),
            gpu_solid_vertex(end, end_color, SpriteBlitState::normal()),
            GpuPrimitiveTopology::LineList,
            GpuSolidAlphaMode::SourceOver,
            clip,
            GpuBlend::Normal,
            GpuSolidStyle::with_gamma(gamma.is_some_and(|gamma| !gamma.is_passthrough())),
        );
        return;
    }
    let Some((start, end)) = clip_pxs_line(surface, start, end) else {
        return;
    };
    let (mut x0, mut y0) = (start.0.round() as i32, start.1.round() as i32);
    let (x1, y1) = (end.0.round() as i32, end.1.round() as i32);
    if x0 == x1 && y0 == y1 {
        let color = line_color_at(
            original_start,
            original_end,
            start_color,
            end_color,
            (x0, y0),
        );
        draw_pxs_pixel(surface, x0 as f32, y0 as f32, color, gamma, None);
        return;
    }
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut error = dx + dy;
    while x0 != x1 || y0 != y1 {
        let color = line_color_at(
            original_start,
            original_end,
            start_color,
            end_color,
            (x0, y0),
        );
        draw_pxs_pixel(surface, x0 as f32, y0 as f32, color, gamma, None);
        let doubled = error * 2;
        if doubled >= dy {
            error += dy;
            x0 += sx;
        }
        if doubled <= dx {
            error += dx;
            y0 += sy;
        }
    }
}

pub(crate) fn draw_mouse_selection_frame_raster(
    surface: &mut Surface,
    viewport_clip: SurfaceRect,
    current: (i32, i32),
    down: (i32, i32),
    color: Color,
    gamma: Option<&clonk_graphics::GammaRamp>,
) {
    let previous_clip = surface.clip();
    let clip = previous_clip
        .and_then(|clip| clip.intersection(viewport_clip))
        .unwrap_or_else(|| {
            if previous_clip.is_some() {
                SurfaceRect::new(0, 0, 0, 0)
            } else {
                viewport_clip
            }
        });
    surface.set_clip(clip);

    let (x1, y1) = current;
    let (x2, y2) = down;
    // CStdDDraw::DrawFrame calls these four edges in this exact order. The
    // endpoints are intentionally not normalized: GL_LINES is half-open at
    // the second vertex, which leaves the common (x2,y2) endpoint untouched
    // (src/StdDDraw2.cpp:1173-1180; src/StdGL.cpp:893-933).
    draw_pxs_line(
        surface,
        (x1 as f32, y1 as f32),
        (x2 as f32, y1 as f32),
        color,
        gamma,
        None,
    );
    draw_pxs_line(
        surface,
        (x1 as f32, y2 as f32),
        (x2 as f32, y2 as f32),
        color,
        gamma,
        None,
    );
    draw_pxs_line(
        surface,
        (x1 as f32, y1 as f32),
        (x1 as f32, y2 as f32),
        color,
        gamma,
        None,
    );
    draw_pxs_line(
        surface,
        (x2 as f32, y1 as f32),
        (x2 as f32, y2 as f32),
        color,
        gamma,
        None,
    );

    match previous_clip {
        Some(clip) => surface.set_clip(clip),
        None => surface.clear_clip(),
    }
}

fn clip_pxs_line(
    surface: &Surface,
    start: (f32, f32),
    end: (f32, f32),
) -> Option<((f32, f32), (f32, f32))> {
    if surface.width() == 0 || surface.height() == 0 {
        return None;
    }
    // GL clips C4PXS velocity lines to the render target. Liang-Barsky keeps
    // a malicious/extreme xdir from turning the CPU raster into a multi-
    // billion-step loop while preserving the in-target segment.
    let (x0, y0) = (f64::from(start.0), f64::from(start.1));
    let (dx, dy) = (f64::from(end.0) - x0, f64::from(end.1) - y0);
    if !(x0.is_finite() && y0.is_finite() && dx.is_finite() && dy.is_finite()) {
        return None;
    }
    let mut enter = 0.0f64;
    let mut exit = 1.0f64;
    let bounds = [
        (-dx, x0),
        (dx, f64::from(surface.width() - 1) - x0),
        (-dy, y0),
        (dy, f64::from(surface.height() - 1) - y0),
    ];
    for (direction, distance) in bounds {
        if direction.abs() <= f64::EPSILON {
            if distance < 0.0 {
                return None;
            }
            continue;
        }
        let ratio = distance / direction;
        if direction < 0.0 {
            enter = enter.max(ratio);
        } else {
            exit = exit.min(ratio);
        }
        if enter > exit {
            return None;
        }
    }
    Some((
        ((x0 + enter * dx) as f32, (y0 + enter * dy) as f32),
        ((x0 + exit * dx) as f32, (y0 + exit * dy) as f32),
    ))
}

pub(crate) fn draw_pxs_image_region(
    surface: &mut Surface,
    target: &GuiRect,
    image: &ImageData,
    source: &SourceRect,
    modulation_transparency: u8,
    lighting: f32,
    renderer_config: AdvancedRendererConfig,
    gamma: Option<&clonk_graphics::GammaRamp>,
    fog: Option<&FogDrawContext>,
) {
    if target.size.width <= 0.0
        || target.size.height <= 0.0
        || source.width <= 0
        || source.height <= 0
    {
        return;
    }
    if image.width() == 0 || image.height() == 0 {
        return;
    }
    let offset = renderer_config.destination_offset();
    let dest = (
        target.origin.x + offset,
        target.origin.y + offset,
        target.size.width,
        target.size.height,
    );
    if !dest.0.is_finite() || !dest.1.is_finite() {
        return;
    }
    let source = FloatSourceRect::scaled(*source, 1.0);
    let base_modulation = (u32::from(modulation_transparency) << 24) | 0x00ff_ffff;
    // Graphical PXS never selects a special blit mode. Still install the
    // device snapshot here so the normal mode passes through the same native
    // AllowedBlitModes masking as every other textured submission.
    let blit = SpriteBlitState {
        mode: renderer_config.masked_blit_mode(0),
        modulation: Some(base_modulation),
        fog_modulation: None,
        renderer_config,
    };
    let lighting_channel = (lighting.max(0.0) * 255.0).round().clamp(0.0, 255.0) as u32;
    let gpu_blit = SpriteBlitState {
        modulation: Some(
            (u32::from(modulation_transparency) << 24)
                | (lighting_channel << 16)
                | (lighting_channel << 8)
                | lighting_channel,
        ),
        ..blit
    };
    if capture_gpu_sprite(
        surface,
        dest,
        dest,
        &GraphicsTransform::identity(),
        image,
        None,
        source,
        false,
        None,
        gpu_blit,
        gamma,
        fog,
        GpuSampler::Linear,
        false,
    ) {
        return;
    }
    let fog_sampler = fog.and_then(|fog| {
        FogSpriteSampler::new(
            fog,
            dest,
            (source.x, source.y, source.width, source.height),
            (image.width(), image.height()),
            false,
            |x, y| (x, y),
        )
    });
    let bounds = surface.bounds();
    let first_x = ((dest.0 - 0.5).ceil() as i32).max(bounds.x);
    let first_y = ((dest.1 - 0.5).ceil() as i32).max(bounds.y);
    let last_x = ((dest.0 + dest.2 - 0.5).ceil() as i32).min(bounds.x + bounds.width as i32);
    let last_y = ((dest.1 + dest.3 - 0.5).ceil() as i32).min(bounds.y + bounds.height as i32);
    for y in first_y..last_y {
        let normalized_y = (y as f32 + 0.5 - dest.1) / dest.3;
        if !(0.0..1.0).contains(&normalized_y) {
            continue;
        }
        for x in first_x..last_x {
            let normalized_x = (x as f32 + 0.5 - dest.0) / dest.2;
            if !(0.0..1.0).contains(&normalized_x) {
                continue;
            }
            let (source_edge_x, source_edge_y) =
                source.source_edge(normalized_x, normalized_y, false);
            let pixel_blit = fog_sprite_blit_at(
                fog_sampler.as_ref(),
                fog,
                blit,
                normalized_x,
                normalized_y,
                x,
                y,
            );
            let Some(fragment) = prepare_runtime_sprite_sample(
                image,
                None,
                &source,
                fog.is_some(),
                source_edge_x,
                source_edge_y,
                BlitSampling::Linear,
                None,
                pixel_blit,
            ) else {
                continue;
            };
            // ActivateBlitModulation guarantees the filtered base surface is
            // prepared as one shader-equivalent fragment. Apply viewport
            // lighting after texture/fog modulation but before gamma, which
            // is algebraically identical to StdGL's vertex-color product.
            let PreparedSpriteFragment::Shader { mut rgb, alpha } = fragment else {
                unreachable!("graphical PXS always activates blit modulation");
            };
            for channel in &mut rgb {
                *channel = (*channel * lighting).clamp(0.0, 255.0);
            }
            if alpha <= 0.0 {
                continue;
            }
            blend_prepared_sprite_fragment(
                surface,
                x as u32,
                y as u32,
                PreparedSpriteFragment::Shader { rgb, alpha },
                pixel_blit,
                gamma,
            );
        }
    }
}

pub(crate) fn blend_color_over(source: Color, dest: Color) -> Color {
    let alpha = source.a as u16;
    if alpha == 0 {
        return dest;
    }
    if alpha == 255 {
        return source;
    }
    let inv_alpha = 255u16 - alpha;
    let blend_channel =
        |src: u8, dst: u8| -> u8 { ((src as u16 * alpha + dst as u16 * inv_alpha) / 255) as u8 };

    Color::new(
        blend_channel(source.r, dest.r),
        blend_channel(source.g, dest.g),
        blend_channel(source.b, dest.b),
        (alpha + (dest.a as u16 * inv_alpha) / 255).min(255) as u8,
    )
}

pub(crate) fn fill_polygon(surface: &mut Surface, points: &[(i32, i32)], color: Color) -> bool {
    fill_polygon_impl(surface, points, color, None, None)
}

pub(crate) fn fill_polygon_impl(
    surface: &mut Surface,
    points: &[(i32, i32)],
    color: Color,
    fog: Option<&FogDrawContext>,
    gamma: Option<&clonk_graphics::GammaRamp>,
) -> bool {
    if points.len() < 3 {
        return false;
    }

    let width = surface.width() as i32;
    let height = surface.height() as i32;
    if width <= 0 || height <= 0 {
        return false;
    }

    let mut min_y = i32::MAX;
    let mut max_y = i32::MIN;
    for &(_, y) in points {
        min_y = min_y.min(y);
        max_y = max_y.max(y);
    }

    if min_y > max_y || min_y >= height || max_y < 0 {
        return false;
    }

    let start_y = min_y.max(0);
    let end_y = max_y.min(height - 1);
    if start_y > end_y {
        return false;
    }

    let mut intersections = Vec::with_capacity(points.len());
    let mut drawn = false;

    for y in start_y..=end_y {
        intersections.clear();
        let y_f = y as f64;
        for i in 0..points.len() {
            let (x1, y1) = points[i];
            let (x2, y2) = points[(i + 1) % points.len()];
            let y1_f = y1 as f64;
            let y2_f = y2 as f64;
            if ((y1_f <= y_f) && (y2_f > y_f)) || ((y2_f <= y_f) && (y1_f > y_f)) {
                let x1_f = x1 as f64;
                let x2_f = x2 as f64;
                let x = x1_f + (y_f - y1_f) * (x2_f - x1_f) / (y2_f - y1_f);
                intersections.push(x);
            }
        }

        if intersections.len() < 2 {
            continue;
        }

        intersections.sort_by(|a, b| a.partial_cmp(b).unwrap());

        let mut iter = intersections.iter();
        while let Some(&start) = iter.next() {
            if let Some(&end) = iter.next() {
                let mut x_start = start.ceil() as i32;
                let mut x_end = end.floor() as i32;
                if x_end < x_start {
                    continue;
                }
                if x_end < 0 || x_start >= width {
                    continue;
                }
                if x_start < 0 {
                    x_start = 0;
                }
                if x_end >= width {
                    x_end = width - 1;
                }
                for x in x_start..=x_end {
                    let color = fog.map_or(color, |fog| fog.color_at(color, x, y));
                    if surface.is_gpu_scene_capture_active() {
                        surface.push_gpu_command(GpuCommand::Solid {
                            vertices: vec![GpuSolidVertex {
                                position: [x as f32 + 0.5, y as f32 + 0.5, 1.0],
                                color: gpu_rgba(color),
                                outer_modulation: GpuSolidOuterModulation::PackedC4,
                            }],
                            topology: GpuPrimitiveTopology::PointList,
                            alpha_mode: GpuSolidAlphaMode::SourceOver,
                            clip: surface.clip(),
                            blend: GpuBlend::Normal,
                            style: GpuSolidStyle::with_gamma(
                                gamma.is_some_and(|gamma| !gamma.is_passthrough()),
                            ),
                        });
                        drawn = true;
                        continue;
                    }
                    let destination = surface.get_pixel(x as u32, y as u32).unwrap_or_default();
                    let output = match (color.a, gamma) {
                        (255, Some(gamma)) => gamma_encode_fragment(color, gamma),
                        (255, None) => color,
                        (_, Some(gamma)) => gamma_blend_fragment_over(color, destination, gamma),
                        (_, None) => blend_colors(color, destination),
                    };
                    let _ = surface.set_pixel(x as u32, y as u32, output);
                    drawn = true;
                }
            }
        }
    }

    drawn
}

pub(crate) fn fill_rect(surface: &mut Surface, rect: &GuiRect, color: Color) {
    fill_rect_impl(surface, rect, color, None, None);
}

pub(crate) fn fill_rect_impl(
    surface: &mut Surface,
    rect: &GuiRect,
    color: Color,
    fog: Option<&FogDrawContext>,
    gamma: Option<&clonk_graphics::GammaRamp>,
) {
    let x0 = rect.origin.x.floor() as i32;
    let y0 = rect.origin.y.floor() as i32;
    let x1 = (rect.origin.x + rect.size.width).ceil() as i32;
    let y1 = (rect.origin.y + rect.size.height).ceil() as i32;

    let x0 = x0.clamp(0, surface.width() as i32);
    let y0 = y0.clamp(0, surface.height() as i32);
    let x1 = x1.clamp(0, surface.width() as i32);
    let y1 = y1.clamp(0, surface.height() as i32);

    if surface.is_gpu_scene_capture_active() && fog.is_none() && x0 < x1 && y0 < y1 {
        let vertex = |x, y| GpuSolidVertex {
            position: [x as f32, y as f32, 1.0],
            color: gpu_rgba(color),
            outer_modulation: GpuSolidOuterModulation::PackedC4,
        };
        let corners = [
            vertex(x0, y0),
            vertex(x1, y0),
            vertex(x0, y1),
            vertex(x1, y1),
        ];
        surface.push_gpu_command(GpuCommand::Solid {
            vertices: vec![
                corners[0], corners[1], corners[2], corners[2], corners[1], corners[3],
            ],
            topology: GpuPrimitiveTopology::TriangleList,
            alpha_mode: GpuSolidAlphaMode::SourceOver,
            clip: surface.clip(),
            blend: GpuBlend::Normal,
            style: GpuSolidStyle::with_gamma(gamma.is_some_and(|gamma| !gamma.is_passthrough())),
        });
        return;
    }

    for y in y0..y1 {
        for x in x0..x1 {
            let color = fog.map_or(color, |fog| fog.color_at(color, x, y));
            if surface.is_gpu_scene_capture_active() {
                surface.push_gpu_command(GpuCommand::Solid {
                    vertices: vec![GpuSolidVertex {
                        position: [x as f32 + 0.5, y as f32 + 0.5, 1.0],
                        color: gpu_rgba(color),
                        outer_modulation: GpuSolidOuterModulation::PackedC4,
                    }],
                    topology: GpuPrimitiveTopology::PointList,
                    alpha_mode: GpuSolidAlphaMode::SourceOver,
                    clip: surface.clip(),
                    blend: GpuBlend::Normal,
                    style: GpuSolidStyle::with_gamma(
                        gamma.is_some_and(|gamma| !gamma.is_passthrough()),
                    ),
                });
                continue;
            }
            let destination = surface.get_pixel(x as u32, y as u32).unwrap_or_default();
            let output = match (color.a, gamma) {
                (255, Some(gamma)) => gamma_encode_fragment(color, gamma),
                (255, None) => color,
                (_, Some(gamma)) => gamma_blend_fragment_over(color, destination, gamma),
                (_, None) => blend_colors(color, destination),
            };
            let _ = surface.set_pixel(x as u32, y as u32, output);
        }
    }
}

/// Rebase a C4 draw matrix around the target-space pivot used by
/// `C4DrawTransform::SetTransformAt` (src/C4Facet.cpp:446-456).
pub(crate) fn draw_transform_at(matrix: [f32; 9], off_x: f32, off_y: f32) -> GraphicsTransform {
    let [a, b, c, d, e, f, g, h, i] = matrix;
    let rebased_a = a + g * off_x;
    let rebased_b = b + h * off_x;
    let rebased_d = d + g * off_y;
    let rebased_e = e + h * off_y;
    GraphicsTransform::set(
        rebased_a,
        rebased_b,
        c - rebased_a * off_x - rebased_b * off_y + i * off_x,
        rebased_d,
        rebased_e,
        f - rebased_d * off_x - rebased_e * off_y + i * off_y,
        g,
        h,
        i - g * off_x - h * off_y,
    )
}

/// The 1.5x-scaled text caret every editable classic dialog draws.
///
/// The five dialogs that own a text field each grew their own copy of this
/// routine. They render the same glyph through the same atlas and the same
/// clipped stretch, so they share one implementation here.
pub(crate) fn draw_scaled_caret(
    surface: &mut Surface,
    font: &clonk_graphics::clonk_font::ClonkFont,
    x: i32,
    y: i32,
    clip: crate::classic_gui::IntRect,
    gamma: Option<&clonk_graphics::GammaRamp>,
) {
    const SCALE: f32 = 1.5;
    let Some(glyph) = font.glyph('\u{a6}') else {
        return;
    };
    let Ok(width) = u32::try_from(glyph.width) else {
        return;
    };
    let Ok(height) = u32::try_from(font.cell_height) else {
        return;
    };
    if width == 0 || height == 0 || glyph.pixels.len() != width as usize * height as usize {
        return;
    }
    // Keep the glyph inside one texture tile like the real font atlas. A
    // narrow standalone image would otherwise be split into width-sized
    // vertical tiles by the shared C4Surface blitter.
    let atlas_width = width.max(height).next_power_of_two();
    let mut glyph_hash = 0xcbf2_9ce4_8422_2325_u64;
    for pixel in &glyph.pixels {
        for byte in [pixel.r, pixel.g, pixel.b, pixel.a] {
            glyph_hash = (glyph_hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    thread_local! {
        /// The caret atlas is immutable for a given rasterized font. Reusing
        /// its ImageData identity keeps the retained renderer from allocating
        /// a fresh GPU texture cache entry on every blinking frame.
        static CARET_ATLASES: std::cell::RefCell<
            std::collections::HashMap<(u32, u32, u64), ImageData>,
        > = std::cell::RefCell::new(std::collections::HashMap::new());
    }
    let image = CARET_ATLASES.with(|atlases| {
        let key = (width, height, glyph_hash);
        if let Some(image) = atlases.borrow().get(&key).cloned() {
            return image;
        }
        let mut pixels = vec![255_u8; atlas_width as usize * height as usize * 4];
        for pixel in pixels.chunks_exact_mut(4) {
            pixel[3] = 0;
        }
        for row in 0..height as usize {
            for column in 0..width as usize {
                let pixel = glyph.pixels[row * width as usize + column];
                let destination = (row * atlas_width as usize + column) * 4;
                // C4Surface initializes unused font-atlas pixels to transparent
                // white. Preserve that RGB for the GL_LINEAR 1.5x sample.
                let (red, green, blue) = if pixel.a == 0 {
                    (255, 255, 255)
                } else {
                    (pixel.r, pixel.g, pixel.b)
                };
                pixels[destination..destination + 4].copy_from_slice(&[red, green, blue, pixel.a]);
            }
        }
        let image = ImageData::new(atlas_width, height, pixels);
        atlases.borrow_mut().insert(key, image.clone());
        image
    });
    let destination = (
        x as f32,
        y as f32,
        width as f32 * SCALE,
        height as f32 * SCALE,
    );
    let left = destination.0.max(clip.x as f32);
    let top = destination.1.max(clip.y as f32);
    let right = (destination.0 + destination.2).min((clip.x + clip.w) as f32);
    let bottom = (destination.1 + destination.3).min((clip.y + clip.h) as f32);
    if left >= right || top >= bottom {
        return;
    }
    crate::classic_gui::draw_facet_stretch(
        surface,
        &image,
        (
            (left - destination.0) / SCALE,
            (top - destination.1) / SCALE,
            (right - left) / SCALE,
            (bottom - top) / SCALE,
        ),
        (left, top, right - left, bottom - top),
        gamma,
    );
}
