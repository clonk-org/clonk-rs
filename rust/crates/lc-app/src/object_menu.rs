use std::collections::HashMap;

use lc_engine::{CommandKind, ControlCommand, Engine, ObjectId, SimulationSnapshot, OWNER_NONE};
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
const MODE_HINT: &str = "Press ←/→ to switch menus";

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

trait MenuEntry {
    fn label(&self) -> &str;
    fn description(&self) -> Option<&str>;
    fn count(&self) -> usize;
}

impl MenuEntry for ObjectMenuItem {
    fn label(&self) -> &str {
        &self.label
    }

    fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    fn count(&self) -> usize {
        self.count()
    }
}

#[derive(Clone, Debug)]
struct BuildMenuItem {
    definition_id: String,
    label: String,
    description: Option<String>,
    available: u32,
}

impl BuildMenuItem {
    fn available(&self) -> u32 {
        self.available
    }
}

impl MenuEntry for BuildMenuItem {
    fn label(&self) -> &str {
        &self.label
    }

    fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    fn count(&self) -> usize {
        self.available as usize
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MenuMode {
    Inventory,
    Build,
}

impl MenuMode {
    fn title_suffix(self) -> &'static str {
        match self {
            MenuMode::Inventory => "Inventory",
            MenuMode::Build => "Build",
        }
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
    Build {
        selection: BuildMenuSelection,
        amount: u32,
    },
}

#[derive(Clone, Debug)]
pub struct ObjectMenuSelection {
    pub crew_id: ObjectId,
    pub primary_id: ObjectId,
    pub instances: Vec<ObjectId>,
    pub definition_id: String,
    pub label: String,
}

impl ObjectMenuSelection {
    pub fn count(&self) -> usize {
        self.instances.len()
    }
}

#[derive(Clone, Debug)]
pub struct BuildMenuSelection {
    pub crew_id: ObjectId,
    pub owner: i32,
    pub definition_id: String,
    pub label: String,
}

#[derive(Clone, Debug)]
pub struct ObjectMenuState {
    crew_id: ObjectId,
    crew_label: String,
    owner: i32,
    mode: MenuMode,
    inventory: Vec<ObjectMenuItem>,
    build: Vec<BuildMenuItem>,
    inventory_selected: Option<usize>,
    build_selected: Option<usize>,
    inventory_known_empty: bool,
}

impl ObjectMenuState {
    pub fn for_player(owner: i32, engine: &Engine, snapshot: &SimulationSnapshot) -> Option<Self> {
        let cursor = engine.crew_cursor(owner)?;
        Self::new(engine, snapshot, cursor)
    }

    pub fn new(engine: &Engine, snapshot: &SimulationSnapshot, crew_id: ObjectId) -> Option<Self> {
        let crew = snapshot.object(crew_id)?.clone();
        let owner = crew.owner;
        let crew_label = engine
            .definition_name(&crew.definition_id)
            .unwrap_or(&crew.definition_id)
            .to_string();
        let inventory = collect_inventory(engine, snapshot, &crew);
        let inventory_known_empty = inventory.is_empty();
        let build = collect_build_items(engine, &crew);
        let inventory_selected = if inventory.is_empty() { None } else { Some(0) };
        let build_selected = if build.is_empty() { None } else { Some(0) };
        let mut mode = MenuMode::Inventory;
        if inventory.is_empty() && !build.is_empty() {
            mode = MenuMode::Build;
        }
        Some(Self {
            crew_id,
            crew_label,
            owner,
            mode,
            inventory,
            build,
            inventory_selected,
            build_selected,
            inventory_known_empty,
        })
    }

    pub fn refresh(&mut self, engine: &Engine, snapshot: &SimulationSnapshot) -> bool {
        let crew = match snapshot.object(self.crew_id) {
            Some(crew) => crew,
            None => return false,
        };
        self.owner = crew.owner;
        self.crew_label = engine
            .definition_name(&crew.definition_id)
            .unwrap_or(&crew.definition_id)
            .to_string();
        self.inventory = collect_inventory(engine, snapshot, crew);
        self.build = collect_build_items(engine, crew);
        self.inventory_known_empty = self.inventory.is_empty();
        clamp_selection(&mut self.inventory_selected, self.inventory.len());
        clamp_selection(&mut self.build_selected, self.build.len());
        if self.inventory_selected.is_none() && !self.inventory.is_empty() {
            self.inventory_selected = Some(0);
        }
        if self.build_selected.is_none() && !self.build.is_empty() {
            self.build_selected = Some(0);
        }
        self.ensure_valid_mode();
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
            ControlCommand::MenuUp => {
                self.advance_selection(-1);
                None
            }
            ControlCommand::MenuDown => {
                self.advance_selection(1);
                None
            }
            ControlCommand::MenuLeft => {
                if self.switch_mode(MenuMode::Inventory) {
                    return None;
                }
                self.advance_selection(-1);
                None
            }
            ControlCommand::MenuRight => {
                if self.switch_mode(MenuMode::Build) {
                    return None;
                }
                self.advance_selection(1);
                None
            }
            ControlCommand::MenuSelect | ControlCommand::MenuEnter => match self.mode {
                MenuMode::Inventory => self.activation_action(ObjectMenuCommand::Focus),
                MenuMode::Build => self.build_action(1),
            },
            ControlCommand::MenuEnterAll => match self.mode {
                MenuMode::Inventory => self.activation_action(ObjectMenuCommand::DropAll),
                MenuMode::Build => {
                    let amount = self
                        .build_selected
                        .and_then(|index| self.build.get(index))
                        .map(|item| item.available())
                        .unwrap_or(0);
                    self.build_action(amount)
                }
            },
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

        let title = format!("{} {}", self.crew_label, self.mode.title_suffix());
        let has_inventory = !self.inventory.is_empty();
        let has_build = !self.build.is_empty();
        let hint = if has_inventory && has_build {
            Some(MODE_HINT)
        } else {
            None
        };

        match self.mode {
            MenuMode::Inventory => {
                let empty_message = if self.inventory_known_empty {
                    "Inventory is empty."
                } else {
                    "Inventory unavailable."
                };
                self.render_entries(
                    surface,
                    font,
                    &self.inventory,
                    self.inventory_selected,
                    &title,
                    empty_message,
                    hint,
                );
            }
            MenuMode::Build => {
                self.render_entries(
                    surface,
                    font,
                    &self.build,
                    self.build_selected,
                    &title,
                    "No home base supplies available.",
                    hint,
                );
            }
        }
    }

    fn render_entries<E: MenuEntry>(
        &self,
        surface: &mut Surface,
        font: &dyn TextFont,
        items: &[E],
        selected: Option<usize>,
        title: &str,
        empty_message: &str,
        hint: Option<&str>,
    ) {
        let width = surface.width() as i32;
        let height = surface.height() as i32;

        let mut panel_width = (width as f32 * 0.42).round() as i32;
        panel_width = panel_width.clamp(PANEL_WIDTH_MIN, PANEL_WIDTH_MAX);
        panel_width = panel_width
            .min(width - PANEL_PADDING * 2)
            .max(PANEL_WIDTH_MIN);

        let list_height = if items.is_empty() {
            ITEM_HEIGHT
        } else {
            (items.len() as i32).saturating_mul(ITEM_HEIGHT + ITEM_SPACING) - ITEM_SPACING
        };
        let mut panel_height = (PANEL_PADDING * 2) + TITLE_GAP + list_height.max(ITEM_HEIGHT);
        if hint.is_some() {
            panel_height += ITEM_SPACING + DETAIL_FONT_SIZE as i32 + 6;
        }

        let panel_x = (width - panel_width) / 2;
        let panel_y = (height - panel_height) / 2;

        let panel_rect = Rect::new(panel_x, panel_y, panel_width as u32, panel_height as u32);
        fill_rect(surface, panel_rect, PANEL_COLOR);
        draw_border(surface, panel_rect, PANEL_BORDER);

        let mut cursor_y = panel_y + PANEL_PADDING;
        font.draw_text(
            surface,
            (panel_x + PANEL_PADDING) as f32,
            cursor_y as f32,
            title,
            TITLE_FONT_SIZE,
            TITLE_COLOR,
        );

        cursor_y += TITLE_GAP;
        if items.is_empty() {
            font.draw_text(
                surface,
                (panel_x + PANEL_PADDING) as f32,
                (cursor_y + 10) as f32,
                empty_message,
                ITEM_FONT_SIZE,
                MUTED_TEXT_COLOR,
            );
            return;
        }

        for (index, item) in items.iter().enumerate() {
            let row_rect = Rect::new(
                panel_x + PANEL_PADDING,
                cursor_y,
                (panel_width - PANEL_PADDING * 2) as u32,
                ITEM_HEIGHT as u32,
            );
            if Some(index) == selected {
                fill_rect(surface, row_rect, HIGHLIGHT_COLOR);
            }

            let primary_color = if Some(index) == selected {
                EMPHASIS_TEXT_COLOR
            } else {
                TEXT_COLOR
            };

            let count = item.count();
            let label_text = if count > 1 {
                format!("{} (x{})", item.label(), count)
            } else {
                item.label().to_string()
            };
            font.draw_text(
                surface,
                (row_rect.x + 12) as f32,
                (row_rect.y + 8) as f32,
                &label_text,
                ITEM_FONT_SIZE,
                primary_color,
            );

            if let Some(description) = item.description() {
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

        if let Some(hint) = hint {
            font.draw_text(
                surface,
                (panel_x + PANEL_PADDING) as f32,
                (panel_y + panel_height - PANEL_PADDING - 18) as f32,
                hint,
                DETAIL_FONT_SIZE,
                MUTED_TEXT_COLOR,
            );
        }
    }

    fn advance_selection(&mut self, delta: i32) {
        let (selected, len) = self.current_selection_mut();
        if len == 0 {
            return;
        }
        let len = len as i32;
        let next = match selected {
            Some(index) => (*index as i32 + delta).rem_euclid(len),
            None => {
                if delta >= 0 {
                    0
                } else {
                    len - 1
                }
            }
        };
        *selected = Some(next as usize);
    }

    fn activation_action(&self, command: ObjectMenuCommand) -> Option<ObjectMenuAction> {
        let index = self.inventory_selected?;
        let item = self.inventory.get(index)?;
        let primary_id = item.primary_object()?;
        Some(ObjectMenuAction::Execute {
            command,
            selection: ObjectMenuSelection {
                crew_id: self.crew_id,
                primary_id,
                instances: item.instances.clone(),
                definition_id: item.definition_id.clone(),
                label: item.label.clone(),
            },
        })
    }

    fn build_action(&self, amount: u32) -> Option<ObjectMenuAction> {
        if amount == 0 {
            return None;
        }
        let index = self.build_selected?;
        let item = self.build.get(index)?;
        let available = item.available();
        if available == 0 {
            return None;
        }
        Some(ObjectMenuAction::Build {
            selection: BuildMenuSelection {
                crew_id: self.crew_id,
                owner: self.owner,
                definition_id: item.definition_id.clone(),
                label: item.label.clone(),
            },
            amount: amount.min(available),
        })
    }

    fn current_selection_mut(&mut self) -> (&mut Option<usize>, usize) {
        match self.mode {
            MenuMode::Inventory => (&mut self.inventory_selected, self.inventory.len()),
            MenuMode::Build => (&mut self.build_selected, self.build.len()),
        }
    }

    fn ensure_valid_mode(&mut self) {
        match self.mode {
            MenuMode::Inventory if self.inventory.is_empty() && !self.build.is_empty() => {
                self.mode = MenuMode::Build;
            }
            MenuMode::Build if self.build.is_empty() && !self.inventory.is_empty() => {
                self.mode = MenuMode::Inventory;
            }
            _ => {}
        }
    }

    fn switch_mode(&mut self, mode: MenuMode) -> bool {
        if self.mode == mode {
            return false;
        }
        match mode {
            MenuMode::Inventory if self.inventory.is_empty() => return false,
            MenuMode::Build if self.build.is_empty() => return false,
            _ => {}
        }
        self.mode = mode;
        match self.mode {
            MenuMode::Inventory => {
                if self.inventory_selected.is_none() && !self.inventory.is_empty() {
                    self.inventory_selected = Some(0);
                }
            }
            MenuMode::Build => {
                if self.build_selected.is_none() && !self.build.is_empty() {
                    self.build_selected = Some(0);
                }
            }
        }
        true
    }
}

fn clamp_selection(selection: &mut Option<usize>, len: usize) {
    if len == 0 {
        *selection = None;
    } else if let Some(index) = selection {
        if *index >= len {
            *selection = Some(len - 1);
        }
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
        let description = build_definition_summary(engine, &child.definition_id);
        if let Some(index) = lookup.get(&child.definition_id).copied() {
            if let Some(entry) = order.get_mut(index) {
                entry.push_instance(child.id);
            }
        } else {
            let index = order.len();
            let entry = ObjectMenuItem::new(name, &child.definition_id, description, child.id);
            order.push(entry);
            lookup.insert(child.definition_id.clone(), index);
        }
    }
    order
}

fn collect_build_items(engine: &Engine, crew: &lc_engine::ObjectSnapshot) -> Vec<BuildMenuItem> {
    if crew.owner == OWNER_NONE {
        return Vec::new();
    }
    let Some(player) = engine.player(crew.owner) else {
        return Vec::new();
    };
    let mut entries = Vec::new();
    for (definition_id, count) in player.home_base_material() {
        if *count == 0 {
            continue;
        }
        let label = engine
            .definition_name(definition_id)
            .unwrap_or(definition_id)
            .to_string();
        let description = build_definition_summary(engine, definition_id);
        entries.push(BuildMenuItem {
            definition_id: definition_id.clone(),
            label,
            description,
            available: *count,
        });
    }
    entries.sort_by(|a, b| a.label.cmp(&b.label));
    entries
}

fn build_definition_summary(engine: &Engine, definition_id: &str) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(value) = engine.definition_value(definition_id) {
        if value > 0 {
            parts.push(format!("Value {value}"));
        }
    }
    if let Some(mass) = engine.definition_mass(definition_id) {
        if mass > 0 {
            parts.push(format!("Mass {mass}"));
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" • "))
    }
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
    use lc_engine::{
        Definition, Engine, MovementProfile, ObjectSnapshot, ObjectStatus, PlayerConfig, Vector2,
    };
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
        players: Vec<lc_engine::PlayerState>,
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
            players,
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
        let snapshot = make_snapshot(crew.clone(), contents, Vec::new());
        let mut menu =
            ObjectMenuState::new(&engine, &snapshot, crew.id).expect("menu should exist");
        assert_eq!(menu.inventory.len(), 2);
        assert_eq!(menu.inventory[0].definition_id, "Shovel");
        assert_eq!(menu.inventory[0].count(), 2);
        assert_eq!(menu.inventory[1].definition_id, "Hammer");
        assert_eq!(menu.inventory[1].count(), 1);

        // Simulate removing an item and refreshing.
        let mut snapshot_updated = snapshot.clone();
        if let Some(crew_obj) = snapshot_updated.objects.get_mut(0) {
            crew_obj.contents.pop();
        }
        assert!(menu.refresh(&engine, &snapshot_updated));
        assert_eq!(menu.inventory.len(), 1);
        assert_eq!(menu.inventory[0].count(), 2);
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
        let snapshot = make_snapshot(crew.clone(), contents, Vec::new());
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
                assert_eq!(selection.definition_id, "Shovel");
                assert_eq!(selection.count(), 2);
                assert_eq!(selection.instances.len(), 2);
                assert_eq!(selection.primary_id, ObjectId::new(2));
            }
            _ => panic!("expected execute action"),
        }
    }

    #[test]
    fn inventory_item_uses_definition_metadata() {
        let mut engine = Engine::new();
        let mut shovel =
            Definition::from_script("Shovel", "Shovel", "func Initialize() {}").unwrap();
        shovel.set_movement_profile(MovementProfile::default());
        shovel.set_value(75);
        shovel.set_mass(18);
        engine
            .register_definition(shovel)
            .expect("register shovel definition");

        let crew = make_object(1, "Clonk");
        let contents = vec![make_object(2, "Shovel")];
        let snapshot = make_snapshot(crew.clone(), contents, Vec::new());
        let menu = ObjectMenuState::new(&engine, &snapshot, crew.id).expect("menu should exist");
        assert_eq!(menu.inventory.len(), 1);
        assert_eq!(
            menu.inventory[0].description.as_deref(),
            Some("Value 75 • Mass 18")
        );
    }

    #[test]
    fn build_menu_lists_home_base_supplies() {
        let mut engine = Engine::new();
        engine
            .register_player(
                PlayerConfig::new(1, "Player")
                    .with_home_base_material(HashMap::from([("Hammer".to_string(), 3_u32)])),
            )
            .expect("register player");
        let mut hammer =
            Definition::from_script("Hammer", "Hammer", "func Initialize() {}").unwrap();
        hammer.set_movement_profile(MovementProfile::default());
        engine.register_definition(hammer).expect("register hammer");

        let mut crew = make_object(1, "Clonk");
        crew.owner = 1;
        let snapshot = make_snapshot(crew.clone(), Vec::new(), Vec::new());
        let mut menu =
            ObjectMenuState::new(&engine, &snapshot, crew.id).expect("menu should exist");
        assert_eq!(menu.build.len(), 1);
        assert_eq!(menu.build[0].definition_id, "Hammer");
        assert_eq!(menu.build[0].available(), 3);

        // Switch to build mode.
        assert!(menu
            .handle_command(ControlCommand::MenuRight, CommandKind::Press)
            .is_none());
        assert_eq!(menu.mode, MenuMode::Build);
        let action = menu
            .handle_command(ControlCommand::MenuSelect, CommandKind::Press)
            .expect("build action");
        match action {
            ObjectMenuAction::Build { selection, amount } => {
                assert_eq!(selection.definition_id, "Hammer");
                assert_eq!(selection.owner, 1);
                assert_eq!(selection.crew_id, crew.id);
                assert_eq!(amount, 1);
            }
            other => panic!("unexpected action: {:?}", other),
        }
    }
}
