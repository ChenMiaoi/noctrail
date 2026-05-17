# ADR 0003: Terminal Core First

- Status: Accepted
- Date: 2026-05-17
- Deciders: Noctrail contributors
- Supersedes: none

## Context

The roadmap explicitly says Noctrail must first become a correct,
cross-platform terminal before it grows into a multi-pane workspace with
visual polish, shell integration, command blocks, or agent workflows.

This needs a dedicated ADR because earlier iterations failed less from a
bad product direction than from mixing too many layers at once. Terminal
state, renderer work, runtime orchestration, storage, policy, and agent
behavior all advanced in parallel and then constrained one another.

The core engineering question is not whether Noctrail will eventually
have richer product features. It is whether terminal correctness remains
the gating dependency for everything above it.

## Decision

Noctrail adopts a terminal-core-first execution order:

1. `noctrail-term` becomes a correct, replayable terminal state machine
   before higher product layers are allowed to depend on unstable
   terminal semantics.
2. Terminal correctness is judged by fixtures, not by whether the GUI
   looks complete.
3. Shell integration, command blocks, policy, storage, and agent
   workflows remain deferred until terminal MVP and pane runtime
   behavior are stable.

`noctrail-term` is allowed to own:

- bytes to terminal state transitions
- grid, cursor, style, scrollback, selection, resize, and damage
- immutable snapshots and replay fixtures

`noctrail-term` is not allowed to own:

- pane or workspace state
- PTY lifecycle
- renderer resources
- shell hook metadata such as cwd, git branch, or command blocks
- agent, policy, storage, or audit state

The acceptance rule is strict: later layers may consume terminal
snapshots and damage metadata, but they may not push product semantics
back into terminal state just to make adjacent features easier.

## Consequences

Positive consequences:

- Terminal regressions can be isolated and tested without dragging GUI
  and process lifecycle concerns into every change.
- The replay fixture harness becomes the canonical proof that terminal
  behavior improved or stayed stable.
- Deferred product layers retain freedom to change because they do not
  leak into the terminal core API prematurely.

Costs and tradeoffs:

- Some GUI-visible progress will appear slower because correctness work
  comes before richer product semantics.
- Product ideas that depend on shell meaning or structured command
  boundaries must wait for explicit higher-layer designs.
- It becomes harder to justify shortcuts that stash product state inside
  terminal rows or cells.

## Follow-up

- Keep expanding terminal replay coverage before widening public APIs.
- Reject changes that add block, shell-integration, or agent context to
  `noctrail-term`.
- Add specs for terminal sequences and snapshot semantics once the core
  API settles further.
