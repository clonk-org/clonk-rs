//! Runtime `C4ChartDialog` presentation and input state.
//!
//! Network sampling remains in `clonk-app`/`clonk-network`. This module deliberately
//! owns only a frontend-neutral copy of the visible graphs, the native tab
//! order, and the classic dialog renderer.

use anyhow::{ensure, Result};
use clonk_graphics::clonk_font::TextAlign;
use clonk_graphics::{GammaRamp, Surface};

use crate::classic_gui::{
    draw_3d_frame, draw_clipped_text, draw_engine_box,
    draw_engine_line as draw_retained_engine_line, draw_facet_stretch, ClassicGuiSkin, IntRect,
};
use crate::{ClonkFontSet, GuiPoint, ImageData, KeyCode};

pub const NETWORK_CHART_DIALOG_WIDTH: i32 = 400;
pub const NETWORK_CHART_DIALOG_HEIGHT: i32 = 300;
/// Exact LanguageUS value of `IDS_NET_STATISTICS`.
pub const NETWORK_CHART_DIALOG_TITLE: &str = "Statistics";

const TABULAR_INSET: i32 = 5;
const TABULAR_MARGIN: i32 = 4;
const TAB_STRIP_HEIGHT: i32 = 20;
const TAB_INITIAL_OFFSET: i32 = 20;
const TAB_CAPTION_PADDING: i32 = 20;
const TAB_SPACING: i32 = 2;
const MIN_CAPTION_HEIGHT: i32 = 23;
const CLOSE_BUTTON_SIZE: i32 = 16;
const CLOSE_BUTTON_INSET: i32 = 4;
const CLOSE_ICON_PHASE: u32 = 34;
const ICON_CELL: u32 = 40;
const AXIS_ARROW_LENGTH: i32 = 6;
const AXIS_ARROW_THICKNESS: i32 = 3;
const AXIS_ARROW_INDENT: i32 = 2;
const AXIS_MARKER_LENGTH: i32 = 5;
const AXIS_COLOR: u32 = 0x007f_7f7f;
const ACTIVE_TAB_COLOR: [u8; 4] = [255, 255, 255, 255];
const INACTIVE_TAB_COLOR: [u8; 4] = [175, 175, 175, 255];

const LOCAL_TABS: [(&str, &str); 5] = [
    ("oc", "Object count"),
    ("FPS", "FPS"),
    ("NetIO", "Network I/O"),
    ("Control", "Control"),
    ("APM", "APM"),
];

/// One retained-time series copied from `C4Graph`/`TableGraph`.
///
/// `values[0]` belongs to `start_time`; `end_time` is exclusive. The explicit
/// extrema let the application preserve the graph owner's averaged range
/// without coupling this crate to `clonk-network`.
#[derive(Clone, Debug, PartialEq)]
pub struct NetworkChartSeriesSnapshot {
    pub title: String,
    /// Packed engine `AARRGGBB`, whose alpha byte is inverse opacity.
    pub color: u32,
    pub start_time: i32,
    pub end_time: i32,
    pub min_value: f32,
    pub max_value: f32,
    pub values: Vec<f32>,
}

impl NetworkChartSeriesSnapshot {
    pub fn new(title: impl Into<String>, color: u32, start_time: i32, values: Vec<f32>) -> Self {
        let end_time = start_time.saturating_add(i32::try_from(values.len()).unwrap_or(i32::MAX));
        let (min_value, max_value) = finite_extrema(&values).unwrap_or((0.0, 0.0));
        Self {
            title: title.into(),
            color,
            start_time,
            end_time,
            min_value,
            max_value,
            values,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty() || self.end_time <= self.start_time
    }

    pub fn value(&self, time: i32) -> Option<f32> {
        if time < self.start_time || time >= self.end_time {
            return None;
        }
        let offset = usize::try_from(time - self.start_time).ok()?;
        self.values.get(offset).copied()
    }
}

/// A chart collection displayed in one native tab.
#[derive(Clone, Debug, PartialEq)]
pub struct NetworkChartGraphSnapshot {
    pub title: String,
    pub series: Vec<NetworkChartSeriesSnapshot>,
}

impl NetworkChartGraphSnapshot {
    pub fn new(title: impl Into<String>, series: Vec<NetworkChartSeriesSnapshot>) -> Self {
        Self {
            title: title.into(),
            series,
        }
    }

    pub fn is_empty(&self) -> bool {
        !self.series.iter().any(|series| !series.is_empty())
    }

    pub fn series_count(&self) -> usize {
        self.series.len()
    }

    pub fn start_time(&self) -> i32 {
        self.series
            .iter()
            .map(|series| series.start_time)
            .min()
            .unwrap_or(0)
    }

    pub fn end_time(&self) -> i32 {
        self.series
            .iter()
            .map(|series| series.end_time)
            .max()
            .unwrap_or(0)
    }

    pub fn min_value(&self) -> f32 {
        self.series
            .iter()
            .filter(|series| series.min_value.is_finite())
            .map(|series| series.min_value)
            .reduce(f32::min)
            .unwrap_or(0.0)
    }

    pub fn max_value(&self) -> f32 {
        self.series
            .iter()
            .filter(|series| series.max_value.is_finite())
            .map(|series| series.max_value)
            .reduce(f32::max)
            .unwrap_or(0.0)
    }
}

/// Snapshot and literal native caption for one `C4GUI::Tabular::Sheet`.
#[derive(Clone, Debug, PartialEq)]
pub struct NetworkChartTabSnapshot {
    pub caption: String,
    pub graph: NetworkChartGraphSnapshot,
}

impl NetworkChartTabSnapshot {
    pub fn new(caption: impl Into<String>, graph: NetworkChartGraphSnapshot) -> Self {
        Self {
            caption: caption.into(),
            graph,
        }
    }
}

#[derive(Clone, Copy)]
pub struct NetworkChartResources<'a> {
    pub skin: ClassicGuiSkin<'a>,
    pub fonts: &'a ClonkFontSet,
    pub icons: &'a ImageData,
}

impl NetworkChartResources<'_> {
    pub fn validate(self) -> Result<()> {
        self.skin.validate_message_dialog_assets()?;
        ensure!(
            self.fonts.text.line_height > 0 && self.fonts.mini.line_height > 0,
            "FontRegular is not initialized"
        );
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetworkChartTabLayout {
    pub index: usize,
    pub bounds: IntRect,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkChartLayout {
    pub bounds: IntRect,
    /// The dialog's own title strip, absent where the host window draws one.
    pub caption: Option<IntRect>,
    /// The dialog's own close icon, absent for the same reason.
    pub close_button: Option<IntRect>,
    pub tabular: IntRect,
    pub chart: IntRect,
    pub tabs: Vec<NetworkChartTabLayout>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkChartDialogAction {
    Ignored,
    Handled,
    Captured,
    Close,
}

/// Singleton-ready state for the runtime chart dialog.
///
/// The owner represents the native singleton with `Option<NetworkChartDialog>`.
/// Constructing the option opens it and dropping the option closes it.
#[derive(Clone, Debug)]
pub struct NetworkChartDialog {
    caption: String,
    tabs: Vec<NetworkChartTabSnapshot>,
    active_tab: usize,
    position: Option<(i32, i32)>,
    drag_anchor: Option<(f32, f32)>,
    close_pressed: bool,
}

impl NetworkChartDialog {
    pub fn new(networked: bool) -> Self {
        Self::new_with_caption(networked, NETWORK_CHART_DIALOG_TITLE)
    }

    pub fn new_with_caption(networked: bool, caption: impl Into<String>) -> Self {
        let mut tabs = Vec::with_capacity(if networked { 6 } else { 5 });
        for (caption, title) in LOCAL_TABS[..3].iter().copied() {
            tabs.push(empty_tab(caption, title));
        }
        if networked {
            tabs.push(empty_tab("Pings", "Pings"));
        }
        for (caption, title) in LOCAL_TABS[3..].iter().copied() {
            tabs.push(empty_tab(caption, title));
        }
        Self {
            caption: caption.into(),
            tabs,
            active_tab: 0,
            position: None,
            drag_anchor: None,
            close_pressed: false,
        }
    }

    pub fn caption(&self) -> &str {
        &self.caption
    }

    pub fn tabs(&self) -> &[NetworkChartTabSnapshot] {
        &self.tabs
    }

    pub fn tab_names(&self) -> Vec<&str> {
        self.tabs.iter().map(|tab| tab.caption.as_str()).collect()
    }

    pub fn active_tab_index(&self) -> usize {
        self.active_tab
    }

    pub fn active_tab(&self) -> Option<&NetworkChartTabSnapshot> {
        self.tabs.get(self.active_tab)
    }

    pub fn active_graph(&self) -> Option<&NetworkChartGraphSnapshot> {
        self.active_tab().map(|tab| &tab.graph)
    }

    pub fn active_graph_has_data(&self) -> bool {
        self.active_graph().is_some_and(|graph| !graph.is_empty())
    }

    pub fn select_tab(&mut self, index: usize) -> bool {
        if index >= self.tabs.len() {
            return false;
        }
        self.active_tab = index;
        true
    }

    pub fn select_tab_named(&mut self, caption: &str) -> bool {
        let Some(index) = self
            .tabs
            .iter()
            .position(|tab| tab.caption.eq_ignore_ascii_case(caption))
        else {
            return false;
        };
        self.active_tab = index;
        true
    }

    /// Replaces one live graph while retaining the oracle's tab order and
    /// literal sheet caption.
    pub fn set_graph_snapshot(&mut self, caption: &str, graph: NetworkChartGraphSnapshot) -> bool {
        let Some(tab) = self
            .tabs
            .iter_mut()
            .find(|tab| tab.caption.eq_ignore_ascii_case(caption))
        else {
            return false;
        };
        tab.graph = graph;
        true
    }

    pub fn layout(
        &self,
        preferred: IntRect,
        resources: NetworkChartResources<'_>,
    ) -> NetworkChartLayout {
        let (x, y) = self
            .position
            .unwrap_or((preferred.x + 30, preferred.y + 30));
        let bounds = IntRect::new(
            x,
            y,
            NETWORK_CHART_DIALOG_WIDTH,
            NETWORK_CHART_DIALOG_HEIGHT,
        );
        let caption_height = resources.fonts.text.line_height.max(MIN_CAPTION_HEIGHT);
        let caption = IntRect::new(bounds.x, bounds.y, bounds.w, caption_height);
        let close_button = IntRect::new(
            caption.x + caption.w - CLOSE_BUTTON_SIZE - CLOSE_BUTTON_INSET,
            caption.y + CLOSE_BUTTON_INSET,
            CLOSE_BUTTON_SIZE,
            CLOSE_BUTTON_SIZE,
        );
        let tabular = IntRect::new(
            bounds.x + TABULAR_INSET,
            bounds.y + caption_height + TABULAR_INSET,
            bounds.w - 2 * TABULAR_INSET,
            bounds.h - caption_height - 2 * TABULAR_INSET,
        );
        let chart = IntRect::new(
            tabular.x + TABULAR_MARGIN,
            tabular.y + TAB_STRIP_HEIGHT + TABULAR_MARGIN,
            (tabular.w - 2 * TABULAR_MARGIN).max(1),
            (tabular.h - TAB_STRIP_HEIGHT - 2 * TABULAR_MARGIN).max(1),
        );
        let mut tab_x = tabular.x + TAB_INITIAL_OFFSET;
        let tabs = self
            .tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| {
                let width =
                    resources.fonts.text.measure(&tab.caption, false).0 + TAB_CAPTION_PADDING;
                let layout = NetworkChartTabLayout {
                    index,
                    bounds: IntRect::new(
                        tab_x,
                        tabular.y,
                        width.max(TAB_STRIP_HEIGHT),
                        TAB_STRIP_HEIGHT,
                    ),
                };
                tab_x += layout.bounds.w + TAB_SPACING;
                layout
            })
            .collect();
        NetworkChartLayout {
            bounds,
            caption: Some(caption),
            close_button: Some(close_button),
            tabular,
            chart,
            tabs,
        }
    }

    /// Stronger fullscreen Escape binding.
    ///
    /// The native chart is non-exclusive in the shared in-game screen, so
    /// its ordinary GUI-only focus and tabular key bindings are out of scope.
    /// Sheets remain pointer-selectable.
    pub fn handle_key(&mut self, key: KeyCode, pressed: bool) -> NetworkChartDialogAction {
        match key {
            KeyCode::Escape => {
                if pressed {
                    NetworkChartDialogAction::Close
                } else {
                    NetworkChartDialogAction::Handled
                }
            }
            _ => NetworkChartDialogAction::Ignored,
        }
    }

    /// Native top-tab selection occurs on left-button down. The title is the
    /// dialog's drag element, while the close icon captures through release.
    pub fn pointer_down(
        &mut self,
        point: GuiPoint,
        preferred: IntRect,
        resources: NetworkChartResources<'_>,
    ) -> NetworkChartDialogAction {
        let layout = self.layout(preferred, resources);
        if !contains(layout.bounds, point) {
            return NetworkChartDialogAction::Ignored;
        }
        if contains_widget(layout.close_button, point) {
            self.close_pressed = true;
            self.drag_anchor = None;
            return NetworkChartDialogAction::Captured;
        }
        if contains_widget(layout.caption, point) {
            self.drag_anchor = Some((
                point.x - layout.bounds.x as f32,
                point.y - layout.bounds.y as f32,
            ));
            self.close_pressed = false;
            return NetworkChartDialogAction::Captured;
        }
        if let Some(tab) = layout.tabs.iter().find(|tab| contains(tab.bounds, point)) {
            self.active_tab = tab.index;
        }
        NetworkChartDialogAction::Handled
    }

    pub fn pointer_move(
        &mut self,
        point: GuiPoint,
        preferred: IntRect,
        resources: NetworkChartResources<'_>,
    ) -> bool {
        if let Some((anchor_x, anchor_y)) = self.drag_anchor {
            self.position = Some((
                (point.x - anchor_x).round() as i32,
                (point.y - anchor_y).round() as i32,
            ));
            return true;
        }
        self.close_pressed || contains(self.layout(preferred, resources).bounds, point)
    }

    pub fn pointer_up(
        &mut self,
        point: GuiPoint,
        preferred: IntRect,
        resources: NetworkChartResources<'_>,
    ) -> NetworkChartDialogAction {
        let layout = self.layout(preferred, resources);
        if self.close_pressed {
            self.close_pressed = false;
            self.drag_anchor = None;
            return if contains_widget(layout.close_button, point) {
                NetworkChartDialogAction::Close
            } else {
                NetworkChartDialogAction::Handled
            };
        }
        if self.drag_anchor.take().is_some() {
            return NetworkChartDialogAction::Handled;
        }
        if contains(layout.bounds, point) {
            NetworkChartDialogAction::Handled
        } else {
            NetworkChartDialogAction::Ignored
        }
    }

    pub fn cancel_pointer_capture(&mut self) {
        self.drag_anchor = None;
        self.close_pressed = false;
    }

    pub fn render(
        &self,
        surface: &mut Surface,
        preferred: IntRect,
        resources: NetworkChartResources<'_>,
        gamma: Option<&GammaRamp>,
    ) -> Result<()> {
        resources.validate()?;
        let layout = self.layout(preferred, resources);
        resources.skin.draw_dialog(surface, layout.bounds, gamma);
        // A layout without these widgets is one whose host window draws them,
        // so there is nothing to paint rather than an empty rect to paint into.
        if let Some(caption) = layout.caption {
            resources.skin.draw_caption_with_right_indent(
                surface,
                caption,
                &self.caption,
                &resources.fonts.text,
                ACTIVE_TAB_COLOR,
                TextAlign::Left,
                20,
                gamma,
            );
        }
        if let Some(close_button) = layout.close_button {
            draw_icon_phase(
                surface,
                resources.icons,
                CLOSE_ICON_PHASE,
                close_button,
                gamma,
            )?;
        }

        // Classical top Tabular: a 20px caption strip over the sheet client.
        draw_engine_box(
            surface,
            layout.tabular.x,
            layout.tabular.y + TAB_STRIP_HEIGHT,
            layout.tabular.x + layout.tabular.w - 1,
            layout.tabular.y + layout.tabular.h - 1,
            0x0000_0000,
            gamma,
        );
        draw_3d_frame(
            surface,
            layout.tabular.with_vertical(
                layout.tabular.y + TAB_STRIP_HEIGHT,
                layout.tabular.h - TAB_STRIP_HEIGHT,
            ),
            gamma,
        );
        for tab_layout in &layout.tabs {
            let active = tab_layout.index == self.active_tab;
            draw_engine_box(
                surface,
                tab_layout.bounds.x,
                tab_layout.bounds.y,
                tab_layout.bounds.x + tab_layout.bounds.w - 1,
                tab_layout.bounds.y + tab_layout.bounds.h - 1,
                if active { 0x0000_0000 } else { 0x5f00_0000 },
                gamma,
            );
            draw_3d_frame(surface, tab_layout.bounds, gamma);
            if let Some(tab) = self.tabs.get(tab_layout.index) {
                draw_clipped_text(
                    surface,
                    &resources.fonts.text,
                    tab_layout.bounds.x + tab_layout.bounds.w / 2,
                    tab_layout.bounds.y + 2,
                    &tab.caption,
                    if active {
                        ACTIVE_TAB_COLOR
                    } else {
                        INACTIVE_TAB_COLOR
                    },
                    TextAlign::Center,
                    gamma,
                    tab_layout.bounds,
                );
            }
        }

        if let Some(graph) = self.active_graph() {
            draw_graph(surface, layout.chart, graph, resources.fonts, gamma);
        }
        Ok(())
    }
}

fn draw_icon_phase(
    surface: &mut Surface,
    icons: &ImageData,
    phase: u32,
    destination: IntRect,
    gamma: Option<&GammaRamp>,
) -> Result<()> {
    let columns = icons.width() / ICON_CELL;
    ensure!(columns != 0, "GUIIcons.png has no complete icon columns");
    let source_x = (phase % columns) * ICON_CELL;
    let source_y = (phase / columns) * ICON_CELL;
    ensure!(
        source_x + ICON_CELL <= icons.width() && source_y + ICON_CELL <= icons.height(),
        "GUIIcons.png does not contain classic icon phase {phase}"
    );
    draw_facet_stretch(
        surface,
        icons,
        (
            source_x as f32,
            source_y as f32,
            ICON_CELL as f32,
            ICON_CELL as f32,
        ),
        (
            destination.x as f32,
            destination.y as f32,
            destination.w as f32,
            destination.h as f32,
        ),
        gamma,
    );
    Ok(())
}

fn empty_tab(caption: &str, title: &str) -> NetworkChartTabSnapshot {
    NetworkChartTabSnapshot::new(caption, NetworkChartGraphSnapshot::new(title, Vec::new()))
}

fn finite_extrema(values: &[f32]) -> Option<(f32, f32)> {
    values
        .iter()
        .copied()
        .filter(|value| value.is_finite())
        .fold(None, |range, value| {
            Some(match range {
                Some((minimum, maximum)) => (minimum.min(value), maximum.max(value)),
                None => (value, value),
            })
        })
}

fn draw_graph(
    surface: &mut Surface,
    bounds: IntRect,
    graph: &NetworkChartGraphSnapshot,
    fonts: &ClonkFontSet,
    gamma: Option<&GammaRamp>,
) {
    // C4GraphCollection retains registered graphs before their first sample:
    // they immediately affect the shared zero baseline and multi-series
    // legend, even though their individual line has no values to draw yet.
    if graph.series.is_empty() {
        return;
    }
    let min_time = graph.start_time();
    let max_time = graph.end_time().saturating_sub(1);
    if min_time >= max_time {
        return;
    }

    let mut min_value = graph.min_value();
    let mut max_value = graph.max_value();
    if !min_value.is_finite() || !max_value.is_finite() {
        return;
    }
    if min_value == max_value {
        max_value += 1.0;
    }
    if min_value > 0.0 && max_value / min_value >= 2.0 {
        min_value = 0.0;
    } else if max_value < 0.0 && min_value / max_value >= 2.0 {
        max_value = 0.0;
    }
    min_value = rounded_range_limit(min_value, false);
    max_value = rounded_range_limit(max_value, true);
    let value_range = max_value - min_value;
    if !value_range.is_finite() || value_range <= 0.0 {
        return;
    }

    let axis_extent_label = format!(
        "-{}",
        max_value.abs().max(min_value.abs()).min(i32::MAX as f32) as i32
    );
    let (axis_label_width, axis_label_height) = fonts.mini.measure(&axis_extent_label, false);
    let y_axis_width = 5 + axis_label_width;
    let x_axis_height = 15 + axis_label_height;
    let x_axis_min_step = (axis_label_width + 2).max(1);
    let y_axis_min_step = (axis_label_height + 2).max(1);
    let mut plot_width = bounds.w - y_axis_width;
    let mut plot_height = bounds.h - x_axis_height;
    let plot_x = bounds.x + y_axis_width;
    let mut plot_y = bounds.y;

    if graph.series.len() > 1 {
        let legend_width = graph
            .series
            .iter()
            .map(|series| fonts.mini.measure(&series.title, true).0)
            .max()
            .unwrap_or(0);
        plot_width -= legend_width + 1;
        let legend_height = fonts.mini.line_height.max(1);
        let mut legend_y = plot_y
            + (plot_height - i32::try_from(graph.series.len()).unwrap_or(i32::MAX) * legend_height)
                / 2;
        for series in &graph.series {
            draw_clipped_text(
                surface,
                &fonts.mini,
                plot_x + plot_width,
                legend_y,
                &series.title,
                packed_rgb(series.color),
                TextAlign::Left,
                gamma,
                bounds,
            );
            legend_y += legend_height;
        }
    }
    if plot_width < 10 || plot_height < 10 {
        return;
    }

    draw_engine_line(
        surface,
        plot_x,
        plot_y + plot_height,
        plot_x + plot_width - 1,
        plot_y + plot_height,
        AXIS_COLOR,
        gamma,
        bounds,
    );
    draw_engine_line(
        surface,
        plot_x + plot_width - 1,
        plot_y + plot_height,
        plot_x + plot_width - 1 - AXIS_ARROW_LENGTH,
        plot_y + plot_height - AXIS_ARROW_THICKNESS,
        AXIS_COLOR,
        gamma,
        bounds,
    );
    draw_engine_line(
        surface,
        plot_x + plot_width - 1,
        plot_y + plot_height,
        plot_x + plot_width - 1 - AXIS_ARROW_LENGTH,
        plot_y + plot_height + AXIS_ARROW_THICKNESS,
        AXIS_COLOR,
        gamma,
        bounds,
    );
    draw_engine_line(
        surface,
        plot_x,
        plot_y,
        plot_x,
        plot_y + plot_height,
        AXIS_COLOR,
        gamma,
        bounds,
    );
    draw_engine_line(
        surface,
        plot_x,
        plot_y,
        plot_x - AXIS_ARROW_THICKNESS,
        plot_y + AXIS_ARROW_LENGTH,
        AXIS_COLOR,
        gamma,
        bounds,
    );
    draw_engine_line(
        surface,
        plot_x,
        plot_y,
        plot_x + AXIS_ARROW_THICKNESS,
        plot_y + AXIS_ARROW_LENGTH,
        AXIS_COLOR,
        gamma,
        bounds,
    );

    plot_width -= AXIS_ARROW_LENGTH + AXIS_ARROW_INDENT;
    plot_height -= AXIS_ARROW_LENGTH + AXIS_ARROW_INDENT;
    plot_y += AXIS_ARROW_LENGTH + AXIS_ARROW_INDENT;
    if plot_width <= 0 || plot_height <= 0 {
        return;
    }
    let time_range = max_time - min_time;
    let x_step = axis_step_range(time_range, plot_width / x_axis_min_step);
    let y_step = axis_step_range(value_range as i32, plot_height / y_axis_min_step);

    let mut time = first_time_marker(min_time, x_step);
    while time <= max_time {
        let x = plot_x + plot_width * (time - min_time) / time_range;
        draw_engine_line(
            surface,
            x,
            plot_y + plot_height + 1,
            x,
            plot_y + plot_height + AXIS_MARKER_LENGTH,
            AXIS_COLOR,
            gamma,
            bounds,
        );
        draw_clipped_text(
            surface,
            &fonts.mini,
            x,
            plot_y + plot_height + AXIS_MARKER_LENGTH,
            &time.to_string(),
            [127, 127, 127, 255],
            TextAlign::Center,
            gamma,
            bounds,
        );
        let next = time.saturating_add(x_step);
        if next <= time {
            break;
        }
        time = next;
    }

    let mut value_marker = first_value_marker(min_value, y_step);
    while value_marker as f32 <= max_value {
        let y = plot_y + plot_height
            - (((value_marker as f32 - min_value) / value_range) * plot_height as f32) as i32;
        draw_engine_line(
            surface,
            plot_x - AXIS_MARKER_LENGTH,
            y,
            plot_x - 1,
            y,
            AXIS_COLOR,
            gamma,
            bounds,
        );
        draw_clipped_text(
            surface,
            &fonts.mini,
            plot_x - AXIS_MARKER_LENGTH,
            y - fonts.mini.line_height / 2,
            &value_marker.to_string(),
            [127, 127, 127, 255],
            TextAlign::Right,
            gamma,
            bounds,
        );
        let next = value_marker.saturating_add(y_step);
        if next <= value_marker {
            break;
        }
        value_marker = next;
    }

    for series in &graph.series {
        let mut previous = None;
        for pixel_x in 0..plot_width {
            let sample_time = min_time + time_range * pixel_x / plot_width;
            let Some(value) = series.value(sample_time).filter(|value| value.is_finite()) else {
                previous = None;
                continue;
            };
            let y = ((-value + min_value) * plot_height as f32 / value_range) as i32
                + plot_y
                + plot_height;
            let point = (plot_x + pixel_x, y);
            if let Some((previous_x, previous_y)) = previous {
                draw_engine_line(
                    surface,
                    previous_x,
                    previous_y,
                    point.0,
                    point.1,
                    series.color,
                    gamma,
                    bounds,
                );
            }
            previous = Some(point);
        }
    }
}

fn rounded_range_limit(value: f32, upper: bool) -> f32 {
    let integer = value.clamp(i32::MIN as f32, i32::MAX as f32).trunc() as i32;
    let divider = value_decade(integer) / 50;
    if divider == 0 || (upper && value <= 0.0) || (!upper && value == 0.0) {
        return value;
    }
    let adjustment = if upper || value < 0.0 { 1.0 } else { 0.0 };
    ((value - adjustment) / divider as f32 + adjustment) * divider as f32
}

fn value_decade(mut value: i32) -> i32 {
    let mut decade = 1_i32;
    while value != 0 {
        value /= 10;
        decade = decade.saturating_mul(10);
    }
    decade
}

fn axis_step_range(range: i32, max_steps: i32) -> i32 {
    let mut decade = value_decade(range);
    if decade == 1 {
        return 1;
    }
    let mut divider = 2_i32;
    while decade >= divider && divider.saturating_mul(range) / decade <= max_steps {
        decade /= divider;
        divider = 7 - divider;
    }
    decade.max(1)
}

fn first_time_marker(minimum: i32, step: i32) -> i32 {
    let positive = if minimum > 0 { 1 } else { 0 };
    ((minimum - positive) / step + positive) * step
}

fn first_value_marker(minimum: f32, step: i32) -> i32 {
    let positive = if minimum > 0.0 { 1.0 } else { 0.0 };
    (((minimum - positive) / step as f32 + positive) * step as f32) as i32
}

fn packed_rgb(color: u32) -> [u8; 4] {
    [
        ((color >> 16) & 0xff) as u8,
        ((color >> 8) & 0xff) as u8,
        (color & 0xff) as u8,
        255,
    ]
}

#[allow(clippy::too_many_arguments)]
fn draw_engine_line(
    surface: &mut Surface,
    mut x0: i32,
    mut y0: i32,
    x1: i32,
    y1: i32,
    color: u32,
    gamma: Option<&GammaRamp>,
    clip: IntRect,
) {
    if surface.is_gpu_scene_capture_active() {
        if clip.w <= 0 || clip.h <= 0 {
            return;
        }
        let previous_clip = surface.clip();
        let chart_clip = clonk_graphics::Rect::new(clip.x, clip.y, clip.w as u32, clip.h as u32);
        let effective_clip =
            previous_clip.map_or(Some(chart_clip), |current| current.intersection(chart_clip));
        let Some(effective_clip) = effective_clip else {
            return;
        };
        surface.set_clip(effective_clip);
        draw_retained_engine_line(surface, x0, y0, x1, y1, color, gamma);
        match previous_clip {
            Some(previous) => surface.set_clip(previous),
            None => surface.clear_clip(),
        }
        return;
    }

    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut error = dx + dy;
    loop {
        if x0 >= clip.x && x0 < clip.x + clip.w && y0 >= clip.y && y0 < clip.y + clip.h {
            draw_engine_box(surface, x0, y0, x0, y0, color, gamma);
        }
        if x0 == x1 && y0 == y1 {
            break;
        }
        let doubled = 2 * error;
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

fn contains(rect: IntRect, point: GuiPoint) -> bool {
    point.x >= rect.x as f32
        && point.x < (rect.x + rect.w) as f32
        && point.y >= rect.y as f32
        && point.y < (rect.y + rect.h) as f32
}

/// A widget the layout does not carry cannot be hit.
fn contains_widget(rect: Option<IntRect>, point: GuiPoint) -> bool {
    rect.is_some_and(|rect| contains(rect, point))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retained_chart_lines_keep_native_geometry_and_alpha_provenance() {
        let mut surface = Surface::new(12, 8, clonk_graphics::PixelFormat::Rgba8888);
        surface.begin_gpu_scene_capture();
        draw_engine_line(
            &mut surface,
            2,
            2,
            9,
            5,
            0x7f20_4060,
            None,
            IntRect::new(1, 1, 10, 6),
        );

        let scene = surface
            .take_gpu_scene_capture()
            .expect("capture remains active")
            .into_scene(
                [12, 8],
                clonk_graphics::Color::transparent(),
                &GammaRamp::identity(),
            );
        let [clonk_graphics::GpuCommand::Solid {
            topology,
            alpha_mode,
            clip,
            ..
        }] = scene.commands.as_slice()
        else {
            panic!("chart line did not remain one retained solid command");
        };
        assert_eq!(*topology, clonk_graphics::GpuPrimitiveTopology::LineList);
        assert_eq!(*alpha_mode, clonk_graphics::GpuSolidAlphaMode::SourceOver);
        assert_eq!(*clip, Some(clonk_graphics::Rect::new(1, 1, 10, 6)));
    }

    #[test]
    fn native_tab_order_only_includes_pings_for_network_games() {
        assert_eq!(
            NetworkChartDialog::new(false).tab_names(),
            ["oc", "FPS", "NetIO", "Control", "APM"]
        );
        assert_eq!(
            NetworkChartDialog::new(true).tab_names(),
            ["oc", "FPS", "NetIO", "Pings", "Control", "APM"]
        );
    }

    #[test]
    fn live_snapshot_replacement_makes_a_tab_nonempty() {
        let mut dialog = NetworkChartDialog::new(false);
        let series =
            NetworkChartSeriesSnapshot::new("Object count", 0x7fff_0000, 11, vec![3.0, 4.0, 5.0]);
        assert!(dialog.set_graph_snapshot(
            "OC",
            NetworkChartGraphSnapshot::new("Object count", vec![series]),
        ));
        assert!(dialog.active_graph_has_data());
        let graph = dialog.active_graph().unwrap();
        assert_eq!((graph.start_time(), graph.end_time()), (11, 14));
        assert_eq!((graph.min_value(), graph.max_value()), (3.0, 5.0));
    }

    #[test]
    fn empty_registered_series_still_affects_scale_and_legend_membership() {
        let waiting = NetworkChartSeriesSnapshot::new("Waiting", 0x0000_ff00, 3, Vec::new());
        let sampled =
            NetworkChartSeriesSnapshot::new("Sampled", 0x00ff_0000, 5, vec![4.0, 8.0, 6.0]);
        let graph = NetworkChartGraphSnapshot::new("Pings", vec![waiting, sampled]);

        assert_eq!(graph.series_count(), 2, "both legend rows remain visible");
        assert_eq!((graph.start_time(), graph.end_time()), (3, 8));
        assert_eq!(
            (graph.min_value(), graph.max_value()),
            (0.0, 8.0),
            "the registered empty series preserves the native zero baseline"
        );
    }

    #[test]
    fn only_stronger_escape_is_active_in_the_shared_running_screen() {
        let mut dialog = NetworkChartDialog::new(false);
        assert_eq!(
            dialog.handle_key(KeyCode::Tab, true),
            NetworkChartDialogAction::Ignored
        );
        assert_eq!(dialog.active_tab_index(), 0);
        assert_eq!(
            dialog.handle_key(KeyCode::Up, true),
            NetworkChartDialogAction::Ignored
        );
        assert_eq!(dialog.active_tab_index(), 0);
        assert_eq!(
            dialog.handle_key(KeyCode::Escape, true),
            NetworkChartDialogAction::Close
        );
    }
}
