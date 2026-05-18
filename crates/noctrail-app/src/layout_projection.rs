use noctrail_layout::LayoutRect;
use noctrail_pty::PtySize;
use noctrail_render::{ChromeLayer, ChromeRect, RenderRect, Rgba};
use noctrail_term::{Cursor, DamageSet, Position, Selection, TerminalSnapshot};

use crate::PaneChromeConfig;

pub(crate) fn full_frame_damage(size: PtySize) -> DamageSet {
    DamageSet {
        dirty_rows: (0..usize::from(size.rows)).collect(),
        full_frame: true,
    }
}

pub(crate) fn collect_all_rows(
    snapshot: &TerminalSnapshot,
) -> Vec<noctrail_term::ScreenRowSnapshot> {
    let mut rows = snapshot.scrollback.clone();
    rows.extend(snapshot.rows.clone());
    rows
}

pub(crate) fn max_scrollback_offset(snapshot: &TerminalSnapshot) -> usize {
    snapshot.scrollback.len()
}

pub(crate) fn visible_row_range(
    snapshot: &TerminalSnapshot,
    visible_height: usize,
    scrollback_offset: usize,
) -> std::ops::Range<usize> {
    let total_rows = snapshot.scrollback.len() + snapshot.rows.len();
    let end = total_rows.saturating_sub(scrollback_offset.min(max_scrollback_offset(snapshot)));
    let start = end.saturating_sub(visible_height.max(1));
    start..end
}

pub(crate) fn viewport_to_terminal_position(
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

pub(crate) fn remap_cursor(
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

pub(crate) fn remap_selection(
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

pub(crate) fn pane_terminal_size(
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

pub(crate) fn scratch_surface(surface: LayoutRect, scratch_height_percent: u8) -> LayoutRect {
    let horizontal_inset = ((u32::from(surface.width) * 6) / 100).max(12) as u16;
    let max_width = surface
        .width
        .saturating_sub(horizontal_inset.saturating_mul(2));
    let width = max_width.max(surface.width.saturating_sub(24)).max(1);
    let top_inset = ((u32::from(surface.height) * 4) / 100).max(12) as u16;
    let max_height = surface.height.saturating_sub(top_inset.saturating_add(12));
    let height = ((u32::from(surface.height) * u32::from(scratch_height_percent.max(1))) / 100)
        .max(1)
        .min(u32::from(max_height.max(1))) as u16;
    let x = surface
        .x
        .saturating_add((surface.width.saturating_sub(width)) / 2);
    let y = surface
        .y
        .saturating_add(top_inset.min(surface.height.saturating_sub(height)));

    LayoutRect::new(x, y, width, height)
}

pub(crate) fn scratch_terminal_size(
    surface: LayoutRect,
    terminal_size: PtySize,
    pane_chrome: PaneChromeConfig,
    scratch_height_percent: u8,
) -> PtySize {
    pane_terminal_size(
        surface,
        terminal_size,
        pane_content_surface(
            scratch_surface(surface, scratch_height_percent),
            pane_chrome,
        ),
    )
}

pub(crate) fn pane_content_surface(
    pane_surface: LayoutRect,
    pane_chrome: PaneChromeConfig,
) -> LayoutRect {
    let outer = pane_outer_insets(pane_surface, pane_chrome);
    let inner = inset_layout_rect(pane_surface, outer);
    let status_height = effective_status_height(inner.height, pane_chrome);

    inset_layout_rect(
        inner,
        EdgeInsets {
            left: 0,
            right: 0,
            top: status_height
                .saturating_add(status_spacing_for_height(status_height, pane_chrome)),
            bottom: 0,
        },
    )
}

pub(crate) fn pane_chrome_rects(
    pane_surface: LayoutRect,
    status_surface: LayoutRect,
    active: bool,
    pane_chrome: PaneChromeConfig,
) -> Vec<ChromeRect> {
    let mut rects = Vec::with_capacity(4);
    let pane_background = if active {
        pane_chrome.background
    } else {
        mix_rgba(pane_chrome.status_background, pane_chrome.background, 0.9)
    };
    let pane_rect = layout_rect_to_render_rect(pane_surface, pane_surface);
    if pane_rect.width > 0 && pane_rect.height > 0 {
        rects.push(ChromeRect {
            layer: ChromeLayer::PaneBackground,
            rect: pane_rect,
            color: pane_background,
        });
    }

    let header_rect = layout_rect_to_render_rect(status_surface, pane_surface);
    if header_rect.width > 0 && header_rect.height > 0 {
        let status_background = if active {
            pane_chrome.status_background
        } else {
            mix_rgba(pane_chrome.status_background, pane_background, 0.78)
        };
        rects.push(ChromeRect {
            layer: ChromeLayer::StatusBackground,
            rect: header_rect,
            color: status_background,
        });

        let indicator_height = if active { 3_usize } else { 2_usize }
            .min(header_rect.height)
            .max(1);
        rects.push(ChromeRect {
            layer: ChromeLayer::StatusIndicator,
            rect: RenderRect::new(
                header_rect.x,
                header_rect.y,
                header_rect.width,
                indicator_height,
            ),
            color: if active {
                pane_chrome.active_indicator
            } else {
                pane_chrome.inactive_indicator
            },
        });
    }

    if status_surface.height == 0 && pane_rect.width > 0 && pane_rect.height > 0 {
        let indicator_height = if active { 3_usize } else { 2_usize }
            .min(pane_rect.height)
            .max(1);
        rects.push(ChromeRect {
            layer: ChromeLayer::StatusIndicator,
            rect: RenderRect::new(0, 0, pane_rect.width, indicator_height),
            color: if active {
                pane_chrome.active_indicator
            } else {
                pane_chrome.inactive_indicator
            },
        });
    }

    let status_rect = layout_rect_to_render_rect(status_surface, pane_surface);
    if status_rect.width > 0 && status_rect.height > 0 {
        rects.push(ChromeRect {
            layer: ChromeLayer::StatusSeparator,
            rect: RenderRect::new(
                status_rect.x,
                status_rect.y + status_rect.height.saturating_sub(1),
                status_rect.width,
                1,
            ),
            color: if active {
                pane_chrome.status_separator
            } else {
                mix_rgba(pane_chrome.status_separator, pane_background, 0.86)
            },
        });
    }

    rects
}

pub(crate) fn pane_status_surface(
    pane_surface: LayoutRect,
    pane_chrome: PaneChromeConfig,
) -> LayoutRect {
    let outer = pane_outer_insets(pane_surface, pane_chrome);
    let inner = inset_layout_rect(pane_surface, outer);
    let status_height = effective_status_height(inner.height, pane_chrome);
    if status_height == 0 {
        return LayoutRect::new(inner.x, inner.y, inner.width, 0);
    }

    LayoutRect::new(inner.x, inner.y, inner.width, status_height)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct EdgeInsets {
    left: u16,
    right: u16,
    top: u16,
    bottom: u16,
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

fn pane_outer_insets(pane_surface: LayoutRect, pane_chrome: PaneChromeConfig) -> EdgeInsets {
    let left_gap = pane_chrome.gap / 2;
    let right_gap = pane_chrome.gap - left_gap;
    let top_gap = pane_chrome.gap / 2;
    let bottom_gap = pane_chrome.gap - top_gap;
    let horizontal_cap = (pane_surface.width / 10).max(4);
    let vertical_cap = (pane_surface.height / 10).max(4);
    EdgeInsets {
        left: left_gap
            .saturating_add(pane_chrome.padding)
            .min(horizontal_cap),
        right: right_gap
            .saturating_add(pane_chrome.padding)
            .min(horizontal_cap),
        top: top_gap
            .saturating_add(pane_chrome.padding)
            .min(vertical_cap),
        bottom: bottom_gap
            .saturating_add(pane_chrome.padding)
            .min(vertical_cap),
    }
}

fn effective_status_height(inner_height: u16, pane_chrome: PaneChromeConfig) -> u16 {
    if pane_chrome.status_height == 0 || inner_height <= 2 || inner_height < 44 {
        return 0;
    }

    let spacing = status_spacing_for_height(pane_chrome.status_height, pane_chrome);
    let max_height = inner_height
        .saturating_sub(spacing.saturating_add(1))
        .min((inner_height / 3).max(1));
    pane_chrome.status_height.min(max_height)
}

fn status_spacing_for_height(status_height: u16, pane_chrome: PaneChromeConfig) -> u16 {
    if status_height == 0 {
        0
    } else {
        pane_chrome.status_spacing.min((status_height / 2).max(4))
    }
}

fn layout_rect_to_render_rect(rect: LayoutRect, origin: LayoutRect) -> RenderRect {
    RenderRect::new(
        usize::from(rect.x.saturating_sub(origin.x)),
        usize::from(rect.y.saturating_sub(origin.y)),
        usize::from(rect.width),
        usize::from(rect.height),
    )
}

fn mix_rgba(foreground: Rgba, background: Rgba, background_ratio: f32) -> Rgba {
    let foreground_ratio = (1.0 - background_ratio).clamp(0.0, 1.0);
    let background_ratio = background_ratio.clamp(0.0, 1.0);
    Rgba {
        red: ((f32::from(foreground.red) * foreground_ratio)
            + (f32::from(background.red) * background_ratio))
            .round() as u8,
        green: ((f32::from(foreground.green) * foreground_ratio)
            + (f32::from(background.green) * background_ratio))
            .round() as u8,
        blue: ((f32::from(foreground.blue) * foreground_ratio)
            + (f32::from(background.blue) * background_ratio))
            .round() as u8,
        alpha: u8::MAX,
    }
}

fn inset_layout_rect(rect: LayoutRect, insets: EdgeInsets) -> LayoutRect {
    let total_horizontal = insets.left.saturating_add(insets.right).min(rect.width);
    let total_vertical = insets.top.saturating_add(insets.bottom).min(rect.height);

    let width = rect.width.saturating_sub(total_horizontal).max(1);
    let height = rect.height.saturating_sub(total_vertical).max(1);
    let left = insets.left.min(rect.width.saturating_sub(width));
    let top = insets.top.min(rect.height.saturating_sub(height));

    LayoutRect::new(
        rect.x.saturating_add(left),
        rect.y.saturating_add(top),
        width,
        height,
    )
}
