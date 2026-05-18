mod keyboard_encoder;

use noctrail_config::KeymapConfig;
use noctrail_layout::FocusDirection;
use winit::{
    event::ElementState,
    keyboard::{Key, ModifiersState, NamedKey},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutAction {
    Copy,
    Paste,
    Focus(FocusDirection),
    ToggleInputMode,
    ToggleAgentAuditBrowser,
    ToggleAgentContextPreview,
    ToggleBlockBrowser,
    ToggleCommandPalette,
    TogglePatchPreview,
    ToggleReviewPanel,
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

#[derive(Debug, Clone, Copy)]
pub(crate) struct KeyboardEncodeRequest<'a> {
    pub state: ElementState,
    pub logical_key: &'a Key,
    pub text: Option<&'a str>,
    pub key_without_modifiers: Option<&'a str>,
    pub modifiers: ModifiersState,
    pub repeat: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyboardProtocol {
    Xterm,
}

pub fn shortcut_action(
    logical_key: &Key,
    modifiers: ModifiersState,
    keymap: &KeymapConfig,
) -> Option<ShortcutAction> {
    let binding = shortcut_binding_string(logical_key, modifiers)?;
    if binding.eq_ignore_ascii_case("ctrl-shift-e") {
        return Some(ShortcutAction::ToggleInputMode);
    }

    [
        (ShortcutAction::Copy, keymap.copy.as_slice()),
        (ShortcutAction::Paste, keymap.paste.as_slice()),
        (
            ShortcutAction::ToggleCommandPalette,
            keymap.command_palette.as_slice(),
        ),
        (
            ShortcutAction::ToggleBlockBrowser,
            keymap.block_browser.as_slice(),
        ),
        (
            ShortcutAction::TogglePatchPreview,
            keymap.patch_preview.as_slice(),
        ),
        (
            ShortcutAction::ToggleReviewPanel,
            keymap.review_panel.as_slice(),
        ),
        (
            ShortcutAction::ToggleAgentContextPreview,
            keymap.agent_context.as_slice(),
        ),
        (
            ShortcutAction::ToggleAgentAuditBrowser,
            keymap.agent_audit.as_slice(),
        ),
        (
            ShortcutAction::Focus(FocusDirection::Left),
            keymap.focus_left.as_slice(),
        ),
        (
            ShortcutAction::Focus(FocusDirection::Right),
            keymap.focus_right.as_slice(),
        ),
        (
            ShortcutAction::Focus(FocusDirection::Up),
            keymap.focus_up.as_slice(),
        ),
        (
            ShortcutAction::Focus(FocusDirection::Down),
            keymap.focus_down.as_slice(),
        ),
    ]
    .into_iter()
    .find_map(|(action, bindings)| {
        bindings
            .iter()
            .any(|candidate| candidate.trim().eq_ignore_ascii_case(&binding))
            .then_some(action)
    })
}

pub fn key_to_pty_bytes(
    logical_key: &Key,
    text: Option<&str>,
    modifiers: ModifiersState,
) -> Option<Vec<u8>> {
    encode_key_event(KeyboardEncodeRequest {
        state: ElementState::Pressed,
        logical_key,
        text,
        key_without_modifiers: None,
        modifiers,
        repeat: false,
    })
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
    encode_key_event(KeyboardEncodeRequest {
        state,
        logical_key,
        text,
        key_without_modifiers: None,
        modifiers,
        repeat: false,
    })
}

pub(crate) fn encode_key_event(request: KeyboardEncodeRequest<'_>) -> Option<Vec<u8>> {
    keyboard_encoder::encode(request)
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

fn shortcut_binding_string(logical_key: &Key, modifiers: ModifiersState) -> Option<String> {
    if modifiers.super_key() {
        return None;
    }

    let key = match logical_key.as_ref() {
        Key::Character(text) if text.chars().count() == 1 => text.to_ascii_lowercase(),
        Key::Named(NamedKey::Insert) => "insert".to_string(),
        _ => return None,
    };

    let mut parts = Vec::new();
    if modifiers.control_key() {
        parts.push("ctrl".to_string());
    }
    if modifiers.alt_key() {
        parts.push("alt".to_string());
    }
    if modifiers.shift_key() {
        parts.push("shift".to_string());
    }
    parts.push(key);
    Some(parts.join("-"))
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
        '/' => Some(0x1f),
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
    if let Some(parameter) = modifiers_parameter(modifiers) {
        let protocol = KeyboardProtocol::Xterm;
        let suffix = match named {
            NamedKey::ArrowUp => Some(("1", 'A')),
            NamedKey::ArrowDown => Some(("1", 'B')),
            NamedKey::ArrowRight => Some(("1", 'C')),
            NamedKey::ArrowLeft => Some(("1", 'D')),
            NamedKey::Home => Some(("1", 'H')),
            NamedKey::End => Some(("1", 'F')),
            NamedKey::F1 => Some(("1", 'P')),
            NamedKey::F2 => Some(("1", 'Q')),
            NamedKey::F3 => Some(("1", 'R')),
            NamedKey::F4 => Some(("1", 'S')),
            _ => None,
        };
        if let Some((prefix, final_byte)) = suffix {
            return Some(modified_csi_sequence(
                protocol, prefix, parameter, final_byte,
            ));
        }

        let tilde_code = match named {
            NamedKey::PageUp => Some(5),
            NamedKey::PageDown => Some(6),
            NamedKey::Insert => Some(2),
            NamedKey::Delete => Some(3),
            NamedKey::F5 => Some(15),
            NamedKey::F6 => Some(17),
            NamedKey::F7 => Some(18),
            NamedKey::F8 => Some(19),
            NamedKey::F9 => Some(20),
            NamedKey::F10 => Some(21),
            NamedKey::F11 => Some(23),
            NamedKey::F12 => Some(24),
            _ => None,
        };
        if let Some(code) = tilde_code {
            return Some(format!("\x1b[{code};{parameter}~").into_bytes());
        }
    }

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

fn modifiers_parameter(modifiers: ModifiersState) -> Option<u8> {
    let encoded = 1
        + u8::from(modifiers.shift_key())
        + (u8::from(modifiers.alt_key()) * 2)
        + (u8::from(modifiers.control_key()) * 4);
    (encoded > 1).then_some(encoded)
}

fn modified_csi_sequence(
    protocol: KeyboardProtocol,
    prefix: &str,
    parameter: u8,
    final_byte: char,
) -> Vec<u8> {
    match protocol {
        KeyboardProtocol::Xterm => format!("\x1b[{prefix};{parameter}{final_byte}").into_bytes(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_keymap() -> KeymapConfig {
        KeymapConfig::default()
    }

    fn request<'a>(
        state: ElementState,
        logical_key: &'a Key,
        text: Option<&'a str>,
        key_without_modifiers: Option<&'a str>,
        modifiers: ModifiersState,
        repeat: bool,
    ) -> KeyboardEncodeRequest<'a> {
        KeyboardEncodeRequest {
            state,
            logical_key,
            text,
            key_without_modifiers,
            modifiers,
            repeat,
        }
    }

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
    fn repeat_key_event_uses_press_encoding() {
        let bytes = encode_key_event(request(
            ElementState::Pressed,
            &Key::Character("a".into()),
            Some("a"),
            Some("a"),
            ModifiersState::empty(),
            true,
        ))
        .expect("repeat should map like press");
        assert_eq!(bytes, b"a");
    }

    #[test]
    fn released_key_event_does_not_write_pty_bytes() {
        assert_eq!(
            encode_key_event(request(
                ElementState::Released,
                &Key::Character("a".into()),
                Some("a"),
                Some("a"),
                ModifiersState::empty(),
                false,
            )),
            None
        );
    }

    #[test]
    fn ctrl_shift_two_prefers_unmodified_key_for_nul() {
        let bytes = encode_key_event(request(
            ElementState::Pressed,
            &Key::Character("@".into()),
            Some("@"),
            Some("2"),
            ModifiersState::CONTROL | ModifiersState::SHIFT,
            false,
        ))
        .expect("ctrl-shift-2 should map");
        assert_eq!(bytes, vec![0x00]);
    }

    #[test]
    fn ctrl_shift_question_mark_falls_back_to_shifted_text() {
        let bytes = encode_key_event(request(
            ElementState::Pressed,
            &Key::Character("?".into()),
            Some("?"),
            Some("/"),
            ModifiersState::CONTROL | ModifiersState::SHIFT,
            false,
        ))
        .expect("ctrl-shift-? should map");
        assert_eq!(bytes, vec![0x7f]);
    }

    #[test]
    fn ctrl_slash_uses_unit_separator() {
        let bytes = encode_key_event(request(
            ElementState::Pressed,
            &Key::Character("/".into()),
            Some("/"),
            Some("/"),
            ModifiersState::CONTROL,
            false,
        ))
        .expect("ctrl-/ should map");
        assert_eq!(bytes, vec![0x1f]);
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
    fn alt_arrow_keys_use_modified_xterm_sequences() {
        let bytes = key_to_pty_bytes(&Key::Named(NamedKey::ArrowLeft), None, ModifiersState::ALT)
            .expect("alt-left should map");
        assert_eq!(bytes, b"\x1b[1;3D");
    }

    #[test]
    fn ctrl_home_uses_modified_xterm_sequence() {
        let bytes = key_to_pty_bytes(&Key::Named(NamedKey::Home), None, ModifiersState::CONTROL)
            .expect("ctrl-home should map");
        assert_eq!(bytes, b"\x1b[1;5H");
    }

    #[test]
    fn shift_function_key_uses_modified_xterm_sequence() {
        let bytes = key_to_pty_bytes(&Key::Named(NamedKey::F5), None, ModifiersState::SHIFT)
            .expect("shift-f5 should map");
        assert_eq!(bytes, b"\x1b[15;2~");
    }

    #[test]
    fn shift_enter_keeps_carriage_return() {
        let bytes = key_to_pty_bytes(&Key::Named(NamedKey::Enter), None, ModifiersState::SHIFT)
            .expect("shift-enter should map");
        assert_eq!(bytes, b"\r");
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
                &Key::Character("b".into()),
                ModifiersState::CONTROL | ModifiersState::SHIFT,
                &default_keymap(),
            ),
            Some(ShortcutAction::ToggleBlockBrowser)
        );
        assert_eq!(
            shortcut_action(
                &Key::Character("c".into()),
                ModifiersState::CONTROL | ModifiersState::SHIFT,
                &default_keymap(),
            ),
            Some(ShortcutAction::Copy)
        );
        assert_eq!(
            shortcut_action(
                &Key::Character("v".into()),
                ModifiersState::CONTROL | ModifiersState::SHIFT,
                &default_keymap(),
            ),
            Some(ShortcutAction::Paste)
        );
        assert_eq!(
            shortcut_action(
                &Key::Named(NamedKey::Insert),
                ModifiersState::SHIFT,
                &default_keymap(),
            ),
            Some(ShortcutAction::Paste)
        );
        assert_eq!(
            shortcut_action(
                &Key::Character("d".into()),
                ModifiersState::CONTROL | ModifiersState::SHIFT,
                &default_keymap(),
            ),
            Some(ShortcutAction::TogglePatchPreview)
        );
        assert_eq!(
            shortcut_action(
                &Key::Character("p".into()),
                ModifiersState::CONTROL | ModifiersState::SHIFT,
                &default_keymap(),
            ),
            Some(ShortcutAction::ToggleCommandPalette)
        );
        assert_eq!(
            shortcut_action(
                &Key::Character("r".into()),
                ModifiersState::CONTROL | ModifiersState::SHIFT,
                &default_keymap(),
            ),
            Some(ShortcutAction::ToggleReviewPanel)
        );
        assert_eq!(
            shortcut_action(
                &Key::Character("e".into()),
                ModifiersState::CONTROL | ModifiersState::SHIFT,
                &default_keymap(),
            ),
            Some(ShortcutAction::ToggleInputMode)
        );
    }

    #[test]
    fn shortcut_actions_cover_agent_context_preview() {
        let modifiers = ModifiersState::CONTROL | ModifiersState::SHIFT;

        assert_eq!(
            shortcut_action(&Key::Character("a".into()), modifiers, &default_keymap()),
            Some(ShortcutAction::ToggleAgentContextPreview)
        );
        assert_eq!(
            shortcut_action(&Key::Character("l".into()), modifiers, &default_keymap()),
            Some(ShortcutAction::ToggleAgentAuditBrowser)
        );
    }

    #[test]
    fn shortcut_actions_cover_directional_focus() {
        assert_eq!(
            shortcut_action(
                &Key::Character("h".into()),
                ModifiersState::ALT,
                &default_keymap(),
            ),
            Some(ShortcutAction::Focus(FocusDirection::Left))
        );
        assert_eq!(
            shortcut_action(
                &Key::Character("j".into()),
                ModifiersState::ALT,
                &default_keymap(),
            ),
            Some(ShortcutAction::Focus(FocusDirection::Down))
        );
        assert_eq!(
            shortcut_action(
                &Key::Character("k".into()),
                ModifiersState::ALT,
                &default_keymap(),
            ),
            Some(ShortcutAction::Focus(FocusDirection::Up))
        );
        assert_eq!(
            shortcut_action(
                &Key::Character("l".into()),
                ModifiersState::ALT,
                &default_keymap(),
            ),
            Some(ShortcutAction::Focus(FocusDirection::Right))
        );
    }

    #[test]
    fn shortcut_actions_follow_custom_keymap_bindings() {
        let keymap = KeymapConfig {
            copy: vec!["ctrl-shift-y".to_string()],
            focus_left: vec!["alt-a".to_string()],
            ..KeymapConfig::default()
        };

        assert_eq!(
            shortcut_action(
                &Key::Character("y".into()),
                ModifiersState::CONTROL | ModifiersState::SHIFT,
                &keymap,
            ),
            Some(ShortcutAction::Copy)
        );
        assert_eq!(
            shortcut_action(&Key::Character("a".into()), ModifiersState::ALT, &keymap,),
            Some(ShortcutAction::Focus(FocusDirection::Left))
        );
        assert_eq!(
            shortcut_action(
                &Key::Character("c".into()),
                ModifiersState::CONTROL | ModifiersState::SHIFT,
                &keymap,
            ),
            None
        );
    }
}
