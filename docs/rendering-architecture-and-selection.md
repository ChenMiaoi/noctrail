# Noctrail Rendering Architecture and Selection

This is the short-form rendering architecture document for the current
repository state. It complements:

- [docs/adr/0002-renderer-wgpu-text-stack.md](adr/0002-renderer-wgpu-text-stack.md)
- [docs/rendering-ecosystem-notes.md](rendering-ecosystem-notes.md)

The ADR is the binding decision. This file records the current practical
boundary and the selection/render ownership split.

## Pipeline

The render path stays snapshot-driven:

```text
TerminalState
  -> TerminalSnapshot + DamageSet
  -> RenderInput
  -> RenderPlan
  -> PreparedRenderFrame
  -> GpuRenderer or software fixture path
```

Current code boundaries:

- `noctrail-term` owns grid, cursor, selection, alternate screen, and
  damage semantics.
- `noctrail-app` owns pane rectangles, active pane state, pane chrome,
  and backend choice.
- `noctrail-render` consumes immutable input and prepares glyph, paint,
  and border overlays.

## Render Input

`RenderInput` currently carries:

- pane outer rect
- inner viewport rect
- render backend
- immutable terminal snapshot
- damage set
- active pane bit
- border style
- corner radius

`RenderPlan` materializes those inputs into renderer-friendly rows while
keeping terminal ownership outside the renderer.

## Selection Ownership

Selection remains a terminal concern, not a renderer concern.

- `TerminalState` stores and normalizes selection ranges.
- `TerminalSnapshot.selection` exposes the immutable selection state.
- `noctrail-render` turns selection rows into paint rect overlays.
- Copy semantics continue to come from terminal selection text, not from
  renderer hit-testing.

This keeps selection behavior stable across GUI, fixtures, and future
renderer changes.

## Pane Chrome

Pane chrome is injected by the app layer rather than inferred inside the
renderer.

Current pane-level render metadata includes:

- active/inactive border colors
- border width
- pane surface fills
- status/header chrome fills
- pane gap and padding
- corner radius

The renderer uses that metadata to prepare border overlays and to keep
inner terminal content separate from outer pane chrome.

The current chrome split is:

- `noctrail-app` computes pane/status/header rects and their colors.
- `noctrail-render` consumes those immutable chrome rects alongside
  terminal rows.
- GUI-specific status text may still be composed separately, but the
  pane surface, header background, active indicator, and separator are
  now part of the shared render input rather than ad-hoc presenter
  fills.

## Fallback and Diagnostics

The GPU path is primary, but the repository keeps explicit fallback and
diagnostic surfaces:

- `cargo run -p noctrail-cli -- doctor gpu`
- `cargo run -p noctrail-cli -- doctor font`
- `cargo run -p noctrail-cli -- render-smoke`
- `cargo run -p noctrail-cli -- render-fixtures`
- `cargo run -p noctrail-app -- smoke`

If GPU init fails, the GUI falls back to a diagnosable software mode
instead of silently black-screening. Unsupported blur or transparency
also degrades to readable solid rendering.

## Current Validation

The repository currently validates the rendering boundary through:

- deterministic `.ntshot` fixtures under `tests/fixtures/render/`
- `noctrail-cli render-fixtures`
- `noctrail-cli render-smoke`
- renderer unit tests for glyph preparation, damage, paint overlays, and
  pane borders

This is the current proof that selection, cursor, damage, and pane
chrome stay renderer-readable without pushing ownership into the GPU
layer.
