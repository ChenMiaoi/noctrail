//! Desktop app shell for Noctrail.

use std::{collections::HashMap, fmt};

mod clipboard;

pub mod gui;
pub mod input;

use noctrail_layout::{
    FocusDirection, LayoutError, LayoutRect, PaneLayout, WorkspaceId, WorkspaceSet,
};
use noctrail_pty::{PtyCommand, PtyError, PtyExitStatus, PtySize};
use noctrail_render::{RenderBackend, RenderInput, RenderPlan, RenderRect};
use noctrail_runtime::{PaneId, PaneRuntime};
use noctrail_term::{
    Cursor, DamageSet, LineEnding, MouseTrackingMode, Position, Selection, SelectionMode,
    TerminalSnapshot, TerminalState,
};
use thiserror::Error;

const ROOT_PANE_ID: PaneId = PaneId::new(1);

#[derive(Debug, Error)]
pub enum AppError {
    #[error("the active pane does not have a runtime")]
    MissingRuntime,
    #[error("the desktop app does not have an active pane")]
    MissingActivePane,
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
            scrollback_offset: 0,
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

    pub fn paste_bytes(&self, text: &str) -> Vec<u8> {
        input::paste_bytes(text, self.bracketed_paste_enabled())
    }

    pub fn advance_output(&mut self, bytes: &[u8]) {
        self.last_damage = self.terminal.advance_bytes(bytes).damage;
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
        surface: LayoutRect,
        backend: RenderBackend,
        active: bool,
    ) -> RenderPlan {
        let snapshot = self.render_snapshot();
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
            active,
        })
    }

    pub fn close_runtime(&mut self) -> Result<Option<PtyExitStatus>, AppError> {
        let runtime = self.runtime.take().ok_or(AppError::MissingRuntime)?;
        runtime.close().map_err(AppError::from)
    }

    fn clamp_scrollback_offset(&mut self) {
        let snapshot = self.snapshot();
        self.scrollback_offset = self.scrollback_offset.min(max_scrollback_offset(&snapshot));
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
    pub pane_id: PaneId,
    pub surface: LayoutRect,
    pub terminal_size: PtySize,
    pub process_id: Option<u32>,
    pub render_plan: RenderPlan,
}

#[derive(Debug)]
pub struct DesktopApp {
    surface: LayoutRect,
    terminal_size: PtySize,
    backend: RenderBackend,
    workspaces: WorkspaceSet,
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

    pub fn surface(&self) -> LayoutRect {
        self.surface
    }

    pub fn active_pane_id(&self) -> Option<PaneId> {
        self.workspaces.active_layout().active_pane()
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

    pub fn pane(&self) -> &TerminalPane {
        self.active_pane_ref()
    }

    pub fn pane_mut(&mut self) -> &mut TerminalPane {
        self.active_pane_mut()
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

    pub fn switch_workspace(&mut self, workspace_id: WorkspaceId) -> Result<PaneId, AppError> {
        self.workspaces.switch_to(workspace_id);
        let active = if let Some(pane_id) = self.active_pane_id() {
            pane_id
        } else {
            let pane_id = self.allocate_pane_id()?;
            let pane = TerminalPane::spawn_shell(pane_id, self.terminal_size)?;
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
        let pane_surface = self
            .pane_layouts()
            .into_iter()
            .find(|layout| layout.pane_id == pane_id)
            .map(|layout| layout.rect)
            .ok_or(AppError::PaneNotFound(pane_id))?;

        Ok(DesktopFrame {
            workspace_id,
            pane_id,
            surface: pane_surface,
            terminal_size: pane.terminal_size(),
            process_id: pane.process_id(),
            render_plan: pane.render_plan(pane_surface, self.backend, pane_id == active_pane),
        })
    }

    pub fn close_runtime(&mut self) -> Result<Option<PtyExitStatus>, AppError> {
        self.active_pane_mut().close_runtime()
    }

    pub fn close_active_pane(&mut self) -> Result<(PaneId, Option<PtyExitStatus>), AppError> {
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
            workspaces: WorkspaceSet::new(ROOT_PANE_ID),
            panes,
            next_pane_id: ROOT_PANE_ID.0 + 1,
        }
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
            .workspaces
            .active_layout()
            .active_pane()
            .expect("desktop app should always have an active pane");
        self.panes
            .get(&pane_id)
            .expect("layout active pane should exist in the pane registry")
    }

    fn active_pane_mut(&mut self) -> &mut TerminalPane {
        let pane_id = self
            .workspaces
            .active_layout()
            .active_pane()
            .expect("desktop app should always have an active pane");
        self.panes
            .get_mut(&pane_id)
            .expect("layout active pane should exist in the pane registry")
    }

    fn sync_pane_terminal_sizes(&mut self) -> Result<(), AppError> {
        let layouts = self.pane_layouts();
        for layout in layouts {
            let pane_size = pane_terminal_size(self.surface, self.terminal_size, layout.rect);
            self.pane_mut_by_id(layout.pane_id)
                .ok_or(AppError::PaneNotFound(layout.pane_id))?
                .resize(pane_size)?;
        }
        Ok(())
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

fn projected_cells(offset: u16, span: u16, total_span: u16, total_cells: u16) -> u16 {
    if total_span == 0 || total_cells <= 1 {
        return total_cells.max(1);
    }

    let start = (u32::from(offset) * u32::from(total_cells)) / u32::from(total_span);
    let end =
        (u32::from(offset.saturating_add(span)) * u32::from(total_cells)) / u32::from(total_span);
    end.saturating_sub(start).max(1) as u16
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error as StdError;

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
        assert_eq!(frame.pane_id, PaneId::new(1));
        assert_eq!(frame.surface, LayoutRect::new(0, 0, 120, 80));
        assert_eq!(frame.terminal_size, PtySize::new(10, 3));
        assert!(frame.process_id.is_none());
        assert_eq!(frame.render_plan.rows.len(), 3);
        assert!(frame.render_plan.damage.full_frame);
        assert_eq!(frame.render_plan.damage.dirty_rows, vec![0, 1, 2]);
        assert_eq!(frame.render_plan.scrollback_rows, 0);
        assert!(frame.render_plan.active);
        assert!(frame.render_plan.selection.is_none());
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

        let workspace_two = app.switch_workspace(WorkspaceId::new(2))?;
        assert_eq!(workspace_two, workspace_two_pane);
        assert_eq!(
            app.pane_by_id(workspace_two_pane)
                .and_then(TerminalPane::process_id),
            Some(workspace_two_pid)
        );
        assert_eq!(app.frame().workspace_id, WorkspaceId::new(2));

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

    fn render_row_text(row: &noctrail_render::RenderRow) -> String {
        row.glyphs
            .iter()
            .map(|glyph| glyph.text.as_str())
            .collect::<String>()
    }

    #[cfg(not(windows))]
    fn single_byte_hex_dump_command() -> noctrail_pty::PtyCommand {
        let mut command = noctrail_pty::PtyCommand::new("sh");
        command.args(["-lc", "stty raw -echo; od -An -tx1 -N1"]);
        command
    }
}
