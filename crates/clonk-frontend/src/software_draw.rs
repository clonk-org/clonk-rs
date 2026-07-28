use super::*;

#[allow(clippy::too_many_arguments)]
pub(crate) fn capture_gpu_gui_image(
    surface: &mut Surface,
    dest: (f32, f32, f32, f32),
    image: &ImageData,
    source: FloatSourceRect,
    sampler: GpuSampler,
    blend_mode: BilinearBlend,
    modulation: Option<u32>,
    gamma: Option<&clonk_graphics::GammaRamp>,
) -> bool {
    let renderer_config =
        active_advanced_renderer_config().unwrap_or(AdvancedRendererConfig::DEFAULT);
    capture_gpu_gui_image_with_renderer_config(
        surface,
        dest,
        image,
        source,
        sampler,
        blend_mode,
        modulation,
        gamma,
        renderer_config,
    )
}

#[allow(clippy::too_many_arguments)]
fn capture_gpu_gui_image_with_renderer_config(
    surface: &mut Surface,
    dest: (f32, f32, f32, f32),
    image: &ImageData,
    source: FloatSourceRect,
    sampler: GpuSampler,
    blend_mode: BilinearBlend,
    modulation: Option<u32>,
    gamma: Option<&clonk_graphics::GammaRamp>,
    renderer_config: AdvancedRendererConfig,
) -> bool {
    let offset = renderer_config.destination_offset();
    let dest = (dest.0 + offset, dest.1 + offset, dest.2, dest.3);
    let requested_mode = match blend_mode {
        BilinearBlend::AlphaOver => 0,
        BilinearBlend::Additive => C4GFXBLIT_ADDITIVE,
    };
    capture_gpu_sprite(
        surface,
        dest,
        dest,
        &GraphicsTransform::identity(),
        image,
        None,
        source,
        false,
        None,
        SpriteBlitState {
            mode: renderer_config.masked_blit_mode(requested_mode),
            modulation,
            fog_modulation: None,
            renderer_config,
        },
        gamma,
        None,
        sampler,
        false,
    )
}

/// Generic CStdDDraw blit path used while a production renderer snapshot is
/// active. Runtime sprites already model the native physical texture tiles,
/// transparent padding, TexIndent transform, modulation combiner, and final
/// blend mode; reuse that pipeline so GUI/HUD submissions cannot drift from
/// world rendering.
#[allow(clippy::too_many_arguments)]
fn draw_image_source_configured<T: SurfaceDrawTarget + ?Sized>(
    surface: &mut T,
    rect: &GuiRect,
    image: &ImageData,
    source: FloatSourceRect,
    sampling: BlitSampling,
    gamma: Option<&clonk_graphics::GammaRamp>,
    blend_mode: BilinearBlend,
    modulation: Option<u32>,
    renderer_config: AdvancedRendererConfig,
) {
    if rect.size.width <= 0.0
        || rect.size.height <= 0.0
        || !source.is_valid()
        || image.width() == 0
        || image.height() == 0
    {
        return;
    }
    let offset = renderer_config.destination_offset();
    let destination = (
        rect.origin.x + offset,
        rect.origin.y + offset,
        rect.size.width,
        rect.size.height,
    );
    if !destination.0.is_finite()
        || !destination.1.is_finite()
        || !destination.2.is_finite()
        || !destination.3.is_finite()
    {
        return;
    }
    let requested_mode = match blend_mode {
        BilinearBlend::AlphaOver => 0,
        BilinearBlend::Additive => C4GFXBLIT_ADDITIVE,
    };
    let blit = SpriteBlitState {
        mode: renderer_config.masked_blit_mode(requested_mode),
        modulation: modulation.map(|color| if color == 0 { 0xff } else { color }),
        fog_modulation: None,
        renderer_config,
    };
    let first_x = ((destination.0 - 0.5).ceil() as i32).max(0);
    let first_y = ((destination.1 - 0.5).ceil() as i32).max(0);
    let last_x = ((destination.0 + destination.2 - 0.5).ceil() as i32)
        .min(i32::try_from(surface.width()).unwrap_or(i32::MAX));
    let last_y = ((destination.1 + destination.3 - 0.5).ceil() as i32)
        .min(i32::try_from(surface.height()).unwrap_or(i32::MAX));
    for target_y in first_y..last_y {
        let normalized_y = (target_y as f32 + 0.5 - destination.1) / destination.3;
        if !(0.0..1.0).contains(&normalized_y) {
            continue;
        }
        for target_x in first_x..last_x {
            let normalized_x = (target_x as f32 + 0.5 - destination.0) / destination.2;
            if !(0.0..1.0).contains(&normalized_x) {
                continue;
            }
            let (source_edge_x, source_edge_y) =
                source.source_edge(normalized_x, normalized_y, false);
            let Some(source_fragment) = prepare_runtime_sprite_sample(
                image,
                None,
                &source,
                false,
                source_edge_x,
                source_edge_y,
                sampling,
                None,
                blit,
            ) else {
                continue;
            };
            if source_fragment.alpha() <= 0.0 {
                continue;
            }
            blend_prepared_sprite_fragment_target(
                surface,
                target_x as u32,
                target_y as u32,
                source_fragment,
                blit,
                gamma,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_image_source_configured_on_surface(
    surface: &mut Surface,
    rect: &GuiRect,
    image: &ImageData,
    source: FloatSourceRect,
    sampling: BlitSampling,
    gamma: Option<&clonk_graphics::GammaRamp>,
    blend_mode: BilinearBlend,
    modulation: Option<u32>,
    renderer_config: AdvancedRendererConfig,
) {
    let modulation = modulation.map(|color| if color == 0 { 0xff } else { color });
    if capture_gpu_gui_image_with_renderer_config(
        surface,
        (
            rect.origin.x,
            rect.origin.y,
            rect.size.width,
            rect.size.height,
        ),
        image,
        source,
        gpu_sampler_for_blit(sampling),
        blend_mode,
        modulation,
        gamma,
        renderer_config,
    ) {
        return;
    }
    draw_image_source_configured(
        surface,
        rect,
        image,
        source,
        sampling,
        gamma,
        blend_mode,
        modulation,
        renderer_config,
    );
}

/// Lets sibling GUI modules retain their exact compatibility rasterizer when
/// called directly, while joining the configured CStdDDraw path inside a
/// production render scope.
pub(crate) fn draw_image_source_with_active_renderer_config(
    surface: &mut Surface,
    rect: &GuiRect,
    image: &ImageData,
    source: (f32, f32, f32, f32),
    sampling: BlitSampling,
    gamma: Option<&clonk_graphics::GammaRamp>,
) -> bool {
    let Some(renderer_config) = active_advanced_renderer_config()
        .filter(|config| config.changes_generic_textured_blit(0, false))
    else {
        return false;
    };
    draw_image_source_configured_on_surface(
        surface,
        rect,
        image,
        FloatSourceRect {
            x: source.0,
            y: source.1,
            width: source.2,
            height: source.3,
        },
        sampling,
        gamma,
        BilinearBlend::AlphaOver,
        None,
        renderer_config,
    );
    true
}

pub(crate) fn draw_image_source_modulated_with_active_renderer_config(
    surface: &mut Surface,
    rect: &GuiRect,
    image: &ImageData,
    source: (f32, f32, f32, f32),
    sampling: BlitSampling,
    modulation: u32,
    gamma: Option<&clonk_graphics::GammaRamp>,
) -> bool {
    let Some(renderer_config) = active_advanced_renderer_config()
        .filter(|config| config.changes_generic_textured_blit(0, true))
    else {
        return false;
    };
    draw_image_source_configured_on_surface(
        surface,
        rect,
        image,
        FloatSourceRect {
            x: source.0,
            y: source.1,
            width: source.2,
            height: source.3,
        },
        sampling,
        gamma,
        BilinearBlend::AlphaOver,
        Some(modulation),
        renderer_config,
    );
    true
}

/// Stretches a floating-point source window into `rect` like
/// `CStdDDraw::Blit` (StdDDraw2.cpp:637-786): one quad per power-of-two texture
/// tile, GL_LINEAR sampling with GL_CLAMP_TO_EDGE per tile, the blit shader's
/// gamma lookup on the fragment color, and float blending rounded once on
/// store.
fn draw_image_bilinear_source_cpu<T: SurfaceDrawTarget + ?Sized>(
    surface: &mut T,
    rect: &GuiRect,
    image: &ImageData,
    source: FloatSourceRect,
    gamma: Option<&clonk_graphics::GammaRamp>,
    blend_mode: BilinearBlend,
    modulation: Option<u32>,
) {
    let requested_mode = match blend_mode {
        BilinearBlend::AlphaOver => 0,
        BilinearBlend::Additive => C4GFXBLIT_ADDITIVE,
    };
    if let Some(renderer_config) = active_advanced_renderer_config()
        .filter(|config| config.changes_generic_textured_blit(requested_mode, modulation.is_some()))
    {
        return draw_image_source_configured(
            surface,
            rect,
            image,
            source,
            BlitSampling::Linear,
            gamma,
            blend_mode,
            modulation,
            renderer_config,
        );
    }
    if rect.size.width <= 0.0
        || rect.size.height <= 0.0
        || !source.is_valid()
        || image.width() == 0
        || image.height() == 0
    {
        return;
    }
    let (fw, fh) = (image.width() as f32, image.height() as f32);
    let (tx, ty) = (rect.origin.x, rect.origin.y);
    let scale_x = rect.size.width / source.width;
    let scale_y = rect.size.height / source.height;
    let source_right = source.x + source.width;
    let source_bottom = source.y + source.height;
    let ts = cpp_tex_size(image.width(), image.height()) as i32;
    let tiles_x = (image.width() as i32 - 1) / ts + 1;
    let tiles_y = (image.height() as i32 - 1) / ts + 1;
    // CStdDDraw chooses the final involved texture with a cast of
    // `source_end - 1` before integer division. A source window ending less
    // than one texel into the next tile consequently does not emit that tile
    // (StdDDraw2.cpp:695-696).
    let first_tile_x = ((source.x / ts as f32) as i32).max(0);
    let first_tile_y = ((source.y / ts as f32) as i32).max(0);
    let final_tile_x = (((source_right - 1.0) as i32) / ts + 1).min(tiles_x);
    let final_tile_y = (((source_bottom - 1.0) as i32) / ts + 1).min(tiles_y);
    let modulation = modulation.map(|color| split_c4_color(if color == 0 { 0xff } else { color }));

    for tile_iy in first_tile_y..final_tile_y {
        for tile_ix in first_tile_x..final_tile_x {
            let (blit_x, blit_y) = (tile_ix * ts, tile_iy * ts);
            // Intersect this texture tile with the requested source window
            // (fTexBlt* in StdDDraw2.cpp:731-734).
            let s_left = (blit_x as f32).max(source.x);
            let s_top = (blit_y as f32).max(source.y);
            let s_right = ((blit_x + ts) as f32).min(source_right).min(fw);
            let s_bottom = ((blit_y + ts) as f32).min(source_bottom).min(fh);
            if s_left >= s_right || s_top >= s_bottom {
                continue;
            }
            // Destination quad (tTexBlt* in StdDDraw2.cpp:738-741).
            let t_left = (s_left - source.x) * scale_x + tx;
            let t_top = (s_top - source.y) * scale_y + ty;
            let t_right = (s_right - source.x) * scale_x + tx;
            let t_bottom = (s_bottom - source.y) * scale_y + ty;
            // Pixels whose centers fall inside the quad.
            let px0 = (t_left - 0.5).ceil() as i32;
            let py0 = (t_top - 0.5).ceil() as i32;
            for py in py0.max(0)..surface.height() as i32 {
                if (py as f32 + 0.5) >= t_bottom {
                    break;
                }
                for px in px0.max(0)..surface.width() as i32 {
                    if (px as f32 + 0.5) >= t_right {
                        break;
                    }
                    let u_rel = source.x + (px as f32 + 0.5 - tx) / scale_x - 0.5 - blit_x as f32;
                    let v_rel = source.y + (py as f32 + 0.5 - ty) / scale_y - 0.5 - blit_y as f32;
                    let mut s = bilinear_sample_tile(image, blit_x, blit_y, ts, u_rel, v_rel);
                    if let Some([red, green, blue, transparency]) = modulation {
                        s[0] *= f32::from(red) / 255.0;
                        s[1] *= f32::from(green) / 255.0;
                        s[2] *= f32::from(blue) / 255.0;
                        s[3] = (s[3] - f32::from(transparency)).max(0.0);
                    }
                    if s[3] <= 0.0 {
                        continue;
                    }
                    let _ = match blend_mode {
                        BilinearBlend::AlphaOver => {
                            surface.blend_fragment_over(px as u32, py as u32, s, gamma)
                        }
                        BilinearBlend::Additive => {
                            surface.blend_fragment_additive(px as u32, py as u32, s, gamma)
                        }
                    };
                }
            }
        }
    }
}

fn draw_image_bilinear_source_impl(
    surface: &mut Surface,
    rect: &GuiRect,
    image: &ImageData,
    source: FloatSourceRect,
    gamma: Option<&clonk_graphics::GammaRamp>,
    blend_mode: BilinearBlend,
    modulation: Option<u32>,
) {
    let modulation = modulation.map(|color| if color == 0 { 0xff } else { color });
    if capture_gpu_gui_image(
        surface,
        (
            rect.origin.x,
            rect.origin.y,
            rect.size.width,
            rect.size.height,
        ),
        image,
        source,
        GpuSampler::Linear,
        blend_mode,
        modulation,
        gamma,
    ) {
        return;
    }
    draw_image_bilinear_source_cpu(surface, rect, image, source, gamma, blend_mode, modulation);
}

fn draw_image_bilinear_cpu<T: SurfaceDrawTarget + ?Sized>(
    surface: &mut T,
    rect: &GuiRect,
    image: &ImageData,
    gamma: Option<&clonk_graphics::GammaRamp>,
    blend_mode: BilinearBlend,
    modulation: Option<u32>,
) {
    draw_image_bilinear_source_cpu(
        surface,
        rect,
        image,
        FloatSourceRect {
            x: 0.0,
            y: 0.0,
            width: image.width() as f32,
            height: image.height() as f32,
        },
        gamma,
        blend_mode,
        modulation,
    );
}

fn draw_image_bilinear_impl(
    surface: &mut Surface,
    rect: &GuiRect,
    image: &ImageData,
    gamma: Option<&clonk_graphics::GammaRamp>,
    blend_mode: BilinearBlend,
    modulation: Option<u32>,
) {
    draw_image_bilinear_source_impl(
        surface,
        rect,
        image,
        FloatSourceRect {
            x: 0.0,
            y: 0.0,
            width: image.width() as f32,
            height: image.height() as f32,
        },
        gamma,
        blend_mode,
        modulation,
    );
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct DrawXFloatCrop {
    pub(crate) target_x: i32,
    pub(crate) target_y: i32,
    pub(crate) target_width: i32,
    pub(crate) target_height: i32,
    pub(crate) source: FloatSourceRect,
}

/// Computes `C4Facet::DrawXFloat`'s inward integer destination and matching
/// proportional source crop (C4Facet.cpp:306-319).
pub(crate) fn draw_x_float_crop(
    rect: &GuiRect,
    source_width: u32,
    source_height: u32,
) -> Option<DrawXFloatCrop> {
    if source_width == 0
        || source_height == 0
        || !rect.origin.x.is_finite()
        || !rect.origin.y.is_finite()
        || !rect.size.width.is_finite()
        || !rect.size.height.is_finite()
        || rect.size.width <= 0.0
        || rect.size.height <= 0.0
    {
        return None;
    }

    let right = rect.origin.x + rect.size.width;
    let bottom = rect.origin.y + rect.size.height;
    if !right.is_finite() || !bottom.is_finite() {
        return None;
    }
    let target_x = rect.origin.x.ceil() as i32;
    let target_y = rect.origin.y.ceil() as i32;
    let target_right = right.floor() as i32;
    let target_bottom = bottom.floor() as i32;
    let target_width = target_right.checked_sub(target_x)?;
    let target_height = target_bottom.checked_sub(target_y)?;
    if target_width <= 0 || target_height <= 0 {
        return None;
    }

    let zoom_x = rect.size.width / source_width as f32;
    let zoom_y = rect.size.height / source_height as f32;
    let offset_x = (-rect.origin.x + target_x as f32) / zoom_x;
    let offset_y = (-rect.origin.y + target_y as f32) / zoom_y;
    let trailing_x = (right - target_right as f32) / zoom_x;
    let trailing_y = (bottom - target_bottom as f32) / zoom_y;
    Some(DrawXFloatCrop {
        target_x,
        target_y,
        target_width,
        target_height,
        source: FloatSourceRect {
            x: offset_x,
            y: offset_y,
            width: source_width as f32 - offset_x - trailing_x,
            height: source_height as f32 - offset_y - trailing_y,
        },
    })
}

/// Samples a filtered colour channel the way the C++ blit shader does. The
/// normalized R16 result stays in float for blending and is rounded only on
/// framebuffer store (StdGL.cpp:908,1082-1086,1246-1255).
pub(crate) fn sample_channel(
    gamma: Option<&clonk_graphics::GammaRamp>,
    channel: clonk_graphics::gamma::GammaChannel,
    x: f32,
) -> f32 {
    gamma
        .map(|ramp| ramp.sample_channel_float(channel, x))
        .unwrap_or_else(|| x.clamp(0.0, 255.0))
}

pub(crate) fn store_channel(value: f32) -> u8 {
    value.round().clamp(0.0, 255.0) as u8
}

/// Applies the C++ shader's independent normalized R16 lookups to one source
/// fragment. Alpha bypasses gamma unchanged (StdGL.cpp:1081-1087).
pub fn gamma_encode_fragment(color: Color, gamma: &clonk_graphics::GammaRamp) -> Color {
    Color::new(
        store_channel(sample_channel(
            Some(gamma),
            clonk_graphics::gamma::GammaChannel::Red,
            f32::from(color.r),
        )),
        store_channel(sample_channel(
            Some(gamma),
            clonk_graphics::gamma::GammaChannel::Green,
            f32::from(color.g),
        )),
        store_channel(sample_channel(
            Some(gamma),
            clonk_graphics::gamma::GammaChannel::Blue,
            f32::from(color.b),
        )),
        color.a,
    )
}

/// Gamma-samples the source in float, then performs source-alpha blending and
/// rounds once on framebuffer store. A post-composite gamma pass is observably
/// different (StdGL.cpp:908,1081-1087,1246-1263).
pub fn gamma_blend_fragment_over(
    source: Color,
    destination: Color,
    gamma: &clonk_graphics::GammaRamp,
) -> Color {
    if source.a == 0 {
        return destination;
    }
    let alpha = f32::from(source.a) / 255.0;
    let blend = |channel, source: u8, destination: u8| {
        store_channel(
            sample_channel(Some(gamma), channel, f32::from(source)) * alpha
                + f32::from(destination) * (1.0 - alpha),
        )
    };
    Color::new(
        blend(
            clonk_graphics::gamma::GammaChannel::Red,
            source.r,
            destination.r,
        ),
        blend(
            clonk_graphics::gamma::GammaChannel::Green,
            source.g,
            destination.g,
        ),
        blend(
            clonk_graphics::gamma::GammaChannel::Blue,
            source.b,
            destination.b,
        ),
        blend_color_over(source, destination).a,
    )
}

/// Draws a solid or translucent GUI rectangle through the active fragment
/// gamma lookup before alpha blending, matching `DrawBoxDw`/the GL shader.
pub fn draw_color_rect(
    surface: &mut Surface,
    rect: SurfaceRect,
    color: Color,
    gamma: Option<&clonk_graphics::GammaRamp>,
) {
    if let Some(renderer_config) =
        active_advanced_renderer_config().filter(|config| config.changes_generic_color_quad())
    {
        let color = if renderer_config.no_box_fades {
            normalize_quad_colors([color; 4])
        } else {
            color
        };
        let offset = renderer_config.destination_offset();
        let left = rect.x as f32 + offset;
        let top = rect.y as f32 + offset;
        let right = left + rect.width as f32;
        let bottom = top + rect.height as f32;
        if surface.is_gpu_scene_capture_active() {
            record_gpu_solid_quad(
                surface,
                (left, top, right, bottom),
                [color; 4],
                GpuBlend::Normal,
                GpuSolidStyle::with_gamma(gamma.is_some_and(|gamma| !gamma.is_passthrough())),
            );
            return;
        }
        let first_x = ((left - 0.5).ceil() as i32).max(0);
        let first_y = ((top - 0.5).ceil() as i32).max(0);
        let last_x = ((right - 0.5).ceil() as i32).min(surface.width() as i32);
        let last_y = ((bottom - 0.5).ceil() as i32).min(surface.height() as i32);
        for y in first_y..last_y {
            for x in first_x..last_x {
                let destination = surface.get_pixel(x as u32, y as u32).unwrap_or_default();
                let output = match gamma {
                    Some(gamma) if color.a == 255 => gamma_encode_fragment(color, gamma),
                    Some(gamma) => gamma_blend_fragment_over(color, destination, gamma),
                    None if color.a == 255 => color,
                    None => {
                        let alpha = f32::from(color.a) / 255.0;
                        let blend = |source: u8, destination: u8| {
                            store_channel(
                                f32::from(source) * alpha + f32::from(destination) * (1.0 - alpha),
                            )
                        };
                        Color::new(
                            blend(color.r, destination.r),
                            blend(color.g, destination.g),
                            blend(color.b, destination.b),
                            store_channel(
                                f32::from(color.a) + f32::from(destination.a) * (1.0 - alpha),
                            ),
                        )
                    }
                };
                let _ = surface.set_pixel(x as u32, y as u32, output);
            }
        }
        return;
    }
    let Some(clipped) = rect.intersection(surface.bounds()) else {
        return;
    };
    if surface.is_gpu_scene_capture_active() {
        let left = clipped.x as f32;
        let top = clipped.y as f32;
        let right = left + clipped.width as f32;
        let bottom = top + clipped.height as f32;
        let vertex = |x, y| GpuSolidVertex {
            position: [x, y, 1.0],
            color: gpu_rgba(color),
            outer_modulation: GpuSolidOuterModulation::PackedC4,
        };
        surface.push_gpu_command(GpuCommand::Solid {
            vertices: vec![
                vertex(left, top),
                vertex(right, top),
                vertex(left, bottom),
                vertex(left, bottom),
                vertex(right, top),
                vertex(right, bottom),
            ],
            topology: GpuPrimitiveTopology::TriangleList,
            alpha_mode: GpuSolidAlphaMode::SourceOver,
            clip: surface.clip(),
            blend: GpuBlend::Normal,
            style: GpuSolidStyle::with_gamma(gamma.is_some_and(|gamma| !gamma.is_passthrough())),
        });
        return;
    }
    for y in clipped.y..clipped.y + clipped.height as i32 {
        for x in clipped.x..clipped.x + clipped.width as i32 {
            let destination = surface.get_pixel(x as u32, y as u32).unwrap_or_default();
            let output = match gamma {
                Some(gamma) if color.a == 255 => gamma_encode_fragment(color, gamma),
                Some(gamma) => gamma_blend_fragment_over(color, destination, gamma),
                None if color.a == 255 => color,
                None => blend_colors(color, destination),
            };
            let _ = surface.set_pixel(x as u32, y as u32, output);
        }
    }
}

/// Draws fallback `TextFont` glyphs through the same source-fragment gamma
/// path as `CStdDDraw::TextOut`. The temporary mask preserves glyph coverage
/// so gamma is applied before, rather than after, alpha blending.
#[allow(clippy::too_many_arguments)]
pub fn draw_text_with_gamma(
    font: &dyn TextFont,
    surface: &mut Surface,
    x: f32,
    y: f32,
    text: &str,
    size: f32,
    color: Color,
    gamma: Option<&clonk_graphics::GammaRamp>,
) {
    if let Some(renderer_config) = active_advanced_renderer_config()
        .filter(|config| config.changes_generic_textured_blit(0, true))
    {
        let metrics = font.measure_text(text, size);
        let padding = (size * 0.25).ceil() as i32 + 2;
        let mask_width = (metrics.width.ceil() as i32 + 2 * padding).max(1) as u32;
        let mask_height = (metrics.height.ceil() as i32 + 2 * padding).max(1) as u32;
        let target_x = x.floor() as i32 - padding;
        let target_y = y.floor() as i32 - padding;
        let mut mask = Surface::new(mask_width, mask_height, surface.format());
        font.draw_text(
            &mut mask,
            x - target_x as f32,
            y - target_y as f32,
            text,
            size,
            Color::opaque(255, 255, 255),
        );
        let image = ImageData::new(mask_width, mask_height, mask.pixels().to_vec());
        let modulation = (u32::from(255 - color.a) << 24)
            | (u32::from(color.r) << 16)
            | (u32::from(color.g) << 8)
            | u32::from(color.b);
        draw_image_source_configured_on_surface(
            surface,
            &GuiRect::new(
                target_x as f32,
                target_y as f32,
                mask_width as f32,
                mask_height as f32,
            ),
            &image,
            FloatSourceRect {
                x: 0.0,
                y: 0.0,
                width: mask_width as f32,
                height: mask_height as f32,
            },
            BlitSampling::Linear,
            gamma,
            BilinearBlend::AlphaOver,
            Some(modulation),
            renderer_config,
        );
        return;
    }
    let Some(gamma) = gamma else {
        font.draw_text(surface, x, y, text, size, color);
        return;
    };
    let metrics = font.measure_text(text, size);
    let padding = (size * 0.25).ceil() as i32 + 2;
    let mask_width = (metrics.width.ceil() as i32 + 2 * padding).max(1) as u32;
    let mask_height = (metrics.height.ceil() as i32 + 2 * padding).max(1) as u32;
    let target_x = x.floor() as i32 - padding;
    let target_y = y.floor() as i32 - padding;
    let mut mask = Surface::new(mask_width, mask_height, surface.format());
    font.draw_text(
        &mut mask,
        x - target_x as f32,
        y - target_y as f32,
        text,
        size,
        Color::new(255, 255, 255, color.a),
    );
    for mask_y in 0..mask.height() {
        let pixel_y = target_y + mask_y as i32;
        if pixel_y < 0 || pixel_y >= surface.height() as i32 {
            continue;
        }
        for mask_x in 0..mask.width() {
            let pixel_x = target_x + mask_x as i32;
            if pixel_x < 0 || pixel_x >= surface.width() as i32 {
                continue;
            }
            let Some(coverage) = mask.get_pixel(mask_x, mask_y).map(|pixel| pixel.a) else {
                continue;
            };
            if coverage == 0 {
                continue;
            }
            let _ = surface.blend_fragment(
                pixel_x as u32,
                pixel_y as u32,
                [
                    f32::from(color.r),
                    f32::from(color.g),
                    f32::from(color.b),
                    f32::from(coverage),
                ],
                Some(gamma),
            );
        }
    }
}

/// Textured `C4GFXBLIT_ADDITIVE`: owner/source modulation and the optional
/// R16 lookup have already selected the source fragment, then StdGL combines
/// it as `destination + source * source_alpha` and preserves framebuffer
/// alpha. C++ stores texture alpha as transparency and therefore spells the
/// equivalent source factor `GL_ONE_MINUS_SRC_ALPHA` (src/StdGL.cpp:1320-1324).
fn blend_fragment_additive(
    source: Color,
    destination: Color,
    gamma: Option<&clonk_graphics::GammaRamp>,
) -> Color {
    if source.a == 0 {
        return destination;
    }
    let alpha = f32::from(source.a) / 255.0;
    let add = |channel, source: u8, destination: u8| {
        store_channel(
            f32::from(destination) + sample_channel(gamma, channel, f32::from(source)) * alpha,
        )
    };
    Color::new(
        add(
            clonk_graphics::gamma::GammaChannel::Red,
            source.r,
            destination.r,
        ),
        add(
            clonk_graphics::gamma::GammaChannel::Green,
            source.g,
            destination.g,
        ),
        add(
            clonk_graphics::gamma::GammaChannel::Blue,
            source.b,
            destination.b,
        ),
        destination.a,
    )
}

pub(crate) fn composite_sprite_fragment(
    source: PreparedSpriteFragment,
    destination: Color,
    blit: SpriteBlitState,
    gamma: Option<&clonk_graphics::GammaRamp>,
) -> Color {
    if let PreparedSpriteFragment::Layers { base, overlay } = source {
        let destination = composite_sprite_fragment(base.into_fragment(), destination, blit, gamma);
        return composite_sprite_fragment(overlay.into_fragment(), destination, blit, gamma);
    }

    if let PreparedSpriteFragment::Legacy(source) = source {
        if blit.mode & C4GFXBLIT_ADDITIVE != 0 {
            return blend_fragment_additive(source, destination, gamma);
        }
        return match (source.a, gamma) {
            (255, Some(gamma)) => gamma_encode_fragment(source, gamma),
            (255, None) => source,
            (_, Some(gamma)) => gamma_blend_fragment_over(source, destination, gamma),
            (_, None) => blend_colors(source, destination),
        };
    }

    let PreparedSpriteFragment::Shader { rgb, alpha } = source else {
        unreachable!();
    };
    if alpha == 0.0 {
        return destination;
    }
    let alpha_factor = (alpha / 255.0).clamp(0.0, 1.0);
    let channel = |gamma_channel, source: f32, destination: u8| {
        let source = sample_channel(gamma, gamma_channel, source);
        if blit.mode & C4GFXBLIT_ADDITIVE != 0 {
            store_channel(f32::from(destination) + source * alpha_factor)
        } else {
            store_channel(source * alpha_factor + f32::from(destination) * (1.0 - alpha_factor))
        }
    };
    Color::new(
        channel(
            clonk_graphics::gamma::GammaChannel::Red,
            rgb[0],
            destination.r,
        ),
        channel(
            clonk_graphics::gamma::GammaChannel::Green,
            rgb[1],
            destination.g,
        ),
        channel(
            clonk_graphics::gamma::GammaChannel::Blue,
            rgb[2],
            destination.b,
        ),
        if blit.mode & C4GFXBLIT_ADDITIVE != 0 {
            destination.a
        } else {
            store_channel(alpha + f32::from(destination.a) * (1.0 - alpha_factor))
        },
    )
}

/// Stretches `image` into `rect` with GL_LINEAR-equivalent bilinear sampling
/// (tiled textures, GL_CLAMP_TO_EDGE) and normal alpha-over blending. `gamma`
/// mirrors the per-fragment gamma lookup of the C++ blit shader.
pub fn draw_image_bilinear(
    surface: &mut Surface,
    rect: &GuiRect,
    image: &ImageData,
    gamma: Option<&clonk_graphics::GammaRamp>,
) {
    draw_image_bilinear_impl(surface, rect, image, gamma, BilinearBlend::AlphaOver, None);
}

/// Draws a complete image through `C4Facet::DrawXFloat`: fractional target
/// edges are cropped inward to integer pixel boundaries and the same margins
/// are removed proportionally from the source before the regular tiled
/// bilinear blit.
pub fn draw_image_x_float(
    surface: &mut Surface,
    rect: &GuiRect,
    image: &ImageData,
    gamma: Option<&clonk_graphics::GammaRamp>,
) {
    let Some(crop) = draw_x_float_crop(rect, image.width(), image.height()) else {
        return;
    };
    draw_image_bilinear_source_impl(
        surface,
        &GuiRect::new(
            crop.target_x as f32,
            crop.target_y as f32,
            crop.target_width as f32,
            crop.target_height as f32,
        ),
        image,
        crop.source,
        gamma,
        BilinearBlend::AlphaOver,
        None,
    );
}

pub(crate) fn draw_image_bilinear_target<T: SurfaceDrawTarget + ?Sized>(
    surface: &mut T,
    rect: &GuiRect,
    image: &ImageData,
    gamma: Option<&clonk_graphics::GammaRamp>,
) {
    draw_image_bilinear_cpu(surface, rect, image, gamma, BilinearBlend::AlphaOver, None);
}

/// Scale-native font counterpart of [`draw_image_bilinear_target`] with the
/// centered horizontal shear installed by `CStdFont::DrawText` markup. The
/// coefficient is already projected into destination coordinates. Texture
/// filtering precedes font RGB/alpha modulation, matching the blit shader.
pub(crate) fn draw_image_bilinear_sheared_target<T: SurfaceDrawTarget + ?Sized>(
    surface: &mut T,
    rect: &GuiRect,
    image: &ImageData,
    gamma: Option<&clonk_graphics::GammaRamp>,
    shear: f32,
    modulation: [u8; 3],
    color_alpha: u8,
) {
    draw_image_bilinear_sheared_target_with_texture_size(
        surface,
        rect,
        image,
        gamma,
        shear,
        modulation,
        color_alpha,
        None,
    );
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_image_bilinear_sheared_target_on_texture<T: SurfaceDrawTarget + ?Sized>(
    surface: &mut T,
    rect: &GuiRect,
    image: &ImageData,
    gamma: Option<&clonk_graphics::GammaRamp>,
    shear: f32,
    modulation: [u8; 3],
    color_alpha: u8,
    physical_texture_size: i32,
) {
    draw_image_bilinear_sheared_target_with_texture_size(
        surface,
        rect,
        image,
        gamma,
        shear,
        modulation,
        color_alpha,
        Some(physical_texture_size),
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_image_bilinear_sheared_target_with_texture_size<T: SurfaceDrawTarget + ?Sized>(
    surface: &mut T,
    rect: &GuiRect,
    image: &ImageData,
    gamma: Option<&clonk_graphics::GammaRamp>,
    shear: f32,
    modulation: [u8; 3],
    color_alpha: u8,
    physical_texture_size: Option<i32>,
) {
    if rect.size.width <= 0.0
        || rect.size.height <= 0.0
        || image.width() == 0
        || image.height() == 0
        || !rect.origin.x.is_finite()
        || !rect.origin.y.is_finite()
        || !rect.size.width.is_finite()
        || !rect.size.height.is_finite()
        || !shear.is_finite()
    {
        return;
    }

    let renderer_config = active_advanced_renderer_config()
        .filter(|config| config.changes_generic_textured_blit(0, true));
    let offset = renderer_config.map_or(0.0, AdvancedRendererConfig::destination_offset);
    let (tx, ty) = (rect.origin.x + offset, rect.origin.y + offset);
    let (width, height) = (rect.size.width, rect.size.height);
    // CStdFont constructs markup transforms around the original facet center;
    // CStdGL adds BlitOffset to the submitted vertices afterwards.
    let center_y = rect.origin.y + height / 2.0;
    let top_shift = shear * (ty - center_y);
    let bottom_shift = shear * (ty + height - center_y);
    let surface_width = i32::try_from(surface.width()).unwrap_or(i32::MAX);
    let surface_height = i32::try_from(surface.height()).unwrap_or(i32::MAX);
    let x0 = ((tx + top_shift.min(bottom_shift) - 0.5).ceil() as i32).max(0);
    let x1 = ((tx + width + top_shift.max(bottom_shift) - 0.5).ceil() as i32).min(surface_width);
    let y0 = ((ty - 0.5).ceil() as i32).max(0);
    let y1 = ((ty + height - 0.5).ceil() as i32).min(surface_height);
    if x0 >= x1 || y0 >= y1 {
        return;
    }

    let scale_x = width / image.width() as f32;
    let scale_y = height / image.height() as f32;
    let tile_size = cpp_tex_size(image.width(), image.height()) as i32;
    let tiles_x = (image.width() as i32 - 1) / tile_size + 1;
    let tiles_y = (image.height() as i32 - 1) / tile_size + 1;
    let configured_source = FloatSourceRect {
        x: 0.0,
        y: 0.0,
        width: image.width() as f32,
        height: image.height() as f32,
    };
    let configured_blit = renderer_config.map(|renderer_config| SpriteBlitState {
        mode: renderer_config.masked_blit_mode(0),
        modulation: Some(
            (u32::from(255 - color_alpha) << 24)
                | (u32::from(modulation[0]) << 16)
                | (u32::from(modulation[1]) << 8)
                | u32::from(modulation[2]),
        ),
        fog_modulation: None,
        renderer_config,
    });
    for target_y in y0..y1 {
        let pixel_y = target_y as f32 + 0.5;
        let local_y = pixel_y - ty;
        if local_y < 0.0 || local_y >= height {
            continue;
        }
        for target_x in x0..x1 {
            let pixel_x = target_x as f32 + 0.5;
            let unsheared_x = pixel_x - shear * (pixel_y - center_y);
            let local_x = unsheared_x - tx;
            if local_x < 0.0 || local_x >= width {
                continue;
            }

            let source_x = local_x / scale_x;
            let source_y = local_y / scale_y;
            if let Some(blit) = configured_blit {
                let Some(source) = prepare_runtime_sprite_sample_with_texture_size(
                    image,
                    None,
                    &configured_source,
                    false,
                    source_x,
                    source_y,
                    BlitSampling::Linear,
                    None,
                    blit,
                    physical_texture_size,
                ) else {
                    continue;
                };
                if source.alpha() <= 0.0 {
                    continue;
                }
                let source = match source {
                    PreparedSpriteFragment::Legacy(color) => [
                        f32::from(color.r),
                        f32::from(color.g),
                        f32::from(color.b),
                        f32::from(color.a),
                    ],
                    PreparedSpriteFragment::Shader { rgb, alpha } => {
                        [rgb[0], rgb[1], rgb[2], alpha]
                    }
                    PreparedSpriteFragment::Layers { .. } => {
                        unreachable!("font sprites never carry owner-color layers")
                    }
                };
                let _ = surface.blend_fragment(
                    target_x as u32,
                    target_y as u32,
                    source,
                    gamma.filter(|gamma| !gamma.is_passthrough()),
                );
                continue;
            }
            let tile_x =
                ((source_x / tile_size as f32).floor() as i32).clamp(0, tiles_x - 1) * tile_size;
            let tile_y =
                ((source_y / tile_size as f32).floor() as i32).clamp(0, tiles_y - 1) * tile_size;
            let sample = bilinear_sample_tile(
                image,
                tile_x,
                tile_y,
                tile_size,
                source_x - 0.5 - tile_x as f32,
                source_y - 0.5 - tile_y as f32,
            );
            // The C++ blit shader adds inverted texture/modulation alpha.
            // Converted back to normal opacity, this subtracts modulation
            // transparency after filtering rather than multiplying alpha.
            let source_alpha = (sample[3] - f32::from(255 - color_alpha)).max(0.0);
            if source_alpha <= 0.0 {
                continue;
            }
            let _ = surface.blend_fragment(
                target_x as u32,
                target_y as u32,
                [
                    sample[0] * f32::from(modulation[0]) / 255.0,
                    sample[1] * f32::from(modulation[1]) / 255.0,
                    sample[2] * f32::from(modulation[2]) / 255.0,
                    source_alpha,
                ],
                gamma.filter(|gamma| !gamma.is_passthrough()),
            );
        }
    }
}

/// Owner-color surface counterpart of [`draw_image_bilinear`]. Filtering
/// precedes packed C4 `ColorDw` modulation, matching the DrawClr shader.
pub(crate) fn draw_image_bilinear_owner(
    surface: &mut Surface,
    rect: &GuiRect,
    image: &ImageData,
    owner_color: u32,
    gamma: Option<&clonk_graphics::GammaRamp>,
) {
    draw_image_bilinear_impl(
        surface,
        rect,
        image,
        gamma,
        BilinearBlend::AlphaOver,
        Some(owner_color),
    );
}

/// Stretches `image` into `rect` with bilinear sampling and additive blending
/// (`dst + src*alpha`, StdGL.cpp:908 `glBlendFunc(GL_SRC_ALPHA, GL_ONE)`), as
/// used for the GUI button focus highlight (C4GuiButton.cpp:94-98).
pub fn draw_image_bilinear_additive(
    surface: &mut Surface,
    rect: &GuiRect,
    image: &ImageData,
    gamma: Option<&clonk_graphics::GammaRamp>,
) {
    draw_image_bilinear_impl(surface, rect, image, gamma, BilinearBlend::Additive, None);
}

pub(crate) fn blend_colors(foreground: Color, background: Color) -> Color {
    if foreground.a == 0 {
        return background;
    }
    if foreground.a == 255 {
        return foreground;
    }

    let alpha = foreground.a as u16;
    let inv_alpha = 255u16 - alpha;
    let blend_channel =
        |fg: u8, bg: u8| -> u8 { ((fg as u16 * alpha + bg as u16 * inv_alpha) / 255) as u8 };
    let blended_alpha = alpha + (background.a as u16 * inv_alpha) / 255;

    Color::new(
        blend_channel(foreground.r, background.r),
        blend_channel(foreground.g, background.g),
        blend_channel(foreground.b, background.b),
        blended_alpha.min(255) as u8,
    )
}

pub(crate) fn draw_text(
    surface: &mut Surface,
    rect: &GuiRect,
    text: &str,
    color: Color,
    font_size: f32,
    padding: f32,
    font: &dyn TextFont,
) {
    let origin_x = rect.origin.x + padding;
    let origin_y = rect.origin.y + padding;
    font.draw_text(surface, origin_x, origin_y, text, font_size.max(1.0), color);
}

pub(crate) fn rect_contains(rect: SurfaceRect, point: GuiPoint, tolerance: f32) -> bool {
    let left = rect.x as f32 - tolerance;
    let top = rect.y as f32 - tolerance;
    let right = rect.x as f32 + rect.width as f32 + tolerance;
    let bottom = rect.y as f32 + rect.height as f32 + tolerance;
    point.x >= left && point.x < right && point.y >= top && point.y < bottom
}
