# Terminal Sequences

This document records the current tested VT/OSC subset rather than a
full terminal emulation contract.

## Scope

The parser boundary comes from `vte`, but terminal meaning is owned by
`noctrail-term`.

This spec only lists sequence families that the current repository
explicitly tests or routes into visible state.

## Current Tested Families

- printable ASCII and Unicode cell writes
- combining-mark append behavior
- wide-character continuation behavior
- cursor movement that dirties both old and new rows
- resize with visible-prefix preservation
- alternate-screen enter and exit
- bracketed paste private mode tracking
- DEC mouse tracking mode tracking
- SGR style updates
- Noctrail shell-integration OSC markers

## Style Surface

Current visible style fields are:

- foreground color
- background color
- bold
- italic
- underline

Supported color storage:

- default color
- 8-bit indexed color
- 24-bit RGB color

## Mode Bits

Current snapshot-visible mode bits:

- `alternate_screen`
- `bracketed_paste`

Current terminal-owned mouse state:

- tracking mode
  - disabled
  - press
  - drag
  - motion
- SGR mouse reporting flag

## OSC Handling

The terminal currently recognizes a Noctrail-specific OSC subset under:

- `OSC 1337;Noctrail;... BEL`
- `OSC 1337;Noctrail;... ST`

Recognized payloads are documented in
[shell-integration-osc.md](shell-integration-osc.md).

Unknown or malformed Noctrail markers are ignored rather than rendered.

## Validation

- `cargo test -p noctrail-term --all-targets`
- `cargo run -p noctrail-cli -- replay "tests/fixtures/terminal/*.ntrec"`
