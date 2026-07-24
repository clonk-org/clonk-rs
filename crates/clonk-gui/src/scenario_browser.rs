use crate::{
    ButtonTextures, DrawCommand, Gui, GuiAction, GuiEvent, GuiEventResult, GuiResult, ImageData,
    KeyCode, Rect, Size, WidgetId,
};
use clonk_graphics::TextFont;
use std::{fmt, sync::Arc};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScenarioKind {
    Scenario,
    Folder,
    Editor,
}

impl ScenarioKind {
    fn display_name(self) -> &'static str {
        match self {
            ScenarioKind::Scenario => "Scenario",
            ScenarioKind::Folder => "Folder",
            ScenarioKind::Editor => "Editor",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ScenarioEntry {
    pub identifier: String,
    pub title: String,
    pub description: Option<String>,
    pub kind: ScenarioKind,
    pub is_editable: bool,
    pub is_playable: bool,
    pub location: Option<String>,
    pub preview: Option<ImageData>,
}

impl ScenarioEntry {
    pub fn summary(&self) -> ScenarioEntrySummary {
        ScenarioEntrySummary {
            identifier: self.identifier.clone(),
            title: self.title.clone(),
            kind: self.kind,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioEntrySummary {
    pub identifier: String,
    pub title: String,
    pub kind: ScenarioKind,
}

impl fmt::Display for ScenarioEntrySummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.title, self.kind.display_name())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScenarioBrowserMessage {
    SelectionChanged(ScenarioEntrySummary),
    StartScenario(ScenarioEntrySummary),
    OpenEntry(ScenarioEntrySummary),
    EditEntry(ScenarioEntrySummary),
}

#[derive(Debug, Default)]
pub struct ScenarioBrowserResponse {
    pub gui: GuiEventResult,
    pub messages: Vec<ScenarioBrowserMessage>,
}

pub struct ScenarioBrowser {
    gui: Gui,
    entries: Vec<ScenarioEntry>,
    layout: ScenarioBrowserLayout,
    selected: Option<usize>,
    font: Arc<dyn TextFont>,
    button_textures: Option<ButtonTextures>,
}

impl ScenarioBrowser {
    pub fn new(entries: Vec<ScenarioEntry>, font: Arc<dyn TextFont>) -> GuiResult<Self> {
        let mut gui = Gui::new(font.clone());
        let layout = ScenarioBrowserLayout::build(&mut gui, &entries);
        let mut browser = Self {
            gui,
            entries,
            layout,
            selected: None,
            font,
            button_textures: None,
        };
        browser.clear_selection_ui()?;
        Ok(browser)
    }

    pub fn entries(&self) -> &[ScenarioEntry] {
        &self.entries
    }

    pub fn set_entries(&mut self, entries: Vec<ScenarioEntry>) -> GuiResult<()> {
        self.gui = Gui::new(self.font.clone());
        if let Some(textures) = self.button_textures.clone() {
            self.gui.set_button_textures(Some(textures));
        }
        self.layout = ScenarioBrowserLayout::build(&mut self.gui, &entries);
        self.entries = entries;
        self.selected = None;
        self.clear_selection_ui()
    }

    pub fn select_entry_by_index(
        &mut self,
        index: usize,
    ) -> GuiResult<Option<ScenarioBrowserMessage>> {
        self.select_entry(index)
    }

    pub fn set_button_textures(&mut self, textures: Option<ButtonTextures>) {
        let gui_textures = textures.clone();
        self.button_textures = textures;
        self.gui.set_button_textures(gui_textures);
    }

    pub fn layout(&mut self, available: Size) -> Size {
        self.gui.layout(available)
    }

    pub fn render(&self) -> Vec<DrawCommand> {
        self.gui.render()
    }

    pub fn widget_rect(&self, id: WidgetId) -> Option<Rect> {
        self.gui.rect_of(id)
    }

    pub fn start_button_id(&self) -> WidgetId {
        self.layout.action_buttons.start
    }

    pub fn open_button_id(&self) -> WidgetId {
        self.layout.action_buttons.open
    }

    pub fn edit_button_id(&self) -> WidgetId {
        self.layout.action_buttons.edit
    }

    pub fn entry_button(&self, identifier: &str) -> Option<WidgetId> {
        self.layout
            .entry_widgets
            .iter()
            .find(|entry| entry.identifier == identifier)
            .map(|entry| entry.button)
    }

    pub fn handle_event(&mut self, event: GuiEvent) -> ScenarioBrowserResponse {
        match event {
            GuiEvent::KeyDown { key } => {
                let gui_result = self.gui.handle_event(GuiEvent::KeyDown { key });
                let mut response = self.process_gui_result(gui_result);
                let (captured, mut messages) = self.handle_key_down_event(key);
                response.gui.captured |= captured;
                response.messages.append(&mut messages);
                response
            }
            GuiEvent::KeyUp { key } => {
                let gui_result = self.gui.handle_event(GuiEvent::KeyUp { key });
                self.process_gui_result(gui_result)
            }
            other => {
                let gui_result = self.gui.handle_event(other);
                self.process_gui_result(gui_result)
            }
        }
    }

    pub fn cancel_interaction(&mut self) {
        self.gui.cancel_interaction();
    }

    pub fn selected_entry(&self) -> Option<&ScenarioEntry> {
        self.selected.and_then(|index| self.entries.get(index))
    }

    pub fn selected_index(&self) -> Option<usize> {
        self.selected
    }

    fn select_entry(&mut self, index: usize) -> GuiResult<Option<ScenarioBrowserMessage>> {
        if index >= self.entries.len() {
            return Ok(None);
        }
        if self.selected == Some(index) {
            return Ok(None);
        }
        if let Some(previous) = self.selected {
            if let Some(prev_widget) = self.layout.entry_widgets.get(previous) {
                self.gui.set_button_selected(prev_widget.button, false)?;
            }
        }
        if let Some(new_widget) = self.layout.entry_widgets.get(index) {
            self.gui.set_button_selected(new_widget.button, true)?;
        }
        self.selected = Some(index);
        self.update_info_panel()?;
        self.update_action_buttons()?;
        Ok(self
            .selected_entry()
            .map(|entry| ScenarioBrowserMessage::SelectionChanged(entry.summary())))
    }

    fn clear_selection_ui(&mut self) -> GuiResult<()> {
        for entry in &self.layout.entry_widgets {
            self.gui.set_button_selected(entry.button, false)?;
        }
        self.selected = None;
        self.gui
            .set_picture_image(self.layout.info_panel.preview, None)?;
        self.gui
            .set_label_text(self.layout.info_panel.title_label, "No scenario selected")?;
        self.gui
            .set_label_text(self.layout.info_panel.kind_label, "Kind: -")?;
        self.gui.set_label_text(
            self.layout.info_panel.availability_label,
            "Playable: No    Editable: No",
        )?;
        self.gui
            .set_label_text(self.layout.info_panel.location_label, "Location: -")?;
        self.gui.set_label_text(
            self.layout.info_panel.description_label,
            "Select a scenario to view its details.",
        )?;
        self.gui
            .set_button_enabled(self.layout.action_buttons.start, false)?;
        self.gui
            .set_button_enabled(self.layout.action_buttons.open, false)?;
        self.gui
            .set_button_enabled(self.layout.action_buttons.edit, false)?;
        Ok(())
    }

    fn update_info_panel(&mut self) -> GuiResult<()> {
        if let Some(index) = self.selected {
            if let Some(entry) = self.entries.get(index) {
                let title = entry.title.clone();
                let description = entry
                    .description
                    .clone()
                    .unwrap_or_else(|| "No description provided.".into());
                let kind_label = format!("Kind: {}", entry.kind.display_name());
                self.gui
                    .set_label_text(self.layout.info_panel.title_label, title)?;
                self.gui
                    .set_label_text(self.layout.info_panel.kind_label, kind_label)?;
                let availability = format!(
                    "Playable: {}    Editable: {}",
                    if entry.is_playable { "Yes" } else { "No" },
                    if entry.is_editable { "Yes" } else { "No" }
                );
                self.gui
                    .set_label_text(self.layout.info_panel.availability_label, availability)?;
                let location = entry
                    .location
                    .as_deref()
                    .filter(|value| !value.is_empty())
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_string());
                self.gui.set_label_text(
                    self.layout.info_panel.location_label,
                    format!("Location: {}", location),
                )?;
                self.gui
                    .set_label_text(self.layout.info_panel.description_label, description)?;
                self.gui
                    .set_picture_image(self.layout.info_panel.preview, entry.preview.clone())?;
                return Ok(());
            }
        }
        self.clear_selection_ui()
    }

    fn update_action_buttons(&mut self) -> GuiResult<()> {
        if let Some(index) = self.selected {
            if let Some(entry) = self.entries.get(index) {
                let is_playable = entry.is_playable;
                let is_editable = entry.is_editable;
                self.gui
                    .set_button_enabled(self.layout.action_buttons.start, is_playable)?;
                self.gui
                    .set_button_enabled(self.layout.action_buttons.open, true)?;
                self.gui
                    .set_button_enabled(self.layout.action_buttons.edit, is_editable)?;
                return Ok(());
            }
        }
        self.gui
            .set_button_enabled(self.layout.action_buttons.start, false)?;
        self.gui
            .set_button_enabled(self.layout.action_buttons.open, false)?;
        self.gui
            .set_button_enabled(self.layout.action_buttons.edit, false)?;
        Ok(())
    }

    fn process_gui_result(&mut self, gui_result: GuiEventResult) -> ScenarioBrowserResponse {
        let mut messages = Vec::new();

        for (widget, action) in gui_result.actions.iter().copied() {
            if action != GuiAction::Activate {
                continue;
            }
            if let Some(index) = self.layout.index_of(widget) {
                if let Ok(Some(message)) = self.select_entry(index) {
                    messages.push(message);
                }
                continue;
            }
            if widget == self.layout.action_buttons.start {
                if let Some(entry) = self.selected_entry() {
                    if entry.is_playable {
                        messages.push(ScenarioBrowserMessage::StartScenario(entry.summary()));
                    }
                }
                continue;
            }
            if widget == self.layout.action_buttons.open {
                if let Some(entry) = self.selected_entry() {
                    messages.push(ScenarioBrowserMessage::OpenEntry(entry.summary()));
                }
                continue;
            }
            if widget == self.layout.action_buttons.edit {
                if let Some(entry) = self.selected_entry() {
                    if entry.is_editable {
                        messages.push(ScenarioBrowserMessage::EditEntry(entry.summary()));
                    }
                }
                continue;
            }
        }

        ScenarioBrowserResponse {
            gui: gui_result,
            messages,
        }
    }

    fn handle_key_down_event(&mut self, key: KeyCode) -> (bool, Vec<ScenarioBrowserMessage>) {
        let mut captured = false;
        let mut messages = Vec::new();

        match key {
            KeyCode::Up | KeyCode::Left => {
                if self.entries.is_empty() {
                    return (false, messages);
                }
                captured = true;
                if let Ok(Some(message)) = self.move_selection(-1) {
                    messages.push(message);
                }
            }
            KeyCode::Down | KeyCode::Right | KeyCode::Tab => {
                if self.entries.is_empty() {
                    return (false, messages);
                }
                captured = true;
                if let Ok(Some(message)) = self.move_selection(1) {
                    messages.push(message);
                }
            }
            KeyCode::Enter | KeyCode::Space => {
                if let Some(entry) = self.selected_entry() {
                    captured = true;
                    if entry.is_playable {
                        messages.push(ScenarioBrowserMessage::StartScenario(entry.summary()));
                    } else {
                        messages.push(ScenarioBrowserMessage::OpenEntry(entry.summary()));
                    }
                }
            }
            KeyCode::Escape if self.selected.is_some() => {
                captured = true;
                let _ = self.clear_selection_ui();
            }
            _ => {}
        }

        (captured, messages)
    }

    fn move_selection(&mut self, delta: isize) -> GuiResult<Option<ScenarioBrowserMessage>> {
        if self.entries.is_empty() {
            return Ok(None);
        }
        let len = self.entries.len() as isize;
        let new_index = match self.selected {
            Some(current) => ((current as isize + delta).rem_euclid(len)) as usize,
            None => {
                if delta.is_negative() {
                    (len - 1) as usize
                } else {
                    0
                }
            }
        };
        self.select_entry(new_index)
    }
}

#[derive(Debug, Clone)]
struct EntryWidgets {
    button: WidgetId,
    identifier: String,
}

#[derive(Debug)]
struct ScenarioBrowserLayout {
    entry_widgets: Vec<EntryWidgets>,
    info_panel: InfoPanel,
    action_buttons: ActionButtons,
}

impl ScenarioBrowserLayout {
    fn build(gui: &mut Gui, entries: &[ScenarioEntry]) -> Self {
        let root = gui.root();
        let _title = gui.add_label(root, "Scenario Browser");
        let _list_label = gui.add_label(root, "Available Scenarios");
        let entry_column = gui.add_column(root, true);

        let entry_widgets = entries
            .iter()
            .map(|entry| EntryWidgets {
                button: gui.add_button(entry_column, entry.title.clone()),
                identifier: entry.identifier.clone(),
            })
            .collect::<Vec<_>>();

        let _details_label = gui.add_label(root, "Details");
        let info_column = gui.add_column(root, true);
        let preview = gui.add_picture(info_column, 320.0, 200.0);
        let title_label = gui.add_label(info_column, "No scenario selected");
        let kind_label = gui.add_label(info_column, "Kind: -");
        let availability_label = gui.add_label(info_column, "Playable: No    Editable: No");
        let location_label = gui.add_label(info_column, "Location: -");
        let description_label =
            gui.add_label(info_column, "Select a scenario to view its details.");

        let action_row = gui.add_row(root, false);
        let start = gui.add_button(action_row, "Start");
        let open = gui.add_button(action_row, "Open");
        let edit = gui.add_button(action_row, "Edit");

        Self {
            entry_widgets,
            info_panel: InfoPanel {
                preview,
                title_label,
                kind_label,
                availability_label,
                location_label,
                description_label,
            },
            action_buttons: ActionButtons { start, open, edit },
        }
    }

    fn index_of(&self, widget: WidgetId) -> Option<usize> {
        self.entry_widgets
            .iter()
            .position(|entry| entry.button == widget)
    }
}

#[derive(Debug)]
struct InfoPanel {
    preview: WidgetId,
    title_label: WidgetId,
    kind_label: WidgetId,
    availability_label: WidgetId,
    location_label: WidgetId,
    description_label: WidgetId,
}

#[derive(Debug)]
struct ActionButtons {
    start: WidgetId,
    open: WidgetId,
    edit: WidgetId,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GuiEvent, KeyCode, Point, Size};
    use clonk_graphics::BitmapFont;
    use std::sync::Arc;

    fn center(rect: Rect) -> Point {
        Point::new(
            rect.origin.x + rect.size.width * 0.5,
            rect.origin.y + rect.size.height * 0.5,
        )
    }

    fn test_font() -> Arc<dyn TextFont> {
        Arc::new(BitmapFont::new())
    }

    #[test]
    fn selecting_entry_updates_selection_and_enables_actions() {
        let entries = vec![
            ScenarioEntry {
                identifier: "scenario_1".into(),
                title: "Tutorial".into(),
                description: Some("Learn the basics".into()),
                kind: ScenarioKind::Scenario,
                is_editable: true,
                is_playable: true,
                location: Some("/scenarios/tutorial".into()),
                preview: None,
            },
            ScenarioEntry {
                identifier: "folder_1".into(),
                title: "Missions".into(),
                description: None,
                kind: ScenarioKind::Folder,
                is_editable: false,
                is_playable: false,
                location: Some("/scenarios/missions".into()),
                preview: None,
            },
        ];

        let mut browser = ScenarioBrowser::new(entries, test_font()).expect("browser");
        browser.layout(Size::new(480.0, 720.0));

        let button_id = browser.entry_button("scenario_1").expect("button");
        let rect = browser.widget_rect(button_id).expect("rect");
        let pos = center(rect);

        let down = browser.handle_event(GuiEvent::PointerDown { position: pos });
        assert!(down.gui.captured);
        assert!(down.messages.is_empty());

        let up = browser.handle_event(GuiEvent::PointerUp { position: pos });
        assert!(up.gui.captured);
        assert_eq!(up.messages.len(), 1);
        match &up.messages[0] {
            ScenarioBrowserMessage::SelectionChanged(summary) => {
                assert_eq!(summary.identifier, "scenario_1");
                assert_eq!(summary.kind, ScenarioKind::Scenario);
            }
            message => panic!("unexpected message: {:?}", message),
        }

        let selected = browser.selected_entry().expect("selection");
        assert_eq!(selected.title, "Tutorial");

        // Start button should now be enabled and react to clicks.
        let start_rect = browser
            .widget_rect(browser.start_button_id())
            .expect("start rect");
        let start_pos = center(start_rect);
        let start_down = browser.handle_event(GuiEvent::PointerDown {
            position: start_pos,
        });
        assert!(start_down.gui.captured);
        let start_up = browser.handle_event(GuiEvent::PointerUp {
            position: start_pos,
        });
        assert!(start_up.gui.captured);
        assert!(start_up
            .messages
            .iter()
            .any(|msg| matches!(msg, ScenarioBrowserMessage::StartScenario(summary) if summary.identifier == "scenario_1")));
    }

    #[test]
    fn keyboard_navigation_supports_selection_and_activation() {
        let entries = vec![
            ScenarioEntry {
                identifier: "scenario_1".into(),
                title: "Tutorial".into(),
                description: Some("Learn the basics".into()),
                kind: ScenarioKind::Scenario,
                is_editable: false,
                is_playable: true,
                location: Some("/scenarios/tutorial".into()),
                preview: None,
            },
            ScenarioEntry {
                identifier: "folder_1".into(),
                title: "Campaign".into(),
                description: None,
                kind: ScenarioKind::Folder,
                is_editable: true,
                is_playable: false,
                location: Some("/scenarios/campaign".into()),
                preview: None,
            },
        ];

        let mut browser = ScenarioBrowser::new(entries, test_font()).expect("browser");
        browser.layout(Size::new(480.0, 720.0));

        let first = browser.handle_event(GuiEvent::KeyDown { key: KeyCode::Down });
        assert!(first.gui.captured);
        assert_eq!(first.messages.len(), 1);
        assert!(matches!(
            &first.messages[0],
            ScenarioBrowserMessage::SelectionChanged(summary)
                if summary.identifier == "scenario_1"
        ));

        let start = browser.handle_event(GuiEvent::KeyDown {
            key: KeyCode::Enter,
        });
        assert!(start.gui.captured);
        assert!(matches!(
            start.messages.as_slice(),
            [ScenarioBrowserMessage::StartScenario(summary)]
                if summary.identifier == "scenario_1"
        ));

        let second = browser.handle_event(GuiEvent::KeyDown { key: KeyCode::Down });
        assert!(second.gui.captured);
        assert!(matches!(
            &second.messages[0],
            ScenarioBrowserMessage::SelectionChanged(summary)
                if summary.identifier == "folder_1"
        ));

        let open = browser.handle_event(GuiEvent::KeyDown {
            key: KeyCode::Enter,
        });
        assert!(open.gui.captured);
        assert!(matches!(
            open.messages.as_slice(),
            [ScenarioBrowserMessage::OpenEntry(summary)]
                if summary.identifier == "folder_1"
        ));

        let escape = browser.handle_event(GuiEvent::KeyDown {
            key: KeyCode::Escape,
        });
        assert!(escape.gui.captured);
        assert!(escape.messages.is_empty());
        assert!(browser.selected_entry().is_none());
    }
}
