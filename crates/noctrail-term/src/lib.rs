//! Terminal state-machine boundary for Noctrail.

use std::cmp::min;

use unicode_width::UnicodeWidthChar;
use vte::{Params, Parser, Perform};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Color {
    #[default]
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Style {
    pub foreground: Color,
    pub background: Color,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Cell {
    pub text: String,
    pub style: Style,
    pub wide_continuation: bool,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Cursor {
    pub row: usize,
    pub col: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grid {
    width: usize,
    height: usize,
    cells: Vec<Cell>,
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
        let copy_rows = min(self.height, height);
        let copy_cols = min(self.width, width);

        for row in 0..copy_rows {
            for col in 0..copy_cols {
                let old_idx = row * self.width + col;
                let new_idx = row * width + col;
                cells[new_idx] = self.cells[old_idx].clone();
            }
        }

        self.width = width;
        self.height = height;
        self.cells = cells;
        self.cursor.row = min(self.cursor.row, height - 1);
        self.cursor.col = min(self.cursor.col, width - 1);
        self.dirty_rows = vec![true; height];
    }

    pub fn advance_char(&mut self, ch: char) {
        self.advance_char_with_style(ch, Style::default());
    }

    pub fn advance_char_with_style(&mut self, ch: char, style: Style) {
        match ch {
            '\n' => self.newline(),
            '\r' => self.cursor.col = 0,
            _ => {
                let width = UnicodeWidthChar::width(ch).unwrap_or(1);
                if width == 0 {
                    self.append_combining_mark(ch);
                    return;
                }

                let width = width.min(self.width);
                if self.cursor.col + width > self.width {
                    self.newline();
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
                    self.newline();
                }
            }
        }
    }

    pub fn advance_str(&mut self, text: &str) {
        self.advance_str_with_style(text, Style::default());
    }

    pub fn advance_str_with_style(&mut self, text: &str, style: Style) {
        for ch in text.chars() {
            self.advance_char_with_style(ch, style);
        }
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

    fn newline(&mut self) {
        self.cursor.col = 0;
        if self.cursor.row + 1 < self.height {
            self.cursor.row += 1;
        }
        self.mark_dirty(self.cursor.row);
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

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TerminalSnapshot {
    pub cells: Vec<Vec<Cell>>,
    pub cursor: Cursor,
}

#[derive(Default)]
pub struct TerminalState {
    grid: Grid,
    style: Style,
    saved_cursor: Cursor,
    saved_style: Style,
    parser: Parser,
}

impl TerminalState {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            grid: Grid::new(width, height),
            style: Style::default(),
            saved_cursor: Cursor::default(),
            saved_style: Style::default(),
            parser: Parser::new(),
        }
    }

    pub fn grid(&self) -> &Grid {
        &self.grid
    }

    pub fn grid_mut(&mut self) -> &mut Grid {
        &mut self.grid
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
        self.saved_cursor = self.grid.cursor();
        self.saved_style = self.style;
    }

    pub fn restore_cursor(&mut self) {
        self.grid.cursor = self.saved_cursor;
        self.style = self.saved_style;
    }

    pub fn resize(&mut self, width: usize, height: usize) {
        self.grid.resize(width, height);
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
            cells: self.grid.snapshot_rows(),
            cursor: self.grid.cursor(),
        }
    }

    fn advance_printable(&mut self, ch: char) {
        self.grid.advance_char_with_style(ch, self.style);
    }

    fn execute_control(&mut self, byte: u8) {
        match byte {
            b'\n' => self.grid.advance_char_with_style('\n', self.style),
            b'\r' => self.grid.advance_char_with_style('\r', self.style),
            b'\t' => self.tab(),
            0x08 => self.backspace(),
            0x0c => self.reset_style(),
            _ => {}
        }
    }

    fn move_cursor_up(&mut self, count: usize) {
        self.grid.cursor.row = self.grid.cursor.row.saturating_sub(count);
    }

    fn move_cursor_down(&mut self, count: usize) {
        self.grid.cursor.row = self
            .grid
            .cursor
            .row
            .saturating_add(count)
            .min(self.grid.height() - 1);
    }

    fn move_cursor_forward(&mut self, count: usize) {
        self.grid.cursor.col = self
            .grid
            .cursor
            .col
            .saturating_add(count)
            .min(self.grid.width() - 1);
    }

    fn move_cursor_backward(&mut self, count: usize) {
        self.grid.cursor.col = self.grid.cursor.col.saturating_sub(count);
    }

    fn set_cursor_position(&mut self, row: usize, col: usize) {
        self.grid.cursor.row = min(row, self.grid.height() - 1);
        self.grid.cursor.col = min(col, self.grid.width() - 1);
    }

    fn backspace(&mut self) {
        if self.grid.cursor.col > 0 {
            self.grid.cursor.col -= 1;
        }
    }

    fn tab(&mut self) {
        let next_tab_stop = ((self.grid.cursor.col / 8) + 1) * 8;
        let spaces = next_tab_stop.saturating_sub(self.grid.cursor.col);
        for _ in 0..spaces {
            self.grid.advance_char_with_style(' ', self.style);
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
}

fn first_value(param: &[u16]) -> u16 {
    param.first().copied().unwrap_or(0)
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

    fn csi_dispatch(
        &mut self,
        params: &Params,
        _intermediates: &[u8],
        _ignore: bool,
        action: char,
    ) {
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
            's' => self.state.save_cursor(),
            'u' => self.state.restore_cursor(),
            _ => {}
        }
    }

    fn esc_dispatch(&mut self, _intermediates: &[u8], _ignore: bool, byte: u8) {
        match byte {
            b'7' => self.state.save_cursor(),
            b'8' => self.state.restore_cursor(),
            b'c' => {
                self.state.reset_style();
                self.state.set_cursor_position(0, 0);
            }
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
}
