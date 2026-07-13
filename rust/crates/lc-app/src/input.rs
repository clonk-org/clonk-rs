use std::collections::{HashMap, HashSet};
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
        self.spec().default_key
    }

    fn binding(self) -> Binding {
        self.spec().binding
    }

    fn spec(self) -> &'static ControlBindingSpec {
        &CONTROL_BINDING_SPECS[self as usize]
    }
}

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
    id: ControlBindingId,
    index: usize,
    default_key: VirtualKeyCode,
    binding: Binding,
}

impl ControlBindingSpec {
    const fn new(
        id: ControlBindingId,
        index: usize,
        default_key: VirtualKeyCode,
        binding: Binding,
    ) -> Self {
        Self {
            id,
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
        let mut any_override = false;
        let mut configured_keys = Vec::with_capacity(CONTROL_BINDING_SPECS.len());

        for spec in CONTROL_BINDING_SPECS {
            let key = read_keyboard_entry(config, 0, spec.index)
                .inspect(|_| any_override = true)
                .unwrap_or(spec.default_key);
            configured_keys.push((spec.binding, key));
        }

        if !any_override {
            return None;
        }

        // The C++ keyboard dispatcher registers controls in ascending control-index order and
        // executes the first matching callback. Preserve that ordering when multiple configured
        // controls use the same physical key, including collisions with an untouched default.
        let mut bindings = HashMap::new();
        for (binding, key) in configured_keys {
            bindings.entry(key).or_insert(binding);
        }

        Some(Self {
            bindings,
            clear_keys: HashSet::from([VirtualKeyCode::Space]),
        })
    }

    fn default_bindings() -> Self {
        let mut bindings = HashMap::new();
        for spec in CONTROL_BINDING_SPECS {
            bindings.insert(spec.default_key, spec.binding);
        }

        let clear_keys = HashSet::from([VirtualKeyCode::Space]);

        Self {
            bindings,
            clear_keys,
        }
    }

    pub fn key_for(&self, id: ControlBindingId) -> Option<VirtualKeyCode> {
        let target = id.binding();
        self.bindings
            .iter()
            .find_map(|(key, binding)| if *binding == target { Some(*key) } else { None })
    }

    pub fn rebind(&mut self, id: ControlBindingId, key: VirtualKeyCode) {
        self.assign_binding(id.binding(), key);
    }

    pub fn reset_binding(&mut self, id: ControlBindingId) {
        let spec = id.spec();
        self.assign_binding(spec.binding, spec.default_key);
    }

    pub fn reset_all(&mut self) {
        for spec in CONTROL_BINDING_SPECS {
            self.assign_binding(spec.binding, spec.default_key);
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
            let keycode = self.key_for(spec.id).unwrap_or(spec.default_key);
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

    fn assign_binding(&mut self, binding: Binding, key: VirtualKeyCode) {
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
        ControlBindingId::CursorLeft,
        0,
        VirtualKeyCode::Q,
        Binding::command(ControlCommand::CursorLeft, true),
    ),
    ControlBindingSpec::new(
        ControlBindingId::CursorToggle,
        1,
        VirtualKeyCode::W,
        Binding::command(ControlCommand::CursorToggle, true),
    ),
    ControlBindingSpec::new(
        ControlBindingId::CursorRight,
        2,
        VirtualKeyCode::E,
        Binding::command(ControlCommand::CursorRight, true),
    ),
    ControlBindingSpec::new(
        ControlBindingId::Throw,
        3,
        VirtualKeyCode::A,
        Binding::command(ControlCommand::Throw, true),
    ),
    ControlBindingSpec::new(
        ControlBindingId::Up,
        4,
        VirtualKeyCode::S,
        Binding::button(ControlButton::Up),
    ),
    ControlBindingSpec::new(
        ControlBindingId::Dig,
        5,
        VirtualKeyCode::D,
        Binding::command(ControlCommand::Dig, true),
    ),
    ControlBindingSpec::new(
        ControlBindingId::Left,
        6,
        VirtualKeyCode::Z,
        Binding::button(ControlButton::Left),
    ),
    ControlBindingSpec::new(
        ControlBindingId::Down,
        7,
        VirtualKeyCode::X,
        Binding::button(ControlButton::Down),
    ),
    ControlBindingSpec::new(
        ControlBindingId::Right,
        8,
        VirtualKeyCode::C,
        Binding::button(ControlButton::Right),
    ),
    ControlBindingSpec::new(
        ControlBindingId::PlayerMenu,
        9,
        VirtualKeyCode::R,
        Binding::command(ControlCommand::PlayerMenu, false),
    ),
    ControlBindingSpec::new(
        ControlBindingId::Special,
        10,
        VirtualKeyCode::V,
        Binding::command(ControlCommand::Special, true),
    ),
    ControlBindingSpec::new(
        ControlBindingId::Special2,
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
    map_numeric_keycode_for_backend(value, active_classic_key_backend())
}

fn map_numeric_keycode_for_backend(
    value: i32,
    backend: ClassicKeyBackend,
) -> Option<VirtualKeyCode> {
    if let Some(function_key) = map_function_keycode_for_backend(value, backend) {
        return Some(function_key);
    }

    // Decode the active backend's native values first. Several Win32 virtual keys and SDL
    // scancodes share integers with ASCII or with one another, so cross-backend fallbacks must
    // never get priority over the selected backend.
    let native = match backend {
        ClassicKeyBackend::Win32 => map_win32_keycode(value),
        ClassicKeyBackend::X11 => map_x11_keycode(value),
        ClassicKeyBackend::Sdl => map_sdl_keycode(value),
    };
    if native.is_some() {
        return native;
    }

    // Retain permissive parsing for older configs copied between platforms, but only after no
    // supported backend-native value matched.
    map_ascii_keycode(value)
        .or_else(|| map_sdl_letter_scancode(value))
        .or_else(|| map_sdl_digit_scancode(value))
        .or_else(|| map_sdl_arrow_scancode(value))
        .or(match value {
            0x25 | 0xff51 => Some(VirtualKeyCode::Left),
            0x26 | 0xff52 => Some(VirtualKeyCode::Up),
            0x27 | 0xff53 => Some(VirtualKeyCode::Right),
            0x28 | 0xff54 => Some(VirtualKeyCode::Down),
            0x20 | 44 => Some(VirtualKeyCode::Space),
            _ => None,
        })
}

fn function_key(index: i32) -> Option<VirtualKeyCode> {
    match index {
        0 => Some(VirtualKeyCode::F1),
        1 => Some(VirtualKeyCode::F2),
        2 => Some(VirtualKeyCode::F3),
        3 => Some(VirtualKeyCode::F4),
        4 => Some(VirtualKeyCode::F5),
        5 => Some(VirtualKeyCode::F6),
        6 => Some(VirtualKeyCode::F7),
        7 => Some(VirtualKeyCode::F8),
        8 => Some(VirtualKeyCode::F9),
        9 => Some(VirtualKeyCode::F10),
        10 => Some(VirtualKeyCode::F11),
        11 => Some(VirtualKeyCode::F12),
        _ => None,
    }
}

fn function_key_index(key: VirtualKeyCode) -> Option<i32> {
    match key {
        VirtualKeyCode::F1 => Some(0),
        VirtualKeyCode::F2 => Some(1),
        VirtualKeyCode::F3 => Some(2),
        VirtualKeyCode::F4 => Some(3),
        VirtualKeyCode::F5 => Some(4),
        VirtualKeyCode::F6 => Some(5),
        VirtualKeyCode::F7 => Some(6),
        VirtualKeyCode::F8 => Some(7),
        VirtualKeyCode::F9 => Some(8),
        VirtualKeyCode::F10 => Some(9),
        VirtualKeyCode::F11 => Some(10),
        VirtualKeyCode::F12 => Some(11),
        _ => None,
    }
}

#[derive(Clone, Copy)]
enum ClassicKeyBackend {
    Win32,
    X11,
    Sdl,
}

fn active_classic_key_backend() -> ClassicKeyBackend {
    #[cfg(target_os = "windows")]
    {
        ClassicKeyBackend::Win32
    }
    #[cfg(target_os = "linux")]
    {
        ClassicKeyBackend::X11
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        ClassicKeyBackend::Sdl
    }
}

fn first_function_key_code(backend: ClassicKeyBackend) -> i32 {
    match backend {
        ClassicKeyBackend::Win32 => 0x70,
        ClassicKeyBackend::X11 => 0xffbe,
        ClassicKeyBackend::Sdl => 58,
    }
}

fn map_function_keycode_for_backend(
    value: i32,
    backend: ClassicKeyBackend,
) -> Option<VirtualKeyCode> {
    value
        .checked_sub(first_function_key_code(backend))
        .and_then(function_key)
}

fn encode_function_keycode_for_backend(
    key: VirtualKeyCode,
    backend: ClassicKeyBackend,
) -> Option<i32> {
    function_key_index(key).map(|index| first_function_key_code(backend) + index)
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

fn map_win32_keycode(value: i32) -> Option<VirtualKeyCode> {
    map_ascii_keycode(value).or(match value {
        0x25 => Some(VirtualKeyCode::Left),
        0x26 => Some(VirtualKeyCode::Up),
        0x27 => Some(VirtualKeyCode::Right),
        0x28 => Some(VirtualKeyCode::Down),
        _ => None,
    })
}

fn map_x11_keycode(value: i32) -> Option<VirtualKeyCode> {
    map_ascii_keycode(value).or(match value {
        0xff51 => Some(VirtualKeyCode::Left),
        0xff52 => Some(VirtualKeyCode::Up),
        0xff53 => Some(VirtualKeyCode::Right),
        0xff54 => Some(VirtualKeyCode::Down),
        _ => None,
    })
}

fn map_sdl_keycode(value: i32) -> Option<VirtualKeyCode> {
    map_sdl_letter_scancode(value)
        .or_else(|| map_sdl_digit_scancode(value))
        .or_else(|| map_sdl_arrow_scancode(value))
        .or_else(|| (value == 44).then_some(VirtualKeyCode::Space))
}

fn encode_virtual_key_code(key: VirtualKeyCode) -> Option<i32> {
    encode_virtual_key_code_for_backend(key, active_classic_key_backend())
}

fn encode_virtual_key_code_for_backend(
    key: VirtualKeyCode,
    backend: ClassicKeyBackend,
) -> Option<i32> {
    if let Some(function_keycode) = encode_function_keycode_for_backend(key, backend) {
        return Some(function_keycode);
    }
    match backend {
        ClassicKeyBackend::Win32 => encode_win32_keycode(key),
        ClassicKeyBackend::X11 => encode_x11_keycode(key),
        ClassicKeyBackend::Sdl => encode_sdl_keycode(key),
    }
}

fn encode_win32_keycode(key: VirtualKeyCode) -> Option<i32> {
    match key {
        VirtualKeyCode::A => Some('A' as i32),
        VirtualKeyCode::B => Some('B' as i32),
        VirtualKeyCode::C => Some('C' as i32),
        VirtualKeyCode::D => Some('D' as i32),
        VirtualKeyCode::E => Some('E' as i32),
        VirtualKeyCode::F => Some('F' as i32),
        VirtualKeyCode::G => Some('G' as i32),
        VirtualKeyCode::H => Some('H' as i32),
        VirtualKeyCode::I => Some('I' as i32),
        VirtualKeyCode::J => Some('J' as i32),
        VirtualKeyCode::K => Some('K' as i32),
        VirtualKeyCode::L => Some('L' as i32),
        VirtualKeyCode::M => Some('M' as i32),
        VirtualKeyCode::N => Some('N' as i32),
        VirtualKeyCode::O => Some('O' as i32),
        VirtualKeyCode::P => Some('P' as i32),
        VirtualKeyCode::Q => Some('Q' as i32),
        VirtualKeyCode::R => Some('R' as i32),
        VirtualKeyCode::S => Some('S' as i32),
        VirtualKeyCode::T => Some('T' as i32),
        VirtualKeyCode::U => Some('U' as i32),
        VirtualKeyCode::V => Some('V' as i32),
        VirtualKeyCode::W => Some('W' as i32),
        VirtualKeyCode::X => Some('X' as i32),
        VirtualKeyCode::Y => Some('Y' as i32),
        VirtualKeyCode::Z => Some('Z' as i32),
        VirtualKeyCode::Key0 => Some('0' as i32),
        VirtualKeyCode::Key1 => Some('1' as i32),
        VirtualKeyCode::Key2 => Some('2' as i32),
        VirtualKeyCode::Key3 => Some('3' as i32),
        VirtualKeyCode::Key4 => Some('4' as i32),
        VirtualKeyCode::Key5 => Some('5' as i32),
        VirtualKeyCode::Key6 => Some('6' as i32),
        VirtualKeyCode::Key7 => Some('7' as i32),
        VirtualKeyCode::Key8 => Some('8' as i32),
        VirtualKeyCode::Key9 => Some('9' as i32),
        VirtualKeyCode::Space => Some(0x20),
        VirtualKeyCode::Left => Some(0x25),
        VirtualKeyCode::Up => Some(0x26),
        VirtualKeyCode::Right => Some(0x27),
        VirtualKeyCode::Down => Some(0x28),
        _ => None,
    }
}

fn encode_x11_keycode(key: VirtualKeyCode) -> Option<i32> {
    match key {
        VirtualKeyCode::Left => Some(0xff51),
        VirtualKeyCode::Up => Some(0xff52),
        VirtualKeyCode::Right => Some(0xff53),
        VirtualKeyCode::Down => Some(0xff54),
        _ => encode_win32_keycode(key).map(|value| {
            if (b'A' as i32..=b'Z' as i32).contains(&value) {
                value + (b'a' - b'A') as i32
            } else {
                value
            }
        }),
    }
}

fn encode_sdl_keycode(key: VirtualKeyCode) -> Option<i32> {
    match key {
        VirtualKeyCode::A => Some(4),
        VirtualKeyCode::B => Some(5),
        VirtualKeyCode::C => Some(6),
        VirtualKeyCode::D => Some(7),
        VirtualKeyCode::E => Some(8),
        VirtualKeyCode::F => Some(9),
        VirtualKeyCode::G => Some(10),
        VirtualKeyCode::H => Some(11),
        VirtualKeyCode::I => Some(12),
        VirtualKeyCode::J => Some(13),
        VirtualKeyCode::K => Some(14),
        VirtualKeyCode::L => Some(15),
        VirtualKeyCode::M => Some(16),
        VirtualKeyCode::N => Some(17),
        VirtualKeyCode::O => Some(18),
        VirtualKeyCode::P => Some(19),
        VirtualKeyCode::Q => Some(20),
        VirtualKeyCode::R => Some(21),
        VirtualKeyCode::S => Some(22),
        VirtualKeyCode::T => Some(23),
        VirtualKeyCode::U => Some(24),
        VirtualKeyCode::V => Some(25),
        VirtualKeyCode::W => Some(26),
        VirtualKeyCode::X => Some(27),
        VirtualKeyCode::Y => Some(28),
        VirtualKeyCode::Z => Some(29),
        VirtualKeyCode::Key1 => Some(30),
        VirtualKeyCode::Key2 => Some(31),
        VirtualKeyCode::Key3 => Some(32),
        VirtualKeyCode::Key4 => Some(33),
        VirtualKeyCode::Key5 => Some(34),
        VirtualKeyCode::Key6 => Some(35),
        VirtualKeyCode::Key7 => Some(36),
        VirtualKeyCode::Key8 => Some(37),
        VirtualKeyCode::Key9 => Some(38),
        VirtualKeyCode::Key0 => Some(39),
        VirtualKeyCode::Space => Some(44),
        VirtualKeyCode::Right => Some(79),
        VirtualKeyCode::Left => Some(80),
        VirtualKeyCode::Down => Some(81),
        VirtualKeyCode::Up => Some(82),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control_options::format_key_label;

    const FUNCTION_KEYS: [VirtualKeyCode; 12] = [
        VirtualKeyCode::F1,
        VirtualKeyCode::F2,
        VirtualKeyCode::F3,
        VirtualKeyCode::F4,
        VirtualKeyCode::F5,
        VirtualKeyCode::F6,
        VirtualKeyCode::F7,
        VirtualKeyCode::F8,
        VirtualKeyCode::F9,
        VirtualKeyCode::F10,
        VirtualKeyCode::F11,
        VirtualKeyCode::F12,
    ];

    const NON_FUNCTION_KEYS: [VirtualKeyCode; 41] = [
        VirtualKeyCode::A,
        VirtualKeyCode::B,
        VirtualKeyCode::C,
        VirtualKeyCode::D,
        VirtualKeyCode::E,
        VirtualKeyCode::F,
        VirtualKeyCode::G,
        VirtualKeyCode::H,
        VirtualKeyCode::I,
        VirtualKeyCode::J,
        VirtualKeyCode::K,
        VirtualKeyCode::L,
        VirtualKeyCode::M,
        VirtualKeyCode::N,
        VirtualKeyCode::O,
        VirtualKeyCode::P,
        VirtualKeyCode::Q,
        VirtualKeyCode::R,
        VirtualKeyCode::S,
        VirtualKeyCode::T,
        VirtualKeyCode::U,
        VirtualKeyCode::V,
        VirtualKeyCode::W,
        VirtualKeyCode::X,
        VirtualKeyCode::Y,
        VirtualKeyCode::Z,
        VirtualKeyCode::Key0,
        VirtualKeyCode::Key1,
        VirtualKeyCode::Key2,
        VirtualKeyCode::Key3,
        VirtualKeyCode::Key4,
        VirtualKeyCode::Key5,
        VirtualKeyCode::Key6,
        VirtualKeyCode::Key7,
        VirtualKeyCode::Key8,
        VirtualKeyCode::Key9,
        VirtualKeyCode::Space,
        VirtualKeyCode::Left,
        VirtualKeyCode::Up,
        VirtualKeyCode::Right,
        VirtualKeyCode::Down,
    ];

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
    fn default_player_one_movement_matches_cpp_keyboard_set_one() {
        // C++ parity: C4Config.cpp:624-635 defines the complete keyboard set 1;
        // movement is S/Z/X/C and does not include arrow-key aliases.
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
            format_key_label(bindings.key_for(binding).expect("default movement binding"))
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
    fn config_overrides_replace_defaults() {
        let mut cfg = Config::new();
        // Move the earlier default W/A/D controls out of the way, as the C++ key-selection UI
        // does when assigning those physical keys to later control slots.
        for (name, key) in [
            ("Kbd1Key2", VirtualKeyCode::Key1),
            ("Kbd1Key4", VirtualKeyCode::Key2),
            ("Kbd1Key6", VirtualKeyCode::Key3),
            ("Kbd1Key5", VirtualKeyCode::W),
            ("Kbd1Key7", VirtualKeyCode::A),
            ("Kbd1Key8", VirtualKeyCode::S),
            ("Kbd1Key9", VirtualKeyCode::D),
        ] {
            let encoded = encode_virtual_key_code(key).expect("supported config override");
            cfg.set_in(Some("Controls"), name, encoded.to_string());
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
        // Falling back to default should still handle the clear command.
        assert_eq!(
            bindings.event_for_key(VirtualKeyCode::Space, ElementState::Pressed),
            Some(ControlEvent::ClearPressed)
        );
    }

    #[test]
    fn parse_supports_hex_values() {
        let encoded_j = encode_virtual_key_code(VirtualKeyCode::J).expect("J is supported");
        let encoded_k = encode_virtual_key_code(VirtualKeyCode::K).expect("K is supported");
        assert_eq!(
            parse_key_code_value(&format!("0x{encoded_j:X}")),
            Some(VirtualKeyCode::J),
            "hex parsing should support uppercase prefix"
        );
        assert_eq!(
            parse_key_code_value(&format!("0X{encoded_k:X}")),
            Some(VirtualKeyCode::K),
            "hex parsing should support alternate prefix"
        );
    }

    #[test]
    fn parse_supports_sdl_scancodes() {
        assert_eq!(
            map_numeric_keycode_for_backend(22, ClassicKeyBackend::Sdl),
            Some(VirtualKeyCode::S),
            "SDL scancode for S should parse"
        );
        assert_eq!(
            map_numeric_keycode_for_backend(79, ClassicKeyBackend::Sdl),
            Some(VirtualKeyCode::Right),
            "SDL scancode for Right should parse"
        );
    }

    #[test]
    fn platform_function_key_config_can_bind_physical_f1_through_f12() {
        for key in FUNCTION_KEYS {
            let encoded = encode_virtual_key_code(key)
                .expect("the active classic backend represents the function key");
            assert_eq!(map_numeric_keycode(encoded), Some(key));

            let mut config = Config::new();
            config.set_in(Some("Controls"), "Kbd1Key7", encoded.to_string());
            let bindings =
                KeyboardBindings::from_config(&config).expect("function-key override present");
            assert_eq!(bindings.key_for(ControlBindingId::Left), Some(key));
            assert_eq!(
                bindings.event_for_key(key, ElementState::Pressed),
                Some(ControlEvent::Press(ControlButton::Left))
            );
        }
    }

    #[test]
    fn every_classic_backend_round_trips_its_complete_function_key_range() {
        for (backend, first) in [
            (ClassicKeyBackend::Win32, 0x70),
            (ClassicKeyBackend::X11, 0xffbe),
            (ClassicKeyBackend::Sdl, 58),
        ] {
            for (index, key) in FUNCTION_KEYS.into_iter().enumerate() {
                let code = first + index as i32;
                assert_eq!(map_function_keycode_for_backend(code, backend), Some(key));
                assert_eq!(
                    encode_function_keycode_for_backend(key, backend),
                    Some(code)
                );
            }
            assert_eq!(map_function_keycode_for_backend(first - 1, backend), None);
            assert_eq!(map_function_keycode_for_backend(first + 12, backend), None);
        }

        // Win32 F1-F11 overlap lowercase ASCII code points, and SDL F8-F12 overlap uppercase
        // ASCII code points. Consuming their complete backend ranges prevents those values from
        // falling through as player-control letters.
        assert_eq!(map_ascii_keycode(0x71), Some(VirtualKeyCode::Q));
        assert_eq!(
            map_function_keycode_for_backend(0x71, ClassicKeyBackend::Win32),
            Some(VirtualKeyCode::F2)
        );
        assert_eq!(map_ascii_keycode(65), Some(VirtualKeyCode::A));
        assert_eq!(
            map_function_keycode_for_backend(65, ClassicKeyBackend::Sdl),
            Some(VirtualKeyCode::F8)
        );
        assert_eq!(map_numeric_keycode(i32::MIN), None);
        assert_eq!(map_numeric_keycode(i32::MAX), None);
    }

    #[test]
    fn every_classic_backend_round_trips_all_supported_non_function_keys() {
        for backend in [
            ClassicKeyBackend::Win32,
            ClassicKeyBackend::X11,
            ClassicKeyBackend::Sdl,
        ] {
            for key in NON_FUNCTION_KEYS {
                let encoded = encode_virtual_key_code_for_backend(key, backend)
                    .expect("supported key has a native backend encoding");
                assert_eq!(map_numeric_keycode_for_backend(encoded, backend), Some(key));
            }
        }

        assert_eq!(
            encode_virtual_key_code_for_backend(VirtualKeyCode::A, ClassicKeyBackend::Win32),
            Some('A' as i32)
        );
        assert_eq!(
            encode_virtual_key_code_for_backend(VirtualKeyCode::A, ClassicKeyBackend::X11),
            Some('a' as i32)
        );
        assert_eq!(
            encode_virtual_key_code_for_backend(VirtualKeyCode::A, ClassicKeyBackend::Sdl),
            Some(4)
        );
    }

    #[test]
    fn duplicate_configured_f3_uses_first_control_index() {
        let encoded_f3 = encode_virtual_key_code(VirtualKeyCode::F3)
            .expect("the active classic backend represents F3");
        let mut config = Config::new();
        config.set_in(Some("Controls"), "Kbd1Key1", encoded_f3.to_string());
        config.set_in(Some("Controls"), "Kbd1Key7", encoded_f3.to_string());

        let bindings = KeyboardBindings::from_config(&config).expect("F3 overrides present");
        assert_eq!(
            bindings.event_for_key(VirtualKeyCode::F3, ElementState::Pressed),
            Some(ControlEvent::Command {
                command: ControlCommand::CursorLeft,
                kind: CommandKind::Press,
            }),
            "Kbd1Key1 must win over the later Kbd1Key7 assignment"
        );
        assert_eq!(
            bindings.key_for(ControlBindingId::CursorLeft),
            Some(VirtualKeyCode::F3)
        );
    }

    #[test]
    fn configured_key_cannot_displace_an_earlier_default_control() {
        let mut config = Config::new();
        let encoded_q = encode_virtual_key_code(VirtualKeyCode::Q).expect("Q is supported");
        config.set_in(Some("Controls"), "Kbd1Key7", encoded_q.to_string());

        let bindings = KeyboardBindings::from_config(&config).expect("left override present");
        assert_eq!(
            bindings.event_for_key(VirtualKeyCode::Q, ElementState::Pressed),
            Some(ControlEvent::Command {
                command: ControlCommand::CursorLeft,
                kind: CommandKind::Press,
            }),
            "the untouched Kbd1Key1 default has the earlier C++ control index"
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
        assert!(KeyboardBindings::is_supported_key(VirtualKeyCode::F1));
        assert!(KeyboardBindings::is_supported_key(VirtualKeyCode::F3));
        assert!(KeyboardBindings::is_supported_key(VirtualKeyCode::F12));
        assert!(!KeyboardBindings::is_supported_key(VirtualKeyCode::F13));
    }
}
