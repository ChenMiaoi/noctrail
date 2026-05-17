# ADR 0005: Agent Deferred Review Boundary

- Status: Accepted
- Date: 2026-05-18
- Deciders: Noctrail contributors
- Supersedes: none

## Context

The roadmap places Agent, policy, and review work after terminal core,
runtime, renderer, app, tiling, shell integration, and command blocks.
That ordering is not only about product priority. It is also a safety
boundary.

The current repository already contains:

- default-off agent config
- provider adapters
- read-only context preview
- redaction
- command proposals
- patch previews
- review and audit UI

What still needed a single reference was the rule that these features
must not bypass foreground terminal behavior or silently execute shell
commands.

## Decision

Noctrail keeps Agent behind a deferred, explicit review boundary.

The boundary has five rules:

1. Agent is default-off.
2. Agent context is read-only and intentionally narrow.
3. Provider failures must not break the foreground terminal path.
4. Agent suggestions are not execution.
5. Any PTY write derived from Agent output requires explicit review.

Current behavior is therefore fixed as follows.

### Access and context

- `agent.enabled = false` remains the default.
- `read-env` and `read-history` remain default-off.
- Read-only context preview is limited to current block, current
  selection, cwd, and explicit files chosen by the app.

### Suggestion surfaces

- Command proposals are typed suggestions with reason, risk, and
  permission metadata.
- Patch previews are read-only unified diffs.
- Neither surface modifies files or writes to PTY by itself.

### Review boundary

- Low- and medium-risk command proposals require explicit confirmation
  before PTY write.
- High- and critical-risk proposals require stronger confirmation.
- Reviewed execution is observable through the audit ledger.

### Privacy and failure handling

- Redaction runs before agent context or crash diagnostics are surfaced.
- Provider adapter errors stay on the agent side and do not corrupt the
  pane runtime, terminal grid, or render path.

## Consequences

Positive consequences:

- Agent work can evolve without weakening terminal correctness.
- Default-off startup matches the privacy and release-blocker docs.
- Reviewable command and patch surfaces remain auditable instead of
  hidden in provider callbacks.

Costs and tradeoffs:

- Agent features feel less automatic because every execution boundary is
  explicit.
- Some provider output must be normalized into typed proposals instead
  of free-form convenience text.
- Future storage or background agent workflows must preserve this review
  boundary instead of inventing side channels.

## Follow-up

- Keep agent execution tied to reviewable UI state, not provider return
  values.
- If a dedicated `noctrail-policy` crate is introduced later, migrate
  risk classification and execution gating there without weakening these
  rules.
- Keep smoke tests for default-off behavior, provider isolation,
  redaction, proposal review, patch preview, and audit logging.
