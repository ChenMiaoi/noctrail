# ADR 0002: Renderer WGPU Text Stack

- Status: Accepted
- Date: 2026-05-17
- Deciders: Noctrail contributors
- Supersedes: none
- Related: [rendering note](../rendering-ecosystem-notes.md)

## Context

The roadmap requires a real GPU-first rendering path instead of a
software-presenter prototype. The current repository already has a
`noctrail-render` crate and a long-form rendering note, but the actual
decision still needs an ADR that defines the default stack and the
fallback policy.

This ADR has to settle four questions:

1. Which window and GPU abstraction layers are the default path.
2. Which text stack is used to validate the first GPU renderer.
3. Where the renderer boundary starts and stops.
4. How fallback works when GPU initialization or advanced effects fail.

## Decision

Noctrail adopts a GPU-first, RenderPlan-driven renderer stack with these
defaults:

| Layer | Choice | Purpose |
|---|---|---|
| window and event loop | `winit` | cross-platform window lifecycle and input |
| GPU abstraction | `wgpu` | primary backend across Metal, DX12, and Vulkan |
| text shaping and fallback | `cosmic-text` | font discovery, shaping, fallback, rasterization |
| initial `wgpu` text renderer | `glyphon` | fastest path to prove GPU text rendering |
| terminal parser | `vte` | escape parsing remains outside renderer ownership |
| PTY boundary | `portable-pty` | session abstraction remains outside renderer ownership |

Renderer architecture follows this flow:

```text
TerminalState
  -> TerminalSnapshot + DamageSet
  -> RenderInput / RenderPlan
  -> Renderer
      -> wgpu surface/device/queue
      -> cosmic-text shaping and fallback
      -> glyphon-backed glyph cache path
      -> frame presentation
```

The renderer boundary is intentionally narrow:

- `noctrail-render` reads immutable terminal snapshots.
- `noctrail-render` does not parse VT, manage PTY sessions, or hold
  layout/workspace ownership.
- `noctrail-app` owns window lifecycle and surface recovery.
- `noctrail-term` owns cell semantics, cursor state, and damage.

Fallback policy is also fixed:

1. GPU is the default render path on supported platforms.
2. Software or debug rendering may exist for diagnostics and screenshot
   testing, but it is not the long-term feature path.
3. Unsupported transparency, blur, or animation must degrade to readable
   solid rendering without changing terminal correctness.
4. GPU init failures must surface a diagnosable error instead of a black
   window or silent mode switch.

## Consequences

Positive consequences:

- The project now has a concrete path toward a real GPU terminal instead
  of an open-ended renderer placeholder.
- Text shaping and fallback can evolve without coupling renderer design
  to terminal storage.
- Screenshot and diagnostics tooling can share one documented backend
  story.

Costs and tradeoffs:

- `glyphon` is a tactical starting point, not a promise that Noctrail
  will never need a more specialized glyph pipeline.
- GPU-first means backend diagnostics and recovery paths must be treated
  as product work, not optional polish.
- The renderer must keep a careful separation from terminal semantics so
  convenience shortcuts are harder to justify.

## Follow-up

- Define `RenderInput` and `DamageSet` more precisely in a render spec.
- Add a backend diagnostics surface to `noctrail-cli doctor`.
- Revisit the glyph path only after measurement shows `glyphon` no
  longer satisfies multi-pane or high-output requirements.
