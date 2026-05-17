//! Desktop app shell for Noctrail.

use std::fmt;

mod clipboard;

pub mod gui;
pub mod input;

use noctrail_layout::LayoutRect;
use noctrail_pty::{PtyCommand, PtyError, PtyExitStatus, PtySize};
use noctrail_render::{RenderBackend, RenderInput, RenderPlan, RenderRect};
use noctrail_runtime::{PaneId, PaneRuntime};
use noctrail_term::{DamageSet, LineEnding, TerminalSnapshot, TerminalState};
use thiserror::Error;

const ROOT_PANE_ID: PaneId = PaneId::new(1);

#[derive(Debug, Error)]
pub enum AppError {
    #[error("the active pane does not have a runtime")]
    MissingRuntime,
    #[error(transparent)]
    Pty(#[from] PtyError),
}

pub struct TerminalPane {
    pane_id: PaneId,
    terminal: TerminalState,
    runtime: Option<PaneRuntime>,
    terminal_size: PtySize,
    last_damage: DamageSet,
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
            last_damage: full_frame_damage(terminal_size),
        }
    }

    pub fn spawn(
        pane_id: PaneId,
        command: PtyCommand,
        terminal_size: PtySize,
    ) -> Result<Self, AppError> {
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
            last_damage: full_frame_damage(terminal_size),
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

    pub fn copy_selection_text(&self) -> Option<String> {
        self.terminal.selection_text(selection_line_ending())
    }

    pub fn process_id(&self) -> Option<u32> {
        self.runtime.as_ref().and_then(PaneRuntime::process_id)
    }

    pub fn paste_bytes(&self, text: &str) -> Vec<u8> {
        input::paste_bytes(text, self.bracketed_paste_enabled())
    }

    pub fn advance_output(&mut self, bytes: &[u8]) {
        self.last_damage = self.terminal.advance_bytes(bytes).damage;
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
        self.last_damage = full_frame_damage(size);
        let _ = self.terminal.grid_mut().take_dirty_rows();
        Ok(())
    }

    pub fn snapshot(&self) -> TerminalSnapshot {
        self.terminal.snapshot()
    }

    pub fn render_plan(&self, surface: LayoutRect, backend: RenderBackend) -> RenderPlan {
        let snapshot = self.snapshot();
        RenderPlan::from_input(RenderInput {
            viewport: RenderRect::new(
                usize::from(surface.x),
                usize::from(surface.y),
                usize::from(surface.width),
                usize::from(surface.height),
            ),
            backend,
            snapshot: &snapshot,
            damage: &self.last_damage,
        })
    }

    pub fn close_runtime(&mut self) -> Result<Option<PtyExitStatus>, AppError> {
        let runtime = self.runtime.take().ok_or(AppError::MissingRuntime)?;
        runtime.close().map_err(AppError::from)
    }
}

#[derive(Debug)]
pub struct DesktopFrame {
    pub pane_id: PaneId,
    pub surface: LayoutRect,
    pub terminal_size: PtySize,
    pub process_id: Option<u32>,
    pub render_plan: RenderPlan,
}

#[derive(Debug)]
pub struct DesktopApp {
    surface: LayoutRect,
    backend: RenderBackend,
    pane: TerminalPane,
}

impl DesktopApp {
    pub fn new(surface: LayoutRect, terminal_size: PtySize) -> Self {
        Self {
            surface,
            backend: RenderBackend::default(),
            pane: TerminalPane::new(ROOT_PANE_ID, terminal_size),
        }
    }

    pub fn spawn_shell(surface: LayoutRect, terminal_size: PtySize) -> Result<Self, AppError> {
        Ok(Self {
            surface,
            backend: RenderBackend::default(),
            pane: TerminalPane::spawn_shell(ROOT_PANE_ID, terminal_size)?,
        })
    }

    pub fn spawn(
        surface: LayoutRect,
        command: PtyCommand,
        terminal_size: PtySize,
    ) -> Result<Self, AppError> {
        Ok(Self {
            surface,
            backend: RenderBackend::default(),
            pane: TerminalPane::spawn(ROOT_PANE_ID, command, terminal_size)?,
        })
    }

    pub fn backend(&self) -> RenderBackend {
        self.backend
    }

    pub fn set_backend(&mut self, backend: RenderBackend) {
        self.backend = backend;
    }

    pub fn surface(&self) -> LayoutRect {
        self.surface
    }

    pub fn pane(&self) -> &TerminalPane {
        &self.pane
    }

    pub fn pane_mut(&mut self) -> &mut TerminalPane {
        &mut self.pane
    }

    pub fn advance_output(&mut self, bytes: &[u8]) {
        self.pane.advance_output(bytes);
    }

    pub fn write_input(&mut self, bytes: &[u8]) -> Result<usize, AppError> {
        self.pane.write_input(bytes)
    }

    pub fn paste_text(&mut self, text: &str) -> Result<usize, AppError> {
        self.pane.paste_text(text)
    }

    pub fn copy_selection_text(&self) -> Option<String> {
        self.pane.copy_selection_text()
    }

    pub fn resize(&mut self, surface: LayoutRect, terminal_size: PtySize) -> Result<(), AppError> {
        self.pane.resize(terminal_size)?;
        self.surface = surface;
        Ok(())
    }

    pub fn frame(&self) -> DesktopFrame {
        DesktopFrame {
            pane_id: self.pane.pane_id(),
            surface: self.surface,
            terminal_size: self.pane.terminal_size(),
            process_id: self.pane.process_id(),
            render_plan: self.pane.render_plan(self.surface, self.backend),
        }
    }

    pub fn close_runtime(&mut self) -> Result<Option<PtyExitStatus>, AppError> {
        self.pane.close_runtime()
    }
}

fn selection_line_ending() -> LineEnding {
    if cfg!(windows) {
        LineEnding::CrLf
    } else {
        LineEnding::Lf
    }
}

fn full_frame_damage(size: PtySize) -> DamageSet {
    DamageSet {
        dirty_rows: (0..usize::from(size.rows)).collect(),
        full_frame: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shellless_app_builds_single_pane_frame() {
        let app = DesktopApp::new(LayoutRect::new(0, 0, 120, 80), PtySize::new(10, 3));

        let frame = app.frame();
        assert_eq!(frame.pane_id, PaneId::new(1));
        assert_eq!(frame.surface, LayoutRect::new(0, 0, 120, 80));
        assert_eq!(frame.terminal_size, PtySize::new(10, 3));
        assert!(frame.process_id.is_none());
        assert_eq!(frame.render_plan.rows.len(), 3);
        assert!(frame.render_plan.damage.full_frame);
        assert_eq!(frame.render_plan.damage.dirty_rows, vec![0, 1, 2]);
        assert_eq!(frame.render_plan.scrollback_rows, 0);
        assert!(frame.render_plan.selection.is_none());
    }

    #[test]
    fn output_bytes_feed_the_render_plan() {
        let mut app = DesktopApp::new(LayoutRect::new(0, 0, 80, 24), PtySize::new(5, 2));

        app.advance_output(b"hi");

        let frame = app.frame();
        assert_eq!(frame.render_plan.rows.len(), 2);
        assert_eq!(frame.render_plan.damage.dirty_rows, vec![0]);
        assert!(!frame.render_plan.damage.full_frame);
        assert_eq!(frame.render_plan.rows[0].glyphs[0].text, "h");
        assert_eq!(frame.render_plan.rows[0].glyphs[1].text, "i");
    }

    #[test]
    fn resize_updates_terminal_size_without_runtime() -> Result<(), Box<dyn std::error::Error>> {
        let mut app = DesktopApp::new(LayoutRect::new(0, 0, 80, 24), PtySize::new(5, 2));

        app.resize(LayoutRect::new(10, 20, 160, 90), PtySize::new(7, 4))?;
        let frame = app.frame();
        assert_eq!(frame.surface, LayoutRect::new(10, 20, 160, 90));
        assert_eq!(frame.terminal_size, PtySize::new(7, 4));
        assert!(frame.render_plan.damage.full_frame);
        assert_eq!(frame.render_plan.damage.dirty_rows, vec![0, 1, 2, 3]);
        Ok(())
    }
}
