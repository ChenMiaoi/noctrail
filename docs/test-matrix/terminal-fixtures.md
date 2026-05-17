# Terminal Fixtures

This document records the current terminal replay matrix.

## Entry Points

- `cargo test -p noctrail-term --all-targets`
- `cargo run -p noctrail-cli -- replay "tests/fixtures/terminal/*.ntrec"`

The replay harness reads JSON `.ntrec` files through
`noctrail_term::recording`.

## Fixture Format

Each `.ntrec` file is a `RecordingSuite` with one or more
`RecordingCase` values.

A case currently supports:

- terminal width and height
- hex-encoded PTY input
- optional scrollback limit
- optional resize sequence
- optional selection
- expected terminal snapshot
- optional expected selection text
- optional expected shell integration events

## Current Coverage

`tests/fixtures/terminal/core.ntrec` currently carries `34` replay
cases. The covered behaviors are:

- ASCII writes and wrap behavior
- CJK wide cells
- combining marks
- carriage return, line feed, CRLF, backspace, and tab
- scrollback limits and scrollback clearing
- alternate screen enter, clear, and restore
- normal, line, and block selection
- erase line and erase display paths
- resize repair and multi-step resize
- cursor save and restore
- SGR style, 256-color, and true-color cases
- Noctrail OSC shell markers

Current case names:

- `ascii_write`
- `wide_char`
- `combining_mark`
- `noctrail_osc_markers`
- `cr_overwrite`
- `lf_preserves_column`
- `wrap_to_next_line`
- `scrollback_limit_two`
- `clear_scrollback_only`
- `alt_restore_primary`
- `alt_no_scrollback`
- `selection_normal`
- `selection_line_wrap`
- `selection_block`
- `erase_line_right`
- `erase_line_left`
- `erase_display_all`
- `resize_clamps_cursor`
- `resize_wide_repair`
- `resize_multiple_steps`
- `scrollback_limit_one`
- `scrollback_after_resize`
- `backspace_moves_left`
- `tab_advances_to_next_stop`
- `cursor_save_restore_primary`
- `cursor_save_restore_alt`
- `mixed_crlf`
- `line_selection_across_scrollback`
- `block_selection_with_wide`
- `alt_clear_then_restore`
- `sgr_ansi_style_reset`
- `sgr_256_color`
- `sgr_true_color`
- `wrap_variant`

## Intended Use

Add replay cases when terminal behavior changes in a way that should be
proven at the byte-to-snapshot boundary without involving PTY, GUI, or
GPU layers.
