# ADR 0006: Platform Module Boundaries

- Status: Accepted
- Date: 2026-05-18
- Deciders: Noctrail contributors
- Supersedes: none

## Context

Cross-platform support is already a product requirement, but several
crates still mix platform branches directly into large entry files.
That creates two maintenance problems:

1. Windows, macOS, and Unix behavior becomes harder to review because
   product logic and platform glue live in the same function body.
2. Large files keep growing because every new platform exception lands
   next to unrelated state transitions, rendering, or command routing.

The roadmap already says platform differences belong at the edges. This
ADR makes that rule concrete for day-to-day refactors.

## Decision

Noctrail keeps platform differences behind explicit modules instead of
scattered `cfg` branches in product logic.

Required structure:

- Use `platform/{windows,unix}.rs` or
  `platform/{windows,linux,macos}.rs` when a crate has real
  OS-specific behavior.
- The crate root or parent module may keep a small `mod platform;`
  declaration and a narrow dispatch wrapper.
- Business logic may call uniform helpers such as
  `platform::configure_window_attributes(...)` or
  `platform::configure_pty_writer(...)`, but it should not inline the
  OS split itself.

Review rules:

1. Extract platform decision points instead of copying entire workflows.
2. Do not introduce trait-heavy abstraction solely to hide `cfg`.
3. Prefer narrow data-in/data-out helpers over stateful platform objects.
4. Keep most platform-specific APIs `pub(crate)` or private.

File-size guardrails:

- Entry files such as `main.rs`, `lib.rs`, `gui.rs`, or other top-level
  assembly modules should target roughly 200-400 lines.
- Pure logic modules may exceed that when the logic is cohesive, but
  should usually stay below 500-700 lines.
- When a file grows beyond those limits, contributors should split by
  responsibility rather than by arbitrary naming.

## Consequences

Positive consequences:

- Reviews can reason about cross-platform behavior in one place.
- Large files shrink without introducing speculative architecture.
- Product logic becomes easier to test because platform choices are
  isolated to smaller helpers.

Costs and tradeoffs:

- Some crates gain extra module files even when behavior is simple.
- Refactors must touch imports and visibility more often.
- A few tiny `cfg` wrappers will still exist at module boundaries.

## Follow-up

- Apply this rule first to `noctrail-pty`, `noctrail-render`,
  `noctrail-cli` installer smoke, and `noctrail-app` GUI helpers.
- Reject future changes that add new OS-specific branches to large
  product files when a platform helper would keep the code flatter.
