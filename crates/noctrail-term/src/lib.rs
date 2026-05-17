//! Terminal state-machine boundary for Noctrail.

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
    selection: Option<Selection>,
    parser: Parser,
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
            selection: None,
            parser: Parser::new(),
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

    pub fn selection_text(&self, line_ending: LineEnding) -> Option<String> {
        let selection = self.selection.as_ref()?.clone().normalized();
        let rows = self.active_rows();
        Some(render_selection(&rows, &selection, line_ending))
    }

    pub fn advance_char(&mut self, ch: char) {
        let mut buf = [0; 4];
        let text = ch.encode_utf8(&mut buf);
        self.advance_bytes(text.as_bytes());
    }

    pub fn advance_str(&mut self, text: &str) {
        self.advance_bytes(text.as_bytes());
    }

    pub fn advance_bytes(&mut self, bytes: &[u8]) {
        let mut parser = std::mem::take(&mut self.parser);
        {
            let mut performer = TerminalPerform { state: self };
            parser.advance(&mut performer, bytes);
        }
        self.parser = parser;
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

    fn line_feed(&mut self, wrapped: bool) {
        let scrolled = {
            let grid = self.active_grid_mut();
            grid.line_feed(wrapped)
        };

        if let Some(row) = scrolled {
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
        self.selection = None;
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

    fn osc_dispatch(&mut self, _params: &[&[u8]], _bell_terminated: bool) {}

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
}
