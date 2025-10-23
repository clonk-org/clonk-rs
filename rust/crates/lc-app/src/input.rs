use std::collections::{HashMap, HashSet};

use lc_core::std_config::Config;
use lc_engine::{CommandKind, ControlButton, ControlCommand, ControlEvent};
use lc_platform::AppPaths;
use winit::event::{ElementState, VirtualKeyCode};

/// Keyboard control bindings backed by the legacy `Config.Controls` section.
#[derive(Debug, Clone)]
pub struct KeyboardBindings {
    bindings: HashMap<VirtualKeyCode, Binding>,
    clear_keys: HashSet<VirtualKeyCode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Binding {
    kind: BindingKind,
    emit_release: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BindingKind {
    Button(ControlButton),
    Command(ControlCommand),
}

impl Binding {
    const fn button(button: ControlButton) -> Self {
        Self {
            kind: BindingKind::Button(button),
            emit_release: true,
        }
    }

    const fn command(command: ControlCommand, emit_release: bool) -> Self {
        Self {
            kind: BindingKind::Command(command),
            emit_release,
        }
    }

    fn press_event(self) -> ControlEvent {
        match self.kind {
            BindingKind::Button(button) => ControlEvent::Press(button),
            BindingKind::Command(command) => ControlEvent::Command {
                command,
                kind: CommandKind::Press,
            },
        }
    }

    fn release_event(self) -> Option<ControlEvent> {
        if !self.emit_release {
            return None;
        }
        match self.kind {
            BindingKind::Button(button) => Some(ControlEvent::Release(button)),
            BindingKind::Command(command) => Some(ControlEvent::Command {
                command,
                kind: CommandKind::Release,
            }),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct ControlBindingSpec {
    index: usize,
    default_key: VirtualKeyCode,
    binding: Binding,
}

impl ControlBindingSpec {
    const fn new(index: usize, default_key: VirtualKeyCode, binding: Binding) -> Self {
        Self {
            index,
            default_key,
            binding,
        }
    }
}

impl KeyboardBindings {
    /// Loads bindings from the user config, falling back to the built-in defaults when parsing
    /// fails or no user configuration is available.
    pub fn load(paths: Option<&AppPaths>) -> Self {
        let mut bindings = KeyboardBindings::default_bindings();
        let Some(paths) = paths else {
            return bindings;
        };
        let config_path = paths.config_file();
        match Config::load(&config_path) {
            Ok(config) => {
                if let Some(overrides) = KeyboardBindings::from_config(&config) {
                    bindings = overrides;
                }
            }
            Err(err) => {
                if err.kind() != std::io::ErrorKind::NotFound {
                    tracing::warn!(
                        error = %err,
                        path = %config_path.display(),
                        "failed to load controls config"
                    );
                }
            }
        }
        bindings
    }

    fn from_config(config: &Config) -> Option<Self> {
        let mut bindings = KeyboardBindings::default_bindings();
        let mut any_override = false;

        for spec in CONTROL_BINDING_SPECS {
            if let Some(key) = read_keyboard_entry(config, 0, spec.index) {
                bindings.set_binding(spec.binding, key);
                any_override = true;
            }
        }

        if any_override {
            Some(bindings)
        } else {
            None
        }
    }

    fn default_bindings() -> Self {
        let mut bindings = HashMap::new();
        for spec in CONTROL_BINDING_SPECS {
            bindings.insert(spec.default_key, spec.binding);
        }

        insert_default_binding(
            &mut bindings,
            VirtualKeyCode::Up,
            Binding::button(ControlButton::Up),
        );
        insert_default_binding(
            &mut bindings,
            VirtualKeyCode::Left,
            Binding::button(ControlButton::Left),
        );
        insert_default_binding(
            &mut bindings,
            VirtualKeyCode::Down,
            Binding::button(ControlButton::Down),
        );
        insert_default_binding(
            &mut bindings,
            VirtualKeyCode::Right,
            Binding::button(ControlButton::Right),
        );

        let clear_keys = HashSet::from([VirtualKeyCode::Space]);

        Self {
            bindings,
            clear_keys,
        }
    }

    fn set_binding(&mut self, binding: Binding, key: VirtualKeyCode) {
        self.bindings.remove(&key);
        self.bindings.retain(|_, mapped| *mapped != binding);
        self.bindings.insert(key, binding);
    }

    /// Returns the engine control event to emit for a given keyboard input.
    pub fn event_for_key(&self, key: VirtualKeyCode, state: ElementState) -> Option<ControlEvent> {
        match state {
            ElementState::Pressed => {
                if let Some(binding) = self.bindings.get(&key) {
                    Some(binding.press_event())
                } else if self.clear_keys.contains(&key) {
                    Some(ControlEvent::ClearPressed)
                } else {
                    None
                }
            }
            ElementState::Released => self
                .bindings
                .get(&key)
                .and_then(|binding| binding.release_event()),
        }
    }
}

const CONTROL_BINDING_SPECS: &[ControlBindingSpec] = &[
    ControlBindingSpec::new(
        0,
        VirtualKeyCode::Q,
        Binding::command(ControlCommand::CursorLeft, true),
    ),
    ControlBindingSpec::new(
        1,
        VirtualKeyCode::W,
        Binding::command(ControlCommand::CursorToggle, true),
    ),
    ControlBindingSpec::new(
        2,
        VirtualKeyCode::E,
        Binding::command(ControlCommand::CursorRight, true),
    ),
    ControlBindingSpec::new(
        3,
        VirtualKeyCode::A,
        Binding::command(ControlCommand::Throw, true),
    ),
    ControlBindingSpec::new(4, VirtualKeyCode::S, Binding::button(ControlButton::Up)),
    ControlBindingSpec::new(
        5,
        VirtualKeyCode::D,
        Binding::command(ControlCommand::Dig, true),
    ),
    ControlBindingSpec::new(6, VirtualKeyCode::Z, Binding::button(ControlButton::Left)),
    ControlBindingSpec::new(7, VirtualKeyCode::X, Binding::button(ControlButton::Down)),
    ControlBindingSpec::new(8, VirtualKeyCode::C, Binding::button(ControlButton::Right)),
    ControlBindingSpec::new(
        9,
        VirtualKeyCode::R,
        Binding::command(ControlCommand::PlayerMenu, false),
    ),
    ControlBindingSpec::new(
        10,
        VirtualKeyCode::V,
        Binding::command(ControlCommand::Special, true),
    ),
    ControlBindingSpec::new(
        11,
        VirtualKeyCode::F,
        Binding::command(ControlCommand::Special2, true),
    ),
];

fn read_keyboard_entry(
    config: &Config,
    set_index: usize,
    control_index: usize,
) -> Option<VirtualKeyCode> {
    let key_name = format!("Kbd{}Key{}", set_index + 1, control_index + 1);
    let raw = config.get_in(Some("Controls"), &key_name)?;
    parse_key_code_value(raw)
}

fn parse_key_code_value(raw: &str) -> Option<VirtualKeyCode> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let value = if let Some(hex) = trimmed.strip_prefix("0x") {
        i32::from_str_radix(hex, 16).ok()
    } else if let Some(hex) = trimmed.strip_prefix("0X") {
        i32::from_str_radix(hex, 16).ok()
    } else {
        trimmed.parse::<i32>().ok()
    }?;
    map_numeric_keycode(value)
}

fn map_numeric_keycode(value: i32) -> Option<VirtualKeyCode> {
    #[cfg(not(target_os = "windows"))]
    if let Some(arrow) = map_sdl_arrow_scancode(value) {
        return Some(arrow);
    }
    if let Some(ascii_key) = map_ascii_keycode(value) {
        return Some(ascii_key);
    }
    if let Some(scancode_key) = map_sdl_letter_scancode(value) {
        return Some(scancode_key);
    }
    if let Some(scancode_digit) = map_sdl_digit_scancode(value) {
        return Some(scancode_digit);
    }
    #[cfg(target_os = "windows")]
    if let Some(arrow) = map_sdl_arrow_scancode(value) {
        return Some(arrow);
    }
    match value {
        0x25 | 0xff51 | 80 => Some(VirtualKeyCode::Left),
        0x26 | 0xff52 | 82 => Some(VirtualKeyCode::Up),
        0x27 | 0xff53 | 79 => Some(VirtualKeyCode::Right),
        0x28 | 0xff54 | 81 => Some(VirtualKeyCode::Down),
        0x20 | 44 => Some(VirtualKeyCode::Space),
        _ => None,
    }
}

fn map_ascii_keycode(value: i32) -> Option<VirtualKeyCode> {
    let as_u32 = value as u32;
    if as_u32 > 0x7f {
        return None;
    }
    let ch = char::from_u32(as_u32)?.to_ascii_uppercase();
    match ch {
        'A' => Some(VirtualKeyCode::A),
        'B' => Some(VirtualKeyCode::B),
        'C' => Some(VirtualKeyCode::C),
        'D' => Some(VirtualKeyCode::D),
        'E' => Some(VirtualKeyCode::E),
        'F' => Some(VirtualKeyCode::F),
        'G' => Some(VirtualKeyCode::G),
        'H' => Some(VirtualKeyCode::H),
        'I' => Some(VirtualKeyCode::I),
        'J' => Some(VirtualKeyCode::J),
        'K' => Some(VirtualKeyCode::K),
        'L' => Some(VirtualKeyCode::L),
        'M' => Some(VirtualKeyCode::M),
        'N' => Some(VirtualKeyCode::N),
        'O' => Some(VirtualKeyCode::O),
        'P' => Some(VirtualKeyCode::P),
        'Q' => Some(VirtualKeyCode::Q),
        'R' => Some(VirtualKeyCode::R),
        'S' => Some(VirtualKeyCode::S),
        'T' => Some(VirtualKeyCode::T),
        'U' => Some(VirtualKeyCode::U),
        'V' => Some(VirtualKeyCode::V),
        'W' => Some(VirtualKeyCode::W),
        'X' => Some(VirtualKeyCode::X),
        'Y' => Some(VirtualKeyCode::Y),
        'Z' => Some(VirtualKeyCode::Z),
        '0' => Some(VirtualKeyCode::Key0),
        '1' => Some(VirtualKeyCode::Key1),
        '2' => Some(VirtualKeyCode::Key2),
        '3' => Some(VirtualKeyCode::Key3),
        '4' => Some(VirtualKeyCode::Key4),
        '5' => Some(VirtualKeyCode::Key5),
        '6' => Some(VirtualKeyCode::Key6),
        '7' => Some(VirtualKeyCode::Key7),
        '8' => Some(VirtualKeyCode::Key8),
        '9' => Some(VirtualKeyCode::Key9),
        ' ' => Some(VirtualKeyCode::Space),
        _ => None,
    }
}

fn map_sdl_letter_scancode(value: i32) -> Option<VirtualKeyCode> {
    match value {
        4 => Some(VirtualKeyCode::A),
        5 => Some(VirtualKeyCode::B),
        6 => Some(VirtualKeyCode::C),
        7 => Some(VirtualKeyCode::D),
        8 => Some(VirtualKeyCode::E),
        9 => Some(VirtualKeyCode::F),
        10 => Some(VirtualKeyCode::G),
        11 => Some(VirtualKeyCode::H),
        12 => Some(VirtualKeyCode::I),
        13 => Some(VirtualKeyCode::J),
        14 => Some(VirtualKeyCode::K),
        15 => Some(VirtualKeyCode::L),
        16 => Some(VirtualKeyCode::M),
        17 => Some(VirtualKeyCode::N),
        18 => Some(VirtualKeyCode::O),
        19 => Some(VirtualKeyCode::P),
        20 => Some(VirtualKeyCode::Q),
        21 => Some(VirtualKeyCode::R),
        22 => Some(VirtualKeyCode::S),
        23 => Some(VirtualKeyCode::T),
        24 => Some(VirtualKeyCode::U),
        25 => Some(VirtualKeyCode::V),
        26 => Some(VirtualKeyCode::W),
        27 => Some(VirtualKeyCode::X),
        28 => Some(VirtualKeyCode::Y),
        29 => Some(VirtualKeyCode::Z),
        _ => None,
    }
}

fn map_sdl_digit_scancode(value: i32) -> Option<VirtualKeyCode> {
    match value {
        30 => Some(VirtualKeyCode::Key1),
        31 => Some(VirtualKeyCode::Key2),
        32 => Some(VirtualKeyCode::Key3),
        33 => Some(VirtualKeyCode::Key4),
        34 => Some(VirtualKeyCode::Key5),
        35 => Some(VirtualKeyCode::Key6),
        36 => Some(VirtualKeyCode::Key7),
        37 => Some(VirtualKeyCode::Key8),
        38 => Some(VirtualKeyCode::Key9),
        39 => Some(VirtualKeyCode::Key0),
        _ => None,
    }
}

fn map_sdl_arrow_scancode(value: i32) -> Option<VirtualKeyCode> {
    match value {
        79 => Some(VirtualKeyCode::Right),
        80 => Some(VirtualKeyCode::Left),
        81 => Some(VirtualKeyCode::Down),
        82 => Some(VirtualKeyCode::Up),
        _ => None,
    }
}

fn insert_default_binding(
    bindings: &mut HashMap<VirtualKeyCode, Binding>,
    key: VirtualKeyCode,
    binding: Binding,
) {
    bindings.insert(key, binding);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_bindings_cover_basic_controls() {
        let bindings = KeyboardBindings::default_bindings();
        assert_eq!(
            bindings.event_for_key(VirtualKeyCode::S, ElementState::Pressed),
            Some(ControlEvent::Press(ControlButton::Up))
        );
        assert_eq!(
            bindings.event_for_key(VirtualKeyCode::C, ElementState::Released),
            Some(ControlEvent::Release(ControlButton::Right))
        );
        assert_eq!(
            bindings.event_for_key(VirtualKeyCode::Space, ElementState::Pressed),
            Some(ControlEvent::ClearPressed)
        );
    }

    #[test]
    fn cursor_toggle_binding_produces_command() {
        let bindings = KeyboardBindings::default_bindings();
        assert_eq!(
            bindings.event_for_key(VirtualKeyCode::W, ElementState::Pressed),
            Some(ControlEvent::Command {
                command: ControlCommand::CursorToggle,
                kind: CommandKind::Press
            })
        );
        assert_eq!(
            bindings.event_for_key(VirtualKeyCode::W, ElementState::Released),
            Some(ControlEvent::Command {
                command: ControlCommand::CursorToggle,
                kind: CommandKind::Release
            })
        );
    }

    #[test]
    fn player_menu_binding_has_no_release() {
        let bindings = KeyboardBindings::default_bindings();
        assert_eq!(
            bindings.event_for_key(VirtualKeyCode::R, ElementState::Pressed),
            Some(ControlEvent::Command {
                command: ControlCommand::PlayerMenu,
                kind: CommandKind::Press
            })
        );
        assert_eq!(
            bindings.event_for_key(VirtualKeyCode::R, ElementState::Released),
            None,
            "player menu key should not emit release event"
        );
    }

    #[test]
    fn config_overrides_replace_defaults() {
        let mut cfg = Config::new();
        cfg.set_in(Some("Controls"), "Kbd1Key5", "87"); // W
        cfg.set_in(Some("Controls"), "Kbd1Key7", "65"); // A
        cfg.set_in(Some("Controls"), "Kbd1Key8", "83"); // S
        cfg.set_in(Some("Controls"), "Kbd1Key9", "68"); // D
        let bindings = KeyboardBindings::from_config(&cfg).expect("overrides present");

        assert_eq!(
            bindings.event_for_key(VirtualKeyCode::W, ElementState::Pressed),
            Some(ControlEvent::Press(ControlButton::Up))
        );
        assert_eq!(
            bindings.event_for_key(VirtualKeyCode::A, ElementState::Released),
            Some(ControlEvent::Release(ControlButton::Left))
        );
        // Falling back to default should still handle the clear command.
        assert_eq!(
            bindings.event_for_key(VirtualKeyCode::Space, ElementState::Pressed),
            Some(ControlEvent::ClearPressed)
        );
    }

    #[test]
    fn parse_supports_hex_values() {
        assert_eq!(
            parse_key_code_value("0x41"),
            Some(VirtualKeyCode::A),
            "hex parsing should support uppercase prefix"
        );
        assert_eq!(
            parse_key_code_value("0X42"),
            Some(VirtualKeyCode::B),
            "hex parsing should support alternate prefix"
        );
    }

    #[test]
    fn parse_supports_sdl_scancodes() {
        assert_eq!(
            parse_key_code_value("22"),
            Some(VirtualKeyCode::S),
            "SDL scancode for S should parse"
        );
        assert_eq!(
            parse_key_code_value("79"),
            Some(VirtualKeyCode::Right),
            "SDL scancode for Right should parse"
        );
    }
}
