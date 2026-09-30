use super::*;

impl GameApp {
    pub(crate) fn unified_settings_key(
        &mut self,
        key: VirtualKeyCode,
        state: ElementState,
    ) -> Result<bool, EngineError> {
        let modifiers = self.input_routing.live.modifiers;
        let command = modifiers.intersects(ModifiersState::CONTROL | ModifiersState::SUPER);
        if self.unified_settings.is_none() {
            if self.config.compat_profile != crate::settings::CompatProfile::Normal {
                return Ok(false);
            }
            let open = key == self.settings_key
                && command
                && !modifiers.intersects(ModifiersState::SHIFT | ModifiersState::ALT);
            let voice = key == VirtualKeyCode::KeyV
                && modifiers == (ModifiersState::CONTROL | ModifiersState::SHIFT);
            if !open && !voice {
                return Ok(false);
            }
            self.input_routing.key_event_suppresses_text = true;
            if state == ElementState::Pressed
                && !self.input_routing.engine_key_repeated
                && self.window_active
            {
                if voice {
                    self.open_unified_voice_settings()?;
                } else {
                    self.open_unified_settings_where_left()?;
                }
            }
            return Ok(true);
        }
        if self
            .unified_settings
            .as_ref()
            .is_some_and(|settings| settings.controller.view.capturing.is_some())
        {
            self.input_routing.key_event_suppresses_text = true;
            if state == ElementState::Pressed
                && self.window_active
                && !self.input_routing.engine_key_repeated
            {
                if key == VirtualKeyCode::Escape {
                    if let Some(settings) = self.unified_settings.as_mut() {
                        settings.controller.view.capturing = None;
                        settings.controller.view.message = "Binding unchanged".into();
                    }
                } else if let Some(raw) = crate::input::encode_virtual_key_code(key) {
                    self.capture_unified_binding(raw, false)?;
                }
            }
            return Ok(true);
        }
        let Some(settings) = self.unified_settings.as_mut() else {
            return Ok(true);
        };
        self.input_routing.key_event_suppresses_text = !settings.controller.accepts_text()
            || command
            || matches!(
                key,
                VirtualKeyCode::Enter | VirtualKeyCode::Escape | VirtualKeyCode::Tab
            );
        if state != ElementState::Pressed || !self.window_active {
            return Ok(true);
        }
        if key == VirtualKeyCode::Backspace || key == VirtualKeyCode::Delete {
            let actions = settings.controller.edit_command(
                if key == VirtualKeyCode::Backspace {
                    "backspace"
                } else {
                    "delete"
                },
                command,
                modifiers.shift_key(),
            );
            self.process_unified_settings_actions(actions)?;
        } else if command && key == VirtualKeyCode::KeyA {
            settings.controller.edit_command("all", false, false);
        } else if command && key == VirtualKeyCode::KeyF {
            settings.controller.focus_search();
        } else if command && key == VirtualKeyCode::KeyV {
            if let Ok(text) =
                arboard::Clipboard::new().and_then(|mut clipboard| clipboard.get_text())
            {
                settings.controller.text(&text);
            }
        } else if let Some(key) = map_key_code(key) {
            let actions = settings.controller.key(key, modifiers.shift_key(), command);
            self.process_unified_settings_actions(actions)?;
        }
        Ok(true)
    }

    pub(crate) fn unified_settings_pointer(
        &mut self,
        point: GuiPoint,
        down: bool,
    ) -> Result<bool, EngineError> {
        if let Some(settings) = self.unified_settings.as_mut() {
            let actions = settings.controller.pointer(point, down);
            self.process_unified_settings_actions(actions)?;
            return Ok(true);
        }
        if self.settings_launcher().is_some_and(|r| {
            point.x >= r.x as f32
                && point.x < (r.x + r.w) as f32
                && point.y >= r.y as f32
                && point.y < (r.y + r.h) as f32
        }) {
            if down {
                self.open_unified_settings_where_left()?;
            }
            return Ok(true);
        }
        Ok(false)
    }

    fn capture_unified_binding(&mut self, raw: i32, gamepad: bool) -> Result<(), EngineError> {
        use clonk_frontend::startup_options_advanced::AdvancedConfigValue;
        let Some((index, setting)) = self.unified_settings.as_ref().and_then(|settings| {
            let index = settings.controller.view.capturing?;
            Some((index, settings.controller.settings.get(index)?.clone()))
        }) else {
            return Ok(());
        };
        let pad_binding = crate::settings_catalog::gamepad_binding(&setting.id);
        if gamepad != pad_binding.is_some() {
            return Ok(());
        }
        let duplicate =
            if let Some((set, binding)) = crate::settings_catalog::keyboard_binding(&setting.id) {
                crate::input::decode_platform_key_code(raw).and_then(|key| {
                    ControlBindingId::ALL.into_iter().find(|other| {
                        *other != binding && self.bindings.key_for_set(set, *other) == Some(key)
                    })
                })
            } else if let Some((set, binding)) = pad_binding {
                ControlBindingId::ALL.into_iter().find(|other| {
                    *other != binding
                        && self
                            .input_routing
                            .gamepad_bindings
                            .raw_key_for_set(set, *other)
                            == Some(raw)
                })
            } else {
                None
            };
        if let Some(binding) = duplicate {
            if let Some(settings) = self.unified_settings.as_mut() {
                settings.controller.view.message = format!(
                    "Already assigned to {}. Choose another input or Esc.",
                    clonk_frontend::startup_options_controls::CONTROL_KEY_LABELS[binding as usize]
                );
            }
            return Ok(());
        }
        if let AdvancedConfigValue::Integer { min, max, .. } = setting.value {
            self.change_unified_setting(
                index,
                AdvancedConfigValue::Integer {
                    value: i128::from(raw),
                    min,
                    max,
                },
            )?;
            if let Some(settings) = self.unified_settings.as_mut() {
                settings.controller.view.capturing = None;
            }
        }
        Ok(())
    }

    pub(crate) fn unified_settings_gamepad_cluster(
        &mut self,
        events: &[GamepadEvent],
    ) -> Result<(), EngineError> {
        if events
            .iter()
            .any(|event| matches!(event, GamepadEvent::Clear { .. }))
        {
            if let Some(settings) = self.unified_settings.as_mut() {
                settings.controller.view.capturing = None;
                settings.controller.cancel_interaction();
            }
            self.cancel_voice_setup_test();
            return Ok(());
        }
        if !self.window_active {
            return Ok(());
        }
        if self
            .unified_settings
            .as_ref()
            .is_some_and(|settings| settings.controller.view.capturing.is_some())
        {
            let raw = events.iter().find_map(|event| match *event {
                GamepadEvent::Button {
                    slot,
                    button,
                    state: ElementState::Pressed,
                } => crate::input::legacy_gamepad_button_key(slot.index(), button.index()),
                GamepadEvent::Axis {
                    slot,
                    axis,
                    state: ElementState::Pressed,
                } => crate::input::legacy_gamepad_axis_key(slot.index(), axis.index(), axis.high()),
                _ => None,
            });
            if let Some(raw) = raw {
                self.capture_unified_binding(raw, true)?;
            }
            return Ok(());
        }
        // Bumpers change categories; the west face button advances focus.
        // Consume their complete alias cluster before the generic GUI mapping.
        let navigation = events.iter().find_map(|event| match *event {
            GamepadEvent::Button {
                button,
                state: ElementState::Pressed,
                ..
            } => match button.index() {
                4 => Some((true, true)),
                5 => Some((false, true)),
                3 => Some((false, false)),
                _ => None,
            },
            _ => None,
        });
        if let Some((shift, control)) = navigation {
            if let Some(settings) = self.unified_settings.as_mut() {
                let actions = settings.controller.key(KeyCode::Tab, shift, control);
                self.process_unified_settings_actions(actions)?;
            }
            return Ok(());
        }
        let direction = |button| match button {
            ControlButton::Up => VirtualKeyCode::ArrowUp,
            ControlButton::Down => VirtualKeyCode::ArrowDown,
            ControlButton::Left => VirtualKeyCode::ArrowLeft,
            ControlButton::Right => VirtualKeyCode::ArrowRight,
        };
        // A physical event may carry raw, direction, action and GUI aliases.
        // Exactly one alias owns it, including the event that closes settings.
        let key = events
            .iter()
            .find_map(|event| match *event {
                GamepadEvent::Direction {
                    button,
                    state: ElementState::Pressed,
                    ..
                } => Some(direction(button)),
                _ => None,
            })
            .or_else(|| {
                events.iter().find_map(|event| match *event {
                    GamepadEvent::Action {
                        action,
                        state: ElementState::Pressed,
                        ..
                    } => Some(match action {
                        GamepadActionType::Select => VirtualKeyCode::Enter,
                        GamepadActionType::Cancel | GamepadActionType::MenuToggle => {
                            VirtualKeyCode::Escape
                        }
                    }),
                    GamepadEvent::GuiButton {
                        class,
                        state: ElementState::Pressed,
                        ..
                    } => Some(if class == GuiButtonClass::Low {
                        VirtualKeyCode::Enter
                    } else {
                        VirtualKeyCode::Escape
                    }),
                    _ => None,
                })
            })
            .or_else(|| {
                events.iter().find_map(|event| match *event {
                    GamepadEvent::Axis {
                        axis,
                        state: ElementState::Pressed,
                        ..
                    } => Some(direction(axis.direction())),
                    _ => None,
                })
            });
        if let Some(key) = key.and_then(map_key_code) {
            if let Some(settings) = self.unified_settings.as_mut() {
                let actions = settings.controller.key(key, false, false);
                self.process_unified_settings_actions(actions)?;
            }
        }
        Ok(())
    }
}
