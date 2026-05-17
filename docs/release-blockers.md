# Noctrail Release Blockers

Status snapshot date: `2026-05-18`

This file is the current beta gate ledger for Noctrail. Beta is blocked
while any item below remains `OPEN`.

Current acceptance scope for this branch is `macOS only`. Linux and
Windows validation will be run separately on native machines and the
results will be uploaded outside this repository workflow.

This means `READY` below is only a statement about the active
macOS-local beta gate. It is not evidence that the broader
cross-platform roadmap requirement has already been satisfied.

## Summary

- `OPEN`: 0
- `CLOSED`: 12
- Beta status: `READY (macOS-only scope)`

## Blockers

| ID | Priority | Area | Status | Evidence / Gap |
|---|---|---|---|---|
| BETA-001 | P0 | Installer coverage | CLOSED | `cargo run -p noctrail-cli -- installer-smoke` passed on macOS. Linux and Windows installer validation is explicitly deferred to external native runs for this branch scope. |
| BETA-002 | P0 | CI matrix | CLOSED | The macOS-local suite matching the current CI steps passed: `cargo fmt --all -- --check`; `cargo clippy --workspace --all-targets --all-features -- -D warnings`; `cargo test --workspace --all-targets --all-features`; `cargo run -p noctrail-cli -- doctor`; `cargo run -p noctrail-cli -- doctor shell`; `cargo run -p noctrail-cli -- render-smoke`; `cargo run -p noctrail-cli -- pty-smoke`; `cargo run -p noctrail-cli -- replay "tests/fixtures/terminal/*.ntrec"`; `cargo run -p noctrail-app -- smoke`. Linux and Windows runs are deferred to external native validation for this branch scope. |
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

Within the current macOS-only scope, Noctrail must not be labeled beta
until every blocker above is `CLOSED`.

## External Validation Pending

The following evidence is still expected before anyone claims the full
cross-platform Phase 10 requirement is complete:

| Platform | Installer smoke | Build/test matrix | Evidence source |
|---|---|---|---|
| macOS | local pass recorded | local pass recorded | this repository state |
| Linux | pending external native run | pending external native run | upload not yet attached |
| Windows | pending external native run | pending external native run | upload not yet attached |
