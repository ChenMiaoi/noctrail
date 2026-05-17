# Noctrail Privacy and Security

Status snapshot date: `2026-05-17`

This document describes the current privacy and security behavior of the
repository as it exists today. It does not promise future storage,
telemetry, or cloud features that are not yet implemented.

## Defaults

- No telemetry is implemented in the current workspace.
- Agent features are default-off.
- With Agent off, Noctrail does not issue provider requests.
- `read-env` and `read-history` default to `false`.
- There is no `noctrail-storage` crate in the current workspace, so
  block history and audit history remain in-memory for the lifetime of
  the app process.

## Network Surfaces

Noctrail has three agent provider modes in code:

- `openai-compatible`
- `local`
- `cli`

These adapters are only constructed when all of the following are true:

1. `agent.enabled = true`
2. `agent.provider` is configured
3. A caller explicitly asks for provider work

If Agent remains disabled, `ProviderAdapter::from_agent_config()` returns
`None`, and the app-side policy reports `provider_request = none`.

Validation:

- `cargo run -p noctrail-app -- agent-default-smoke`
- `cargo run -p noctrail-app -- agent-provider-smoke`

## Data Access Boundaries

The current read-only context preview is intentionally narrow. Agent
context is limited to:

- current command block
- current selection
- cwd
- explicit files chosen by the app

No full shell history import, environment dump, SSH key scan, or
clipboard scrape is performed by default.

Validation:

- `cargo run -p noctrail-app -- agent-context-smoke`

## Redaction

Noctrail redacts secrets before they are written into crash diagnostics
or surfaced through agent context preview paths.

The current redaction corpus covers:

- marker-style secrets such as `token=`, `password=`, `secret=`
- bearer tokens
- OpenAI-style `sk-...` tokens
- GitHub-style `ghp_...` tokens
- JWT-like tokens
- AWS access keys
- Google API keys
- Azure storage account keys
- SSH public keys
- private key blocks

Validation:

- `cargo run -p noctrail-app -- redaction-smoke`
- `cargo run -p noctrail-app -- crash-smoke`

## Review and Execution Model

Agent suggestions do not directly execute shell commands.

Current behavior:

- command proposals are suggestions only
- patch previews are read-only diffs
- reviewed command execution requires explicit confirmation
- high-risk commands require stronger confirmation before PTY write
- audit entries are recorded for context, read, suggest, review, and
  execute actions

Validation:

- `cargo run -p noctrail-app -- agent-proposal-smoke`
- `cargo run -p noctrail-app -- agent-patch-preview-smoke`
- `cargo run -p noctrail-app -- agent-review-smoke`
- `cargo run -p noctrail-app -- agent-audit-smoke`

## Local Artifacts

Current local artifacts are limited to:

- user-specified config files
- temporary crash diagnostics written by the panic hook
- installer/package outputs under `target/packager`

Current non-persistent state:

- block history
- audit ledger
- context preview state
- review panel state

These remain in process memory and are discarded when the app exits.

## Operational Notes

- Provider failures must not break the foreground terminal path.
- Safe mode ignores broken config and forces the software renderer.
- `doctor` surfaces shell, PTY, GPU, font, config, and local permission
  diagnostics, but it does not upload those diagnostics anywhere.

Validation:

- `cargo run -p noctrail-app -- smoke --safe-mode --config <bad.toml>`
- `cargo run -p noctrail-cli -- doctor`
