use crate::{
    draw_text, fill_rect, GuiPoint, KeyCode, ScenarioEntry, ScenarioKind, StartupMenuResult,
};
use lc_graphics::Surface;
use lc_gui::{
    DrawCommand, GuiEvent, ScenarioBrowser, ScenarioBrowserMessage, ScenarioEntrySummary,
    Size as GuiSize,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScenarioSummary {
    pub identifier: String,
    pub title: String,
    pub kind: ScenarioKind,
}

impl From<ScenarioEntrySummary> for ScenarioSummary {
    fn from(summary: ScenarioEntrySummary) -> Self {
        Self {
            identifier: summary.identifier,
            title: summary.title,
            kind: summary.kind,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StartupMenuAction {
    SelectionChanged(ScenarioSummary),
    StartScenario(ScenarioSummary),
    OpenEntry(ScenarioSummary),
    EditEntry(ScenarioSummary),
}

pub struct StartupMenu {
    browser: ScenarioBrowser,
    size: GuiSize,
}

impl StartupMenu {
    pub fn new(entries: Vec<ScenarioEntry>) -> StartupMenuResult<Self> {
        let browser = ScenarioBrowser::new(entries)?;
        Ok(Self {
            browser,
            size: GuiSize::new(0.0, 0.0),
        })
    }

    pub fn set_entries(&mut self, entries: Vec<ScenarioEntry>) -> StartupMenuResult<()> {
        self.browser.set_entries(entries)?;
        Ok(())
    }

    pub fn resize(&mut self, width: f32, height: f32) {
        self.size = GuiSize::new(width.max(1.0), height.max(1.0));
        self.browser.layout(self.size);
    }

    pub fn handle_pointer_down(&mut self, position: GuiPoint) -> Vec<StartupMenuAction> {
        self.dispatch(GuiEvent::PointerDown { position })
    }

    pub fn handle_pointer_up(&mut self, position: GuiPoint) -> Vec<StartupMenuAction> {
        self.dispatch(GuiEvent::PointerUp { position })
    }

    pub fn handle_pointer_move(&mut self, position: GuiPoint) -> Vec<StartupMenuAction> {
        self.dispatch(GuiEvent::PointerMove { position })
    }

    pub fn handle_key_down(&mut self, key: KeyCode) -> Vec<StartupMenuAction> {
        self.dispatch(GuiEvent::KeyDown { key })
    }

    pub fn handle_key_up(&mut self, key: KeyCode) -> Vec<StartupMenuAction> {
        self.dispatch(GuiEvent::KeyUp { key })
    }

    pub fn render(&mut self, surface: &mut Surface) {
        if self.size.width <= 0.0 || self.size.height <= 0.0 {
            self.resize(surface.width() as f32, surface.height() as f32);
        } else {
            self.browser.layout(self.size);
        }

        for command in self.browser.render() {
            match command {
                DrawCommand::Quad { rect, color } => fill_rect(surface, &rect, color),
                DrawCommand::Text {
                    rect,
                    text,
                    color,
                    font_size,
                    padding,
                } => draw_text(surface, &rect, &text, color, font_size, padding),
            }
        }
    }

    fn dispatch(&mut self, event: GuiEvent) -> Vec<StartupMenuAction> {
        let response = self.browser.handle_event(event);
        response
            .messages
            .into_iter()
            .filter_map(StartupMenuAction::from_browser_message)
            .collect()
    }
}

impl StartupMenuAction {
    fn from_browser_message(message: ScenarioBrowserMessage) -> Option<Self> {
        match message {
            ScenarioBrowserMessage::SelectionChanged(summary) => {
                Some(Self::SelectionChanged(summary.into()))
            }
            ScenarioBrowserMessage::StartScenario(summary) => {
                Some(Self::StartScenario(summary.into()))
            }
            ScenarioBrowserMessage::OpenEntry(summary) => Some(Self::OpenEntry(summary.into())),
            ScenarioBrowserMessage::EditEntry(summary) => Some(Self::EditEntry(summary.into())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry(identifier: &str, title: &str) -> ScenarioEntry {
        ScenarioEntry {
            identifier: identifier.to_string(),
            title: title.to_string(),
            description: Some("Example scenario".to_string()),
            kind: ScenarioKind::Scenario,
            is_editable: true,
            is_playable: true,
        }
    }

    #[test]
    fn key_navigation_emits_start_action() {
        let entries = vec![entry("rust_sandbox", "Rust Sandbox")];
        let mut menu = StartupMenu::new(entries).expect("menu");
        menu.resize(640.0, 480.0);

        let select = menu.handle_key_down(KeyCode::Down);
        assert!(select
            .iter()
            .any(|action| matches!(action, StartupMenuAction::SelectionChanged(summary) if summary.identifier == "rust_sandbox")));

        let start = menu.handle_key_down(KeyCode::Enter);
        assert!(start.iter().any(|action| {
            matches!(
                action,
                StartupMenuAction::StartScenario(summary) if summary.identifier == "rust_sandbox"
            )
        }));
    }
}
