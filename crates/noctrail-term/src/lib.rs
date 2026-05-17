//! Terminal state-machine boundary for Noctrail.

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TerminalState;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TerminalSnapshot;

impl TerminalState {
    pub const fn new() -> Self {
        Self
    }

    pub fn snapshot(&self) -> TerminalSnapshot {
        TerminalSnapshot
    }
}
