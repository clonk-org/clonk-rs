use winit::keyboard::KeyCode as VirtualKeyCode;

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn sdl_key_label(key: VirtualKeyCode) -> Option<&'static str> {
    Some(match key {
        VirtualKeyCode::Enter => "Return",
        VirtualKeyCode::Escape => "Escape",
        VirtualKeyCode::Backspace => "Backspace",
        VirtualKeyCode::Tab => "Tab",
        VirtualKeyCode::Space => "Space",
        VirtualKeyCode::Minus => "-",
        VirtualKeyCode::Equal => "=",
        VirtualKeyCode::BracketLeft => "[",
        VirtualKeyCode::BracketRight => "]",
        VirtualKeyCode::Backslash => "\\",
        VirtualKeyCode::Semicolon => ";",
        VirtualKeyCode::Quote => "'",
        VirtualKeyCode::Backquote => "`",
        VirtualKeyCode::Comma => ",",
        VirtualKeyCode::Period => ".",
        VirtualKeyCode::Slash => "/",
        VirtualKeyCode::CapsLock => "CapsLock",
        VirtualKeyCode::PrintScreen => "PrintScreen",
        VirtualKeyCode::ScrollLock => "ScrollLock",
        VirtualKeyCode::Pause => "Pause",
        VirtualKeyCode::Insert => "Insert",
        VirtualKeyCode::Home => "Home",
        VirtualKeyCode::PageUp => "PageUp",
        VirtualKeyCode::Delete => "Delete",
        VirtualKeyCode::End => "End",
        VirtualKeyCode::PageDown => "PageDown",
        VirtualKeyCode::ArrowRight => "Right",
        VirtualKeyCode::ArrowLeft => "Left",
        VirtualKeyCode::ArrowDown => "Down",
        VirtualKeyCode::ArrowUp => "Up",
        VirtualKeyCode::NumLock => "Numlock",
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
        VirtualKeyCode::IntlBackslash => "",
        VirtualKeyCode::ContextMenu => "Application",
        VirtualKeyCode::NumpadEqual => "Keypad =",
        VirtualKeyCode::NumpadComma => "Keypad ,",
        _ => return None,
    })
}

pub fn format_key_label(key: VirtualKeyCode) -> String {
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    if let Some(label) = sdl_key_label(key) {
        return label.to_string();
    }

    let debug_name = format!("{key:?}");
    if let Some(label) = debug_name
        .strip_prefix("Key")
        .or_else(|| debug_name.strip_prefix("Digit"))
    {
        return label.to_string();
    }

    match key {
        VirtualKeyCode::Space => "Space".into(),
        VirtualKeyCode::ArrowLeft => "Left".into(),
        VirtualKeyCode::ArrowRight => "Right".into(),
        VirtualKeyCode::ArrowUp => "Up".into(),
        VirtualKeyCode::ArrowDown => "Down".into(),
        other => format!("{:?}", other),
    }
}

#[cfg(all(
    test,
    any(not(feature = "app-test-shard-mode"), feature = "app-test-shard-5",),
))]
mod tests {
    use super::*;

    #[test]
    fn alphanumeric_key_labels_remain_unchanged() {
        for (key, expected) in [
            (VirtualKeyCode::KeyA, "A"),
            (VirtualKeyCode::KeyB, "B"),
            (VirtualKeyCode::KeyC, "C"),
            (VirtualKeyCode::KeyD, "D"),
            (VirtualKeyCode::KeyE, "E"),
            (VirtualKeyCode::KeyF, "F"),
            (VirtualKeyCode::KeyG, "G"),
            (VirtualKeyCode::KeyH, "H"),
            (VirtualKeyCode::KeyI, "I"),
            (VirtualKeyCode::KeyJ, "J"),
            (VirtualKeyCode::KeyK, "K"),
            (VirtualKeyCode::KeyL, "L"),
            (VirtualKeyCode::KeyM, "M"),
            (VirtualKeyCode::KeyN, "N"),
            (VirtualKeyCode::KeyO, "O"),
            (VirtualKeyCode::KeyP, "P"),
            (VirtualKeyCode::KeyQ, "Q"),
            (VirtualKeyCode::KeyR, "R"),
            (VirtualKeyCode::KeyS, "S"),
            (VirtualKeyCode::KeyT, "T"),
            (VirtualKeyCode::KeyU, "U"),
            (VirtualKeyCode::KeyV, "V"),
            (VirtualKeyCode::KeyW, "W"),
            (VirtualKeyCode::KeyX, "X"),
            (VirtualKeyCode::KeyY, "Y"),
            (VirtualKeyCode::KeyZ, "Z"),
            (VirtualKeyCode::Digit0, "0"),
            (VirtualKeyCode::Digit1, "1"),
            (VirtualKeyCode::Digit2, "2"),
            (VirtualKeyCode::Digit3, "3"),
            (VirtualKeyCode::Digit4, "4"),
            (VirtualKeyCode::Digit5, "5"),
            (VirtualKeyCode::Digit6, "6"),
            (VirtualKeyCode::Digit7, "7"),
            (VirtualKeyCode::Digit8, "8"),
            (VirtualKeyCode::Digit9, "9"),
        ] {
            assert_eq!(format_key_label(key), expected, "label for {key:?}");
        }
    }

    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    #[test]
    fn key_labels_match_sdl_scancode_names() {
        for (key, expected) in [
            (VirtualKeyCode::Enter, "Return"),
            (VirtualKeyCode::Escape, "Escape"),
            (VirtualKeyCode::Backspace, "Backspace"),
            (VirtualKeyCode::Tab, "Tab"),
            (VirtualKeyCode::Space, "Space"),
            (VirtualKeyCode::Minus, "-"),
            (VirtualKeyCode::Equal, "="),
            (VirtualKeyCode::BracketLeft, "["),
            (VirtualKeyCode::BracketRight, "]"),
            (VirtualKeyCode::Backslash, "\\"),
            (VirtualKeyCode::Semicolon, ";"),
            (VirtualKeyCode::Quote, "'"),
            (VirtualKeyCode::Backquote, "`"),
            (VirtualKeyCode::Comma, ","),
            (VirtualKeyCode::Period, "."),
            (VirtualKeyCode::Slash, "/"),
            (VirtualKeyCode::CapsLock, "CapsLock"),
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
            (VirtualKeyCode::PrintScreen, "PrintScreen"),
            (VirtualKeyCode::ScrollLock, "ScrollLock"),
            (VirtualKeyCode::Pause, "Pause"),
            (VirtualKeyCode::Insert, "Insert"),
            (VirtualKeyCode::Home, "Home"),
            (VirtualKeyCode::PageUp, "PageUp"),
            (VirtualKeyCode::Delete, "Delete"),
            (VirtualKeyCode::End, "End"),
            (VirtualKeyCode::PageDown, "PageDown"),
            (VirtualKeyCode::ArrowRight, "Right"),
            (VirtualKeyCode::ArrowLeft, "Left"),
            (VirtualKeyCode::ArrowDown, "Down"),
            (VirtualKeyCode::ArrowUp, "Up"),
            (VirtualKeyCode::NumLock, "Numlock"),
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
            (VirtualKeyCode::IntlBackslash, ""),
            (VirtualKeyCode::ContextMenu, "Application"),
            (VirtualKeyCode::NumpadEqual, "Keypad ="),
            (VirtualKeyCode::NumpadComma, "Keypad ,"),
        ] {
            assert_eq!(format_key_label(key), expected, "label for {key:?}");
        }
    }
}
