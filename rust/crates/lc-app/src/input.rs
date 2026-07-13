use std::io::ErrorKind;

use lc_core::std_config::Config;
use lc_engine::{CommandKind, ControlButton, ControlCommand, ControlEvent};
use lc_platform::AppPaths;
use winit::event::{ElementState, VirtualKeyCode};

#[repr(usize)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ControlBindingId {
    CursorLeft = 0,
    CursorToggle = 1,
    CursorRight = 2,
    Throw = 3,
    Up = 4,
    Dig = 5,
    Left = 6,
    Down = 7,
    Right = 8,
    PlayerMenu = 9,
    Special = 10,
    Special2 = 11,
}

impl ControlBindingId {
    pub const ALL: [ControlBindingId; 12] = [
        ControlBindingId::CursorLeft,
        ControlBindingId::CursorToggle,
        ControlBindingId::CursorRight,
        ControlBindingId::Throw,
        ControlBindingId::Up,
        ControlBindingId::Dig,
        ControlBindingId::Left,
        ControlBindingId::Down,
        ControlBindingId::Right,
        ControlBindingId::PlayerMenu,
        ControlBindingId::Special,
        ControlBindingId::Special2,
    ];

    pub fn default_key(self) -> VirtualKeyCode {
        cpp_default_keyboard_keys(is_german_system())[0][self.spec().index]
    }

    fn spec(self) -> &'static ControlBindingSpec {
        &CONTROL_BINDING_SPECS[self as usize]
    }
}

/// Keyboard control bindings backed by the legacy `Config.Controls` section.
#[derive(Debug, Clone)]
pub struct KeyboardBindings {
    keys: [[VirtualKeyCode; CONTROL_BINDING_COUNT]; KEYBOARD_SET_COUNT],
}

/// C++ `[Gamepad0]` through `[Gamepad3]` player-control registrations.
///
/// Each `ButtonN` is a logical control slot whose value is a complete C++
/// physical keycode. Missing entries stay unregistered; C++ defaults all of
/// them to `-1` rather than installing controller-layout defaults
/// (C4Config.cpp:287-317; C4Game.cpp:3439-3452).
#[derive(Debug, Clone)]
pub struct GamepadBindings {
    keys: [[Option<i32>; CONTROL_BINDING_COUNT]; GAMEPAD_SET_COUNT],
}

const KEYBOARD_SET_COUNT: usize = 4;
const GAMEPAD_SET_COUNT: usize = 4;
const GAMEPAD_CONTROL_SET_OFFSET: usize = KEYBOARD_SET_COUNT;
const CONTROL_BINDING_COUNT: usize = 12;
const LEGACY_GAMEPAD_KEY_PREFIX: i32 = 0x0042_0000;
const LEGACY_GAMEPAD_BUTTON_OFFSET: u8 = 10;
const LEGACY_GAMEPAD_BUTTON_COUNT: u8 = 32;

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
    id: ControlBindingId,
    index: usize,
    binding: Binding,
}

impl ControlBindingSpec {
    const fn new(id: ControlBindingId, index: usize, binding: Binding) -> Self {
        Self { id, index, binding }
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

        for set_index in 0..KEYBOARD_SET_COUNT {
            for spec in CONTROL_BINDING_SPECS {
                if let Some(key) = read_keyboard_entry(config, set_index, spec.index) {
                    bindings.assign_binding(set_index, spec.id, key);
                    any_override = true;
                }
            }
        }

        if any_override {
            Some(bindings)
        } else {
            None
        }
    }

    fn default_bindings() -> Self {
        Self {
            keys: cpp_default_keyboard_keys(is_german_system()),
        }
    }

    pub fn key_for(&self, id: ControlBindingId) -> Option<VirtualKeyCode> {
        self.key_for_set(0, id)
    }

    pub fn key_for_set(
        &self,
        control_set: usize,
        id: ControlBindingId,
    ) -> Option<VirtualKeyCode> {
        self.keys
            .get(control_set)
            .map(|keys| keys[id.spec().index])
    }

    pub fn rebind(&mut self, id: ControlBindingId, key: VirtualKeyCode) {
        self.assign_binding(0, id, key);
    }

    pub fn reset_binding(&mut self, id: ControlBindingId) {
        self.assign_binding(0, id, id.default_key());
    }

    pub fn reset_all(&mut self) {
        for spec in CONTROL_BINDING_SPECS {
            self.assign_binding(0, spec.id, spec.id.default_key());
        }
    }

    pub fn is_supported_key(key: VirtualKeyCode) -> bool {
        encode_virtual_key_code(key).is_some()
    }

    pub fn save(&self, paths: &AppPaths) {
        let config_path = paths.config_file();
        let mut config = match Config::load(&config_path) {
            Ok(existing) => existing,
            Err(err) if err.kind() == ErrorKind::NotFound => Config::new(),
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    path = %config_path.display(),
                    "failed to load config for saving controls"
                );
                return;
            }
        };

        for spec in CONTROL_BINDING_SPECS {
            let key_name = format!("Kbd{}Key{}", 1, spec.index + 1);
            let keycode = self.key_for(spec.id).unwrap_or_else(|| spec.id.default_key());
            if let Some(encoded) = encode_virtual_key_code(keycode) {
                config.set_in(Some("Controls"), &key_name, encoded.to_string());
            } else {
                tracing::warn!(
                    ?keycode,
                    "skipping persistence for unsupported virtual key code"
                );
            }
        }

        if let Err(err) = config.save(&config_path) {
            tracing::warn!(
                error = %err,
                path = %config_path.display(),
                "failed to persist control bindings"
            );
        }
    }

    fn assign_binding(
        &mut self,
        control_set: usize,
        id: ControlBindingId,
        key: VirtualKeyCode,
    ) {
        self.keys[control_set][id.spec().index] = key;
    }

    /// Returns every player-control callback candidate for a physical key in
    /// the exact set-major, control-major order used by `C4Game::InitKeyboard`.
    /// `None` retains callbacks such as PlayerMenu key-up that can consume the
    /// input without emitting a synchronized command.
    pub fn control_candidates_for_key(
        &self,
        key: VirtualKeyCode,
        state: ElementState,
    ) -> impl Iterator<Item = (usize, Option<ControlEvent>)> + '_ {
        self.keys
            .iter()
            .enumerate()
            .flat_map(move |(control_set, keys)| {
                keys.iter()
                    .enumerate()
                    .filter_map(move |(control_index, configured_key)| {
                        if *configured_key != key {
                            return None;
                        }
                        let binding = CONTROL_BINDING_SPECS[control_index].binding;
                        let event = match state {
                            ElementState::Pressed => Some(binding.press_event()),
                            ElementState::Released => binding.release_event(),
                        };
                        Some((control_set, event))
                    })
            })
    }

    /// Filters the exact callback stream down to candidates that emit an
    /// engine event. Routing code that models C++ consumption must use
    /// `control_candidates_for_key` instead.
    pub fn control_events_for_key(
        &self,
        key: VirtualKeyCode,
        state: ElementState,
    ) -> impl Iterator<Item = (usize, ControlEvent)> + '_ {
        self.control_candidates_for_key(key, state)
            .filter_map(|(control_set, event)| event.map(|event| (control_set, event)))
    }

    /// Returns the engine control event to emit for a given keyboard input.
    pub fn event_for_key(&self, key: VirtualKeyCode, state: ElementState) -> Option<ControlEvent> {
        // The running app still routes only keyboard set 1. Preserve its
        // previous last-configured-wins collision behavior until routing can
        // consume the ordered candidates above using live player control sets.
        let set_zero = self
            .control_candidates_for_key(key, state)
            .filter(|(control_set, _)| *control_set == 0)
            .last();
        if let Some((_, event)) = set_zero {
            return event;
        }
        None
    }
}

impl GamepadBindings {
    pub fn load(paths: Option<&AppPaths>) -> Self {
        let Some(paths) = paths else {
            return Self::default();
        };
        let config_path = paths.config_file();
        match Config::load(&config_path) {
            Ok(config) => Self::from_config(&config),
            Err(err) => {
                if err.kind() != ErrorKind::NotFound {
                    tracing::warn!(
                        error = %err,
                        path = %config_path.display(),
                        "failed to load gamepad controls config"
                    );
                }
                Self::default()
            }
        }
    }

    pub(crate) fn from_config(config: &Config) -> Self {
        let mut bindings = Self::default();
        for gamepad_index in 0..GAMEPAD_SET_COUNT {
            let section = format!("Gamepad{gamepad_index}");
            for control_index in 0..CONTROL_BINDING_COUNT {
                let key = format!("Button{}", control_index + 1);
                bindings.keys[gamepad_index][control_index] = config
                    .get_in(Some(&section), &key)
                    .and_then(parse_raw_key_code_value)
                    .filter(|key| *key != -1);
            }
        }
        bindings
    }

    /// Reproduce the callbacks installed by the outer gamepad-set and inner
    /// logical-control loops in `C4Game::InitKeyboard`. The physical slot is
    /// encoded into the candidate key; the config section determines control
    /// set 4..7. Keeping those identities separate also preserves handwritten
    /// configs whose full keycode names a different physical pad.
    pub fn control_candidates_for_button(
        &self,
        physical_slot: u8,
        physical_button: u8,
        state: ElementState,
    ) -> impl Iterator<Item = (usize, Option<ControlEvent>)> + '_ {
        let physical_key = legacy_gamepad_button_key(physical_slot, physical_button);
        self.keys
            .iter()
            .enumerate()
            .flat_map(move |(gamepad_set, keys)| {
                keys.iter()
                    .enumerate()
                    .filter_map(move |(control_index, configured_key)| {
                        if physical_key.is_none() || *configured_key != physical_key {
                            return None;
                        }
                        let binding = CONTROL_BINDING_SPECS[control_index].binding;
                        let event = match state {
                            ElementState::Pressed => Some(binding.press_event()),
                            ElementState::Released => binding.release_event(),
                        };
                        Some((GAMEPAD_CONTROL_SET_OFFSET + gamepad_set, event))
                    })
            })
    }
}

impl Default for GamepadBindings {
    fn default() -> Self {
        Self {
            keys: [[None; CONTROL_BINDING_COUNT]; GAMEPAD_SET_COUNT],
        }
    }
}

const CONTROL_BINDING_SPECS: &[ControlBindingSpec] = &[
    ControlBindingSpec::new(
        ControlBindingId::CursorLeft,
        0,
        Binding::command(ControlCommand::CursorLeft, true),
    ),
    ControlBindingSpec::new(
        ControlBindingId::CursorToggle,
        1,
        Binding::command(ControlCommand::CursorToggle, true),
    ),
    ControlBindingSpec::new(
        ControlBindingId::CursorRight,
        2,
        Binding::command(ControlCommand::CursorRight, true),
    ),
    ControlBindingSpec::new(
        ControlBindingId::Throw,
        3,
        Binding::command(ControlCommand::Throw, true),
    ),
    ControlBindingSpec::new(
        ControlBindingId::Up,
        4,
        Binding::button(ControlButton::Up),
    ),
    ControlBindingSpec::new(
        ControlBindingId::Dig,
        5,
        Binding::command(ControlCommand::Dig, true),
    ),
    ControlBindingSpec::new(
        ControlBindingId::Left,
        6,
        Binding::button(ControlButton::Left),
    ),
    ControlBindingSpec::new(
        ControlBindingId::Down,
        7,
        Binding::button(ControlButton::Down),
    ),
    ControlBindingSpec::new(
        ControlBindingId::Right,
        8,
        Binding::button(ControlButton::Right),
    ),
    ControlBindingSpec::new(
        ControlBindingId::PlayerMenu,
        9,
        Binding::command(ControlCommand::PlayerMenu, false),
    ),
    ControlBindingSpec::new(
        ControlBindingId::Special,
        10,
        Binding::command(ControlCommand::Special, true),
    ),
    ControlBindingSpec::new(
        ControlBindingId::Special2,
        11,
        Binding::command(ControlCommand::Special2, true),
    ),
];

fn cpp_default_keyboard_keys(
    german_system: bool,
) -> [[VirtualKeyCode; CONTROL_BINDING_COUNT]; KEYBOARD_SET_COUNT] {
    let raw = cpp_default_raw_keyboard_keys(german_system);
    std::array::from_fn(|set_index| {
        std::array::from_fn(|control_index| {
            decode_platform_key_code(raw[set_index][control_index])
                .expect("the active C++ platform codec supports every default key")
        })
    })
}

// C4ConfigControls::CompileFunc selects exactly one KEY(win, x11, sdl)
// branch and initializes all 48 values before reading `[Controls]`
// overrides (pristine 9ffa0a5d src/C4Config.cpp:320-382).
#[cfg(target_os = "windows")]
fn cpp_default_raw_keyboard_keys(
    german_system: bool,
) -> [[i32; CONTROL_BINDING_COUNT]; KEYBOARD_SET_COUNT] {
    let mut keys = [
        [81, 87, 69, 65, 83, 68, 90, 88, 67, 82, 86, 70],
        [103, 104, 105, 100, 101, 102, 97, 98, 99, 96, 110, 107],
        [73, 79, 80, 75, 76, 186, 188, 190, 191, 77, 222, 186],
        [45, 36, 33, 46, 38, 34, 37, 40, 39, 35, 13, 8],
    ];
    if german_system {
        keys[0][6] = 89;
        keys[0][9] = 226;
        keys[2][5] = 192;
        keys[2][8] = 189;
    }
    keys
}

#[cfg(target_os = "linux")]
fn cpp_default_raw_keyboard_keys(
    german_system: bool,
) -> [[i32; CONTROL_BINDING_COUNT]; KEYBOARD_SET_COUNT] {
    let mut keys = [
        [113, 119, 101, 97, 115, 100, 122, 120, 99, 114, 118, 102],
        [
            0xff95, 0xff97, 0xff9a, 0xff96, 0xff9d, 0xff98, 0xff9c, 0xff99, 0xff9b,
            0xff9e, 0xff9f, 0xffab,
        ],
        [105, 111, 112, 107, 108, 59, 44, 46, 47, 109, 228, 252],
        [
            0xff63, 0xff50, 0xff55, 0xffff, 0xff52, 0xff56, 0xff51, 0xff54, 0xff53,
            0xff57, 0xff0d, 0xff08,
        ],
    ];
    if german_system {
        keys[0][6] = 121;
        keys[0][9] = 60;
        keys[2][5] = 246;
        keys[2][8] = 45;
    }
    keys
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn cpp_default_raw_keyboard_keys(
    german_system: bool,
) -> [[i32; CONTROL_BINDING_COUNT]; KEYBOARD_SET_COUNT] {
    let mut keys = [
        [20, 26, 8, 4, 22, 7, 29, 27, 6, 21, 25, 9],
        [95, 96, 97, 92, 93, 94, 89, 90, 91, 98, 99, 87],
        [12, 18, 19, 14, 15, 51, 54, 55, 56, 16, 52, 47],
        [73, 74, 75, 76, 82, 78, 80, 81, 79, 77, 40, 42],
    ];
    if german_system {
        // SDL scancodes are physical: only the extra ISO key differs.
        keys[0][9] = 100;
    }
    keys
}

fn is_german_system() -> bool {
    // The C++ branches use the OS language (C4Config.cpp:46-58). Without an
    // additional native dependency, the Rust launcher can reproduce the Unix
    // locale branch and common Windows/macOS launcher environments.
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .find_map(|name| std::env::var(name).ok().filter(|value| !value.is_empty()))
        .is_some_and(|value| value.to_ascii_lowercase().contains("de"))
}

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
    decode_platform_key_code(parse_raw_key_code_value(raw)?)
}

fn parse_raw_key_code_value(raw: &str) -> Option<i32> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(hex) = trimmed.strip_prefix("0x") {
        i32::from_str_radix(hex, 16).ok()
    } else if let Some(hex) = trimmed.strip_prefix("0X") {
        i32::from_str_radix(hex, 16).ok()
    } else {
        trimmed.parse::<i32>().ok()
    }
}

fn legacy_gamepad_button_key(physical_slot: u8, physical_button: u8) -> Option<i32> {
    if physical_slot >= GAMEPAD_SET_COUNT as u8
        || physical_button >= LEGACY_GAMEPAD_BUTTON_COUNT
    {
        return None;
    }
    Some(
        LEGACY_GAMEPAD_KEY_PREFIX
            + (i32::from(physical_slot) << 8)
            + i32::from(LEGACY_GAMEPAD_BUTTON_OFFSET + physical_button),
    )
}

fn letter_from_offset(offset: i32) -> Option<VirtualKeyCode> {
    Some(match offset {
        0 => VirtualKeyCode::A,
        1 => VirtualKeyCode::B,
        2 => VirtualKeyCode::C,
        3 => VirtualKeyCode::D,
        4 => VirtualKeyCode::E,
        5 => VirtualKeyCode::F,
        6 => VirtualKeyCode::G,
        7 => VirtualKeyCode::H,
        8 => VirtualKeyCode::I,
        9 => VirtualKeyCode::J,
        10 => VirtualKeyCode::K,
        11 => VirtualKeyCode::L,
        12 => VirtualKeyCode::M,
        13 => VirtualKeyCode::N,
        14 => VirtualKeyCode::O,
        15 => VirtualKeyCode::P,
        16 => VirtualKeyCode::Q,
        17 => VirtualKeyCode::R,
        18 => VirtualKeyCode::S,
        19 => VirtualKeyCode::T,
        20 => VirtualKeyCode::U,
        21 => VirtualKeyCode::V,
        22 => VirtualKeyCode::W,
        23 => VirtualKeyCode::X,
        24 => VirtualKeyCode::Y,
        25 => VirtualKeyCode::Z,
        _ => return None,
    })
}

fn digit_key(value: i32) -> Option<VirtualKeyCode> {
    Some(match value {
        0 => VirtualKeyCode::Key0,
        1 => VirtualKeyCode::Key1,
        2 => VirtualKeyCode::Key2,
        3 => VirtualKeyCode::Key3,
        4 => VirtualKeyCode::Key4,
        5 => VirtualKeyCode::Key5,
        6 => VirtualKeyCode::Key6,
        7 => VirtualKeyCode::Key7,
        8 => VirtualKeyCode::Key8,
        9 => VirtualKeyCode::Key9,
        _ => return None,
    })
}

fn numpad_key(value: i32) -> Option<VirtualKeyCode> {
    Some(match value {
        0 => VirtualKeyCode::Numpad0,
        1 => VirtualKeyCode::Numpad1,
        2 => VirtualKeyCode::Numpad2,
        3 => VirtualKeyCode::Numpad3,
        4 => VirtualKeyCode::Numpad4,
        5 => VirtualKeyCode::Numpad5,
        6 => VirtualKeyCode::Numpad6,
        7 => VirtualKeyCode::Numpad7,
        8 => VirtualKeyCode::Numpad8,
        9 => VirtualKeyCode::Numpad9,
        _ => return None,
    })
}

fn letter_offset(key: VirtualKeyCode) -> Option<i32> {
    Some(match key {
        VirtualKeyCode::A => 0,
        VirtualKeyCode::B => 1,
        VirtualKeyCode::C => 2,
        VirtualKeyCode::D => 3,
        VirtualKeyCode::E => 4,
        VirtualKeyCode::F => 5,
        VirtualKeyCode::G => 6,
        VirtualKeyCode::H => 7,
        VirtualKeyCode::I => 8,
        VirtualKeyCode::J => 9,
        VirtualKeyCode::K => 10,
        VirtualKeyCode::L => 11,
        VirtualKeyCode::M => 12,
        VirtualKeyCode::N => 13,
        VirtualKeyCode::O => 14,
        VirtualKeyCode::P => 15,
        VirtualKeyCode::Q => 16,
        VirtualKeyCode::R => 17,
        VirtualKeyCode::S => 18,
        VirtualKeyCode::T => 19,
        VirtualKeyCode::U => 20,
        VirtualKeyCode::V => 21,
        VirtualKeyCode::W => 22,
        VirtualKeyCode::X => 23,
        VirtualKeyCode::Y => 24,
        VirtualKeyCode::Z => 25,
        _ => return None,
    })
}

fn digit_value(key: VirtualKeyCode) -> Option<i32> {
    Some(match key {
        VirtualKeyCode::Key0 => 0,
        VirtualKeyCode::Key1 => 1,
        VirtualKeyCode::Key2 => 2,
        VirtualKeyCode::Key3 => 3,
        VirtualKeyCode::Key4 => 4,
        VirtualKeyCode::Key5 => 5,
        VirtualKeyCode::Key6 => 6,
        VirtualKeyCode::Key7 => 7,
        VirtualKeyCode::Key8 => 8,
        VirtualKeyCode::Key9 => 9,
        _ => return None,
    })
}

fn numpad_value(key: VirtualKeyCode) -> Option<i32> {
    Some(match key {
        VirtualKeyCode::Numpad0 => 0,
        VirtualKeyCode::Numpad1 => 1,
        VirtualKeyCode::Numpad2 => 2,
        VirtualKeyCode::Numpad3 => 3,
        VirtualKeyCode::Numpad4 => 4,
        VirtualKeyCode::Numpad5 => 5,
        VirtualKeyCode::Numpad6 => 6,
        VirtualKeyCode::Numpad7 => 7,
        VirtualKeyCode::Numpad8 => 8,
        VirtualKeyCode::Numpad9 => 9,
        _ => return None,
    })
}

#[cfg(target_os = "windows")]
fn decode_platform_key_code(value: i32) -> Option<VirtualKeyCode> {
    match value {
        0x70 => Some(VirtualKeyCode::F1),
        value @ 65..=90 => letter_from_offset(value - 65),
        value @ 48..=57 => digit_key(value - 48),
        value @ 96..=105 => numpad_key(value - 96),
        8 => Some(VirtualKeyCode::Back),
        13 => Some(VirtualKeyCode::Return),
        32 => Some(VirtualKeyCode::Space),
        33 => Some(VirtualKeyCode::PageUp),
        34 => Some(VirtualKeyCode::PageDown),
        35 => Some(VirtualKeyCode::End),
        36 => Some(VirtualKeyCode::Home),
        37 => Some(VirtualKeyCode::Left),
        38 => Some(VirtualKeyCode::Up),
        39 => Some(VirtualKeyCode::Right),
        40 => Some(VirtualKeyCode::Down),
        45 => Some(VirtualKeyCode::Insert),
        46 => Some(VirtualKeyCode::Delete),
        107 => Some(VirtualKeyCode::NumpadAdd),
        110 => Some(VirtualKeyCode::NumpadDecimal),
        186 => Some(VirtualKeyCode::Semicolon),
        188 => Some(VirtualKeyCode::Comma),
        189 => Some(VirtualKeyCode::Minus),
        190 => Some(VirtualKeyCode::Period),
        191 => Some(VirtualKeyCode::Slash),
        192 => Some(VirtualKeyCode::Grave),
        219 => Some(VirtualKeyCode::LBracket),
        220 => Some(VirtualKeyCode::Backslash),
        221 => Some(VirtualKeyCode::RBracket),
        222 => Some(VirtualKeyCode::Apostrophe),
        226 => Some(VirtualKeyCode::OEM102),
        _ => None,
    }
}

#[cfg(target_os = "linux")]
fn decode_platform_key_code(value: i32) -> Option<VirtualKeyCode> {
    match value {
        0xffbe => Some(VirtualKeyCode::F1),
        value @ 97..=122 => letter_from_offset(value - 97),
        value @ 65..=90 => letter_from_offset(value - 65),
        value @ 48..=57 => digit_key(value - 48),
        value @ 0xffb0..=0xffb9 => numpad_key(value - 0xffb0),
        0x20 => Some(VirtualKeyCode::Space),
        0x27 => Some(VirtualKeyCode::Apostrophe),
        0x2c => Some(VirtualKeyCode::Comma),
        0x2d => Some(VirtualKeyCode::Minus),
        0x2e => Some(VirtualKeyCode::Period),
        0x2f => Some(VirtualKeyCode::Slash),
        0x3b => Some(VirtualKeyCode::Semicolon),
        0x3c => Some(VirtualKeyCode::OEM102),
        0x3d => Some(VirtualKeyCode::Equals),
        0x5b => Some(VirtualKeyCode::LBracket),
        0x5c => Some(VirtualKeyCode::Backslash),
        0x5d => Some(VirtualKeyCode::RBracket),
        0x60 => Some(VirtualKeyCode::Grave),
        0xe4 => Some(VirtualKeyCode::Apostrophe),
        0xf6 => Some(VirtualKeyCode::Semicolon),
        0xfc => Some(VirtualKeyCode::LBracket),
        0xff08 => Some(VirtualKeyCode::Back),
        0xff0d => Some(VirtualKeyCode::Return),
        0xff50 => Some(VirtualKeyCode::Home),
        0xff51 => Some(VirtualKeyCode::Left),
        0xff52 => Some(VirtualKeyCode::Up),
        0xff53 => Some(VirtualKeyCode::Right),
        0xff54 => Some(VirtualKeyCode::Down),
        0xff55 => Some(VirtualKeyCode::PageUp),
        0xff56 => Some(VirtualKeyCode::PageDown),
        0xff57 => Some(VirtualKeyCode::End),
        0xff95 => Some(VirtualKeyCode::Numpad7),
        0xff96 => Some(VirtualKeyCode::Numpad4),
        0xff97 => Some(VirtualKeyCode::Numpad8),
        0xff98 => Some(VirtualKeyCode::Numpad6),
        0xff99 => Some(VirtualKeyCode::Numpad2),
        0xff9a => Some(VirtualKeyCode::Numpad9),
        0xff9b => Some(VirtualKeyCode::Numpad3),
        0xff9c => Some(VirtualKeyCode::Numpad1),
        0xff9d => Some(VirtualKeyCode::Numpad5),
        0xff9e => Some(VirtualKeyCode::Numpad0),
        0xff9f => Some(VirtualKeyCode::NumpadDecimal),
        0xff63 => Some(VirtualKeyCode::Insert),
        0xffff => Some(VirtualKeyCode::Delete),
        0xff8d => Some(VirtualKeyCode::NumpadEnter),
        0xffab => Some(VirtualKeyCode::NumpadAdd),
        0xffae => Some(VirtualKeyCode::NumpadDecimal),
        _ => None,
    }
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn decode_platform_key_code(value: i32) -> Option<VirtualKeyCode> {
    match value {
        58 => Some(VirtualKeyCode::F1),
        value @ 4..=29 => letter_from_offset(value - 4),
        value @ 30..=38 => digit_key(value - 29),
        39 => Some(VirtualKeyCode::Key0),
        40 => Some(VirtualKeyCode::Return),
        42 => Some(VirtualKeyCode::Back),
        44 => Some(VirtualKeyCode::Space),
        45 => Some(VirtualKeyCode::Minus),
        46 => Some(VirtualKeyCode::Equals),
        47 => Some(VirtualKeyCode::LBracket),
        48 => Some(VirtualKeyCode::RBracket),
        49 => Some(VirtualKeyCode::Backslash),
        50 | 100 => Some(VirtualKeyCode::OEM102),
        51 => Some(VirtualKeyCode::Semicolon),
        52 => Some(VirtualKeyCode::Apostrophe),
        53 => Some(VirtualKeyCode::Grave),
        54 => Some(VirtualKeyCode::Comma),
        55 => Some(VirtualKeyCode::Period),
        56 => Some(VirtualKeyCode::Slash),
        73 => Some(VirtualKeyCode::Insert),
        74 => Some(VirtualKeyCode::Home),
        75 => Some(VirtualKeyCode::PageUp),
        76 => Some(VirtualKeyCode::Delete),
        77 => Some(VirtualKeyCode::End),
        78 => Some(VirtualKeyCode::PageDown),
        79 => Some(VirtualKeyCode::Right),
        80 => Some(VirtualKeyCode::Left),
        81 => Some(VirtualKeyCode::Down),
        82 => Some(VirtualKeyCode::Up),
        87 => Some(VirtualKeyCode::NumpadAdd),
        88 => Some(VirtualKeyCode::NumpadEnter),
        value @ 89..=97 => numpad_key(value - 88),
        98 => Some(VirtualKeyCode::Numpad0),
        99 => Some(VirtualKeyCode::NumpadDecimal),
        _ => None,
    }
}

#[cfg(target_os = "windows")]
fn encode_virtual_key_code(key: VirtualKeyCode) -> Option<i32> {
    if let Some(offset) = letter_offset(key) {
        return Some(65 + offset);
    }
    if let Some(value) = digit_value(key) {
        return Some(48 + value);
    }
    if let Some(value) = numpad_value(key) {
        return Some(96 + value);
    }
    Some(match key {
        VirtualKeyCode::F1 => 0x70,
        VirtualKeyCode::Back => 8,
        VirtualKeyCode::Return => 13,
        VirtualKeyCode::Space => 32,
        VirtualKeyCode::PageUp => 33,
        VirtualKeyCode::PageDown => 34,
        VirtualKeyCode::End => 35,
        VirtualKeyCode::Home => 36,
        VirtualKeyCode::Left => 37,
        VirtualKeyCode::Up => 38,
        VirtualKeyCode::Right => 39,
        VirtualKeyCode::Down => 40,
        VirtualKeyCode::Insert => 45,
        VirtualKeyCode::Delete => 46,
        VirtualKeyCode::NumpadAdd => 107,
        VirtualKeyCode::NumpadDecimal => 110,
        VirtualKeyCode::Semicolon => 186,
        VirtualKeyCode::Comma => 188,
        VirtualKeyCode::Minus => 189,
        VirtualKeyCode::Period => 190,
        VirtualKeyCode::Slash => 191,
        VirtualKeyCode::Grave => 192,
        VirtualKeyCode::LBracket => 219,
        VirtualKeyCode::Backslash => 220,
        VirtualKeyCode::RBracket => 221,
        VirtualKeyCode::Apostrophe => 222,
        VirtualKeyCode::OEM102 => 226,
        _ => return None,
    })
}

#[cfg(target_os = "linux")]
fn encode_virtual_key_code(key: VirtualKeyCode) -> Option<i32> {
    if let Some(offset) = letter_offset(key) {
        return Some(97 + offset);
    }
    if let Some(value) = digit_value(key) {
        return Some(48 + value);
    }
    if let Some(value) = numpad_value(key) {
        return Some(0xffb0 + value);
    }
    Some(match key {
        VirtualKeyCode::F1 => 0xffbe,
        VirtualKeyCode::Space => 0x20,
        VirtualKeyCode::Apostrophe => 0x27,
        VirtualKeyCode::Comma => 0x2c,
        VirtualKeyCode::Minus => 0x2d,
        VirtualKeyCode::Period => 0x2e,
        VirtualKeyCode::Slash => 0x2f,
        VirtualKeyCode::Semicolon => 0x3b,
        VirtualKeyCode::OEM102 => 0x3c,
        VirtualKeyCode::Equals => 0x3d,
        VirtualKeyCode::LBracket => 0x5b,
        VirtualKeyCode::Backslash => 0x5c,
        VirtualKeyCode::RBracket => 0x5d,
        VirtualKeyCode::Grave => 0x60,
        VirtualKeyCode::Back => 0xff08,
        VirtualKeyCode::Return => 0xff0d,
        VirtualKeyCode::Home => 0xff50,
        VirtualKeyCode::Left => 0xff51,
        VirtualKeyCode::Up => 0xff52,
        VirtualKeyCode::Right => 0xff53,
        VirtualKeyCode::Down => 0xff54,
        VirtualKeyCode::PageUp => 0xff55,
        VirtualKeyCode::PageDown => 0xff56,
        VirtualKeyCode::End => 0xff57,
        VirtualKeyCode::Insert => 0xff63,
        VirtualKeyCode::Delete => 0xffff,
        VirtualKeyCode::NumpadEnter => 0xff8d,
        VirtualKeyCode::NumpadAdd => 0xffab,
        VirtualKeyCode::NumpadDecimal => 0xffae,
        _ => return None,
    })
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn encode_virtual_key_code(key: VirtualKeyCode) -> Option<i32> {
    if let Some(offset) = letter_offset(key) {
        return Some(4 + offset);
    }
    if let Some(value) = digit_value(key) {
        return Some(if value == 0 { 39 } else { 29 + value });
    }
    if let Some(value) = numpad_value(key) {
        return Some(if value == 0 { 98 } else { 88 + value });
    }
    Some(match key {
        VirtualKeyCode::F1 => 58,
        VirtualKeyCode::Return => 40,
        VirtualKeyCode::Back => 42,
        VirtualKeyCode::Space => 44,
        VirtualKeyCode::Minus => 45,
        VirtualKeyCode::Equals => 46,
        VirtualKeyCode::LBracket => 47,
        VirtualKeyCode::RBracket => 48,
        VirtualKeyCode::Backslash => 49,
        VirtualKeyCode::Semicolon => 51,
        VirtualKeyCode::Apostrophe => 52,
        VirtualKeyCode::Grave => 53,
        VirtualKeyCode::Comma => 54,
        VirtualKeyCode::Period => 55,
        VirtualKeyCode::Slash => 56,
        VirtualKeyCode::Insert => 73,
        VirtualKeyCode::Home => 74,
        VirtualKeyCode::PageUp => 75,
        VirtualKeyCode::Delete => 76,
        VirtualKeyCode::End => 77,
        VirtualKeyCode::PageDown => 78,
        VirtualKeyCode::Right => 79,
        VirtualKeyCode::Left => 80,
        VirtualKeyCode::Down => 81,
        VirtualKeyCode::Up => 82,
        VirtualKeyCode::NumpadAdd => 87,
        VirtualKeyCode::NumpadEnter => 88,
        VirtualKeyCode::NumpadDecimal => 99,
        VirtualKeyCode::OEM102 => 100,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control_options::format_key_label;

    #[test]
    fn default_player_bindings_do_not_turn_space_into_clear_pressed() {
        // Space belongs to FullscreenMenuOpen/MenuOK by scope; the player
        // control registrations contain only the configured 48 callbacks and
        // never synthesize COM_ClearPressedComs (pristine 9ffa0a5d
        // src/C4Game.cpp:3388-3437; src/C4PlayerList.cpp:588-594).
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
            None
        );
    }

    #[test]
    fn default_player_one_movement_matches_cpp_keyboard_set_one() {
        // C++ parity: pristine 9ffa0a5d src/C4Config.cpp:332-343 defines the
        // complete keyboard set 1; movement is S/Z/X/C and does not include
        // arrow-key aliases.
        let bindings = KeyboardBindings::default_bindings();
        for (id, key, button) in [
            (ControlBindingId::Up, VirtualKeyCode::S, ControlButton::Up),
            (
                ControlBindingId::Left,
                VirtualKeyCode::Z,
                ControlButton::Left,
            ),
            (
                ControlBindingId::Down,
                VirtualKeyCode::X,
                ControlButton::Down,
            ),
            (
                ControlBindingId::Right,
                VirtualKeyCode::C,
                ControlButton::Right,
            ),
        ] {
            assert_eq!(bindings.key_for(id), Some(key));
            assert_eq!(
                bindings.event_for_key(key, ElementState::Pressed),
                Some(ControlEvent::Press(button))
            );
        }

        for key in [
            VirtualKeyCode::Up,
            VirtualKeyCode::Left,
            VirtualKeyCode::Down,
            VirtualKeyCode::Right,
        ] {
            assert_eq!(bindings.event_for_key(key, ElementState::Pressed), None);
        }
    }

    #[test]
    fn default_tutorial_guide_labels_follow_cpp_control_order() {
        // C4Viewport::DrawPlayerControls indexes the first ten CON_* slots
        // (C4Viewport.cpp:1394-1441). The Rust overlay follows this exact
        // ControlBindingId::ALL order before formatting each configured key.
        let bindings = KeyboardBindings::default_bindings();
        let labels: Vec<_> = ControlBindingId::ALL
            .iter()
            .take(10)
            .map(|binding| {
                bindings
                    .key_for(*binding)
                    .map(format_key_label)
                    .unwrap_or_default()
            })
            .collect();
        let expected: Vec<_> = ["Q", "W", "E", "A", "S", "D", "Z", "X", "C", "R"]
            .into_iter()
            .map(str::to_string)
            .collect();
        assert_eq!(labels, expected);

        let movement: Vec<_> = [
            ControlBindingId::Left,
            ControlBindingId::Up,
            ControlBindingId::Down,
            ControlBindingId::Right,
        ]
        .into_iter()
        .map(|binding| {
            format_key_label(
                bindings
                    .key_for(binding)
                    .expect("default movement binding"),
            )
        })
        .collect();
        assert_eq!(movement, ["Z", "S", "X", "C"]);
        assert!(labels.iter().all(|label| !label.contains("Arrow")));

        let fallback_movement: Vec<_> = [
            ControlBindingId::Left,
            ControlBindingId::Up,
            ControlBindingId::Down,
            ControlBindingId::Right,
        ]
        .into_iter()
        .map(|binding| format_key_label(binding.default_key()))
        .collect();
        assert_eq!(fallback_movement, movement);
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
    fn missing_gamepad_entries_register_no_gameplay_candidates() {
        // All twelve C4ConfigGamepad entries default to -1, and registration
        // skips -1 rather than inventing a controller layout (pristine
        // 9ffa0a5d src/C4Config.cpp:287-317; src/C4Game.cpp:3439-3452).
        let bindings = GamepadBindings::from_config(&Config::new());
        assert_eq!(
            bindings
                .control_candidates_for_button(1, 0, ElementState::Pressed)
                .collect::<Vec<_>>(),
            Vec::new()
        );
    }

    #[test]
    fn gamepad1_button10_full_keycode_maps_to_set_five_player_menu() {
        // Button10 is logical PlayerMenu, while 0x0042010a encodes physical
        // slot 1/raw button 0. C++ registers that exact key for control set 5
        // (pristine 9ffa0a5d src/C4KeyboardInput.h:57-80;
        // src/C4Game.cpp:3439-3452; src/C4ObjectCom.cpp:874-900).
        assert_eq!(legacy_gamepad_button_key(1, 0), Some(4_325_642));
        let mut config = Config::new();
        config.set_in(Some("Gamepad1"), "Button10", "4325642");
        let bindings = GamepadBindings::from_config(&config);

        assert_eq!(
            bindings
                .control_candidates_for_button(1, 0, ElementState::Pressed)
                .collect::<Vec<_>>(),
            vec![(
                5,
                Some(ControlEvent::Command {
                    command: ControlCommand::PlayerMenu,
                    kind: CommandKind::Press,
                }),
            )]
        );
        assert_eq!(
            bindings
                .control_candidates_for_button(1, 0, ElementState::Released)
                .collect::<Vec<_>>(),
            vec![(5, None)]
        );
        assert!(bindings
            .control_candidates_for_button(0, 0, ElementState::Pressed)
            .next()
            .is_none());
    }

    #[test]
    fn config_overrides_replace_defaults() {
        let mut cfg = Config::new();
        for (name, key) in [
            ("Kbd1Key5", VirtualKeyCode::W),
            ("Kbd1Key7", VirtualKeyCode::A),
            ("Kbd1Key8", VirtualKeyCode::S),
            ("Kbd1Key9", VirtualKeyCode::D),
        ] {
            cfg.set_in(
                Some("Controls"),
                name,
                encode_virtual_key_code(key)
                    .expect("fixture key is supported")
                    .to_string(),
            );
        }
        let bindings = KeyboardBindings::from_config(&cfg).expect("overrides present");

        assert_eq!(
            bindings.event_for_key(VirtualKeyCode::W, ElementState::Pressed),
            Some(ControlEvent::Press(ControlButton::Up))
        );
        assert_eq!(
            bindings.event_for_key(VirtualKeyCode::A, ElementState::Released),
            Some(ControlEvent::Release(ControlButton::Left))
        );
        assert_eq!(
            bindings.key_for(ControlBindingId::Up),
            Some(VirtualKeyCode::W)
        );
        assert_eq!(
            bindings.key_for(ControlBindingId::Left),
            Some(VirtualKeyCode::A)
        );
        assert_eq!(
            bindings.key_for(ControlBindingId::Down),
            Some(VirtualKeyCode::S)
        );
        assert_eq!(
            bindings.key_for(ControlBindingId::Right),
            Some(VirtualKeyCode::D)
        );
        // Config overrides do not add a non-C++ Space fallback either.
        assert_eq!(
            bindings.event_for_key(VirtualKeyCode::Space, ElementState::Pressed),
            None
        );
    }

    #[test]
    fn configured_keyboard_sets_preserve_cpp_set_then_control_callback_order() {
        // InitKeyboard registers Kbd1Key1..Kbd4Key12 with the keyboard-set
        // loop outside the control-index loop. Equal key codes remain distinct
        // callbacks and execute in that insertion order (pristine 9ffa0a5d
        // src/C4Game.cpp:3425-3437; src/C4KeyboardInput.cpp:682-750).
        #[cfg(target_os = "windows")]
        let expected_defaults = [
            [
                VirtualKeyCode::Q,
                VirtualKeyCode::W,
                VirtualKeyCode::E,
                VirtualKeyCode::A,
                VirtualKeyCode::S,
                VirtualKeyCode::D,
                VirtualKeyCode::Z,
                VirtualKeyCode::X,
                VirtualKeyCode::C,
                VirtualKeyCode::R,
                VirtualKeyCode::V,
                VirtualKeyCode::F,
            ],
            [
                VirtualKeyCode::Numpad7,
                VirtualKeyCode::Numpad8,
                VirtualKeyCode::Numpad9,
                VirtualKeyCode::Numpad4,
                VirtualKeyCode::Numpad5,
                VirtualKeyCode::Numpad6,
                VirtualKeyCode::Numpad1,
                VirtualKeyCode::Numpad2,
                VirtualKeyCode::Numpad3,
                VirtualKeyCode::Numpad0,
                VirtualKeyCode::NumpadDecimal,
                VirtualKeyCode::NumpadAdd,
            ],
            [
                VirtualKeyCode::I,
                VirtualKeyCode::O,
                VirtualKeyCode::P,
                VirtualKeyCode::K,
                VirtualKeyCode::L,
                VirtualKeyCode::Semicolon,
                VirtualKeyCode::Comma,
                VirtualKeyCode::Period,
                VirtualKeyCode::Slash,
                VirtualKeyCode::M,
                VirtualKeyCode::Apostrophe,
                VirtualKeyCode::Semicolon,
            ],
            [
                VirtualKeyCode::Insert,
                VirtualKeyCode::Home,
                VirtualKeyCode::PageUp,
                VirtualKeyCode::Delete,
                VirtualKeyCode::Up,
                VirtualKeyCode::PageDown,
                VirtualKeyCode::Left,
                VirtualKeyCode::Down,
                VirtualKeyCode::Right,
                VirtualKeyCode::End,
                VirtualKeyCode::Return,
                VirtualKeyCode::Back,
            ],
        ];
        #[cfg(not(target_os = "windows"))]
        let expected_defaults = [
            [
                VirtualKeyCode::Q,
                VirtualKeyCode::W,
                VirtualKeyCode::E,
                VirtualKeyCode::A,
                VirtualKeyCode::S,
                VirtualKeyCode::D,
                VirtualKeyCode::Z,
                VirtualKeyCode::X,
                VirtualKeyCode::C,
                VirtualKeyCode::R,
                VirtualKeyCode::V,
                VirtualKeyCode::F,
            ],
            [
                VirtualKeyCode::Numpad7,
                VirtualKeyCode::Numpad8,
                VirtualKeyCode::Numpad9,
                VirtualKeyCode::Numpad4,
                VirtualKeyCode::Numpad5,
                VirtualKeyCode::Numpad6,
                VirtualKeyCode::Numpad1,
                VirtualKeyCode::Numpad2,
                VirtualKeyCode::Numpad3,
                VirtualKeyCode::Numpad0,
                VirtualKeyCode::NumpadDecimal,
                VirtualKeyCode::NumpadAdd,
            ],
            [
                VirtualKeyCode::I,
                VirtualKeyCode::O,
                VirtualKeyCode::P,
                VirtualKeyCode::K,
                VirtualKeyCode::L,
                VirtualKeyCode::Semicolon,
                VirtualKeyCode::Comma,
                VirtualKeyCode::Period,
                VirtualKeyCode::Slash,
                VirtualKeyCode::M,
                VirtualKeyCode::Apostrophe,
                VirtualKeyCode::LBracket,
            ],
            [
                VirtualKeyCode::Insert,
                VirtualKeyCode::Home,
                VirtualKeyCode::PageUp,
                VirtualKeyCode::Delete,
                VirtualKeyCode::Up,
                VirtualKeyCode::PageDown,
                VirtualKeyCode::Left,
                VirtualKeyCode::Down,
                VirtualKeyCode::Right,
                VirtualKeyCode::End,
                VirtualKeyCode::Return,
                VirtualKeyCode::Back,
            ],
        ];
        assert_eq!(cpp_default_keyboard_keys(false), expected_defaults);

        #[cfg(target_os = "windows")]
        let raw_g = "71"; // 'G'
        #[cfg(target_os = "linux")]
        let raw_g = "103"; // XK_g
        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        let raw_g = "10"; // SDL_SCANCODE_G
        assert_eq!(
            encode_virtual_key_code(VirtualKeyCode::G),
            Some(raw_g.parse().expect("fixture platform code"))
        );

        let mut config = Config::new();
        for name in [
            "Kbd1Key1",
            "Kbd1Key5",
            "Kbd2Key1",
            "Kbd3Key10",
            "Kbd4Key12",
        ] {
            config.set_in(Some("Controls"), name, raw_g);
        }
        let bindings = KeyboardBindings::from_config(&config).expect("configured bindings");

        assert_eq!(
            bindings
                .control_events_for_key(VirtualKeyCode::G, ElementState::Pressed)
                .collect::<Vec<_>>(),
            vec![
                (
                    0,
                    ControlEvent::Command {
                        command: ControlCommand::CursorLeft,
                        kind: CommandKind::Press,
                    },
                ),
                (0, ControlEvent::Press(ControlButton::Up)),
                (
                    1,
                    ControlEvent::Command {
                        command: ControlCommand::CursorLeft,
                        kind: CommandKind::Press,
                    },
                ),
                (
                    2,
                    ControlEvent::Command {
                        command: ControlCommand::PlayerMenu,
                        kind: CommandKind::Press,
                    },
                ),
                (
                    3,
                    ControlEvent::Command {
                        command: ControlCommand::Special2,
                        kind: CommandKind::Press,
                    },
                ),
            ]
        );
    }

    #[test]
    fn player_menu_release_candidate_is_retained_without_an_emitted_event() {
        // LocalControlKeyUp returns true for an active AutoStop player even
        // when Control2Com maps PlayerMenu release to COM_None. The callback
        // must therefore remain in collision order without an emitted event
        // (pristine 9ffa0a5d src/C4Game.cpp:3554-3567;
        // src/C4ObjectCom.cpp:874-899).
        #[cfg(target_os = "windows")]
        let raw_g = "71";
        #[cfg(target_os = "linux")]
        let raw_g = "103";
        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        let raw_g = "10";

        let mut config = Config::new();
        config.set_in(Some("Controls"), "Kbd1Key10", raw_g);
        config.set_in(Some("Controls"), "Kbd2Key1", raw_g);
        let bindings = KeyboardBindings::from_config(&config).expect("configured bindings");

        assert_eq!(
            bindings
                .control_candidates_for_key(VirtualKeyCode::G, ElementState::Released)
                .collect::<Vec<_>>(),
            vec![
                (0, None),
                (
                    1,
                    Some(ControlEvent::Command {
                        command: ControlCommand::CursorLeft,
                        kind: CommandKind::Release,
                    }),
                ),
            ]
        );
    }

    #[test]
    fn parse_supports_hex_values() {
        #[cfg(target_os = "windows")]
        let (a, b) = (0x41, 0x42);
        #[cfg(target_os = "linux")]
        let (a, b) = (0x61, 0x62);
        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        let (a, b) = (0x04, 0x05);

        assert_eq!(
            parse_key_code_value(&format!("0x{a:x}")),
            Some(VirtualKeyCode::A),
            "hex parsing should support the active C++ platform code"
        );
        assert_eq!(
            parse_key_code_value(&format!("0X{b:X}")),
            Some(VirtualKeyCode::B),
            "hex parsing should support alternate prefix"
        );
    }

    #[test]
    fn parse_supports_active_cpp_platform_codes() {
        #[cfg(target_os = "windows")]
        let (s, right) = (83, 39);
        #[cfg(target_os = "linux")]
        let (s, right) = (115, 0xff53);
        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        let (s, right) = (22, 79);

        assert_eq!(
            parse_key_code_value(&s.to_string()),
            Some(VirtualKeyCode::S),
            "active-platform code for S should parse"
        );
        assert_eq!(
            parse_key_code_value(&right.to_string()),
            Some(VirtualKeyCode::Right),
            "active-platform code for Right should parse"
        );
    }

    #[test]
    fn rebind_updates_and_resets() {
        let mut bindings = KeyboardBindings::default_bindings();
        assert_eq!(
            bindings.key_for(ControlBindingId::Throw),
            Some(VirtualKeyCode::A)
        );

        bindings.rebind(ControlBindingId::Throw, VirtualKeyCode::Key1);
        assert_eq!(
            bindings.key_for(ControlBindingId::Throw),
            Some(VirtualKeyCode::Key1)
        );
        assert_ne!(
            bindings.key_for(ControlBindingId::Throw),
            Some(VirtualKeyCode::A)
        );

        bindings.rebind(ControlBindingId::Right, VirtualKeyCode::Right);
        assert_eq!(
            bindings.key_for(ControlBindingId::Right),
            Some(VirtualKeyCode::Right)
        );

        bindings.reset_binding(ControlBindingId::Throw);
        assert_eq!(
            bindings.key_for(ControlBindingId::Throw),
            Some(ControlBindingId::Throw.default_key())
        );
    }

    #[test]
    fn supported_key_detection_matches_encoder() {
        assert!(KeyboardBindings::is_supported_key(VirtualKeyCode::Q));
        assert!(KeyboardBindings::is_supported_key(VirtualKeyCode::Space));
        assert!(!KeyboardBindings::is_supported_key(VirtualKeyCode::F1));
    }
}
