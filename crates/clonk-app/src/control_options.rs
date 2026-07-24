use crate::input::{ControlBindingId, KeyboardBindings};
use clonk_frontend::{
    ControlOptionItem, ControlOptionsAction, ControlOptionsView, GuiPoint, KeyCode,
};
use clonk_graphics::{Surface, TextFont};
use std::sync::Arc;
use winit::event::VirtualKeyCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlOptionsCommand {
    SelectionChanged(ControlBindingId),
    BeginRebind(ControlBindingId),
    BindingUpdated(ControlBindingId),
    ResetAll,
    Close,
    UnsupportedKey(VirtualKeyCode),
}

pub struct ControlOptionsState {
    view: ControlOptionsView,
    binding_order: Vec<ControlBindingId>,
    waiting_for: Option<ControlBindingId>,
}

impl ControlOptionsState {
    pub fn new(font: Arc<dyn TextFont>) -> Self {
        Self {
            view: ControlOptionsView::new(font),
            binding_order: ControlBindingId::ALL.into_iter().collect(),
            waiting_for: None,
        }
    }

    pub fn resize(&mut self, width: f32, height: f32) {
        self.view.resize(width, height);
    }

    pub fn set_pointer_position(&mut self, position: Option<GuiPoint>) {
        self.view.set_pointer_position(position);
    }

    pub fn pointer_position(&self) -> Option<GuiPoint> {
        self.view.pointer_position()
    }

    pub fn set_selected_binding(&mut self, id: ControlBindingId) {
        if let Some(index) = self.index_for(id) {
            self.view.set_selected_index(Some(index));
        }
    }

    pub fn selected_binding(&self) -> Option<ControlBindingId> {
        self.view
            .selected_index()
            .and_then(|idx| self.binding_order.get(idx).copied())
    }

    pub fn begin_rebind(&mut self, id: ControlBindingId) {
        self.waiting_for = Some(id);
        self.refresh_waiting_marker();
    }

    pub fn cancel_rebind(&mut self) {
        self.waiting_for = None;
        self.refresh_waiting_marker();
    }

    pub fn apply_binding_change(&mut self, id: ControlBindingId, bindings: &KeyboardBindings) {
        if let Some(index) = self.index_for(id) {
            self.view.update_item(index, make_item(id, bindings));
        }
        self.cancel_rebind();
    }

    pub fn apply_reset_all(&mut self, bindings: &KeyboardBindings) {
        self.refresh_from_bindings(bindings);
        self.cancel_rebind();
    }

    pub fn refresh_from_bindings(&mut self, bindings: &KeyboardBindings) {
        let items = self
            .binding_order
            .iter()
            .map(|&id| make_item(id, bindings))
            .collect::<Vec<_>>();
        self.view.set_items(items);
        self.refresh_waiting_marker();
    }

    pub fn render(&mut self, surface: &mut Surface) {
        self.view.render(surface);
    }

    pub fn handle_pointer_move(&mut self, position: GuiPoint) -> Vec<ControlOptionsCommand> {
        let actions = self.view.handle_pointer_move(position);
        self.process_actions(actions)
    }

    pub fn handle_pointer_down(&mut self, position: GuiPoint) -> Vec<ControlOptionsCommand> {
        let actions = self.view.handle_pointer_down(position);
        self.process_actions(actions)
    }

    pub fn handle_pointer_up(&mut self, position: GuiPoint) -> Vec<ControlOptionsCommand> {
        let actions = self.view.handle_pointer_up(position);
        self.process_actions(actions)
    }

    pub fn handle_key_down(&mut self, key: KeyCode) -> Vec<ControlOptionsCommand> {
        let actions = self.view.handle_key_down(key);
        self.process_actions(actions)
    }

    pub fn handle_key_up(&mut self, key: KeyCode) -> Vec<ControlOptionsCommand> {
        let actions = self.view.handle_key_up(key);
        self.process_actions(actions)
    }

    pub fn handle_virtual_key(
        &mut self,
        key: VirtualKeyCode,
        bindings: &mut KeyboardBindings,
    ) -> Option<ControlOptionsCommand> {
        if let Some(waiting) = self.waiting_for {
            if key == VirtualKeyCode::Escape {
                self.cancel_rebind();
                return None;
            }
            if !KeyboardBindings::is_supported_key(key) {
                return Some(ControlOptionsCommand::UnsupportedKey(key));
            }
            bindings.rebind(waiting, key);
            self.apply_binding_change(waiting, bindings);
            return Some(ControlOptionsCommand::BindingUpdated(waiting));
        }

        match key {
            VirtualKeyCode::Back => {
                if let Some(binding) = self.selected_binding() {
                    bindings.reset_binding(binding);
                    self.apply_binding_change(binding, bindings);
                    return Some(ControlOptionsCommand::BindingUpdated(binding));
                }
                None
            }
            VirtualKeyCode::R => {
                bindings.reset_all();
                self.apply_reset_all(bindings);
                Some(ControlOptionsCommand::ResetAll)
            }
            _ => None,
        }
    }

    fn process_actions(
        &mut self,
        actions: Vec<ControlOptionsAction>,
    ) -> Vec<ControlOptionsCommand> {
        actions
            .into_iter()
            .filter_map(|action| self.translate_action(action))
            .collect()
    }

    fn translate_action(&mut self, action: ControlOptionsAction) -> Option<ControlOptionsCommand> {
        match action {
            ControlOptionsAction::SelectionChanged(index) => self
                .binding_order
                .get(index)
                .copied()
                .map(ControlOptionsCommand::SelectionChanged),
            ControlOptionsAction::RequestRebind(index) => {
                self.binding_order.get(index).copied().map(|id| {
                    self.begin_rebind(id);
                    ControlOptionsCommand::BeginRebind(id)
                })
            }
            ControlOptionsAction::ResetAll => Some(ControlOptionsCommand::ResetAll),
            ControlOptionsAction::Close => Some(ControlOptionsCommand::Close),
        }
    }

    fn refresh_waiting_marker(&mut self) {
        let index = self.waiting_for.and_then(|id| self.index_for(id));
        self.view.set_waiting_for_rebind(index);
    }

    fn index_for(&self, id: ControlBindingId) -> Option<usize> {
        self.binding_order
            .iter()
            .position(|candidate| *candidate == id)
    }
}

fn make_item(id: ControlBindingId, bindings: &KeyboardBindings) -> ControlOptionItem {
    let key = bindings.key_for(id).unwrap_or_else(|| id.default_key());
    let is_default = key == id.default_key();
    let mut key_label = format_key_label(key);
    if !is_default {
        key_label.push_str("  (custom)");
    }
    ControlOptionItem {
        label: binding_display_name(id).to_string(),
        key_label,
        is_default,
    }
}

pub fn binding_display_name(id: ControlBindingId) -> &'static str {
    match id {
        ControlBindingId::CursorLeft => "Cursor Left",
        ControlBindingId::CursorToggle => "Cursor Toggle",
        ControlBindingId::CursorRight => "Cursor Right",
        ControlBindingId::Throw => "Throw",
        ControlBindingId::Up => "Jump / Up",
        ControlBindingId::Dig => "Dig",
        ControlBindingId::Left => "Move Left",
        ControlBindingId::Down => "Duck / Down",
        ControlBindingId::Right => "Move Right",
        ControlBindingId::PlayerMenu => "Player Menu",
        ControlBindingId::Special => "Special",
        ControlBindingId::Special2 => "Special 2",
    }
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn sdl_key_label(key: VirtualKeyCode) -> Option<&'static str> {
    Some(match key {
        VirtualKeyCode::Return => "Return",
        VirtualKeyCode::Escape => "Escape",
        VirtualKeyCode::Back => "Backspace",
        VirtualKeyCode::Tab => "Tab",
        VirtualKeyCode::Space => "Space",
        VirtualKeyCode::Minus => "-",
        VirtualKeyCode::Equals => "=",
        VirtualKeyCode::LBracket => "[",
        VirtualKeyCode::RBracket => "]",
        VirtualKeyCode::Backslash => "\\",
        VirtualKeyCode::Semicolon => ";",
        VirtualKeyCode::Apostrophe => "'",
        VirtualKeyCode::Grave => "`",
        VirtualKeyCode::Comma => ",",
        VirtualKeyCode::Period => ".",
        VirtualKeyCode::Slash => "/",
        VirtualKeyCode::Capital => "CapsLock",
        VirtualKeyCode::Snapshot => "PrintScreen",
        VirtualKeyCode::Scroll => "ScrollLock",
        VirtualKeyCode::Pause => "Pause",
        VirtualKeyCode::Insert => "Insert",
        VirtualKeyCode::Home => "Home",
        VirtualKeyCode::PageUp => "PageUp",
        VirtualKeyCode::Delete => "Delete",
        VirtualKeyCode::End => "End",
        VirtualKeyCode::PageDown => "PageDown",
        VirtualKeyCode::Right => "Right",
        VirtualKeyCode::Left => "Left",
        VirtualKeyCode::Down => "Down",
        VirtualKeyCode::Up => "Up",
        VirtualKeyCode::Numlock => "Numlock",
        VirtualKeyCode::NumpadDivide => "Keypad /",
        VirtualKeyCode::NumpadMultiply => "Keypad *",
        VirtualKeyCode::NumpadSubtract => "Keypad -",
        VirtualKeyCode::NumpadAdd => "Keypad +",
        VirtualKeyCode::NumpadEnter => "Keypad Enter",
        VirtualKeyCode::Numpad1 => "Keypad 1",
        VirtualKeyCode::Numpad2 => "Keypad 2",
        VirtualKeyCode::Numpad3 => "Keypad 3",
        VirtualKeyCode::Numpad4 => "Keypad 4",
        VirtualKeyCode::Numpad5 => "Keypad 5",
        VirtualKeyCode::Numpad6 => "Keypad 6",
        VirtualKeyCode::Numpad7 => "Keypad 7",
        VirtualKeyCode::Numpad8 => "Keypad 8",
        VirtualKeyCode::Numpad9 => "Keypad 9",
        VirtualKeyCode::Numpad0 => "Keypad 0",
        VirtualKeyCode::NumpadDecimal => "Keypad .",
        // SDL_SCANCODE_NONUSBACKSLASH has no default SDL 2.32.10 name.
        VirtualKeyCode::OEM102 => "",
        VirtualKeyCode::Apps => "Application",
        VirtualKeyCode::NumpadEquals => "Keypad =",
        VirtualKeyCode::NumpadComma => "Keypad ,",
        _ => return None,
    })
}

pub fn format_key_label(key: VirtualKeyCode) -> String {
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    if let Some(label) = sdl_key_label(key) {
        return label.to_string();
    }

    match key {
        VirtualKeyCode::A => "A".into(),
        VirtualKeyCode::B => "B".into(),
        VirtualKeyCode::C => "C".into(),
        VirtualKeyCode::D => "D".into(),
        VirtualKeyCode::E => "E".into(),
        VirtualKeyCode::F => "F".into(),
        VirtualKeyCode::G => "G".into(),
        VirtualKeyCode::H => "H".into(),
        VirtualKeyCode::I => "I".into(),
        VirtualKeyCode::J => "J".into(),
        VirtualKeyCode::K => "K".into(),
        VirtualKeyCode::L => "L".into(),
        VirtualKeyCode::M => "M".into(),
        VirtualKeyCode::N => "N".into(),
        VirtualKeyCode::O => "O".into(),
        VirtualKeyCode::P => "P".into(),
        VirtualKeyCode::Q => "Q".into(),
        VirtualKeyCode::R => "R".into(),
        VirtualKeyCode::S => "S".into(),
        VirtualKeyCode::T => "T".into(),
        VirtualKeyCode::U => "U".into(),
        VirtualKeyCode::V => "V".into(),
        VirtualKeyCode::W => "W".into(),
        VirtualKeyCode::X => "X".into(),
        VirtualKeyCode::Y => "Y".into(),
        VirtualKeyCode::Z => "Z".into(),
        VirtualKeyCode::Key0 => "0".into(),
        VirtualKeyCode::Key1 => "1".into(),
        VirtualKeyCode::Key2 => "2".into(),
        VirtualKeyCode::Key3 => "3".into(),
        VirtualKeyCode::Key4 => "4".into(),
        VirtualKeyCode::Key5 => "5".into(),
        VirtualKeyCode::Key6 => "6".into(),
        VirtualKeyCode::Key7 => "7".into(),
        VirtualKeyCode::Key8 => "8".into(),
        VirtualKeyCode::Key9 => "9".into(),
        VirtualKeyCode::Space => "Space".into(),
        VirtualKeyCode::Left => "Left".into(),
        VirtualKeyCode::Right => "Right".into(),
        VirtualKeyCode::Up => "Up".into(),
        VirtualKeyCode::Down => "Down".into(),
        other => format!("{:?}", other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn l050_alphanumeric_key_labels_remain_unchanged() {
        for (key, expected) in [
            (VirtualKeyCode::A, "A"),
            (VirtualKeyCode::B, "B"),
            (VirtualKeyCode::C, "C"),
            (VirtualKeyCode::D, "D"),
            (VirtualKeyCode::E, "E"),
            (VirtualKeyCode::F, "F"),
            (VirtualKeyCode::G, "G"),
            (VirtualKeyCode::H, "H"),
            (VirtualKeyCode::I, "I"),
            (VirtualKeyCode::J, "J"),
            (VirtualKeyCode::K, "K"),
            (VirtualKeyCode::L, "L"),
            (VirtualKeyCode::M, "M"),
            (VirtualKeyCode::N, "N"),
            (VirtualKeyCode::O, "O"),
            (VirtualKeyCode::P, "P"),
            (VirtualKeyCode::Q, "Q"),
            (VirtualKeyCode::R, "R"),
            (VirtualKeyCode::S, "S"),
            (VirtualKeyCode::T, "T"),
            (VirtualKeyCode::U, "U"),
            (VirtualKeyCode::V, "V"),
            (VirtualKeyCode::W, "W"),
            (VirtualKeyCode::X, "X"),
            (VirtualKeyCode::Y, "Y"),
            (VirtualKeyCode::Z, "Z"),
            (VirtualKeyCode::Key0, "0"),
            (VirtualKeyCode::Key1, "1"),
            (VirtualKeyCode::Key2, "2"),
            (VirtualKeyCode::Key3, "3"),
            (VirtualKeyCode::Key4, "4"),
            (VirtualKeyCode::Key5, "5"),
            (VirtualKeyCode::Key6, "6"),
            (VirtualKeyCode::Key7, "7"),
            (VirtualKeyCode::Key8, "8"),
            (VirtualKeyCode::Key9, "9"),
        ] {
            assert_eq!(format_key_label(key), expected, "label for {key:?}");
        }
    }

    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    #[test]
    fn l050_key_labels_match_sdl_scancode_names() {
        for (key, expected) in [
            (VirtualKeyCode::Return, "Return"),
            (VirtualKeyCode::Escape, "Escape"),
            (VirtualKeyCode::Back, "Backspace"),
            (VirtualKeyCode::Tab, "Tab"),
            (VirtualKeyCode::Space, "Space"),
            (VirtualKeyCode::Minus, "-"),
            (VirtualKeyCode::Equals, "="),
            (VirtualKeyCode::LBracket, "["),
            (VirtualKeyCode::RBracket, "]"),
            (VirtualKeyCode::Backslash, "\\"),
            (VirtualKeyCode::Semicolon, ";"),
            (VirtualKeyCode::Apostrophe, "'"),
            (VirtualKeyCode::Grave, "`"),
            (VirtualKeyCode::Comma, ","),
            (VirtualKeyCode::Period, "."),
            (VirtualKeyCode::Slash, "/"),
            (VirtualKeyCode::Capital, "CapsLock"),
            (VirtualKeyCode::F1, "F1"),
            (VirtualKeyCode::F2, "F2"),
            (VirtualKeyCode::F3, "F3"),
            (VirtualKeyCode::F4, "F4"),
            (VirtualKeyCode::F5, "F5"),
            (VirtualKeyCode::F6, "F6"),
            (VirtualKeyCode::F7, "F7"),
            (VirtualKeyCode::F8, "F8"),
            (VirtualKeyCode::F9, "F9"),
            (VirtualKeyCode::F10, "F10"),
            (VirtualKeyCode::F11, "F11"),
            (VirtualKeyCode::F12, "F12"),
            (VirtualKeyCode::F13, "F13"),
            (VirtualKeyCode::F14, "F14"),
            (VirtualKeyCode::F15, "F15"),
            (VirtualKeyCode::F16, "F16"),
            (VirtualKeyCode::F17, "F17"),
            (VirtualKeyCode::F18, "F18"),
            (VirtualKeyCode::F19, "F19"),
            (VirtualKeyCode::F20, "F20"),
            (VirtualKeyCode::F21, "F21"),
            (VirtualKeyCode::F22, "F22"),
            (VirtualKeyCode::F23, "F23"),
            (VirtualKeyCode::F24, "F24"),
            (VirtualKeyCode::Snapshot, "PrintScreen"),
            (VirtualKeyCode::Scroll, "ScrollLock"),
            (VirtualKeyCode::Pause, "Pause"),
            (VirtualKeyCode::Insert, "Insert"),
            (VirtualKeyCode::Home, "Home"),
            (VirtualKeyCode::PageUp, "PageUp"),
            (VirtualKeyCode::Delete, "Delete"),
            (VirtualKeyCode::End, "End"),
            (VirtualKeyCode::PageDown, "PageDown"),
            (VirtualKeyCode::Right, "Right"),
            (VirtualKeyCode::Left, "Left"),
            (VirtualKeyCode::Down, "Down"),
            (VirtualKeyCode::Up, "Up"),
            (VirtualKeyCode::Numlock, "Numlock"),
            (VirtualKeyCode::NumpadDivide, "Keypad /"),
            (VirtualKeyCode::NumpadMultiply, "Keypad *"),
            (VirtualKeyCode::NumpadSubtract, "Keypad -"),
            (VirtualKeyCode::NumpadAdd, "Keypad +"),
            (VirtualKeyCode::NumpadEnter, "Keypad Enter"),
            (VirtualKeyCode::Numpad1, "Keypad 1"),
            (VirtualKeyCode::Numpad2, "Keypad 2"),
            (VirtualKeyCode::Numpad3, "Keypad 3"),
            (VirtualKeyCode::Numpad4, "Keypad 4"),
            (VirtualKeyCode::Numpad5, "Keypad 5"),
            (VirtualKeyCode::Numpad6, "Keypad 6"),
            (VirtualKeyCode::Numpad7, "Keypad 7"),
            (VirtualKeyCode::Numpad8, "Keypad 8"),
            (VirtualKeyCode::Numpad9, "Keypad 9"),
            (VirtualKeyCode::Numpad0, "Keypad 0"),
            (VirtualKeyCode::NumpadDecimal, "Keypad ."),
            (VirtualKeyCode::OEM102, ""),
            (VirtualKeyCode::Apps, "Application"),
            (VirtualKeyCode::NumpadEquals, "Keypad ="),
            (VirtualKeyCode::NumpadComma, "Keypad ,"),
        ] {
            assert_eq!(format_key_label(key), expected, "label for {key:?}");
        }
    }
}
