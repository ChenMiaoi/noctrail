# Security Policy

Noctrail handles terminal output, shell context, local files, and future agent actions. Treat security and privacy issues as release blockers.

## Reporting

Until a dedicated security contact is published, open a minimal public issue requesting a private security contact. Do not include exploit details, secrets, logs, tokens, or private paths in the public issue.

## Scope

Security-sensitive areas include:

- Command execution without explicit user review.
- Secret redaction failures.
- Unauthorized file, environment, shell history, or key access.
- Agent/tool permission bypass.
- Audit log leaks.
- Cross-platform process or PTY lifecycle bugs that expose user data.

## Defaults

- Do not upload shell history, environment variables, SSH keys, tokens, or browser data by default.
- Redact secrets before logging, auditing, or sending context to an agent provider.
- Require review before shell execution or filesystem writes.
