use winit::{
    event::ElementState,
    keyboard::{Key, NamedKey},
};

use super::KeyboardEncodeRequest;

pub(super) fn encode(request: KeyboardEncodeRequest<'_>) -> Option<Vec<u8>> {
    if request.modifiers.super_key() {
        return None;
    }
    if matches!(request.state, ElementState::Released) {
        return None;
    }
    let _ = request.repeat;

    let mut bytes = match request.logical_key.as_ref() {
        Key::Named(named) => super::named_key_bytes(named, request.modifiers)?,
        Key::Character(ch) => character_bytes(request, ch)?,
        Key::Dead(_) | Key::Unidentified(_) => {
            let text = request.text?;
            if request.modifiers.control_key() {
                control_bytes(request, text)?
            } else {
                text.as_bytes().to_vec()
            }
        }
    };

    if request.modifiers.alt_key()
        && !matches!(request.logical_key.as_ref(), Key::Named(NamedKey::Escape))
        && !named_key_uses_explicit_modifier_encoding(request)
    {
        let mut prefixed = Vec::with_capacity(bytes.len() + 1);
        prefixed.push(0x1b);
        prefixed.append(&mut bytes);
        bytes = prefixed;
    }

    Some(bytes)
}

fn character_bytes(request: KeyboardEncodeRequest<'_>, ch: &str) -> Option<Vec<u8>> {
    let text = request.text.unwrap_or(ch);

    if request.modifiers.control_key() {
        control_bytes(request, text)
    } else {
        Some(text.as_bytes().to_vec())
    }
}

fn control_bytes(request: KeyboardEncodeRequest<'_>, fallback: &str) -> Option<Vec<u8>> {
    control_text_candidates(request, fallback).find_map(super::control_text_bytes)
}

fn control_text_candidates<'a>(
    request: KeyboardEncodeRequest<'a>,
    fallback: &'a str,
) -> impl Iterator<Item = &'a str> {
    request
        .text
        .into_iter()
        .chain(request.key_without_modifiers)
        .chain(match request.logical_key.as_ref() {
            Key::Character(ch) => Some(ch).into_iter(),
            _ => None.into_iter(),
        })
        .chain(std::iter::once(fallback))
}

fn named_key_uses_explicit_modifier_encoding(request: KeyboardEncodeRequest<'_>) -> bool {
    let Key::Named(named) = request.logical_key.as_ref() else {
        return false;
    };

    if !request.modifiers.alt_key() {
        return false;
    }

    matches!(
        named,
        NamedKey::ArrowUp
            | NamedKey::ArrowDown
            | NamedKey::ArrowLeft
            | NamedKey::ArrowRight
            | NamedKey::Home
            | NamedKey::End
            | NamedKey::PageUp
            | NamedKey::PageDown
            | NamedKey::Insert
            | NamedKey::Delete
            | NamedKey::F1
            | NamedKey::F2
            | NamedKey::F3
            | NamedKey::F4
            | NamedKey::F5
            | NamedKey::F6
            | NamedKey::F7
            | NamedKey::F8
            | NamedKey::F9
            | NamedKey::F10
            | NamedKey::F11
            | NamedKey::F12
    )
}
