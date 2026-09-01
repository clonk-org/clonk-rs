//! `main.rs` — startup and scenario-selection rendering, overlays and group I/O.
//!
//! A contiguous slice moved verbatim from the crate root; it stays part of
//! the same binary crate, re-exported from `main.rs` so every path resolves.

use super::*;

#[derive(Clone, Default)]
pub(crate) struct NativePresentationPlan {
    pub(crate) batches: Vec<NativePresentationBatch>,
    /// CStdGL's non-shader path installs one monitor ramp after every raster
    /// and scale-native text layer has reached the physical framebuffer.
    pub(crate) monitor_gamma: Option<clonk_graphics::GammaRamp>,
}

pub(crate) struct RetainedGpuFrame {
    pub(crate) layers: Vec<RetainedGpuFrameLayer>,
    pub(crate) capture_stats: clonk_graphics::GpuSceneCaptureStats,
}

pub(crate) struct RetainedGpuFrameLayer {
    pub(crate) scene: GpuScene,
    pub(crate) presentation: GpuPresentation,
}

#[derive(Clone)]
pub(crate) struct NativePresentationBatch {
    /// `None` for text attached directly to FramePresenter's already-scaled
    /// base; subsequent batches own a premultiplied logical chrome layer.
    pub(crate) logical_layer: Option<Vec<u8>>,
    /// Set only for a raster layer explicitly isolated to one primary clipper.
    /// Mixed and otherwise unproven layers retain full-frame composition.
    pub(crate) clip: Option<Rect>,
    /// Draw C4LoaderScreen text with its dedicated native-metric path between
    /// this batch's base/chrome and every later GUI layer.
    pub(crate) native_loader_text: bool,
    pub(crate) text: Vec<clonk_graphics::clonk_font::CapturedClonkText>,
    /// A fading outgoing dialog retains the font bundle it was captured with.
    pub(crate) fonts: Option<Arc<clonk_frontend::clonk_fonts::NativeClonkFontSet>>,
    /// Painter-ordered logical commands recorded for this exact batch. CPU
    /// presentation leaves this empty and replays `logical_layer` instead.
    pub(crate) gpu_recorder: Option<GpuSceneRecorder>,
}

/// Config-driven bits the startup parity renderers display.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct StartupViewFlags {
    /// `Config.General.FairCrew`, serialized by C++ as `General.NoCrew`.
    pub(crate) fair_crew: bool,
    /// `Config.General.Record`.
    pub(crate) record: bool,
}

/// Fills the inclusive rect with an engine AARRGGBB color (inverted alpha,
/// gamma-encoded rgb, float blend) like `DrawBoxDw` through the blit shader.
fn fill_engine_box(
    surface: &mut Surface,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    clr: u32,
    gamma: &clonk_graphics::GammaRamp,
) {
    clonk_frontend::classic_gui::draw_engine_box(surface, x0, y0, x1, y1, clr, Some(gamma));
}

/// The list icon of an entry, mirroring the C++ defaults: scenarios use
/// C4S.Head.Icon clamped to the strip else 14 (Scenario::LoadCustom,
/// C4StartupScenSelDlg.cpp:705-710), .c4f folders 0 (SubFolder::LoadCustom,
/// :951-952), plain directories 44 (RegularFolder::LoadCustom, :1036-1037).
pub(crate) fn scensel_entry_icon(entry: &FrontendScenario) -> u32 {
    match entry.kind {
        ScenarioKind::Scenario => entry
            .icon_index
            .filter(|icon| (0..=51).contains(icon))
            .map(|icon| icon as u32)
            .unwrap_or(14),
        _ => match entry.path.as_deref().and_then(|path| path.extension()) {
            Some(ext) if ext.eq_ignore_ascii_case("c4f") => 0,
            Some(_) => 0,
            None => 44,
        },
    }
}

fn scensel_selection(menu: &MenuState) -> Option<&FrontendScenario> {
    menu.selected_scenario().or_else(|| menu.current_folder())
}

pub(crate) fn startup_scensel_game_option_bounds(
    width: i32,
    height: i32,
    fonts: &clonk_frontend::ClonkFontSet,
) -> clonk_frontend::classic_gui::IntRect {
    clonk_frontend::startup_scensel::scen_sel_layout(width, height, fonts).game_option_bounds()
}

pub(crate) fn scensel_selection_info(
    menu: &MenuState,
) -> clonk_frontend::startup_scensel::SelectionInfo<'_> {
    scensel_selection(menu)
        .map(|entry| clonk_frontend::startup_scensel::SelectionInfo {
            picture: entry.title_picture.as_ref(),
            title: Some(entry.title.as_str()),
            desc: entry.description.as_deref(),
            author: entry.author.as_deref(),
            version: entry.version.as_deref(),
        })
        .unwrap_or_default()
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct MapFolderTransform {
    pub(crate) background: GuiRect,
    scale_x: f32,
    scale_y: f32,
}

impl MapFolderTransform {
    pub(crate) fn for_map(
        map: &MapFolderData,
        layout: &clonk_frontend::startup_scensel::ScenSelLayout,
        surface_width: u32,
        surface_height: u32,
    ) -> Self {
        let container = if map.fullscreen_background {
            clonk_frontend::classic_gui::IntRect::new(
                0,
                0,
                surface_width as i32,
                surface_height as i32,
            )
        } else {
            layout.map_sheet
        };
        let image_width = map.background.width().max(1) as f32;
        let image_height = map.background.height().max(1) as f32;
        let (scale_x, scale_y) = if map.fullscreen_background {
            (
                container.w as f32 / image_width,
                container.h as f32 / image_height,
            )
        } else {
            let scale = (container.w as f32 / image_width).min(container.h as f32 / image_height);
            (scale, scale)
        };
        let width = image_width * scale_x;
        let height = image_height * scale_y;
        Self {
            background: GuiRect::new(
                container.x as f32 + (container.w as f32 - width) / 2.0,
                container.y as f32 + (container.h as f32 - height) / 2.0,
                width,
                height,
            ),
            scale_x,
            scale_y,
        }
    }

    pub(crate) fn rect(self, rect: MapFolderRect) -> GuiRect {
        GuiRect::new(
            self.background.origin.x + rect.x as f32 * self.scale_x,
            self.background.origin.y + rect.y as f32 * self.scale_y,
            rect.w as f32 * self.scale_x,
            rect.h as f32 * self.scale_y,
        )
    }

    /// An identity-placed transform, so a caption probe can be drawn without
    /// a background image or a laid-out sheet.
    #[cfg(test)]
    pub(crate) fn identity_for_test() -> Self {
        Self {
            background: GuiRect::new(0.0, 0.0, 0.0, 0.0),
            scale_x: 1.0,
            scale_y: 1.0,
        }
    }

    pub(crate) fn point(self, x: i32, y: i32) -> (i32, i32) {
        (
            (self.background.origin.x + x as f32 * self.scale_x).round() as i32,
            (self.background.origin.y + y as f32 * self.scale_y).round() as i32,
        )
    }
}

pub(crate) fn point_in_map_rect(point: GuiPoint, rect: &GuiRect) -> bool {
    point.x >= rect.origin.x
        && point.x < rect.origin.x + rect.size.width
        && point.y >= rect.origin.y
        && point.y < rect.origin.y + rect.size.height
}

pub(crate) fn map_folder_button_at(
    map: &MapFolderData,
    transform: MapFolderTransform,
    point: GuiPoint,
) -> Option<usize> {
    map.scenarios
        .iter()
        .rposition(|button| point_in_map_rect(point, &transform.rect(button.area)))
}

/// C++ tolerates a font zoom near unity rather than rescaling glyphs for it
/// (`C4StartupScenSelDlg.cpp:380`).
fn tolerated_font_zoom(height: i32, line_height: i32) -> f32 {
    if line_height == 0 {
        return 1.0;
    }
    let zoom = height as f32 / line_height as f32;
    if (0.8..1.25).contains(&zoom) {
        1.0
    } else {
        zoom
    }
}

/// `C4GUI::Resource::GetFontByHeight` — the first font whose line height is at
/// least the requested height, else the title font (`C4Gui.cpp:1235-1253`).
pub(crate) fn gui_font_by_height(
    fonts: &clonk_frontend::ClonkFontSet,
    height: i32,
) -> (&clonk_graphics::clonk_font::ClonkFont, f32) {
    let font = if height <= fonts.mini.line_height {
        &fonts.mini
    } else if height <= fonts.text.line_height {
        &fonts.text
    } else if height <= fonts.caption.line_height {
        &fonts.caption
    } else {
        &fonts.title
    };
    (font, tolerated_font_zoom(height, font.line_height))
}

/// `C4StartupGraphics::GetBlackFontByHeight` over the book faces
/// (`C4Startup.cpp:125-143`).
pub(crate) fn book_font_by_height(
    fonts: &clonk_frontend::startup_scensel::BookFontSet,
    height: i32,
) -> (&clonk_graphics::clonk_font::ClonkFont, f32) {
    let font = if height <= fonts.small.line_height {
        &fonts.small
    } else if height <= fonts.text.line_height {
        &fonts.text
    } else if height <= fonts.caption.line_height {
        &fonts.caption
    } else {
        &fonts.title
    };
    (font, tolerated_font_zoom(height, font.line_height))
}

/// One folder-map button's caption.
///
/// `C4StartupScenSelDlg` picks the font by the scaled title height and passes
/// the tolerated zoom straight into `SetTextFont`
/// (`src/C4StartupScenSelDlg.cpp:371-377`).
pub(crate) fn draw_map_scenario_title(
    surface: &mut clonk_graphics::Surface,
    fonts: &clonk_frontend::ClonkFontSet,
    book_fonts: &clonk_frontend::startup_scensel::BookFontSet,
    button: &MapFolderScenarioButton,
    transform: &MapFolderTransform,
    active: bool,
    gamma: &clonk_graphics::GammaRamp,
) {
    if button.title.is_empty() {
        return;
    }
    let scaled_size = (button.title_font_size as f32 * transform.scale_y).round() as i32;
    let (font, zoom) = if button.title_use_book_font {
        book_font_by_height(book_fonts, scaled_size)
    } else {
        gui_font_by_height(fonts, scaled_size)
    };
    let (x, y) = transform.point(
        button.area.x + button.title_offset_x,
        button.area.y + button.title_offset_y,
    );
    let align = match button.title_align {
        0 => clonk_graphics::clonk_font::TextAlign::Left,
        2 => clonk_graphics::clonk_font::TextAlign::Right,
        _ => clonk_graphics::clonk_font::TextAlign::Center,
    };
    font.draw_zoomed_with_gamma(
        surface,
        x,
        y,
        &button.title,
        map_folder_text_color(if active {
            button.title_color_active
        } else {
            button.title_color_inactive
        }),
        align,
        true,
        Some(gamma),
        zoom,
    );
}

fn map_folder_text_color(color: u32) -> [u8; 4] {
    [
        ((color >> 16) & 0xff) as u8,
        ((color >> 8) & 0xff) as u8,
        (color & 0xff) as u8,
        255 - ((color >> 24) & 0xff) as u8,
    ]
}

#[allow(clippy::too_many_arguments)]
fn draw_scensel_map_dynamic(
    surface: &mut Surface,
    scenario_menu: &mut MenuState,
    assets: &clonk_frontend::startup_scensel::ScenSelAssets,
    button_down: &ImageData,
    fonts: &clonk_frontend::ClonkFontSet,
    book_fonts: &clonk_frontend::startup_scensel::BookFontSet,
    gamma: &clonk_graphics::GammaRamp,
    draw_focus: bool,
    title: &str,
) -> Result<()> {
    use clonk_frontend::startup_scensel as scensel;

    let layout = scensel::scen_sel_layout(surface.width() as i32, surface.height() as i32, fonts);
    let Some(map) = scenario_menu.current_map() else {
        return Ok(());
    };
    let transform = MapFolderTransform::for_map(map, &layout, surface.width(), surface.height());
    let pointer = scenario_menu.pointer_position();
    let selected_button = map.selected_button;
    let hide_title = map.hide_title;
    let scenario_info_area = map.scenario_info_area;

    if map.fullscreen_background {
        clonk_frontend::draw_image_bilinear(
            surface,
            &transform.background,
            &map.background,
            Some(gamma),
        );
    } else {
        clonk_frontend::draw_image_x_float(
            surface,
            &transform.background,
            &map.background,
            Some(gamma),
        );
    }
    for overlay in &map.access_overlays {
        if let Some(image) = overlay.image.as_ref() {
            clonk_frontend::draw_image_x_float(
                surface,
                &transform.rect(overlay.area),
                image,
                Some(gamma),
            );
        }
    }
    for (index, button) in map.scenarios.iter().enumerate() {
        let rect = transform.rect(button.area);
        let active = draw_focus
            && ((selected_button == Some(index)
                && scenario_menu.dialog_focus() == ScenselDialogFocus::List)
                || pointer.is_some_and(|point| point_in_map_rect(point, &rect)));
        let image = if active {
            button.overlay_image.as_ref()
        } else {
            button.base_image.as_ref()
        };
        if let Some(image) = image {
            clonk_frontend::draw_image_x_float(surface, &rect, image, Some(gamma));
        }
        draw_map_scenario_title(
            surface, fonts, book_fonts, button, &transform, active, gamma,
        );
    }

    let info_rect = transform.rect(scenario_info_area);
    let mut info_layout = layout;
    info_layout.selection_info = clonk_frontend::classic_gui::IntRect::new(
        info_rect.origin.x.round() as i32,
        info_rect.origin.y.round() as i32,
        info_rect.size.width.round() as i32,
        info_rect.size.height.round() as i32,
    );
    let info = scensel_selection_info(scenario_menu);
    let metrics = scensel::draw_selection_info_scrolled(
        surface,
        &info_layout,
        assets,
        book_fonts,
        &info,
        scenario_menu.selection_info_scroll,
        Some(gamma),
    );
    scenario_menu.selection_info_scroll = metrics.clamp_offset(scenario_menu.selection_info_scroll);

    if !hide_title {
        fonts.title.draw_with_gamma(
            surface,
            layout.title_anchor.0,
            layout.title_anchor.1,
            title,
            [255, 255, 0, 255],
            clonk_graphics::clonk_font::TextAlign::Center,
            true,
            Some(gamma),
        );
    }

    let pointer_over = |rect: clonk_frontend::classic_gui::IntRect| {
        draw_focus
            && pointer.is_some_and(|point| {
                point.x >= rect.x as f32
                    && point.x < (rect.x + rect.w) as f32
                    && point.y >= rect.y as f32
                    && point.y < (rect.y + rect.h) as f32
            })
    };
    scensel::draw_back_button_with_state(
        surface,
        &layout,
        "Back",
        assets,
        button_down,
        fonts,
        scensel::ScenSelButtonState {
            highlighted: draw_focus
                && (scenario_menu.dialog_focus() == ScenselDialogFocus::Back
                    || pointer_over(layout.back_button)),
            pressed: false,
        },
        Some(gamma),
    )?;
    let is_scenario = scenario_menu.selected_scenario().is_some();
    scensel::draw_open_button_with_state(
        surface,
        &layout,
        if is_scenario { "&Start" } else { "Open" },
        assets,
        button_down,
        fonts,
        scensel::ScenSelButtonState {
            highlighted: draw_focus
                && (scenario_menu.dialog_focus() == ScenselDialogFocus::Open
                    || pointer_over(layout.open_button)),
            pressed: false,
        },
        Some(gamma),
    )?;
    let cb_enabled = scenario_menu.definition_checkbox_enabled;
    let cb_checked = scenario_menu.definition_checkbox_checked;
    let cb_highlighted = cb_enabled
        && (scenario_menu.definition_checkbox_focused || pointer_over(layout.user_change_checkbox));
    scensel::draw_user_change_checkbox(
        surface,
        &layout,
        assets,
        fonts,
        cb_enabled,
        cb_checked,
        cb_highlighted,
        Some(gamma),
    );
    Ok(())
}

/// Draws the selection-dependent layer of the scenario book over the cached
/// chrome: caption, list rows + selection bar, the right info page, the
/// Open/Start button and the "Choose definitions" checkbox.
pub(crate) fn draw_scensel_dynamic(
    surface: &mut Surface,
    scenario_menu: &mut MenuState,
    scenario_entry_enabled: &HashMap<String, bool>,
    assets: &clonk_frontend::startup_scensel::ScenSelAssets,
    button_down: &ImageData,
    fonts: &clonk_frontend::ClonkFontSet,
    book_fonts: &clonk_frontend::startup_scensel::BookFontSet,
    loading_label: Option<&str>,
    gamma: &clonk_graphics::GammaRamp,
    draw_focus: bool,
) -> Result<()> {
    use clonk_frontend::startup_scensel as scensel;

    let layout = scensel::scen_sel_layout(surface.width() as i32, surface.height() as i32, fonts);
    let pointer = scenario_menu.pointer_position();
    let pointer_over = |rect: clonk_frontend::classic_gui::IntRect| {
        draw_focus
            && pointer.is_some_and(|point| {
                point.x >= rect.x as f32
                    && point.x < (rect.x + rect.w) as f32
                    && point.y >= rect.y as f32
                    && point.y < (rect.y + rect.h) as f32
            })
    };
    scensel::draw_back_button_with_state(
        surface,
        &layout,
        "Back",
        assets,
        button_down,
        fonts,
        scensel::ScenSelButtonState {
            highlighted: draw_focus
                && (scenario_menu.dialog_focus() == ScenselDialogFocus::Back
                    || pointer_over(layout.back_button)),
            pressed: false,
        },
        Some(gamma),
    )?;

    let search_cursor_x = fonts
        .text
        .measure(
            &scenario_menu.search_edit.text[..scenario_menu.search_edit.caret],
            false,
        )
        .0;
    let search_cursor_half = fonts.text.measure("¦", false).0 / 2;
    let search_has_clear = !scenario_menu.search_text().is_empty();
    let search_clear_width = if search_has_clear {
        scensel::search_clear_button_bounds(&layout).w
    } else {
        0
    };
    scenario_menu.search_edit.scroll_cursor_in_view(
        search_cursor_x,
        layout.search_edit.w - 8 - search_clear_width,
        search_cursor_half,
    );
    let search_selection = scenario_menu
        .search_edit
        .selection_range()
        .map(|range| (range.start, range.end));
    scensel::draw_search_edit_contents(
        surface,
        &layout,
        fonts,
        scenario_menu.search_text(),
        scenario_menu.search_edit.caret,
        search_selection,
        scenario_menu.search_edit.horizontal_scroll,
        draw_focus && scenario_menu.search_edit.cursor_visible(),
        scenario_menu.search_edit.composition(),
        Some(gamma),
    );
    if search_has_clear {
        scensel::draw_search_clear_button(surface, &layout, fonts, Some(gamma));
    }

    if let Some(label) = loading_label {
        scensel::draw_loading_label(surface, &layout, fonts, book_fonts, label, Some(gamma));
        return Ok(());
    }

    // Enhanced search replaces the folder caption with settled result status;
    // ordinary browsing retains the C++ folder caption.
    let enhanced_caption = scenario_menu.enhanced_search_caption();
    scensel::draw_book_caption(
        surface,
        &layout,
        book_fonts,
        enhanced_caption
            .as_deref()
            .unwrap_or_else(|| scenario_menu.book_caption()),
        Some(gamma),
    );

    // List rows (ScenListItem, cpp:1210-1238) with the ListBox selection bar.
    let item_h = scensel::scen_list_item_height(&book_fonts.text);
    let pitch = item_h + 1; // C4GUI_DefaultListSpacing
    let x = layout.list.x + 3;
    let item_w = layout.list.w - 6 - 16;
    let top = layout.list.y + 3;
    let bottom = layout.list.y + layout.list.h - 3;
    let viewport_height = bottom - top;
    let offset = usize::from(scenario_menu.include_back);
    let selected = scenario_menu.menu().selected_index();
    if scenario_menu.list_scroll_selection != Some(selected) {
        scenario_menu.ensure_list_selection_visible(viewport_height, pitch, item_h);
    }
    let rows: Vec<(String, u32, String, bool)> = scenario_menu
        .visible_entries()
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let mut title = entry.title.clone();
            Markup::strip_markup(&mut title);
            if let Some(context) = scenario_menu.search_result_context(index) {
                title.push_str(" - ");
                title.push_str(context);
            }
            let enabled = scenario_entry_enabled
                .get(&entry.identifier)
                .copied()
                .unwrap_or(!matches!(entry.kind, ScenarioKind::Scenario));
            (
                entry.identifier.clone(),
                scensel_entry_icon(entry),
                title,
                enabled,
            )
        })
        .collect();
    // The list viewport is the C4GUI::ListBox primary clipper. Draw directly
    // into the destination so scale-native ClonkFont capture remains attached
    // to the frame; a cloned scratch surface would retain and then discard the
    // semantic row-label commands while only its raster pixels were copied.
    let previous_clip = surface.clip();
    let viewport_clip = Rect::new(x, top, item_w.max(0) as u32, viewport_height.max(0) as u32);
    let list_clip = previous_clip.map_or(viewport_clip, |clip| {
        clip.intersection(viewport_clip).unwrap_or(Rect::new(
            viewport_clip.x,
            viewport_clip.y,
            0,
            0,
        ))
    });
    surface.set_clip(list_clip);
    if let Some(message) = scenario_menu.enhanced_search_empty_message() {
        book_fonts.text.draw_with_gamma(
            surface,
            x + 4,
            top + 4,
            &message,
            [0, 0, 0, 255],
            clonk_graphics::clonk_font::TextAlign::Left,
            false,
            Some(gamma),
        );
        book_fonts.text.draw_with_gamma(
            surface,
            x + 4,
            top + 4 + book_fonts.text.line_height,
            scenario_menu.enhanced_search_clear_hint(),
            [0, 0, 0, 255],
            clonk_graphics::clonk_font::TextAlign::Left,
            false,
            Some(gamma),
        );
    }
    let mut y = top - scenario_menu.scenario_list_scroll();
    for (index, (identifier, icon, title, enabled)) in rows.iter().enumerate() {
        if y >= bottom {
            break;
        }
        if y + item_h > top && selected == Some(index + offset) {
            // C4GUI_ListBoxSelColor while the list draws focus; the edit or
            // an open context retains logical focus but uses InactiveSelColor.
            let selection_color = if draw_focus
                && scenario_menu.dialog_focus() == ScenselDialogFocus::List
                && scenario_menu.rename_edit.is_none()
            {
                0xafaf0000
            } else {
                0xaf7f7f7f
            };
            fill_engine_box(
                surface,
                x,
                y,
                x + item_w - 1,
                y + item_h - 1,
                selection_color,
                gamma,
            );
        }
        if y + item_h > top {
            let is_renaming = scenario_menu.rename_edit.as_ref().is_some_and(|rename| {
                rename.identifier == *identifier && !rename.edit.label_visible()
            });
            scensel::draw_scen_list_item(
                surface,
                &assets.scen_icons,
                &book_fonts.text,
                Some(gamma),
                x,
                y,
                *icon,
                if is_renaming { "" } else { title },
                *enabled,
            );
            if is_renaming {
                let edit_x = x + item_h + 2;
                let edit_w = (item_w - item_h - 4).max(1) as u32;
                let edit_h = fonts.text.line_height.max(1) as u32;
                if let Some(rename) = scenario_menu.rename_edit.as_mut() {
                    rename.edit.render(
                        surface,
                        &fonts.text,
                        clonk_frontend::classic_gui::IntRect::new(
                            edit_x,
                            y + 2,
                            edit_w as i32,
                            edit_h as i32,
                        ),
                        Some(gamma),
                    );
                }
            }
        }
        y += pitch;
    }
    if let Some(clip) = previous_clip {
        surface.set_clip(clip);
    } else {
        surface.clear_clip();
    }
    let list_max_scroll = scenario_menu.scenario_list_max_scroll(viewport_height, pitch);
    if list_max_scroll > 0 {
        let bar = layout.list_scrollbar;
        let max_pin_travel = (bar.h - 48).max(0);
        let pin = scenario_menu
            .scrollbar_interaction
            .filter(|interaction| interaction.target == ScenselScrollbarTarget::List)
            .map(|interaction| interaction.pin)
            .unwrap_or_else(|| {
                max_pin_travel * scenario_menu.scenario_list_scroll() / list_max_scroll
            })
            .clamp(0, max_pin_travel);
        let pin_y = bar.y + 16 + pin;
        clonk_frontend::draw_image_strip(
            surface,
            bar.x,
            pin_y,
            &assets.book_scroll,
            16,
            16,
            16,
            16,
            Some(gamma),
        );
    }

    // Right page + selection-specific button/checkbox states
    // (UpdateSelection, cpp:1551-1619): the selected entry, else the current
    // folder (but not the root).
    let info = scensel_selection_info(scenario_menu);
    let scroll_metrics = scensel::draw_selection_info_scrolled(
        surface,
        &layout,
        assets,
        book_fonts,
        &info,
        scenario_menu.selection_info_scroll,
        Some(gamma),
    );
    scenario_menu.selection_info_scroll =
        scroll_metrics.clamp_offset(scenario_menu.selection_info_scroll);

    if let Some(interaction) = scenario_menu.scrollbar_interaction {
        let bar = match interaction.target {
            ScenselScrollbarTarget::List => layout.list_scrollbar,
            ScenselScrollbarTarget::Description => scensel::selection_info_scrollbar_rect(&layout),
        };
        if interaction.target == ScenselScrollbarTarget::Description
            && scroll_metrics.max_scroll > 0
            && scensel_scrollbar_pin_travel(bar.h).is_some()
        {
            clonk_frontend::draw_image_strip(
                surface,
                bar.x,
                bar.y + SCENSEL_SCROLLBAR_PART + interaction.pin,
                &assets.book_scroll,
                16,
                16,
                16,
                16,
                Some(gamma),
            );
        }
        if let ScenselScrollbarInteractionKind::Arrow(direction) = interaction.kind {
            let (destination_y, source_y) = if direction < 0 {
                (bar.y, 0)
            } else {
                (bar.y + bar.h - SCENSEL_SCROLLBAR_PART, 32)
            };
            clonk_frontend::draw_image_strip(
                surface,
                bar.x,
                destination_y,
                &assets.book_scroll,
                16,
                source_y,
                16,
                16,
                Some(gamma),
            );
        }
    }

    let selection = scensel_selection(scenario_menu);
    let is_scenario = selection.is_some_and(|entry| matches!(entry.kind, ScenarioKind::Scenario));
    let open_text = if is_scenario { "&Start" } else { "Open" };
    scensel::draw_open_button_with_state(
        surface,
        &layout,
        open_text,
        assets,
        button_down,
        fonts,
        scensel::ScenSelButtonState {
            highlighted: draw_focus
                && (scenario_menu.dialog_focus() == ScenselDialogFocus::Open
                    || pointer_over(layout.open_button)),
            pressed: false,
        },
        Some(gamma),
    )?;

    let cb_enabled = scenario_menu.definition_checkbox_enabled;
    let cb_checked = scenario_menu.definition_checkbox_checked;
    let cb_highlighted = cb_enabled
        && (scenario_menu.definition_checkbox_focused
            || scenario_menu.pointer_position().is_some_and(|point| {
                point.x >= layout.user_change_checkbox.x as f32
                    && point.x
                        < (layout.user_change_checkbox.x + layout.user_change_checkbox.h) as f32
                    && point.y >= layout.user_change_checkbox.y as f32
                    && point.y
                        < (layout.user_change_checkbox.y + layout.user_change_checkbox.h) as f32
            }));
    scensel::draw_user_change_checkbox(
        surface,
        &layout,
        assets,
        fonts,
        cb_enabled,
        cb_checked,
        cb_highlighted,
        Some(gamma),
    );
    Ok(())
}

/// The startup render gamma ramp (default config: identity + black floor).
pub(crate) fn startup_gamma() -> &'static clonk_graphics::GammaRamp {
    static STARTUP_GAMMA: std::sync::OnceLock<clonk_graphics::GammaRamp> =
        std::sync::OnceLock::new();
    STARTUP_GAMMA.get_or_init(clonk_graphics::GammaRamp::standard)
}

pub(crate) fn startup_identity_gamma() -> &'static clonk_graphics::GammaRamp {
    static IDENTITY_GAMMA: std::sync::OnceLock<clonk_graphics::GammaRamp> =
        std::sync::OnceLock::new();
    IDENTITY_GAMMA.get_or_init(clonk_graphics::GammaRamp::identity)
}

pub(crate) fn render_startup_underlay(
    graphics: &mut GraphicsSystem,
    assets: &FrontendAssets,
    gamma: &clonk_graphics::GammaRamp,
    frame: &mut [u8],
) {
    let surface = graphics.surface_mut();
    if let Some(background) = assets.menu_background() {
        let rect = clonk_gui::Rect::from_origin_size(
            GuiPoint::new(0.0, 0.0),
            clonk_gui::Size::new(surface.width() as f32, surface.height() as f32),
        );
        clonk_frontend::draw_image_bilinear(surface, &rect, &background, Some(gamma));
    } else {
        surface.fill(Color::opaque(0, 0, 0));
    }
    if !surface.is_gpu_scene_capture_active() {
        if surface.pixels().len() == frame.len() {
            frame.copy_from_slice(surface.pixels());
        } else {
            copy_surface(surface.pixels(), surface.width(), surface.height(), frame);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StartupMainLogoLayout {
    /// The product-brand allocation inherited from the native startup layout.
    pub(crate) slot: (i32, i32, i32, i32),
    /// The aspect-preserving destination of the active Clonk Rust artwork.
    pub(crate) image: (i32, i32, i32, i32),
}

pub(crate) fn startup_main_logo_layout(
    surface_width: i32,
    surface_height: i32,
    logo_width: u32,
    logo_height: u32,
) -> StartupMainLogoLayout {
    // The vertical footprint of the classic 960x320 logo at C++'s 0.4 zoom.
    const CLASSIC_LOGO_MAX_HEIGHT: i32 = 128;

    let mut logo_w = (0.4 * logo_width as f32) as i32;
    let mut logo_h = (0.4 * logo_height as f32) as i32;
    if logo_h > CLASSIC_LOGO_MAX_HEIGHT {
        logo_w = (u64::from(logo_width) * CLASSIC_LOGO_MAX_HEIGHT as u64 / u64::from(logo_height))
            as i32;
        logo_h = CLASSIC_LOGO_MAX_HEIGHT;
    }
    let logo_x = surface_width * 30 / 31 - logo_w;
    let logo_y = surface_height / 21 - 5;
    StartupMainLogoLayout {
        slot: (surface_width * 30 / 31 - 384, logo_y, 384, 128),
        image: (logo_x, logo_y, logo_w, logo_h),
    }
}

pub(crate) fn scenario_list_scrollbar_visible(
    scenario_menu: &MenuState,
    layout: &clonk_frontend::startup_scensel::ScenSelLayout,
    book_fonts: &clonk_frontend::startup_scensel::BookFontSet,
) -> bool {
    let item_height = clonk_frontend::startup_scensel::scen_list_item_height(&book_fonts.text);
    let pitch = item_height + 1;
    let viewport_height = layout.list.h - 6;
    scenario_menu.scenario_list_max_scroll(viewport_height, pitch) > 0
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_startup_frame(
    graphics: &mut GraphicsSystem,
    assets: &FrontendAssets,
    main_menu: &mut MainMenuState,
    scenario_menu: &mut MenuState,
    scenario_entry_enabled: &HashMap<String, bool>,
    scenario_loading_label: Option<&str>,
    network_dialog: Option<&clonk_frontend::startup_netdlg::NetDlgController>,
    player_dialog: Option<&clonk_frontend::startup_plrsel::PlrSelController>,
    player_models: &[clonk_frontend::startup_plrsel::PlrSelPlayer],
    crew_models: &[clonk_frontend::startup_plrsel::PlrSelCrew],
    crew_rename: Option<&mut StartupCrewRenameState>,
    context_menu: Option<&ClassicContextMenu<AppContextMenuCommand>>,
    context_menu_open: bool,
    definition_selector_open: bool,
    game_option_input_open: bool,
    message_dialog_open: bool,
    scenario_game_options: &GameOptionButtons,
    scenario_selector_mode: ScenarioSelectorMode,
    options_dialog: Option<&clonk_frontend::startup_options_dlg::OptionsDlgState>,
    options_advanced: Option<
        &mut clonk_frontend::startup_options_advanced::AdvancedConfigController,
    >,
    options_draw_focus: bool,
    about_dialog: Option<&clonk_frontend::startup_about_dlg::AboutDlgState>,
    view: StartupView,
    network_lobby: Option<&mut NetworkLobbyState>,
    flags: StartupViewFlags,
    backdrop: &mut StartupBackdropCache,
    defer_native_main_text: bool,
    gamma: &clonk_graphics::GammaRamp,
    frame: &mut [u8],
) -> Result<()> {
    if view == StartupView::MainMenu {
        assets
            .require_classic_startup_main_resources()
            .map_err(report_classic_parity_boundary)?;
    }
    {
        let surface = graphics.surface_mut();
        let scenario_list_scrollbar_visible = if view == StartupView::ScenarioBrowser {
            assets
                .clonk_fonts
                .as_ref()
                .zip(assets.book_fonts.as_ref())
                .is_some_and(|(fonts, book_fonts)| {
                    let layout = clonk_frontend::startup_scensel::scen_sel_layout(
                        surface.width() as i32,
                        surface.height() as i32,
                        fonts,
                    );
                    scenario_list_scrollbar_visible(scenario_menu, &layout, book_fonts)
                })
        } else {
            false
        };
        let backdrop_key = StartupBackdropKey {
            view,
            width: surface.width(),
            height: surface.height(),
            fair_crew: flags.fair_crew,
            record: flags.record,
            network_host_selector: scenario_selector_mode == ScenarioSelectorMode::NetworkHost,
            scenario_list_scrollbar_visible,
        };

        // C++-faithful parity renderers draw their own backgrounds.
        let parity_rendered = match view {
            StartupView::About => match (
                assets.about_dlg_assets(),
                assets.clonk_fonts.as_ref(),
                about_dialog,
            ) {
                (Some(dlg_assets), Some(fonts), Some(dialog)) => {
                    clonk_frontend::startup_about_dlg::AboutDlgScreen::render_state_with_draw_focus(
                        surface,
                        &dlg_assets,
                        fonts,
                        dialog,
                        Some(gamma),
                        !context_menu_open
                            && !definition_selector_open
                            && !game_option_input_open
                            && !message_dialog_open,
                    );
                    true
                }
                _ => false,
            },
            StartupView::ScenarioBrowser => match (
                assets.scensel_assets(),
                assets.startup_dialog_images.get("GUIButtonDown.png"),
                assets.clonk_fonts.as_ref(),
                assets.book_fonts.as_ref(),
            ) {
                (Some(dlg_assets), Some(button_down), Some(fonts), Some(book_fonts)) => {
                    let title = if scenario_selector_mode == ScenarioSelectorMode::NetworkHost {
                        "Start Network Game"
                    } else {
                        "Start Game"
                    };
                    let draw_focus =
                        !context_menu_open && !definition_selector_open && !game_option_input_open;
                    if scenario_loading_label.is_none() && scenario_menu.current_map().is_some() {
                        // The book sheet is hidden in map mode. Paint only the
                        // dialog background before the map so cached search/
                        // list chrome cannot leak through aspect-fit margins.
                        clonk_frontend::draw_image_bilinear(
                            surface,
                            &GuiRect::new(
                                -1.0,
                                -1.0,
                                surface.width() as f32 + 2.0,
                                surface.height() as f32 + 2.0,
                            ),
                            &dlg_assets.background,
                            Some(gamma),
                        );
                        draw_scensel_map_dynamic(
                            surface,
                            scenario_menu,
                            &dlg_assets,
                            button_down,
                            fonts,
                            book_fonts,
                            gamma,
                            draw_focus,
                            title,
                        )?;
                    } else {
                        // Selection-independent chrome only; the caption,
                        // list, right page, Open button and checkbox change
                        // with the selection and are drawn fresh over the
                        // restored copy.
                        restore_or_render_backdrop(backdrop, backdrop_key, surface, |surface| {
                            clonk_frontend::startup_scensel::ScenSelScreen::render_backdrop_without_game_options(
                                surface,
                                &dlg_assets,
                                fonts,
                                scenario_list_scrollbar_visible,
                                Some(gamma),
                            );
                        });
                        // CStdFont glyphs are semantic commands during
                        // scale-native capture and therefore are not part of
                        // the pixel-only backdrop cache. C++ redraws both
                        // labels each frame; do the same on cache hits.
                        clonk_frontend::startup_scensel::ScenSelScreen::draw_chrome_text(
                            surface,
                            fonts,
                            title,
                            Some(gamma),
                        );
                        draw_scensel_dynamic(
                            surface,
                            scenario_menu,
                            scenario_entry_enabled,
                            &dlg_assets,
                            button_down,
                            fonts,
                            book_fonts,
                            scenario_loading_label,
                            gamma,
                            draw_focus,
                        )?;
                    }
                    let resources = assets.game_option_resources().with_context(|| {
                        "classic scenario game-option resources are unavailable"
                    })?;
                    scenario_game_options.render(
                        surface,
                        &resources,
                        !context_menu_open
                            && !definition_selector_open
                            && !game_option_input_open
                            && !message_dialog_open,
                        Some(gamma),
                    )?;
                    true
                }
                _ => false,
            },
            StartupView::NetworkGame => match (
                assets.netdlg_assets(),
                assets.clonk_fonts.as_ref(),
                network_dialog,
            ) {
                (Some(dlg_assets), Some(fonts), Some(dialog)) => {
                    clonk_frontend::startup_netdlg::NetDlgScreen::render_controller_with_draw_focus(
                        surface,
                        &dlg_assets,
                        fonts,
                        Some(gamma),
                        dialog,
                        0,
                        !context_menu_open
                            && !definition_selector_open
                            && !game_option_input_open
                            && !message_dialog_open,
                    );
                    true
                }
                _ => false,
            },
            StartupView::NetworkLobby => match network_lobby {
                Some(lobby)
                    if assets.game_lobby_resources().is_ok()
                        && assets.game_option_resources().is_ok() =>
                {
                    lobby.render_classic(
                        surface,
                        assets,
                        scenario_game_options,
                        false,
                        !context_menu_open
                            && !definition_selector_open
                            && !game_option_input_open
                            && !message_dialog_open,
                        gamma,
                    )?;
                    true
                }
                _ => false,
            },
            StartupView::Options => match (
                assets.options_dlg_assets(),
                assets.clonk_fonts.as_ref(),
                assets.options_book_fonts.as_ref(),
                options_dialog,
            ) {
                (Some(dlg_assets), Some(fonts), Some(book), Some(dialog)) => {
                    clonk_frontend::startup_options_dlg::OptionsDlgScreen::render_state_with_draw_focus(
                        surface,
                        &dlg_assets,
                        fonts,
                        book,
                        dialog,
                        Some(gamma),
                        options_draw_focus,
                    );
                    if let Some(controller) = options_advanced {
                        let advanced_assets =
                            assets.options_advanced_assets().with_context(|| {
                                "classic advanced-options resources are unavailable"
                            })?;
                        clonk_frontend::startup_options_advanced::AdvancedConfigScreen::render(
                            surface,
                            &advanced_assets,
                            fonts,
                            controller,
                            !message_dialog_open,
                            Some(gamma),
                        )?;
                    }
                    true
                }
                _ => false,
            },
            StartupView::PlayerSelection => match (
                assets.plrsel_assets(),
                assets.clonk_fonts.as_ref(),
                assets.plrsel_book_fonts.as_ref(),
                player_dialog,
            ) {
                (Some(dlg_assets), Some(fonts), Some(book), Some(dialog)) => {
                    clonk_frontend::startup_plrsel::PlrSelScreen::render_controller_with_crew_rename_and_draw_focus(
                        surface,
                        &dlg_assets,
                        fonts,
                        book.as_ref(),
                        player_models,
                        crew_models,
                        dialog,
                        crew_rename.map(|rename| (rename.index, &mut rename.edit)),
                        !context_menu_open
                            && !definition_selector_open
                            && !game_option_input_open
                            && !message_dialog_open,
                        Some(gamma),
                    );
                    true
                }
                _ => false,
            },
            _ => false,
        };
        if parity_rendered {
            if let Some(context_menu) = context_menu {
                context_menu.render_panels(surface, Some(gamma))?;
            }
            let surface = graphics.surface();
            if !surface.is_gpu_scene_capture_active() {
                let pixels = surface.pixels();
                if pixels.len() == frame.len() {
                    frame.copy_from_slice(pixels);
                } else {
                    copy_surface(pixels, surface.width(), surface.height(), frame);
                }
            }
            return Ok(());
        }
        if view != StartupView::MainMenu {
            tracing::error!(
                ?view,
                "refusing to render non-classic startup fallback pane"
            );
            anyhow::bail!(
                "classic startup screen {view:?} is unavailable; refusing generic Rust fallback"
            );
        }
        let background = match view {
            StartupView::ScenarioBrowser | StartupView::NetworkLobby => {
                assets.scenario_browser_background()
            }
            StartupView::Options => assets.options_background(),
            StartupView::About => assets.about_background(),
            _ => assets.menu_background(),
        };
        restore_or_render_backdrop(backdrop, backdrop_key, surface, |surface| {
            if let Some(background) = background {
                // C++ stretches the loader fullscreen with GL_LINEAR filtering
                // (C4Facet::DrawFullScreen, C4Facet.cpp:130-140; StdGL.cpp:528-532).
                let rect = clonk_gui::Rect::from_origin_size(
                    GuiPoint::new(0.0, 0.0),
                    clonk_gui::Size::new(surface.width() as f32, surface.height() as f32),
                );
                clonk_frontend::draw_image_bilinear(surface, &rect, &background, Some(gamma));
            } else {
                surface.fill(Color::opaque(16, 28, 52));
            }
        });
        match view {
            // Missing parity assets leave the dialog's fallback background.
            StartupView::NetworkGame | StartupView::PlayerSelection => {}
            StartupView::MainMenu => {
                if defer_native_main_text {
                    main_menu.render_chrome(surface);
                } else {
                    main_menu.render(surface, !context_menu_open);
                }
                // Logo + version line per C4StartupMainDlg::DrawElement
                // (C4StartupMainDlg.cpp:111-122), in C++ integer math.
                if let Some(logo) = assets.logo() {
                    let width = surface.width() as i32;
                    let height = surface.height() as i32;
                    let (logo_x, logo_y, logo_w, logo_h) =
                        startup_main_logo_layout(width, height, logo.width(), logo.height()).image;
                    let logo_rect = clonk_gui::Rect::new(
                        logo_x as f32,
                        logo_y as f32,
                        logo_w as f32,
                        logo_h as f32,
                    );
                    clonk_frontend::draw_image_bilinear(surface, &logo_rect, &logo, Some(gamma));

                    // Placement, font, colour and markup follow C++ exactly:
                    // right-aligned at (Wdt*39/40, Hgt/18 + 0.4*logoHgt) in the
                    // GUI TextFont, white, markup on (C4StartupMainDlg.cpp:121).
                    //
                    // The string itself deliberately diverges. C++ draws
                    // C4VERSION ("4.9.11.0 [362] "); this port draws its own
                    // release version, because that is what identifies a build
                    // in a bug report. The engine compatibility version still
                    // lives in `clonk_core::version::ENGINE_VERSION` and is what
                    // content gating and protocol identification use.
                    if !defer_native_main_text {
                        let version_text = format!("Version {}", clonk_core::version::PORT_VERSION);
                        let version_text = version_text.as_str();
                        let version_x = width * 39 / 40;
                        let version_y = height / 18 + logo_h;
                        if let Some(fonts) = assets.clonk_fonts.as_ref() {
                            fonts.text.draw_with_gamma(
                                surface,
                                version_x,
                                version_y,
                                version_text,
                                [255, 255, 255, 255],
                                clonk_graphics::clonk_font::TextAlign::Right,
                                true,
                                Some(gamma),
                            );
                        } else {
                            let font = assets.font_arc();
                            let metrics = font.measure_text(version_text, 14.0);
                            font.draw_text(
                                surface,
                                (version_x as f32) - metrics.width,
                                version_y as f32,
                                version_text,
                                14.0,
                                Color::new(255, 255, 255, 255),
                            );
                        }
                    }
                }
            }
            StartupView::ScenarioBrowser => scenario_menu.menu().render(surface),
            StartupView::NetworkLobby => scenario_menu.menu().render(surface),
            StartupView::Options | StartupView::About => {}
        }
        if let Some(context_menu) = context_menu {
            context_menu.render_panels(surface, Some(gamma))?;
        }
    }
    let surface = graphics.surface();
    if !surface.is_gpu_scene_capture_active() {
        let pixels = surface.pixels();
        if pixels.len() == frame.len() {
            frame.copy_from_slice(pixels);
        } else {
            copy_surface(pixels, surface.width(), surface.height(), frame);
        }
    }
    Ok(())
}

pub(crate) fn copy_surface(src: &[u8], width: u32, height: u32, dest: &mut [u8]) {
    const BYTES_PER_PIXEL: usize = 4;
    if width == 0 || height == 0 {
        return;
    }
    let stride = width as usize * BYTES_PER_PIXEL;
    for row in 0..height as usize {
        let src_offset = row * stride;
        let dest_offset = row * stride;
        let end = src_offset + stride;
        if end <= src.len() && dest_offset + stride <= dest.len() {
            dest[dest_offset..dest_offset + stride].copy_from_slice(&src[src_offset..end]);
        }
    }
}

/// Converts C4GUI's 0..=100 fade value to effective opaque coverage. The
/// renderer stores the inverse byte as `((100 - fade) * 255 / 100)`.
pub(crate) fn startup_dialog_fade_opacity(fade: u8) -> u8 {
    let fade = fade.min(100);
    255 - (u16::from(100 - fade) * 255 / 100) as u8
}

/// C4GUI expresses dialog fades as one active packed-C4 blit modulation.
/// Packed alpha is transparency, so it is the inverse of straight opacity.
pub(crate) fn startup_fade_packed_modulation(opacity: u8) -> u32 {
    (u32::from(255_u8.saturating_sub(opacity)) << 24) | 0x00ff_ffff
}

pub(crate) fn apply_startup_fade_to_batch(
    batch: &mut NativePresentationBatch,
    opacity: u8,
) -> Result<()> {
    // `C4GUI::Dialog::Draw` switches to eFadeNone at the fully visible
    // endpoint and therefore does not activate even a white modulation.
    if opacity == u8::MAX {
        return Ok(());
    }
    let modulation = startup_fade_packed_modulation(opacity);
    if let Some(recorder) = batch.gpu_recorder.as_mut() {
        recorder
            .apply_packed_c4_modulation(modulation)
            .context("startup fade command is not exactly representable as packed C4 color")?;
    }
    batch.text = startup_fade_native_text(&batch.text, opacity);
    Ok(())
}

pub(crate) fn startup_fade_native_layer(pixels: &[u8], opacity: u8) -> Vec<u8> {
    pixels
        .iter()
        .map(|value| ((u16::from(*value) * u16::from(opacity) + 127) / 255) as u8)
        .collect()
}

pub(crate) fn startup_fade_native_text(
    commands: &[clonk_graphics::clonk_font::CapturedClonkText],
    opacity: u8,
) -> Vec<clonk_graphics::clonk_font::CapturedClonkText> {
    if opacity == u8::MAX {
        return commands.to_vec();
    }
    let modulation = startup_fade_packed_modulation(opacity);
    commands
        .iter()
        .cloned()
        .map(|mut command| {
            command.color =
                clonk_graphics::gpu_scene::modulate_rgba8_by_packed_c4(command.color, modulation);
            command
        })
        .collect()
}

pub(crate) fn blend_startup_dialog_frames(
    underlay: &[u8],
    outgoing: Option<&[u8]>,
    incoming: &mut [u8],
    incoming_percent: u8,
) {
    debug_assert_eq!(underlay.len(), incoming.len());
    debug_assert!(outgoing.is_none_or(|outgoing| outgoing.len() == incoming.len()));
    let incoming_opacity = u32::from(startup_dialog_fade_opacity(incoming_percent));
    let outgoing_opacity = u32::from(startup_dialog_fade_opacity(
        100_u8.saturating_sub(incoming_percent),
    ));
    for (index, new) in incoming.iter_mut().enumerate() {
        let mut composed = u32::from(underlay[index]);
        if let Some(outgoing) = outgoing {
            composed = (u32::from(outgoing[index]) * outgoing_opacity
                + composed * (255 - outgoing_opacity)
                + 127)
                / 255;
        }
        composed =
            (u32::from(*new) * incoming_opacity + composed * (255 - incoming_opacity) + 127) / 255;
        *new = composed as u8;
    }
}

pub(crate) fn scaled_viewport_extent(logical_extent: u32, scale: f32) -> Option<u32> {
    let scaled = logical_extent as f32 * scale;
    (scaled.is_finite() && scaled > 0.0 && scaled <= u32::MAX as f32).then(|| scaled.ceil() as u32)
}

/// C++ `LayoutOrder` used by `SortViewportsByPlayerControl` immediately
/// before fullscreen viewport layout (C4GraphicsSystem.cpp:422-441).
pub(crate) const fn classic_viewport_layout_order(control_set: i32) -> i32 {
    match control_set {
        0 => 0, // Keyboard1
        1 => 3, // Keyboard2
        2 => 1, // Keyboard3
        3 => 2, // Keyboard4
        _ => control_set,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PhysicalViewportState {
    /// Current `C4Viewport::Player`. `CloseViewport(player)` matches this,
    /// not the local player that originally caused the viewport to exist.
    pub(crate) displayed_player: i32,
    /// Stable graphics/camera identity retained across `Init(..., true)`.
    pub(crate) camera_identity_owner: i32,
    /// Identity of the concrete C4Viewport object, independent of reusable
    /// player numbers and ownerless viewport recreation.
    pub(crate) physical_identity: u64,
    /// Native `C4Viewport::fIsNoOwnerViewport`; film retargets preserve it.
    pub(crate) is_no_owner_viewport: bool,
    /// Rust snapshots may expose several camera slots for one local player.
    /// They remain a presentation expansion of one ordinary physical entry;
    /// replay-created/ownerless entries always project exactly one viewport.
    pub(crate) expand_player_slots: bool,
    /// True only while this is the original owned viewport presentation.
    /// A temporary Init stays temporary even if that player number is later
    /// assigned again to this same physical viewport.
    pub(crate) uses_live_player_presentation: bool,
    /// `C4Viewport::Init(..., true)` keeps these physical presentation values
    /// while switching the displayed player's center/focus.
    pub(crate) preserved_zoom: f32,
    pub(crate) preserved_offset: Vector2,
    /// `C4Viewport::PlayerLock`, set by `C4Viewport::Default`
    /// (`C4Viewport.cpp:1272`). Clearing it is how a console viewport window
    /// stops following its player so its scroll bars can move the view.
    pub(crate) player_lock: bool,
}

impl PhysicalViewportState {
    pub(crate) const fn owned(
        player: i32,
        expand_player_slots: bool,
        physical_identity: u64,
    ) -> Self {
        Self {
            displayed_player: player,
            camera_identity_owner: player,
            physical_identity,
            is_no_owner_viewport: false,
            expand_player_slots,
            uses_live_player_presentation: true,
            preserved_zoom: 1.0,
            preserved_offset: Vector2::ZERO,
            player_lock: true,
        }
    }

    pub(crate) const fn ownerless(physical_identity: u64) -> Self {
        Self {
            displayed_player: OWNER_NONE,
            camera_identity_owner: OWNER_NONE,
            physical_identity,
            is_no_owner_viewport: true,
            expand_player_slots: false,
            uses_live_player_presentation: false,
            preserved_zoom: 1.0,
            preserved_offset: Vector2::ZERO,
            player_lock: true,
        }
    }

    pub(crate) const fn matches_close(self, player: i32) -> bool {
        self.displayed_player == player || (player == OWNER_NONE && self.is_no_owner_viewport)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PhysicalViewportCloseEffect {
    pub(crate) closed_any: bool,
    pub(crate) remaining_count: usize,
}

/// Project `C4GraphicsSystem::CloseViewport(player)` over the physical
/// fullscreen viewport list. `SetFilmView` mutates the first viewport's
/// displayed player in place, so local-control ownership alone cannot tell
/// either whether a close matched or whether film fallback must recreate it.
pub(crate) fn physical_viewport_close_effect(
    local_owners_primary_first: &[i32],
    film_view_player: Option<i32>,
    closing_player: i32,
) -> PhysicalViewportCloseEffect {
    let mut closed_count = 0;
    let physical_count = local_owners_primary_first.len().max(1);
    if local_owners_primary_first.is_empty() {
        if film_view_player.unwrap_or(OWNER_NONE) == closing_player {
            closed_count = 1;
        }
    } else {
        for (index, &local_owner) in local_owners_primary_first.iter().enumerate() {
            let displayed_owner = if index == 0 {
                film_view_player.unwrap_or(local_owner)
            } else {
                local_owner
            };
            closed_count += usize::from(displayed_owner == closing_player);
        }
    }
    PhysicalViewportCloseEffect {
        closed_any: closed_count != 0,
        remaining_count: physical_count - closed_count,
    }
}

pub(crate) fn collect_viewport_inputs<'a>(
    snapshot: &'a SimulationSnapshot,
) -> std::result::Result<Vec<ViewportInput<'a>>, ClassicViewportBoundary> {
    let mut inputs = Vec::new();

    // C++ creates viewports from C4Player::LocalControl, not from the global
    // player list (C4Game.cpp:2736-2746). The snapshot's local_players list is
    // the authoritative projection of that flag. RecalculateViewports sorts
    // the resulting C++ viewport list by the player's keyboard layout before
    // assigning cells; a stable sort also keeps duplicate slots for one owner
    // in their existing order. Eliminated players retain their viewports until
    // they are actually removed.
    let mut local_players = snapshot
        .hud
        .local_players
        .iter()
        .map(|owner| {
            let state = snapshot
                .players
                .iter()
                .find(|state| state.id == *owner)
                .ok_or(ClassicViewportBoundary::LocalViewportUnavailable { owner: *owner })?;
            if state.viewports.is_empty() {
                return Err(ClassicViewportBoundary::LocalViewportUnavailable { owner: state.id });
            }
            Ok(state)
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    local_players.sort_by_key(|state| classic_viewport_layout_order(state.control_set));
    for state in local_players {
        for (slot, viewport) in state.viewports.iter().enumerate() {
            let object = viewport
                .focus
                .and_then(|focus_id| snapshot.object(focus_id))
                .or_else(|| state.cursor.and_then(|cursor| snapshot.object(cursor)))
                .or_else(|| state.crew.first().and_then(|crew| snapshot.object(*crew)));
            let center = Vector2::new(viewport.center.x, viewport.center.y);
            let input = object.map_or_else(
                || ViewportInput::owned_without_focus(state.id, center, viewport.zoom),
                |object| ViewportInput::new(state.id, center, viewport.zoom, object),
            );
            inputs.push(
                input
                    .with_offset(state.view_offset)
                    .with_scrolling(state.view_mode == PLAYER_VIEW_MODE_SCROLLING)
                    .with_camera_identity(state.id, slot),
            );
        }
    }

    if inputs.is_empty() {
        // Fullscreen C++ owns exactly one NO_OWNER observer viewport when no
        // local player has a viewport (C4FullScreen.cpp:499-535). It is
        // object-independent, including a landscape-only empty scenario.
        let center = snapshot
            .landscape
            .as_ref()
            .map(|landscape| {
                Vector2::new(
                    i32::try_from(landscape.width()).unwrap_or(i32::MAX) / 2,
                    landscape.estimated_height() / 2,
                )
            })
            .unwrap_or(Vector2::ZERO);
        inputs.push(ViewportInput::ownerless(center, 1.0).with_camera_identity(OWNER_NONE, 0));
    }

    Ok(inputs)
}

/// Project a temporary replay film target onto the existing first physical
/// viewport. Its zoom, stable camera identity, classification, and every
/// later viewport remain untouched, matching C4Viewport::Init(..., true).
pub(crate) fn collect_viewport_inputs_with_film_view<'a>(
    snapshot: &'a SimulationSnapshot,
    film_view_player: Option<i32>,
) -> std::result::Result<Vec<ViewportInput<'a>>, ClassicViewportBoundary> {
    let mut inputs = collect_viewport_inputs(snapshot)?;
    let Some(player) = film_view_player else {
        return Ok(inputs);
    };
    let primary = inputs
        .first_mut()
        .expect("collect_viewport_inputs returns at least one viewport");
    if player == OWNER_NONE {
        primary.owner = OWNER_NONE;
        return Ok(inputs);
    }

    let Some(state) = snapshot.players.iter().find(|state| state.id == player) else {
        return Ok(inputs);
    };
    let viewport = state.viewports.first();
    let object = viewport
        .and_then(|viewport| viewport.focus)
        .and_then(|focus_id| snapshot.object(focus_id))
        .or_else(|| state.cursor.and_then(|cursor| snapshot.object(cursor)))
        .or_else(|| state.crew.first().and_then(|crew| snapshot.object(*crew)));
    primary.owner = player;
    if let Some(viewport) = viewport {
        primary.center = viewport.center;
    }
    primary.set_scrolling(state.view_mode == PLAYER_VIEW_MODE_SCROLLING);
    if let Some(object) = object {
        primary.focus = Some(object);
    }
    Ok(inputs)
}

fn ownerless_physical_viewport_center(snapshot: &SimulationSnapshot) -> Vector2 {
    snapshot
        .landscape
        .as_ref()
        .map(|landscape| {
            Vector2::new(
                i32::try_from(landscape.width()).unwrap_or(i32::MAX) / 2,
                landscape.estimated_height() / 2,
            )
        })
        .unwrap_or(Vector2::ZERO)
}

fn physical_player_viewport_input<'a>(
    snapshot: &'a SimulationSnapshot,
    physical: PhysicalViewportState,
    slot: usize,
) -> std::result::Result<ViewportInput<'a>, ClassicViewportBoundary> {
    if physical.displayed_player == OWNER_NONE {
        let source = snapshot
            .players
            .iter()
            .find(|state| state.id == physical.camera_identity_owner);
        let source_viewport = source.and_then(|state| state.viewports.first());
        let center = source_viewport
            .map(|viewport| viewport.center)
            .unwrap_or_else(|| ownerless_physical_viewport_center(snapshot));
        let focus = source_viewport
            .and_then(|viewport| viewport.focus)
            .and_then(|focus| snapshot.object(focus))
            .or_else(|| {
                source.and_then(|state| state.cursor.and_then(|cursor| snapshot.object(cursor)))
            })
            .or_else(|| {
                source.and_then(|state| state.crew.first().and_then(|crew| snapshot.object(*crew)))
            });
        let mut input = if physical.is_no_owner_viewport {
            ViewportInput::ownerless(center, physical.preserved_zoom)
        } else {
            ViewportInput::owned_without_focus(OWNER_NONE, center, physical.preserved_zoom)
        };
        input.focus = focus;
        return Ok(input
            .with_offset(physical.preserved_offset)
            .with_scrolling(
                source.is_some_and(|state| state.view_mode == PLAYER_VIEW_MODE_SCROLLING),
            )
            .with_physical_camera_identity(physical.physical_identity, slot));
    }

    let state = snapshot
        .players
        .iter()
        .find(|state| state.id == physical.displayed_player)
        .ok_or(ClassicViewportBoundary::LocalViewportUnavailable {
            owner: physical.displayed_player,
        })?;
    let viewport = state
        .viewports
        .get(slot.min(state.viewports.len().saturating_sub(1)))
        .ok_or(ClassicViewportBoundary::LocalViewportUnavailable { owner: state.id })?;
    let object = viewport
        .focus
        .and_then(|focus_id| snapshot.object(focus_id))
        .or_else(|| state.cursor.and_then(|cursor| snapshot.object(cursor)))
        .or_else(|| state.crew.first().and_then(|crew| snapshot.object(*crew)));
    let use_live_player_presentation = physical.uses_live_player_presentation;
    let zoom = if use_live_player_presentation {
        viewport.zoom
    } else {
        physical.preserved_zoom
    };
    let offset = physical.preserved_offset;
    let mut input = if physical.is_no_owner_viewport {
        let mut input = ViewportInput::ownerless(viewport.center, zoom);
        input.owner = state.id;
        input
    } else {
        ViewportInput::owned_without_focus(state.id, viewport.center, zoom)
    };
    input.focus = object;
    Ok(input
        .with_offset(offset)
        .with_scrolling(state.view_mode == PLAYER_VIEW_MODE_SCROLLING)
        .with_player_lock(physical.player_lock)
        .with_physical_camera_identity(physical.physical_identity, slot))
}

/// Render the app-owned physical list without deriving membership from local
/// controls. The output Vec is the same allocation the legacy renderer
/// already performs; the lifecycle state itself is borrowed in place.
pub(crate) fn collect_viewport_inputs_from_physical_state<'a>(
    snapshot: &'a SimulationSnapshot,
    physical_viewports: &[PhysicalViewportState],
) -> std::result::Result<Vec<ViewportInput<'a>>, ClassicViewportBoundary> {
    let mut inputs = Vec::new();
    for &physical in physical_viewports {
        if physical.expand_player_slots
            && physical.uses_live_player_presentation
            && physical.displayed_player != OWNER_NONE
        {
            let state = snapshot
                .players
                .iter()
                .find(|state| state.id == physical.displayed_player)
                .ok_or(ClassicViewportBoundary::LocalViewportUnavailable {
                    owner: physical.displayed_player,
                })?;
            if state.viewports.is_empty() {
                return Err(ClassicViewportBoundary::LocalViewportUnavailable { owner: state.id });
            }
            for slot in 0..state.viewports.len() {
                inputs.push(physical_player_viewport_input(snapshot, physical, slot)?);
            }
        } else {
            inputs.push(physical_player_viewport_input(snapshot, physical, 0)?);
        }
    }
    Ok(inputs)
}

/// [`draw_commands::CommandContext`] over the live engine, the local
/// keyboard/gamepad bindings and the current snapshot.
pub(crate) struct AppCommandContext<'a> {
    pub(crate) engine: &'a Engine,
    pub(crate) bindings: &'a KeyboardBindings,
    pub(crate) gamepad_bindings: &'a GamepadBindings,
    pub(crate) snapshot: &'a SimulationSnapshot,
    pub(crate) resources: &'a HashMap<String, String>,
}

impl draw_commands::CommandContext for AppCommandContext<'_> {
    fn def_has_function(&self, definition_id: &str, function: &str) -> bool {
        self.engine
            .definition_script_has_function(definition_id, function)
    }

    fn def_picture(&self, definition_id: &str) -> Option<ImageData> {
        self.engine
            .definition_picture_image(definition_id)
            .map(definition_menu_picture)
    }

    fn def_grab_put_get(&self, definition_id: &str) -> i32 {
        self.engine.definition_grab_put_get(definition_id)
    }

    fn def_picture_phase(&self, definition_id: &str, phase: i32) -> Option<ImageData> {
        if phase <= 0 {
            return self.def_picture(definition_id);
        }
        // C4Def::Draw iPhaseX offsets Picture by phase widths and retains the
        // paired owner-color surface (src/C4Object.cpp:4055).
        self.engine
            .definition_picture_phase_image(definition_id, phase)
            .map(definition_menu_picture)
    }

    fn control_image(
        &self,
        definition_id: &str,
        function: &str,
    ) -> Option<draw_commands::ImageAnnotation> {
        // GetSFunc resolves across the #include merge, child shadowing
        // parent (C4AulScript::GetSFunc); walk the chain in that order.
        let mut stack = vec![definition_id.to_string()];
        let mut seen = std::collections::HashSet::new();
        while let Some(id) = stack.pop() {
            if !seen.insert(id.clone()) {
                continue;
            }
            if let Some(source) = self.engine.definition_script_source(&id) {
                if draw_commands::source_defines_function(source, function) {
                    return draw_commands::control_image_annotation(source, function);
                }
            }
            if let Some(includes) = self.engine.definition_includes(&id) {
                for include in includes.iter().rev() {
                    stack.push(include.clone());
                }
            }
        }
        None
    }

    fn control_description(&self, definition_id: &str, function: &str) -> Option<String> {
        self.engine
            .definition_control_description(definition_id, function)
            .map(|description| c4_presentation_text(&description))
    }

    fn object_name(&self, object: &ObjectSnapshot) -> String {
        let name = object
            .custom_name
            .as_deref()
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .or_else(|| {
                self.engine
                    .crew_object_info(object.id)
                    .map(|info| info.name.clone())
            })
            .or_else(|| {
                self.engine
                    .definition_name(&object.definition_id)
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| object.definition_id.clone());
        c4_presentation_text(&name)
    }

    fn localized_caption(&self, key: &str, fallback: &str, arguments: &[&str]) -> String {
        let template = self
            .resources
            .get(key)
            .cloned()
            .unwrap_or_else(|| fallback.to_owned());
        format_resource_string(template, arguments)
    }

    fn def_shape(&self, definition_id: &str) -> Option<clonk_engine::DefinitionRect> {
        self.engine.definition_shape_rect(definition_id)
    }

    fn key_label(&self, owner: i32, control: i32) -> String {
        // PlrControlKeyName (src/C4Viewport.cpp:1363-1374): the owning
        // player's selected keyboard set's key for the CON_* index, short
        // name. The ControlBindingId order IS the CON_* order
        // (src/C4Constants.h:158).
        let binding = usize::try_from(control)
            .ok()
            .and_then(|index| ControlBindingId::ALL.get(index).copied());
        let control_set = self
            .snapshot
            .players
            .iter()
            .find(|player| player.id == owner)
            .map(|player| player.control_set);
        binding
            .zip(control_set)
            .map(|(binding, control_set)| {
                if (0..4).contains(&control_set) {
                    self.bindings
                        .key_for_set(control_set as usize, binding)
                        .map(format_key_label)
                        .unwrap_or_default()
                } else if (4..8).contains(&control_set) {
                    self.gamepad_bindings
                        .key_label_for_set((control_set - 4) as usize, binding)
                } else {
                    String::new()
                }
            })
            .unwrap_or_default()
    }

    fn base_owner(&self, container: &ObjectSnapshot) -> Option<i32> {
        // ValidPlr(Contained->Base) gates the contained Buy/Sell commands
        // (src/C4Object.cpp:3034-3048); the engine's ExecBase now models
        // C4Object::Base.
        let base = container.base;
        self.snapshot
            .players
            .iter()
            .any(|player| player.id == base)
            .then_some(base)
    }

    fn base_sell_enabled(&self) -> bool {
        true
    }

    fn base_buy_enabled(&self) -> bool {
        true
    }

    fn owner_color(&self, owner: i32) -> Color {
        self.snapshot
            .players
            .iter()
            .find(|player| player.id == owner)
            .and_then(|player| player.color.map(|rgb| Color::opaque(rgb.r, rgb.g, rgb.b)))
            .unwrap_or_else(|| default_owner_color(owner))
    }
}

pub(crate) fn script_text_spec_resources_from_assets_and_hud<'a>(
    assets: &'a FrontendAssets,
    hud: &'a HudGraphics,
) -> ScriptTextSpecResources<'a> {
    ScriptTextSpecResources {
        gui_icons: assets.startup_dialog_images.get("GUIIcons.png"),
        gui_icons_extended: assets.startup_dialog_images.get("GUIIcons2.png"),
        score: hud.score.as_ref(),
    }
}

pub(crate) fn resolve_message_portrait(engine: &Engine, spec: &str) -> Option<ImageData> {
    resolve_message_portrait_with_color(engine, spec, 0xff)
}

pub(crate) fn resolve_message_portrait_with_color(
    engine: &Engine,
    spec: &str,
    fallback_color: u32,
) -> Option<ImageData> {
    let TextSpec::Portrait {
        definition_id,
        portrait_name,
        color,
    } = parse_text_spec(spec)?
    else {
        return None;
    };
    resolve_portrait_text_spec(engine, definition_id, portrait_name, color, fallback_color)
}

#[derive(Default)]
pub(crate) struct MessageFontImages(HashMap<String, ImageData>);

impl FontImageProvider for MessageFontImages {
    fn font_image(&self, tag: &str) -> Option<FontImageRef<'_>> {
        let image = self.0.get(tag)?;
        Some(FontImageRef {
            width: image.width(),
            height: image.height(),
            rgba: image.pixels(),
        })
    }
}

pub(crate) fn resolve_font_images_in_texts<'a>(
    engine: &Engine,
    texts: impl IntoIterator<Item = &'a str>,
    resources: ScriptTextSpecResources<'_>,
) -> MessageFontImages {
    let mut images = HashMap::new();
    for mut text in texts {
        while !text.is_empty() {
            if let Some((spec, advance)) = inline_image_token(text) {
                let tag = font_image_lookup_tag(spec);
                if !images.contains_key(tag) {
                    if let Some(image) = resolve_script_font_image(engine, tag, 0xff, resources) {
                        images.insert(tag.to_string(), image);
                    }
                }
                text = &text[advance..];
            } else {
                let character = text
                    .chars()
                    .next()
                    .expect("nonempty FontRegular image scan");
                text = &text[character.len_utf8()..];
            }
        }
    }
    MessageFontImages(images)
}

pub(crate) fn resolve_message_font_images(
    engine: &Engine,
    message: &clonk_engine::MessageSnapshot,
    resources: ScriptTextSpecResources<'_>,
) -> MessageFontImages {
    resolve_font_images_in_texts(engine, message.lines.iter().map(String::as_str), resources)
}

/// `CStdFont::DrawText` consumes `{{id}}` markup *before* it looks the image
/// up — `szText += iImgLgt + 3` — and then skips a spec it cannot resolve:
/// "image renderer not hooked or ID not found, or surface not present: just
/// ignore it" (oracle-src-pinned src/StdFont.cpp:869-890). An unresolved
/// inline image is therefore drawn as zero pixels with zero advance and the
/// row keeps drawing, so an unresolvable spec is simply absent from this map
/// rather than a refusal to draw the menu at all.
pub(crate) fn resolve_script_menu_font_images(
    engine: &Engine,
    menu: &clonk_engine::ObjectMenuState,
    resources: ScriptTextSpecResources<'_>,
) -> HashMap<String, ImageData> {
    engine_script_menu_inline_image_specs(menu)
        .into_iter()
        .filter_map(|spec| {
            resolve_script_font_image(engine, &spec, 0xff, resources).map(|image| (spec, image))
        })
        .collect()
}

/// The scoreboard's resolvable inline images.
///
/// A spec that resolves to nothing is simply left out: `CStdFont::DrawText`
/// consumes `{{...}}` markup and `continue`s when the image renderer is not
/// hooked or the id is unknown — "printing it out wouldn't look better"
/// (src/StdFont.cpp:868-890). The font layer already measures and draws an
/// absent tag that way, so an unresolved image costs the cell no pixels and no
/// advance rather than failing the frame (clonk-org/clonk-rs#1209).
pub(crate) fn resolve_scoreboard_font_images(
    engine: &Engine,
    scoreboard: &clonk_engine::ScoreboardState,
    resources: ScriptTextSpecResources<'_>,
) -> HashMap<String, ImageData> {
    clonk_frontend::scoreboard::scoreboard_inline_image_specs(scoreboard)
        .into_iter()
        .filter_map(|spec| {
            resolve_script_font_image(engine, &spec, 0xff, resources).map(|image| (spec, image))
        })
        .collect()
}

pub(crate) fn scoreboard_preferred_rect(rect: Rect) -> clonk_frontend::classic_gui::IntRect {
    clonk_frontend::classic_gui::IntRect::new(
        rect.x,
        rect.y,
        i32::try_from(rect.width).unwrap_or(i32::MAX),
        i32::try_from(rect.height).unwrap_or(i32::MAX),
    )
}

pub(crate) fn collect_player_overlays(
    engine: &mut Engine,
    snapshot: &SimulationSnapshot,
    focus_id: Option<ObjectId>,
    bindings: &KeyboardBindings,
    gamepad_bindings: &GamepadBindings,
) -> Vec<PlayerOverlay> {
    collect_player_overlays_filtered(engine, snapshot, focus_id, bindings, gamepad_bindings, None)
}

pub(crate) fn collect_speaking_overlay_objects(
    snapshot: &SimulationSnapshot,
    active_speakers: &[(i32, i32)],
) -> Vec<ObjectId> {
    let mut seen = HashSet::with_capacity(active_speakers.len());
    active_speakers
        .iter()
        .filter_map(|&(by_client, player_id)| {
            crate::voice_chat::authenticated_selected_voice_crew(snapshot, by_client, player_id)
                .map(|object| object.id)
        })
        .filter(|object_id| seen.insert(*object_id))
        .collect()
}

/// Prepare overlay state only for players presented by a physical viewport.
///
/// C4GraphicsSystem iterates its viewport list and C4Viewport::DrawOverlay
/// resolves only that viewport's Player (src/C4GraphicsSystem.cpp:167-170;
/// src/C4Viewport.cpp:836-897). An ownerless observer is the exception because
/// C4Game::DrawCursors(NO_OWNER) traverses every player
/// (src/C4Game.cpp:1852-1887).
pub(crate) fn collect_player_overlays_for_viewports(
    engine: &mut Engine,
    snapshot: &SimulationSnapshot,
    focus_id: Option<ObjectId>,
    bindings: &KeyboardBindings,
    gamepad_bindings: &GamepadBindings,
    viewports: &[ViewportInput<'_>],
) -> Vec<PlayerOverlay> {
    let visible_owners = (!viewports
        .iter()
        .any(|viewport| viewport.owner == OWNER_NONE))
    .then(|| {
        viewports
            .iter()
            .map(|viewport| viewport.owner)
            .collect::<HashSet<_>>()
    });
    collect_player_overlays_filtered(
        engine,
        snapshot,
        focus_id,
        bindings,
        gamepad_bindings,
        visible_owners.as_ref(),
    )
}

fn collect_player_overlays_filtered(
    engine: &mut Engine,
    snapshot: &SimulationSnapshot,
    focus_id: Option<ObjectId>,
    bindings: &KeyboardBindings,
    gamepad_bindings: &GamepadBindings,
    visible_owners: Option<&HashSet<i32>>,
) -> Vec<PlayerOverlay> {
    let detail_map: HashMap<_, _> = snapshot
        .players
        .iter()
        .map(|state| (state.id, state))
        .collect();
    let mut players =
        Vec::with_capacity(visible_owners.map_or(snapshot.hud.players.len(), HashSet::len));
    for player in snapshot
        .hud
        .players
        .iter()
        .filter(|player| visible_owners.is_none_or(|owners| owners.contains(&player.owner)))
    {
        let cursor = detail_map
            .get(&player.owner)
            .and_then(|state| state.view_cursor.or(state.cursor));
        let mut overlay_objects = player.crew.clone();
        if let Some(cursor) = cursor {
            if !overlay_objects.contains(&cursor) && snapshot.object(cursor).is_some() {
                overlay_objects.push(cursor);
            }
        }
        let mut crew = Vec::with_capacity(overlay_objects.len());
        for object_id in &overlay_objects {
            let (
                label,
                energy,
                energy_capacity,
                view_energy,
                magic_energy,
                magic_capacity,
                breath,
                breath_capacity,
                is_focus,
                hide_hud_elements,
                hide_hud_bars,
            ) = if let Some(object) = snapshot.object(*object_id) {
                let label = format!("{} #{}", object.definition_id, object.id.as_u64());
                let physical = engine
                    .find_object_index(object.id)
                    .map(|index| engine.object_physical(index))
                    .or(object.temporary_physical)
                    .or(object.info_physical);
                // Keep both raw operands until DrawEnergyLevelEx applies its
                // native integer BoundBy/multiply/divide sequence.
                let energy_capacity = physical.map(|physical| physical.energy).unwrap_or(0);
                let magic_capacity = physical
                    .map(|physical| physical.magic)
                    .unwrap_or(object.magic_capacity);
                let breath_capacity = physical.map(|physical| physical.breath).unwrap_or(0);
                let view_energy = engine
                    .find_object_index(object.id)
                    .map(|index| engine.object_view_energy(index))
                    .unwrap_or(0);
                let is_focus = focus_id == Some(object.id) || cursor == Some(object.id);
                let hide_hud_elements =
                    engine.definition_hide_hud_elements(object.definition_id.as_str());
                let hide_hud_bars = engine.definition_hide_hud_bars(object.definition_id.as_str());
                (
                    label,
                    object.energy,
                    energy_capacity,
                    view_energy,
                    object.magic_energy,
                    magic_capacity,
                    object.breath,
                    breath_capacity,
                    is_focus,
                    hide_hud_elements,
                    hide_hud_bars,
                )
            } else {
                let label = format!("Object #{}", object_id.as_u64());
                (label, 0, 0, 0, 0, 0, 0, 0, false, 0, 0)
            };
            crew.push(CrewOverlay {
                object_id: *object_id,
                label,
                energy,
                energy_capacity,
                view_energy,
                magic_energy,
                magic_capacity,
                breath,
                breath_capacity,
                is_focus,
                hide_hud_elements,
                hide_hud_bars,
                portrait: None,
                portrait_owner_overlay: None,
                portrait_owner_color: u32::MAX,
                rank: 0,
                rank_symbols: None,
                rank_symbol_count: None,
                info_name: None,
                rank_name: None,
                inventory: Vec::new(),
            });
        }
        // DrawPlayerInfo reads the cached Player::SelectCount populated at
        // the beginning of Player::Execute, not the live Select bits.
        let select_count = detail_map
            .get(&player.owner)
            .map(|state| state.select_count)
            .unwrap_or(0);
        let name = detail_map
            .get(&player.owner)
            .and_then(|state| {
                if state.name.trim().is_empty() {
                    None
                } else {
                    Some(c4_presentation_text(&state.name))
                }
            })
            .unwrap_or_else(|| format!("Player {}", player.owner));
        let wealth = detail_map
            .get(&player.owner)
            .map(|state| state.wealth)
            .unwrap_or(0);
        let view_wealth = detail_map
            .get(&player.owner)
            .is_some_and(|state| state.view_wealth != 0);
        let score = detail_map
            .get(&player.owner)
            .map(|state| state.value_gain)
            .unwrap_or(0);
        let view_value = engine.scenario_value_gain_enabled()
            && detail_map
                .get(&player.owner)
                .is_some_and(|state| state.view_value != 0);
        let owner_color = detail_map
            .get(&player.owner)
            .and_then(|state| state.color.map(|rgb| Color::opaque(rgb.r, rgb.g, rgb.b)))
            .unwrap_or_else(|| default_owner_color(player.owner));
        let show_control = detail_map
            .get(&player.owner)
            .map(|state| state.show_control)
            .unwrap_or(0);
        let show_control_position = detail_map
            .get(&player.owner)
            .map(|state| state.show_control_position)
            .unwrap_or(0);
        let last_com = detail_map
            .get(&player.owner)
            .map(|state| state.control.last_com)
            .unwrap_or(0);
        let control_set = detail_map
            .get(&player.owner)
            .map(|state| state.control_set)
            .unwrap_or(-1);
        let control_key_labels = ControlBindingId::ALL
            .iter()
            .take(10)
            .map(|&binding| {
                if (0..4).contains(&control_set) {
                    bindings
                        .key_for_set(control_set as usize, binding)
                        .map(format_key_label)
                        .unwrap_or_default()
                } else if (4..8).contains(&control_set) {
                    let gamepad_set = (control_set - 4) as usize;
                    gamepad_bindings.key_label_for_set(gamepad_set, binding)
                } else {
                    String::new()
                }
            })
            .collect();
        players.push(PlayerOverlay {
            owner: player.owner,
            name,
            wealth,
            view_wealth,
            score,
            view_value,
            cursor,
            captain: detail_map
                .get(&player.owner)
                .and_then(|state| state.captain),
            eliminated: player.eliminated,
            owner_color,
            select_count,
            show_startup: snapshot.hud.local_players.contains(&player.owner)
                && detail_map
                    .get(&player.owner)
                    .is_some_and(|state| state.show_startup),
            control_set,
            mouse_control: detail_map
                .get(&player.owner)
                .is_some_and(|state| state.mouse_control != 0),
            show_control,
            show_control_position,
            last_com,
            control_key_labels,
            crew_count: player.crew.len() as i32,
            crew,
            commands: Vec::new(),
            flash_command: engine
                .player(player.owner)
                .map(|player| player.flash_command())
                .unwrap_or(0),
        });
    }
    players
}

/// Populate the effective cursor's inventory presentation. C++ only reaches
/// the contents list after resolving `ViewCursor ?: Cursor`
/// (src/C4Viewport.cpp:888-917); non-cursor crew therefore retain an empty
/// presentation list.
pub(crate) fn populate_crew_inventories(
    engine: &Engine,
    snapshot: &SimulationSnapshot,
    players: &mut [PlayerOverlay],
    renderer_config: clonk_frontend::AdvancedRendererConfig,
) {
    for player in players {
        let cursor = player.cursor;
        for crew in &mut player.crew {
            crew.inventory = if cursor == Some(crew.object_id) {
                collect_crew_inventory(engine, snapshot, crew.object_id, renderer_config)
            } else {
                Vec::new()
            };
        }
    }
}

pub(crate) fn collect_crew_inventory(
    engine: &Engine,
    snapshot: &SimulationSnapshot,
    crew_id: ObjectId,
    renderer_config: clonk_frontend::AdvancedRendererConfig,
) -> Vec<InventoryOverlay> {
    let Some(crew) = snapshot.object(crew_id) else {
        return Vec::new();
    };
    let eligible = crew
        .contents
        .iter()
        .filter_map(|child_id| {
            snapshot
                .object(*child_id)
                .filter(|child| child.status.is_active())
        })
        .collect::<Vec<_>>();
    let mut groups: Vec<InventoryOverlay> = Vec::new();

    let mut chunk_start = 0usize;
    while chunk_start < eligible.len() {
        let definition_id = eligible[chunk_start].definition_id.as_str();
        let chunk_end = eligible[chunk_start..]
            .iter()
            .position(|child| child.definition_id != definition_id)
            .map(|offset| chunk_start + offset)
            .unwrap_or(eligible.len());

        for current in chunk_start..chunk_end {
            // C4ObjectListIterator first asks every earlier object in this
            // same-ID chunk whether it concatenates the candidate
            // (src/C4ObjectList.cpp:863-885).
            if (chunk_start..current)
                .any(|prior| engine.can_concat_picture_with(eligible[prior], eligible[current]))
            {
                continue;
            }
            // Its count scan reverses the call direction: each later object
            // asks whether it concatenates the representative (:887-899).
            let count = 1
                + (current + 1..chunk_end)
                    .filter(|later| {
                        engine.can_concat_picture_with(eligible[*later], eligible[current])
                    })
                    .count();
            let child = eligible[current];
            let prepared = inventory_object_picture_layers(engine, child, renderer_config);
            let (picture, picture_overlays) = prepared
                .map(|prepared| (Some(prepared.base), prepared.overlays))
                .unwrap_or_default();
            groups.push(InventoryOverlay {
                object_id: child.id,
                definition_id: child.definition_id.clone(),
                picture,
                additive: child.blit_mode & renderer_config.allowed_blit_modes & 1 != 0,
                picture_overlays,
                count,
            });
        }
        chunk_start = chunk_end;
    }

    groups
}

pub(crate) fn object_menu_buying_player_color(
    snapshot: &SimulationSnapshot,
    command_object: Option<ObjectId>,
) -> u32 {
    command_object
        .and_then(|object_id| snapshot.object(object_id))
        .and_then(|object| {
            snapshot
                .players
                .iter()
                .find(|player| player.id == object.owner)
        })
        .and_then(|player| player.color)
        .map(|color| u32::from(color.r) << 16 | u32::from(color.g) << 8 | u32::from(color.b))
        .unwrap_or(0)
}

pub(crate) fn default_owner_definition_sprite(
    image: clonk_engine::DefinitionSpriteImage,
) -> ImageData {
    let width = image.width();
    let height = image.height();
    let mask = image.color_mask();
    let mut pixels = image.into_pixels().to_vec();
    if let Some(mask) = mask {
        // FrameDecoration::SetFacetByAction draws GetBitmap() with C4's
        // default zero->blue owner color.
        apply_definition_owner_color(&mut pixels, &mask, [0, 0, 255]);
    }
    ImageData::new(width, height, pixels)
}

pub(crate) fn select_focus_candidate(
    snapshot: &SimulationSnapshot,
    preferred_owner: i32,
) -> Option<(ObjectId, i32, bool)> {
    for object in snapshot.objects.iter() {
        if is_focusable(object) && object.crew_member && object.owner == preferred_owner {
            return Some((object.id, object.owner, true));
        }
    }
    for object in snapshot.objects.iter() {
        if is_focusable(object) && object.crew_member && object.owner >= 0 {
            return Some((object.id, object.owner, true));
        }
    }
    for object in snapshot.objects.iter() {
        if is_focusable(object) && object.crew_member {
            return Some((object.id, object.owner, true));
        }
    }
    for object in snapshot.objects.iter() {
        if is_focusable(object) && object.owner >= 0 {
            return Some((object.id, object.owner, object.crew_member));
        }
    }
    for object in snapshot.objects.iter() {
        if is_focusable(object) {
            return Some((object.id, object.owner, object.crew_member));
        }
    }
    snapshot
        .objects
        .first()
        .map(|object| (object.id, object.owner, object.crew_member))
}

fn is_focusable(object: &ObjectSnapshot) -> bool {
    object.alive && object.status.is_active()
}

/// Script errors raised while executing a control are shown and survived in
/// C++ (ErrorOrWarning → C4AulExecError::show, C4AulExec.cpp:1345-1361); only
/// engine-model errors stay fatal. InvalidScriptOutput is script-caused too:
/// C++ coerces whatever a control/menu script returns
/// (static_cast<bool>, C4Object.cpp:3300,3736) and never aborts over it.
/// Returns the status-line message to show.
pub(crate) fn control_script_error_to_status(err: EngineError) -> Result<String, EngineError> {
    match err {
        EngineError::Script { ref source, .. } => Ok(format!("Script error: {err}: {source}")),
        EngineError::InvalidScriptOutput { .. } => Ok(format!("Script error: {err}")),
        other => Err(other),
    }
}

pub(crate) fn map_key_code(code: VirtualKeyCode) -> Option<KeyCode> {
    match code {
        VirtualKeyCode::Enter | VirtualKeyCode::NumpadEnter => Some(KeyCode::Enter),
        VirtualKeyCode::Escape => Some(KeyCode::Escape),
        VirtualKeyCode::Space => Some(KeyCode::Space),
        VirtualKeyCode::Tab => Some(KeyCode::Tab),
        VirtualKeyCode::ArrowUp => Some(KeyCode::Up),
        VirtualKeyCode::ArrowDown => Some(KeyCode::Down),
        VirtualKeyCode::ArrowLeft | VirtualKeyCode::Backspace => Some(KeyCode::Left),
        VirtualKeyCode::ArrowRight => Some(KeyCode::Right),
        VirtualKeyCode::Home => Some(KeyCode::Home),
        VirtualKeyCode::End => Some(KeyCode::End),
        VirtualKeyCode::PageUp => Some(KeyCode::PageUp),
        VirtualKeyCode::PageDown => Some(KeyCode::PageDown),
        _ => None,
    }
}

pub(crate) fn league_signup_dialog_key_code(
    code: VirtualKeyCode,
    modifiers: ModifiersState,
) -> Option<KeyCode> {
    match code {
        VirtualKeyCode::Enter
        | VirtualKeyCode::NumpadEnter
        | VirtualKeyCode::Escape
        | VirtualKeyCode::Space
            if modifiers.is_empty() =>
        {
            map_key_code(code)
        }
        VirtualKeyCode::Tab if modifiers.is_empty() || modifiers == ModifiersState::SHIFT => {
            Some(KeyCode::Tab)
        }
        VirtualKeyCode::ArrowUp
        | VirtualKeyCode::ArrowDown
        | VirtualKeyCode::PageUp
        | VirtualKeyCode::PageDown
            if modifiers.is_empty() =>
        {
            map_key_code(code)
        }
        _ => None,
    }
}

pub(crate) fn definition_selector_key_code(
    code: VirtualKeyCode,
) -> Option<clonk_frontend::definition_sel::DefinitionSelKey> {
    use clonk_frontend::definition_sel::DefinitionSelKey;
    match code {
        VirtualKeyCode::Enter | VirtualKeyCode::NumpadEnter => Some(DefinitionSelKey::Enter),
        VirtualKeyCode::Escape => Some(DefinitionSelKey::Escape),
        VirtualKeyCode::Space => Some(DefinitionSelKey::Space),
        VirtualKeyCode::Tab => Some(DefinitionSelKey::Tab),
        VirtualKeyCode::ArrowUp => Some(DefinitionSelKey::Up),
        VirtualKeyCode::ArrowDown => Some(DefinitionSelKey::Down),
        VirtualKeyCode::ArrowLeft => Some(DefinitionSelKey::Left),
        VirtualKeyCode::ArrowRight => Some(DefinitionSelKey::Right),
        VirtualKeyCode::PageUp => Some(DefinitionSelKey::PageUp),
        VirtualKeyCode::PageDown => Some(DefinitionSelKey::PageDown),
        VirtualKeyCode::Home => Some(DefinitionSelKey::Home),
        VirtualKeyCode::End => Some(DefinitionSelKey::End),
        VirtualKeyCode::F5 => Some(DefinitionSelKey::Refresh),
        _ => None,
    }
}

pub(crate) fn definition_selector_label_row_at(
    controller: &clonk_frontend::definition_sel::DefinitionSelController,
    layout: &clonk_frontend::definition_sel::DefinitionSelLayout,
    point: GuiPoint,
) -> Option<usize> {
    if point.x < (layout.list_client.x + layout.row_height) as f32
        || point.x >= (layout.list_client.x + layout.list_client.w) as f32
        || point.y < layout.list_client.y as f32
        || point.y >= (layout.list_client.y + layout.list_client.h) as f32
    {
        return None;
    }
    let content_y = point.y as i32 - layout.list_client.y + controller.scroll_y();
    let index = usize::try_from(content_y / layout.row_pitch.max(1)).ok()?;
    let within = content_y.rem_euclid(layout.row_pitch.max(1));
    (index < controller.rows().len() && within < layout.row_height).then_some(index)
}

pub(crate) fn context_menu_key_code(code: VirtualKeyCode) -> Option<KeyCode> {
    match code {
        VirtualKeyCode::Enter => Some(KeyCode::Enter),
        VirtualKeyCode::Escape => Some(KeyCode::Escape),
        VirtualKeyCode::Space => Some(KeyCode::Space),
        VirtualKeyCode::Tab => Some(KeyCode::Tab),
        VirtualKeyCode::ArrowUp => Some(KeyCode::Up),
        VirtualKeyCode::ArrowDown => Some(KeyCode::Down),
        VirtualKeyCode::ArrowLeft => Some(KeyCode::Left),
        VirtualKeyCode::ArrowRight => Some(KeyCode::Right),
        _ => None,
    }
}

pub(crate) fn context_menu_hotkey(code: VirtualKeyCode) -> Option<char> {
    match code {
        VirtualKeyCode::KeyA => Some('A'),
        VirtualKeyCode::KeyB => Some('B'),
        VirtualKeyCode::KeyC => Some('C'),
        VirtualKeyCode::KeyD => Some('D'),
        VirtualKeyCode::KeyE => Some('E'),
        VirtualKeyCode::KeyF => Some('F'),
        VirtualKeyCode::KeyG => Some('G'),
        VirtualKeyCode::KeyH => Some('H'),
        VirtualKeyCode::KeyI => Some('I'),
        VirtualKeyCode::KeyJ => Some('J'),
        VirtualKeyCode::KeyK => Some('K'),
        VirtualKeyCode::KeyL => Some('L'),
        VirtualKeyCode::KeyM => Some('M'),
        VirtualKeyCode::KeyN => Some('N'),
        VirtualKeyCode::KeyO => Some('O'),
        VirtualKeyCode::KeyP => Some('P'),
        VirtualKeyCode::KeyQ => Some('Q'),
        VirtualKeyCode::KeyR => Some('R'),
        VirtualKeyCode::KeyS => Some('S'),
        VirtualKeyCode::KeyT => Some('T'),
        VirtualKeyCode::KeyU => Some('U'),
        VirtualKeyCode::KeyV => Some('V'),
        VirtualKeyCode::KeyW => Some('W'),
        VirtualKeyCode::KeyX => Some('X'),
        VirtualKeyCode::KeyY => Some('Y'),
        VirtualKeyCode::KeyZ => Some('Z'),
        VirtualKeyCode::Digit0 => Some('0'),
        VirtualKeyCode::Digit1 => Some('1'),
        VirtualKeyCode::Digit2 => Some('2'),
        VirtualKeyCode::Digit3 => Some('3'),
        VirtualKeyCode::Digit4 => Some('4'),
        VirtualKeyCode::Digit5 => Some('5'),
        VirtualKeyCode::Digit6 => Some('6'),
        VirtualKeyCode::Digit7 => Some('7'),
        VirtualKeyCode::Digit8 => Some('8'),
        VirtualKeyCode::Digit9 => Some('9'),
        _ => None,
    }
}

/// First ASCII character of
/// `SDL_GetKeyName(SDL_GetKeyFromScancode(scancode))`, as used by
/// `C4GUI::Dialog::KeyHotkey`. The winit route currently retains the virtual
/// key rather than the raw SDL scancode, so this mirrors SDL's US key names
/// for every corresponding winit key.
pub(crate) fn startup_dialog_hotkey(code: VirtualKeyCode) -> Option<char> {
    context_menu_hotkey(code).or_else(|| {
        Some(match code {
            // SDL names these Application, Audio*, or AC *.
            VirtualKeyCode::ContextMenu
            | VirtualKeyCode::MediaStop
            | VirtualKeyCode::AudioVolumeMute
            | VirtualKeyCode::BrowserBack
            | VirtualKeyCode::BrowserForward
            | VirtualKeyCode::MediaTrackNext
            | VirtualKeyCode::MediaPlayPause
            | VirtualKeyCode::MediaTrackPrevious
            | VirtualKeyCode::BrowserFavorites
            | VirtualKeyCode::BrowserHome
            | VirtualKeyCode::BrowserRefresh
            | VirtualKeyCode::BrowserSearch
            | VirtualKeyCode::BrowserStop => 'A',
            VirtualKeyCode::Backspace => 'B',
            VirtualKeyCode::LaunchApp2
            | VirtualKeyCode::CapsLock
            | VirtualKeyCode::Copy
            | VirtualKeyCode::Cut
            | VirtualKeyCode::LaunchApp1 => 'C',
            VirtualKeyCode::Delete | VirtualKeyCode::ArrowDown => 'D',
            VirtualKeyCode::End | VirtualKeyCode::Escape => 'E',
            VirtualKeyCode::F1
            | VirtualKeyCode::F2
            | VirtualKeyCode::F3
            | VirtualKeyCode::F4
            | VirtualKeyCode::F5
            | VirtualKeyCode::F6
            | VirtualKeyCode::F7
            | VirtualKeyCode::F8
            | VirtualKeyCode::F9
            | VirtualKeyCode::F10
            | VirtualKeyCode::F11
            | VirtualKeyCode::F12
            | VirtualKeyCode::F13
            | VirtualKeyCode::F14
            | VirtualKeyCode::F15
            | VirtualKeyCode::F16
            | VirtualKeyCode::F17
            | VirtualKeyCode::F18
            | VirtualKeyCode::F19
            | VirtualKeyCode::F20
            | VirtualKeyCode::F21
            | VirtualKeyCode::F22
            | VirtualKeyCode::F23
            | VirtualKeyCode::F24 => 'F',
            VirtualKeyCode::Home => 'H',
            VirtualKeyCode::Insert => 'I',
            // SDL names every numeric-keypad scancode "Keypad ...".
            VirtualKeyCode::Numpad0
            | VirtualKeyCode::Numpad1
            | VirtualKeyCode::Numpad2
            | VirtualKeyCode::Numpad3
            | VirtualKeyCode::Numpad4
            | VirtualKeyCode::Numpad5
            | VirtualKeyCode::Numpad6
            | VirtualKeyCode::Numpad7
            | VirtualKeyCode::Numpad8
            | VirtualKeyCode::Numpad9
            | VirtualKeyCode::NumpadAdd
            | VirtualKeyCode::NumpadComma
            | VirtualKeyCode::NumpadDecimal
            | VirtualKeyCode::NumpadDivide
            | VirtualKeyCode::NumpadEnter
            | VirtualKeyCode::NumpadEqual
            | VirtualKeyCode::NumpadMultiply
            | VirtualKeyCode::NumpadSubtract => 'K',
            VirtualKeyCode::AltLeft
            | VirtualKeyCode::ControlLeft
            | VirtualKeyCode::ArrowLeft
            | VirtualKeyCode::ShiftLeft
            | VirtualKeyCode::SuperLeft => 'L',
            VirtualKeyCode::LaunchMail | VirtualKeyCode::MediaSelect => 'M',
            VirtualKeyCode::NumLock => 'N',
            VirtualKeyCode::PageDown
            | VirtualKeyCode::PageUp
            | VirtualKeyCode::Paste
            | VirtualKeyCode::Pause
            | VirtualKeyCode::Power
            | VirtualKeyCode::PrintScreen => 'P',
            VirtualKeyCode::AltRight
            | VirtualKeyCode::ControlRight
            | VirtualKeyCode::Enter
            | VirtualKeyCode::ArrowRight
            | VirtualKeyCode::ShiftRight
            | VirtualKeyCode::SuperRight => 'R',
            VirtualKeyCode::ScrollLock
            | VirtualKeyCode::Sleep
            | VirtualKeyCode::Space
            | VirtualKeyCode::Abort => 'S',
            VirtualKeyCode::Tab => 'T',
            VirtualKeyCode::ArrowUp => 'U',
            VirtualKeyCode::AudioVolumeDown | VirtualKeyCode::AudioVolumeUp => 'V',
            VirtualKeyCode::WakeUp => 'W',
            // SDL_GetKeyName returns punctuation or an empty name for these.
            _ => return None,
        })
    })
}

pub(crate) fn message_dialog_hotkey(code: VirtualKeyCode) -> Option<char> {
    match code {
        VirtualKeyCode::KeyA => Some('A'),
        VirtualKeyCode::KeyB => Some('B'),
        VirtualKeyCode::KeyC => Some('C'),
        VirtualKeyCode::KeyD => Some('D'),
        VirtualKeyCode::KeyE => Some('E'),
        VirtualKeyCode::KeyF => Some('F'),
        VirtualKeyCode::KeyG => Some('G'),
        VirtualKeyCode::KeyH => Some('H'),
        VirtualKeyCode::KeyI => Some('I'),
        VirtualKeyCode::KeyJ => Some('J'),
        VirtualKeyCode::KeyK => Some('K'),
        VirtualKeyCode::KeyL => Some('L'),
        VirtualKeyCode::KeyM => Some('M'),
        VirtualKeyCode::KeyN => Some('N'),
        VirtualKeyCode::KeyO => Some('O'),
        VirtualKeyCode::KeyP => Some('P'),
        VirtualKeyCode::KeyQ => Some('Q'),
        VirtualKeyCode::KeyR => Some('R'),
        VirtualKeyCode::KeyS => Some('S'),
        VirtualKeyCode::KeyT => Some('T'),
        VirtualKeyCode::KeyU => Some('U'),
        VirtualKeyCode::KeyV => Some('V'),
        VirtualKeyCode::KeyW => Some('W'),
        VirtualKeyCode::KeyX => Some('X'),
        VirtualKeyCode::KeyY => Some('Y'),
        VirtualKeyCode::KeyZ => Some('Z'),
        VirtualKeyCode::Digit0 | VirtualKeyCode::Numpad0 => Some('0'),
        VirtualKeyCode::Digit1 | VirtualKeyCode::Numpad1 => Some('1'),
        VirtualKeyCode::Digit2 | VirtualKeyCode::Numpad2 => Some('2'),
        VirtualKeyCode::Digit3 | VirtualKeyCode::Numpad3 => Some('3'),
        VirtualKeyCode::Digit4 | VirtualKeyCode::Numpad4 => Some('4'),
        VirtualKeyCode::Digit5 | VirtualKeyCode::Numpad5 => Some('5'),
        VirtualKeyCode::Digit6 | VirtualKeyCode::Numpad6 => Some('6'),
        VirtualKeyCode::Digit7 | VirtualKeyCode::Numpad7 => Some('7'),
        VirtualKeyCode::Digit8 | VirtualKeyCode::Numpad8 => Some('8'),
        VirtualKeyCode::Digit9 | VirtualKeyCode::Numpad9 => Some('9'),
        _ => None,
    }
}

pub(crate) fn menu_key_from_control_button(button: ControlButton) -> Option<KeyCode> {
    match button {
        ControlButton::Left => Some(KeyCode::Left),
        ControlButton::Right => Some(KeyCode::Right),
        ControlButton::Up => Some(KeyCode::Up),
        ControlButton::Down => Some(KeyCode::Down),
    }
}

pub(crate) fn gui_point_from_position(position: PhysicalPosition<f64>) -> GuiPoint {
    GuiPoint::new(position.x as f32, position.y as f32)
}

pub(crate) fn ingame_pointer_world_pixel(pointer: ViewportPointer) -> Vector2 {
    Vector2::new(pointer.world.x as i32, pointer.world.y as i32)
}

fn fow_object_is_closed(snapshot: &SimulationSnapshot, object: &ObjectSnapshot) -> bool {
    object
        .container
        .and_then(|container| snapshot.object(container))
        .and_then(|container| {
            snapshot
                .definition_closed_containers
                .get(&container.definition_id)
        })
        .is_some_and(|closed| *closed == 1)
}

/// Exact interaction predicate from `C4Player::FoWIsVisible`. This is
/// deliberately independent from the renderer's faded modulation map.
pub(crate) fn fow_point_is_visible(
    snapshot: &SimulationSnapshot,
    owner: i32,
    point: Vector2,
) -> bool {
    let Some(player) = snapshot.players.iter().find(|player| player.id == owner) else {
        return false;
    };
    let fow_player = snapshot.fow_players.get(&owner);
    let fallback_view_objects;
    let view_objects = if let Some(fow_player) = fow_player {
        fow_player.view_objects.as_slice()
    } else {
        fallback_view_objects = snapshot
            .objects
            .iter()
            .filter(|object| {
                object.status != clonk_engine::ObjectStatus::Deleted
                    && object.plr_view_range != 0
                    && (object.owner == owner
                        || !snapshot
                            .players
                            .iter()
                            .any(|player| player.id == object.owner))
            })
            .map(|object| object.id)
            .collect::<Vec<_>>();
        fallback_view_objects.as_slice()
    };
    let in_range = |object: &ObjectSnapshot, range: i32| {
        if fow_object_is_closed(snapshot, object) {
            return false;
        }
        let dx = i128::from(object.position.x) - i128::from(point.x);
        let dy = i128::from(object.position.y) - i128::from(point.y);
        let radius = i128::from(range).abs();
        dx * dx + dy * dy < radius * radius
    };

    let mut seen = false;
    for object in view_objects
        .iter()
        .filter_map(|object| snapshot.object(*object))
    {
        let range = object.plr_view_range;
        if !in_range(object, range) {
            continue;
        }
        if range < 0 {
            // Faded generators darken the modulation map but do not block
            // mouse interaction in FoWIsVisible.
            if object.color_modulation & 0xff00_0000 == 0 {
                return false;
            }
        } else if range > 0 {
            seen = true;
        }
    }

    if player.view_mode == PLAYER_VIEW_MODE_TARGET {
        if let Some(target) = player
            .view_target
            .or_else(|| fow_player.and_then(|fow_player| fow_player.view_target))
            .and_then(|target| snapshot.object(target))
        {
            let mut range = target.plr_view_range;
            if range == 0 {
                range = player
                    .cursor
                    .and_then(|cursor| snapshot.object(cursor))
                    .map_or(0, |cursor| cursor.plr_view_range);
            }
            if range == 0 {
                range = 500;
            }
            if in_range(target, range) {
                if range < 0 {
                    if target.color_modulation & 0xff00_0000 == 0 {
                        return false;
                    }
                } else {
                    seen = true;
                }
            }
        }
    }
    seen
}

pub(crate) fn ingame_pointer_viewport_pixel(
    pointer: ViewportPointer,
    viewport: ActiveViewportProjection,
) -> (i32, i32) {
    (
        (pointer.screen.x as i32).saturating_sub(viewport.rect.x),
        (pointer.screen.y as i32).saturating_sub(viewport.rect.y),
    )
}

pub(crate) fn build_menu_entries(
    entries: &[FrontendScenario],
    include_back: bool,
) -> Vec<ScenarioEntry> {
    let mut result = Vec::new();
    if include_back {
        result.push(ScenarioEntry {
            identifier: BACK_ENTRY_IDENTIFIER.to_string(),
            title: BACK_ENTRY_TITLE.to_string(),
            description: Some("Return to the previous folder.".to_string()),
            kind: ScenarioKind::Folder,
            is_editable: false,
            is_playable: false,
            location: None,
            preview: None,
        });
    }
    result.extend(entries.iter().map(FrontendScenario::to_ui_entry));
    result
}

pub(crate) fn build_scenario_catalog(
    entries: &[FrontendScenario],
) -> HashMap<String, FrontendScenario> {
    let mut catalog = HashMap::new();
    for entry in entries {
        insert_scenario_recursive(entry, &mut catalog);
    }
    catalog
}

fn normalized_scenario_identifier(identifier: &str) -> String {
    identifier
        .replace('\\', "/")
        .split('/')
        .filter(|component| !component.is_empty() && *component != ".")
        .collect::<Vec<_>>()
        .join("/")
        .to_ascii_lowercase()
}

pub(crate) fn resolve_next_mission_scenario(
    catalog: &HashMap<String, FrontendScenario>,
    identifier: &str,
) -> Option<FrontendScenario> {
    let requested = normalized_scenario_identifier(identifier);
    catalog
        .iter()
        .find(|(candidate, _)| normalized_scenario_identifier(candidate) == requested)
        .map(|(_, scenario)| scenario.clone())
}

fn insert_scenario_recursive(
    entry: &FrontendScenario,
    catalog: &mut HashMap<String, FrontendScenario>,
) {
    catalog
        .entry(entry.identifier.clone())
        .or_insert_with(|| entry.clone());
    for child in &entry.children {
        insert_scenario_recursive(child, catalog);
    }
}

pub(crate) fn load_install_definitions(
    engine: &mut Engine,
    paths: &AppPaths,
    audio: Option<&mut AudioContext>,
) -> Result<Option<String>, EngineError> {
    let group = match open_install_objects_group(paths) {
        Some(group) => group,
        None => {
            tracing::debug!(
                install_root = %paths.install_root().display(),
                planet = %paths.planet_dir().display(),
                "no install object definitions found; continuing with sandbox fallback"
            );
            return Ok(None);
        }
    };

    let mut seen = HashSet::new();
    let mut spawn_candidate = None;
    let audio_ptr = audio.map(NonNull::from);
    let _ =
        load_definitions_from_group(engine, &group, audio_ptr, &mut seen, &mut spawn_candidate)?;
    Ok(spawn_candidate)
}

fn open_install_objects_group(paths: &AppPaths) -> Option<Group> {
    const OBJECT_GROUP_NAMES: &[&str] = &["Objects.ocd", "Objects.c4d", "Objects.ocg"];

    let mut bases = Vec::new();
    bases.push(paths.planet_dir().to_path_buf());
    bases.push(paths.install_root().to_path_buf());
    if let Some(content) = paths.content_dir() {
        bases.push(content.to_path_buf());
    }
    bases.sort();
    bases.dedup();

    for base in &bases {
        match Group::open(base) {
            Ok(group) => {
                for name in OBJECT_GROUP_NAMES {
                    match open_child_flexible(&group, Path::new(name)) {
                        Ok(Some(child)) => return Some(child),
                        Ok(None) => {}
                        Err(err) => {
                            tracing::debug!(
                                base = %base.display(),
                                candidate = *name,
                                error = %err,
                                "error while probing install object group"
                            );
                        }
                    }
                }
            }
            Err(err) => {
                // The bases were just discovered to exist; anything but a
                // Missing race here is a real io failure worth surfacing
                // before the caller silently falls back to no definitions.
                tracing::warn!(
                    base = %base.display(),
                    error = %err,
                    "failed to open existing install definition base"
                );
            }
        }

        for name in OBJECT_GROUP_NAMES {
            let candidate = base.join(name);
            match Group::open(&candidate) {
                Ok(group) => return Some(group),
                Err(clonk_resources::GroupError::Missing(_)) => {}
                Err(err) => {
                    tracing::warn!(
                        candidate = %candidate.display(),
                        error = %err,
                        "failed to open install object group candidate"
                    );
                }
            }
        }
    }

    None
}

pub(crate) fn load_definitions_from_group(
    engine: &mut Engine,
    group: &Group,
    mut audio: Option<NonNull<AudioContext>>,
    seen: &mut HashSet<String>,
    spawn_candidate: &mut Option<String>,
) -> Result<Option<NonNull<AudioContext>>, EngineError> {
    if group.exists("Particle.txt") {
        match ResourceParticleDefinition::load(group) {
            Ok(resource) => {
                if let Err(error) = engine.register_particle_resource(&resource) {
                    tracing::warn!(
                        particle = %resource.core.name,
                        group = %group.root().display(),
                        %error,
                        "install particle definition failed to register; skipping"
                    );
                }
            }
            Err(error) => tracing::warn!(
                group = %group.root().display(),
                %error,
                "install particle definition failed to load; skipping"
            ),
        }
        if let Some(mut ptr) = audio {
            unsafe {
                ptr.as_mut().register_definition_sounds("NONE", group);
            }
        }
    } else if group.exists("DefCore.txt") {
        let loadable = match ResourceDefCore::load(group) {
            Ok(core) => {
                let valid_id = core.has_valid_id();
                if !valid_id {
                    tracing::warn!(
                        id = %core.id,
                        group = %group.root().display(),
                        "skipping install definition with invalid C4ID"
                    );
                }
                if core.needed_gfx_mode == 2 {
                    false
                } else if !valid_id {
                    if let Some(mut ptr) = audio {
                        unsafe {
                            ptr.as_mut().register_definition_sounds(&core.id, group);
                        }
                    }
                    false
                } else {
                    true
                }
            }
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    group = %group.root().display(),
                    "failed to load definition core"
                );
                if let Some(mut ptr) = audio {
                    unsafe {
                        ptr.as_mut().register_definition_sounds("NONE", group);
                    }
                }
                false
            }
        };
        if loadable {
            match ResourceDefinitionData::load(group) {
                Ok(resource) if resource.graphics_image.is_none() => {
                    tracing::warn!(
                        group = %group.root().display(),
                        error = "required Graphics.png/Graphics.bmp is missing or invalid",
                        "install definition failed to load; skipping"
                    );
                }
                Ok(resource) => {
                    let id_normalized = resource.core.id.to_ascii_lowercase();
                    if seen.insert(id_normalized) {
                        match Definition::from_resource(&resource) {
                            Ok(definition) => match engine.register_definition(definition) {
                                Ok(()) => {
                                    if resource.core.crew_member != 0 {
                                        if spawn_candidate
                                            .as_ref()
                                            .map(|existing| existing.eq_ignore_ascii_case("CLNK"))
                                            .unwrap_or(false)
                                        {
                                            // Clonk already selected; keep it.
                                        } else if resource.core.id.eq_ignore_ascii_case("CLNK")
                                            || spawn_candidate.is_none()
                                        {
                                            *spawn_candidate = Some(resource.core.id.clone());
                                        }
                                    }
                                }
                                Err(EngineError::DefinitionAlreadyExists(_)) => {}
                                Err(error) => {
                                    tracing::warn!(
                                        definition = %resource.core.id,
                                        error = ?error,
                                        "failed to register install definition"
                                    );
                                }
                            },
                            Err(error) => {
                                tracing::warn!(
                                    definition = %resource.core.id,
                                    error = ?error,
                                    "failed to compile install definition script"
                                );
                            }
                        }
                    }
                    if let Some(mut ptr) = audio {
                        unsafe {
                            ptr.as_mut()
                                .register_definition_sounds(&resource.core.id, group);
                        }
                    }
                }
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        group = %group.root().display(),
                        "failed to load definition resources"
                    );
                }
            }
        }
    } else if let Some(mut ptr) = audio {
        // C4Def::Load still calls LoadEffects for a group without DefCore so
        // pure *.c4d sound folders participate in the sample bank.
        unsafe {
            ptr.as_mut().register_definition_sounds("NONE", group);
        }
    }

    let entries = match group.entries() {
        Ok(entries) => entries,
        Err(err) => {
            tracing::warn!(
                error = %err,
                group = %group.root().display(),
                "unable to list definition contents"
            );
            return Ok(audio);
        }
    };

    for entry in entries {
        // C4DefList::Load walks only immediate C4CFN_DefFiles (`*.c4d`)
        // entries in native group order. Ordinary directories such as a
        // System.c4g must not become an accidental particle-definition root.
        if !classic_wildcard_match(b"*.c4d", &entry.name_bytes) {
            continue;
        }
        match group.open_child_entry_exact(&entry) {
            Ok(child) => {
                audio = load_definitions_from_group(engine, &child, audio, seen, spawn_candidate)?;
            }
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    path = %entry.relative_path.display(),
                    "failed to inspect nested definition group"
                );
            }
        }
    }

    Ok(audio)
}

#[derive(Clone, Copy)]
pub(crate) enum SandboxDefinitionLoad<'a> {
    None,
    InstallCatalog(&'a AppPaths),
    InstallCrew(&'a AppPaths),
}

pub(crate) fn configure_sandbox_engine(
    engine: &mut Engine,
    definition_load: SandboxDefinitionLoad<'_>,
    mut audio: Option<&mut AudioContext>,
) -> Result<String, EngineError> {
    if let Some(audio) = audio.as_deref_mut() {
        audio.set_music_playlist(None);
        audio.configure_scenario(None);
        audio.clear_object_sound_instances();
    }
    let install_paths = match definition_load {
        SandboxDefinitionLoad::InstallCatalog(paths) => {
            match load_install_definitions(engine, paths, audio.as_deref_mut()) {
                Ok(Some(spawn_definition)) => {
                    sync_engine_audio_catalogs(engine, audio.as_deref());
                    engine.set_environment(EnvironmentSettings::default());
                    engine.set_landscape(Landscape::flat(2048, DEFAULT_GROUND_HEIGHT));
                    return Ok(spawn_definition);
                }
                Ok(None) => {
                    // No install definitions found; fall back to targeted loader.
                }
                Err(error) => {
                    tracing::warn!(
                        error = ?error,
                        "encountered error while loading install definitions; trying the sandbox crew definition"
                    );
                }
            }
            Some(paths)
        }
        SandboxDefinitionLoad::InstallCrew(paths) => Some(paths),
        SandboxDefinitionLoad::None => None,
    };
    if let Some(paths) = install_paths {
        let install_definition_id = "CLNK";
        if let Some(resource_def) = try_load_install_definition(paths, install_definition_id) {
            match Definition::from_resource(&resource_def) {
                Ok(definition) => {
                    engine.register_definition(definition)?;
                    sync_engine_audio_catalogs(engine, audio.as_deref());
                    engine.set_environment(EnvironmentSettings::default());
                    engine.set_landscape(Landscape::flat(2048, DEFAULT_GROUND_HEIGHT));
                    return Ok(resource_def.core.id);
                }
                Err(err) => {
                    tracing::warn!(
                        definition = install_definition_id,
                        error = %err,
                        "failed to compile install definition; falling back to sandbox walker"
                    );
                }
            }
        }
    }

    let mut definition = Definition::from_script("Walker", "Rust Walker", walker_script())?;
    let mut actions = HashMap::new();
    actions.insert("Walk".to_string(), ActionSpec::for_procedure("Walk"));
    definition.configure_actions(Some("Walk".to_string()), actions);
    definition.set_crew_member(true);
    let profile = MovementProfile::default()
        .with_walk_speed(8)
        .with_walk_acceleration(2);
    definition.set_movement_profile(profile);
    engine.register_definition(definition)?;
    sync_engine_audio_catalogs(engine, audio.as_deref());
    engine.set_environment(EnvironmentSettings::default());
    engine.set_landscape(Landscape::flat(2048, DEFAULT_GROUND_HEIGHT));
    Ok("Walker".to_string())
}

fn sync_engine_audio_catalogs(engine: &mut Engine, audio: Option<&AudioContext>) {
    engine.configure_sound_samples(
        audio
            .map(AudioContext::available_sound_samples)
            .unwrap_or_default(),
    );
    engine.configure_music_tracks(
        audio
            .map(AudioContext::available_music_tracks)
            .unwrap_or_default(),
    );
}

fn try_load_install_definition(
    paths: &AppPaths,
    definition_id: &str,
) -> Option<ResourceDefinitionData> {
    let objects_group = match open_install_objects_group(paths) {
        Some(group) => group,
        None => {
            tracing::debug!(
                definition = definition_id,
                "install object group unavailable; cannot load real definition"
            );
            return None;
        }
    };

    #[cfg(test)]
    {
        // The repository's ordinary sandbox crew is a stable test resource.
        // Open its canonical group directly before the recursive resolver so
        // every isolated app test does not walk all of Objects.c4d. Restrict
        // this shortcut to the stock test install: production and custom
        // fixture installs retain the resolver's first-match precedence.
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent);
        if repository == Some(paths.install_root()) && definition_id.eq_ignore_ascii_case("CLNK") {
            let relative_path = Path::new("Crew.c4d/Clonk.c4d");
            if let Ok(group) = objects_group.open_child(relative_path) {
                let eligible = !group.exists("Particle.txt")
                    && ResourceDefCore::load(&group).is_ok_and(|core| {
                        core.has_valid_id()
                            && core.needed_gfx_mode != 2
                            && core.id.eq_ignore_ascii_case(definition_id)
                    });
                if eligible {
                    match ResourceDefinitionData::load(&group) {
                        Ok(definition) if definition.graphics_image.is_some() => {
                            return Some(definition);
                        }
                        Ok(_) => {}
                        Err(error) => {
                            tracing::debug!(
                                definition = definition_id,
                                path = %relative_path.display(),
                                error = %error,
                                "canonical install definition lookup failed; using recursive fallback"
                            );
                        }
                    }
                }
            }
        }
    }

    match find_definition_in_group(&objects_group, definition_id) {
        Ok(result) => result,
        Err(err) => {
            tracing::warn!(
                definition = definition_id,
                error = %err,
                "error while searching for definition in install data"
            );
            None
        }
    }
}

pub(crate) fn find_definition_in_group(
    group: &Group,
    definition_id: &str,
) -> Result<Option<ResourceDefinitionData>, ResourceDefinitionError> {
    for entry in group.entries()? {
        if !entry.is_directory {
            continue;
        }
        let child = group.open_child(&entry.relative_path)?;
        if !child.exists("Particle.txt") {
            match ResourceDefCore::load(&child) {
                Ok(core) => {
                    if core.has_valid_id()
                        && core.needed_gfx_mode != 2
                        && core.id.eq_ignore_ascii_case(definition_id)
                    {
                        match ResourceDefinitionData::load(&child) {
                            Ok(definition) if definition.graphics_image.is_some() => {
                                return Ok(Some(definition));
                            }
                            Ok(_) => {}
                            Err(error) if is_rejected_install_definition_error(&error) => {}
                            Err(error) => return Err(error),
                        }
                    }
                }
                Err(ResourceDefinitionError::DefCoreMissing) => {}
                Err(ResourceDefinitionError::Resources(err)) => match err {
                    GroupError::EntryNotFound(_) => {}
                    other => return Err(ResourceDefinitionError::Resources(other)),
                },
                Err(error) if is_rejected_install_definition_error(&error) => {}
                Err(other) => return Err(other),
            }
        }
        if let Some(found) = find_definition_in_group(&child, definition_id)? {
            return Ok(Some(found));
        }
    }
    Ok(None)
}

fn is_rejected_install_definition_error(error: &ResourceDefinitionError) -> bool {
    matches!(
        error,
        ResourceDefinitionError::MissingDefCoreField(_)
            | ResourceDefinitionError::InvalidCategoryValue(_)
            | ResourceDefinitionError::DefCoreParse(_)
            | ResourceDefinitionError::ActMapParse(_)
            | ResourceDefinitionError::Graphics { .. }
            | ResourceDefinitionError::ColorByOwnerOverlay { .. }
    )
}

/// With available configuration, uses the component-language sequence
/// materialized by `C4ConfigGeneral::DefaultLanguage`; any US/DE fallbacks
/// then exist only when the options dialog persisted them into `LanguageEx`.
pub(crate) fn startup_language_sequence(paths: Option<&AppPaths>) -> Vec<String> {
    if let Some(paths) = paths {
        if let Ok(codes) = classic_loader_language_sequence(paths) {
            return codes;
        }
    }

    // Retain the historical best-effort bootstrap when no strict loader
    // configuration is available (including pathless test/dev contexts).
    let mut codes: Vec<String> = Vec::new();
    let push_code = |codes: &mut Vec<String>, segment: &str| {
        let code: String = segment
            .chars()
            .filter(char::is_ascii_alphabetic)
            .take(2)
            .map(|ch| ch.to_ascii_uppercase())
            .collect();
        if code.len() == 2 && !codes.contains(&code) {
            codes.push(code);
        }
    };

    let config =
        paths.and_then(|paths| clonk_core::std_config::Config::load(paths.config_file()).ok());
    if let Some(config) = config.as_ref() {
        if let Some(sequence) = config
            .get_in(Some("General"), "LanguageEx")
            .or_else(|| config.get("LanguageEx"))
        {
            for segment in sequence.split(',') {
                push_code(&mut codes, segment);
            }
        }
        if codes.is_empty() {
            if let Some(primary) = config
                .get_in(Some("General"), "Language")
                .or_else(|| config.get("Language"))
            {
                push_code(&mut codes, primary);
            }
        }
    }
    if codes.is_empty() {
        for key in ["LC_LANGUAGE", "LC_ALL", "LANG"] {
            if let Ok(value) = std::env::var(key) {
                push_code(&mut codes, &value);
                if !codes.is_empty() {
                    break;
                }
            }
        }
    }
    // Internal fallbacks (C4StartupOptionsDlg.cpp:1221-1231).
    for fallback in ["US", "DE"] {
        push_code(&mut codes, fallback);
    }
    codes
}

pub(crate) fn scenario_title_language(paths: Option<&AppPaths>) -> String {
    paths
        .and_then(|paths| Config::load(paths.config_file()).ok())
        .and_then(|config| {
            config
                .get_in(Some("General"), "Language")
                .or_else(|| config.get("Language"))
                .map(str::trim)
                .filter(|language| !language.is_empty())
                .map(str::to_string)
                .or_else(|| {
                    config
                        .get_in(Some("General"), "LanguageEx")
                        .or_else(|| config.get("LanguageEx"))
                        .and_then(|sequence| sequence.split(',').next())
                        .map(str::trim)
                        .filter(|language| !language.is_empty())
                        .map(|language| language.chars().take(2).collect())
                })
        })
        // `/Language:` temporarily replaces LanguageEx for resource lookup,
        // but scenario-title persistence still follows General.Language. If
        // neither persisted field exists, retain C4Config's system-language
        // default without reintroducing the command-line override here.
        .or_else(|| startup_language_sequence(None).into_iter().next())
        .unwrap_or_else(|| "US".to_string())
}

pub(crate) fn load_frontend_scenarios() -> Vec<FrontendScenario> {
    match AppPaths::discover() {
        Ok(paths) => return load_frontend_scenarios_from_paths(&paths),
        Err(err) => tracing::error!(
            error = %err,
            "app paths discovery failed; no synthetic scenario will be exposed in menus"
        ),
    }

    tracing::error!(
        "no classic scenarios were discovered; keeping the player-facing catalog empty"
    );
    Vec::new()
}

pub(crate) fn load_frontend_scenarios_from_paths(paths: &AppPaths) -> Vec<FrontendScenario> {
    load_frontend_scenarios_from_paths_with_progress(paths, |_| true).unwrap_or_default()
}

pub(crate) fn load_frontend_scenarios_from_paths_with_progress<F>(
    paths: &AppPaths,
    mut report_progress: F,
) -> Option<Vec<FrontendScenario>>
where
    F: FnMut(u8) -> bool,
{
    let alphabetical_sorting = load_startup_alphabetical_sorting(Some(paths));
    let languages = startup_language_sequence(Some(paths));
    let language_packs = classic_language_packs(paths);
    let roots = scenario_roots(paths)
        .into_iter()
        .filter(|root| root.path.exists())
        .collect::<Vec<_>>();
    if !report_progress(0) {
        return None;
    }
    let mut combined_entries: Vec<(resource_scenario::ScenarioEntry, String)> = Vec::new();
    let root_count = roots.len().max(1);
    let mut emitted_percent = 0_u8;
    for (root_index, root) in roots.iter().enumerate() {
        let root_base = root_index.saturating_mul(100);
        let mut report_root_progress = |progress: resource_scenario::ScenarioDiscoveryProgress| {
            let combined = (root_base.saturating_add(usize::from(progress.percent()))) / root_count;
            let combined = u8::try_from(combined.min(100)).unwrap_or(100);
            emitted_percent = emitted_percent.max(combined);
            if report_progress(emitted_percent) {
                OpsControlFlow::Continue(())
            } else {
                OpsControlFlow::Break(())
            }
        };
        let result = if root.include_container {
            resource_scenario::discover_entry_with_languages_and_packs_with_progress(
                &root.path,
                &languages,
                &language_packs,
                &mut report_root_progress,
            )
            .map(|entry| entry.into_iter().collect())
        } else {
            resource_scenario::discover_with_languages_and_packs_with_progress(
                &root.path,
                &languages,
                &language_packs,
                &mut report_root_progress,
            )
        };
        match result {
            Ok(entries) => combined_entries
                .extend(entries.into_iter().map(|entry| (entry, root.label.clone()))),
            Err(resource_scenario::ScenarioDiscoveryError::Cancelled) => return None,
            Err(err) => tracing::warn!(
                error = %err,
                path = %root.path.display(),
                "failed to discover scenarios from install root"
            ),
        }
    }
    if !report_progress(100) {
        return None;
    }
    Some(merge_frontend_scenarios(
        combined_entries
            .into_iter()
            .map(|(entry, label)| FrontendScenario::from_resource(entry, &label))
            .collect(),
        alphabetical_sorting,
    ))
}

/// Resolves the physical C4Group file and child path represented by a
/// scenario-discovery path. Children of packed folders deliberately look
/// like ordinary joined paths even though they do not exist in the host FS.
pub(crate) fn scenario_logical_storage(path: &Path) -> Result<(PathBuf, Vec<String>)> {
    if path.exists() {
        // RegisterParentFolders treats an immediately enclosing `.c4f` as a
        // true mother group even when both it and the scenario are unpacked
        // directories. Trace the consecutive c4f chain back to its physical
        // root so SaveGame sees the same child relationship as C4Group.
        let is_c4f = |candidate: &Path| {
            candidate
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("c4f"))
        };
        if let Some(parent) = path.parent().filter(|parent| is_c4f(parent)) {
            let mut physical = parent.to_path_buf();
            while let Some(parent) = physical.parent().filter(|parent| is_c4f(parent)) {
                physical = parent.to_path_buf();
            }
            let children = path
                .strip_prefix(&physical)
                .expect("physical parent was selected from the scenario path")
                .components()
                .filter_map(|component| match component {
                    std::path::Component::Normal(name) => {
                        Some(name.to_str().map(str::to_owned).ok_or_else(|| {
                            anyhow!("scenario path has no UTF-8 group entry: {}", path.display())
                        }))
                    }
                    _ => None,
                })
                .collect::<Result<Vec<_>>>()?;
            return Ok((physical, children));
        }
        return Ok((path.to_path_buf(), Vec::new()));
    }
    let mut ancestor = path.to_path_buf();
    let mut children = Vec::new();
    while !ancestor.exists() {
        let name = ancestor
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| anyhow!("scenario path has no UTF-8 group entry: {}", path.display()))?;
        children.push(name.to_string());
        ancestor = ancestor
            .parent()
            .ok_or_else(|| anyhow!("scenario path has no physical ancestor: {}", path.display()))?
            .to_path_buf();
    }
    anyhow::ensure!(
        ancestor.is_file(),
        "logical scenario child is not inside a packed group: {}",
        path.display()
    );
    children.reverse();
    Ok((ancestor, children))
}

fn mutable_group_descend<'a>(
    group: &'a mut MutableGroup,
    children: &[String],
) -> Result<&'a mut MutableGroup> {
    let Some((child_name, rest)) = children.split_first() else {
        return Ok(group);
    };
    match group.child_mut(child_name)? {
        MutableGroupChildMut::Child(child) => mutable_group_descend(child, rest),
        MutableGroupChildMut::Missing => {
            Err(anyhow!("C4Group child `{child_name}` does not exist"))
        }
        MutableGroupChildMut::File => Err(anyhow!("C4Group entry `{child_name}` is not a group")),
    }
}

pub(crate) fn scenario_filename_from_title(
    title: &str,
    kind: ScenarioKind,
    old_path: &Path,
) -> String {
    const STRIP: &[char] = &[
        '!', '"', '§', '%', '&', '/', '=', '?', '+', '*', '#', ':', ';', '<', '>', '\\', '.',
    ];
    let mut filename = title
        .chars()
        .skip_while(|character| character.is_whitespace())
        .filter(|character| !STRIP.contains(character))
        .collect::<String>();
    filename = filename.trim_end_matches(char::is_whitespace).to_string();
    if filename.is_empty() {
        filename.push_str("unnamed");
    }
    let extension = match kind {
        ScenarioKind::Scenario => Some("c4s"),
        ScenarioKind::Folder
            if old_path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("c4f")) =>
        {
            Some("c4f")
        }
        _ => None,
    };
    if let Some(extension) = extension {
        filename.push('.');
        filename.push_str(extension);
    }
    filename
}

static NEXT_GROUP_REWRITE: AtomicU64 = AtomicU64::new(0);

pub(crate) fn replace_file_from_same_directory(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("rewrite target has no parent: {}", path.display()))?;
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("group");
    let permissions = fs::symlink_metadata(path)
        .ok()
        .filter(|metadata| metadata.file_type().is_file())
        .map(|metadata| metadata.permissions());
    let mut last_error = None;
    for _ in 0..100 {
        let nonce = NEXT_GROUP_REWRITE.fetch_add(1, AtomicOrdering::Relaxed);
        let temporary = parent.join(format!(
            ".{filename}.lc-rewrite-{}-{nonce}",
            std::process::id()
        ));
        let mut file = match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                last_error = Some(error);
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        let result = (|| -> io::Result<()> {
            file.write_all(bytes)?;
            file.sync_all()?;
            if let Some(permissions) = permissions.clone() {
                file.set_permissions(permissions)?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            let _ = remove_file_or_directory(&temporary);
            return Err(error.into());
        }
        drop(file);
        if let Err(error) = commit_staged_file_rewrite(&temporary, path) {
            let _ = remove_file_or_directory(&temporary);
            return Err(error);
        }
        return Ok(());
    }
    Err(last_error
        .unwrap_or_else(|| io::Error::new(io::ErrorKind::AlreadyExists, "temporary rewrite path"))
        .into())
}

fn console_save_ignored_directory_entry(name: &[u8]) -> bool {
    (name.first() == Some(&b'.') && name != b".legacyclonk")
        || name.eq_ignore_ascii_case(b"cvs")
        || name.eq_ignore_ascii_case(b"Thumbs.db")
}

pub(crate) fn remove_file_or_directory(path: &Path) -> io::Result<()> {
    #[cfg(windows)]
    fn clear_readonly(path: &Path) -> io::Result<()> {
        let metadata = fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink() {
            return Ok(());
        }
        let mut permissions = metadata.permissions();
        if permissions.readonly() {
            // Clearing `FILE_ATTRIBUTE_READONLY` is exactly what this helper
            // exists to do, and it is compiled only for Windows, where that
            // attribute is the entire permission being changed.
            #[allow(clippy::permissions_set_readonly_false)]
            permissions.set_readonly(false);
            fs::set_permissions(path, permissions)?;
        }
        if metadata.file_type().is_dir() {
            for entry in fs::read_dir(path)? {
                clear_readonly(&entry?.path())?;
            }
        }
        Ok(())
    }

    #[cfg(windows)]
    clear_readonly(path)?;
    let file_type = fs::symlink_metadata(path)?.file_type();
    if file_type.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

/// `EraseItem` (src/StdFile.cpp:642-658): remove an existing target of either
/// kind, ignoring a target that was not there.
pub(crate) fn erase_item(path: &Path) {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return;
    };
    let _ = if metadata.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };
}

pub(crate) fn copy_file_or_directory(source: &Path, destination: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(source)?;
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        let target = fs::read_link(source)?;
        #[cfg(unix)]
        {
            return std::os::unix::fs::symlink(target, destination);
        }
        #[cfg(windows)]
        {
            return if fs::metadata(source)?.is_dir() {
                std::os::windows::fs::symlink_dir(target, destination)
            } else {
                std::os::windows::fs::symlink_file(target, destination)
            };
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = (target, destination);
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "copying a symbolic link is unsupported on this platform",
            ));
        }
    }
    if file_type.is_dir() {
        fs::create_dir(destination)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy_file_or_directory(&entry.path(), &destination.join(entry.file_name()))?;
        }
        return Ok(());
    }
    if file_type.is_file() {
        fs::copy(source, destination)?;
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        format!("unsupported directory entry: {}", source.display()),
    ))
}

pub(crate) fn copy_directory_contents(source: &Path, destination: &Path) -> Result<()> {
    for entry in
        fs::read_dir(source).with_context(|| format!("read folder group {}", source.display()))?
    {
        let entry = entry?;
        copy_file_or_directory(&entry.path(), &destination.join(entry.file_name()))
            .with_context(|| format!("copy folder-group entry {}", entry.path().display()))?;
    }
    Ok(())
}

fn restore_directory_permissions(source: &Path, destination: &Path) -> io::Result<()> {
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let metadata = fs::symlink_metadata(&source_path)?;
        if !metadata.file_type().is_dir() {
            continue;
        }
        let destination_path = destination.join(entry.file_name());
        let Ok(destination_metadata) = fs::symlink_metadata(&destination_path) else {
            continue;
        };
        if !destination_metadata.file_type().is_dir() {
            continue;
        }
        restore_directory_permissions(&source_path, &destination_path)?;
        fs::set_permissions(&destination_path, metadata.permissions())?;
    }
    Ok(())
}

pub(crate) fn create_sibling_rewrite_directory(parent: &Path, filename: &str) -> Result<PathBuf> {
    let mut last_error = None;
    for _ in 0..100 {
        let nonce = NEXT_GROUP_REWRITE.fetch_add(1, AtomicOrdering::Relaxed);
        let temporary = parent.join(format!(
            ".{filename}.lc-rewrite-{}-{nonce}",
            std::process::id()
        ));
        match fs::create_dir(&temporary) {
            Ok(()) => return Ok(temporary),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                last_error = Some(error);
            }
            Err(error) => return Err(error.into()),
        }
    }
    Err(last_error
        .unwrap_or_else(|| io::Error::new(io::ErrorKind::AlreadyExists, "temporary rewrite path"))
        .into())
}

fn unused_sibling_rewrite_path(parent: &Path, filename: &str) -> Result<PathBuf> {
    for _ in 0..100 {
        let nonce = NEXT_GROUP_REWRITE.fetch_add(1, AtomicOrdering::Relaxed);
        let candidate = parent.join(format!(
            ".{filename}.lc-rewrite-backup-{}-{nonce}",
            std::process::id()
        ));
        match fs::symlink_metadata(&candidate) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(candidate),
            Ok(_) => {}
            Err(error) => return Err(error.into()),
        }
    }
    Err(io::Error::new(io::ErrorKind::AlreadyExists, "backup rewrite path").into())
}

pub(crate) fn commit_staged_path_with_backup(
    staged: &Path,
    destination: &Path,
    after_backup: impl FnOnce() -> io::Result<()>,
) -> Result<()> {
    let parent = destination
        .parent()
        .ok_or_else(|| anyhow!("rewrite target has no parent: {}", destination.display()))?;
    let filename = destination
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("group");
    let backup = match fs::symlink_metadata(destination) {
        Ok(_) => {
            let backup = unused_sibling_rewrite_path(parent, filename)?;
            fs::rename(destination, &backup)
                .with_context(|| format!("stage existing group {}", destination.display()))?;
            Some(backup)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };

    let committed = after_backup().and_then(|()| fs::rename(staged, destination));
    if let Err(error) = committed {
        if let Some(backup) = backup.as_ref() {
            if let Err(rollback_error) = fs::rename(backup, destination) {
                return Err(anyhow!(
                    "commit group {} failed: {error}; restoring the original from {} failed: {rollback_error}",
                    destination.display(),
                    backup.display()
                ));
            }
        }
        return Err(error).with_context(|| format!("commit group {}", destination.display()));
    }

    if let Some(backup) = backup {
        remove_file_or_directory(&backup)
            .with_context(|| format!("remove replaced group {}", backup.display()))?;
    }
    Ok(())
}

fn commit_staged_file_rewrite(staged: &Path, destination: &Path) -> Result<()> {
    #[cfg(not(windows))]
    {
        match fs::symlink_metadata(destination) {
            Ok(metadata) if metadata.file_type().is_dir() => {
                return commit_staged_path_with_backup(staged, destination, || Ok(()));
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        fs::rename(staged, destination)?;
        Ok(())
    }
    #[cfg(windows)]
    {
        // std::fs::rename cannot replace an existing path on Windows. Move
        // the old file aside and restore it if the staged commit fails.
        commit_staged_path_with_backup(staged, destination, || Ok(()))
    }
}

fn replace_directory_from_same_parent(source: &Group, destination: &Path) -> Result<()> {
    replace_directory_from_same_parent_with_hook(source, destination, || Ok(()))
}

/// Stage a complete folder group before moving the admitted profile aside.
/// The hook exists only so tests can force the narrow two-rename failure
/// window and prove that the old directory is restored intact.
pub(crate) fn replace_directory_from_same_parent_with_hook(
    source: &Group,
    destination: &Path,
    after_backup: impl FnOnce() -> io::Result<()>,
) -> Result<()> {
    let parent = destination
        .parent()
        .ok_or_else(|| anyhow!("rewrite target has no parent: {}", destination.display()))?;
    fs::create_dir_all(parent)
        .with_context(|| format!("create save parent {}", parent.display()))?;
    let filename = destination
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("group");
    let staged = create_sibling_rewrite_directory(parent, filename)?;
    let prepared = (|| -> Result<()> {
        if destination.is_dir() {
            // C4Player::Save copies the complete admitted local group before
            // mutating it. Seed the sibling stage likewise so ignored files,
            // executable bits and other unrelated entries survive.
            copy_directory_contents(destination, &staged)?;
        }
        sync_console_save_group_directory(source, &staged)?;
        if destination.is_dir() {
            restore_directory_permissions(destination, &staged)?;
            if let Ok(metadata) = fs::metadata(destination) {
                fs::set_permissions(&staged, metadata.permissions())?;
            }
        }
        Ok(())
    })();
    if let Err(error) = prepared {
        let _ = remove_file_or_directory(&staged);
        return Err(error);
    }
    let committed = commit_staged_path_with_backup(&staged, destination, after_backup);
    if committed.is_err() {
        let _ = remove_file_or_directory(&staged);
    }
    committed
}

/// Reconcile a folder-backed C4Group with an already-mutated in-memory group.
/// Hidden/CVS metadata ignored by C4Group remains untouched, while every
/// visible file and child follows the same case-insensitive replacement and
/// deletion view that a packed-group rewrite would expose.
fn sync_console_save_group_directory(source: &Group, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)
        .with_context(|| format!("create scenario directory {}", destination.display()))?;
    let desired = source.entries()?;
    let desired_names = desired
        .iter()
        .map(|entry| entry.name_bytes.clone())
        .collect::<Vec<_>>();

    for existing in fs::read_dir(destination)
        .with_context(|| format!("read scenario directory {}", destination.display()))?
    {
        let existing = existing?;
        let bytes = path_to_legacy_bytes(Path::new(&existing.file_name()));
        if console_save_ignored_directory_entry(&bytes)
            || desired_names
                .iter()
                .any(|desired| desired.eq_ignore_ascii_case(&bytes))
        {
            continue;
        }
        let path = existing.path();
        remove_file_or_directory(&path)?;
    }

    for entry in desired {
        let requested = path_from_group_name_bytes(&entry.name_bytes);
        let target = destination.join(&requested);
        let existing = fs::read_dir(destination)?.find_map(|candidate| {
            let candidate = candidate.ok()?;
            let bytes = path_to_legacy_bytes(Path::new(&candidate.file_name()));
            bytes
                .eq_ignore_ascii_case(&entry.name_bytes)
                .then_some(candidate.path())
        });
        if let Some(existing) = existing.as_ref().filter(|existing| **existing != target) {
            remove_file_or_directory(existing)?;
        }

        if entry.is_directory {
            match fs::symlink_metadata(&target) {
                Ok(metadata) if !metadata.file_type().is_dir() => {
                    remove_file_or_directory(&target)?;
                }
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            let child = source.open_child(&entry.relative_path)?;
            sync_console_save_group_directory(&child, &target)?;
        } else {
            match fs::symlink_metadata(&target) {
                Ok(metadata) if metadata.file_type().is_dir() => {
                    remove_file_or_directory(&target)?;
                }
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            let payload = source.read_entry_bytes_exact(&entry)?;
            replace_file_from_same_directory(&target, &payload)?;
            #[cfg(unix)]
            if entry.executable && !entry.is_directory {
                use std::os::unix::fs::PermissionsExt;

                let mut permissions = fs::metadata(&target)?.permissions();
                permissions.set_mode(permissions.mode() | 0o111);
                fs::set_permissions(&target, permissions)?;
            }
        }
    }
    Ok(())
}

/// Replay the operations which `C4Group` performs immediately for an open
/// `GRPF_Folder`. Unlike packed groups, a folder save has no close-time
/// transaction: a late failure leaves every earlier truncate and deletion on
/// disk, and child buffers are gzip-wrapped files rather than subdirectories.
pub(crate) fn replay_folder_save_journal(
    journal: &developer_console_save::FolderSaveJournal,
    destination: &Path,
    maker: &[u8],
) -> Result<()> {
    anyhow::ensure!(
        destination.is_dir(),
        "folder save target is not a directory: {}",
        destination.display()
    );
    for mutation in journal.mutations() {
        use developer_console_save::{FolderSaveAddFailure, FolderSaveMutation};

        let (result, failure) = match mutation {
            FolderSaveMutation::DeletePattern { pattern } => {
                if let Err(error) = delete_folder_save_pattern(destination, pattern) {
                    // All live-save Delete results are either explicitly
                    // ignored or flow through a no-fail component host.
                    tracing::warn!(
                        %error,
                        pattern = %String::from_utf8_lossy(pattern),
                        "folder-backed live-save deletion failed"
                    );
                }
                continue;
            }
            FolderSaveMutation::DeleteEntry { name } => {
                if let Err(error) = delete_folder_save_entry(destination, name) {
                    tracing::warn!(
                        %error,
                        entry = %String::from_utf8_lossy(name),
                        "folder-backed live-save entry deletion failed"
                    );
                }
                continue;
            }
            FolderSaveMutation::PutFile {
                name,
                payload,
                failure,
            } => (
                write_folder_save_entry(destination, name, payload),
                *failure,
            ),
            FolderSaveMutation::PutChild {
                name,
                raw_image,
                failure,
            } => (
                pack_folder_save_child(raw_image, maker)
                    .and_then(|packed| write_folder_save_entry(destination, name, &packed)),
                *failure,
            ),
            FolderSaveMutation::MergeMaterialGroup { raw_patch } => (
                merge_folder_save_material(destination, raw_patch, maker),
                FolderSaveAddFailure::Fatal,
            ),
        };
        if let Err(error) = result {
            match failure {
                FolderSaveAddFailure::Fatal => return Err(error),
                FolderSaveAddFailure::Ignore => {
                    tracing::warn!(%error, "ignored folder-backed live-save add failure");
                }
            }
        }
    }
    Ok(())
}

fn folder_save_entry_path(root: &Path, name: &[u8]) -> Result<PathBuf> {
    let relative = path_from_group_name_bytes(name);
    anyhow::ensure!(
        relative.components().count() == 1
            && relative
                .components()
                .all(|component| matches!(component, Component::Normal(_))),
        "unsafe folder-group entry name: {}",
        String::from_utf8_lossy(name)
    );
    Ok(root.join(relative))
}

pub(crate) fn write_folder_save_entry(root: &Path, name: &[u8], payload: &[u8]) -> Result<()> {
    let path = folder_save_entry_path(root, name)?;
    write_folder_save_path(&path, payload)
}

fn write_folder_save_path(path: &Path, payload: &[u8]) -> Result<()> {
    // CStdFile::Create opens the final folder path with truncation. Do not use
    // a sibling temporary: a short write must leave its physical prefix.
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .with_context(|| format!("create folder-group entry {}", path.display()))?;
    file.write_all(payload)
        .with_context(|| format!("write folder-group entry {}", path.display()))?;
    Ok(())
}

pub(crate) fn delete_folder_save_entry(root: &Path, name: &[u8]) -> Result<()> {
    let path = folder_save_entry_path(root, name)?;
    match remove_file_or_directory(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => {
            Err(error).with_context(|| format!("delete folder-group entry {}", path.display()))
        }
    }
}

fn delete_folder_save_pattern(root: &Path, pattern: &[u8]) -> Result<()> {
    let separator = if pattern.contains(&b';') {
        Some(b';')
    } else if pattern.contains(&b'|') {
        Some(b'|')
    } else {
        None
    };
    if let Some(separator) = separator {
        for segment in pattern.split(|byte| *byte == separator) {
            if let Err(error) = delete_folder_save_pattern_segment(root, segment) {
                tracing::warn!(
                    %error,
                    pattern = %String::from_utf8_lossy(segment),
                    "folder-group deletion segment failed"
                );
            }
        }
        return Ok(());
    }
    delete_folder_save_pattern_segment(root, pattern)
}

fn delete_folder_save_pattern_segment(root: &Path, pattern: &[u8]) -> Result<()> {
    let entries = fs::read_dir(root)
        .with_context(|| format!("enumerate folder-group target {}", root.display()))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for entry in entries {
        let name = path_to_legacy_bytes(Path::new(&entry.file_name()));
        if console_save_ignored_directory_entry(&name) || !classic_wildcard_match(pattern, &name) {
            continue;
        }
        // Native Delete stops this wildcard scan at its first failed erase.
        remove_file_or_directory(&entry.path())
            .with_context(|| format!("delete folder-group entry {}", entry.path().display()))?;
    }
    Ok(())
}

fn pack_folder_save_child(raw_image: &[u8], maker: &[u8]) -> Result<Vec<u8>> {
    let source = Group::from_raw_memory(PathBuf::from("FolderSaveChild.c4g"), raw_image.to_vec())
        .context("open folder-save child image")?;
    let mut child = MutableGroup::from_group(&source).context("copy folder-save child image")?;
    if !maker.is_empty() {
        child.set_maker_bytes(maker);
    }
    child.pack().context("pack folder-save child image")
}

fn find_folder_save_entry(root: &Path, pattern: &[u8]) -> Result<Option<PathBuf>> {
    for entry in fs::read_dir(root)
        .with_context(|| format!("enumerate folder-group target {}", root.display()))?
    {
        let entry = entry?;
        let name = path_to_legacy_bytes(Path::new(&entry.file_name()));
        if !console_save_ignored_directory_entry(&name) && classic_wildcard_match(pattern, &name) {
            return Ok(Some(entry.path()));
        }
    }
    Ok(None)
}

fn merge_folder_save_material(root: &Path, raw_patch: &[u8], maker: &[u8]) -> Result<()> {
    let patch = Group::from_raw_memory(PathBuf::from("Material.c4g"), raw_patch.to_vec())
        .context("open live Material.c4g patch")?;
    let Some(target_path) = find_folder_save_entry(root, b"Material.c4g")? else {
        let packed =
            pack_folder_save_child(raw_patch, maker).context("close new live Material.c4g")?;
        return write_folder_save_entry(root, b"Material.c4g", &packed)
            .context("move new live Material.c4g into folder group");
    };

    if target_path.is_dir() {
        return merge_folder_save_patch_into_directory(&target_path, &patch, maker);
    }

    let target = Group::open(&target_path)
        .with_context(|| format!("open copied Material.c4g {}", target_path.display()))?;
    let mut target = MutableGroup::from_group(&target)
        .with_context(|| format!("copy Material.c4g {}", target_path.display()))?;
    merge_folder_save_patch_into_group(&mut target, &patch)?;
    if !maker.is_empty() {
        target.set_maker_bytes(maker);
    }
    // OpenAsChild reports SaveMap's in-memory Add result, then ignores Close.
    // Preserve that asymmetry: a failed packed-child truncate/write is
    // nonfatal even though its physical prefix may already have changed.
    let close_result = target
        .pack()
        .context("close copied packed Material.c4g")
        .and_then(|packed| write_folder_save_path(&target_path, &packed));
    if let Err(error) = close_result {
        tracing::warn!(%error, "ignored packed Material.c4g close failure");
    }
    Ok(())
}

fn merge_folder_save_patch_into_directory(
    target: &Path,
    patch: &Group,
    maker: &[u8],
) -> Result<()> {
    for entry in patch
        .entries()
        .context("enumerate live Material.c4g patch")?
    {
        let payload = patch
            .read_entry_bytes_exact(&entry)
            .context("read live Material.c4g patch entry")?;
        let payload = if entry.is_directory {
            pack_folder_save_child(&payload, maker)?
        } else {
            payload
        };
        write_folder_save_entry(target, &entry.name_bytes, &payload)?;
    }
    Ok(())
}

fn merge_folder_save_patch_into_group(target: &mut MutableGroup, patch: &Group) -> Result<()> {
    for entry in patch
        .entries()
        .context("enumerate live Material.c4g patch")?
    {
        if entry.is_directory {
            let child = patch
                .open_child(&entry.relative_path)
                .context("open live Material.c4g patch child")?;
            let child =
                MutableGroup::from_group(&child).context("copy live Material.c4g patch child")?;
            target
                .add_child_bytes_with_metadata(
                    entry.name_bytes,
                    child,
                    entry.time,
                    entry.executable,
                )
                .context("merge live Material.c4g child")?;
        } else {
            let payload = patch
                .read_entry_bytes_exact(&entry)
                .context("read live Material.c4g patch entry")?;
            target
                .add_file_bytes_with_metadata(
                    entry.name_bytes,
                    payload,
                    entry.time,
                    entry.executable,
                )
                .context("merge live Material.c4g file")?;
        }
    }
    Ok(())
}

pub(crate) fn persist_live_console_save_group(
    group: &MutableGroup,
    destination: &Path,
    preserve_folder_group: bool,
    folder_journal: &developer_console_save::FolderSaveJournal,
    maker: &[u8],
) -> Result<()> {
    persist_live_console_save_group_timed(
        group,
        destination,
        preserve_folder_group,
        folder_journal,
        maker,
    )
    .map(|_| ())
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ConsoleSavePersistTimings {
    pub(crate) pack_compress: std::time::Duration,
    pub(crate) physical_publish: std::time::Duration,
}

pub(crate) fn persist_live_console_save_group_timed(
    group: &MutableGroup,
    destination: &Path,
    preserve_folder_group: bool,
    folder_journal: &developer_console_save::FolderSaveJournal,
    maker: &[u8],
) -> Result<ConsoleSavePersistTimings> {
    if preserve_folder_group {
        let started = std::time::Instant::now();
        replay_folder_save_journal(folder_journal, destination, maker)?;
        Ok(ConsoleSavePersistTimings {
            physical_publish: started.elapsed(),
            ..ConsoleSavePersistTimings::default()
        })
    } else {
        persist_console_save_group_timed(group, destination, false)
    }
}

pub(crate) fn persist_console_save_group(
    group: &MutableGroup,
    destination: &Path,
    preserve_folder_group: bool,
) -> Result<()> {
    persist_console_save_group_timed(group, destination, preserve_folder_group).map(|_| ())
}

fn persist_console_save_group_timed(
    group: &MutableGroup,
    destination: &Path,
    preserve_folder_group: bool,
) -> Result<ConsoleSavePersistTimings> {
    let pack_started = std::time::Instant::now();
    let packed = group.pack()?;
    let pack_compress = pack_started.elapsed();
    let publish_started = std::time::Instant::now();
    if preserve_folder_group {
        let source = Group::from_memory(destination.to_path_buf(), packed)?;
        let physical_destination = match fs::symlink_metadata(destination) {
            Ok(metadata) if metadata.file_type().is_symlink() => fs::canonicalize(destination)
                .with_context(|| {
                    format!("resolve folder-group target {}", destination.display())
                })?,
            Ok(_) => destination.to_path_buf(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => destination.to_path_buf(),
            Err(error) => return Err(error.into()),
        };
        replace_directory_from_same_parent(&source, &physical_destination)?;
        return Ok(ConsoleSavePersistTimings {
            pack_compress,
            physical_publish: publish_started.elapsed(),
        });
    }
    if let Some(parent) = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("create save parent {}", parent.display()))?;
    }
    replace_file_from_same_directory(destination, &packed)?;
    Ok(ConsoleSavePersistTimings {
        pack_compress,
        physical_publish: publish_started.elapsed(),
    })
}

pub(crate) fn replace_native_save_title_png_if_unchanged(
    request: &PendingNativeSaveThumbnail,
    png: &[u8],
    maker: &[u8],
) -> Result<bool> {
    let current = fs::read(&request.path)
        .with_context(|| format!("read native savegame {}", request.path.display()))?;
    if current != request.packed_group {
        return Ok(false);
    }
    let source = Group::from_memory(request.path.clone(), current)
        .with_context(|| format!("open native savegame {}", request.path.display()))?;
    let mut group = MutableGroup::from_group(&source)
        .with_context(|| format!("copy native savegame {}", request.path.display()))?;
    group
        .add_file("Title.png", png.to_vec())
        .context("replace native savegame Title.png")?;
    if !maker.is_empty() {
        group.set_maker_bytes_recursively(maker);
    }
    persist_console_save_group(&group, &request.path, false)
        .with_context(|| format!("persist native savegame {}", request.path.display()))?;
    Ok(true)
}

/// C4Record::Start unpacks only the record's top-level group. Nested group
/// entries remain compressed physical files so AddFile and replay lookup can
/// use the directory as an open C4Group throughout recording.
pub(crate) fn unpack_recording_group(source: &Group, destination: &Path) -> Result<()> {
    let parent = destination
        .parent()
        .ok_or_else(|| anyhow!("record target has no parent: {}", destination.display()))?;
    fs::create_dir_all(parent)?;
    let filename = destination
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("record");
    let staged = create_sibling_rewrite_directory(parent, filename)?;
    let extracted = (|| -> Result<()> {
        for entry in source.entries()? {
            let relative = path_from_group_name_bytes(&entry.name_bytes);
            anyhow::ensure!(
                relative
                    .components()
                    .all(|component| matches!(component, Component::Normal(_))),
                "record entry has an unsafe name: {}",
                relative.display()
            );
            let target = staged.join(relative);
            let raw = source.read_entry_bytes_exact(&entry)?;
            let payload = if entry.is_directory {
                clonk_resources::compress_c4group_image(&raw)?
            } else {
                raw
            };
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            if entry.executable && !entry.is_directory {
                use std::os::unix::fs::OpenOptionsExt;

                options.mode(0o777);
            }
            let mut file = options.open(&target)?;
            file.write_all(&payload)?;
            let stamp = UNIX_EPOCH + Duration::from_secs(u64::from(entry.time));
            let _ = file.set_times(fs::FileTimes::new().set_accessed(stamp).set_modified(stamp));
        }
        Ok(())
    })();
    if let Err(error) = extracted {
        let _ = remove_file_or_directory(&staged);
        return Err(error);
    }
    let committed = commit_staged_path_with_backup(&staged, destination, || Ok(()));
    if committed.is_err() {
        let _ = remove_file_or_directory(&staged);
    }
    committed
}

fn rewrite_directory_scenario_title(
    path: &Path,
    kind: ScenarioKind,
    title: &str,
    language: &str,
) -> Result<()> {
    let title_paths = fs::read_dir(path)
        .with_context(|| format!("open {}", path.display()))?
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.eq_ignore_ascii_case("Title.txt"))
        })
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    let stem_matches = !matches!(kind, ScenarioKind::Scenario)
        && path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .is_some_and(|stem| stem == title);
    if stem_matches {
        for title_path in title_paths {
            fs::remove_file(title_path)?;
        }
        return Ok(());
    }
    let canonical_title = path.join("Title.txt");
    replace_file_from_same_directory(&canonical_title, format!("{language}:{title}").as_bytes())?;
    for title_path in title_paths {
        let same_file = matches!(
            (fs::canonicalize(&title_path), fs::canonicalize(&canonical_title)),
            (Ok(left), Ok(right)) if left == right
        );
        if !same_file && title_path != canonical_title {
            fs::remove_file(title_path)?;
        }
    }
    Ok(())
}

fn rewrite_mutable_scenario_title(
    group: &mut MutableGroup,
    logical_path: &Path,
    kind: ScenarioKind,
    title: &str,
    language: &str,
) -> Result<()> {
    group.remove_entry("Title.txt");
    let stem_matches = !matches!(kind, ScenarioKind::Scenario)
        && logical_path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .is_some_and(|stem| stem == title);
    if !stem_matches {
        group.add_file("Title.txt", format!("{language}:{title}").into_bytes())?;
    }
    Ok(())
}

fn rewrite_scenario_title(
    path: &Path,
    kind: ScenarioKind,
    title: &str,
    language: &str,
) -> Result<()> {
    if path.is_dir() {
        return rewrite_directory_scenario_title(path, kind, title, language);
    }
    let (physical, children) = scenario_logical_storage(path)?;
    let source = Group::open(&physical)?;
    let mut mutable = MutableGroup::from_group(&source)?;
    let target = mutable_group_descend(&mut mutable, &children)?;
    rewrite_mutable_scenario_title(target, path, kind, title, language)?;
    let packed = mutable.pack()?;
    replace_file_from_same_directory(&physical, &packed)?;
    Ok(())
}

pub(crate) fn rename_scenario_storage(
    path: &Path,
    kind: ScenarioKind,
    title: &str,
    language: &str,
) -> Result<PathBuf> {
    anyhow::ensure!(!title.is_empty(), "scenario title is empty");
    let filename = scenario_filename_from_title(title, kind, path);
    if path.exists() {
        let parent = path
            .parent()
            .ok_or_else(|| anyhow!("scenario path has no parent: {}", path.display()))?;
        let destination = parent.join(filename);
        let identical = destination.exists()
            && matches!(
                (fs::canonicalize(path), fs::canonicalize(&destination)),
                (Ok(left), Ok(right)) if left == right
            );
        anyhow::ensure!(
            !destination.exists() || identical,
            "{} already exists",
            destination.display()
        );
        let moved = destination != path;
        if moved {
            fs::rename(path, &destination).with_context(|| {
                format!("rename {} to {}", path.display(), destination.display())
            })?;
        }
        if let Err(error) = rewrite_scenario_title(&destination, kind, title, language) {
            if moved {
                fs::rename(&destination, path).with_context(|| {
                    format!(
                        "roll back failed title rewrite from {} to {} after: {error:#}",
                        destination.display(),
                        path.display()
                    )
                })?;
            }
            return Err(error);
        }
        return Ok(destination);
    }

    let (physical, mut children) = scenario_logical_storage(path)?;
    let old_name = children
        .pop()
        .ok_or_else(|| anyhow!("logical scenario path has no child entry"))?;
    let source = Group::open(&physical)?;
    let mut mutable = MutableGroup::from_group(&source)?;
    let parent = mutable_group_descend(&mut mutable, &children)?;
    anyhow::ensure!(
        parent.rename_entry_checked(&old_name, &filename)?,
        "C4Group entry `{old_name}` does not exist"
    );
    children.push(filename.clone());
    let target = mutable_group_descend(&mut mutable, &children)?;
    let destination = path.with_file_name(filename);
    rewrite_mutable_scenario_title(target, &destination, kind, title, language)?;
    let packed = mutable.pack()?;
    replace_file_from_same_directory(&physical, &packed)?;
    Ok(destination)
}

pub(crate) fn delete_scenario_storage(path: &Path) -> Result<()> {
    if path.is_dir() {
        fs::remove_dir_all(path)?;
        return Ok(());
    }
    if path.is_file() {
        fs::remove_file(path)?;
        return Ok(());
    }
    let (physical, mut children) = scenario_logical_storage(path)?;
    let name = children
        .pop()
        .ok_or_else(|| anyhow!("logical scenario path has no child entry"))?;
    let source = Group::open(&physical)?;
    let mut mutable = MutableGroup::from_group(&source)?;
    let parent = mutable_group_descend(&mut mutable, &children)?;
    anyhow::ensure!(
        parent.remove_entry(&name),
        "C4Group entry `{name}` does not exist"
    );
    let packed = mutable.pack()?;
    replace_file_from_same_directory(&physical, &packed)?;
    Ok(())
}

pub(crate) fn scenario_storage_is_original(path: &Path) -> bool {
    let Ok((physical, children)) = scenario_logical_storage(path) else {
        return false;
    };
    let Ok(mut group) = Group::open(physical) else {
        return false;
    };
    for child in children {
        let Ok(next) = group.open_child(child) else {
            return false;
        };
        group = next;
    }
    group.is_original()
}

pub(crate) struct ScenarioRoot {
    pub(crate) path: PathBuf,
    pub(crate) label: String,
    include_container: bool,
}

pub(crate) fn scenario_roots(paths: &AppPaths) -> Vec<ScenarioRoot> {
    let mut roots = Vec::new();
    let mut seen = HashSet::new();
    push_root(&mut roots, &mut seen, paths.scenario_dir(), "Scenarios");
    if let Some(content) = paths.content_dir() {
        push_root(&mut roots, &mut seen, content.to_path_buf(), "Scenarios");
    }
    // C++ scans ExePath itself, so its configured SaveGameFolder remains one
    // visible SubFolder before the individual save groups are loaded
    // (C4StartupScenSelDlg.cpp:948-958, 1431-1439). Rust scans selected roots
    // instead; retain the configured folder explicitly, including absolute
    // locations outside the install tree.
    push_entry_root(
        &mut roots,
        &mut seen,
        configured_savegame_directory(Some(paths)),
        "Scenarios",
    );
    // The classic install root is a flat pack namespace. Discover its direct
    // `*.c4f` entries without recursively treating build/source directories
    // beside the executable as scenario folders.
    let mut install_folders = fs::read_dir(paths.install_root())
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| has_extension(path, "c4f"))
        .collect::<Vec<_>>();
    install_folders.sort();
    for folder in install_folders {
        push_root(&mut roots, &mut seen, folder, "Scenarios");
    }
    push_root(
        &mut roots,
        &mut seen,
        paths.install_root().join("Scenarios"),
        "Scenarios",
    );
    push_root(
        &mut roots,
        &mut seen,
        paths.install_root().join("scenarios"),
        "Scenarios",
    );
    push_root(
        &mut roots,
        &mut seen,
        paths.planet_dir().to_path_buf(),
        "Scenarios",
    );
    push_root(
        &mut roots,
        &mut seen,
        paths.system_group_path().to_path_buf(),
        "System",
    );
    roots
}

fn push_root(
    roots: &mut Vec<ScenarioRoot>,
    seen: &mut HashSet<String>,
    path: PathBuf,
    label: &str,
) {
    push_scenario_root(roots, seen, path, label, false);
}

fn push_entry_root(
    roots: &mut Vec<ScenarioRoot>,
    seen: &mut HashSet<String>,
    path: PathBuf,
    label: &str,
) {
    push_scenario_root(roots, seen, path, label, true);
}

fn push_scenario_root(
    roots: &mut Vec<ScenarioRoot>,
    seen: &mut HashSet<String>,
    path: PathBuf,
    label: &str,
    include_container: bool,
) {
    let key = scenario_root_key(&path);
    if !seen.insert(key) {
        return;
    }
    roots.push(ScenarioRoot {
        path,
        label: label.to_string(),
        include_container,
    });
}

pub(crate) fn scenario_root_key(path: &Path) -> String {
    let mut key = String::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => {
                key.push_str(&prefix.as_os_str().to_string_lossy().replace('\\', "/"));
            }
            Component::RootDir => {
                if !key.ends_with('/') {
                    key.push('/');
                }
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if !key.ends_with('/') && !key.is_empty() {
                    key.push('/');
                }
                key.push_str("..");
            }
            Component::Normal(part) => {
                if !key.ends_with('/') && !key.is_empty() {
                    key.push('/');
                }
                key.push_str(&part.to_string_lossy());
            }
        }
    }
    if key.is_empty() {
        key.push('.');
    }
    if cfg!(windows) || cfg!(target_os = "macos") {
        key = key.to_ascii_lowercase();
    }
    key
}

const MUSIC_FILE_EXTENSIONS: [&str; 7] = ["it", "mid", "mod", "mp3", "ogg", "s3m", "xm"];

pub(crate) fn music_playlist_matches(playlist: &[u8], filename: &[u8]) -> bool {
    playlist
        .split(|byte| *byte == b';')
        .any(|pattern| classic_raw_wildcard_match(pattern, filename))
}

#[derive(Clone)]
pub(crate) struct MusicCatalog {
    pub(crate) assets: Vec<MusicAsset>,
}

impl MusicCatalog {
    pub(crate) fn from_group(group: Group) -> Result<Self, clonk_resources::GroupError> {
        let mut catalog = Self::empty();
        catalog.extend_group(group)?;
        Ok(catalog)
    }

    pub(crate) fn empty() -> Self {
        Self { assets: Vec::new() }
    }

    fn extend_group(&mut self, group: Group) -> Result<(), clonk_resources::GroupError> {
        self.extend_group_matching(group, b"*")
    }

    fn extend_group_matching(
        &mut self,
        group: Group,
        pattern: &[u8],
    ) -> Result<(), clonk_resources::GroupError> {
        let root_bytes = music_path_bytes(group.root())?;
        self.extend_group_matching_from(group, pattern, &root_bytes)
    }

    fn extend_group_matching_from(
        &mut self,
        group: Group,
        pattern: &[u8],
        root_bytes: &[u8],
    ) -> Result<(), clonk_resources::GroupError> {
        let source = Arc::new(group);
        let mut entries: Vec<_> = source
            .entries()?
            .into_iter()
            .filter(|entry| {
                !entry.is_directory
                    && classic_wildcard_match(pattern, &entry.name_bytes)
                    && is_music_name_bytes(&entry.name_bytes)
            })
            .collect();
        entries.sort_by_cached_key(|entry| entry.name_bytes.to_ascii_lowercase());
        let assets = entries
            .into_iter()
            .map(|entry| MusicAsset::new(Arc::clone(&source), entry, root_bytes))
            .collect::<Vec<_>>();
        self.assets.extend(assets);
        Ok(())
    }

    pub(crate) fn resolve(&self, name: &str) -> Option<&MusicAsset> {
        let name = music_script_c_string_bytes(name);
        self.assets
            .iter()
            .find(|asset| asset.full_path_bytes == name)
            .or_else(|| {
                self.assets
                    .iter()
                    .find(|asset| asset.file_name_bytes == name)
            })
            .or_else(|| {
                self.assets.iter().find(|asset| {
                    MUSIC_FILE_EXTENSIONS.iter().any(|extension| {
                        asset.file_name_bytes.len() == name.len() + extension.len() + 1
                            && asset.file_name_bytes.starts_with(&name)
                            && asset.file_name_bytes[name.len()] == b'.'
                            && &asset.file_name_bytes[name.len() + 1..] == extension.as_bytes()
                    })
                })
            })
    }

    pub(crate) fn first_enabled(&self, playlist: Option<&str>) -> Option<&MusicAsset> {
        let playlist = playlist.map(music_script_c_string_bytes);
        self.assets
            .iter()
            .find(|asset| Self::is_enabled(asset, playlist.as_deref()))
    }

    pub(crate) fn select_enabled_with(
        &self,
        playlist: Option<&str>,
        most_recently_played: Option<&Arc<MusicAssetIdentity>>,
        mut next_mod: impl FnMut(usize) -> usize,
    ) -> Option<&MusicAsset> {
        let playlist = playlist.map(music_script_c_string_bytes);
        let candidates = self
            .assets
            .iter()
            .filter(|asset| {
                Self::is_enabled(asset, playlist.as_deref())
                    && most_recently_played
                        .is_none_or(|recent| !Arc::ptr_eq(&asset.identity, recent))
            })
            .collect::<Vec<_>>();
        if !candidates.is_empty() {
            // C4MusicSystem::Play makes exactly one SafeRandom call whenever
            // there is a fresh candidate, including SafeRandom(1).
            let index = next_mod(candidates.len());
            assert!(
                index < candidates.len(),
                "random selector exceeded its range"
            );
            return Some(candidates[index]);
        }

        // If the recent song is the sole enabled choice, C++ reuses it
        // without consuming another SafeRandom value.
        most_recently_played.and_then(|recent| {
            self.assets.iter().find(|asset| {
                Arc::ptr_eq(&asset.identity, recent) && Self::is_enabled(asset, playlist.as_deref())
            })
        })
    }

    fn is_enabled(asset: &MusicAsset, playlist: Option<&[u8]>) -> bool {
        match playlist {
            Some(playlist) => music_playlist_matches(playlist, &asset.file_name_bytes),
            None => ![
                b"@".as_slice(),
                b"Credits.".as_slice(),
                b"Frontend.".as_slice(),
            ]
            .iter()
            .any(|prefix| asset.file_name_bytes.starts_with(prefix)),
        }
    }

    pub(crate) fn filenames(&self) -> Vec<String> {
        self.assets
            .iter()
            .map(|asset| clonk_script::c4_string_from_bytes(&asset.file_name_bytes))
            .collect()
    }
}

#[derive(Clone)]
pub(crate) struct MusicAsset {
    source: Arc<Group>,
    entry: GroupEntry,
    pub(crate) full_path_bytes: Vec<u8>,
    pub(crate) file_name_bytes: Vec<u8>,
    pub(crate) identity: Arc<MusicAssetIdentity>,
}

#[derive(Debug)]
pub(crate) struct MusicAssetIdentity;

impl MusicAsset {
    fn new(source: Arc<Group>, entry: GroupEntry, root_bytes: &[u8]) -> Self {
        let file_name_bytes = entry.name_bytes.clone();
        let mut full_path_bytes = root_bytes.to_vec();
        if full_path_bytes
            .last()
            .is_some_and(|byte| music_path_separator(*byte))
        {
            full_path_bytes.pop();
        }
        full_path_bytes.push(std::path::MAIN_SEPARATOR as u8);
        full_path_bytes.extend_from_slice(&file_name_bytes);
        Self {
            source,
            entry,
            full_path_bytes,
            file_name_bytes,
            identity: Arc::new(MusicAssetIdentity),
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test_path(source: Arc<Group>, relative_path: PathBuf) -> Self {
        let root_bytes = music_path_bytes(source.root()).expect("encode test music root");
        let name_bytes = relative_path
            .file_name()
            .unwrap_or(relative_path.as_os_str())
            .as_encoded_bytes()
            .to_vec();
        Self::new(
            source,
            GroupEntry {
                relative_path,
                name_bytes,
                is_directory: false,
                size: 0,
                time: 0,
                executable: false,
                crc_state: 0,
                stored_crc: 0,
            },
            &root_bytes,
        )
    }

    pub(crate) fn load_audio(&self) -> Result<Vec<u8>, clonk_resources::GroupError> {
        self.source.read_entry_bytes_exact(&self.entry)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum MoreMusicDirective {
    Clear,
    Add(Vec<u8>),
}

pub(crate) fn parse_more_music(contents: &[u8]) -> Vec<MoreMusicDirective> {
    contents
        .split(|byte| *byte == b'\n')
        .filter_map(|line| {
            let trim = |byte: &u8| matches!(*byte, b' ' | b'\t' | b'\r');
            let start = line.iter().position(|byte| !trim(byte))?;
            let end = line.iter().rposition(|byte| !trim(byte))? + 1;
            let line = &line[start..end];
            if line == b"#clear" {
                Some(MoreMusicDirective::Clear)
            } else if line.starts_with(b"#") {
                None
            } else {
                Some(MoreMusicDirective::Add(line.to_vec()))
            }
        })
        .collect()
}

fn music_c_string_bytes(bytes: &[u8]) -> &[u8] {
    bytes
        .iter()
        .position(|byte| *byte == 0)
        .map_or(bytes, |end| &bytes[..end])
}

fn music_script_c_string_bytes(value: &str) -> Vec<u8> {
    let mut bytes = clonk_script::c4_string_bytes(value);
    if let Some(end) = bytes.iter().position(|byte| *byte == 0) {
        bytes.truncate(end);
    }
    bytes
}

fn music_path_separator(byte: u8) -> bool {
    byte == b'/' || (cfg!(windows) && byte == b'\\')
}

#[cfg(unix)]
fn music_path_from_bytes(bytes: &[u8]) -> io::Result<PathBuf> {
    use std::os::unix::ffi::OsStrExt as _;

    Ok(PathBuf::from(std::ffi::OsStr::from_bytes(bytes)))
}

#[cfg(windows)]
fn music_path_from_bytes(bytes: &[u8]) -> io::Result<PathBuf> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt as _;
    use windows::Win32::Globalization::{
        MultiByteToWideChar, CP_ACP, MULTI_BYTE_TO_WIDE_CHAR_FLAGS,
    };

    if bytes.is_empty() {
        return Ok(PathBuf::new());
    }
    let required =
        unsafe { MultiByteToWideChar(CP_ACP, MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0), bytes, None) };
    if required == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut wide = vec![0; required as usize];
    let written = unsafe {
        MultiByteToWideChar(
            CP_ACP,
            MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0),
            bytes,
            Some(&mut wide),
        )
    };
    if written == 0 {
        return Err(io::Error::last_os_error());
    }
    wide.truncate(written as usize);
    Ok(PathBuf::from(OsString::from_wide(&wide)))
}

#[cfg(not(any(unix, windows)))]
fn music_path_from_bytes(bytes: &[u8]) -> io::Result<PathBuf> {
    Ok(PathBuf::from(clonk_script::c4_string_from_bytes(bytes)))
}

#[cfg(unix)]
fn music_path_bytes(path: &Path) -> io::Result<Vec<u8>> {
    use std::os::unix::ffi::OsStrExt as _;

    Ok(path.as_os_str().as_bytes().to_vec())
}

#[cfg(windows)]
fn music_path_bytes(path: &Path) -> io::Result<Vec<u8>> {
    use std::os::windows::ffi::OsStrExt as _;
    use windows::core::PCSTR;
    use windows::Win32::Globalization::{WideCharToMultiByte, CP_ACP};

    let wide = path.as_os_str().encode_wide().collect::<Vec<_>>();
    if wide.is_empty() {
        return Ok(Vec::new());
    }
    let required = unsafe { WideCharToMultiByte(CP_ACP, 0, &wide, None, PCSTR::null(), None) };
    if required == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut bytes = vec![0; required as usize];
    let written =
        unsafe { WideCharToMultiByte(CP_ACP, 0, &wide, Some(&mut bytes), PCSTR::null(), None) };
    if written == 0 {
        return Err(io::Error::last_os_error());
    }
    bytes.truncate(written as usize);
    Ok(bytes)
}

#[cfg(not(any(unix, windows)))]
fn music_path_bytes(path: &Path) -> io::Result<Vec<u8>> {
    Ok(path.as_os_str().as_encoded_bytes().to_vec())
}

fn open_music_group_path(path: &Path) -> Result<Group, GroupError> {
    if path.exists() {
        return Group::open(path);
    }

    let mut ancestor = path.to_path_buf();
    let mut children = Vec::new();
    while !ancestor.exists() {
        let Some(name) = ancestor.file_name().map(|name| name.to_os_string()) else {
            return Err(GroupError::Missing(path.to_path_buf()));
        };
        children.push(name);
        if !ancestor.pop() {
            return Err(GroupError::Missing(path.to_path_buf()));
        }
    }

    let mut group = Group::open(ancestor)?;
    for child in children.iter().rev() {
        let pattern = music_path_bytes(Path::new(child))?;
        if pattern.contains(&b'*') {
            return Err(GroupError::InvalidGroup(
                "OpenAsChild: No wildcards allowed".to_string(),
            ));
        }
        let entry = group
            .entries()?
            .into_iter()
            .find(|entry| classic_raw_wildcard_match(&pattern, &entry.name_bytes))
            .ok_or_else(|| GroupError::EntryNotFound(path.to_path_buf()))?;
        group = group.open_child_entry_exact(&entry)?;
    }
    Ok(group)
}

fn add_more_music_spec(
    catalog: &mut MusicCatalog,
    base: &Path,
    spec: &[u8],
) -> Result<(), GroupError> {
    let path = music_path_from_bytes(music_c_string_bytes(spec))?;
    let path = if path.is_absolute() {
        path
    } else {
        base.join(path)
    };
    let pattern = path
        .file_name()
        .map(|name| music_path_bytes(Path::new(name)))
        .transpose()?
        .unwrap_or_else(|| b"*".to_vec());
    let has_wildcard = pattern.iter().any(|byte| matches!(byte, b'*' | b'?'));

    if !has_wildcard {
        if let Ok(group) = open_music_group_path(&path) {
            let root_bytes = music_path_bytes(&path)?;
            return catalog.extend_group_matching_from(group, b"*", &root_bytes);
        }
    }

    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let group = open_music_group_path(parent)?;
    let root_bytes = music_path_bytes(parent)?;
    catalog.extend_group_matching_from(group, &pattern, &root_bytes)
}

pub(crate) fn load_more_music(catalog: &mut MusicCatalog, manifest: &Path) -> io::Result<()> {
    let contents = match fs::read(manifest) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let base = manifest
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));

    for directive in parse_more_music(&contents) {
        match directive {
            MoreMusicDirective::Clear => catalog.assets.clear(),
            MoreMusicDirective::Add(spec) => {
                if let Err(error) = add_more_music_spec(catalog, base, &spec) {
                    let spec = String::from_utf8_lossy(&spec);
                    tracing::warn!(%spec, %error, "MoreMusic entry skipped");
                }
            }
        }
    }
    Ok(())
}

pub(crate) struct MusicResolver {
    pub(crate) global: MusicCatalog,
    pub(crate) extra: Option<Group>,
    scenario: MusicCatalog,
    pub(crate) scenario_has_local_sources: bool,
    pub(crate) scenario_root: Option<PathBuf>,
    pub(crate) playlist: Option<String>,
}

impl MusicResolver {
    pub(crate) fn empty() -> Self {
        Self {
            global: MusicCatalog::empty(),
            extra: None,
            scenario: MusicCatalog::empty(),
            scenario_has_local_sources: false,
            scenario_root: None,
            playlist: None,
        }
    }

    fn discover() -> Self {
        let paths = match AppPaths::discover() {
            Ok(paths) => paths,
            Err(error) => {
                tracing::warn!(%error, "music resource discovery skipped");
                return Self::empty();
            }
        };
        Self::discover_from_paths(&paths)
    }

    pub(crate) fn discover_for_paths(paths: Option<&AppPaths>) -> Self {
        match paths {
            Some(paths) => Self::discover_from_paths(paths),
            None => Self::discover(),
        }
    }

    fn discover_from_paths(paths: &AppPaths) -> Self {
        let global = (|| -> anyhow::Result<MusicCatalog> {
            let path = find_music_group(paths)?;
            let group = Group::open(&path)
                .with_context(|| format!("failed to open music group at {}", path.display()))?;
            MusicCatalog::from_group(group).map_err(anyhow::Error::from)
        })();
        let mut global = match global {
            Ok(global) => global,
            Err(error) => {
                tracing::warn!(%error, "global music catalog discovery skipped");
                MusicCatalog::empty()
            }
        };
        let exe_data_root = paths.executable_data_root();
        let more_music_path = exe_data_root.join("MoreMusic.txt");
        if let Err(error) = load_more_music(&mut global, &more_music_path) {
            tracing::warn!(
                path = %more_music_path.display(),
                %error,
                "MoreMusic discovery skipped"
            );
        }
        let extra = match mapped_classic_extra_group_path(paths) {
            Ok(Some(path)) => match Group::open(&path) {
                Ok(extra) => Some(extra),
                Err(error) => {
                    tracing::warn!(
                        path = %path.display(),
                        %error,
                        "failed to open optional Extra.c4g music root"
                    );
                    None
                }
            },
            Ok(None) => None,
            Err(error) => {
                tracing::warn!(%error, "Extra.c4g music discovery skipped");
                None
            }
        };
        Self {
            global,
            extra,
            scenario: MusicCatalog::empty(),
            scenario_has_local_sources: false,
            scenario_root: None,
            playlist: None,
        }
    }

    pub(crate) fn with_global_group(group: Group) -> Result<Self, clonk_resources::GroupError> {
        Ok(Self {
            global: MusicCatalog::from_group(group)?,
            extra: None,
            scenario: MusicCatalog::empty(),
            scenario_has_local_sources: false,
            scenario_root: None,
            playlist: None,
        })
    }

    pub(crate) fn configure_scenario(
        &mut self,
        path: Option<&Path>,
    ) -> Result<bool, clonk_resources::GroupError> {
        // `play_scenario_audio` repeats the path-only configuration after the
        // resource-aware activation pass. Preserve that pass's definition
        // roots when the scenario itself did not change.
        if self.scenario_root.as_deref() == path {
            return Ok(false);
        }
        self.configure_scenario_with_definition_roots(path, &[])
    }

    pub(crate) fn configure_scenario_with_definition_roots(
        &mut self,
        path: Option<&Path>,
        definition_roots: &[Group],
    ) -> Result<bool, clonk_resources::GroupError> {
        // A resource-aware call marks a real scenario activation/reload. C++
        // rebuilds its song list even when the path and selected root names are
        // unchanged, so never reuse the prior catalog here.
        let (scenario, has_local_sources) = path
            .map(|path| build_scenario_music_catalog(path, definition_roots, self.extra.as_ref()))
            .transpose()?
            .unwrap_or_else(|| (MusicCatalog::empty(), false));
        self.scenario = scenario;
        self.scenario_has_local_sources = has_local_sources;
        self.scenario_root = path.map(Path::to_path_buf);
        self.playlist = None;
        Ok(true)
    }

    pub(crate) fn active_catalog(&self) -> &MusicCatalog {
        if self.scenario_has_local_sources {
            &self.scenario
        } else {
            &self.global
        }
    }

    pub(crate) fn resolve(&self, name: &str) -> Option<&MusicAsset> {
        self.active_catalog().resolve(name)
    }

    pub(crate) fn active_filenames(&self) -> Vec<String> {
        self.active_catalog().filenames()
    }

    pub(crate) fn set_playlist(&mut self, playlist: Option<String>) {
        self.playlist = playlist;
    }

    pub(crate) fn first_default(&self) -> Option<&MusicAsset> {
        self.active_catalog()
            .first_enabled(self.playlist.as_deref())
    }

    pub(crate) fn select_default_with(
        &self,
        most_recently_played: Option<&Arc<MusicAssetIdentity>>,
        next_mod: impl FnMut(usize) -> usize,
    ) -> Option<&MusicAsset> {
        self.active_catalog().select_enabled_with(
            self.playlist.as_deref(),
            most_recently_played,
            next_mod,
        )
    }
}

fn build_scenario_music_catalog(
    path: &Path,
    definition_roots: &[Group],
    extra: Option<&Group>,
) -> Result<(MusicCatalog, bool), clonk_resources::GroupError> {
    let scenario = Group::open(path)?;
    let mut catalog = MusicCatalog::empty();
    let scenario_has_tracks = scenario
        .entries()?
        .into_iter()
        .any(|entry| !entry.is_directory && is_music_name_bytes(&entry.name_bytes));
    if scenario_has_tracks {
        catalog.extend_group(scenario.clone())?;
    }

    let mut has_local_sources = scenario_has_tracks;
    has_local_sources |= extend_direct_music_child(&mut catalog, &scenario, "scenario Music.c4g");

    let mut parent = path.parent();
    while let Some(folder_path) = parent.filter(|parent| has_extension(parent, "c4f")) {
        let folder = Group::open(folder_path)?;
        has_local_sources |= extend_direct_music_child(&mut catalog, &folder, "parent Music.c4g");
        parent = folder_path.parent();
    }

    // Extra children sit above the Extra root and definition roots. Direct
    // Game.GroupSet iteration keeps later equal-priority registrations first.
    if let Some(extra) = extra {
        for definition_root in definition_roots.iter().rev() {
            let Some(name) = definition_root.root().file_name() else {
                continue;
            };
            let child = match open_child_flexible(extra, Path::new(name)) {
                Ok(Some(child)) => child,
                Ok(None) => continue,
                Err(error) => {
                    tracing::warn!(
                        extra = %extra.root().display(),
                        definition = %name.to_string_lossy(),
                        %error,
                        "failed to open activated Extra.c4g definition group for music"
                    );
                    continue;
                }
            };
            let Some(child_path) = music_child_path(&child)? else {
                continue;
            };
            has_local_sources = true;
            match child.open_child(&child_path) {
                Ok(group) => {
                    if let Err(error) = catalog.extend_group(group) {
                        tracing::warn!(
                            root = %child.root().display(),
                            %error,
                            "failed to enumerate Extra definition Music.c4g"
                        );
                    }
                }
                Err(error) => {
                    tracing::warn!(
                        root = %child.root().display(),
                        %error,
                        "failed to open Extra definition Music.c4g"
                    );
                }
            }
        }
        if let Some(child_path) = music_child_path(extra)? {
            has_local_sources = true;
            match extra.open_child(&child_path) {
                Ok(group) => {
                    if let Err(error) = catalog.extend_group(group) {
                        tracing::warn!(
                            root = %extra.root().display(),
                            %error,
                            "failed to enumerate Extra root Music.c4g"
                        );
                    }
                }
                Err(error) => {
                    tracing::warn!(
                        root = %extra.root().display(),
                        %error,
                        "failed to open Extra root Music.c4g"
                    );
                }
            }
        }
    }

    // FindGroup(C4GSCnt_Music) walks equal-priority definition roots newest
    // first. Every direct Music.c4g is a local source, even when it contains no
    // tracks, and duplicate root registrations remain independently visible.
    for definition_root in definition_roots.iter().rev() {
        let Some(child_path) = music_child_path(definition_root)? else {
            continue;
        };
        has_local_sources = true;
        match definition_root.open_child(&child_path) {
            Ok(group) => {
                if let Err(error) = catalog.extend_group(group) {
                    tracing::warn!(
                        root = %definition_root.root().display(),
                        %error,
                        "failed to enumerate definition-root Music.c4g"
                    );
                }
            }
            Err(error) => {
                tracing::warn!(
                    root = %definition_root.root().display(),
                    %error,
                    "failed to open definition-root Music.c4g"
                );
            }
        }
    }

    Ok((catalog, has_local_sources))
}

pub(crate) fn extend_music_source(catalog: &mut MusicCatalog, group: Group, source: &'static str) {
    let root = group.root().to_path_buf();
    if let Err(error) = catalog.extend_group(group) {
        tracing::warn!(
            root = %root.display(),
            %source,
            %error,
            "failed to enumerate local music source"
        );
    }
}

fn extend_direct_music_child(
    catalog: &mut MusicCatalog,
    group: &Group,
    source: &'static str,
) -> bool {
    let child_path = match music_child_path(group) {
        Ok(Some(path)) => path,
        Ok(None) => return false,
        Err(error) => {
            tracing::warn!(
                root = %group.root().display(),
                %source,
                %error,
                "failed to inspect local music source"
            );
            return false;
        }
    };

    // CheckGroupContents marks the owning group as a music source from the
    // direct entry alone. PlayScenarioMusic clears the old song list before
    // LoadDir tries to open that child, so a malformed Music.c4g still counts.
    match group.open_child(&child_path) {
        Ok(child) => extend_music_source(catalog, child, source),
        Err(error) => {
            tracing::warn!(
                root = %group.root().display(),
                child = %child_path.display(),
                %source,
                %error,
                "failed to open local Music.c4g source"
            );
        }
    }
    true
}

fn open_music_child(group: &Group) -> Result<Option<Group>, clonk_resources::GroupError> {
    music_child_path(group)?
        .map(|path| group.open_child(path))
        .transpose()
}

fn music_child_path(group: &Group) -> Result<Option<PathBuf>, clonk_resources::GroupError> {
    Ok(group
        .entries()?
        .into_iter()
        .find(|entry| {
            entry
                .relative_path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case("Music.c4g"))
        })
        .map(|entry| entry.relative_path))
}

pub(crate) fn load_scenario_music_bytes(path: &Path) -> anyhow::Result<Option<Vec<u8>>> {
    let scenario = Group::open(path)
        .with_context(|| format!("failed to open scenario group at {}", path.display()))?;
    if let Some(data) = find_music_asset(&scenario)
        .with_context(|| format!("failed to inspect {} for music", path.display()))?
    {
        return Ok(Some(data));
    }
    if let Some(data) = find_music_group_asset(&scenario)
        .with_context(|| format!("failed to inspect {} for Music.c4g", path.display()))?
    {
        return Ok(Some(data));
    }

    // C4Game::OpenScenario registers the contiguous .c4f parent chain in the
    // group set. C4MusicSystem then loads each registered group's direct
    // Music.c4g child (C4Game.cpp:141-161; C4MusicSystem.cpp:152-163).
    let mut parent = path.parent();
    while let Some(folder_path) = parent.filter(|parent| has_extension(parent, "c4f")) {
        let folder = Group::open(folder_path).with_context(|| {
            format!(
                "failed to open scenario parent group at {}",
                folder_path.display()
            )
        })?;
        if let Some(data) = find_music_group_asset(&folder)
            .with_context(|| format!("failed to inspect {} for Music.c4g", folder_path.display()))?
        {
            return Ok(Some(data));
        }
        parent = folder_path.parent();
    }

    Ok(None)
}

fn find_music_asset(group: &Group) -> Result<Option<Vec<u8>>, clonk_resources::GroupError> {
    // C4MusicSystem's FindEntry/LoadDir searches one group level only. Never
    // descend into definitions: their WAV files are sound effects.
    let catalog = MusicCatalog::from_group(group.clone())?;
    catalog
        .first_enabled(None)
        .map(MusicAsset::load_audio)
        .transpose()
}

fn find_music_group_asset(group: &Group) -> Result<Option<Vec<u8>>, clonk_resources::GroupError> {
    let Some(music_group) = open_music_child(group)? else {
        return Ok(None);
    };
    find_music_asset(&music_group)
}

pub(crate) fn has_extension(path: &Path, expected: &str) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case(expected))
}

fn is_music_name_bytes(name: &[u8]) -> bool {
    let Some(dot) = name.iter().rposition(|byte| *byte == b'.') else {
        return false;
    };
    let extension = &name[dot + 1..];
    // C4MusicSystem.cpp:31-32. WAV belongs to the sound-effect resolver.
    MUSIC_FILE_EXTENSIONS
        .iter()
        .any(|candidate| extension.eq_ignore_ascii_case(candidate.as_bytes()))
}

pub(crate) fn sandbox_music_bytes() -> &'static [u8] {
    static DATA: OnceLock<Vec<u8>> = OnceLock::new();
    if let Some(data) = DATA.get() {
        return data.as_slice();
    }
    // Cache successful loads only: a failed discovery (e.g. under an
    // env-guarded test) must not poison the process-wide cache.
    match load_menu_music() {
        Ok(bytes) if !bytes.is_empty() => DATA.get_or_init(|| bytes).as_slice(),
        Ok(_) => &[],
        Err(err) => {
            tracing::warn!(error = %err, "failed to load menu music, no music will play");
            &[]
        }
    }
}

fn load_menu_music() -> Result<Vec<u8>> {
    let paths = AppPaths::discover()?;
    let music_group_path = find_music_group(&paths)?;
    let group = Group::open(&music_group_path).with_context(|| {
        format!(
            "failed to open music group at {}",
            music_group_path.display()
        )
    })?;

    // Try Frontend.ogg first (main menu music in C++)
    let music_data = group
        .read_file(Path::new("Frontend.ogg"))
        .or_else(|_| group.read_file(Path::new("frontend.ogg")))
        .context("failed to read Frontend.ogg from music group")?;

    Ok(music_data)
}

fn find_music_group(paths: &AppPaths) -> Result<PathBuf> {
    let mut search_roots = vec![
        paths.install_root().to_path_buf(),
        paths.planet_dir().to_path_buf(),
        paths.user_data_dir().to_path_buf(),
    ];
    if let Some(content) = paths.content_dir() {
        search_roots.push(content.to_path_buf());
    }

    for root in search_roots {
        for name in ["Music.c4g", "music.c4g", "Music.ocg", "music.ocg"] {
            let candidate = root.join(name);
            if candidate.exists() {
                return Ok(candidate);
            }
        }
    }

    Err(anyhow!("Music.c4g not found in standard directories"))
}

pub(crate) fn compute_mix_values(
    info: &mut ChannelInfo,
    snapshot: &SimulationSnapshot,
    viewports: &[ActiveViewportProjection],
) -> (f32, f32) {
    compute_mix_values_with_rendered_audibility(info, snapshot, viewports, None)
}

pub(crate) fn compute_mix_values_with_rendered_audibility(
    info: &mut ChannelInfo,
    snapshot: &SimulationSnapshot,
    viewports: &[ActiveViewportProjection],
    rendered_object_audibility: Option<&HashMap<ObjectId, CachedObjectAudibilityMix>>,
) -> (f32, f32) {
    let base_volume = (info.volume as f32 / 100.0).max(0.0);
    let Some(target_id) = info.target else {
        return info.detached_mix.unwrap_or((base_volume, 0.0));
    };
    let Some(target) = snapshot.object(target_id) else {
        return info.detached_mix.unwrap_or((base_volume, 0.0));
    };
    let origin_mix = compute_positional_mix_values(target.position, snapshot, viewports);
    info.detached_mix = Some((f32::from(origin_mix.0) / 100.0, origin_mix.1));
    let (audibility, pan) = rendered_object_audibility
        .and_then(|cache| cached_attached_object_mix_values(target, cache))
        .unwrap_or(origin_mix);

    (
        base_volume * adjusted_audibility(audibility, info.custom_falloff),
        pan,
    )
}

pub(crate) fn compute_object_positional_mix(
    source: Vector2,
    snapshot: &SimulationSnapshot,
    viewports: &[ActiveViewportProjection],
) -> (f32, f32) {
    let (audibility, pan) = compute_positional_mix_values(source, snapshot, viewports);
    (f32::from(audibility) / 100.0, pan)
}

pub(crate) fn compute_mix_values_for(
    volume: u8,
    target_id: Option<ObjectId>,
    custom_falloff: Option<i32>,
    snapshot: &SimulationSnapshot,
    viewports: &[ActiveViewportProjection],
) -> (f32, f32) {
    let base_volume = (f32::from(volume) / 100.0).clamp(0.0, 1.0);
    let Some(target_id) = target_id else {
        return (base_volume, 0.0);
    };
    let Some(target) = snapshot.object(target_id) else {
        return (base_volume, 0.0);
    };
    let (audibility, pan) = compute_positional_mix_values(target.position, snapshot, viewports);
    (
        base_volume * adjusted_audibility(audibility, custom_falloff),
        pan,
    )
}
