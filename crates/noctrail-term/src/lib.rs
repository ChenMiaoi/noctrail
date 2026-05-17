//! Terminal state-machine boundary for Noctrail.
//!
//! Known limitations:
//! - ZWJ emoji sequences are not yet collapsed into a single terminal
//!   cell cluster.
//! - RTL shaping is not modeled beyond scalar-width cursor accounting.
//! - Current coverage targets printable cells, combining marks, and
//!   wide-cell continuation behavior rather than full text shaping.

use std::{cmp::min, collections::VecDeque};

use serde::{Deserialize, Serialize};
use unicode_width::UnicodeWidthChar;
use vte::{Params, Parser, Perform};

pub mod recording;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Color {
    #[default]
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Style {
    pub foreground: Color,
    pub background: Color,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Cell {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    #[serde(default, skip_serializing_if = "Style::is_default")]
    pub style: Style,
    #[serde(default, skip_serializing_if = "is_false")]
    pub wide_continuation: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl Cell {
    pub fn blank() -> Self {
        Self::default()
    }

    pub fn wide_continuation(style: Style) -> Self {
        Self {
            text: String::new(),
            style,
            wide_continuation: true,
        }
    }
}

impl Style {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Cursor {
    pub row: usize,
    pub col: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Position {
    pub row: usize,
    pub col: usize,
}

impl From<Cursor> for Position {
    fn from(cursor: Cursor) -> Self {
        Self {
            row: cursor.row,
            col: cursor.col,
        }
    }
}

impl From<Position> for Cursor {
    fn from(position: Position) -> Self {
        Self {
            row: position.row,
            col: position.col,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SelectionMode {
    Normal,
    Line,
    Block,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LineEnding {
    Lf,
    CrLf,
}

impl LineEnding {
    fn as_str(self) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::CrLf => "\r\n",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MouseTrackingMode {
    #[default]
    Disabled,
    Press,
    Drag,
    Motion,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShellIntegrationEvent {
    Prompt,
    CommandStart,
    CommandText(String),
    CommandEnd,
    Cwd(String),
    ExitCode(i32),
    DurationMs(u64),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Selection {
    pub mode: SelectionMode,
    pub start: Position,
    pub end: Position,
}

impl Selection {
    pub fn normalized(self) -> Self {
        let (start, end) = normalize_positions(self.start, self.end);
        Self { start, end, ..self }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenRowSnapshot {
    pub cells: Vec<Cell>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub wrapped: bool,
}

impl ScreenRowSnapshot {
    pub fn rendered_text(&self) -> String {
        render_cells(&self.cells, 0, self.cells.len(), SelectionMode::Normal)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grid {
    width: usize,
    height: usize,
    cells: Vec<Cell>,
    row_wrapped: Vec<bool>,
    cursor: Cursor,
    dirty_rows: Vec<bool>,
}

impl Grid {
    pub fn new(width: usize, height: usize) -> Self {
        let width = width.max(1);
        let height = height.max(1);
        let cells = vec![Cell::default(); width * height];

        Self {
            width,
            height,
            cells,
            row_wrapped: vec![false; height],
            cursor: Cursor::default(),
            dirty_rows: vec![true; height],
        }
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn cursor(&self) -> Cursor {
        self.cursor
    }

    pub fn row(&self, row: usize) -> Option<&[Cell]> {
        self.row_range(row)
            .map(|range| &self.cells[range.start..range.end])
    }

    pub fn row_mut(&mut self, row: usize) -> Option<&mut [Cell]> {
        self.row_range(row)
            .map(move |range| &mut self.cells[range.start..range.end])
    }

    pub fn cell(&self, row: usize, col: usize) -> Option<&Cell> {
        self.index(row, col).map(|idx| &self.cells[idx])
    }

    pub fn cell_mut(&mut self, row: usize, col: usize) -> Option<&mut Cell> {
        self.index(row, col).map(move |idx| &mut self.cells[idx])
    }

    pub fn resize(&mut self, width: usize, height: usize) {
        let width = width.max(1);
        let height = height.max(1);
        let mut cells = vec![Cell::default(); width * height];
        let mut row_wrapped = vec![false; height];
        let copy_rows = min(self.height, height);
        let copy_cols = min(self.width, width);

        for (row, wrapped) in row_wrapped.iter_mut().enumerate().take(copy_rows) {
            *wrapped = self.row_wrapped[row];
            for col in 0..copy_cols {
                let old_idx = row * self.width + col;
                let new_idx = row * width + col;
                cells[new_idx] = self.cells[old_idx].clone();
            }
        }

        self.width = width;
        self.height = height;
        self.cells = cells;
        self.row_wrapped = row_wrapped;
        self.cursor.row = min(self.cursor.row, height - 1);
        self.cursor.col = min(self.cursor.col, width - 1);
        self.dirty_rows = vec![true; height];
    }

    pub fn advance_char(&mut self, ch: char) {
        let _ = self.advance_char_with_style(ch, Style::default());
    }

    pub fn advance_char_with_style(&mut self, ch: char, style: Style) -> Vec<ScreenRowSnapshot> {
        match ch {
            '\n' => self.line_feed(false).into_iter().collect(),
            '\r' => {
                self.cursor.col = 0;
                Vec::new()
            }
            _ => {
                let width = UnicodeWidthChar::width(ch).unwrap_or(1);
                if width == 0 {
                    self.append_combining_mark(ch);
                    return Vec::new();
                }

                let width = width.min(self.width);
                let mut scrolled = Vec::new();
                if self.cursor.col + width > self.width {
                    if let Some(row) = self.wrap_line() {
                        scrolled.push(row);
                    }
                    let row = self.cursor.row;
                    let col = self.cursor.col;
                    self.write_glyph(row, col, ch, style);
                    if width == 2 && col + 1 < self.width {
                        self.write_wide_continuation(row, col + 1, style);
                    }
                    self.cursor.col += width;
                    if self.cursor.col >= self.width
                        && let Some(row) = self.wrap_line()
                    {
                        scrolled.push(row);
                    }
                    return scrolled;
                }

                let width = width.min(self.width - self.cursor.col);
                let row = self.cursor.row;
                let col = self.cursor.col;

                self.write_glyph(row, col, ch, style);
                if width == 2 && col + 1 < self.width {
                    self.write_wide_continuation(row, col + 1, style);
                }

                self.cursor.col += width;
                if self.cursor.col >= self.width {
                    if let Some(row) = self.wrap_line() {
                        scrolled.push(row);
                    }
                } else {
                    return scrolled;
                }
                scrolled
            }
        }
    }

    pub fn advance_str(&mut self, text: &str) {
        let _ = self.advance_str_with_style(text, Style::default());
    }

    pub fn advance_str_with_style(&mut self, text: &str, style: Style) -> Vec<ScreenRowSnapshot> {
        let mut rows = Vec::new();
        for ch in text.chars() {
            rows.extend(self.advance_char_with_style(ch, style));
        }
        rows
    }

    pub fn take_dirty_rows(&mut self) -> Vec<usize> {
        let mut rows = Vec::new();

        for (idx, dirty) in self.dirty_rows.iter_mut().enumerate() {
            if *dirty {
                rows.push(idx);
                *dirty = false;
            }
        }

        rows
    }

    pub fn snapshot_rows(&self) -> Vec<Vec<Cell>> {
        (0..self.height)
            .map(|row| self.row(row).map_or_else(Vec::new, |cells| cells.to_vec()))
            .collect()
    }

    pub fn snapshot_lines(&self) -> Vec<ScreenRowSnapshot> {
        (0..self.height)
            .map(|row| ScreenRowSnapshot {
                cells: self.row(row).map_or_else(Vec::new, |cells| cells.to_vec()),
                wrapped: self.row_wrapped[row],
            })
            .collect()
    }

    pub fn row_wrapped(&self, row: usize) -> Option<bool> {
        self.row_wrapped.get(row).copied()
    }

    pub fn clear_row(&mut self, row: usize) {
        if let Some(slice) = self.row_mut(row) {
            for cell in slice.iter_mut() {
                *cell = Cell::blank();
            }
            self.mark_dirty(row);
        }
    }

    pub fn clear_rows_before(&mut self, row: usize) {
        let limit = min(row, self.height);
        for current in 0..limit {
            self.clear_row(current);
        }
    }

    pub fn clear_rows_after(&mut self, row: usize) {
        for current in row.saturating_add(1)..self.height {
            self.clear_row(current);
        }
    }

    pub fn clear_row_from(&mut self, row: usize, col: usize) {
        if row >= self.height {
            return;
        }
        let start = min(col, self.width);
        for current in start..self.width {
            self.clear_cell_at(row, current);
        }
    }

    pub fn clear_row_to(&mut self, row: usize, col: usize) {
        if row >= self.height {
            return;
        }
        let end = min(col, self.width.saturating_sub(1));
        for current in 0..=end {
            self.clear_cell_at(row, current);
        }
    }

    pub fn clear_display(&mut self, row: usize, col: usize, mode: usize) {
        match mode {
            0 => {
                self.clear_row_from(row, col);
                self.clear_rows_after(row);
            }
            1 => {
                self.clear_rows_before(row);
                self.clear_row_to(row, col);
            }
            2 => {
                for current in 0..self.height {
                    self.clear_row(current);
                }
            }
            _ => {}
        }
    }

    pub fn clear_screen(&mut self) {
        for row in 0..self.height {
            self.clear_row(row);
            self.row_wrapped[row] = false;
        }
        self.cursor = Cursor::default();
        self.dirty_rows.fill(true);
    }

    pub fn scroll_up(&mut self, wrapped: bool) -> ScreenRowSnapshot {
        let removed = ScreenRowSnapshot {
            cells: self.row(0).map_or_else(Vec::new, |cells| cells.to_vec()),
            wrapped: self.row_wrapped.first().copied().unwrap_or(false),
        };

        for row in 1..self.height {
            for col in 0..self.width {
                let old_idx = row * self.width + col;
                let new_idx = (row - 1) * self.width + col;
                self.cells[new_idx] = self.cells[old_idx].clone();
            }
            self.row_wrapped[row - 1] = self.row_wrapped[row];
        }

        for col in 0..self.width {
            let idx = (self.height - 1) * self.width + col;
            self.cells[idx] = Cell::default();
        }
        self.row_wrapped[self.height - 1] = wrapped;
        self.cursor.row = self.height - 1;
        self.dirty_rows.fill(true);

        removed
    }

    pub fn line_feed(&mut self, wrapped: bool) -> Option<ScreenRowSnapshot> {
        if self.cursor.row + 1 < self.height {
            self.cursor.row += 1;
            self.row_wrapped[self.cursor.row] = wrapped;
            None
        } else {
            Some(self.scroll_up(wrapped))
        }
    }

    pub fn wrap_line(&mut self) -> Option<ScreenRowSnapshot> {
        self.cursor.col = 0;
        self.line_feed(true)
    }

    pub fn carriage_return(&mut self) {
        self.cursor.col = 0;
    }

    pub fn backspace(&mut self) {
        if self.cursor.col > 0 {
            self.cursor.col -= 1;
        } else if self.cursor.row > 0 {
            self.cursor.row -= 1;
            self.cursor.col = self.width - 1;
        }
    }

    fn row_range(&self, row: usize) -> Option<std::ops::Range<usize>> {
        if row < self.height {
            let start = row * self.width;
            Some(start..start + self.width)
        } else {
            None
        }
    }

    fn index(&self, row: usize, col: usize) -> Option<usize> {
        if row < self.height && col < self.width {
            Some(row * self.width + col)
        } else {
            None
        }
    }

    fn mark_dirty(&mut self, row: usize) {
        if let Some(dirty) = self.dirty_rows.get_mut(row) {
            *dirty = true;
        }
    }

    fn write_glyph(&mut self, row: usize, col: usize, ch: char, style: Style) {
        if let Some(cell) = self.cell_mut(row, col) {
            cell.text.clear();
            cell.text.push(ch);
            cell.style = style;
            cell.wide_continuation = false;
            self.mark_dirty(row);
        }
    }

    fn write_wide_continuation(&mut self, row: usize, col: usize, style: Style) {
        if let Some(cell) = self.cell_mut(row, col) {
            *cell = Cell::wide_continuation(style);
            self.mark_dirty(row);
        }
    }

    fn clear_cell_at(&mut self, row: usize, col: usize) {
        if row >= self.height || col >= self.width {
            return;
        }

        let idx = row * self.width + col;
        let is_wide_continuation = self.cells[idx].wide_continuation;
        let has_wide_next = col + 1 < self.width && self.cells[idx + 1].wide_continuation;

        self.cells[idx] = Cell::blank();
        if is_wide_continuation && col > 0 {
            let prev_idx = row * self.width + col - 1;
            self.cells[prev_idx] = Cell::blank();
        }
        if has_wide_next {
            self.cells[idx + 1] = Cell::blank();
        }
        self.mark_dirty(row);
    }

    fn append_combining_mark(&mut self, ch: char) {
        let row = self.cursor.row;
        let target_col = self.previous_visible_column(row);
        if let Some(cell) = self.cell_mut(row, target_col) {
            cell.text.push(ch);
            self.mark_dirty(row);
        }
    }

    fn previous_visible_column(&self, row: usize) -> usize {
        let mut col = self.cursor.col.saturating_sub(1);
        while col > 0 {
            match self.cell(row, col) {
                Some(cell) if cell.wide_continuation => col -= 1,
                _ => break,
            }
        }
        col
    }
}

impl Default for Grid {
    fn default() -> Self {
        Self::new(80, 24)
    }
}

fn osc_value(params: &[&[u8]], start: usize) -> Option<String> {
    if start >= params.len() {
        return None;
    }

    Some(
        params[start..]
            .iter()
            .map(|segment| String::from_utf8_lossy(segment).into_owned())
            .collect::<Vec<_>>()
            .join(";"),
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TerminalSnapshot {
    pub rows: Vec<ScreenRowSnapshot>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scrollback: Vec<ScreenRowSnapshot>,
    pub cursor: Cursor,
    #[serde(default, skip_serializing_if = "is_false")]
    pub alternate_screen: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub bracketed_paste: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<Selection>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DamageSet {
    pub dirty_rows: Vec<usize>,
    pub full_frame: bool,
}

impl DamageSet {
    fn from_rows(mut dirty_rows: Vec<usize>, full_frame: bool) -> Self {
        dirty_rows.sort_unstable();
        dirty_rows.dedup();
        Self {
            dirty_rows,
            full_frame,
        }
    }

    fn full_frame(height: usize) -> Self {
        Self {
            dirty_rows: (0..height).collect(),
            full_frame: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AdvanceResult {
    pub damage: DamageSet,
    pub cursor_moved: bool,
    pub scrolled: bool,
    pub alternate_screen_changed: bool,
    pub title_changed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct SavedState {
    cursor: Cursor,
    style: Style,
}

const DEFAULT_SCROLLBACK_LIMIT: usize = 10_000;

#[derive(Default)]
pub struct TerminalState {
    primary: Grid,
    alternate: Option<Grid>,
    primary_scrollback: VecDeque<ScreenRowSnapshot>,
    scrollback_limit: usize,
    style: Style,
    saved_primary: Option<SavedState>,
    saved_alternate: Option<SavedState>,
    bracketed_paste: bool,
    mouse_tracking: MouseTrackingMode,
    mouse_reporting_sgr: bool,
    selection: Option<Selection>,
    shell_integration_events: VecDeque<ShellIntegrationEvent>,
    parser: Parser,
    pending_scroll: bool,
}

impl TerminalState {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            primary: Grid::new(width, height),
            alternate: None,
            primary_scrollback: VecDeque::new(),
            scrollback_limit: DEFAULT_SCROLLBACK_LIMIT,
            style: Style::default(),
            saved_primary: None,
            saved_alternate: None,
            bracketed_paste: false,
            mouse_tracking: MouseTrackingMode::Disabled,
            mouse_reporting_sgr: false,
            selection: None,
            shell_integration_events: VecDeque::new(),
            parser: Parser::new(),
            pending_scroll: false,
        }
    }

    pub fn grid(&self) -> &Grid {
        self.active_grid()
    }

    pub fn grid_mut(&mut self) -> &mut Grid {
        self.active_grid_mut()
    }

    pub fn style(&self) -> Style {
        self.style
    }

    pub fn set_style(&mut self, style: Style) {
        self.style = style;
    }

    pub fn reset_style(&mut self) {
        self.style = Style::default();
    }

    pub fn save_cursor(&mut self) {
        let state = SavedState {
            cursor: self.active_grid().cursor(),
            style: self.style,
        };
        if self.alternate.is_some() {
            self.saved_alternate = Some(state);
        } else {
            self.saved_primary = Some(state);
        }
    }

    pub fn restore_cursor(&mut self) {
        let saved = if self.alternate.is_some() {
            self.saved_alternate
        } else {
            self.saved_primary
        };

        if let Some(saved) = saved {
            self.active_grid_mut().cursor = saved.cursor;
            self.style = saved.style;
        }
    }

    pub fn resize(&mut self, width: usize, height: usize) {
        self.primary.resize(width, height);
        if let Some(alternate) = self.alternate.as_mut() {
            alternate.resize(width, height);
        }
    }

    pub fn clear_scrollback(&mut self) {
        self.primary_scrollback.clear();
    }

    pub fn set_scrollback_limit(&mut self, limit: usize) {
        self.scrollback_limit = limit;
        while self.primary_scrollback.len() > self.scrollback_limit {
            self.primary_scrollback.pop_front();
        }
    }

    pub fn set_selection(&mut self, selection: Option<Selection>) {
        self.selection = selection.map(Selection::normalized);
    }

    pub fn selection(&self) -> Option<&Selection> {
        self.selection.as_ref()
    }

    pub fn clear_selection(&mut self) {
        self.selection = None;
    }

    pub fn bracketed_paste_mode(&self) -> bool {
        self.bracketed_paste
    }

    pub fn set_bracketed_paste_mode(&mut self, enabled: bool) {
        self.bracketed_paste = enabled;
    }

    pub fn mouse_tracking_mode(&self) -> MouseTrackingMode {
        self.mouse_tracking
    }

    pub fn mouse_reporting_enabled(&self) -> bool {
        self.mouse_tracking != MouseTrackingMode::Disabled
    }

    pub fn sgr_mouse_mode(&self) -> bool {
        self.mouse_reporting_sgr
    }

    pub fn selection_text(&self, line_ending: LineEnding) -> Option<String> {
        let selection = self.selection.as_ref()?.clone().normalized();
        let rows = self.active_rows();
        Some(render_selection(&rows, &selection, line_ending))
    }

    pub fn drain_shell_integration_events(&mut self) -> Vec<ShellIntegrationEvent> {
        self.shell_integration_events.drain(..).collect()
    }

    pub fn advance_char(&mut self, ch: char) {
        let mut buf = [0; 4];
        let text = ch.encode_utf8(&mut buf);
        self.advance_bytes(text.as_bytes());
    }

    pub fn advance_str(&mut self, text: &str) {
        self.advance_bytes(text.as_bytes());
    }

    pub fn advance_bytes(&mut self, bytes: &[u8]) -> AdvanceResult {
        let previous_cursor = self.active_grid().cursor();
        let previous_alternate_screen = self.alternate.is_some();
        self.pending_scroll = false;

        let mut parser = std::mem::take(&mut self.parser);
        {
            let mut performer = TerminalPerform { state: self };
            parser.advance(&mut performer, bytes);
        }
        self.parser = parser;

        let alternate_screen_changed = previous_alternate_screen != self.alternate.is_some();
        let current_cursor = self.active_grid().cursor();
        let cursor_moved = previous_cursor != current_cursor || alternate_screen_changed;

        if !alternate_screen_changed && cursor_moved {
            let grid = self.active_grid_mut();
            grid.mark_dirty(previous_cursor.row);
            grid.mark_dirty(current_cursor.row);
        }

        let damage = if alternate_screen_changed {
            let height = self.active_grid().height();
            self.active_grid_mut().take_dirty_rows();
            DamageSet::full_frame(height)
        } else {
            DamageSet::from_rows(self.active_grid_mut().take_dirty_rows(), false)
        };

        AdvanceResult {
            damage,
            cursor_moved,
            scrolled: std::mem::take(&mut self.pending_scroll),
            alternate_screen_changed,
            title_changed: false,
        }
    }

    pub fn snapshot(&self) -> TerminalSnapshot {
        TerminalSnapshot {
            rows: self.active_grid().snapshot_lines(),
            scrollback: if self.alternate.is_none() {
                self.primary_scrollback.iter().cloned().collect()
            } else {
                Vec::new()
            },
            cursor: self.active_grid().cursor(),
            alternate_screen: self.alternate.is_some(),
            bracketed_paste: self.bracketed_paste,
            selection: self.selection.clone(),
        }
    }

    pub fn enter_alternate_screen(&mut self) {
        if self.alternate.is_none() {
            let width = self.primary.width();
            let height = self.primary.height();
            self.alternate = Some(Grid::new(width, height));
            self.saved_alternate = None;
            self.clear_selection();
        }
    }

    pub fn exit_alternate_screen(&mut self) {
        if self.alternate.is_some() {
            self.alternate = None;
            self.saved_alternate = None;
            self.clear_selection();
        }
    }

    fn active_grid(&self) -> &Grid {
        if let Some(alternate) = self.alternate.as_ref() {
            alternate
        } else {
            &self.primary
        }
    }

    fn active_grid_mut(&mut self) -> &mut Grid {
        if let Some(alternate) = self.alternate.as_mut() {
            alternate
        } else {
            &mut self.primary
        }
    }

    fn active_rows(&self) -> Vec<ScreenRowSnapshot> {
        if let Some(alternate) = self.alternate.as_ref() {
            alternate.snapshot_lines()
        } else {
            let mut rows = self.primary_scrollback.iter().cloned().collect::<Vec<_>>();
            rows.extend(self.primary.snapshot_lines());
            rows
        }
    }

    fn push_scrollback(&mut self, row: ScreenRowSnapshot) {
        if self.alternate.is_some() || self.scrollback_limit == 0 {
            return;
        }

        self.primary_scrollback.push_back(row);
        while self.primary_scrollback.len() > self.scrollback_limit {
            self.primary_scrollback.pop_front();
        }
    }

    fn push_shell_integration_event(&mut self, event: ShellIntegrationEvent) {
        self.shell_integration_events.push_back(event);
    }

    fn handle_osc_dispatch(&mut self, params: &[&[u8]]) {
        if params.len() < 3 || params[0] != b"1337" || params[1] != b"Noctrail" {
            return;
        }

        match params[2] {
            b"Prompt" => self.push_shell_integration_event(ShellIntegrationEvent::Prompt),
            b"CommandStart" => {
                self.push_shell_integration_event(ShellIntegrationEvent::CommandStart);
            }
            b"CommandText" => {
                if let Some(value) = osc_value(params, 3) {
                    self.push_shell_integration_event(ShellIntegrationEvent::CommandText(value));
                }
            }
            b"CommandEnd" => {
                self.push_shell_integration_event(ShellIntegrationEvent::CommandEnd);
            }
            b"Cwd" => {
                if let Some(value) = osc_value(params, 3) {
                    self.push_shell_integration_event(ShellIntegrationEvent::Cwd(value));
                }
            }
            b"ExitCode" => {
                if let Some(value) =
                    osc_value(params, 3).and_then(|value| value.trim().parse::<i32>().ok())
                {
                    self.push_shell_integration_event(ShellIntegrationEvent::ExitCode(value));
                }
            }
            b"DurationMs" => {
                if let Some(value) =
                    osc_value(params, 3).and_then(|value| value.trim().parse::<u64>().ok())
                {
                    self.push_shell_integration_event(ShellIntegrationEvent::DurationMs(value));
                }
            }
            _ => {}
        }
    }

    fn line_feed(&mut self, wrapped: bool) {
        let scrolled = {
            let grid = self.active_grid_mut();
            grid.line_feed(wrapped)
        };

        if let Some(row) = scrolled {
            self.pending_scroll = true;
            self.push_scrollback(row);
        }
    }

    fn carriage_return(&mut self) {
        self.active_grid_mut().carriage_return();
    }

    fn backspace(&mut self) {
        self.active_grid_mut().backspace();
    }

    fn advance_printable(&mut self, ch: char) {
        let style = self.style;
        let scrolled = {
            let grid = self.active_grid_mut();
            grid.advance_char_with_style(ch, style)
        };

        if !scrolled.is_empty() {
            self.pending_scroll = true;
        }
        for row in scrolled {
            self.push_scrollback(row);
        }
    }

    fn execute_control(&mut self, byte: u8) {
        match byte {
            b'\n' => self.line_feed(false),
            b'\r' => self.carriage_return(),
            b'\t' => self.tab(),
            0x08 => self.backspace(),
            0x0c => self.reset_style(),
            _ => {}
        }
    }

    fn move_cursor_up(&mut self, count: usize) {
        let row = self.active_grid().cursor.row.saturating_sub(count);
        self.active_grid_mut().cursor.row = row;
    }

    fn move_cursor_down(&mut self, count: usize) {
        let height = self.active_grid().height();
        let row = self
            .active_grid()
            .cursor
            .row
            .saturating_add(count)
            .min(height - 1);
        self.active_grid_mut().cursor.row = row;
    }

    fn move_cursor_forward(&mut self, count: usize) {
        let width = self.active_grid().width();
        let col = self
            .active_grid()
            .cursor
            .col
            .saturating_add(count)
            .min(width - 1);
        self.active_grid_mut().cursor.col = col;
    }

    fn move_cursor_backward(&mut self, count: usize) {
        let col = self.active_grid().cursor.col.saturating_sub(count);
        self.active_grid_mut().cursor.col = col;
    }

    fn set_cursor_position(&mut self, row: usize, col: usize) {
        let height = self.active_grid().height();
        let width = self.active_grid().width();
        let grid = self.active_grid_mut();
        grid.cursor.row = min(row, height - 1);
        grid.cursor.col = min(col, width - 1);
    }

    fn tab(&mut self) {
        let next_tab_stop = ((self.active_grid().cursor.col / 8) + 1) * 8;
        let spaces = next_tab_stop.saturating_sub(self.active_grid().cursor.col);
        for _ in 0..spaces {
            self.advance_printable(' ');
        }
    }

    fn apply_sgr(&mut self, params: &Params) {
        let mut iter = params.iter();
        let mut saw_param = false;

        while let Some(param) = iter.next() {
            saw_param = true;
            let value = first_value(param);
            match value {
                0 => self.reset_style(),
                1 => self.style.bold = true,
                3 => self.style.italic = true,
                4 => self.style.underline = true,
                22 => self.style.bold = false,
                23 => self.style.italic = false,
                24 => self.style.underline = false,
                30..=37 => self.style.foreground = Color::Indexed((value - 30) as u8),
                39 => self.style.foreground = Color::Default,
                40..=47 => self.style.background = Color::Indexed((value - 40) as u8),
                49 => self.style.background = Color::Default,
                90..=97 => self.style.foreground = Color::Indexed((value - 90 + 8) as u8),
                100..=107 => self.style.background = Color::Indexed((value - 100 + 8) as u8),
                38 | 48 => {
                    let target = if value == 38 {
                        &mut self.style.foreground
                    } else {
                        &mut self.style.background
                    };
                    let mode = iter.next().map(first_value).unwrap_or(0);
                    match mode {
                        2 => {
                            let red = iter.next().map(first_value).unwrap_or(0) as u8;
                            let green = iter.next().map(first_value).unwrap_or(0) as u8;
                            let blue = iter.next().map(first_value).unwrap_or(0) as u8;
                            *target = Color::Rgb(red, green, blue);
                        }
                        5 => {
                            let index = iter.next().map(first_value).unwrap_or(0) as u8;
                            *target = Color::Indexed(index);
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }

        if !saw_param {
            self.reset_style();
        }
    }

    fn erase_in_line(&mut self, mode: usize) {
        let cursor = self.active_grid().cursor();
        let grid = self.active_grid_mut();
        match mode {
            0 => grid.clear_row_from(cursor.row, cursor.col),
            1 => grid.clear_row_to(cursor.row, cursor.col),
            2 => grid.clear_row(cursor.row),
            _ => {}
        }
    }

    fn erase_in_display(&mut self, mode: usize) {
        match mode {
            3 => {
                if self.alternate.is_none() {
                    self.primary_scrollback.clear();
                }
            }
            0..=2 => {
                let cursor = self.active_grid().cursor();
                let grid = self.active_grid_mut();
                grid.clear_display(cursor.row, cursor.col, mode);
            }
            _ => {}
        }
    }

    fn reset_terminal(&mut self) {
        self.primary.clear_screen();
        self.primary_scrollback.clear();
        self.alternate = None;
        self.style = Style::default();
        self.saved_primary = None;
        self.saved_alternate = None;
        self.bracketed_paste = false;
        self.mouse_tracking = MouseTrackingMode::Disabled;
        self.mouse_reporting_sgr = false;
        self.selection = None;
    }

    fn set_mouse_tracking_mode(&mut self, mode: MouseTrackingMode) {
        self.mouse_tracking = mode;
    }

    fn set_sgr_mouse_mode(&mut self, enabled: bool) {
        self.mouse_reporting_sgr = enabled;
    }

    fn has_private_mode(intermediates: &[u8]) -> bool {
        intermediates.contains(&b'?')
    }
}

fn first_value(param: &[u16]) -> u16 {
    param.first().copied().unwrap_or(0)
}

fn normalize_positions(start: Position, end: Position) -> (Position, Position) {
    if position_before(end, start) {
        (end, start)
    } else {
        (start, end)
    }
}

fn position_before(left: Position, right: Position) -> bool {
    left.row < right.row || (left.row == right.row && left.col < right.col)
}

fn render_cells(cells: &[Cell], start_col: usize, end_col: usize, mode: SelectionMode) -> String {
    let start_col = start_col.min(cells.len());
    let end_col = end_col.min(cells.len());
    let mut output = String::new();

    for cell in &cells[start_col..end_col] {
        if cell.wide_continuation {
            if matches!(mode, SelectionMode::Block) {
                output.push(' ');
            }
            continue;
        }

        if cell.text.is_empty() {
            output.push(' ');
        } else {
            output.push_str(&cell.text);
        }
    }

    output
}

fn render_selection(
    rows: &[ScreenRowSnapshot],
    selection: &Selection,
    line_ending: LineEnding,
) -> String {
    if rows.is_empty() {
        return String::new();
    }

    let selection = selection.clone().normalized();
    let mut start_row = selection.start.row.min(rows.len() - 1);
    let mut end_row = selection.end.row.min(rows.len() - 1);

    if matches!(selection.mode, SelectionMode::Line) {
        while start_row > 0 && rows[start_row].wrapped {
            start_row -= 1;
        }

        while end_row + 1 < rows.len() && rows[end_row + 1].wrapped {
            end_row += 1;
        }
    }

    let mut output = String::new();
    for row_idx in start_row..=end_row {
        let row = &rows[row_idx];
        let (start_col, end_col) = match selection.mode {
            SelectionMode::Block => (
                selection.start.col.min(selection.end.col),
                selection.start.col.max(selection.end.col) + 1,
            ),
            SelectionMode::Line => (0, row.cells.len()),
            SelectionMode::Normal => {
                if row_idx == start_row && row_idx == end_row {
                    (
                        selection.start.col.min(selection.end.col),
                        selection.start.col.max(selection.end.col) + 1,
                    )
                } else if row_idx == start_row {
                    (selection.start.col, row.cells.len())
                } else if row_idx == end_row {
                    (0, selection.end.col + 1)
                } else {
                    (0, row.cells.len())
                }
            }
        };

        let mut row_text = render_cells(&row.cells, start_col, end_col, selection.mode);
        if matches!(selection.mode, SelectionMode::Line) {
            row_text = row_text.trim_end_matches(' ').to_owned();
        }
        output.push_str(&row_text);

        if row_idx < end_row {
            let should_break = match selection.mode {
                SelectionMode::Block => true,
                _ => !rows[row_idx + 1].wrapped,
            };
            if should_break {
                output.push_str(line_ending.as_str());
            }
        }
    }

    output
}

struct TerminalPerform<'a> {
    state: &'a mut TerminalState,
}

impl Perform for TerminalPerform<'_> {
    fn print(&mut self, c: char) {
        self.state.advance_printable(c);
    }

    fn execute(&mut self, byte: u8) {
        self.state.execute_control(byte);
    }

    fn hook(&mut self, _params: &Params, _intermediates: &[u8], _ignore: bool, _byte: char) {}

    fn put(&mut self, _byte: u8) {}

    fn unhook(&mut self) {}

    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        self.state.handle_osc_dispatch(params);
    }

    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], _ignore: bool, action: char) {
        let mut values = params.iter().map(first_value);
        match action {
            'A' => self
                .state
                .move_cursor_up(values.next().unwrap_or(1) as usize),
            'B' => self
                .state
                .move_cursor_down(values.next().unwrap_or(1) as usize),
            'C' => self
                .state
                .move_cursor_forward(values.next().unwrap_or(1) as usize),
            'D' => self
                .state
                .move_cursor_backward(values.next().unwrap_or(1) as usize),
            'H' | 'f' => {
                let row = values.next().unwrap_or(1).saturating_sub(1) as usize;
                let col = values.next().unwrap_or(1).saturating_sub(1) as usize;
                self.state.set_cursor_position(row, col);
            }
            'm' => self.state.apply_sgr(params),
            'J' => self
                .state
                .erase_in_display(values.next().unwrap_or(0) as usize),
            'K' => self
                .state
                .erase_in_line(values.next().unwrap_or(0) as usize),
            's' => self.state.save_cursor(),
            'u' => self.state.restore_cursor(),
            'h' | 'l' if TerminalState::has_private_mode(intermediates) => {
                let mode = values.next().unwrap_or(0);
                match (mode, action) {
                    (47 | 1047 | 1049, 'h') => self.state.enter_alternate_screen(),
                    (47 | 1047 | 1049, 'l') => self.state.exit_alternate_screen(),
                    (1000, 'h') => self.state.set_mouse_tracking_mode(MouseTrackingMode::Press),
                    (1000, 'l') => self
                        .state
                        .set_mouse_tracking_mode(MouseTrackingMode::Disabled),
                    (1002, 'h') => self.state.set_mouse_tracking_mode(MouseTrackingMode::Drag),
                    (1002, 'l') => self
                        .state
                        .set_mouse_tracking_mode(MouseTrackingMode::Disabled),
                    (1003, 'h') => self
                        .state
                        .set_mouse_tracking_mode(MouseTrackingMode::Motion),
                    (1003, 'l') => self
                        .state
                        .set_mouse_tracking_mode(MouseTrackingMode::Disabled),
                    (1006, 'h') => self.state.set_sgr_mouse_mode(true),
                    (1006, 'l') => self.state.set_sgr_mouse_mode(false),
                    (2004, 'h') => self.state.set_bracketed_paste_mode(true),
                    (2004, 'l') => self.state.set_bracketed_paste_mode(false),
                    (1048, 'h') => self.state.save_cursor(),
                    (1048, 'l') => self.state.restore_cursor(),
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn esc_dispatch(&mut self, _intermediates: &[u8], _ignore: bool, byte: u8) {
        match byte {
            b'7' => self.state.save_cursor(),
            b'8' => self.state.restore_cursor(),
            b'c' => self.state.reset_terminal(),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_ascii_characters_into_separate_cells() {
        let mut terminal = TerminalState::new(4, 2);

        terminal.advance_str("ab");

        let row = terminal.grid().row(0).expect("row 0");
        assert_eq!(row[0].text, "a");
        assert_eq!(row[1].text, "b");
        assert_eq!(terminal.grid().cursor(), Cursor { row: 0, col: 2 });
    }

    #[test]
    fn wide_characters_reserve_the_following_cell() {
        let mut terminal = TerminalState::new(4, 1);

        terminal.advance_char('中');

        let row = terminal.grid().row(0).expect("row 0");
        assert_eq!(row[0].text, "中");
        assert!(!row[0].wide_continuation);
        assert!(row[1].wide_continuation);
        assert_eq!(terminal.grid().cursor(), Cursor { row: 0, col: 2 });
    }

    #[test]
    fn emoji_and_combining_marks_follow_the_base_character() {
        let mut terminal = TerminalState::new(4, 1);

        terminal.advance_char('e');
        terminal.advance_char('\u{301}');
        terminal.advance_char('🙂');

        let row = terminal.grid().row(0).expect("row 0");
        assert_eq!(row[0].text, "e\u{301}");
        assert_eq!(row[1].text, "🙂");
        assert!(row[2].wide_continuation);
    }

    #[test]
    fn resize_preserves_the_visible_prefix_and_marks_rows_dirty() {
        let mut terminal = TerminalState::new(3, 2);
        terminal.advance_str("abc");

        terminal.resize(4, 2);
        let rows = terminal.grid_mut().take_dirty_rows();

        assert_eq!(rows, vec![0, 1]);
        let row = terminal.grid().row(0).expect("row 0");
        assert_eq!(row[0].text, "a");
        assert_eq!(row[1].text, "b");
        assert_eq!(row[2].text, "c");
        assert_eq!(terminal.grid().cursor(), Cursor { row: 1, col: 0 });
    }

    #[test]
    fn bracketed_paste_mode_tracks_private_csi_sequences() {
        let mut terminal = TerminalState::new(4, 1);

        assert!(!terminal.bracketed_paste_mode());
        terminal.advance_bytes(b"\x1b[?2004h");
        assert!(terminal.bracketed_paste_mode());
        terminal.advance_bytes(b"\x1b[?2004l");
        assert!(!terminal.bracketed_paste_mode());
    }

    #[test]
    fn mouse_tracking_modes_follow_private_csi_sequences() {
        let mut terminal = TerminalState::new(4, 1);

        assert_eq!(terminal.mouse_tracking_mode(), MouseTrackingMode::Disabled);
        assert!(!terminal.sgr_mouse_mode());

        terminal.advance_bytes(b"\x1b[?1000h");
        assert_eq!(terminal.mouse_tracking_mode(), MouseTrackingMode::Press);

        terminal.advance_bytes(b"\x1b[?1002h");
        assert_eq!(terminal.mouse_tracking_mode(), MouseTrackingMode::Drag);

        terminal.advance_bytes(b"\x1b[?1003h");
        assert_eq!(terminal.mouse_tracking_mode(), MouseTrackingMode::Motion);

        terminal.advance_bytes(b"\x1b[?1006h");
        assert!(terminal.sgr_mouse_mode());

        terminal.advance_bytes(b"\x1b[?1003l");
        terminal.advance_bytes(b"\x1b[?1006l");
        assert_eq!(terminal.mouse_tracking_mode(), MouseTrackingMode::Disabled);
        assert!(!terminal.sgr_mouse_mode());
    }

    #[test]
    fn noctrail_osc_markers_emit_events_without_visible_cells() {
        let mut terminal = TerminalState::new(8, 2);

        terminal.advance_bytes(b"\x1b]1337;Noctrail;Prompt\x07");
        terminal.advance_bytes(b"\x1b]1337;Noctrail;CommandStart\x1b\\");
        terminal.advance_bytes(b"\x1b]1337;Noctrail;Cwd;/tmp/noctrail\x07");
        terminal.advance_bytes(b"\x1b]1337;Noctrail;ExitCode;42\x07");
        terminal.advance_bytes(b"\x1b]1337;Noctrail;CommandEnd\x07");

        let snapshot = terminal.snapshot();
        assert!(
            snapshot
                .rows
                .iter()
                .all(|row| row.rendered_text().trim().is_empty())
        );
        assert_eq!(
            terminal.drain_shell_integration_events(),
            vec![
                ShellIntegrationEvent::Prompt,
                ShellIntegrationEvent::CommandStart,
                ShellIntegrationEvent::Cwd("/tmp/noctrail".to_string()),
                ShellIntegrationEvent::ExitCode(42),
                ShellIntegrationEvent::CommandEnd,
            ]
        );
    }

    #[test]
    fn unknown_osc_markers_are_ignored() {
        let mut terminal = TerminalState::new(4, 1);

        terminal.advance_bytes(b"\x1b]1337;Other;Prompt\x07");
        terminal.advance_bytes(b"\x1b]1337;Noctrail;ExitCode;nope\x07");

        assert!(terminal.drain_shell_integration_events().is_empty());
    }

    #[test]
    fn osc_command_text_and_duration_values_round_trip() {
        let mut terminal = TerminalState::new(8, 2);

        terminal.advance_bytes(b"\x1b]1337;Noctrail;CommandText;printf foo;pwd\x07");
        terminal.advance_bytes(b"\x1b]1337;Noctrail;DurationMs;1200\x07");

        assert_eq!(
            terminal.drain_shell_integration_events(),
            vec![
                ShellIntegrationEvent::CommandText("printf foo;pwd".to_string()),
                ShellIntegrationEvent::DurationMs(1200),
            ]
        );
    }

    #[test]
    fn cursor_only_movements_mark_old_and_new_rows_dirty() {
        let mut terminal = TerminalState::new(4, 2);
        let _ = terminal.grid_mut().take_dirty_rows();

        let first = terminal.advance_bytes(b"ab");
        assert_eq!(first.damage.dirty_rows, vec![0]);
        assert!(first.cursor_moved);
        assert!(!first.scrolled);

        let second = terminal.advance_bytes(b"\n");
        assert_eq!(second.damage.dirty_rows, vec![0, 1]);
        assert!(second.cursor_moved);
        assert!(!second.scrolled);
    }

    #[test]
    fn alternate_screen_switch_reports_full_frame_damage() {
        let mut terminal = TerminalState::new(3, 2);
        let _ = terminal.grid_mut().take_dirty_rows();

        let result = terminal.advance_bytes(b"\x1b[?1049h");

        assert!(result.alternate_screen_changed);
        assert!(result.damage.full_frame);
        assert_eq!(result.damage.dirty_rows, vec![0, 1]);
    }

    #[test]
    fn scroll_events_are_reported_by_advance_result() {
        let mut terminal = TerminalState::new(2, 1);
        let _ = terminal.grid_mut().take_dirty_rows();

        let result = terminal.advance_bytes(b"abc");

        assert!(result.scrolled);
        assert_eq!(result.damage.dirty_rows, vec![0]);
    }

    #[test]
    fn long_mixed_input_does_not_panic() {
        let mut terminal = TerminalState::new(80, 24);
        let mut input = Vec::with_capacity(100_000);
        let mut state = 0x1234_5678_u64;

        while input.len() < 100_000 {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            match state % 9 {
                0 => input.extend_from_slice(b"\x1b[31m"),
                1 => input.extend_from_slice(b"\x1b[0m"),
                2 => input.extend_from_slice(b"\x1b[38;5;201m"),
                3 => input.extend_from_slice("中".as_bytes()),
                4 => input.extend_from_slice("e\u{301}".as_bytes()),
                5 => input.push(b'\n'),
                6 => input.push(b'\r'),
                7 => input.push(b'\t'),
                _ => input.push(b'a' + (state % 26) as u8),
            }
        }

        input.truncate(100_000);
        let result = terminal.advance_bytes(&input);

        assert!(!result.damage.dirty_rows.is_empty());
        assert_eq!(terminal.snapshot().rows.len(), 24);
    }
}
