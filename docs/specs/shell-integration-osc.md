# Shell Integration OSC

This document records the current Noctrail shell-integration marker
format.

## Transport

The terminal currently recognizes Noctrail markers in OSC 1337 with two
valid terminators:

- BEL (`\x07`)
- ST (`\x1b\\`)

Prefix:

```text
OSC 1337;Noctrail;...
```

## Current Payloads

Payloads currently recognized by `noctrail-term` are:

- `Prompt`
- `CommandStart`
- `CommandText;<text>`
- `CommandEnd`
- `Cwd;<cwd>`
- `ExitCode;<code>`
- `DurationMs;<milliseconds>`

Malformed integer payloads are ignored.

## Observer Model

These markers are not rendered into visible cells. They are drained from
`TerminalState` as `ShellIntegrationEvent` values and then consumed by
higher layers such as:

- prompt and shell compatibility probes
- command block observation
- status line metadata
- review and audit UI

## Producer Surface

The repository currently ships hook renderers or probe scripts for:

- bash
- zsh
- fish
- PowerShell
- Nushell

The compatibility matrices validate marker emission separately from
prompt text layout.

## Validation

- `cargo test -p noctrail-term --all-targets`
- `cargo run -p noctrail-cli -- prompt-matrix`
- `cargo run -p noctrail-cli -- shell-matrix`
