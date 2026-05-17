use winit::{
    event::ElementState,
    keyboard::{Key, ModifiersState, NamedKey},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutAction {
    Copy,
    Paste,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseReportKind {
    Press(MouseButton),
    Release(MouseButton),
    Drag(MouseButton),
    Move,
    WheelUp,
    WheelDown,
}

pub fn shortcut_action(logical_key: &Key, modifiers: ModifiersState) -> Option<ShortcutAction> {
    if modifiers.alt_key() || modifiers.super_key() {
        return None;
    }

    match logical_key.as_ref() {
        Key::Character(text) if modifiers.control_key() && modifiers.shift_key() => {
            match text.to_ascii_lowercase().as_str() {
                "c" => Some(ShortcutAction::Copy),
                "v" => Some(ShortcutAction::Paste),
                _ => None,
            }
        }
        Key::Named(NamedKey::Insert) if modifiers.shift_key() && !modifiers.control_key() => {
            Some(ShortcutAction::Paste)
        }
        Key::Named(NamedKey::Insert) if modifiers.control_key() && !modifiers.shift_key() => {
            Some(ShortcutAction::Copy)
        }
        _ => None,
    }
}

pub fn key_to_pty_bytes(
    logical_key: &Key,
    text: Option<&str>,
    modifiers: ModifiersState,
) -> Option<Vec<u8>> {
    if modifiers.super_key() {
        return None;
    }

    let mut bytes = match logical_key.as_ref() {
        Key::Named(named) => named_key_bytes(named, modifiers)?,
        Key::Character(ch) => character_bytes(ch, text, modifiers)?,
        Key::Dead(_) | Key::Unidentified(_) => {
            let text = text?;
            if modifiers.control_key() {
                control_text_bytes(text)?
            } else {
                text.as_bytes().to_vec()
            }
        }
    };

    if modifiers.alt_key() && !matches!(logical_key.as_ref(), Key::Named(NamedKey::Escape)) {
        let mut prefixed = Vec::with_capacity(bytes.len() + 1);
        prefixed.push(0x1b);
        prefixed.append(&mut bytes);
        bytes = prefixed;
    }

    Some(bytes)
}

pub fn paste_bytes(text: &str, bracketed_paste: bool) -> Vec<u8> {
    if text.is_empty() {
        return Vec::new();
    }

    if bracketed_paste {
        let mut bytes = Vec::with_capacity(text.len() + 12);
        bytes.extend_from_slice(b"\x1b[200~");
        bytes.extend_from_slice(text.as_bytes());
        bytes.extend_from_slice(b"\x1b[201~");
        bytes
    } else {
        text.as_bytes().to_vec()
    }
}

pub fn key_event_to_pty_bytes(
    state: ElementState,
    logical_key: &Key,
    text: Option<&str>,
    modifiers: ModifiersState,
) -> Option<Vec<u8>> {
    if !state.is_pressed() {
        return None;
    }

    key_to_pty_bytes(logical_key, text, modifiers)
}

pub fn mouse_report_bytes(kind: MouseReportKind, row: usize, col: usize, sgr: bool) -> Vec<u8> {
    let row = row.saturating_add(1);
    let col = col.saturating_add(1);
    let button = mouse_button_code(kind);

    if sgr {
        let suffix = match kind {
            MouseReportKind::Release(_) => 'm',
            _ => 'M',
        };
        format!("\x1b[<{button};{col};{row}{suffix}").into_bytes()
    } else {
        let encoded_col = u8::try_from(col.min(223))
            .ok()
            .and_then(|value| value.checked_add(32))
            .unwrap_or(u8::MAX);
        let encoded_row = u8::try_from(row.min(223))
            .ok()
            .and_then(|value| value.checked_add(32))
            .unwrap_or(u8::MAX);
        vec![b'\x1b', b'[', b'M', button + 32, encoded_col, encoded_row]
    }
}

fn character_bytes(ch: &str, text: Option<&str>, modifiers: ModifiersState) -> Option<Vec<u8>> {
    let text = text.unwrap_or(ch);

    if modifiers.control_key() {
        control_text_bytes(text)
    } else {
        Some(text.as_bytes().to_vec())
    }
}

fn control_text_bytes(text: &str) -> Option<Vec<u8>> {
    let ch = text.chars().next()?;
    let byte = control_byte(ch)?;
    Some(vec![byte])
}

fn control_byte(ch: char) -> Option<u8> {
    match ch {
        'a'..='z' | 'A'..='Z' => Some(ch.to_ascii_uppercase() as u8 - b'@'),
        '@' | ' ' => Some(0x00),
        '[' => Some(0x1b),
        '\\' => Some(0x1c),
        ']' => Some(0x1d),
        '^' => Some(0x1e),
        '_' => Some(0x1f),
        '?' => Some(0x7f),
        '2' => Some(0x00),
        '3' => Some(0x1b),
        '4' => Some(0x1c),
        '5' => Some(0x1d),
        '6' => Some(0x1e),
        '7' => Some(0x1f),
        '8' => Some(0x7f),
        _ => None,
    }
}

fn mouse_button_code(kind: MouseReportKind) -> u8 {
    match kind {
        MouseReportKind::Press(MouseButton::Left) => 0,
        MouseReportKind::Press(MouseButton::Middle) => 1,
        MouseReportKind::Press(MouseButton::Right) => 2,
        MouseReportKind::Release(MouseButton::Left)
        | MouseReportKind::Release(MouseButton::Middle)
        | MouseReportKind::Release(MouseButton::Right) => 3,
        MouseReportKind::Drag(MouseButton::Left) => 32,
        MouseReportKind::Drag(MouseButton::Middle) => 33,
        MouseReportKind::Drag(MouseButton::Right) => 34,
        MouseReportKind::Move => 35,
        MouseReportKind::WheelUp => 64,
        MouseReportKind::WheelDown => 65,
    }
}

fn named_key_bytes(named: NamedKey, modifiers: ModifiersState) -> Option<Vec<u8>> {
    let base = match named {
        NamedKey::Enter => b"\r".to_vec(),
        NamedKey::Tab => {
            if modifiers.shift_key() {
                b"\x1b[Z".to_vec()
            } else {
                b"\t".to_vec()
            }
        }
        NamedKey::Space => {
            if modifiers.control_key() {
                vec![0x00]
            } else {
                b" ".to_vec()
            }
        }
        NamedKey::Escape => b"\x1b".to_vec(),
        NamedKey::Backspace => b"\x7f".to_vec(),
        NamedKey::ArrowUp => b"\x1b[A".to_vec(),
        NamedKey::ArrowDown => b"\x1b[B".to_vec(),
        NamedKey::ArrowRight => b"\x1b[C".to_vec(),
        NamedKey::ArrowLeft => b"\x1b[D".to_vec(),
        NamedKey::Home => b"\x1b[H".to_vec(),
        NamedKey::End => b"\x1b[F".to_vec(),
        NamedKey::PageUp => b"\x1b[5~".to_vec(),
        NamedKey::PageDown => b"\x1b[6~".to_vec(),
        NamedKey::Insert => b"\x1b[2~".to_vec(),
        NamedKey::Delete => b"\x1b[3~".to_vec(),
        NamedKey::F1 => b"\x1bOP".to_vec(),
        NamedKey::F2 => b"\x1bOQ".to_vec(),
        NamedKey::F3 => b"\x1bOR".to_vec(),
        NamedKey::F4 => b"\x1bOS".to_vec(),
        NamedKey::F5 => b"\x1b[15~".to_vec(),
        NamedKey::F6 => b"\x1b[17~".to_vec(),
        NamedKey::F7 => b"\x1b[18~".to_vec(),
        NamedKey::F8 => b"\x1b[19~".to_vec(),
        NamedKey::F9 => b"\x1b[20~".to_vec(),
        NamedKey::F10 => b"\x1b[21~".to_vec(),
        NamedKey::F11 => b"\x1b[23~".to_vec(),
        NamedKey::F12 => b"\x1b[24~".to_vec(),
        NamedKey::Shift
        | NamedKey::Control
        | NamedKey::Alt
        | NamedKey::Super
        | NamedKey::Meta
        | NamedKey::Fn
        | NamedKey::FnLock
        | NamedKey::CapsLock
        | NamedKey::NumLock
        | NamedKey::ScrollLock
        | NamedKey::Symbol
        | NamedKey::SymbolLock
        | NamedKey::Hyper
        | NamedKey::AltGraph => return None,
        _ => return None,
    };

    Some(base)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn printable_text_is_forwarded() {
        let bytes = key_to_pty_bytes(
            &Key::Character("a".into()),
            Some("a"),
            ModifiersState::empty(),
        )
        .expect("character should map");
        assert_eq!(bytes, b"a");
    }

    #[test]
    fn ctrl_letter_maps_to_caret_byte() {
        let bytes = key_to_pty_bytes(
            &Key::Character("c".into()),
            Some("c"),
            ModifiersState::CONTROL,
        )
        .expect("ctrl-c should map");
        assert_eq!(bytes, vec![0x03]);
    }

    #[test]
    fn ctrl_d_maps_to_eot_byte() {
        let bytes = key_to_pty_bytes(
            &Key::Character("d".into()),
            Some("d"),
            ModifiersState::CONTROL,
        )
        .expect("ctrl-d should map");
        assert_eq!(bytes, vec![0x04]);
    }

    #[test]
    fn alt_prefixes_escape() {
        let bytes = key_to_pty_bytes(&Key::Character("x".into()), Some("x"), ModifiersState::ALT)
            .expect("alt-x should map");
        assert_eq!(bytes, b"\x1bx");
    }

    #[test]
    fn shift_tab_uses_backtab_sequence() {
        let bytes = key_to_pty_bytes(
            &Key::Named(NamedKey::Tab),
            Some("\t"),
            ModifiersState::SHIFT,
        )
        .expect("shift-tab should map");
        assert_eq!(bytes, b"\x1b[Z");
    }

    #[test]
    fn function_keys_use_escape_sequences() {
        let bytes = key_to_pty_bytes(&Key::Named(NamedKey::F5), None, ModifiersState::empty())
            .expect("f5 should map");
        assert_eq!(bytes, b"\x1b[15~");
    }

    #[test]
    fn paste_bytes_wraps_when_bracketed() {
        assert_eq!(paste_bytes("hello", true), b"\x1b[200~hello\x1b[201~");
        assert_eq!(paste_bytes("hello", false), b"hello");
    }

    #[test]
    fn sgr_mouse_reports_use_one_based_coordinates() {
        assert_eq!(
            mouse_report_bytes(MouseReportKind::Press(MouseButton::Left), 4, 9, true),
            b"\x1b[<0;10;5M"
        );
        assert_eq!(
            mouse_report_bytes(MouseReportKind::Release(MouseButton::Left), 4, 9, true),
            b"\x1b[<3;10;5m"
        );
        assert_eq!(
            mouse_report_bytes(MouseReportKind::WheelDown, 1, 2, true),
            b"\x1b[<65;3;2M"
        );
        assert_eq!(
            mouse_report_bytes(MouseReportKind::Move, 0, 0, true),
            b"\x1b[<35;1;1M"
        );
    }

    #[test]
    fn legacy_mouse_reports_use_x10_encoding() {
        assert_eq!(
            mouse_report_bytes(MouseReportKind::Press(MouseButton::Left), 0, 0, false),
            vec![0x1b, b'[', b'M', 32, 33, 33]
        );
        assert_eq!(
            mouse_report_bytes(MouseReportKind::Drag(MouseButton::Left), 1, 2, false),
            vec![0x1b, b'[', b'M', 64, 35, 34]
        );
    }

    #[test]
    fn shortcut_actions_cover_copy_and_paste() {
        assert_eq!(
            shortcut_action(
                &Key::Character("c".into()),
                ModifiersState::CONTROL | ModifiersState::SHIFT,
            ),
            Some(ShortcutAction::Copy)
        );
        assert_eq!(
            shortcut_action(
                &Key::Character("v".into()),
                ModifiersState::CONTROL | ModifiersState::SHIFT,
            ),
            Some(ShortcutAction::Paste)
        );
        assert_eq!(
            shortcut_action(&Key::Named(NamedKey::Insert), ModifiersState::SHIFT),
            Some(ShortcutAction::Paste)
        );
    }
}
