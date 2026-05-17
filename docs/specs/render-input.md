# Render Input

This document records the immutable render boundary consumed by
`noctrail-render`.

## Input

Current `RenderInput` fields:

- `pane_rect`
- `viewport`
- `backend`
- `snapshot`
- `damage`
- `active`
- `border`
- `corner_radius`

The app layer chooses pane geometry, backend mode, and pane chrome. The
terminal layer provides immutable snapshot and damage data.

## Render Plan

Current `RenderPlan` fields include:

- backend
- pane outer rect
- inner viewport rect
- damage
- scrollback row count
- cursor
- alternate-screen bit
- optional selection
- active pane bit
- border style
- corner radius
- prepared render rows

`RenderPlan` is derived data. It does not mutate terminal state.

## Prepared Outputs

Current renderer preparation paths produce:

- glyph frame data
- cell paint frame data
- border overlay data
- combined `PreparedRenderFrame`

Current deterministic software checks cover:

- glyph dedupe
- wide-cell spans
- combining-mark carry-through
- partial damage row limiting
- selection paint overlays
- cursor overlays
- active and inactive pane border overlays

## Backend Notes

- `RenderBackend::Gpu` is the preferred GUI path.
- `RenderBackend::Software` remains the deterministic fixture and
  fallback path.
- GPU diagnostics are surfaced through `noctrail-cli doctor gpu`.

## Validation

- `cargo test -p noctrail-render --all-targets`
- `cargo run -p noctrail-cli -- render-fixtures`
- `cargo run -p noctrail-cli -- render-smoke`
