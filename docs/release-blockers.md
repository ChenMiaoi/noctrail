# Noctrail Release Blockers

Status snapshot date: `2026-05-17`

This file is the current beta gate ledger for Noctrail. Beta is blocked
while any item below remains `OPEN`.

## Summary

- `OPEN`: 2
- `CLOSED`: 10
- Beta status: `BLOCKED`

## Blockers

| ID | Priority | Area | Status | Evidence / Gap |
|---|---|---|---|---|
| BETA-001 | P0 | Installer coverage | OPEN | macOS installer smoke has been re-verified locally, and `.github/workflows/installer-smoke.yml` now defines native Windows/macOS/Linux packaging smoke. The blocker remains open until Linux and Windows native runs complete green on GitHub Actions. |
| BETA-002 | P0 | CI matrix | OPEN | `.github/workflows/ci.yml` and `.github/workflows/installer-smoke.yml` now define the intended three-platform build/test/package matrix, but this branch does not yet have a fresh all-green remote run recorded after the latest Phase 10 changes. |
| BETA-003 | P0 | Single-pane shell stability | CLOSED | `cargo run -p noctrail-app -- smoke`; `cargo run -p noctrail-cli -- pty-smoke` |
| BETA-004 | P0 | High-output responsiveness | CLOSED | `cargo run -p noctrail-app -- perf-smoke` reported `high_output_p95_ms=2.860` and `idle_premature_redraw=false` during the last recorded run. |
| BETA-005 | P0 | Resize correctness | CLOSED | `cargo run -p noctrail-cli -- pty-smoke`; `cargo run -p noctrail-cli -- tui-matrix` |
| BETA-006 | P0 | Multi-pane isolation | CLOSED | `cargo run -p noctrail-cli -- pty-smoke`; `cargo test -p noctrail-app --all-targets` |
| BETA-007 | P0 | Workspace lifecycle | CLOSED | `cargo test -p noctrail-app --all-targets` covers workspace switching without killing processes. |
| BETA-008 | P0 | Pane close cleanup | CLOSED | `cargo test -p noctrail-runtime --all-targets`; `cargo test -p noctrail-pty --all-targets` |
| BETA-009 | P0 | Safe-mode startup | CLOSED | `cargo run -p noctrail-app -- smoke --safe-mode --config <bad.toml>`; config tests in `noctrail-app` and `noctrail-config` |
| BETA-010 | P0 | Agent review boundary | CLOSED | `cargo run -p noctrail-app -- agent-review-smoke`; `cargo run -p noctrail-app -- agent-patch-preview-smoke` |
| BETA-011 | P1 | Diagnostics surface | CLOSED | `cargo run -p noctrail-cli -- doctor`; `cargo run -p noctrail-cli -- doctor pty`; `cargo run -p noctrail-cli -- doctor config examples/noctrail.example.toml`; `cargo run -p noctrail-cli -- doctor permissions` |
| BETA-012 | P1 | Privacy/security docs | CLOSED | [docs/privacy-security.md](privacy-security.md); `cargo run -p noctrail-app -- agent-default-smoke`; `cargo run -p noctrail-app -- redaction-smoke`; `cargo run -p noctrail-app -- agent-review-smoke` |

## Exit Rule

Noctrail must not be labeled beta until every blocker above is `CLOSED`.
