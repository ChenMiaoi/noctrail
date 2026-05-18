use std::path::PathBuf;

use noctrail_term::ShellIntegrationEvent;
use serde_json::Value as JsonValue;
use toml::Value as TomlValue;

pub(crate) const MAX_COMMAND_BLOCKS: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CommandBlock {
    pub command: Option<String>,
    pub cwd: Option<PathBuf>,
    pub exit_code: Option<i32>,
    pub duration_ms: Option<u64>,
    pub output: String,
    pub folded: bool,
    pub structured_output: Option<StructuredOutputLens>,
}

impl CommandBlock {
    fn is_empty(&self) -> bool {
        self.command.is_none()
            && self.cwd.is_none()
            && self.exit_code.is_none()
            && self.duration_ms.is_none()
            && self.output.is_empty()
    }

    pub fn failed(&self) -> bool {
        self.exit_code.is_some_and(|exit_code| exit_code != 0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructuredOutputKind {
    Json,
    Csv,
    Toml,
}

impl StructuredOutputKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Csv => "csv",
            Self::Toml => "toml",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuredOutputLens {
    pub kind: StructuredOutputKind,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct CommandBlockObserver {
    enabled: bool,
    current: Option<CommandBlock>,
    completed: Vec<CommandBlock>,
    selected: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum BlockChunkPart {
    Output(String),
    Event(ShellIntegrationEvent),
}

impl CommandBlockObserver {
    pub(crate) fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.current = None;
        }
    }

    pub(crate) fn enabled(&self) -> bool {
        self.enabled
    }

    pub(crate) fn observe_chunk(&mut self, bytes: &[u8]) {
        if !self.enabled {
            return;
        }

        for part in parse_block_chunk(bytes) {
            match part {
                BlockChunkPart::Output(output) => {
                    if let Some(current) = self.current.as_mut() {
                        current.output.push_str(&output);
                    }
                }
                BlockChunkPart::Event(event) => self.observe_event(event),
            }
        }
    }

    pub(crate) fn current(&self) -> Option<&CommandBlock> {
        self.current.as_ref()
    }

    pub(crate) fn completed(&self) -> &[CommandBlock] {
        &self.completed
    }

    pub(crate) fn selected_index(&self) -> Option<usize> {
        self.selected
    }

    pub(crate) fn selected(&self) -> Option<&CommandBlock> {
        self.selected.and_then(|index| self.completed.get(index))
    }

    pub(crate) fn select_oldest(&mut self) -> Option<usize> {
        if self.completed.is_empty() {
            self.selected = None;
        } else {
            self.selected = Some(0);
        }
        self.selected
    }

    pub(crate) fn select_newest(&mut self) -> Option<usize> {
        if self.completed.is_empty() {
            self.selected = None;
        } else {
            self.selected = Some(self.completed.len() - 1);
        }
        self.selected
    }

    pub(crate) fn select_previous(&mut self) -> Option<usize> {
        let len = self.completed.len();
        if len == 0 {
            self.selected = None;
            return None;
        }

        let next = match self.selected {
            Some(0) | None => len - 1,
            Some(index) => index - 1,
        };
        self.selected = Some(next);
        self.selected
    }

    pub(crate) fn select_next(&mut self) -> Option<usize> {
        let len = self.completed.len();
        if len == 0 {
            self.selected = None;
            return None;
        }

        let next = match self.selected {
            Some(index) => (index + 1) % len,
            None => 0,
        };
        self.selected = Some(next);
        self.selected
    }

    pub(crate) fn toggle_selected_fold(&mut self) -> Option<bool> {
        let index = self.selected?;
        let block = self.completed.get_mut(index)?;
        block.folded = !block.folded;
        Some(block.folded)
    }

    pub(crate) fn copy_selected_command(&self) -> Option<String> {
        self.selected()?.command.clone()
    }

    pub(crate) fn copy_selected_output(&self) -> Option<String> {
        let output = self.selected()?.output.clone();
        if output.is_empty() {
            None
        } else {
            Some(output)
        }
    }

    pub(crate) fn copy_selected_structured_output(&self) -> Option<String> {
        let block = self.selected()?;
        if block.structured_output.is_some() && !block.output.is_empty() {
            Some(block.output.clone())
        } else {
            None
        }
    }

    pub(crate) fn failed_count(&self) -> usize {
        self.completed.iter().filter(|block| block.failed()).count()
    }

    pub(crate) fn select_newest_failed(&mut self) -> Option<usize> {
        let index = self.completed.iter().rposition(CommandBlock::failed)?;
        self.selected = Some(index);
        Some(index)
    }

    fn observe_event(&mut self, event: ShellIntegrationEvent) {
        match event {
            ShellIntegrationEvent::Prompt => {}
            ShellIntegrationEvent::CommandStart => {
                self.finish_current();
                self.current = Some(CommandBlock::default());
            }
            ShellIntegrationEvent::CommandText(command) => {
                if let Some(current) = self.current.as_mut() {
                    current.command = Some(command);
                }
            }
            ShellIntegrationEvent::CommandEnd => self.finish_current(),
            ShellIntegrationEvent::Cwd(cwd) => {
                if let Some(current) = self.current.as_mut() {
                    current.cwd = Some(PathBuf::from(cwd));
                }
            }
            ShellIntegrationEvent::ExitCode(exit_code) => {
                if let Some(current) = self.current.as_mut() {
                    current.exit_code = Some(exit_code);
                }
            }
            ShellIntegrationEvent::DurationMs(duration_ms) => {
                if let Some(current) = self.current.as_mut() {
                    current.duration_ms = Some(duration_ms);
                }
            }
        }
    }

    fn finish_current(&mut self) {
        let Some(current) = self.current.take() else {
            return;
        };
        if current.is_empty() {
            return;
        }

        let mut current = current;
        current.structured_output = detect_structured_output(&current.output);
        self.completed.push(current);
        if self.completed.len() > MAX_COMMAND_BLOCKS {
            let overflow = self.completed.len() - MAX_COMMAND_BLOCKS;
            self.completed.drain(0..overflow);
            self.selected = self
                .selected
                .map(|selected| selected.saturating_sub(overflow));
        }
        self.selected = Some(self.completed.len() - 1);
    }
}

fn parse_block_chunk(bytes: &[u8]) -> Vec<BlockChunkPart> {
    let mut parts = Vec::new();
    let mut text = Vec::new();
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == 0x1b {
            if let Some((event, consumed)) = parse_noctrail_osc(&bytes[index..]) {
                flush_block_output(&mut text, &mut parts);
                parts.push(BlockChunkPart::Event(event));
                index += consumed;
                continue;
            }

            if let Some(consumed) = skip_escape_sequence(&bytes[index..]) {
                flush_block_output(&mut text, &mut parts);
                index += consumed;
                continue;
            }
        }

        match bytes[index] {
            b'\r' => {
                if bytes.get(index + 1) == Some(&b'\n') {
                    index += 1;
                }
                text.push(b'\n');
            }
            b'\n' | b'\t' => text.push(bytes[index]),
            0x20..=0x7e => text.push(bytes[index]),
            byte if byte >= 0x80 => text.push(byte),
            _ => {}
        }
        index += 1;
    }

    flush_block_output(&mut text, &mut parts);
    parts
}

fn flush_block_output(text: &mut Vec<u8>, parts: &mut Vec<BlockChunkPart>) {
    if text.is_empty() {
        return;
    }

    let output = String::from_utf8_lossy(text).into_owned();
    if !output.is_empty() {
        parts.push(BlockChunkPart::Output(output));
    }
    text.clear();
}

fn parse_noctrail_osc(bytes: &[u8]) -> Option<(ShellIntegrationEvent, usize)> {
    if bytes.len() < 2 || bytes[0] != 0x1b || bytes[1] != b']' {
        return None;
    }

    let mut index = 2;
    let mut payload_end = None;
    let mut consumed = None;
    while index < bytes.len() {
        match bytes[index] {
            0x07 => {
                payload_end = Some(index);
                consumed = Some(index + 1);
                break;
            }
            0x1b if bytes.get(index + 1) == Some(&b'\\') => {
                payload_end = Some(index);
                consumed = Some(index + 2);
                break;
            }
            _ => index += 1,
        }
    }

    let consumed = consumed?;
    let payload_end = payload_end?;
    let payload = String::from_utf8_lossy(&bytes[2..payload_end]).into_owned();
    let event = parse_noctrail_osc_payload(payload)?;
    Some((event, consumed))
}

fn parse_noctrail_osc_payload(payload: impl AsRef<str>) -> Option<ShellIntegrationEvent> {
    let prefix = "1337;Noctrail;";
    let rest = payload.as_ref().strip_prefix(prefix)?;
    match rest {
        "Prompt" => Some(ShellIntegrationEvent::Prompt),
        "CommandStart" => Some(ShellIntegrationEvent::CommandStart),
        "CommandEnd" => Some(ShellIntegrationEvent::CommandEnd),
        _ => {
            if let Some(value) = rest.strip_prefix("CommandText;") {
                return Some(ShellIntegrationEvent::CommandText(value.to_string()));
            }
            if let Some(value) = rest.strip_prefix("Cwd;") {
                return Some(ShellIntegrationEvent::Cwd(value.to_string()));
            }
            if let Some(value) = rest.strip_prefix("ExitCode;") {
                return value
                    .trim()
                    .parse::<i32>()
                    .ok()
                    .map(ShellIntegrationEvent::ExitCode);
            }
            if let Some(value) = rest.strip_prefix("DurationMs;") {
                return value
                    .trim()
                    .parse::<u64>()
                    .ok()
                    .map(ShellIntegrationEvent::DurationMs);
            }
            None
        }
    }
}

fn skip_escape_sequence(bytes: &[u8]) -> Option<usize> {
    if bytes.len() < 2 || bytes[0] != 0x1b {
        return None;
    }

    match bytes[1] {
        b'[' => {
            let mut index = 2;
            while index < bytes.len() {
                if (0x40..=0x7e).contains(&bytes[index]) {
                    return Some(index + 1);
                }
                index += 1;
            }
            Some(bytes.len())
        }
        b']' => None,
        _ => Some(bytes.len().min(2)),
    }
}

fn detect_structured_output(output: &str) -> Option<StructuredOutputLens> {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return None;
    }

    detect_json_lens(trimmed)
        .or_else(|| detect_toml_lens(trimmed))
        .or_else(|| detect_csv_lens(trimmed))
}

fn detect_json_lens(trimmed: &str) -> Option<StructuredOutputLens> {
    let value = serde_json::from_str::<JsonValue>(trimmed).ok()?;
    let summary = match value {
        JsonValue::Object(map) => format!("json object {} keys", map.len()),
        JsonValue::Array(items) => format!("json array {} items", items.len()),
        JsonValue::String(_) => "json string".to_string(),
        JsonValue::Number(_) => "json number".to_string(),
        JsonValue::Bool(_) => "json boolean".to_string(),
        JsonValue::Null => "json null".to_string(),
    };
    Some(StructuredOutputLens {
        kind: StructuredOutputKind::Json,
        summary,
    })
}

fn detect_toml_lens(trimmed: &str) -> Option<StructuredOutputLens> {
    if !looks_like_toml(trimmed) {
        return None;
    }

    let value = toml::from_str::<TomlValue>(trimmed).ok()?;
    let summary = match value {
        TomlValue::Table(table) => format!("toml table {} keys", table.len()),
        TomlValue::Array(items) => format!("toml array {} items", items.len()),
        TomlValue::String(_) => "toml string".to_string(),
        TomlValue::Integer(_) => "toml integer".to_string(),
        TomlValue::Float(_) => "toml float".to_string(),
        TomlValue::Boolean(_) => "toml boolean".to_string(),
        TomlValue::Datetime(_) => "toml datetime".to_string(),
    };
    Some(StructuredOutputLens {
        kind: StructuredOutputKind::Toml,
        summary,
    })
}

fn looks_like_toml(trimmed: &str) -> bool {
    trimmed.lines().any(|line| {
        let line = line.trim();
        (!line.is_empty() && line.contains('=')) || (line.starts_with('[') && line.ends_with(']'))
    })
}

fn detect_csv_lens(trimmed: &str) -> Option<StructuredOutputLens> {
    let rows = trimmed
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if rows.len() < 2 {
        return None;
    }

    let mut width = None;
    for row in &rows {
        let fields = parse_csv_fields(row)?;
        if fields.len() < 2 {
            return None;
        }
        match width {
            Some(expected) if expected != fields.len() => return None,
            None => width = Some(fields.len()),
            _ => {}
        }
    }

    Some(StructuredOutputLens {
        kind: StructuredOutputKind::Csv,
        summary: format!("csv {} rows x {} cols", rows.len(), width.unwrap_or(0)),
    })
}

fn parse_csv_fields(line: &str) -> Option<Vec<String>> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut chars = line.chars().peekable();
    let mut in_quotes = false;

    while let Some(ch) = chars.next() {
        match ch {
            '"' => {
                if in_quotes {
                    if chars.peek() == Some(&'"') {
                        current.push('"');
                        let _ = chars.next();
                    } else {
                        in_quotes = false;
                    }
                } else if current.is_empty() {
                    in_quotes = true;
                } else {
                    return None;
                }
            }
            ',' if !in_quotes => {
                fields.push(std::mem::take(&mut current));
            }
            _ => current.push(ch),
        }
    }

    if in_quotes {
        return None;
    }

    fields.push(current);
    Some(fields)
}
