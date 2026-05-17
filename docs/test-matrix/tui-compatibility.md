# TUI Compatibility

This document records the current shell, prompt, Unicode, and TUI
compatibility matrix entrypoints.

## Entry Points

- `cargo run -p noctrail-cli -- shell-matrix`
- `cargo run -p noctrail-cli -- prompt-matrix`
- `cargo run -p noctrail-cli -- unicode-matrix`
- `cargo run -p noctrail-cli -- tui-matrix`

## Shell Matrix

Current shell targets:

- `bash`
- `zsh`
- `fish`
- `pwsh`
- `nu`
- `cmd`
- `WSL`

The shell matrix checks:

- startup
- input echo or command handling
- exit
- cwd reporting

Targets without a native probe may be supplied through
`NOCTRAIL_SHELL_*` override scripts.

## Prompt Matrix

Current prompt targets:

- `starship`
- `oh-my-zsh`
- `powerlevel10k`

The prompt matrix checks:

- prompt layout
- clean ANSI escape handling
- Noctrail hook marker emission

Current prompt emulation is available on non-Windows platforms.

## Unicode Matrix

Current Unicode targets:

- `cjk`
- `emoji`
- `combining`
- `fullwidth`

The Unicode matrix checks:

- input placement
- selection behavior
- copy behavior
- cursor accounting

## TUI Matrix

Current TUI targets:

- `nvim`
- `tmux`
- `fzf`
- `less`
- `top/htop`
- `ssh`

The TUI matrix checks a target-specific subset of:

- alt-screen transitions
- mouse mode signaling
- resize handling
- color output

Targets without a native probe may be supplied through
`NOCTRAIL_TUI_*` override scripts.

## Native vs Override Probes

Current built-in native probes are intentionally small:

- `less` has a native scripted probe
- several shell and prompt paths have native probes on Unix-like
  platforms
- the remaining matrix entries are designed to accept explicit override
  scripts so native environments can validate them without changing the
  core CLI code
