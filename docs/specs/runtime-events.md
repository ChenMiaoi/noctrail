# Runtime Events

This document records the current `noctrail-runtime` command/event
boundary.

## Identity

- Every live pane is addressed by `PaneId`.
- `PaneRuntimeRegistry` owns pane ids and runtime handles.
- Terminal mutation stays outside the runtime layer.

## Commands

Current commands:

```rust
pub enum RuntimeCommand {
    Write { pane_id: PaneId, bytes: Vec<u8> },
    Resize { pane_id: PaneId, size: PtySize },
    Close { pane_id: PaneId },
    Restart { pane_id: PaneId, command: PtyCommand },
}
```

The runtime layer currently exposes synchronous helpers that apply these
commands to live pane runtimes.

## Events

Current events:

```rust
pub enum RuntimeEvent {
    Output { pane_id: PaneId, bytes: Vec<u8> },
    Exited { pane_id: PaneId, status: PtyExitStatus },
    Error { pane_id: PaneId, error: RuntimeError },
}
```

`Output` represents PTY bytes. `Exited` marks observable process
termination. `Error` carries runtime or PTY failures without requiring
the app to guess whether the pane still exists.

## Output Queue

Current bounded queue controls:

- `capacity_bytes`
- `high_watermark_bytes`
- `drain_budget_bytes`

Current defaults:

- capacity: `256 KiB`
- high watermark: `192 KiB`
- drain budget: `32 KiB`

The queue rejects invalid config and oversize writes instead of growing
without limit.

## Lifecycle

Current runtime lifecycle paths include:

- spawn shell
- write input
- resize
- read output
- close
- restart under the same `PaneId`
- kill

## Validation

- `cargo test -p noctrail-runtime --all-targets`
- `cargo run -p noctrail-cli -- pty-smoke`
