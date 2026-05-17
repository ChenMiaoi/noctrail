//! Recording fixture harness for terminal state replays.

use std::{error::Error, fmt, fs, path::Path};

use serde::{Deserialize, Serialize};

use crate::{LineEnding, Selection, TerminalSnapshot, TerminalState};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordingSuite {
    pub cases: Vec<RecordingCase>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordingCase {
    pub name: String,
    pub width: usize,
    pub height: usize,
    pub input_hex: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scrollback_limit: Option<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resizes: Vec<[usize; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<Selection>,
    #[serde(default = "default_line_ending")]
    pub line_ending: LineEnding,
    pub expected: TerminalSnapshot,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_selection_text: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RecordingError {
    message: String,
}

impl RecordingError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for RecordingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for RecordingError {}

pub fn replay_recording_file(path: impl AsRef<Path>) -> Result<(), RecordingError> {
    let path = path.as_ref();
    let data = fs::read_to_string(path).map_err(|error| {
        RecordingError::new(format!(
            "{}: failed to read fixture: {error}",
            path.display()
        ))
    })?;
    let suite: RecordingSuite = serde_json::from_str(&data).map_err(|error| {
        RecordingError::new(format!(
            "{}: failed to parse fixture: {error}",
            path.display()
        ))
    })?;
    replay_recording_suite(path, &suite)
}

pub fn replay_recording_suite(
    path: impl AsRef<Path>,
    suite: &RecordingSuite,
) -> Result<(), RecordingError> {
    let path = path.as_ref();
    for case in &suite.cases {
        replay_recording_case(path, case)?;
    }
    Ok(())
}

pub fn replay_recording_case(
    path: impl AsRef<Path>,
    case: &RecordingCase,
) -> Result<(), RecordingError> {
    let path = path.as_ref();
    let bytes = decode_hex(&case.input_hex).map_err(|error| {
        RecordingError::new(format!(
            "{} [{}]: invalid input_hex: {error}",
            path.display(),
            case.name
        ))
    })?;

    let mut terminal = TerminalState::new(case.width, case.height);
    if let Some(limit) = case.scrollback_limit {
        terminal.set_scrollback_limit(limit);
    }
    terminal.advance_bytes(&bytes);

    for resize in &case.resizes {
        terminal.resize(resize[0], resize[1]);
    }

    if let Some(selection) = case.selection.clone() {
        terminal.set_selection(Some(selection));
    }

    let actual_snapshot = terminal.snapshot();
    if actual_snapshot != case.expected {
        let expected = serde_json::to_string_pretty(&case.expected)
            .unwrap_or_else(|_| "<failed to format expected snapshot>".to_string());
        let actual = serde_json::to_string_pretty(&actual_snapshot)
            .unwrap_or_else(|_| "<failed to format actual snapshot>".to_string());
        return Err(RecordingError::new(format!(
            "{} [{}]: snapshot mismatch\nexpected:\n{expected}\nactual:\n{actual}",
            path.display(),
            case.name
        )));
    }

    if let Some(expected_selection_text) = &case.expected_selection_text {
        let actual_selection_text = terminal
            .selection_text(case.line_ending)
            .unwrap_or_default();
        if &actual_selection_text != expected_selection_text {
            return Err(RecordingError::new(format!(
                "{} [{}]: selection text mismatch\nexpected: {:?}\nactual:   {:?}",
                path.display(),
                case.name,
                expected_selection_text,
                actual_selection_text
            )));
        }
    }

    Ok(())
}

fn default_line_ending() -> LineEnding {
    LineEnding::Lf
}

fn decode_hex(input: &str) -> Result<Vec<u8>, RecordingError> {
    let mut filtered = String::new();
    for ch in input.chars() {
        if !ch.is_whitespace() {
            filtered.push(ch);
        }
    }

    if !filtered.len().is_multiple_of(2) {
        return Err(RecordingError::new(
            "hex input length must be even after removing whitespace",
        ));
    }

    let mut bytes = Vec::with_capacity(filtered.len() / 2);
    let mut chars = filtered.chars();
    while let (Some(high), Some(low)) = (chars.next(), chars.next()) {
        let pair = format!("{high}{low}");
        let byte = u8::from_str_radix(&pair, 16).map_err(|error| {
            RecordingError::new(format!("failed to decode hex pair {pair:?}: {error}"))
        })?;
        bytes.push(byte);
    }

    Ok(bytes)
}
