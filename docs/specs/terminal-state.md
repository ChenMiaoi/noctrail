# Terminal State

This document records the current `noctrail-term` state boundary.

## Ownership

`TerminalState` owns:

- VT byte ingestion
- primary and alternate grids
- cursor position
- cell style
- scrollback
- selection
- bracketed paste mode
- mouse tracking mode
- shell integration event buffering
- damage reporting

It does not own:

- PTY lifecycle
- pane or workspace layout
- renderer resources
- agent, policy, or audit state

## Current Data Model

Core serializable types:

- `Color`
  - `Default`
  - `Indexed(u8)`
  - `Rgb(u8, u8, u8)`
- `Style`
  - foreground
  - background
  - bold
  - italic
  - underline
- `Cell`
  - `text`
  - `style`
  - `wide_continuation`
- `Cursor`
  - `row`
  - `col`
- `Selection`
  - mode
  - start
  - end
- `TerminalSnapshot`
  - visible rows
  - scrollback rows
  - cursor
  - alternate-screen bit
  - bracketed-paste bit
  - optional selection

## Advance Result

`advance_bytes()` reports:

- `damage`
- `cursor_moved`
- `scrolled`
- `alternate_screen_changed`
- `title_changed`

`DamageSet` currently carries:

- `dirty_rows: Vec<usize>`
- `full_frame: bool`

## Current Guarantees

- Grid width and height clamp to at least `1`.
- Dirty rows are deduplicated before they leave the terminal boundary.
- Wide-cell continuation is explicit in `Cell`.
- Selection remains terminal-owned and is copied into snapshots.
- Shell integration events are buffered separately from visible cells.

## Known Limits

Current source-level limits remain:

- ZWJ emoji sequences are not yet collapsed into a single cell cluster.
- RTL shaping is not modeled beyond scalar-width cursor accounting.
- Current coverage is strongest for printable text, combining marks, and
  wide-cell continuation behavior.

## Validation

- `cargo test -p noctrail-term --all-targets`
- `cargo run -p noctrail-cli -- replay "tests/fixtures/terminal/*.ntrec"`
