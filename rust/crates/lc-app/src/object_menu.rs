use std::collections::HashMap;

use lc_engine::{CommandKind, ControlCommand, Engine, ObjectId, SimulationSnapshot};
use lc_graphics::{Color, Rect, Surface, TextFont};

const BACKDROP_COLOR: Color = Color::new(0, 0, 0, 172);
const PANEL_COLOR: Color = Color::new(18, 28, 48, 240);
const PANEL_BORDER: Color = Color::new(210, 224, 255, 220);
const HIGHLIGHT_COLOR: Color = Color::new(58, 92, 164, 220);
const TITLE_COLOR: Color = Color::opaque(240, 244, 255);
const TEXT_COLOR: Color = Color::opaque(214, 220, 235);
const EMPHASIS_TEXT_COLOR: Color = Color::opaque(255, 255, 255);
const MUTED_TEXT_COLOR: Color = Color::opaque(144, 152, 166);

const PANEL_WIDTH_MIN: i32 = 340;
const PANEL_WIDTH_MAX: i32 = 720;
const PANEL_PADDING: i32 = 24;
const TITLE_GAP: i32 = 28;
const ITEM_HEIGHT: i32 = 42;
const ITEM_SPACING: i32 = 4;
const TITLE_FONT_SIZE: f32 = 22.0;
const ITEM_FONT_SIZE: f32 = 18.0;
const DETAIL_FONT_SIZE: f32 = 14.0;

#[derive(Clone, Debug)]
struct ObjectMenuItem {
    label: String,
    definition_id: String,
    instances: Vec<ObjectId>,
    description: Option<String>,
}

impl ObjectMenuItem {
    fn new(
        label: impl Into<String>,
        definition_id: impl Into<String>,
        description: Option<String>,
        primary: ObjectId,
    ) -> Self {
        Self {
            label: label.into(),
            definition_id: definition_id.into(),
            instances: vec![primary],
            description,
        }
    }

    fn push_instance(&mut self, id: ObjectId) {
        if !self.instances.contains(&id) {
            self.instances.push(id);
        }
    }

    fn count(&self) -> usize {
        self.instances.len()
    }

    fn primary_object(&self) -> Option<ObjectId> {
        self.instances.first().copied()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectMenuCommand {
    Focus,
    DropAll,
}

#[derive(Clone, Debug)]
pub enum ObjectMenuAction {
    Close,
    Execute {
        command: ObjectMenuCommand,
        selection: ObjectMenuSelection,
    },
}

#[derive(Clone, Debug)]
pub struct ObjectMenuSelection {
    pub crew_id: ObjectId,
    pub primary_id: ObjectId,
    pub instances: Vec<ObjectId>,
    pub label: String,
}

impl ObjectMenuSelection {
    pub fn count(&self) -> usize {
        self.instances.len()
    }
}

#[derive(Clone, Debug)]
pub struct ObjectMenuState {
    crew_id: ObjectId,
    crew_label: String,
    items: Vec<ObjectMenuItem>,
    selected: Option<usize>,
    inventory_empty: bool,
}

impl ObjectMenuState {
    pub fn for_player(owner: i32, engine: &Engine, snapshot: &SimulationSnapshot) -> Option<Self> {
        let cursor = engine.crew_cursor(owner)?;
        Self::new(engine, snapshot, cursor)
    }

    pub fn new(engine: &Engine, snapshot: &SimulationSnapshot, crew_id: ObjectId) -> Option<Self> {
        let crew = snapshot.object(crew_id)?.clone();
        let crew_label = engine
            .definition_name(&crew.definition_id)
            .unwrap_or(&crew.definition_id)
            .to_string();
        let items = collect_inventory(engine, snapshot, &crew);
        let selected = if items.is_empty() { None } else { Some(0) };
        let inventory_empty = items.is_empty();
        Some(Self {
            crew_id,
            crew_label,
            items,
            selected,
            inventory_empty,
        })
    }

    pub fn refresh(&mut self, engine: &Engine, snapshot: &SimulationSnapshot) -> bool {
        let crew = match snapshot.object(self.crew_id) {
            Some(crew) => crew,
            None => return false,
        };
        self.crew_label = engine
            .definition_name(&crew.definition_id)
            .unwrap_or(&crew.definition_id)
            .to_string();
        self.items = collect_inventory(engine, snapshot, crew);
        self.inventory_empty = self.items.is_empty();
        if let Some(selected) = self.selected {
            if self.items.is_empty() {
                self.selected = None;
            } else if selected >= self.items.len() {
                self.selected = Some(self.items.len() - 1);
            }
        } else if !self.items.is_empty() {
            self.selected = Some(0);
        }
        true
    }

    pub fn handle_command(
        &mut self,
        command: ControlCommand,
        kind: CommandKind,
    ) -> Option<ObjectMenuAction> {
        if !matches!(
            kind,
            CommandKind::Press | CommandKind::Single | CommandKind::Double
        ) {
            return None;
        }

        match command {
            ControlCommand::MenuUp | ControlCommand::MenuLeft => {
                self.advance_selection(-1);
                None
            }
            ControlCommand::MenuDown | ControlCommand::MenuRight => {
                self.advance_selection(1);
                None
            }
            ControlCommand::MenuSelect | ControlCommand::MenuEnter => {
                self.activation_action(ObjectMenuCommand::Focus)
            }
            ControlCommand::MenuEnterAll => self.activation_action(ObjectMenuCommand::DropAll),
            ControlCommand::MenuClose => Some(ObjectMenuAction::Close),
            ControlCommand::MenuShowText => None,
            _ => None,
        }
    }

    pub fn render(&self, surface: &mut Surface, font: &dyn TextFont) {
        let width = surface.width() as i32;
        let height = surface.height() as i32;
        if width <= 0 || height <= 0 {
            return;
        }

        fill_rect(surface, surface.bounds(), BACKDROP_COLOR);

        let mut panel_width = (width as f32 * 0.42).round() as i32;
        panel_width = panel_width.clamp(PANEL_WIDTH_MIN, PANEL_WIDTH_MAX);
        panel_width = panel_width
            .min(width - PANEL_PADDING * 2)
            .max(PANEL_WIDTH_MIN);

        let list_height = if self.items.is_empty() {
            ITEM_HEIGHT
        } else {
            (self.items.len() as i32).saturating_mul(ITEM_HEIGHT + ITEM_SPACING) - ITEM_SPACING
        };
        let panel_height = (PANEL_PADDING * 2) + TITLE_GAP + list_height.max(ITEM_HEIGHT);
        let panel_x = (width - panel_width) / 2;
        let panel_y = (height - panel_height) / 2;

        let panel_rect = Rect::new(panel_x, panel_y, panel_width as u32, panel_height as u32);
        fill_rect(surface, panel_rect, PANEL_COLOR);
        draw_border(surface, panel_rect, PANEL_BORDER);

        let mut cursor_y = panel_y + PANEL_PADDING;
        let title = format!("{} Inventory", self.crew_label);
        font.draw_text(
            surface,
            (panel_x + PANEL_PADDING) as f32,
            cursor_y as f32,
            &title,
            TITLE_FONT_SIZE,
            TITLE_COLOR,
        );

        cursor_y += TITLE_GAP;
        if self.items.is_empty() {
            font.draw_text(
                surface,
                (panel_x + PANEL_PADDING) as f32,
                (cursor_y + 10) as f32,
                if self.inventory_empty {
                    "Inventory is empty."
                } else {
                    "Inventory unavailable."
                },
                ITEM_FONT_SIZE,
                MUTED_TEXT_COLOR,
            );
            return;
        }

        for (index, item) in self.items.iter().enumerate() {
            let row_rect = Rect::new(
                panel_x + PANEL_PADDING,
                cursor_y,
                (panel_width - PANEL_PADDING * 2) as u32,
                ITEM_HEIGHT as u32,
            );
            if Some(index) == self.selected {
                fill_rect(surface, row_rect, HIGHLIGHT_COLOR);
            }

            let primary_color = if Some(index) == self.selected {
                EMPHASIS_TEXT_COLOR
            } else {
                TEXT_COLOR
            };

            let count = item.count();
            let label_text = if count > 1 {
                format!("{} (x{count})", item.label)
            } else {
                item.label.clone()
            };
            font.draw_text(
                surface,
                (row_rect.x + 12) as f32,
                (row_rect.y + 8) as f32,
                &label_text,
                ITEM_FONT_SIZE,
                primary_color,
            );

            if let Some(description) = item.description.as_ref() {
                font.draw_text(
                    surface,
                    (row_rect.x + 12) as f32,
                    (row_rect.y + 22) as f32,
                    description,
                    DETAIL_FONT_SIZE,
                    MUTED_TEXT_COLOR,
                );
            }

            cursor_y += ITEM_HEIGHT + ITEM_SPACING;
        }
    }

    fn advance_selection(&mut self, delta: i32) {
        if self.items.is_empty() {
            return;
        }
        let len = self.items.len() as i32;
        let next = match self.selected {
            Some(index) => (index as i32 + delta).rem_euclid(len),
            None => {
                if delta >= 0 {
                    0
                } else {
                    len - 1
                }
            }
        };
        self.selected = Some(next as usize);
    }

    fn activation_action(&self, command: ObjectMenuCommand) -> Option<ObjectMenuAction> {
        let index = self.selected?;
        let item = self.items.get(index)?;
        let primary_id = item.primary_object()?;
        Some(ObjectMenuAction::Execute {
            command,
            selection: ObjectMenuSelection {
                crew_id: self.crew_id,
                primary_id,
                instances: item.instances.clone(),
                label: item.label.clone(),
            },
        })
    }
}

fn collect_inventory(
    engine: &Engine,
    snapshot: &SimulationSnapshot,
    crew: &lc_engine::ObjectSnapshot,
) -> Vec<ObjectMenuItem> {
    if crew.contents.is_empty() {
        return Vec::new();
    }

    let mut order: Vec<ObjectMenuItem> = Vec::new();
    let mut lookup: HashMap<String, usize> = HashMap::new();

    for child_id in &crew.contents {
        let child = match snapshot.object(*child_id) {
            Some(child) => child,
            None => continue,
        };
        let name = engine
            .definition_name(&child.definition_id)
            .unwrap_or(&child.definition_id);
        if let Some(index) = lookup.get(&child.definition_id).copied() {
            if let Some(entry) = order.get_mut(index) {
                entry.push_instance(child.id);
            }
        } else {
            let index = order.len();
            let entry = ObjectMenuItem::new(name, &child.definition_id, None, child.id);
            order.push(entry);
            lookup.insert(child.definition_id.clone(), index);
        }
    }
    order
}

fn fill_rect(surface: &mut Surface, rect: Rect, color: Color) {
    if let Some(clipped) = rect.intersection(surface.bounds()) {
        for y in clipped.y..(clipped.y + clipped.height as i32) {
            for x in clipped.x..(clipped.x + clipped.width as i32) {
                let result = if color.a == 255 {
                    surface.set_pixel(x as u32, y as u32, color)
                } else {
                    surface.blend_pixel(x as u32, y as u32, color)
                };
                if result.is_err() {
                    break;
                }
            }
        }
    }
}

fn draw_border(surface: &mut Surface, rect: Rect, color: Color) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    let top = Rect::new(rect.x, rect.y, rect.width, 1);
    let bottom = Rect::new(rect.x, rect.y + rect.height as i32 - 1, rect.width, 1);
    let left = Rect::new(rect.x, rect.y, 1, rect.height);
    let right = Rect::new(rect.x + rect.width as i32 - 1, rect.y, 1, rect.height);
    fill_rect(surface, top, color);
    fill_rect(surface, bottom, color);
    fill_rect(surface, left, color);
    fill_rect(surface, right, color);
}

#[cfg(test)]
mod tests {
    use super::*;
    use lc_engine::{Engine, ObjectSnapshot, ObjectStatus, Vector2};
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    fn make_object(id: u64, definition: &str) -> ObjectSnapshot {
        ObjectSnapshot {
            id: ObjectId::new(id),
            definition_id: definition.to_string(),
            position: Vector2::new(0, 0),
            velocity: Vector2::new(0, 0),
            energy: 100,
            damage: 0,
            action: Default::default(),
            direction: Default::default(),
            command_direction: Default::default(),
            action_procedure: None,
            effects: Vec::new(),
            vertices: Vec::new(),
            container: None,
            contents: Vec::new(),
            status: ObjectStatus::Normal,
            owner: 1,
            category: 0,
            crew_member: false,
            alive: true,
        }
    }

    fn make_snapshot(
        mut crew: ObjectSnapshot,
        contents: Vec<ObjectSnapshot>,
    ) -> SimulationSnapshot {
        let mut objects = Vec::new();
        let mut crew_contents = Vec::new();
        for object in contents {
            crew_contents.push(object.id);
            objects.push(object);
        }
        crew.contents = crew_contents;
        crew.crew_member = true;
        objects.insert(0, crew);
        SimulationSnapshot {
            frame: 0,
            physics: None,
            objects,
            environment: Default::default(),
            global_effects: Vec::new(),
            particles: Vec::new(),
            players: Vec::new(),
            crew_selection: HashMap::new(),
            crew_roles: HashMap::new(),
            known_crew_owners: Vec::new(),
            eliminated_crew_owners: Vec::new(),
            landscape: None,
            rng: ChaCha8Rng::seed_from_u64(42),
            surfaces: Vec::new(),
            hud: Default::default(),
            controls: Vec::new(),
            network_packets: Vec::new(),
            definition_categories: HashMap::new(),
            transfer_zones: Vec::new(),
            audio: Vec::new(),
        }
    }

    #[test]
    fn inventory_groups_by_definition() {
        let engine = Engine::new();
        let crew = make_object(1, "Clonk");
        let contents = vec![
            make_object(2, "Shovel"),
            make_object(3, "Shovel"),
            make_object(4, "Hammer"),
        ];
        let snapshot = make_snapshot(crew.clone(), contents);
        let mut menu =
            ObjectMenuState::new(&engine, &snapshot, crew.id).expect("menu should exist");
        assert_eq!(menu.items.len(), 2);
        assert_eq!(menu.items[0].definition_id, "Shovel");
        assert_eq!(menu.items[0].count(), 2);
        assert_eq!(menu.items[1].definition_id, "Hammer");
        assert_eq!(menu.items[1].count(), 1);

        // Simulate removing an item and refreshing.
        let mut snapshot_updated = snapshot.clone();
        if let Some(crew_obj) = snapshot_updated.objects.get_mut(0) {
            crew_obj.contents.pop();
        }
        assert!(menu.refresh(&engine, &snapshot_updated));
        assert_eq!(menu.items.len(), 1);
        assert_eq!(menu.items[0].count(), 2);
    }

    #[test]
    fn menu_enter_all_emits_drop_action() {
        let engine = Engine::new();
        let crew = make_object(1, "Clonk");
        let contents = vec![
            make_object(2, "Shovel"),
            make_object(3, "Shovel"),
            make_object(4, "Hammer"),
        ];
        let snapshot = make_snapshot(crew.clone(), contents);
        let mut menu =
            ObjectMenuState::new(&engine, &snapshot, crew.id).expect("menu should exist");
        let action = menu
            .handle_command(ControlCommand::MenuEnterAll, CommandKind::Press)
            .expect("drop action");
        match action {
            ObjectMenuAction::Execute { command, selection } => {
                assert_eq!(command, ObjectMenuCommand::DropAll);
                assert_eq!(selection.crew_id, crew.id);
                assert_eq!(selection.label, "Shovel");
                assert_eq!(selection.count(), 2);
                assert_eq!(selection.instances.len(), 2);
                assert_eq!(selection.primary_id, ObjectId::new(2));
            }
            _ => panic!("expected execute action"),
        }
    }
}
