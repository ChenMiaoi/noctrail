# Layout Tree

This document records the current pure-layout boundary and the app-level
rules layered on top of it.

## Pure Layout

`noctrail-layout` owns:

- `WorkspaceId`
- `SplitAxis`
- `FocusDirection`
- `LayoutRect`
- `PaneLayout`
- `LayoutTree`
- `WorkspaceSet`

`LayoutTree` supports:

- insert root
- set active pane
- directional focus
- swap active pane with a directional neighbor
- resize the active split
- arrange a surface into pane rects
- split active or explicit panes
- close panes and collapse survivors

## Workspace Rules

- Workspace ids are limited to `1..=9`.
- `WorkspaceSet` keeps one active workspace id.
- Switching to a missing workspace creates an empty layout entry.

## Geometry Rules

- Auto split axis is derived from the target rect shape.
- Explicit split axis may override auto behavior.
- Split ratios stay clamped so neither child collapses to zero size.

## App-Level Notes

`noctrail-app` layers these rules on top:

- each workspace owns its own pane tree
- pane PTY sizes are derived from arranged rects
- scratch pane is not part of the workspace layout tree
- scratch pane visibility is app state, not pure layout state

## Validation

- `cargo test -p noctrail-layout --all-targets`
- `cargo test -p noctrail-app --all-targets`
- `cargo run -p noctrail-app -- smoke`
