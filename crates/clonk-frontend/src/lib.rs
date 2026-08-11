#![allow(
    clippy::manual_clamp,
    clippy::op_ref,
    clippy::question_mark,
    clippy::too_many_arguments
)]

pub mod classic_gui;
pub mod clonk_fonts;
pub mod context_menu;
pub mod definition_sel;
pub mod developer_console;
pub mod download_dialog;
mod draw_primitives;
pub mod flash_message;
mod fog_modulation;
pub mod game_lobby;
pub mod game_option_buttons;
mod graphics_system;
pub mod hud;
pub mod info_dialog;
mod input;
pub mod input_dialog;
mod landscape_sky;
pub mod league_signup;
pub mod loader_screen;
mod materials;
pub mod message_dialog;
pub mod network_chart;
pub mod network_start_wait;
pub mod progress_dialog;
pub mod rename_edit;
mod render_config;
pub mod runtime_client_list;
pub mod runtime_help;
pub mod scoreboard;
mod software_draw;
mod sprite_capture;
mod sprites;
mod startup_about;
pub mod startup_about_dlg;
mod startup_main_menu;
mod startup_menu;
pub mod startup_netdlg;
mod startup_options;
pub mod startup_options_advanced;
pub mod startup_options_controls;
pub mod startup_options_dlg;
pub mod startup_options_graphics;
pub mod startup_options_network;
pub mod startup_plrproperties;
pub mod startup_plrsel;
pub mod startup_portraitsel;
pub mod startup_scensel;
#[cfg(test)]
pub(crate) mod test_support;
mod viewport;
pub mod viewport_draw_order;
pub mod viewport_projection;

use clonk_engine::landscape::PixelGrid;
use clonk_engine::{
    math::{fixtoi, itofix, C4Fixed},
    object_visible_for_player,
    particles::{ParticleDefCore, ParticleDrawProc, SafeRng},
    physical_action_graphics_key, DefinitionActionGraphics, DefinitionId, DefinitionLineMetadata,
    DefinitionRect, DefinitionTargetRect, Direction, DrawTransform, EnvironmentFrame,
    EnvironmentSettings, FloatVector2, GammaControlState, GraphicsOverlayMode, Landscape,
    ObjectGraphicsOverlay, ObjectId, ObjectSnapshot, ObjectStatus, ParticleLayer, ParticleSnapshot,
    PlayerState, RgbColor, SimulationSnapshot, SkyFrame, SkySettings,
    SurfaceSnapshot as EngineSurfaceSnapshot, Vector2, WeatherEvent, FULL_CON, OWNER_NONE,
    PHYSICAL_ACTION_GRAPHICS_MARKER, PLAYER_VIEW_MODE_TARGET,
};
#[cfg(test)]
use clonk_engine::{
    VIS_ALLIES, VIS_ENEMIES, VIS_GOD, VIS_LAYER_TOGGLE, VIS_LOCAL, VIS_OVERLAY_ONLY, VIS_OWNER,
};
use clonk_graphics::{
    stdgl_blit_sampling, BlitSampling, Color, GpuBlend, GpuCommand, GpuObjectSprite,
    GpuOuterModulation, GpuPrimitiveTopology, GpuSampler, GpuSolidAlphaMode,
    GpuSolidOuterModulation, GpuSolidStyle, GpuSolidVertex, GpuSpriteQuad, GpuTextureId,
    GpuTextureResource, GpuVertex, PixelFormat, Point as SurfacePoint, Rect as SurfaceRect,
    Surface, SurfaceDrawTarget, SurfaceSnapshot as GraphicsSurfaceSnapshot, TextFont,
    Transform as GraphicsTransform,
};
use clonk_gui::{Rect as GuiRect, Size as GuiSize};
use rayon::prelude::*;
use std::cell::Cell;
use std::collections::{hash_map::DefaultHasher, HashMap, HashSet};
use std::convert::TryFrom;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::rc::Rc;
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

pub use clonk_fonts::{expand_hotkey_markup, ClonkFontSet};
pub use clonk_gui::{
    GuiError as StartupMenuError, GuiResult as StartupMenuResult, ImageData, KeyCode,
    Point as GuiPoint, ScenarioEntry, ScenarioKind,
};
pub use hud::{CommandIcon, CommandImage, CommandOverlayIcon};
pub use input::InputDispatcher;
pub use startup_about::{AboutAction, StartupAboutDialog};
pub use startup_main_menu::{
    centered_label_rect, centered_label_tooltip_at, main_menu_layout, MainMenuAction, MainMenuItem,
    StartupMainMenu, StartupTooltip,
};
pub use startup_menu::{ScenarioSummary, StartupMenu, StartupMenuAction};
pub use startup_options::{ControlOptionItem, ControlOptionsAction, ControlOptionsView};

// The split family modules are re-exported wholesale so item paths
// (crate-internal `crate::...` and cross-crate `clonk_frontend::...`)
// match the pre-split single-file layout.
pub(crate) use draw_primitives::*;
pub(crate) use fog_modulation::*;
pub use graphics_system::*;
pub(crate) use landscape_sky::*;
pub use materials::*;
pub use render_config::*;
pub use software_draw::*;
pub use sprite_capture::*;
pub use sprites::*;
pub use viewport::*;

const MIN_VIEWPORT_ZOOM: f32 = 0.125;
const MAX_VIEWPORT_ZOOM: f32 = 4.0;

fn c4_presentation_text(text: &str) -> String {
    clonk_resources::decode_legacy_script_text(&clonk_script::c4_string_bytes(text))
}

/// `SWordWrap(text, ' ', '|', max_line)` (src/C4Strings.cpp:311-331).
/// Native counts encoded bytes; presentation strings are already decoded at
/// this boundary, so preserve the same separator/last-space algorithm over
/// displayed scalar values.
fn c4_word_wrap(text: &str, max_line: usize) -> String {
    let mut characters = text.chars().collect::<Vec<_>>();
    let mut last_space = None;
    let mut line_run = 0usize;
    for index in 0..characters.len() {
        if characters[index] == ' ' {
            last_space = Some(index);
        }
        if characters[index] == '|' {
            line_run = 0;
        }
        if line_run >= max_line {
            if let Some(space) = last_space {
                characters[space] = '|';
                line_run = index.saturating_sub(space);
            }
        }
        line_run = line_run.saturating_add(1);
    }
    characters.into_iter().collect()
}

/// `C4ViewportScrollBorder` (src/C4Constants.h:95).
const VIEWPORT_SCROLL_BORDER: i32 = 40;
/// `Config.General.ScrollSmooth` (src/C4Config.cpp:386). The C++ viewport
/// clamps the configured value to 1..=50 at the point of use.
const DEFAULT_SCROLL_SMOOTH: i32 = 4;
const CAMERA_UNINITIALIZED: i32 = -31_337;
const PICK_TOLERANCE: f32 = 6.0;
/// `CRed`, palette entry 10 after C4GraphicsResource expands C4.PAL's
/// six-bit channels (`src/StdColors.h:33-34`; `src/C4GraphicsResource.cpp:176-193`).
pub const MOUSE_SELECTION_FRAME_COLOR: Color = Color::opaque(0xc8, 0x00, 0x00);
/// `MagicPhysicalFactor` (src/C4Object.h:81).
const MAGIC_PHYSICAL_FACTOR: i32 = 1_000;
const MATERIAL_OVERLAY_EXACT: i32 = 1;
const MATERIAL_OVERLAY_HUGE_ZOOM: i32 = 4;
const MATERIAL_OVERLAY_MONOCHROME: i32 = 8;
/// `C4GFXBLIT_ADDITIVE` (src/C4Surface.h:40).
const C4GFXBLIT_ADDITIVE: u32 = 1;
/// `C4GFXBLIT_MOD2`: MOD2 source modulation for the main surface.
const C4GFXBLIT_MOD2: u32 = 2;
/// Green construction-placement preview used by `C4MouseControl`.
const CONSTRUCTION_DRAG_VALID_MODULATION: u32 = 0x1f00_7f00;
/// Red construction-placement preview used by `C4MouseControl`.
const CONSTRUCTION_DRAG_INVALID_MODULATION: u32 = 0x8f7f_0000;
/// `C4GFXBLIT_CLRSFC_OWNCLR`: do not fold global ColorMod into owner color.
const C4GFXBLIT_CLRSFC_OWNCLR: u32 = 4;
/// `C4GFXBLIT_CLRSFC_MOD2`: MOD2 source modulation for the owner surface.
const C4GFXBLIT_CLRSFC_MOD2: u32 = 8;
/// `C4GFXBLIT_PARENT` is an exact overlay sentinel, not a combinable flag
/// (src/C4DefGraphics.cpp:762-768).
const C4GFXBLIT_PARENT: u32 = 256;

#[cfg(test)]
mod tests {
    use super::*;
    use clonk_engine::scenario::{load_system_scripts, LegacyDefinitionResolver};
    use clonk_engine::{
        CommandStackSnapshot, Engine, EnvironmentFrame, FogOfWarPlayerFrame, JoinPlayerConfig,
        Landscape, LiquidSegment, MaterialId, ObjectId, ObjectUpdate, ObjectVertex, PlayerState,
        RgbColor, Scenario, ScenarioError, SpawnConfig, Vector2,
    };
    use clonk_graphics::{BitmapFont, PixelFormat};
    use clonk_resources::{Group, MaterialLibrary};
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn test_font() -> Arc<dyn TextFont> {
        Arc::new(BitmapFont::new())
    }

    fn gray(v: u8) -> Color {
        Color::new(v, v, v, 255)
    }

    /// The lobby carried a pre-atlas copy of the caret routine. A
    /// `width`-sized `ImageData` makes `cpp_texture_size` tile the glyph on
    /// its narrow side, and every tile clamps its own bilinear edge, so each
    /// bar of the broken-bar caret lost its outermost scaled row (12 px
    /// instead of 13). The shared routine pads to a single power-of-two tile
    /// like the real C4Surface font atlas, which is what the four other
    /// dialogs already did.
    #[test]
    fn scaled_caret_keeps_whole_bars_through_one_atlas_tile() {
        const WIDTH: i32 = 3;
        const CELL_HEIGHT: i32 = 20;

        let mut font = clonk_graphics::clonk_font::ClonkFont::new(CELL_HEIGHT - 1);
        // U+00A6 is a BROKEN bar: opaque, gap, opaque.
        let mut pixels = vec![Color::new(255, 255, 255, 255); (WIDTH * CELL_HEIGHT) as usize];
        for row in 8..12 {
            for column in 0..WIDTH {
                pixels[(row * WIDTH + column) as usize] = Color::new(0, 0, 0, 0);
            }
        }
        font.add_glyph(
            '\u{a6}',
            clonk_graphics::clonk_font::GlyphCell {
                width: WIDTH,
                pixels,
            },
        );

        let mut surface = Surface::new(48, 48, PixelFormat::Rgba8888);
        surface.fill(Color::new(0, 0, 0, 255));
        draw_scaled_caret(
            &mut surface,
            &font,
            4,
            4,
            crate::classic_gui::IntRect {
                x: 0,
                y: 0,
                w: 48,
                h: 48,
            },
            None,
        );

        let lit: Vec<bool> = (0..48)
            .map(|y| surface.get_pixel(5, y).expect("in bounds").r > 0)
            .collect();
        let mut runs = Vec::new();
        let mut run = 0;
        for on in lit {
            if on {
                run += 1;
            } else if run > 0 {
                runs.push(run);
                run = 0;
            }
        }
        if run > 0 {
            runs.push(run);
        }
        assert_eq!(runs, vec![13, 13], "tile-clamped caret bars");
    }

    #[test]
    fn player_startup_name_decodes_native_bytes_only_for_presentation() {
        let raw = clonk_script::c4_string_from_bytes(b"Andr\xe9");
        assert_eq!(c4_presentation_text(&raw), "Andr\u{e9}");
        assert_eq!(clonk_script::c4_string_bytes(&raw), b"Andr\xe9");
    }

    #[test]
    fn clr_mod_map_reset_aligns_cells_and_keeps_the_extra_edge() {
        let map = ClrModMap::reset(64, 64, 100, 70, 10, 70, 5, 9, 0).unwrap();

        assert_eq!((map.origin_x, map.origin_y), (-5, 3));
        assert_eq!((map.width, map.height), (3, 3));
        assert_eq!(map.cells, vec![0; 9]);
    }

    #[test]
    fn clr_mod_map_uses_native_nonstandard_corner_term() {
        let map = ClrModMap {
            resolution_x: 64,
            resolution_y: 64,
            width: 2,
            height: 2,
            origin_x: 0,
            origin_y: 0,
            fade_transparent: false,
            cells: vec![0x0000_0000, 0x4040_4040, 0x8080_8080, 0xffff_ffff],
        };

        assert_eq!(map.get_mod_at(32, 32), 0x8787_8787);
        assert_eq!(map.get_mod_at(16, 48), 0x8a8a_8a8a);
    }

    #[test]
    fn fog_sprite_sampler_uses_64px_corner_quads_instead_of_per_pixel_get_mod_at() {
        let map = ClrModMap {
            resolution_x: 64,
            resolution_y: 64,
            width: 2,
            height: 2,
            origin_x: 0,
            origin_y: 0,
            fade_transparent: false,
            cells: vec![0x0000_0000, 0x0040_4040, 0x0080_8080, 0x00ff_ffff],
        };
        let fog = FogDrawContext {
            map: Arc::new(map.clone()),
            zoom: 1.0,
        };
        let sampler = FogSpriteSampler::new(
            &fog,
            (0.0, 0.0, 64.0, 64.0),
            (0.0, 0.0, 64.0, 64.0),
            (64, 64),
            false,
            |x, y| (x, y),
        )
        .unwrap();

        assert_eq!(map.get_mod_at(32, 32), 0x0087_8787);
        assert_eq!(sampler.modulation_at(0.5, 0.5), 0x0060_6060);

        let (x_samples, y_samples) = sampler.raster_axes(73, 41);
        for y in 0..41 {
            for x in 0..73 {
                let normalized_x = (x as f32 + 0.5) / 73.0;
                let normalized_y = (y as f32 + 0.5) / 41.0;
                let scalar = sampler.modulation_sample(normalized_x, normalized_y);
                let cached = sampler
                    .modulation_sample_for_axes(x_samples[x as usize], y_samples[y as usize]);
                assert_eq!(cached.modulation, scalar.modulation);
                assert_eq!(cached.weights, scalar.weights);
                assert_eq!(cached.interpolate(), scalar.interpolate());
            }
        }

        let flipped_partial = FogSpriteSampler::new(
            &fog,
            (0.0, 0.0, 40.0, 1.0),
            (50.0, 0.0, 40.0, 1.0),
            (128, 128),
            true,
            |x, y| (x, y),
        )
        .unwrap();
        assert_eq!(flipped_partial.x_ranges, vec![(0.0, 26.0), (26.0, 40.0)]);
        assert!(flipped_partial
            .x_ranges
            .iter()
            .all(|(left, right)| right - left <= 64.0));

        let local_box = FogSpriteSampler::new_with_chunks(
            &fog,
            (0.0, 0.0, 40.0, 1.0),
            (0.0, 0.0, 40.0, 1.0),
            (16.0, 16.0),
            false,
            |x, y| (x, y),
        )
        .unwrap();
        let world_aligned_box = FogSpriteSampler::new_with_chunks(
            &fog,
            (0.0, 0.0, 40.0, 1.0),
            (5.0, 0.0, 40.0, 1.0),
            (16.0, 16.0),
            false,
            |x, y| (x, y),
        )
        .unwrap();
        assert_eq!(local_box.x_ranges[0], (0.0, 16.0));
        assert_eq!(world_aligned_box.x_ranges[0], (0.0, 11.0));

        let vertex_first = FogModulationSample {
            modulation: [0, 0x0002_0202, 0, 0],
            weights: [0.5, 0.5, 0.0, 0.0],
        };
        assert_eq!(vertex_first.interpolate(), 0x0001_0101);
        assert_eq!(
            modulate_c4_colors(0x0080_8080, vertex_first.interpolate()),
            0,
            "combining after interpolation loses the low byte",
        );
        assert_eq!(
            vertex_first.combine_with(0x0080_8080),
            0x0001_0101,
            "native ModulateClr runs at vertices before GL interpolation",
        );

        let mod2_at_black_corner = prepare_sprite_fragment(
            Color::opaque(200, 200, 200),
            None,
            None,
            SpriteBlitState {
                mode: C4GFXBLIT_MOD2,
                modulation: Some(0x00ff_ffff),
                fog_modulation: Some(FogModulationSample {
                    modulation: [0, 0x0002_0202, 0, 0],
                    weights: [1.0, 0.0, 0.0, 0.0],
                }),
                renderer_config: AdvancedRendererConfig::DEFAULT,
            },
        );
        let PreparedSpriteFragment::Shader { rgb, alpha } = mod2_at_black_corner else {
            panic!("fogged MOD2 sprite must use the shader path");
        };
        assert_eq!(rgb, [145.0; 3]);
        assert_eq!(
            alpha, 255.0,
            "one nonblack quad vertex keeps MOD2 active at its black corner",
        );
    }

    #[test]
    fn cached_fog_axes_match_independent_multichunk_reference() {
        fn legacy_quad_and_weights(
            sampler: &FogSpriteSampler,
            normalized_x: f32,
            normalized_y: f32,
        ) -> (FogColorQuad, [f32; 4]) {
            let local_x = normalized_x.clamp(0.0, 1.0) * sampler.source_width;
            let local_y = normalized_y.clamp(0.0, 1.0) * sampler.source_height;
            let column = sampler
                .x_ranges
                .iter()
                .position(|range| local_x < range.1)
                .unwrap_or(sampler.x_ranges.len() - 1);
            let row = sampler
                .y_ranges
                .iter()
                .position(|range| local_y < range.1)
                .unwrap_or(sampler.y_ranges.len() - 1);
            let quad = sampler.quads[row * sampler.columns + column];
            let u = ((local_x - quad.x.0) / (quad.x.1 - quad.x.0)).clamp(0.0, 1.0);
            let v = ((local_y - quad.y.0) / (quad.y.1 - quad.y.0)).clamp(0.0, 1.0);
            let weights = if u + v <= 1.0 {
                [1.0 - u - v, u, v, 0.0]
            } else {
                [0.0, 1.0 - v, 1.0 - u, u + v - 1.0]
            };
            (quad, weights)
        }

        let map = ClrModMap {
            resolution_x: 32,
            resolution_y: 32,
            width: 9,
            height: 9,
            origin_x: -17,
            origin_y: 11,
            fade_transparent: false,
            cells: (0..81)
                .map(|index| {
                    let red = (index * 37 % 256) as u32;
                    let green = (index * 71 % 256) as u32;
                    let blue = (index * 113 % 256) as u32;
                    (red << 16) | (green << 8) | blue
                })
                .collect(),
        };
        let fog = FogDrawContext {
            map: Arc::new(map),
            zoom: 1.25,
        };

        for flipped in [false, true] {
            let sampler = FogSpriteSampler::new(
                &fog,
                (23.5, -9.25, 150.0, 130.0),
                (13.0, 7.0, 150.0, 130.0),
                (256, 192),
                flipped,
                |x, y| (x * 1.125 + 3.0, y * 0.75 - 4.0),
            )
            .unwrap();
            assert!(sampler.x_ranges.len() > 2);
            assert!(sampler.y_ranges.len() > 2);

            // These dimensions put pixel centers exactly on the first global
            // 64px source seams (local x=51 and y=57), pinning `< end`
            // ownership as well as samples on both sides of every chunk.
            let (x_samples, y_samples) = sampler.raster_axes(75, 65);
            for y in 0..65 {
                for x in 0..75 {
                    let normalized_x = (x as f32 + 0.5) / 75.0;
                    let normalized_y = (y as f32 + 0.5) / 65.0;
                    let (quad, weights) =
                        legacy_quad_and_weights(&sampler, normalized_x, normalized_y);
                    let cached_axes = (x_samples[x as usize], y_samples[y as usize]);
                    let (cached_quad, cached_weights) =
                        sampler.quad_and_weights_for_axes(cached_axes.0, cached_axes.1);
                    assert_eq!(cached_quad, quad);
                    assert_eq!(cached_weights, weights);

                    let color = Color::new(213, 147, 89, 255);
                    let reference_color = interpolate_quad_color(
                        quad.modulation
                            .map(|modulation| modulate_surface_color(color, modulation)),
                        weights,
                    );
                    assert_eq!(
                        sampler.color_at_axes(color, cached_axes.0, cached_axes.1),
                        reference_color,
                    );

                    let color_at_y = |vertex_y: f32| {
                        Color::new(
                            (vertex_y * 91.0).round().clamp(0.0, 255.0) as u8,
                            (vertex_y * 53.0 + 17.0).round().clamp(0.0, 255.0) as u8,
                            201,
                            255,
                        )
                    };
                    let top = color_at_y(quad.y.0 / sampler.source_height);
                    let bottom = color_at_y(quad.y.1 / sampler.source_height);
                    let reference_vertical = interpolate_quad_color(
                        [
                            modulate_surface_color(top, quad.modulation[0]),
                            modulate_surface_color(top, quad.modulation[1]),
                            modulate_surface_color(bottom, quad.modulation[2]),
                            modulate_surface_color(bottom, quad.modulation[3]),
                        ],
                        weights,
                    );
                    assert_eq!(
                        sampler.vertical_color_at_axes(cached_axes.0, cached_axes.1, color_at_y,),
                        reference_vertical,
                    );
                }
            }
        }
    }

    #[test]
    fn fog_lines_interpolate_original_endpoint_samples_and_fog_precedes_gamma() {
        let fog = FogDrawContext {
            map: Arc::new(ClrModMap {
                resolution_x: 64,
                resolution_y: 64,
                width: 2,
                height: 2,
                origin_x: 0,
                origin_y: 0,
                fade_transparent: false,
                cells: vec![0x0000_0000, 0x0040_4040, 0x0080_8080, 0x00ff_ffff],
            }),
            zoom: 1.0,
        };
        let mut line = Surface::new(65, 65, PixelFormat::Rgba8888);
        draw_pxs_line(
            &mut line,
            (0.0, 0.0),
            (64.0, 64.0),
            Color::opaque(255, 255, 255),
            None,
            Some(&fog),
        );
        assert_eq!(line.get_pixel(32, 32), Some(gray(127)));

        let black_fog = FogDrawContext {
            map: Arc::new(ClrModMap {
                resolution_x: 64,
                resolution_y: 64,
                width: 2,
                height: 2,
                origin_x: 0,
                origin_y: 0,
                fade_transparent: false,
                cells: vec![0; 4],
            }),
            zoom: 1.0,
        };
        let image = ImageData::new(1, 1, vec![255, 255, 255, 255]);
        let mut pixel = Surface::new(1, 1, PixelFormat::Rgba8888);
        let gamma = clonk_graphics::GammaRamp::standard();
        draw_image_region(
            &mut pixel,
            &GuiRect::new(0.0, 0.0, 1.0, 1.0),
            &image,
            None,
            &SourceRect::new(0, 0, 1, 1),
            false,
            None,
            SpriteBlitState::normal(),
            Some(&gamma),
            Some(&black_fog),
        );
        assert_eq!(
            pixel.get_pixel(0, 0),
            Some(gamma_encode_fragment(Color::opaque(0, 0, 0), &gamma))
        );
    }

    #[test]
    fn fog_transparency_adds_to_sky_texture_transparency() {
        let mut graphics = test_graphics(1, 1, 1, "FoW sky alpha");
        graphics.surface_mut().fill(Color::opaque(0, 255, 0));
        graphics.active_fog_map = Some(Arc::new(ClrModMap {
            resolution_x: 64,
            resolution_y: 64,
            width: 2,
            height: 2,
            origin_x: 0,
            origin_y: 0,
            fade_transparent: true,
            cells: vec![0x80ff_ffff; 4],
        }));
        graphics.blit_sky_tile(
            &ImageData::new(1, 1, vec![255, 0, 0, 128]),
            0,
            0,
            None,
            1.0,
            None,
        );
        assert_eq!(
            graphics.surface().get_pixel(0, 0),
            Some(Color::opaque(0, 255, 0))
        );
    }

    #[test]
    fn sky_modulation_combines_with_fog_vertices_and_keeps_packed_alpha() {
        let mut graphics = test_graphics(1, 1, 1, "FoW sky modulation");
        graphics.surface_mut().fill(Color::opaque(0, 255, 0));
        graphics.active_fog_map = Some(Arc::new(ClrModMap {
            resolution_x: 64,
            resolution_y: 64,
            width: 2,
            height: 2,
            origin_x: 0,
            origin_y: 0,
            fade_transparent: false,
            cells: vec![0x00ff_ffff; 4],
        }));

        graphics.blit_sky_tile(
            &ImageData::new(1, 1, vec![255, 0, 0, 255]),
            0,
            0,
            Some(0x80ff_ffff),
            1.0,
            None,
        );

        assert_eq!(
            graphics.surface().get_pixel(0, 0),
            Some(Color::opaque(127, 128, 0)),
            "Sky.Modulation is combined with GetModAt by packed ModulateClr before blending",
        );
    }

    #[test]
    fn fogged_sky_gradient_applies_global_modulation_before_the_map() {
        let mut graphics = test_graphics(1, 1, 1, "FoW gradient modulation");
        graphics.active_fog_map = Some(Arc::new(ClrModMap {
            resolution_x: 64,
            resolution_y: 64,
            width: 2,
            height: 2,
            origin_x: 0,
            origin_y: 0,
            fade_transparent: false,
            cells: vec![0x00ff_ffff; 4],
        }));
        let settings = SkySettings {
            fade_top: RgbColor::new(255, 255, 255),
            fade_bottom: RgbColor::new(255, 255, 255),
            modulation: Some(0x0080_8080),
            ..SkySettings::default()
        };

        graphics.fill_sky_gradient(&settings, 1.0, None);

        assert_eq!(graphics.surface().get_pixel(0, 0), Some(gray(126)));
    }

    #[test]
    fn cropped_sky_tile_uses_visible_crop_edges_as_fog_vertices() {
        let mut graphics = test_graphics(49, 1, 1, "cropped FoW sky tile");
        graphics.surface_mut().fill(Color::opaque(0, 0, 0));
        let row = [0x0064_6464, 0x00c8_c8c8, 0x0032_3232, 0x0032_3232];
        graphics.active_fog_map = Some(Arc::new(ClrModMap {
            resolution_x: 32,
            resolution_y: 1,
            width: 4,
            height: 2,
            origin_x: 0,
            origin_y: 0,
            fade_transparent: false,
            cells: row.into_iter().chain(row).collect(),
        }));
        let image = ImageData::new(64, 64, vec![255; 64 * 64 * 4]);
        let fog = graphics.fog_draw_context().unwrap();
        let cropped = FogSpriteSampler::new(
            &fog,
            (0.0, 0.0, 49.0, 1.0),
            (15.0, 0.0, 49.0, 1.0),
            (64, 64),
            false,
            |x, y| (x, y),
        )
        .unwrap();
        let uncropped = FogSpriteSampler::new(
            &fog,
            (-15.0, 0.0, 64.0, 1.0),
            (0.0, 0.0, 64.0, 1.0),
            (64, 64),
            false,
            |x, y| (x, y),
        )
        .unwrap();
        let output = |sampler: &FogSpriteSampler, normalized_x: f32| {
            let blit = sampler.blit_at(SpriteBlitState::normal(), normalized_x, 0.5);
            composite_sprite_fragment(
                prepare_sprite_fragment(Color::opaque(255, 255, 255), None, None, blit),
                Color::opaque(0, 0, 0),
                blit,
                None,
            )
        };
        let expected = output(&cropped, 0.5 / 49.0);
        let stale_offscreen_vertex = output(&uncropped, 15.5 / 64.0);

        graphics.blit_sky_tile(&image, -15, 0, None, 1.0, None);

        assert_eq!(graphics.surface().get_pixel(0, 0), Some(expected));
        assert_ne!(expected, stale_offscreen_vertex);
    }

    #[test]
    fn parallel_tiled_sky_rows_match_scalar_crops_fog_modulation_and_gamma() {
        const SURFACE_WIDTH: u32 = 173;
        const SURFACE_HEIGHT: u32 = 131;
        const IMAGE_WIDTH: u32 = 37;
        const IMAGE_HEIGHT: u32 = 29;

        let image = ImageData::new(
            IMAGE_WIDTH,
            IMAGE_HEIGHT,
            (0..IMAGE_HEIGHT)
                .flat_map(|y| {
                    (0..IMAGE_WIDTH).flat_map(move |x| {
                        let alpha = match (x * 5 + y * 7) % 4 {
                            0 => 0,
                            1 => 73,
                            2 => 161,
                            _ => 255,
                        };
                        [
                            (19 + x * 11 + y * 3) as u8,
                            (37 + x * 5 + y * 13) as u8,
                            (53 + x * 17 + y * 7) as u8,
                            alpha,
                        ]
                    })
                })
                .collect(),
        );
        let settings = SkySettings {
            has_surface: true,
            width: IMAGE_WIDTH,
            height: IMAGE_HEIGHT,
            parallax_x: 7,
            parallax_y: 13,
            modulation: Some(0x2080_c0f0),
            ..SkySettings::default()
        };
        let frame = SkyFrame {
            settings: settings.clone(),
            offset_x: 3.375,
            offset_y: -2.625,
            fixed: None,
        };
        let fog = Arc::new(ClrModMap {
            resolution_x: 19,
            resolution_y: 17,
            width: 14,
            height: 12,
            origin_x: -48,
            origin_y: -35,
            fade_transparent: false,
            cells: (0..168)
                .map(|index| {
                    let transparency = (index * 13 % 80) as u32;
                    let red = (64 + index * 17 % 192) as u32;
                    let green = (48 + index * 29 % 208) as u32;
                    let blue = (32 + index * 43 % 224) as u32;
                    (transparency << 24) | (red << 16) | (green << 8) | blue
                })
                .collect(),
        });
        let make_graphics = || {
            let mut graphics = test_graphics(
                SURFACE_WIDTH,
                SURFACE_HEIGHT,
                SURFACE_HEIGHT as i32,
                "parallel sky rows",
            );
            graphics.viewport_x = 18.75;
            graphics.viewport_y = -7.25;
            graphics.viewport_zoom = 1.3;
            graphics.active_fog_map = Some(Arc::clone(&fog));
            for (index, pixel) in graphics
                .surface_mut()
                .pixels_mut()
                .chunks_exact_mut(4)
                .enumerate()
            {
                let x = (index as u32 % SURFACE_WIDTH) as u8;
                let y = (index as u32 / SURFACE_WIDTH) as u8;
                pixel.copy_from_slice(&[
                    11u8.wrapping_add(x),
                    23u8.wrapping_add(y),
                    41u8.wrapping_add(x ^ y),
                    255,
                ]);
            }
            graphics
                .surface_mut()
                .set_clip(SurfaceRect::new(6, 4, 161, 119));
            graphics
        };
        let gamma = clonk_graphics::GammaRamp::from_control_points([0x000000, 0x646464, 0xc8c8c8]);

        let mut scalar = make_graphics();
        scalar.tile_sky_image_with_parallel_rows(
            &image,
            &settings,
            Some(&frame),
            0.67,
            Some(&gamma),
            false,
        );
        let mut parallel = make_graphics();
        parallel.tile_sky_image_with_parallel_rows(
            &image,
            &settings,
            Some(&frame),
            0.67,
            Some(&gamma),
            true,
        );

        assert_eq!(parallel.surface().pixels(), scalar.surface().pixels());
        assert_eq!(
            scalar.surface().get_pixel(0, 0),
            Some(Color::opaque(11, 23, 41)),
            "the direct row path must retain pixels outside the surface clip",
        );
        assert!(
            (4..123).any(|y| {
                (6..167).any(|x| {
                    let background = Color::opaque(
                        11u8.wrapping_add(x as u8),
                        23u8.wrapping_add(y as u8),
                        41u8.wrapping_add(x as u8 ^ y as u8),
                    );
                    scalar.surface().get_pixel(x, y) != Some(background)
                })
            }),
            "the cropped, rounded tiles must exercise the clipped draw region",
        );
    }

    #[test]
    fn half_pixel_sky_offset_keeps_repeated_tiles_contiguous() {
        // C4Sky passes integer parallax offsets to BlitSurfaceTile2
        // (src/C4Sky.cpp:215-217), which advances from one integer origin by
        // exact tile extents (src/StdDDraw2.cpp:1005-1029). A fractional
        // intermediate parallax phase must not expose the backing surface.
        let tile = [
            Color::opaque(47, 139, 211),
            Color::opaque(211, 139, 47),
            Color::opaque(139, 47, 211),
            Color::opaque(47, 211, 139),
        ];
        let image = ImageData::new(
            2,
            2,
            tile.iter()
                .flat_map(|color| [color.r, color.g, color.b, color.a])
                .collect(),
        );
        let mut graphics = test_graphics(5, 5, 5, "contiguous sky tiles");
        graphics.surface_mut().fill(Color::opaque(1, 1, 1));
        graphics.viewport_x = 1.0;
        graphics.viewport_y = 1.0;
        let settings = SkySettings {
            parallax_x: 20,
            parallax_y: 20,
            ..SkySettings::default().with_surface(2, 2)
        };

        graphics.tile_sky_image_with_parallel_rows(&image, &settings, None, 1.0, None, false);

        for y in 0..5 {
            for x in 0..5 {
                assert_eq!(
                    graphics.surface().get_pixel(x, y),
                    Some(tile[((y % 2) * 2 + x % 2) as usize]),
                    "native integer phase covers ({x}, {y})"
                );
            }
        }
    }

    #[test]
    fn sky_scroll_uses_raw_fixed_rounding_for_the_tile_phase() {
        // C4Sky subtracts `fixtoi(x)` before tiling (src/C4Sky.cpp:215-217),
        // and C4Fixed rounds the raw 16.16 word rather than its float
        // projection (src/Fixed.h:82-94). In particular, -0.5 rounds to 0.
        let left = Color::opaque(31, 97, 211);
        let right = Color::opaque(211, 97, 31);
        let image = ImageData::new(
            2,
            1,
            [left, right]
                .into_iter()
                .flat_map(|color| [color.r, color.g, color.b, color.a])
                .collect(),
        );
        let settings = SkySettings {
            parallax_x: 20,
            ..SkySettings::default().with_surface(2, 1)
        };
        let frame = SkyFrame {
            settings: settings.clone(),
            offset_x: -0.5,
            offset_y: 0.0,
            fixed: Some([-(1 << 15), 0, 0, 0]),
        };
        let mut graphics = test_graphics(3, 1, 1, "fixed sky phase");
        graphics.viewport_x = 1.0;

        graphics.tile_sky_image_with_parallel_rows(
            &image,
            &settings,
            Some(&frame),
            1.0,
            None,
            false,
        );

        assert_eq!(graphics.surface().get_pixel(0, 0), Some(left));
        assert_eq!(graphics.surface().get_pixel(1, 0), Some(right));
        assert_eq!(graphics.surface().get_pixel(2, 0), Some(left));
    }

    #[test]
    fn clr_mod_map_squared_reveal_and_generator_values_match_cpp() {
        let mut reveal = ClrModMap::reset(64, 64, 256, 256, 0, 0, 0, 0, 0).unwrap();
        reveal.reduce_modulation(64, 64, 64, 96);
        assert_eq!(reveal.get_mod_at(128, 128), 0x0033_3333);

        let mut alpha_reveal = ClrModMap::reset(64, 64, 256, 256, 0, 0, 0, 0, 0x0000_0001).unwrap();
        alpha_reveal.reduce_modulation(64, 64, 64, 96);
        assert_eq!(alpha_reveal.get_mod_at(128, 128), 0xccff_ffff);

        let mut generator = ClrModMap::reset(64, 64, 256, 256, 0, 0, 0, 0, 0).unwrap();
        generator.reduce_modulation(0, 0, 10_000, 11_000);
        generator.add_modulation(0, 0, 64, 264, 0);
        assert_eq!(generator.get_mod_at(128, 0), 0x0030_3030);

        let mut alpha_generator =
            ClrModMap::reset(64, 64, 256, 256, 0, 0, 0, 0, 0x0000_0001).unwrap();
        alpha_generator.reduce_modulation(0, 0, 10_000, 11_000);
        alpha_generator.add_modulation(0, 0, 64, 264, 64);
        assert_eq!(alpha_generator.get_mod_at(128, 0), 0x8fff_ffff);
    }

    #[test]
    fn packed_fog_modulation_keeps_native_shift_and_transparency_math() {
        let color = Color::new(200, 100, 50, 245);

        assert_eq!(
            modulate_surface_color(color, 0x0080_8080),
            Color::new(100, 50, 25, 245)
        );
        assert_eq!(
            modulate_surface_color(color, 0x8080_8080),
            Color::new(100, 50, 25, 122)
        );
        assert_eq!(
            modulate_surface_color(color, 0x00ff_ffff),
            Color::new(199, 99, 49, 245)
        );
    }

    #[test]
    fn fog_map_defers_generators_skips_closed_containers_and_adds_view_target() {
        let mut snapshot = make_snapshot();
        snapshot.environment.fow_resolution = 16;
        let mut repeller = snapshot.objects[0].clone();
        repeller.id = ObjectId::new(1);
        repeller.position = Vector2::new(96, 64);
        repeller.plr_view_range = 90;

        let mut generator = repeller.clone();
        generator.id = ObjectId::new(2);
        generator.plr_view_range = -20;
        generator.color_modulation = 0;

        let mut container = repeller.clone();
        container.id = ObjectId::new(3);
        container.definition_id = "Closed".into();
        container.plr_view_range = 0;

        let mut hidden_repeller = repeller.clone();
        hidden_repeller.id = ObjectId::new(4);
        hidden_repeller.position = Vector2::new(192, 64);
        hidden_repeller.plr_view_range = 50;
        hidden_repeller.container = Some(container.id);

        let mut open_container = container.clone();
        open_container.id = ObjectId::new(7);
        open_container.definition_id = "ClosedTwo".into();
        let mut visible_repeller = repeller.clone();
        visible_repeller.id = ObjectId::new(8);
        visible_repeller.position = Vector2::new(416, 64);
        visible_repeller.plr_view_range = 50;
        visible_repeller.container = Some(open_container.id);

        let mut target = repeller.clone();
        target.id = ObjectId::new(5);
        target.position = Vector2::new(336, 64);
        target.plr_view_range = 0;

        let mut cursor = repeller.clone();
        cursor.id = ObjectId::new(6);
        cursor.plr_view_range = 60;
        snapshot.objects = vec![
            repeller,
            generator,
            container,
            hidden_repeller,
            target,
            cursor,
            open_container,
            visible_repeller,
        ];
        snapshot
            .definition_closed_containers
            .insert("Closed".into(), 1);
        snapshot
            .definition_closed_containers
            .insert("ClosedTwo".into(), 2);
        // Generator intentionally precedes the repeller: native still applies
        // it last and leaves their shared center black.
        snapshot.fow_players.insert(
            0,
            FogOfWarPlayerFrame {
                view_objects: vec![
                    ObjectId::new(2),
                    ObjectId::new(1),
                    ObjectId::new(4),
                    ObjectId::new(8),
                ],
                view_target: Some(ObjectId::new(5)),
            },
        );
        snapshot.players = vec![PlayerState {
            id: 0,
            fog_of_war: true,
            view_mode: PLAYER_VIEW_MODE_TARGET,
            view_target: Some(ObjectId::new(5)),
            cursor: Some(ObjectId::new(6)),
            ..PlayerState::default()
        }];

        let map = build_fog_modulation_map(&snapshot, 0, 0, 0, 480, 128).unwrap();
        assert_eq!(map.get_mod_at(96, 64), 0, "generator wins after reveal");
        assert_eq!(
            map.get_mod_at(192, 64),
            0,
            "ClosedContainer==1 suppresses the contained repeller"
        );
        assert_eq!(
            map.get_mod_at(336, 64),
            0x00ff_ffff,
            "target uses the cursor's nonzero fallback range"
        );
        assert_eq!(
            map.get_mod_at(416, 64),
            0x00ff_ffff,
            "ClosedContainer==2 explicitly retains outward vision"
        );

        snapshot
            .objects
            .iter_mut()
            .find(|object| object.id == ObjectId::new(5))
            .unwrap()
            .position = Vector2::new(500, 64);
        snapshot
            .objects
            .iter_mut()
            .find(|object| object.id == ObjectId::new(6))
            .unwrap()
            .plr_view_range = 0;
        let fow_player = snapshot.fow_players.get_mut(&0).unwrap();
        fow_player.view_objects.clear();
        let default_target = build_fog_modulation_map(&snapshot, 0, 0, 0, 800, 128).unwrap();
        assert_ne!(
            default_target.get_mod_at(100, 64) & 0x00ff_ffff,
            0,
            "zero target and cursor ranges fall back to the native 500px range"
        );
        assert_eq!(
            default_target.get_mod_at(0, 64) & 0x00ff_ffff,
            0,
            "the fallback reveal excludes its exact outer-radius boundary"
        );
    }

    #[test]
    fn ignore_fow_suppresses_only_an_object_base_draw() {
        let sprite = DefinitionSprite {
            image: ImageData::new(1, 1, vec![255, 255, 255, 255]),
            actions: HashMap::new(),
            color_mask: None,
            graphics_scale: 1.0,
            shape: Some(DefinitionRect::new(0, 0, 1, 1)),
            fire_top: 0,
            rotateable: 0,
            line: 0,
            stretch_growth: false,
            top_face: None,
            picture: None,
        };
        let mut object = make_snapshot().objects.remove(0);
        object.definition_id = "IgnoreFog".into();
        object.position = Vector2::new(1, 1);
        object.category = clonk_engine::DEFAULT_CATEGORY | CATEGORY_IGNORE_FOW_FLAG;
        object.color_modulation = 0;
        object.blit_mode = 0;
        object.action = Default::default();

        let mut graphics = GraphicsSystem::new(
            3,
            3,
            3,
            "Ignore FoW",
            test_font(),
            Arc::new(HashMap::from([(sprite_map_key("IgnoreFog", None), sprite)])),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics.surface_mut().fill(Color::opaque(0, 0, 0));
        graphics.active_fog_map = Some(Arc::new(ClrModMap {
            resolution_x: 64,
            resolution_y: 64,
            width: 2,
            height: 2,
            origin_x: 0,
            origin_y: 0,
            fade_transparent: false,
            cells: vec![0; 4],
        }));
        graphics.draw_objects(
            std::slice::from_ref(&object),
            &[object.id],
            &HashMap::new(),
            &[],
            OWNER_NONE,
            1.0,
            &HashMap::new(),
            ObjectRenderPass::Normal,
            None,
        );
        assert_eq!(
            graphics.surface().get_pixel(1, 1),
            Some(Color::opaque(255, 255, 255))
        );

        object.category &= !CATEGORY_IGNORE_FOW_FLAG;
        graphics.surface_mut().fill(Color::opaque(0, 0, 0));
        graphics.draw_objects(
            std::slice::from_ref(&object),
            &[object.id],
            &HashMap::new(),
            &[],
            OWNER_NONE,
            1.0,
            &HashMap::new(),
            ObjectRenderPass::Normal,
            None,
        );
        assert_eq!(
            graphics.surface().get_pixel(1, 1),
            Some(Color::opaque(0, 0, 0))
        );
    }

    #[test]
    fn renderer_resolves_raw_draw_dir_and_flip_dir_rows() {
        // C4Object::UpdateFlipDir keeps raw Action.Dir and computes
        // DrawDir=2*FlipDir-1-Dir for the mirrored half
        // (C4Object.cpp:404-430).
        let banner = DefinitionActionGraphics {
            directions: 14,
            flip_dir: Some(7),
            ..DefinitionActionGraphics::default()
        };
        for (raw, expected_row, expected_mirror) in [(13, 0, true), (7, 6, true)] {
            let direction = Direction::from_script_value(raw);
            assert_eq!(
                GraphicsSystem::resolve_draw_direction(&banner, direction),
                expected_row
            );
            assert_eq!(
                GraphicsSystem::resolve_overlay_action_flip(&banner, direction),
                expected_mirror
            );
        }

        let flag = DefinitionActionGraphics {
            directions: 9,
            ..DefinitionActionGraphics::default()
        };
        let direction = Direction::from_script_value(4);
        assert_eq!(GraphicsSystem::resolve_draw_direction(&flag, direction), 4);
        assert!(!GraphicsSystem::resolve_overlay_action_flip(
            &flag, direction
        ));

        let malformed = DefinitionActionGraphics {
            flip_dir: Some(-2),
            ..DefinitionActionGraphics::default()
        };
        assert_eq!(
            GraphicsSystem::resolve_draw_direction(&malformed, Direction::from_script_value(0),),
            -5,
            "negative FlipDir remains truthy and uses the signed C++ formula"
        );
        assert!(GraphicsSystem::resolve_overlay_action_flip(
            &malformed,
            Direction::from_script_value(0),
        ));
    }

    #[test]
    fn renderer_uses_physical_action_graphics_for_duplicate_names() {
        let first = DefinitionActionGraphics {
            length: Some(2),
            ..DefinitionActionGraphics::default()
        };
        let last = DefinitionActionGraphics {
            length: Some(5),
            ..DefinitionActionGraphics::default()
        };
        let actions = HashMap::from([
            ("Dup".to_string(), first),
            (physical_action_graphics_key(1), last),
            (
                PHYSICAL_ACTION_GRAPHICS_MARKER.to_string(),
                DefinitionActionGraphics::default(),
            ),
        ]);
        let physical = clonk_engine::ActionState {
            act_map_index: Some(1),
            ..clonk_engine::ActionState::new("Dup")
        };
        assert_eq!(
            GraphicsSystem::live_action_graphics(&actions, &physical)
                .and_then(|graphics| graphics.length),
            Some(5)
        );

        let idle = clonk_engine::ActionState::new("Idle");
        assert!(GraphicsSystem::live_action_graphics(&actions, &idle).is_none());
    }

    #[test]
    fn top_face_uses_live_physical_facet_phase_reverse_and_draw_dir() {
        // DrawTopFace selects Def->ActMap[Action.Act], not the first action
        // with the same name. FacetTopFace offsets the definition TopFace by
        // the live reversed Phase and DrawDir (src/C4Object.cpp:2639-2647).
        let red = Color::opaque(180, 0, 0);
        let green = Color::opaque(0, 200, 0);
        let mut pixels = [red.r, red.g, red.b, red.a].repeat(16 * 8);
        let expected_source = (7usize, 4usize);
        let expected_offset = (expected_source.1 * 16 + expected_source.0) * 4;
        pixels[expected_offset..expected_offset + 4]
            .copy_from_slice(&[green.r, green.g, green.b, green.a]);

        let named_first = DefinitionActionGraphics {
            facet: Some(clonk_engine::DefinitionActionFacet {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
                target_x: 0,
                target_y: 0,
            }),
            facet_top_face: true,
            length: Some(1),
            ..DefinitionActionGraphics::default()
        };
        let physical_second = DefinitionActionGraphics {
            facet: Some(clonk_engine::DefinitionActionFacet {
                x: 2,
                y: 1,
                width: 2,
                height: 1,
                target_x: 99,
                target_y: 99,
            }),
            reverse: true,
            facet_top_face: true,
            length: Some(3),
            ..DefinitionActionGraphics::default()
        };
        let sprite = DefinitionSprite {
            graphics_scale: 1.0,
            image: ImageData::new(16, 8, pixels),
            actions: HashMap::from([
                ("Dup".to_string(), named_first),
                (physical_action_graphics_key(1), physical_second),
                (
                    PHYSICAL_ACTION_GRAPHICS_MARKER.to_string(),
                    DefinitionActionGraphics::default(),
                ),
            ]),
            color_mask: None,
            shape: Some(DefinitionRect::new(0, 0, 4, 4)),
            fire_top: 0,
            rotateable: 0,
            line: 0,
            stretch_growth: false,
            // Expected source: Facet(2,1) + TopFace(1,1)
            // + FacetSize(2,1) * (reversed phase 2, DrawDir 2) = (7,4).
            top_face: Some(DefinitionTargetRect::new(1, 1, 1, 1, 0, 0)),
            picture: None,
        };
        let mut object = make_snapshot().objects.remove(0);
        object.position = Vector2::new(10, 10);
        object.action = clonk_engine::ActionState::new("Dup");
        object.action.act_map_index = Some(1);
        object.action.phase = 0;
        object.direction = Direction::from_script_value(2);

        let sprites = Arc::new(HashMap::from([(
            sprite_map_key("TestObject", None),
            sprite,
        )]));
        let mut graphics = GraphicsSystem::new(
            24,
            24,
            24,
            "FacetTopFace physical slot",
            test_font(),
            sprites,
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics.surface_mut().fill(Color::opaque(0, 0, 0));
        graphics.paint_object_top_face(&object, SpriteBlitState::for_object(&object), None);

        assert_eq!(
            graphics.surface().get_pixel(10, 10),
            Some(green),
            "the physical duplicate's live FacetTopFace source must win"
        );
    }

    #[test]
    fn top_face_with_cross_definition_graphics_uses_live_definition_metadata() {
        // SetGraphics may source the bitmap from another definition, but
        // UpdateFace and DrawTopFace still read Shape and TopFace from the
        // object's live Def. Source sampling alone uses the selected
        // graphics definition's Scale (C4Object.cpp:357-376,2639-2670).
        let blue = Color::opaque(0, 0, 180);
        let red = Color::opaque(180, 0, 0);
        let green = Color::opaque(0, 200, 0);
        let mut override_pixels = [red.r, red.g, red.b, red.a].repeat(12 * 8);
        // The live TopFace source (2,1,2,2) is mapped through the selected
        // override bitmap's 2x scale to (4,2,4,4). A 2x2 destination samples
        // these four physical pixels; omitting width/height scaling samples
        // neighboring red pixels instead.
        // Point filtering still samples GL texel centers: the 4x4 source
        // scaled to 2x2 lands on offsets (1,1) and (3,3).
        for (x, y) in [(5usize, 3usize), (7, 3), (5, 5), (7, 5)] {
            let offset = (y * 12 + x) * 4;
            override_pixels[offset..offset + 4]
                .copy_from_slice(&[green.r, green.g, green.b, green.a]);
        }

        let definition_sprite = DefinitionSprite {
            graphics_scale: 1.0,
            image: ImageData::new(12, 8, [blue.r, blue.g, blue.b, blue.a].repeat(12 * 8)),
            actions: HashMap::new(),
            color_mask: None,
            shape: Some(DefinitionRect::new(-2, -1, 4, 2)),
            fire_top: 0,
            rotateable: 0,
            line: 0,
            stretch_growth: false,
            top_face: Some(DefinitionTargetRect::new(2, 1, 2, 2, 1, 0)),
            picture: None,
        };
        let override_sprite = DefinitionSprite {
            graphics_scale: 2.0,
            image: ImageData::new(12, 8, override_pixels),
            actions: HashMap::new(),
            color_mask: None,
            // Deliberately conflicting metadata: the old path used these
            // coordinates and drew a red pixel at (14,12).
            shape: Some(DefinitionRect::new(2, 2, 2, 2)),
            fire_top: 0,
            rotateable: 0,
            line: 0,
            stretch_growth: false,
            top_face: Some(DefinitionTargetRect::new(0, 0, 1, 1, 0, 0)),
            picture: None,
        };
        let sprites = Arc::new(HashMap::from([
            (sprite_map_key("TestObject", None), definition_sprite),
            (sprite_map_key("OverrideSheet", None), override_sprite),
        ]));
        let mut object = make_snapshot().objects.remove(0);
        object.position = Vector2::new(12, 10);
        object.base_graphics = Some(clonk_engine::ObjectBaseGraphics {
            definition: "OverrideSheet".to_string(),
            graphics_name: None,
            blit_mode: 0,
        });

        let mut graphics = GraphicsSystem::new(
            24,
            20,
            20,
            "cross-definition plain TopFace",
            test_font(),
            sprites,
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics.set_point_filtering(true);
        let background = Color::opaque(0, 0, 0);
        graphics.surface_mut().fill(background);
        graphics.paint_object_top_face(&object, SpriteBlitState::for_object(&object), None);

        assert_eq!(
            [
                graphics.surface().get_pixel(11, 9),
                graphics.surface().get_pixel(12, 9),
                graphics.surface().get_pixel(11, 10),
                graphics.surface().get_pixel(12, 10),
            ],
            [Some(green); 4],
            "live TopFace metadata must sample the scaled override source rectangle"
        );
        assert_eq!(
            graphics.surface().get_pixel(13, 9),
            Some(background),
            "the selected bitmap scale must not enlarge destination geometry"
        );
        assert_eq!(
            graphics.surface().get_pixel(14, 12),
            Some(background),
            "override-definition TopFace metadata must not relocate the draw"
        );
    }

    #[test]
    fn fractional_selected_graphics_scale_keeps_subpixel_top_face_sources() {
        // C4Facet forwards the float source extent to Blit. Scale=0.5 turns
        // this logical 1x1 TopFace into a non-empty 0.5x0.5 source; integer
        // truncation used to discard the draw entirely.
        let background = Color::opaque(0, 0, 0);
        let green = Color::opaque(0, 200, 0);
        let red = Color::opaque(180, 0, 0);
        let mut pixels = [red.r, red.g, red.b, red.a].repeat(4);
        pixels[12..16].copy_from_slice(&[green.r, green.g, green.b, green.a]);

        let definition_sprite = DefinitionSprite {
            graphics_scale: 1.0,
            image: ImageData::new(2, 2, [0, 0, 180, 255].repeat(4)),
            actions: HashMap::new(),
            color_mask: None,
            shape: Some(DefinitionRect::new(0, 0, 1, 1)),
            fire_top: 0,
            rotateable: 0,
            line: 0,
            stretch_growth: false,
            top_face: Some(DefinitionTargetRect::new(2, 2, 1, 1, 0, 0)),
            picture: None,
        };
        let override_sprite = DefinitionSprite {
            graphics_scale: 0.5,
            image: ImageData::new(2, 2, pixels),
            actions: HashMap::new(),
            color_mask: None,
            shape: None,
            fire_top: 0,
            rotateable: 0,
            line: 0,
            stretch_growth: false,
            top_face: None,
            picture: None,
        };
        let sprites = Arc::new(HashMap::from([
            (sprite_map_key("TestObject", None), definition_sprite),
            (sprite_map_key("HalfScale", None), override_sprite),
        ]));
        let mut object = make_snapshot().objects.remove(0);
        object.position = Vector2::new(5, 5);
        object.base_graphics = Some(clonk_engine::ObjectBaseGraphics {
            definition: "HalfScale".to_string(),
            graphics_name: None,
            blit_mode: 0,
        });

        let mut graphics = GraphicsSystem::new(
            12,
            12,
            12,
            "fractional TopFace scale",
            test_font(),
            sprites,
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics.set_point_filtering(true);
        graphics.surface_mut().fill(background);
        graphics.paint_object_top_face(&object, SpriteBlitState::for_object(&object), None);

        assert_eq!(graphics.surface().get_pixel(5, 5), Some(green));
        assert_eq!(
            graphics.surface().get_pixel(6, 5),
            Some(background),
            "fractional source scale must not alter the 1x1 destination"
        );
    }

    #[test]
    fn fractional_source_extent_survives_straight_and_transformed_object_blits() {
        // Logical (1,0,2,1) at Scale=1.25 is the float source
        // (1.25,0,2.5,1.25). The last of four destination samples reaches
        // source x=3; the old integer cast collapsed the extent to x=1..2.
        let red = Color::opaque(200, 0, 0);
        let green = Color::opaque(0, 200, 0);
        let blue = Color::opaque(0, 0, 200);
        let row = [
            0, 0, 0, 255, red.r, red.g, red.b, red.a, green.r, green.g, green.b, green.a, blue.r,
            blue.g, blue.b, blue.a,
        ];
        let sprite = DefinitionSprite {
            graphics_scale: 1.25,
            image: ImageData::new(4, 2, row.repeat(2)),
            actions: HashMap::new(),
            color_mask: None,
            shape: Some(DefinitionRect::new(0, 0, 4, 1)),
            fire_top: 0,
            rotateable: 0,
            line: 0,
            stretch_growth: false,
            top_face: None,
            picture: None,
        };
        let background = Color::opaque(0, 0, 0);
        let render = |source, transform| {
            let mut graphics = test_graphics(4, 2, 2, "fractional object source");
            graphics.set_point_filtering(true);
            graphics.surface_mut().fill(background);
            graphics.blit_face(
                &sprite,
                source,
                (0.0, 0.0, 4.0, 1.0),
                (2.0, 0.5),
                None,
                1.0,
                0.0,
                transform,
                SpriteBlitState::normal(),
                None,
            );
            (0..4)
                .map(|x| graphics.surface().get_pixel(x, 0))
                .collect::<Vec<_>>()
        };

        assert_eq!(
            render(SourceRect::new(1, 0, 2, 1), None),
            vec![Some(red), Some(green), Some(green), Some(blue)]
        );
        assert_eq!(
            render(SourceRect::new(1, 0, 2, 1), Some(DrawTransform::identity()),),
            vec![Some(red), Some(green), Some(green), Some(blue)]
        );
        // Logical x=2 scales to x=2.5 with width 2.5. Only 1.5 source
        // pixels remain before the four-pixel sheet edge, so the target is
        // clipped by 1.5/2.5 and rounds to two rendered pixels.
        let clipped = vec![Some(green), Some(blue), Some(background), Some(background)];
        assert_eq!(render(SourceRect::new(2, 0, 2, 1), None), clipped);
        assert_eq!(
            render(SourceRect::new(2, 0, 2, 1), Some(DrawTransform::identity()),),
            clipped
        );
    }

    #[test]
    fn facet_top_face_with_cross_definition_graphics_uses_live_act_map_and_flip_dir() {
        // FacetTopFace source offsets and UpdateFlipDir come from the live
        // object's ActMap even when SetGraphics selected a different Def's
        // bitmap (C4Object.cpp:404-425,2639-2667).
        let blue = Color::opaque(0, 0, 180);
        let red = Color::opaque(180, 0, 0);
        let green = Color::opaque(0, 200, 0);
        let mut override_pixels = [red.r, red.g, red.b, red.a].repeat(8 * 4);
        // Live metadata computes Facet(2,1) + TopFace(1,1)
        // + FacetSize(2,1) * (Phase 1, mirrored DrawDir 0) = (5,2).
        let live_source = (5usize, 2usize);
        let live_source_offset = (live_source.1 * 8 + live_source.0) * 4;
        override_pixels[live_source_offset..live_source_offset + 4]
            .copy_from_slice(&[green.r, green.g, green.b, green.a]);

        let live_action = DefinitionActionGraphics {
            facet: Some(clonk_engine::DefinitionActionFacet {
                x: 2,
                y: 1,
                width: 2,
                height: 1,
                target_x: 99,
                target_y: 99,
            }),
            directions: 2,
            flip_dir: Some(1),
            facet_top_face: true,
            length: Some(3),
            ..DefinitionActionGraphics::default()
        };
        let definition_sprite = DefinitionSprite {
            graphics_scale: 1.0,
            image: ImageData::new(8, 4, [blue.r, blue.g, blue.b, blue.a].repeat(8 * 4)),
            actions: HashMap::from([("Active".to_string(), live_action)]),
            color_mask: None,
            shape: Some(DefinitionRect::new(-3, -1, 6, 2)),
            fire_top: 0,
            rotateable: 0,
            line: 0,
            stretch_growth: false,
            top_face: Some(DefinitionTargetRect::new(1, 1, 1, 1, 0, 0)),
            picture: None,
        };
        let override_sprite = DefinitionSprite {
            graphics_scale: 1.0,
            image: ImageData::new(8, 4, override_pixels),
            actions: HashMap::from([("Active".to_string(), DefinitionActionGraphics::default())]),
            color_mask: None,
            shape: Some(DefinitionRect::new(0, 0, 2, 2)),
            fire_top: 0,
            rotateable: 0,
            line: 0,
            stretch_growth: false,
            top_face: Some(DefinitionTargetRect::new(0, 0, 1, 1, 0, 0)),
            picture: None,
        };
        let sprites = Arc::new(HashMap::from([
            (sprite_map_key("TestObject", None), definition_sprite),
            (sprite_map_key("OverrideSheet", None), override_sprite),
        ]));
        let mut object = make_snapshot().objects.remove(0);
        object.position = Vector2::new(12, 8);
        object.action = clonk_engine::ActionState::new("Active");
        object.action.phase = 1;
        object.direction = Direction::from_script_value(1);
        // UpdateFlipDir owns the mirror: it folds into pDrawTransform, and
        // C4Object::Draw hands that matrix straight to the blit
        // (src/C4Object.cpp:415-428,2506-2515). Source it from the ported
        // engine function so this stays an assertion about the real fold.
        object.draw_transform = DrawTransform::updated_flip_dir(object.draw_transform, 1, 1);
        object.base_graphics = Some(clonk_engine::ObjectBaseGraphics {
            definition: "OverrideSheet".to_string(),
            graphics_name: None,
            blit_mode: 0,
        });

        let mut graphics = GraphicsSystem::new(
            24,
            16,
            16,
            "cross-definition FacetTopFace",
            test_font(),
            sprites,
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        let background = Color::opaque(0, 0, 0);
        graphics.surface_mut().fill(background);
        graphics.paint_object_top_face(&object, SpriteBlitState::for_object(&object), None);

        assert_eq!(
            graphics.surface().get_pixel(14, 7),
            Some(green),
            "live FacetTopFace must sample the override bitmap and FlipDir must mirror it"
        );
        assert_eq!(
            graphics.surface().get_pixel(9, 7),
            Some(background),
            "the live FlipDir transform must move the unflipped target"
        );
    }

    #[test]
    fn flip_dir_mirrors_plain_definition_top_face() {
        // UpdateFlipDir owns pDrawTransform, so its mirror applies even when
        // FacetTopFace is false and the source remains Def->TopFace
        // (src/C4Object.cpp:404-430,2639-2668).
        let green = Color::opaque(0, 200, 0);
        let action = DefinitionActionGraphics {
            flip_dir: Some(1),
            facet_top_face: false,
            ..DefinitionActionGraphics::default()
        };
        let sprite = DefinitionSprite {
            graphics_scale: 1.0,
            image: ImageData::new(1, 1, vec![green.r, green.g, green.b, green.a]),
            actions: HashMap::from([("Active".to_string(), action)]),
            color_mask: None,
            shape: Some(DefinitionRect::new(-3, -1, 6, 2)),
            fire_top: 0,
            rotateable: 0,
            line: 0,
            stretch_growth: false,
            top_face: Some(DefinitionTargetRect::new(0, 0, 1, 1, 0, 0)),
            picture: None,
        };
        let sprites = Arc::new(HashMap::from([(
            sprite_map_key("TestObject", None),
            sprite,
        )]));
        let render = |direction| {
            let mut object = make_snapshot().objects.remove(0);
            object.position = Vector2::new(12, 8);
            object.action = clonk_engine::ActionState::new("Active");
            object.direction = Direction::from_script_value(direction);
            // Same fold, sourced from the engine port rather than
            // hand-built (src/C4Object.cpp:415-428).
            object.draw_transform =
                DrawTransform::updated_flip_dir(object.draw_transform, direction, 1);
            let mut graphics = GraphicsSystem::new(
                24,
                16,
                16,
                "plain TopFace FlipDir",
                test_font(),
                Arc::clone(&sprites),
                empty_cursor_atlas(),
                empty_hud_graphics(),
            );
            graphics.surface_mut().fill(Color::opaque(0, 0, 0));
            graphics.paint_object_top_face(&object, SpriteBlitState::for_object(&object), None);
            graphics.surface().clone()
        };

        let unflipped = render(0);
        let flipped = render(1);
        assert_eq!(unflipped.get_pixel(9, 7), Some(green));
        assert_eq!(flipped.get_pixel(14, 7), Some(green));
        assert_ne!(
            flipped.get_pixel(9, 7),
            Some(green),
            "FlipDir must move the plain TopFace across the shape center"
        );
    }

    #[test]
    fn partial_growth_type_scales_and_draws_its_top_face_like_cpp() {
        // UpdateFace retains TopFace below FullCon for GrowthType defs, and
        // DrawTopFace uses DrawXT with Con-scaled offsets and dimensions
        // (src/C4Object.cpp:370-376,2653-2662).
        let green = Color::opaque(0, 200, 0);
        let sprite = DefinitionSprite {
            graphics_scale: 1.0,
            image: ImageData::new(2, 2, [green.r, green.g, green.b, green.a].repeat(4)),
            actions: HashMap::new(),
            color_mask: None,
            shape: Some(DefinitionRect::new(-2, -2, 4, 4)),
            fire_top: 0,
            rotateable: 0,
            line: 0,
            stretch_growth: true,
            top_face: Some(DefinitionTargetRect::new(0, 0, 2, 2, 1, 1)),
            picture: None,
        };
        let sprites = Arc::new(HashMap::from([(
            sprite_map_key("TestObject", None),
            sprite,
        )]));
        let mut object = make_snapshot().objects.remove(0);
        object.position = Vector2::new(10, 10);
        object.construction = FULL_CON / 2;
        let mut graphics = GraphicsSystem::new(
            20,
            20,
            20,
            "partial GrowthType TopFace",
            test_font(),
            sprites,
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics.surface_mut().fill(Color::opaque(0, 0, 0));

        graphics.paint_object_top_face(&object, SpriteBlitState::for_object(&object), None);

        assert_eq!(graphics.surface().get_pixel(9, 9), Some(green));
        assert_eq!(
            graphics.surface().get_pixel(10, 9),
            Some(Color::opaque(0, 0, 0)),
            "a 2x2 TopFace scales to exactly 1x1 at fifty-percent Con"
        );
    }

    #[test]
    fn set_obj_draw_transform_rotation_reaches_presenter() {
        let sprite = DefinitionSprite {
            graphics_scale: 1.0,
            image: ImageData::new(9, 3, [220, 40, 20, 255].repeat(27)),
            actions: HashMap::new(),
            color_mask: None,
            shape: Some(DefinitionRect::new(-4, -1, 9, 3)),
            fire_top: 0,
            rotateable: 0,
            line: 0,
            stretch_growth: false,
            top_face: None,
            picture: None,
        };
        let render = |transform| {
            let mut graphics = test_graphics(24, 24, 24, "Draw transform");
            graphics.blit_face(
                &sprite,
                SourceRect::new(0, 0, 9, 3),
                (6.0, 9.0, 9.0, 3.0),
                (10.5, 10.5),
                None,
                1.0,
                0.0,
                transform,
                SpriteBlitState::normal(),
                None,
            );
            let mut min_x = i32::MAX;
            let mut min_y = i32::MAX;
            let mut max_x = i32::MIN;
            let mut max_y = i32::MIN;
            for y in 0..graphics.surface().height() {
                for x in 0..graphics.surface().width() {
                    if graphics
                        .surface()
                        .get_pixel(x, y)
                        .is_some_and(|pixel| pixel == Color::opaque(220, 40, 20))
                    {
                        min_x = min_x.min(x as i32);
                        min_y = min_y.min(y as i32);
                        max_x = max_x.max(x as i32);
                        max_y = max_y.max(y as i32);
                    }
                }
            }
            (min_x, min_y, max_x, max_y)
        };

        let straight = render(None);
        let rotated = render(Some(DrawTransform::from_matrix([
            0.866, -0.5, 0.0, 0.5, 0.866, 0.0, 0.0, 0.0, 1.0,
        ])));
        assert_eq!(straight.3 - straight.1 + 1, 3);
        assert!(
            rotated.3 - rotated.1 + 1 >= 6,
            "30-degree b/d rotation must increase the 9x3 sprite's vertical span: {rotated:?}"
        );
        assert!(
            rotated.2 - rotated.0 + 1 >= 8,
            "rotated sprite unexpectedly collapsed: {rotated:?}"
        );
    }

    #[test]
    fn overlay_draw_transform_uses_its_full_local_matrix() {
        let sprite = DefinitionSprite {
            graphics_scale: 1.0,
            image: ImageData::new(9, 3, [30, 180, 70, 255].repeat(27)),
            actions: HashMap::new(),
            color_mask: None,
            shape: Some(DefinitionRect::new(-4, -1, 9, 3)),
            fire_top: 0,
            rotateable: 0,
            line: 0,
            stretch_growth: false,
            top_face: None,
            picture: None,
        };
        let mut object = make_snapshot().objects.remove(0);
        object.position = Vector2::new(12, 12);
        object.graphics_overlays = vec![ObjectGraphicsOverlay::new(1, GraphicsOverlayMode::Base)
            .with_definition(Some("RotatedOverlay".to_string()))
            .with_transform(Some(DrawTransform::from_matrix([
                0.866, -0.5, 0.0, 0.5, 0.866, 0.0, 0.0, 0.0, 1.0,
            ])))];
        let mut graphics = GraphicsSystem::new(
            28,
            28,
            28,
            "Overlay transform",
            test_font(),
            Arc::new(HashMap::from([(
                sprite_map_key("RotatedOverlay", None),
                sprite,
            )])),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics.draw_object_overlays(
            &object,
            &[],
            &[],
            OWNER_NONE,
            None,
            12.0,
            12.0,
            1.0,
            90.0,
            Some(DrawTransform::from_components(3.0, 3.0, 0.0, 0.0)),
            None,
        );

        let mut min_x = i32::MAX;
        let mut min_y = i32::MAX;
        let mut max_x = i32::MIN;
        let mut max_y = i32::MIN;
        for y in 0..graphics.surface().height() {
            for x in 0..graphics.surface().width() {
                if graphics
                    .surface()
                    .get_pixel(x, y)
                    .is_some_and(|pixel| pixel == Color::opaque(30, 180, 70))
                {
                    min_x = min_x.min(x as i32);
                    min_y = min_y.min(y as i32);
                    max_x = max_x.max(x as i32);
                    max_y = max_y.max(y as i32);
                }
            }
        }
        let width = max_x - min_x + 1;
        let height = max_y - min_y + 1;
        assert!(
            height >= 6,
            "overlay b/d terms were not presented: {:?}",
            (min_x, min_y, max_x, max_y)
        );
        assert!(
            width <= 11,
            "ordinary overlay incorrectly inherited host scale/rotation: {:?}",
            (min_x, min_y, max_x, max_y)
        );
    }

    #[test]
    fn draw_image_strip_copies_subregion_one_to_one() {
        // 4x1 source: columns 0..4 are 10,20,30,40 gray.
        let pixels: Vec<u8> = [10u8, 20, 30, 40]
            .iter()
            .flat_map(|v| [*v, *v, *v, 255])
            .collect();
        let image = ImageData::new(4, 1, pixels);
        let mut surface = Surface::new(2, 1, PixelFormat::Rgba8888);
        draw_image_strip(&mut surface, 0, 0, &image, 2, 0, 2, 1, None);
        assert_eq!(surface.get_pixel(0, 0), Some(gray(30)));
        assert_eq!(surface.get_pixel(1, 0), Some(gray(40)));
    }

    #[test]
    fn draw_image_strip_gamma_uses_independent_rgb_tables() {
        // The blit shader samples three independent R16 gamma textures after
        // texture modulation (StdGL.cpp:1068-1087,1246-1263).
        let image = ImageData::new(1, 1, vec![0, 0, 0, 255]);
        let gamma = clonk_graphics::GammaRamp::from_control_points([0x102030, 0x405060, 0x708090]);
        let mut surface = Surface::new(1, 1, PixelFormat::Rgba8888);

        draw_image_strip(&mut surface, 0, 0, &image, 0, 0, 1, 1, Some(&gamma));

        assert_eq!(surface.get_pixel(0, 0), Some(Color::new(17, 33, 49, 255)));
    }

    #[test]
    fn gpu_capture_lowers_image_strip_to_nearest_gamma_quad() {
        let image = ImageData::new(4, 2, [64, 128, 192, 128].repeat(8));
        let gamma = clonk_graphics::GammaRamp::standard();
        let mut surface = Surface::new(8, 4, PixelFormat::Rgba8888);
        surface.begin_gpu_scene_capture();

        draw_image_strip(&mut surface, 2, 1, &image, 1, 0, 2, 2, Some(&gamma));

        let scene = surface
            .take_gpu_scene_capture()
            .expect("GPU capture remains active")
            .into_scene([8, 4], Color::transparent(), &gamma);
        assert_eq!(scene.textures.len(), 1);
        assert_eq!(scene.commands.len(), 1);
        let GpuCommand::Quad {
            vertices,
            sampler,
            blend,
            gamma,
            ..
        } = &scene.commands[0]
        else {
            panic!("image strip did not lower to a textured quad");
        };
        assert_eq!(*sampler, GpuSampler::Nearest);
        assert_eq!(*blend, GpuBlend::Normal);
        assert!(*gamma);
        assert_eq!(vertices[0].position, [2.0, 1.0, 1.0]);
        assert_eq!(vertices[3].position, [4.0, 3.0, 1.0]);
        assert_eq!(vertices[0].uv, [0.25, 0.0]);
        assert_eq!(vertices[3].uv, [0.75, 1.0]);
    }

    #[test]
    fn draw_image_bilinear_gamma_samples_r16_before_alpha_blending() {
        // Gamma lookup precedes fixed-function source-alpha blending; the
        // normalized R16 sample stays in float until framebuffer storage
        // (StdGL.cpp:908,1081-1087,1246-1255).
        let image = ImageData::new(1, 1, vec![64, 128, 192, 128]);
        let gamma = clonk_graphics::GammaRamp::from_control_points([0x000000, 0x646464, 0xc8c8c8]);
        let mut surface = Surface::new(1, 1, PixelFormat::Rgba8888);
        surface
            .set_pixel(0, 0, Color::opaque(200, 200, 200))
            .unwrap();

        draw_image_bilinear(
            &mut surface,
            &GuiRect::new(0.0, 0.0, 1.0, 1.0),
            &image,
            Some(&gamma),
        );

        assert_eq!(
            surface.get_pixel(0, 0),
            Some(Color::new(125, 150, 175, 255))
        );
    }

    #[test]
    fn draw_image_bilinear_matches_gl_linear_sampling() {
        // 2x1 black|white stretched to 4x1: GL_LINEAR samples at texel centres
        // (i+0.5)*sw/dw - 0.5 with GL_CLAMP_TO_EDGE (C4Surface.cpp:1102):
        // 0, 64, 191, 255.
        let pixels: Vec<u8> = [0u8, 255].iter().flat_map(|v| [*v, *v, *v, 255]).collect();
        let image = ImageData::new(2, 1, pixels);
        let mut surface = Surface::new(4, 1, PixelFormat::Rgba8888);
        draw_image_bilinear(
            &mut surface,
            &GuiRect::new(0.0, 0.0, 4.0, 1.0),
            &image,
            None,
        );
        assert_eq!(surface.get_pixel(0, 0), Some(gray(0)));
        assert_eq!(surface.get_pixel(1, 0), Some(gray(64)));
        assert_eq!(surface.get_pixel(2, 0), Some(gray(191)));
        assert_eq!(surface.get_pixel(3, 0), Some(gray(255)));
    }

    #[test]
    fn draw_x_float_crops_to_inward_integer_bounds() {
        // 8x4 selects 4x4 native texture tiles, so the retained samples cross
        // the same physical tile boundary as CStdDDraw's source-window blit.
        let pixels = (0_u8..32)
            .flat_map(|value| {
                [
                    value.wrapping_mul(13),
                    value.wrapping_mul(7),
                    value.wrapping_mul(3),
                    255,
                ]
            })
            .collect();
        let image = ImageData::new(8, 4, pixels);
        let sentinel = Color::opaque(1, 2, 3);
        let render = |rect: GuiRect, inward: bool| {
            let mut surface = Surface::new(8, 7, PixelFormat::Rgba8888);
            surface.fill(sentinel);
            if inward {
                draw_image_x_float(&mut surface, &rect, &image, None);
            } else {
                draw_image_bilinear(&mut surface, &rect, &image, None);
            }
            surface
        };

        let positive = GuiRect::new(1.25, 1.25, 4.5, 3.5);
        let positive_crop = draw_x_float_crop(&positive, 8, 4).unwrap();
        assert_eq!(
            (
                positive_crop.target_x,
                positive_crop.target_y,
                positive_crop.target_width,
                positive_crop.target_height,
            ),
            (2, 2, 3, 2)
        );
        assert!((positive_crop.source.x - 4.0 / 3.0).abs() < 1e-6);
        assert!((positive_crop.source.y - 6.0 / 7.0).abs() < 1e-6);
        assert!((positive_crop.source.width - 16.0 / 3.0).abs() < 1e-6);
        assert!((positive_crop.source.height - 16.0 / 7.0).abs() < 1e-6);

        let ordinary = render(positive, false);
        let inward = render(positive, true);
        for y in 0..inward.height() {
            for x in 0..inward.width() {
                let pixel = inward.get_pixel(x, y).unwrap();
                if (2..5).contains(&(x as i32)) && (2..4).contains(&(y as i32)) {
                    assert_eq!(pixel, ordinary.get_pixel(x, y).unwrap());
                } else {
                    assert_eq!(pixel, sentinel, "unexpected inward pixel at {x},{y}");
                }
            }
        }
        assert_ne!(ordinary.get_pixel(1, 2), Some(sentinel));
        assert_ne!(ordinary.get_pixel(5, 2), Some(sentinel));
        assert_ne!(ordinary.get_pixel(2, 1), Some(sentinel));
        assert_ne!(ordinary.get_pixel(2, 4), Some(sentinel));

        let negative = GuiRect::new(-1.25, 1.25, 4.0, 3.5);
        let negative_crop = draw_x_float_crop(&negative, 8, 4).unwrap();
        assert_eq!(
            (
                negative_crop.target_x,
                negative_crop.target_y,
                negative_crop.target_width,
                negative_crop.target_height,
            ),
            (-1, 2, 3, 2)
        );
        assert!((negative_crop.source.x - 0.5).abs() < 1e-6);
        assert!((negative_crop.source.width - 6.0).abs() < 1e-6);
        let ordinary = render(negative, false);
        let inward = render(negative, true);
        for y in 2..4 {
            for x in 0..2 {
                assert_eq!(inward.get_pixel(x, y), ordinary.get_pixel(x, y));
            }
        }
        assert_eq!(inward.get_pixel(2, 2), Some(sentinel));
        assert_ne!(ordinary.get_pixel(2, 2), Some(sentinel));

        let negative_y = GuiRect::new(1.25, -1.25, 4.5, 4.0);
        let negative_y_crop = draw_x_float_crop(&negative_y, 8, 4).unwrap();
        assert_eq!(
            (
                negative_y_crop.target_x,
                negative_y_crop.target_y,
                negative_y_crop.target_width,
                negative_y_crop.target_height,
            ),
            (2, -1, 3, 3)
        );
        assert!((negative_y_crop.source.y - 0.25).abs() < 1e-6);
        assert!((negative_y_crop.source.height - 3.0).abs() < 1e-6);
        let ordinary = render(negative_y, false);
        let inward = render(negative_y, true);
        for y in 0..2 {
            for x in 2..5 {
                assert_eq!(inward.get_pixel(x, y), ordinary.get_pixel(x, y));
            }
        }
        assert_eq!(inward.get_pixel(2, 2), Some(sentinel));
        assert_ne!(ordinary.get_pixel(2, 2), Some(sentinel));

        // Native tile selection casts `source_right - 1` before dividing by
        // the 4px tile size. The 4.0..4.8 source tail therefore never emits
        // tile 1, even though pixel x=4 lies within the inward destination.
        let tile_edge_image = ImageData::new(
            5,
            4,
            (0_u8..20)
                .flat_map(|value| [value.wrapping_mul(11), value, value, 255])
                .collect(),
        );
        let tile_edge_rect = GuiRect::new(0.2, 1.0, 5.0, 4.0);
        let tile_edge_crop = draw_x_float_crop(&tile_edge_rect, 5, 4).unwrap();
        assert_eq!(
            (tile_edge_crop.target_x, tile_edge_crop.target_width),
            (1, 4)
        );
        assert!((tile_edge_crop.source.x - 0.8).abs() < 1e-6);
        assert!((tile_edge_crop.source.width - 4.0).abs() < 1e-6);
        let mut ordinary = Surface::new(7, 7, PixelFormat::Rgba8888);
        ordinary.fill(sentinel);
        draw_image_bilinear(&mut ordinary, &tile_edge_rect, &tile_edge_image, None);
        let mut inward = Surface::new(7, 7, PixelFormat::Rgba8888);
        inward.fill(sentinel);
        draw_image_x_float(&mut inward, &tile_edge_rect, &tile_edge_image, None);
        assert_ne!(inward.get_pixel(3, 2), Some(sentinel));
        assert_eq!(inward.get_pixel(4, 2), Some(sentinel));
        assert_ne!(ordinary.get_pixel(4, 2), Some(sentinel));

        for empty in [
            GuiRect::new(2.2, 1.25, 0.6, 3.5),
            GuiRect::new(1.25, 2.2, 3.5, 0.6),
            GuiRect::new(1.25, 1.25, 0.0, 3.5),
            GuiRect::new(1.25, 1.25, -1.0, 3.5),
            GuiRect::new(1.25, 1.25, 3.5, 0.0),
            GuiRect::new(1.25, 1.25, 3.5, -1.0),
        ] {
            assert!(draw_x_float_crop(&empty, 8, 4).is_none());
            assert!(render(empty, true)
                .pixels()
                .chunks_exact(4)
                .all(|pixel| pixel == [sentinel.r, sentinel.g, sentinel.b, sentinel.a]));
        }
        assert_ne!(
            render(GuiRect::new(2.2, 1.25, 0.6, 3.5), false).get_pixel(2, 2),
            Some(sentinel)
        );
    }

    #[test]
    fn runtime_sprite_filtering_matches_stdgl_exact_and_point_modes() {
        let image = ImageData::new(
            2,
            2,
            vec![
                0, 0, 0, 255, // top-left
                255, 0, 0, 255, // top-right
                0, 255, 0, 255, // bottom-left
                0, 0, 255, 255, // bottom-right
            ],
        );
        let render = |application_scale: f32,
                      point_filtering: bool,
                      destination_extent: u32|
         -> (BlitSampling, Vec<Color>) {
            let mut graphics = test_graphics(
                destination_extent,
                destination_extent,
                0,
                "runtime sprite filtering",
            );
            graphics.set_runtime_sprite_filtering(application_scale, point_filtering);
            graphics.surface_mut().fill(Color::opaque(0, 0, 0));
            let (source, sampling) = graphics.runtime_sprite_blit(
                FloatSourceRect::scaled(SourceRect::new(0, 0, 2, 2), 1.0),
                (destination_extent as f32, destination_extent as f32),
                false,
            );
            draw_image_region_float_source(
                &mut graphics.surface,
                &GuiRect::new(
                    0.0,
                    0.0,
                    destination_extent as f32,
                    destination_extent as f32,
                ),
                &image,
                None,
                &source,
                sampling,
                false,
                None,
                SpriteBlitState::normal(),
                None,
                None,
            );
            let pixels = (0..destination_extent)
                .flat_map(|y| {
                    (0..destination_extent)
                        .map(|x| graphics.surface().get_pixel(x, y).unwrap())
                        .collect::<Vec<_>>()
                })
                .collect();
            (sampling, pixels)
        };

        // Exact scale-one blits stay nearest for both PointFiltering values.
        for point_filtering in [false, true] {
            let (sampling, pixels) = render(1.0, point_filtering, 2);
            assert_eq!(sampling, BlitSampling::Nearest);
            assert_eq!(
                pixels,
                vec![
                    Color::opaque(0, 0, 0),
                    Color::opaque(255, 0, 0),
                    Color::opaque(0, 255, 0),
                    Color::opaque(0, 0, 255),
                ]
            );
        }

        // At scale one, PointFiltering alone distinguishes a non-exact 2x
        // stretch. The centre-near sample retains GL's fractional channels.
        let (sampling, linear) = render(1.0, false, 4);
        assert_eq!(sampling, BlitSampling::Linear);
        assert_eq!(linear[5], Color::opaque(48, 48, 16));
        let (sampling, point) = render(1.0, true, 4);
        assert_eq!(sampling, BlitSampling::Nearest);
        assert_eq!(point[5], Color::opaque(0, 0, 0));

        // Non-100% scale forces linear for both config values and first
        // applies CStdDDraw's 0.5/-1 source correction.
        for point_filtering in [false, true] {
            let (sampling, exact_geometry) = render(2.0, point_filtering, 2);
            assert_eq!(sampling, BlitSampling::Linear);
            assert_eq!(exact_geometry[0], Color::opaque(48, 48, 16));
            let (sampling, scaled) = render(2.0, point_filtering, 4);
            assert_eq!(sampling, BlitSampling::Linear);
            assert_eq!(scaled[5], Color::opaque(60, 60, 36));
        }

        // A transform pointer makes identity, rotation, mirror and projective
        // calls alike non-exact; the shared selector then supplies StdGL's
        // default-linear / PointFiltering-nearest choice at scale one.
        let mut graphics = test_graphics(2, 2, 0, "transformed sampler selection");
        let source = FloatSourceRect::scaled(SourceRect::new(0, 0, 2, 2), 1.0);
        for point_filtering in [false, true] {
            graphics.set_runtime_sprite_filtering(1.0, point_filtering);
            let (_, transformed_sampling) = graphics.runtime_sprite_blit(source, (2.0, 2.0), true);
            assert_eq!(
                transformed_sampling,
                if point_filtering {
                    BlitSampling::Nearest
                } else {
                    BlitSampling::Linear
                }
            );
        }
    }

    #[test]
    fn advanced_renderer_config_changes_frame_like_stdgl() {
        let configured_blit =
            |config: AdvancedRendererConfig, mode: u32, modulation: Option<u32>| {
                SpriteBlitState {
                    mode,
                    modulation,
                    fog_modulation: None,
                    renderer_config: AdvancedRendererConfig::DEFAULT,
                }
                .with_renderer_config(config)
            };

        for shader in [false, true] {
            let config = AdvancedRendererConfig {
                shader,
                allowed_blit_modes: C4GFXBLIT_ADDITIVE,
                ..AdvancedRendererConfig::DEFAULT
            };
            let additive = configured_blit(config, C4GFXBLIT_ADDITIVE, None);
            let source = prepare_sprite_fragment(Color::opaque(40, 50, 60), None, None, additive);
            assert_eq!(
                composite_sprite_fragment(source, Color::opaque(10, 20, 30), additive, None),
                Color::opaque(50, 70, 90),
            );

            let masked = configured_blit(
                AdvancedRendererConfig {
                    allowed_blit_modes: 0,
                    ..config
                },
                C4GFXBLIT_ADDITIVE,
                None,
            );
            let source = prepare_sprite_fragment(Color::opaque(40, 50, 60), None, None, masked);
            assert_eq!(
                composite_sprite_fragment(source, Color::opaque(10, 20, 30), masked, None),
                Color::opaque(40, 50, 60),
            );
        }

        let alpha_result = |shader, no_alpha_add| {
            let blit = configured_blit(
                AdvancedRendererConfig {
                    shader,
                    no_alpha_add,
                    ..AdvancedRendererConfig::DEFAULT
                },
                0,
                Some(0x40ff_ffff),
            );
            let source = prepare_sprite_fragment(Color::new(200, 100, 50, 192), None, None, blit);
            composite_sprite_fragment(source, Color::opaque(0, 0, 0), blit, None)
        };
        assert_eq!(alpha_result(false, false), Color::opaque(100, 50, 25));
        assert_eq!(alpha_result(false, true), Color::opaque(151, 75, 38));
        assert_eq!(alpha_result(true, true), Color::opaque(100, 50, 25));

        let mod2_alpha = |shader| {
            let blit = configured_blit(
                AdvancedRendererConfig {
                    shader,
                    ..AdvancedRendererConfig::DEFAULT
                },
                C4GFXBLIT_MOD2,
                Some(0x40ff_ffff),
            );
            prepare_sprite_fragment(Color::new(64, 96, 128, 192), None, None, blit).alpha()
        };
        assert_eq!(mod2_alpha(false), 128.0);
        assert_eq!(mod2_alpha(true), 192.0);

        let fog_fragment = |shader: bool, no_box_fades: bool, weights: [f32; 4]| {
            let blit = configured_blit(
                AdvancedRendererConfig {
                    shader,
                    no_box_fades,
                    ..AdvancedRendererConfig::DEFAULT
                },
                0,
                None,
            )
            .with_fog_modulation(FogModulationSample {
                modulation: [0x0040_4040, 0x0080_8080, 0x00c0_c0c0, 0x00ff_ffff],
                weights,
            });
            let source = prepare_sprite_fragment(Color::opaque(255, 255, 255), None, None, blit);
            composite_sprite_fragment(source, Color::opaque(0, 0, 0), blit, None).r
        };
        for shader in [false, true] {
            assert_eq!(fog_fragment(shader, false, [0.5, 0.25, 0.25, 0.0]), 111);
            assert_eq!(fog_fragment(shader, true, [0.5, 0.25, 0.25, 0.0]), 191);
            assert_eq!(fog_fragment(shader, false, [0.0, 0.25, 0.25, 0.5]), 207);
            assert_eq!(fog_fragment(shader, true, [0.0, 0.25, 0.25, 0.5]), 254);

            let gradient = |no_box_fades| {
                let mut graphics = test_graphics(1, 3, 0, "advanced renderer gradient");
                graphics.set_advanced_renderer_config(AdvancedRendererConfig {
                    shader,
                    no_box_fades,
                    ..AdvancedRendererConfig::DEFAULT
                });
                graphics.surface_mut().fill(Color::opaque(0, 0, 0));
                graphics.fill_vertical_gradient_modulated(
                    Color::opaque(64, 0, 0),
                    Color::opaque(255, 0, 0),
                    1.0,
                    None,
                    None,
                );
                (0..3)
                    .map(|y| graphics.surface().get_pixel(0, y).unwrap().r)
                    .collect::<Vec<_>>()
            };
            assert_eq!(gradient(false), vec![64, 160, 255]);
            assert_eq!(gradient(true), vec![124, 124, 124]);

            let solid = |no_box_fades| {
                let mut graphics = test_graphics(1, 1, 0, "advanced renderer solid box");
                graphics.set_advanced_renderer_config(AdvancedRendererConfig {
                    shader,
                    no_box_fades,
                    ..AdvancedRendererConfig::DEFAULT
                });
                graphics.fill_world_color(Color::opaque(64, 0, 0), false, None);
                graphics.surface().get_pixel(0, 0).unwrap().r
            };
            assert_eq!(solid(false), 64);
            assert_eq!(solid(true), 8);
        }

        let row = [
            0, 0, 0, 255, 64, 64, 64, 255, 128, 128, 128, 255, 255, 255, 255, 255,
        ];
        let image = ImageData::new(4, 4, row.repeat(4));
        let sentinel = Color::opaque(7, 11, 13);
        let render_quad = |config: AdvancedRendererConfig| {
            let mut surface = Surface::new(17, 8, PixelFormat::Rgba8888);
            surface.fill(sentinel);
            let blit = configured_blit(config, 0, None);
            draw_image_region_float_source(
                &mut surface,
                &GuiRect::new(10.0, 1.0, 4.0, 4.0),
                &image,
                None,
                &FloatSourceRect {
                    x: 0.0,
                    y: 0.0,
                    width: 4.0,
                    height: 4.0,
                },
                BlitSampling::Linear,
                false,
                None,
                blit,
                None,
                None,
            );
            surface
        };
        for shader in [false, true] {
            let baseline = render_quad(AdvancedRendererConfig {
                shader,
                ..AdvancedRendererConfig::DEFAULT
            });
            assert_eq!(
                (10..14)
                    .map(|x| baseline.get_pixel(x, 1).unwrap().r)
                    .collect::<Vec<_>>(),
                vec![0, 64, 128, 255]
            );

            let indented = render_quad(AdvancedRendererConfig {
                shader,
                tex_indent: 500,
                ..AdvancedRendererConfig::DEFAULT
            });
            assert_eq!(
                (10..14)
                    .map(|x| indented.get_pixel(x, 1).unwrap().r)
                    .collect::<Vec<_>>(),
                vec![26, 77, 128, 230]
            );

            let shifted = render_quad(AdvancedRendererConfig {
                shader,
                blit_offset: 100,
                ..AdvancedRendererConfig::DEFAULT
            });
            assert_eq!(shifted.get_pixel(10, 2), Some(sentinel));
            assert_eq!(shifted.get_pixel(11, 1), Some(sentinel));
            assert_eq!(
                (11..15)
                    .map(|x| shifted.get_pixel(x, 2).unwrap().r)
                    .collect::<Vec<_>>(),
                vec![0, 64, 128, 255]
            );
        }

        let cropped_row = (0_u8..8)
            .flat_map(|value| [value.saturating_mul(32), 0, 0, 255])
            .collect::<Vec<_>>();
        let cropped_image = ImageData::new(8, 8, cropped_row.repeat(8));
        let mut cropped = Surface::new(4, 4, PixelFormat::Rgba8888);
        draw_image_region_float_source(
            &mut cropped,
            &GuiRect::new(0.0, 0.0, 4.0, 4.0),
            &cropped_image,
            None,
            &FloatSourceRect {
                x: 2.0,
                y: 0.0,
                width: 4.0,
                height: 4.0,
            },
            BlitSampling::Linear,
            false,
            None,
            configured_blit(
                AdvancedRendererConfig {
                    tex_indent: 500,
                    ..AdvancedRendererConfig::DEFAULT
                },
                0,
                None,
            ),
            None,
            None,
        );
        assert_eq!(
            (0..4)
                .map(|x| cropped.get_pixel(x, 0).unwrap().r)
                .collect::<Vec<_>>(),
            vec![78, 107, 135, 164],
            "ordinary TexIndent re-anchors its rescale at the cropped quad edge",
        );

        let sky_row = [0, 0, 0, 255, 50, 0, 0, 255, 100, 0, 0, 255, 150, 0, 0, 255];
        let sky_image = ImageData::new(4, 4, sky_row.repeat(4));
        let mut clipped_sky = test_graphics(2, 4, 0, "advanced renderer cropped sky");
        clipped_sky.set_advanced_renderer_config(AdvancedRendererConfig {
            tex_indent: 1000,
            ..AdvancedRendererConfig::DEFAULT
        });
        clipped_sky.draw_sky_tile_positions_with_parallel_rows(
            &sky_image,
            &[(-2, 0)],
            None,
            1.0,
            None,
            false,
        );
        assert_eq!(
            clipped_sky.surface().get_pixel(0, 0),
            Some(Color::opaque(150, 0, 0)),
            "BlitSurfaceTile2 crops the source before ordinary TexIndent rescaling",
        );

        let padded_image = ImageData::new(3, 3, [255, 0, 0, 255].repeat(9));
        let padded_source = FloatSourceRect {
            x: 0.0,
            y: 0.0,
            width: 3.0,
            height: 3.0,
        };
        let padded = prepare_runtime_sprite_sample(
            &padded_image,
            None,
            &padded_source,
            false,
            2.5,
            1.5,
            BlitSampling::Linear,
            None,
            configured_blit(
                AdvancedRendererConfig {
                    tex_indent: -500,
                    ..AdvancedRendererConfig::DEFAULT
                },
                0,
                None,
            ),
        )
        .expect("physical padding remains part of the selected texture");
        assert!(
            (padded.alpha() - 170.0).abs() < 0.01,
            "negative indent mixes the last logical texel with transparent-white padding",
        );

        assert_eq!(
            cpp_landscape_source_texel(6, 4, 3.5, 0.5, 1.0),
            Some((3, 1, 4, 1)),
            "landscape indent clamps at the raw coordinate's left texture seam",
        );
        assert_eq!(
            cpp_landscape_source_texel(6, 4, 4.1, 0.5, 1.0),
            Some((5, 1, 1, 1)),
            "the next raw coordinate selects the next texture before indent",
        );
        assert_eq!(
            cpp_landscape_source_texel(6, 4, 5.5, 0.5, 1.0),
            None,
            "landscape physical padding is transparent rather than the last logical texel",
        );
        let texture_size = cpp_tex_size(6, 4) as i32;
        let raw_y = 2.5_f32;
        let world_y = raw_y.floor() as i32;
        let liquid_y = world_y.rem_euclid(texture_size);
        for raw_x in [f32::NAN, -0.5, 0.5, 3.5, 4.1, 5.5, 6.0] {
            assert_eq!(
                LandscapeXSample::new(raw_x, texture_size).zero_indent_texel(6, world_y, liquid_y,),
                cpp_landscape_source_texel(6, 4, raw_x, raw_y, 0.0),
                "zero-indent fast path at x={raw_x:?}",
            );
        }

        let fog = FogDrawContext {
            map: Arc::new(ClrModMap {
                resolution_x: 64,
                resolution_y: 64,
                width: 2,
                height: 2,
                origin_x: 0,
                origin_y: 0,
                fade_transparent: false,
                cells: vec![0x0000_0000, 0x0040_4040, 0x0080_8080, 0x00ff_ffff],
            }),
            zoom: 1.0,
        };
        let sampler = FogSpriteSampler::new_with_chunks(
            &fog,
            (0.0, 0.0, 64.0, 64.0),
            (0.0, 0.0, 64.0, 64.0),
            (64.0, 64.0),
            false,
            |x, y| (x, y),
        )
        .unwrap();
        let (x_samples, y_samples) = sampler.raster_axes_with_destination_offset(64, 64, 1.0, 1.0);
        assert_eq!(
            sampler
                .modulation_sample_for_axes(x_samples[1], y_samples[1])
                .interpolate(),
            sampler.modulation_at(0.5 / 64.0, 0.5 / 64.0),
            "DrawQuad samples ClrModMap at raw vertices before adding BlitOffset",
        );

        let mut gamma_state = GammaControlState::default();
        gamma_state.set_ramp(0, [0x000000, 0x646464, 0xc8c8c8]);
        let configured_gamma_output = |config: AdvancedRendererConfig| {
            let blit = configured_blit(config, 0, Some(0x00ff_ffff));
            let source =
                prepare_filtered_sprite_fragment([127.25, 96.5, 64.75, 128.0], None, None, blit);
            let ramp =
                clonk_graphics::GammaRamp::from_control_points([0x000000, 0x646464, 0xc8c8c8]);
            let mut output = composite_sprite_fragment(
                source,
                Color::opaque(48, 48, 48),
                blit,
                config.uses_fragment_gamma().then_some(&ramp),
            );
            if config.uses_monitor_gamma() {
                output = gamma_encode_fragment(output, &ramp);
            }
            output
        };
        for shader in [false, true] {
            for use_shader_gamma in [false, true] {
                for disable_gamma in [false, true] {
                    let mut graphics = test_graphics(1, 1, 0, "advanced renderer gamma");
                    graphics.set_advanced_renderer_config(AdvancedRendererConfig {
                        shader,
                        use_shader_gamma,
                        disable_gamma,
                        ..AdvancedRendererConfig::DEFAULT
                    });
                    graphics.apply_gamma_now(&gamma_state);
                    let fragment_gamma = !disable_gamma && shader && use_shader_gamma;
                    let monitor_gamma = !disable_gamma && !fragment_gamma;
                    assert_eq!(graphics.fragment_gamma_enabled(), fragment_gamma);
                    assert_eq!(graphics.monitor_gamma_enabled(), monitor_gamma);
                    let ramp = graphics.active_gamma_ramp(&gamma_state);
                    assert_eq!(
                        gamma_encode_fragment(Color::opaque(64, 128, 192), &ramp),
                        if disable_gamma {
                            Color::opaque(64, 128, 192)
                        } else {
                            Color::opaque(50, 100, 150)
                        },
                    );
                    assert_eq!(
                        configured_gamma_output(graphics.advanced_renderer_config()),
                        if fragment_gamma {
                            Color::opaque(74, 61, 49)
                        } else if monitor_gamma {
                            Color::opaque(69, 56, 44)
                        } else {
                            Color::opaque(88, 72, 56)
                        },
                        "shader={shader}, use_shader_gamma={use_shader_gamma}, disable_gamma={disable_gamma}",
                    );
                }
            }
        }
    }

    #[test]
    fn runtime_sprite_linear_filtering_uses_native_tile_edges_and_float_pipeline() {
        let row = [0, 0, 0, 255, 100, 0, 0, 255, 200, 0, 0, 255, 255, 0, 0, 255];
        let image = ImageData::new(4, 2, row.repeat(2));
        let source = FloatSourceRect {
            x: 0.0,
            y: 0.0,
            width: 4.0,
            height: 2.0,
        };
        let sample = |source_edge_x: f32| {
            let fragment = prepare_runtime_sprite_sample(
                &image,
                None,
                &source,
                false,
                source_edge_x,
                0.5,
                BlitSampling::Linear,
                None,
                SpriteBlitState::normal(),
            )
            .expect("source coordinate belongs to a native texture tile");
            composite_sprite_fragment(
                fragment,
                Color::opaque(0, 0, 0),
                SpriteBlitState::normal(),
                None,
            )
        };

        assert_eq!(
            sample(1.25),
            Color::opaque(75, 0, 0),
            "a source-facet boundary may filter an adjacent atlas texel",
        );
        assert_eq!(
            sample(1.75),
            Color::opaque(100, 0, 0),
            "the left C4TexRef clamps instead of sampling the right tile",
        );
        assert_eq!(
            sample(2.25),
            Color::opaque(200, 0, 0),
            "the right C4TexRef independently clamps its left edge",
        );

        // Keep sub-byte filtered channels in float until after shader
        // modulation: 0..3 sampled at 25% is 0.75, then *128/255 rounds to
        // zero. Quantizing the texture sample first would incorrectly yield 1.
        let tiny = ImageData::new(2, 1, vec![0, 0, 0, 255, 3, 0, 0, 255]);
        let blit = SpriteBlitState {
            mode: 0,
            modulation: Some(0x0080_0000),
            fog_modulation: None,
            renderer_config: AdvancedRendererConfig::DEFAULT,
        };
        let tiny_source = FloatSourceRect {
            x: 0.0,
            y: 0.0,
            width: 2.0,
            height: 1.0,
        };
        let fragment = prepare_runtime_sprite_sample(
            &tiny,
            None,
            &tiny_source,
            false,
            0.75,
            0.5,
            BlitSampling::Linear,
            None,
            blit,
        )
        .unwrap();
        assert_eq!(
            composite_sprite_fragment(fragment, Color::opaque(0, 0, 0), blit, None),
            Color::opaque(0, 0, 0),
        );

        // The owner bitmap is a second filtered texture pass. Its filtered
        // purple midpoint is tinted and composed over the independently
        // filtered black base, rather than choosing one owner texel.
        let base = ImageData::new(2, 1, vec![0, 0, 0, 255, 0, 0, 0, 255]);
        let owner = ColorByOwnerMask::new(2, 1, Arc::from([255, 0, 0, 255, 0, 0, 255, 255]));
        let fragment = prepare_runtime_sprite_sample(
            &base,
            Some(&owner),
            &tiny_source,
            false,
            1.0,
            0.5,
            BlitSampling::Linear,
            Some(0x00ff_ffff),
            SpriteBlitState::normal(),
        )
        .unwrap();
        assert_eq!(
            composite_sprite_fragment(
                fragment,
                Color::opaque(0, 0, 0),
                SpriteBlitState::normal(),
                None,
            ),
            Color::opaque(128, 0, 128),
        );
    }

    #[test]
    fn draw_image_bilinear_additive_adds_weighted_source() {
        // Additive blit per StdGL.cpp:908 glBlendFunc(GL_SRC_ALPHA, GL_ONE):
        // dst = min(dst + src*a/255, 255).
        let pixels: Vec<u8> = vec![100, 100, 100, 128];
        let image = ImageData::new(1, 1, pixels);
        let mut surface = Surface::new(1, 1, PixelFormat::Rgba8888);
        surface.set_pixel(0, 0, gray(200)).unwrap();
        draw_image_bilinear_additive(
            &mut surface,
            &GuiRect::new(0.0, 0.0, 1.0, 1.0),
            &image,
            None,
        );
        // 200 + round(100*128/255) = 200 + 50 = 250
        assert_eq!(
            surface.get_pixel(0, 0),
            Some(Color::new(250, 250, 250, 255))
        );
    }

    #[test]
    fn gpu_capture_lowers_bilinear_hud_images_with_native_blend_and_modulation() {
        let image = ImageData::new(2, 2, [64, 128, 192, 128].repeat(4));
        let gamma = clonk_graphics::GammaRamp::standard();
        let mut surface = Surface::new(12, 4, PixelFormat::Rgba8888);
        surface.begin_gpu_scene_capture();

        draw_image_bilinear(
            &mut surface,
            &GuiRect::new(0.0, 0.0, 4.0, 4.0),
            &image,
            Some(&gamma),
        );
        draw_image_bilinear_additive(
            &mut surface,
            &GuiRect::new(4.0, 0.0, 4.0, 4.0),
            &image,
            Some(&gamma),
        );
        draw_image_bilinear_owner(
            &mut surface,
            &GuiRect::new(8.0, 0.0, 4.0, 4.0),
            &image,
            0x2080_c0f0,
            Some(&gamma),
        );

        let scene = surface
            .take_gpu_scene_capture()
            .expect("GPU capture remains active")
            .into_scene([12, 4], Color::transparent(), &gamma);
        assert_eq!(scene.textures.len(), 1);
        assert_eq!(scene.commands.len(), 3);
        for (index, command) in scene.commands.iter().enumerate() {
            let GpuCommand::Quad {
                vertices,
                sampler,
                blend,
                base_mod2,
                gamma,
                ..
            } = command
            else {
                panic!("bilinear HUD draw {index} did not lower to a textured quad");
            };
            assert_eq!(*sampler, GpuSampler::Linear);
            assert_eq!(
                *blend,
                if index == 1 {
                    GpuBlend::Additive
                } else {
                    GpuBlend::Normal
                }
            );
            assert!(!*base_mod2);
            assert!(*gamma);
            if index == 2 {
                assert_eq!(
                    vertices[0].modulation,
                    [128.0 / 255.0, 192.0 / 255.0, 240.0 / 255.0, 32.0 / 255.0]
                );
            }
        }
    }

    #[test]
    fn gpu_capture_raster_fallback_keeps_partial_source_as_ordered_fragment() {
        let image = ImageData::new(1, 1, vec![40, 80, 120, 128]);
        let mut surface = Surface::new(2, 1, PixelFormat::Rgba8888);
        surface.begin_gpu_scene_capture();
        surface.set_pixel(0, 0, Color::opaque(1, 2, 3)).unwrap();
        draw_image_region(
            &mut surface,
            &GuiRect::new(0.0, 0.0, 2.0, 1.0),
            &image,
            None,
            &SourceRect::new(-1, 0, 2, 1),
            false,
            None,
            SpriteBlitState::normal(),
            None,
            None,
        );

        assert!(surface.pixels().iter().all(|component| *component == 0));
        let scene = surface
            .take_gpu_scene_capture()
            .expect("GPU capture remains active")
            .into_scene(
                [2, 1],
                Color::transparent(),
                &clonk_graphics::GammaRamp::standard(),
            );
        assert_eq!(scene.commands.len(), 2);
        let GpuCommand::Solid {
            vertices,
            topology,
            blend,
            ..
        } = &scene.commands[1]
        else {
            panic!("partial source fallback did not remain a solid fragment");
        };
        assert_eq!(*topology, GpuPrimitiveTopology::PointList);
        assert_eq!(*blend, GpuBlend::Normal);
        assert_eq!(vertices.len(), 1);
        assert_eq!(vertices[0].position, [1.5, 0.5, 1.0]);
    }

    #[test]
    fn gpu_capture_records_native_texture_tile_size_for_linear_blits() {
        // min(6, 2) rounds to a two-pixel C4TexRef. The backend derives each
        // fragment's tile origin, retaining native seams in one draw call.
        let image = ImageData::new(6, 2, [64, 128, 192, 255].repeat(12));
        let mut surface = Surface::new(12, 2, PixelFormat::Rgba8888);
        surface.begin_gpu_scene_capture();

        draw_image_bilinear(
            &mut surface,
            &GuiRect::new(0.0, 0.0, 12.0, 2.0),
            &image,
            None,
        );

        let scene = surface
            .take_gpu_scene_capture()
            .expect("GPU capture remains active")
            .into_scene(
                [12, 2],
                Color::transparent(),
                &clonk_graphics::GammaRamp::standard(),
            );
        assert_eq!(scene.commands.len(), 1);
        let GpuCommand::Quad {
            vertices, sampler, ..
        } = &scene.commands[0]
        else {
            panic!("native texture tile did not lower to a quad");
        };
        assert_eq!(*sampler, GpuSampler::Linear);
        assert_eq!(vertices[0].position[0], 0.0);
        assert_eq!(vertices[3].position[0], 12.0);
        assert!(vertices
            .iter()
            .all(|vertex| vertex.sample_tile == [0.0, 0.0, 2.0, 1.0]));
    }

    #[test]
    fn simple_gpu_sprite_capture_avoids_temporary_batching() {
        let image = ImageData::new(2, 2, [64, 128, 192, 255].repeat(4));
        let mut surface = Surface::new(4, 4, PixelFormat::Rgba8888);
        surface.begin_gpu_scene_capture();
        reset_gpu_sprite_batch_fallbacks();

        assert!(capture_gpu_sprite(
            &mut surface,
            (0.0, 0.0, 4.0, 4.0),
            (0.0, 0.0, 4.0, 4.0),
            &GraphicsTransform::identity(),
            &image,
            None,
            FloatSourceRect {
                x: 0.0,
                y: 0.0,
                width: 2.0,
                height: 2.0,
            },
            false,
            None,
            SpriteBlitState::normal(),
            None,
            None,
            GpuSampler::Nearest,
            false,
        ));

        assert_eq!(gpu_sprite_batch_fallbacks(), 0);
    }

    #[test]
    fn st5b_shaped_object_faces_form_one_compact_resource_run() {
        let image = ImageData::new(300, 110, vec![255; 300 * 110 * 4]);
        let mut surface = Surface::new(300, 15, PixelFormat::Rgba8888);
        surface.begin_gpu_scene_capture();

        for phase in 0..20_u32 {
            assert!(capture_gpu_object_sprite(
                &mut surface,
                (phase as f32 * 15.0, 0.0, 15.0, 15.0),
                (phase as f32 * 15.0, 0.0, 15.0, 15.0),
                &GraphicsTransform::identity(),
                &image,
                None,
                FloatSourceRect {
                    x: phase as f32 * 15.0,
                    y: 0.0,
                    width: 15.0,
                    height: 15.0,
                },
                phase % 2 != 0,
                None,
                SpriteBlitState {
                    modulation: Some(0x0001_0101_u32.saturating_mul(phase + 1)),
                    ..SpriteBlitState::normal()
                },
                None,
                None,
                if phase % 2 == 0 {
                    GpuSampler::Nearest
                } else {
                    GpuSampler::Linear
                },
                false,
            ));
        }

        let scene = surface
            .take_gpu_scene_capture()
            .expect("GPU capture remains active")
            .into_scene(
                [300, 15],
                Color::transparent(),
                &clonk_graphics::GammaRamp::standard(),
            );
        let [GpuCommand::ObjectBatch { sprites, .. }] = scene.commands.as_slice() else {
            panic!("representable object faces did not form one compact resource run");
        };
        assert_eq!(sprites.len(), 20);
        assert_eq!(sprites[0].sampler(), GpuSampler::Nearest);
        assert_eq!(sprites[1].sampler(), GpuSampler::Linear);
        assert!(sprites
            .windows(2)
            .all(|pair| pair[0].modulation != pair[1].modulation));
    }

    #[test]
    fn fogged_st5b_phase_crossing_a_64px_boundary_stays_compact() {
        let image = ImageData::new(300, 110, vec![255; 300 * 110 * 4]);
        let fog = FogDrawContext {
            map: Arc::new(ClrModMap {
                resolution_x: 8,
                resolution_y: 8,
                width: 4,
                height: 4,
                origin_x: 0,
                origin_y: 0,
                fade_transparent: false,
                cells: (0..16)
                    .map(|index| (index as u32 + 1) * 0x0008_0402)
                    .collect(),
            }),
            zoom: 1.0,
        };
        let mut surface = Surface::new(15, 15, PixelFormat::Rgba8888);
        surface.begin_gpu_scene_capture();

        assert!(capture_gpu_object_sprite(
            &mut surface,
            (0.0, 0.0, 15.0, 15.0),
            (0.0, 0.0, 15.0, 15.0),
            &GraphicsTransform::identity(),
            &image,
            None,
            FloatSourceRect {
                x: 60.0,
                y: 0.0,
                width: 15.0,
                height: 15.0,
            },
            false,
            None,
            SpriteBlitState {
                modulation: Some(0x00c0_8040),
                ..SpriteBlitState::normal()
            },
            None,
            Some(&fog),
            GpuSampler::Linear,
            false,
        ));

        let scene = surface
            .take_gpu_scene_capture()
            .expect("GPU capture remains active")
            .into_scene(
                [15, 15],
                Color::transparent(),
                &clonk_graphics::GammaRamp::standard(),
            );
        let [GpuCommand::ObjectBatch { sprites, .. }] = scene.commands.as_slice() else {
            panic!("fogged ST5B phase entered the generic quad fallback");
        };
        assert_eq!(sprites.len(), 2, "source phase crosses the 64px chunk edge");
        assert_eq!(sprites[0].uv[2], 64.0 / 300.0);
        assert_eq!(sprites[1].uv[0], 64.0 / 300.0);
        assert!(sprites
            .iter()
            .all(|sprite| sprite.sampler() == GpuSampler::Linear));

        let mut generic_surface = Surface::new(15, 15, PixelFormat::Rgba8888);
        generic_surface.begin_gpu_scene_capture();
        assert!(capture_gpu_sprite(
            &mut generic_surface,
            (0.0, 0.0, 15.0, 15.0),
            (0.0, 0.0, 15.0, 15.0),
            &GraphicsTransform::identity(),
            &image,
            None,
            FloatSourceRect {
                x: 60.0,
                y: 0.0,
                width: 15.0,
                height: 15.0,
            },
            false,
            None,
            SpriteBlitState {
                modulation: Some(0x00c0_8040),
                ..SpriteBlitState::normal()
            },
            None,
            Some(&fog),
            GpuSampler::Linear,
            false,
        ));
        let generic = generic_surface
            .take_gpu_scene_capture()
            .expect("generic GPU capture remains active")
            .into_scene(
                [15, 15],
                Color::transparent(),
                &clonk_graphics::GammaRamp::standard(),
            );
        assert_eq!(generic.commands.len(), sprites.len());
        for (sprite, command) in sprites.iter().zip(&generic.commands) {
            let GpuCommand::Quad {
                vertices,
                base_mod2,
                sampler,
                ..
            } = command
            else {
                panic!("generic fog reference did not retain one quad per chunk");
            };
            let expected_uv = [
                [sprite.uv[0], sprite.uv[1]],
                [sprite.uv[2], sprite.uv[1]],
                [sprite.uv[0], sprite.uv[3]],
                [sprite.uv[2], sprite.uv[3]],
            ];
            assert_eq!(vertices.map(|vertex| vertex.position), sprite.positions);
            assert_eq!(vertices.map(|vertex| vertex.uv), expected_uv);
            assert_eq!(
                vertices.map(|vertex| {
                    let [red, green, blue, transparency] = vertex
                        .modulation
                        .map(|channel| (channel * 255.0).round() as u32);
                    (transparency << 24) | (red << 16) | (green << 8) | blue
                }),
                sprite.modulation,
            );
            assert_eq!(*base_mod2, sprite.mod2());
            assert_eq!(*sampler, sprite.sampler());
        }
    }

    #[test]
    fn gpu_capture_reanchors_tex_indent_at_fog_chunks_and_flattens_each_triangle() {
        let image = ImageData::new(128, 128, vec![255; 128 * 128 * 4]);
        let fog = FogDrawContext {
            map: Arc::new(ClrModMap {
                resolution_x: 64,
                resolution_y: 64,
                width: 3,
                height: 2,
                origin_x: 0,
                origin_y: 0,
                fade_transparent: false,
                cells: vec![
                    0x0020_4060,
                    0x0040_6080,
                    0x0060_80a0,
                    0x0080_a0c0,
                    0x00a0_c0e0,
                    0x00ff_ffff,
                ],
            }),
            zoom: 1.0,
        };
        let mut surface = Surface::new(100, 1, PixelFormat::Rgba8888);
        surface.begin_gpu_scene_capture();
        assert!(capture_gpu_sprite(
            &mut surface,
            (0.0, 0.0, 100.0, 1.0),
            (0.0, 0.0, 100.0, 1.0),
            &GraphicsTransform::identity(),
            &image,
            None,
            FloatSourceRect {
                x: 10.0,
                y: 0.0,
                width: 100.0,
                height: 1.0,
            },
            false,
            None,
            SpriteBlitState {
                mode: 0,
                modulation: None,
                fog_modulation: None,
                renderer_config: AdvancedRendererConfig {
                    tex_indent: 1000,
                    no_box_fades: true,
                    ..AdvancedRendererConfig::DEFAULT
                },
            },
            None,
            Some(&fog),
            GpuSampler::Linear,
            false,
        ));
        let scene = surface
            .take_gpu_scene_capture()
            .expect("GPU capture remains active")
            .into_scene(
                [100, 1],
                Color::transparent(),
                &clonk_graphics::GammaRamp::standard(),
            );
        assert_eq!(
            scene.commands.len(),
            4,
            "two fog chunks split into two flat triangles"
        );

        let mut chunks = Vec::new();
        for command in &scene.commands {
            let GpuCommand::Quad { vertices, .. } = command else {
                panic!("fogged sprite did not lower to a textured quad");
            };
            assert_eq!(
                vertices[2], vertices[3],
                "each command is one degenerate triangle"
            );
            assert!(vertices
                .iter()
                .all(|vertex| vertex.modulation == vertices[0].modulation));
            chunks.push(*vertices);
        }

        let close = |actual: f32, expected: f32| {
            assert!((actual - expected).abs() < 1.0e-6, "{actual} != {expected}");
        };
        close(chunks[0][0].uv[0], 11.0 / 128.0);
        close(chunks[2][0].uv[0], 65.0 / 128.0);
        close(chunks[3][2].uv[0], (65.0 + 46.0 * 128.0 / 130.0) / 128.0);
    }

    #[test]
    fn gpu_capture_retains_configured_sheared_font_fragments() {
        let image = ImageData::new(2, 2, [160, 96, 32, 255].repeat(4));
        let mut surface = Surface::new(8, 8, PixelFormat::Rgba8888);
        surface.begin_gpu_scene_capture();
        let _config = activate_advanced_renderer_config(AdvancedRendererConfig {
            tex_indent: 500,
            blit_offset: 100,
            shader: false,
            no_alpha_add: true,
            ..AdvancedRendererConfig::DEFAULT
        });
        draw_image_bilinear_sheared_target(
            &mut surface,
            &GuiRect::new(1.0, 1.0, 2.0, 2.0),
            &image,
            None,
            0.25,
            [128, 192, 255],
            192,
        );

        assert!(surface.pixels().iter().all(|component| *component == 0));
        let scene = surface
            .take_gpu_scene_capture()
            .expect("GPU capture remains active")
            .into_scene(
                [8, 8],
                Color::transparent(),
                &clonk_graphics::GammaRamp::standard(),
            );
        assert!(!scene.commands.is_empty());
        assert!(scene.commands.iter().all(|command| matches!(
            command,
            GpuCommand::Solid {
                topology: GpuPrimitiveTopology::PointList,
                style: GpuSolidStyle::NONE,
                ..
            }
        )));
    }

    #[test]
    fn gpu_capture_retains_owner_mod2_additive_spatial_fog_and_projective_transform() {
        let image = ImageData::new(
            2,
            2,
            vec![
                20, 40, 60, 255, 80, 100, 120, 255, 140, 160, 180, 255, 200, 220, 240, 255,
            ],
        );
        let mask = ColorByOwnerMask::new(2, 2, Arc::from([0_u8, 255, 128, 0]));
        let fog = FogDrawContext {
            map: Arc::new(ClrModMap {
                resolution_x: 2,
                resolution_y: 2,
                width: 3,
                height: 3,
                origin_x: 0,
                origin_y: 0,
                fade_transparent: false,
                cells: vec![
                    0x0020_4060,
                    0x0040_6080,
                    0x0060_80a0,
                    0x0040_8060,
                    0x0080_a0c0,
                    0x00a0_c0e0,
                    0x0060_a080,
                    0x00a0_c0e0,
                    0x00ff_ffff,
                ],
            }),
            zoom: 1.0,
        };
        let transform = GraphicsTransform::set(1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.03, 0.02, 1.0);
        let mut surface = Surface::new(8, 8, PixelFormat::Rgba8888);
        surface.set_clip(SurfaceRect::new(1, 1, 6, 6));
        surface.begin_gpu_scene_capture();
        assert!(capture_gpu_sprite(
            &mut surface,
            (1.0, 1.0, 4.0, 4.0),
            (1.0, 1.0, 4.0, 4.0),
            &transform,
            &image,
            Some(&mask),
            FloatSourceRect {
                x: 0.0,
                y: 0.0,
                width: 2.0,
                height: 2.0,
            },
            false,
            Some(0x00e0_4020),
            SpriteBlitState {
                mode: C4GFXBLIT_ADDITIVE | C4GFXBLIT_MOD2 | C4GFXBLIT_CLRSFC_MOD2,
                modulation: Some(0x00c0_a080),
                fog_modulation: None,
                renderer_config: AdvancedRendererConfig::DEFAULT,
            },
            Some(&clonk_graphics::GammaRamp::standard()),
            Some(&fog),
            GpuSampler::Nearest,
            false,
        ));
        let scene = surface
            .take_gpu_scene_capture()
            .expect("owner/FoW capture remains active")
            .into_scene(
                [8, 8],
                Color::transparent(),
                &clonk_graphics::GammaRamp::standard(),
            );
        assert_eq!(
            scene.textures.len(),
            2,
            "scalar owner masks split two layers"
        );
        assert!(!scene.commands.is_empty() && scene.commands.len().is_multiple_of(2));
        let layer_commands = scene.commands.len() / 2;
        let base_texture = scene.textures[0].id;
        let overlay_texture = scene.textures[1].id;
        let mut modulations = Vec::new();
        let mut homogeneous_w = Vec::new();
        for (index, command) in scene.commands.iter().enumerate() {
            let GpuCommand::Quad {
                texture,
                vertices,
                clip,
                blend,
                base_mod2,
                gamma,
                ..
            } = command
            else {
                panic!("owner capture must lower to ordinary quads");
            };
            assert_eq!(
                *texture,
                if index < layer_commands {
                    base_texture
                } else {
                    overlay_texture
                },
                "every base chunk must precede every owner chunk"
            );
            assert_eq!(*clip, Some(SurfaceRect::new(1, 1, 6, 6)));
            assert_eq!(*blend, GpuBlend::Additive);
            assert!(*base_mod2);
            assert!(*gamma);
            modulations.extend(vertices.iter().map(|vertex| vertex.modulation));
            homogeneous_w.extend(vertices.iter().map(|vertex| vertex.position[2]));
        }
        assert!(modulations.windows(2).any(|values| values[0] != values[1]));
        assert!(homogeneous_w.iter().all(|value| *value > 0.0));
        assert!(homogeneous_w
            .windows(2)
            .any(|values| values[0] != values[1]));
        assert_eq!(
            &scene.textures[0].pixels[4..8],
            &[0, 0, 0, 0],
            "owner pixels are removed from the base layer"
        );
        assert_eq!(
            &scene.textures[1].pixels[4..8],
            &[255, 255, 255, 255],
            "scalar masks become owner-intensity RGBA"
        );
    }

    #[test]
    fn scoped_renderer_config_reaches_generic_gui_draws_and_restores_nesting() {
        let image = ImageData::new(1, 1, vec![100, 100, 100, 128]);
        let mut surface = Surface::new(3, 2, PixelFormat::Rgba8888);
        surface.fill(Color::opaque(10, 10, 10));

        {
            let _outer = activate_advanced_renderer_config(AdvancedRendererConfig {
                blit_offset: 100,
                allowed_blit_modes: 0,
                ..AdvancedRendererConfig::DEFAULT
            });
            draw_image_bilinear_additive(
                &mut surface,
                &GuiRect::new(0.0, 0.0, 1.0, 1.0),
                &image,
                None,
            );
            assert_eq!(surface.get_pixel(0, 0), Some(Color::opaque(10, 10, 10)));
            assert_eq!(surface.get_pixel(1, 1), Some(Color::opaque(55, 55, 55)));

            {
                let _inner = activate_advanced_renderer_config(AdvancedRendererConfig {
                    allowed_blit_modes: C4GFXBLIT_ADDITIVE,
                    ..AdvancedRendererConfig::DEFAULT
                });
                draw_image_bilinear_additive(
                    &mut surface,
                    &GuiRect::new(0.0, 0.0, 1.0, 1.0),
                    &image,
                    None,
                );
                assert_eq!(surface.get_pixel(0, 0), Some(Color::opaque(60, 60, 60)));
            }

            draw_color_rect(
                &mut surface,
                SurfaceRect::new(0, 0, 1, 1),
                Color::opaque(200, 0, 0),
                None,
            );
            assert_eq!(
                surface.get_pixel(1, 1),
                Some(Color::opaque(200, 0, 0)),
                "dropping the nested scope restores the outer BlitOffset",
            );
        }

        draw_image_bilinear_additive(
            &mut surface,
            &GuiRect::new(2.0, 0.0, 1.0, 1.0),
            &image,
            None,
        );
        assert_eq!(
            surface.get_pixel(2, 0),
            Some(Color::opaque(60, 60, 60)),
            "dropping the outer scope restores the compatibility path",
        );

        let mut normalized_box = Surface::new(1, 1, PixelFormat::Rgba8888);
        let _box = activate_advanced_renderer_config(AdvancedRendererConfig {
            no_box_fades: true,
            ..AdvancedRendererConfig::DEFAULT
        });
        draw_color_rect(
            &mut normalized_box,
            SurfaceRect::new(0, 0, 1, 1),
            Color::opaque(64, 0, 0),
            None,
        );
        assert_eq!(
            normalized_box.get_pixel(0, 0),
            Some(Color::opaque(8, 0, 0)),
            "NoBoxFades maps even a uniform DrawBoxDw through NormalizeColors",
        );

        let row = [
            0, 0, 0, 255, 64, 64, 64, 255, 128, 128, 128, 255, 255, 255, 255, 255,
        ];
        let indented_image = ImageData::new(4, 4, row.repeat(4));
        let mut indented = Surface::new(4, 1, PixelFormat::Rgba8888);
        let _indent = activate_advanced_renderer_config(AdvancedRendererConfig {
            tex_indent: 500,
            ..AdvancedRendererConfig::DEFAULT
        });
        draw_image_bilinear(
            &mut indented,
            &GuiRect::new(0.0, 0.0, 4.0, 1.0),
            &indented_image,
            None,
        );
        assert_eq!(
            (0..4)
                .map(|x| indented.get_pixel(x, 0).unwrap().r)
                .collect::<Vec<_>>(),
            vec![26, 77, 128, 230],
        );
    }

    fn empty_sprites() -> Arc<HashMap<String, DefinitionSprite>> {
        Arc::new(HashMap::new())
    }

    fn empty_cursor_atlas() -> Arc<CursorAtlas> {
        Arc::new(CursorAtlas::empty())
    }

    fn empty_hud_graphics() -> Arc<HudGraphics> {
        Arc::new(HudGraphics::default())
    }

    /// A graphics system with the empty test assets: no object sprites, no
    /// cursor atlas and no HUD sheets. The render tests construct dozens of
    /// these and only ever vary the surface size, ground height and label.
    fn test_graphics(
        surface_width: u32,
        surface_height: u32,
        fallback_ground_height: i32,
        scenario_label: &str,
    ) -> GraphicsSystem {
        GraphicsSystem::new(
            surface_width,
            surface_height,
            fallback_ground_height,
            scenario_label,
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        )
    }

    struct RepositoryContentResolver {
        root: PathBuf,
    }

    impl LegacyDefinitionResolver for RepositoryContentResolver {
        fn resolve_definition_groups(
            &self,
            _scenario: &Group,
            identifier: &str,
        ) -> Result<Vec<Group>, ScenarioError> {
            Group::open(self.root.join(identifier.replace('\\', "/")))
                .map(|group| vec![group])
                .map_err(ScenarioError::Resources)
        }
    }

    /// Loads an installed tutorial through the same definition/material/system
    /// prerequisites as the app. These tests deliberately render real engine
    /// snapshots and real Graphics.png facets rather than reconstructed test
    /// sprites.
    fn load_repository_tutorial(number: u8) -> Engine {
        let repository = test_support::repo_root();
        let content = repository.join("content");
        let scenario_path = content.join(format!("Tutorial.c4f/Tutorial{number:02}.c4s"));
        let scenario = Scenario::load_from_path_with(
            &scenario_path,
            &RepositoryContentResolver {
                root: content.clone(),
            },
        )
        .unwrap_or_else(|error| panic!("scenario `{}` loads: {error}", scenario_path.display()));

        let material_group =
            Group::open(content.join("Material.c4g")).expect("installed Material.c4g opens");
        let materials =
            MaterialLibrary::from_group(&material_group).expect("installed materials load");
        let system_group =
            Group::open(repository.join("planet/System.c4g")).expect("System.c4g opens");
        let system_scripts = load_system_scripts(&system_group).expect("system scripts load");

        let mut engine = Engine::with_seed(0);
        engine.configure_materials_from_library(&materials);
        engine.install_global_scripts(&system_scripts);
        engine.set_standard_names(
            system_group
                .read_file("Names.txt")
                .ok()
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned()),
        );
        scenario.apply(&mut engine).unwrap_or_else(|error| {
            panic!("scenario `{}` applies: {error}", scenario_path.display())
        });
        engine
    }

    fn join_repository_player(engine: &mut Engine, name: &str) -> i32 {
        engine
            .join_player(JoinPlayerConfig {
                name: name.to_string(),
                player_info_id: 0,
                score: 0,
                rounds: 0,
                rounds_won: 0,
                rounds_lost: 0,
                total_playing_time: 0,
                team: None,
                color_dw: 0xff_00_00,
                pref_color: 0,
                pref_position: 0,
                crew: Vec::new(),
                control_style: false,
                auto_context_menu: false,
                startup_player_count: 1,
            })
            .expect("repository tutorial player joins")
            .number()
    }

    fn real_elevator_sprites(engine: &Engine) -> Arc<HashMap<String, DefinitionSprite>> {
        let mut sprites = HashMap::new();
        for definition_id in ["ELEV", "ELEC"] {
            let image = engine
                .definition_sprite_image(definition_id, None)
                .unwrap_or_else(|| panic!("{definition_id} has its real Graphics.png"));
            let width = image.width();
            let height = image.height();
            let color_mask = image
                .color_mask()
                .map(|mask| ColorByOwnerMask::new(width, height, mask));
            sprites.insert(
                sprite_map_key(definition_id, None),
                DefinitionSprite {
                    graphics_scale: 1.0,
                    image: ImageData::from_arc(width, height, image.into_pixels()),
                    actions: engine
                        .definition_action_graphics(definition_id)
                        .unwrap_or_default(),
                    color_mask,
                    shape: engine.definition_shape_rect(definition_id),
                    fire_top: engine.definition_fire_top(definition_id),
                    rotateable: engine.definition_rotateable(definition_id),
                    line: engine.definition_line(definition_id),
                    stretch_growth: engine.definition_stretch_growth(definition_id),
                    top_face: engine.definition_top_face(definition_id),
                    picture: None,
                },
            );
        }
        Arc::new(sprites)
    }

    fn assert_surface_pixels_eq(actual: &Surface, expected: &Surface, context: &str) {
        assert_eq!(actual.width(), expected.width(), "{context}: width");
        assert_eq!(actual.height(), expected.height(), "{context}: height");
        if let Some((index, (actual_pixel, expected_pixel))) = actual
            .pixels()
            .chunks_exact(4)
            .zip(expected.pixels().chunks_exact(4))
            .enumerate()
            .find(|(_, (actual, expected))| actual != expected)
        {
            let x = index % actual.width() as usize;
            let y = index / actual.width() as usize;
            panic!(
                "{context}: first mismatch at ({x}, {y}): actual={actual_pixel:?}, expected={expected_pixel:?}"
            );
        }
    }

    fn make_snapshot() -> SimulationSnapshot {
        SimulationSnapshot {
            frame: 0,
            game_time: 0,
            game_over: false,
            round_results: Default::default(),
            league_name: Vec::new(),
            player_info_league_progress_data: Default::default(),
            player_info_league_scores: Default::default(),
            physics: None,
            objects: vec![ObjectSnapshot {
                id: ObjectId::new(1),
                definition_id: "TestObject".to_string(),
                custom_name: None,
                position: Vector2::new(100, 100),
                velocity: Vector2::ZERO,
                rotation: 0,
                energy: 100,
                need_energy: false,
                construction: clonk_engine::FULL_CON,
                damage: 0,
                magic_energy: 0,
                magic_capacity: 0,
                action: Default::default(),
                direction: Default::default(),
                command_direction: Default::default(),
                action_procedure: None,
                effects: Vec::new(),
                vertices: Vec::new(),
                current_shape: None,
                current_fire_top: None,
                contact_density: 50,
                own_vertices: None,
                vertex_contacts: Vec::new(),
                solid_mask_override: None,
                container: None,
                layer: None,
                visibility: 0,
                blit_mode: 0,
                color: 0,
                color_modulation: 0,
                picture_rect: Default::default(),
                contents: Vec::new(),
                components: HashMap::new(),
                component_order: Vec::new(),
                status: Default::default(),
                owner: 0,
                controller: 0,
                category: clonk_engine::DEFAULT_CATEGORY,
                crew_member: true,
                plr_view_range: 0,
                selected: false,
                alive: true,
                base_graphics: None,
                graphics_overlays: Vec::new(),
                draw_transform: None,
                command_queue: Vec::new(),
                command_stack: CommandStackSnapshot::default(),
                local_vars: HashMap::new(),
                in_liquid: false,
                mobile: false,
                ocf: 0,
                timer: 0,
                own_mass: 0,
                on_fire: false,
                fire_phase: 0,
                fire_caused_by: -1,
                info_physical: None,
                temporary_physical: None,
                physical_changes: Vec::new(),
                breath: 0,
                last_energy_loss_cause: -1,
                base: -1,
                fixed_position: None,
                fixed_velocity: None,
                rotation_velocity: None,
                fixed_rotation: None,
            }],
            render_order: Vec::new(),
            environment: EnvironmentFrame::default(),
            sky: None,
            weather_events: Vec::new(),
            global_effects: Vec::new(),
            script_globals: Default::default(),
            particles: Vec::new(),
            players: Vec::new(),
            fow_players: Default::default(),
            crew_selection: Default::default(),
            crew_roles: Default::default(),
            known_crew_owners: Vec::new(),
            eliminated_crew_owners: Vec::new(),
            landscape: Some(Landscape::flat(256, 120)),
            rng: clonk_engine::LcgRng::seed_from_u64(0),
            surfaces: Vec::new(),
            hud: Default::default(),
            controls: Vec::new(),
            network_packets: Vec::new(),
            definition_categories: Default::default(),
            definition_closed_containers: Default::default(),
            definition_lines: Default::default(),
            transfer_zones: Vec::new(),
            pathfinder_debug: Default::default(),
            menu_requests: Vec::new(),
            audio: Vec::new(),
        }
    }

    #[test]
    fn debug_vertex_marks_are_flag_gated() {
        let mut snapshot = make_snapshot();
        let object = &mut snapshot.objects[0];
        object.position = Vector2::new(16, 16);
        object.vertices = vec![
            ObjectVertex::new(0, 0).with_cnat(clonk_engine::CNAT_NO_COLLISION),
            ObjectVertex::new(4, 0),
        ];
        object.vertex_contacts = vec![0, clonk_engine::CNAT_BOTTOM];
        let sprite = DefinitionSprite {
            image: ImageData::new(2, 2, vec![0; 16]),
            actions: HashMap::new(),
            color_mask: None,
            graphics_scale: 1.0,
            shape: Some(DefinitionRect::new(-1, -1, 2, 2)),
            fire_top: 0,
            rotateable: 0,
            line: 0,
            stretch_growth: false,
            top_face: None,
            picture: None,
        };
        let mut graphics = GraphicsSystem::new(
            32,
            32,
            32,
            "vertex overlay",
            test_font(),
            Arc::new(HashMap::from([(
                sprite_map_key("TestObject", None),
                sprite,
            )])),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        let draw = |graphics: &mut GraphicsSystem| {
            graphics.draw_objects(
                &snapshot.objects,
                &snapshot.render_order,
                &snapshot.definition_lines,
                &snapshot.players,
                OWNER_NONE,
                1.0,
                &HashMap::new(),
                ObjectRenderPass::Normal,
                None,
            );
        };

        graphics.surface_mut().fill(Color::transparent());
        draw(&mut graphics);
        assert_eq!(
            graphics.surface().get_pixel(16, 16),
            Some(Color::transparent()),
            "the developer overlay is inert by default"
        );

        graphics.set_debug_draw_flags(DebugDrawFlags {
            show_vertices: true,
            ..DebugDrawFlags::default()
        });
        draw(&mut graphics);
        assert_eq!(
            graphics.surface().get_pixel(16, 16),
            Some(graphics.game_palette.color(14)),
            "CNAT_NoCollision uses CBlue for the three-pixel vertex cross"
        );
        assert_eq!(
            graphics.surface().get_pixel(18, 14),
            Some(graphics.game_palette.color(6)),
            "a contacted vertex receives the surrounding CWhite frame"
        );

        snapshot.objects[0].position = Vector2::new(-20, 16);
        snapshot.objects[0].vertices[0].x = 36;
        snapshot.objects[0].vertices[1].x = 40;
        graphics.surface_mut().fill(Color::transparent());
        graphics.draw_objects(
            &snapshot.objects,
            &snapshot.render_order,
            &snapshot.definition_lines,
            &snapshot.players,
            OWNER_NONE,
            1.0,
            &HashMap::new(),
            ObjectRenderPass::Normal,
            None,
        );
        assert_eq!(
            graphics.surface().get_pixel(16, 16),
            Some(Color::transparent()),
            "the native output-boundary return suppresses the debug tail even when a displaced vertex would land onscreen"
        );
    }

    #[test]
    fn network_status_text_is_flag_gated_per_viewport() {
        let mut snapshot = make_snapshot();
        snapshot.objects[0].position = Vector2::new(80, 60);
        let render = |show_net_status| {
            let mut graphics = test_graphics(180, 110, 120, "network status");
            graphics.set_debug_draw_flags(DebugDrawFlags {
                show_net_status,
                ..DebugDrawFlags::default()
            });
            graphics.set_network_status_text(Some(
                "Local: Active host Alice (ID 0)|Game Status: go (tick 7) reached ack|Protocols: UDP: UDP (11112 i0 o0 bc0)|Control: Central, Tick 7, Behind 0, Rate 2, PreSend 1, ACT: 20|Clients:"
                    .to_string(),
            ));
            graphics.render_frame(
                &snapshot,
                &[ViewportInput::from_focus(&snapshot.objects[0])],
            );
            graphics.draw_network_status(None);
            (
                graphics.surface().pixels().to_vec(),
                graphics.surface().width() as usize,
                graphics.active_viewport_projections()[0].rect,
            )
        };

        let (hidden, width, rect) = render(false);
        let (visible, visible_width, visible_rect) = render(true);
        assert_eq!(width, visible_width);
        assert_eq!(rect, visible_rect);
        let changed = hidden
            .chunks_exact(4)
            .zip(visible.chunks_exact(4))
            .enumerate()
            .filter(|(_, (hidden, visible))| hidden != visible)
            .map(|(index, _)| ((index % width) as i32, (index / width) as i32))
            .collect::<Vec<_>>();
        assert!(
            changed.len() > 20,
            "enabling ShowNetstatus must rasterize detailed status text"
        );
        assert!(changed.iter().all(|(x, y)| {
            *x >= rect.x
                && *x < rect.x + rect.width as i32
                && *y >= rect.y
                && *y < rect.y + rect.height as i32
        }));
    }

    #[test]
    fn the_diagnostics_overlay_is_inert_until_it_is_given_text() {
        // The port's own overlay (clonk-org/clonk-rs#118) has no C++
        // counterpart, so the whole gate is the app declining to compose text:
        // with none set the frame must be byte-identical to the oracle's.
        let mut snapshot = make_snapshot();
        snapshot.objects[0].position = Vector2::new(80, 60);
        let render = |text: Option<&str>| {
            let mut graphics = test_graphics(180, 110, 120, "diagnostics overlay");
            graphics.set_diagnostics_overlay_text(text.map(str::to_string));
            graphics.render_frame(
                &snapshot,
                &[ViewportInput::from_focus(&snapshot.objects[0])],
            );
            let drawn = graphics.draw_diagnostics_overlay(None);
            (drawn, graphics.surface().pixels().to_vec())
        };

        let (drawn_without, without) = render(None);
        let (drawn_with, with) = render(Some("Sim 36 FPS|Render 9 FPS|Draw 32.5 ms"));
        assert!(!drawn_without, "no text means no draw site at all");
        assert!(drawn_with);
        assert_ne!(without, with);
    }

    #[test]
    fn the_diagnostics_overlay_stands_clear_of_the_network_status_block() {
        // `C4Network2::DrawStatus` owns (+20,+50) in every viewport and its
        // placement is pinned to the oracle. The port-only overlay is the one
        // that yields, so turning it on can never move a C++-parity draw.
        let mut snapshot = make_snapshot();
        snapshot.objects[0].position = Vector2::new(80, 60);
        let render = |show_net_status| {
            let mut graphics = test_graphics(180, 220, 120, "diagnostics overlay");
            graphics.set_debug_draw_flags(DebugDrawFlags {
                show_net_status,
                ..DebugDrawFlags::default()
            });
            graphics.set_network_status_text(Some(
                "Local: Active host Alice (ID 0)|Game Status: go (tick 7) reached ack".to_string(),
            ));
            graphics.set_diagnostics_overlay_text(Some(
                "Sim 36 FPS|Render 9 FPS|Draw 32.5 ms".to_string(),
            ));
            graphics.render_frame(
                &snapshot,
                &[ViewportInput::from_focus(&snapshot.objects[0])],
            );
            let baseline = graphics.surface().pixels().to_vec();
            graphics.draw_network_status(None);
            let with_status = graphics.surface().pixels().to_vec();
            graphics.draw_diagnostics_overlay(None);
            let width = graphics.surface().width() as usize;
            let top_row = |before: &[u8], after: &[u8]| {
                before
                    .chunks_exact(4)
                    .zip(after.chunks_exact(4))
                    .position(|(before, after)| before != after)
                    .map(|index| index / width)
            };
            (
                top_row(&with_status, graphics.surface().pixels()),
                top_row(&baseline, &with_status),
            )
        };

        let (alone, no_status) = render(false);
        let (below, status_top) = render(true);
        assert_eq!(no_status, None, "the flag still gates the network status");
        let alone = alone.expect("the overlay draws on its own");
        let below = below.expect("the overlay still draws beside the status");
        let status_top = status_top.expect("the network status drew its own text");
        assert_eq!(
            alone, status_top,
            "with nothing above it the overlay takes the same anchor"
        );
        assert!(
            below > alone,
            "a visible network status pushes the overlay below it: \
             alone at row {alone}, beside it at row {below}"
        );
    }

    #[test]
    fn solid_mask_mode_uses_surface8_and_suppresses_the_object_sprite() {
        let mut snapshot = make_snapshot();
        let object = &mut snapshot.objects[0];
        object.position = Vector2::new(12, 12);
        object.solid_mask_override = Some(DefinitionTargetRect::new(0, 0, 1, 1, 0, 0));
        let sprite = DefinitionSprite {
            image: ImageData::new(3, 3, [220, 30, 20, 255].repeat(9)),
            actions: HashMap::new(),
            color_mask: None,
            graphics_scale: 1.0,
            shape: Some(DefinitionRect::new(-1, -1, 3, 3)),
            fire_top: 0,
            rotateable: 0,
            line: 0,
            stretch_growth: false,
            top_face: None,
            picture: None,
        };
        let mut graphics = GraphicsSystem::new(
            24,
            24,
            24,
            "solid masks",
            test_font(),
            Arc::new(HashMap::from([(
                sprite_map_key("TestObject", None),
                sprite,
            )])),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        let draw_object = |graphics: &mut GraphicsSystem| {
            graphics.draw_objects(
                &snapshot.objects,
                &snapshot.render_order,
                &snapshot.definition_lines,
                &snapshot.players,
                OWNER_NONE,
                1.0,
                &HashMap::new(),
                ObjectRenderPass::Normal,
                None,
            );
        };
        graphics.surface_mut().fill(Color::transparent());
        draw_object(&mut graphics);
        assert_eq!(
            graphics.surface().get_pixel(12, 12),
            Some(Color::opaque(220, 30, 20))
        );
        graphics.surface_mut().fill(Color::transparent());
        graphics.set_debug_draw_flags(DebugDrawFlags {
            show_solid_mask: true,
            ..DebugDrawFlags::default()
        });
        draw_object(&mut graphics);
        assert_eq!(
            graphics.surface().get_pixel(12, 12),
            Some(Color::transparent()),
            "the mask is already represented by Surface8, so Draw and DrawTopFace return"
        );

        let mut bytes = vec![0; 24 * 24];
        bytes[4 * 24 + 3] = 14;
        bytes[4 * 24 + 4] = 142;
        graphics.set_material_render_info(Arc::new(HashMap::from([(
            "earth".to_string(),
            MaterialRenderInfo::new(
                [17, 33, 65, 0, 0, 0, 0, 0, 0],
                [64, 0, 0, 192, 0, 0],
                None,
                0,
                50,
            ),
        )])));
        let mut material_names = vec![None; 128];
        material_names[14] = Some("Earth".to_string());
        let mut landscape =
            Landscape::with_default_material(24, vec![24; 24], None).expect("test landscape");
        landscape.set_world_height(24);
        landscape.set_pixel_grid(PixelGrid::new(
            24,
            24,
            bytes,
            vec![0; 128],
            material_names,
            vec![None; 128],
        ));
        assert!(graphics.draw_ground_surface8(Some(&landscape), None));
        assert_eq!(
            graphics.surface().get_pixel(3, 4),
            Some(Color::new(12, 24, 48, 191)),
            "the low Surface8 slot uses Mat2Pal Color and Alpha[0]"
        );
        assert_eq!(
            graphics.surface().get_pixel(4, 4),
            Some(Color::new(4, 8, 16, 63)),
            "the +128 IFT slot uses the same Mat2Pal RGB and Alpha[3]"
        );
    }

    #[test]
    fn a_put_solid_mask_composes_as_the_material_it_covers() {
        // C4Landscape::DoRelights removes every C4SolidMask before recomputing
        // Surface32 and puts them back afterwards (C4Landscape.cpp:2497,2501),
        // so the drawn landscape never contains a mask byte - neither as its
        // own material nor in the placement its neighbours shade against.
        let mut graphics = test_graphics(24, 24, 24, "Masked landscape");
        graphics.surface_mut().fill(Color::transparent());
        graphics.set_material_textures(Arc::new(HashMap::from([(
            "earth".to_string(),
            ImageData::new(1, 1, vec![255, 255, 255, 255]),
        )])));
        graphics.set_material_render_info(Arc::new(HashMap::from([(
            "earth".to_string(),
            MaterialRenderInfo::new([17, 33, 65, 0, 0, 0, 0, 0, 0], [0; 6], None, 0, 50),
        )])));
        let mut material_names = vec![None; 128];
        material_names[14] = Some("Earth".to_string());
        // Vehicle carries no render info, so a composed mask byte shows up as
        // an untouched pixel rather than as earth.
        material_names[2] = Some("Vehicle".to_string());
        let mut landscape =
            Landscape::with_default_material(24, vec![24; 24], None).expect("test landscape");
        landscape.set_world_height(24);
        let mut texture_names = vec![None; 128];
        texture_names[14] = Some("Earth".to_string());
        landscape.set_pixel_grid(PixelGrid::new(
            24,
            24,
            vec![14; 24 * 24],
            vec![0; 128],
            material_names,
            texture_names,
        ));

        landscape.grid_write_mask_byte(10, 10, 2);

        assert!(graphics.draw_ground_textured(Some(&landscape), None));
        let masked = graphics.surface().get_pixel(10, 10);
        assert_eq!(
            masked,
            graphics.surface().get_pixel(14, 10),
            "the masked pixel draws exactly like the earth beside it"
        );
        assert_ne!(masked, Some(Color::transparent()), "and it is drawn at all");
    }

    #[test]
    fn full_landscape_capture_uses_ownerless_world_pass_and_restores_viewport() {
        let mut snapshot = make_snapshot();
        snapshot.objects[0].position = Vector2::new(100, 60);
        snapshot.players = vec![PlayerState {
            id: 0,
            fog_of_war: true,
            ..PlayerState::default()
        }];
        snapshot.environment.fow_color = 0x00ff_0000;
        let focus = &snapshot.objects[0];
        let mut graphics = test_graphics(64, 40, 120, "Full landscape capture");
        graphics.render_frame(
            &snapshot,
            &[ViewportInput::new(0, focus.position, 1.0, focus)],
        );
        let screen_before = graphics.surface().pixels().to_vec();
        let viewport_before = graphics.active_viewport_projections();

        let capture = graphics
            .render_full_landscape(&snapshot)
            .expect("active viewport and landscape produce a full capture");

        assert_eq!((capture.width(), capture.height()), (256, 120));
        assert_ne!(
            capture.get_pixel(0, 0),
            Some(Color::opaque(255, 0, 0)),
            "temporary NO_OWNER projection disables the player's red fog"
        );
        assert_eq!(graphics.surface().pixels(), screen_before);
        assert_eq!(graphics.active_viewport_projections(), viewport_before);
    }

    #[test]
    fn full_landscape_capture_uses_installed_not_pending_gamma() {
        let snapshot = make_snapshot();
        let mut changed = snapshot.clone();
        changed
            .environment
            .gamma
            .set_ramp(0, [0x102030, 0x405060, 0x708090]);
        let mut graphics = test_graphics(64, 40, 120, "Full landscape gamma");
        graphics.render_frame(
            &snapshot,
            &[ViewportInput::from_focus(&snapshot.objects[0])],
        );

        let before_latch = graphics
            .render_full_landscape(&changed)
            .expect("full capture before pending gamma is latched");
        graphics.render_frame(&changed, &[ViewportInput::from_focus(&changed.objects[0])]);
        let after_latch = graphics
            .render_full_landscape(&changed)
            .expect("full capture after pending gamma is latched");

        assert_ne!(
            before_latch.pixels(),
            after_latch.pixels(),
            "full capture reads CStdDDraw's installed ramp, not pending controls"
        );
    }

    #[test]
    fn viewport_fog_shrouds_far_pixels_fades_the_edge_and_preserves_non_fow_bytes() {
        let mut snapshot = make_snapshot();
        snapshot.objects[0].position = Vector2::new(100, 60);
        snapshot.objects[0].plr_view_range = 48;
        snapshot.players = vec![PlayerState {
            id: 0,
            fog_of_war: false,
            ..PlayerState::default()
        }];

        let render = |snapshot: &SimulationSnapshot| {
            let mut graphics = test_graphics(128, 80, 120, "FoW render");
            graphics.render_frame_with_gamma(
                snapshot,
                &[ViewportInput::new(
                    0,
                    snapshot.objects[0].position,
                    1.0,
                    &snapshot.objects[0],
                )],
                None,
            );
            graphics
        };

        let baseline = render(&snapshot);
        let mut metadata_only = snapshot.clone();
        metadata_only.environment.fow_resolution = 8;
        metadata_only.environment.fow_color = 0x0000_ff00;
        metadata_only.fow_players.insert(
            0,
            FogOfWarPlayerFrame {
                view_objects: vec![ObjectId::new(1)],
                view_target: None,
            },
        );
        let metadata_render = render(&metadata_only);
        assert_eq!(
            baseline.surface().pixels(),
            metadata_render.surface().pixels(),
            "FoW metadata is inert while the viewport player's flag is false"
        );

        let mut fog = metadata_only.clone();
        fog.players[0].fog_of_war = true;
        fog.environment.fow_color = 0;
        let fog_render = render(&fog);
        let viewport = fog_render.active_viewports[0].clone();
        let output_at_world = |world_x: i32, world_y: i32| {
            (
                (viewport.content_rect.x as f32
                    + (world_x as f32 - viewport.viewport_x) * viewport.zoom)
                    .round() as u32,
                (viewport.content_rect.y as f32
                    + (world_y as f32 - viewport.viewport_y) * viewport.zoom)
                    .round() as u32,
            )
        };
        let far = output_at_world(viewport.viewport_x as i32, viewport.viewport_y as i32);
        let near = output_at_world(100, 60);
        let fade = output_at_world(140, 60);
        assert_eq!(
            fog_render.surface().get_pixel(far.0, far.1),
            Some(Color::opaque(0, 0, 0))
        );
        let baseline_near = baseline.surface().get_pixel(near.0, near.1).unwrap();
        assert_eq!(
            fog_render.surface().get_pixel(near.0, near.1),
            Some(modulate_surface_color(baseline_near, 0x00ff_ffff))
        );
        let fade_color = fog_render.surface().get_pixel(fade.0, fade.1).unwrap();
        let near_color = fog_render.surface().get_pixel(near.0, near.1).unwrap();
        let brightness =
            |color: Color| u16::from(color.r) + u16::from(color.g) + u16::from(color.b);
        assert!(brightness(fade_color) < brightness(near_color));
        assert!(brightness(fade_color) > 0);

        let mut colored_fog = fog;
        colored_fog.environment.fow_color = 0x0000_ff00;
        let colored_render = render(&colored_fog);
        assert_eq!(
            colored_render.surface().get_pixel(far.0, far.1),
            Some(Color::opaque(0, 255, 0)),
            "nonzero FoWColor is the fully shrouded backdrop"
        );
    }

    #[test]
    fn fog_stops_before_parallax_foreground_and_hud() {
        let mut snapshot = make_snapshot();
        snapshot.objects[0].position = Vector2::new(100, 60);
        snapshot.objects[0].plr_view_range = 12;
        let mut fogged = snapshot.objects[0].clone();
        fogged.id = ObjectId::new(2);
        fogged.definition_id = "FoggedWorld".into();
        fogged.position = Vector2::new(50, 60);
        fogged.plr_view_range = 0;
        fogged.crew_member = false;
        let mut parallax = fogged.clone();
        parallax.id = ObjectId::new(3);
        parallax.definition_id = "ParallaxHud".into();
        parallax.position = Vector2::new(120, 50);
        parallax.category =
            clonk_engine::DEFAULT_CATEGORY | CATEGORY_FOREGROUND_FLAG | CATEGORY_PARALLAX_FLAG;
        snapshot.objects.extend([fogged.clone(), parallax]);
        snapshot.render_order = snapshot.objects.iter().map(|object| object.id).collect();
        snapshot.players = vec![PlayerState {
            id: 0,
            fog_of_war: false,
            ..PlayerState::default()
        }];
        snapshot.environment.fow_resolution = 8;
        snapshot.fow_players.insert(
            0,
            FogOfWarPlayerFrame {
                view_objects: vec![snapshot.objects[0].id],
                view_target: None,
            },
        );

        let mut sprites = (*solid_sprite(
            "FoggedWorld",
            4,
            4,
            Color::opaque(220, 20, 20),
            Some(DefinitionRect::new(0, 0, 4, 4)),
            false,
        ))
        .clone();
        sprites.extend(
            (*solid_sprite(
                "ParallaxHud",
                4,
                4,
                Color::opaque(20, 220, 20),
                Some(DefinitionRect::new(0, 0, 4, 4)),
                false,
            ))
            .clone(),
        );
        let board_color = Color::opaque(20, 40, 220);
        let board = ImageData::new(
            4,
            50,
            (0..200)
                .flat_map(|_| [board_color.r, board_color.g, board_color.b, board_color.a])
                .collect(),
        );
        let render = |snapshot: &SimulationSnapshot| {
            let mut graphics = GraphicsSystem::new(
                128,
                120,
                120,
                "FoW lifetime",
                test_font(),
                Arc::new(sprites.clone()),
                empty_cursor_atlas(),
                Arc::new(HudGraphics {
                    upper_board: Some(board.clone()),
                    ..HudGraphics::default()
                }),
            );
            graphics.render_frame_with_gamma(
                snapshot,
                &[ViewportInput::new(
                    0,
                    snapshot.objects[0].position,
                    1.0,
                    &snapshot.objects[0],
                )],
                None,
            );
            graphics
        };

        let baseline = render(&snapshot);
        let normal = baseline
            .world_to_screen(0, fogged.position)
            .expect("far world object in viewport");
        let normal = (normal.0.round() as u32 + 1, normal.1.round() as u32 + 1);
        // world_to_screen models an ordinary world object. This one is
        // C4D_Parallax with Local(0)/Local(1) unset and non-negative
        // coordinates, so ApplyParallaxity yields cotx = coty = 0
        // (src/C4Object.cpp:5839-5852) and C4Object::Draw anchors it to the
        // viewport content origin rather than to the scroll
        // (src/C4Object.cpp:2271).
        let parallax_viewport = baseline
            .active_viewports
            .iter()
            .find(|viewport| viewport.owner == 0)
            .expect("owner viewport");
        let parallax_screen = (
            120.0 * parallax_viewport.zoom + parallax_viewport.content_rect.x as f32,
            50.0 * parallax_viewport.zoom + parallax_viewport.content_rect.y as f32,
        );
        let parallax_pixel = (
            parallax_screen.0.round() as u32 + 1,
            parallax_screen.1.round() as u32 + 1,
        );
        assert_eq!(
            baseline.surface().get_pixel(normal.0, normal.1),
            Some(Color::opaque(220, 20, 20)),
        );
        assert_eq!(
            baseline
                .surface()
                .get_pixel(parallax_pixel.0, parallax_pixel.1),
            Some(Color::opaque(20, 220, 20)),
        );

        let mut fog_snapshot = snapshot;
        fog_snapshot.players[0].fog_of_war = true;
        let fog = render(&fog_snapshot);
        assert_eq!(
            fog.surface().get_pixel(normal.0, normal.1),
            Some(Color::opaque(0, 0, 0)),
            "ordinary world sprite is shrouded",
        );
        assert_eq!(
            fog.surface().get_pixel(parallax_pixel.0, parallax_pixel.1),
            baseline
                .surface()
                .get_pixel(parallax_pixel.0, parallax_pixel.1),
            "ForegroundParallax is drawn after ClrModMap is disabled",
        );
        assert_eq!(
            fog.surface().get_pixel(0, 0),
            baseline.surface().get_pixel(0, 0),
            "fullscreen HUD chrome remains byte-identical",
        );
        assert_eq!(fog.surface().get_pixel(0, 0), Some(board_color));
    }

    #[test]
    fn zoomed_scroll_border_keeps_fog_in_content_coordinates() {
        let mut snapshot = make_snapshot();
        snapshot.objects[0].position = Vector2::new(10, 10);
        snapshot.objects[0].plr_view_range = 12;
        snapshot.players = vec![PlayerState {
            id: 0,
            fog_of_war: true,
            ..PlayerState::default()
        }];
        snapshot.environment.fow_resolution = 8;
        snapshot.fow_players.insert(
            0,
            FogOfWarPlayerFrame {
                view_objects: vec![snapshot.objects[0].id],
                view_target: None,
            },
        );
        let mut graphics = test_graphics(200, 120, 120, "zoomed FoW border");
        graphics.render_frame_with_gamma(
            &snapshot,
            &[ViewportInput::new(
                0,
                snapshot.objects[0].position,
                2.0,
                &snapshot.objects[0],
            )],
            None,
        );
        let viewport = &graphics.active_viewports[0];
        assert!(viewport.content_rect.x > viewport.rect.x);
        assert!(viewport.content_rect.y > viewport.rect.y);
        let at_world = |world: Vector2| {
            (
                (viewport.content_rect.x as f32
                    + (world.x as f32 - viewport.viewport_x) * viewport.zoom)
                    .round() as u32,
                (viewport.content_rect.y as f32
                    + (world.y as f32 - viewport.viewport_y) * viewport.zoom)
                    .round() as u32,
            )
        };
        let near = at_world(snapshot.objects[0].position);
        let far = at_world(Vector2::new(50, 10));
        assert_ne!(
            graphics.surface().get_pixel(near.0, near.1),
            Some(Color::opaque(0, 0, 0)),
        );
        assert_eq!(
            graphics.surface().get_pixel(far.0, far.1),
            Some(Color::opaque(0, 0, 0)),
        );
    }

    fn standard_gamma_color(color: Color) -> Color {
        gamma_encode_fragment(color, &clonk_graphics::GammaRamp::standard())
    }

    #[test]
    fn bolt_quad_culls_before_rng_and_uses_cpp_random_order() {
        let mut rng = SafeRng::new(1);
        let untouched = rng.clone();
        assert_eq!(
            build_bolt_quad((-1, 3), (8, 3), 8, 8, &mut rng),
            None,
            "both x endpoints outside cull even when the segment spans the facet"
        );
        assert_eq!(rng, untouched, "the x cull precedes SafeRandom");
        assert_eq!(
            build_bolt_quad((3, -1), (3, 8), 8, 8, &mut rng),
            None,
            "the same coarse rule applies independently to y"
        );
        assert_eq!(rng, untouched, "the y cull also precedes SafeRandom");

        assert!(
            build_bolt_quad((0, -1), (8, 7), 8, 8, &mut rng).is_some(),
            "different endpoints can satisfy the independent x/y tests"
        );

        let mut rng = SafeRng::new(1);
        assert_eq!(
            build_bolt_quad((4, 5), (12, 8), 16, 16, &mut rng),
            Some([(4, 5), (12, 8), (12, 9), (6, 3)]),
            "C++ consumes end-x, end-y, start-x, start-y"
        );
        let mut mirror = SafeRng::new(1);
        for _ in 0..4 {
            mirror.random(7);
        }
        assert_eq!(
            rng, mirror,
            "one visible segment consumes exactly four draws"
        );
    }

    #[test]
    fn bolt_rasterizes_the_triangle_strip_instead_of_a_bowtie_contour() {
        let mut surface = Surface::new(24, 20, PixelFormat::Rgba8888);
        let black = Color::opaque(0, 0, 0);
        surface.fill(black);
        let mut rng = SafeRng::new(129);
        draw_object_bolt_segment(
            &mut surface,
            (10, 10),
            (14, 10),
            24,
            20,
            1.0,
            c4_palette_color(6),
            c4_palette_color(6),
            SpriteBlitState::normal(),
            None,
            None,
            &mut rng,
        );

        assert_eq!(
            surface.get_pixel(11, 9),
            Some(Color::opaque(252, 252, 252)),
            "the folded GL strip covers its interior"
        );
        assert_eq!(surface.get_pixel(8, 9), Some(black));
        assert_eq!(surface.get_pixel(13, 9), Some(black));
    }

    #[test]
    fn bolt_strip_shared_edge_blends_once() {
        let mut surface = Surface::new(5, 5, PixelFormat::Rgba8888);
        let black = Color::opaque(0, 0, 0);
        let translucent = Color::new(200, 40, 20, 128);
        surface.fill(black);
        draw_object_triangle(
            &mut surface,
            [(0.0, 0.0), (4.0, 0.0), (0.0, 4.0)],
            [translucent; 3],
            SpriteBlitState::normal(),
            None,
        );
        draw_object_triangle(
            &mut surface,
            [(0.0, 4.0), (4.0, 0.0), (4.0, 4.0)],
            [translucent; 3],
            SpriteBlitState::normal(),
            None,
        );

        assert_eq!(
            surface.get_pixel(1, 2),
            Some(blend_color_over(translucent, black)),
            "GL's top-left rule assigns the shared diagonal to one triangle"
        );
    }

    #[test]
    fn retained_bolt_is_two_native_gpu_triangles_not_covered_pixel_points() {
        let mut surface = Surface::new(24, 20, PixelFormat::Rgba8888);
        surface.begin_gpu_scene_capture();
        let mut rng = SafeRng::new(129);
        draw_object_bolt_segment(
            &mut surface,
            (10, 10),
            (14, 10),
            24,
            20,
            1.0,
            c4_palette_color(6),
            c4_palette_color(6),
            SpriteBlitState::normal(),
            None,
            None,
            &mut rng,
        );

        let scene = surface
            .take_gpu_scene_capture()
            .expect("bolt capture remains active")
            .into_scene(
                [24, 20],
                Color::transparent(),
                &clonk_graphics::GammaRamp::standard(),
            );
        assert_eq!(scene.commands.len(), 1);
        let GpuCommand::Solid {
            vertices, topology, ..
        } = &scene.commands[0]
        else {
            panic!("bolt did not lower to solid geometry");
        };
        assert_eq!(*topology, GpuPrimitiveTopology::TriangleList);
        assert_eq!(vertices.len(), 6, "the GL strip is exactly two triangles");
    }

    #[test]
    fn retained_connect_segment_is_one_gpu_line_plus_its_start_marker() {
        let mut surface = Surface::new(32, 16, PixelFormat::Rgba8888);
        surface.begin_gpu_scene_capture();
        draw_object_line_segment(
            &mut surface,
            (2.25, 3.75),
            (20.125, 9.375),
            c4_palette_color(68),
            c4_palette_color(26),
            SpriteBlitState::normal().with_renderer_config(AdvancedRendererConfig {
                blit_offset: 100,
                ..AdvancedRendererConfig::DEFAULT
            }),
            None,
            None,
        );

        let scene = surface
            .take_gpu_scene_capture()
            .expect("line capture remains active")
            .into_scene(
                [32, 16],
                Color::transparent(),
                &clonk_graphics::GammaRamp::standard(),
            );
        assert_eq!(scene.commands.len(), 2);
        let GpuCommand::Solid {
            vertices,
            topology,
            alpha_mode,
            ..
        } = &scene.commands[0]
        else {
            panic!("CONNECT segment did not lower to solid geometry");
        };
        assert_eq!(*topology, GpuPrimitiveTopology::LineList);
        assert_eq!(*alpha_mode, GpuSolidAlphaMode::SourceOver);
        assert_eq!(vertices.len(), 2);
        assert_eq!(vertices[0].position, [2.75, 4.25, 1.0]);
        assert_eq!(vertices[1].position, [20.625, 9.875, 1.0]);
        let GpuCommand::Solid {
            vertices,
            topology,
            alpha_mode,
            ..
        } = &scene.commands[1]
        else {
            panic!("CONNECT marker did not lower to a solid point");
        };
        assert_eq!(*topology, GpuPrimitiveTopology::PointList);
        assert_eq!(*alpha_mode, GpuSolidAlphaMode::SourceOver);
        assert_eq!(vertices.len(), 1);
        assert_eq!(vertices[0].position, [2.75, 4.25, 1.0]);
    }

    #[test]
    fn retained_zero_length_connect_keeps_empty_line_and_float_marker() {
        let mut surface = Surface::new(8, 8, PixelFormat::Rgba8888);
        surface.begin_gpu_scene_capture();
        draw_object_line_segment(
            &mut surface,
            (2.25, 3.75),
            (2.25, 3.75),
            c4_palette_color(68),
            c4_palette_color(26),
            SpriteBlitState::normal(),
            None,
            None,
        );

        let scene = surface
            .take_gpu_scene_capture()
            .expect("line capture remains active")
            .into_scene(
                [8, 8],
                Color::transparent(),
                &clonk_graphics::GammaRamp::standard(),
            );
        assert_eq!(scene.commands.len(), 2);
        let GpuCommand::Solid {
            vertices, topology, ..
        } = &scene.commands[0]
        else {
            panic!("zero-length CONNECT primary did not remain a line");
        };
        assert_eq!(*topology, GpuPrimitiveTopology::LineList);
        assert_eq!(vertices.len(), 2);
        assert_eq!(vertices[0].position, [2.75, 4.25, 1.0]);
        assert_eq!(vertices[0].position, vertices[1].position);
        let GpuCommand::Solid {
            vertices, topology, ..
        } = &scene.commands[1]
        else {
            panic!("zero-length CONNECT marker did not remain a point");
        };
        assert_eq!(*topology, GpuPrimitiveTopology::PointList);
        assert_eq!(vertices[0].position, [2.75, 4.25, 1.0]);
    }

    #[test]
    fn lightning_render_ignores_the_lockstep_rng() {
        let mut first = make_snapshot();
        first.objects[0].definition_id = "LightningLine".to_string();
        first.objects[0].position = Vector2::new(16, 8);
        first.objects[0].vertices = vec![
            clonk_engine::ObjectVertex::new(10, 8),
            clonk_engine::ObjectVertex::new(20, 8),
        ];
        first.definition_lines.insert(
            DefinitionId::from("LightningLine"),
            DefinitionLineMetadata {
                line: 4,
                line_intersect: 0,
            },
        );
        first.rng = clonk_engine::LcgRng::seed_from_u64(7);
        let first_rng = first.rng.clone();
        let mut second = first.clone();
        second.rng = clonk_engine::LcgRng::seed_from_u64(999_999);
        let second_rng = second.rng.clone();

        let render = |snapshot: &SimulationSnapshot| {
            let mut graphics = test_graphics(32, 16, 16, "Lightning RNG");
            graphics.presentation_rng = SafeRng::new(23);
            graphics.render_frame(snapshot, &[ViewportInput::from_focus(&snapshot.objects[0])]);
            (
                graphics.surface().pixels().to_vec(),
                graphics.presentation_rng,
            )
        };
        let first_render = render(&first);
        let second_render = render(&second);

        assert_eq!(first_render, second_render);
        assert_eq!(first.rng, first_rng);
        assert_eq!(second.rng, second_rng);
    }

    #[test]
    fn typed_lines_draw_before_containment_and_render_lightning_without_sprite_faces() {
        // C4Object::Draw returns through DrawLine before the Contained check,
        // TargetPos/parallax adjustment, or any face drawing
        // (src/C4Object.cpp:2249-2254). Shape vertices are already absolute.
        let sprite = DefinitionSprite {
            graphics_scale: 1.0,
            image: ImageData::new(1, 1, vec![255, 0, 255, 255]),
            actions: HashMap::new(),
            color_mask: None,
            shape: Some(DefinitionRect::new(0, 0, 1, 1)),
            fire_top: 0,
            rotateable: 0,
            line: 0,
            stretch_growth: false,
            top_face: None,
            picture: None,
        };
        let template = make_snapshot().objects.remove(0);
        let mut power = template.clone();
        power.definition_id = "PowerLine".to_string();
        power.position = Vector2::new(20, 8);
        power.container = Some(ObjectId::new(99));
        power.vertices = vec![
            clonk_engine::ObjectVertex::new(2, 3),
            clonk_engine::ObjectVertex::new(6, 3),
        ];
        let mut lightning = template;
        lightning.id = ObjectId::new(2);
        lightning.definition_id = "LightningLine".to_string();
        lightning.position = Vector2::new(20, 8);
        lightning.vertices = vec![
            clonk_engine::ObjectVertex::new(20, 8),
            clonk_engine::ObjectVertex::new(24, 8),
        ];
        let lines = HashMap::from([
            (
                DefinitionId::from("PowerLine"),
                DefinitionLineMetadata {
                    line: 1,
                    line_intersect: 0,
                },
            ),
            (
                DefinitionId::from("LightningLine"),
                DefinitionLineMetadata {
                    line: 4,
                    line_intersect: 0,
                },
            ),
        ]);
        let mut graphics = GraphicsSystem::new(
            32,
            12,
            12,
            "Typed lines",
            test_font(),
            Arc::new(HashMap::from([
                (sprite_map_key("PowerLine", None), sprite.clone()),
                (sprite_map_key("LightningLine", None), sprite),
            ])),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        let black = Color::opaque(0, 0, 0);
        graphics.surface_mut().fill(black);
        graphics.draw_objects(
            &[power.clone(), lightning.clone()],
            &[],
            &lines,
            &[],
            OWNER_NONE,
            1.0,
            &HashMap::new(),
            ObjectRenderPass::Normal,
            None,
        );

        assert_eq!(
            graphics.surface().get_pixel(2, 3),
            Some(Color::opaque(168, 168, 168)),
            "C4FacetEx::DrawLine's secondary color overwrites the start pixel"
        );
        assert_eq!(
            graphics.surface().get_pixel(3, 3),
            Some(Color::opaque(152, 100, 44)),
            "Power uses expanded C4.PAL entry 68"
        );
        assert_eq!(
            graphics.surface().get_pixel(6, 3),
            Some(black),
            "GL_LINES diamond-exit raster is half-open at the endpoint"
        );
        assert_eq!(
            graphics.surface().get_pixel(21, 7),
            Some(Color::opaque(252, 252, 252)),
            "Lightning uses C4FacetEx::DrawBolt's CWhite quadrilateral"
        );
        assert_ne!(
            graphics.surface().get_pixel(20, 8),
            Some(Color::opaque(255, 0, 255)),
            "the line path suppresses the object's magenta sprite face"
        );

        let mut palette_bytes = vec![0_u8; GamePalette::BYTE_LEN];
        palette_bytes[6 * 3..6 * 3 + 3].copy_from_slice(&[1, 2, 3]);
        palette_bytes[26 * 3..26 * 3 + 3].copy_from_slice(&[7, 8, 9]);
        palette_bytes[68 * 3..68 * 3 + 3].copy_from_slice(&[4, 5, 6]);
        let palette = GamePalette::from_c4_pal(&palette_bytes).expect("complete custom C4.pal");
        assert_eq!(palette.color(0), Color::transparent());
        assert_eq!(palette.color(191), Color::new(0, 0, 255, 128));
        graphics.set_game_palette(Arc::new(palette));
        graphics.presentation_rng = SafeRng::default();
        graphics.surface_mut().fill(black);
        graphics.draw_objects(
            &[power, lightning],
            &[],
            &lines,
            &[],
            OWNER_NONE,
            1.0,
            &HashMap::new(),
            ObjectRenderPass::Normal,
            None,
        );
        assert_eq!(
            graphics.surface().get_pixel(2, 3),
            Some(Color::opaque(28, 32, 36)),
            "the active C4.pal supplies the typed-line marker"
        );
        assert_eq!(
            graphics.surface().get_pixel(3, 3),
            Some(Color::opaque(16, 20, 24)),
            "the active C4.pal supplies the typed-line primary color"
        );
        assert_eq!(
            graphics.surface().get_pixel(21, 7),
            Some(Color::opaque(4, 8, 12)),
            "the active C4.pal supplies DrawBolt's white index"
        );
    }

    #[test]
    fn audibility_call_cache_preserves_line_order_and_exact_pass_facets() {
        let template = make_snapshot().objects.remove(0);

        let mut line = template.clone();
        line.id = ObjectId::new(1);
        line.definition_id = "AudibleLine".to_string();
        line.container = Some(ObjectId::new(99));
        line.vertices = vec![clonk_engine::ObjectVertex::new(7, 9)];

        let mut ordinary = template.clone();
        ordinary.id = ObjectId::new(2);
        ordinary.definition_id = "LazyOrdinary".to_string();
        ordinary.position = Vector2::new(12, 14);

        let mut parallax = template.clone();
        parallax.id = ObjectId::new(3);
        parallax.definition_id = "ContentParallax".to_string();
        parallax.position = Vector2::new(300, 70);
        parallax.category = CATEGORY_PARALLAX_FLAG;
        parallax
            .local_vars
            .insert("__local_0".to_string(), clonk_script::Value::Int(50));
        parallax
            .local_vars
            .insert("__local_1".to_string(), clonk_script::Value::Int(25));

        let mut foreground_parallax = template;
        foreground_parallax.id = ObjectId::new(4);
        foreground_parallax.definition_id = "FullParallax".to_string();
        foreground_parallax.position = Vector2::new(400, 90);
        foreground_parallax.category = CATEGORY_FOREGROUND_FLAG | CATEGORY_PARALLAX_FLAG;
        foreground_parallax
            .local_vars
            .insert("__local_0".to_string(), clonk_script::Value::Int(100));
        foreground_parallax
            .local_vars
            .insert("__local_1".to_string(), clonk_script::Value::Int(50));

        let lines = HashMap::from([(
            DefinitionId::from("AudibleLine"),
            DefinitionLineMetadata {
                line: 1,
                line_intersect: 0,
            },
        )]);
        let mut graphics = test_graphics(32, 16, 16, "Audibility calls");
        graphics.content_audibility_facet = Some(AudibilityFacet {
            target_x: 20,
            target_y: 30,
            width: 80,
            height: 40,
        });
        graphics.full_audibility_facet = Some(AudibilityFacet {
            target_x: -10,
            target_y: -20,
            width: 120,
            height: 60,
        });

        graphics.draw_objects(
            &[line.clone(), ordinary.clone(), parallax.clone()],
            &[],
            &lines,
            &[],
            OWNER_NONE,
            1.0,
            &HashMap::new(),
            ObjectRenderPass::Normal,
            None,
        );
        graphics.draw_objects(
            &[foreground_parallax.clone()],
            &[],
            &HashMap::new(),
            &[],
            OWNER_NONE,
            1.0,
            &HashMap::new(),
            ObjectRenderPass::ForegroundParallax,
            None,
        );

        assert_eq!(
            graphics.rendered_object_audibility_calls().get(&line.id),
            Some(&vec![
                RenderedAudibilityCall::World {
                    point: Vector2::new(7, 9),
                },
                RenderedAudibilityCall::World {
                    point: Vector2::new(7, 9),
                },
            ]),
            "DrawLine calls both first and last even for one live vertex, before containment",
        );
        assert!(
            !graphics
                .rendered_object_audibility_calls()
                .contains_key(&ordinary.id),
            "ordinary non-line objects retain native lazy origin mixing",
        );
        assert_eq!(
            graphics
                .rendered_object_audibility_calls()
                .get(&parallax.id),
            Some(&vec![RenderedAudibilityCall::Parallax {
                point: parallax.position,
                rendered_center: Vector2::new(50, 27),
            }]),
            "normal-pass parallax uses the border-clipped content facet",
        );
        assert_eq!(
            graphics
                .rendered_object_audibility_calls()
                .get(&foreground_parallax.id),
            Some(&vec![RenderedAudibilityCall::Parallax {
                point: foreground_parallax.position,
                rendered_center: Vector2::new(50, 20),
            }]),
            "foreground parallax uses the restored full viewport facet",
        );
        assert_eq!(graphics.current_audibility_facet, None);
    }

    #[test]
    fn typed_line_variants_use_cpp_palette_and_saved_numbered_locals() {
        // C4Object::DrawLine's exact type/color table lives at
        // src/C4Object.cpp:2684-2712; Colored/Vertex cast Local[0]/Local[1]
        // to palette indices. Saved numbered locals surface as __local_N.
        let template = make_snapshot().objects.remove(0);
        let cases = [
            ("Power", 1, 68_u8, 26_u8),
            ("Source", 2, 23, 26),
            ("Drain", 3, 23, 26),
            ("Rope", 6, 65, 65),
            ("Colored", 7, 65, 68),
            ("Vertex", 8, 65, 68),
        ];
        let mut objects = Vec::new();
        let mut lines = HashMap::new();
        for (index, (name, line, _, _)) in cases.iter().enumerate() {
            let mut object = template.clone();
            object.id = ObjectId::new(index as u64 + 1);
            object.definition_id = (*name).to_string();
            object.position = Vector2::new(24, 2 + index as i32 * 2);
            object.vertices = vec![
                clonk_engine::ObjectVertex::new(2, 1 + index as i32 * 2),
                clonk_engine::ObjectVertex::new(5, 1 + index as i32 * 2),
            ];
            if *line == 7 || *line == 8 {
                object
                    .local_vars
                    .insert("__local_0".to_string(), clonk_script::Value::Int(65));
                object
                    .local_vars
                    .insert("__local_1".to_string(), clonk_script::Value::Int(68));
            }
            lines.insert(
                DefinitionId::from(*name),
                DefinitionLineMetadata {
                    line: *line,
                    line_intersect: 0,
                },
            );
            objects.push(object);
        }
        let mut graphics = test_graphics(32, 16, 16, "Typed-line variants");
        let black = Color::opaque(0, 0, 0);
        graphics.surface_mut().fill(black);
        graphics.draw_objects(
            &objects,
            &[],
            &lines,
            &[],
            OWNER_NONE,
            1.0,
            &HashMap::new(),
            ObjectRenderPass::Normal,
            None,
        );

        let palette = |index| match index {
            23 => Color::opaque(116, 116, 116),
            26 => Color::opaque(168, 168, 168),
            65 => Color::opaque(88, 52, 8),
            68 => Color::opaque(152, 100, 44),
            _ => unreachable!(),
        };
        for (index, (name, _, primary, marker)) in cases.iter().enumerate() {
            let y = 1 + index as u32 * 2;
            assert_eq!(
                graphics.surface().get_pixel(2, y),
                Some(palette(*marker)),
                "{name} start marker"
            );
            assert_eq!(
                graphics.surface().get_pixel(3, y),
                Some(palette(*primary)),
                "{name} primary segment"
            );
            assert_eq!(
                graphics.surface().get_pixel(5, y),
                Some(black),
                "{name} endpoint remains half-open"
            );
        }
    }

    #[test]
    fn typed_line_raster_preserves_cpp_joints_zero_length_and_palette_alpha() {
        let mut object = make_snapshot().objects.remove(0);
        object.definition_id = "BentPower".to_string();
        object.position = Vector2::new(24, 10);
        object.vertices = vec![
            clonk_engine::ObjectVertex::new(2, 2),
            clonk_engine::ObjectVertex::new(5, 2),
            clonk_engine::ObjectVertex::new(5, 5),
        ];
        let mut mod2_object = object.clone();
        mod2_object.id = ObjectId::new(2);
        mod2_object.vertices = vec![
            clonk_engine::ObjectVertex::new(2, 6),
            clonk_engine::ObjectVertex::new(5, 6),
        ];
        mod2_object.blit_mode = C4GFXBLIT_MOD2;
        mod2_object.color_modulation = 0;
        let lines = HashMap::from([(
            DefinitionId::from("BentPower"),
            DefinitionLineMetadata {
                line: 1,
                line_intersect: 0,
            },
        )]);
        let mut graphics = test_graphics(12, 8, 8, "Bent typed line");
        let background = Color::opaque(10, 20, 30);
        graphics.surface_mut().fill(background);
        graphics.draw_objects(
            &[object],
            &[],
            &lines,
            &[],
            OWNER_NONE,
            1.0,
            &HashMap::new(),
            ObjectRenderPass::Normal,
            None,
        );

        let primary = Color::opaque(152, 100, 44);
        let marker = Color::opaque(168, 168, 168);
        assert_eq!(graphics.surface().get_pixel(2, 2), Some(marker));
        assert_eq!(graphics.surface().get_pixel(3, 2), Some(primary));
        assert_eq!(graphics.surface().get_pixel(4, 2), Some(primary));
        assert_eq!(
            graphics.surface().get_pixel(5, 2),
            Some(marker),
            "the next segment's start marker overwrites the L joint"
        );
        assert_eq!(graphics.surface().get_pixel(5, 3), Some(primary));
        assert_eq!(graphics.surface().get_pixel(5, 4), Some(primary));
        assert_eq!(
            graphics.surface().get_pixel(5, 5),
            Some(background),
            "the polyline's final vertex remains untouched"
        );

        draw_object_line_segment(
            graphics.surface_mut(),
            (8.0, 2.0),
            (8.0, 2.0),
            primary,
            c4_palette_color(191),
            SpriteBlitState::normal(),
            None,
            None,
        );
        assert_eq!(
            graphics.surface().get_pixel(8, 2),
            Some(Color::new(4, 9, 142, 255)),
            "zero-length GL_LINES emits no primary fragment but still receives the secondary marker"
        );

        draw_object_line_segment(
            graphics.surface_mut(),
            (8.0, 4.0),
            (11.0, 4.0),
            c4_palette_color(191),
            c4_palette_color(0),
            SpriteBlitState::normal(),
            None,
            None,
        );
        assert_eq!(
            graphics.surface().get_pixel(8, 4),
            Some(Color::new(4, 9, 142, 255)),
            "transparent palette index 0 leaves the half-blue index 191 primary pixel visible"
        );
        assert_eq!(graphics.surface().get_pixel(11, 4), Some(background));

        graphics.draw_objects(
            &[mod2_object],
            &[],
            &lines,
            &[],
            OWNER_NONE,
            1.0,
            &HashMap::new(),
            ObjectRenderPass::Normal,
            None,
        );
        assert_eq!(
            graphics.surface().get_pixel(3, 6),
            Some(Color::opaque(0, 0, 0)),
            "line primitives keep zero ColorMod activation but do not run texture MOD2 math"
        );
        assert_eq!(
            modulate_line_palette_color(primary, Some(0x00ff_ffff)),
            Color::opaque(151, 99, 43),
            "line ColorMod uses C++ >>8 channel multiplication, even for white"
        );
        assert_eq!(
            modulate_line_palette_color(c4_palette_color(191), Some(0x4080_ffff)),
            Color::new(0, 0, 254, 95),
            "palette and ColorMod transparencies screen-combine in packed C4 alpha"
        );
    }

    #[test]
    fn public_gamma_render_defers_runtime_change_until_after_current_pass() {
        // A runtime SetGamma marks fSetGamma during simulation, but
        // C4GraphicsSystem::Execute draws all viewports first and calls
        // ApplyGamma only at its tail (C4GraphicsSystem.cpp:160-199).
        let snapshot = make_snapshot();
        let mut changed = snapshot.clone();
        changed
            .environment
            .gamma
            .set_ramp(0, [0x102030, 0x405060, 0x708090]);
        let viewports = [ViewportInput::from_focus(&snapshot.objects[0])];
        let make_graphics = || {
            let mut graphics = test_graphics(320, 180, 150, "Gamma Seam");
            graphics.set_advanced_renderer_config(AdvancedRendererConfig {
                shader: true,
                ..AdvancedRendererConfig::DEFAULT
            });
            graphics
        };
        let mut public = make_graphics();
        public.render_frame(&snapshot, &viewports);
        public.render_frame(&changed, &viewports);
        let before_apply = public.surface().pixels().to_vec();
        public.render_frame(&changed, &viewports);
        let after_apply = public.surface().pixels().to_vec();

        let standard = clonk_graphics::GammaRamp::from_control_points(
            snapshot.environment.gamma.combined_control_points(),
        );
        let changed_ramp = clonk_graphics::GammaRamp::from_control_points(
            changed.environment.gamma.combined_control_points(),
        );
        let mut standard_render = make_graphics();
        standard_render.render_frame_with_gamma(&changed, &viewports, Some(&standard));
        let mut changed_render = make_graphics();
        changed_render.render_frame_with_gamma(&changed, &viewports, Some(&changed_ramp));

        assert_eq!(before_apply, standard_render.surface().pixels());
        assert_eq!(after_apply, changed_render.surface().pixels());
        assert_ne!(before_apply, after_apply);
    }

    #[test]
    fn ordered_frame_phases_match_the_single_call_frame_and_atlas() {
        let snapshot = make_snapshot();
        let viewports = [ViewportInput::from_focus(&snapshot.objects[0])];
        let board_color = Color::opaque(36, 72, 144);
        let board = ImageData::new(
            4,
            50,
            (0..4 * 50)
                .flat_map(|_| [board_color.r, board_color.g, board_color.b, board_color.a])
                .collect(),
        );
        let make_graphics = || {
            GraphicsSystem::new(
                128,
                120,
                120,
                "Split frame",
                test_font(),
                empty_sprites(),
                empty_cursor_atlas(),
                Arc::new(HudGraphics {
                    upper_board: Some(board.clone()),
                    ..HudGraphics::default()
                }),
            )
        };

        let mut single = make_graphics();
        let single_atlas = single.render_frame(&snapshot, &viewports);
        let single_pixels = single.surface().pixels().to_vec();

        let mut split = make_graphics();
        let pending = split.render_frame_base(&snapshot, &viewports);
        split.render_frame_foreground(&pending);
        let pending = split.render_frame_hud_players(pending);
        let split_atlas = split.render_frame_hud_chrome(pending);

        assert_eq!(split.surface().pixels(), single_pixels);
        assert_eq!(split_atlas, single_atlas);
        assert_eq!(
            split.active_gamma_control_points,
            single.active_gamma_control_points
        );
    }

    /// A GPU scene-capture frame records commands instead of rasterizing, so
    /// the per-viewport content surface `render_viewport` allocates every frame
    /// is never read or written. Deferring the pixel plane must therefore cost
    /// a steady-state frame zero materializations; each avoided 640x480 plane
    /// is 1.23 MB of allocate-and-zero, roughly 0.5 ms of Raspberry Pi 4
    /// memory bandwidth.
    #[test]
    fn gpu_capture_frames_materialize_no_viewport_pixel_planes() {
        const WIDTH: u32 = 640;
        const HEIGHT: u32 = 480;
        const FRAMES: u64 = 60;

        let mut snapshot = make_snapshot();
        // A world larger than the viewport removes the letterbox borders, so
        // the content surface is the full viewport the shipping game renders.
        snapshot.landscape = Some(Landscape::flat(WIDTH * 2, HEIGHT as i32 * 2));
        let viewports = [ViewportInput::from_focus(&snapshot.objects[0])];
        let mut graphics = GraphicsSystem::new(
            WIDTH,
            HEIGHT,
            120,
            "Deferred pixel plane probe",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        let gamma = clonk_graphics::GammaRamp::identity();
        let capture_frame = |graphics: &mut GraphicsSystem| {
            graphics.begin_gpu_scene_capture();
            graphics.render_frame_without_atlas_deferred_monitor_gamma(&snapshot, &viewports);
            graphics.finish_gpu_scene_capture(&gamma)
        };
        // Warm the retained caches so the measured window is steady state.
        assert!(capture_frame(&mut graphics).is_some());

        let before = clonk_graphics::pixel_plane_stats();
        let start = std::time::Instant::now();
        for _ in 0..FRAMES {
            capture_frame(&mut graphics);
        }
        let elapsed = start.elapsed();
        let stats = clonk_graphics::pixel_plane_stats();

        // The work the deferral removes, timed on this machine: allocating,
        // zeroing and freeing one viewport-sized plane per frame is exactly
        // what `Surface::new` used to do unconditionally.
        let start = std::time::Instant::now();
        for _ in 0..FRAMES {
            let surface = Surface::new(WIDTH, HEIGHT, clonk_graphics::PixelFormat::Rgba8888);
            std::hint::black_box(surface.pixels()[0]);
        }
        let eager_plane = start.elapsed();

        let deferred_bytes = stats.deferred_bytes - before.deferred_bytes;
        let materialized = stats.materialized - before.materialized;
        let materialized_bytes = stats.materialized_bytes - before.materialized_bytes;
        println!(
            "{FRAMES} capture frames at {WIDTH}x{HEIGHT}: {:.3} ms/frame; \
             {} deferred plane bytes/frame, {materialized} materializations \
             ({materialized_bytes} bytes); the removed allocate+zero+free costs \
             {:.3} ms/frame here",
            elapsed.as_secs_f64() * 1000.0 / FRAMES as f64,
            deferred_bytes / FRAMES,
            eager_plane.as_secs_f64() * 1000.0 / FRAMES as f64,
        );
        assert_eq!(
            materialized, 0,
            "a scene-capture frame rasterized into a deferred pixel plane"
        );
        assert!(
            deferred_bytes / FRAMES >= u64::from(WIDTH * HEIGHT * 4),
            "expected at least one full viewport plane deferred per frame, got {} bytes",
            deferred_bytes / FRAMES
        );
    }

    #[test]
    fn no_atlas_completions_match_snapshot_render_state() {
        let snapshot = make_snapshot();
        let viewports = [ViewportInput::from_focus(&snapshot.objects[0])];
        let make_graphics = || test_graphics(128, 120, 120, "No-atlas frame");

        let mut snapshot_render = make_graphics();
        let atlas = snapshot_render.render_frame(&snapshot, &viewports);
        assert!(!atlas.is_empty());

        let mut direct = make_graphics();
        direct.render_frame_without_atlas(&snapshot, &viewports);
        assert_eq!(
            direct.surface().pixels(),
            snapshot_render.surface().pixels()
        );
        assert_eq!(
            direct.active_gamma_control_points,
            snapshot_render.active_gamma_control_points
        );

        let mut ordered = make_graphics();
        let pending = ordered.render_frame_base(&snapshot, &viewports);
        ordered.render_frame_foreground(&pending);
        let pending = ordered.render_frame_hud_players(pending);
        ordered.render_frame_hud_chrome_without_atlas(pending);
        assert_eq!(
            ordered.surface().pixels(),
            snapshot_render.surface().pixels()
        );
        assert_eq!(
            ordered.active_gamma_control_points,
            snapshot_render.active_gamma_control_points
        );
    }

    #[test]
    fn hud_chrome_phase_keeps_captured_gamma_after_transparent_seams() {
        let snapshot = make_snapshot();
        let mut changed = snapshot.clone();
        changed
            .environment
            .gamma
            .set_ramp(0, [0x102030, 0x405060, 0x708090]);
        let board_color = Color::opaque(48, 96, 192);
        let board = ImageData::new(
            4,
            50,
            (0..4 * 50)
                .flat_map(|_| [board_color.r, board_color.g, board_color.b, board_color.a])
                .collect(),
        );
        let mut graphics = GraphicsSystem::new(
            128,
            120,
            120,
            "Transparent HUD",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            Arc::new(HudGraphics {
                upper_board: Some(board),
                ..HudGraphics::default()
            }),
        );
        let initial_viewports = [ViewportInput::from_focus(&snapshot.objects[0])];
        graphics.render_frame(&snapshot, &initial_viewports);
        let installed_before = snapshot.environment.gamma.combined_control_points();

        let changed_viewports = [ViewportInput::from_focus(&changed.objects[0])];
        let pending = graphics.render_frame_base(&changed, &changed_viewports);
        assert_eq!(graphics.active_gamma_control_points, Some(installed_before));
        graphics.surface_mut().fill(Color::transparent());
        graphics.render_frame_foreground(&pending);
        graphics.surface_mut().fill(Color::transparent());
        let pending = graphics.render_frame_hud_players(pending);
        graphics.surface_mut().fill(Color::transparent());
        let atlas = graphics.render_frame_hud_chrome(pending);

        assert!(!atlas.is_empty());
        assert_eq!(
            graphics.surface().get_pixel(0, 0),
            Some(standard_gamma_color(board_color))
        );
        assert_eq!(
            graphics.active_gamma_control_points,
            Some(changed.environment.gamma.combined_control_points())
        );
    }

    #[test]
    fn tiled_viewport_background_gamma_encodes_raw_translucent_texels() {
        // The back-buffer and small-world border use fctBackground through
        // BlitSurfaceTile (C4GraphicsSystem.cpp:290; C4Viewport.cpp:1033-1036).
        let image = ImageData::new(1, 1, vec![64, 128, 192, 128]);
        let gamma = clonk_graphics::GammaRamp::from_control_points([0x000000, 0x646464, 0xc8c8c8]);
        let render = |gamma: Option<&clonk_graphics::GammaRamp>| {
            let mut surface = Surface::new(3, 2, PixelFormat::Rgba8888);
            surface.fill(Color::opaque(7, 11, 13));
            tile_image_on_surface(&mut surface, &image, 0, 0, gamma);
            surface.pixels().to_vec()
        };

        assert!(render(Some(&gamma))
            .chunks_exact(4)
            .all(|pixel| pixel == [50, 100, 150, 128]));
        assert!(render(None)
            .chunks_exact(4)
            .all(|pixel| pixel == [64, 128, 192, 128]));
    }

    #[test]
    fn tiled_underlay_cache_matches_uncached_pixels_and_reuses_exact_key() {
        let image = ImageData::new(
            3,
            2,
            vec![
                4, 32, 160, 255, 48, 96, 144, 192, 200, 120, 40, 128, 12, 64, 192, 96, 88, 136,
                184, 64, 240, 176, 112, 32,
            ],
        );
        let gamma = clonk_graphics::GammaRamp::from_control_points([0x102030, 0x506070, 0xa0b0c0]);
        let mut expected = Surface::new(7, 5, PixelFormat::Rgba8888);
        tile_image_on_surface(&mut expected, &image, -2, 3, Some(&gamma));

        let mut cache = TiledUnderlayCache::default();
        let mut cached = Surface::new(7, 5, PixelFormat::Rgba8888);
        cache.begin_frame(&cached, Some(&image), Some(&gamma));
        cache.draw(&mut cached, &image, -2, 3, Some(&gamma));
        assert_eq!(cached.pixels(), expected.pixels());
        assert_eq!(cache.rasterizations, 1);

        cached.fill(Color::opaque(1, 2, 3));
        cache.draw(&mut cached, &image, -2, 3, Some(&gamma));
        assert_eq!(cached.pixels(), expected.pixels());
        assert_eq!(
            cache.rasterizations, 1,
            "an identical frame key must restore the retained backing without retiling"
        );
    }

    #[test]
    fn tiled_underlay_cache_invalidates_all_pixel_inputs() {
        let image_a = ImageData::new(2, 1, vec![24, 72, 120, 255, 200, 160, 80, 128]);
        let image_b = ImageData::new(2, 1, vec![220, 40, 100, 64, 12, 180, 240, 192]);
        let gamma_a = clonk_graphics::GammaRamp::standard();
        let gamma_b =
            clonk_graphics::GammaRamp::from_control_points([0x081018, 0x405060, 0x90a0b0]);
        let mut cache = TiledUnderlayCache::default();

        let mut surface = Surface::new(3, 2, PixelFormat::Rgba8888);
        cache.begin_frame(&surface, Some(&image_a), Some(&gamma_a));
        cache.draw(&mut surface, &image_a, 0, 0, Some(&gamma_a));
        assert_eq!(cache.rasterizations, 1);

        let mut resized = Surface::new(4, 2, PixelFormat::Rgba8888);
        cache.begin_frame(&resized, Some(&image_a), Some(&gamma_a));
        assert!(
            cache.entries.is_empty(),
            "an output resize drops old backings"
        );
        cache.draw(&mut resized, &image_a, 0, 0, Some(&gamma_a));
        assert_eq!(cache.rasterizations, 2);

        cache.begin_frame(&resized, Some(&image_b), Some(&gamma_a));
        assert!(
            cache.entries.is_empty(),
            "replacing the HUD background drops old-image backings"
        );
        cache.draw(&mut resized, &image_b, 0, 0, Some(&gamma_a));
        let mut expected = Surface::new(4, 2, PixelFormat::Rgba8888);
        tile_image_on_surface(&mut expected, &image_b, 0, 0, Some(&gamma_a));
        assert_eq!(resized.pixels(), expected.pixels());
        assert_eq!(cache.rasterizations, 3);

        cache.begin_frame(&resized, Some(&image_b), Some(&gamma_b));
        assert!(
            cache.entries.is_empty(),
            "a new gamma ramp drops old pixels"
        );
        cache.draw(&mut resized, &image_b, 0, 0, Some(&gamma_b));
        tile_image_on_surface(&mut expected, &image_b, 0, 0, Some(&gamma_b));
        assert_eq!(resized.pixels(), expected.pixels());
        assert_eq!(cache.rasterizations, 4);

        let origin_zero = resized.pixels().to_vec();
        cache.draw(&mut resized, &image_b, 1, -1, Some(&gamma_b));
        tile_image_on_surface(&mut expected, &image_b, 1, -1, Some(&gamma_b));
        assert_eq!(resized.pixels(), expected.pixels());
        assert_ne!(resized.pixels(), origin_zero);
        assert_eq!(cache.rasterizations, 5);

        cache.draw(&mut resized, &image_b, 1, -1, Some(&gamma_b));
        assert_eq!(resized.pixels(), expected.pixels());
        assert_eq!(
            cache.rasterizations, 5,
            "the distinct origin becomes independently reusable"
        );
    }

    #[test]
    fn configured_tiled_underlay_crops_before_indent_and_invalidates_snapshot_cache() {
        let row = [0, 0, 0, 255, 64, 0, 0, 255, 128, 0, 0, 255, 255, 0, 0, 255];
        let image = ImageData::new(4, 4, row.repeat(4));
        let mut cache = TiledUnderlayCache::default();
        let mut surface = Surface::new(2, 1, PixelFormat::Rgba8888);

        let _indent = activate_advanced_renderer_config(AdvancedRendererConfig {
            tex_indent: 1000,
            ..AdvancedRendererConfig::DEFAULT
        });
        cache.begin_frame(&surface, Some(&image), None);
        cache.draw(&mut surface, &image, 2, 0, None);
        assert_eq!(
            surface.get_pixel(0, 0),
            Some(Color::opaque(255, 0, 0)),
            "the visible source crop, not the offscreen tile edge, anchors TexIndent",
        );
        assert_eq!(cache.rasterizations, 1);

        let _shift = activate_advanced_renderer_config(AdvancedRendererConfig {
            blit_offset: 100,
            ..AdvancedRendererConfig::DEFAULT
        });
        cache.begin_frame(&surface, Some(&image), None);
        assert!(
            cache.entries.is_empty(),
            "replacing the immutable device snapshot drops cached underlay pixels",
        );
        cache.draw(&mut surface, &image, 2, 0, None);
        assert_eq!(cache.rasterizations, 2);
    }

    #[test]
    fn borderless_viewport_direct_presentation_matches_scratch_composition() {
        let rect = SurfaceRect::new(2, 1, 3, 2);
        let content = Surface::from_bytes(
            rect.width,
            rect.height,
            PixelFormat::Rgba8888,
            vec![
                1, 2, 3, 0, 4, 5, 6, 64, 7, 8, 9, 255, 10, 11, 12, 127, 13, 14, 15, 192, 16, 17,
                18, 255,
            ],
        )
        .expect("valid content bytes");
        let destination_bytes = (0..7 * 5)
            .flat_map(|index| {
                let value = index as u8;
                [value, value.wrapping_add(1), value.wrapping_add(2), 255]
            })
            .collect::<Vec<_>>();
        let mut scratch_path =
            Surface::from_bytes(7, 5, PixelFormat::Rgba8888, destination_bytes.clone())
                .expect("valid destination bytes");
        let mut direct_path = Surface::from_bytes(7, 5, PixelFormat::Rgba8888, destination_bytes)
            .expect("valid destination bytes");
        let mut viewport_underlay = Surface::new(rect.width, rect.height, PixelFormat::Rgba8888);
        viewport_underlay.fill(Color::opaque(91, 73, 55));

        present_viewport_content(
            &mut scratch_path,
            Some(&mut viewport_underlay),
            &content,
            rect,
            0,
            0,
        );
        present_viewport_content(&mut direct_path, None, &content, rect, 0, 0);

        assert_eq!(direct_path.pixels(), scratch_path.pixels());
    }

    #[test]
    fn gamma_render_seam_encodes_sky_channels_independently() {
        // C4Sky::Draw emits its solid/fade colours through DrawBoxDw/Fade;
        // DummyShader samples three independent gamma textures before output
        // (C4Sky.cpp:206-225; StdGL.cpp:1185-1200).
        let mut graphics = test_graphics(1, 1, 1, "Gamma Sky");
        let environment = EnvironmentFrame {
            sky_color: Some(RgbColor::new(0, 0, 0)),
            ..EnvironmentFrame::default()
        };
        let gamma = clonk_graphics::GammaRamp::from_control_points([0x102030, 0x405060, 0x708090]);

        graphics.draw_sky(None, &environment, &[], &[], &[], 1.0, Some(&gamma));

        assert_eq!(
            graphics.surface().get_pixel(0, 0),
            Some(Color::new(17, 33, 49, 255))
        );
    }

    #[test]
    fn sky_render_state_caches_only_complete_opaque_rgba_images() {
        let settings = SkySettings::default().with_surface(2, 1);
        assert!(SkyRenderState::new(
            settings.clone(),
            Some(ImageData::new(2, 1, vec![1, 2, 3, 255, 4, 5, 6, 255])),
        )
        .image_is_fully_opaque());
        assert!(!SkyRenderState::new(
            settings.clone(),
            Some(ImageData::new(2, 1, vec![1, 2, 3, 255, 4, 5, 6, 254])),
        )
        .image_is_fully_opaque());
        assert!(
            !SkyRenderState::new(settings, Some(ImageData::new(2, 1, vec![1, 2, 3, 255])))
                .image_is_fully_opaque()
        );
        assert!(!SkyRenderState::new(
            SkySettings::default().with_surface(0, 0),
            Some(ImageData::new(0, 0, Vec::new())),
        )
        .image_is_fully_opaque());
    }

    #[test]
    fn gamma_render_seam_encodes_tutorial_six_sky_gradient() {
        // DrawBoxFade interpolation is gamma sampled per fragment before the
        // framebuffer store (C4Sky.cpp:219-225; StdGL.cpp:846-889,1193-1200).
        let mut graphics = test_graphics(1, 1, 1, "Gamma Gradient");
        let gamma = clonk_graphics::GammaRamp::from_control_points([0x000000, 0x646464, 0xc8c8c8]);

        graphics.fill_vertical_gradient(
            Color::opaque(64, 128, 192),
            Color::opaque(64, 128, 192),
            1.0,
            Some(&gamma),
        );

        assert_eq!(
            graphics.surface().get_pixel(0, 0),
            Some(Color::new(50, 100, 150, 255))
        );
    }

    #[test]
    fn sky_dither_marks_a_real_gradient_only_when_enabled() {
        // The shipped default sky fade RGB(28,64,152)->RGB(192,196,252) spans
        // 100 blue steps, so on a 2160-row viewport 8-bit interpolation lands
        // a visible band every ~22 rows. The dither is sub-LSB noise, but it
        // still moves bytes away from C++, so it stays opt-in — and a flat
        // fill has no banding to hide.
        let gamma = clonk_graphics::GammaRamp::from_control_points([0x000000, 0x646464, 0xc8c8c8]);
        let dithered = |enabled: bool, top: Color, bottom: Color| {
            let mut graphics = test_graphics(8, 6, 12, "sky dither");
            graphics.set_sky_dither(enabled);
            graphics.begin_gpu_scene_capture();
            graphics.fill_vertical_gradient(top, bottom, 1.0, Some(&gamma));
            let scene = graphics
                .finish_gpu_scene_capture(&gamma)
                .expect("GPU capture remains active");
            let GpuCommand::Solid { style, .. } = &scene.commands[0] else {
                panic!("sky gradient did not lower to solid triangles");
            };
            style.dither
        };

        let top = Color::opaque(28, 64, 152);
        let bottom = Color::opaque(192, 196, 252);
        assert!(
            dithered(true, top, bottom),
            "an enabled gradient sky must ask for dithering"
        );
        assert!(
            !dithered(false, top, bottom),
            "the C++ byte-exact gradient stays the default"
        );
        assert!(
            !dithered(true, top, top),
            "a flat sky has no interpolation to dither"
        );
    }

    #[test]
    fn gpu_capture_lowers_sky_gradient_to_one_gamma_solid_draw() {
        let mut graphics = test_graphics(8, 6, 12, "GPU Gradient");
        let gamma = clonk_graphics::GammaRamp::from_control_points([0x000000, 0x646464, 0xc8c8c8]);
        graphics.begin_gpu_scene_capture();
        graphics.fill_vertical_gradient(
            Color::opaque(28, 64, 152),
            Color::opaque(192, 196, 252),
            1.0,
            Some(&gamma),
        );
        let scene = graphics
            .finish_gpu_scene_capture(&gamma)
            .expect("GPU capture remains active");

        assert_eq!(scene.commands.len(), 1);
        let GpuCommand::Solid {
            vertices,
            topology,
            blend,
            style,
            ..
        } = &scene.commands[0]
        else {
            panic!("sky gradient did not lower to solid triangles");
        };
        assert_eq!(*topology, GpuPrimitiveTopology::TriangleList);
        assert_eq!(*blend, GpuBlend::Normal);
        assert!(style.gamma);
        assert_eq!(
            vertices.len(),
            6,
            "gradient must not emit one point per pixel"
        );
    }

    #[test]
    fn advanced_lit_sky_keeps_one_texture_identity_and_revises_pixels() {
        let mut graphics = test_graphics(8, 6, 12, "retained advanced sky");
        let image = ImageData::new(2, 2, [120, 80, 40, 255].repeat(4));

        let (_, initial) = graphics.retained_lit_sky_texture(&image, 1.0);
        let (_, unchanged) = graphics.retained_lit_sky_texture(&image, 1.0);
        assert_eq!(initial.id, unchanged.id);
        assert_eq!(initial.revision, unchanged.revision);
        assert!(unchanged.dirty.is_empty());

        let (_, darkened) = graphics.retained_lit_sky_texture(&image, 0.5);
        assert_eq!(initial.id, darkened.id);
        assert_eq!(darkened.revision, initial.revision + 1);
        assert_eq!(darkened.base_revision, Some(initial.revision));
        assert_eq!(darkened.dirty, vec![clonk_graphics::Rect::new(0, 0, 2, 2)]);
        assert_ne!(darkened.pixels, initial.pixels);

        let (_, repeated) = graphics.retained_lit_sky_texture(&image, 0.5);
        assert_eq!(repeated.id, darkened.id);
        assert_eq!(repeated.revision, darkened.revision);
        assert!(repeated.dirty.is_empty());
    }

    #[test]
    fn gpu_capture_sections_fogged_solid_sky_into_gamma_quads() {
        let mut graphics = test_graphics(130, 70, 70, "GPU fogged solid sky");
        graphics.active_fog_map = Some(Arc::new(
            ClrModMap::reset(64, 64, 130, 70, 0, 0, 0, 0, 0).expect("valid fog map"),
        ));
        let gamma = clonk_graphics::GammaRamp::standard();
        graphics.begin_gpu_scene_capture();

        graphics.fill_world_color(Color::opaque(153, 141, 255), false, Some(&gamma));

        let scene = graphics
            .finish_gpu_scene_capture(&gamma)
            .expect("GPU capture remains active");
        assert_eq!(scene.commands.len(), 1);
        for command in &scene.commands {
            let GpuCommand::Solid {
                vertices,
                topology,
                blend,
                style,
                ..
            } = command
            else {
                panic!("fogged sky did not lower to solid triangles");
            };
            assert_eq!(*topology, GpuPrimitiveTopology::TriangleList);
            assert_eq!(*blend, GpuBlend::Normal);
            assert!(style.gamma);
            assert_eq!(
                vertices.len(),
                36,
                "three 64px fog columns by two rows become six merged GPU quads"
            );
        }
    }

    #[test]
    fn gamma_render_seam_encodes_sky_image_before_alpha_blending() {
        // C4Sky::Draw sends its tiled surface through BlitSurfaceTile2, whose
        // shader gamma-samples the source before blending (C4Sky.cpp:210-218;
        // StdGL.cpp:1068-1087).
        let mut graphics = test_graphics(1, 1, 1, "Gamma Sky Image");
        graphics
            .surface_mut()
            .set_pixel(0, 0, Color::opaque(200, 200, 200))
            .expect("background pixel");
        let image = ImageData::new(1, 1, vec![64, 128, 192, 128]);
        let gamma = clonk_graphics::GammaRamp::from_control_points([0x000000, 0x646464, 0xc8c8c8]);

        graphics.blit_sky_tile(&image, 0, 0, None, 1.0, Some(&gamma));

        assert_eq!(
            graphics.surface().get_pixel(0, 0),
            Some(Color::new(125, 150, 175, 255))
        );
    }

    #[test]
    fn gamma_render_seam_encodes_fallback_landscape_fragments() {
        // The fallback painter stands in for the same landscape presentation
        // shader. Even black is sampled through MinGamma, yielding one rather
        // than a raw zero (StdGL.cpp:1139-1148; StdDDraw2.cpp:237-271).
        let mut graphics = test_graphics(1, 1, 0, "Gamma Fallback Ground");
        let gamma = clonk_graphics::GammaRamp::from_control_points([0x000000, 0x646464, 0xc8c8c8]);

        assert!(!graphics.draw_ground(0, None, 0.0, Some(&gamma)));

        assert_eq!(
            graphics.surface().get_pixel(0, 0),
            Some(Color::new(1, 1, 1, 255))
        );
    }

    #[test]
    fn gpu_column_landscape_fallback_matches_cpu_reference_pixels() {
        let mut landscape = Landscape::flat(4, 2);
        landscape.set_liquid_column(1, vec![clonk_engine::landscape::LiquidSegment::new(0, 2)]);
        let make_graphics = || test_graphics(4, 4, 2, "Column fallback");

        let mut cpu = make_graphics();
        assert!(!cpu.draw_ground(0, Some(&landscape), 1.0, None));
        cpu.draw_liquids(0, Some(&landscape), 1.0, None);
        let expected = cpu.surface().pixels().to_vec();

        let mut gpu = make_graphics();
        gpu.begin_gpu_scene_capture();
        assert!(gpu.draw_ground(0, Some(&landscape), 1.0, None));
        let scene = gpu
            .finish_gpu_scene_capture(&clonk_graphics::GammaRamp::standard())
            .expect("column landscape capture remains active");
        assert_eq!(scene.commands.len(), 2, "ground precedes the liquid pass");
        assert_eq!(scene.textures.len(), 2);

        let mut replay = Surface::new(4, 4, PixelFormat::Rgba8888);
        replay.fill(scene.clear);
        for command in &scene.commands {
            let GpuCommand::Quad {
                texture,
                sampler,
                blend,
                gamma,
                ..
            } = command
            else {
                panic!("column fallback must lower to retained source quads");
            };
            assert_eq!(*sampler, GpuSampler::Nearest);
            assert_eq!(*blend, GpuBlend::Normal);
            assert!(!*gamma);
            let resource = scene
                .textures
                .iter()
                .find(|resource| resource.id == *texture)
                .expect("quad resource is retained");
            draw_image(
                &mut replay,
                &GuiRect::new(0.0, 0.0, 4.0, 4.0),
                &ImageData::transient_from_arc(4, 4, Arc::clone(&resource.pixels)),
            );
        }
        assert_eq!(replay.pixels(), expected);

        let first_ids = scene
            .textures
            .iter()
            .map(|resource| resource.id)
            .collect::<Vec<_>>();
        // A production frame drops its CPU resource snapshot after command
        // submission. Do the same before mutating the retained source again;
        // keeping the snapshot alive intentionally triggers Surface COW and
        // therefore a new resource identity.
        drop(scene);
        gpu.begin_gpu_scene_capture();
        assert!(gpu.draw_ground(0, Some(&landscape), 1.0, None));
        let second = gpu
            .finish_gpu_scene_capture(&clonk_graphics::GammaRamp::standard())
            .expect("second column landscape capture remains active");
        assert_eq!(
            second
                .textures
                .iter()
                .map(|resource| resource.id)
                .collect::<Vec<_>>(),
            first_ids,
            "column source textures persist across frames"
        );
        assert!(second
            .textures
            .iter()
            .all(|resource| resource.base_revision.is_some() && !resource.dirty.is_empty()));
    }

    #[test]
    fn full_rgba_owner_overlay_keeps_color_and_composites_over_the_base() {
        let render = |base: Color, overlay: Color, owner_color: u32| {
            let fragment = prepare_sprite_fragment(
                base,
                Some(ColorByOwnerSample::Overlay(overlay)),
                Some(owner_color),
                SpriteBlitState::normal(),
            );
            composite_sprite_fragment(
                fragment,
                Color::opaque(17, 23, 31),
                SpriteBlitState::normal(),
                None,
            )
        };

        assert_eq!(
            render(
                Color::opaque(0, 255, 0),
                Color::new(128, 128, 128, 128),
                0x00ff_0000,
            ),
            Color::opaque(64, 127, 0),
            "a half-alpha owner pass blends over, rather than replacing, green base"
        );
        assert_eq!(
            render(
                Color::opaque(20, 30, 40),
                Color::opaque(0, 0, 255),
                0x00ff_ffff,
            ),
            Color::opaque(0, 0, 255),
            "colored Overlay.png channels survive white-owner modulation"
        );
        assert_eq!(
            render(
                Color::opaque(20, 30, 40),
                Color::opaque(0, 0, 255),
                0x00ff_0000,
            ),
            Color::opaque(0, 0, 0),
            "an opaque red-zero overlay still covers the base after modulation"
        );
        assert_eq!(
            render(
                Color::opaque(20, 30, 40),
                Color::new(200, 150, 100, 0),
                0x00ff_0000,
            ),
            Color::opaque(20, 30, 40),
            "transparent overlay RGB never changes the base pass"
        );
    }

    #[test]
    fn shipped_knight_walk_crop_matches_two_pass_owner_overlay_oracle() {
        let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../content/Knights.c4d/Crew.c4d/Knight.c4d");
        assert!(
            directory.is_dir(),
            "the initialized official content submodule must provide {}",
            directory.display()
        );
        let group = Group::open(&directory).expect("open Knight definition");
        let resource =
            clonk_resources::ResourceDefinition::load(&group).expect("load Knight resources");
        let mut engine = Engine::new();
        engine
            .register_definition(
                clonk_engine::Definition::from_resource(&resource)
                    .expect("compile Knight definition"),
            )
            .expect("register Knight definition");
        let image = engine
            .definition_sprite_image("KNIG", None)
            .expect("Knight sprite image");
        let width = image.width();
        let height = image.height();
        let overlay = image.color_mask().expect("Knight Overlay.png");
        assert_eq!(overlay.len(), width as usize * height as usize * 4);

        let mut raw_base =
            image::load_from_memory(&group.read_file("Graphics.png").expect("read Graphics.png"))
                .expect("decode Graphics.png")
                .into_rgba8();
        let mut raw_overlay =
            image::load_from_memory(&group.read_file("Overlay.png").expect("read Overlay.png"))
                .expect("decode Overlay.png")
                .into_rgba8();
        for image in [&mut raw_base, &mut raw_overlay] {
            for pixel in image.pixels_mut() {
                if pixel[3] == 0 {
                    pixel[0] = 0;
                    pixel[1] = 0;
                    pixel[2] = 0;
                }
            }
        }
        assert_eq!(image.pixels().as_ref(), raw_base.as_raw());
        assert_eq!(overlay.as_ref(), raw_overlay.as_raw());

        // Find a 16x20 window that actually contains antialiased owner-color
        // edges rather than assuming they sit at the sheet origin. The crop is
        // only a fixture for the two-pass blend, and where the interesting
        // texels land moved when the crew art was re-rendered at DefCore
        // `Scale=300` — the top-left corner of a high-resolution sheet is
        // inside one frame's empty margin.
        let (crop_x, crop_y) = (0..height.saturating_sub(20))
            .flat_map(|y| (0..width.saturating_sub(16)).map(move |x| (x, y)))
            .find(|&(ox, oy)| {
                (0..20).any(|y| {
                    (0..16).any(|x| (1..=254).contains(&raw_overlay.get_pixel(ox + x, oy + y).0[3]))
                })
            })
            .expect("the shipped sheet must contain antialiased owner-color edges somewhere");

        let mut rendered = Surface::new(16, 20, PixelFormat::Rgba8888);
        rendered.fill(Color::opaque(0, 0, 0));
        draw_image_region(
            &mut rendered,
            &GuiRect::new(0.0, 0.0, 16.0, 20.0),
            &ImageData::from_arc(width, height, image.into_pixels()),
            Some(&ColorByOwnerMask::new(width, height, overlay)),
            &SourceRect::new(crop_x as i32, crop_y as i32, 16, 20),
            false,
            Some(0x00ff_0000),
            SpriteBlitState::normal(),
            None,
            None,
        );

        let mut partial_overlay_pixels = 0;
        for y in 0..20 {
            for x in 0..16 {
                let base = raw_base.get_pixel(crop_x + x, crop_y + y).0;
                let owner = raw_overlay.get_pixel(crop_x + x, crop_y + y).0;
                partial_overlay_pixels += usize::from((1..=254).contains(&owner[3]));
                let base_alpha = u16::from(base[3]);
                let base_rgb = [
                    (u16::from(base[0]) * base_alpha / 255) as u8,
                    (u16::from(base[1]) * base_alpha / 255) as u8,
                    (u16::from(base[2]) * base_alpha / 255) as u8,
                ];
                let overlay_alpha = f32::from(owner[3]) / 255.0;
                let expected = Color::new(
                    (f32::from(owner[0]) * overlay_alpha
                        + f32::from(base_rgb[0]) * (1.0 - overlay_alpha))
                        .round() as u8,
                    (f32::from(base_rgb[1]) * (1.0 - overlay_alpha)).round() as u8,
                    (f32::from(base_rgb[2]) * (1.0 - overlay_alpha)).round() as u8,
                    255,
                );
                let actual = rendered.get_pixel(x, y).expect("rendered crop pixel");
                for (actual, expected) in [actual.r, actual.g, actual.b, actual.a]
                    .into_iter()
                    .zip([expected.r, expected.g, expected.b, expected.a])
                {
                    assert!(
                        actual.abs_diff(expected) <= 1,
                        "Knight Walk crop mismatch at ({x},{y}): {actual} vs {expected}"
                    );
                }
            }
        }
        assert!(
            partial_overlay_pixels > 0,
            "the reference crop must exercise antialiased owner-color edges"
        );
    }

    #[test]
    fn color_by_owner_uses_object_color_instead_of_owner_lookup() {
        // C4Object::DrawFace passes the live C4Object::Color to GetBitmap
        // (C4Object.cpp:440-477). This may differ from the current player
        // color after SetColorDw, and unowned FISH explicitly sets white in
        // Birth (Objects.c4d/Animals.c4d/Fish.c4d/Script.c:233-240).
        let sprite = DefinitionSprite {
            graphics_scale: 1.0,
            // CreateColorByOwner clears owner-only base pixels to black.
            image: ImageData::new(1, 1, vec![0, 0, 0, 255]),
            actions: HashMap::new(),
            color_mask: Some(ColorByOwnerMask::new(1, 1, Arc::from([128]))),
            shape: Some(DefinitionRect::new(0, 0, 1, 1)),
            fire_top: 0,
            rotateable: 0,
            line: 0,
            stretch_growth: false,
            top_face: None,
            picture: None,
        };
        let mut recolored = make_snapshot().objects.remove(0);
        recolored.definition_id = "ObjectColor".to_string();
        recolored.position = Vector2::new(1, 1);
        recolored.owner = 7;
        recolored.color = 0x00ff_0000;
        recolored.crew_member = false;

        let mut fish = recolored.clone();
        fish.id = ObjectId::new(2);
        fish.position = Vector2::new(2, 1);
        fish.owner = OWNER_NONE;
        fish.color = 0x00ff_ffff;

        let mut graphics = GraphicsSystem::new(
            4,
            3,
            3,
            "Object ColorByOwner",
            test_font(),
            Arc::new(HashMap::from([(
                sprite_map_key("ObjectColor", None),
                sprite,
            )])),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics.surface_mut().fill(Color::opaque(0, 0, 0));
        graphics.draw_objects(
            &[recolored, fish],
            &[],
            &HashMap::new(),
            &[],
            OWNER_NONE,
            1.0,
            &HashMap::from([(7, Color::opaque(0, 0, 255))]),
            ObjectRenderPass::Normal,
            None,
        );

        assert_eq!(
            graphics.surface().get_pixel(1, 1),
            Some(Color::opaque(128, 0, 0)),
            "SetColorDw red must win over the owner's blue player color"
        );
        assert_eq!(
            graphics.surface().get_pixel(2, 1),
            Some(Color::opaque(128, 128, 128)),
            "an unowned white FISH must not expose its cleared black base"
        );
    }

    #[test]
    fn real_fish_overlay_uses_its_live_object_color_without_an_owner() {
        // Shipped FISH is ColorByOwner, but Birth sets an unowned fish's live
        // C4Object::Color to white (Fish.c4d/Script.c:233-240). DrawFace passes
        // that Color to GetBitmap (C4Object.cpp:438-475); it never looks up a
        // player color. Exercise the actual Graphics.png + Overlay.png split.
        let mut engine = load_repository_tutorial(9);
        let fish_id = engine
            .spawn_object(
                SpawnConfig::new("FISH")
                    .with_position(Vector2::new(200, 100))
                    .with_direction(Direction::Left),
            )
            .expect("real unowned FISH spawns");
        let first_snapshot = engine.snapshot();
        let fish = first_snapshot
            .object(fish_id)
            .expect("spawned FISH is present");
        assert_eq!(fish.owner, OWNER_NONE);
        assert_eq!(
            fish.color, 0x00ff_ffff,
            "Birth applies the shipped white tint"
        );
        assert_eq!(fish.action.name, "Swim");
        assert_eq!(fish.action.phase, 0);

        let image = engine
            .definition_sprite_image("FISH", None)
            .expect("FISH loads its real Graphics.png and Overlay.png");
        let width = image.width();
        let height = image.height();
        assert_eq!((width, height), (448, 64));
        let mask = image
            .color_mask()
            .expect("FISH has its real owner-color mask");
        // Swim phase zero starts at (0,12). Its opaque body pixel at local
        // (7,7) is grey 147 in Overlay.png and transparent in Graphics.png.
        let body_mask_index = 19 * width as usize + 7;
        let body_mask_offset = body_mask_index * 4;
        assert_eq!(
            &mask[body_mask_offset..body_mask_offset + 4],
            &[147, 147, 147, 255]
        );
        let sprite = DefinitionSprite {
            graphics_scale: 1.0,
            image: ImageData::from_arc(width, height, image.into_pixels()),
            actions: engine
                .definition_action_graphics("FISH")
                .expect("FISH loads its real ActMap facets"),
            color_mask: Some(ColorByOwnerMask::new(width, height, mask)),
            shape: engine.definition_shape_rect("FISH"),
            fire_top: engine.definition_fire_top("FISH"),
            rotateable: engine.definition_rotateable("FISH"),
            line: engine.definition_line("FISH"),
            stretch_growth: engine.definition_stretch_growth("FISH"),
            top_face: engine.definition_top_face("FISH"),
            picture: None,
        };
        let mut graphics = GraphicsSystem::new(
            16,
            12,
            12,
            "real unowned FISH",
            test_font(),
            Arc::new(HashMap::from([(sprite_map_key("FISH", None), sprite)])),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        let render_body_pixel =
            |graphics: &mut GraphicsSystem, snapshot: &SimulationSnapshot| -> Color {
                let fish = snapshot.object(fish_id).expect("FISH remains present");
                graphics.surface_mut().fill(Color::opaque(0, 0, 0));
                graphics.viewport_x = (fish.position.x - 8) as f32;
                graphics.viewport_y = (fish.position.y - 6) as f32;
                graphics.paint_object(
                    fish,
                    &snapshot.objects,
                    &snapshot.players,
                    OWNER_NONE,
                    1.0,
                    &HashMap::new(),
                    0,
                    None,
                );
                graphics
                    .surface()
                    .get_pixel(7, 7)
                    .expect("body pixel lies on the output surface")
            };

        assert_eq!(
            render_body_pixel(&mut graphics, &first_snapshot),
            Color::opaque(147, 147, 147),
            "white Birth color must reveal the shipped grey fish body, not black"
        );

        let mut recolor = ObjectUpdate::new();
        recolor.color = Some(0x00ff_0000);
        engine
            .apply_object_update(fish_id, recolor)
            .expect("live FISH color changes");
        let recolored_snapshot = engine.snapshot();
        assert_eq!(
            recolored_snapshot
                .object(fish_id)
                .expect("FISH remains present")
                .color,
            0x00ff_0000
        );
        assert_eq!(
            render_body_pixel(&mut graphics, &recolored_snapshot),
            Color::opaque(147, 0, 0),
            "the real overlay follows live SetColorDw-style color, not owner lookup"
        );
    }

    #[test]
    fn color_by_owner_preserves_packed_transparency_with_ownclr() {
        // C4Surface::ClrByOwnerClr is passed to PerformBlt as the full packed
        // C4 color. Its high byte is transparency, and bit 4 keeps this raw
        // object color instead of folding in global ColorMod
        // (StdDDraw2.cpp:773-777). ReleaseClonk uses this exact combination.
        let sprite = DefinitionSprite {
            graphics_scale: 1.0,
            image: ImageData::new(1, 1, vec![0, 0, 0, 255]),
            actions: HashMap::new(),
            color_mask: Some(ColorByOwnerMask::new(1, 1, Arc::from([255]))),
            shape: Some(DefinitionRect::new(0, 0, 1, 1)),
            fire_top: 0,
            rotateable: 0,
            line: 0,
            stretch_growth: false,
            top_face: None,
            picture: None,
        };
        let mut object = make_snapshot().objects.remove(0);
        object.definition_id = "OwnerTransparency".to_string();
        object.position = Vector2::new(1, 1);
        object.color = 0x80ff_0000;
        object.color_modulation = 0x0000_ff00;
        object.blit_mode = C4GFXBLIT_CLRSFC_OWNCLR;
        object.crew_member = false;

        let mut graphics = GraphicsSystem::new(
            3,
            3,
            3,
            "Packed owner transparency",
            test_font(),
            Arc::new(HashMap::from([(
                sprite_map_key("OwnerTransparency", None),
                sprite,
            )])),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics.surface_mut().fill(Color::opaque(0, 0, 200));
        graphics.draw_objects(
            &[object],
            &[],
            &HashMap::new(),
            &[],
            OWNER_NONE,
            1.0,
            &HashMap::new(),
            ObjectRenderPass::Normal,
            None,
        );

        assert_eq!(
            graphics.surface().get_pixel(1, 1),
            Some(Color::opaque(127, 0, 100)),
            "0x80 transparency must blend raw red over blue; OWNCLR must ignore green ColorMod"
        );
    }

    #[test]
    fn object_additive_bit_covers_base_action_and_top_face_after_gamma() {
        // C4Object::Draw brackets its base/action facet with PrepareDrawing
        // (C4Object.cpp:2410-2416,2498-2499), and DrawTopFace brackets the
        // separate top pass the same way (C4Object.cpp:2648-2672). Bit 1 is
        // additive even alongside C4GFXBLIT_CUSTOM (C4Surface.h:38-49).
        let source = Color::new(64, 128, 192, 128);
        let mut sprites = HashMap::new();
        sprites.insert(
            sprite_map_key("BaseAdd", None),
            DefinitionSprite {
                graphics_scale: 1.0,
                image: ImageData::new(1, 1, vec![source.r, source.g, source.b, source.a]),
                actions: HashMap::new(),
                color_mask: None,
                shape: Some(DefinitionRect::new(0, 0, 1, 1)),
                fire_top: 0,
                rotateable: 0,
                line: 0,
                stretch_growth: false,
                top_face: None,
                picture: None,
            },
        );
        sprites.insert(
            sprite_map_key("ActionAdd", None),
            DefinitionSprite {
                graphics_scale: 1.0,
                image: ImageData::new(1, 1, vec![source.r, source.g, source.b, source.a]),
                actions: HashMap::from([(
                    "Active".to_string(),
                    DefinitionActionGraphics {
                        facet: Some(clonk_engine::DefinitionActionFacet {
                            x: 0,
                            y: 0,
                            width: 1,
                            height: 1,
                            target_x: 0,
                            target_y: 0,
                        }),
                        length: Some(1),
                        ..DefinitionActionGraphics::default()
                    },
                )]),
                color_mask: None,
                shape: Some(DefinitionRect::new(0, 0, 1, 1)),
                fire_top: 0,
                rotateable: 0,
                line: 0,
                stretch_growth: false,
                top_face: None,
                picture: None,
            },
        );
        sprites.insert(
            sprite_map_key("TopAdd", None),
            DefinitionSprite {
                graphics_scale: 1.0,
                image: ImageData::new(
                    2,
                    1,
                    vec![0, 0, 0, 0, source.r, source.g, source.b, source.a],
                ),
                actions: HashMap::new(),
                color_mask: None,
                shape: Some(DefinitionRect::new(0, 0, 1, 1)),
                fire_top: 0,
                rotateable: 0,
                line: 0,
                stretch_growth: false,
                top_face: Some(DefinitionTargetRect::new(1, 0, 1, 1, 0, 0)),
                picture: None,
            },
        );

        let template = make_snapshot().objects.remove(0);
        let make_object = |id, definition_id: &str, x, action: &str, blit_mode| {
            let mut object = template.clone();
            object.id = ObjectId::new(id);
            object.definition_id = definition_id.to_string();
            object.position = Vector2::new(x, 1);
            object.action = clonk_engine::ActionState::new(action);
            object.blit_mode = blit_mode;
            object.crew_member = false;
            object
        };
        let objects = vec![
            make_object(1, "BaseAdd", 1, "Idle", 129),
            make_object(2, "ActionAdd", 3, "Active", 129),
            make_object(3, "TopAdd", 5, "Idle", 129),
            make_object(4, "BaseAdd", 7, "Idle", 0),
        ];
        let gamma = clonk_graphics::GammaRamp::from_control_points([0x000000, 0x646464, 0xc8c8c8]);
        let mut graphics = GraphicsSystem::new(
            9,
            3,
            3,
            "Object additive",
            test_font(),
            Arc::new(sprites),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics.surface_mut().fill(Color::opaque(200, 200, 200));

        graphics.draw_objects(
            &objects,
            &[],
            &HashMap::new(),
            &[],
            OWNER_NONE,
            1.0,
            &HashMap::new(),
            ObjectRenderPass::Normal,
            Some(&gamma),
        );

        let additive = Some(Color::opaque(225, 250, 255));
        for (label, x) in [("base", 1), ("action", 3), ("top", 5)] {
            assert_eq!(graphics.surface().get_pixel(x, 1), additive, "{label}");
        }
        assert_eq!(
            graphics.surface().get_pixel(7, 1),
            Some(Color::opaque(125, 150, 175)),
            "normal mode must remain source-alpha over"
        );
    }

    #[test]
    fn graphics_overlay_additive_and_exact_parent_mode_preserve_owner_modulation() {
        // C4GraphicsOverlay::Draw uses its own mode, except exact
        // C4GFXBLIT_PARENT, which calls the parent object's PrepareDrawing
        // (C4DefGraphics.cpp:753-768,824-831). ColorByOwner modulation happens
        // before the selected framebuffer blend (StdDDraw2.cpp:769-777).
        let mut object = make_snapshot().objects.remove(0);
        object.position = Vector2::new(1, 1);
        object.blit_mode = 1;
        let source = Color::new(10, 20, 30, 128);
        let owner = 0x0064_788c;
        let sprite = DefinitionSprite {
            graphics_scale: 1.0,
            image: ImageData::new(1, 1, vec![source.r, source.g, source.b, source.a]),
            actions: HashMap::new(),
            color_mask: Some(ColorByOwnerMask::new(1, 1, Arc::from([255]))),
            shape: Some(DefinitionRect::new(0, 0, 1, 1)),
            fire_top: 0,
            rotateable: 0,
            line: 0,
            stretch_growth: false,
            top_face: None,
            picture: None,
        };
        let render = |object_mode, overlay_mode| {
            let mut object = object.clone();
            object.blit_mode = object_mode;
            object.graphics_overlays =
                vec![ObjectGraphicsOverlay::new(1, GraphicsOverlayMode::Base)
                    .with_definition(Some("OverlayAdd".to_string()))
                    .with_blit_mode(overlay_mode)];
            let mut graphics = GraphicsSystem::new(
                3,
                3,
                3,
                "Overlay additive",
                test_font(),
                Arc::new(HashMap::from([(
                    sprite_map_key("OverlayAdd", None),
                    sprite.clone(),
                )])),
                empty_cursor_atlas(),
                empty_hud_graphics(),
            );
            graphics.surface_mut().fill(Color::opaque(20, 30, 40));
            graphics.draw_object_overlays(
                &object,
                &[],
                &[],
                OWNER_NONE,
                Some(owner),
                1.0,
                1.0,
                1.0,
                0.0,
                None,
                None,
            );
            graphics.surface().get_pixel(1, 1)
        };

        let additive = Some(Color::opaque(70, 90, 110));
        assert_eq!(render(0, 1), additive, "explicit overlay additive");
        assert_eq!(render(1, 256), additive, "exact parent inheritance");
        assert_eq!(
            render(1, 0),
            Some(Color::opaque(60, 75, 90)),
            "explicit normal overlay must override an additive parent"
        );
    }

    #[test]
    fn object_overlay_draws_contained_overlay_only_target_at_host_offset() {
        // MODE_Object rewrites the viewport target so the referenced object is
        // drawn at the host position, using only the overlay transform's
        // truncated translation. It invokes both Draw and DrawTopFace with
        // ODM_Overlay, so containment is ignored and VIS_OverlayOnly is
        // evaluated with fAsOverlay=true (C4DefGraphics.cpp:753-789;
        // C4Object.cpp:2237-2258,2502-2505,2572-2580,2631-2633,5600-5608).
        let mut template = make_snapshot().objects.remove(0);
        template.crew_member = false;

        let mut host = template.clone();
        host.id = ObjectId::new(1);
        host.definition_id = "OverlayHost".to_string();
        host.position = Vector2::new(3, 3);
        host.blit_mode = 0;
        host.draw_transform = Some(DrawTransform::from_components(4.0, 4.0, 5.0, 5.0));
        host.graphics_overlays = vec![ObjectGraphicsOverlay::new(1, GraphicsOverlayMode::Object)
            .with_blit_mode(C4GFXBLIT_PARENT)
            .with_overlay_object(Some(ObjectId::new(2)))
            .with_transform(Some(DrawTransform::from_components(3.0, 2.0, 2.9, -1.9)))];

        let mut target = template;
        target.id = ObjectId::new(2);
        target.definition_id = "OverlayTarget".to_string();
        target.position = Vector2::new(10, 6);
        target.container = Some(host.id);
        target.owner = 4;
        target.visibility = clonk_engine::VIS_OVERLAY_ONLY | clonk_engine::VIS_OWNER;
        target.blit_mode = C4GFXBLIT_ADDITIVE;
        target.on_fire = true;
        target.current_shape = Some(DefinitionRect::new(0, 0, 1, 1));
        target.graphics_overlays.clear();

        let sprites = Arc::new(HashMap::from([
            (
                sprite_map_key("OverlayHost", None),
                DefinitionSprite {
                    graphics_scale: 1.0,
                    image: ImageData::new(1, 1, vec![0, 0, 0, 0]),
                    actions: HashMap::new(),
                    color_mask: None,
                    shape: Some(DefinitionRect::new(0, 0, 1, 1)),
                    fire_top: 0,
                    rotateable: 0,
                    line: 0,
                    stretch_growth: false,
                    top_face: None,
                    picture: None,
                },
            ),
            (
                sprite_map_key("OverlayTarget", None),
                DefinitionSprite {
                    graphics_scale: 1.0,
                    image: ImageData::new(2, 1, vec![40, 0, 0, 255, 0, 40, 0, 255]),
                    actions: HashMap::new(),
                    color_mask: None,
                    shape: Some(DefinitionRect::new(0, 0, 1, 1)),
                    fire_top: 0,
                    rotateable: 0,
                    line: 0,
                    stretch_growth: false,
                    top_face: Some(DefinitionTargetRect::new(1, 0, 1, 1, 1, 0)),
                    picture: None,
                },
            ),
        ]));
        let render = |for_player, target_blit_mode| {
            let mut target = target.clone();
            target.blit_mode = target_blit_mode;
            let mut graphics = GraphicsSystem::new(
                12,
                8,
                8,
                "Object overlay",
                test_font(),
                Arc::clone(&sprites),
                empty_cursor_atlas(),
                Arc::new(HudGraphics {
                    fire: Some(ImageData::new(1, 1, vec![0, 0, 20, 255])),
                    ..HudGraphics::default()
                }),
            );
            graphics.surface_mut().fill(Color::opaque(10, 10, 10));
            graphics.draw_objects(
                &[host.clone(), target.clone()],
                &[],
                &HashMap::new(),
                &[],
                for_player,
                1.0,
                &HashMap::new(),
                ObjectRenderPass::Normal,
                None,
            );
            graphics.surface().clone()
        };

        let visible = render(4, C4GFXBLIT_ADDITIVE);
        assert_eq!(
            visible.get_pixel(5, 2),
            Some(Color::opaque(50, 10, 30)),
            "MODE_Object fire inherits the target's additive PARENT state"
        );
        assert_eq!(visible.get_pixel(6, 2), Some(Color::opaque(10, 50, 10)));
        assert_eq!(visible.get_pixel(7, 2), Some(Color::opaque(10, 10, 10)));
        assert_eq!(visible.get_pixel(10, 6), Some(Color::opaque(10, 10, 10)));

        let normal = render(4, 0);
        assert_eq!(
            normal.get_pixel(5, 2),
            Some(Color::opaque(40, 0, 0)),
            "MODE_Object paints fire before the referenced object's opaque face"
        );

        let hidden = render(5, C4GFXBLIT_ADDITIVE);
        assert_eq!(hidden.get_pixel(5, 2), Some(Color::opaque(10, 10, 10)));
        assert_eq!(hidden.get_pixel(6, 2), Some(Color::opaque(10, 10, 10)));

        let render_parallax = |host: ObjectSnapshot,
                               target: ObjectSnapshot,
                               viewport: Vector2,
                               width: u32,
                               height: u32| {
            let mut graphics = GraphicsSystem::new(
                width,
                height,
                height as i32,
                "Parallax object overlay",
                test_font(),
                Arc::clone(&sprites),
                empty_cursor_atlas(),
                empty_hud_graphics(),
            );
            graphics.viewport_x = viewport.x as f32;
            graphics.viewport_y = viewport.y as f32;
            graphics.surface_mut().fill(Color::opaque(10, 10, 10));
            graphics.draw_objects(
                &[host, target],
                &[],
                &HashMap::new(),
                &[],
                4,
                1.0,
                &HashMap::new(),
                ObjectRenderPass::Normal,
                None,
            );
            graphics.surface().clone()
        };

        let int_value = |value| {
            serde_json::from_value(serde_json::json!({ "Int": value }))
                .expect("deserialize C4Script integer")
        };
        let mut percentage_host = host.clone();
        percentage_host.position = Vector2::new(50, 30);
        percentage_host.category |= CATEGORY_PARALLAX_FLAG;
        percentage_host
            .local_vars
            .insert("__local_0".to_string(), int_value(50));
        percentage_host
            .local_vars
            .insert("__local_1".to_string(), int_value(50));
        let mut percentage_target = target.clone();
        percentage_target.position = Vector2::new(70, 40);
        let percentage = render_parallax(
            percentage_host,
            percentage_target,
            Vector2::new(20, 20),
            80,
            50,
        );
        assert_eq!(
            percentage.get_pixel(42, 19),
            Some(Color::opaque(50, 10, 10)),
            "Local(0/1)=50 scales viewport TargetX/Y before MODE_Object anchoring"
        );

        let mut hud_host = host.clone();
        hud_host.position = Vector2::new(-10, -5);
        hud_host.category |= CATEGORY_PARALLAX_FLAG;
        hud_host
            .local_vars
            .insert("__local_0".to_string(), int_value(0));
        hud_host
            .local_vars
            .insert("__local_1".to_string(), int_value(0));
        let hud = render_parallax(hud_host, target, Vector2::new(20, 20), 80, 50);
        assert_eq!(
            hud.get_pixel(72, 44),
            Some(Color::opaque(50, 10, 10)),
            "zero parallax plus negative host coordinates anchors from right/bottom"
        );
    }

    /// Bounding box of every pixel matching `wanted`, as (min_x, min_y, max_x, max_y).
    fn colour_bbox(surface: &Surface, wanted: Color) -> Option<(u32, u32, u32, u32)> {
        let mut found: Option<(u32, u32, u32, u32)> = None;
        for y in 0..surface.height() {
            for x in 0..surface.width() {
                if surface.get_pixel(x, y) == Some(wanted) {
                    found = Some(match found {
                        None => (x, y, x, y),
                        Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
                    });
                }
            }
        }
        found
    }

    #[test]
    fn base_overlay_draws_the_source_definition_shape_instead_of_the_whole_sheet() {
        // MODE_Base selects (0,0,Shape.Wdt,Shape.Hgt), carries Shape.x/y as
        // the facet target, and DrawT scales only that source crop by the
        // source definition's Scale (src/C4DefGraphics.cpp:636-637,815-821;
        // src/C4Facet.cpp:61-68). Hazard's Spawnpoint uses this mode for its
        // floating item, whose graphics sheet is much larger than its shape.
        let left_half = Color::opaque(30, 210, 90);
        let right_half = Color::opaque(220, 40, 70);
        let mut pixels = vec![0u8; 16 * 8 * 4];
        for y in 0..6 {
            for x in 0..8 {
                let base = (y * 16 + x) * 4;
                pixels[base..base + 4].copy_from_slice(if x < 4 {
                    &[30, 210, 90, 255]
                } else {
                    &[220, 40, 70, 255]
                });
            }
        }
        let source_sprite = DefinitionSprite {
            graphics_scale: 2.0,
            image: ImageData::new(16, 8, pixels),
            actions: HashMap::new(),
            color_mask: None,
            shape: Some(DefinitionRect::new(-3, 1, 4, 3)),
            fire_top: 0,
            rotateable: 0,
            line: 0,
            stretch_growth: false,
            top_face: None,
            picture: None,
        };
        let mut object = make_snapshot().objects.remove(0);
        object.position = Vector2::new(12, 10);
        object.graphics_overlays = vec![ObjectGraphicsOverlay::new(1, GraphicsOverlayMode::Base)
            .with_definition(Some("Pickup".to_string()))
            .with_transform(Some(DrawTransform::identity()))];
        let mut graphics = GraphicsSystem::new(
            24,
            24,
            24,
            "Base overlay shape",
            test_font(),
            Arc::new(HashMap::from([(
                sprite_map_key("Pickup", None),
                source_sprite,
            )])),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics.set_point_filtering(true);
        graphics.surface_mut().fill(Color::opaque(10, 10, 10));

        graphics.draw_object_overlays(
            &object,
            &[],
            &[],
            OWNER_NONE,
            None,
            12.0,
            10.0,
            1.0,
            0.0,
            None,
            None,
        );

        assert_eq!(
            colour_bbox(graphics.surface(), left_half),
            Some((9, 11, 10, 13)),
        );
        assert_eq!(
            colour_bbox(graphics.surface(), right_half),
            Some((11, 11, 12, 13)),
        );
    }

    #[test]
    fn base_overlay_with_no_definition_shape_draws_nothing() {
        // C4DefCore::Default zeroes Shape, MODE_Base copies that zero-sized
        // facet, and C4Facet::DrawT rejects it (src/C4Def.cpp:122-131;
        // src/C4DefGraphics.cpp:636-637; src/C4Facet.cpp:61-68).
        let overlay_color = Color::opaque(220, 40, 70);
        let mut object = make_snapshot().objects.remove(0);
        object.position = Vector2::new(3, 3);
        object.graphics_overlays = vec![ObjectGraphicsOverlay::new(1, GraphicsOverlayMode::Base)
            .with_definition(Some("Shapeless".to_string()))
            .with_transform(Some(DrawTransform::identity()))];
        let mut graphics = GraphicsSystem::new(
            7,
            7,
            7,
            "Shapeless base overlay",
            test_font(),
            solid_sprite("Shapeless", 2, 2, overlay_color, None, false),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics.surface_mut().fill(Color::opaque(10, 10, 10));

        graphics.draw_object_overlays(
            &object,
            &[],
            &[],
            OWNER_NONE,
            None,
            3.0,
            3.0,
            1.0,
            0.0,
            None,
            None,
        );

        assert_eq!(colour_bbox(graphics.surface(), overlay_color), None);
    }

    #[test]
    fn ingame_picture_overlay_draws_the_source_picture_rect_zoomed_to_the_host_shape() {
        // C4GraphicsOverlay::UpdateFacet sets MODE_IngamePicture's facet to the
        // SOURCE definition's PictureRect and turns on fZoomToShape
        // (src/C4DefGraphics.cpp:660-664). Draw then rebases the script
        // transform at the object origin, applies
        // fZoom = min(Shape.Wdt/fct.Wdt, Shape.Hgt/fct.Hgt) about that origin,
        // and blits the facet centred there (src/C4DefGraphics.cpp:818-826).
        let host_sprite = DefinitionSprite {
            graphics_scale: 1.0,
            image: ImageData::new(1, 1, vec![0, 0, 0, 0]),
            actions: HashMap::new(),
            color_mask: None,
            shape: Some(DefinitionRect::new(-8, -6, 16, 12)),
            fire_top: 0,
            rotateable: 0,
            line: 0,
            stretch_growth: false,
            top_face: None,
            picture: None,
        };
        // 16x16 sheet, transparent except a 4x4 marker block at (8,0) that the
        // source definition declares as its DefCore Picture rect.
        let marker = Color::opaque(30, 210, 90);
        let mut pixels = vec![0u8; 16 * 16 * 4];
        for y in 0..4 {
            for x in 8..12 {
                let base = (y * 16 + x) * 4;
                pixels[base..base + 4].copy_from_slice(&[30, 210, 90, 255]);
            }
        }
        let source_sprite = DefinitionSprite {
            graphics_scale: 1.0,
            image: ImageData::new(16, 16, pixels),
            actions: HashMap::new(),
            color_mask: None,
            shape: Some(DefinitionRect::new(-2, -2, 4, 4)),
            fire_top: 0,
            rotateable: 0,
            line: 0,
            stretch_growth: false,
            top_face: None,
            picture: Some(DefinitionRect::new(8, 0, 4, 4)),
        };

        let render = |transform: Option<DrawTransform>| {
            let mut object = make_snapshot().objects.remove(0);
            object.definition_id = "PicHost".to_string();
            object.position = Vector2::new(20, 20);
            object.graphics_overlays =
                vec![
                    ObjectGraphicsOverlay::new(1, GraphicsOverlayMode::IngamePicture)
                        .with_definition(Some("PicSrc".to_string()))
                        .with_transform(transform),
                ];
            let mut graphics = GraphicsSystem::new(
                40,
                40,
                40,
                "Ingame picture overlay",
                test_font(),
                Arc::new(HashMap::from([
                    (sprite_map_key("PicHost", None), host_sprite.clone()),
                    (sprite_map_key("PicSrc", None), source_sprite.clone()),
                ])),
                empty_cursor_atlas(),
                empty_hud_graphics(),
            );
            // Keep the magnified edges exactly the marker colour so the bbox is
            // a real assertion rather than a bilinear smear.
            graphics.set_point_filtering(true);
            graphics.surface_mut().fill(Color::opaque(10, 10, 10));
            graphics.draw_object_overlays(
                &object,
                &[],
                &[],
                OWNER_NONE,
                None,
                20.0,
                20.0,
                1.0,
                0.0,
                None,
                None,
            );
            colour_bbox(graphics.surface(), marker)
        };

        // fZoom = min(16/4, 12/4) = 3. The 4x4 facet is centred at the object
        // origin (20,20) — i.e. 18..22 — then scaled 3x about (20,20), giving
        // 14..26 on both axes: a 12x12 block.
        assert_eq!(render(None), Some((14, 14, 25, 25)));

        // C4DrawTransform::ScaleAt composes the zoom OUTSIDE the script
        // transform (src/StdDDraw2.h:83-94), so a script translation of +4 is
        // multiplied by fZoom and moves the block by 12, not 4.
        let translated = DrawTransform::from_components(1.0, 1.0, 4.0, 0.0);
        assert_eq!(render(Some(translated)), Some((26, 14, 37, 25)));
    }

    #[test]
    fn extra_graphics_overlay_redraws_the_host_face_from_the_overlay_bitmap() {
        // MODE_ExtraGraphics swaps the host's bitmap for the overlay's, installs
        // the composed transform, and re-enters the host's own base draw:
        //   pForObj->SetGraphics(pSourceGfx, true);
        //   pForObj->pDrawTransform = &trf;
        //   pForObj->Draw(cgo, iByPlayer, ODM_BaseOnly);
        //   pForObj->DrawTopFace(cgo, iByPlayer, ODM_BaseOnly);
        // (src/C4DefGraphics.cpp:788-811). SetGraphics(gfx, fTemp) swaps only
        // the bitmap; Shape and the ActMap stay with the host's own definition
        // (src/C4Object.cpp:377-382). This is the Knights shield
        // (content/Knights.c4d/Crew.c4d/Knight.c4d/Script.c:1214), which passes
        // GetID() so host and source share a definition and differ only in the
        // named graphics sheet.
        let sheet = |rgba: [u8; 4]| DefinitionSprite {
            graphics_scale: 1.0,
            image: ImageData::new(8, 8, rgba.repeat(64)),
            actions: HashMap::new(),
            color_mask: None,
            shape: Some(DefinitionRect::new(-4, -4, 8, 8)),
            fire_top: 0,
            rotateable: 0,
            line: 0,
            stretch_growth: false,
            top_face: None,
            picture: None,
        };
        let base_colour = Color::opaque(200, 40, 40);
        let shield_colour = Color::opaque(40, 80, 200);

        let render = |with_overlay: bool| {
            let mut object = make_snapshot().objects.remove(0);
            object.definition_id = "KNIG".to_string();
            object.position = Vector2::new(16, 16);
            object.graphics_overlays = if with_overlay {
                vec![
                    ObjectGraphicsOverlay::new(1, GraphicsOverlayMode::ExtraGraphics)
                        .with_definition(Some("KNIG".to_string()))
                        .with_graphics_name(Some("Shield".to_string())),
                ]
            } else {
                Vec::new()
            };
            let mut graphics = GraphicsSystem::new(
                32,
                32,
                32,
                "Extra graphics overlay",
                test_font(),
                Arc::new(HashMap::from([
                    (sprite_map_key("KNIG", None), sheet([200, 40, 40, 255])),
                    (
                        sprite_map_key("KNIG", Some("Shield")),
                        sheet([40, 80, 200, 255]),
                    ),
                ])),
                empty_cursor_atlas(),
                empty_hud_graphics(),
            );
            graphics.set_point_filtering(true);
            graphics.surface_mut().fill(Color::opaque(10, 10, 10));
            graphics.draw_object_overlays(
                &object,
                &[],
                &[],
                OWNER_NONE,
                None,
                16.0,
                16.0,
                1.0,
                0.0,
                None,
                None,
            );
            graphics.surface().clone()
        };

        // Without the overlay the walk draws nothing at all.
        assert_eq!(colour_bbox(&render(false), shield_colour), None);

        // With it, the host's own 8x8 Shape is redrawn from the shield sheet at
        // the host's shape origin (16-4, 16-4) = (12, 12).
        let drawn = render(true);
        assert_eq!(
            colour_bbox(&drawn, shield_colour),
            Some((12, 12, 19, 19)),
            "the host face is redrawn from the overlay's bitmap at host geometry"
        );
        // The overlay must not pull in the host's own base sheet: that is drawn
        // by the normal face pass, not by the overlay walk.
        assert_eq!(colour_bbox(&drawn, base_colour), None);
    }

    #[test]
    fn extra_graphics_overlay_composes_the_host_transform_first() {
        // `trf = *pPrevTrf; trf *= Transform;` (src/C4DefGraphics.cpp:795-804).
        // CBltTransform::operator*= applies the RIGHT operand last
        // (src/StdDDraw2.h:96-110), so the host's own draw transform is the
        // inner map and the overlay's is the outer one. With a host translate
        // of +4 and an overlay scale of 2 that is x' = 2(x + 4); the reversed
        // order would give x' = 2x + 4 and shift the face by 4 px instead of 8.
        let sheet = DefinitionSprite {
            graphics_scale: 1.0,
            image: ImageData::new(8, 8, [40, 80, 200, 255].repeat(64)),
            actions: HashMap::new(),
            color_mask: None,
            shape: Some(DefinitionRect::new(-4, -4, 8, 8)),
            fire_top: 0,
            rotateable: 0,
            line: 0,
            stretch_growth: false,
            top_face: None,
            picture: None,
        };
        let mut object = make_snapshot().objects.remove(0);
        object.definition_id = "KNIG".to_string();
        object.position = Vector2::new(16, 16);
        object.draw_transform = Some(DrawTransform::from_components(1.0, 1.0, 4.0, 0.0));
        object.graphics_overlays =
            vec![
                ObjectGraphicsOverlay::new(1, GraphicsOverlayMode::ExtraGraphics)
                    .with_definition(Some("KNIG".to_string()))
                    .with_transform(Some(DrawTransform::from_components(2.0, 2.0, 0.0, 0.0))),
            ];

        let mut graphics = GraphicsSystem::new(
            48,
            48,
            48,
            "Extra graphics transform order",
            test_font(),
            Arc::new(HashMap::from([(sprite_map_key("KNIG", None), sheet)])),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics.set_point_filtering(true);
        graphics.surface_mut().fill(Color::opaque(10, 10, 10));
        graphics.draw_object_overlays(
            &object,
            &[],
            &[],
            OWNER_NONE,
            None,
            16.0,
            16.0,
            1.0,
            0.0,
            None,
            None,
        );

        // The 8x8 face spans world 12..20. Composed and rebased at the shape
        // centre (16,16) the map is x' = 2x - 8, y' = 2y - 16, so the block
        // covers x 16..31 and y 8..23.
        assert_eq!(
            colour_bbox(graphics.surface(), Color::opaque(40, 80, 200)),
            Some((16, 8, 31, 23)),
        );
    }

    #[test]
    fn shipped_clonkmars_hud_item_log_icon_matches_the_cpp_geometry() {
        // End-to-end pin for ClonkMars' MHUD item-pickup log
        // (ClonkMars.c4d/Helpers.c4d/HUD.c4d/Script.c DrawLogItem):
        //   SetGraphics(0, this, val, HUD_ItemLog, GFXOV_MODE_IngamePicture)
        //   SetObjDrawTransform(15*1000/w, 0, OverlayShiftX(w) + 18000,
        //                       0, 15*1000/h, OverlayShiftY(h) + 7000, this, ov)
        // with OverlayShiftX(w) = 1000*(Offset.x + w/2). The script asks for a
        // 15px icon, but fZoomToShape multiplies that by
        // min(160/64, 120/64) = 1.875 (src/C4DefGraphics.cpp:820-825), so C++
        // actually renders 64 * 0.234 * 1.875 = 28.08px. That is the oracle;
        // do not "correct" it toward the content's stated intent.
        let hud =
            crate::test_support::repo_root().join("content/ClonkMars.c4d/Helpers.c4d/HUD.c4d");
        assert!(
            hud.is_dir(),
            "the bundled ClonkMars pack must provide {}",
            hud.display()
        );
        let def_core =
            std::fs::read_to_string(hud.join("DefCore.txt")).expect("read shipped MHUD DefCore");
        let value = |key: &str| {
            def_core
                .lines()
                .find_map(|line| line.trim().strip_prefix(key))
                .unwrap_or_else(|| panic!("MHUD DefCore has {key}"))
                .to_string()
        };
        assert_eq!(value("Width="), "160");
        assert_eq!(value("Height="), "120");
        assert_eq!(value("Offset="), "-80,-60");

        // The content comment assumes a 64x64 item picture.
        let picture_extent = 64;
        let marker = Color::opaque(15, 120, 240);
        let source_sprite = DefinitionSprite {
            graphics_scale: 1.0,
            image: ImageData::new(
                picture_extent as u32,
                picture_extent as u32,
                [15, 120, 240, 255].repeat((picture_extent * picture_extent) as usize),
            ),
            actions: HashMap::new(),
            color_mask: None,
            shape: Some(DefinitionRect::new(-32, -32, 64, 64)),
            fire_top: 0,
            rotateable: 0,
            line: 0,
            stretch_growth: false,
            top_face: None,
            picture: Some(DefinitionRect::new(0, 0, picture_extent, picture_extent)),
        };
        let hud_sprite = DefinitionSprite {
            graphics_scale: 1.0,
            image: ImageData::new(1, 1, vec![0, 0, 0, 0]),
            actions: HashMap::new(),
            color_mask: None,
            shape: Some(DefinitionRect::new(-80, -60, 160, 120)),
            fire_top: 0,
            rotateable: 0,
            line: 0,
            stretch_growth: false,
            top_face: None,
            picture: None,
        };

        // DrawLogItem's transform, in the engine's 1/1000 units.
        let scale = 15.0 * 1000.0 / picture_extent as f32 / 1000.0;
        let shift_x = (-80 + picture_extent / 2) as f32 + 18.0;
        let shift_y = (-60 + picture_extent / 2) as f32 + 7.0;

        let mut object = make_snapshot().objects.remove(0);
        object.definition_id = "MHUD".to_string();
        object.position = Vector2::new(150, 105);
        object.graphics_overlays =
            vec![
                ObjectGraphicsOverlay::new(6, GraphicsOverlayMode::IngamePicture)
                    .with_definition(Some("ITEM".to_string()))
                    .with_transform(Some(DrawTransform::from_components(
                        scale, scale, shift_x, shift_y,
                    ))),
            ];

        let mut graphics = GraphicsSystem::new(
            200,
            140,
            140,
            "ClonkMars item log",
            test_font(),
            Arc::new(HashMap::from([
                (sprite_map_key("MHUD", None), hud_sprite),
                (sprite_map_key("ITEM", None), source_sprite),
            ])),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics.set_point_filtering(true);
        graphics.surface_mut().fill(Color::opaque(10, 10, 10));
        graphics.draw_object_overlays(
            &object,
            &[],
            &[],
            OWNER_NONE,
            None,
            150.0,
            105.0,
            1.0,
            0.0,
            None,
            None,
        );

        // Final map is x' = 0.43875x + 27.9375, y' = 0.43875y + 19.55625 over a
        // 64x64 quad at (118, 73): x 79.71..107.79, y 51.59..79.67.
        let bbox = colour_bbox(graphics.surface(), marker).expect("item-log icon is drawn");
        assert_eq!(bbox, (80, 52, 107, 79));
        assert_eq!(bbox.2 - bbox.0 + 1, 28, "64px picture renders 28px wide");
        assert_eq!(bbox.3 - bbox.1 + 1, 28, "64px picture renders 28px tall");
    }

    #[test]
    fn picture_and_none_overlay_modes_never_draw_in_the_object_walk() {
        // C4Object::Draw filters the overlay chain on !IsPicture(), and
        // IsPicture() is `eMode == MODE_Picture` alone
        // (src/C4Object.cpp:2526-2529; src/C4DefGraphics.h:247) —
        // C4GraphicsOverlay::Draw even asserts it (src/C4DefGraphics.cpp:758).
        // MODE_None passes that filter but IsValid rejects it because
        // UpdateFacet leaves fctBlit defaulted (src/C4DefGraphics.cpp:638-639,
        // :709-710). Locks both before the dispatcher's catch-all is removed.
        let sprite = solid_sprite(
            "PicSrc",
            16,
            16,
            Color::opaque(220, 40, 40),
            Some(DefinitionRect::new(-8, -8, 16, 16)),
            false,
        );
        let background = Color::opaque(200, 0, 200);
        let render = |mode| {
            let mut object = make_snapshot().objects.remove(0);
            object.position = Vector2::new(8, 8);
            object.graphics_overlays = vec![
                ObjectGraphicsOverlay::new(1, mode).with_definition(Some("PicSrc".to_string()))
            ];
            let mut graphics = GraphicsSystem::new(
                16,
                16,
                16,
                "Non-drawing overlay modes",
                test_font(),
                Arc::clone(&sprite),
                empty_cursor_atlas(),
                empty_hud_graphics(),
            );
            graphics.surface_mut().fill(background);
            graphics.draw_object_overlays(
                &object,
                &[],
                &[],
                OWNER_NONE,
                None,
                8.0,
                8.0,
                1.0,
                0.0,
                None,
                None,
            );
            let surface = graphics.surface().clone();
            (0..16).all(|y| (0..16).all(|x| surface.get_pixel(x, y) == Some(background)))
        };

        assert!(
            render(GraphicsOverlayMode::Picture),
            "MODE_Picture is in-game invisible"
        );
        assert!(render(GraphicsOverlayMode::None), "MODE_None has no facet");
    }

    #[test]
    fn action_overlay_scales_its_source_by_the_source_definition_scale() {
        // C4GraphicsOverlay::Draw hands pSourceGfx->pDef->Scale to
        // C4Facet::DrawT for every facet-drawn overlay mode
        // (src/C4DefGraphics.cpp:826), and DrawT multiplies only the source
        // rectangle by it, leaving the destination at the unscaled facet
        // extent (src/C4Facet.cpp:79-81). MODE_Action takes its facet
        // straight from the ActMap entry (src/C4DefGraphics.cpp:810), which
        // stays in unscaled coordinates, so a Scale=200 sheet must be read
        // at twice those coordinates.
        let red = Color::opaque(200, 40, 40);
        let green = Color::opaque(0, 200, 0);
        let mut pixels = [red.r, red.g, red.b, red.a].repeat(16 * 16);
        for y in 0..8 {
            for x in 8..16 {
                let base = (y * 16 + x) * 4;
                pixels[base..base + 4].copy_from_slice(&[green.r, green.g, green.b, green.a]);
            }
        }
        let sprite = DefinitionSprite {
            graphics_scale: 2.0,
            image: ImageData::new(16, 16, pixels),
            actions: HashMap::from([(
                "Wave".to_string(),
                DefinitionActionGraphics {
                    facet: Some(clonk_engine::DefinitionActionFacet {
                        x: 4,
                        y: 0,
                        width: 4,
                        height: 4,
                        target_x: 0,
                        target_y: 0,
                    }),
                    length: Some(1),
                    ..DefinitionActionGraphics::default()
                },
            )]),
            color_mask: None,
            shape: Some(DefinitionRect::new(-2, -2, 4, 4)),
            fire_top: 0,
            rotateable: 0,
            line: 0,
            stretch_growth: false,
            top_face: None,
            picture: None,
        };

        let mut object = make_snapshot().objects.remove(0);
        object.definition_id = "HdOverlay".to_string();
        object.position = Vector2::new(8, 4);
        object.graphics_overlays = vec![ObjectGraphicsOverlay::new(1, GraphicsOverlayMode::Action)
            .with_definition(Some("HdOverlay".to_string()))
            .with_action(Some("Wave".to_string()))];

        let background = Color::opaque(10, 10, 10);
        let mut graphics = GraphicsSystem::new(
            16,
            8,
            8,
            "HD action overlay",
            test_font(),
            Arc::new(HashMap::from([(sprite_map_key("HdOverlay", None), sprite)])),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics.surface_mut().fill(background);
        graphics.draw_object_overlays(
            &object,
            &[],
            &[],
            OWNER_NONE,
            None,
            8.0,
            4.0,
            1.0,
            0.0,
            None,
            None,
        );

        // The 4x4 ActMap facet at (4,0) reads (8,0,8,8) from the 2x sheet —
        // the green half — into a 4x4 destination centred on the object,
        // covering x 6..9 and y 2..5.
        for (x, y) in [(6u32, 2u32), (9, 2), (6, 5), (9, 5)] {
            assert_eq!(
                graphics.surface().get_pixel(x, y),
                Some(green),
                "MODE_Action must read its facet through the source definition's Scale"
            );
        }
        assert_eq!(
            graphics.surface().get_pixel(10, 4),
            Some(background),
            "the source scale must not enlarge the destination facet"
        );
    }

    #[test]
    fn hd_crew_art_reaches_the_gpu_one_authored_texel_per_device_pixel() {
        // The end of the high-resolution chain. A rendered crew pack authors
        // Walk as `Facet=0,0,16,22` at DefCore `Scale=300`, so the source is
        // 48x66 texels for a facet that stays 16x22 GAME units — object
        // geometry never follows the art (C4Object.cpp:438-467).
        //
        // The retained scene records vertices in LOGICAL units and the
        // renderer projects them by the presentation scale
        // (gpu_renderer.rs:2588-2615), so the quad only lands 1:1 if it
        // reaches the GPU with an uncorrected source and a nearest sampler.
        let facet = SourceRect::new(0, 0, 16, 22);
        let sprite = DefinitionSprite {
            graphics_scale: 3.0,
            image: ImageData::new(96, 132, vec![255_u8; 96 * 132 * 4]),
            actions: HashMap::new(),
            color_mask: None,
            shape: Some(DefinitionRect::new(0, 0, 16, 22)),
            fire_top: 0,
            rotateable: 0,
            line: 0,
            stretch_growth: false,
            top_face: None,
            picture: None,
        };

        let capture = |presentation_scale: f32, hd_exact_blits: bool| {
            let mut graphics = test_graphics(64, 64, 0, "hd crew gpu");
            graphics.set_presentation_scale(presentation_scale);
            graphics.set_hd_exact_blits(hd_exact_blits);
            graphics.begin_gpu_scene_capture();
            graphics.blit_face(
                &sprite,
                facet,
                (0.0, 0.0, 16.0, 22.0),
                (8.0, 11.0),
                None,
                1.0,
                0.0,
                None,
                SpriteBlitState::normal(),
                None,
            );
            let gamma = clonk_graphics::GammaRamp::identity();
            let scene = graphics
                .surface_mut()
                .take_gpu_scene_capture()
                .expect("sprite capture stays active across the blit")
                .into_scene([64, 64], Color::transparent(), &gamma);
            let extent = scene.textures[0].extent;
            let quad = scene
                .commands
                .iter()
                .find_map(|command| match command {
                    clonk_graphics::GpuCommand::ObjectBatch { sprites, .. } => {
                        sprites.first().map(|sprite| {
                            (
                                sprite.positions,
                                [
                                    [sprite.uv[0], sprite.uv[1]],
                                    [sprite.uv[2], sprite.uv[1]],
                                    [sprite.uv[0], sprite.uv[3]],
                                    [sprite.uv[2], sprite.uv[3]],
                                ],
                                sprite.sampler(),
                            )
                        })
                    }
                    _ => None,
                })
                .expect("the face blit retains a compact object sprite");
            (extent, quad)
        };

        let (extent, (positions, uv, sampler)) = capture(3.0, true);
        assert_eq!(sampler, clonk_graphics::GpuSampler::Nearest);

        // Source: the UV span covers exactly the 48x66 authored texels.
        let span = |axis: usize| {
            let values = uv.iter().map(|uv| uv[axis]);
            let max = values.clone().fold(f32::MIN, f32::max);
            let min = values.fold(f32::MAX, f32::min);
            (max - min) * extent[axis] as f32
        };
        assert_eq!((span(0), span(1)), (48.0, 66.0), "authored source texels");

        // Destination: the retained vertices are logical game units.
        let logical = |axis: usize| {
            let values = positions.iter().map(|position| position[axis]);
            let max = values.clone().fold(f32::MIN, f32::max);
            let min = values.fold(f32::MAX, f32::min);
            max - min
        };
        assert_eq!((logical(0), logical(1)), (16.0, 22.0), "logical game units");

        // Projection: the renderer's own transform turns those logical units
        // into device pixels, and 48x66 texels land on 48x66 of them.
        let projection = clonk_graphics::ClipperProjection::new(
            3.0,
            (64, 64),
            64 * 3,
            clonk_graphics::Rect::new(0, 0, 64, 64),
        );
        let (left, top) = projection.logical_to_physical(0.0, 0.0);
        let (right, bottom) = projection.logical_to_physical(16.0, 22.0);
        assert_eq!(
            (right - left, bottom - top),
            (48.0, 66.0),
            "one authored texel per device pixel at Graphics.Scale=300"
        );

        // Without the opt-in the same blit takes C++'s half-texel correction
        // and a linear filter, which is what softens the art today.
        let (_, (_, corrected, sampling)) = capture(3.0, false);
        assert_eq!(sampling, clonk_graphics::GpuSampler::Linear);
        assert_ne!(
            corrected[0], uv[0],
            "the correction must move the sampled origin"
        );
    }

    #[test]
    fn action_overlay_clamps_a_scaled_facet_to_the_source_sheet() {
        // blit_face clamps the source rect to the sheet after applying the
        // definition Scale and shrinks the destination by the same ratio
        // (graphics_system.rs:7285-7299), mirroring the per-tile clamp C++
        // performs in CStdDDraw::Blit (src/StdDDraw2.cpp:757-766).
        // draw_action_graphic checked `source_within_image` on the UNSCALED
        // facet and then scaled it, so a facet that fits the logical grid but
        // overflows once multiplied by Scale reached the rasterizer unclamped.
        //
        // Facet (6,0,4,4) fits a 16x16 sheet; at Scale=200 it reads (12,0,8,8),
        // which runs four pixels past the right edge. Only the four columns
        // that exist may be drawn, into a correspondingly narrower destination.
        let red = Color::opaque(200, 40, 40);
        let green = Color::opaque(0, 200, 0);
        let mut pixels = [red.r, red.g, red.b, red.a].repeat(16 * 16);
        for y in 0..16 {
            for x in 12..16 {
                let base = (y * 16 + x) * 4;
                pixels[base..base + 4].copy_from_slice(&[green.r, green.g, green.b, green.a]);
            }
        }
        let sprite = DefinitionSprite {
            graphics_scale: 2.0,
            image: ImageData::new(16, 16, pixels),
            actions: HashMap::from([(
                "Wave".to_string(),
                DefinitionActionGraphics {
                    facet: Some(clonk_engine::DefinitionActionFacet {
                        x: 6,
                        y: 0,
                        width: 4,
                        height: 4,
                        target_x: 0,
                        target_y: 0,
                    }),
                    length: Some(1),
                    ..DefinitionActionGraphics::default()
                },
            )]),
            color_mask: None,
            shape: Some(DefinitionRect::new(-2, -2, 4, 4)),
            fire_top: 0,
            rotateable: 0,
            line: 0,
            stretch_growth: false,
            top_face: None,
            picture: None,
        };

        let mut object = make_snapshot().objects.remove(0);
        object.definition_id = "HdClamp".to_string();
        object.position = Vector2::new(8, 4);
        object.graphics_overlays = vec![ObjectGraphicsOverlay::new(1, GraphicsOverlayMode::Action)
            .with_definition(Some("HdClamp".to_string()))
            .with_action(Some("Wave".to_string()))];

        let background = Color::opaque(10, 10, 10);
        let mut graphics = GraphicsSystem::new(
            16,
            8,
            8,
            "HD facet clamp",
            test_font(),
            Arc::new(HashMap::from([(sprite_map_key("HdClamp", None), sprite)])),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics.surface_mut().fill(background);
        graphics.draw_object_overlays(
            &object,
            &[],
            &[],
            OWNER_NONE,
            None,
            8.0,
            4.0,
            1.0,
            0.0,
            None,
            None,
        );

        // Half the eight scaled source columns exist, so the 4-wide facet is
        // drawn two pixels wide from the same origin.
        assert_eq!(
            graphics.surface().get_pixel(6, 3),
            Some(green),
            "the in-bounds half of the scaled facet must still be drawn"
        );
        assert_eq!(
            graphics.surface().get_pixel(9, 3),
            Some(background),
            "a scaled facet may not be drawn past the columns the sheet actually has"
        );
    }

    #[test]
    fn parallax_action_overlay_ignores_viewport_scroll() {
        // C4Object::Draw resolves its output origin through TargetPos before
        // the face draw (src/C4Object.cpp:2271), and C4GraphicsOverlay::Draw
        // repeats that resolution for every overlay
        // (src/C4DefGraphics.cpp:763-765). For a C4D_Parallax object whose
        // Local(0)/Local(1) are unset and whose coordinates are non-negative,
        // ApplyParallaxity yields riTx = riTy = 0 (src/C4Object.cpp:5839-5852),
        // so the overlay is pinned to the viewport at every scroll position.
        // ClonkMars' MHUD oxygen meter is exactly that object: a C4D_Parallax
        // StaticBack at (150,105) carrying a MODE_Action overlay per meter.
        let mut pixels = vec![0u8; 8 * 4 * 4];
        for y in 0..4 {
            for x in 4..8 {
                let base = (y * 8 + x) * 4;
                pixels[base..base + 4].copy_from_slice(&[60, 20, 80, 255]);
            }
        }
        let sprite = DefinitionSprite {
            graphics_scale: 1.0,
            image: ImageData::new(8, 4, pixels),
            actions: HashMap::from([(
                "O20".to_string(),
                DefinitionActionGraphics {
                    facet: Some(clonk_engine::DefinitionActionFacet {
                        x: 4,
                        y: 0,
                        width: 4,
                        height: 4,
                        target_x: 0,
                        target_y: 0,
                    }),
                    length: Some(1),
                    ..DefinitionActionGraphics::default()
                },
            )]),
            color_mask: None,
            // The left half of the sheet is fully transparent, so the base
            // face contributes nothing and only the overlay is asserted on.
            shape: Some(DefinitionRect::new(-2, -2, 4, 4)),
            fire_top: 0,
            rotateable: 0,
            line: 0,
            stretch_growth: false,
            top_face: None,
            picture: None,
        };

        let mut template = make_snapshot().objects.remove(0);
        template.crew_member = false;
        template.id = ObjectId::new(1);
        template.definition_id = "ParallaxHud".to_string();
        template.position = Vector2::new(10, 6);
        template.category = CATEGORY_PARALLAX_FLAG;
        template.graphics_overlays =
            vec![ObjectGraphicsOverlay::new(1, GraphicsOverlayMode::Action)
                .with_definition(Some("ParallaxHud".to_string()))
                .with_action(Some("O20".to_string()))];

        let render = |viewport: Vector2| {
            let mut graphics = GraphicsSystem::new(
                24,
                16,
                16,
                "Parallax action overlay",
                test_font(),
                Arc::new(HashMap::from([(
                    sprite_map_key("ParallaxHud", None),
                    sprite.clone(),
                )])),
                empty_cursor_atlas(),
                empty_hud_graphics(),
            );
            graphics.viewport_x = viewport.x as f32;
            graphics.viewport_y = viewport.y as f32;
            graphics.surface_mut().fill(Color::opaque(10, 10, 10));
            graphics.draw_objects(
                &[template.clone()],
                &[],
                &HashMap::new(),
                &[],
                4,
                1.0,
                &HashMap::new(),
                ObjectRenderPass::Normal,
                None,
            );
            graphics.surface().clone()
        };

        // A 4x4 facet centred on the object's own coordinates (10,6) covers
        // x 8..12, y 4..8 regardless of how far the landscape has scrolled.
        let marker = Some(Color::opaque(60, 20, 80));
        let background = Some(Color::opaque(10, 10, 10));

        let unscrolled = render(Vector2::new(0, 0));
        assert_eq!(unscrolled.get_pixel(8, 4), marker);
        assert_eq!(unscrolled.get_pixel(11, 7), marker);

        let scrolled = render(Vector2::new(5, 3));
        assert_eq!(
            scrolled.get_pixel(8, 4),
            marker,
            "TargetPos pins a zero-parallax overlay against viewport scroll"
        );
        assert_eq!(scrolled.get_pixel(11, 7), marker);
        assert_eq!(
            scrolled.get_pixel(3, 1),
            background,
            "the overlay must not slide by the raw scroll offset"
        );
    }

    #[test]
    fn nested_object_overlay_line_calls_use_rewritten_audibility_facets() {
        let mut template = make_snapshot().objects.remove(0);
        template.crew_member = false;

        let mut host = template.clone();
        host.id = ObjectId::new(1);
        host.definition_id = "AudibleOverlayHost".to_string();
        host.position = Vector2::new(50, 60);
        host.category = CATEGORY_PARALLAX_FLAG;
        host.local_vars
            .insert("__local_0".to_string(), clonk_script::Value::Int(50));
        host.local_vars
            .insert("__local_1".to_string(), clonk_script::Value::Int(25));
        host.graphics_overlays = vec![ObjectGraphicsOverlay::new(1, GraphicsOverlayMode::Object)
            .with_overlay_object(Some(ObjectId::new(2)))
            .with_transform(Some(DrawTransform::from_components(1.0, 1.0, 3.9, -2.9)))];

        let mut middle = template.clone();
        middle.id = ObjectId::new(2);
        middle.definition_id = "AudibleOverlayMiddle".to_string();
        middle.position = Vector2::new(100, 120);
        middle.category = CATEGORY_PARALLAX_FLAG;
        middle.visibility = clonk_engine::VIS_OVERLAY_ONLY;
        middle
            .local_vars
            .insert("__local_0".to_string(), clonk_script::Value::Int(75));
        middle
            .local_vars
            .insert("__local_1".to_string(), clonk_script::Value::Int(50));
        middle.graphics_overlays = vec![ObjectGraphicsOverlay::new(1, GraphicsOverlayMode::Object)
            .with_overlay_object(Some(ObjectId::new(3)))
            .with_transform(Some(DrawTransform::from_components(1.0, 1.0, -4.9, 5.9)))];

        let mut line = template;
        line.id = ObjectId::new(3);
        line.definition_id = "AudibleOverlayLine".to_string();
        line.position = Vector2::new(150, 160);
        line.category = CATEGORY_PARALLAX_FLAG;
        line.visibility = clonk_engine::VIS_OVERLAY_ONLY;
        line.vertices = vec![clonk_engine::ObjectVertex::new(200, 90)];
        line.local_vars
            .insert("__local_0".to_string(), clonk_script::Value::Int(25));
        line.local_vars
            .insert("__local_1".to_string(), clonk_script::Value::Int(100));
        line.graphics_overlays.clear();

        let sprite = |line| DefinitionSprite {
            graphics_scale: 1.0,
            image: ImageData::new(1, 1, vec![0, 0, 0, 0]),
            actions: HashMap::new(),
            color_mask: None,
            shape: Some(DefinitionRect::new(0, 0, 1, 1)),
            fire_top: 0,
            rotateable: 0,
            line,
            stretch_growth: false,
            top_face: None,
            picture: None,
        };
        let mut graphics = GraphicsSystem::new(
            160,
            100,
            100,
            "Nested overlay audibility",
            test_font(),
            Arc::new(HashMap::from([
                (sprite_map_key("AudibleOverlayHost", None), sprite(0)),
                (sprite_map_key("AudibleOverlayMiddle", None), sprite(0)),
                (sprite_map_key("AudibleOverlayLine", None), sprite(1)),
            ])),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics.viewport_x = 20.0;
        graphics.viewport_y = 30.0;
        graphics.content_audibility_facet = Some(AudibilityFacet {
            target_x: 20,
            target_y: 30,
            width: 80,
            height: 40,
        });
        graphics.draw_objects(
            &[host.clone(), middle.clone(), line.clone()],
            &[],
            &HashMap::new(),
            &[],
            OWNER_NONE,
            1.0,
            &HashMap::new(),
            ObjectRenderPass::Normal,
            None,
        );

        assert_eq!(
            graphics.rendered_object_audibility_calls().get(&line.id),
            Some(&vec![
                RenderedAudibilityCall::Parallax {
                    point: Vector2::new(200, 90),
                    rendered_center: Vector2::new(64, 89),
                },
                RenderedAudibilityCall::Parallax {
                    point: Vector2::new(200, 90),
                    rendered_center: Vector2::new(64, 89),
                },
            ]),
            "each nesting level rewrites TargetX/Y before the line target applies parallax",
        );
        assert!(
            !graphics
                .rendered_object_audibility_calls()
                .contains_key(&middle.id),
            "ODM_Overlay does not run the ordinary non-line parallax call",
        );
        assert_eq!(graphics.current_audibility_facet, None);
    }

    #[test]
    fn objects_txt_locals_restore_drives_cpp_parallax_target() {
        // C4Object::CompileFunc restores `Locals=` through C4ValueList before
        // ApplyParallaxity reads Local[0]/Local[1] (C4Object.cpp:2788,
        // 5800-5814; C4ValueList.cpp:102-136).
        let dir = tempfile::tempdir().expect("temporary scenario root");
        let definition = dir.path().join("Defs.c4d/Host.c4d");
        std::fs::create_dir_all(&definition).expect("definition directory");
        std::fs::write(
            definition.join("DefCore.txt"),
            "[DefCore]\nid=HOST\nName=Host\nCategory=1\nCrewMember=0\n",
        )
        .expect("definition core");
        std::fs::write(definition.join("Script.c"), "// host\n").expect("definition script");
        image::RgbaImage::from_pixel(1, 1, image::Rgba([1, 2, 3, 255]))
            .save(definition.join("Graphics.png"))
            .expect("definition graphics");

        let scenario_path = dir.path().join("ParallaxSave.c4s");
        std::fs::create_dir_all(&scenario_path).expect("scenario directory");
        std::fs::write(
            scenario_path.join("Scenario.txt"),
            "[Head]\nTitle=Parallax save\n\n[Definitions]\nDefinition1=Defs.c4d\n",
        )
        .expect("scenario core");
        std::fs::write(
            scenario_path.join("Objects.txt"),
            "[Object]\nid=HOST\nNumber=1\nStatus=1\nCategory=2097153\nX=50\nY=30\nLocals=2;i50,i25\n",
        )
        .expect("saved object");

        let scenario = Scenario::load_from_path_with(
            &scenario_path,
            &RepositoryContentResolver {
                root: dir.path().to_path_buf(),
            },
        )
        .expect("saved scenario loads");
        let mut engine = Engine::with_seed(0);
        scenario.apply(&mut engine).expect("saved scenario applies");
        let restored = engine
            .object_snapshot(ObjectId::new(1))
            .expect("saved parallax host restored");

        assert_eq!(
            restored
                .local_vars
                .get("__local_0")
                .and_then(|value| value.as_c4_int()),
            Some(50)
        );
        assert_eq!(
            restored
                .local_vars
                .get("__local_1")
                .and_then(|value| value.as_c4_int()),
            Some(25)
        );

        let mut graphics = test_graphics(80, 50, 50, "Restored parallax");
        graphics.viewport_x = 20.0;
        graphics.viewport_y = 20.0;
        assert_eq!(graphics.object_target_position(&restored), (10.0, 5.0));
    }

    #[test]
    fn shipped_star_definition_uses_additive_action_graphics() {
        // The real STAR definition declares BlitMode=1 and its Appear action
        // uses ten 3x3 frames. Phase four's opaque centre is grey 184.
        let definition = crate::test_support::repo_root()
            .join("content/Objects.c4d/Environment.c4d/Stars.c4d/Star.c4d");
        let def_core = std::fs::read_to_string(definition.join("DefCore.txt"))
            .expect("read shipped STAR DefCore");
        assert!(def_core.lines().any(|line| line.trim() == "BlitMode=1"));
        let rgba = image::open(definition.join("Graphics.png"))
            .expect("decode shipped STAR graphics")
            .into_rgba8();
        let (width, height) = rgba.dimensions();
        let sprite = DefinitionSprite {
            graphics_scale: 1.0,
            image: ImageData::new(width, height, rgba.into_raw()),
            actions: HashMap::from([(
                "Appear".to_string(),
                DefinitionActionGraphics {
                    facet: Some(clonk_engine::DefinitionActionFacet {
                        x: 0,
                        y: 0,
                        width: 3,
                        height: 3,
                        target_x: 0,
                        target_y: 0,
                    }),
                    length: Some(10),
                    ..DefinitionActionGraphics::default()
                },
            )]),
            color_mask: None,
            shape: Some(DefinitionRect::new(-2, -2, 4, 4)),
            fire_top: 0,
            rotateable: 0,
            line: 0,
            stretch_growth: false,
            top_face: None,
            picture: None,
        };
        let mut star = make_snapshot().objects.remove(0);
        star.definition_id = "STAR".to_string();
        star.position = Vector2::new(5, 5);
        star.action = clonk_engine::ActionState::new("Appear");
        star.action.phase = 4;
        star.blit_mode = 1;
        star.crew_member = false;
        let mut graphics = GraphicsSystem::new(
            10,
            10,
            10,
            "Shipped STAR additive",
            test_font(),
            Arc::new(HashMap::from([(sprite_map_key("STAR", None), sprite)])),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics.surface_mut().fill(Color::opaque(50, 60, 70));

        graphics.draw_objects(
            &[star],
            &[],
            &HashMap::new(),
            &[],
            OWNER_NONE,
            1.0,
            &HashMap::new(),
            ObjectRenderPass::Normal,
            None,
        );

        assert_eq!(
            graphics.surface().get_pixel(4, 4),
            Some(Color::opaque(234, 244, 254))
        );
    }

    #[test]
    fn object_mod2_modulates_base_action_top_and_rotated_faces() {
        // Object ColorMod is activated around both C4Object::Draw passes
        // (C4Object.cpp:2410-2499,2648-2672). Bit 2 selects BlitShaderMod2
        // for the main surface (StdDDraw2.cpp:768-770; StdGL.cpp:1072-1079).
        let source = Color::new(64, 128, 192, 128);
        let plain_sprite = |width, height, shape| DefinitionSprite {
            graphics_scale: 1.0,
            image: ImageData::new(
                width,
                height,
                (0..width * height)
                    .flat_map(|_| [source.r, source.g, source.b, source.a])
                    .collect(),
            ),
            actions: HashMap::new(),
            color_mask: None,
            shape: Some(shape),
            fire_top: 0,
            rotateable: 0,
            line: 0,
            stretch_growth: false,
            top_face: None,
            picture: None,
        };
        let mut sprites = HashMap::from([(
            sprite_map_key("BaseMod2", None),
            plain_sprite(1, 1, DefinitionRect::new(0, 0, 1, 1)),
        )]);
        let mut action = plain_sprite(1, 1, DefinitionRect::new(0, 0, 1, 1));
        action.actions.insert(
            "Active".to_string(),
            DefinitionActionGraphics {
                facet: Some(clonk_engine::DefinitionActionFacet {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1,
                    target_x: 0,
                    target_y: 0,
                }),
                length: Some(1),
                ..DefinitionActionGraphics::default()
            },
        );
        sprites.insert(sprite_map_key("ActionMod2", None), action);
        sprites.insert(
            sprite_map_key("TopMod2", None),
            DefinitionSprite {
                graphics_scale: 1.0,
                image: ImageData::new(
                    2,
                    1,
                    vec![0, 0, 0, 0, source.r, source.g, source.b, source.a],
                ),
                actions: HashMap::new(),
                color_mask: None,
                shape: Some(DefinitionRect::new(0, 0, 1, 1)),
                fire_top: 0,
                rotateable: 0,
                line: 0,
                stretch_growth: false,
                top_face: Some(DefinitionTargetRect::new(1, 0, 1, 1, 0, 0)),
                picture: None,
            },
        );
        sprites.insert(
            sprite_map_key("RotatedMod2", None),
            plain_sprite(3, 3, DefinitionRect::new(-1, -1, 3, 3)),
        );

        let template = make_snapshot().objects.remove(0);
        let make_object = |id, definition: &str, position, action: &str, rotation| {
            let mut object = template.clone();
            object.id = ObjectId::new(id);
            object.definition_id = definition.to_string();
            object.position = position;
            object.action = clonk_engine::ActionState::new(action);
            object.rotation = rotation;
            object.blit_mode = 2;
            object.color_modulation = 0x0020_4080;
            object.crew_member = false;
            object
        };
        let objects = vec![
            make_object(1, "BaseMod2", Vector2::new(1, 2), "Idle", 0),
            make_object(2, "ActionMod2", Vector2::new(3, 2), "Active", 0),
            make_object(3, "TopMod2", Vector2::new(5, 2), "Idle", 0),
            make_object(4, "RotatedMod2", Vector2::new(8, 2), "Idle", 45),
        ];
        let mut graphics = GraphicsSystem::new(
            11,
            5,
            5,
            "Object MOD2 routes",
            test_font(),
            Arc::new(sprites),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics.surface_mut().fill(Color::opaque(200, 200, 200));

        graphics.draw_objects(
            &objects,
            &[],
            &HashMap::new(),
            &[],
            OWNER_NONE,
            1.0,
            &HashMap::new(),
            ObjectRenderPass::Normal,
            None,
        );

        // Shader MOD2 source: clamp(2*src + 2*mod - 255) = (0,129,255),
        // then ordinary source-alpha over the framebuffer.
        let expected = Some(Color::opaque(100, 164, 228));
        for (route, x) in [("base", 1), ("action", 3), ("top", 5), ("rotated", 8)] {
            assert_eq!(graphics.surface().get_pixel(x, 2), expected, "{route}");
        }
    }

    #[test]
    fn object_mod2_black_reset_and_additive_gamma_precedence_match_stdgl() {
        // PerformBlt resets MOD2 when the active modulation is all black
        // (StdGL.cpp:442-472), yielding a normal black silhouette. Additive
        // remains an independent framebuffer blend bit (StdGL.cpp:1320-1324).
        let mut sprites = HashMap::new();
        sprites.insert(
            sprite_map_key("BlackMod2", None),
            DefinitionSprite {
                graphics_scale: 1.0,
                image: ImageData::new(1, 1, vec![200, 200, 200, 255]),
                actions: HashMap::new(),
                color_mask: None,
                shape: Some(DefinitionRect::new(0, 0, 1, 1)),
                fire_top: 0,
                rotateable: 0,
                line: 0,
                stretch_growth: false,
                top_face: None,
                picture: None,
            },
        );
        sprites.insert(
            sprite_map_key("AddMod2", None),
            DefinitionSprite {
                graphics_scale: 1.0,
                image: ImageData::new(1, 1, vec![64, 128, 192, 128]),
                actions: HashMap::new(),
                color_mask: None,
                shape: Some(DefinitionRect::new(0, 0, 1, 1)),
                fire_top: 0,
                rotateable: 0,
                line: 0,
                stretch_growth: false,
                top_face: None,
                picture: None,
            },
        );
        let template = make_snapshot().objects.remove(0);
        let mut black = template.clone();
        black.definition_id = "BlackMod2".to_string();
        black.position = Vector2::new(1, 1);
        black.blit_mode = 2;
        black.color_modulation = 0;
        black.crew_member = false;
        let mut combined = template;
        combined.id = ObjectId::new(2);
        combined.definition_id = "AddMod2".to_string();
        combined.position = Vector2::new(3, 1);
        combined.blit_mode = 1 | 2 | 128;
        combined.color_modulation = 0x0020_4080;
        combined.crew_member = false;
        let gamma = clonk_graphics::GammaRamp::from_control_points([0x000000, 0x646464, 0xc8c8c8]);
        let mut graphics = GraphicsSystem::new(
            5,
            3,
            3,
            "MOD2 precedence",
            test_font(),
            Arc::new(sprites),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics.surface_mut().fill(Color::opaque(100, 100, 100));

        graphics.draw_objects(
            &[black, combined],
            &[],
            &HashMap::new(),
            &[],
            OWNER_NONE,
            1.0,
            &HashMap::new(),
            ObjectRenderPass::Normal,
            Some(&gamma),
        );

        assert_eq!(
            graphics.surface().get_pixel(1, 1),
            Some(gamma_encode_fragment(Color::opaque(0, 0, 0), &gamma))
        );
        let modulated = [0.0, 129.0, 255.0];
        let alpha = 128.0 / 255.0;
        let expected = Color::opaque(
            store_channel(
                100.0
                    + sample_channel(
                        Some(&gamma),
                        clonk_graphics::gamma::GammaChannel::Red,
                        modulated[0],
                    ) * alpha,
            ),
            store_channel(
                100.0
                    + sample_channel(
                        Some(&gamma),
                        clonk_graphics::gamma::GammaChannel::Green,
                        modulated[1],
                    ) * alpha,
            ),
            store_channel(
                100.0
                    + sample_channel(
                        Some(&gamma),
                        clonk_graphics::gamma::GammaChannel::Blue,
                        modulated[2],
                    ) * alpha,
            ),
        );
        assert_eq!(graphics.surface().get_pixel(3, 1), Some(expected));
    }

    #[test]
    fn color_by_owner_bits_four_and_eight_have_distinct_source_modulation() {
        // Base and owner surfaces are separate C++ passes. Bit 4 keeps the
        // owner's raw color independent of global ColorMod; bit 8 selects
        // MOD2 only for the grey owner surface (StdDDraw2.cpp:768-778).
        let sprite = DefinitionSprite {
            graphics_scale: 1.0,
            image: ImageData::new(1, 1, vec![255, 255, 255, 255]),
            actions: HashMap::new(),
            color_mask: Some(ColorByOwnerMask::new(1, 1, Arc::from([64]))),
            shape: Some(DefinitionRect::new(0, 0, 1, 1)),
            fire_top: 0,
            rotateable: 0,
            line: 0,
            stretch_growth: false,
            top_face: None,
            picture: None,
        };
        let owner = Color::opaque(64, 128, 192);
        let render = |mode| {
            let mut object = make_snapshot().objects.remove(0);
            object.definition_id = "OwnerModes".to_string();
            object.position = Vector2::new(1, 1);
            object.blit_mode = mode;
            object.color = 0x0040_80c0;
            object.color_modulation = 0x0080_4020;
            object.crew_member = false;
            let mut graphics = GraphicsSystem::new(
                3,
                3,
                3,
                "Owner modulation modes",
                test_font(),
                Arc::new(HashMap::from([(
                    sprite_map_key("OwnerModes", None),
                    sprite.clone(),
                )])),
                empty_cursor_atlas(),
                empty_hud_graphics(),
            );
            graphics.surface_mut().fill(Color::opaque(9, 11, 13));
            graphics.draw_objects(
                &[object],
                &[],
                &HashMap::new(),
                &[],
                OWNER_NONE,
                1.0,
                &HashMap::from([(0, owner)]),
                ObjectRenderPass::Normal,
                None,
            );
            graphics.surface().get_pixel(1, 1)
        };

        // owner ⊗ global is (32,32,24) by C++'s >>8 combine. The owner
        // texture is grey 64. Bit 8's normalized shader formula is
        // clamp(2*grey + 2*mod - 255), proving it is not bit-2 aliasing.
        assert_eq!(render(0), Some(Color::opaque(8, 8, 6)));
        assert_eq!(render(4), Some(Color::opaque(16, 32, 48)));
        assert_eq!(render(8), Some(Color::opaque(0, 0, 0)));
        assert_eq!(render(4 | 8), Some(Color::opaque(1, 129, 255)));
    }

    #[test]
    fn overlay_mod2_uses_local_modulation_or_exact_parent_state() {
        // Explicit overlays activate modulation only when their color differs
        // from 0x00ffffff (C4DefGraphics.cpp:762-768). Thus mode 2 + default
        // white is MOD2-to-white, while explicit black triggers the PerformBlt
        // black reset. Exact parent mode inherits both mode and ColorMod.
        let sprite = DefinitionSprite {
            graphics_scale: 1.0,
            image: ImageData::new(3, 3, [64, 128, 192, 255].repeat(9)),
            actions: HashMap::new(),
            color_mask: None,
            shape: Some(DefinitionRect::new(-1, -1, 3, 3)),
            fire_top: 0,
            rotateable: 0,
            line: 0,
            stretch_growth: false,
            top_face: None,
            picture: None,
        };
        let render = |overlay_mode, overlay_modulation, rotation| {
            let mut object = make_snapshot().objects.remove(0);
            object.position = Vector2::new(2, 2);
            object.blit_mode = 2;
            object.color_modulation = 0x0020_4080;
            let mut overlay = ObjectGraphicsOverlay::new(1, GraphicsOverlayMode::Base)
                .with_definition(Some("OverlayMod2".to_string()))
                .with_blit_mode(overlay_mode);
            overlay.color_modulation = overlay_modulation;
            object.graphics_overlays = vec![overlay];
            let mut graphics = GraphicsSystem::new(
                5,
                5,
                5,
                "Overlay MOD2",
                test_font(),
                Arc::new(HashMap::from([(
                    sprite_map_key("OverlayMod2", None),
                    sprite.clone(),
                )])),
                empty_cursor_atlas(),
                empty_hud_graphics(),
            );
            graphics.surface_mut().fill(Color::opaque(9, 11, 13));
            graphics.draw_object_overlays(
                &object,
                &[],
                &[],
                OWNER_NONE,
                None,
                2.0,
                2.0,
                1.0,
                rotation,
                None,
                None,
            );
            graphics.surface().get_pixel(2, 2)
        };

        assert_eq!(
            render(2, 0x00ff_ffff, 0.0),
            Some(Color::opaque(255, 255, 255))
        );
        assert_eq!(render(2, 0, 0.0), Some(Color::opaque(0, 0, 0)));
        assert_eq!(
            render(256, 0x00ff_ffff, 45.0),
            Some(Color::opaque(0, 129, 255))
        );
        assert_eq!(render(0, 0x0020_4080, 0.0), Some(Color::opaque(8, 32, 96)));
    }

    #[test]
    fn shipped_firelump_uses_mod2_color_modulation() {
        // FRBL declares BlitMode=2; Existing() continuously assigns
        // SetClrModulation(RGB(iR,iG,64)). Use one real sheet texel from its
        // base face to pin shipped MOD2 behavior.
        let definition = crate::test_support::repo_root()
            .join("content/Fantasy.c4d/Magic.c4d/Firelump.c4d/Fball.c4d");
        let def_core = std::fs::read_to_string(definition.join("DefCore.txt"))
            .expect("read shipped FRBL DefCore");
        assert!(def_core.lines().any(|line| line.trim() == "BlitMode=2"));
        let script = std::fs::read(definition.join("Script.c")).expect("read shipped FRBL Script");
        assert!(script
            .windows(b"SetClrModulation(RGB(iR,iG,64))".len())
            .any(|window| window == b"SetClrModulation(RGB(iR,iG,64))"));
        let rgba = image::open(definition.join("Graphics.png"))
            .expect("decode shipped FRBL graphics")
            .into_rgba8();
        let (width, height) = rgba.dimensions();
        let sprite = DefinitionSprite {
            graphics_scale: 1.0,
            image: ImageData::new(width, height, rgba.into_raw()),
            actions: HashMap::from([(
                "Exist".to_string(),
                DefinitionActionGraphics {
                    facet_base: true,
                    ..DefinitionActionGraphics::default()
                },
            )]),
            color_mask: None,
            shape: Some(DefinitionRect::new(-5, -5, 10, 10)),
            fire_top: 0,
            rotateable: 0,
            line: 0,
            stretch_growth: false,
            top_face: None,
            picture: None,
        };
        let mut firelump = make_snapshot().objects.remove(0);
        firelump.definition_id = "FRBL".to_string();
        firelump.position = Vector2::new(10, 10);
        firelump.action = clonk_engine::ActionState::new("Exist");
        firelump.blit_mode = 2;
        firelump.color_modulation = 0x0018_2040;
        firelump.crew_member = false;
        let mut graphics = GraphicsSystem::new(
            20,
            20,
            20,
            "Shipped FRBL MOD2",
            test_font(),
            Arc::new(HashMap::from([(sprite_map_key("FRBL", None), sprite)])),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics.surface_mut().fill(Color::opaque(50, 60, 70));

        graphics.draw_objects(
            &[firelump],
            &[],
            &HashMap::new(),
            &[],
            OWNER_NONE,
            1.0,
            &HashMap::new(),
            ObjectRenderPass::Normal,
            None,
        );

        // Graphics.png (6,0) = (255,140,0,179). MOD2 with (24,32,64)
        // produces (255,89,0), then alpha-over gives this framebuffer value.
        assert_eq!(
            graphics.surface().get_pixel(11, 5),
            Some(Color::opaque(194, 80, 21))
        );
    }

    #[test]
    fn object_and_old_style_pxs_gamma_sample_independent_r16_channels() {
        let gamma = clonk_graphics::GammaRamp::from_control_points([0x102030, 0x405060, 0x708090]);
        let snapshot = make_snapshot();
        let mut graphics = GraphicsSystem::new(
            128,
            128,
            128,
            "Gamma Object/PXS",
            test_font(),
            solid_sprite(
                "TestObject",
                1,
                1,
                Color::opaque(0, 0, 0),
                Some(DefinitionRect::new(0, 0, 1, 1)),
                false,
            ),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics.surface_mut().fill(Color::opaque(200, 200, 200));
        graphics.set_material_render_info(Arc::new(HashMap::from([(
            "rain".to_string(),
            MaterialRenderInfo::new([0; 9], [0; 6], None, 0, 25),
        )])));

        graphics.draw_pxs(
            &[pxs_particle("rain", [96 << 16, 100 << 16, 0, 0], 0)],
            1.0,
            Some(&gamma),
        );
        graphics.draw_objects(
            &snapshot.objects,
            &snapshot.render_order,
            &snapshot.definition_lines,
            &snapshot.players,
            OWNER_NONE,
            1.0,
            &HashMap::new(),
            ObjectRenderPass::Normal,
            Some(&gamma),
        );

        let encoded = Some(Color::new(17, 33, 49, 255));
        assert_eq!(graphics.surface().get_pixel(96, 100), encoded);
        assert_eq!(graphics.surface().get_pixel(100, 100), encoded);
    }

    #[test]
    fn rotated_base_overlay_gamma_samples_before_translucent_blending() {
        let mut object = make_snapshot().objects.remove(0);
        let mut overlay = ObjectGraphicsOverlay::new(1, GraphicsOverlayMode::Base);
        overlay.definition = Some("Overlay".to_string());
        object.graphics_overlays.push(overlay);
        let mut graphics = GraphicsSystem::new(
            9,
            9,
            9,
            "Gamma Rotated Overlay",
            test_font(),
            solid_sprite(
                "Overlay",
                3,
                3,
                Color::new(64, 128, 192, 128),
                Some(DefinitionRect::new(-1, -1, 3, 3)),
                false,
            ),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics.surface_mut().fill(Color::opaque(200, 200, 200));
        let gamma = clonk_graphics::GammaRamp::from_control_points([0x000000, 0x646464, 0xc8c8c8]);

        graphics.draw_object_overlays(
            &object,
            &[],
            &[],
            OWNER_NONE,
            None,
            4.0,
            4.0,
            1.0,
            45.0,
            None,
            Some(&gamma),
        );

        assert_eq!(
            graphics.surface().get_pixel(4, 4),
            Some(Color::new(125, 150, 175, 255))
        );
    }

    #[test]
    fn graphical_pxs_gamma_samples_filtered_rgb_before_translucent_blending() {
        let mut surface = Surface::new(1, 1, PixelFormat::Rgba8888);
        surface.fill(Color::opaque(200, 200, 200));
        let image = ImageData::new(1, 1, vec![64, 128, 192, 128]);
        let gamma = clonk_graphics::GammaRamp::from_control_points([0x000000, 0x646464, 0xc8c8c8]);

        draw_pxs_image_region(
            &mut surface,
            &GuiRect::new(0.0, 0.0, 1.0, 1.0),
            &image,
            &SourceRect::new(0, 0, 1, 1),
            0,
            1.0,
            AdvancedRendererConfig::DEFAULT,
            Some(&gamma),
            None,
        );

        assert_eq!(
            surface.get_pixel(0, 0),
            Some(Color::new(125, 150, 175, 255))
        );
    }

    #[test]
    fn gpu_capture_retains_graphical_pxs_as_linear_gamma_quad() {
        let mut surface = Surface::new(4, 4, PixelFormat::Rgba8888);
        surface.begin_gpu_scene_capture();
        let image = ImageData::new(2, 2, [64, 128, 192, 128].repeat(4));
        let gamma = clonk_graphics::GammaRamp::from_control_points([0x000000, 0x646464, 0xc8c8c8]);

        draw_pxs_image_region(
            &mut surface,
            &GuiRect::new(0.0, 0.0, 4.0, 4.0),
            &image,
            &SourceRect::new(0, 0, 2, 2),
            16,
            1.0,
            AdvancedRendererConfig::DEFAULT,
            Some(&gamma),
            None,
        );

        let scene = surface
            .take_gpu_scene_capture()
            .expect("GPU capture remains active")
            .into_scene([4, 4], Color::transparent(), &gamma);
        let GpuCommand::Quad {
            sampler,
            blend,
            gamma,
            ..
        } = &scene.commands[0]
        else {
            panic!("graphical PXS did not lower to a textured quad");
        };
        assert_eq!(*sampler, GpuSampler::Linear);
        assert_eq!(*blend, GpuBlend::Normal);
        assert!(*gamma);
    }

    #[test]
    fn graphical_pxs_honors_advanced_renderer_snapshot() {
        let alpha_result = |shader, no_alpha_add| {
            let mut surface = Surface::new(1, 1, PixelFormat::Rgba8888);
            surface.fill(Color::opaque(0, 0, 0));
            draw_pxs_image_region(
                &mut surface,
                &GuiRect::new(0.0, 0.0, 1.0, 1.0),
                &ImageData::new(1, 1, vec![200, 100, 50, 192]),
                &SourceRect::new(0, 0, 1, 1),
                64,
                1.0,
                AdvancedRendererConfig {
                    shader,
                    no_alpha_add,
                    ..AdvancedRendererConfig::DEFAULT
                },
                None,
                None,
            );
            surface.get_pixel(0, 0).unwrap()
        };
        assert_eq!(alpha_result(false, false), Color::opaque(100, 50, 25));
        assert_eq!(alpha_result(false, true), Color::opaque(151, 75, 38));
        assert_eq!(alpha_result(true, true), Color::opaque(100, 50, 25));

        let sentinel = Color::opaque(7, 11, 13);
        let image = ImageData::new(1, 1, vec![255, 0, 0, 255]);
        let mut shifted = Surface::new(3, 3, PixelFormat::Rgba8888);
        shifted.fill(sentinel);
        draw_pxs_image_region(
            &mut shifted,
            &GuiRect::new(0.0, 0.0, 1.0, 1.0),
            &image,
            &SourceRect::new(0, 0, 1, 1),
            0,
            1.0,
            AdvancedRendererConfig {
                blit_offset: 100,
                allowed_blit_modes: 0,
                ..AdvancedRendererConfig::DEFAULT
            },
            None,
            None,
        );
        assert_eq!(shifted.get_pixel(0, 0), Some(sentinel));
        assert_eq!(shifted.get_pixel(1, 1), Some(Color::opaque(255, 0, 0)));

        let pixels = (0..8)
            .flat_map(|_| {
                (0..8).flat_map(|column| {
                    let value = column * 32;
                    [value, value, value, 255]
                })
            })
            .collect();
        let cropped = ImageData::new(8, 8, pixels);
        let sample_crop = |tex_indent| {
            let mut surface = Surface::new(4, 1, PixelFormat::Rgba8888);
            draw_pxs_image_region(
                &mut surface,
                &GuiRect::new(0.0, 0.0, 4.0, 1.0),
                &cropped,
                &SourceRect::new(2, 0, 4, 1),
                0,
                1.0,
                AdvancedRendererConfig {
                    tex_indent,
                    ..AdvancedRendererConfig::DEFAULT
                },
                None,
                None,
            );
            (0..4)
                .map(|x| surface.get_pixel(x, 0).unwrap().r)
                .collect::<Vec<_>>()
        };
        assert_eq!(sample_crop(0), vec![64, 96, 128, 160]);
        assert_eq!(sample_crop(500), vec![78, 107, 135, 164]);

        let fog = FogDrawContext {
            map: Arc::new(ClrModMap {
                resolution_x: 64,
                resolution_y: 64,
                width: 2,
                height: 2,
                origin_x: 0,
                origin_y: 0,
                fade_transparent: false,
                cells: vec![0x0040_4040, 0x0080_8080, 0x00c0_c0c0, 0x00ff_ffff],
            }),
            zoom: 1.0,
        };
        let white = ImageData::new(64, 64, vec![255; 64 * 64 * 4]);
        let fog_result = |no_box_fades| {
            let mut surface = Surface::new(64, 64, PixelFormat::Rgba8888);
            draw_pxs_image_region(
                &mut surface,
                &GuiRect::new(0.0, 0.0, 64.0, 64.0),
                &white,
                &SourceRect::new(0, 0, 64, 64),
                0,
                1.0,
                AdvancedRendererConfig {
                    no_box_fades,
                    ..AdvancedRendererConfig::DEFAULT
                },
                None,
                Some(&fog),
            );
            surface.get_pixel(0, 0).unwrap().r
        };
        assert_eq!(fog_result(false), 65);
        assert_eq!(fog_result(true), 191);
    }

    #[test]
    fn tutorial_seven_acid_rain_pxs_uses_its_green_gamma_ramp() {
        // Tutorial07 Script.c:12 and AcidRain.c4m:3: the opaque old-style
        // PXS fragment (200,250,200) is sampled by the scenario's green-heavy
        // ramp before it replaces the framebuffer pixel.
        let mut graphics = test_graphics(4, 4, 4, "Tutorial 07 Acid Rain");
        let background = Color::opaque(7, 11, 13);
        graphics.surface_mut().fill(background);
        graphics.set_material_render_info(Arc::new(HashMap::from([(
            "acidrain".to_string(),
            MaterialRenderInfo::new(
                [200, 250, 200, 200, 250, 200, 200, 250, 200],
                [0; 6],
                None,
                0,
                25,
            ),
        )])));
        let gamma = clonk_graphics::GammaRamp::from_control_points([0x000000, 0x648064, 0xc8ffc8]);

        graphics.draw_pxs(
            &[pxs_particle("acidrain", [2 << 16, 2 << 16, 0, 0], 0)],
            1.0,
            Some(&gamma),
        );

        assert_eq!(
            graphics.surface().get_pixel(2, 2),
            Some(Color::opaque(157, 250, 157))
        );
        assert_eq!(graphics.surface().get_pixel(1, 2), Some(background));
    }

    #[test]
    fn real_tutorial_seven_apply_gamma_now_replaces_reused_menu_gamma() {
        // Tutorial07 Initialize installs this ramp before the first game
        // render (Tutorial07.c4s/Script.c:12; C4Game.cpp:490), and its shipped
        // AcidRain material supplies opaque old-style PXS colour 200,250,200
        // (AcidRain.c4m:3). C4PXS::Draw emits that fragment through the active
        // shader gamma textures (C4PXS.cpp:242-277; StdGL.cpp:1082-1087).
        let tutorial = crate::test_support::repo_root().join("content/Tutorial.c4f/Tutorial07.c4s");
        let script = std::fs::read_to_string(tutorial.join("Script.c"))
            .expect("read shipped Tutorial07 Script.c");
        let gamma_values = script
            .lines()
            .find(|line| line.contains("SetGamma("))
            .expect("shipped Tutorial07 sets gamma")
            .split(|character: char| !character.is_ascii_digit())
            .filter(|value| !value.is_empty())
            .map(|value| value.parse::<u32>().expect("Tutorial07 gamma channel"))
            .collect::<Vec<_>>();
        assert_eq!(gamma_values.len(), 9);
        let rgb = |offset: usize| {
            (gamma_values[offset] << 16)
                | (gamma_values[offset + 1] << 8)
                | gamma_values[offset + 2]
        };

        let material = std::fs::read_to_string(tutorial.join("Material.c4g/AcidRain.c4m"))
            .expect("read shipped Tutorial07 AcidRain material");
        let material_color = material
            .lines()
            .find_map(|line| line.strip_prefix("Color="))
            .expect("shipped AcidRain material color")
            .split(',')
            .map(|value| value.parse::<u8>().expect("AcidRain color channel"))
            .collect::<Vec<_>>()
            .try_into()
            .expect("AcidRain has three RGB triplets");

        let mut snapshot = make_snapshot();
        snapshot
            .environment
            .gamma
            .set_ramp(0, [rgb(0), rgb(3), rgb(6)]);
        snapshot
            .particles
            .push(pxs_particle("acidrain", [100 << 16, 60 << 16, 0, 0], 0));
        let mut graphics = test_graphics(120, 100, 100, "Tutorial 07 Acid Rain");
        graphics.set_material_render_info(Arc::new(HashMap::from([(
            "acidrain".to_string(),
            MaterialRenderInfo::new(material_color, [0; 6], None, 0, 25),
        )])));

        let menu_snapshot = make_snapshot();
        graphics.render_frame(
            &menu_snapshot,
            &[ViewportInput::from_focus(&menu_snapshot.objects[0])],
        );
        graphics.apply_gamma_now(&snapshot.environment.gamma);
        graphics.render_frame(
            &snapshot,
            &[ViewportInput::from_focus(&snapshot.objects[0])],
        );
        let (x, y) = graphics
            .world_to_screen(0, Vector2::new(100, 60))
            .expect("acid-rain point is in the Tutorial07 viewport");

        assert_eq!(
            graphics
                .surface()
                .get_pixel(x.round() as u32, y.round() as u32),
            Some(Color::opaque(157, 250, 157)),
        );
    }

    #[test]
    #[cfg_attr(
        not(target_os = "macos"),
        ignore = "recording-host material order; required macOS CI job"
    )]
    fn real_tutorial_seven_acid_rain_matches_cpp_animated_pxs_sequence() {
        // Tutorial07 fixes rain at 77 and wind at 50 and selects AcidRain
        // (Scenario.txt:70-75). FXP1's Process action calls Precipitation
        // every two frames and that callback inserts three PXS at strength 77
        // (Precipitation.c4d/ActMap.txt:1-8; Script.c:5-17). ExecObjects runs
        // before PXS.Execute, so each new triplet moves on its creation frame
        // (C4Game.cpp:808-835; C4PXS.cpp:218-239). AcidRain has no PXSGfx:
        // C++ therefore draws a gamma-shaded, half-open velocity line from
        // x-xdir/y-ydir to x/y (C4PXS.cpp:242-277; StdGL.cpp:893-933,
        // 1082-1087). Pin ten actual frames, not merely spawn counts.
        let mut engine = load_repository_tutorial(7);
        let material_group = Group::open(
            crate::test_support::repo_root()
                .join("content/Tutorial.c4f/Tutorial07.c4s/Material.c4g"),
        )
        .expect("Tutorial07 Material.c4g opens");
        let materials =
            MaterialLibrary::from_group(&material_group).expect("Tutorial07 materials load");
        let acid = materials.get("AcidRain").expect("AcidRain material");
        let color: [u8; 9] = acid
            .int_list("Color")
            .expect("AcidRain color")
            .into_iter()
            .map(|value| value as u8)
            .collect::<Vec<_>>()
            .try_into()
            .expect("AcidRain has three RGB triplets");
        assert!(acid.value("PXSGfx").is_none());
        assert_eq!(color, [200, 250, 200, 200, 250, 200, 200, 250, 200]);
        assert_eq!(acid.int("Density"), Some(25));
        let material = MaterialRenderInfo::new(
            color,
            [0; 6],
            acid.value("TextureOverlay").map(ToOwned::to_owned),
            acid.int("OverlayType").unwrap_or(0),
            acid.int("Density").unwrap_or(0),
        );

        // The burned TRB1/TRB2 trees include TREE's Construction callback but
        // have no Initialize action. C++ SetAction therefore returns false and
        // skips TREE's SetDir(Random(2)), which is observable in this synced
        // precipitation ledger (Tree.c4d/Script.c:20-30;
        // C4Object.cpp:4218-4234).
        let expected_frames = [
            (0, 0xdbdc_9dc5, 0, None),
            (3, 0x0bad_19e8, 21, Some((62, 1, 217, 7))),
            (3, 0x6c8a_4955, 24, Some((63, 8, 217, 15))),
            (6, 0x31b1_5c65, 42, Some((63, 1, 218, 22))),
            (6, 0xddd7_6c00, 45, Some((64, 8, 219, 29))),
            (9, 0xaecd_41a0, 63, Some((65, 1, 220, 36))),
            (9, 0x378c_d1f5, 68, Some((66, 8, 221, 43))),
            (12, 0xdeb1_9f59, 84, Some((67, 1, 222, 50))),
            (12, 0x5669_9b94, 87, Some((68, 8, 224, 57))),
            (15, 0x5549_b99d, 106, Some((69, 1, 225, 64))),
        ];
        let expected_first_particle = [
            [4_102_875, 552_373, 39_643, 486_837],
            [4_151_777, 1_033_657, 48_902, 481_284],
            [4_208_380, 1_504_401, 56_603, 470_744],
            [4_270_101, 1_973_587, 61_721, 469_186],
            [4_334_985, 2_436_830, 64_884, 463_243],
            [4_400_655, 2_899_856, 65_670, 463_026],
            [4_471_515, 3_362_274, 70_860, 462_418],
            [4_549_622, 3_824_705, 78_107, 462_431],
            [4_631_974, 4_279_441, 82_352, 454_736],
        ];

        for (frame_index, &(expected_count, checksum, changed_count, expected_bounds)) in
            expected_frames.iter().enumerate()
        {
            let snapshot = engine.tick().expect("Tutorial07 weather frame");
            let pxs = snapshot
                .particles
                .iter()
                .filter(|particle| particle.definition_id == "material/pxs/acidrain")
                .collect::<Vec<_>>();
            assert_eq!(
                pxs.len(),
                expected_count,
                "frame {} PXS cadence",
                snapshot.frame
            );
            assert_eq!(
                pxs.iter()
                    .map(|particle| particle.pxs_slot)
                    .collect::<Vec<_>>(),
                (0..expected_count as u32).map(Some).collect::<Vec<_>>(),
                "frame {} preserves C4PXS slot order",
                snapshot.frame,
            );
            if let Some(first) = pxs.first() {
                assert_eq!(
                    first.pxs_fixed,
                    Some(expected_first_particle[frame_index - 1]),
                    "frame {} first AcidRain PXS trajectory",
                    snapshot.frame,
                );
            }
            let mut graphics = test_graphics(1024, 256, 256, "Tutorial 07 Acid Rain");
            graphics.surface_mut().fill(Color::opaque(7, 11, 13));
            graphics.set_material_render_info(Arc::new(HashMap::from([(
                "acidrain".to_string(),
                material.clone(),
            )])));
            let gamma_points = snapshot.environment.gamma.combined_control_points();
            assert_eq!(gamma_points, [0x000000, 0x648064, 0xc8ffc8]);
            let gamma = clonk_graphics::GammaRamp::from_control_points(gamma_points);
            graphics.draw_pxs(
                &snapshot.particles,
                GraphicsSystem::lighting_factor(snapshot.environment.settings.time_of_day),
                Some(&gamma),
            );
            let background = Color::opaque(7, 11, 13);
            let changed = (0..graphics.surface().height())
                .flat_map(|y| (0..graphics.surface().width()).map(move |x| (x, y)))
                .filter(|&(x, y)| graphics.surface().get_pixel(x, y) != Some(background))
                .collect::<Vec<_>>();
            let bounds = changed.iter().fold(None, |bounds, &(x, y)| {
                Some(
                    bounds.map_or((x, y, x, y), |(x0, y0, x1, y1): (u32, u32, u32, u32)| {
                        (x0.min(x), y0.min(y), x1.max(x), y1.max(y))
                    }),
                )
            });
            assert_eq!(
                graphics.surface().snapshot().checksum(),
                checksum,
                "frame {} rendered PXS streaks",
                snapshot.frame,
            );
            assert_eq!(
                (changed.len(), bounds),
                (changed_count, expected_bounds),
                "frame {} rendered PXS coverage",
                snapshot.frame,
            );
        }
    }

    #[test]
    fn real_tutorial_seven_acid_landscape_matches_cpp_material_color() {
        // The shipped slot is Acid-Smooth, but C4TexMapEntry changes every
        // liquid <mat>-Smooth primary pattern to Liquid. Acid's own Liquid
        // overlay is then sampled at zoom two (C4Texture.cpp:68-99;
        // C4Material.cpp:349-377), after which Tutorial07's three-channel
        // gamma ramp is applied by the landscape shader (StdGL.cpp:1130-1148).
        // The optional liquid-animation tint is off by C++ default
        // (C4Config.cpp:451).
        let mut engine = load_repository_tutorial(7);
        let snapshot = engine.tick().expect("Tutorial07 first frame");
        let grid = snapshot
            .landscape
            .as_ref()
            .and_then(Landscape::pixel_grid)
            .expect("Tutorial07 pixel landscape");

        let local_material_group = Group::open(
            crate::test_support::repo_root()
                .join("content/Tutorial.c4f/Tutorial07.c4s/Material.c4g"),
        )
        .expect("Tutorial07 Material.c4g opens");
        let texmap_source = local_material_group
            .read_file("Texmap.txt")
            .expect("Tutorial07 Texmap.txt reads");
        let texmap = clonk_resources::texmap::TextureMap::parse_bytes(&texmap_source);
        let acid_slot = texmap.entry(22).expect("Tutorial07 Acid texmap slot");
        assert_eq!(
            (acid_slot.material.as_str(), acid_slot.texture.as_str()),
            ("Acid", "Smooth"),
        );
        assert_eq!(grid.material_names()[22].as_deref(), Some("Acid"));
        assert_eq!(grid.texture_names()[22].as_deref(), Some("Liquid"));

        // C4Landscape::ApplyLighting shades material edges from Placement
        // (C4Landscape.cpp:2534-2588). This real 16x16 interior plus its
        // complete x/y comparison neighbourhood is Acid (Placement=10), so
        // C++ applies neither edge lightening nor darkening here.
        let acid_at = |x: i32, y: i32| {
            grid.byte_at(x, y)
                .and_then(|byte| grid.material_names().get((byte & 0x7f) as usize))
                .and_then(|name| name.as_deref())
                .is_some_and(|name| name.eq_ignore_ascii_case("Acid"))
        };
        assert!((0..16).all(|dy| {
            (0..16).all(|dx| {
                let x = 196 + dx;
                let y = 349 + dy;
                acid_at(x - 1, y)
                    && acid_at(x + 1, y)
                    && (-9..=8).all(|offset| acid_at(x, y + offset))
            })
        }));

        let global_material_group =
            Group::open(crate::test_support::repo_root().join("content/Material.c4g"))
                .expect("installed Material.c4g opens");
        let global_materials =
            MaterialLibrary::from_group(&global_material_group).expect("installed materials load");
        let acid = global_materials.get("Acid").expect("Acid material");
        let mut color = [0u8; 9];
        for (target, source) in color
            .iter_mut()
            .zip(acid.int_list("Color").expect("Acid Color"))
        {
            *target = source as u8;
        }
        assert_eq!(color, [0, 190, 0, 0, 200, 0, 0, 210, 0]);
        assert_eq!(acid.value("TextureOverlay"), Some("Liquid"));
        assert_eq!(
            (acid.int("Density"), acid.int("Placement")),
            (Some(25), Some(10)),
        );
        let resource =
            clonk_resources::graphics::GraphicsResource::from_group(global_material_group)
                .expect("installed material graphics index");
        let liquid = resource.load_image("LIQUID.png").expect("Liquid texture");
        assert_eq!((liquid.width(), liquid.height()), (128, 128));
        let liquid = ImageData::new(liquid.width(), liquid.height(), liquid.pixels().to_vec());
        let material = MaterialRenderInfo::new(
            color,
            [0; 6],
            acid.value("TextureOverlay").map(ToOwned::to_owned),
            acid.int("OverlayType").unwrap_or(0),
            acid.int("Density").unwrap_or(0),
        )
        .with_placement(acid.int("Placement").unwrap_or(0));
        let mut graphics = test_graphics(16, 16, 16, "Tutorial 07 Acid");
        graphics.viewport_x = 196.0;
        graphics.viewport_y = 349.0;
        graphics.surface_mut().fill(Color::opaque(7, 11, 13));
        graphics.set_material_textures(Arc::new(HashMap::from([("liquid".to_string(), liquid)])));
        graphics
            .set_material_render_info(Arc::new(HashMap::from([("acid".to_string(), material)])));
        let gamma_points = snapshot.environment.gamma.combined_control_points();
        assert_eq!(gamma_points, [0x000000, 0x648064, 0xc8ffc8]);
        assert_eq!(snapshot.environment.settings.time_of_day, 0);
        assert_eq!(GraphicsSystem::lighting_factor(0), 1.0);
        let gamma = clonk_graphics::GammaRamp::from_control_points(gamma_points);
        assert!(graphics.draw_ground_textured(snapshot.landscape.as_ref(), Some(&gamma)));

        assert_eq!(
            [(0, 0), (4, 0), (15, 0), (0, 8), (8, 8), (15, 15)]
                .map(|(x, y)| graphics.surface().get_pixel(x, y)),
            [
                Some(Color::opaque(1, 192, 1)),
                Some(Color::opaque(1, 188, 1)),
                Some(Color::opaque(1, 190, 1)),
                Some(Color::opaque(1, 190, 1)),
                Some(Color::opaque(1, 192, 1)),
                Some(Color::opaque(1, 182, 1)),
            ],
        );
        assert_eq!(graphics.surface().snapshot().checksum(), 0x03df_cb2d);
    }

    #[test]
    fn viewport_point_at_maps_screen_to_world() {
        let snapshot = make_snapshot();
        let focus = &snapshot.objects[0];
        let mut graphics = test_graphics(320, 180, 150, "Viewport Test");
        let viewports = vec![ViewportInput::from_focus(focus)];
        graphics.render_frame(&snapshot, &viewports);

        let (screen_x, screen_y) = graphics
            .world_to_screen(focus.owner, focus.position)
            .expect("screen coordinates available");
        let pointer = graphics
            .viewport_point_at(GuiPoint::new(screen_x, screen_y))
            .expect("viewport pointer available");
        assert_eq!(pointer.owner, focus.owner);
        assert!(
            (pointer.world.x - focus.position.x as f32).abs() < 0.5,
            "expected world x close to focus, got {}",
            pointer.world.x
        );
        assert!(
            (pointer.world.y - focus.position.y as f32).abs() < 0.5,
            "expected world y close to focus, got {}",
            pointer.world.y
        );
    }

    #[test]
    fn owner_viewport_projection_clamps_pointer_over_other_owner() {
        // Fullscreen C4MouseControl routes every physical point through its
        // stored player's first viewport, then clamps local coordinates to
        // 0..ViewWdt-1 / 0..ViewHgt-1. It does not switch to the viewport under
        // the pointer (pristine 9ffa0a5d src/C4GraphicsSystem.cpp:410-419,
        // 476-484; src/C4MouseControl.cpp:203-216).
        let mut snapshot = make_snapshot();
        let mut second = snapshot.objects[0].clone();
        second.id = ObjectId::new(2);
        second.owner = 1;
        second.controller = 1;
        second.position = Vector2::new(180, 100);
        snapshot.objects.push(second);
        let mut graphics = test_graphics(320, 180, 150, "Mouse owner viewport");
        graphics.render_frame(
            &snapshot,
            &[
                ViewportInput::new(0, Vector2::new(100, 100), 1.0, &snapshot.objects[0]),
                ViewportInput::new(1, Vector2::new(180, 100), 1.0, &snapshot.objects[1]),
            ],
        );
        let owner_viewport = &graphics.active_viewports[0];
        let other_viewport = &graphics.active_viewports[1];
        let physical_point = GuiPoint::new(
            other_viewport.rect.x as f32 + other_viewport.rect.width as f32 / 2.0,
            other_viewport.rect.y as f32 + other_viewport.rect.height as f32 / 2.0,
        );
        assert_eq!(
            graphics
                .viewport_output_point_at(physical_point)
                .expect("hovered viewport pointer")
                .owner,
            1,
            "the existing hit-test confirms the physical point is over owner 1"
        );

        let pointer = graphics
            .viewport_output_point_for_owner(0, physical_point)
            .expect("mouse owner's viewport pointer");
        let expected_screen = GuiPoint::new(
            (owner_viewport.rect.x + owner_viewport.rect.width as i32 - 1) as f32,
            physical_point.y,
        );
        let expected_world = FloatVector2::new(
            (expected_screen.x - owner_viewport.content_rect.x as f32) / owner_viewport.zoom
                + owner_viewport.viewport_x,
            (expected_screen.y - owner_viewport.content_rect.y as f32) / owner_viewport.zoom
                + owner_viewport.viewport_y,
        );
        assert_eq!(pointer.owner, 0);
        assert_eq!(pointer.screen, expected_screen);
        assert_eq!(pointer.world, expected_world);
    }

    #[test]
    fn crew_at_point_returns_local_crew() {
        let mut snapshot = make_snapshot();
        snapshot.objects[0].owner = 1;
        let focus = &snapshot.objects[0];

        let mut graphics = test_graphics(320, 180, 150, "Crew Pick");
        let viewports = vec![ViewportInput::from_focus(focus)];
        graphics.render_frame(&snapshot, &viewports);

        let (screen_x, screen_y) = graphics
            .world_to_screen(1, focus.position)
            .expect("screen coordinates available");
        let point = GuiPoint::new(screen_x, screen_y);

        let picked = graphics.crew_at_point(&snapshot, 1, point);
        assert_eq!(picked, Some(focus.id));
        assert_eq!(
            graphics.crew_at_point(&snapshot, 2, point),
            None,
            "other owners should not pick crew"
        );
    }

    #[test]
    fn object_at_point_uses_cpp_front_to_back_order() {
        // C4Game::FindVisObject walks Objects.First -> Next, the reverse of
        // C4ObjectList::Draw's Last -> Prev order. Ownership does not filter
        // context targets; MouseIgnore and contained objects do.
        let mut snapshot = make_snapshot();
        snapshot.objects[0].owner = 1;
        snapshot.objects[0].ocf = 1;
        let back_id = snapshot.objects[0].id;
        let mut front = snapshot.objects[0].clone();
        front.id = ObjectId::new(2);
        front.owner = 2;
        let front_id = front.id;
        snapshot.objects.push(front);
        snapshot.render_order = vec![back_id, front_id];

        let focus = snapshot.objects[0].clone();
        let mut graphics = test_graphics(320, 180, 150, "Object Pick");
        graphics.render_frame(&snapshot, &[ViewportInput::from_focus(&focus)]);
        let (screen_x, screen_y) = graphics
            .world_to_screen(1, focus.position)
            .expect("screen coordinates available");
        let point = GuiPoint::new(screen_x, screen_y);

        assert_eq!(
            graphics.object_at_point(&snapshot, 1, point),
            Some(front_id)
        );
        assert_eq!(
            graphics.object_at_point_excluding(&snapshot, 1, point, front_id),
            Some(back_id),
            "FindVisObject skips only the exact excluded object"
        );

        snapshot.objects[1].visibility = clonk_engine::VIS_NONE;
        assert_eq!(
            graphics.object_at_point(&snapshot, 1, point),
            Some(back_id),
            "FindVisObject must skip a VIS_None front object"
        );
        snapshot.objects[1].visibility = clonk_engine::VIS_ALL;

        snapshot.objects[0].ocf = clonk_engine::ocf::CONTAINER;
        assert_eq!(
            graphics.object_at_point_with_ocf(&snapshot, 1, point, clonk_engine::ocf::CONTAINER,),
            Some(back_id),
            "an OCF-filtered search skips a nonmatching front object"
        );
        snapshot.objects[0].ocf = 1;

        snapshot.objects[1].alive = false;
        assert_eq!(
            graphics.object_at_point(&snapshot, 1, point),
            Some(front_id),
            "structures and items are context targets despite Alive=false"
        );

        snapshot.objects[1].category |= CATEGORY_MOUSE_IGNORE_FLAG;
        assert_eq!(graphics.object_at_point(&snapshot, 1, point), Some(back_id));

        snapshot.objects[0].container = Some(front_id);
        assert_eq!(graphics.object_at_point(&snapshot, 1, point), None);

        snapshot.objects[0].container = None;
        snapshot.objects[1].category &= !CATEGORY_MOUSE_IGNORE_FLAG;
        snapshot.players = vec![PlayerState {
            id: 1,
            cursor: None,
            ..PlayerState::default()
        }];
        assert_eq!(
            graphics.object_at_point(&snapshot, 1, point),
            None,
            "a valid player without a cursor must fall through to select-next"
        );
    }

    #[test]
    fn object_visibility_matches_cpp_masks_layers_and_local_bits() {
        let mut snapshot = make_snapshot();
        let object = &mut snapshot.objects[0];
        object.owner = 1;
        snapshot.players = vec![
            PlayerState {
                id: 1,
                ..PlayerState::default()
            },
            PlayerState {
                id: 2,
                hostility: vec![1],
                ..PlayerState::default()
            },
            PlayerState {
                id: 3,
                ..PlayerState::default()
            },
        ];

        snapshot.objects[0].visibility = VIS_OWNER;
        assert!(GraphicsSystem::object_is_visible(
            &snapshot.objects,
            &snapshot.players,
            &snapshot.objects[0],
            1,
            false,
        ));
        assert!(!GraphicsSystem::object_is_visible(
            &snapshot.objects,
            &snapshot.players,
            &snapshot.objects[0],
            2,
            false,
        ));

        snapshot.objects[0].visibility = VIS_ALLIES | VIS_ENEMIES;
        assert!(GraphicsSystem::object_is_visible(
            &snapshot.objects,
            &snapshot.players,
            &snapshot.objects[0],
            2,
            false,
        ));
        assert!(GraphicsSystem::object_is_visible(
            &snapshot.objects,
            &snapshot.players,
            &snapshot.objects[0],
            3,
            false,
        ));
        assert!(!GraphicsSystem::object_is_visible(
            &snapshot.objects,
            &snapshot.players,
            &snapshot.objects[0],
            1,
            false,
        ));

        snapshot.objects[0].visibility = VIS_LOCAL;
        snapshot.objects[0].local_vars.insert(
            "__local_0".into(),
            serde_json::from_value(serde_json::json!({"Int": 1 << 3}))
                .expect("numbered Local value"),
        );
        assert!(GraphicsSystem::object_is_visible(
            &snapshot.objects,
            &snapshot.players,
            &snapshot.objects[0],
            3,
            false,
        ));
        assert!(!GraphicsSystem::object_is_visible(
            &snapshot.objects,
            &snapshot.players,
            &snapshot.objects[0],
            2,
            false,
        ));

        snapshot.objects[0].visibility = VIS_GOD;
        assert!(GraphicsSystem::object_is_visible(
            &snapshot.objects,
            &snapshot.players,
            &snapshot.objects[0],
            OWNER_NONE,
            false,
        ));
        snapshot.objects[0].visibility = VIS_OVERLAY_ONLY;
        assert!(!GraphicsSystem::object_is_visible(
            &snapshot.objects,
            &snapshot.players,
            &snapshot.objects[0],
            1,
            false,
        ));
        assert!(GraphicsSystem::object_is_visible(
            &snapshot.objects,
            &snapshot.players,
            &snapshot.objects[0],
            1,
            true,
        ));

        let mut layer = snapshot.objects[0].clone();
        layer.id = ObjectId::new(2);
        layer.layer = None;
        layer.visibility = VIS_OWNER | VIS_LAYER_TOGGLE;
        snapshot.objects[0].visibility = 0;
        snapshot.objects[0].layer = Some(layer.id);
        snapshot.objects.push(layer);
        assert!(!GraphicsSystem::object_is_visible(
            &snapshot.objects,
            &snapshot.players,
            &snapshot.objects[0],
            1,
            false,
        ));
        assert!(GraphicsSystem::object_is_visible(
            &snapshot.objects,
            &snapshot.players,
            &snapshot.objects[0],
            3,
            false,
        ));
    }

    #[test]
    fn graphics_system_draws_ground() {
        let snapshot = make_snapshot();
        let focus = &snapshot.objects[0];
        let mut graphics = test_graphics(320, 180, 150, "Test Scenario");
        graphics.set_world_width(256);

        let viewports = vec![ViewportInput::from_focus(focus)];
        let atlas = graphics.render_frame(&snapshot, &viewports);
        assert!(!atlas.is_empty());

        let ground = graphics.surface().get_pixel(0, 179).unwrap();
        assert_ne!(ground, Color::opaque(8, 12, 24));
    }

    fn pxs_particle(material: &str, fixed: [i32; 4], slot: u32) -> clonk_engine::ParticleSnapshot {
        clonk_engine::ParticleSnapshot {
            definition_id: format!("material/pxs/{material}"),
            position: FloatVector2::new(fixed[0] as f32 / 65_536.0, fixed[1] as f32 / 65_536.0),
            velocity: FloatVector2::new(fixed[2] as f32 / 65_536.0, fixed[3] as f32 / 65_536.0),
            life: 0,
            parameter_a: 0.0,
            parameter_b: 0,
            layer: clonk_engine::ParticleLayer::Global,
            pxs_fixed: Some(fixed),
            pxs_slot: Some(slot),
        }
    }

    #[test]
    fn old_style_pxs_draws_cpp_velocity_line_with_alpha() {
        // C4PXSSystem::Draw uses the material palette color and turns moving
        // pixels into x-xdir/y-ydir velocity lines. Its Clonk transparency is
        // max(alpha, 195-(195-alpha)/fixtoi(|xdir|+|ydir|))
        // (C4PXS.cpp:242-275).
        let mut graphics = test_graphics(12, 12, 12, "PXS");
        graphics.surface_mut().fill(Color::opaque(0, 0, 0));
        graphics.set_material_render_info(Arc::new(HashMap::from([(
            "rain".to_string(),
            MaterialRenderInfo::new([200, 100, 50, 0, 0, 0, 0, 0, 0], [0; 6], None, 0, 25),
        )])));
        let particle = pxs_particle("rain", [8 << 16, 8 << 16, 2 << 16, 0], 3);

        graphics.draw_pxs(std::slice::from_ref(&particle), 1.0, None);

        assert_eq!(
            graphics.surface().get_pixel(6, 8),
            Some(Color::opaque(123, 61, 30)),
            "two-pixel velocity has C++ transparency 98 (opacity 157)"
        );
        assert_eq!(
            graphics.surface().get_pixel(7, 8),
            Some(Color::opaque(123, 61, 30))
        );
        assert_eq!(
            graphics.surface().get_pixel(8, 8),
            Some(Color::opaque(0, 0, 0)),
            "GL_LINES applies the diamond-exit rule and omits the final endpoint",
        );
    }

    #[test]
    fn retained_moving_pxs_is_one_gpu_line_not_one_point_per_covered_pixel() {
        let mut surface = Surface::new(16, 12, PixelFormat::Rgba8888);
        surface.begin_gpu_scene_capture();
        draw_pxs_line(
            &mut surface,
            (2.25, 4.75),
            (11.125, 7.375),
            Color::new(200, 100, 50, 157),
            None,
            None,
        );

        let scene = surface
            .take_gpu_scene_capture()
            .expect("PXS capture remains active")
            .into_scene(
                [16, 12],
                Color::transparent(),
                &clonk_graphics::GammaRamp::standard(),
            );
        assert_eq!(scene.commands.len(), 1);
        let GpuCommand::Solid {
            vertices, topology, ..
        } = &scene.commands[0]
        else {
            panic!("moving PXS did not lower to solid geometry");
        };
        assert_eq!(*topology, GpuPrimitiveTopology::LineList);
        assert_eq!(vertices.len(), 2);
        assert_eq!(vertices[0].position, [2.75, 5.25, 1.0]);
        assert_eq!(vertices[1].position, [11.625, 7.875, 1.0]);
    }

    #[test]
    fn retained_degenerate_pxs_line_is_not_promoted_to_a_point() {
        let mut surface = Surface::new(8, 8, PixelFormat::Rgba8888);
        surface.begin_gpu_scene_capture();
        draw_pxs_line(
            &mut surface,
            (2.25, 3.75),
            (2.25, 3.75),
            Color::opaque(200, 100, 50),
            None,
            None,
        );

        let scene = surface
            .take_gpu_scene_capture()
            .expect("PXS capture remains active")
            .into_scene(
                [8, 8],
                Color::transparent(),
                &clonk_graphics::GammaRamp::standard(),
            );
        let [GpuCommand::Solid {
            vertices, topology, ..
        }] = scene.commands.as_slice()
        else {
            panic!("degenerate DrawLineDw did not remain one line primitive");
        };
        assert_eq!(*topology, GpuPrimitiveTopology::LineList);
        assert_eq!(vertices.len(), 2);
        assert_eq!(vertices[0].position, [2.75, 4.25, 1.0]);
        assert_eq!(vertices[0].position, vertices[1].position);
    }

    #[test]
    fn retained_draw_pix_preserves_fractional_vertex_until_point_rasterization() {
        let mut surface = Surface::new(4, 3, PixelFormat::Rgba8888);
        surface.begin_gpu_scene_capture();
        draw_pxs_pixel(
            &mut surface,
            0.75,
            1.25,
            Color::opaque(200, 100, 50),
            None,
            None,
        );

        let scene = surface
            .take_gpu_scene_capture()
            .expect("point capture remains active")
            .into_scene(
                [4, 3],
                Color::transparent(),
                &clonk_graphics::GammaRamp::standard(),
            );
        let [GpuCommand::Solid {
            vertices,
            topology,
            alpha_mode,
            ..
        }] = scene.commands.as_slice()
        else {
            panic!("DrawPixInt did not remain one retained point");
        };
        assert_eq!(*topology, GpuPrimitiveTopology::PointList);
        assert_eq!(*alpha_mode, GpuSolidAlphaMode::SourceOver);
        assert_eq!(vertices[0].position, [1.25, 1.75, 1.0]);
    }

    #[test]
    fn retained_draw_pix_defers_scaled_edge_coverage_to_gpu_rasterization() {
        let mut surface = Surface::new(4, 3, PixelFormat::Rgba8888);
        surface.begin_gpu_scene_capture();
        draw_pxs_pixel(
            &mut surface,
            -0.5,
            1.0,
            Color::opaque(200, 100, 50),
            None,
            None,
        );

        let scene = surface
            .take_gpu_scene_capture()
            .expect("point capture remains active")
            .into_scene(
                [4, 3],
                Color::transparent(),
                &clonk_graphics::GammaRamp::standard(),
            );
        let [GpuCommand::Solid { vertices, .. }] = scene.commands.as_slice() else {
            panic!("edge DrawPixInt was culled before physical scale was known");
        };
        assert_eq!(vertices[0].position, [0.0, 1.5, 1.0]);
    }

    #[test]
    fn stationary_pxs_samples_fog_before_rounding_its_raster_position() {
        let mut graphics = test_graphics(3, 2, 2, "fractional PXS fog");
        graphics.surface_mut().fill(Color::opaque(0, 0, 0));
        graphics.set_material_render_info(Arc::new(HashMap::from([(
            "rain".to_string(),
            MaterialRenderInfo::new([200, 100, 50, 0, 0, 0, 0, 0, 0], [0; 6], None, 0, 25),
        )])));
        graphics.active_fog_map = Some(Arc::new(ClrModMap {
            resolution_x: 1,
            resolution_y: 1,
            width: 3,
            height: 2,
            origin_x: 0,
            origin_y: 0,
            fade_transparent: false,
            cells: vec![0x00ff_ffff, 0, 0, 0x00ff_ffff, 0, 0],
        }));
        let particle = pxs_particle("rain", [49_152, 0, 0, 0], 0); // x = 0.75

        graphics.draw_pxs(&[particle], 1.0, None);

        assert_eq!(
            graphics.surface().get_pixel(1, 0),
            Some(modulate_surface_color(
                Color::opaque(200, 100, 50),
                0x00ff_ffff,
            )),
            "DrawPix samples fog at int(0.75)=0 before rasterizing at round(0.75)=1",
        );
    }

    #[test]
    fn mouse_selection_frame_matches_cpp_palette_raster_and_viewport_clip() {
        // C4MouseControl::Draw passes current -> down endpoints and CRed to
        // DrawFrame. On the render target, each edge is an independent
        // half-open GL_LINES primitive, clipped by C4Viewport's full output
        // clipper (C4MouseControl.cpp:406-414; C4Viewport.cpp:1092-1118;
        // StdDDraw2.cpp:1113-1180; StdGL.cpp:893-933).
        let background = Color::opaque(1, 2, 3);
        let mut surface = Surface::new(8, 8, PixelFormat::Rgba8888);
        surface.fill(background);

        draw_mouse_selection_frame_raster(
            &mut surface,
            SurfaceRect::new(2, 2, 5, 5),
            (1, 3),
            (5, 6),
            MOUSE_SELECTION_FRAME_COLOR,
            None,
        );

        let expected = [
            (2, 3),
            (3, 3),
            (4, 3),
            (5, 3),
            (5, 4),
            (5, 5),
            (2, 6),
            (3, 6),
            (4, 6),
        ];
        for y in 0..8 {
            for x in 0..8 {
                assert_eq!(
                    surface.get_pixel(x, y),
                    Some(if expected.contains(&(x, y)) {
                        MOUSE_SELECTION_FRAME_COLOR
                    } else {
                        background
                    }),
                    "selection-frame pixel ({x},{y})"
                );
            }
        }
        assert_eq!(
            surface.get_pixel(5, 6),
            Some(background),
            "the shared second endpoint stays omitted by all four GL lines"
        );

        let active_palette_red = Color::opaque(4, 8, 12);
        surface.fill(background);
        draw_mouse_selection_frame_raster(
            &mut surface,
            SurfaceRect::new(2, 2, 5, 5),
            (1, 3),
            (5, 6),
            active_palette_red,
            None,
        );
        assert_eq!(
            surface.get_pixel(3, 3),
            Some(active_palette_red),
            "the selection frame uses color index 10 from the active game palette"
        );
    }

    #[test]
    fn retained_mouse_selection_frame_is_four_gpu_lines() {
        let mut surface = Surface::new(12, 10, PixelFormat::Rgba8888);
        surface.begin_gpu_scene_capture();
        draw_mouse_selection_frame_raster(
            &mut surface,
            SurfaceRect::new(1, 1, 10, 8),
            (2, 3),
            (9, 7),
            MOUSE_SELECTION_FRAME_COLOR,
            None,
        );

        let scene = surface
            .take_gpu_scene_capture()
            .expect("selection capture remains active")
            .into_scene(
                [12, 10],
                Color::transparent(),
                &clonk_graphics::GammaRamp::standard(),
            );
        assert_eq!(scene.commands.len(), 1);
        let GpuCommand::Solid {
            vertices,
            topology,
            clip,
            ..
        } = &scene.commands[0]
        else {
            panic!("selection frame did not lower to solid geometry");
        };
        assert_eq!(*topology, GpuPrimitiveTopology::LineList);
        assert_eq!(vertices.len(), 8);
        assert_eq!(*clip, Some(SurfaceRect::new(1, 1, 10, 8)));
    }

    #[test]
    fn mouse_selection_frame_uses_the_active_cpp_gamma_ramp() {
        // DrawLineDw binds the same gamma textures as the rest of the GL
        // overlay before emitting CRed (StdGL.cpp:893-919,1246-1263).
        let gamma = clonk_graphics::GammaRamp::from_control_points([0x102030, 0x405060, 0x708090]);
        let mut surface = Surface::new(4, 4, PixelFormat::Rgba8888);
        surface.fill(Color::opaque(0, 0, 0));

        draw_mouse_selection_frame_raster(
            &mut surface,
            SurfaceRect::new(0, 0, 4, 4),
            (0, 1),
            (3, 3),
            MOUSE_SELECTION_FRAME_COLOR,
            Some(&gamma),
        );

        assert_eq!(
            surface.get_pixel(1, 1),
            Some(gamma_encode_fragment(MOUSE_SELECTION_FRAME_COLOR, &gamma))
        );
    }

    #[test]
    fn offscreen_pxs_endpoint_culls_crossing_velocity_line() {
        // The enlarged VisibleRect checks fixtoi(x,y) before drawing the
        // x-xdir velocity line (C4PXS.cpp:245-275). This endpoint is far
        // outside that rect even though its 100px line crosses the surface.
        let mut graphics = test_graphics(12, 12, 12, "PXS");
        graphics.surface_mut().fill(Color::opaque(1, 2, 3));
        graphics.set_material_render_info(Arc::new(HashMap::from([(
            "rain".to_string(),
            MaterialRenderInfo::new([255, 255, 255, 0, 0, 0, 0, 0, 0], [0; 6], None, 0, 25),
        )])));
        let particle = pxs_particle("rain", [100 << 16, 6 << 16, 100 << 16, 0], 3);

        graphics.draw_pxs(std::slice::from_ref(&particle), 1.0, None);

        assert!(graphics
            .surface()
            .pixels()
            .chunks_exact(4)
            .all(|pixel| pixel == [1, 2, 3, 255]));
    }

    #[test]
    fn graphical_pxs_uses_saved_slot_phase_and_falls_back_without_texture() {
        // The graphical pass derives phase and z from cnt2, the slot WITHIN
        // the 500-entry chunk, then applies PXSGfxRt offsets and size
        // (C4PXS.cpp:280-307). A missing PXSGfx texture stays in the first,
        // old-style pass (C4Material.cpp:382-385; C4PXS.cpp:257-260).
        let mut graphics = test_graphics(16, 16, 16, "PXS");
        graphics.surface_mut().fill(Color::opaque(0, 0, 0));
        let mut snow_pixels = vec![0; 12 * 6 * 4];
        for y in 0..6usize {
            for x in 0..12usize {
                let index = (y * 12 + x) * 4;
                if x < 6 {
                    snow_pixels[index + 2] = 255;
                } else {
                    snow_pixels[index] = 255;
                }
                snow_pixels[index + 3] = 128;
            }
        }
        graphics.set_material_texture_surfaces(Arc::new(HashMap::from([
            (
                "snow".to_string(),
                MaterialTextureSurface::surface32(ImageData::new(12, 6, snow_pixels)),
            ),
            (
                "indexed".to_string(),
                MaterialTextureSurface::surface8(1, 1, vec![2]),
            ),
            (
                "empty".to_string(),
                MaterialTextureSurface::surface32(ImageData::new(0, 0, Vec::new())),
            ),
        ])));
        graphics.set_material_render_info(Arc::new(HashMap::from([
            (
                "snow".to_string(),
                MaterialRenderInfo::new([0, 255, 0, 0, 0, 0, 0, 0, 0], [0; 6], None, 0, 25)
                    .with_pxs_graphics(Some("Snow".to_string()), [0, 0, 6, 6, 6, 0], 1),
            ),
            (
                "ash".to_string(),
                MaterialRenderInfo::new([90, 80, 70, 0, 0, 0, 0, 0, 0], [0; 6], None, 0, 25)
                    .with_pxs_graphics(Some("Missing".to_string()), [0, 0, 2, 2, 0, 0], 3),
            ),
            (
                "mud".to_string(),
                MaterialRenderInfo::new([11, 22, 33, 0, 0, 0, 0, 0, 0], [0; 6], None, 0, 25)
                    .with_pxs_graphics(Some("Indexed".to_string()), [0, 0, 1, 1, 0, 0], 1),
            ),
            (
                "dust".to_string(),
                MaterialRenderInfo::new([44, 55, 66, 0, 0, 0, 0, 0, 0], [0; 6], None, 0, 25)
                    .with_pxs_graphics(Some("Empty".to_string()), [0, 0, 1, 1, 0, 0], 1),
            ),
        ])));
        let graphical = pxs_particle("snow", [4 << 16, 4 << 16, 0, 0], 507);
        let fallback = pxs_particle("ash", [10 << 16, 10 << 16, 0, 0], 1);
        let indexed_fallback = pxs_particle("mud", [13 << 16, 13 << 16, 0, 0], 2);
        let empty_surface32 = pxs_particle("dust", [14 << 16, 14 << 16, 0, 0], 3);

        graphics.draw_pxs(
            &[graphical, fallback, indexed_fallback, empty_surface32],
            1.0,
            None,
        );

        // 507 % 500 = 7: phase x=1; z=1; tx=6 shifts x by one. Texture
        // transparency 127 plus modulation 16 gives 143, i.e. source opacity
        // 112 over black (PerformBlt alpha addition).
        assert_eq!(
            graphics.surface().get_pixel(5, 4),
            Some(Color::opaque(112, 0, 0))
        );
        assert_eq!(
            graphics.surface().get_pixel(10, 10),
            Some(Color::opaque(90, 80, 70))
        );
        assert_eq!(
            graphics.surface().get_pixel(13, 13),
            Some(Color::opaque(11, 22, 33)),
            "Surface8 textures are landscape patterns, not graphical PXS sheets"
        );
        assert_eq!(
            graphics.surface().get_pixel(14, 14),
            Some(Color::opaque(44, 55, 66)),
            "a 0x0 Surface32 identity contains native divide-by-zero as old-style PXS"
        );
    }

    #[test]
    fn graphical_pxs_uses_gl_linear_filtering_across_its_source_facet() {
        // C4Facet::DrawX supplies a 2x4 source facet and a non-exact 4x4
        // target, which enables GL_LINEAR (C4Facet.cpp:296-303;
        // StdDDraw2.cpp:663-669; StdGL.cpp:527-531). Internal facet edges are
        // not sampler boundaries, so the first/last columns blend adjacent
        // sheet texels too.
        let columns = [
            Color::opaque(255, 0, 0),
            Color::opaque(0, 0, 0),
            Color::opaque(255, 255, 255),
            Color::opaque(0, 0, 255),
        ];
        let pixels = (0..4)
            .flat_map(|_| columns)
            .flat_map(|color| [color.r, color.g, color.b, color.a])
            .collect();
        let image = ImageData::new(4, 4, pixels);
        let mut surface = Surface::new(4, 4, PixelFormat::Rgba8888);
        surface.fill(Color::opaque(0, 0, 0));

        draw_pxs_image_region(
            &mut surface,
            &GuiRect::new(0.0, 0.0, 4.0, 4.0),
            &image,
            &SourceRect::new(1, 0, 2, 4),
            0,
            1.0,
            AdvancedRendererConfig::DEFAULT,
            None,
            None,
        );

        assert_eq!(surface.get_pixel(0, 1), Some(Color::opaque(64, 0, 0)));
        assert_eq!(surface.get_pixel(1, 1), Some(Color::opaque(64, 64, 64)));
        assert_eq!(surface.get_pixel(2, 1), Some(Color::opaque(191, 191, 191)));
        assert_eq!(surface.get_pixel(3, 1), Some(Color::opaque(191, 191, 255)));
    }

    #[test]
    fn scalar_precipitation_without_real_pxs_does_not_paint_the_viewport() {
        // C4Viewport has no synthetic precipitation pass: weather launches
        // the FXP1 precipitation object, whose callback inserts real material
        // into the simulation (C4Viewport.cpp:1056-1078; C4Weather.cpp:48-58,
        // 205-214). A scalar alone must not alter otherwise identical pixels.
        let render = |snapshot: &SimulationSnapshot| {
            let mut graphics = test_graphics(80, 60, 60, "Weather");
            graphics.render_frame(snapshot, &[ViewportInput::from_focus(&snapshot.objects[0])]);
            graphics.surface().pixels().to_vec()
        };
        let dry = make_snapshot();
        let mut scalar_only = dry.clone();
        scalar_only.environment.precipitation = 80;
        scalar_only.environment.settings = scalar_only
            .environment
            .settings
            .with_precipitation(80)
            .with_precipitation_strength(80);

        assert_eq!(render(&scalar_only), render(&dry));
    }

    #[test]
    fn weather_lightning_event_does_not_synthesize_an_early_flash() {
        // C4Weather::LaunchLightning only creates FXL1 and calls Activate
        // (C4Weather.cpp:158-168). Activate accumulates enlightenment, but
        // SetGamma is deferred until FXL1's Advance callback executes on the
        // next object phase (Effects/Lightning/Script.c:16-31, 72-92), because
        // Weather.Execute runs after ExecObjects (C4Game.cpp:811-835). The
        // launch-frame presentation therefore must not add a separate flash.
        let render = |snapshot: &SimulationSnapshot| {
            let mut graphics = test_graphics(80, 60, 60, "Weather lightning");
            graphics.set_sky(Some(SkyRenderState::new(
                SkySettings {
                    fade_top: RgbColor::new(24, 48, 96),
                    fade_bottom: RgbColor::new(96, 128, 192),
                    ..Default::default()
                },
                None,
            )));
            graphics.render_frame(snapshot, &[ViewportInput::from_focus(&snapshot.objects[0])]);
            graphics.surface().pixels().to_vec()
        };
        let clear = make_snapshot();
        let mut launched = clear.clone();
        launched
            .weather_events
            .push(WeatherEvent::Lightning { position: 40 });

        assert_eq!(render(&launched), render(&clear));
    }

    #[test]
    fn render_frame_places_pxs_between_landscape_and_objects() {
        // C4Viewport::Draw orders Landscape -> PXS -> Objects
        // (C4Viewport.cpp:1056-1073). The red 1x1 object must therefore cover
        // the blue old-style PXS at the same world position.
        let mut snapshot = make_snapshot();
        snapshot
            .particles
            .push(pxs_particle("rain", [100 << 16, 100 << 16, 0, 0], 0));
        let sprites = solid_sprite(
            "TestObject",
            1,
            1,
            Color::opaque(240, 0, 0),
            Some(DefinitionRect::new(0, 0, 1, 1)),
            false,
        );
        let mut graphics = GraphicsSystem::new(
            80,
            60,
            60,
            "PXS order",
            test_font(),
            Arc::clone(&sprites),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics.set_material_render_info(Arc::new(HashMap::from([(
            "rain".to_string(),
            MaterialRenderInfo::new([0, 0, 240, 0, 0, 0, 0, 0, 0], [0; 6], None, 0, 25),
        )])));
        graphics.render_frame(
            &snapshot,
            &[ViewportInput::from_focus(&snapshot.objects[0])],
        );
        let (screen_x, screen_y) = graphics
            .world_to_screen(0, snapshot.objects[0].position)
            .expect("active viewport");

        assert_eq!(
            graphics
                .surface()
                .get_pixel(screen_x.round() as u32, screen_y.round() as u32),
            Some(standard_gamma_color(Color::opaque(240, 0, 0))),
        );

        let mut background_snapshot = snapshot.clone();
        background_snapshot.objects[0].category |= CATEGORY_BACKGROUND_FLAG;
        let mut background_graphics = GraphicsSystem::new(
            80,
            60,
            60,
            "PXS background order",
            test_font(),
            sprites,
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        background_graphics.set_material_render_info(Arc::new(HashMap::from([(
            "rain".to_string(),
            MaterialRenderInfo::new([0, 0, 240, 0, 0, 0, 0, 0, 0], [0; 6], None, 0, 25),
        )])));
        background_graphics.render_frame(
            &background_snapshot,
            &[ViewportInput::from_focus(&background_snapshot.objects[0])],
        );
        let (screen_x, screen_y) = background_graphics
            .world_to_screen(0, background_snapshot.objects[0].position)
            .expect("active viewport");
        let lighting =
            GraphicsSystem::lighting_factor(background_snapshot.environment.settings.time_of_day);

        assert_eq!(
            background_graphics
                .surface()
                .get_pixel(screen_x.round() as u32, screen_y.round() as u32),
            Some(standard_gamma_color(
                Color::opaque(0, 0, 240).modulate(lighting),
            )),
            "PXS must cover C4D_Background objects drawn before landscape",
        );
    }

    #[test]
    fn parallax_select_mark_ignores_viewport_scroll() {
        // C4Object::DrawSelectMark resolves cox/coy through TargetPos exactly
        // like C4Object::Draw does (src/C4Object.cpp:3887-3893), so the marks
        // stay locked to a pinned C4D_Parallax object instead of scrolling
        // off-screen with the landscape.
        let mut snapshot = make_snapshot();
        snapshot.landscape = Some(Landscape::flat(128, 80));
        snapshot.objects[0].position = Vector2::new(100, 60);
        snapshot.objects[0].owner = 1;

        let mut pinned = snapshot.objects[0].clone();
        pinned.id = ObjectId::new(2);
        pinned.definition_id = "PinnedHud".into();
        pinned.position = Vector2::new(20, 15);
        pinned.crew_member = false;
        pinned.category = clonk_engine::DEFAULT_CATEGORY | CATEGORY_PARALLAX_FLAG;
        snapshot.objects.push(pinned.clone());
        snapshot.render_order = snapshot.objects.iter().map(|object| object.id).collect();
        snapshot.players.push(PlayerState {
            id: 1,
            cursor: Some(pinned.id),
            control: clonk_engine::PlayerControlState {
                select_flash: 30,
                ..Default::default()
            },
            ..PlayerState::default()
        });

        // A fully transparent face keeps the assertion on the mark alone.
        let sprites = solid_sprite(
            "PinnedHud",
            12,
            12,
            Color::new(0, 0, 0, 0),
            Some(DefinitionRect::new(-6, -6, 12, 12)),
            false,
        );
        let hud = HudGraphics {
            select_mark: Some(ImageData::new(
                20,
                5,
                (0..100).flat_map(|_| [0, 220, 0, 255]).collect(),
            )),
            ..Default::default()
        };
        let mut graphics = GraphicsSystem::new(
            80,
            60,
            60,
            "Parallax select mark",
            test_font(),
            sprites,
            empty_cursor_atlas(),
            Arc::new(hud),
        );
        graphics.render_frame(
            &snapshot,
            &[ViewportInput::new(
                1,
                snapshot.objects[0].position,
                1.0,
                &snapshot.objects[0],
            )],
        );

        // The camera is scrolled to (48,20) by the far focus object, so the
        // unresolved origin would place this mark at (-36,-13) — outside the
        // cull margin entirely. TargetPos pins it at x + Shape.x - 2 = 12.
        let (viewport_x, viewport_y) = graphics.viewport();
        assert!(
            viewport_x > 16 && viewport_y > 5,
            "camera must be scrolled far enough to separate the two origins",
        );
        assert_eq!(
            graphics.surface().get_pixel(13, 8),
            Some(standard_gamma_color(Color::opaque(0, 220, 0))),
            "select marks follow the pinned parallax object",
        );
    }

    #[test]
    fn foreground_parallax_split_straddles_cursor_marks_like_cpp() {
        // ForeObjects.DrawIfCategory(... C4D_Parallax, true) draws the
        // non-parallax foreground before Game.DrawCursors; the false pass
        // draws parallax/custom-GUI objects afterwards
        // (C4Viewport.cpp:1080-1103; C4ObjectList.cpp:400-409).
        let render = |category: i32, viewport_overlays_visible: bool| {
            let mut snapshot = make_snapshot();
            snapshot.objects[0].position = Vector2::new(40, 40);
            snapshot.objects[0].owner = 1;
            snapshot.objects[0].category = category;
            snapshot.landscape = Some(Landscape::flat(128, 80));
            snapshot.players.push(PlayerState {
                id: 1,
                cursor: Some(snapshot.objects[0].id),
                control: clonk_engine::PlayerControlState {
                    select_flash: 30,
                    ..Default::default()
                },
                ..PlayerState::default()
            });
            let sprites = solid_sprite(
                "TestObject",
                12,
                12,
                Color::opaque(220, 0, 0),
                Some(DefinitionRect::new(-6, -6, 12, 12)),
                false,
            );
            let hud = HudGraphics {
                select_mark: Some(ImageData::new(
                    20,
                    5,
                    (0..100).flat_map(|_| [0, 220, 0, 255]).collect(),
                )),
                ..Default::default()
            };
            let mut graphics = GraphicsSystem::new(
                80,
                60,
                60,
                "Foreground order",
                test_font(),
                sprites,
                empty_cursor_atlas(),
                Arc::new(hud),
            );
            graphics.viewport_overlays_visible = viewport_overlays_visible;
            graphics.render_frame(
                &snapshot,
                &[ViewportInput::from_focus(&snapshot.objects[0])],
            );
            let (viewport_x, viewport_y) = graphics.viewport();
            // C4D_Parallax with Local(0)/Local(1) unset and non-negative
            // coordinates resolves to cotx = coty = 0, so both the face and
            // its select marks are pinned to the viewport
            // (src/C4Object.cpp:2271,3887-3893,5839-5852).
            let (target_x, target_y) = if category & CATEGORY_PARALLAX_FLAG != 0 {
                (0, 0)
            } else {
                (viewport_x, viewport_y)
            };
            let x = snapshot.objects[0].position.x - target_x - 6;
            let y = snapshot.objects[0].position.y - target_y - 6;
            graphics.surface().get_pixel(x as u32, y as u32)
        };

        assert_eq!(
            render(
                clonk_engine::DEFAULT_CATEGORY | CATEGORY_FOREGROUND_FLAG,
                true,
            ),
            Some(standard_gamma_color(Color::opaque(0, 220, 0))),
            "cursor mark covers ordinary foreground",
        );
        assert_eq!(
            render(
                clonk_engine::DEFAULT_CATEGORY | CATEGORY_FOREGROUND_FLAG | CATEGORY_PARALLAX_FLAG,
                true,
            ),
            Some(standard_gamma_color(Color::opaque(220, 0, 0))),
            "custom-GUI/parallax foreground covers cursor mark",
        );
        assert_eq!(
            render(
                clonk_engine::DEFAULT_CATEGORY | CATEGORY_FOREGROUND_FLAG,
                false,
            ),
            Some(standard_gamma_color(Color::opaque(220, 0, 0))),
            "film replay suppresses object selection marks",
        );
    }

    #[test]
    fn overlay_state_feeds_the_hud_render() {
        let snapshot = make_snapshot();
        let focus = &snapshot.objects[0];
        let mut graphics = test_graphics(320, 180, 150, "Test Scenario");
        graphics.update_overlay(&GraphicsOverlay {
            frame_text: "FRAME",
            status_text: "STATUS",
            debug_hud: false,
            viewport_overlays_visible: true,
            players: Vec::new(),
            crew_name_labels: Vec::new(),
            game_time_seconds: 61,
            message_board: MessageBoardOverlay {
                log_lines: vec!["Player join: Test".to_string()],
                back_scroll: 0,
                ..MessageBoardOverlay::default()
            },
            clock_text: None,
            frames_per_second: None,
            upper_board_mode: hud::UpperBoardMode::Full,
            show_portraits: false,
            show_commands: true,
            show_command_keys: true,
        });
        assert!(!graphics.show_portraits);
        let viewports = vec![ViewportInput::from_focus(focus)];
        graphics.render_frame(&snapshot, &viewports);
        // Rendering with overlay state must not panic; without an
        // UpperBoard texture the chrome stays off (C4UpperBoard::Init,
        // src/C4UpperBoard.cpp:114-118) and the viewport spans the surface.
    }

    #[test]
    fn graphics_system_draws_player_control_hints_from_overlay() {
        // DrawOverlay reaches DrawPlayerInfo -> DrawPlayerControls after the
        // world pass (src/C4Viewport.cpp:835-848,1324-1327), so the selected
        // Control.png key cap must overwrite the viewport pixel.
        let snapshot = make_snapshot();
        let focus = &snapshot.objects[0];
        let width = 320u32;
        let height = 164u32;
        let mut pixels = vec![0u8; (width * height * 4) as usize];
        for y in 100..164 {
            for x in 0..64 {
                let index = ((y * width + x) * 4) as usize;
                pixels[index..index + 4].copy_from_slice(&[10, 10, 200, 255]);
            }
        }
        let hud_graphics = Arc::new(HudGraphics {
            control: Some(ImageData::new(width, height, pixels)),
            ..HudGraphics::default()
        });
        let mut graphics = GraphicsSystem::new(
            320,
            180,
            150,
            "Control Hint",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            hud_graphics,
        );
        graphics.update_overlay(&GraphicsOverlay {
            frame_text: "",
            status_text: "",
            debug_hud: false,
            viewport_overlays_visible: true,
            players: vec![PlayerOverlay {
                owner: 0,
                name: "Player".to_string(),
                wealth: 0,
                score: 0,
                view_wealth: false,
                view_value: false,
                cursor: None,
                captain: None,
                eliminated: false,
                owner_color: Color::opaque(0, 100, 200),
                select_count: 0,
                show_startup: false,
                control_set: -1,
                mouse_control: false,
                show_control: 1,
                show_control_position: 0,
                last_com: 5,
                control_key_labels: Vec::new(),
                crew_count: 0,
                crew: Vec::new(),
                commands: Vec::new(),
                flash_command: 0,
            }],
            crew_name_labels: Vec::new(),
            game_time_seconds: 0,
            message_board: MessageBoardOverlay::default(),
            clock_text: None,
            frames_per_second: None,
            upper_board_mode: hud::UpperBoardMode::Full,
            show_portraits: true,
            show_commands: true,
            show_command_keys: true,
        });

        graphics.render_frame(&snapshot, &[ViewportInput::from_focus(focus)]);
        // size=min(320/3,7*180/24)=52, default origin=(134,15).
        assert_eq!(
            graphics.surface().get_pixel(135, 16),
            Some(Color::opaque(10, 10, 200))
        );
    }

    #[test]
    fn chrome_layout_reserves_upper_board_and_message_board_strips() {
        // C4GraphicsSystem::RecalculateViewports: the viewport area sits
        // between the 50px upper board and the one-line message board
        // (src/C4GraphicsSystem.cpp:343-348, src/C4Constants.h:77).
        let snapshot = make_snapshot();
        let focus = &snapshot.objects[0];
        let board = ImageData::new(4, 55, vec![120; 4 * 55 * 4]);
        let hud_graphics = Arc::new(HudGraphics {
            upper_board: Some(board),
            ..HudGraphics::default()
        });
        let mut graphics = GraphicsSystem::new(
            320,
            240,
            150,
            "Chrome",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            hud_graphics,
        );
        let viewports = vec![ViewportInput::from_focus(focus)];
        graphics.render_frame(&snapshot, &viewports);
        let rect = graphics.active_viewports[0].rect;
        assert_eq!(rect.y, hud::UPPER_BOARD_HEIGHT);
        let board_height = graphics.message_board_height();
        assert_eq!(
            rect.height as i32,
            240 - hud::UPPER_BOARD_HEIGHT - board_height
        );
        assert_eq!(
            graphics.preferred_dialog_rect(None),
            SurfaceRect::new(
                0,
                hud::UPPER_BOARD_HEIGHT,
                320,
                (240 - hud::UPPER_BOARD_HEIGHT - board_height) as u32,
            )
        );
        assert_eq!(
            graphics.preferred_dialog_rect(Some(focus.owner)),
            rect,
            "mouse control narrows dialog placement to its viewport"
        );
    }

    #[test]
    fn message_board_mode_recalculates_viewport_bottom_border() {
        let snapshot = make_snapshot();
        let focus = &snapshot.objects[0];
        let board = ImageData::new(4, 55, vec![120; 4 * 55 * 4]);
        let hud_graphics = Arc::new(HudGraphics {
            upper_board: Some(board),
            ..HudGraphics::default()
        });
        let mut graphics = GraphicsSystem::new(
            320,
            240,
            150,
            "Dynamic message board",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            hud_graphics,
        );
        let line_height = graphics.hud_font().line_height();
        let viewports = vec![ViewportInput::from_focus(focus)];

        for (message_board, expected_height) in [
            (
                MessageBoardOverlay {
                    mode: MessageBoardMode::Hidden,
                    ..MessageBoardOverlay::default()
                },
                0,
            ),
            (MessageBoardOverlay::default(), line_height),
            (
                MessageBoardOverlay {
                    mode: MessageBoardMode::Continuous,
                    line_count: 3,
                    ..MessageBoardOverlay::default()
                },
                4 * line_height,
            ),
        ] {
            graphics.message_board = message_board;
            graphics.render_frame(&snapshot, &viewports);
            let rect = graphics.active_viewports[0].rect;
            assert_eq!(graphics.message_board_height(), expected_height);
            assert_eq!(
                rect.height as i32,
                240 - hud::UPPER_BOARD_HEIGHT - expected_height
            );
            assert_eq!(
                graphics.preferred_dialog_rect(None),
                SurfaceRect::new(
                    0,
                    hud::UPPER_BOARD_HEIGHT,
                    320,
                    (240 - hud::UPPER_BOARD_HEIGHT - expected_height) as u32,
                )
            );
        }
    }

    #[test]
    fn chrome_layout_tracks_hide_small_and_mini_upper_board_modes() {
        let mut snapshot = make_snapshot();
        snapshot.landscape = None;
        let focus = &snapshot.objects[0];
        for (mode, expected_top) in [
            (hud::UpperBoardMode::Hide, 0),
            (hud::UpperBoardMode::Full, 50),
            (hud::UpperBoardMode::Small, 25),
            (hud::UpperBoardMode::Mini, 0),
        ] {
            let hud_graphics = Arc::new(HudGraphics {
                upper_board: Some(ImageData::new(4, 55, vec![120; 4 * 55 * 4])),
                ..HudGraphics::default()
            });
            let mut graphics = GraphicsSystem::new(
                800,
                240,
                1_000,
                "Chrome modes",
                test_font(),
                empty_sprites(),
                empty_cursor_atlas(),
                hud_graphics,
            );
            graphics.update_overlay(&GraphicsOverlay {
                frame_text: "",
                status_text: "",
                debug_hud: false,
                viewport_overlays_visible: true,
                players: Vec::new(),
                crew_name_labels: Vec::new(),
                game_time_seconds: 0,
                message_board: MessageBoardOverlay::default(),
                clock_text: None,
                frames_per_second: None,
                upper_board_mode: mode,
                show_portraits: true,
                show_commands: true,
                show_command_keys: true,
            });
            assert_eq!(graphics.upper_board_mode, mode);
            let atlas = graphics.render_frame(&snapshot, &[ViewportInput::from_focus(focus)]);
            assert_eq!(graphics.upper_board_mode, mode);

            let message_height = graphics.message_board_height();
            let viewport = graphics.active_viewports[0].rect;
            assert_eq!(viewport.y, expected_top, "mode {mode:?}");
            assert_eq!(
                viewport.height as i32,
                240 - expected_top - message_height,
                "mode {mode:?}"
            );
            assert_eq!(
                graphics.preferred_dialog_rect(None),
                SurfaceRect::new(
                    0,
                    expected_top,
                    800,
                    (240 - expected_top - message_height) as u32,
                ),
                "mode {mode:?}"
            );

            let upper = atlas.iter().find(|entry| entry.label == "upper_board");
            let message = atlas
                .iter()
                .find(|entry| entry.label == "message_board")
                .unwrap_or_else(|| {
                    panic!(
                        "classic message-board output facet for {mode:?} at {:?}; labels: {:?}",
                        graphics.message_board_output_rect(),
                        atlas.iter().map(|entry| &entry.label).collect::<Vec<_>>()
                    )
                });
            match mode {
                hud::UpperBoardMode::Hide => {
                    let upper = upper.expect("Hide retains native raw Output facet");
                    assert_eq!((upper.width, upper.height), (800, 55));
                    assert_eq!(message.width, 800);
                }
                hud::UpperBoardMode::Full => {
                    let upper = upper.expect("Full upper-board output facet");
                    assert_eq!((upper.width, upper.height), (800, 55));
                    assert_eq!(message.width, 800);
                }
                hud::UpperBoardMode::Small => {
                    let upper = upper.expect("Small upper-board output facet");
                    assert_eq!((upper.width, upper.height), (800, 27));
                    assert_eq!(message.width, 800);
                }
                hud::UpperBoardMode::Mini => {
                    let upper = upper.expect("Mini upper-board output facet");
                    assert_eq!(upper.width + message.width, 800);
                    assert_eq!(upper.height, message.height);

                    let initialized_width = upper.width as u32;
                    graphics.set_upper_board_mode(hud::UpperBoardMode::Mini, 100 * 60 * 60);
                    assert_eq!(
                        graphics
                            .upper_board_output_rect()
                            .expect("latched Mini facet")
                            .width,
                        initialized_width,
                        "TextWidth stays fixed between fullscreen-component initializations"
                    );
                    graphics.set_upper_board_mode(hud::UpperBoardMode::Full, 100 * 60 * 60);
                    graphics.set_upper_board_mode(hud::UpperBoardMode::Mini, 100 * 60 * 60);
                    assert!(
                        graphics
                            .upper_board_output_rect()
                            .expect("reinitialized Mini facet")
                            .width
                            > initialized_width,
                        "a reinitialization latches the wider 100-hour time string"
                    );
                }
            }
        }
    }

    #[test]
    fn upper_board_relayout_keeps_small_world_centered_synchronously() {
        let mut snapshot = make_snapshot();
        snapshot.landscape = Some(Landscape::flat(40, 40));
        snapshot.objects[0].position = Vector2::new(20, 20);
        let focus = &snapshot.objects[0];
        let hud_graphics = Arc::new(HudGraphics {
            upper_board: Some(ImageData::new(4, 50, vec![120; 4 * 50 * 4])),
            ..HudGraphics::default()
        });
        let mut graphics = GraphicsSystem::new(
            320,
            200,
            1_000,
            "Small world chrome",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            hud_graphics,
        );
        graphics.render_frame(&snapshot, &[ViewportInput::from_focus(focus)]);

        let stored_world = (
            graphics.active_viewports[0].world_width,
            graphics.active_viewports[0].world_height,
        );
        assert_ne!(stored_world.1, graphics.world_height);
        graphics.set_upper_board_mode(hud::UpperBoardMode::Small, 0);

        let cell = graphics.layout_viewports(1)[0];
        let expected =
            GraphicsSystem::centered_viewport_rect_for_world(cell, stored_world.0, stored_world.1);
        assert_eq!(graphics.active_viewports[0].rect, expected);
        assert_eq!((expected.width, expected.height), (120, 120));
    }

    #[test]
    fn upper_board_relayout_clamps_no_owner_camera_at_large_world_edge() {
        let mut snapshot = make_snapshot();
        snapshot.landscape = Some(Landscape::flat(1_000, 1_000));
        let hud_graphics = Arc::new(HudGraphics {
            upper_board: Some(ImageData::new(4, 50, vec![120; 4 * 50 * 4])),
            ..HudGraphics::default()
        });
        let mut graphics = GraphicsSystem::new(
            320,
            200,
            1_000,
            "Observer edge chrome",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            hud_graphics,
        );
        graphics.render_frame(
            &snapshot,
            &[ViewportInput::ownerless(Vector2::new(500, 500), 1.0)],
        );

        let viewport = &mut graphics.active_viewports[0];
        viewport.target_x = viewport.world_width - viewport.logical_width;
        viewport.target_y = viewport.world_height - viewport.logical_height;
        let key = viewport.camera_key;
        let state = graphics
            .camera_states
            .get_mut(&key)
            .expect("observer camera");
        state.view_x = viewport.target_x;
        state.view_y = viewport.target_y;

        graphics.set_upper_board_mode(hud::UpperBoardMode::Small, 0);

        let viewport = &graphics.active_viewports[0];
        assert_eq!(
            (viewport.target_x, viewport.target_y),
            (
                viewport.world_width - viewport.logical_width,
                viewport.world_height - viewport.logical_height,
            )
        );
        assert_eq!(
            viewport.content_rect, viewport.rect,
            "a large-world observer gains no synthetic scroll border"
        );
    }

    #[test]
    fn fixed_item_visibility_combines_global_and_script_requests() {
        assert_eq!(
            player_fixed_item_visibility(false, false, false),
            (false, false, false)
        );
        assert_eq!(
            player_fixed_item_visibility(false, true, false),
            (true, false, false)
        );
        assert_eq!(
            player_fixed_item_visibility(false, false, true),
            (false, true, false)
        );
        assert_eq!(
            player_fixed_item_visibility(true, false, false),
            (true, true, true)
        );
    }

    fn viewport_layout(width: u32, height: u32, count: usize) -> Vec<SurfaceRect> {
        viewport_layout_with_dividers(width, height, count, true)
    }

    fn viewport_layout_with_dividers(
        width: u32,
        height: u32,
        count: usize,
        splitscreen_dividers: bool,
    ) -> Vec<SurfaceRect> {
        let mut graphics = test_graphics(width, height, height as i32, "Viewport layout");
        graphics.set_renderer_config(true, splitscreen_dividers);
        graphics.layout_viewports(count)
    }

    #[test]
    fn splitscreen_layout_matches_cpp_for_two_players() {
        assert_eq!(
            viewport_layout(800, 600, 2),
            vec![
                SurfaceRect::new(0, 0, 396, 600),
                SurfaceRect::new(400, 0, 400, 600),
            ]
        );
    }

    #[test]
    fn splitscreen_layout_matches_cpp_for_three_players() {
        assert_eq!(
            viewport_layout(800, 600, 3),
            vec![
                SurfaceRect::new(0, 0, 262, 600),
                SurfaceRect::new(266, 0, 262, 600),
                SurfaceRect::new(532, 0, 266, 600),
            ]
        );
    }

    #[test]
    fn splitscreen_layout_matches_cpp_for_four_players() {
        assert_eq!(
            viewport_layout(800, 600, 4),
            vec![
                SurfaceRect::new(0, 0, 396, 296),
                SurfaceRect::new(400, 0, 400, 296),
                SurfaceRect::new(0, 300, 396, 300),
                SurfaceRect::new(400, 300, 400, 300),
            ]
        );
    }

    #[test]
    fn splitscreen_layout_leaves_cpp_integer_division_remainder_unassigned() {
        assert_eq!(
            viewport_layout(801, 601, 4),
            vec![
                SurfaceRect::new(0, 0, 396, 296),
                SurfaceRect::new(400, 0, 400, 296),
                SurfaceRect::new(0, 300, 396, 300),
                SurfaceRect::new(400, 300, 400, 300),
            ]
        );
    }

    #[test]
    fn disabled_splitscreen_dividers_remove_four_pixel_layout_gaps() {
        assert_eq!(
            viewport_layout_with_dividers(800, 600, 2, false),
            vec![
                SurfaceRect::new(0, 0, 400, 600),
                SurfaceRect::new(400, 0, 400, 600),
            ]
        );
        assert_eq!(
            viewport_layout_with_dividers(800, 600, 4, false),
            vec![
                SurfaceRect::new(0, 0, 400, 300),
                SurfaceRect::new(400, 0, 400, 300),
                SurfaceRect::new(0, 300, 400, 300),
                SurfaceRect::new(400, 300, 400, 300),
            ]
        );
    }

    #[test]
    fn sprite_atlas_captures_back_buffer_and_object() {
        let mut snapshot = make_snapshot();
        snapshot.objects[0].vertices = vec![
            ObjectVertex::new(-4, -4),
            ObjectVertex::new(4, -4),
            ObjectVertex::new(4, 4),
            ObjectVertex::new(-4, 4),
        ];
        let focus = &snapshot.objects[0];
        let mut graphics = test_graphics(120, 80, 60, "Atlas Scenario");

        let viewports = vec![ViewportInput::from_focus(focus)];
        let atlas = graphics.render_frame(&snapshot, &viewports);

        assert!(atlas.iter().any(|entry| entry.label == "back_buffer"));
        let object_label = format!("object#{}:def={}", focus.id.as_u64(), focus.definition_id);
        assert!(
            atlas.iter().any(|entry| entry.label == object_label),
            "expected atlas entry for {object_label}, got labels: {:?}",
            atlas.iter().map(|entry| &entry.label).collect::<Vec<_>>()
        );
    }

    #[test]
    fn viewport_tracks_focus_vertically() {
        let mut snapshot = make_snapshot();
        snapshot.objects[0].position = Vector2::new(100, 260);
        snapshot.landscape = Some(Landscape::flat(256, 280));
        let mut graphics = test_graphics(320, 180, 150, "Test Scenario");
        let focus = &snapshot.objects[0];
        let viewports = vec![ViewportInput::from_focus(focus)];
        graphics.render_frame(&snapshot, &viewports);

        let (_, viewport_y) = graphics.viewport();
        assert!(viewport_y > 0);
    }

    fn initialized_camera(view_x: i32, view_y: i32, width: i32, height: i32) -> CameraState {
        CameraState {
            d_view_x: itofix(view_x),
            d_view_y: itofix(view_y),
            view_x,
            view_y,
            view_width: width,
            view_height: height,
        }
    }

    #[test]
    fn viewport_edge_scroll_uses_inclusive_clamped_pixel_zones() {
        let viewport = SurfaceRect::new(10, 20, 100, 50);
        assert_eq!(
            viewport_edge_scroll(viewport, GuiPoint::new(10.0, 45.0)),
            Some(ViewportEdgeScroll {
                delta: Vector2::new(-10, 0),
                cursor: MouseCursorPhase::Left,
                edge_mask: 0b0001,
            })
        );
        assert_eq!(
            viewport_edge_scroll(viewport, GuiPoint::new(108.1, 68.1)),
            Some(ViewportEdgeScroll {
                delta: Vector2::new(10, 10),
                cursor: MouseCursorPhase::DownRight,
                edge_mask: 0b1100,
            }),
            "ceil before subtraction reaches the inclusive right/bottom pixel"
        );
        assert_eq!(
            viewport_edge_scroll(viewport, GuiPoint::new(-100.0, -100.0)),
            Some(ViewportEdgeScroll {
                delta: Vector2::new(-10, -10),
                cursor: MouseCursorPhase::UpLeft,
                edge_mask: 0b0011,
            }),
            "points outside the owning viewport are clamped like C4MouseControl"
        );
        assert_eq!(
            viewport_edge_scroll(viewport, GuiPoint::new(11.0, 21.0)),
            None
        );

        let degenerate =
            viewport_edge_scroll(SurfaceRect::new(0, 0, 1, 1), GuiPoint::new(0.0, 0.0))
                .expect("one-pixel viewport is all four edges");
        assert_eq!(degenerate.delta, Vector2::ZERO);
        assert_eq!(degenerate.cursor, MouseCursorPhase::DownRight);
        assert_eq!(
            degenerate.steps().collect::<Vec<_>>(),
            vec![
                Vector2::new(-10, 0),
                Vector2::new(0, -10),
                Vector2::new(10, 0),
                Vector2::new(0, 10),
            ],
            "all four native ScrollView calls survive the zero net delta"
        );
    }

    #[test]
    fn retained_viewport_coordinate_is_not_reclamped_after_resize() {
        assert_eq!(
            viewport_edge_scroll_at(99, 25, 100, 50),
            Some(ViewportEdgeScroll {
                delta: Vector2::new(10, 0),
                cursor: MouseCursorPhase::Right,
                edge_mask: 0b0100,
            })
        );
        assert_eq!(
            viewport_edge_scroll_at(99, 25, 120, 50),
            None,
            "the old right pixel becomes interior when the viewport grows"
        );
        assert_eq!(
            viewport_edge_scroll_at(119, 25, 120, 50),
            Some(ViewportEdgeScroll {
                delta: Vector2::new(10, 0),
                cursor: MouseCursorPhase::Right,
                edge_mask: 0b0100,
            })
        );
    }

    #[test]
    fn scrolling_camera_has_no_dead_zone_and_uses_fixed_border() {
        let mut following = initialized_camera(100, 100, 100, 80);
        assert_eq!(
            following
                .update(
                    155,
                    140,
                    100,
                    80,
                    500,
                    500,
                    VIEWPORT_SCROLL_BORDER,
                    false,
                    1
                )
                .0,
            100,
            "normal following retains the eight-pixel dead zone"
        );

        let mut scrolling = initialized_camera(100, 100, 100, 80);
        assert_eq!(
            scrolling
                .update(155, 140, 100, 80, 500, 500, VIEWPORT_SCROLL_BORDER, true, 1)
                .0,
            105,
            "C4PVM_Scrolling targets the player center exactly"
        );

        let mut edge = CameraState::new(500, 500, 100, 80);
        assert_eq!(
            edge.update(40, 250, 100, 80, 500, 500, VIEWPORT_SCROLL_BORDER, true, 1)
                .0,
            -10,
            "scrolling mode keeps the full 40px fullscreen extra bound"
        );
    }

    #[test]
    fn observer_scroll_uses_physical_classification_across_film_assignment() {
        let snapshot = camera_world_snapshot();
        let mut graphics = test_graphics(100, 80, 80, "Observer scroll");
        graphics.render_frame(
            &snapshot,
            &[ViewportInput::new(
                OWNER_NONE,
                Vector2::new(500, 500),
                1.0,
                &snapshot.objects[0],
            )],
        );
        let key = graphics.active_viewports[0].camera_key;
        let before = graphics.camera_states[&key];
        let before_projection = graphics.active_viewport_projections()[0];
        assert!(before_projection.is_no_owner_viewport);
        assert!(graphics.scroll_observer_viewport(0, Vector2::new(-10, 10)));
        let after = graphics.camera_states[&key];
        let after_projection = graphics.active_viewport_projections()[0];
        assert_eq!(after.view_x, before.view_x - 10);
        assert_eq!(after.view_y, before.view_y + 10);
        assert_eq!(after_projection.target_x, before_projection.target_x - 10);
        assert_eq!(after_projection.target_y, before_projection.target_y + 10);
        assert_eq!(
            after_projection.content_origin_x,
            before_projection.content_origin_x - 10.0
        );
        assert_eq!(
            after_projection.content_origin_y,
            before_projection.content_origin_y + 10.0
        );

        graphics.camera_states.get_mut(&key).unwrap().view_x = 0;
        assert!(graphics.scroll_observer_viewport(0, Vector2::new(-10, 0)));
        assert_eq!(graphics.camera_states[&key].view_x, 0);

        graphics.active_viewports[0].owner = 0;
        assert!(graphics.scroll_observer_viewport(0, Vector2::new(10, 0)));
        let film_projection = graphics.active_viewport_projections()[0];
        assert_eq!(film_projection.owner, 0);
        assert!(film_projection.is_no_owner_viewport);

        graphics.active_viewports[0].is_no_owner_viewport = false;
        assert!(!graphics.scroll_observer_viewport(0, Vector2::new(10, 0)));
        assert!(!graphics.scroll_observer_viewport(99, Vector2::new(10, 0)));
    }

    // C4Viewport gives each window its own object, so a detached console
    // window must address its viewport by identity — the list index is the
    // rendered layout order and the owner repeats.
    #[test]
    fn detached_viewport_projection_is_addressable_by_physical_identity() {
        let snapshot = camera_world_snapshot();
        let mut graphics = test_graphics(100, 80, 80, "Identity addressing");
        // Two viewports following the *same* player, as a console second
        // window on an already-viewed player produces.
        graphics.render_frame(
            &snapshot,
            &[
                ViewportInput::new(0, Vector2::new(500, 500), 1.0, &snapshot.objects[0])
                    .with_physical_camera_identity(41, 0),
                ViewportInput::new(0, Vector2::new(500, 500), 1.0, &snapshot.objects[0])
                    .with_physical_camera_identity(42, 1),
            ],
        );

        let projections = graphics.active_viewport_projections();
        assert_eq!(projections.len(), 2);
        assert_eq!(
            projections[0].owner, projections[1].owner,
            "duplicate owners are exactly the case the index cannot disambiguate"
        );
        assert_eq!(projections[0].identity, Some(41));
        assert_eq!(projections[1].identity, Some(42));

        // Each identity resolves to its own projection, whatever its index.
        assert_eq!(
            graphics
                .viewport_projection_for_identity(41)
                .expect("identity 41 is live")
                .index,
            0
        );
        assert_eq!(
            graphics
                .viewport_projection_for_identity(42)
                .expect("identity 42 is live")
                .index,
            1
        );
        assert!(graphics.viewport_projection_for_identity(99).is_none());

        // Re-rendering with the layout order swapped moves the indices but not
        // the identities, which is the whole point.
        graphics.render_frame(
            &snapshot,
            &[
                ViewportInput::new(0, Vector2::new(500, 500), 1.0, &snapshot.objects[0])
                    .with_physical_camera_identity(42, 1),
                ViewportInput::new(0, Vector2::new(500, 500), 1.0, &snapshot.objects[0])
                    .with_physical_camera_identity(41, 0),
            ],
        );
        assert_eq!(
            graphics
                .viewport_projection_for_identity(41)
                .expect("identity 41 survives a relayout")
                .index,
            1,
            "the index moved with the layout"
        );
        assert_eq!(
            graphics
                .viewport_projection_for_identity(42)
                .expect("identity 42 survives a relayout")
                .index,
            0
        );
    }

    // `C4GraphicsSystem::Execute` runs `cvp->Execute()` per viewport
    // (C4GraphicsSystem.cpp:167-169) and each console viewport draws through
    // *its own* window context, so one identity reaches one target.
    // `C4Viewport::Execute` sets `cgo` to the whole window extent
    // (C4Viewport.cpp:1146), and the message board and upper board are gated
    // on `Application.isFullScreen` (C4GraphicsSystem.cpp:171-177) — so a
    // detached window reserves nothing for them and is never split.
    #[test]
    fn detached_viewport_render_targets_only_requested_physical_identity() {
        let mut snapshot = camera_world_snapshot();
        let mut second = snapshot.objects[0].clone();
        second.id = ObjectId::new(2);
        second.position = Vector2::new(900, 500);
        snapshot.objects.push(second);
        let mut graphics = test_graphics(100, 80, 80, "Detached render");

        // Duplicate owners, distinct identities — exactly what a console
        // second window on an already-viewed player produces.
        let inputs = || {
            vec![
                ViewportInput::new(0, Vector2::new(500, 500), 1.0, &snapshot.objects[0])
                    .with_physical_camera_identity(41, 0),
                ViewportInput::new(0, Vector2::new(900, 500), 1.0, &snapshot.objects[1])
                    .with_physical_camera_identity(42, 1),
            ]
        };

        let first = graphics
            .render_detached_viewport(&snapshot, &inputs(), 41, 320, 200)
            .expect("identity 41 is in the supplied list");
        let second_frame = graphics
            .render_detached_viewport(&snapshot, &inputs(), 42, 320, 200)
            .expect("identity 42 is in the supplied list");

        // The supplied target is filled whole: no split layout, and no
        // upper-board/message-board reservation.
        assert_eq!(first.surface.width(), 320);
        assert_eq!(first.surface.height(), 200);
        assert_eq!(first.projection.logical_width, 320);
        assert_eq!(first.projection.logical_height, 200);
        assert_eq!(
            first.projection.rect,
            SurfaceRect::new(0, 0, 320, 200),
            "a detached viewport owns its entire window"
        );

        // Each call drew the viewport it was asked for, not the first one.
        assert_eq!(first.projection.identity, Some(41));
        assert_eq!(second_frame.projection.identity, Some(42));
        assert_ne!(
            first.projection.target_x, second_frame.projection.target_x,
            "the two identities follow different world positions"
        );

        // An identity that is not in the supplied list draws nothing rather
        // than falling back to the first viewport.
        assert!(graphics
            .render_detached_viewport(&snapshot, &inputs(), 99, 320, 200)
            .is_none());

        // The frame that was drawn is the frame the window's pointer input is
        // converted through: `ViewX + static_cast<int32_t>(local / scale)`
        // per viewport (C4Viewport.cpp:112,181,192). Two windows showing the
        // same player must not share one projection.
        assert_eq!(
            first
                .projection
                .pointer_projection(1.0)
                .world_position(7, 3),
            (first.projection.target_x + 7, first.projection.target_y + 3)
        );
        assert_ne!(
            first
                .projection
                .pointer_projection(1.0)
                .world_position(7, 3),
            second_frame
                .projection
                .pointer_projection(1.0)
                .world_position(7, 3)
        );
        // The window's own presenter scale divides before the origin is added.
        assert_eq!(
            first
                .projection
                .pointer_projection(2.0)
                .world_position(7, 3),
            (first.projection.target_x + 3, first.projection.target_y + 1)
        );

        // A detached pass must not disturb the fullscreen layout state the
        // other windows and the audibility reduction read.
        graphics.render_frame(&snapshot, &inputs());
        let fullscreen = graphics.active_viewport_projections();
        assert_eq!(fullscreen.len(), 2);
        let _ = graphics
            .render_detached_viewport(&snapshot, &inputs(), 41, 320, 200)
            .expect("identity 41 is still live");
        assert_eq!(
            graphics.active_viewport_projections(),
            fullscreen,
            "the detached pass restored the fullscreen viewport records"
        );
    }

    // Two `Application.isFullScreen` gates decide what a console viewport
    // window looks like on a map smaller than itself, and both of them are
    // easy to lose because the fullscreen arm is the one a port writes first.
    //
    // `C4GraphicsSystem::RecalculateViewports` — the only writer of the
    // landscape-extent cap, the layout cell and `DrawX`/`DrawY` — opens with
    // `if (!Application.isFullScreen) return;` (C4GraphicsSystem.cpp:335-336),
    // so a console viewport is never capped to the landscape and always draws
    // at its target's origin. And `C4Viewport::UpdateViewPosition` centres an
    // ownerless view on an undersized map only `if (Application.isFullScreen)`
    // (C4Viewport.cpp:1237,1246); otherwise it runs
    // `min(ViewX, GBackWdt - ViewWdt)` then `max(ViewX, 0)` and pins it at 0.
    #[test]
    fn detached_viewport_window_is_never_capped_or_centred_on_a_small_map() {
        let mut snapshot = camera_world_snapshot();
        // A map smaller than the window is what makes both gates observable.
        snapshot.landscape = Some(Landscape::flat(200, 150));
        let mut graphics = test_graphics(320, 200, 80, "Small map");
        let inputs = || {
            vec![ViewportInput::ownerless(Vector2::new(100, 75), 1.0)
                .with_physical_camera_identity(7, 0)]
        };

        let detached = graphics
            .render_detached_viewport(&snapshot, &inputs(), 7, 320, 200)
            .expect("identity 7 is in the supplied list");
        assert_eq!(
            detached.projection.rect,
            SurfaceRect::new(0, 0, 320, 200),
            "RecalculateViewports never runs in console mode, so no landscape cap"
        );
        assert_eq!(
            (detached.projection.target_x, detached.projection.target_y),
            (0, 0),
            "an undersized map pins a detached ownerless view at the origin"
        );

        // The fullscreen pass keeps both behaviours: it caps the output to the
        // landscape plus its scroll borders and centres the view on the map.
        graphics.render_frame(&snapshot, &inputs());
        let fullscreen = graphics.active_viewport_projections()[0];
        assert_ne!(
            fullscreen.rect,
            SurfaceRect::new(0, 0, 320, 200),
            "the fullscreen layout still caps a viewport to the landscape"
        );
        assert!(
            fullscreen.target_x < 0,
            "the fullscreen arm centres an undersized map, giving a negative origin"
        );
    }

    #[test]
    fn observer_scroll_queued_before_projection_moves_the_first_rendered_camera() {
        let snapshot = camera_world_snapshot();
        let new_graphics = || test_graphics(100, 80, 80, "Queued observer scroll");
        let input = || {
            ViewportInput::new(
                OWNER_NONE,
                Vector2::new(500, 500),
                1.0,
                &snapshot.objects[0],
            )
            .with_physical_camera_identity(41, 0)
        };

        let mut baseline = new_graphics();
        baseline.render_frame(&snapshot, &[input()]);
        let baseline_projection = baseline.active_viewport_projections()[0];

        let mut queued = new_graphics();
        assert!(queued.active_viewports.is_empty());
        assert!(queued.scroll_observer_viewport(0, Vector2::new(5, 0)));
        assert!(queued.scroll_observer_viewport(0, Vector2::new(10, -5)));
        assert_eq!(queued.pending_primary_observer_scroll, Vector2::new(15, -5));
        let mut rebuilt = new_graphics();
        rebuilt.inherit_pending_observer_scroll(&queued);
        queued = new_graphics();
        queued.inherit_pending_observer_scroll(&rebuilt);
        assert_eq!(
            queued.pending_primary_observer_scroll,
            Vector2::new(15, -5),
            "consecutive resize rebuilds preserve unprojected FreeView input"
        );
        queued.render_frame(&snapshot, &[input()]);

        let projection = queued.active_viewport_projections()[0];
        assert_eq!(projection.target_x, baseline_projection.target_x + 15);
        assert_eq!(projection.target_y, baseline_projection.target_y - 5);
        assert_eq!(queued.pending_primary_observer_scroll, Vector2::ZERO);
    }

    #[test]
    fn queued_observer_scroll_replaces_a_stale_owned_projection() {
        let snapshot = camera_world_snapshot();
        let new_graphics = || test_graphics(100, 80, 80, "Stale observer projection");
        let ownerless = || {
            ViewportInput::ownerless(Vector2::new(500, 500), 1.0)
                .with_physical_camera_identity(42, 0)
        };

        let mut baseline = new_graphics();
        baseline.render_frame(&snapshot, &[ownerless()]);
        let baseline_projection = baseline.active_viewport_projections()[0];

        let mut transitioning = new_graphics();
        transitioning.render_frame(
            &snapshot,
            &[
                ViewportInput::new(0, Vector2::new(500, 500), 1.0, &snapshot.objects[0])
                    .with_physical_camera_identity(41, 0),
            ],
        );
        assert!(!transitioning.active_viewports[0].is_no_owner_viewport);
        transitioning.drop_physical_camera(41);
        let delta = Vector2::new(5, -5);
        assert!(!transitioning.scroll_observer_viewport(0, delta));
        transitioning.queue_primary_observer_scroll(delta);
        transitioning.render_frame(&snapshot, &[ownerless()]);

        let projection = transitioning.active_viewport_projections()[0];
        assert_eq!(projection.target_x, baseline_projection.target_x + 5);
        assert_eq!(projection.target_y, baseline_projection.target_y - 5);
        assert_eq!(transitioning.pending_primary_observer_scroll, Vector2::ZERO);
    }

    #[test]
    fn ownerless_viewport_renders_without_an_object_anchor() {
        let mut snapshot = make_snapshot();
        snapshot.objects.clear();
        snapshot.render_order.clear();
        let mut graphics = test_graphics(320, 180, 150, "Anchor-free observer");
        let input = ViewportInput::ownerless(Vector2::new(128, 75), 1.0)
            .with_camera_identity(OWNER_NONE, 0);
        assert!(input.focus.is_none());

        graphics.render_frame(&snapshot, &[input]);

        let projection = graphics.active_viewport_projections();
        assert_eq!(projection.len(), 1);
        assert_eq!(projection[0].owner, OWNER_NONE);
        assert!(projection[0].is_no_owner_viewport);
        assert!(graphics.active_viewports[0].focus.is_none());
        let capture = graphics
            .render_full_landscape(&snapshot)
            .expect("focusless observer still supports full-map capture");
        assert_eq!((capture.width(), capture.height()), (256, 120));
    }

    #[test]
    fn owned_focusless_scrolling_viewport_renders_without_live_objects() {
        let mut snapshot = make_snapshot();
        snapshot.objects.clear();
        snapshot.render_order.clear();
        let mut graphics = test_graphics(100, 80, 80, "Focusless player scroll");
        let input = ViewportInput::owned_without_focus(0, Vector2::new(128, 60), 1.0)
            .with_scrolling(true)
            .with_camera_identity(0, 0);
        assert!(input.focus.is_none());

        graphics.render_frame(&snapshot, &[input]);

        let projection = graphics.active_viewport_projections();
        assert_eq!(projection.len(), 1);
        assert_eq!(projection[0].owner, 0);
        assert!(!projection[0].is_no_owner_viewport);
        assert!(graphics.active_viewports[0].focus.is_none());
    }

    #[test]
    fn camera_smoothing_uses_cpp_fixed_divisor_four_sequence() {
        // C4Viewport.cpp:1203-1206 retains the 16.16 residue and projects
        // each graphics pass with fixtoi. A 0 -> 100 target therefore does
        // not follow the old f32 alpha-0.2 sequence (20,36,48.8).
        let mut camera = initialized_camera(0, 0, 100, 1);
        let mut visible = Vec::new();
        for _ in 0..3 {
            visible.push(
                camera
                    .update(150, 0, 100, 1, 1_000, 1, VIEWPORT_SCROLL_BORDER, false, 4)
                    .0,
            );
        }
        assert_eq!(visible, vec![25, 44, 58]);
    }

    #[test]
    fn camera_smoothing_has_no_small_or_jump_snap_thresholds() {
        let mut one_pixel = initialized_camera(0, 0, 100, 1);
        let mut one_pixel_visible = Vec::new();
        for _ in 0..3 {
            one_pixel_visible.push(
                one_pixel
                    .update(51, 0, 100, 1, 1_000, 1, VIEWPORT_SCROLL_BORDER, false, 4)
                    .0,
            );
        }
        assert_eq!(one_pixel_visible, vec![0, 0, 1]);

        let mut jump = initialized_camera(0, 0, 100, 1);
        assert_eq!(
            jump.update(450, 0, 100, 1, 1_000, 1, VIEWPORT_SCROLL_BORDER, false, 4)
                .0,
            100,
            "a 400px target delta is quartered rather than snapped"
        );
    }

    #[test]
    fn camera_scroll_smooth_is_clamped_like_cpp_config() {
        let mut zero = initialized_camera(0, 0, 100, 1);
        assert_eq!(
            zero.update(150, 0, 100, 1, 1_000, 1, VIEWPORT_SCROLL_BORDER, false, 0)
                .0,
            100,
            "ScrollSmooth=0 clamps to divisor one"
        );

        let mut huge = initialized_camera(0, 0, 100, 1);
        assert_eq!(
            huge.update(150, 0, 100, 1, 1_000, 1, VIEWPORT_SCROLL_BORDER, false, 500)
                .0,
            2,
            "ScrollSmooth values above 50 clamp to divisor 50"
        );
    }

    #[test]
    fn camera_dead_zone_delays_slow_elevator_follow_per_render() {
        // With a 100x80 viewport the shared range is 8px. The focus can move
        // eight pixels without changing the target. At nine pixels the target
        // advances to 451, whose fixed projection remains 450 for two more
        // graphics passes before rounding to 451 on the third.
        let mut camera = initialized_camera(450, 460, 100, 80);
        assert_eq!(
            camera
                .update(
                    508,
                    500,
                    100,
                    80,
                    1_000,
                    1_000,
                    VIEWPORT_SCROLL_BORDER,
                    false,
                    4
                )
                .0,
            450
        );
        let repeated = (0..3)
            .map(|_| {
                camera
                    .update(
                        509,
                        500,
                        100,
                        80,
                        1_000,
                        1_000,
                        VIEWPORT_SCROLL_BORDER,
                        false,
                        4,
                    )
                    .0
            })
            .collect::<Vec<_>>();
        assert_eq!(repeated, vec![450, 450, 451]);
    }

    #[test]
    fn camera_edge_bounds_progress_through_the_cpp_scroll_border() {
        let first_view = |center_x| {
            let mut camera = CameraState::new(500, 500, 100, 80);
            camera
                .update(
                    center_x,
                    250,
                    100,
                    80,
                    500,
                    500,
                    VIEWPORT_SCROLL_BORDER,
                    false,
                    DEFAULT_SCROLL_SMOOTH,
                )
                .0
        };
        assert_eq!(first_view(0), -40);
        assert_eq!(first_view(20), -20);
        assert_eq!(first_view(40), 0);

        // The negative dViewX makes C++ take the coupled initialization
        // branch on the next pass, snapping both axes to their new targets.
        let mut camera = CameraState::new(500, 500, 100, 80);
        assert_eq!(
            camera
                .update(0, 250, 100, 80, 500, 500, VIEWPORT_SCROLL_BORDER, false, 4)
                .0,
            -40
        );
        assert_eq!(
            camera
                .update(
                    100,
                    250,
                    100,
                    80,
                    500,
                    500,
                    VIEWPORT_SCROLL_BORDER,
                    false,
                    4
                )
                .0,
            42
        );
    }

    fn camera_world_snapshot() -> SimulationSnapshot {
        let mut snapshot = make_snapshot();
        snapshot.objects[0].position = Vector2::new(500, 500);
        snapshot.landscape = Some(Landscape::flat(1_000, 1_000));
        snapshot
    }

    #[test]
    fn camera_state_survives_focus_changes_in_the_same_viewport_slot() {
        let mut snapshot = camera_world_snapshot();
        let mut second = snapshot.objects[0].clone();
        second.id = ObjectId::new(2);
        second.position = Vector2::new(900, 500);
        snapshot.objects.push(second);
        let mut graphics = test_graphics(100, 80, 80, "Camera focus");

        graphics.render_frame(
            &snapshot,
            &[ViewportInput::new(
                0,
                Vector2::new(500, 500),
                1.0,
                &snapshot.objects[0],
            )],
        );
        graphics.render_frame(
            &snapshot,
            &[ViewportInput::new(
                0,
                Vector2::new(900, 500),
                1.0,
                &snapshot.objects[1],
            )],
        );

        let camera = graphics
            .camera_states
            .get(&CameraKey::Player { owner: 0, slot: 0 })
            .expect("stable viewport camera");
        assert_eq!(camera.view_x, 548);
        assert_eq!(graphics.active_viewports[0].viewport_x, 548.0);
    }

    #[test]
    fn film_view_retarget_preserves_physical_camera_identity_and_classification() {
        let mut snapshot = camera_world_snapshot();
        let mut second = snapshot.objects[0].clone();
        second.id = ObjectId::new(2);
        second.owner = 1;
        second.position = Vector2::new(900, 500);
        snapshot.objects.push(second);
        let mut graphics = test_graphics(100, 80, 80, "Film view");

        graphics.render_frame(
            &snapshot,
            &[
                ViewportInput::new(0, Vector2::new(500, 500), 1.0, &snapshot.objects[0])
                    .with_physical_camera_identity(41, 0),
            ],
        );
        graphics.render_frame(
            &snapshot,
            &[
                ViewportInput::new(1, Vector2::new(900, 500), 1.0, &snapshot.objects[1])
                    .with_physical_camera_identity(41, 0),
            ],
        );
        assert_eq!(graphics.active_viewports[0].owner, 1);
        assert_eq!(
            graphics
                .camera_states
                .get(&CameraKey::Physical {
                    identity: 41,
                    slot: 0,
                })
                .expect("physical viewport camera survives player switch")
                .view_x,
            548
        );
        assert!(!graphics.camera_states.contains_key(&CameraKey::Physical {
            identity: 42,
            slot: 0,
        }));

        let mut ownerless =
            ViewportInput::new(0, Vector2::new(900, 500), 1.0, &snapshot.objects[1])
                .with_physical_camera_identity(41, 0);
        ownerless.owner = OWNER_NONE;
        graphics.render_frame(&snapshot, &[ownerless]);
        assert_eq!(graphics.active_viewports[0].owner, OWNER_NONE);
        assert_eq!(
            graphics
                .camera_states
                .get(&CameraKey::Physical {
                    identity: 41,
                    slot: 0,
                })
                .expect("temporary NO_OWNER keeps the owned viewport camera")
                .view_x,
            548,
            "temporary NO_OWNER freezes rather than reclassifying the viewport"
        );
        graphics.drop_physical_camera(41);
        assert!(!graphics.camera_states.contains_key(&CameraKey::Physical {
            identity: 41,
            slot: 0,
        }));
    }

    #[test]
    fn film_assigned_ownerless_tracks_player_and_applies_current_frame_offset() {
        let snapshot = camera_world_snapshot();
        let mut graphics = test_graphics(100, 80, 80, "Ownerless film camera");
        let mut input = ViewportInput::new(0, Vector2::new(900, 500), 1.0, &snapshot.objects[0])
            .with_offset(Vector2::new(7, -4))
            .with_physical_camera_identity(42, 0);
        // SetFilmView changes Player but preserves fIsNoOwnerViewport.
        input.is_no_owner_viewport = true;

        graphics.render_frame(&snapshot, &[input]);

        let camera = graphics
            .camera_states
            .get(&CameraKey::Physical {
                identity: 42,
                slot: 0,
            })
            .expect("film-assigned ownerless camera");
        assert_eq!((camera.view_x, camera.view_y), (842, 460));
        let projection = graphics.active_viewport_projections()[0];
        assert_eq!(projection.owner, 0);
        assert!(projection.is_no_owner_viewport);
        assert_eq!((projection.target_x, projection.target_y), (849, 456));
    }

    #[test]
    fn script_view_offset_applies_after_camera_smoothing() {
        // C4Viewport::Execute computes/smooths dViewX/Y first, then adds
        // ViewOffsX/Y only to the rendered ViewX/Y (C4Viewport.cpp:1183-1214).
        // Earthquake shake must therefore move the current frame instantly
        // without feeding the displacement back into the smooth camera.
        let snapshot = camera_world_snapshot();
        let mut graphics = test_graphics(100, 80, 80, "Script view offset");
        let base = ViewportInput::new(0, Vector2::new(500, 500), 1.0, &snapshot.objects[0]);
        graphics.render_frame(&snapshot, &[base]);
        let camera_before = *graphics
            .camera_states
            .get(&CameraKey::Player { owner: 0, slot: 0 })
            .expect("camera state after baseline render");

        let shaken = ViewportInput::new(0, Vector2::new(500, 500), 1.0, &snapshot.objects[0])
            .with_offset(Vector2::new(7, -4));
        graphics.render_frame(&snapshot, &[shaken]);

        let camera_after = graphics
            .camera_states
            .get(&CameraKey::Player { owner: 0, slot: 0 })
            .expect("camera state after shaken render");
        assert_eq!(camera_after.view_x, camera_before.view_x);
        assert_eq!(camera_after.view_y, camera_before.view_y);
        assert_eq!(
            graphics.active_viewports[0].viewport_x,
            camera_before.view_x as f32 + 7.0
        );
        assert_eq!(
            graphics.active_viewports[0].viewport_y,
            camera_before.view_y as f32 - 4.0
        );
    }

    #[test]
    fn camera_state_survives_a_render_where_the_viewport_is_absent() {
        let mut snapshot = camera_world_snapshot();
        let mut second = snapshot.objects[0].clone();
        second.id = ObjectId::new(2);
        second.position = Vector2::new(900, 500);
        snapshot.objects.push(second);
        let mut graphics = test_graphics(100, 80, 80, "Camera absence");
        graphics.render_frame(
            &snapshot,
            &[ViewportInput::new(
                0,
                Vector2::new(500, 500),
                1.0,
                &snapshot.objects[0],
            )],
        );

        let mut absent = snapshot.clone();
        absent.objects.clear();
        graphics.render_frame(&absent, &[]);

        graphics.render_frame(
            &snapshot,
            &[ViewportInput::new(
                0,
                Vector2::new(900, 500),
                1.0,
                &snapshot.objects[1],
            )],
        );
        assert_eq!(
            graphics
                .camera_states
                .get(&CameraKey::Player { owner: 0, slot: 0 })
                .expect("camera retained across missed draw")
                .view_x,
            548
        );
    }

    #[test]
    fn camera_edge_border_remains_tiled_outside_world_content() {
        let mut snapshot = make_snapshot();
        snapshot.objects[0].position = Vector2::new(0, 250);
        snapshot.landscape = Some(Landscape::flat(500, 500));
        let background_color = Color::opaque(73, 41, 19);
        let background = ImageData::new(
            1,
            1,
            vec![
                background_color.r,
                background_color.g,
                background_color.b,
                background_color.a,
            ],
        );
        let mut graphics = GraphicsSystem::new(
            100,
            80,
            80,
            "Camera border",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            Arc::new(HudGraphics {
                background: Some(background),
                ..HudGraphics::default()
            }),
        );
        graphics.render_frame(
            &snapshot,
            &[ViewportInput::from_focus(&snapshot.objects[0])],
        );

        let camera = graphics
            .camera_states
            .get(&CameraKey::Player { owner: 0, slot: 0 })
            .expect("camera state");
        assert_eq!(camera.view_x, -40);
        assert_eq!(graphics.active_viewports[0].content_rect.x, 40);
        assert_eq!(graphics.active_viewports[0].content_rect.width, 60);
        assert_eq!(graphics.surface().get_pixel(0, 0), Some(background_color));
        assert_ne!(
            graphics.surface().get_pixel(40, 0),
            Some(background_color),
            "in-world sky starts after the tiled border"
        );
    }

    #[test]
    fn no_owner_viewport_stays_centered_without_free_scroll_input() {
        let mut snapshot = make_snapshot();
        snapshot.objects[0].owner = OWNER_NONE;
        snapshot.objects[0].position = Vector2::new(0, 0);
        snapshot.landscape = Some(Landscape::flat(500, 500));
        let mut graphics = test_graphics(100, 80, 80, "Observer");
        graphics.render_frame(
            &snapshot,
            &[ViewportInput::from_focus(&snapshot.objects[0])],
        );
        let camera = graphics
            .camera_states
            .get(&CameraKey::Player {
                owner: OWNER_NONE,
                slot: 0,
            })
            .expect("no-owner camera");
        assert_eq!((camera.view_x, camera.view_y), (200, 210));
    }

    #[test]
    fn viewport_zoom_uses_cpp_ceil_extent_without_resetting_fixed_state() {
        let snapshot = camera_world_snapshot();
        let mut graphics = test_graphics(100, 80, 80, "Camera scale");
        graphics.render_frame(
            &snapshot,
            &[ViewportInput::new(
                0,
                Vector2::new(500, 500),
                1.5,
                &snapshot.objects[0],
            )],
        );
        let camera = graphics
            .camera_states
            .get(&CameraKey::Player { owner: 0, slot: 0 })
            .expect("scaled camera");
        assert_eq!((camera.view_width, camera.view_height), (67, 54));
        assert_ne!(camera.d_view_x, itofix(CAMERA_UNINITIALIZED));
    }

    #[test]
    fn viewport_clamps_to_world_height() {
        let mut snapshot = make_snapshot();
        snapshot.objects[0].position = Vector2::new(100, 30);
        snapshot.landscape = Some(Landscape::flat(256, 200));
        let mut graphics = test_graphics(320, 180, 150, "Test Scenario");
        let focus = &snapshot.objects[0];
        let viewports = vec![ViewportInput::from_focus(focus)];
        graphics.render_frame(&snapshot, &viewports);
        let (_, top_view) = graphics.viewport();
        assert_eq!(top_view, 0);

        let mut snapshot = make_snapshot();
        snapshot.objects[0].position = Vector2::new(100, 360);
        snapshot.landscape = Some(Landscape::flat(256, 360));
        let mut graphics = test_graphics(320, 180, 150, "Test Scenario");
        let focus = &snapshot.objects[0];
        let viewports = vec![ViewportInput::from_focus(focus)];
        graphics.render_frame(&snapshot, &viewports);
        let (_, bottom_view) = graphics.viewport();
        assert_eq!(
            bottom_view,
            360 - 180 + VIEWPORT_SCROLL_BORDER,
            "a focus at the raw map bottom exposes C++'s 40px scroll border"
        );
    }

    #[test]
    fn viewport_uses_the_landscape_world_height_below_the_surface_depth() {
        // `GBackHgt` is the authoritative viewport bound; it is not inferred
        // from the deepest solid column (C4Viewport.cpp:1160-1209).
        let mut snapshot = make_snapshot();
        snapshot.objects[0].position = Vector2::new(320, 240);
        let mut landscape = Landscape::flat(640, 300);
        landscape.set_world_height(480);
        snapshot.landscape = Some(landscape);
        let mut graphics = test_graphics(640, 480, 300, "Tutorial");

        let viewports = vec![ViewportInput::from_focus(&snapshot.objects[0])];
        graphics.render_frame(&snapshot, &viewports);

        assert_eq!(graphics.active_viewports[0].content_rect.height, 480);
    }

    #[test]
    fn small_world_viewport_is_centered_with_tiled_scroll_borders() {
        // Fullscreen viewports are capped to GBackWdt/Hgt plus the two
        // 40-pixel scroll borders (C4GraphicsSystem.cpp:384-396). Areas
        // outside the viewport and its landscape borders tile Background.png
        // (C4GraphicsSystem.cpp:285-290; C4Viewport.cpp:1030-1041).
        let mut snapshot = make_snapshot();
        snapshot.objects[0].position = Vector2::new(320, 240);
        let surface = (0..640)
            .map(|x| if x % 2 == 0 { 479 } else { 480 })
            .collect();
        let mut landscape = Landscape::new(640, surface).expect("valid landscape surface");
        landscape.set_world_height(480);
        snapshot.landscape = Some(landscape);
        let background_pattern = [
            Color::opaque(73, 41, 19),
            Color::opaque(19, 73, 41),
            Color::opaque(41, 19, 73),
            Color::opaque(101, 83, 59),
        ];
        let background = ImageData::new(
            2,
            2,
            background_pattern
                .iter()
                .flat_map(|color| [color.r, color.g, color.b, color.a])
                .collect(),
        );
        let hud_graphics = Arc::new(HudGraphics {
            background: Some(background),
            ..HudGraphics::default()
        });
        let mut graphics = GraphicsSystem::new(
            1_000,
            800,
            300,
            "Tutorial",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            hud_graphics,
        );

        let viewports = vec![ViewportInput::from_focus(&snapshot.objects[0])];
        graphics.render_frame(&snapshot, &viewports);

        let viewport = &graphics.active_viewports[0];
        assert_eq!(
            (
                viewport.rect.x,
                viewport.rect.y,
                viewport.rect.width,
                viewport.rect.height
            ),
            (140, 120, 720, 560)
        );
        assert_eq!(
            (
                viewport.content_rect.x,
                viewport.content_rect.y,
                viewport.content_rect.width,
                viewport.content_rect.height
            ),
            (180, 160, 640, 480)
        );

        let pattern_at = |x: u32, y: u32| background_pattern[((y % 2) * 2 + x % 2) as usize];
        assert_eq!(graphics.surface().get_pixel(0, 0), Some(pattern_at(0, 0)));
        assert_eq!(
            graphics.surface().get_pixel(140, 120),
            Some(pattern_at(140, 120))
        );

        let last_content_y =
            (viewport.content_rect.y + viewport.content_rect.height as i32 - 1) as u32;
        let first_border_y = (viewport.content_rect.y + viewport.content_rect.height as i32) as u32;
        let terrain_x = (viewport.content_rect.x + 10) as u32;
        let sky_x = terrain_x + 1;
        let terrain_bottom = graphics
            .surface()
            .get_pixel(terrain_x, last_content_y)
            .expect("terrain bottom pixel");
        let sky_bottom = graphics
            .surface()
            .get_pixel(sky_x, last_content_y)
            .expect("sky bottom pixel");
        assert_ne!(terrain_bottom, sky_bottom, "bottom row must be nonuniform");

        for x in [terrain_x, sky_x] {
            let border = graphics
                .surface()
                .get_pixel(x, first_border_y)
                .expect("first border pixel below content");
            let last_content = graphics
                .surface()
                .get_pixel(x, last_content_y)
                .expect("last content pixel");
            assert_eq!(border, pattern_at(x, first_border_y));
            assert_ne!(border, last_content, "terrain edge must not be extended");
        }
    }

    #[test]
    fn object_color_reflects_energy_level() {
        let snapshot = make_snapshot();
        let mut energized = snapshot.objects[0].clone();
        energized.energy = 100;
        let high = object_color(&energized);

        let mut depleted = energized.clone();
        depleted.energy = 0;
        let low = object_color(&depleted);

        assert_ne!(high, low);
        let high_sum = u16::from(high.r) + u16::from(high.g) + u16::from(high.b);
        let low_sum = u16::from(low.r) + u16::from(low.g) + u16::from(low.b);
        assert!(high_sum > low_sum);
    }

    #[test]
    fn fill_polygon_paints_triangle() {
        let mut surface = Surface::new(32, 32, PixelFormat::Rgba8888);
        let color = Color::opaque(48, 64, 96);
        let triangle = [(4, 4), (24, 6), (10, 24)];

        let painted = fill_polygon(&mut surface, &triangle, color);
        assert!(painted);
        assert_eq!(surface.get_pixel(12, 12), Some(color));
    }

    #[test]
    fn render_frame_draws_object_vertices() {
        let mut snapshot = make_snapshot();
        snapshot.objects[0].position = Vector2::new(40, 40);
        snapshot.objects[0].vertices = vec![
            ObjectVertex::new(-6, -6),
            ObjectVertex::new(6, -6),
            ObjectVertex::new(6, 6),
            ObjectVertex::new(-6, 6),
        ];
        snapshot.landscape = Some(Landscape::flat(128, 80));

        let mut graphics = test_graphics(80, 60, 60, "Polygon Scenario");
        let focus = &snapshot.objects[0];
        let viewports = vec![ViewportInput::from_focus(focus)];
        graphics.render_frame(&snapshot, &viewports);

        let lighting = GraphicsSystem::lighting_factor(snapshot.environment.settings.time_of_day);
        let expected = GraphicsSystem::apply_lighting(object_color(&snapshot.objects[0]), lighting);
        let (viewport_x, viewport_y) = graphics.viewport();
        let screen_x = snapshot.objects[0].position.x - viewport_x;
        let screen_y = snapshot.objects[0].position.y - viewport_y;
        assert!(screen_x >= 0 && screen_x < graphics.surface().width() as i32);
        assert!(screen_y >= 0 && screen_y < graphics.surface().height() as i32);
        let pixel = graphics
            .surface()
            .get_pixel(screen_x as u32, screen_y as u32);
        assert_eq!(pixel, Some(expected));
    }

    #[test]
    fn contained_objects_are_not_drawn_in_the_world() {
        // `if (Contained && !eDrawMode) return;` (src/C4Object.cpp:2363):
        // carried items (e.g. the Mage's starting FLAG) never blit into the
        // landscape — they only appear in HUD inventory/menus.
        let mut snapshot = make_snapshot();
        snapshot.objects[0].position = Vector2::new(40, 40);
        snapshot.objects[0].vertices = vec![
            ObjectVertex::new(-6, -6),
            ObjectVertex::new(6, -6),
            ObjectVertex::new(6, 6),
            ObjectVertex::new(-6, 6),
        ];
        snapshot.objects[0].container = Some(ObjectId::new(999));
        snapshot.landscape = Some(Landscape::flat(128, 80));

        let mut graphics = test_graphics(80, 60, 60, "Contained Scenario");
        let focus = &snapshot.objects[0];
        let viewports = vec![ViewportInput::from_focus(focus)];
        graphics.render_frame(&snapshot, &viewports);

        let lighting = GraphicsSystem::lighting_factor(snapshot.environment.settings.time_of_day);
        let filled = GraphicsSystem::apply_lighting(object_color(&snapshot.objects[0]), lighting);
        let (viewport_x, viewport_y) = graphics.viewport();
        let screen_x = snapshot.objects[0].position.x - viewport_x;
        let screen_y = snapshot.objects[0].position.y - viewport_y;
        let pixel = graphics
            .surface()
            .get_pixel(screen_x as u32, screen_y as u32);
        assert_ne!(
            pixel,
            Some(filled),
            "contained object must not paint its debug polygon"
        );
    }

    fn solid_sprite(
        definition_id: &str,
        width: u32,
        height: u32,
        color: Color,
        shape: Option<DefinitionRect>,
        stretch_growth: bool,
    ) -> Arc<HashMap<String, DefinitionSprite>> {
        let pixels: Vec<u8> = (0..width * height)
            .flat_map(|_| [color.r, color.g, color.b, color.a])
            .collect();
        let mut sprites = HashMap::new();
        sprites.insert(
            sprite_map_key(definition_id, None),
            DefinitionSprite {
                graphics_scale: 1.0,
                image: ImageData::new(width, height, pixels),
                actions: HashMap::new(),
                color_mask: None,
                shape,
                fire_top: 0,
                rotateable: 0,
                line: 0,
                stretch_growth,
                top_face: None,
                picture: None,
            },
        );
        Arc::new(sprites)
    }

    fn fire_test_strip() -> ImageData {
        let colors = [
            Color::opaque(200, 0, 0),
            Color::opaque(0, 200, 0),
            Color::opaque(0, 0, 200),
        ];
        let mut pixels = Vec::with_capacity(6 * 2 * 4);
        for _ in 0..2 {
            for color in colors {
                for _ in 0..2 {
                    pixels.extend([color.r, color.g, color.b, color.a]);
                }
            }
        }
        ImageData::new(6, 2, pixels)
    }

    #[test]
    fn stock_particle_groups_render_std_smoke_and_layer_order() {
        fn solid_definition(
            name: &str,
            color: Color,
            core: ParticleDefCore,
        ) -> ParticleRenderDefinition {
            ParticleRenderDefinition {
                image: ImageData::new(1, 1, vec![color.r, color.g, color.b, color.a]),
                facet: ParticleFacet::new(0, 0, 1, 1),
                length: 1,
                aspect: 1.0,
                core: ParticleDefCore {
                    name: name.to_string(),
                    ..core
                },
                draw_proc: ParticleDrawProc::Std,
            }
        }

        fn particle(
            definition_id: &str,
            x: f32,
            y: f32,
            life: i32,
            parameter_a: f32,
            layer: ParticleLayer,
        ) -> ParticleSnapshot {
            ParticleSnapshot {
                definition_id: definition_id.to_string(),
                position: FloatVector2::new(x, y),
                velocity: FloatVector2::new(0.0, 0.0),
                life,
                parameter_a,
                parameter_b: 0x00ff_ffff,
                layer,
                pxs_fixed: None,
                pxs_slot: None,
            }
        }

        fn shipped_particle_image(path: &str) -> ImageData {
            let path = crate::test_support::repo_root().join(path);
            let rgba = image::open(&path)
                .unwrap_or_else(|error| panic!("decode {}: {error}", path.display()))
                .into_rgba8();
            let (width, height) = rgba.dimensions();
            ImageData::new(width, height, rgba.into_raw())
        }

        let owner_id = ObjectId::new(1);
        let mut owner = make_snapshot().objects.remove(0);
        owner.id = owner_id;
        owner.definition_id = "ParticleOwner".to_string();
        owner.position = Vector2::new(8, 8);
        owner.crew_member = false;

        let mut foreground = owner.clone();
        foreground.id = ObjectId::new(2);
        foreground.definition_id = "ParticleForeground".to_string();
        foreground.position = Vector2::new(22, 8);
        foreground.category |= CATEGORY_FOREGROUND_FLAG;
        let objects = vec![owner, foreground];

        let shape = Some(DefinitionRect::new(-3, -3, 6, 6));
        let mut object_sprites = HashMap::new();
        object_sprites.extend(
            (*solid_sprite(
                "ParticleOwner",
                6,
                6,
                Color::opaque(0, 0, 220),
                shape,
                false,
            ))
            .clone(),
        );
        object_sprites.extend(
            (*solid_sprite(
                "ParticleForeground",
                6,
                6,
                Color::opaque(220, 220, 0),
                shape,
                false,
            ))
            .clone(),
        );

        let mut definitions = HashMap::from([
            (
                "BackRed".to_string(),
                solid_definition(
                    "BackRed",
                    Color::opaque(220, 0, 0),
                    ParticleDefCore {
                        attach: 1,
                        ..ParticleDefCore::default()
                    },
                ),
            ),
            (
                "FrontGreen".to_string(),
                solid_definition(
                    "FrontGreen",
                    Color::opaque(0, 220, 0),
                    ParticleDefCore {
                        attach: 1,
                        ..ParticleDefCore::default()
                    },
                ),
            ),
            (
                "GlobalWhite".to_string(),
                solid_definition(
                    "GlobalWhite",
                    Color::opaque(255, 255, 255),
                    ParticleDefCore::default(),
                ),
            ),
            (
                "GlobalMagenta".to_string(),
                solid_definition(
                    "GlobalMagenta",
                    Color::opaque(220, 0, 220),
                    ParticleDefCore::default(),
                ),
            ),
            (
                "OldOrange".to_string(),
                solid_definition(
                    "OldOrange",
                    Color::opaque(220, 80, 0),
                    ParticleDefCore::default(),
                ),
            ),
            (
                "NewCyan".to_string(),
                solid_definition(
                    "NewCyan",
                    Color::opaque(0, 200, 220),
                    ParticleDefCore::default(),
                ),
            ),
            (
                "YOffBlue".to_string(),
                solid_definition(
                    "YOffBlue",
                    Color::opaque(0, 80, 220),
                    ParticleDefCore {
                        y_off: 18,
                        ..ParticleDefCore::default()
                    },
                ),
            ),
            (
                "AdditiveRed".to_string(),
                solid_definition(
                    "AdditiveRed",
                    Color::opaque(100, 0, 0),
                    ParticleDefCore {
                        additive: 1,
                        ..ParticleDefCore::default()
                    },
                ),
            ),
        ]);
        definitions.insert(
            "Phase".to_string(),
            ParticleRenderDefinition {
                image: ImageData::new(2, 1, vec![220, 0, 0, 255, 0, 220, 0, 255]),
                facet: ParticleFacet::new(0, 0, 1, 1),
                length: 2,
                aspect: 1.0,
                core: ParticleDefCore {
                    name: "Phase".to_string(),
                    delay: 1,
                    ..ParticleDefCore::default()
                },
                draw_proc: ParticleDrawProc::Std,
            },
        );
        definitions.insert(
            "Smoke".to_string(),
            ParticleRenderDefinition {
                image: shipped_particle_image(
                    "content/Objects.c4d/Effects.c4d/Smoke.c4d/Graphics.png",
                ),
                facet: ParticleFacet::new(0, 0, 64, 64),
                length: 4,
                aspect: 1.0,
                core: ParticleDefCore {
                    name: "Smoke".to_string(),
                    ..ParticleDefCore::default()
                },
                draw_proc: ParticleDrawProc::Smoke,
            },
        );
        definitions.insert(
            "Fire".to_string(),
            ParticleRenderDefinition {
                image: shipped_particle_image(
                    "content/Objects.c4d/Effects.c4d/Particles.c4d/Fire.c4d/Graphics.png",
                ),
                facet: ParticleFacet::new(0, 0, 26, 26),
                length: 1,
                aspect: 1.0,
                core: ParticleDefCore {
                    name: "Fire".to_string(),
                    attach: 1,
                    ..ParticleDefCore::default()
                },
                draw_proc: ParticleDrawProc::Std,
            },
        );

        let particles = vec![
            particle(
                "BackRed",
                0.0,
                0.0,
                0,
                2.0,
                ParticleLayer::ObjectBack(owner_id),
            ),
            particle(
                "FrontGreen",
                2.0,
                0.0,
                0,
                1.0,
                ParticleLayer::ObjectFront(owner_id),
            ),
            particle("GlobalWhite", 7.0, 8.0, 0, 1.0, ParticleLayer::Global),
            particle("GlobalMagenta", 22.0, 8.0, 0, 1.0, ParticleLayer::Global),
            particle("Phase", 14.0, 8.0, 1, 1.0, ParticleLayer::Global),
            particle("Smoke", 36.0, 8.0, 0, 4.0, ParticleLayer::Global),
            particle("Fire", 48.0, 8.0, 0, 5.0, ParticleLayer::Global),
            particle("fire", 59.0, 8.0, 0, 2.0, ParticleLayer::Global),
            particle("OldOrange", 15.0, 18.0, 0, 1.0, ParticleLayer::Global),
            particle("NewCyan", 15.0, 18.0, 0, 1.0, ParticleLayer::Global),
            particle("YOffBlue", 28.0, 18.0, 0, 2.0, ParticleLayer::Global),
            particle("AdditiveRed", 31.0, 18.0, 0, 1.0, ParticleLayer::Global),
        ];

        let mut graphics = GraphicsSystem::new(
            64,
            24,
            24,
            "particle draw procedures",
            test_font(),
            Arc::new(object_sprites),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics.set_particle_sprites(Arc::new(definitions));
        graphics.set_renderer_config(true, true);
        graphics.surface_mut().fill(Color::opaque(0, 0, 0));
        graphics
            .surface_mut()
            .set_pixel(31, 18, Color::opaque(50, 20, 30))
            .expect("seed additive destination");

        graphics.draw_objects_at_frame(
            0,
            &objects,
            &[],
            &HashMap::new(),
            &particles,
            &[],
            OWNER_NONE,
            1.0,
            &HashMap::new(),
            ObjectRenderPass::Normal,
            None,
        );
        assert_eq!(
            graphics.surface().get_pixel(7, 8),
            Some(Color::opaque(0, 0, 220))
        );
        assert_eq!(
            graphics.surface().get_pixel(9, 8),
            Some(Color::opaque(0, 220, 0))
        );

        graphics.draw_definition_particles(&particles, &ParticleLayer::Global, None, None);
        assert_eq!(
            graphics.surface().get_pixel(7, 8),
            Some(Color::opaque(255, 255, 255))
        );
        assert_eq!(
            graphics.surface().get_pixel(22, 8),
            Some(Color::opaque(220, 0, 220))
        );
        assert_eq!(
            graphics.surface().get_pixel(14, 8),
            Some(Color::opaque(0, 220, 0))
        );
        assert_eq!(
            graphics.surface().get_pixel(15, 18),
            Some(Color::opaque(220, 80, 0)),
            "definition particles draw newest-first"
        );
        assert_eq!(
            graphics.surface().get_pixel(28, 17),
            Some(Color::opaque(0, 0, 0))
        );
        assert_eq!(
            graphics.surface().get_pixel(28, 18),
            Some(Color::opaque(0, 80, 220))
        );
        assert_eq!(
            graphics.surface().get_pixel(31, 18),
            Some(Color::opaque(150, 20, 30))
        );
        assert_eq!(
            graphics.surface().get_pixel(59, 8),
            Some(Color::opaque(0, 0, 0)),
            "lookup remains exact-case"
        );
        assert!((32..40).any(|x| (4..12)
            .any(|y| { graphics.surface().get_pixel(x, y) != Some(Color::opaque(0, 0, 0)) })));
        assert!((43..53).any(|x| (3..13)
            .any(|y| { graphics.surface().get_pixel(x, y) != Some(Color::opaque(0, 0, 0)) })));

        graphics.draw_objects_at_frame(
            0,
            &objects,
            &[],
            &HashMap::new(),
            &particles,
            &[],
            OWNER_NONE,
            1.0,
            &HashMap::new(),
            ObjectRenderPass::ForegroundNonParallax,
            None,
        );
        assert_eq!(
            graphics.surface().get_pixel(22, 8),
            Some(Color::opaque(220, 220, 0))
        );
    }

    #[test]
    fn unfogged_definition_particles_capture_in_compatible_batches() {
        let definition = |name: &str, additive| ParticleRenderDefinition {
            image: ImageData::new(1, 1, vec![255; 4]),
            facet: ParticleFacet::new(0, 0, 1, 1),
            length: 1,
            aspect: 1.0,
            core: ParticleDefCore {
                name: name.to_owned(),
                additive,
                ..ParticleDefCore::default()
            },
            draw_proc: ParticleDrawProc::Std,
        };
        let particle = |definition_id: &str, x| ParticleSnapshot {
            definition_id: definition_id.to_owned(),
            position: FloatVector2::new(x, 4.0),
            velocity: FloatVector2::new(0.0, 0.0),
            life: 0,
            parameter_a: 1.0,
            parameter_b: 0x00ff_ffff,
            layer: ParticleLayer::Global,
            pxs_fixed: None,
            pxs_slot: None,
        };
        let mut graphics = test_graphics(8, 8, 8, "particle batches");
        graphics.set_particle_sprites(Arc::new(HashMap::from([
            ("Fire".to_owned(), definition("Fire", 0)),
            ("Fire2".to_owned(), definition("Fire2", 1)),
        ])));
        let particles = vec![
            particle("Fire", 2.0),
            particle("Fire", 3.0),
            particle("Fire2", 4.0),
            particle("Fire2", 5.0),
        ];

        graphics.begin_gpu_scene_capture();
        graphics.draw_definition_particles(&particles, &ParticleLayer::Global, None, None);
        let scene = graphics
            .finish_gpu_scene_capture(&clonk_graphics::GammaRamp::identity())
            .expect("particle scene capture");

        let [fire2, fire] = scene.commands.as_slice() else {
            panic!("expected one adjacent batch per definition");
        };
        let batch = |command: &GpuCommand, expected_blend, expected_rects| {
            let GpuCommand::SpriteBatch {
                quads,
                blend,
                mod2,
                gamma,
                outer_modulation,
                ..
            } = command
            else {
                panic!("expected a compact sprite batch");
            };
            assert_eq!(*blend, expected_blend);
            assert!(!mod2);
            assert!(!gamma);
            assert_eq!(*outer_modulation, GpuOuterModulation::Combine);
            assert_eq!(
                quads.iter().map(|quad| quad.rect).collect::<Vec<_>>(),
                expected_rects,
                "instances retain native newest-first painter order",
            );
            assert!(quads
                .iter()
                .all(|quad| { quad.uv == [0.0, 0.0, 1.0, 1.0] && quad.modulation == 0x00ff_ffff }));
        };
        batch(
            fire2,
            GpuBlend::Additive,
            vec![[4.0, 3.0, 6.0, 5.0], [3.0, 3.0, 5.0, 5.0]],
        );
        batch(
            fire,
            GpuBlend::Normal,
            vec![[2.0, 3.0, 4.0, 5.0], [1.0, 3.0, 3.0, 5.0]],
        );
    }

    #[test]
    fn shipped_fire2_particles_draw_with_the_additive_blit() {
        // The engine's burning-object emitter makes three quarters of every
        // double set the `Fire2` def (src/C4Effect.cpp:732-742), and that def
        // is additive by content. `C4GFXBLIT_ADDITIVE` maps to
        // `glBlendFunc(GL_SRC_ALPHA, GL_ONE)` (src/StdGL.cpp:908), so the
        // flame adds to what is behind it instead of replacing it — which is
        // what makes engine fire read as a glow rather than a boxy sprite.
        // The sibling `Fire` def is opaque and must stay a normal blit.
        let load_shipped = |name: &str| {
            let path = crate::test_support::repo_root()
                .join("content/Objects.c4d/Effects.c4d/Particles.c4d")
                .join(format!("{name}.c4d"));
            let group = clonk_resources::Group::open(&path)
                .unwrap_or_else(|error| panic!("open {}: {error}", path.display()));
            clonk_resources::ParticleDefinition::load(&group)
                .unwrap_or_else(|error| panic!("load {}: {error}", path.display()))
        };

        let mut definitions = HashMap::new();
        for name in ["Fire", "Fire2"] {
            let shipped = load_shipped(name);
            let core = ParticleDefCore::from(&shipped.core);
            assert_eq!(
                core.name, name,
                "SetDefParticles resolves pFire1/pFire2 by this name \
                 (src/C4Particles.cpp:485-486)",
            );
            // One flat opaque phase, so the blend is the only variable.
            definitions.insert(
                name.to_string(),
                ParticleRenderDefinition {
                    image: ImageData::new(1, 1, vec![100, 0, 0, 255]),
                    facet: ParticleFacet::new(0, 0, 1, 1),
                    length: 1,
                    aspect: 1.0,
                    core,
                    draw_proc: ParticleDrawProc::Std,
                },
            );
        }
        assert_eq!(
            definitions["Fire2"].core.additive, 1,
            "the shipped Fire2 Particle.txt carries Additive=1",
        );
        assert_eq!(
            definitions["Fire"].core.additive, 0,
            "the shipped Fire underlay is an ordinary alpha blit",
        );

        // Engine fire is dealt to the burning object's own lists with
        // Attach=1, so the stored position is relative to that object
        // (src/C4Particles.cpp:404-408).
        let owner_id = ObjectId::new(1);
        let mut owner = make_snapshot().objects.remove(0);
        owner.id = owner_id;
        owner.definition_id = "FlameOwner".to_string();
        owner.position = Vector2::new(8, 8);
        owner.crew_member = false;
        let objects = vec![owner];
        let object_sprites: HashMap<_, _> = (*solid_sprite(
            "FlameOwner",
            6,
            6,
            Color::opaque(0, 0, 220),
            Some(DefinitionRect::new(-3, -3, 6, 6)),
            false,
        ))
        .clone();

        let particle = |name: &str, x: f32| ParticleSnapshot {
            definition_id: name.to_string(),
            position: FloatVector2::new(x - 8.0, 10.0),
            velocity: FloatVector2::new(0.0, 0.0),
            life: 0,
            parameter_a: 1.0,
            parameter_b: 0x00ff_ffff,
            layer: ParticleLayer::ObjectBack(owner_id),
            pxs_fixed: None,
            pxs_slot: None,
        };
        let particles = vec![particle("Fire2", 31.0), particle("Fire", 41.0)];

        let mut graphics = GraphicsSystem::new(
            64,
            24,
            24,
            "shipped fire particle blending",
            test_font(),
            Arc::new(object_sprites),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics.set_particle_sprites(Arc::new(definitions));
        graphics.set_renderer_config(true, true);
        graphics.surface_mut().fill(Color::opaque(0, 0, 0));
        for x in [31, 41] {
            graphics
                .surface_mut()
                .set_pixel(x, 18, Color::opaque(50, 20, 30))
                .expect("seed the destination behind each flame");
        }

        graphics.draw_objects_at_frame(
            0,
            &objects,
            &[],
            &HashMap::new(),
            &particles,
            &[],
            OWNER_NONE,
            1.0,
            &HashMap::new(),
            ObjectRenderPass::Normal,
            None,
        );

        assert_eq!(
            graphics.surface().get_pixel(31, 18),
            Some(Color::opaque(150, 20, 30)),
            "Fire2 adds src*srcAlpha to the destination",
        );
        assert_eq!(
            graphics.surface().get_pixel(41, 18),
            Some(Color::opaque(100, 0, 0)),
            "Fire replaces the destination through the ordinary alpha blit",
        );
    }

    #[test]
    fn std_particles_apply_aspect_parallax_velocity_rotation_and_packed_modulation() {
        fn definition(
            name: &str,
            color: Color,
            aspect: f32,
            core: ParticleDefCore,
        ) -> ParticleRenderDefinition {
            ParticleRenderDefinition {
                image: ImageData::new(1, 1, vec![color.r, color.g, color.b, color.a]),
                facet: ParticleFacet::new(0, 0, 1, 1),
                length: 1,
                aspect,
                core: ParticleDefCore {
                    name: name.to_string(),
                    ..core
                },
                draw_proc: ParticleDrawProc::Std,
            }
        }
        fn particle(name: &str, x: f32, y: f32, a: f32) -> ParticleSnapshot {
            ParticleSnapshot {
                definition_id: name.to_string(),
                position: FloatVector2::new(x, y),
                velocity: FloatVector2::new(0.0, 0.0),
                life: 0,
                parameter_a: a,
                parameter_b: 0x00ff_ffff,
                layer: ParticleLayer::Global,
                pxs_fixed: None,
                pxs_slot: None,
            }
        }

        let definitions = HashMap::from([
            (
                "Packed".to_string(),
                definition(
                    "Packed",
                    Color::opaque(255, 255, 255),
                    1.0,
                    ParticleDefCore::default(),
                ),
            ),
            (
                "Tall".to_string(),
                definition(
                    "Tall",
                    Color::opaque(0, 180, 0),
                    2.0,
                    ParticleDefCore::default(),
                ),
            ),
            (
                "Rotated".to_string(),
                definition(
                    "Rotated",
                    Color::opaque(200, 0, 0),
                    2.0,
                    ParticleDefCore {
                        r_by_v: 1,
                        ..ParticleDefCore::default()
                    },
                ),
            ),
            (
                "Parallax".to_string(),
                definition(
                    "Parallax",
                    Color::opaque(220, 220, 0),
                    1.0,
                    ParticleDefCore {
                        parallaxity: [50, 50],
                        ..ParticleDefCore::default()
                    },
                ),
            ),
        ]);
        let mut packed = particle("Packed", 14.0, 10.0, 1.0);
        packed.parameter_b = 0x0011_2233;
        let tall = particle("Tall", 22.0, 14.0, 2.0);
        let mut rotated = particle("Rotated", 30.0, 14.0, 2.0);
        rotated.velocity = FloatVector2::new(1.0, 0.0);
        let parallax = particle("Parallax", 35.0, 7.0, 1.0);

        let mut graphics = test_graphics(36, 16, 16, "Std particle branches");
        graphics.viewport_x = 10.0;
        graphics.viewport_y = 6.0;
        graphics.set_particle_sprites(Arc::new(definitions));
        graphics.surface_mut().fill(Color::opaque(0, 0, 0));
        graphics.draw_definition_particles(
            &[packed, tall, rotated, parallax],
            &ParticleLayer::Global,
            None,
            None,
        );

        assert_eq!(
            graphics.surface().get_pixel(4, 4),
            Some(Color::opaque(17, 34, 51))
        );
        assert_eq!(
            graphics.surface().get_pixel(12, 4),
            Some(Color::opaque(0, 180, 0))
        );
        assert_eq!(
            graphics.surface().get_pixel(9, 8),
            Some(Color::opaque(0, 0, 0))
        );
        assert_eq!(
            graphics.surface().get_pixel(17, 8),
            Some(Color::opaque(200, 0, 0)),
            "RByV rotates the tall destination onto the horizontal axis"
        );
        assert_eq!(
            graphics.surface().get_pixel(20, 5),
            Some(Color::opaque(0, 0, 0))
        );
        assert_eq!(
            graphics.surface().get_pixel(30, 4),
            Some(Color::opaque(220, 220, 0))
        );
        assert_eq!(
            graphics.surface().get_pixel(25, 1),
            Some(Color::opaque(0, 0, 0))
        );
    }

    #[test]
    fn the_fire_detail_rung_skips_only_a_burning_objects_own_flames() {
        // The `NoFireParticles` presentation-detail rung exists to reclaim
        // the unbatched per-particle draw calls the stock fire defs cost, so
        // it has to actually skip them. Everything else keeps drawing: the
        // rung is about fire, not about particles in general.
        fn solid(name: &str) -> ParticleRenderDefinition {
            ParticleRenderDefinition {
                image: ImageData::new(1, 1, vec![200, 0, 0, 255]),
                facet: ParticleFacet::new(0, 0, 1, 1),
                length: 1,
                aspect: 1.0,
                core: ParticleDefCore {
                    name: name.to_string(),
                    ..ParticleDefCore::default()
                },
                draw_proc: ParticleDrawProc::Std,
            }
        }
        let particle = |name: &str, x: f32, owner: ObjectId| ParticleSnapshot {
            definition_id: name.to_string(),
            position: FloatVector2::new(x, 8.0),
            velocity: FloatVector2::new(0.0, 0.0),
            life: 0,
            parameter_a: 1.0,
            parameter_b: 0x00ff_ffff,
            layer: ParticleLayer::ObjectBack(owner),
            pxs_fixed: None,
            pxs_slot: None,
        };
        let definitions: HashMap<_, _> = ["Fire", "Fire2", "Smoke2"]
            .iter()
            .map(|name| (name.to_string(), solid(name)))
            .collect();
        let burning = ObjectId::new(1);
        let cold = ObjectId::new(2);
        let particles = vec![
            particle("Fire", 4.0, burning),
            particle("Fire2", 12.0, burning),
            particle("Smoke2", 20.0, burning),
            // Script fire: the EkeReloaded flamethrower projectile and the
            // Baldoon torches build their flame from CreateParticle on an
            // object that is not alight. Suppressing those would erase a
            // damaging projectile, not trim frame cost.
            particle("Fire2", 26.0, cold),
        ];

        // The particle pass runs inside the object draw, so the viewport
        // needs at least one object; it sits clear of the sampled pixels.
        let mut owner = make_snapshot().objects.remove(0);
        owner.id = burning;
        owner.definition_id = "ParticleOwner".to_string();
        owner.position = Vector2::new(30, 14);
        owner.crew_member = false;
        owner.on_fire = true;
        let mut unlit = owner.clone();
        unlit.id = cold;
        unlit.position = Vector2::new(30, 2);
        unlit.on_fire = false;
        let objects = vec![owner, unlit];
        let object_sprites: HashMap<_, _> = (*solid_sprite(
            "ParticleOwner",
            2,
            2,
            Color::opaque(0, 0, 220),
            Some(DefinitionRect::new(-1, -1, 2, 2)),
            false,
        ))
        .clone();

        let render = |fire_particles: bool| {
            let mut graphics = GraphicsSystem::new(
                32,
                16,
                16,
                "fire particle suppression",
                test_font(),
                Arc::new(object_sprites.clone()),
                empty_cursor_atlas(),
                empty_hud_graphics(),
            );
            graphics.set_particle_sprites(Arc::new(definitions.clone()));
            graphics.set_renderer_config(true, true);
            graphics.set_fire_particle_detail(fire_particles);
            graphics.surface_mut().fill(Color::opaque(0, 0, 0));
            graphics.draw_objects_at_frame(
                0,
                &objects,
                &[],
                &HashMap::new(),
                &particles,
                &[],
                OWNER_NONE,
                1.0,
                &HashMap::new(),
                ObjectRenderPass::Normal,
                None,
            );
            [4, 12, 20, 26].map(|x| graphics.surface().get_pixel(x, 8))
        };

        let drawn = Color::opaque(200, 0, 0);
        let blank = Color::opaque(0, 0, 0);
        assert_eq!(
            render(true),
            [Some(drawn), Some(drawn), Some(drawn), Some(drawn)],
            "at full detail every particle draws",
        );
        assert_eq!(
            render(false),
            [Some(blank), Some(blank), Some(drawn), Some(drawn)],
            "the rung skips the burning object's Fire and Fire2, and leaves \
             both other defs and script fire on an unlit object alone",
        );
    }

    #[test]
    fn disabling_extended_fire_particles_keeps_simple_fire_facet() {
        // The detail rung suppresses flame *particles* only. The simple
        // object Fire.png facet is unconditional in C++ (C4Object.cpp:2387,
        // "always draw, even if particles are drawn as well") and stays so
        // here, which is what keeps a burning object legible at the lowest
        // detail level.
        let mut object = make_snapshot().objects.remove(0);
        object.position = Vector2::new(6, 6);
        object.crew_member = false;
        object.on_fire = true;
        object.fire_phase = 0;
        let sprite = DefinitionSprite {
            graphics_scale: 1.0,
            image: ImageData::new(2, 2, vec![0; 2 * 2 * 4]),
            actions: HashMap::new(),
            color_mask: None,
            shape: Some(DefinitionRect::new(-1, -1, 2, 2)),
            fire_top: 0,
            rotateable: 0,
            line: 0,
            stretch_growth: false,
            top_face: None,
            picture: None,
        };
        let hud = Arc::new(HudGraphics {
            fire: Some(fire_test_strip()),
            ..HudGraphics::default()
        });
        let mut graphics = GraphicsSystem::new(
            12,
            12,
            12,
            "simple fire fallback",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            hud,
        );
        graphics.set_renderer_config(true, true);
        graphics.set_fire_particle_detail(false);
        assert!(!graphics.draws_fire_particles());
        graphics.surface_mut().fill(Color::opaque(0, 0, 0));

        graphics.draw_object_fire(
            &object,
            &sprite,
            (0.0, 0.0),
            SpriteBlitState::normal(),
            None,
        );

        assert_eq!(
            graphics.surface().get_pixel(5, 5),
            Some(Color::opaque(200, 0, 0))
        );
    }

    #[test]
    fn burning_object_uses_phase_plain_stretch_scaled_fire_top_and_cpp_order() {
        // C4Object::Draw stretches one height-square FirePhase cell over the
        // live Shape and draws it before the object face
        // (src/C4Object.cpp:2388-2418). At half construction, Jolt changes
        // Shape(-5,-4,10,8) to (-5,-2,10,4) and FireTop 4 to 2, producing
        // the exact world rect x=15..24, y=18..19.
        let mut object = make_snapshot().objects.remove(0);
        object.definition_id = "BurningStraight".to_string();
        object.position = Vector2::new(20, 20);
        object.construction = FULL_CON / 2;
        object.current_shape = None;
        object.crew_member = false;
        object.on_fire = true;
        object.fire_phase = 1;

        let mut base_pixels = vec![0; 10 * 8 * 4];
        let base_index = (4 * 10 + 5) * 4;
        base_pixels[base_index..base_index + 4].copy_from_slice(&[0, 0, 200, 255]);
        let sprite = DefinitionSprite {
            graphics_scale: 1.0,
            image: ImageData::new(10, 8, base_pixels),
            actions: HashMap::new(),
            color_mask: None,
            shape: Some(DefinitionRect::new(-5, -4, 10, 8)),
            fire_top: 4,
            rotateable: 0,
            line: 0,
            stretch_growth: false,
            top_face: None,
            picture: None,
        };
        let hud = Arc::new(HudGraphics {
            fire: Some(fire_test_strip()),
            ..HudGraphics::default()
        });
        let mut graphics = GraphicsSystem::new(
            40,
            32,
            32,
            "straight fire facet",
            test_font(),
            Arc::new(HashMap::from([(
                sprite_map_key("BurningStraight", None),
                sprite,
            )])),
            empty_cursor_atlas(),
            hud,
        );
        let black = Color::opaque(0, 0, 0);
        let green = Color::opaque(0, 200, 0);
        let blue = Color::opaque(0, 0, 200);
        graphics.surface_mut().fill(black);
        graphics.draw_objects(
            &[object.clone()],
            &[],
            &HashMap::new(),
            &[],
            OWNER_NONE,
            1.0,
            &HashMap::new(),
            ObjectRenderPass::Normal,
            None,
        );

        assert_eq!(graphics.surface().get_pixel(15, 18), Some(green));
        assert_eq!(graphics.surface().get_pixel(24, 19), Some(green));
        assert_eq!(graphics.surface().get_pixel(14, 18), Some(black));
        assert_eq!(graphics.surface().get_pixel(25, 18), Some(black));
        assert_eq!(graphics.surface().get_pixel(15, 17), Some(black));
        assert_eq!(graphics.surface().get_pixel(15, 20), Some(black));
        assert_eq!(
            graphics.surface().get_pixel(20, 18),
            Some(blue),
            "the base face is drawn after and can cover the fire facet"
        );

        object.current_fire_top = Some(1);
        graphics.surface_mut().fill(black);
        graphics.draw_objects(
            &[object.clone()],
            &[],
            &HashMap::new(),
            &[],
            OWNER_NONE,
            1.0,
            &HashMap::new(),
            ObjectRenderPass::Normal,
            None,
        );
        assert_eq!(graphics.surface().get_pixel(15, 20), Some(green));
        assert_eq!(graphics.surface().get_pixel(15, 21), Some(black));

        // TargetPos applies object-local parallax before fire. The existing
        // base-face path intentionally remains transparent at the sampled
        // edge, so this pins the fire coordinates independently.
        let int_value = |value| {
            serde_json::from_value(serde_json::json!({ "Int": value }))
                .expect("deserialize C4Script integer")
        };
        object.current_fire_top = None;
        object.category |= CATEGORY_PARALLAX_FLAG;
        object
            .local_vars
            .insert("__local_0".to_string(), int_value(50));
        object
            .local_vars
            .insert("__local_1".to_string(), int_value(50));
        graphics.viewport_x = 10.0;
        graphics.viewport_y = 10.0;
        graphics.surface_mut().fill(black);
        graphics.draw_objects(
            &[object.clone()],
            &[],
            &HashMap::new(),
            &[],
            OWNER_NONE,
            1.0,
            &HashMap::new(),
            ObjectRenderPass::Normal,
            None,
        );
        assert_eq!(graphics.surface().get_pixel(10, 13), Some(green));
        assert_eq!(graphics.surface().get_pixel(5, 8), Some(black));
        graphics.viewport_x = 0.0;
        graphics.viewport_y = 0.0;
        object.category &= !CATEGORY_PARALLAX_FLAG;

        object.on_fire = false;
        graphics.surface_mut().fill(black);
        graphics.draw_objects(
            &[object.clone()],
            &[],
            &HashMap::new(),
            &[],
            OWNER_NONE,
            1.0,
            &HashMap::new(),
            ObjectRenderPass::Normal,
            None,
        );
        assert_eq!(
            graphics.surface().get_pixel(15, 18),
            Some(black),
            "the phase-one pixels come only from the burning overlay"
        );

        // C4Object::UpdateShape also scales burning Oversize definitions
        // beyond 100%; fire must not clamp Con to FullCon.
        object.on_fire = true;
        object.construction = FULL_CON + FULL_CON / 2;
        object.current_shape = Some(DefinitionRect::new(-5, -6, 10, 12));
        graphics.surface_mut().fill(black);
        graphics.draw_objects(
            &[object],
            &[],
            &HashMap::new(),
            &[],
            OWNER_NONE,
            1.0,
            &HashMap::new(),
            ObjectRenderPass::Normal,
            None,
        );
        assert_eq!(graphics.surface().get_pixel(15, 14), Some(green));
        assert_eq!(graphics.surface().get_pixel(15, 19), Some(green));
        assert_eq!(graphics.surface().get_pixel(15, 20), Some(black));
    }

    #[test]
    fn rotated_burning_object_uses_origin_inclusive_vertex_outline() {
        // C4Shape::Rotate turns Shape(-6,-4,12,8) into y=-9. The unusual
        // GetVertexOutline starts at the origin, so all-positive vertices
        // [(2,-1)..(5,3)] produce x=0,w=5,y=-9,h=12. FireTop is ignored for
        // rotated fire (src/C4Shape.cpp:41-92,130-163;
        // src/C4Object.cpp:2397-2405).
        let mut object = make_snapshot().objects.remove(0);
        object.definition_id = "BurningRotated".to_string();
        object.position = Vector2::new(16, 16);
        object.rotation = 90;
        object.current_shape = None;
        object.crew_member = false;
        object.on_fire = true;
        object.fire_phase = 2;
        object.vertices = vec![
            ObjectVertex::new(2, -1),
            ObjectVertex::new(5, -1),
            ObjectVertex::new(5, 3),
            ObjectVertex::new(2, 3),
        ];

        let sprite = DefinitionSprite {
            graphics_scale: 1.0,
            image: ImageData::new(12, 8, vec![0; 12 * 8 * 4]),
            actions: HashMap::new(),
            color_mask: None,
            shape: Some(DefinitionRect::new(-6, -4, 12, 8)),
            fire_top: 7,
            rotateable: 1,
            line: 0,
            stretch_growth: false,
            top_face: None,
            picture: None,
        };
        let hud = Arc::new(HudGraphics {
            fire: Some(fire_test_strip()),
            ..HudGraphics::default()
        });
        let mut graphics = GraphicsSystem::new(
            40,
            32,
            32,
            "rotated fire facet",
            test_font(),
            Arc::new(HashMap::from([(
                sprite_map_key("BurningRotated", None),
                sprite,
            )])),
            empty_cursor_atlas(),
            hud,
        );
        let black = Color::opaque(0, 0, 0);
        let blue = Color::opaque(0, 0, 200);
        graphics.surface_mut().fill(black);
        graphics.draw_objects(
            &[object.clone()],
            &[],
            &HashMap::new(),
            &[],
            OWNER_NONE,
            1.0,
            &HashMap::new(),
            ObjectRenderPass::Normal,
            None,
        );

        assert_eq!(graphics.surface().get_pixel(16, 7), Some(blue));
        assert_eq!(graphics.surface().get_pixel(20, 18), Some(blue));
        assert_eq!(graphics.surface().get_pixel(15, 7), Some(black));
        assert_eq!(graphics.surface().get_pixel(21, 7), Some(black));
        assert_eq!(graphics.surface().get_pixel(16, 6), Some(black));
        assert_eq!(graphics.surface().get_pixel(16, 19), Some(black));

        // A non-rotateable object may retain raw r != 0. C++ still selects
        // the vertex-outline fire branch, but its live Shape is not enlarged.
        // Using current_shape also covers a script SetShape override.
        object.current_shape = Some(DefinitionRect::new(-6, -4, 12, 8));
        graphics.surface_mut().fill(black);
        graphics.draw_objects(
            &[object],
            &[],
            &HashMap::new(),
            &[],
            OWNER_NONE,
            1.0,
            &HashMap::new(),
            ObjectRenderPass::Normal,
            None,
        );
        assert_eq!(graphics.surface().get_pixel(16, 12), Some(blue));
        assert_eq!(graphics.surface().get_pixel(20, 18), Some(blue));
        assert_eq!(graphics.surface().get_pixel(16, 11), Some(black));
        assert_eq!(graphics.surface().get_pixel(16, 19), Some(black));
    }

    #[test]
    fn burning_object_culls_on_live_shape_before_vertex_outline_fire() {
        // C4Object::Draw rejects a normal-mode object against its live Shape
        // before constructing the rotated fire rectangle. Even if the vertex
        // outline would reach back onto the output, no flame is drawn
        // (src/C4Object.cpp:2266-2283,2388-2408).
        let mut object = make_snapshot().objects.remove(0);
        object.definition_id = "CulledBurning".to_string();
        object.position = Vector2::new(5, 5);
        object.rotation = 1;
        object.current_shape = Some(DefinitionRect::new(20, 0, 1, 1));
        object.vertices = vec![
            ObjectVertex::new(-5, 0),
            ObjectVertex::new(0, 0),
            ObjectVertex::new(0, 2),
        ];
        object.crew_member = false;
        object.on_fire = true;
        object.fire_phase = 0;

        let sprite = DefinitionSprite {
            graphics_scale: 1.0,
            image: ImageData::new(1, 1, vec![0, 0, 0, 0]),
            actions: HashMap::new(),
            color_mask: None,
            shape: Some(DefinitionRect::new(0, 0, 1, 1)),
            fire_top: 0,
            rotateable: 1,
            line: 0,
            stretch_growth: false,
            top_face: None,
            picture: None,
        };
        let mut graphics = GraphicsSystem::new(
            10,
            10,
            10,
            "culled fire facet",
            test_font(),
            Arc::new(HashMap::from([(
                sprite_map_key("CulledBurning", None),
                sprite,
            )])),
            empty_cursor_atlas(),
            Arc::new(HudGraphics {
                fire: Some(fire_test_strip()),
                ..HudGraphics::default()
            }),
        );
        let black = Color::opaque(0, 0, 0);
        graphics.surface_mut().fill(black);
        graphics.draw_objects(
            &[object],
            &[],
            &HashMap::new(),
            &[],
            OWNER_NONE,
            1.0,
            &HashMap::new(),
            ObjectRenderPass::Normal,
            None,
        );

        assert_eq!(graphics.surface().get_pixel(1, 5), Some(black));
        assert!(graphics
            .surface()
            .pixels()
            .chunks_exact(4)
            .all(|pixel| pixel == [0, 0, 0, 255]));
    }

    #[test]
    fn zero_sized_definition_shape_does_not_gain_image_sized_fire() {
        // An explicit zero DefCore Shape is still authoritative. Only a
        // loader sprite with no shape metadata falls back to image bounds.
        let mut object = make_snapshot().objects.remove(0);
        object.definition_id = "ZeroShapeBurning".to_string();
        object.position = Vector2::new(5, 5);
        object.current_shape = None;
        object.crew_member = false;
        object.on_fire = true;

        let sprite = DefinitionSprite {
            graphics_scale: 1.0,
            image: ImageData::new(4, 4, vec![0; 4 * 4 * 4]),
            actions: HashMap::new(),
            color_mask: None,
            shape: Some(DefinitionRect::new(0, 0, 0, 0)),
            fire_top: 0,
            rotateable: 0,
            line: 0,
            stretch_growth: false,
            top_face: None,
            picture: None,
        };
        let mut graphics = GraphicsSystem::new(
            10,
            10,
            10,
            "zero shape fire facet",
            test_font(),
            Arc::new(HashMap::from([(
                sprite_map_key("ZeroShapeBurning", None),
                sprite,
            )])),
            empty_cursor_atlas(),
            Arc::new(HudGraphics {
                fire: Some(fire_test_strip()),
                ..HudGraphics::default()
            }),
        );
        let black = Color::opaque(0, 0, 0);
        graphics.surface_mut().fill(black);
        graphics.draw_objects(
            &[object],
            &[],
            &HashMap::new(),
            &[],
            OWNER_NONE,
            1.0,
            &HashMap::new(),
            ObjectRenderPass::Normal,
            None,
        );

        assert!(graphics
            .surface()
            .pixels()
            .chunks_exact(4)
            .all(|pixel| pixel == [0, 0, 0, 255]));
    }

    #[test]
    fn construction_sign_uses_scaled_shape_bottom_left_in_the_top_face_pass() {
        // DrawTopFace places fctConstruction at the current Shape's bottom
        // left, after every object's base pass (src/C4Object.cpp:2617-2638;
        // src/C4ObjectList.cpp:387-395). A later base at that pixel must be
        // covered by the sign, and the unscaled Def shape must not position it.
        let mut snapshot = make_snapshot();
        let site = &mut snapshot.objects[0];
        site.definition_id = "ConstructionSite".to_string();
        site.position = Vector2::new(40, 40);
        site.crew_member = false;
        site.construction = FULL_CON / 2;
        site.ocf = clonk_engine::ocf::CONSTRUCT;

        // At Con=50%, Shape(-4,-8,8,16) jolts to (-4,-4,8,8), so a
        // 2x2 sign begins at world (36,42). This base is deliberately drawn
        // there after the construction site's base.
        let mut covering_base = site.clone();
        covering_base.id = ObjectId::new(2);
        covering_base.definition_id = "CoveringBase".to_string();
        covering_base.position = Vector2::new(36, 42);
        covering_base.construction = FULL_CON;
        covering_base.ocf = 0;
        snapshot.objects.push(covering_base);
        snapshot.render_order = vec![ObjectId::new(1), ObjectId::new(2)];

        let red = Color::opaque(200, 0, 0);
        let blue = Color::opaque(0, 0, 200);
        let green = Color::opaque(0, 200, 0);
        let mut sprites = solid_sprite(
            "ConstructionSite",
            8,
            16,
            red,
            Some(DefinitionRect::new(-4, -8, 8, 16)),
            false,
        )
        .as_ref()
        .clone();
        sprites.extend(
            solid_sprite(
                "CoveringBase",
                1,
                1,
                blue,
                Some(DefinitionRect::new(0, 0, 1, 1)),
                false,
            )
            .as_ref()
            .clone(),
        );
        let hud = Arc::new(HudGraphics {
            construction: Some(ImageData::new(2, 2, [0, 200, 0, 255].repeat(4))),
            ..HudGraphics::default()
        });
        let mut graphics = GraphicsSystem::new(
            80,
            60,
            60,
            "Construction sign",
            test_font(),
            Arc::new(sprites),
            empty_cursor_atlas(),
            hud,
        );
        graphics.surface_mut().fill(Color::opaque(0, 0, 0));
        graphics.draw_objects(
            &snapshot.objects,
            &snapshot.render_order,
            &snapshot.definition_lines,
            &snapshot.players,
            OWNER_NONE,
            1.0,
            &HashMap::new(),
            ObjectRenderPass::Normal,
            None,
        );

        assert_eq!(
            graphics.surface().get_pixel(36, 42),
            Some(green),
            "the top-face sign must cover a base drawn later in the base pass"
        );
        assert_ne!(
            graphics.surface().get_pixel(36, 46),
            Some(green),
            "the sign must use the Con-scaled Shape, not the unscaled Def shape"
        );
    }

    #[test]
    fn definition_top_faces_draw_after_every_object_base_like_cpp() {
        // C4ObjectList::Draw performs one complete base pass and only then a
        // complete TopFace pass (src/C4ObjectList.cpp:390-396). Thus A's
        // TopFace must cover the later overlapping base of B.
        let mut snapshot = make_snapshot();
        snapshot.objects[0].container = Some(ObjectId::new(99));
        let mut top_object = snapshot.objects[0].clone();
        top_object.id = ObjectId::new(2);
        top_object.definition_id = "TopObject".to_string();
        top_object.position = Vector2::new(105, 100);
        top_object.container = None;
        top_object.crew_member = false;
        top_object.action = clonk_engine::ActionState::new("Active");
        let mut base_object = top_object.clone();
        base_object.id = ObjectId::new(3);
        base_object.definition_id = "BaseObject".to_string();
        base_object.action = Default::default();
        snapshot.objects.extend([top_object, base_object]);
        snapshot.landscape = Some(Landscape::flat(160, 140));

        let green = Color::opaque(0, 200, 0);
        let blue = Color::opaque(0, 0, 200);
        let mut sprites = HashMap::new();
        sprites.insert(
            sprite_map_key("TopObject", None),
            DefinitionSprite {
                graphics_scale: 1.0,
                image: ImageData::new(2, 1, vec![0, 0, 0, 0, 0, 200, 0, 255]),
                actions: HashMap::from([(
                    "Active".to_string(),
                    DefinitionActionGraphics::default(),
                )]),
                color_mask: None,
                shape: Some(DefinitionRect::new(0, 0, 1, 1)),
                fire_top: 0,
                rotateable: 0,
                line: 0,
                stretch_growth: false,
                top_face: Some(DefinitionTargetRect::new(1, 0, 1, 1, 0, 0)),
                picture: None,
            },
        );
        sprites.insert(
            sprite_map_key("BaseObject", None),
            DefinitionSprite {
                graphics_scale: 1.0,
                image: ImageData::new(1, 1, vec![0, 0, 200, 255]),
                actions: HashMap::new(),
                color_mask: None,
                shape: Some(DefinitionRect::new(0, 0, 1, 1)),
                fire_top: 0,
                rotateable: 0,
                line: 0,
                stretch_growth: false,
                top_face: None,
                picture: None,
            },
        );

        let mut graphics = GraphicsSystem::new(
            80,
            60,
            60,
            "TopFace pass",
            test_font(),
            Arc::new(sprites),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics.render_frame(
            &snapshot,
            &[ViewportInput::from_focus(&snapshot.objects[0])],
        );
        let (viewport_x, viewport_y) = graphics.viewport();
        let x = (105 - viewport_x) as u32;
        let y = (100 - viewport_y) as u32;
        assert_eq!(
            graphics.surface().get_pixel(x, y),
            Some(standard_gamma_color(green))
        );
        assert_ne!(
            graphics.surface().get_pixel(x, y),
            Some(standard_gamma_color(blue))
        );
    }

    #[test]
    fn snapshot_order_keeps_elevator_case_over_base_when_y_sort_conflicts() {
        // C4ObjectList draws both its base and TopFace passes Last -> Prev
        // without positional sorting (src/C4ObjectList.cpp:387-396).
        // ELEV explicitly orders ELEC over itself (Elevator/Script.c:12-14),
        // which must still hold after the carriage rises above the base.
        let mut snapshot = make_snapshot();
        snapshot.objects[0].position = Vector2::new(40, 50);
        snapshot.objects[0].container = Some(ObjectId::new(99));

        let elevator_id = ObjectId::new(3);
        let case_id = ObjectId::new(2);
        let mut elevator = snapshot.objects[0].clone();
        elevator.id = elevator_id;
        elevator.definition_id = "ELEV".to_string();
        elevator.position = Vector2::new(64, 50);
        elevator.container = None;
        elevator.crew_member = false;
        elevator.action = clonk_engine::ActionState::new("Active");

        let mut case = elevator.clone();
        case.id = case_id;
        case.definition_id = "ELEC".to_string();
        case.position.y = 40;
        // Object payloads remain canonical by ID; the sidecar is C++'s
        // Last->Prev draw order: ELEV, then ELEC.
        snapshot.objects.extend([case, elevator]);
        snapshot.render_order = vec![ObjectId::new(1), elevator_id, case_id];
        snapshot.landscape = Some(Landscape::flat(128, 100));

        let red = Color::opaque(200, 0, 0);
        let green = Color::opaque(0, 200, 0);
        let mut sprites = HashMap::new();
        sprites.insert(
            sprite_map_key("ELEV", None),
            DefinitionSprite {
                graphics_scale: 1.0,
                image: ImageData::new(1, 1, vec![red.r, red.g, red.b, red.a]),
                actions: HashMap::from([(
                    "Active".to_string(),
                    DefinitionActionGraphics::default(),
                )]),
                color_mask: None,
                shape: Some(DefinitionRect::new(0, 0, 1, 1)),
                fire_top: 0,
                rotateable: 0,
                line: 0,
                stretch_growth: false,
                top_face: Some(DefinitionTargetRect::new(0, 0, 1, 1, 0, 0)),
                picture: None,
            },
        );
        sprites.insert(
            sprite_map_key("ELEC", None),
            DefinitionSprite {
                graphics_scale: 1.0,
                image: ImageData::new(1, 1, vec![green.r, green.g, green.b, green.a]),
                actions: HashMap::from([(
                    "Active".to_string(),
                    DefinitionActionGraphics::default(),
                )]),
                color_mask: None,
                shape: Some(DefinitionRect::new(0, 0, 1, 1)),
                fire_top: 0,
                rotateable: 0,
                line: 0,
                stretch_growth: false,
                top_face: Some(DefinitionTargetRect::new(0, 0, 1, 1, 0, 10)),
                picture: None,
            },
        );

        let mut graphics = GraphicsSystem::new(
            80,
            60,
            60,
            "Elevator object order",
            test_font(),
            Arc::new(sprites),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        let viewports = vec![ViewportInput::from_focus(&snapshot.objects[0])];
        graphics.render_frame(&snapshot, &viewports);

        let (viewport_x, viewport_y) = graphics.viewport();
        assert_eq!(
            graphics
                .surface()
                .get_pixel((64 - viewport_x) as u32, (50 - viewport_y) as u32),
            Some(standard_gamma_color(green)),
            "SetObjectOrder keeps the raised ELEC TopFace over ELEV"
        );
    }

    #[test]
    fn sprite_takes_precedence_over_vertex_polygon() {
        // C4Object::Draw never renders shape vertices as geometry — an
        // object with a graphics facet always blits it (src/C4Object.cpp:
        // 2388-2392 idle DrawFace); the polygon is only our debug fallback
        // for sprite-less objects.
        let mut snapshot = make_snapshot();
        snapshot.objects[0].position = Vector2::new(40, 40);
        snapshot.objects[0].vertices = vec![
            ObjectVertex::new(-6, -6),
            ObjectVertex::new(6, -6),
            ObjectVertex::new(6, 6),
            ObjectVertex::new(-6, 6),
        ];
        snapshot.landscape = Some(Landscape::flat(128, 80));

        let green = Color::opaque(0, 200, 0);
        let mut graphics = GraphicsSystem::new(
            80,
            60,
            60,
            "Sprite Precedence",
            test_font(),
            solid_sprite(
                "TestObject",
                8,
                8,
                green,
                Some(DefinitionRect::new(-4, -4, 8, 8)),
                false,
            ),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        let focus = &snapshot.objects[0];
        let viewports = vec![ViewportInput::from_focus(focus)];
        graphics.render_frame(&snapshot, &viewports);

        let (viewport_x, viewport_y) = graphics.viewport();
        let screen_x = (snapshot.objects[0].position.x - viewport_x) as u32;
        let screen_y = (snapshot.objects[0].position.y - viewport_y) as u32;
        assert_eq!(
            graphics.surface().get_pixel(screen_x, screen_y),
            Some(standard_gamma_color(green)),
            "expected the sprite pixel, not the vertex-polygon fill"
        );
    }

    #[test]
    fn idle_face_is_anchored_at_shape_top_left() {
        // C4Object::Draw anchors the face at the shape top-left:
        // cox = x + Shape.x, coy = y + Shape.y (src/C4Object.cpp:2231),
        // and DrawFace blits Shape.Wdt x Shape.Hgt there
        // (src/C4Object.cpp:438-451) — never centered on the position.
        let mut snapshot = make_snapshot();
        // Keep the focus on a sprite-less dummy so its selection marks /
        // energy bar do not overdraw the probed object.
        snapshot.objects[0].position = Vector2::new(40, 40);
        snapshot.objects[0].definition_id = "FocusDummy".to_string();
        let mut subject = snapshot.objects[0].clone();
        subject.id = ObjectId::new(2);
        subject.definition_id = "TestObject".to_string();
        subject.position = Vector2::new(64, 40);
        subject.crew_member = false;
        snapshot.objects.push(subject);
        snapshot.landscape = Some(Landscape::flat(128, 80));

        let green = Color::opaque(0, 200, 0);
        let mut graphics = GraphicsSystem::new(
            80,
            60,
            60,
            "Idle Anchor",
            test_font(),
            solid_sprite(
                "TestObject",
                8,
                8,
                green,
                Some(DefinitionRect::new(-8, -8, 8, 8)),
                false,
            ),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        let focus = &snapshot.objects[0];
        let viewports = vec![ViewportInput::from_focus(focus)];
        graphics.render_frame(&snapshot, &viewports);

        let (viewport_x, viewport_y) = graphics.viewport();
        let sx = 64 - viewport_x;
        let sy = 40 - viewport_y;
        // The face covers world [56,64) x [32,40).
        assert_eq!(
            graphics
                .surface()
                .get_pixel((sx - 5) as u32, (sy - 5) as u32),
            Some(standard_gamma_color(green)),
            "expected the face inside the shape rect"
        );
        assert_ne!(
            graphics
                .surface()
                .get_pixel((sx + 1) as u32, (sy + 1) as u32),
            Some(standard_gamma_color(green)),
            "face must not extend past the shape rect (centered draw would)"
        );
        assert_ne!(
            graphics
                .surface()
                .get_pixel((sx - 9) as u32, (sy - 9) as u32),
            Some(standard_gamma_color(green)),
            "face must start at the shape top-left"
        );
    }

    #[test]
    fn growing_face_shrinks_toward_the_scaled_shape_rect() {
        // GrowthType con display (src/C4Object.cpp:448-451): the target
        // is swdt*Con/FullCon x shgt*Con/FullCon centered in the
        // con-scaled shape rect (C4Shape::Stretch scales Offset too,
        // src/C4Shape.cpp:105-109) — not centered on the position.
        let mut snapshot = make_snapshot();
        // Keep the focus on a sprite-less dummy so its selection marks /
        // energy bar do not overdraw the probed object.
        snapshot.objects[0].position = Vector2::new(40, 40);
        snapshot.objects[0].definition_id = "FocusDummy".to_string();
        let mut subject = snapshot.objects[0].clone();
        subject.id = ObjectId::new(2);
        subject.definition_id = "TestObject".to_string();
        subject.position = Vector2::new(64, 40);
        subject.crew_member = false;
        subject.construction = FULL_CON / 2;
        snapshot.objects.push(subject);
        snapshot.landscape = Some(Landscape::flat(128, 80));

        let green = Color::opaque(0, 200, 0);
        let mut graphics = GraphicsSystem::new(
            80,
            60,
            60,
            "Growth Anchor",
            test_font(),
            solid_sprite(
                "TestObject",
                8,
                16,
                green,
                Some(DefinitionRect::new(0, -16, 8, 16)),
                true,
            ),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        let focus = &snapshot.objects[0];
        let viewports = vec![ViewportInput::from_focus(focus)];
        graphics.render_frame(&snapshot, &viewports);

        let (viewport_x, viewport_y) = graphics.viewport();
        let sx = 64 - viewport_x;
        let sy = 40 - viewport_y;
        // Con 50%: inst shape (0,-8,4,8), target 4x8 at world [64,68) x [32,40).
        // (Probe the lower face rows: the GUI overlay text occupies the
        // top rows of the tiny test surface.)
        assert_eq!(
            graphics
                .surface()
                .get_pixel((sx + 1) as u32, (sy - 2) as u32),
            Some(standard_gamma_color(green)),
            "expected the half-grown face inside the con-scaled shape"
        );
        assert_ne!(
            graphics
                .surface()
                .get_pixel((sx - 2) as u32, (sy - 2) as u32),
            Some(standard_gamma_color(green)),
            "half-grown face must not spill left of the scaled shape"
        );
        assert_ne!(
            graphics
                .surface()
                .get_pixel((sx + 5) as u32, (sy - 2) as u32),
            Some(standard_gamma_color(green)),
            "half-grown face must be half-width"
        );
    }

    #[test]
    fn base_graphics_variant_selects_the_named_sprite() {
        // SetGraphics swaps GetGraphics() to a named C4AdditionalDefGraphics
        // (src/C4DefGraphics.cpp, C4Object::SetGraphics); the snapshot
        // carries the variant on ObjectBaseGraphics and the renderer must
        // blit that sheet, not the default one.
        let mut snapshot = make_snapshot();
        snapshot.objects[0].position = Vector2::new(40, 40);
        snapshot.objects[0].definition_id = "FocusDummy".to_string();
        let mut subject = snapshot.objects[0].clone();
        subject.id = ObjectId::new(2);
        subject.definition_id = "TestObject".to_string();
        subject.position = Vector2::new(64, 40);
        subject.crew_member = false;
        subject.base_graphics = Some(clonk_engine::ObjectBaseGraphics {
            definition: "TestObject".to_string(),
            graphics_name: Some("2".to_string()),
            blit_mode: 0,
        });
        snapshot.objects.push(subject);
        snapshot.landscape = Some(Landscape::flat(128, 80));

        let red = Color::opaque(200, 0, 0);
        let green = Color::opaque(0, 200, 0);
        let shape = Some(DefinitionRect::new(-4, -4, 8, 8));
        let mut sprites = HashMap::new();
        sprites.extend(
            solid_sprite("TestObject", 8, 8, red, shape, false)
                .as_ref()
                .clone(),
        );
        sprites.insert(
            sprite_map_key("TestObject", Some("2")),
            solid_sprite("TestObject", 8, 8, green, shape, false)
                .as_ref()
                .clone()
                .remove(&sprite_map_key("TestObject", None))
                .expect("variant sprite"),
        );

        let mut graphics = GraphicsSystem::new(
            80,
            60,
            60,
            "Variant",
            test_font(),
            Arc::new(sprites),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        let focus = &snapshot.objects[0];
        let viewports = vec![ViewportInput::from_focus(focus)];
        graphics.render_frame(&snapshot, &viewports);

        let (viewport_x, viewport_y) = graphics.viewport();
        let sx = 64 - viewport_x;
        let sy = 40 - viewport_y;
        assert_eq!(
            graphics
                .surface()
                .get_pixel((sx + 1) as u32, (sy + 1) as u32),
            Some(standard_gamma_color(green)),
            "expected the '2' graphics variant, not the default sheet"
        );
    }

    #[test]
    fn action_facet_is_anchored_at_shape_plus_facet_target() {
        // Regular action facet at full con: drawn facet-sized at
        // cox + Action.FacetX / coy + Action.FacetY (src/C4Object.cpp:
        // 2453-2459), sourcing Facet x/y from the sheet.
        let mut snapshot = make_snapshot();
        // Keep the focus on a sprite-less dummy so its selection marks /
        // energy bar do not overdraw the probed object.
        snapshot.objects[0].position = Vector2::new(40, 40);
        snapshot.objects[0].definition_id = "FocusDummy".to_string();
        let mut subject = snapshot.objects[0].clone();
        subject.id = ObjectId::new(2);
        subject.definition_id = "TestObject".to_string();
        subject.position = Vector2::new(64, 40);
        subject.crew_member = false;
        subject.action = clonk_engine::ActionState::new("Still");
        snapshot.objects.push(subject);
        snapshot.landscape = Some(Landscape::flat(128, 80));

        // 16x8 sheet: left half red, right half green.
        let red = Color::opaque(200, 0, 0);
        let green = Color::opaque(0, 200, 0);
        let mut pixels = Vec::new();
        for _y in 0..8 {
            for x in 0..16 {
                let color = if x < 8 { red } else { green };
                pixels.extend_from_slice(&[color.r, color.g, color.b, color.a]);
            }
        }
        let mut actions = HashMap::new();
        actions.insert(
            "Still".to_string(),
            DefinitionActionGraphics {
                facet: Some(clonk_engine::DefinitionActionFacet {
                    x: 8,
                    y: 0,
                    width: 8,
                    height: 8,
                    target_x: 2,
                    target_y: 4,
                }),
                directions: 1,
                flip_dir: None,
                reverse: false,
                facet_base: false,
                facet_top_face: false,
                facet_target_stretch: false,
                length: Some(1),
            },
        );
        let mut sprites = HashMap::new();
        sprites.insert(
            sprite_map_key("TestObject", None),
            DefinitionSprite {
                graphics_scale: 1.0,
                image: ImageData::new(16, 8, pixels),
                actions,
                color_mask: None,
                shape: Some(DefinitionRect::new(-4, -4, 8, 8)),
                fire_top: 0,
                rotateable: 0,
                line: 0,
                stretch_growth: false,
                top_face: None,
                picture: None,
            },
        );

        let mut graphics = GraphicsSystem::new(
            80,
            60,
            60,
            "Facet Anchor",
            test_font(),
            Arc::new(sprites),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        let focus = &snapshot.objects[0];
        let viewports = vec![ViewportInput::from_focus(focus)];
        graphics.render_frame(&snapshot, &viewports);

        let (viewport_x, viewport_y) = graphics.viewport();
        let sx = 64 - viewport_x;
        let sy = 40 - viewport_y;
        // cox = 64-4, coy = 40-4; facet dest [62,70) x [40,48) with the
        // GREEN sheet half (Facet x=8).
        assert_eq!(
            graphics
                .surface()
                .get_pixel((sx + 1) as u32, (sy + 3) as u32),
            Some(standard_gamma_color(green)),
            "expected the facet at cox+FacetX/coy+FacetY sourcing Facet x/y"
        );
        assert_ne!(
            graphics
                .surface()
                .get_pixel((sx - 3) as u32, (sy - 3) as u32),
            Some(standard_gamma_color(green)),
            "facet must not be centered on the position"
        );
    }

    #[test]
    fn action_facet_target_stretches_exactly_to_target_shape_top() {
        // C4Object::Draw stretches FacetTargetStretch from
        // coy + Action.FacetY through, but not beyond,
        // (Target->y + Target->Shape.y) (src/C4Object.cpp:2426-2438).
        // C4Facet::DrawX scales the declared 2x4 source into that rectangle
        // (src/C4Facet.cpp:296-303).
        let mut snapshot = make_snapshot();
        snapshot.objects[0].position = Vector2::new(40, 60);
        snapshot.objects[0].definition_id = "FocusDummy".to_string();

        let case_id = ObjectId::new(3);
        let mut elevator = snapshot.objects[0].clone();
        elevator.id = ObjectId::new(2);
        elevator.definition_id = "ELEV".to_string();
        elevator.position = Vector2::new(64, 60);
        elevator.crew_member = false;
        elevator.action = clonk_engine::ActionState::new("LiftCase");
        elevator.action.target = Some(case_id);

        let mut case = snapshot.objects[0].clone();
        case.id = case_id;
        case.definition_id = "ELEC".to_string();
        case.position = Vector2::new(64, 95);
        case.crew_member = false;
        case.container = Some(ObjectId::new(99));
        snapshot.objects.extend([elevator, case]);
        snapshot.landscape = Some(Landscape::flat(128, 120));

        let green = Color::opaque(0, 200, 0);
        let mut elevator_pixels = vec![0; 60 * 9 * 4];
        for y in 5..9 {
            for x in 58..60 {
                let offset = (y * 60 + x) * 4;
                elevator_pixels[offset..offset + 4]
                    .copy_from_slice(&[green.r, green.g, green.b, green.a]);
            }
        }
        let lift_case = DefinitionActionGraphics {
            facet: Some(clonk_engine::DefinitionActionFacet {
                x: 58,
                y: 5,
                width: 2,
                height: 4,
                target_x: 13,
                target_y: 13,
            }),
            directions: 1,
            facet_base: false,
            facet_target_stretch: true,
            length: Some(1),
            ..DefinitionActionGraphics::default()
        };
        let mut sprites = HashMap::new();
        sprites.insert(
            sprite_map_key("ELEV", None),
            DefinitionSprite {
                graphics_scale: 1.0,
                image: ImageData::new(60, 9, elevator_pixels),
                actions: HashMap::from([("LiftCase".to_string(), lift_case)]),
                color_mask: None,
                shape: Some(DefinitionRect::new(-14, -28, 28, 56)),
                fire_top: 0,
                rotateable: 0,
                line: 0,
                stretch_growth: false,
                top_face: None,
                picture: None,
            },
        );
        sprites.insert(
            sprite_map_key("ELEC", None),
            DefinitionSprite {
                graphics_scale: 1.0,
                image: ImageData::new(24, 26, vec![0; 24 * 26 * 4]),
                actions: HashMap::new(),
                color_mask: None,
                shape: Some(DefinitionRect::new(-12, -13, 24, 26)),
                fire_top: 0,
                rotateable: 0,
                line: 0,
                stretch_growth: false,
                top_face: None,
                picture: None,
            },
        );

        let mut graphics = GraphicsSystem::new(
            80,
            60,
            60,
            "Facet target stretch",
            test_font(),
            Arc::new(sprites),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        // This regression isolates FacetTargetStretch geometry under C++'s
        // explicit point-filter mode; default non-exact blits are linear.
        graphics.set_point_filtering(true);
        let viewports = vec![ViewportInput::from_focus(&snapshot.objects[0])];
        graphics.render_frame(&snapshot, &viewports);

        let (viewport_x, viewport_y) = graphics.viewport();
        // ELEV: cox=64-14, coy=60-28. Facet target starts at (63,45).
        // ELEC target Shape.y=-13, so the exclusive bottom is 95-13=82.
        let cable_x = (63 - viewport_x) as u32;
        let cable_top = (45 - viewport_y) as u32;
        let cable_last = (81 - viewport_y) as u32;
        let cable_bottom = (82 - viewport_y) as u32;
        assert_eq!(
            graphics.surface().get_pixel(cable_x, cable_top),
            Some(standard_gamma_color(green))
        );
        assert_eq!(
            graphics.surface().get_pixel(cable_x, cable_last),
            Some(standard_gamma_color(green))
        );
        assert_eq!(
            graphics.surface().get_pixel(cable_x + 1, cable_last),
            Some(standard_gamma_color(green))
        );
        assert_ne!(
            graphics.surface().get_pixel(cable_x + 2, cable_last),
            Some(standard_gamma_color(green)),
            "the stretched target keeps the source facet's two-pixel width"
        );
        assert_ne!(
            graphics.surface().get_pixel(cable_x, cable_bottom),
            Some(standard_gamma_color(green)),
            "FacetTargetStretch must stop at the target shape's top edge"
        );
    }

    #[test]
    fn real_tutorial_elevator_facets_construction_and_live_frame_delta_match_cpp() {
        fn draw_point_region(
            surface: &mut Surface,
            rect: &GuiRect,
            image: &ImageData,
            source: SourceRect,
        ) {
            let dest_width = rect.size.width.round() as i32;
            let dest_height = rect.size.height.round() as i32;
            let dest_x = rect.origin.x.round() as i32;
            let dest_y = rect.origin.y.round() as i32;
            assert!(dest_width > 0 && dest_height > 0);
            assert!(source.width > 0 && source.height > 0);

            for dy in 0..dest_height {
                // Independent GL_NEAREST oracle: the rasterizer evaluates
                // texture coordinates at destination pixel centres.
                let source_y = source.y
                    + ((2_i64 * i64::from(dy) + 1) * i64::from(source.height)
                        / (2_i64 * i64::from(dest_height))) as i32;
                for dx in 0..dest_width {
                    let target_x = dest_x + dx;
                    let target_y = dest_y + dy;
                    if target_x < 0
                        || target_y < 0
                        || target_x >= surface.width() as i32
                        || target_y >= surface.height() as i32
                    {
                        continue;
                    }
                    let source_x = source.x
                        + ((2_i64 * i64::from(dx) + 1) * i64::from(source.width)
                            / (2_i64 * i64::from(dest_width))) as i32;
                    assert!(source_x >= 0 && source_y >= 0);
                    let idx = ((source_y as u32 * image.width() + source_x as u32) * 4) as usize;
                    let pixel = &image.pixels()[idx..idx + 4];
                    let source = Color::new(pixel[0], pixel[1], pixel[2], pixel[3]);
                    if source.a == 0 {
                        continue;
                    }
                    let destination = surface
                        .get_pixel(target_x as u32, target_y as u32)
                        .unwrap_or_default();
                    let output = if source.a == 255 {
                        source
                    } else {
                        blend_colors(source, destination)
                    };
                    surface
                        .set_pixel(target_x as u32, target_y as u32, output)
                        .unwrap();
                }
            }
        }

        // Real Tutorial05 starts ELEV at Con=80 with no case
        // (Tutorial05/Script.c:30-34). C4Object::DrawFace exposes the bottom
        // Con slice for construction graphics (src/C4Object.cpp:440-475), and
        // UpdateFace installs a non-growth TopFace only at full con
        // (src/C4Object.cpp:357-376).
        let mut tutorial05 = load_repository_tutorial(5);
        join_repository_player(&mut tutorial05, "real Tutorial05 elevator render");
        let partial_snapshot = tutorial05.snapshot();
        let partial = partial_snapshot
            .objects
            .iter()
            .find(|object| object.definition_id == "ELEV")
            .expect("Tutorial05 creates its partial ELEV");
        assert_eq!(partial.construction, 80_000);
        assert_ne!(
            partial.ocf & clonk_engine::ocf::CONSTRUCT,
            0,
            "the real upright, unburned partial ELEV carries OCF_Construct"
        );
        assert!(
            partial_snapshot
                .objects
                .iter()
                .all(|object| object.definition_id != "ELEC"),
            "ELEV Initialize creates ELEC only after completion"
        );

        let partial_sprites = real_elevator_sprites(&tutorial05);
        let real_elev = partial_sprites
            .get(&sprite_map_key("ELEV", None))
            .expect("real ELEV sprite");
        assert_eq!(real_elev.image.width(), 84);
        assert_eq!(real_elev.image.height(), 56);
        assert_eq!(real_elev.shape, Some(DefinitionRect::new(-14, -28, 28, 56)));
        assert_eq!(
            real_elev.top_face,
            Some(DefinitionTargetRect::new(28, 0, 28, 56, 0, 0))
        );
        assert!(!real_elev.stretch_growth);
        assert!(real_elev.color_mask.is_none());

        let partial_origin = Vector2::new(partial.position.x - 48, partial.position.y - 56);
        let construction_sign = test_support::load_graphics_png("Construction.png");
        assert_eq!(
            (construction_sign.width(), construction_sign.height()),
            (16, 16),
            "C++ fctConstruction uses the whole shipped image"
        );
        let mut partial_graphics = GraphicsSystem::new(
            96,
            112,
            112,
            "real Tutorial05 partial ELEV",
            test_font(),
            Arc::clone(&partial_sprites),
            empty_cursor_atlas(),
            Arc::new(HudGraphics {
                construction: Some(construction_sign.clone()),
                ..HudGraphics::default()
            }),
        );
        // This oracle isolates construction/facet geometry under C++'s
        // explicit PointFiltering mode; default non-exact blits are linear.
        partial_graphics.set_point_filtering(true);
        partial_graphics.surface_mut().fill(Color::opaque(0, 0, 0));
        partial_graphics.viewport_x = partial_origin.x as f32;
        partial_graphics.viewport_y = partial_origin.y as f32;
        partial_graphics.paint_object(
            partial,
            &partial_snapshot.objects,
            &partial_snapshot.players,
            OWNER_NONE,
            1.0,
            &HashMap::new(),
            0,
            None,
        );

        let mut partial_expected = Surface::new(96, 112, PixelFormat::Rgba8888);
        partial_expected.fill(Color::opaque(0, 0, 0));
        // C++ construction display: source y=56*(100-80)/100=11,
        // source/destination h=56*80/100=44; Jolt makes Shape.y=-22.
        draw_point_region(
            &mut partial_expected,
            &GuiRect::new(
                (partial.position.x - 14 - partial_origin.x) as f32,
                (partial.position.y - 22 - partial_origin.y) as f32,
                28.0,
                44.0,
            ),
            &real_elev.image,
            SourceRect::new(0, 11, 28, 44),
        );
        assert_surface_pixels_eq(
            partial_graphics.surface(),
            &partial_expected,
            "real Tutorial05 ELEV must render the exact C++ eighty-percent construction slice",
        );
        let before_top_face = partial_graphics.surface().clone();
        partial_graphics.paint_object_top_face(partial, SpriteBlitState::for_object(partial), None);
        // DrawTopFace bottom-left aligns the 16x16 sign to the Con-scaled
        // Shape (-14,-22,28,44), hence the real ELEV-relative (-14,+6).
        draw_point_region(
            &mut partial_expected,
            &GuiRect::new(
                (partial.position.x - 14 - partial_origin.x) as f32,
                (partial.position.y + 6 - partial_origin.y) as f32,
                16.0,
                16.0,
            ),
            &construction_sign,
            SourceRect::new(0, 0, 16, 16),
        );
        assert_surface_pixels_eq(
            partial_graphics.surface(),
            &partial_expected,
            "an incomplete ELEV draws only its construction sign in the TopFace pass",
        );
        assert_ne!(
            partial_graphics.surface().pixels(),
            before_top_face.pixels(),
            "the real Construction.png sign must visibly change the TopFace pass"
        );

        // Tutorial06 supplies the same real definitions and builds ELEV to
        // completion. Spawn one through the real ELEV Initialize callback so
        // SetAction("LiftCase", pCase) and SetObjectOrder run exactly as in
        // Elevator/Script.c:10-15.
        let mut tutorial06 = load_repository_tutorial(6);
        let elevator_id = tutorial06
            .spawn_object(SpawnConfig::new("ELEV").with_position(Vector2::new(332, 148)))
            .expect("real Tutorial06 ELEV spawns");
        let first_snapshot = tutorial06.snapshot();
        let elevator = first_snapshot
            .object(elevator_id)
            .expect("spawned ELEV is present");
        assert_eq!(elevator.construction, FULL_CON);
        assert_eq!(elevator.action.name, "LiftCase");
        let case_id = elevator.action.target.expect("LiftCase targets real ELEC");
        let first_case = first_snapshot.object(case_id).expect("ELEV creates ELEC");
        assert_eq!(first_case.definition_id, "ELEC");
        assert_eq!(
            first_case.action.name, "Wait",
            "ELEC Initialize selects its facet-less active action"
        );
        // CreateObject(ELEC, 0, +27) supplies the requested construction
        // bottom. Initial DoCon then keeps that bottom fixed while changing
        // ELEC's zero-con shape to its full 26px shape, moving its center up
        // by Shape.Hgt+Shape.y=13 (src/C4Object.cpp:1428-1496).
        assert_eq!(
            first_case.position,
            Vector2::new(elevator.position.x, elevator.position.y + 14)
        );

        let sprites = real_elevator_sprites(&tutorial06);
        let elev_sprite = sprites
            .get(&sprite_map_key("ELEV", None))
            .expect("real Tutorial06 ELEV sprite");
        let lift_case = elev_sprite
            .actions
            .get("LiftCase")
            .expect("real ELEV LiftCase ActMap entry");
        let cable_facet = lift_case.facet.as_ref().expect("LiftCase cable facet");
        assert_eq!(
            (
                cable_facet.x,
                cable_facet.y,
                cable_facet.width,
                cable_facet.height,
                cable_facet.target_x,
                cable_facet.target_y,
            ),
            (58, 5, 2, 4, 13, 0),
            "the five-value C4TargetRect defaults FacetY to zero (src/C4Rect.cpp:80-84)"
        );
        assert!(lift_case.facet_base);
        assert!(lift_case.facet_target_stretch);

        let case_sprite = sprites
            .get(&sprite_map_key("ELEC", None))
            .expect("real Tutorial06 ELEC sprite");
        assert_eq!(case_sprite.image.width(), 24);
        assert_eq!(case_sprite.image.height(), 28);
        assert_eq!(
            case_sprite.shape,
            Some(DefinitionRect::new(-12, -13, 24, 26))
        );
        assert_eq!(
            case_sprite.top_face,
            Some(DefinitionTargetRect::new(0, 0, 24, 26, 0, 0))
        );
        assert!(case_sprite.color_mask.is_none());

        let origin = Vector2::new(elevator.position.x - 48, elevator.position.y - 48);
        let render_elevator_base_and_cable = |snapshot: &SimulationSnapshot| {
            let elevator = snapshot.object(elevator_id).expect("ELEV remains live");
            let mut graphics = GraphicsSystem::new(
                96,
                128,
                128,
                "real Tutorial06 ELEV cable",
                test_font(),
                Arc::clone(&sprites),
                empty_cursor_atlas(),
                empty_hud_graphics(),
            );
            graphics.set_point_filtering(true);
            graphics.surface_mut().fill(Color::opaque(0, 0, 0));
            graphics.viewport_x = origin.x as f32;
            graphics.viewport_y = origin.y as f32;
            graphics.paint_object(
                elevator,
                &snapshot.objects,
                &snapshot.players,
                OWNER_NONE,
                1.0,
                &HashMap::new(),
                0,
                None,
            );
            graphics.surface().clone()
        };
        let expected_elevator_base_and_cable = |snapshot: &SimulationSnapshot| {
            let elevator = snapshot.object(elevator_id).expect("ELEV remains live");
            let case = snapshot.object(case_id).expect("ELEC remains live");
            let mut expected = Surface::new(96, 128, PixelFormat::Rgba8888);
            expected.fill(Color::opaque(0, 0, 0));
            draw_point_region(
                &mut expected,
                &GuiRect::new(
                    (elevator.position.x - 14 - origin.x) as f32,
                    (elevator.position.y - 28 - origin.y) as f32,
                    28.0,
                    56.0,
                ),
                &elev_sprite.image,
                SourceRect::new(0, 0, 28, 56),
            );
            // C4Object::Draw computes the live target every draw:
            // height=(Target.y+Target.Shape.y)-(y+Shape.y+FacetY)
            // (src/C4Object.cpp:2426-2438), then DrawX stretches the declared
            // 2x4 source (src/C4Facet.cpp:296-303).
            let cable_top = elevator.position.y - 28 + cable_facet.target_y;
            let case_top = case.position.y - 13;
            draw_point_region(
                &mut expected,
                &GuiRect::new(
                    (elevator.position.x - 14 + cable_facet.target_x - origin.x) as f32,
                    (cable_top - origin.y) as f32,
                    cable_facet.width as f32,
                    (case_top - cable_top) as f32,
                ),
                &elev_sprite.image,
                SourceRect::new(
                    cable_facet.x,
                    cable_facet.y,
                    cable_facet.width,
                    cable_facet.height,
                ),
            );
            expected
        };
        let render_case = |snapshot: &SimulationSnapshot| {
            let case = snapshot.object(case_id).expect("ELEC remains live");
            let mut graphics = GraphicsSystem::new(
                96,
                128,
                128,
                "real Tutorial06 ELEC carriage",
                test_font(),
                Arc::clone(&sprites),
                empty_cursor_atlas(),
                empty_hud_graphics(),
            );
            graphics.set_point_filtering(true);
            graphics.surface_mut().fill(Color::opaque(0, 0, 0));
            graphics.viewport_x = origin.x as f32;
            graphics.viewport_y = origin.y as f32;
            graphics.paint_object(
                case,
                &snapshot.objects,
                &snapshot.players,
                OWNER_NONE,
                1.0,
                &HashMap::new(),
                0,
                None,
            );
            graphics.paint_object_top_face(case, SpriteBlitState::for_object(case), None);
            graphics.surface().clone()
        };
        let expected_case = |snapshot: &SimulationSnapshot| {
            let case = snapshot.object(case_id).expect("ELEC remains live");
            let mut expected = Surface::new(96, 128, PixelFormat::Rgba8888);
            expected.fill(Color::opaque(0, 0, 0));
            // Wait is active but has neither FacetBase nor Facet, so the
            // C4Object::Draw base pass draws nothing (src/C4Object.cpp:
            // 2419-2496). The full carriage is this one DrawTopFace blit in
            // the second object-list pass (src/C4ObjectList.cpp:387-396;
            // src/C4Object.cpp:2617-2670).
            draw_point_region(
                &mut expected,
                &GuiRect::new(
                    (case.position.x - 12 - origin.x) as f32,
                    (case.position.y - 13 - origin.y) as f32,
                    24.0,
                    26.0,
                ),
                &case_sprite.image,
                SourceRect::new(0, 0, 24, 26),
            );
            expected
        };

        let first_cable = render_elevator_base_and_cable(&first_snapshot);
        assert_surface_pixels_eq(
            &first_cable,
            &expected_elevator_base_and_cable(&first_snapshot),
            "real LiftCase must use the shipped cable facet and live ELEC top",
        );
        let first_carriage = render_case(&first_snapshot);
        assert_surface_pixels_eq(
            &first_carriage,
            &expected_case(&first_snapshot),
            "real ELEC full-con TopFace must render the carriage",
        );
        assert!(
            first_carriage
                .pixels()
                .chunks_exact(4)
                .any(|pixel| pixel != [0, 0, 0, 255]),
            "regression guard: a missing ELEC carriage must fail visibly"
        );

        let moved_position = Vector2::new(first_case.position.x, first_case.position.y + 1);
        tutorial06
            .apply_object_update(case_id, ObjectUpdate::new().with_position(moved_position))
            .expect("move real ELEC by one live simulation pixel");
        let second_snapshot = tutorial06.tick().expect("next Tutorial06 frame");
        assert_eq!(second_snapshot.frame, first_snapshot.frame + 1);
        let second_case = second_snapshot
            .object(case_id)
            .expect("moved ELEC survives");
        assert_eq!(second_case.position, moved_position);

        let second_cable = render_elevator_base_and_cable(&second_snapshot);
        assert_surface_pixels_eq(
            &second_cable,
            &expected_elevator_base_and_cable(&second_snapshot),
            "the cable endpoint must follow the live case by one frame pixel",
        );
        let second_carriage = render_case(&second_snapshot);
        assert_surface_pixels_eq(
            &second_carriage,
            &expected_case(&second_snapshot),
            "the rendered carriage must follow the live case without lag or quantization",
        );
        assert!(
            first_carriage.pixels() != second_carriage.pixels(),
            "consecutive snapshots one pixel apart must produce distinct carriage placement"
        );
    }

    #[test]
    fn high_dpi_cursor_tiers_climb_the_shipped_ladder_by_physical_width() {
        // Deliberate divergence (PORT_STATUS "Deliberate divergences"): C++
        // pins the sheet at index 5 for every width above 1280 and only steps
        // up with Graphics.Scale (src/C4GraphicsResource.cpp:474-491), so a
        // 4K panel at Scale=100 draws a 50px pointer and the shipped 75..338px
        // sheets are never used. The eight sizes are authored on an exact
        // 50/1280 ratio, so selecting by physical width keeps C++'s angular
        // size and every tier stays a 1:1 blit.
        let hd = |width: u32, scale: f32| {
            CursorAtlas::index_for_tiers(width, scale, CursorTiers::HighDpi)
        };

        // At or below the C++ breakpoint the classic choice is already right.
        for width in [320u32, 640, 799, 800, 1279, 1280] {
            assert_eq!(
                hd(width, 1.0),
                CursorAtlas::index_for_scaled_resolution(width, 1.0),
                "width {width} must keep the C++ selection"
            );
        }

        // Above it, each shipped cell size takes over at the width where it
        // matches C++'s 50px-at-1280 ratio.
        for (width, index) in [
            (1281u32, 5usize),
            (1919, 5),
            (1920, 4),
            (2559, 4),
            (2560, 3),
            (3839, 3),
            (3840, 2),
            (5759, 2),
            (5760, 1),
            (8652, 1),
            (8653, 0),
        ] {
            assert_eq!(hd(width, 1.0), index, "physical width {width}");
        }

        // The tier follows physical pixels, so Graphics.Scale cannot shrink
        // the pointer below its angular size the way the C++ shift does.
        assert_eq!(hd(1920, 2.0), 2, "3840 physical through a 2x GUI scale");
        assert_eq!(hd(1280, 3.0), 2, "3840 physical through a 3x GUI scale");

        // Classic selection is untouched by the new policy.
        assert_eq!(
            CursorAtlas::index_for_tiers(3840, 1.0, CursorTiers::Classic),
            5
        );
    }

    #[test]
    fn high_dpi_cursor_tiers_reach_the_drawn_cursor_cell() {
        // The policy is only worth anything if the selected sheet reaches the
        // draw path: C4MouseControl derives its `iOffset` from the live cell
        // size (src/C4MouseControl.cpp:333-344), so the hotspot moves with the
        // tier.
        let sheet = |cell: u32| {
            Some(ImageData::new(
                cell.saturating_mul(39),
                cell,
                vec![255u8; (cell * 39 * cell * 4) as usize],
            ))
        };
        let images = vec![None, None, sheet(150), None, None, sheet(50), None, None];
        let atlas = Arc::new(CursorAtlas::new(images));
        let mut graphics = GraphicsSystem::new(
            3840,
            8,
            8,
            "HD cursor tiers",
            test_font(),
            Arc::new(HashMap::new()),
            Arc::clone(&atlas),
            empty_hud_graphics(),
        );

        assert_eq!(
            graphics.construction_cursor_primary_offset(),
            Some(GuiPoint::new(25.0, 25.0)),
            "the default policy keeps C++'s 50px cell at 3840"
        );

        graphics.set_cursor_tiers(CursorTiers::HighDpi);
        assert_eq!(
            graphics.construction_cursor_primary_offset(),
            Some(GuiPoint::new(75.0, 75.0)),
            "HighDpi selects the 150px sheet and its hotspot at 3840"
        );

        // The policy is configured once at startup, but every resolution
        // change and scenario start builds a fresh GraphicsSystem.
        let mut rebuilt = GraphicsSystem::new(
            3840,
            8,
            8,
            "HD cursor tiers rebuilt",
            test_font(),
            Arc::new(HashMap::new()),
            atlas,
            empty_hud_graphics(),
        );
        rebuilt.inherit_cursor_tiers(&graphics);
        assert_eq!(
            rebuilt.construction_cursor_primary_offset(),
            Some(GuiPoint::new(75.0, 75.0)),
            "a rebuilt viewport must keep the configured cursor policy"
        );
    }

    #[test]
    fn cursor_atlas_matches_cpp_scale_selection() {
        let entries = (0u8..8)
            .map(|index| Some(ImageData::new(1, 1, vec![index, 0, 0, 255])))
            .collect();
        let atlas = CursorAtlas::new(entries);

        // Scale=100 keeps the legacy width-only choice byte-for-byte.
        for width in 640..=3840 {
            let legacy_index = if width >= 1280 {
                5
            } else if width >= 800 {
                6
            } else {
                7
            };
            assert_eq!(
                CursorAtlas::index_for_scaled_resolution(width, 1.0),
                legacy_index,
                "Scale=100 width {width}"
            );
            assert_eq!(
                atlas
                    .image_for_scaled_resolution(width, 1.0)
                    .expect("selected cursor sheet")
                    .pixels()[0],
                legacy_index as u8
            );
        }

        // Strict physical-width edges and the truncated scale shift from
        // C4GraphicsResource.cpp:474-490.
        assert_eq!(CursorAtlas::index_for_scaled_resolution(399, 2.0), 7);
        assert_eq!(CursorAtlas::index_for_scaled_resolution(400, 2.0), 6);
        assert_eq!(CursorAtlas::index_for_scaled_resolution(639, 2.0), 6);
        assert_eq!(CursorAtlas::index_for_scaled_resolution(640, 2.0), 5);
        assert_eq!(CursorAtlas::index_for_scaled_resolution(641, 2.0), 4);
        assert_eq!(CursorAtlas::index_for_scaled_resolution(1280, 2.0), 4);
        assert_eq!(CursorAtlas::index_for_scaled_resolution(853, 1.5), 6);
        assert_eq!(CursorAtlas::index_for_scaled_resolution(854, 1.5), 4);
        assert_eq!(CursorAtlas::index_for_scaled_resolution(512, 2.5), 5);
        assert_eq!(CursorAtlas::index_for_scaled_resolution(513, 2.5), 3);

        let odd_cell = GraphicsSystem::cursor_mark_rect(100.0, 100.0, 12.0, 75, 2.0);
        assert_eq!(odd_cell.origin, GuiPoint::new(81.5, 59.5));
        assert_eq!(odd_cell.size, GuiSize::new(37.5, 37.5));

        let mut sparse = vec![None; 8];
        sparse[5] = Some(ImageData::new(1, 1, vec![1, 2, 3, 255]));
        assert!(
            CursorAtlas::new(sparse)
                .image_for_scaled_resolution(320, 1.0)
                .is_none(),
            "C++ does not substitute a nearby loaded cursor sheet"
        );
    }

    #[test]
    fn all_cursor_phases_use_cpp_cells_and_hotspots() {
        assert_eq!(MouseCursorPhase::Down.hotspot(15), (7, 14));
        assert_eq!(MouseCursorPhase::Right.hotspot(15), (14, 7));
        assert_eq!(MouseCursorPhase::DownRight.hotspot(15), (14, 14));

        let cell = 4u32;
        let mut pixels = Vec::with_capacity((40 * cell * cell * 4) as usize);
        for _y in 0..cell {
            for x in 0..40 * cell {
                let phase = (x / cell) as u8;
                pixels.extend_from_slice(&[phase, phase.wrapping_add(40), 200, 255]);
            }
        }
        let mut entries = vec![None; 8];
        entries[7] = Some(ImageData::new(40 * cell, cell, pixels));
        let mut graphics = GraphicsSystem::new(
            24,
            24,
            24,
            "Mouse scroll cursor",
            test_font(),
            empty_sprites(),
            Arc::new(CursorAtlas::new(entries)),
            empty_hud_graphics(),
        );

        let cases = [
            (MouseCursorPhase::Region, (8, 8)),
            (MouseCursorPhase::Crosshair, (8, 8)),
            (MouseCursorPhase::Enter, (8, 8)),
            (MouseCursorPhase::Grab, (8, 8)),
            (MouseCursorPhase::Chop, (8, 8)),
            (MouseCursorPhase::Dig, (8, 8)),
            (MouseCursorPhase::Build, (8, 8)),
            (MouseCursorPhase::Select, (8, 8)),
            (MouseCursorPhase::Object, (8, 8)),
            (MouseCursorPhase::Ungrab, (8, 8)),
            (MouseCursorPhase::Up, (8, 10)),
            (MouseCursorPhase::Down, (8, 6)),
            (MouseCursorPhase::Left, (10, 8)),
            (MouseCursorPhase::Right, (6, 8)),
            (MouseCursorPhase::UpLeft, (10, 10)),
            (MouseCursorPhase::UpRight, (6, 10)),
            (MouseCursorPhase::DownLeft, (10, 6)),
            (MouseCursorPhase::DownRight, (6, 6)),
            (MouseCursorPhase::JumpLeft, (8, 8)),
            (MouseCursorPhase::JumpRight, (8, 8)),
            (MouseCursorPhase::Drop, (8, 8)),
            (MouseCursorPhase::ThrowRight, (8, 8)),
            (MouseCursorPhase::Put, (8, 8)),
            (MouseCursorPhase::Vehicle, (8, 8)),
            (MouseCursorPhase::VehiclePut, (8, 8)),
            (MouseCursorPhase::ThrowLeft, (8, 8)),
            (MouseCursorPhase::Point, (8, 8)),
            (MouseCursorPhase::DigObject, (8, 8)),
            (MouseCursorPhase::Help, (8, 8)),
            (MouseCursorPhase::DigMaterial, (8, 8)),
            (MouseCursorPhase::Add, (8, 8)),
            (MouseCursorPhase::Construct, (8, 8)),
            (MouseCursorPhase::Attack, (8, 8)),
            (MouseCursorPhase::Nothing, (8, 8)),
        ];
        for (phase, expected_origin) in cases {
            graphics.surface_mut().fill(Color::opaque(0, 0, 0));
            assert!(graphics.draw_mouse_cursor(phase, GuiPoint::new(10.0, 10.0), None,));
            let expected = Color::opaque(
                phase.atlas_phase() as u8,
                phase.atlas_phase() as u8 + 40,
                200,
            );
            let points = (0..24)
                .flat_map(|y| (0..24).map(move |x| (x, y)))
                .filter(|&(x, y)| graphics.surface().get_pixel(x, y) == Some(expected))
                .collect::<Vec<_>>();
            assert_eq!(points.len(), 16, "phase {phase:?} source cell");
            assert_eq!(
                points.iter().map(|point| point.0).min(),
                Some(expected_origin.0),
                "phase {phase:?} x hotspot"
            );
            assert_eq!(
                points.iter().map(|point| point.1).min(),
                Some(expected_origin.1),
                "phase {phase:?} y hotspot"
            );
        }
    }

    #[test]
    fn world_cursor_is_clipped_to_its_viewport() {
        let cell = 4u32;
        let mut pixels = Vec::with_capacity((40 * cell * cell * 4) as usize);
        for _y in 0..cell {
            for x in 0..40 * cell {
                let phase = (x / cell) as u8;
                pixels.extend_from_slice(&[phase, 100, 200, 255]);
            }
        }
        let mut entries = vec![None; 8];
        entries[7] = Some(ImageData::new(40 * cell, cell, pixels));
        let mut graphics = GraphicsSystem::new(
            24,
            24,
            24,
            "Mouse cursor clip",
            test_font(),
            empty_sprites(),
            Arc::new(CursorAtlas::new(entries)),
            empty_hud_graphics(),
        );
        graphics.surface_mut().fill(Color::opaque(0, 0, 0));

        assert!(graphics.draw_mouse_cursor_clipped(
            MouseCursorPhase::Grab,
            SurfaceRect::new(10, 9, 2, 2),
            GuiPoint::new(10.0, 10.0),
            None,
        ));
        let expected = Color::opaque(MouseCursorPhase::Grab.atlas_phase() as u8, 100, 200);
        let points = (0..24)
            .flat_map(|y| (0..24).map(move |x| (x, y)))
            .filter(|&(x, y)| graphics.surface().get_pixel(x, y) == Some(expected))
            .collect::<Vec<_>>();
        assert_eq!(points, vec![(10, 9), (11, 9), (10, 10), (11, 10)]);
        assert_eq!(graphics.surface().clip(), None, "caller clip is restored");
    }

    #[test]
    fn old_style_cursor_uses_cpp_thirteen_pixel_hotspots() {
        let cell = 13u32;
        let mut pixels = Vec::with_capacity((40 * cell * cell * 4) as usize);
        for _y in 0..cell {
            for x in 0..40 * cell {
                let phase = (x / cell) as u8;
                pixels.extend_from_slice(&[phase, phase.wrapping_add(40), 200, 255]);
            }
        }
        let mut entries = vec![None; 8];
        entries[7] = Some(ImageData::new(40 * cell, cell, pixels));
        let mut graphics = GraphicsSystem::new(
            48,
            48,
            48,
            "Old-style mouse cursor",
            test_font(),
            empty_sprites(),
            Arc::new(CursorAtlas::new(entries)),
            empty_hud_graphics(),
        );
        let origin = |graphics: &GraphicsSystem, phase: MouseCursorPhase| {
            let expected = Color::opaque(
                phase.atlas_phase() as u8,
                phase.atlas_phase() as u8 + 40,
                200,
            );
            let points = (0..48)
                .flat_map(|y| (0..48).map(move |x| (x, y)))
                .filter(|&(x, y)| graphics.surface().get_pixel(x, y) == Some(expected))
                .collect::<Vec<_>>();
            (
                points.iter().map(|point| point.0).min(),
                points.iter().map(|point| point.1).min(),
            )
        };
        for (phase, expected) in [
            (MouseCursorPhase::Region, (Some(20), Some(20))),
            (MouseCursorPhase::Select, (Some(20), Some(20))),
            (MouseCursorPhase::Dig, (Some(20), Some(7))),
            (MouseCursorPhase::DigMaterial, (Some(20), Some(7))),
            (MouseCursorPhase::Up, (Some(14), Some(14))),
        ] {
            graphics.surface_mut().fill(Color::opaque(1, 2, 3));
            assert!(graphics.draw_mouse_cursor(phase, GuiPoint::new(20.0, 20.0), None));
            assert_eq!(origin(&graphics, phase), expected, "{phase:?}");
        }

        graphics.surface_mut().fill(Color::opaque(1, 2, 3));
        assert!(graphics.draw_gui_mouse_cursor(GuiPoint::new(20.0, 20.0), true, None));
        assert_eq!(
            origin(&graphics, MouseCursorPhase::Region),
            (Some(20), Some(20))
        );
        assert_eq!(
            origin(&graphics, MouseCursorPhase::Help),
            (Some(25), Some(15))
        );
    }

    #[test]
    fn gui_help_cursor_offsets_second_cell_in_native_pixels() {
        let cell = 4u32;
        let mut pixels = Vec::with_capacity((40 * cell * cell * 4) as usize);
        for _y in 0..cell {
            for x in 0..40 * cell {
                let phase = (x / cell) as u8;
                pixels.extend_from_slice(&[phase, phase, phase, 255]);
            }
        }
        let mut entries = vec![None; 8];
        entries[7] = Some(ImageData::new(40 * cell, cell, pixels));
        let mut graphics = GraphicsSystem::new(
            40,
            40,
            40,
            "GUI help cursor",
            test_font(),
            empty_sprites(),
            Arc::new(CursorAtlas::new(entries)),
            empty_hud_graphics(),
        );
        graphics.surface_mut().fill(Color::opaque(200, 200, 200));

        assert!(graphics.draw_gui_mouse_cursor(GuiPoint::new(10.0, 10.0), true, None));
        let region = Color::opaque(0, 0, 0);
        let help = Color::opaque(29, 29, 29);
        assert_eq!(graphics.surface().get_pixel(8, 8), Some(region));
        assert_eq!(graphics.surface().get_pixel(13, 3), Some(help));

        graphics.set_presentation_scale(0.5);
        graphics.surface_mut().fill(Color::opaque(200, 200, 200));
        assert!(graphics.draw_gui_mouse_cursor(GuiPoint::new(20.0, 20.0), true, None));
        assert_eq!(graphics.surface().get_pixel(16, 16), Some(region));
        assert_eq!(graphics.surface().get_pixel(26, 6), Some(help));
    }

    #[test]
    fn construction_add_marker_uses_primary_offset_before_inverse_scale() {
        let cell = 4u32;
        let mut pixels = Vec::with_capacity((40 * cell * cell * 4) as usize);
        for _y in 0..cell {
            for x in 0..40 * cell {
                let phase = (x / cell) as u8;
                pixels.extend_from_slice(&[phase, phase.wrapping_add(40), 200, 255]);
            }
        }
        let mut entries = vec![None; 8];
        entries[7] = Some(ImageData::new(40 * cell, cell, pixels));
        let mut graphics = GraphicsSystem::new(
            24,
            24,
            24,
            "Construction add marker",
            test_font(),
            empty_sprites(),
            Arc::new(CursorAtlas::new(entries)),
            empty_hud_graphics(),
        );
        graphics.set_presentation_scale(2.0);
        let screen = GuiPoint::new(10.0, 10.0);
        let marker = Color::opaque(31, 71, 200);
        let marker_points = |graphics: &GraphicsSystem| {
            (0..24)
                .flat_map(|y| (0..24).map(move |x| (x, y)))
                .filter(|&(x, y)| graphics.surface().get_pixel(x, y) == Some(marker))
                .collect::<Vec<_>>()
        };

        let cursor_offset = graphics
            .construction_cursor_primary_offset()
            .expect("selected Construct cell");
        assert_eq!(cursor_offset, GuiPoint::new(2.0, 2.0));
        assert!(graphics.draw_construction_add_marker(
            SurfaceRect::new(0, 0, 24, 24),
            screen,
            cursor_offset,
            None,
        ));
        assert_eq!(
            marker_points(&graphics),
            vec![(13, 13), (14, 13), (13, 14), (14, 14)],
            "(cursor*2 - centered 2px offset + 8px) / 2",
        );

        graphics.surface_mut().fill(Color::opaque(0, 0, 0));
        let drag_image_offset = GuiPoint::new(4.0, 6.0); // 8x6 drag image
        assert!(graphics.draw_construction_add_marker(
            SurfaceRect::new(13, 11, 1, 2),
            screen,
            drag_image_offset,
            None,
        ));
        assert_eq!(
            marker_points(&graphics),
            vec![(13, 11), (13, 12)],
            "the marker keeps its native offset but stays inside the viewport clip",
        );

        graphics.surface_mut().fill(Color::opaque(0, 0, 0));
        let fallback = Color::opaque(32, 72, 200);
        assert!(graphics.draw_construction_cursor_fallback(
            SurfaceRect::new(10, 9, 1, 2),
            screen,
            None,
        ));
        let fallback_points = (0..24)
            .flat_map(|y| (0..24).map(move |x| (x, y)))
            .filter(|&(x, y)| graphics.surface().get_pixel(x, y) == Some(fallback))
            .collect::<Vec<_>>();
        assert_eq!(fallback_points, vec![(10, 9), (10, 10)]);
    }

    #[test]
    fn construction_drag_preview_uses_native_mod2_validity_colors() {
        let image = ImageData::new(1, 1, vec![128, 128, 128, 255]);
        let mut graphics = test_graphics(3, 2, 2, "Construction drag MOD2");
        graphics.set_advanced_renderer_config(AdvancedRendererConfig {
            shader: true,
            ..AdvancedRendererConfig::DEFAULT
        });

        graphics.surface_mut().fill(Color::opaque(0, 0, 0));
        assert!(graphics.draw_construction_drag_preview(
            &image,
            SurfaceRect::new(0, 0, 3, 2),
            GuiPoint::new(1.0, 1.0),
            true,
            None,
        ));
        assert_eq!(
            graphics.surface().get_pixel(1, 0),
            Some(Color::opaque(1, 255, 1)),
            "0x1f007f00 uses the native green MOD2 channels",
        );

        graphics.surface_mut().fill(Color::opaque(0, 0, 0));
        assert!(graphics.draw_construction_drag_preview(
            &image,
            SurfaceRect::new(0, 0, 3, 2),
            GuiPoint::new(1.0, 1.0),
            false,
            None,
        ));
        assert_eq!(
            graphics.surface().get_pixel(1, 0),
            Some(Color::opaque(255, 1, 1)),
            "0x8f7f0000 uses the native red MOD2 channels",
        );
    }

    #[test]
    fn construction_drag_preview_uses_cpp_hotspot_clipping_and_gamma() {
        let image = ImageData::new(4, 3, (0..12).flat_map(|_| [128, 128, 128, 255]).collect());
        let mut graphics = test_graphics(4, 3, 3, "Construction drag hotspot");
        graphics.set_advanced_renderer_config(AdvancedRendererConfig {
            shader: true,
            ..AdvancedRendererConfig::DEFAULT
        });
        graphics.set_presentation_scale(2.0);
        let background = Color::opaque(9, 11, 13);
        graphics.surface_mut().fill(background);
        let gamma = clonk_graphics::GammaRamp::from_control_points([0x000000, 0x646464, 0xc8c8c8]);

        assert!(graphics.draw_construction_drag_preview(
            &image,
            SurfaceRect::new(1, 0, 1, 3),
            GuiPoint::new(1.0, 1.0),
            true,
            Some(&gamma),
        ));

        // Wdt/2 and Hgt place the 4x3 image at (-1,-2). Only its last row,
        // columns 1..=3, remains on the logical surface, and the originating
        // viewport clips that further to x=1. Presentation scale does not
        // alter this output-space UI primitive.
        let modulated = gamma_encode_fragment(Color::opaque(1, 255, 1), &gamma);
        for y in 0..3 {
            for x in 0..4 {
                let expected = if y == 0 && x == 1 {
                    modulated
                } else {
                    background
                };
                assert_eq!(
                    graphics.surface().get_pixel(x, y),
                    Some(expected),
                    "pixel ({x},{y})",
                );
            }
        }

        assert!(!graphics.draw_construction_drag_preview(
            &ImageData::new(0, 0, Vec::new()),
            SurfaceRect::new(0, 0, 4, 3),
            GuiPoint::new(1.0, 1.0),
            true,
            None,
        ));
    }

    #[test]
    fn render_frame_draws_scale_selected_player_cursor() {
        // C4Game::DrawCursors (src/C4Game.cpp:1852-1874): while CursorFlash
        // or SelectFlash runs, ONE cell of the mouse-cursor sheet is drawn
        // above the cursor clonk — fctCursor is the 35th square cell (cell
        // size = sheet height, C4GraphicsResource::ApplyCursorGfx,
        // src/C4GraphicsResource.cpp:328-336), NOT the whole sheet.
        let mut snapshot = make_snapshot();
        snapshot.objects[0].owner = 1;
        let object_id = snapshot.objects[0].id;
        snapshot.players.push(PlayerState {
            id: 1,
            cursor: Some(object_id),
            control: clonk_engine::PlayerControlState {
                cursor_flash: 30,
                ..Default::default()
            },
            ..PlayerState::default()
        });

        // 40-cell sheet, 4px cells: cell 35 magenta-ish, everything else green.
        let cell = 4u32;
        let mut cursor_pixels = Vec::new();
        for _y in 0..cell {
            for x in 0..40 * cell {
                if (35 * cell..36 * cell).contains(&x) {
                    cursor_pixels.extend_from_slice(&[123, 45, 210, 255]);
                } else {
                    cursor_pixels.extend_from_slice(&[0, 200, 0, 255]);
                }
            }
        }
        let cursor_pixels = Arc::from(cursor_pixels.into_boxed_slice());
        let cursor_image = ImageData::from_arc(40 * cell, cell, cursor_pixels);
        let mut cursor_entries = vec![None; 8];
        cursor_entries[4] = Some(cursor_image);
        let cursor_atlas = Arc::new(CursorAtlas::new(cursor_entries));

        let mut graphics = GraphicsSystem::new(
            800,
            180,
            150,
            "Cursor Scenario",
            test_font(),
            empty_sprites(),
            cursor_atlas,
            empty_hud_graphics(),
        );
        graphics.set_presentation_scale(2.0);
        let focus = &snapshot.objects[0];
        let viewports = vec![
            ViewportInput::from_focus(focus),
            ViewportInput::from_focus(focus),
        ];
        graphics.render_frame(&snapshot, &viewports);

        let cell_color = [123u8, 45, 210, 255];
        let other_cells = [0u8, 200, 0, 255];
        let mut found = false;
        let mut leaked = false;
        for chunk in graphics.surface().pixels().chunks_exact(4) {
            if chunk == cell_color {
                found = true;
            }
            if chunk == other_cells {
                leaked = true;
            }
        }
        assert!(found, "expected the fctCursor cell above the cursor crew");
        assert!(!leaked, "other sheet cells must not be drawn");
    }

    #[test]
    fn scale_two_cursor_mark_keeps_selected_native_cell_size() {
        let mut snapshot = make_snapshot();
        snapshot.objects[0].owner = 1;
        let object_id = snapshot.objects[0].id;
        snapshot.players.push(PlayerState {
            id: 1,
            cursor: Some(object_id),
            control: clonk_engine::PlayerControlState {
                cursor_flash: 30,
                ..Default::default()
            },
            ..PlayerState::default()
        });

        // Scale=200 and logical width 800 select CursorXLarge (index 4),
        // whose shipped cell is 75 physical pixels.
        let cell = 75u32;
        let mark = [123u8, 45, 210, 255];
        let mut pixels = Vec::with_capacity((40 * cell * cell * 4) as usize);
        for _y in 0..cell {
            for x in 0..40 * cell {
                pixels.extend_from_slice(if (35 * cell..36 * cell).contains(&x) {
                    &mark
                } else {
                    &[0, 200, 0, 255]
                });
            }
        }
        let image = ImageData::new(40 * cell, cell, pixels);
        let mut entries = vec![None; 8];
        entries[4] = Some(image);
        let mut graphics = GraphicsSystem::new(
            800,
            180,
            150,
            "Scaled Cursor Scenario",
            test_font(),
            empty_sprites(),
            Arc::new(CursorAtlas::new(entries)),
            empty_hud_graphics(),
        );
        graphics.set_presentation_scale(2.0);
        let focus = &snapshot.objects[0];
        let viewports = vec![ViewportInput::from_focus(focus)];
        let mut presenter = clonk_scaling::FramePresenter::new(2.0, 1600, 360);
        let mut physical = vec![0; 1600 * 360 * 4];
        presenter
            .present(&mut physical, |frame| {
                graphics.render_frame(&snapshot, &viewports);
                frame.copy_from_slice(graphics.surface().pixels());
                Ok::<bool, ()>(true)
            })
            .expect("present scaled cursor frame");

        let points = physical
            .chunks_exact(4)
            .enumerate()
            .filter_map(|(index, pixel)| (pixel == mark).then_some((index % 1600, index / 1600)))
            .collect::<Vec<_>>();
        let min_x = points
            .iter()
            .map(|point| point.0)
            .min()
            .expect("cursor pixels");
        let max_x = points
            .iter()
            .map(|point| point.0)
            .max()
            .expect("cursor pixels");
        let min_y = points
            .iter()
            .map(|point| point.1)
            .min()
            .expect("cursor pixels");
        let max_y = points
            .iter()
            .map(|point| point.1)
            .max()
            .expect("cursor pixels");
        let physical_width = max_x - min_x + 1;
        let physical_height = max_y - min_y + 1;
        assert!(
            physical_width.abs_diff(cell as usize) <= 1,
            "inverse-scaled 75px cell rendered {physical_width}px wide"
        );
        assert!(
            physical_height.abs_diff(cell as usize) <= 1,
            "inverse-scaled 75px cell rendered {physical_height}px high"
        );
    }

    /// Cursor + flash + a 40-cell atlas sheet so the mark (cell 35) draws.
    fn cursor_label_fixture(info_name: Option<&str>) -> (SimulationSnapshot, GraphicsSystem) {
        let mut snapshot = make_snapshot();
        snapshot.objects[0].owner = 1;
        snapshot.objects[0].position = Vector2::new(160, 90);
        let object_id = snapshot.objects[0].id;
        snapshot.players.push(PlayerState {
            id: 1,
            cursor: Some(object_id),
            control: clonk_engine::PlayerControlState {
                cursor_flash: 30,
                ..Default::default()
            },
            ..PlayerState::default()
        });

        let cell = 4u32;
        let pixels: Vec<u8> = (0..40 * cell * cell)
            .flat_map(|_| [0u8, 200, 0, 255])
            .collect();
        let cursor_image = ImageData::new(40 * cell, cell, pixels);
        let mut cursor_entries = vec![None; 8];
        cursor_entries[7] = Some(cursor_image);
        let cursor_atlas = Arc::new(CursorAtlas::new(cursor_entries));

        let mut graphics = GraphicsSystem::new(
            320,
            180,
            150,
            "Cursor Label Scenario",
            test_font(),
            empty_sprites(),
            cursor_atlas,
            empty_hud_graphics(),
        );
        let players = vec![PlayerOverlay {
            owner: 1,
            name: "P1".to_string(),
            wealth: 0,
            score: 0,
            view_wealth: false,
            view_value: false,
            cursor: Some(object_id),
            captain: None,
            eliminated: false,
            owner_color: Color::opaque(0, 100, 200),
            select_count: 1,
            show_startup: false,
            control_set: -1,
            mouse_control: false,
            show_control: 0,
            show_control_position: 0,
            last_com: 0,
            control_key_labels: Vec::new(),
            crew_count: 1,
            crew: vec![CrewOverlay {
                view_energy: 100,
                object_id,
                label: "Joe".to_string(),
                energy: 100,
                energy_capacity: 100,
                magic_energy: 0,
                magic_capacity: 0,
                breath: 0,
                breath_capacity: 0,
                is_focus: true,
                hide_hud_elements: 0,
                hide_hud_bars: 0,
                portrait: None,
                portrait_owner_overlay: None,
                portrait_owner_color: u32::MAX,
                rank: 0,
                rank_symbols: None,
                rank_symbol_count: None,
                info_name: info_name.map(str::to_string),
                rank_name: None,
                inventory: Vec::new(),
            }],
            commands: Vec::new(),
            flash_command: 0,
        }];
        graphics.update_overlay(&GraphicsOverlay {
            frame_text: "",
            status_text: "",
            debug_hud: false,
            viewport_overlays_visible: true,
            players,
            crew_name_labels: Vec::new(),
            game_time_seconds: 0,
            message_board: MessageBoardOverlay::default(),
            clock_text: None,
            frames_per_second: None,
            upper_board_mode: hud::UpperBoardMode::Full,
            show_portraits: true,
            show_commands: true,
            show_command_keys: true,
        });
        (snapshot, graphics)
    }

    fn count_red_text_pixels(graphics: &GraphicsSystem) -> usize {
        let red = standard_gamma_color(Color::opaque(255, 0, 0));
        graphics
            .surface()
            .pixels()
            .chunks_exact(4)
            .filter(|chunk| *chunk == [red.r, red.g, red.b, red.a])
            .count()
    }

    fn count_cursor_info_white_pixels(graphics: &GraphicsSystem) -> usize {
        let white = standard_gamma_color(Color::opaque(255, 255, 255));
        let width = graphics.surface().width() as usize;
        graphics
            .surface()
            .pixels()
            .chunks_exact(4)
            .enumerate()
            .filter(|(index, chunk)| {
                index % width < 100
                    && index / width < 40
                    && *chunk == [white.r, white.g, white.b, white.a]
            })
            .count()
    }

    #[test]
    fn cursor_name_label_drawn_in_red_above_cursor_mark() {
        // C4Game::DrawCursors (src/C4Game.cpp:1873-1887): with cursor->Info,
        // the crew name is drawn in FontRegular, color 0xffff0000, centered
        // above the flashing cursor mark.
        let (snapshot, mut graphics) = cursor_label_fixture(Some("Joe"));
        let focus = &snapshot.objects[0];
        let viewports = vec![ViewportInput::from_focus(focus)];
        graphics.render_frame(&snapshot, &viewports);
        assert!(
            count_red_text_pixels(&graphics) > 0,
            "expected red 0xffff0000 name text above the cursor mark"
        );
        assert!(
            count_cursor_info_white_pixels(&graphics) > 0,
            "cursor object info also draws the white HUD row"
        );
    }

    #[test]
    fn crew_name_label_respects_display_flags() {
        // The app projects ShowCrewNames/ShowCrewCNames into zero or one
        // precomposed label. The world renderer consumes that projection only
        // for an allowed viewport (src/C4Object.cpp:2582-2612).
        let mut snapshot = make_snapshot();
        snapshot.objects[0].owner = 1;
        snapshot.objects[0].ocf |= clonk_engine::ocf::CREW_MEMBER;
        snapshot.players = vec![
            PlayerState {
                id: 0,
                ..PlayerState::default()
            },
            PlayerState {
                id: 1,
                color: Some(RgbColor::new(220, 40, 20)),
                ..PlayerState::default()
            },
        ];
        let object_id = snapshot.objects[0].id;
        let sprites = solid_sprite(
            "TestObject",
            12,
            20,
            Color::opaque(20, 80, 160),
            Some(DefinitionRect::new(-6, -10, 12, 20)),
            false,
        );
        let render_at = |text: Option<&str>,
                         visible_to: Vec<i32>,
                         overlays_visible: bool,
                         width: u32,
                         height: u32,
                         zoom: f32| {
            let mut graphics = GraphicsSystem::new(
                width,
                height,
                height as i32,
                "Crew label",
                test_font(),
                Arc::clone(&sprites),
                empty_cursor_atlas(),
                empty_hud_graphics(),
            );
            graphics.update_overlay(&GraphicsOverlay {
                frame_text: "",
                status_text: "",
                debug_hud: false,
                viewport_overlays_visible: overlays_visible,
                players: Vec::new(),
                crew_name_labels: text
                    .map(|text| {
                        vec![CrewNameOverlay {
                            object_id,
                            text: text.to_string(),
                            visible_to,
                        }]
                    })
                    .unwrap_or_default(),
                game_time_seconds: 0,
                message_board: MessageBoardOverlay::default(),
                clock_text: None,
                frames_per_second: None,
                upper_board_mode: hud::UpperBoardMode::Full,
                show_portraits: true,
                show_commands: true,
                show_command_keys: true,
            });
            graphics.render_frame(
                &snapshot,
                &[ViewportInput::new(
                    0,
                    snapshot.objects[0].position,
                    zoom,
                    &snapshot.objects[0],
                )],
            );
            graphics.surface().pixels().to_vec()
        };
        let render = |text: Option<&str>, visible_to: Vec<i32>, overlays_visible: bool| {
            render_at(text, visible_to, overlays_visible, 200, 120, 1.0)
        };

        let hidden = render(None, Vec::new(), true);
        let player_name = render(Some("Owner"), vec![0], true);
        let clonk_name = render(Some("Clonk"), vec![0], true);
        let both_names = render(Some("Clonk (Owner)"), vec![0], true);
        assert_ne!(player_name, hidden, "ShowCrewNames draws the player name");
        assert_ne!(clonk_name, hidden, "ShowCrewCNames draws the clonk name");
        assert_ne!(both_names, player_name, "both flags use the composed label");
        assert_ne!(both_names, clonk_name, "both flags retain both names");
        assert_eq!(
            render(Some("<c ff0000>Clonk</c>"), vec![0], true),
            clonk_name,
            "fallback rendering consumes markup instead of drawing its tags literally"
        );
        assert_eq!(
            render(Some("Clonk (Owner)"), vec![1], true),
            hidden,
            "the owning/non-visible viewport receives no label"
        );
        assert_eq!(
            render(Some("Clonk (Owner)"), vec![0], false),
            hidden,
            "film replay suppresses world crew labels"
        );

        let zoomed_hidden = render_at(None, Vec::new(), true, 800, 240, 2.0);
        let zoomed_wrapped = render_at(
            Some("Alpha Bravo Charlie Delta Echo Foxtrot Golf"),
            vec![0],
            true,
            800,
            240,
            2.0,
        );
        let changed_rows = zoomed_wrapped
            .chunks_exact(4)
            .zip(zoomed_hidden.chunks_exact(4))
            .enumerate()
            .filter_map(|(pixel, (actual, baseline))| {
                (actual != baseline).then_some((pixel / 800) as i32)
            })
            .collect::<Vec<_>>();
        let changed_span = changed_rows
            .last()
            .zip(changed_rows.first())
            .map_or(0, |(last, first)| last - first + 1);
        assert!(
            changed_span > 14,
            "2x zoom wraps against the 400-unit logical viewport, not 800 physical pixels"
        );
    }

    #[test]
    fn film_replay_hides_player_hud_and_world_cursor_marks() {
        let (snapshot, mut graphics) = cursor_label_fixture(Some("Joe"));
        let viewports = vec![ViewportInput::from_focus(&snapshot.objects[0])];
        let cursor_color = standard_gamma_color(Color::opaque(0, 200, 0));
        let count_cursor_pixels = |graphics: &GraphicsSystem| {
            graphics
                .surface()
                .pixels()
                .chunks_exact(4)
                .filter(|pixel| {
                    *pixel
                        == [
                            cursor_color.r,
                            cursor_color.g,
                            cursor_color.b,
                            cursor_color.a,
                        ]
                })
                .count()
        };

        graphics.render_frame(&snapshot, &viewports);
        assert!(
            count_cursor_pixels(&graphics) > 0,
            "ordinary play draws Cursor.png"
        );
        assert!(
            count_red_text_pixels(&graphics) > 0,
            "ordinary play draws the cursor label"
        );
        assert!(
            count_cursor_info_white_pixels(&graphics) > 0,
            "ordinary play draws cursor-info HUD text"
        );

        let players = graphics.hud_players.clone();
        graphics.update_overlay(&GraphicsOverlay {
            frame_text: "",
            status_text: "",
            debug_hud: false,
            viewport_overlays_visible: false,
            players,
            crew_name_labels: Vec::new(),
            game_time_seconds: 0,
            message_board: MessageBoardOverlay::default(),
            clock_text: None,
            frames_per_second: None,
            upper_board_mode: hud::UpperBoardMode::Full,
            show_portraits: true,
            show_commands: true,
            show_command_keys: true,
        });
        graphics.render_frame(&snapshot, &viewports);

        assert_eq!(
            count_cursor_pixels(&graphics),
            0,
            "film replay hides Cursor.png"
        );
        assert_eq!(
            count_red_text_pixels(&graphics),
            0,
            "film replay hides cursor labels"
        );
        assert_eq!(
            count_cursor_info_white_pixels(&graphics),
            0,
            "film replay hides the per-player HUD"
        );

        graphics.surface_mut().fill(Color::transparent());
        graphics.draw_viewport_control_overlays(None, false, None, None);
        assert!(
            graphics
                .surface()
                .pixels()
                .iter()
                .all(|channel| *channel == 0),
            "film replay hides the late viewport command buttons"
        );
    }

    #[test]
    fn cursor_label_fog_is_sampled_per_glyph_instead_of_once_per_line() {
        let mut raster = clonk_graphics::clonk_font::ClonkFont::new(1);
        raster.add_glyph(
            'A',
            clonk_graphics::clonk_font::GlyphCell {
                width: 4,
                pixels: vec![Color::opaque(255, 255, 255); 8],
            },
        );
        let font = hud::HudFont::Clonk(&raster);
        let fog = FogDrawContext {
            map: Arc::new(ClrModMap {
                resolution_x: 4,
                resolution_y: 2,
                width: 4,
                height: 2,
                origin_x: 0,
                origin_y: 0,
                fade_transparent: false,
                cells: vec![0, 0x00ff_ffff, 0, 0, 0, 0x00ff_ffff, 0, 0],
            }),
            zoom: 1.0,
        };
        let mut surface = Surface::new(9, 2, PixelFormat::Rgba8888);

        draw_fogged_cursor_text_line(
            &mut surface,
            &font,
            4,
            0,
            "AA",
            Color::opaque(255, 0, 0),
            None,
            AdvancedRendererConfig::DEFAULT,
            &fog,
        );

        let distinct_red: HashSet<u8> = (0..surface.width())
            .filter_map(|x| surface.get_pixel(x, 0))
            .filter(|pixel| pixel.r != 0)
            .map(|pixel| pixel.r)
            .collect();
        assert!(
            distinct_red.len() > 1,
            "glyph-local fog vertices must produce a spatially varying label",
        );
    }

    #[test]
    fn retained_fogged_cursor_text_is_one_textured_gamma_quad_per_glyph() {
        let mut raster = clonk_graphics::clonk_font::ClonkFont::new(1);
        raster.add_glyph(
            'A',
            clonk_graphics::clonk_font::GlyphCell {
                width: 2,
                pixels: vec![Color::new(255, 255, 255, 128); 4],
            },
        );
        let font = hud::HudFont::Clonk(&raster);
        let fog = FogDrawContext {
            map: Arc::new(ClrModMap {
                resolution_x: 64,
                resolution_y: 64,
                width: 2,
                height: 2,
                origin_x: 0,
                origin_y: 0,
                fade_transparent: false,
                cells: vec![0x00ff_ffff; 4],
            }),
            zoom: 1.0,
        };
        let gamma = clonk_graphics::GammaRamp::standard();
        let mut surface = Surface::new(3, 2, PixelFormat::Rgba8888);
        surface.begin_gpu_scene_capture();

        draw_fogged_cursor_text_line(
            &mut surface,
            &font,
            1,
            0,
            "A",
            Color::new(255, 0, 0, 192),
            Some(&gamma),
            AdvancedRendererConfig::DEFAULT,
            &fog,
        );

        let scene = surface
            .take_gpu_scene_capture()
            .expect("GPU capture remains active")
            .into_scene([3, 2], Color::transparent(), &gamma);
        assert_eq!(scene.commands.len(), 1);
        assert_eq!(scene.textures.len(), 1);
        let GpuCommand::Quad {
            vertices,
            sampler,
            blend,
            gamma,
            ..
        } = &scene.commands[0]
        else {
            panic!("fogged text did not lower to a textured glyph quad");
        };
        assert_eq!(vertices.len(), 4);
        assert_eq!(*sampler, GpuSampler::Linear);
        assert_eq!(*blend, GpuBlend::Normal);
        assert!(*gamma);
    }

    #[test]
    fn premultiplied_text_layer_unpremultiplies_without_color_saturation() {
        let mut surface = Surface::new(1, 1, PixelFormat::Rgba8888);
        PremultipliedTextLayer::new(&mut surface)
            .blend_fragment(0, 0, [120.0, 80.0, 40.0, 128.0], None)
            .unwrap();

        assert_eq!(surface.get_pixel(0, 0), Some(Color::new(60, 40, 20, 128)));
        let image = retained_straight_alpha_text_image(&surface);
        assert_eq!(image.pixels(), &[120, 80, 40, 128]);
    }

    #[test]
    fn retained_fogged_markup_text_is_a_stable_textured_draw_not_point_coverage() {
        let mut raster = clonk_graphics::clonk_font::ClonkFont::new(1);
        raster.add_glyph(
            'A',
            clonk_graphics::clonk_font::GlyphCell {
                width: 3,
                pixels: vec![Color::opaque(255, 255, 255); 6],
            },
        );
        let font = hud::HudFont::Clonk(&raster);
        let fog = FogDrawContext {
            map: Arc::new(ClrModMap {
                resolution_x: 64,
                resolution_y: 64,
                width: 2,
                height: 2,
                origin_x: 0,
                origin_y: 0,
                fade_transparent: false,
                cells: vec![0x00ff_ffff; 4],
            }),
            zoom: 1.0,
        };
        let mut surface = Surface::new(8, 4, PixelFormat::Rgba8888);
        surface.begin_gpu_scene_capture();

        draw_fogged_markup_text(
            &mut surface,
            &font,
            4,
            0,
            "<i>A</i>",
            Color::opaque(240, 120, 40),
            None,
            AdvancedRendererConfig::DEFAULT,
            &fog,
        );

        let scene = surface
            .take_gpu_scene_capture()
            .expect("markup capture remains active")
            .into_scene(
                [8, 4],
                Color::transparent(),
                &clonk_graphics::GammaRamp::standard(),
            );
        assert!(!scene.commands.is_empty());
        assert!(
            scene
                .commands
                .iter()
                .all(|command| matches!(command, GpuCommand::Quad { .. })),
            "fogged markup must not lower glyph coverage to retained points"
        );
        assert_eq!(scene.textures.len(), 1);
    }

    #[test]
    fn cursor_name_label_needs_object_info() {
        // `if (cursor->Info)` (src/C4Game.cpp:1873): no info, no label.
        let (snapshot, mut graphics) = cursor_label_fixture(None);
        let focus = &snapshot.objects[0];
        let viewports = vec![ViewportInput::from_focus(focus)];
        graphics.render_frame(&snapshot, &viewports);
        assert_eq!(
            count_red_text_pixels(&graphics),
            0,
            "objects without info draw no cursor label"
        );
        assert_eq!(
            count_cursor_info_white_pixels(&graphics),
            0,
            "objects without info draw no cursor-info name/rank row"
        );
    }

    #[test]
    fn cursor_label_rank_line_stacks_above_the_name() {
        // `Rank > 0` doubles texthgt and prefixes the sRankName line
        // (src/C4Game.cpp:1877-1881), so the label block starts one line
        // higher than the rank-0 name-only label.
        let min_red_y = |graphics: &GraphicsSystem| {
            let red = standard_gamma_color(Color::opaque(255, 0, 0));
            graphics
                .surface()
                .pixels()
                .chunks_exact(4)
                .enumerate()
                .filter(|(_, chunk)| *chunk == [red.r, red.g, red.b, red.a])
                .map(|(index, _)| index / graphics.surface().width() as usize)
                .min()
        };

        let (snapshot, mut graphics) = cursor_label_fixture(Some("Joe"));
        let viewports = vec![ViewportInput::from_focus(&snapshot.objects[0])];
        graphics.render_frame(&snapshot, &viewports);
        let name_only_top = min_red_y(&graphics).expect("name label drawn");

        let (snapshot, mut graphics) = cursor_label_fixture(Some("Joe"));
        let mut players = graphics.hud_players.clone();
        players[0].crew[0].rank = 3;
        players[0].crew[0].rank_name = Some("Captain".to_string());
        graphics.update_overlay(&GraphicsOverlay {
            frame_text: "",
            status_text: "",
            debug_hud: false,
            viewport_overlays_visible: true,
            players,
            crew_name_labels: Vec::new(),
            game_time_seconds: 0,
            message_board: MessageBoardOverlay::default(),
            clock_text: None,
            frames_per_second: None,
            upper_board_mode: hud::UpperBoardMode::Full,
            show_portraits: true,
            show_commands: true,
            show_command_keys: true,
        });
        let viewports = vec![ViewportInput::from_focus(&snapshot.objects[0])];
        graphics.render_frame(&snapshot, &viewports);
        let ranked_top = min_red_y(&graphics).expect("rank|name label drawn");

        assert!(
            ranked_top < name_only_top,
            "rank line must raise the label block (ranked_top={ranked_top}, name_only_top={name_only_top})"
        );
    }

    #[test]
    fn focused_crew_draws_partial_breath_in_the_next_cpp_bar_slot() {
        // C4Viewport::DrawCursorInfo places Breath after Energy and the
        // optional MagicEnergy bar; C4Object::DrawBreath selects bar_idx=2,
        // i.e. EnergyBars columns 4/5 (src/C4Viewport.cpp:920-943;
        // src/C4Object.cpp:2728-2731; src/C4Facet.cpp:334-387).
        let (mut snapshot, mut graphics) = cursor_label_fixture(Some("Joe"));
        snapshot.objects[0].breath = 50;
        snapshot.objects[0].info_physical = Some(clonk_engine::PhysicalInfo {
            breath: 100,
            ..clonk_engine::PhysicalInfo::default()
        });
        graphics.hud_players[0].crew[0].breath = 50;
        graphics.hud_players[0].crew[0].breath_capacity = 100;

        // Sentinel 6x3 EnergyBars sheet: every source column has a distinct
        // opaque color, repeated for top/middle/bottom cells.
        let columns = [
            [220, 0, 0, 255],
            [70, 0, 0, 255],
            [0, 220, 0, 255],
            [0, 70, 0, 255],
            [0, 0, 220, 255],
            [0, 0, 70, 255],
        ];
        let pixels = (0..3).flat_map(|_| columns.into_iter().flatten()).collect();
        graphics.hud_graphics = Arc::new(HudGraphics {
            energy_bars: Some(ImageData::new(6, 3, pixels)),
            ..HudGraphics::default()
        });

        let focus = &snapshot.objects[0];
        graphics.render_frame(&snapshot, &[ViewportInput::from_focus(focus)]);

        let bar_bottom_y = 180 - hud::SYMBOL_SIZE - hud::SYMBOL_BORDER - 1;
        let energy_x = hud::SYMBOL_BORDER as u32;
        let breath_x = energy_x + 2; // one-pixel bar + C++'s one-pixel gap
        assert_eq!(
            graphics.surface().get_pixel(energy_x, bar_bottom_y as u32),
            Some(standard_gamma_color(Color::opaque(220, 0, 0))),
            "energy remains in bar index 0"
        );
        assert_eq!(
            graphics.surface().get_pixel(breath_x, bar_bottom_y as u32),
            Some(standard_gamma_color(Color::opaque(0, 0, 220))),
            "partial breath uses filled source column 4 immediately after energy"
        );

        let portraitless_top_y = hud::SYMBOL_SIZE + 2 * hud::SYMBOL_BORDER;
        let energy_color = standard_gamma_color(Color::opaque(220, 0, 0));
        assert_ne!(
            graphics
                .surface()
                .get_pixel(energy_x, portraitless_top_y as u32),
            Some(energy_color),
            "portraits-on retains the ten-pixel gap above the energy bar"
        );
        let players = graphics.hud_players.clone();
        graphics.update_overlay(&GraphicsOverlay {
            frame_text: "",
            status_text: "",
            debug_hud: false,
            viewport_overlays_visible: true,
            players,
            crew_name_labels: Vec::new(),
            game_time_seconds: 0,
            message_board: MessageBoardOverlay::default(),
            clock_text: None,
            frames_per_second: None,
            upper_board_mode: hud::UpperBoardMode::Full,
            show_portraits: false,
            show_commands: true,
            show_command_keys: true,
        });
        graphics.render_frame(&snapshot, &[ViewportInput::from_focus(focus)]);
        assert_eq!(
            graphics
                .surface()
                .get_pixel(energy_x, portraitless_top_y as u32),
            Some(energy_color),
            "the overlay portrait flag moves the energy bar up ten pixels"
        );

        graphics.hud_players[0].crew[0].magic_energy = 1_000;
        graphics.hud_players[0].crew[0].magic_capacity = 1_999;
        graphics.render_frame(&snapshot, &[ViewportInput::from_focus(focus)]);
        assert_eq!(
            graphics
                .surface()
                .get_pixel(breath_x, portraitless_top_y as u32),
            Some(standard_gamma_color(Color::opaque(0, 220, 0))),
            "magic level and range are divided by 1000 separately before drawing"
        );
        assert_eq!(
            graphics.surface().get_pixel(breath_x, bar_bottom_y as u32),
            Some(standard_gamma_color(Color::opaque(0, 220, 0))),
            "present magic occupies the middle slot with source column 2"
        );
        assert_eq!(
            graphics
                .surface()
                .get_pixel(breath_x + 2, bar_bottom_y as u32),
            Some(standard_gamma_color(Color::opaque(0, 0, 220))),
            "breath shifts one compact slot right when magic is present"
        );
    }

    /// `DrawCursorInfo` gates the whole energy/magic/breath group on
    /// `cursor->ViewEnergy || Config.Graphics.ShowPlayerHUDAlways`
    /// (C4Viewport.cpp:921), so a stale cursor shows no bars at all.
    #[test]
    fn cursor_bars_require_view_energy_when_always_hud_is_disabled() {
        let render = |view_energy: i32, always: bool| {
            let (snapshot, mut graphics) = cursor_label_fixture(None);
            let crew = &mut graphics.hud_players[0].crew[0];
            crew.energy = 100;
            crew.energy_capacity = 100;
            crew.magic_energy = 1_000;
            crew.magic_capacity = 2_000;
            crew.breath = 50;
            crew.breath_capacity = 100;
            crew.hide_hud_elements = 0;
            crew.hide_hud_bars = 0;
            crew.view_energy = view_energy;
            crew.inventory = vec![InventoryOverlay {
                object_id: ObjectId::new(20),
                definition_id: "ITEM".into(),
                picture: Some(ImageData::new(
                    hud::SYMBOL_SIZE as u32,
                    hud::SYMBOL_SIZE as u32,
                    std::iter::repeat_n(
                        [220u8, 0, 220, 255],
                        (hud::SYMBOL_SIZE * hud::SYMBOL_SIZE) as usize,
                    )
                    .flatten()
                    .collect(),
                )),
                count: 1,
                additive: false,
                picture_overlays: Vec::new(),
            }];
            // A synthetic 6x3 EnergyBars atlas, so each bar has its own color.
            let columns = [
                [220, 0, 0, 255],
                [70, 0, 0, 255],
                [0, 220, 0, 255],
                [0, 70, 0, 255],
                [0, 0, 220, 255],
                [0, 0, 70, 255],
            ];
            let pixels = (0..3).flat_map(|_| columns.into_iter().flatten()).collect();
            graphics.hud_graphics = Arc::new(HudGraphics {
                energy_bars: Some(ImageData::new(6, 3, pixels)),
                ..HudGraphics::default()
            });
            graphics.set_renderer_config(always, true);
            graphics.render_frame(
                &snapshot,
                &[ViewportInput::from_focus(&snapshot.objects[0])],
            );
            let bar_y = (180 - hud::SYMBOL_SIZE - hud::SYMBOL_BORDER - 1) as u32;
            let inventory_y = (180 - hud::SYMBOL_BORDER - hud::SYMBOL_SIZE) as u32;
            (
                graphics
                    .surface()
                    .get_pixel(hud::SYMBOL_BORDER as u32, inventory_y),
                [
                    graphics
                        .surface()
                        .get_pixel(hud::SYMBOL_BORDER as u32, bar_y),
                    graphics
                        .surface()
                        .get_pixel((hud::SYMBOL_BORDER + 2) as u32, bar_y),
                    graphics
                        .surface()
                        .get_pixel((hud::SYMBOL_BORDER + 4) as u32, bar_y),
                ],
            )
        };
        let magenta = Some(standard_gamma_color(Color::opaque(220, 0, 220)));
        let red = Some(standard_gamma_color(Color::opaque(220, 0, 0)));
        let green = Some(standard_gamma_color(Color::opaque(0, 220, 0)));
        let blue = Some(standard_gamma_color(Color::opaque(0, 0, 220)));
        let drawn = [red, green, blue];

        // A live timer draws them, and so does the always-HUD option.
        assert_eq!(
            render(100, false).1,
            drawn,
            "a live ViewEnergy draws the bars"
        );
        assert_eq!(render(1, false).1, drawn, "the last tick still draws them");
        assert_eq!(
            render(0, true).1,
            drawn,
            "ShowPlayerHUDAlways draws them regardless"
        );

        // An expired timer without the option draws none of the three, while
        // the inventory beside them is untouched: only the bar group is
        // transient (C4Viewport.cpp:921).
        let (inventory, hidden) = render(0, false);
        assert_eq!(inventory, magenta);
        assert_ne!(hidden[0], red);
        assert_ne!(hidden[1], green);
        assert_ne!(hidden[2], blue);
    }

    #[test]
    fn focused_crew_hud_masks_hide_inventory_and_compact_bars() {
        let render = |hide_hud_elements: i32, hide_hud_bars: i32| {
            let (snapshot, mut graphics) = cursor_label_fixture(None);
            let crew = &mut graphics.hud_players[0].crew[0];
            crew.energy = 100;
            crew.energy_capacity = 100;
            crew.magic_energy = 1_000;
            crew.magic_capacity = 2_000;
            crew.breath = 50;
            crew.breath_capacity = 100;
            crew.hide_hud_elements = hide_hud_elements;
            crew.hide_hud_bars = hide_hud_bars;
            crew.inventory = vec![InventoryOverlay {
                object_id: ObjectId::new(20),
                definition_id: "ITEM".into(),
                picture: Some(ImageData::new(
                    hud::SYMBOL_SIZE as u32,
                    hud::SYMBOL_SIZE as u32,
                    std::iter::repeat_n(
                        [220u8, 0, 220, 255],
                        (hud::SYMBOL_SIZE * hud::SYMBOL_SIZE) as usize,
                    )
                    .flatten()
                    .collect(),
                )),
                count: 1,
                additive: false,
                picture_overlays: Vec::new(),
            }];
            let columns = [
                [220, 0, 0, 255],
                [70, 0, 0, 255],
                [0, 220, 0, 255],
                [0, 70, 0, 255],
                [0, 0, 220, 255],
                [0, 0, 70, 255],
            ];
            let pixels = (0..3).flat_map(|_| columns.into_iter().flatten()).collect();
            graphics.hud_graphics = Arc::new(HudGraphics {
                energy_bars: Some(ImageData::new(6, 3, pixels)),
                ..HudGraphics::default()
            });
            graphics.render_frame(
                &snapshot,
                &[ViewportInput::from_focus(&snapshot.objects[0])],
            );
            let bar_y = (180 - hud::SYMBOL_SIZE - hud::SYMBOL_BORDER - 1) as u32;
            let inventory_y = (180 - hud::SYMBOL_BORDER - hud::SYMBOL_SIZE) as u32;
            [
                graphics
                    .surface()
                    .get_pixel(hud::SYMBOL_BORDER as u32, inventory_y),
                graphics
                    .surface()
                    .get_pixel(hud::SYMBOL_BORDER as u32, bar_y),
                graphics
                    .surface()
                    .get_pixel((hud::SYMBOL_BORDER + 2) as u32, bar_y),
                graphics
                    .surface()
                    .get_pixel((hud::SYMBOL_BORDER + 4) as u32, bar_y),
            ]
        };
        let magenta = Some(standard_gamma_color(Color::opaque(220, 0, 220)));
        let red = Some(standard_gamma_color(Color::opaque(220, 0, 0)));
        let green = Some(standard_gamma_color(Color::opaque(0, 220, 0)));
        let blue = Some(standard_gamma_color(Color::opaque(0, 0, 220)));

        let baseline = render(0, 0);
        assert_eq!(baseline, [magenta, red, green, blue]);

        let hidden_energy = render(0, clonk_engine::HIDE_HUD_BAR_ENERGY);
        assert_eq!(hidden_energy[1], green);
        assert_eq!(hidden_energy[2], blue);

        let hidden_magic = render(0, clonk_engine::HIDE_HUD_BAR_MAGIC_ENERGY);
        assert_eq!(hidden_magic[1], red);
        assert_eq!(hidden_magic[2], blue);

        let hidden_breath = render(0, clonk_engine::HIDE_HUD_BAR_BREATH);
        assert_eq!(hidden_breath[1], red);
        assert_eq!(hidden_breath[2], green);
        assert_ne!(hidden_breath[3], blue);

        let hidden_inventory = render(clonk_engine::HIDE_HUD_ELEMENT_INVENTORY, 0);
        assert_ne!(hidden_inventory[0], magenta);
        assert_eq!(&hidden_inventory[1..], &[red, green, blue]);
    }

    #[test]
    fn no_floating_energy_bars_or_bolt_over_crew() {
        // C4Object::Draw (src/C4Object.cpp:2151-2556) draws NO energy or
        // magic bars attached to the object — energy lives in the HUD
        // corner (C4Viewport::DrawCursorInfo, src/C4Viewport.cpp:920-945).
        // The fctEnergy bolt appears world-space only for NeedEnergy
        // structures, blinking on `Tick35 > 12` (src/C4Object.cpp:2505-2510)
        // — never as a persistent crew marker.
        let mut snapshot = make_snapshot();
        snapshot.frame = 13;
        snapshot.objects[0].position = Vector2::new(40, 40);
        snapshot.objects[0].owner = 1;
        snapshot.objects[0].energy = 70;
        snapshot.objects[0].magic_energy = 30;
        snapshot.objects[0].magic_capacity = 50;
        snapshot.landscape = Some(Landscape::flat(128, 80));
        snapshot.players.push(PlayerState {
            id: 1,
            cursor: Some(snapshot.objects[0].id),
            ..PlayerState::default()
        });

        let bolt = [230u8, 20, 20, 255];
        let bolt_pixels: Vec<u8> = (0..8 * 8).flat_map(|_| bolt).collect();
        let hud = HudGraphics {
            energy: Some(ImageData::new(8, 8, bolt_pixels.clone())),
            magic: Some(ImageData::new(8, 8, bolt_pixels)),
            ..Default::default()
        };

        let mut graphics = GraphicsSystem::new(
            80,
            60,
            60,
            "Energy Scenario",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            Arc::new(hud),
        );
        let focus = &snapshot.objects[0];
        let viewports = vec![ViewportInput::from_focus(focus)];
        graphics.render_frame(&snapshot, &viewports);

        let bar_background = Color::new(16, 24, 40, 210);
        for chunk in graphics.surface().pixels().chunks_exact(4) {
            assert_ne!(chunk, bolt, "no floating Energy/Magic bolt icons");
            assert_ne!(
                chunk,
                [
                    bar_background.r,
                    bar_background.g,
                    bar_background.b,
                    bar_background.a
                ],
                "no floating bar backgrounds"
            );
        }
    }

    #[test]
    fn need_energy_bolt_respects_tick35_flag_shape_and_viewport_projection() {
        // C4Object::Draw centers the unscaled fctEnergy facet above the live
        // Shape and shows it only while Tick35 > 12 (src/C4Object.cpp:2518-2524).
        let mut snapshot = make_snapshot();
        snapshot.objects[0].position = Vector2::new(64, 40);
        snapshot.objects[0].crew_member = false;
        snapshot.objects[0].need_energy = true;
        snapshot.objects[0].blit_mode = C4GFXBLIT_ADDITIVE | C4GFXBLIT_MOD2;
        snapshot.objects[0].color_modulation = 0xff00_ff00;
        snapshot.objects[0].color = 0x00ff_0000;
        snapshot.landscape = Some(Landscape::flat(128, 80));

        let shape = DefinitionRect::new(-3, -4, 6, 9);
        let sprite = DefinitionSprite {
            image: ImageData::new(6, 9, vec![0; 6 * 9 * 4]),
            actions: HashMap::new(),
            color_mask: None,
            graphics_scale: 1.0,
            shape: Some(shape),
            fire_top: 0,
            rotateable: 0,
            line: 0,
            stretch_growth: false,
            top_face: None,
            picture: None,
        };
        let sprites = Arc::new(HashMap::from([(
            sprite_map_key("TestObject", None),
            sprite,
        )]));

        let bolt = Color::opaque(231, 47, 113);
        let bolt_image = ImageData::new(5, 3, [bolt.r, bolt.g, bolt.b, bolt.a].repeat(5 * 3));
        let mut graphics = GraphicsSystem::new(
            160,
            120,
            80,
            "NeedEnergy",
            test_font(),
            sprites,
            empty_cursor_atlas(),
            Arc::new(HudGraphics {
                energy: Some(bolt_image),
                ..HudGraphics::default()
            }),
        );
        graphics.set_rotateable_definitions(HashSet::from(["TestObject".to_string()]));
        let rendered_bolt = standard_gamma_color(bolt);
        let mut hidden_frame = None;
        let visible_pixels = 5 * 3 * 2 * 2;

        for (
            frame,
            need_energy,
            rotation,
            construction,
            expected_changed_pixels,
            expected_origin,
        ) in [
            (12, true, 0, FULL_CON, 0, None),
            (13, true, 0, FULL_CON, visible_pixels, Some((62, 28))),
            (34, true, 0, FULL_CON, visible_pixels, Some((62, 28))),
            (13, true, 90, FULL_CON, visible_pixels, Some((62, 25))),
            (13, true, 360, FULL_CON, visible_pixels, Some((62, 25))),
            (13, true, 90, 100, visible_pixels, Some((62, 27))),
            (35, true, 0, FULL_CON, 0, None),
            (13, false, 0, FULL_CON, 0, None),
        ] {
            snapshot.frame = frame;
            snapshot.objects[0].need_energy = need_energy;
            snapshot.objects[0].rotation = rotation;
            snapshot.objects[0].construction = construction;
            let viewport =
                ViewportInput::new(0, snapshot.objects[0].position, 2.0, &snapshot.objects[0]);
            graphics.render_frame(&snapshot, &[viewport]);

            let hidden_frame =
                hidden_frame.get_or_insert_with(|| graphics.surface().pixels().to_vec());
            let changed_pixels = graphics
                .surface()
                .pixels()
                .chunks_exact(4)
                .zip(hidden_frame.chunks_exact(4))
                .filter(|(actual, hidden)| actual != hidden)
                .count();
            assert_eq!(
                changed_pixels, expected_changed_pixels,
                "NeedEnergy bolt footprint for frame={frame}, need_energy={need_energy}"
            );

            if expected_changed_pixels == 0 {
                continue;
            }
            let projection = graphics.active_viewport_projections()[0];
            let (logical_x, logical_y) =
                expected_origin.expect("visible bolt has an oracle origin");
            let logical_origin = Vector2::new(logical_x, logical_y);
            let (output_x, output_y) = projection.logical_to_output(logical_origin);
            let output_x = output_x.round() as u32;
            let output_y = output_y.round() as u32;
            let surface_width = graphics.surface().width();
            let hidden_pixel = |x: u32, y: u32| {
                let index = ((y * surface_width + x) * 4) as usize;
                Color::new(
                    hidden_frame[index],
                    hidden_frame[index + 1],
                    hidden_frame[index + 2],
                    hidden_frame[index + 3],
                )
            };
            for y in output_y..output_y + 6 {
                for x in output_x..output_x + 10 {
                    assert_ne!(
                        graphics.surface().get_pixel(x, y),
                        Some(hidden_pixel(x, y)),
                        "bolt extent at ({x}, {y})"
                    );
                }
            }
            assert_eq!(
                graphics.surface().get_pixel(output_x + 2, output_y + 2),
                Some(rendered_bolt),
                "an interior texel retains the Energy image color"
            );
            assert_eq!(
                graphics.surface().get_pixel(output_x - 1, output_y),
                Some(hidden_pixel(output_x - 1, output_y)),
                "bolt starts at the exact Shape-centered x coordinate"
            );
            assert_eq!(
                graphics.surface().get_pixel(output_x, output_y - 1),
                Some(hidden_pixel(output_x, output_y - 1)),
                "bolt starts exactly five logical pixels above the facet"
            );
        }

        // C4Object::Draw's bounds return precedes NeedEnergy. Keep the
        // structure body just below the view while its would-be bolt overlaps
        // the bottom edge: neither the body nor the bolt may leak onscreen.
        snapshot.frame = 13;
        snapshot.objects[0].position = Vector2::new(64, 75);
        snapshot.objects[0].rotation = 0;
        snapshot.objects[0].construction = FULL_CON;
        snapshot.objects[0].need_energy = true;
        let mut focus = snapshot.objects[0].clone();
        focus.id = ObjectId::new(2);
        focus.position = Vector2::new(64, 40);
        focus.need_energy = false;
        snapshot.objects.push(focus);
        let viewport =
            ViewportInput::new(0, snapshot.objects[1].position, 2.0, &snapshot.objects[1]);
        graphics.render_frame(&snapshot, &[viewport]);
        assert_eq!(
            graphics.surface().pixels(),
            hidden_frame
                .as_deref()
                .expect("frame 12 established the hidden reference"),
            "an output-culled object cannot leak its NeedEnergy bolt"
        );
    }

    #[test]
    fn select_marks_draw_four_corner_phases_while_select_flash_runs() {
        // C4Object::DrawSelectMark (src/C4Object.cpp:3839-3857): the four
        // PHASES of fctSelectMark (SelectMark.png, 4 square cells of sheet
        // height) sit at the shape corners offset by -2 — never the whole
        // sheet blitted over the object. Gated on the owner's SelectFlash
        // (src/C4Object.cpp:2497-2502).
        let mut snapshot = make_snapshot();
        snapshot.objects[0].position = Vector2::new(40, 40);
        snapshot.objects[0].owner = 1;
        snapshot.landscape = Some(Landscape::flat(128, 80));
        snapshot.players.push(PlayerState {
            id: 1,
            cursor: Some(snapshot.objects[0].id),
            control: clonk_engine::PlayerControlState {
                select_flash: 30,
                ..Default::default()
            },
            ..PlayerState::default()
        });

        let corner_colors = [
            [200u8, 10, 10, 255],
            [10, 200, 10, 255],
            [10, 10, 200, 255],
            [200, 200, 10, 255],
        ];
        let mut pixels = Vec::new();
        for _y in 0..5 {
            for x in 0..20 {
                pixels.extend_from_slice(&corner_colors[(x / 5) as usize]);
            }
        }
        let hud = HudGraphics {
            select_mark: Some(ImageData::new(20, 5, pixels)),
            ..Default::default()
        };

        let mut graphics = GraphicsSystem::new(
            80,
            60,
            60,
            "SelectMark Scenario",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            Arc::new(hud),
        );
        let focus = &snapshot.objects[0];
        let viewports = vec![ViewportInput::from_focus(focus)];
        graphics.render_frame(&snapshot, &viewports);

        let (viewport_x, viewport_y) = graphics.viewport();
        let sx = snapshot.objects[0].position.x - viewport_x;
        let sy = snapshot.objects[0].position.y - viewport_y;
        // Fallback shape (-6,-6,12,12): cox = sx - 6 - 2, corners 12 apart.
        let expected = [
            (sx - 6, sy - 6, corner_colors[0]),
            (sx + 6, sy - 6, corner_colors[1]),
            (sx - 6, sy + 6, corner_colors[2]),
            (sx + 6, sy + 6, corner_colors[3]),
        ];
        for (px, py, color) in expected {
            assert_eq!(
                graphics.surface().get_pixel(px as u32, py as u32),
                Some(Color::new(color[0], color[1], color[2], color[3])),
                "corner phase at ({px}, {py})"
            );
        }
        // The whole-sheet regression put cell colors at the object center.
        let center = graphics.surface().get_pixel(sx as u32, sy as u32);
        assert!(
            corner_colors
                .iter()
                .all(|c| center != Some(Color::new(c[0], c[1], c[2], c[3]))),
            "no sheet cells across the object center"
        );
    }

    #[test]
    fn select_marks_stay_hidden_without_select_flash() {
        // `Game.Players.Get(Owner)->SelectFlash` gate (src/C4Object.cpp:2501).
        let mut snapshot = make_snapshot();
        snapshot.objects[0].position = Vector2::new(40, 40);
        snapshot.objects[0].owner = 1;
        snapshot.landscape = Some(Landscape::flat(128, 80));
        snapshot.players.push(PlayerState {
            id: 1,
            cursor: Some(snapshot.objects[0].id),
            ..PlayerState::default()
        });

        let mark = [200u8, 10, 10, 255];
        let pixels: Vec<u8> = (0..20 * 5).flat_map(|_| mark).collect();
        let hud = HudGraphics {
            select_mark: Some(ImageData::new(20, 5, pixels)),
            ..Default::default()
        };

        let mut graphics = GraphicsSystem::new(
            80,
            60,
            60,
            "SelectMark Scenario",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            Arc::new(hud),
        );
        let focus = &snapshot.objects[0];
        let viewports = vec![ViewportInput::from_focus(focus)];
        graphics.render_frame(&snapshot, &viewports);

        assert!(
            graphics
                .surface()
                .pixels()
                .chunks_exact(4)
                .all(|chunk| chunk != mark),
            "no flash → no select marks"
        );
    }

    #[test]
    fn mouse_drag_candidates_draw_select_marks_without_select_flash() {
        // C4MouseControl::Draw calls its private Selection.DrawSelectMark
        // before drawing the rectangle. Unlike C4Object::Draw's ordinary
        // selected-object path, this has no player SelectFlash gate
        // (C4MouseControl.cpp:317-327; C4ObjectList.cpp:698-703;
        // C4Object.cpp:2497-2502,3853-3869).
        let mut snapshot = make_snapshot();
        snapshot.objects[0].position = Vector2::new(40, 40);
        snapshot.objects[0].owner = 1;
        snapshot.landscape = Some(Landscape::flat(128, 80));
        snapshot.players.push(PlayerState {
            id: 1,
            cursor: Some(snapshot.objects[0].id),
            ..PlayerState::default()
        });

        let corner_colors = [
            [200u8, 10, 10, 255],
            [10, 200, 10, 255],
            [10, 10, 200, 255],
            [200, 200, 10, 255],
        ];
        let mut pixels = Vec::new();
        for _y in 0..5 {
            for x in 0..20 {
                pixels.extend_from_slice(&corner_colors[(x / 5) as usize]);
            }
        }
        let hud = HudGraphics {
            select_mark: Some(ImageData::new(20, 5, pixels)),
            ..Default::default()
        };
        let mut graphics = GraphicsSystem::new(
            80,
            60,
            60,
            "Mouse selection candidates",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            Arc::new(hud),
        );
        let focus = &snapshot.objects[0];
        graphics.render_frame(&snapshot, &[ViewportInput::from_focus(focus)]);
        assert!(corner_colors.iter().all(|color| {
            graphics
                .surface()
                .pixels()
                .chunks_exact(4)
                .all(|pixel| pixel != *color)
        }));

        assert!(graphics.draw_mouse_selection_marks(&snapshot, 1, &[snapshot.objects[0].id], None,));

        for color in corner_colors {
            assert!(
                graphics
                    .surface()
                    .pixels()
                    .chunks_exact(4)
                    .any(|pixel| pixel == color),
                "mouse-local Selection draws corner phase {color:?}"
            );
        }
    }

    #[test]
    fn player_cursor_mark_stays_hidden_without_flash() {
        // The `pPlr->CursorFlash || pPlr->SelectFlash` gate
        // (src/C4Game.cpp:1863): expired flash timers draw no mark.
        let mut snapshot = make_snapshot();
        snapshot.objects[0].owner = 1;
        let object_id = snapshot.objects[0].id;
        snapshot.players.push(PlayerState {
            id: 1,
            cursor: Some(object_id),
            ..PlayerState::default()
        });

        let cell = 4u32;
        let pixels: Vec<u8> = (0..40 * cell * cell)
            .flat_map(|_| [123, 45, 210, 255])
            .collect();
        let cursor_image =
            ImageData::from_arc(40 * cell, cell, Arc::from(pixels.into_boxed_slice()));
        let mut cursor_entries = vec![None; 8];
        cursor_entries[7] = Some(cursor_image);
        let cursor_atlas = Arc::new(CursorAtlas::new(cursor_entries));

        let mut graphics = GraphicsSystem::new(
            320,
            180,
            150,
            "Cursor Scenario",
            test_font(),
            empty_sprites(),
            cursor_atlas,
            empty_hud_graphics(),
        );
        let focus = &snapshot.objects[0];
        let viewports = vec![ViewportInput::from_focus(focus)];
        graphics.render_frame(&snapshot, &viewports);

        let cell_color = [123u8, 45, 210, 255];
        assert!(
            graphics
                .surface()
                .pixels()
                .chunks_exact(4)
                .all(|chunk| chunk != cell_color),
            "no flash → no cursor mark"
        );
    }

    #[test]
    fn sky_fade_color_matches_c4sky_get_sky_fade_clr() {
        // C4Sky::GetSkyFadeClr (C4Sky.cpp:230-236): integer fade between
        // FadeClr1 (world top) and FadeClr2 across the landscape height:
        // iPos2 = iY*256/GBackHgt, channel = (c1*iPos1 + c2*iPos2) >> 8.
        let settings = SkySettings {
            fade_top: RgbColor::new(28, 64, 152),
            fade_bottom: RgbColor::new(192, 196, 252),
            ..Default::default()
        };

        assert_eq!(
            GraphicsSystem::sky_fade_color(&settings, 0, 400),
            RgbColor::new(28, 64, 152),
            "world top shows FadeClr1"
        );
        assert_eq!(
            GraphicsSystem::sky_fade_color(&settings, 400, 400),
            RgbColor::new(192, 196, 252),
            "world bottom shows FadeClr2"
        );
        // iY=100, GBackHgt=400: iPos2 = 64, iPos1 = 192;
        // r = (28*192 + 192*64) >> 8 = 69, g = (64*192 + 196*64) >> 8 = 97,
        // b = (152*192 + 252*64) >> 8 = 177.
        assert_eq!(
            GraphicsSystem::sky_fade_color(&settings, 100, 400),
            RgbColor::new(69, 97, 177),
        );
    }

    #[test]
    fn sky_gradient_shows_fade_top_at_the_top_of_the_view() {
        // C4Sky::Draw without a surface fades FadeClr1 -> FadeClr2 top to
        // bottom (C4Sky.cpp:219-225 via GetSkyFadeClr, C4Sky.cpp:230-236).
        let mut snapshot = make_snapshot();
        snapshot.environment.settings.time_of_day = 0; // full daylight
        snapshot.landscape = None;
        snapshot.objects[0].position = Vector2::new(60, 40);

        let settings = clonk_engine::SkySettings {
            fade_top: RgbColor::new(200, 16, 16),
            fade_bottom: RgbColor::new(16, 16, 200),
            ..Default::default()
        };

        let focus = &snapshot.objects[0];
        let mut graphics = test_graphics(120, 80, 60, "Sky Fade");
        graphics.set_sky(Some(SkyRenderState::new(settings, None)));
        let viewports = vec![ViewportInput::from_focus(focus)];
        graphics.render_frame(&snapshot, &viewports);

        let top = graphics.surface().get_pixel(0, 0).unwrap();
        assert!(
            top.r > top.b,
            "expected the red fade_top at the top of the view, got {top:?}"
        );
    }

    #[test]
    fn sky_draws_distinct_sun_and_moon_for_noon_and_midnight() {
        let background = [24, 48, 96, 255];
        let render = |time_of_day| {
            let mut graphics = test_graphics(120, 80, 80, "Celestial Clock");
            let environment = EnvironmentFrame {
                settings: EnvironmentSettings::new(0).with_time_of_day(time_of_day),
                sky_color: Some(RgbColor::new(background[0], background[1], background[2])),
                ..EnvironmentFrame::default()
            };

            graphics.draw_sky(None, &environment, &[], &[], &[], 1.0, None);
            graphics.surface().pixels().to_vec()
        };

        let noon = render(1_200);
        let midnight = render(1);
        let disabled = render(0);

        assert!(
            noon.chunks_exact(4).any(|pixel| pixel != background),
            "noon must show a sun against a flat sky"
        );
        assert!(
            midnight.chunks_exact(4).any(|pixel| pixel != background),
            "midnight must show a moon against a flat sky"
        );
        assert!(
            disabled.chunks_exact(4).all(|pixel| pixel == background),
            "the native all-zero clock must leave ordinary skies unchanged"
        );
        assert_ne!(noon, midnight, "sun and moon must be visually distinct");
    }

    #[test]
    fn non_clock_time_controller_leaves_the_sky_unchanged() {
        // Arctic's PolarNight controller also uses ID TIME but has no numbered
        // Local(1) clock (FarWorlds.c4d/Arctic.c4d/Environment.c4d/
        // PolarNight.c4d/Script.c:5-22).
        let background = [24, 48, 96, 255];
        let mut object = make_snapshot().objects.remove(0);
        object.definition_id = "TIME".into();
        let environment = EnvironmentFrame {
            sky_color: Some(RgbColor::new(background[0], background[1], background[2])),
            ..EnvironmentFrame::default()
        };
        let mut graphics = test_graphics(120, 80, 80, "Non-clock TIME");

        graphics.draw_sky(None, &environment, &[], &[object], &[], 1.0, None);

        assert!(graphics
            .surface()
            .pixels()
            .chunks_exact(4)
            .all(|pixel| pixel == background));
    }

    #[test]
    fn gpu_capture_retains_the_celestial_body_texture() {
        let mut graphics = test_graphics(120, 80, 80, "Retained Celestial Clock");
        let environment = EnvironmentFrame {
            settings: EnvironmentSettings::new(0).with_time_of_day(1_200),
            sky_color: Some(RgbColor::new(24, 48, 96)),
            ..EnvironmentFrame::default()
        };
        let gamma =
            clonk_graphics::GammaRamp::from_control_points([0x10_20_30, 0x40_50_60, 0x70_80_90]);

        graphics.begin_gpu_scene_capture();
        graphics.draw_sky(None, &environment, &[], &[], &[], 1.0, Some(&gamma));
        let scene = graphics
            .finish_gpu_scene_capture(&gamma)
            .expect("GPU capture remains active");
        let (body_index, texture) = scene
            .commands
            .iter()
            .enumerate()
            .find_map(|(index, command)| match command {
                GpuCommand::Quad {
                    texture,
                    sampler: GpuSampler::Nearest,
                    blend: GpuBlend::Normal,
                    gamma: true,
                    ..
                } => Some((index, *texture)),
                _ => None,
            })
            .expect("the celestial body must be a retained textured quad");

        assert!(matches!(
            scene.commands.first(),
            Some(GpuCommand::Solid { .. })
        ));
        assert!(body_index > 0, "the body must draw after the sky");
        assert!(scene
            .textures
            .iter()
            .any(|resource| resource.id == texture && resource.extent == [24, 24]));
    }

    #[test]
    fn sun_and_moon_follow_the_clock_across_the_sky() {
        // The moving sun in shipped Desert content follows horizon -> apex ->
        // horizon (Worlds.c4f/Desert.c4s/Sonne.c4d/Script.c:103-171). Convert
        // that readout to this port's 600/1200/1800 dawn/noon/dusk clock.
        let background = [24, 48, 96, 255];
        let body_center = |time_of_day| {
            let width = 120_u32;
            let mut graphics = test_graphics(width, 80, 80, "Celestial Orbit");
            let mut settings = EnvironmentSettings::new(0);
            settings.time_of_day = time_of_day;
            let environment = EnvironmentFrame {
                settings,
                sky_color: Some(RgbColor::new(background[0], background[1], background[2])),
                ..EnvironmentFrame::default()
            };
            graphics.draw_sky(None, &environment, &[], &[], &[], 1.0, None);

            let (x_sum, y_sum, count) = graphics
                .surface()
                .pixels()
                .chunks_exact(4)
                .enumerate()
                .filter(|(_, pixel)| *pixel != background)
                .fold(
                    (0_u64, 0_u64, 0_u64),
                    |(x_sum, y_sum, count), (index, _)| {
                        (
                            x_sum + index as u64 % u64::from(width),
                            y_sum + index as u64 / u64::from(width),
                            count + 1,
                        )
                    },
                );
            assert!(count > 0, "time {time_of_day} must expose a celestial body");
            (x_sum as f32 / count as f32, y_sum as f32 / count as f32)
        };

        let dawn = body_center(600);
        let noon = body_center(1_200);
        let dusk = body_center(1_799);
        assert!(dawn.0 > noon.0 && noon.0 > dusk.0);
        assert!(noon.1 < dawn.1 && noon.1 < dusk.1);

        let moonrise = body_center(1_800);
        let midnight = body_center(1);
        let moonset = body_center(599);
        assert!(moonrise.0 > midnight.0 && midnight.0 > moonset.0);
        assert!(midnight.1 < moonrise.1 && midnight.1 < moonset.1);
        assert_eq!(dawn, body_center(3_000), "snapshot clock values must wrap");
    }

    #[test]
    fn shipped_time_object_drives_the_celestial_clock() {
        // The standard TIME definition stores its noon-to-noon 0..10000 clock
        // in Local(1) (Objects.c4d/Environment.c4d/Time.c4d/Script.c:15-16,
        // 52-69). Shipped scenarios use this object instead of Rust's
        // synthetic EnvironmentSettings clock.
        let render = |legacy_time| {
            let mut snapshot = make_snapshot();
            snapshot.environment.sky_color = Some(RgbColor::new(24, 48, 96));
            snapshot.landscape = None;
            snapshot.objects[0].definition_id = "TIME".into();
            snapshot.objects[0].local_vars.insert(
                "__local_1".to_string(),
                clonk_script::Value::Int(legacy_time),
            );

            let mut graphics = test_graphics(120, 80, 80, "Script Time");
            let viewports = vec![ViewportInput::from_focus(&snapshot.objects[0])];
            graphics.render_frame(&snapshot, &viewports);
            graphics.surface().pixels().to_vec()
        };

        let background = [24, 48, 96, 255];
        let noon = render(0);
        let midnight = render(5_000);
        assert!(noon
            .chunks_exact(4)
            .any(|pixel| pixel != background && pixel[0] > pixel[2]));
        assert!(midnight
            .chunks_exact(4)
            .any(|pixel| pixel != background && pixel[2] > pixel[0]));
    }

    #[test]
    fn time_object_selection_follows_master_object_order() {
        // Script FindObject returns the first full-range match while walking
        // Game.Objects First -> Next (C4Game.cpp:1366-1391).
        let render = |clocks: &[(ObjectId, i32)], render_order: Vec<ObjectId>| {
            let mut snapshot = make_snapshot();
            snapshot.environment.sky_color = Some(RgbColor::new(24, 48, 96));
            snapshot.landscape = None;

            let template = snapshot.objects[0].clone();
            for (id, legacy_time) in clocks {
                let mut object = template.clone();
                object.id = *id;
                object.definition_id = "TIME".into();
                object.crew_member = false;
                object.local_vars.insert(
                    "__local_1".to_string(),
                    clonk_script::Value::Int(*legacy_time),
                );
                snapshot.objects.push(object);
            }
            snapshot.render_order = render_order;

            let mut graphics = test_graphics(120, 80, 80, "Ordered Script Time");
            let viewports = vec![ViewportInput::from_focus(&snapshot.objects[0])];
            graphics.render_frame(&snapshot, &viewports);
            graphics.surface().pixels().to_vec()
        };

        let focus_id = ObjectId::new(1);
        let noon_id = ObjectId::new(2);
        let midnight_id = ObjectId::new(3);
        let expected = render(&[(midnight_id, 5_000)], vec![focus_id, midnight_id]);
        let actual = render(
            &[(noon_id, 0), (midnight_id, 5_000)],
            vec![focus_id, noon_id, midnight_id],
        );

        assert!(
            actual == expected,
            "the TIME object first in master order must drive the sky"
        );

        let partial = render(&[(midnight_id, 5_000)], vec![focus_id]);
        assert!(
            partial == expected,
            "objects omitted from a partial draw-order sidecar must remain searchable"
        );

        let legacy = render(&[(noon_id, 0), (midnight_id, 5_000)], Vec::new());
        assert!(
            legacy == expected,
            "the canonical draw-order fallback must be reversed for selection"
        );
    }

    #[test]
    fn sky_gradient_is_not_pre_tinted_by_the_season_curve() {
        // C4Sky::Draw emits GetSkyFadeClr directly (C4Sky.cpp:219-236).
        // C4Weather's season curve is one global gamma control applied to
        // the completed frame (C4GraphicsSystem.cpp:787-809), so tinting
        // only the sky here would apply it once before the global LUT.
        let mut snapshot = make_snapshot();
        snapshot.environment.settings = EnvironmentSettings::new(0)
            .with_season(0)
            .with_temperature(-20)
            .with_gamma_enabled();
        snapshot.landscape = None;
        let fade = RgbColor::new(100, 120, 140);
        let settings = SkySettings {
            fade_top: fade,
            fade_bottom: fade,
            ..Default::default()
        };

        let focus = &snapshot.objects[0];
        let mut graphics = test_graphics(120, 80, 60, "Unmodified Sky Fade");
        graphics.set_sky(Some(SkyRenderState::new(settings, None)));
        let viewports = vec![ViewportInput::from_focus(focus)];
        graphics.render_frame(&snapshot, &viewports);

        assert_eq!(
            graphics.surface().get_pixel(0, 0),
            Some(Color::opaque(fade.r, fade.g, fade.b))
        );
    }

    #[test]
    fn lighting_darkens_sky_at_night() {
        let mut daytime = make_snapshot();
        daytime.environment.sky_color = Some(RgbColor::new(160, 160, 160));
        daytime.environment.settings.time_of_day = EnvironmentSettings::TIME_CYCLE / 2;

        let focus = &daytime.objects[0];
        let mut day_view = test_graphics(120, 80, 60, "Day");
        let day_viewports = vec![ViewportInput::from_focus(focus)];
        day_view.render_frame(&daytime, &day_viewports);
        let day_pixel = day_view.surface().get_pixel(0, 0).unwrap();

        let mut nighttime = daytime.clone();
        // 0 means "no day/night cycle" (full daylight); 1 is deepest night.
        nighttime.environment.settings.time_of_day = 1;
        let mut night_view = test_graphics(120, 80, 60, "Night");
        let night_focus = &nighttime.objects[0];
        let night_viewports = vec![ViewportInput::from_focus(night_focus)];
        night_view.render_frame(&nighttime, &night_viewports);
        let night_pixel = night_view.surface().get_pixel(0, 0).unwrap();

        let base_color = Color::opaque(160, 160, 160);
        let day_factor = GraphicsSystem::lighting_factor(daytime.environment.settings.time_of_day);
        let night_factor =
            GraphicsSystem::lighting_factor(nighttime.environment.settings.time_of_day);
        let expected_day = GraphicsSystem::apply_lighting(base_color, day_factor);
        let expected_night = GraphicsSystem::apply_lighting(base_color, night_factor);

        assert_eq!(day_pixel, expected_day);
        assert_eq!(night_pixel, expected_night);
        assert_ne!(expected_day, expected_night);
    }

    #[test]
    fn lighting_darkens_objects_at_night() {
        let mut daytime = make_snapshot();
        daytime.environment.settings.time_of_day = EnvironmentSettings::TIME_CYCLE / 2;
        daytime.objects[0].position = Vector2::new(150, 140);
        // Keep the probe inside GBackHgt: C4Viewport clips landscape drawing
        // at the borders (C4Viewport.cpp:1035-1041).
        daytime.landscape = Some(Landscape::flat(256, 150));

        let mut day_view = test_graphics(200, 150, 150, "Day Object");
        let day_focus = &daytime.objects[0];
        let day_viewports = vec![ViewportInput::from_focus(day_focus)];
        day_view.render_frame(&daytime, &day_viewports);
        let (day_viewport_x, day_viewport_y) = day_view.viewport();
        let day_screen_x = (daytime.objects[0].position.x - day_viewport_x) as u32;
        let day_screen_y = (daytime.objects[0].position.y - day_viewport_y) as u32;
        let day_pixel = day_view
            .surface()
            .get_pixel(day_screen_x, day_screen_y)
            .unwrap();

        let mut nighttime = daytime.clone();
        // 0 means "no day/night cycle" (full daylight); 1 is deepest night.
        nighttime.environment.settings.time_of_day = 1;
        let mut night_view = test_graphics(200, 150, 150, "Night Object");
        let night_focus = &nighttime.objects[0];
        let night_viewports = vec![ViewportInput::from_focus(night_focus)];
        night_view.render_frame(&nighttime, &night_viewports);
        let (night_viewport_x, night_viewport_y) = night_view.viewport();
        let night_screen_x = (nighttime.objects[0].position.x - night_viewport_x) as u32;
        let night_screen_y = (nighttime.objects[0].position.y - night_viewport_y) as u32;
        let night_pixel = night_view
            .surface()
            .get_pixel(night_screen_x, night_screen_y)
            .unwrap();

        let day_factor = GraphicsSystem::lighting_factor(daytime.environment.settings.time_of_day);
        let night_factor =
            GraphicsSystem::lighting_factor(nighttime.environment.settings.time_of_day);
        assert!(night_factor < day_factor);
        let ratio = if day_factor <= 0.0 {
            0.0
        } else {
            night_factor / day_factor
        };
        let expected_night = GraphicsSystem::apply_lighting(day_pixel, ratio);

        assert_eq!(night_pixel, expected_night);
        assert_ne!(day_pixel, night_pixel);
    }

    #[test]
    fn narrow_world_produces_letterbox_content_rect() {
        let mut snapshot = make_snapshot();
        snapshot.landscape = Some(Landscape::flat(40, 40));
        snapshot.objects[0].position = Vector2::new(20, 20);

        let focus = &snapshot.objects[0];
        let mut graphics = test_graphics(120, 80, 40, "Letterbox");
        let viewports = vec![ViewportInput::new(0, Vector2::new(20, 20), 1.0, focus)];
        graphics.render_frame(&snapshot, &viewports);

        let viewport = graphics
            .active_viewports
            .first()
            .expect("expected active viewport");
        assert!(viewport.content_rect.width < viewport.rect.width);
        assert_eq!(viewport.content_rect.width, 40);

        let left_bar = viewport.content_rect.x - viewport.rect.x;
        let right_bar = (viewport.rect.x + viewport.rect.width as i32)
            - (viewport.content_rect.x + viewport.content_rect.width as i32);
        assert!(left_bar > 0);
        assert!(right_bar > 0);
    }

    #[test]
    fn liquids_overlay_ground_with_blending() {
        let mut snapshot = make_snapshot();
        snapshot.environment.settings.time_of_day = EnvironmentSettings::TIME_CYCLE / 2;
        snapshot.objects[0].position = Vector2::new(40, 50);
        if let Some(landscape) = snapshot.landscape.as_mut() {
            landscape.set_liquid_column(30, vec![LiquidSegment::new(40, 60)]);
        }
        let focus = &snapshot.objects[0];
        let mut graphics = test_graphics(120, 80, 80, "Liquid Scenario");
        let viewports = vec![ViewportInput::from_focus(focus)];
        graphics.render_frame(&snapshot, &viewports);

        let (viewport_x, viewport_y) = graphics.viewport();
        let screen_x = (30 - viewport_x) as u32;
        let screen_y = (50 - viewport_y) as u32;

        let pixel = graphics
            .surface()
            .get_pixel(screen_x, screen_y)
            .expect("pixel in bounds");

        let lighting = GraphicsSystem::lighting_factor(snapshot.environment.settings.time_of_day);
        let liquid = GraphicsSystem::apply_lighting(
            GraphicsSystem::liquid_color_for_temperature(snapshot.environment.ambient_temperature),
            lighting,
        );
        let sky = GraphicsSystem::apply_lighting(
            snapshot
                .environment
                .sky_color
                .map(|color| Color::opaque(color.r, color.g, color.b))
                .unwrap_or_else(|| {
                    GraphicsSystem::sky_color_for_temperature(
                        snapshot.environment.ambient_temperature,
                    )
                }),
            lighting,
        );
        let expected = blend_color_over(liquid, sky);
        assert_eq!(pixel, expected);
    }

    #[test]
    fn liquid_animation_uses_stdgl_call_cycle_and_wrap() {
        let mut animation = LiquidAnimationCycle::default();
        let mut samples = Vec::new();
        for call in 1..=46 {
            let modulation = animation.advance();
            if matches!(call, 1 | 2 | 3 | 14 | 18 | 22 | 46) {
                samples.push((call, modulation.map(f32::to_bits)));
            }
        }
        assert_eq!(
            samples,
            [
                (1, [0xbd4c_cccd, 0x3c88_8889, 0x3daa_aaab]),
                (2, [0xbd08_8889, 0x3d08_8889, 0x3dcc_cccd]),
                (3, [0xbc88_888a, 0x3d4c_cccd, 0x3daa_aaab]),
                (14, [0x3d08_888b, 0xbd08_8890, 0xbdcc_cccd]),
                (18, [0xbd08_888b, 0xbdcc_cccd, 0xbd08_8889]),
                (22, [0xbdcc_cccd, 0xbd08_8889, 0x3d08_8888]),
                (46, [0xbdcc_cccd, 0xbd08_8889, 0x3d08_8888]),
            ]
        );
    }

    #[test]
    fn liquid_animation_cycle_survives_texture_swaps_and_renderer_rebuilds() {
        let make_graphics = || test_graphics(1, 1, 1, "Liquid phase");
        let image = || ImageData::new(1, 1, vec![255, 128, 0, 255]);
        let mut original = make_graphics();
        original.set_liquid_animation(Some(image()));
        assert_eq!(
            original.liquid_animation_cycle.advance().map(f32::to_bits),
            [0xbd4c_cccd, 0x3c88_8889, 0x3daa_aaab]
        );

        original.set_liquid_animation(None);
        let paused = original.liquid_animation_cycle.values;
        original.set_liquid_animation(Some(image()));
        assert_eq!(original.liquid_animation_cycle.values, paused);

        let mut rebuilt = make_graphics();
        rebuilt.inherit_liquid_animation_cycle(&original);
        rebuilt.set_liquid_animation(Some(image()));
        assert_eq!(
            rebuilt.liquid_animation_cycle.advance().map(f32::to_bits),
            [0xbd08_8889, 0x3d08_8889, 0x3dcc_cccd]
        );
    }

    #[test]
    fn liquid_animation_adds_then_clamps_before_vertex_modulation() {
        let blit = SpriteBlitState {
            mode: 0,
            modulation: Some(0x0080_8080),
            fog_modulation: None,
            renderer_config: AdvancedRendererConfig::DEFAULT,
        };
        let PreparedSpriteFragment::Shader { rgb, alpha } =
            prepare_liquid_animation_fragment(Color::opaque(230, 102, 26), 0.2, blit)
        else {
            panic!("liquid animation must retain float shader channels");
        };
        assert_eq!(alpha, 255.0);
        for (actual, expected) in rgb.into_iter().zip([128.0, 76.8, 38.650_98]) {
            assert!((actual - expected).abs() < 0.000_01);
        }

        let PreparedSpriteFragment::Shader { rgb, .. } =
            prepare_liquid_animation_fragment(Color::opaque(10, 20, 30), -0.2, blit)
        else {
            panic!("liquid animation must retain float shader channels");
        };
        assert_eq!(rgb, [0.0; 3]);
    }

    #[test]
    fn liquid_animation_changes_only_liquids_and_reuses_static_cache() {
        let landscape: Landscape = serde_json::from_value(serde_json::json!({
            "width": 2,
            "surface": [1, 1],
            "world_height": 1,
            "shade_materials": false,
            "pixels": {
                "width": 2,
                "height": 1,
                "bytes": "0102",
                "texture_names": [null, "Smooth", "Smooth"],
                "densities": [0, 25, 50],
                "material_names": [null, "Water", "Earth"]
            }
        }))
        .expect("liquid and solid pixel landscape");
        let textures = Arc::new(HashMap::from([
            (
                "liquid".to_string(),
                ImageData::new(1, 1, vec![128, 128, 128, 255]),
            ),
            (
                "smooth".to_string(),
                ImageData::new(1, 1, vec![128, 128, 128, 255]),
            ),
        ]));
        let materials = Arc::new(HashMap::from([
            (
                "water".to_string(),
                MaterialRenderInfo::new(
                    [120, 140, 160, 120, 140, 160, 120, 140, 160],
                    [0; 6],
                    None,
                    0,
                    25,
                ),
            ),
            (
                "earth".to_string(),
                MaterialRenderInfo::new(
                    [80, 100, 120, 80, 100, 120, 80, 100, 120],
                    [0; 6],
                    None,
                    0,
                    50,
                ),
            ),
        ]));
        let make_graphics = || {
            let mut graphics = test_graphics(2, 1, 1, "Liquid animation");
            graphics.set_material_textures(Arc::clone(&textures));
            graphics.set_material_render_info(Arc::clone(&materials));
            graphics
        };

        let mut animated = make_graphics();
        animated.set_liquid_animation(Some(ImageData::new(1, 1, vec![255, 128, 128, 255])));
        assert!(animated.draw_ground_textured(Some(&landscape), None));
        let first = [
            animated.surface().get_pixel(0, 0),
            animated.surface().get_pixel(1, 0),
        ];
        reset_material_composition_calls();
        assert!(animated.draw_ground_textured(Some(&landscape), None));
        let second = [
            animated.surface().get_pixel(0, 0),
            animated.surface().get_pixel(1, 0),
        ];
        assert_ne!(first[0], second[0], "liquid RGB must follow the next phase");
        assert_eq!(first[1], second[1], "solid density must remain static");
        assert_eq!(
            material_composition_calls(),
            0,
            "animation must not invalidate the static Surface32 cache"
        );

        animated.set_liquid_animation(None);
        assert!(animated.draw_ground_textured(Some(&landscape), None));
        let disabled = [
            animated.surface().get_pixel(0, 0),
            animated.surface().get_pixel(1, 0),
        ];
        let mut baseline = make_graphics();
        assert!(baseline.draw_ground_textured(Some(&landscape), None));
        assert_eq!(
            disabled,
            [
                baseline.surface().get_pixel(0, 0),
                baseline.surface().get_pixel(1, 0),
            ],
            "disabling animation must retain the pre-animation renderer bytes"
        );
    }

    #[test]
    fn surface32_landscape_renders_without_material_resources() {
        // Landscape.png supplies C4Landscape::Surface32 directly. It does not
        // depend on a texture/material composition pass, so an authored exact
        // surface remains drawable even when those resource maps are empty.
        let landscape: Landscape = serde_json::from_value(serde_json::json!({
            "width": 2,
            "surface": [1, 1],
            "world_height": 1,
            "shade_materials": false,
            "pixels": {
                "width": 2,
                "height": 1,
                "bytes": "0000",
                "surface32_pixels": {
                    "0": 0x0011_2233_u32,
                    "1": 0x0044_5566_u32
                },
                "texture_names": [],
                "densities": [],
                "material_names": []
            }
        }))
        .expect("PNG-backed exact landscape");
        let mut graphics = test_graphics(2, 1, 1, "Exact Landscape.png");

        assert!(graphics.draw_ground_textured(Some(&landscape), None));
        assert_eq!(
            [
                graphics.surface().get_pixel(0, 0),
                graphics.surface().get_pixel(1, 0),
            ],
            [
                Some(Color::opaque(0x11, 0x22, 0x33)),
                Some(Color::opaque(0x44, 0x55, 0x66)),
            ]
        );
    }

    #[test]
    fn parallel_landscape_rows_match_scalar_fog_liquid_gamma_and_clip() {
        const WORLD_WIDTH: u32 = 128;
        const WORLD_HEIGHT: u32 = 96;
        const SCREEN_WIDTH: u32 = 96;
        const SCREEN_HEIGHT: u32 = 80;

        let bytes = (0..WORLD_HEIGHT)
            .flat_map(|y| {
                (0..WORLD_WIDTH).map(move |x| {
                    if (x * 7 + y * 11) % 23 == 0 {
                        0
                    } else if (x + y * 3) % 5 < 2 {
                        1
                    } else {
                        2
                    }
                })
            })
            .collect::<Vec<_>>();
        let mut cache_pixels = vec![0; (WORLD_WIDTH * WORLD_HEIGHT * 4) as usize];
        for (index, byte) in bytes.iter().copied().enumerate() {
            let x = (index as u32 % WORLD_WIDTH) as u8;
            let y = (index as u32 / WORLD_WIDTH) as u8;
            let color = match byte {
                0 => Color::transparent(),
                1 => Color::new(
                    48u8.wrapping_add(x),
                    112u8.wrapping_add(y),
                    176u8.wrapping_sub(x / 2),
                    127,
                ),
                _ => Color::opaque(
                    96u8.wrapping_add(x / 3),
                    64u8.wrapping_add(y / 2),
                    32u8.wrapping_add((x ^ y) / 4),
                ),
            };
            cache_pixels[index * 4..index * 4 + 4]
                .copy_from_slice(&[color.r, color.g, color.b, color.a]);
        }

        let mut landscape = Landscape::flat(WORLD_WIDTH, WORLD_HEIGHT as i32);
        landscape.set_pixel_grid(PixelGrid::new(
            WORLD_WIDTH,
            WORLD_HEIGHT,
            bytes,
            vec![0, 25, 50],
            vec![None, Some("Water".to_string()), Some("Earth".to_string())],
            vec![None, Some("Smooth".to_string()), Some("Rough".to_string())],
        ));
        landscape.set_shade_materials(false);

        let textures = Arc::new(HashMap::from([(
            "rough".to_string(),
            ImageData::new(1, 1, vec![128, 128, 128, 255]),
        )]));
        let materials = Arc::new(HashMap::from([(
            "earth".to_string(),
            MaterialRenderInfo::new([128; 9], [0; 6], None, 0, 50),
        )]));
        let fog = Arc::new(ClrModMap {
            resolution_x: 16,
            resolution_y: 16,
            width: 10,
            height: 8,
            origin_x: -24,
            origin_y: -16,
            fade_transparent: false,
            cells: (0..80)
                .map(|index| {
                    let red = (64 + index * 17 % 192) as u32;
                    let green = (48 + index * 29 % 208) as u32;
                    let blue = (32 + index * 43 % 224) as u32;
                    (red << 16) | (green << 8) | blue
                })
                .collect(),
        });
        let grid = landscape.pixel_grid().expect("pixel grid").clone();
        let border_state = (
            landscape.left_open(),
            landscape.right_open(),
            landscape.top_open(),
            landscape.bottom_open(),
            landscape.grid_vehicle_byte(),
        );
        let make_graphics = || {
            let mut graphics = test_graphics(
                SCREEN_WIDTH,
                SCREEN_HEIGHT,
                WORLD_HEIGHT as i32,
                "parallel landscape rows",
            );
            graphics.set_material_textures(Arc::clone(&textures));
            graphics.set_material_render_info(Arc::clone(&materials));
            graphics.set_liquid_animation(Some(ImageData::new(
                3,
                2,
                vec![
                    255, 128, 64, 255, 32, 192, 96, 255, 144, 16, 224, 255, 48, 240, 112, 255, 208,
                    80, 160, 255, 96, 176, 16, 255,
                ],
            )));
            graphics.active_fog_map = Some(Arc::clone(&fog));
            graphics.viewport_x = 5.25;
            graphics.viewport_y = 4.5;
            graphics.viewport_zoom = 1.25;
            for (index, pixel) in graphics
                .surface_mut()
                .pixels_mut()
                .chunks_exact_mut(4)
                .enumerate()
            {
                let x = (index as u32 % SCREEN_WIDTH) as u8;
                let y = (index as u32 / SCREEN_WIDTH) as u8;
                pixel.copy_from_slice(&[
                    13u8.wrapping_add(x),
                    29u8.wrapping_add(y),
                    47u8.wrapping_add(x ^ y),
                    255,
                ]);
            }
            graphics
                .surface_mut()
                .set_clip(SurfaceRect::new(7, 5, 83, 69));
            let mut cache = LandscapeRenderCache::new(
                grid.clone(),
                WORLD_WIDTH,
                WORLD_HEIGHT,
                false,
                border_state,
            );
            cache.pixels = Arc::from(cache_pixels.clone().into_boxed_slice());
            graphics.landscape_cache = Some(cache);
            graphics
        };

        let gamma = clonk_graphics::GammaRamp::from_control_points([0x000000, 0x646464, 0xc8c8c8]);
        let mut scalar = make_graphics();
        reset_landscape_destination_samples();
        assert!(scalar.draw_ground_textured_with_parallel_rows(
            Some(&landscape),
            Some(&gamma),
            false,
        ));
        let scalar_destination_samples = landscape_destination_samples();

        let mut parallel = make_graphics();
        reset_landscape_destination_samples();
        assert!(parallel.draw_ground_textured_with_parallel_rows(
            Some(&landscape),
            Some(&gamma),
            true,
        ));
        let parallel_destination_samples = landscape_destination_samples();

        assert_eq!(parallel.surface().pixels(), scalar.surface().pixels());
        assert_eq!(
            parallel
                .landscape_cache
                .as_ref()
                .expect("parallel cache")
                .pixels
                .as_ref(),
            scalar
                .landscape_cache
                .as_ref()
                .expect("scalar cache")
                .pixels
                .as_ref(),
        );
        assert_eq!(
            parallel.liquid_animation_cycle.values.map(f32::to_bits),
            scalar.liquid_animation_cycle.values.map(f32::to_bits),
        );
        assert!(scalar_destination_samples > 0);
        assert_eq!(parallel_destination_samples, scalar_destination_samples);
    }

    #[test]
    fn parallel_gpu_landscape_cache_recomposition_matches_scalar_bytes() {
        const WIDTH: u32 = 129;
        const HEIGHT: u32 = 129;
        let base_grid = PixelGrid::new(
            WIDTH,
            HEIGHT,
            vec![1; (WIDTH * HEIGHT) as usize],
            vec![0, 25],
            vec![None, Some("Water".to_string())],
            vec![None, Some("Rough".to_string())],
        );
        let mut changed_grid = base_grid.clone();
        for y in 0..HEIGHT as i32 {
            for x in 0..WIDTH as i32 {
                if (x + y) % 3 == 0 {
                    changed_grid.write_byte(x, y, 0);
                }
            }
        }
        let mut landscape = Landscape::flat(WIDTH, HEIGHT as i32);
        landscape.set_pixel_grid(changed_grid);
        landscape.set_shade_materials(true);
        let textures = Arc::new(HashMap::from([(
            "rough".to_string(),
            MaterialTextureSurface::surface32(ImageData::new(
                2,
                2,
                vec![
                    32, 64, 96, 255, 48, 80, 112, 255, 64, 96, 128, 255, 80, 112, 144, 255,
                ],
            )),
        )]));
        let materials = Arc::new(HashMap::from([(
            "water".to_string(),
            MaterialRenderInfo::new(
                [96, 128, 160, 96, 128, 160, 96, 128, 160],
                [0; 6],
                Some("Rough".to_string()),
                0,
                25,
            ),
        )]));
        let border_state = (
            landscape.left_open(),
            landscape.right_open(),
            landscape.top_open(),
            landscape.bottom_open(),
            landscape.grid_vehicle_byte(),
        );
        let make_graphics = || {
            let mut graphics = test_graphics(
                WIDTH,
                HEIGHT,
                HEIGHT as i32,
                "parallel retained landscape cache",
            );
            graphics.set_material_texture_surfaces(Arc::clone(&textures));
            graphics.set_material_render_info(Arc::clone(&materials));
            graphics.landscape_cache = Some(LandscapeRenderCache::new(
                base_grid.clone(),
                WIDTH,
                HEIGHT,
                true,
                border_state,
            ));
            graphics.begin_gpu_scene_capture();
            graphics
        };

        let mut scalar = make_graphics();
        assert!(scalar.draw_ground_textured_with_parallel_rows(Some(&landscape), None, false,));
        let mut parallel = make_graphics();
        assert!(parallel.draw_ground_textured_with_parallel_rows(Some(&landscape), None, true,));

        let scalar = scalar.landscape_cache.as_ref().expect("scalar cache");
        let parallel = parallel.landscape_cache.as_ref().expect("parallel cache");
        assert_eq!(parallel.pixels, scalar.pixels);
        assert_eq!(parallel.liquid_mask, scalar.liquid_mask);
        assert_eq!(parallel.gpu_dirty, scalar.gpu_dirty);
    }

    #[test]
    fn textured_acid_liquid_keeps_its_material_color() {
        // C++ bakes the material's Color and both texture patterns into
        // Surface32 (C4Landscape.cpp:2619-2633). Its liquid pass supplies an
        // alpha-only animation mask (:2599-2616) to BlitLandscape (:261-270);
        // it never replaces Acid's RGB with a generic water color.
        let mut landscape: Landscape = serde_json::from_value(serde_json::json!({
            "width": 1,
            "surface": [1],
            "world_height": 1,
            "shade_materials": false,
            "pixels": {
                "width": 1,
                "height": 1,
                "bytes": "01",
                "texture_names": [null, "Smooth"],
                "densities": [0, 25],
                "material_names": [null, "Acid"]
            }
        }))
        .expect("pixel landscape");
        landscape.set_liquid_column(
            0,
            vec![LiquidSegment::with_material(0, 0, MaterialId::new(1))],
        );

        // Neutral 128-valued patterns preserve Acid's (0,190,0) RGB under
        // CPattern's ModulateClrA + LightenClr composition.
        let textures = HashMap::from([(
            "liquid".to_string(),
            ImageData::new(1, 1, vec![128, 128, 128, 255]),
        )]);
        let materials = HashMap::from([(
            "acid".to_string(),
            MaterialRenderInfo::new(
                [0, 190, 0, 0, 200, 0, 0, 210, 0],
                [0; 6],
                Some("Liquid".to_string()),
                0,
                25,
            ),
        )]);

        let mut snapshot = make_snapshot();
        snapshot.landscape = Some(landscape);
        let mut graphics = test_graphics(1, 1, 1, "Acid color");
        graphics.set_material_textures(Arc::new(textures));
        graphics.set_material_render_info(Arc::new(materials));

        let viewports = vec![ViewportInput::new(
            0,
            Vector2::ZERO,
            1.0,
            &snapshot.objects[0],
        )];
        graphics.render_frame(&snapshot, &viewports);

        assert_eq!(
            graphics.surface().get_pixel(0, 0),
            Some(standard_gamma_color(Color::opaque(0, 190, 0))),
            "the liquid animation path must preserve Acid's material RGB"
        );
    }

    #[test]
    fn material_patterns_modulate_texmap_then_overlay_at_overlay_zoom() {
        // C4Landscape::GetClrByTex applies the texmap pattern and then the
        // material pattern (C4Landscape.cpp:2619-2633). CPattern samples the
        // material overlay at zoom two and performs ModulateClrA + LightenClr
        // (C4Material.cpp:374-377; StdDDraw2.cpp:187-207).
        let material = MaterialRenderInfo::new(
            [64, 96, 128, 0, 0, 0, 0, 0, 0],
            [10, 0, 0, 0, 0, 0],
            Some("Smooth".to_string()),
            0,
            50,
        );
        let rough = ImageData::new(
            4,
            1,
            vec![1, 1, 1, 255, 2, 2, 2, 255, 128, 64, 255, 235, 3, 3, 3, 255],
        );
        let smooth = ImageData::new(2, 1, vec![4, 4, 4, 255, 64, 128, 32, 245]);

        assert_eq!(
            compose_material_pixel(&material, 1, 2, 0, &rough, Some(&smooth)),
            Color::new(32, 48, 62, 215),
        );
    }

    #[test]
    fn material_ift_bit_selects_the_background_alpha_triplet() {
        // Mat2Pal selects Alpha[0] for a foreground texmap byte and Alpha[3]
        // for the same byte plus IFT (C4Landscape.cpp:2828-2845).
        let material = MaterialRenderInfo::new(
            [100, 110, 120, 0, 0, 0, 0, 0, 0],
            [10, 0, 0, 70, 0, 0],
            None,
            0,
            50,
        );
        let texture = ImageData::new(1, 1, vec![255, 255, 255, 255]);

        let foreground = compose_material_pixel(&material, 1, 0, 0, &texture, None);
        let background = compose_material_pixel(&material, 1 | 0x80, 0, 0, &texture, None);

        assert_eq!(foreground.a, 245);
        assert_eq!(background.a, 185);
        assert_eq!(foreground.r, background.r);
        assert_eq!(foreground.g, background.g);
        assert_eq!(foreground.b, background.b);
    }

    #[test]
    fn material_overlay_flags_control_primary_and_overlay_sampling() {
        // C4TexMapEntry::Init uses HugeZoom=4 for the primary pattern;
        // C4Material::CrossMapMaterials forces the secondary overlay to zoom
        // two unless Exact selects one (C4Texture.cpp:91-102;
        // C4Material.cpp:374-377).
        let primary = ImageData::new(
            4,
            1,
            vec![
                32, 32, 32, 255, 16, 16, 16, 255, 8, 8, 8, 255, 64, 64, 64, 255,
            ],
        );
        let white = ImageData::new(1, 1, vec![255, 255, 255, 255]);
        let overlay = ImageData::new(
            4,
            1,
            vec![
                8, 8, 8, 255, 64, 64, 64, 255, 32, 32, 32, 255, 16, 16, 16, 255,
            ],
        );
        let default = MaterialRenderInfo::new([128; 9], [0; 6], None, 0, 50);
        let huge = MaterialRenderInfo::new([128; 9], [0; 6], None, MATERIAL_OVERLAY_HUGE_ZOOM, 50);
        let exact = MaterialRenderInfo::new([128; 9], [0; 6], None, MATERIAL_OVERLAY_EXACT, 50);

        assert_eq!(
            compose_material_pixel(&default, 1, 3, 0, &primary, None).r,
            64,
        );
        assert_eq!(compose_material_pixel(&huge, 1, 3, 0, &primary, None).r, 32,);
        assert_eq!(
            compose_material_pixel(&default, 1, 2, 0, &white, Some(&overlay)).r,
            126,
        );
        assert_eq!(
            compose_material_pixel(&exact, 1, 2, 0, &white, Some(&overlay)).r,
            62,
        );
    }

    #[test]
    fn monochrome_material_patterns_use_the_blue_texture_channel() {
        // CPattern::PatternClr passes the low byte of the BGRA dword to
        // ModulateClrMonoA, i.e. the source texture's blue channel
        // (StdDDraw2.cpp:195-205; StdPNGLibpng.cpp:200-223).
        let mut pixel = MaterialPixel {
            red: 64,
            green: 96,
            blue: 128,
            transparency: 0,
        };
        let texture = ImageData::new(1, 1, vec![10, 20, 200, 255]);

        apply_material_pattern(&mut pixel, &texture, 0, 0, 0, true);

        assert_eq!([pixel.red, pixel.green, pixel.blue], [100, 150, 200]);
    }

    #[test]
    fn indexed_material_patterns_select_the_native_color_triplet() {
        let material = MaterialRenderInfo::new(
            [10, 20, 30, 40, 50, 60, 70, 80, 90],
            [1, 2, 3, 4, 5, 6],
            None,
            MATERIAL_OVERLAY_MONOCHROME,
            50,
        );
        let surface = MaterialTextureSurface::surface8(2, 1, vec![2, 1]);

        assert_eq!(
            compose_material_surface_pixel(&material, 1, 0, 0, (&surface).into(), None),
            Color::new(70, 80, 90, 252),
            "Surface8 shift 2 selects the third RGB/alpha triplet"
        );
        assert_eq!(
            compose_material_surface_pixel(&material, 0x81, 1, 0, (&surface).into(), None),
            Color::new(40, 50, 60, 250),
            "IFT pixels select the second alpha triplet; monochrome is ignored for Surface8"
        );
        assert_eq!(
            compose_material_surface_pixel(&material, 0x10, 0, 0, (&surface).into(), None),
            Color::new(70, 80, 90, 249),
            "native alpha selection tests the whole high nibble, not only IFT"
        );
    }

    #[test]
    fn textured_material_alpha_blends_over_the_rendered_sky() {
        // C4Landscape::GetClrByTex stores the material transparency in
        // Surface32 (C4Landscape.cpp:2619-2633), and BlitLandscape composites
        // that surface over the already-rendered viewport (StdGL.cpp:578-580,
        // 640-664). The Rust cache must not force the material opaque.
        let landscape: Landscape = serde_json::from_value(serde_json::json!({
            "width": 1,
            "surface": [1],
            "world_height": 1,
            "shade_materials": false,
            "pixels": {
                "width": 1,
                "height": 1,
                "bytes": "01",
                "texture_names": [null, "Rough"],
                "densities": [0, 50],
                "material_names": [null, "Earth"]
            }
        }))
        .expect("pixel landscape");
        let textures = HashMap::from([
            (
                "rough".to_string(),
                ImageData::new(1, 1, vec![255, 255, 255, 255]),
            ),
            (
                "smooth".to_string(),
                ImageData::new(1, 1, vec![255, 255, 255, 255]),
            ),
        ]);
        let materials = HashMap::from([(
            "earth".to_string(),
            MaterialRenderInfo::new(
                [64, 0, 0, 0, 0, 0, 0, 0, 0],
                [127, 0, 0, 0, 0, 0],
                Some("Smooth".to_string()),
                0,
                50,
            ),
        )]);
        let mut graphics = test_graphics(1, 1, 1, "Material alpha");
        graphics
            .surface_mut()
            .set_pixel(0, 0, Color::opaque(10, 20, 30))
            .expect("sky pixel");
        graphics.set_material_textures(Arc::new(textures));
        graphics.set_material_render_info(Arc::new(materials));

        assert!(graphics.draw_ground_textured(Some(&landscape), None));
        assert_eq!(
            graphics.surface().get_pixel(0, 0),
            Some(Color::opaque(130, 9, 14)),
        );
    }

    #[test]
    fn textured_landscape_gamma_samples_r16_before_alpha_blending() {
        // BlitLandscape applies the per-channel R16 gamma lookup to its source
        // fragment before fixed-function alpha blending (StdGL.cpp:578-618,
        // 1139-1148,1246-1263).
        let landscape: Landscape = serde_json::from_value(serde_json::json!({
            "width": 1,
            "surface": [1],
            "world_height": 1,
            "shade_materials": false,
            "pixels": {
                "width": 1,
                "height": 1,
                "bytes": "01",
                "texture_names": [null, "Rough"],
                "densities": [0, 50],
                "material_names": [null, "Earth"]
            }
        }))
        .expect("pixel landscape");
        let cached_grid = landscape.pixel_grid().expect("pixel grid").clone();
        let mut graphics = test_graphics(1, 1, 1, "Gamma Material");
        graphics.set_material_textures(Arc::new(HashMap::from([(
            "rough".to_string(),
            ImageData::new(1, 1, vec![255, 255, 255, 255]),
        )])));
        graphics.set_material_render_info(Arc::new(HashMap::from([(
            "earth".to_string(),
            MaterialRenderInfo::new([255; 9], [0; 6], None, 0, 50),
        )])));
        // Presentation is under test, not cache construction. Keeping the raw
        // cached source unencoded also pins that later gamma changes do not
        // require rebuilding the landscape cache.
        let mut cache =
            LandscapeRenderCache::new(cached_grid, 1, 1, false, (0, 0, true, false, None));
        cache.pixels = Arc::from(vec![64, 128, 192, 128].into_boxed_slice());
        graphics.landscape_cache = Some(cache);
        graphics
            .surface_mut()
            .set_pixel(0, 0, Color::opaque(200, 200, 200))
            .expect("sky pixel");
        let gamma = clonk_graphics::GammaRamp::from_control_points([0x000000, 0x646464, 0xc8c8c8]);

        assert!(graphics.draw_ground_textured(Some(&landscape), Some(&gamma)));

        assert_eq!(
            graphics.surface().get_pixel(0, 0),
            Some(Color::new(125, 150, 175, 255))
        );
    }

    #[test]
    fn landscape_modulation_uses_the_blit_pipeline_for_textured_and_fallback_landscapes() {
        const MODULATION: u32 = 0x4080_40ff;
        let gamma = clonk_graphics::GammaRamp::from_control_points([0x000000, 0x646464, 0xc8c8c8]);
        let background = Color::opaque(20, 40, 60);
        let blit = |modulation| SpriteBlitState {
            mode: 0,
            modulation: (modulation != 0).then_some(modulation),
            fog_modulation: None,
            renderer_config: AdvancedRendererConfig::DEFAULT,
        };
        let with_modulation = |landscape: Landscape, modulation: u32| {
            let mut value = serde_json::to_value(landscape).expect("landscape serializes");
            value["modulation"] = serde_json::json!(modulation);
            serde_json::from_value::<Landscape>(value).expect("modulated landscape decodes")
        };

        let render_textured = |modulation| {
            let landscape: Landscape = serde_json::from_value(serde_json::json!({
                "width": 1,
                "surface": [1],
                "modulation": modulation,
                "world_height": 1,
                "shade_materials": false,
                "pixels": {
                    "width": 1,
                    "height": 1,
                    "bytes": "01",
                    "texture_names": [null, "Rough"],
                    "densities": [0, 50],
                    "material_names": [null, "Earth"]
                }
            }))
            .expect("pixel landscape");
            let mut graphics = test_graphics(1, 1, 1, "modulated landscape");
            graphics.set_material_textures(Arc::new(HashMap::from([(
                "rough".to_string(),
                ImageData::new(1, 1, vec![255, 255, 255, 255]),
            )])));
            graphics.set_material_render_info(Arc::new(HashMap::from([(
                "earth".to_string(),
                MaterialRenderInfo::new([255; 9], [0; 6], None, 0, 50),
            )])));
            let mut cache = LandscapeRenderCache::new(
                landscape.pixel_grid().expect("pixel grid").clone(),
                1,
                1,
                false,
                (0, 0, true, false, None),
            );
            cache.pixels = Arc::from(vec![96, 144, 208, 255].into_boxed_slice());
            graphics.landscape_cache = Some(cache);
            graphics
                .surface_mut()
                .set_pixel(0, 0, background)
                .expect("background pixel");
            assert!(graphics.draw_ground_textured(Some(&landscape), Some(&gamma)));
            graphics.surface().get_pixel(0, 0).expect("drawn pixel")
        };

        for modulation in [0, MODULATION] {
            let source = Color::new(96, 144, 208, 255);
            let expected = composite_sprite_fragment(
                prepare_sprite_fragment(source, None, None, blit(modulation)),
                background,
                blit(modulation),
                Some(&gamma),
            );
            assert_eq!(render_textured(modulation), expected);
        }
        assert_ne!(render_textured(0), render_textured(MODULATION));

        let render_fallback = |modulation| {
            let mut landscape = Landscape::flat(1, 1);
            landscape.set_liquid_column(0, vec![LiquidSegment::new(0, 0)]);
            let landscape = with_modulation(landscape, modulation);
            let mut graphics = test_graphics(1, 2, 2, "modulated fallback landscape");
            graphics.surface_mut().fill(background);
            assert!(!graphics.draw_ground(0, Some(&landscape), 1.0, Some(&gamma)));
            graphics.draw_liquids(0, Some(&landscape), 1.0, Some(&gamma));
            (
                graphics.surface().get_pixel(0, 0).expect("liquid pixel"),
                graphics.surface().get_pixel(0, 1).expect("ground pixel"),
            )
        };

        for modulation in [0, MODULATION] {
            let liquid = GraphicsSystem::apply_lighting(
                GraphicsSystem::liquid_color_for_temperature(0),
                1.0,
            );
            let ground = GraphicsSystem::apply_lighting(
                GraphicsSystem::ground_color_for_temperature(0),
                1.0,
            );
            let expected_liquid = composite_sprite_fragment(
                prepare_sprite_fragment(liquid, None, None, blit(modulation)),
                background,
                blit(modulation),
                Some(&gamma),
            );
            let expected_ground = composite_sprite_fragment(
                prepare_sprite_fragment(ground, None, None, blit(modulation)),
                background,
                blit(modulation),
                Some(&gamma),
            );
            assert_eq!(
                render_fallback(modulation),
                (expected_liquid, expected_ground)
            );
        }
        assert_ne!(render_fallback(0), render_fallback(MODULATION));
    }

    #[test]
    fn landscape_placement_shading_matches_apply_lighting() {
        fn render_rows(rows: &[u8], shade_materials: bool) -> Vec<Color> {
            const WIDTH: u32 = 3;
            let height = rows.len() as u32;
            let bytes = rows
                .iter()
                .flat_map(|&byte| [byte; WIDTH as usize])
                .collect();
            let mut landscape = Landscape::flat(WIDTH, height as i32);
            landscape.set_pixel_grid(PixelGrid::new(
                WIDTH,
                height,
                bytes,
                vec![0, 50, 50, 50],
                vec![
                    None,
                    Some("High".to_string()),
                    Some("Low".to_string()),
                    Some("Threshold".to_string()),
                ],
                vec![
                    None,
                    Some("Neutral".to_string()),
                    Some("Neutral".to_string()),
                    Some("Neutral".to_string()),
                ],
            ));
            landscape.set_shade_materials(shade_materials);

            let mut graphics = test_graphics(WIDTH, height, height as i32, "placement shading");
            graphics.set_material_textures(Arc::new(HashMap::from([(
                "neutral".to_string(),
                ImageData::new(1, 1, vec![128, 128, 128, 255]),
            )])));
            let base = [100, 100, 100, 0, 0, 0, 0, 0, 0];
            graphics.set_material_render_info(Arc::new(HashMap::from([
                (
                    "high".to_string(),
                    MaterialRenderInfo::new(base, [0; 6], None, 0, 50).with_placement(70),
                ),
                (
                    "low".to_string(),
                    MaterialRenderInfo::new(base, [0; 6], None, 0, 50).with_placement(10),
                ),
                (
                    "threshold".to_string(),
                    MaterialRenderInfo::new(base, [0; 6], None, 0, 50).with_placement(30),
                ),
            ])));
            assert!(graphics.draw_ground_textured(Some(&landscape), None));
            let cache = graphics.landscape_cache.as_ref().expect("cache built");
            (0..height as usize)
                .map(|y| {
                    let offset = (y * WIDTH as usize + 1) * 4;
                    Color::new(
                        cache.pixels[offset],
                        cache.pixels[offset + 1],
                        cache.pixels[offset + 2],
                        cache.pixels[offset + 3],
                    )
                })
                .collect()
        }

        let mut sky_to_high = vec![0; 8];
        sky_to_high.extend(vec![1; 22]);
        let shaded = render_rows(&sky_to_high, true);
        assert_eq!(shaded[7], Color::new(0, 0, 0, 0));
        assert_eq!(
            shaded[8],
            Color::opaque(130, 130, 130),
            "the first dense row lightens by the capped above-placement delta"
        );
        assert_eq!(shaded[16], Color::opaque(100, 100, 100));
        assert_eq!(
            render_rows(&sky_to_high, false)[8],
            Color::opaque(100, 100, 100),
            "ShadeMaterials=0 retains the unshaded material pattern"
        );

        let mut high_to_sky = vec![1; 9];
        high_to_sky.extend(vec![0; 21]);
        assert_eq!(
            render_rows(&high_to_sky, true)[8],
            Color::opaque(70, 70, 70),
            "a dense row over lower placement darkens through BelowDensity"
        );

        let mut high_to_low = vec![1; 8];
        high_to_low.extend(vec![2; 22]);
        assert_eq!(
            render_rows(&high_to_low, true)[8],
            Color::opaque(70, 70, 70),
            "placement below 30 darkens against denser rows above"
        );
        let mut high_to_threshold = vec![1; 8];
        high_to_threshold.extend(vec![3; 22]);
        assert_eq!(
            render_rows(&high_to_threshold, true)[8],
            Color::opaque(100, 100, 100),
            "the asymmetric above-darkening arm excludes own placement 30"
        );
    }

    /// `draw_definition_particles` filters the whole particle slice on every
    /// call and an object pass calls it up to twice per object, so a frame's
    /// particle walk was O(objects * particles). Grouping the slice by layer
    /// once per object list makes the pass O(particles).
    #[test]
    fn object_pass_examines_the_particle_slice_once() {
        const OBJECTS: usize = 40;
        const PARTICLES: usize = 200;

        let template = make_snapshot().objects.remove(0);
        let objects = (0..OBJECTS)
            .map(|index| {
                let mut object = template.clone();
                object.id = ObjectId::new(index as u64 + 1);
                object.position = Vector2::new(index as i32 * 2, 8);
                object
            })
            .collect::<Vec<_>>();
        // Every particle sits on the global layer, so no object draws one and
        // the whole cost of the old walk was the membership test itself.
        let particles = (0..PARTICLES)
            .map(|index| ParticleSnapshot {
                definition_id: "Smoke".to_string(),
                position: FloatVector2::new(index as f32, 4.0),
                velocity: FloatVector2::new(0.0, 0.0),
                life: 10,
                parameter_a: 2.0,
                parameter_b: 0x00ff_ffff,
                layer: ParticleLayer::Global,
                pxs_fixed: None,
                pxs_slot: None,
            })
            .collect::<Vec<_>>();
        let mut graphics = GraphicsSystem::new(
            160,
            120,
            120,
            "particle layer index",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );

        let pass = |graphics: &mut GraphicsSystem| {
            graphics.draw_objects_at_frame(
                0,
                &objects,
                &[],
                &HashMap::new(),
                &particles,
                &[],
                0,
                1.0,
                &HashMap::new(),
                ObjectRenderPass::Normal,
                None,
            );
        };
        pass(&mut graphics);

        reset_particle_layer_scans();
        let start = std::time::Instant::now();
        const PASSES: u32 = 100;
        for _ in 0..PASSES {
            pass(&mut graphics);
        }
        let elapsed = start.elapsed();
        let scans = particle_layer_scans();
        println!(
            "{OBJECTS} objects x {PARTICLES} particles: {:.3} us/pass, {} particle \
             examinations/pass",
            elapsed.as_secs_f64() * 1e6 / f64::from(PASSES),
            scans / PASSES as usize,
        );
        assert_eq!(
            scans,
            PARTICLES * PASSES as usize,
            "an object pass examined the particle slice more than once"
        );
    }

    #[test]
    fn normal_object_visibility_is_evaluated_only_in_the_normal_pass() {
        const OBJECTS: usize = 1_000;

        let template = make_snapshot().objects.remove(0);
        let objects = (0..OBJECTS)
            .map(|index| {
                let mut object = template.clone();
                object.id = ObjectId::new(index as u64 + 1);
                object
            })
            .collect::<Vec<_>>();
        let mut graphics = test_graphics(1, 1, 1, "object visibility pass filtering");

        reset_object_visibility_evaluations();
        for pass in [
            ObjectRenderPass::Background,
            ObjectRenderPass::Normal,
            ObjectRenderPass::ForegroundNonParallax,
            ObjectRenderPass::ForegroundParallax,
        ] {
            graphics.draw_objects_at_frame(
                0,
                &objects,
                &[],
                &HashMap::new(),
                &[],
                &[],
                OWNER_NONE,
                1.0,
                &HashMap::new(),
                pass,
                None,
            );
        }

        assert_eq!(object_visibility_evaluations(), OBJECTS);
    }

    /// `C4Viewport::Draw` walks each category list at its painter-order site,
    /// while the retained presentation plan replaces those four support-list
    /// rebuilds with one canonical walk (`src/C4Viewport.cpp:1051-1088`).
    #[test]
    fn viewport_prepares_object_phase_partitions_and_visibility_once() {
        let mut snapshot = make_snapshot();
        let template = snapshot.objects.remove(0);
        snapshot.objects = [
            0,
            CATEGORY_BACKGROUND_FLAG,
            CATEGORY_FOREGROUND_FLAG,
            CATEGORY_FOREGROUND_FLAG | CATEGORY_PARALLAX_FLAG,
            CATEGORY_BACKGROUND_FLAG | CATEGORY_FOREGROUND_FLAG,
        ]
        .into_iter()
        .enumerate()
        .map(|(index, category_flags)| {
            let mut object = template.clone();
            object.id = ObjectId::new(index as u64 + 1);
            object.category = (object.category
                & !(CATEGORY_BACKGROUND_FLAG | CATEGORY_FOREGROUND_FLAG | CATEGORY_PARALLAX_FLAG))
                | category_flags;
            object
        })
        .collect();
        snapshot.render_order = snapshot.objects.iter().map(|object| object.id).collect();
        let mut graphics = test_graphics(160, 120, 120, "retained object phase plan");

        reset_object_render_plan_evaluations();
        reset_object_visibility_evaluations();
        graphics.render_frame(
            &snapshot,
            &[ViewportInput::ownerless(Vector2::new(80, 60), 1.0)],
        );

        assert_eq!(object_render_plan_evaluations(), snapshot.objects.len());
        assert_eq!(object_visibility_evaluations(), snapshot.objects.len());
    }

    #[test]
    fn thousand_st5b_faces_capture_as_one_ordered_compact_run() {
        const OBJECTS: usize = 1_000;
        let mut template = make_snapshot().objects.remove(0);
        template.definition_id = "ST5B".to_string();
        template.crew_member = false;
        template.action = clonk_engine::ActionState::new("Walk");
        let objects = (0..OBJECTS)
            .map(|index| {
                let mut object = template.clone();
                object.id = ObjectId::new(index as u64 + 1);
                object.position =
                    Vector2::new(20 + (index % 40) as i32 * 15, 20 + (index / 40) as i32 * 15);
                object.action.phase = (index % 20) as i32;
                object.direction = if index % 2 == 0 {
                    Direction::Left
                } else {
                    Direction::Right
                };
                object.draw_transform =
                    (index % 2 != 0).then(|| DrawTransform::from_components(-1.0, 1.0, 0.0, 0.0));
                object.color_modulation = 0x0040_0000 | (index as u32 + 1);
                object
            })
            .collect::<Vec<_>>();
        let walk = DefinitionActionGraphics {
            facet: Some(clonk_engine::DefinitionActionFacet {
                x: 0,
                y: 0,
                width: 15,
                height: 15,
                target_x: 0,
                target_y: 0,
            }),
            directions: 2,
            flip_dir: Some(1),
            length: Some(20),
            ..DefinitionActionGraphics::default()
        };
        let sprite = DefinitionSprite {
            image: ImageData::new(300, 110, vec![255; 300 * 110 * 4]),
            actions: HashMap::from([("Walk".to_string(), walk)]),
            color_mask: None,
            graphics_scale: 1.0,
            shape: Some(DefinitionRect::new(-7, -7, 15, 15)),
            fire_top: 0,
            rotateable: 0,
            line: 0,
            stretch_growth: true,
            top_face: None,
            picture: None,
        };
        let mut graphics = GraphicsSystem::new(
            640,
            420,
            420,
            "ST5B compact capture",
            test_font(),
            Arc::new(HashMap::from([("ST5B".to_string(), sprite)])),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        let gamma = clonk_graphics::GammaRamp::standard();
        graphics.begin_gpu_scene_capture();

        graphics.draw_objects(
            &objects,
            &[],
            &HashMap::new(),
            &[],
            OWNER_NONE,
            1.0,
            &HashMap::new(),
            ObjectRenderPass::Normal,
            Some(&gamma),
        );

        let scene = graphics
            .finish_gpu_scene_capture(&gamma)
            .expect("GPU capture was started");
        let [GpuCommand::ObjectBatch { sprites, .. }] = scene.commands.as_slice() else {
            panic!("representable ST5B faces entered the generic capture path");
        };
        assert_eq!(sprites.len(), OBJECTS);
        assert_eq!(
            sprites
                .iter()
                .map(|sprite| sprite.modulation[0])
                .collect::<std::collections::HashSet<_>>()
                .len(),
            OBJECTS,
            "each benchmark object needs a distinct color modulation"
        );
        assert_eq!(sprites[0].modulation[0], 0x0040_0001);
        assert_eq!(sprites[1].modulation[0], 0x0040_0002);
        assert_eq!(sprites[0].sampler(), GpuSampler::Nearest);
        assert_eq!(sprites[1].sampler(), GpuSampler::Linear);
    }

    #[test]
    fn compact_object_capture_keeps_every_base_before_every_top_face() {
        // C4ObjectList::Draw completes its base loop before starting the
        // TopFace loop (src/C4ObjectList.cpp:390-396).
        let mut template = make_snapshot().objects.remove(0);
        template.definition_id = "Layered".to_owned();
        template.crew_member = false;
        template.ocf = 0;
        let objects = (0..2)
            .map(|index| {
                let mut object = template.clone();
                object.id = ObjectId::new(index + 1);
                object.position = Vector2::new(16 + index as i32 * 4, 16);
                object.color_modulation = 0x0010_1010 * (index as u32 + 1);
                object
            })
            .collect::<Vec<_>>();
        let sprite = DefinitionSprite {
            image: ImageData::new(30, 15, vec![255; 30 * 15 * 4]),
            actions: HashMap::new(),
            color_mask: None,
            graphics_scale: 1.0,
            shape: Some(DefinitionRect::new(-7, -7, 15, 15)),
            fire_top: 0,
            rotateable: 0,
            line: 0,
            stretch_growth: false,
            top_face: Some(DefinitionTargetRect::new(15, 0, 15, 15, 0, 0)),
            picture: None,
        };
        let mut graphics = GraphicsSystem::new(
            48,
            32,
            32,
            "compact base and top ordering",
            test_font(),
            Arc::new(HashMap::from([("Layered".to_owned(), sprite)])),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        let gamma = clonk_graphics::GammaRamp::standard();
        graphics.begin_gpu_scene_capture();

        graphics.draw_objects(
            &objects,
            &[],
            &HashMap::new(),
            &[],
            OWNER_NONE,
            1.0,
            &HashMap::new(),
            ObjectRenderPass::Normal,
            Some(&gamma),
        );

        let scene = graphics
            .finish_gpu_scene_capture(&gamma)
            .expect("GPU capture was started");
        let [GpuCommand::ObjectBatch { sprites, .. }] = scene.commands.as_slice() else {
            panic!("base and TopFace sprites split out of their ordered resource run");
        };
        assert_eq!(sprites.len(), 4);
        assert!(sprites[..2]
            .iter()
            .all(|sprite| (sprite.uv[0] - 0.0).abs() < f32::EPSILON));
        assert!(sprites[2..]
            .iter()
            .all(|sprite| (sprite.uv[0] - 0.5).abs() < f32::EPSILON));
        assert_eq!(
            sprites
                .iter()
                .map(|sprite| sprite.modulation[0])
                .collect::<Vec<_>>(),
            vec![0x0010_1010, 0x0020_2020, 0x0010_1010, 0x0020_2020]
        );
    }

    #[test]
    fn construction_sign_remains_a_generic_ordered_fallback() {
        // The construction facet is a global-resource draw at the start of
        // C4Object::DrawTopFace (src/C4Object.cpp:2617-2638).
        let mut object = make_snapshot().objects.remove(0);
        object.definition_id = "Building".to_owned();
        object.crew_member = false;
        object.position = Vector2::new(16, 16);
        object.construction = FULL_CON / 2;
        object.ocf = clonk_engine::ocf::CONSTRUCT;
        let sprite = DefinitionSprite {
            image: ImageData::new(15, 15, vec![255; 15 * 15 * 4]),
            actions: HashMap::new(),
            color_mask: None,
            graphics_scale: 1.0,
            shape: Some(DefinitionRect::new(-7, -7, 15, 15)),
            fire_top: 0,
            rotateable: 0,
            line: 0,
            stretch_growth: false,
            top_face: None,
            picture: None,
        };
        let construction = ImageData::new(16, 16, vec![255; 16 * 16 * 4]);
        let mut graphics = GraphicsSystem::new(
            32,
            32,
            32,
            "construction fallback",
            test_font(),
            Arc::new(HashMap::from([("Building".to_owned(), sprite)])),
            empty_cursor_atlas(),
            Arc::new(HudGraphics {
                construction: Some(construction),
                ..HudGraphics::default()
            }),
        );
        let gamma = clonk_graphics::GammaRamp::standard();
        graphics.begin_gpu_scene_capture();

        graphics.draw_objects(
            &[object],
            &[],
            &HashMap::new(),
            &[],
            OWNER_NONE,
            1.0,
            &HashMap::new(),
            ObjectRenderPass::Normal,
            Some(&gamma),
        );

        let scene = graphics
            .finish_gpu_scene_capture(&gamma)
            .expect("GPU capture was started");
        assert!(matches!(
            scene.commands.first(),
            Some(GpuCommand::ObjectBatch { .. })
        ));
        assert!(matches!(
            scene.commands.get(1),
            Some(GpuCommand::Quad { .. })
        ));
        assert_eq!(scene.commands.len(), 2);
    }

    #[test]
    fn normal_object_draw_borrows_default_sprite_keys() {
        let sprite = DefinitionSprite {
            image: ImageData::new(1, 1, vec![255; 4]),
            actions: HashMap::new(),
            color_mask: None,
            graphics_scale: 1.0,
            shape: Some(DefinitionRect::new(0, 0, 1, 1)),
            fire_top: 0,
            rotateable: 0,
            line: 0,
            stretch_growth: false,
            top_face: None,
            picture: None,
        };
        let snapshot = make_snapshot();
        let mut graphics = GraphicsSystem::new(
            160,
            120,
            120,
            "borrowed sprite keys",
            test_font(),
            Arc::new(HashMap::from([("TestObject".to_owned(), sprite)])),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );

        reset_default_sprite_key_allocations();
        graphics.draw_objects(
            &snapshot.objects,
            &snapshot.render_order,
            &snapshot.definition_lines,
            &snapshot.players,
            OWNER_NONE,
            1.0,
            &HashMap::new(),
            ObjectRenderPass::Normal,
            None,
        );

        assert_eq!(default_sprite_key_allocations(), 0);
    }

    #[test]
    fn object_without_top_face_or_construction_skips_top_face_draw_setup() {
        let sprite = DefinitionSprite {
            image: ImageData::new(1, 1, vec![255; 4]),
            actions: HashMap::new(),
            color_mask: None,
            graphics_scale: 1.0,
            shape: Some(DefinitionRect::new(0, 0, 1, 1)),
            fire_top: 0,
            rotateable: 0,
            line: 0,
            stretch_growth: false,
            top_face: None,
            picture: None,
        };
        let snapshot = make_snapshot();
        let mut graphics = GraphicsSystem::new(
            160,
            120,
            120,
            "absent top face",
            test_font(),
            Arc::new(HashMap::from([("TestObject".to_owned(), sprite)])),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );

        reset_top_face_draw_setups();
        graphics.draw_objects(
            &snapshot.objects,
            &snapshot.render_order,
            &snapshot.definition_lines,
            &snapshot.players,
            OWNER_NONE,
            1.0,
            &HashMap::new(),
            ObjectRenderPass::Normal,
            None,
        );

        assert_eq!(top_face_draw_setups(), 0);
    }

    #[test]
    fn objects_without_overlays_skip_recursive_ancestry_setup() {
        let sprite = DefinitionSprite {
            image: ImageData::new(1, 1, vec![255; 4]),
            actions: HashMap::new(),
            color_mask: None,
            graphics_scale: 1.0,
            shape: Some(DefinitionRect::new(0, 0, 1, 1)),
            fire_top: 0,
            rotateable: 0,
            line: 0,
            stretch_growth: false,
            top_face: None,
            picture: None,
        };
        let snapshot = make_snapshot();
        let mut graphics = GraphicsSystem::new(
            160,
            120,
            120,
            "absent overlays",
            test_font(),
            Arc::new(HashMap::from([("TestObject".to_owned(), sprite)])),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );

        reset_object_overlay_ancestry_setups();
        graphics.draw_objects(
            &snapshot.objects,
            &snapshot.render_order,
            &snapshot.definition_lines,
            &snapshot.players,
            OWNER_NONE,
            1.0,
            &HashMap::new(),
            ObjectRenderPass::Normal,
            None,
        );

        assert_eq!(object_overlay_ancestry_setups(), 0);
    }

    #[test]
    fn representable_object_output_reach_is_evaluated_once() {
        // C4Object::Draw has one output-boundary return before overlays and
        // selection graphics (src/C4Object.cpp:2266-2283).
        let sprite = DefinitionSprite {
            image: ImageData::new(15, 15, vec![255; 15 * 15 * 4]),
            actions: HashMap::new(),
            color_mask: None,
            graphics_scale: 1.0,
            shape: Some(DefinitionRect::new(-7, -7, 15, 15)),
            fire_top: 0,
            rotateable: 0,
            line: 0,
            stretch_growth: true,
            top_face: None,
            picture: None,
        };
        let mut snapshot = make_snapshot();
        snapshot.objects[0].need_energy = true;
        let mut graphics = GraphicsSystem::new(
            160,
            120,
            120,
            "single output reach",
            test_font(),
            Arc::new(HashMap::from([("TestObject".to_owned(), sprite)])),
            empty_cursor_atlas(),
            Arc::new(HudGraphics {
                energy: Some(ImageData::new(1, 1, vec![255; 4])),
                ..HudGraphics::default()
            }),
        );

        reset_object_output_reach_evaluations();
        graphics.draw_objects_at_frame(
            13,
            &snapshot.objects,
            &snapshot.render_order,
            &snapshot.definition_lines,
            &[],
            &snapshot.players,
            OWNER_NONE,
            1.0,
            &HashMap::new(),
            ObjectRenderPass::Normal,
            None,
        );

        assert_eq!(object_output_reach_evaluations(), 1);
    }

    /// The landscape cache re-anchors to the byte plane the frame presented so
    /// the engine's next write forks a distinct COW generation
    /// (clonk-engine landscape.rs:550-554). A frame that changed nothing is
    /// already anchored to that exact `Arc`, yet re-cloning still deep-copies
    /// the grid's texture names, material names, densities, materials, dirty
    /// generations and pending relights (landscape.rs:290-348).
    #[test]
    fn unchanged_landscape_reuses_its_anchored_cache_grid() {
        const WIDTH: u32 = 64;
        const HEIGHT: u32 = 64;
        let mut landscape = Landscape::flat(WIDTH, HEIGHT as i32);
        let texture_names = (0..128)
            .map(|index| (index > 0).then(|| format!("Texture{index}")))
            .collect::<Vec<_>>();
        let material_names = (0..128)
            .map(|index| (index > 0).then(|| format!("Material{index}")))
            .collect::<Vec<_>>();
        landscape.set_pixel_grid(PixelGrid::new(
            WIDTH,
            HEIGHT,
            vec![1; (WIDTH * HEIGHT) as usize],
            (0..128)
                .map(|index| if index == 0 { 0 } else { 50 })
                .collect(),
            material_names,
            texture_names,
        ));
        let mut graphics = GraphicsSystem::new(
            WIDTH,
            HEIGHT,
            HEIGHT as i32,
            "landscape cache anchor",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics.set_material_textures(Arc::new(HashMap::from([(
            "texture1".to_string(),
            ImageData::new(1, 1, vec![128, 128, 128, 255]),
        )])));
        graphics.set_material_render_info(Arc::new(HashMap::from([(
            "material1".to_string(),
            MaterialRenderInfo::new([100, 100, 100, 0, 0, 0, 0, 0, 0], [0; 6], None, 0, 50),
        )])));

        let anchor = |graphics: &GraphicsSystem| {
            let cache = graphics.landscape_cache.as_ref().expect("cache built");
            (
                cache.grid.texture_names().as_ptr(),
                cache.grid.material_names().as_ptr(),
                cache.grid.bytes().as_ptr(),
            )
        };
        assert!(graphics.draw_ground_textured(Some(&landscape), None));
        let first = anchor(&graphics);
        assert!(graphics.draw_ground_textured(Some(&landscape), None));
        let second = anchor(&graphics);
        assert_eq!(
            (first.0, first.1),
            (second.0, second.1),
            "an unchanged landscape re-cloned its cache grid"
        );
        // The whole point of the anchor: the cache still shares the presented
        // byte plane, so the engine's next write cannot mutate it in place.
        let presented = landscape.pixel_grid().expect("grid").bytes().as_ptr();
        assert_eq!(second.2, presented, "cache lost the presented byte plane");

        landscape.grid_write_byte(4, 4, 2);
        assert!(graphics.draw_ground_textured(Some(&landscape), None));
        let third = anchor(&graphics);
        assert_ne!(
            (second.0, second.1),
            (third.0, third.1),
            "a changed landscape must re-anchor its cache grid"
        );
        assert_eq!(
            third.2,
            landscape.pixel_grid().expect("grid").bytes().as_ptr(),
            "cache lost the rewritten byte plane"
        );

        let grid = landscape.pixel_grid().expect("grid");
        let start = std::time::Instant::now();
        const CLONES: u32 = 1000;
        for _ in 0..CLONES {
            std::hint::black_box(grid.clone());
        }
        println!(
            "PixelGrid::clone with 128 texture and 128 material names: {:.3} us",
            start.elapsed().as_secs_f64() * 1e6 / f64::from(CLONES)
        );
    }

    #[test]
    fn landscape_placement_shading_expands_warm_cache_relight_region() {
        const WIDTH: u32 = 25;
        const HEIGHT: u32 = 25;
        const CHANGE_X: i32 = 12;
        const CHANGE_Y: i32 = 12;
        let mut landscape = Landscape::flat(WIDTH, HEIGHT as i32);
        landscape.set_pixel_grid(PixelGrid::new(
            WIDTH,
            HEIGHT,
            vec![1; (WIDTH * HEIGHT) as usize],
            vec![0, 50, 50],
            vec![None, Some("High".to_string()), Some("Low".to_string())],
            vec![
                None,
                Some("Neutral".to_string()),
                Some("Neutral".to_string()),
            ],
        ));
        landscape.set_shade_materials(true);
        let mut graphics = test_graphics(WIDTH, HEIGHT, HEIGHT as i32, "placement shading cache");
        graphics.set_material_textures(Arc::new(HashMap::from([(
            "neutral".to_string(),
            ImageData::new(1, 1, vec![128, 128, 128, 255]),
        )])));
        let base = [100, 100, 100, 0, 0, 0, 0, 0, 0];
        graphics.set_material_render_info(Arc::new(HashMap::from([
            (
                "high".to_string(),
                MaterialRenderInfo::new(base, [0; 6], None, 0, 50).with_placement(70),
            ),
            (
                "low".to_string(),
                MaterialRenderInfo::new(base, [0; 6], None, 0, 50).with_placement(10),
            ),
        ])));

        assert!(graphics.draw_ground_textured(Some(&landscape), None));
        landscape.grid_write_byte(CHANGE_X, CHANGE_Y, 2);
        reset_material_composition_calls();
        assert!(graphics.draw_ground_textured(Some(&landscape), None));
        assert_eq!(
            material_composition_calls(),
            3 * 17,
            "Relight expands one Surface8 change by x=1 and y=8"
        );
        let cache = graphics.landscape_cache.as_ref().expect("cache patched");
        let color_at = |x: i32, y: i32| {
            let offset = (y as usize * WIDTH as usize + x as usize) * 4;
            Color::new(
                cache.pixels[offset],
                cache.pixels[offset + 1],
                cache.pixels[offset + 2],
                cache.pixels[offset + 3],
            )
        };
        assert_eq!(color_at(CHANGE_X, CHANGE_Y - 1), Color::opaque(84, 84, 84));
        assert_eq!(
            color_at(CHANGE_X, CHANGE_Y + 1),
            Color::opaque(116, 116, 116)
        );
    }

    #[test]
    fn opaque_landscape_blit_does_not_sample_the_destination() {
        // GL_SRC_ALPHA with source alpha one is algebraically a source copy;
        // C++ submits the visible Surface32 as one hardware blit instead of
        // reading the framebuffer on the CPU (StdGL.cpp:578-580,640-664).
        // Alchemy's ordinary solid materials are opaque, so the software
        // counterpart must not pay for a destination read and float blend for
        // every visible material pixel.
        const WIDTH: u32 = 64;
        const HEIGHT: u32 = 32;
        let mut landscape = Landscape::flat(WIDTH, HEIGHT as i32);
        landscape.set_pixel_grid(PixelGrid::new(
            WIDTH,
            HEIGHT,
            vec![1; (WIDTH * HEIGHT) as usize],
            vec![0, 50],
            vec![None, Some("Earth".to_string())],
            vec![None, Some("Rough".to_string())],
        ));
        landscape.set_shade_materials(false);
        let mut graphics = test_graphics(WIDTH, HEIGHT, HEIGHT as i32, "opaque landscape blit");
        graphics.set_material_textures(Arc::new(HashMap::from([(
            "rough".to_string(),
            ImageData::new(1, 1, vec![128, 96, 64, 255]),
        )])));
        graphics.set_material_render_info(Arc::new(HashMap::from([(
            "earth".to_string(),
            MaterialRenderInfo::new([255; 9], [0; 6], None, 0, 50),
        )])));

        reset_landscape_destination_samples();
        assert!(graphics.draw_ground_textured(
            Some(&landscape),
            Some(&clonk_graphics::GammaRamp::standard()),
        ));

        assert_eq!(
            landscape_destination_samples(),
            0,
            "opaque source fragments cannot depend on the destination framebuffer"
        );
        assert_eq!(
            graphics.surface().get_pixel(WIDTH - 1, HEIGHT - 1),
            Some(Color::opaque(254, 190, 126)),
            "the opaque specialization preserves the C++ material and gamma output"
        );
    }

    #[test]
    fn warm_landscape_cache_patches_one_direct_surface32_sky_pixel() {
        const WIDTH: u32 = 8;
        const CHANGE_X: i32 = 3;
        const BACKGROUND: Color = Color::new(4, 8, 12, 255);
        let mut bytes = vec![1; WIDTH as usize];
        bytes[CHANGE_X as usize] = 0;
        let mut landscape = Landscape::flat(WIDTH, 1);
        landscape.set_pixel_grid(PixelGrid::new(
            WIDTH,
            1,
            bytes,
            vec![0, 50],
            vec![None, Some("Earth".to_string())],
            vec![None, Some("Rough".to_string())],
        ));
        landscape.set_shade_materials(false);
        let mut graphics = test_graphics(WIDTH, 1, 1, "direct Surface32 cache patch");
        graphics.set_material_textures(Arc::new(HashMap::from([(
            "rough".to_string(),
            ImageData::new(1, 1, vec![128, 96, 64, 255]),
        )])));
        graphics.set_material_render_info(Arc::new(HashMap::from([(
            "earth".to_string(),
            MaterialRenderInfo::new([255; 9], [0; 6], None, 0, 50),
        )])));
        graphics.surface_mut().fill(BACKGROUND);

        reset_material_composition_calls();
        assert!(graphics.draw_ground_textured(Some(&landscape), None));
        assert_eq!(
            material_composition_calls(),
            WIDTH as usize - 1,
            "the cold cache composes every non-sky material cell"
        );
        let unchanged_material = graphics
            .surface()
            .get_pixel(CHANGE_X as u32 - 1, 0)
            .expect("neighboring material pixel");
        assert_eq!(
            graphics.surface().get_pixel(CHANGE_X as u32, 0),
            Some(BACKGROUND),
            "the untouched Surface8 sky cell leaves the backdrop visible"
        );

        assert!(landscape.set_surface32_pixel(CHANGE_X, 0, 0x0011_2233));
        assert_eq!(
            landscape.grid_byte_at(CHANGE_X, 0),
            Some(0),
            "the direct color write leaves Surface8 sky unchanged"
        );
        graphics.surface_mut().fill(BACKGROUND);
        reset_material_composition_calls();
        assert!(graphics.draw_ground_textured(Some(&landscape), None));

        assert_eq!(
            material_composition_calls(),
            0,
            "a warm cache patches the direct color cell instead of rebuilding material pixels"
        );
        assert_eq!(
            graphics.surface().get_pixel(CHANGE_X as u32, 0),
            Some(Color::opaque(0x11, 0x22, 0x33)),
            "packed C4 transparency zero renders as an opaque replacement sky pixel"
        );
        assert_eq!(
            graphics.surface().get_pixel(CHANGE_X as u32 - 1, 0),
            Some(unchanged_material),
            "neighboring cached material output remains unchanged"
        );
    }

    #[test]
    fn distant_landscape_edits_patch_sparse_surface32_regions() {
        // C4Landscape::SetPix keeps distant relights separate and
        // DoRelights updates each bounded Surface32 region independently
        // (C4Landscape.cpp:741-763,2477-2511). Joining these cells would
        // compose the 508-pixel strip between them; multi-million-pixel Far
        // Worlds landscapes magnify that mistake across both axes.
        const WIDTH: u32 = 512;
        const HEIGHT: u32 = 64;
        const CHANGE_Y: i32 = 32;
        let mut landscape = Landscape::flat(WIDTH, HEIGHT as i32);
        landscape.set_pixel_grid(PixelGrid::new(
            WIDTH,
            HEIGHT,
            vec![1; (WIDTH * HEIGHT) as usize],
            vec![0, 50, 50],
            vec![None, Some("Earth".to_string()), Some("Earth".to_string())],
            vec![None, Some("Rough".to_string()), Some("Smooth".to_string())],
        ));
        landscape.set_shade_materials(true);
        let mut graphics = test_graphics(1, 1, 1, "sparse landscape cache patch");
        graphics.set_material_textures(Arc::new(HashMap::from([
            (
                "rough".to_string(),
                ImageData::new(1, 1, vec![255, 0, 0, 255]),
            ),
            (
                "smooth".to_string(),
                ImageData::new(1, 1, vec![0, 255, 0, 255]),
            ),
        ])));
        graphics.set_material_render_info(Arc::new(HashMap::from([(
            "earth".to_string(),
            MaterialRenderInfo::new([255; 9], [0; 6], None, 0, 50),
        )])));

        assert!(graphics.draw_ground_textured(Some(&landscape), None));
        assert!(landscape.insert_material_texture_pix(2, CHANGE_Y, 2));
        assert!(landscape.insert_material_texture_pix(509, CHANGE_Y, 2));
        reset_material_composition_calls();

        assert!(graphics.draw_ground_textured(Some(&landscape), None));
        assert_eq!(
            material_composition_calls(),
            2 * 3 * 17,
            "only the two C++ x=1/y=8 relight neighborhoods should be recomposed"
        );
    }

    #[test]
    fn one_pixel_landscape_edit_recomposes_only_its_dirty_cache_cell() {
        // C4Landscape::SetPix records the changed pixel and DoRelights updates
        // only a bounded rectangle of persistent Surface32
        // (C4Landscape.cpp:741-763,2490-2609). A tiny active-terrain change on
        // Alchemy's large raster must not rebuild every material pixel.
        const WIDTH: u32 = 256;
        const HEIGHT: u32 = 256;
        const CHANGE_X: i32 = 137;
        const CHANGE_Y: i32 = 123;
        let mut landscape = Landscape::flat(WIDTH, HEIGHT as i32);
        landscape.set_pixel_grid(PixelGrid::new(
            WIDTH,
            HEIGHT,
            vec![1; (WIDTH * HEIGHT) as usize],
            vec![0, 50, 50],
            vec![None, Some("Earth".to_string()), Some("Earth".to_string())],
            vec![None, Some("Rough".to_string()), Some("Smooth".to_string())],
        ));
        landscape.set_shade_materials(false);
        let mut graphics = test_graphics(1, 1, 1, "bounded landscape cache patch");
        graphics.set_material_textures(Arc::new(HashMap::from([
            (
                "rough".to_string(),
                ImageData::new(1, 1, vec![255, 0, 0, 255]),
            ),
            (
                "smooth".to_string(),
                ImageData::new(1, 1, vec![0, 255, 0, 255]),
            ),
        ])));
        graphics.set_material_render_info(Arc::new(HashMap::from([(
            "earth".to_string(),
            MaterialRenderInfo::new([255; 9], [0; 6], None, 0, 50),
        )])));
        graphics.viewport_x = CHANGE_X as f32;
        graphics.viewport_y = CHANGE_Y as f32;

        reset_material_composition_calls();
        assert!(graphics.draw_ground_textured(Some(&landscape), None));
        assert_eq!(
            material_composition_calls(),
            (WIDTH * HEIGHT) as usize,
            "the cold cache composes the complete raster once"
        );
        let before = graphics.surface().get_pixel(0, 0).expect("visible pixel");
        let mut sibling = landscape.clone();

        landscape.grid_write_byte(CHANGE_X, CHANGE_Y, 2);
        reset_material_composition_calls();
        assert!(graphics.draw_ground_textured(Some(&landscape), None));
        assert_eq!(
            material_composition_calls(),
            1,
            "one changed texmap byte must not recompose all 65,536 cache pixels"
        );
        assert_ne!(graphics.surface().get_pixel(0, 0), Some(before));

        reset_material_composition_calls();
        assert!(graphics.draw_ground_textured(Some(&landscape), None));
        assert_eq!(
            material_composition_calls(),
            0,
            "an unchanged revision keeps the patched cache"
        );

        sibling.grid_write_byte(CHANGE_X + 1, CHANGE_Y, 2);
        assert_eq!(
            landscape.pixel_grid().expect("live grid").revision(),
            sibling.pixel_grid().expect("sibling grid").revision(),
            "sibling snapshots can carry the same numeric revision"
        );
        reset_material_composition_calls();
        assert!(graphics.draw_ground_textured(Some(&sibling), None));
        assert_eq!(
            material_composition_calls(),
            (WIDTH * HEIGHT) as usize,
            "an unrelated same-revision sibling requires a safe full rebuild"
        );
        reset_material_composition_calls();
        assert!(graphics.draw_ground_textured(Some(&landscape), None));
        assert_eq!(
            material_composition_calls(),
            (WIDTH * HEIGHT) as usize,
            "returning to the other sibling also rebuilds instead of reusing stale pixels"
        );

        landscape.grid_write_byte(CHANGE_X, CHANGE_Y, 0);
        graphics.surface_mut().fill(Color::opaque(4, 8, 12));
        reset_material_composition_calls();
        assert!(graphics.draw_ground_textured(Some(&landscape), None));
        assert_eq!(
            material_composition_calls(),
            0,
            "sky needs no material sample"
        );
        assert_eq!(
            graphics.surface().get_pixel(0, 0),
            Some(Color::opaque(4, 8, 12)),
            "patching a texmap byte to sky clears the old cached material pixel"
        );
        landscape.grid_write_byte(CHANGE_X, CHANGE_Y, 2);
        reset_material_composition_calls();
        assert!(graphics.draw_ground_textured(Some(&landscape), None));
        assert_eq!(material_composition_calls(), 1);

        graphics.set_material_render_info(Arc::new(HashMap::from([(
            "earth".to_string(),
            MaterialRenderInfo::new([255; 9], [0; 6], None, 0, 50),
        )])));
        reset_material_composition_calls();
        assert!(graphics.draw_ground_textured(Some(&landscape), None));
        assert_eq!(
            material_composition_calls(),
            (WIDTH * HEIGHT) as usize,
            "changing material presentation invalidates the complete cache"
        );

        let mut resized = Landscape::flat(8, 4);
        resized.set_pixel_grid(PixelGrid::new(
            8,
            4,
            vec![1; 32],
            vec![0, 50],
            vec![None, Some("Earth".to_string())],
            vec![None, Some("Rough".to_string())],
        ));
        resized.set_shade_materials(false);
        reset_material_composition_calls();
        assert!(graphics.draw_ground_textured(Some(&resized), None));
        assert_eq!(
            material_composition_calls(),
            32,
            "incompatible landscape dimensions require a complete new cache"
        );
    }
}
