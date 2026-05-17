# Noctrail Release Blockers

Status snapshot date: `2026-05-17`

This file is the current beta gate ledger for Noctrail. Beta is blocked
while any item below remains `OPEN`.

## Summary

- `OPEN`: 3
- `CLOSED`: 9
- Beta status: `BLOCKED`

## Blockers

| ID | Priority | Area | Status | Evidence / Gap |
|---|---|---|---|---|
| BETA-001 | P0 | Installer coverage | OPEN | macOS installer smoke was verified locally with `./scripts/package-installer.sh`, but Linux and Windows installer lifecycle smoke have not yet been re-run on native hosts after the current packaging changes. |
| BETA-002 | P0 | CI matrix | OPEN | `.github/workflows/ci.yml` runs the intended Windows/macOS/Linux matrix, but this branch does not yet have a fresh all-green three-platform run recorded after the latest config and diagnostics work. |
| BETA-003 | P0 | Single-pane shell stability | CLOSED | `cargo run -p noctrail-app -- smoke`; `cargo run -p noctrail-cli -- pty-smoke` |
| BETA-004 | P0 | High-output responsiveness | CLOSED | `cargo run -p noctrail-app -- perf-smoke` reported `high_output_p95_ms=2.860` and `idle_premature_redraw=false` during the last recorded run. |
| BETA-005 | P0 | Resize correctness | CLOSED | `cargo run -p noctrail-cli -- pty-smoke`; `cargo run -p noctrail-cli -- tui-matrix` |
| BETA-006 | P0 | Multi-pane isolation | CLOSED | `cargo run -p noctrail-cli -- pty-smoke`; `cargo test -p noctrail-app --all-targets` |
| BETA-007 | P0 | Workspace lifecycle | CLOSED | `cargo test -p noctrail-app --all-targets` covers workspace switching without killing processes. |
| BETA-008 | P0 | Pane close cleanup | CLOSED | `cargo test -p noctrail-runtime --all-targets`; `cargo test -p noctrail-pty --all-targets` |
| BETA-009 | P0 | Safe-mode startup | CLOSED | `cargo run -p noctrail-app -- smoke --safe-mode --config <bad.toml>`; config tests in `noctrail-app` and `noctrail-config` |
| BETA-010 | P0 | Agent review boundary | CLOSED | `cargo run -p noctrail-app -- agent-review-smoke`; `cargo run -p noctrail-app -- agent-patch-preview-smoke` |
| BETA-011 | P1 | Diagnostics surface | CLOSED | `cargo run -p noctrail-cli -- doctor`; `cargo run -p noctrail-cli -- doctor pty`; `cargo run -p noctrail-cli -- doctor config examples/noctrail.example.toml`; `cargo run -p noctrail-cli -- doctor permissions` |
| BETA-012 | P1 | Privacy/security docs | OPEN | Roadmap item `10.6` is still incomplete. Agent/storage/telemetry/logging defaults exist in code, but the user-facing privacy/security document set has not been published yet. |

## Exit Rule

Noctrail must not be labeled beta until every blocker above is `CLOSED`.
