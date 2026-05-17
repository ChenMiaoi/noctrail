//! Pane runtime registry boundary for Noctrail.

use std::collections::HashMap;

use noctrail_pty::{PtyCommand, PtyError, PtyExitStatus, PtySession, PtySize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PaneId(pub u64);

impl PaneId {
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }
}

#[derive(Debug)]
pub struct PaneRuntime {
    session: PtySession,
}

impl PaneRuntime {
    pub fn new(session: PtySession) -> Self {
        Self { session }
    }

    pub fn spawn(command: PtyCommand, size: PtySize) -> Result<Self, PtyError> {
        Ok(Self::new(PtySession::spawn(command, size)?))
    }

    pub fn spawn_shell(size: PtySize) -> Result<Self, PtyError> {
        Ok(Self::new(PtySession::spawn_shell(size)?))
    }

    pub fn session(&self) -> &PtySession {
        &self.session
    }

    pub fn session_mut(&mut self) -> &mut PtySession {
        &mut self.session
    }

    pub fn size(&self) -> PtySize {
        self.session.size()
    }

    pub fn process_id(&self) -> Option<u32> {
        self.session.process_id()
    }

    pub fn write(&mut self, bytes: &[u8]) -> Result<usize, PtyError> {
        self.session.write(bytes)
    }

    pub fn read_output(&mut self, buf: &mut [u8]) -> Result<usize, PtyError> {
        self.session.read(buf)
    }

    pub fn resize(&mut self, size: PtySize) -> Result<(), PtyError> {
        self.session.resize(size)
    }

    pub fn try_wait(&mut self) -> Result<Option<PtyExitStatus>, PtyError> {
        self.session.try_wait()
    }

    pub fn close(self) -> Result<Option<PtyExitStatus>, PtyError> {
        self.session.close()
    }
}

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("pane {0:?} was not found")]
    PaneNotFound(PaneId),
    #[error("pane id space exhausted")]
    PaneIdExhausted,
    #[error(transparent)]
    Pty(#[from] PtyError),
}

#[derive(Debug)]
pub struct PaneRuntimeRegistry {
    next_id: u64,
    panes: HashMap<PaneId, PaneRuntime>,
}

impl Default for PaneRuntimeRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl PaneRuntimeRegistry {
    pub fn new() -> Self {
        Self {
            next_id: 1,
            panes: HashMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.panes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.panes.is_empty()
    }

    pub fn contains(&self, pane_id: PaneId) -> bool {
        self.panes.contains_key(&pane_id)
    }

    pub fn get(&self, pane_id: PaneId) -> Option<&PaneRuntime> {
        self.panes.get(&pane_id)
    }

    pub fn get_mut(&mut self, pane_id: PaneId) -> Option<&mut PaneRuntime> {
        self.panes.get_mut(&pane_id)
    }

    pub fn iter(&self) -> impl Iterator<Item = (PaneId, &PaneRuntime)> {
        self.panes.iter().map(|(pane_id, pane)| (*pane_id, pane))
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (PaneId, &mut PaneRuntime)> {
        self.panes
            .iter_mut()
            .map(|(pane_id, pane)| (*pane_id, pane))
    }

    pub fn insert_with_id(&mut self, pane_id: PaneId, runtime: PaneRuntime) -> Option<PaneRuntime> {
        self.next_id = self.next_id.max(pane_id.0.saturating_add(1));
        self.panes.insert(pane_id, runtime)
    }

    pub fn insert(&mut self, runtime: PaneRuntime) -> Result<PaneId, RuntimeError> {
        let pane_id = self.allocate_id()?;
        self.panes.insert(pane_id, runtime);
        Ok(pane_id)
    }

    pub fn spawn(&mut self, command: PtyCommand, size: PtySize) -> Result<PaneId, RuntimeError> {
        let pane = PaneRuntime::spawn(command, size)?;
        self.insert(pane)
    }

    pub fn spawn_shell(&mut self, size: PtySize) -> Result<PaneId, RuntimeError> {
        let pane = PaneRuntime::spawn_shell(size)?;
        self.insert(pane)
    }

    pub fn write_input(&mut self, pane_id: PaneId, bytes: &[u8]) -> Result<usize, RuntimeError> {
        let pane = self
            .get_mut(pane_id)
            .ok_or(RuntimeError::PaneNotFound(pane_id))?;
        pane.write(bytes).map_err(Into::into)
    }

    pub fn resize_pane(&mut self, pane_id: PaneId, size: PtySize) -> Result<(), RuntimeError> {
        let pane = self
            .get_mut(pane_id)
            .ok_or(RuntimeError::PaneNotFound(pane_id))?;
        pane.resize(size).map_err(Into::into)
    }

    pub fn read_output(&mut self, pane_id: PaneId, buf: &mut [u8]) -> Result<usize, RuntimeError> {
        let pane = self
            .get_mut(pane_id)
            .ok_or(RuntimeError::PaneNotFound(pane_id))?;
        pane.read_output(buf).map_err(Into::into)
    }

    pub fn close(&mut self, pane_id: PaneId) -> Result<Option<PtyExitStatus>, RuntimeError> {
        let pane = self
            .remove(pane_id)
            .ok_or(RuntimeError::PaneNotFound(pane_id))?;
        pane.close().map_err(Into::into)
    }

    pub fn remove(&mut self, pane_id: PaneId) -> Option<PaneRuntime> {
        self.panes.remove(&pane_id)
    }

    fn allocate_id(&mut self) -> Result<PaneId, RuntimeError> {
        let pane_id = PaneId(self.next_id);
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or(RuntimeError::PaneIdExhausted)?;
        Ok(pane_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error as StdError;

    #[test]
    fn registry_tracks_shells_independently() -> Result<(), Box<dyn StdError>> {
        let mut registry = PaneRuntimeRegistry::new();
        assert!(registry.is_empty());

        let first = registry.spawn_shell(PtySize::new(80, 24))?;
        let second = registry.spawn_shell(PtySize::new(80, 24))?;

        assert_ne!(first, second);
        assert_eq!(registry.len(), 2);
        assert!(registry.contains(first));
        assert!(registry.contains(second));

        registry.resize_pane(first, PtySize::new(100, 30))?;
        assert_eq!(
            registry
                .get(first)
                .expect("first pane should still be present")
                .size(),
            PtySize::new(100, 30)
        );

        registry.write_input(second, b"exit\r\n")?;

        assert!(registry.close(first)?.is_some());
        assert_eq!(registry.len(), 1);
        assert!(!registry.contains(first));
        assert!(registry.contains(second));

        assert!(registry.close(second)?.is_some());
        assert!(registry.is_empty());

        Ok(())
    }

    #[test]
    fn insert_with_id_updates_allocator() {
        let mut registry = PaneRuntimeRegistry::new();
        let session =
            PtySession::spawn_shell(PtySize::new(80, 24)).expect("test shell should spawn");
        let runtime = PaneRuntime::new(session);

        registry.insert_with_id(PaneId::new(7), runtime);
        let next = registry
            .insert(PaneRuntime::spawn_shell(PtySize::new(80, 24)).expect("second shell"))
            .expect("allocator should still work");

        assert_eq!(next, PaneId::new(8));
        let _ = registry.close(PaneId::new(7));
        let _ = registry.close(next);
    }

    #[test]
    fn registry_reads_four_panes_without_cross_talk() -> Result<(), Box<dyn StdError>> {
        let mut registry = PaneRuntimeRegistry::new();
        let markers = ["pane-one", "pane-two", "pane-three", "pane-four"];
        let mut pane_ids = Vec::new();

        for marker in markers {
            let pane_id = registry.spawn(smoke_command(marker), PtySize::new(80, 24))?;
            pane_ids.push((pane_id, marker));
        }

        for (pane_id, marker) in pane_ids {
            let output = read_all_output(&mut registry, pane_id)?;
            let text = String::from_utf8_lossy(&output);
            assert!(
                text.contains(marker),
                "expected output for {pane_id:?} to contain {marker:?}, got {text:?}"
            );
            assert!(
                !markers
                    .iter()
                    .filter(|other| **other != marker)
                    .any(|other| text.contains(other)),
                "pane {pane_id:?} output leaked another marker: {text:?}"
            );
            assert!(registry.close(pane_id)?.is_some());
        }

        Ok(())
    }

    fn read_all_output(
        registry: &mut PaneRuntimeRegistry,
        pane_id: PaneId,
    ) -> Result<Vec<u8>, RuntimeError> {
        let mut output = Vec::new();
        let mut chunk = [0_u8; 1024];

        loop {
            let count = registry.read_output(pane_id, &mut chunk)?;
            if count == 0 {
                break;
            }
            output.extend_from_slice(&chunk[..count]);
        }

        Ok(output)
    }

    fn smoke_command(marker: &str) -> PtyCommand {
        #[cfg(windows)]
        {
            let mut command = PtyCommand::new("cmd.exe");
            command.args(["/C", "echo", marker]);
            command
        }

        #[cfg(not(windows))]
        {
            let mut command = PtyCommand::new("sh");
            command.args(["-lc", &format!("printf '{marker}'")]);
            command
        }
    }
}
