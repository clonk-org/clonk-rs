//! The shared software picture pipeline: object/definition menu symbols,
//! inventory picture caches, rank compositing, and script text-spec image
//! resolution, peeled verbatim out of the clonk-app monolith.

use clonk_engine::text_spec::{parse_text_spec, TextSpec, TextSpecIcon};
use clonk_engine::{DefinitionPictureImage, Engine, ObjectSnapshot, SimulationSnapshot};
use clonk_frontend::hud::GuiArtScale;
use clonk_frontend::{HudGraphics, InventoryPictureOverlay};
use clonk_graphics::{
    BlitMode, Color, PixelFormat, Point as SurfacePoint, Rect, Surface, Transform,
};
use clonk_gui::ImageData;

use crate::menu_images::{
    composite_software_picture_layer, copy_menu_image, copy_menu_image_aspect,
    copy_stretched_picture, menu_aspect_fit_rect, software_blit_menu_image,
};

#[derive(Clone, Copy, Default)]
pub struct ScriptTextSpecResources<'a> {
    pub gui_icons: Option<&'a ImageData>,
    pub gui_icons_extended: Option<&'a ImageData>,
    pub score: Option<&'a ImageData>,
}

pub fn resolve_portrait_text_spec(
    engine: &Engine,
    definition_id: &str,
    portrait_name: &str,
    color: Option<u32>,
    fallback_color: u32,
) -> Option<ImageData> {
    let image = engine.definition_named_portrait_graphics_image(definition_id, portrait_name)?;
    let color = color.unwrap_or(fallback_color);
    let width = image.width();
    let height = image.height();
    Some(ImageData::new(
        width,
        height,
        inventory_picture_pixels(&image, color),
    ))
}

/// GUIIcons.png — a 6x9 grid of `C4GUI_IconWdt` cells (`C4Gui.cpp:1090`).
const GUI_ICONS_NATIVE_SIZE: (u32, u32) = (240, 360);
const GUI_ICONS_NATIVE_CELL: u32 = 40;
/// GUIIcons2.png — a 4x5 grid of `C4GUI_IconExWdt` cells (`C4Gui.cpp:1091`).
const GUI_ICONS2_NATIVE_SIZE: (u32, u32) = (256, 320);
const GUI_ICONS2_NATIVE_CELL: u32 = 64;

/// Phase `phase` of GUIIcons.png, addressed through the sheet's own art scale
/// so a higher-resolution replacement keeps the oracle's grid and only grows
/// the cell (see [`GuiArtScale`]).
fn resolve_gui_icons_phase(image: &ImageData, phase: u32) -> Option<ImageData> {
    resolve_gui_icon_phase(
        image,
        phase,
        GuiArtScale::of(image, GUI_ICONS_NATIVE_SIZE).scale_up(GUI_ICONS_NATIVE_CELL),
    )
}

/// Phase `phase` of GUIIcons2.png; see [`resolve_gui_icons_phase`].
fn resolve_gui_icons2_phase(image: &ImageData, phase: u32) -> Option<ImageData> {
    resolve_gui_icon_phase(
        image,
        phase,
        GuiArtScale::of(image, GUI_ICONS2_NATIVE_SIZE).scale_up(GUI_ICONS2_NATIVE_CELL),
    )
}

/// The raw grid crop. `cell` is in *source* pixels, so callers that hold a
/// hard-coded 1x cell must project it through the sheet's [`GuiArtScale`].
pub fn resolve_gui_icon_phase(image: &ImageData, phase: u32, cell: u32) -> Option<ImageData> {
    let columns = image.width().checked_div(cell)?;
    (columns != 0).then_some(())?;
    crop_menu_image(
        image,
        phase.checked_rem(columns)?.checked_mul(cell)?,
        phase.checked_div(columns)?.checked_mul(cell)?,
        cell,
        cell,
    )
}

pub fn resolve_script_font_image(
    engine: &Engine,
    spec: &str,
    color: u32,
    resources: ScriptTextSpecResources<'_>,
) -> Option<ImageData> {
    match parse_text_spec(spec)? {
        TextSpec::Definition { id, phase } => {
            engine
                .definition_picture_phase_image(id, phase)
                .map(|image| {
                    ImageData::new(
                        image.width(),
                        image.height(),
                        inventory_picture_pixels(&image, color),
                    )
                })
        }
        TextSpec::Portrait {
            definition_id,
            portrait_name,
            color: portrait_color,
        } => {
            resolve_portrait_text_spec(engine, definition_id, portrait_name, portrait_color, color)
        }
        TextSpec::Icon(icon) => match icon {
            TextSpecIcon::Locked => resolve_gui_icons2_phase(resources.gui_icons_extended?, 13),
            TextSpecIcon::League => resolve_gui_icons2_phase(resources.gui_icons_extended?, 8),
            TextSpecIcon::GameRunning => resolve_gui_icons_phase(resources.gui_icons?, 30),
            TextSpecIcon::Lobby => resolve_gui_icons_phase(resources.gui_icons?, 31),
            TextSpecIcon::RuntimeJoin => resolve_gui_icons_phase(resources.gui_icons?, 32),
            TextSpecIcon::FairCrew => resolve_gui_icons2_phase(resources.gui_icons_extended?, 2),
            TextSpecIcon::Settlement => resources.score.cloned(),
        },
    }
}

pub fn apply_definition_owner_color(pixels: &mut [u8], mask: &[u8], owner: [u8; 3]) {
    if mask.len() == pixels.len() {
        for (pixel, overlay) in pixels.chunks_exact_mut(4).zip(mask.chunks_exact(4)) {
            let overlay_alpha = u32::from(overlay[3]);
            let base_alpha = u32::from(pixel[3]);
            let inverse = 255 - overlay_alpha;
            let output_alpha_weight = overlay_alpha * 255 + base_alpha * inverse;
            if output_alpha_weight == 0 {
                pixel.copy_from_slice(&[0, 0, 0, 0]);
                continue;
            }
            for (channel_index, (channel, owner)) in pixel[..3].iter_mut().zip(owner).enumerate() {
                let tinted = u32::from(overlay[channel_index]) * u32::from(owner) / 255;
                let premultiplied =
                    tinted * overlay_alpha * 255 + u32::from(*channel) * base_alpha * inverse;
                *channel = (premultiplied / output_alpha_weight) as u8;
            }
            pixel[3] = (output_alpha_weight / 255).min(255) as u8;
        }
        return;
    }

    for (index, mask_value) in mask.iter().copied().enumerate() {
        let offset = index * 4;
        let Some(pixel) = pixels.get_mut(offset..offset + 4) else {
            break;
        };
        let mask = u16::from(mask_value);
        if mask == 0 {
            continue;
        }
        let inverse = 255_u16 - mask;
        for (channel, owner) in pixel[..3].iter_mut().zip(owner) {
            *channel = ((u16::from(*channel) * inverse + u16::from(owner) * mask) / 255) as u8;
        }
    }
}

pub fn apply_default_menu_owner_color(pixels: &mut [u8], mask: &[u8]) {
    // C4Def::Picture2Facet calls Graphics.GetBitmap(0); C4Surface::SetClr
    // maps zero to 0xff, the engine's default blue owner color
    // (C4Def.cpp:1374-1378; C4DefGraphics.h:49; C4Surface.h:110).
    apply_definition_owner_color(pixels, mask, [0, 0, 255]);
}

pub fn definition_menu_picture(image: DefinitionPictureImage) -> ImageData {
    let width = image.width();
    let height = image.height();
    let mask = image.color_mask();
    let pixels = image.into_pixels();
    let Some(mask) = mask else {
        return ImageData::from_arc(width, height, pixels);
    };
    let mut pixels = pixels.to_vec();
    apply_default_menu_owner_color(&mut pixels, &mask);
    ImageData::new(width, height, pixels)
}

pub fn inventory_object_picture(engine: &Engine, object: &ObjectSnapshot) -> Option<ImageData> {
    inventory_object_picture_with_allowed_modes(engine, object, 15)
}

pub fn inventory_object_picture_with_allowed_modes(
    engine: &Engine,
    object: &ObjectSnapshot,
    allowed_blit_modes: u32,
) -> Option<ImageData> {
    let image = engine.object_picture_image(object)?;
    compose_inventory_picture_with_allowed_modes(
        image,
        engine.object_picture_overlay_images(object),
        object.color,
        object.color_modulation,
        object.blit_mode,
        allowed_blit_modes,
    )
}

pub fn inventory_object_picture_layers(
    engine: &Engine,
    object: &ObjectSnapshot,
    renderer_config: clonk_frontend::AdvancedRendererConfig,
) -> Option<PreparedInventoryPicture> {
    prepare_inventory_picture_with_renderer_config(
        engine.object_picture_image(object)?,
        engine.object_picture_overlay_images(object),
        object.color,
        object.color_modulation,
        object.blit_mode,
        renderer_config,
    )
}

pub fn cached_menu_object_picture(
    engine: &Engine,
    object: &clonk_engine::ObjectMenuPictureSnapshot,
    force_owned: bool,
) -> Option<ImageData> {
    cached_menu_object_picture_with_allowed_modes(engine, object, force_owned, 15)
}

pub fn cached_menu_object_picture_with_allowed_modes(
    engine: &Engine,
    object: &clonk_engine::ObjectMenuPictureSnapshot,
    force_owned: bool,
    allowed_blit_modes: u32,
) -> Option<ImageData> {
    let image = engine.object_menu_picture_image(object)?;
    if !force_owned
        && object.color_modulation == 0
        && object.blit_mode == 0
        && object.graphics_overlays.is_empty()
    {
        let width = image.width();
        let height = image.height();
        return Some(ImageData::new(
            width,
            height,
            inventory_picture_pixels(&image, object.color),
        ));
    }
    compose_owned_menu_picture_with_allowed_modes(
        image,
        engine.object_menu_picture_overlay_images(object),
        object,
        allowed_blit_modes,
    )
}

pub fn crop_menu_image(
    image: &ImageData,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
) -> Option<ImageData> {
    if width == 0
        || height == 0
        || x.checked_add(width)? > image.width()
        || y.checked_add(height)? > image.height()
    {
        return None;
    }
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
    for row in y..y + height {
        let start = ((row * image.width() + x) * 4) as usize;
        let end = start + width as usize * 4;
        pixels.extend_from_slice(image.pixels().get(start..end)?);
    }
    Some(ImageData::new(width, height, pixels))
}

pub fn menu_rank_picture(
    engine: &Engine,
    hud: &HudGraphics,
    definition_id: &str,
    rank: i32,
) -> Option<ImageData> {
    let custom = engine
        .definition_rank_symbols_image(definition_id)
        .map(definition_menu_picture);
    let symbols = custom.as_ref().or(hud.rank.as_ref())?;
    let cell = symbols.height();
    if cell == 0 || symbols.width() < cell {
        return None;
    }
    let total_count = (symbols.width() / cell).max(1);
    let base_count = if custom.is_some() {
        engine
            .definition_rank_symbol_count(definition_id)
            .unwrap_or(total_count)
            .clamp(1, total_count)
    } else {
        total_count
    };
    let rank = rank.max(0) as u32;
    let mut base_rank = rank % base_count;
    let extension_level = rank / base_count;
    if extension_level == 0 {
        return crop_menu_image(symbols, base_rank * cell, 0, cell, cell);
    }

    let extension = if total_count > base_count {
        let requested = extension_level.saturating_sub(1).saturating_add(base_count);
        let phase = if requested >= total_count {
            base_rank = base_count - 1;
            total_count - 1
        } else {
            requested
        };
        crop_menu_image(symbols, phase * cell, 0, cell, cell)
    } else {
        hud.captain.clone()
    };
    let base = crop_menu_image(symbols, base_rank * cell, 0, cell, cell)?;
    let mut composed = Surface::new(cell, cell, PixelFormat::Rgba8888);
    copy_menu_image(&mut composed, &base, Rect::new(0, 0, cell, cell))?;
    if let Some(extension) = extension {
        let size = cell.saturating_mul(2) / 3;
        software_blit_menu_image(
            &mut composed,
            &extension,
            Rect::new(0, 0, size, size),
            BlitMode::Normal,
        )?;
    }
    Some(ImageData::new(cell, cell, composed.pixels().to_vec()))
}

pub fn menu_object_rank_picture(
    engine: &Engine,
    hud: &HudGraphics,
    object: &clonk_engine::ObjectMenuPictureSnapshot,
    object_picture: ImageData,
    menu_style: i32,
) -> Option<ImageData> {
    menu_object_rank_picture_with_item_height(engine, hud, object, object_picture, menu_style, None)
}

/// A Context row sizes its ObjectRank facet from the menu's live
/// `GetItemHeight()`, not from `GetSymbolSize()`: `fctSymbol.Create(H * 2, H)`
/// with the object left and the rank right (C4Script.cpp:1717-1728). Every
/// other style keeps the add-time symbol size.
pub fn menu_object_rank_picture_with_item_height(
    engine: &Engine,
    hud: &HudGraphics,
    object: &clonk_engine::ObjectMenuPictureSnapshot,
    object_picture: ImageData,
    menu_style: i32,
    item_height: Option<i32>,
) -> Option<ImageData> {
    let resolved = match (menu_style, item_height) {
        (1, Some(height)) => height,
        _ => object.symbol_size,
    };
    let side = u32::try_from(resolved.max(1)).ok()?;
    let width = if menu_style == 1 {
        side.saturating_mul(2)
    } else {
        side
    };
    let mut composed = Surface::new(width, side, PixelFormat::Rgba8888);
    copy_menu_image_aspect(&mut composed, &object_picture, Rect::new(0, 0, side, side))?;
    if let Some(rank) = object.rank {
        if let Some(rank_picture) = menu_rank_picture(engine, hud, &object.definition_id, rank) {
            let rank_width = rank_picture.width().min(side);
            let rank_height = rank_picture.height().min(side);
            let x = if menu_style == 1 {
                side as i32
            } else {
                side.saturating_sub(rank_width) as i32
            };
            software_blit_menu_image(
                &mut composed,
                &rank_picture,
                Rect::new(x, 0, rank_width, rank_height),
                BlitMode::Normal,
            )?;
        }
    }
    Some(ImageData::new(width, side, composed.pixels().to_vec()))
}

pub fn compose_owned_menu_picture(
    image: clonk_engine::DefinitionPictureImage,
    overlays: Vec<(
        clonk_engine::ObjectGraphicsOverlay,
        clonk_engine::DefinitionPictureImage,
    )>,
    object: &clonk_engine::ObjectMenuPictureSnapshot,
) -> Option<ImageData> {
    compose_owned_menu_picture_with_allowed_modes(image, overlays, object, 15)
}

pub fn compose_owned_menu_picture_with_allowed_modes(
    image: clonk_engine::DefinitionPictureImage,
    overlays: Vec<(
        clonk_engine::ObjectGraphicsOverlay,
        clonk_engine::DefinitionPictureImage,
    )>,
    object: &clonk_engine::ObjectMenuPictureSnapshot,
    allowed_blit_modes: u32,
) -> Option<ImageData> {
    let side = u32::try_from(object.symbol_size.max(1)).ok()?;
    let destination = Rect::new(0, 0, side, side);
    let effective_object_blit_mode = object.blit_mode & allowed_blit_modes;
    let object_mode = inventory_blit_mode(effective_object_blit_mode);
    let object_modulation = inventory_modulation(object.color_modulation, object.blit_mode);
    let base_pixels = prepare_owned_menu_definition_pixels(
        &image,
        object.color,
        object_modulation,
        effective_object_blit_mode,
    )?;
    let base = Surface::from_bytes(
        image.width(),
        image.height(),
        PixelFormat::Rgba8888,
        base_pixels,
    )
    .ok()?;
    let mut composed = Surface::new(side, side, PixelFormat::Rgba8888);
    copy_stretched_picture(
        &base,
        Rect::new(0, 0, image.width(), image.height()),
        &mut composed,
        menu_aspect_fit_rect(image.width(), image.height(), destination)?,
    )?;

    for (overlay, image) in overlays {
        let inherits_parent = overlay.blit_mode == 256;
        let effective_blit_mode = if inherits_parent {
            object.blit_mode
        } else {
            overlay.blit_mode
        };
        let allowed_blit_mode = effective_blit_mode & allowed_blit_modes;
        let mode = if inherits_parent {
            object_mode
        } else {
            inventory_blit_mode(allowed_blit_mode)
        };
        let modulation = if inherits_parent {
            object_modulation
        } else if overlay.color_modulation == 0x00ff_ffff {
            None
        } else {
            inventory_modulation(overlay.color_modulation, overlay.blit_mode)
        };
        let overlay_pixels = prepare_owned_menu_definition_pixels(
            &image,
            object.color,
            modulation,
            allowed_blit_mode,
        )?;
        let overlay_surface = Surface::from_bytes(
            image.width(),
            image.height(),
            PixelFormat::Rgba8888,
            overlay_pixels,
        )
        .ok()?;
        let source_rect = Rect::new(0, 0, image.width(), image.height());
        let fitted = menu_aspect_fit_rect(image.width(), image.height(), destination)?;
        let mut layer = Surface::new(side, side, PixelFormat::Rgba8888);
        let mut coverage_source =
            Surface::new(image.width(), image.height(), PixelFormat::Rgba8888);
        coverage_source.fill(Color::opaque(255, 255, 255));
        let mut coverage = Surface::new(side, side, PixelFormat::Rgba8888);
        if let Some(transform) = overlay.transform {
            let mut stretched = Surface::new(side, side, PixelFormat::Rgba8888);
            copy_stretched_picture(&overlay_surface, source_rect, &mut stretched, fitted)?;
            let mut stretched_coverage = Surface::new(side, side, PixelFormat::Rgba8888);
            copy_stretched_picture(
                &coverage_source,
                source_rect,
                &mut stretched_coverage,
                fitted,
            )?;
            let scale_factor = side as f32 / 35.0;
            let center = side as f32 / 2.0;
            let matrix =
                centered_picture_transform(transform.matrix(), scale_factor, center, center);
            layer
                .copy_transformed(&stretched, destination, SurfacePoint::new(0, 0), &matrix)
                .ok()?;
            coverage
                .copy_transformed(
                    &stretched_coverage,
                    destination,
                    SurfacePoint::new(0, 0),
                    &matrix,
                )
                .ok()?;
        } else {
            copy_stretched_picture(&overlay_surface, source_rect, &mut layer, fitted)?;
            copy_stretched_picture(&coverage_source, source_rect, &mut coverage, fitted)?;
        }
        composite_software_picture_layer(&mut composed, &layer, &coverage, mode)?;
    }

    Some(ImageData::new(side, side, composed.pixels().to_vec()))
}

pub struct PreparedInventoryPicture {
    pub base: ImageData,
    pub overlays: Vec<InventoryPictureOverlay>,
}

pub fn prepare_inventory_picture(
    image: clonk_engine::DefinitionPictureImage,
    overlays: Vec<(
        clonk_engine::ObjectGraphicsOverlay,
        clonk_engine::DefinitionPictureImage,
    )>,
    object_color: u32,
    color_modulation: u32,
    blit_mode: u32,
) -> Option<PreparedInventoryPicture> {
    prepare_inventory_picture_with_allowed_modes(
        image,
        overlays,
        object_color,
        color_modulation,
        blit_mode,
        15,
    )
}

pub fn prepare_inventory_picture_with_allowed_modes(
    image: clonk_engine::DefinitionPictureImage,
    overlays: Vec<(
        clonk_engine::ObjectGraphicsOverlay,
        clonk_engine::DefinitionPictureImage,
    )>,
    object_color: u32,
    color_modulation: u32,
    blit_mode: u32,
    allowed_blit_modes: u32,
) -> Option<PreparedInventoryPicture> {
    prepare_inventory_picture_with_renderer_config(
        image,
        overlays,
        object_color,
        color_modulation,
        blit_mode,
        clonk_frontend::AdvancedRendererConfig {
            allowed_blit_modes,
            ..clonk_frontend::AdvancedRendererConfig::DEFAULT
        },
    )
}

pub fn prepare_inventory_picture_with_renderer_config(
    image: clonk_engine::DefinitionPictureImage,
    overlays: Vec<(
        clonk_engine::ObjectGraphicsOverlay,
        clonk_engine::DefinitionPictureImage,
    )>,
    object_color: u32,
    color_modulation: u32,
    blit_mode: u32,
    renderer_config: clonk_frontend::AdvancedRendererConfig,
) -> Option<PreparedInventoryPicture> {
    let width = image.width();
    let height = image.height();
    let allowed_blit_modes = renderer_config.allowed_blit_modes;
    let effective_object_blit_mode = blit_mode & allowed_blit_modes;
    let object_mode = inventory_blit_mode(effective_object_blit_mode);
    let object_modulation = inventory_modulation(color_modulation, blit_mode);
    let (base_pixels, owner_pixels) = prepare_inventory_definition_layers(
        &image,
        object_color,
        object_modulation,
        effective_object_blit_mode,
        renderer_config,
    )?;
    let base = ImageData::new(width, height, base_pixels);
    let mut prepared_overlays =
        Vec::with_capacity(overlays.len() * 2 + usize::from(owner_pixels.is_some()));
    if let Some(owner_pixels) = owner_pixels {
        prepared_overlays.push(InventoryPictureOverlay {
            picture: ImageData::new(width, height, owner_pixels),
            additive: matches!(object_mode, BlitMode::Additive | BlitMode::Mod2Additive),
        });
    }
    for (overlay, image) in overlays {
        let inherits_parent = overlay.blit_mode == 256;
        let effective_blit_mode = if inherits_parent {
            blit_mode
        } else {
            overlay.blit_mode
        };
        let allowed_blit_mode = effective_blit_mode & allowed_blit_modes;
        let mode = if inherits_parent {
            object_mode
        } else {
            inventory_blit_mode(allowed_blit_mode)
        };
        let modulation = if inherits_parent {
            object_modulation
        } else if overlay.color_modulation == 0x00ff_ffff {
            None
        } else {
            inventory_modulation(overlay.color_modulation, overlay.blit_mode)
        };
        let (overlay_pixels, owner_pixels) = prepare_inventory_definition_layers(
            &image,
            object_color,
            modulation,
            allowed_blit_mode,
            renderer_config,
        )?;
        let source_rect = Rect::new(0, 0, image.width(), image.height());
        let destination_rect = Rect::new(0, 0, width, height);
        for pixels in std::iter::once(overlay_pixels).chain(owner_pixels) {
            let overlay_surface =
                Surface::from_bytes(image.width(), image.height(), PixelFormat::Rgba8888, pixels)
                    .ok()?;
            let mut layer = Surface::new(width, height, PixelFormat::Rgba8888);
            if let Some(transform) = overlay.transform {
                let scale_factor = width as f32 / 64.0;
                let center_x = width as f32 / 2.0;
                let center_y = height as f32 / 2.0;
                let matrix = centered_picture_transform(
                    transform.matrix(),
                    scale_factor,
                    center_x,
                    center_y,
                );
                let mut stretched = Surface::new(width, height, PixelFormat::Rgba8888);
                copy_stretched_picture(
                    &overlay_surface,
                    source_rect,
                    &mut stretched,
                    destination_rect,
                )?;
                layer
                    .copy_transformed(
                        &stretched,
                        destination_rect,
                        SurfacePoint::new(0, 0),
                        &matrix,
                    )
                    .ok()?;
            } else {
                copy_stretched_picture(
                    &overlay_surface,
                    source_rect,
                    &mut layer,
                    destination_rect,
                )?;
            }
            prepared_overlays.push(InventoryPictureOverlay {
                picture: ImageData::new(width, height, layer.pixels().to_vec()),
                additive: matches!(mode, BlitMode::Additive | BlitMode::Mod2Additive),
            });
        }
    }

    Some(PreparedInventoryPicture {
        base,
        overlays: prepared_overlays,
    })
}

pub fn compose_inventory_picture(
    image: clonk_engine::DefinitionPictureImage,
    overlays: Vec<(
        clonk_engine::ObjectGraphicsOverlay,
        clonk_engine::DefinitionPictureImage,
    )>,
    object_color: u32,
    color_modulation: u32,
    blit_mode: u32,
) -> Option<ImageData> {
    compose_inventory_picture_with_allowed_modes(
        image,
        overlays,
        object_color,
        color_modulation,
        blit_mode,
        15,
    )
}

pub fn compose_inventory_picture_with_allowed_modes(
    image: clonk_engine::DefinitionPictureImage,
    overlays: Vec<(
        clonk_engine::ObjectGraphicsOverlay,
        clonk_engine::DefinitionPictureImage,
    )>,
    object_color: u32,
    color_modulation: u32,
    blit_mode: u32,
    allowed_blit_modes: u32,
) -> Option<ImageData> {
    let prepared = prepare_inventory_picture_with_allowed_modes(
        image,
        overlays,
        object_color,
        color_modulation,
        blit_mode,
        allowed_blit_modes,
    )?;
    if prepared.overlays.is_empty() {
        return Some(prepared.base);
    }

    // Non-HUD consumers still require one flattened ImageData. The viewport
    // inventory retains `prepared.overlays` and draws each blend mode directly.
    let mut composed = Surface::from_bytes(
        prepared.base.width(),
        prepared.base.height(),
        PixelFormat::Rgba8888,
        prepared.base.pixels().to_vec(),
    )
    .ok()?;
    for overlay in prepared.overlays {
        let layer = Surface::from_bytes(
            overlay.picture.width(),
            overlay.picture.height(),
            PixelFormat::Rgba8888,
            overlay.picture.pixels().to_vec(),
        )
        .ok()?;
        let mode = if overlay.additive {
            BlitMode::Additive
        } else {
            BlitMode::Normal
        };
        composite_inventory_picture_layer(&mut composed, &layer, mode)?;
    }

    Some(ImageData::new(
        composed.width(),
        composed.height(),
        composed.pixels().to_vec(),
    ))
}

/// C4GraphicsOverlay::DrawPicture first rescales the transform's translation
/// into the destination picture's coordinate system and then applies
/// C4DrawTransform::SetTransformAt at the picture center
/// (src/C4DefGraphics.cpp:849-855; src/C4Facet.cpp:446-456).
pub fn centered_picture_transform(
    mut matrix: [f32; 9],
    translation_scale: f32,
    center_x: f32,
    center_y: f32,
) -> Transform {
    matrix[2] *= translation_scale;
    matrix[5] *= translation_scale;

    let a = matrix[0] + matrix[6] * center_x;
    let b = matrix[1] + matrix[7] * center_x;
    let d = matrix[3] + matrix[6] * center_y;
    let e = matrix[4] + matrix[7] * center_y;
    Transform::set(
        a,
        b,
        matrix[2] - a * center_x - b * center_y + matrix[8] * center_x,
        d,
        e,
        matrix[5] - d * center_x - e * center_y + matrix[8] * center_y,
        matrix[6],
        matrix[7],
        matrix[8] - matrix[6] * center_x - matrix[7] * center_y,
    )
}

pub fn object_menu_item_picture(
    engine: &Engine,
    snapshot: &SimulationSnapshot,
    item: &clonk_engine::ObjectMenuItem,
    definition_color: u32,
    hud: &HudGraphics,
    menu_style: i32,
) -> Option<ImageData> {
    object_menu_item_picture_with_text_spec_resources(
        engine,
        snapshot,
        item,
        definition_color,
        hud,
        menu_style,
        ScriptTextSpecResources::default(),
    )
}

pub fn object_menu_item_picture_with_text_spec_resources(
    engine: &Engine,
    snapshot: &SimulationSnapshot,
    item: &clonk_engine::ObjectMenuItem,
    definition_color: u32,
    hud: &HudGraphics,
    menu_style: i32,
    text_spec_resources: ScriptTextSpecResources<'_>,
) -> Option<ImageData> {
    object_menu_item_picture_with_renderer_modes(
        engine,
        snapshot,
        item,
        definition_color,
        hud,
        menu_style,
        text_spec_resources,
        15,
    )
}

// Keep the parity-facing render inputs explicit: grouping the borrowed text resources
// and renderer capability mask would only hide this boundary's independent inputs.
#[allow(clippy::too_many_arguments)]
pub fn object_menu_item_picture_with_renderer_modes(
    engine: &Engine,
    snapshot: &SimulationSnapshot,
    item: &clonk_engine::ObjectMenuItem,
    definition_color: u32,
    hud: &HudGraphics,
    menu_style: i32,
    text_spec_resources: ScriptTextSpecResources<'_>,
    allowed_blit_modes: u32,
) -> Option<ImageData> {
    object_menu_item_picture_with_context_height(
        engine,
        snapshot,
        item,
        definition_color,
        hud,
        menu_style,
        text_spec_resources,
        allowed_blit_modes,
        None,
    )
}

/// `context_item_height` is the Context menu's resolved `ItemHeight`, which
/// only the layout knows; it sizes ObjectRank rows (C4Script.cpp:1717-1721).
#[allow(clippy::too_many_arguments)]
pub fn object_menu_item_picture_with_context_height(
    engine: &Engine,
    snapshot: &SimulationSnapshot,
    item: &clonk_engine::ObjectMenuItem,
    definition_color: u32,
    hud: &HudGraphics,
    menu_style: i32,
    text_spec_resources: ScriptTextSpecResources<'_>,
    allowed_blit_modes: u32,
    context_item_height: Option<i32>,
) -> Option<ImageData> {
    if let (true, Some(picture)) = (
        matches!(&item.image, clonk_engine::ObjectMenuImage::Definition),
        item.picture_snapshot.as_ref(),
    ) {
        // Native C4ObjectMenu rows own the Picture2Facet surface
        // created during refill; do not re-resolve their source object
        // from the frame snapshot (C4ObjectMenu.cpp:194-199,
        // 311-313,350-372).
        return cached_menu_object_picture_with_allowed_modes(
            engine,
            picture,
            false,
            allowed_blit_modes,
        );
    }
    match &item.image {
        clonk_engine::ObjectMenuImage::None => None,
        clonk_engine::ObjectMenuImage::Object { object } => item
            .picture_snapshot
            .as_ref()
            .and_then(|picture| {
                cached_menu_object_picture_with_allowed_modes(
                    engine,
                    picture,
                    false,
                    allowed_blit_modes,
                )
            })
            .or_else(|| {
                // Backward compatibility for snapshots written before
                // add-time picture descriptors were retained.
                if item.presentation_definition_id.is_none() {
                    snapshot.object(*object).and_then(|object| {
                        inventory_object_picture_with_allowed_modes(
                            engine,
                            object,
                            allowed_blit_modes,
                        )
                    })
                } else {
                    None
                }
            }),
        clonk_engine::ObjectMenuImage::ObjectRank { object } => item
            .picture_snapshot
            .as_ref()
            .and_then(|picture| {
                cached_menu_object_picture_with_allowed_modes(
                    engine,
                    picture,
                    true,
                    allowed_blit_modes,
                )
                .and_then(|object_picture| {
                    menu_object_rank_picture_with_item_height(
                        engine,
                        hud,
                        picture,
                        object_picture,
                        menu_style,
                        context_item_height,
                    )
                })
            })
            .or_else(|| {
                if item.presentation_definition_id.is_none() {
                    snapshot.object(*object).and_then(|object| {
                        inventory_object_picture_with_allowed_modes(
                            engine,
                            object,
                            allowed_blit_modes,
                        )
                    })
                } else {
                    None
                }
            }),
        clonk_engine::ObjectMenuImage::TextSpec { spec, color } => {
            resolve_script_font_image(engine, spec, *color, text_spec_resources)
        }
        clonk_engine::ObjectMenuImage::Rank { rank } => {
            let definition_id = item
                .presentation_definition_id
                .as_deref()
                .unwrap_or(&item.item_id);
            menu_rank_picture(engine, hud, definition_id, *rank)
        }
        clonk_engine::ObjectMenuImage::Definition
        | clonk_engine::ObjectMenuImage::Indexed { .. }
        | clonk_engine::ObjectMenuImage::Color { .. }
        | clonk_engine::ObjectMenuImage::IndexedColor { .. } => match item.picture_object {
            Some(object_id) => snapshot.object(object_id).and_then(|object| {
                inventory_object_picture_with_allowed_modes(engine, object, allowed_blit_modes)
            }),
            None => {
                let definition_id = item
                    .presentation_definition_id
                    .as_deref()
                    .unwrap_or(&item.item_id);
                let image = match &item.image {
                    clonk_engine::ObjectMenuImage::Indexed { index }
                    | clonk_engine::ObjectMenuImage::IndexedColor { index, .. } => {
                        engine.definition_picture_phase_image(definition_id, *index)
                    }
                    _ => engine
                        .definition_picture_phase_image(definition_id, 0)
                        .or_else(|| engine.definition_picture_image(definition_id)),
                };
                image.map(|image| {
                    let recipe_color = match &item.image {
                        clonk_engine::ObjectMenuImage::Color { color }
                        | clonk_engine::ObjectMenuImage::IndexedColor { color, .. } => *color,
                        _ => definition_color,
                    };
                    if recipe_color == 0 {
                        definition_menu_picture(image)
                    } else {
                        let width = image.width();
                        let height = image.height();
                        let pixels = inventory_picture_pixels(&image, recipe_color);
                        ImageData::new(width, height, pixels)
                    }
                })
            }
        },
    }
}

pub fn inventory_picture_pixels(
    image: &clonk_engine::DefinitionPictureImage,
    object_color: u32,
) -> Vec<u8> {
    let mut pixels = image.pixels().to_vec();
    if let Some(mask) = image.color_mask() {
        // C4Surface::SetClr maps zero to 0xff before applying a
        // ColorByOwner bitmap (src/C4Surface.h:110).
        let owner = if object_color == 0 {
            0xff
        } else {
            object_color
        };
        let owner_color = [
            ((owner >> 16) & 0xff) as u8,
            ((owner >> 8) & 0xff) as u8,
            (owner & 0xff) as u8,
        ];
        apply_definition_owner_color(&mut pixels, &mask, owner_color);
    }
    pixels
}

pub fn inventory_owner_modulation(object_color: u32) -> Color {
    let owner = if object_color == 0 {
        0xff
    } else {
        object_color
    };
    Color::new(
        ((owner >> 16) & 0xff) as u8,
        ((owner >> 8) & 0xff) as u8,
        (owner & 0xff) as u8,
        ((owner >> 24) & 0xff) as u8,
    )
}

pub fn combine_inventory_modulations(owner: Color, global: Color) -> Color {
    let multiply =
        |left: u8, right: u8| -> u8 { ((u16::from(left) * u16::from(right)) >> 8) as u8 };
    let screen_transparency = |left: u8, right: u8| -> u8 {
        let product = (u16::from(left) * u16::from(right)) >> 8;
        (u16::from(left) + u16::from(right) - product).min(255) as u8
    };
    Color::new(
        multiply(owner.r, global.r),
        multiply(owner.g, global.g),
        multiply(owner.b, global.b),
        screen_transparency(owner.a, global.a),
    )
}

pub fn inventory_owner_blit_mode(raw: u32) -> BlitMode {
    match (raw & 1 != 0, raw & 8 != 0) {
        (false, false) => BlitMode::Normal,
        (true, false) => BlitMode::Additive,
        (false, true) => BlitMode::Mod2,
        (true, true) => BlitMode::Mod2Additive,
    }
}

pub fn prepared_inventory_alpha(
    source: u8,
    modulation: Color,
    mode: BlitMode,
    renderer_config: clonk_frontend::AdvancedRendererConfig,
) -> u8 {
    let live_mod2 = matches!(mode, BlitMode::Mod2 | BlitMode::Mod2Additive)
        && modulation != Color::transparent();
    if live_mod2 {
        if renderer_config.shader {
            source
        } else {
            source.saturating_sub(modulation.a)
        }
    } else if !renderer_config.shader && renderer_config.no_alpha_add {
        // Exact packed C4 white keeps GL_REPLACE. Every actually modulated
        // NoAlphaAdd draw instead uses GL_MODULATE but ORs dwModMask's
        // 0xff000000 into the primary C4-transparency alpha first; multiplying
        // the texture transparency by 255 likewise preserves source opacity.
        source
    } else {
        source.saturating_sub(modulation.a)
    }
}

pub fn prepare_inventory_owner_pixels(
    pixels: &mut [u8],
    modulation: Color,
    mode: BlitMode,
    renderer_config: clonk_frontend::AdvancedRendererConfig,
) {
    let mod2 = matches!(mode, BlitMode::Mod2 | BlitMode::Mod2Additive)
        && modulation != Color::transparent();
    for pixel in pixels.chunks_exact_mut(4) {
        let source = Color::new(pixel[0], pixel[1], pixel[2], pixel[3]);
        let mut prepared = if mod2 {
            let channel = |source: u8, modulation: u8| -> u8 {
                (2 * i32::from(source) + 2 * i32::from(modulation) - 255).clamp(0, 255) as u8
            };
            Color::new(
                channel(source.r, modulation.r),
                channel(source.g, modulation.g),
                channel(source.b, modulation.b),
                source.a,
            )
        } else {
            let channel = |source: u8, modulation: u8| -> u8 {
                (u16::from(source) * u16::from(modulation) / 255) as u8
            };
            Color::new(
                channel(source.r, modulation.r),
                channel(source.g, modulation.g),
                channel(source.b, modulation.b),
                source.a,
            )
        };
        prepared.a = prepared_inventory_alpha(source.a, modulation, mode, renderer_config);
        pixel.copy_from_slice(&[prepared.r, prepared.g, prepared.b, prepared.a]);
    }
}

pub fn prepare_inventory_definition_layers(
    image: &clonk_engine::DefinitionPictureImage,
    object_color: u32,
    global_modulation: Option<Color>,
    raw_blit_mode: u32,
    renderer_config: clonk_frontend::AdvancedRendererConfig,
) -> Option<(Vec<u8>, Option<Vec<u8>>)> {
    let mut base = image.pixels().to_vec();
    let Some(owner_pixels) = image.color_mask().filter(|mask| mask.len() == base.len()) else {
        let mut flattened = inventory_picture_pixels(image, object_color);
        if let Some(modulation) = global_modulation {
            prepare_inventory_pixels(
                &mut flattened,
                modulation,
                inventory_blit_mode(raw_blit_mode),
                renderer_config,
            );
        } else if raw_blit_mode & 2 != 0 {
            prepare_inventory_pixels(
                &mut flattened,
                Color::new(255, 255, 255, 0),
                inventory_blit_mode(raw_blit_mode),
                renderer_config,
            );
        }
        return Some((flattened, None));
    };

    if let Some(modulation) = global_modulation {
        prepare_inventory_pixels(
            &mut base,
            modulation,
            inventory_blit_mode(raw_blit_mode),
            renderer_config,
        );
    } else if raw_blit_mode & 2 != 0 {
        prepare_inventory_pixels(
            &mut base,
            Color::new(255, 255, 255, 0),
            inventory_blit_mode(raw_blit_mode),
            renderer_config,
        );
    }
    let mut owner_pixels = owner_pixels.to_vec();
    let mut owner_modulation = inventory_owner_modulation(object_color);
    if raw_blit_mode & 4 == 0 {
        if let Some(global) = global_modulation {
            owner_modulation = combine_inventory_modulations(owner_modulation, global);
        }
    }
    prepare_inventory_owner_pixels(
        &mut owner_pixels,
        owner_modulation,
        inventory_owner_blit_mode(raw_blit_mode),
        renderer_config,
    );
    Some((base, Some(owner_pixels)))
}

pub fn prepare_owned_menu_definition_pixels(
    image: &clonk_engine::DefinitionPictureImage,
    object_color: u32,
    global_modulation: Option<Color>,
    raw_blit_mode: u32,
) -> Option<Vec<u8>> {
    let mut base = image.pixels().to_vec();
    let Some(original_owner) = image.color_mask().filter(|mask| mask.len() == base.len()) else {
        let mut flattened = inventory_picture_pixels(image, object_color);
        prepare_owned_menu_pixels(
            &mut flattened,
            global_modulation,
            inventory_blit_mode(raw_blit_mode),
        );
        return Some(flattened);
    };

    prepare_owned_menu_pixels(
        &mut base,
        global_modulation,
        inventory_blit_mode(raw_blit_mode),
    );
    let mut owner = original_owner.to_vec();
    prepare_owned_menu_pixels(
        &mut owner,
        Some(inventory_owner_modulation(object_color)),
        inventory_owner_blit_mode(raw_blit_mode),
    );
    if raw_blit_mode & 4 == 0 {
        prepare_owned_menu_pixels(&mut owner, global_modulation, BlitMode::Normal);
    }
    composite_owned_menu_owner_pixels(&mut base, &owner, &original_owner);
    Some(base)
}

pub fn composite_owned_menu_owner_pixels(base: &mut [u8], owner: &[u8], original_owner: &[u8]) {
    for ((base, owner), original_owner) in base
        .chunks_exact_mut(4)
        .zip(owner.chunks_exact(4))
        .zip(original_owner.chunks_exact(4))
    {
        if original_owner[3] == 0 {
            continue;
        }
        if original_owner[3] == 255 || base[3] == 0 {
            base.copy_from_slice(owner);
            continue;
        }
        let alpha = u16::from(owner[3]);
        for channel in 0..3 {
            base[channel] = ((u16::from(owner[channel]) * alpha
                + u16::from(base[channel]) * (255 - alpha))
                >> 8) as u8;
        }
        base[3] = base[3].saturating_add(owner[3]);
    }
}

pub fn inventory_modulation(color: u32, blit_mode: u32) -> Option<Color> {
    (color != 0 || blit_mode & (2 | 8) != 0).then(|| {
        Color::new(
            ((color >> 16) & 0xff) as u8,
            ((color >> 8) & 0xff) as u8,
            (color & 0xff) as u8,
            ((color >> 24) & 0xff) as u8,
        )
    })
}

pub fn inventory_blit_mode(raw: u32) -> BlitMode {
    match raw & 3 {
        1 => BlitMode::Additive,
        2 => BlitMode::Mod2,
        3 => BlitMode::Mod2Additive,
        _ => BlitMode::Normal,
    }
}

pub fn prepare_inventory_pixels(
    pixels: &mut [u8],
    modulation: Color,
    mode: BlitMode,
    renderer_config: clonk_frontend::AdvancedRendererConfig,
) {
    if modulation == Color::opaque(255, 255, 255)
        && matches!(mode, BlitMode::Normal | BlitMode::Additive)
    {
        for pixel in pixels.chunks_exact_mut(4) {
            pixel[3] = prepared_inventory_alpha(pixel[3], modulation, mode, renderer_config);
        }
        return;
    }
    for pixel in pixels.chunks_exact_mut(4) {
        let source = Color::new(pixel[0], pixel[1], pixel[2], pixel[3]);
        let mut prepared = mode.prepare_source(source, modulation);
        prepared.a = prepared_inventory_alpha(source.a, modulation, mode, renderer_config);
        pixel.copy_from_slice(&[prepared.r, prepared.g, prepared.b, prepared.a]);
    }
}

/// `C4Object::Picture2Facet` renders owned menu symbols into a non-primary
/// surface, so native dispatches to the packed software color helpers instead
/// of the live GL shader. Convert Rust opacity to/from C4's transparency byte
/// around that operation.
pub fn prepare_owned_menu_pixels(pixels: &mut [u8], modulation: Option<Color>, mode: BlitMode) {
    let Some(modulation) = modulation else {
        return;
    };
    let packed_modulate = |source: Color| {
        let multiply = |source: u8, modulation: u8| -> u8 {
            ((u16::from(source) * u16::from(modulation)) >> 8) as u8
        };
        let screen_transparency = |source: u8, modulation: u8| -> u8 {
            let product = (u16::from(source) * u16::from(modulation)) >> 8;
            (u16::from(source) + u16::from(modulation) - product).min(255) as u8
        };
        Color::new(
            multiply(source.r, modulation.r),
            multiply(source.g, modulation.g),
            multiply(source.b, modulation.b),
            screen_transparency(source.a, modulation.a),
        )
    };
    for pixel in pixels.chunks_exact_mut(4) {
        let packed = Color::new(pixel[0], pixel[1], pixel[2], 255 - pixel[3]);
        let prepared = match mode {
            BlitMode::Mod2 | BlitMode::Mod2Additive => packed.modulate_clr_mod2(modulation),
            BlitMode::Normal | BlitMode::Additive => packed_modulate(packed),
        };
        pixel.copy_from_slice(&[prepared.r, prepared.g, prepared.b, 255 - prepared.a]);
    }
}

pub fn composite_inventory_picture_layer(
    destination: &mut Surface,
    source: &Surface,
    mode: BlitMode,
) -> Option<()> {
    if destination.width() != source.width() || destination.height() != source.height() {
        return None;
    }
    for y in 0..destination.height() {
        for x in 0..destination.width() {
            let foreground = source.get_pixel(x, y)?;
            if foreground.a == 0 {
                continue;
            }
            let background = destination.get_pixel(x, y)?;
            let output = match mode {
                BlitMode::Normal | BlitMode::Mod2 => {
                    blend_straight_picture_over(foreground, background)
                }
                BlitMode::Additive | BlitMode::Mod2Additive => {
                    blend_straight_picture_additive(foreground, background)
                }
            };
            destination.set_pixel(x, y, output).ok()?;
        }
    }
    Some(())
}

/// Flatten source-over layers into a straight-alpha cache. This is the
/// associative form needed when the finished inventory image is blended once
/// more onto the real HUD framebuffer.
pub fn blend_straight_picture_over(source: Color, destination: Color) -> Color {
    if destination.a == 0 || source.a == 255 {
        return source;
    }
    let source_alpha = f32::from(source.a) / 255.0;
    let destination_alpha = f32::from(destination.a) / 255.0;
    let output_alpha = source_alpha + destination_alpha * (1.0 - source_alpha);
    let channel = |source: u8, destination: u8| -> u8 {
        ((f32::from(source) * source_alpha
            + f32::from(destination) * destination_alpha * (1.0 - source_alpha))
            / output_alpha)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    Color::new(
        channel(source.r, destination.r),
        channel(source.g, destination.g),
        channel(source.b, destination.b),
        (output_alpha * 255.0).round().clamp(0.0, 255.0) as u8,
    )
}

/// Best representable flattening of an additive layer. With a nonzero cached
/// destination alpha, scale the additive contribution back into straight RGB
/// so the later normal HUD draw recreates it. A wholly transparent cache has
/// no exact ImageData representation for background-preserving addition.
pub fn blend_straight_picture_additive(source: Color, destination: Color) -> Color {
    if destination.a == 0 {
        return source;
    }
    let source_alpha = f32::from(source.a) / 255.0;
    let destination_alpha = f32::from(destination.a) / 255.0;
    let channel = |source: u8, destination: u8| -> u8 {
        (f32::from(destination) + f32::from(source) * source_alpha / destination_alpha)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    Color::new(
        channel(source.r, destination.r),
        channel(source.g, destination.g),
        channel(source.b, destination.b),
        destination.a,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A grid sheet whose every phase cell is filled with its own phase index,
    /// scaled up by `factor` in both axes.
    fn phase_sheet(cell: u32, columns: u32, rows: u32, factor: u32) -> ImageData {
        let (width, height) = (cell * columns * factor, cell * rows * factor);
        let mut pixels = vec![0_u8; (width * height * 4) as usize];
        for phase in 0..columns * rows {
            let phase_x = (phase % columns) * cell * factor;
            let phase_y = (phase / columns) * cell * factor;
            for y in phase_y..phase_y + cell * factor {
                for x in phase_x..phase_x + cell * factor {
                    let offset = ((y * width + x) * 4) as usize;
                    pixels[offset..offset + 4].copy_from_slice(&[phase as u8, 1, 2, 255]);
                }
            }
        }
        ImageData::new(width, height, pixels)
    }

    #[test]
    fn gui_icon_phases_follow_double_resolution_icon_sheets() {
        // C4Gui.cpp:1090-1092 slices GUIIcons/GUIIcons2 as pure C4GUI_IconWdt
        // grids, so a sheet that is an exact integer multiple of the oracle's
        // 240x360 / 256x320 keeps the grid and grows the cell. Addressing it
        // with the 1x cell would land on an entirely different phase.
        let icons = phase_sheet(40, 6, 9, 2);
        let icons2 = phase_sheet(64, 4, 5, 2);
        assert_eq!((icons.width(), icons.height()), (480, 720));
        assert_eq!((icons2.width(), icons2.height()), (512, 640));
        let engine = Engine::new();
        let resources = ScriptTextSpecResources {
            gui_icons: Some(&icons),
            gui_icons_extended: Some(&icons2),
            score: None,
        };

        for (spec, phase, native_cell) in [
            ("Ico:GameRunning", 30_u8, 40_u32),
            ("Ico:Lobby", 31, 40),
            ("Ico:RuntimeJoin", 32, 40),
            ("Ico:Locked", 13, 64),
            ("Ico:League", 8, 64),
            ("Ico:FairCrew", 2, 64),
        ] {
            let image = resolve_script_font_image(&engine, spec, 0xff, resources)
                .unwrap_or_else(|| panic!("{spec} resolves"));
            assert_eq!(
                (image.width(), image.height()),
                (native_cell * 2, native_cell * 2),
                "{spec} crops the doubled cell"
            );
            assert!(
                image
                    .pixels()
                    .chunks_exact(4)
                    .all(|pixel| pixel[0] == phase),
                "{spec} must address phase {phase} on the doubled sheet"
            );
        }
    }

    #[test]
    fn native_icon_sheets_keep_their_oracle_cells() {
        let icons = phase_sheet(40, 6, 9, 1);
        let icons2 = phase_sheet(64, 4, 5, 1);
        let engine = Engine::new();
        let resources = ScriptTextSpecResources {
            gui_icons: Some(&icons),
            gui_icons_extended: Some(&icons2),
            score: None,
        };
        for (spec, phase, cell) in [("Ico:Lobby", 31_u8, 40_u32), ("Ico:League", 8, 64)] {
            let image = resolve_script_font_image(&engine, spec, 0xff, resources)
                .unwrap_or_else(|| panic!("{spec} resolves"));
            assert_eq!((image.width(), image.height()), (cell, cell), "{spec}");
            assert!(image
                .pixels()
                .chunks_exact(4)
                .all(|pixel| pixel[0] == phase));
        }
    }
}
