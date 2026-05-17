//! Terminal state-machine boundary for Noctrail.

use std::cmp::min;

use unicode_width::UnicodeWidthChar;

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

                self.write_glyph(row, col, ch);
                if width == 2 && col + 1 < self.width {
                    self.write_wide_continuation(row, col + 1);
                }

                self.cursor.col += width;
                if self.cursor.col >= self.width {
                    self.newline();
                }
            }
        }
    }

    pub fn advance_str(&mut self, text: &str) {
        for ch in text.chars() {
            self.advance_char(ch);
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

    fn write_glyph(&mut self, row: usize, col: usize, ch: char) {
        if let Some(cell) = self.cell_mut(row, col) {
            cell.text.clear();
            cell.text.push(ch);
            cell.style = Style::default();
            cell.wide_continuation = false;
            self.mark_dirty(row);
        }
    }

    fn write_wide_continuation(&mut self, row: usize, col: usize) {
        if let Some(cell) = self.cell_mut(row, col) {
            *cell = Cell::wide_continuation(Style::default());
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

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TerminalState {
    grid: Grid,
}

impl TerminalState {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            grid: Grid::new(width, height),
        }
    }

    pub fn grid(&self) -> &Grid {
        &self.grid
    }

    pub fn grid_mut(&mut self) -> &mut Grid {
        &mut self.grid
    }

    pub fn resize(&mut self, width: usize, height: usize) {
        self.grid.resize(width, height);
    }

    pub fn advance_char(&mut self, ch: char) {
        self.grid.advance_char(ch);
    }

    pub fn advance_str(&mut self, text: &str) {
        self.grid.advance_str(text);
    }

    pub fn snapshot(&self) -> TerminalSnapshot {
        TerminalSnapshot {
            cells: self.grid.snapshot_rows(),
            cursor: self.grid.cursor(),
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
