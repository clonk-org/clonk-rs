pub mod ffi;
mod scenario_browser;

use lc_graphics::Color;
use std::fmt;

pub use scenario_browser::{
    ScenarioBrowser, ScenarioBrowserMessage, ScenarioBrowserResponse, ScenarioEntry,
    ScenarioEntrySummary, ScenarioKind,
};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}

impl Size {
    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }

    fn clamp(self, constraints: LayoutConstraints) -> Self {
        let width = self.width.min(constraints.max_width).max(0.0);
        let height = self.height.min(constraints.max_height).max(0.0);
        Size { width, height }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub origin: Point,
    pub size: Size,
}

impl Rect {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            origin: Point { x, y },
            size: Size { width, height },
        }
    }

    pub const fn from_origin_size(origin: Point, size: Size) -> Self {
        Self { origin, size }
    }

    pub fn contains(&self, point: Point) -> bool {
        point.x >= self.origin.x
            && point.y >= self.origin.y
            && point.x < self.origin.x + self.size.width
            && point.y < self.origin.y + self.size.height
    }
}

#[derive(Clone, Copy, Debug)]
pub struct LayoutConstraints {
    pub max_width: f32,
    pub max_height: f32,
}

impl LayoutConstraints {
    pub fn new(max_width: f32, max_height: f32) -> Self {
        Self {
            max_width: max_width.max(0.0),
            max_height: max_height.max(0.0),
        }
    }

    pub fn tight(size: Size) -> Self {
        Self::new(size.width, size.height)
    }

    pub fn unbounded() -> Self {
        Self::new(f32::INFINITY, f32::INFINITY)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KeyCode {
    Enter,
    Escape,
    Space,
    Tab,
}

#[derive(Clone, Copy, Debug)]
pub enum GuiEvent {
    PointerDown { position: Point },
    PointerUp { position: Point },
    PointerMove { position: Point },
    KeyDown { key: KeyCode },
    KeyUp { key: KeyCode },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GuiAction {
    Activate,
}

#[derive(Debug, Default)]
pub struct GuiEventResult {
    pub captured: bool,
    pub actions: Vec<(WidgetId, GuiAction)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WidgetId(usize);

impl WidgetId {
    pub(crate) const fn new(index: usize) -> Self {
        Self(index)
    }

    pub const fn to_raw(self) -> usize {
        self.0
    }

    pub(crate) fn index(self) -> usize {
        self.0
    }

    pub(crate) const fn from_raw(index: usize) -> Self {
        Self(index)
    }
}

impl fmt::Display for WidgetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuiError {
    InvalidWidget(WidgetId),
    WrongWidgetType {
        id: WidgetId,
        expected: &'static str,
        found: &'static str,
    },
}

impl fmt::Display for GuiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GuiError::InvalidWidget(id) => write!(f, "invalid widget id {}", id),
            GuiError::WrongWidgetType {
                id,
                expected,
                found,
            } => write!(
                f,
                "widget {} expected kind {} but found {}",
                id, expected, found
            ),
        }
    }
}

impl std::error::Error for GuiError {}

pub type GuiResult<T> = Result<T, GuiError>;

#[derive(Debug, Clone)]
struct Column {
    padding: f32,
    spacing: f32,
    expand_width: bool,
}

impl Column {
    fn new(padding: f32, spacing: f32, expand_width: bool) -> Self {
        Self {
            padding,
            spacing,
            expand_width,
        }
    }
}

impl Default for Column {
    fn default() -> Self {
        Column::new(8.0, 6.0, false)
    }
}

#[derive(Debug, Clone)]
struct Row {
    padding: f32,
    spacing: f32,
    expand_height: bool,
}

impl Row {
    fn new(padding: f32, spacing: f32, expand_height: bool) -> Self {
        Self {
            padding,
            spacing,
            expand_height,
        }
    }
}

impl Default for Row {
    fn default() -> Self {
        Row::new(8.0, 6.0, false)
    }
}

#[derive(Debug, Clone)]
struct Label {
    text: String,
    color: Color,
    font_size: f32,
    padding: f32,
}

impl Label {
    fn new(text: String) -> Self {
        Self {
            text,
            color: Color::opaque(235, 235, 235),
            font_size: 16.0,
            padding: 4.0,
        }
    }

    fn intrinsic_size(&self) -> Size {
        let measured = measure_text(&self.text, self.font_size);
        Size::new(
            measured.width + self.padding * 2.0,
            measured.height + self.padding * 2.0,
        )
    }
}

#[derive(Debug, Clone)]
struct Button {
    text: String,
    font_size: f32,
    padding: f32,
    min_width: f32,
    normal_color: Color,
    pressed_color: Color,
    selected_color: Color,
    disabled_color: Color,
    text_color: Color,
    disabled_text_color: Color,
    pressed: bool,
    selected: bool,
    enabled: bool,
}

impl Button {
    fn new(text: String) -> Self {
        Self {
            text,
            font_size: 16.0,
            padding: 6.0,
            min_width: 96.0,
            normal_color: Color::opaque(48, 96, 160),
            pressed_color: Color::opaque(28, 68, 120),
            selected_color: Color::opaque(72, 128, 196),
            disabled_color: Color::opaque(36, 44, 60),
            text_color: Color::opaque(255, 255, 255),
            disabled_text_color: Color::opaque(180, 188, 200),
            pressed: false,
            selected: false,
            enabled: true,
        }
    }

    fn intrinsic_size(&self) -> Size {
        let measured = measure_text(&self.text, self.font_size);
        let width = (measured.width + self.padding * 2.0).max(self.min_width);
        let height = measured.height + self.padding * 2.0;
        Size::new(width, height)
    }

    fn current_color(&self) -> Color {
        if !self.enabled {
            self.disabled_color
        } else if self.pressed {
            self.pressed_color
        } else if self.selected {
            self.selected_color
        } else {
            self.normal_color
        }
    }

    fn current_text_color(&self) -> Color {
        if self.enabled {
            self.text_color
        } else {
            self.disabled_text_color
        }
    }
}

#[derive(Debug)]
struct WidgetNode {
    id: WidgetId,
    parent: Option<WidgetId>,
    children: Vec<WidgetId>,
    kind: WidgetKind,
    rect: Rect,
}

#[derive(Debug)]
enum WidgetKind {
    Column(Column),
    Row(Row),
    Label(Label),
    Button(Button),
}

impl WidgetKind {
    fn name(&self) -> &'static str {
        match self {
            WidgetKind::Column(_) => "column",
            WidgetKind::Row(_) => "row",
            WidgetKind::Label(_) => "label",
            WidgetKind::Button(_) => "button",
        }
    }
}

impl WidgetNode {
    fn new(id: WidgetId, parent: Option<WidgetId>, kind: WidgetKind) -> Self {
        Self {
            id,
            parent,
            children: Vec::new(),
            kind,
            rect: Rect::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum DrawCommand {
    Quad {
        rect: Rect,
        color: Color,
    },
    Text {
        rect: Rect,
        text: String,
        color: Color,
    },
}

pub struct Gui {
    nodes: Vec<WidgetNode>,
    root: WidgetId,
    next_id: usize,
    pressed_button: Option<WidgetId>,
}

impl Gui {
    pub fn new() -> Self {
        let root_id = WidgetId::new(0);
        let root_node = WidgetNode::new(
            root_id,
            None,
            WidgetKind::Column(Column::new(8.0, 6.0, true)),
        );
        Self {
            nodes: vec![root_node],
            root: root_id,
            next_id: 1,
            pressed_button: None,
        }
    }

    pub fn root(&self) -> WidgetId {
        self.root
    }

    pub fn add_column(&mut self, parent: WidgetId, expand_width: bool) -> WidgetId {
        let id = self.alloc_id();
        let node = WidgetNode::new(
            id,
            Some(parent),
            WidgetKind::Column(Column::new(8.0, 6.0, expand_width)),
        );
        self.attach_child(parent, node);
        id
    }

    pub fn add_row(&mut self, parent: WidgetId, expand_height: bool) -> WidgetId {
        let id = self.alloc_id();
        let node = WidgetNode::new(
            id,
            Some(parent),
            WidgetKind::Row(Row::new(8.0, 6.0, expand_height)),
        );
        self.attach_child(parent, node);
        id
    }

    pub fn add_label(&mut self, parent: WidgetId, text: impl Into<String>) -> WidgetId {
        let id = self.alloc_id();
        let node = WidgetNode::new(id, Some(parent), WidgetKind::Label(Label::new(text.into())));
        self.attach_child(parent, node);
        id
    }

    pub fn add_button(&mut self, parent: WidgetId, text: impl Into<String>) -> WidgetId {
        let id = self.alloc_id();
        let node = WidgetNode::new(
            id,
            Some(parent),
            WidgetKind::Button(Button::new(text.into())),
        );
        self.attach_child(parent, node);
        id
    }

    pub fn set_label_text(&mut self, id: WidgetId, text: impl Into<String>) -> GuiResult<()> {
        let node = self.widget_mut(id)?;
        match &mut node.kind {
            WidgetKind::Label(label) => {
                label.text = text.into();
                Ok(())
            }
            kind => Err(wrong_widget_type(id, "label", kind)),
        }
    }

    pub fn set_button_text(&mut self, id: WidgetId, text: impl Into<String>) -> GuiResult<()> {
        let node = self.widget_mut(id)?;
        match &mut node.kind {
            WidgetKind::Button(button) => {
                button.text = text.into();
                Ok(())
            }
            kind => Err(wrong_widget_type(id, "button", kind)),
        }
    }

    pub fn set_button_selected(&mut self, id: WidgetId, selected: bool) -> GuiResult<()> {
        let node = self.widget_mut(id)?;
        match &mut node.kind {
            WidgetKind::Button(button) => {
                button.selected = selected;
                Ok(())
            }
            kind => Err(wrong_widget_type(id, "button", kind)),
        }
    }

    pub fn set_button_enabled(&mut self, id: WidgetId, enabled: bool) -> GuiResult<()> {
        let node = self.widget_mut(id)?;
        match &mut node.kind {
            WidgetKind::Button(button) => {
                button.enabled = enabled;
                if !enabled {
                    button.pressed = false;
                }
                Ok(())
            }
            kind => Err(wrong_widget_type(id, "button", kind)),
        }
    }

    pub fn layout(&mut self, available: Size) -> Size {
        let constraints = LayoutConstraints::tight(available);
        self.layout_with_constraints(constraints)
    }

    pub fn layout_with_constraints(&mut self, constraints: LayoutConstraints) -> Size {
        self.layout_node(self.root, constraints, Point::new(0.0, 0.0))
    }

    pub fn render(&self) -> Vec<DrawCommand> {
        let mut commands = Vec::new();
        self.render_node(self.root, &mut commands);
        commands
    }

    pub fn handle_event(&mut self, event: GuiEvent) -> GuiEventResult {
        match event {
            GuiEvent::PointerDown { position } => self.handle_pointer_down(position),
            GuiEvent::PointerUp { position } => self.handle_pointer_up(position),
            GuiEvent::PointerMove { position } => self.handle_pointer_move(position),
            GuiEvent::KeyDown { .. } | GuiEvent::KeyUp { .. } => GuiEventResult::default(),
        }
    }

    pub fn rect_of(&self, id: WidgetId) -> Option<Rect> {
        self.nodes.get(id.index()).map(|node| node.rect)
    }

    fn widget_mut(&mut self, id: WidgetId) -> GuiResult<&mut WidgetNode> {
        if id.index() >= self.nodes.len() {
            return Err(GuiError::InvalidWidget(id));
        }
        Ok(&mut self.nodes[id.index()])
    }

    fn handle_pointer_down(&mut self, position: Point) -> GuiEventResult {
        if let Some(id) = self.hit_test(self.root, position) {
            if let WidgetKind::Button(button) = &mut self.nodes[id.index()].kind {
                if button.enabled {
                    button.pressed = true;
                    self.pressed_button = Some(id);
                    return GuiEventResult {
                        captured: true,
                        actions: Vec::new(),
                    };
                }
            }
        }
        GuiEventResult::default()
    }

    fn handle_pointer_up(&mut self, position: Point) -> GuiEventResult {
        let mut result = GuiEventResult::default();
        if let Some(active) = self.pressed_button.take() {
            let inside = self
                .nodes
                .get(active.index())
                .map(|node| node.rect.contains(position))
                .unwrap_or(false);
            if let WidgetKind::Button(button) = &mut self.nodes[active.index()].kind {
                button.pressed = false;
                if button.enabled && inside {
                    result.captured = true;
                    result.actions.push((active, GuiAction::Activate));
                }
            }
        }
        result
    }

    fn handle_pointer_move(&mut self, position: Point) -> GuiEventResult {
        if let Some(active) = self.pressed_button {
            let rect = self.nodes[active.index()].rect;
            if let WidgetKind::Button(button) = &mut self.nodes[active.index()].kind {
                if button.enabled {
                    button.pressed = rect.contains(position);
                    return GuiEventResult {
                        captured: true,
                        actions: Vec::new(),
                    };
                }
            }
        }
        GuiEventResult::default()
    }

    fn render_node(&self, id: WidgetId, commands: &mut Vec<DrawCommand>) {
        let node = &self.nodes[id.index()];
        match &node.kind {
            WidgetKind::Button(button) => {
                commands.push(DrawCommand::Quad {
                    rect: node.rect,
                    color: button.current_color(),
                });
                commands.push(DrawCommand::Text {
                    rect: node.rect,
                    text: button.text.clone(),
                    color: button.current_text_color(),
                });
            }
            WidgetKind::Label(label) => {
                commands.push(DrawCommand::Text {
                    rect: node.rect,
                    text: label.text.clone(),
                    color: label.color,
                });
            }
            WidgetKind::Column(_) | WidgetKind::Row(_) => {}
        }

        for child in &node.children {
            self.render_node(*child, commands);
        }
    }

    fn layout_node(&mut self, id: WidgetId, constraints: LayoutConstraints, origin: Point) -> Size {
        match self.nodes[id.index()].kind {
            WidgetKind::Label(_) => self.layout_label(id, constraints, origin),
            WidgetKind::Button(_) => self.layout_button(id, constraints, origin),
            WidgetKind::Row(_) => self.layout_row(id, constraints, origin),
            WidgetKind::Column(_) => self.layout_column(id, constraints, origin),
        }
    }

    fn layout_label(
        &mut self,
        id: WidgetId,
        constraints: LayoutConstraints,
        origin: Point,
    ) -> Size {
        let intrinsic = {
            let label = match &self.nodes[id.index()].kind {
                WidgetKind::Label(label) => label,
                _ => unreachable!(),
            };
            label.intrinsic_size()
        };
        let size = intrinsic.clamp(constraints);
        self.nodes[id.index()].rect = Rect::from_origin_size(origin, size);
        size
    }

    fn layout_button(
        &mut self,
        id: WidgetId,
        constraints: LayoutConstraints,
        origin: Point,
    ) -> Size {
        let intrinsic = {
            let button = match &self.nodes[id.index()].kind {
                WidgetKind::Button(button) => button,
                _ => unreachable!(),
            };
            button.intrinsic_size()
        };
        let size = intrinsic.clamp(constraints);
        self.nodes[id.index()].rect = Rect::from_origin_size(origin, size);
        size
    }

    fn layout_row(&mut self, id: WidgetId, constraints: LayoutConstraints, origin: Point) -> Size {
        let (padding, spacing, expand_height) = {
            let row = match &self.nodes[id.index()].kind {
                WidgetKind::Row(row) => row,
                _ => unreachable!(),
            };
            (row.padding, row.spacing, row.expand_height)
        };

        let children = self.nodes[id.index()].children.clone();
        let mut x = origin.x + padding;
        let mut total_width: f32 = 0.0;
        let mut max_height: f32 = 0.0;
        for (i, child) in children.iter().enumerate() {
            if i > 0 {
                x += spacing;
                total_width += spacing;
            }
            let child_constraints = LayoutConstraints::new(
                constraints.max_width,
                if constraints.max_height.is_finite() {
                    (constraints.max_height - padding * 2.0).max(0.0)
                } else {
                    f32::INFINITY
                },
            );
            let child_origin = Point::new(x, origin.y + padding);
            let child_size = self.layout_node(*child, child_constraints, child_origin);
            x += child_size.width;
            total_width += child_size.width;
            max_height = max_height.max(child_size.height);
        }

        let content_width = if children.is_empty() {
            0.0
        } else {
            total_width
        };
        let mut width = padding * 2.0 + content_width;
        if constraints.max_width.is_finite() {
            width = width.min(constraints.max_width);
        }
        let mut height = padding * 2.0 + max_height;
        if expand_height && constraints.max_height.is_finite() {
            height = constraints.max_height;
        } else if constraints.max_height.is_finite() {
            height = height.min(constraints.max_height);
        }
        let size = Size::new(width, height);
        self.nodes[id.index()].rect = Rect::from_origin_size(origin, size);
        size
    }

    fn layout_column(
        &mut self,
        id: WidgetId,
        constraints: LayoutConstraints,
        origin: Point,
    ) -> Size {
        let (padding, spacing, expand_width) = {
            let column = match &self.nodes[id.index()].kind {
                WidgetKind::Column(column) => column,
                _ => unreachable!(),
            };
            (column.padding, column.spacing, column.expand_width)
        };

        let children = self.nodes[id.index()].children.clone();
        let mut y = origin.y + padding;
        let mut max_width: f32 = 0.0;
        for (i, child) in children.iter().enumerate() {
            if i > 0 {
                y += spacing;
            }
            let child_constraints = LayoutConstraints::new(
                if constraints.max_width.is_finite() {
                    (constraints.max_width - padding * 2.0).max(0.0)
                } else {
                    f32::INFINITY
                },
                constraints.max_height,
            );
            let child_origin = Point::new(origin.x + padding, y);
            let child_size = self.layout_node(*child, child_constraints, child_origin);
            y += child_size.height;
            max_width = max_width.max(child_size.width);
        }

        let content_height = if children.is_empty() {
            0.0
        } else {
            y - (origin.y + padding)
        };
        let mut width = max_width + padding * 2.0;
        if expand_width && constraints.max_width.is_finite() {
            width = constraints.max_width;
        } else if constraints.max_width.is_finite() {
            width = width.min(constraints.max_width);
        }
        let mut height = padding * 2.0 + content_height;
        if constraints.max_height.is_finite() {
            height = height.min(constraints.max_height);
        }
        let size = Size::new(width, height);
        self.nodes[id.index()].rect = Rect::from_origin_size(origin, size);
        size
    }

    fn hit_test(&self, id: WidgetId, point: Point) -> Option<WidgetId> {
        let node = &self.nodes[id.index()];
        if !node.rect.contains(point) {
            return None;
        }
        for child in node.children.iter().rev() {
            if let Some(hit) = self.hit_test(*child, point) {
                return Some(hit);
            }
        }
        Some(id)
    }

    fn attach_child(&mut self, parent: WidgetId, node: WidgetNode) {
        let index = node.id.index();
        if self.nodes.len() != index {
            panic!(
                "widget id mismatch: expected {} got {}",
                self.nodes.len(),
                index
            );
        }
        self.nodes.push(node);
        self.nodes[parent.index()]
            .children
            .push(WidgetId::new(index));
    }

    fn alloc_id(&mut self) -> WidgetId {
        let id = WidgetId::new(self.next_id);
        self.next_id += 1;
        id
    }
}

fn measure_text(text: &str, font_size: f32) -> Size {
    let glyph_width = (font_size * 0.6).max(4.0);
    let mut max_width: f32 = 0.0;
    let mut line_width: f32 = 0.0;
    let mut lines = 1;
    for ch in text.chars() {
        if ch == '\n' {
            max_width = max_width.max(line_width);
            line_width = 0.0;
            lines += 1;
            continue;
        }
        line_width += match ch {
            ' ' => glyph_width * 0.5,
            '\t' => glyph_width * 2.0,
            _ => glyph_width,
        };
    }
    if text.is_empty() {
        line_width = glyph_width * 0.5;
    }
    max_width = max_width.max(line_width);
    Size::new(max_width, font_size * lines as f32)
}

fn wrong_widget_type(id: WidgetId, expected: &'static str, kind: &WidgetKind) -> GuiError {
    GuiError::WrongWidgetType {
        id,
        expected,
        found: kind.name(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn center(rect: Rect) -> Point {
        Point::new(
            rect.origin.x + rect.size.width * 0.5,
            rect.origin.y + rect.size.height * 0.5,
        )
    }

    #[test]
    fn column_layout_places_children_vertically() {
        let mut gui = Gui::new();
        let root = gui.root();
        let label = gui.add_label(root, "Ready?");
        let button = gui.add_button(root, "Start Game");

        gui.layout(Size::new(300.0, 200.0));

        let label_rect = gui.rect_of(label).unwrap();
        let button_rect = gui.rect_of(button).unwrap();

        assert!(button_rect.origin.y > label_rect.origin.y + label_rect.size.height);
        assert!((label_rect.origin.x - button_rect.origin.x).abs() < f32::EPSILON);
    }

    #[test]
    fn button_click_generates_activate_action() {
        let mut gui = Gui::new();
        let root = gui.root();
        let button = gui.add_button(root, "Launch");

        gui.layout(Size::new(200.0, 120.0));
        let button_rect = gui.rect_of(button).unwrap();
        let inside = center(button_rect);

        let down = gui.handle_event(GuiEvent::PointerDown { position: inside });
        assert!(down.captured);
        if let WidgetKind::Button(button_state) = &gui.nodes[button.index()].kind {
            assert!(button_state.pressed);
        }

        let up = gui.handle_event(GuiEvent::PointerUp { position: inside });
        assert!(up.captured);
        assert_eq!(up.actions.len(), 1);
        assert_eq!(up.actions[0], (button, GuiAction::Activate));
    }

    #[test]
    fn button_release_outside_cancels_action() {
        let mut gui = Gui::new();
        let root = gui.root();
        let button = gui.add_button(root, "Abort");

        gui.layout(Size::new(200.0, 120.0));
        let button_rect = gui.rect_of(button).unwrap();
        let inside = center(button_rect);
        let outside = Point::new(button_rect.origin.x - 10.0, button_rect.origin.y - 10.0);

        gui.handle_event(GuiEvent::PointerDown { position: inside });
        let up = gui.handle_event(GuiEvent::PointerUp { position: outside });
        assert!(!up.captured);
        assert!(up.actions.is_empty());
    }

    #[test]
    fn render_emits_draw_commands() {
        let mut gui = Gui::new();
        let root = gui.root();
        gui.add_label(root, "Headline");
        gui.add_button(root, "Continue");

        gui.layout(Size::new(320.0, 180.0));
        let commands = gui.render();
        assert!(commands
            .iter()
            .any(|cmd| matches!(cmd, DrawCommand::Quad { .. })));
        assert!(commands
            .iter()
            .any(|cmd| matches!(cmd, DrawCommand::Text { text, .. } if text == "Continue")));
    }
}
