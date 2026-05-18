use std::path::PathBuf;

use noctrail_agent::{CommandProposal, PatchPreview};

use crate::features::blocks::CommandBlock;

pub(crate) const MAX_AUDIT_ENTRIES: usize = 200;

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
pub(crate) struct AgentAuditLedger {
    entries: Vec<AgentAuditEntry>,
    selected: Option<usize>,
}

impl AgentAuditLedger {
    pub(crate) fn entries(&self) -> &[AgentAuditEntry] {
        &self.entries
    }

    pub(crate) fn selected_index(&self) -> Option<usize> {
        self.selected
    }

    pub(crate) fn selected(&self) -> Option<&AgentAuditEntry> {
        self.selected.and_then(|index| self.entries.get(index))
    }

    pub(crate) fn push(&mut self, entry: AgentAuditEntry) {
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

    pub(crate) fn select_oldest(&mut self) -> Option<usize> {
        self.selected = (!self.entries.is_empty()).then_some(0);
        self.selected
    }

    pub(crate) fn select_newest(&mut self) -> Option<usize> {
        self.selected = self.entries.len().checked_sub(1);
        self.selected
    }

    pub(crate) fn select_previous(&mut self) -> Option<usize> {
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

    pub(crate) fn select_next(&mut self) -> Option<usize> {
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

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct AgentProposalState {
    proposals: Vec<CommandProposal>,
    selected: Option<usize>,
}

impl AgentProposalState {
    pub(crate) fn proposals(&self) -> &[CommandProposal] {
        &self.proposals
    }

    pub(crate) fn selected_index(&self) -> Option<usize> {
        self.selected
    }

    pub(crate) fn selected(&self) -> Option<&CommandProposal> {
        self.selected.and_then(|index| self.proposals.get(index))
    }

    pub(crate) fn set_proposals(&mut self, proposals: Vec<CommandProposal>) {
        self.proposals = proposals;
        self.selected = (!self.proposals.is_empty()).then_some(0);
    }

    pub(crate) fn clear(&mut self) {
        self.proposals.clear();
        self.selected = None;
    }

    pub(crate) fn select_oldest(&mut self) -> Option<usize> {
        self.selected = (!self.proposals.is_empty()).then_some(0);
        self.selected
    }

    pub(crate) fn select_newest(&mut self) -> Option<usize> {
        self.selected = self.proposals.len().checked_sub(1);
        self.selected
    }

    pub(crate) fn select_previous(&mut self) -> Option<usize> {
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

    pub(crate) fn select_next(&mut self) -> Option<usize> {
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

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct AgentPatchPreviewState {
    previews: Vec<PatchPreview>,
    selected: Option<usize>,
}

impl AgentPatchPreviewState {
    pub(crate) fn previews(&self) -> &[PatchPreview] {
        &self.previews
    }

    pub(crate) fn selected_index(&self) -> Option<usize> {
        self.selected
    }

    pub(crate) fn selected(&self) -> Option<&PatchPreview> {
        self.selected.and_then(|index| self.previews.get(index))
    }

    pub(crate) fn set_previews(&mut self, previews: Vec<PatchPreview>) {
        self.previews = previews;
        self.selected = (!self.previews.is_empty()).then_some(0);
    }

    pub(crate) fn clear(&mut self) {
        self.previews.clear();
        self.selected = None;
    }

    pub(crate) fn select_oldest(&mut self) -> Option<usize> {
        self.selected = (!self.previews.is_empty()).then_some(0);
        self.selected
    }

    pub(crate) fn select_newest(&mut self) -> Option<usize> {
        self.selected = self.previews.len().checked_sub(1);
        self.selected
    }

    pub(crate) fn select_previous(&mut self) -> Option<usize> {
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

    pub(crate) fn select_next(&mut self) -> Option<usize> {
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

pub(crate) fn preview_agent_summary(text: &str) -> String {
    let redacted = crate::redaction::redact_secret_text(text);
    let normalized = redacted.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut preview = normalized.chars().take(48).collect::<String>();
    if normalized.chars().count() > 48 {
        preview.push_str("...");
    }
    preview
}
