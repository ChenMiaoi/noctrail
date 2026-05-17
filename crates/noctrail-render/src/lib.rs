//! Render plan and backend boundary for Noctrail.

use noctrail_term::{
    Cell, Cursor, DamageSet, ScreenRowSnapshot, Selection, Style, TerminalSnapshot,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RenderBackend {
    Gpu,
    #[default]
    Software,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RenderRect {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

impl RenderRect {
    pub const fn new(x: usize, y: usize, width: usize, height: usize) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderGlyph {
    pub col: usize,
    pub text: String,
    pub style: Style,
    pub span: usize,
    pub wide_continuation: bool,
}

impl RenderGlyph {
    fn from_cell(col: usize, cells: &[Cell]) -> Self {
        let cell = &cells[col];
        let wide_continuation = cell.wide_continuation;
        let span = if wide_continuation {
            0
        } else if col + 1 < cells.len() && cells[col + 1].wide_continuation {
            2
        } else {
            1
        };

        Self {
            col,
            text: cell.text.clone(),
            style: cell.style,
            span,
            wide_continuation,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderRow {
    pub row: usize,
    pub wrapped: bool,
    pub glyphs: Vec<RenderGlyph>,
}

impl RenderRow {
    fn from_snapshot(row: usize, snapshot: &ScreenRowSnapshot) -> Self {
        let glyphs = snapshot
            .cells
            .iter()
            .enumerate()
            .map(|(col, _)| RenderGlyph::from_cell(col, &snapshot.cells))
            .collect();

        Self {
            row,
            wrapped: snapshot.wrapped,
            glyphs,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RenderInput<'a> {
    pub viewport: RenderRect,
    pub backend: RenderBackend,
    pub snapshot: &'a TerminalSnapshot,
    pub damage: &'a DamageSet,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RenderPlan {
    pub backend: RenderBackend,
    pub viewport: RenderRect,
    pub damage: DamageSet,
    pub scrollback_rows: usize,
    pub cursor: Cursor,
    pub alternate_screen: bool,
    pub selection: Option<Selection>,
    pub rows: Vec<RenderRow>,
}

impl RenderPlan {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_terminal(
        viewport: RenderRect,
        backend: RenderBackend,
        snapshot: &TerminalSnapshot,
    ) -> Self {
        Self::from_input(RenderInput {
            viewport,
            backend,
            snapshot,
            damage: &DamageSet {
                dirty_rows: (0..snapshot.rows.len()).collect(),
                full_frame: true,
            },
        })
    }

    pub fn from_input(input: RenderInput<'_>) -> Self {
        Self {
            backend: input.backend,
            viewport: input.viewport,
            damage: input.damage.clone(),
            scrollback_rows: input.snapshot.scrollback.len(),
            cursor: input.snapshot.cursor,
            alternate_screen: input.snapshot.alternate_screen,
            selection: input.snapshot.selection.clone(),
            rows: input
                .snapshot
                .rows
                .iter()
                .enumerate()
                .map(|(row, snapshot)| RenderRow::from_snapshot(row, snapshot))
                .collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty() && self.scrollback_rows == 0
    }
}

#[derive(Debug, Default)]
pub struct RenderSurface;

#[cfg(test)]
mod tests {
    use super::*;
    use noctrail_term::{Color, ScreenRowSnapshot, SelectionMode};

    fn cell(text: &str) -> Cell {
        Cell {
            text: text.to_owned(),
            style: Style {
                foreground: Color::Indexed(7),
                background: Color::Indexed(0),
                bold: false,
                italic: false,
                underline: false,
            },
            wide_continuation: false,
        }
    }

    fn wide_continuation_cell() -> Cell {
        Cell {
            text: String::new(),
            style: Style {
                foreground: Color::Indexed(7),
                background: Color::Indexed(0),
                bold: false,
                italic: false,
                underline: false,
            },
            wide_continuation: true,
        }
    }

    #[test]
    fn from_terminal_copies_snapshot_metadata() {
        let snapshot = TerminalSnapshot {
            rows: vec![ScreenRowSnapshot {
                cells: vec![cell("h"), cell("i")],
                wrapped: false,
            }],
            scrollback: vec![ScreenRowSnapshot {
                cells: vec![cell("o"), cell("l")],
                wrapped: true,
            }],
            cursor: Cursor { row: 1, col: 2 },
            alternate_screen: true,
            bracketed_paste: false,
            selection: Some(Selection {
                mode: SelectionMode::Line,
                start: noctrail_term::Position { row: 0, col: 0 },
                end: noctrail_term::Position { row: 0, col: 1 },
            }),
        };

        let plan =
            RenderPlan::from_terminal(RenderRect::new(1, 2, 80, 24), RenderBackend::Gpu, &snapshot);

        assert_eq!(plan.backend, RenderBackend::Gpu);
        assert_eq!(plan.viewport, RenderRect::new(1, 2, 80, 24));
        assert!(plan.damage.full_frame);
        assert_eq!(plan.damage.dirty_rows, vec![0]);
        assert_eq!(plan.scrollback_rows, 1);
        assert_eq!(plan.cursor, snapshot.cursor);
        assert!(plan.alternate_screen);
        assert_eq!(plan.selection, snapshot.selection);
        assert_eq!(plan.rows.len(), 1);
        assert_eq!(plan.rows[0].row, 0);
        assert!(!plan.rows[0].wrapped);
    }

    #[test]
    fn glyph_path_marks_wide_cells() {
        let snapshot = TerminalSnapshot {
            rows: vec![ScreenRowSnapshot {
                cells: vec![cell("字"), wide_continuation_cell(), cell("x")],
                wrapped: false,
            }],
            ..TerminalSnapshot::default()
        };

        let plan = RenderPlan::from_terminal(
            RenderRect::new(0, 0, 3, 1),
            RenderBackend::Software,
            &snapshot,
        );

        let glyphs = &plan.rows[0].glyphs;
        assert_eq!(glyphs.len(), 3);
        assert_eq!(glyphs[0].text, "字");
        assert_eq!(glyphs[0].span, 2);
        assert!(!glyphs[0].wide_continuation);
        assert_eq!(glyphs[1].text, "");
        assert_eq!(glyphs[1].span, 0);
        assert!(glyphs[1].wide_continuation);
        assert_eq!(glyphs[2].text, "x");
        assert_eq!(glyphs[2].span, 1);
    }

    #[test]
    fn glyph_path_preserves_combining_marks() {
        let snapshot = TerminalSnapshot {
            rows: vec![ScreenRowSnapshot {
                cells: vec![cell("e\u{301}"), cell(" ")],
                wrapped: false,
            }],
            ..TerminalSnapshot::default()
        };

        let plan = RenderPlan::from_terminal(
            RenderRect::new(0, 0, 2, 1),
            RenderBackend::Software,
            &snapshot,
        );

        assert_eq!(plan.rows[0].glyphs[0].text, "e\u{301}");
        assert_eq!(plan.rows[0].glyphs[0].span, 1);
    }

    #[test]
    fn from_input_preserves_damage_metadata() {
        let snapshot = TerminalSnapshot {
            rows: vec![ScreenRowSnapshot {
                cells: vec![cell("a"), cell("b"), cell("c")],
                wrapped: false,
            }],
            ..TerminalSnapshot::default()
        };
        let damage = DamageSet {
            dirty_rows: vec![0],
            full_frame: false,
        };

        let plan = RenderPlan::from_input(RenderInput {
            viewport: RenderRect::new(4, 5, 6, 7),
            backend: RenderBackend::Software,
            snapshot: &snapshot,
            damage: &damage,
        });

        assert_eq!(plan.viewport, RenderRect::new(4, 5, 6, 7));
        assert_eq!(plan.damage, damage);
        assert_eq!(plan.rows[0].glyphs.len(), 3);
    }
}
