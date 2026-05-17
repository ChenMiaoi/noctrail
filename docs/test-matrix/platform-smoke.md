# Platform Smoke

This document records the current smoke commands used to prove the
end-to-end app, CLI, and packaging paths.

## macOS Local Scope

The current branch-level beta gate is scoped to local macOS validation.
Linux and Windows native validation are expected to run separately on
real machines and be uploaded outside this repository workflow.

See [docs/release-blockers.md](../release-blockers.md) for the active
scope language.

## External Evidence Ledger

The repository now distinguishes between:

- branch-local macOS validation that is runnable here
- external native Linux and Windows validation that must be attached
  from outside this local workspace

Current evidence state:

| Platform | Core smoke commands | Current state |
|---|---|---|
| macOS | local commands in this document | verified locally |
| Linux | equivalent native smoke run | awaiting uploaded native evidence |
| Windows | equivalent native smoke run | awaiting uploaded native evidence |

## Core Smoke Commands

Current macOS-local platform smoke commands are:

- `cargo run -p noctrail-cli -- doctor`
- `cargo run -p noctrail-cli -- doctor shell`
- `cargo run -p noctrail-cli -- render-smoke`
- `cargo run -p noctrail-cli -- pty-smoke`
- `cargo run -p noctrail-cli -- replay "tests/fixtures/terminal/*.ntrec"`
- `cargo run -p noctrail-app -- smoke`
- `cargo run -p noctrail-cli -- installer-smoke`

These commands collectively exercise:

- shell resolution
- PTY spawn, read, write, resize, and close
- terminal replay fixtures
- render setup
- GUI startup, input, and shutdown
- packaged installer lifecycle on macOS

## Extended Local Probes

The repository also carries additional local probes that are useful when
auditing non-blocking behavior:

- `cargo run -p noctrail-app -- perf-smoke`
- `cargo run -p noctrail-app -- soak-smoke`
- `cargo run -p noctrail-app -- crash-smoke`
- `cargo run -p noctrail-app -- block-smoke`
- `cargo run -p noctrail-app -- structured-output-smoke`
- `cargo run -p noctrail-app -- failure-block-smoke`
- `cargo run -p noctrail-app -- agent-default-smoke`
- `cargo run -p noctrail-app -- agent-provider-smoke`
- `cargo run -p noctrail-app -- agent-context-smoke`
- `cargo run -p noctrail-app -- redaction-smoke`
- `cargo run -p noctrail-app -- agent-review-smoke`
- `cargo run -p noctrail-app -- agent-patch-preview-smoke`
- `cargo run -p noctrail-app -- agent-audit-smoke`

## CI Mapping

The current GitHub workflows still define broader multi-platform
matrices, but the branch's active release gate does not require remote
Linux or Windows success until those external native runs are supplied.

When those uploads arrive, they should be attached as the authoritative
evidence for the Linux and Windows rows above rather than inferred from
this macOS-local workspace.
