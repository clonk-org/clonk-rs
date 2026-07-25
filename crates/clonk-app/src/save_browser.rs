use std::path::PathBuf;

use clonk_engine::{CommandKind, ControlCommand};



#[derive(Clone, Debug)]
pub struct SaveEntry {
    pub display_name: String,
    pub saved_at_seconds: u64,
    pub path: PathBuf,
}

#[derive(Clone, Debug)]
enum SaveMenuItem {
    NewSlot { label: String },
    Entry(SaveEntry),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveBrowserMode {
    Save { suggested_label: String },
    Load,
}

#[derive(Clone, Debug)]
pub enum SaveBrowserAction {
    Close,
    SaveNew { label: String },
    SaveExisting { entry: SaveEntry },
    Load { entry: SaveEntry },
}

pub struct SaveBrowserState {
    mode: SaveBrowserMode,
    items: Vec<SaveMenuItem>,
    selected: Option<usize>,
}

impl SaveBrowserState {
    pub fn new(mode: SaveBrowserMode, mut entries: Vec<SaveEntry>) -> Self {
        entries.sort_by(|a, b| {
            b.saved_at_seconds
                .cmp(&a.saved_at_seconds)
                .then_with(|| a.display_name.cmp(&b.display_name))
        });

        let mut items = Vec::new();
        let mut selected = None;
        match mode {
            SaveBrowserMode::Save {
                ref suggested_label,
            } => {
                items.push(SaveMenuItem::NewSlot {
                    label: suggested_label.clone(),
                });
                selected = Some(0);
            }
            SaveBrowserMode::Load => {}
        }
        let start_index = items.len();
        for entry in entries {
            items.push(SaveMenuItem::Entry(entry));
        }
        if selected.is_none() && !items.is_empty() {
            selected = Some(start_index);
        }
        Self {
            mode,
            items,
            selected,
        }
    }

    pub fn mode(&self) -> &SaveBrowserMode {
        &self.mode
    }

    pub fn handle_command(
        &mut self,
        command: ControlCommand,
        kind: CommandKind,
    ) -> Option<SaveBrowserAction> {
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
            ControlCommand::MenuSelect
            | ControlCommand::MenuEnter
            | ControlCommand::MenuEnterAll => self.activate(),
            ControlCommand::MenuClose => Some(SaveBrowserAction::Close),
            ControlCommand::MenuShowText => None,
            _ => None,
        }
    }


    fn advance_selection(&mut self, delta: i32) {
        let Some(current) = self.selected else {
            if !self.items.is_empty() {
                self.selected = Some(0);
            }
            return;
        };
        if self.items.is_empty() {
            self.selected = None;
            return;
        }
        let len = self.items.len() as i32;
        let mut next = current as i32;
        for _ in 0..len {
            next = (next + delta).rem_euclid(len);
            if self.items.get(next as usize).is_some() {
                self.selected = Some(next as usize);
                return;
            }
        }
        self.selected = Some(((current as i32 + delta).rem_euclid(len)) as usize);
    }

    fn activate(&self) -> Option<SaveBrowserAction> {
        let index = self.selected?;
        match (&self.mode, self.items.get(index)?) {
            (SaveBrowserMode::Save { .. }, SaveMenuItem::NewSlot { label }) => {
                Some(SaveBrowserAction::SaveNew {
                    label: label.clone(),
                })
            }
            (_, SaveMenuItem::Entry(entry)) => match self.mode {
                SaveBrowserMode::Save { .. } => Some(SaveBrowserAction::SaveExisting {
                    entry: entry.clone(),
                }),
                SaveBrowserMode::Load => Some(SaveBrowserAction::Load {
                    entry: entry.clone(),
                }),
            },
            _ => None,
        }
    }
}
