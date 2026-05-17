# ADR 0001: Crate Boundaries

- Status: Accepted
- Date: 2026-05-17
- Deciders: Noctrail contributors
- Supersedes: none

## Context

The reboot roadmap requires a small workspace with explicit boundaries.
The current repository already contains the eight core crates named in
`docs/roadmap.md`, but the boundary expectations only live in roadmap
tables and README summaries.

That is not enough for review. Contributors need one short document that
answers three questions before new code lands:

1. Which crate owns a concern.
2. Which crate must not absorb that concern.
3. Which crates are intentionally deferred until terminal MVP is stable.

Without that guardrail, the repository can regress back into the earlier
prototype shape where terminal state, runtime orchestration, rendering,
agent features, and storage concerns all drift toward one another.

## Decision

Noctrail keeps the reboot workspace at eight core crates:

| Crate | Owns | Must not own |
|---|---|---|
| `noctrail-cli` | developer entrypoints such as `doctor`, `replay`, and smoke commands | long-lived GUI state, renderer internals, agent workflows |
| `noctrail-config` | TOML config loading, validation, defaults, and safe reload boundaries | shell history, secrets, PTY state, renderer resources |
| `noctrail-term` | terminal state machine, VT semantics, grid, scrollback, selection, resize, damage, snapshots | panes, PTY lifecycle, GUI layout, agent or block state |
| `noctrail-pty` | shell resolution, PTY session lifecycle, read/write, resize, exit status | terminal parsing, render state, workspace state |
| `noctrail-runtime` | pane runtime registry, pane ids, bounded routing between PTY sessions and app owners | VT parsing, render plan construction, layout policy |
| `noctrail-layout` | workspace and pane tree logic, focus movement, tiling algorithms, rectangle arrangement | PTY ownership, terminal grids, renderer resources |
| `noctrail-render` | immutable render input, render plans, renderer backends, glyph and surface concerns | PTY IO, shell integration, mutable terminal ownership |
| `noctrail-app` | window lifecycle, input routing, frame scheduling, platform glue, top-level composition | hidden cross-crate business logic, parser details, storage or agent policy |

These boundaries are enforced by three rules:

1. Default to `pub(crate)` and only expose cross-crate APIs that are
   already needed by an active caller.
2. Move data across boundaries as immutable snapshots or explicit
   commands, not shared mutable state.
3. Keep platform-specific logic at the PTY, renderer, and app edges
   instead of scattering `cfg` branches through product logic.

The following crates remain explicitly out of the workspace until later
roadmap phases require them:

- `noctrail-shell-integration`
- `noctrail-blocks`
- `noctrail-policy`
- `noctrail-agent`
- `noctrail-storage`

They may be documented in roadmap or ADRs, but they are not allowed to
re-enter the workspace during Phase 0 or terminal-core MVP work.

## Consequences

Positive consequences:

- Reviews can reject cross-layer leakage with a single reference.
- Terminal correctness work can proceed without carrying agent or storage
  debt.
- The current eight-crate skeleton now has a documented reason to stay
  small.

Costs and tradeoffs:

- Some seemingly convenient helper code will need to stay duplicated
  until a stable boundary emerges.
- Cross-crate APIs should grow more slowly because public surface now
  needs stronger justification.
- Future features such as blocks or agent review may need extra
  integration work because they cannot short-circuit into terminal state.

## Follow-up

- Add a runtime event model ADR before `noctrail-runtime` grows beyond
  direct session wrappers.
- Add renderer stack ADRs before landing long-lived GPU backend code.
- Revisit this ADR only when a new crate removes real complexity rather
  than moving code for cosmetic reasons.
