use std::{fs, io::ErrorKind};

use clonk_core::std_config::Config;
use clonk_engine::{CommandKind, ControlButton, ControlCommand, ControlEvent};
use clonk_platform::AppPaths;
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
    axis_calibrations: GamepadAxisCalibrations,
    axis_calibration_dirty: bool,
}

const KEYBOARD_SET_COUNT: usize = 4;
const GAMEPAD_SET_COUNT: usize = 4;
const GAMEPAD_CALIBRATED_AXIS_COUNT: usize = 6;
const GAMEPAD_CONTROL_SET_OFFSET: usize = KEYBOARD_SET_COUNT;
const CONTROL_BINDING_COUNT: usize = 12;
const LEGACY_GAMEPAD_KEY_PREFIX: i32 = 0x0042_0000;
const LEGACY_GAMEPAD_BUTTON_OFFSET: u8 = 10;
const LEGACY_GAMEPAD_BUTTON_COUNT: u8 = 32;
const LEGACY_GAMEPAD_BUTTON_MAX: u8 =
    LEGACY_GAMEPAD_BUTTON_OFFSET + LEGACY_GAMEPAD_BUTTON_COUNT - 1;
const LEGACY_GAMEPAD_AXIS_OFFSET: u8 = 0x30;
const LEGACY_GAMEPAD_AXIS_MAX: u8 = 0x50;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct GamepadAxisCalibration {
    pub(crate) min: u32,
    pub(crate) max: u32,
    pub(crate) calibrated: bool,
}

impl GamepadAxisCalibration {
    pub(crate) const fn new(min: u32, max: u32, calibrated: bool) -> Self {
        Self {
            min,
            max,
            calibrated,
        }
    }
}

pub(crate) type GamepadAxisCalibrations =
    [[GamepadAxisCalibration; GAMEPAD_CALIBRATED_AXIS_COUNT]; GAMEPAD_SET_COUNT];

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
    pub const SET_COUNT: usize = KEYBOARD_SET_COUNT;

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

    pub fn key_for_set(&self, control_set: usize, id: ControlBindingId) -> Option<VirtualKeyCode> {
        self.keys.get(control_set).map(|keys| keys[id.spec().index])
    }

    pub fn default_key_for_set(control_set: usize, id: ControlBindingId) -> Option<VirtualKeyCode> {
        cpp_default_keyboard_keys(is_german_system())
            .get(control_set)
            .map(|keys| keys[id.spec().index])
    }

    pub fn rebind(&mut self, id: ControlBindingId, key: VirtualKeyCode) {
        self.assign_binding(0, id, key);
    }

    /// Rebind one logical control in one of the four C++ keyboard blocks.
    /// Returns `false` for an out-of-range block without mutating anything.
    pub fn rebind_for_set(
        &mut self,
        control_set: usize,
        id: ControlBindingId,
        key: VirtualKeyCode,
    ) -> bool {
        let Some(keys) = self.keys.get_mut(control_set) else {
            return false;
        };
        keys[id.spec().index] = key;
        true
    }

    pub fn reset_binding(&mut self, id: ControlBindingId) {
        self.assign_binding(0, id, id.default_key());
    }

    pub fn reset_binding_for_set(&mut self, control_set: usize, id: ControlBindingId) -> bool {
        let Some(default) = Self::default_key_for_set(control_set, id) else {
            return false;
        };
        self.rebind_for_set(control_set, id, default)
    }

    pub fn reset_all(&mut self) {
        self.keys = cpp_default_keyboard_keys(is_german_system());
    }

    pub fn is_supported_key(key: VirtualKeyCode) -> bool {
        encode_virtual_key_code(key).is_some()
    }

    /// Writes all four keyboard blocks into an already loaded config. This is
    /// used by the Options dialog so its other sheets can share one atomic
    /// load/modify/save pass on Back.
    pub fn write_to_config(&self, config: &mut Config) {
        for (set_index, keys) in self.keys.iter().enumerate() {
            for spec in CONTROL_BINDING_SPECS {
                let keycode = keys[spec.index];
                if let Some(encoded) = encode_virtual_key_code(keycode) {
                    let key_name = format!("Kbd{}Key{}", set_index + 1, spec.index + 1);
                    config.set_in(Some("Controls"), key_name, encoded.to_string());
                } else {
                    tracing::warn!(
                        ?keycode,
                        set = set_index + 1,
                        control = spec.index + 1,
                        "skipping persistence for unsupported virtual key code"
                    );
                }
            }
        }
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
        self.write_to_config(&mut config);

        if let Err(err) = config.save(&config_path) {
            tracing::warn!(
                error = %err,
                path = %config_path.display(),
                "failed to persist control bindings"
            );
        }
    }

    fn assign_binding(&mut self, control_set: usize, id: ControlBindingId, key: VirtualKeyCode) {
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

    /// Builds the callback candidate for one named `KbdNKeyM` registration.
    /// Runtime `KeyConfig.txt` overlays use this after replacing the physical
    /// code list without mutating the persisted `[Controls]` binding.
    pub(crate) fn control_candidate_for_set(
        control_set: usize,
        id: ControlBindingId,
        state: ElementState,
    ) -> Option<(usize, Option<ControlEvent>)> {
        (control_set < KEYBOARD_SET_COUNT).then(|| {
            let binding = id.spec().binding;
            let event = match state {
                ElementState::Pressed => Some(binding.press_event()),
                ElementState::Released => binding.release_event(),
            };
            (control_set, event)
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
    pub const SET_COUNT: usize = GAMEPAD_SET_COUNT;

    pub fn load(paths: Option<&AppPaths>) -> Self {
        let Some(paths) = paths else {
            return Self::default();
        };
        let config_path = paths.config_file();
        let config_bytes = match fs::read(&config_path) {
            Ok(config) => config,
            Err(err) => {
                if err.kind() != ErrorKind::NotFound {
                    tracing::warn!(
                        error = %err,
                        path = %config_path.display(),
                        "failed to load gamepad controls config"
                    );
                }
                return Self::default();
            }
        };
        let mut reader = config_bytes.as_slice();
        let mut bindings = match Config::from_reader(&mut reader) {
            Ok(config) => Self::from_config(&config),
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    path = %config_path.display(),
                    "failed to load gamepad controls config"
                );
                Self::default()
            }
        };
        // C++ configuration strings may contain native non-UTF-8 bytes. Axis
        // fields are ASCII scalars, so recover them independently even when
        // the convenience parser could not load an unrelated string field.
        bindings.load_axis_calibrations_from_native_config(&config_bytes);
        bindings
    }

    fn load_axis_calibrations_from_native_config(&mut self, config: &[u8]) {
        for gamepad_index in 0..GAMEPAD_SET_COUNT {
            let section = format!("Gamepad{gamepad_index}");
            for axis in 0..GAMEPAD_CALIBRATED_AXIS_COUNT {
                let min = configured_native_u32(config, &section, &format!("Axis{axis}Min"))
                    .unwrap_or_default();
                let max = configured_native_u32(config, &section, &format!("Axis{axis}Max"))
                    .unwrap_or_default();
                let calibrated = clonk_app_netplay::configured_native_boolean(
                    config,
                    &section,
                    &format!("Axis{axis}Calibrated"),
                )
                .unwrap_or_default();
                self.axis_calibrations[gamepad_index][axis] =
                    GamepadAxisCalibration::new(min, max, calibrated);
            }
        }
    }

    pub(crate) fn from_config(config: &Config) -> Self {
        let mut bindings = Self::default();
        for gamepad_index in 0..GAMEPAD_SET_COUNT {
            let section = format!("Gamepad{gamepad_index}");
            for axis in 0..GAMEPAD_CALIBRATED_AXIS_COUNT {
                let min = config
                    .get_in(Some(&section), &format!("Axis{axis}Min"))
                    .and_then(parse_u32_config_value)
                    .unwrap_or_default();
                let max = config
                    .get_in(Some(&section), &format!("Axis{axis}Max"))
                    .and_then(parse_u32_config_value)
                    .unwrap_or_default();
                let calibrated = config
                    .get_in(Some(&section), &format!("Axis{axis}Calibrated"))
                    .and_then(parse_cpp_boolean_value)
                    .unwrap_or_default();
                bindings.axis_calibrations[gamepad_index][axis] =
                    GamepadAxisCalibration::new(min, max, calibrated);
            }
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

    /// Returns the complete legacy keycode stored for a logical control.
    pub fn raw_key_for_set(&self, gamepad_set: usize, id: ControlBindingId) -> Option<i32> {
        self.keys
            .get(gamepad_set)
            .and_then(|keys| keys[id.spec().index])
    }

    /// Human-readable form used by `KeySelButton`. This follows the gamepad
    /// branches of `C4KeyCodeEx::KeyCode2String(..., true, false)`.
    pub fn key_label_for_set(&self, gamepad_set: usize, id: ControlBindingId) -> String {
        legacy_gamepad_key_label(self.raw_key_for_set(gamepad_set, id))
    }

    /// Stores a complete legacy keycode. `-1` has the C++ meaning "not
    /// assigned". Returns `false` for an out-of-range config block.
    pub fn rebind_raw(&mut self, gamepad_set: usize, id: ControlBindingId, raw_key: i32) -> bool {
        let Some(keys) = self.keys.get_mut(gamepad_set) else {
            return false;
        };
        keys[id.spec().index] = (raw_key != -1).then_some(raw_key);
        true
    }

    pub fn rebind_button(
        &mut self,
        gamepad_set: usize,
        id: ControlBindingId,
        physical_slot: u8,
        physical_button: u8,
    ) -> bool {
        let Some(raw_key) = legacy_gamepad_button_key(physical_slot, physical_button) else {
            return false;
        };
        self.rebind_raw(gamepad_set, id, raw_key)
    }

    /// Mirrors the Options-sheet Reset button: all four gamepad button maps
    /// and all persisted axis-calibration values return to C++ defaults.
    pub fn reset_all(&mut self) {
        self.keys = [[None; CONTROL_BINDING_COUNT]; GAMEPAD_SET_COUNT];
        self.axis_calibrations =
            [[GamepadAxisCalibration::default(); GAMEPAD_CALIBRATED_AXIS_COUNT]; GAMEPAD_SET_COUNT];
        self.axis_calibration_dirty = true;
    }

    pub fn write_to_config(&self, config: &mut Config) {
        for (gamepad_index, keys) in self.keys.iter().enumerate() {
            let section = format!("Gamepad{gamepad_index}");
            for (control_index, raw_key) in keys.iter().enumerate() {
                config.set_in(
                    Some(&section),
                    format!("Button{}", control_index + 1),
                    raw_key.unwrap_or(-1).to_string(),
                );
            }
        }
        if self.axis_calibration_dirty {
            self.write_axis_calibration_to_config(config);
        }
    }

    pub(crate) const fn axis_calibrations(&self) -> GamepadAxisCalibrations {
        self.axis_calibrations
    }

    pub(crate) fn replace_axis_calibrations(&mut self, calibrations: GamepadAxisCalibrations) {
        if self.axis_calibrations != calibrations {
            self.axis_calibrations = calibrations;
            self.axis_calibration_dirty = true;
        }
    }

    pub(crate) const fn axis_calibration_dirty(&self) -> bool {
        self.axis_calibration_dirty
    }

    pub(crate) fn mark_axis_calibration_persisted(&mut self) {
        self.axis_calibration_dirty = false;
    }

    pub(crate) fn write_axis_calibration_to_config(&self, config: &mut Config) {
        for (gamepad_index, calibrations) in self.axis_calibrations.iter().enumerate() {
            let section = format!("Gamepad{gamepad_index}");
            for (axis, calibration) in calibrations.iter().enumerate() {
                config.set_in(
                    Some(&section),
                    format!("Axis{axis}Min"),
                    calibration.min.to_string(),
                );
                config.set_in(
                    Some(&section),
                    format!("Axis{axis}Max"),
                    calibration.max.to_string(),
                );
                config.set_in(
                    Some(&section),
                    format!("Axis{axis}Calibrated"),
                    calibration.calibrated.to_string(),
                );
            }
        }
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
                    "failed to load config for saving gamepad controls"
                );
                return;
            }
        };
        self.write_to_config(&mut config);
        if let Err(err) = config.save(&config_path) {
            tracing::warn!(
                error = %err,
                path = %config_path.display(),
                "failed to persist gamepad control bindings"
            );
        }
    }

    /// Reproduce the callbacks installed by the outer gamepad-set and inner
    /// logical-control loops in `C4Game::InitKeyboard` for one complete raw
    /// gamepad key. The physical slot is encoded into `raw_key`; the config
    /// section determines control set 4..7. Keeping those identities separate
    /// also preserves handwritten configs whose full keycode names a different
    /// physical pad.
    pub fn control_candidates_for_raw_key(
        &self,
        raw_key: i32,
        state: ElementState,
    ) -> impl Iterator<Item = (usize, Option<ControlEvent>)> + '_ {
        self.keys
            .iter()
            .enumerate()
            .flat_map(move |(gamepad_set, keys)| {
                keys.iter()
                    .enumerate()
                    .filter_map(move |(control_index, configured_key)| {
                        if *configured_key != Some(raw_key) {
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

    /// Return the configured callbacks for one physical button key.
    pub fn control_candidates_for_button(
        &self,
        physical_slot: u8,
        physical_button: u8,
        state: ElementState,
    ) -> impl Iterator<Item = (usize, Option<ControlEvent>)> + '_ {
        legacy_gamepad_button_key(physical_slot, physical_button)
            .into_iter()
            .flat_map(move |raw_key| self.control_candidates_for_raw_key(raw_key, state))
    }

    /// Return the exact `KEY_JOY_Axis` callbacks before the synthetic
    /// Left/Up/Right/Down callbacks, matching `C4KeyboardInput::DoInput`'s
    /// key-range order. Axis parity selects the generic direction alias.
    pub fn control_candidates_for_axis(
        &self,
        physical_slot: u8,
        axis: u8,
        high: bool,
        state: ElementState,
    ) -> impl Iterator<Item = (usize, Option<ControlEvent>)> + '_ {
        [
            legacy_gamepad_axis_key(physical_slot, axis, high),
            legacy_gamepad_axis_alias_key(physical_slot, axis, high),
        ]
        .into_iter()
        .flatten()
        .flat_map(move |raw_key| self.control_candidates_for_raw_key(raw_key, state))
    }
}

impl Default for GamepadBindings {
    fn default() -> Self {
        Self {
            keys: [[None; CONTROL_BINDING_COUNT]; GAMEPAD_SET_COUNT],
            axis_calibrations: [[GamepadAxisCalibration::default(); GAMEPAD_CALIBRATED_AXIS_COUNT];
                GAMEPAD_SET_COUNT],
            axis_calibration_dirty: false,
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
    ControlBindingSpec::new(ControlBindingId::Up, 4, Binding::button(ControlButton::Up)),
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
            0xff95, 0xff97, 0xff9a, 0xff96, 0xff9d, 0xff98, 0xff9c, 0xff99, 0xff9b, 0xff9e, 0xff9f,
            0xffab,
        ],
        [105, 111, 112, 107, 108, 59, 44, 46, 47, 109, 228, 252],
        [
            0xff63, 0xff50, 0xff55, 0xffff, 0xff52, 0xff56, 0xff51, 0xff54, 0xff53, 0xff57, 0xff0d,
            0xff08,
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

#[cfg(target_os = "macos")]
fn macos_apple_language() -> Option<String> {
    use objc2_foundation::{ns_string, NSUserDefaults};

    NSUserDefaults::standardUserDefaults()
        .stringArrayForKey(ns_string!("AppleLanguages"))?
        .firstObject()
        .map(|language| language.to_string())
}

fn environment_locale() -> Option<String> {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .find_map(|name| std::env::var(name).ok().filter(|value| !value.is_empty()))
}

fn german_system_from_sources(
    apple_language: Option<&str>,
    environment_locale: Option<&str>,
) -> bool {
    apple_language.map_or_else(
        || environment_locale.is_some_and(|value| value.to_ascii_lowercase().contains("de")),
        |language| language == "de",
    )
}

fn german_system_from_windows_lang_id(lang_id: u16) -> bool {
    const PRIMARY_LANGUAGE_MASK: u16 = 0x03ff;
    const LANG_GERMAN: u16 = 0x07;
    lang_id & PRIMARY_LANGUAGE_MASK == LANG_GERMAN
}

#[cfg(windows)]
fn windows_user_default_language_is_german() -> bool {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetUserDefaultLangID() -> u16;
    }

    // SAFETY: GetUserDefaultLangID has no arguments and returns a LANGID by
    // value. This is the same platform query used by C4Config.cpp.
    german_system_from_windows_lang_id(unsafe { GetUserDefaultLangID() })
}

pub(super) fn is_german_system() -> bool {
    #[cfg(windows)]
    {
        return windows_user_default_language_is_german();
    }
    #[cfg(target_os = "macos")]
    let apple_language = macos_apple_language();
    #[cfg(all(not(windows), not(target_os = "macos")))]
    let apple_language: Option<String> = None;
    #[cfg(not(windows))]
    let environment_locale = environment_locale();
    #[cfg(not(windows))]
    german_system_from_sources(apple_language.as_deref(), environment_locale.as_deref())
}

/// Raw defaults traversed by the advanced-config compiler before per-key INI
/// overrides are applied. Keeping the editor on this table prevents its
/// platform/locale view from drifting from the live controls loader.
pub(crate) fn advanced_config_default_raw_keyboard_keys(
) -> [[i32; CONTROL_BINDING_COUNT]; KEYBOARD_SET_COUNT] {
    cpp_default_raw_keyboard_keys(is_german_system())
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

fn parse_u32_config_value(raw: &str) -> Option<u32> {
    let trimmed = raw.trim();
    if let Some(hex) = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        u32::from_str_radix(hex, 16).ok()
    } else {
        trimmed.parse::<u32>().ok()
    }
}

fn configured_native_u32(config: &[u8], section: &str, key: &str) -> Option<u32> {
    let value = clonk_app_netplay::configured_native_value(config, section, key)?;
    parse_u32_config_value(std::str::from_utf8(value.as_bytes()).ok()?)
}

fn parse_cpp_boolean_value(raw: &str) -> Option<bool> {
    let value = raw.trim().as_bytes();
    if value.first() == Some(&b'1') && !value.get(1).is_some_and(u8::is_ascii_digit) {
        Some(true)
    } else if value.first() == Some(&b'0') && !value.get(1).is_some_and(u8::is_ascii_digit) {
        Some(false)
    } else if value.starts_with(b"true") {
        Some(true)
    } else if value.starts_with(b"false") {
        Some(false)
    } else {
        None
    }
}

fn legacy_gamepad_key(physical_slot: u8, key: u8) -> Option<i32> {
    if physical_slot >= GAMEPAD_SET_COUNT as u8 {
        return None;
    }
    Some(LEGACY_GAMEPAD_KEY_PREFIX + (i32::from(physical_slot) << 8) + i32::from(key))
}

pub(crate) fn legacy_gamepad_button_key(physical_slot: u8, physical_button: u8) -> Option<i32> {
    if physical_button >= LEGACY_GAMEPAD_BUTTON_COUNT {
        return None;
    }
    legacy_gamepad_key(
        physical_slot,
        LEGACY_GAMEPAD_BUTTON_OFFSET + physical_button,
    )
}

fn legacy_gamepad_axis_code(axis: u8, high: bool) -> Option<u8> {
    let code = u16::from(LEGACY_GAMEPAD_AXIS_OFFSET)
        .checked_add(u16::from(axis).checked_mul(2)?)?
        .checked_add(if high { 1 } else { 0 })?;
    (code <= u16::from(LEGACY_GAMEPAD_AXIS_MAX)).then_some(code as u8)
}

/// Exact `KEY_Gamepad(slot, KEY_JOY_Axis(axis, high))` encoding.
pub(crate) fn legacy_gamepad_axis_key(physical_slot: u8, axis: u8, high: bool) -> Option<i32> {
    legacy_gamepad_key(physical_slot, legacy_gamepad_axis_code(axis, high)?)
}

/// Synthetic direction key emitted alongside an exact axis key by
/// `C4KeyboardInput::DoInput`: even axes are horizontal and odd axes vertical.
pub(crate) fn legacy_gamepad_axis_alias_key(
    physical_slot: u8,
    axis: u8,
    high: bool,
) -> Option<i32> {
    legacy_gamepad_axis_code(axis, high)?;
    let key = match (axis % 2, high) {
        (0, false) => 1,
        (1, false) => 2,
        (0, true) => 3,
        (1, true) => 4,
        _ => unreachable!("axis parity is zero or one"),
    };
    legacy_gamepad_key(physical_slot, key)
}

/// Human-readable gamepad key label from
/// `C4KeyCodeEx::KeyCode2String(key, true, false)`. `None` represents the
/// default `-1`/unassigned config value.
pub fn legacy_gamepad_key_label(raw_key: Option<i32>) -> String {
    let Some(raw_key) = raw_key else {
        return "invalid".to_string();
    };
    let raw = raw_key as u32;
    if raw & 0x00ff_0000 != LEGACY_GAMEPAD_KEY_PREFIX as u32 {
        return "invalid".to_string();
    }
    let gamepad = ((raw >> 8) & 0xff) + 1;
    let button = (raw & 0xff) as u8;
    match button {
        1 => format!("Joy{gamepad}Left"),
        2 => format!("Joy{gamepad}Up"),
        3 => format!("Joy{gamepad}Right"),
        4 => format!("Joy{gamepad}Down"),
        0x30..=0x50 => {
            let axis = 1 + (button - 0x30) / 2;
            let extent = if button & 1 == 0 { "Min" } else { "Max" };
            format!("[{axis}] {extent}")
        }
        LEGACY_GAMEPAD_BUTTON_OFFSET..=LEGACY_GAMEPAD_BUTTON_MAX => {
            format!("< {} >", 1 + button - LEGACY_GAMEPAD_BUTTON_OFFSET)
        }
        _ => "invalid".to_string(),
    }
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

fn function_key(value: i32) -> Option<VirtualKeyCode> {
    Some(match value {
        0 => VirtualKeyCode::F1,
        1 => VirtualKeyCode::F2,
        2 => VirtualKeyCode::F3,
        3 => VirtualKeyCode::F4,
        4 => VirtualKeyCode::F5,
        5 => VirtualKeyCode::F6,
        6 => VirtualKeyCode::F7,
        7 => VirtualKeyCode::F8,
        8 => VirtualKeyCode::F9,
        9 => VirtualKeyCode::F10,
        10 => VirtualKeyCode::F11,
        11 => VirtualKeyCode::F12,
        12 => VirtualKeyCode::F13,
        13 => VirtualKeyCode::F14,
        14 => VirtualKeyCode::F15,
        15 => VirtualKeyCode::F16,
        16 => VirtualKeyCode::F17,
        17 => VirtualKeyCode::F18,
        18 => VirtualKeyCode::F19,
        19 => VirtualKeyCode::F20,
        20 => VirtualKeyCode::F21,
        21 => VirtualKeyCode::F22,
        22 => VirtualKeyCode::F23,
        23 => VirtualKeyCode::F24,
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

fn function_key_index(key: VirtualKeyCode) -> Option<i32> {
    Some(match key {
        VirtualKeyCode::F1 => 0,
        VirtualKeyCode::F2 => 1,
        VirtualKeyCode::F3 => 2,
        VirtualKeyCode::F4 => 3,
        VirtualKeyCode::F5 => 4,
        VirtualKeyCode::F6 => 5,
        VirtualKeyCode::F7 => 6,
        VirtualKeyCode::F8 => 7,
        VirtualKeyCode::F9 => 8,
        VirtualKeyCode::F10 => 9,
        VirtualKeyCode::F11 => 10,
        VirtualKeyCode::F12 => 11,
        VirtualKeyCode::F13 => 12,
        VirtualKeyCode::F14 => 13,
        VirtualKeyCode::F15 => 14,
        VirtualKeyCode::F16 => 15,
        VirtualKeyCode::F17 => 16,
        VirtualKeyCode::F18 => 17,
        VirtualKeyCode::F19 => 18,
        VirtualKeyCode::F20 => 19,
        VirtualKeyCode::F21 => 20,
        VirtualKeyCode::F22 => 21,
        VirtualKeyCode::F23 => 22,
        VirtualKeyCode::F24 => 23,
        _ => return None,
    })
}

#[cfg(target_os = "windows")]
pub(crate) fn decode_platform_key_code(value: i32) -> Option<VirtualKeyCode> {
    match value {
        value @ 0x70..=0x87 => function_key(value - 0x70),
        value @ 65..=90 => letter_from_offset(value - 65),
        value @ 48..=57 => digit_key(value - 48),
        value @ 96..=105 => numpad_key(value - 96),
        8 => Some(VirtualKeyCode::Back),
        9 => Some(VirtualKeyCode::Tab),
        13 => Some(VirtualKeyCode::Return),
        19 => Some(VirtualKeyCode::Pause),
        20 => Some(VirtualKeyCode::Capital),
        27 => Some(VirtualKeyCode::Escape),
        32 => Some(VirtualKeyCode::Space),
        33 => Some(VirtualKeyCode::PageUp),
        34 => Some(VirtualKeyCode::PageDown),
        35 => Some(VirtualKeyCode::End),
        36 => Some(VirtualKeyCode::Home),
        37 => Some(VirtualKeyCode::Left),
        38 => Some(VirtualKeyCode::Up),
        39 => Some(VirtualKeyCode::Right),
        40 => Some(VirtualKeyCode::Down),
        44 => Some(VirtualKeyCode::Snapshot),
        45 => Some(VirtualKeyCode::Insert),
        46 => Some(VirtualKeyCode::Delete),
        93 => Some(VirtualKeyCode::Apps),
        106 => Some(VirtualKeyCode::NumpadMultiply),
        107 => Some(VirtualKeyCode::NumpadAdd),
        108 => Some(VirtualKeyCode::NumpadComma),
        109 => Some(VirtualKeyCode::NumpadSubtract),
        110 => Some(VirtualKeyCode::NumpadDecimal),
        111 => Some(VirtualKeyCode::NumpadDivide),
        144 => Some(VirtualKeyCode::Numlock),
        145 => Some(VirtualKeyCode::Scroll),
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
pub(crate) fn decode_platform_key_code(value: i32) -> Option<VirtualKeyCode> {
    match value {
        value @ 0xffbe..=0xffd5 => function_key(value - 0xffbe),
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
        0xff09 => Some(VirtualKeyCode::Tab),
        0xff0d => Some(VirtualKeyCode::Return),
        0xff13 => Some(VirtualKeyCode::Pause),
        0xff14 => Some(VirtualKeyCode::Scroll),
        0xff1b => Some(VirtualKeyCode::Escape),
        0xff50 => Some(VirtualKeyCode::Home),
        0xff51 => Some(VirtualKeyCode::Left),
        0xff52 => Some(VirtualKeyCode::Up),
        0xff53 => Some(VirtualKeyCode::Right),
        0xff54 => Some(VirtualKeyCode::Down),
        0xff55 => Some(VirtualKeyCode::PageUp),
        0xff56 => Some(VirtualKeyCode::PageDown),
        0xff57 => Some(VirtualKeyCode::End),
        0xff61 => Some(VirtualKeyCode::Snapshot),
        0xff67 => Some(VirtualKeyCode::Apps),
        0xff7f => Some(VirtualKeyCode::Numlock),
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
        0xffaa => Some(VirtualKeyCode::NumpadMultiply),
        0xffab => Some(VirtualKeyCode::NumpadAdd),
        0xffac => Some(VirtualKeyCode::NumpadComma),
        0xffad => Some(VirtualKeyCode::NumpadSubtract),
        0xffae => Some(VirtualKeyCode::NumpadDecimal),
        0xffaf => Some(VirtualKeyCode::NumpadDivide),
        0xffbd => Some(VirtualKeyCode::NumpadEquals),
        0xffe5 => Some(VirtualKeyCode::Capital),
        _ => None,
    }
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
pub(crate) fn decode_platform_key_code(value: i32) -> Option<VirtualKeyCode> {
    match value {
        value @ 58..=69 => function_key(value - 58),
        value @ 104..=115 => function_key(value - 104 + 12),
        value @ 4..=29 => letter_from_offset(value - 4),
        value @ 30..=38 => digit_key(value - 29),
        39 => Some(VirtualKeyCode::Key0),
        40 => Some(VirtualKeyCode::Return),
        41 => Some(VirtualKeyCode::Escape),
        42 => Some(VirtualKeyCode::Back),
        43 => Some(VirtualKeyCode::Tab),
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
        57 => Some(VirtualKeyCode::Capital),
        70 => Some(VirtualKeyCode::Snapshot),
        71 => Some(VirtualKeyCode::Scroll),
        72 => Some(VirtualKeyCode::Pause),
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
        83 => Some(VirtualKeyCode::Numlock),
        84 => Some(VirtualKeyCode::NumpadDivide),
        85 => Some(VirtualKeyCode::NumpadMultiply),
        86 => Some(VirtualKeyCode::NumpadSubtract),
        87 => Some(VirtualKeyCode::NumpadAdd),
        88 => Some(VirtualKeyCode::NumpadEnter),
        value @ 89..=97 => numpad_key(value - 88),
        98 => Some(VirtualKeyCode::Numpad0),
        99 => Some(VirtualKeyCode::NumpadDecimal),
        101 => Some(VirtualKeyCode::Apps),
        103 => Some(VirtualKeyCode::NumpadEquals),
        133 => Some(VirtualKeyCode::NumpadComma),
        _ => None,
    }
}

#[cfg(target_os = "windows")]
pub(crate) fn encode_virtual_key_code(key: VirtualKeyCode) -> Option<i32> {
    if let Some(index) = function_key_index(key) {
        return Some(0x70 + index);
    }
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
        VirtualKeyCode::Back => 8,
        VirtualKeyCode::Tab => 9,
        VirtualKeyCode::Return | VirtualKeyCode::NumpadEnter => 13,
        VirtualKeyCode::Pause => 19,
        VirtualKeyCode::Capital => 20,
        VirtualKeyCode::Escape => 27,
        VirtualKeyCode::Space => 32,
        VirtualKeyCode::PageUp => 33,
        VirtualKeyCode::PageDown => 34,
        VirtualKeyCode::End => 35,
        VirtualKeyCode::Home => 36,
        VirtualKeyCode::Left => 37,
        VirtualKeyCode::Up => 38,
        VirtualKeyCode::Right => 39,
        VirtualKeyCode::Down => 40,
        VirtualKeyCode::Snapshot => 44,
        VirtualKeyCode::Insert => 45,
        VirtualKeyCode::Delete => 46,
        VirtualKeyCode::Apps => 93,
        VirtualKeyCode::NumpadMultiply => 106,
        VirtualKeyCode::NumpadAdd => 107,
        VirtualKeyCode::NumpadComma => 108,
        VirtualKeyCode::NumpadSubtract => 109,
        VirtualKeyCode::NumpadDecimal => 110,
        VirtualKeyCode::NumpadDivide => 111,
        VirtualKeyCode::Numlock => 144,
        VirtualKeyCode::Scroll => 145,
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
pub(crate) fn encode_virtual_key_code(key: VirtualKeyCode) -> Option<i32> {
    if let Some(index) = function_key_index(key) {
        return Some(0xffbe + index);
    }
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
        VirtualKeyCode::Tab => 0xff09,
        VirtualKeyCode::Return => 0xff0d,
        VirtualKeyCode::Pause => 0xff13,
        VirtualKeyCode::Scroll => 0xff14,
        VirtualKeyCode::Escape => 0xff1b,
        VirtualKeyCode::Home => 0xff50,
        VirtualKeyCode::Left => 0xff51,
        VirtualKeyCode::Up => 0xff52,
        VirtualKeyCode::Right => 0xff53,
        VirtualKeyCode::Down => 0xff54,
        VirtualKeyCode::PageUp => 0xff55,
        VirtualKeyCode::PageDown => 0xff56,
        VirtualKeyCode::End => 0xff57,
        VirtualKeyCode::Snapshot => 0xff61,
        VirtualKeyCode::Apps => 0xff67,
        VirtualKeyCode::Numlock => 0xff7f,
        VirtualKeyCode::Insert => 0xff63,
        VirtualKeyCode::Delete => 0xffff,
        VirtualKeyCode::NumpadEnter => 0xff8d,
        VirtualKeyCode::NumpadMultiply => 0xffaa,
        VirtualKeyCode::NumpadAdd => 0xffab,
        VirtualKeyCode::NumpadComma => 0xffac,
        VirtualKeyCode::NumpadSubtract => 0xffad,
        VirtualKeyCode::NumpadDecimal => 0xffae,
        VirtualKeyCode::NumpadDivide => 0xffaf,
        VirtualKeyCode::NumpadEquals => 0xffbd,
        VirtualKeyCode::Capital => 0xffe5,
        _ => return None,
    })
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
pub(crate) fn encode_virtual_key_code(key: VirtualKeyCode) -> Option<i32> {
    if let Some(index) = function_key_index(key) {
        return Some(if index < 12 {
            58 + index
        } else {
            104 + index - 12
        });
    }
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
        VirtualKeyCode::Return => 40,
        VirtualKeyCode::Escape => 41,
        VirtualKeyCode::Back => 42,
        VirtualKeyCode::Tab => 43,
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
        VirtualKeyCode::Capital => 57,
        VirtualKeyCode::Snapshot => 70,
        VirtualKeyCode::Scroll => 71,
        VirtualKeyCode::Pause => 72,
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
        VirtualKeyCode::Numlock => 83,
        VirtualKeyCode::NumpadDivide => 84,
        VirtualKeyCode::NumpadMultiply => 85,
        VirtualKeyCode::NumpadSubtract => 86,
        VirtualKeyCode::NumpadAdd => 87,
        VirtualKeyCode::NumpadEnter => 88,
        VirtualKeyCode::NumpadDecimal => 99,
        VirtualKeyCode::OEM102 => 100,
        VirtualKeyCode::Apps => 101,
        VirtualKeyCode::NumpadEquals => 103,
        VirtualKeyCode::NumpadComma => 133,
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
    fn l027_apple_languages_german_selects_iso_player_menu_without_locale_environment() {
        let german_system = german_system_from_sources(Some("de"), None);
        assert!(german_system);
        assert_eq!(
            cpp_default_keyboard_keys(german_system)[0][ControlBindingId::PlayerMenu.spec().index],
            VirtualKeyCode::OEM102
        );
        #[cfg(target_os = "macos")]
        assert_eq!(
            cpp_default_raw_keyboard_keys(german_system)[0]
                [ControlBindingId::PlayerMenu.spec().index],
            100,
            "SDL_SCANCODE_NONUSBACKSLASH"
        );
        assert!(
            !german_system_from_sources(Some("en"), Some("de_DE.UTF-8")),
            "a valid AppleLanguages value is authoritative on macOS"
        );
    }

    #[test]
    fn l027_environment_locale_remains_german_system_fallback() {
        assert!(german_system_from_sources(None, Some("de_DE.UTF-8")));
        assert!(!german_system_from_sources(None, Some("en_US.UTF-8")));
        assert!(!german_system_from_sources(None, None));
    }

    #[test]
    fn l032_windows_language_uses_the_primary_lang_id() {
        assert!(german_system_from_windows_lang_id(0x0407));
        assert!(german_system_from_windows_lang_id(0x0807));
        assert!(!german_system_from_windows_lang_id(0x0409));
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
    fn l026_legacy_axis_and_hat_keycodes_match_cpp_encoding() {
        assert_eq!(legacy_gamepad_axis_key(0, 0, false), Some(0x0042_0030));
        assert_eq!(legacy_gamepad_axis_key(0, 0, true), Some(0x0042_0031));
        assert_eq!(legacy_gamepad_axis_key(0, 1, false), Some(0x0042_0032));
        assert_eq!(legacy_gamepad_axis_key(0, 1, true), Some(0x0042_0033));

        // SDL hat zero becomes axes 6 and 7 before entering DoInput.
        assert_eq!(legacy_gamepad_axis_key(0, 6, false), Some(0x0042_003c));
        assert_eq!(legacy_gamepad_axis_key(0, 6, true), Some(0x0042_003d));
        assert_eq!(legacy_gamepad_axis_key(0, 7, false), Some(0x0042_003e));
        assert_eq!(legacy_gamepad_axis_key(0, 7, true), Some(0x0042_003f));
        assert_eq!(
            legacy_gamepad_axis_alias_key(0, 6, false),
            Some(0x0042_0001)
        );
        assert_eq!(legacy_gamepad_axis_alias_key(0, 6, true), Some(0x0042_0003));
        assert_eq!(
            legacy_gamepad_axis_alias_key(0, 7, false),
            Some(0x0042_0002)
        );
        assert_eq!(legacy_gamepad_axis_alias_key(0, 7, true), Some(0x0042_0004));

        assert_eq!(legacy_gamepad_axis_key(4, 0, false), None);
        assert_eq!(legacy_gamepad_axis_alias_key(4, 0, false), None);
        assert_eq!(legacy_gamepad_axis_key(0, 16, false), Some(0x0042_0050));
        assert_eq!(legacy_gamepad_axis_key(0, 16, true), None);
        assert_eq!(legacy_gamepad_axis_alias_key(0, 16, true), None);
        assert_eq!(legacy_gamepad_axis_key(0, 17, false), None);
    }

    #[test]
    fn l026_unconfigured_axis_registers_no_gameplay_candidates() {
        let bindings = GamepadBindings::from_config(&Config::new());
        assert_eq!(
            bindings
                .control_candidates_for_axis(0, 0, false, ElementState::Pressed)
                .collect::<Vec<_>>(),
            Vec::new()
        );
        assert_eq!(
            bindings
                .control_candidates_for_axis(0, 6, false, ElementState::Pressed)
                .collect::<Vec<_>>(),
            Vec::new()
        );
    }

    #[test]
    fn l026_axis_key_bound_to_left_routes_the_configured_logical_control() {
        let mut config = Config::new();
        config.set_in(Some("Gamepad0"), "Button7", "0x00420030");
        let bindings = GamepadBindings::from_config(&config);

        assert_eq!(
            bindings
                .control_candidates_for_axis(0, 0, false, ElementState::Pressed)
                .collect::<Vec<_>>(),
            vec![(4, Some(ControlEvent::Press(ControlButton::Left)))]
        );
        assert_eq!(
            bindings
                .control_candidates_for_axis(0, 0, false, ElementState::Released)
                .collect::<Vec<_>>(),
            vec![(4, Some(ControlEvent::Release(ControlButton::Left)))]
        );
    }

    #[test]
    fn l026_axis_key_can_route_a_non_direction_logical_control() {
        let mut config = Config::new();
        config.set_in(Some("Gamepad0"), "Button6", "0x00420032");
        let bindings = GamepadBindings::from_config(&config);

        assert_eq!(
            bindings
                .control_candidates_for_axis(0, 1, false, ElementState::Pressed)
                .collect::<Vec<_>>(),
            vec![(
                4,
                Some(ControlEvent::Command {
                    command: ControlCommand::Dig,
                    kind: CommandKind::Press,
                }),
            )]
        );
        assert_eq!(
            bindings
                .control_candidates_for_axis(0, 1, false, ElementState::Released)
                .collect::<Vec<_>>(),
            vec![(
                4,
                Some(ControlEvent::Command {
                    command: ControlCommand::Dig,
                    kind: CommandKind::Release,
                }),
            )]
        );
    }

    #[test]
    fn l026_axis_candidates_keep_exact_range_before_synthetic_alias() {
        let mut config = Config::new();
        // Registration order alone would put Button1 first. DoInput instead
        // exhausts the exact 0x30 range before the generic Left range.
        config.set_in(Some("Gamepad0"), "Button1", "0x00420001");
        config.set_in(Some("Gamepad0"), "Button12", "0x00420030");
        let bindings = GamepadBindings::from_config(&config);

        assert_eq!(
            bindings
                .control_candidates_for_axis(0, 0, false, ElementState::Pressed)
                .collect::<Vec<_>>(),
            vec![
                (
                    4,
                    Some(ControlEvent::Command {
                        command: ControlCommand::Special2,
                        kind: CommandKind::Press,
                    }),
                ),
                (
                    4,
                    Some(ControlEvent::Command {
                        command: ControlCommand::CursorLeft,
                        kind: CommandKind::Press,
                    }),
                ),
            ]
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
    fn gamepad_raw_rebind_labels_and_writes_all_four_sets() {
        let mut bindings = GamepadBindings::default();
        let raw_set_three = legacy_gamepad_button_key(2, 5).expect("valid pad 3 button");
        let raw_set_four = legacy_gamepad_button_key(3, 31).expect("valid pad 4 button");
        assert!(bindings.rebind_raw(2, ControlBindingId::Dig, raw_set_three));
        assert!(bindings.rebind_button(3, ControlBindingId::Special2, 3, 31));
        assert!(!bindings.rebind_raw(4, ControlBindingId::Up, raw_set_three));

        assert_eq!(
            bindings.raw_key_for_set(2, ControlBindingId::Dig),
            Some(raw_set_three)
        );
        assert_eq!(
            bindings.key_label_for_set(2, ControlBindingId::Dig),
            "< 6 >"
        );
        assert_eq!(
            bindings.key_label_for_set(3, ControlBindingId::Special2),
            "< 32 >"
        );

        let mut config = Config::new();
        bindings.write_to_config(&mut config);
        assert_eq!(
            config.get_in(Some("Gamepad2"), "Button6"),
            Some(raw_set_three.to_string().as_str())
        );
        assert_eq!(
            config.get_in(Some("Gamepad3"), "Button12"),
            Some(raw_set_four.to_string().as_str())
        );
        for gamepad in 0..GamepadBindings::SET_COUNT {
            for control in 0..CONTROL_BINDING_COUNT {
                assert!(config
                    .get_in(
                        Some(&format!("Gamepad{gamepad}")),
                        &format!("Button{}", control + 1),
                    )
                    .is_some());
            }
        }

        let loaded = GamepadBindings::from_config(&config);
        assert_eq!(
            loaded.raw_key_for_set(2, ControlBindingId::Dig),
            Some(raw_set_three)
        );
        assert_eq!(
            loaded.raw_key_for_set(3, ControlBindingId::Special2),
            Some(raw_set_four)
        );
    }

    #[test]
    fn gamepad_reset_clears_every_button_and_axis_calibration_on_write() {
        let mut config = Config::new();
        let mut bindings = GamepadBindings::default();
        for gamepad in 0..GamepadBindings::SET_COUNT {
            let section = format!("Gamepad{gamepad}");
            for (control, id) in ControlBindingId::ALL.into_iter().enumerate() {
                let raw = legacy_gamepad_button_key(gamepad as u8, control as u8)
                    .expect("all test buttons fit the legacy range");
                assert!(bindings.rebind_raw(gamepad, id, raw));
            }
            for axis in 0..6 {
                config.set_in(Some(&section), format!("Axis{axis}Min"), "17");
                config.set_in(Some(&section), format!("Axis{axis}Max"), "23");
                config.set_in(Some(&section), format!("Axis{axis}Calibrated"), "true");
            }
        }

        bindings.reset_all();
        bindings.write_to_config(&mut config);
        for gamepad in 0..GamepadBindings::SET_COUNT {
            let section = format!("Gamepad{gamepad}");
            for control in 0..CONTROL_BINDING_COUNT {
                assert_eq!(
                    config.get_in(Some(&section), &format!("Button{}", control + 1)),
                    Some("-1")
                );
            }
            for axis in 0..6 {
                assert_eq!(
                    config.get_in(Some(&section), &format!("Axis{axis}Min")),
                    Some("0")
                );
                assert_eq!(
                    config.get_in(Some(&section), &format!("Axis{axis}Max")),
                    Some("0")
                );
                assert_eq!(
                    config.get_in(Some(&section), &format!("Axis{axis}Calibrated")),
                    Some("false")
                );
            }
        }
    }

    #[test]
    fn l034_native_byte_config_loads_axis_calibration_without_utf8() {
        let config = b"[General]\nName=Andr\xe9\n[Gamepad1]\nAxis2Min=0x10\nAxis2Max=4294967295\nAxis2Calibrated=1\n";
        let mut bindings = GamepadBindings::default();
        bindings.load_axis_calibrations_from_native_config(config);

        assert_eq!(
            bindings.axis_calibrations()[1][2],
            GamepadAxisCalibration::new(16, u32::MAX, true)
        );
        assert!(!bindings.axis_calibration_dirty());
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
        for name in ["Kbd1Key1", "Kbd1Key5", "Kbd2Key1", "Kbd3Key10", "Kbd4Key12"] {
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
    fn keyboard_set_three_and_four_rebinds_round_trip_through_config() {
        let mut bindings = KeyboardBindings::default_bindings();
        assert!(bindings.rebind_for_set(2, ControlBindingId::Dig, VirtualKeyCode::F11));
        assert!(bindings.rebind_for_set(3, ControlBindingId::Special2, VirtualKeyCode::Key9));
        assert!(!bindings.rebind_for_set(4, ControlBindingId::Up, VirtualKeyCode::F12));

        let mut config = Config::new();
        bindings.write_to_config(&mut config);
        assert_eq!(
            config.get_in(Some("Controls"), "Kbd3Key6"),
            Some(
                encode_virtual_key_code(VirtualKeyCode::F11)
                    .unwrap()
                    .to_string()
                    .as_str()
            )
        );
        assert_eq!(
            config.get_in(Some("Controls"), "Kbd4Key12"),
            Some(
                encode_virtual_key_code(VirtualKeyCode::Key9)
                    .unwrap()
                    .to_string()
                    .as_str()
            )
        );
        for set in 0..KeyboardBindings::SET_COUNT {
            for control in 0..CONTROL_BINDING_COUNT {
                assert!(config
                    .get_in(
                        Some("Controls"),
                        &format!("Kbd{}Key{}", set + 1, control + 1),
                    )
                    .is_some());
            }
        }

        let loaded = KeyboardBindings::from_config(&config).expect("all bindings persisted");
        assert_eq!(
            loaded.key_for_set(2, ControlBindingId::Dig),
            Some(VirtualKeyCode::F11)
        );
        assert_eq!(
            loaded.key_for_set(3, ControlBindingId::Special2),
            Some(VirtualKeyCode::Key9)
        );
    }

    #[test]
    fn keyboard_reset_all_restores_every_control_in_all_four_sets() {
        let defaults = KeyboardBindings::default_bindings();
        let mut bindings = defaults.clone();
        for set in 0..KeyboardBindings::SET_COUNT {
            for id in ControlBindingId::ALL {
                assert!(bindings.rebind_for_set(set, id, VirtualKeyCode::F12));
            }
        }

        bindings.reset_all();
        for set in 0..KeyboardBindings::SET_COUNT {
            for id in ControlBindingId::ALL {
                assert_eq!(bindings.key_for_set(set, id), defaults.key_for_set(set, id));
            }
        }
    }

    #[test]
    fn supported_key_detection_matches_encoder() {
        assert!(KeyboardBindings::is_supported_key(VirtualKeyCode::Q));
        assert!(KeyboardBindings::is_supported_key(VirtualKeyCode::Space));
        assert!(KeyboardBindings::is_supported_key(VirtualKeyCode::F1));
        assert!(KeyboardBindings::is_supported_key(VirtualKeyCode::F3));
        assert!(KeyboardBindings::is_supported_key(VirtualKeyCode::F12));
        assert!(KeyboardBindings::is_supported_key(VirtualKeyCode::F13));
        assert!(KeyboardBindings::is_supported_key(VirtualKeyCode::F24));
        assert!(KeyboardBindings::is_supported_key(VirtualKeyCode::Escape));
        assert!(KeyboardBindings::is_supported_key(VirtualKeyCode::Tab));
        assert!(KeyboardBindings::is_supported_key(
            VirtualKeyCode::NumpadMultiply
        ));
        assert!(KeyboardBindings::is_supported_key(
            VirtualKeyCode::NumpadSubtract
        ));
        assert!(KeyboardBindings::is_supported_key(
            VirtualKeyCode::NumpadDivide
        ));
        assert!(KeyboardBindings::is_supported_key(
            VirtualKeyCode::NumpadEnter
        ));
    }

    #[test]
    fn every_winit_function_key_round_trips_through_the_platform_config_codec() {
        let function_keys = [
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
            VirtualKeyCode::F13,
            VirtualKeyCode::F14,
            VirtualKeyCode::F15,
            VirtualKeyCode::F16,
            VirtualKeyCode::F17,
            VirtualKeyCode::F18,
            VirtualKeyCode::F19,
            VirtualKeyCode::F20,
            VirtualKeyCode::F21,
            VirtualKeyCode::F22,
            VirtualKeyCode::F23,
            VirtualKeyCode::F24,
        ];
        for key in function_keys {
            let raw = encode_virtual_key_code(key)
                .unwrap_or_else(|| panic!("{key:?} must have a config representation"));
            assert_eq!(decode_platform_key_code(raw), Some(key), "raw key {raw}");
        }
    }

    #[test]
    fn common_capture_keys_round_trip_through_the_platform_config_codec() {
        for key in [
            VirtualKeyCode::Escape,
            VirtualKeyCode::Tab,
            VirtualKeyCode::NumpadMultiply,
            VirtualKeyCode::NumpadSubtract,
            VirtualKeyCode::NumpadDivide,
            VirtualKeyCode::NumpadComma,
        ] {
            let raw = encode_virtual_key_code(key)
                .unwrap_or_else(|| panic!("{key:?} must have a config representation"));
            assert_eq!(decode_platform_key_code(raw), Some(key), "raw key {raw}");
        }

        #[cfg(not(target_os = "windows"))]
        {
            let raw = encode_virtual_key_code(VirtualKeyCode::NumpadEquals)
                .expect("numpad Equals must have a config representation");
            assert_eq!(
                decode_platform_key_code(raw),
                Some(VirtualKeyCode::NumpadEquals)
            );
            let raw = encode_virtual_key_code(VirtualKeyCode::NumpadEnter)
                .expect("numpad Enter must have a config representation");
            assert_eq!(
                decode_platform_key_code(raw),
                Some(VirtualKeyCode::NumpadEnter)
            );
        }
        #[cfg(target_os = "windows")]
        {
            // Win32's virtual-key representation deliberately aliases both
            // Enter keys to VK_RETURN; persistence therefore cannot retain
            // their physical distinction.
            let raw = encode_virtual_key_code(VirtualKeyCode::NumpadEnter)
                .expect("numpad Enter aliases VK_RETURN");
            assert_eq!(
                raw,
                encode_virtual_key_code(VirtualKeyCode::Return).unwrap()
            );
            assert_eq!(decode_platform_key_code(raw), Some(VirtualKeyCode::Return));
        }
    }
}
