//! Local voice settings and an explicit recording test, independent of game mode.
use super::*;
use clonk_audio::{
    VoiceInputDeviceInventory, VoiceMicrophoneTest, VoiceMicrophoneTestState,
    VoiceMicrophoneTestStatus,
};
use clonk_frontend::voice_setup::VoiceSetupController;
use clonk_frontend::voice_setup::{VoiceSetupControl, VoiceSetupView};

pub(crate) trait LocalMicrophoneCheck {
    fn cancel(&self);
    fn status(&self) -> VoiceMicrophoneTestStatus;
}
impl LocalMicrophoneCheck for VoiceMicrophoneTest {
    fn cancel(&self) {
        self.cancel();
    }
    fn status(&self) -> VoiceMicrophoneTestStatus {
        self.status()
    }
}

type StartLocalTest = Box<
    dyn FnMut(
        clonk_audio::VoiceCaptureOptions,
        clonk_audio::AudioWorkerHandle,
    ) -> Result<Box<dyn LocalMicrophoneCheck>, clonk_audio::VoiceCaptureError>,
>;

pub(crate) struct VoiceSetup {
    pub(crate) start_test: StartLocalTest,
    pub(crate) controller: VoiceSetupController,
    pub(crate) test: Option<Box<dyn LocalMicrophoneCheck>>,
    binding: bool,
    message: String,
    opened_in: AppMode,
}
impl VoiceSetup {
    pub(crate) fn cancel_test(&mut self) {
        if let Some(test) = self.test.take() {
            test.cancel();
        }
        self.controller.cancel_interaction();
        self.binding = false;
    }
}
impl Drop for VoiceSetup {
    fn drop(&mut self) {
        self.cancel_test();
    }
}

impl GameApp {
    pub(crate) fn ingame_options_menu(
        &self,
        flags: &OptionFlags,
        selection: usize,
        labels: &IngameMenuLabels,
    ) -> IngameMenuState {
        let menu = IngameMenuState::options_menu(flags, selection, labels);
        if self.config.compat_profile == crate::settings::CompatProfile::LegacyClonk {
            menu
        } else {
            menu.with_voice_setup(self.runtime_resource_text("IDS_CTL_VOICESETUP", "Voice setup"))
        }
    }

    pub(crate) fn open_voice_setup(&mut self) -> Result<(), EngineError> {
        self.voice_chat.stop_capture();
        self.guard_classic_global_gui_bootstrap()?;
        if self.mode == AppMode::Running {
            // Explicit modal entry clears held player controls through the
            // synchronized control path (C4PlayerList.cpp:588-595).
            self.dispatch_control_event(ControlEvent::ClearPressed)?;
            self.release_all_running_pointer_elements();
            self.suspend_ingame_pointer_for_gui();
        }
        self.close_context_menu_silently();
        self.startup_tooltip.pointer_left();
        self.note_classic_lobby_non_pointer_input();
        if self.sound.context.is_none() {
            let options = AudioOptions::load(self.app_paths.as_ref());
            match AudioContext::try_new_with_paths(options, self.app_paths.as_ref()) {
                Ok(audio) => {
                    self.sound.context = Some(connect_audio_context(&mut self.engine, audio))
                }
                Err(error) => tracing::warn!(%error, "voice setup audio is unavailable"),
            }
        }
        if let Some(audio) = self.sound.context.as_ref() {
            audio.borrow().system.prepare_voice_output();
            audio.borrow().system.refresh_voice_input_devices();
        }
        self.voice_setup = Some(VoiceSetup {
            controller: VoiceSetupController::default(),
            test: None,
            start_test: Box::new(start_local_microphone_check),
            binding: false,
            message: String::new(),
            opened_in: self.mode,
        });
        Ok(())
    }

    pub(crate) fn cancel_voice_setup_test(&mut self) {
        if let Some(setup) = self.voice_setup.as_mut() {
            setup.cancel_test();
            setup.message = "Test stopped. Press Record to try again.".into();
        }
    }

    pub(crate) fn close_voice_setup(&mut self) {
        self.voice_setup = None;
        self.voice_chat.stop_capture();
        let audio = borrow_audio_context(self.sound.context.as_ref());
        if let Some(dialog) = self.startup.options_dialog.as_mut() {
            dialog.set_sound_state(load_options_sound_state(audio.as_deref()));
        }
    }

    pub(crate) fn voice_setup_key(
        &mut self,
        key: VirtualKeyCode,
        state: ElementState,
    ) -> Result<bool, EngineError> {
        if self.voice_setup.is_none() {
            if self.mode != AppMode::Loading
                && self.window_active
                && key == VirtualKeyCode::KeyV
                && state == ElementState::Pressed
                && !self.input_routing.engine_key_repeated
                && self
                    .input_routing
                    .live
                    .modifiers
                    .contains(ModifiersState::CONTROL | ModifiersState::SHIFT)
            {
                self.open_voice_setup()?;
                return Ok(true);
            }
            return Ok(false);
        }
        if !self.window_active {
            self.cancel_voice_setup_test();
            return Ok(true);
        }
        let binding = self.voice_setup.as_ref().is_some_and(|setup| setup.binding);
        if binding {
            if state == ElementState::Pressed && !self.input_routing.engine_key_repeated {
                if key != VirtualKeyCode::Escape
                    && crate::input::encode_virtual_key_code(key).is_some()
                {
                    if let Some(audio) = self.sound.context.as_ref() {
                        audio.borrow_mut().options.voice_push_to_talk = key;
                    }
                    self.save_voice_setup_options();
                }
                if let Some(setup) = self.voice_setup.as_mut() {
                    setup.binding = false;
                }
            }
            return Ok(true);
        }
        let action = map_key_code(key).and_then(|key| {
            self.voice_setup.as_mut()?.controller.key(
                key,
                state == ElementState::Pressed,
                self.input_routing
                    .live
                    .modifiers
                    .contains(ModifiersState::SHIFT),
            )
        });
        if let Some(action) = action {
            self.voice_setup_action(action)?;
        }
        Ok(true)
    }

    pub(crate) fn voice_setup_pointer(
        &mut self,
        point: GuiPoint,
        down: bool,
    ) -> Result<bool, EngineError> {
        if self.voice_setup.is_some() {
            if !self.window_active {
                self.cancel_voice_setup_test();
                return Ok(true);
            }
            let surface = self.rendering.graphics.surface();
            let bounds =
                clonk_frontend::voice_setup::panel(surface.width() as i32, surface.height() as i32);
            let action = self
                .voice_setup
                .as_mut()
                .and_then(|setup| setup.controller.pointer(bounds, point, down));
            if let Some(action) = action {
                self.voice_setup_action(action)?;
            }
            return Ok(true);
        }
        if down
            && self.voice_setup_launcher().is_some_and(|r| {
                point.x >= r.x as f32
                    && point.y >= r.y as f32
                    && point.x < (r.x + r.w) as f32
                    && point.y < (r.y + r.h) as f32
            })
        {
            self.open_voice_setup()?;
            return Ok(true);
        }
        Ok(false)
    }

    pub(crate) fn voice_setup_action(
        &mut self,
        action: VoiceSetupControl,
    ) -> Result<(), EngineError> {
        use VoiceSetupControl::*;
        if self.voice_setup.is_none() {
            return Ok(());
        }
        if action == Close {
            self.close_voice_setup();
            return Ok(());
        }
        if !self.window_active {
            self.cancel_voice_setup_test();
            return Ok(());
        }
        if action == Test {
            self.start_voice_setup_test();
            return Ok(());
        }
        self.cancel_voice_setup_test();
        if action == PushToTalk {
            if let Some(setup) = self.voice_setup.as_mut() {
                setup.binding = true;
            }
            return Ok(());
        }
        let Some(audio) = self.sound.context.as_ref() else {
            return Ok(());
        };
        {
            let mut audio = audio.borrow_mut();
            match action {
                Input => {
                    let ids = match audio.system.voice_input_inventory() {
                        VoiceInputDeviceInventory::Ready(devices) => {
                            devices.into_iter().map(|device| device.id).collect()
                        }
                        _ => Vec::new(),
                    };
                    audio.options.voice_input_device =
                        next_device(audio.options.voice_input_device.as_ref(), &ids);
                }
                Output => {
                    let ids = audio
                        .system
                        .output_devices()
                        .into_iter()
                        .map(|device| device.id)
                        .collect::<Vec<_>>();
                    audio.options.voice_output_device =
                        next_device(audio.options.voice_output_device.as_ref(), &ids);
                    audio
                        .system
                        .select_output_device(audio.options.voice_output_device.clone());
                }
                Enabled => audio.options.voice_enabled = !audio.options.voice_enabled,
                Activation => {
                    audio.options.voice_activation_mode = match audio.options.voice_activation_mode
                    {
                        crate::settings::VoiceActivationMode::PushToTalk => {
                            crate::settings::VoiceActivationMode::VoiceActivated
                        }
                        crate::settings::VoiceActivationMode::VoiceActivated => {
                            crate::settings::VoiceActivationMode::PushToTalk
                        }
                    }
                }
                VolumeDown | VolumeUp => {
                    let value = audio.options.voice_volume_percent()
                        + if action == VolumeUp { 10 } else { -10 };
                    audio.options.set_voice_volume_percent(value);
                }
                Echo => {
                    audio.options.voice_echo_cancellation = !audio.options.voice_echo_cancellation
                }
                Noise => {
                    audio.options.voice_noise_suppression = !audio.options.voice_noise_suppression
                }
                Gain => {
                    audio.options.voice_automatic_gain_control =
                        !audio.options.voice_automatic_gain_control
                }
                Retry => {
                    audio.system.retry_output();
                    audio.system.refresh_voice_input_devices();
                }
                Test | Close | PushToTalk => {}
            }
        }
        self.save_voice_setup_options();
        Ok(())
    }

    fn save_voice_setup_options(&mut self) {
        let Some(audio) = self.sound.context.as_ref() else {
            return;
        };
        let mut values = Config::new();
        audio.borrow().options.write_voice_setup_config(&mut values);
        let enabled = audio.borrow().options.voice_enabled.to_string();
        for entry in values.iter() {
            if matches!(entry.key.as_str(), "InputDevice" | "OutputDevice") {
                self.config.deferred.set_escaped(
                    "Voice",
                    &entry.key,
                    &entry.value,
                    entry.value.as_bytes().to_vec(),
                );
            } else {
                self.config.deferred.set("Voice", &entry.key, &entry.value);
            }
        }
        // A complete config save carries deferred values through Config's
        // escaped-string writer, preserving endpoint IDs and other sections.
        if self.app_paths.is_some() {
            if let Err(error) = self.persist_config_value_with_display("Voice", "Enabled", enabled)
            {
                if let Some(setup) = self.voice_setup.as_mut() {
                    setup.message = format!("Could not save voice settings: {error}");
                }
            }
        }
    }

    fn start_voice_setup_test(&mut self) {
        if self
            .voice_setup
            .as_ref()
            .and_then(|setup| setup.test.as_ref())
            .is_some_and(|test| test_running(&test.status()))
        {
            self.cancel_voice_setup_test();
            return;
        }
        self.cancel_voice_setup_test();
        let Some(audio) = self.sound.context.as_ref() else {
            return;
        };
        let audio = audio.borrow();
        audio.system.prepare_voice_output();
        let mut options = clonk_audio::VoiceCaptureOptions::new(
            clonk_audio::VoiceProcessingSwitches::new(audio.options.voice_processing()),
        );
        options.input_device = audio.options.voice_input_device.clone();
        options.echo_reference = Some(audio.system.voice_echo_reference());
        if let Some(setup) = self.voice_setup.as_mut() {
            let result = (setup.start_test)(options, audio.system.worker_handle());
            match result {
                Ok(test) => {
                    setup.test = Some(test);
                    setup.message.clear();
                }
                Err(error) => setup.message = format!("Microphone test: {error}"),
            }
        }
    }

    pub(crate) fn voice_setup_launcher(&self) -> Option<clonk_frontend::classic_gui::IntRect> {
        (self.config.compat_profile != crate::settings::CompatProfile::LegacyClonk
            && self.mode == AppMode::Menu
            && matches!(
                self.startup.view,
                StartupView::Options | StartupView::NetworkLobby
            )
            && self.dialogs.messages.is_empty()
            && self.startup.options_advanced_dialog.is_none()
            && self.context_menus.open.is_none()
            && self.voice_setup.is_none())
        .then(|| {
            clonk_frontend::classic_gui::IntRect::new(
                self.rendering.graphics.surface().width() as i32 - 240,
                8,
                228,
                28,
            )
        })
    }

    pub(crate) fn render_voice_setup(
        &mut self,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) -> Result<bool> {
        let launcher = self.voice_setup_launcher();
        if launcher.is_none() && self.voice_setup.is_none() {
            return Ok(false);
        }
        let assets = self.assets.clone();
        let Some(resources) = assets.message_dialog_resources() else {
            return Ok(false);
        };
        if let Some(bounds) = launcher {
            resources.skin.draw_button(
                self.rendering.graphics.surface_mut(),
                bounds,
                "Voice setup (Ctrl+Shift+V)",
                resources.fonts,
                Default::default(),
                gamma,
            );
        }
        if let Some(setup) = self.voice_setup.as_ref() {
            let view = self.voice_setup_view();
            setup.controller.render(
                self.rendering.graphics.surface_mut(),
                resources,
                &view,
                gamma,
            );
        }
        Ok(true)
    }
}

fn next_device<T: Clone + PartialEq>(current: Option<&T>, devices: &[T]) -> Option<T> {
    match current {
        None => devices.first().cloned(),
        Some(current) => devices
            .iter()
            .position(|id| id == current)
            .and_then(|index| devices.get(index + 1))
            .cloned(),
    }
}

fn test_running(status: &VoiceMicrophoneTestStatus) -> bool {
    matches!(
        status.state,
        VoiceMicrophoneTestState::Opening
            | VoiceMicrophoneTestState::Recording
            | VoiceMicrophoneTestState::Finishing
            | VoiceMicrophoneTestState::Playing
    )
}

fn start_local_microphone_check(
    options: clonk_audio::VoiceCaptureOptions,
    audio: clonk_audio::AudioWorkerHandle,
) -> Result<Box<dyn LocalMicrophoneCheck>, clonk_audio::VoiceCaptureError> {
    #[cfg(not(test))]
    {
        VoiceMicrophoneTest::start(options, audio).map(|test| Box::new(test) as _)
    }
    #[cfg(test)]
    {
        drop((options, audio));
        Err(clonk_audio::VoiceCaptureError::Stream(
            "native microphone tests require an explicit hardware run".into(),
        ))
    }
}

fn capture_status_text(status: &clonk_audio::VoiceCaptureStatus) -> String {
    use clonk_audio::VoiceCaptureStatus::*;
    match status {
        PermissionDenied => {
            "Microphone permission denied. Allow access in system settings, then retry.".into()
        }
        DeviceBusy => "Microphone is busy. Close the other audio app, then retry.".into(),
        Opening => "Microphone did not open in time. Check the device, then retry.".into(),
        Active => "Microphone ready".into(),
        Unavailable => {
            "Selected microphone is unavailable. Connect it or choose another input.".into()
        }
        Retrying(error) => format!("Microphone reconnecting: {error}"),
    }
}

impl GameApp {
    pub(crate) fn update_voice_setup(&mut self) {
        if self
            .voice_setup
            .as_ref()
            .is_some_and(|setup| setup.opened_in != self.mode)
        {
            self.close_voice_setup();
        }
        if !self.window_active {
            self.cancel_voice_setup_test();
        }
    }

    fn voice_setup_view(&self) -> VoiceSetupView {
        let audio = self.sound.context.as_ref().map(|audio| audio.borrow());
        let options = audio
            .as_ref()
            .map(|audio| audio.options.clone())
            .unwrap_or_default();
        let setup = self.voice_setup.as_ref();
        let test = setup
            .and_then(|setup| setup.test.as_ref())
            .map(|test| test.status());
        let inputs = audio
            .as_ref()
            .map(|audio| audio.system.voice_input_inventory());
        let input = options.voice_input_device.as_ref().map_or_else(
            || "System default".into(),
            |selected| match inputs.as_ref() {
                Some(VoiceInputDeviceInventory::Ready(devices)) => devices
                    .iter()
                    .find(|device| &device.id == selected)
                    .map(|device| device.name.clone())
                    .unwrap_or_else(|| format!("Unavailable ({selected})")),
                _ => selected.to_string(),
            },
        );
        let output = options.voice_output_device.as_ref().map_or_else(
            || "System default".into(),
            |selected| {
                audio
                    .as_ref()
                    .and_then(|audio| {
                        audio
                            .system
                            .output_devices()
                            .into_iter()
                            .find(|device| &device.id == selected)
                    })
                    .map(|device| device.name)
                    .unwrap_or_else(|| format!("Unavailable ({selected})"))
            },
        );
        let on = |value| if value { "On" } else { "Off" };
        let recording = test.as_ref().is_some_and(test_running);
        let key = if setup.is_some_and(|setup| setup.binding) {
            "Press a key (Esc cancels)".into()
        } else {
            format!(
                "Push to talk: {}",
                format_key_label(options.voice_push_to_talk)
            )
        };
        let mut status = [String::new(), String::new(), String::new(), String::new()];
        status[0] = match test.as_ref().map(|test| &test.state) {
            Some(VoiceMicrophoneTestState::Opening) => "Opening microphone…".into(),
            Some(VoiceMicrophoneTestState::Recording) => format!(
                "Recording: {:.1} seconds remaining",
                test.as_ref()
                    .map_or(0.0, |test| test.remaining.as_secs_f32())
            ),
            Some(VoiceMicrophoneTestState::Finishing) => "Finishing the recording…".into(),
            Some(VoiceMicrophoneTestState::Playing) => {
                "Playing your recording. Microphone is closed.".into()
            }
            Some(VoiceMicrophoneTestState::Complete) => {
                "Test complete. Microphone is closed.".into()
            }
            Some(VoiceMicrophoneTestState::Unavailable(error)) => capture_status_text(error),
            Some(VoiceMicrophoneTestState::Failed(error)) => format!("Test failed: {error}"),
            _ => "Microphone closed. Record a test to check its level and sound.".into(),
        };
        status[1] = match audio.as_ref().map(|audio| audio.system.output_status()) {
            Some(clonk_audio::AudioOutputStatus::Active { device, .. }) => {
                format!("Output ready: {device}")
            }
            Some(clonk_audio::AudioOutputStatus::Opening) => "Opening output…".into(),
            Some(clonk_audio::AudioOutputStatus::Retrying(error)) => {
                format!("Output reconnecting: {error}")
            }
            _ => {
                "Output unavailable. Connect a device or select another output, then retry.".into()
            }
        };
        status[2] = match self.netplay.manager.as_ref() {
            None => "Offline. The local microphone test works without a game connection.".into(),
            Some(network) if network.voice_available() => {
                "UDP voice negotiated. Live microphone is suspended while setup is open.".into()
            }
            Some(_) => {
                "Voice route unavailable. Voice needs a compatible peer and working UDP.".into()
            }
        };
        status[3] = setup.map_or_else(String::new, |setup| setup.message.clone());
        if status[3].is_empty() {
            status[3] = match inputs {
                Some(VoiceInputDeviceInventory::Scanning) => "Finding microphones…".into(),
                Some(VoiceInputDeviceInventory::Unavailable(error)) => {
                    format!("Input devices unavailable: {error}")
                }
                _ => "Click Input or Output to select the next device; Retry refreshes devices."
                    .into(),
            };
        }
        VoiceSetupView {
            labels: [
                format!("Input: {input} >"),
                format!("Output (game and voice): {output} >"),
                format!("Voice chat: {}", on(options.voice_enabled)),
                format!(
                    "Activation: {}",
                    if options.voice_activation_mode
                        == crate::settings::VoiceActivationMode::PushToTalk
                    {
                        "Push to talk"
                    } else {
                        "Voice activated"
                    }
                ),
                key,
                format!("Volume -  {}%", options.voice_volume_percent()),
                "Volume +".into(),
                format!("Echo cancel: {}", on(options.voice_echo_cancellation)),
                format!("Noise filter: {}", on(options.voice_noise_suppression)),
                format!("Auto gain: {}", on(options.voice_automatic_gain_control)),
                if recording {
                    "Stop test".into()
                } else {
                    "Record microphone test".into()
                },
                "Retry devices".into(),
                "Close".into(),
            ],
            status,
            level: test.map_or(0.0, |test| test.level),
        }
    }
}

impl GameApp {
    pub(crate) fn voice_setup_gamepad(&mut self, event: GamepadEvent) -> Result<(), EngineError> {
        let key = match event {
            GamepadEvent::GuiButton { class, state, .. } => Some((
                match class {
                    GuiButtonClass::Low => VirtualKeyCode::Enter,
                    GuiButtonClass::High => VirtualKeyCode::Escape,
                },
                state,
            )),
            GamepadEvent::Action { action, state, .. } => Some((
                match action {
                    GamepadActionType::Select => VirtualKeyCode::Enter,
                    GamepadActionType::Cancel | GamepadActionType::MenuToggle => {
                        VirtualKeyCode::Escape
                    }
                },
                state,
            )),
            GamepadEvent::Direction { button, state, .. } => Some((
                match button {
                    ControlButton::Up | ControlButton::Left => VirtualKeyCode::ArrowUp,
                    ControlButton::Down | ControlButton::Right => VirtualKeyCode::ArrowDown,
                },
                state,
            )),
            GamepadEvent::Axis { axis, state, .. } => Some((
                if axis.high() {
                    VirtualKeyCode::ArrowDown
                } else {
                    VirtualKeyCode::ArrowUp
                },
                state,
            )),
            GamepadEvent::Clear { .. } => {
                self.cancel_voice_setup_test();
                None
            }
            GamepadEvent::Button { .. } => None,
        };
        if let Some((key, state)) = key {
            if !self.voice_setup.as_ref().is_some_and(|setup| setup.binding)
                || key == VirtualKeyCode::Escape
            {
                self.voice_setup_key(key, state)?;
            }
        }
        Ok(())
    }
}
