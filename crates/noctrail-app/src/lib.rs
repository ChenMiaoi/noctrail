//! Desktop app shell for Noctrail.

use std::{
    collections::HashMap,
    fmt,
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
};

mod clipboard;

pub mod gui;
pub mod input;
pub mod redaction;

use noctrail_agent::{CommandProposal, PatchPreview, ProviderRequestPreview};
use noctrail_layout::{
    FocusDirection, LayoutError, LayoutRect, PaneLayout, SplitAxis, WorkspaceId, WorkspaceSet,
};
use noctrail_pty::{PtyCommand, PtyError, PtyExitStatus, PtySize};
use noctrail_render::{PaneBorderStyle, RenderBackend, RenderInput, RenderPlan, RenderRect};
use noctrail_runtime::{PaneId, PaneRuntime};
use noctrail_term::{
    Cursor, DamageSet, LineEnding, MouseTrackingMode, Position, Selection, SelectionMode,
    ShellIntegrationEvent, TerminalSnapshot, TerminalState,
};
use serde_json::Value as JsonValue;
use thiserror::Error;
use toml::Value as TomlValue;

const ROOT_PANE_ID: PaneId = PaneId::new(1);
const SCRATCH_HEIGHT_DIVISOR: u16 = 3;
const MAX_COMMAND_BLOCKS: usize = 100;
const MAX_AUDIT_ENTRIES: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PaneChromeConfig {
    pub border: PaneBorderStyle,
    pub gap: u16,
    pub padding: u16,
    pub radius: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PaneStatusLine {
    pub shell: Option<String>,
    pub cwd: Option<PathBuf>,
    pub git_branch: Option<String>,
    pub exit_status: Option<String>,
}

impl PaneStatusLine {
    fn from_command(command: &PtyCommand) -> Self {
        let cwd = command.cwd().cloned();

        Self {
            shell: Some(command_shell_label(command)),
            cwd: cwd.clone(),
            git_branch: cwd.as_deref().and_then(detect_git_branch),
            exit_status: None,
        }
    }
}

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentContextBlock {
    pub command: Option<String>,
    pub output: String,
    pub exit_code: Option<i32>,
}

impl From<&CommandBlock> for AgentContextBlock {
    fn from(block: &CommandBlock) -> Self {
        Self {
            command: block.command.clone(),
            output: block.output.clone(),
            exit_code: block.exit_code,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AgentContextPreview {
    pub current_block: Option<AgentContextBlock>,
    pub selection: Option<String>,
    pub cwd: Option<PathBuf>,
    pub explicit_files: Vec<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentAuditKind {
    Context,
    Read,
    Suggest,
    Review,
    Execute,
}

impl AgentAuditKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Context => "context",
            Self::Read => "read",
            Self::Suggest => "suggest",
            Self::Review => "review",
            Self::Execute => "execute",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentAuditEntry {
    pub kind: AgentAuditKind,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct AgentAuditLedger {
    entries: Vec<AgentAuditEntry>,
    selected: Option<usize>,
}

impl AgentAuditLedger {
    fn entries(&self) -> &[AgentAuditEntry] {
        &self.entries
    }

    fn selected_index(&self) -> Option<usize> {
        self.selected
    }

    fn selected(&self) -> Option<&AgentAuditEntry> {
        self.selected.and_then(|index| self.entries.get(index))
    }

    fn push(&mut self, entry: AgentAuditEntry) {
        self.entries.push(entry);
        if self.entries.len() > MAX_AUDIT_ENTRIES {
            let overflow = self.entries.len() - MAX_AUDIT_ENTRIES;
            self.entries.drain(0..overflow);
            self.selected = self
                .selected
                .map(|selected| selected.saturating_sub(overflow));
        }
        self.selected = Some(self.entries.len() - 1);
    }

    fn select_oldest(&mut self) -> Option<usize> {
        self.selected = (!self.entries.is_empty()).then_some(0);
        self.selected
    }

    fn select_newest(&mut self) -> Option<usize> {
        self.selected = self.entries.len().checked_sub(1);
        self.selected
    }

    fn select_previous(&mut self) -> Option<usize> {
        let len = self.entries.len();
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

    fn select_next(&mut self) -> Option<usize> {
        let len = self.entries.len();
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
struct CommandBlockObserver {
    enabled: bool,
    current: Option<CommandBlock>,
    completed: Vec<CommandBlock>,
    selected: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct AgentProposalState {
    proposals: Vec<CommandProposal>,
    selected: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct AgentPatchPreviewState {
    previews: Vec<PatchPreview>,
    selected: Option<usize>,
}

impl AgentPatchPreviewState {
    fn previews(&self) -> &[PatchPreview] {
        &self.previews
    }

    fn selected_index(&self) -> Option<usize> {
        self.selected
    }

    fn selected(&self) -> Option<&PatchPreview> {
        self.selected.and_then(|index| self.previews.get(index))
    }

    fn set_previews(&mut self, previews: Vec<PatchPreview>) {
        self.previews = previews;
        self.selected = (!self.previews.is_empty()).then_some(0);
    }

    fn clear(&mut self) {
        self.previews.clear();
        self.selected = None;
    }

    fn select_oldest(&mut self) -> Option<usize> {
        self.selected = (!self.previews.is_empty()).then_some(0);
        self.selected
    }

    fn select_newest(&mut self) -> Option<usize> {
        self.selected = self.previews.len().checked_sub(1);
        self.selected
    }

    fn select_previous(&mut self) -> Option<usize> {
        let len = self.previews.len();
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

    fn select_next(&mut self) -> Option<usize> {
        let len = self.previews.len();
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
}

impl AgentProposalState {
    fn proposals(&self) -> &[CommandProposal] {
        &self.proposals
    }

    fn selected_index(&self) -> Option<usize> {
        self.selected
    }

    fn selected(&self) -> Option<&CommandProposal> {
        self.selected.and_then(|index| self.proposals.get(index))
    }

    fn set_proposals(&mut self, proposals: Vec<CommandProposal>) {
        self.proposals = proposals;
        self.selected = (!self.proposals.is_empty()).then_some(0);
    }

    fn clear(&mut self) {
        self.proposals.clear();
        self.selected = None;
    }

    fn select_oldest(&mut self) -> Option<usize> {
        self.selected = (!self.proposals.is_empty()).then_some(0);
        self.selected
    }

    fn select_newest(&mut self) -> Option<usize> {
        self.selected = self.proposals.len().checked_sub(1);
        self.selected
    }

    fn select_previous(&mut self) -> Option<usize> {
        let len = self.proposals.len();
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

    fn select_next(&mut self) -> Option<usize> {
        let len = self.proposals.len();
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum BlockChunkPart {
    Output(String),
    Event(ShellIntegrationEvent),
}

impl CommandBlockObserver {
    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.current = None;
        }
    }

    fn observe_chunk(&mut self, bytes: &[u8]) {
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

    fn current(&self) -> Option<&CommandBlock> {
        self.current.as_ref()
    }

    fn completed(&self) -> &[CommandBlock] {
        &self.completed
    }

    fn selected_index(&self) -> Option<usize> {
        self.selected
    }

    fn selected(&self) -> Option<&CommandBlock> {
        self.selected.and_then(|index| self.completed.get(index))
    }

    fn select_oldest(&mut self) -> Option<usize> {
        if self.completed.is_empty() {
            self.selected = None;
        } else {
            self.selected = Some(0);
        }
        self.selected
    }

    fn select_newest(&mut self) -> Option<usize> {
        if self.completed.is_empty() {
            self.selected = None;
        } else {
            self.selected = Some(self.completed.len() - 1);
        }
        self.selected
    }

    fn select_previous(&mut self) -> Option<usize> {
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

    fn select_next(&mut self) -> Option<usize> {
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

    fn toggle_selected_fold(&mut self) -> Option<bool> {
        let index = self.selected?;
        let block = self.completed.get_mut(index)?;
        block.folded = !block.folded;
        Some(block.folded)
    }

    fn copy_selected_command(&self) -> Option<String> {
        self.selected()?.command.clone()
    }

    fn copy_selected_output(&self) -> Option<String> {
        let output = self.selected()?.output.clone();
        if output.is_empty() {
            None
        } else {
            Some(output)
        }
    }

    fn copy_selected_structured_output(&self) -> Option<String> {
        let block = self.selected()?;
        if block.structured_output.is_some() && !block.output.is_empty() {
            Some(block.output.clone())
        } else {
            None
        }
    }

    fn failed_count(&self) -> usize {
        self.completed.iter().filter(|block| block.failed()).count()
    }

    fn select_newest_failed(&mut self) -> Option<usize> {
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

#[derive(Debug, Error)]
pub enum AppError {
    #[error("the active pane does not have a runtime")]
    MissingRuntime,
    #[error("the desktop app does not have an active pane")]
    MissingActivePane,
    #[error("the active pane does not have a selected agent proposal")]
    MissingAgentProposal,
    #[error("cannot close the last remaining pane")]
    CannotCloseLastPane,
    #[error("pane {0:?} was not found")]
    PaneNotFound(PaneId),
    #[error("pane id space exhausted")]
    PaneIdExhausted,
    #[error(transparent)]
    Layout(#[from] LayoutError),
    #[error(transparent)]
    Pty(#[from] PtyError),
}

pub struct TerminalPane {
    pane_id: PaneId,
    terminal: TerminalState,
    runtime: Option<PaneRuntime>,
    terminal_size: PtySize,
    scrollback_offset: usize,
    last_damage: DamageSet,
    status_line: PaneStatusLine,
    block_observer: CommandBlockObserver,
    agent_proposals: AgentProposalState,
    patch_previews: AgentPatchPreviewState,
}

impl fmt::Debug for TerminalPane {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TerminalPane")
            .field("pane_id", &self.pane_id)
            .field("terminal_size", &self.terminal_size)
            .field("runtime_present", &self.runtime.is_some())
            .field("process_id", &self.process_id())
            .finish()
    }
}

impl TerminalPane {
    pub fn new(pane_id: PaneId, terminal_size: PtySize) -> Self {
        let mut terminal = TerminalState::new(
            usize::from(terminal_size.cols),
            usize::from(terminal_size.rows),
        );
        let _ = terminal.grid_mut().take_dirty_rows();

        Self {
            pane_id,
            terminal,
            runtime: None,
            terminal_size,
            scrollback_offset: 0,
            last_damage: full_frame_damage(terminal_size),
            status_line: PaneStatusLine::default(),
            block_observer: CommandBlockObserver::default(),
            agent_proposals: AgentProposalState::default(),
            patch_previews: AgentPatchPreviewState::default(),
        }
    }

    pub fn spawn(
        pane_id: PaneId,
        command: PtyCommand,
        terminal_size: PtySize,
    ) -> Result<Self, AppError> {
        let status_line = PaneStatusLine::from_command(&command);
        let runtime = PaneRuntime::spawn(command, terminal_size)?;
        let mut terminal = TerminalState::new(
            usize::from(terminal_size.cols),
            usize::from(terminal_size.rows),
        );
        let _ = terminal.grid_mut().take_dirty_rows();

        Ok(Self {
            pane_id,
            terminal,
            runtime: Some(runtime),
            terminal_size,
            scrollback_offset: 0,
            last_damage: full_frame_damage(terminal_size),
            status_line,
            block_observer: CommandBlockObserver::default(),
            agent_proposals: AgentProposalState::default(),
            patch_previews: AgentPatchPreviewState::default(),
        })
    }

    pub fn spawn_shell(pane_id: PaneId, terminal_size: PtySize) -> Result<Self, AppError> {
        Self::spawn(pane_id, PtyCommand::shell(), terminal_size)
    }

    pub fn pane_id(&self) -> PaneId {
        self.pane_id
    }

    pub fn terminal(&self) -> &TerminalState {
        &self.terminal
    }

    pub fn terminal_mut(&mut self) -> &mut TerminalState {
        &mut self.terminal
    }

    pub fn runtime(&self) -> Option<&PaneRuntime> {
        self.runtime.as_ref()
    }

    pub fn runtime_mut(&mut self) -> Option<&mut PaneRuntime> {
        self.runtime.as_mut()
    }

    pub fn runtime_present(&self) -> bool {
        self.runtime.is_some()
    }

    pub fn terminal_size(&self) -> PtySize {
        self.terminal_size
    }

    pub fn bracketed_paste_enabled(&self) -> bool {
        self.terminal.bracketed_paste_mode()
    }

    pub fn mouse_tracking_mode(&self) -> MouseTrackingMode {
        self.terminal.mouse_tracking_mode()
    }

    pub fn mouse_reporting_enabled(&self) -> bool {
        self.terminal.mouse_reporting_enabled()
    }

    pub fn sgr_mouse_mode(&self) -> bool {
        self.terminal.sgr_mouse_mode()
    }

    pub fn copy_selection_text(&self) -> Option<String> {
        self.terminal.selection_text(selection_line_ending())
    }

    pub fn process_id(&self) -> Option<u32> {
        self.runtime.as_ref().and_then(PaneRuntime::process_id)
    }

    pub fn status_line(&self) -> &PaneStatusLine {
        &self.status_line
    }

    pub fn set_block_observer_enabled(&mut self, enabled: bool) {
        self.block_observer.set_enabled(enabled);
    }

    pub fn block_observer_enabled(&self) -> bool {
        self.block_observer.enabled
    }

    pub fn current_command_block(&self) -> Option<&CommandBlock> {
        self.block_observer.current()
    }

    fn context_command_block(&self) -> Option<&CommandBlock> {
        self.selected_command_block()
            .or_else(|| self.current_command_block())
            .or_else(|| self.command_blocks().last())
    }

    pub fn command_blocks(&self) -> &[CommandBlock] {
        self.block_observer.completed()
    }

    pub fn selected_command_block_index(&self) -> Option<usize> {
        self.block_observer.selected_index()
    }

    pub fn selected_command_block(&self) -> Option<&CommandBlock> {
        self.block_observer.selected()
    }

    pub fn select_oldest_command_block(&mut self) -> Option<usize> {
        self.block_observer.select_oldest()
    }

    pub fn select_newest_command_block(&mut self) -> Option<usize> {
        self.block_observer.select_newest()
    }

    pub fn select_previous_command_block(&mut self) -> Option<usize> {
        self.block_observer.select_previous()
    }

    pub fn select_next_command_block(&mut self) -> Option<usize> {
        self.block_observer.select_next()
    }

    pub fn toggle_selected_command_block_fold(&mut self) -> Option<bool> {
        self.block_observer.toggle_selected_fold()
    }

    pub fn copy_selected_command_block_command(&self) -> Option<String> {
        self.block_observer.copy_selected_command()
    }

    pub fn copy_selected_command_block_output(&self) -> Option<String> {
        self.block_observer.copy_selected_output()
    }

    pub fn copy_selected_command_block_structured_output(&self) -> Option<String> {
        self.block_observer.copy_selected_structured_output()
    }

    pub fn failed_command_blocks_count(&self) -> usize {
        self.block_observer.failed_count()
    }

    pub fn select_newest_failed_command_block(&mut self) -> Option<usize> {
        self.block_observer.select_newest_failed()
    }

    pub fn paste_bytes(&self, text: &str) -> Vec<u8> {
        input::paste_bytes(text, self.bracketed_paste_enabled())
    }

    pub fn agent_context_preview(&self, explicit_files: &[PathBuf]) -> AgentContextPreview {
        let current_block = self.context_command_block().map(AgentContextBlock::from);
        let cwd = self.status_line.cwd.clone().or_else(|| {
            self.context_command_block()
                .and_then(|block| block.cwd.clone())
        });

        AgentContextPreview {
            current_block,
            selection: self.copy_selection_text(),
            cwd,
            explicit_files: explicit_files.to_vec(),
        }
    }

    pub fn agent_command_proposals(&self) -> &[CommandProposal] {
        self.agent_proposals.proposals()
    }

    pub fn selected_agent_command_proposal_index(&self) -> Option<usize> {
        self.agent_proposals.selected_index()
    }

    pub fn selected_agent_command_proposal(&self) -> Option<&CommandProposal> {
        self.agent_proposals.selected()
    }

    pub fn set_agent_command_proposals(&mut self, proposals: Vec<CommandProposal>) {
        self.agent_proposals.set_proposals(proposals);
    }

    pub fn clear_agent_command_proposals(&mut self) {
        self.agent_proposals.clear();
    }

    pub fn agent_patch_previews(&self) -> &[PatchPreview] {
        self.patch_previews.previews()
    }

    pub fn selected_agent_patch_preview_index(&self) -> Option<usize> {
        self.patch_previews.selected_index()
    }

    pub fn selected_agent_patch_preview(&self) -> Option<&PatchPreview> {
        self.patch_previews.selected()
    }

    pub fn set_agent_patch_previews(&mut self, previews: Vec<PatchPreview>) {
        self.patch_previews.set_previews(previews);
    }

    pub fn clear_agent_patch_previews(&mut self) {
        self.patch_previews.clear();
    }

    pub fn select_oldest_agent_patch_preview(&mut self) -> Option<usize> {
        self.patch_previews.select_oldest()
    }

    pub fn select_newest_agent_patch_preview(&mut self) -> Option<usize> {
        self.patch_previews.select_newest()
    }

    pub fn select_previous_agent_patch_preview(&mut self) -> Option<usize> {
        self.patch_previews.select_previous()
    }

    pub fn select_next_agent_patch_preview(&mut self) -> Option<usize> {
        self.patch_previews.select_next()
    }

    pub fn select_oldest_agent_command_proposal(&mut self) -> Option<usize> {
        self.agent_proposals.select_oldest()
    }

    pub fn select_newest_agent_command_proposal(&mut self) -> Option<usize> {
        self.agent_proposals.select_newest()
    }

    pub fn select_previous_agent_command_proposal(&mut self) -> Option<usize> {
        self.agent_proposals.select_previous()
    }

    pub fn select_next_agent_command_proposal(&mut self) -> Option<usize> {
        self.agent_proposals.select_next()
    }

    pub fn submit_selected_agent_command_proposal(&mut self) -> Result<usize, AppError> {
        let command = self
            .selected_agent_command_proposal()
            .map(|proposal| proposal.command.clone())
            .ok_or(AppError::MissingAgentProposal)?;
        self.write_input(proposal_submission_bytes(&command).as_slice())
    }

    pub fn advance_output(&mut self, bytes: &[u8]) {
        self.last_damage = self.terminal.advance_bytes(bytes).damage;
        // Keep the terminal core authoritative for rendering while the block
        // observer reparses the shell-integration chunk to preserve output and
        // marker ordering for command block history.
        let _ = self.terminal.drain_shell_integration_events();
        self.block_observer.observe_chunk(bytes);
        self.clamp_scrollback_offset();
    }

    pub fn write_input(&mut self, bytes: &[u8]) -> Result<usize, AppError> {
        let runtime = self.runtime.as_mut().ok_or(AppError::MissingRuntime)?;
        runtime.write(bytes).map_err(AppError::from)
    }

    pub fn paste_text(&mut self, text: &str) -> Result<usize, AppError> {
        let bytes = self.paste_bytes(text);
        self.write_input(&bytes)
    }

    pub fn resize(&mut self, size: PtySize) -> Result<(), AppError> {
        if let Some(runtime) = self.runtime.as_mut() {
            runtime.resize(size)?;
        }

        self.terminal
            .resize(usize::from(size.cols), usize::from(size.rows));
        self.terminal_size = size;
        self.clamp_scrollback_offset();
        self.last_damage = full_frame_damage(size);
        let _ = self.terminal.grid_mut().take_dirty_rows();
        Ok(())
    }

    pub fn snapshot(&self) -> TerminalSnapshot {
        self.terminal.snapshot()
    }

    pub fn scrollback_offset(&self) -> usize {
        self.scrollback_offset
    }

    pub fn scroll_scrollback(&mut self, delta_lines: i32) {
        let snapshot = self.snapshot();
        let max_offset = max_scrollback_offset(&snapshot);
        let next_offset = if delta_lines >= 0 {
            self.scrollback_offset
                .saturating_add(delta_lines as usize)
                .min(max_offset)
        } else {
            self.scrollback_offset
                .saturating_sub(delta_lines.unsigned_abs() as usize)
        };

        if next_offset != self.scrollback_offset {
            self.scrollback_offset = next_offset;
            self.last_damage = full_frame_damage(self.terminal_size);
        }
    }

    pub fn clear_selection(&mut self) {
        if self.terminal.selection().is_some() {
            self.terminal.clear_selection();
            self.last_damage = full_frame_damage(self.terminal_size);
        }
    }

    pub fn select_viewport_range(&mut self, start: Position, end: Position, mode: SelectionMode) {
        let Some(selection) = self.viewport_selection(start, end, mode) else {
            self.clear_selection();
            return;
        };

        self.terminal.set_selection(Some(selection));
        self.last_damage = full_frame_damage(self.terminal_size);
    }

    pub fn render_plan(
        &self,
        pane_surface: LayoutRect,
        content_surface: LayoutRect,
        backend: RenderBackend,
        active: bool,
        chrome: PaneChromeConfig,
    ) -> RenderPlan {
        let snapshot = self.render_snapshot();
        RenderPlan::from_input(RenderInput {
            pane_rect: RenderRect::new(
                usize::from(pane_surface.x),
                usize::from(pane_surface.y),
                usize::from(pane_surface.width),
                usize::from(pane_surface.height),
            ),
            viewport: RenderRect::new(
                usize::from(content_surface.x),
                usize::from(content_surface.y),
                usize::from(content_surface.width),
                usize::from(content_surface.height),
            ),
            backend,
            snapshot: &snapshot,
            damage: &self.last_damage,
            active,
            border: chrome.border,
            corner_radius: usize::from(chrome.radius),
        })
    }

    pub fn invalidate_full_frame(&mut self) {
        self.last_damage = full_frame_damage(self.terminal_size);
    }

    pub fn refresh_exit_status(&mut self) -> Result<bool, AppError> {
        if self.status_line.exit_status.is_some() {
            return Ok(false);
        }

        let Some(runtime) = self.runtime.as_mut() else {
            return Ok(false);
        };
        let Some(status) = runtime.try_wait()? else {
            return Ok(false);
        };

        Ok(self.record_exit_status(&status))
    }

    pub fn close_runtime(&mut self) -> Result<Option<PtyExitStatus>, AppError> {
        let runtime = self.runtime.take().ok_or(AppError::MissingRuntime)?;
        let status = runtime.close().map_err(AppError::from)?;
        if let Some(status) = status.as_ref() {
            self.record_exit_status(status);
        }
        Ok(status)
    }

    fn clamp_scrollback_offset(&mut self) {
        let snapshot = self.snapshot();
        self.scrollback_offset = self.scrollback_offset.min(max_scrollback_offset(&snapshot));
    }

    fn record_exit_status(&mut self, status: &PtyExitStatus) -> bool {
        let next = format_exit_status(status);
        if self.status_line.exit_status.as_deref() == Some(next.as_str()) {
            return false;
        }

        self.status_line.exit_status = Some(next);
        true
    }

    fn render_snapshot(&self) -> TerminalSnapshot {
        let snapshot = self.snapshot();
        let scrollback_offset = self.scrollback_offset.min(max_scrollback_offset(&snapshot));
        if scrollback_offset == 0 || snapshot.alternate_screen {
            return snapshot;
        }

        let all_rows = collect_all_rows(&snapshot);
        let visible_range = visible_row_range(
            &snapshot,
            usize::from(self.terminal_size.rows),
            scrollback_offset,
        );
        let cursor = remap_cursor(snapshot.cursor, snapshot.scrollback.len(), &visible_range);
        let selection = snapshot
            .selection
            .as_ref()
            .and_then(|selection| remap_selection(selection, &visible_range));

        TerminalSnapshot {
            rows: all_rows[visible_range.start..visible_range.end].to_vec(),
            scrollback: all_rows[..visible_range.start].to_vec(),
            cursor,
            alternate_screen: snapshot.alternate_screen,
            bracketed_paste: snapshot.bracketed_paste,
            selection,
        }
    }

    fn viewport_selection(
        &self,
        start: Position,
        end: Position,
        mode: SelectionMode,
    ) -> Option<Selection> {
        let snapshot = self.snapshot();
        let visible_range = visible_row_range(
            &snapshot,
            usize::from(self.terminal_size.rows),
            self.scrollback_offset,
        );
        if visible_range.is_empty() {
            return None;
        }

        Some(Selection {
            mode,
            start: viewport_to_terminal_position(start, &visible_range, self.terminal_size),
            end: viewport_to_terminal_position(end, &visible_range, self.terminal_size),
        })
    }
}

#[derive(Debug)]
pub struct DesktopFrame {
    pub workspace_id: WorkspaceId,
    pub is_scratch: bool,
    pub pane_id: PaneId,
    pub pane_surface: LayoutRect,
    pub surface: LayoutRect,
    pub terminal_size: PtySize,
    pub process_id: Option<u32>,
    pub status_line: PaneStatusLine,
    pub render_plan: RenderPlan,
}

#[derive(Debug)]
pub struct DesktopApp {
    surface: LayoutRect,
    terminal_size: PtySize,
    backend: RenderBackend,
    pane_chrome: PaneChromeConfig,
    workspaces: WorkspaceSet,
    scratch_pane_id: Option<PaneId>,
    scratch_visible: bool,
    explicit_agent_files: Vec<PathBuf>,
    audit_ledger: AgentAuditLedger,
    panes: HashMap<PaneId, TerminalPane>,
    next_pane_id: u64,
}

impl DesktopApp {
    pub fn new(surface: LayoutRect, terminal_size: PtySize) -> Self {
        Self::from_root_pane(
            surface,
            terminal_size,
            TerminalPane::new(ROOT_PANE_ID, terminal_size),
        )
    }

    pub fn spawn_shell(surface: LayoutRect, terminal_size: PtySize) -> Result<Self, AppError> {
        Ok(Self::from_root_pane(
            surface,
            terminal_size,
            TerminalPane::spawn_shell(ROOT_PANE_ID, terminal_size)?,
        ))
    }

    pub fn spawn(
        surface: LayoutRect,
        command: PtyCommand,
        terminal_size: PtySize,
    ) -> Result<Self, AppError> {
        Ok(Self::from_root_pane(
            surface,
            terminal_size,
            TerminalPane::spawn(ROOT_PANE_ID, command, terminal_size)?,
        ))
    }

    pub fn backend(&self) -> RenderBackend {
        self.backend
    }

    pub fn set_backend(&mut self, backend: RenderBackend) {
        self.backend = backend;
    }

    pub fn pane_chrome(&self) -> PaneChromeConfig {
        self.pane_chrome
    }

    pub fn set_pane_chrome(&mut self, pane_chrome: PaneChromeConfig) -> Result<(), AppError> {
        if self.pane_chrome == pane_chrome {
            return Ok(());
        }

        self.pane_chrome = pane_chrome;
        self.sync_pane_terminal_sizes()?;
        self.invalidate_visuals();
        Ok(())
    }

    pub fn surface(&self) -> LayoutRect {
        self.surface
    }

    pub fn active_pane_id(&self) -> Option<PaneId> {
        if self.scratch_visible {
            self.scratch_pane_id
        } else {
            self.workspaces.active_layout().active_pane()
        }
    }

    pub fn active_workspace_id(&self) -> WorkspaceId {
        self.workspaces.active_workspace()
    }

    pub fn pane_count(&self) -> usize {
        self.panes.len()
    }

    pub fn pane_layouts(&self) -> Vec<PaneLayout> {
        self.workspaces.active_layout().arrange(self.surface)
    }

    pub fn workspace_ids(&self) -> Vec<WorkspaceId> {
        self.workspaces.workspace_ids()
    }

    pub fn scratch_visible(&self) -> bool {
        self.scratch_visible
    }

    pub fn scratch_pane_id(&self) -> Option<PaneId> {
        self.scratch_pane_id
    }

    pub fn pane(&self) -> &TerminalPane {
        self.active_pane_ref()
    }

    pub fn pane_mut(&mut self) -> &mut TerminalPane {
        self.active_pane_mut()
    }

    pub fn set_block_observer_enabled(&mut self, enabled: bool) {
        self.active_pane_mut().set_block_observer_enabled(enabled);
    }

    pub fn block_observer_enabled(&self) -> bool {
        self.active_pane_ref().block_observer_enabled()
    }

    pub fn command_blocks(&self) -> &[CommandBlock] {
        self.active_pane_ref().command_blocks()
    }

    pub fn selected_command_block_index(&self) -> Option<usize> {
        self.active_pane_ref().selected_command_block_index()
    }

    pub fn selected_command_block(&self) -> Option<&CommandBlock> {
        self.active_pane_ref().selected_command_block()
    }

    pub fn select_oldest_command_block(&mut self) -> Option<usize> {
        self.active_pane_mut().select_oldest_command_block()
    }

    pub fn select_newest_command_block(&mut self) -> Option<usize> {
        self.active_pane_mut().select_newest_command_block()
    }

    pub fn select_previous_command_block(&mut self) -> Option<usize> {
        self.active_pane_mut().select_previous_command_block()
    }

    pub fn select_next_command_block(&mut self) -> Option<usize> {
        self.active_pane_mut().select_next_command_block()
    }

    pub fn toggle_selected_command_block_fold(&mut self) -> Option<bool> {
        self.active_pane_mut().toggle_selected_command_block_fold()
    }

    pub fn copy_selected_command_block_command(&self) -> Option<String> {
        self.active_pane_ref().copy_selected_command_block_command()
    }

    pub fn copy_selected_command_block_output(&self) -> Option<String> {
        self.active_pane_ref().copy_selected_command_block_output()
    }

    pub fn copy_selected_command_block_structured_output(&self) -> Option<String> {
        self.active_pane_ref()
            .copy_selected_command_block_structured_output()
    }

    pub fn failed_command_blocks_count(&self) -> usize {
        self.active_pane_ref().failed_command_blocks_count()
    }

    pub fn select_newest_failed_command_block(&mut self) -> Option<usize> {
        self.active_pane_mut().select_newest_failed_command_block()
    }

    pub fn pane_by_id(&self, pane_id: PaneId) -> Option<&TerminalPane> {
        self.panes.get(&pane_id)
    }

    pub fn pane_mut_by_id(&mut self, pane_id: PaneId) -> Option<&mut TerminalPane> {
        self.panes.get_mut(&pane_id)
    }

    pub fn focus_direction(&mut self, direction: FocusDirection) -> Result<PaneId, AppError> {
        Ok(self
            .workspaces
            .active_layout_mut()
            .focus_direction(direction, self.surface)?)
    }

    pub fn swap_active_pane(&mut self, direction: FocusDirection) -> Result<PaneId, AppError> {
        Ok(self
            .workspaces
            .active_layout_mut()
            .swap_active(direction, self.surface)?)
    }

    pub fn resize_active_split(
        &mut self,
        direction: FocusDirection,
        delta: u16,
    ) -> Result<(), AppError> {
        self.workspaces
            .active_layout_mut()
            .resize_active(direction, delta, self.surface)?;
        self.sync_pane_terminal_sizes()
    }

    pub fn split_active_pane_shell(&mut self) -> Result<PaneId, AppError> {
        self.split_active_pane_with(PtyCommand::shell())
    }

    pub fn split_active_pane_shell_with_axis(
        &mut self,
        axis: SplitAxis,
    ) -> Result<PaneId, AppError> {
        self.split_active_pane_with_axis(PtyCommand::shell(), axis)
    }

    pub fn split_active_pane_with(&mut self, command: PtyCommand) -> Result<PaneId, AppError> {
        let new_pane_id = self.allocate_pane_id()?;
        let terminal_size = self.active_pane_ref().terminal_size();
        let pane = TerminalPane::spawn(new_pane_id, command, terminal_size)?;

        self.workspaces
            .active_layout_mut()
            .split_active(new_pane_id, self.surface)?;
        self.panes.insert(new_pane_id, pane);
        self.sync_pane_terminal_sizes()?;
        Ok(new_pane_id)
    }

    pub fn split_active_pane_with_axis(
        &mut self,
        command: PtyCommand,
        axis: SplitAxis,
    ) -> Result<PaneId, AppError> {
        let new_pane_id = self.allocate_pane_id()?;
        let terminal_size = self.active_pane_ref().terminal_size();
        let pane = TerminalPane::spawn(new_pane_id, command, terminal_size)?;

        self.workspaces
            .active_layout_mut()
            .split_active_with_axis(new_pane_id, axis)?;
        self.panes.insert(new_pane_id, pane);
        self.sync_pane_terminal_sizes()?;
        Ok(new_pane_id)
    }

    pub fn advance_output(&mut self, bytes: &[u8]) {
        self.active_pane_mut().advance_output(bytes);
    }

    pub fn write_input(&mut self, bytes: &[u8]) -> Result<usize, AppError> {
        self.active_pane_mut().write_input(bytes)
    }

    pub fn paste_text(&mut self, text: &str) -> Result<usize, AppError> {
        self.active_pane_mut().paste_text(text)
    }

    pub fn copy_selection_text(&self) -> Option<String> {
        self.active_pane_ref().copy_selection_text()
    }

    pub fn agent_context_preview(&self) -> AgentContextPreview {
        self.active_pane_ref()
            .agent_context_preview(&self.explicit_agent_files)
    }

    pub fn set_agent_explicit_files(&mut self, files: Vec<PathBuf>) {
        self.explicit_agent_files = files;
    }

    pub fn agent_explicit_files(&self) -> &[PathBuf] {
        &self.explicit_agent_files
    }

    pub fn agent_command_proposals(&self) -> &[CommandProposal] {
        self.active_pane_ref().agent_command_proposals()
    }

    pub fn selected_agent_command_proposal_index(&self) -> Option<usize> {
        self.active_pane_ref()
            .selected_agent_command_proposal_index()
    }

    pub fn selected_agent_command_proposal(&self) -> Option<&CommandProposal> {
        self.active_pane_ref().selected_agent_command_proposal()
    }

    pub fn set_agent_command_proposals(&mut self, proposals: Vec<CommandProposal>) {
        self.record_agent_command_suggestions(&proposals);
        self.active_pane_mut()
            .set_agent_command_proposals(proposals);
    }

    pub fn clear_agent_command_proposals(&mut self) {
        self.active_pane_mut().clear_agent_command_proposals();
    }

    pub fn select_oldest_agent_command_proposal(&mut self) -> Option<usize> {
        self.active_pane_mut()
            .select_oldest_agent_command_proposal()
    }

    pub fn select_newest_agent_command_proposal(&mut self) -> Option<usize> {
        self.active_pane_mut()
            .select_newest_agent_command_proposal()
    }

    pub fn select_previous_agent_command_proposal(&mut self) -> Option<usize> {
        self.active_pane_mut()
            .select_previous_agent_command_proposal()
    }

    pub fn select_next_agent_command_proposal(&mut self) -> Option<usize> {
        self.active_pane_mut().select_next_agent_command_proposal()
    }

    pub fn submit_selected_agent_command_proposal(&mut self) -> Result<usize, AppError> {
        let command = self
            .selected_agent_command_proposal()
            .map(|proposal| proposal.command.clone())
            .ok_or(AppError::MissingAgentProposal)?;
        let written = self
            .active_pane_mut()
            .submit_selected_agent_command_proposal()?;
        self.record_agent_execute(&command);
        Ok(written)
    }

    pub fn agent_patch_previews(&self) -> &[PatchPreview] {
        self.active_pane_ref().agent_patch_previews()
    }

    pub fn selected_agent_patch_preview_index(&self) -> Option<usize> {
        self.active_pane_ref().selected_agent_patch_preview_index()
    }

    pub fn selected_agent_patch_preview(&self) -> Option<&PatchPreview> {
        self.active_pane_ref().selected_agent_patch_preview()
    }

    pub fn set_agent_patch_previews(&mut self, previews: Vec<PatchPreview>) {
        self.record_agent_patch_suggestions(&previews);
        self.active_pane_mut().set_agent_patch_previews(previews);
    }

    pub fn clear_agent_patch_previews(&mut self) {
        self.active_pane_mut().clear_agent_patch_previews();
    }

    pub fn select_oldest_agent_patch_preview(&mut self) -> Option<usize> {
        self.active_pane_mut().select_oldest_agent_patch_preview()
    }

    pub fn select_newest_agent_patch_preview(&mut self) -> Option<usize> {
        self.active_pane_mut().select_newest_agent_patch_preview()
    }

    pub fn select_previous_agent_patch_preview(&mut self) -> Option<usize> {
        self.active_pane_mut().select_previous_agent_patch_preview()
    }

    pub fn select_next_agent_patch_preview(&mut self) -> Option<usize> {
        self.active_pane_mut().select_next_agent_patch_preview()
    }

    pub fn agent_audit_entries(&self) -> &[AgentAuditEntry] {
        self.audit_ledger.entries()
    }

    pub fn selected_agent_audit_entry_index(&self) -> Option<usize> {
        self.audit_ledger.selected_index()
    }

    pub fn selected_agent_audit_entry(&self) -> Option<&AgentAuditEntry> {
        self.audit_ledger.selected()
    }

    pub fn select_oldest_agent_audit_entry(&mut self) -> Option<usize> {
        self.audit_ledger.select_oldest()
    }

    pub fn select_newest_agent_audit_entry(&mut self) -> Option<usize> {
        self.audit_ledger.select_newest()
    }

    pub fn select_previous_agent_audit_entry(&mut self) -> Option<usize> {
        self.audit_ledger.select_previous()
    }

    pub fn select_next_agent_audit_entry(&mut self) -> Option<usize> {
        self.audit_ledger.select_next()
    }

    pub fn record_agent_context_access(&mut self, preview: &AgentContextPreview) {
        let preview = crate::redaction::redact_agent_context_preview(preview);
        let command = preview
            .current_block
            .as_ref()
            .and_then(|block| block.command.as_deref())
            .unwrap_or("none");
        let cwd = preview
            .cwd
            .as_deref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "none".to_string());
        self.record_agent_audit(
            AgentAuditKind::Context,
            format!("cwd={cwd} command={command}"),
        );
    }

    pub fn record_agent_read(&mut self, preview: &ProviderRequestPreview) {
        let target = preview
            .endpoint
            .as_deref()
            .or(preview.model.as_deref())
            .unwrap_or("cli");
        self.record_agent_audit(
            AgentAuditKind::Read,
            format!(
                "provider={} target={} prompt_chars={}",
                preview.kind, target, preview.prompt_chars
            ),
        );
    }

    pub fn record_agent_command_suggestions(&mut self, proposals: &[CommandProposal]) {
        let first = proposals
            .first()
            .map(|proposal| preview_agent_summary(&proposal.command))
            .unwrap_or_else(|| "none".to_string());
        self.record_agent_audit(
            AgentAuditKind::Suggest,
            format!("commands={} first={first}", proposals.len()),
        );
    }

    pub fn record_agent_patch_suggestions(&mut self, previews: &[PatchPreview]) {
        let first = previews
            .first()
            .map(|preview| preview.path.display().to_string())
            .unwrap_or_else(|| "none".to_string());
        self.record_agent_audit(
            AgentAuditKind::Suggest,
            format!("patches={} first={first}", previews.len()),
        );
    }

    pub fn record_agent_review(&mut self, summary: impl Into<String>) {
        self.record_agent_audit(
            AgentAuditKind::Review,
            preview_agent_summary(&summary.into()),
        );
    }

    pub fn record_agent_execute(&mut self, command: &str) {
        self.record_agent_audit(AgentAuditKind::Execute, preview_agent_summary(command));
    }

    pub fn mouse_tracking_mode(&self) -> MouseTrackingMode {
        self.active_pane_ref().mouse_tracking_mode()
    }

    pub fn mouse_reporting_enabled(&self) -> bool {
        self.active_pane_ref().mouse_reporting_enabled()
    }

    pub fn sgr_mouse_mode(&self) -> bool {
        self.active_pane_ref().sgr_mouse_mode()
    }

    pub fn resize(&mut self, surface: LayoutRect, terminal_size: PtySize) -> Result<(), AppError> {
        self.surface = surface;
        self.terminal_size = terminal_size;
        self.sync_pane_terminal_sizes()
    }

    pub fn scroll_scrollback(&mut self, delta_lines: i32) {
        self.active_pane_mut().scroll_scrollback(delta_lines);
    }

    pub fn select_viewport_range(&mut self, start: Position, end: Position, mode: SelectionMode) {
        self.active_pane_mut()
            .select_viewport_range(start, end, mode);
    }

    pub fn clear_selection(&mut self) {
        self.active_pane_mut().clear_selection();
    }

    pub fn toggle_scratch(&mut self) -> Result<PaneId, AppError> {
        if self.scratch_visible {
            self.scratch_visible = false;
            return self.active_workspace_pane_id();
        }

        let pane_id = if let Some(pane_id) = self.scratch_pane_id {
            pane_id
        } else {
            let pane_id = self.allocate_pane_id()?;
            let pane = TerminalPane::spawn_shell(
                pane_id,
                scratch_terminal_size(self.surface, self.terminal_size, self.pane_chrome),
            )?;
            self.panes.insert(pane_id, pane);
            self.scratch_pane_id = Some(pane_id);
            pane_id
        };

        self.scratch_visible = true;
        self.sync_pane_terminal_sizes()?;
        Ok(pane_id)
    }

    pub fn switch_workspace(&mut self, workspace_id: WorkspaceId) -> Result<PaneId, AppError> {
        self.workspaces.switch_to(workspace_id);
        let active = if let Some(pane_id) = self.active_pane_id() {
            pane_id
        } else {
            let pane_id = self.allocate_pane_id()?;
            let pane = TerminalPane::spawn_shell(
                pane_id,
                pane_terminal_size(
                    self.surface,
                    self.terminal_size,
                    pane_content_surface(self.surface, self.pane_chrome),
                ),
            )?;
            self.workspaces.active_layout_mut().insert_root(pane_id)?;
            self.panes.insert(pane_id, pane);
            pane_id
        };

        self.sync_pane_terminal_sizes()?;
        Ok(active)
    }

    pub fn frame(&self) -> DesktopFrame {
        let pane_id = self
            .active_pane_id()
            .expect("desktop app should always have an active pane");
        self.frame_for_pane(pane_id)
            .expect("active pane should exist in the pane registry")
    }

    pub fn frame_for_pane(&self, pane_id: PaneId) -> Result<DesktopFrame, AppError> {
        let active_pane = self.active_pane_id().ok_or(AppError::MissingActivePane)?;
        let workspace_id = self.active_workspace_id();
        let pane = self
            .pane_by_id(pane_id)
            .ok_or(AppError::PaneNotFound(pane_id))?;
        let is_scratch = self.scratch_pane_id == Some(pane_id);
        let pane_surface = if is_scratch {
            scratch_surface(self.surface)
        } else {
            self.pane_layouts()
                .into_iter()
                .find(|layout| layout.pane_id == pane_id)
                .map(|layout| layout.rect)
                .ok_or(AppError::PaneNotFound(pane_id))?
        };
        let content_surface = pane_content_surface(pane_surface, self.pane_chrome);

        Ok(DesktopFrame {
            workspace_id,
            is_scratch,
            pane_id,
            pane_surface,
            surface: content_surface,
            terminal_size: pane.terminal_size(),
            process_id: pane.process_id(),
            status_line: pane.status_line().clone(),
            render_plan: pane.render_plan(
                pane_surface,
                content_surface,
                self.backend,
                pane_id == active_pane,
                self.pane_chrome,
            ),
        })
    }

    pub fn close_runtime(&mut self) -> Result<Option<PtyExitStatus>, AppError> {
        self.active_pane_mut().close_runtime()
    }

    pub fn refresh_runtime_statuses(&mut self) -> Result<bool, AppError> {
        let mut changed = false;
        for pane in self.panes.values_mut() {
            changed |= pane.refresh_exit_status()?;
        }
        Ok(changed)
    }

    pub fn invalidate_visuals(&mut self) {
        for pane in self.panes.values_mut() {
            pane.invalidate_full_frame();
        }
    }

    pub fn close_active_pane(&mut self) -> Result<(PaneId, Option<PtyExitStatus>), AppError> {
        if self.scratch_visible {
            let scratch_pane_id = self.scratch_pane_id.ok_or(AppError::MissingActivePane)?;
            let status = if self
                .pane_by_id(scratch_pane_id)
                .ok_or(AppError::PaneNotFound(scratch_pane_id))?
                .runtime_present()
            {
                self.pane_mut_by_id(scratch_pane_id)
                    .ok_or(AppError::PaneNotFound(scratch_pane_id))?
                    .close_runtime()?
            } else {
                None
            };

            self.panes.remove(&scratch_pane_id);
            self.scratch_pane_id = None;
            self.scratch_visible = false;
            return Ok((self.active_workspace_pane_id()?, status));
        }

        if self.pane_count() <= 1 {
            return Err(AppError::CannotCloseLastPane);
        }

        if self.workspaces.active_layout().pane_count() <= 1 {
            return Err(AppError::CannotCloseLastPane);
        }

        let active = self.active_pane_id().ok_or(AppError::MissingActivePane)?;
        let status = if self
            .pane_by_id(active)
            .ok_or(AppError::PaneNotFound(active))?
            .runtime_present()
        {
            self.pane_mut_by_id(active)
                .ok_or(AppError::PaneNotFound(active))?
                .close_runtime()?
        } else {
            None
        };

        let next_active = self
            .workspaces
            .active_layout_mut()
            .close(active)?
            .ok_or(AppError::MissingActivePane)?;
        self.panes.remove(&active);
        self.sync_pane_terminal_sizes()?;
        Ok((next_active, status))
    }

    fn from_root_pane(surface: LayoutRect, terminal_size: PtySize, pane: TerminalPane) -> Self {
        let mut panes = HashMap::new();
        panes.insert(ROOT_PANE_ID, pane);
        Self {
            surface,
            terminal_size,
            backend: RenderBackend::default(),
            pane_chrome: PaneChromeConfig::default(),
            workspaces: WorkspaceSet::new(ROOT_PANE_ID),
            scratch_pane_id: None,
            scratch_visible: false,
            explicit_agent_files: Vec::new(),
            audit_ledger: AgentAuditLedger::default(),
            panes,
            next_pane_id: ROOT_PANE_ID.0 + 1,
        }
    }

    fn record_agent_audit(&mut self, kind: AgentAuditKind, summary: String) {
        self.audit_ledger.push(AgentAuditEntry { kind, summary });
    }

    fn allocate_pane_id(&mut self) -> Result<PaneId, AppError> {
        while self.next_pane_id < u64::MAX {
            let pane_id = PaneId::new(self.next_pane_id);
            self.next_pane_id += 1;
            if !self.panes.contains_key(&pane_id) {
                return Ok(pane_id);
            }
        }

        Err(AppError::PaneIdExhausted)
    }

    fn active_pane_ref(&self) -> &TerminalPane {
        let pane_id = self
            .active_pane_id()
            .expect("desktop app should always have an active pane");
        self.panes
            .get(&pane_id)
            .expect("layout active pane should exist in the pane registry")
    }

    fn active_pane_mut(&mut self) -> &mut TerminalPane {
        let pane_id = self
            .active_pane_id()
            .expect("desktop app should always have an active pane");
        self.panes
            .get_mut(&pane_id)
            .expect("layout active pane should exist in the pane registry")
    }

    fn sync_pane_terminal_sizes(&mut self) -> Result<(), AppError> {
        let layouts = self.pane_layouts();
        for layout in layouts {
            let pane_size = pane_terminal_size(
                self.surface,
                self.terminal_size,
                pane_content_surface(layout.rect, self.pane_chrome),
            );
            self.pane_mut_by_id(layout.pane_id)
                .ok_or(AppError::PaneNotFound(layout.pane_id))?
                .resize(pane_size)?;
        }

        if let Some(scratch_pane_id) = self.scratch_pane_id {
            let pane_size =
                scratch_terminal_size(self.surface, self.terminal_size, self.pane_chrome);
            self.pane_mut_by_id(scratch_pane_id)
                .ok_or(AppError::PaneNotFound(scratch_pane_id))?
                .resize(pane_size)?;
        }

        Ok(())
    }

    fn active_workspace_pane_id(&self) -> Result<PaneId, AppError> {
        self.workspaces
            .active_layout()
            .active_pane()
            .ok_or(AppError::MissingActivePane)
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

fn selection_line_ending() -> LineEnding {
    if cfg!(windows) {
        LineEnding::CrLf
    } else {
        LineEnding::Lf
    }
}

fn preview_agent_summary(text: &str) -> String {
    let redacted = crate::redaction::redact_secret_text(text);
    let normalized = redacted.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut preview = normalized.chars().take(48).collect::<String>();
    if normalized.chars().count() > 48 {
        preview.push_str("...");
    }
    preview
}

fn full_frame_damage(size: PtySize) -> DamageSet {
    DamageSet {
        dirty_rows: (0..usize::from(size.rows)).collect(),
        full_frame: true,
    }
}

fn command_shell_label(command: &PtyCommand) -> String {
    Path::new(command.program())
        .file_name()
        .unwrap_or(command.program())
        .to_string_lossy()
        .into_owned()
}

fn detect_git_branch(cwd: &Path) -> Option<String> {
    for args in [
        ["symbolic-ref", "--quiet", "--short", "HEAD"].as_slice(),
        ["rev-parse", "--abbrev-ref", "HEAD"].as_slice(),
    ] {
        let output = ProcessCommand::new("git")
            .arg("-C")
            .arg(cwd)
            .args(args)
            .output()
            .ok()?;
        if !output.status.success() {
            continue;
        }

        let branch = String::from_utf8(output.stdout).ok()?;
        let branch = branch.trim();
        if !branch.is_empty() && branch != "HEAD" {
            return Some(branch.to_string());
        }
    }

    None
}

fn format_exit_status(status: &PtyExitStatus) -> String {
    match status.signal() {
        Some(signal) => format!("signal {signal}"),
        None => format!("code {}", status.exit_code()),
    }
}

fn collect_all_rows(snapshot: &TerminalSnapshot) -> Vec<noctrail_term::ScreenRowSnapshot> {
    let mut rows = snapshot.scrollback.clone();
    rows.extend(snapshot.rows.clone());
    rows
}

fn max_scrollback_offset(snapshot: &TerminalSnapshot) -> usize {
    snapshot.scrollback.len()
}

fn visible_row_range(
    snapshot: &TerminalSnapshot,
    visible_height: usize,
    scrollback_offset: usize,
) -> std::ops::Range<usize> {
    let total_rows = snapshot.scrollback.len() + snapshot.rows.len();
    let end = total_rows.saturating_sub(scrollback_offset.min(max_scrollback_offset(snapshot)));
    let start = end.saturating_sub(visible_height.max(1));
    start..end
}

fn viewport_to_terminal_position(
    position: Position,
    visible_range: &std::ops::Range<usize>,
    terminal_size: PtySize,
) -> Position {
    Position {
        row: visible_range.start.saturating_add(
            position
                .row
                .min(usize::from(terminal_size.rows).saturating_sub(1)),
        ),
        col: position
            .col
            .min(usize::from(terminal_size.cols).saturating_sub(1)),
    }
}

fn remap_cursor(
    cursor: Cursor,
    scrollback_rows: usize,
    visible_range: &std::ops::Range<usize>,
) -> Cursor {
    let global_row = scrollback_rows.saturating_add(cursor.row);
    if visible_range.contains(&global_row) {
        Cursor {
            row: global_row - visible_range.start,
            col: cursor.col,
        }
    } else {
        Cursor {
            row: usize::MAX,
            col: cursor.col,
        }
    }
}

fn remap_selection(
    selection: &Selection,
    visible_range: &std::ops::Range<usize>,
) -> Option<Selection> {
    let selection = selection.clone().normalized();
    if selection.end.row < visible_range.start || selection.start.row >= visible_range.end {
        return None;
    }

    Some(Selection {
        mode: selection.mode,
        start: Position {
            row: selection
                .start
                .row
                .clamp(visible_range.start, visible_range.end - 1)
                - visible_range.start,
            col: selection.start.col,
        },
        end: Position {
            row: selection
                .end
                .row
                .clamp(visible_range.start, visible_range.end - 1)
                - visible_range.start,
            col: selection.end.col,
        },
    })
}

fn pane_terminal_size(
    surface: LayoutRect,
    terminal_size: PtySize,
    pane_rect: LayoutRect,
) -> PtySize {
    let cols = projected_cells(
        pane_rect.x.saturating_sub(surface.x),
        pane_rect.width,
        surface.width,
        terminal_size.cols,
    );
    let rows = projected_cells(
        pane_rect.y.saturating_sub(surface.y),
        pane_rect.height,
        surface.height,
        terminal_size.rows,
    );
    PtySize::new(cols, rows)
}

fn scratch_surface(surface: LayoutRect) -> LayoutRect {
    let height = (surface.height / SCRATCH_HEIGHT_DIVISOR).max(1);
    LayoutRect::new(surface.x, surface.y, surface.width, height)
}

fn scratch_terminal_size(
    surface: LayoutRect,
    terminal_size: PtySize,
    pane_chrome: PaneChromeConfig,
) -> PtySize {
    pane_terminal_size(
        surface,
        terminal_size,
        pane_content_surface(scratch_surface(surface), pane_chrome),
    )
}

fn projected_cells(offset: u16, span: u16, total_span: u16, total_cells: u16) -> u16 {
    if total_span == 0 || total_cells <= 1 {
        return total_cells.max(1);
    }

    let start = (u32::from(offset) * u32::from(total_cells)) / u32::from(total_span);
    let end =
        (u32::from(offset.saturating_add(span)) * u32::from(total_cells)) / u32::from(total_span);
    end.saturating_sub(start).max(1) as u16
}

fn pane_content_surface(pane_surface: LayoutRect, pane_chrome: PaneChromeConfig) -> LayoutRect {
    inset_layout_rect(pane_surface, pane_content_insets(pane_chrome))
}

fn pane_content_insets(pane_chrome: PaneChromeConfig) -> EdgeInsets {
    let left_gap = pane_chrome.gap / 2;
    let right_gap = pane_chrome.gap - left_gap;
    let top_gap = pane_chrome.gap / 2;
    let bottom_gap = pane_chrome.gap - top_gap;
    EdgeInsets {
        left: left_gap.saturating_add(pane_chrome.padding),
        right: right_gap.saturating_add(pane_chrome.padding),
        top: top_gap.saturating_add(pane_chrome.padding),
        bottom: bottom_gap.saturating_add(pane_chrome.padding),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct EdgeInsets {
    left: u16,
    right: u16,
    top: u16,
    bottom: u16,
}

fn inset_layout_rect(rect: LayoutRect, insets: EdgeInsets) -> LayoutRect {
    let total_horizontal = insets.left.saturating_add(insets.right).min(rect.width);
    let total_vertical = insets.top.saturating_add(insets.bottom).min(rect.height);

    let width = rect.width.saturating_sub(total_horizontal).max(1);
    let height = rect.height.saturating_sub(total_vertical).max(1);
    let left = insets.left.min(rect.width.saturating_sub(width));
    let top = insets.top.min(rect.height.saturating_sub(height));

    LayoutRect::new(
        rect.x.saturating_add(left),
        rect.y.saturating_add(top),
        width,
        height,
    )
}

fn proposal_submission_bytes(command: &str) -> Vec<u8> {
    let mut bytes = command.as_bytes().to_vec();
    bytes.push(b'\r');
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        error::Error as StdError,
        fs, thread,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn shellless_app_builds_single_pane_frame() {
        let app = DesktopApp::new(LayoutRect::new(0, 0, 120, 80), PtySize::new(10, 3));

        assert_eq!(app.active_workspace_id(), WorkspaceId::new(1));
        assert_eq!(app.workspace_ids(), vec![WorkspaceId::new(1)]);
        assert_eq!(app.active_pane_id(), Some(PaneId::new(1)));
        assert_eq!(app.pane_count(), 1);
        assert_eq!(app.pane_layouts().len(), 1);
        let frame = app.frame();
        assert_eq!(frame.workspace_id, WorkspaceId::new(1));
        assert!(!frame.is_scratch);
        assert_eq!(frame.pane_id, PaneId::new(1));
        assert_eq!(frame.surface, LayoutRect::new(0, 0, 120, 80));
        assert_eq!(frame.terminal_size, PtySize::new(10, 3));
        assert!(frame.process_id.is_none());
        assert_eq!(frame.status_line, PaneStatusLine::default());
        assert_eq!(frame.render_plan.rows.len(), 3);
        assert!(frame.render_plan.damage.full_frame);
        assert_eq!(frame.render_plan.damage.dirty_rows, vec![0, 1, 2]);
        assert_eq!(frame.render_plan.scrollback_rows, 0);
        assert!(frame.render_plan.active);
        assert!(frame.render_plan.selection.is_none());
    }

    #[test]
    fn spawned_shell_frame_exposes_status_line_metadata() -> Result<(), Box<dyn StdError>> {
        let repo_dir = temp_git_repo("status-line", "status-line-test")?;
        let mut command = PtyCommand::shell();
        command.cwd_path(&repo_dir);
        let mut app = DesktopApp::spawn(
            LayoutRect::new(0, 0, 120, 40),
            command,
            PtySize::new(80, 24),
        )?;

        let frame = app.frame();
        assert_eq!(frame.status_line.cwd.as_deref(), Some(repo_dir.as_path()));
        assert_eq!(
            frame.status_line.git_branch.as_deref(),
            Some("status-line-test")
        );
        assert!(
            frame
                .status_line
                .shell
                .as_deref()
                .is_some_and(|shell| !shell.is_empty())
        );
        assert!(frame.status_line.exit_status.is_none());

        let _ = app.close_runtime()?;
        let _ = fs::remove_dir_all(repo_dir);
        Ok(())
    }

    #[test]
    fn refresh_runtime_statuses_caches_exit_code() -> Result<(), Box<dyn StdError>> {
        let mut app = DesktopApp::spawn(
            LayoutRect::new(0, 0, 120, 40),
            exit_status_probe_command(7),
            PtySize::new(80, 24),
        )?;

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut observed = false;
        while Instant::now() < deadline {
            if app.refresh_runtime_statuses()? {
                observed = true;
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }

        assert!(
            observed,
            "runtime exit status was not observed before timeout"
        );
        assert_eq!(
            app.frame().status_line.exit_status.as_deref(),
            Some("code 7")
        );

        let status = app.close_runtime()?;
        assert_eq!(status.as_ref().map(PtyExitStatus::exit_code), Some(7));
        assert_eq!(
            app.frame().status_line.exit_status.as_deref(),
            Some("code 7")
        );
        Ok(())
    }

    #[test]
    fn setting_pane_chrome_insets_content_surface_and_terminal_size()
    -> Result<(), Box<dyn StdError>> {
        let mut app = DesktopApp::new(LayoutRect::new(0, 0, 120, 80), PtySize::new(12, 4));
        let chrome = PaneChromeConfig {
            border: PaneBorderStyle {
                width: 2,
                active: noctrail_render::Rgba::opaque(0x7a, 0xa2, 0xf7),
                inactive: noctrail_render::Rgba::opaque(0x3b, 0x42, 0x61),
            },
            gap: 8,
            padding: 6,
            radius: 10,
        };

        app.set_pane_chrome(chrome)?;

        let frame = app.frame();
        assert_eq!(frame.pane_surface, LayoutRect::new(0, 0, 120, 80));
        assert_eq!(frame.surface, LayoutRect::new(10, 10, 100, 60));
        assert_eq!(frame.terminal_size, PtySize::new(10, 3));
        assert_eq!(frame.render_plan.pane_rect, RenderRect::new(0, 0, 120, 80));
        assert_eq!(frame.render_plan.viewport, RenderRect::new(10, 10, 100, 60));
        assert_eq!(frame.render_plan.border, chrome.border);
        assert_eq!(frame.render_plan.corner_radius, 10);
        Ok(())
    }

    #[test]
    fn odd_gap_insets_keep_adjacent_panes_aligned() {
        let chrome = PaneChromeConfig {
            border: PaneBorderStyle::default(),
            gap: 3,
            padding: 2,
            radius: 8,
        };
        let left = pane_content_surface(LayoutRect::new(0, 0, 62, 53), chrome);
        let right = pane_content_surface(LayoutRect::new(62, 0, 63, 53), chrome);

        assert_eq!(left, LayoutRect::new(3, 3, 55, 46));
        assert_eq!(right, LayoutRect::new(65, 3, 56, 46));
        assert!(left.x.saturating_add(left.width) < right.x);
        assert_eq!(right.x.saturating_add(right.width), 121);
    }

    #[test]
    fn splitting_active_pane_adds_a_new_leaf_and_focuses_it() -> Result<(), Box<dyn StdError>> {
        let mut app = DesktopApp::new(LayoutRect::new(0, 0, 120, 40), PtySize::new(80, 24));

        let new_pane = app.split_active_pane_shell()?;

        assert_eq!(app.active_pane_id(), Some(new_pane));
        assert_eq!(app.pane_count(), 2);
        assert!(app.pane_by_id(PaneId::new(1)).is_some());
        assert!(app.pane_by_id(new_pane).is_some());

        let layouts = app.pane_layouts();
        assert_eq!(layouts.len(), 2);

        let original_frame = app.frame_for_pane(PaneId::new(1))?;
        let new_frame = app.frame_for_pane(new_pane)?;
        assert_eq!(original_frame.surface, LayoutRect::new(0, 0, 60, 40));
        assert_eq!(new_frame.surface, LayoutRect::new(60, 0, 60, 40));
        assert!(!original_frame.render_plan.active);
        assert!(new_frame.render_plan.active);
        assert_eq!(app.frame().pane_id, new_pane);
        Ok(())
    }

    #[test]
    fn explicit_split_axis_overrides_auto_split_direction() -> Result<(), Box<dyn StdError>> {
        let mut app = DesktopApp::new(LayoutRect::new(0, 0, 120, 40), PtySize::new(12, 4));

        let split = app.split_active_pane_shell_with_axis(SplitAxis::Horizontal)?;

        assert_eq!(
            app.frame_for_pane(PaneId::new(1))?.surface,
            LayoutRect::new(0, 0, 120, 20)
        );
        assert_eq!(
            app.frame_for_pane(split)?.surface,
            LayoutRect::new(0, 20, 120, 20)
        );
        Ok(())
    }

    #[test]
    fn resizing_active_split_updates_pane_terminal_sizes() -> Result<(), Box<dyn StdError>> {
        let mut app = DesktopApp::spawn_shell(LayoutRect::new(0, 0, 120, 40), PtySize::new(12, 4))?;
        let root = app.active_pane_id().expect("root pane should exist");
        let split = app.split_active_pane_shell()?;

        assert_eq!(app.frame_for_pane(root)?.terminal_size, PtySize::new(6, 4));
        assert_eq!(app.frame_for_pane(split)?.terminal_size, PtySize::new(6, 4));

        app.resize_active_split(FocusDirection::Left, 10)?;

        assert_eq!(app.frame_for_pane(root)?.terminal_size, PtySize::new(4, 4));
        assert_eq!(app.frame_for_pane(split)?.terminal_size, PtySize::new(8, 4));
        Ok(())
    }

    #[test]
    fn output_bytes_feed_the_render_plan() {
        let mut app = DesktopApp::new(LayoutRect::new(0, 0, 80, 24), PtySize::new(5, 2));

        app.advance_output(b"hi");

        let frame = app.frame();
        assert_eq!(frame.render_plan.rows.len(), 2);
        assert_eq!(frame.render_plan.damage.dirty_rows, vec![0]);
        assert!(!frame.render_plan.damage.full_frame);
        assert!(frame.render_plan.active);
        assert_eq!(frame.render_plan.rows[0].glyphs[0].text, "h");
        assert_eq!(frame.render_plan.rows[0].glyphs[1].text, "i");
    }

    #[test]
    fn block_observer_is_disabled_by_default_and_does_not_buffer_old_events() {
        let mut app = DesktopApp::new(LayoutRect::new(0, 0, 120, 40), PtySize::new(20, 4));
        let disabled_bytes = shell_integration_probe_bytes(
            "printf disabled",
            "/tmp/noctrail-disabled",
            9,
            1200,
            b"visible disabled",
        );

        app.advance_output(&disabled_bytes);

        assert!(!app.pane().block_observer_enabled());
        assert!(app.pane().current_command_block().is_none());
        assert!(app.pane().command_blocks().is_empty());
        assert_eq!(
            render_row_text(&app.frame().render_plan.rows[0]),
            "visible disabled"
        );

        app.set_block_observer_enabled(true);
        app.advance_output(&shell_integration_probe_bytes(
            "printf enabled",
            "/tmp/noctrail-enabled",
            0,
            33,
            b"visible enabled",
        ));

        assert_eq!(app.pane().command_blocks().len(), 1);
        assert_eq!(
            app.pane().command_blocks()[0],
            CommandBlock {
                command: Some("printf enabled".to_string()),
                cwd: Some(PathBuf::from("/tmp/noctrail-enabled")),
                exit_code: Some(0),
                duration_ms: Some(33),
                output: "visible enabled".to_string(),
                folded: false,
                structured_output: None,
            }
        );
    }

    #[test]
    fn block_observer_tracks_running_and_completed_command_metadata() {
        let mut app = DesktopApp::new(LayoutRect::new(0, 0, 120, 40), PtySize::new(20, 4));
        app.set_block_observer_enabled(true);

        app.advance_output(&shell_integration_start_bytes(
            "cargo test -p noctrail-app",
            "/tmp/noctrail-running",
            b"running output",
        ));

        assert_eq!(
            app.pane().current_command_block(),
            Some(&CommandBlock {
                command: Some("cargo test -p noctrail-app".to_string()),
                cwd: Some(PathBuf::from("/tmp/noctrail-running")),
                exit_code: None,
                duration_ms: None,
                output: "running output".to_string(),
                folded: false,
                structured_output: None,
            })
        );
        assert!(app.pane().command_blocks().is_empty());
        assert_eq!(
            render_row_text(&app.frame().render_plan.rows[0]),
            "running output"
        );

        app.advance_output(&shell_integration_end_bytes(0, 58));

        assert!(app.pane().current_command_block().is_none());
        assert_eq!(
            app.pane().command_blocks(),
            &[CommandBlock {
                command: Some("cargo test -p noctrail-app".to_string()),
                cwd: Some(PathBuf::from("/tmp/noctrail-running")),
                exit_code: Some(0),
                duration_ms: Some(58),
                output: "running output".to_string(),
                folded: false,
                structured_output: None,
            }]
        );
    }

    #[test]
    fn stray_shell_integration_events_do_not_break_terminal_output() {
        let mut app = DesktopApp::new(LayoutRect::new(0, 0, 120, 40), PtySize::new(20, 4));
        app.set_block_observer_enabled(true);

        app.advance_output(
            &[
                osc_marker_pair_bytes("CommandText", "echo stray").as_slice(),
                osc_marker_pair_bytes("ExitCode", "7").as_slice(),
                b"plain text".as_slice(),
            ]
            .concat(),
        );

        assert!(app.pane().current_command_block().is_none());
        assert!(app.pane().command_blocks().is_empty());
        assert_eq!(
            render_row_text(&app.frame().render_plan.rows[0]),
            "plain text"
        );
    }

    #[test]
    fn block_observer_keeps_recent_hundred_and_supports_copy_jump_fold() {
        let mut app = DesktopApp::new(LayoutRect::new(0, 0, 120, 40), PtySize::new(20, 4));
        app.set_block_observer_enabled(true);

        for index in 0..=100 {
            let command = format!("cmd-{index:03}");
            let cwd = format!("/tmp/block-{index:03}");
            let output = format!("output-{index:03}");
            app.advance_output(&shell_integration_probe_bytes(
                command.as_str(),
                cwd.as_str(),
                index,
                index as u64,
                output.as_bytes(),
            ));
        }

        assert_eq!(app.command_blocks().len(), 100);
        assert_eq!(app.command_blocks()[0].command.as_deref(), Some("cmd-001"));
        assert_eq!(app.command_blocks()[99].command.as_deref(), Some("cmd-100"));
        assert_eq!(app.selected_command_block_index(), Some(99));
        assert_eq!(
            app.copy_selected_command_block_output().as_deref(),
            Some("output-100")
        );

        assert_eq!(app.select_oldest_command_block(), Some(0));
        assert_eq!(
            app.copy_selected_command_block_command().as_deref(),
            Some("cmd-001")
        );
        assert_eq!(app.select_previous_command_block(), Some(99));
        assert_eq!(
            app.copy_selected_command_block_command().as_deref(),
            Some("cmd-100")
        );
        assert_eq!(app.select_next_command_block(), Some(0));
        assert_eq!(app.toggle_selected_command_block_fold(), Some(true));
        assert!(
            app.selected_command_block()
                .expect("selected block should exist")
                .folded
        );
        assert_eq!(
            app.copy_selected_command_block_output().as_deref(),
            Some("output-001")
        );
    }

    #[test]
    fn structured_output_lenses_detect_json_csv_and_toml_without_rewriting_raw_output() {
        let mut app = DesktopApp::new(LayoutRect::new(0, 0, 120, 40), PtySize::new(40, 6));
        app.set_block_observer_enabled(true);

        let json_output = "{\"ok\":true,\"items\":[1,2]}\n";
        app.advance_output(&shell_integration_probe_bytes(
            "cat json",
            "/tmp/json",
            0,
            12,
            json_output.as_bytes(),
        ));
        let json_block = app
            .selected_command_block()
            .expect("json block should be selected");
        assert_eq!(
            json_block.structured_output.as_ref().map(|lens| lens.kind),
            Some(StructuredOutputKind::Json)
        );
        assert_eq!(
            json_block
                .structured_output
                .as_ref()
                .map(|lens| lens.summary.as_str()),
            Some("json object 2 keys")
        );
        assert_eq!(
            app.copy_selected_command_block_structured_output()
                .as_deref(),
            Some(json_output)
        );
        assert_eq!(
            app.copy_selected_command_block_output().as_deref(),
            Some(json_output)
        );

        let csv_output = "name,count\nalpha,1\nbeta,2\n";
        app.advance_output(&shell_integration_probe_bytes(
            "cat csv",
            "/tmp/csv",
            0,
            13,
            csv_output.as_bytes(),
        ));
        let csv_block = app
            .selected_command_block()
            .expect("csv block should be selected");
        assert_eq!(
            csv_block.structured_output.as_ref().map(|lens| lens.kind),
            Some(StructuredOutputKind::Csv)
        );
        assert_eq!(
            csv_block
                .structured_output
                .as_ref()
                .map(|lens| lens.summary.as_str()),
            Some("csv 3 rows x 2 cols")
        );
        assert_eq!(
            app.copy_selected_command_block_structured_output()
                .as_deref(),
            Some(csv_output)
        );

        let toml_output = "name = \"noctrail\"\nenabled = true\n";
        app.advance_output(&shell_integration_probe_bytes(
            "cat toml",
            "/tmp/toml",
            0,
            14,
            toml_output.as_bytes(),
        ));
        let toml_block = app
            .selected_command_block()
            .expect("toml block should be selected");
        assert_eq!(
            toml_block.structured_output.as_ref().map(|lens| lens.kind),
            Some(StructuredOutputKind::Toml)
        );
        assert_eq!(
            toml_block
                .structured_output
                .as_ref()
                .map(|lens| lens.summary.as_str()),
            Some("toml table 2 keys")
        );
        assert_eq!(
            app.copy_selected_command_block_structured_output()
                .as_deref(),
            Some(toml_output)
        );
    }

    #[test]
    fn non_structured_blocks_do_not_offer_structured_copy() {
        let mut app = DesktopApp::new(LayoutRect::new(0, 0, 120, 40), PtySize::new(20, 4));
        app.set_block_observer_enabled(true);

        app.advance_output(&shell_integration_probe_bytes(
            "echo plain",
            "/tmp/plain",
            0,
            3,
            b"plain output\n",
        ));

        let block = app
            .selected_command_block()
            .expect("plain block should be selected");
        assert!(block.structured_output.is_none());
        assert!(
            app.copy_selected_command_block_structured_output()
                .is_none()
        );
    }

    #[test]
    fn failure_blocks_are_counted_and_selectable_without_agent_side_effects() {
        let mut app = DesktopApp::new(LayoutRect::new(0, 0, 120, 40), PtySize::new(20, 4));
        app.set_block_observer_enabled(true);

        app.advance_output(&shell_integration_probe_bytes(
            "echo ok",
            "/tmp/ok",
            0,
            2,
            b"ok output\n",
        ));
        app.advance_output(&shell_integration_probe_bytes(
            "echo fail",
            "/tmp/fail",
            7,
            3,
            b"failure output\n",
        ));

        assert_eq!(app.failed_command_blocks_count(), 1);
        assert_eq!(app.select_newest_failed_command_block(), Some(1));
        let block = app
            .selected_command_block()
            .expect("failed block should be selected");
        assert!(block.failed());
        assert_eq!(block.exit_code, Some(7));
        assert_eq!(
            app.copy_selected_command_block_output().as_deref(),
            Some("failure output\n")
        );
        assert!(
            app.copy_selected_command_block_structured_output()
                .is_none()
        );
    }

    #[test]
    fn agent_context_preview_only_contains_block_selection_cwd_and_explicit_files() {
        let mut app = DesktopApp::new(LayoutRect::new(0, 0, 120, 40), PtySize::new(20, 4));
        app.set_block_observer_enabled(true);
        app.advance_output(&shell_integration_probe_bytes(
            "cargo test -p noctrail-app",
            "/tmp/noctrail-agent",
            0,
            7,
            b"alpha beta\r\ngamma delta\r\n",
        ));
        let _ = app.select_newest_command_block();
        app.select_viewport_range(
            Position { row: 0, col: 0 },
            Position { row: 0, col: 4 },
            SelectionMode::Normal,
        );
        app.set_agent_explicit_files(vec![
            PathBuf::from("/tmp/noctrail/Cargo.toml"),
            PathBuf::from("/tmp/noctrail/crates/noctrail-app/src/lib.rs"),
        ]);

        let preview = app.agent_context_preview();
        assert_eq!(
            preview
                .current_block
                .as_ref()
                .and_then(|block| block.command.as_deref()),
            Some("cargo test -p noctrail-app")
        );
        assert_eq!(
            preview
                .current_block
                .as_ref()
                .map(|block| block.output.as_str()),
            Some("alpha beta\ngamma delta\n")
        );
        assert_eq!(preview.selection.as_deref(), Some("alpha"));
        assert_eq!(
            preview.cwd.as_deref(),
            Some(Path::new("/tmp/noctrail-agent"))
        );
        assert_eq!(
            preview.explicit_files,
            vec![
                PathBuf::from("/tmp/noctrail/Cargo.toml"),
                PathBuf::from("/tmp/noctrail/crates/noctrail-app/src/lib.rs"),
            ]
        );
    }

    #[test]
    fn agent_command_proposals_stay_read_only_on_desktop_state() {
        let mut app = DesktopApp::new(LayoutRect::new(0, 0, 120, 40), PtySize::new(20, 4));
        app.set_agent_command_proposals(vec![CommandProposal {
            command: "git status".to_string(),
            reason: "Inspect the repository state.".to_string(),
            risk: noctrail_agent::CommandRisk::Low,
            permission: noctrail_agent::CommandPermission::Review,
        }]);

        assert_eq!(app.agent_command_proposals().len(), 1);
        assert_eq!(app.agent_command_proposals()[0].command, "git status");

        app.clear_agent_command_proposals();
        assert!(app.agent_command_proposals().is_empty());
    }

    #[test]
    fn agent_patch_previews_stay_read_only_on_desktop_state() {
        let mut app = DesktopApp::new(LayoutRect::new(0, 0, 120, 40), PtySize::new(20, 4));
        app.set_agent_patch_previews(vec![PatchPreview {
            path: PathBuf::from("src/lib.rs"),
            reason: "Guard a missing check.".to_string(),
            diff: "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,2 @@\n-foo\n+foo\n+bar\n"
                .to_string(),
        }]);

        assert_eq!(app.agent_patch_previews().len(), 1);
        assert_eq!(
            app.selected_agent_patch_preview()
                .map(|preview| preview.path.as_path()),
            Some(Path::new("src/lib.rs"))
        );

        let _ = app.select_next_agent_patch_preview();
        app.clear_agent_patch_previews();
        assert!(app.agent_patch_previews().is_empty());
    }

    #[test]
    fn audit_ledger_tracks_context_suggest_review_and_execute() -> Result<(), Box<dyn StdError>> {
        let mut app = DesktopApp::spawn_shell(LayoutRect::new(0, 0, 120, 40), PtySize::new(20, 4))?;
        app.set_block_observer_enabled(true);
        app.advance_output(&shell_integration_probe_bytes(
            "echo token=sk-live-secretvalue12345",
            "/tmp/noctrail-audit",
            0,
            7,
            b"ok\n",
        ));
        app.record_agent_context_access(&app.agent_context_preview());
        app.record_agent_read(&ProviderRequestPreview {
            kind: "cli",
            endpoint: None,
            model: None,
            command: vec!["sh".to_string(), "-lc".to_string(), "echo".to_string()],
            prompt_chars: 42,
        });
        app.set_agent_command_proposals(vec![CommandProposal {
            command: "printf 'NOCTRAIL_AUDIT_OK\\n'".to_string(),
            reason: "Verify the shell remains interactive.".to_string(),
            risk: noctrail_agent::CommandRisk::Low,
            permission: noctrail_agent::CommandPermission::Review,
        }]);
        app.record_agent_review("confirm printf 'NOCTRAIL_AUDIT_OK\\n'");
        let _ = app.submit_selected_agent_command_proposal()?;

        let entries = app.agent_audit_entries();
        assert_eq!(
            entries.iter().map(|entry| entry.kind).collect::<Vec<_>>(),
            vec![
                AgentAuditKind::Context,
                AgentAuditKind::Read,
                AgentAuditKind::Suggest,
                AgentAuditKind::Review,
                AgentAuditKind::Execute,
            ]
        );
        assert!(entries[0].summary.contains("[REDACTED]"));
        assert!(entries[2].summary.contains("commands=1"));
        assert!(entries[4].summary.contains("NOCTRAIL_AUDIT_OK"));
        Ok(())
    }

    #[test]
    fn audit_ledger_rolls_forward_and_supports_selection_navigation() {
        let mut app = DesktopApp::new(LayoutRect::new(0, 0, 120, 40), PtySize::new(20, 4));
        for index in 0..=MAX_AUDIT_ENTRIES {
            app.record_agent_review(format!("entry-{index:03}"));
        }

        assert_eq!(app.agent_audit_entries().len(), MAX_AUDIT_ENTRIES);
        assert_eq!(
            app.agent_audit_entries()
                .first()
                .map(|entry| entry.summary.as_str()),
            Some("entry-001")
        );
        assert_eq!(
            app.agent_audit_entries()
                .last()
                .map(|entry| entry.summary.as_str()),
            Some("entry-200")
        );
        assert_eq!(
            app.selected_agent_audit_entry()
                .map(|entry| entry.summary.as_str()),
            Some("entry-200")
        );
        assert_eq!(app.select_oldest_agent_audit_entry(), Some(0));
        assert_eq!(
            app.selected_agent_audit_entry()
                .map(|entry| entry.summary.as_str()),
            Some("entry-001")
        );
        assert_eq!(
            app.select_previous_agent_audit_entry(),
            Some(MAX_AUDIT_ENTRIES - 1)
        );
        assert_eq!(
            app.selected_agent_audit_entry()
                .map(|entry| entry.summary.as_str()),
            Some("entry-200")
        );
        assert_eq!(app.select_next_agent_audit_entry(), Some(0));
    }

    #[test]
    fn resize_updates_terminal_size_without_runtime() -> Result<(), Box<dyn std::error::Error>> {
        let mut app = DesktopApp::new(LayoutRect::new(0, 0, 80, 24), PtySize::new(5, 2));

        app.resize(LayoutRect::new(10, 20, 160, 90), PtySize::new(7, 4))?;
        let frame = app.frame();
        assert_eq!(frame.surface, LayoutRect::new(10, 20, 160, 90));
        assert_eq!(frame.terminal_size, PtySize::new(7, 4));
        assert!(frame.render_plan.damage.full_frame);
        assert!(frame.render_plan.active);
        assert_eq!(frame.render_plan.damage.dirty_rows, vec![0, 1, 2, 3]);
        Ok(())
    }

    #[test]
    fn scrollback_offset_changes_visible_render_rows() {
        let mut app = DesktopApp::new(LayoutRect::new(0, 0, 120, 80), PtySize::new(8, 2));

        app.advance_output(b"one\r\ntwo\r\nthree");
        let live_frame = app.frame();
        assert_eq!(render_row_text(&live_frame.render_plan.rows[0]), "two");
        assert_eq!(render_row_text(&live_frame.render_plan.rows[1]), "three");

        app.scroll_scrollback(1);
        let scrolled_frame = app.frame();
        assert_eq!(render_row_text(&scrolled_frame.render_plan.rows[0]), "one");
        assert_eq!(render_row_text(&scrolled_frame.render_plan.rows[1]), "two");
        assert!(scrolled_frame.render_plan.damage.full_frame);
    }

    #[test]
    fn viewport_selection_maps_through_scrollback() {
        let mut app = DesktopApp::new(LayoutRect::new(0, 0, 120, 80), PtySize::new(8, 2));

        app.advance_output(b"one\r\ntwo\r\nthree");
        app.scroll_scrollback(1);
        app.select_viewport_range(
            Position { row: 0, col: 0 },
            Position { row: 1, col: 2 },
            SelectionMode::Normal,
        );

        assert_eq!(app.copy_selection_text().as_deref(), Some("one     \ntwo"));
        let frame = app.frame();
        assert!(frame.render_plan.selection.is_some());
    }

    #[test]
    fn mouse_modes_surface_from_terminal_state() {
        let mut app = DesktopApp::new(LayoutRect::new(0, 0, 120, 80), PtySize::new(8, 2));

        assert!(!app.mouse_reporting_enabled());
        assert_eq!(app.mouse_tracking_mode(), MouseTrackingMode::Disabled);
        assert!(!app.sgr_mouse_mode());

        app.advance_output(b"\x1b[?1002h\x1b[?1006h");

        assert!(app.mouse_reporting_enabled());
        assert_eq!(app.mouse_tracking_mode(), MouseTrackingMode::Drag);
        assert!(app.sgr_mouse_mode());
    }

    #[test]
    fn active_pane_writes_and_pastes_into_shell() -> Result<(), Box<dyn StdError>> {
        let mut app =
            DesktopApp::spawn_shell(LayoutRect::new(0, 0, 120, 80), PtySize::new(80, 24))?;

        app.write_input(shell_command_bytes("NOCTRAIL_APP_WRITE").as_slice())?;
        app.paste_text(shell_command_text("NOCTRAIL_APP_PASTE").as_str())?;
        app.write_input(shell_exit_bytes().as_slice())?;

        let output = read_all_runtime_output(&mut app)?;
        let text = String::from_utf8_lossy(&output);
        assert!(
            text.contains("NOCTRAIL_APP_WRITE"),
            "active pane write did not reach shell: {text:?}"
        );
        assert!(
            text.contains("NOCTRAIL_APP_PASTE"),
            "active pane paste did not reach shell: {text:?}"
        );

        let status = app.close_runtime()?;
        assert!(status.is_some(), "shell should exit after smoke commands");
        Ok(())
    }

    #[test]
    fn split_panes_keep_independent_shell_sessions() -> Result<(), Box<dyn StdError>> {
        let mut app =
            DesktopApp::spawn_shell(LayoutRect::new(0, 0, 120, 40), PtySize::new(80, 24))?;
        let root_pane = app.active_pane_id().expect("root pane should exist");
        let new_pane = app.split_active_pane_shell()?;

        app.pane_mut_by_id(root_pane)
            .ok_or(AppError::PaneNotFound(root_pane))?
            .write_input(shell_command_bytes("NOCTRAIL_ROOT").as_slice())?;
        app.pane_mut_by_id(root_pane)
            .ok_or(AppError::PaneNotFound(root_pane))?
            .write_input(shell_exit_bytes().as_slice())?;

        app.pane_mut_by_id(new_pane)
            .ok_or(AppError::PaneNotFound(new_pane))?
            .write_input(shell_command_bytes("NOCTRAIL_SPLIT").as_slice())?;
        app.pane_mut_by_id(new_pane)
            .ok_or(AppError::PaneNotFound(new_pane))?
            .write_input(shell_exit_bytes().as_slice())?;

        let root_output = read_all_runtime_output_for_pane(&mut app, root_pane)?;
        let split_output = read_all_runtime_output_for_pane(&mut app, new_pane)?;
        let root_text = String::from_utf8_lossy(&root_output);
        let split_text = String::from_utf8_lossy(&split_output);

        assert!(
            root_text.contains("NOCTRAIL_ROOT"),
            "root pane output missing its marker: {root_text:?}"
        );
        assert!(
            !root_text.contains("NOCTRAIL_SPLIT"),
            "root pane output leaked split marker: {root_text:?}"
        );
        assert!(
            split_text.contains("NOCTRAIL_SPLIT"),
            "split pane output missing its marker: {split_text:?}"
        );
        assert!(
            !split_text.contains("NOCTRAIL_ROOT"),
            "split pane output leaked root marker: {split_text:?}"
        );

        let root_status = app
            .pane_mut_by_id(root_pane)
            .ok_or(AppError::PaneNotFound(root_pane))?
            .close_runtime()?;
        let split_status = app
            .pane_mut_by_id(new_pane)
            .ok_or(AppError::PaneNotFound(new_pane))?
            .close_runtime()?;
        assert!(root_status.is_some());
        assert!(split_status.is_some());
        Ok(())
    }

    #[test]
    fn switching_workspaces_creates_and_preserves_independent_session_sets()
    -> Result<(), Box<dyn StdError>> {
        let mut app = DesktopApp::spawn_shell(LayoutRect::new(0, 0, 120, 40), PtySize::new(12, 4))?;
        let workspace_one_pane = app.active_pane_id().expect("workspace 1 pane should exist");
        let workspace_one_pid = app
            .pane_by_id(workspace_one_pane)
            .and_then(TerminalPane::process_id)
            .expect("workspace 1 shell should have a process id");

        let workspace_two_pane = app.switch_workspace(WorkspaceId::new(2))?;
        let workspace_two_pid = app
            .pane_by_id(workspace_two_pane)
            .and_then(TerminalPane::process_id)
            .expect("workspace 2 shell should have a process id");

        assert_eq!(app.active_workspace_id(), WorkspaceId::new(2));
        assert_ne!(workspace_one_pane, workspace_two_pane);
        assert_ne!(workspace_one_pid, workspace_two_pid);
        assert_eq!(
            app.workspace_ids(),
            vec![WorkspaceId::new(1), WorkspaceId::new(2)]
        );

        let switched_back = app.switch_workspace(WorkspaceId::new(1))?;
        assert_eq!(switched_back, workspace_one_pane);
        assert_eq!(app.active_workspace_id(), WorkspaceId::new(1));
        assert_eq!(app.active_pane_id(), Some(workspace_one_pane));
        assert_eq!(
            app.pane_by_id(workspace_one_pane)
                .and_then(TerminalPane::process_id),
            Some(workspace_one_pid)
        );

        let first_frame = app.frame();
        assert_eq!(first_frame.workspace_id, WorkspaceId::new(1));
        assert!(!first_frame.is_scratch);

        let workspace_two = app.switch_workspace(WorkspaceId::new(2))?;
        assert_eq!(workspace_two, workspace_two_pane);
        assert_eq!(
            app.pane_by_id(workspace_two_pane)
                .and_then(TerminalPane::process_id),
            Some(workspace_two_pid)
        );
        assert_eq!(app.frame().workspace_id, WorkspaceId::new(2));
        assert!(!app.frame().is_scratch);

        let first_status = app
            .pane_mut_by_id(workspace_one_pane)
            .ok_or(AppError::PaneNotFound(workspace_one_pane))?
            .close_runtime()?;
        let second_status = app
            .pane_mut_by_id(workspace_two_pane)
            .ok_or(AppError::PaneNotFound(workspace_two_pane))?
            .close_runtime()?;
        assert!(first_status.is_some());
        assert!(second_status.is_some());
        Ok(())
    }

    #[test]
    fn focus_direction_switches_the_active_pane() -> Result<(), Box<dyn StdError>> {
        let mut app = DesktopApp::new(LayoutRect::new(0, 0, 120, 80), PtySize::new(80, 24));
        let second = app.split_active_pane_shell()?;
        let third = app.split_active_pane_shell()?;

        assert_eq!(app.active_pane_id(), Some(third));
        assert_eq!(app.focus_direction(FocusDirection::Left)?, PaneId::new(1));
        assert_eq!(app.active_pane_id(), Some(PaneId::new(1)));
        assert_eq!(app.focus_direction(FocusDirection::Right)?, second);
        assert_eq!(app.focus_direction(FocusDirection::Down)?, third);
        assert_eq!(app.active_pane_id(), Some(third));
        Ok(())
    }

    #[test]
    fn swapping_active_pane_preserves_focus_and_moves_its_rect() -> Result<(), Box<dyn StdError>> {
        let mut app = DesktopApp::new(LayoutRect::new(0, 0, 120, 40), PtySize::new(12, 4));
        let split = app.split_active_pane_shell()?;

        assert_eq!(app.active_pane_id(), Some(split));
        app.swap_active_pane(FocusDirection::Left)?;

        assert_eq!(app.active_pane_id(), Some(split));
        assert_eq!(
            app.frame_for_pane(split)?.surface,
            LayoutRect::new(0, 0, 60, 40)
        );
        assert_eq!(
            app.frame_for_pane(PaneId::new(1))?.surface,
            LayoutRect::new(60, 0, 60, 40)
        );
        Ok(())
    }

    #[test]
    fn closing_active_pane_focuses_the_survivor() -> Result<(), Box<dyn StdError>> {
        let mut app = DesktopApp::spawn_shell(LayoutRect::new(0, 0, 120, 40), PtySize::new(12, 4))?;
        let split = app.split_active_pane_shell()?;

        let (survivor, status) = app.close_active_pane()?;

        assert_eq!(survivor, PaneId::new(1));
        assert_eq!(app.active_pane_id(), Some(PaneId::new(1)));
        assert_eq!(app.pane_count(), 1);
        assert!(app.pane_by_id(split).is_none());
        assert!(status.is_some());
        Ok(())
    }

    #[cfg(not(windows))]
    #[test]
    fn ctrl_d_writes_eot_byte_to_foreground_process() -> Result<(), Box<dyn StdError>> {
        let mut app = DesktopApp::spawn(
            LayoutRect::new(0, 0, 120, 80),
            single_byte_hex_dump_command(),
            PtySize::new(80, 24),
        )?;

        app.write_input(&[0x04])?;
        let output = read_all_runtime_output(&mut app)?;
        let text = String::from_utf8_lossy(&output);
        assert!(
            text.contains("04"),
            "ctrl-d byte did not reach the foreground process: {text:?}"
        );

        let status = app.close_runtime()?;
        assert!(
            status.is_some(),
            "foreground process should exit after one byte"
        );
        Ok(())
    }

    #[cfg(not(windows))]
    #[test]
    fn pane_resize_reaches_each_shell_session() -> Result<(), Box<dyn StdError>> {
        let mut app = DesktopApp::spawn_shell(LayoutRect::new(0, 0, 120, 40), PtySize::new(12, 4))?;
        let root = app.active_pane_id().expect("root pane should exist");
        let split = app.split_active_pane_shell()?;
        app.resize_active_split(FocusDirection::Left, 10)?;

        app.pane_mut_by_id(root)
            .ok_or(AppError::PaneNotFound(root))?
            .write_input(b"printf 'ROOT\\n'; stty size; exit\r")?;
        app.pane_mut_by_id(split)
            .ok_or(AppError::PaneNotFound(split))?
            .write_input(b"printf 'SPLIT\\n'; stty size; exit\r")?;

        let root_output = read_all_runtime_output_for_pane(&mut app, root)?;
        let split_output = read_all_runtime_output_for_pane(&mut app, split)?;
        let root_text = String::from_utf8_lossy(&root_output);
        let split_text = String::from_utf8_lossy(&split_output);

        assert!(root_text.contains("ROOT"));
        assert!(
            root_text.contains("4 4"),
            "unexpected root size output: {root_text:?}"
        );
        assert!(split_text.contains("SPLIT"));
        assert!(
            split_text.contains("4 8"),
            "unexpected split size output: {split_text:?}"
        );
        let root_status = app
            .pane_mut_by_id(root)
            .ok_or(AppError::PaneNotFound(root))?
            .close_runtime()?;
        let split_status = app
            .pane_mut_by_id(split)
            .ok_or(AppError::PaneNotFound(split))?
            .close_runtime()?;
        assert!(root_status.is_some());
        assert!(split_status.is_some());
        Ok(())
    }

    #[test]
    fn scratch_toggle_preserves_main_layout_and_reuses_its_session() -> Result<(), Box<dyn StdError>>
    {
        let mut app = DesktopApp::spawn_shell(LayoutRect::new(0, 0, 120, 40), PtySize::new(12, 4))?;
        let main_active = app.split_active_pane_shell()?;
        let main_layouts = app.pane_layouts();

        let scratch = app.toggle_scratch()?;
        let scratch_pid = app
            .pane_by_id(scratch)
            .and_then(TerminalPane::process_id)
            .expect("scratch pane should have a process id");

        assert!(app.scratch_visible());
        assert_eq!(app.scratch_pane_id(), Some(scratch));
        assert_eq!(app.active_pane_id(), Some(scratch));
        assert_eq!(app.pane_layouts(), main_layouts);
        assert_eq!(app.frame().surface, scratch_surface(app.surface()));
        assert!(app.frame().is_scratch);

        let restored = app.toggle_scratch()?;
        assert_eq!(restored, main_active);
        assert!(!app.scratch_visible());
        assert_eq!(app.active_pane_id(), Some(main_active));
        assert_eq!(app.pane_layouts(), main_layouts);

        let scratch_again = app.toggle_scratch()?;
        assert_eq!(scratch_again, scratch);
        assert_eq!(
            app.pane_by_id(scratch_again)
                .and_then(TerminalPane::process_id),
            Some(scratch_pid)
        );

        let scratch_status = app
            .pane_mut_by_id(scratch)
            .ok_or(AppError::PaneNotFound(scratch))?
            .close_runtime()?;
        let main_status = app
            .pane_mut_by_id(main_active)
            .ok_or(AppError::PaneNotFound(main_active))?
            .close_runtime()?;
        assert!(scratch_status.is_some());
        assert!(main_status.is_some());
        Ok(())
    }

    #[test]
    fn scratch_pane_keeps_an_independent_shell_session() -> Result<(), Box<dyn StdError>> {
        let mut app =
            DesktopApp::spawn_shell(LayoutRect::new(0, 0, 120, 40), PtySize::new(80, 24))?;
        let main_pane = app.active_pane_id().expect("main pane should exist");
        let scratch_pane = app.toggle_scratch()?;

        app.write_input(shell_command_bytes("NOCTRAIL_SCRATCH").as_slice())?;
        app.write_input(shell_exit_bytes().as_slice())?;

        let restored = app.toggle_scratch()?;
        assert_eq!(restored, main_pane);

        app.pane_mut_by_id(main_pane)
            .ok_or(AppError::PaneNotFound(main_pane))?
            .write_input(shell_command_bytes("NOCTRAIL_MAIN").as_slice())?;
        app.pane_mut_by_id(main_pane)
            .ok_or(AppError::PaneNotFound(main_pane))?
            .write_input(shell_exit_bytes().as_slice())?;

        let scratch_output = read_all_runtime_output_for_pane(&mut app, scratch_pane)?;
        let main_output = read_all_runtime_output_for_pane(&mut app, main_pane)?;
        let scratch_text = String::from_utf8_lossy(&scratch_output);
        let main_text = String::from_utf8_lossy(&main_output);

        assert!(
            scratch_text.contains("NOCTRAIL_SCRATCH"),
            "scratch pane output missing its marker: {scratch_text:?}"
        );
        assert!(
            !scratch_text.contains("NOCTRAIL_MAIN"),
            "scratch pane output leaked main marker: {scratch_text:?}"
        );
        assert!(
            main_text.contains("NOCTRAIL_MAIN"),
            "main pane output missing its marker: {main_text:?}"
        );
        assert!(
            !main_text.contains("NOCTRAIL_SCRATCH"),
            "main pane output leaked scratch marker: {main_text:?}"
        );

        let scratch_status = app
            .pane_mut_by_id(scratch_pane)
            .ok_or(AppError::PaneNotFound(scratch_pane))?
            .close_runtime()?;
        let main_status = app
            .pane_mut_by_id(main_pane)
            .ok_or(AppError::PaneNotFound(main_pane))?
            .close_runtime()?;
        assert!(scratch_status.is_some());
        assert!(main_status.is_some());
        Ok(())
    }

    fn read_all_runtime_output(app: &mut DesktopApp) -> Result<Vec<u8>, AppError> {
        let runtime = app
            .pane_mut()
            .runtime_mut()
            .ok_or(AppError::MissingRuntime)?;
        read_all_runtime_output_from_runtime(runtime)
    }

    fn read_all_runtime_output_for_pane(
        app: &mut DesktopApp,
        pane_id: PaneId,
    ) -> Result<Vec<u8>, AppError> {
        let runtime = app
            .pane_mut_by_id(pane_id)
            .ok_or(AppError::PaneNotFound(pane_id))?
            .runtime_mut()
            .ok_or(AppError::MissingRuntime)?;
        read_all_runtime_output_from_runtime(runtime)
    }

    fn read_all_runtime_output_from_runtime(
        runtime: &mut PaneRuntime,
    ) -> Result<Vec<u8>, AppError> {
        let mut output = Vec::new();
        let mut chunk = [0_u8; 1024];

        loop {
            let count = runtime.read_output(&mut chunk)?;
            if count == 0 {
                break;
            }
            output.extend_from_slice(&chunk[..count]);
        }

        Ok(output)
    }

    fn shell_command_text(marker: &str) -> String {
        #[cfg(windows)]
        {
            format!("echo {marker}\r\n")
        }

        #[cfg(not(windows))]
        {
            format!("printf '{marker}\\n'\r")
        }
    }

    fn shell_command_bytes(marker: &str) -> Vec<u8> {
        shell_command_text(marker).into_bytes()
    }

    fn shell_exit_bytes() -> Vec<u8> {
        b"exit\r\n".to_vec()
    }

    fn shell_integration_probe_bytes(
        command: &str,
        cwd: &str,
        exit_code: i32,
        duration_ms: u64,
        visible: &[u8],
    ) -> Vec<u8> {
        [
            shell_integration_start_bytes(command, cwd, visible).as_slice(),
            shell_integration_end_bytes(exit_code, duration_ms).as_slice(),
        ]
        .concat()
    }

    fn shell_integration_start_bytes(command: &str, cwd: &str, visible: &[u8]) -> Vec<u8> {
        [
            osc_marker_bytes("Prompt").as_slice(),
            osc_marker_bytes("CommandStart").as_slice(),
            osc_marker_pair_bytes("CommandText", command).as_slice(),
            osc_marker_pair_bytes("Cwd", cwd).as_slice(),
            visible,
        ]
        .concat()
    }

    fn shell_integration_end_bytes(exit_code: i32, duration_ms: u64) -> Vec<u8> {
        [
            osc_marker_pair_bytes("ExitCode", exit_code.to_string().as_str()).as_slice(),
            osc_marker_pair_bytes("DurationMs", duration_ms.to_string().as_str()).as_slice(),
            osc_marker_bytes("CommandEnd").as_slice(),
        ]
        .concat()
    }

    fn osc_marker_bytes(marker: &str) -> Vec<u8> {
        format!("\x1b]1337;Noctrail;{marker}\x07").into_bytes()
    }

    fn osc_marker_pair_bytes(marker: &str, value: &str) -> Vec<u8> {
        format!("\x1b]1337;Noctrail;{marker};{value}\x07").into_bytes()
    }

    fn exit_status_probe_command(code: u32) -> PtyCommand {
        #[cfg(windows)]
        {
            let mut command = PtyCommand::new("cmd.exe");
            command.args(["/C", &format!("exit {code}")]);
            command
        }

        #[cfg(not(windows))]
        {
            let mut command = PtyCommand::new("sh");
            command.args(["-lc", &format!("exit {code}")]);
            command
        }
    }

    fn render_row_text(row: &noctrail_render::RenderRow) -> String {
        row.glyphs
            .iter()
            .map(|glyph| glyph.text.as_str())
            .collect::<String>()
    }

    fn temp_git_repo(label: &str, branch: &str) -> Result<PathBuf, Box<dyn StdError>> {
        let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let repo_dir = std::env::temp_dir().join(format!("noctrail-{label}-{unique}"));
        fs::create_dir_all(&repo_dir)?;

        let init = ProcessCommand::new("git")
            .arg("init")
            .arg(&repo_dir)
            .output()?;
        if !init.status.success() {
            return Err(
                format!("git init failed: {}", String::from_utf8_lossy(&init.stderr)).into(),
            );
        }

        let checkout = ProcessCommand::new("git")
            .arg("-C")
            .arg(&repo_dir)
            .args(["checkout", "-b", branch])
            .output()?;
        if !checkout.status.success() {
            return Err(format!(
                "git checkout -b failed: {}",
                String::from_utf8_lossy(&checkout.stderr)
            )
            .into());
        }

        Ok(repo_dir)
    }

    #[cfg(not(windows))]
    fn single_byte_hex_dump_command() -> noctrail_pty::PtyCommand {
        let mut command = noctrail_pty::PtyCommand::new("sh");
        command.args(["-lc", "stty raw -echo; od -An -tx1 -N1"]);
        command
    }
}
