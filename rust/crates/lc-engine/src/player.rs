use crate::{DefinitionId, ObjectId, Vector2};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayerStatus {
    Inactive,
    Active,
    Eliminated,
    TeamSelection,
    Surrendered,
}

impl Default for PlayerStatus {
    fn default() -> Self {
        PlayerStatus::Inactive
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayerViewport {
    #[serde(default)]
    pub focus: Option<ObjectId>,
    pub center: Vector2,
    #[serde(default = "default_zoom")]
    pub zoom: f32,
}

impl PlayerViewport {
    pub fn new(center: Vector2) -> Self {
        Self {
            focus: None,
            center,
            zoom: default_zoom(),
        }
    }

    pub fn with_focus(mut self, focus: Option<ObjectId>) -> Self {
        self.focus = focus;
        self
    }

    pub fn with_zoom(mut self, zoom: f32) -> Self {
        self.zoom = zoom.max(0.0);
        self
    }
}

fn default_zoom() -> f32 {
    1.0
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PlayerState {
    pub id: i32,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub status: PlayerStatus,
    #[serde(default)]
    pub wealth: i32,
    #[serde(default)]
    pub knowledge: Vec<DefinitionId>,
    #[serde(default)]
    pub inventory: HashMap<DefinitionId, u32>,
    #[serde(default)]
    pub cursor: Option<ObjectId>,
    #[serde(default)]
    pub viewports: Vec<PlayerViewport>,
    #[serde(default)]
    pub crew: Vec<ObjectId>,
}

#[derive(Debug, Clone)]
pub struct Player {
    id: i32,
    name: String,
    status: PlayerStatus,
    wealth: i32,
    knowledge: HashSet<DefinitionId>,
    inventory: HashMap<DefinitionId, u32>,
    cursor: Option<ObjectId>,
    viewports: Vec<PlayerViewport>,
    crew: Vec<ObjectId>,
}

impl Player {
    pub fn new(id: i32, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            status: PlayerStatus::Active,
            wealth: 0,
            knowledge: HashSet::new(),
            inventory: HashMap::new(),
            cursor: None,
            viewports: Vec::new(),
            crew: Vec::new(),
        }
    }

    pub fn from_config(config: PlayerConfig) -> Self {
        let PlayerConfig {
            id,
            name,
            status,
            wealth,
            knowledge,
            inventory,
            cursor,
            viewports,
        } = config;
        let mut player = Self {
            id,
            name,
            status,
            wealth,
            knowledge: knowledge.into_iter().collect(),
            inventory,
            cursor,
            viewports,
            crew: Vec::new(),
        };
        player.sort_crew();
        player
    }

    pub fn from_state(state: PlayerState) -> Self {
        let PlayerState {
            id,
            name,
            status,
            wealth,
            knowledge,
            inventory,
            cursor,
            viewports,
            crew,
        } = state;
        let mut player = Self {
            id,
            name,
            status,
            wealth,
            knowledge: knowledge.into_iter().collect(),
            inventory,
            cursor,
            viewports,
            crew,
        };
        player.sort_crew();
        player
    }

    pub fn to_state(&self) -> PlayerState {
        let mut knowledge: Vec<_> = self.knowledge.iter().cloned().collect();
        knowledge.sort();
        PlayerState {
            id: self.id,
            name: self.name.clone(),
            status: self.status,
            wealth: self.wealth,
            knowledge,
            inventory: self.inventory.clone(),
            cursor: self.cursor,
            viewports: self.viewports.clone(),
            crew: self.crew.clone(),
        }
    }

    pub fn id(&self) -> i32 {
        self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn set_name(&mut self, name: impl Into<String>) {
        self.name = name.into();
    }

    pub fn status(&self) -> PlayerStatus {
        self.status
    }

    pub fn set_status(&mut self, status: PlayerStatus) {
        self.status = status;
    }

    pub fn wealth(&self) -> i32 {
        self.wealth
    }

    pub fn set_wealth(&mut self, wealth: i32) {
        self.wealth = wealth.max(0);
    }

    pub fn adjust_wealth(&mut self, delta: i32) -> i32 {
        self.wealth = if delta >= 0 {
            self.wealth.saturating_add(delta)
        } else {
            let decrease = delta.saturating_abs();
            if self.wealth >= decrease {
                self.wealth - decrease
            } else {
                0
            }
        };
        self.wealth
    }

    pub fn knowledge(&self) -> impl Iterator<Item = &DefinitionId> {
        self.knowledge.iter()
    }

    pub fn grant_knowledge(&mut self, definition_id: DefinitionId) {
        self.knowledge.insert(definition_id);
    }

    pub fn revoke_knowledge(&mut self, definition_id: &DefinitionId) {
        self.knowledge.remove(definition_id);
    }

    pub fn inventory(&self) -> &HashMap<DefinitionId, u32> {
        &self.inventory
    }

    pub fn set_inventory_item(&mut self, definition_id: DefinitionId, quantity: u32) {
        if quantity == 0 {
            self.inventory.remove(&definition_id);
        } else {
            self.inventory.insert(definition_id, quantity);
        }
    }

    pub fn adjust_inventory_item(&mut self, definition_id: DefinitionId, delta: i32) -> u32 {
        let current = self.inventory.get(&definition_id).copied().unwrap_or(0);
        let updated = if delta >= 0 {
            current.saturating_add(delta as u32)
        } else {
            let decrease = delta.checked_abs().unwrap_or(i32::MAX) as u32;
            current.saturating_sub(decrease)
        };
        if updated == 0 {
            self.inventory.remove(&definition_id);
        } else {
            self.inventory.insert(definition_id.clone(), updated);
        }
        updated
    }

    pub fn cursor(&self) -> Option<ObjectId> {
        self.cursor
    }

    pub fn set_cursor(&mut self, cursor: Option<ObjectId>) {
        self.cursor = cursor;
    }

    pub fn viewports(&self) -> &[PlayerViewport] {
        &self.viewports
    }

    pub fn replace_viewports(&mut self, viewports: Vec<PlayerViewport>) {
        self.viewports = viewports;
    }

    pub fn set_viewport(&mut self, index: usize, viewport: PlayerViewport) {
        if self.viewports.len() <= index {
            self.viewports
                .resize_with(index + 1, || PlayerViewport::new(Vector2::ZERO));
        }
        self.viewports[index] = viewport;
    }

    pub fn crew(&self) -> &[ObjectId] {
        &self.crew
    }

    pub fn set_crew(&mut self, crew: Vec<ObjectId>) {
        self.crew = crew;
        self.sort_crew();
    }

    fn sort_crew(&mut self) {
        self.crew.sort_unstable_by_key(|id| id.as_u64());
    }
}

#[derive(Debug, Clone)]
pub struct PlayerConfig {
    id: i32,
    name: String,
    status: PlayerStatus,
    wealth: i32,
    knowledge: Vec<DefinitionId>,
    inventory: HashMap<DefinitionId, u32>,
    cursor: Option<ObjectId>,
    viewports: Vec<PlayerViewport>,
}

impl PlayerConfig {
    pub fn new(id: i32, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            status: PlayerStatus::Active,
            wealth: 0,
            knowledge: Vec::new(),
            inventory: HashMap::new(),
            cursor: None,
            viewports: Vec::new(),
        }
    }

    pub fn with_status(mut self, status: PlayerStatus) -> Self {
        self.status = status;
        self
    }

    pub fn with_wealth(mut self, wealth: i32) -> Self {
        self.wealth = wealth;
        self
    }

    pub fn with_knowledge<I>(mut self, knowledge: I) -> Self
    where
        I: IntoIterator<Item = DefinitionId>,
    {
        self.knowledge = knowledge.into_iter().collect();
        self
    }

    pub fn with_inventory(mut self, inventory: HashMap<DefinitionId, u32>) -> Self {
        self.inventory = inventory;
        self
    }

    pub fn with_cursor(mut self, cursor: Option<ObjectId>) -> Self {
        self.cursor = cursor;
        self
    }

    pub fn with_viewports<I>(mut self, viewports: I) -> Self
    where
        I: IntoIterator<Item = PlayerViewport>,
    {
        self.viewports = viewports.into_iter().collect();
        self
    }

    pub fn id(&self) -> i32 {
        self.id
    }

    pub fn build(self) -> Player {
        Player::from_config(self)
    }
}
