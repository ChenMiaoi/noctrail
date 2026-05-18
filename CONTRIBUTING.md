# Contributing

Noctrail is early-stage. Keep changes small, reviewable, and aligned with [docs/roadmap.md](docs/roadmap.md).

## Principles

- Terminal correctness comes before visual features.
- Agent actions must be explicit, reviewable, and permission-gated.
- Cross-platform behavior matters from the start.
- Code is maintenance cost: avoid redundant state, broad abstractions, and speculative framework code.

## Workflow

1. Open or reference an issue for non-trivial work.
2. Keep each pull request focused on one logical change.
3. Add or update tests for behavior changes.
4. Run the validation commands before requesting review.

## Module Boundaries

- Keep platform-specific logic behind `platform/...` modules instead of
  scattering `cfg` branches through large product files.
- Extract platform decision points, not duplicated end-to-end workflows.
- Avoid speculative abstractions; prefer narrow helpers with explicit
  inputs and outputs.
- When a top-level entry file grows past a few hundred lines, split by
  responsibility before adding more behavior.

## Validation

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
```

If a command cannot run in your environment, explain why in the pull request.

## Commits

Use focused Conventional Commits:

```text
feat(cli): add doctor command
fix(policy): redact tokens in audit logs
docs(plan): clarify terminal MVP scope
```

Do not mix unrelated refactors, formatting churn, and behavior changes in one commit.
