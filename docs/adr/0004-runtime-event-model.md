# ADR 0004: Runtime Event Model

- Status: Accepted
- Date: 2026-05-17
- Deciders: Noctrail contributors
- Supersedes: none

## Context

The roadmap places PTY and pane runtime immediately after terminal core.
That stage only works if the boundary between input routing, PTY IO, and
terminal mutation is explicit.

The current codebase already has `PaneRuntime`, `PaneRuntimeRegistry`,
and `PtySession`, but it does not yet have a documented event model that
explains how pane writes, resize requests, PTY output, exit status, and
runtime errors should move through the system.

Without an ADR here, the runtime layer can regress into ad hoc direct
calls, hidden threading assumptions, and application-specific lifecycle
rules.

## Decision

Noctrail standardizes on a command-and-event runtime boundary:

```rust
pub enum RuntimeCommand {
    Write { pane_id: PaneId, bytes: Vec<u8> },
    Resize { pane_id: PaneId, size: PtySize },
    Close { pane_id: PaneId },
    Restart { pane_id: PaneId, command: PtyCommand },
}

pub enum RuntimeEvent {
    Output { pane_id: PaneId, bytes: Bytes },
    Exited { pane_id: PaneId, status: PtyExitStatus },
    Error { pane_id: PaneId, error: RuntimeError },
}
```

The consequences of this boundary are intentional:

1. Input routing produces runtime commands instead of calling random PTY
   internals directly.
2. PTY output is delivered as runtime events and then interpreted by the
   terminal owner thread.
3. Terminal mutation remains single-owner even if PTY reads happen in a
   background task.
4. Pane lifecycle transitions such as close and restart must always
   produce observable outcomes instead of implicit side effects.

The runtime layer owns:

- pane ids and runtime handles
- PTY session orchestration
- command routing and event emission
- bounded buffering and drain budgeting

The runtime layer does not own:

- VT parsing
- render planning
- layout decisions
- shell integration semantics
- agent or policy behavior

## Consequences

Positive consequences:

- Multi-pane scaling can evolve without rewriting terminal semantics.
- Backpressure and shutdown behavior have a single architectural home.
- App and layout layers can stay focused on routing, focus, and redraw
  rather than process internals.

Costs and tradeoffs:

- Early implementations may feel more verbose than direct method calls
  because commands and events make lifecycle edges explicit.
- Runtime tests need to cover close, restart, and error paths instead of
  assuming success-only behavior.
- Bounded output handling becomes a first-class design problem instead of
  something hidden inside PTY readers.

## Follow-up

- Introduce concrete runtime command/event types in `noctrail-runtime`
  before adding multi-pane reader tasks.
- Add bounded-output and redraw-budget tests once runtime events are
  asynchronous.
- Keep pane close behavior auditable: no zombie child process, no silent
  output loss, and no hidden renderer coupling.
